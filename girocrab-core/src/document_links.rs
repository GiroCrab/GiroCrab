//! Linked receipts (GitHub #590): a receipt that stays where it is (the
//! user's own document archive) and is only referenced from a booking by its
//! URL, title and date, instead of being copied into the vault as a BLOB.
//!
//! GiroCrab never fetches the URL: no network access, no token, no preview.
//! The app opens it in the default browser. Only `http(s)` links are stored.
//!
//! A row is one reference on (at most) one booking. The same URL may hang on
//! several bookings (an invoice paid in instalments): there is no unique
//! index on `url`. Like a stored file, a reference remembers the account of
//! its booking, so it stays with that account once it is unlinked. Towards
//! the front ends a reference is a [`DocumentInfo`] like a stored file, with
//! `link_url` set; its `id` is the negated row id ([`doc_id_of`]), so ids of
//! stored files (`booking_documents.id`, positive) and of references never
//! collide and `link_document` / `delete_document` take either.
//!
//! Sync: unlike the BLOBs in `booking_documents`, references travel in the
//! vault merge. A reference is identified by its random `uuid`; the newest
//! edit wins for booking, account, URL, title and date, a deletion always
//! stays. Every edit writes its time (`updated_ms`) and a random `rev`; two
//! edits in the same millisecond are decided by `rev`, which is the same on
//! every device (booking and account ids, and even their sync keys, are
//! not). A deleted reference keeps only its key and stamps: URL, title,
//! date, booking and account are cleared, again after every merge.
//!
//! Bookings have no id of their own across devices, and a booking can change
//! its identity: a MoneyMoney twin is folded into its bank account, a
//! re-import repair lets one row take over another's key. A reference that is
//! moved along with such a change is stamped like an edit (`twins::fold_inner`,
//! [`Row::normalize`]), so the other devices take over the new booking
//! through the exact booking map of their merge instead of working it out
//! again. Where both sides hold the same edit and only one of them still
//! points at a live booking, that one wins.
//!
//! A reference whose booking does not survive a merge at all (deleted on the
//! other device) stays in the vault without a booking, like a stored file in
//! that case: nothing is lost, it shows up under "ohne zugeordneten Umsatz"
//! and can be removed or linked again.

use std::collections::{BTreeMap, HashMap};

use rusqlite::{Connection, OptionalExtension, params};
use url::Url;

use crate::dates::now_ms;
use crate::documents::{DocumentFilter, DocumentInfo, booking_label_of};
use crate::error::{CoreError, CoreResult};
use crate::i18n::{tr, trf};
use crate::vault::Vault;

#[cfg(test)]
mod tests;

/// Step 32 (#590): references to receipts kept outside the vault.
pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS document_links (
    id          INTEGER PRIMARY KEY,
    uuid        TEXT    NOT NULL UNIQUE,            -- sync key
    booking_id  INTEGER REFERENCES transactions(id) ON DELETE SET NULL,
    account_id  INTEGER REFERENCES accounts(id) ON DELETE SET NULL,   -- kept when unlinked
    url         TEXT    NOT NULL DEFAULT '',        -- http(s); '' once deleted
    title       TEXT    NOT NULL DEFAULT '',
    doc_date    TEXT    NOT NULL DEFAULT '',
    created_at  INTEGER NOT NULL DEFAULT (strftime('%s','now')),
    updated_ms  INTEGER NOT NULL DEFAULT 0,         -- newest edit wins in a vault merge
    rev         TEXT    NOT NULL DEFAULT '',        -- random per edit: decides equal updated_ms
    deleted_ms  INTEGER                             -- tombstone, kept for sync
);
-- answers the document count of the booking list (live references only)
CREATE INDEX IF NOT EXISTS document_links_booking ON document_links(booking_id) WHERE deleted_ms IS NULL;
";

/// The document count of a booking adds this to its stored files. The
/// condition must stay literally `deleted_ms IS NULL`: the partial index
/// `document_links_booking` answers it (a test checks the plan).
pub(crate) const COUNT_FOR_BOOKING_SQL: &str =
    "(SELECT count(*) FROM document_links l WHERE l.booking_id = t.id AND l.deleted_ms IS NULL)";

