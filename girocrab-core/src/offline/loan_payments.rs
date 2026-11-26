//! Source links for mirrored principal bookings. Both endpoints are local booking
//! IDs, translated by the ordinary booking merge and by MoneyMoney twin folding.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::CoreResult;

pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS offline_loan_payments (
    payment_id INTEGER PRIMARY KEY REFERENCES transactions(id) ON DELETE CASCADE,
    source_id INTEGER NOT NULL REFERENCES transactions(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS offline_loan_payments_source ON offline_loan_payments(source_id);
CREATE TABLE IF NOT EXISTS offline_loan_source_moves (
    old_id INTEGER PRIMARY KEY REFERENCES transactions(id) ON DELETE CASCADE,
    new_id INTEGER NOT NULL REFERENCES transactions(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS offline_loan_source_moves_new ON offline_loan_source_moves(new_id);
";

const PREFIX: &str = "manual-loan-payment-";

pub(crate) fn is_payment_key(external_id: &str) -> bool {
    external_id.starts_with(PREFIX)
}

pub(super) fn payment_key(source_key: &str, source_external_id: &str) -> String {
    let identity = format!("{}:{source_key}{source_external_id}", source_key.len());
    let id = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, identity.as_bytes());
    format!("{PREFIX}{id}")
}

pub(crate) fn migrate(conn: &Connection) -> CoreResult<()> {
    conn.execute_batch(SCHEMA)?;
    let existing: Vec<(String, i64)> = conn
        .prepare(
            "SELECT t.external_id, t.id FROM transactions t JOIN accounts a ON a.id = t.account_id
          WHERE a.kind = 'loan' AND a.external_ref GLOB 'offline-loan-*'
            AND t.external_id GLOB 'manual-loan-payment-*'
            AND NOT EXISTS(SELECT 1 FROM offline_loan_payments p WHERE p.payment_id=t.id)",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    if existing.is_empty() {
        return Ok(());
    }
    // Version 44 stored only hashes. Recover an endpoint only if the original
    // account key and external ID still identify exactly one source.
    let mut sources = HashMap::new();
    let mut accounts = HashMap::new();
    let mut query =
        conn.prepare("SELECT id,account_id,external_id FROM transactions ORDER BY account_id")?;
    let mut rows = query.query([])?;
    while let Some(row) = rows.next()? {
        let (id, account, external): (i64, i64, String) = (row.get(0)?, row.get(1)?, row.get(2)?);
        let account_key = match accounts.entry(account) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(crate::sync_merge::account_key(conn, account)?)
            }
        };
        sources
            .entry(payment_key(account_key, &external))
            .and_modify(|old| *old = None)
            .or_insert(Some(id));
    }
    for (key, payment) in existing {
        if let Some(Some(source)) = sources.get(&key) {
            record(conn, payment, *source)?;
        }
    }
    Ok(())
}

/// A hidden duplicate keeps its booking and its link for the next peer merge.
/// A real deletion has a positive stamp and applies to all copies of the payment.
const SHADOW: i64 = -1;

const SOURCE_ALIASES: &str = "WITH RECURSIVE aliases(id) AS (
    VALUES (?1)
    UNION SELECT m.old_id FROM offline_loan_source_moves m JOIN aliases a ON a.id = m.new_id
)";

pub(super) fn already_mirrored(conn: &Connection, source: i64, loan: i64) -> CoreResult<bool> {
    // A destination can become a tombstone before the next reconciliation.
    // Check the entire alias group, including links to its former representative.
    Ok(conn
        .prepare_cached(&format!(
            "{SOURCE_ALIASES}
             SELECT EXISTS(SELECT 1 FROM aliases a JOIN offline_loan_payments p ON p.source_id = a.id
                JOIN transactions t ON t.id = p.payment_id WHERE t.account_id = ?2)"
        ))?
        .query_row(params![source_root(conn, source)?, loan], |r| r.get(0))?)
}

pub(super) fn is_mirrored(conn: &Connection, booking: i64) -> CoreResult<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM offline_loan_payments WHERE payment_id = ?1)",
        [booking],
        |r| r.get(0),
    )?)
}

