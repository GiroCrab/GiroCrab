//! Finpension (Switzerland): pillar 3a, vested benefits (Freizügigkeit) and 1e
//! portfolios.
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
//! Also supports offline CSV and JSON export files via [`Vault::import_finpension_export`].

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use reqwest::header::{
    HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE, ORIGIN, REFERER,
};
use serde_json::{json, Value};

use super::{
    http_client_same_host, persist::store_fetch, read_json_or_null, BankConnector, Challenge,
    ChallengeKind, ChallengeResponse, ConnectContext, ContentIds, FetchResult, Phase,
    RemoteAccount, RemoteHolding, RemoteTransaction, SyncSummary,
};
use crate::{
    error::{CoreError, CoreResult},
    i18n::{tr, trf},
    models::{AccountKind, ConnectorKind, Holding},
    money::json_number_to_cents_rounded,
    secret::{SecretJson, SecretString},
    vault::Vault,
};

fn invalid(message: impl Into<String>) -> CoreError {
    CoreError::invalid(format!("Finpension: {}", message.into()))
}

/// Host every Finpension URL must belong to.
const HOST: &str = "finpension.ch";

const DEFAULT_AUTH_URL: &str = "https://accounts.finpension.ch/api";
const DEFAULT_API_URL: &str = "https://3a.finpension.ch/api";
const APP_ORIGIN: &str = "https://app.finpension.ch";
const APP_VERSION: &str = "6.1.5";

/// Accepts only `https://finpension.ch/…` or `https://<sub>.finpension.ch/…`
/// (port 443 at most). Rejects other schemes, credentials in the URL,
/// look-alike hosts (`finpension.ch.evil.com`, `evilfinpension.ch`), IP
/// addresses, backslashes, whitespace and control characters.
pub fn validate_finpension_url(url: &str) -> CoreResult<()> {
    let bad = || invalid("only https URLs on finpension.ch are allowed");
    if url
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
        || !url.is_ascii()
    {
        return Err(bad());
    }
    let rest = url
        .get(..8)
        .filter(|p| p.eq_ignore_ascii_case("https://"))
        .map(|_| &url[8..])
        .ok_or_else(bad)?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return Err(bad());
    }
    let host = match authority.rsplit_once(':') {
        Some((h, "443")) => h,
        Some(_) => return Err(bad()),
        None => authority,
    };
    let host = host.to_ascii_lowercase();
    let ok = host == HOST
        || host
            .strip_suffix(".finpension.ch")
            .is_some_and(|sub| !sub.is_empty() && sub.split('.').all(valid_label));
    if ok {
        Ok(())
    } else {
        Err(bad())
    }
}

