//! Union Investment customer portal (UnionDepot). Requests and authorization stay
//! inside the embedded browser; only the selected depot data crosses the bridge.
//! The portal is the only interface (no FinTS/PSD2), see `docs/unioninvestment-connector.md`.
use std::{collections::HashSet, sync::Arc};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;

use super::{FetchResult, RemoteAccount, RemoteHolding, SyncSummary};
use crate::{
    error::{CoreError, CoreResult},
    i18n::tr,
    models::{AccountKind, ConnectorKind, Holding},
    vault::Vault,
};

fn invalid() -> CoreError {
    CoreError::InvalidInput {
        message: tr("Union Investment: Unvollständige oder ungültige Portalantwort. Es wurden keine Daten übernommen.").into(),
    }
}

fn text<'a>(v: &'a Value, key: &str) -> CoreResult<&'a str> {
    v[key]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(invalid)
}

fn date(v: &Value, key: &str) -> CoreResult<String> {
    let s = text(v, key)?.get(..10).ok_or_else(invalid)?;
    jiff::civil::Date::strptime("%Y-%m-%d", s).map_err(|_| invalid())?;
    Ok(s.into())
}

fn number(v: &Value, key: &str) -> CoreResult<f64> {
    v[key]
        .as_f64()
        .filter(|n| n.is_finite() && n.abs() < 1e12)
        .ok_or_else(invalid)
}

// The portal returns binary-float artefacts (e.g. 8398.279999999999).
// Round monetary values once at the boundary, never quantities.
fn cents_of(n: f64) -> i64 {
    (n * 100.0).round() as i64
}

fn cents(v: &Value, key: &str) -> CoreResult<i64> {
    Ok(cents_of(number(v, key)?))
}

fn rows<'a>(v: &'a Value, key: &str) -> CoreResult<&'a Vec<Value>> {
    v[key].as_array().ok_or_else(invalid)
}

/// One position per ISIN: the portal lists one entry per investment goal
/// (sub-depot), and several goals may hold the same fund. Quantities, values
/// and gains are summed; the price is the (identical) redemption price.
struct Position {
    isin: String,
    name: String,
    quantity: f64,
    price: f64,
    value: i64,
    unrealized: i64,
}

fn positions(depot: &Value) -> CoreResult<Vec<Position>> {
    let mut out: Vec<Position> = Vec::new();
    for item in rows(depot, "items")? {
        let isin = text(item, "isin")?;
        if isin.len() != 12 || !isin.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(invalid());
        }
        let quantity = number(item, "quantity")?;
        if quantity < 0.0 || text(item, "currency")? != "EUR" {
            return Err(invalid());
        }
        date(item, "priceDate")?;
        let value = cents(item, "value")?;
        let unrealized = cents(item, "unrealized")?;
        let price = number(item, "price")?;
        match out.iter_mut().find(|p| p.isin == isin) {
            Some(p) => {
                p.quantity += quantity;
                p.value += value;
                p.unrealized += unrealized;
            }
            None => out.push(Position {
                isin: isin.into(),
                name: text(item, "name")?.into(),
                quantity,
                price,
                value,
                unrealized,
            }),
        }
    }
    Ok(out)
}

fn holding(p: &Position) -> Holding {
    let cost = p.value - p.unrealized;
    // cost price per share; without shares or a cost basis there is nothing to show
    let purchase = if p.quantity > 0.0 && cost > 0 {
        (cost as f64 / p.quantity).round() as i64
    } else {
        0
    };
    Holding {
        isin: p.isin.clone(),
        name: p.name.clone(),
        quantity: p.quantity,
        current_price_cents: cents_of(p.price),
        purchase_price_cents: purchase,
        value_cents: p.value,
        profit_loss_purchase_cents: (purchase > 0).then_some(p.unrealized),
        profit_loss_purchase_rel: (cost > 0).then(|| p.unrealized as f64 / cost as f64 * 100.0),
        ..Default::default()
    }
}

