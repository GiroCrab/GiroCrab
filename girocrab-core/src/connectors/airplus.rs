//! AirPlus Corporate Card (#1143): booked card bookings from the business travel portal.
//! Login (OpenID Connect + SMS TAN) and every request stay inside the embedded browser
//! (WKWebView); only the whitelisted booking fields and two scalar values cross the bridge.
//! No cookie, token, password or TAN enters this module. The portal's interface is
//! undocumented and may change.
use std::sync::Arc;

use serde_json::Value;

use super::{FetchResult, RemoteAccount, RemoteTransaction, SyncSummary};
use crate::{
    error::{CoreError, CoreResult},
    i18n::tr,
    models::{AccountKind, ConnectorKind},
    vault::Vault,
};

/// A fetch with any broken booking is rejected as a whole rather than imported in part.
fn invalid() -> CoreError {
    CoreError::InvalidInput {
        message: tr("AirPlus: Unvollständige oder ungültige Portalantwort. Es wurden keine Daten übernommen.").into(),
    }
}

fn text<'a>(v: &'a Value, key: &str) -> CoreResult<&'a str> {
    v[key]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(invalid)
}

fn optional<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().map(str::trim).unwrap_or_default()
}

/// `2026-03-04T00:00:00+00:00` -> `2026-03-04`; anything that is not a date is refused.
fn date(v: &Value, key: &str) -> CoreResult<String> {
    let s = text(v, key)?.get(..10).ok_or_else(invalid)?;
    jiff::civil::Date::strptime("%Y-%m-%d", s).map_err(|_| invalid())?;
    Ok(s.into())
}

/// The portal reports charges as positive numbers; the account books them as outflow.
/// A refund is expected negative and becomes a credit (sign of refunds not yet verified).
fn booking_cents(v: &Value, key: &str) -> CoreResult<i64> {
    let cents = crate::money::json_number_to_cents_rounded(&v[key]).map_err(|_| invalid())?;
    cents.checked_neg().ok_or_else(invalid)
}

fn currency(v: &Value, key: &str) -> CoreResult<String> {
    let c = text(v, key)?;
    (c.len() == 3 && c.bytes().all(|b| b.is_ascii_uppercase()))
        .then(|| c.to_owned())
        .ok_or_else(invalid)
}

/// Booked card bookings (`GetAccountCardTransactionListTransaction`), flattened by the page
/// script. `AuthorizationNo` of the portal is stable per booking: the de-duplication key.
fn transactions(root: &Value, account: &RemoteAccount) -> CoreResult<Vec<RemoteTransaction>> {
    let rows = root["transactions"].as_array().ok_or_else(invalid)?;
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(rows.len());
    for t in rows {
        let id = text(t, "id")?;
        if !seen.insert(id.to_owned()) {
            continue; // a page boundary may deliver a booking twice
        }
        let cur = currency(t, "currency")?;
        if cur != account.currency {
            return Err(invalid());
        }
        out.push(RemoteTransaction {
            account_ref: account.external_ref.clone(),
            external_id: format!("airplus:{id}"),
            booking_date: date(t, "bookingDate")?,
            value_date: date(t, "salesDate")?,
            amount_cents: booking_cents(t, "amount")?,
            currency: cur,
            counterparty: text(t, "merchant")?.into(),
            purpose: optional(t, "city").into(),
            booking_text: optional(t, "type").into(),
            ..Default::default()
        });
    }
    Ok(out)
}

fn snapshot(json: &str) -> CoreResult<FetchResult> {
    let root: Value = serde_json::from_str(json).map_err(|_| invalid())?;
    if root["version"].as_i64() != Some(1) {
        return Err(invalid());
    }
    let card = &root["card"];
    let mut account = RemoteAccount {
        external_ref: format!("airplus:{}", text(card, "id")?),
        name: tr("AirPlus Corporate Card").into(),
        owner: String::new(),
        iban: String::new(),
        account_number: optional(card, "maskedNumber").into(),
        currency: currency(card, "currency")?,
        kind: AccountKind::CreditCard,
        balance_cents: 0,
        balance_date: String::new(),
    };
    let mut result = FetchResult::default();
    // Used credit limit as the (negative) card balance. A limit that was not read is
    // reported as unknown, never as 0 (#369).
    let used = &card["limitUsed"];
    if used.is_null() {
        result.balances_unknown.push(account.external_ref.clone());
    } else {
        let cents = crate::money::json_number_to_cents_rounded(used).map_err(|_| invalid())?;
        account.balance_cents = cents.checked_neg().ok_or_else(invalid)?;
        account.balance_date = date(card, "balanceDate")?;
        result.balances_reported.push(account.external_ref.clone());
    }
    result.transactions = transactions(&root, &account)?;
    result.accounts.push(account);
    Ok(result)
}