fn valid_label(l: &str) -> bool {
    !l.is_empty()
        && !l.starts_with('-')
        && !l.ends_with('-')
        && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

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
    if let Some(rest) = t.strip_prefix('+') {
        let digits: String = rest.chars().filter(|c| c.is_ascii_digit()).collect();
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

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct FinpensionState {
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub country_code: Option<String>,
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
        if let Ok(st) = ctx.state.decode::<FinpensionState>() {
            if let Some(ref stored_token) = st.token {
                if !stored_token.trim().is_empty() {
                    if let Ok(client) = http_client_same_host() {
                        let verify_res = client
                            .get(format!("{}/portfolios", self.api_base_url))
                            .headers(headers(Some(stored_token), false))
                            .send()
                            .await;
                        if let Ok(resp) = verify_res {
                            if resp.status().is_success() {
                                self.auth_token = Some(stored_token.clone());
                                return Ok(Phase::Ready);
                            }
                        }
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

        let client = http_client_same_host()?;
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
        let val: Value = read_json_or_null(resp).await;

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

        let password = self
            .cached_password
            .take()
            .ok_or_else(|| CoreError::Authentication {
                message: tr("Passwort nicht mehr im Speicher. Bitte Anmeldung neu starten.").into(),
            })?;

        let client = http_client_same_host()?;
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
        let val: Value = read_json_or_null(resp).await;

        if !status.is_success() {
            let msg = val["message"]
                .as_str()
                .or_else(|| val["error"].as_str())
                .unwrap_or("2FA-Code ungültig");
            return Err(CoreError::Authentication {
                message: format!("finpension 2FA: {msg}"),
            });
        }

        let token = val["token"]
            .as_str()
            .ok_or_else(|| CoreError::Authentication {
                message: tr("finpension: Kein Token in der 2FA-Antwort.").into(),
            })?;

        self.auth_token = Some(token.to_string());
        Ok(Phase::Ready)
    }

    async fn fetch(&mut self, _ctx: &ConnectContext) -> CoreResult<FetchResult> {
        let token = self.auth_token.as_ref().ok_or(CoreError::SessionClosed)?;
        let client = http_client_same_host()?;

        let resp = client
            .get(format!("{}/portfolios", self.api_base_url))
            .headers(headers(Some(token), false))
            .send()
            .await?;

        let status = resp.status();
        let val: Value = read_json_or_null(resp).await;

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
            state: SecretJson::encode(&FinpensionState {
                token: self.auth_token.clone(),
                phone: Some(self.phone.clone()),
                country_code: Some(self.country_code.clone()),
            })
            .ok(),
            ..Default::default()
        };

        for acc in &out.accounts {
            out.holdings_fetched.push(acc.external_ref.clone());
            out.balances_reported.push(acc.external_ref.clone());
        }

        Ok(out)
    }
}

/// Parses the finpension `/portfolios` response into GiroCrab accounts, holdings, and transactions.
pub fn parse_portfolios_data(
    val: &Value,
    today: &str,
) -> CoreResult<(
    Vec<RemoteAccount>,
    Vec<RemoteHolding>,
    Vec<RemoteTransaction>,
)> {
    let portfolios = val.as_array().ok_or_else(|| CoreError::Protocol {
        message: "finpension: Erwartete Liste von Portfolios".into(),
    })?;

    let mut accounts = Vec::new();
    let mut holdings = Vec::new();
    let mut transactions = Vec::new();
    let mut ids = ContentIds::default();

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
            .or_else(|| p["number"].as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| id_str.clone());
        let name = format!("Finpension Portfolio {number}");
        let balance_cents =
            json_number_to_cents_rounded(&p["performance"]["current_value"]).unwrap_or(0);

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
        let mut isin_by_name = HashMap::new();
        if let Some(cats) = p["strategy"]["categories"].as_array() {
            for cat in cats {
                if let Some(positions) = cat["positions"].as_array() {
                    for pos in positions {
                        if let (Some(asset_name), Some(isin)) =
                            (pos["asset_name"].as_str(), pos["isin"].as_str())
                        {
                            if !isin.trim().is_empty() {
                                isin_by_name
                                    .insert(asset_name.to_string(), isin.trim().to_string());
                            }
                        }
                    }
                }
            }
        }

        // 2. Estimate purchase price ratio via performance profit/deposits
        let current_profit_cents =
            json_number_to_cents_rounded(&p["performance"]["current_profit_value"]).unwrap_or(0);
        let deposits_cents =
            json_number_to_cents_rounded(&p["performance"]["deposits_ytd"]).unwrap_or(0);
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
                        let value_cents = json_number_to_cents_rounded(&pos["value"]).unwrap_or(0);
                        let shares = pos["shares"]
                            .as_f64()
                            .or_else(|| pos["shares"].as_str().and_then(|s| s.parse().ok()))
                            .unwrap_or(0.0);

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
                                    currency: "CHF".into(),
                                    ..Default::default()
                                },
                                ..Default::default()
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
                                    currency: "CHF".into(),
                                    ..Default::default()
                                },
                                ..Default::default()
                            });
                        }
                    }
                }
            }
        }

        // 4. Extract portfolio transactions
        if let Some(tx_list) = p["transactions"].as_array() {
            for tx in tx_list {
                let booking_date = tx["date"]
                    .as_str()
                    .unwrap_or(today)
                    .get(..10)
                    .unwrap_or(today)
                    .to_string();
                let amount_val = json_number_to_cents_rounded(&tx["amount"]).unwrap_or(0);
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
                    _ => tx_type.to_string(),
                };

                let mut r = RemoteTransaction {
                    account_ref: account_ref.clone(),
                    booking_date: booking_date.clone(),
                    value_date: booking_date,
                    amount_cents,
                    currency: "CHF".into(),
                    purpose,
                    ..Default::default()
                };

                let has_id = tx["id"].as_i64().map(|n| n.to_string()).or_else(|| {
                    tx["id"]
                        .as_str()
                        .filter(|s| !s.trim().is_empty())
                        .map(|s| s.to_string())
                });
                if let Some(tid) = has_id {
                    r.external_id = format!("finpension-{id_str}-{tid}");
                } else {
                    r.external_id = ids.next(&r);
                }

                transactions.push(r);
            }
        }
    }

    Ok((accounts, holdings, transactions))
}

