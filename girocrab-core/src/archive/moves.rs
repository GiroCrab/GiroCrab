//! Moving a stored file into the document archive: the receipt ends up where
//! it belongs (the archive reads its text, keeps and backs it up) and the
//! vault keeps only the reference.
//!
//! The file stays in the vault, readable and previewable as before, until the
//! archive has taken it in and the document can be read there. Only then is
//! the reference stored and the file removed, in one transaction.
//!
//! A move is one line in `archive_moves`, identified by a random `op`. Every
//! request is followed by a look at that line: if it is gone, belongs to
//! another `op`, another archive or another step (the user kept the file in
//! the vault, deleted it, queued it anew, changed the archive; a second run
//! was faster), the answer is dropped and nothing is touched. The id of a
//! stored file alone would not do: SQLite hands the id of a deleted file to
//! the next one.
//!
//! The steps:
//! 1. `queued`. A run claims the line (`posting`) before it sends anything,
//!    so two runs never post the same file.
//! 2. `posting`. The file is posted; the archive answers with the id of its
//!    processing task (`uploaded`). If no connection could be made, nothing
//!    was sent: back to `queued`. If the request went out and no answer came
//!    back, or the app quit in between, nobody knows whether the archive has
//!    the file. It is not posted again unasked (an archive may accept the
//!    same file twice): the move fails with that explanation and the user
//!    decides.
//! 3. `uploaded`. The task is asked for until it is done. Before the file is
//!    removed, the document it names is read from the archive: a document in
//!    the archive's trash, or one this token may not read, is no home for a
//!    receipt. The reference then goes onto whatever booking the file hangs
//!    on *now*.
//! 4. `failed`. The file stays; the user can try again or keep it in the vault.
//!
//! The table is local to the device, like the files in `booking_documents`
//! it points at: neither is part of the vault merge.
//!
//! Documents a bank delivered (statements, PostBox) move the same way. The
//! bank fetch knows a document by the origin stored with its file
//! (`source_url`); the reference takes that origin over, so the document is
//! not fetched into the vault again (`documents::knows_source`). GiroCrab
//! knows the bank, the account and the kind of such a document and hands
//! them to the archive with the file ([`Labels`]). The origin is remembered
//! on this device only (`document_links::archived_sources`): every device
//! fetches the bank's documents itself and moves its own copy. If the
//! archive has the very same file already (another device was first), that
//! document is linked and nothing is uploaded.
//!
//! "Keep in the vault" is a state of its own (`kept`): such a file is not
//! queued again by itself, only when the user asks for this file.

use std::sync::Arc;

use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use super::backend::{Backend, Labels, TaskState, Upload, Uploaded};
use super::client::Failure;
use super::{connection, current, kind_of, setting, store, Connected};
use crate::connectors::MAX_DOC_BYTES;
use crate::dates::now_ms;
use crate::error::{CoreError, CoreResult};
use crate::i18n::{tr, trf};
use crate::vault::Vault;

/// Step 34: stored files waiting to be moved into the archive.
pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS archive_moves (
    doc_id      INTEGER PRIMARY KEY REFERENCES booking_documents(id) ON DELETE CASCADE,
    op          TEXT    NOT NULL,                  -- this move; every write names it
    archive_id  TEXT    NOT NULL,                  -- the archive it goes to
    state       TEXT    NOT NULL DEFAULT 'queued', -- queued | posting | uploaded | failed | kept (stays in the vault)
    task        TEXT    NOT NULL DEFAULT '',       -- the archive's processing task, once it took the upload
    attempts    INTEGER NOT NULL DEFAULT 0,        -- tries in the current state
    claimed_ms  INTEGER NOT NULL DEFAULT 0,        -- when a run took it for posting
    problem     TEXT    NOT NULL DEFAULT '',       -- why it failed, worded for the user
    digest      TEXT    NOT NULL DEFAULT ''        -- checksum of the file once posted: the same file queued twice goes up once
);
";

/// `1`: files the user adds are moved into the archive right away.
pub(crate) const KEY_UPLOAD: &str = "archive.upload";
/// Set: documents the banks deliver are moved into the archive as they
/// arrive. The value is the time it was switched on (unix seconds): what
/// was fetched before stays until the user moves the files that are there.
pub(crate) const KEY_BANK: &str = "archive.bank";
/// `0`: a document of the bank goes up without sender, type and tag.
pub(crate) const KEY_LABELS: &str = "archive.labels";

/// Files posted in one run (each can be many megabytes).
const POSTS_PER_RUN: usize = 5;
/// Tasks asked about in one run.
const CHECKS_PER_RUN: usize = 50;
/// Refused posts before a file is given up on.
const MAX_POSTS: i64 = 3;
/// Runs in which the archive may not know a task (it is told about an upload
/// a moment after answering it) before the file is given up on.
const MAX_UNKNOWN: i64 = 12;
/// A line in `posting` older than this was left by a run that is gone (the
/// request itself times out after a minute).
pub(super) const POSTING_STALE_MS: i64 = 3 * 60 * 1000;