/// Longest URL (bytes, after normalisation).
const MAX_URL: usize = 2048;
/// Longest title (characters); longer ones are cut.
const MAX_TITLE: usize = 200;
/// Longest title derived from a URL (characters).
const MAX_DERIVED_TITLE: usize = 80;

/// `DocumentInfo.id` of the reference in row `row_id`.
pub(crate) fn doc_id_of(row_id: i64) -> i64 {
    -row_id
}

/// The row behind a `DocumentInfo.id`, if it names a reference (negative;
/// `i64::MIN` has no row).
pub(crate) fn row_id_of(doc_id: i64) -> Option<i64> {
    if doc_id < 0 {
        doc_id.checked_neg()
    } else {
        None
    }
}

/// The mark of one edit (see the module docs): random, so it compares the
/// same way on every device.
pub(crate) fn new_rev() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn invalid(msg: &str) -> CoreError {
    CoreError::InvalidInput {
        message: msg.to_string(),
    }
}

/// Checks a link and returns it parsed. Only `http(s)` with a host and
/// without embedded credentials; the message never repeats the input (it may
/// end up in a log or a bug report).
fn parse_link(input: &str) -> CoreResult<Url> {
    let s = input.trim();
    if s.is_empty() {
        return Err(invalid(tr("Der Link fehlt.")));
    }
    if s.len() > MAX_URL {
        return Err(invalid(tr("Der Link ist länger als 2048 Zeichen.")));
    }
    let url = Url::parse(s).map_err(|_| invalid(tr("Der Link ist ungültig.")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(invalid(tr(
            "Nur Links, die mit http:// oder https:// beginnen, lassen sich verknüpfen.",
        )));
    }
    if url.host_str().unwrap_or_default().is_empty() {
        return Err(invalid(tr("Der Link ist ungültig.")));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid(tr(
            "Links mit eingebetteten Zugangsdaten werden nicht gespeichert.",
        )));
    }
    if url.as_str().len() > MAX_URL {
        return Err(invalid(tr("Der Link ist länger als 2048 Zeichen.")));
    }
    Ok(url)
}

fn truncate_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => s[..i].to_string(),
        None => s.to_string(),
    }
}

/// Trimmed title, inner runs of whitespace collapsed, cut to [`MAX_TITLE`].
fn clean_title(title: &str) -> String {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(&title, MAX_TITLE)
}

/// A title for a link without one: host and path, without scheme, `www.`,
/// query and fragment (`archiv.example/documents/123`).
fn title_from_url(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default();
    let host = host.strip_prefix("www.").unwrap_or(host);
    let path = url.path().trim_matches('/');
    let title = if path.is_empty() {
        host.to_string()
    } else {
        format!("{host}/{path}")
    };
    if title.chars().count() > MAX_DERIVED_TITLE {
        format!("{}…", truncate_chars(&title, MAX_DERIVED_TITLE - 1))
    } else {
        title
    }
}

/// Columns read by [`row_to_info`]. A link to a deleted booking or account
/// reads as none, like for stored files: `t` and `la` only join live rows.
const SELECT_INFO: &str = "SELECT l.id, t.id, COALESCE(la.id, t.account_id), l.title, l.url,
            COALESCE(NULLIF(l.doc_date,''), t.booking_date, ''), l.created_at, a.name,
            t.counterparty, t.amount_cents, t.booking_date, t.purpose
       FROM document_links l
       LEFT JOIN transactions t ON t.id = l.booking_id AND t.deleted_ms IS NULL
       LEFT JOIN accounts la ON la.id = l.account_id AND la.deleted_ms IS NULL
       LEFT JOIN accounts a ON a.id = COALESCE(la.id, t.account_id)";