/// Result of parsing an export.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct FinpensionExport {
    pub holdings: Vec<FinpensionHolding>,
    pub transactions: Vec<FinpensionTransaction>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FinpensionHolding {
    pub isin: String,
    pub name: String,
    pub quantity: f64,
    pub price_cents: i64,
    pub value_cents: i64,
    /// `CHF` or `EUR`.
    pub currency: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FinpensionTransaction {
    /// `YYYY-MM-DD`.
    pub date: String,
    pub description: String,
    pub amount_cents: i64,
    pub currency: String,
    pub reference: String,
}

impl FinpensionHolding {
    pub fn into_remote(self, account_ref: &str) -> RemoteHolding {
        RemoteHolding {
            account_ref: account_ref.into(),
            holding: Holding {
                isin: self.isin,
                name: self.name,
                quantity: self.quantity,
                current_price_cents: self.price_cents,
                value_cents: self.value_cents,
                currency: self.currency,
                ..Default::default()
            },
            ..Default::default()
        }
    }
}

/// Converts transactions to connector rows with stable content ids.
pub fn remote_transactions(
    account_ref: &str,
    transactions: &[FinpensionTransaction],
) -> Vec<RemoteTransaction> {
    let mut ids = ContentIds::default();
    transactions
        .iter()
        .map(|t| {
            let mut r = RemoteTransaction {
                account_ref: account_ref.into(),
                booking_date: t.date.clone(),
                value_date: t.date.clone(),
                amount_cents: t.amount_cents,
                currency: t.currency.clone(),
                purpose: t.description.clone(),
                end_to_end_id: t.reference.clone(),
                ..Default::default()
            };
            r.external_id = ids.next(&r);
            r
        })
        .collect()
}

/// Builds the fetch of one import: a depot (holdings) and a cash account
/// (transactions) per currency present in the export.
pub fn to_fetch(connection_id: i64, name: &str, export: FinpensionExport) -> FetchResult {
    let mut out = FetchResult::default();
    let mut currencies: Vec<String> = export
        .holdings
        .iter()
        .map(|h| h.currency.clone())
        .chain(export.transactions.iter().map(|t| t.currency.clone()))
        .collect();
    currencies.sort();
    currencies.dedup();
    let multi = currencies.len() > 1;
    for cur in currencies {
        let label = |what: &str| {
            if multi {
                format!("{name} {what} ({cur})")
            } else {
                format!("{name} {what}")
            }
        };
        let holdings: Vec<_> = export
            .holdings
            .iter()
            .filter(|h| h.currency == cur)
            .collect();
        if !holdings.is_empty() {
            let depot_ref = format!("finpension-{connection_id}-depot-{cur}");
            out.accounts.push(RemoteAccount {
                external_ref: depot_ref.clone(),
                name: label("Depot"),
                currency: cur.clone(),
                kind: AccountKind::Brokerage,
                balance_cents: holdings.iter().map(|h| h.value_cents).sum(),
                balance_date: crate::dates::today_iso(),
                owner: String::new(),
                iban: String::new(),
                account_number: String::new(),
            });
            out.holdings_fetched.push(depot_ref.clone());
            out.balances_reported.push(depot_ref.clone());
            out.holdings.extend(
                holdings
                    .into_iter()
                    .cloned()
                    .map(|h| h.into_remote(&depot_ref)),
            );
        }
        let txs: Vec<_> = export
            .transactions
            .iter()
            .filter(|t| t.currency == cur)
            .cloned()
            .collect();
        if !txs.is_empty() {
            let cash_ref = format!("finpension-{connection_id}-cash-{cur}");
            out.accounts.push(RemoteAccount {
                external_ref: cash_ref.clone(),
                name: label("Cash"),
                currency: cur.clone(),
                kind: AccountKind::Cash,
                balance_cents: 0,
                balance_date: crate::dates::today_iso(),
                owner: String::new(),
                iban: String::new(),
                account_number: String::new(),
            });
            out.transactions
                .extend(remote_transactions(&cash_ref, &txs));
            // an export carries no cash balance: add the bookings to the stored one
            out.adjust_balances = true;
        }
    }
    out
}

#[uniffi::export]
impl Vault {
    /// Imports a Finpension CSV/JSON export (holdings and cash transactions)
    /// into the connection. Local only, no network access.
    pub fn import_finpension_export(
        self: Arc<Self>,
        connection_id: i64,
        bytes: Vec<u8>,
    ) -> Result<SyncSummary, CoreError> {
        let (kind, name) = self.portal_connection(connection_id)?;
        if kind != ConnectorKind::Finpension {
            return Err(invalid(tr("Die Verbindung gehört nicht zu Finpension.")));
        }
        let text = crate::text::decode_bank_bytes(&bytes);
        let export = parse_export(&text)?;
        if export.holdings.is_empty() && export.transactions.is_empty() {
            return Err(invalid(tr("Keine Positionen oder Buchungen erkannt.")));
        }
        store_fetch(
            &self,
            kind,
            connection_id,
            to_fetch(connection_id, &name, export),
        )
    }
}

/// Parses an export; JSON when it starts with `{` or `[`, otherwise CSV.
/// A CSV is recognized as transactions when it contains cash flow / category / date columns,
/// or as holdings when it has an ISIN column without cash flow columns.
pub fn parse_export(input: &str) -> CoreResult<FinpensionExport> {
    let input = input.trim_start_matches('\u{feff}').trim();
    if input.starts_with('{') || input.starts_with('[') {
        parse_json(input)
    } else {
        let rows = parse_csv_rows(input)?;
        let first_row = rows.first().ok_or_else(|| invalid("CSV has no rows"))?;
        let is_tx = first_row.iter().any(|c| {
            let n = norm(c);
            matches!(
                n.as_str(),
                "cashflow" | "flow" | "buchungstext" | "verwendungszweck"
            )
        }) || (first_row.iter().any(|c| {
            matches!(
                norm(c).as_str(),
                "date" | "datum" | "buchungsdatum" | "valuta"
            )
        }) && first_row.iter().any(|c| {
            matches!(
                norm(c).as_str(),
                "category" | "kategorie" | "amount" | "betrag"
            )
        }));

        let is_holdings = !is_tx && first_row.iter().any(|c| matches!(norm(c).as_str(), "isin"));
        let mut out = FinpensionExport::default();
        if is_holdings {
            out.holdings = holdings_from_csv_rows(&rows)?;
        } else {
            out.transactions = transactions_from_csv_rows(&rows)?;
        }
        Ok(out)
    }
}

pub fn parse_holdings_csv(csv: &str) -> CoreResult<Vec<FinpensionHolding>> {
    holdings_from_csv_rows(&parse_csv_rows(csv.trim_start_matches('\u{feff}'))?)
}

pub fn parse_transactions_csv(csv: &str) -> CoreResult<Vec<FinpensionTransaction>> {
    transactions_from_csv_rows(&parse_csv_rows(csv.trim_start_matches('\u{feff}'))?)
}

// ---------- CSV ----------

fn parse_csv_rows(csv: &str) -> CoreResult<Vec<Vec<String>>> {
    let first = csv.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let delim = if first.matches(';').count() >= first.matches(',').count() && first.contains(';') {
        ';'
    } else {
        ','
    };
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = csv.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            c if c == delim && !quoted => row.push(std::mem::take(&mut field)),
            '\r' if !quoted => {}
            '\n' if !quoted => {
                row.push(std::mem::take(&mut field));
                push_row(&mut rows, std::mem::take(&mut row));
            }
            c => field.push(c),
        }
    }
    if quoted {
        return Err(invalid("unterminated quote in CSV"));
    }
    row.push(field);
    push_row(&mut rows, row);
    if rows.len() < 2 {
        return Err(invalid("CSV has no data rows"));
    }
    Ok(rows)
}