fn snapshot(json: &str) -> CoreResult<FetchResult> {
    let root: Value = serde_json::from_str(json).map_err(|_| invalid())?;
    if root["version"].as_u64() != Some(1) {
        return Err(invalid());
    }
    let as_of = date(&root, "asOf")?;
    let mut result = FetchResult {
        bank_name: Some("Union Investment".into()),
        ..Default::default()
    };
    let mut seen = HashSet::new();
    for depot in rows(&root, "depots")? {
        let number_ = text(depot, "number")?;
        if !number_.bytes().all(|b| b.is_ascii_digit()) || !seen.insert(number_.to_string()) {
            return Err(invalid());
        }
        let account_ref = format!("depot:{number_}");
        let positions = positions(depot)?;
        let depot_value = cents(depot, "value")?;
        // the portal's own depot value must match the listed funds (rounding: 1 ct per row)
        let total: i64 = positions.iter().map(|p| p.value).sum();
        if (total - depot_value).abs() > positions.len() as i64 {
            return Err(invalid());
        }
        result.accounts.push(RemoteAccount {
            external_ref: account_ref.clone(),
            name: text(depot, "name")?.into(),
            owner: text(depot, "owner")?.into(),
            iban: String::new(),
            account_number: number_.into(),
            currency: "EUR".into(),
            kind: AccountKind::Brokerage,
            balance_cents: depot_value,
            balance_date: as_of.clone(),
        });
        for p in &positions {
            result.holdings.push(RemoteHolding {
                classification: Default::default(),
                direct_crypto: false,
                account_ref: account_ref.clone(),
                holding: holding(p),
            });
        }
        // an empty depot clears positions that were sold since the last fetch
        result.holdings_fetched.push(account_ref);
    }
    if result.accounts.is_empty() {
        return Err(invalid());
    }
    Ok(result)
}

#[uniffi::export]
impl Vault {
    pub fn import_unioninvestment_snapshot(
        self: Arc<Self>,
        connection_id: i64,
        snapshot_json: String,
    ) -> CoreResult<SyncSummary> {
        if self.portal_connection(connection_id)?.0 != ConnectorKind::UnionInvestment {
            return Err(invalid());
        }
        crate::banking_gate::check_connector("unioninvestment")?;
        let summary = super::store_fetch(
            &self,
            ConnectorKind::UnionInvestment,
            connection_id,
            snapshot(&snapshot_json)?,
        )?;
        let _ = self.checkpoint();
        Ok(summary)
    }
}

impl Vault {
    fn check_unioninvestment(&self, connection_id: i64) -> CoreResult<()> {
        if self.portal_connection(connection_id)?.0 != ConnectorKind::UnionInvestment {
            return Err(invalid());
        }
        crate::banking_gate::check_connector("unioninvestment")
    }
}

/// Stable per connection and mailbox document; no session state is part of it.
fn document_source(connection_id: i64, key: &str) -> String {
    format!(
        "unioninvestment:{connection_id}:{}",
        URL_SAFE_NO_PAD.encode(key)
    )
}

/// The page script identifies a mailbox document by depot number (or, for letters to the
/// depot owner that belong to no depot, the owner index), message id and attachment
/// number; all of them must be plain digits.
///
/// On failure the error names the field, never its content.
fn document_key(doc: &Value) -> Result<String, &'static str> {
    let digits = |key: &'static str| -> Result<String, &'static str> {
        match doc[key].as_str() {
            Some(s) if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) => Ok(s.into()),
            _ => Err(key),
        }
    };
    let scope = if doc.get("depotNumber").is_some() {
        digits("depotNumber")?
    } else {
        format!("owner{}", digits("ownerIndex")?)
    };
    Ok(format!(
        "{scope}:{}:{}",
        digits("id")?,
        digits("attachmentId")?
    ))
}

/// A mailbox entry the core rejects: which entry (1-based) and which field, no content.
fn document_error(index: usize, field: &str) -> CoreError {
    CoreError::InvalidInput {
        message: crate::i18n::trf(
            "Union Investment: Postbox-Eintrag {0} abgelehnt, Feld {1} ungültig.",
            &[&(index + 1), &field],
        ),
    }
}

