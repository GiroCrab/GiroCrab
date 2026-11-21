//! Explicit refund links (#714): a refund (incoming booking) points at the booking it pays back
//! (outgoing). One original can have several refunds, a refund has one original, and links do not
//! chain. The link only documents what belongs together: amounts, categories and balances remain
//! unchanged. Removing a link keeps the row as a tombstone (`deleted_ms`), so the removal reaches
//! the other devices; the newest change wins on merge.
use crate::{
    error::{CoreError, CoreResult},
    i18n::tr,
    vault::Vault,
};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashMap;

pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS refund_links (
 refund_id INTEGER PRIMARY KEY REFERENCES transactions(id) ON DELETE CASCADE,
 original_id INTEGER NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
 created_ms INTEGER NOT NULL,
 changed_ms INTEGER NOT NULL,
 deleted_ms INTEGER
);
CREATE INDEX IF NOT EXISTS refund_links_original ON refund_links(original_id);
";

/// What a booking is in a refund link, for the marker in the booking list.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RefundLinkKind {
    None,
    Refund,
    Original,
}

/// One booking of a refund link or a candidate for one.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RefundBooking {
    pub id: i64,
    pub account_id: i64,
    pub account_name: String,
    pub booking_date: String,
    pub amount_cents: i64,
    pub currency: String,
    /// Merchant name, or the counterparty when no merchant is recognised.
    pub display_name: String,
    /// Raw counterparty and purpose, for the second line of a row and for searching.
    pub counterparty: String,
    pub purpose: String,
}

/// Why a booking is suggested for a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RefundMatch {
    /// The same amount in the other direction (and the same currency).
    SameAmount,
    /// The same counterparty or merchant within [`SUGGESTION_DAYS`] days.
    SameMerchant,
    /// Both texts name the same reference (order or invoice number, end-to-end id).
    SameReference,
}

/// A booking suggested for a link, with the reason.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RefundSuggestion {
    pub booking: RefundBooking,
    pub reason: RefundMatch,
}

/// "The last two months" of the same-merchant suggestions.
const SUGGESTION_DAYS: i64 = 62;
/// How far the suggestions and the unfiltered list look back or ahead.
const WINDOW_DAYS: u32 = 365;

/// The links of one booking: its original when it is a refund, its refunds when it is an original.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RefundLinks {
    pub original: Option<RefundBooking>,
    pub refunds: Vec<RefundBooking>,
}

fn live(conn: &Connection, id: i64) -> CoreResult<i64> {
    conn.query_row(
        "SELECT amount_cents FROM transactions WHERE id=?1 AND deleted_ms IS NULL",
        [id],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| CoreError::invalid(tr("Die Buchung existiert nicht oder wurde gelöscht.")))
}

fn validate(conn: &Connection, refund: i64, original: i64) -> CoreResult<()> {
    let r = live(conn, refund)?;
    let o = live(conn, original)?;
    if refund == original || r <= 0 || o >= 0 {
        return Err(CoreError::invalid(tr(
            "Eine Erstattung muss positiv und die ursprüngliche Buchung negativ sein.",
        )));
    }
    let conflict: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM refund_links WHERE deleted_ms IS NULL AND (refund_id=?1 OR original_id=?2))",
        params![original, refund],
        |r| r.get(0),
    )?;
    if conflict {
        return Err(CoreError::invalid(tr(
            "Erstattungen dürfen nicht miteinander verkettet werden.",
        )));
    }
    Ok(())
}

const ROWS: &str = "SELECT t.id,t.account_id,a.name,t.booking_date,t.amount_cents,t.currency,t.counterparty,t.purpose,t.end_to_end_id FROM transactions t JOIN accounts a ON a.id=t.account_id";

/// A row of [`ROWS`] with what only the ranking needs.
struct Row {
    booking: RefundBooking,
    end_to_end_id: String,
}

