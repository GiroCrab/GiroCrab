//! Paperless-ngx as a document archive: its REST API (version 9, which
//! current servers answer alongside newer ones), its answers and its custom
//! fields. Everything the rest of the archive code does not need to know.
//!
//! Tried against Paperless-ngx 3.4.2 (`tests.rs`, `live_archive`). Things
//! that are particular to it and decided here:
//! - An upload is a task; the document comes later (`/api/tasks/`).
//! - Since 3.x the same file uploaded twice is kept twice, so the archive is
//!   asked for the file's checksum first (`checksum__iexact`, SHA-256 there;
//!   older servers keep MD5 and refuse duplicates themselves). What it lists
//!   is compared byte for byte: a server that does not know the filter lists
//!   anything.
//! - A document in the trash still hands out its file, only its own address
//!   answers 404.
//! - Booking data goes into custom fields, written with `bulk_edit`'s
//!   `modify_custom_fields`, which touches only the fields it names.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use url::Url;
use zeroize::Zeroizing;

use super::backend::{Backend, FactKind, Filed, Labels, Page, TaskState, Upload, Uploaded};
use super::client::{Client, Failure, Watch};
use super::facts::{Facts, FIELDS};
use super::{invalid, unexpected, valid_date, ArchiveDocument, ArchiveFile, ArchiveInfo};
use crate::connectors::{MAX_DOC_BYTES, MAX_TEXT_BYTES};
use crate::error::{CoreError, CoreResult};
use crate::i18n::{tr, trf};

/// API version asked for with every request.
const API_VERSION: &str = "application/json; version=9";
/// Largest page the archive is asked for.
const MAX_PAGE: u32 = 50;
pub(crate) const MAX_THUMB_BYTES: usize = 2 * 1024 * 1024;
const FIELD_PAGES: u32 = 20;
/// Longest name the archive takes for a sender, a type or a tag.
const MAX_LABEL: usize = 128;

pub(crate) struct Paperless {
    client: Client,
    base: Url,
    /// The vault's id for this archive: goes into the documents returned.
    archive_id: String,
    /// Sender and type (their ids) of the documents a page listed, for
    /// `describe`.
    refs: Mutex<HashMap<i64, (Option<i64>, Option<i64>)>>,
    /// The ids of senders, types and tags asked for by name (list, name in
    /// lower case), for the uploads of one run.
    labels: Mutex<HashMap<(&'static str, String), i64>>,
}

impl Paperless {
    pub(crate) async fn connect(
        base: &Url,
        token: &str,
        archive_id: &str,
        watch: Watch,
    ) -> CoreResult<Self> {
        let headers = vec![
            ("Authorization", Zeroizing::new(format!("Token {token}"))),
            ("Accept", Zeroizing::new(API_VERSION.to_string())),
        ];
        Ok(Self {
            client: Client::new(base, headers, watch).await?,
            base: base.clone(),
            archive_id: archive_id.to_string(),
            refs: Mutex::default(),
            labels: Mutex::default(),
        })
    }

    /// The id of the sender, type or tag called `name` in `list` (the part
    /// of the address they live under), made when the archive has none. It
    /// is made without a rule of its own: the archive does not start to put
    /// it on other documents. `None`: the archive does not let this token
    /// look or make one, or does not take the name; the file goes up
    /// without. An error means no file should go up now (nothing was sent
    /// of it): the archive is not there, or the token is not valid.
    async fn label(&self, list: &'static str, name: &str) -> Result<Option<i64>, Failure> {
        let name: String = name
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(MAX_LABEL)
            .collect();
        if name.is_empty() {
            return Ok(None);
        }
        let key = (list, name.to_lowercase());
        if let Some(id) = self.labels.lock().ok().and_then(|l| l.get(&key).copied()) {
            return Ok(Some(id));
        }
        let without = |f: Failure| -> Result<Option<i64>, Failure> {
            match f {
                Failure::Status(s) if s.as_u16() == 401 => Err(Failure::Status(s)),
                // not allowed, not known here, not a name it takes
                Failure::Status(s)
                    if (400..500).contains(&s.as_u16()) && !matches!(s.as_u16(), 408 | 429) =>
                {
                    Ok(None)
                }
                _ => Err(Failure::NotSent),
            }
        };
        let path = format!("/api/{list}/");
        // An archive that does not know the question lists anything: only an
        // entry of this very name counts.
        let listed = match self
            .client
            .get_checked(
                &path,
                &[
                    ("name__iexact", name.clone()),
                    ("page_size", MAX_PAGE.to_string()),
                ],
                MAX_TEXT_BYTES,
            )
            .await
        {
            Ok(reply) => reply.json().ok(),
            Err(f) => return without(f),
        };
        let found = listed.as_ref().and_then(|page| {
            page.get("results")?.as_array()?.iter().find_map(|entry| {
                let same = entry.get("name")?.as_str()?.to_lowercase() == key.1;
                same.then(|| entry.get("id")?.as_i64()).flatten()
            })
        });
        let id = match found {
            Some(id) => id,
            None => match self
                .client
                .post_json(&path, &json!({"name": name, "matching_algorithm": 0}))
                .await
            {
                Ok(reply) => match reply.json().ok().and_then(|made| made.get("id")?.as_i64()) {
                    Some(id) => id,
                    None => return Ok(None),
                },
                Err(f) => return without(f),
            },
        };
        if let Ok(mut labels) = self.labels.lock() {
            labels.insert(key, id);
        }
        Ok(Some(id))
    }

