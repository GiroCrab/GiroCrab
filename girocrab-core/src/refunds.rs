//! UUID-addressed refund links. Resolution and allocation never mutate bookings or splits.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};

use rusqlite::Connection;

use crate::error::{CoreError, CoreResult};
use crate::i18n::tr;
use crate::vault::Vault;

mod api;
mod offsets;
mod search;
mod store;
#[cfg(test)]
mod tests;

pub use offsets::NET_REFUNDS_KEY;
pub(crate) use offsets::{OffsetSlot, scope_sql};
pub(crate) use store::{fold_booking, merge_attached, merge_folded, move_booking, move_endpoints};

/// One stored link between a refund and an original (docs/adr/0004-refund-links.md).
///
/// An endpoint is a booking plus a part: `""` = the whole (unsplit) booking, otherwise the
/// UUID of a split part. `refund_*`/`original_*` are the stored key, used for
/// `unlink_refund(…, expected_changed_ms: changed_ms)` and `restore_refund(…, token)`;
/// `resolved_*` are the endpoints as they are now (`None` when the part or booking is gone).
/// `allocated_cents` is the amount this link covers (allocation order: `created_ms`),
/// `*_open_cents` what is left on each endpoint. `inactive`: the link resolves to nothing
/// (deleted part, sign or currency change) and covers nothing; it stays stored.
#[derive(Debug, Clone, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RefundLink {
    pub refund_id: i64,
    pub refund_part: String,
    pub original_id: i64,
    pub original_part: String,
    pub resolved_refund_part: Option<String>,
    pub resolved_original_part: Option<String>,
    pub refund_label: String,
    pub original_label: String,
    pub allocated_cents: i64,
    pub refund_open_cents: i64,
    pub original_open_cents: i64,
    pub created_ms: i64,
    pub changed_ms: i64,
    pub inactive: bool,
}

#[derive(Debug, Clone, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RefundCandidate {
    pub transaction_id: i64,
    pub part: String,
    pub part_label: String,
    pub account_id: i64,
    pub booking_date: String,
    pub counterparty: String,
    pub merchant_name: String,
    pub amount_cents: i64,
    pub open_cents: i64,
    pub currency: String,
    pub category_id: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RefundMatch {
    SameAmount,
    SameMerchant,
    SameReference,
}

#[derive(Debug, Clone, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RefundSuggestion {
    pub candidate: RefundCandidate,
    pub reason: RefundMatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct Endpoint(pub i64, pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Key {
    refund: Endpoint,
    original: Endpoint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    key: Key,
    refund_cents: i64,
    original_cents: i64,
    created_ms: i64,
    changed_ms: i64,
    deleted_ms: Option<i64>,
    moved: bool,
    moved_from: Option<Key>,
    moved_from_key: String,
    unlink_id: String,
}

impl Row {
    /// An explicit unlink outranks a move marker of the same stamp (it must not be replaced
    /// by the marker a fold writes at that stamp).
    fn rank(&self) -> (i64, bool, bool, i64, i64, i64, Option<i64>, &str, &str) {
        (
            self.changed_ms,
            self.deleted_ms.is_some(),
            !self.moved,
            self.created_ms,
            self.refund_cents,
            self.original_cents,
            self.deleted_ms,
            &self.moved_from_key,
            &self.unlink_id,
        )
    }
}

#[derive(Clone)]
pub(crate) struct Part {
    pub uuid: String,
    pub amount: i64,
    pub category_id: Option<i64>,
    label: String,
}

#[derive(Clone)]
pub(crate) struct Book {
    live: bool,
    pub id: i64,
    pub identity: String,
    pub account_id: i64,
    pub date: String,
    pub amount: i64,
    pub currency: String,
    pub category_id: Option<i64>,
    pub parts: Vec<Part>,
    counterparty: String,
    purpose: String,
    reference: String,
    label: String,
}

impl Book {
    fn endpoints(&self) -> Vec<(Endpoint, i64)> {
        if self.parts.is_empty() {
            vec![(Endpoint(self.id, String::new()), self.amount)]
        } else {
            self.parts
                .iter()
                .map(|p| (Endpoint(self.id, p.uuid.clone()), p.amount))
                .collect()
        }
    }

    pub(crate) fn amount_of(&self, part: &str) -> Option<i64> {
        if self.parts.is_empty() {
            part.is_empty().then_some(self.amount)
        } else {
            self.parts.iter().find(|p| p.uuid == part).map(|p| p.amount)
        }
    }

    fn label_of(&self, part: &str) -> String {
        self.parts
            .iter()
            .find(|p| p.uuid == part)
            .map(|p| p.label.clone())
            .unwrap_or_else(|| self.label.clone())
    }

    fn check_capacity(&self) -> CoreResult<()> {
        let mut positive = 0i64;
        let mut negative = 0i64;
        for (_, amount) in self.endpoints() {
            let total = if amount > 0 {
                &mut positive
            } else {
                &mut negative
            };
            *total = total
                .checked_add(amount.checked_abs().ok_or_else(invalid)?)
                .ok_or_else(invalid)?;
        }
        Ok(())
    }
}

#[derive(Clone, Default)]
struct State {
    books: HashMap<i64, Book>,
    rows: Vec<Row>,
}

#[derive(Clone)]
pub(crate) struct Allocation {
    row: Row,
    pub refund: Option<Endpoint>,
    pub original: Option<Endpoint>,
    pub cents: i64,
}

#[derive(Default)]
pub(crate) struct Summary {
    pub refund_cents: i64,
    pub refunded_cents: i64,
    pub split_count: u32,
}

impl Summary {
    pub fn marker(&self) -> &'static str {
        match (self.refund_cents > 0, self.refunded_cents > 0) {
            (false, false) => "none",
            (true, false) => "refund",
            (false, true) => "original",
            (true, true) => "both",
        }
    }
}

