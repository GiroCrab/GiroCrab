//! What GiroCrab asks of a document archive, whatever software runs it.
//!
//! The archive logic (`archive.rs`: linking, suggestions; `moves.rs`: the
//! queue of files on their way; `facts.rs`: booking data on documents) knows
//! bookings, the vault and what must never be lost. It does not know a single
//! address or answer format: those live behind [`Backend`], one
//! implementation per kind of archive, like the bank connectors behind
//! `BankConnector`. Today there is one, [`Kind::Paperless`].
//!
//! Adding a kind: a module next to `paperless.rs` that implements
//! [`Backend`], a variant of [`Kind`] with its id, name and the two address
//! functions. Nothing in the vault changes: a reference stores the archive's
//! id, the document's id there and its address, never the kind.
//!
//! What every implementation owes the rest (see `client.rs`): requests only
//! to the configured address, no redirects followed, errors without the
//! address, the token or the archive's own words.

use async_trait::async_trait;
use url::Url;

use super::client::{Failure, Watch};
use super::{ArchiveDocument, ArchiveFile, ArchiveInfo};
use crate::error::CoreResult;

/// The kinds of archive this version can talk to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Paperless-ngx and whatever speaks its REST API (`paperless.rs`).
    Paperless,
}

impl Kind {
    pub(crate) const ALL: [Kind; 1] = [Kind::Paperless];
    /// What a connection is when nothing says otherwise.
    pub(crate) const DEFAULT: Kind = Kind::Paperless;

    /// Stored with the connection (`archive.kind`).
    pub(crate) fn id(self) -> &'static str {
        match self {
            Kind::Paperless => "paperless",
        }
    }

    /// The name of the software, for the settings.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Kind::Paperless => "Paperless-ngx",
        }
    }

    pub(crate) fn of(id: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.id() == id)
    }

    /// Address of a document in the archive's own web interface.
    pub(crate) fn document_url(self, base: &Url, doc: i64) -> String {
        match self {
            Kind::Paperless => super::paperless::document_url(base, doc),
        }
    }

    /// The document a link points at, if it is an address inside the archive
    /// at `base`.
    pub(crate) fn document_id_in(self, base: &Url, link: &Url) -> Option<i64> {
        match self {
            Kind::Paperless => super::paperless::document_id_in(base, link),
        }
    }

    /// A backend for the archive at `base`. `archive_id` is the vault's own
    /// id for it and goes into the documents it returns; `watch` ends its
    /// requests when the vault is locked or the connection changes.
    pub(crate) async fn connect(
        self,
        base: &Url,
        token: &str,
        archive_id: &str,
        watch: Watch,
    ) -> CoreResult<Box<dyn Backend>> {
        Ok(match self {
            Kind::Paperless => Box::new(
                super::paperless::Paperless::connect(base, token, archive_id, watch).await?,
            ),
        })
    }
}

/// One page of documents with their full text.
pub(crate) struct Page {
    /// Document and its text, newest first.
    pub(crate) documents: Vec<(ArchiveDocument, String)>,
    /// There is a page after this one.
    pub(crate) more: bool,
}

/// What an archive did with a file handed to it.
#[derive(Debug, PartialEq)]
pub(crate) enum Uploaded {
    /// It is being taken in; ask with this handle
    /// ([`Backend::upload_state`]).
    Processing(String),
    /// Taken, but with nothing to ask about it: nobody knows what became of it.
    Unknown,
}

/// Where an archive files a document, in its own terms: what
/// [`Backend::file_under`] made of [`Labels`]. Nothing for an archive that
/// has no such thing.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Filed(pub(crate) Vec<(&'static str, String)>);

/// A file to hand to the archive.
pub(crate) struct Upload {
    /// May be empty.
    pub(crate) title: String,
    /// `YYYY-MM-DD`, if known.
    pub(crate) created: Option<String>,
    pub(crate) file_name: String,
    pub(crate) mime: String,
    pub(crate) bytes: Vec<u8>,
    /// Where the archive files it (from [`Backend::file_under`]).
    pub(crate) filed: Filed,
}