#[uniffi::export]
impl Vault {
    /// Only returns unknown mailbox documents; a PDF is stored once per
    /// connection, message and attachment.
    pub fn pending_unioninvestment_documents(
        self: Arc<Self>,
        connection_id: i64,
        documents_json: String,
    ) -> CoreResult<Vec<String>> {
        self.check_unioninvestment(connection_id)?;
        // document retrieval switched off for this connection: nothing to download
        if !super::documents_fetch_on(&*self.db()?, connection_id)? {
            return Ok(Vec::new());
        }
        let docs: Vec<Value> = serde_json::from_str(&documents_json).map_err(|_| invalid())?;
        let db = self.db()?;
        let mut seen = HashSet::new();
        let mut pending = Vec::new();
        for (index, doc) in docs.into_iter().enumerate() {
            let key = document_key(&doc).map_err(|f| document_error(index, f))?;
            date(&doc, "sentDate").map_err(|_| document_error(index, "sentDate"))?;
            if seen.insert(key.clone())
                && !db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM booking_documents WHERE source_url=?1)",
                    [document_source(connection_id, &key)],
                    |r| r.get::<_, bool>(0),
                )?
            {
                pending.push(doc.to_string());
            }
        }
        Ok(pending)
    }

    /// Stores one mailbox PDF at the securities account of its depot.
    pub fn import_unioninvestment_document(
        self: Arc<Self>,
        connection_id: i64,
        document_json: String,
        bytes: Vec<u8>,
    ) -> CoreResult<bool> {
        self.check_unioninvestment(connection_id)?;
        if !bytes.starts_with(b"%PDF-") {
            return Err(invalid());
        }
        let doc: Value = serde_json::from_str(&document_json).map_err(|_| invalid())?;
        let key = document_key(&doc).map_err(|_| invalid())?;
        let sent = date(&doc, "sentDate")?;
        // some messages carry no subject; they are still documents
        let title = doc["subject"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("Postbox-Nachricht");
        let db = self.db()?;
        // A document of a depot goes to that depot. A letter to the depot owner belongs to no
        // depot: it is attached to the connection's first securities account.
        let account_id: i64 = if doc.get("depotNumber").is_some() {
            db.query_row(
                "SELECT id FROM accounts WHERE connection_id=?1 AND deleted_ms IS NULL AND external_ref=?2",
                params![connection_id, format!("depot:{}", text(&doc, "depotNumber")?)],
                |r| r.get(0),
            )
        } else {
            db.query_row(
                "SELECT id FROM accounts WHERE connection_id=?1 AND deleted_ms IS NULL AND external_ref LIKE 'depot:%' ORDER BY id LIMIT 1",
                params![connection_id],
                |r| r.get(0),
            )
        }
        .optional()?
        .ok_or_else(invalid)?;
        Ok(crate::documents::insert_doc(
            &db,
            None,
            Some(account_id),
            title,
            "postbox",
            "application/pdf",
            &format!("unioninvestment_{}.pdf", URL_SAFE_NO_PAD.encode(&key)),
            &bytes,
            &document_source(connection_id, &key),
            &sent,
        )?
        .is_some())
    }
}

pub struct WebOnlyConnector;

