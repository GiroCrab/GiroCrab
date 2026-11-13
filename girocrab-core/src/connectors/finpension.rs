//! finpension Säule 3a (Swiss 3a pension foundation) connector.
//!
//! Synchronises finpension 3a portfolios, fund holdings, cash reserves and
//! transaction histories into GiroCrab using finpension's official web app APIs
//! (`accounts.finpension.ch` and `3a.finpension.ch`).
//!
//! Two-phase authentication flow:
//!   Phase 1 (`begin`):
//!     POST https://accounts.finpension.ch/api/login
//!     Body: {"country_code": "CH", "password": "...", "server": "", "mobile_number": "+41..."}
//!     Response: {"message": "OK", "method": "sms"} -> Returns Phase::Challenge (SMS Code)
//!     (Or re-uses a valid stored Bearer token from ctx.state)
//!
//!   Phase 2 (`respond`):
//!     POST https://accounts.finpension.ch/api/login2fa
//!     Body: {"country_code": "CH", "password": "...", "code": "...", "mobile_number": "+41..."}
//!     Response: {"token": "<JWT/Bearer token>"} -> Returns Phase::Ready
//!
//!   Fetch (`fetch`):
//!     GET https://3a.finpension.ch/api/portfolios
//!     Authorization: Bearer <token>
//!     Returns portfolio accounts, asset holdings (including liquidity/cash), and transactions.
//!
//!   Finish (`finish`):
//!     POST https://accounts.finpension.ch/api/logout

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE, ORIGIN, REFERER};
use serde_json::{json, Value};

use super::{
    http_client, BankConnector, Challenge, ChallengeKind, ChallengeResponse, ConnectContext,
    FetchResult, Phase, RemoteAccount, RemoteHolding, RemoteTransaction,
};
use crate::error::{CoreError, CoreResult};
use crate::i18n::{tr, trf};
use crate::models::{AccountKind, Holding};
use crate::money::json_number_to_cents;
use crate::secret::SecretString;

const DEFAULT_AUTH_URL: &str = "https://accounts.finpension.ch/api";
const DEFAULT_API_URL: &str = "https://3a.finpension.ch/api";
const APP_ORIGIN: &str = "https://app.finpension.ch";
const APP_VERSION: &str = "6.1.5";

/// Extracts ISO 3166-1 alpha-2 country code from an E.164 phone number.
pub fn extract_country_code(phone: &str) -> Option<&'static str> {
    let digits = phone.strip_prefix('+')?;
    // Match longest prefix first (e.g. +423 before +42 before +4)
    for len in (1..=3).rev() {
        if digits.len() >= len {
            if let Some(cc) = calling_code_to_country(&digits[..len]) {
                return Some(cc);
            }
        }
    }
    None
}

fn calling_code_to_country(prefix: &str) -> Option<&'static str> {
    match prefix {
        "41" => Some("CH"),
        "49" => Some("DE"),
        "43" => Some("AT"),
        "423" => Some("LI"),
        "1" => Some("US"),
        "33" => Some("FR"),
        "34" => Some("ES"),
        "39" => Some("IT"),
        "44" => Some("GB"),
        "45" => Some("DK"),
        "46" => Some("SE"),
        "47" => Some("NO"),
        "48" => Some("PL"),
        "31" => Some("NL"),
        "32" => Some("BE"),
        "351" => Some("PT"),
        "352" => Some("LU"),
        "353" => Some("IE"),
        "358" => Some("FI"),
        "420" => Some("CZ"),
        "421" => Some("SK"),
        _ => None,
    }
}

/// Normalises phone numbers to E.164 format. Defaulting trunk 0 to Switzerland (+41).
pub fn normalize_phone(input: &str) -> String {
    let t = input.trim();
    if t.is_empty() {
        return String::new();
    }
    if t.starts_with('+') {
        let digits: String = t[1..].chars().filter(|c| c.is_ascii_digit()).collect();
        return format!("+{digits}");
    }
    if let Some(rest) = t.strip_prefix("00") {
        let digits: String = rest.chars().filter(|c| c.is_ascii_digit()).collect();
        return format!("+{digits}");
    }
    let digits: String = t.chars().filter(|c| c.is_ascii_digit()).collect();
    if let Some(rest) = digits.strip_prefix('0') {
        return format!("+41{rest}");
    }
    format!("+{digits}")
}

