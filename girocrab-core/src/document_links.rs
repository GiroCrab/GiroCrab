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
//!
//! A document a bank delivered that moved into the document archive
//! (`archive/moves.rs`) is a reference too. It keeps the kind the stored
//! file had (`doc_type`: statement, PostBox …), which travels like its title.
//!
//! The origin the bank fetch knows such a document by stays on the device
//! that moved it (table `archived_sources`, not part of the vault merge):
//! while the reference lives, the fetch there does not store the document
//! again ([`has_source`]). An origin means nothing on another device. It is
//! built from ids of that device (the bank login), and every device fetches
//! the bank's documents itself. Another device therefore keeps its own copy
//! and shows that file instead of the reference (one document, one line),
//! until it moves its copy too; the two references to one document of the
//! archive are then one ([`fold_doubles`]).

use std::collections::{BTreeMap, HashMap};

use rusqlite::{params, Connection, OptionalExtension};
use url::Url;

use crate::dates::now_ms;
use crate::documents::{booking_label_of, DocumentFilter, DocumentInfo};
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

/// Part of step 33: what a reference was as a stored file (its kind), and
/// what this device alone knows about it: whether it is unread, and the
/// origins the bank fetch knows its document by.
pub(crate) fn origin_columns(conn: &Connection) -> CoreResult<()> {
    for (column, definition) in [
        ("doc_type", "TEXT NOT NULL DEFAULT 'beleg'"),
        // 1: it was a document a bank delivered. Travels with the reference,
        // unlike the origin the fetch knows it by.
        ("from_bank", "INTEGER NOT NULL DEFAULT 0"),
        // 1: unread, like a stored file with `seen_at IS NULL`. Local to the
        // device like that: not part of the vault merge.
        ("unseen", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        let has: i64 = conn.query_row(
            "SELECT count(*) FROM pragma_table_info('document_links') WHERE name = ?1",
            [column],
            |r| r.get(0),
        )?;
        if has == 0 {
            conn.execute_batch(&format!(
                "ALTER TABLE document_links ADD COLUMN {column} {definition};"
            ))?;
        }
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS document_links_unseen ON document_links(unseen)
             WHERE unseen = 1 AND deleted_ms IS NULL;
         -- A document of the bank that this device moved into the archive: the
         -- origin its fetch knows it by, the reference it stands behind, and
         -- the checksum of the file as it was stored (hex; connectors that
         -- tell their documents apart by content ask for it). Local to the
         -- device like booking_documents: an origin is built from ids of
         -- this device.
         CREATE TABLE IF NOT EXISTS archived_sources (
             source_url TEXT PRIMARY KEY,
             link_id    INTEGER NOT NULL REFERENCES document_links(id) ON DELETE CASCADE,
             digest     TEXT NOT NULL DEFAULT ''
         ) WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS archived_sources_link ON archived_sources(link_id);",
    )?;
    Ok(())
}

/// Whether a live reference stands for the bank's document `source_url`: this
/// device moved it into the archive, and its fetch does not store it again.
pub(crate) fn has_source(conn: &Connection, source_url: &str) -> CoreResult<bool> {
    if source_url.is_empty() {
        return Ok(false);
    }
    Ok(conn.query_row(LINKED_SOURCE_SQL, [source_url], |r| r.get(0))?)
}

/// "Does a live reference stand for the bank's document ?1?" (by the primary
/// key of `archived_sources`)
pub(crate) const LINKED_SOURCE_SQL: &str = "SELECT EXISTS (SELECT 1 FROM archived_sources s
      JOIN document_links l ON l.id = s.link_id
     WHERE s.source_url = ?1 AND l.deleted_ms IS NULL)";