    /// The form fields that file a document under `labels`.
    async fn label_fields(&self, labels: &Labels) -> Result<Vec<(&'static str, String)>, Failure> {
        let mut fields = Vec::new();
        if let Some(sender) = &labels.sender {
            if let Some(id) = self.label("correspondents", sender).await? {
                fields.push(("correspondent", id.to_string()));
            }
        }
        if let Some(kind) = &labels.kind {
            if let Some(id) = self.label("document_types", kind).await? {
                fields.push(("document_type", id.to_string()));
            }
        }
        for tag in &labels.tags {
            if let Some(id) = self.label("tags", tag).await? {
                fields.push(("tags", id.to_string()));
            }
        }
        Ok(fields)
    }
}

// ------------------------------------------------------------------ addresses

/// Address of document `id` in the archive's own web interface.
pub(crate) fn document_url(base: &Url, id: i64) -> String {
    format!(
        "{}/documents/{id}/details",
        base.as_str().trim_end_matches('/')
    )
}

/// The id of the archive document a link points at, if it is an address
/// inside the archive at `base`: same scheme, host and port, and the path
/// `<base path>/documents/<id>` (anything may follow the id).
pub(crate) fn document_id_in(base: &Url, link: &Url) -> Option<i64> {
    if base.scheme() != link.scheme()
        || base.host_str() != link.host_str()
        || base.port_or_known_default() != link.port_or_known_default()
    {
        return None;
    }
    let rest = link
        .path()
        .strip_prefix(base.path().trim_end_matches('/'))?
        .strip_prefix("/documents/")?;
    let id = rest.split('/').next()?;
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    id.parse().ok().filter(|id: &i64| *id > 0)
}

// ------------------------------------------------------------------ reading

fn str_of(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// First ten characters of a date or date-time (`2026-11-07`).
fn date_of(v: &Value, key: &str) -> String {
    str_of(v, key).chars().take(10).collect()
}

/// Plain text of a search highlight (`a <span class="match">b</span> c`),
/// or the first words of the content.
pub(crate) fn excerpt_of(doc: &Value) -> String {
    let highlight = doc
        .get("__search_hit__")
        .map(|h| str_of(h, "highlights"))
        .unwrap_or_default();
    let source = if highlight.trim().is_empty() {
        str_of(doc, "content")
    } else {
        let mut plain = String::with_capacity(highlight.len());
        let mut in_tag = false;
        for c in highlight.chars() {
            match c {
                '<' => in_tag = true,
                '>' => in_tag = false,
                c if !in_tag => plain.push(c),
                _ => {}
            }
        }
        plain
    };
    let words: Vec<&str> = source.split_whitespace().collect();
    let mut out = String::new();
    for w in words {
        if out.chars().count() + w.chars().count() + 1 > 220 {
            out.push('…');
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    out
}

/// The documents of a list response, with the raw objects (the suggestions
/// read the full text).
fn documents_of<'a>(
    page: &'a Value,
    base: &Url,
    archive_id: &str,
) -> CoreResult<Vec<(ArchiveDocument, &'a Value)>> {
    let results = page
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(unexpected)?;
    Ok(results
        .iter()
        .filter_map(|d| {
            let id = d.get("id")?.as_i64()?;
            Some((
                ArchiveDocument {
                    id,
                    archive_id: archive_id.to_string(),
                    title: str_of(d, "title"),
                    created: date_of(d, "created"),
                    correspondent: String::new(),
                    document_type: String::new(),
                    file_name: str_of(d, "original_file_name"),
                    page_count: d.get("page_count").and_then(Value::as_u64).unwrap_or(0) as u32,
                    excerpt: excerpt_of(d),
                    linked: false,
                    url: document_url(base, id),
                },
                d,
            ))
        })
        .collect())
}

/// Names for the ids of `kind` (`correspondents`, `document_types`) the
/// results use: one request for exactly those ids. The names are a nicety:
/// a token may read documents but not these lists, and then (or on any other
/// failure) the documents are shown without them.
async fn names(client: &Client, kind: &str, ids: &[i64]) -> std::collections::HashMap<i64, String> {
    let mut ids: Vec<i64> = ids.to_vec();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Default::default();
    }
    let list = ids
        .iter()
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let reply = client
        .get(
            &format!("/api/{kind}/"),
            &[("id__in", list), ("page_size", ids.len().to_string())],
            MAX_TEXT_BYTES,
        )
        .await;
    let Ok(page) = reply.and_then(|r| r.json()) else {
        return Default::default();
    };
    page.get("results")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|r| Some((r.get("id")?.as_i64()?, str_of(r, "name"))))
                .collect()
        })
        .unwrap_or_default()
}