fn headers(token: Option<&str>, is_login: bool) -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(ACCEPT, HeaderValue::from_static("application/json"));
    h.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    h.insert("x-app-version", HeaderValue::from_static(APP_VERSION));
    h.insert("x-app-platform", HeaderValue::from_static("web"));
    h.insert(ORIGIN, HeaderValue::from_static(APP_ORIGIN));
    if is_login {
        h.insert(
            REFERER,
            HeaderValue::from_static("https://app.finpension.ch/login"),
        );
    }
    if let Some(t) = token {
        if let Ok(hv) = HeaderValue::from_str(&format!("Bearer {t}")) {
            h.insert(AUTHORIZATION, hv);
        }
    }
    h
}

pub struct FinpensionConnector {
    auth_base_url: String,
    api_base_url: String,
    phone: String,
    country_code: String,
    cached_password: Option<SecretString>,
    auth_token: Option<String>,
}

impl FinpensionConnector {
    pub fn new() -> Self {
        Self {
            auth_base_url: DEFAULT_AUTH_URL.into(),
            api_base_url: DEFAULT_API_URL.into(),
            phone: String::new(),
            country_code: String::new(),
            cached_password: None,
            auth_token: None,
        }
    }

    #[cfg(test)]
    pub fn with_base_urls(auth_base: String, api_base: String) -> Self {
        Self {
            auth_base_url: auth_base,
            api_base_url: api_base,
            phone: String::new(),
            country_code: String::new(),
            cached_password: None,
            auth_token: None,
        }
    }
}

impl Default for FinpensionConnector {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BankConnector for FinpensionConnector {
    async fn begin(&mut self, ctx: &ConnectContext) -> CoreResult<Phase> {
        self.phone = normalize_phone(&ctx.login_name);
        self.country_code = match extract_country_code(&self.phone) {
            Some(cc) => cc.to_string(),
            None => {
                return Err(CoreError::InvalidInput {
                    message: trf(
                        "Konnte Ländercode nicht aus der Telefonnummer ermitteln („{0}“). Bitte im Format +41… oder +49… eingeben.",
                        &[&ctx.login_name],
                    ),
                });
            }
        };

        // 1. Check if a persisted token is still valid
        if let Some(stored_token) = ctx.state["token"].as_str() {
            if !stored_token.trim().is_empty() {
                let client = http_client()?;
                let verify_res = client
                    .get(format!("{}/portfolios", self.api_base_url))
                    .headers(headers(Some(stored_token), false))
                    .send()
                    .await;
                if let Ok(resp) = verify_res {
                    if resp.status().is_success() {
                        self.auth_token = Some(stored_token.to_string());
                        return Ok(Phase::Ready);
                    }
                }
            }
        }

        // 2. Fresh login requires password
        let secret = ctx.secret.expose();
        if secret.is_empty() {
            return Err(CoreError::Authentication {
                message: tr("finpension: Passwort erforderlich.").into(),
            });
        }

        let client = http_client()?;
        let body = json!({
            "country_code": self.country_code,
            "password": secret,
            "server": "",
            "mobile_number": self.phone,
        });

        let resp = client
            .post(format!("{}/login", self.auth_base_url))
            .headers(headers(None, true))
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let val: Value = super::read_json_or_null(resp).await;

        if !status.is_success() {
            let msg = val["message"]
                .as_str()
                .or_else(|| val["error"].as_str())
                .unwrap_or("Anmeldung fehlgeschlagen");
            return Err(CoreError::Authentication {
                message: format!("finpension: {msg}"),
            });
        }

        // Check if token was directly returned without 2FA
        if let Some(token) = val["token"].as_str() {
            self.auth_token = Some(token.to_string());
            return Ok(Phase::Ready);
        }

        // SMS challenge expected
        let is_sms = val["method"].as_str() == Some("sms") || val["message"].as_str() == Some("OK");
        if is_sms {
            self.cached_password = Some(ctx.secret.clone());
            let challenge = Challenge {
                kind: ChallengeKind::Code,
                title: tr("SMS-Code erforderlich").into(),
                message: trf(
                    "Es wurde ein SMS-Code an {0} gesendet. Bitte den Code eingeben:",
                    &[&self.phone],
                ),
                method_name: tr("finpension SMS-Code").into(),
                redirect_url: None,
                image: None,
                expires_in_seconds: Some(300),
                poll_interval_seconds: 0,
            };
            return Ok(Phase::Challenge(challenge));
        }

        Err(CoreError::Authentication {
            message: tr("finpension: Unerwartete Antwort beim Anmeldeversuch.").into(),
        })
    }

