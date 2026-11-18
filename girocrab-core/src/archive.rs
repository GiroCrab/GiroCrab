//! Document archive: receipts that live in the user's own, self-hosted
//! archive (a Paperless-ngx compatible REST API) instead of the vault.
//!
//! Builds on the linked receipts of `document_links.rs` (#590): a reference
//! that names a document of the connected archive (`archive_doc`) can be
//! shown, searched and suggested from inside the app.
//!
//! Strictly opt-in: without an address and a token nothing here sends a
//! request. This is the only network destination besides the banks, and it is
//! one the user runs and enters themselves.
//!
//! Address and token are settings of this device (the `settings` table is
//! not part of the vault merge); the references travel between devices, and
//! each device reaches the archive under its own address. A document id only
//! means something inside one archive, so every reference also carries the id
//! of the archive it belongs to (`archive_id`, a random id made when an
//! archive is first connected). A device shows, fetches and suggests only
//! documents of the archive it is connected to; a reference into another
//! archive (after a change of archive here or on another device) is a plain
//! link to the address it was made with.
//!
//! Network calls never run under the vault lock: the configuration is read,
//! the lock released, then the request sent (`client.rs` has the rules for
//! the connection itself).

use std::sync::Arc;

use rusqlite::{params, Connection, OptionalExtension};
use url::Url;
use zeroize::Zeroizing;

use crate::documents::{DocumentInfo, ReceiptFacts, MATCH_SCORE};
use crate::error::{CoreError, CoreResult};
use crate::i18n::{tr, trf};
use crate::vault::Vault;

pub(crate) mod backend;
pub(crate) mod client;
pub(crate) mod moves;
pub(crate) mod paperless;
#[cfg(test)]
mod tests;

use backend::{Backend, Kind};

/// Step 33: a linked receipt can name a document of an archive
/// (`document_links.archive_doc`: its id there, NULL = plain link;
/// `archive_id`: which archive), and keeps what it was as a stored file a
/// bank delivered (`document_links::origin_columns`). Idempotent like the
/// steps before it.
pub(crate) fn archive_columns(conn: &Connection) -> CoreResult<()> {
    crate::document_links::origin_columns(conn)?;
    for (column, definition) in [
        ("archive_doc", "INTEGER"),
        ("archive_id", "TEXT NOT NULL DEFAULT ''"),
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
        "CREATE INDEX IF NOT EXISTS document_links_archive ON document_links(archive_id, archive_doc)
             WHERE archive_doc IS NOT NULL AND deleted_ms IS NULL;",
    )?;
    Ok(())
}

const KEY_URL: &str = "archive.url";
const KEY_TOKEN: &str = "archive.token";
const KEY_ID: &str = "archive.id";
/// Which kind of archive the connection is (`backend::Kind`).
const KEY_KIND: &str = "archive.kind";

/// Largest thumbnail taken from the archive.
/// Most documents one search or one round of suggestions looks at.
const MAX_PAGE: u32 = 50;
/// A receipt is usually dated before its booking, rarely after.
const SUGGEST_DAYS_BEFORE: i64 = 45;
const SUGGEST_DAYS_AFTER: i64 = 10;
/// One pass of `archive_matches` looks at this many bookings and pages.
const MATCH_BOOKINGS: usize = 2000;
const MATCH_PAGES: u32 = 20;
const MATCH_LIMIT: u32 = 300;

fn invalid(msg: &str) -> CoreError {
    CoreError::InvalidInput {
        message: msg.to_string(),
    }
}

fn not_connected() -> CoreError {
    CoreError::MissingSetting {
        key: KEY_URL.to_string(),
    }
}

fn unexpected() -> CoreError {
    CoreError::Protocol {
        message: tr("Das Dokumentenarchiv hat unerwartet geantwortet.").into(),
    }
}

// ------------------------------------------------------------------ address

/// Checks the address of an archive and returns it normalised: `http(s)`,
/// a host, no credentials, no query or fragment, no trailing slash (a path
/// is kept: archives can run under a sub-path). Plain `http` to an IP
/// address is only accepted for the user's own network; for a host name the
/// connection itself checks where it leads (`client.rs`).
pub(crate) fn parse_base(input: &str) -> CoreResult<Url> {
    let s = input.trim();
    if s.is_empty() {
        return Err(invalid(tr("Die Adresse des Dokumentenarchivs fehlt.")));
    }
    let bad = || invalid(tr("Die Adresse des Dokumentenarchivs ist ungültig."));
    let mut url = Url::parse(s).map_err(|_| bad())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(invalid(tr(
            "Die Adresse des Dokumentenarchivs muss mit http:// oder https:// beginnen.",
        )));
    }
    let private = match url.host() {
        None => return Err(bad()),
        Some(url::Host::Domain("")) => return Err(bad()),
        Some(url::Host::Domain(_)) => true,
        Some(url::Host::Ipv4(ip)) => client::is_private_ip(ip.into()),
        Some(url::Host::Ipv6(ip)) => client::is_private_ip(ip.into()),
    };
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid(tr(
            "Die Adresse darf keine Zugangsdaten enthalten. Der Token wird getrennt eingetragen.",
        )));
    }
    if url.scheme() == "http" && !private {
        return Err(invalid(tr("Unverschlüsselt (http) verbindet sich GiroCrab nur mit Adressen im eigenen Netz. Bitte https verwenden.")));
    }
    url.set_query(None);
    url.set_fragment(None);
    let path = url.path().trim_end_matches('/').to_string();
    url.set_path(&path);
    Ok(url)
}