/// The origins (with the checksums of their files, hex or '') of the bank's
/// documents behind live references, for origins matching the `LIKE`
/// pattern: what a connector that numbers its documents itself must count
/// along with the files still stored.
pub(crate) fn sources_like(conn: &Connection, pattern: &str) -> CoreResult<Vec<(String, String)>> {
    let rows = conn
        .prepare(
            "SELECT s.source_url, s.digest FROM archived_sources s
               JOIN document_links l ON l.id = s.link_id
              WHERE s.source_url LIKE ?1 AND l.deleted_ms IS NULL",
        )?
        .query_map([pattern], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// The bank's document `source_url` now belongs to the (live) account
/// `account_id`: a reference without account and booking that stands for it
/// takes the account, as an edit. Like the stored file in
/// `connectors::persist`, which gets its account when one is found later.
pub(crate) fn give_account(conn: &Connection, source_url: &str, account_id: i64) -> CoreResult<()> {
    conn.execute(
        "UPDATE document_links
            SET account_id = ?2, updated_ms = max(updated_ms + 1, ?3), rev = ?4
          WHERE id = (SELECT link_id FROM archived_sources WHERE source_url = ?1)
            AND account_id IS NULL AND booking_id IS NULL AND deleted_ms IS NULL",
        params![source_url, account_id, now_ms(), new_rev()],
    )?;
    Ok(())
}

/// A reference another device made to a document of the archive is not
/// shown while this device stores a file of a bank that is the same document
/// to the eye (same booking or account, kind, date and title): the other
/// device moved its copy, this one fetched its own. One document is one
/// line, and here the file is the better one. An origin could not tell (see
/// the module docs). A reference this device made itself is always shown:
/// its document is not fetched again, so a file that looks like it is
/// another document.
const NOT_STORED_HERE: &str = "NOT (l.archive_doc IS NOT NULL
        AND NOT EXISTS (SELECT 1 FROM archived_sources s WHERE s.link_id = l.id)
        AND EXISTS (
            SELECT 1 FROM booking_documents d INDEXED BY booking_documents_info
             WHERE d.doc_date = l.doc_date AND d.title = l.title AND d.doc_type = l.doc_type
               AND d.source_url != ''
               AND d.booking_id IS l.booking_id
               AND (l.booking_id IS NOT NULL OR d.account_id IS l.account_id)))";

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
/// The archive document is only read for the archive this device is
/// connected to (its id is parameter `archive`, '' without one): an id of
/// another archive means nothing here.
fn select_info(archive: usize) -> String {
    format!(
        "SELECT l.id, t.id, COALESCE(la.id, t.account_id), l.title, l.url,
            COALESCE(NULLIF(l.doc_date,''), t.booking_date, ''), l.created_at, a.name,
            t.counterparty, t.amount_cents, t.booking_date,
            CASE WHEN ?{archive} != '' AND l.archive_id = ?{archive} THEN l.archive_doc END,
            l.doc_type,
            COALESCE((SELECT min(s.source_url) FROM archived_sources s WHERE s.link_id = l.id), ''),
            l.unseen, l.from_bank
       FROM document_links l
       LEFT JOIN transactions t ON t.id = l.booking_id AND t.deleted_ms IS NULL
       LEFT JOIN accounts la ON la.id = l.account_id AND la.deleted_ms IS NULL
       LEFT JOIN accounts a ON a.id = COALESCE(la.id, t.account_id)"
    )
}

fn row_to_info(r: &rusqlite::Row) -> rusqlite::Result<DocumentInfo> {
    Ok(DocumentInfo {
        id: doc_id_of(r.get(0)?),
        booking_id: r.get(1)?,
        account_id: r.get(2)?,
        title: r.get(3)?,
        doc_type: r.get(12)?,
        mime_type: String::new(),
        file_name: String::new(),
        size_bytes: 0,
        source_url: r.get(13)?,
        link_url: r.get(4)?,
        archive_doc: r.get(11)?,
        doc_date: r.get(5)?,
        created_at: r.get(6)?,
        account_name: r.get::<_, Option<String>>(7)?.unwrap_or_default(),
        booking_label: booking_label_of(r.get(8)?, r.get(9)?, r.get(10)?),
        // a link the user made is nothing new to read; a document of the bank
        // that moved into the archive stays unread until it is opened
        seen: r.get::<_, i64>(14)? == 0,
        from_bank: r.get(15)?,
    })
}

/// The live references of a booking.
pub(crate) fn list_for_booking(
    conn: &Connection,
    booking_id: i64,
) -> CoreResult<Vec<DocumentInfo>> {
    let archive = crate::archive::current(conn)?;
    let archive_id = archive.as_ref().map(|a| a.0.as_str()).unwrap_or_default();
    let mut st = conn.prepare(&format!(
        "{} WHERE l.booking_id = ?1 AND l.deleted_ms IS NULL AND t.id IS NOT NULL
              AND {NOT_STORED_HERE}",
        select_info(2)
    ))?;
    let mut rows = st
        .query_map(params![booking_id, archive_id], row_to_info)?
        .collect::<Result<Vec<_>, _>>()?;
    crate::archive::localize(conn, &mut rows)?;
    Ok(rows)
}

/// The live references matching `filter`, in no particular order. `search`
/// is the prepared `LIKE` pattern; it looks at title, URL and counterparty.
pub(crate) fn list_all(
    conn: &Connection,
    filter: &DocumentFilter,
    search: Option<&str>,
) -> CoreResult<Vec<DocumentInfo>> {
    let archive = crate::archive::current(conn)?;
    let archive_id = archive.as_ref().map(|a| a.0.as_str()).unwrap_or_default();
    let mut st = conn.prepare(&format!(
        "{} WHERE l.deleted_ms IS NULL
            AND (?1 IS NULL OR COALESCE(la.id, t.account_id) = ?1)
            AND (?2 IS NULL OR l.title LIKE ?2 OR l.url LIKE ?2 OR t.counterparty LIKE ?2)
            AND (?3 = 0 OR t.id IS NULL)
            AND (?5 IS NULL OR l.doc_type = ?5)
            AND {NOT_STORED_HERE}",
        select_info(4)
    ))?;
    let mut rows = st
        .query_map(
            params![
                filter.account_id,
                search,
                filter.unlinked_only as i64,
                archive_id,
                filter.doc_type
            ],
            row_to_info,
        )?
        .collect::<Result<Vec<_>, _>>()?;
    crate::archive::localize(conn, &mut rows)?;
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

/// Whether booking `booking_id` already carries a live reference to `url`
/// or to the same document of the same archive (`archive`: archive id and
/// document id), other than the one in row `except`.
fn already_linked(
    conn: &Connection,
    booking_id: i64,
    url: &str,
    archive: Option<(&str, i64)>,
    except: Option<i64>,
) -> CoreResult<bool> {
    let (archive_id, archive_doc) = archive.unzip();
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM document_links
          WHERE booking_id = ?1 AND deleted_ms IS NULL AND id IS NOT ?5
            AND (url = ?2 OR (?4 IS NOT NULL AND archive_doc = ?4 AND archive_id = ?3)))",
        params![booking_id, url, archive_id, archive_doc, except],
        |r| r.get(0),
    )?)
}

