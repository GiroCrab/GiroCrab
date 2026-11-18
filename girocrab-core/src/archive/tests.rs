use std::sync::{Arc, Mutex};

use rusqlite::params;

use serde_json::Value;

use super::backend::TaskState;
use super::paperless::*;
use super::*;
use crate::test_support::TempDir;

const KEY: [u8; 32] = [7u8; 32];
const TOKEN: &str = "geheimer-token-4711";

fn open(dir: &TempDir, file: &str) -> Arc<Vault> {
    Vault::open(dir.join(file).to_string_lossy().into(), KEY.to_vec()).unwrap()
}

/// One account with the bookings (external id, date, cents, counterparty).
fn seed(v: &Vault, bookings: &[(&str, &str, i64, &str)]) {
    let c = v.db().unwrap();
    c.execute_batch(
        "INSERT OR IGNORE INTO account_groups(name) VALUES ('G');
         INSERT OR IGNORE INTO accounts(group_id, external_ref, name)
             VALUES ((SELECT id FROM account_groups WHERE name='G'), 'A1', 'Konto');",
    )
    .unwrap();
    for (ext, date, cents, who) in bookings {
        c.execute(
            "INSERT INTO transactions(account_id, external_id, booking_date, amount_cents, counterparty)
             VALUES ((SELECT id FROM accounts WHERE external_ref = 'A1'), ?1, ?2, ?3, ?4)",
            params![ext, date, cents, who],
        )
        .unwrap();
    }
}

fn id_of(v: &Vault, ext: &str) -> i64 {
    v.db()
        .unwrap()
        .query_row(
            "SELECT id FROM transactions WHERE external_id = ?1",
            [ext],
            |r| r.get(0),
        )
        .unwrap()
}

/// The id of the archive `v` is connected to.
fn aid(v: &Vault) -> String {
    v.db()
        .unwrap()
        .query_row(
            "SELECT value FROM settings WHERE key = 'archive.id'",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

fn message(e: &CoreError) -> String {
    format!("{e} {e:?}")
}

// ------------------------------------------------------------------ a scripted archive

#[derive(Clone, Debug)]
struct Request {
    method: String,
    /// path with query, as sent
    target: String,
    /// header lines, lower-cased
    head: String,
    body: Vec<u8>,
}

impl Request {
    fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or_default()
    }
    fn has(&self, part: &str) -> bool {
        self.target.contains(part)
    }
}

struct Answer {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
}

fn json(body: &str) -> Answer {
    Answer {
        status: 200,
        headers: vec![("Content-Type", "application/json".into())],
        body: body.as_bytes().to_vec(),
    }
}

fn status(code: u16) -> Answer {
    Answer {
        status: code,
        headers: Vec::new(),
        body: b"{\"detail\":\"geheimnis vom server\"}".to_vec(),
    }
}

struct Archive {
    base: String,
    seen: Arc<Mutex<Vec<Request>>>,
}

impl Archive {
    fn seen(&self) -> Vec<Request> {
        self.seen.lock().unwrap().clone()
    }
}

/// A local server that answers every request with `handler`.
async fn archive(handler: impl Fn(&Request) -> Answer + Send + Sync + 'static) -> Archive {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let handler = Arc::new(handler);
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let log = log.clone();
            let handler = handler.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                let end = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
                let head = String::from_utf8_lossy(&buf[..end]).to_string();
                // the whole body, so an upload is answered only once it is in
                let length: usize = head
                    .to_lowercase()
                    .lines()
                    .find_map(|l| {
                        l.strip_prefix("content-length:")
                            .map(|v| v.trim().to_string())
                    })
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                while buf.len() < end + length {
                    match sock.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                let mut first = head.lines().next().unwrap_or_default().split(' ');
                let request = Request {
                    method: first.next().unwrap_or_default().to_string(),
                    target: first.next().unwrap_or_default().to_string(),
                    head: head.to_lowercase(),
                    body: buf[end..].to_vec(),
                };
                let answer = handler(&request);
                log.lock().unwrap().push(request);
                let mut out = format!(
                    "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n",
                    answer.status,
                    answer.body.len()
                );
                for (k, v) in &answer.headers {
                    out.push_str(&format!("{k}: {v}\r\n"));
                }
                out.push_str("\r\n");
                let _ = sock.write_all(out.as_bytes()).await;
                let _ = sock.write_all(&answer.body).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    Archive { base, seen }
}

fn connected(dir: &TempDir, archive: &Archive) -> Arc<Vault> {
    let v = open(dir, "v.db");
    v.set_archive_connection(archive.base.clone(), TOKEN.into(), None)
        .unwrap();
    v
}

const DOCS: &str = r#"{"count": 3, "next": null, "results": [
  {"id": 11, "title": "Rechnung Elektro Weber", "created": "2026-11-05", "correspondent": 4,
   "document_type": 2, "original_file_name": "rechnung_weber.pdf", "page_count": 2,
   "content": "Elektro Weber GmbH Rechnung 2026-117 Gesamtbetrag 238,00 EUR zahlbar bis 20.11.2026",
   "__search_hit__": {"score": 0.9, "highlights": "Elektro <span class=\"match\">Weber</span> GmbH", "rank": 0}},
  {"id": 12, "title": "Kassenbon", "created": "2026-11-06", "correspondent": null,
   "document_type": null, "original_file_name": "scan.pdf", "page_count": 1,
   "content": "Baumarkt Summe 19,99 EUR"},
  {"id": 13, "title": "Mietvertrag", "created": "2026-11-01", "correspondent": 4,
   "document_type": 2, "original_file_name": "miete.pdf", "page_count": null,
   "content": "Mietvertrag ohne Betrag"}
]}"#;