const UNCLEAR: &str = "Unklar, ob das Dokumentenarchiv die Datei übernommen hat. Bitte dort nachsehen, bevor Sie es erneut versuchen.";
/// `archive_moves.task` of an `uploaded` file: the handle to ask the archive
/// with, or the document itself when there is nothing to ask (the archive
/// had the file already).
const TASK: &str = "task:";
const DOC: &str = "doc:";
/// `archive_moves.task` of a `failed` file whose upload may have arrived (it
/// stays when that file is kept in the vault): its twins must not be
/// uploaded on top of it.
const MAYBE_THERE: &str = "unclear";
const TWIN_UNCLEAR: &str = "Dieselbe Datei wurde schon bei einem anderen Beleg hochgeladen, mit unklarem Ausgang. Bitte zuerst dort erneut versuchen.";
const NOT_THE_SAME: &str =
    "Das Dokumentenarchiv gibt die Datei nicht unverändert zurück. Sie bleibt im Tresor.";
const IN_TRASH: &str = "Das Dokumentenarchiv hat die Datei, aber das Dokument ist dort nicht lesbar (zum Beispiel im Papierkorb). Die Datei bleibt im Tresor.";

/// A stored file on its way into the archive, for the document lists.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ArchiveMove {
    /// `DocumentInfo.id` of the stored file.
    pub doc_id: i64,
    /// `queued` / `posting` (waits for or is in the upload), `uploaded` (the
    /// archive is working on it) or `failed`.
    pub state: String,
    /// Why it failed (empty otherwise).
    pub problem: String,
}

/// What one run of `process_archive_moves` did.
#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ArchiveMoveReport {
    /// Files that are in the archive now and gone from the vault.
    pub moved: u32,
    /// Files still waiting (upload, or the archive is working): run again.
    pub waiting: u32,
    /// Files given up on in this run.
    pub failed: u32,
}

/// The stored files this device moves and where they are, for the settings.
#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ArchiveMoveSummary {
    /// Files stored in the vault that are not queued: the user's own
    /// receipts, and the bank's documents while those are moved too
    /// (`archive_bank_enabled`). What `queue_archive_moves` would queue.
    pub in_vault: u32,
    /// Files on their way (queued, posting, uploaded).
    pub waiting: u32,
    /// Files whose move failed.
    pub failed: u32,
}

fn invalid(msg: &str) -> CoreError {
    CoreError::InvalidInput {
        message: msg.to_string(),
    }
}

// ------------------------------------------------------------------ the line of a move

/// A move as a run took it up: what must still be true when it writes.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Claim {
    pub(super) doc_id: i64,
    pub(super) op: String,
    pub(super) archive_id: String,
    pub(super) task: String,
}

impl Claim {
    /// Whether the line is still this move in `state`, and the vault is still
    /// connected to its archive.
    pub(super) fn holds(&self, conn: &Connection, state: &str) -> CoreResult<bool> {
        if current(conn)?.map(|c| c.0).as_deref() != Some(self.archive_id.as_str()) {
            return Ok(false);
        }
        Ok(conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM archive_moves
              WHERE doc_id = ?1 AND op = ?2 AND archive_id = ?3 AND state = ?4)",
            params![self.doc_id, self.op, self.archive_id, state],
            |r| r.get(0),
        )?)
    }

    /// Moves the line from `from` to `to`, if it is still this move there.
    /// `false`: it is not, nothing was written.
    fn step(
        &self,
        conn: &Connection,
        from: &str,
        to: &str,
        task: &str,
        problem: &str,
    ) -> CoreResult<bool> {
        if !self.holds(conn, from)? {
            return Ok(false);
        }
        Ok(conn.execute(
            "UPDATE archive_moves
                SET state = ?5, task = ?6, problem = ?7, claimed_ms = ?8,
                    -- refusals are counted across the tries of a file; a new step starts at 0
                    attempts = CASE WHEN ?5 IN ('uploaded', 'failed') THEN 0 ELSE attempts END
              WHERE doc_id = ?1 AND op = ?2 AND archive_id = ?3 AND state = ?4",
            params![
                self.doc_id,
                self.op,
                self.archive_id,
                from,
                to,
                task,
                problem,
                now_ms()
            ],
        )? == 1)
    }

    /// One more try in `state`; `true` once `max` is reached.
    fn tried(&self, conn: &Connection, state: &str, max: i64) -> CoreResult<bool> {
        if !self.holds(conn, state)? {
            return Ok(false);
        }
        conn.execute(
            "UPDATE archive_moves SET attempts = attempts + 1 WHERE doc_id = ?1 AND op = ?2",
            params![self.doc_id, self.op],
        )?;
        let attempts: i64 = conn.query_row(
            "SELECT attempts FROM archive_moves WHERE doc_id = ?1 AND op = ?2",
            params![self.doc_id, self.op],
            |r| r.get(0),
        )?;
        Ok(attempts >= max)
    }

    /// Takes a `queued` line for posting. `false`: another run has it, or it
    /// is no longer queued.
    pub(super) fn claim(&self, conn: &Connection) -> CoreResult<bool> {
        self.step(conn, "queued", "posting", "", "")
    }

    /// The archive took the upload: its task is remembered.
    pub(super) fn posted(&self, conn: &Connection, task: &str) -> CoreResult<bool> {
        self.step(conn, "posting", "uploaded", task, "")
    }

    /// The archive refused the posted file: back into the queue, or given up
    /// on (`true`) once it has refused [`MAX_POSTS`] times.
    fn refused(&self, conn: &Connection) -> CoreResult<bool> {
        if self.tried(conn, "posting", MAX_POSTS)? {
            return self.fail(
                conn,
                "posting",
                tr("Das Dokumentenarchiv hat die Datei nicht angenommen."),
            );
        }
        self.step(conn, "posting", "queued", "", "")?;
        Ok(false)
    }

    /// Gives the move up with an explanation, if it is still in `state`.
    pub(super) fn fail(&self, conn: &Connection, state: &str, problem: &str) -> CoreResult<bool> {
        self.step(conn, state, "failed", "", problem)
    }

    /// Gives the move up because nobody knows whether the archive has the
    /// file: marked, so that the same file is not uploaded for another
    /// receipt meanwhile.
    fn fail_unclear(&self, conn: &Connection, state: &str) -> CoreResult<bool> {
        self.step(conn, state, "failed", MAYBE_THERE, tr(UNCLEAR))
    }
}