/// Stores a reference on the live booking `booking_id`: `url` is checked and
/// normalised already, an empty `title` is the caller's to fill. Returns the
/// `DocumentInfo.id`.
pub(crate) fn insert(
    conn: &Connection,
    booking_id: i64,
    url: &str,
    title: &str,
    archive: Option<(&str, i64)>,
) -> CoreResult<i64> {
    let (account, date): (i64, String) = conn
        .query_row(
            "SELECT account_id, booking_date FROM transactions WHERE id = ?1 AND deleted_ms IS NULL",
            [booking_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| CoreError::NotFound {
            what: trf("Buchung {0}", &[&booking_id]),
        })?;
    if already_linked(conn, booking_id, url, archive, None)? {
        return Err(invalid(tr("Dieser Link hängt schon an der Buchung.")));
    }
    insert_row(
        conn,
        Some(booking_id),
        Some(account),
        url,
        title,
        &date,
        archive,
    )
}

/// What a stored file was before it moved into the archive.
pub(crate) struct Origin<'a> {
    /// Its kind (`booking_documents.doc_type`).
    pub(crate) doc_type: &'a str,
    /// The origin the bank fetch knows it by ('' for a receipt of the user's).
    pub(crate) source_url: &'a str,
    /// The checksum of the file (hex), if known.
    pub(crate) digest: &'a str,
    /// Not opened yet.
    pub(crate) unread: bool,
}

