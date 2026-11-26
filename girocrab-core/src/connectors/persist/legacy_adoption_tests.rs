//! Legacy id adoption (`adopt_legacy_content_id`, #1721): a booking whose
//! ultimate party replaced the `Cdtr`/`Dbtr` name takes over its legacy row;
//! another booking that only shares the payment provider's IBAN never does.

use super::*;

const IBAN: &str = "DE00000000000000000000";
const LEGACY_ID: &str = "h0123456789abcdef";

fn conn_with_legacy_row() -> rusqlite::Connection {
    conn_with_legacy_row_named("Zahlungsdienst GmbH")
}

/// A legacy row whose counterparty an older version stored (already cleaned).
fn conn_with_legacy_row_named(counterparty: &str) -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE transactions(id INTEGER PRIMARY KEY, account_id INTEGER,
             external_id TEXT, booking_date TEXT, value_date TEXT, amount_cents INTEGER,
             counterparty TEXT, counterparty_iban TEXT, purpose TEXT);",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO transactions(account_id, external_id, booking_date, value_date,
             amount_cents, counterparty, counterparty_iban, purpose)
         VALUES (1, ?1, '2025-01-02', '2025-01-02', -2345, ?3, ?2, 'Bestellung 4711')",
        params![LEGACY_ID, IBAN, counterparty],
    )
    .unwrap();
    conn
}

/// A fetched booking with the legacy row's dates, amount and purpose.
fn fetched(counterparty: &str, original: &str, iban: &str) -> RemoteTransaction {
    let mut t = RemoteTransaction {
        booking_date: "2025-01-02".into(),
        value_date: "2025-01-02".into(),
        amount_cents: -2345,
        counterparty: counterparty.into(),
        counterparty_iban: iban.into(),
        purpose: "Bestellung 4711".into(),
        ..Default::default()
    };
    t.external_id = t.content_id(0);
    t.original_counterparty = original.into();
    t
}