fn rows(
    conn: &Connection,
    sql: &str,
    args: impl rusqlite::Params,
    rules: &crate::merchants::MerchantRules,
) -> CoreResult<Vec<Row>> {
    let rows = conn
        .prepare(sql)?
        .query_map(args, |r| {
            let cp: String = r.get(6)?;
            let purpose: String = r.get(7)?;
            let merchant = rules.resolve_with_bundled(&cp, &purpose).0;
            Ok(Row {
                booking: RefundBooking {
                    id: r.get(0)?,
                    account_id: r.get(1)?,
                    account_name: r.get(2)?,
                    booking_date: r.get(3)?,
                    amount_cents: r.get(4)?,
                    currency: r.get(5)?,
                    display_name: if merchant.name.is_empty() {
                        cp.clone()
                    } else {
                        merchant.name
                    },
                    counterparty: cp,
                    purpose,
                },
                end_to_end_id: r.get(8)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

fn entries(
    conn: &Connection,
    sql: &str,
    args: impl rusqlite::Params,
    rules: &crate::merchants::MerchantRules,
) -> CoreResult<Vec<RefundBooking>> {
    Ok(rows(conn, sql, args, rules)?
        .into_iter()
        .map(|r| r.booking)
        .collect())
}

/// Ids of the refunds with a live link. The booking list reads them as refunds, not as income
/// (`BookingFinisher`): a linked refund reduces the expenses instead of raising the income.
pub(crate) fn linked_refund_ids(conn: &Connection) -> CoreResult<std::collections::HashSet<i64>> {
    Ok(markers(conn)?
        .into_iter()
        .filter(|(_, kind)| *kind == RefundLinkKind::Refund)
        .map(|(id, _)| id)
        .collect())
}

/// The booking a link is looked for, and the bookings of the other direction around it.
struct Pool {
    amount: i64,
    currency: String,
    date: String,
    counterparty: String,
    merchant: String,
    reference: String,
    /// Nearest booking date first.
    rows: Vec<Row>,
}

/// `days`: how far back (refund) or ahead (outgoing booking) the pool reaches; `None` = no bound.
/// Leaves out what cannot be linked: refunds of another original, and everything when the
/// booking itself is the other end of a link (no chains).
fn pool(vault: &Vault, db: &Connection, id: i64, days: Option<u32>) -> CoreResult<Pool> {
    let rules = vault.merchant_rules(db)?;
    let (amount, currency, date, counterparty, purpose, e2e): (
        i64,
        String,
        String,
        String,
        String,
        String,
    ) = db
        .query_row(
            "SELECT amount_cents,currency,booking_date,counterparty,purpose,end_to_end_id FROM transactions WHERE id=?1 AND deleted_ms IS NULL",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .optional()?
        .ok_or_else(|| CoreError::invalid(tr("Die Buchung existiert nicht oder wurde gelöscht.")))?;
    let merchant = rules.resolve_with_bundled(&counterparty, &purpose).0.name;
    let mut out = Pool {
        amount,
        currency,
        date,
        counterparty,
        merchant,
        reference: format!("{purpose} {e2e}"),
        rows: Vec::new(),
    };
    let conflict: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM refund_links WHERE deleted_ms IS NULL AND ((?2>0 AND original_id=?1) OR (?2<0 AND refund_id=?1)))",
        params![id, amount],
        |r| r.get(0),
    )?;
    if amount == 0 || conflict {
        return Ok(out);
    }
    let days = days.map_or(f64::MAX, f64::from);
    let direction = if amount > 0 {
        "t.amount_cents<0 AND julianday(?1)-julianday(t.booking_date) BETWEEN 0 AND ?3 AND NOT EXISTS(SELECT 1 FROM refund_links WHERE refund_id=t.id AND deleted_ms IS NULL)"
    } else {
        "t.amount_cents>0 AND julianday(t.booking_date)-julianday(?1) BETWEEN 0 AND ?3 AND NOT EXISTS(SELECT 1 FROM refund_links WHERE refund_id=t.id AND original_id<>?2 AND deleted_ms IS NULL)"
    };
    out.rows = rows(
        db,
        &format!(
            "{ROWS} WHERE t.deleted_ms IS NULL AND t.id<>?2 AND {direction} ORDER BY abs(julianday(t.booking_date)-julianday(?1)),t.id"
        ),
        params![out.date, id, days],
        &rules,
    )?;
    Ok(out)
}

fn currency_code(c: &str) -> String {
    if c.trim().is_empty() {
        "EUR".into()
    } else {
        c.trim().to_uppercase()
    }
}

/// Order numbers, invoice numbers and the like: words of at least six letters and digits with a
/// digit among them, read without their separators (`RE-2025/7781` = `RE20257781`). Dates and
/// amounts (digits and dots only) are no reference.
fn reference_tokens(text: &str) -> std::collections::HashSet<String> {
    text.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | ':' | '(' | ')' | '+'))
        .filter(|w| !w.chars().all(|c| c.is_ascii_digit() || c == '.'))
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(char::to_uppercase)
                .collect::<String>()
        })
        .filter(|w| {
            w.chars().count() >= 6 && w.chars().any(|c| c.is_ascii_digit()) && w != "NOTPROVIDED"
        })
        .collect()
}

fn days_between(a: &str, b: &str) -> Option<i64> {
    let day = |s: &str| crate::dates::parse_iso(s.get(..10)?);
    Some(crate::dates::days_between(day(a)?, day(b)?).abs())
}

