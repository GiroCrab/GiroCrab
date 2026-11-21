//! Explicit refund links (#714): a refund (incoming booking) points at the booking it pays back
//! (outgoing). One original can have several refunds and one refund can pay back several originals
//! (step 38); links do not chain. A link to a split booking names one of its parts (step 39,
//! `original_part`): the refund pays back that part only. A booking that is split differently
//! or not at all any more keeps its refunds as links to the whole booking (`split_changed`);
//! such a link to a split booking fills its parts from the last to the first. A split refund
//! works the same way (step 40, `refund_part`): the link names the part of the income that is
//! the refund, and a link to the whole of a split refund takes from its parts from the last to
//! the first. The link only documents what belongs together: amounts, categories and balances remain
//! unchanged. Removing a link keeps the row as a tombstone (`deleted_ms`), so the removal reaches
//! the other devices; the newest change wins on merge.
use crate::{
    error::{CoreError, CoreResult},
    i18n::tr,
    vault::Vault,
};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashMap;

/// Step 38: a refund may pay back several bookings, so the key is the pair.
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

/// Step 39: a link to a split booking names the part it pays back: `original_part` is the
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

/// Step 40: a link from a split refund names the part of it that pays back: `refund_part` is
/// the part's position among the refund's parts (from 1), 0 is the whole refund.
pub(crate) const SCHEMA_SIDES: &str = "
CREATE TABLE refund_links_sides (
 refund_id INTEGER NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
 original_id INTEGER NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
 original_part INTEGER NOT NULL DEFAULT 0,
 refund_part INTEGER NOT NULL DEFAULT 0,
 created_ms INTEGER NOT NULL,
 changed_ms INTEGER NOT NULL,
 deleted_ms INTEGER,
 PRIMARY KEY (refund_id, original_id, original_part, refund_part)
);
INSERT INTO refund_links_sides SELECT refund_id,original_id,original_part,0,created_ms,changed_ms,deleted_ms FROM refund_links;
DROP TABLE refund_links;
ALTER TABLE refund_links_sides RENAME TO refund_links;
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
    /// The part of a split refund that pays back (from 1), 0 = the whole refund: on a refund
    /// the part this row stands for (`amount_cents` is then the part's amount), on an original
    /// in the links of a split refund the part of that refund. Pass it to `link_refund_parts`
    /// / `unlink_refund_parts`.
    pub refund_part: u32,
    /// Category name and note of that part of the refund; empty without one.
    pub refund_part_label: String,
}

impl RefundBooking {
    /// The part of its own booking the row stands for.
    fn own_part(&self) -> u32 {
        if self.amount_cents < 0 {
            self.part
        } else {
            self.refund_part
        }
    }
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
    /// Row id in `transaction_splits`.
    id: i64,
    amount: i64,
    category: Option<i64>,
    label: String,
}

/// The parts of every split booking in their order (`original_part` counts from 1).
fn split_parts(conn: &Connection) -> CoreResult<HashMap<i64, Vec<Part>>> {
    let mut out: HashMap<i64, Vec<Part>> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT s.transaction_id,s.amount_cents,COALESCE(c.name,''),s.note,s.category_id,s.id FROM transaction_splits s
         LEFT JOIN categories c ON c.id=s.category_id ORDER BY s.id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<i64>>(4)?,
            r.get::<_, i64>(5)?,
        ))
    })?;
    for row in rows {
        let (id, amount, name, note, category, split) = row?;
        let label = [name.trim(), note.trim()]
            .into_iter()
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        out.entry(id).or_default().push(Part {
            id: split,
            amount,
            category,
            label,
        });
    }
    Ok(out)
}

/// The part at position `part` (from 1) of a split booking; `None` for the whole booking, also
/// when the booking has no such part any more (the split was changed or removed).
fn part_of(parts: &HashMap<i64, Vec<Part>>, booking: i64, part: i64) -> Option<&Part> {
    let index = usize::try_from(part).ok()?.checked_sub(1)?;
    parts.get(&booking)?.get(index)
}

