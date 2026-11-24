//! Sharinbox (Société Générale Securities Services): employee share plans. The pages are
//! read inside the embedded browser; only the selected share data crosses the bridge.
//! The portal is the only interface (no FinTS/PSD2), see `docs/sharinbox-connector.md`.
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

/// Stable reference of the single securities account; it carries no personal number.
const ACCOUNT_REF: &str = "depot:sharinbox";

fn invalid() -> CoreError {
    CoreError::InvalidInput {
        message: tr("Sharinbox: Unvollständige oder ungültige Portalantwort. Es wurden keine Daten übernommen.").into(),
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

/// Monetary values are rounded once at the boundary, never quantities.
fn cents_of(n: f64) -> i64 {
    (n * 100.0).round() as i64
}

/// One security as the portal shows it: the available and the blocked balance are added up
/// by the page script, so quantity and value are totals.
struct Position {
    isin: String,
    name: String,
    quantity: f64,
    price: f64,
    value: i64,
}

/// Share rights (claims to shares that are allotted later): only the number is shown by the
/// portal, so the position carries no price and no value. `None` = the page lists none.
fn rights(depot: &Value) -> CoreResult<Option<f64>> {
    let rights = &depot["rights"];
    if rights.is_null() {
        return Ok(None);
    }
    let quantity = number(rights, "quantity")?;
    if quantity < 0.0 {
        return Err(invalid());
    }
    Ok((quantity > 0.0).then_some(quantity))
}

/// ISO 6166: two capital letters, nine capital letters or digits, one check digit (Luhn over the
/// letters written as 10..35).
fn valid_isin(isin: &str) -> bool {
    let b = isin.as_bytes();
    if b.len() != 12
        || !b[..2].iter().all(u8::is_ascii_uppercase)
        || !b[2..11]
            .iter()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        || !b[11].is_ascii_digit()
    {
        return false;
    }
    let digits: Vec<u32> = isin
        .chars()
        .flat_map(|c| {
            let v = c.to_digit(36).unwrap_or(0);
            if v >= 10 {
                vec![v / 10, v % 10]
            } else {
                vec![v]
            }
        })
        .collect();
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &d)| {
            if i % 2 == 1 {
                let x = d * 2;
                x / 10 + x % 10
            } else {
                d
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

fn positions(depot: &Value) -> CoreResult<Vec<Position>> {
    let items = depot["items"].as_array().ok_or_else(invalid)?;
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let isin = text(item, "isin")?;
        if !valid_isin(isin) || !seen.insert(isin.to_string()) {
            return Err(invalid());
        }
        if text(item, "currency")? != "EUR" {
            return Err(invalid());
        }
        date(item, "priceDate")?;
        let quantity = number(item, "quantity")?;
        let price = number(item, "price")?;
        let value = cents_of(number(item, "value")?);
        if quantity < 0.0 || price < 0.0 || value < 0 {
            return Err(invalid());
        }
        // value = shares x price. The price is shown with three decimals, so allow 1 ct plus
        // 0.05 ct per share for that rounding.
        let expected = (quantity * price * 100.0).round() as i64;
        if (value - expected).abs() > 1 + (quantity * 0.05).ceil() as i64 {
            return Err(invalid());
        }
        out.push(Position {
            isin: isin.into(),
            name: item["name"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(isin)
                .into(),
            quantity,
            price,
            value,
        });
    }
    Ok(out)
}

fn holding(p: &Position) -> Holding {
    // the portal shows no purchase price, so there is no profit or loss either
    Holding {
        isin: p.isin.clone(),
        name: p.name.clone(),
        quantity: p.quantity,
        current_price_cents: cents_of(p.price),
        value_cents: p.value,
        ..Default::default()
    }
}

fn snapshot(json: &str) -> CoreResult<FetchResult> {
    let root: Value = serde_json::from_str(json).map_err(|_| invalid())?;
    if root["version"].as_u64() != Some(1) {
        return Err(invalid());
    }
    let as_of = date(&root, "asOf")?;
    let depot = &root["depot"];
    let positions = positions(depot)?;
    let rights = rights(depot)?;
    let total: i64 = positions.iter().map(|p| p.value).sum();
    let mut result = FetchResult {
        bank_name: Some("Sharinbox".into()),
        ..Default::default()
    };
    result.accounts.push(RemoteAccount {
        external_ref: ACCOUNT_REF.into(),
        name: tr("Mitarbeiteraktien").into(),
        owner: String::new(),
        iban: String::new(),
        account_number: String::new(),
        currency: "EUR".into(),
        kind: AccountKind::Brokerage,
        balance_cents: total,
        balance_date: as_of,
    });
    for p in &positions {
        result.holdings.push(RemoteHolding {
            classification: Default::default(),
            direct_crypto: false,
            account_ref: ACCOUNT_REF.into(),
            holding: holding(p),
        });
    }
    if let Some(quantity) = rights {
        // own position, no ISIN, price or value yet: the note is part of the name
        result.holdings.push(RemoteHolding {
            classification: Default::default(),
            direct_crypto: false,
            account_ref: ACCOUNT_REF.into(),
            holding: Holding {
                name: tr("Aktienrechte (noch nicht zugeteilt)").into(),
                quantity,
                ..Default::default()
            },
        });
    }
    // an empty list clears positions that were sold since the last fetch
    result.holdings_fetched.push(ACCOUNT_REF.into());
    Ok(result)
}

#[uniffi::export]
impl Vault {
    pub fn import_sharinbox_snapshot(
        self: Arc<Self>,
        connection_id: i64,
        snapshot_json: String,
    ) -> CoreResult<SyncSummary> {
        if self.portal_connection(connection_id)?.0 != ConnectorKind::Sharinbox {
            return Err(invalid());
        }
        crate::banking_gate::check_connector("sharinbox")?;
        let summary = super::store_fetch(
            &self,
            ConnectorKind::Sharinbox,
            connection_id,
            snapshot(&snapshot_json)?,
        )?;
        let _ = self.checkpoint();
        Ok(summary)
    }
}

impl Vault {
    fn check_sharinbox(&self, connection_id: i64) -> CoreResult<()> {
        if self.portal_connection(connection_id)?.0 != ConnectorKind::Sharinbox {
            return Err(invalid());
        }
        crate::banking_gate::check_connector("sharinbox")
    }
}

/// The depot is archived, deleted or switched off in the account selection: no data is stored for
/// it (sync contract in `types.rs`).
fn depot_skipped(db: &rusqlite::Connection, connection_id: i64) -> CoreResult<bool> {
    Ok(super::account_selection::skipped_refs(db, connection_id)?
        .iter()
        .any(|r| r == ACCOUNT_REF))
}

/// Stable per connection and document; no session state is part of it.
fn document_source(connection_id: i64, id: &str) -> String {
    format!("sharinbox:{connection_id}:{}", URL_SAFE_NO_PAD.encode(id))
}

/// The portal identifies a document by a UUID (`8-4-4-4-12` hex digits). It ends up in the
/// download address, so nothing else is accepted.
///
/// On failure the error names the field, never its content.
fn document_id(doc: &Value) -> Result<String, &'static str> {
    let id = doc["id"].as_str().ok_or("id")?;
    let groups: Vec<&str> = id.split('-').collect();
    let shape = [8, 4, 4, 4, 12];
    let ok = groups.len() == shape.len()
        && groups
            .iter()
            .zip(shape)
            .all(|(g, n)| g.len() == n && g.bytes().all(|b| b.is_ascii_hexdigit()));
    if ok {
        Ok(id.to_ascii_lowercase())
    } else {
        Err("id")
    }
}

/// A document the core rejects: which entry (1-based) and which field, no content.
fn document_error(index: usize, field: &str) -> CoreError {
    CoreError::InvalidInput {
        message: crate::i18n::trf(
            "Sharinbox: Dokument {0} abgelehnt, Feld {1} ungültig.",
            &[&(index + 1), &field],
        ),
    }
}

#[uniffi::export]
impl Vault {
    /// Only returns unknown documents; a PDF is stored once per connection and document.
    pub fn pending_sharinbox_documents(
        self: Arc<Self>,
        connection_id: i64,
        documents_json: String,
    ) -> CoreResult<Vec<String>> {
        self.check_sharinbox(connection_id)?;
        // document retrieval switched off for this connection: nothing to download
        if !super::documents_fetch_on(&*self.db()?, connection_id)? {
            return Ok(Vec::new());
        }
        let docs: Vec<Value> = serde_json::from_str(&documents_json).map_err(|_| invalid())?;
        let db = self.db()?;
        // an archived depot or one left out of the account selection receives nothing
        if depot_skipped(&db, connection_id)? {
            return Ok(Vec::new());
        }
        let mut seen = HashSet::new();
        let mut pending = Vec::new();
        for (index, doc) in docs.into_iter().enumerate() {
            let id = document_id(&doc).map_err(|f| document_error(index, f))?;
            date(&doc, "sentDate").map_err(|_| document_error(index, "sentDate"))?;
            if seen.insert(id.clone())
                && !db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM booking_documents WHERE source_url=?1)",
                    [document_source(connection_id, &id)],
                    |r| r.get::<_, bool>(0),
                )?
            {
                pending.push(doc.to_string());
            }
        }
        Ok(pending)
    }

    /// Stores one PDF at the securities account of the connection.
    pub fn import_sharinbox_document(
        self: Arc<Self>,
        connection_id: i64,
        document_json: String,
        bytes: Vec<u8>,
    ) -> CoreResult<bool> {
        self.check_sharinbox(connection_id)?;
        if !bytes.starts_with(b"%PDF-") {
            return Err(invalid());
        }
        let doc: Value = serde_json::from_str(&document_json).map_err(|_| invalid())?;
        let id = document_id(&doc).map_err(|_| invalid())?;
        let sent = date(&doc, "sentDate")?;
        let title = doc["subject"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("Sharinbox-Dokument");
        let db = self.db()?;
        if depot_skipped(&db, connection_id)? {
            return Ok(false);
        }
        let account_id: i64 = db
            .query_row(
                "SELECT id FROM accounts WHERE connection_id=?1 AND deleted_ms IS NULL AND external_ref=?2",
                params![connection_id, ACCOUNT_REF],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(invalid)?;
        Ok(crate::documents::insert_doc(
            &db,
            None,
            Some(account_id),
            title,
            "postbox",
            "application/pdf",
            &format!("sharinbox_{id}.pdf"),
            &bytes,
            &document_source(connection_id, &id),
            &sent,
        )?
        .is_some())
    }
}

/// Placeholder connector: the login and the page reads run in the embedded browser.
pub struct WebOnlyConnector;

#[async_trait::async_trait]
impl super::BankConnector for WebOnlyConnector {
    async fn begin(&mut self, _ctx: &super::ConnectContext) -> CoreResult<super::Phase> {
        Err(CoreError::BrowserLoginRequired {
            message: tr("Bitte bei Sharinbox im Browserfenster anmelden und die Daten abrufen.")
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
            message: tr("Bitte bei Sharinbox im Browserfenster abrufen.").into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn item(isin: &str, quantity: f64, price: f64, value: f64) -> Value {
        json!({"isin":isin,"name":"Test Aktie","quantity":quantity,"price":price,
            "priceDate":"2026-11-20","currency":"EUR","value":value})
    }

    fn fixture() -> Value {
        json!({"version":1,"asOf":"2026-11-21","depot":{
            "items":[item("FR0000120271", 100.0, 45.14, 4514.0)]}})
    }

    #[test]
    fn maps_the_share_and_the_total() {
        let r = snapshot(&fixture().to_string()).unwrap();
        assert_eq!(r.accounts.len(), 1);
        assert_eq!(r.accounts[0].kind, AccountKind::Brokerage);
        assert_eq!(r.accounts[0].balance_cents, 451_400);
        assert_eq!(r.accounts[0].external_ref, ACCOUNT_REF);
        assert_eq!(r.holdings.len(), 1);
        let h = &r.holdings[0].holding;
        assert_eq!((h.isin.as_str(), h.quantity), ("FR0000120271", 100.0));
        assert_eq!((h.current_price_cents, h.value_cents), (4514, 451_400));
        assert_eq!(h.purchase_price_cents, 0);
        assert_eq!(r.holdings_fetched, vec![ACCOUNT_REF.to_string()]);
    }

    #[test]
    fn share_rights_become_their_own_position_without_value() {
        let mut f = fixture();
        f["depot"]["rights"] = json!({"quantity": 2085.0});
        let r = snapshot(&f.to_string()).unwrap();
        assert_eq!(r.holdings.len(), 2);
        let h = &r.holdings[1].holding;
        assert_eq!(
            (h.isin.as_str(), h.quantity, h.value_cents),
            ("", 2085.0, 0)
        );
        assert!(h.name.contains("noch nicht zugeteilt"));
        // the total of the account stays the value of the shares held
        assert_eq!(r.accounts[0].balance_cents, 451_400);
        // zero rights, no entry, negative or missing quantity
        f["depot"]["rights"] = json!({"quantity": 0.0});
        assert_eq!(snapshot(&f.to_string()).unwrap().holdings.len(), 1);
        f["depot"]["rights"] = json!({"quantity": -1.0});
        assert!(snapshot(&f.to_string()).is_err());
        f["depot"]["rights"] = json!({});
        assert!(snapshot(&f.to_string()).is_err());
        // rights alone are a valid depot
        f["depot"] = json!({"items": [], "rights": {"quantity": 5.0}});
        assert_eq!(snapshot(&f.to_string()).unwrap().holdings.len(), 1);
    }

    #[test]
    fn the_price_with_three_decimals_is_tolerated() {
        // 16 290 shares at a price shown as 45.140: no cent difference
        let mut f = fixture();
        f["depot"]["items"] = json!([item("FR0000120271", 16290.0, 45.14, 735330.6)]);
        assert!(snapshot(&f.to_string()).is_ok());
        // rounded price, value from the unrounded one: stays within 0.05 ct per share
        f["depot"]["items"] = json!([item("FR0000120271", 16290.0, 45.14, 735335.0)]);
        assert!(snapshot(&f.to_string()).is_ok());
    }

    #[test]
    fn an_empty_list_is_valid_and_clears_positions() {
        let mut f = fixture();
        f["depot"]["items"] = json!([]);
        let r = snapshot(&f.to_string()).unwrap();
        assert!(r.holdings.is_empty());
        assert_eq!(r.accounts[0].balance_cents, 0);
        assert_eq!(r.holdings_fetched.len(), 1);
    }

    #[test]
    fn rejects_incomplete_or_inconsistent_snapshots() {
        let bad = |f: Value| assert!(snapshot(&f.to_string()).is_err(), "{f}");
        let mut f = fixture();
        f["version"] = json!(2);
        bad(f);
        let mut f = fixture();
        f["asOf"] = json!("gestern");
        bad(f);
        let mut f = fixture();
        f["depot"] = json!({});
        bad(f);
        // value does not match shares x price
        let mut f = fixture();
        f["depot"]["items"] = json!([item("FR0000120271", 1.0, 10.0, 100.0)]);
        bad(f);
        // malformed ISIN, other currency, negative quantity, duplicate ISIN
        for broken in [
            item("FR00", 1.0, 1.0, 1.0),
            // twelve valid characters, wrong check digit, and a lower-case country code
            item("FR0000120272", 1.0, 1.0, 1.0),
            item("ABCDEFGHIJKL", 1.0, 1.0, 1.0),
            item("fr0000120271", 1.0, 1.0, 1.0),
            json!({"isin":"FR0000120271","name":"x","quantity":1.0,"price":1.0,
                "priceDate":"2026-11-20","currency":"USD","value":1.0}),
            item("FR0000120271", -1.0, 1.0, -1.0),
        ] {
            let mut f = fixture();
            f["depot"]["items"] = json!([broken]);
            bad(f);
        }
        let mut f = fixture();
        f["depot"]["items"] = json!([
            item("FR0000120271", 1.0, 1.0, 1.0),
            item("FR0000120271", 1.0, 1.0, 1.0)
        ]);
        bad(f);
        assert!(snapshot("not json").is_err());
    }

    #[test]
    fn a_missing_name_falls_back_to_the_isin() {
        let mut f = fixture();
        f["depot"]["items"][0]["name"] = json!("  ");
        let r = snapshot(&f.to_string()).unwrap();
        assert_eq!(r.holdings[0].holding.name, "FR0000120271");
    }

    #[test]
    fn import_stores_the_depot_and_is_idempotent() {
        let dir = crate::test_support::TempDir::new("sharinbox-import");
        let vault =
            Vault::open(dir.join("vault.db").to_string_lossy().into(), vec![7; 32]).unwrap();
        let id = vault
            .add_bank_connection(
                ConnectorKind::Sharinbox,
                "Mitarbeiteraktien".into(),
                "x".into(),
            )
            .unwrap();
        for _ in 0..2 {
            let s = vault
                .clone()
                .import_sharinbox_snapshot(id, fixture().to_string())
                .unwrap();
            assert_eq!(s.accounts_updated, 1);
        }
    }

    const DOC_A: &str = "0a1b2c3d-1111-4222-8333-444455556666";
    const DOC_B: &str = "0a1b2c3d-1111-4222-8333-444455557777";

    fn document(id: &str) -> Value {
        json!({"id":id,"sentDate":"2026-11-10","subject":"Transaktionsmitteilung Wertpapiere"})
    }

    fn vault_with_depot(name: &str) -> (crate::test_support::TempDir, Arc<Vault>, i64) {
        let dir = crate::test_support::TempDir::new(name);
        let vault =
            Vault::open(dir.join("vault.db").to_string_lossy().into(), vec![7; 32]).unwrap();
        let connection = vault
            .add_bank_connection(ConnectorKind::Sharinbox, "Test".into(), "x".into())
            .unwrap();
        vault
            .clone()
            .import_sharinbox_snapshot(connection, fixture().to_string())
            .unwrap();
        (dir, vault, connection)
    }

    #[test]
    fn real_isins_pass_the_check_digit() {
        for isin in [
            "US0378331005",
            "DE0007164600",
            "FR0000120271",
            "DE000BAY0017",
        ] {
            assert!(valid_isin(isin), "{isin}");
        }
    }

    #[test]
    fn an_archived_depot_receives_no_documents() {
        let (_dir, vault, connection) = vault_with_depot("sharinbox-archived");
        vault
            .db()
            .unwrap()
            .execute("UPDATE accounts SET archived_unix = 1", [])
            .unwrap();
        let doc = document(DOC_A).to_string();
        assert!(
            vault
                .clone()
                .pending_sharinbox_documents(connection, format!("[{doc}]"))
                .unwrap()
                .is_empty()
        );
        assert!(
            !vault
                .clone()
                .import_sharinbox_document(connection, doc, b"%PDF-1.7 x".to_vec())
                .unwrap()
        );
        let stored: i64 = vault
            .db()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM booking_documents", [], |r| r.get(0))
            .unwrap();
        assert_eq!(stored, 0);
    }

    #[test]
    fn documents_are_stored_once_at_the_depot() {
        let (_dir, vault, connection) = vault_with_depot("sharinbox-docs");
        let doc = document(DOC_A).to_string();
        let pending = |list: String| {
            vault
                .clone()
                .pending_sharinbox_documents(connection, list)
                .unwrap()
        };
        // the same document twice in one list is reported once
        assert_eq!(pending(format!("[{doc},{doc}]")).len(), 1);
        let pdf = b"%PDF-1.7 synthetic".to_vec();
        assert!(
            vault
                .clone()
                .import_sharinbox_document(connection, doc.clone(), pdf.clone())
                .unwrap()
        );
        // a second import and a second listing add nothing
        assert!(
            !vault
                .clone()
                .import_sharinbox_document(connection, doc.clone(), pdf.clone())
                .unwrap()
        );
        assert!(pending(format!("[{doc}]")).is_empty());
        // another document is new; the id is compared in lower case
        let other = document(&DOC_B.to_uppercase()).to_string();
        assert_eq!(pending(format!("[{doc},{other}]")).len(), 1);
        let stored = vault
            .list_all_documents(crate::documents::DocumentFilter::default())
            .unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].title, "Transaktionsmitteilung Wertpapiere");
    }

    #[test]
    fn documents_reject_bad_ids_dates_and_files() {
        let (_dir, vault, connection) = vault_with_depot("sharinbox-docs-bad");
        for id in [
            "",
            "../1",
            "0a1b2c3d-1111-4222-8333",
            "zzzzzzzz-1111-4222-8333-444455556666",
        ] {
            let list = json!([document(id)]).to_string();
            let err = vault
                .clone()
                .pending_sharinbox_documents(connection, list)
                .unwrap_err()
                .to_string();
            assert!(err.contains("Dokument 1") && err.contains("id"), "{err}");
            assert!(!err.contains(id) || id.is_empty());
        }
        let mut d = document(DOC_A);
        d["sentDate"] = json!("gestern");
        let err = vault
            .clone()
            .pending_sharinbox_documents(connection, json!([d]).to_string())
            .unwrap_err()
            .to_string();
        assert!(err.contains("sentDate"), "{err}");
        // not a PDF
        assert!(
            vault
                .clone()
                .import_sharinbox_document(
                    connection,
                    document(DOC_A).to_string(),
                    b"<html>login</html>".to_vec()
                )
                .is_err()
        );
        // empty subject: still a document
        let mut d = document(DOC_B);
        d["subject"] = json!(" ");
        assert!(
            vault
                .clone()
                .import_sharinbox_document(connection, d.to_string(), b"%PDF-1.7 x".to_vec())
                .unwrap()
        );
        let stored = vault
            .list_all_documents(crate::documents::DocumentFilter::default())
            .unwrap();
        assert_eq!(stored[0].title, "Sharinbox-Dokument");
        // another connector kind may not use the calls
        let other = vault
            .add_bank_connection(ConnectorKind::Quirion, "Andere".into(), "x".into())
            .unwrap();
        assert!(
            vault
                .clone()
                .pending_sharinbox_documents(other, "[]".into())
                .is_err()
        );
    }

    #[test]
    fn rejects_other_connector_kinds() {
        let dir = crate::test_support::TempDir::new("sharinbox-other");
        let vault =
            Vault::open(dir.join("vault.db").to_string_lossy().into(), vec![7; 32]).unwrap();
        let id = vault
            .add_bank_connection(ConnectorKind::Quirion, "Andere".into(), "x".into())
            .unwrap();
        assert!(
            vault
                .clone()
                .import_sharinbox_snapshot(id, fixture().to_string())
                .is_err()
        );
    }
}