/// What GiroCrab knows about a file it hands over, in the words an archive
/// sorts its documents by. An archive files the document under them, making
/// what it lacks; one that has no such thing, or does not let this token
/// make it, takes the file without. Never a reason for a file not to go up.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Labels {
    /// Who the document is from (the bank).
    pub(crate) sender: Option<String>,
    /// What kind of document it is (statement, tax document …).
    pub(crate) kind: Option<String>,
    /// Further words to find it by (the account).
    pub(crate) tags: Vec<String>,
}

/// What the archive says about a processing task.
#[derive(Debug, PartialEq)]
pub(crate) enum TaskState {
    /// Still pending or running.
    Working,
    /// Done: the document id (new, or the one the file is a duplicate of).
    Document(i64),
    /// Refused or broken; the archive's own words are not passed on.
    Failed,
    /// The file is a duplicate of a document that lies in the archive's trash.
    InTrash,
    /// The archive has no task with this id (yet, or any more).
    Unknown,
}

/// One archive, connected. Document ids are the archive's own.
///
/// `CoreResult` errors are worded for the user. Where the caller must know
/// *how* a request failed (was anything sent? did the archive refuse?), the
/// error is a [`Failure`].
#[async_trait]
pub(crate) trait Backend: Send + Sync {
    // ------------------------------------------------------------ reading

    /// Tries the connection: version and number of documents.
    async fn info(&self) -> CoreResult<ArchiveInfo>;

    /// Full-text search (an empty `query`: newest first), optionally only
    /// documents created between two days (inclusive). With the names the
    /// archive has for sender and type. At most `limit` documents.
    async fn search(
        &self,
        query: &str,
        from: Option<&str>,
        to: Option<&str>,
        limit: u32,
    ) -> CoreResult<Vec<ArchiveDocument>>;

    /// Page `page` (from 1) of the documents created between two days,
    /// newest first, each with its full text. Names may be left out here
    /// ([`describe`](Self::describe) adds them for the few that are shown).
    async fn created_between(&self, from: &str, to: &str, page: u32) -> CoreResult<Page>;

    /// Fills in sender and type of documents a page listed. A nicety: on
    /// any failure they simply stay empty.
    async fn describe(&self, documents: &mut [ArchiveDocument]);

    /// The file of a document: the uploaded `original`, or the archive's own
    /// version of it.
    async fn file(&self, doc: i64, original: bool) -> CoreResult<ArchiveFile>;

    /// A small picture of the first page.
    async fn thumbnail(&self, doc: i64) -> CoreResult<Vec<u8>>;

    // ------------------------------------------------------------ taking files in

    /// The document that is this very file (SHA-256 of its bytes), if the
    /// archive has one: found and compared byte for byte, never guessed.
    /// `Ok(None)` also when this kind of archive cannot be asked that. An
    /// error means it could not answer for sure right now (busy, a document
    /// this token may not read, an answer that is none): nothing may be
    /// uploaded on that.
    async fn find_file(&self, sha256: &[u8]) -> Result<Option<i64>, Failure>;

    /// Finds where the archive files a document with these `labels`, making
    /// what it lacks. Asked before the file is handed over and apart from
    /// it: it may take a while, and whoever hands the file over makes sure
    /// right before that it is still wanted. A label the archive does not
    /// let this token look up or make is left out. An error means no file
    /// should go up now (the archive is not there, the token not valid).
    async fn file_under(&self, labels: &Labels) -> Result<Filed, Failure>;

    /// Hands a file over. The failure says whether the archive may have
    /// taken it.
    async fn upload(&self, file: Upload) -> Result<Uploaded, Failure>;

    /// What became of an upload.
    async fn upload_state(&self, handle: &str) -> CoreResult<TaskState>;

    /// Whether the document is there for this token. `false` only when the
    /// archive says so itself: it does not exist, or lies in its trash. An
    /// answer that says neither is a failure, never a no: files are kept
    /// and fields left alone on a no.
    async fn document_exists(&self, doc: i64) -> Result<bool, Failure>;

    /// The file a document was first stored with, as the archive hands it
    /// back: not a later version the document may have got there since.
    async fn original(&self, doc: i64) -> Result<Vec<u8>, Failure>;
}
