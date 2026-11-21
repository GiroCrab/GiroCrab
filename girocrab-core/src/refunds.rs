//! Explicit refund links (#714): a refund (incoming booking) points at the booking it pays back
//! (outgoing). One original can have several refunds and one refund can pay back several originals
//! (step 37); links do not chain. A link to a split booking names one of its parts (step 38,
//! `original_part`): the refund pays back that part only. The link only documents what belongs together: amounts, categories and balances remain
//! unchanged. Removing a link keeps the row as a tombstone (`deleted_ms`), so the removal reaches
//! the other devices; the newest change wins on merge.
use crate::{
    error::{CoreError, CoreResult},
    i18n::tr,
    vault::Vault,
};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashMap;

/// Step 37: a refund may pay back several bookings, so the key is the pair.
pub(crate) const SCHEMA_PAIRS: &str = "
CREATE TABLE refund_links_pairs (
 refund_id INTEGER NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
 original_id INTEGER NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
 created_ms INTEGER NOT NULL,
 changed_ms INTEGER NOT NULL,
 deleted_ms INTEGER,
 PRIMARY KEY (refund_id, original_id)
);
INSERT INTO refund_links_pairs SELECT refund_id,original_id,created_ms,changed_ms,deleted_ms FROM refund_links;
DROP TABLE refund_links;
ALTER TABLE refund_links_pairs RENAME TO refund_links;
CREATE INDEX IF NOT EXISTS refund_links_original ON refund_links(original_id);
";

/// Step 38: a link to a split booking names the part it pays back: `original_part` is the
/// part's position among the booking's parts (from 1), 0 is the whole booking.
pub(crate) const SCHEMA_PARTS: &str = "
CREATE TABLE refund_links_parts (
 refund_id INTEGER NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
 original_id INTEGER NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
 original_part INTEGER NOT NULL DEFAULT 0,
 created_ms INTEGER NOT NULL,
 changed_ms INTEGER NOT NULL,
 deleted_ms INTEGER,
 PRIMARY KEY (refund_id, original_id, original_part)
);
INSERT INTO refund_links_parts SELECT refund_id,original_id,0,created_ms,changed_ms,deleted_ms FROM refund_links;
DROP TABLE refund_links;
ALTER TABLE refund_links_parts RENAME TO refund_links;
CREATE INDEX IF NOT EXISTS refund_links_original ON refund_links(original_id);
";

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
    /// In [`RefundLinks`]: how much of the refund this link pays back, at most the original's
    /// amount and at most what the refund has left after its earlier originals. `None` for a
    /// candidate and for a link between different currencies.
    pub share_cents: Option<i64>,
    /// What the booking has left for a further link: its amount less what its links cover. With
    /// 0 it is used up and cannot be linked again.
    pub open_cents: i64,
    /// The part of a split booking this row stands for (its position from 1, see
    /// `list_transaction_splits`); `amount_cents` is then the part's amount. 0 = the whole
    /// booking. On a refund in the links of a split original: the part of that original it
    /// pays back. Pass it to `link_refund` / `unlink_refund`.
    pub part: u32,
    /// Category name and note of that part, for the row; empty without a part.
    pub part_label: String,
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

/// The links of one booking: its originals when it is a refund, its refunds when it is an original.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RefundLinks {
    pub originals: Vec<RefundBooking>,
    pub refunds: Vec<RefundBooking>,
    /// For an original: how much of it its refunds pay back, never more than its amount. A
    /// refund counts once: what it pays back of one booking is not available for the next.
    /// `None` without refunds or when a linked booking has another currency (#569).
    pub refunded_cents: Option<i64>,
    /// For a refund: how much of it its originals take up, never more than its amount; the rest
    /// belongs to no linked booking. `None` without originals or with mixed currencies.
    pub assigned_cents: Option<i64>,
    /// What the booking itself has left for a further link ([`RefundBooking::open_cents`]).
    pub open_cents: i64,
}