/// [`part_of`] for a link: only a part on the side of its booking (an outgoing part of an
/// original, an incoming one of a refund of `amount`) pays back or is paid back.
fn linked_part(
    parts: &HashMap<i64, Vec<Part>>,
    booking: i64,
    amount: i64,
    part: i64,
) -> Option<&Part> {
    part_of(parts, booking, part).filter(|p| p.amount.signum() == amount.signum())
}

/// A link: refund, original, `original_part`, `refund_part`.
type Link = (i64, i64, i64, i64);

/// What the live links pay back.
#[derive(Default)]
struct Shares {
    links: HashMap<Link, i64>,
    /// Links between different currencies: they get nothing.
    mixed: std::collections::HashSet<Link>,
    /// Per booking, and per part (booking, part from 1) of a split booking.
    taken: HashMap<(i64, i64), i64>,
    /// Per link to a split original: what it pays back of which part (from 1).
    into: HashMap<Link, Vec<(i64, i64)>>,
    /// Per link of a split refund: what it takes from which part (from 1).
    from: HashMap<Link, Vec<(i64, i64)>>,
}

impl Shares {
    /// What a booking (`part` 0) or one part of it has left for a further link.
    fn open(&self, parts: &HashMap<i64, Vec<Part>>, id: i64, amount: i64, part: i64) -> i64 {
        let taken = |part: i64| self.taken.get(&(id, part)).copied().unwrap_or(0);
        let whole = amount.saturating_abs() - taken(0);
        match linked_part(parts, id, amount, part) {
            Some(p) => whole.min(p.amount.saturating_abs() - taken(part)),
            None => whole,
        }
        .max(0)
    }

    /// Books `share` on the booking `id` of `amount` and says which of its parts carry it: the
    /// part the link names, else (a link to the whole of a split booking, or to a part it does
    /// not have) the parts on the booking's side from the last to the first, each until the
    /// share runs out. Empty for a booking without parts.
    fn place(
        &mut self,
        parts: &HashMap<i64, Vec<Part>>,
        id: i64,
        amount: i64,
        part: i64,
        share: i64,
    ) -> Vec<(i64, i64)> {
        *self.taken.entry((id, 0)).or_default() += share;
        if linked_part(parts, id, amount, part).is_some() {
            *self.taken.entry((id, part)).or_default() += share;
            return vec![(part, share)];
        }
        let mut left = share;
        let mut placed = Vec::new();
        let own = parts.get(&id).map(Vec::as_slice).unwrap_or_default();
        for (n, p) in own.iter().enumerate().rev() {
            if p.amount.signum() != amount.signum() {
                continue;
            }
            let n = n as i64 + 1;
            let taken = self.taken.entry((id, n)).or_default();
            let take = left.min(p.amount.saturating_abs() - *taken);
            if take > 0 {
                *taken += take;
                left -= take;
                placed.push((n, take));
            }
        }
        placed
    }
}

/// The live links between a refund (incoming) and an original (outgoing) that both exist. A
/// booking whose amount changed its side since is no refund or original any more.
const LIVE: &str = "l.deleted_ms IS NULL AND r.deleted_ms IS NULL AND o.deleted_ms IS NULL AND r.amount_cents>0 AND o.amount_cents<0";

