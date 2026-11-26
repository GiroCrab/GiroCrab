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

fn settled_count(v: &Vault) -> i64 {
    v.db()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM planned_transfers WHERE settled_ms IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

fn settle_all(v: &Vault) {
    v.db()
        .unwrap()
        .execute("UPDATE planned_transfers SET settled_ms = 1000", [])
        .unwrap();
}

/// Two vaults with the same account and the same scheduled order `E1`.
fn merge_pair(tag: &str) -> (Arc<Vault>, Arc<Vault>) {
    let a = vault(&format!("{tag}-a"));
    let b = vault(&format!("{tag}-b"));
    for v in [&a, &b] {
        let acc = account(v, "Giro");
        v.record_planned_transfer(acc, &job("E1", Some("2999-01-15"), 900))
            .unwrap();
    }
    (a, b)
}

#[test]
fn booking_marks_the_order_settled_and_hides_it() {
    let v = vault("planned-settle-marks");
    let a = account(&v, "Giro");
    v.record_planned_transfer(a, &job("E1", Some("2999-01-15"), 900))
        .unwrap();
    book(&v, a, "x1", "2999-01-10", -900, "DE89370400440532013000");
    settle(&v, a);
    let rows: i64 = v
        .db()
        .unwrap()
        .query_row("SELECT count(*) FROM planned_transfers", [], |r| r.get(0))
        .unwrap();
    assert_eq!((rows, settled_count(&v)), (1, 1));
    assert_eq!(v.count_planned_payments().unwrap(), 0);
    let list = v.list_planned_payments(None, None, vec![]).unwrap();
    assert!(list.is_empty());
    let totals = v.planned_payment_totals(None, None, vec![]).unwrap();
    assert_eq!((totals.count, totals.outgoing_cents), (0, 0));
}

#[test]
fn settled_order_is_not_matched_again() {
    let v = vault("planned-settled-once");
    let a = account(&v, "Giro");
    v.record_planned_transfer(a, &job("E1", Some("2999-01-15"), 700))
        .unwrap();
    book(&v, a, "x1", "2999-01-10", -700, "DE89370400440532013000");
    settle(&v, a);
    let first: i64 = v
        .db()
        .unwrap()
        .query_row("SELECT settled_ms FROM planned_transfers", [], |r| r.get(0))
        .unwrap();
    // a second equal booking must stay free for the next order, not re-settle the first
    book(&v, a, "x2", "2999-01-11", -700, "DE89370400440532013000");
    v.record_planned_transfer(a, &job("E2", Some("2999-02-15"), 700))
        .unwrap();
    settle(&v, a);
    let again: i64 = v
        .db()
        .unwrap()
        .query_row(
            "SELECT settled_ms FROM planned_transfers WHERE end_to_end_id = 'E1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(first, again);
    assert_eq!(settled_count(&v), 2);
}

#[test]
fn merge_keeps_a_locally_settled_order_hidden() {
    let (a, b) = merge_pair("planned-merge-local");
    settle_all(&a);
    a.merge_peer_snapshot(b.file_path()).unwrap();
    assert_eq!(settled_count(&a), 1);
    assert_eq!(a.count_planned_payments().unwrap(), 0);
}

#[test]
fn merge_settles_a_locally_open_order_the_other_settled() {
    let (a, b) = merge_pair("planned-merge-other");
    settle_all(&b);
    b.checkpoint().unwrap();
    a.merge_peer_snapshot(b.file_path()).unwrap();
    assert_eq!(settled_count(&a), 1);
    assert_eq!(a.count_planned_payments().unwrap(), 0);
}

#[test]
fn merge_from_an_other_without_settled_column_keeps_rows_open() {
    let v = vault("planned-merge-legacy");
    let acc = account(&v, "Giro");
    v.record_planned_transfer(acc, &job("E1", Some("2999-01-15"), 900))
        .unwrap();
    let db = v.db().unwrap();
    db.execute_batch(
        "ATTACH ':memory:' AS other;
         CREATE TABLE other.planned_transfers (
            id INTEGER PRIMARY KEY, account_id INTEGER NOT NULL,
            end_to_end_id TEXT NOT NULL UNIQUE, execution_date TEXT NOT NULL,
            recipient_name TEXT NOT NULL DEFAULT '', recipient_iban TEXT NOT NULL DEFAULT '',
            amount_cents INTEGER NOT NULL, purpose TEXT NOT NULL DEFAULT '',
            created_ms INTEGER NOT NULL DEFAULT 0);
         INSERT INTO other.planned_transfers(account_id, end_to_end_id, execution_date, amount_cents)
            VALUES (1, 'E1', '2999-01-15', 900), (1, 'E9', '2999-03-01', 50);",
    )
    .unwrap();
    let map = std::collections::HashMap::from([(1i64, acc)]);
    super::merge_attached(&db, &map).unwrap();
    drop(db);
    assert_eq!(settled_count(&v), 0);
    assert_eq!(v.count_planned_payments().unwrap(), 2);
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

fn book_ref(v: &Vault, acc: i64, ext: &str, cents: i64, e2e: &str) {
    v.db()
        .unwrap()
        .execute(
            "INSERT INTO transactions(account_id, external_id, booking_date, amount_cents,
                    counterparty_iban, end_to_end_id)
             VALUES (?1, ?2, '2999-01-10', ?3, 'DE89370400440532013000', ?4)",
            rusqlite::params![acc, ext, cents, e2e],
        )
        .unwrap();
}

fn open_orders(v: &Vault) -> Vec<String> {
    v.list_planned_payments(None, None, vec![])
        .unwrap()
        .into_iter()
        .map(|p| p.date)
        .collect()
}

#[test]
fn booking_with_a_reference_settles_only_the_order_it_names() {
    let v = vault("planned-reference");
    let a = account(&v, "Giro");
    // E1 (January) is older than E2 (December); same amount and IBAN
    v.record_planned_transfer(a, &job("E1", Some("2999-01-15"), 800))
        .unwrap();
    v.record_planned_transfer(a, &job("E2", Some("2999-12-15"), 800))
        .unwrap();
    book_ref(&v, a, "x1", -800, "E2");
    settle(&v, a);
    assert_eq!(open_orders(&v), ["2999-01-15"]);
    // a booking naming an unknown order settles nothing; the placeholder counts as none
    book_ref(&v, a, "x2", -800, "E9");
    settle(&v, a);
    assert_eq!(open_orders(&v), ["2999-01-15"]);
    book_ref(&v, a, "x3", -800, "NOTPROVIDED");
    settle(&v, a);
    assert!(open_orders(&v).is_empty());
}

/// Without a reference the earliest order takes the booking, whatever its local row id, so
/// two replicas with the rows in another order settle the same one.
#[test]
fn booking_without_reference_settles_the_earliest_order_not_the_lowest_id() {
    let v = vault("planned-stable-order");
    let a = account(&v, "Giro");
    v.record_planned_transfer(a, &job("E2", Some("2999-12-15"), 600))
        .unwrap();
    v.record_planned_transfer(a, &job("E1", Some("2999-01-15"), 600))
        .unwrap();
    book(&v, a, "x1", "2999-01-10", -600, "DE89370400440532013000");
    settle(&v, a);
    assert_eq!(open_orders(&v), ["2999-12-15"]);
}

#[test]
fn merge_that_brings_nothing_new_changes_no_row() {
    let v = vault("planned-merge-noop");
    let acc = account(&v, "Giro");
    v.record_planned_transfer(acc, &job("E1", Some("2999-01-15"), 900))
        .unwrap();
    v.record_planned_transfer(acc, &job("E2", Some("2999-02-15"), 900))
        .unwrap();
    let db = v.db().unwrap();
    db.execute(
        "UPDATE planned_transfers SET settled_ms = 5 WHERE end_to_end_id = 'E2'",
        [],
    )
    .unwrap();
    // the other side: E1 open like here, E2 settled at another time
    db.execute_batch(
        "ATTACH ':memory:' AS other;
         CREATE TABLE other.planned_transfers AS SELECT * FROM main.planned_transfers;
         UPDATE other.planned_transfers SET settled_ms = 9 WHERE end_to_end_id = 'E2';",
    )
    .unwrap();
    let map = std::collections::HashMap::from([(acc, acc)]);
    let before = db.total_changes();
    super::merge_attached(&db, &map).unwrap();
    assert_eq!(db.total_changes(), before);
    let kept: i64 = db
        .query_row(
            "SELECT settled_ms FROM planned_transfers WHERE end_to_end_id = 'E2'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(kept, 5);
}

#[test]
fn an_order_whose_date_has_passed_does_not_take_the_booking() {
    let v = vault("planned-past-order");
    let a = account(&v, "Giro");
    // an order of yesterday (still stored, no longer listed; earlier, so it would be tried
    // first) and the one actually booked
    let yesterday = (crate::dates::today() - jiff::Span::new().days(1)).to_string();
    v.record_planned_transfer(a, &job("OLD", Some(&yesterday), 600))
        .unwrap();
    v.record_planned_transfer(a, &job("NEW", Some("2999-01-15"), 600))
        .unwrap();
    book(&v, a, "x", "2999-01-10", -600, "DE89370400440532013000");
    settle(&v, a);
    let settled: Vec<(String, bool)> = v
        .db()
        .unwrap()
        .prepare("SELECT end_to_end_id, settled_ms IS NOT NULL FROM planned_transfers ORDER BY 1")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(settled, [("NEW".into(), true), ("OLD".into(), false)]);
}

/// A booking that gets its reference only when the bank delivers it again (a later fetch
/// that inserts nothing) still settles the order it names.
#[test]
fn booking_that_gains_the_reference_later_settles_the_order() {
    let v = vault("planned-late-reference");
    let a = account(&v, "Giro");
    v.record_planned_transfer(a, &job("E1", Some("2999-01-15"), 800))
        .unwrap();
    // first fetch: no reference and another IBAN, so it does not match
    book(&v, a, "x1", "2999-01-10", -800, "DE02120300000000202051");
    settle(&v, a);
    assert_eq!(v.count_planned_payments().unwrap(), 1);
    // the next fetch fills the reference in; the booking is not new
    v.db()
        .unwrap()
        .execute(
            "UPDATE transactions SET end_to_end_id = 'E1' WHERE external_id = 'x1'",
            [],
        )
        .unwrap();
    super::drop_booked(&v.db().unwrap(), a, &Default::default()).unwrap();
    assert_eq!(v.count_planned_payments().unwrap(), 0);
}

/// The booking reached this vault before its order: the merge that brings the order in
/// open settles it, since the booking names it.
#[test]
fn merge_settles_an_arriving_order_a_stored_booking_names() {
    let with_order = vault("planned-merge-arrives-a");
    let acc = account(&with_order, "Giro");
    with_order
        .record_planned_transfer(acc, &job("E1", Some("2999-01-15"), 900))
        .unwrap();
    with_order.checkpoint().unwrap();
    let with_booking = vault("planned-merge-arrives-b");
    let mine = account(&with_booking, "Giro");
    book_ref(&with_booking, mine, "x1", -900, "E1");
    with_booking
        .merge_peer_snapshot(with_order.file_path())
        .unwrap();
    assert_eq!(settled_count(&with_booking), 1);
    assert_eq!(with_booking.count_planned_payments().unwrap(), 0);
}

fn only_account(v: &Vault) -> i64 {
    v.db()
        .unwrap()
        .query_row("SELECT id FROM accounts", [], |r| r.get(0))
        .unwrap()
}

fn reference_of(v: &Vault, ext: &str) -> String {
    v.db()
        .unwrap()
        .query_row(
            "SELECT end_to_end_id FROM transactions WHERE external_id = ?1",
            [ext],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn usable_reference_is_neither_blank_nor_the_placeholder() {
    assert!(super::names_order(" E1 "));
    assert!(!super::names_order("  "));
    assert!(!super::names_order(" notprovided "));
}

/// The booking both vaults have names `E1` only on the other side (here the placeholder): the
/// merge matches it (no second row), takes the reference over and settles the order.
#[test]
fn merge_takes_the_peer_reference_of_a_matched_booking() {
    let (a, b) = merge_pair("planned-merge-peer-ref");
    book_ref(&a, only_account(&a), "x1", -900, "NOTPROVIDED");
    book_ref(&b, only_account(&b), "x1", -900, "E1");
    b.checkpoint().unwrap();
    a.merge_peer_snapshot(b.file_path()).unwrap();
    let rows: i64 = a
        .db()
        .unwrap()
        .query_row("SELECT count(*) FROM transactions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 1);
    assert_eq!(reference_of(&a, "x1"), "E1");
    assert_eq!(settled_count(&a), 1);
    assert_eq!(a.count_planned_payments().unwrap(), 0);
}

/// A usable reference of our own stays: the order only the other side's reference names
/// remains open.
#[test]
fn merge_keeps_the_own_reference_of_a_matched_booking() {
    let (a, b) = merge_pair("planned-merge-own-ref");
    book_ref(&a, only_account(&a), "x1", -900, "E7");
    book_ref(&b, only_account(&b), "x1", -900, "E1");
    b.checkpoint().unwrap();
    a.merge_peer_snapshot(b.file_path()).unwrap();
    assert_eq!(reference_of(&a, "x1"), "E7");
    assert_eq!(settled_count(&a), 0);
    assert_eq!(a.count_planned_payments().unwrap(), 1);
}