fn standard(r: &Request) -> Answer {
    match r.path() {
        "/api/documents/" => {
            let mut a = json(DOCS);
            a.headers.push(("X-Api-Version", "9".into()));
            a.headers.push(("X-Version", "2.20.0".into()));
            a
        }
        "/api/correspondents/" => json(r#"{"results": [{"id": 4, "name": "Elektro Weber"}]}"#),
        "/api/document_types/" => json(r#"{"results": [{"id": 2, "name": "Rechnung"}]}"#),
        "/api/documents/11/download/" => Answer {
            status: 200,
            headers: vec![
                ("Content-Type", "application/pdf".into()),
                (
                    "Content-Disposition",
                    "attachment; filename=\"../2026-11-05 Rechnung.pdf\"".into(),
                ),
            ],
            body: b"%PDF-1.4 inhalt".to_vec(),
        },
        "/api/documents/11/thumb/" => Answer {
            status: 200,
            headers: vec![("Content-Type", "image/webp".into())],
            body: vec![1, 2, 3],
        },
        _ => status(404),
    }
}

// ------------------------------------------------------------------ address and configuration

#[test]
fn addresses_are_checked_and_normalised() {
    let ok = |s: &str| {
        parse_base(s)
            .unwrap()
            .as_str()
            .trim_end_matches('/')
            .to_string()
    };
    assert_eq!(ok(" https://Archiv.Example/ "), "https://archiv.example");
    assert_eq!(ok("http://192.168.1.20:8000/"), "http://192.168.1.20:8000");
    assert_eq!(
        ok("http://nas.fritz.box:8000/paperless/?x=1#y"),
        "http://nas.fritz.box:8000/paperless"
    );
    assert_eq!(ok("http://[fd00::20]:8000"), "http://[fd00::20]:8000");
    assert_eq!(ok("https://8.8.8.8/dms"), "https://8.8.8.8/dms");
    for bad in [
        "",
        "archiv.example",
        "ftp://archiv.example",
        "https://",
        "https://name:geheim@archiv.example",
        "https://name@archiv.example",
        // unencrypted only inside the own network
        "http://8.8.8.8:8000",
        "http://[2001:db8::1]:8000",
    ] {
        assert!(
            matches!(parse_base(bad), Err(CoreError::InvalidInput { .. })),
            "{bad}"
        );
    }
}

#[test]
fn own_network_addresses() {
    let private = |s: &str| client::is_private_ip(s.parse().unwrap());
    for ip in [
        "127.0.0.1",
        "10.1.2.3",
        "172.16.0.9",
        "172.31.255.1",
        "192.168.178.20",
        "169.254.10.1",
        "100.64.0.1",
        "100.127.255.254",
        "::1",
        "fd12:3456::1",
        "fe80::1",
        "::ffff:192.168.1.5",
    ] {
        assert!(private(ip), "{ip}");
    }
    for ip in [
        "8.8.8.8",
        "172.32.0.1",
        "100.128.0.1",
        "192.169.0.1",
        "2001:db8::1",
        "::ffff:8.8.8.8",
    ] {
        assert!(!private(ip), "{ip}");
    }
}

#[test]
fn links_into_the_archive_name_its_documents() {
    let base = parse_base("http://192.168.1.20:8000/dms/").unwrap();
    let id = |s: &str| document_id_in(&base, &Url::parse(s).unwrap());
    assert_eq!(
        id("http://192.168.1.20:8000/dms/documents/123/details"),
        Some(123)
    );
    assert_eq!(id("http://192.168.1.20:8000/dms/documents/123"), Some(123));
    assert_eq!(
        id("http://192.168.1.20:8000/dms/documents/123/?x=1"),
        Some(123)
    );
    assert_eq!(
        document_url(&base, 123),
        "http://192.168.1.20:8000/dms/documents/123/details"
    );
    for other in [
        "http://192.168.1.20:8000/documents/123/details", // not under the base path
        "http://192.168.1.20:9000/dms/documents/123",     // other port
        "https://192.168.1.20:8000/dms/documents/123",    // other scheme
        "http://192.168.1.21:8000/dms/documents/123",     // other host
        "http://192.168.1.20:8000/dms/documents/",
        "http://192.168.1.20:8000/dms/documents/abc",
        "http://192.168.1.20:8000/dms/documents/12x/details",
        "http://192.168.1.20:8000/dms/documents/0",
        "http://192.168.1.20:8000/dms/share/abc",
    ] {
        assert_eq!(id(other), None, "{other}");
    }
}

#[test]
fn the_connection_is_stored_with_a_secret_token() {
    let dir = TempDir::new("archive-config");
    let v = open(&dir, "v.db");
    assert_eq!(v.archive_config().unwrap(), None);
    // a first connection needs a token
    assert!(v
        .set_archive_connection("http://192.168.1.20:8000".into(), "  ".into(), None)
        .is_err());
    assert_eq!(v.archive_config().unwrap(), None);

    let config = v
        .set_archive_connection("http://192.168.1.20:8000/".into(), TOKEN.into(), None)
        .unwrap();
    assert_eq!(
        config,
        ArchiveConfig {
            url: "http://192.168.1.20:8000".into(),
            has_token: true,
            unencrypted: true,
            linked_documents: 0,
            kind: "Paperless-ngx".into(),
        }
    );
    let stored: (String, bool) = v
        .db()
        .unwrap()
        .query_row(
            "SELECT value, is_secret FROM settings WHERE key = 'archive.token'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(stored, (TOKEN.to_string(), true));
    // the generic settings list never shows it
    assert!(v
        .list_settings()
        .unwrap()
        .iter()
        .all(|s| !s.key.starts_with("archive.") && !s.value.contains(TOKEN)));

    // a new address comes with its token
    let config = v
        .set_archive_connection("https://archiv.example".into(), "t2".into(), Some(aid(&v)))
        .unwrap();
    assert!(config.has_token && !config.unencrypted);

    v.remove_archive_connection().unwrap();
    assert_eq!(v.archive_config().unwrap(), None);
    assert_eq!(
        v.db()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM settings WHERE key LIKE 'archive.%'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

/// The kind of archive is part of the connection. One this version does not
/// know (connected by a newer one) is not talked to as if it were another.
#[tokio::test]
async fn the_kind_of_archive_is_stored_with_the_connection() {
    use super::backend::Kind;
    assert_eq!(Kind::of("paperless"), Some(Kind::Paperless));
    assert_eq!(Kind::of("etwas-neues"), None);
    assert!(Kind::ALL
        .iter()
        .all(|k| Kind::of(k.id()) == Some(*k) && !k.name().is_empty()));

    let dir = TempDir::new("archive-kind");
    let server = archive(standard).await;
    let v = connected(&dir, &server);
    seed(&v, &[("T1", "2026-11-07", -23800, "Elektro Weber")]);
    let kind = |v: &Vault| -> Option<String> {
        v.db()
            .unwrap()
            .query_row(
                "SELECT value FROM settings WHERE key = 'archive.kind'",
                [],
                |r| r.get(0),
            )
            .ok()
    };
    assert_eq!(kind(&v).as_deref(), Some("paperless"));
    assert_eq!(v.archive_config().unwrap().unwrap().kind, "Paperless-ngx");
    let archive_id = aid(&v);
    let linked = v
        .attach_archive_document(id_of(&v, "T1"), archive_id.clone(), 11, "Rechnung".into())
        .unwrap();

    // a connection from before the note existed is the first kind there was
    v.db()
        .unwrap()
        .execute("DELETE FROM settings WHERE key = 'archive.kind'", [])
        .unwrap();
    assert!(v.clone().archive_test().await.is_ok());
    assert_eq!(
        v.list_documents_for_booking(id_of(&v, "T1")).unwrap()[0].archive_doc,
        Some(11)
    );

    // a kind this version does not know: nothing is sent, nothing is guessed
    v.db()
        .unwrap()
        .execute(
            "INSERT INTO settings(key, value) VALUES ('archive.kind', 'etwas-neues')",
            [],
        )
        .unwrap();
    let before = server.seen().len();
    let refused = v.clone().archive_test().await.unwrap_err();
    assert!(matches!(refused, CoreError::InvalidInput { .. }));
    assert!(v.clone().archive_file(linked, false).await.is_err());
    assert!(v.clone().process_archive_moves().await.is_err());
    assert_eq!(server.seen().len(), before);
    assert_eq!(v.archive_config().unwrap().unwrap().kind, "");
    // the reference is still there, under the address it was stored with
    let doc = v
        .list_documents_for_booking(id_of(&v, "T1"))
        .unwrap()
        .remove(0);
    assert_eq!(doc.archive_doc, Some(11));
    assert!(doc.link_url.ends_with("/documents/11/details"));

    // gone with the connection, back with the next
    v.remove_archive_connection().unwrap();
    assert_eq!(kind(&v), None);
    v.set_archive_connection(server.base.clone(), TOKEN.into(), Some(archive_id))
        .unwrap();
    assert_eq!(kind(&v).as_deref(), Some("paperless"));
}

/// The token was entered for one address of one archive. It is never sent
/// to another address, and never to what is said to be another archive.
#[test]
fn the_stored_token_goes_to_no_other_address_and_no_other_archive() {
    let dir = TempDir::new("archive-token-bound");
    let v = open(&dir, "v.db");
    v.set_archive_connection("https://archiv.example".into(), TOKEN.into(), None)
        .unwrap();
    let a = aid(&v);
    let stored = |v: &Vault| -> (String, String) {
        let db = v.db().unwrap();
        let get = |key: &str| {
            db.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                r.get::<_, String>(0)
            })
            .unwrap()
        };
        (get("archive.url"), get("archive.token"))
    };
    // the same address of the same archive: the token stays
    v.set_archive_connection(
        "https://archiv.example/".into(),
        String::new(),
        Some(a.clone()),
    )
    .unwrap();
    assert_eq!(stored(&v), ("https://archiv.example".into(), TOKEN.into()));

    // a new address, even of "the same" archive; the same address as a new archive
    for (url, id) in [
        ("https://neu.example", Some(a.clone())),
        ("https://archiv.example:8443", Some(a.clone())),
        ("https://archiv.example", None),
    ] {
        let refused = v.set_archive_connection(url.into(), "  ".into(), id);
        assert!(
            matches!(refused, Err(CoreError::InvalidInput { .. })),
            "{url}"
        );
        assert_eq!(aid(&v), a);
        assert_eq!(stored(&v), ("https://archiv.example".into(), TOKEN.into()));
    }
    // with its token, a new address is fine
    v.set_archive_connection("https://neu.example".into(), "t2".into(), Some(a.clone()))
        .unwrap();
    assert_eq!(stored(&v), ("https://neu.example".into(), "t2".into()));
    assert_eq!(aid(&v), a);
}

#[test]
fn a_pasted_archive_address_becomes_an_archive_document() {
    let dir = TempDir::new("archive-paste");
    let v = open(&dir, "v.db");
    seed(
        &v,
        &[
            ("T1", "2026-11-07", -23800, "Elektro Weber"),
            ("T2", "2026-11-08", -1999, "Baumarkt"),
        ],
    );
    let (t1, t2) = (id_of(&v, "T1"), id_of(&v, "T2"));
    v.set_archive_connection("http://192.168.1.20:8000".into(), TOKEN.into(), None)
        .unwrap();

    v.attach_document_link(
        t1,
        "http://192.168.1.20:8000/documents/42/details".into(),
        String::new(),
    )
    .unwrap();
    v.attach_document_link(
        t1,
        "https://anderswo.example/documents/42".into(),
        String::new(),
    )
    .unwrap();
    let docs = v.list_documents_for_booking(t1).unwrap();
    let archive_docs: Vec<Option<i64>> = docs.iter().map(|d| d.archive_doc).collect();
    assert!(
        archive_docs.contains(&Some(42)) && archive_docs.contains(&None),
        "{archive_docs:?}"
    );
    assert_eq!(v.archive_config().unwrap().unwrap().linked_documents, 1);

    // the same document once per booking, however it is named
    assert!(v
        .attach_archive_document(t1, aid(&v), 42, "Rechnung".into())
        .is_err());
    assert!(v
        .attach_document_link(
            t1,
            "http://192.168.1.20:8000/documents/42".into(),
            String::new()
        )
        .is_err());
    let second = v
        .attach_archive_document(t2, aid(&v), 42, "  Rechnung  Weber ".into())
        .unwrap();
    let doc = v
        .list_documents_for_booking(t2)
        .unwrap()
        .into_iter()
        .find(|d| d.id == second)
        .unwrap();
    assert_eq!(doc.title, "Rechnung Weber");
    assert_eq!(
        doc.link_url,
        "http://192.168.1.20:8000/documents/42/details"
    );
    assert_eq!(doc.doc_date, "2026-11-08");
    assert!(v
        .attach_archive_document(t2, aid(&v), 0, "x".into())
        .is_err());
}

#[test]
fn a_new_address_of_the_same_archive_keeps_the_documents_another_archive_does_not() {
    let dir = TempDir::new("archive-move");
    let v = open(&dir, "v.db");
    seed(&v, &[("T1", "2026-11-07", -23800, "Elektro Weber")]);
    let t1 = id_of(&v, "T1");
    let first = "http://192.168.1.20:8000";
    v.set_archive_connection(first.into(), TOKEN.into(), None)
        .unwrap();
    let old_archive = aid(&v);
    v.attach_archive_document(t1, old_archive.clone(), 42, "Rechnung".into())
        .unwrap();
    let rechnung = |v: &Vault| {
        v.list_documents_for_booking(t1)
            .unwrap()
            .into_iter()
            .find(|d| d.title == "Rechnung")
            .unwrap()
    };

    // the same archive, now reached through a VPN name: the link follows
    v.set_archive_connection(
        "https://archiv.example".into(),
        "t2".into(),
        Some(old_archive.clone()),
    )
    .unwrap();
    assert_eq!(aid(&v), old_archive);
    assert_eq!(rechnung(&v).archive_doc, Some(42));
    assert_eq!(
        rechnung(&v).link_url,
        "https://archiv.example/documents/42/details"
    );
    assert_eq!(v.archive_documents_elsewhere().unwrap(), 0);

    // another archive: id 42 means nothing there. The reference is a plain
    // link again, to the address it was made with; nothing was rewritten.
    v.set_archive_connection("https://neu.example".into(), "t2".into(), None)
        .unwrap();
    assert_ne!(aid(&v), old_archive);
    assert_eq!(rechnung(&v).archive_doc, None);
    assert_eq!(
        rechnung(&v).link_url,
        format!("{first}/documents/42/details")
    );
    assert_eq!(v.archive_config().unwrap().unwrap().linked_documents, 0);
    assert_eq!(v.archive_documents_elsewhere().unwrap(), 1);
    // a result of the old connection is refused, the same document id may be linked anew
    assert!(v
        .attach_archive_document(t1, old_archive.clone(), 43, "alt".into())
        .is_err());
    v.attach_archive_document(t1, aid(&v), 42, "Bon".into())
        .unwrap();
    assert_eq!(v.archive_config().unwrap().unwrap().linked_documents, 1);

    // disconnected: every reference is a plain link, none is lost
    v.remove_archive_connection().unwrap();
    let docs = v.list_documents_for_booking(t1).unwrap();
    assert_eq!(docs.len(), 2);
    assert!(docs.iter().all(|d| d.archive_doc.is_none()));
    assert!(docs
        .iter()
        .any(|d| d.link_url == "https://neu.example/documents/42/details"));
    assert_eq!(v.archive_documents_elsewhere().unwrap(), 2);
}

/// A vault whose references belong to two archives (after a change on some
/// device). Which of them an address is must be said, never guessed: the one
/// with more references is not automatically the right one.
#[test]
fn the_archive_an_address_is_must_be_chosen_among_those_the_vault_knows() {
    let dir = TempDir::new("archive-identities");
    let v = open(&dir, "v.db");
    seed(
        &v,
        &[
            ("T1", "2026-11-07", -100, "A"),
            ("T2", "2026-11-08", -200, "B"),
            ("T3", "2026-11-09", -300, "C"),
        ],
    );
    assert!(v.archive_identities().unwrap().is_empty());
    v.set_archive_connection("http://192.168.1.20:8000".into(), TOKEN.into(), None)
        .unwrap();
    let a = aid(&v);
    // connected, no reference yet: it is listed all the same
    assert_eq!(
        v.archive_identities().unwrap(),
        [ArchiveIdentity {
            id: a.clone(),
            documents: 0,
            host: String::new(),
            connected: true
        }]
    );
    v.attach_archive_document(id_of(&v, "T1"), a.clone(), 1, "A1".into())
        .unwrap();
    v.attach_archive_document(id_of(&v, "T2"), a.clone(), 2, "A2".into())
        .unwrap();
    v.set_archive_connection("https://neu.example".into(), "t2".into(), None)
        .unwrap();
    let b = aid(&v);
    v.attach_archive_document(id_of(&v, "T3"), b.clone(), 1, "B1".into())
        .unwrap();
    // as a device sees it that is not connected yet
    v.remove_archive_connection().unwrap();
    let known = v.archive_identities().unwrap();
    assert_eq!(
        known
            .iter()
            .map(|i| (i.id.as_str(), i.documents, i.host.as_str(), i.connected))
            .collect::<Vec<_>>(),
        [
            (a.as_str(), 2, "192.168.1.20", false),
            (b.as_str(), 1, "neu.example", false)
        ]
    );

    // it connects the newer archive and says so: its document, not the older ones
    v.set_archive_connection("https://neu.example".into(), TOKEN.into(), Some(b.clone()))
        .unwrap();
    assert_eq!(aid(&v), b);
    let docs = |ext: &str| {
        v.list_documents_for_booking(id_of(&v, ext))
            .unwrap()
            .remove(0)
    };
    assert_eq!(docs("T3").archive_doc, Some(1));
    assert_eq!(
        docs("T1").archive_doc,
        None,
        "document 1 of the other archive"
    );
    assert_eq!(v.archive_identities().unwrap()[0].id, b);
    assert!(v.archive_identities().unwrap()[0].connected);

    // an id the vault does not know is refused, the connection stays as it is
    assert!(v
        .set_archive_connection(
            "https://x.example".into(),
            String::new(),
            Some("erfunden".into())
        )
        .is_err());
    assert_eq!(aid(&v), b);
    assert_eq!(
        v.archive_config().unwrap().unwrap().url,
        "https://neu.example"
    );
}

// ------------------------------------------------------------------ requests

#[tokio::test]
async fn without_a_connection_nothing_is_sent() {
    let server = archive(standard).await;
    let dir = TempDir::new("archive-off");
    let v = open(&dir, "v.db");
    seed(&v, &[("T1", "2026-11-07", -23800, "Elektro Weber")]);
    let t1 = id_of(&v, "T1");
    assert!(matches!(
        v.clone().archive_test().await,
        Err(CoreError::MissingSetting { .. })
    ));
    assert!(v
        .clone()
        .archive_search("x".into(), None, None, 10)
        .await
        .is_err());
    assert!(v.clone().archive_suggestions(t1, 5).await.is_err());
    assert!(v.clone().archive_thumbnail(11).await.is_err());
    assert!(server.seen().is_empty());
}

#[tokio::test]
async fn the_test_reports_what_the_archive_says_and_sends_the_token_only_there() {
    let server = archive(standard).await;
    let dir = TempDir::new("archive-test");
    let v = connected(&dir, &server);
    let info = v.clone().archive_test().await.unwrap();
    assert_eq!(
        (
            info.document_count,
            info.api_version.as_str(),
            info.server_version.as_str()
        ),
        (3, "9", "2.20.0")
    );
    let seen = server.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "GET");
    assert!(seen[0]
        .head
        .contains(&format!("authorization: token {TOKEN}")));
    assert!(seen[0].head.contains("accept: application/json; version=9"));
}

#[tokio::test]
async fn search_names_correspondents_and_marks_linked_documents() {
    let server = archive(standard).await;
    let dir = TempDir::new("archive-search");
    let v = connected(&dir, &server);
    seed(&v, &[("T1", "2026-11-07", -23800, "Elektro Weber")]);
    v.attach_archive_document(id_of(&v, "T1"), aid(&v), 12, "Kassenbon".into())
        .unwrap();

    let docs = v
        .clone()
        .archive_search(
            " Weber & Söhne ".into(),
            Some("2026-11-01".into()),
            Some("2026-11-30".into()),
            25,
        )
        .await
        .unwrap();
    assert_eq!(docs.iter().map(|d| d.id).collect::<Vec<_>>(), [11, 12, 13]);
    let first = &docs[0];
    assert_eq!(
        (
            first.title.as_str(),
            first.created.as_str(),
            first.page_count
        ),
        ("Rechnung Elektro Weber", "2026-11-05", 2)
    );
    assert_eq!(
        (first.correspondent.as_str(), first.document_type.as_str()),
        ("Elektro Weber", "Rechnung")
    );
    assert_eq!(first.excerpt, "Elektro Weber GmbH");
    assert_eq!(docs[1].excerpt, "Baumarkt Summe 19,99 EUR");
    assert_eq!(
        docs.iter().map(|d| d.linked).collect::<Vec<_>>(),
        [false, true, false]
    );

    let seen = server.seen();
    let list = &seen[0];
    assert!(list.has("query=Weber+%26+S%C3%B6hne"), "{}", list.target);
    assert!(list.has("created__date__gte=2026-11-01") && list.has("created__date__lte=2026-11-30"));
    assert!(list.has("page_size=25") && list.has("truncate_content=true"));
    // exactly the ids the results use
    assert!(seen
        .iter()
        .any(|r| r.path() == "/api/correspondents/" && r.has("id__in=4")));
    assert!(seen
        .iter()
        .any(|r| r.path() == "/api/document_types/" && r.has("id__in=2")));

    // no search term: newest first; a malformed date is refused before any request
    let before = server.seen().len();
    v.clone()
        .archive_search(String::new(), None, None, 500)
        .await
        .unwrap();
    let newest = &server.seen()[before];
    assert!(newest.has("ordering=-created") && newest.has("page_size=50") && !newest.has("query="));
    let before = server.seen().len();
    assert!(v
        .clone()
        .archive_search("x".into(), Some("5.11.2026".into()), None, 10)
        .await
        .is_err());
    assert_eq!(server.seen().len(), before);
}

#[tokio::test]
async fn suggestions_score_the_text_of_nearby_documents() {
    let server = archive(standard).await;
    let dir = TempDir::new("archive-suggest");
    let v = connected(&dir, &server);
    seed(
        &v,
        &[
            ("T1", "2026-11-07", -23800, "Elektro Weber"),
            ("T2", "2026-11-08", -1999, "Baumarkt Nord"),
            ("T3", "2026-11-09", -555, "Unbekannt"),
        ],
    );
    let found = v
        .clone()
        .archive_suggestions(id_of(&v, "T1"), 5)
        .await
        .unwrap();
    assert_eq!(
        found.iter().map(|s| s.document.id).collect::<Vec<_>>(),
        [11]
    );
    // amount (5) + merchant (3) + date within three days (3)
    assert_eq!(found[0].score, 11);
    assert_eq!(found[0].document.correspondent, "Elektro Weber");

    let window = &server.seen()[0];
    assert!(
        window.has("created__date__gte=2026-09-23") && window.has("created__date__lte=2026-11-17")
    );
    assert!(!window.has("truncate_content"), "the full text is scored");

    let found = v
        .clone()
        .archive_suggestions(id_of(&v, "T2"), 5)
        .await
        .unwrap();
    assert_eq!(
        found.iter().map(|s| s.document.id).collect::<Vec<_>>(),
        [12]
    );
    assert!(v
        .clone()
        .archive_suggestions(id_of(&v, "T3"), 5)
        .await
        .unwrap()
        .is_empty());

    // a document that already hangs on the booking is not suggested again
    v.attach_archive_document(id_of(&v, "T1"), aid(&v), 11, "Rechnung".into())
        .unwrap();
    assert!(v
        .clone()
        .archive_suggestions(id_of(&v, "T1"), 5)
        .await
        .unwrap()
        .is_empty());
}

/// The app has locked the vault. A call that was already on its way into
/// the core, or work that was under way, sends nothing any more.
#[tokio::test]
async fn a_locked_vault_sends_nothing_whatever_is_still_called() {
    let dir = TempDir::new("archive-closed");
    let server = archive(standard).await;
    let v = connected(&dir, &server);
    seed(
        &v,
        &[
            ("T1", "2026-11-07", -23800, "Elektro Weber"),
            ("T2", "2026-11-08", -1999, "Baumarkt Nord"),
        ],
    );
    let archive = aid(&v);
    v.attach_archive_document(id_of(&v, "T1"), archive, 11, "Rechnung".into())
        .unwrap();
    assert!(v.clone().archive_test().await.is_ok());
    let before = server.seen().len();

    v.archive_close();
    v.archive_stop(); // (stays closed)
    assert!(v.clone().archive_test().await.is_err());
    assert!(v
        .clone()
        .archive_search("x".into(), None, None, 5)
        .await
        .is_err());
    assert!(v.clone().archive_matches(None, None, 5).await.is_err());
    let _ = v.clone().process_archive_moves().await;
    assert_eq!(server.seen().len(), before, "not one request");
}

/// Many bookings without a receipt, one pass over the archive: each gets
/// the document that fits best, no booking and no document twice, and what
/// is not clear-cut says so. Nothing is linked by looking.
#[tokio::test]
async fn bookings_without_a_receipt_are_matched_in_one_pass() {
    let dir = TempDir::new("archive-matches");
    let server = archive(standard).await;
    let v = connected(&dir, &server);
    seed(
        &v,
        &[
            ("T1", "2026-11-07", -23800, "Elektro Weber"),
            ("T2", "2026-11-08", -1999, "Baumarkt Nord"),
            // the same amount a day later: a rival for the same receipt
            ("T3", "2026-11-09", -1999, "Baumarkt Nord"),
            ("T4", "2026-11-07", -5000, "Niemand"),
            // has a receipt already: not asked about
            ("T5", "2026-11-06", -23800, "Elektro Weber"),
            // the right amount, months away from the document
            ("T6", "2026-01-10", -23800, "Elektro Weber"),
        ],
    );
    v.attach_document_link(
        id_of(&v, "T5"),
        "https://shop.example/r/1".into(),
        "R".into(),
    )
    .unwrap();

    let found = v.clone().archive_matches(None, None, 50).await.unwrap();
    assert_eq!(
        (found.open_bookings, found.documents, found.more),
        (5, 3, false)
    );
    let pairs: Vec<(i64, i64, bool)> = found
        .matches
        .iter()
        .map(|m| (m.booking_id, m.document.id, m.ambiguous))
        .collect();
    // "Elektro Weber" stands in its invoice; of "Baumarkt Nord" the receipt
    // only says "Baumarkt", which is not the name
    assert_eq!(
        found
            .matches
            .iter()
            .map(|m| m.merchant_fits)
            .collect::<Vec<_>>(),
        vec![false, true]
    );
    // newest booking first; 12 goes to the nearer of the two rivals and is marked
    assert_eq!(
        pairs,
        vec![(id_of(&v, "T2"), 12, true), (id_of(&v, "T1"), 11, false)]
    );
    assert_eq!(found.matches[1].document.correspondent, "Elektro Weber");
    assert!(found.matches[1].booking.contains("2026-11-07"));
    assert!(found
        .matches
        .iter()
        .all(|m| m.score >= 5 && m.document.archive_id == aid(&v)));
    // one question for the whole period, 45 days before the oldest booking
    // to 10 after the newest
    let asked: Vec<Request> = server
        .seen()
        .into_iter()
        .filter(|r| r.path() == "/api/documents/")
        .collect();
    assert_eq!(asked.len(), 1);
    assert!(asked[0].has("created__date__gte=2025-11-26"));
    assert!(asked[0].has("created__date__lte=2026-11-19"));
    assert_eq!(
        v.db()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM document_links WHERE archive_doc IS NOT NULL",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0,
        "looking links nothing"
    );

    // the period narrows the bookings
    let found = v
        .clone()
        .archive_matches(Some("2026-11-09".into()), Some("2026-11-30".into()), 50)
        .await
        .unwrap();
    assert_eq!(found.open_bookings, 1);
    assert_eq!(
        found
            .matches
            .iter()
            .map(|m| (m.booking_id, m.document.id, m.ambiguous))
            .collect::<Vec<_>>(),
        vec![(id_of(&v, "T3"), 12, false)]
    );

    // accepted: the booking is no longer open and the document no longer offered
    let archive = aid(&v);
    v.attach_archive_document(id_of(&v, "T1"), archive.clone(), 11, "Rechnung".into())
        .unwrap();
    v.attach_archive_document(id_of(&v, "T2"), archive, 12, "Bon".into())
        .unwrap();
    let found = v.clone().archive_matches(None, None, 50).await.unwrap();
    assert_eq!(found.open_bookings, 3);
    assert!(found.matches.is_empty(), "12 is taken, so T3 gets nothing");
}

/// A page may hold fewer documents than asked for and still not be the
/// last: the archive says whether one follows.
#[tokio::test]
async fn a_short_page_is_not_the_last_while_the_archive_names_a_next_one() {
    let dir = TempDir::new("archive-matches-pages");
    let server = archive(|r| {
        if r.path() != "/api/documents/" {
            return status(404);
        }
        if r.has("page=1") {
            json(r#"{"count": 2, "next": "http://anderswo.example/api/documents/?page=2", "results": [
                {"id": 31, "title": "Bon", "created": "2026-11-06", "content": "nichts davon"}]}"#)
        } else {
            json(r#"{"count": 2, "next": null, "results": [
                {"id": 32, "title": "Rechnung", "created": "2026-11-05",
                 "content": "Elektro Weber GmbH Gesamtbetrag 238,00 EUR"}]}"#)
        }
    })
    .await;
    let v = connected(&dir, &server);
    seed(&v, &[("T1", "2026-11-07", -23800, "Elektro Weber")]);
    let found = v.clone().archive_matches(None, None, 50).await.unwrap();
    assert_eq!(found.documents, 2);
    assert_eq!(found.matches.len(), 1);
    assert_eq!(found.matches[0].document.id, 32);
    assert!(found.matches[0]
        .document
        .url
        .ends_with("/documents/32/details"));
    assert!(found.matches[0].document.url.starts_with(&server.base));
    // the next page is asked for by number, at the configured address
    let pages: Vec<String> = server.seen().iter().map(|r| r.target.clone()).collect();
    assert!(pages.iter().any(|t| t.contains("page=2")));
    assert!(pages.iter().all(|t| !t.contains("anderswo")));
}