/// The stored file behind a move, as it is now.
struct Stored {
    title: String,
    file_name: String,
    mime: String,
    doc_date: String,
    bytes: Vec<u8>,
    /// The origin the bank fetch knows it by ('' for a receipt of the user's).
    source_url: String,
    doc_type: String,
    /// Bank and name of the (live) account it belongs to, '' if none.
    bank: String,
    account: String,
}

fn stored(conn: &Connection, doc_id: i64) -> CoreResult<Option<Stored>> {
    Ok(conn
        .query_row(
            "SELECT d.title, d.file_name, d.mime_type, d.doc_date, d.pdf_bytes, d.source_url,
                    d.doc_type, COALESCE(a.bank_name, ''), COALESCE(a.name, '')
               FROM booking_documents d
               LEFT JOIN transactions t ON t.id = d.booking_id AND t.deleted_ms IS NULL
               LEFT JOIN accounts da ON da.id = d.account_id AND da.deleted_ms IS NULL
               LEFT JOIN accounts a ON a.id = COALESCE(da.id, t.account_id)
              WHERE d.id = ?1",
            [doc_id],
            |r| {
                Ok(Stored {
                    title: r.get(0)?,
                    file_name: r.get(1)?,
                    mime: r.get(2)?,
                    doc_date: r.get(3)?,
                    bytes: r.get(4)?,
                    source_url: r.get(5)?,
                    doc_type: r.get(6)?,
                    bank: r.get(7)?,
                    account: r.get(8)?,
                })
            },
        )
        .optional()?)
}

/// What GiroCrab knows about a document a bank delivered, for the archive:
/// the bank as its sender, the kind of document, the account as a tag. A
/// receipt of the user's own gets none: whose invoice it is, the archive
/// reads from the document itself.
fn labels_of(conn: &Connection, file: &Stored) -> CoreResult<Labels> {
    if file.source_url.is_empty() || setting(conn, KEY_LABELS)?.as_deref() == Some("0") {
        return Ok(Labels::default());
    }
    let kind = match file.doc_type.as_str() {
        "kontoauszug" => Some(tr("Kontoauszug")),
        "abrechnung" => Some(tr("Abrechnung")),
        "steuer" => Some(tr("Steuerunterlage")),
        "postbox" => Some(tr("Bankmitteilung")),
        _ => None,
    };
    let named = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
    Ok(Labels {
        sender: named(&file.bank),
        kind: kind.map(str::to_string),
        tags: named(&file.account).into_iter().collect(),
    })
}

