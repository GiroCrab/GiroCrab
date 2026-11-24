use std::sync::Arc;

use rusqlite::params;

use super::ManualBookingSetup;
use crate::error::CoreError;
use crate::models::{AccountKind, TransactionSplitInput};
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

fn setup(date: &str, cents: i64, who: &str) -> ManualBookingSetup {
    ManualBookingSetup {
        booking_date: date.into(),
        amount_cents: cents,
        counterparty: who.into(),
        purpose: String::new(),
        category_id: None,
        note: String::new(),
    }
}

fn balance(v: &Vault, id: i64) -> i64 {
    v.list_accounts()
        .unwrap()
        .into_iter()
        .find(|a| a.id == id)
        .unwrap()
        .balance_cents
}

fn live_count(v: &Vault, id: i64) -> i64 {
    v.db()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM transactions WHERE account_id = ?1 AND deleted_ms IS NULL",
            [id],
            |r| r.get(0),
        )
        .unwrap()
}

/// CodeRabbit (#1855): a date-only edit on one device and a split edit on the other both
/// survive the merge, in either order (the date travels with the text, not with the splits).
#[test]
fn date_only_edit_does_not_outrank_a_peer_split_edit() {
    for date_first in [true, false] {
        let a = vault("date-vs-split-a");
        let b = vault("date-vs-split-b");
        let account = a
            .create_offline_account("Wallet".into(), AccountKind::Cash, "EUR".into(), 0)
            .unwrap();
        let id = a
            .save_manual_booking(account, None, setup("2026-01-10", -1000, "Shop"))
            .unwrap();
        b.merge_other_vault(a.file_path(), vec![7; 32]).unwrap();
        let bid: i64 = b
            .db()
            .unwrap()
            .query_row(
                "SELECT id FROM transactions WHERE counterparty='Shop'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let part = |cents| TransactionSplitInput {
            uuid: None,
            category_id: None,
            amount_cents: cents,
            note: String::new(),
        };
        let splits_ms = |v: &Vault, id: i64| -> i64 {
            v.db()
                .unwrap()
                .query_row(
                    "SELECT splits_ms FROM transactions WHERE id=?1",
                    [id],
                    |r| r.get(0),
                )
                .unwrap()
        };
        let date_edit = || {
            let before = splits_ms(&a, id);
            a.save_manual_booking(account, Some(id), setup("2026-01-12", -1000, "Shop"))
                .unwrap();
            // a date-only edit leaves the amount/split unit's stamp alone
            assert_eq!(splits_ms(&a, id), before);
        };
        let split_edit = || {
            b.set_transaction_splits(bid, vec![part(-400), part(-600)])
                .unwrap()
        };
        if date_first {
            date_edit();
            split_edit();
        } else {
            split_edit();
            date_edit();
        }
        a.merge_other_vault(b.file_path(), vec![7; 32]).unwrap();
        b.merge_other_vault(a.file_path(), vec![7; 32]).unwrap();
        for (v, id) in [(&a, id), (&b, bid)] {
            let date: String = v
                .db()
                .unwrap()
                .query_row(
                    "SELECT booking_date FROM transactions WHERE id=?1",
                    [id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(date, "2026-01-12", "date_first={date_first}");
            assert_eq!(
                v.list_transaction_splits(id).unwrap().len(),
                2,
                "date_first={date_first}"
            );
        }
    }
}

/// CodeRabbit (#1853): a newer date edit survives a peer's later text-only edit, and the text
/// edit survives too (each has its own stamp).
#[test]
fn date_edit_and_peer_text_edit_both_survive() {
    let a = vault("date-vs-text-a");
    let b = vault("date-vs-text-b");
    let account = a
        .create_offline_account("Wallet".into(), AccountKind::Cash, "EUR".into(), 0)
        .unwrap();
    let id = a
        .save_manual_booking(account, None, setup("2026-01-10", -1000, "Shop"))
        .unwrap();
    b.merge_other_vault(a.file_path(), vec![7; 32]).unwrap();
    let (bid, bacc): (i64, i64) = b
        .db()
        .unwrap()
        .query_row(
            "SELECT id, account_id FROM transactions WHERE counterparty='Shop'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    a.save_manual_booking(account, Some(id), setup("2026-01-12", -1000, "Shop"))
        .unwrap();
    b.save_manual_booking(bacc, Some(bid), setup("2026-01-10", -1000, "Shop GmbH"))
        .unwrap();
    a.merge_other_vault(b.file_path(), vec![7; 32]).unwrap();
    b.merge_other_vault(a.file_path(), vec![7; 32]).unwrap();
    for (v, id) in [(&a, id), (&b, bid)] {
        let got: (String, String) = v
            .db()
            .unwrap()
            .query_row(
                "SELECT booking_date, counterparty FROM transactions WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(got, ("2026-01-12".into(), "Shop GmbH".into()));
    }
}

/// CodeRabbit (#1855): the text stamp only ever grows, also when the stored stamp is ahead
/// of this device's clock (set by a peer).
#[test]
fn text_stamp_advances_past_a_stamp_ahead_of_the_clock() {
    let v = vault("text-stamp-ahead");
    let account = v
        .create_offline_account("Wallet".into(), AccountKind::Cash, "EUR".into(), 0)
        .unwrap();
    let id = v
        .save_manual_booking(account, None, setup("2026-01-10", -1000, "Shop"))
        .unwrap();
    let ahead = crate::dates::now_ms() + 86_400_000;
    v.db()
        .unwrap()
        .execute(
            "UPDATE transactions SET text_ms=?2, date_ms=?2 WHERE id=?1",
            params![id, ahead],
        )
        .unwrap();
    v.save_manual_booking(account, Some(id), setup("2026-01-11", -1000, "Shop GmbH"))
        .unwrap();
    let (text_ms, date_ms): (i64, i64) = v
        .db()
        .unwrap()
        .query_row(
            "SELECT text_ms, date_ms FROM transactions WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((text_ms, date_ms), (ahead + 1, ahead + 1));
}

#[test]
fn round7_manual_dates_sync_with_amount_and_date_only_edits() {
    for amount in [-1000, -500] {
        let v = vault("round7-manual-dates");
        let peer = vault("round7-manual-dates-peer");
        let account = v
            .create_offline_account("Wallet".into(), AccountKind::Cash, "EUR".into(), 0)
            .unwrap();
        let id = v
            .save_manual_booking(account, None, setup("2026-01-31", -500, "Shop"))
            .unwrap();
        peer.merge_other_vault(v.file_path(), vec![7; 32]).unwrap();
        v.save_manual_booking(account, Some(id), setup("2026-02-01", amount, "Shop"))
            .unwrap();
        peer.merge_other_vault(v.file_path(), vec![7; 32]).unwrap();
        let got: (String, String, i64) = peer.db().unwrap().query_row("SELECT booking_date,value_date,amount_cents FROM transactions WHERE counterparty='Shop'", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(got, ("2026-02-01".into(), "2026-02-01".into(), amount));
    }
}

#[test]
fn round7_zero_split_metadata_save_is_allowed_but_new_zero_is_not() {
    let v = vault("round7-zero-split");
    let account = v
        .create_offline_account("Wallet".into(), AccountKind::Cash, "EUR".into(), 0)
        .unwrap();
    assert!(
        v.save_manual_booking(account, None, setup("2026-01-01", 0, "Shop"))
            .is_err()
    );
    let id = v
        .save_manual_booking(account, None, setup("2026-01-01", 100, "Shop"))
        .unwrap();
    v.db()
        .unwrap()
        .execute("UPDATE transactions SET amount_cents=0 WHERE id=?1", [id])
        .unwrap();
    v.set_transaction_splits(
        id,
        vec![
            TransactionSplitInput {
                uuid: None,
                category_id: None,
                amount_cents: 100,
                note: String::new(),
            },
            TransactionSplitInput {
                uuid: None,
                category_id: None,
                amount_cents: -100,
                note: String::new(),
            },
        ],
    )
    .unwrap();
    v.save_manual_booking(account, Some(id), setup("2026-01-01", 0, "Updated"))
        .unwrap();
    assert_eq!(v.list_transaction_splits(id).unwrap().len(), 2);
    // CodeRabbit (#1851): an empty replacement would leave a zero booking without parts
    assert!(
        v.save_manual_booking_with_splits(
            account,
            Some(id),
            setup("2026-01-01", 0, "Updated"),
            Some(Vec::new())
        )
        .is_err()
    );
    assert_eq!(v.list_transaction_splits(id).unwrap().len(), 2);
    v.save_manual_booking(account, Some(id), setup("2026-01-01", 0, "Again"))
        .unwrap();
}

#[test]
fn split_amount_edits_require_atomic_matching_replacements() {
    let v = vault("offline-split-amount");
    let account = v
        .create_offline_account("Wallet".into(), AccountKind::Cash, "EUR".into(), 0)
        .unwrap();
    let id = v
        .save_manual_booking(account, None, setup("2026-01-01", -100, "Shop"))
        .unwrap();
    let part = |uuid, amount| TransactionSplitInput {
        uuid,
        category_id: None,
        amount_cents: amount,
        note: String::new(),
    };
    v.set_transaction_splits(id, vec![part(None, -40), part(None, -60)])
        .unwrap();
    let parts = v.list_transaction_splits(id).unwrap();
    let changed = setup("2026-01-02", -200, "Changed");
    assert!(
        v.save_manual_booking(account, Some(id), changed.clone())
            .is_err()
    );
    assert!(
        v.save_manual_booking_with_splits(account, Some(id), changed.clone(), Some(vec![]))
            .is_err()
    );
    assert!(
        v.save_manual_booking_with_splits(
            account,
            Some(id),
            changed.clone(),
            Some(vec![part(None, -1), part(None, -1)])
        )
        .is_err()
    );
    assert!(
        v.save_manual_booking_with_splits(
            account,
            Some(id),
            changed.clone(),
            Some(vec![part(Some("missing".into()), -100), part(None, -100)])
        )
        .is_err()
    );
    assert_eq!(balance(&v, account), -100);
    let db = v.db().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT booking_date || ':' || counterparty FROM transactions WHERE id=?1",
            [id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "2026-01-01:Shop"
    );
    drop(db);
    assert_eq!(
        v.list_transaction_splits(id)
            .unwrap()
            .iter()
            .map(|p| &p.uuid)
            .collect::<Vec<_>>(),
        parts.iter().map(|p| &p.uuid).collect::<Vec<_>>()
    );
    v.save_manual_booking_with_splits(
        account,
        Some(id),
        changed,
        Some(vec![
            part(Some(parts[0].uuid.clone()), -80),
            part(Some(parts[1].uuid.clone()), -120),
        ]),
    )
    .unwrap();
    assert_eq!(balance(&v, account), -200);
    let after = v.list_transaction_splits(id).unwrap();
    // each kept UUID keeps the amount it was saved with, in order
    assert_eq!(
        after
            .iter()
            .map(|p| (&p.uuid, p.amount_cents))
            .collect::<Vec<_>>(),
        vec![(&parts[0].uuid, -80), (&parts[1].uuid, -120)]
    );
    v.save_manual_booking(account, Some(id), setup("2026-01-03", -200, "Same amount"))
        .unwrap();
    v.save_manual_booking_with_splits(
        account,
        Some(id),
        setup("2026-01-03", -200, "Unsplit"),
        Some(vec![]),
    )
    .unwrap();
    assert!(v.list_transaction_splits(id).unwrap().is_empty());
}

#[test]
fn new_account_starts_with_its_opening_balance() {
    let v = vault("offline-new");
    let id = v
        .create_offline_account(
            "Portemonnaie".into(),
            AccountKind::Cash,
            "EUR".into(),
            50_00,
        )
        .unwrap();
    let a = v
        .list_accounts()
        .unwrap()
        .into_iter()
        .find(|a| a.id == id)
        .unwrap();
    assert_eq!(a.name, "Portemonnaie");
    assert_eq!(a.kind, AccountKind::Cash);
    assert_eq!(a.balance_cents, 50_00);
    assert!(a.connection_id.is_none());
    assert!(a.balance_derived);
    assert_eq!(v.list_offline_account_ids().unwrap(), vec![id]);
}

#[test]
fn empty_name_becomes_cash_and_loans_are_refused() {
    let v = vault("offline-name");
    let id = v
        .create_offline_account("  ".into(), AccountKind::Cash, "EUR".into(), 0)
        .unwrap();
    assert_eq!(v.list_accounts().unwrap()[0].id, id);
    assert_eq!(v.list_accounts().unwrap()[0].name, "Bargeld");
    for kind in [AccountKind::Loan, AccountKind::Brokerage] {
        assert!(matches!(
            v.create_offline_account("X".into(), kind, "EUR".into(), 0),
            Err(CoreError::InvalidInput { .. })
        ));
    }
}

#[test]
fn bookings_move_the_balance_and_can_be_edited_and_deleted() {
    let v = vault("offline-edit");
    let acc = v
        .create_offline_account("Kasse".into(), AccountKind::Cash, "EUR".into(), 100_00)
        .unwrap();
    let a = v
        .save_manual_booking(acc, None, setup("2026-11-01", -20_00, "Bäcker"))
        .unwrap();
    let b = v
        .save_manual_booking(acc, None, setup("2026-11-02", 5_00, "Fund"))
        .unwrap();
    assert_eq!(balance(&v, acc), 85_00);

    // edit: amount, date and text
    v.save_manual_booking(acc, Some(a), setup("2026-11-03", -30_00, "Metzger"))
        .unwrap();
    assert_eq!(balance(&v, acc), 75_00);
    let t = v
        .list_transactions(crate::models::BookingFilter {
            account_ids: vec![acc],
            ..Default::default()
        })
        .unwrap()
        .transactions;
    assert!(
        t.iter()
            .any(|x| x.id == a && x.counterparty == "Metzger" && x.booking_date == "2026-11-03")
    );

    // soft delete: the row stays for the merge, nothing lists or sums it
    v.delete_manual_booking(b).unwrap();
    assert_eq!(balance(&v, acc), 70_00);
    assert_eq!(live_count(&v, acc), 1);
    let stamp: Option<i64> = v
        .db()
        .unwrap()
        .query_row(
            "SELECT deleted_ms FROM transactions WHERE id = ?1",
            [b],
            |r| r.get(0),
        )
        .unwrap();
    assert!(stamp.is_some());
}

#[test]
fn category_is_set_by_hand_and_validated() {
    let v = vault("offline-category");
    let acc = v
        .create_offline_account("Kasse".into(), AccountKind::Cash, "EUR".into(), 0)
        .unwrap();
    let cat: i64 = {
        let db = v.db().unwrap();
        db.execute("INSERT INTO categories(name) VALUES ('Essen')", [])
            .unwrap();
        db.last_insert_rowid()
    };
    let mut s = setup("2026-11-01", -9_00, "Imbiss");
    s.category_id = Some(cat);
    let id = v.save_manual_booking(acc, None, s.clone()).unwrap();
    let src: String = v
        .db()
        .unwrap()
        .query_row(
            "SELECT category_source FROM transactions WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(src, "manual");
    s.category_id = Some(cat + 999);
    assert!(matches!(
        v.save_manual_booking(acc, Some(id), s),
        Err(CoreError::NotFound { .. })
    ));
}

#[test]
fn input_is_validated() {
    let v = vault("offline-validate");
    let acc = v
        .create_offline_account("Kasse".into(), AccountKind::Cash, "EUR".into(), 0)
        .unwrap();
    for bad in [
        setup("2026-11-01", 0, "x"),
        setup("kein Datum", 1_00, "x"),
        setup("2026-02-30", 1_00, "x"),
    ] {
        assert!(matches!(
            v.save_manual_booking(acc, None, bad),
            Err(CoreError::InvalidInput { .. })
        ));
    }
    assert!(matches!(
        v.save_manual_booking(acc, None, setup("2026-11-01", 1_00, &"x".repeat(501))),
        Err(CoreError::InvalidInput { .. })
    ));
    assert_eq!(live_count(&v, acc), 0);
}

#[test]
fn synced_accounts_stay_read_only() {
    let v = vault("offline-readonly");
    let bank = {
        let db = v.db().unwrap();
        db.execute(
            "INSERT INTO account_groups(name, sf_symbol, sort_order) VALUES ('G', 'tray', 1)",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO bank_connections(connector, display_name, login_name)
             VALUES ('fints:52090000', 'VB', '1234567890')",
            [],
        )
        .unwrap();
        let conn = db.last_insert_rowid();
        db.execute(
            "INSERT INTO accounts(connection_id, group_id, external_ref, name, kind)
             VALUES (?1, (SELECT min(id) FROM account_groups), 'DE1', 'Giro', 'checking')",
            params![conn],
        )
        .unwrap();
        db.last_insert_rowid()
    };
    assert!(matches!(
        v.save_manual_booking(bank, None, setup("2026-11-01", 1_00, "x")),
        Err(CoreError::InvalidInput { .. })
    ));
    assert!(!v.list_offline_account_ids().unwrap().contains(&bank));
    assert!(matches!(
        v.save_manual_booking(9999, None, setup("2026-11-01", 1_00, "x")),
        Err(CoreError::NotFound { .. })
    ));
}

#[test]
fn an_imported_account_keeps_its_balance_when_it_takes_a_booking() {
    let v = vault("offline-imported");
    let acc = {
        let db = v.db().unwrap();
        db.execute(
            "INSERT INTO account_groups(name, sf_symbol, sort_order) VALUES ('G', 'tray', 1)",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO accounts(group_id, external_ref, name, kind, balance_cents)
             VALUES ((SELECT min(id) FROM account_groups), 'csv-1', 'Alt', 'cash', 40_00)",
            [],
        )
        .unwrap();
        db.last_insert_rowid()
    };
    assert_eq!(v.list_offline_account_ids().unwrap(), vec![acc]);
    v.save_manual_booking(acc, None, setup("2026-11-01", -10_00, "Kiosk"))
        .unwrap();
    assert_eq!(balance(&v, acc), 30_00);
}

#[test]
fn mirroring_reverses_the_amount_and_leaves_no_category() {
    let v = vault("offline-mirror");
    let cash = v
        .create_offline_account("Kasse".into(), AccountKind::Cash, "EUR".into(), 0)
        .unwrap();
    let giro = {
        let db = v.db().unwrap();
        db.execute(
            "INSERT INTO account_groups(name, sf_symbol, sort_order) VALUES ('G', 'tray', 1)",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO accounts(group_id, external_ref, name, kind) VALUES ((SELECT min(id) FROM account_groups), 'g', 'Giro', 'checking')",
            [],
        )
        .unwrap();
        let giro = db.last_insert_rowid();
        db.execute(
            "INSERT INTO transactions(account_id, external_id, booking_date, amount_cents, counterparty, category_id, category_source)
             VALUES (?1, 'atm', '2026-11-05', -80_00, 'Geldautomat', NULL, 'none')",
            [giro],
        )
        .unwrap();
        giro
    };
    // the Giro account has no connection either, so it is "offline" by the SQL rule; mirror from it
    let atm: i64 = v
        .db()
        .unwrap()
        .query_row(
            "SELECT id FROM transactions WHERE external_id = 'atm'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let copy = v.mirror_booking_to_offline_account(atm, cash).unwrap();
    assert_eq!(balance(&v, cash), 80_00);
    let (cents, cat): (i64, Option<i64>) = v
        .db()
        .unwrap()
        .query_row(
            "SELECT amount_cents, category_id FROM transactions WHERE id = ?1",
            [copy],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((cents, cat), (80_00, None));
    // the original is untouched, and a booking cannot be mirrored into its own account
    assert!(matches!(
        v.mirror_booking_to_offline_account(atm, giro),
        Err(CoreError::InvalidInput { .. })
    ));
    assert_eq!(live_count(&v, giro), 1);
}

#[test]
fn a_correction_booking_reaches_the_target_balance() {
    let v = vault("offline-correct");
    let acc = v
        .create_offline_account("Kasse".into(), AccountKind::Cash, "EUR".into(), 10_00)
        .unwrap();
    assert_eq!(v.correct_offline_balance(acc, 10_00).unwrap(), None);
    let id = v.correct_offline_balance(acc, 25_50).unwrap().unwrap();
    assert_eq!(balance(&v, acc), 25_50);
    let amount: i64 = v
        .db()
        .unwrap()
        .query_row(
            "SELECT amount_cents FROM transactions WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(amount, 15_50);
    v.correct_offline_balance(acc, -5_00).unwrap();
    assert_eq!(balance(&v, acc), -5_00);
}

#[test]
fn savings_account_in_a_foreign_currency_keeps_kind_and_currency() {
    let v = vault("offline-savings-usd");
    let id = v
        .create_offline_account(
            "Tagesgeld".into(),
            AccountKind::Savings,
            "usd".into(),
            250_00,
        )
        .unwrap();
    let a = v
        .list_accounts()
        .unwrap()
        .into_iter()
        .find(|a| a.id == id)
        .unwrap();
    assert_eq!(a.kind, AccountKind::Savings);
    assert_eq!(a.sf_symbol, "percent");
    assert_eq!(a.currency, "USD");
    assert_eq!(a.balance_cents, 250_00);
    assert_eq!(v.list_offline_account_ids().unwrap(), vec![id]);

    let b = v
        .save_manual_booking(id, None, setup("2026-11-01", -50_00, "Gebühr"))
        .unwrap();
    let cur: String = v
        .db()
        .unwrap()
        .query_row(
            "SELECT currency FROM transactions WHERE id = ?1",
            [b],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cur, "USD");
    assert_eq!(balance(&v, id), 200_00);
    v.correct_offline_balance(id, 300_00).unwrap();
    assert_eq!(balance(&v, id), 300_00);
}

#[test]
fn empty_currency_is_eur_and_invalid_codes_are_refused() {
    let v = vault("offline-currency");
    let id = v
        .create_offline_account("Kasse".into(), AccountKind::Cash, " ".into(), 0)
        .unwrap();
    let a = v
        .list_accounts()
        .unwrap()
        .into_iter()
        .find(|a| a.id == id)
        .unwrap();
    assert_eq!(a.currency, "EUR");
    for bad in ["EU", "EURO", "E1R", "€€€"] {
        assert!(
            matches!(
                v.create_offline_account("X".into(), AccountKind::Cash, bad.into(), 0),
                Err(CoreError::InvalidInput { .. })
            ),
            "{bad}"
        );
    }
}

/// #1049: sums over several accounts count only EUR; one account counts in its own currency;
/// the rows of other currencies stay in the list and show up as "other currencies".
#[test]
fn category_sums_count_only_eur_across_accounts() {
    use crate::models::{BookingDirection, BookingFilter};
    let v = vault("offline-category-currency");
    let eur = v
        .create_offline_account("Kasse".into(), AccountKind::Cash, "EUR".into(), 0)
        .unwrap();
    let usd = v
        .create_offline_account("Dollar".into(), AccountKind::Cash, "USD".into(), 0)
        .unwrap();
    let cat: i64 = {
        let db = v.db().unwrap();
        db.execute("INSERT INTO categories(name) VALUES ('Reisen')", [])
            .unwrap();
        db.last_insert_rowid()
    };
    for (acc, cents) in [(eur, -10_00), (usd, -30_00), (usd, -5_00)] {
        let mut s = setup("2026-11-02", cents, "Laden");
        s.category_id = Some(cat);
        v.save_manual_booking(acc, None, s).unwrap();
    }
    let total = |accounts: Vec<i64>| -> i64 {
        v.list_categories_filtered_amount(
            None,
            None,
            accounts,
            None,
            BookingDirection::All,
            None,
            None,
            vec![],
            false,
        )
        .unwrap()
        .into_iter()
        .find(|c| c.id == cat)
        .unwrap()
        .total_cents
    };
    assert_eq!(total(vec![]), -10_00, "all accounts: EUR only");
    assert_eq!(total(vec![eur, usd]), -10_00);
    assert_eq!(total(vec![usd]), -35_00, "one account: its own currency");

    let hist = |accounts: Vec<i64>| -> i64 {
        v.category_history(cat, accounts, None, None)
            .unwrap()
            .iter()
            .map(|p| p.cents)
            .sum()
    };
    assert_eq!(hist(vec![]), -10_00);
    assert_eq!(hist(vec![usd]), -35_00);

    let other = v
        .category_other_currencies(cat, vec![], None, None)
        .unwrap();
    assert_eq!(other.len(), 1);
    assert_eq!(
        (other[0].currency.as_str(), other[0].cents),
        ("USD", -35_00)
    );
    assert!(
        v.category_other_currencies(cat, vec![usd], None, None)
            .unwrap()
            .is_empty()
    );

    let flow = v.selection_flow(vec![], None, None).unwrap();
    assert_eq!(flow.consumption_cents, 10_00);
    let flow_usd = v.selection_flow(vec![usd], None, None).unwrap();
    assert_eq!(
        flow_usd.consumption_cents, 35_00,
        "one account: its own currency"
    );

    // "Zusätzlich" next to the sidebar totals and the KPI tiles, rolled up to the parent category
    let child: i64 = {
        let db = v.db().unwrap();
        db.execute(
            "INSERT INTO categories(name, parent_id) VALUES ('Flug', ?1)",
            [cat],
        )
        .unwrap();
        db.last_insert_rowid()
    };
    let mut s = setup("2026-11-03", -20_00, "Airline");
    s.category_id = Some(child);
    v.save_manual_booking(usd, None, s).unwrap();
    let other_totals = |accounts: Vec<i64>| {
        v.category_other_currency_totals(
            None,
            None,
            accounts,
            None,
            BookingDirection::All,
            None,
            None,
            vec![],
            false,
        )
        .unwrap()
    };
    let rows = other_totals(vec![]);
    let of = |id: i64| {
        rows.iter()
            .find(|r| r.category_id == id)
            .map(|r| (r.currency.clone(), r.cents))
    };
    assert_eq!(of(child), Some(("USD".into(), -20_00)));
    assert_eq!(
        of(cat),
        Some(("USD".into(), -55_00)),
        "parent rolls up its child"
    );
    assert!(other_totals(vec![usd]).is_empty());

    let kpi = v.flow_other_currencies(BookingFilter::default()).unwrap();
    assert_eq!(kpi.len(), 1);
    assert_eq!(
        (kpi[0].currency.as_str(), kpi[0].consumption_cents),
        ("USD", 55_00)
    );
    assert!(
        v.flow_other_currencies(BookingFilter {
            account_ids: vec![usd],
            ..Default::default()
        })
        .unwrap()
        .is_empty()
    );

    // budgets and the cashflow chart: EUR only as well
    v.set_category_budget(cat, Some(100_00)).unwrap();
    let budget = v.list_category_budget_progress(2026, 11).unwrap();
    assert_eq!(
        budget
            .iter()
            .find(|b| b.category_id == cat)
            .unwrap()
            .spent_cents,
        10_00
    );
    let flows = crate::planning::cashflow_for(
        &v,
        crate::models::PeriodGranularity::Month,
        3,
        &[],
        2026,
        11,
        false,
    )
    .unwrap();
    assert_eq!(
        flows.last().unwrap().expense_cents,
        10_00,
        "cashflow: EUR only"
    );
    let flows_usd = crate::planning::cashflow_for(
        &v,
        crate::models::PeriodGranularity::Month,
        3,
        &[usd],
        2026,
        11,
        false,
    )
    .unwrap();
    assert_eq!(
        flows_usd.last().unwrap().expense_cents,
        55_00,
        "one account: its own currency"
    );

    let page = v
        .list_transactions(BookingFilter {
            ..Default::default()
        })
        .unwrap();
    assert_eq!(page.transactions.len(), 4, "rows of every currency stay");
    assert_eq!(page.expense_cents, -10_00, "page sums: EUR only");
}