pub(super) fn record(conn: &Connection, payment: i64, source: i64) -> CoreResult<()> {
    conn.execute(
        "INSERT INTO offline_loan_payments(payment_id, source_id) VALUES (?1, ?2)",
        params![payment, resolve_source(conn, source)?],
    )?;
    Ok(())
}

pub(crate) fn merge_attached(conn: &Connection, bookings: &HashMap<i64, i64>) -> CoreResult<()> {
    let mut moves =
        conn.prepare("SELECT old_id,new_id FROM other.offline_loan_source_moves ORDER BY old_id")?;
    let mut rows = moves.query([])?;
    while let Some(row) = rows.next()? {
        let (old, new): (i64, i64) = (row.get(0)?, row.get(1)?);
        if let (Some(&old), Some(&new)) = (bookings.get(&old), bookings.get(&new)) {
            move_source(conn, old, new)?;
        }
    }

    let mut query =
        conn.prepare("SELECT payment_id, source_id FROM other.offline_loan_payments")?;
    let mut rows = query.query([])?;
    while let Some(row) = rows.next()? {
        let (payment, source): (i64, i64) = (row.get(0)?, row.get(1)?);
        if let (Some(&payment), Some(&source)) = (bookings.get(&payment), bookings.get(&source)) {
            // A peer can already have folded a source account that still
            // exists here. Keep the live endpoint if the other is a tombstone.
            conn.execute(
                "INSERT INTO offline_loan_payments(payment_id, source_id) VALUES (?1, ?2)
                 ON CONFLICT(payment_id) DO UPDATE SET source_id = excluded.source_id
                  WHERE EXISTS(SELECT 1 FROM transactions t JOIN accounts a ON a.id = t.account_id
                    WHERE t.id = excluded.source_id AND t.deleted_ms IS NULL AND a.deleted_ms IS NULL)
                    AND EXISTS(SELECT 1 FROM transactions t JOIN accounts a ON a.id = t.account_id
                    WHERE t.id = offline_loan_payments.source_id AND (t.deleted_ms IS NOT NULL OR a.deleted_ms IS NOT NULL))",
                params![payment, resolve_source(conn, source)?],
            )?;
        }
    }
    reconcile(conn)
}

fn source_root(conn: &Connection, source: i64) -> CoreResult<i64> {
    Ok(conn
        .prepare_cached(
            "WITH RECURSIVE chain(id) AS (
            VALUES (?1)
            UNION SELECT m.new_id FROM offline_loan_source_moves m JOIN chain c ON c.id=m.old_id
         )
         SELECT id FROM chain WHERE id NOT IN (SELECT old_id FROM offline_loan_source_moves)",
        )?
        .query_row([source], |r| r.get(0))
        .optional()?
        .unwrap_or(source))
}

fn source_is_live(conn: &Connection, id: i64) -> CoreResult<bool> {
    Ok(conn
        .prepare_cached(
            "SELECT t.deleted_ms IS NULL AND a.deleted_ms IS NULL
               FROM transactions t JOIN accounts a ON a.id = t.account_id WHERE t.id = ?1",
        )?
        .query_row([id], |r| r.get(0))?)
}

fn resolve_source(conn: &Connection, source: i64) -> CoreResult<i64> {
    let root = source_root(conn, source)?;
    if source_is_live(conn, root)? {
        return Ok(root);
    }

    // A canonical destination may later be deleted. The incoming aliases
    // still provide the exact equivalent bookings, without matching by amount.
    let mut query = conn.prepare_cached(&format!(
        "{SOURCE_ALIASES}
         SELECT t.id FROM aliases x
           JOIN transactions t ON t.id = x.id JOIN accounts a ON a.id = t.account_id
          WHERE t.deleted_ms IS NULL AND a.deleted_ms IS NULL"
    ))?;
    let live = query
        .query_map([root], |r| r.get(0))?
        .map(|id| {
            let id = id?;
            Ok((crate::sync_merge::booking_key(conn, id)?, id))
        })
        .collect::<CoreResult<Vec<_>>>()?;
    Ok(live.into_iter().min().map_or(root, |(_, id)| id))
}