fn stored_id(conn: &rusqlite::Connection) -> String {
    conn.query_row("SELECT external_id FROM transactions", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn ultimate_party_redelivery_adopts_the_row_stored_under_the_original_name() {
    let conn = conn_with_legacy_row();
    // No IBAN: the original name alone identifies the legacy row.
    let t = fetched("Shop am Markt", "Zahlungsdienst GmbH", "");
    assert!(adopt_legacy_content_id(&conn, 1, &t).unwrap());
    assert_eq!(stored_id(&conn), t.external_id);
}

#[test]
fn another_booking_sharing_the_iban_never_takes_the_legacy_row() {
    let conn = conn_with_legacy_row();
    for t in [
        fetched("Anderer Shop", "", IBAN),
        fetched("Anderer Shop", "Anderer Dienst AG", IBAN),
    ] {
        assert!(!adopt_legacy_content_id(&conn, 1, &t).unwrap());
    }
    assert_eq!(stored_id(&conn), LEGACY_ID);
}

/// A real vault with one account holding a MoneyMoney row without IBAN under
/// the ordinary name and with a purpose too short for `purposes_alike`: only
/// the name can match it.
fn vault_with_imported_row() -> (crate::test_support::TempDir, Arc<Vault>, i64) {
    let dir = crate::test_support::TempDir::new("legacy-adoption");
    let v = Vault::open(
        dir.join("v.db").to_string_lossy().to_string(),
        vec![7u8; 32],
    )
    .unwrap();
    let account = {
        let db = v.db().unwrap();
        let account = crate::assets::create_account_in(&db, "Konto").unwrap();
        insert_row(&db, account, "mm-1", "Zahlungsdienst GmbH");
        account
    };
    (dir, v, account)
}

fn insert_row(conn: &rusqlite::Connection, account: i64, external_id: &str, name: &str) -> i64 {
    conn.execute(
        "INSERT INTO transactions(account_id, external_id, booking_date, value_date,
             amount_cents, counterparty, counterparty_iban, purpose)
         VALUES (?1, ?2, '2025-01-03', '2025-01-03', -999, ?3, '', 'Abo')",
        params![account, external_id, name],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn live_rows(conn: &rusqlite::Connection) -> i64 {
    conn.query_row(
        "SELECT count(*) FROM transactions WHERE amount_cents = -999 AND deleted_ms IS NULL",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn ibanless_redelivery_adopts_the_imported_row_by_the_original_name() {
    use crate::importers::moneymoney_import::{adopt_import, adopt_import_or_original};
    let (_dir, v, account) = vault_with_imported_row();
    let conn = v.db().unwrap();
    let adopt = |original: &str| {
        adopt_import_or_original(
            &conn,
            account,
            "fints-1",
            "2025-01-03",
            -999,
            "",
            "Shop am Markt",
            original,
            "Abo",
            "",
        )
        .unwrap()
    };
    // The displayed (ultimate) name alone misses the row …
    assert!(
        !adopt_import(
            &conn,
            account,
            "fints-1",
            "2025-01-03",
            -999,
            "",
            "Shop am Markt",
            "Abo",
            ""
        )
        .unwrap()
    );
    assert!(!adopt(""));
    // … the ordinary name takes it over instead of inserting a duplicate.
    assert!(adopt("Zahlungsdienst GmbH"));
    let ext: String = conn
        .query_row(
            "SELECT external_id FROM transactions WHERE amount_cents = -999",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(ext, "fints-1");
    assert_eq!(live_rows(&conn), 1);
}

#[test]
fn ibanless_duplicate_merges_into_the_imported_row_by_the_original_name() {
    use crate::importers::moneymoney_import::merge_synced_twin_or_original;
    let (_dir, v, account) = vault_with_imported_row();
    let conn = v.db().unwrap();
    let bank_id = insert_row(&conn, account, "fints-1", "Shop am Markt");
    let merge = |original: &str| {
        merge_synced_twin_or_original(
            &conn,
            bank_id,
            "2025-01-03",
            -999,
            "",
            "Shop am Markt",
            original,
            "Abo",
        )
        .unwrap()
    };
    assert!(!merge(""));
    assert_eq!(live_rows(&conn), 2);
    assert!(merge("Zahlungsdienst GmbH"));
    assert_eq!(live_rows(&conn), 1);
}

#[test]
fn unchanged_name_still_adopts() {
    let conn = conn_with_legacy_row();
    let t = fetched("Zahlungsdienst GmbH", "", IBAN);
    assert!(adopt_legacy_content_id(&conn, 1, &t).unwrap());
    assert_eq!(stored_id(&conn), t.external_id);
}

#[test]
fn repaired_original_name_adopts_the_row_stored_cleaned() {
    // Older versions stored the `Cdtr` name through `clean_bank_text`.
    let conn = conn_with_legacy_row_named("Müller Versand");
    let mut t = fetched("Shop am Markt", "MÃ¼ller Versand", "");
    let id = t.external_id.clone();
    clean_fetched_text(&mut t);
    assert_eq!(t.original_counterparty, "Müller Versand");
    // the content id depends only on the counterparty and stays the same
    assert_eq!(t.external_id, id);
    assert_eq!(t.content_id(0), id);
    assert!(adopt_legacy_content_id(&conn, 1, &t).unwrap());
    assert_eq!(stored_id(&conn), id);
}

#[test]
fn ultimate_name_replaces_only_the_stored_original_name() {
    let t = fetched("Shop am Markt", "Zahlungsdienst GmbH", IBAN);
    assert_eq!(
        ultimate_rename("Zahlungsdienst GmbH", &t),
        Some("Shop am Markt")
    );
    assert_eq!(
        ultimate_rename(" Zahlungsdienst GmbH ", &t),
        Some("Shop am Markt")
    );
    // a name from another source stays
    assert_eq!(ultimate_rename("Eigener Name", &t), None);
    // already the ultimate name: nothing to write
    assert_eq!(ultimate_rename("Shop am Markt", &t), None);
    // no ultimate party delivered: the stored name stays
    let plain = fetched("Zahlungsdienst GmbH", "", IBAN);
    assert_eq!(ultimate_rename("Zahlungsdienst GmbH", &plain), None);
    assert_eq!(ultimate_rename("Anderer Name", &plain), None);
}