/// What every live link pays back, filled in the order the links were made: a link takes at
/// most what the earlier links left of its refund and of its original (of the parts it names).
/// A link to the whole of a split booking (it names no part, or one the booking does not
/// have) uses the parts on that booking's side from the last to the first, each until the
/// share runs out. Links between different currencies get nothing.
fn shares(conn: &Connection, parts: &HashMap<i64, Vec<Part>>) -> CoreResult<Shares> {
    type Row = (i64, i64, i64, i64, i64, i64, String, String);
    let links: Vec<Row> = conn
        .prepare(&format!(
            "SELECT l.refund_id,l.original_id,l.original_part,l.refund_part,r.amount_cents,o.amount_cents,r.currency,o.currency
             FROM refund_links l JOIN transactions r ON r.id=l.refund_id JOIN transactions o ON o.id=l.original_id
             WHERE {LIVE}
             ORDER BY l.changed_ms,r.booking_date,o.booking_date,l.refund_id,l.original_id,l.original_part,l.refund_part"
        ))?
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
            ))
        })?
        .collect::<Result<_, _>>()?;
    let mut out = Shares::default();
    for (r, o, part, refund_part, r_amount, o_amount, r_cur, o_cur) in links {
        let link = (r, o, part, refund_part);
        if currency_code(&r_cur) != currency_code(&o_cur) {
            out.mixed.insert(link);
            continue;
        }
        let share = out
            .open(parts, r, r_amount, refund_part)
            .min(out.open(parts, o, o_amount, part));
        let from = out.place(parts, r, r_amount, refund_part, share);
        let into = out.place(parts, o, o_amount, part, share);
        out.from.insert(link, from);
        out.into.insert(link, into);
        out.links.insert(link, share);
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
/// outgoing parts, a split refund by one of its incoming parts or as a whole (`refund_part`
/// 0), and a booking (or part) that is used up takes no further link. Links arriving from
/// another device are not held to this; they get a share of 0.
fn validate_new(
    conn: &Connection,
    refund: i64,
    original: i64,
    part: i64,
    refund_part: i64,
) -> CoreResult<()> {
    validate(conn, refund, original)?;
    let (r, o) = (live(conn, refund)?, live(conn, original)?);
    let parts = split_parts(conn)?;
    let fits = match parts.get(&original) {
        Some(_) => linked_part(&parts, original, o, part).is_some(),
        None => part == 0,
    } && (refund_part == 0 || linked_part(&parts, refund, r, refund_part).is_some());
    if !fits {
        return Err(CoreError::invalid(tr(
            "Bei einer geteilten Buchung wird ein Teil verknüpft.",
        )));
    }
    let linked: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM refund_links WHERE deleted_ms IS NULL AND refund_id=?1 AND original_id=?2 AND original_part=?3 AND refund_part=?4)",
        params![refund, original, part, refund_part],
        |r| r.get(0),
    )?;
    if !linked {
        // a link that could take nothing would only mislead: the booking is used up
        let shares = shares(conn, &parts)?;
        if shares.open(&parts, refund, r, refund_part) == 0 {
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

/// `{part}`, `{refund_part}`: the SQL of the parts a row stands for (`0`, or the link's
/// `l.original_part` and `l.refund_part`).
const ROWS: &str = "SELECT t.id,t.account_id,a.name,t.booking_date,t.amount_cents,t.currency,t.counterparty,t.purpose,t.end_to_end_id,{part},{refund_part} FROM transactions t JOIN accounts a ON a.id=t.account_id";

/// [`ROWS`] for bookings that are not read through a link.
fn plain_rows() -> String {
    ROWS.replace("{part}", "0").replace("{refund_part}", "0")
}

/// A row of [`ROWS`] with what only the ranking needs.
struct Row {
    booking: RefundBooking,
    end_to_end_id: String,
}

/// `expand`: a split booking is returned as the parts on its side (the outgoing parts of an
/// outgoing booking, the incoming ones of an incoming booking), one row each, instead of as a
/// whole: the bookings a link can be made to.
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
                    refund_part: r.get(10)?,
                    refund_part_label: String::new(),
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
            Some(own) if expand => (1..=own.len() as u32).collect(),
            _ => vec![row.booking.own_part()],
        };
        for part in own {
            let split = linked_part(&parts, id, whole, i64::from(part));
            if expand && part > 0 && split.is_none() {
                continue;
            }
            let mut b = row.booking.clone();
            // the row stands for a part of its own booking: an original for one of its
            // outgoing parts, a refund for one of its incoming parts
            let (own, label) = if whole < 0 {
                (&mut b.part, &mut b.part_label)
            } else {
                (&mut b.refund_part, &mut b.refund_part_label)
            };
            *own = part;
            if let Some(p) = split {
                *label = p.label.clone();
                b.amount_cents = p.amount;
            }
            b.open_cents = shares.open(&parts, id, whole, i64::from(part));
            rows.push(Row {
                booking: b,
                end_to_end_id: row.end_to_end_id.clone(),
            });
        }
    }
    Ok(rows)
}

/// How much of every linked booking its links cover: of a refund what its originals take up, of
/// an original what its refunds pay back.
pub(crate) struct Covered {
    /// Per booking with a link that gets a share (also one of 0).
    pub(crate) cents: HashMap<i64, i64>,
    /// The bookings with a link between different currencies: nothing is added up for them
    /// (#569), so their sum is not shown.
    pub(crate) mixed: std::collections::HashSet<i64>,
}