fn base_str(base: &Url) -> &str {
    base.as_str().trim_end_matches('/')
}

// ------------------------------------------------------------------ configuration

/// The connected archive as the settings UI shows it. The token never
/// leaves the vault.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ArchiveConfig {
    /// Normalised address, without a trailing slash.
    pub url: String,
    pub has_token: bool,
    /// The address is plain `http`: token and documents cross the network
    /// unencrypted.
    pub unencrypted: bool,
    /// Live references that name a document of this archive.
    pub linked_documents: u32,
    /// What kind of archive it is: the name of its software ("Paperless-ngx").
    #[uniffi(default = "")]
    #[cfg_attr(feature = "serde", serde(default))]
    pub kind: String,
}

/// The kinds of document archive this version can connect to, by the name
/// of their software, for the settings.
pub fn archive_kinds() -> Vec<String> {
    Kind::ALL.iter().map(|k| k.name().to_string()).collect()
}

pub(crate) fn setting(conn: &Connection, key: &str) -> CoreResult<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
            r.get::<_, String>(0)
        })
        .optional()?
        .filter(|v| !v.is_empty()))
}

/// The stored address, if an archive is connected and its address still
/// passes the check.
pub(crate) fn base_of(conn: &Connection) -> CoreResult<Option<Url>> {
    Ok(setting(conn, KEY_URL)?.and_then(|u| parse_base(&u).ok()))
}

/// The archive this device is connected to: its id and address.
pub(crate) fn current(conn: &Connection) -> CoreResult<Option<(String, Url)>> {
    let Some(base) = base_of(conn)? else {
        return Ok(None);
    };
    Ok(setting(conn, KEY_ID)?.map(|id| (id, base)))
}

/// What kind of archive the connection is. A connection without the note
/// is the first kind there was. A kind this version does not know (the
/// vault was connected by a newer one) cannot be talked to.
pub(crate) fn kind_of(conn: &Connection) -> CoreResult<Kind> {
    match setting(conn, KEY_KIND)? {
        None => Ok(Kind::DEFAULT),
        Some(id) => Kind::of(&id).ok_or_else(|| CoreError::InvalidInput {
            message: tr("Diese Art von Dokumentenarchiv kennt diese Version von GiroCrab nicht.")
                .into(),
        }),
    }
}

/// Everything a request needs, read in one go.
pub(crate) struct Connected {
    pub(crate) id: String,
    pub(crate) kind: Kind,
    pub(crate) base: Url,
    pub(crate) token: Zeroizing<String>,
}

impl Connected {
    /// The archive itself, to talk to. `watch` from `Vault::archive_watch`,
    /// taken before this connection was read.
    pub(crate) async fn backend(&self, watch: client::Watch) -> CoreResult<Box<dyn Backend>> {
        self.kind
            .connect(&self.base, &self.token, &self.id, watch)
            .await
    }
}

/// Id, kind, address and token for a request; an error unless all are set.
pub(crate) fn connection(conn: &Connection) -> CoreResult<Connected> {
    let (id, base) = current(conn)?.ok_or_else(not_connected)?;
    let token = setting(conn, KEY_TOKEN)?.ok_or_else(not_connected)?;
    Ok(Connected {
        id,
        kind: kind_of(conn)?,
        base,
        token: Zeroizing::new(token),
    })
}

/// Archive id and document id a link points at, if an archive is connected
/// and the link is one of its addresses.
pub(crate) fn document_in_archive(
    conn: &Connection,
    link: &Url,
) -> CoreResult<Option<(String, i64)>> {
    let Some((id, base)) = current(conn)? else {
        return Ok(None);
    };
    let Ok(kind) = kind_of(conn) else {
        return Ok(None);
    };
    Ok(kind.document_id_in(&base, link).map(|doc| (id, doc)))
}

/// References of the connected archive open under this device's address of
/// it: the stored URL is the one of the device that made the link.
/// (`archive_doc` is only set for that archive, see `document_links`.)
pub(crate) fn localize(conn: &Connection, docs: &mut [DocumentInfo]) -> CoreResult<()> {
    let Some((_, base)) = current(conn)? else {
        return Ok(());
    };
    let Ok(kind) = kind_of(conn) else {
        return Ok(());
    };
    for d in docs.iter_mut() {
        if let Some(id) = d.archive_doc {
            d.link_url = kind.document_url(&base, id);
        }
    }
    Ok(())
}

