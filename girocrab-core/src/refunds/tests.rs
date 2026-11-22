use super::*;
use crate::{models::BookingFilter, test_support::TempDir};

fn open(dir: &TempDir, name: &str) -> std::sync::Arc<Vault> {
    Vault::open(dir.join(name).to_string_lossy().into(), vec![7; 32]).unwrap()
}
fn originals(v: &Vault, id: i64) -> Vec<i64> {
    v.refund_links(id)
        .unwrap()
        .originals
        .iter()
        .map(|o| o.id)
        .collect()
}
fn seed(v: &Vault) -> Vec<i64> {
    let db = v.db().unwrap();
    db.execute_batch(
        "INSERT INTO account_groups(name) VALUES ('G');
 INSERT INTO accounts(group_id,external_ref,name) VALUES (1,'A','Konto');",
    )
    .unwrap();
    [(-100,"2025-01-01","Laden",""),(-200,"2025-01-31","Anderer","Suche"),(50,"2025-02-01","Laden",""),(25,"2025-02-02","Anderer",""),(0,"2025-02-01","",""),(-1,"2023-01-01","Laden",""),(12,"2026-02-02","Laden","")]
 .into_iter().enumerate().map(|(i,(amount,date,cp,purpose))|{
 db.execute("INSERT INTO transactions(account_id,external_id,booking_date,amount_cents,counterparty,purpose) VALUES (1,?1,?2,?3,?4,?5)",params![format!("T{i}"),date,amount,cp,purpose]).unwrap();
 db.last_insert_rowid()
 }).collect()
}

#[test]
fn validation_replace_unlink_and_markers() {
    let d = TempDir::new("refunds");
    let v = open(&d, "v");
    let b = seed(&v);
    for (r, o) in [
        (b[0], b[2]),
        (b[2], b[2]),
        (b[4], b[0]),
        (999, b[0]),
        (b[2], 999),
    ] {
        assert!(v.link_refund(r, o, 0).is_err());
    }
    v.link_refund(b[2], b[0], 0).unwrap();
    assert_eq!(originals(&v, b[2]), vec![b[0]]);
    assert_eq!(v.refund_links(b[0]).unwrap().refunds[0].id, b[2]);
    let list = v
        .list_transactions(BookingFilter::default())
        .unwrap()
        .transactions;
    assert_eq!(
        list.iter().find(|t| t.id == b[2]).unwrap().refund_link,
        RefundLinkKind::Refund
    );
    assert_eq!(
        list.iter().find(|t| t.id == b[0]).unwrap().refund_link,
        RefundLinkKind::Original
    );
    assert_eq!(
        list.iter().find(|t| t.id == b[1]).unwrap().refund_link,
        RefundLinkKind::None
    );
    // a refund that is used up takes no further original
    assert_eq!(
        v.link_refund(b[2], b[1], 0).unwrap_err().to_string(),
        "Ungültige Eingabe: Die Erstattung ist bereits vollständig zugeordnet."
    );
    assert_eq!(v.refund_links(b[2]).unwrap().refund_open_cents, 0);
    v.db()
        .unwrap()
        .execute(
            "UPDATE transactions SET amount_cents=-20 WHERE id=?1",
            [b[0]],
        )
        .unwrap();
    // nor does an original that is paid back in full take a further refund
    assert_eq!(
        v.link_refund(b[3], b[0], 0).unwrap_err().to_string(),
        "Ungültige Eingabe: Die Buchung ist bereits vollständig erstattet."
    );
    assert_eq!(v.refund_links(b[2]).unwrap().refund_open_cents, 30);
    // a second original is added, the first link stays
    v.link_refund(b[2], b[1], 0).unwrap();
    assert_eq!(originals(&v, b[2]), vec![b[0], b[1]]);
    v.unlink_refund(b[2], b[0], 0).unwrap();
    assert!(v.refund_links(b[0]).unwrap().refunds.is_empty());
    assert_eq!(originals(&v, b[2]), vec![b[1]]);
    v.unlink_refund(b[2], b[1], 0).unwrap();
    v.unlink_refund(999, b[1], 0).unwrap();
    assert!(originals(&v, b[2]).is_empty());
    v.db()
        .unwrap()
        .execute("UPDATE transactions SET deleted_ms=1 WHERE id=?1", [b[0]])
        .unwrap();
    assert!(v.link_refund(b[2], b[0], 0).is_err());
    assert!(v.refund_links(b[0]).is_err());
}

