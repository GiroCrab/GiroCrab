//! Planned payments (#240): everything that is already ordered at the bank
//! but not booked yet. Today that are scheduled transfers (HKCSE); standing
//! orders, direct-debit collections and batch transfers add their own sources
//! to [`Vault::list_planned_payments`]. Only real orders appear here, never
//! guesses from the booking history (`recurring.rs`).

use rusqlite::{Connection, params};

use crate::connectors::TransferJob;
use crate::dates::today_iso as today;
use crate::error::{CoreError, CoreResult};
use crate::schema::{has_column, has_table};
use crate::vault::Vault;

/// Step 17: scheduled transfers sent to the bank. `end_to_end_id` is the merge
/// key. Rows stay until the execution date has passed; a fetched booking marks them
/// settled ([`drop_booked`], #1881; `settled_ms`, step 43, #1900). Idempotent like the
/// steps before.
pub(crate) fn planned_transfers(conn: &Connection) -> CoreResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS planned_transfers (
            id              INTEGER PRIMARY KEY,
            account_id      INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            end_to_end_id   TEXT    NOT NULL UNIQUE,
            execution_date  TEXT    NOT NULL,          -- YYYY-MM-DD
            recipient_name  TEXT    NOT NULL DEFAULT '',
            recipient_iban  TEXT    NOT NULL DEFAULT '',
            amount_cents    INTEGER NOT NULL,          -- positive = outgoing
            purpose         TEXT    NOT NULL DEFAULT '',
            created_ms      INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS planned_transfers_date
            ON planned_transfers(execution_date);",
    )?;
    Ok(())
}

/// Step 21 (#143): `kind` tells scheduled transfers (`transfer`, outgoing)
/// from submitted direct debit collections (`debit`, incoming); amounts stay
/// positive. Idempotent.
pub(crate) fn planned_kind(conn: &Connection) -> CoreResult<()> {
    if !has_column(conn, "main", "planned_transfers", "kind")? {
        conn.execute_batch(
            "ALTER TABLE planned_transfers ADD COLUMN kind TEXT NOT NULL DEFAULT 'transfer';",
        )?;
    }
    Ok(())
}

/// Step 43 (#1900): `settled_ms` marks a scheduled order the bank's booking has
/// settled (NULL = still open), so a vault merge cannot bring it back. List, count and
/// totals filter `settled_ms IS NULL`; the index over `(execution_date, settled_ms)`
/// replaces the plain date index for that filter. Idempotent.
pub(crate) fn planned_settled(conn: &Connection) -> CoreResult<()> {
    if !has_column(conn, "main", "planned_transfers", "settled_ms")? {
        conn.execute_batch("ALTER TABLE planned_transfers ADD COLUMN settled_ms INTEGER;")?;
    }
    conn.execute_batch(
        "DROP INDEX IF EXISTS planned_transfers_date;
         CREATE INDEX IF NOT EXISTS planned_transfers_date_settled
            ON planned_transfers(execution_date, settled_ms);",
    )?;
    Ok(())
}

/// What kind of order a planned payment comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PlannedKind {
    /// Scheduled transfer (Terminüberweisung, HKCSE).
    ScheduledTransfer,
    /// Standing order (Dauerauftrag, #239), read from the bank.
    StandingOrder,
    /// Direct debit collection handed to the bank (#143), incoming.
    DirectDebit,
}

/// One future payment with an order at the bank.
#[derive(Debug, Clone, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PlannedPayment {
    /// Stable across reloads, e.g. `transfer-12`.
    pub id: String,
    pub kind: PlannedKind,
    pub account_id: i64,
    pub account_name: String,
    /// Execution date `YYYY-MM-DD`.
    pub date: String,
    pub counterparty: String,
    pub counterparty_iban: String,
    /// Negative = outgoing, like bookings.
    pub amount_cents: i64,
    pub purpose: String,
    /// Standing orders: `M` (months) or `W` (weeks), else "".
    pub rhythm_unit: String,
    /// Standing orders: every n-th month/week, else 0.
    pub rhythm_interval: u32,
}

/// Sums of the planned payments in a period (SQL aggregate, no row loading).
#[derive(Debug, Clone, Default, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PlannedTotals {
    pub count: u32,
    /// Sum of the outgoing payments (negative or 0).
    pub outgoing_cents: i64,
    /// Sum of the incoming payments (positive or 0).
    pub incoming_cents: i64,
}

