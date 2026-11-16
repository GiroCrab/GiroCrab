//! PayPal – classic NVP API with an API signature (prototype, #156).
//!
//! For accounts without access to the REST "Transaction Search" permission:
//! the user creates API credentials (API username, API password, signature)
//! under PayPal › Kontoeinstellungen › API-Zugriff.
//!
//!  POST https://api-3t.paypal.com/nvp   (form-encoded, answer form-encoded)
//!    METHOD=GetBalance&RETURNALLCURRENCIES=1        -> L_AMTn / L_CURRENCYCODEn
//!    METHOD=TransactionSearch&STARTDATE&ENDDATE     -> L_TIMESTAMPn, L_NAMEn, L_NETAMTn …
//!
//! `TransactionSearch` returns at most 100 rows and cannot page; a full answer
//! carries warning 11002. The window is then halved until it fits.

use std::collections::HashMap;

use chrono::{DateTime, Duration, NaiveDate, Utc};

use super::{http_client, RemoteAccount, RemoteTransaction};
use crate::error::{CoreError, CoreResult};
use crate::models::AccountKind;
use crate::money::parse_cents;
use crate::secret::SecretString;

const VERSION: &str = "204.0";
/// Warning code of a TransactionSearch answer cut at 100 rows.
const TRUNCATED: &str = "11002";
/// Window of the first request per step; halved on truncation.
const WINDOW_DAYS: i64 = 31;

pub(crate) struct NvpCredentials {
    pub user: String,
    pub password: SecretString,
    pub signature: SecretString,
}

pub(crate) struct NvpClient {
    client: reqwest::Client,
    endpoint: String,
    creds: NvpCredentials,
}

/// Decoded NVP answer (`KEY=value&…`).
pub(crate) type Nvp = HashMap<String, String>;

