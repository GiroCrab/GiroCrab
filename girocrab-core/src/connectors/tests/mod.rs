use std::sync::Arc;

use rusqlite::params;

use super::*;
use crate::error::CoreError;
use crate::models::{AccountKind, BookingFilter, BookingScope, ConnectorKind, SepaTransferRequest};
use crate::vault::Vault;

#[test]
fn errors_after_submit_leave_outcome_unknown() {
    use super::{error_leaves_outcome_unknown as unknown, CoreError};
    assert!(unknown(&CoreError::Network {
        message: "x".into()
    }));
    assert!(unknown(&CoreError::Protocol {
        message: "x".into()
    }));
    assert!(!unknown(&CoreError::Authentication {
        message: "x".into()
    }));
    assert!(!unknown(&CoreError::InvalidInput {
        message: "x".into()
    }));
}

#[test]
fn store_fetch_matches_depot_holdings_without_iban() {
    let p_dir = crate::test_support::TempDir::new("girocrab-depot");
    let p = p_dir.join("v.db");
    let v = Vault::open(p.to_string_lossy().to_string(), vec![3u8; 32]).unwrap();
    let conn = v
        .add_bank_connection(ConnectorKind::Comdirect, String::new(), "login".into())
        .unwrap();
    let mk = |r: &str| RemoteHolding {
        classification: Default::default(),
        direct_crypto: false,

        account_ref: r.into(),
        holding: crate::models::Holding {
            isin: "DE0001234567".into(),
            name: "X".into(),
            quantity: 2.0,
            value_cents: 100,
            ..Default::default()
        },
    };
    let fetch = |hold_ref: &str| FetchResult {
        accounts: vec![RemoteAccount {
            external_ref: "0012345678".into(),
            name: "Depot".into(),
            owner: String::new(),
            iban: String::new(),
            account_number: "0012345678".into(),
            currency: "EUR".into(),
            kind: AccountKind::Brokerage,
            balance_cents: 5000,
            balance_date: "2026-11-10".into(),
        }],
        holdings: vec![mk(hold_ref)],
        ..Default::default()
    };
    // differently formatted ref (no leading zeros) and an unknown ref (single-depot fallback)
    for r in ["12345678", "unknown"] {
        store_fetch(&v, ConnectorKind::Comdirect, conn, fetch(r)).unwrap();
        let acc = v.list_accounts().unwrap().pop().unwrap();
        assert_eq!(v.list_holdings(acc.id).unwrap().len(), 1, "ref {r}");
    }
}

#[test]
fn history_bookings_arrive_checked_and_stored_documents_get_linked() {
    let p_dir = crate::test_support::TempDir::new("girocrab-history-docs");
    let p = p_dir.join("v.db");
    let v = Vault::open(p.to_string_lossy().to_string(), vec![3u8; 32]).unwrap();
    let conn = v
        .add_bank_connection(
            ConnectorKind::TradeRepublic,
            String::new(),
            "+491701234567".into(),
        )
        .unwrap();
    let account = RemoteAccount {
        external_ref: "tr:cash".into(),
        name: "Verrechnungskonto".into(),
        owner: String::new(),
        iban: String::new(),
        account_number: String::new(),
        currency: "EUR".into(),
        kind: AccountKind::Checking,
        balance_cents: 1000,
        balance_date: "2026-11-10".into(),
    };
    let doc = |bytes: &[u8], booking: Option<&str>| crate::documents::RemoteDocument {
        account_ref: "tr:cash".into(),
        booking_external_id: booking.map(str::to_string),
        title: "Abrechnung – Beispiel".into(),
        doc_type: "abrechnung".into(),
        mime_type: "application/pdf".into(),
        file_name: "a.pdf".into(),
        bytes: bytes.to_vec(),
        source_url: "https://example.invalid/doc/1".into(),
        date: "2024-05-02".into(),
    };
    // first the PDF alone (its booking is older than the sync window of that time)
    let first = FetchResult {
        accounts: vec![account.clone()],
        documents: vec![doc(b"%PDF-1.4 test", None)],
        ..Default::default()
    };
    let s = store_fetch(&v, ConnectorKind::TradeRepublic, conn, first).unwrap();
    assert_eq!(s.documents_imported, 1);
    let acc = v.list_accounts().unwrap().pop().unwrap();
    let docs = v.list_all_documents(Default::default()).unwrap();
    assert!(docs.iter().all(|d| d.booking_id.is_none()));

    // later the history pass brings the booking (as checked) and the link-only document
    let booking = RemoteTransaction {
        account_ref: "tr:cash".into(),
        external_id: "item-1".into(),
        booking_date: "2024-05-02".into(),
        value_date: "2024-05-02".into(),
        amount_cents: -7500,
        currency: "EUR".into(),
        counterparty: "Beispiel".into(),
        checked: true,
        ..Default::default()
    };
    let second = FetchResult {
        accounts: vec![account],
        transactions: vec![booking],
        documents: vec![doc(&[], Some("item-1"))],
        ..Default::default()
    };
    let s = store_fetch(&v, ConnectorKind::TradeRepublic, conn, second).unwrap();
    assert_eq!(
        s.documents_imported, 0,
        "a link-only document is no new document"
    );
    assert_eq!(
        v.list_accounts().unwrap().pop().unwrap().new_count,
        0,
        "history is not new"
    );
    let _ = acc;
    let docs = v.list_all_documents(Default::default()).unwrap();
    assert_eq!(docs.len(), 1);
    assert!(
        docs[0].booking_id.is_some(),
        "the stored document is linked now"
    );
}

#[test]
fn unread_depot_keeps_its_positions_and_value() {
    // A sync that could not read the depot (Trade Republic: portfolio request failed)
    // still lists the account, with a placeholder balance of 0.
    let p_dir = crate::test_support::TempDir::new("girocrab-depot-unread");
    let p = p_dir.join("v.db");
    let v = Vault::open(p.to_string_lossy().to_string(), vec![3u8; 32]).unwrap();
    let conn = v
        .add_bank_connection(ConnectorKind::TradeRepublic, String::new(), "login".into())
        .unwrap();
    let depot = |balance_cents: i64| RemoteAccount {
        external_ref: "tr:depot".into(),
        name: "Depot".into(),
        owner: String::new(),
        iban: String::new(),
        account_number: String::new(),
        currency: "EUR".into(),
        kind: AccountKind::Brokerage,
        balance_cents,
        balance_date: "2026-11-10".into(),
    };
    let holding = RemoteHolding {
        account_ref: "tr:depot".into(),
        holding: crate::models::Holding {
            isin: "DE0001234567".into(),
            name: "X".into(),
            quantity: 2.0,
            value_cents: 5000,
            ..Default::default()
        },
        ..Default::default()
    };
    let read = FetchResult {
        accounts: vec![depot(5000)],
        holdings: vec![holding],
        holdings_fetched: vec!["tr:depot".into()],
        ..Default::default()
    };
    store_fetch(&v, ConnectorKind::TradeRepublic, conn, read).unwrap();

    let unread = FetchResult {
        accounts: vec![depot(0)],
        balances_unknown: vec!["tr:depot".into()],
        ..Default::default()
    };
    store_fetch(&v, ConnectorKind::TradeRepublic, conn, unread).unwrap();
    let acc = v.list_accounts().unwrap().pop().unwrap();
    assert_eq!(acc.balance_cents, 5000);
    assert_eq!(v.list_holdings(acc.id).unwrap().len(), 1);

    // read and empty: everything was sold
    let sold = FetchResult {
        accounts: vec![depot(0)],
        holdings_fetched: vec!["tr:depot".into()],
        ..Default::default()
    };
    store_fetch(&v, ConnectorKind::TradeRepublic, conn, sold).unwrap();
    let acc = v.list_accounts().unwrap().pop().unwrap();
    assert_eq!(acc.balance_cents, 0);
    assert!(v.list_holdings(acc.id).unwrap().is_empty());
}

