mod review_regressions {
    #[test]
    fn manual_unit_ties_and_amount_only_edits_converge() {
        for a_first in [true, false] {
            let (a, account, o, _) = seed();
            let b = vault();
            merge(&b, &a);
            let bo = peer_id(&b, &a, o);
            a.save_manual_booking_with_splits(
                account,
                Some(o),
                setup(-6000),
                Some(vec![part(None, -3000), part(None, -3000)]),
            )
            .unwrap();
            let ba = number(
                &b,
                &format!("SELECT account_id FROM transactions WHERE id={bo}"),
            );
            b.save_manual_booking_with_splits(
                ba,
                Some(bo),
                setup(-7000),
                Some(vec![part(None, -3000), part(None, -4000)]),
            )
            .unwrap();
            sql(&a, "UPDATE transactions SET splits_ms=10000000000000");
            sql(&b, "UPDATE transactions SET splits_ms=10000000000000");
            if a_first {
                merge(&a, &b);
                merge(&b, &a);
            } else {
                merge(&b, &a);
                merge(&a, &b);
            }
            assert_eq!(splits(&a, o), splits(&b, bo));
            for (v, id) in [(&a, o), (&b, bo)] {
                assert_eq!(
                    number(
                        v,
                        &format!("SELECT amount_cents FROM transactions WHERE id={id}")
                    ),
                    -6000
                );
                assert_eq!(splits(v, id).iter().map(|p| p.1).sum::<i64>(), -6000);
            }
            a.set_transaction_splits(o, vec![]).unwrap();
            a.save_manual_booking(account, Some(o), setup(-8000))
                .unwrap();
            merge(&b, &a);
            merge(&a, &b);
            assert!(splits(&b, bo).is_empty());
            assert_eq!(
                number(
                    &b,
                    &format!("SELECT amount_cents FROM transactions WHERE id={bo}")
                ),
                -8000
            );
        }
    }

    #[test]
    fn hand_over_retains_part_keys_and_resolves_survivor_parts_conservatively() {
        for amounts in [
            vec![-3000, -2000],
            vec![-1000, -2000, -2000],
            vec![-2500, -2500],
        ] {
            let (v, account, o, r) = seed();
            v.save_manual_booking(account, Some(r), setup(2000))
                .unwrap();
            v.set_transaction_splits(o, vec![part(None, -3000), part(None, -2000)])
                .unwrap();
            let old = splits(&v, o)[1].0.clone();
            v.link_refund(r, String::new(), o, old.clone()).unwrap();
            let survivor = v.save_manual_booking(account, None, setup(-5000)).unwrap();
            v.set_transaction_splits(survivor, amounts.iter().map(|a| part(None, *a)).collect())
                .unwrap();
            crate::importers::portal_import::hand_over(&v.db().unwrap(), o, survivor).unwrap();
            let links = v.refund_links(r).unwrap();
            assert_eq!(links[0].original_id, survivor);
            assert_eq!(links[0].original_part, old);
            assert_eq!(
                links[0].allocated_cents,
                if amounts == [-3000, -2000] { 2000 } else { 0 }
            );
            assert_eq!(links[0].inactive, amounts != [-3000, -2000]);
            let token = v
                .unlink_refund(r, String::new(), survivor, old.clone())
                .unwrap();
            let restored = v.restore_refund(r, String::new(), survivor, old, token);
            assert_eq!(restored.is_ok(), amounts == [-3000, -2000]);
        }
    }

    #[test]
    fn hand_over_collisions_keep_the_complete_lww_winner() {
        let (v, account, o, r) = seed();
        let survivor = v.save_manual_booking(account, None, setup(-5000)).unwrap();
        v.link_refund(r, String::new(), o, String::new()).unwrap();
        v.unlink_refund(r, String::new(), o, String::new()).unwrap();
        v.link_refund(r, String::new(), survivor, String::new())
            .unwrap();
        let created = v.refund_links(r).unwrap()[0].created_ms;
        crate::importers::portal_import::hand_over(&v.db().unwrap(), o, survivor).unwrap();
        assert_eq!(number(&v, "SELECT COUNT(*) FROM refund_links"), 1);
        let links = v.refund_links(r).unwrap();
        assert_eq!(links[0].created_ms, created);
        assert_eq!(links[0].allocated_cents, 5000);
    }
    use std::sync::Arc;

    use crate::models::{AccountKind, TransactionSplitInput};
    use crate::offline::ManualBookingSetup;
    use crate::vault::Vault;

    fn vault() -> Arc<Vault> {
        let path = crate::test_support::thread_temp_path(&format!(
            "refund-review-{}",
            uuid::Uuid::new_v4()
        ));
        Vault::open(path.to_string_lossy().into(), vec![7; 32]).unwrap()
    }

    fn setup(amount_cents: i64) -> ManualBookingSetup {
        ManualBookingSetup {
            booking_date: "2026-01-01".into(),
            amount_cents,
            counterparty: "Review".into(),
            purpose: String::new(),
            category_id: None,
            note: String::new(),
        }
    }

    fn part(uuid: Option<String>, amount_cents: i64) -> TransactionSplitInput {
        TransactionSplitInput {
            uuid,
            amount_cents,
            category_id: None,
            note: String::new(),
        }
    }

    fn seed() -> (Arc<Vault>, i64, i64, i64) {
        let v = vault();
        let account = v
            .create_offline_account("Review".into(), AccountKind::Cash, "EUR".into(), 0)
            .unwrap();
        let o = v.save_manual_booking(account, None, setup(-5000)).unwrap();
        let r = v.save_manual_booking(account, None, setup(5000)).unwrap();
        (v, account, o, r)
    }

    fn merge(into: &Vault, from: &Vault) {
        into.merge_other_vault(from.file_path(), vec![7; 32])
            .unwrap();
    }