pub(crate) fn parse_nvp(body: &str) -> Nvp {
    url::form_urlencoded::parse(body.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

fn ack_ok(r: &Nvp) -> bool {
    matches!(
        r.get("ACK").map(String::as_str),
        Some("Success") | Some("SuccessWithWarning")
    )
}

fn error_codes(r: &Nvp) -> Vec<&str> {
    (0..)
        .map_while(|i| r.get(&format!("L_ERRORCODE{i}")))
        .map(String::as_str)
        .collect()
}

fn error_of(r: &Nvp) -> CoreError {
    let code = r.get("L_ERRORCODE0").cloned().unwrap_or_default();
    let message = r
        .get("L_LONGMESSAGE0")
        .or(r.get("L_SHORTMESSAGE0"))
        .cloned()
        .unwrap_or_else(|| "PayPal NVP error".into());
    // 10002: "Security header is not valid" / wrong API credentials.
    if code == "10002" || code == "10008" {
        CoreError::Authentication { message }
    } else {
        CoreError::Bank { code, message }
    }
}

impl NvpClient {
    pub fn new(sandbox: bool, creds: NvpCredentials) -> CoreResult<Self> {
        Ok(NvpClient {
            client: http_client()?,
            endpoint: if sandbox {
                "https://api-3t.sandbox.paypal.com/nvp".into()
            } else {
                "https://api-3t.paypal.com/nvp".into()
            },
            creds,
        })
    }

    async fn call(&self, method: &str, params: &[(&str, String)]) -> CoreResult<Nvp> {
        let mut form: Vec<(&str, &str)> = vec![
            ("METHOD", method),
            ("VERSION", VERSION),
            ("USER", &self.creds.user),
            ("PWD", self.creds.password.expose()),
            ("SIGNATURE", self.creds.signature.expose()),
        ];
        form.extend(params.iter().map(|(k, v)| (*k, v.as_str())));
        let resp = self.client.post(&self.endpoint).form(&form).send().await?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let r = parse_nvp(&body);
        if !status.is_success() && r.is_empty() {
            return Err(CoreError::Bank {
                code: status.as_u16().to_string(),
                message: "PayPal NVP error".into(),
            });
        }
        if !ack_ok(&r) {
            return Err(error_of(&r));
        }
        Ok(r)
    }

    /// Validates the credentials (cheap call, also used by `begin`).
    pub async fn balances(&self, owner: &str) -> CoreResult<Vec<RemoteAccount>> {
        let r = self
            .call("GetBalance", &[("RETURNALLCURRENCIES", "1".into())])
            .await?;
        Ok(accounts_from_balance(&r, owner))
    }

    pub async fn transactions(&self, since: NaiveDate) -> CoreResult<Vec<RemoteTransaction>> {
        let now = Utc::now();
        let mut out = Vec::new();
        let mut start = since
            .and_hms_opt(0, 0, 0)
            .map(|d| d.and_utc())
            .unwrap_or(now - Duration::days(90));
        while start < now {
            let mut end = (start + Duration::days(WINDOW_DAYS)).min(now);
            loop {
                let r = self.search(start, end).await?;
                let truncated = error_codes(&r).contains(&TRUNCATED);
                // Halve until the window fits; below one hour take what came.
                if truncated && end - start > Duration::hours(1) {
                    end = start + (end - start) / 2;
                    continue;
                }
                out.extend(transactions_from_search(&r));
                break;
            }
            start = end + Duration::seconds(1);
        }
        // Windows touch at the boundary second: drop duplicates.
        let mut seen = std::collections::HashSet::new();
        out.retain(|t| seen.insert((t.account_ref.clone(), t.external_id.clone())));
        Ok(out)
    }

    async fn search(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> CoreResult<Nvp> {
        let fmt = |d: DateTime<Utc>| d.format("%Y-%m-%dT%H:%M:%SZ").to_string();
        self.call(
            "TransactionSearch",
            &[("STARTDATE", fmt(start)), ("ENDDATE", fmt(end))],
        )
        .await
    }
}

pub(crate) fn accounts_from_balance(r: &Nvp, owner: &str) -> Vec<RemoteAccount> {
    let mut out = Vec::new();
    for i in 0.. {
        let Some(cur) = r.get(&format!("L_CURRENCYCODE{i}")) else {
            break;
        };
        let amount = r
            .get(&format!("L_AMT{i}"))
            .and_then(|a| parse_cents(a).ok())
            .unwrap_or(0);
        out.push(RemoteAccount {
            external_ref: format!("paypal:{cur}"),
            // The first entry is the primary currency.
            name: if i == 0 {
                "PayPal".into()
            } else {
                format!("PayPal {cur}")
            },
            owner: owner.to_string(),
            iban: String::new(),
            account_number: owner.to_string(),
            currency: cur.clone(),
            kind: AccountKind::PaymentService,
            balance_cents: amount,
            balance_date: super::today(),
        });
    }
    out
}

pub(crate) fn transactions_from_search(r: &Nvp) -> Vec<RemoteTransaction> {
    let mut out = Vec::new();
    for i in 0.. {
        let Some(id) = r.get(&format!("L_TRANSACTIONID{i}")) else {
            break;
        };
        let get = |k: &str| r.get(&format!("{k}{i}")).cloned().unwrap_or_default();
        let status = get("L_STATUS");
        let kind = get("L_TYPE");
        // Failed or undone entries and mere authorisations move no money.
        if matches!(
            status.as_str(),
            "Denied" | "Reversed" | "Voided" | "Canceled" | "Failed"
        ) || matches!(kind.as_str(), "Authorization" | "Order")
        {
            continue;
        }
        // Net = gross + fee: what actually moves the balance.
        let net = parse_cents(&get("L_NETAMT")).or_else(|_| parse_cents(&get("L_AMT")));
        let Ok(amount_cents) = net else { continue };
        let fee = parse_cents(&get("L_FEEAMT")).unwrap_or(0);
        let date: String = get("L_TIMESTAMP").chars().take(10).collect();
        let currency = {
            let c = get("L_CURRENCYCODE");
            if c.is_empty() {
                "EUR".into()
            } else {
                c
            }
        };
        let name = get("L_NAME");
        let email = get("L_EMAIL");
        let mut purpose = if name.is_empty() || email.is_empty() {
            String::new()
        } else {
            email.clone()
        };
        if fee != 0 {
            let f = crate::money::format_cents(fee, currency.clone());
            purpose = if purpose.is_empty() {
                format!("Gebühr {f}")
            } else {
                format!("{purpose} · Gebühr {f}")
            };
        }
        out.push(RemoteTransaction {
            account_ref: format!("paypal:{currency}"),
            external_id: id.clone(),
            booking_date: date.clone(),
            value_date: date,
            amount_cents,
            currency,
            counterparty: if name.is_empty() { email } else { name },
            counterparty_iban: String::new(),
            purpose,
            booking_text: kind,
            ..Default::default()
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balance_lists_every_currency() {
        let r = parse_nvp(
            "L_AMT0=123%2e45&L_AMT1=-5%2e00&L_CURRENCYCODE0=EUR&L_CURRENCYCODE1=USD\
             &TIMESTAMP=2026%2d11%2d16T10%3a00%3a00Z&ACK=Success&VERSION=204%2e0",
        );
        assert!(ack_ok(&r));
        let a = accounts_from_balance(&r, "x");
        assert_eq!(a.len(), 2);
        assert_eq!(
            (a[0].external_ref.as_str(), a[0].name.as_str()),
            ("paypal:EUR", "PayPal")
        );
        assert_eq!(a[0].balance_cents, 12345);
        assert_eq!(
            (a[1].name.as_str(), a[1].balance_cents),
            ("PayPal USD", -500)
        );
    }

    #[test]
    fn search_rows_book_net_and_skip_denied() {
        let r = parse_nvp(
            "L_TIMESTAMP0=2026%2d11%2d10T08%3a15%3a00Z&L_TYPE0=Payment&L_EMAIL0=shop%40example%2ecom\
             &L_NAME0=Beispiel%20Shop&L_TRANSACTIONID0=AAA&L_STATUS0=Completed&L_AMT0=%2d20%2e00\
             &L_FEEAMT0=0%2e00&L_NETAMT0=%2d20%2e00&L_CURRENCYCODE0=EUR\
             &L_TIMESTAMP1=2026%2d11%2d09T08%3a00%3a00Z&L_TYPE1=Payment&L_NAME1=Erika\
             &L_TRANSACTIONID1=BBB&L_STATUS1=Completed&L_AMT1=100%2e00&L_FEEAMT1=%2d2%2e85\
             &L_NETAMT1=97%2e15&L_CURRENCYCODE1=EUR\
             &L_TIMESTAMP2=2026%2d11%2d08T08%3a00%3a00Z&L_TYPE2=Payment&L_NAME2=Weg\
             &L_TRANSACTIONID2=CCC&L_STATUS2=Denied&L_AMT2=%2d5%2e00&L_NETAMT2=%2d5%2e00\
             &L_CURRENCYCODE2=EUR&ACK=Success",
        );
        let t = transactions_from_search(&r);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].external_id, "AAA");
        assert_eq!(t[0].booking_date, "2026-11-10");
        assert_eq!(t[0].amount_cents, -2000);
        assert_eq!(t[0].counterparty, "Beispiel Shop");
        assert_eq!(t[0].purpose, "shop@example.com");
        assert_eq!(t[0].account_ref, "paypal:EUR");
        assert_eq!(t[1].amount_cents, 9715);
        assert!(t[1].purpose.starts_with("Gebühr"), "{}", t[1].purpose);
    }

    #[test]
    fn truncation_warning_and_auth_error_are_recognised() {
        let r =
            parse_nvp("ACK=SuccessWithWarning&L_ERRORCODE0=11002&L_SHORTMESSAGE0=Search%20warning");
        assert!(ack_ok(&r));
        assert_eq!(error_codes(&r), vec![TRUNCATED]);
        let r = parse_nvp(
            "ACK=Failure&L_ERRORCODE0=10002&L_LONGMESSAGE0=Security%20header%20is%20not%20valid",
        );
        assert!(!ack_ok(&r));
        assert!(matches!(error_of(&r), CoreError::Authentication { .. }));
    }
}