pub(crate) fn covered(conn: &Connection) -> CoreResult<Covered> {
    let shares = shares(conn, &split_parts(conn)?)?;
    Ok(Covered {
        cents: shares
            .taken
            .iter()
            .filter(|((_, part), _)| *part == 0)
            .map(|((id, _), taken)| (*id, *taken))
            .collect(),
        mixed: shares
            .mixed
            .iter()
            .flat_map(|(refund, original, ..)| [*refund, *original])
            .collect(),
    })
}

/// What the links move between categories. The part of a refund that pays back a booking
/// counts in that booking's category, or in the categories of the split parts it pays back
/// (`Shares::place`); what is left of the refund stays where it is.
#[derive(Default)]
pub(crate) struct Moved {
    /// (refund, category it counts in, cents).
    into: Vec<(i64, Option<i64>, i64)>,
    /// What leaves a refund without parts: (refund, cents).
    paid: HashMap<i64, i64>,
    /// What leaves the parts of a split refund: (refund, row id of the part in
    /// `transaction_splits`, the part's category, cents).
    drawn: Vec<(i64, i64, Option<i64>, i64)>,
}

pub(crate) fn moves(conn: &Connection) -> CoreResult<Moved> {
    let has_links: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM refund_links WHERE deleted_ms IS NULL)",
        [],
        |r| r.get(0),
    )?;
    if !has_links {
        return Ok(Moved::default());
    }
    let parts = split_parts(conn)?;
    let shares = shares(conn, &parts)?;
    let own: HashMap<i64, Option<i64>> = conn
        .prepare(
            "SELECT DISTINCT o.id,o.category_id FROM refund_links l
             JOIN transactions o ON o.id=l.original_id WHERE l.deleted_ms IS NULL",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let mut out = Moved::default();
    let mut drawn: HashMap<(i64, i64), (Option<i64>, i64)> = HashMap::new();
    for (link, share) in shares.links.iter().filter(|(_, share)| **share > 0) {
        let (refund, original, ..) = *link;
        match parts.get(&original) {
            Some(_) => {
                for (n, cents) in shares.into.get(link).into_iter().flatten() {
                    if let Some(p) = part_of(&parts, original, *n) {
                        out.into.push((refund, p.category, *cents));
                    }
                }
            }
            None => {
                if let Some(category) = own.get(&original) {
                    out.into.push((refund, *category, *share));
                }
            }
        }
        match parts.get(&refund) {
            Some(_) => {
                for (n, cents) in shares.from.get(link).into_iter().flatten() {
                    if let Some(p) = part_of(&parts, refund, *n) {
                        drawn.entry((refund, p.id)).or_insert((p.category, 0)).1 += cents;
                    }
                }
            }
            None => *out.paid.entry(refund).or_default() += share,
        }
    }
    out.into.sort_unstable();
    out.drawn = drawn
        .into_iter()
        .map(|((refund, split), (category, cents))| (refund, split, category, cents))
        .collect();
    out.drawn.sort_unstable();
    Ok(out)
}

/// [`Moved`] as table expressions for SQL sums.
pub(crate) struct MovedSql {
    /// (refund_id, category_id, amount_cents): what counts where the paid-back booking is.
    pub(crate) into: String,
    /// (refund_id, cents): what leaves a refund without parts.
    pub(crate) paid: String,
    /// (split_id, cents): what leaves a part of a split refund.
    pub(crate) drawn: String,
}

