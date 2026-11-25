use std::sync::Arc;

use crate::connectors::TransferJob;
use crate::vault::Vault;

fn vault(tag: &str) -> Arc<Vault> {
    Vault::open(
        crate::test_support::thread_temp_path(tag)
            .to_string_lossy()
            .into(),
        vec![7u8; 32],
    )
    .unwrap()
}

fn account(v: &Vault, name: &str) -> i64 {
    let db = v.db().unwrap();
    db.execute(
        "INSERT INTO account_groups(name, sf_symbol, sort_order) VALUES ('G', 'tray', 1)
         ON CONFLICT DO NOTHING",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO accounts(group_id, external_ref, name, kind)
         VALUES ((SELECT min(id) FROM account_groups), ?1, ?1, 'checking')",
        [name],
    )
    .unwrap();
    db.last_insert_rowid()
}

fn job(e2e: &str, date: Option<&str>, cents: i64) -> TransferJob {
    TransferJob {
        source_iban: "DE02120300000000202051".into(),
        source_owner: "Max".into(),
        recipient_name: "Vermieter".into(),
        recipient_iban: "DE89370400440532013000".into(),
        recipient_bic: None,
        amount_cents: cents,
        purpose: "Miete".into(),
        instant: false,
        execution_date: date.map(str::to_string),
        end_to_end_id: e2e.into(),
    }
}

#[test]
fn scheduled_transfer_is_listed_as_outgoing() {
    let v = vault("planned-list");
    let a = account(&v, "Giro");
    v.record_planned_transfer(a, &job("E1", Some("2999-01-15"), 85_000))
        .unwrap();
    let rows = v.list_planned_payments(None, None, vec![]).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].amount_cents, -85_000);
    assert_eq!(rows[0].date, "2999-01-15");
    assert_eq!(rows[0].account_name, "Giro");
    let t = v.planned_payment_totals(None, None, vec![]).unwrap();
    assert_eq!(
        (t.count, t.outgoing_cents, t.incoming_cents),
        (1, -85_000, 0)
    );
}

#[test]
fn same_order_is_stored_once_and_without_date_not_at_all() {
    let v = vault("planned-dup");
    let a = account(&v, "Giro");
    let j = job("E1", Some("2999-01-15"), 100);
    v.record_planned_transfer(a, &j).unwrap();
    v.record_planned_transfer(a, &j).unwrap();
    v.record_planned_transfer(a, &job("E2", None, 100)).unwrap();
    assert_eq!(
        v.list_planned_payments(None, None, vec![]).unwrap().len(),
        1
    );
}

#[test]
fn past_orders_range_and_account_filter() {
    let v = vault("planned-filter");
    let a = account(&v, "Giro");
    let b = account(&v, "Tagesgeld");
    v.record_planned_transfer(a, &job("OLD", Some("2000-01-01"), 1))
        .unwrap();
    v.record_planned_transfer(a, &job("E1", Some("2999-01-15"), 10))
        .unwrap();
    v.record_planned_transfer(b, &job("E2", Some("2999-03-01"), 20))
        .unwrap();
    // executed in the past: gone from the default view
    assert_eq!(
        v.list_planned_payments(None, None, vec![]).unwrap().len(),
        2
    );
    let r = v
        .list_planned_payments(None, Some("2999-02-01".into()), vec![])
        .unwrap();
    assert_eq!(r.len(), 1);
    let r = v.list_planned_payments(None, None, vec![b]).unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].account_id, b);
    assert!(
        v.list_planned_payments(Some("kaputt".into()), None, vec![])
            .is_err()
    );
}

#[test]
fn planned_payments_are_counted_without_past_ones() {
    let v = vault("planned-count");
    let a = account(&v, "Giro");
    assert_eq!(v.count_planned_payments().unwrap(), 0);
    v.record_planned_transfer(a, &job("E1", Some("2999-01-15"), 100))
        .unwrap();
    v.record_planned_transfer(a, &job("E2", Some("2999-02-15"), 200))
        .unwrap();
    v.record_planned_transfer(a, &job("E3", Some("2000-01-01"), 300))
        .unwrap();
    assert_eq!(v.count_planned_payments().unwrap(), 2);
}