pub(crate) fn markers(conn: &Connection) -> CoreResult<HashMap<i64, RefundLinkKind>> {
    let mut out = HashMap::new();
    let pairs = conn.prepare("SELECT refund_id,original_id FROM refund_links l JOIN transactions r ON r.id=l.refund_id JOIN transactions o ON o.id=l.original_id WHERE l.deleted_ms IS NULL AND r.deleted_ms IS NULL AND o.deleted_ms IS NULL")?
 .query_map([], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?)))?.collect::<Result<Vec<_>,_>>()?;
    for (r, o) in pairs {
        out.insert(r, RefundLinkKind::Refund);
        out.insert(o, RefundLinkKind::Original);
    }
    Ok(out)
}

type LinkRow = (i64, i64, i64, i64, Option<i64>);
type LinkRank = (i64, Option<i64>, Vec<String>, i64);

fn original_key(conn: &Connection, id: i64) -> CoreResult<Vec<String>> {
    Ok(conn.query_row(
        "SELECT COALESCE(c.connector,''),COALESCE(c.login_name,''),COALESCE(c.bank_code,''),a.external_ref,t.external_id FROM transactions t JOIN accounts a ON a.id=t.account_id LEFT JOIN bank_connections c ON c.id=a.connection_id WHERE t.id=?1",
        [id], |r| (0..5).map(|i|r.get(i)).collect(),
    )?)
}

fn rank(conn: &Connection, row: &LinkRow) -> CoreResult<LinkRank> {
    Ok((row.3, row.4, original_key(conn, row.1)?, row.2))
}

pub(crate) fn merge(conn: &Connection, ids: &HashMap<i64, i64>) -> CoreResult<()> {
    let links = conn
        .prepare(
            "SELECT refund_id,original_id,created_ms,changed_ms,deleted_ms FROM other.refund_links",
        )?
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Option<i64>>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    'links: for (r, o, created, changed, deleted) in links {
        let (Some(&r), Some(&o)) = (ids.get(&r), ids.get(&o)) else {
            continue;
        };
        for id in [r, o] {
            match live(conn, id) {
                Ok(_) => {}
                Err(CoreError::InvalidInput { .. }) => continue 'links,
                Err(error) => return Err(error),
            }
        }
        let incoming = (r, o, created, changed, deleted);
        let local: Option<LinkRow> = conn.query_row(
            "SELECT refund_id,original_id,created_ms,changed_ms,deleted_ms FROM refund_links WHERE refund_id=?1",
            [r], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
        ).optional()?;
        if let Some(local) = local
            && rank(conn, &incoming)? <= rank(conn, &local)?
        {
            continue;
        }
        if deleted.is_none() {
            match validate(conn, r, o) {
                Ok(()) => {}
                Err(CoreError::InvalidInput { .. }) => continue,
                Err(error) => return Err(error),
            }
        }
        conn.execute(
            "INSERT INTO refund_links(refund_id,original_id,created_ms,changed_ms,deleted_ms) VALUES (?1,?2,?3,?4,?5)
             ON CONFLICT(refund_id) DO UPDATE SET original_id=excluded.original_id,created_ms=excluded.created_ms,changed_ms=excluded.changed_ms,deleted_ms=excluded.deleted_ms",
            params![r,o,created,changed,deleted],
        )?;
    }
    Ok(())
}