/// Rows whose date is long past never show again (the list starts at today);
/// dropped on every insert so the table cannot grow without bound.
fn prune_old(db: &Connection) -> CoreResult<()> {
    db.execute(
        "DELETE FROM planned_transfers WHERE execution_date < date('now', '-400 days')",
        [],
    )?;
    Ok(())
}

impl Vault {
    /// Remembers a scheduled transfer the bank accepted.
    pub(crate) fn record_planned_transfer(
        &self,
        account_id: i64,
        job: &TransferJob,
    ) -> CoreResult<()> {
        let Some(date) = job.execution_date.as_deref() else {
            return Ok(());
        };
        let db = self.db()?;
        prune_old(&db)?;
        db.execute(
            "INSERT OR IGNORE INTO planned_transfers(account_id, end_to_end_id, execution_date,
                    recipient_name, recipient_iban, amount_cents, purpose, created_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                account_id,
                job.end_to_end_id,
                date,
                job.recipient_name,
                job.recipient_iban,
                job.amount_cents,
                job.purpose,
                crate::dates::now_ms()
            ],
        )?;
        Ok(())
    }
}

impl Vault {
    /// Remembers a collection the bank accepted (incoming on `date`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn insert_planned_debit(
        db: &Connection,
        account_id: i64,
        end_to_end_id: &str,
        date: &str,
        debtor_name: &str,
        debtor_iban: &str,
        amount_cents: i64,
        purpose: &str,
    ) -> CoreResult<()> {
        prune_old(db)?;
        db.execute(
            "INSERT OR IGNORE INTO planned_transfers(account_id, end_to_end_id, execution_date,
                    recipient_name, recipient_iban, amount_cents, purpose, created_ms, kind)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'debit')",
            params![
                account_id,
                end_to_end_id,
                date,
                debtor_name,
                debtor_iban,
                amount_cents,
                purpose,
                crate::dates::now_ms()
            ],
        )?;
        Ok(())
    }
}

fn check_date(label: &str, d: &str) -> CoreResult<()> {
    jiff::civil::Date::strptime("%Y-%m-%d", d)
        .map(|_| ())
        .map_err(|_| CoreError::invalid(format!("{label}: {d}")))
}

#[uniffi::export]
impl Vault {
    /// Planned payments with execution date in `[from, to]` (`YYYY-MM-DD`,
    /// `None` = from today / open end), oldest first. `account_ids` empty = all.
    pub fn list_planned_payments(
        &self,
        from: Option<String>,
        to: Option<String>,
        account_ids: Vec<i64>,
    ) -> Result<Vec<PlannedPayment>, CoreError> {
        let from = from.unwrap_or_else(today);
        check_date("from", &from)?;
        if let Some(t) = &to {
            check_date("to", t)?;
        }
        let db = self.db()?;
        let mut q = db.prepare(
            "SELECT p.id, p.account_id, a.name, p.execution_date, p.recipient_name,
                    p.recipient_iban, p.amount_cents, p.purpose, p.kind
             FROM planned_transfers p JOIN accounts a ON a.id = p.account_id
             WHERE a.deleted_ms IS NULL AND p.settled_ms IS NULL AND p.execution_date >= ?1
               AND (?2 IS NULL OR p.execution_date <= ?2)
             ORDER BY p.execution_date, p.id",
        )?;
        let rows = q.query_map(params![from, to], |r| {
            let debit = r.get::<_, String>(8)? == "debit";
            Ok(PlannedPayment {
                id: format!("transfer-{}", r.get::<_, i64>(0)?),
                kind: if debit {
                    PlannedKind::DirectDebit
                } else {
                    PlannedKind::ScheduledTransfer
                },
                account_id: r.get(1)?,
                account_name: r.get(2)?,
                date: r.get(3)?,
                counterparty: r.get(4)?,
                counterparty_iban: r.get(5)?,
                amount_cents: if debit {
                    r.get::<_, i64>(6)?
                } else {
                    -r.get::<_, i64>(6)?
                },
                purpose: r.get(7)?,
                rhythm_unit: String::new(),
                rhythm_interval: 0,
            })
        })?;
        let mut out = Vec::new();
        for p in rows {
            let p = p?;
            if account_ids.is_empty() || account_ids.contains(&p.account_id) {
                out.push(p);
            }
        }
        // Standing orders: the executions in the same period.
        let parse = |s: &str| jiff::civil::Date::strptime("%Y-%m-%d", s).ok();
        if let Some(f) = parse(&from) {
            let t = to.as_deref().and_then(parse);
            out.extend(crate::standing_orders::planned_executions(
                &db,
                f,
                t,
                &account_ids,
            )?);
        }
        out.sort_by(|a, b| (&a.date, &a.id).cmp(&(&b.date, &b.id)));
        Ok(out)
    }

