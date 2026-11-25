//! Headless FNZ portal access with the login form's own credentials
//! (Zugangsnummer + PIN) instead of the browser window. The login page is a
//! plain server form on `portal.fnz.de`; afterwards the same validated REST
//! calls run that the portal snapshot scripts make in the web view, and the
//! result goes through the same snapshot parser as the browser import – see
//! `docs/fnz-connector.md`.
//!
//! The session prefix is session data of this sync only: never persisted,
//! never logged. The PIN stays in the vault, the cookie store dies with the
//! connector, and only `portal.fnz.de` ever sees credentials or cookies
//! (`ensure_fnz`). FNZsecure approvals cannot be driven by GiroCrab: the
//! login phase verifies the session with the same read-only REST call the
//! fetch uses and reports a decoupled approval while it is still missing.

use std::sync::LazyLock;

use async_trait::async_trait;
use regex::Regex;
use serde_json::{Value, json};
use zeroize::Zeroize;

use super::{
    document_account, document_date, document_source, document_title, field, invalid, open_rows,
};
use crate::connectors::{
    BankConnector, Challenge, ChallengeKind, ChallengeResponse, ConnectContext, FetchResult,
    KnownDocuments, Phase, today,
};
use crate::documents::RemoteDocument;
use crate::error::{CoreError, CoreResult};
use crate::i18n::{tr, trf};

/// FNZ entry point; redirects to the login of the chosen brand.
const ENTRY: &str = "https://portal.fnz.de/finvesto";

/// Portal release the validated requests run against; the portal scripts send
/// the same value. If FNZ ships a new release and drops the old one, the
/// fetch fails with a clear message instead of importing broken data.
const CLIENT_VERSION: &str = "26.4.01";

/// Poll interval while the FNZsecure approval is still missing.
const APPROVAL_POLL_SECONDS: u32 = 5;

/// Upper bound for paged portal lists, like the snapshot script's pages loop.
const MAX_PAGES: usize = 1000;
/// Largest list the portal may report.
const MAX_ROWS: usize = 100_000;
/// Mailbox page size (`indexFrom`/`indexTo`) and the same loop bound.
const DOCUMENTS_PAGE: usize = 100;
/// Bytes of mailbox PDFs one sync keeps in memory before the rest stays due
/// for the next sync (`documents.state` remember nothing of the deferred).
const SYNC_DOC_BYTES: usize = 64 * 1024 * 1024;
/// Broken documents in a row the mailbox loop tolerates before it stops for
/// this sync (the next tries again, the ids of the failures stay due).
const MAX_DOC_FAILURES: u32 = 10;

fn page_changed() -> CoreError {
    CoreError::protocol(tr(
        "FNZ: Die Anmeldung hat unerwartet geantwortet. Wahrscheinlich wurde die Portal-Schnittstelle geändert; bitte GiroCrab aktualisieren.",
    ))
}

fn wrong_credentials() -> CoreError {
    CoreError::Authentication {
        message: tr("FNZ: Anmeldung fehlgeschlagen. Bitte Zugangsnummer und PIN prüfen.").into(),
    }
}

fn session_ended() -> CoreError {
    CoreError::Authentication {
        message: tr("FNZ: Die Anmeldung ist nicht mehr gültig. Bitte den Abruf erneut starten.")
            .into(),
    }
}

fn rejected(status: reqwest::StatusCode) -> CoreError {
    CoreError::Bank {
        code: status.as_u16().to_string(),
        message: trf(
            "FNZ: Das Portal hat die Anfrage abgelehnt (HTTP {0}).",
            &[&status.as_str()],
        ),
    }
}

fn unavailable() -> CoreError {
    invalid("FNZ: Die angeforderten Daten sind derzeit nicht verfügbar.")
}

/// FNZsecure verlangt die Freigabe in der FNZsecure-App; ein Code kann die
/// Anmeldung nicht abschließen.
fn approval_needs_app() -> CoreError {
    CoreError::invalid(tr(
        "FNZsecure verlangt die Freigabe in der FNZsecure-App; ein Code kann die Anmeldung nicht abschließen. Bitte in der App bestätigen.",
    ))
}

fn network(message: &'static str) -> CoreError {
    CoreError::Network {
        message: tr(message).into(),
    }
}

/// Only `https://portal.fnz.de` may receive the credentials, cookies or the
/// session prefix.
fn ensure_fnz(url: &url::Url) -> CoreResult<()> {
    if url.scheme() == "https" && url.host_str() == Some("portal.fnz.de") {
        Ok(())
    } else {
        Err(network(
            "FNZ: Unerwartete Weiterleitung. Der Abruf wurde abgebrochen.",
        ))
    }
}

