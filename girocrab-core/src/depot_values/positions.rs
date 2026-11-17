//! Value history of the positions of a depot (#578), read from the raw observations with the
//! same day rule as the derived depot totals ([`super::observations::day_values`]).

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::NaiveDate;
use rusqlite::{params, Connection};

use super::observations::{self, day_values, position_key, source_rank, Observed, KIND_POSITION};
use crate::error::{CoreError, CoreResult};
use crate::models::DepotValuePoint;
use crate::vault::Vault;

/// A position of a depot with a value history, sold ones included.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ValuePosition {
    /// ISIN, `cash` or `name:…` (the key [`Vault::position_value_history`] takes).
    pub position: String,
    /// Latest known name.
    pub name: String,
    /// Currency of the series: the account's, else the one reported latest.
    pub currency: String,
    pub first_date: String,
    pub last_date: String,
    /// Value on `last_date` (0 = sold).
    pub last_value_cents: i64,
    /// Part of the depot's current holdings; otherwise sold or no longer reported.
    pub held: bool,
}

/// One value per day: the best source of each day ([`source_rank`]).
fn best_per_day(rows: &[Observed]) -> Vec<(String, i64)> {
    let mut days: BTreeMap<String, (u8, i64)> = BTreeMap::new();
    for d in day_values(rows) {
        let rank = source_rank(&d.source);
        match days.get(&d.date) {
            Some((r, _)) if *r <= rank => {}
            _ => {
                days.insert(d.date, (rank, d.value_cents));
            }
        }
    }
    days.into_iter().map(|(d, (_, v))| (d, v)).collect()
}

fn account_currency(conn: &Connection, account_id: i64) -> CoreResult<String> {
    conn.query_row(
        "SELECT currency FROM accounts WHERE id = ?1",
        [account_id],
        |r| r.get(0),
    )
    .map_err(|_| CoreError::NotFound {
        what: crate::i18n::trf("Konto {0}", &[&account_id]),
    })
}

/// The currency a position's series is shown in: the account's when the position has rows in
/// it, else the one reported latest (a position valued in another currency).
fn position_currency(
    conn: &Connection,
    account_id: i64,
    position: &str,
    account_currency: &str,
) -> CoreResult<Option<String>> {
    let mut q = conn.prepare_cached(
        "SELECT currency FROM depot_observations
         WHERE account_id = ?1 AND kind = 'position' AND position = ?2
         ORDER BY currency = ?3 DESC, observed_ms DESC LIMIT 1",
    )?;
    let mut rows = q.query(params![account_id, position, account_currency])?;
    Ok(match rows.next()? {
        Some(r) => Some(r.get(0)?),
        None => None,
    })
}

/// Days from `first` to `end` with the last known value carried on.
fn carried(values: &[(String, i64)], end: NaiveDate) -> Vec<DepotValuePoint> {
    let Some(first) = values
        .first()
        .and_then(|(d, _)| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
    else {
        return vec![];
    };
    let mut out = Vec::new();
    let mut next = 0;
    let mut current = 0;
    let mut day = first;
    while day <= end {
        let key = day.format("%Y-%m-%d").to_string();
        while next < values.len() && values[next].0 <= key {
            current = values[next].1;
            next += 1;
        }
        out.push(DepotValuePoint {
            date: key,
            value_cents: current,
            covered_depots: 1,
            total_depots: 1,
        });
        day += chrono::Duration::days(1);
    }
    out
}

pub(crate) fn positions(conn: &Connection, account_id: i64) -> CoreResult<Vec<ValuePosition>> {
    let own = account_currency(conn, account_id)?;
    let held: HashSet<String> = {
        let mut q = conn.prepare("SELECT isin, name FROM holdings WHERE account_id = ?1")?;
        let rows = q.query_map([account_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        rows.filter_map(|r| r.ok())
            .filter_map(|(isin, name)| position_key(&isin, &name))
            .collect()
    };
    // all position rows in one pass, grouped by (position, currency)
    let mut q = conn.prepare(
        "SELECT position, currency, value_date, source, observed_ms, value_cents, confirmed, name
         FROM depot_observations WHERE account_id = ?1 AND kind = 'position'
         ORDER BY position, currency, source, value_date, observed_ms",
    )?;
    type Group = (Vec<Observed>, i64, String);
    let mut groups: Vec<((String, String), Group)> = Vec::new();
    let mut rows = q.query([account_id])?;
    while let Some(r) = rows.next()? {
        let key: (String, String) = (r.get(0)?, r.get(1)?);
        let ms: i64 = r.get(4)?;
        let name: String = r.get(7)?;
        if groups.last().is_none_or(|g| g.0 != key) {
            groups.push((key, (Vec::new(), i64::MIN, String::new())));
        }
        let g = &mut groups.last_mut().expect("pushed above").1;
        if ms >= g.1 && !name.is_empty() {
            g.1 = ms;
            g.2 = name;
        }
        g.0.push(Observed {
            value_date: r.get(2)?,
            source: r.get(3)?,
            observed_ms: ms,
            value_cents: r.get(5)?,
            confirmed: r.get(6)?,
        });
    }
    // per position the account's currency, else the one reported latest
    let mut best: HashMap<String, ((bool, i64), ValuePosition)> = HashMap::new();
    for ((position, currency), (rows, latest_ms, name)) in groups {
        let days = best_per_day(&rows);
        let (Some(first), Some(last)) = (days.first(), days.last()) else {
            continue;
        };
        let rank = (currency == own, latest_ms);
        let p = ValuePosition {
            held: held.contains(&position),
            position: position.clone(),
            name,
            currency,
            first_date: first.0.clone(),
            last_date: last.0.clone(),
            last_value_cents: last.1,
        };
        match best.get(&position) {
            Some((r, _)) if *r >= rank => {}
            _ => {
                best.insert(position, (rank, p));
            }
        }
    }
    let mut out: Vec<ValuePosition> = best.into_values().map(|(_, p)| p).collect();
    out.sort_by_key(|p| p.name.to_lowercase());
    Ok(out)
}

pub(crate) fn history(
    conn: &Connection,
    account_id: i64,
    position: &str,
) -> CoreResult<Vec<DepotValuePoint>> {
    let own = account_currency(conn, account_id)?;
    let Some(currency) = position_currency(conn, account_id, position, &own)? else {
        return Ok(vec![]);
    };
    let rows = observations::observed(conn, account_id, KIND_POSITION, position, &currency)?;
    let days = best_per_day(&rows);
    let today = chrono::Local::now().date_naive();
    // a sold position ends with its sale, an open one carries on to today
    let end = match days.last() {
        Some((d, 0)) => NaiveDate::parse_from_str(d, "%Y-%m-%d").unwrap_or(today),
        _ => today,
    };
    Ok(carried(&days, end))
}

#[uniffi::export]
impl Vault {
    /// Positions of a depot that have a value history, sold ones included (`held` = false),
    /// sorted by name.
    pub fn depot_value_positions(&self, account_id: i64) -> Result<Vec<ValuePosition>, CoreError> {
        Ok(positions(&*self.db()?, account_id)?)
    }

    /// Daily value of one position of a depot ([`ValuePosition::position`]) from its first value
    /// on, the last value carried on (to today, or to the sale of a sold position). Empty
    /// without a history.
    pub fn position_value_history(
        &self,
        account_id: i64,
        position: String,
    ) -> Result<Vec<DepotValuePoint>, CoreError> {
        Ok(history(&*self.db()?, account_id, &position)?)
    }
}