fn push_row(rows: &mut Vec<Vec<String>>, row: Vec<String>) {
    if row.iter().any(|c| !c.trim().is_empty()) {
        rows.push(row);
    }
}

fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

struct Columns(HashMap<String, usize>);

impl Columns {
    fn new(header: &[String]) -> Self {
        let mut m = HashMap::new();
        for (i, h) in header.iter().enumerate() {
            m.entry(norm(h)).or_insert(i);
        }
        Columns(m)
    }
    fn find(&self, names: &[&str]) -> Option<usize> {
        names.iter().find_map(|n| self.0.get(*n).copied())
    }
    fn require(&self, names: &[&str]) -> CoreResult<usize> {
        self.find(names)
            .ok_or_else(|| invalid(format!("missing column {}", names[0])))
    }
}

fn cell(row: &[String], i: usize) -> &str {
    row.get(i).map(|s| s.trim()).unwrap_or("")
}

fn holdings_from_csv_rows(rows: &[Vec<String>]) -> CoreResult<Vec<FinpensionHolding>> {
    let c = Columns::new(&rows[0]);
    let isin = c.require(&["isin"])?;
    let name = c.require(&[
        "name",
        "bezeichnung",
        "wertschrift",
        "security",
        "assetname",
    ])?;
    let qty = c.require(&[
        "quantity",
        "anzahl",
        "menge",
        "stueck",
        "stück",
        "numberofshares",
    ])?;
    let price = c.require(&[
        "price",
        "kurs",
        "preis",
        "preisprostueck",
        "assetpriceinchf",
        "assetprice",
    ])?;
    let value = c.require(&[
        "marketvalue",
        "marktwert",
        "value",
        "wert",
        "marketvalueinchf",
    ])?;
    let cur = c.find(&["currency", "währung", "waehrung", "whg", "assetcurrency"]);
    rows[1..]
        .iter()
        .map(|r| {
            build_holding(
                cell(r, isin),
                cell(r, name),
                parse_decimal(cell(r, qty))?,
                parse_decimal(cell(r, price))?,
                parse_decimal(cell(r, value))?,
                cur.map(|i| cell(r, i)).unwrap_or(""),
            )
        })
        .collect()
}

fn transactions_from_csv_rows(rows: &[Vec<String>]) -> CoreResult<Vec<FinpensionTransaction>> {
    let c = Columns::new(&rows[0]);
    let date = c.require(&["date", "datum", "buchungsdatum", "valuta"])?;
    let desc_col = c.find(&[
        "description",
        "beschreibung",
        "text",
        "buchungstext",
        "verwendungszweck",
    ]);
    let cat_col = c.find(&["category", "kategorie"]);
    let asset_col = c.find(&["assetname", "asset", "wertschrift", "security", "name"]);
    if desc_col.is_none() && cat_col.is_none() && asset_col.is_none() {
        return Err(invalid("missing column description or category"));
    }
    let amount = c.require(&["amount", "betrag", "cashflow", "flow"])?;
    let cur = c.find(&["currency", "währung", "waehrung", "whg", "assetcurrency"]);
    let reference = c.find(&["reference", "referenz", "id", "isin"]);
    rows[1..]
        .iter()
        .map(|r| {
            let desc = if let Some(i) = desc_col {
                cell(r, i).to_string()
            } else {
                let cat = cat_col.map(|i| cell(r, i)).unwrap_or("");
                let asset = asset_col.map(|i| cell(r, i)).unwrap_or("");
                match (cat.is_empty(), asset.is_empty()) {
                    (false, false) => format!("{cat}: {asset}"),
                    (false, true) => cat.to_string(),
                    (true, false) => asset.to_string(),
                    (true, true) => String::new(),
                }
            };
            build_transaction(
                cell(r, date),
                &desc,
                parse_decimal(cell(r, amount))?,
                cur.map(|i| cell(r, i)).unwrap_or(""),
                reference.map(|i| cell(r, i)).unwrap_or(""),
            )
        })
        .collect()
}

// ---------- JSON ----------