/// Stores a reference as it is: on a booking or none, with the given
/// account and date. For a stored file that moved into the archive
/// (`archive/moves.rs`), which keeps where and when it belonged. Returns the
/// `DocumentInfo.id`; `None` when the booking (or, without a booking, the
/// account) already carries this document of the archive: another device
/// moved its copy of the same document first, or the user linked it by hand.
///
/// The origin of a document a bank delivered is never lost: the reference
/// that stands for the document now (the new one, or the one that was there
/// already) is remembered for it, or the fetch would store the file again.
/// An unread file stays unread as a reference.
pub(crate) fn insert_moved(
    conn: &Connection,
    booking_id: Option<i64>,
    account_id: Option<i64>,
    url: &str,
    title: &str,
    doc_date: &str,
    archive: (&str, i64),
    origin: Origin,
) -> CoreResult<Option<i64>> {
    let carried: Option<i64> = match booking_id {
        Some(b) => conn
            .query_row(
                "SELECT min(id) FROM document_links
                  WHERE booking_id = ?1 AND deleted_ms IS NULL
                    AND (url = ?2 OR (archive_doc = ?4 AND archive_id = ?3))",
                params![b, url, archive.0, archive.1],
                |r| r.get(0),
            )
            .optional()?
            .flatten(),
        None => conn
            .query_row(
                "SELECT min(id) FROM document_links
                  WHERE booking_id IS NULL AND account_id IS ?1 AND deleted_ms IS NULL
                    AND archive_doc = ?3 AND archive_id = ?2",
                params![account_id, archive.0, archive.1],
                |r| r.get(0),
            )
            .optional()?
            .flatten(),
    };
    let (row, made) = match carried {
        Some(row) => {
            // It turns out to be a document of the bank: the reference that
            // was there says so from now on, with the kind the file had (an
            // edit, so every device shows it among the bank's documents).
            if !origin.source_url.is_empty() {
                conn.execute(
                    "UPDATE document_links
                        SET doc_type = ?2, from_bank = 1,
                            updated_ms = max(updated_ms + 1, ?3), rev = ?4
                      WHERE id = ?1 AND (doc_type != ?2 OR from_bank = 0)",
                    params![row, origin.doc_type, now_ms(), new_rev()],
                )?;
            }
            (row, None)
        }
        None => {
            let id = insert_row(
                conn,
                booking_id,
                account_id,
                url,
                title,
                doc_date,
                Some(archive),
            )?;
            let row = row_id_of(id).expect("a reference");
            conn.execute(
                "UPDATE document_links SET doc_type = ?2, from_bank = ?3 WHERE id = ?1",
                params![row, origin.doc_type, !origin.source_url.is_empty()],
            )?;
            (row, Some(id))
        }
    };
    if origin.unread {
        conn.execute("UPDATE document_links SET unseen = 1 WHERE id = ?1", [row])?;
    }
    if !origin.source_url.is_empty() {
        conn.execute(
            "INSERT INTO archived_sources(source_url, link_id, digest) VALUES (?1, ?2, ?3)
             ON CONFLICT(source_url) DO UPDATE SET link_id = excluded.link_id, digest = excluded.digest",
            params![origin.source_url, row, origin.digest],
        )?;
    }
    Ok(made)
}

/// Marks the reference in row `row_id` read or unread; returns how many
/// changed (0 or 1). Not an edit: the read state stays on this device.
pub(crate) fn mark_seen(conn: &Connection, row_id: i64, seen: bool) -> CoreResult<u32> {
    Ok(conn.execute(
        "UPDATE document_links SET unseen = ?2
          WHERE id = ?1 AND deleted_ms IS NULL AND unseen != ?2",
        params![row_id, !seen],
    )? as u32)
}

