//! Explicit refund links (#714): a refund (incoming booking) points at the booking it pays back
//! (outgoing). One original can have several refunds and one refund can pay back several originals
//! (schema step 37 holds all of it); links do not chain. A link to a split booking names one of
//! its parts (`original_part`): the refund pays back that part only. A link whose part is gone
//! (the split was removed or has no such part any more) becomes a link to the whole booking (`split_changed`);
//! such a link to a split booking fills its parts from the last to the first. A split refund
//! works the same way (`refund_part`): the link names the part of the income that is
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

/// Step 37: the links between refunds and the bookings they pay back. A refund may pay back
/// several bookings and a booking may have several refunds, so the key is the pair with the
/// parts it names: `original_part` / `refund_part` are the position of a split booking's part
/// (from 1), 0 is the whole booking. A removed link stays as a tombstone (`deleted_ms`).
pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS refund_links (
 refund_id INTEGER NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
 original_id INTEGER NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
 original_part INTEGER NOT NULL DEFAULT 0,
 refund_part INTEGER NOT NULL DEFAULT 0,
 created_ms INTEGER NOT NULL,
 changed_ms INTEGER NOT NULL,
 deleted_ms INTEGER,
 PRIMARY KEY (refund_id, original_id, original_part, refund_part)
);
CREATE INDEX IF NOT EXISTS refund_links_original ON refund_links(original_id);
";

/// Step 37: the table, and one order for the parts of the splits there are. A link names a
/// part by its position, but until this step the merge compared the parts of a split sorted
/// (category uuid, amount, note) and wrote them that way, while the device the split was made
/// on kept the order they were entered in: two devices could hold the same split in two
/// orders. Before the first link exists every split gets the sorted order, on every device
/// and in every file that is merged (it is migrated first). Later splits keep the order they
/// are entered in; the merge takes it over as it is.
pub(crate) fn schema_step(conn: &Connection) -> CoreResult<()> {
    conn.execute_batch(SCHEMA)?;
    conn.execute_batch(
        "CREATE TEMP TABLE split_order AS
           SELECT s.transaction_id, s.category_id, s.amount_cents, s.note
             FROM transaction_splits s LEFT JOIN categories c ON c.id = s.category_id
            ORDER BY s.transaction_id, c.uuid, s.amount_cents, s.note, s.id;
         DELETE FROM transaction_splits;
         INSERT INTO transaction_splits(transaction_id, category_id, amount_cents, note)
           SELECT transaction_id, category_id, amount_cents, note FROM split_order ORDER BY rowid;
         DROP TABLE split_order;",
    )?;
    Ok(())
}

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
    /// The part that is paid back (its position from 1, see `list_transaction_splits`), 0 =
    /// the whole booking: on a row that is paid back the part it stands for (`amount_cents`
    /// is then the part's amount, also for an outgoing part of an incoming booking), on a row
    /// that pays back the part of the booking it pays back. Pass it to `link_refund` /
    /// `unlink_refund`.
    pub part: u32,
    /// Category name and note of that part, for the row; empty without a part.
    pub part_label: String,
    /// The part that pays back (from 1), 0 = the whole booking: on a row that pays back the
    /// part it stands for (`amount_cents` is then the part's amount, also for an incoming part
    /// of an outgoing booking, like store credit), on a row that is paid back the part of the
    /// booking that pays. Pass it to `link_refund_parts` / `unlink_refund_parts`.
    pub refund_part: u32,
    /// Category name and note of that part of the refund; empty without one.
    pub refund_part_label: String,
}