/// Live references into the archive with id `archive_id`.
fn linked_count(conn: &Connection, archive_id: &str) -> CoreResult<u32> {
    Ok(conn.query_row(
        "SELECT count(*) FROM document_links
          WHERE archive_id = ?1 AND archive_doc IS NOT NULL AND deleted_ms IS NULL",
        [archive_id],
        |r| r.get(0),
    )?)
}

/// An archive the references of this vault belong to, for the question
/// "which archive is this address?".
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ArchiveIdentity {
    pub id: String,
    /// Live references that name a document of it.
    pub documents: u32,
    /// Host of the address one of them was made with (on whatever device),
    /// to tell archives apart; empty without a reference.
    pub host: String,
    /// The archive this device is connected to.
    pub connected: bool,
}

/// Every archive the vault knows: the connected one, and each one a live
/// reference belongs to (made on another device, or before a change of
/// archive here). The connected one first, then by number of references.
fn identities(conn: &Connection) -> CoreResult<Vec<ArchiveIdentity>> {
    let own = setting(conn, KEY_ID)?.unwrap_or_default();
    let mut out: Vec<ArchiveIdentity> = conn
        .prepare(
            "SELECT archive_id, count(*),
                    (SELECT url FROM document_links u
                      WHERE u.archive_id = l.archive_id AND u.deleted_ms IS NULL
                      ORDER BY u.id DESC LIMIT 1)
               FROM document_links l
              WHERE archive_doc IS NOT NULL AND archive_id != '' AND deleted_ms IS NULL
              GROUP BY archive_id ORDER BY count(*) DESC, archive_id",
        )?
        .query_map([], |r| {
            let id: String = r.get(0)?;
            let url: Option<String> = r.get(2)?;
            Ok(ArchiveIdentity {
                connected: false,
                documents: r.get(1)?,
                host: url
                    .and_then(|u| Url::parse(&u).ok())
                    .and_then(|u| u.host_str().map(str::to_string))
                    .unwrap_or_default(),
                id,
            })
        })?
        .collect::<Result<_, _>>()?;
    if !own.is_empty() {
        match out.iter().position(|a| a.id == own) {
            Some(i) => {
                let mut mine = out.remove(i);
                mine.connected = true;
                out.insert(0, mine);
            }
            None => out.insert(
                0,
                ArchiveIdentity {
                    id: own,
                    documents: 0,
                    host: String::new(),
                    connected: true,
                },
            ),
        }
    }
    Ok(out)
}

fn config_of(conn: &Connection) -> CoreResult<Option<ArchiveConfig>> {
    let Some((id, base)) = current(conn)? else {
        return Ok(None);
    };
    Ok(Some(ArchiveConfig {
        url: base_str(&base).to_string(),
        has_token: setting(conn, KEY_TOKEN)?.is_some(),
        unencrypted: base.scheme() == "http",
        linked_documents: linked_count(conn, &id)?,
        kind: kind_of(conn)
            .map(|k| k.name().to_string())
            .unwrap_or_default(),
    }))
}

pub(crate) fn store(conn: &Connection, key: &str, value: &str, secret: bool) -> CoreResult<()> {
    conn.execute(
        "INSERT INTO settings(key, value, is_secret) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, is_secret = excluded.is_secret",
        params![key, value, secret],
    )?;
    Ok(())
}

#[uniffi::export]
impl Vault {
    /// The connected document archive, `None` if there is none.
    pub fn archive_config(&self) -> Result<Option<ArchiveConfig>, CoreError> {
        config_of(&*self.db()?)
    }

    /// References in this vault that name a document of an archive other
    /// than the connected one (all of them without a connection): made on
    /// another device, or before the archive was changed.
    pub fn archive_documents_elsewhere(&self) -> Result<u32, CoreError> {
        let db = self.db()?;
        let own = setting(&db, KEY_ID)?.unwrap_or_default();
        Ok(db.query_row(
            "SELECT count(*) FROM document_links
              WHERE archive_doc IS NOT NULL AND archive_id != ?1 AND deleted_ms IS NULL",
            [own],
            |r| r.get(0),
        )?)
    }

    /// The archives this vault knows (see [`ArchiveIdentity`]). When a
    /// connection is made or its address changes and this list holds anything
    /// the user could mean, the app asks which one the address is.
    pub fn archive_identities(&self) -> Result<Vec<ArchiveIdentity>, CoreError> {
        identities(&*self.db()?)
    }