#[uniffi::export]
impl Vault {
    /// Imports the snapshot the page script collected from the signed-in portal.
    pub fn import_airplus_snapshot(
        self: Arc<Self>,
        connection_id: i64,
        snapshot_json: String,
    ) -> CoreResult<SyncSummary> {
        if self.portal_connection(connection_id)?.0 != ConnectorKind::AirPlus {
            return Err(invalid());
        }
        crate::banking_gate::check_connector("airplus")?;
        let summary = super::store_fetch(
            &self,
            ConnectorKind::AirPlus,
            connection_id,
            snapshot(&snapshot_json)?,
        )?;
        let _ = self.checkpoint();
        Ok(summary)
    }
}

pub(crate) fn bank_descriptor(kind: ConnectorKind) -> super::BankDescriptor {
    super::BankDescriptor {
        kind,
        name: "AirPlus".into(),
        protocol: tr("Direkt-Login (Web-Session)").into(),
        bank_code: String::new(),
        sf_symbol: "creditcard".into(),
        login_label: tr("Zugangsbezeichnung").into(),
        secret_label: tr("(Anmeldung im Browser-Fenster)").into(),
        required_settings: vec![],
        note: tr("AirPlus Corporate Card: gebuchte Umsätze und verbrauchtes Limit. Anmeldung im Browserfenster mit SMS-TAN. Inoffizielle Portal-Schnittstelle; Änderungen und Einschränkungen durch Nutzungsbedingungen sind möglich. Vorgemerkte Umsätze und Abrechnungs-PDFs werden noch nicht übernommen.").into(),
    }
}

pub struct WebOnlyConnector;

