//! Valuation history of securities depots: one value per depot and day.
//!
//! A depot has no cash bookings to walk back over, and the bank reports only today's valuation.
//! So each fetch leaves a point: the depot value the bank reported, stamped with its valuation
//! date. The history therefore starts with the first fetch (or an imported history) and never
//! claims a past it did not see: before its first point a depot's value is **unknown** (it is not
//! 0 and not the first value). A confirmed value of 0 (a depot that was sold off) is a point like
//! any other. Between two points the last value carries on (no interpolation). Values are the
//! reported depot value, so a change in them may also come from purchases, sales and transfers:
//! it is no return. A sum over several depots says per day how many of them were covered.
//!
//! Two layers (#578): [`observations`] keeps what a fetch or an import delivered
//! (`depot_observations`, totals and positions, merged between devices as a union).
//! `depot_values` is derived from it on each device by [`rebuild`]: one value per depot, day and
//! source, which [`series`] and the charts read.

use std::collections::HashMap;

use chrono::NaiveDate;
use rusqlite::{params, Connection, OptionalExtension};

use crate::error::CoreResult;

pub mod observations;
pub(crate) mod positions;

/// Step 28. `depot_observations` is the raw layer ([`observations`]): one row per value a
/// fetch (`source = 'fetch'`) or an import (`'import'`) delivered, for the depot total or a
/// position, never edited. It is stored in the order its readers walk it (one series, by
/// source, day and time), so a series or all positions of a depot are one range scan. `depot_values` is derived from it ([`rebuild`]) and not merged: per
/// depot, valuation day and source one value with a quality (`observed`, `reconstructed`,
/// `estimated`; `prices` and the last two are reserved for computed values), and the
/// observation it came from (`observed_ms`). Several sources of a day live side by side;
/// [`series`] shows the best one. Both disappear with their account (`ON DELETE CASCADE`).
pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS depot_observations (
    account_id   INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    kind         TEXT    NOT NULL,             -- total | position
    position     TEXT    NOT NULL DEFAULT '',  -- '' (total) | ISIN | 'cash' | 'name:<normalised>'
    currency     TEXT    NOT NULL,
    source       TEXT    NOT NULL,             -- fetch | import
    value_date   TEXT    NOT NULL,             -- YYYY-MM-DD, the bank's valuation day
    observed_ms  INTEGER NOT NULL,             -- retrieval time; import: the source's timestamp
    source_ref   TEXT    NOT NULL DEFAULT '',  -- import: the source's row of the position
    date_assumed INTEGER NOT NULL DEFAULT 0,   -- 1 = no valuation date, the retrieval day
    value_cents  INTEGER NOT NULL,
    confirmed    INTEGER NOT NULL DEFAULT 0,   -- 1 = the inventory was delivered: a 0 is real
    quantity     REAL,                         -- fetch only
    price_cents  INTEGER,                      -- fetch only
    name         TEXT    NOT NULL DEFAULT '',  -- name of the position at that time
    PRIMARY KEY (account_id, kind, position, currency, source, value_date, observed_ms, source_ref)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS depot_values (
    account_id  INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    value_date  TEXT    NOT NULL,                    -- YYYY-MM-DD, the valuation day
    source      TEXT    NOT NULL DEFAULT 'fetch',    -- fetch | import | prices
    quality     TEXT    NOT NULL DEFAULT 'observed', -- observed | reconstructed | estimated
    value_cents INTEGER NOT NULL,
    observed_ms INTEGER NOT NULL DEFAULT 0,          -- the observation it came from
    PRIMARY KEY (account_id, value_date, source)
) WITHOUT ROWID;
";

pub(crate) const QUALITY_OBSERVED: &str = "observed";

fn is_iso_date(s: &str) -> bool {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

/// The currency of a securities depot whose values are derived, `None` for any other account
/// (a Sachwerte account has its own value series).
fn depot_currency(conn: &Connection, account_id: i64) -> CoreResult<Option<String>> {
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT kind, currency FROM accounts WHERE id = ?1 AND deleted_ms IS NULL",
            [account_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(match row {
        Some((kind, currency))
            if kind == "brokerage" && !crate::assets::is_tangible(conn, account_id)? =>
        {
            Some(currency)
        }
        _ => None,
    })
}

/// Derives `depot_values` of a depot from its total observations in the account currency
/// (values in another currency stay out): all days, or only `fetch_day` of the fetch source
/// (a fetch needs no more: its rule looks at one day only, see [`observations::day_values`]).
pub(crate) fn rebuild(
    conn: &Connection,
    account_id: i64,
    fetch_day: Option<&str>,
) -> CoreResult<()> {
    let Some(currency) = depot_currency(conn, account_id)? else {
        conn.execute(
            "DELETE FROM depot_values WHERE account_id = ?1",
            [account_id],
        )?;
        return Ok(());
    };
    let mut rows =
        observations::observed(conn, account_id, observations::KIND_TOTAL, "", &currency)?;
    match fetch_day {
        Some(day) => {
            let key = observations::day_key(day);
            rows.retain(|o| o.source == observations::SOURCE_FETCH && o.day == key);
            conn.execute(
                "DELETE FROM depot_values WHERE account_id = ?1 AND value_date = ?2 AND source = 'fetch'",
                params![account_id, day],
            )?;
        }
        None => {
            conn.execute(
                "DELETE FROM depot_values WHERE account_id = ?1",
                [account_id],
            )?;
        }
    }
    let mut ins = conn.prepare_cached(
        "INSERT OR REPLACE INTO depot_values(account_id, value_date, source, quality, value_cents, observed_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for d in observations::day_values(&rows) {
        ins.execute(params![
            account_id,
            observations::day_text(d.day),
            d.source,
            QUALITY_OBSERVED,
            d.value_cents,
            d.observed_ms
        ])?;
    }
    Ok(())
}

/// Records what a fetch reported for a depot and derives that day (see
/// [`observations::record_fetch`]).
pub(crate) fn record_fetch(
    conn: &Connection,
    account_id: i64,
    holdings: Option<&[&crate::models::Holding]>,
    observed_ms: i64,
) -> CoreResult<()> {
    if let Some(day) = observations::record_fetch(conn, account_id, holdings, observed_ms)? {
        rebuild(conn, account_id, Some(&day))?;
    }
    Ok(())
}

/// Vault merge: adds the other vault's observations of mapped accounts and derives the depots
/// that gained some again.
pub(crate) fn merge_attached(conn: &Connection, acc_map: &HashMap<i64, i64>) -> CoreResult<()> {
    for id in observations::merge_attached(conn, acc_map)? {
        rebuild(conn, id, None)?;
    }
    Ok(())
}

/// Per-day values of the depots among `ids` that have a history, summed, for `start..=end`.
/// `None` when none has a point. A depot counts from its first point on; before that it adds
/// nothing and is not counted as covered.
pub(crate) struct Series {
    /// Sum of the known depot values per day (depots not yet covered add nothing).
    pub values: Vec<i64>,
    /// Per day: how many depots had a value (a confirmed 0 counts).
    pub covered: Vec<u32>,
    /// Securities depots among `ids`, with or without a history. The chart reads it next to
    /// `covered` ("2 of 3 depots") in `Vault::depot_value_history`.
    pub total_depots: u32,
    /// The depots with at least one point: the caller leaves them out of its own walk back over
    /// bookings.
    pub accounts: Vec<i64>,
}

pub(crate) fn series(
    conn: &Connection,
    ids: &[i64],
    start: NaiveDate,
    end: NaiveDate,
) -> CoreResult<Option<Series>> {
    if ids.is_empty() || end < start {
        return Ok(None);
    }
    let days = (end - start).num_days() as usize + 1;
    let mut total = vec![0_i64; days];
    let mut covered_days = vec![0_u32; days];
    let mut with_points = Vec::new();
    let mut total_depots = 0_u32;
    let mut q = conn.prepare(
        "SELECT value_date, value_cents FROM depot_values
         WHERE account_id = ?1 AND value_date <= ?2
         ORDER BY value_date,
                  CASE quality WHEN 'observed' THEN 0 WHEN 'reconstructed' THEN 1 ELSE 2 END,
                  CASE source WHEN 'fetch' THEN 0 WHEN 'import' THEN 1 ELSE 2 END",
    )?;
    let last = end.format("%Y-%m-%d").to_string();
    for &id in ids {
        let kind: Option<String> = conn
            .query_row("SELECT kind FROM accounts WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .ok();
        if kind.as_deref() == Some("brokerage") && !crate::assets::is_tangible(conn, id)? {
            total_depots += 1;
        }
        let mut points: Vec<(String, i64)> = q
            .query_map(params![id, last], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        // the best source of a day comes first (see the ORDER BY)
        points.dedup_by(|later, first| later.0 == first.0);
        if points.is_empty() {
            // A depot that was never recorded (a vault from before the history) is known from its
            // reported balance on: one value on its valuation day, unknown before it. Its balance
            // is never carried backwards over the days it was not seen.
            match reported_point(conn, id, end)? {
                Some(point) => points.push(point),
                None => continue,
            }
        }
        with_points.push(id);
        let mut next = 0;
        let mut current: Option<i64> = None;
        for i in 0..days {
            let day = (start + chrono::Duration::days(i as i64))
                .format("%Y-%m-%d")
                .to_string();
            while next < points.len() && points[next].0 <= day {
                current = Some(points[next].1);
                next += 1;
            }
            if let Some(v) = current {
                total[i] += v;
                covered_days[i] += 1;
            }
        }
    }
    Ok((!with_points.is_empty()).then_some(Series {
        values: total,
        covered: covered_days,
        total_depots,
        accounts: with_points,
    }))
}

/// The balance an account reports, as one point (valuation day or `end`), for a securities depot
/// that has no recorded value and no bookings. A balance of 0 is not a reported value.
fn reported_point(conn: &Connection, id: i64, end: NaiveDate) -> CoreResult<Option<(String, i64)>> {
    let row: Option<(String, i64, String)> = conn
        .query_row(
            "SELECT kind, balance_cents, balance_date FROM accounts WHERE id = ?1 AND deleted_ms IS NULL",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .ok();
    let Some((kind, cents, date)) = row else {
        return Ok(None);
    };
    if kind != "brokerage" || cents == 0 || crate::assets::is_tangible(conn, id)? {
        return Ok(None);
    }
    // An account with bookings keeps the walk back over them: that history is real. Only a depot
    // without any would show nothing but today's value carried backwards.
    let bookings: i64 = conn.query_row(
        "SELECT count(*) FROM transactions WHERE account_id = ?1 AND deleted_ms IS NULL",
        [id],
        |r| r.get(0),
    )?;
    if bookings > 0 {
        return Ok(None);
    }
    let day = if is_iso_date(date.trim()) {
        date.trim().to_string()
    } else {
        end.format("%Y-%m-%d").to_string()
    };
    Ok(Some((day, cents)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::AccountKind;
    use crate::vault::Vault;

    fn open(name: &str) -> (crate::test_support::TempDir, std::sync::Arc<Vault>) {
        let dir = crate::test_support::TempDir::new(name);
        let v = Vault::open(
            dir.join("v.db").to_string_lossy().to_string(),
            vec![7u8; 32],
        )
        .unwrap();
        (dir, v)
    }

    fn depot(v: &Vault, balance: i64, date: &str) -> i64 {
        let db = v.db().unwrap();
        db.execute(
            "INSERT INTO accounts(group_id, name, kind, balance_cents, balance_date, currency)
             VALUES ((SELECT COALESCE(MIN(id), 1) FROM account_groups), 'D', ?1, ?2, ?3, 'EUR')",
            params![AccountKind::Brokerage.as_str(), balance, date],
        )
        .or_else(|_| {
            db.execute(
                "INSERT INTO account_groups(name, sf_symbol, sort_order) VALUES ('G', 'x', 0)",
                [],
            )?;
            db.execute(
                "INSERT INTO accounts(group_id, name, kind, balance_cents, balance_date, currency)
                 VALUES (last_insert_rowid(), 'D', ?1, ?2, ?3, 'EUR')",
                params![AccountKind::Brokerage.as_str(), balance, date],
            )
        })
        .unwrap();
        db.last_insert_rowid()
    }

    /// One confirmed fetch observation of the depot total on `date`, derived.
    fn observe(db: &Connection, id: i64, date: &str, cents: i64, ms: i64) {
        observe_from(db, id, date, cents, ms, observations::SOURCE_FETCH);
    }

    fn observe_from(db: &Connection, id: i64, date: &str, cents: i64, ms: i64, source: &str) {
        let row = observations::ObservedValue {
            kind: observations::KIND_TOTAL,
            position: String::new(),
            source_ref: String::new(),
            name: String::new(),
            currency: "EUR".into(),
            observed_ms: ms,
            value_date: date.into(),
            value_cents: cents,
        };
        observations::record_as(db, id, source, &[row]).unwrap();
        rebuild(db, id, None).unwrap();
    }

    /// A derived value as such (sources and qualities no observation produces yet).
    fn derived(db: &Connection, id: i64, date: &str, cents: i64, source: &str, quality: &str) {
        db.execute(
            "INSERT OR REPLACE INTO depot_values(account_id, value_date, source, quality, value_cents)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, date, source, quality, cents],
        )
        .unwrap();
    }

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn the_later_fetch_of_a_day_replaces_the_earlier_one() {
        let (_d, v) = open("depot-values-record");
        let id = depot(&v, 100_000, "2026-03-01");
        let db = v.db().unwrap();
        record_fetch(&db, id, None, 1).unwrap();
        db.execute(
            "UPDATE accounts SET balance_cents = 120_000 WHERE id = ?1",
            [id],
        )
        .unwrap();
        record_fetch(&db, id, None, 2).unwrap();
        // both fetches stay observed, the later one is the value of the day
        let observed: i64 = db
            .query_row("SELECT count(*) FROM depot_observations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(observed, 2);
        let rows: Vec<(String, i64)> = db
            .prepare("SELECT value_date, value_cents FROM depot_values WHERE account_id = ?1")
            .unwrap()
            .query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows, vec![("2026-03-01".to_string(), 120_000)]);
    }

    #[test]
    fn an_unconfirmed_zero_and_other_accounts_leave_no_point() {
        let (_d, v) = open("depot-values-skip");
        let empty = depot(&v, 0, "2026-03-01");
        let db = v.db().unwrap();
        record_fetch(&db, empty, None, 1).unwrap();
        db.execute(
            "UPDATE accounts SET kind = 'checking', balance_cents = 5 WHERE id = ?1",
            [empty],
        )
        .unwrap();
        record_fetch(&db, empty, Some(&[]), 2).unwrap();
        let n: i64 = db
            .query_row("SELECT count(*) FROM depot_values", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn a_confirmed_zero_is_a_point_and_ends_the_last_positive_value() {
        let (_d, v) = open("depot-values-zero");
        let id = depot(&v, 50_000, "2026-03-02");
        let db = v.db().unwrap();
        record_fetch(&db, id, Some(&[]), 1).unwrap();
        db.execute(
            "UPDATE accounts SET balance_cents = 0, balance_date = '2026-03-05' WHERE id = ?1",
            [id],
        )
        .unwrap();
        record_fetch(&db, id, Some(&[]), 2).unwrap();
        let s = series(&db, &[id], day("2026-03-02"), day("2026-03-07"))
            .unwrap()
            .unwrap();
        assert_eq!(s.values, vec![50_000, 50_000, 50_000, 0, 0, 0]);
        assert_eq!(s.covered, vec![1; 6], "a confirmed zero counts as covered");
    }

    #[test]
    fn values_carry_forward_and_stay_unknown_before_the_first_point() {
        let (_d, v) = open("depot-values-series");
        let id = depot(&v, 130_000, "2026-03-10");
        let db = v.db().unwrap();
        observe(&db, id, "2026-03-03", 100_000, 1);
        observe(&db, id, "2026-03-07", 110_000, 2);
        observe(&db, id, "2026-03-10", 130_000, 3);
        let s = series(&db, &[id], day("2026-03-01"), day("2026-03-10"))
            .unwrap()
            .unwrap();
        assert_eq!(
            s.values,
            vec![0, 0, 100_000, 100_000, 100_000, 100_000, 110_000, 110_000, 110_000, 130_000]
        );
        assert_eq!(s.covered, vec![0, 0, 1, 1, 1, 1, 1, 1, 1, 1]);
        assert_eq!(s.total_depots, 1);
        assert_eq!(s.accounts, vec![id]);
        assert!(
            series(&db, &[id + 99], day("2026-03-01"), day("2026-03-10"))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_second_depot_counts_from_its_first_point_and_the_coverage_says_so() {
        let (_d, v) = open("depot-values-coverage");
        let a = depot(&v, 100, "2026-03-04");
        let b = depot(&v, 40, "2026-03-04");
        let none = depot(&v, 0, "2026-03-04");
        let db = v.db().unwrap();
        observe(&db, a, "2026-03-01", 100, 1);
        observe(&db, b, "2026-03-03", 40, 1);
        let s = series(&db, &[a, b, none], day("2026-03-01"), day("2026-03-04"))
            .unwrap()
            .unwrap();
        assert_eq!(s.values, vec![100, 100, 140, 140]);
        assert_eq!(s.covered, vec![1, 1, 2, 2]);
        assert_eq!(
            s.total_depots, 3,
            "a depot without any point is still a depot"
        );
        assert_eq!(s.accounts, vec![a, b]);
    }

    #[test]
    fn the_history_uses_the_depot_values_even_when_the_balance_differs() {
        let (_d, v) = open("depot-values-history");
        let today = chrono::Local::now().date_naive();
        let f = |d: NaiveDate| d.format("%Y-%m-%d").to_string();
        // the balance (100) differs from the last point (120), as after a device merge
        let id = depot(&v, 100_00, &f(today));
        {
            let db = v.db().unwrap();
            observe(&db, id, &f(today - chrono::Duration::days(1)), 80_00, 1);
            observe(&db, id, &f(today), 120_00, 2);
        }
        let h = v.balance_history(vec![id], 5).unwrap();
        let values: Vec<i64> = h.iter().map(|p| p.balance_cents).collect();
        // starts on the first known day: no invented past, no shifted line
        assert_eq!(values, vec![80_00, 120_00]);
    }

    #[test]
    fn a_depot_without_history_is_known_only_from_its_reported_balance_on() {
        let (_d, v) = open("depot-values-unrecorded");
        let a = depot(&v, 100, "2026-03-04"); // never recorded, balance reported on the 4th
        let b = depot(&v, 40, "2026-03-04");
        let db = v.db().unwrap();
        observe(&db, b, "2026-03-01", 40, 1);
        let s = series(&db, &[a, b], day("2026-03-01"), day("2026-03-05"))
            .unwrap()
            .unwrap();
        // before the 4th only the recorded depot is known; today's balance is not carried back
        assert_eq!(s.values, vec![40, 40, 40, 140, 140]);
        assert_eq!(s.covered, vec![1, 1, 1, 2, 2]);
        assert_eq!(s.total_depots, 2);
    }

    #[test]
    fn the_total_history_does_not_invent_a_past_for_a_depot_without_history() {
        let (_d, v) = open("depot-values-total");
        let today = chrono::Local::now().date_naive();
        let f = |n: i64| {
            (today - chrono::Duration::days(n))
                .format("%Y-%m-%d")
                .to_string()
        };
        let a = depot(&v, 100, &f(0)); // no recorded value
        let b = depot(&v, 40, &f(0));
        {
            let db = v.db().unwrap();
            observe(&db, b, &f(3), 40, 1);
        }
        let h = v.balance_history(vec![a, b], 5).unwrap();
        let values: Vec<i64> = h.iter().map(|p| p.balance_cents).collect();
        // the unrecorded depot adds nothing before its balance day (it was 100 only today)
        assert_eq!(values, vec![40, 40, 40, 140]);
    }

    #[test]
    fn the_best_source_of_a_day_is_shown_and_the_others_stay() {
        let (_d, v) = open("depot-values-sources");
        let id = depot(&v, 1, "2026-03-03");
        let db = v.db().unwrap();
        observe_from(&db, id, "2026-03-01", 950, 1, "import");
        observe_from(&db, id, "2026-03-03", 990, 3, "import");
        observe(&db, id, "2026-03-03", 1000, 2);
        derived(&db, id, "2026-03-01", 900, "prices", "estimated");
        derived(&db, id, "2026-03-02", 970, "prices", "reconstructed");
        derived(&db, id, "2026-03-02", 960, "prices", "estimated");
        let s = series(&db, &[id], day("2026-03-01"), day("2026-03-03"))
            .unwrap()
            .unwrap();
        // observed beats reconstructed beats estimated; a fetch beats an import of the same quality
        assert_eq!(s.values, vec![950, 960, 1000]);
        let n: i64 = db
            .query_row("SELECT count(*) FROM depot_values", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 5, "nothing is overwritten across sources");
    }

    #[test]
    fn the_depot_history_says_how_many_depots_each_day_covers() {
        let (_d, v) = open("depot-values-api");
        let today = chrono::Local::now().date_naive();
        let f = |n: i64| {
            (today - chrono::Duration::days(n))
                .format("%Y-%m-%d")
                .to_string()
        };
        let a = depot(&v, 100, &f(0));
        let b = depot(&v, 40, &f(0));
        {
            let db = v.db().unwrap();
            observe(&db, a, &f(3), 100, 1);
            observe(&db, b, &f(1), 40, 1);
        }
        let h = v.depot_value_history(vec![], 10).unwrap();
        let shape: Vec<(i64, u32, u32)> = h
            .iter()
            .map(|p| (p.value_cents, p.covered_depots, p.total_depots))
            .collect();
        // starts on the first known day; the second depot joins on day -1
        assert_eq!(h.first().unwrap().date, f(3));
        assert_eq!(
            shape,
            vec![(100, 1, 2), (100, 1, 2), (140, 2, 2), (140, 2, 2)]
        );
        let only_b = v.depot_value_history(vec![b], 10).unwrap();
        assert_eq!(only_b.len(), 2);
        assert_eq!(only_b[0].total_depots, 1);
        assert!(v.depot_value_history(vec![a + 99], 10).unwrap().is_empty());
    }

    #[test]
    fn the_latest_observation_of_a_day_wins_in_whatever_order_it_arrives() {
        let (_d, v) = open("depot-values-merge");
        let id = depot(&v, 1, "2026-03-01");
        let db = v.db().unwrap();
        let value = || -> i64 {
            db.query_row(
                "SELECT value_cents FROM depot_values WHERE account_id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
        };
        observe(&db, id, "2026-03-01", 100, 50);
        // an older observation (another device's, merged later) changes nothing
        observe(&db, id, "2026-03-01", 90, 40);
        assert_eq!(value(), 100);
        observe(&db, id, "2026-03-01", 120, 60);
        assert_eq!(value(), 120);
    }
}