    async fn respond(
        &mut self,
        _ctx: &ConnectContext,
        response: ChallengeResponse,
    ) -> CoreResult<Phase> {
        let ChallengeResponse::Code(code) = response else {
            return Err(CoreError::InvalidInput {
                message: tr("finpension benötigt den SMS-Code.").into(),
            });
        };

        let code_str = code.expose().trim().to_string();
        if code_str.is_empty() {
            return Err(CoreError::InvalidInput {
                message: tr("Kein SMS-Code eingegeben.").into(),
            });
        }

        let password = self.cached_password.take().ok_or_else(|| CoreError::Authentication {
            message: tr("Passwort nicht mehr im Speicher. Bitte Anmeldung neu starten.").into(),
        })?;

        let client = http_client()?;
        let body = json!({
            "mobile_number": self.phone,
            "country_code": self.country_code,
            "code": code_str,
            "password": password.expose(),
        });

        let resp = client
            .post(format!("{}/login2fa", self.auth_base_url))
            .headers(headers(None, false))
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let val: Value = super::read_json_or_null(resp).await;

        if !status.is_success() {
            let msg = val["message"]
                .as_str()
                .or_else(|| val["error"].as_str())
                .unwrap_or("2FA-Code ungültig");
            return Err(CoreError::Authentication {
                message: format!("finpension 2FA: {msg}"),
            });
        }

        let token = val["token"].as_str().ok_or_else(|| CoreError::Authentication {
            message: tr("finpension: Kein Token in der 2FA-Antwort.").into(),
        })?;

        self.auth_token = Some(token.to_string());
        Ok(Phase::Ready)
    }

    async fn fetch(&mut self, _ctx: &ConnectContext) -> CoreResult<FetchResult> {
        let token = self.auth_token.as_ref().ok_or(CoreError::SessionClosed)?;
        let client = http_client()?;

        let resp = client
            .get(format!("{}/portfolios", self.api_base_url))
            .headers(headers(Some(token), false))
            .send()
            .await?;

        let status = resp.status();
        let val: Value = super::read_json_or_null(resp).await;

        if !status.is_success() {
            let msg = val["message"]
                .as_str()
                .or_else(|| val["error"].as_str())
                .unwrap_or("Portfolios konnten nicht abgerufen werden");
            return Err(CoreError::Bank {
                code: status.as_u16().to_string(),
                message: format!("finpension: {msg}"),
            });
        }

        let today = super::today();
        let (accounts, holdings, transactions) = parse_portfolios_data(&val, &today)?;

        let mut out = FetchResult {
            accounts,
            holdings,
            transactions,
            state: self.auth_token.as_ref().map(|t| {
                json!({
                    "token": t,
                    "phone": self.phone,
                    "country_code": self.country_code,
                })
            }),
            ..Default::default()
        };

        // Mark fetched accounts
        for acc in &out.accounts {
            out.holdings_fetched.push(acc.external_ref.clone());
        }

        Ok(out)
    }