/// A completed portal page loop: all rows plus the last page's metadata (the
/// cash account detail reads `cashAccountId` and the IBAN from it).
struct ListPage {
    rows: Vec<Value>,
    metadata: Value,
}

/// Result of the session check: approved and usable, or still waiting for the
/// FNZsecure approval.
enum Verified {
    Ready,
    AwaitingApproval,
}

enum Check {
    Verified(Verified),
    Failed(CoreError),
}

// ------------------------------------------------------------------ login page

fn has_password_field(form: &str) -> bool {
    use crate::connectors::html::{INPUT, attr, attrs};
    INPUT
        .captures_iter(form)
        .any(|c| attr(&attrs(&c[1]), "type").is_some_and(|t| t.eq_ignore_ascii_case("password")))
}

/// The page is still the login (wrong credentials or the session ended).
fn is_login_html(html: &str) -> bool {
    use crate::connectors::html::FORM;
    FORM.captures_iter(html).any(|f| has_password_field(&f[2]))
}

fn is_login_path(url: &url::Url) -> bool {
    url.path().contains("/login")
}

/// `__doPostBack('<target>','')` targets in the page. `href` attributes carry
/// HTML-escaped quotes, so the text is entity-decoded first.
static POSTBACK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"__doPostBack\('([^']+)'"#).unwrap());

/// The login form of the portal, as the browser would submit it.
#[derive(Debug)]
struct Login {
    action: url::Url,
    /// Hidden fields (`__VIEWSTATE`, …) with their current values.
    state: Vec<(String, String)>,
    user_field: String,
    pin_field: String,
    /// `__EVENTTARGET` of the login command link.
    submit: String,
}

impl Login {
    /// The request body. Every value is wiped again after the POST.
    fn filled(&self, user: &str, pin: &str) -> Vec<(String, String)> {
        let mut out = Vec::with_capacity(self.state.len() + 4);
        out.push(("__EVENTTARGET".into(), self.submit.clone()));
        out.push(("__EVENTARGUMENT".into(), String::new()));
        out.extend(self.state.iter().cloned());
        out.push((self.user_field.clone(), user.to_string()));
        out.push((self.pin_field.clone(), pin.to_string()));
        out
    }
}

/// The filled login form: every value (the PIN included) is wiped again when
/// the guard is dropped, also when the request future is dropped mid-flight.
struct WipeFields(Vec<(String, String)>);

impl Drop for WipeFields {
    fn drop(&mut self) {
        for (_, v) in self.0.iter_mut() {
            v.zeroize();
        }
    }
}

fn login_form(html: &str, page: &url::Url) -> CoreResult<Login> {
    use crate::connectors::html::{FORM, INPUT, attr, attrs};
    let form = FORM
        .captures_iter(html)
        .find(|f| has_password_field(&f[2]))
        .ok_or_else(page_changed)?;
    let form_attrs = attrs(&form[1]);
    if !attr(&form_attrs, "method").is_some_and(|m| m.eq_ignore_ascii_case("post")) {
        return Err(page_changed()); // never put the PIN into a URL
    }
    let action = page
        .join(attr(&form_attrs, "action").map(str::trim).unwrap_or(""))
        .map_err(|_| page_changed())?;
    ensure_fnz(&action)?;
    let mut state = Vec::new();
    let mut user_field = None;
    let mut pin_field = None;
    for input in INPUT.captures_iter(&form[2]) {
        let a = attrs(&input[1]);
        let Some(name) = attr(&a, "name").filter(|n| !n.is_empty()) else {
            continue;
        };
        if attr(&a, "disabled").is_some() {
            continue;
        }
        match attr(&a, "type")
            .unwrap_or("text")
            .to_ascii_lowercase()
            .as_str()
        {
            "password" if pin_field.is_none() => pin_field = Some(name.to_string()),
            "hidden" => state.push((name.to_string(), attr(&a, "value").unwrap_or("").into())),
            "text" if name.ends_with("$txtPbzZugangsId") => user_field = Some(name.to_string()),
            _ => {}
        }
    }
    let decoded = crate::text::decode_html_entities(html);
    let submit = POSTBACK
        .captures_iter(&decoded)
        .map(|c| c[1].to_string())
        .find(|n| n.ends_with("$LOGIN_PBZ"))
        .ok_or_else(page_changed)?;
    Ok(Login {
        action,
        state,
        user_field: user_field.ok_or_else(page_changed)?,
        pin_field: pin_field.ok_or_else(page_changed)?,
        submit,
    })
}