fn next_change(conn: &Connection, refund_id: i64) -> CoreResult<i64> {
    let previous: Option<i64> = conn
        .query_row(
            "SELECT changed_ms FROM refund_links WHERE refund_id=?1",
            [refund_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(crate::dates::now_ms().max(previous.unwrap_or(0).saturating_add(1)))
}

#[uniffi::export]
impl Vault {
    /// Links the incoming booking `refund_id` to the outgoing booking `original_id`; an existing
    /// link of the refund is replaced. Fails for wrong signs, deleted bookings and chains.
    pub fn link_refund(&self, refund_id: i64, original_id: i64) -> CoreResult<()> {
        let db = self.db()?;
        validate(&db, refund_id, original_id)?;
        let changed = next_change(&db, refund_id)?;
        db.execute("INSERT INTO refund_links(refund_id,original_id,created_ms,changed_ms,deleted_ms) VALUES (?1,?2,?3,?3,NULL) ON CONFLICT(refund_id) DO UPDATE SET original_id=excluded.original_id,changed_ms=excluded.changed_ms,deleted_ms=NULL",params![refund_id,original_id,changed])?;
        Ok(())
    }
    /// Removes the link of `refund_id` to its original (no-op without one).
    pub fn unlink_refund(&self, refund_id: i64) -> CoreResult<()> {
        let db = self.db()?;
        let changed = next_change(&db, refund_id)?;
        db.execute(
            // an already removed link keeps its stamp: a second removal must not outrank a newer re-link
            "UPDATE refund_links SET deleted_ms=?2,changed_ms=?2 WHERE refund_id=?1 AND deleted_ms IS NULL",
            params![refund_id, changed],
        )?;
        Ok(())
    }
    /// The live links of one booking; deleted bookings are left out.
    pub fn refund_links(&self, transaction_id: i64) -> CoreResult<RefundLinks> {
        let db = self.db()?;
        live(&db, transaction_id)?;
        let rules = self.merchant_rules(&db)?;
        let original=entries(&db,&format!("{ROWS} WHERE t.deleted_ms IS NULL AND t.id IN (SELECT original_id FROM refund_links WHERE refund_id=?1 AND deleted_ms IS NULL)"),[transaction_id],&rules)?.pop();
        let refunds = entries(
            &db,
            &format!(
                "{ROWS} WHERE t.deleted_ms IS NULL AND t.id IN (SELECT refund_id FROM refund_links WHERE original_id=?1 AND deleted_ms IS NULL) ORDER BY t.booking_date,t.id"
            ),
            [transaction_id],
            &rules,
        )?;
        Ok(RefundLinks { original, refunds })
    }
    /// The bookings `transaction_id` could be linked to, nearest date first: for a refund the
    /// outgoing bookings before it, for an outgoing booking the incoming ones after it that are
    /// not linked elsewhere; all accounts and currencies. Without `query` the last / next 365
    /// days, with one (name or purpose) every such booking. Resolves the merchant of every
    /// booking it reads, so call it off the main thread.
    pub fn refund_candidates(
        &self,
        transaction_id: i64,
        query: String,
        limit: u32,
    ) -> CoreResult<Vec<RefundBooking>> {
        let db = self.db()?;
        let query = query.trim().to_lowercase();
        if limit == 0 {
            live(&db, transaction_id)?;
            return Ok(Vec::new());
        }
        let days = query.is_empty().then_some(WINDOW_DAYS);
        let pool = pool(self, &db, transaction_id, days)?;
        Ok(pool
            .rows
            .into_iter()
            .map(|r| r.booking)
            .filter(|b| {
                query.is_empty()
                    || b.counterparty.to_lowercase().contains(&query)
                    || b.purpose.to_lowercase().contains(&query)
                    || b.display_name.to_lowercase().contains(&query)
            })
            .take(limit as usize)
            .collect())
    }

    /// Up to three bookings of [`refund_candidates`](Self::refund_candidates) (365 days) that most
    /// likely belong to `transaction_id`, filled in this order, the nearest date first within
    /// each: the same amount in the other direction, the same counterparty or merchant within
    /// two months, a shared reference in the texts. Bookings already linked to it are left out.
    pub fn refund_suggestions(&self, transaction_id: i64) -> CoreResult<Vec<RefundSuggestion>> {
        const MAX: usize = 3;
        let db = self.db()?;
        let pool = pool(self, &db, transaction_id, Some(WINDOW_DAYS))?;
        let linked: std::collections::HashSet<i64> = db
            .prepare(
                "SELECT original_id FROM refund_links WHERE refund_id=?1 AND deleted_ms IS NULL
                 UNION SELECT refund_id FROM refund_links WHERE original_id=?1 AND deleted_ms IS NULL",
            )?
            .query_map([transaction_id], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let source_refs = reference_tokens(&pool.reference);
        let counterparty = pool.counterparty.trim().to_lowercase();
        let matches = |row: &Row, reason: RefundMatch| -> bool {
            let b = &row.booking;
            match reason {
                RefundMatch::SameAmount => {
                    b.amount_cents == -pool.amount
                        && currency_code(&b.currency) == currency_code(&pool.currency)
                }
                RefundMatch::SameMerchant => {
                    (!counterparty.is_empty()
                        && b.counterparty.trim().to_lowercase() == counterparty
                        || !pool.merchant.is_empty() && b.display_name == pool.merchant)
                        && days_between(&b.booking_date, &pool.date)
                            .is_some_and(|d| d <= SUGGESTION_DAYS)
                }
                RefundMatch::SameReference => {
                    !source_refs.is_empty()
                        && !source_refs.is_disjoint(&reference_tokens(&format!(
                            "{} {}",
                            b.purpose, row.end_to_end_id
                        )))
                }
            }
        };
        let mut out: Vec<RefundSuggestion> = Vec::new();
        for reason in [
            RefundMatch::SameAmount,
            RefundMatch::SameMerchant,
            RefundMatch::SameReference,
        ] {
            for row in &pool.rows {
                if out.len() == MAX {
                    return Ok(out);
                }
                let id = row.booking.id;
                if linked.contains(&id) || out.iter().any(|s| s.booking.id == id) {
                    continue;
                }
                if matches(row, reason) {
                    out.push(RefundSuggestion {
                        booking: row.booking.clone(),
                        reason,
                    });
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests;