pub(crate) struct Resolved {
    state: State,
    pub allocations: Vec<Allocation>,
    remaining: HashMap<Endpoint, i64>,
    pub summaries: HashMap<i64, Summary>,
    search: OnceLock<search::SearchIndex>,
}

fn invalid() -> CoreError {
    CoreError::invalid(tr("Diese Erstattungszuordnung ist nicht möglich."))
}

/// Link rows by key, for the move-chain walks of `move_blocked`.
fn rows_by_key(rows: &[Row]) -> HashMap<&Key, &Row> {
    rows.iter().map(|r| (&r.key, r)).collect()
}

fn move_blocked(rows: &HashMap<&Key, &Row>, row: &Row) -> bool {
    let mut key = row.moved_from.as_ref();
    let mut visited = std::collections::HashSet::new();
    while let Some(current) = key {
        if !visited.insert(current) {
            break;
        }
        let Some(source) = rows.get(current) else {
            break;
        };
        if source.deleted_ms.is_some() && !source.moved && source.changed_ms >= row.changed_ms {
            return true;
        }
        key = source.moved_from.as_ref();
    }
    false
}

/// Only exact endpoints and immutable allocation keys enter this function.
fn resolve(state: &State) -> Vec<Allocation> {
    let identity = |id| {
        state
            .books
            .get(&id)
            .map(|b| b.identity.as_str())
            .unwrap_or("")
    };
    let mut rows: Vec<_> = state
        .rows
        .iter()
        .filter(|r| r.deleted_ms.is_none())
        .collect();
    rows.sort_by(|a, b| {
        (
            a.created_ms,
            identity(a.key.refund.0),
            &a.key.refund.1,
            identity(a.key.original.0),
            &a.key.original.1,
        )
            .cmp(&(
                b.created_ms,
                identity(b.key.refund.0),
                &b.key.refund.1,
                identity(b.key.original.0),
                &b.key.original.1,
            ))
    });
    let by_key = rows_by_key(&state.rows);
    rows.into_iter()
        .map(|row| {
            let endpoint = |e: &Endpoint, positive| {
                let book = state.books.get(&e.0).filter(|b| b.live)?;
                let amount = book.amount_of(&e.1)?;
                (amount != 0 && (amount > 0) == positive && amount.checked_abs().is_some())
                    .then(|| e.clone())
            };
            let compatible = state
                .books
                .get(&row.key.refund.0)
                .zip(state.books.get(&row.key.original.0))
                .is_some_and(|(a, b)| a.currency == b.currency && a.id != b.id);
            let pair = if compatible && !move_blocked(&by_key, row) {
                endpoint(&row.key.refund, true).zip(endpoint(&row.key.original, false))
            } else {
                None
            };
            let (refund, original) = pair.map_or((None, None), |(r, o)| (Some(r), Some(o)));
            Allocation {
                row: row.clone(),
                refund,
                original,
                cents: 0,
            }
        })
        .collect()
}