/// The SPA base the snapshot scripts build from the reached URL: the session
/// path the login ended on, one trailing file name dropped, then `eox/rest/`
/// appended. The session prefix is never guessed – it is read from the reached
/// URL, or the check fails instead of fetching under a wrong base.
fn rest_base_of(page: &url::Url) -> CoreResult<url::Url> {
    let mut path = page.path().trim_end_matches('/').to_string();
    // A trailing file name (`redirect.aspx`) or the login route: the SPA paths
    // the snapshot scripts see never end in either.
    while let Some(pos) = path.rfind('/') {
        let last = &path[pos + 1..];
        if last.contains('.') || last == "login" {
            path.truncate(pos);
        } else {
            break;
        }
    }
    if !path.contains("/EOY/") {
        return Err(CoreError::protocol(tr(
            "FNZ: Die Anmeldung wurde nicht abgeschlossen. Bitte den Abruf erneut starten (FNZsecure-Freigabe nötig?).",
        )));
    }
    url::Url::parse(&format!("https://portal.fnz.de{path}/eox/rest/")).map_err(|_| page_changed())
}

/// PDF base: the session prefix before `EOY`, like the mailbox script builds
/// it (`location.pathname.split('EOY')[0]` in the browser window).
fn pdf_base_of(rest_base: &url::Url) -> url::Url {
    let path = rest_base.path();
    let prefix = path
        .split_once("/EOY/")
        .map_or(path.to_string(), |(p, _)| format!("{p}/"));
    url::Url::parse(&format!("https://portal.fnz.de{prefix}p/postkorb.aspx"))
        .unwrap_or_else(|_| rest_base.clone())
}

/// The portal's `hasId`: a nonempty string or an object where some value is one.
fn has_id(value: &Value) -> bool {
    match value {
        Value::String(s) => !s.trim().is_empty(),
        Value::Object(o) => o.values().any(has_id),
        _ => false,
    }
}