    /// Connects the document archive at `url` (or changes its address or
    /// token; an empty `token` keeps the stored one). Sends nothing: use
    /// `archive_test` to try the connection.
    ///
    /// `archive_id` says which archive the address is: the id of one the
    /// vault already knows (`archive_identities`: the one connected so far
    /// under a new address, or one another device made), or `None` for an
    /// archive the vault has no reference into yet, which gets an id of its
    /// own. References into other archives stay plain links; nothing is
    /// rewritten. A choice is never guessed: an id the vault does not know is
    /// refused.
    pub fn set_archive_connection(
        &self,
        url: String,
        token: String,
        archive_id: Option<String>,
    ) -> Result<ArchiveConfig, CoreError> {
        let base = parse_base(&url)?;
        let token = Zeroizing::new(token.trim().to_string());
        let mut db = self.db()?;
        if token.is_empty() && setting(&db, KEY_TOKEN)?.is_none() {
            return Err(invalid(tr("Der Token des Dokumentenarchivs fehlt.")));
        }
        let id = match archive_id {
            Some(id) => {
                if !identities(&db)?.iter().any(|a| a.id == id) {
                    return Err(unexpected());
                }
                id
            }
            None => uuid::Uuid::new_v4().to_string(),
        };
        // The stored token was entered for the stored address of this very
        // archive: it is sent nowhere else, whatever the new address is said
        // to be.
        let as_before = current(&db)?
            .is_some_and(|(before, at)| before == id && base_str(&at) == base_str(&base));
        if token.is_empty() && !as_before {
            return Err(invalid(tr(
                "Für eine neue Adresse oder ein anderes Archiv bitte den Token erneut eingeben.",
            )));
        }
        let tx = db.transaction()?;
        store(&tx, KEY_ID, &id, false)?;
        // the one kind there is; a second one comes with a choice here
        store(&tx, KEY_KIND, Kind::DEFAULT.id(), false)?;
        store(&tx, KEY_URL, base_str(&base), false)?;
        if !token.is_empty() {
            store(&tx, KEY_TOKEN, token.as_str(), true)?;
        }
        // files queued for another archive stay in the vault, off the queue
        tx.execute("DELETE FROM archive_moves WHERE archive_id != ?1", [&id])?;
        tx.commit()?;
        // whatever is under way was for the connection before
        self.archive_stop();
        config_of(&db)?.ok_or_else(not_connected)
    }

    /// Disconnects the archive: address, token and its id here are removed.
    /// The references stay as plain links that open in the browser; connecting
    /// the same archive again makes them its documents again.
    pub fn remove_archive_connection(&self) -> Result<(), CoreError> {
        let db = self.db()?;
        db.execute(
            "DELETE FROM settings WHERE key IN (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                KEY_URL,
                KEY_TOKEN,
                KEY_ID,
                KEY_KIND,
                moves::KEY_UPLOAD,
                moves::KEY_BANK,
                moves::KEY_LABELS
            ],
        )?;
        // files that were queued stay in the vault
        db.execute("DELETE FROM archive_moves", [])?;
        self.archive_stop();
        Ok(())
    }

    /// Ends all work for the archive that is under way: requests already
    /// sent may still come back, but no further one goes out and runs stop
    /// at their next step. The app calls it when it locks the vault or
    /// switches to another one; a change of the connection does it itself.
    pub fn archive_stop(&self) {
        use std::sync::atomic::Ordering::SeqCst;
        let mut now = self.archive_epoch.load(SeqCst);
        // (a closed vault stays closed)
        while now != client::CLOSED {
            match self
                .archive_epoch
                .compare_exchange(now, now + 1, SeqCst, SeqCst)
            {
                Ok(_) => break,
                Err(seen) => now = seen,
            }
        }
    }

    /// The app locks this vault: no archive request goes out for it any
    /// more, neither from work under way nor from a call that only starts
    /// now (a front end cannot take back a call it has already made).
    /// Unlocking opens the vault anew.
    pub fn archive_close(&self) {
        self.archive_epoch
            .store(client::CLOSED, std::sync::atomic::Ordering::SeqCst);
    }

    /// Links a document of the connected archive to a booking: `archive_id`
    /// and `archive_doc` as a search or suggestion returned them
    /// (`ArchiveDocument`), `title` as the archive shows it. Refused when the
    /// connection has changed since. Only the vault is written.
    pub fn attach_archive_document(
        &self,
        booking_id: i64,
        archive_id: String,
        archive_doc: i64,
        title: String,
    ) -> Result<i64, CoreError> {
        let db = self.db()?;
        let (id, base) = current(&db)?.ok_or_else(not_connected)?;
        if id != archive_id {
            return Err(invalid(tr(
                "Die Verbindung zum Dokumentenarchiv hat sich geändert. Bitte erneut suchen.",
            )));
        }
        if archive_doc <= 0 {
            return Err(unexpected());
        }
        let url = kind_of(&db)?.document_url(&base, archive_doc);
        let mut title = title.split_whitespace().collect::<Vec<_>>().join(" ");
        if title.is_empty() {
            title = trf("Dokument {0}", &[&archive_doc]);
        }
        crate::document_links::insert(&db, booking_id, &url, &title, Some((&id, archive_doc)))
    }

    /// Links the matches of `archive_matches` the user accepted. Each one is
    /// looked at again as it is written: a match whose booking has a receipt
    /// by now, or whose document hangs on a booking by now (another device,
    /// a file that moved meanwhile), or that belongs to an earlier
    /// connection, is left out. Returns how many were linked.
    pub fn attach_archive_matches(&self, matches: Vec<ArchiveMatch>) -> Result<u32, CoreError> {
        let mut db = self.db()?;
        let Some((id, base)) = current(&db)? else {
            return Ok(0);
        };
        let kind = kind_of(&db)?;
        let tx = db.transaction()?;
        let mut linked = 0;
        for m in matches {
            if m.document.archive_id != id || m.document.id <= 0 {
                continue;
            }
            let free: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM transactions t WHERE t.id = ?1 AND t.deleted_ms IS NULL)
                    AND NOT EXISTS (SELECT 1 FROM booking_documents d WHERE d.booking_id = ?1)
                    AND NOT EXISTS (SELECT 1 FROM document_links l
                                     WHERE l.booking_id = ?1 AND l.deleted_ms IS NULL)
                    AND NOT EXISTS (SELECT 1 FROM document_links l
                                     WHERE l.archive_id = ?2 AND l.archive_doc = ?3
                                       AND l.deleted_ms IS NULL AND l.booking_id IS NOT NULL)",
                params![m.booking_id, id, m.document.id],
                |r| r.get(0),
            )?;
            if !free {
                continue;
            }
            let mut title = m
                .document
                .title
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            if title.is_empty() {
                title = trf("Dokument {0}", &[&m.document.id]);
            }
            let url = kind.document_url(&base, m.document.id);
            if crate::document_links::insert(
                &tx,
                m.booking_id,
                &url,
                &title,
                Some((&id, m.document.id)),
            )
            .is_ok()
            {
                linked += 1;
            }
        }
        tx.commit()?;
        Ok(linked)
    }
}