/// One part of a split booking: amount and label (category name, note).
#[derive(Clone)]
struct Part {
    amount: i64,
    label: String,
}

/// The parts of every split booking in their order (`original_part` counts from 1).
fn split_parts(conn: &Connection) -> CoreResult<HashMap<i64, Vec<Part>>> {
    let mut out: HashMap<i64, Vec<Part>> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT s.transaction_id,s.amount_cents,COALESCE(c.name,''),s.note FROM transaction_splits s
         LEFT JOIN categories c ON c.id=s.category_id ORDER BY s.id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    for row in rows {
        let (id, amount, category, note) = row?;
        let label = [category.trim(), note.trim()]
            .into_iter()
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        out.entry(id).or_default().push(Part { amount, label });
    }
    Ok(out)
}

/// The part a link's `original_part` stands for now; `None` for the whole booking, also when
/// the booking has no such part any more (the split was changed or removed).
fn part_of(parts: &HashMap<i64, Vec<Part>>, original: i64, part: i64) -> Option<&Part> {
    let index = usize::try_from(part).ok()?.checked_sub(1)?;
    parts.get(&original)?.get(index)
}

/// What the live links pay back.
#[derive(Default)]
struct Shares {
    /// Per link (refund, original, `original_part`).
    links: HashMap<(i64, i64, i64), i64>,
    /// Links between different currencies: they get nothing.
    mixed: std::collections::HashSet<(i64, i64, i64)>,
    /// Per booking, and per part (booking, part from 1) of a split original.
    taken: HashMap<(i64, i64), i64>,
}

impl Shares {
    /// What a booking (`part` 0) or one part of it has left for a further link.
    fn open(&self, parts: &HashMap<i64, Vec<Part>>, id: i64, amount: i64, part: i64) -> i64 {
        let taken = |part: i64| self.taken.get(&(id, part)).copied().unwrap_or(0);
        let whole = amount.saturating_abs() - taken(0);
        match part_of(parts, id, part) {
            Some(p) => whole.min(p.amount.saturating_abs() - taken(part)),
            None => whole,
        }
        .max(0)
    }
}

/// What every live link pays back, filled in the order the links were made: a link takes at
/// most what the earlier links left of its refund and of its original (of the part it names).
/// Links between different currencies get nothing.
fn shares(conn: &Connection, parts: &HashMap<i64, Vec<Part>>) -> CoreResult<Shares> {
    type Link = (i64, i64, i64, i64, i64, String, String);
    let links: Vec<Link> = conn
        .prepare(
            "SELECT l.refund_id,l.original_id,l.original_part,r.amount_cents,o.amount_cents,r.currency,o.currency
             FROM refund_links l JOIN transactions r ON r.id=l.refund_id JOIN transactions o ON o.id=l.original_id
             WHERE l.deleted_ms IS NULL AND r.deleted_ms IS NULL AND o.deleted_ms IS NULL
             ORDER BY l.changed_ms,r.booking_date,o.booking_date,l.refund_id,l.original_id,l.original_part",
        )?
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        })?
        .collect::<Result<_, _>>()?;
    let mut out = Shares::default();
    for (r, o, part, r_amount, o_amount, r_cur, o_cur) in links {
        if currency_code(&r_cur) != currency_code(&o_cur) {
            out.mixed.insert((r, o, part));
            continue;
        }
        let share = out
            .open(parts, r, r_amount.max(0), 0)
            .min(out.open(parts, o, o_amount, part));
        *out.taken.entry((r, 0)).or_default() += share;
        *out.taken.entry((o, 0)).or_default() += share;
        if part_of(parts, o, part).is_some() {
            *out.taken.entry((o, part)).or_default() += share;
        }
        out.links.insert((r, o, part), share);
    }
    Ok(out)
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