fn row_to_info(
    r: &rusqlite::Row,
    merchants: &crate::merchants::MerchantRules,
) -> rusqlite::Result<DocumentInfo> {
    Ok(DocumentInfo {
        id: doc_id_of(r.get(0)?),
        booking_id: r.get(1)?,
        account_id: r.get(2)?,
        title: r.get(3)?,
        doc_type: "beleg".into(),
        mime_type: String::new(),
        file_name: String::new(),
        size_bytes: 0,
        source_url: String::new(),
        link_url: r.get(4)?,
        doc_date: r.get(5)?,
        created_at: r.get(6)?,
        account_name: r.get::<_, Option<String>>(7)?.unwrap_or_default(),
        booking_label: booking_label_of(merchants, r.get(8)?, r.get(11)?, r.get(9)?, r.get(10)?),
        // the user put it there: nothing new to read
        seen: true,
    })
}

/// The live references of a booking.
pub(crate) fn list_for_booking(
    conn: &Connection,
    merchants: &crate::merchants::MerchantRules,
    booking_id: i64,
) -> CoreResult<Vec<DocumentInfo>> {
    let mut st = conn.prepare(&format!(
        "{SELECT_INFO} WHERE l.booking_id = ?1 AND l.deleted_ms IS NULL AND t.id IS NOT NULL"
    ))?;
    let rows = st
        .query_map([booking_id], |r| row_to_info(r, merchants))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// The live references matching `filter`, in no particular order. `search`
/// is the prepared `LIKE` pattern; it looks at title, URL and counterparty.
pub(crate) fn list_all(
    conn: &Connection,
    merchants: &crate::merchants::MerchantRules,
    filter: &DocumentFilter,
    search: Option<&str>,
) -> CoreResult<Vec<DocumentInfo>> {
    // a reference is always a receipt
    if filter.doc_type.as_deref().is_some_and(|t| t != "beleg") {
        return Ok(Vec::new());
    }
    let mut st = conn.prepare(&format!(
        "{SELECT_INFO} WHERE l.deleted_ms IS NULL
            AND (?1 IS NULL OR COALESCE(la.id, t.account_id) = ?1)
            AND (?2 IS NULL OR l.title LIKE ?2 OR l.url LIKE ?2 OR t.counterparty LIKE ?2)
            AND (?3 = 0 OR t.id IS NULL)
            AND (?4 IS NULL OR a.connection_id = ?4)"
    ))?;
    let rows = st
        .query_map(
            params![
                filter.account_id,
                search,
                filter.unlinked_only as i64,
                filter.connection_id
            ],
            |r| row_to_info(r, merchants),
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn not_found(doc_id: i64) -> CoreError {
    CoreError::NotFound {
        what: trf("Beleg {0}", &[&doc_id]),
    }
}

/// Account of the live booking `booking_id`.
fn live_booking_account(conn: &Connection, booking_id: i64) -> CoreResult<i64> {
    conn.query_row(
        "SELECT account_id FROM transactions WHERE id = ?1 AND deleted_ms IS NULL",
        [booking_id],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| CoreError::NotFound {
        what: trf("Buchung {0}", &[&booking_id]),
    })
}

/// Whether booking `booking_id` already carries a live reference to `url`,
/// other than the one in row `except`.
fn already_linked(
    conn: &Connection,
    booking_id: i64,
    url: &str,
    except: Option<i64>,
) -> CoreResult<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM document_links
          WHERE booking_id = ?1 AND url = ?2 AND deleted_ms IS NULL AND id IS NOT ?3)",
        params![booking_id, url, except],
        |r| r.get(0),
    )?)
}