// ------------------------------------------------------------------ reading

/// What a connection test found.
#[derive(Debug, Clone, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ArchiveInfo {
    /// Version of the archive software, as it reports it (may be empty).
    pub server_version: String,
    /// Highest API version the archive supports, as it reports it (may be
    /// empty). GiroCrab asks for version 9.
    pub api_version: String,
    /// Documents the token's user can see.
    pub document_count: u32,
}

/// A document of the archive, as a search result.
#[derive(Debug, Clone, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ArchiveDocument {
    pub id: i64,
    /// The archive connection this result came from; linking hands it back
    /// (`attach_archive_document`), so a result never lands in another archive.
    pub archive_id: String,
    pub title: String,
    /// `YYYY-MM-DD`
    pub created: String,
    pub correspondent: String,
    pub document_type: String,
    pub file_name: String,
    pub page_count: u32,
    /// A few words of the document's text around the search term, or its
    /// beginning.
    pub excerpt: String,
    /// Already linked to a booking in this vault.
    pub linked: bool,
    /// Its page in the archive's own web interface.
    #[uniffi(default = "")]
    #[cfg_attr(feature = "serde", serde(default))]
    pub url: String,
}

/// A document of the archive that fits a booking.
#[derive(Debug, Clone, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ArchiveSuggestion {
    pub document: ArchiveDocument,
    /// Amount 5, merchant 3, date up to 3; at least 5.
    pub score: i32,
}

/// A booking without a receipt and the document of the archive that fits it.
#[derive(Debug, Clone, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ArchiveMatch {
    pub booking_id: i64,
    /// The booking in a few words ("Elektro Weber · −238,00 EUR · 2026-11-07").
    pub booking: String,
    pub document: ArchiveDocument,
    /// Amount 5, merchant 3, date up to 3. The amount always fits.
    pub score: i32,
    /// The document names the booking's merchant. Without that the same
    /// amount on a date nearby may be a coincidence.
    pub merchant_fits: bool,
    /// Another booking fits this document, or another document this
    /// booking, just as well: one for the user to look at.
    pub ambiguous: bool,
}

/// What `archive_matches` found, and how far it looked.
#[derive(Debug, Clone, Default, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ArchiveMatches {
    /// Newest booking first; every booking and every document at most once.
    pub matches: Vec<ArchiveMatch>,
    /// Bookings without a receipt in the period.
    pub open_bookings: u32,
    /// Documents of the archive that were held against them.
    pub documents: u32,
    /// There were more bookings or documents than one pass looks at: a
    /// shorter period finds the rest.
    pub more: bool,
}

/// The file of an archive document.
#[derive(Debug, Clone, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ArchiveFile {
    pub bytes: Vec<u8>,
    pub mime_type: String,
    pub file_name: String,
}

