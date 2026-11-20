//! Booking data on the archive's documents: a linked document carries the
//! date, amount, counterparty and account of its booking as custom fields, so
//! receipts can be found in the archive by what was paid ("everything booked
//! in March", "the receipt for 238,00").
//!
//! Off unless the user switches it on: it writes into the archive, and it
//! puts booking data there.
//!
//! There is no queue of changes. What a document should carry follows from
//! the references in the vault as they are now ([`Facts`]); what this device
//! last wrote is remembered per document in `archive_facts`. A run writes
//! the documents where the two differ. So linking, moving a reference to
//! another booking, removing it, a merge from another device and a folded
//! twin need no hook of their own, and a run that is interrupted loses
//! nothing: the difference is still there next time.
//!
//! - A document on several bookings carries the earliest date, the sum of
//!   the amounts (one currency; otherwise none) and all counterparties and
//!   accounts.
//! - The fields are the app's own: their names carry its name, so they are
//!   never fields the user keeps by hand, and they are fixed in every
//!   language of the app, because they are how two devices, and a later
//!   run, find the same fields.
//! - Only these four fields are touched, by id, with the archive's
//!   operation for exactly that (`modify_custom_fields`): the document is
//!   never read and written back, so nothing someone else changes meanwhile
//!   is lost.
//! - A field is only taken off a document that carries it from this device
//!   (or may: a write whose answer never came). They are all taken off when
//!   the last reference is gone.
//! - Before a request goes out the document is marked as "may carry
//!   something"; the answer replaces the mark. So a lost answer, a crash or
//!   a run that is ended leaves a difference the next run settles. Writing
//!   the same values twice changes nothing.
//! - One run at a time, and before every write a look whether the switch is
//!   still on and the archive still the connected one.
//!
//! What it cannot see: values changed by hand in the archive. They stay
//! until the booking data of the document changes.
//!
//! The table is local to the device, like `archive_moves`.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use rusqlite::{params, Connection};
use serde_json::json;

use super::backend::FactKind;
use super::client::Failure;
use super::{connection, current, setting, store, valid_date};
use crate::error::{CoreError, CoreResult};
use crate::i18n::tr;
use crate::merchants::MerchantResolver;
use crate::vault::Vault;

/// Step 35: what this device wrote onto documents of an archive.
pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS archive_facts (
    archive_id  TEXT    NOT NULL,
    archive_doc INTEGER NOT NULL,
    written     TEXT    NOT NULL DEFAULT '', -- the booking data the document carries from here ('' none)
    tried       TEXT    NOT NULL DEFAULT '', -- the booking data the archive last refused
    attempts    INTEGER NOT NULL DEFAULT 0,  -- how often it refused that
    problem     TEXT    NOT NULL DEFAULT '', -- why, worded for the user
    PRIMARY KEY (archive_id, archive_doc)
) WITHOUT ROWID;
";

/// Setting: booking data is written onto linked documents.
pub(crate) const KEY_FACTS: &str = "archive.facts";
/// Setting: the archive and the ids of the four fields the documents were
/// last written with (`<archive id>:<id>,<id>,<id>,<id>`).
pub(crate) const KEY_FIELDS: &str = "archive.facts.fields";

/// Documents written per run (the caller runs again while some wait).
const WRITES_PER_RUN: usize = 20;
/// After this many refusals of the same data a document is given up on.
const MAX_REFUSALS: i64 = 3;
/// Longest text the archive takes in a text field.
pub(crate) const MAX_TEXT: usize = 128;

/// The four fields, in the order of [`Facts::values`]: name in the archive
/// and its data type there.
pub(crate) const FIELDS: [(&str, FactKind); 4] = [
    ("Buchungsdatum (GiroCrab)", FactKind::Date),
    ("Buchungsbetrag (GiroCrab)", FactKind::Money),
    ("Gegenpartei (GiroCrab)", FactKind::Text),
    ("Buchungskonto (GiroCrab)", FactKind::Text),
];
/// `archive_facts.written` while a write is under way or its outcome is
/// not known: the document may carry anything from here.
const UNSURE: &str = "?";

