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
    super::drop_booked(&v.db().unwrap(), a, 0).unwrap();
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
    super::drop_booked(&v.db().unwrap(), a, 0).unwrap();
    assert_eq!(v.count_planned_payments().unwrap(), 1);
}

#[test]
fn booking_before_the_order_does_not_settle_it() {
    let v = vault("planned-old-booking");
    let a = account(&v, "Giro");
    v.record_planned_transfer(a, &job("E1", Some("2999-01-15"), 500))
        .unwrap();
    book(&v, a, "old", "2000-01-01", -500, "DE89370400440532013000");
    super::drop_booked(&v.db().unwrap(), a, 0).unwrap();
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
    let now = crate::dates::now_ms() / 1000;
    super::drop_booked(&v.db().unwrap(), a, now - 5).unwrap();
    assert_eq!(v.count_planned_payments().unwrap(), 1);
    // the next fetch brings no new booking
    super::drop_booked(&v.db().unwrap(), a, now + 60).unwrap();
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
    super::drop_booked(&v.db().unwrap(), a, 0).unwrap();
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
    super::drop_booked(&v.db().unwrap(), a, 0).unwrap();
    assert_eq!(v.count_planned_payments().unwrap(), 1);
    book(
        &v,
        a,
        "t",
        &crate::dates::today_iso(),
        -300,
        "DE89370400440532013000",
    );
    super::drop_booked(&v.db().unwrap(), a, 0).unwrap();
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
    super::drop_booked(&v.db().unwrap(), a, 0).unwrap();
    assert_eq!(v.count_planned_payments().unwrap(), 0);
}