/// Safe path encoding for ids the portal returns (no `urlencoding` dep here).
fn path_encoding(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The FNZsecure challenge the UI polls until the approval was given.
fn approval_challenge() -> Challenge {
    Challenge {
        kind: ChallengeKind::DecoupledPush,
        title: "FNZsecure".into(),
        message: tr("Bitte die Anmeldung in der FNZsecure-App bestätigen.").into(),
        method_name: "FNZsecure".into(),
        redirect_url: None,
        image: None,
        expires_in_seconds: None,
        poll_interval_seconds: APPROVAL_POLL_SECONDS,
    }
}

fn incomplete_list() -> CoreError {
    invalid("FNZ: Unvollständige Umsatzliste. Bitte den Abruf erneut starten.")
}

fn incomplete_documents() -> CoreError {
    invalid("FNZ: Unvollständige Dokumentenliste. Bitte den Abruf erneut starten.")
}

/// Headless FNZ connector: the stored Zugangsnummer + PIN take the place of
/// the browser window. After the FNZsecure approval the same validated REST
/// calls run that the portal snapshot scripts make in the web view, and the
/// result goes through the same snapshot parser as the browser import.
#[derive(Default)]
pub struct PortalConnector {
    /// Session of one sync: accepted in `begin`, used in `fetch`, gone with
    /// the connector instance. `None` = no live session.
    client: Option<reqwest::Client>,
    rest_base: Option<url::Url>,
    pdf_base: Option<url::Url>,
}

impl PortalConnector {
    fn client(&self) -> CoreResult<&reqwest::Client> {
        self.client.as_ref().ok_or_else(session_ended)
    }

    fn rest_base(&self) -> CoreResult<&url::Url> {
        self.rest_base.as_ref().ok_or_else(session_ended)
    }

    /// The headers every authenticated portal request carries (the snapshot
    /// script sends the same set; `X-Context-ID` is fresh per request).
    fn request(
        &self,
        method: reqwest::Method,
        url: url::Url,
        body: Option<&Value>,
    ) -> CoreResult<reqwest::RequestBuilder> {
        use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderValue};
        let mut builder = self
            .client()?
            .request(method, url)
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, "application/json")
            .header("X-EOX-Session", HeaderValue::from_static(""))
            .header("X-EOX-CLIENT-VERSION", CLIENT_VERSION)
            .header("X-Context-ID", uuid::Uuid::new_v4().to_string());
        if let Some(body) = body {
            builder = builder.body(serde_json::to_string(body).map_err(|_| page_changed())?);
        }
        Ok(builder)
    }

    /// Error mapping of one REST answer, the way the snapshot scripts read it.
    async fn rest_answer(&self, response: reqwest::Response, what: &str) -> CoreResult<Value> {
        let status = response.status();
        if is_login_path(response.url())
            || response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|ct| ct.starts_with("text/html"))
        {
            // The portal answered with a page instead of data: the session is
            // gone. Rest answers never run before `check` said Ready, so this
            // is not a still-missing approval.
            return Err(session_ended());
        }
        if !status.is_success() {
            if matches!(status.as_u16(), 401 | 403) {
                return Err(session_ended());
            }
            return Err(CoreError::Bank {
                code: status.as_u16().to_string(),
                message: trf(
                    "FNZ: {0} wurde abgelehnt (HTTP {1}).",
                    &[&what, &status.as_str()],
                ),
            });
        }
        let text = crate::connectors::read_text_limited(response).await?;
        let v: Value = serde_json::from_str(&text).map_err(|_| page_changed())?;
        if v["resourceIsUnavailable"].as_bool() == Some(true) {
            return Err(unavailable());
        }
        if !v.is_object() {
            return Err(page_changed());
        }
        Ok(v)
    }

    async fn get(&self, path: &str, what: &str) -> CoreResult<Value> {
        let url = self.rest_base()?.join(path).map_err(|_| page_changed())?;
        let request = self.request(reqwest::Method::GET, url, None)?;
        self.rest_answer(request.send().await?, what).await
    }

    async fn post(&self, path: &str, body: &Value, what: &str) -> CoreResult<Value> {
        let url = self.rest_base()?.join(path).map_err(|_| page_changed())?;
        let request = self.request(reqwest::Method::POST, url, Some(body))?;
        self.rest_answer(request.send().await?, what).await
    }

    /// Is the session approved? Only the read-only overview call decides;
    /// nothing is guessed from page texts.
    async fn check(&self) -> Check {
        let url = match self.rest_base() {
            Ok(base) => match base.join("depot/getDepotOverviewDataForEndCustomer") {
                Ok(u) => u,
                Err(_) => return Check::Failed(page_changed()),
            },
            Err(e) => return Check::Failed(e),
        };
        let request = match self.request(reqwest::Method::GET, url, None) {
            Ok(r) => r,
            Err(e) => return Check::Failed(e),
        };
        let Ok(response) = request.send().await else {
            return Check::Failed(network(
                "FNZ: Das Portal ist nicht erreichbar. Bitte später erneut versuchen.",
            ));
        };
        let status = response.status();
        // While the approval is missing, the portal refuses the authenticated
        // call (401/403) or serves a page instead of the JSON API.
        if matches!(status.as_u16(), 401 | 403)
            || is_login_path(response.url())
            || response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|ct| ct.starts_with("text/html"))
        {
            return Check::Verified(Verified::AwaitingApproval);
        }
        if !status.is_success() {
            return Check::Failed(rejected(status));
        }
        let body = match crate::connectors::read_text_limited(response).await {
            Ok(b) => b,
            Err(e) => return Check::Failed(e),
        };
        match serde_json::from_str::<Value>(&body) {
            Ok(v) if v.is_object() && v["ownerGroups"].is_array() => {
                Check::Verified(Verified::Ready)
            }
            _ => Check::Failed(page_changed()),
        }
    }

    /// Every row of one paged portal list, with the snapshot script's
    /// validations: consistent totals, no repeated row, no silent stops.
    async fn page_all(
        &self,
        ctx: &ConnectContext,
        path: &str,
        filter: &Value,
        array_key: &str,
        total_key: &str,
        id_key: &str,
    ) -> CoreResult<ListPage> {
        let mut rows: Vec<Value> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut total: Option<i64> = None;
        for page in 0..MAX_PAGES {
            // cancelled during the fetch: no further request to the portal
            if ctx.cancelled.is_set() {
                return Err(CoreError::SessionClosed);
            }
            let mut body = filter.clone();
            body["pagination"] = json!({ "page": page.to_string(), "pageSize": "100" });
            let data = self.post(path, &body, "Die Umsatzliste").await?;
            let list = data[array_key].as_array().ok_or_else(incomplete_list)?;
            let count = data[total_key].as_i64().ok_or_else(incomplete_list)?;
            if !(0..=MAX_ROWS as i64).contains(&count) || total.is_some_and(|t| t != count) {
                return Err(incomplete_list());
            }
            total = Some(count);
            for row in list {
                let id = row
                    .get(id_key)
                    .filter(|id| has_id(id))
                    .ok_or_else(|| invalid("FNZ: Umsatz ohne Kennung."))?;
                if !seen.insert(id.to_string()) {
                    return Err(invalid(
                        "FNZ: Wiederholte Umsatzseite. Bitte den Abruf erneut starten.",
                    ));
                }
                rows.push(row.clone());
            }
            if rows.len() as i64 == count {
                return Ok(ListPage {
                    rows,
                    metadata: data["metadata"].clone(),
                });
            }
            if list.is_empty() || rows.len() as i64 > count {
                return Err(incomplete_list());
            }
        }
        Err(incomplete_list())
    }

    /// Mailbox plus archive: metadata of every document, with the snapshot
    /// script's paging and validations.
    async fn document_list(&mut self, ctx: &ConnectContext) -> CoreResult<Vec<Value>> {
        let date_to = crate::dates::now_utc()
            .strftime("%Y-%m-%dT%H:%M:%S")
            .to_string();
        let mut all: Vec<Value> = Vec::new();
        let mut ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        for archived in [false, true] {
            let mut fetched = 0usize;
            let mut index = 0usize;
            loop {
                // cancelled during the fetch: no further request to the portal
                if ctx.cancelled.is_set() {
                    return Err(CoreError::SessionClosed);
                }
                let body = json!({
                    "postboxCategoryAndStateFilter": {
                        "dateFrom": "2000-01-01T00:00:00", "dateTo": date_to,
                        "category": Value::Null, "onlyUnread": false,
                        "onlyFavoured": false, "archived": archived,
                    },
                    "postboxDepotFilter": Value::Null,
                    "indexFrom": index.to_string(),
                    "indexTo": (index + DOCUMENTS_PAGE).to_string(),
                });
                let data = self
                    .post(
                        "postbox/getCustomerPostboxForCustomer",
                        &body,
                        "Die Dokumentenliste",
                    )
                    .await?;
                let documents = data["documents"]
                    .as_array()
                    .ok_or_else(incomplete_documents)?;
                for d in documents {
                    if d["id"].as_str().is_none_or(|s| s.trim().is_empty()) {
                        return Err(invalid("FNZ: Dokument ohne Kennung."));
                    }
                }
                if documents
                    .iter()
                    .any(|d| !ids.insert(d["id"].as_str().unwrap_or_default().to_string()))
                {
                    return Err(invalid(
                        "FNZ: Wiederholte Dokumentenseite. Bitte den Abruf erneut starten.",
                    ));
                }
                all.extend(documents.iter().cloned());
                fetched += documents.len();
                index += DOCUMENTS_PAGE;
                // Complete only when the reported total is reached. A short
                // page against a remaining total keeps going (the portal
                // delivers pages smaller than asked for); without a usable
                // total, a short page is the end, like the browser script.
                let total = data["totalMessages"].as_i64().filter(|t| *t >= 0);
                if let Some(total) = total {
                    if fetched as i64 >= total {
                        break;
                    }
                    if documents.is_empty() {
                        return Err(incomplete_documents());
                    }
                } else if documents.len() < DOCUMENTS_PAGE {
                    break;
                }
                if index > MAX_ROWS {
                    return Err(incomplete_documents());
                }
            }
        }
        Ok(all)
    }

    /// Pending PDFs of the mailbox, ready for `store_fetch` (which skips
    /// archived/blocked accounts and stores each document once). Returns the
    /// documents, the known-ids state (only what is stored here) and whether
    /// the mailbox was not fully read (failed download, budget, cancellation).
    async fn pending_documents(
        &mut self,
        connection_id: i64,
        ctx: &ConnectContext,
        refs: &[String],
    ) -> CoreResult<(Vec<RemoteDocument>, KnownDocuments, bool)> {
        let mut known = KnownDocuments::from_state(&ctx.state);
        // Documents retrieval switched off: list nothing, download nothing and
        // remember nothing, so a later sync can fetch what is missing.
        if !ctx.documents.enabled() {
            return Ok((Vec::new(), known, false));
        }
        // One account's sync (#1124): only its PDFs are downloaded (and
        // remembered) – keep_only would discard the others without storing
        // them, and their ids must not block later syncs.
        let only = ctx.only_account.as_ref();
        let list = self.document_list(ctx).await?;
        let mut documents = Vec::new();
        let mut remaining_budget = SYNC_DOC_BYTES;
        let mut incomplete = false;
        let mut consecutive_failures = 0u32;
        for doc in &list {
            // cancelled during the fetch: what is here stays, the rest comes
            // with the next sync.
            if ctx.cancelled.is_set() {
                incomplete = true;
                break;
            }
            let Some(id) = doc["id"].as_str().filter(|s| !s.trim().is_empty()) else {
                continue;
            };
            if known.contains(id) {
                continue;
            }
            let Ok(account_ref) = document_account(doc) else {
                continue; // an unknown account gets no download, like the browser flow
            };
            if only.is_some_and(|o| *o != account_ref)
                || ctx.skipped_accounts.contains(account_ref)
                || !refs.contains(&account_ref.to_string())
            {
                continue;
            }
            if remaining_budget == 0 {
                // Byte budget of this sync spent: the rest stays due (the id
                // is not remembered) and the next sync catches up.
                incomplete = true;
                break;
            }
            match self.fetch_pdf(id).await {
                Ok(bytes) => {
                    remaining_budget = remaining_budget.saturating_sub(bytes.len());
                    let title = document_title(doc);
                    documents.push(RemoteDocument {
                        account_ref: account_ref.to_string(),
                        booking_external_id: None,
                        doc_type: crate::documents::account_document_type(&title).into(),
                        mime_type: "application/pdf".into(),
                        file_name: format!("FNZ_{}.pdf", file_name_id(id)),
                        bytes,
                        source_url: document_source(connection_id, id),
                        title,
                        date: document_date(doc),
                    });
                    known.insert(id.to_string());
                    consecutive_failures = 0;
                }
                // The session is gone: further downloads all fail the same
                // way, so the loop ends here.
                Err(CoreError::Authentication { .. }) => {
                    incomplete = true;
                    break;
                }
                // One document failed (rejected, too large, no PDF): the
                // others still come, the failed id stays eligible for retry.
                Err(_) => {
                    incomplete = true;
                    consecutive_failures += 1;
                    if consecutive_failures >= MAX_DOC_FAILURES {
                        // A broken mailbox run must not become hundreds of
                        // failed requests; the next sync tries anew.
                        break;
                    }
                }
            }
        }
        Ok((documents, known, incomplete))
    }

    /// One mailbox PDF, validated like the browser download: content type
    /// `application/pdf`, `%PDF-` marker, bounded size. Runs inside the login
    /// session (cookies, `X-EOX-Session` et al.). A lost session is an
    /// `Authentication` error, a broken document its own error.
    async fn fetch_pdf(&self, id: &str) -> CoreResult<Vec<u8>> {
        let pdf_base = self.pdf_base.as_ref().ok_or_else(session_ended)?;
        let mut url = pdf_base.clone();
        url.query_pairs_mut()
            .append_pair("docId", id)
            .append_pair("download", "true");
        let response = self
            .client()?
            .get(url)
            .send()
            .await
            .map_err(|_| session_ended())?;
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|v| {
                v.split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase()
            });
        let status = response.status();
        if content_type.as_deref() == Some("text/html") || matches!(status.as_u16(), 401 | 403) {
            // A page or a refusal instead of the document: the session is
            // gone.
            return Err(session_ended());
        }
        if !status.is_success() {
            return Err(CoreError::Bank {
                code: status.as_u16().to_string(),
                message: trf(
                    "FNZ: {0} wurde abgelehnt (HTTP {1}).",
                    &[&"Das Dokument", &status.as_str()],
                ),
            });
        }
        let bytes =
            crate::connectors::read_body_limited(response, crate::connectors::MAX_DOC_BYTES)
                .await?;
        if !bytes.starts_with(b"%PDF-") {
            return Err(invalid("FNZ: Das Dokument ist keine gültige PDF-Datei."));
        }
        Ok(bytes)
    }
    /// Detail row of one cash account: the validated transaction list.
    async fn cash_detail(
        &self,
        ctx: &ConnectContext,
        base_filter: &Value,
        number: &str,
    ) -> CoreResult<Value> {
        let mut body = base_filter.clone();
        body["cashAccountId"] = json!(number);
        body["referenceFilter"] = json!("");
        let cash = self
            .page_all(
                ctx,
                "cash-account-transaction/getCashAccountTransactionOverviewForEndCustomer",
                &body,
                "cashAccountTransactions",
                "totalCashAccountTransactions",
                "orderOrTransactionId",
            )
            .await?;
        Ok(json!({
            "accountRef": number, "kind": "cash",
            "cashAccountId": cash.metadata["cashAccountId"],
            "iban": cash.metadata["iban"],
            "transactions": cash.rows,
        }))
    }

    /// Detail row of one investment depot: inventory, order transactions and
    /// the migrated AAB history. Only investment requests need the depot-level
    /// id; a totals-only stock depot works without one.
    async fn investment_detail(
        &self,
        ctx: &ConnectContext,
        row: &Value,
        base_filter: &Value,
        number: &str,
    ) -> CoreResult<Value> {
        let account = &row["depotOrCashAccount"];
        let Some(depot_id) = account
            .pointer("/depotOrCashAccountId/depotId")
            .filter(|v| v["id"].as_str().is_some_and(|s| !s.trim().is_empty()))
            .cloned()
        else {
            return Err(invalid("FNZ: Konto ohne eindeutige Kennung."));
        };
        let inventory = self
            .get(
                &format!(
                    "depot-inventory/getEndCustomerDepotInventory/depotId/{}",
                    path_encoding(depot_id["id"].as_str().unwrap_or_default())
                ),
                "Die Depotbestände",
            )
            .await?;
        if inventory["resourceIsUnavailable"].as_bool() == Some(true)
            || money_total(&inventory["totalInventoryValueAccountCurrency"])
                != money_total(&row["depotInventorySumAccountCurrency"])
        {
            // The depot's data contradicts the overview (mid-sync valuation or
            // a portal change): this account stays back for the next sync
            // instead of making the whole fetch fail.
            return Err(invalid(
                "FNZ: Der Depotbestand stimmt nicht mit der Kontoübersicht überein.",
            ));
        }
        let mut orders_body = base_filter.clone();
        orders_body["depotId"] = depot_id.clone();
        orders_body["showTransactionsOnDepotLevel"] = json!(true);
        orders_body["subDepotPos"] = Value::Null;
        orders_body["showTaxTransactions"] = json!(true);
        orders_body["showOtherTransactions"] = json!(true);
        orders_body["includeCanceledTransactions"] = json!(true);
        let orders = self
            .page_all(
                ctx,
                "order-transaction/getOrderTransactionOverviewForEndCustomer",
                &orders_body,
                "orderTransactions",
                "totalOrderTransactions",
                "orderTransactionId",
            )
            .await?;
        let historical = self
            .page_all(
                ctx,
                "order-transaction/getMigratedAabTransactionOverview",
                &json!({ "depotId": depot_id }),
                "migratedAabTransactions",
                "totalTransactions",
                "migratedAabTransactionId",
            )
            .await?;
        Ok(json!({
            "accountRef": number, "kind": "investment", "inventory": inventory,
            "transactions": orders.rows,
            "historicalTransactions": historical.rows,
        }))
    }
}

