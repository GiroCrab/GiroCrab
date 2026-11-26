//! Legacy id adoption (`adopt_legacy_content_id`, #1721): a booking whose
//! ultimate party replaced the `Cdtr`/`Dbtr` name takes over its legacy row;
//! another booking that only shares the payment provider's IBAN never does.

use super::*;

const IBAN: &str = "DE00000000000000000000";
const LEGACY_ID: &str = "h0123456789abcdef";

fn conn_with_legacy_row() -> rusqlite::Connection {
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
         VALUES (1, ?1, '2025-01-02', '2025-01-02', -2345, 'Zahlungsdienst GmbH', ?2,
             'Bestellung 4711')",
        params![LEGACY_ID, IBAN],
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

#[test]
fn unchanged_name_still_adopts() {
    let conn = conn_with_legacy_row();
    let t = fetched("Zahlungsdienst GmbH", "", IBAN);
    assert!(adopt_legacy_content_id(&conn, 1, &t).unwrap());
    assert_eq!(stored_id(&conn), t.external_id);
}
