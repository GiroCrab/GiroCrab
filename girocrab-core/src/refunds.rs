//! UUID-addressed refund links. Resolution and allocation never mutate bookings or splits.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};

use rusqlite::Connection;

use crate::error::{CoreError, CoreResult};
use crate::i18n::tr;
use crate::vault::Vault;

mod api;
mod search;
mod store;
#[cfg(test)]
mod tests;

pub(crate) use store::{fold_booking, merge_attached, merge_folded, move_booking, move_endpoints};

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
    fn rank(&self) -> (i64, bool, i64, i64, i64, Option<i64>, bool, &str, &str) {
        (
            self.changed_ms,
            self.deleted_ms.is_some(),
            self.created_ms,
            self.refund_cents,
            self.original_cents,
            self.deleted_ms,
            !self.moved,
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
}

impl Vault {
    pub(crate) fn refund_projection(&self, db: &Connection) -> CoreResult<Arc<Resolved>> {
        let stamp = self.data_stamp(db);
        if db.is_autocommit()
            && let Some(p) = self.refunds.get(stamp)
        {
            return Ok(p);
        }
        let p = Arc::new(Resolved::from_state(State::load(db)?)?);
        if db.is_autocommit() {
            self.refunds.put(stamp, p.clone());
        }
        Ok(p)
    }
}
