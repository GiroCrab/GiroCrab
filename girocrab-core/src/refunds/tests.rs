mod round7 {
    use super::*;

    #[test]
    fn every_suggestion_reason_uses_the_inclusive_62_day_window() {
        for (amount, merchant, reference) in [
            (-100, false, false),
            (-200, true, false),
            (-200, false, true),
        ] {
            let f = Fixture::new();
            let r = f.add("refund", 100);
            let o = f.add("original", amount);
            raw(
                &f.v,
                &format!(
                    "UPDATE transactions SET booking_date='2024-01-01',purpose='{}',counterparty=CASE WHEN id={r} THEN 'Refund Vendor' ELSE '{}' END WHERE id IN ({r},{o}); UPDATE transactions SET booking_date='2024-03-04' WHERE id={r}",
                    if reference { "INV123456" } else { "" },
                    if merchant {
                        "Refund Vendor"
                    } else {
                        "Other Vendor"
                    }
                ),
            );
            assert!(f.v.refund_suggestions(r, String::new()).unwrap().is_empty());
            assert!(
                f.v.refund_candidates(r, String::new(), String::new(), 200)
                    .unwrap()
                    .iter()
                    .any(|c| c.transaction_id == o)
            );
            raw(
                &f.v,
                &format!("UPDATE transactions SET booking_date='2024-03-03' WHERE id={r}"),
            );
            assert_eq!(f.v.refund_suggestions(r, String::new()).unwrap().len(), 1);
        }
    }
    #[test]
    fn concurrent_unique_replacements_on_both_endpoints_keep_the_link() {
        for reverse in [false, true] {
            let a = Fixture::new();
            let (r, o) = (a.add("r", 150), a.add("o", -300));
            let rp = a.split(r, &[100, 50]);
            let op = a.split(o, &[-100, -200]);
            a.link(r, &rp[0], o, &op[0]);
            let b = Fixture::new();
            merge(&b.v, &a.v);
            let br = review_peer_id(&b.v, &a.v, r);
            let bo = review_peer_id(&b.v, &a.v, o);
            a.v.set_transaction_splits(r, vec![part(None, 100), part(Some(rp[1].clone()), 50)])
                .unwrap();
            b.v.set_transaction_splits(bo, vec![part(None, -100), part(Some(op[1].clone()), -200)])
                .unwrap();
            if reverse {
                merge(&b.v, &a.v);
                merge(&a.v, &b.v);
            } else {
                merge(&a.v, &b.v);
                merge(&b.v, &a.v);
            }
            assert_eq!(shares(&a.v, r), [100]);
            assert_eq!(shares(&b.v, br), [100]);
        }
    }

    #[test]
    fn deleted_split_vector_is_not_partially_discarded_by_merge() {
        for reverse in [false, true] {
            let a = Fixture::new();
            let old = a.add("old", -100);
            let canonical = a.add("canonical", -100);
            let parts = a.split(old, &[-70, -30]);
            let b = Fixture::new();
            merge(&b.v, &a.v);
            let bo = review_peer_id(&b.v, &a.v, old);
            raw(
                &a.v,
                &format!(
                    "UPDATE transaction_splits SET transaction_id={canonical} WHERE transaction_id={old}; UPDATE transactions SET deleted_ms=1000,splits_ms=100 WHERE id={old}"
                ),
            );
            a.v.set_transaction_splits(
                canonical,
                vec![part(Some(parts[0].clone()), -70), part(None, -30)],
            )
            .unwrap();
            raw(
                &b.v,
                &format!("UPDATE transactions SET deleted_ms=1000,splits_ms=300 WHERE id={bo}"),
            );
            if reverse {
                merge(&b.v, &a.v);
                merge(&a.v, &b.v);
            } else {
                merge(&a.v, &b.v);
                merge(&b.v, &a.v);
            }
            for (v, id) in [(&a.v, old), (&b.v, bo)] {
                assert_eq!(
                    number(
                        v,
                        &format!(
                            "SELECT SUM(amount_cents) FROM transaction_splits WHERE transaction_id={id}"
                        )
                    ),
                    -100
                );
            }
            let next = a.add("next", -100);
            raw(
                &a.v,
                &format!(
                    "UPDATE transaction_splits SET transaction_id={next} WHERE transaction_id={old}; UPDATE transactions SET splits_ms=100 WHERE id={old}; UPDATE transactions SET splits_ms=400 WHERE id={next}"
                ),
            );
            merge(&a.v, &b.v);
            merge(&b.v, &a.v);
            assert_eq!(
                number(
                    &a.v,
                    &format!(
                        "SELECT SUM(amount_cents) FROM transaction_splits WHERE transaction_id={old}"
                    )
                ),
                -100
            );
        }
    }

    #[test]
    fn concurrent_twin_folds_keep_split_uuids_unique() {
        for (stamp, reverse) in [(100, false), (100, true), (200, false), (200, true)] {
            let a = Fixture::new();
            let source = a.add("purchase", -100);
            let parts = a.split(source, &[-40, -60]);
            let first =
                a.v.create_offline_account("First".into(), AccountKind::Checking, "EUR".into(), 0)
                    .unwrap();
            let second =
                a.v.create_offline_account("Second".into(), AccountKind::Checking, "EUR".into(), 0)
                    .unwrap();
            raw(
                &a.v,
                &format!(
                    "INSERT INTO bank_connections(connector,display_name,login_name) VALUES ('fints:52090000','Bank','concurrent'); UPDATE accounts SET connection_id=last_insert_rowid() WHERE id IN ({first},{second}); UPDATE accounts SET external_ref='moneymoney-db:concurrent',kind='checking',iban='' WHERE id={}; UPDATE accounts SET iban='DE11111111111111111111' WHERE id={first}; UPDATE accounts SET iban='DE22222222222222222222' WHERE id={second}",
                    a.account
                ),
            );
            raw(
                &a.v,
                &format!(
                    "INSERT INTO transactions(account_id,external_id,booking_date,amount_cents,currency,counterparty) VALUES ({first},'purchase','2026-01-01',-100,'EUR','purchase'),({second},'purchase','2026-01-01',-100,'EUR','purchase')"
                ),
            );
            raw(
                &a.v,
                &format!(
                    "UPDATE transactions SET splits_ms=100 WHERE id={source}; INSERT INTO transactions(account_id,external_id,booking_date,amount_cents,currency,counterparty) VALUES ({first},'refund','2026-01-01',100,'EUR','refund')"
                ),
            );
            let refund = number(
                &a.v,
                "SELECT id FROM transactions WHERE external_id='refund'",
            );
            a.link(refund, "", source, &parts[0]);
            let b = Fixture::new();
            merge(&b.v, &a.v);
            raw(
                &a.v,
                &format!(
                    "UPDATE accounts SET iban='DE11111111111111111111' WHERE id={}",
                    a.account
                ),
            );
            let peer_source: i64 =
                b.v.db()
                    .unwrap()
                    .query_row(
                        "SELECT id FROM accounts WHERE external_ref='moneymoney-db:concurrent'",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap();
            raw(
                &b.v,
                &format!(
                    "UPDATE accounts SET iban='DE22222222222222222222' WHERE id={peer_source}"
                ),
            );
            assert_eq!(
                crate::sync_merge::twins::fold_twins(&a.v.db().unwrap()).unwrap(),
                1
            );
            assert_eq!(
                crate::sync_merge::twins::fold_twins(&b.v.db().unwrap()).unwrap(),
                1
            );
            raw(
                &b.v,
                &format!(
                    "UPDATE transactions SET splits_ms={stamp} WHERE external_id='purchase' AND account_id IN (SELECT id FROM accounts WHERE name='Second')"
                ),
            );
            if reverse {
                merge(&b.v, &a.v);
                merge(&a.v, &b.v);
            } else {
                merge(&a.v, &b.v);
                merge(&b.v, &a.v);
            }
            merge(&a.v, &b.v);
            let snapshot = |v: &Vault| -> Vec<(String, String, i64, String)> {
                v.db().unwrap().prepare("SELECT a.external_ref,t.external_id,s.position,s.uuid FROM transaction_splits s JOIN transactions t ON t.id=s.transaction_id JOIN accounts a ON a.id=t.account_id WHERE t.deleted_ms IS NULL ORDER BY a.external_ref,t.external_id,s.position").unwrap()
                .query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap().collect::<Result<_,_>>().unwrap()
            };
            assert_eq!(snapshot(&a.v), snapshot(&b.v));
            for v in [&a.v, &b.v] {
                assert_eq!(
                    number(
                        v,
                        &format!(
                            "SELECT count(*) FROM transaction_splits WHERE uuid='{}'",
                            parts[0]
                        )
                    ),
                    1
                );
                assert_eq!(
                    number(
                        v,
                        "SELECT count(*) FROM transaction_splits s JOIN transactions t ON t.id=s.transaction_id WHERE t.deleted_ms IS NULL"
                    ),
                    4
                );
                let refund = number(
                    v,
                    "SELECT id FROM transactions WHERE external_id='refund' AND deleted_ms IS NULL",
                );
                let links = v.refund_links(refund).unwrap();
                assert_eq!(links.iter().map(|l| l.allocated_cents).sum::<i64>(), 40);
                assert_eq!(links.iter().filter(|l| l.inactive).count(), 1);
                if stamp == 200 {
                    let owner:String=v.db().unwrap().query_row("SELECT a.name FROM transaction_splits s JOIN transactions t ON t.id=s.transaction_id JOIN accounts a ON a.id=t.account_id WHERE s.uuid=?1",[&parts[0]],|r|r.get(0)).unwrap();
                    assert_eq!(owner, "Second");
                }
            }
        }
    }

    #[test]
    fn split_never_becomes_whole_on_move() {
        let f = Fixture::new();
        let r = f.add("r", 100);
        let old = f.add("old", -8600);
        let new = f.add("new", -8600);
        let p = f.split(old, &[-8600, 100, -100]);
        f.link(r, "", old, &p[0]);
        store::move_booking(&f.v.db().unwrap(), old, new).unwrap();
        assert_eq!(f.v.refund_links(r).unwrap()[0].allocated_cents, 0);
    }

    #[test]
    fn unequal_fold_preserves_direct_link_and_source_unlink() {
        let f = Fixture::new();
        let r = f.add("r", 200);
        let old = f.add("old", -100);
        let new = f.add("new", -200);
        f.link(r, "", old, "");
        f.link(r, "", new, "");
        f.unlink(r, "", old, "");
        let before = State::load(&f.v.db().unwrap()).unwrap().rows;
        store::fold_booking(&f.v.db().unwrap(), old, new, false, false).unwrap();
        assert_eq!(State::load(&f.v.db().unwrap()).unwrap().rows, before);
    }

    #[test]
    fn round8_inactive_whole_unlink_does_not_delete_keeper_link() {
        let f = Fixture::new();
        let r = f.add("r", 200);
        let old = f.add("old", -100);
        let keeper = f.add("keeper", -100);
        f.link(r, "", old, "");
        f.link(r, "", keeper, "");
        f.split(old, &[-60, -40]);
        f.unlink(r, "", old, "");
        store::fold_booking(&f.v.db().unwrap(), old, keeper, true, true).unwrap();
        assert_eq!(shares(&f.v, r), [100]);
    }

    #[test]
    fn unequal_fold_does_not_retarget_matching_parts() {
        let f = Fixture::new();
        let r = f.add("r", 100);
        let old = f.add("old", -100);
        let new = f.add("new", -200);
        let p = f.split(old, &[-60, -40]);
        f.split(new, &[-60, -140]);
        f.link(r, "", old, &p[0]);
        let before = State::load(&f.v.db().unwrap()).unwrap().rows;
        store::fold_booking(&f.v.db().unwrap(), old, new, false, true).unwrap();
        assert_eq!(State::load(&f.v.db().unwrap()).unwrap().rows, before);
    }
}

mod round5 {
    use super::*;

    #[test]
    fn inactive_whole_booking_does_not_reactivate_on_hand_over() {
        for split in [false, true] {
            let f = Fixture::new();
            let r = f.add("refund", 100);
            let old = f.add("old", -100);
            let next = f.add("next", -100);
            f.link(r, "", old, "");
            if split {
                f.split(old, &[-60, -40]);
            }
            crate::importers::portal_import::hand_over(&f.v.db().unwrap(), old, next).unwrap();
            assert_eq!(
                f.v.refund_links(r)
                    .unwrap()
                    .iter()
                    .map(|l| l.allocated_cents)
                    .sum::<i64>(),
                if split { 0 } else { 100 }
            );
        }
    }

    #[test]
    fn staged_direct_link_with_provenance_survives_a_fold_collision() {
        let f = Fixture::new();
        let r = f.add("refund", 200);
        let bank = f.add("bank", -100);
        let twin = f.add("twin", -100);
        let u = f.split(bank, &[-60, -40])[0].clone();
        let v = f.split(twin, &[-60, -40])[0].clone();
        f.link(r, "", bank, &u);
        f.link(r, "", twin, &v);
        let db = f.v.db().unwrap();
        db.execute("UPDATE refund_links SET moved_from_refund_id=?1,moved_from_refund_part='',moved_from_original_id=?2,moved_from_original_part='old',moved_from_key='old-key' WHERE original_id=?3", params![r,twin,bank]).unwrap();
        let direct = State::load(&db)
            .unwrap()
            .rows
            .into_iter()
            .find(|l| l.key.original.0 == bank)
            .unwrap();
        db.execute_batch(
            "ATTACH DATABASE ':memory:' AS other;
            CREATE TABLE other.refund_links AS SELECT * FROM main.refund_links;
            CREATE TABLE other.transactions AS SELECT * FROM main.transactions;
            CREATE TABLE other.transaction_splits AS SELECT * FROM main.transaction_splits;
            DELETE FROM main.refund_links;",
        )
        .unwrap();
        store::merge_folded(&db, &HashMap::from([(r, r), (twin, bank), (bank, bank)])).unwrap();
        let rows = State::load(&db).unwrap().rows;
        let kept = rows
            .iter()
            .find(|l| l.key.original == Endpoint(bank, u.clone()))
            .unwrap();
        assert_eq!(kept.changed_ms, direct.changed_ms);
        assert_eq!(kept.moved_from_key, direct.moved_from_key);
        drop(db);
        assert_eq!(
            f.v.refund_links(r)
                .unwrap()
                .iter()
                .map(|l| l.allocated_cents)
                .sum::<i64>(),
            60
        );
    }

    #[test]
    fn imported_bank_and_part_collisions_are_inactive_together() {
        for reverse in [false, true] {
            let f = Fixture::new();
            let r = f.add("refund", 200);
            let bank = f.add("id:REVIEW123456---bank", -100);
            let mut parts = vec![-100, -50, 50];
            if reverse {
                parts.reverse();
            }
            let mut original = 0;
            for (i, amount) in parts.into_iter().enumerate() {
                let id = f.add(&format!("mm-part-{i}"), amount);
                raw(
                    &f.v,
                    &format!("UPDATE transactions SET purpose='EREF: REVIEW123456' WHERE id={id}"),
                );
                if amount == -100 {
                    original = id;
                }
            }
            f.link(r, "", bank, "");
            f.link(r, "", original, "");
            assert_eq!(
                crate::importers::portal_import::merge_imported_split_parts(
                    &f.v.db().unwrap(),
                    f.account
                )
                .unwrap(),
                1
            );
            assert!(
                f.v.refund_links(r)
                    .unwrap()
                    .iter()
                    .all(|l| l.allocated_cents == 0)
            );
        }
    }

    #[test]
    fn synced_twin_preserves_link_stamps_and_source_identity() {
        let f = Fixture::new();
        let r = f.add("refund", 200);
        let bank = f.add("bank", -100);
        let twin = f.add("mm-twin", -100);
        raw(
            &f.v,
            &format!("UPDATE transactions SET counterparty='bank' WHERE id={twin}"),
        );
        f.split(twin, &[-100, -50, 50]);
        f.link(r, "", bank, "");
        let db = f.v.db().unwrap();
        let before = State::load(&db).unwrap().rows.remove(0);
        let source_identity = crate::sync_merge::booking_key(&db, bank).unwrap();
        assert!(
            crate::importers::moneymoney_import::merge_synced_twin(
                &db,
                bank,
                "2026-01-01",
                -100,
                "",
                "bank",
                ""
            )
            .unwrap()
        );
        let after = State::load(&db)
            .unwrap()
            .rows
            .into_iter()
            .find(|l| l.deleted_ms.is_none())
            .unwrap();
        assert_eq!(after.changed_ms, before.changed_ms);
        assert_eq!(after.created_ms, before.created_ms);
        assert!(after.moved_from_key.contains(&source_identity));
    }
}

mod self_review {
    use super::*;

    #[test]
    fn staged_fold_does_not_reactivate_an_inactive_whole_endpoint() {
        let f = Fixture::new();
        let r = f.add("refund", 100);
        let old = f.add("old", -100);
        let next = f.add("next", -100);
        f.link(r, "", old, "");
        f.split(old, &[-60, -40]);
        let db = f.v.db().unwrap();
        db.execute_batch(
            "ATTACH DATABASE ':memory:' AS other;
            CREATE TABLE other.refund_links AS SELECT * FROM main.refund_links;
            CREATE TABLE other.transactions AS SELECT * FROM main.transactions;
            CREATE TABLE other.transaction_splits AS SELECT * FROM main.transaction_splits;
            DELETE FROM main.refund_links;",
        )
        .unwrap();
        store::merge_folded(&db, &HashMap::from([(r, r), (old, next)])).unwrap();
        drop(db);
        // the old whole-booking link is still there (not merely "no active link"): it keeps
        // the old booking id and the empty part key, inactive and with nothing allocated
        let links = f.v.refund_links(r).unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(
            (links[0].original_id, links[0].original_part.as_str()),
            (old, "")
        );
        assert!(links[0].inactive && links[0].allocated_cents == 0);
    }

    /// CodeRabbit (#1852): a twin's whole-booking link does not widen onto a bank booking of
    /// another amount; with the same amount it moves.
    #[test]
    fn twin_whole_link_moves_only_with_the_same_amount() {
        for (bank_amount, moves) in [(-100, true), (-120, false)] {
            let f = Fixture::new();
            let r = f.add("refund", 100);
            let bank = f.add("bank", bank_amount);
            let twin = f.add("twin", -100);
            f.link(r, "", twin, "");
            let db = f.v.db().unwrap();
            db.execute_batch(
                "ATTACH DATABASE ':memory:' AS other;
                CREATE TABLE other.refund_links AS SELECT * FROM main.refund_links;
                CREATE TABLE other.transactions AS SELECT * FROM main.transactions;
                CREATE TABLE other.transaction_splits AS SELECT * FROM main.transaction_splits;
                DELETE FROM main.refund_links;",
            )
            .unwrap();
            store::merge_folded(&db, &HashMap::from([(r, r), (bank, bank), (twin, bank)])).unwrap();
            crate::sync_merge::soft_delete_booking(&db, twin).unwrap();
            drop(db);
            let active =
                f.v.refund_links(r)
                    .unwrap()
                    .iter()
                    .any(|l| l.original_id == bank && !l.inactive);
            assert_eq!(active, moves, "{bank_amount}");
        }
    }

    /// CodeRabbit (#1852): in a local fold an explicit unlink follows to the keeper's part like a
    /// live link, so it still suppresses the keeper's own copy of that link.
    #[test]
    fn local_fold_moves_an_unlink_to_the_keeper_part() {
        let f = Fixture::new();
        let r = f.add("refund", 200);
        let keeper = f.add("keeper", -150);
        let dup = f.add("dup", -100);
        let parts = f.split(keeper, &[-100, -50]);
        f.link(r, "", keeper, &parts[0]);
        f.link(r, "", dup, "");
        f.unlink(r, "", dup, "");
        store::fold_booking(&f.v.db().unwrap(), dup, keeper, true, false).unwrap();
        assert!(
            f.v.refund_links(r)
                .unwrap()
                .iter()
                .all(|l| l.original_id != keeper)
        );
    }

    /// CodeRabbit (#1853): a whole-booking link moves to a replacement only with the same signed
    /// amount; a hold settled at another amount leaves it inactive.
    #[test]
    fn move_to_a_whole_booking_needs_the_same_amount() {
        for (posted, moves) in [(-100, true), (-120, false)] {
            let f = Fixture::new();
            let r = f.add("refund", 100);
            let hold = f.add("hold", -100);
            let post = f.add("post", posted);
            f.link(r, "", hold, "");
            {
                let db = f.v.db().unwrap();
                store::move_booking(&db, hold, post).unwrap();
                crate::sync_merge::soft_delete_booking(&db, hold).unwrap();
            }
            let links = f.v.refund_links(r).unwrap();
            assert_eq!(
                links.iter().any(|l| l.original_id == post && !l.inactive),
                moves,
                "{posted}"
            );
        }
    }

    /// CodeRabbit (#1852): a user unlink on the twin follows to the survivor's part, so it
    /// suppresses the older bank copy of that link instead of staying on the whole booking.
    #[test]
    fn twin_unlink_follows_to_the_survivor_part() {
        let f = Fixture::new();
        let r = f.add("refund", 200);
        let bank = f.add("bank", -150);
        let twin = f.add("twin", -100);
        let parts = f.split(bank, &[-100, -50]);
        f.link(r, "", bank, &parts[0]);
        f.link(r, "", twin, "");
        f.unlink(r, "", twin, "");
        let db = f.v.db().unwrap();
        db.execute_batch(&format!(
            "ATTACH DATABASE ':memory:' AS other;
            CREATE TABLE other.refund_links AS SELECT * FROM main.refund_links WHERE original_id = {twin};
            CREATE TABLE other.transactions AS SELECT * FROM main.transactions;
            CREATE TABLE other.transaction_splits AS SELECT * FROM main.transaction_splits;
            DELETE FROM main.refund_links WHERE original_id = {twin};"
        ))
        .unwrap();
        store::merge_folded(&db, &HashMap::from([(r, r), (bank, bank), (twin, bank)])).unwrap();
        drop(db);
        assert!(
            f.v.refund_links(r)
                .unwrap()
                .iter()
                .all(|l| l.original_id != bank)
        );
    }

    #[test]
    fn staged_inactive_whole_endpoint_does_not_tombstone_a_direct_link() {
        let f = Fixture::new();
        let r = f.add("refund", 200);
        let old = f.add("old", -100);
        let next = f.add("next", -100);
        f.link(r, "", next, "");
        f.link(r, "", old, "");
        f.split(old, &[-60, -40]);
        let db = f.v.db().unwrap();
        db.execute_batch(
            "ATTACH DATABASE ':memory:' AS other;
            CREATE TABLE other.refund_links AS SELECT * FROM main.refund_links;
            CREATE TABLE other.transactions AS SELECT * FROM main.transactions;
            CREATE TABLE other.transaction_splits AS SELECT * FROM main.transaction_splits;
            DELETE FROM main.refund_links;",
        )
        .unwrap();
        store::merge_folded(&db, &HashMap::from([(r, r), (old, next), (next, next)])).unwrap();
        drop(db);
        assert_eq!(
            f.v.refund_links(r)
                .unwrap()
                .iter()
                .map(|l| l.allocated_cents)
                .sum::<i64>(),
            100
        );
    }

    #[test]
    fn fold_does_not_reactivate_an_inactive_whole_endpoint() {
        let f = Fixture::new();
        let r = f.add("refund", 100);
        let old = f.add("old", -100);
        let next = f.add("next", -100);
        f.link(r, "", old, "");
        f.split(old, &[-60, -40]);
        store::fold_booking(&f.v.db().unwrap(), old, next, true, true).unwrap();
        // the old whole-booking link is still there (not merely "no active link"): it keeps
        // the old booking id and the empty part key, inactive and with nothing allocated
        let links = f.v.refund_links(r).unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(
            (links[0].original_id, links[0].original_part.as_str()),
            (old, "")
        );
        assert!(links[0].inactive && links[0].allocated_cents == 0);
    }

    #[test]
    fn local_fold_keeps_a_direct_link_with_older_provenance() {
        let f = Fixture::new();
        let r = f.add("refund", 200);
        let bank = f.add("bank", -100);
        let twin = f.add("twin", -100);
        let u = f.split(bank, &[-60, -40])[0].clone();
        let v = f.split(twin, &[-60, -40])[0].clone();
        f.link(r, "", bank, &u);
        f.link(r, "", twin, &v);
        let db = f.v.db().unwrap();
        db.execute("UPDATE refund_links SET moved_from_refund_id=?1,moved_from_refund_part='',moved_from_original_id=?2,moved_from_original_part='old',moved_from_key='old-key' WHERE original_id=?3",params![r,twin,bank]).unwrap();
        store::fold_booking(&db, twin, bank, true, true).unwrap();
        drop(db);
        assert_eq!(
            f.v.refund_links(r)
                .unwrap()
                .iter()
                .map(|l| l.allocated_cents)
                .sum::<i64>(),
            60
        );
    }
}

mod survivor_matching {
    use super::*;

    #[test]
    fn unlinked_removed_parts_also_make_replacement_ambiguous() {
        let f = Fixture::new();
        let r = f.add("refund", 40);
        let o = f.add("original", -100);
        let old = f.split(o, &[-40, -40, -20]);
        f.link(r, "", o, &old[0]);
        f.v.set_transaction_splits(o, vec![part(Some(old[2].clone()), -60), part(None, -40)])
            .unwrap();
        let parts = f.v.list_transaction_splits(o).unwrap();
        assert!(parts.iter().all(|p| p.uuid != old[0] && p.uuid != old[1]));
        assert!(f.v.refund_links(r).unwrap()[0].inactive);
    }

    #[test]
    fn direct_bank_link_wins_over_a_colliding_part_remap() {
        for occupied in [false, true] {
            let f = Fixture::new();
            let r = f.add("refund", 200);
            let bank = f.add("bank", -100);
            let twin = f.add("twin", -100);
            let u = f.split(bank, &[-60, -40])[0].clone();
            let v = f.split(twin, &[-60, -40])[0].clone();
            if occupied {
                f.link(r, "", bank, &u);
            }
            f.link(r, "", twin, &v);
            let db = f.v.db().unwrap();
            db.execute_batch(
                "ATTACH DATABASE ':memory:' AS other;
                CREATE TABLE other.refund_links AS SELECT * FROM main.refund_links;
                CREATE TABLE other.transactions AS SELECT * FROM main.transactions;
                CREATE TABLE other.transaction_splits AS SELECT * FROM main.transaction_splits;",
            )
            .unwrap();
            db.execute(
                "DELETE FROM other.refund_links WHERE original_id=?1",
                [bank],
            )
            .unwrap();
            db.execute("DELETE FROM main.refund_links WHERE original_id=?1", [twin])
                .unwrap();
            let before = State::load(&db).unwrap().rows;
            store::merge_folded(&db, &HashMap::from([(r, r), (twin, bank)])).unwrap();
            let state = State::load(&db).unwrap();
            let target = state
                .rows
                .iter()
                .find(|row| row.key.original == Endpoint(bank, u.clone()))
                .unwrap();
            if occupied {
                assert_eq!(target, &before[0]);
            }
            drop(db);
            assert_eq!(
                f.v.refund_links(r)
                    .unwrap()
                    .iter()
                    .map(|l| l.allocated_cents)
                    .sum::<i64>(),
                60
            );
        }
    }

    fn merge(into: &Vault, from: &Vault) {
        into.merge_other_vault(from.file_path(), vec![7; 32])
            .unwrap();
    }

    fn state(v: &Vault, refund: i64) -> Vec<(String, String, i64, bool)> {
        let mut rows = v
            .refund_links(refund)
            .unwrap()
            .into_iter()
            .map(|l| {
                let external: String = v
                    .db()
                    .unwrap()
                    .query_row(
                        "SELECT external_id FROM transactions WHERE id=?1",
                        [l.original_id],
                        |r| r.get(0),
                    )
                    .unwrap();
                (external, l.original_part, l.allocated_cents, l.inactive)
            })
            .collect::<Vec<_>>();
        rows.sort();
        rows
    }

    #[test]
    fn whole_targets_survive_twin_takeover_and_imported_part_consolidation_in_both_orders() {
        for imported_parts in [false, true] {
            for a_first in [false, true] {
                for amounts in [
                    vec![-5000, 1000, -1000],
                    vec![-5000, -5000, 5000],
                    vec![-3000, -2000],
                ] {
                    let f = Fixture::new();
                    let r = f.add("refund", 2000);
                    let o = f.add(
                        if imported_parts {
                            "id:REVIEW123456---bank"
                        } else {
                            "bank"
                        },
                        -5000,
                    );
                    f.link(r, "", o, "");
                    let target = if imported_parts {
                        for (i, amount) in amounts.iter().enumerate() {
                            let id = f.add(&format!("mm-part-{i}"), *amount);
                            raw(
                                &f.v,
                                &format!(
                                    "UPDATE transactions SET purpose='EREF: REVIEW123456' WHERE id={id}"
                                ),
                            );
                        }
                        o
                    } else {
                        let twin = f.add("mm-twin", -5000);
                        raw(
                            &f.v,
                            &format!("UPDATE transactions SET counterparty='bank' WHERE id={twin}"),
                        );
                        f.split(twin, &amounts);
                        twin
                    };
                    let peer = Fixture::new();
                    merge(&peer.v, &f.v);
                    let peer_r = number(
                        &peer.v,
                        "SELECT id FROM transactions WHERE external_id='refund'",
                    );
                    if imported_parts {
                        assert_eq!(
                            crate::importers::portal_import::merge_imported_split_parts(
                                &f.v.db().unwrap(),
                                f.account
                            )
                            .unwrap(),
                            1
                        );
                    } else {
                        assert!(
                            crate::importers::moneymoney_import::merge_synced_twin(
                                &f.v.db().unwrap(),
                                o,
                                "2026-01-01",
                                -5000,
                                "",
                                "bank",
                                ""
                            )
                            .unwrap()
                        );
                    }
                    let expected =
                        f.v.list_transaction_splits(target)
                            .unwrap()
                            .into_iter()
                            .filter(|p| p.amount_cents == -5000)
                            .collect::<Vec<_>>();
                    let expected_part = (expected.len() == 1).then(|| expected[0].uuid.clone());
                    if a_first {
                        merge(&f.v, &peer.v);
                        merge(&peer.v, &f.v);
                    } else {
                        merge(&peer.v, &f.v);
                        merge(&f.v, &peer.v);
                    }
                    let left = state(&f.v, r);
                    assert_eq!(
                        left,
                        state(&peer.v, peer_r),
                        "imported_parts={imported_parts}, a_first={a_first}, amounts={amounts:?}"
                    );
                    assert_eq!(
                        left.iter().map(|row| row.2).sum::<i64>(),
                        if expected_part.is_some() { 2000 } else { 0 }
                    );
                    for row in left.iter().filter(|row| row.2 > 0) {
                        assert_eq!(Some(&row.1), expected_part.as_ref());
                    }
                }
            }
        }
    }

    #[test]
    fn refund_parts_match_by_signed_amount_and_missing_sources_never_use_snapshots() {
        for removed in [false, true] {
            for amounts in [
                vec![3000, 2000],
                vec![1000, 2000, 2000],
                vec![2500, 2500],
                vec![-2000, 7000],
            ] {
                let f = Fixture::new();
                let o = f.add("original", -5000);
                let r = f.add("refund", 5000);
                let old = f.split(r, &[3000, 2000])[1].clone();
                f.link(r, &old, o, "");
                if removed {
                    f.v.set_transaction_splits(r, vec![]).unwrap();
                }
                let survivor = f.add("survivor", 5000);
                let uuids = f.split(survivor, &amounts);
                let matches = amounts
                    .iter()
                    .enumerate()
                    .filter(|(_, n)| **n == 2000)
                    .collect::<Vec<_>>();
                let expected =
                    (!removed && matches.len() == 1).then(|| uuids[matches[0].0].clone());
                crate::importers::portal_import::hand_over(&f.v.db().unwrap(), r, survivor)
                    .unwrap();
                let links = f.v.refund_links(o).unwrap();
                assert_eq!(links.len(), 1);
                assert_eq!(links[0].refund_id, survivor);
                assert_eq!(
                    links[0].refund_part,
                    expected.as_ref().unwrap_or(&old).clone()
                );
                assert_eq!(
                    links[0].allocated_cents,
                    if expected.is_some() { 2000 } else { 0 }
                );
            }
        }
    }

    #[test]
    fn canonical_fold_retargeting_preserves_stamps_and_later_unlinks() {
        for unlink_first in [false, true] {
            let f = Fixture::new();
            let o = f.add("source", -5000);
            let r = f.add("refund", 2000);
            let old = f.split(o, &[-3000, -2000])[1].clone();
            let target = f.add("target", -5000);
            let new = f.split(target, &[-3000, -2000])[1].clone();
            f.link(r, "", o, &old);
            let db = f.v.db().unwrap();
            let before = State::load(&db).unwrap().rows.remove(0);
            db.execute_batch(
                "ATTACH DATABASE ':memory:' AS other;
                CREATE TABLE other.refund_links AS SELECT * FROM main.refund_links;
                CREATE TABLE other.transactions AS SELECT * FROM main.transactions;
                CREATE TABLE other.transaction_splits AS SELECT * FROM main.transaction_splits;
                DELETE FROM main.refund_links;",
            )
            .unwrap();
            let map = HashMap::from([(r, r), (o, target)]);
            let unlink = |db: &Connection| {
                db.execute(
                    "UPDATE other.refund_links SET deleted_ms=?1,changed_ms=?1",
                    [before.changed_ms + 1],
                )
                .unwrap();
                store::merge_attached(db, &map).unwrap();
                db.execute(
                    "UPDATE other.refund_links SET deleted_ms=NULL,changed_ms=?1",
                    [before.changed_ms],
                )
                .unwrap();
            };
            if unlink_first {
                unlink(&db);
            }
            store::merge_folded(&db, &map).unwrap();
            let row = State::load(&db)
                .unwrap()
                .rows
                .into_iter()
                .find(|row| row.key.original.1 == new)
                .unwrap();
            assert_eq!(
                (row.created_ms, row.changed_ms),
                (before.created_ms, before.changed_ms)
            );
            assert_eq!(
                row.moved_from.as_ref().unwrap().original,
                Endpoint(target, old)
            );
            drop(db);
            if !unlink_first {
                assert_eq!(f.v.refund_links(r).unwrap()[0].allocated_cents, 2000);
                unlink(&f.v.db().unwrap());
            }
            assert!(
                f.v.refund_links(r)
                    .unwrap()
                    .iter()
                    .all(|l| l.allocated_cents == 0)
            );
        }
    }
}

mod review_round3 {
    use super::super::{Endpoint, Key, State, store};
    #[cfg(unix)]
    use crate::local_api::ShareScope;
    use crate::models::{AccountKind, TransactionSplitInput};
    use crate::offline::ManualBookingSetup;
    use crate::vault::Vault;
    #[cfg(unix)]
    use serde_json::{Value, json};
    use std::collections::HashMap;
    use std::sync::Arc;

    fn vault() -> Arc<Vault> {
        let path =
            crate::test_support::thread_temp_path(&format!("review4-{}", uuid::Uuid::new_v4()));
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
            .create_offline_account("Visible".into(), AccountKind::Cash, "EUR".into(), 0)
            .unwrap();
        let o = v.save_manual_booking(account, None, setup(-5000)).unwrap();
        let r = v.save_manual_booking(account, None, setup(2000)).unwrap();
        (v, account, o, r)
    }

    /// CodeRabbit (#1728): `link_refund` says whether it created the link, so a front end
    /// registers an undo only for its own link.
    #[test]
    fn link_reports_whether_it_created_the_link() {
        let (v, _, o, r) = seed();
        assert!(v.link_refund(r, "".into(), o, "".into()).unwrap());
        assert!(!v.link_refund(r, "".into(), o, "".into()).unwrap());
        assert_eq!(v.refund_links(r).unwrap().len(), 1);
    }

    /// CodeRabbit (#1728): an unlink based on an old view does not remove a link that was
    /// re-created since (compare-and-set on `changed_ms`).
    #[test]
    fn unlink_with_a_stale_revision_keeps_the_newer_link() {
        let (v, _, o, r) = seed();
        v.link_refund(r, "".into(), o, "".into()).unwrap();
        let seen = v.refund_links(r).unwrap()[0].changed_ms;
        v.unlink_refund(r, "".into(), o, "".into(), Some(seen))
            .unwrap();
        v.link_refund(r, "".into(), o, "".into()).unwrap();
        let current = v.refund_links(r).unwrap()[0].changed_ms;
        assert_ne!(current, seen);
        assert!(
            v.unlink_refund(r, "".into(), o, "".into(), Some(seen))
                .is_err()
        );
        assert_eq!(v.refund_links(r).unwrap().len(), 1);
        v.unlink_refund(r, "".into(), o, "".into(), Some(current))
            .unwrap();
        assert!(v.refund_links(r).unwrap().is_empty());
    }

    #[test]
    fn chained_moves_keep_later_unlinks_in_both_merge_orders() {
        for unlink_first in [false, true] {
            let (a, account, o, r) = seed();
            let next = a.save_manual_booking(account, None, setup(-5000)).unwrap();
            let last = a.save_manual_booking(account, None, setup(-5000)).unwrap();
            a.link_refund(r, "".into(), o, "".into()).unwrap();
            let b = vault();
            merge(&b, &a);
            let br = peer_id(&b, &a, r);
            let bo = peer_id(&b, &a, o);
            store::move_booking(&a.db().unwrap(), o, next).unwrap();
            store::move_booking(&a.db().unwrap(), next, last).unwrap();
            let moved = a.refund_links(r).unwrap();
            assert_eq!(moved.len(), 1);
            assert_eq!(
                (moved[0].original_id, moved[0].allocated_cents),
                (last, 2000)
            );
            b.unlink_refund(br, "".into(), bo, "".into(), None).unwrap();
            let later = store::next_stamp(&a.db().unwrap()).unwrap() + 1;
            b.db()
                .unwrap()
                .execute(
                    "UPDATE refund_links SET changed_ms=?1, deleted_ms=?1",
                    [later],
                )
                .unwrap();
            if unlink_first {
                merge(&a, &b);
                merge(&b, &a);
            } else {
                merge(&b, &a);
                merge(&a, &b);
            }
            for (v, id) in [(&a, r), (&b, br)] {
                assert!(
                    v.refund_links(id)
                        .unwrap()
                        .iter()
                        .all(|l| l.allocated_cents == 0)
                );
            }
        }
    }

    #[test]
    fn blocked_move_cannot_create_a_new_live_successor() {
        let (v, account, o, r) = seed();
        let next = v.save_manual_booking(account, None, setup(-5000)).unwrap();
        let last = v.save_manual_booking(account, None, setup(-5000)).unwrap();
        v.link_refund(r, "".into(), o, "".into()).unwrap();
        let peer = vault();
        merge(&peer, &v);
        let peer_r = peer_id(&peer, &v, r);
        let peer_o = peer_id(&peer, &v, o);
        store::move_booking(&v.db().unwrap(), o, next).unwrap();
        let moved = v.refund_links(r).unwrap();
        assert_eq!(moved.len(), 1);
        assert_eq!(
            (moved[0].original_id, moved[0].allocated_cents),
            (next, 2000)
        );
        peer.unlink_refund(peer_r, "".into(), peer_o, "".into(), None)
            .unwrap();
        let later = store::next_stamp(&v.db().unwrap()).unwrap() + 1;
        peer.db()
            .unwrap()
            .execute(
                "UPDATE refund_links SET changed_ms=?1, deleted_ms=?1",
                [later],
            )
            .unwrap();
        merge(&v, &peer);
        store::move_booking(&v.db().unwrap(), next, last).unwrap();
        assert_eq!(
            number(
                &v,
                &format!("SELECT COUNT(*) FROM refund_links WHERE original_id={last}")
            ),
            0
        );
        assert!(
            v.refund_links(r)
                .unwrap()
                .iter()
                .all(|l| l.allocated_cents == 0)
        );
    }

    #[test]
    fn provenance_rank_uses_immutable_identity_not_mapping_presence() {
        let (v, _, o, r) = seed();
        v.link_refund(r, "".into(), o, "".into()).unwrap();
        let mut lower = State::load(&v.db().unwrap()).unwrap().rows.remove(0);
        lower.moved_from = Some(lower.key.clone());
        lower.moved_from_key = "a".into();
        let mut higher = lower.clone();
        higher.moved_from_key = "z".into();
        for lower_mapped in [false, true] {
            for higher_mapped in [false, true] {
                let mut a = lower.clone();
                let mut b = higher.clone();
                if !lower_mapped {
                    a.moved_from = None;
                }
                if !higher_mapped {
                    b.moved_from = None;
                }
                assert!(b.rank() > a.rank());
            }
        }
    }

    #[test]
    fn provenance_collision_keeps_the_same_winner_with_partial_maps() {
        for higher_first in [false, true] {
            let (v, account, o, r) = seed();
            let source = v.save_manual_booking(account, None, setup(-5000)).unwrap();
            let local_o = v.save_manual_booking(account, None, setup(-5000)).unwrap();
            let local_r = v.save_manual_booking(account, None, setup(2000)).unwrap();
            v.link_refund(r, "".into(), o, "".into()).unwrap();
            let db = v.db().unwrap();
            db.execute_batch(
                "ATTACH DATABASE ':memory:' AS other;
                CREATE TABLE other.refund_links AS SELECT * FROM main.refund_links;
                UPDATE other.refund_links SET moved_from_refund_id=refund_id,
                    moved_from_refund_part='', moved_from_original_id=original_id,
                    moved_from_original_part='', moved_from_key='a';
                INSERT INTO other.refund_links SELECT * FROM other.refund_links;",
            )
            .unwrap();
            db.execute("UPDATE other.refund_links SET moved_from_key='z', moved_from_original_id=?1 WHERE rowid=?2", rusqlite::params![source, if higher_first {1} else {2}]).unwrap();
            for higher_mapped in [false, true] {
                db.execute("DELETE FROM main.refund_links", []).unwrap();
                let mut map = HashMap::from([(r, local_r), (o, local_o)]);
                if higher_mapped {
                    map.insert(source, source);
                }
                store::merge_attached(&db, &map).unwrap();
                let row = State::load(&db).unwrap().rows.remove(0);
                assert_eq!(row.moved_from_key, "z");
                assert_eq!(row.moved_from.is_some(), higher_mapped);
                map.insert(source, source);
                store::merge_attached(&db, &map).unwrap();
                assert!(State::load(&db).unwrap().rows[0].moved_from.is_some());
            }
        }
    }

    #[test]
    fn linking_a_retained_missing_uuid_fails_but_exhausted_live_links_are_idempotent() {
        let (v, _, o, r) = seed();
        v.set_transaction_splits(r, vec![part(None, 1000), part(None, 1000)])
            .unwrap();
        let uuid = parts(&v, r)[0].0.clone();
        // the first call creates the link, the repeat finds it (no second undo)
        assert!(v.link_refund(r, uuid.clone(), o, "".into()).unwrap());
        assert!(!v.link_refund(r, uuid.clone(), o, "".into()).unwrap());
        v.set_transaction_splits(r, vec![]).unwrap();
        assert!(v.link_refund(r, uuid, o, "".into()).is_err());
    }

    #[test]
    fn exhausted_candidates_remain_visible_but_not_suggested() {
        let (v, account, o, r) = seed();
        let full = v.save_manual_booking(account, None, setup(5000)).unwrap();
        v.link_refund(full, "".into(), o, "".into()).unwrap();
        let candidates = v.refund_candidates(r, "".into(), "".into(), 200).unwrap();
        assert!(
            candidates
                .iter()
                .any(|c| c.transaction_id == o && c.open_cents == 0)
        );
        assert!(
            v.refund_suggestions(r, "".into())
                .unwrap()
                .iter()
                .all(|s| s.candidate.transaction_id != o)
        );
    }

    #[test]
    fn handover_self_links_cannot_return_idempotent_success() {
        let (v, _, o, r) = seed();
        v.set_transaction_splits(o, vec![part(None, -7000), part(None, 2000)])
            .unwrap();
        let parts = parts(&v, o);
        let negative = parts.iter().find(|(_, n)| *n < 0).unwrap().0.clone();
        let positive = parts.iter().find(|(_, n)| *n > 0).unwrap().0.clone();
        assert!(
            v.link_refund(o, positive.clone(), o, negative.clone())
                .is_err()
        );
        v.link_refund(r, String::new(), o, negative.clone())
            .unwrap();
        crate::importers::portal_import::hand_over(&v.db().unwrap(), r, o).unwrap();
        let before = State::load(&v.db().unwrap()).unwrap().rows;
        assert!(v.refund_links(o).unwrap()[0].inactive);
        assert!(v.link_refund(o, positive, o, negative).is_err());
        assert_eq!(State::load(&v.db().unwrap()).unwrap().rows, before);
    }

    #[test]
    fn undo_finds_chained_successors_in_both_merge_orders_but_requires_a_live_one() {
        for unlink_first in [false, true] {
            for live in [false, true] {
                let (a, account, o, r) = seed();
                let next = a.save_manual_booking(account, None, setup(-5000)).unwrap();
                let last = a.save_manual_booking(account, None, setup(-5000)).unwrap();
                a.link_refund(r, String::new(), o, String::new()).unwrap();
                let b = vault();
                merge(&b, &a);
                let br = peer_id(&b, &a, r);
                let bo = peer_id(&b, &a, o);
                for (from, to) in [(o, next), (next, last)] {
                    crate::importers::portal_import::hand_over(&a.db().unwrap(), from, to).unwrap();
                }
                assert_eq!(a.refund_links(r).unwrap()[0].allocated_cents, 2000);
                b.unlink_refund(br, String::new(), bo, String::new(), None)
                    .unwrap();
                let token = store::next_stamp(&a.db().unwrap()).unwrap() + 1;
                b.db()
                    .unwrap()
                    .execute(
                        "UPDATE refund_links SET changed_ms=?1,deleted_ms=?1,unlink_id=CAST(?1 AS TEXT)",
                        [token],
                    )
                    .unwrap();
                if !live {
                    crate::sync_merge::soft_delete_booking(&a.db().unwrap(), last).unwrap();
                }
                if unlink_first {
                    merge(&a, &b);
                    merge(&b, &a);
                } else {
                    merge(&b, &a);
                    merge(&a, &b);
                }
                assert!(
                    a.refund_links(r)
                        .unwrap()
                        .iter()
                        .all(|l| l.allocated_cents == 0)
                );
                let before = State::load(&a.db().unwrap()).unwrap().rows;
                assert!(
                    a.restore_refund(r, String::new(), o, String::new(), format!("{token}-stale"))
                        .is_err()
                );
                assert_eq!(State::load(&a.db().unwrap()).unwrap().rows, before);
                assert_eq!(
                    a.restore_refund(r, String::new(), o, String::new(), token.to_string())
                        .is_ok(),
                    live
                );
                if live {
                    assert!(
                        a.refund_links(r)
                            .unwrap()
                            .iter()
                            .any(|l| l.original_id == last && l.allocated_cents == 2000)
                    );
                    merge(&b, &a);
                    assert_eq!(
                        b.refund_links(br)
                            .unwrap()
                            .iter()
                            .map(|l| l.allocated_cents)
                            .sum::<i64>(),
                        2000
                    );
                } else {
                    assert_eq!(State::load(&a.db().unwrap()).unwrap().rows, before);
                }
            }
        }
    }

    #[test]
    fn candidate_limits_keep_open_matches_ahead_of_many_exhausted_matches() {
        let (v, account, o, r) = seed();
        let full = v.save_manual_booking(account, None, setup(50000)).unwrap();
        v.link_refund(full, String::new(), o, String::new())
            .unwrap();
        for _ in 0..9 {
            let exhausted = v.save_manual_booking(account, None, setup(-5000)).unwrap();
            v.link_refund(full, String::new(), exhausted, String::new())
                .unwrap();
        }
        let open = v.save_manual_booking(account, None, setup(-2000)).unwrap();
        v.db()
            .unwrap()
            .execute(
                "UPDATE transactions SET booking_date='2025-12-31' WHERE id=?1",
                [open],
            )
            .unwrap();
        let rows = v
            .refund_candidates(r, String::new(), String::new(), 2)
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].transaction_id, rows[0].open_cents), (open, 2000));
        assert_eq!(rows[1].open_cents, 0);
        assert!(
            v.refund_candidates(r, String::new(), String::new(), 0)
                .unwrap()
                .is_empty()
        );
        assert!(
            v.refund_candidates(r, String::new(), "no such merchant".into(), 2)
                .unwrap()
                .is_empty()
        );
        let suggestions = v.refund_suggestions(r, String::new()).unwrap();
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].candidate.transaction_id, open);
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

    fn number(v: &Vault, query: &str) -> i64 {
        v.db().unwrap().query_row(query, [], |r| r.get(0)).unwrap()
    }

    fn sql(v: &Vault, query: &str) {
        v.db().unwrap().execute_batch(query).unwrap();
    }

    fn parts(v: &Vault, id: i64) -> Vec<(String, i64)> {
        v.list_transaction_splits(id)
            .unwrap()
            .into_iter()
            .map(|p| (p.uuid, p.amount_cents))
            .collect()
    }

    fn live(v: &Vault, r: i64) -> Vec<(String, String, i64)> {
        let mut rows: Vec<_> = v
            .refund_links(r)
            .unwrap()
            .into_iter()
            .filter(|l| !l.inactive && l.allocated_cents > 0)
            .map(|l| {
                let ext: String = v
                    .db()
                    .unwrap()
                    .query_row(
                        "SELECT external_id FROM transactions WHERE id=?1",
                        [l.original_id],
                        |r| r.get(0),
                    )
                    .unwrap();
                (
                    ext,
                    l.resolved_original_part.unwrap_or_default(),
                    l.allocated_cents,
                )
            })
            .collect();
        rows.sort();
        rows
    }

    fn sync(a: &Vault, b: &Vault, a_first: bool) {
        if a_first {
            merge(a, b);
            merge(b, a);
        } else {
            merge(b, a);
            merge(a, b);
        }
    }

    fn split_seed() -> (Arc<Vault>, i64, i64, i64, Vec<(String, i64)>) {
        let (v, account, o, r) = seed();
        v.set_transaction_splits(o, vec![part(None, -3000), part(None, -2000)])
            .unwrap();
        let p = parts(&v, o);
        v.link_refund(r, String::new(), o, p[1].0.clone()).unwrap();
        (v, account, o, r, p)
    }
    #[cfg(unix)]
    fn api(v: &Vault, method: &str, token: &str) -> Value {
        let params = if method == "summary" {
            json!({"date_from":"2026-01-01","date_to":"2026-01-31"})
        } else {
            json!({})
        };
        let response: Value = serde_json::from_str(&v.local_api_socket_request(
            &json!({"v":1,"token":token,"method":method,"params":params}).to_string(),
        ))
        .unwrap();
        assert!(response.get("error").is_none(), "{response}");
        response["result"].clone()
    }
    #[cfg(unix)]
    fn scope(v: &Vault, ids: Vec<i64>) {
        v.set_local_api_scope(ShareScope {
            enabled: true,
            account_ids: ids,
            ..Default::default()
        })
        .unwrap();
    }
    #[test]
    fn n1_sync_in_both_orders_does_not_revive_dead_link() {
        for a_first in [true, false] {
            let (a, account, o, r, p) = split_seed();
            let o3 = a.save_manual_booking(account, None, setup(-2000)).unwrap();
            let b = vault();
            merge(&b, &a);
            let br = peer_id(&b, &a, r);
            a.set_transaction_splits(
                o,
                vec![
                    part(Some(p[0].0.clone()), -3000),
                    part(None, -1000),
                    part(None, -1000),
                ],
            )
            .unwrap();
            assert!(live(&a, r).is_empty());
            a.link_refund(r, String::new(), o3, String::new()).unwrap();
            a.set_transaction_splits(
                o,
                vec![part(Some(p[0].0.clone()), -3000), part(None, -2000)],
            )
            .unwrap();
            let expected = live(&a, r);
            assert_eq!(expected.iter().map(|x| x.2).sum::<i64>(), 2000);
            assert_eq!(
                expected[0].0,
                a.db()
                    .unwrap()
                    .query_row(
                        "SELECT external_id FROM transactions WHERE id=?1",
                        [o3],
                        |r| r.get::<_, String>(0)
                    )
                    .unwrap()
            );
            sync(&a, &b, a_first);
            println!(
                "N1 sync a_first={a_first}: A={:?}; B={:?}",
                live(&a, r),
                live(&b, br)
            );
            assert_eq!(live(&a, r), expected);
            assert_eq!(live(&b, br), expected);
        }
    }

    #[test]
    fn n2_sync_in_both_orders_preserves_equal_imported_parts() {
        for a_first in [true, false] {
            let (a, account, bank, r1) = seed();
            let p1 = a.save_manual_booking(account, None, setup(-2500)).unwrap();
            let p2 = a.save_manual_booking(account, None, setup(-2500)).unwrap();
            a.save_manual_booking(account, Some(r1), setup(2500))
                .unwrap();
            let r2 = a.save_manual_booking(account, None, setup(2500)).unwrap();
            sql(
                &a,
                &format!(
                    "UPDATE transactions SET external_id='id:REVIEW123456---bank' WHERE id={bank}; UPDATE transactions SET external_id='mm-part1',purpose='EREF: REVIEW123456' WHERE id={p1}; UPDATE transactions SET external_id='mm-part2',purpose='EREF: REVIEW123456' WHERE id={p2};"
                ),
            );
            a.link_refund(r1, String::new(), p1, String::new()).unwrap();
            a.link_refund(r2, String::new(), p2, String::new()).unwrap();
            let b = vault();
            merge(&b, &a);
            let br1 = peer_id(&b, &a, r1);
            let br2 = peer_id(&b, &a, r2);
            assert_eq!(
                crate::importers::portal_import::merge_imported_split_parts(
                    &a.db().unwrap(),
                    account
                )
                .unwrap(),
                1
            );
            sync(&a, &b, a_first);
            let expected = parts(&a, bank);
            for (index, (ar, br)) in [(r1, br1), (r2, br2)].into_iter().enumerate() {
                let (aa, bb) = (live(&a, ar), live(&b, br));
                println!("N2 sync a_first={a_first}: A={aa:?}; B={bb:?}");
                assert_eq!(aa, bb);
                assert_eq!(aa.iter().map(|x| x.2).sum::<i64>(), 2500);
                assert!(
                    aa.iter()
                        .all(|row| row.0 == "id:REVIEW123456---bank" && row.1 == expected[index].0)
                );
            }
        }
    }

    #[test]
    fn r1b_concurrent_split_edits_converge_in_both_orders() {
        for a_first in [true, false] {
            let (a, _, o, r, p) = split_seed();
            let b = vault();
            merge(&b, &a);
            let bo = peer_id(&b, &a, o);
            let br = peer_id(&b, &a, r);
            a.set_transaction_splits(
                o,
                vec![part(Some(p[0].0.clone()), -3000), part(None, -2000)],
            )
            .unwrap();
            b.set_transaction_splits(
                bo,
                vec![part(Some(p[0].0.clone()), -2500), part(None, -2500)],
            )
            .unwrap();
            let stamp = number(
                &a,
                &format!("SELECT splits_ms+1000 FROM transactions WHERE id={o}"),
            );
            sql(
                &b,
                &format!("UPDATE transactions SET splits_ms={stamp} WHERE id={bo}"),
            );
            let expected = parts(&b, bo);
            sync(&a, &b, a_first);
            let (aa, bb) = (live(&a, r), live(&b, br));
            assert_eq!(parts(&a, o), expected);
            assert_eq!(parts(&b, bo), expected);
            assert!(aa.is_empty() && bb.is_empty());
            println!("R1b a_first={a_first}: A={aa:?}; B={bb:?}");
            assert_eq!(parts(&a, o), parts(&b, bo), "split unit must converge");
            assert_eq!(aa, bb, "allocation must converge");
        }
    }

    #[test]
    fn r3_moved_tombstone_cannot_undo_later_unlink_but_relink_works() {
        for a_first in [true, false] {
            let (a, account, o, r) = seed();
            let o2 = a.save_manual_booking(account, None, setup(-5000)).unwrap();
            a.link_refund(r, String::new(), o, String::new()).unwrap();
            crate::importers::portal_import::hand_over(&a.db().unwrap(), o, o2).unwrap();
            let token = "move-token";
            sql(
                &a,
                &format!(
                    "UPDATE refund_links SET unlink_id='{token}' WHERE original_id={o} AND moved=1"
                ),
            );
            assert_eq!(
                number(
                    &a,
                    &format!(
                        "SELECT COUNT(*) FROM refund_links WHERE original_id={o2} AND moved_from_original_id={o} AND moved_from_key!=''"
                    )
                ),
                1
            );
            let b = vault();
            merge(&b, &a);
            let br = peer_id(&b, &a, r);
            let bo2 = peer_id(&b, &a, o2);
            b.unlink_refund(br, String::new(), bo2, String::new(), None)
                .unwrap();
            let undo = a.restore_refund(r, String::new(), o, String::new(), token.to_string());
            assert!(
                undo.is_err(),
                "a move tombstone is not an undoable user unlink"
            );
            sync(&a, &b, a_first);
            assert!(
                live(&a, r).is_empty() && live(&b, br).is_empty(),
                "later destination unlink must survive stale source undo"
            );
            a.link_refund(r, String::new(), o2, String::new()).unwrap();
            sync(&a, &b, a_first);
            let (aa, bb) = (live(&a, r), live(&b, br));
            println!("R3 a_first={a_first}: stale undo rejected; after re-link A={aa:?}; B={bb:?}");
            assert_eq!(aa, bb);
            assert_eq!(aa.iter().map(|x| x.2).sum::<i64>(), 2000);
        }
    }

    #[test]
    #[cfg(unix)]
    fn r4_scoped_api_is_independent_of_global_cache_warm_order() {
        for global_first in [true, false] {
            let (v, visible, o, r) = seed();
            let hidden = v
                .create_offline_account(
                    "HIDDEN_ACCOUNT_SENTINEL".into(),
                    AccountKind::Cash,
                    "EUR".into(),
                    0,
                )
                .unwrap();
            let category = v
                .create_category(
                    "HIDDEN_CATEGORY_SENTINEL".into(),
                    None,
                    "tag".into(),
                    "#ff0000".into(),
                )
                .unwrap();
            let mut hidden_setup = setup(-5000);
            hidden_setup.category_id = Some(category);
            hidden_setup.counterparty = "HIDDEN_BOOKING_SENTINEL".into();
            let hidden_o = v.save_manual_booking(hidden, None, hidden_setup).unwrap();
            let token = v.create_local_api_token().unwrap();
            scope(&v, vec![visible]);
            let baseline = api(&v, "summary", &token);
            assert_eq!(baseline["totals"][0]["income_cents"], 2000);
            assert_eq!(baseline["totals"][0]["expense_cents"], 5000);
            v.link_refund(r, String::new(), hidden_o, String::new())
                .unwrap();
            if global_first {
                v.list_categories(None, None).unwrap();
            }
            assert_eq!(api(&v, "summary", &token), baseline);
            v.list_categories(None, None).unwrap();
            assert_eq!(api(&v, "summary", &token), baseline);
            let calls = if global_first {
                vec![false, true, false, true]
            } else {
                vec![true, false, true, false, true]
            };
            for scoped in calls {
                scope(&v, if scoped { vec![visible] } else { Vec::new() });
                let summary = api(&v, "summary", &token);
                v.list_categories(None, None).unwrap();
                if scoped {
                    assert_eq!(
                        summary, baseline,
                        "scoped totals/category totals changed; global_first={global_first}"
                    );
                    for method in ["summary", "accounts", "transactions"] {
                        let response = api(&v, method, &token);
                        if method == "summary" {
                            assert_eq!(
                                response, baseline,
                                "global cache warmup changed scoped totals"
                            );
                        }
                        let response = response.to_string();
                        assert!(
                            !response.contains("HIDDEN_"),
                            "hidden account/booking/category leaked via {method}: {response}"
                        );
                        assert!(
                            !response.contains("refund_cents") && !response.contains("original_id"),
                            "refund endpoint data leaked: {response}"
                        );
                    }
                } else {
                    assert_ne!(
                        summary, baseline,
                        "unscoped request must actually include hidden booking"
                    );
                }
            }
            assert!(
                number(
                    &v,
                    &format!("SELECT COUNT(*) FROM transactions WHERE id={o}")
                ) > 0
            );
            println!(
                "R4 global_first={global_first}: scoped summary/category totals unchanged: {baseline}"
            );
        }
    }

    #[test]
    fn r5_deleted_account_hold_and_brokerage_sources_remain_mappable() {
        for case in ["deleted-account", "pending-hold", "brokerage"] {
            for a_first in [true, false] {
                let (a, source_account, o, _) = seed();
                let target_account = a
                    .create_offline_account("Target".into(), AccountKind::Cash, "EUR".into(), 0)
                    .unwrap();
                let target = a
                    .save_manual_booking(target_account, None, setup(-5000))
                    .unwrap();
                let r = a
                    .save_manual_booking(target_account, None, setup(2000))
                    .unwrap();
                if case == "pending-hold" {
                    sql(
                        &a,
                        &format!(
                            "UPDATE transactions SET external_id='pending-hold-review4' WHERE id={o}"
                        ),
                    );
                }
                if case == "brokerage" {
                    sql(
                        &a,
                        &format!("UPDATE accounts SET kind='brokerage' WHERE id={source_account}"),
                    );
                }
                a.link_refund(r, String::new(), o, String::new()).unwrap();
                let b = vault();
                merge(&b, &a);
                let br = peer_id(&b, &a, r);
                crate::importers::portal_import::hand_over(&a.db().unwrap(), o, target).unwrap();
                if case == "deleted-account" {
                    a.set_account_archived(source_account, true).unwrap();
                    a.delete_account(source_account).unwrap();
                }
                let expected = live(&a, r);
                assert_eq!(expected.iter().map(|x| x.2).sum::<i64>(), 2000);
                sync(&a, &b, a_first);
                println!(
                    "R5 case={case} a_first={a_first}: A={:?}; B={:?}",
                    live(&a, r),
                    live(&b, br)
                );
                assert_eq!(live(&a, r), expected);
                assert_eq!(live(&b, br), expected);
            }
        }
    }

    #[test]
    fn merge_attached_keeps_rows_when_only_provenance_is_unmapped() {
        let (v, account, old_original, old_refund) = seed();
        let new_original = v.save_manual_booking(account, None, setup(-5000)).unwrap();
        let new_refund = v.save_manual_booking(account, None, setup(2000)).unwrap();
        let local_original = v.save_manual_booking(account, None, setup(-5000)).unwrap();
        let local_refund = v.save_manual_booking(account, None, setup(2000)).unwrap();
        v.link_refund(old_refund, String::new(), old_original, String::new())
            .unwrap();
        let db = v.db().unwrap();
        store::move_endpoints(
            &db,
            &HashMap::from([
                (
                    Endpoint(old_refund, String::new()),
                    Endpoint(new_refund, String::new()),
                ),
                (
                    Endpoint(old_original, String::new()),
                    Endpoint(new_original, String::new()),
                ),
            ]),
        )
        .unwrap();
        let expected = State::load(&db)
            .unwrap()
            .rows
            .into_iter()
            .find(|r| r.key.refund.0 == new_refund)
            .unwrap();
        assert!(expected.moved_from.is_some());
        assert!(!expected.moved_from_key.is_empty());
        db.execute_batch(
            "ATTACH DATABASE ':memory:' AS other;
        CREATE TABLE other.refund_links AS SELECT * FROM main.refund_links;",
        )
        .unwrap();
        for (map_refund, map_original) in
            [(true, true), (true, false), (false, true), (false, false)]
        {
            db.execute("DELETE FROM main.refund_links", []).unwrap();
            let mut map =
                HashMap::from([(new_refund, local_refund), (new_original, local_original)]);
            if map_refund {
                map.insert(old_refund, old_refund);
            }
            if map_original {
                map.insert(old_original, old_original);
            }
            store::merge_attached(&db, &map).unwrap();
            let rows = State::load(&db).unwrap().rows;
            let row = rows
                .iter()
                .find(|r| r.key.refund.0 == local_refund)
                .expect("mapped current endpoints survive missing provenance");
            assert_eq!(
                row.key,
                Key {
                    refund: Endpoint(local_refund, String::new()),
                    original: Endpoint(local_original, String::new())
                }
            );
            assert_eq!(
                row.moved_from,
                if map_refund && map_original {
                    expected.moved_from.clone()
                } else {
                    None
                }
            );
            assert_eq!(
                (
                    row.refund_cents,
                    row.original_cents,
                    row.created_ms,
                    row.changed_ms,
                    row.deleted_ms,
                    row.moved,
                    &row.moved_from_key
                ),
                (
                    expected.refund_cents,
                    expected.original_cents,
                    expected.created_ms,
                    expected.changed_ms,
                    expected.deleted_ms,
                    expected.moved,
                    &expected.moved_from_key
                )
            );
        }
        db.execute("DELETE FROM main.refund_links", []).unwrap();
        store::merge_attached(&db, &HashMap::from([(new_refund, local_refund)])).unwrap();
        assert!(
            State::load(&db).unwrap().rows.is_empty(),
            "unmapped current endpoints are still rejected"
        );
        db.execute("DELETE FROM other.refund_links", []).unwrap();
        store::merge_attached(&db, &HashMap::new()).unwrap();
        assert!(State::load(&db).unwrap().rows.is_empty());
        db.execute_batch("DETACH DATABASE other").unwrap();
        assert!(
            store::merge_attached(&db, &HashMap::new()).is_err(),
            "attachment errors are not silently ignored"
        );
    }
}