/// Ids of every booking in the vault, as if one fetch had inserted them all.
fn all_booked(v: &Vault) -> std::collections::HashSet<i64> {
    let db = v.db().unwrap();
    let mut q = db.prepare("SELECT id FROM transactions").unwrap();
    q.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// `drop_booked` for a fetch that inserted every booking. The ids are read first: `v.db()`
/// holds the vault lock, so `all_booked` must not run while its guard is alive.
fn settle(v: &Vault, account: i64) {
    let fetched = all_booked(v);
    super::drop_booked(&v.db().unwrap(), account, &fetched).unwrap();
}

fn book(v: &Vault, acc: i64, ext: &str, date: &str, cents: i64, iban: &str) {
    v.db()
        .unwrap()
        .execute(
            "INSERT INTO transactions(account_id, external_id, booking_date, amount_cents, counterparty_iban)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![acc, ext, date, cents, iban],
        )
        .unwrap();
}

#[test]
fn booked_order_leaves_the_list_once_per_booking() {
    let v = vault("planned-booked");
    let a = account(&v, "Giro");
    v.record_planned_transfer(a, &job("E1", Some("2999-01-15"), 85_000))
        .unwrap();
    v.record_planned_transfer(a, &job("E2", Some("2999-02-15"), 85_000))
        .unwrap();
    // no booking yet, and an unrelated one (other amount, other IBAN): both stay
    book(&v, a, "x1", "2999-01-10", -1, "DE89370400440532013000");
    book(&v, a, "x2", "2999-01-10", -85_000, "DE02120300000000202051");
    settle(&v, a);
    assert_eq!(v.count_planned_payments().unwrap(), 2);
    // one booking settles one order only
    book(
        &v,
        a,
        "x3",
        "2999-01-10",
        -85_000,
        "DE89 3704 0044 0532 0130 00",
    );
    settle(&v, a);
    assert_eq!(v.count_planned_payments().unwrap(), 1);
}

#[test]
fn booking_before_the_order_does_not_settle_it() {
    let v = vault("planned-old-booking");
    let a = account(&v, "Giro");
    v.record_planned_transfer(a, &job("E1", Some("2999-01-15"), 500))
        .unwrap();
    book(&v, a, "old", "2000-01-01", -500, "DE89370400440532013000");
    settle(&v, a);
    assert_eq!(v.count_planned_payments().unwrap(), 1);
}

#[test]
fn one_booking_does_not_settle_a_second_order_in_a_later_fetch() {
    let v = vault("planned-later-fetch");
    let a = account(&v, "Giro");
    v.record_planned_transfer(a, &job("E1", Some("2999-01-15"), 700))
        .unwrap();
    v.record_planned_transfer(a, &job("E2", Some("2999-02-15"), 700))
        .unwrap();
    book(&v, a, "x1", "2999-01-10", -700, "DE89370400440532013000");
    let fetched = all_booked(&v);
    super::drop_booked(&v.db().unwrap(), a, &fetched).unwrap();
    assert_eq!(v.count_planned_payments().unwrap(), 1);
    // the next fetch inserts no new booking
    super::drop_booked(&v.db().unwrap(), a, &Default::default()).unwrap();
    assert_eq!(v.count_planned_payments().unwrap(), 1);
}

#[test]
fn pending_rows_are_no_bookings() {
    let v = vault("planned-pending");
    let a = account(&v, "Giro");
    v.record_planned_transfer(a, &job("E1", Some("2999-01-15"), 900))
        .unwrap();
    book(
        &v,
        a,
        "pending-E9",
        "2999-01-10",
        -900,
        "DE89370400440532013000",
    );
    book(&v, a, "advanzia-hold-1", "2999-01-10", -900, "");
    settle(&v, a);
    assert_eq!(v.count_planned_payments().unwrap(), 1);
}

#[test]
fn booking_of_the_day_before_the_order_does_not_settle_it() {
    let v = vault("planned-day-before");
    let a = account(&v, "Giro");
    v.record_planned_transfer(a, &job("E1", Some("2999-01-15"), 300))
        .unwrap();
    let yesterday = (crate::dates::today() - jiff::Span::new().days(1)).to_string();
    book(&v, a, "y", &yesterday, -300, "DE89370400440532013000");
    settle(&v, a);
    assert_eq!(v.count_planned_payments().unwrap(), 1);
    book(
        &v,
        a,
        "t",
        &crate::dates::today_iso(),
        -300,
        "DE89370400440532013000",
    );
    settle(&v, a);
    assert_eq!(v.count_planned_payments().unwrap(), 0);
}

#[test]
fn exact_iban_booking_is_reserved_before_empty_iban_fallback() {
    let v = vault("planned-exact-first");
    let a = account(&v, "Giro");
    let mut other = job("E2", Some("2999-02-15"), 400);
    other.recipient_iban = "DE02120300000000202051".into();
    v.record_planned_transfer(a, &job("E1", Some("2999-01-15"), 400))
        .unwrap();
    v.record_planned_transfer(a, &other).unwrap();
    // the booking without an IBAN comes first, then the one of the first order
    book(&v, a, "n", "2999-01-10", -400, "");
    book(&v, a, "e", "2999-01-10", -400, "DE89370400440532013000");
    settle(&v, a);
    assert_eq!(v.count_planned_payments().unwrap(), 0);
}

#[test]
fn an_order_whose_date_has_passed_does_not_take_the_booking() {
    let v = vault("planned-past-order");
    let a = account(&v, "Giro");
    // Yesterday, not long ago: `prune_old` would delete an older row on the next insert.
    let yesterday = (crate::dates::today() - jiff::Span::new().days(1)).to_string();
    v.record_planned_transfer(a, &job("OLD", Some(&yesterday), 600))
        .unwrap();
    v.record_planned_transfer(a, &job("NEW", Some("2999-01-15"), 600))
        .unwrap();
    book(&v, a, "x", "2999-01-10", -600, "DE89370400440532013000");
    settle(&v, a);
    // the booking settles the listed order; the executed one stays untouched
    let db = v.db().unwrap();
    let left: Vec<String> = db
        .prepare("SELECT end_to_end_id FROM planned_transfers")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(left, ["OLD"]);
}

#[test]
fn settled_step_adds_null_column_and_is_idempotent() {
    let v = vault("planned-settled-step");
    let acc = account(&v, "A");
    let db = v.db().unwrap();
    db.execute(
        "INSERT INTO planned_transfers(account_id, end_to_end_id, execution_date, amount_cents)
         VALUES (?1, 'E2E', '2030-01-01', 100)",
        [acc],
    )
    .unwrap();
    assert!(super::has_column(&db, "main", "planned_transfers", "settled_ms").unwrap());
    let settled: Option<i64> = db
        .query_row("SELECT settled_ms FROM planned_transfers", [], |r| r.get(0))
        .unwrap();
    assert_eq!(settled, None);
    super::planned_settled(&db).unwrap();
    super::planned_settled(&db).unwrap();
    let idx: Vec<String> = db
        .prepare(
            "SELECT name FROM sqlite_master WHERE type='index' AND tbl_name='planned_transfers'",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(idx.contains(&"planned_transfers_date_settled".to_string()));
    assert!(!idx.contains(&"planned_transfers_date".to_string()));
}

/// The upgrade path of an existing vault: a table from before step 43 with the
/// old date index and a stored order keeps the order (open, `settled_ms` NULL),
/// and a replay of the step keeps a settlement set in between.
#[test]
fn settled_step_upgrades_old_table_and_keeps_rows() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE planned_transfers(
            id INTEGER PRIMARY KEY,
            account_id INTEGER NOT NULL,
            end_to_end_id TEXT NOT NULL,
            execution_date TEXT NOT NULL,
            amount_cents INTEGER NOT NULL);
         CREATE INDEX planned_transfers_date ON planned_transfers(execution_date);
         INSERT INTO planned_transfers(account_id, end_to_end_id, execution_date, amount_cents)
            VALUES (1, 'E2E', '2030-01-01', 100);",
    )
    .unwrap();
    assert!(!super::has_column(&db, "main", "planned_transfers", "settled_ms").unwrap());

    fn row(db: &rusqlite::Connection) -> (i64, String, String, i64, Option<i64>) {
        db.query_row(
            "SELECT account_id, end_to_end_id, execution_date, amount_cents, settled_ms
             FROM planned_transfers",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap()
    }
    fn indexes(db: &rusqlite::Connection) -> Vec<String> {
        db.prepare(
            "SELECT name FROM sqlite_master WHERE type='index' AND tbl_name='planned_transfers'",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
    }

    super::planned_settled(&db).unwrap();
    assert!(super::has_column(&db, "main", "planned_transfers", "settled_ms").unwrap());
    assert_eq!(row(&db), (1, "E2E".into(), "2030-01-01".into(), 100, None));
    let idx = indexes(&db);
    assert!(idx.contains(&"planned_transfers_date_settled".to_string()));
    assert!(!idx.contains(&"planned_transfers_date".to_string()));

    db.execute(
        "UPDATE planned_transfers SET settled_ms = 1700000000000",
        [],
    )
    .unwrap();
    super::planned_settled(&db).unwrap();
    assert_eq!(
        row(&db),
        (
            1,
            "E2E".into(),
            "2030-01-01".into(),
            100,
            Some(1_700_000_000_000)
        )
    );
    assert_eq!(indexes(&db), idx);
}
