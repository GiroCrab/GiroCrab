//! Linked refunds netted in the statistics (#714, opt-in): with `stats.net_refunds` on, every
//! active refund link takes its allocated amount out of the sums. The refund endpoint counts
//! `amount - allocated` (a full refund counts zero), the original `amount + allocated` (a
//! refunded purchase counts only what was kept). Bookings, their list rows and balances never
//! change: only the aggregates read amounts through the SQL functions registered here.
//!
//! `refund_offset(id, part, scope)` is the offset of one endpoint (`''` = the whole booking),
//! `refund_offset_total(id, scope)` the sum over all endpoints of a booking. `scope` lists the
//! aggregate's accounts (`''` = all): a link counts only when its other booking is in there,
//! so a single account or the totals set still adds up to its own bookings. Both read the map the
//! last [`Vault::refund_offsets_ready`] built. Aggregates never call it themselves: they get
//! their SQL from [`Vault::effective_sql`] or `filter_clauses`, which bring the map up to date.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, PoisonError, RwLock};

use rusqlite::Connection;
use rusqlite::functions::FunctionFlags;

use crate::error::CoreResult;
use crate::vault::Vault;
use crate::vault::cache::DataStamp;

/// The setting (`"1"` = on). Off by default: refund links are documentation only.
pub const NET_REFUNDS_KEY: &str = "stats.net_refunds";

#[derive(Default)]
pub(crate) struct Offsets {
    /// booking id → (part, account of the linked booking, cents); `""` = whole booking
    by_booking: HashMap<i64, Vec<(String, i64, i64)>>,
}

impl Offsets {
    fn add(&mut self, id: i64, part: &str, counterpart_account: i64, cents: i64) {
        self.by_booking
            .entry(id)
            .or_default()
            .push((part.to_string(), counterpart_account, cents));
    }

    /// What `id` (one `part`, or all parts with `None`) leaves out for links whose other
    /// booking lies in `scope` (empty = every account): a link to a booking outside the
    /// selection keeps both amounts, so a single account still adds up to its bookings.
    fn sum(&self, id: i64, part: Option<&str>, scope: &Scope) -> i64 {
        self.by_booking.get(&id).map_or(0, |entries| {
            entries
                .iter()
                .filter(|(p, account, _)| {
                    part.is_none_or(|part| p == part) && scope.contains(*account)
                })
                .fold(0i64, |sum, (_, _, cents)| sum.saturating_add(*cents))
        })
    }
}

/// The accounts an aggregate covers, as the SQL literal `scope_sql` writes ("" = all).
struct Scope(Option<HashSet<i64>>);

impl Scope {
    fn parse(text: &str) -> Self {
        if text.is_empty() {
            return Scope(None);
        }
        Scope(Some(
            text.split(',').filter_map(|id| id.parse().ok()).collect(),
        ))
    }

    fn contains(&self, account: i64) -> bool {
        self.0.as_ref().is_none_or(|set| set.contains(&account))
    }
}

/// The scope argument of `refund_offset`: the aggregate's accounts (empty = all), as a SQL
/// string literal of comma-separated ids (integers only, nothing to escape).
pub(crate) fn scope_sql(accounts: &[i64]) -> String {
    let ids: Vec<String> = accounts.iter().map(i64::to_string).collect();
    format!("'{}'", ids.join(","))
}

/// The map the SQL functions read, with the (setting, data) state it was built for (`None`:
/// built inside a transaction, rebuilt on the next call).
#[derive(Default)]
pub(crate) struct OffsetSlot(Arc<RwLock<(Option<(bool, DataStamp)>, Arc<Offsets>)>>);

impl OffsetSlot {
    fn current(&self) -> Arc<Offsets> {
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .1
            .clone()
    }