mod review_round2 {
    use crate::models::{AccountKind, TransactionSplitInput};
    use crate::offline::ManualBookingSetup;
    use crate::vault::Vault;
    use rusqlite::OptionalExtension;
    use std::sync::Arc;
    #[test]
    fn equal_stamp_import_moves_merge_by_immutable_source_key_not_local_ids() {
        let (base, account, o, r) = seed();
        let other = base
            .save_manual_booking(account, None, setup(-5000))
            .unwrap();
        let target = base
            .save_manual_booking(account, None, setup(-5000))
            .unwrap();
        base.link_refund(r, String::new(), o, String::new())
            .unwrap();
        let a = vault();
        let b = vault();
        merge(&a, &base);
        merge(&b, &base);
        let (ar, ao, at) = (
            peer_id(&a, &base, r),
            peer_id(&a, &base, o),
            peer_id(&a, &base, target),
        );
        let (br, bo, bt) = (
            peer_id(&b, &base, r),
            peer_id(&b, &base, o),
            peer_id(&b, &base, target),
        );
        b.unlink_refund(br, String::new(), bo, String::new(), None)
            .unwrap();
        let other = peer_id(&b, &base, other);
        b.link_refund(br, String::new(), other, String::new())
            .unwrap();
        crate::importers::portal_import::hand_over(&a.db().unwrap(), ao, at).unwrap();
        crate::importers::portal_import::hand_over(&b.db().unwrap(), other, bt).unwrap();
        for v in [&a, &b] {
            sql(
                v,
                "UPDATE refund_links SET created_ms=100,
                 changed_ms=CASE WHEN deleted_ms IS NOT NULL AND moved=0 THEN 199 ELSE 200 END,
                 deleted_ms=CASE WHEN deleted_ms IS NULL THEN NULL WHEN moved=0 THEN 199 ELSE 200 END",
            );
        }
        let source_key = |v: &Vault, target| {
            v.db().unwrap().query_row("SELECT moved_from_key FROM refund_links WHERE original_id=?1 AND deleted_ms IS NULL",[target],|r|r.get::<_,String>(0)).unwrap()
        };
        let (a_source, b_source) = (source_key(&a, at), source_key(&b, bt));
        assert_ne!(a_source, b_source);
        let expected = a_source.max(b_source);
        let left = vault();
        let right = vault();
        merge(&left, &a);
        merge(&left, &b);
        merge(&right, &b);
        merge(&right, &a);
        assert_eq!(canonical(&left), canonical(&right));
        for v in [&left, &right] {
            assert_eq!(source_key(v, peer_id(v, &base, target)), expected);
            assert_eq!(
                live(v, peer_id(v, &base, r))
                    .iter()
                    .map(|a| a.2)
                    .sum::<i64>(),
                2000
            );
        }
        assert!(!a.refund_links(ar).unwrap()[0].inactive);
    }