pub(crate) fn move_source(conn: &Connection, old: i64, new: i64) -> CoreResult<()> {
    if old == new {
        return Ok(());
    }
    let previous = source_root(conn, old)?;
    let proposed = source_root(conn, new)?;
    if previous == proposed {
        return Ok(());
    }

    // A direct replacement follows the destination chosen by account/booking
    // matching. If the source was already moved, join both destinations:
    // prefer a live one, then compare stable booking keys, never local IDs.
    let prefer_previous = if previous == old {
        false
    } else {
        let previous_live = resolve_source(conn, previous)?;
        let proposed_live = resolve_source(conn, proposed)?;
        let rank = |id| -> CoreResult<_> {
            Ok((
                !source_is_live(conn, id)?,
                crate::sync_merge::booking_key(conn, id)?,
            ))
        };
        rank(previous_live)? <= rank(proposed_live)?
    };
    let (from, to) = if prefer_previous {
        (proposed, previous)
    } else {
        (previous, proposed)
    };

    conn.prepare_cached("INSERT INTO offline_loan_source_moves(old_id, new_id) VALUES (?1, ?2)")?
        .execute(params![from, to])?;
    // Preserve links if a caller physically removes the original booking.
    conn.prepare_cached("UPDATE offline_loan_payments SET source_id = ?2 WHERE source_id = ?1")?
        .execute(params![from, resolve_source(conn, to)?])?;
    Ok(())
}

/// Persist the same live representative for every linked copy in a cohort.
fn normalize_sources(conn: &Connection) -> CoreResult<()> {
    let sources = conn
        .prepare_cached(
            "SELECT DISTINCT source_id FROM offline_loan_payments
              WHERE source_id IN (SELECT old_id FROM offline_loan_source_moves)
                 OR source_id IN (SELECT new_id FROM offline_loan_source_moves)",
        )?
        .query_map([], |r| r.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let mut update = conn
        .prepare_cached("UPDATE offline_loan_payments SET source_id = ?2 WHERE source_id = ?1")?;
    for source in sources {
        let canonical = resolve_source(conn, source)?;
        if source != canonical {
            update.execute(params![source, canonical])?;
        }
    }
    Ok(())
}

pub(crate) fn remap_folded(conn: &Connection, bookings: &HashMap<i64, i64>) -> CoreResult<()> {
    for (&old, &new) in bookings {
        move_source(conn, old, new)?;
    }
    reconcile(conn)
}

/// At most one live principal booking per source and target. If independent
/// copies meet, retain the latest amount edit, then date edit, then a stable key.
/// A user deletion from either copy wins. Shadows can be unhidden if a later
/// merge corrects a source mapping, so they never act as user deletions.
pub(crate) fn reconcile(conn: &Connection) -> CoreResult<()> {
    normalize_sources(conn)?;
    let mut update = conn.prepare_cached(
        "WITH ranked AS (
            SELECT t.id, a.deleted_ms AS account_deleted,
                MAX(CASE WHEN t.deleted_ms > 0 AND t.deleted_ms IS NOT a.deleted_ms
                         THEN t.deleted_ms END) OVER cohort AS removed,
                ROW_NUMBER() OVER (cohort ORDER BY t.splits_ms DESC, t.date_ms DESC, t.external_id) AS rank
            FROM offline_loan_payments p JOIN transactions t ON t.id = p.payment_id
                 JOIN accounts a ON a.id = t.account_id
            WINDOW cohort AS (PARTITION BY p.source_id, t.account_id)
         ), desired AS (
            SELECT id, COALESCE(removed, account_deleted, CASE WHEN rank > 1 THEN ?1 END) AS stamp
            FROM ranked
         )
         UPDATE transactions SET deleted_ms = desired.stamp FROM desired
          WHERE transactions.id = desired.id AND transactions.deleted_ms IS NOT desired.stamp
         RETURNING account_id",
    )?;
    let mut accounts = update
        .query_map([SHADOW], |r| r.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    accounts.sort_unstable();
    accounts.dedup();
    for id in accounts {
        if conn.query_row(
            "SELECT deleted_ms IS NULL FROM accounts WHERE id = ?1",
            [id],
            |r| r.get::<_, bool>(0),
        )? {
            crate::opening_balance::refresh(conn, id)?;
        }
    }
    Ok(())
}