/// `None` when no link moves anything.
pub(crate) fn moves_sql(conn: &Connection) -> CoreResult<Option<MovedSql>> {
    let moved = moves(conn)?;
    if moved.into.is_empty() {
        return Ok(None);
    }
    // a table of integers; without rows a query that returns none (`VALUES` needs one)
    let table = |columns: &[&str], rows: Vec<String>| {
        let names: Vec<String> = columns
            .iter()
            .enumerate()
            .map(|(i, name)| format!("column{} AS {name}", i + 1))
            .collect();
        if rows.is_empty() {
            let nulls: Vec<String> = columns.iter().map(|c| format!("NULL AS {c}")).collect();
            format!("(SELECT {} WHERE 0)", nulls.join(", "))
        } else {
            format!(
                "(SELECT {} FROM (VALUES {}))",
                names.join(", "),
                rows.join(",")
            )
        }
    };
    let mut paid: Vec<(i64, i64)> = moved.paid.into_iter().collect();
    paid.sort_unstable();
    Ok(Some(MovedSql {
        into: table(
            &["refund_id", "category_id", "amount_cents"],
            moved
                .into
                .iter()
                .map(|(refund, category, cents)| {
                    let category = category.map_or("NULL".to_string(), |c| c.to_string());
                    format!("({refund},{category},{cents})")
                })
                .collect(),
        ),
        paid: table(
            &["refund_id", "cents"],
            paid.iter()
                .map(|(refund, cents)| format!("({refund},{cents})"))
                .collect(),
        ),
        drawn: table(
            &["split_id", "cents"],
            moved
                .drawn
                .iter()
                .map(|(_, split, _, cents)| format!("({split},{cents})"))
                .collect(),
        ),
    }))
}

/// The moved refund parts a category filter has to count (`moves`).
pub(crate) struct CategoryRefunds {
    /// Per refund: what its links pay back in total.
    assigned: HashMap<i64, i64>,
    /// Per refund: what of that counts in the filtered categories.
    into: HashMap<i64, i64>,
    /// Per split refund: what its links take from its parts in the filtered categories.
    out_of: HashMap<i64, i64>,
}

impl CategoryRefunds {
    /// `categories`: the filtered categories (`None` = bookings without one). `None` when no
    /// link moves anything.
    pub(crate) fn load(conn: &Connection, categories: &[Option<i64>]) -> CoreResult<Option<Self>> {
        let moved = moves(conn)?;
        if moved.into.is_empty() {
            return Ok(None);
        }
        let mut out = Self {
            assigned: HashMap::new(),
            into: HashMap::new(),
            out_of: HashMap::new(),
        };
        for (refund, category, cents) in moved.into {
            *out.assigned.entry(refund).or_default() += cents;
            if categories.contains(&category) {
                *out.into.entry(refund).or_default() += cents;
            }
        }
        for (refund, _, category, cents) in moved.drawn {
            let taken = out.out_of.entry(refund).or_default();
            if categories.contains(&category) {
                *taken += cents;
            }
        }
        Ok(Some(out))
    }

    /// The refunds that count in the filtered categories through a link.
    pub(crate) fn moved_in(&self) -> impl Iterator<Item = i64> + '_ {
        self.into.keys().copied()
    }

    /// `models::split_flow_parts` under a category filter: `share` is what the booking's own
    /// category (or its split parts) puts into the filter. Of a linked refund only the rest its
    /// links leave counts there; what they pay back counts where the paid-back booking is.
    pub(crate) fn flow_parts(
        this: Option<&Self>,
        id: i64,
        flow: crate::merchants::FlowKind,
        share_cents: i64,
        amount_cents: i64,
        refund_income_cents: i64,
    ) -> [(crate::merchants::FlowKind, i64); 2] {
        let Some((this, assigned)) = this
            .and_then(|m| Some((m, m.assigned.get(&id)?)))
            .filter(|_| amount_cents > 0)
        else {
            return crate::models::split_flow_parts(
                flow,
                share_cents,
                amount_cents,
                refund_income_cents,
            );
        };
        let rest = match this.out_of.get(&id) {
            // a split refund: what its links leave of its parts in the filter
            Some(taken) => (share_cents - taken).max(0),
            // else the refund is in the filter as a whole or not at all
            None => {
                let open = (amount_cents - assigned).max(0);
                (i128::from(open) * i128::from(share_cents) / i128::from(amount_cents)) as i64
            }
        };
        let income = if refund_income_cents > 0 { rest } else { 0 };
        let into = this.into.get(&id).copied().unwrap_or(0);
        [
            (flow, rest - income + into),
            (crate::merchants::FlowKind::Income, income),
        ]
    }
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
            plain_rows()
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
    let pairs = conn.prepare(&format!("SELECT refund_id,original_id FROM refund_links l JOIN transactions r ON r.id=l.refund_id JOIN transactions o ON o.id=l.original_id WHERE {LIVE}"))?
 .query_map([], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?)))?.collect::<Result<Vec<_>,_>>()?;
    for (r, o) in pairs {
        out.insert(r, RefundLinkKind::Refund);
        out.insert(o, RefundLinkKind::Original);
    }
    Ok(out)
}