    #[test]
    fn replacement_inheritance_is_unique_among_removed_references_and_new_parts() {
        for (two_references, two_new) in
            [(false, false), (false, true), (true, false), (true, true)]
        {
            let (v, account, o, r) = seed();
            v.save_manual_booking(account, Some(r), setup(2500))
                .unwrap();
            v.set_transaction_splits(o, vec![part(None, -2500), part(None, -2500)])
                .unwrap();
            let old = splits(&v, o);
            v.link_refund(r, String::new(), o, old[0].0.clone())
                .unwrap();
            if two_references {
                let r2 = v.save_manual_booking(account, None, setup(2500)).unwrap();
                v.link_refund(r2, String::new(), o, old[1].0.clone())
                    .unwrap();
            }
            let before = canonical(&v);
            let amounts = if two_new {
                vec![-2500, -2500]
            } else {
                vec![-2500, -1000, -1500]
            };
            v.set_transaction_splits(o, amounts.into_iter().map(|a| part(None, a)).collect())
                .unwrap();
            let inherited = false;
            assert_eq!(splits(&v, o).iter().any(|p| p.0 == old[0].0), inherited);
            assert_eq!(!live(&v, r).is_empty(), inherited);
            assert_eq!(canonical(&v), before);
        }
        let (v, account, o, r) = seed();
        v.save_manual_booking(account, Some(r), setup(5000))
            .unwrap();
        v.set_transaction_splits(r, vec![part(None, 2500), part(None, 2500)])
            .unwrap();
        let old = splits(&v, r);
        v.link_refund(r, old[0].0.clone(), o, String::new())
            .unwrap();
        v.set_transaction_splits(
            r,
            vec![part(None, 2500), part(Some(old[1].0.clone()), 2500)],
        )
        .unwrap();
        assert_eq!(splits(&v, r)[0].0, old[0].0);
        assert_eq!(live(&v, r)[0].2, 2500);
    }