#[test]
fn depot_value_points_follow_what_a_fetch_confirms() {
    // total without positions, unknown balance, unconfirmed 0, confirmed 0
    let p_dir = crate::test_support::TempDir::new("girocrab-depot-points");
    let p = p_dir.join("v.db");
    let v = Vault::open(p.to_string_lossy().to_string(), vec![3u8; 32]).unwrap();
    let conn = v
        .add_bank_connection(ConnectorKind::TradeRepublic, String::new(), "login".into())
        .unwrap();
    let depot = |balance_cents: i64, date: &str| RemoteAccount {
        external_ref: "tr:depot".into(),
        name: "Depot".into(),
        owner: String::new(),
        iban: String::new(),
        account_number: String::new(),
        currency: "EUR".into(),
        kind: AccountKind::Brokerage,
        balance_cents,
        balance_date: date.into(),
    };
    let points = |v: &Vault| -> Vec<(String, i64)> {
        let db = v.db().unwrap();
        let mut q = db
            .prepare("SELECT value_date, value_cents FROM depot_values ORDER BY value_date")
            .unwrap();
        q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    let fetch = |accounts, unknown: bool, fetched: bool| FetchResult {
        accounts,
        balances_unknown: if unknown {
            vec!["tr:depot".into()]
        } else {
            vec![]
        },
        holdings_fetched: if fetched {
            vec!["tr:depot".into()]
        } else {
            vec![]
        },
        ..Default::default()
    };

    // 1. a total without positions (an overview import) is a value
    let f = fetch(vec![depot(7_000_00, "2026-11-01")], false, false);
    store_fetch(&v, ConnectorKind::TradeRepublic, conn, f).unwrap();
    assert_eq!(points(&v), vec![("2026-11-01".to_string(), 7_000_00)]);

    // 2. a balance that was not read leaves no point
    let f = fetch(vec![depot(0, "2026-11-02")], true, false);
    store_fetch(&v, ConnectorKind::TradeRepublic, conn, f).unwrap();
    assert_eq!(points(&v).len(), 1);

    // 3. a 0 without a confirmed inventory may only mean "not reported": no point
    let f = fetch(vec![depot(0, "2026-11-03")], false, false);
    store_fetch(&v, ConnectorKind::TradeRepublic, conn, f).unwrap();
    assert_eq!(points(&v).len(), 1);

    // 4. an inventory that was read and is empty: the 0 is confirmed
    let f = fetch(vec![depot(0, "2026-11-04")], false, true);
    store_fetch(&v, ConnectorKind::TradeRepublic, conn, f).unwrap();
    assert_eq!(
        points(&v),
        vec![
            ("2026-11-01".to_string(), 7_000_00),
            ("2026-11-04".to_string(), 0)
        ]
    );
}

#[test]
fn archived_psd2_account_is_not_reported_as_revoked() {
    // #65 (archive) + #66 (revoked marking): an account the user archived
    // is skipped by store_fetch, so it must not look "missing" from a
    // complete PSD2 sync.
    let p_dir = crate::test_support::TempDir::new("girocrab-arch-eb");
    let p = p_dir.join("v.db");
    let v = Vault::open(p.to_string_lossy().to_string(), vec![3u8; 32]).unwrap();
    let conn = v
        .add_bank_connection(ConnectorKind::EnableBanking, String::new(), "C24".into())
        .unwrap();
    let acc = |r: &str, name: &str| RemoteAccount {
        external_ref: r.into(),
        name: name.into(),
        owner: String::new(),
        iban: String::new(),
        account_number: String::new(),
        currency: "EUR".into(),
        kind: AccountKind::Checking,
        balance_cents: 100,
        balance_date: "2026-11-11".into(),
    };
    let both = || vec![acc("G", "C24 Smartkonto"), acc("P", "C24 Pocket")];
    store_fetch(
        &v,
        ConnectorKind::EnableBanking,
        conn,
        FetchResult {
            accounts: both(),
            accounts_complete: true,
            ..Default::default()
        },
    )
    .unwrap();
    let pocket = v
        .list_accounts()
        .unwrap()
        .into_iter()
        .find(|a| a.name == "C24 Pocket")
        .unwrap();
    v.set_account_archived(pocket.id, true).unwrap();
    // the bank still delivers both
    let s = store_fetch(
        &v,
        ConnectorKind::EnableBanking,
        conn,
        FetchResult {
            accounts: both(),
            accounts_complete: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(s.revoked_accounts.is_empty(), "{:?}", s.revoked_accounts);
    // the bank no longer delivers the archived one: still no notice
    let s = store_fetch(
        &v,
        ConnectorKind::EnableBanking,
        conn,
        FetchResult {
            accounts: vec![acc("G", "C24 Smartkonto")],
            accounts_complete: true,
            revoked_refs: vec!["P".into()],
            ..Default::default()
        },
    )
    .unwrap();
    assert!(s.revoked_accounts.is_empty(), "{:?}", s.revoked_accounts);
    let st = v.connection_status(conn).unwrap();
    assert_eq!(st.revoked_account_count, 0);
    std::fs::remove_file(p).ok();
}

#[test]
fn store_fetch_skips_archived_accounts() {
    let p_dir = crate::test_support::TempDir::new("girocrab-arch");
    let p = p_dir.join("v.db");
    let v = Vault::open(p.to_string_lossy().to_string(), vec![3u8; 32]).unwrap();
    let conn = v
        .add_bank_connection(ConnectorKind::Comdirect, String::new(), "login".into())
        .unwrap();
    let acc = |r: &str, name: &str, balance: i64| RemoteAccount {
        external_ref: r.into(),
        name: name.into(),
        owner: String::new(),
        iban: String::new(),
        account_number: r.into(),
        currency: "EUR".into(),
        kind: AccountKind::Checking,
        balance_cents: balance,
        balance_date: "2026-11-10".into(),
    };
    let tx = |r: &str, id: &str| RemoteTransaction {
        account_ref: r.into(),
        external_id: id.into(),
        booking_date: "2026-11-09".into(),
        value_date: "2026-11-09".into(),
        amount_cents: -500,
        currency: "EUR".into(),
        counterparty: "Laden".into(),
        counterparty_iban: String::new(),
        purpose: String::new(),
        booking_text: String::new(),
        ..Default::default()
    };
    let fetch = |balance: i64, ids: &[&str]| FetchResult {
        accounts: vec![
            acc("alt", "Altes Konto", balance),
            acc("neu", "Neues Konto", balance),
        ],
        transactions: ids
            .iter()
            .flat_map(|id| [tx("alt", id), tx("neu", id)])
            .collect(),
        ..Default::default()
    };
    let s = store_fetch(&v, ConnectorKind::Comdirect, conn, fetch(1000, &["t1"])).unwrap();
    assert_eq!(
        (s.transactions_imported, s.transactions_skipped_archived),
        (2, 0)
    );
    let old = v
        .list_accounts()
        .unwrap()
        .into_iter()
        .find(|a| a.name == "Altes Konto")
        .unwrap();
    v.set_account_archived(old.id, true).unwrap();
    assert!(!v.list_bank_connections().unwrap()[0].all_accounts_archived);

    // t1 is known, t2 is new: only the active account takes it
    let s = store_fetch(
        &v,
        ConnectorKind::Comdirect,
        conn,
        fetch(2000, &["t1", "t2"]),
    )
    .unwrap();
    assert_eq!(
        (
            s.accounts_updated,
            s.transactions_imported,
            s.transactions_skipped_archived
        ),
        (1, 1, 1)
    );
    let accs = v.list_accounts().unwrap();
    assert_eq!(accs.len(), 2, "no second account for the archived one");
    let by = |n: &str| accs.iter().find(|a| a.name == n).unwrap();
    assert_eq!(
        (
            by("Altes Konto").balance_cents,
            by("Altes Konto").transaction_count
        ),
        (1000, 1)
    );
    assert_eq!(
        (
            by("Neues Konto").balance_cents,
            by("Neues Konto").transaction_count
        ),
        (2000, 2)
    );

    v.set_account_archived(by("Neues Konto").id, true).unwrap();
    assert!(v.list_bank_connections().unwrap()[0].all_accounts_archived);
    let _ = std::fs::remove_file(p);
}

#[test]
fn card_open_balance_since_settlement() {
    let r = |d: &str, a: i64, t: &str| (d.to_string(), a, t.to_string());
    let rows = vec![
        r("2026-09-20", -5000, "Shop"),
        r("2026-10-05", 5000, "LASTSCHRIFT Kartenabrechnung"),
        r("2026-10-10", -1200, "Cafe"),
        r("2026-11-02", -3400, "Hotel"),
    ];
    assert_eq!(open_card_balance(&rows), Some(-4600));
    let no_settle = vec![
        r("2026-10-30", -100, "x"),
        r("2026-11-01", -200, "y"),
        r("2026-11-03", -300, "z"),
    ];
    assert_eq!(open_card_balance(&no_settle), Some(-600));
    let with_refund = vec![
        r("2026-10-20", -170224, "Shops"),
        r("2026-11-02", 33853, "Refund Merchant"),
        r("2026-11-05", -1000, "Cafe"),
    ];
    assert_eq!(open_card_balance(&with_refund), Some(-137371));
    assert_eq!(open_card_balance(&[]), None);
}

fn demo_vault() -> (Arc<Vault>, std::path::PathBuf) {
    let path_dir = crate::test_support::TempDir::new("girocrab-sync");
    let path = path_dir.join("v.db");
    let v = Vault::open(path.to_string_lossy().to_string(), vec![3u8; 32]).unwrap();
    v.seed_demo_data().unwrap();
    (v, path)
}

#[test]
fn card_holds_are_replaced_on_each_sync() {
    let (v, path) = demo_vault();
    let conn = v
        .add_bank_connection(ConnectorKind::Bunq, String::new(), "me@example.com".into())
        .unwrap();
    let acc = RemoteAccount {
        external_ref: "bunq:1".into(),
        name: "bunq".into(),
        owner: String::new(),
        iban: "NL00BUNQ0000000000".into(),
        account_number: String::new(),
        currency: "EUR".into(),
        kind: AccountKind::Checking,
        balance_cents: 0,
        balance_date: "2026-11-10".into(),
    };
    let hold = |id: &str, cents: i64| RemoteTransaction {
        account_ref: "bunq:1".into(),
        external_id: format!("pending-hold-bunq-{id}"),
        booking_date: "2026-11-09".into(),
        value_date: "2026-11-09".into(),
        amount_cents: cents,
        currency: "EUR".into(),
        counterparty: "Hotel".into(),
        counterparty_iban: String::new(),
        purpose: String::new(),
        booking_text: "Kartenzahlung (vorgemerkt)".into(),
        ..Default::default()
    };
    let fetch = |txs: Vec<RemoteTransaction>| FetchResult {
        accounts: vec![acc.clone()],
        transactions: txs,
        hold_prefix: Some("pending-hold-bunq-".into()),
        ..Default::default()
    };
    let holds = || -> Vec<String> {
        let db = v.db().unwrap();
        let mut q = db
            .prepare("SELECT external_id FROM transactions WHERE external_id LIKE 'pending-hold-%' AND deleted_ms IS NULL ORDER BY 1")
            .unwrap();
        q.query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    };
    store_fetch(
        &v,
        ConnectorKind::Bunq,
        conn,
        fetch(vec![hold("1", -50_00), hold("2", -7_10)]),
    )
    .unwrap();
    assert_eq!(holds(), vec!["pending-hold-bunq-1", "pending-hold-bunq-2"]);
    // Hold 1 cleared (booked elsewhere), hold 2 still open.
    store_fetch(&v, ConnectorKind::Bunq, conn, fetch(vec![hold("2", -7_10)])).unwrap();
    assert_eq!(holds(), vec!["pending-hold-bunq-2"]);
    std::fs::remove_file(path).ok();
}

#[test]
fn pending_batch_booking_is_replaced_by_the_banks_booking_whatever_its_iban() {
    let (v, path) = demo_vault();
    let conn = v
        .add_bank_connection(ConnectorKind::FinTsGeneric, String::new(), "u".into())
        .unwrap();
    let acc = RemoteAccount {
        external_ref: "f:1".into(),
        name: "Giro".into(),
        owner: String::new(),
        iban: "DE02120300000000202051".into(),
        account_number: String::new(),
        currency: "EUR".into(),
        kind: AccountKind::Checking,
        balance_cents: 0,
        balance_date: "2026-11-10".into(),
    };
    let fetch = |txs: Vec<RemoteTransaction>| FetchResult {
        accounts: vec![acc.clone()],
        transactions: txs,
        ..Default::default()
    };
    store_fetch(&v, ConnectorKind::FinTsGeneric, conn, fetch(vec![])).unwrap();
    let account_id: i64 = v
        .db()
        .unwrap()
        .query_row(
            "SELECT id FROM accounts WHERE external_ref = 'f:1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let summary = |e2e: &str, cents: i64| TransferJob {
        source_iban: acc.iban.clone(),
        source_owner: "A".into(),
        recipient_name: "Sammelüberweisung (3 Zahlungen)".into(),
        recipient_iban: String::new(),
        recipient_bic: None,
        amount_cents: cents,
        purpose: String::new(),
        instant: false,
        execution_date: None,
        end_to_end_id: e2e.into(),
    };
    v.record_pending_transfer(
        account_id,
        &summary("A", 3_550),
        "Sammelüberweisung (vorgemerkt)",
    )
    .unwrap();
    v.record_pending_transfer(
        account_id,
        &summary("B", 9_900),
        "Sammelüberweisung (vorgemerkt)",
    )
    .unwrap();
    let pending = || -> i64 {
        v.db()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM transactions WHERE external_id LIKE 'pending-%' AND deleted_ms IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_eq!(pending(), 2);
    // A single payment of the same size is no batch booking: both stay pending.
    let today = crate::dates::today().to_string();
    let other = RemoteTransaction {
        account_ref: "f:1".into(),
        external_id: "bank-0".into(),
        booking_date: today.clone(),
        value_date: today,
        amount_cents: -3_550,
        currency: "EUR".into(),
        counterparty: "Shop".into(),
        counterparty_iban: "DE89370400440532013000".into(),
        booking_text: "Kartenzahlung".into(),
        ..Default::default()
    };
    store_fetch(&v, ConnectorKind::FinTsGeneric, conn, fetch(vec![other])).unwrap();
    assert_eq!(pending(), 2);
    // The bank books 35,50 € and names the own IBAN as counterparty.
    let booked = RemoteTransaction {
        account_ref: "f:1".into(),
        external_id: "bank-1".into(),
        booking_date: crate::dates::today().to_string(),
        value_date: crate::dates::today().to_string(),
        amount_cents: -3_550,
        currency: "EUR".into(),
        counterparty: "A".into(),
        counterparty_iban: acc.iban.clone(),
        booking_text: "SAMMELÜBERWEISUNG".into(),
        ..Default::default()
    };
    store_fetch(&v, ConnectorKind::FinTsGeneric, conn, fetch(vec![booked])).unwrap();
    // Only the matching one is gone; the other batch is still pending.
    assert_eq!(pending(), 1);
    std::fs::remove_file(path).ok();
}

#[test]
fn standing_orders_are_replaced_only_for_completely_fetched_accounts() {
    let (v, path) = demo_vault();
    let conn = v
        .add_bank_connection(ConnectorKind::FinTsGeneric, String::new(), "u".into())
        .unwrap();
    let acc = RemoteAccount {
        external_ref: "f:1".into(),
        name: "Giro".into(),
        owner: String::new(),
        iban: "DE02120300000000202051".into(),
        account_number: String::new(),
        currency: "EUR".into(),
        kind: AccountKind::Checking,
        balance_cents: 0,
        balance_date: "2026-11-10".into(),
    };
    let order = RemoteStandingOrder {
        account_ref: "f:1".into(),
        order_id: "O1".into(),
        recipient_name: "Vermieter".into(),
        recipient_iban: "DE89370400440532013000".into(),
        amount_cents: 85_000,
        purpose: "Miete".into(),
        first_date: "2026-01-01".into(),
        last_date: None,
        time_unit: "M".into(),
        interval: 1,
        exec_day: 1,
        ..Default::default()
    };
    let count = || -> i64 {
        v.db()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM standing_orders WHERE key = 'O1'",
                [],
                |r| r.get(0),
            )
            .unwrap()
    };
    let fetch = |orders: Vec<RemoteStandingOrder>, complete: bool| FetchResult {
        accounts: vec![acc.clone()],
        standing_orders: orders,
        standing_orders_fetched: if complete { vec!["f:1".into()] } else { vec![] },
        ..Default::default()
    };
    store_fetch(
        &v,
        ConnectorKind::FinTsGeneric,
        conn,
        fetch(vec![order], true),
    )
    .unwrap();
    assert_eq!(count(), 1);
    // A sync without the standing-order request (bank lacks it, or it failed) keeps them.
    store_fetch(&v, ConnectorKind::FinTsGeneric, conn, fetch(vec![], false)).unwrap();
    assert_eq!(count(), 1);
    // A complete answer without orders means: the user deleted them at the bank.
    store_fetch(&v, ConnectorKind::FinTsGeneric, conn, fetch(vec![], true)).unwrap();
    assert_eq!(count(), 0);
    std::fs::remove_file(path).ok();
}

#[test]
fn revoked_accounts_are_marked_and_new_ones_named() {
    let (v, path) = demo_vault();
    let conn = v
        .add_bank_connection(ConnectorKind::EnableBanking, String::new(), "C24".into())
        .unwrap();
    let acc = |r: &str, name: &str| RemoteAccount {
        external_ref: r.into(),
        name: name.into(),
        owner: String::new(),
        iban: String::new(),
        account_number: String::new(),
        currency: "EUR".into(),
        kind: AccountKind::Checking,
        balance_cents: 100_00,
        balance_date: "2026-11-11".into(),
    };
    let revoked = || -> Vec<(String, bool)> {
        let db = v.db().unwrap();
        let mut q = db
            .prepare("SELECT external_ref, access_revoked FROM accounts WHERE connection_id = ?1 ORDER BY external_ref")
            .unwrap();
        q.query_map([conn], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    };
    // first sync: nothing is "new"
    let s = store_fetch(
        &v,
        ConnectorKind::EnableBanking,
        conn,
        FetchResult {
            accounts: vec![
                acc("A", "Girokonto"),
                acc("B", "Pocket"),
                acc("C", "Gemeinschaftskonto"),
            ],
            ..Default::default()
        },
    )
    .unwrap();
    assert!(s.new_accounts.is_empty() && s.revoked_accounts.is_empty());
    // reconnect with only A: B and C marked, named once, bookings kept
    let s = store_fetch(
        &v,
        ConnectorKind::EnableBanking,
        conn,
        FetchResult {
            accounts: vec![acc("A", "Girokonto")],
            revoked_refs: vec!["B".into(), "C".into()],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(s.revoked_accounts, vec!["Pocket", "Gemeinschaftskonto"]);
    assert_eq!(
        revoked(),
        vec![("A".into(), false), ("B".into(), true), ("C".into(), true)]
    );
    let st = v.connection_status(conn).unwrap();
    assert_eq!((st.account_count, st.revoked_account_count), (3, 2));
    // repeated report: no second notice
    let s = store_fetch(
        &v,
        ConnectorKind::EnableBanking,
        conn,
        FetchResult {
            accounts: vec![acc("A", "Girokonto")],
            revoked_refs: vec!["B".into()],
            ..Default::default()
        },
    )
    .unwrap();
    assert!(s.revoked_accounts.is_empty());
    // later consent includes B again plus a new account D
    let s = store_fetch(
        &v,
        ConnectorKind::EnableBanking,
        conn,
        FetchResult {
            accounts: vec![
                acc("A", "Girokonto"),
                acc("B", "Pocket"),
                acc("D", "GLS Giro"),
            ],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(s.new_accounts, vec!["GLS Giro"]);
    assert_eq!(
        revoked(),
        vec![
            ("A".into(), false),
            ("B".into(), false),
            ("C".into(), true),
            ("D".into(), false)
        ]
    );
    let accs = v.list_accounts().unwrap();
    assert!(accs
        .iter()
        .any(|a| a.name == "Gemeinschaftskonto" && a.access_revoked));
    std::fs::remove_file(path).ok();
}

#[test]
fn automatic_names_and_bank_name_are_updated_user_names_kept() {
    let (v, path) = demo_vault();
    let conn = v
        .add_bank_connection(ConnectorKind::EnableBanking, String::new(), "C24".into())
        .unwrap();
    let acc = |r: &str, name: &str, owner: &str| RemoteAccount {
        external_ref: r.into(),
        name: name.into(),
        owner: owner.into(),
        iban: String::new(),
        account_number: String::new(),
        currency: "EUR".into(),
        kind: AccountKind::Checking,
        balance_cents: 0,
        balance_date: "2026-11-11".into(),
    };
    // as stored by the earlier build: holder as name, generic bank name
    store_fetch(
        &v,
        ConnectorKind::EnableBanking,
        conn,
        FetchResult {
            accounts: vec![acc("G", "Jay Licina", ""), acc("S", "Jay Licina", "")],
            ..Default::default()
        },
    )
    .unwrap();
    let db_rename = |r: &str, n: &str| {
        v.db()
            .unwrap()
            .execute(
                "UPDATE accounts SET name = ?1 WHERE external_ref = ?2",
                params![n, r],
            )
            .unwrap();
    };
    db_rename("S", "Mein Sparkonto"); // the user renamed this one
    store_fetch(
        &v,
        ConnectorKind::EnableBanking,
        conn,
        FetchResult {
            accounts: vec![
                acc("G", "C24 Smartkonto", "Jay Licina"),
                acc("S", "C24 PocketZINS Smart", "Jay Licina"),
            ],
            bank_name: Some("C24".into()),
            name_updates: vec![
                ("G".into(), "Jay Licina".into(), "C24 Smartkonto".into()),
                (
                    "S".into(),
                    "Jay Licina".into(),
                    "C24 PocketZINS Smart".into(),
                ),
            ],
            ..Default::default()
        },
    )
    .unwrap();
    let rows: Vec<(String, String, String, String)> = {
        let db = v.db().unwrap();
        let mut q = db
            .prepare("SELECT external_ref, name, bank_name, owner FROM accounts WHERE connection_id = ?1 ORDER BY external_ref")
            .unwrap();
        q.query_map([conn], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    };
    assert_eq!(
        rows,
        vec![
            (
                "G".into(),
                "C24 Smartkonto".into(),
                "C24".into(),
                "Jay Licina".into()
            ),
            (
                "S".into(),
                "Mein Sparkonto".into(),
                "C24".into(),
                "Jay Licina".into()
            ),
        ]
    );
    std::fs::remove_file(path).ok();
}

#[test]
fn complete_account_list_marks_the_missing_ones() {
    let (v, path) = demo_vault();
    let conn = v
        .add_bank_connection(ConnectorKind::EnableBanking, String::new(), "C24".into())
        .unwrap();
    let acc = |r: &str, name: &str| RemoteAccount {
        external_ref: r.into(),
        name: name.into(),
        owner: String::new(),
        iban: String::new(),
        account_number: String::new(),
        currency: "EUR".into(),
        kind: AccountKind::Checking,
        balance_cents: 0,
        balance_date: "2026-11-11".into(),
    };
    let three = || {
        vec![
            acc("DE23", "Girokonto"),
            acc("hs", "PocketZINS"),
            acc("DE48", "Gemeinschaftskonto"),
        ]
    };
    store_fetch(
        &v,
        ConnectorKind::EnableBanking,
        conn,
        FetchResult {
            accounts: three(),
            accounts_complete: true,
            ..Default::default()
        },
    )
    .unwrap();
    // a partial sync (e.g. bank's daily limit) marks nothing
    let s = store_fetch(
        &v,
        ConnectorKind::EnableBanking,
        conn,
        FetchResult {
            accounts: vec![acc("DE23", "Girokonto")],
            accounts_complete: false,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(s.revoked_accounts.is_empty());
    // an empty "complete" list is never trusted
    let s = store_fetch(
        &v,
        ConnectorKind::EnableBanking,
        conn,
        FetchResult {
            accounts_complete: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(s.revoked_accounts.is_empty());
    // the real case: new consent with only the main account
    let s = store_fetch(
        &v,
        ConnectorKind::EnableBanking,
        conn,
        FetchResult {
            accounts: vec![acc("DE23", "Girokonto")],
            accounts_complete: true,
            ..Default::default()
        },
    )
    .unwrap();
    let mut names = s.revoked_accounts.clone();
    names.sort();
    assert_eq!(names, vec!["Gemeinschaftskonto", "PocketZINS"]);
    assert_eq!(v.connection_status(conn).unwrap().revoked_account_count, 2);
    // next sync: no repeated notice
    let s = store_fetch(
        &v,
        ConnectorKind::EnableBanking,
        conn,
        FetchResult {
            accounts: vec![acc("DE23", "Girokonto")],
            accounts_complete: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(s.revoked_accounts.is_empty());
    // all three granted again: marks cleared
    store_fetch(
        &v,
        ConnectorKind::EnableBanking,
        conn,
        FetchResult {
            accounts: three(),
            accounts_complete: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(v.connection_status(conn).unwrap().revoked_account_count, 0);
    std::fs::remove_file(path).ok();
}

#[test]
fn renamed_refs_keep_the_account() {
    let (v, path) = demo_vault();
    let conn = v
        .add_bank_connection(ConnectorKind::EnableBanking, String::new(), "C24".into())
        .unwrap();
    let acc = |r: &str| RemoteAccount {
        external_ref: r.into(),
        name: "Girokonto".into(),
        owner: String::new(),
        iban: "DE00100000000000000001".into(),
        account_number: String::new(),
        currency: "EUR".into(),
        kind: AccountKind::Checking,
        balance_cents: 100_00,
        balance_date: "2026-11-10".into(),
    };
    let count = || -> i64 {
        v.db()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM accounts WHERE connection_id = ?1",
                [conn],
                |r| r.get(0),
            )
            .unwrap()
    };
    let uid = "3fa85f64-5717-4562-b3fc-2c963f66afa6";
    let stable = "DE00100000000000000001";
    store_fetch(
        &v,
        ConnectorKind::EnableBanking,
        conn,
        FetchResult {
            accounts: vec![acc(uid)],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(count(), 1);
    store_fetch(
        &v,
        ConnectorKind::EnableBanking,
        conn,
        FetchResult {
            accounts: vec![acc(stable)],
            renamed_refs: vec![(uid.into(), stable.into())],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(count(), 1, "rename must not create a second account");
    // applying the same rename again is harmless
    store_fetch(
        &v,
        ConnectorKind::EnableBanking,
        conn,
        FetchResult {
            accounts: vec![acc(stable)],
            renamed_refs: vec![(uid.into(), stable.into())],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(count(), 1);
    // a later sync recognises it as savings and learns its BBAN
    let mut savings = acc(stable);
    savings.kind = AccountKind::Savings;
    savings.account_number = "500240240000000099".into();
    store_fetch(
        &v,
        ConnectorKind::EnableBanking,
        conn,
        FetchResult {
            accounts: vec![savings],
            ..Default::default()
        },
    )
    .unwrap();
    let (kind, number): (String, String) = v
        .db()
        .unwrap()
        .query_row(
            "SELECT kind, IFNULL(account_number, '') FROM accounts WHERE connection_id = ?1",
            [conn],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        (kind.as_str(), number.as_str()),
        ("savings", "500240240000000099")
    );
    assert_eq!(count(), 1);
    std::fs::remove_file(path).ok();
}

#[tokio::test]
async fn magic_demo_login_syncs_without_network_or_product_id() {
    let dir = std::env::temp_dir().join(format!("gc-magic-demo-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("v.girocrab");
    let v = Vault::open(path.to_str().unwrap().into(), vec![9u8; 32]).unwrap();
    // Real (non-demo) flag, no FinTS product ID set: only the login "Demo" makes it work.
    let id = v
        .add_fints_connection(
            "99999999".into(),
            "https://demo.invalid/fints".into(),
            "Demo-Bank".into(),
            " Demo ".into(),
            String::new(),
        )
        .unwrap();
    let s = v.clone().begin_sync(id, "12345".into(), false).unwrap();
    assert!(matches!(
        s.start().await.unwrap(),
        SyncStep::NeedsChallenge { .. }
    ));
    let _ = s.poll().await.unwrap();
    match s.poll().await.unwrap() {
        SyncStep::Finished { .. } => {}
        other => panic!("expected finish, got {other:?}"),
    }
    assert!(v
        .list_accounts()
        .unwrap()
        .iter()
        .any(|a| a.connection_id == Some(id)));
    // Any other login still needs the product ID.
    let other = v
        .add_fints_connection(
            "88888888".into(),
            "https://x.example/fints".into(),
            "X".into(),
            "someone".into(),
            String::new(),
        )
        .unwrap();
    // (Only without an embedded GiroCrab registration: release builds carry one.)
    if !fints::product_id::builtin_available() {
        assert!(matches!(
            v.clone().begin_sync(other, "1".into(), false),
            Err(CoreError::MissingSetting { .. })
        ));
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn demo_push_tan_flow_imports_and_categorises() {
    let (v, path) = demo_vault();
    let conn = v
        .list_bank_connections()
        .unwrap()
        .into_iter()
        .find(|c| c.connector == ConnectorKind::VolksbankKasselGoettingen)
        .unwrap();
    let before = v
        .list_accounts()
        .unwrap()
        .into_iter()
        .find(|a| a.name == "Musterbank Girokonto" && a.connection_id == Some(conn.id))
        .unwrap();
    let s = v.clone().begin_sync(conn.id, "pin".into(), true).unwrap();
    match s.start().await.unwrap() {
        SyncStep::NeedsChallenge { challenge } => {
            assert_eq!(challenge.kind, ChallengeKind::DecoupledPush)
        }
        other => panic!("expected push challenge, got {other:?}"),
    }
    assert!(matches!(
        s.poll().await.unwrap(),
        SyncStep::StillWaiting { .. }
    ));
    match s.poll().await.unwrap() {
        SyncStep::Finished { summary } => {
            assert_eq!(summary.transactions_imported, 1);
            assert_eq!(
                summary.transactions_categorized, 1,
                "REWE booking is categorised by a rule"
            );
        }
        other => panic!("expected finish, got {other:?}"),
    }
    let after = v
        .list_accounts()
        .unwrap()
        .into_iter()
        .find(|a| a.id == before.id)
        .unwrap();
    assert_eq!(after.balance_cents, before.balance_cents - 27_65);
    assert!(matches!(s.poll().await, Err(CoreError::SessionClosed)));
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn demo_code_and_redirect_flows() {
    let (v, path) = demo_vault();
    let conns = v.list_bank_connections().unwrap();
    let tr = conns
        .iter()
        .find(|c| c.connector == ConnectorKind::TradeRepublic)
        .unwrap();
    let s = v.clone().begin_sync(tr.id, "1234".into(), true).unwrap();
    assert!(
        matches!(s.start().await.unwrap(), SyncStep::NeedsChallenge { challenge } if challenge.kind == ChallengeKind::Code)
    );
    assert!(
        s.submit_code("12".into()).await.is_err(),
        "bad code rejected"
    );
    let amex = conns
        .iter()
        .find(|c| c.connector == ConnectorKind::AmericanExpress)
        .unwrap();
    let s = v.clone().begin_sync(amex.id, String::new(), true).unwrap();
    assert!(
        matches!(s.start().await.unwrap(), SyncStep::NeedsChallenge { challenge } if challenge.kind == ChallengeKind::Redirect)
    );
    assert!(matches!(
        s.submit_code("https://localhost/cb?code=x".into())
            .await
            .unwrap(),
        SyncStep::Finished { .. }
    ));
    let pp_id = v
        .add_bank_connection(
            ConnectorKind::PayPal,
            String::new(),
            "demo@example.org".into(),
        )
        .unwrap();
    let pp = v
        .list_bank_connections()
        .unwrap()
        .into_iter()
        .find(|c| c.id == pp_id)
        .unwrap();
    let s = v.clone().begin_sync(pp.id, String::new(), true).unwrap();
    assert!(matches!(
        s.start().await.unwrap(),
        SyncStep::Finished { .. }
    ));
    // Real (non-demo) sync refuses to start without the user's own keys.
    assert!(matches!(
        v.clone().begin_sync(pp.id, String::new(), false),
        Err(CoreError::MissingSetting { .. })
    ));
    let _ = std::fs::remove_file(path);
}

#[tokio::test(start_paused = true)]
async fn demo_transfer_flow_adds_pending_booking_and_suggests_recipient() {
    let (v, path) = demo_vault();
    let acc = v
        .list_accounts()
        .unwrap()
        .into_iter()
        .find(|a| a.name == "Musterbank Girokonto" && a.connection_id.is_some())
        .unwrap();
    let req = |iban: &str, amount: i64| SepaTransferRequest {
        account_id: acc.id,
        recipient_name: "Müller GmbH".into(),
        recipient_iban: iban.into(),
        recipient_bic: None,
        amount_cents: amount,
        purpose: "Rechnung 42".into(),
        instant: true,
        execution_date: None,
    };
    assert!(v
        .clone()
        .begin_transfer(req("DE89370400440532013001", 100), "pin".into(), true)
        .is_err());
    assert!(v
        .clone()
        .begin_transfer(req("DE89370400440532013000", 0), "pin".into(), true)
        .is_err());
    let s = v
        .clone()
        .begin_transfer(req("DE89 3704 0044 0532 0130 00", 1250), "pin".into(), true)
        .unwrap();
    let mut step = s.start().await.unwrap();
    let mut challenges = 0;
    while matches!(
        step,
        TransferStep::NeedsChallenge { .. } | TransferStep::StillWaiting { .. }
    ) {
        challenges += 1;
        step = s.poll().await.unwrap();
    }
    assert!(challenges >= 2, "login + transfer challenge");
    let TransferStep::Finished { outcome } = &step else {
        panic!("{step:?}")
    };
    assert!(outcome.instant);
    assert!(outcome.end_to_end_id.starts_with("GC") && outcome.end_to_end_id.len() == 32);
    let page = v
        .list_transactions(BookingFilter {
            account_ids: vec![acc.id],
            category_id: None,
            include_subcategories: false,
            uncategorized_only: false,
            search: Some("Müller GmbH".into()),
            date_from: None,
            date_to: None,
            limit: 10,
            offset: 0,
            scope: BookingScope::All,
            merchant_key: None,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(page.transactions.len(), 1);
    assert_eq!(page.transactions[0].amount_cents, -1250);
    assert_eq!(
        page.transactions[0].counterparty_iban,
        "DE89370400440532013000"
    );
    let sug = v.suggest_recipients("müller".into(), 5).unwrap();
    assert_eq!(sug.len(), 1);
    assert_eq!(sug[0].iban, "DE89370400440532013000");
    assert!(matches!(s.poll().await, Err(CoreError::SessionClosed)));

    // Runs a transfer to the end, approving every challenge.
    async fn run(s: &TransferSession) -> (TransferStep, Vec<String>) {
        let mut step = s.start().await.unwrap();
        let mut titles = Vec::new();
        while let TransferStep::NeedsChallenge { challenge }
        | TransferStep::StillWaiting { challenge } = &step
        {
            titles.push(challenge.title.clone());
            step = s.poll().await.unwrap();
        }
        (step, titles)
    }
    let bookings = |name: &str| {
        v.list_transactions(BookingFilter {
            account_ids: vec![acc.id],
            category_id: None,
            include_subcategories: false,
            uncategorized_only: false,
            search: Some(name.into()),
            date_from: None,
            date_to: None,
            limit: 10,
            offset: 0,
            scope: BookingScope::All,
            merchant_key: None,
            ..Default::default()
        })
        .unwrap()
        .transactions
    };

    // Bank does not offer instant: declined, nothing sent, no booking,
    // and no automatic regular transfer.
    let mut slow = req("DE89370400440532013000", 990);
    slow.recipient_name = "Langsam AG".into();
    let s = v.clone().begin_transfer(slow, "pin".into(), true).unwrap();
    let (step, titles) = run(&s).await;
    assert!(
        matches!(step, TransferStep::InstantDeclined { .. }),
        "{step:?}"
    );
    assert!(
        !titles.iter().any(|t| t.contains("Überweisung")),
        "only the login, no transfer approval: {titles:?}"
    );
    assert!(bookings("Langsam AG").is_empty());
    assert!(matches!(s.poll().await, Err(CoreError::SessionClosed)));

    // Ambiguous reply after the approval: unknown, booking marked as
    // unclear, session over (nothing resent).
    let mut unclear = req("DE89370400440532013000", 770);
    unclear.recipient_name = "Unklar KG".into();
    let s = v
        .clone()
        .begin_transfer(unclear, "pin".into(), true)
        .unwrap();
    let (step, _) = run(&s).await;
    let TransferStep::OutcomeUnknown { end_to_end_id, .. } = &step else {
        panic!("{step:?}")
    };
    assert!(end_to_end_id.starts_with("GC"));
    let b = bookings("Unklar KG");
    assert_eq!(b.len(), 1);
    assert!(
        b[0].booking_text.contains("Status unklar"),
        "{}",
        b[0].booking_text
    );
    assert!(matches!(s.poll().await, Err(CoreError::SessionClosed)));

    // Connection lost after the approval: unknown as well, not an error.
    let mut lost = req("DE89370400440532013000", 660);
    lost.recipient_name = "Netzfehler GmbH".into();
    let s = v.clone().begin_transfer(lost, "pin".into(), true).unwrap();
    let (step, _) = run(&s).await;
    assert!(
        matches!(step, TransferStep::OutcomeUnknown { .. }),
        "{step:?}"
    );
    assert_eq!(bookings("Netzfehler GmbH").len(), 1);

    // Regular transfer: never a second order either.
    let mut regular = req("DE89370400440532013000", 550);
    regular.instant = false;
    regular.recipient_name = "Regulär OHG".into();
    let s = v
        .clone()
        .begin_transfer(regular, "pin".into(), true)
        .unwrap();
    let (step, _) = run(&s).await;
    let TransferStep::Finished { outcome } = &step else {
        panic!("{step:?}")
    };
    assert!(!outcome.instant);

    // Name check: close match needs confirmation, then the approval.
    let mut close = req("DE89370400440532013000", 500);
    close.recipient_name = "Fast Mustermann".into();
    let s = v.clone().begin_transfer(close, "pin".into(), true).unwrap();
    let mut step = s.start().await.unwrap();
    while matches!(
        step,
        TransferStep::NeedsChallenge { .. } | TransferStep::StillWaiting { .. }
    ) {
        step = s.poll().await.unwrap();
    }
    let TransferStep::NeedsPayeeConfirmation { check } = step else {
        panic!("{step:?}")
    };
    assert_eq!(check.result, PayeeMatch::CloseMatch);
    assert_eq!(check.bank_name.as_deref(), Some("Erika Mustermann"));
    let mut step = s.confirm_payee().await.unwrap();
    while matches!(
        step,
        TransferStep::NeedsChallenge { .. } | TransferStep::StillWaiting { .. }
    ) {
        step = s.poll().await.unwrap();
    }
    assert!(matches!(step, TransferStep::Finished { .. }));

    // Cancelling at the name check sends nothing.
    let mut wrong = req("DE89370400440532013000", 700);
    wrong.recipient_name = "Falsch GmbH".into();
    let s = v.clone().begin_transfer(wrong, "pin".into(), true).unwrap();
    let mut step = s.start().await.unwrap();
    while matches!(
        step,
        TransferStep::NeedsChallenge { .. } | TransferStep::StillWaiting { .. }
    ) {
        step = s.poll().await.unwrap();
    }
    assert!(matches!(
        step,
        TransferStep::NeedsPayeeConfirmation {
            check: PayeeCheck {
                result: PayeeMatch::NoMatch,
                ..
            }
        }
    ));
    s.cancel().await;
    assert!(matches!(
        s.confirm_payee().await,
        Err(CoreError::SessionClosed)
    ));
    let _ = std::fs::remove_file(path);
}

mod account_selection;
mod bank_name;
mod banking_gate;
mod batch_transfer;
mod body_limit;
mod content_id;
mod direct_debit;
mod payment_method;
mod postbox;
mod standing_order;
mod statements_auto;
mod transfer_cancel;

/// Local server answering every request with a redirect to `location(port)`;
/// returns (port, request counter).
fn redirect_server(
    location: fn(u16) -> String,
) -> (u16, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = hits.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { break };
            let n = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut buf = [0u8; 8192];
            let _ = s.read(&mut buf);
            let resp = if n == 0 {
                format!(
                    "HTTP/1.1 307 Moved\r\nLocation: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    location(port)
                )
            } else {
                "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
            };
            let _ = s.write_all(resp.as_bytes());
        }
    });
    (port, hits)
}

#[tokio::test]
async fn credential_clients_do_not_follow_redirects_to_another_host() {
    let (port, hits) = redirect_server(|p| format!("http://localhost:{p}/other"));
    let c = super::http_client_same_host().unwrap();
    let r = c
        .post(format!("http://127.0.0.1:{port}/login"))
        .body("user=a&pin=secret")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 307);
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn credential_clients_still_follow_redirects_inside_the_host() {
    let (port, hits) = redirect_server(|p| format!("http://127.0.0.1:{p}/next"));
    let c = super::http_client_same_host().unwrap();
    let r = c
        .post(format!("http://127.0.0.1:{port}/login"))
        .body("x")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 2);
}

fn fetch_with_document() -> FetchResult {
    FetchResult {
        accounts: vec![RemoteAccount {
            external_ref: "tr:cash".into(),
            name: "Verrechnungskonto".into(),
            owner: String::new(),
            iban: String::new(),
            account_number: String::new(),
            currency: "EUR".into(),
            kind: AccountKind::Checking,
            balance_cents: 100,
            balance_date: "2026-11-10".into(),
        }],
        documents: vec![crate::documents::RemoteDocument {
            account_ref: "tr:cash".into(),
            booking_external_id: None,
            title: "Kosteninformation".into(),
            doc_type: "postbox".into(),
            mime_type: "application/pdf".into(),
            file_name: "k.pdf".into(),
            bytes: b"%PDF-1.4 x".to_vec(),
            source_url: "tr:doc:1".into(),
            date: "2026-11-01".into(),
        }],
        ..Default::default()
    }
}

#[test]
fn document_retrieval_is_on_by_default_and_per_connection() {
    let dir = crate::test_support::TempDir::new("girocrab-docs-switch");
    let v = Vault::open(
        dir.join("v.db").to_string_lossy().to_string(),
        vec![3u8; 32],
    )
    .unwrap();
    let a = v
        .add_bank_connection(ConnectorKind::TradeRepublic, String::new(), "a".into())
        .unwrap();
    let b = v
        .add_bank_connection(ConnectorKind::TradeRepublic, String::new(), "b".into())
        .unwrap();
    assert!(v.documents_fetch_enabled(a).unwrap());
    // only banks that deliver documents offer the switch
    assert!(v.documents_fetch_supported(a).unwrap());
    let ibkr = v
        .add_bank_connection(ConnectorKind::Ibkr, String::new(), "c".into())
        .unwrap();
    assert!(!v.documents_fetch_supported(ibkr).unwrap());
    v.set_documents_fetch(a, false).unwrap();
    assert!(!v.documents_fetch_enabled(a).unwrap());
    assert!(
        v.documents_fetch_enabled(b).unwrap(),
        "another connection is not affected"
    );
    v.set_documents_fetch(a, true).unwrap();
    assert!(v.documents_fetch_enabled(a).unwrap());
}

#[test]
fn switching_retrieval_on_again_catches_up_until_a_sync_brings_nothing_new() {
    let dir = crate::test_support::TempDir::new("girocrab-docs-catchup");
    let v = Vault::open(
        dir.join("v.db").to_string_lossy().to_string(),
        vec![5u8; 32],
    )
    .unwrap();
    let conn = v
        .add_bank_connection(ConnectorKind::TradeRepublic, String::new(), "login".into())
        .unwrap();
    let catchup = format!("documents.fetch.catchup.{conn}");
    let off_since = format!("documents.fetch.off_since.{conn}");
    // never switched off: nothing to catch up
    v.set_documents_fetch(conn, true).unwrap();
    assert_eq!(v.get_setting(catchup.clone()).unwrap(), None);
    // off: the day is noted; on again: it becomes the catch-up day
    v.set_documents_fetch(conn, false).unwrap();
    let day = v.get_setting(off_since.clone()).unwrap();
    assert!(day.is_some());
    v.set_documents_fetch(conn, true).unwrap();
    assert_eq!(v.get_setting(catchup.clone()).unwrap(), day);
    assert_eq!(v.get_setting(off_since.clone()).unwrap(), None);
    // switched off again (and on) while it runs: the oldest day stays
    v.set_setting(catchup.clone(), "2026-01-01".into()).unwrap();
    v.set_documents_fetch(conn, false).unwrap();
    v.set_documents_fetch(conn, true).unwrap();
    assert_eq!(
        v.get_setting(catchup.clone()).unwrap().as_deref(),
        Some("2026-01-01")
    );
    // switched off twice in a row: the first day counts
    v.set_documents_fetch(conn, false).unwrap();
    v.set_setting(off_since.clone(), "2026-02-02".into())
        .unwrap();
    v.set_documents_fetch(conn, false).unwrap();
    assert_eq!(
        v.get_setting(off_since.clone()).unwrap().as_deref(),
        Some("2026-02-02")
    );
    v.set_documents_fetch(conn, true).unwrap();
    assert_eq!(
        v.get_setting(catchup.clone()).unwrap().as_deref(),
        Some("2026-01-01")
    );
    v.set_setting(catchup.clone(), day.clone().unwrap())
        .unwrap();
    // a sync that stores documents keeps the catch-up going …
    store_fetch(
        &v,
        ConnectorKind::TradeRepublic,
        conn,
        fetch_with_document(),
    )
    .unwrap();
    assert_eq!(v.get_setting(catchup.clone()).unwrap(), day);
    // a sync that left documents out (failed request, limit) does not end it either
    let mut left_out = fetch_with_document();
    left_out.documents.clear();
    left_out.documents_incomplete = true;
    store_fetch(&v, ConnectorKind::TradeRepublic, conn, left_out).unwrap();
    assert_eq!(v.get_setting(catchup.clone()).unwrap(), day);
    // … and the first one with nothing new ends it
    store_fetch(
        &v,
        ConnectorKind::TradeRepublic,
        conn,
        fetch_with_document(),
    )
    .unwrap();
    assert_eq!(v.get_setting(catchup.clone()).unwrap(), None);
    // while it is off, a sync does not end a running catch-up
    v.set_documents_fetch(conn, false).unwrap();
    v.set_documents_fetch(conn, true).unwrap();
    v.set_documents_fetch(conn, false).unwrap();
    store_fetch(
        &v,
        ConnectorKind::TradeRepublic,
        conn,
        fetch_with_document(),
    )
    .unwrap();
    assert!(v.get_setting(catchup).unwrap().is_some());
}

#[test]
fn a_deleted_connection_forgets_its_document_retrieval_settings() {
    let dir = crate::test_support::TempDir::new("girocrab-docs-forget");
    let v = Vault::open(
        dir.join("v.db").to_string_lossy().to_string(),
        vec![6u8; 32],
    )
    .unwrap();
    let conn = v
        .add_bank_connection(ConnectorKind::TradeRepublic, String::new(), "login".into())
        .unwrap();
    v.set_documents_fetch(conn, false).unwrap();
    v.set_documents_fetch(conn, true).unwrap();
    v.set_documents_fetch(conn, false).unwrap();
    let keys = [
        format!("documents.fetch.{conn}"),
        format!("documents.fetch.off_since.{conn}"),
        format!("documents.fetch.catchup.{conn}"),
    ];
    for key in &keys {
        assert!(v.get_setting(key.clone()).unwrap().is_some(), "{key}");
    }
    v.delete_bank_connection(conn).unwrap();
    for key in &keys {
        assert_eq!(v.get_setting(key.clone()).unwrap(), None, "{key}");
    }
}

#[test]
fn a_fetch_stores_no_documents_while_retrieval_is_off() {
    let dir = crate::test_support::TempDir::new("girocrab-docs-off");
    let v = Vault::open(
        dir.join("v.db").to_string_lossy().to_string(),
        vec![4u8; 32],
    )
    .unwrap();
    let conn = v
        .add_bank_connection(ConnectorKind::TradeRepublic, String::new(), "login".into())
        .unwrap();
    v.set_documents_fetch(conn, false).unwrap();
    let summary = store_fetch(
        &v,
        ConnectorKind::TradeRepublic,
        conn,
        fetch_with_document(),
    )
    .unwrap();
    // the log can say why no document came
    assert!(summary.documents_off);
    assert!(v
        .list_all_documents(crate::documents::DocumentFilter::default())
        .unwrap()
        .is_empty());
    // on again: the same delivery is stored
    v.set_documents_fetch(conn, true).unwrap();
    let on = store_fetch(
        &v,
        ConnectorKind::TradeRepublic,
        conn,
        fetch_with_document(),
    )
    .unwrap();
    assert!(!on.documents_off);
    assert_eq!(
        v.list_all_documents(crate::documents::DocumentFilter::default())
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn portal_connections_list_nothing_to_download_while_retrieval_is_off() {
    let dir = crate::test_support::TempDir::new("girocrab-docs-portal");
    let v = Vault::open(
        dir.join("v.db").to_string_lossy().to_string(),
        vec![5u8; 32],
    )
    .unwrap();
    let conn = v
        .add_bank_connection(ConnectorKind::Fnz, String::new(), "login".into())
        .unwrap();
    v.set_documents_fetch(conn, false).unwrap();
    // not even a list that would be invalid is looked at
    assert!(v
        .clone()
        .pending_fnz_documents(conn, "kein json".into())
        .unwrap()
        .is_empty());
}

// ---- documents linked to bookings by their content (`document_matching`), either order ----

fn tr_cash_account() -> RemoteAccount {
    RemoteAccount {
        external_ref: "tr:cash".into(),
        name: "Verrechnungskonto".into(),
        owner: String::new(),
        iban: String::new(),
        account_number: String::new(),
        currency: "EUR".into(),
        kind: AccountKind::Checking,
        balance_cents: 0,
        balance_date: "2025-12-10".into(),
    }
}

fn tr_booking(id: &str, date: &str, cents: i64, counterparty: &str) -> RemoteTransaction {
    RemoteTransaction {
        account_ref: "tr:cash".into(),
        external_id: id.into(),
        booking_date: date.into(),
        value_date: date.into(),
        amount_cents: cents,
        currency: "EUR".into(),
        counterparty: counterparty.into(),
        ..Default::default()
    }
}

/// A PDF whose text carries what the matching reads: event word, amount and a date.
fn tr_pdf(event: &str, subject: &str, amount: &str, date: &str) -> Vec<u8> {
    crate::documents::simple_pdf(&[
        format!("WERTPAPIERABRECHNUNG {event} {subject}"),
        format!("{amount} EUR"),
        date.to_string(),
    ])
}

#[test]
fn an_unstructured_trade_document_with_several_amounts_stays_unlinked() {
    let subject = "JPM USD EM Bond USD (Dist)";
    let (_dir, v) = open_vault("girocrab-tr-ambiguous-amounts");
    let connection = v
        .add_bank_connection(ConnectorKind::TradeRepublic, String::new(), "login".into())
        .unwrap();
    let pdf = crate::documents::simple_pdf(&[
        format!("WERTPAPIERABRECHNUNG KAUF {subject}"),
        "Kurs 100,00 EUR".into(),
        "Gesamt 500,00 EUR".into(),
        "04.12.2025".into(),
    ]);
    store_fetch(
        &v,
        ConnectorKind::TradeRepublic,
        connection,
        FetchResult {
            accounts: vec![tr_cash_account()],
            transactions: vec![tr_booking("different-buy", "2025-12-04", -100_00, subject)],
            documents: vec![tr_document("ambiguous", subject, pdf, "2025-12-04")],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        v.list_all_documents(Default::default()).unwrap()[0].booking_id,
        None
    );
}

fn tr_document(
    source: &str,
    subject: &str,
    bytes: Vec<u8>,
    date: &str,
) -> crate::documents::RemoteDocument {
    crate::documents::RemoteDocument {
        account_ref: "tr:cash".into(),
        booking_external_id: None,
        title: format!("Abrechnung \u{2013} {subject}"),
        doc_type: "abrechnung".into(),
        mime_type: "application/pdf".into(),
        file_name: format!("{source}.pdf"),
        bytes,
        source_url: format!("https://example.invalid/{source}"),
        date: date.into(),
    }
}

fn open_vault(tag: &str) -> (crate::test_support::TempDir, std::sync::Arc<Vault>) {
    let dir = crate::test_support::TempDir::new(tag);
    let v = Vault::open(
        dir.join("v.db").to_string_lossy().to_string(),
        vec![8u8; 32],
    )
    .unwrap();
    (dir, v)
}

fn booking_id_of(v: &Vault, external_id: &str) -> i64 {
    v.db()
        .unwrap()
        .query_row(
            "SELECT id FROM transactions WHERE external_id = ?1",
            [external_id],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn a_late_pdf_is_linked_to_an_older_stored_booking_and_stays_put() {
    let subject = "JPM USD EM Bond USD (Dist)";
    let (_dir, v) = open_vault("girocrab-late-pdf");
    let conn = v
        .add_bank_connection(ConnectorKind::TradeRepublic, String::new(), "login".into())
        .unwrap();
    // the booking is stored first, in a fetch without documents
    let first = FetchResult {
        accounts: vec![tr_cash_account()],
        transactions: vec![tr_booking("b1", "2025-12-04", -25_00, subject)],
        ..Default::default()
    };
    store_fetch(&v, ConnectorKind::TradeRepublic, conn, first).unwrap();
    // the PDF comes later, alone, under another id than the booking
    let pdf = tr_pdf("KAUF", subject, "25,00", "02.12.2025");
    let later = |bytes: &Vec<u8>| FetchResult {
        accounts: vec![tr_cash_account()],
        documents: vec![tr_document("d1", subject, bytes.clone(), "2025-12-04")],
        ..Default::default()
    };
    store_fetch(&v, ConnectorKind::TradeRepublic, conn, later(&pdf)).unwrap();
    let docs = v.list_all_documents(Default::default()).unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].booking_id, Some(booking_id_of(&v, "b1")));
    let account = docs[0].account_id;
    assert!(account.is_some());
    // the same delivery again: no second document, the link and the account do not change
    store_fetch(&v, ConnectorKind::TradeRepublic, conn, later(&pdf)).unwrap();
    let again = v.list_all_documents(Default::default()).unwrap();
    assert_eq!(again.len(), 1);
    assert_eq!(
        (again[0].booking_id, again[0].account_id),
        (docs[0].booking_id, account)
    );
}

#[test]
fn a_purchase_pdf_is_not_linked_to_the_credit_of_an_equal_sale() {
    let subject = "JPM USD EM Bond USD (Dist)";
    let (_dir, v) = open_vault("girocrab-pdf-direction");
    let conn = v
        .add_bank_connection(ConnectorKind::TradeRepublic, String::new(), "login".into())
        .unwrap();
    let data = FetchResult {
        accounts: vec![tr_cash_account()],
        transactions: vec![
            // the sale's credit is on the very day of the PDF, the purchase's debit two days later
            tr_booking("sale", "2025-12-02", 25_00, subject),
            tr_booking("buy", "2025-12-04", -25_00, subject),
        ],
        documents: vec![tr_document(
            "d2",
            subject,
            tr_pdf("KAUF", subject, "25,00", "02.12.2025"),
            "2025-12-02",
        )],
        ..Default::default()
    };
    store_fetch(&v, ConnectorKind::TradeRepublic, conn, data).unwrap();
    let docs = v.list_all_documents(Default::default()).unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].booking_id, Some(booking_id_of(&v, "buy")));
}

#[test]
fn a_manual_link_and_another_login_are_left_alone() {
    let subject = "JPM USD EM Bond USD (Dist)";
    let (_dir, v) = open_vault("girocrab-pdf-isolation");
    let a = v
        .add_bank_connection(ConnectorKind::TradeRepublic, String::new(), "a".into())
        .unwrap();
    let b = v
        .add_bank_connection(ConnectorKind::TradeRepublic, String::new(), "b".into())
        .unwrap();
    // login b owns the only matching booking
    let other = FetchResult {
        accounts: vec![tr_cash_account()],
        transactions: vec![tr_booking("only-b", "2025-12-04", -25_00, subject)],
        ..Default::default()
    };
    store_fetch(&v, ConnectorKind::TradeRepublic, b, other).unwrap();
    // login a gets the PDF: nothing in its own accounts matches, so it stays unlinked
    let pdf = tr_pdf("KAUF", subject, "25,00", "02.12.2025");
    let mine = FetchResult {
        accounts: vec![tr_cash_account()],
        documents: vec![tr_document("d3", subject, pdf, "2025-12-04")],
        ..Default::default()
    };
    store_fetch(&v, ConnectorKind::TradeRepublic, a, mine).unwrap();
    let docs = v.list_all_documents(Default::default()).unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(
        docs[0].booking_id, None,
        "a booking of another login is no candidate"
    );
    // the user links it by hand to a booking of login a; a later fetch keeps that choice
    let own = FetchResult {
        accounts: vec![tr_cash_account()],
        transactions: vec![
            tr_booking("mine-1", "2025-12-04", -25_00, subject),
            tr_booking("mine-2", "2025-12-20", -99_00, "Etwas anderes"),
        ],
        ..Default::default()
    };
    store_fetch(&v, ConnectorKind::TradeRepublic, a, own).unwrap();
    let manual = booking_id_of(&v, "mine-2");
    v.link_document(docs[0].id, Some(manual)).unwrap();
    store_fetch(
        &v,
        ConnectorKind::TradeRepublic,
        a,
        FetchResult {
            accounts: vec![tr_cash_account()],
            transactions: vec![tr_booking("mine-1", "2025-12-04", -25_00, subject)],
            ..Default::default()
        },
    )
    .unwrap();
    let kept = v.list_all_documents(Default::default()).unwrap();
    assert_eq!(
        kept[0].booking_id,
        Some(manual),
        "a manual link is never replaced"
    );
}

// ---- comdirect: trade documents belong to the depot and document a booking on the Girokonto ----

const CD_GIRO: &str = "DE02200411550123456700";
const CD_GIRO2: &str = "DE75200411550765432100";
const CD_DEPOT: &str = "123456700";
const CD_ISIN: &str = "IE00B3RBWM25";

fn cd_account(external_ref: &str, kind: AccountKind) -> RemoteAccount {
    RemoteAccount {
        external_ref: external_ref.into(),
        name: external_ref.into(),
        owner: String::new(),
        iban: if kind == AccountKind::Checking {
            external_ref.into()
        } else {
            String::new()
        },
        // the depot is named by its number in the documents
        account_number: if kind == AccountKind::Brokerage {
            external_ref.into()
        } else {
            String::new()
        },
        currency: "EUR".into(),
        kind,
        balance_cents: 0,
        balance_date: "2025-12-10".into(),
    }
}

fn cd_accounts(two_girokonten: bool) -> Vec<RemoteAccount> {
    let mut all = vec![
        cd_account(CD_GIRO, AccountKind::Checking),
        cd_account(CD_DEPOT, AccountKind::Brokerage),
    ];
    if two_girokonten {
        all.push(cd_account(CD_GIRO2, AccountKind::Checking));
    }
    all
}

fn cd_booking(
    account: &str,
    id: &str,
    date: &str,
    cents: i64,
    currency: &str,
    purpose: &str,
) -> RemoteTransaction {
    RemoteTransaction {
        account_ref: account.into(),
        external_id: id.into(),
        booking_date: date.into(),
        value_date: date.into(),
        amount_cents: cents,
        currency: currency.into(),
        counterparty: "comdirect".into(),
        // what a trade booking names: the security by its ISIN
        purpose: format!("{purpose} {CD_ISIN}"),
        ..Default::default()
    }
}

/// A trade confirmation as text: event word, ISIN, amount with currency and a date.
fn cd_pdf(event: &str, amount: &str, date: &str, currency: &str) -> Vec<u8> {
    crate::documents::simple_pdf(&[
        format!("GESCHAEFTSABRECHNUNG {event}"),
        format!("Wertpapier ISIN {CD_ISIN}"),
        format!("Ausmachender Betrag {amount} {currency}"),
        format!("Geschaeftstag {date}"),
    ])
}

/// Like `cd_pdf`, with the settlement account the confirmation names.
fn cd_pdf_for(iban: &str, event: &str, amount: &str, date: &str) -> Vec<u8> {
    crate::documents::simple_pdf(&[
        format!("GESCHAEFTSABRECHNUNG {event}"),
        format!("Verrechnungskonto IBAN {iban}"),
        format!("Wertpapier ISIN {CD_ISIN}"),
        format!("Ausmachender Betrag {amount} EUR"),
        format!("Geschaeftstag {date}"),
    ])
}

fn cd_document(
    source: &str,
    account_ref: &str,
    title: &str,
    doc_type: &str,
    bytes: Vec<u8>,
    date: &str,
) -> crate::documents::RemoteDocument {
    crate::documents::RemoteDocument {
        account_ref: account_ref.into(),
        booking_external_id: None,
        title: title.into(),
        doc_type: doc_type.into(),
        mime_type: "application/pdf".into(),
        file_name: format!("{source}.pdf"),
        bytes,
        source_url: format!("comdirect:postbox:{source}"),
        date: date.into(),
    }
}

fn cd_purchase_title() -> &'static str {
    "Wertpapierabrechnung Kauf 12,5 St. WKN A1JX52 (VANGUARD TEST FONDS) vom 04.12.2025"
}

#[test]
fn deleting_a_comdirect_login_does_not_transfer_its_orphan_documents() {
    let (_dir, v) = open_vault("girocrab-cd-deleted-origin");
    let (kind, remaining) = cd_connection(&v, false, "remaining");
    store_fetch(&v, kind, remaining, cd_fetch(false, vec![], vec![])).unwrap();
    let (_, deleted) = cd_connection(&v, false, "deleted");
    store_fetch(
        &v,
        kind,
        deleted,
        cd_fetch(
            false,
            vec![],
            vec![cd_document(
                "deleted-orphan",
                "",
                "Information",
                "postbox",
                crate::documents::simple_pdf(&["Neue Bedingungen".into()]),
                "2025-12-05",
            )],
        ),
    )
    .unwrap();
    v.delete_bank_connection(deleted).unwrap();
    assert_eq!(v.resort_portal_documents(remaining).unwrap().resorted, 0);
    let account: Option<i64> = v.db().unwrap().query_row(
        "SELECT account_id FROM booking_documents WHERE source_url = 'comdirect:postbox:deleted-orphan'",
        [],
        |r| r.get(0),
    ).unwrap();
    assert_eq!(account, None);
}

fn account_id_of(v: &Vault, external_ref: &str) -> i64 {
    v.db()
        .unwrap()
        .query_row(
            "SELECT id FROM accounts WHERE external_ref = ?1",
            [external_ref],
            |r| r.get(0),
        )
        .unwrap()
}

/// A comdirect connection: the REST kind, or a FinTS login (kind `FinTsGeneric`, key with a comdirect BLZ).
fn cd_connection(v: &Vault, fints: bool, login: &str) -> (ConnectorKind, i64) {
    if fints {
        let (id, _) = Vault::add_bank_connection_keyed(
            &v.db().unwrap(),
            ConnectorKind::FinTsGeneric,
            "fints:20041155",
            Some("20041155"),
            String::new(),
            login.into(),
        )
        .unwrap();
        (ConnectorKind::FinTsGeneric, id)
    } else {
        let id = v
            .add_bank_connection(ConnectorKind::Comdirect, String::new(), login.into())
            .unwrap();
        (ConnectorKind::Comdirect, id)
    }
}

fn cd_fetch(
    two_girokonten: bool,
    transactions: Vec<RemoteTransaction>,
    documents: Vec<crate::documents::RemoteDocument>,
) -> FetchResult {
    FetchResult {
        accounts: cd_accounts(two_girokonten),
        transactions,
        documents,
        ..Default::default()
    }
}

#[test]
fn a_comdirect_trade_document_stays_with_the_depot_and_documents_the_girokonto_booking_in_either_order(
) {
    for fints in [false, true] {
        // 0: both in one fetch, 1: the booking first, 2: the PDF first
        for order in 0..3 {
            let (_dir, v) = open_vault("girocrab-cd-order");
            let (kind, conn) = cd_connection(&v, fints, "login");
            let booking = cd_booking(
                CD_GIRO,
                "b1",
                "2025-12-04",
                -801_50,
                "EUR",
                "Wertpapierkauf",
            );
            let doc = cd_document(
                "d1",
                CD_DEPOT,
                cd_purchase_title(),
                "abrechnung",
                cd_pdf("Wertpapierkauf", "801,50", "04.12.2025", "EUR"),
                "2025-12-05",
            );
            match order {
                0 => {
                    store_fetch(&v, kind, conn, cd_fetch(false, vec![booking], vec![doc])).unwrap();
                }
                1 => {
                    store_fetch(&v, kind, conn, cd_fetch(false, vec![booking], vec![])).unwrap();
                    store_fetch(&v, kind, conn, cd_fetch(false, vec![], vec![doc])).unwrap();
                }
                _ => {
                    store_fetch(&v, kind, conn, cd_fetch(false, vec![], vec![doc])).unwrap();
                    store_fetch(&v, kind, conn, cd_fetch(false, vec![booking], vec![])).unwrap();
                }
            }
            let docs = v.list_all_documents(Default::default()).unwrap();
            assert_eq!(docs.len(), 1, "fints={fints} order={order}");
            // the document keeps the depot, the booking is the one on the Girokonto
            assert_eq!(docs[0].account_id, Some(account_id_of(&v, CD_DEPOT)));
            assert_eq!(
                docs[0].booking_id,
                Some(booking_id_of(&v, "b1")),
                "fints={fints} order={order}"
            );
            // fetching it again changes nothing and creates no second document
            let again_doc = cd_document(
                "d1",
                CD_DEPOT,
                cd_purchase_title(),
                "abrechnung",
                cd_pdf("Wertpapierkauf", "801,50", "04.12.2025", "EUR"),
                "2025-12-05",
            );
            store_fetch(&v, kind, conn, cd_fetch(false, vec![], vec![again_doc])).unwrap();
            let after = v.list_all_documents(Default::default()).unwrap();
            assert_eq!(after.len(), 1);
            assert_eq!(
                (after[0].booking_id, after[0].account_id),
                (docs[0].booking_id, docs[0].account_id)
            );
        }
    }
}

#[test]
fn comdirect_leaves_ambiguous_wrong_way_and_wrong_currency_candidates_alone() {
    let (_dir, v) = open_vault("girocrab-cd-ambiguous");
    let (kind, conn) = cd_connection(&v, false, "login");
    let pdf = cd_pdf("Wertpapierkauf", "100,00", "04.12.2025", "EUR");
    let doc = |source: &str| {
        cd_document(
            source,
            CD_DEPOT,
            cd_purchase_title(),
            "abrechnung",
            pdf.clone(),
            "2025-12-05",
        )
    };
    // the same debit on two Girokonten: nothing tells them apart
    let twin = |account: &str, id: &str| {
        cd_booking(account, id, "2025-12-04", -100_00, "EUR", "Wertpapierkauf")
    };
    store_fetch(
        &v,
        kind,
        conn,
        cd_fetch(
            true,
            vec![twin(CD_GIRO, "t1"), twin(CD_GIRO2, "t2")],
            vec![doc("d1")],
        ),
    )
    .unwrap();
    let docs = v.list_all_documents(Default::default()).unwrap();
    assert_eq!(docs[0].booking_id, None, "ambiguous: left unlinked");
    assert_eq!(docs[0].account_id, Some(account_id_of(&v, CD_DEPOT)));

    // a credit of the same amount is no candidate for a purchase, a USD booking none for a EUR document
    let (_dir2, v2) = open_vault("girocrab-cd-direction");
    let (kind2, conn2) = cd_connection(&v2, false, "login");
    store_fetch(
        &v2,
        kind2,
        conn2,
        cd_fetch(
            false,
            vec![
                cd_booking(
                    CD_GIRO,
                    "credit",
                    "2025-12-04",
                    100_00,
                    "EUR",
                    "Wertpapierverkauf",
                ),
                cd_booking(
                    CD_GIRO,
                    "usd",
                    "2025-12-04",
                    -100_00,
                    "USD",
                    "Wertpapierkauf",
                ),
            ],
            vec![doc("d2")],
        ),
    )
    .unwrap();
    assert_eq!(
        v2.list_all_documents(Default::default()).unwrap()[0].booking_id,
        None
    );
    // a sale confirmation does take the credit
    store_fetch(
        &v2,
        kind2,
        conn2,
        cd_fetch(
            false,
            vec![],
            vec![cd_document(
                "d3",
                CD_DEPOT,
                "Wertpapierabrechnung Verkauf 12,5 St. WKN A1JX52 (VANGUARD TEST FONDS) vom 04.12.2025",
                "abrechnung",
                cd_pdf("Wertpapierverkauf", "100,00", "04.12.2025", "EUR"),
                "2025-12-05",
            )],
        ),
    )
    .unwrap();
    let sale = v2
        .list_all_documents(Default::default())
        .unwrap()
        .into_iter()
        .find(|d| d.title.contains("Verkauf"))
        .unwrap();
    assert_eq!(sale.booking_id, Some(booking_id_of(&v2, "credit")));
}

#[test]
fn comdirect_period_documents_and_other_logins_get_no_booking_and_a_manual_link_stays() {
    let (_dir, v) = open_vault("girocrab-cd-period");
    let (kind, a) = cd_connection(&v, false, "a");
    let (_, b) = cd_connection(&v, false, "b");
    let amount = "100,00";
    // login b owns a matching booking (on an account of its own)
    store_fetch(
        &v,
        kind,
        b,
        FetchResult {
            accounts: vec![cd_account(CD_GIRO2, AccountKind::Checking)],
            transactions: vec![cd_booking(
                CD_GIRO2,
                "only-b",
                "2025-12-04",
                -100_00,
                "EUR",
                "Wertpapierkauf",
            )],
            ..Default::default()
        },
    )
    .unwrap();
    let pdf = cd_pdf("Wertpapierkauf", amount, "04.12.2025", "EUR");
    let finanzreport = cd_document(
        "report",
        CD_DEPOT,
        "Finanzreport Nr. 12 per 31.12.2025",
        "kontoauszug",
        pdf.clone(),
        "2025-12-05",
    );
    let tax = cd_document(
        "tax",
        CD_DEPOT,
        "Steuermitteilung WKN A1JX52 vom 05.12.2025",
        "steuer",
        pdf.clone(),
        "2025-12-05",
    );
    let trade = cd_document(
        "trade",
        CD_DEPOT,
        cd_purchase_title(),
        "abrechnung",
        pdf,
        "2025-12-05",
    );
    store_fetch(
        &v,
        kind,
        a,
        cd_fetch(
            false,
            vec![cd_booking(
                CD_GIRO,
                "mine-1",
                "2025-12-04",
                -100_00,
                "EUR",
                "Wertpapierkauf",
            )],
            vec![finanzreport, tax, trade],
        ),
    )
    .unwrap();
    let docs = v.list_all_documents(Default::default()).unwrap();
    assert_eq!(docs.len(), 3);
    for d in &docs {
        if d.title.starts_with("Finanzreport") || d.title.starts_with("Steuermitteilung") {
            assert_eq!(d.booking_id, None, "{}: a period document", d.title);
        } else {
            // its own login's booking, not the one of login b
            assert_eq!(d.booking_id, Some(booking_id_of(&v, "mine-1")));
        }
    }
    // the user moves the trade document to another booking; sorting and fetching again keep that
    store_fetch(
        &v,
        kind,
        a,
        cd_fetch(
            false,
            vec![cd_booking(
                CD_GIRO,
                "other",
                "2025-12-20",
                -5_00,
                "EUR",
                "Etwas anderes",
            )],
            vec![],
        ),
    )
    .unwrap();
    let trade_id = docs.iter().find(|d| d.title.contains("Kauf")).unwrap().id;
    let manual = booking_id_of(&v, "other");
    v.link_document(trade_id, Some(manual)).unwrap();
    assert_eq!(
        v.resort_portal_documents(a).unwrap().resorted,
        0,
        "nothing left to sort"
    );
    assert_eq!(v.resort_portal_documents(a).unwrap().resorted, 0);
    let kept = v
        .list_all_documents(Default::default())
        .unwrap()
        .into_iter()
        .find(|d| d.id == trade_id)
        .unwrap();
    assert_eq!(kept.booking_id, Some(manual));
}

#[test]
fn a_comdirect_document_goes_to_the_girokonto_it_names_or_stays_open() {
    let (_dir, v) = open_vault("girocrab-cd-accounts");
    let (kind, conn) = cd_connection(&v, false, "login");
    let twin = |account: &str, id: &str, date: &str| {
        cd_booking(account, id, date, -100_00, "EUR", "Wertpapierkauf")
    };
    // the same purchase on both Girokonten; the wrong one is nearer to the date in the PDF
    store_fetch(
        &v,
        kind,
        conn,
        cd_fetch(
            true,
            vec![
                twin(CD_GIRO, "wrong", "2025-12-04"),
                twin(CD_GIRO2, "right", "2025-12-08"),
            ],
            vec![cd_document(
                "named",
                CD_DEPOT,
                cd_purchase_title(),
                "abrechnung",
                cd_pdf_for(CD_GIRO2, "Wertpapierkauf", "100,00", "04.12.2025"),
                "2025-12-05",
            )],
        ),
    )
    .unwrap();
    let docs = v.list_all_documents(Default::default()).unwrap();
    assert_eq!(docs[0].booking_id, Some(booking_id_of(&v, "right")));
    assert_eq!(docs[0].account_id, Some(account_id_of(&v, CD_DEPOT)));

    // a second confirmation names no account: with two Girokonten that is no evidence, it stays open
    store_fetch(
        &v,
        kind,
        conn,
        cd_fetch(
            true,
            vec![twin(CD_GIRO, "only-here", "2025-12-04")],
            vec![cd_document(
                "unnamed",
                CD_DEPOT,
                "Wertpapierabrechnung Kauf 3 St. WKN A1JX52 (VANGUARD TEST FONDS) vom 04.12.2025",
                "abrechnung",
                cd_pdf("Wertpapierkauf", "100,00", "04.12.2025", "EUR"),
                "2025-12-05",
            )],
        ),
    )
    .unwrap();
    let open = v
        .list_all_documents(Default::default())
        .unwrap()
        .into_iter()
        .find(|d| d.title.contains("3 St."))
        .unwrap();
    assert_eq!(open.booking_id, None);
}

#[test]
fn comdirect_information_and_cost_documents_never_take_a_booking() {
    let (_dir, v) = open_vault("girocrab-cd-info");
    let (kind, conn) = cd_connection(&v, false, "login");
    // a cost information names the ISIN and an amount, a date: the same traits as a trade confirmation
    let cost = cd_document(
        "cost",
        CD_DEPOT,
        "Kosteninformation VANGUARD TEST FONDS WKN A1JX52 vom 04.12.2025",
        "postbox",
        cd_pdf("Wertpapierkauf", "100,00", "04.12.2025", "EUR"),
        "2025-12-05",
    );
    // an unclassified document with a trade's words, and a dividend notice without a direction word
    let notice = cd_document(
        "notice",
        CD_DEPOT,
        "Mitteilung WKN A1JX52 (VANGUARD TEST FONDS)",
        "postbox",
        cd_pdf("Wertpapierkauf", "100,00", "04.12.2025", "EUR"),
        "2025-12-05",
    );
    store_fetch(
        &v,
        kind,
        conn,
        cd_fetch(
            false,
            vec![cd_booking(
                CD_GIRO,
                "b",
                "2025-12-04",
                -100_00,
                "EUR",
                "Wertpapierkauf",
            )],
            vec![cost, notice],
        ),
    )
    .unwrap();
    for d in v.list_all_documents(Default::default()).unwrap() {
        assert_eq!(d.booking_id, None, "{}", d.title);
        assert_eq!(d.account_id, Some(account_id_of(&v, CD_DEPOT)));
    }
}

#[test]
fn comdirect_links_only_the_settled_amount_not_a_price_or_a_fee() {
    let (_dir, v) = open_vault("girocrab-cd-settled");
    let (kind, conn) = cd_connection(&v, false, "login");
    // the confirmation settles 500,00; a 100,00 booking of the same security exists, the 500,00 one does not
    let pdf = crate::documents::simple_pdf(&[
        "GESCHAEFTSABRECHNUNG Wertpapierkauf".to_string(),
        format!("Wertpapier ISIN {CD_ISIN}"),
        "Kurs 100,00 EUR".to_string(),
        "Provision 1,50 EUR".to_string(),
        "Ausmachender Betrag 500,00 EUR".to_string(),
        "Geschaeftstag 04.12.2025".to_string(),
    ]);
    let doc = cd_document(
        "settled",
        CD_DEPOT,
        cd_purchase_title(),
        "abrechnung",
        pdf.clone(),
        "2025-12-05",
    );
    store_fetch(
        &v,
        kind,
        conn,
        cd_fetch(
            false,
            vec![cd_booking(
                CD_GIRO,
                "price",
                "2025-12-04",
                -100_00,
                "EUR",
                "Wertpapierkauf",
            )],
            vec![doc],
        ),
    )
    .unwrap();
    assert_eq!(
        v.list_all_documents(Default::default()).unwrap()[0].booking_id,
        None
    );
    // the booking of the settled amount arrives later: now it is linked
    store_fetch(
        &v,
        kind,
        conn,
        cd_fetch(
            false,
            vec![cd_booking(
                CD_GIRO,
                "settled",
                "2025-12-04",
                -500_00,
                "EUR",
                "Wertpapierkauf",
            )],
            vec![],
        ),
    )
    .unwrap();
    assert_eq!(
        v.list_all_documents(Default::default()).unwrap()[0].booking_id,
        Some(booking_id_of(&v, "settled"))
    );
}

#[test]
fn sorting_comdirect_documents_anew_repairs_the_account_an_earlier_version_chose() {
    let (_dir, v) = open_vault("girocrab-cd-repair");
    let (kind, conn) = cd_connection(&v, false, "login");
    let trade = |source: &str, account: &str| {
        cd_document(
            source,
            account,
            cd_purchase_title(),
            "abrechnung",
            cd_pdf_for(CD_DEPOT, "Wertpapierkauf", "100,00", "04.12.2025"),
            "2025-12-05",
        )
    };
    // as the old REST path stored them: a depot document on the first Girokonto, another without account;
    // one the user moved to the second Girokonto on purpose
    store_fetch(
        &v,
        kind,
        conn,
        cd_fetch(
            true,
            vec![],
            vec![
                trade("on-giro", CD_GIRO),
                trade("nowhere", ""),
                trade("by-hand", CD_GIRO2),
            ],
        ),
    )
    .unwrap();
    let depot = account_id_of(&v, CD_DEPOT);
    let account_of = |title_source: &str| -> Option<i64> {
        v.db()
            .unwrap()
            .query_row(
                "SELECT account_id FROM booking_documents WHERE source_url = ?1",
                [format!("comdirect:postbox:{title_source}")],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_eq!(account_of("on-giro"), Some(account_id_of(&v, CD_GIRO)));
    assert_eq!(account_of("nowhere"), None);
    assert_eq!(v.resort_portal_documents(conn).unwrap().resorted, 2);
    assert_eq!(account_of("on-giro"), Some(depot));
    assert_eq!(account_of("nowhere"), Some(depot));
    assert_eq!(
        account_of("by-hand"),
        Some(account_id_of(&v, CD_GIRO2)),
        "a document the user put elsewhere stays"
    );
    // running it again changes nothing
    assert_eq!(v.resort_portal_documents(conn).unwrap().resorted, 0);

    // with a second comdirect login the origin of a document without account is not certain
    let (_dir2, v2) = open_vault("girocrab-cd-repair-two");
    let (kind2, a) = cd_connection(&v2, false, "a");
    let (_, _b) = cd_connection(&v2, false, "b");
    store_fetch(
        &v2,
        kind2,
        a,
        cd_fetch(true, vec![], vec![trade("nowhere", "")]),
    )
    .unwrap();
    assert_eq!(v2.resort_portal_documents(a).unwrap().resorted, 0);
    let orphan: Option<i64> = v2
        .db()
        .unwrap()
        .query_row("SELECT account_id FROM booking_documents", [], |r| r.get(0))
        .unwrap();
    assert_eq!(orphan, None);
}

// ---- Trade Republic: one account rule for every way a document arrives ----

fn tr_depot_account() -> RemoteAccount {
    RemoteAccount {
        external_ref: "tr:depot".into(),
        name: "Depot".into(),
        owner: String::new(),
        iban: String::new(),
        account_number: String::new(),
        currency: "EUR".into(),
        kind: AccountKind::Brokerage,
        balance_cents: 0,
        balance_date: "2025-12-10".into(),
    }
}

/// A dividend confirmation as Trade Republic hands it out, with the account and booking the way
/// of arrival gives it.
fn tr_dividend(
    source: &str,
    account_ref: &str,
    booking_external_id: Option<&str>,
) -> crate::documents::RemoteDocument {
    crate::documents::RemoteDocument {
        account_ref: account_ref.into(),
        booking_external_id: booking_external_id.map(str::to_string),
        title: "Dividendenbeleg \u{2013} Realty Income".into(),
        doc_type: "abrechnung".into(),
        mime_type: "application/pdf".into(),
        file_name: format!("{source}.pdf"),
        bytes: tr_pdf("DIVIDENDE", "Realty Income", "17,28", "16.11.2025"),
        source_url: format!("https://example.invalid/{source}"),
        date: "2025-11-16".into(),
    }
}

fn tr_fetch(
    transactions: Vec<RemoteTransaction>,
    documents: Vec<crate::documents::RemoteDocument>,
) -> FetchResult {
    FetchResult {
        accounts: vec![tr_cash_account(), tr_depot_account()],
        transactions,
        documents,
        ..Default::default()
    }
}

/// (account `external_ref`, `external_id` of the booking) of a stored document.
fn doc_places(v: &Vault, source: &str) -> (String, Option<String>) {
    v.db()
        .unwrap()
        .query_row(
            "SELECT COALESCE(a.external_ref, ''), t.external_id FROM booking_documents d
               LEFT JOIN accounts a ON a.id = d.account_id
               LEFT JOIN transactions t ON t.id = d.booking_id
              WHERE d.source_url = ?1",
            [format!("https://example.invalid/{source}")],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
}

#[test]
fn a_dividend_confirmation_sits_at_the_depot_and_documents_the_cash_booking_whichever_way_it_came()
{
    let credit = || tr_booking("div-1", "2025-11-16", 17_28, "Realty Income");
    let expected = ("tr:depot".to_string(), Some("div-1".to_string()));
    // 1: with the booking's own detail screen (the confirmation names the booking), bookings first
    let (_d1, v1) = open_vault("girocrab-tr-div-id");
    let c1 = v1
        .add_bank_connection(ConnectorKind::TradeRepublic, String::new(), "a".into())
        .unwrap();
    store_fetch(
        &v1,
        ConnectorKind::TradeRepublic,
        c1,
        tr_fetch(
            vec![credit()],
            vec![tr_dividend("d", "tr:depot", Some("div-1"))],
        ),
    )
    .unwrap();
    assert_eq!(doc_places(&v1, "d"), expected);
    // 2: from the activity list (no booking id), the document first, the booking later: by content
    let (_d2, v2) = open_vault("girocrab-tr-div-content");
    let c2 = v2
        .add_bank_connection(ConnectorKind::TradeRepublic, String::new(), "a".into())
        .unwrap();
    store_fetch(
        &v2,
        ConnectorKind::TradeRepublic,
        c2,
        tr_fetch(vec![], vec![tr_dividend("d", "tr:depot", None)]),
    )
    .unwrap();
    assert_eq!(doc_places(&v2, "d"), ("tr:depot".to_string(), None));
    store_fetch(
        &v2,
        ConnectorKind::TradeRepublic,
        c2,
        tr_fetch(vec![credit()], vec![]),
    )
    .unwrap();
    assert_eq!(doc_places(&v2, "d"), expected);
    // 3: stored by an earlier version at the cash account with its booking: sorting anew moves the
    // document to the depot and keeps the booking, the PDF and the source; a second run changes nothing
    let (_d3, v3) = open_vault("girocrab-tr-div-legacy");
    let c3 = v3
        .add_bank_connection(ConnectorKind::TradeRepublic, String::new(), "a".into())
        .unwrap();
    store_fetch(
        &v3,
        ConnectorKind::TradeRepublic,
        c3,
        tr_fetch(
            vec![credit()],
            vec![tr_dividend("d", "tr:cash", Some("div-1"))],
        ),
    )
    .unwrap();
    assert_eq!(
        doc_places(&v3, "d"),
        ("tr:cash".to_string(), Some("div-1".to_string()))
    );
    let first = v3.reset_trade_republic_history(c3).unwrap();
    assert_eq!(first.resorted, 1);
    assert_eq!(doc_places(&v3, "d"), expected);
    assert_eq!(v3.reset_trade_republic_history(c3).unwrap().resorted, 0);
    assert_eq!(doc_places(&v3, "d"), expected);
    assert_eq!(v3.list_all_documents(Default::default()).unwrap().len(), 1);
}

#[test]
fn linking_by_hand_keeps_the_documents_account_and_a_chosen_account_survives_sorting_anew() {
    let (_dir, v) = open_vault("girocrab-tr-manual");
    let c = v
        .add_bank_connection(ConnectorKind::TradeRepublic, String::new(), "a".into())
        .unwrap();
    store_fetch(
        &v,
        ConnectorKind::TradeRepublic,
        c,
        tr_fetch(
            vec![
                tr_booking("div-1", "2025-11-16", 17_28, "Realty Income"),
                tr_booking("other", "2025-12-20", -5_00, "Etwas anderes"),
            ],
            vec![tr_dividend("d", "tr:depot", Some("div-1"))],
        ),
    )
    .unwrap();
    let id = v.list_all_documents(Default::default()).unwrap()[0].id;
    // the user links another booking: the document stays at the depot
    v.link_document(id, Some(booking_id_of(&v, "other")))
        .unwrap();
    assert_eq!(
        doc_places(&v, "d"),
        ("tr:depot".to_string(), Some("other".to_string()))
    );
    // the user chooses the cash account: the link stays, the choice is remembered
    let cash = account_id_of(&v, "tr:cash");
    v.set_document_account(id, Some(cash)).unwrap();
    assert_eq!(
        doc_places(&v, "d"),
        ("tr:cash".to_string(), Some("other".to_string()))
    );
    let manual: i64 = v
        .db()
        .unwrap()
        .query_row(
            "SELECT account_manual FROM booking_documents WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(manual, 1);
    // sorting anew (and the history rebuild) leaves that choice alone
    assert_eq!(v.reset_trade_republic_history(c).unwrap().resorted, 0);
    assert_eq!(doc_places(&v, "d").0, "tr:cash");
}