    /// Number of scheduled transfers and direct debits from today on, over all accounts
    /// (sidebar badge): one `COUNT(*)`, no rows are loaded.
    pub fn count_planned_payments(&self) -> Result<u32, CoreError> {
        let db = self.db()?;
        let n: i64 = db.query_row(
            "SELECT count(*) FROM planned_transfers p JOIN accounts a ON a.id = p.account_id
             WHERE a.deleted_ms IS NULL AND p.settled_ms IS NULL AND p.execution_date >= ?1",
            [today()],
            |r| r.get(0),
        )?;
        Ok(u32::try_from(n).unwrap_or(u32::MAX))
    }

    /// Count and sums of what [`Self::list_planned_payments`] would return, as one SQL
    /// aggregate (no rows are loaded or built).
    pub fn planned_payment_totals(
        &self,
        from: Option<String>,
        to: Option<String>,
        account_ids: Vec<i64>,
    ) -> Result<PlannedTotals, CoreError> {
        let from = from.unwrap_or_else(today);
        check_date("from", &from)?;
        if let Some(t) = &to {
            check_date("to", t)?;
        }
        // Integer ids only: safe to inline.
        let filter = if account_ids.is_empty() {
            String::new()
        } else {
            let ids: Vec<String> = account_ids.iter().map(i64::to_string).collect();
            format!(" AND p.account_id IN ({})", ids.join(","))
        };
        let sql = format!(
            "SELECT count(*),
                    coalesce(sum(CASE WHEN p.kind = 'debit' THEN 0 ELSE p.amount_cents END), 0),
                    coalesce(sum(CASE WHEN p.kind = 'debit' THEN p.amount_cents ELSE 0 END), 0)
             FROM planned_transfers p JOIN accounts a ON a.id = p.account_id
             WHERE a.deleted_ms IS NULL AND p.settled_ms IS NULL AND p.execution_date >= ?1
               AND (?2 IS NULL OR p.execution_date <= ?2){filter}"
        );
        let db = self.db()?;
        let (count, out, inc): (i64, i64, i64) = db.query_row(&sql, params![from, to], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
        // Scheduled transfers are stored as positive amounts and listed as outgoing;
        // scheduled direct debits are incoming.
        let mut t = PlannedTotals {
            count: u32::try_from(count).unwrap_or(u32::MAX),
            outgoing_cents: out.saturating_neg(),
            incoming_cents: inc,
        };
        // Standing orders only exist as computed executions (few orders, no row load of
        // the transfers needed).
        let parse = |s: &str| jiff::civil::Date::strptime("%Y-%m-%d", s).ok();
        if let Some(f) = parse(&from) {
            let end = to.as_deref().and_then(parse);
            for p in crate::standing_orders::planned_executions(&db, f, end, &account_ids)? {
                t.count += 1;
                if p.amount_cents < 0 {
                    t.outgoing_cents = t.outgoing_cents.saturating_add(p.amount_cents);
                } else {
                    t.incoming_cents = t.incoming_cents.saturating_add(p.amount_cents);
                }
            }
        }
        Ok(t)
    }
}

/// Marks open planned payments (from today on) as settled whose SEPA end-to-end reference a
/// live booking of the same account carries, with the order's amount, booked on or after the
/// day the order was placed (#1900). The reference names one order (`end_to_end_id` is
/// unique), so this needs no "booking used once" bookkeeping and may look at every booking:
/// one that gained its reference in a later fetch counts, and so does one a vault merge
/// brought in before or after the order. `account_id` `None` = all accounts (after a merge).
pub(crate) fn settle_referenced(db: &Connection, account_id: Option<i64>) -> CoreResult<()> {
    db.execute(
        "UPDATE planned_transfers SET settled_ms = ?1
         WHERE settled_ms IS NULL AND execution_date >= ?2
           AND (?3 IS NULL OR account_id = ?3)
           AND trim(end_to_end_id) <> '' AND upper(trim(end_to_end_id)) <> 'NOTPROVIDED'
           AND EXISTS (
               SELECT 1 FROM transactions t
               WHERE t.account_id = planned_transfers.account_id AND t.deleted_ms IS NULL
                 AND trim(t.end_to_end_id) = trim(planned_transfers.end_to_end_id)
                 AND t.amount_cents = CASE WHEN planned_transfers.kind = 'debit'
                     THEN planned_transfers.amount_cents ELSE -planned_transfers.amount_cents END
                 AND t.booking_date >=
                     date(planned_transfers.created_ms / 1000, 'unixepoch', 'localtime')
                 AND t.external_id NOT LIKE 'pending-%'
                 AND t.external_id NOT LIKE 'advanzia-hold-%')",
        params![crate::dates::now_ms(), today(), account_id],
    )?;
    Ok(())
}

/// A usable SEPA end-to-end reference: neither blank nor the placeholder `NOTPROVIDED`.
/// Only such a reference names an order (#1900).
pub(crate) fn names_order(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty() && !s.eq_ignore_ascii_case("NOTPROVIDED")
}

/// Vault merge (#1900): a booking both vaults have keeps a usable reference of its own, but
/// takes the other side's when its own is blank or `NOTPROVIDED`, so [`settle_referenced`]
/// finds the order it names. Only when the mandate and creditor identifiers are compatible
/// (blank on either side, else equal): a pair that names different mandates is not the same
/// payment, so the reference is left alone. `booking_map`: the other vault's booking id →
/// ours. Writes only rows that change, so a merge that brings nothing new changes no row.
pub(crate) fn adopt_peer_references<'a>(
    conn: &Connection,
    booking_map: impl IntoIterator<Item = (&'a i64, &'a i64)>,
) -> CoreResult<()> {
    type PeerRef = (String, String, String);
    let refs: std::collections::HashMap<i64, PeerRef> = conn
        .prepare(
            "SELECT id, trim(end_to_end_id), trim(coalesce(mandate_id, '')),
                    trim(coalesce(creditor_id, ''))
             FROM other.transactions
             WHERE upper(trim(coalesce(end_to_end_id, ''))) NOT IN ('', 'NOTPROVIDED')",
        )?
        .query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?, r.get(3)?))))?
        .collect::<Result<_, _>>()?;
    if refs.is_empty() {
        return Ok(());
    }
    let mut set = conn.prepare(
        "UPDATE main.transactions SET end_to_end_id = ?2
         WHERE id = ?1 AND upper(trim(coalesce(end_to_end_id, ''))) IN ('', 'NOTPROVIDED')
           AND (trim(coalesce(mandate_id, '')) = '' OR ?3 = '' OR upper(trim(mandate_id)) = upper(?3))
           AND (trim(coalesce(creditor_id, '')) = '' OR ?4 = '' OR upper(trim(creditor_id)) = upper(?4))",
    )?;
    for (other, mine) in booking_map {
        if let Some((e2e, mandate, creditor)) = refs.get(other) {
            set.execute(params![*mine, e2e, mandate, creditor])?;
        }
    }
    Ok(())
}