/// The file is in the archive as the readable document `archive_doc`: stores
/// the reference where the file hangs now and removes the file, all or
/// nothing. `false`: the line is no longer this move in `uploaded` with this
/// task; nothing was touched.
pub(super) fn finish(
    conn: &mut Connection,
    claim: &Claim,
    base: &url::Url,
    archive_doc: i64,
) -> CoreResult<bool> {
    let tx = conn.transaction()?;
    let same_task: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM archive_moves WHERE doc_id = ?1 AND op = ?2 AND task = ?3)",
        params![claim.doc_id, claim.op, claim.task],
        |r| r.get(0),
    )?;
    if !same_task || !claim.holds(&tx, "uploaded")? {
        return Ok(false);
    }
    // a link to a deleted booking or account reads as none, like in the lists
    let (booking, account, title, file_name, doc_date, doc_type, source_url, unread, digest): (
        Option<i64>,
        Option<i64>,
        String,
        String,
        String,
        String,
        String,
        bool,
        String,
    ) = tx.query_row(
        "SELECT t.id, COALESCE(da.id, t.account_id), d.title, d.file_name,
                COALESCE(NULLIF(d.doc_date,''), t.booking_date, ''), d.doc_type, d.source_url,
                d.seen_at IS NULL,
                COALESCE((SELECT m.digest FROM archive_moves m WHERE m.doc_id = d.id), '')
           FROM booking_documents d
           LEFT JOIN transactions t ON t.id = d.booking_id AND t.deleted_ms IS NULL
           LEFT JOIN accounts da ON da.id = d.account_id AND da.deleted_ms IS NULL
          WHERE d.id = ?1",
        [claim.doc_id],
        |r| {
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
            ))
        },
    )?;
    let mut title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        title = crate::documents::title_from_file(&file_name);
    }
    if title.is_empty() {
        title = trf("Dokument {0}", &[&archive_doc]);
    }
    crate::document_links::insert_moved(
        &tx,
        booking,
        account,
        &kind_of(&tx)?.document_url(base, archive_doc),
        &title,
        &doc_date,
        (&claim.archive_id, archive_doc),
        // what the file was: this device remembers the origin the bank fetch
        // knows it by; unread in the vault is unread as a reference
        crate::document_links::Origin {
            doc_type: &doc_type,
            source_url: &source_url,
            digest: &digest,
            unread,
        },
    )?;
    // the move's own line goes with the file (ON DELETE CASCADE)
    tx.execute(
        "DELETE FROM booking_documents WHERE id = ?1",
        [claim.doc_id],
    )?;
    tx.execute(
        "DELETE FROM archive_moves WHERE doc_id = ?1 AND op = ?2",
        params![claim.doc_id, claim.op],
    )?;
    tx.commit()?;
    Ok(true)
}

/// Since when (unix seconds) documents of the banks are moved as they
/// arrive; `None` while they stay in the vault.
fn bank_since(conn: &Connection) -> CoreResult<Option<i64>> {
    Ok(setting(conn, KEY_BANK)?.and_then(|v| v.parse().ok()))
}

fn upload_enabled(conn: &Connection) -> CoreResult<bool> {
    Ok(setting(conn, KEY_UPLOAD)?.as_deref() == Some("1"))
}

fn new_op() -> String {
    uuid::Uuid::new_v4().to_string()
}

// ------------------------------------------------------------------ queue

#[uniffi::export]
impl Vault {
    /// Whether files the user adds are moved into the connected archive
    /// right away (off without a connection).
    pub fn archive_upload_enabled(&self) -> Result<bool, CoreError> {
        let db = self.db()?;
        Ok(current(&db)?.is_some() && upload_enabled(&db)?)
    }

    pub fn set_archive_upload(&self, enabled: bool) -> Result<(), CoreError> {
        let db = self.db()?;
        if enabled {
            store(&db, KEY_UPLOAD, "1", false)
        } else {
            db.execute("DELETE FROM settings WHERE key = ?1", [KEY_UPLOAD])?;
            Ok(())
        }
    }

    /// Whether documents the banks deliver are moved into the connected
    /// archive as they arrive (off without a connection).
    pub fn archive_bank_enabled(&self) -> Result<bool, CoreError> {
        let db = self.db()?;
        Ok(current(&db)?.is_some() && bank_since(&db)?.is_some())
    }

    /// Switched on, documents fetched from now on are moved
    /// (`queue_bank_documents`); those in the vault already stay until
    /// `queue_archive_moves` takes them along.
    pub fn set_archive_bank(&self, enabled: bool) -> Result<(), CoreError> {
        let db = self.db()?;
        if !enabled {
            db.execute("DELETE FROM settings WHERE key = ?1", [KEY_BANK])?;
        } else if bank_since(&db)?.is_none() {
            store(&db, KEY_BANK, &(now_ms() / 1000).to_string(), false)?;
        }
        Ok(())
    }

    /// Whether a document of the bank goes up with what GiroCrab knows
    /// about it: the bank as sender, the kind of document, the account as a
    /// tag (on unless switched off).
    pub fn archive_labels_enabled(&self) -> Result<bool, CoreError> {
        Ok(setting(&*self.db()?, KEY_LABELS)?.as_deref() != Some("0"))
    }

    pub fn set_archive_labels(&self, enabled: bool) -> Result<(), CoreError> {
        let db = self.db()?;
        if enabled {
            db.execute("DELETE FROM settings WHERE key = ?1", [KEY_LABELS])?;
            Ok(())
        } else {
            store(&db, KEY_LABELS, "0", false)
        }
    }