    fn canonical(v: &Vault) -> Vec<String> {
        let state = super::super::State::load(&v.db().unwrap()).unwrap();
        let key = |key: &super::super::Key| {
            (
                state.books[&key.refund.0].identity.clone(),
                key.refund.1.clone(),
                state.books[&key.original.0].identity.clone(),
                key.original.1.clone(),
            )
        };
        let mut rows: Vec<_> = state
            .rows
            .iter()
            .map(|r| {
                format!(
                    "{:?}",
                    (
                        key(&r.key),
                        r.refund_cents,
                        r.original_cents,
                        r.created_ms,
                        r.changed_ms,
                        r.deleted_ms,
                        r.moved,
                        r.moved_from.as_ref().map(key),
                        &r.moved_from_key,
                        &r.unlink_id
                    )
                )
            })
            .collect();
        rows.sort();
        rows
    }

    #[test]
    fn old_key_undo_reactivates_moved_row_but_newer_unlink_keeps_cas() {
        for a_first in [true, false] {
            let (a, account, o, r) = seed();
            let target = a.save_manual_booking(account, None, setup(-5000)).unwrap();
            a.link_refund(r, String::new(), o, String::new()).unwrap();
            let created = a.refund_links(r).unwrap()[0].created_ms;
            let b = vault();
            merge(&b, &a);
            let (br, bo) = (peer_id(&b, &a, r), peer_id(&b, &a, o));
            crate::importers::portal_import::hand_over(&a.db().unwrap(), o, target).unwrap();
            let undo = b
                .unlink_refund(br, String::new(), bo, String::new(), None)
                .unwrap();
            let token = number(&a, "SELECT MAX(changed_ms)+1000 FROM refund_links");
            sql(
                &b,
                &format!(
                    "UPDATE refund_links SET changed_ms={token},deleted_ms={token} WHERE original_id={bo}"
                ),
            );
            if a_first {
                merge(&a, &b);
                merge(&b, &a);
            } else {
                merge(&b, &a);
                merge(&a, &b);
            }
            assert!(live(&a, r).is_empty());
            b.restore_refund(br, String::new(), bo, String::new(), undo.clone())
                .unwrap();
            merge(&a, &b);
            merge(&b, &a);
            assert_eq!(live(&a, r), vec![(target, String::new(), 2000)]);
            assert_eq!(
                a.refund_links(r)
                    .unwrap()
                    .iter()
                    .find(|l| !l.inactive)
                    .unwrap()
                    .created_ms,
                created
            );
            b.unlink_refund(br, String::new(), bo, String::new(), None)
                .unwrap();
            assert!(
                b.restore_refund(br, String::new(), bo, String::new(), undo)
                    .is_err()
            );
        }
    }

