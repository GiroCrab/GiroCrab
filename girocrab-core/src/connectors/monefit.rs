//! Monefit SmartSaver (monefit.com) savings account: balance and bookings via
//! the JSON API of the SmartSaver web app (unofficial, may change at any time).
//!
//!  POST /v1/auth/login              {"identificator": e-mail, "password"}
//!                                   -> data.result.token (Bearer); data.result.mfa = true
//!                                      means a second factor, which is not supported
//!  GET  /v1/account/summary         -> data.result.closingBalance (balance, decimal string)
//!  GET  /v1/account/list?page=N&perPage=M
//!                                   -> data.result[]: { id, sum, time, balance, txnType,
//!                                      fee, description }, newest first; dateFrom/dateTo
//!                                      are ignored, so the list is read page by page
//!
//! `sum` is always positive, the direction comes from `txnType`. The `id` of a
//! list entry is not unique (all interest bookings share it), so the booking id
//! is built from the timestamp (to the second), the type and the amount.
//! Only `https://api-smartsaver.monefit.com` receives the password and the token.

use async_trait::async_trait;
use rust_decimal::Decimal;
use serde_json::{Value, json};

use super::{
    BankConnector, ChallengeResponse, ConnectContext, FetchResult, Phase, RemoteAccount,
    RemoteTransaction,
};
use crate::error::{CoreError, CoreResult};
use crate::i18n::{tr, trf};
use crate::models::AccountKind;
use crate::money::{cents_rounded, loose_decimal};
use crate::secret::SecretString;

const BASE: &str = "https://api-smartsaver.monefit.com/v1";
/// The one SmartSaver account of a login.
pub(crate) const ACCOUNT_REF: &str = "monefit:smartsaver";
/// Counterparty of every booking, as the bookings imported from other
/// programs carry it, so the first sync adopts them.
const COUNTERPARTY: &str = "Monefit SmartSaver";
/// `perPage` is not capped by the API; 500 keeps a page small enough.
const PAGE_SIZE: usize = 500;
/// Stop after this many pages (100 000 bookings) instead of looping forever.
const MAX_PAGES: usize = 200;

fn unexpected(status: u16) -> CoreError {
    CoreError::protocol(trf(
        "Monefit: Unerwartete Antwort der API (HTTP {0}).",
        &[&status],
    ))
}

// ------------------------------------------------------------------ parsing

/// `data.result` of an API answer.
fn result(v: &Value) -> &Value {
    &v["data"]["result"]
}

/// A required amount: a decimal string or number of the API ("4.79000000");
/// `None` when it is missing, blank or not a number.
fn amount(v: &Value) -> Option<Decimal> {
    match v {
        Value::String(s) if !s.trim().is_empty() => loose_decimal(s),
        Value::Number(n) => loose_decimal(&n.to_string()),
        _ => None,
    }
}

/// The optional fee: missing or blank is zero, anything else must be a number.
fn fee(v: &Value) -> Option<Decimal> {
    match v {
        Value::Null => Some(Decimal::ZERO),
        Value::String(s) if s.trim().is_empty() => Some(Decimal::ZERO),
        other => amount(other),
    }
}

/// Booking text and sign for a `txnType`. Outgoing types are guessed from the
/// API's naming; only INTEREST, DEPOSIT and BONUS were seen in real data.
fn classify(txn_type: &str) -> (i64, String) {
    match txn_type {
        "WITHDRAWAL" | "WITHDRAW" | "PAYOUT" => (-1, tr("Auszahlung").into()),
        "FEE" => (-1, tr("Gebühr").into()),
        "INTEREST" => (1, tr("Zinsen").into()),
        "DEPOSIT" => (1, tr("Einzahlung").into()),
        "BONUS" => (1, tr("Bonus").into()),
        other => (1, other.to_string()),
    }
}