    async fn finish(&mut self, _ctx: &ConnectContext) {
        if let Some(token) = self.auth_token.take() {
            if let Ok(client) = http_client() {
                let _ = client
                    .post(format!("{}/logout", self.auth_base_url))
                    .headers(headers(Some(&token), false))
                    .send()
                    .await;
            }
        }
    }
}

/// Parses the finpension `/portfolios` response into GiroCrab accounts, holdings, and transactions.
pub fn parse_portfolios_data(
    val: &Value,
    today: &str,
) -> CoreResult<(Vec<RemoteAccount>, Vec<RemoteHolding>, Vec<RemoteTransaction>)> {
    let portfolios = val.as_array().ok_or_else(|| CoreError::Protocol {
        message: "finpension: Erwartete Liste von Portfolios".into(),
    })?;

    let mut accounts = Vec::new();
    let mut holdings = Vec::new();
    let mut transactions = Vec::new();

    for p in portfolios {
        let id_num = p["id"]
            .as_i64()
            .or_else(|| p["id"].as_str().and_then(|s| s.parse().ok()))
            .ok_or_else(|| CoreError::Protocol {
                message: "finpension: Portfolio ohne gültige ID".into(),
            })?;
        let id_str = id_num.to_string();
        let number = p["number"]
            .as_i64()
            .map(|n| n.to_string())
            .unwrap_or_else(|| id_str.clone());
        let name = format!("Finpension Portfolio {number}");
        let balance_cents = json_number_to_cents(&p["performance"]["current_value"]).unwrap_or(0);

        let account_ref = format!("finpension-{id_str}");
        accounts.push(RemoteAccount {
            external_ref: account_ref.clone(),
            name,
            owner: format!("ID: {id_str}"),
            iban: String::new(),
            account_number: id_str.clone(),
            currency: "CHF".into(),
            kind: AccountKind::Brokerage,
            balance_cents,
            balance_date: today.to_string(),
        });

        // 1. Collect ISIN mapping from strategy categories
        let mut isin_by_name = std::collections::HashMap::new();
        if let Some(cats) = p["strategy"]["categories"].as_array() {
            for cat in cats {
                if let Some(positions) = cat["positions"].as_array() {
                    for pos in positions {
                        if let (Some(asset_name), Some(isin)) =
                            (pos["asset_name"].as_str(), pos["isin"].as_str())
                        {
                            if !isin.trim().is_empty() {
                                isin_by_name.insert(asset_name.to_string(), isin.trim().to_string());
                            }
                        }
                    }
                }
            }
        }

        // 2. Estimate purchase price ratio via performance profit/deposits
        let current_profit_cents =
            json_number_to_cents(&p["performance"]["current_profit_value"]).unwrap_or(0);
        let deposits_cents =
            json_number_to_cents(&p["performance"]["deposits_ytd"]).unwrap_or(0);
        let profit_ratio = if deposits_cents > 0 && current_profit_cents != 0 {
            current_profit_cents as f64 / deposits_cents as f64
        } else {
            0.0
        };

        // 3. Extract asset allocations into holdings
        if let Some(classes) = p["allocation_by_asset_class"].as_array() {
            for cls in classes {
                if let Some(positions) = cls["positions"].as_array() {
                    for pos in positions {
                        let asset_name = pos["asset_name"].as_str().unwrap_or("Unbekannt");
                        let value_cents = json_number_to_cents(&pos["value"]).unwrap_or(0);
                        let shares = pos["shares"].as_f64().unwrap_or(0.0);

                        if asset_name.eq_ignore_ascii_case("cash")
                            || asset_name.eq_ignore_ascii_case("liquidität")
                        {
                            holdings.push(RemoteHolding {
                                account_ref: account_ref.clone(),
                                holding: Holding {
                                    isin: String::new(),
                                    wkn: String::new(),
                                    name: tr("Liquidität").into(),
                                    quantity: 1.0,
                                    current_price_cents: value_cents,
                                    purchase_price_cents: value_cents,
                                    value_cents,
                                },
                            });
                        } else {
                            let current_price_cents = if shares > 0.0 {
                                ((value_cents as f64) / shares).round() as i64
                            } else {
                                0
                            };

                            let purchase_price_cents = if profit_ratio != 0.0 {
                                ((current_price_cents as f64) / (1.0 + profit_ratio)).round() as i64
                            } else {
                                current_price_cents
                            };

                            let isin = isin_by_name.get(asset_name).cloned().unwrap_or_default();

                            holdings.push(RemoteHolding {
                                account_ref: account_ref.clone(),
                                holding: Holding {
                                    isin,
                                    wkn: String::new(),
                                    name: asset_name.into(),
                                    quantity: shares,
                                    current_price_cents,
                                    purchase_price_cents,
                                    value_cents,
                                },
                            });
                        }
                    }
                }
            }
        }

        // 4. Extract portfolio transactions
        if let Some(tx_list) = p["transactions"].as_array() {
            for tx in tx_list {
                let tx_id = tx["id"]
                    .as_i64()
                    .map(|n| n.to_string())
                    .or_else(|| tx["id"].as_str().map(|s| s.to_string()))
                    .unwrap_or_else(|| {
                        let date = tx["date"].as_str().unwrap_or(today);
                        let typ = tx["type"].as_str().unwrap_or("tx");
                        format!("{date}-{typ}")
                    });
                let booking_date = tx["date"]
                    .as_str()
                    .unwrap_or(today)
                    .get(..10)
                    .unwrap_or(today)
                    .to_string();
                let amount_val = json_number_to_cents(&tx["amount"]).unwrap_or(0);
                let tx_type = tx["type"].as_str().unwrap_or("deposit");

                let amount_cents = if tx_type.eq_ignore_ascii_case("fee")
                    || tx_type.eq_ignore_ascii_case("withdrawal")
                {
                    -amount_val.abs()
                } else {
                    amount_val.abs()
                };

                let purpose = match tx_type {
                    "deposit" => tr("Einzahlung Säule 3a").to_string(),
                    "fee" => tr("Pauschalgebühr").to_string(),
                    "withdrawal" => tr("Auszahlung").to_string(),
                    other => other.to_string(),
                };

                transactions.push(RemoteTransaction {
                    account_ref: account_ref.clone(),
                    external_id: format!("finpension-{id_str}-{tx_id}"),
                    booking_date: booking_date.clone(),
                    value_date: booking_date,
                    amount_cents,
                    currency: "CHF".into(),
                    counterparty: "finpension 3a".into(),
                    counterparty_iban: String::new(),
                    purpose,
                    booking_text: tx_type.to_uppercase(),
                    ..Default::default()
                });
            }
        }
    }

    Ok((accounts, holdings, transactions))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_phone_normalization_and_country_code() {
        assert_eq!(normalize_phone("+41 79 123 45 67"), "+41791234567");
        assert_eq!(normalize_phone("079 123 45 67"), "+41791234567");
        assert_eq!(normalize_phone("0049 170 1234567"), "+491701234567");
        assert_eq!(normalize_phone("+423 123 45 67"), "+4231234567");

        assert_eq!(extract_country_code("+41791234567"), Some("CH"));
        assert_eq!(extract_country_code("+491701234567"), Some("DE"));
        assert_eq!(extract_country_code("+436601234567"), Some("AT"));
        assert_eq!(extract_country_code("+4231234567"), Some("LI"));
        assert_eq!(extract_country_code("+12025550123"), Some("US"));
        assert_eq!(extract_country_code("invalid"), None);
    }

    #[test]
    fn test_parse_portfolios_response() {
        let fixture = json!([
            {
                "id": 12345,
                "number": 1,
                "created_at": "2023-01-15T10:00:00Z",
                "performance": {
                    "current_value": 15500.50,
                    "current_profit_value": 1200.00,
                    "deposits_ytd": 7056.00
                },
                "strategy": {
                    "categories": [
                        {
                            "positions": [
                                {
                                    "asset_name": "CSIF (CH) III Equity World ex CH Blue - ZB",
                                    "isin": "CH0033782431"
                                }
                            ]
                        }
                    ]
                },
                "allocation_by_asset_class": [
                    {
                        "asset_class": "Liquidity",
                        "positions": [
                            {
                                "asset_name": "Cash",
                                "value": 500.50,
                                "shares": 1.0
                            }
                        ]
                    },
                    {
                        "asset_class": "Equities",
                        "positions": [
                            {
                                "asset_name": "CSIF (CH) III Equity World ex CH Blue - ZB",
                                "value": 15000.00,
                                "shares": 10.0
                            }
                        ]
                    }
                ],
                "transactions": [
                    {
                        "id": 101,
                        "date": "2024-01-10",
                        "type": "deposit",
                        "amount": 7056.00
                    },
                    {
                        "id": 102,
                        "date": "2024-06-30",
                        "type": "fee",
                        "amount": 15.25
                    }
                ]
            }
        ]);

        let (accounts, holdings, transactions) = parse_portfolios_data(&fixture, "2026-01-01").unwrap();

        // 1 Account
        assert_eq!(accounts.len(), 1);
        let acc = &accounts[0];
        assert_eq!(acc.external_ref, "finpension-12345");
        assert_eq!(acc.name, "Finpension Portfolio 1");
        assert_eq!(acc.currency, "CHF");
        assert_eq!(acc.kind, AccountKind::Brokerage);
        assert_eq!(acc.balance_cents, 1_550_050);

        // 2 Holdings: Cash + 1 Fund
        assert_eq!(holdings.len(), 2);
        let cash_pos = &holdings[0].holding;
        assert_eq!(cash_pos.name, "Liquidität");
        assert_eq!(cash_pos.value_cents, 50_050);
        assert_eq!(cash_pos.current_price_cents, 50_050);

        let equity_pos = &holdings[1].holding;
        assert_eq!(equity_pos.name, "CSIF (CH) III Equity World ex CH Blue - ZB");
        assert_eq!(equity_pos.isin, "CH0033782431");
        assert_eq!(equity_pos.quantity, 10.0);
        assert_eq!(equity_pos.value_cents, 1_500_000);
        assert_eq!(equity_pos.current_price_cents, 150_000);

        // 2 Transactions: 1 deposit (+), 1 fee (-)
        assert_eq!(transactions.len(), 2);
        assert_eq!(transactions[0].amount_cents, 705_600);
        assert_eq!(transactions[0].currency, "CHF");
        assert_eq!(transactions[0].booking_date, "2024-01-10");

        assert_eq!(transactions[1].amount_cents, -1_525);
        assert_eq!(transactions[1].booking_text, "FEE");
    }

    #[tokio::test]
    async fn test_finpension_full_sync_flow_with_mock_server() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());