/// [`validate`] for a link the user makes here: a split booking is linked by one of its
/// outgoing parts, and a booking (or part) that is used up takes no further link. Links
/// arriving from another device are not held to this; they get a share of 0.
fn validate_new(conn: &Connection, refund: i64, original: i64, part: i64) -> CoreResult<()> {
    validate(conn, refund, original)?;
    let (r, o) = (live(conn, refund)?, live(conn, original)?);
    let parts = split_parts(conn)?;
    let fits = match parts.get(&original) {
        Some(_) => part_of(&parts, original, part).is_some_and(|p| p.amount < 0),
        None => part == 0,
    };
    if !fits {
        return Err(CoreError::invalid(tr(
            "Bei einer geteilten Buchung wird ein Teil verknüpft.",
        )));
    }
    let linked: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM refund_links WHERE deleted_ms IS NULL AND refund_id=?1 AND original_id=?2 AND original_part=?3)",
        params![refund, original, part],
        |r| r.get(0),
    )?;
    if !linked {
        // a link that could take nothing would only mislead: the booking is used up
        let shares = shares(conn, &parts)?;
        if shares.open(&parts, refund, r, 0) == 0 {
            return Err(CoreError::invalid(tr(
                "Die Erstattung ist bereits vollständig zugeordnet.",
            )));
        }
        if shares.open(&parts, original, o, part) == 0 {
            return Err(CoreError::invalid(tr(
                "Die Buchung ist bereits vollständig erstattet.",
            )));
        }
    }
    Ok(())
}

/// `{part}`: the SQL of the part a row stands for (`0`, or the link's `l.original_part`).
const ROWS: &str = "SELECT t.id,t.account_id,a.name,t.booking_date,t.amount_cents,t.currency,t.counterparty,t.purpose,t.end_to_end_id,{part} FROM transactions t JOIN accounts a ON a.id=t.account_id";

/// A row of [`ROWS`] with what only the ranking needs.
struct Row {
    booking: RefundBooking,
    end_to_end_id: String,
}

/// `expand`: an outgoing split booking is returned as its outgoing parts, one row each,
/// instead of as a whole (the bookings a refund can be linked to).
fn rows(
    conn: &Connection,
    sql: &str,
    args: impl rusqlite::Params,
    rules: &crate::merchants::MerchantRules,
    expand: bool,
) -> CoreResult<Vec<Row>> {
    let read: Vec<Row> = conn
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
                    share_cents: None,
                    open_cents: 0,
                    part: r.get(9)?,
                    part_label: String::new(),
                },
                end_to_end_id: r.get(8)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    let parts = split_parts(conn)?;
    let shares = shares(conn, &parts)?;
    let mut rows = Vec::with_capacity(read.len());
    for row in read {
        let id = row.booking.id;
        let whole = row.booking.amount_cents;
        let own: Vec<u32> = match parts.get(&id) {
            Some(own) if expand && whole < 0 => (1..=own.len() as u32).collect(),
            _ => vec![row.booking.part],
        };
        for part in own {
            let split = part_of(&parts, id, i64::from(part));
            if expand && split.is_some_and(|p| p.amount >= 0) {
                continue;
            }
            let mut b = row.booking.clone();
            b.part = part;
            // of a refund the part belongs to the original it pays back: its amount stays
            if let Some(p) = split.filter(|_| whole < 0) {
                b.amount_cents = p.amount;
                b.part_label = p.label.clone();
            }
            b.open_cents = shares.open(
                &parts,
                id,
                whole,
                if whole < 0 { i64::from(part) } else { 0 },
            );
            rows.push(Row {
                booking: b,
                end_to_end_id: row.end_to_end_id.clone(),
            });
        }
    }
    Ok(rows)
}