/// A portal money object as `(cents, currency)`; `None` when unusable (an
/// estimated value counts as unusable, like the snapshot parser rejects it).
/// Both sides of the comparison parse the same way, so a mismatch here is a
/// portal-side contradiction, not a parsing artifact.
fn money_total(money: &Value) -> Option<(i64, String)> {
    if money["estimatedValueFlag"].as_bool() == Some(true) {
        return None;
    }
    let currency = money["currencyId"]["id"].as_str()?;
    let cents = crate::money::json_number_to_cents(&money["value"]).ok()?;
    Some((cents, currency.to_string()))
}

/// Safe, stable file name stem for a mailbox id (`FNZ_<id>.pdf`).
fn file_name_id(id: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(id)
}

#[async_trait]
impl BankConnector for PortalConnector {
    async fn begin(&mut self, ctx: &ConnectContext) -> CoreResult<Phase> {
        let user = ctx.login_name.trim().to_string();
        let pin_empty = ctx.secret.expose().trim().is_empty();
        if user.is_empty() || pin_empty {
            // No stored credentials: like before this change, the in-app
            // browser window takes over.
            return Err(CoreError::BrowserLoginRequired {
                message: tr("Bitte bei FNZ anmelden und in FNZsecure freigeben.").into(),
            });
        }
        // Credentials and cookies go to portal.fnz.de only; a redirect to
        // another host ends the chain instead of replaying the POST.
        let client = crate::connectors::client_builder(std::time::Duration::from_secs(60))
            .user_agent(crate::connectors::BROWSER_UA)
            .cookie_store(true)
            .redirect(crate::connectors::same_host_redirects())
            .build()?;
        let resp = client.get(ENTRY).send().await.map_err(|_| {
            network("FNZ: Das Portal ist nicht erreichbar. Bitte später erneut versuchen.")
        })?;
        let page = resp.url().clone();
        ensure_fnz(&page)?;
        let html = crate::connectors::read_text_limited(resp).await?;
        let form = login_form(&html, &page)?;
        let fields = WipeFields(form.filled(&user, ctx.secret.expose()));
        let sent = client
            .post(form.action.clone())
            .form(&fields.0)
            .send()
            .await;
        drop(fields);
        let resp = sent.map_err(|_| {
            network("FNZ: Das Portal ist nicht erreichbar. Bitte später erneut versuchen.")
        })?;
        let reached = resp.url().clone();
        ensure_fnz(&reached)?;
        let status = resp.status();
        let html = crate::connectors::read_text_limited(resp).await?;
        if is_login_path(&reached) || is_login_html(&html) {
            return Err(wrong_credentials());
        }
        if !status.is_success() {
            return Err(rejected(status));
        }
        let rest_base = rest_base_of(&reached)?;
        self.pdf_base = Some(pdf_base_of(&rest_base));
        self.rest_base = Some(rest_base);
        self.client = Some(client);
        match self.check().await {
            Check::Verified(Verified::Ready) => Ok(Phase::Ready),
            Check::Verified(Verified::AwaitingApproval) => Ok(Phase::Pending(approval_challenge())),
            Check::Failed(e) => Err(e),
        }
    }