    fn peer_id(peer: &Vault, source: &Vault, id: i64) -> i64 {
        let ext: String = source
            .db()
            .unwrap()
            .query_row(
                "SELECT external_id FROM transactions WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        peer.db()
            .unwrap()
            .query_row(
                "SELECT id FROM transactions WHERE external_id=?1",
                [ext],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn sql(v: &Vault, sql: &str) {
        v.db().unwrap().execute_batch(sql).unwrap();
    }
    fn number(v: &Vault, sql: &str) -> i64 {
        v.db().unwrap().query_row(sql, [], |r| r.get(0)).unwrap()
    }
    fn splits(v: &Vault, id: i64) -> Vec<(String, i64)> {
        v.list_transaction_splits(id)
            .unwrap()
            .into_iter()
            .map(|s| (s.uuid, s.amount_cents))
            .collect()
    }

    #[test]
    fn t1_hand_over_preserves_refund_allocation() {
        let (v, account, o, r) = seed();
        let o2 = v.save_manual_booking(account, None, setup(-5000)).unwrap();
        v.link_refund(r, "".into(), o, "".into()).unwrap();
        crate::importers::portal_import::hand_over(&v.db().unwrap(), o, o2).unwrap();
        let links = v.refund_links(r).unwrap();
        assert_eq!(
            links.iter().map(|l| l.allocated_cents).sum::<i64>(),
            5000,
            "hand-over must preserve the refund allocation on the surviving booking; links={links:?}"
        );
    }

    #[test]
    fn t1b_synced_twin_take_over_preserves_refund_allocation() {
        let (v, account, o, r) = seed();
        let twin = v.save_manual_booking(account, None, setup(-5000)).unwrap();
        sql(
            &v,
            &format!("UPDATE transactions SET external_id='mm-review-twin' WHERE id={twin}"),
        );
        sql(
            &v,
            &format!(
                "UPDATE transactions SET category_source='none', checked=0, note='' WHERE id={o}"
            ),
        );
        v.link_refund(r, "".into(), o, "".into()).unwrap();
        let merged = crate::importers::moneymoney_import::merge_synced_twin(
            &v.db().unwrap(),
            o,
            "2026-01-01",
            -5000,
            "",
            "Review",
            "",
        )
        .unwrap();
        assert!(
            merged,
            "fixture must execute the synced-twin take-over path"
        );
        let links = v.refund_links(r).unwrap();
        assert_eq!(
            links.iter().map(|l| l.allocated_cents).sum::<i64>(),
            5000,
            "synced-twin take-over must preserve the refund allocation on its survivor; links={links:?}"
        );
    }

    #[test]
    fn t2_later_unlink_wins_over_local_split_retarget_in_both_orders() {
        let mut failures = Vec::new();
        for a_first in [true, false] {
            let (a, account, o, r) = seed();
            a.save_manual_booking_with_splits(account, Some(r), setup(2000), None)
                .unwrap();
            a.set_transaction_splits(o, vec![part(None, -3000), part(None, -2000)])
                .unwrap();
            let old = splits(&a, o);
            a.link_refund(r, "".into(), o, old[1].0.clone()).unwrap();
            let b = vault();
            merge(&b, &a);
            let bo = peer_id(&b, &a, o);
            let br = peer_id(&b, &a, r);
            a.set_transaction_splits(
                o,
                vec![part(Some(old[0].0.clone()), -3000), part(None, -2000)],
            )
            .unwrap();
            b.unlink_refund(br, "".into(), bo, old[1].0.clone())
                .unwrap();
            let later = number(&a, "SELECT MAX(changed_ms)+1000 FROM refund_links");
            sql(
                &b,
                &format!("UPDATE refund_links SET changed_ms={later},deleted_ms={later}"),
            );
            if a_first {
                merge(&a, &b);
                merge(&b, &a);
            } else {
                merge(&b, &a);
                merge(&a, &b);
            }
            let al = a.refund_links(r).unwrap();
            let bl = b.refund_links(br).unwrap();
            if !al.is_empty() || !bl.is_empty() {
                failures.push(format!("a_first={a_first}: A={al:?}; B={bl:?}"));
            }
        }
        assert!(
            failures.is_empty(),
            "later explicit unlink must win and both peers must have no live allocation: {failures:#?}"
        );
    }

    #[test]
    fn t3_manual_amount_and_split_identities_converge() {
        for a_first in [true, false] {
            let (a, account, o, _) = seed();
            a.set_transaction_splits(o, vec![part(None, -3000), part(None, -2000)])
                .unwrap();
            let b = vault();
            merge(&b, &a);
            let bo = peer_id(&b, &a, o);
            let old = splits(&a, o);
            a.save_manual_booking_with_splits(
                account,
                Some(o),
                setup(-6000),
                Some(vec![part(Some(old[0].0.clone()), -3000), part(None, -3000)]),
            )
            .unwrap();
            if a_first {
                merge(&a, &b);
                merge(&b, &a);
            } else {
                merge(&b, &a);
                merge(&a, &b);
            }
            let ap = splits(&a, o);
            let bp = splits(&b, bo);
            let aa = number(
                &a,
                &format!("SELECT amount_cents FROM transactions WHERE id={o}"),
            );
            let ba = number(
                &b,
                &format!("SELECT amount_cents FROM transactions WHERE id={bo}"),
            );
            assert!(
                ap == bp
                    && ap.iter().map(|p| p.1).sum::<i64>() == aa
                    && bp.iter().map(|p| p.1).sum::<i64>() == ba,
                "manual amount/splits must converge with valid sums: A amount={aa}, splits={ap:?}; B amount={ba}, splits={bp:?}"
            );
        }
    }

    #[test]
    fn t4_same_key_merge_keeps_complete_lww_winner_in_both_orders() {
        let mut actual = Vec::new();
        for a_first in [true, false] {
            let (a, _, o, r) = seed();
            let b = vault();
            merge(&b, &a);
            let bo = peer_id(&b, &a, o);
            let br = peer_id(&b, &a, r);
            a.link_refund(r, "".into(), o, "".into()).unwrap();
            b.link_refund(br, "".into(), bo, "".into()).unwrap();
            sql(&a, "UPDATE refund_links SET created_ms=100,changed_ms=100");
            sql(&b, "UPDATE refund_links SET created_ms=200,changed_ms=200");
            if a_first {
                merge(&a, &b);
                merge(&b, &a);
            } else {
                merge(&b, &a);
                merge(&a, &b);
            }
            actual.push((
                a_first,
                number(&a, "SELECT created_ms FROM refund_links"),
                number(&b, "SELECT created_ms FROM refund_links"),
            ));
        }
        assert!(
            actual.iter().all(|(_, a, b)| *a == 200 && *b == 200),
            "same-key merge must preserve the complete LWW winner: (a_first,A,B)={actual:?}"
        );
    }

    #[test]
    fn t5_account_delete_removes_live_refund_allocation() {
        let (v, account, o, _) = seed();
        let refund_account = v
            .create_offline_account("Refund".into(), AccountKind::Cash, "EUR".into(), 0)
            .unwrap();
        let r = v
            .save_manual_booking(refund_account, None, setup(5000))
            .unwrap();
        v.link_refund(r, "".into(), o, "".into()).unwrap();
        v.set_account_archived(account, true).unwrap();
        v.delete_account(account).unwrap();
        assert!(
            v.refund_links(r)
                .unwrap()
                .iter()
                .all(|l| l.inactive && l.allocated_cents == 0),
            "account deletion must remove live allocations to its bookings"
        );
        assert_eq!(
            number(
                &v,
                "SELECT COUNT(*) FROM refund_links WHERE deleted_ms IS NOT NULL"
            ),
            0,
            "account deletion must leave link rows unchanged"
        );
    }
}

use std::sync::Arc;

use rusqlite::params;

use super::*;
use crate::models::{AccountKind, BookingFilter, TransactionSplitInput};
use crate::offline::ManualBookingSetup;

struct Fixture {
    v: Arc<Vault>,
    account: i64,
}

impl Fixture {
    fn new() -> Self {
        let path =
            crate::test_support::thread_temp_path(&format!("refunds-{}", uuid::Uuid::new_v4()));
        let v = Vault::open(path.to_string_lossy().into(), vec![7; 32]).unwrap();
        let account = v
            .create_offline_account("Wallet".into(), AccountKind::Cash, "EUR".into(), 0)
            .unwrap();
        Self { v, account }
    }
    fn add(&self, external: &str, amount: i64) -> i64 {
        let db = self.v.db().unwrap();
        db.execute("INSERT INTO transactions(account_id,external_id,booking_date,amount_cents,currency,counterparty) VALUES (?1,?2,'2026-01-01',?3,'EUR',?2)",params![self.account,external,amount]).unwrap();
        db.last_insert_rowid()
    }
    fn split(&self, id: i64, amounts: &[i64]) -> Vec<String> {
        self.v
            .set_transaction_splits(id, amounts.iter().map(|a| part(None, *a)).collect())
            .unwrap();
        self.v
            .list_transaction_splits(id)
            .unwrap()
            .into_iter()
            .map(|p| p.uuid)
            .collect()
    }
    fn link(&self, r: i64, rp: &str, o: i64, op: &str) {
        self.v.link_refund(r, rp.into(), o, op.into()).unwrap();
    }
    fn unlink(&self, r: i64, rp: &str, o: i64, op: &str) -> i64 {
        self.v.unlink_refund(r, rp.into(), o, op.into()).unwrap()
    }
    fn change(&self, id: i64, amount: i64, parts: Option<Vec<TransactionSplitInput>>) {
        self.v
            .save_manual_booking_with_splits(
                self.account,
                Some(id),
                ManualBookingSetup {
                    booking_date: "2026-01-01".into(),
                    amount_cents: amount,
                    counterparty: "Edited".into(),
                    purpose: String::new(),
                    category_id: None,
                    note: String::new(),
                },
                parts,
            )
            .unwrap();
    }
}

fn part(uuid: Option<String>, amount_cents: i64) -> TransactionSplitInput {
    TransactionSplitInput {
        uuid,
        category_id: None,
        amount_cents,
        note: String::new(),
    }
}
fn merge(into: &Vault, from: &Vault) {
    into.merge_other_vault(from.file_path(), vec![7; 32])
        .unwrap();
}
fn shares(v: &Vault, id: i64) -> Vec<i64> {
    v.refund_links(id)
        .unwrap()
        .iter()
        .map(|l| l.allocated_cents)
        .collect()
}
fn raw(v: &Vault, sql: &str) {
    v.db().unwrap().execute_batch(sql).unwrap();
}
fn number(v: &Vault, sql: &str) -> i64 {
    v.db().unwrap().query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn partial_and_multi_endpoint_allocation_follows_creation_not_change_stamp() {
    let f = Fixture::new();
    let (r, o1, o2, r2) = (
        f.add("r", 150),
        f.add("o1", -100),
        f.add("o2", -100),
        f.add("r2", 80),
    );
    assert!(f.v.refund_links(r).unwrap().is_empty());
    f.link(r, "", o1, "");
    f.link(r, "", o2, "");
    f.link(r2, "", o2, "");
    assert_eq!(shares(&f.v, r), [100, 50]);
    assert_eq!(shares(&f.v, r2), [50]);
    let before = f.v.refund_links(r).unwrap();
    raw(
        &f.v,
        &format!("UPDATE refund_links SET changed_ms=changed_ms+100 WHERE original_id={o1}"),
    );
    f.link(r, "", o1, "");
    assert_eq!(shares(&f.v, r), [100, 50]);
    let after = f.v.refund_links(r).unwrap();
    assert_eq!(
        before.iter().map(|l| l.created_ms).collect::<Vec<_>>(),
        after.iter().map(|l| l.created_ms).collect::<Vec<_>>()
    );
    assert!(f.v.link_refund(r2, "".into(), o1, "".into()).is_err());
}

#[test]
fn uuid_parts_mixed_roles_zero_total_and_booking_markers_leave_splits_untouched() {
    let f = Fixture::new();
    let (mixed, expense, refund) = (
        f.add("mixed", 0),
        f.add("expense", -100),
        f.add("refund", 100),
    );
    let p = f.split(mixed, &[100, -100]);
    let before = f.v.list_transaction_splits(mixed).unwrap();
    assert!(
        f.v.link_refund(mixed, "".into(), expense, "".into())
            .is_err()
    );
    f.link(mixed, &p[0], expense, "");
    f.link(refund, "", mixed, &p[1]);
    let after = f.v.list_transaction_splits(mixed).unwrap();
    assert_eq!(
        before
            .iter()
            .map(|p| (&p.uuid, p.amount_cents, p.category_id, &p.note))
            .collect::<Vec<_>>(),
        after
            .iter()
            .map(|p| (&p.uuid, p.amount_cents, p.category_id, &p.note))
            .collect::<Vec<_>>()
    );
    let rows = f.v.list_transactions(BookingFilter::default()).unwrap();
    let b = rows.transactions.iter().find(|b| b.id == mixed).unwrap();
    assert_eq!(
        (
            &*b.refund_marker,
            b.refund_cents,
            b.refunded_cents,
            b.split_count
        ),
        ("both", 100, 100, 2)
    );
    assert_eq!(f.v.refund_links(mixed).unwrap().len(), 2);
    assert!(
        f.v.link_refund(mixed, p[0].clone(), mixed, p[1].clone())
            .is_err()
    );
    assert!(
        f.v.link_refund(expense, "".into(), refund, "".into())
            .is_err()
    );
}

#[test]
fn deleted_part_resolves_only_a_unique_snapshot_match_without_changing_the_key() {
    let f = Fixture::new();
    let (r, o) = (f.add("r", 100), f.add("o", -200));
    let parts = f.split(o, &[-100, -100]);
    f.link(r, "", o, &parts[0]);
    let created = f.v.refund_links(r).unwrap()[0].created_ms;
    f.v.set_transaction_splits(
        o,
        vec![part(None, -100), part(Some(parts[1].clone()), -100)],
    )
    .unwrap();
    // The removed part has two indistinguishable replacements: no guessing.
    assert!(
        f.v.refund_links(r)
            .unwrap()
            .iter()
            .all(|l| l.inactive && l.allocated_cents == 0)
    );
    assert_eq!(
        number(
            &f.v,
            "SELECT COUNT(*) FROM refund_links WHERE deleted_ms IS NOT NULL"
        ),
        0
    );
    let r2 = f.add("r2", 100);
    let current = f.v.list_transaction_splits(o).unwrap();
    f.link(r2, "", o, &current[0].uuid);
    f.v.set_transaction_splits(
        o,
        vec![
            part(None, -100),
            part(Some(current[1].uuid.clone()), -50),
            part(None, -50),
        ],
    )
    .unwrap();
    let l = f.v.refund_links(r2).unwrap();
    assert_eq!(l.len(), 1);
    assert_eq!(shares(&f.v, r), [100]);
    assert_eq!(l[0].allocated_cents, 0);
    assert_eq!(l[0].original_part, current[0].uuid);
    assert_ne!(l[0].resolved_original_part.as_ref(), Some(&current[0].uuid));
    assert!(l[0].created_ms > created);
    assert_eq!(
        number(
            &f.v,
            "SELECT COUNT(*) FROM refund_links WHERE deleted_ms IS NOT NULL"
        ),
        0
    );
}

#[test]
fn whole_booking_split_and_split_removal_use_the_same_resolver() {
    let f = Fixture::new();
    let (r, o) = (f.add("r", 100), f.add("o", -100));
    f.link(r, "", o, "");
    let original_created = f.v.refund_links(r).unwrap()[0].created_ms;
    let p = f.split(o, &[-100, 20, -20]);
    let l = f.v.refund_links(r).unwrap();
    assert_eq!(l[0].original_part, "");
    assert_eq!(l[0].resolved_original_part.as_ref(), Some(&p[0]));
    assert_eq!(l[0].created_ms, original_created);
    f.v.set_transaction_splits(o, vec![]).unwrap();
    let l = f.v.refund_links(r).unwrap();
    assert_eq!(l[0].original_part, "");
    assert_eq!(l[0].created_ms, original_created);
    f.split(o, &[-60, -40]);
    assert!(f.v.refund_links(r).unwrap()[0].inactive);
}

#[test]
fn resolver_reserves_explicit_pairs_before_unique_fallbacks() {
    let f = Fixture::new();
    let (r, o) = (f.add("r", 200), f.add("o", -200));
    let p = f.split(o, &[-100, -100]);
    f.link(r, "", o, &p[0]);
    f.link(r, "", o, &p[1]);
    raw(
        &f.v,
        &format!(
            "UPDATE refund_links SET original_part='missing' WHERE original_part='{}'",
            p[0]
        ),
    );
    let l = f.v.refund_links(r).unwrap();
    assert_eq!(
        l.iter().map(|l| l.allocated_cents).collect::<Vec<_>>(),
        [100, 100]
    );
    assert_eq!(l[0].resolved_original_part.as_deref(), Some(p[0].as_str()));
}

#[test]
fn sign_flip_currency_change_and_deletion_drop_links_without_rewriting_bookings() {
    let f = Fixture::new();
    let (r, o) = (f.add("r", 100), f.add("o", -100));
    raw(
        &f.v,
        &format!("UPDATE transactions SET currency='USD' WHERE id={o}"),
    );
    assert!(f.v.link_refund(r, "".into(), o, "".into()).is_err());
    raw(
        &f.v,
        &format!("UPDATE transactions SET currency='EUR' WHERE id={o}"),
    );
    f.link(r, "", o, "");
    raw(
        &f.v,
        &format!("UPDATE transactions SET currency='USD' WHERE id={o}"),
    );
    assert!(f.v.refund_links(r).unwrap()[0].inactive);
    assert_eq!(
        number(
            &f.v,
            "SELECT COUNT(*) FROM refund_links WHERE deleted_ms IS NULL"
        ),
        1
    );
    raw(
        &f.v,
        &format!("UPDATE transactions SET currency='EUR' WHERE id={o}"),
    );
    f.change(o, 100, None);
    assert!(
        f.v.refund_links(r)
            .unwrap()
            .iter()
            .all(|l| l.inactive && l.allocated_cents == 0)
    );
    let o2 = f.add("o2", -100);
    f.link(r, "", o2, "");
    f.v.delete_manual_booking(o2).unwrap();
    assert!(
        f.v.refund_links(r)
            .unwrap()
            .iter()
            .all(|l| l.inactive && l.allocated_cents == 0)
    );
}

#[test]
fn mixed_sign_amount_edit_keeps_valid_part_links_and_rejects_bad_replacements_atomically() {
    let f = Fixture::new();
    let (r, o, m) = (f.add("r", 100), f.add("o", -100), f.add("m", 50));
    let p = f.split(m, &[100, -50]);
    f.link(m, &p[0], o, "");
    f.link(r, "", m, &p[1]);
    f.change(
        m,
        -50,
        Some(vec![
            part(Some(p[0].clone()), 100),
            part(Some(p[1].clone()), -150),
        ]),
    );
    assert_eq!(f.v.refund_links(m).unwrap().len(), 2);
    assert!(f.v.refund_links(m).unwrap().iter().all(|l| !l.inactive));
    let before =
        f.v.refund_links(m)
            .unwrap()
            .iter()
            .map(|l| l.changed_ms)
            .collect::<Vec<_>>();
    assert!(
        f.v.save_manual_booking(
            f.account,
            Some(m),
            ManualBookingSetup {
                booking_date: "2026-01-01".into(),
                amount_cents: 200,
                counterparty: "bad".into(),
                purpose: String::new(),
                category_id: None,
                note: String::new()
            }
        )
        .is_err()
    );
    assert_eq!(
        f.v.refund_links(m)
            .unwrap()
            .iter()
            .map(|l| l.changed_ms)
            .collect::<Vec<_>>(),
        before
    );
}

#[test]
fn undo_is_cas_preserves_order_and_relink_gets_a_new_position_even_after_merge() {
    let f = Fixture::new();
    let (r, a, b) = (f.add("r", 100), f.add("a", -100), f.add("b", -100));
    f.link(r, "", a, "");
    let first = f.v.refund_links(r).unwrap()[0].created_ms;
    let token = f.unlink(r, "", a, "");
    f.link(r, "", b, "");
    f.v.restore_refund(r, "".into(), a, "".into(), token)
        .unwrap();
    assert_eq!(shares(&f.v, r), [100, 0]);
    assert_eq!(f.v.refund_links(r).unwrap()[0].created_ms, first);
    assert!(
        f.v.restore_refund(r, "".into(), a, "".into(), token)
            .is_err()
    );
    f.unlink(r, "", a, "");
    f.unlink(r, "", b, "");
    f.link(r, "", a, "");
    assert!(f.v.refund_links(r).unwrap()[0].created_ms > first);
    let peer = Fixture::new();
    merge(&peer.v, &f.v);
    let local_token = f.unlink(r, "", a, "");
    merge(&peer.v, &f.v);
    let pr = number(&peer.v, "SELECT id FROM transactions WHERE external_id='r'");
    let pa = number(&peer.v, "SELECT id FROM transactions WHERE external_id='a'");
    peer.v
        .restore_refund(pr, "".into(), pa, "".into(), local_token)
        .unwrap();
    peer.v.unlink_refund(pr, "".into(), pa, "".into()).unwrap();
    merge(&f.v, &peer.v);
    assert!(
        f.v.restore_refund(r, "".into(), a, "".into(), local_token)
            .is_err()
    );
    f.link(r, "", a, "");
    let created = f.v.refund_links(r).unwrap()[0].created_ms;
    merge(&f.v, &peer.v);
    merge(&peer.v, &f.v);
    assert_eq!(f.v.refund_links(r).unwrap()[0].created_ms, created);
    assert_eq!(peer.v.refund_links(pr).unwrap()[0].created_ms, created);
}

#[test]
fn restore_resolves_the_original_key_and_inactive_links_can_still_be_unlinked() {
    let f = Fixture::new();
    let (r, o) = (f.add("r", 100), f.add("o", -100));
    f.link(r, "", o, "");
    let token = f.unlink(r, "", o, "");
    f.split(o, &[-100, 20, -20]);
    f.v.restore_refund(r, "".into(), o, "".into(), token)
        .unwrap();
    assert_eq!(shares(&f.v, r), [100]);
    f.split(o, &[-60, -40]);
    assert!(
        f.v.restore_refund(r, "".into(), o, "".into(), token)
            .is_err()
    );
    let token = f.v.unlink_refund(r, "".into(), o, "".into()).unwrap();
    assert!(
        f.v.restore_refund(r, "".into(), o, "".into(), token)
            .is_err()
    );
}

#[test]
fn cache_tracks_link_split_booking_writes_merge_and_data_replacement() {
    let f = Fixture::new();
    let (r, o) = (f.add("r", 100), f.add("o", -100));
    let projection = || {
        let db = f.v.db().unwrap();
        f.v.refund_projection(&db).unwrap()
    };
    let a = projection();
    assert!(Arc::ptr_eq(&a, &projection()));
    f.link(r, "", o, "");
    let b = projection();
    assert!(!Arc::ptr_eq(&a, &b));
    f.split(o, &[-60, -40]);
    let c = projection();
    assert!(!Arc::ptr_eq(&b, &c));
    f.change(r, 90, None);
    let d = projection();
    assert!(!Arc::ptr_eq(&c, &d));
    let peer = Fixture::new();
    peer.add("extra", 1);
    merge(&f.v, &peer.v);
    let e = projection();
    assert!(!Arc::ptr_eq(&d, &e));
    {
        let _db = f.v.db().unwrap();
        f.v.data_replaced();
    }
    assert!(!Arc::ptr_eq(&e, &projection()));
    let cached = projection();
    {
        let db = f.v.db().unwrap();
        let tx = db.unchecked_transaction().unwrap();
        tx.execute("UPDATE transactions SET note='transient'", [])
            .unwrap();
        let transient = f.v.refund_projection(&tx).unwrap();
        assert!(!Arc::ptr_eq(&cached, &transient));
        assert!(Arc::ptr_eq(&cached, &f.v.refunds.current().unwrap()));
    }
    assert!(!Arc::ptr_eq(&cached, &projection()));
}

#[test]
fn stable_identity_ties_ignore_local_ids_and_mutable_stamps() {
    let f = Fixture::new();
    let (r, z, a) = (f.add("r", 100), f.add("z", -100), f.add("a", -100));
    f.link(r, "", z, "");
    let token = f.unlink(r, "", z, "");
    f.link(r, "", a, "");
    f.v.restore_refund(r, "".into(), z, "".into(), token)
        .unwrap();
    raw(
        &f.v,
        "UPDATE refund_links SET created_ms=10,changed_ms=20,deleted_ms=NULL",
    );
    let l = f.v.refund_links(r).unwrap();
    assert_eq!(l[0].original_id, a);
    assert_eq!(l[0].allocated_cents, 100);
    raw(
        &f.v,
        &format!("UPDATE refund_links SET changed_ms=999 WHERE original_id={z}"),
    );
    assert_eq!(f.v.refund_links(r).unwrap()[0].original_id, a);
}

#[test]
fn allocation_ties_use_account_sync_identity_before_local_booking_ids() {
    let f = Fixture::new();
    let (z, a, o) = (
        f.add("refund", 100),
        f.add("other-refund", 100),
        f.add("original", -100),
    );
    let account =
        f.v.create_offline_account("Other".into(), AccountKind::Cash, "EUR".into(), 0)
            .unwrap();
    raw(
        &f.v,
        &format!(
            "UPDATE accounts SET external_ref='Z' WHERE id={};UPDATE accounts SET external_ref='A' WHERE id={account};UPDATE transactions SET account_id={account} WHERE id={a}",
            f.account
        ),
    );
    f.link(z, "", o, "");
    let token = f.unlink(z, "", o, "");
    f.link(a, "", o, "");
    f.v.restore_refund(z, String::new(), o, String::new(), token)
        .unwrap();
    raw(&f.v, "UPDATE refund_links SET created_ms=10");
    let l = f.v.refund_links(o).unwrap();
    assert_eq!(l[0].refund_id, a);
    assert_eq!(l[0].allocated_cents, 100);
    assert_eq!(l[1].allocated_cents, 0);
}

#[test]
fn relink_appends_after_other_live_links() {
    let f = Fixture::new();
    let (r, a, b) = (f.add("r", 200), f.add("a", -100), f.add("b", -100));
    f.link(r, "", a, "");
    f.link(r, "", b, "");
    let original = f.v.refund_links(r).unwrap()[0].created_ms;
    f.unlink(r, "", a, "");
    f.link(r, "", a, "");
    let l = f.v.refund_links(r).unwrap();
    assert_eq!((l[0].original_id, l[1].original_id), (b, a));
    assert!(l[1].created_ms > original);
}

#[test]
fn search_filters_capacity_before_limit_and_suggestions_are_part_specific() {
    let f = Fixture::new();
    let (r, a, b) = (f.add("r", 200), f.add("a", -100), f.add("b", -100));
    let p = f.split(r, &[100, 100]);
    f.link(r, &p[0], a, "");
    let one =
        f.v.refund_candidates(r, p[1].clone(), String::new(), 1)
            .unwrap();
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].transaction_id, b);
    f.unlink(r, &p[0], a, "");
    f.link(r, &p[0], b, "");
    let suggestions = f.v.refund_suggestions(r, p[1].clone()).unwrap();
    assert!(suggestions.iter().any(|s| s.candidate.transaction_id == a));
    assert!(
        f.v.refund_candidates(r, p[0].clone(), String::new(), 10)
            .unwrap()
            .is_empty()
    );
    assert!(
        f.v.refund_candidates(r, p[1].clone(), "no match".into(), 10)
            .unwrap()
            .is_empty()
    );
    assert!(
        f.v.refund_candidates(r, p[1].clone(), String::new(), 0)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn suggestion_ranking_merchant_window_reference_tokens_and_iso_date_exclusion() {
    let f = Fixture::new();
    let r = f.add("source", 100);
    let equal = f.add("equal", -100);
    let merchant = f.add("merchant", -200);
    let reference = f.add("reference", -300);
    let date_only = f.add("date-only", -400);
    let old = f.add("old", -500);
    raw(
        &f.v,
        &format!(
            "UPDATE transactions SET purpose='2026-01-01 REF12345' WHERE id IN ({r},{reference});
        UPDATE transactions SET purpose='2026-01-01' WHERE id={date_only};
        UPDATE transactions SET counterparty='Shop' WHERE id IN ({r},{merchant},{old});
        UPDATE transactions SET booking_date='2025-10-30' WHERE id={old};"
        ),
    );
    let s = f.v.refund_suggestions(r, String::new()).unwrap();
    assert_eq!(
        s.iter()
            .map(|s| (s.candidate.transaction_id, s.reason))
            .collect::<Vec<_>>(),
        [
            (equal, RefundMatch::SameAmount),
            (merchant, RefundMatch::SameMerchant),
            (reference, RefundMatch::SameReference)
        ]
    );
    raw(
        &f.v,
        &format!(
            "UPDATE transactions SET purpose='2026-01-01' WHERE id={r};UPDATE transactions SET counterparty='other' WHERE id IN ({merchant},{old});"
        ),
    );
    assert!(
        f.v.refund_suggestions(r, String::new())
            .unwrap()
            .iter()
            .all(|s| s.candidate.transaction_id != reference
                && s.candidate.transaction_id != date_only
                && s.candidate.transaction_id != old)
    );
}

#[test]
fn gross_capacity_and_minimum_i64_never_overflow() {
    let f = Fixture::new();
    let (r, o) = (f.add("r", 100), f.add("o", -100));
    let m = f.add("mixed", 0);
    f.split(m, &[i64::MAX, -i64::MAX, 1, -1]);
    assert!(f.v.link_refund(r, "".into(), m, String::new()).is_err());
    let uuid = f.v.list_transaction_splits(m).unwrap()[0].uuid.clone();
    assert!(f.v.link_refund(m, uuid, o, String::new()).is_err());
    let min = f.add("min", i64::MIN);
    assert!(f.v.link_refund(r, "".into(), min, String::new()).is_err());
}

fn fingerprint(v: &Vault) -> Vec<(String, String, String, String, i64)> {
    let db = v.db().unwrap();
    let p = v.refund_projection(&db).unwrap();
    let mut rows: Vec<_> = p
        .allocations
        .iter()
        .filter(|a| a.cents > 0)
        .map(|a| {
            let r = a.refund.as_ref().unwrap();
            let o = a.original.as_ref().unwrap();
            (
                p.state.books[&r.0].identity.clone(),
                r.1.clone(),
                p.state.books[&o.0].identity.clone(),
                o.1.clone(),
                a.cents,
            )
        })
        .collect();
    rows.sort();
    rows
}

type StoredLink = (
    String,
    String,
    String,
    String,
    i64,
    i64,
    Option<i64>,
    i64,
    i64,
);
type Undo = (i64, String, i64, String, i64);

fn raw_fingerprint(v: &Vault) -> Vec<StoredLink> {
    let db = v.db().unwrap();
    let state = State::load(&db).unwrap();
    let mut rows: Vec<_> = state
        .rows
        .iter()
        .map(|r| {
            (
                state.books[&r.key.refund.0].identity.clone(),
                r.key.refund.1.clone(),
                state.books[&r.key.original.0].identity.clone(),
                r.key.original.1.clone(),
                r.created_ms,
                r.changed_ms,
                r.deleted_ms,
                r.refund_cents,
                r.original_cents,
            )
        })
        .collect();
    rows.sort();
    rows
}

#[test]
fn full_row_lww_is_associative_with_equal_stamps_different_creations_and_payloads() {
    let a = Fixture::new();
    let (r, o) = (a.add("r", 100), a.add("o", -100));
    a.link(r, "", o, "");
    let b = Fixture::new();
    let c = Fixture::new();
    merge(&b.v, &a.v);
    merge(&c.v, &a.v);
    for (v, created, cents) in [(&a.v, 10, 100), (&b.v, 20, 200), (&c.v, 10, 250)] {
        raw(
            v,
            &format!(
                "UPDATE refund_links SET changed_ms=50,created_ms={created},refund_cents={cents},original_cents=-{cents}"
            ),
        );
    }
    let left = Fixture::new();
    let right = Fixture::new();
    merge(&left.v, &a.v);
    merge(&right.v, &a.v);
    merge(&left.v, &b.v);
    merge(&left.v, &c.v);
    merge(&b.v, &c.v);
    merge(&right.v, &b.v);
    assert_eq!(raw_fingerprint(&left.v), raw_fingerprint(&right.v));
    assert_eq!(number(&left.v, "SELECT created_ms FROM refund_links"), 20);
    assert_eq!(
        number(&left.v, "SELECT refund_cents FROM refund_links"),
        200
    );
    raw(
        &c.v,
        "UPDATE refund_links SET created_ms=20,refund_cents=300,original_cents=-300",
    );
    merge(&left.v, &c.v);
    merge(&c.v, &left.v);
    assert_eq!(raw_fingerprint(&left.v), raw_fingerprint(&c.v));
    assert_eq!(
        number(&left.v, "SELECT refund_cents FROM refund_links"),
        300
    );
    raw(&c.v, "UPDATE refund_links SET deleted_ms=50");
    merge(&left.v, &c.v);
    assert!(fingerprint(&left.v).is_empty());
}

#[test]
fn generated_two_peer_link_unlink_undo_split_delete_histories_converge_in_both_merge_orders() {
    for seed in 1..=8u64 {
        let a = Fixture::new();
        for i in 0..4 {
            a.add(&format!("r{i}"), 100);
            a.add(&format!("o{i}"), -100);
        }
        let id = |v: &Vault, name: &str| {
            v.db()
                .unwrap()
                .query_row(
                    "SELECT id FROM transactions WHERE external_id=?1",
                    [name],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap()
        };
        a.link(id(&a.v, "r0"), "", id(&a.v, "o0"), "");
        let b = Fixture::new();
        merge(&b.v, &a.v);
        let mut rng = seed;
        let mut tokens: [Vec<Undo>; 2] = Default::default();
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        for step in 0..96 {
            let side = (next() % 2) as usize;
            let v = if side == 0 { &a.v } else { &b.v };
            let r = id(v, &format!("r{}", next() % 4));
            let o = id(v, &format!("o{}", next() % 4));
            let endpoint = |id, positive| {
                let parts = v.list_transaction_splits(id).unwrap_or_default();
                parts
                    .iter()
                    .find(|p| (p.amount_cents > 0) == positive)
                    .map(|p| p.uuid.clone())
                    .unwrap_or_default()
            };
            match step % 6 {
                0 => {
                    let _ = v.link_refund(r, endpoint(r, true), o, endpoint(o, false));
                }
                1 => {
                    if let Some(l) = v.refund_links(r).unwrap().first()
                        && let Ok(token) = v.unlink_refund(
                            l.refund_id,
                            l.refund_part.clone(),
                            l.original_id,
                            l.original_part.clone(),
                        )
                    {
                        tokens[side].push((
                            l.refund_id,
                            l.refund_part.clone(),
                            l.original_id,
                            l.original_part.clone(),
                            token,
                        ));
                    }
                }
                2 => {
                    if let Some((r, rp, o, op, t)) = tokens[side].pop() {
                        let _ = v.restore_refund(r, rp, o, op, t);
                    }
                }
                3 | 4 => {
                    let target = if next() % 2 == 0 { r } else { o };
                    let sign = if target == r { 1 } else { -1 };
                    let parts = if next() % 3 == 0 {
                        Vec::new()
                    } else {
                        vec![part(None, 40 * sign), part(None, 60 * sign)]
                    };
                    let _ = v.set_transaction_splits(target, parts);
                }
                _ => {
                    if step > 72 {
                        let _ = v.delete_manual_booking(if next() % 2 == 0 { r } else { o });
                    }
                }
            }
            if step % 24 == 23 {
                let left = Fixture::new();
                let right = Fixture::new();
                merge(&left.v, &a.v);
                merge(&right.v, &b.v);
                merge(&left.v, &b.v);
                merge(&right.v, &a.v);
                assert_eq!(
                    raw_fingerprint(&left.v),
                    raw_fingerprint(&right.v),
                    "raw rows seed {seed}, step {step}"
                );
                assert_eq!(
                    fingerprint(&left.v),
                    fingerprint(&right.v),
                    "allocations seed {seed}, step {step}"
                );
                let before = raw_fingerprint(&left.v);
                merge(&left.v, &right.v);
                assert_eq!(before, raw_fingerprint(&left.v));
            }
        }
    }
}

#[test]
fn synced_split_dangling_uuids_and_unlinks_are_never_rewritten_by_merge() {
    let a = Fixture::new();
    let (r, o) = (a.add("r", 200), a.add("o", -200));
    let p = a.split(o, &[-80, -120]);
    a.link(r, "", o, &p[0]);
    a.link(r, "", o, &p[1]);
    let b = Fixture::new();
    merge(&b.v, &a.v);
    let br = number(&b.v, "SELECT id FROM transactions WHERE external_id='r'");
    let bo = number(&b.v, "SELECT id FROM transactions WHERE external_id='o'");
    b.v.unlink_refund(br, "".into(), bo, p[0].clone()).unwrap();
    merge(&a.v, &b.v);
    assert_eq!(a.v.refund_links(r).unwrap().len(), 1);
    assert_eq!(shares(&a.v, r), [120]);
    raw(
        &b.v,
        &format!(
            "UPDATE transaction_splits SET uuid='replacement' WHERE uuid='{}';UPDATE transactions SET splits_ms=splits_ms+1000 WHERE id={bo}",
            p[1]
        ),
    );
    let before = raw_fingerprint(&b.v);
    merge(&a.v, &b.v);
    assert_eq!(before, raw_fingerprint(&a.v));
    assert_eq!(
        a.v.refund_links(r).unwrap()[0]
            .resolved_original_part
            .as_deref(),
        Some("replacement")
    );
    assert_eq!(a.v.refund_links(r).unwrap()[0].original_part, p[1]);
    merge(&b.v, &a.v);
    assert_eq!(fingerprint(&a.v), fingerprint(&b.v));
}

#[test]
fn suggestions_exclude_the_selected_part_pair_not_the_entire_booking_pair() {
    let f = Fixture::new();
    let (r, o) = (f.add("r", 200), f.add("o", -150));
    let p = f.split(r, &[50, 150]);
    f.link(r, &p[0], o, "");
    assert!(
        f.v.refund_suggestions(r, p[1].clone())
            .unwrap()
            .iter()
            .any(|s| s.candidate.transaction_id == o)
    );
    let reverse =
        f.v.refund_candidates(o, String::new(), String::new(), 10)
            .unwrap();
    assert_eq!(reverse.len(), 1);
    assert_eq!(reverse[0].part, p[1]);
    assert!(
        f.v.refund_candidates(r, String::new(), String::new(), 10)
            .is_err()
    );
}

#[test]
fn expired_pending_booking_cleanup_leaves_links_read_time_inactive() {
    let f = Fixture::new();
    let (r, o) = (f.add("r", 100), f.add("pending-old", -100));
    raw(
        &f.v,
        &format!("UPDATE transactions SET imported_unix=0 WHERE id={o}"),
    );
    f.link(r, "", o, "");
    let c =
        f.v.add_bank_connection(
            crate::models::ConnectorKind::Comdirect,
            String::new(),
            "login".into(),
        )
        .unwrap();
    crate::connectors::store_fetch(
        &f.v,
        crate::models::ConnectorKind::Comdirect,
        c,
        crate::connectors::FetchResult::default(),
    )
    .unwrap();
    assert!(
        f.v.refund_links(r)
            .unwrap()
            .iter()
            .all(|l| l.inactive && l.allocated_cents == 0)
    );
    assert_eq!(
        number(
            &f.v,
            "SELECT COUNT(*) FROM refund_links WHERE deleted_ms IS NOT NULL"
        ),
        0
    );
}