fn parse_json(input: &str) -> CoreResult<FinpensionExport> {
    let root: Value = serde_json::from_str(input).map_err(|_| invalid("invalid JSON"))?;
    let (holdings, transactions) = match &root {
        Value::Array(_) => (None, Some(&root)),
        Value::Object(o) => (
            o.get("holdings").or_else(|| o.get("positions")),
            o.get("transactions"),
        ),
        _ => return Err(invalid("unexpected JSON structure")),
    };
    let mut out = FinpensionExport::default();
    for h in rows_of(holdings)? {
        out.holdings.push(build_holding(
            str_of(h, &["isin"]),
            str_of(h, &["name"]),
            num_of(h, &["quantity", "amount"])?,
            num_of(h, &["price", "currentPrice", "current_price"])?,
            num_of(h, &["marketValue", "market_value", "value"])?,
            str_of(h, &["currency"]),
        )?);
    }
    for t in rows_of(transactions)? {
        out.transactions.push(build_transaction(
            str_of(t, &["date", "bookingDate", "booking_date"]),
            str_of(t, &["description", "text"]),
            num_of(t, &["amount"])?,
            str_of(t, &["currency"]),
            str_of(t, &["reference", "id"]),
        )?);
    }
    if out.holdings.is_empty() && out.transactions.is_empty() {
        return Err(invalid("export contains no holdings or transactions"));
    }
    Ok(out)
}

fn rows_of(v: Option<&Value>) -> CoreResult<&[Value]> {
    match v {
        None | Some(Value::Null) => Ok(&[]),
        Some(Value::Array(a)) => Ok(a),
        Some(_) => Err(invalid("expected a list")),
    }
}

fn str_of<'a>(v: &'a Value, keys: &[&str]) -> &'a str {
    keys.iter()
        .find_map(|k| v.get(*k).and_then(Value::as_str))
        .unwrap_or("")
        .trim()
}

/// JSON numbers or numeric strings.
fn num_of(v: &Value, keys: &[&str]) -> CoreResult<f64> {
    let x = keys
        .iter()
        .find_map(|k| v.get(*k))
        .ok_or_else(|| invalid(format!("missing field {}", keys[0])))?;
    match x {
        Value::Number(n) => n.as_f64().ok_or_else(|| invalid("invalid number")),
        Value::String(s) => parse_decimal(s),
        _ => Err(invalid("invalid number")),
    }
}

// ---------- shared validation ----------

/// Parses `1'234.50`, `1’234.50`, `1 234,50`, `1.234,50`, `-12.5`.
/// The last of `.`/`,` is the decimal separator; a lone `,` is decimal.
fn parse_decimal(s: &str) -> CoreResult<f64> {
    let t: String = s
        .chars()
        .filter(|c| !matches!(c, '\'' | '’' | '‘' | ' ' | '\u{a0}' | '\u{202f}'))
        .collect();
    let normalized = match (t.rfind('.'), t.rfind(',')) {
        (Some(d), Some(c)) if c > d => t.replace('.', "").replace(',', "."),
        (Some(_), Some(_)) => t.replace(',', ""),
        (None, Some(_)) => t.replace(',', "."),
        _ => t,
    };
    normalized
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite() && n.abs() < 1e12)
        .ok_or_else(|| invalid(format!("invalid number '{s}'")))
}

fn cents(x: f64) -> i64 {
    (x * 100.0).round() as i64
}

fn currency(s: &str) -> CoreResult<String> {
    let c = if s.is_empty() { "CHF" } else { s };
    match c.to_ascii_uppercase().as_str() {
        "CHF" => Ok("CHF".into()),
        "EUR" => Ok("EUR".into()),
        _ => Err(invalid(format!("unsupported currency '{c}'"))),
    }
}

fn valid_isin(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 12
        && b[..2].iter().all(u8::is_ascii_uppercase)
        && b[2..11]
            .iter()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        && b[11].is_ascii_digit()
}

fn build_holding(
    isin: &str,
    name: &str,
    quantity: f64,
    price: f64,
    value: f64,
    cur: &str,
) -> CoreResult<FinpensionHolding> {
    let isin = isin.to_ascii_uppercase();
    if !valid_isin(&isin) {
        return Err(invalid(format!("invalid ISIN '{isin}'")));
    }
    if name.is_empty() {
        return Err(invalid(format!("missing name for {isin}")));
    }
    if quantity < 0.0 || price < 0.0 || value < 0.0 {
        return Err(invalid(format!("negative figures for {isin}")));
    }
    Ok(FinpensionHolding {
        isin,
        name: name.into(),
        quantity,
        price_cents: cents(price),
        value_cents: cents(value),
        currency: currency(cur)?,
    })
}

fn build_transaction(
    date: &str,
    description: &str,
    amount: f64,
    cur: &str,
    reference: &str,
) -> CoreResult<FinpensionTransaction> {
    Ok(FinpensionTransaction {
        date: parse_date(date)?,
        description: description.into(),
        amount_cents: cents(amount),
        currency: currency(cur)?,
        reference: reference.into(),
    })
}