    async fn respond(
        &mut self,
        _ctx: &ConnectContext,
        response: ChallengeResponse,
    ) -> CoreResult<Phase> {
        // A typed code cannot finish an FNZsecure approval: only the app can.
        if matches!(response, ChallengeResponse::Code(_)) {
            return Err(approval_needs_app());
        }
        match self.check().await {
            Check::Verified(Verified::Ready) => Ok(Phase::Ready),
            Check::Verified(Verified::AwaitingApproval) => Ok(Phase::Pending(approval_challenge())),
            Check::Failed(e) => Err(e),
        }
    }

    async fn fetch(&mut self, ctx: &ConnectContext) -> CoreResult<FetchResult> {
        let overview = self
            .get(
                "depot/getDepotOverviewDataForEndCustomer",
                "Die Kontoübersicht",
            )
            .await?;
        if !overview["ownerGroups"].is_array() {
            return Err(invalid(
                "FNZ: Keine Kontoübersicht erkannt. Bitte den Abruf erneut starten.",
            ));
        }
        // Historical range and validations of the snapshot scripts: all
        // available data, complete pages, consistent account identities.
        let base_filter = json!({
            "dateFrom": "2000-01-01",
            "dateTo": today(),
            "showIngoingTransactions": true,
            "showOutgoingTransactions": true,
            "sortOrder": "BOOKING_DATE_DESC",
        });
        let mut details = Vec::new();
        let mut totals_only = 0u32;
        let mut refs = Vec::new();
        let mut warnings = Vec::new();
        for (_group, key, row) in open_rows(&overview)? {
            let account = &row["depotOrCashAccount"];
            let number = field(account, "/externalDepotNumber/externalDepotNumber")?;
            if refs.iter().any(|r| r == number) {
                continue;
            }
            refs.push(number.to_string());
            // Per-account error isolation: one account's failing request does
            // not end the fetch of the others – its row stays without data
            // ("notFetched"), the next sync fetches it again. A cancelled or
            // lost session still ends the whole fetch.
            let detail = if key == "cashAccounts" {
                self.cash_detail(ctx, &base_filter, number).await
            } else if key == "investmentDepots" {
                self.investment_detail(ctx, row, &base_filter, number).await
            } else {
                // Depot kinds whose detail endpoints are not yet live
                // validated: totals and PDFs only, like the browser flow.
                totals_only += 1;
                Ok(json!({ "accountRef": number, "kind": "totalsOnly" }))
            };
            match detail {
                Ok(row) => details.push(row),
                // cancelled mid-request, or the session died after this
                // account: the remaining accounts would fail the same way.
                Err(e)
                    if matches!(e, CoreError::SessionClosed)
                        || matches!(e, CoreError::Authentication { .. }) =>
                {
                    return Err(e);
                }
                Err(_) => {
                    warnings.push(trf(
                        "FNZ: Konto {0}: Angaben konnten nicht geladen werden – der nächste Abruf wiederholt es.",
                        &[&number],
                    ));
                    details.push(json!({ "accountRef": number, "kind": "notFetched" }));
                }
            }
        }
        let snapshot = json!({
            "version": 1, "overview": overview, "details": details, "totalsOnly": totals_only,
        });
        let mut result = super::snapshot::snapshot(&snapshot.to_string())?;
        result.warnings = warnings;
        // The mailbox may fail without taking the account data with it: the
        // sync reports the document catch-up as due instead.
        match self.pending_documents(ctx.connection_id, ctx, &refs).await {
            Ok((documents, known, incomplete)) => {
                result.documents = documents;
                result.state = known.to_state();
                result.documents_incomplete = incomplete;
            }
            Err(_) => result.documents_incomplete = true,
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