/// Marks every unread reference read; with `account_id` only those of that
/// account (its own and those of its bookings). Returns how many changed.
pub(crate) fn mark_all_seen(conn: &Connection, account_id: Option<i64>) -> CoreResult<u32> {
    Ok(conn.execute(
        "UPDATE document_links SET unseen = 0
          WHERE unseen = 1 AND deleted_ms IS NULL
            AND (?1 IS NULL OR account_id = ?1
                 OR booking_id IN (SELECT id FROM transactions WHERE account_id = ?1))",
        [account_id],
    )? as u32)
}

/// The unread references (the conditions of the partial index
/// `document_links_unseen`).
pub(crate) fn unseen_count(conn: &Connection) -> CoreResult<u32> {
    Ok(conn.query_row(
        "SELECT count(*) FROM document_links WHERE unseen = 1 AND deleted_ms IS NULL",
        [],
        |r| r.get(0),
    )?)
}

fn insert_row(
    conn: &Connection,
    booking_id: Option<i64>,
    account_id: Option<i64>,
    url: &str,
    title: &str,
    doc_date: &str,
    archive: Option<(&str, i64)>,
) -> CoreResult<i64> {
    conn.execute(
        "INSERT INTO document_links(uuid, booking_id, account_id, url, title, doc_date,
                updated_ms, rev, archive_id, archive_doc)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            uuid::Uuid::new_v4().to_string(),
            booking_id,
            account_id,
            url,
            title,
            doc_date,
            now_ms(),
            new_rev(),
            archive.map(|a| a.0).unwrap_or_default(),
            archive.map(|a| a.1)
        ],
    )?;
    Ok(doc_id_of(conn.last_insert_rowid()))
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
        let own: Option<(String, String, Option<i64>)> = conn
            .query_row(
                "SELECT url, archive_id, archive_doc FROM document_links
                  WHERE id = ?1 AND deleted_ms IS NULL",
                [row_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        if let Some((url, archive_id, archive_doc)) = own {
            let archive = archive_doc.map(|doc| (archive_id.as_str(), doc));
            if already_linked(conn, b, &url, archive, Some(row_id))? {
                return Err(invalid(tr("Dieser Link hängt schon an der Buchung.")));
            }
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
                booking_id = NULL, account_id = NULL, url = '', title = '', doc_date = '',
                archive_id = '', archive_doc = NULL, doc_type = 'beleg', from_bank = 0,
                unseen = 0
          WHERE id = ?1 AND deleted_ms IS NULL",
        params![row_id, now, new_rev()],
    )?;
    // what the fetch knew this reference for: the document may come again
    conn.execute("DELETE FROM archived_sources WHERE link_id = ?1", [row_id])?;
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
    /// of the reference. GiroCrab never fetches the URL. An address inside
    /// the connected document archive (`archive.rs`) is recognised as that
    /// archive's document, without a request.
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
        // an address of the connected archive names its document there
        let archive = crate::archive::document_in_archive(&db, &parsed)?;
        let archive = archive.as_ref().map(|(id, doc)| (id.as_str(), *doc));
        insert(&db, booking_id, parsed.as_str(), &title, archive)
    }
}

// ------------------------------------------------------------------ merge

/// One reference in a vault merge. `booking` and `account` are local ids.
#[derive(Clone, PartialEq)]
struct Row {
    booking: Option<i64>,
    account: Option<i64>,
    /// which archive `archive_doc` is an id of ('' = none)
    archive_id: String,
    archive_doc: Option<i64>,
    url: String,
    title: String,
    doc_date: String,
    /// what it was as a stored file: its kind, and whether a bank delivered it
    doc_type: String,
    from_bank: bool,
    created_at: i64,
    updated_ms: i64,
    rev: String,
    deleted_ms: Option<i64>,
}