fn valid_date(d: &str) -> bool {
    // (`parse_iso` reads the day at the start of a longer text too)
    d.len() == 10 && crate::dates::parse_iso(d).is_some()
}

impl Vault {
    /// The connected archive, to talk to, and its id. The vault lock is only
    /// held to read the connection, in one go.
    async fn archive_backend(&self) -> CoreResult<(Box<dyn Backend>, String)> {
        let watch = self.archive_watch();
        let c = connection(&*self.db()?)?;
        Ok((c.backend(watch).await?, c.id))
    }

    /// The counter that ends archive work, and where it stands: taken
    /// before the connection is read, handed to the client.
    pub(crate) fn archive_watch(&self) -> client::Watch {
        (
            self.archive_epoch.clone(),
            self.archive_epoch.load(std::sync::atomic::Ordering::SeqCst),
        )
    }

    /// Marks the documents that already hang on a booking of this vault.
    fn mark_linked(&self, archive_id: &str, docs: &mut [ArchiveDocument]) -> CoreResult<()> {
        if docs.is_empty() {
            return Ok(());
        }
        let db = self.db()?;
        let linked: std::collections::HashSet<i64> = db
            .prepare(
                "SELECT DISTINCT archive_doc FROM document_links
                  WHERE archive_id = ?1 AND archive_doc IS NOT NULL AND deleted_ms IS NULL
                    AND booking_id IS NOT NULL",
            )?
            .query_map([archive_id], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        for d in docs {
            d.linked = linked.contains(&d.id);
        }
        Ok(())
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl Vault {
    /// Tries the connection: one request for the number of documents.
    pub async fn archive_test(self: Arc<Self>) -> Result<ArchiveInfo, CoreError> {
        let (archive, _) = self.archive_backend().await?;
        archive.info().await
    }

    /// Searches the archive: full text for `query` (empty = newest first),
    /// optionally only documents created between `date_from` and `date_to`
    /// (`YYYY-MM-DD`, inclusive). At most `limit` documents (up to 50).
    pub async fn archive_search(
        self: Arc<Self>,
        query: String,
        date_from: Option<String>,
        date_to: Option<String>,
        limit: u32,
    ) -> Result<Vec<ArchiveDocument>, CoreError> {
        let (archive, archive_id) = self.archive_backend().await?;
        let mut docs = archive
            .search(
                &query,
                date_from.as_deref(),
                date_to.as_deref(),
                limit.clamp(1, MAX_PAGE),
            )
            .await?;
        self.mark_linked(&archive_id, &mut docs)?;
        Ok(docs)
    }

    /// Documents of the archive that fit a booking: those created shortly
    /// before or after it whose text holds its amount, merchant or date
    /// (the scoring of dropped receipts, `documents::ReceiptFacts`). Best
    /// first, at most `limit`; documents already linked to this booking are
    /// left out.
    pub async fn archive_suggestions(
        self: Arc<Self>,
        booking_id: i64,
        limit: u32,
    ) -> Result<Vec<ArchiveSuggestion>, CoreError> {
        let watch = self.archive_watch();
        let (connected, amount, counterparty, purpose, date, own): (
            Connected,
            i64,
            String,
            String,
            String,
            Vec<i64>,
        ) = {
            let db = self.db()?;
            let connected = connection(&db)?;
            let row = db
                .query_row(
                    "SELECT amount_cents, counterparty, purpose, booking_date FROM transactions
                      WHERE id = ?1 AND deleted_ms IS NULL",
                    [booking_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()?
                .ok_or_else(|| CoreError::NotFound {
                    what: trf("Buchung {0}", &[&booking_id]),
                })?;
            let own = db
                .prepare(
                    "SELECT archive_doc FROM document_links
                      WHERE booking_id = ?1 AND archive_id = ?2 AND archive_doc IS NOT NULL
                        AND deleted_ms IS NULL",
                )?
                .query_map(params![booking_id, connected.id], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            (connected, row.0, row.1, row.2, row.3, own)
        };
        let Some(day) = crate::dates::parse_iso(&date) else {
            return Ok(Vec::new());
        };
        let merchant = crate::merchants::MerchantResolver::resolve(&counterparty, &purpose);
        let window =
            |days: i64| crate::dates::iso(crate::dates::add_days(day, days).unwrap_or(day));
        let archive = connected.backend(watch).await?;
        let mut scored: Vec<(ArchiveDocument, i32)> = Vec::new();
        // two pages at most: the window is small, a booking rarely has more candidates
        for page_no in 1..=2u32 {
            let page = archive
                .created_between(
                    &window(-SUGGEST_DAYS_BEFORE),
                    &window(SUGGEST_DAYS_AFTER),
                    page_no,
                )
                .await?;
            for (doc, content) in page.documents {
                if own.contains(&doc.id) {
                    continue;
                }
                let score = ReceiptFacts::of(&text_of(&doc, &content)).score(
                    amount,
                    &merchant.name,
                    &merchant.key,
                    &date,
                );
                if let Some(score) = score.filter(|s| *s >= MATCH_SCORE) {
                    scored.push((doc, score));
                }
            }
            if !page.more {
                break;
            }
        }
        scored.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.created.cmp(&a.0.created)));
        scored.truncate(limit.clamp(1, MAX_PAGE) as usize);
        let mut docs: Vec<ArchiveDocument> = scored.iter().map(|(d, _)| d.clone()).collect();
        archive.describe(&mut docs).await;
        self.mark_linked(&connected.id, &mut docs)?;
        Ok(docs
            .into_iter()
            .zip(scored)
            .map(|(document, (_, score))| ArchiveSuggestion { document, score })
            .collect())
    }

    /// Bookings without a receipt in the period (`YYYY-MM-DD`, either end
    /// open) and the documents of the archive that fit them, for assigning
    /// many at once. Stricter than the suggestions for one booking: the
    /// amount must be in the document, and every booking and document
    /// appears once, with its best partner. Documents that already hang on
    /// a booking are left out. Nothing is linked: `attach_archive_document`
    /// does that for the matches the user accepts.
    pub async fn archive_matches(
        self: Arc<Self>,
        date_from: Option<String>,
        date_to: Option<String>,
        limit: u32,
    ) -> Result<ArchiveMatches, CoreError> {
        struct Open {
            id: i64,
            cents: i64,
            date: String,
            day: jiff::civil::Date,
            label: String,
            merchant: crate::merchants::MerchantInfo,
        }
        let watch = self.archive_watch();
        let (connected, open, taken, more_bookings): (
            Connected,
            Vec<Open>,
            std::collections::HashSet<i64>,
            bool,
        ) = {
            let db = self.db()?;
            let connected = connection(&db)?;
            let mut st = db.prepare(
                "SELECT t.id, t.amount_cents, t.counterparty, t.purpose, t.booking_date
                   FROM transactions t
                  WHERE t.deleted_ms IS NULL AND t.amount_cents != 0
                    AND (?1 IS NULL OR t.booking_date >= ?1)
                    AND (?2 IS NULL OR t.booking_date <= ?2)
                    AND NOT EXISTS (SELECT 1 FROM booking_documents d WHERE d.booking_id = t.id)
                    AND NOT EXISTS (SELECT 1 FROM document_links l
                                     WHERE l.booking_id = t.id AND l.deleted_ms IS NULL)
                  ORDER BY t.booking_date DESC, t.id DESC LIMIT ?3",
            )?;
            let mut rows = st.query(params![date_from, date_to, MATCH_BOOKINGS as i64 + 1])?;
            let mut open = Vec::new();
            while let Some(r) = rows.next()? {
                let (id, cents, counterparty, purpose, date): (i64, i64, String, String, String) =
                    (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?);
                let Some(day) = crate::dates::parse_iso(&date) else {
                    continue;
                };
                open.push(Open {
                    id,
                    cents,
                    label: crate::documents::booking_label_of(
                        Some(counterparty.clone()),
                        Some(cents),
                        Some(date.clone()),
                    ),
                    merchant: crate::merchants::MerchantResolver::resolve(&counterparty, &purpose),
                    date,
                    day,
                });
            }
            let more = open.len() > MATCH_BOOKINGS;
            open.truncate(MATCH_BOOKINGS);
            let taken = db
                .prepare(
                    "SELECT DISTINCT archive_doc FROM document_links
                      WHERE archive_id = ?1 AND archive_doc IS NOT NULL AND deleted_ms IS NULL
                        AND booking_id IS NOT NULL",
                )?
                .query_map([&connected.id], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            (connected, open, taken, more)
        };
        let mut out = ArchiveMatches {
            open_bookings: open.len() as u32,
            more: more_bookings,
            ..Default::default()
        };
        let (Some(first), Some(last)) = (
            open.iter().map(|b| b.day).min(),
            open.iter().map(|b| b.day).max(),
        ) else {
            return Ok(out);
        };
        let mut by_amount: std::collections::HashMap<i64, Vec<usize>> = Default::default();
        for (i, b) in open.iter().enumerate() {
            by_amount.entry(b.cents.abs()).or_default().push(i);
        }

        let archive = connected.backend(watch).await?;
        // (score, days apart, booking, document, merchant named)
        let mut fits: Vec<(i32, i64, usize, usize, bool)> = Vec::new();
        let mut seen: std::collections::HashSet<i64> = Default::default();
        let mut docs: Vec<ArchiveDocument> = Vec::new();
        let (from, to) = (
            crate::dates::iso(crate::dates::add_days(first, -SUGGEST_DAYS_BEFORE).unwrap_or(first)),
            crate::dates::iso(crate::dates::add_days(last, SUGGEST_DAYS_AFTER).unwrap_or(last)),
        );
        for page_no in 1..=MATCH_PAGES {
            let page = archive.created_between(&from, &to, page_no).await?;
            for (doc, content) in page.documents {
                // pages may overlap when the archive takes in documents meanwhile
                if !seen.insert(doc.id) {
                    continue;
                }
                out.documents += 1;
                if taken.contains(&doc.id) {
                    continue;
                }
                let Some(created) = crate::dates::parse_iso(&doc.created) else {
                    continue;
                };
                let facts = ReceiptFacts::of(&text_of(&doc, &content));
                let at = docs.len();
                let mut fitting = false;
                for amount in facts.amounts() {
                    for &i in by_amount.get(amount).map(Vec::as_slice).unwrap_or_default() {
                        let b = &open[i];
                        let apart = crate::dates::days_between(b.day, created);
                        if !(-SUGGEST_DAYS_BEFORE..=SUGGEST_DAYS_AFTER).contains(&apart) {
                            continue;
                        }
                        let score =
                            facts.score(b.cents, &b.merchant.name, &b.merchant.key, &b.date);
                        if let Some(score) = score.filter(|s| *s >= MATCH_SCORE) {
                            let named = facts.names_exactly(&b.merchant.name);
                            fits.push((score, apart.abs(), i, at, named));
                            fitting = true;
                        }
                    }
                }
                if fitting {
                    docs.push(doc);
                }
            }
            if !page.more {
                break;
            }
            if page_no == MATCH_PAGES {
                out.more = true;
            }
        }

        // the best pairs first; every booking and every document once
        fits.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then(a.1.cmp(&b.1))
                .then(a.2.cmp(&b.2))
                .then(a.3.cmp(&b.3))
        });
        fits.dedup();
        let mut booking_used = vec![false; open.len()];
        let mut doc_used = vec![false; docs.len()];
        let mut chosen: Vec<(i32, usize, usize, bool)> = Vec::new();
        for &(score, _, b, d, named) in &fits {
            if !booking_used[b] && !doc_used[d] {
                booking_used[b] = true;
                doc_used[d] = true;
                chosen.push((score, b, d, named));
            }
        }
        // newest booking first (that is the order of `open`)
        chosen.sort_by_key(|c| c.1);
        let limit = limit.clamp(1, MATCH_LIMIT) as usize;
        out.more |= chosen.len() > limit;
        chosen.truncate(limit);
        let mut found: Vec<ArchiveDocument> = chosen.iter().map(|c| docs[c.2].clone()).collect();
        archive.describe(&mut found).await;
        out.matches = chosen
            .iter()
            .zip(found)
            .map(|(&(score, b, d, merchant_fits), document)| ArchiveMatch {
                booking_id: open[b].id,
                booking: open[b].label.clone(),
                document,
                score,
                merchant_fits,
                // a rival of the same strength for the booking or the document
                ambiguous: fits
                    .iter()
                    .any(|f| f.0 == score && (f.2 == b) != (f.3 == d)),
            })
            .collect();
        Ok(out)
    }

    /// The file of a linked receipt (`DocumentInfo.id` of a reference into
    /// the archive), for the preview, Quick Look and saving. `original`:
    /// the uploaded file instead of the archive's own PDF version.
    pub async fn archive_file(
        self: Arc<Self>,
        doc_id: i64,
        original: bool,
    ) -> Result<ArchiveFile, CoreError> {
        let watch = self.archive_watch();
        // the document and the connection it belongs to, read together: a
        // change of archive in between must not pair an id with another archive
        let (connected, archive_doc): (Connected, i64) = {
            let db = self.db()?;
            let row = crate::document_links::row_id_of(doc_id).ok_or_else(unexpected)?;
            let connected = connection(&db)?;
            let archive_doc = db
                .query_row(
                    "SELECT archive_doc FROM document_links
                      WHERE id = ?1 AND archive_id = ?2 AND deleted_ms IS NULL",
                    params![row, connected.id],
                    |r| r.get::<_, Option<i64>>(0),
                )
                .optional()?
                .flatten()
                .ok_or_else(|| CoreError::NotFound {
                    what: tr("Dokument im Archiv").into(),
                })?;
            (connected, archive_doc)
        };
        connected
            .backend(watch)
            .await?
            .file(archive_doc, original)
            .await
    }

    /// Thumbnail of an archive document (first page), for the search results.
    pub async fn archive_thumbnail(
        self: Arc<Self>,
        archive_doc: i64,
    ) -> Result<Vec<u8>, CoreError> {
        let (archive, _) = self.archive_backend().await?;
        archive.thumbnail(archive_doc).await
    }
}

/// What a document says, for holding it against bookings: title, file name,
/// date and its text.
fn text_of(doc: &ArchiveDocument, content: &str) -> String {
    format!(
        "{} {} {} {}",
        doc.title,
        doc.file_name.replace(['_', '-'], " "),
        doc.created,
        content
    )
}