    /// Queues the documents of the banks that arrived since they are moved
    /// (`set_archive_bank`). Call it after a fetch.
    /// Returns how many were queued; none while the banks' documents stay
    /// in the vault or another archive than `archive_id` is connected.
    pub fn queue_bank_documents(&self, archive_id: String) -> Result<u32, CoreError> {
        let db = self.db()?;
        let connected = connection(&db)?;
        let Some(since) = bank_since(&db)? else {
            return Ok(0);
        };
        if connected.id != archive_id {
            return Ok(0);
        }
        // every column asked for is in the index `booking_documents_info`:
        // no stored file is read
        let queued = db.execute(
            "INSERT OR IGNORE INTO archive_moves(doc_id, op, archive_id)
             SELECT d.id, lower(hex(randomblob(16))), ?2 FROM booking_documents d
              WHERE d.id IN (SELECT id FROM booking_documents WHERE source_url != '')
                AND length(d.pdf_bytes) <= ?1
                AND d.created_at >= ?3
                AND d.id NOT IN (SELECT doc_id FROM archive_moves)",
            params![MAX_DOC_BYTES as i64, connected.id, since],
        )?;
        Ok(queued as u32)
    }

    /// Queues a stored file (a receipt of the user's own or a document a
    /// bank delivered) for the move into the connected archive, or
    /// queues it anew after a failure or after it was kept in the vault (a
    /// new move: answers to the old one no longer count). A move that is
    /// under way is left alone. Sends nothing:
    /// `process_archive_moves` does.
    pub fn queue_archive_move(&self, doc_id: i64) -> Result<(), CoreError> {
        let db = self.db()?;
        let connected = connection(&db)?;
        let size: Option<i64> = db
            .query_row(
                "SELECT length(pdf_bytes) FROM booking_documents WHERE id = ?1",
                [doc_id],
                |r| r.get(0),
            )
            .optional()?;
        let Some(size) = size else {
            return Err(CoreError::NotFound {
                what: trf("Beleg {0}", &[&doc_id]),
            });
        };
        if size as usize > MAX_DOC_BYTES {
            return Err(invalid(tr(
                "Die Datei ist zu groß für das Hochladen ins Dokumentenarchiv.",
            )));
        }
        db.execute(
            "INSERT INTO archive_moves(doc_id, op, archive_id) VALUES (?1, ?2, ?3)
             ON CONFLICT(doc_id) DO UPDATE SET op = excluded.op, archive_id = excluded.archive_id,
                 state = 'queued', task = '', attempts = 0, claimed_ms = 0, problem = ''
              WHERE archive_moves.state IN ('failed', 'kept')
                 OR archive_moves.archive_id != excluded.archive_id",
            params![doc_id, new_op(), connected.id],
        )?;
        Ok(())
    }

    /// Queues the user's own receipts that are not queued yet: all of them,
    /// or of the files `doc_ids` those that are such receipts (the ids
    /// `attach_document_to_booking` and its kin returned). "All of them"
    /// takes the bank's documents along while those are moved too
    /// (`archive_bank_enabled`). Returns how many were queued. One
    /// statement, however many.
    ///
    /// `archive_id` is the archive they are meant for
    /// (`archive_identities`): when another one is connected by the time
    /// this runs, nothing is queued.
    pub fn queue_archive_moves(
        &self,
        doc_ids: Option<Vec<i64>>,
        archive_id: String,
    ) -> Result<u32, CoreError> {
        let db = self.db()?;
        let connected = connection(&db)?;
        if connected.id != archive_id {
            return Ok(0);
        }
        let with_bank = doc_ids.is_none() && bank_since(&db)?.is_some();
        let only = doc_ids.map(|ids| serde_json::json!(ids).to_string());
        // The bank's documents come from the partial index on `source_url`:
        // that column lies behind the file in the row. The `op` only has to
        // be random.
        let queued = db.execute(
            "INSERT OR IGNORE INTO archive_moves(doc_id, op, archive_id)
             SELECT d.id, lower(hex(randomblob(16))), ?3 FROM booking_documents d
              WHERE (?1 IS NULL OR d.id IN (SELECT value FROM json_each(?1)))
                AND length(d.pdf_bytes) <= ?2
                AND (?4 OR d.id NOT IN (SELECT id FROM booking_documents WHERE source_url != ''))
                AND d.id NOT IN (SELECT doc_id FROM archive_moves)",
            params![only, MAX_DOC_BYTES as i64, connected.id, with_bank],
        )?;
        Ok(queued as u32)
    }

    /// Keeps a file in the vault: it comes off the queue, whatever the
    /// archive answers later (if the archive already took it in, the
    /// document stays there too), and is not queued again by itself, neither
    /// after the next fetch nor with "everything that is there". Only
    /// `queue_archive_move` for this very file moves it after all. The mark
    /// goes with the file, and with the connection to this archive.
    pub fn cancel_archive_move(&self, doc_id: i64) -> Result<(), CoreError> {
        let db = self.db()?;
        let archive_id = current(&db)?.map(|c| c.0).unwrap_or_default();
        // (a file that is gone has no line to keep: the insert fails on its
        // foreign key, and that is fine)
        // A file that was (or is being) handed to the archive may be there:
        // that stays known, so that the same file is not uploaded for another
        // receipt on top of it.
        let _ = db.execute(
            "INSERT INTO archive_moves(doc_id, op, archive_id, state) VALUES (?1, ?2, ?3, 'kept')
             ON CONFLICT(doc_id) DO UPDATE SET
                 task = CASE WHEN archive_moves.state IN ('posting', 'uploaded')
                               OR archive_moves.task = ?4 THEN ?4 ELSE '' END,
                 state = 'kept', problem = '', attempts = 0",
            params![doc_id, new_op(), archive_id, MAYBE_THERE],
        );
        Ok(())
    }

