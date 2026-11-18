//! HTTP client for the document archive: one configured base address and
//! the headers its kind of archive wants (token, API version).
//!
//! Rules that keep the token and the documents where they belong:
//! - Redirects are never followed, so the token is never sent anywhere but
//!   to the configured address.
//! - Plain `http` is only spoken to addresses in the user's own network. The
//!   name is resolved here and the connection pinned to the addresses that
//!   were checked, without a proxy.
//! - Every URL is built from the configured base; a `next` link from a
//!   response is never followed.
//! - Errors never carry the address, the token or anything the server said:
//!   `reqwest`'s own messages contain the URL, so they are replaced here.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use reqwest::header::HeaderMap;
use reqwest::{Method, StatusCode};
use url::Url;
use zeroize::Zeroizing;

use crate::error::{CoreError, CoreResult};
use crate::i18n::{tr, trf};

/// How long resolving the archive's name may take.
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(10);

/// Whether `ip` is an address of the user's own network: loopback, private
/// (RFC 1918), link-local, carrier-grade NAT (VPN meshes) and the IPv6
/// equivalents. An IPv4-mapped IPv6 address counts as its IPv4 address.
pub(crate) fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || (o[0] == 100 && (o[1] & 0b1100_0000) == 64)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_private_ip(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            v6.is_loopback() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
    }
}

fn unreachable() -> CoreError {
    CoreError::Network {
        message: tr("Das Dokumentenarchiv ist nicht erreichbar.").into(),
    }
}

/// The error for a response that is not a success. Says what to do, never
/// what the server sent.
fn status_error(status: StatusCode) -> CoreError {
    match status.as_u16() {
        401 | 403 => CoreError::Authentication {
            message: tr("Das Dokumentenarchiv hat die Anmeldung abgelehnt. Bitte Token und Berechtigungen prüfen.").into(),
        },
        404 => CoreError::NotFound {
            what: tr("Dokument im Archiv").into(),
        },
        406 => CoreError::Protocol {
            message: tr("Das Dokumentenarchiv unterstützt die benötigte API-Version nicht. Bitte das Archiv aktualisieren.").into(),
        },
        300..=399 => CoreError::Protocol {
            message: tr("Das Dokumentenarchiv leitet auf eine andere Adresse um. Bitte die endgültige Adresse eintragen.").into(),
        },
        // the archive is there and understood the request, but does not
        // take it (a file it does not accept, a value it does not allow)
        code @ (400 | 409 | 413 | 415 | 422) => CoreError::InvalidInput {
            message: trf(
                "Das Dokumentenarchiv hat die Anfrage abgelehnt (HTTP {0}).",
                &[&code],
            ),
        },
        code => CoreError::Network {
            message: trf(
                "Das Dokumentenarchiv meldet einen Fehler (HTTP {0}).",
                &[&code],
            ),
        },
    }
}

fn too_large() -> CoreError {
    CoreError::Network {
        message: tr("Die Antwort des Dokumentenarchivs ist zu groß.").into(),
    }
}

/// How a request failed.
#[derive(Debug)]
pub(crate) enum Failure {
    /// No connection could be made: the archive never saw the request.
    NotSent,
    /// The request went out, but no complete answer came back: the archive
    /// may or may not have acted on it.
    Unclear,
    /// The archive answered with this status and did not do what was asked.
    Status(StatusCode),
    /// The answer is larger than allowed (it was a success).
    TooLarge,
}

impl From<Failure> for CoreError {
    fn from(f: Failure) -> Self {
        match f {
            Failure::NotSent | Failure::Unclear => unreachable(),
            Failure::Status(status) => status_error(status),
            Failure::TooLarge => too_large(),
        }
    }
}

pub(crate) struct Client {
    base: String,
    /// Sent with every request: how this kind of archive wants the token and
    /// its API version named (`backend.rs`). Wiped on drop.
    headers: Vec<(&'static str, Zeroizing<String>)>,
    http: reqwest::Client,
    /// The vault's counter and its value when the work began.
    watch: Watch,
}

/// The value of a vault's `archive_epoch` once the app has locked the vault:
/// this `Vault` sends no archive request any more.
pub(crate) const CLOSED: u64 = u64::MAX;

/// A vault's `archive_epoch` and the value it had when some work began.
pub(crate) type Watch = (std::sync::Arc<std::sync::atomic::AtomicU64>, u64);

/// Whether the work a watch belongs to has ended since it began, or the
/// vault is closed for good (also for work that only begins now).
fn ended((epoch, began): &Watch) -> bool {
    let now = epoch.load(std::sync::atomic::Ordering::SeqCst);
    now != *began || now == CLOSED
}

/// A response body with the headers it came with.
pub(crate) struct Reply {
    pub(crate) headers: HeaderMap,
    pub(crate) body: Vec<u8>,
}

impl Reply {
    pub(crate) fn json(&self) -> CoreResult<serde_json::Value> {
        serde_json::from_slice(&self.body).map_err(|_| CoreError::Protocol {
            message: tr("Das Dokumentenarchiv hat unerwartet geantwortet.").into(),
        })
    }