/// Moves the reference in row `row_id` to another (live) booking, or off its
/// booking; unlinked it stays with the account it had. Like when attaching,
/// a booking takes one link only once.
pub(crate) fn relink(conn: &Connection, row_id: i64, booking_id: Option<i64>) -> CoreResult<()> {
    let account = match booking_id {
        Some(b) => Some(live_booking_account(conn, b)?),
        None => None,
    };
    if let Some(b) = booking_id {
        let url: Option<String> = conn
            .query_row(
                "SELECT url FROM document_links WHERE id = ?1 AND deleted_ms IS NULL",
                [row_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(url) = url
            && already_linked(conn, b, &url, Some(row_id))?
        {
            return Err(invalid(tr("Dieser Link hängt schon an der Buchung.")));
        }
    }
    let n = conn.execute(
        "UPDATE document_links
            SET booking_id = ?2, account_id = COALESCE(?3, account_id),
                updated_ms = max(updated_ms + 1, ?4), rev = ?5
          WHERE id = ?1 AND deleted_ms IS NULL",
        params![row_id, booking_id, account, now_ms(), new_rev()],
    )?;
    if n == 0 {
        return Err(not_found(doc_id_of(row_id)));
    }
    Ok(())
}

/// Moves the reference in row `row_id` to the (live) account `account_id`, or
/// to none, like a stored file (`Vault::set_document_account`): a booking of
/// another account it hangs on is unlinked, one of the new account stays.
pub(crate) fn set_account(
    conn: &Connection,
    row_id: i64,
    account_id: Option<i64>,
) -> CoreResult<()> {
    let n = conn.execute(
        "UPDATE document_links
            SET account_id = ?2,
                booking_id = CASE WHEN ?2 IS NOT NULL AND
                    (SELECT account_id FROM transactions WHERE id = booking_id) = ?2
                    THEN booking_id ELSE NULL END,
                updated_ms = max(updated_ms + 1, ?3), rev = ?4
          WHERE id = ?1 AND deleted_ms IS NULL",
        params![row_id, account_id, now_ms(), new_rev()],
    )?;
    if n == 0 {
        return Err(not_found(doc_id_of(row_id)));
    }
    Ok(())
}

/// Deletes the reference in row `row_id`: the row stays as a tombstone for
/// the vault merge, without anything that says what it pointed at.
pub(crate) fn delete(conn: &Connection, row_id: i64) -> CoreResult<()> {
    let now = now_ms();
    conn.execute(
        "UPDATE document_links
            SET deleted_ms = ?2, updated_ms = max(updated_ms + 1, ?2), rev = ?3,
                booking_id = NULL, account_id = NULL, url = '', title = '', doc_date = ''
          WHERE id = ?1 AND deleted_ms IS NULL",
        params![row_id, now, new_rev()],
    )?;
    Ok(())
}

/// A stored file was asked for, but `doc_id` names a reference.
pub(crate) fn no_file() -> CoreError {
    invalid(tr("Ein verlinkter Beleg hat keine Datei."))
}

#[uniffi::export]
impl Vault {
    /// Links a receipt that stays outside the vault to a booking (#590):
    /// only its `http(s)` URL, a title and the booking's date are stored.
    /// An empty `title` is derived from the URL. Returns the `DocumentInfo.id`
    /// of the reference. GiroCrab never fetches the URL.
    pub fn attach_document_link(
        &self,
        booking_id: i64,
        url: String,
        title: String,
    ) -> Result<i64, CoreError> {
        let parsed = parse_link(&url)?;
        let mut title = clean_title(&title);
        if title.is_empty() {
            title = title_from_url(&parsed);
        }
        let db = self.db()?;
        let (account, date): (i64, String) = db
            .query_row(
                "SELECT account_id, booking_date FROM transactions WHERE id = ?1 AND deleted_ms IS NULL",
                [booking_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| CoreError::NotFound {
                what: trf("Buchung {0}", &[&booking_id]),
            })?;
        if already_linked(&db, booking_id, parsed.as_str(), None)? {
            return Err(invalid(tr("Dieser Link hängt schon an der Buchung.")));
        }
        db.execute(
            "INSERT INTO document_links(uuid, booking_id, account_id, url, title, doc_date,
                    updated_ms, rev)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                uuid::Uuid::new_v4().to_string(),
                booking_id,
                account,
                parsed.as_str(),
                title,
                date,
                now_ms(),
                new_rev()
            ],
        )?;
        Ok(doc_id_of(db.last_insert_rowid()))
    }
}

// ------------------------------------------------------------------ merge

/// One reference in a vault merge. `booking` and `account` are local ids.
#[derive(Clone, PartialEq)]
struct Row {
    booking: Option<i64>,
    account: Option<i64>,
    url: String,
    title: String,
    doc_date: String,
    created_at: i64,
    updated_ms: i64,
    rev: String,
    deleted_ms: Option<i64>,
}

impl Row {
    /// What a reference looks like at rest: a deleted one carries no content
    /// (whichever side deleted it, whatever a later edit elsewhere brought
    /// back), and one on a booking or account of a folded MoneyMoney twin
    /// hangs on the bank account's copy. That move is stamped like an edit:
    /// which copy it is was worked out here, from this device's bookings, and
    /// the other devices take it over instead of working it out again.
    fn normalize(
        &mut self,
        conn: &Connection,
        folded: &mut HashMap<i64, Option<i64>>,
    ) -> CoreResult<()> {
        if self.deleted_ms.is_some() {
            self.booking = None;
            self.account = None;
            self.url.clear();
            self.title.clear();
            self.doc_date.clear();
            return Ok(());
        }
        let mut moved = false;
        if let Some(b) = self.booking
            && let Some(copy) = crate::sync_merge::twins::folded_booking(conn, b, folded)?
        {
            moved |= copy != b;
            self.booking = Some(copy);
        }
        if let Some(a) = self.account
            && let Some(bank) = crate::sync_merge::twins::folded_account(conn, a, folded)?
        {
            moved |= bank != a;
            self.account = Some(bank);
        }
        if moved {
            self.updated_ms = (self.updated_ms + 1).max(now_ms());
            self.rev = new_rev();
        }
        Ok(())
    }
}

/// The same address linked to the same booking on two devices before they
/// met is one reference: the one with the smallest uuid stays, the others
/// are deleted. The deletion is stamped from the references themselves, so
/// every device that sees both writes the very same one. References that
/// hang on no booking are never folded: the same address may be several
/// receipts there, each with its own title and date.
fn fold_doubles(state: &mut BTreeMap<String, Row>) {
    // (booking, address) -> kept uuid
    let mut kept: HashMap<(i64, String), String> = HashMap::new();
    let mut doubles: Vec<(String, String)> = Vec::new();
    // in uuid order: the first of a kind is the smallest
    for (uuid, r) in state.iter() {
        let Some(booking) = r.booking.filter(|_| r.deleted_ms.is_none()) else {
            continue;
        };
        match kept.entry((booking, r.url.clone())) {
            std::collections::hash_map::Entry::Occupied(first) => {
                doubles.push((uuid.clone(), first.get().clone()));
            }
            std::collections::hash_map::Entry::Vacant(free) => {
                free.insert(uuid.clone());
            }
        }
    }
    for (uuid, first) in doubles {
        let stamp = state[&first].updated_ms.max(state[&uuid].updated_ms) + 1;
        let r = state.get_mut(&uuid).expect("just seen");
        r.deleted_ms = Some(stamp);
        r.updated_ms = stamp;
        r.rev = format!("=> {first}");
        r.booking = None;
        r.account = None;
        r.url.clear();
        r.title.clear();
        r.doc_date.clear();
    }
}

/// Whether `booking` names a live booking.
fn is_live(conn: &Connection, booking: Option<i64>) -> CoreResult<bool> {
    let Some(id) = booking else {
        return Ok(false);
    };
    Ok(conn
        .query_row(
            // includes soft-deleted rows: that is the question
            "SELECT deleted_ms IS NULL FROM transactions WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(false))
}

fn has_table(conn: &Connection, schema: &str) -> CoreResult<bool> {
    Ok(conn.query_row(
        &format!(
            "SELECT count(*) FROM {schema}.sqlite_master
              WHERE type = 'table' AND name = 'document_links'"
        ),
        [],
        |r| r.get::<_, i64>(0),
    )? > 0)
}

/// Rows of `schema` by uuid, tombstones included (deletions travel), with
/// booking and account ids as stored there.
fn load(conn: &Connection, schema: &str) -> CoreResult<BTreeMap<String, Row>> {
    let rows = conn
        .prepare(&format!(
            "SELECT uuid, booking_id, account_id, url, title, doc_date, created_at, updated_ms,
                    rev, deleted_ms
               FROM {schema}.document_links"
        ))?
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                Row {
                    booking: r.get(1)?,
                    account: r.get(2)?,
                    url: r.get(3)?,
                    title: r.get(4)?,
                    doc_date: r.get(5)?,
                    created_at: r.get(6)?,
                    updated_ms: r.get(7)?,
                    rev: r.get(8)?,
                    deleted_ms: r.get(9)?,
                },
            ))
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

/// Merges `other.document_links` into `main.document_links`, after accounts
/// and bookings (`acc_map` / `booking_map`: id in `other` -> local id; the
/// latter from `sync_merge::merge_bookings`). Per reference the newest edit
/// (`updated_ms`, then `rev`) wins as a whole: booking, account, URL, title
/// and date. Content is never compared, so nothing device-specific (ids,
/// account references) can decide, and both merge orders pick the same edit.
/// The same edit on both sides, but only the other one still on a live
/// booking (its booking changed identity here, see the module docs): the
/// other one's booking is taken. A deletion always stays. Afterwards every
/// reference is brought to its form at rest ([`Row::normalize`]), the local
/// ones too; only a move there is stamped. Returns how many references were
/// added or changed here.
pub(crate) fn merge_attached(
    conn: &Connection,
    acc_map: &HashMap<i64, i64>,
    booking_map: &HashMap<i64, i64>,
) -> CoreResult<u32> {
    if !has_table(conn, "other")? {
        return Ok(0);
    }
    let mut changed = 0u32;
    let local = load(conn, "main")?;
    let mut state = local.clone();
    for (uuid, mut o) in load(conn, "other")? {
        if o.deleted_ms.is_some() {
            o.booking = None;
            o.account = None;
        }
        if let Some(other_booking) = o.booking {
            match booking_map.get(&other_booking) {
                Some(&id) => o.booking = Some(id),
                // its booking did not come over (account not merged): the
                // reference would arrive unlinked and look like an edit
                None => continue,
            }
        }
        o.account = o.account.and_then(|a| acc_map.get(&a).copied());
        let Some(s) = state.get_mut(&uuid) else {
            state.insert(uuid, o);
            continue;
        };
        let order = (o.updated_ms, &o.rev).cmp(&(s.updated_ms, &s.rev));
        match order {
            std::cmp::Ordering::Greater => {
                s.booking = o.booking;
                s.account = o.account;
                s.url = o.url;
                s.title = o.title;
                s.doc_date = o.doc_date;
                s.updated_ms = o.updated_ms;
                s.rev = o.rev;
            }
            std::cmp::Ordering::Equal
                if s.booking != o.booking
                    && !is_live(conn, s.booking)?
                    && is_live(conn, o.booking)? =>
            {
                s.booking = o.booking;
                s.account = o.account;
            }
            _ => {}
        }
        s.created_at = s.created_at.min(o.created_at);
        s.deleted_ms = s.deleted_ms.max(o.deleted_ms);
    }
    // per folded twin account: its live bank account (asked once)
    let mut folded: HashMap<i64, Option<i64>> = HashMap::new();
    for r in state.values_mut() {
        r.normalize(conn, &mut folded)?;
    }
    fold_doubles(&mut state);
    for (uuid, r) in &state {
        match local.get(uuid) {
            Some(old) if old == r => {}
            Some(_) => {
                changed += 1;
                conn.execute(
                    "UPDATE document_links
                        SET booking_id = ?2, account_id = ?3, url = ?4, title = ?5, doc_date = ?6,
                            created_at = ?7, updated_ms = ?8, rev = ?9, deleted_ms = ?10
                      WHERE uuid = ?1",
                    params![
                        uuid,
                        r.booking,
                        r.account,
                        r.url,
                        r.title,
                        r.doc_date,
                        r.created_at,
                        r.updated_ms,
                        r.rev,
                        r.deleted_ms
                    ],
                )?;
            }
            None => {
                changed += 1;
                conn.execute(
                    "INSERT INTO document_links(uuid, booking_id, account_id, url, title, doc_date,
                            created_at, updated_ms, rev, deleted_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        uuid,
                        r.booking,
                        r.account,
                        r.url,
                        r.title,
                        r.doc_date,
                        r.created_at,
                        r.updated_ms,
                        r.rev,
                        r.deleted_ms
                    ],
                )?;
            }
        }
    }
    Ok(changed)
}