/// Ids of the refunds with a live link. The booking list reads them as refunds, not as income
/// (`BookingFinisher`): a linked refund reduces the expenses instead of raising the income.
/// How much of every linked booking its links cover: of a refund what its originals take up, of
/// an original what its refunds pay back. A booking with a link between different currencies is
/// left out (nothing is added up there, #569).
pub(crate) fn covered(conn: &Connection) -> CoreResult<HashMap<i64, i64>> {
    let shares = shares(conn, &split_parts(conn)?)?;
    let mut out: HashMap<i64, i64> = shares
        .taken
        .iter()
        .filter(|((_, part), _)| *part == 0)
        .map(|((id, _), taken)| (*id, *taken))
        .collect();
    for (refund, original, _) in &shares.mixed {
        out.remove(refund);
        out.remove(original);
    }
    Ok(out)
}

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
/// Empty when the booking itself is the other end of a link (no chains).
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
        "t.amount_cents<0 AND julianday(?1)-julianday(t.booking_date) BETWEEN 0 AND ?3"
    } else {
        "t.amount_cents>0 AND julianday(t.booking_date)-julianday(?1) BETWEEN 0 AND ?3"
    };
    out.rows = rows(
        db,
        &format!(
            "{} WHERE t.deleted_ms IS NULL AND t.id<>?2 AND {direction} ORDER BY abs(julianday(t.booking_date)-julianday(?1)),t.id",
            ROWS.replace("{part}", "0")
        ),
        params![out.date, id, days],
        &rules,
        true,
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

type LinkRow = (i64, i64, i64, i64, Option<i64>, i64);
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
            "SELECT refund_id,original_id,created_ms,changed_ms,deleted_ms,original_part FROM other.refund_links",
        )?
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, i64>(5)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    'links: for (r, o, created, changed, deleted, part) in links {
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
        let incoming = (r, o, created, changed, deleted, part);
        let local: Option<LinkRow> = conn.query_row(
            "SELECT refund_id,original_id,created_ms,changed_ms,deleted_ms,original_part FROM refund_links WHERE refund_id=?1 AND original_id=?2 AND original_part=?3",
            [r, o, part], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)),
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
            "INSERT INTO refund_links(refund_id,original_id,original_part,created_ms,changed_ms,deleted_ms) VALUES (?1,?2,?6,?3,?4,?5)
             ON CONFLICT(refund_id,original_id,original_part) DO UPDATE SET created_ms=excluded.created_ms,changed_ms=excluded.changed_ms,deleted_ms=excluded.deleted_ms",
            params![r,o,created,changed,deleted,part],
        )?;
    }
    Ok(())
}

fn next_change(conn: &Connection) -> CoreResult<i64> {
    // later than every other link too: the shares are filled in the order the links were made
    let previous: Option<i64> =
        conn.query_row("SELECT MAX(changed_ms) FROM refund_links", [], |r| r.get(0))?;
    Ok(crate::dates::now_ms().max(previous.unwrap_or(0).saturating_add(1)))
}