/// One run at a time, whoever asks.
static ONE_RUN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// What one run of `process_archive_facts` did and what is left.
#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ArchiveFactsReport {
    /// Documents written in this run.
    pub written: u32,
    /// Documents the archive refused in this run (they are tried again a
    /// few times; with `written` this says whether the run got anywhere).
    pub refused: u32,
    /// Documents that still differ from the vault: run again.
    pub waiting: u32,
    /// Documents the archive keeps refusing.
    pub failed: u32,
    /// Why, for one of them (empty when none failed).
    pub problem: String,
}

/// The booking data one document should carry. An empty value means the
/// field is not set.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Facts {
    pub(crate) date: String,
    pub(crate) amount: String,
    pub(crate) party: String,
    pub(crate) account: String,
}

/// One booking (or account) a document hangs on.
#[derive(Debug, Clone, Default)]
pub(crate) struct Source {
    pub(crate) date: Option<String>,
    pub(crate) cents: Option<i64>,
    pub(crate) currency: Option<String>,
    pub(crate) party: Option<String>,
    pub(crate) account: Option<String>,
}

/// `text` cut to what a text field takes.
fn fitted(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= MAX_TEXT {
        return text.to_string();
    }
    let mut out: String = text.chars().take(MAX_TEXT - 1).collect();
    out.push('…');
    out
}

/// The distinct, non-empty values in their order, as one text.
fn listed<'a>(values: impl Iterator<Item = &'a str>) -> String {
    let mut seen: Vec<&str> = Vec::new();
    for v in values.map(str::trim).filter(|v| !v.is_empty()) {
        if !seen.contains(&v) {
            seen.push(v);
        }
    }
    fitted(&seen.join(", "))
}

/// An amount with its currency and the booking's sign, `EUR-238.00`: the
/// one form every kind of archive gets (`backend::FactKind::Money`).
fn money(cents: i64, currency: &str) -> String {
    if currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_uppercase()) {
        return String::new();
    }
    let abs = cents.unsigned_abs();
    // twelve digits in all is what the archive stores
    if abs >= 1_000_000_000_000 {
        return String::new();
    }
    let sign = if cents < 0 { "-" } else { "" };
    format!("{currency}{sign}{}.{:02}", abs / 100, abs % 100)
}

impl Facts {
    /// From everything the document hangs on, earliest booking first.
    pub(crate) fn of(sources: &[Source]) -> Self {
        let date = sources
            .iter()
            .filter_map(|s| s.date.as_deref())
            .filter(|d| valid_date(d))
            .min()
            .unwrap_or_default()
            .to_string();
        let booked: Vec<(i64, &str)> = sources
            .iter()
            .filter_map(|s| Some((s.cents?, s.currency.as_deref()?)))
            .collect();
        let amount = match booked.first() {
            Some((_, currency)) if booked.iter().all(|(_, c)| c == currency) => booked
                .iter()
                .try_fold(0i64, |sum, (cents, _)| sum.checked_add(*cents))
                .map(|sum| money(sum, currency))
                .unwrap_or_default(),
            _ => String::new(),
        };
        Self {
            date,
            amount,
            party: listed(sources.iter().filter_map(|s| s.party.as_deref())),
            account: listed(sources.iter().filter_map(|s| s.account.as_deref())),
        }
    }

    /// In the order of [`FIELDS`].
    pub(crate) fn values(&self) -> [&str; 4] {
        [&self.date, &self.amount, &self.party, &self.account]
    }

    /// One text for the whole state, to compare with what was written
    /// (empty: the document carries nothing).
    pub(crate) fn key(&self) -> String {
        if self.values().iter().all(|v| v.is_empty()) {
            return String::new();
        }
        json!(self.values()).to_string()
    }
}

fn enabled(conn: &Connection) -> CoreResult<bool> {
    Ok(setting(conn, KEY_FACTS)?.is_some())
}

