use super::store::{next_stamp, write};
use super::*;

fn key(refund: i64, refund_part: String, original: i64, original_part: String) -> Key {
    Key {
        refund: Endpoint(refund, refund_part),
        original: Endpoint(original, original_part),
    }
}

fn direct(state: &State, e: &Endpoint, positive: bool) -> CoreResult<i64> {
    let book = state
        .books
        .get(&e.0)
        .filter(|b| b.live)
        .ok_or_else(invalid)?;
    book.check_capacity()?;
    let amount = book.amount_of(&e.1).ok_or_else(invalid)?;
    if amount == 0 || (amount > 0) != positive {
        return Err(invalid());
    }
    Ok(amount)
}

fn descends_from(rows: &HashMap<&Key, &Row>, row: &Row, ancestor: &Key) -> bool {
    if row.key == *ancestor {
        return true;
    }
    let mut key = row.moved_from.as_ref();
    let mut visited = std::collections::HashSet::new();
    while let Some(current) = key {
        if current == ancestor {
            return true;
        }
        if !visited.insert(current) {
            break;
        }
        key = rows.get(current).and_then(|row| row.moved_from.as_ref());
    }
    false
}

#[uniffi::export]
impl Vault {
    /// Links a refund to an original. `true`: this call created the link; `false`: the
    /// same live link already existed (nothing written, so an undo must not unlink it).
    pub fn link_refund(
        &self,
        refund_id: i64,
        refund_part: String,
        original_id: i64,
        original_part: String,
    ) -> CoreResult<bool> {
        if refund_id == original_id {
            return Err(invalid());
        }
        let db = self.db()?;
        let tx = db.unchecked_transaction()?;
        let state = State::load(&tx)?;
        let key = key(refund_id, refund_part, original_id, original_part);
        let by_key = rows_by_key(&state.rows);
        if by_key
            .get(&key)
            .is_some_and(|r| r.deleted_ms.is_none() && !move_blocked(&by_key, r))
        {
            direct(&state, &key.refund, true)?;
            direct(&state, &key.original, false)?;
            if state.books[&refund_id].currency != state.books[&original_id].currency {
                return Err(invalid());
            }
            return Ok(false);
        }
        let refund_cents = direct(&state, &key.refund, true)?;
        let original_cents = direct(&state, &key.original, false)?;
        if state.books[&refund_id].currency != state.books[&original_id].currency {
            return Err(invalid());
        }
        let projection = Resolved::from_state(state)?;
        if projection.open(&key.refund) == 0
            || projection.open(&key.original) == 0
            || projection.allocations.iter().any(|a| {
                a.original.as_ref() == Some(&key.refund) || a.refund.as_ref() == Some(&key.original)
            })
        {
            return Err(invalid());
        }
        let ms = next_stamp(&tx)?;
        write(
            &tx,
            &Row {
                key,
                refund_cents,
                original_cents,
                created_ms: ms,
                changed_ms: ms,
                deleted_ms: None,
                moved: false,
                moved_from: None,
                moved_from_key: String::new(),
                unlink_id: String::new(),
            },
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// The returned token identifies this exact unlink, not merely the link's creation.
    /// `expected_changed_ms`: the `RefundLink::changed_ms` the caller showed; when the stored
    /// link changed or was replaced since (even with matching `changed_ms`), nothing is removed.
    pub fn unlink_refund(
        &self,
        refund_id: i64,
        refund_part: String,
        original_id: i64,
        original_part: String,
        expected_changed_ms: Option<i64>,
    ) -> CoreResult<String> {
        let db = self.db()?;
        let tx = db.unchecked_transaction()?;
        let key = key(refund_id, refund_part, original_id, original_part);
        let mut row = super::store::live_row(&tx, &key)?.ok_or_else(invalid)?;
        if let Some(ms) = expected_changed_ms
            && (ms != row.changed_ms || row.created_ms <= 0)
        {
            return Err(invalid());
        }
        let stamp = next_stamp(&tx)?;
        let token = uuid::Uuid::new_v4().to_string();
        row.unlink_id = token.clone();
        row.changed_ms = stamp;
        row.deleted_ms = Some(stamp);
        row.moved = false;
        write(&tx, &row)?;
        tx.commit()?;
        Ok(token)
    }

    /// Compare-and-set undo. Restoring retains the original allocation position.
    pub fn restore_refund(
        &self,
        refund_id: i64,
        refund_part: String,
        original_id: i64,
        original_part: String,
        token: String,
    ) -> CoreResult<()> {
        let db = self.db()?;
        let tx = db.unchecked_transaction()?;
        let key = key(refund_id, refund_part, original_id, original_part);
        let mut state = State::load(&tx)?;
        let index = state
            .rows
            .iter()
            .position(|r| {
                r.key == key
                    && r.deleted_ms.is_some()
                    && !r.moved
                    && !token.is_empty()
                    && r.unlink_id == token
            })
            .ok_or_else(invalid)?;
        {
            let by_key = rows_by_key(&state.rows);
            // judged by the revision the unlink wrote, not the one the undo would write: a newer
            // explicit unlink of an ancestor keeps blocking this link (CodeRabbit #1855)
            if move_blocked(&by_key, &state.rows[index]) {
                return Err(invalid());
            }
            // a live successor moved from this key already holds the link: restoring the
            // ancestor would allocate the one link twice, so the undo fails (CodeRabbit #1855)
            if state
                .rows
                .iter()
                .any(|r| r.deleted_ms.is_none() && r.key != key && descends_from(&by_key, r, &key))
            {
                return Err(invalid());
            }
        }
        let row = &mut state.rows[index];
        row.changed_ms = next_stamp(&tx)?;
        row.deleted_ms = None;
        let restored = row.clone();
        let by_key = rows_by_key(&state.rows);
        let resolved = resolve(&state);
        let active = |a: &Allocation| a.refund.is_some() && a.original.is_some();
        // exactly one of them may allocate: the restored link itself, or (handover) a successor
        // of it; never both for one assignment (CodeRabbit #1852)
        let own = resolved.iter().any(|a| a.row.key == key && active(a));
        let successor = resolved
            .iter()
            .any(|a| a.row.key != key && descends_from(&by_key, &a.row, &key) && active(a));
        if own == successor {
            return Err(invalid());
        }
        write(&tx, &restored)?;
        tx.commit()?;
        Ok(())
    }

    /// Live stored links in allocation order, including cheaply detected inactive links.
    /// Key fields address unlink/undo; resolved fields identify the current endpoints.
    pub fn refund_links(&self, transaction_id: i64) -> CoreResult<Vec<RefundLink>> {
        let db = self.db()?;
        let p = self.refund_projection(&db)?;
        Ok(p.allocations
            .iter()
            .filter(|a| {
                a.row.key.refund.0 == transaction_id || a.row.key.original.0 == transaction_id
            })
            .map(|a| {
                let label = |e: &Option<Endpoint>| {
                    e.as_ref()
                        .and_then(|e| p.state.books.get(&e.0).map(|b| b.label_of(&e.1)))
                        .unwrap_or_default()
                };
                RefundLink {
                    refund_id: a.row.key.refund.0,
                    refund_part: a.row.key.refund.1.clone(),
                    original_id: a.row.key.original.0,
                    original_part: a.row.key.original.1.clone(),
                    resolved_refund_part: a.refund.as_ref().map(|e| e.1.clone()),
                    resolved_original_part: a.original.as_ref().map(|e| e.1.clone()),
                    refund_label: label(&a.refund),
                    original_label: label(&a.original),
                    allocated_cents: a.cents,
                    refund_open_cents: a.refund.as_ref().map_or(0, |e| p.open(e)),
                    original_open_cents: a.original.as_ref().map_or(0, |e| p.open(e)),
                    created_ms: a.row.created_ms,
                    changed_ms: a.row.changed_ms,
                    inactive: a.refund.is_none() || a.original.is_none(),
                }
            })
            .collect())
    }
}

#[cfg(test)]
mod restore_tests {
    use super::*;
    use crate::models::AccountKind;
    use crate::offline::ManualBookingSetup;

    fn manual_setup(cents: i64) -> ManualBookingSetup {
        ManualBookingSetup {
            booking_date: "2026-01-10".into(),
            amount_cents: cents,
            counterparty: "Shop".into(),
            purpose: String::new(),
            category_id: None,
            note: String::new(),
        }
    }

    /// The plain undo of an unmoved link still works after the successor check, and only once.
    #[test]
    fn undo_restores_an_unmoved_link_once() {
        let v = Vault::open(
            crate::test_support::thread_temp_path("refund-restore-once")
                .to_string_lossy()
                .into(),
            vec![7u8; 32],
        )
        .unwrap();
        let account = v
            .create_offline_account("W".into(), AccountKind::Cash, "EUR".into(), 0)
            .unwrap();
        let o = v
            .save_manual_booking(account, None, manual_setup(-100))
            .unwrap();
        let r = v
            .save_manual_booking(account, None, manual_setup(100))
            .unwrap();
        assert!(v.link_refund(r, String::new(), o, String::new()).unwrap());
        let changed = v.refund_links(r).unwrap()[0].changed_ms;
        let token = v
            .unlink_refund(r, String::new(), o, String::new(), Some(changed))
            .unwrap();
        v.restore_refund(r, String::new(), o, String::new(), token.clone())
            .unwrap();
        assert_eq!(v.refund_links(r).unwrap()[0].allocated_cents, 100);
        assert!(
            v.restore_refund(r, String::new(), o, String::new(), token)
                .is_err()
        );
    }

    /// A 200-cent refund and two 100-cent debits; returns `[refund, first, second]`.
    fn setup(tag: &str) -> (Arc<Vault>, std::path::PathBuf, [i64; 3]) {
        let dir = std::env::temp_dir().join(format!(
            "gc-refund-restore-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let v = Vault::open(dir.join("v.db").to_string_lossy().into(), vec![6u8; 32]).unwrap();
        let ids = {
            let c = v.db().unwrap();
            c.execute_batch(
                "INSERT INTO account_groups(name) VALUES ('G');
                 INSERT INTO accounts(group_id, external_ref, name)
                     VALUES ((SELECT id FROM account_groups), 'A', 'K');",
            )
            .unwrap();
            [("r", 200), ("o1", -100), ("o2", -100)].map(|(ext, cents)| {
                c.execute(
                    "INSERT INTO transactions(account_id, external_id, booking_date, amount_cents)
                     VALUES ((SELECT id FROM accounts), ?1, '2025-01-01', ?2)",
                    rusqlite::params![ext, cents],
                )
                .unwrap();
                c.last_insert_rowid()
            })
        };
        (v, dir, ids)
    }

    /// A link row `refund → original`, optionally moved from `refund → from`.
    fn row(
        v: &Vault,
        refund: i64,
        original: i64,
        from: Option<i64>,
        changed: i64,
        deleted: Option<i64>,
        unlink: &str,
    ) {
        v.db()
            .unwrap()
            .execute(
                "INSERT INTO refund_links(refund_id,refund_part,original_id,original_part,refund_cents,
                     original_cents,created_ms,changed_ms,deleted_ms,moved,moved_from_refund_id,
                     moved_from_refund_part,moved_from_original_id,moved_from_original_part,
                     moved_from_key,unlink_id)
                 VALUES (?1,'',?2,'',200,-100,1,?3,?4,0,?5,?6,?7,?8,'',?9)",
                rusqlite::params![
                    refund,
                    original,
                    changed,
                    deleted,
                    from.map(|_| refund),
                    from.map(|_| ""),
                    from,
                    from.map(|_| ""),
                    unlink
                ],
            )
            .unwrap();
    }

    /// CodeRabbit (#1855): a newer explicit unlink of the ancestor blocks the undo of its moved
    /// successor; an older one does not.
    #[test]
    fn a_newer_ancestor_unlink_blocks_the_undo() {
        let (v, dir, [r, o1, o2]) = setup("ancestor");
        row(&v, r, o1, None, 50, Some(50), "");
        row(&v, r, o2, Some(o1), 40, Some(40), "tok");
        assert!(
            v.restore_refund(r, String::new(), o2, String::new(), "tok".into())
                .is_err()
        );
        v.db()
            .unwrap()
            .execute(
                "UPDATE refund_links SET changed_ms = 30, deleted_ms = 30 WHERE original_id = ?1",
                [o1],
            )
            .unwrap();
        v.restore_refund(r, String::new(), o2, String::new(), "tok".into())
            .unwrap();
        assert_eq!(
            v.refund_links(r)
                .unwrap()
                .iter()
                .filter(|l| !l.inactive)
                .count(),
            1
        );
        drop(v);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// CodeRabbit (#1855): with a live successor moved from the unlinked key, the undo fails
    /// instead of allocating the one link twice (the refund could cover both debits).
    #[test]
    fn a_live_successor_blocks_restoring_its_ancestor() {
        let (v, dir, [r, o1, o2]) = setup("successor");
        row(&v, r, o1, None, 40, Some(40), "tok");
        row(&v, r, o2, Some(o1), 50, None, "");
        assert!(
            v.restore_refund(r, String::new(), o1, String::new(), "tok".into())
                .is_err()
        );
        let live: Vec<_> = v
            .refund_links(r)
            .unwrap()
            .into_iter()
            .filter(|l| !l.inactive)
            .collect();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].original_id, o2);
        drop(v);
        let _ = std::fs::remove_dir_all(dir);
    }
}