/// Marks the planned payments of `account_id` the bank has booked by now as settled (#1881,
/// #1900; `settled_ms`, the row stays so a vault merge cannot bring the order back). A booking
/// with the order's end-to-end reference settles it first ([`settle_referenced`]). A booking
/// without a reference settles by the same IBAN (when both sides have one) and amount, booked
/// on or after the day the order was placed; such a booking settles one order only: only the
/// bookings this fetch inserted or adopted (`fetched`, their ids) count, so a later fetch, or a second
/// call within the same second, cannot use it for a second order. A booking with a reference
/// settles another order by amount only when that order has no payee IBAN (a batch sum, whose
/// local id the bank never shows, #1898) and no stored order carries the reference, so it never
/// settles a second order next to the one it names. Pending/held rows are no bookings. Orders the bank
/// executes early or on another day (weekend, holiday) would otherwise stay until their date
/// has passed. Orders are tried by date, creation time and reference, bookings by date and bank
/// id, never by the local row id, so every replica of the vault settles the same order for the
/// same booking (the merge keeps either side's mark).
pub(crate) fn drop_booked(
    db: &Connection,
    account_id: i64,
    fetched: &std::collections::HashSet<i64>,
) -> CoreResult<()> {
    settle_referenced(db, Some(account_id))?;
    if fetched.is_empty() {
        return Ok(());
    }
    let mut q = db.prepare(
        "SELECT id, recipient_iban, amount_cents, kind,
                date(created_ms / 1000, 'unixepoch', 'localtime')
         FROM planned_transfers
         WHERE account_id = ?1 AND settled_ms IS NULL AND execution_date >= ?2
         ORDER BY execution_date, created_ms, end_to_end_id",
    )?;
    let orders: Vec<(i64, String, i64, String, String)> = q
        .query_map(params![account_id, today()], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?
        .collect::<Result<_, _>>()?;
    if orders.is_empty() {
        return Ok(());
    }
    let now_ms = crate::dates::now_ms();
    let norm = |s: &str| s.split_whitespace().collect::<String>().to_uppercase();
    let mut used: std::collections::HashSet<i64> = Default::default();
    // `named`: a stored order (open or settled) carries the booking's reference
    let mut find = db.prepare(
        "SELECT t.id, COALESCE(t.counterparty_iban, ''), COALESCE(t.end_to_end_id, ''),
                EXISTS (SELECT 1 FROM planned_transfers p
                        WHERE p.end_to_end_id = trim(t.end_to_end_id))
         FROM transactions t
         WHERE t.account_id = ?1 AND t.deleted_ms IS NULL AND t.amount_cents = ?2
           AND t.booking_date >= ?3
           AND t.external_id NOT LIKE 'pending-%' AND t.external_id NOT LIKE 'advanzia-hold-%'
         ORDER BY t.booking_date, t.external_id, t.id",
    )?;
    // References were settled above. First pass: exact IBAN, so an order without a better
    // booking cannot take the booking of another order; second: no IBAN on either side.
    let mut open = orders;
    for exact in [true, false] {
        let mut left = Vec::new();
        for order in open {
            let (id, iban, cents, kind, since) = &order;
            let signed = if kind == "debit" { *cents } else { -*cents };
            let cands: Vec<(i64, String, String, bool)> = find
                .query_map(params![account_id, signed, since], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })?
                .collect::<Result<_, _>>()?;
            let hit = cands.iter().find(|(tid, tiban, te2e, named)| {
                !used.contains(tid)
                    && fetched.contains(tid)
                    // the booking belongs to the order its reference names; only an order
                    // without a payee IBAN (batch sum) takes one whose reference no order has
                    && (!names_order(te2e) || (iban.is_empty() && !named))
                    && if exact {
                        !iban.is_empty() && norm(iban) == norm(tiban)
                    } else {
                        iban.is_empty() || tiban.is_empty() || norm(iban) == norm(tiban)
                    }
            });
            if let Some((tid, _, _, _)) = hit {
                used.insert(*tid);
                db.execute(
                    "UPDATE planned_transfers SET settled_ms = ?2 WHERE id = ?1",
                    params![id, now_ms],
                )?;
            } else {
                left.push(order);
            }
        }
        open = left;
    }
    Ok(())
}