impl Resolved {
    fn from_state(state: State) -> CoreResult<Self> {
        let mut remaining: HashMap<_, _> = state
            .books
            .values()
            .flat_map(Book::endpoints)
            .filter_map(|(e, amount)| amount.checked_abs().map(|c| (e, c)))
            .collect();
        let mut allocations = resolve(&state);
        let mut summaries: HashMap<_, _> = state
            .books
            .values()
            .map(|b| {
                (
                    b.id,
                    Summary {
                        split_count: b.parts.len().try_into().unwrap_or(u32::MAX),
                        ..Summary::default()
                    },
                )
            })
            .collect();
        for a in &mut allocations {
            if let (Some(r), Some(o)) = (&a.refund, &a.original) {
                a.cents = remaining[r].min(remaining[o]);
                *remaining.get_mut(r).expect("resolved endpoint") -= a.cents;
                *remaining.get_mut(o).expect("resolved endpoint") -= a.cents;
                let s = summaries.get_mut(&r.0).expect("resolved booking");
                // markers only: a sum beyond i64 saturates instead of failing every booking read
                s.refund_cents = s.refund_cents.saturating_add(a.cents);
                let s = summaries.get_mut(&o.0).expect("resolved booking");
                s.refunded_cents = s.refunded_cents.saturating_add(a.cents);
            }
        }
        Ok(Self {
            state,
            allocations,
            remaining,
            summaries,
            search: OnceLock::new(),
        })
    }

    fn open(&self, e: &Endpoint) -> i64 {
        self.remaining.get(e).copied().unwrap_or(0)
    }

    /// The summary of booking `id` from links whose both bookings sit in `visible` accounts
    /// (local interface, #136): a link to a hidden account adds nothing.
    pub(crate) fn summary_within(&self, id: i64, visible: &[i64]) -> Summary {
        let shown = |booking: i64| {
            self.state
                .books
                .get(&booking)
                .is_some_and(|b| visible.contains(&b.account_id))
        };
        let mut s = Summary {
            split_count: self.summaries.get(&id).map_or(0, |s| s.split_count),
            ..Summary::default()
        };
        for a in &self.allocations {
            let (Some(r), Some(o)) = (&a.refund, &a.original) else {
                continue;
            };
            if (r.0 != id && o.0 != id) || !shown(r.0) || !shown(o.0) {
                continue;
            }
            if r.0 == id {
                s.refund_cents = s.refund_cents.saturating_add(a.cents);
            }
            if o.0 == id {
                s.refunded_cents = s.refunded_cents.saturating_add(a.cents);
            }
        }
        s
    }
}

impl Vault {
    /// What the booking rows show (markers, allocated cents, split counts). Without any link
    /// row only the split counts are read, not the whole ledger after every write.
    pub(crate) fn refund_markers(&self, db: &Connection) -> CoreResult<Arc<Resolved>> {
        // tombstones (unlinks, move markers) allocate nothing: only a live link needs the full
        // projection (which still loads the tombstones, so an ancestor unlink keeps blocking
        // its successor)
        let any_link: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM refund_links WHERE deleted_ms IS NULL)",
            [],
            |r| r.get(0),
        )?;
        if any_link {
            return self.refund_projection(db);
        }
        let mut summaries = HashMap::new();
        let mut stmt = db.prepare(
            "SELECT transaction_id, count(*) FROM transaction_splits GROUP BY transaction_id",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            summaries.insert(
                r.get::<_, i64>(0)?,
                Summary {
                    split_count: r.get::<_, i64>(1)?.try_into().unwrap_or(u32::MAX),
                    ..Summary::default()
                },
            );
        }
        Ok(Arc::new(Resolved {
            state: State::default(),
            allocations: Vec::new(),
            remaining: HashMap::new(),
            summaries,
            search: OnceLock::new(),
        }))
    }

    pub(crate) fn refund_projection(&self, db: &Connection) -> CoreResult<Arc<Resolved>> {
        let stamp = self.data_stamp(db);
        if db.is_autocommit()
            && let Some(p) = self.refunds.get(stamp)
        {
            return Ok(p);
        }
        // bookings, parts and links from one snapshot (a merge committing in between could
        // otherwise pair new links with old bookings)
        let state = if db.is_autocommit() {
            let tx = db.unchecked_transaction()?;
            let state = State::load(&tx)?;
            tx.commit()?;
            state
        } else {
            State::load(db)?
        };
        let p = Arc::new(Resolved::from_state(state)?);
        if db.is_autocommit() {
            self.refunds.put(stamp, p.clone());
        }
        Ok(p)
    }
}