/// What every document of the archive should carry, from the references of
/// the vault as they are.
fn wanted(conn: &Connection, archive_id: &str) -> CoreResult<BTreeMap<i64, Facts>> {
    let mut sources: BTreeMap<i64, Vec<Source>> = BTreeMap::new();
    let mut stmt = conn.prepare(
        // the account of a reference only counts when it hangs on no booking
        "SELECT l.archive_doc, t.booking_date, t.amount_cents, t.currency, t.counterparty,
                t.purpose, CASE WHEN l.booking_id IS NULL THEN la.name ELSE ta.name END
           FROM document_links l
           LEFT JOIN transactions t ON t.id = l.booking_id AND t.deleted_ms IS NULL
           LEFT JOIN accounts ta ON ta.id = t.account_id
           LEFT JOIN accounts la ON la.id = l.account_id
          WHERE l.archive_id = ?1 AND l.archive_doc IS NOT NULL AND l.deleted_ms IS NULL
          ORDER BY l.archive_doc, t.booking_date, t.id",
    )?;
    let mut rows = stmt.query([archive_id])?;
    while let Some(r) = rows.next()? {
        let counterparty: Option<String> = r.get(4)?;
        let purpose: Option<String> = r.get(5)?;
        // the name the booking list shows
        let party = counterparty
            .map(|c| MerchantResolver::resolve(&c, purpose.as_deref().unwrap_or_default()).name);
        sources.entry(r.get(0)?).or_default().push(Source {
            date: r.get(1)?,
            cents: r.get(2)?,
            currency: r.get(3)?,
            party,
            account: r.get(6)?,
        });
    }
    Ok(sources
        .into_iter()
        .map(|(doc, s)| (doc, Facts::of(&s)))
        .collect())
}

/// Where the documents of an archive stand.
#[derive(Default)]
struct Standing {
    /// To write now: document, the data it should carry, and what this
    /// device wrote onto it before.
    work: Vec<(i64, Facts, String)>,
    /// All that differ and are not given up on.
    waiting: u32,
    failed: u32,
    problem: String,
}

fn standing(conn: &Connection, archive_id: &str) -> CoreResult<Standing> {
    let mut wanted = wanted(conn, archive_id)?;
    let mut known: HashMap<i64, (String, String, i64, String)> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT archive_doc, written, tried, attempts, problem FROM archive_facts
          WHERE archive_id = ?1",
    )?;
    let mut rows = stmt.query([archive_id])?;
    while let Some(r) = rows.next()? {
        known.insert(r.get(0)?, (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?));
    }
    // documents written before whose last reference is gone: to be cleared
    for doc in known.keys() {
        wanted.entry(*doc).or_default();
    }
    let mut out = Standing::default();
    for (doc, facts) in wanted {
        let key = facts.key();
        let (written, tried, attempts, problem) = known.remove(&doc).unwrap_or_default();
        if key == written {
            continue;
        }
        if tried == key && attempts >= MAX_REFUSALS {
            out.failed += 1;
            if out.problem.is_empty() {
                out.problem = problem;
            }
            continue;
        }
        out.waiting += 1;
        if out.work.len() < WRITES_PER_RUN {
            out.work.push((doc, facts, written));
        }
    }
    Ok(out)
}

/// Which of the four fields a document may carry from this device, going
/// by what was written onto it (`archive_facts.written`).
pub(crate) fn carried(written: &str) -> [bool; 4] {
    if written.is_empty() {
        return [false; 4];
    }
    match serde_json::from_str::<[String; 4]>(written) {
        Ok(values) => values.map(|v| !v.is_empty()),
        // a write with an unknown outcome
        Err(_) => [true; 4],
    }
}

/// A request for `doc` is about to go out: until its answer is stored, the
/// document may carry anything from here.
fn sending(conn: &Connection, archive_id: &str, doc: i64) -> CoreResult<()> {
    conn.execute(
        "INSERT INTO archive_facts(archive_id, archive_doc, written) VALUES (?1, ?2, ?3)
         ON CONFLICT(archive_id, archive_doc) DO UPDATE SET written = excluded.written",
        params![archive_id, doc, UNSURE],
    )?;
    Ok(())
}

/// Whether the switch is still on and `archive_id` still the connected
/// archive: asked before every write.
fn still_wanted(conn: &Connection, archive_id: &str) -> CoreResult<bool> {
    Ok(enabled(conn)? && current(conn)?.is_some_and(|(now, _)| now == archive_id))
}

