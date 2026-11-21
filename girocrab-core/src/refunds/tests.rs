use super::*;
use crate::{models::BookingFilter, test_support::TempDir};

fn open(dir: &TempDir, name: &str) -> std::sync::Arc<Vault> {
    Vault::open(dir.join(name).to_string_lossy().into(), vec![7; 32]).unwrap()
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
        assert!(v.link_refund(r, o).is_err());
    }
    v.link_refund(b[2], b[0]).unwrap();
    assert_eq!(v.refund_links(b[2]).unwrap().original.unwrap().id, b[0]);
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
    v.link_refund(b[2], b[1]).unwrap();
    assert!(v.refund_links(b[0]).unwrap().refunds.is_empty());
    v.unlink_refund(b[2]).unwrap();
    v.unlink_refund(999).unwrap();
    assert!(v.refund_links(b[2]).unwrap().original.is_none());
    v.db()
        .unwrap()
        .execute("UPDATE transactions SET deleted_ms=1 WHERE id=?1", [b[0]])
        .unwrap();
    assert!(v.link_refund(b[2], b[0]).is_err());
    assert!(v.refund_links(b[0]).is_err());
}

#[test]
fn cascade_and_no_chains() {
    let d = TempDir::new("refunds");
    let v = open(&d, "v");
    let b = seed(&v);
    v.link_refund(b[2], b[0]).unwrap();
    v.db()
        .unwrap()
        .execute(
            "UPDATE transactions SET amount_cents=-50 WHERE id=?1",
            [b[2]],
        )
        .unwrap();
    assert!(v.link_refund(b[3], b[2]).is_err());
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
    assert!(v.link_refund(b[0], b[1]).is_err());
    v.db()
        .unwrap()
        .execute("DELETE FROM transactions WHERE id=?1", [b[0]])
        .unwrap();
    assert!(v.refund_links(b[2]).unwrap().original.is_none());
    v.link_refund(b[2], b[1]).unwrap();
    v.db()
        .unwrap()
        .execute("DELETE FROM transactions WHERE id=?1", [b[2]])
        .unwrap();
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
        vec![b[0], b[1]]
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
    v.link_refund(b[2], b[1]).unwrap();
    assert_eq!(
        ids(v.refund_candidates(b[0], String::new(), 20).unwrap()),
        vec![b[3]]
    );
    assert!(ids(v.refund_candidates(b[1], String::new(), 20).unwrap()).contains(&b[2]));
    v.db()
        .unwrap()
        .execute("UPDATE transactions SET deleted_ms=1 WHERE id=?1", [b[3]])
        .unwrap();
    assert!(
        v.refund_candidates(b[0], String::new(), 20)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn merge_maps_links_and_newer_relink_wins() {
    let d = TempDir::new("refunds");
    let a = open(&d, "a");
    let b = open(&d, "b");
    let aa = seed(&a);
    let bb = seed(&b);
    b.link_refund(bb[2], bb[0]).unwrap();
    a.merge_other_vault(b.file_path(), vec![7; 32]).unwrap();
    assert_eq!(a.refund_links(aa[2]).unwrap().original.unwrap().id, aa[0]);
    a.link_refund(aa[2], aa[1]).unwrap();
    a.db()
        .unwrap()
        .execute(
            "UPDATE refund_links SET changed_ms=changed_ms+100 WHERE refund_id=?1",
            [aa[2]],
        )
        .unwrap();
    a.merge_other_vault(b.file_path(), vec![7; 32]).unwrap();
    assert_eq!(a.refund_links(aa[2]).unwrap().original.unwrap().id, aa[1]);
}

#[test]
fn unlink_survives_older_peer_and_relink_revives() {
    let d = TempDir::new("refunds-sync");
    let a = open(&d, "a");
    let b = open(&d, "b");
    let aa = seed(&a);
    let bb = seed(&b);
    a.link_refund(aa[2], aa[0]).unwrap();
    b.merge_other_vault(a.file_path(), vec![7; 32]).unwrap();
    a.unlink_refund(aa[2]).unwrap();
    a.merge_other_vault(b.file_path(), vec![7; 32]).unwrap();
    assert!(a.refund_links(aa[2]).unwrap().original.is_none());
    assert!(a.refund_links(aa[0]).unwrap().refunds.is_empty());
    assert!(!markers(&a.db().unwrap()).unwrap().contains_key(&aa[2]));
    assert!(
        a.refund_candidates(aa[1], String::new(), 20)
            .unwrap()
            .iter()
            .any(|r| r.id == aa[2])
    );
    b.merge_other_vault(a.file_path(), vec![7; 32]).unwrap();
    assert!(b.refund_links(bb[2]).unwrap().original.is_none());
    b.link_refund(bb[2], bb[1]).unwrap();
    a.merge_other_vault(b.file_path(), vec![7; 32]).unwrap();
    assert_eq!(a.refund_links(aa[2]).unwrap().original.unwrap().id, aa[1]);
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
            a.link_refund(aa[2], aa[0]).unwrap();
            b.link_refund(bb[2], bb[1]).unwrap();
            if deleted {
                a.unlink_refund(aa[2]).unwrap();
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
            let original =
                |v: &Vault, id| v.refund_links(id).unwrap().original.map(|o| o.booking_date);
            let expected = if deleted {
                None
            } else {
                Some("2025-01-31".into())
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
    v.link_refund(b[2], b[0]).unwrap();
    v.link_refund(b[3], b[0]).unwrap();
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