/// `YYYY-MM-DD` (a time suffix is ignored) or `DD.MM.YYYY`.
fn parse_date(s: &str) -> CoreResult<String> {
    let s = s.trim();
    let parsed = s
        .get(..10)
        .and_then(|p| jiff::civil::Date::strptime("%Y-%m-%d", p).ok())
        .or_else(|| jiff::civil::Date::strptime("%d.%m.%Y", s).ok());
    parsed
        .map(crate::dates::iso)
        .ok_or_else(|| invalid(format!("invalid date '{s}'")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOLDINGS_CSV: &str = "\u{feff}ISIN;Name;Anzahl;Kurs;Marktwert;Währung\n\
CH0000000011;\"Synthetic Equity Fund; World\";12.5;100.20;1'252.50;CHF\n\
IE0000000022;Synthetic Bond Fund;3;49.99;149.97;EUR\n";

    const TX_CSV: &str = "Datum,Beschreibung,Betrag,Währung,Referenz\n\
15.03.2026,\"Einzahlung Säule 3a, März\",\"6'000.00\",CHF,R-1\n\
2026-03-31,Verwaltungsgebühr,-12.34,CHF,R-2\n";

    const TRANS_REPORT_CSV: &str = "Date;Category;\"Asset Name\";ISIN;\"Number of Shares\";\"Asset Currency\";\"Currency Rate\";\"Asset Price in CHF\";\"Cash Flow\";Balance\n\
2024-01-15;Buy;\"Swisscanto (CH) IPF I\";CH0117044948;1.234;CHF;1.0;150.00;-185.10;1200.00\n\
2024-01-20;Deposit;\"\";\"\";;CHF;1.0;;500.00;1700.00\n";

    #[test]
    fn transaktions_report_csv_with_isin() {
        let export = parse_export(TRANS_REPORT_CSV).unwrap();
        assert!(export.holdings.is_empty());
        assert_eq!(export.transactions.len(), 2);

        let t1 = &export.transactions[0];
        assert_eq!(t1.date, "2024-01-15");
        assert_eq!(t1.description, "Buy: Swisscanto (CH) IPF I");
        assert_eq!(t1.amount_cents, -18510);
        assert_eq!(t1.currency, "CHF");
        assert_eq!(t1.reference, "CH0117044948");

        let t2 = &export.transactions[1];
        assert_eq!(t2.date, "2024-01-20");
        assert_eq!(t2.description, "Deposit");
        assert_eq!(t2.amount_cents, 50000);
        assert_eq!(t2.currency, "CHF");
    }

    #[test]
    fn holdings_csv() {
        let h = parse_holdings_csv(HOLDINGS_CSV).unwrap();
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].isin, "CH0000000011");
        assert_eq!(h[0].name, "Synthetic Equity Fund; World");
        assert_eq!(h[0].quantity, 12.5);
        assert_eq!(h[0].price_cents, 10020);
        assert_eq!(h[0].value_cents, 125250);
        assert_eq!(h[0].currency, "CHF");
        assert_eq!(h[1].currency, "EUR");
    }

    #[test]
    fn transactions_csv_and_ids() {
        let t = parse_transactions_csv(TX_CSV).unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].date, "2026-03-15");
        assert_eq!(t[0].description, "Einzahlung Säule 3a, März");
        assert_eq!(t[0].amount_cents, 600000);
        assert_eq!(t[1].amount_cents, -1234);
        let rem = remote_transactions("depot-1", &t);
        assert_eq!(rem.len(), 2);
        assert_ne!(rem[0].external_id, rem[1].external_id);
        assert_eq!(
            rem[0].external_id,
            remote_transactions("depot-1", &t)[0].external_id
        );
        assert!(rem[0].external_id.starts_with('c'));
    }

    #[test]
    fn json_export() {
        let json = r#"{
            "holdings": [{"isin": "CH0000000011", "name": "Eq", "quantity": 1, "price": 10.0, "value": 10.0}],
            "transactions": [{"date": "2026-01-01", "description": "Dep", "amount": 100.0}]
        }"#;
        let e = parse_export(json).unwrap();
        assert_eq!(e.holdings.len(), 1);
        assert_eq!(e.transactions.len(), 1);
    }

    #[test]
    fn export_detects_kind() {
        assert_eq!(parse_export(HOLDINGS_CSV).unwrap().holdings.len(), 2);
        assert_eq!(parse_export(TX_CSV).unwrap().transactions.len(), 2);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse_export("").is_err());
        assert!(parse_export("foo,bar\n1,2").is_err());
        assert!(parse_export("{\"holdings\": []}").is_err());
        assert!(parse_holdings_csv("ISIN,Name,Anzahl,Kurs,Marktwert\nBAD,X,1,1,1").is_err());
    }

    #[test]
    fn decimals() {
        assert_eq!(parse_decimal("1'234.56").unwrap(), 1234.56);
        assert_eq!(parse_decimal("1 234,56").unwrap(), 1234.56);
        assert_eq!(parse_decimal("12,50").unwrap(), 12.5);
        assert_eq!(parse_decimal("-42.10").unwrap(), -42.1);
    }

    #[test]
    fn urls_accepted() {
        assert!(validate_finpension_url("https://finpension.ch").is_ok());
        assert!(validate_finpension_url("https://finpension.ch/app").is_ok());
        assert!(validate_finpension_url("https://app.finpension.ch/export.csv").is_ok());
        assert!(validate_finpension_url("https://3a.finpension.ch/api").is_ok());
        assert!(validate_finpension_url("https://accounts.finpension.ch:443/login").is_ok());
    }

    #[test]
    fn urls_rejected() {
        assert!(validate_finpension_url("http://finpension.ch").is_err());
        assert!(validate_finpension_url("https://evilfinpension.ch").is_err());
        assert!(validate_finpension_url("https://finpension.ch.evil.com").is_err());
        assert!(validate_finpension_url("https://user:pass@finpension.ch").is_err());
        assert!(validate_finpension_url("https://finpension.ch:8443").is_err());
        assert!(validate_finpension_url("https://1.2.3.4/finpension.ch").is_err());
        assert!(validate_finpension_url("https://finpension.ch/\\path").is_err());
        assert!(validate_finpension_url("https://finpension.ch/ hello").is_err());
    }

    #[test]
    fn identical_transactions_get_distinct_ids() {
        let t = FinpensionTransaction {
            date: "2026-01-01".into(),
            description: "Same".into(),
            amount_cents: 1000,
            currency: "CHF".into(),
            reference: "".into(),
        };
        let rem = remote_transactions("depot-1", &[t.clone(), t]);
        assert_eq!(rem.len(), 2);
        assert_ne!(rem[0].external_id, rem[1].external_id);
    }

    #[test]
    fn to_fetch_creates_depot_and_cash_accounts_per_currency() {
        let export = parse_export(HOLDINGS_CSV).unwrap();
        let fetch = to_fetch(42, "FP", export);
        let refs: Vec<_> = fetch
            .accounts
            .iter()
            .map(|a| a.external_ref.as_str())
            .collect();
        assert_eq!(
            refs,
            vec!["finpension-42-depot-CHF", "finpension-42-depot-EUR"]
        );
        let chf = fetch.accounts.iter().find(|a| a.currency == "CHF").unwrap();
        assert_eq!(chf.name, "FP Depot (CHF)");
        assert_eq!(chf.balance_cents, 125250);
    }

    #[test]
    fn import_creates_depot_and_cash_accounts_per_currency() {
        let dir = crate::test_support::TempDir::new("girocrab-finpension-import");
        let v = Vault::open(
            dir.join("v.db").to_string_lossy().to_string(),
            vec![7u8; 32],
        )
        .unwrap();
        let conn = v
            .add_bank_connection(ConnectorKind::Finpension, "Finpension".into(), "3a".into())
            .unwrap();
        let json = r#"{"holdings":[{"isin":"CH0000000011","name":"Fund","quantity":2,"price":10.5,"value":21.0,"currency":"CHF"}],
            "transactions":[{"date":"2026-03-15","description":"Einzahlung","amount":100.0,"currency":"CHF","reference":"R1"}]}"#;
        let s = v
            .clone()
            .import_finpension_export(conn, json.as_bytes().to_vec())
            .unwrap();
        assert_eq!(s.transactions_imported, 1);
        let accounts = v.list_accounts().unwrap();
        assert_eq!(accounts.len(), 2);
        let depot = accounts
            .iter()
            .find(|a| a.kind == crate::models::AccountKind::Brokerage)
            .unwrap();
        assert_eq!(depot.balance_cents, 2100);
        assert_eq!(v.list_holdings(depot.id).unwrap().len(), 1);
        // importing again does not duplicate bookings
        let again = v
            .clone()
            .import_finpension_export(conn, json.as_bytes().to_vec())
            .unwrap();
        assert_eq!(again.transactions_imported, 0);
        // garbage and wrong connector
        assert!(v
            .clone()
            .import_finpension_export(conn, b"x".to_vec())
            .is_err());
    }

    #[test]
    fn phone_normalization_and_country_code() {
        assert_eq!(normalize_phone("079 123 45 67"), "+41791234567");
        assert_eq!(normalize_phone("+41 79 123 45 67"), "+41791234567");
        assert_eq!(normalize_phone("0049 170 1234567"), "+491701234567");
        assert_eq!(normalize_phone("+43 664 1234567"), "+436641234567");

        assert_eq!(extract_country_code("+41791234567"), Some("CH"));
        assert_eq!(extract_country_code("+491701234567"), Some("DE"));
        assert_eq!(extract_country_code("+436641234567"), Some("AT"));
        assert_eq!(extract_country_code("+4231234567"), Some("LI"));
        assert_eq!(extract_country_code("+12025550123"), Some("US"));
        assert_eq!(extract_country_code("invalid"), None);
    }

    fn mock_portfolios_json() -> String {
        r#"[
          {
            "id": 1001,
            "number": 1,
            "performance": {
              "current_value": 15000.50,
              "current_profit_value": 500.00,
              "deposits_ytd": 7000.00
            },
            "strategy": {
              "categories": [
                {
                  "positions": [
                    {
                      "asset_name": "Swiss Equity Fund",
                      "isin": "CH0117044948"
                    }
                  ]
                }
              ]
            },
            "allocation_by_asset_class": [
              {
                "positions": [
                  {
                    "asset_name": "Cash",
                    "value": 5000.50,
                    "shares": 1.0
                  },
                  {
                    "asset_name": "Swiss Equity Fund",
                    "value": 10000.00,
                    "shares": 100.0
                  }
                ]
              }
            ],
            "transactions": [
              {
                "id": 5001,
                "date": "2026-01-15",
                "type": "deposit",
                "amount": 7000.00
              },
              {
                "id": 5002,
                "date": "2026-01-31",
                "type": "fee",
                "amount": 12.50
              }
            ]
          },
          {
            "id": 1002,
            "number": 2,
            "performance": {
              "current_value": 2000.00,
              "current_profit_value": 0,
              "deposits_ytd": 2000.00
            },
            "strategy": {
              "categories": []
            },
            "allocation_by_asset_class": [
              {
                "positions": [
                  {
                    "asset_name": "Liquidität",
                    "value": 2000.00,
                    "shares": 1.0
                  }
                ]
              }
            ],
            "transactions": [
              {
                "id": 6001,
                "date": "2026-02-01",
                "type": "deposit",
                "amount": 2000.00
              }
            ]
          }
        ]"#
        .to_string()
    }

    fn serve_finpension_mock() -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let log_clone = log.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { break };
                let mut buf = [0u8; 8192];
                let len = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..len]).to_string();
                let first_line = req.lines().next().unwrap_or("").to_string();
                log_clone.lock().unwrap().push(first_line.clone());
                let (status, body) = if first_line.starts_with("POST /login2fa") {
                    ("200 OK", r#"{"token": "test-jwt-token-12345"}"#.to_string())
                } else if first_line.starts_with("POST /login") {
                    (
                        "200 OK",
                        r#"{"message": "OK", "method": "sms"}"#.to_string(),
                    )
                } else if first_line.starts_with("GET /portfolios") {
                    ("200 OK", mock_portfolios_json())
                } else if first_line.starts_with("POST /logout") {
                    ("200 OK", r#"{"message": "OK"}"#.to_string())
                } else {
                    ("404 Not Found", r#"{"error": "not found"}"#.to_string())
                };
                let resp = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = s.write_all(resp.as_bytes());
            }
        });
        (base, log)
    }

    #[tokio::test]
    async fn live_sync_two_phase_and_session_persistence() {
        let (base, log) = serve_finpension_mock();

        let mut connector = FinpensionConnector::with_base_urls(base.clone(), base.clone());
        let ctx = ConnectContext {
            connection_id: 1,
            login_name: "+41 79 123 45 67".into(),
            secret: SecretString::new("secret-password".into()),
            ..Default::default()
        };

        // Phase 1: begin() -> triggers /login and returns SMS Challenge
        let phase = connector.begin(&ctx).await.unwrap();
        match phase {
            Phase::Challenge(ch) => {
                assert_eq!(ch.kind, ChallengeKind::Code);
                assert!(ch.message.contains("+41791234567"));
            }
            _ => panic!("Expected Phase::Challenge"),
        }

        // Phase 2: respond() -> sends SMS code to /login2fa, receives token
        let phase = connector
            .respond(
                &ctx,
                ChallengeResponse::Code(SecretString::new("123456".into())),
            )
            .await
            .unwrap();
        assert!(matches!(phase, Phase::Ready));

        // Fetch: GET /portfolios -> extracts multiple accounts, holdings, and transactions
        let result = connector.fetch(&ctx).await.unwrap();
        assert_eq!(result.accounts.len(), 2);

        // Verify account 1
        let acc1 = &result.accounts[0];
        assert_eq!(acc1.external_ref, "finpension-1001");
        assert_eq!(acc1.name, "Finpension Portfolio 1");
        assert_eq!(acc1.balance_cents, 1500050);
        assert_eq!(acc1.currency, "CHF");
        assert_eq!(acc1.kind, AccountKind::Brokerage);

        // Verify account 2
        let acc2 = &result.accounts[1];
        assert_eq!(acc2.external_ref, "finpension-1002");
        assert_eq!(acc2.name, "Finpension Portfolio 2");
        assert_eq!(acc2.balance_cents, 200000);

        // Verify holdings
        assert_eq!(result.holdings.len(), 3);
        let cash_holding = result
            .holdings
            .iter()
            .find(|h| h.account_ref == "finpension-1001" && h.holding.isin.is_empty())
            .unwrap();
        assert_eq!(cash_holding.holding.name, "Liquidität");
        assert_eq!(cash_holding.holding.value_cents, 500050);

        let fund_holding = result
            .holdings
            .iter()
            .find(|h| h.holding.isin == "CH0117044948")
            .unwrap();
        assert_eq!(fund_holding.account_ref, "finpension-1001");
        assert_eq!(fund_holding.holding.name, "Swiss Equity Fund");
        assert_eq!(fund_holding.holding.quantity, 100.0);
        assert_eq!(fund_holding.holding.value_cents, 1000000);

        // Verify transactions with stable external_id
        assert_eq!(result.transactions.len(), 3);
        let t_dep = result
            .transactions
            .iter()
            .find(|t| t.external_id == "finpension-1001-5001")
            .unwrap();
        assert_eq!(t_dep.amount_cents, 700000);
        assert_eq!(t_dep.purpose, "Einzahlung Säule 3a");

        let t_fee = result
            .transactions
            .iter()
            .find(|t| t.external_id == "finpension-1001-5002")
            .unwrap();
        assert_eq!(t_fee.amount_cents, -1250);
        assert_eq!(t_fee.purpose, "Pauschalgebühr");

        // Verify state has stored token
        let saved_state = result.state.expect("state must be persisted");
        let decoded = saved_state.decode::<FinpensionState>().unwrap();
        assert_eq!(decoded.token.as_deref(), Some("test-jwt-token-12345"));

        // Follow-up sync: begin() with existing state immediately returns Phase::Ready without challenge
        let mut second_connector = FinpensionConnector::with_base_urls(base.clone(), base.clone());
        let ctx2 = ConnectContext {
            connection_id: 1,
            login_name: "+41 79 123 45 67".into(),
            secret: SecretString::new("".into()), // no password needed when session is valid!
            state: saved_state,
            ..Default::default()
        };

        let phase2 = second_connector.begin(&ctx2).await.unwrap();
        assert!(matches!(phase2, Phase::Ready));

        let requests = log.lock().unwrap().clone();
        assert!(requests.iter().any(|r| r.starts_with("POST /login")));
        assert!(requests.iter().any(|r| r.starts_with("POST /login2fa")));
        assert!(requests.iter().any(|r| r.starts_with("GET /portfolios")));
    }
}