fn wrote(conn: &Connection, archive_id: &str, doc: i64, key: &str) -> CoreResult<()> {
    if key.is_empty() {
        conn.execute(
            "DELETE FROM archive_facts WHERE archive_id = ?1 AND archive_doc = ?2",
            params![archive_id, doc],
        )?;
    } else {
        conn.execute(
            "INSERT INTO archive_facts(archive_id, archive_doc, written) VALUES (?1, ?2, ?3)
             ON CONFLICT(archive_id, archive_doc) DO UPDATE
                SET written = excluded.written, tried = '', attempts = 0, problem = ''",
            params![archive_id, doc, key],
        )?;
    }
    Ok(())
}

/// Notes a refusal. `true`: that was the last try for this data.
fn refused(
    conn: &Connection,
    archive_id: &str,
    doc: i64,
    key: &str,
    problem: &str,
) -> CoreResult<bool> {
    conn.execute(
        "INSERT INTO archive_facts(archive_id, archive_doc, tried, attempts, problem)
         VALUES (?1, ?2, ?3, 1, ?4)
         ON CONFLICT(archive_id, archive_doc) DO UPDATE
            SET attempts = CASE WHEN tried = excluded.tried THEN attempts + 1 ELSE 1 END,
                tried = excluded.tried, problem = excluded.problem",
        params![archive_id, doc, key, problem],
    )?;
    let attempts: i64 = conn.query_row(
        "SELECT attempts FROM archive_facts WHERE archive_id = ?1 AND archive_doc = ?2",
        params![archive_id, doc],
        |r| r.get(0),
    )?;
    Ok(attempts >= MAX_REFUSALS)
}

/// Forgets what the archive refused: the next run tries everything again
/// (asked for by the user, and after the connection was stored anew).
pub(crate) fn forget_refusals(conn: &Connection) -> CoreResult<()> {
    conn.execute(
        "UPDATE archive_facts SET tried = '', attempts = 0, problem = '' WHERE attempts > 0",
        [],
    )?;
    conn.execute(
        "DELETE FROM archive_facts WHERE written = '' AND attempts = 0",
        [],
    )?;
    Ok(())
}

/// Remembers the fields the documents of the archive are written with. When
/// they are other fields than before (somebody deleted one in the archive
/// and it was made anew), what a document carries is no longer known:
/// every document written so far is to be written again.
/// `true`: they are other fields than before.
pub(crate) fn fields_are(
    conn: &Connection,
    archive_id: &str,
    ids: &[String; 4],
) -> CoreResult<bool> {
    let now = format!("{archive_id}:{}", ids.join(","));
    let before = setting(conn, KEY_FIELDS)?;
    if before.as_deref() == Some(now.as_str()) {
        return Ok(false);
    }
    let others = before.is_some_and(|b| b.starts_with(&format!("{archive_id}:")));
    if others {
        // Not "nothing": the fields that are still the same carry what was
        // written, and a clearing the archive keeps refusing stays one.
        conn.execute(
            "UPDATE archive_facts SET written = ?2 WHERE archive_id = ?1 AND written != ''",
            params![archive_id, UNSURE],
        )?;
    }
    store(conn, KEY_FIELDS, &now, false)?;
    Ok(others)
}

/// Whether a refusal is about this one document (it is gone, or this token
/// may not change it). Anything else is about the archive as a whole and
/// ends the run.
fn about_the_document(failure: &Failure) -> bool {
    match failure {
        Failure::Status(status) => {
            status.is_client_error() && !matches!(status.as_u16(), 401 | 406 | 408 | 429)
        }
        _ => false,
    }
}

fn message(e: CoreError) -> String {
    match e {
        CoreError::Network { message }
        | CoreError::Authentication { message }
        | CoreError::Protocol { message }
        | CoreError::InvalidInput { message } => message,
        _ => tr("Das Dokumentenarchiv hat die Buchungsdaten nicht angenommen.").into(),
    }
}

#[uniffi::export]
impl Vault {
    /// Whether booking data is written onto linked documents of the
    /// connected archive (off without a connection).
    pub fn archive_facts_enabled(&self) -> Result<bool, CoreError> {
        let db = self.db()?;
        Ok(current(&db)?.is_some() && enabled(&db)?)
    }