    pub(crate) fn header(&self, name: &str) -> String {
        self.headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string()
    }
}

impl Client {
    /// A client for the archive at `base` (checked by `archive::parse_base`)
    /// that sends `headers` with every request, bound to the state the
    /// vault was in when the work began (`watch`): once the app has locked
    /// the vault or the connection was changed (`Vault::archive_stop`),
    /// nothing goes out any more, not even the lookup of the archive's
    /// name. A front end that drops an async call does not end it in the
    /// core; this does.
    pub(crate) async fn new(
        base: &Url,
        headers: Vec<(&'static str, Zeroizing<String>)>,
        watch: Watch,
    ) -> CoreResult<Self> {
        if ended(&watch) {
            return Err(Failure::NotSent.into());
        }
        let mut builder = reqwest::Client::builder()
            .user_agent(concat!(
                "GiroCrab/",
                env!("CARGO_PKG_VERSION"),
                " (open source; +https://github.com/)"
            ))
            .timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none());
        if base.scheme() == "http" {
            let host = base.host_str().unwrap_or_default().to_string();
            let port = base.port_or_known_default().unwrap_or(80);
            let lookup = host.trim_start_matches('[').trim_end_matches(']');
            // the lookup has no timeout of its own
            let addrs: Vec<SocketAddr> =
                tokio::time::timeout(LOOKUP_TIMEOUT, tokio::net::lookup_host((lookup, port)))
                    .await
                    .map_err(|_| unreachable())?
                    .map_err(|_| unreachable())?
                    .collect();
            if addrs.is_empty() {
                return Err(unreachable());
            }
            if !addrs.iter().all(|a| is_private_ip(a.ip())) {
                return Err(CoreError::InvalidInput {
                    message: tr("Unverschlüsselt (http) verbindet sich GiroCrab nur mit Adressen im eigenen Netz. Bitte https verwenden.").into(),
                });
            }
            builder = builder.resolve_to_addrs(&host, &addrs).no_proxy();
        }
        Ok(Self {
            base: base.as_str().trim_end_matches('/').to_string(),
            headers,
            http: builder.build().map_err(|_| unreachable())?,
            watch,
        })
    }

    fn url(&self, path: &str, query: &[(&str, String)]) -> CoreResult<Url> {
        let mut url = Url::parse(&format!("{}{path}", self.base)).map_err(|_| unreachable())?;
        if !query.is_empty() {
            url.query_pairs_mut()
                .extend_pairs(query.iter().map(|(k, v)| (*k, v.as_str())));
        }
        Ok(url)
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<(&str, Vec<u8>)>,
        limit: usize,
    ) -> CoreResult<Reply> {
        Ok(self.send_checked(method, path, query, body, limit).await?)
    }

    /// [`send`](Self::send), telling apart how a request failed: a writing
    /// request must know whether the archive may have acted on it.
    async fn send_checked(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<(&str, Vec<u8>)>,
        limit: usize,
    ) -> Result<Reply, Failure> {
        if ended(&self.watch) {
            return Err(Failure::NotSent);
        }
        let url = self.url(path, query).map_err(|_| Failure::NotSent)?;
        let mut req = self.http.request(method, url);
        for (name, value) in &self.headers {
            req = req.header(*name, value.as_str());
        }
        if let Some((content_type, bytes)) = body {
            req = req.header("Content-Type", content_type).body(bytes);
        }
        let resp = req.send().await.map_err(|e| {
            // no connection at all: nothing was sent
            if e.is_connect() {
                Failure::NotSent
            } else {
                Failure::Unclear
            }
        })?;
        let status = resp.status();
        if !status.is_success() {
            return Err(Failure::Status(status));
        }
        let headers = resp.headers().clone();
        // the request went out: an answer that breaks off says nothing
        // about what the archive did with it
        let body = crate::connectors::read_body_capped(resp, limit)
            .await
            .map_err(|e| match e {
                crate::connectors::BodyError::TooLarge => Failure::TooLarge,
                crate::connectors::BodyError::Read(_) => Failure::Unclear,
            })?;
        Ok(Reply { headers, body })
    }

    /// `GET path?query`, at most `limit` bytes.
    pub(crate) async fn get(
        &self,
        path: &str,
        query: &[(&str, String)],
        limit: usize,
    ) -> CoreResult<Reply> {
        self.send(Method::GET, path, query, None, limit).await
    }

    /// [`get`](Self::get), telling apart how it failed.
    pub(crate) async fn get_checked(
        &self,
        path: &str,
        query: &[(&str, String)],
        limit: usize,
    ) -> Result<Reply, Failure> {
        self.send_checked(Method::GET, path, query, None, limit)
            .await
    }

    /// `POST path` with a body of the given content type (an upload). The
    /// failure says whether the archive may have taken it.
    pub(crate) async fn post(
        &self,
        path: &str,
        content_type: &str,
        body: Vec<u8>,
    ) -> Result<Reply, Failure> {
        self.send_checked(
            Method::POST,
            path,
            &[],
            Some((content_type, body)),
            crate::connectors::MAX_TEXT_BYTES,
        )
        .await
    }

    /// `POST path` with a JSON body. The failure says whether the archive
    /// may have acted on it.
    pub(crate) async fn post_json(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<Reply, Failure> {
        let bytes = serde_json::to_vec(body).map_err(|_| Failure::NotSent)?;
        self.send_checked(
            Method::POST,
            path,
            &[],
            Some(("application/json", bytes)),
            crate::connectors::MAX_TEXT_BYTES,
        )
        .await
    }
}