    /// The stored files that are on their way into the archive or failed
    /// (not those kept in the vault: nothing is going on with them).
    pub fn archive_moves(&self) -> Result<Vec<ArchiveMove>, CoreError> {
        let db = self.db()?;
        let mut st =
            db.prepare("SELECT doc_id, state, problem FROM archive_moves WHERE state != 'kept' ORDER BY doc_id")?;
        let rows = st
            .query_map([], |r| {
                Ok(ArchiveMove {
                    doc_id: r.get(0)?,
                    state: r.get(1)?,
                    problem: r.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// How many of the files this device moves are still in the vault, on
    /// their way, or failed: three counts, no list.
    pub fn archive_move_summary(&self) -> Result<ArchiveMoveSummary, CoreError> {
        let db = self.db()?;
        let count = |sql: &str| -> CoreResult<u32> { Ok(db.query_row(sql, [], |r| r.get(0))?) };
        let (own, queued_own) = if bank_since(&db)?.is_some() {
            (
                count("SELECT count(*) FROM booking_documents")?,
                count("SELECT count(*) FROM archive_moves")?,
            )
        } else {
            (
                count(
                    "SELECT (SELECT count(*) FROM booking_documents)
                          - (SELECT count(*) FROM booking_documents WHERE source_url != '')",
                )?,
                count(
                    "SELECT count(*) FROM archive_moves
                      WHERE doc_id NOT IN (SELECT id FROM booking_documents WHERE source_url != '')",
                )?,
            )
        };
        Ok(ArchiveMoveSummary {
            in_vault: own.saturating_sub(queued_own),
            waiting: count(
                "SELECT count(*) FROM archive_moves WHERE state NOT IN ('failed', 'kept')",
            )?,
            failed: count("SELECT count(*) FROM archive_moves WHERE state = 'failed'")?,
        })
    }
}

/// The lines of the connected archive in `state`, the ones that waited
/// longest first.
fn lines(conn: &Connection, archive_id: &str, state: &str, limit: usize) -> CoreResult<Vec<Claim>> {
    let mut st = conn.prepare(
        "SELECT doc_id, op, task FROM archive_moves
          WHERE archive_id = ?1 AND state = ?2 ORDER BY claimed_ms, doc_id LIMIT ?3",
    )?;
    let rows = st
        .query_map(params![archive_id, state, limit as i64], |r| {
            Ok(Claim {
                doc_id: r.get(0)?,
                op: r.get(1)?,
                archive_id: archive_id.to_string(),
                task: r.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Lines left in `posting` by a run that is gone (the app quit, the request
/// was cancelled): whether the archive has the file is not known. They fail
/// with that explanation instead of being posted again. Returns how many.
pub(super) fn settle_stale_posts(conn: &Connection, archive_id: &str, now: i64) -> CoreResult<u32> {
    Ok(conn.execute(
        "UPDATE archive_moves SET state = 'failed', problem = ?3, attempts = 0, task = ?4
          WHERE archive_id = ?1 AND state = 'posting' AND claimed_ms < ?2",
        params![archive_id, now - POSTING_STALE_MS, tr(UNCLEAR), MAYBE_THERE],
    )? as u32)
}

fn waiting_count(conn: &Connection, archive_id: &str) -> CoreResult<u32> {
    Ok(conn.query_row(
        "SELECT count(*) FROM archive_moves
          WHERE archive_id = ?1 AND state NOT IN ('failed', 'kept')",
        [archive_id],
        |r| r.get(0),
    )?)
}

// ------------------------------------------------------------------ a run

enum Step {
    Moved,
    Waiting,
    Failed,
}

#[uniffi::export(async_runtime = "tokio")]
impl Vault {
    /// Works on the queue: asks the archive about the files it is
    /// processing and finishes those that are done, then posts waiting files.
    /// Safe to call at any time, as often as wanted and from several places
    /// at once; call it again while the report says files are `waiting`. The
    /// vault lock is never held during a request, and every answer is checked
    /// against the queue before anything is written.
    pub async fn process_archive_moves(self: Arc<Self>) -> Result<ArchiveMoveReport, CoreError> {
        let watch = self.archive_watch();
        let mut report = ArchiveMoveReport::default();
        let (connected, checks, posts) = {
            let db = self.db()?;
            let connected = connection(&db)?;
            report.failed += settle_stale_posts(&db, &connected.id, now_ms())?;
            let checks = lines(&db, &connected.id, "uploaded", CHECKS_PER_RUN)?;
            let posts = lines(&db, &connected.id, "queued", POSTS_PER_RUN)?;
            (connected, checks, posts)
        };
        if !checks.is_empty() || !posts.is_empty() {
            let archive = connected.backend(watch).await?;
            let mut stopped = None;
            for claim in &checks {
                match self.check_move(archive.as_ref(), &connected, claim).await {
                    Ok(Step::Moved) => report.moved += 1,
                    Ok(Step::Failed) => report.failed += 1,
                    Ok(Step::Waiting) => {}
                    Err(e) => {
                        stopped = Some(e);
                        break;
                    }
                }
            }
            if stopped.is_none() {
                for claim in &posts {
                    match self.post_move(archive.as_ref(), claim).await {
                        Ok(Step::Failed) => report.failed += 1,
                        Ok(_) => {}
                        Err(e) => {
                            stopped = Some(e);
                            break;
                        }
                    }
                }
            }
            // the archive is not there right now: what was done counts, the rest waits
            if let Some(e) = stopped {
                if report.moved + report.failed == 0 {
                    return Err(e);
                }
            }
        }
        report.waiting = waiting_count(&*self.db()?, &connected.id)?;
        Ok(report)
    }
}

impl Vault {
    /// Posts one queued file. An error means the archive cannot be worked
    /// with right now (the file is back in the queue, untouched).
    async fn post_move(&self, archive: &dyn Backend, claim: &Claim) -> CoreResult<Step> {
        let file = {
            let db = self.db()?;
            // taken before anything is read or sent: no second run posts it
            if !claim.claim(&db)? {
                return Ok(Step::Waiting);
            }
            match stored(&db, claim.doc_id)? {
                Some(file) => file,
                None => return Ok(Step::Waiting), // deleted; its line went with it
            }
        };
        let title = if file.title.trim().is_empty() {
            crate::documents::title_from_file(&file.file_name)
        } else {
            file.title.clone()
        };
        let labels = labels_of(&*self.db()?, &file)?;
        // An archive that takes the same file twice keeps it twice. So the
        // same file is never on its way twice: while a twin is being posted
        // or processed, this one waits and is then linked to the twin's
        // document. Across devices nothing but the archive could do that.
        let digest = Sha256::digest(&file.bytes).to_vec();
        {
            let db = self.db()?;
            db.execute(
                "UPDATE archive_moves SET digest = ?3 WHERE doc_id = ?1 AND op = ?2",
                params![claim.doc_id, claim.op, hex::encode(&digest)],
            )?;
            // a twin on its way, or one whose upload may have arrived
            let twin: Option<String> = db
                .query_row(
                    "SELECT state FROM archive_moves
                      WHERE digest = ?1 AND doc_id != ?2 AND archive_id = ?3
                        AND (state IN ('posting', 'uploaded')
                             OR (state IN ('failed', 'kept') AND task = ?4))
                      ORDER BY state IN ('failed', 'kept') LIMIT 1",
                    params![
                        hex::encode(&digest),
                        claim.doc_id,
                        claim.archive_id,
                        MAYBE_THERE
                    ],
                    |r| r.get(0),
                )
                .optional()?;
            match twin.as_deref() {
                None => {}
                // The twin's upload may have arrived (also when the user
                // kept that file in the vault since): trying the twin again
                // looks the file up in the archive first, and settles it.
                Some("failed" | "kept") => {
                    claim.fail(&db, "posting", tr(TWIN_UNCLEAR))?;
                    return Ok(Step::Failed);
                }
                Some(_) => {
                    claim.step(&db, "posting", "queued", "", "")?;
                    return Ok(Step::Waiting);
                }
            }
        }
        // If the archive has this very file already, that document is the
        // one: nothing is sent.
        match archive.find_file(&digest).await {
            Ok(Some(archive_doc)) => {
                claim.posted(&*self.db()?, &format!("{DOC}{archive_doc}"))?;
                return Ok(Step::Waiting);
            }
            Ok(None) => {}
            // no connection: nothing was sent, the file simply waits on
            Err(failure) => {
                claim.step(&*self.db()?, "posting", "queued", "", "")?;
                return Err(failure.into());
            }
        }
        // Where the archive files it: looked up (or made) before the file
        // goes, so that the look below is the last thing before it.
        let filed = match archive.file_under(&labels).await {
            Ok(filed) => filed,
            // nothing of the file was sent: it simply waits on
            Err(failure) => {
                claim.step(&*self.db()?, "posting", "queued", "", "")?;
                return Err(failure.into());
            }
        };
        {
            // the questions took a while: still this move, still wanted?
            let db = self.db()?;
            if !claim.holds(&db, "posting")? {
                return Ok(Step::Waiting);
            }
            db.execute(
                "UPDATE archive_moves SET claimed_ms = ?3 WHERE doc_id = ?1 AND op = ?2",
                params![claim.doc_id, claim.op, now_ms()],
            )?;
        }
        let posted = archive
            .upload(Upload {
                title,
                created: Some(file.doc_date).filter(|d| !d.is_empty()),
                file_name: file.file_name,
                mime: file.mime,
                bytes: file.bytes,
                filed,
            })
            .await;
        let db = self.db()?;
        match posted {
            Ok(Uploaded::Processing(handle)) => {
                claim.posted(&db, &format!("{TASK}{handle}"))?;
                Ok(Step::Waiting)
            }
            // taken, but there is nothing to ask about it with
            Ok(Uploaded::Unknown) => {
                claim.fail_unclear(&db, "posting")?;
                Ok(Step::Failed)
            }
            // nothing was sent: the file simply waits on
            Err(Failure::NotSent) => {
                claim.step(&db, "posting", "queued", "", "")?;
                Err(Failure::NotSent.into())
            }
            Err(Failure::Unclear) | Err(Failure::TooLarge) => {
                claim.fail_unclear(&db, "posting")?;
                Ok(Step::Failed)
            }
            Err(Failure::Status(status)) => {
                let code = status.as_u16();
                // Something on the way gave up (a proxy's 504 may follow an
                // upload the archive did take): as unclear as no answer.
                if code >= 500 || code == 408 {
                    claim.fail_unclear(&db, "posting")?;
                    return Ok(Step::Failed);
                }
                // about the connection, not about this file: it waits on
                if matches!(code, 401 | 403 | 404 | 406 | 429) || (300..400).contains(&code) {
                    claim.step(&db, "posting", "queued", "", "")?;
                    return Err(Failure::Status(status).into());
                }
                // refused: not a file it takes, too large for it, …
                Ok(if claim.refused(&db)? {
                    Step::Failed
                } else {
                    Step::Waiting
                })
            }
        }
    }

    /// Asks about the task of one uploaded file and finishes the move when
    /// the document is there and readable.
    async fn check_move(
        &self,
        archive: &dyn Backend,
        connected: &Connected,
        claim: &Claim,
    ) -> CoreResult<Step> {
        let gone =
            |f: &Failure| matches!(f, Failure::Status(s) if matches!(s.as_u16(), 401 | 403 | 404));
        // a file the archive had already has no task: the document is known
        let known = claim
            .task
            .strip_prefix(DOC)
            .and_then(|id| id.parse::<i64>().ok());
        let archive_doc = if let Some(id) = known {
            id
        } else {
            let handle = claim.task.strip_prefix(TASK).unwrap_or(&claim.task);
            match archive.upload_state(handle).await? {
                TaskState::Working => {
                    // asked just now: the others come first in the next run
                    self.db()?.execute(
                        "UPDATE archive_moves SET claimed_ms = ?3
                      WHERE doc_id = ?1 AND op = ?2 AND state = 'uploaded'",
                        params![claim.doc_id, claim.op, now_ms()],
                    )?;
                    return Ok(Step::Waiting);
                }
                TaskState::Document(id) => id,
                TaskState::Failed => {
                    return self.fail(
                        claim,
                        tr("Das Dokumentenarchiv konnte die Datei nicht verarbeiten."),
                    )
                }
                TaskState::InTrash => return self.fail(claim, tr(IN_TRASH)),
                TaskState::Unknown => {
                    let given_up = claim.tried(&*self.db()?, "uploaded", MAX_UNKNOWN)?;
                    return Ok(
                        if given_up && claim.fail_unclear(&*self.db()?, "uploaded")? {
                            Step::Failed
                        } else {
                            Step::Waiting
                        },
                    );
                }
            }
        };
        // The document must be one this token can read, now (not in the
        // trash, not somebody else's), and the archive must hand back the
        // very file. Only then does it leave the vault.
        match archive.document_exists(archive_doc).await {
            Ok(true) => {}
            Ok(false) => return self.fail(claim, tr(IN_TRASH)),
            Err(f) if gone(&f) => return self.fail(claim, tr(IN_TRASH)),
            Err(f) => return Err(f.into()),
        }
        let theirs = match archive.original(archive_doc).await {
            Ok(bytes) => Sha256::digest(&bytes).to_vec(),
            Err(f) if gone(&f) || matches!(f, Failure::TooLarge) => {
                return self.fail(claim, tr(NOT_THE_SAME))
            }
            Err(f) => return Err(f.into()),
        };
        let ours = {
            let db = self.db()?;
            if !claim.holds(&db, "uploaded")? {
                return Ok(Step::Waiting);
            }
            stored(&db, claim.doc_id)?
        };
        match ours {
            // deleted meanwhile; its line went with it
            None => return Ok(Step::Waiting),
            Some(file) if Sha256::digest(&file.bytes).to_vec() != theirs => {
                return self.fail(claim, tr(NOT_THE_SAME))
            }
            Some(_) => {}
        }
        let mut db = self.db()?;
        Ok(if finish(&mut db, claim, &connected.base, archive_doc)? {
            Step::Moved
        } else {
            Step::Waiting
        })
    }

    fn fail(&self, claim: &Claim, problem: &str) -> CoreResult<Step> {
        Ok(if claim.fail(&*self.db()?, "uploaded", problem)? {
            Step::Failed
        } else {
            Step::Waiting
        })
    }
}