type LinkRow = (i64, i64, i64, i64, Option<i64>, i64, i64);
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
            "SELECT refund_id,original_id,created_ms,changed_ms,deleted_ms,original_part,refund_part FROM other.refund_links",
        )?
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    'links: for (r, o, created, changed, deleted, part, refund_part) in links {
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
        let incoming = (r, o, created, changed, deleted, part, refund_part);
        let local: Option<LinkRow> = conn.query_row(
            "SELECT refund_id,original_id,created_ms,changed_ms,deleted_ms,original_part,refund_part FROM refund_links WHERE refund_id=?1 AND original_id=?2 AND original_part=?3 AND refund_part=?4",
            [r, o, part, refund_part], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?)),
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
            "INSERT INTO refund_links(refund_id,original_id,original_part,refund_part,created_ms,changed_ms,deleted_ms) VALUES (?1,?2,?6,?7,?3,?4,?5)
             ON CONFLICT(refund_id,original_id,original_part,refund_part) DO UPDATE SET created_ms=excluded.created_ms,changed_ms=excluded.changed_ms,deleted_ms=excluded.deleted_ms",
            params![r,o,created,changed,deleted,part,refund_part],
        )?;
    }
    Ok(())
}

/// The booking `id` is split differently or not at all any more: the parts its links name are
/// gone, so every such link becomes a link to the whole booking. The old row stays as a
/// tombstone and the new one gets a new stamp (both reach the other devices); the links keep
/// their order among themselves.
pub(crate) fn split_changed(conn: &Connection, id: i64) -> CoreResult<()> {
    let links: Vec<(i64, i64, i64, i64, i64)> = conn
        .prepare(
            "SELECT refund_id,original_id,original_part,refund_part,created_ms FROM refund_links
             WHERE deleted_ms IS NULL AND ((original_id=?1 AND original_part>0) OR (refund_id=?1 AND refund_part>0))
             ORDER BY changed_ms,refund_id,original_id,original_part,refund_part",
        )?
        .query_map([id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?
        .collect::<Result<_, _>>()?;
    for (refund, original, part, refund_part, created) in links {
        let changed = next_change(conn)?;
        conn.execute(
            "UPDATE refund_links SET deleted_ms=?5,changed_ms=?5 WHERE refund_id=?1 AND original_id=?2 AND original_part=?3 AND refund_part=?4",
            params![refund, original, part, refund_part, changed],
        )?;
        let (part, refund_part) = if original == id {
            (0, refund_part)
        } else {
            (part, 0)
        };
        // a booking linked by several of its parts keeps one link
        conn.execute(
            "INSERT INTO refund_links(refund_id,original_id,original_part,refund_part,created_ms,changed_ms,deleted_ms) VALUES (?1,?2,?3,?4,?5,?6,NULL)
             ON CONFLICT(refund_id,original_id,original_part,refund_part) DO UPDATE SET changed_ms=excluded.changed_ms,deleted_ms=NULL WHERE deleted_ms IS NOT NULL",
            params![refund, original, part, refund_part, created, changed + 1],
        )?;
    }
    Ok(())
}

/// The booking `id` changed its side (an outgoing booking became an incoming one or the other
/// way round, or its amount is 0): it is no refund or original any more, so its links are
/// removed like by hand.
pub(crate) fn side_changed(conn: &Connection, id: i64) -> CoreResult<()> {
    let changed = next_change(conn)?;
    conn.execute(
        "UPDATE refund_links SET deleted_ms=?2,changed_ms=?2 WHERE deleted_ms IS NULL AND (refund_id=?1 OR original_id=?1)",
        params![id, changed],
    )?;
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
    /// a split booking without a part and bookings that are used up. A split refund is linked
    /// as a whole; `link_refund_parts` names the part of it that is the refund.
    pub fn link_refund(
        &self,
        refund_id: i64,
        original_id: i64,
        original_part: u32,
    ) -> CoreResult<()> {
        self.link_refund_parts(refund_id, 0, original_id, original_part)
    }
    /// `link_refund` for a split refund: `refund_part` is the part of the income that pays
    /// back (its position from 1 in `list_transaction_splits`), at most with its amount; 0 is
    /// the whole refund.
    pub fn link_refund_parts(
        &self,
        refund_id: i64,
        refund_part: u32,
        original_id: i64,
        original_part: u32,
    ) -> CoreResult<()> {
        let db = self.db()?;
        validate_new(
            &db,
            refund_id,
            original_id,
            i64::from(original_part),
            i64::from(refund_part),
        )?;
        let changed = next_change(&db)?;
        db.execute("INSERT INTO refund_links(refund_id,original_id,original_part,refund_part,created_ms,changed_ms,deleted_ms) VALUES (?1,?2,?4,?5,?3,?3,NULL) ON CONFLICT(refund_id,original_id,original_part,refund_part) DO UPDATE SET changed_ms=excluded.changed_ms,deleted_ms=NULL WHERE deleted_ms IS NOT NULL",params![refund_id,original_id,changed,original_part,refund_part])?;
        Ok(())
    }
    /// Removes the links of `refund_id` to `original_id` (its part `original_part`), whatever
    /// part of the refund they name; no-op without one.
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
    /// Removes the one link of the part `refund_part` of `refund_id` to `original_id` (its
    /// part `original_part`); no-op without it.
    pub fn unlink_refund_parts(
        &self,
        refund_id: i64,
        refund_part: u32,
        original_id: i64,
        original_part: u32,
    ) -> CoreResult<()> {
        let db = self.db()?;
        let changed = next_change(&db)?;
        db.execute(
            "UPDATE refund_links SET deleted_ms=?3,changed_ms=?3 WHERE refund_id=?1 AND original_id=?2 AND original_part=?4 AND refund_part=?5 AND deleted_ms IS NULL",
            params![refund_id, original_id, changed, original_part, refund_part],
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
                    "{} JOIN refund_links l ON l.{other}=t.id AND l.{own}=?1 AND l.deleted_ms IS NULL WHERE t.deleted_ms IS NULL ORDER BY l.changed_ms,t.id,l.original_part,l.refund_part",
                    ROWS.replace("{part}", "l.original_part")
                        .replace("{refund_part}", "l.refund_part")
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
        let link = |refund: i64, original: i64, row: &RefundBooking| -> Link {
            (
                refund,
                original,
                i64::from(row.part),
                i64::from(row.refund_part),
            )
        };
        // the part of this booking a link names, for the row of the booking on its other end
        let own_label = |part: u32| {
            linked_part(&parts, transaction_id, own_amount, i64::from(part))
                .map(|p| p.label.clone())
                .unwrap_or_default()
        };
        for o in &mut originals {
            o.share_cents = shares.links.get(&link(transaction_id, o.id, o)).copied();
            o.refund_part_label = own_label(o.refund_part);
        }
        for r in &mut refunds {
            r.share_cents = shares.links.get(&link(r.id, transaction_id, r)).copied();
            r.part_label = own_label(r.part);
        }
        let sum = |links: Vec<Link>| -> Option<i64> {
            (!links.is_empty() && !links.iter().any(|l| shares.mixed.contains(l)))
                .then(|| links.iter().filter_map(|l| shares.links.get(l)).sum())
        };
        Ok(RefundLinks {
            refunded_cents: sum(refunds
                .iter()
                .map(|r| link(r.id, transaction_id, r))
                .collect()),
            assigned_cents: sum(originals
                .iter()
                .map(|o| link(transaction_id, o.id, o))
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
                    || b.refund_part_label.to_lowercase().contains(&query)
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
                 UNION SELECT refund_id,refund_part FROM refund_links WHERE original_id=?1 AND deleted_ms IS NULL",
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
                let id = (row.booking.id, row.booking.own_part());
                // a booking that is used up cannot be linked: no suggestion
                if linked.contains(&id)
                    || row.booking.open_cents == 0
                    || out
                        .iter()
                        .any(|s| (s.booking.id, s.booking.own_part()) == id)
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