impl Row {
    /// A deleted reference carries no content.
    fn clear(&mut self) {
        self.booking = None;
        self.account = None;
        self.archive_id.clear();
        self.archive_doc = None;
        self.url.clear();
        self.title.clear();
        self.doc_date.clear();
        self.doc_type = "beleg".into();
        self.from_bank = false;
    }
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
            self.clear();
            return Ok(());
        }
        let mut moved = false;
        if let Some(b) = self.booking {
            if let Some(copy) = crate::sync_merge::twins::folded_booking(conn, b, folded)? {
                moved |= copy != b;
                self.booking = Some(copy);
            }
        }
        if let Some(a) = self.account {
            if let Some(bank) = crate::sync_merge::twins::folded_account(conn, a, folded)? {
                moved |= bank != a;
                self.account = Some(bank);
            }
        }
        if moved {
            self.updated_ms = (self.updated_ms + 1).max(now_ms());
            self.rev = new_rev();
        }
        Ok(())
    }
}

/// The same document linked to the same booking on two devices before they
/// met is one reference: the one with the smallest uuid stays, the others
/// are deleted. The deletion is stamped from the references themselves, so
/// every device that sees both writes the very same one. Likewise one
/// document of the archive on one account without a booking (a statement
/// two devices each moved there). A plain address without a booking is never
/// folded: it may be several receipts, each with its own title and date.
fn fold_doubles(state: &mut BTreeMap<String, Row>) {
    // (booking, or the account where there is none; archive, document, address) -> kept uuid
    type Key = (Option<i64>, Option<i64>, String, Option<i64>, String);
    let mut kept: HashMap<Key, String> = HashMap::new();
    let mut doubles: Vec<(String, String)> = Vec::new();
    // in uuid order: the first of a kind is the smallest
    for (uuid, r) in state.iter() {
        if r.deleted_ms.is_some() {
            continue;
        }
        let key: Key = match (r.booking, r.archive_doc) {
            // the address of an archive may differ from device to device
            (Some(b), Some(doc)) => (
                Some(b),
                None,
                r.archive_id.clone(),
                Some(doc),
                String::new(),
            ),
            (Some(b), None) => (Some(b), None, String::new(), None, r.url.clone()),
            (None, Some(doc)) => (
                None,
                r.account,
                r.archive_id.clone(),
                Some(doc),
                String::new(),
            ),
            (None, None) => continue,
        };
        match kept.get(&key) {
            Some(first) => doubles.push((uuid.clone(), first.clone())),
            None => {
                kept.insert(key, uuid.clone());
            }
        }
    }
    for (uuid, first) in &doubles {
        let stamp = state[first].updated_ms.max(state[uuid].updated_ms) + 1;
        let r = state.get_mut(uuid).expect("just seen");
        let (from_bank, doc_type) = (r.from_bank, std::mem::take(&mut r.doc_type));
        r.deleted_ms = Some(stamp);
        r.updated_ms = stamp;
        r.rev = format!("=> {first}");
        r.clear();
        // That it is a document of a bank, and of which kind, belongs to the
        // document: the reference that stays says so too (it may be one the
        // user made by hand), stamped the same by every device that sees both.
        let stays = state.get_mut(first).expect("kept");
        if from_bank && !stays.from_bank {
            stays.from_bank = true;
            stays.doc_type = doc_type;
            stays.updated_ms = stamp;
            stays.rev = format!("<= {uuid}");
        }
    }
}