/// One entry of `/account/list`; `None` for an entry without a usable date, amount
/// or type (a missing type must not turn into an incoming booking).
fn parse_booking(t: &Value) -> Option<RemoteTransaction> {
    let time = t["time"].as_str()?.trim();
    // "2026-07-09 12:34:56": the calendar day is the booking date
    let date = time
        .get(..10)
        .filter(|d| crate::dates::parse_iso(d).is_some())?;
    let txn_type = t["txnType"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    let sum = amount(&t["sum"])?;
    let fee = fee(&t["fee"])?;
    let (sign, booking_text) = classify(txn_type);
    // values from the network: an overflow rejects the booking instead of panicking
    let signed = Decimal::from(sign).checked_mul(sum)?.checked_sub(fee)?;
    let amount_cents = cents_rounded(signed)?;
    let description = t["description"].as_str().map(str::trim).unwrap_or_default();
    Some(RemoteTransaction {
        account_ref: ACCOUNT_REF.into(),
        // normalised amounts: "5.07000000", "5.07" and 5.07 give the same id. With the
        // fee (if any) the id covers everything that makes up the booking, so equal ids
        // are identical bookings and `fetch` numbering them does not depend on their order
        external_id: if fee.is_zero() {
            format!("monefit:{time}|{txn_type}|{}", sum.normalize())
        } else {
            format!(
                "monefit:{time}|{txn_type}|{}|{}",
                sum.normalize(),
                fee.normalize()
            )
        },
        booking_date: date.into(),
        value_date: date.into(),
        amount_cents,
        currency: "EUR".into(),
        counterparty: COUNTERPARTY.into(),
        purpose: if description.is_empty() {
            booking_text.clone()
        } else {
            description.to_string()
        },
        booking_text,
        ..Default::default()
    })
}

/// Bookings with the same id (same second, type, amount and fee) get `#1`,
/// `#2` … in the order of their purpose, not of the answer: if the API lists
/// them in another order, each keeps its number. The purpose is no part of
/// the id itself, so a reworded description never turns into new bookings.
fn number_identical(bookings: &mut [RemoteTransaction]) {
    let mut order: Vec<usize> = (0..bookings.len()).collect();
    order.sort_by(|&a, &b| {
        (&bookings[a].external_id, &bookings[a].purpose)
            .cmp(&(&bookings[b].external_id, &bookings[b].purpose))
    });
    let mut previous: Option<String> = None;
    let mut n = 0;
    for i in order {
        let base = bookings[i].external_id.clone();
        if previous.as_deref() == Some(base.as_str()) {
            n += 1;
            bookings[i].external_id = format!("{base}#{n}");
        } else {
            n = 0;
            previous = Some(base);
        }
    }
}

// ------------------------------------------------------------------ connector

#[derive(Default)]
pub struct MonefitConnector {
    client: Option<reqwest::Client>,
    token: SecretString,
}

impl MonefitConnector {
    pub fn new() -> Self {
        Self::default()
    }

    async fn get(&self, path: &str) -> CoreResult<Value> {
        let client = self.client.as_ref().ok_or(CoreError::SessionClosed)?;
        let resp = client
            .get(format!("{BASE}{path}"))
            .bearer_auth(self.token.expose())
            .header("Accept", "application/json")
            .send()
            .await?;
        let status = resp.status();
        let v = super::read_json_or_null(resp).await;
        if matches!(status.as_u16(), 401 | 403) {
            return Err(CoreError::Authentication {
                message: tr("Monefit: Sitzung abgelaufen. Bitte erneut abrufen.").into(),
            });
        }
        if !status.is_success() || v["data"].is_null() {
            return Err(unexpected(status.as_u16()));
        }
        Ok(v)
    }
}

#[async_trait]
impl BankConnector for MonefitConnector {
    async fn begin(&mut self, ctx: &ConnectContext) -> CoreResult<Phase> {
        let login = ctx.login_name.trim();
        if login.is_empty() || ctx.secret.expose().is_empty() {
            return Err(CoreError::Authentication {
                message: tr("Monefit: E-Mail-Adresse und Passwort erforderlich.").into(),
            });
        }
        let client = super::client_builder(std::time::Duration::from_secs(60))
            .user_agent(super::BROWSER_UA)
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let resp = client
            .post(format!("{BASE}/auth/login"))
            .header("Accept", "application/json")
            .json(&json!({ "identificator": login, "password": ctx.secret.expose() }))
            .send()
            .await?;
        let status = resp.status();
        let v = super::read_json_or_null(resp).await;
        if status.is_server_error() || status.as_u16() == 429 {
            return Err(CoreError::Network {
                message: trf(
                    "Monefit: Der Anmeldedienst antwortet nicht (HTTP {0}). Bitte später erneut versuchen.",
                    &[&status.as_u16()],
                ),
            });
        }
        if result(&v)["mfa"].as_bool() == Some(true) {
            return Err(CoreError::Authentication {
                message: tr("Monefit: Die Anmeldung verlangt einen zweiten Faktor; das wird noch nicht unterstützt.").into(),
            });
        }
        let token = result(&v)["token"].as_str().unwrap_or_default();
        if !status.is_success() || token.is_empty() {
            return Err(CoreError::Authentication {
                message: tr(
                    "Monefit: Anmeldung fehlgeschlagen. Bitte E-Mail-Adresse und Passwort prüfen.",
                )
                .into(),
            });
        }
        self.token = SecretString::new(token.to_string());
        self.client = Some(client);
        Ok(Phase::Ready)
    }

    async fn respond(&mut self, _ctx: &ConnectContext, _r: ChallengeResponse) -> CoreResult<Phase> {
        Ok(Phase::Ready)
    }

    async fn fetch(&mut self, ctx: &ConnectContext) -> CoreResult<FetchResult> {
        let summary = self.get("/account/summary").await?;
        let balance_cents = amount(&result(&summary)["closingBalance"])
            .and_then(cents_rounded)
            .ok_or_else(|| unexpected(200))?;

        let since = ctx.since_for(ACCOUNT_REF).to_string();
        let mut out = FetchResult::default();
        'pages: for page in 1..=MAX_PAGES {
            let v = self
                .get(&format!("/account/list?page={page}&perPage={PAGE_SIZE}"))
                .await?;
            let Some(items) = result(&v).as_array().cloned() else {
                return Err(unexpected(200));
            };
            for t in &items {
                let Some(b) = parse_booking(t) else {
                    return Err(CoreError::invalid(tr(
                        "Monefit: Unvollständige Buchung in der Antwort. Es wurden keine Daten übernommen.",
                    )));
                };
                // newest first: everything after the first older booking is older too
                if b.booking_date.as_str() < since.as_str() {
                    break 'pages;
                }
                out.transactions.push(b);
            }
            if items.len() < PAGE_SIZE {
                break;
            }
            if page == MAX_PAGES {
                return Err(CoreError::protocol(tr(
                    "Monefit: Die Umsatzliste wurde nicht vollständig geliefert. Bitte den Abruf erneut starten.",
                )));
            }
        }
        number_identical(&mut out.transactions);
        out.accounts.push(RemoteAccount {
            external_ref: ACCOUNT_REF.into(),
            name: "Monefit SmartSaver".into(),
            owner: String::new(),
            iban: String::new(),
            account_number: String::new(),
            currency: "EUR".into(),
            kind: AccountKind::Savings,
            balance_cents,
            balance_date: super::today(),
        });
        out.balances_reported.push(ACCOUNT_REF.into());
        out.accounts_complete = true;
        Ok(out)
    }

    async fn finish(&mut self, _ctx: &ConnectContext) {
        self.client = None;
        self.token = SecretString::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Invented entries in the shape of the API; no real data.
    fn entry(time: &str, txn_type: &str, sum: &str, fee: &str, desc: Option<&str>) -> Value {
        json!({ "id": "4", "sum": sum, "time": time, "balance": "1000.00000000",
                "txnType": txn_type, "fee": fee, "description": desc, "detail": null })
    }

    #[test]
    fn interest_booking_keeps_description_and_day() {
        let b = parse_booking(&entry(
            "2026-11-17 03:12:45",
            "INTEREST",
            "5.07493150",
            "0",
            Some("Earnings received"),
        ))
        .unwrap();
        assert_eq!(b.booking_date, "2026-11-17");
        assert_eq!(b.amount_cents, 507);
        assert_eq!(b.counterparty, "Monefit SmartSaver");
        assert_eq!(b.purpose, "Earnings received");
        assert_eq!(
            b.external_id,
            "monefit:2026-11-17 03:12:45|INTEREST|5.0749315"
        );
    }

    #[test]
    fn same_list_id_gives_different_booking_ids() {
        let a =
            parse_booking(&entry("2026-11-17 03:12:45", "INTEREST", "5.07", "0", None)).unwrap();
        let b =
            parse_booking(&entry("2026-11-18 03:12:45", "INTEREST", "5.07", "0", None)).unwrap();
        assert_ne!(a.external_id, b.external_id);
    }

    #[test]
    fn booking_id_uses_the_normalised_amount() {
        let a = parse_booking(&entry(
            "2026-11-17 03:12:45",
            "DEPOSIT",
            "10.00000000",
            "0",
            None,
        ))
        .unwrap();
        let mut numeric = entry("2026-11-17 03:12:45", "DEPOSIT", "", "0", None);
        numeric["sum"] = json!(10);
        let b = parse_booking(&numeric).unwrap();
        numeric["sum"] = json!(20);
        let c = parse_booking(&numeric).unwrap();
        assert_eq!(a.external_id, "monefit:2026-11-17 03:12:45|DEPOSIT|10");
        assert_eq!(a.external_id, b.external_id);
        assert_ne!(b.external_id, c.external_id);
    }

    #[test]
    fn booking_id_includes_a_fee() {
        let a = parse_booking(&entry(
            "2026-10-01 10:00:00",
            "WITHDRAWAL",
            "100",
            "1.50",
            None,
        ))
        .unwrap();
        let b = parse_booking(&entry(
            "2026-10-01 10:00:00",
            "WITHDRAWAL",
            "100",
            "2.00",
            None,
        ))
        .unwrap();
        assert_eq!(
            a.external_id,
            "monefit:2026-10-01 10:00:00|WITHDRAWAL|100|1.5"
        );
        assert_ne!(a.external_id, b.external_id);
    }

    #[test]
    fn identical_ids_are_numbered_by_purpose_not_by_answer_order() {
        let b = |purpose: &str| {
            let mut t = parse_booking(&entry(
                "2026-10-01 10:00:00",
                "BONUS",
                "5",
                "0",
                Some(purpose),
            ))
            .unwrap();
            t.purpose = purpose.into();
            t
        };
        let mut one = vec![b("Welcome bonus"), b("Referral bonus"), b("Cashback")];
        let mut two = vec![b("Cashback"), b("Welcome bonus"), b("Referral bonus")];
        number_identical(&mut one);
        number_identical(&mut two);
        let id = |list: &[RemoteTransaction], purpose: &str| {
            list.iter()
                .find(|t| t.purpose == purpose)
                .unwrap()
                .external_id
                .clone()
        };
        for purpose in ["Welcome bonus", "Referral bonus", "Cashback"] {
            assert_eq!(id(&one, purpose), id(&two, purpose));
        }
        assert_eq!(id(&one, "Cashback"), "monefit:2026-10-01 10:00:00|BONUS|5");
        assert_eq!(
            id(&one, "Referral bonus"),
            "monefit:2026-10-01 10:00:00|BONUS|5#1"
        );
    }

    #[test]
    fn a_fee_is_booked_as_fee() {
        let b = parse_booking(&entry("2026-10-01 10:00:00", "FEE", "1", "0", None)).unwrap();
        assert_eq!(
            (b.amount_cents, b.booking_text.as_str()),
            (-100, tr("Gebühr"))
        );
    }

    #[test]
    fn missing_or_blank_amounts_are_rejected_but_a_missing_fee_is_zero() {
        assert!(parse_booking(&entry("2026-10-01 10:00:00", "DEPOSIT", "", "0", None)).is_none());
        assert!(
            parse_booking(&json!({ "time": "2026-10-01 10:00:00", "txnType": "DEPOSIT" }))
                .is_none()
        );
        assert!(parse_booking(&entry("2026-10-01 10:00:00", "DEPOSIT", "1", "x", None)).is_none());
        let b = parse_booking(
            &json!({ "time": "2026-10-01 10:00:00", "txnType": "DEPOSIT", "sum": "0" }),
        )
        .unwrap();
        assert_eq!(b.amount_cents, 0);
        assert_eq!(amount(&Value::Null), None);
    }

    #[test]
    fn withdrawal_is_negative_and_fee_is_deducted() {
        let b = parse_booking(&entry(
            "2026-10-01 10:00:00",
            "WITHDRAWAL",
            "100.00",
            "1.50",
            None,
        ))
        .unwrap();
        assert_eq!(b.amount_cents, -10150);
        assert_eq!(b.purpose, tr("Auszahlung"));
    }

    #[test]
    fn unknown_type_keeps_its_name_and_counts_as_incoming() {
        let b = parse_booking(&entry("2026-10-01 10:00:00", "CASHBACK", "2", "", None)).unwrap();
        assert_eq!((b.amount_cents, b.booking_text.as_str()), (200, "CASHBACK"));
    }

    #[test]
    fn entry_without_time_amount_or_type_is_rejected() {
        assert!(parse_booking(&json!({ "sum": "1.00", "txnType": "DEPOSIT" })).is_none());
        assert!(
            parse_booking(
                &json!({ "time": "2026-10-01 10:00:00", "sum": "abc", "txnType": "DEPOSIT" })
            )
            .is_none()
        );
        assert!(
            parse_booking(&json!({ "time": "kaputt", "sum": "1.00", "txnType": "DEPOSIT" }))
                .is_none()
        );
        // missing, null or blank type: no silent incoming booking
        assert!(parse_booking(&json!({ "time": "2026-10-01 10:00:00", "sum": "1.00" })).is_none());
        assert!(
            parse_booking(
                &json!({ "time": "2026-10-01 10:00:00", "sum": "1.00", "txnType": null })
            )
            .is_none()
        );
        assert!(parse_booking(&entry("2026-10-01 10:00:00", "", "1", "0", None)).is_none());
        assert!(parse_booking(&entry("2026-10-01 10:00:00", "  ", "1", "0", None)).is_none());
    }

    #[test]
    fn overflowing_amount_is_rejected_without_panic() {
        // -MAX - fee overflows Decimal; the booking is dropped instead of panicking
        let max = Decimal::MAX.to_string();
        assert!(
            parse_booking(&entry("2026-10-01 10:00:00", "WITHDRAWAL", &max, "1", None)).is_none()
        );
    }
}