#[async_trait::async_trait]
impl super::BankConnector for WebOnlyConnector {
    async fn begin(&mut self, _ctx: &super::ConnectContext) -> CoreResult<super::Phase> {
        Err(CoreError::BrowserLoginRequired {
            message: tr(
                "Bitte bei Union Investment im Browserfenster anmelden und die Daten abrufen.",
            )
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
            message: tr("Bitte bei Union Investment im Browserfenster abrufen.").into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item(isin: &str, quantity: f64, price: f64, value: f64, unrealized: f64) -> Value {
        json!({"isin":isin,"name":"Test Fonds","quantity":quantity,"price":price,
            "priceDate":"2026-11-19","currency":"EUR","value":value,"unrealized":unrealized})
    }

    fn fixture() -> Value {
        json!({"version":1,"asOf":"2026-11-20","depots":[{
        "number":"123456789","owner":"Test Owner","name":"UnionDepot","value":330.0,
        "items":[
            item("DE0000000001", 2.5, 40.0, 100.0, 10.0),
            // same fund in a second goal: summed into one position
            item("DE0000000001", 2.5, 40.0, 100.0, -5.0),
            item("DE0000000002", 10.0, 13.0, 130.0, 0.0)
        ]}]})
    }

    #[test]
    fn maps_depot_and_sums_goals_per_isin() {
        let r = snapshot(&fixture().to_string()).unwrap();
        assert_eq!(r.accounts.len(), 1);
        assert_eq!(r.accounts[0].kind, AccountKind::Brokerage);
        assert_eq!(r.accounts[0].balance_cents, 33000);
        assert_eq!(r.accounts[0].balance_date, "2026-11-20");
        assert_eq!(r.holdings.len(), 2);
        let h = &r.holdings[0].holding;
        assert_eq!(h.quantity, 5.0);
        assert_eq!(h.value_cents, 20000);
        // cost basis = value - unrealized gain: (20000 - 500) / 5 shares
        assert_eq!(h.purchase_price_cents, 3900);
        assert_eq!(h.profit_loss_purchase_cents, Some(500));
        assert_eq!(r.holdings_fetched, vec!["depot:123456789".to_string()]);
    }

    #[test]
    fn float_artifacts_are_rounded_once() {
        let mut f = fixture();
        f["depots"][0]["items"] =
            json!([item("DE0000000001", 1.0001, 83.98, 83.98279999999999, 0.0)]);
        f["depots"][0]["value"] = json!(83.98);
        let r = snapshot(&f.to_string()).unwrap();
        assert_eq!(r.holdings[0].holding.value_cents, 8398);
        assert_eq!(r.holdings[0].holding.quantity, 1.0001);
    }

    #[test]
    fn empty_depot_is_valid_and_clears_positions() {
        let mut f = fixture();
        f["depots"][0]["items"] = json!([]);
        f["depots"][0]["value"] = json!(0.0);
        let r = snapshot(&f.to_string()).unwrap();
        assert!(r.holdings.is_empty());
        assert_eq!(r.holdings_fetched.len(), 1);
    }

    #[test]
    fn rejects_incomplete_or_inconsistent_snapshots() {
        let bad = |f: Value| assert!(snapshot(&f.to_string()).is_err(), "{f}");
        let mut f = fixture();
        f["version"] = json!(2);
        bad(f);
        let mut f = fixture();
        f["depots"][0]["value"] = json!(999.0); // does not match the funds
        bad(f);
        let mut f = fixture();
        f["depots"][0]["items"][0]["isin"] = json!("XX");
        bad(f);
        let mut f = fixture();
        f["depots"][0]["items"][0]["currency"] = json!("USD");
        bad(f);
        let mut f = fixture();
        f["depots"][0]["number"] = json!("12 34");
        bad(f);
        let mut f = fixture();
        f["depots"] = json!([]);
        bad(f);
        let mut f = fixture();
        f["depots"] = json!([f["depots"][0].clone(), f["depots"][0].clone()]);
        bad(f);
        assert!(snapshot("not json").is_err());
    }

    #[test]
    fn rejects_other_connector_kinds() {
        let dir = crate::test_support::TempDir::new("unioninvestment-other");
        let vault =
            Vault::open(dir.join("vault.db").to_string_lossy().into(), vec![7; 32]).unwrap();
        let id = vault
            .add_bank_connection(ConnectorKind::Quirion, "Andere".into(), "x".into())
            .unwrap();
        assert!(
            vault
                .clone()
                .import_unioninvestment_snapshot(id, fixture().to_string())
                .is_err()
        );
    }

    fn mailbox_doc(id: &str) -> Value {
        json!({"depotNumber":"123456789","id":id,"attachmentId":"1",
            "sentDate":"2026-10-01T08:30:00","subject":"Quartalsauszug 3. Quartal 2026"})
    }

    fn vault_with_depot(name: &str) -> (crate::test_support::TempDir, Arc<Vault>, i64) {
        let dir = crate::test_support::TempDir::new(name);
        let vault =
            Vault::open(dir.join("vault.db").to_string_lossy().into(), vec![7; 32]).unwrap();
        let connection = vault
            .add_bank_connection(
                ConnectorKind::UnionInvestment,
                "Test Union".into(),
                "test".into(),
            )
            .unwrap();
        vault
            .clone()
            .import_unioninvestment_snapshot(connection, fixture().to_string())
            .unwrap();
        (dir, vault, connection)
    }

    #[test]
    fn mailbox_documents_are_stored_once_at_the_depot() {
        let (_dir, vault, connection) = vault_with_depot("unioninvestment-docs");
        let doc = mailbox_doc("1000000001").to_string();
        let list = format!("[{doc},{doc}]");
        let pending = |list: &str| {
            vault
                .clone()
                .pending_unioninvestment_documents(connection, list.into())
                .unwrap()
        };
        // the same document twice in one list is reported once
        assert_eq!(pending(&list).len(), 1);
        assert!(
            vault
                .clone()
                .import_unioninvestment_document(
                    connection,
                    doc.clone(),
                    b"<html>login</html>".to_vec()
                )
                .is_err()
        );
        assert!(
            vault
                .clone()
                .import_unioninvestment_document(
                    connection,
                    doc.clone(),
                    b"%PDF-1.7 synthetic".to_vec()
                )
                .unwrap()
        );
        // a second import and a second sync add nothing
        assert!(
            !vault
                .clone()
                .import_unioninvestment_document(connection, doc, b"%PDF-1.7 synthetic".to_vec())
                .unwrap()
        );
        assert!(pending(&list).is_empty());
        // another message of the same depot is new
        assert_eq!(
            pending(&format!("[{}]", mailbox_doc("1000000002"))).len(),
            1
        );
    }

    #[test]
    fn a_message_without_subject_is_still_a_document() {
        let (_dir, vault, connection) = vault_with_depot("unioninvestment-docs-subject");
        for subject in [json!(""), json!("   "), json!(null)] {
            let mut doc = mailbox_doc("1000000042");
            doc["subject"] = subject;
            let doc = doc.to_string();
            assert_eq!(
                vault
                    .clone()
                    .pending_unioninvestment_documents(connection, format!("[{doc}]"))
                    .unwrap()
                    .len(),
                1
            );
        }
        let mut doc = mailbox_doc("1000000042");
        doc.as_object_mut().unwrap().remove("subject");
        assert!(
            vault
                .clone()
                .import_unioninvestment_document(
                    connection,
                    doc.to_string(),
                    b"%PDF-1.7 synthetic".to_vec()
                )
                .unwrap()
        );
        let stored = vault
            .list_all_documents(crate::documents::DocumentFilter::default())
            .unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].title, "Postbox-Nachricht");
    }

    #[test]
    fn letters_to_the_depot_owner_attach_to_the_depot_and_are_deduplicated() {
        let (_dir, vault, connection) = vault_with_depot("unioninvestment-docs-owner");
        let letter = json!({"ownerIndex":"1","id":"2000000001","attachmentId":"1",
            "sentDate":"2026-01-05T00:00:00","subject":"Mitteilung"})
        .to_string();
        let pending = |l: &str| {
            vault
                .clone()
                .pending_unioninvestment_documents(connection, format!("[{l}]"))
                .unwrap()
        };
        assert_eq!(pending(&letter).len(), 1);
        assert!(
            vault
                .clone()
                .import_unioninvestment_document(
                    connection,
                    letter.clone(),
                    b"%PDF-1.7 synthetic".to_vec()
                )
                .unwrap()
        );
        assert!(pending(&letter).is_empty());
        // a depot document with the same message number is a different document
        let mut same_id = mailbox_doc("2000000001");
        same_id["attachmentId"] = json!("1");
        assert_eq!(pending(&same_id.to_string()).len(), 1);
        // neither key may be missing
        let none = json!({"id":"1","attachmentId":"1","sentDate":"2026-01-05","subject":"x"});
        assert!(
            vault
                .clone()
                .pending_unioninvestment_documents(connection, json!([none]).to_string())
                .is_err()
        );
    }

    #[test]
    fn mailbox_documents_reject_bad_keys_unknown_depots_and_other_connections() {
        let (_dir, vault, connection) = vault_with_depot("unioninvestment-docs-bad");
        let bad = |doc: Value| {
            assert!(
                vault
                    .clone()
                    .pending_unioninvestment_documents(connection, json!([doc]).to_string())
                    .is_err(),
                "pending"
            )
        };
        let mut d = mailbox_doc("1");
        d["id"] = json!("../1");
        bad(d);
        let mut d = mailbox_doc("1");
        d["depotNumber"] = json!("12 3");
        bad(d);
        let mut d = mailbox_doc("1");
        d["sentDate"] = json!("gestern");
        bad(d);
        // the error names the entry and the field, never the content
        let mut d = mailbox_doc("1");
        d["id"] = json!("12 secret");
        let err = vault
            .clone()
            .pending_unioninvestment_documents(connection, json!([mailbox_doc("2"), d]).to_string())
            .unwrap_err()
            .to_string();
        assert!(err.contains("2") && err.contains("id"), "{err}");
        assert!(!err.contains("secret"), "{err}");
        // a depot this connection never imported cannot take the document
        let mut other_depot = mailbox_doc("1000000003");
        other_depot["depotNumber"] = json!("999999999");
        assert!(
            vault
                .clone()
                .import_unioninvestment_document(
                    connection,
                    other_depot.to_string(),
                    b"%PDF-1.7 synthetic".to_vec()
                )
                .is_err()
        );
        let wrong = vault
            .add_bank_connection(ConnectorKind::PayPal, "Other".into(), "test".into())
            .unwrap();
        assert!(
            vault
                .clone()
                .pending_unioninvestment_documents(wrong, "[]".into())
                .is_err()
        );
    }
}