/// What this device alone knows about a reference (the origins its fetch
/// knows the document by, unread) must not go with a reference that a merge
/// deletes while the document is still referenced. It goes to the live
/// reference to the same document of the archive in the same place (its
/// booking, or its account without one): that is what a folded reference was
/// folded into, whichever device folded it, through however many folds and
/// whatever was edited since. Without such a reference it is dropped: the
/// document is no longer referenced here, and the fetch may bring it again.
/// `local` is the vault before the merge, `state` after it; a place of
/// before is also looked for where the merge moved it (`folded`: the twin
/// accounts folded into their bank accounts, as in [`Row::normalize`]).
fn keep_local_knowledge(
    conn: &Connection,
    local: &BTreeMap<String, Row>,
    state: &BTreeMap<String, Row>,
    folded: &mut HashMap<i64, Option<i64>>,
) -> CoreResult<()> {
    let holders: Vec<String> = conn
        .prepare(
            "SELECT uuid FROM document_links
              WHERE deleted_ms IS NOT NULL
                AND (unseen != 0 OR id IN (SELECT link_id FROM archived_sources))",
        )?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    for gone in holders {
        let mut stays: Option<String> = None;
        if let Some(was) = local.get(&gone).filter(|was| was.archive_doc.is_some()) {
            // where it hung, and where that is now
            let mut bookings = vec![was.booking];
            if let Some(b) = was.booking {
                if let Some(copy) = crate::sync_merge::twins::folded_booking(conn, b, folded)? {
                    bookings.push(Some(copy));
                }
            }
            let mut accounts = vec![was.account];
            if let Some(a) = was.account {
                if let Some(bank) = crate::sync_merge::twins::folded_account(conn, a, folded)? {
                    accounts.push(Some(bank));
                }
            }
            // in uuid order: the same one on every run
            stays = state
                .iter()
                .find(|(uuid, r)| {
                    **uuid != gone
                        && r.deleted_ms.is_none()
                        && r.archive_doc == was.archive_doc
                        && r.archive_id == was.archive_id
                        && match was.booking {
                            Some(_) => bookings.contains(&r.booking),
                            None => r.booking.is_none() && accounts.contains(&r.account),
                        }
                })
                .map(|(uuid, _)| uuid.clone());
        }
        if let Some(stays) = stays {
            conn.execute(
                "UPDATE archived_sources
                    SET link_id = (SELECT id FROM document_links WHERE uuid = ?2)
                  WHERE link_id = (SELECT id FROM document_links WHERE uuid = ?1)",
                params![gone, stays],
            )?;
            conn.execute(
                "UPDATE document_links SET unseen = 1
                  WHERE uuid = ?2 AND (SELECT unseen FROM document_links WHERE uuid = ?1) != 0",
                params![gone, stays],
            )?;
        }
        conn.execute(
            "UPDATE document_links SET unseen = 0 WHERE uuid = ?1",
            [&gone],
        )?;
        conn.execute(
            "DELETE FROM archived_sources
              WHERE link_id = (SELECT id FROM document_links WHERE uuid = ?1)",
            [&gone],
        )?;
    }
    Ok(())
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
                    rev, deleted_ms, archive_doc, archive_id, doc_type, from_bank
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
                    archive_doc: r.get(10)?,
                    archive_id: r.get(11)?,
                    doc_type: r.get(12)?,
                    from_bank: r.get(13)?,
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
            o.archive_id.clear();
            o.archive_doc = None;
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
                s.archive_id = o.archive_id;
                s.archive_doc = o.archive_doc;
                s.url = o.url;
                s.title = o.title;
                s.doc_date = o.doc_date;
                s.doc_type = o.doc_type;
                s.from_bank = o.from_bank;
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
                            created_at = ?7, updated_ms = ?8, rev = ?9, deleted_ms = ?10,
                            archive_doc = ?11, archive_id = ?12, doc_type = ?13, from_bank = ?14
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
                        r.deleted_ms,
                        r.archive_doc,
                        r.archive_id,
                        r.doc_type,
                        r.from_bank
                    ],
                )?;
            }
            None => {
                changed += 1;
                conn.execute(
                    "INSERT INTO document_links(uuid, booking_id, account_id, url, title, doc_date,
                            created_at, updated_ms, rev, deleted_ms, archive_doc, archive_id,
                            doc_type, from_bank)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
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
                        r.deleted_ms,
                        r.archive_doc,
                        r.archive_id,
                        r.doc_type,
                        r.from_bank
                    ],
                )?;
            }
        }
    }
    keep_local_knowledge(conn, &local, &state, &mut folded)?;
    Ok(changed)
}