    /// Switches it on or off. Sends nothing: `process_archive_facts` does.
    /// What documents carry already stays when it is switched off.
    pub fn set_archive_facts(&self, enabled: bool) -> Result<(), CoreError> {
        let db = self.db()?;
        if enabled {
            store(&db, KEY_FACTS, "1", false)?;
            forget_refusals(&db)
        } else {
            db.execute("DELETE FROM settings WHERE key = ?1", [KEY_FACTS])?;
            Ok(())
        }
    }

    /// Has the next run try again what the archive refused.
    pub fn retry_archive_facts(&self) -> Result<(), CoreError> {
        forget_refusals(&*self.db()?)
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl Vault {
    /// Brings the documents of the connected archive in line with the
    /// bookings they hang on: a handful per call, so call it again while the
    /// report says some are `waiting`. Sends nothing when it is switched off,
    /// when no archive is connected (an error then) or when nothing differs.
    /// Safe to call at any time and as often as wanted; the vault lock is
    /// never held during a request.
    pub async fn process_archive_facts(self: Arc<Self>) -> Result<ArchiveFactsReport, CoreError> {
        // taken before the wait for a run under way: a lock of the vault or
        // a change of the connection meanwhile ends this one too
        let watch = self.archive_watch();
        let _one_run = ONE_RUN.lock().await;
        if self.archive_watch().1 != watch.1 {
            return Ok(ArchiveFactsReport::default());
        }
        let (connected, now) = {
            let db = self.db()?;
            if !enabled(&db)? {
                return Ok(ArchiveFactsReport::default());
            }
            let connected = connection(&db)?;
            let now = standing(&db, &connected.id)?;
            (connected, now)
        };
        let mut report = ArchiveFactsReport {
            waiting: now.waiting,
            failed: now.failed,
            problem: now.problem,
            ..Default::default()
        };
        if now.work.is_empty() {
            return Ok(report);
        }
        let archive = connected.backend(watch).await?;
        let wanted = || still_wanted(&*self.db()?, &connected.id);
        // switched off while the fields were looked up or made: nothing more
        let Some(ids) = archive.fact_fields(&wanted).await? else {
            return Ok(report);
        };
        let other_fields = fields_are(&*self.db()?, &connected.id, &ids)?;
        for (doc, facts, written) in &now.work {
            let key = facts.key();
            {
                let db = self.db()?;
                // switched off, or another archive, since the run began:
                // not one more write
                if !still_wanted(&db, &connected.id)? {
                    break;
                }
                sending(&db, &connected.id, *doc)?;
            }
            match archive
                .write_facts(&ids, *doc, facts, carried(written))
                .await
            {
                Ok(()) => {
                    wrote(&*self.db()?, &connected.id, *doc, &key)?;
                    report.written += 1;
                    report.waiting = report.waiting.saturating_sub(1);
                }
                Err(failure) if about_the_document(&failure) => {
                    // Nothing can be cleared on a document that is gone.
                    // Any other refusal the user should see, also of a
                    // clearing: the data would stay in the archive.
                    let gone =
                        key.is_empty() && matches!(archive.document_exists(*doc).await, Ok(false));
                    let db = self.db()?;
                    if gone {
                        wrote(&db, &connected.id, *doc, "")?;
                        report.waiting = report.waiting.saturating_sub(1);
                        continue;
                    }
                    let problem = message(failure.into());
                    report.refused += 1;
                    if refused(&db, &connected.id, *doc, &key, &problem)? {
                        report.failed += 1;
                        report.waiting = report.waiting.saturating_sub(1);
                        if report.problem.is_empty() {
                            report.problem = problem;
                        }
                    }
                }
                // the archive is not there right now (or the run was ended):
                // what was written counts, the rest still differs next time
                Err(failure) => {
                    if report.written + report.refused == 0 {
                        return Err(failure.into());
                    }
                    break;
                }
            }
        }
        if other_fields {
            // every document is to be written again: count anew
            let left = standing(&*self.db()?, &connected.id)?;
            report.waiting = left.waiting;
            report.failed = left.failed;
        }
        Ok(report)
    }
}