    #[test]
    fn generated_moves_unlinks_and_undo_converge_with_provenance_in_both_orders() {
        for seed_value in 1..=4u64 {
            let (base, account, o, r) = seed();
            for _ in 0..5 {
                base.save_manual_booking(account, None, setup(-5000))
                    .unwrap();
            }
            base.link_refund(r, String::new(), o, String::new())
                .unwrap();
            let peers = [vault(), vault()];
            for p in &peers {
                merge(p, &base);
            }
            let mut tokens: [Vec<(i64, String, String, String)>; 2] = [Vec::new(), Vec::new()];
            let mut random = seed_value;
            for step in 0..32 {
                random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
                let i = ((random >> 32) % 2) as usize;
                let v = &peers[i];
                let rid = peer_id(v, &base, r);
                let links = v.refund_links(rid).unwrap();
                let current = links.iter().find(|l| !l.inactive);
                match (random >> 40) % 4 {
                    0 => {
                        if let Some(l) = current {
                            let target:Option<i64>=v.db().unwrap().query_row("SELECT id FROM transactions WHERE deleted_ms IS NULL AND amount_cents<0 AND id!=?1 ORDER BY id LIMIT 1",[l.original_id],|r|r.get(0)).optional().unwrap();
                            if let Some(target) = target {
                                crate::importers::portal_import::hand_over(
                                    &v.db().unwrap(),
                                    l.original_id,
                                    target,
                                )
                                .unwrap();
                            }
                        }
                    }
                    1 => {
                        if let Some(l) = current {
                            let token = v
                                .unlink_refund(
                                    rid,
                                    l.refund_part.clone(),
                                    l.original_id,
                                    l.original_part.clone(),
                                    None,
                                )
                                .unwrap();
                            tokens[i].push((
                                l.original_id,
                                l.refund_part.clone(),
                                l.original_part.clone(),
                                token,
                            ));
                        }
                    }
                    2 => {
                        if let Some((oid, rp, op, t)) = tokens[i].last() {
                            let _ = v.restore_refund(rid, rp.clone(), *oid, op.clone(), t.clone());
                        }
                    }
                    _ => {
                        if let Some(c) = v
                            .refund_candidates(rid, String::new(), String::new(), 10)
                            .unwrap()
                            .first()
                        {
                            let _ =
                                v.link_refund(rid, String::new(), c.transaction_id, c.part.clone());
                        }
                    }
                }
                if step % 4 == 3 {
                    let left = vault();
                    let right = vault();
                    merge(&left, &peers[0]);
                    merge(&left, &peers[1]);
                    merge(&right, &peers[1]);
                    merge(&right, &peers[0]);
                    assert_eq!(
                        canonical(&left),
                        canonical(&right),
                        "seed {seed_value} step {step}"
                    );
                    let allocated = |v: &Vault| {
                        live(v, peer_id(v, &base, r))
                            .into_iter()
                            .map(|(id, p, c)| {
                                (
                                    crate::sync_merge::booking_key(&v.db().unwrap(), id).unwrap(),
                                    p,
                                    c,
                                )
                            })
                            .collect::<Vec<_>>()
                    };
                    assert_eq!(allocated(&left), allocated(&right));
                    merge(&peers[0], &left);
                    merge(&peers[1], &right);
                }
            }
        }
    }
    fn vault() -> Arc<Vault> {
        let path =
            crate::test_support::thread_temp_path(&format!("review2-{}", uuid::Uuid::new_v4()));
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
        let r = v.save_manual_booking(account, None, setup(2000)).unwrap();
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
            .map(|p| (p.uuid, p.amount_cents))
            .collect()
    }
    fn amount(v: &Vault, id: i64) -> i64 {
        number(
            v,
            &format!("SELECT amount_cents FROM transactions WHERE id={id}"),
        )
    }
    fn balance(v: &Vault, account: i64) -> i64 {
        number(
            v,
            &format!("SELECT balance_cents FROM accounts WHERE id={account}"),
        )
    }
    fn live(v: &Vault, r: i64) -> Vec<(i64, String, i64)> {
        v.refund_links(r)
            .unwrap()
            .into_iter()
            .filter(|l| !l.inactive && l.allocated_cents > 0)
            .map(|l| {
                (
                    l.original_id,
                    l.resolved_original_part.unwrap_or_default(),
                    l.allocated_cents,
                )
            })
            .collect()
    }