/// What the user accepts is looked at again when it is written: by then the
/// booking may have a receipt, the document a booking, or the connection may
/// be another one.
#[test]
fn accepted_matches_are_checked_again_as_they_are_linked() {
    let dir = TempDir::new("archive-matches-accept");
    let v = open(&dir, "v.db");
    v.set_archive_connection("https://archiv.example".into(), TOKEN.into(), None)
        .unwrap();
    seed(
        &v,
        &[
            ("T1", "2026-11-07", -23800, "Elektro Weber"),
            ("T2", "2026-11-08", -1999, "Baumarkt Nord"),
            ("T3", "2026-11-09", -1999, "Baumarkt Nord"),
            ("T4", "2026-11-09", -500, "Bäcker"),
        ],
    );
    let archive = aid(&v);
    let ids: std::collections::HashMap<&str, i64> = ["T1", "T2", "T3", "T4"]
        .into_iter()
        .map(|ext| (ext, id_of(&v, ext)))
        .collect();
    let pick = |ext: &str, doc: i64, archive: &str| ArchiveMatch {
        booking_id: ids[ext],
        booking: String::new(),
        document: ArchiveDocument {
            id: doc,
            archive_id: archive.to_string(),
            title: format!("  Rechnung   {doc} "),
            created: "2026-11-05".into(),
            correspondent: String::new(),
            document_type: String::new(),
            file_name: String::new(),
            page_count: 1,
            excerpt: String::new(),
            linked: false,
            url: String::new(),
        },
        score: 11,
        merchant_fits: true,
        ambiguous: false,
    };
    // T2 got a receipt meanwhile, document 13 a booking
    v.attach_document_link(ids["T2"], "https://shop.example/r/2".into(), "R".into())
        .unwrap();
    v.attach_archive_document(ids["T4"], archive.clone(), 13, "Schon da".into())
        .unwrap();
    let linked = v
        .attach_archive_matches(vec![
            pick("T1", 11, &archive),
            pick("T2", 12, &archive),      // the booking has a receipt
            pick("T3", 13, &archive),      // the document has a booking
            pick("T3", 11, &archive),      // 11 went to T1 a moment ago
            pick("T3", 14, "ein-anderes"), // a match of another connection
            pick("T3", 0, &archive),
        ])
        .unwrap();
    assert_eq!(linked, 1);
    let t1 = v.list_documents_for_booking(ids["T1"]).unwrap();
    assert_eq!(t1.len(), 1);
    assert_eq!(
        (t1[0].archive_doc, t1[0].title.as_str()),
        (Some(11), "Rechnung 11")
    );
    assert!(v.list_documents_for_booking(ids["T3"]).unwrap().is_empty());
    assert_eq!(v.list_documents_for_booking(ids["T2"]).unwrap().len(), 1);

    // without a connection nothing is linked
    v.remove_archive_connection().unwrap();
    assert_eq!(
        v.attach_archive_matches(vec![pick("T3", 14, &archive)])
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn files_come_with_type_and_a_plain_name() {
    let server = archive(standard).await;
    let dir = TempDir::new("archive-file");
    let v = connected(&dir, &server);
    seed(&v, &[("T1", "2026-11-07", -23800, "Elektro Weber")]);
    let t1 = id_of(&v, "T1");
    let doc = v
        .attach_archive_document(t1, aid(&v), 11, "Rechnung".into())
        .unwrap();
    let plain = v
        .attach_document_link(t1, "https://anderswo.example/x".into(), String::new())
        .unwrap();

    let file = v.clone().archive_file(doc, false).await.unwrap();
    assert_eq!(file.bytes, b"%PDF-1.4 inhalt");
    assert_eq!(file.mime_type, "application/pdf");
    assert_eq!(
        file.file_name, "2026-11-05 Rechnung.pdf",
        "no folders from the header"
    );
    v.clone().archive_file(doc, true).await.unwrap();
    let seen = server.seen();
    assert_eq!(seen[0].target, "/api/documents/11/download/");
    assert_eq!(seen[1].target, "/api/documents/11/download/?original=true");

    // a plain link and a stored file have nothing to fetch
    let before = server.seen().len();
    assert!(v.clone().archive_file(plain, false).await.is_err());
    assert!(v.clone().archive_file(5, false).await.is_err());
    assert_eq!(server.seen().len(), before);

    assert_eq!(v.clone().archive_thumbnail(11).await.unwrap(), [1, 2, 3]);
    assert_eq!(file_name_of("", 7, "image/png"), "dokument-7.png");
    // RFC 5987: decoded first, then reduced to a plain name; it wins over `filename=`
    assert_eq!(
        file_name_of(
            "inline; filename=\"x.pdf\"; filename*=UTF-8''Beleg%20Nr%C3%BC1.pdf",
            7,
            ""
        ),
        "Beleg Nrü1.pdf"
    );
    assert_eq!(
        file_name_of("attachment; filename*=UTF-8''..%2F..%2Fetc%2Fpasswd", 7, ""),
        "passwd"
    );
    assert_eq!(
        file_name_of("attachment; filename*=UTF-8'de'Beleg%20Nr1.pdf", 7, ""),
        "Beleg Nr1.pdf"
    );
    assert_eq!(percent_decoded("100%25%zz%4"), "100%%zz%4");

    // the connection changed since the link was made: its id is not fetched from the new archive
    v.set_archive_connection(server.base.clone(), TOKEN.into(), None)
        .unwrap();
    let before = server.seen().len();
    assert!(matches!(
        v.clone().archive_file(doc, false).await,
        Err(CoreError::NotFound { .. })
    ));
    assert_eq!(server.seen().len(), before);
}

#[tokio::test]
async fn an_oversized_answer_is_cut_off() {
    let server = archive(|_| Answer {
        status: 200,
        headers: Vec::new(),
        body: vec![0u8; MAX_THUMB_BYTES + 1],
    })
    .await;
    let dir = TempDir::new("archive-big");
    let v = connected(&dir, &server);
    assert!(matches!(
        v.clone().archive_thumbnail(11).await,
        Err(CoreError::Network { .. })
    ));
}

#[tokio::test]
async fn errors_say_what_to_do_and_never_what_was_sent() {
    for (code, check) in [
        (401u16, "auth"),
        (403, "auth"),
        (404, "missing"),
        (406, "protocol"),
        (302, "protocol"),
        (400, "refused"),
        (500, "network"),
    ] {
        let server = archive(move |_| {
            let mut a = status(code);
            if code == 302 {
                a.headers
                    .push(("Location", "http://127.0.0.1:9/woanders".into()));
            }
            a
        })
        .await;
        let dir = TempDir::new("archive-errors");
        let v = connected(&dir, &server);
        let err = v.clone().archive_test().await.unwrap_err();
        let kind = match &err {
            CoreError::Authentication { .. } => "auth",
            CoreError::NotFound { .. } => "missing",
            CoreError::Protocol { .. } => "protocol",
            CoreError::Network { .. } => "network",
            CoreError::InvalidInput { .. } => "refused",
            _ => "other",
        };
        assert_eq!(kind, check, "HTTP {code}: {err:?}");
        let text = message(&err);
        for secret in [
            TOKEN,
            "127.0.0.1",
            "geheimnis vom server",
            "woanders",
            "/api/",
        ] {
            assert!(!text.contains(secret), "HTTP {code} leaks {secret}: {text}");
        }
        // a redirect is never followed
        assert_eq!(server.seen().len(), 1, "HTTP {code}");
    }

    // nobody listening
    let dir = TempDir::new("archive-down");
    let v = open(&dir, "v.db");
    v.set_archive_connection("http://127.0.0.1:9".into(), TOKEN.into(), None)
        .unwrap();
    let err = v.clone().archive_test().await.unwrap_err();
    assert!(matches!(err, CoreError::Network { .. }), "{err:?}");
    let text = message(&err);
    assert!(
        !text.contains("127.0.0.1") && !text.contains(TOKEN),
        "{text}"
    );

    // an answer that is not what the API promises
    let server = archive(|_| json("[1, 2, 3]")).await;
    let dir = TempDir::new("archive-odd");
    let v = connected(&dir, &server);
    assert!(matches!(
        v.clone().archive_test().await,
        Err(CoreError::Protocol { .. })
    ));
}

#[test]
fn excerpts_are_plain_and_short() {
    let doc: Value = serde_json::from_str(
        r#"{"content": "eins zwei", "__search_hit__": {"highlights": "a <span class=\"match\">b</span> c<br/>d"}}"#,
    )
    .unwrap();
    assert_eq!(excerpt_of(&doc), "a b cd");
    let long: Value = serde_json::json!({ "content": "wort ".repeat(200) });
    let text = excerpt_of(&long);
    assert!(text.chars().count() <= 221 && text.ends_with('…'), "{text}");
}

/// A token may read documents but not the lists of correspondents and
/// types: the documents are found all the same, without those names.
#[tokio::test]
async fn documents_are_found_without_the_right_to_read_names() {
    let server = archive(|r| match r.path() {
        "/api/correspondents/" | "/api/document_types/" => status(403),
        _ => standard(r),
    })
    .await;
    let dir = TempDir::new("archive-names");
    let v = connected(&dir, &server);
    seed(&v, &[("T1", "2026-11-07", -23800, "Elektro Weber")]);
    let docs = v
        .clone()
        .archive_search("weber".into(), None, None, 10)
        .await
        .unwrap();
    assert_eq!(docs.len(), 3);
    assert!(docs
        .iter()
        .all(|d| d.correspondent.is_empty() && d.document_type.is_empty()));
    assert!(docs.iter().all(|d| d.archive_id == aid(&v)));
    let found = v
        .clone()
        .archive_suggestions(id_of(&v, "T1"), 5)
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
}

// ------------------------------------------------------------------ moving files into the archive

mod moving {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::super::moves::*;
    use super::*;
    use crate::documents::{DocumentFilter, DocumentInfo};
    use crate::models::BookingFilter;

    const PDF_NAME: &str = "Rechnung Weber.pdf";

    fn pdf() -> Vec<u8> {
        crate::documents::simple_pdf(&["Rechnung Elektro Weber".into(), "238,00 EUR".into()])
    }

    /// A vault connected to `server` with booking T1 and one stored file on it.
    fn with_file(dir: &TempDir, server: &Archive) -> (Arc<Vault>, i64, i64) {
        let v = connected(dir, server);
        seed(
            &v,
            &[
                ("T1", "2026-11-07", -23800, "Elektro Weber"),
                ("T2", "2026-11-09", -500, "Bäcker"),
            ],
        );
        let t1 = id_of(&v, "T1");
        let doc = v
            .attach_document_to_booking(t1, PDF_NAME.into(), pdf())
            .unwrap();
        (v, t1, doc)
    }

    fn count(v: &Vault, sql: &str) -> i64 {
        v.db().unwrap().query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn document_count(v: &Vault, ext: &str) -> u32 {
        let id = id_of(v, ext);
        v.list_transactions(BookingFilter::default())
            .unwrap()
            .transactions
            .into_iter()
            .find(|b| b.id == id)
            .unwrap()
            .document_count
    }

    /// An archive that takes uploads and answers the task with `answers`,
    /// one after the other (the last one again and again).
    async fn taking(answers: Vec<&'static str>) -> (Archive, Arc<AtomicUsize>) {
        let asked = Arc::new(AtomicUsize::new(0));
        let n = asked.clone();
        let server = archive(move |r| match (r.method.as_str(), r.path()) {
            ("POST", "/api/documents/post_document/") => json("\"task-1\""),
            ("GET", "/api/tasks/") => {
                let i = n.fetch_add(1, Ordering::SeqCst).min(answers.len() - 1);
                json(answers[i])
            }
            // nothing with the file's checksum yet
            ("GET", "/api/documents/") => json(r#"{"count": 0, "results": []}"#),
            // the document a task names can be read, and its file is ours
            ("GET", p) if p.starts_with("/api/documents/") && p.ends_with("/download/") => Answer {
                status: 200,
                headers: vec![("Content-Type", "application/pdf".into())],
                body: pdf(),
            },
            ("GET", p) if p.starts_with("/api/documents/") => {
                let id = p
                    .trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .unwrap_or_default();
                json(&format!(
                    r#"{{"id": {id}, "title": "x", "deleted_at": null}}"#
                ))
            }
            _ => status(404),
        })
        .await;
        (server, asked)
    }

    const TRASHED: &str = r#"[{"task_id": "task-1", "status": "FAILURE", "result": "duplicate", "related_document": 9, "duplicate_documents": [{"id": 9, "title": "x", "deleted_at": "2026-11-01T10:00:00Z"}]}]"#;

    /// The move of `doc` as a run would take it up now.
    fn claim_of(v: &Vault, doc: i64) -> Claim {
        v.db()
            .unwrap()
            .query_row(
                "SELECT doc_id, op, archive_id, task FROM archive_moves WHERE doc_id = ?1",
                [doc],
                |r| {
                    Ok(Claim {
                        doc_id: r.get(0)?,
                        op: r.get(1)?,
                        archive_id: r.get(2)?,
                        task: r.get(3)?,
                    })
                },
            )
            .unwrap()
    }

    fn base_of_vault(v: &Vault) -> Url {
        super::super::base_of(&v.db().unwrap()).unwrap().unwrap()
    }

    const PENDING: &str =
        r#"[{"task_id": "task-1", "status": "PENDING", "result": null, "related_document": null}]"#;
    const DONE: &str = r#"[{"task_id": "task-1", "status": "SUCCESS", "result": "Success. New document id 77 created", "related_document": "77"}]"#;
    const DUPLICATE: &str = r#"[{"task_id": "task-1", "status": "FAILURE", "result": "x.pdf: Not consuming x.pdf: It is a duplicate of Rechnung (#5).", "related_document": "5"}]"#;
    const BROKEN: &str = r#"[{"task_id": "task-1", "status": "FAILURE", "result": "geheimnis vom server", "related_document": null}]"#;

    #[test]
    fn the_form_carries_fields_and_file() {
        let (content_type, body) = multipart(
            &[
                ("title", "Rechnung\r\nX-Evil: 1"),
                ("created", "2026-11-07"),
            ],
            "a\"b\\c\r\n.pdf",
            "application/pdf",
            b"%PDF-daten".to_vec(),
        );
        let boundary = content_type
            .strip_prefix("multipart/form-data; boundary=")
            .unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(
            text.contains("name=\"title\"\r\n\r\nRechnung  X-Evil: 1\r\n"),
            "{text}"
        );
        assert!(text.contains("name=\"created\"\r\n\r\n2026-11-07\r\n"));
        assert!(text.contains(
            "name=\"document\"; filename=\"abc.pdf\"\r\nContent-Type: application/pdf\r\n\r\n%PDF-daten\r\n"
        ));
        assert!(text.ends_with(&format!("--{boundary}--\r\n")));
        assert_eq!(text.matches(&format!("--{boundary}")).count(), 4);
    }

    #[test]
    fn the_archives_answer_about_a_task_is_read_in_both_shapes() {
        let state = |s: &str| task_state(&serde_json::from_str(s).unwrap());
        assert_eq!(state(PENDING), TaskState::Working);
        assert_eq!(state(r#"[{"status": "STARTED"}]"#), TaskState::Working);
        assert_eq!(state(DONE), TaskState::Document(77));
        assert_eq!(
            state(r#"[{"status": "SUCCESS", "related_document": 77}]"#),
            TaskState::Document(77)
        );
        assert_eq!(
            state(r#"[{"status": "SUCCESS", "related_document": null}]"#),
            TaskState::Failed
        );
        // refused as a duplicate: the document it already is
        assert_eq!(state(DUPLICATE), TaskState::Document(5));
        assert_eq!(
            state(
                r#"[{"status": "FAILURE", "related_document": null, "duplicate_documents": [{"id": 9, "title": "x", "deleted_at": null}]}]"#
            ),
            TaskState::Document(9)
        );
        // … unless that one lies in the archive's trash
        assert_eq!(
            state(
                r#"[{"status": "FAILURE", "related_document": null, "duplicate_documents": [{"id": 9, "deleted_at": "2026-11-01T10:00:00Z"}]}]"#
            ),
            TaskState::InTrash
        );
        // … also when the archive names that document as the related one
        assert_eq!(state(TRASHED), TaskState::InTrash);
        assert_eq!(
            state(
                r#"[{"status": "SUCCESS", "related_document": 9, "duplicate_documents": [{"id": 9, "deleted_at": "2026-11-01T10:00:00Z"}]}]"#
            ),
            TaskState::InTrash
        );
        // one copy in the trash, one alive: the live one
        assert_eq!(
            state(
                r#"[{"status": "FAILURE", "related_document": 9, "duplicate_documents": [{"id": 9, "deleted_at": "2026-11-01T10:00:00Z"}, {"id": 12, "deleted_at": null}]}]"#
            ),
            TaskState::Document(12)
        );
        assert_eq!(state(BROKEN), TaskState::Failed);
        assert_eq!(state("[]"), TaskState::Unknown);
        // later API versions answer with a page
        assert_eq!(state(r#"{"count": 0, "results": []}"#), TaskState::Unknown);
        assert_eq!(
            state(r#"{"count": 1, "results": [{"status": "success", "related_document": 3}]}"#),
            TaskState::Document(3)
        );
    }

    #[tokio::test]
    async fn a_file_moves_into_the_archive_and_leaves_a_reference() {
        let (server, asked) = taking(vec![PENDING, DONE]).await;
        let dir = TempDir::new("archive-move-ok");
        let (v, t1, doc) = with_file(&dir, &server);
        v.queue_archive_move(doc).unwrap();
        assert_eq!(
            v.archive_moves().unwrap(),
            [ArchiveMove {
                doc_id: doc,
                state: "queued".into(),
                problem: String::new()
            }]
        );
        assert!(server.seen().is_empty(), "queueing sends nothing");

        // 1: posted. 2: the archive is still working. 3: done.
        let run = |v: &Arc<Vault>| v.clone().process_archive_moves();
        assert_eq!(
            run(&v).await.unwrap(),
            ArchiveMoveReport {
                moved: 0,
                waiting: 1,
                failed: 0
            }
        );
        assert_eq!(v.archive_moves().unwrap()[0].state, "uploaded");
        assert_eq!(
            run(&v).await.unwrap(),
            ArchiveMoveReport {
                moved: 0,
                waiting: 1,
                failed: 0
            }
        );
        // the file is still there, readable, until the archive has it
        assert_eq!(v.get_document_bytes(doc).unwrap(), pdf());
        assert_eq!(
            run(&v).await.unwrap(),
            ArchiveMoveReport {
                moved: 1,
                waiting: 0,
                failed: 0
            }
        );
        assert_eq!(asked.load(Ordering::SeqCst), 2);

        // now only the reference is left
        assert_eq!(count(&v, "SELECT count(*) FROM booking_documents"), 0);
        assert!(v.archive_moves().unwrap().is_empty());
        let docs = v.list_documents_for_booking(t1).unwrap();
        assert_eq!(docs.len(), 1);
        let link = &docs[0];
        assert_eq!(link.archive_doc, Some(77));
        assert_eq!(link.title, "Rechnung Weber");
        assert_eq!(link.doc_date, "2026-11-07");
        assert_eq!(
            link.link_url,
            format!("{}/documents/77/details", server.base)
        );
        assert_eq!(document_count(&v, "T1"), 1);
        // nothing left to do, nothing sent
        let before = server.seen().len();
        assert_eq!(run(&v).await.unwrap(), ArchiveMoveReport::default());
        assert_eq!(server.seen().len(), before);

        // what was posted
        let post = server
            .seen()
            .into_iter()
            .find(|r| r.method == "POST")
            .unwrap();
        assert!(post
            .head
            .contains("content-type: multipart/form-data; boundary=girocrab-"));
        assert!(post.head.contains(&format!("authorization: token {TOKEN}")));
        let body = String::from_utf8_lossy(&post.body).to_string();
        assert!(
            body.contains("name=\"title\"\r\n\r\nRechnung Weber\r\n"),
            "{body}"
        );
        assert!(body.contains("name=\"created\"\r\n\r\n2026-11-07\r\n"));
        assert!(body.contains(&format!("filename=\"{PDF_NAME}\"")));
        assert!(post.body.windows(pdf().len()).any(|w| w == pdf()));
    }

    /// While the archive works on a file, the user moves or unlinks it: the
    /// reference lands where the file is at the end.
    #[tokio::test]
    async fn the_reference_goes_where_the_file_is_when_the_archive_is_done() {
        let (server, _) = taking(vec![DONE]).await;
        let dir = TempDir::new("archive-move-relink");
        let (v, t1, doc) = with_file(&dir, &server);
        v.queue_archive_move(doc).unwrap();
        v.clone().process_archive_moves().await.unwrap();
        v.link_document(doc, Some(id_of(&v, "T2"))).unwrap();
        v.clone().process_archive_moves().await.unwrap();
        assert!(v.list_documents_for_booking(t1).unwrap().is_empty());
        assert_eq!(
            v.list_documents_for_booking(id_of(&v, "T2")).unwrap()[0].archive_doc,
            Some(77)
        );

        // unlinked meanwhile: an unlinked reference of its account
        let second = v
            .attach_document_to_booking(t1, "bon.pdf".into(), pdf())
            .unwrap();
        v.queue_archive_move(second).unwrap();
        v.clone().process_archive_moves().await.unwrap();
        v.link_document(second, None).unwrap();
        v.clone().process_archive_moves().await.unwrap();
        let unlinked = v
            .list_all_documents(crate::documents::DocumentFilter {
                unlinked_only: true,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(unlinked.len(), 1);
        assert_eq!(unlinked[0].archive_doc, Some(77));
        assert!(unlinked[0].account_id.is_some());
        assert_eq!(count(&v, "SELECT count(*) FROM booking_documents"), 0);
    }

    /// The archive already has this very file (posted before the app died, or
    /// scanned there by hand): the existing document is linked.
    #[tokio::test]
    async fn a_file_the_archive_already_has_is_linked_not_doubled() {
        let (server, _) = taking(vec![DUPLICATE]).await;
        let dir = TempDir::new("archive-move-dup");
        let (v, t1, doc) = with_file(&dir, &server);
        v.queue_archive_move(doc).unwrap();
        v.clone().process_archive_moves().await.unwrap();
        let report = v.clone().process_archive_moves().await.unwrap();
        assert_eq!(report.moved, 1);
        assert_eq!(
            v.list_documents_for_booking(t1).unwrap()[0].archive_doc,
            Some(5)
        );
        assert_eq!(count(&v, "SELECT count(*) FROM booking_documents"), 0);
    }

    #[tokio::test]
    async fn a_file_the_archive_cannot_process_stays_in_the_vault() {
        let (server, _) = taking(vec![BROKEN]).await;
        let dir = TempDir::new("archive-move-broken");
        let (v, t1, doc) = with_file(&dir, &server);
        v.queue_archive_move(doc).unwrap();
        v.clone().process_archive_moves().await.unwrap();
        let report = v.clone().process_archive_moves().await.unwrap();
        assert_eq!(
            report,
            ArchiveMoveReport {
                moved: 0,
                waiting: 0,
                failed: 1
            }
        );
        let moves = v.archive_moves().unwrap();
        assert_eq!(moves[0].state, "failed");
        assert!(!moves[0].problem.is_empty() && !moves[0].problem.contains("geheimnis"));
        // the file is untouched and still the booking's document
        assert_eq!(v.get_document_bytes(doc).unwrap(), pdf());
        assert_eq!(v.list_documents_for_booking(t1).unwrap()[0].id, doc);
        // a failed file is left alone until the user says so
        let before = server.seen().len();
        assert_eq!(
            v.clone().process_archive_moves().await.unwrap(),
            ArchiveMoveReport::default()
        );
        assert_eq!(server.seen().len(), before);
        v.queue_archive_move(doc).unwrap();
        assert_eq!(v.archive_moves().unwrap()[0].state, "queued");
        v.cancel_archive_move(doc).unwrap();
        assert!(v.archive_moves().unwrap().is_empty());
        assert_eq!(v.get_document_bytes(doc).unwrap(), pdf());
    }

    #[tokio::test]
    async fn an_unreachable_archive_costs_nothing_and_a_refusal_gives_up_after_a_few_tries() {
        // nobody listening: the run stops, the file waits, nothing is counted against it
        let dir = TempDir::new("archive-move-down");
        let v = open(&dir, "v.db");
        seed(&v, &[("T1", "2026-11-07", -23800, "Elektro Weber")]);
        v.set_archive_connection("http://127.0.0.1:9".into(), TOKEN.into(), None)
            .unwrap();
        let doc = v
            .attach_document_to_booking(id_of(&v, "T1"), PDF_NAME.into(), pdf())
            .unwrap();
        v.queue_archive_move(doc).unwrap();
        for _ in 0..6 {
            assert!(matches!(
                v.clone().process_archive_moves().await,
                Err(CoreError::Network { .. })
            ));
        }
        assert_eq!(v.archive_moves().unwrap()[0].state, "queued");
        assert_eq!(count(&v, "SELECT attempts FROM archive_moves"), 0);

        // the archive answers, but refuses the upload: a few tries, then it is given up on
        let server = archive(|_| status(400)).await;
        let dir = TempDir::new("archive-move-refused");
        let (v, _, doc) = with_file(&dir, &server);
        v.queue_archive_move(doc).unwrap();
        let mut failed = 0;
        for _ in 0..4 {
            failed += v.clone().process_archive_moves().await.unwrap().failed;
        }
        assert_eq!(failed, 1);
        // three refusals, then the file is left alone (each upload is
        // preceded by the question whether the archive has the file)
        let posts = server.seen().iter().filter(|r| r.method == "POST").count();
        assert_eq!(posts, 3);
        assert_eq!(v.archive_moves().unwrap()[0].state, "failed");
        assert_eq!(v.get_document_bytes(doc).unwrap(), pdf());

        // sign-in refused: that is about the connection, not about the file
        let server = archive(|_| status(401)).await;
        let dir = TempDir::new("archive-move-auth");
        let (v, _, doc) = with_file(&dir, &server);
        v.queue_archive_move(doc).unwrap();
        assert!(matches!(
            v.clone().process_archive_moves().await,
            Err(CoreError::Authentication { .. })
        ));
        assert_eq!(v.archive_moves().unwrap()[0].state, "queued");
    }

    #[tokio::test]
    async fn only_the_users_own_receipts_are_queued_in_bulk() {
        let (server, _) = taking(vec![DONE]).await;
        let dir = TempDir::new("archive-move-bulk");
        let (v, t1, doc) = with_file(&dir, &server);
        {
            // a statement the bank delivered, and an older receipt
            let db = v.db().unwrap();
            db.execute(
                "INSERT INTO booking_documents(booking_id, title, pdf_bytes, source_url) VALUES (?1, 'Auszug', x'25504446', 'bank:1')",
                [t1],
            )
            .unwrap();
            db.execute(
                "INSERT INTO booking_documents(booking_id, title, pdf_bytes) VALUES (?1, 'Zweiter', x'25504446')",
                [t1],
            )
            .unwrap();
        }
        assert!(!v.archive_upload_enabled().unwrap());
        v.set_archive_upload(true).unwrap();
        assert!(v.archive_upload_enabled().unwrap());

        // of the files named only the user's own: the second receipt, not
        // the statement, and nothing for a file that does not exist
        assert_eq!(
            v.queue_archive_moves(Some(vec![doc + 1, doc + 2, 9999]), aid(&v))
                .unwrap(),
            1
        );
        assert_eq!(v.archive_moves().unwrap()[0].doc_id, doc + 2);
        assert_eq!(
            v.queue_archive_moves(Some(vec![doc + 2]), aid(&v)).unwrap(),
            0
        );
        assert_eq!(v.queue_archive_moves(Some(Vec::new()), aid(&v)).unwrap(), 0);
        // everything of the user's own: the first receipt too, never the bank's statement
        assert_eq!(v.queue_archive_moves(None, aid(&v)).unwrap(), 1);
        assert_eq!(v.archive_moves().unwrap().len(), 2);
        // meant for an archive that is not the connected one: nothing
        assert_eq!(
            v.queue_archive_moves(None, "ein-anderes".into()).unwrap(),
            0
        );
        assert_eq!(v.queue_archive_moves(None, aid(&v)).unwrap(), 0);

        // The bank's statement stays out of "all" while the banks' documents
        // are not moved. Asked for by itself it goes: its reference keeps
        // the origin the fetch knows it by.
        v.queue_archive_move(doc + 1).unwrap();
        assert_eq!(v.archive_moves().unwrap().len(), 3);
        v.cancel_archive_move(doc + 1).unwrap();
        assert_eq!(v.archive_moves().unwrap().len(), 2);

        // a file deleted while it waits simply drops off the queue
        v.delete_document(doc).unwrap();
        assert_eq!(v.archive_moves().unwrap().len(), 1);

        // another archive, or none: what was queued stays in the vault, off the queue
        v.set_archive_connection("https://neu.example".into(), "t2".into(), None)
            .unwrap();
        assert!(v.archive_moves().unwrap().is_empty());
        v.queue_archive_moves(None, aid(&v)).unwrap();
        v.remove_archive_connection().unwrap();
        assert!(v.archive_moves().unwrap().is_empty());
        assert!(!v.archive_upload_enabled().unwrap());
        assert_eq!(count(&v, "SELECT count(*) FROM booking_documents"), 2);
        assert!(v.queue_archive_move(doc).is_err());
    }

    // -------- answers that come too late, or about something else

    /// The archive answers "done" after the user decided to keep the file in
    /// the vault, or deleted it: nothing is linked, nothing is deleted.
    #[tokio::test]
    async fn a_late_answer_never_touches_a_file_that_is_no_longer_on_its_way() {
        let (server, _) = taking(vec![DONE]).await;
        let dir = TempDir::new("archive-move-late");
        let (v, t1, doc) = with_file(&dir, &server);
        v.queue_archive_move(doc).unwrap();
        v.clone().process_archive_moves().await.unwrap();
        let claim = claim_of(&v, doc);
        assert_eq!(claim.task, "task:task-1");
        let base = base_of_vault(&v);

        // "Im Tresor behalten" while the question to the archive is under way
        v.cancel_archive_move(doc).unwrap();
        assert!(!finish(&mut v.db().unwrap(), &claim, &base, 77).unwrap());
        assert_eq!(v.get_document_bytes(doc).unwrap(), pdf());
        assert!(v
            .list_documents_for_booking(t1)
            .unwrap()
            .iter()
            .all(|d| d.archive_doc.is_none()));

        // Deleted, and the next file gets the same id: the old answer is not about it
        v.delete_document(doc).unwrap();
        let next = v
            .attach_document_to_booking(t1, "anderer.pdf".into(), b"%PDF-anderer".to_vec())
            .unwrap();
        assert_eq!(next, doc, "SQLite hands the id out again");
        assert!(!finish(&mut v.db().unwrap(), &claim, &base, 77).unwrap());
        v.queue_archive_move(next).unwrap();
        assert_ne!(claim_of(&v, next).op, claim.op);
        assert!(!finish(&mut v.db().unwrap(), &claim, &base, 77).unwrap());
        assert!(!claim.posted(&v.db().unwrap(), "task-9").unwrap());
        assert_eq!(v.get_document_bytes(next).unwrap(), b"%PDF-anderer");
        assert_eq!(v.archive_moves().unwrap()[0].state, "queued");
        assert_eq!(count(&v, "SELECT count(*) FROM document_links"), 0);
    }

    /// A run that started under one archive must not write once the vault is
    /// connected to another, even if the file was queued again there.
    #[tokio::test]
    async fn a_run_of_the_old_archive_writes_nothing_after_a_change() {
        let (server, _) = taking(vec![DONE]).await;
        let dir = TempDir::new("archive-move-switch");
        let (v, _, doc) = with_file(&dir, &server);
        v.queue_archive_move(doc).unwrap();
        let old = claim_of(&v, doc);

        v.set_archive_connection("https://neu.example".into(), "t2".into(), None)
            .unwrap();
        assert!(v.archive_moves().unwrap().is_empty());
        v.queue_archive_move(doc).unwrap();
        let new = claim_of(&v, doc);
        assert_ne!(new.archive_id, old.archive_id);

        // the old run comes back from its requests
        let db = v.db().unwrap();
        assert!(!old.claim(&db).unwrap());
        assert!(!old.posted(&db, "task-old").unwrap());
        drop(db);
        // the base first: the vault lock is not re-entrant
        let base = base_of_vault(&v);
        assert!(!finish(&mut v.db().unwrap(), &old, &base, 77).unwrap());
        assert_eq!(claim_of(&v, doc), new, "the new move is untouched");
        assert_eq!(v.archive_moves().unwrap()[0].state, "queued");
        assert_eq!(v.get_document_bytes(doc).unwrap(), pdf());
    }

    #[tokio::test]
    async fn two_runs_never_post_the_same_file() {
        let (server, _) = taking(vec![PENDING]).await;
        let dir = TempDir::new("archive-move-twice");
        let (v, _, doc) = with_file(&dir, &server);
        v.queue_archive_move(doc).unwrap();
        // both runs read the queue before either sends
        let (first, second) = (claim_of(&v, doc), claim_of(&v, doc));
        assert!(first.claim(&v.db().unwrap()).unwrap());
        assert!(!second.claim(&v.db().unwrap()).unwrap());
        assert_eq!(v.archive_moves().unwrap()[0].state, "posting");

        // and for real: two runs at once post once
        v.cancel_archive_move(doc).unwrap();
        v.queue_archive_move(doc).unwrap();
        let (a, b) = tokio::join!(
            v.clone().process_archive_moves(),
            v.clone().process_archive_moves()
        );
        a.unwrap();
        b.unwrap();
        assert_eq!(
            server.seen().iter().filter(|r| r.method == "POST").count(),
            1
        );
    }

    /// The upload went out, but its answer never came (the app quit, the
    /// request was cancelled, the answer was garbage): nobody knows whether
    /// the archive has the file. It is not posted again unasked.
    #[tokio::test]
    async fn an_upload_with_an_unknown_outcome_is_not_repeated_unasked() {
        // an answer that is not a task id
        let server = archive(|r| match r.method.as_str() {
            "POST" => json("{}"),
            _ => status(404),
        })
        .await;
        let dir = TempDir::new("archive-move-unclear");
        let (v, _, doc) = with_file(&dir, &server);
        v.queue_archive_move(doc).unwrap();
        let report = v.clone().process_archive_moves().await.unwrap();
        assert_eq!(
            report,
            ArchiveMoveReport {
                moved: 0,
                waiting: 0,
                failed: 1
            }
        );
        v.clone().process_archive_moves().await.unwrap();
        assert_eq!(
            server.seen().iter().filter(|r| r.method == "POST").count(),
            1
        );
        let moves = v.archive_moves().unwrap();
        assert_eq!(moves[0].state, "failed");
        assert!(moves[0].problem.contains("Unklar"), "{}", moves[0].problem);
        assert_eq!(v.get_document_bytes(doc).unwrap(), pdf());

        // The same file on another receipt: the archive may have the first
        // upload, so the copy is not sent on top of it. It says why.
        let copy = v
            .attach_document_to_booking(id_of(&v, "T2"), "Kopie.pdf".into(), pdf())
            .unwrap();
        v.queue_archive_move(copy).unwrap();
        let report = v.clone().process_archive_moves().await.unwrap();
        assert_eq!(report.failed, 1);
        assert_eq!(
            server.seen().iter().filter(|r| r.method == "POST").count(),
            1,
            "no second upload of the file"
        );
        let waiting = v.archive_moves().unwrap();
        let of_copy = waiting.iter().find(|m| m.doc_id == copy).unwrap();
        assert_eq!(of_copy.state, "failed");
        assert!(
            of_copy.problem.contains("anderen Beleg"),
            "{}",
            of_copy.problem
        );
        // the copy does not block the first one in turn: that one can be tried again
        v.cancel_archive_move(copy).unwrap();
        v.delete_document(copy).unwrap();

        // a run that died while posting leaves `posting` behind
        v.queue_archive_move(doc).unwrap();
        let claim = claim_of(&v, doc);
        assert!(claim.claim(&v.db().unwrap()).unwrap());
        let now = crate::dates::now_ms();
        // still fresh: it may be another run that is posting right now
        assert_eq!(
            settle_stale_posts(&v.db().unwrap(), &claim.archive_id, now).unwrap(),
            0
        );
        assert_eq!(v.archive_moves().unwrap()[0].state, "posting");
        v.clone().process_archive_moves().await.unwrap();
        assert_eq!(
            server.seen().iter().filter(|r| r.method == "POST").count(),
            1
        );
        // long gone: failed with the explanation, the user decides
        let later = now + POSTING_STALE_MS + 1;
        assert_eq!(
            settle_stale_posts(&v.db().unwrap(), &claim.archive_id, later).unwrap(),
            1
        );
        assert_eq!(v.archive_moves().unwrap()[0].state, "failed");
        assert_eq!(v.get_document_bytes(doc).unwrap(), pdf());
    }

    /// The archive says the file is a document that lies in its trash, or
    /// names a document this token cannot read: the file stays in the vault.
    #[tokio::test]
    async fn a_document_that_cannot_be_read_in_the_archive_is_no_reason_to_delete_the_file() {
        let (server, _) = taking(vec![TRASHED]).await;
        let dir = TempDir::new("archive-move-trash");
        let (v, t1, doc) = with_file(&dir, &server);
        v.queue_archive_move(doc).unwrap();
        v.clone().process_archive_moves().await.unwrap();
        let report = v.clone().process_archive_moves().await.unwrap();
        assert_eq!(
            report,
            ArchiveMoveReport {
                moved: 0,
                waiting: 0,
                failed: 1
            }
        );
        assert_eq!(v.get_document_bytes(doc).unwrap(), pdf());
        assert!(v.archive_moves().unwrap()[0].problem.contains("Papierkorb"));
        assert_eq!(v.list_documents_for_booking(t1).unwrap().len(), 1);

        // "done, document 77", but 77 is not there for this token
        let server = archive(|r| match (r.method.as_str(), r.path()) {
            ("POST", _) => json("\"task-1\""),
            ("GET", "/api/tasks/") => json(DONE),
            _ => status(404),
        })
        .await;
        let dir = TempDir::new("archive-move-unreadable");
        let (v, _, doc) = with_file(&dir, &server);
        v.queue_archive_move(doc).unwrap();
        v.clone().process_archive_moves().await.unwrap();
        let report = v.clone().process_archive_moves().await.unwrap();
        assert_eq!(report.failed, 1);
        assert_eq!(v.get_document_bytes(doc).unwrap(), pdf());
        assert_eq!(count(&v, "SELECT count(*) FROM document_links"), 0);
        assert!(server
            .seen()
            .iter()
            .any(|r| r.path() == "/api/documents/77/"));
    }

    /// The archive names a document for the upload. Unless that document is
    /// the one asked for, alive, and its original comes back byte for byte,
    /// the file stays in the vault.
    #[tokio::test]
    async fn the_file_stays_until_the_archive_hands_the_very_file_back() {
        for kind in ["no file", "other file", "other id", "trashed", "empty file"] {
            let server = archive(move |r| match (r.method.as_str(), r.path()) {
                ("POST", _) => json("\"task-1\""),
                ("GET", "/api/tasks/") => json(DONE),
                ("GET", "/api/documents/77/") => match kind {
                    "other id" => json(r#"{"id": 1}"#),
                    "trashed" => json(r#"{"id": 77, "deleted_at": "2026-11-01T10:00:00Z"}"#),
                    _ => json(r#"{"id": 77, "deleted_at": null}"#),
                },
                ("GET", "/api/documents/77/download/") => match kind {
                    "no file" => status(404),
                    _ => Answer {
                        status: 200,
                        headers: vec![("Content-Type", "application/pdf".into())],
                        body: match kind {
                            "other file" => b"%PDF-1.4 etwas anderes".to_vec(),
                            "empty file" => Vec::new(),
                            _ => pdf(),
                        },
                    },
                },
                _ => status(404),
            })
            .await;
            let dir = TempDir::new(&format!("archive-move-back-{}", kind.replace(' ', "-")));
            let (v, t1, doc) = with_file(&dir, &server);
            v.queue_archive_move(doc).unwrap();
            v.clone().process_archive_moves().await.unwrap();
            if kind == "other id" {
                // an answer about another document says nothing: no verdict,
                // the file simply stays and is asked about again
                assert!(v.clone().process_archive_moves().await.is_err());
                assert_eq!(v.archive_moves().unwrap()[0].state, "uploaded");
                assert_eq!(v.get_document_bytes(doc).unwrap(), pdf());
                assert_eq!(count(&v, "SELECT count(*) FROM document_links"), 0);
                continue;
            }
            let report = v.clone().process_archive_moves().await.unwrap();
            assert_eq!((report.moved, report.failed), (0, 1), "{kind}");
            assert_eq!(v.get_document_bytes(doc).unwrap(), pdf(), "{kind}");
            assert_eq!(
                count(&v, "SELECT count(*) FROM document_links"),
                0,
                "{kind}"
            );
            assert_eq!(v.list_documents_for_booking(t1).unwrap()[0].id, doc);
            let problem = v.archive_moves().unwrap().remove(0).problem;
            assert!(problem.contains("Tresor"), "{kind}: {problem}");
            // the original is what was asked for, never the archive's own version
            let asked_for_file = server
                .seen()
                .iter()
                .any(|r| r.path() == "/api/documents/77/download/" && r.has("original=true"));
            assert_eq!(asked_for_file, kind.ends_with("file"), "{kind}");
        }
    }

    /// An archive that keeps the same file twice when it is uploaded twice
    /// is asked first: if it has this very file, that document is linked and
    /// nothing is uploaded. What it answers is checked byte for byte, so an
    /// archive that does not know the question (and lists anything) costs a
    /// look, not a wrong link.
    #[tokio::test]
    async fn a_file_the_archive_has_is_linked_without_an_upload() {
        for knows in [true, false] {
            let file = |body: Vec<u8>| Answer {
                status: 200,
                headers: Vec::new(),
                body,
            };
            let server = archive(move |r| match (r.method.as_str(), r.path()) {
                // by checksum: document 5, and 6 that is another file
                ("GET", "/api/documents/") => {
                    json(r#"{"count": 2, "results": [{"id": 6}, {"id": 5}]}"#)
                }
                ("GET", "/api/documents/6/download/") => file(b"%PDF-1.4 etwas anderes".to_vec()),
                // document 5 has been edited in the archive since: without a
                // word about the version the newer file comes, and only the
                // first version is the file that was stored
                ("GET", "/api/documents/5/download/") => file(if knows && r.has("version=5") {
                    pdf()
                } else {
                    b"%PDF-1.4 auch anders".to_vec()
                }),
                ("GET", "/api/documents/5/") => json(r#"{"id": 5, "deleted_at": null}"#),
                ("GET", "/api/documents/77/") => json(r#"{"id": 77, "deleted_at": null}"#),
                ("GET", "/api/documents/77/download/") => file(pdf()),
                ("POST", "/api/documents/post_document/") => json("\"task-1\""),
                ("GET", "/api/tasks/") => json(DONE),
                _ => status(404),
            })
            .await;
            let dir = TempDir::new(if knows {
                "archive-move-known"
            } else {
                "archive-move-unknown"
            });
            let (v, t1, doc) = with_file(&dir, &server);
            v.queue_archive_move(doc).unwrap();
            v.clone().process_archive_moves().await.unwrap();
            let report = v.clone().process_archive_moves().await.unwrap();
            assert_eq!((report.moved, report.failed), (1, 0), "{knows}");
            let posts = server.seen().iter().filter(|r| r.method == "POST").count();
            let linked = v
                .list_documents_for_booking(t1)
                .unwrap()
                .remove(0)
                .archive_doc;
            if knows {
                assert_eq!((posts, linked), (0, Some(5)));
            } else {
                assert_eq!((posts, linked), (1, Some(77)));
            }
            // asked for by the checksum of the file (64 hex digits)
            assert!(server.seen().iter().any(|r| {
                r.path() == "/api/documents/" && r.has("checksum__iexact=") && r.target.len() > 90
            }));
            assert_eq!(count(&v, "SELECT count(*) FROM booking_documents"), 0);
        }
    }

    /// One receipt on two bookings, both queued: the file goes up once. The
    /// second waits while the first is on its way and is then linked to the
    /// first one's document.
    #[tokio::test]
    async fn the_same_file_queued_twice_is_uploaded_once() {
        let asked = Arc::new(AtomicUsize::new(0));
        let n = asked.clone();
        let server = archive(move |r| match (r.method.as_str(), r.path()) {
            ("POST", "/api/documents/post_document/") => json("\"task-1\""),
            // the archive works on it for one round, then document 77 is there
            ("GET", "/api/tasks/") => json(if n.fetch_add(1, Ordering::SeqCst) == 0 {
                PENDING
            } else {
                DONE
            }),
            ("GET", "/api/documents/") => json(if n.load(Ordering::SeqCst) >= 2 {
                r#"{"count": 1, "results": [{"id": 77}]}"#
            } else {
                r#"{"count": 0, "results": []}"#
            }),
            ("GET", "/api/documents/77/") => json(r#"{"id": 77, "deleted_at": null}"#),
            ("GET", "/api/documents/77/download/") => Answer {
                status: 200,
                headers: Vec::new(),
                body: pdf(),
            },
            _ => status(404),
        })
        .await;
        let dir = TempDir::new("archive-move-twins");
        let (v, t1, first) = with_file(&dir, &server);
        let t2 = id_of(&v, "T2");
        let second = v
            .attach_document_to_booking(t2, "Kopie.pdf".into(), pdf())
            .unwrap();
        assert_ne!(first, second);
        v.queue_archive_move(first).unwrap();
        v.queue_archive_move(second).unwrap();
        let mut moved = 0;
        for _ in 0..6 {
            moved += v.clone().process_archive_moves().await.unwrap().moved;
        }
        assert_eq!(moved, 2);
        let posts = server.seen().iter().filter(|r| r.method == "POST").count();
        assert_eq!(posts, 1, "one upload for the two of them");
        for booking in [t1, t2] {
            let docs = v.list_documents_for_booking(booking).unwrap();
            assert_eq!(docs.len(), 1);
            assert_eq!(docs[0].archive_doc, Some(77));
        }
        assert_eq!(count(&v, "SELECT count(*) FROM booking_documents"), 0);
        assert!(v.archive_moves().unwrap().is_empty());
    }

    /// The archive lists a document for the checksum, but will not hand it
    /// out to this token, or lists something that is no document: it may be
    /// the file, so nothing is uploaded on a guess.
    #[tokio::test]
    async fn a_listed_file_that_cannot_be_compared_is_no_reason_to_upload() {
        for kind in ["not allowed", "no document", "gone again"] {
            let server = archive(move |r| match (r.method.as_str(), r.path()) {
                ("GET", "/api/documents/") => json(if kind == "no document" {
                    r#"{"count": 1, "results": [{"title": "ohne Nummer"}]}"#
                } else {
                    r#"{"count": 1, "results": [{"id": 5}]}"#
                }),
                ("GET", "/api/documents/5/download/") => {
                    status(if kind == "gone again" { 404 } else { 403 })
                }
                ("POST", _) => json("\"task-1\""),
                _ => status(404),
            })
            .await;
            let dir = TempDir::new(&format!("archive-move-listed-{}", kind.replace(' ', "-")));
            let (v, _, doc) = with_file(&dir, &server);
            v.queue_archive_move(doc).unwrap();
            let run = v.clone().process_archive_moves().await;
            let posts = server.seen().iter().filter(|r| r.method == "POST").count();
            if kind == "gone again" {
                // it was deleted since it was listed: the file goes up
                assert!(run.is_ok());
                assert_eq!(posts, 1);
            } else {
                assert!(run.is_err(), "{kind}");
                assert_eq!(posts, 0, "{kind}");
                assert_eq!(v.archive_moves().unwrap()[0].state, "queued");
            }
            assert_eq!(v.get_document_bytes(doc).unwrap(), pdf());
        }
    }

    /// Whether the archive has the file cannot be asked right now (busy, not
    /// allowed, an answer that is none): nothing is uploaded on a guess, the
    /// file waits.
    #[tokio::test]
    async fn nothing_is_uploaded_while_the_archive_cannot_say_whether_it_has_the_file() {
        for (code, body) in [
            (503u16, ""),
            (429, ""),
            (403, ""),
            (200, "<html>Wartung</html>"),
            (200, "{}"),
        ] {
            let server = archive(move |r| match (r.method.as_str(), r.path()) {
                ("GET", "/api/documents/") if code == 200 => Answer {
                    status: 200,
                    headers: Vec::new(),
                    body: body.as_bytes().to_vec(),
                },
                ("GET", "/api/documents/") => status(code),
                ("POST", _) => json("\"task-1\""),
                _ => status(404),
            })
            .await;
            let dir = TempDir::new(&format!("archive-move-ask-{code}-{}", body.len()));
            let (v, _, doc) = with_file(&dir, &server);
            v.queue_archive_move(doc).unwrap();
            assert!(
                v.clone().process_archive_moves().await.is_err(),
                "{code} {body}"
            );
            assert_eq!(v.archive_moves().unwrap()[0].state, "queued");
            assert!(
                server.seen().iter().all(|r| r.method != "POST"),
                "{code} {body}"
            );
            assert_eq!(v.get_document_bytes(doc).unwrap(), pdf());
        }
    }

    /// A file whose upload may have arrived keeps its twins from going up on
    /// top of it, also once the user kept that file in the vault: trying it
    /// again is what looks it up in the archive and settles the matter.
    #[tokio::test]
    async fn a_twin_of_an_unclear_upload_waits_also_when_that_file_was_kept() {
        let server = archive(move |r| match (r.method.as_str(), r.path()) {
            ("GET", "/api/documents/") => json(r#"{"count": 0, "results": []}"#),
            // a gateway's error after the archive may have taken the file
            ("POST", "/api/documents/post_document/") => status(504),
            _ => status(404),
        })
        .await;
        let dir = TempDir::new("archive-move-twin-kept");
        let (v, _, doc) = with_file(&dir, &server);
        v.queue_archive_move(doc).unwrap();
        v.clone().process_archive_moves().await.unwrap();
        assert_eq!(v.archive_moves().unwrap()[0].state, "failed");
        v.cancel_archive_move(doc).unwrap();
        assert!(v.archive_moves().unwrap().is_empty());

        // the same file on another booking
        let twin = v
            .attach_document_to_booking(id_of(&v, "T2"), PDF_NAME.into(), pdf())
            .unwrap();
        v.queue_archive_move(twin).unwrap();
        let report = v.clone().process_archive_moves().await.unwrap();
        assert_eq!(report.failed, 1);
        let posts = server.seen().iter().filter(|r| r.method == "POST").count();
        assert_eq!(posts, 1, "only the first one went out");
        let moves = v.archive_moves().unwrap();
        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].doc_id, twin);
        assert!(moves[0].problem.contains("unklarem Ausgang"));
    }

    /// A file the archive is still working on, or one that is being handed
    /// over at this very moment, is kept in the vault after all. It may well
    /// be in the archive: the same file does not go up for another receipt
    /// on top of it.
    #[tokio::test]
    async fn a_file_kept_after_it_went_out_still_guards_its_twins() {
        for during_the_post in [false, true] {
            let asked_for: Arc<Mutex<Option<(Arc<Vault>, i64)>>> = Arc::default();
            let held = asked_for.clone();
            let server = archive(move |r| match (r.method.as_str(), r.path()) {
                ("GET", "/api/documents/") => json(r#"{"count": 0, "results": []}"#),
                ("POST", "/api/documents/post_document/") => {
                    if let Some((v, doc)) = held.lock().unwrap().as_ref() {
                        v.cancel_archive_move(*doc).unwrap();
                    }
                    json("\"task-1\"")
                }
                // the archive is still working on it
                ("GET", "/api/tasks/") => json(PENDING),
                _ => status(404),
            })
            .await;
            let dir = TempDir::new(if during_the_post {
                "archive-move-kept-posting"
            } else {
                "archive-move-kept-uploaded"
            });
            let (v, _, doc) = with_file(&dir, &server);
            if during_the_post {
                *asked_for.lock().unwrap() = Some((v.clone(), doc));
            }
            v.queue_archive_move(doc).unwrap();
            v.clone().process_archive_moves().await.unwrap();
            *asked_for.lock().unwrap() = None;
            if !during_the_post {
                assert_eq!(v.archive_moves().unwrap()[0].state, "uploaded");
                v.cancel_archive_move(doc).unwrap();
            }
            assert!(v.archive_moves().unwrap().is_empty(), "{during_the_post}");

            let twin = v
                .attach_document_to_booking(id_of(&v, "T2"), PDF_NAME.into(), pdf())
                .unwrap();
            v.queue_archive_move(twin).unwrap();
            let report = v.clone().process_archive_moves().await.unwrap();
            assert_eq!(report.failed, 1, "{during_the_post}");
            let posts = server.seen().iter().filter(|r| r.method == "POST").count();
            assert_eq!(posts, 1, "{during_the_post}: only the first one went out");
            // both files are still in the vault
            assert_eq!(count(&v, "SELECT count(*) FROM booking_documents"), 2);
        }
    }

    /// A gateway between the app and the archive gives up after the archive
    /// may have taken the upload: nobody knows, so it is not sent again.
    #[tokio::test]
    async fn an_error_on_the_way_to_the_archive_is_as_unclear_as_no_answer() {
        for code in [502u16, 504, 408] {
            let server = archive(move |r| match r.method.as_str() {
                "POST" => status(code),
                _ => status(404),
            })
            .await;
            let dir = TempDir::new(&format!("archive-move-gateway-{code}"));
            let (v, _, doc) = with_file(&dir, &server);
            v.queue_archive_move(doc).unwrap();
            let report = v.clone().process_archive_moves().await.unwrap();
            assert_eq!(report.failed, 1, "{code}");
            let moves = v.archive_moves().unwrap();
            assert_eq!(moves[0].state, "failed");
            assert!(moves[0].problem.contains("Unklar"));
            v.clone().process_archive_moves().await.unwrap();
            let posts = server.seen().iter().filter(|r| r.method == "POST").count();
            assert_eq!(posts, 1, "{code}");
            assert_eq!(v.get_document_bytes(doc).unwrap(), pdf());
        }
    }

    #[tokio::test]
    async fn the_summary_counts_own_receipts_by_where_they_are() {
        let (server, _) = taking(vec![BROKEN]).await;
        let dir = TempDir::new("archive-move-summary");
        let (v, t1, doc) = with_file(&dir, &server);
        v.db()
            .unwrap()
            .execute(
                "INSERT INTO booking_documents(booking_id, title, pdf_bytes, source_url) VALUES (?1, 'Auszug', x'25504446', 'bank:1')",
                [t1],
            )
            .unwrap();
        let second = v
            .attach_document_to_booking(t1, "zwei.pdf".into(), pdf())
            .unwrap();
        let summary = |v: &Vault| v.archive_move_summary().unwrap();
        assert_eq!(
            summary(&v),
            ArchiveMoveSummary {
                in_vault: 2,
                waiting: 0,
                failed: 0
            }
        );
        v.queue_archive_move(doc).unwrap();
        assert_eq!(
            summary(&v),
            ArchiveMoveSummary {
                in_vault: 1,
                waiting: 1,
                failed: 0
            }
        );
        v.clone().process_archive_moves().await.unwrap();
        v.clone().process_archive_moves().await.unwrap();
        assert_eq!(
            summary(&v),
            ArchiveMoveSummary {
                in_vault: 1,
                waiting: 0,
                failed: 1
            }
        );
        v.delete_document(second).unwrap();
        assert_eq!(
            summary(&v),
            ArchiveMoveSummary {
                in_vault: 0,
                waiting: 0,
                failed: 1
            }
        );
    }

    // -------------------------------------------------------------- documents of the bank

    /// A vault connected to `server`, with booking T1 on the account "Konto"
    /// of the Musterbank. Returns the vault and that account.
    fn at_the_bank(dir: &TempDir, server: &Archive) -> (Arc<Vault>, i64) {
        let v = connected(dir, server);
        seed(&v, &[("T1", "2026-11-07", -23800, "Elektro Weber")]);
        v.db()
            .unwrap()
            .execute(
                "UPDATE accounts SET bank_name = 'Musterbank' WHERE external_ref = 'A1'",
                [],
            )
            .unwrap();
        let account = count(&v, "SELECT id FROM accounts WHERE external_ref = 'A1'");
        (v, account)
    }

    /// A statement the bank delivered for `account`, stored as the fetch
    /// stores it. None: the vault knows that document already.
    fn statement(v: &Vault, account: i64, source: &str, bytes: &[u8]) -> Option<i64> {
        crate::documents::insert_doc(
            &v.db().unwrap(),
            None,
            Some(account),
            "Kontoauszug 10/2026",
            "kontoauszug",
            "application/pdf",
            "auszug.pdf",
            bytes,
            source,
            "2026-11-01",
        )
        .unwrap()
    }

    fn statements(v: &Vault) -> Vec<DocumentInfo> {
        v.list_all_documents(DocumentFilter {
            doc_type: Some("kontoauszug".into()),
            ..Default::default()
        })
        .unwrap()
    }

    /// A document the bank delivered moves like a receipt of the user's own.
    /// Its reference stays what the file was (a statement of that account,
    /// unread) and keeps the origin the fetch knows it by: the document is
    /// not stored again, until the reference is deleted.
    #[tokio::test]
    async fn a_document_of_the_bank_moves_and_is_not_fetched_again() {
        let (server, _) = taking(vec![DONE]).await;
        let dir = TempDir::new("archive-bank-move");
        let (v, account) = at_the_bank(&dir, &server);
        // fetched long before the banks' documents were moved
        let old = statement(&v, account, "bank:alt", b"%PDF-1.4 alt").unwrap();
        v.db()
            .unwrap()
            .execute(
                "UPDATE booking_documents SET created_at = 1 WHERE id = ?1",
                [old],
            )
            .unwrap();
        assert!(!v.archive_bank_enabled().unwrap());
        assert_eq!(v.queue_bank_documents(aid(&v)).unwrap(), 0);

        v.set_archive_bank(true).unwrap();
        assert!(v.archive_bank_enabled().unwrap());
        let doc = statement(&v, account, "bank:1", &pdf()).unwrap();
        // meant for another archive than the connected one: nothing
        assert_eq!(v.queue_bank_documents("ein-anderes".into()).unwrap(), 0);
        // what arrived since goes, what was there before stays for now
        assert_eq!(v.queue_bank_documents(aid(&v)).unwrap(), 1);
        assert_eq!(v.queue_bank_documents(aid(&v)).unwrap(), 0);
        assert_eq!(v.archive_moves().unwrap()[0].doc_id, doc);
        assert_eq!(v.archive_move_summary().unwrap().in_vault, 1);

        v.clone().process_archive_moves().await.unwrap();
        let report = v.clone().process_archive_moves().await.unwrap();
        assert_eq!((report.moved, report.failed), (1, 0));

        let listed = statements(&v);
        assert_eq!(listed.len(), 2, "the old file and the reference");
        let moved = listed.iter().find(|d| d.id < 0).expect("a reference");
        assert_eq!(moved.doc_type, "kontoauszug");
        assert_eq!(moved.source_url, "bank:1");
        assert_eq!(moved.archive_doc, Some(77));
        assert_eq!(moved.account_id, Some(account));
        assert_eq!(moved.title, "Kontoauszug 10/2026");
        assert!(!moved.seen, "new from the bank: still unread");
        assert_eq!(v.unseen_document_count().unwrap(), 2);
        assert_eq!(v.mark_documents_seen(vec![moved.id], true).unwrap(), 1);
        assert!(statements(&v).iter().all(|d| d.id > 0 || d.seen));
        assert_eq!(v.unseen_document_count().unwrap(), 1);

        // the next fetch knows the document: not stored again
        assert!(crate::documents::knows_source(&v.db().unwrap(), "bank:1").unwrap());
        assert_eq!(statement(&v, account, "bank:1", &pdf()), None);
        assert_eq!(count(&v, "SELECT count(*) FROM booking_documents"), 1);

        // "everything that is there" takes the bank's older documents along
        // while the banks' documents are moved, and not otherwise
        v.set_archive_bank(false).unwrap();
        assert_eq!(v.archive_move_summary().unwrap().in_vault, 0);
        assert_eq!(v.queue_archive_moves(None, aid(&v)).unwrap(), 0);
        v.set_archive_bank(true).unwrap();
        assert_eq!(v.archive_move_summary().unwrap().in_vault, 1);
        assert_eq!(v.queue_archive_moves(None, aid(&v)).unwrap(), 1);
        assert_eq!(v.archive_moves().unwrap()[0].doc_id, old);
        v.cancel_archive_move(old).unwrap();

        // the reference deleted: the bank's document may come again
        v.delete_document(moved.id).unwrap();
        assert!(!crate::documents::knows_source(&v.db().unwrap(), "bank:1").unwrap());
        assert!(statement(&v, account, "bank:1", &pdf()).is_some());
    }

    /// GiroCrab knows bank, kind and account of a document the bank
    /// delivered and hands them over with the file: a sender the archive has
    /// is used (whatever its spelling), a kind it lacks is made, without a
    /// rule of its own; a tag the token may not make is left out and the
    /// file goes up all the same. A receipt of the user's own gets none of
    /// this, nor does a bank document once it is switched off.
    #[tokio::test]
    async fn a_document_of_the_bank_goes_up_with_bank_kind_and_account() {
        let server = archive(move |r| match (r.method.as_str(), r.path()) {
            ("GET", "/api/correspondents/") => {
                json(r#"{"results": [{"id": 3, "name": "MUSTERBANK"}]}"#)
            }
            // an archive that lists everything, whatever it was asked for
            ("GET", "/api/document_types/") => {
                json(r#"{"results": [{"id": 1, "name": "Rechnung"}]}"#)
            }
            ("POST", "/api/document_types/") => json(r#"{"id": 8, "name": "Kontoauszug"}"#),
            ("GET", "/api/tags/") => json(r#"{"results": []}"#),
            ("POST", "/api/tags/") => status(403),
            ("POST", "/api/documents/post_document/") => json("\"task-1\""),
            ("GET", "/api/tasks/") => json(DONE),
            ("GET", "/api/documents/") => json(r#"{"count": 0, "results": []}"#),
            ("GET", "/api/documents/77/download/") => Answer {
                status: 200,
                headers: Vec::new(),
                body: pdf(),
            },
            ("GET", "/api/documents/77/") => json(r#"{"id": 77, "deleted_at": null}"#),
            _ => status(404),
        })
        .await;
        let dir = TempDir::new("archive-bank-labels");
        let (v, account) = at_the_bank(&dir, &server);
        assert!(v.archive_labels_enabled().unwrap());
        let doc = statement(&v, account, "bank:1", &pdf()).unwrap();
        v.queue_archive_move(doc).unwrap();
        v.clone().process_archive_moves().await.unwrap();
        let report = v.clone().process_archive_moves().await.unwrap();
        assert_eq!((report.moved, report.failed), (1, 0));

        let seen = server.seen();
        let posted = |path: &str| -> Vec<String> {
            seen.iter()
                .filter(|r| r.method == "POST" && r.path() == path)
                .map(|r| String::from_utf8_lossy(&r.body).into_owned())
                .collect()
        };
        let upload = posted("/api/documents/post_document/");
        assert_eq!(upload.len(), 1);
        let field = |name: &str, value: &str| format!("name=\"{name}\"\r\n\r\n{value}\r\n");
        assert!(upload[0].contains(&field("correspondent", "3")));
        assert!(upload[0].contains(&field("document_type", "8")));
        assert!(!upload[0].contains("name=\"tags\""));
        // made without a rule of its own, under GiroCrab's word for the kind
        let made = posted("/api/document_types/");
        assert_eq!(made.len(), 1);
        let made: Value = serde_json::from_str(&made[0]).unwrap();
        assert_eq!(made["name"], "Kontoauszug");
        assert_eq!(made["matching_algorithm"], 0);
        // asked for by name; the tag is the account
        assert!(seen
            .iter()
            .any(|r| r.path() == "/api/correspondents/" && r.has("name__iexact=Musterbank")));
        assert!(seen
            .iter()
            .any(|r| r.path() == "/api/tags/" && r.has("name__iexact=Konto")));

        // switched off, and for a receipt of the user's own: the file alone
        let before = server.seen().len();
        v.set_archive_labels(false).unwrap();
        assert!(!v.archive_labels_enabled().unwrap());
        let second = statement(&v, account, "bank:2", b"%PDF-1.4 zweiter Auszug").unwrap();
        v.queue_archive_move(second).unwrap();
        v.set_archive_labels(true).unwrap();
        let own = v
            .attach_document_to_booking(id_of(&v, "T1"), "Bon.pdf".into(), b"%PDF-1.4 Bon".to_vec())
            .unwrap();
        v.set_archive_labels(false).unwrap();
        v.queue_archive_move(own).unwrap();
        v.clone().process_archive_moves().await.unwrap();
        let later = &server.seen()[before..];
        assert_eq!(
            later
                .iter()
                .filter(|r| r.path() == "/api/documents/post_document/")
                .count(),
            2
        );
        assert!(!later.iter().any(|r| {
            ["/api/correspondents/", "/api/document_types/", "/api/tags/"].contains(&r.path())
        }));
    }

    /// Every device fetches the bank's documents itself, and an origin means
    /// nothing on another device. A reference another device made (it came
    /// with the vault merge) therefore does not keep this device's fetch
    /// from storing its own copy; while it holds that file, the file is
    /// shown and not the reference. Moving it finds the very same file in
    /// the archive: nothing is uploaded, the account keeps the one
    /// reference, and this device now knows the document by it.
    #[tokio::test]
    async fn another_devices_reference_does_not_stand_for_this_devices_copy() {
        let server = archive(move |r| match (r.method.as_str(), r.path()) {
            // by checksum: document 5 is this very file
            ("GET", "/api/documents/") => json(r#"{"count": 1, "results": [{"id": 5}]}"#),
            ("GET", "/api/documents/5/download/") => Answer {
                status: 200,
                headers: Vec::new(),
                body: pdf(),
            },
            ("GET", "/api/documents/5/") => json(r#"{"id": 5, "deleted_at": null}"#),
            _ => status(404),
        })
        .await;
        let dir = TempDir::new("archive-bank-other-device");
        let (v, account) = at_the_bank(&dir, &server);
        let id = aid(&v);
        v.db()
            .unwrap()
            .execute(
                "INSERT INTO document_links(uuid, account_id, url, title, doc_date, updated_ms,
                        rev, archive_id, archive_doc, doc_type)
                 VALUES ('u-1', ?1, 'https://archiv.example/documents/5/details',
                         'Kontoauszug 10/2026', '2026-11-01', 5, 'r', ?2, 5, 'kontoauszug')",
                params![account, id],
            )
            .unwrap();
        assert_eq!(statements(&v).len(), 1);
        // the fetch here does not know the document by that reference
        assert!(!crate::documents::knows_source(&v.db().unwrap(), "bank:1").unwrap());
        let doc = statement(&v, account, "bank:1", &pdf()).expect("stored");
        // one document, one line: the file this device holds
        let listed = statements(&v);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, doc);

        v.queue_archive_move(doc).unwrap();
        v.clone().process_archive_moves().await.unwrap();
        let report = v.clone().process_archive_moves().await.unwrap();
        assert_eq!((report.moved, report.failed), (1, 0));
        assert!(
            server.seen().iter().all(|r| r.method == "GET"),
            "nothing was uploaded"
        );
        assert_eq!(count(&v, "SELECT count(*) FROM booking_documents"), 0);
        assert_eq!(
            count(
                &v,
                "SELECT count(*) FROM document_links WHERE deleted_ms IS NULL"
            ),
            1
        );
        let listed = statements(&v);
        assert_eq!(listed.len(), 1);
        assert_eq!(
            (listed[0].archive_doc, listed[0].source_url.as_str()),
            (Some(5), "bank:1")
        );
        assert_eq!(statement(&v, account, "bank:1", &pdf()), None);
    }

    /// "Keep in the vault" lasts: neither the next fetch nor "everything
    /// that is there" queues the file again. Only asked for by itself it
    /// goes after all.
    #[tokio::test]
    async fn a_file_kept_in_the_vault_is_not_queued_again_by_itself() {
        let (server, _) = taking(vec![DONE]).await;
        let dir = TempDir::new("archive-bank-kept");
        let (v, account) = at_the_bank(&dir, &server);
        v.set_archive_bank(true).unwrap();
        let doc = statement(&v, account, "bank:1", &pdf()).unwrap();
        assert_eq!(v.queue_bank_documents(aid(&v)).unwrap(), 1);
        v.cancel_archive_move(doc).unwrap();
        // nothing is going on with it, and nothing queues it again
        assert!(v.archive_moves().unwrap().is_empty());
        let summary = v.archive_move_summary().unwrap();
        assert_eq!(
            (summary.in_vault, summary.waiting, summary.failed),
            (0, 0, 0)
        );
        assert_eq!(v.queue_bank_documents(aid(&v)).unwrap(), 0);
        assert_eq!(v.queue_archive_moves(None, aid(&v)).unwrap(), 0);
        let report = v.clone().process_archive_moves().await.unwrap();
        assert_eq!((report.moved, report.waiting), (0, 0));
        assert!(server.seen().is_empty());
        // a file that was not queued yet can be kept too
        let other = statement(&v, account, "bank:2", b"%PDF-1.4 zwei").unwrap();
        v.cancel_archive_move(other).unwrap();
        assert_eq!(v.queue_bank_documents(aid(&v)).unwrap(), 0);
        // keeping a file that is not there is nothing
        v.cancel_archive_move(9999).unwrap();

        // asked for by itself it goes after all
        v.queue_archive_move(doc).unwrap();
        assert_eq!(v.archive_moves().unwrap().len(), 1);
        v.clone().process_archive_moves().await.unwrap();
        assert_eq!(v.clone().process_archive_moves().await.unwrap().moved, 1);
        assert_eq!(count(&v, "SELECT count(*) FROM booking_documents"), 1);
    }

    /// The user keeps the file in the vault while the archive is still asked
    /// where to file it: the file does not go up after all.
    #[tokio::test]
    async fn a_file_kept_while_the_archive_is_asked_does_not_go_up() {
        let asked_for: Arc<Mutex<Option<(Arc<Vault>, i64)>>> = Arc::default();
        let held = asked_for.clone();
        let server = archive(move |r| match (r.method.as_str(), r.path()) {
            ("GET", "/api/documents/") => json(r#"{"count": 0, "results": []}"#),
            ("GET", "/api/correspondents/") => {
                if let Some((v, doc)) = held.lock().unwrap().as_ref() {
                    v.cancel_archive_move(*doc).unwrap();
                }
                json(r#"{"results": [{"id": 3, "name": "Musterbank"}]}"#)
            }
            ("POST", "/api/documents/post_document/") => json("\"task-1\""),
            _ => status(404),
        })
        .await;
        let dir = TempDir::new("archive-bank-kept-meanwhile");
        let (v, account) = at_the_bank(&dir, &server);
        let doc = statement(&v, account, "bank:1", &pdf()).unwrap();
        *asked_for.lock().unwrap() = Some((v.clone(), doc));
        v.queue_archive_move(doc).unwrap();
        v.clone().process_archive_moves().await.unwrap();
        *asked_for.lock().unwrap() = None;
        let seen = server.seen();
        assert!(seen.iter().any(|r| r.path() == "/api/correspondents/"));
        assert!(!seen.iter().any(|r| r.method == "POST"), "not uploaded");
        assert_eq!(count(&v, "SELECT count(*) FROM booking_documents"), 1);
        assert!(v.archive_moves().unwrap().is_empty());
    }

    /// A booking that carries the archive's document already (linked by
    /// hand) stands for the bank's file that moves onto it: no second
    /// reference, and the fetch knows the document by the one that is there.
    #[tokio::test]
    async fn a_booking_that_has_the_document_stands_for_its_origin() {
        let (server, _) = taking(vec![DONE]).await;
        let dir = TempDir::new("archive-bank-origin");
        let (v, account) = at_the_bank(&dir, &server);
        let t1 = id_of(&v, "T1");
        v.attach_archive_document(t1, aid(&v), 77, "Abrechnung".into())
            .unwrap();
        let doc = crate::documents::insert_doc(
            &v.db().unwrap(),
            Some(t1),
            Some(account),
            "Abrechnung",
            "abrechnung",
            "application/pdf",
            "abrechnung.pdf",
            &pdf(),
            "bank:9",
            "2026-11-07",
        )
        .unwrap()
        .unwrap();
        v.queue_archive_move(doc).unwrap();
        v.clone().process_archive_moves().await.unwrap();
        let report = v.clone().process_archive_moves().await.unwrap();
        assert_eq!(report.moved, 1);
        assert_eq!(count(&v, "SELECT count(*) FROM booking_documents"), 0);
        let docs = v.list_documents_for_booking(t1).unwrap();
        assert_eq!(docs.len(), 1, "still one reference");
        assert_eq!(docs[0].source_url, "bank:9");
        // it is a document of the bank now, of the kind the file had
        assert_eq!(docs[0].doc_type, "abrechnung");
        assert!(docs[0].from_bank);
        assert!(crate::documents::knows_source(&v.db().unwrap(), "bank:9").unwrap());
        // the reference removed: the document may come again
        v.delete_document(docs[0].id).unwrap();
        assert!(!crate::documents::knows_source(&v.db().unwrap(), "bank:9").unwrap());
    }
}

// ------------------------------------------------------------------ against a real archive

/// The whole way against a real archive, for whoever has one at hand:
///
/// ```text
/// GIROCRAB_LIVE_ARCHIVE=http://127.0.0.1:8000 GIROCRAB_LIVE_ARCHIVE_TOKEN=… \
///     cargo test -p girocrab-core --lib live_archive -- --ignored --nocapture
/// ```
///
/// It uploads one small made-up invoice (a new one per run), creates the
/// four custom fields if they are missing, and deletes the invoice again at
/// the end. Use a test instance.
mod live_archive {
    use std::time::Duration;

    use super::super::moves::*;
    use super::*;

    fn target() -> Option<(String, String)> {
        Some((
            std::env::var("GIROCRAB_LIVE_ARCHIVE").ok()?,
            std::env::var("GIROCRAB_LIVE_ARCHIVE_TOKEN").ok()?,
        ))
    }

    /// The archive's own answer, read without the app's client.
    async fn raw(base: &str, token: &str, path: &str) -> Value {
        let answer = reqwest::Client::new()
            .get(format!("{base}{path}"))
            .header("Authorization", format!("Token {token}"))
            .header("Accept", "application/json; version=9")
            .send()
            .await
            .unwrap();
        crate::connectors::read_json_or_null(answer).await
    }

    /// Runs the queue until nothing waits (the archive reads the text of a
    /// new file, which takes a while).
    async fn moved(v: &Arc<Vault>) -> ArchiveMoveReport {
        let mut total = ArchiveMoveReport::default();
        for _ in 0..90 {
            let report = v.clone().process_archive_moves().await.unwrap();
            total.moved += report.moved;
            total.failed += report.failed;
            if report.waiting == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        total
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs a real archive: GIROCRAB_LIVE_ARCHIVE and GIROCRAB_LIVE_ARCHIVE_TOKEN"]
    async fn the_whole_way() {
        let Some((base, token)) = target() else {
            panic!("set GIROCRAB_LIVE_ARCHIVE and GIROCRAB_LIVE_ARCHIVE_TOKEN");
        };
        let dir = TempDir::new("archive-live");
        let v = open(&dir, "v.db");
        v.set_archive_connection(base.clone(), token.clone(), None)
            .unwrap();
        let info = v.clone().archive_test().await.unwrap();
        println!("connected: {info:?}");

        // a booking of today and the invoice for it, new with every run
        let today = crate::dates::today();
        let day = crate::dates::iso(today);
        let run = uuid::Uuid::new_v4().simple().to_string();
        let cents = -(10_000 + (u64::from_str_radix(&run[..4], 16).unwrap() % 80_000) as i64);
        let amount = format!("{},{:02}", cents.abs() / 100, cents.abs() % 100);
        seed(
            &v,
            &[
                ("T1", &day, cents, "Elektro Weber"),
                ("T2", &day, -1234, "Niemand Sonst"),
            ],
        );
        let (t1, t2) = (id_of(&v, "T1"), id_of(&v, "T2"));
        let pdf = crate::documents::simple_pdf(&[
            "Elektro Weber GmbH".into(),
            format!("Rechnung {run}"),
            format!("Rechnungsdatum {}", today.strftime("%d.%m.%Y")),
            format!("Gesamtbetrag {amount} EUR"),
        ]);

        // --- a stored file moves into the archive
        let stored = v
            .attach_document_to_booking(t1, format!("Rechnung {run}.pdf"), pdf.clone())
            .unwrap();
        v.queue_archive_move(stored).unwrap();
        let report = moved(&v).await;
        println!("moved: {report:?} {:?}", v.archive_moves().unwrap());
        assert_eq!((report.moved, report.failed), (1, 0));
        let docs = v.list_documents_for_booking(t1).unwrap();
        assert_eq!(docs.len(), 1);
        let reference = docs[0].clone();
        let archive_doc = reference.archive_doc.expect("a document of the archive");
        assert!(
            v.get_document_bytes(stored).is_err(),
            "the file left the vault"
        );
        println!(
            "document {archive_doc}: {:?} ({})",
            reference.title, reference.link_url
        );

        // --- it can be shown: the original byte for byte, the archive's version, a thumbnail
        let original = v.clone().archive_file(reference.id, true).await.unwrap();
        assert_eq!(original.bytes, pdf);
        let shown = v.clone().archive_file(reference.id, false).await.unwrap();
        println!(
            "shown: {} bytes {} {:?}",
            shown.bytes.len(),
            shown.mime_type,
            shown.file_name
        );
        assert!(shown.bytes.starts_with(b"%PDF"));
        let thumbnail = v.clone().archive_thumbnail(archive_doc).await.unwrap();
        assert!(!thumbnail.is_empty());

        // --- it is found by its text (the index may take a moment) and marked as linked
        let mut found = Vec::new();
        for _ in 0..20 {
            found = v
                .clone()
                .archive_search(run.clone(), None, None, 10)
                .await
                .unwrap();
            if !found.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        println!("found: {found:?}");
        let hit = found
            .iter()
            .find(|d| d.id == archive_doc)
            .expect("found by its number");
        assert!(hit.linked);
        assert_eq!(hit.created, day);

        // --- the same file again: the archive knows it, nothing is doubled
        let again = v
            .attach_document_to_booking(t2, format!("Kopie {run}.pdf"), pdf.clone())
            .unwrap();
        v.queue_archive_move(again).unwrap();
        let report = moved(&v).await;
        println!("again: {report:?} {:?}", v.archive_moves().unwrap());
        assert_eq!((report.moved, report.failed), (1, 0));
        let copy = v.list_documents_for_booking(t2).unwrap().remove(0);
        assert_eq!(copy.archive_doc, Some(archive_doc));

        v.delete_document(copy.id).unwrap();

        // --- unlinked, the document is suggested for its booking and matched in one pass
        v.delete_document(reference.id).unwrap();
        let suggested = v.clone().archive_suggestions(t1, 5).await.unwrap();
        println!(
            "suggested: {:?}",
            suggested
                .iter()
                .map(|s| (s.document.id, s.score))
                .collect::<Vec<_>>()
        );
        assert_eq!(suggested[0].document.id, archive_doc);
        let matches = v.clone().archive_matches(None, None, 50).await.unwrap();
        println!(
            "matches: {:?}",
            matches
                .matches
                .iter()
                .map(|m| (m.booking_id, m.document.id, m.score, m.ambiguous))
                .collect::<Vec<_>>()
        );
        let pair = matches
            .matches
            .iter()
            .find(|m| m.booking_id == t1)
            .expect("T1 has a partner");
        assert_eq!(pair.document.id, archive_doc);
        assert!(!pair.ambiguous && pair.score >= 8);
        assert_eq!(pair.document.correspondent, hit.correspondent);

        // --- tidy up: the invoice of this run goes (to the archive's trash)
        let gone = reqwest::Client::new()
            .post(format!("{base}/api/documents/bulk_edit/"))
            .header("Authorization", format!("Token {token}"))
            .json(&serde_json::json!({"documents": [archive_doc], "method": "delete", "parameters": {}}))
            .send()
            .await
            .unwrap();
        println!("deleted: {}", gone.status());
    }

    /// A statement a bank delivered goes into a real archive with what
    /// GiroCrab knows about it (bank, kind, account), a second one finds
    /// those again instead of making them twice, and the fetch knows both
    /// afterwards. Leaves two documents in the archive's trash and the
    /// document type "Kontoauszug" behind.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs a real archive: GIROCRAB_LIVE_ARCHIVE and GIROCRAB_LIVE_ARCHIVE_TOKEN"]
    async fn a_document_of_the_bank() {
        let Some((base, token)) = target() else {
            panic!("set GIROCRAB_LIVE_ARCHIVE and GIROCRAB_LIVE_ARCHIVE_TOKEN");
        };
        let dir = TempDir::new("archive-live-bank");
        let v = open(&dir, "v.db");
        v.set_archive_connection(base.clone(), token.clone(), None)
            .unwrap();
        seed(&v, &[("T1", "2026-11-07", -100, "Bank")]);
        // names of this run only: what is made for them is removed again
        let run = uuid::Uuid::new_v4().simple().to_string();
        let (bank, account_name) = (format!("Testbank {run}"), format!("Testkonto {run}"));
        let account: i64 = {
            let db = v.db().unwrap();
            db.execute(
                "UPDATE accounts SET bank_name = ?1, name = ?2 WHERE external_ref = 'A1'",
                params![bank, account_name],
            )
            .unwrap();
            db.query_row(
                "SELECT id FROM accounts WHERE external_ref = 'A1'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        v.set_archive_bank(true).unwrap();
        let mut archive_docs: Vec<i64> = Vec::new();
        for n in 1..=2 {
            let source = format!("live:{run}:{n}");
            let title = format!("Kontoauszug {n} {run}");
            let bytes = crate::documents::simple_pdf(&[title.clone(), bank.clone()]);
            crate::documents::insert_doc(
                &v.db().unwrap(),
                None,
                Some(account),
                &title,
                "kontoauszug",
                "application/pdf",
                "auszug.pdf",
                &bytes,
                &source,
                "2026-11-01",
            )
            .unwrap()
            .expect("stored");
            assert_eq!(v.queue_bank_documents(aid(&v)).unwrap(), 1);
            let mut moved = 0;
            for _ in 0..60 {
                let report = v.clone().process_archive_moves().await.unwrap();
                moved += report.moved;
                if report.waiting == 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            assert_eq!(moved, 1, "{:?}", v.archive_moves().unwrap());
            let listed = v
                .list_all_documents(crate::documents::DocumentFilter {
                    doc_type: Some("kontoauszug".into()),
                    search: Some(title.clone()),
                    ..Default::default()
                })
                .unwrap();
            assert_eq!(listed.len(), 1);
            assert_eq!(listed[0].source_url, source);
            assert!(crate::documents::knows_source(&v.db().unwrap(), &source).unwrap());
            archive_docs.push(listed[0].archive_doc.expect("in the archive"));
        }

        // what the archive itself says about the two documents
        let mut labels: Vec<(String, i64)> = Vec::new();
        for doc in &archive_docs {
            let document = raw(&base, &token, &format!("/api/documents/{doc}/")).await;
            let sender = document["correspondent"].as_i64().expect("a sender");
            let kind = document["document_type"].as_i64().expect("a type");
            let tags: Vec<i64> = document["tags"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_i64)
                .collect();
            assert_eq!(tags.len(), 1, "the account");
            labels.push(("correspondents".into(), sender));
            labels.push(("document_types".into(), kind));
            labels.push(("tags".into(), tags[0]));
        }
        // the second statement found the first one's sender, type and tag
        assert_eq!(labels[..3], labels[3..]);
        for ((list, id), name) in
            labels[..3]
                .iter()
                .zip([bank.as_str(), "Kontoauszug", account_name.as_str()])
        {
            let entry = raw(&base, &token, &format!("/api/{list}/{id}/")).await;
            println!(
                "{list}: {} (rule {})",
                entry["name"], entry["matching_algorithm"]
            );
            assert_eq!(entry["name"], name);
        }
        // made for this run: without a rule of its own
        for (list, id) in [&labels[0], &labels[2]] {
            let entry = raw(&base, &token, &format!("/api/{list}/{id}/")).await;
            assert_eq!(entry["matching_algorithm"], 0, "{list}");
        }

        // --- tidy up: the documents go to the trash, this run's sender and tag go
        let http = reqwest::Client::new();
        let gone = http
            .post(format!("{base}/api/documents/bulk_edit/"))
            .header("Authorization", format!("Token {token}"))
            .json(&serde_json::json!({"documents": archive_docs, "method": "delete", "parameters": {}}))
            .send()
            .await
            .unwrap();
        println!("deleted: {}", gone.status());
        for (list, id) in [&labels[0], &labels[2]] {
            let gone = http
                .delete(format!("{base}/api/{list}/{id}/"))
                .header("Authorization", format!("Token {token}"))
                .send()
                .await
                .unwrap();
            println!("removed from {list}: {}", gone.status());
        }
    }
}