impl RefundBooking {
    /// The part of its own booking the row stands for as the end `role` of a link.
    fn own_part(&self, role: Role) -> u32 {
        match role {
            Role::Original => self.part,
            Role::Refund => self.refund_part,
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
    /// What the booking can pay back at most: its amount when it is incoming, of a split
    /// booking the sum of its incoming parts (also those of an outgoing booking).
    pub refund_total_cents: i64,
    /// What of that is left for a further link.
    pub refund_open_cents: i64,
    /// What can be paid back of the booking at most: its amount when it is outgoing, of a
    /// split booking the sum of its outgoing parts (also those of an incoming booking).
    pub original_total_cents: i64,
    /// What of that is left for a further link.
    pub original_open_cents: i64,
    /// For an incoming booking that pays back: what of it stays income, i.e. all but what
    /// pays back outgoing bookings (`Booking::refund_income_cents`). Else 0.
    pub refund_income_cents: i64,
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

/// Which end of a link a booking or a part is. An amount is a refund when it is incoming and
/// an original when it is outgoing, whatever the side of the booking it is a part of: store
/// credit that paid a part of a purchase is a refund, a fee taken off an income can be paid back.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Refund,
    Original,
}

impl Role {
    fn sign(self) -> i64 {
        match self {
            Role::Refund => 1,
            Role::Original => -1,
        }
    }
    fn other(self) -> Role {
        match self {
            Role::Refund => Role::Original,
            Role::Original => Role::Refund,
        }
    }
    fn of(amount: i64) -> Option<Role> {
        match amount.signum() {
            1 => Some(Role::Refund),
            -1 => Some(Role::Original),
            _ => None,
        }
    }
}

/// The part a link names, when it can have `role` (see [`Role`]).
fn named(parts: &HashMap<i64, Vec<Part>>, booking: i64, part: i64, role: Role) -> Option<&Part> {
    part_of(parts, booking, part).filter(|p| p.amount.signum() == role.sign())
}

/// A link: refund, original, `original_part`, `refund_part`.
type Link = (i64, i64, i64, i64);

/// What the live links pay back.
#[derive(Default)]
struct Shares {
    links: HashMap<Link, i64>,
    /// The place of every live link in the order the shares are filled.
    place: HashMap<Link, usize>,
    /// Links between different currencies: they get nothing.
    mixed: std::collections::HashSet<Link>,
    /// Per part (booking, part from 1) of a split booking, and per booking without parts
    /// (booking, 0).
    taken: HashMap<(i64, i64), i64>,
    /// Per link to a split original: what it pays back of which part (from 1).
    into: HashMap<Link, Vec<(i64, i64)>>,
    /// Per link of a split refund: what it takes from which part (from 1).
    from: HashMap<Link, Vec<(i64, i64)>>,
    /// Per booking with a link that counts: its amount, and what its links cover of it as a
    /// refund and as an original (`None`: it has no link in that role).
    linked: HashMap<i64, (i64, Option<i64>, Option<i64>)>,
    /// Per booking that pays back: what of that goes to outgoing bookings (expenses), not to
    /// an outgoing part of an incoming booking.
    expense: HashMap<i64, i64>,
    /// The links that pay back an outgoing booking.
    to_expense: std::collections::HashSet<Link>,
}

impl Shares {
    fn left(&self, id: i64, part: i64, amount: i64) -> i64 {
        (amount.saturating_abs() - self.taken.get(&(id, part)).copied().unwrap_or(0)).max(0)
    }

    /// What the booking `id` of `amount` can give or take in `role` at most: its amount when
    /// it is on that side, of a split booking the sum of its parts on that side.
    fn total(parts: &HashMap<i64, Vec<Part>>, id: i64, amount: i64, role: Role) -> i64 {
        match parts.get(&id) {
            Some(own) => own
                .iter()
                .filter(|p| p.amount.signum() == role.sign())
                .map(|p| p.amount.saturating_abs())
                .sum(),
            None if amount.signum() == role.sign() => amount.saturating_abs(),
            None => 0,
        }
    }

    /// What of [`Shares::total`] is left for a further link.
    fn side_open(&self, parts: &HashMap<i64, Vec<Part>>, id: i64, amount: i64, role: Role) -> i64 {
        match parts.get(&id) {
            Some(own) => (1i64..)
                .zip(own)
                .filter(|(_, p)| p.amount.signum() == role.sign())
                .map(|(n, p)| self.left(id, n, p.amount))
                .sum(),
            None if amount.signum() == role.sign() => self.left(id, 0, amount),
            None => 0,
        }
    }

    /// Whether the end `part` of the booking `id` can have `role`: the part it names is on
    /// that side, or (no such part) the booking has an amount on that side at all.
    fn fits(parts: &HashMap<i64, Vec<Part>>, id: i64, amount: i64, part: i64, role: Role) -> bool {
        named(parts, id, part, role).is_some() || Self::total(parts, id, amount, role) > 0
    }

    /// What the end `part` of the booking `id` has left for a further link in `role`: the
    /// part it names, else the booking as a whole.
    fn open(
        &self,
        parts: &HashMap<i64, Vec<Part>>,
        id: i64,
        amount: i64,
        part: i64,
        role: Role,
    ) -> i64 {
        match named(parts, id, part, role) {
            Some(p) => self.left(id, part, p.amount),
            None => self.side_open(parts, id, amount, role),
        }
    }

    /// Books `share` on that end and says which parts of the booking carry it: the part the
    /// link names, else (a link to the whole of a split booking, or to a part it does not
    /// have) its parts on that side from the last to the first, each until the share runs
    /// out. Empty for a booking without parts.
    fn place(
        &mut self,
        parts: &HashMap<i64, Vec<Part>>,
        id: i64,
        part: i64,
        role: Role,
        share: i64,
    ) -> Vec<(i64, i64)> {
        if named(parts, id, part, role).is_some() {
            *self.taken.entry((id, part)).or_default() += share;
            return vec![(part, share)];
        }
        let Some(own) = parts.get(&id) else {
            *self.taken.entry((id, 0)).or_default() += share;
            return Vec::new();
        };
        let mut left = share;
        let mut placed = Vec::new();
        for (n, p) in own.iter().enumerate().rev() {
            if p.amount.signum() != role.sign() {
                continue;
            }
            let n = n as i64 + 1;
            let take = left.min(self.left(id, n, p.amount));
            if take > 0 {
                *self.taken.entry((id, n)).or_default() += take;
                left -= take;
                placed.push((n, take));
            }
        }
        placed
    }
}

thread_local! {
    // The accounts whose links count on this thread (`within`); `None` = all.
    static WITHIN: std::cell::RefCell<Option<Vec<i64>>> = const { std::cell::RefCell::new(None) };
}

/// While it lives, only links between bookings of the given accounts exist for every read
/// of this thread (shares, moved amounts, markers). For the local interface (#136): what it
/// returns for the shared accounts must not depend on a booking of another account, so a
/// refund whose original is not shared reads as not linked there.
pub(crate) struct Within(Option<Vec<i64>>);

pub(crate) fn within(accounts: &[i64]) -> Within {
    Within(WITHIN.with(|w| w.replace(Some(accounts.to_vec()))))
}

/// Every link again while it lives: for what is worked out once and kept for all readers
/// (the vault's list cache), also when a limited reader is the first to ask for it.
pub(crate) fn unlimited() -> Within {
    Within(WITHIN.with(|w| w.replace(None)))
}

impl Drop for Within {
    fn drop(&mut self) {
        WITHIN.with(|w| *w.borrow_mut() = self.0.take());
    }
}

/// What every live link pays back, filled in the order the links were made (links made in the
/// same millisecond on two devices: by what both devices know of the bookings and their
/// accounts (the key of `original_key`), not by their local ids): a link takes at
/// most what the earlier links left of the amount that pays back and of the amount it pays
/// back (the parts it names, see [`Role`]). A link to the whole of a split booking (it names
/// no part, or one the booking does not have) uses the parts on that booking's side from the
/// last to the first, each until the share runs out. A link whose ends are not an incoming
/// and an outgoing amount (any more) counts for nothing; links between different currencies
/// get no share.
fn shares(conn: &Connection, parts: &HashMap<i64, Vec<Part>>) -> CoreResult<Shares> {
    type Row = (i64, i64, i64, i64, i64, i64, String, String);
    // account ids are integers, so inlining them cannot inject anything
    let within = WITHIN.with(|w| {
        w.borrow().as_ref().map_or(String::new(), |accounts| {
            let ids: Vec<String> = accounts.iter().map(i64::to_string).collect();
            let ids = ids.join(",");
            format!(" AND r.account_id IN ({ids}) AND o.account_id IN ({ids})")
        })
    });
    let links: Vec<Row> = conn
        .prepare(&format!(
            "SELECT l.refund_id,l.original_id,l.original_part,l.refund_part,r.amount_cents,o.amount_cents,r.currency,o.currency
             FROM refund_links l JOIN transactions r ON r.id=l.refund_id JOIN transactions o ON o.id=l.original_id
             JOIN accounts ra ON ra.id=r.account_id LEFT JOIN bank_connections rc ON rc.id=ra.connection_id
             JOIN accounts oa ON oa.id=o.account_id LEFT JOIN bank_connections oc ON oc.id=oa.connection_id
             WHERE l.deleted_ms IS NULL AND r.deleted_ms IS NULL AND o.deleted_ms IS NULL{within}
             ORDER BY l.created_ms,l.changed_ms,r.booking_date,o.booking_date,r.external_id,o.external_id,l.original_part,l.refund_part,
                      COALESCE(rc.connector,''),COALESCE(rc.login_name,''),COALESCE(rc.bank_code,''),ra.external_ref,
                      COALESCE(oc.connector,''),COALESCE(oc.login_name,''),COALESCE(oc.bank_code,''),oa.external_ref,
                      l.refund_id,l.original_id"
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
        out.place.insert(link, out.place.len());
        if r == o
            || !Shares::fits(parts, r, r_amount, refund_part, Role::Refund)
            || !Shares::fits(parts, o, o_amount, part, Role::Original)
        {
            continue;
        }
        out.linked
            .entry(r)
            .or_insert((r_amount, None, None))
            .1
            .get_or_insert(0);
        out.linked
            .entry(o)
            .or_insert((o_amount, None, None))
            .2
            .get_or_insert(0);
        if currency_code(&r_cur) != currency_code(&o_cur) {
            out.mixed.insert(link);
            continue;
        }
        let share = out
            .open(parts, r, r_amount, refund_part, Role::Refund)
            .min(out.open(parts, o, o_amount, part, Role::Original));
        let from = out.place(parts, r, refund_part, Role::Refund, share);
        let into = out.place(parts, o, part, Role::Original, share);
        if let Some((_, Some(paid), _)) = out.linked.get_mut(&r) {
            *paid += share;
        }
        if o_amount < 0 {
            *out.expense.entry(r).or_default() += share;
            out.to_expense.insert(link);
        }
        if let Some((_, _, Some(back))) = out.linked.get_mut(&o) {
            *back += share;
        }
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

/// A link must join an incoming amount (the refund: an incoming booking, or an incoming part
/// of any split booking) to an outgoing one of another booking, and no amount is both.
fn validate(
    conn: &Connection,
    parts: &HashMap<i64, Vec<Part>>,
    refund: i64,
    original: i64,
    part: i64,
    refund_part: i64,
) -> CoreResult<()> {
    let r = live(conn, refund)?;
    let o = live(conn, original)?;
    if refund == original
        || !Shares::fits(parts, refund, r, refund_part, Role::Refund)
        || !Shares::fits(parts, original, o, part, Role::Original)
    {
        return Err(CoreError::invalid(tr(
            "Eine Erstattung muss positiv und die ursprüngliche Buchung negativ sein.",
        )));
    }
    // no chains: the amount that is paid back (`original`, `part`) must not pay back itself,
    // and the one that pays back (`refund`, `refund_part`) must not be paid back. Hence the
    // crossed parameters: ?1/?3 look for the original as a refund, ?2/?4 for the refund as an original.
    let conflict: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM refund_links WHERE deleted_ms IS NULL AND ((refund_id=?1 AND refund_part=?3) OR (original_id=?2 AND original_part=?4)))",
        params![original, refund, part, refund_part],
        |r| r.get(0),
    )?;
    if conflict {
        return Err(CoreError::invalid(tr(
            "Erstattungen dürfen nicht miteinander verkettet werden.",
        )));
    }
    Ok(())
}

/// [`validate`] for a link the user makes here: of a split booking a part is linked (an
/// outgoing one is paid back, an incoming one pays back; a split incoming booking may also pay
/// back as a whole, `refund_part` 0), and an amount that is used up takes no further link.
/// Links arriving from another device are not held to this; they get a share of 0.
fn validate_new(
    conn: &Connection,
    refund: i64,
    original: i64,
    part: i64,
    refund_part: i64,
) -> CoreResult<()> {
    let parts = split_parts(conn)?;
    validate(conn, &parts, refund, original, part, refund_part)?;
    let (r, o) = (live(conn, refund)?, live(conn, original)?);
    let fits = match parts.get(&original) {
        Some(_) => named(&parts, original, part, Role::Original).is_some(),
        None => part == 0,
    } && if refund_part == 0 {
        r > 0
    } else {
        named(&parts, refund, refund_part, Role::Refund).is_some()
    };
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
        // a link that could take nothing would only mislead: the amount is used up
        let shares = shares(conn, &parts)?;
        if shares.open(&parts, refund, r, refund_part, Role::Refund) == 0 {
            return Err(CoreError::invalid(tr(
                "Die Erstattung ist bereits vollständig zugeordnet.",
            )));
        }
        if shares.open(&parts, original, o, part, Role::Original) == 0 {
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

/// How [`rows`] lists the bookings a link can be made to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Expand {
    /// As read: the rows of existing links.
    No,
    /// Every amount that can have the role: a booking without parts as a whole, a split
    /// booking by its parts on that side, whatever the side of the booking itself.
    Parts,
    /// What `link_refund` can name (a front end without a choice of the part that pays
    /// back): what is paid back like `Parts`, what pays back only as a whole incoming booking.
    Whole,
}

/// The bookings of `sql` as the end `role` of a link, see [`Expand`].
fn rows(
    conn: &Connection,
    sql: &str,
    args: impl rusqlite::Params,
    rules: &crate::merchants::MerchantRules,
    role: Role,
    expand: Expand,
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
        let on_side = whole.signum() == role.sign();
        let of_side = |own: &Vec<Part>| -> Vec<u32> {
            (1u32..)
                .zip(own)
                .filter(|(_, p)| p.amount.signum() == role.sign())
                .map(|(n, _)| n)
                .collect()
        };
        let own: Vec<u32> = match (expand, parts.get(&id)) {
            (Expand::No, _) => vec![match role {
                Role::Original => row.booking.part,
                Role::Refund => row.booking.refund_part,
            }],
            (Expand::Parts, Some(own)) => of_side(own),
            (Expand::Whole, Some(own)) if role == Role::Original => of_side(own),
            _ if on_side => vec![0],
            _ => Vec::new(),
        };
        for part in own {
            let mut b = row.booking.clone();
            let (own, label) = match role {
                Role::Original => (&mut b.part, &mut b.part_label),
                Role::Refund => (&mut b.refund_part, &mut b.refund_part_label),
            };
            *own = part;
            // the row stands for the part of its booking the link names
            if let Some(p) = named(&parts, id, i64::from(part), role) {
                *label = p.label.clone();
                b.amount_cents = p.amount;
            }
            b.open_cents = shares.open(&parts, id, whole, i64::from(part), role);
            rows.push(Row {
                booking: b,
                end_to_end_id: row.end_to_end_id.clone(),
            });
        }
    }
    Ok(rows)
}

/// How much of every linked booking its links cover.
pub(crate) struct Covered {
    /// Per booking that pays back: what its originals take up, and what it can pay back at
    /// most (`Shares::total`).
    pub(crate) refund: HashMap<i64, (i64, i64)>,
    /// Per booking that pays back: what of that lowers expenses, because it pays back an
    /// outgoing booking. What pays back an outgoing part of an income (a fee) raises that
    /// income instead and stays income.
    pub(crate) expense: HashMap<i64, i64>,
    /// Per booking that is paid back: what its refunds pay back, and what can be paid back.
    pub(crate) original: HashMap<i64, (i64, i64)>,
    /// The bookings with a link between different currencies: nothing is added up for them
    /// (#569), so their sum is not shown.
    pub(crate) mixed: std::collections::HashSet<i64>,
}

pub(crate) fn covered(conn: &Connection) -> CoreResult<Covered> {
    let parts = split_parts(conn)?;
    let shares = shares(conn, &parts)?;
    let side = |role: Role| -> HashMap<i64, (i64, i64)> {
        shares
            .linked
            .iter()
            .filter_map(|(id, (amount, paid, back))| {
                let covered = match role {
                    Role::Refund => paid,
                    Role::Original => back,
                };
                Some((
                    *id,
                    ((*covered)?, Shares::total(&parts, *id, *amount, role)),
                ))
            })
            .collect()
    };
    Ok(Covered {
        refund: side(Role::Refund),
        original: side(Role::Original),
        expense: shares.expense.clone(),
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
    /// (refund, category it counts in, cents, whether it pays back an outgoing booking).
    into: Vec<(i64, Option<i64>, i64, bool)>,
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
        let expense = shares.to_expense.contains(link);
        match parts.get(&original) {
            Some(_) => {
                for (n, cents) in shares.into.get(link).into_iter().flatten() {
                    if let Some(p) = part_of(&parts, original, *n) {
                        out.into.push((refund, p.category, *cents, expense));
                    }
                }
            }
            None => {
                if let Some(category) = own.get(&original) {
                    out.into.push((refund, *category, *share, expense));
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
    /// (refund_id, category_id, amount_cents, expense): what counts where the paid-back
    /// booking is; `expense` 1 when that is an outgoing booking.
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
            &["refund_id", "category_id", "amount_cents", "expense"],
            moved
                .into
                .iter()
                .map(|(refund, category, cents, expense)| {
                    let category = category.map_or("NULL".to_string(), |c| c.to_string());
                    format!("({refund},{category},{cents},{})", i32::from(*expense))
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
    /// Per refund: what of that counts in the filtered categories, as a lower expense and (what
    /// pays back an outgoing part of an income) as income.
    into: HashMap<i64, (i64, i64)>,
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
        for (refund, category, cents, expense) in moved.into {
            *out.assigned.entry(refund).or_default() += cents;
            if categories.contains(&category) {
                let into = out.into.entry(refund).or_default();
                if expense {
                    into.0 += cents;
                } else {
                    into.1 += cents;
                }
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
        // a split booking of 0 in sum (a refund part and a fee part) still moves its parts;
        // only the share of an unsplit booking is worked out from its amount
        let Some((this, assigned)) = this
            .and_then(|m| Some((m, m.assigned.get(&id)?)))
            .filter(|(m, _)| amount_cents != 0 || m.out_of.contains_key(&id))
        else {
            return crate::models::split_flow_parts(
                flow,
                share_cents,
                amount_cents,
                refund_income_cents,
            );
        };
        let rest = match this.out_of.get(&id) {
            // a split booking: what its links leave of its parts in the filter (a part that
            // pays nothing back, like a fee taken off a refund, counts as it is)
            Some(taken) => share_cents - taken,
            // else the refund is in the filter as a whole or not at all
            None => {
                let open = (amount_cents - assigned).max(0);
                (i128::from(open) * i128::from(share_cents) / i128::from(amount_cents)) as i64
            }
        };
        let income = if refund_income_cents > 0 { rest } else { 0 };
        let (into, into_income) = this.into.get(&id).copied().unwrap_or((0, 0));
        // `rest`: what the refund's own category keeps in the filter. It is income when the
        // refund still has an unassigned remainder, else it stays in the booking's own flow.
        // `into`: what refunds pay back of bookings in the filter (an expense reduction there),
        // `into_income`: the part of that the core books as income.
        [
            (flow, rest - income + into),
            (crate::merchants::FlowKind::Income, income + into_income),
        ]
    }
}

/// The booking a link is looked for, and the amounts of the other role around it.
struct Pool {
    /// What the booking can pay back or what can be paid back of it, with the sign of its
    /// role (`Shares::total`).
    amount: i64,
    currency: String,
    date: String,
    counterparty: String,
    merchant: String,
    reference: String,
    /// What it has left for a further link.
    open: i64,
    /// Nearest booking date first.
    rows: Vec<Row>,
}

/// `role`: what the booking `id` is in the links looked for. `days`: how far back (it pays
/// back) or ahead (it is paid back) the pool reaches; `None` = every date, before and after. Empty when the
/// booking has no amount of that role.
fn pool(
    vault: &Vault,
    db: &Connection,
    id: i64,
    role: Role,
    days: Option<u32>,
    expand: Expand,
) -> CoreResult<Pool> {
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
    let parts = split_parts(db)?;
    let total = Shares::total(&parts, id, amount, role);
    let mut out = Pool {
        amount: total * role.sign(),
        currency,
        date,
        counterparty,
        merchant,
        reference: format!("{purpose} {e2e}"),
        open: shares(db, &parts)?.side_open(&parts, id, amount, role),
        rows: Vec::new(),
    };
    if total == 0 {
        return Ok(out);
    }
    // the amounts of the other role: bookings on that side and split bookings with a part on it
    let side = match role {
        Role::Refund => {
            "(t.amount_cents<0 OR EXISTS(SELECT 1 FROM transaction_splits s WHERE s.transaction_id=t.id AND s.amount_cents<0))"
        }
        Role::Original => {
            "(t.amount_cents>0 OR EXISTS(SELECT 1 FROM transaction_splits s WHERE s.transaction_id=t.id AND s.amount_cents>0))"
        }
    };
    // within the days: what a refund pays back lies before it, what pays a booking back after
    // it. Without a bound (a search) the date does not matter: a refund can be booked first.
    let window = match (days, role) {
        (None, _) => "?3 IS NULL",
        (Some(_), Role::Refund) => "julianday(?1)-julianday(t.booking_date) BETWEEN 0 AND ?3",
        (Some(_), Role::Original) => "julianday(t.booking_date)-julianday(?1) BETWEEN 0 AND ?3",
    };
    out.rows = rows(
        db,
        &format!(
            "{} WHERE t.deleted_ms IS NULL AND t.id<>?2 AND {side} AND {window} ORDER BY abs(julianday(t.booking_date)-julianday(?1)),t.id",
            plain_rows()
        ),
        params![out.date, id, days],
        &rules,
        role.other(),
        expand,
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

/// What every linked booking is in its links, for the marker in the list: a booking that pays
/// back is a refund (also an outgoing booking with a part that does), else one that is paid
/// back an original.
pub(crate) fn markers(conn: &Connection) -> CoreResult<HashMap<i64, RefundLinkKind>> {
    let covered = covered(conn)?;
    Ok(covered.markers())
}

impl Covered {
    pub(crate) fn markers(&self) -> HashMap<i64, RefundLinkKind> {
        let mut out: HashMap<i64, RefundLinkKind> = self
            .original
            .keys()
            .map(|id| (*id, RefundLinkKind::Original))
            .collect();
        out.extend(self.refund.keys().map(|id| (*id, RefundLinkKind::Refund)));
        out
    }
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
        // a link the bookings here do not fit (their sides or parts differ for now) is kept:
        // it counts for nothing (`shares`) until they do, and the devices stay alike
        if r == o {
            continue;
        }
        conn.execute(
            "INSERT INTO refund_links(refund_id,original_id,original_part,refund_part,created_ms,changed_ms,deleted_ms) VALUES (?1,?2,?6,?7,?3,?4,?5)
             ON CONFLICT(refund_id,original_id,original_part,refund_part) DO UPDATE SET created_ms=excluded.created_ms,changed_ms=excluded.changed_ms,deleted_ms=excluded.deleted_ms",
            params![r,o,created,changed,deleted,part,refund_part],
        )?;
    }
    Ok(())
}

/// The booking `id` is split differently or not at all any more (call it with the new parts
/// stored). A link stays with the part it names while the booking still has a part of that
/// direction at that position, whatever its new amount: the link the user made for one part
/// never spreads to the others. Only a link whose part is gone (the split was removed, has
/// fewer parts, or the part changed its direction) becomes a link to the whole booking. The old
/// row stays as a tombstone and the new one gets a new stamp (both reach the other devices). It
/// keeps `created_ms`, the stamp of when the link was made, which is what the shares are filled
/// by: a link keeps its place among a refund's other links.
pub(crate) fn split_changed(conn: &Connection, id: i64) -> CoreResult<()> {
    let links: Vec<(i64, i64, i64, i64, i64)> = conn
        .prepare(
            "SELECT refund_id,original_id,original_part,refund_part,created_ms FROM refund_links
             WHERE deleted_ms IS NULL AND ((original_id=?1 AND original_part>0) OR (refund_id=?1 AND refund_part>0))
             ORDER BY created_ms,changed_ms,refund_id,original_id,original_part,refund_part",
        )?
        .query_map([id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?
        .collect::<Result<_, _>>()?;
    let parts = split_parts(conn)?;
    for (refund, original, part, refund_part, created) in links {
        let (own, role) = if original == id {
            (part, Role::Original)
        } else {
            (refund_part, Role::Refund)
        };
        if named(&parts, id, own, role).is_some() {
            continue;
        }
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
        // a booking linked by several of its parts keeps one link, at the place of the
        // earliest of them (the link to the whole booking may come from a later part)
        conn.execute(
            "INSERT INTO refund_links(refund_id,original_id,original_part,refund_part,created_ms,changed_ms,deleted_ms) VALUES (?1,?2,?3,?4,?5,?6,NULL)
             ON CONFLICT(refund_id,original_id,original_part,refund_part) DO UPDATE SET
                created_ms=CASE WHEN deleted_ms IS NULL THEN MIN(created_ms,excluded.created_ms) ELSE excluded.created_ms END,
                changed_ms=excluded.changed_ms,deleted_ms=NULL
             WHERE deleted_ms IS NOT NULL OR excluded.created_ms<created_ms",
            params![refund, original, part, refund_part, created, changed + 1],
        )?;
    }
    // without the part that paid back or was paid back the booking may have no amount on that
    // side any more (store credit in a purchase that is not split any more): that link goes
    let amount = live(conn, id)?;
    for role in [Role::Refund, Role::Original] {
        if Shares::total(&parts, id, amount, role) == 0 {
            let changed = next_change(conn)?;
            conn.execute(
                match role {
                    Role::Refund => "UPDATE refund_links SET deleted_ms=?2,changed_ms=?2 WHERE deleted_ms IS NULL AND refund_id=?1",
                    Role::Original => "UPDATE refund_links SET deleted_ms=?2,changed_ms=?2 WHERE deleted_ms IS NULL AND original_id=?1",
                },
                params![id, changed],
            )?;
        }
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
    /// in `list_transaction_splits`; an outgoing part, also of an incoming booking), else 0.
    /// Fails for wrong signs, deleted bookings, chains, a split booking without a part and
    /// amounts that are used up. A split outgoing booking cannot be linked as a whole: it
    /// needs `original_part`. A split incoming `refund_id` pays back as a whole here, filled
    /// from its last part to its first; `link_refund_parts` names the one part that pays back.
    pub fn link_refund(
        &self,
        refund_id: i64,
        original_id: i64,
        original_part: u32,
    ) -> CoreResult<()> {
        self.link_refund_parts(refund_id, 0, original_id, original_part)
    }
    /// `link_refund` with the part that pays back: `refund_part` is an incoming part of the
    /// split booking `refund_id` (its position from 1 in `list_transaction_splits`), at most
    /// with its amount; 0 is the whole booking, which must be incoming then. The part may
    /// belong to an outgoing booking (store credit that paid a part of a purchase).
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
        db.execute("INSERT INTO refund_links(refund_id,original_id,original_part,refund_part,created_ms,changed_ms,deleted_ms) VALUES (?1,?2,?4,?5,?3,?3,NULL) ON CONFLICT(refund_id,original_id,original_part,refund_part) DO UPDATE SET created_ms=excluded.created_ms,changed_ms=excluded.changed_ms,deleted_ms=NULL WHERE deleted_ms IS NOT NULL",params![refund_id,original_id,changed,original_part,refund_part])?;
        Ok(())
    }
    /// Removes the links of `refund_id` to `original_id` (its part `original_part`), whatever
    /// part of the refund they name: all of them when several parts of the refund pay back
    /// that amount (`unlink_refund_parts` removes one). No-op without one.
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
    /// The live links of one booking; deleted bookings are left out. `originals`: what it pays
    /// back, `refunds`: what pays it back. A split booking with parts on both sides can have
    /// both.
    pub fn refund_links(&self, transaction_id: i64) -> CoreResult<RefundLinks> {
        let db = self.db()?;
        let own_amount = live(&db, transaction_id)?;
        let rules = self.merchant_rules(&db)?;
        let side = |own: &str, other: &str, role: Role| {
            rows(
                &db,
                &format!(
                    "{} JOIN refund_links l ON l.{other}=t.id AND l.{own}=?1 AND l.deleted_ms IS NULL WHERE t.deleted_ms IS NULL ORDER BY l.created_ms,l.changed_ms,t.id,l.original_part,l.refund_part",
                    ROWS.replace("{part}", "l.original_part")
                        .replace("{refund_part}", "l.refund_part")
                ),
                [transaction_id],
                &rules,
                role,
                Expand::No,
            )
            .map(|rows| rows.into_iter().map(|r| r.booking).collect::<Vec<_>>())
        };
        let mut originals = side("refund_id", "original_id", Role::Original)?;
        let mut refunds = side("original_id", "refund_id", Role::Refund)?;
        let parts = split_parts(&db)?;
        let shares = shares(&db, &parts)?;
        let link = |refund: i64, original: i64, row: &RefundBooking| -> Link {
            (
                refund,
                original,
                i64::from(row.part),
                i64::from(row.refund_part),
            )
        };
        // the part of this booking a link names, for the row of the booking on its other end
        let own_label = |part: u32, role: Role| {
            named(&parts, transaction_id, i64::from(part), role)
                .map(|p| p.label.clone())
                .unwrap_or_default()
        };
        // listed in the order they are filled, also with equal stamps (`shares`)
        let place = |l: Link| shares.place.get(&l).copied().unwrap_or(usize::MAX);
        originals.sort_by_key(|o| place(link(transaction_id, o.id, o)));
        refunds.sort_by_key(|r| place(link(r.id, transaction_id, r)));
        for o in &mut originals {
            o.share_cents = shares.links.get(&link(transaction_id, o.id, o)).copied();
            o.refund_part_label = own_label(o.refund_part, Role::Refund);
        }
        for r in &mut refunds {
            r.share_cents = shares.links.get(&link(r.id, transaction_id, r)).copied();
            r.part_label = own_label(r.part, Role::Original);
        }
        let sum = |links: Vec<Link>| -> Option<i64> {
            (!links.is_empty() && !links.iter().any(|l| shares.mixed.contains(l)))
                .then(|| links.iter().filter_map(|l| shares.links.get(l)).sum())
        };
        let total = |role: Role| Shares::total(&parts, transaction_id, own_amount, role);
        let open = |role: Role| shares.side_open(&parts, transaction_id, own_amount, role);
        Ok(RefundLinks {
            refunded_cents: sum(refunds
                .iter()
                .map(|r| link(r.id, transaction_id, r))
                .collect()),
            assigned_cents: sum(originals
                .iter()
                .map(|o| link(transaction_id, o.id, o))
                .collect()),
            refund_total_cents: total(Role::Refund),
            refund_open_cents: open(Role::Refund),
            original_total_cents: total(Role::Original),
            original_open_cents: open(Role::Original),
            refund_income_cents: if own_amount > 0 && !originals.is_empty() {
                (own_amount - shares.expense.get(&transaction_id).copied().unwrap_or(0)).max(0)
            } else {
                0
            },
            originals,
            refunds,
        })
    }
    /// The bookings `transaction_id` could be linked to, nearest date first: for a refund the
    /// outgoing bookings before it, for an outgoing booking the incoming ones after it; all
    /// accounts and currencies. Without `query` the last / next 365
    /// days, with one (name or purpose) every such booking of any date, before or after.
    /// Resolves the merchant of every
    /// booking it reads, so call it off the main thread. For a front end without a choice of
    /// the part that pays back: every row can be linked with `link_refund`.
    pub fn refund_candidates(
        &self,
        transaction_id: i64,
        query: String,
        limit: u32,
    ) -> CoreResult<Vec<RefundBooking>> {
        let db = self.db()?;
        let Some(role) = Role::of(live(&db, transaction_id)?) else {
            return Ok(Vec::new());
        };
        self.candidates(&db, transaction_id, role, query, limit, Expand::Whole)
    }

    /// [`refund_candidates`](Self::refund_candidates) with every amount that can be linked,
    /// for `link_refund_parts`. `pays_back`: the booking is looked at as the one that pays
    /// back (its incoming amounts), else as the one that is paid back (its outgoing amounts);
    /// a split booking with parts on both sides can be either. The rows are the amounts of
    /// the other role: bookings on that side, and of every split booking its parts on that
    /// side, also the incoming parts of an outgoing booking (store credit that paid a part of
    /// a purchase) and the outgoing parts of an incoming booking (a fee taken off an income).
    pub fn refund_part_candidates(
        &self,
        transaction_id: i64,
        pays_back: bool,
        query: String,
        limit: u32,
    ) -> CoreResult<Vec<RefundBooking>> {
        let db = self.db()?;
        live(&db, transaction_id)?;
        let role = if pays_back {
            Role::Refund
        } else {
            Role::Original
        };
        self.candidates(&db, transaction_id, role, query, limit, Expand::Parts)
    }

    /// Up to three bookings of [`refund_candidates`](Self::refund_candidates) (365 days) that most
    /// likely belong to `transaction_id`, filled in this order, the nearest date first within
    /// each: the same amount in the other direction, the same counterparty or merchant within
    /// two months, a shared reference in the texts. Bookings already linked to it are left out.
    pub fn refund_suggestions(&self, transaction_id: i64) -> CoreResult<Vec<RefundSuggestion>> {
        let db = self.db()?;
        let Some(role) = Role::of(live(&db, transaction_id)?) else {
            return Ok(Vec::new());
        };
        self.suggestions(&db, transaction_id, role, Expand::Whole, 0)
    }

    /// [`refund_suggestions`](Self::refund_suggestions) among the rows of
    /// [`refund_part_candidates`](Self::refund_part_candidates).
    /// `own_part`: the part of the split booking the link would be made for (its position
    /// from 1), 0 for any of its amounts. With a part, „same amount“ means that part's amount
    /// and a part that is used up gets no suggestions.
    pub fn refund_part_suggestions(
        &self,
        transaction_id: i64,
        pays_back: bool,
        own_part: u32,
    ) -> CoreResult<Vec<RefundSuggestion>> {
        let db = self.db()?;
        live(&db, transaction_id)?;
        let role = if pays_back {
            Role::Refund
        } else {
            Role::Original
        };
        self.suggestions(&db, transaction_id, role, Expand::Parts, own_part)
    }

    /// What the booking has left for a further link as a refund (`pays_back`) or as a booking
    /// that is paid back: of its part `own_part` (position from 1) when it names one on that
    /// side, else of the booking as a whole (`RefundLinks::refund_open_cents` /
    /// `original_open_cents`).
    pub fn refund_part_open(
        &self,
        transaction_id: i64,
        pays_back: bool,
        own_part: u32,
    ) -> CoreResult<i64> {
        let db = self.db()?;
        let amount = live(&db, transaction_id)?;
        let role = if pays_back {
            Role::Refund
        } else {
            Role::Original
        };
        let parts = split_parts(&db)?;
        Ok(shares(&db, &parts)?.open(&parts, transaction_id, amount, i64::from(own_part), role))
    }
}

impl Vault {
    fn candidates(
        &self,
        db: &Connection,
        transaction_id: i64,
        role: Role,
        query: String,
        limit: u32,
        expand: Expand,
    ) -> CoreResult<Vec<RefundBooking>> {
        let query = query.trim().to_lowercase();
        if limit == 0 {
            return Ok(Vec::new());
        }
        let days = query.is_empty().then_some(WINDOW_DAYS);
        let pool = pool(self, db, transaction_id, role, days, expand)?;
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

    fn suggestions(
        &self,
        db: &Connection,
        transaction_id: i64,
        role: Role,
        expand: Expand,
        own_part: u32,
    ) -> CoreResult<Vec<RefundSuggestion>> {
        const MAX: usize = 3;
        let pool = pool(self, db, transaction_id, role, Some(WINDOW_DAYS), expand)?;
        // a booking that is used up itself takes no further link: nothing to suggest
        if pool.open == 0 {
            return Ok(Vec::new());
        }
        let linked: std::collections::HashSet<(i64, u32)> = db
            .prepare(match role {
                Role::Refund => "SELECT original_id,original_part FROM refund_links WHERE refund_id=?1 AND deleted_ms IS NULL",
                Role::Original => "SELECT refund_id,refund_part FROM refund_links WHERE original_id=?1 AND deleted_ms IS NULL",
            })?
            .query_map([transaction_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        // the amounts of the booking that can be linked: all of it, or one of its parts
        let parts = split_parts(db)?;
        let own_amounts = match named(&parts, transaction_id, i64::from(own_part), role) {
            // the sheet links one part: its amount, and nothing when it is used up
            Some(p) if own_part > 0 => {
                let amount = live(db, transaction_id)?;
                if shares(db, &parts)?.open(
                    &parts,
                    transaction_id,
                    amount,
                    i64::from(own_part),
                    role,
                ) == 0
                {
                    return Ok(Vec::new());
                }
                vec![p.amount]
            }
            _ => {
                let mut all = vec![pool.amount];
                all.extend(
                    parts
                        .get(&transaction_id)
                        .into_iter()
                        .flatten()
                        .filter(|p| p.amount.signum() == role.sign())
                        .map(|p| p.amount),
                );
                all
            }
        };
        let source_refs = reference_tokens(&pool.reference);
        let counterparty = pool.counterparty.trim().to_lowercase();
        let matches = |row: &Row, reason: RefundMatch| -> bool {
            let b = &row.booking;
            match reason {
                RefundMatch::SameAmount => {
                    own_amounts.contains(&b.amount_cents.saturating_neg())
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
        let other = role.other();
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
                let id = (row.booking.id, row.booking.own_part(other));
                // a booking that is used up cannot be linked: no suggestion
                if linked.contains(&id)
                    || row.booking.open_cents == 0
                    || out
                        .iter()
                        .any(|s| (s.booking.id, s.booking.own_part(other)) == id)
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