    #[test]
    fn n1_inactive_old_part_must_not_reclaim_capacity() {
        let (v, account, o, r) = seed();
        v.set_transaction_splits(o, vec![part(None, -3000), part(None, -2000)])
            .unwrap();
        let p = splits(&v, o);
        v.link_refund(r, "".into(), o, p[1].0.clone()).unwrap();
        v.set_transaction_splits(
            o,
            vec![
                part(Some(p[0].0.clone()), -3000),
                part(None, -1000),
                part(None, -1000),
            ],
        )
        .unwrap();
        assert!(
            live(&v, r).is_empty(),
            "removed unmatched part must be inactive"
        );
        let o3 = v.save_manual_booking(account, None, setup(-2000)).unwrap();
        v.link_refund(r, "".into(), o3, "".into()).unwrap();
        v.set_transaction_splits(
            o,
            vec![part(Some(p[0].0.clone()), -3000), part(None, -2000)],
        )
        .unwrap();
        let allocations = live(&v, r);
        assert_eq!(
            allocations,
            vec![(o3, String::new(), 2000)],
            "old dropped link must not reclaim R from O3; allocations={allocations:?}"
        );
    }

    #[test]
    fn n2_equal_imported_parts_preserve_both_refunds() {
        let (v, account, bank, r1) = seed();
        let p1 = v.save_manual_booking(account, None, setup(-2500)).unwrap();
        let p2 = v.save_manual_booking(account, None, setup(-2500)).unwrap();
        v.save_manual_booking(account, Some(r1), setup(2500))
            .unwrap();
        let r2 = v.save_manual_booking(account, None, setup(2500)).unwrap();
        sql(
            &v,
            &format!(
                "UPDATE transactions SET external_id='id:REVIEW123456---bank' WHERE id={bank}; UPDATE transactions SET external_id='mm-part1',purpose='EREF: REVIEW123456' WHERE id={p1}; UPDATE transactions SET external_id='mm-part2',purpose='EREF: REVIEW123456' WHERE id={p2};"
            ),
        );
        v.link_refund(r1, "".into(), p1, "".into()).unwrap();
        v.link_refund(r2, "".into(), p2, "".into()).unwrap();
        let merged =
            crate::importers::portal_import::merge_imported_split_parts(&v.db().unwrap(), account)
                .unwrap();
        assert_eq!(
            merged, 1,
            "fixture must execute imported split consolidation"
        );
        let a = live(&v, r1);
        let b = live(&v, r2);
        let expected = splits(&v, bank);
        assert_eq!(a, vec![(bank, expected[0].0.clone(), 2500)]);
        assert_eq!(b, vec![(bank, expected[1].0.clone(), 2500)]);
        assert!(
            a.iter().map(|p| p.2).sum::<i64>() == 2500
                && b.iter().map(|p| p.2).sum::<i64>() == 2500,
            "both equal imported parts must retain allocation: R1={a:?}; R2={b:?}"
        );
    }