#[uniffi::export]
impl Vault {
    /// Links the incoming booking `refund_id` to the outgoing booking `original_id`; its other
    /// links stay (a refund can pay back several bookings, a booking can have several refunds).
    /// `original_part`: for a split booking the part the refund pays back (its position from 1
    /// in `list_transaction_splits`), else 0. Fails for wrong signs, deleted bookings, chains,
    /// a split booking without a part and bookings that are used up.
    pub fn link_refund(
        &self,
        refund_id: i64,
        original_id: i64,
        original_part: u32,
    ) -> CoreResult<()> {
        let db = self.db()?;
        validate_new(&db, refund_id, original_id, i64::from(original_part))?;
        let changed = next_change(&db)?;
        db.execute("INSERT INTO refund_links(refund_id,original_id,original_part,created_ms,changed_ms,deleted_ms) VALUES (?1,?2,?4,?3,?3,NULL) ON CONFLICT(refund_id,original_id,original_part) DO UPDATE SET changed_ms=excluded.changed_ms,deleted_ms=NULL WHERE deleted_ms IS NOT NULL",params![refund_id,original_id,changed,original_part])?;
        Ok(())
    }
    /// Removes the link of `refund_id` to `original_id` (its part `original_part`); no-op
    /// without one.
    pub fn unlink_refund(
        &self,
        refund_id: i64,
        original_id: i64,
        original_part: u32,
    ) -> CoreResult<()> {
        let db = self.db()?;
        let changed = next_change(&db)?;
        db.execute(
            // an already removed link keeps its stamp: a second removal must not outrank a newer re-link
            "UPDATE refund_links SET deleted_ms=?3,changed_ms=?3 WHERE refund_id=?1 AND original_id=?2 AND original_part=?4 AND deleted_ms IS NULL",
            params![refund_id, original_id, changed, original_part],
        )?;
        Ok(())
    }
    /// The live links of one booking; deleted bookings are left out.
    pub fn refund_links(&self, transaction_id: i64) -> CoreResult<RefundLinks> {
        let db = self.db()?;
        live(&db, transaction_id)?;
        let rules = self.merchant_rules(&db)?;
        let side = |own: &str, other: &str| {
            rows(
                &db,
                &format!(
                    "{} JOIN refund_links l ON l.{other}=t.id AND l.{own}=?1 AND l.deleted_ms IS NULL WHERE t.deleted_ms IS NULL ORDER BY l.changed_ms,t.id,l.original_part",
                    ROWS.replace("{part}", "l.original_part")
                ),
                [transaction_id],
                &rules,
                false,
            )
            .map(|rows| rows.into_iter().map(|r| r.booking).collect::<Vec<_>>())
        };
        let mut originals = side("refund_id", "original_id")?;
        let mut refunds = side("original_id", "refund_id")?;
        let parts = split_parts(&db)?;
        let shares = shares(&db, &parts)?;
        let own_amount: i64 = db.query_row(
            "SELECT amount_cents FROM transactions WHERE id=?1",
            [transaction_id],
            |r| r.get(0),
        )?;
        for o in &mut originals {
            o.share_cents = shares
                .links
                .get(&(transaction_id, o.id, i64::from(o.part)))
                .copied();
        }
        for r in &mut refunds {
            r.share_cents = shares
                .links
                .get(&(r.id, transaction_id, i64::from(r.part)))
                .copied();
            if let Some(p) = part_of(&parts, transaction_id, i64::from(r.part)) {
                r.part_label = p.label.clone();
            }
        }
        let sum = |links: Vec<(i64, i64, i64)>| -> Option<i64> {
            (!links.is_empty() && !links.iter().any(|l| shares.mixed.contains(l)))
                .then(|| links.iter().filter_map(|l| shares.links.get(l)).sum())
        };
        Ok(RefundLinks {
            refunded_cents: sum(refunds
                .iter()
                .map(|r| (r.id, transaction_id, i64::from(r.part)))
                .collect()),
            assigned_cents: sum(originals
                .iter()
                .map(|o| (transaction_id, o.id, i64::from(o.part)))
                .collect()),
            open_cents: shares.open(&parts, transaction_id, own_amount, 0),
            originals,
            refunds,
        })
    }
    /// The bookings `transaction_id` could be linked to, nearest date first: for a refund the
    /// outgoing bookings before it, for an outgoing booking the incoming ones after it; all
    /// accounts and currencies. Without `query` the last / next 365
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
                    || b.part_label.to_lowercase().contains(&query)
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
        // a booking that is used up itself takes no further link: nothing to suggest
        let parts = split_parts(&db)?;
        if shares(&db, &parts)?.open(&parts, transaction_id, pool.amount, 0) == 0 {
            return Ok(Vec::new());
        }
        let linked: std::collections::HashSet<(i64, u32)> = db
            .prepare(
                "SELECT original_id,original_part FROM refund_links WHERE refund_id=?1 AND deleted_ms IS NULL
                 UNION SELECT refund_id,0 FROM refund_links WHERE original_id=?1 AND deleted_ms IS NULL",
            )?
            .query_map([transaction_id], |r| Ok((r.get(0)?, r.get(1)?)))?
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
                let id = (row.booking.id, row.booking.part);
                // a booking that is used up cannot be linked: no suggestion
                if linked.contains(&id)
                    || row.booking.open_cents == 0
                    || out.iter().any(|s| (s.booking.id, s.booking.part) == id)
                {
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
mod qa_tests;
#[cfg(test)]
mod tests;