/// Vault merge: adds the other vault's scheduled transfers we do not have yet
/// (by `end_to_end_id`); account ids are mapped. Settled wins (#1900): an order either side
/// has settled stays settled, so it does not come back from the other vault.
pub(crate) fn merge_attached(
    conn: &Connection,
    acc_map: &std::collections::HashMap<i64, i64>,
) -> CoreResult<()> {
    if !has_table(conn, "other", "planned_transfers")? {
        return Ok(());
    }
    let has_kind = has_column(conn, "other", "planned_transfers", "kind")?;
    let settled_col = if has_column(conn, "other", "planned_transfers", "settled_ms")? {
        "settled_ms"
    } else {
        "NULL"
    };
    let rows: Vec<(
        i64,
        String,
        String,
        String,
        String,
        i64,
        String,
        i64,
        String,
        Option<i64>,
    )> = {
        let kind_col = if has_kind { "kind" } else { "'transfer'" };
        let mut q = conn.prepare(&format!(
            "SELECT account_id, end_to_end_id, execution_date, recipient_name, recipient_iban,
                    amount_cents, purpose, created_ms, {kind_col}, {settled_col}
             FROM other.planned_transfers"
        ))?;
        let it = q.query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
                r.get(9)?,
            ))
        })?;
        it.collect::<Result<_, _>>()?
    };
    for (acc, e2e, date, name, iban, cents, purpose, created, kind, settled) in rows {
        let Some(&mine) = acc_map.get(&acc) else {
            continue;
        };
        conn.execute(
            "INSERT INTO planned_transfers(account_id, end_to_end_id, execution_date,
                    recipient_name, recipient_iban, amount_cents, purpose, created_ms, kind,
                    settled_ms)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
             ON CONFLICT(end_to_end_id) DO UPDATE SET settled_ms = excluded.settled_ms
                WHERE planned_transfers.settled_ms IS NULL AND excluded.settled_ms IS NOT NULL",
            params![
                mine, e2e, date, name, iban, cents, purpose, created, kind, settled
            ],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