        std::thread::spawn(move || {
            while let Ok((mut s, _)) = listener.accept() {
                let mut buf = [0u8; 8192];
                let len = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..len]);
                let first_line = req.lines().next().unwrap_or("");

                let (status, body) = if first_line.contains("/login2fa") {
                    ("200 OK", r#"{"token":"test-jwt-token-12345"}"#)
                } else if first_line.contains("/login") {
                    ("200 OK", r#"{"message":"OK","method":"sms"}"#)
                } else if first_line.contains("/portfolios") {
                    ("200 OK", r#"[{"id":999,"number":1,"performance":{"current_value":1000.00},"allocation_by_asset_class":[{"positions":[{"asset_name":"Cash","value":1000.00,"shares":1.0}]}],"transactions":[]}]"#)
                } else if first_line.contains("/logout") {
                    ("200 OK", r#"{"message":"OK"}"#)
                } else {
                    ("404 Not Found", r#"{"error":"not found"}"#)
                };

                let resp = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = s.write_all(resp.as_bytes());
            }
        });

        let mut connector = FinpensionConnector::with_base_urls(base.clone(), base.clone());
        let ctx = ConnectContext {
            connection_id: 1,
            login_name: "+41791234567".into(),
            secret: SecretString::new("Secret123".into()),
            settings: std::collections::HashMap::new(),
            state: serde_json::Value::Null,
            since: "2026-01-01".into(),
            since_by_account: std::collections::HashMap::new(),
        };

        // 1. Begin phase -> challenge
        let phase1 = connector.begin(&ctx).await.unwrap();
        match phase1 {
            Phase::Challenge(c) => {
                assert_eq!(c.kind, ChallengeKind::Code);
                assert_eq!(c.method_name, "finpension SMS-Code");
            }
            _ => panic!("Expected SMS challenge"),
        }

        // 2. Respond phase -> Ready
        let phase2 = connector
            .respond(&ctx, ChallengeResponse::Code(SecretString::new("654321".into())))
            .await
            .unwrap();
        assert!(matches!(phase2, Phase::Ready));

        // 3. Fetch phase -> Returns account & holdings
        let res = connector.fetch(&ctx).await.unwrap();
        assert_eq!(res.accounts.len(), 1);
        assert_eq!(res.accounts[0].external_ref, "finpension-999");
        assert_eq!(res.accounts[0].balance_cents, 100_000);
        assert_eq!(res.holdings.len(), 1);
        assert_eq!(res.holdings[0].holding.name, "Liquidität");

        // Check persisted state
        let state = res.state.expect("state persisted");
        assert_eq!(state["token"], "test-jwt-token-12345");
        assert_eq!(state["country_code"], "CH");

        // 4. Test session re-use without SMS 2FA!
        let ctx_with_session = ConnectContext {
            connection_id: 1,
            login_name: "+41791234567".into(),
            secret: SecretString::new("Secret123".into()),
            settings: std::collections::HashMap::new(),
            state,
            since: "2026-01-01".into(),
            since_by_account: std::collections::HashMap::new(),
        };
        let mut conn2 = FinpensionConnector::with_base_urls(base.clone(), base.clone());
        let phase_reused = conn2.begin(&ctx_with_session).await.unwrap();
        assert!(matches!(phase_reused, Phase::Ready), "Session token must be re-used directly without challenge");

        // 5. Finish (logout)
        conn2.finish(&ctx_with_session).await;
    }
}