#[cfg(test)]
mod scope_tests {
    use super::*;

    fn book(id: i64, account_id: i64, amount: i64) -> Book {
        Book {
            live: true,
            id,
            identity: format!("{account_id}/{id}"),
            account_id,
            date: "2026-01-01".into(),
            amount,
            currency: "EUR".into(),
            category_id: None,
            parts: Vec::new(),
            counterparty: String::new(),
            purpose: String::new(),
            reference: String::new(),
            label: String::new(),
        }
    }

    /// CodeRabbit (#1852): a refund on a hidden account leaves the markers of a visible
    /// booking untouched, in either direction; links between visible bookings still show.
    #[test]
    fn scoped_markers_ignore_links_to_hidden_accounts() {
        let row = Row {
            key: Key {
                refund: Endpoint(1, String::new()),
                original: Endpoint(2, String::new()),
            },
            refund_cents: 100,
            original_cents: 100,
            created_ms: 1,
            changed_ms: 1,
            deleted_ms: None,
            moved: false,
            moved_from: None,
            moved_from_key: String::new(),
            unlink_id: String::new(),
        };
        let state = State {
            books: [book(1, 10, 100), book(2, 20, -100)]
                .into_iter()
                .map(|b| (b.id, b))
                .collect(),
            rows: vec![row],
        };
        let p = Resolved::from_state(state).unwrap();
        assert_eq!(p.summary_within(2, &[10, 20]).refunded_cents, 100);
        assert_eq!(p.summary_within(1, &[10, 20]).marker(), "refund");
        let hidden_refund = p.summary_within(2, &[20]);
        assert_eq!(
            (hidden_refund.marker(), hidden_refund.refunded_cents),
            ("none", 0)
        );
        let hidden_original = p.summary_within(1, &[10]);
        assert_eq!(
            (hidden_original.marker(), hidden_original.refund_cents),
            ("none", 0)
        );
    }
}

#[cfg(test)]
mod marker_probe_tests {
    use super::*;

    /// CodeRabbit (#1855): only unlink tombstones left: the markers skip the full ledger load.
    #[test]
    fn only_tombstones_skip_the_full_projection() {
        let dir = std::env::temp_dir().join(format!(
            "gc-refund-probe-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let v = Vault::open(dir.join("v.db").to_string_lossy().into(), vec![5u8; 32]).unwrap();
        {
            let c = v.db().unwrap();
            c.execute_batch(
                "INSERT INTO account_groups(name) VALUES ('G');
                 INSERT INTO accounts(group_id, external_ref, name)
                     VALUES ((SELECT id FROM account_groups), 'A', 'K');
                 INSERT INTO transactions(account_id, external_id, booking_date, amount_cents)
                     VALUES ((SELECT id FROM accounts), 'r', '2025-01-01', 100),
                            ((SELECT id FROM accounts), 'o', '2025-01-01', -100);
                 INSERT INTO refund_links(refund_id, refund_part, original_id, original_part,
                     refund_cents, original_cents, created_ms, changed_ms, deleted_ms)
                     VALUES ((SELECT id FROM transactions WHERE external_id = 'r'), '',
                             (SELECT id FROM transactions WHERE external_id = 'o'), '',
                             100, -100, 1, 2, 2);",
            )
            .unwrap();
            assert!(v.refund_markers(&c).unwrap().state.books.is_empty());
            c.execute("UPDATE refund_links SET deleted_ms = NULL", [])
                .unwrap();
            let p = v.refund_markers(&c).unwrap();
            assert!(!p.state.books.is_empty());
            assert!(p.allocations.iter().any(|a| a.cents == 100));
        }
        drop(v);
        let _ = std::fs::remove_dir_all(dir);
    }
}