#[async_trait::async_trait]
impl super::BankConnector for WebOnlyConnector {
    async fn begin(&mut self, _ctx: &super::ConnectContext) -> CoreResult<super::Phase> {
        Err(CoreError::BrowserLoginRequired {
            message: tr("Bitte bei AirPlus im Browserfenster anmelden und die Umsätze abrufen.")
                .into(),
        })
    }
    async fn respond(
        &mut self,
        ctx: &super::ConnectContext,
        _response: super::ChallengeResponse,
    ) -> CoreResult<super::Phase> {
        self.begin(ctx).await
    }
    async fn fetch(&mut self, _ctx: &super::ConnectContext) -> CoreResult<FetchResult> {
        Err(CoreError::BrowserLoginRequired {
            message: tr("Bitte bei AirPlus im Browserfenster abrufen.").into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Synthetic data only: invented merchants, ids and amounts.
    fn fixture() -> Value {
        let tx = |id: &str, amount: f64, day: &str| {
            json!({"id": id, "bookingDate": format!("{day}T00:00:00+00:00"),
            "salesDate": "2026-03-03T00:00:00+00:00", "merchant": "BEISPIEL TAXI 1",
            "city": "MUSTERSTADT", "amount": amount, "currency": "EUR", "type": "GroundTransportation"})
        };
        json!({"version": 1,
            "card": {"id": "170000000001", "maskedNumber": "531954******0000", "currency": "EUR",
                "limitUsed": 123.45, "balanceDate": "2026-11-22"},
            "transactions": [tx("a-1", 42.30, "2026-03-04"), tx("a-2", 18.0, "2026-03-03"),
                tx("a-3", -5.5, "2026-03-03")]})
    }

    #[test]
    fn maps_card_balance_and_bookings() {
        let r = snapshot(&fixture().to_string()).unwrap();
        assert_eq!(r.accounts.len(), 1);
        let a = &r.accounts[0];
        assert_eq!(a.kind, AccountKind::CreditCard);
        assert_eq!(a.external_ref, "airplus:170000000001");
        assert_eq!(a.balance_cents, -12345);
        assert_eq!(a.balance_date, "2026-11-22");
        assert_eq!(r.balances_reported, vec![a.external_ref.clone()]);
        assert!(r.balances_unknown.is_empty());
        let amounts: Vec<i64> = r.transactions.iter().map(|t| t.amount_cents).collect();
        assert_eq!(amounts, vec![-4230, -1800, 550]); // refund = credit
        assert_eq!(r.transactions[0].external_id, "airplus:a-1");
        assert_eq!(r.transactions[0].booking_date, "2026-03-04");
        assert_eq!(r.transactions[0].value_date, "2026-03-03");
        assert_eq!(r.transactions[0].counterparty, "BEISPIEL TAXI 1");
    }

    #[test]
    fn repeated_booking_ids_are_imported_once() {
        let mut f = fixture();
        let first = f["transactions"][0].clone();
        f["transactions"].as_array_mut().unwrap().push(first);
        assert_eq!(snapshot(&f.to_string()).unwrap().transactions.len(), 3);
    }

    #[test]
    fn unread_limit_is_unknown_not_zero() {
        let mut f = fixture();
        f["card"].as_object_mut().unwrap().remove("limitUsed");
        let r = snapshot(&f.to_string()).unwrap();
        assert_eq!(r.balances_unknown, vec!["airplus:170000000001".to_owned()]);
        assert!(r.balances_reported.is_empty());
    }

    #[test]
    fn a_broken_booking_rejects_the_whole_fetch() {
        let mut f = fixture();
        f["transactions"][1]["bookingDate"] = json!("kein Datum");
        assert!(snapshot(&f.to_string()).is_err());
        let mut f = fixture();
        f["transactions"][1]["currency"] = json!("USD");
        assert!(
            snapshot(&f.to_string()).is_err(),
            "foreign currency on a EUR card"
        );
        let mut f = fixture();
        f["version"] = json!(2);
        assert!(snapshot(&f.to_string()).is_err());
        assert!(snapshot("kein json").is_err());
    }

    #[test]
    fn repeated_import_creates_no_duplicates_and_rejects_other_connections() {
        let dir = crate::test_support::TempDir::new("airplus");
        let vault =
            Vault::open(dir.join("vault.db").to_string_lossy().into(), vec![7; 32]).unwrap();
        let connection = vault
            .add_bank_connection(ConnectorKind::AirPlus, "Test AirPlus".into(), "test".into())
            .unwrap();
        let first = vault
            .clone()
            .import_airplus_snapshot(connection, fixture().to_string())
            .unwrap();
        assert_eq!(first.transactions_imported, 3);
        let second = vault
            .clone()
            .import_airplus_snapshot(connection, fixture().to_string())
            .unwrap();
        assert_eq!(second.transactions_imported, 0);
        assert_eq!(vault.list_accounts().unwrap().len(), 1);
        // a new booking arrives on a later fetch
        let mut later = fixture();
        let extra = later["transactions"][0].clone();
        later["transactions"].as_array_mut().unwrap().push(extra);
        later["transactions"][3]["id"] = json!("a-4");
        let third = vault
            .clone()
            .import_airplus_snapshot(connection, later.to_string())
            .unwrap();
        assert_eq!(third.transactions_imported, 1);
        let other = vault
            .add_bank_connection(ConnectorKind::PayPal, "Other".into(), "test".into())
            .unwrap();
        assert!(
            vault
                .clone()
                .import_airplus_snapshot(other, fixture().to_string())
                .is_err()
        );
    }

    #[test]
    fn a_read_limit_of_zero_stays_zero_and_is_not_derived_from_bookings() {
        let dir = crate::test_support::TempDir::new("airplus-zero");
        let vault =
            Vault::open(dir.join("vault.db").to_string_lossy().into(), vec![7; 32]).unwrap();
        let connection = vault
            .add_bank_connection(ConnectorKind::AirPlus, "Test AirPlus".into(), "test".into())
            .unwrap();
        let mut zero = fixture();
        zero["card"]["limitUsed"] = json!(0);
        let parsed = snapshot(&zero.to_string()).unwrap();
        assert_eq!(
            parsed.balances_reported.len(),
            1,
            "{:?}",
            parsed.balances_unknown
        );
        assert_eq!(parsed.accounts[0].balance_cents, 0);
        vault
            .clone()
            .import_airplus_snapshot(connection, zero.to_string())
            .unwrap();
        let account = vault.list_accounts().unwrap().remove(0);
        assert_eq!(account.balance_cents, 0, "bookings must not be summed up");
    }
}