    #[test]
    fn n3_amount_only_sync_refreshes_offline_balance() {
        let (a, account, o, r) = seed();
        a.delete_manual_booking(r).unwrap();
        let b = vault();
        merge(&b, &a);
        let bo = peer_id(&b, &a, o);
        let ba = number(
            &b,
            &format!("SELECT account_id FROM transactions WHERE id={bo}"),
        );
        a.save_manual_booking(account, Some(o), setup(-6000))
            .unwrap();
        merge(&b, &a);
        let actual = (
            amount(&a, o),
            balance(&a, account),
            amount(&b, bo),
            balance(&b, ba),
        );
        println!("N3 (A amount,A balance,B amount,B balance)={actual:?}");
        assert_eq!(
            actual,
            (-6000, -6000, -6000, -6000),
            "amount and derived account balance must sync together: (A amount,A balance,B amount,B balance)={actual:?}"
        );
    }

    #[test]
    fn n4_mirror_correction_and_amount_edit_reach_peer() {
        let (a, _, o, _) = seed();
        let account = a
            .create_offline_account("Cash copy".into(), AccountKind::Cash, "EUR".into(), 0)
            .unwrap();
        let b = vault();
        merge(&b, &a);
        let mirrored = a.mirror_booking_to_offline_account(o, account).unwrap();
        let correction = a.correct_offline_balance(account, 4000).unwrap().unwrap();
        merge(&b, &a);
        let bm = peer_id(&b, &a, mirrored);
        let bc = peer_id(&b, &a, correction);
        assert_eq!(
            (amount(&b, bm), amount(&b, bc)),
            (5000, -1000),
            "new mirrored and correction bookings must sync even with initial zero splits_ms"
        );
        let before = number(
            &a,
            &format!("SELECT splits_ms FROM transactions WHERE id={mirrored}"),
        );
        a.save_manual_booking(account, Some(mirrored), setup(6000))
            .unwrap();
        let after = number(
            &a,
            &format!("SELECT splits_ms FROM transactions WHERE id={mirrored}"),
        );
        assert!(
            after > before,
            "editing an existing manual amount must bump splits_ms"
        );
        merge(&b, &a);
        assert_eq!(
            amount(&b, bm),
            6000,
            "amount-only edit of a mirrored cash booking must reach peer"
        );
        println!(
            "N4 mirror=5000, correction=-1000; edited mirror on peer=6000; splits_ms {before}->{after}"
        );
    }

    #[test]
    fn n5_equal_stamp_upgrade_amounts_converge() {
        let mut actual = Vec::new();
        for a_first in [true, false] {
            let (a, _, o, _) = seed();
            let b = vault();
            merge(&b, &a);
            let bo = peer_id(&b, &a, o);
            sql(
                &a,
                &format!("UPDATE transactions SET amount_cents=-6000 WHERE id={o}"),
            );
            if a_first {
                merge(&a, &b);
                merge(&b, &a);
            } else {
                merge(&b, &a);
                merge(&a, &b);
            }
            actual.push((a_first, amount(&a, o), amount(&b, bo)));
        }
        println!("N5 (a_first,A amount,B amount)={actual:?}");
        assert!(
            actual.iter().all(|(_, a, b)| *a == -5000 && *b == -5000),
            "equal stamp amount tie must converge deterministically to larger amount: {actual:?}"
        );
    }

    #[test]
    fn n6_manual_unit_merge_is_associative_with_distinct_and_equal_stamps() {
        for stamps in [[100, 200, 300], [300, 300, 300]] {
            let (base, _, o, _) = seed();
            base.set_transaction_splits(o, vec![part(None, -3000), part(None, -2000)])
                .unwrap();
            let peers: Vec<_> = (0..3)
                .map(|_| {
                    let v = vault();
                    merge(&v, &base);
                    v
                })
                .collect();
            for (i, v) in peers.iter().enumerate() {
                let id = peer_id(v, &base, o);
                let account = number(
                    v,
                    &format!("SELECT account_id FROM transactions WHERE id={id}"),
                );
                let total = -6000 - i as i64 * 1000;
                v.save_manual_booking_with_splits(
                    account,
                    Some(id),
                    setup(total),
                    Some(vec![part(None, -3000), part(None, total + 3000)]),
                )
                .unwrap();
                sql(
                    v,
                    &format!(
                        "UPDATE transactions SET splits_ms={} WHERE id={id}",
                        stamps[i]
                    ),
                );
            }
            let left = vault();
            merge(&left, &peers[0]);
            merge(&left, &peers[1]);
            merge(&left, &peers[2]);
            let bc = vault();
            merge(&bc, &peers[1]);
            merge(&bc, &peers[2]);
            let right = vault();
            merge(&right, &peers[0]);
            merge(&right, &bc);
            let reverse = vault();
            merge(&reverse, &peers[2]);
            merge(&reverse, &peers[0]);
            merge(&reverse, &peers[1]);
            let states: Vec<_> = [&left, &right, &reverse]
                .iter()
                .map(|v| {
                    let id = peer_id(v, &base, o);
                    (amount(v, id), splits(v, id))
                })
                .collect();
            println!("N6 stamps={stamps:?}; states={states:?}");
            let winner = if stamps[0] == stamps[2] {
                &peers[0]
            } else {
                &peers[2]
            };
            let id = peer_id(winner, &base, o);
            assert_eq!(states[0], (amount(winner, id), splits(winner, id)));
            assert!(
                states.windows(2).all(|p| p[0] == p[1])
                    && states
                        .iter()
                        .all(|(a, p)| p.iter().map(|p| p.1).sum::<i64>() == *a),
                "amount and split UUID unit must be associative and valid; stamps={stamps:?}; states={states:?}"
            );
        }
    }

    #[test]
    fn n7_later_unlink_wins_over_import_endpoint_remap() {
        let mut actual = Vec::new();
        for a_first in [true, false] {
            let (a, account, o, r) = seed();
            let o2 = a.save_manual_booking(account, None, setup(-5000)).unwrap();
            a.link_refund(r, "".into(), o, "".into()).unwrap();
            let b = vault();
            merge(&b, &a);
            let bo = peer_id(&b, &a, o);
            let br = peer_id(&b, &a, r);
            crate::importers::portal_import::hand_over(&a.db().unwrap(), o, o2).unwrap();
            b.unlink_refund(br, "".into(), bo, "".into(), None).unwrap();
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
            actual.push((a_first, live(&a, r), live(&b, br)));
        }
        assert!(
            actual.iter().all(|(_, a, b)| a.is_empty() && b.is_empty()),
            "later unlink of old key must win over import remap in both orders: {actual:?}"
        );
    }
}

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
    fn hand_over_retargets_only_a_unique_signed_survivor_part_in_both_merge_orders() {
        for a_first in [false, true] {
            for amounts in [
                vec![-3000, -2000],
                vec![-1000, -2000, -2000],
                vec![-2500, -2500],
                vec![2000, -7000],
                vec![],
            ] {
                let (v, account, o, r) = seed();
                v.save_manual_booking(account, Some(r), setup(2000))
                    .unwrap();
                v.set_transaction_splits(o, vec![part(None, -3000), part(None, -2000)])
                    .unwrap();
                let old = splits(&v, o)[1].0.clone();
                v.link_refund(r, String::new(), o, old.clone()).unwrap();
                let survivor = v
                    .save_manual_booking(
                        account,
                        None,
                        setup(if amounts.is_empty() { -2000 } else { -5000 }),
                    )
                    .unwrap();
                v.set_transaction_splits(
                    survivor,
                    amounts.iter().map(|a| part(None, *a)).collect(),
                )
                .unwrap();
                let matching: Vec<_> = splits(&v, survivor)
                    .into_iter()
                    .filter(|(_, n)| *n == -2000)
                    .collect();
                let expected = if matching.len() == 1 {
                    Some(matching[0].0.clone())
                } else {
                    None
                };
                let peer = vault();
                merge(&peer, &v);
                let peer_refund = peer_id(&peer, &v, r);
                let peer_survivor = peer_id(&peer, &v, survivor);
                crate::importers::portal_import::hand_over(&v.db().unwrap(), o, survivor).unwrap();
                if a_first {
                    merge(&v, &peer);
                    merge(&peer, &v);
                } else {
                    merge(&peer, &v);
                    merge(&v, &peer);
                }
                for (vault, refund, target) in
                    [(&v, r, survivor), (&peer, peer_refund, peer_survivor)]
                {
                    let links = vault.refund_links(refund).unwrap();
                    assert_eq!(links.len(), 1);
                    assert_eq!(links[0].original_id, target);
                    assert_eq!(
                        links[0].original_part,
                        expected.as_ref().unwrap_or(&old).clone()
                    );
                    assert_eq!(
                        links[0].allocated_cents,
                        if expected.is_some() { 2000 } else { 0 }
                    );
                    assert_eq!(links[0].inactive, expected.is_none());
                    let token = vault
                        .unlink_refund(
                            refund,
                            String::new(),
                            target,
                            links[0].original_part.clone(),
                            None,
                        )
                        .unwrap();
                    assert_eq!(
                        vault
                            .restore_refund(
                                refund,
                                String::new(),
                                target,
                                links[0].original_part.clone(),
                                token
                            )
                            .is_ok(),
                        expected.is_some()
                    );
                }
            }
        }
    }

    #[test]
    fn hand_over_collisions_keep_the_complete_lww_winner() {
        let (v, account, o, r) = seed();
        let survivor = v.save_manual_booking(account, None, setup(-5000)).unwrap();
        v.link_refund(r, String::new(), o, String::new()).unwrap();
        v.unlink_refund(r, String::new(), o, String::new(), None)
            .unwrap();
        v.link_refund(r, String::new(), survivor, String::new())
            .unwrap();
        let created = v.refund_links(r).unwrap()[0].created_ms;
        crate::importers::portal_import::hand_over(&v.db().unwrap(), o, survivor).unwrap();
        assert_eq!(number(&v, "SELECT COUNT(*) FROM refund_links"), 2);
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
            b.unlink_refund(br, "".into(), bo, old[1].0.clone(), None)
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
            let expected = splits(&a, o);
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
            assert_eq!((aa, ba), (-6000, -6000));
            assert_eq!(ap, expected);
            assert_eq!(bp, expected);
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
        let before = super::super::State::load(&v.db().unwrap()).unwrap().rows;
        v.delete_account(account).unwrap();
        assert_eq!(
            super::super::State::load(&v.db().unwrap()).unwrap().rows,
            before
        );
        // the stored row survives (not only "no live link"): inactive, nothing allocated
        let links = v.refund_links(r).unwrap();
        assert_eq!(links.len(), 1, "the link row must survive the deletion");
        assert!(
            links.iter().all(|l| l.inactive && l.allocated_cents == 0),
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

mod previous_pr_regressions {
    use super::*;

    #[test]
    fn qa_a_changed_split_keeps_the_link_order_of_a_collective_refund() {
        let f = Fixture::new();
        let first = f.add("first", -10000);
        let second = f.add("second", -5000);
        let refund = f.add("refund", 10000);
        let parts = f.split(first, &[-6000, -4000]);
        f.link(refund, "", first, &parts[0]);
        f.link(refund, "", second, "");
        assert_eq!(shares(&f.v, refund), [6000, 4000]);
        f.v.set_transaction_splits(
            first,
            vec![
                part(Some(parts[0].clone()), -7000),
                part(Some(parts[1].clone()), -3000),
            ],
        )
        .unwrap();
        assert_eq!(shares(&f.v, refund), [7000, 3000]);
        f.unlink(refund, "", first, &parts[0]);
        f.link(refund, "", first, &parts[0]);
        let links = f.v.refund_links(refund).unwrap();
        assert_eq!(
            links.iter().map(|l| l.original_id).collect::<Vec<_>>(),
            [second, first]
        );
        assert_eq!(shares(&f.v, refund), [5000, 5000]);
    }

    #[test]
    fn qa_vendor_window_is_62_days() {
        let f = Fixture::new();
        let refund = f.add("refund", 10000);
        let boundary = f.add("boundary", -3000);
        let too_old = f.add("too-old", -2000);
        raw(
            &f.v,
            &format!(
                "UPDATE transactions SET counterparty='QA vendor'; UPDATE transactions SET booking_date='2025-04-30' WHERE id={refund}; UPDATE transactions SET booking_date='2025-02-27' WHERE id={boundary}; UPDATE transactions SET booking_date='2025-02-26' WHERE id={too_old}"
            ),
        );
        let ids =
            f.v.refund_suggestions(refund, String::new())
                .unwrap()
                .into_iter()
                .map(|s| s.candidate.transaction_id)
                .collect::<Vec<_>>();
        assert!(ids.contains(&boundary));
        assert!(!ids.contains(&too_old));
    }

    #[test]
    fn error_texts_have_english() {
        let de = "Diese Erstattungszuordnung ist nicht möglich.";
        assert_ne!(crate::i18n::tr_in(crate::i18n::Lang::En, de), de);
    }

    #[test]
    fn part_of_a_split_income_is_suggested_by_its_amount() {
        let f = Fixture::new();
        let original = f.add("purchase", -2000);
        let income = f.add("income", 32000);
        let parts = f.split(income, &[30000, 2000]);
        raw(
            &f.v,
            &format!(
                "UPDATE transactions SET booking_date='2025-03-01' WHERE id={original}; UPDATE transactions SET booking_date='2025-03-20' WHERE id={income}"
            ),
        );
        let suggestions = f.v.refund_suggestions(original, String::new()).unwrap();
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].candidate.transaction_id, income);
        assert_eq!(suggestions[0].candidate.part, parts[1]);
        assert_eq!(suggestions[0].reason, RefundMatch::SameAmount);
        f.link(income, &parts[1], original, "");
        assert!(
            f.v.refund_suggestions(original, String::new())
                .unwrap()
                .is_empty()
        );
    }
}