/// Fills correspondent and document type of `docs` from the raw objects.
async fn name_documents(
    client: &Client,
    docs: &mut [ArchiveDocument],
    raw: &[(i64, Option<i64>, Option<i64>)],
) {
    let correspondents: Vec<i64> = raw.iter().filter_map(|r| r.1).collect();
    let types: Vec<i64> = raw.iter().filter_map(|r| r.2).collect();
    let correspondents = names(client, "correspondents", &correspondents).await;
    let types = names(client, "document_types", &types).await;
    for d in docs {
        if let Some((_, c, t)) = raw.iter().find(|r| r.0 == d.id) {
            d.correspondent = c
                .and_then(|c| correspondents.get(&c).cloned())
                .unwrap_or_default();
            d.document_type = t.and_then(|t| types.get(&t).cloned()).unwrap_or_default();
        }
    }
}

fn refs_of(raw: &Value) -> (Option<i64>, Option<i64>) {
    (
        raw.get("correspondent").and_then(Value::as_i64),
        raw.get("document_type").and_then(Value::as_i64),
    )
}

/// File name from a `Content-Disposition` header, reduced to a plain name
/// (no folders); without one, a name from the document id and its type.
pub(crate) fn file_name_of(disposition: &str, archive_doc: i64, mime_type: &str) -> String {
    let parts = || disposition.split(';').map(str::trim);
    // `filename*=UTF-8''percent%20encoded` (RFC 5987) wins over `filename=`
    let named = parts()
        .find_map(|p| p.strip_prefix("filename*="))
        // charset'language'value: the value is what follows the second apostrophe
        .map(|v| percent_decoded(v.splitn(3, '\'').nth(2).unwrap_or(v)))
        .or_else(|| {
            parts()
                .find_map(|p| p.strip_prefix("filename="))
                .map(|v| v.trim_matches('"').to_string())
        });
    let name: String = named
        .unwrap_or_default()
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    let name = name.trim().trim_start_matches('.').to_string();
    if !name.is_empty() {
        return name;
    }
    let ext = match mime_type {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        _ => "pdf",
    };
    format!("dokument-{archive_doc}.{ext}")
}