    /// Registers `refund_offset(id, part, scope)` and `refund_offset_total(id, scope)` on the
    /// vault's connection. The scope is a constant per query: parsed once (auxiliary data).
    pub(crate) fn register(&self, conn: &Connection) -> rusqlite::Result<()> {
        let slot = OffsetSlot(self.0.clone());
        conn.create_scalar_function("refund_offset", 3, FunctionFlags::SQLITE_UTF8, move |ctx| {
            let id: i64 = ctx.get(0)?;
            let part: Option<String> = ctx.get(1)?;
            let scope = ctx.get_or_create_aux(2, |v| {
                Ok::<_, rusqlite::Error>(Scope::parse(v.as_str().unwrap_or("")))
            })?;
            Ok(slot
                .current()
                .sum(id, Some(part.as_deref().unwrap_or("")), &scope))
        })?;
        let slot = OffsetSlot(self.0.clone());
        conn.create_scalar_function(
            "refund_offset_total",
            2,
            FunctionFlags::SQLITE_UTF8,
            move |ctx| {
                let id: i64 = ctx.get(0)?;
                let scope = ctx.get_or_create_aux(1, |v| {
                    Ok::<_, rusqlite::Error>(Scope::parse(v.as_str().unwrap_or("")))
                })?;
                Ok(slot.current().sum(id, None, &scope))
            },
        )
    }
}

impl Vault {
    /// Whether linked refunds are netted in the statistics.
    pub(crate) fn refunds_netted(db: &Connection) -> CoreResult<bool> {
        Ok(Self::setting_inner(db, NET_REFUNDS_KEY)?.is_some_and(|v| v == "1"))
    }

    /// Brings the offsets the SQL functions read up to date with the setting and the data
    /// `db` shows. `true` when there is anything to net: otherwise the caller emits plain
    /// amounts (no function call per row). Without a live link nothing is loaded.
    pub(crate) fn refund_offsets_ready(&self, db: &Connection) -> CoreResult<bool> {
        let on = Self::refunds_netted(db)?;
        let stamp = self.data_stamp(db);
        let slot = &self.refund_offsets.0;
        {
            let current = slot.read().unwrap_or_else(PoisonError::into_inner);
            if current.0 == Some((on, stamp)) {
                return Ok(!current.1.by_booking.is_empty());
            }
        }
        let mut offsets = Offsets::default();
        let any_link = on
            && db.query_row(
                "SELECT EXISTS(SELECT 1 FROM refund_links WHERE deleted_ms IS NULL)",
                [],
                |r| r.get::<_, bool>(0),
            )?;
        if any_link {
            let resolved = self.refund_projection(db)?;
            let account = |id: i64| resolved.state.books.get(&id).map(|b| b.account_id);
            for a in &resolved.allocations {
                if let (Some(r), Some(o)) = (&a.refund, &a.original)
                    && a.cents != 0
                    && let (Some(ra), Some(oa)) = (account(r.0), account(o.0))
                {
                    offsets.add(r.0, &r.1, oa, a.cents);
                    offsets.add(o.0, &o.1, ra, a.cents.saturating_neg());
                }
            }
        }
        let netting = !offsets.by_booking.is_empty();
        let built = db.is_autocommit().then_some((on, stamp));
        *slot.write().unwrap_or_else(PoisonError::into_inner) = (built, Arc::new(offsets));
        Ok(netting)
    }
}

#[uniffi::export]
impl Vault {
    /// Whether linked refunds are netted against their original bookings in the statistics
    /// (category totals and history, budgets, KPI tiles, cash flow, tag totals). Off by default.
    pub fn net_refunds_in_stats(&self) -> Result<bool, crate::error::CoreError> {
        Self::refunds_netted(&*self.db()?)
    }

    /// Turns netting linked refunds in the statistics on or off.
    pub fn set_net_refunds_in_stats(&self, on: bool) -> Result<(), crate::error::CoreError> {
        self.set_setting(
            NET_REFUNDS_KEY.to_string(),
            if on { "1" } else { "" }.to_string(),
        )
    }
}