#[test]
fn cascade_and_no_chains() {
    let d = TempDir::new("refunds");
    let v = open(&d, "v");
    let b = seed(&v);
    v.link_refund(b[2], b[0], 0).unwrap();
    v.db()
        .unwrap()
        .execute(
            "UPDATE transactions SET amount_cents=-50 WHERE id=?1",
            [b[2]],
        )
        .unwrap();
    assert!(v.link_refund(b[3], b[2], 0).is_err());
    v.db()
        .unwrap()
        .execute(
            "UPDATE transactions SET amount_cents=50 WHERE id=?1",
            [b[2]],
        )
        .unwrap();
    v.db()
        .unwrap()
        .execute(
            "UPDATE transactions SET amount_cents=100 WHERE id=?1",
            [b[0]],
        )
        .unwrap();
    assert!(v.link_refund(b[0], b[1], 0).is_err());
    v.db()
        .unwrap()
        .execute("DELETE FROM transactions WHERE id=?1", [b[0]])
        .unwrap();
    // the inspector joins the bookings and would hide a row left behind
    let stored = |id: i64| -> i64 {
        v.db()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM refund_links WHERE refund_id=?1 OR original_id=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_eq!(stored(b[0]), 0);
    assert!(originals(&v, b[2]).is_empty());
    v.link_refund(b[2], b[1], 0).unwrap();
    v.db()
        .unwrap()
        .execute("DELETE FROM transactions WHERE id=?1", [b[2]])
        .unwrap();
    assert_eq!(stored(b[2]), 0);
    assert!(v.refund_links(b[1]).unwrap().refunds.is_empty());
}

#[test]
fn candidates_sign_window_order_filter_conflicts() {
    let d = TempDir::new("refunds");
    let v = open(&d, "v");
    let b = seed(&v);
    let ids = |rows: Vec<RefundBooking>| rows.into_iter().map(|b| b.id).collect::<Vec<_>>();
    assert_eq!(
        ids(v.refund_candidates(b[2], String::new(), 20).unwrap()),
        vec![b[1], b[0]]
    );
    assert_eq!(
        ids(v.refund_candidates(b[2], "sUcHe".into(), 20).unwrap()),
        vec![b[1]]
    );
    assert_eq!(
        ids(v.refund_candidates(b[2], "lAdEn".into(), 1).unwrap()),
        vec![b[0]]
    );
    assert!(
        v.refund_candidates(b[2], "missing".into(), 20)
            .unwrap()
            .is_empty()
    );
    assert!(
        v.refund_candidates(b[4], String::new(), 20)
            .unwrap()
            .is_empty()
    );
    assert!(
        v.refund_candidates(b[0], String::new(), 0)
            .unwrap()
            .is_empty()
    );
    assert!(v.refund_candidates(999, String::new(), 20).is_err());
    assert_eq!(
        ids(v.refund_candidates(b[0], String::new(), 20).unwrap()),
        vec![b[2], b[3]]
    );
    // a refund of another booking can also pay back this one
    v.link_refund(b[2], b[1], 0).unwrap();
    assert_eq!(
        ids(v.refund_candidates(b[0], String::new(), 20).unwrap()),
        vec![b[2], b[3]]
    );
    assert!(ids(v.refund_candidates(b[1], String::new(), 20).unwrap()).contains(&b[2]));
    v.db()
        .unwrap()
        .execute("UPDATE transactions SET deleted_ms=1 WHERE id=?1", [b[3]])
        .unwrap();
    assert_eq!(
        ids(v.refund_candidates(b[0], String::new(), 20).unwrap()),
        vec![b[2]]
    );
}

#[test]
fn merge_maps_links_and_newer_relink_wins() {
    let d = TempDir::new("refunds");
    let a = open(&d, "a");
    let b = open(&d, "b");
    let aa = seed(&a);
    let bb = seed(&b);
    b.link_refund(bb[2], bb[0], 0).unwrap();
    a.merge_other_vault(b.file_path(), vec![7; 32]).unwrap();
    assert_eq!(originals(&a, aa[2]), vec![aa[0]]);
    a.unlink_refund(aa[2], aa[0], 0).unwrap();
    a.link_refund(aa[2], aa[1], 0).unwrap();
    a.db()
        .unwrap()
        .execute(
            "UPDATE refund_links SET changed_ms=changed_ms+100 WHERE refund_id=?1",
            [aa[2]],
        )
        .unwrap();
    a.merge_other_vault(b.file_path(), vec![7; 32]).unwrap();
    assert_eq!(originals(&a, aa[2]), vec![aa[1]]);
}

#[test]
fn unlink_survives_older_peer_and_relink_revives() {
    let d = TempDir::new("refunds-sync");
    let a = open(&d, "a");
    let b = open(&d, "b");
    let aa = seed(&a);
    let bb = seed(&b);
    a.link_refund(aa[2], aa[0], 0).unwrap();
    b.merge_other_vault(a.file_path(), vec![7; 32]).unwrap();
    a.unlink_refund(aa[2], aa[0], 0).unwrap();
    a.merge_other_vault(b.file_path(), vec![7; 32]).unwrap();
    assert!(originals(&a, aa[2]).is_empty());
    assert!(a.refund_links(aa[0]).unwrap().refunds.is_empty());
    assert!(!markers(&a.db().unwrap()).unwrap().contains_key(&aa[2]));
    assert!(
        a.refund_candidates(aa[1], String::new(), 20)
            .unwrap()
            .iter()
            .any(|r| r.id == aa[2])
    );
    b.merge_other_vault(a.file_path(), vec![7; 32]).unwrap();
    assert!(originals(&b, bb[2]).is_empty());
    b.link_refund(bb[2], bb[1], 0).unwrap();
    a.merge_other_vault(b.file_path(), vec![7; 32]).unwrap();
    assert_eq!(originals(&a, aa[2]), vec![aa[1]]);
}

/// An unlink from the other device reaches the link of a booking that is deleted here, so
/// the booking does not come back with the link.
#[test]
fn unlink_reaches_the_link_of_a_deleted_booking() {
    let d = TempDir::new("refunds-sync-deleted");
    let a = open(&d, "a");
    let b = open(&d, "b");
    let aa = seed(&a);
    let bb = seed(&b);
    a.link_refund(aa[2], aa[0], 0).unwrap();
    b.merge_other_vault(a.file_path(), vec![7; 32]).unwrap();
    assert_eq!(originals(&b, bb[2]), vec![bb[0]]);
    a.unlink_refund(aa[2], aa[0], 0).unwrap();
    let deleted = |v: &Vault, id: i64, ms: Option<i64>| {
        v.db()
            .unwrap()
            .execute(
                "UPDATE transactions SET deleted_ms=?2 WHERE id=?1",
                rusqlite::params![id, ms],
            )
            .unwrap();
    };
    deleted(&b, bb[0], Some(i64::MAX / 2));
    b.merge_other_vault(a.file_path(), vec![7; 32]).unwrap();
    deleted(&b, bb[0], None);
    assert!(originals(&b, bb[2]).is_empty());
}

#[test]
fn equal_stamp_merges_agree_in_both_orders() {
    for deleted in [false, true] {
        for reverse in [false, true] {
            let d = TempDir::new("refunds-ties");
            let a = open(&d, "a");
            let b = open(&d, "b");
            let aa = seed(&a);
            let mut bb = seed(&b);
            b.db()
                .unwrap()
                .execute("UPDATE transactions SET id=id+100", [])
                .unwrap();
            for id in &mut bb {
                *id += 100;
            }
            a.link_refund(aa[2], aa[0], 0).unwrap();
            b.link_refund(bb[2], bb[1], 0).unwrap();
            if deleted {
                a.unlink_refund(aa[2], aa[0], 0).unwrap();
            }
            for v in [&a, &b] {
                v.db().unwrap().execute("UPDATE refund_links SET created_ms=100,changed_ms=200,deleted_ms=CASE WHEN deleted_ms IS NULL THEN NULL ELSE 200 END",[]).unwrap();
            }
            if reverse {
                b.merge_other_vault(a.file_path(), vec![7; 32]).unwrap();
                a.merge_other_vault(b.file_path(), vec![7; 32]).unwrap();
            } else {
                a.merge_other_vault(b.file_path(), vec![7; 32]).unwrap();
                b.merge_other_vault(a.file_path(), vec![7; 32]).unwrap();
            }
            // different pairs: both links survive, a removed one stays removed
            let original = |v: &Vault, id| -> Vec<String> {
                v.refund_links(id)
                    .unwrap()
                    .originals
                    .into_iter()
                    .map(|o| o.booking_date)
                    .collect()
            };
            let expected: Vec<String> = if deleted {
                vec!["2025-01-31".into()]
            } else {
                vec!["2025-01-01".into(), "2025-01-31".into()]
            };
            assert_eq!(original(&a, aa[2]), expected);
            assert_eq!(original(&b, bb[2]), expected);
        }
    }
}

#[test]
fn cross_account_currency_and_inclusive_window() {
    let d = TempDir::new("refunds");
    let v = open(&d, "v");
    let b = seed(&v);
    {
        let db = v.db().unwrap();
        db.execute_batch(
            "INSERT INTO accounts(group_id,external_ref,name) VALUES (1,'B','Zweites Konto');",
        )
        .unwrap();
        db.execute("UPDATE transactions SET account_id=2,currency='USD',booking_date='2026-01-01' WHERE id=?1", [b[3]]).unwrap();
    }
    assert!(
        v.refund_candidates(b[0], String::new(), 20)
            .unwrap()
            .iter()
            .any(|r| r.id == b[3])
    );
    v.link_refund(b[2], b[0], 0).unwrap();
    v.link_refund(b[3], b[0], 0).unwrap();
    let links = v.refund_links(b[0]).unwrap();
    assert_eq!(links.refunds.len(), 2);
    let usd = links.refunds.iter().find(|r| r.id == b[3]).unwrap();
    assert_eq!(usd.currency, "USD");
    assert_eq!(usd.account_name, "Zweites Konto");
    assert_eq!(usd.amount_cents, 25);
    v.db()
        .unwrap()
        .execute(
            "UPDATE transactions SET booking_date='2026-01-02' WHERE id=?1",
            [b[3]],
        )
        .unwrap();
    assert!(
        !v.refund_candidates(b[0], String::new(), 20)
            .unwrap()
            .iter()
            .any(|r| r.id == b[3])
    );
}

#[test]
fn repeated_unlink_keeps_the_stamp_and_rows_carry_texts() {
    let d = TempDir::new("refunds");
    let v = open(&d, "v");
    let b = seed(&v);
    v.link_refund(b[2], b[1], 0).unwrap();
    let original = v.refund_links(b[2]).unwrap().originals.remove(0);
    assert_eq!(original.counterparty, "Anderer");
    assert_eq!(original.purpose, "Suche");
    v.unlink_refund(b[2], b[1], 0).unwrap();
    let stamp = |v: &Vault| -> i64 {
        v.db()
            .unwrap()
            .query_row(
                "SELECT changed_ms FROM refund_links WHERE refund_id=?1",
                [b[2]],
                |r| r.get(0),
            )
            .unwrap()
    };
    let first = stamp(&v);
    v.unlink_refund(b[2], b[1], 0).unwrap();
    assert_eq!(stamp(&v), first);
}

#[test]
fn error_texts_have_english() {
    for de in [
        "Die Buchung existiert nicht oder wurde gelöscht.",
        "Eine Erstattung muss positiv und die ursprüngliche Buchung negativ sein.",
        "Erstattungen dürfen nicht miteinander verkettet werden.",
    ] {
        assert_ne!(crate::i18n::tr_in(crate::i18n::Lang::En, de), de);
    }
}

fn add(v: &Vault, amount: i64, date: &str, cp: &str, purpose: &str) -> i64 {
    let db = v.db().unwrap();
    db.execute(
        "INSERT INTO transactions(account_id,external_id,booking_date,amount_cents,counterparty,purpose) VALUES (1,?1,?2,?3,?4,?5)",
        params![format!("X{date}{amount}{cp}{purpose}"), date, amount, cp, purpose],
    )
    .unwrap();
    db.last_insert_rowid()
}

#[test]
fn suggestions_fill_amount_then_merchant_then_reference() {
    let d = TempDir::new("refunds");
    let v = open(&d, "v");
    seed(&v);
    let refund = add(
        &v,
        4_990,
        "2025-06-20",
        "Versandhaus",
        "Gutschrift RE-2025-7781",
    );
    let far_same_amount = add(&v, -4_990, "2025-01-10", "Fremd", "");
    let near_same_amount = add(&v, -4_990, "2025-06-01", "Fremd", "");
    let vendor_recent = add(&v, -1_200, "2025-05-15", "versandhaus ", "");
    let vendor_old = add(&v, -1_300, "2025-03-01", "Versandhaus", "");
    let reference = add(
        &v,
        -9_900,
        "2025-02-01",
        "Zahlstelle",
        "Rechnung re-2025-7781 Danke",
    );
    add(&v, -4_990, "2025-06-21", "Fremd", ""); // after the refund
    let got = |id| -> Vec<(i64, RefundMatch)> {
        v.refund_suggestions(id)
            .unwrap()
            .into_iter()
            .map(|s| (s.booking.id, s.reason))
            .collect()
    };
    assert_eq!(
        got(refund),
        vec![
            (near_same_amount, RefundMatch::SameAmount),
            (far_same_amount, RefundMatch::SameAmount),
            (vendor_recent, RefundMatch::SameMerchant),
        ]
    );
    // fewer amount matches: the merchant (two months only), then the reference fill up
    v.db()
        .unwrap()
        .execute(
            "UPDATE transactions SET amount_cents=-4991 WHERE id IN (?1,?2)",
            [far_same_amount, near_same_amount],
        )
        .unwrap();
    assert_eq!(
        got(refund),
        vec![
            (vendor_recent, RefundMatch::SameMerchant),
            (reference, RefundMatch::SameReference),
        ]
    );
    assert!(!got(refund).iter().any(|(id, _)| *id == vendor_old));
    // from the outgoing side: the refund is suggested for its original; linked ones are not
    assert_eq!(got(reference), vec![(refund, RefundMatch::SameReference)]);
    v.link_refund(refund, reference, 0).unwrap();
    assert!(got(reference).is_empty());
    // the refund is used up by that link: it takes no further booking, so none is suggested
    assert!(got(refund).is_empty());
    assert!(v.refund_suggestions(999).is_err());
}

#[test]
fn split_original_keeps_its_parts_and_takes_partial_refunds() {
    use crate::models::TransactionSplitInput;
    let d = TempDir::new("refunds");
    let v = open(&d, "v");
    seed(&v);
    let original = add(&v, -10_000, "2025-06-01", "Versandhaus", "");
    let first = add(&v, 3_000, "2025-06-10", "Versandhaus", "");
    let second = add(&v, 2_000, "2025-06-12", "Versandhaus", "");
    let parts = |a: i64, b: i64| {
        vec![
            TransactionSplitInput {
                category_id: None,
                amount_cents: a,
                note: "a".into(),
            },
            TransactionSplitInput {
                category_id: None,
                amount_cents: b,
                note: "b".into(),
            },
        ]
    };
    let split = |v: &Vault| -> Vec<i64> {
        v.list_transaction_splits(original)
            .unwrap()
            .iter()
            .map(|s| s.amount_cents)
            .collect()
    };
    // split first, link afterwards
    v.set_transaction_splits(original, parts(-7_000, -3_000))
        .unwrap();
    // the picker offers the parts, not the split booking itself
    let offered: Vec<(u32, i64, String)> = v
        .refund_candidates(first, String::new(), 20)
        .unwrap()
        .into_iter()
        .filter(|b| b.id == original)
        .map(|b| (b.part, b.amount_cents, b.part_label))
        .collect();
    assert_eq!(
        offered,
        vec![(1, -7_000, "a".to_string()), (2, -3_000, "b".to_string())]
    );
    assert!(v.link_refund(first, original, 0).is_err());
    assert!(v.link_refund(first, original, 3).is_err());
    // the refund pays back the chosen part only: at most that part's amount
    v.link_refund(first, original, 2).unwrap();
    assert!(v.link_refund(second, original, 2).is_err());
    assert!(
        v.refund_candidates(second, String::new(), 20)
            .unwrap()
            .iter()
            .any(|b| b.id == original && b.part == 2 && b.open_cents == 0)
    );
    v.link_refund(second, original, 1).unwrap();
    assert_eq!(split(&v), vec![-7_000, -3_000]);
    let links = v.refund_links(original).unwrap();
    assert_eq!(
        links
            .refunds
            .iter()
            .map(|r| (r.id, r.part, r.part_label.as_str(), r.share_cents))
            .collect::<Vec<_>>(),
        vec![(first, 2, "b", Some(3_000)), (second, 1, "a", Some(2_000))]
    );
    assert_eq!(
        (links.refunded_cents, links.original_open_cents),
        (Some(5_000), 5_000)
    );
    let own = &v.refund_links(first).unwrap().originals[0];
    assert_eq!(
        (own.amount_cents, own.part, own.share_cents, own.open_cents),
        (-3_000, 2, Some(3_000), 0)
    );
    // one refund for two parts of the same booking, removed again part by part
    let third = add(&v, 6_000, "2025-06-14", "Versandhaus", "");
    v.link_refund(third, original, 1).unwrap();
    assert_eq!(
        v.refund_links(third).unwrap().originals[0].share_cents,
        Some(5_000)
    );
    v.unlink_refund(third, original, 1).unwrap();
    assert!(v.refund_links(third).unwrap().originals.is_empty());
    // split again while linked, then remove the split: the links stay
    v.set_transaction_splits(original, parts(-5_000, -5_000))
        .unwrap();
    assert_eq!(v.refund_links(original).unwrap().refunds.len(), 2);
    v.set_transaction_splits(original, Vec::new()).unwrap();
    assert_eq!(v.refund_links(original).unwrap().refunds.len(), 2);
    // without the parts the links count for the whole booking
    assert_eq!(
        v.refund_links(original).unwrap().refunded_cents,
        Some(5_000)
    );
    assert_eq!(
        v.refund_links(first).unwrap().originals[0].amount_cents,
        -10_000
    );
    let row = v
        .list_transactions(BookingFilter::default())
        .unwrap()
        .transactions
        .into_iter()
        .find(|t| t.id == original)
        .unwrap();
    assert_eq!(row.refund_link, RefundLinkKind::Original);
    assert_eq!(row.amount_cents, -10_000);
}

#[test]
fn linked_refund_is_no_income_in_the_analysis() {
    let d = TempDir::new("refunds");
    let v = open(&d, "v");
    seed(&v);
    let original = add(&v, -10_000, "2025-06-01", "Versandhaus", "");
    let refund = add(&v, 3_000, "2025-06-10", "Versandhaus", "");
    let page = |v: &Vault| v.list_transactions(BookingFilter::default()).unwrap();
    let before = page(&v);
    v.link_refund(refund, original, 0).unwrap();
    let after = page(&v);
    assert_eq!(after.income_cents, before.income_cents - 3_000);
    assert_eq!(after.expense_cents, before.expense_cents + 3_000);
    assert_eq!(after.flow.income_cents, before.flow.income_cents - 3_000);
    assert_eq!(
        format!("{:?}", v.selection_flow(vec![], None, None).unwrap()),
        format!("{:?}", after.flow)
    );
    v.unlink_refund(refund, original, 0).unwrap();
    assert_eq!(format!("{:?}", page(&v).flow), format!("{:?}", before.flow));
}

#[test]
fn only_the_assigned_part_of_a_refund_reduces_the_expenses() {
    let d = TempDir::new("refunds");
    let v = open(&d, "v");
    seed(&v);
    let small = add(&v, -2_000, "2025-06-01", "Versandhaus", "a");
    let other = add(&v, -4_000, "2025-06-02", "Versandhaus", "b");
    let refund = add(&v, 5_000, "2025-06-10", "Versandhaus", "");
    let page = |v: &Vault| v.list_transactions(BookingFilter::default()).unwrap();
    let row = |v: &Vault, id: i64| {
        let t = page(v).transactions.into_iter().find(|t| t.id == id);
        t.map(|t| (t.refund_part_cents, t.refund_income_cents))
            .unwrap()
    };
    let before = page(&v);
    v.link_refund(refund, small, 0).unwrap();
    // 20 of the 50 are assigned: they reduce the expenses, 30 stay income
    let part = page(&v);
    assert_eq!(part.income_cents, before.income_cents - 2_000);
    assert_eq!(part.expense_cents, before.expense_cents + 2_000);
    assert_eq!(part.flow.income_cents, before.flow.income_cents - 2_000);
    assert_eq!(
        part.flow.consumption_cents,
        before.flow.consumption_cents - 2_000
    );
    assert_eq!(
        format!("{:?}", v.selection_flow(vec![], None, None).unwrap()),
        format!("{:?}", part.flow)
    );
    assert_eq!(row(&v, refund), (Some(2_000), 3_000));
    // the original is paid back in full: just the marker
    assert_eq!(row(&v, small), (None, 0));
    v.link_refund(refund, other, 0).unwrap();
    let full = page(&v);
    assert_eq!(full.income_cents, before.income_cents - 5_000);
    assert_eq!(full.expense_cents, before.expense_cents + 5_000);
    assert_eq!(row(&v, refund), (None, 0));
    assert_eq!(row(&v, other), (Some(3_000), 0));
}

#[test]
fn refund_larger_than_its_original_is_spread_over_several() {
    let d = TempDir::new("refunds");
    let v = open(&d, "v");
    seed(&v);
    let first = add(&v, -2_000, "2025-06-01", "Versandhaus", "a");
    let second = add(&v, -4_000, "2025-06-02", "Versandhaus", "b");
    let refund = add(&v, 5_000, "2025-06-10", "Versandhaus", "");
    let later = add(&v, 3_000, "2025-06-12", "Versandhaus", "");
    v.link_refund(refund, first, 0).unwrap();
    // never more than the booking's amount; the rest of the refund is open
    assert_eq!(v.refund_links(first).unwrap().refunded_cents, Some(2_000));
    assert_eq!(v.refund_links(refund).unwrap().assigned_cents, Some(2_000));
    // the rest goes to the second original
    v.link_refund(refund, second, 0).unwrap();
    let own = v.refund_links(refund).unwrap();
    assert_eq!(own.assigned_cents, Some(5_000));
    // each booking takes at most its amount, the next one at most the rest
    assert_eq!(
        own.originals
            .iter()
            .map(|o| o.share_cents)
            .collect::<Vec<_>>(),
        vec![Some(2_000), Some(3_000)]
    );
    assert_eq!(v.refund_links(second).unwrap().refunded_cents, Some(3_000));
    assert_eq!(
        v.refund_links(second).unwrap().refunds[0].share_cents,
        Some(3_000)
    );
    // filled in the order the links were made, not by date: linked first, the later booking
    // gets its full amount and the earlier one the rest
    v.unlink_refund(refund, first, 0).unwrap();
    v.unlink_refund(refund, second, 0).unwrap();
    v.link_refund(refund, second, 0).unwrap();
    v.link_refund(refund, first, 0).unwrap();
    let own = v.refund_links(refund).unwrap();
    assert_eq!(
        own.originals
            .iter()
            .map(|o| (o.id, o.share_cents))
            .collect::<Vec<_>>(),
        vec![(second, Some(4_000)), (first, Some(1_000))]
    );
    assert_eq!(own.refund_open_cents, 0);
    // used up: no suggestion or link for it any more, and the candidates say so
    let third = add(&v, -700, "2025-06-03", "Versandhaus", "c");
    assert!(v.link_refund(refund, third, 0).is_err());
    assert!(
        v.refund_suggestions(third)
            .unwrap()
            .iter()
            .all(|s| s.booking.id != refund)
    );
    let candidates = v.refund_candidates(third, String::new(), 50).unwrap();
    assert_eq!(
        candidates
            .iter()
            .find(|c| c.id == refund)
            .map(|c| c.open_cents),
        Some(0)
    );
    v.unlink_refund(refund, first, 0).unwrap();
    v.unlink_refund(refund, second, 0).unwrap();
    v.link_refund(refund, first, 0).unwrap();
    v.link_refund(refund, second, 0).unwrap();
    // a later refund only fills what is still open
    v.link_refund(later, second, 0).unwrap();
    assert_eq!(v.refund_links(second).unwrap().refunded_cents, Some(4_000));
    assert_eq!(v.refund_links(later).unwrap().assigned_cents, Some(1_000));
    assert_eq!(
        v.refund_candidates(first, String::new(), 50).unwrap()[0].share_cents,
        None
    );
    // a linked refund stays a candidate for further originals, a pair links once
    assert!(
        v.refund_candidates(first, String::new(), 50)
            .unwrap()
            .iter()
            .any(|b| b.id == later)
    );
    v.link_refund(refund, second, 0).unwrap();
    assert_eq!(v.refund_links(second).unwrap().refunds.len(), 2);
    // another currency is never added up (#569)
    v.db()
        .unwrap()
        .execute(
            "UPDATE transactions SET currency='USD' WHERE id=?1",
            [later],
        )
        .unwrap();
    assert_eq!(v.refund_links(second).unwrap().refunded_cents, None);
    assert_eq!(v.refund_links(first).unwrap().refunded_cents, Some(2_000));
    assert_eq!(v.refund_links(later).unwrap().assigned_cents, None);
}

#[test]
fn links_work_with_own_categories_from_both_sides() {
    let d = TempDir::new("refunds");
    let v = open(&d, "v");
    seed(&v);
    let own = |name: &str| {
        v.create_category(name.into(), None, "tag".into(), "#abcdef".into())
            .unwrap()
    };
    let (garden, tools) = (own("Garten"), own("Werkzeug"));
    let original = add(&v, -10_000, "2025-06-01", "Baumarkt", "");
    let plain = add(&v, -2_000, "2025-06-02", "Baumarkt", "");
    let refund = add(&v, 5_000, "2025-06-10", "Baumarkt", "");
    v.set_transaction_category(plain, Some(garden)).unwrap();
    v.set_transaction_category(refund, Some(tools)).unwrap();
    let part = |category: i64, amount: i64| crate::models::TransactionSplitInput {
        category_id: Some(category),
        amount_cents: amount,
        note: String::new(),
    };
    v.set_transaction_splits(original, vec![part(garden, -6_000), part(tools, -4_000)])
        .unwrap();
    // from the refund: the parts carry the names of the own categories
    let offered: Vec<(i64, u32, String)> = v
        .refund_candidates(refund, String::new(), 20)
        .unwrap()
        .into_iter()
        .filter(|b| b.id == original || b.id == plain)
        .map(|b| (b.id, b.part, b.part_label))
        .collect();
    assert_eq!(
        offered,
        vec![
            (plain, 0, String::new()),
            (original, 1, "Garten".to_string()),
            (original, 2, "Werkzeug".to_string()),
        ]
    );
    v.link_refund(refund, plain, 0).unwrap();
    // from the outgoing booking: the refund is offered and linked to the chosen part
    assert!(
        v.refund_candidates(original, String::new(), 20)
            .unwrap()
            .iter()
            .any(|b| b.id == refund && b.open_cents == 3_000)
    );
    v.link_refund(refund, original, 2).unwrap();
    let links = v.refund_links(refund).unwrap();
    assert_eq!(
        links
            .originals
            .iter()
            .map(|o| (o.id, o.part, o.share_cents))
            .collect::<Vec<_>>(),
        vec![(plain, 0, Some(2_000)), (original, 2, Some(3_000))]
    );
    assert_eq!(links.assigned_cents, Some(5_000));
}

fn part_links(v: &Vault, refund: i64) -> Vec<(i64, u32, Option<i64>)> {
    v.refund_links(refund)
        .unwrap()
        .originals
        .iter()
        .map(|o| (o.id, o.part, o.share_cents))
        .collect()
}

/// A link that names a part the booking does not have (the other device split it differently)
/// counts for the whole booking, spread from its last part to its first.
#[test]
fn link_to_a_part_that_is_gone_is_spread_over_the_booking() {
    let d = TempDir::new("refunds");
    let v = open(&d, "v");
    seed(&v);
    let own = |name: &str| {
        v.create_category(name.into(), None, "tag".into(), "#abcdef".into())
            .unwrap()
    };
    let (garden, tools) = (own("Garten"), own("Werkzeug"));
    let original = add(&v, -10_000, "2025-06-01", "Baumarkt", "");
    let refund = add(&v, 5_000, "2025-06-10", "Baumarkt", "");
    let part = |category: i64, amount: i64| crate::models::TransactionSplitInput {
        category_id: Some(category),
        amount_cents: amount,
        note: String::new(),
    };
    v.set_transaction_splits(original, vec![part(garden, -6_000), part(tools, -4_000)])
        .unwrap();
    v.link_refund(refund, original, 2).unwrap();
    assert_eq!(part_links(&v, refund), vec![(original, 2, Some(4_000))]);
    v.db()
        .unwrap()
        .execute(
            "UPDATE refund_links SET original_part=3 WHERE refund_id=?1",
            [refund],
        )
        .unwrap();
    assert_eq!(part_links(&v, refund), vec![(original, 3, Some(5_000))]);
    let expenses = |category: i64| {
        v.list_category_expenses(None, None, Vec::new())
            .unwrap()
            .into_iter()
            .find(|c| c.id == category)
            .unwrap()
            .own_total_cents
    };
    assert_eq!((expenses(garden), expenses(tools)), (-5_000, 0));
}

/// Removing a split on one device reaches the other as a link to the whole booking, and
/// the link to the part does not come back from there.
#[test]
fn removed_split_reaches_the_other_device_as_a_link_to_the_whole_booking() {
    let d = TempDir::new("refunds-split-sync");
    let a = open(&d, "a");
    let b = open(&d, "b");
    let aa = seed(&a);
    let bb = seed(&b);
    let part = |amount: i64| crate::models::TransactionSplitInput {
        category_id: None,
        amount_cents: amount,
        note: String::new(),
    };
    a.set_transaction_splits(aa[1], vec![part(-150), part(-50)])
        .unwrap();
    a.link_refund(aa[2], aa[1], 2).unwrap();
    b.merge_other_vault(a.file_path(), vec![7; 32]).unwrap();
    assert_eq!(part_links(&b, bb[2]), vec![(bb[1], 2, Some(50))]);

    a.set_transaction_splits(aa[1], Vec::new()).unwrap();
    assert_eq!(part_links(&a, aa[2]), vec![(aa[1], 0, Some(50))]);
    b.merge_other_vault(a.file_path(), vec![7; 32]).unwrap();
    assert_eq!(part_links(&b, bb[2]), vec![(bb[1], 0, Some(50))]);
    a.merge_other_vault(b.file_path(), vec![7; 32]).unwrap();
    assert_eq!(part_links(&a, aa[2]), vec![(aa[1], 0, Some(50))]);
}

/// A split that was synced before step 38 could lie in two orders (the merge wrote it sorted,
/// the device it was made on kept the order it was entered in). The step gives both the same
/// order, so a link to a part names the same part on both devices after the merge.
#[test]
fn split_synced_before_the_step_has_one_order_on_both_devices() {
    let d = TempDir::new("refunds-split-order");
    let a = open(&d, "a");
    let b = open(&d, "b");
    let aa = seed(&a);
    let bb = seed(&b);
    let part = |amount: i64, note: &str| crate::models::TransactionSplitInput {
        category_id: None,
        amount_cents: amount,
        note: note.into(),
    };
    // as entered on A, and as the merge of before wrote it on B
    a.set_transaction_splits(aa[1], vec![part(-50, "small"), part(-150, "big")])
        .unwrap();
    b.set_transaction_splits(bb[1], vec![part(-150, "big"), part(-50, "small")])
        .unwrap();
    for v in [&a, &b] {
        let db = v.db().unwrap();
        db.execute("UPDATE transactions SET splits_ms=5 WHERE splits_ms>0", [])
            .unwrap();
        db.execute_batch(super::SCHEMA).unwrap();
    }
    b.link_refund(bb[2], bb[1], 1).unwrap();
    a.merge_other_vault(b.file_path(), vec![7; 32]).unwrap();
    b.merge_other_vault(a.file_path(), vec![7; 32]).unwrap();
    let label = |v: &Vault, refund: i64| {
        v.refund_links(refund).unwrap().originals[0]
            .part_label
            .clone()
    };
    assert_eq!(label(&b, bb[2]), "big");
    assert_eq!(label(&a, aa[2]), "big");
}

/// The part of a split refund a link names reaches the other device with the link.
#[test]
fn link_of_a_refund_part_reaches_the_other_device() {
    let d = TempDir::new("refunds-part-sync");
    let a = open(&d, "a");
    let b = open(&d, "b");
    let aa = seed(&a);
    let bb = seed(&b);
    let part = |amount: i64| crate::models::TransactionSplitInput {
        category_id: None,
        amount_cents: amount,
        note: String::new(),
    };
    let own = |v: &Vault, refund: i64| -> Vec<(i64, u32, Option<i64>)> {
        v.refund_links(refund)
            .unwrap()
            .originals
            .iter()
            .map(|o| (o.id, o.refund_part, o.share_cents))
            .collect()
    };
    // the smaller part comes second: the other device must count the parts in this order too
    a.set_transaction_splits(aa[2], vec![part(30), part(20)])
        .unwrap();
    a.link_refund_parts(aa[2], 2, aa[0], 0).unwrap();
    assert_eq!(own(&a, aa[2]), vec![(aa[0], 2, Some(20))]);
    b.merge_other_vault(a.file_path(), vec![7; 32]).unwrap();
    assert_eq!(
        b.list_transaction_splits(bb[2])
            .unwrap()
            .iter()
            .map(|s| s.amount_cents)
            .collect::<Vec<_>>(),
        vec![30, 20]
    );
    assert_eq!(own(&b, bb[2]), vec![(bb[0], 2, Some(20))]);
    b.unlink_refund_parts(bb[2], 2, bb[0], 0).unwrap();
    a.merge_other_vault(b.file_path(), vec![7; 32]).unwrap();
    assert!(own(&a, aa[2]).is_empty());
}

/// A link whose booking is on the wrong side now (its amount was changed elsewhere) pays
/// nothing back and marks no booking; it can still be removed.
#[test]
fn link_of_a_booking_on_the_wrong_side_counts_for_nothing() {
    let d = TempDir::new("refunds");
    let v = open(&d, "v");
    let ids = seed(&v);
    v.link_refund(ids[2], ids[0], 0).unwrap();
    assert_eq!(part_links(&v, ids[2]), vec![(ids[0], 0, Some(50))]);
    v.db()
        .unwrap()
        .execute(
            "UPDATE transactions SET amount_cents=100 WHERE id=?1",
            [ids[0]],
        )
        .unwrap();
    assert_eq!(part_links(&v, ids[2]), vec![(ids[0], 0, None)]);
    assert!(markers(&v.db().unwrap()).unwrap().is_empty());
    v.unlink_refund(ids[2], ids[0], 0).unwrap();
    assert!(part_links(&v, ids[2]).is_empty());
}

/// Two links of one refund made in the same millisecond on two devices are filled in the same
/// order on both, whatever ids the bookings have there.
#[test]
fn links_with_equal_stamps_are_filled_alike_on_both_devices() {
    let d = TempDir::new("refunds-tie");
    let a = open(&d, "a");
    let b = open(&d, "b");
    for v in [&a, &b] {
        v.db()
            .unwrap()
            .execute_batch(
                "INSERT INTO account_groups(name) VALUES ('G');
                 INSERT INTO accounts(group_id,external_ref,name) VALUES (1,'A','Konto');",
            )
            .unwrap();
    }
    // the same bookings, stored in another order: other local ids
    let insert = |v: &Vault, key: &str, amount: i64| -> i64 {
        let db = v.db().unwrap();
        db.execute(
            "INSERT INTO transactions(account_id,external_id,booking_date,amount_cents,counterparty,purpose) VALUES (1,?1,'2025-03-01',?2,'Laden','')",
            params![key, amount],
        )
        .unwrap();
        db.last_insert_rowid()
    };
    let on_a: Vec<i64> = [("R", 100), ("T1", -60), ("T2", -60)]
        .map(|(key, amount)| insert(&a, key, amount))
        .to_vec();
    let on_b: Vec<i64> = [("T2", -60), ("T1", -60), ("R", 100)]
        .map(|(key, amount)| insert(&b, key, amount))
        .to_vec();
    let (r_a, t1_a, t2_a) = (on_a[0], on_a[1], on_a[2]);
    let (t2_b, t1_b, r_b) = (on_b[0], on_b[1], on_b[2]);
    a.link_refund(r_a, t1_a, 0).unwrap();
    b.link_refund(r_b, t2_b, 0).unwrap();
    for v in [&a, &b] {
        v.db()
            .unwrap()
            .execute("UPDATE refund_links SET created_ms=5,changed_ms=5", [])
            .unwrap();
    }
    a.merge_other_vault(b.file_path(), vec![7; 32]).unwrap();
    b.merge_other_vault(a.file_path(), vec![7; 32]).unwrap();
    let share = |v: &Vault, refund: i64, original: i64| {
        v.refund_links(refund)
            .unwrap()
            .originals
            .iter()
            .find(|o| o.id == original)
            .and_then(|o| o.share_cents)
    };
    // both links are there on both devices and filled alike
    assert_eq!(
        (share(&a, r_a, t1_a), share(&a, r_a, t2_a)),
        (Some(60), Some(40))
    );
    assert_eq!(
        (share(&b, r_b, t1_b), share(&b, r_b, t2_b)),
        (Some(60), Some(40))
    );
}

/// The same with originals of two accounts that carry the same date and bank id (ids are
/// unique within an account only): the account decides, not the local id.
#[test]
fn links_with_equal_stamps_and_twin_bookings_of_two_accounts_are_filled_alike() {
    let d = TempDir::new("refunds-tie-accounts");
    let a = open(&d, "a");
    let b = open(&d, "b");
    for v in [&a, &b] {
        v.db()
            .unwrap()
            .execute_batch(
                "INSERT INTO account_groups(name) VALUES ('G');
                 INSERT INTO accounts(group_id,external_ref,name) VALUES (1,'A','Konto');
                 INSERT INTO accounts(group_id,external_ref,name) VALUES (1,'B','Zweites Konto');",
            )
            .unwrap();
    }
    let insert = |v: &Vault, account: i64, key: &str, amount: i64| -> i64 {
        let db = v.db().unwrap();
        db.execute(
            "INSERT INTO transactions(account_id,external_id,booking_date,amount_cents,counterparty,purpose) VALUES (?1,?2,'2025-03-01',?3,'Laden','')",
            params![account, key, amount],
        )
        .unwrap();
        db.last_insert_rowid()
    };
    let on_a: Vec<i64> = [(1, "R", 100), (1, "T", -60), (2, "T", -60)]
        .map(|(account, key, amount)| insert(&a, account, key, amount))
        .to_vec();
    let on_b: Vec<i64> = [(2, "T", -60), (1, "T", -60), (1, "R", 100)]
        .map(|(account, key, amount)| insert(&b, account, key, amount))
        .to_vec();
    let (r_a, t1_a, t2_a) = (on_a[0], on_a[1], on_a[2]);
    let (t2_b, t1_b, r_b) = (on_b[0], on_b[1], on_b[2]);
    a.link_refund(r_a, t1_a, 0).unwrap();
    b.link_refund(r_b, t2_b, 0).unwrap();
    for v in [&a, &b] {
        v.db()
            .unwrap()
            .execute("UPDATE refund_links SET created_ms=5,changed_ms=5", [])
            .unwrap();
    }
    a.merge_other_vault(b.file_path(), vec![7; 32]).unwrap();
    b.merge_other_vault(a.file_path(), vec![7; 32]).unwrap();
    let share = |v: &Vault, refund: i64, original: i64| {
        v.refund_links(refund)
            .unwrap()
            .originals
            .iter()
            .find(|o| o.id == original)
            .and_then(|o| o.share_cents)
    };
    // both links are there on both devices and filled alike
    assert_eq!(
        (share(&a, r_a, t1_a), share(&a, r_a, t2_a)),
        (Some(60), Some(40))
    );
    assert_eq!(
        (share(&b, r_b, t1_b), share(&b, r_b, t2_b)),
        (Some(60), Some(40))
    );
}

/// Links to two parts of one booking that end up as one link to the whole booking keep the
/// place of the earlier one, also when the later part was the first to lose its part.
#[test]
fn links_of_two_parts_that_become_one_keep_the_earliest_place() {
    let d = TempDir::new("refunds-collapse");
    let v = open(&d, "v");
    seed(&v);
    let first = add(&v, -200, "2025-06-01", "Baumarkt", "");
    let second = add(&v, -150, "2025-06-02", "Baumarkt", "");
    let refund = add(&v, 270, "2025-06-10", "Baumarkt", "");
    let part = |amount: i64| crate::models::TransactionSplitInput {
        category_id: None,
        amount_cents: amount,
        note: String::new(),
    };
    v.set_transaction_splits(first, vec![part(-100), part(-100)])
        .unwrap();
    v.link_refund(refund, first, 1).unwrap();
    v.link_refund(refund, second, 0).unwrap();
    v.link_refund(refund, first, 2).unwrap();
    let share = |original: i64| -> i64 {
        part_links(&v, refund)
            .iter()
            .filter(|l| l.0 == original)
            .map(|l| l.2.unwrap_or(0))
            .sum()
    };
    assert_eq!((share(first), share(second)), (120, 150));
    // part 2 is an incoming part now: its link is one to the whole booking, made last
    v.set_transaction_splits(first, vec![part(-250), part(50)])
        .unwrap();
    // without the split the link of part 1 joins it, and the booking keeps its first place
    v.set_transaction_splits(first, Vec::new()).unwrap();
    assert_eq!(part_links(&v, refund).len(), 2);
    assert_eq!((share(first), share(second)), (200, 70));
}

/// The inspector lists the links of a booking in the order they are filled, also when their
/// stamps are equal and the order comes from the bookings.
#[test]
fn links_with_equal_stamps_are_listed_in_the_order_they_are_filled() {
    let d = TempDir::new("refunds-listed");
    let v = open(&d, "v");
    let b = seed(&v);
    v.link_refund(b[2], b[5], 0).unwrap();
    v.link_refund(b[2], b[0], 0).unwrap();
    v.db()
        .unwrap()
        .execute("UPDATE refund_links SET created_ms=5,changed_ms=5", [])
        .unwrap();
    // the earlier booking is filled first, though it has the higher id
    let listed: Vec<(i64, Option<i64>)> = v
        .refund_links(b[2])
        .unwrap()
        .originals
        .iter()
        .map(|o| (o.id, o.share_cents))
        .collect();
    assert_eq!(listed, vec![(b[5], Some(1)), (b[0], Some(49))]);
}

/// Each device removes the split of one end of a link between two parts while offline: both
/// rewrite the link, under two keys. The merge leaves one link of the pair on both devices.
#[test]
fn split_removed_on_both_ends_on_two_devices_leaves_one_link() {
    let d = TempDir::new("refunds-both-splits");
    let a = open(&d, "a");
    let b = open(&d, "b");
    let aa = seed(&a);
    let bb = seed(&b);
    let part = |amount: i64| crate::models::TransactionSplitInput {
        category_id: None,
        amount_cents: amount,
        note: String::new(),
    };
    a.set_transaction_splits(aa[1], vec![part(-150), part(-50)])
        .unwrap();
    a.set_transaction_splits(aa[2], vec![part(30), part(20)])
        .unwrap();
    a.link_refund_parts(aa[2], 2, aa[1], 2).unwrap();
    b.merge_other_vault(a.file_path(), vec![7; 32]).unwrap();
    a.set_transaction_splits(aa[2], Vec::new()).unwrap();
    b.set_transaction_splits(bb[1], Vec::new()).unwrap();
    a.merge_other_vault(b.file_path(), vec![7; 32]).unwrap();
    b.merge_other_vault(a.file_path(), vec![7; 32]).unwrap();
    a.merge_other_vault(b.file_path(), vec![7; 32]).unwrap();
    let live = |v: &Vault| -> Vec<(i64, i64)> {
        v.db()
            .unwrap()
            .prepare("SELECT original_part,refund_part FROM refund_links WHERE deleted_ms IS NULL")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(live(&a), vec![(0, 0)]);
    assert_eq!(live(&b), vec![(0, 0)]);
    assert_eq!(part_links(&a, aa[2]), vec![(aa[1], 0, Some(50))]);
    assert_eq!(part_links(&b, bb[2]), vec![(bb[1], 0, Some(50))]);
    b.unlink_refund(bb[2], bb[1], 0).unwrap();
    a.merge_other_vault(b.file_path(), vec![7; 32]).unwrap();
    assert!(originals(&a, aa[2]).is_empty());
}