/// `%41` -> `A`; anything that is not a valid escape stays as it is.
pub(crate) fn percent_decoded(s: &str) -> String {
    let bytes = s.as_bytes();
    let hex = |b: Option<&u8>| b.and_then(|b| (*b as char).to_digit(16)).map(|d| d as u8);
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match (bytes[i], hex(bytes.get(i + 1)), hex(bytes.get(i + 2))) {
            (b'%', Some(hi), Some(lo)) => {
                out.push(hi * 16 + lo);
                i += 3;
            }
            (b, _, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ------------------------------------------------------------------ taking files in

/// One multipart form with the fields and the file, built here: `reqwest`'s
/// own builder is a feature this crate does not enable. `bytes` becomes the
/// body (head and tail are added around it), so a large file is not held twice.
pub(crate) fn multipart(
    fields: &[(&str, &str)],
    file_name: &str,
    mime: &str,
    bytes: Vec<u8>,
) -> (String, Vec<u8>) {
    let boundary = format!("girocrab-{}", uuid::Uuid::new_v4().simple());
    // header values: no quotes, backslashes or line breaks
    let plain = |s: &str| -> String {
        s.chars()
            .filter(|c| !c.is_control() && *c != '"' && *c != '\\')
            .collect()
    };
    let mut head = String::new();
    for (name, value) in fields {
        head.push_str(&format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"\r\n\r\n{}\r\n",
            plain(name),
            value.replace(['\r', '\n'], " ")
        ));
    }
    let name = plain(file_name);
    head.push_str(&format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"document\"; filename=\"{}\"\r\nContent-Type: {}\r\n\r\n",
        if name.is_empty() { "dokument" } else { &name },
        plain(mime)
    ));
    let mut body = bytes;
    body.splice(0..0, head.into_bytes());
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

fn id_of(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
        .filter(|id| *id > 0)
}

/// Reads `/api/tasks/?task_id=…`: a list in API version 9, a page with
/// `results` in later ones. The document comes from `related_document` (a
/// number or a string), for a refused duplicate also from
/// `duplicate_documents`. A document that list marks as deleted is in the
/// archive's trash, also when `related_document` names it.
pub(crate) fn task_state(reply: &Value) -> TaskState {
    let tasks = reply
        .as_array()
        .or_else(|| reply.get("results").and_then(Value::as_array));
    let Some(task) = tasks.and_then(|t| t.first()) else {
        return TaskState::Unknown;
    };
    let status = task
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_uppercase();
    let related = task.get("related_document").and_then(id_of);
    let duplicates: Vec<(i64, bool)> = task
        .get("duplicate_documents")
        .and_then(Value::as_array)
        .map(|d| {
            d.iter()
                .filter_map(|d| {
                    let trashed = !d.get("deleted_at").is_none_or(Value::is_null);
                    Some((d.get("id").and_then(id_of)?, trashed))
                })
                .collect()
        })
        .unwrap_or_default();
    let trashed = |id: i64| duplicates.iter().any(|(d, t)| *d == id && *t);
    match status.as_str() {
        "SUCCESS" => match related {
            Some(id) if !trashed(id) => TaskState::Document(id),
            Some(_) => TaskState::InTrash,
            None => TaskState::Failed,
        },
        "FAILURE" | "REVOKED" => {
            let live = duplicates.iter().find(|(_, t)| !*t).map(|(id, _)| *id);
            match related.filter(|id| !trashed(*id)).or(live) {
                Some(id) => TaskState::Document(id),
                None if related.is_some() || !duplicates.is_empty() => TaskState::InTrash,
                None => TaskState::Failed,
            }
        }
        _ => TaskState::Working,
    }
}

/// The question for the file a document was first stored with. An archive
/// that keeps versions of a document hands out the newest one unless asked
/// for one by its id, and the first version has the id of the document
/// itself; its checksum is the one a document is found by, and it stays for
/// as long as the document does. An archive without versions ignores the
/// extra word.
fn first_original(doc: i64) -> [(&'static str, String); 2] {
    [
        ("original", "true".to_string()),
        ("version", doc.to_string()),
    ]
}

/// The document of the archive that is this very file, if it has one. Asked
/// for by checksum and then compared byte for byte: an archive that does not
/// know the question answers with other documents (or not at all), and those
/// are no match. Only a missing connection is an error.
async fn known_document(client: &Client, ours: &[u8]) -> Result<Option<i64>, Failure> {
    // "I do not know that question" (or that document) is an answer;
    // "not now", "not you" and no answer are not: then nothing is uploaded
    // on a guess, the file waits.
    let no = |f: &Failure| matches!(f, Failure::Status(s) if matches!(s.as_u16(), 400 | 404));
    let page = match client
        .get_checked(
            "/api/documents/",
            &[
                ("checksum__iexact", hex::encode(ours)),
                ("page_size", "2".to_string()),
                ("truncate_content", "true".to_string()),
            ],
            MAX_TEXT_BYTES,
        )
        .await
    {
        Ok(reply) => reply.json().map_err(|_| Failure::Unclear)?,
        Err(f) if no(&f) => return Ok(None),
        Err(f) => return Err(f),
    };
    let listed = page
        .get("results")
        .and_then(Value::as_array)
        .ok_or(Failure::Unclear)?;
    // every entry must be a document: one that is not is an answer nobody
    // can go by
    let mut candidates: Vec<i64> = Vec::new();
    for entry in listed.iter().take(2) {
        candidates.push(
            entry
                .get("id")
                .and_then(Value::as_i64)
                .ok_or(Failure::Unclear)?,
        );
    }
    for id in candidates {
        match client
            .get_checked(
                &format!("/api/documents/{id}/download/"),
                &first_original(id),
                MAX_DOC_BYTES,
            )
            .await
        {
            Ok(reply) if Sha256::digest(&reply.body).as_slice() == ours => return Ok(Some(id)),
            // another file, or one too large to be ours
            Ok(_) | Err(Failure::TooLarge) => {}
            // gone since it was listed
            Err(f) if no(&f) => {}
            // (also: a document this token may not read; it may be the file)
            Err(f) => return Err(f),
        }
    }
    Ok(None)
}

// ------------------------------------------------------------------ booking data

/// The archive's data type for a field.
fn data_type(kind: FactKind) -> String {
    match kind {
        FactKind::Date => "date",
        FactKind::Money => "monetary",
        FactKind::Text => "string",
    }
    .to_string()
}

/// The ids of the four fields in the archive, in the order of [`FIELDS`].
/// A field that is missing is created, after a look whether the writing is
/// still `wanted` (none when it is not: nothing more is made).
async fn field_ids(
    client: &Client,
    wanted: &(dyn Fn() -> CoreResult<bool> + Sync),
) -> CoreResult<Option<[i64; 4]>> {
    if !wanted()? {
        return Ok(None);
    }
    let mut found = existing_fields(client).await?;
    let mut ids = [0i64; 4];
    for (i, (name, kind)) in FIELDS.iter().enumerate() {
        let data_type = &data_type(*kind);
        if !found.contains_key(*name) {
            if !wanted()? {
                return Ok(None);
            }
            let body = json!({ "name": name, "data_type": data_type });
            match client.post_json("/api/custom_fields/", &body).await {
                Ok(reply) => {
                    let made = reply.json()?;
                    if let Some(id) = made.get("id").and_then(Value::as_i64) {
                        found.insert((*name).to_string(), (id, (*data_type).to_string()));
                    }
                }
                // another device was faster: the field is there now
                Err(Failure::Status(status)) if status.as_u16() == 400 => {
                    found = existing_fields(client).await?;
                }
                Err(e) => return Err(e.into()),
            }
        }
        match found.get(*name) {
            Some((id, kind)) if kind == data_type => ids[i] = *id,
            Some(_) => {
                return Err(CoreError::InvalidInput {
                    message: trf(
                        "Im Dokumentenarchiv gibt es das Feld „{0}“ schon mit einem anderen Typ. Bitte dort umbenennen oder löschen.",
                        &[name],
                    ),
                })
            }
            None => {
                return Err(CoreError::Protocol {
                    message: tr("Das Dokumentenarchiv hat unerwartet geantwortet.").into(),
                })
            }
        }
    }
    Ok(Some(ids))
}

/// Name → (id, data type) of the archive's custom fields. Pages are asked
/// for by number; a `next` link is never followed.
async fn existing_fields(client: &Client) -> CoreResult<HashMap<String, (i64, String)>> {
    let mut out = HashMap::new();
    for page in 1..=FIELD_PAGES {
        let reply = client
            .get(
                "/api/custom_fields/",
                &[("page", page.to_string()), ("page_size", "100".into())],
                MAX_TEXT_BYTES,
            )
            .await?
            .json()?;
        let results = reply
            .get("results")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for f in &results {
            if let (Some(id), Some(name), Some(kind)) = (
                f.get("id").and_then(Value::as_i64),
                f.get("name").and_then(Value::as_str),
                f.get("data_type").and_then(Value::as_str),
            ) {
                out.insert(name.to_string(), (id, kind.to_string()));
            }
        }
        if results.is_empty() || reply.get("next").is_none_or(Value::is_null) {
            break;
        }
    }
    Ok(out)
}

/// The request that makes `doc` carry `facts` in the fields `ids`. A field
/// without a value is taken off the document only where it `carried` one
/// from this device.
pub(crate) fn change(ids: &[i64; 4], doc: i64, facts: &Facts, carried: [bool; 4]) -> Value {
    let mut add = serde_json::Map::new();
    let mut remove = Vec::new();
    for ((id, value), had) in ids.iter().zip(facts.values()).zip(carried) {
        if !value.is_empty() {
            add.insert(id.to_string(), Value::String(value.to_string()));
        } else if had {
            remove.push(*id);
        }
    }
    json!({
        "documents": [doc],
        "method": "modify_custom_fields",
        "parameters": { "add_custom_fields": add, "remove_custom_fields": remove },
    })
}

// ------------------------------------------------------------------ the interface

#[async_trait]
impl Backend for Paperless {
    async fn info(&self) -> CoreResult<ArchiveInfo> {
        let reply = self
            .client
            .get(
                "/api/documents/",
                &[
                    ("page_size", "1".to_string()),
                    ("truncate_content", "true".to_string()),
                ],
                MAX_TEXT_BYTES,
            )
            .await?;
        let page = reply.json()?;
        let count = page
            .get("count")
            .and_then(Value::as_u64)
            .ok_or_else(unexpected)?;
        Ok(ArchiveInfo {
            server_version: reply.header("x-version"),
            api_version: reply.header("x-api-version"),
            document_count: count.min(u32::MAX as u64) as u32,
        })
    }

    async fn search(
        &self,
        query: &str,
        from: Option<&str>,
        to: Option<&str>,
        limit: u32,
    ) -> CoreResult<Vec<ArchiveDocument>> {
        let mut params: Vec<(&str, String)> = vec![
            ("page_size", limit.clamp(1, MAX_PAGE).to_string()),
            ("truncate_content", "true".to_string()),
        ];
        let query = query.trim();
        if query.is_empty() {
            params.push(("ordering", "-created".to_string()));
        } else {
            params.push(("query", query.to_string()));
        }
        for (key, date) in [("created__date__gte", from), ("created__date__lte", to)] {
            if let Some(d) = date.map(str::trim).filter(|d| !d.is_empty()) {
                if !valid_date(d) {
                    return Err(invalid(tr("Das Datum ist ungültig (JJJJ-MM-TT).")));
                }
                params.push((key, d.to_string()));
            }
        }
        let page = self
            .client
            .get("/api/documents/", &params, MAX_TEXT_BYTES)
            .await?
            .json()?;
        let found = documents_of(&page, &self.base, &self.archive_id)?;
        let raw: Vec<(i64, Option<i64>, Option<i64>)> = found
            .iter()
            .map(|(d, v)| {
                let (c, t) = refs_of(v);
                (d.id, c, t)
            })
            .collect();
        let mut docs: Vec<ArchiveDocument> = found.into_iter().map(|(d, _)| d).collect();
        name_documents(&self.client, &mut docs, &raw).await;
        Ok(docs)
    }

    async fn created_between(&self, from: &str, to: &str, page: u32) -> CoreResult<Page> {
        let reply = self
            .client
            .get(
                "/api/documents/",
                &[
                    ("created__date__gte", from.to_string()),
                    ("created__date__lte", to.to_string()),
                    ("ordering", "-created".to_string()),
                    ("page_size", MAX_PAGE.to_string()),
                    ("page", page.to_string()),
                ],
                MAX_TEXT_BYTES,
            )
            .await?
            .json()?;
        let found = documents_of(&reply, &self.base, &self.archive_id)?;
        // the archive names a next page (the link itself is never followed);
        // a page may hold fewer documents than asked for and still not be the last
        let more = !found.is_empty() && !reply.get("next").is_none_or(Value::is_null);
        let mut refs = self.refs.lock().map_err(|_| unexpected())?;
        let documents = found
            .into_iter()
            .map(|(doc, raw)| {
                refs.insert(doc.id, refs_of(raw));
                (doc, str_of(raw, "content"))
            })
            .collect();
        Ok(Page { documents, more })
    }

    async fn describe(&self, documents: &mut [ArchiveDocument]) {
        let raw: Vec<(i64, Option<i64>, Option<i64>)> = match self.refs.lock() {
            Ok(refs) => documents
                .iter()
                .filter_map(|d| refs.get(&d.id).map(|(c, t)| (d.id, *c, *t)))
                .collect(),
            Err(_) => return,
        };
        name_documents(&self.client, documents, &raw).await;
    }

    async fn file(&self, doc: i64, original: bool) -> CoreResult<ArchiveFile> {
        let query: Vec<(&str, String)> = if original {
            vec![("original", "true".to_string())]
        } else {
            Vec::new()
        };
        let reply = self
            .client
            .get(
                &format!("/api/documents/{doc}/download/"),
                &query,
                MAX_DOC_BYTES,
            )
            .await?;
        let mime_type = reply
            .header("content-type")
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_lowercase();
        let file_name = file_name_of(&reply.header("content-disposition"), doc, &mime_type);
        Ok(ArchiveFile {
            mime_type: if mime_type.is_empty() {
                crate::documents::sniff_mime(&reply.body, &file_name)
            } else {
                mime_type
            },
            file_name,
            bytes: reply.body,
        })
    }

    async fn thumbnail(&self, doc: i64) -> CoreResult<Vec<u8>> {
        Ok(self
            .client
            .get(
                &format!("/api/documents/{doc}/thumb/"),
                &[],
                MAX_THUMB_BYTES,
            )
            .await?
            .body)
    }

    async fn find_file(&self, sha256: &[u8]) -> Result<Option<i64>, Failure> {
        known_document(&self.client, sha256).await
    }

    async fn file_under(&self, labels: &Labels) -> Result<Filed, Failure> {
        Ok(Filed(self.label_fields(labels).await?))
    }

    async fn upload(&self, file: Upload) -> Result<Uploaded, Failure> {
        let mut fields: Vec<(&str, &str)> = Vec::new();
        if !file.title.trim().is_empty() {
            fields.push(("title", file.title.trim()));
        }
        if let Some(created) = file.created.as_deref().filter(|d| valid_date(d)) {
            fields.push(("created", created));
        }
        for (name, id) in &file.filed.0 {
            fields.push((name, id));
        }
        let (content_type, body) = multipart(&fields, &file.file_name, &file.mime, file.bytes);
        let reply = self
            .client
            .post("/api/documents/post_document/", &content_type, body)
            .await?;
        // the answer is the id of the task that takes the file in
        Ok(match reply.json() {
            Ok(Value::String(task)) if !task.trim().is_empty() => {
                Uploaded::Processing(task.trim().to_string())
            }
            _ => Uploaded::Unknown,
        })
    }

    async fn upload_state(&self, handle: &str) -> CoreResult<TaskState> {
        let reply = self
            .client
            .get(
                "/api/tasks/",
                &[("task_id", handle.to_string())],
                MAX_TEXT_BYTES,
            )
            .await?
            .json()?;
        Ok(task_state(&reply))
    }

    async fn document_exists(&self, doc: i64) -> Result<bool, Failure> {
        match self
            .client
            .get_checked(
                &format!("/api/documents/{doc}/"),
                &[("truncate_content", "true".to_string())],
                MAX_TEXT_BYTES,
            )
            .await
        {
            Ok(reply) => {
                let meta = reply.json().map_err(|_| Failure::Unclear)?;
                // an answer about another document, or about none, says nothing
                if meta.get("id").and_then(Value::as_i64) != Some(doc) {
                    return Err(Failure::Unclear);
                }
                Ok(meta.get("deleted_at").is_none_or(Value::is_null))
            }
            Err(Failure::Status(status)) if status.as_u16() == 404 => Ok(false),
            Err(f) => Err(f),
        }
    }

    async fn original(&self, doc: i64) -> Result<Vec<u8>, Failure> {
        Ok(self
            .client
            .get_checked(
                &format!("/api/documents/{doc}/download/"),
                &first_original(doc),
                MAX_DOC_BYTES,
            )
            .await?
            .body)
    }

    async fn fact_fields(
        &self,
        wanted: &(dyn Fn() -> CoreResult<bool> + Sync),
    ) -> CoreResult<Option<[String; 4]>> {
        Ok(field_ids(&self.client, wanted)
            .await?
            .map(|ids| ids.map(|id| id.to_string())))
    }

    async fn write_facts(
        &self,
        fields: &[String; 4],
        doc: i64,
        facts: &Facts,
        carried: [bool; 4],
    ) -> Result<(), Failure> {
        let mut ids = [0i64; 4];
        for (id, field) in ids.iter_mut().zip(fields) {
            *id = field.parse().map_err(|_| Failure::NotSent)?;
        }
        let body = change(&ids, doc, facts, carried);
        match self
            .client
            .post_json("/api/documents/bulk_edit/", &body)
            .await
        {
            // an answer too long to read is still a yes
            Ok(_) | Err(Failure::TooLarge) => Ok(()),
            Err(f) => Err(f),
        }
    }
}