struct Fixture {
    v: Arc<Vault>,
    account: i64,
}

#[test]
fn round6_deleted_twin_split_edit_cannot_take_a_deleted_canonical_uuid() {
    let a = Fixture::new();
    let old = a.add("old", -100);
    let canonical = a.add("canonical", -100);
    let parts = a.split(old, &[-40, -60]);
    let b = Fixture::new();
    merge(&b.v, &a.v);
    let bo = review_peer_id(&b.v, &a.v, old);
    let bc = review_peer_id(&b.v, &a.v, canonical);
    raw(
        &a.v,
        &format!(
            "UPDATE transaction_splits SET transaction_id={canonical} WHERE transaction_id={old}; UPDATE transactions SET deleted_ms=1000,splits_ms=200 WHERE id={canonical}; UPDATE transactions SET deleted_ms=1000,splits_ms=100 WHERE id={old}"
        ),
    );
    raw(
        &b.v,
        &format!(
            "UPDATE transactions SET deleted_ms=1000,splits_ms=300 WHERE id={bo}; UPDATE transactions SET deleted_ms=1000 WHERE id={bc}; UPDATE transaction_splits SET note='peer edit' WHERE transaction_id={bo}"
        ),
    );
    merge(&a.v, &b.v);
    assert_eq!(
        number(
            &a.v,
            &format!("SELECT COUNT(*) FROM transaction_splits WHERE transaction_id={old}")
        ),
        2
    );
    assert_eq!(
        a.v.db()
            .unwrap()
            .prepare(
                "SELECT uuid FROM transaction_splits WHERE transaction_id=?1 ORDER BY position,id"
            )
            .unwrap()
            .query_map([canonical], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap(),
        parts
    );
}

fn review_peer_id(peer: &Vault, source: &Vault, id: i64) -> i64 {
    let external: String = source
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
            [external],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn review_unlink_ids_distinguish_equal_stamp_peer_operations() {
    for reverse in [false, true] {
        let a = Fixture::new();
        let r = a.add("refund", 200);
        let o = a.add("purchase", -100);
        a.link(r, "", o, "");
        let b = Fixture::new();
        merge(&b.v, &a.v);
        let br = review_peer_id(&b.v, &a.v, r);
        let bo = review_peer_id(&b.v, &a.v, o);
        let first = a.unlink(r, "", o, "");
        b.unlink(br, "", bo, "");
        b.link(br, "", bo, "");
        let second = b.unlink(br, "", bo, "");
        assert_ne!(first, second);
        for v in [&a.v, &b.v] {
            raw(v, "UPDATE refund_links SET changed_ms=100,deleted_ms=100");
        }
        if reverse {
            merge(&b.v, &a.v);
            merge(&a.v, &b.v);
        } else {
            merge(&a.v, &b.v);
            merge(&b.v, &a.v);
        }
        assert!(
            a.v.restore_refund(r, "".into(), o, "".into(), first)
                .is_err()
        );
        a.v.restore_refund(r, "".into(), o, "".into(), second)
            .unwrap();
    }
}

#[test]
fn review_equal_stamp_unlink_blocks_a_move_in_both_orders() {
    for reverse in [false, true] {
        let a = Fixture::new();
        let r = a.add("refund", 200);
        let o = a.add("purchase", -100);
        let next = a.add("next", -100);
        a.link(r, "", o, "");
        let b = Fixture::new();
        merge(&b.v, &a.v);
        let br = review_peer_id(&b.v, &a.v, r);
        let bo = review_peer_id(&b.v, &a.v, o);
        move_booking(&a.v.db().unwrap(), o, next).unwrap();
        assert_eq!(a.v.refund_links(r).unwrap()[0].allocated_cents, 100);
        b.unlink(br, "", bo, "");
        for v in [&a.v, &b.v] {
            raw(
                v,
                "UPDATE refund_links SET changed_ms=100,deleted_ms=CASE WHEN deleted_ms IS NULL THEN NULL ELSE 100 END",
            );
        }
        if reverse {
            merge(&b.v, &a.v);
            merge(&a.v, &b.v);
        } else {
            merge(&a.v, &b.v);
            merge(&b.v, &a.v);
        }
        assert!(
            a.v.refund_links(r)
                .unwrap()
                .iter()
                .all(|l| l.allocated_cents == 0)
        );
        assert!(
            b.v.refund_links(br)
                .unwrap()
                .iter()
                .all(|l| l.allocated_cents == 0)
        );
    }
}

#[test]
fn review_moves_preserve_destination_unlinks_and_reject_collisions() {
    let f = Fixture::new();
    let r = f.add("refund", 200);
    let old = f.add("old", -100);
    let new = f.add("new", -100);
    f.link(r, "", old, "");
    f.link(r, "", new, "");
    let token = f.unlink(r, "", new, "");
    move_booking(&f.v.db().unwrap(), old, new).unwrap();
    assert!(f.v.refund_links(r).unwrap().is_empty());
    f.v.restore_refund(r, "".into(), new, "".into(), token)
        .unwrap();
    assert_eq!(f.v.refund_links(r).unwrap()[0].allocated_cents, 100);
    for fold in [false, true] {
        let f = Fixture::new();
        let r = f.add("refund", 200);
        let old = f.add("old", -200);
        let new = f.add("new", -200);
        let parts = f.split(old, &[-100, -100]);
        f.split(new, &[-100, -200, 100]);
        for p in &parts {
            f.link(r, "", old, p);
        }
        let db = f.v.db().unwrap();
        if fold {
            fold_booking(&db, old, new, true, true).unwrap();
        } else {
            move_booking(&db, old, new).unwrap();
            // as a take-over does: the source booking goes
            crate::sync_merge::soft_delete_booking(&db, old).unwrap();
        }
        drop(db);
        // colliding moves keep both links where they were: visible, inactive
        assert_eq!(f.v.refund_links(r).unwrap().len(), 2);
        if fold {
            let links = f.v.refund_links(r).unwrap();
            assert_eq!(links.len(), 2);
            assert!(
                links
                    .iter()
                    .all(|l| l.inactive && parts.contains(&l.original_part))
            );
        }
        assert!(
            f.v.refund_links(r)
                .unwrap()
                .iter()
                .all(|l| l.inactive || l.allocated_cents == 0)
        );
    }
}

#[test]
fn review_unpadded_dates_are_not_reference_tokens() {
    for date in ["1/2/2026", "2026-1-2", "01/02/2026", "2026-01-02"] {
        assert!(search::reference_tokens(date).is_empty(), "{date}");
    }
    assert!(!search::reference_tokens("INV123456").is_empty());
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
    fn unlink(&self, r: i64, rp: &str, o: i64, op: &str) -> String {
        self.v
            .unlink_refund(r, rp.into(), o, op.into(), None)
            .unwrap()
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
fn same_edit_replacement_inherits_the_referenced_uuid_without_link_writes() {
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
    assert_eq!(shares(&f.v, r), [100]);
    assert_eq!(f.v.list_transaction_splits(o).unwrap()[0].uuid, parts[0]);
    assert_eq!(f.v.refund_links(r).unwrap()[0].created_ms, created);
    assert_eq!(
        number(
            &f.v,
            "SELECT COUNT(*) FROM refund_links WHERE deleted_ms IS NOT NULL"
        ),
        0
    );
}

#[test]
fn whole_booking_markers_never_match_split_parts() {
    let f = Fixture::new();
    let (r, o) = (f.add("r", 100), f.add("o", -100));
    f.link(r, "", o, "");
    let original_created = f.v.refund_links(r).unwrap()[0].created_ms;
    f.split(o, &[-100, 20, -20]);
    let l = f.v.refund_links(r).unwrap();
    assert_eq!(l[0].original_part, "");
    assert!(l[0].resolved_original_part.is_none());
    assert_eq!(l[0].created_ms, original_created);
    f.v.set_transaction_splits(o, vec![]).unwrap();
    let l = f.v.refund_links(r).unwrap();
    assert_eq!(l[0].original_part, "");
    assert_eq!(l[0].created_ms, original_created);
    f.split(o, &[-60, -40]);
    assert!(f.v.refund_links(r).unwrap()[0].inactive);
}

#[test]
fn missing_uuid_never_borrows_an_equal_amount_part() {
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
        [0, 100]
    );
    assert!(l[0].resolved_original_part.is_none());
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
    let links = f.v.refund_links(r).unwrap();
    assert_eq!(links.len(), 1, "the link row must survive");
    assert!(links.iter().all(|l| l.inactive && l.allocated_cents == 0));
    let o2 = f.add("o2", -100);
    f.link(r, "", o2, "");
    f.v.delete_manual_booking(o2).unwrap();
    let links = f.v.refund_links(r).unwrap();
    assert_eq!(links.len(), 2, "both link rows must survive");
    assert!(links.iter().all(|l| l.inactive && l.allocated_cents == 0));
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
    let booking_state = || {
        let db = f.v.db().unwrap();
        let amount: i64 = db
            .query_row(
                "SELECT amount_cents FROM transactions WHERE id=?1",
                [m],
                |r| r.get(0),
            )
            .unwrap();
        drop(db);
        let splits = f.v.list_transaction_splits(m);
        (
            amount,
            splits
                .unwrap()
                .into_iter()
                .map(|p| (p.uuid, p.amount_cents))
                .collect::<Vec<_>>(),
            f.v.list_accounts()
                .unwrap()
                .into_iter()
                .find(|a| a.id == f.account)
                .unwrap()
                .balance_cents,
        )
    };
    let stored_before = booking_state();
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
    assert_eq!(booking_state(), stored_before);
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
    f.v.restore_refund(r, "".into(), a, "".into(), token.clone())
        .unwrap();
    assert_eq!(shares(&f.v, r), [100, 0]);
    assert_eq!(f.v.refund_links(r).unwrap()[0].created_ms, first);
    assert!(
        f.v.restore_refund(r, "".into(), a, "".into(), token.clone())
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
        .restore_refund(pr, "".into(), pa, "".into(), local_token.clone())
        .unwrap();
    peer.v
        .unlink_refund(pr, "".into(), pa, "".into(), None)
        .unwrap();
    merge(&f.v, &peer.v);
    assert!(
        f.v.restore_refund(r, "".into(), a, "".into(), local_token.clone())
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
    assert!(
        f.v.restore_refund(r, "".into(), o, "".into(), token.clone())
            .is_err()
    );
    f.v.set_transaction_splits(o, vec![]).unwrap();
    f.v.restore_refund(r, "".into(), o, "".into(), token.clone())
        .unwrap();
    assert_eq!(shares(&f.v, r), [100]);
    f.split(o, &[-60, -40]);
    assert!(
        f.v.restore_refund(r, "".into(), o, "".into(), token.clone())
            .is_err()
    );
    let token = f.v.unlink_refund(r, "".into(), o, "".into(), None).unwrap();
    assert!(
        f.v.restore_refund(r, "".into(), o, "".into(), token.clone())
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
    f.v.restore_refund(r, "".into(), z, "".into(), token.clone())
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
    f.v.restore_refund(z, String::new(), o, String::new(), token.to_string())
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

type Undo = (i64, String, i64, String, String);

fn raw_fingerprint(v: &Vault) -> Vec<String> {
    let db = v.db().unwrap();
    let state = State::load(&db).unwrap();
    let mut rows: Vec<_> = state
        .rows
        .iter()
        .map(|r| {
            let provenance = r.moved_from.as_ref().map(|key| {
                (
                    state.books[&key.refund.0].identity.clone(),
                    key.refund.1.clone(),
                    state.books[&key.original.0].identity.clone(),
                    key.original.1.clone(),
                )
            });
            format!(
                "{:?}",
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
                    r.moved,
                    (provenance, &r.moved_from_key, &r.unlink_id),
                )
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
                            None,
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
    b.v.unlink_refund(br, "".into(), bo, p[0].clone(), None)
        .unwrap();
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
        None
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
    let before = State::load(&f.v.db().unwrap()).unwrap().rows;
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
    assert_eq!(State::load(&f.v.db().unwrap()).unwrap().rows, before);
    let links = f.v.refund_links(r).unwrap();
    assert_eq!(links.len(), 1, "the link row must survive");
    assert!(links.iter().all(|l| l.inactive && l.allocated_cents == 0));
    assert_eq!(
        number(
            &f.v,
            "SELECT COUNT(*) FROM refund_links WHERE deleted_ms IS NOT NULL"
        ),
        0
    );
}
