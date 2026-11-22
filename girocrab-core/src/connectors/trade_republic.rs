//! Trade Republic – web login + WebSocket API.
//!
//! Trade Republic publishes no official API; this connector follows the
//! publicly observable behaviour of the web app (as also used by the
//! open-source `pytr` project):
//!
//!  POST https://api.traderepublic.com/api/v1/auth/web/login       (fallback: /api/v2/…)
//!       {"phoneNumber": "+49…", "pin": "1234"}   -> {"processId", "countdownInSeconds"}
//!  POST …/auth/web/login/{processId}/sms                          (optional: code per SMS)
//!  POST …/auth/web/login/{processId}/{code}  -> HTTP 200, often EMPTY body;
//!       the session arrives as Set-Cookie (tr_session, tr_refresh)
//!  GET  …/api/v1/auth/web/session            -> refreshes a stored session
//!  WSS  wss://api.traderepublic.com  (Cookie + Origin headers)
//!       "connect 31 {…}" -> "connected"
//!       "sub 1 {\"type\":\"cash\"}"                         (fallback "availableCash")
//!       "sub 2 {\"type\":\"timelineTransactions\",\"after\":…}"
//!       frames: "<id> A <json>" (answer), "<id> E <json>" (error), "<id> C" (complete)
//!
//! The session cookies are stored (encrypted) in the connection state so the
//! next sync needs no new 2FA code. Cookies from the in-app browser login
//! (WKWebView) can be handed over with `Vault::set_connection_session`.
//! It may break whenever Trade Republic changes its web app.

use std::sync::Arc;

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use reqwest::cookie::{CookieStore, Jar};
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue, ORIGIN, REFERER, SET_COOKIE, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use super::{
    BROWSER_UA, BankConnector, Challenge, ChallengeKind, ChallengeResponse, ConnectContext,
    FetchResult, Phase, RemoteAccount, RemoteHolding, RemoteTransaction,
};
use crate::asset_classification::{AssetClass, ClassificationValues, InstrumentKind};
use crate::error::{CoreError, CoreResult};
use crate::i18n::{tr, trf};
use crate::models::{AccountKind, Holding};
use crate::money::{json_number_to_cents_rounded, parse_cents, parse_cents_rounded};
use crate::secret::{SecretJson, SecretString};

const API: &str = "https://api.traderepublic.com";
const WS: &str = "wss://api.traderepublic.com";
const APP_ORIGIN: &str = "https://app.traderepublic.com";

/// Strict E.164 for German numbers: "0171 1234567", "0049171…", "49171…",
/// "17012345678" -> "+491711234567" / "+4917012345678".
pub fn normalize_phone_number(input: String) -> String {
    normalize_phone(&input)
}

pub fn normalize_phone(input: &str) -> String {
    let t = input.trim();
    let plus = t.starts_with('+');
    let digits: String = t.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return String::new();
    }
    if plus {
        return format!("+{digits}");
    }
    if let Some(rest) = digits.strip_prefix("00") {
        return format!("+{rest}");
    }
    if let Some(rest) = digits.strip_prefix('0') {
        return format!("+49{rest}");
    }
    if digits.starts_with("49") && digits.len() >= 12 {
        return format!("+{digits}");
    }
    // German mobile numbers without the trunk 0 ("1512…", "171…")
    format!("+49{digits}")
}

/// Value of one cookie in a "a=1; b=2" header string.
fn cookie_value(cookies: &str, name: &str) -> Option<String> {
    cookies
        .split(';')
        .filter_map(|p| p.trim().split_once('='))
        .find(|(k, _)| k.trim() == name)
        .map(|(_, v)| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// "a=1; b=2" + Set-Cookie values -> merged "name=value; …" (later wins).
fn merge_cookies(existing: &str, set_cookie_values: &[String]) -> String {
    let mut pairs: Vec<(String, String)> = existing
        .split(';')
        .filter_map(|p| {
            p.trim()
                .split_once('=')
                .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        })
        .filter(|(k, _)| !k.is_empty())
        .collect();
    for sc in set_cookie_values {
        let first = sc.split(';').next().unwrap_or("");
        if let Some((k, v)) = first.split_once('=') {
            let (k, v) = (k.trim().to_string(), v.trim().to_string());
            pairs.retain(|(pk, _)| *pk != k);
            if !v.is_empty() {
                pairs.push((k, v));
            }
        }
    }
    pairs
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ")
}

fn snippet(body: &str) -> String {
    let b = body.trim();
    if b.is_empty() {
        "(leere Antwort)".into()
    } else if b.chars().count() > 300 {
        format!("{}…", b.chars().take(300).collect::<String>())
    } else {
        b.to_string()
    }
}

/// Amounts arrive as JSON numbers that may carry float noise (-4.040000000000001).
fn amount_cents(v: &Value) -> i64 {
    json_number_to_cents_rounded(v).unwrap_or(0)
}

/// A number in a detail text such as "1.234,56 €", "0,5" or "12.5": German
/// notation first. A lone `.` followed by exactly three digits after at most
/// three is a thousands separator ("1.234"), otherwise a decimal point.
fn parse_decimal_text(text: &str) -> Option<f64> {
    let t: String = text
        .chars()
        .filter(|c| c.is_ascii_digit() || matches!(c, ',' | '.' | '-' | '−'))
        .map(|c| if c == '−' { '-' } else { c })
        .collect();
    if t.is_empty() || !t.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    let norm = match (t.rfind(','), t.rfind('.')) {
        (Some(c), Some(d)) if c > d => t.replace('.', "").replace(',', "."),
        (Some(_), Some(_)) => t.replace(',', ""),
        (Some(_), None) => t.replace(',', "."),
        (None, Some(d)) => {
            let (head, tail) = t.split_at(d);
            let thousands = tail.len() == 4
                && head.trim_start_matches('-').len() <= 3
                && !head.trim_start_matches('-').contains('.');
            if thousands { t.replace('.', "") } else { t }
        }
        (None, None) => t,
    };
    norm.parse().ok()
}

/// What a trade's detail screen (`timelineDetailV2`) tells besides the
/// timeline row: ISIN, units, price per unit, fees and taxes.
#[derive(Debug, Default, PartialEq)]
struct TradeDetail {
    isin: Option<String>,
    quantity: Option<f64>,
    price_cents: Option<i64>,
    fees_cents: Option<i64>,
    taxes_cents: Option<i64>,
}

/// `logos/US0378331005/v2` or a bare ISIN → the ISIN.
fn isin_of(icon: &str) -> Option<String> {
    let rest = icon.find('/').map_or(icon, |i| &icon[i + 1..]);
    let isin = rest.split('/').next().unwrap_or_default();
    is_isin(isin).then(|| isin.to_string())
}

/// Shaped like an ISIN: two letters, then ten letters or digits.
fn is_isin(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 12
        && b[..2].iter().all(u8::is_ascii_uppercase)
        && b[2..]
            .iter()
            .all(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
}

fn walk_rows(v: &Value, f: &mut dyn FnMut(&str, &str)) {
    match v {
        Value::Object(m) => {
            if let Some(title) = m.get("title").and_then(Value::as_str) {
                let text = m
                    .get("detail")
                    .and_then(|d| d["text"].as_str().or_else(|| d.as_str()))
                    .unwrap_or_default();
                f(title, text);
            }
            m.values().for_each(|x| walk_rows(x, f));
        }
        Value::Array(a) => a.iter().for_each(|x| walk_rows(x, f)),
        _ => {}
    }
}

fn parse_trade_detail(item_icon: &str, detail: &Value) -> TradeDetail {
    let mut d = TradeDetail {
        isin: isin_of(item_icon),
        ..Default::default()
    };
    if d.isin.is_none() {
        // header section carries the instrument icon, an action its ISIN
        if let Some(sections) = detail["sections"].as_array() {
            d.isin = sections.iter().find_map(|s| {
                let icon = s["data"]["icon"]
                    .as_str()
                    .or_else(|| s["data"]["icon"]["asset"].as_str());
                icon.and_then(isin_of).or_else(|| {
                    (s["action"]["type"] == "instrumentDetail")
                        .then(|| s["action"]["payload"].as_str().and_then(isin_of))
                        .flatten()
                })
            });
        }
    }
    walk_rows(detail, &mut |title, text| {
        match title {
            "Aktien" | "Anteile" | "Shares" | "Aktien hinzugefügt" | "Aktien entfernt"
                if d.quantity.is_none() =>
            {
                d.quantity = parse_decimal_text(text).map(f64::abs);
            }
            "Preis" | "Kurs" | "Aktienkurs" | "Quotation" | "Price" if d.price_cents.is_none() => {
                d.price_cents = parse_cents_rounded(text).ok().map(i64::abs);
            }
            "Gebühr" | "Gebühren" | "Fee" if d.fees_cents.is_none() => {
                d.fees_cents = parse_cents(text).ok().map(i64::abs);
            }
            "Steuer" | "Steuern" | "Tax" if d.taxes_cents.is_none() => {
                d.taxes_cents = parse_cents(text).ok().map(i64::abs);
            }
            _ => {}
        };
    });
    d
}

/// A timeline row of a trade, dividend or savings plan that has a detail screen.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct TradeRow {
    id: String,
    date: String,
    title: String,
    kind_text: String,
    #[serde(default)]
    event_context: String,
    amount_cents: i64,
    currency: String,
    icon: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct DocumentCandidate {
    id: String,
    date: String,
    title: String,
    event_context: String,
}

impl DocumentCandidate {
    fn from_timeline_item(item: &Value, id: String, date: String) -> Self {
        let subtitle = item["subtitle"].as_str().unwrap_or_default();
        let event_type = item["eventType"].as_str().unwrap_or_default();
        Self {
            id,
            date,
            title: item["title"].as_str().unwrap_or_default().into(),
            event_context: format!("{subtitle} {event_type}").to_lowercase(),
        }
    }

    fn classify(&self, title: &str) -> (&'static str, &'static str) {
        let lower = title.to_lowercase();
        let doc_type = refine_document_type(
            crate::documents::type_from_title(&lower),
            &self.event_context,
        );
        let account_hint =
            if doc_type != "steuer" && matches!(lower.as_str(), "dokumente" | "dokument") {
                &self.event_context
            } else {
                &lower
            };
        let account_ref = document_account_ref(account_hint, doc_type).unwrap_or("tr:cash");
        (doc_type, account_ref)
    }
}

/// The queue after a document pass: what the pass did not reach first, then what failed. A
/// candidate that keeps failing must not hold the places of every pass while others never get
/// a try (review of #1660).
fn pending_after_pass(
    not_reached: Vec<DocumentCandidate>,
    failed: Vec<DocumentCandidate>,
) -> Vec<DocumentCandidate> {
    let mut pending = not_reached;
    pending.extend(failed);
    pending
}

/// Where the timeline pass starts for this sync: `(history_done, resume cursor)`. A sync of one
/// account (#1124) is cut down to it after the fetch, so it reads no history and resumes nothing:
/// a history batch it read would lose the other account's rows for good (review of #1660).
fn history_scope(session: &StoredSession, single_account: bool) -> (bool, Option<String>) {
    if single_account {
        return (true, None);
    }
    (
        session.trade_history_complete && session.trade_history_retry.is_empty(),
        session.transaction_history_cursor.clone(),
    )
}

/// Whether the Activity Log listing may stop at an item of `date`: below the sync window, and
/// only once the whole log was listed (a history finished while document retrieval was off has
/// never listed the older statements, review of #1660) and no earlier listing is still open.
fn activity_log_stops_at(date: &str, window_start: &str, session: &StoredSession) -> bool {
    !date.is_empty()
        && date < window_start
        && session.doc_listing_complete
        && !session.doc_listing_open
}

/// New candidates take precedence over queued copies of the same timeline item.
/// A disabled document fetch keeps the queue for the first enabled fetch.
fn documents_for_fetch(
    mut candidates: Vec<DocumentCandidate>,
    pending: &mut Vec<DocumentCandidate>,
    enabled: bool,
) -> Vec<DocumentCandidate> {
    let mut seen: std::collections::HashSet<_> = candidates
        .iter()
        .map(|candidate| candidate.id.clone())
        .collect();
    candidates.extend(
        std::mem::take(pending)
            .into_iter()
            .filter(|candidate| seen.insert(candidate.id.clone())),
    );
    if enabled {
        candidates
    } else {
        *pending = candidates;
        Vec::new()
    }
}

struct TimelinePass {
    recent_only: bool,
    history_done: bool,
    in_history: bool,
}

impl TimelinePass {
    fn stop_at_window(&mut self, below_window: bool) -> bool {
        if below_window && (self.recent_only || self.history_done) {
            return true;
        }
        self.in_history |= below_window;
        false
    }

    /// A failure after the recent window leaves the current cursor in place.
    fn page<T>(&self, result: CoreResult<T>) -> CoreResult<Option<T>> {
        match result {
            Ok(page) => Ok(Some(page)),
            Err(_) if self.in_history => Ok(None),
            Err(error) => Err(error),
        }
    }
}

/// How Trade Republic documents are matched to bookings by content (`document_matching`): they
/// use bookings on the cash account. The security is the part after the dash of the stored title
/// (`"<Dokument> – <Eintrag>"`), statements and tax papers are left out.
pub(crate) const DOCUMENT_MATCH: crate::connectors::document_matching::DocumentMatchRules =
    crate::connectors::document_matching::DocumentMatchRules {
        booking_account: |external_ref, _| external_ref == "tr:cash",
        subjects: |title, _| {
            title
                .split_once(" \u{2013} ")
                .map(|(_, subject)| vec![subject.to_owned()])
                .unwrap_or_default()
        },
        subjects_from_text: false,
        link_types: &["abrechnung", "beleg"],
        direction_required: false,
        account_from_text: false,
        amount_labels: &[],
        slack_days: 5,
        inflow: document_inflow,
    };

/// Infers direction from the document-type title and the start of the PDF text. The
/// security name after the title separator does not influence direction.
fn document_inflow(title: &str, text: &str) -> Option<bool> {
    let (event, security) = title.split_once(" \u{2013} ").unwrap_or((title, ""));
    crate::connectors::document_matching::inflow_in_without(event, text, &[security], 400)
}

/// A stored title has no separate timeline event. Classify the document name and
/// retain the saved type when that name alone does not identify one.
pub(super) fn stored_document_filing<'a>(
    title: &str,
    stored_type: &'a str,
) -> (&'a str, Option<&'static str>) {
    let head = title
        .split_once(" \u{2013} ")
        .map_or(title, |(head, _)| head);
    let head = head.to_lowercase();
    let doc_type = match crate::documents::type_from_title(&head) {
        "beleg" => stored_type,
        known => known,
    };
    (doc_type, document_account_ref(&head, doc_type))
}

/// The account a Trade Republic document belongs to (`tr:depot` or `tr:cash`), decided by what the
/// document is, never by the way it arrived (a booking's detail screen or the activity list): a
/// confirmation or credit for a security (trade, savings plan, dividend, corporate action, the
/// depot statement) belongs to the depot and documents a booking of the settlement account; account
/// statements, interest and fees belong to the settlement account. `None`: nothing says so, the
/// document stays where it is (tax papers, general information, an older document just called
/// "Dokumente" whose event is not known).
///
/// `head` is the lower-cased document title or the timeline context for a generic title.
fn document_account_ref(head: &str, doc_type: &str) -> Option<&'static str> {
    if doc_type == "kontoauszug" {
        return Some(if head.contains("depot") {
            "tr:depot"
        } else {
            "tr:cash"
        });
    }
    // interest and fees of the settlement account (a dividend is no interest)
    if !head.contains("dividende")
        && (["zins", "gebühr", "kontoführung"]
            .iter()
            .any(|k| head.contains(k))
            || head
                .split_whitespace()
                .any(|word| matches!(word, "interest" | "fee" | "fees") || word.ends_with("_fee")))
    {
        return Some("tr:cash");
    }
    if is_securities_notice(head) || doc_type == "abrechnung" {
        return Some("tr:depot");
    }
    None
}

/// Trade Republic names older documents just "Dokumente": what they are follows from the event of
/// the timeline entry they belong to (a dividend, a savings plan execution, a tax booking).
fn refine_document_type(doc_type: &'static str, event_text: &str) -> &'static str {
    if doc_type != "beleg" || event_text.is_empty() {
        return doc_type;
    }
    let has = |k: &str| event_text.contains(k);
    if has("steuer") || has("tax") {
        "steuer"
    } else if has("dividend")
        || has("ausschüttung")
        || has("sparplan")
        || has("savings_plan")
        || has("kauf")
        || has("trade")
        || has("order")
    {
        "abrechnung"
    } else {
        doc_type
    }
}

/// An account-level notice about a security (dividend election, corporate action):
/// it belongs to the depot, not to the settlement account.
fn is_securities_notice(lower_title: &str) -> bool {
    [
        "dividende",
        "wahlweise",
        "kapitalmaßnahme",
        "corporate_action",
        "split",
        "umtausch",
        "spin-off",
        "fusion",
        "hauptversammlung",
        "rückkaufangebot",
        "prospekt",
    ]
    .iter()
    .any(|k| lower_title.contains(k))
}

/// Trades of the securities history one sync reads (newest first).
const HISTORY_BATCH: usize = 400;

/// Trades whose detail screens one sync reads.
struct TradePlan<'a> {
    selected: Vec<&'a TradeRow>,
    /// Oldest date the history pass has reached (None: not started).
    before: Option<String>,
    complete: bool,
    /// Older trades still to read after this batch.
    remaining: usize,
}

/// Newest trades in the sync window each time, and the older history in batches, newest
/// first, so a long history (thousands of savings plan executions) is read over a few syncs
/// instead of one very long one. `rows` are newest first. A date at the batch border is
/// read again next time: activities are keyed by the trade id, so that is harmless.
fn plan_trades<'a>(
    rows: &'a [TradeRow],
    since: &str,
    before: Option<&str>,
    complete: bool,
) -> TradePlan<'a> {
    let is_older = |r: &TradeRow| !r.date.is_empty() && r.date.as_str() < since;
    // Recent trades have no history cursor. Capping them would repeatedly read the same newest
    // entries and permanently omit the rest once the history has already been completed.
    let mut selected: Vec<&TradeRow> = rows.iter().filter(|r| !is_older(r)).collect();
    if complete {
        return TradePlan {
            selected,
            before: before.map(str::to_string),
            complete: true,
            remaining: 0,
        };
    }
    let older: Vec<&TradeRow> = rows
        .iter()
        .filter(|r| is_older(r) && before.is_none_or(|b| r.date.as_str() <= b))
        .collect();
    let mut batch: Vec<&TradeRow> = older.iter().copied().take(HISTORY_BATCH).collect();
    // The checkpoint is a day, not an item id: finish its whole day so that more than one batch
    // on the same day cannot repeat forever without advancing the cursor.
    if let Some(border) = batch.last().map(|r| r.date.clone()) {
        batch.extend(
            older
                .iter()
                .copied()
                .skip(HISTORY_BATCH)
                .take_while(|r| r.date == border),
        );
        // A border day that fills a batch on its own would be read again in full by the next one:
        // take the next day along, also in the first batch (review of #1660).
        let border_fills_batch = older.iter().filter(|r| r.date == border).count() >= HISTORY_BATCH;
        if (before == Some(border.as_str()) || border_fills_batch)
            && let Some(next_day) = older.get(batch.len()).map(|r| r.date.clone())
        {
            let offset = batch.len();
            batch.extend(
                older
                    .iter()
                    .copied()
                    .skip(offset)
                    .take_while(|r| r.date == next_day),
            );
        }
    }
    let done = older.len() == batch.len();
    let next = if done {
        before.map(str::to_string)
    } else {
        batch.last().map(|r| r.date.clone())
    };
    let remaining = older.len().saturating_sub(batch.len());
    selected.extend(batch);
    TradePlan {
        selected,
        before: next,
        complete: done,
        remaining,
    }
}

/// Adds the trades that failed last time to the ones this sync reads (once each), wherever they are
/// in the timeline: the history cursor has moved on, so only this brings them back.
fn add_retries<'a>(selected: &mut Vec<&'a TradeRow>, rows: &'a [TradeRow], retry: &[String]) {
    for row in rows {
        if retry.contains(&row.id) && !selected.iter().any(|s| s.id == row.id) {
            selected.push(row);
        }
    }
}

fn is_trade_kind(kind_text: &str) -> bool {
    [
        "kauf",
        "sparplan",
        "savings_plan",
        "trade",
        "dividende",
        "dividend",
        "ausschüttung",
    ]
    .iter()
    .any(|k| kind_text.contains(k))
}

/// The securities side of a trade row (#314). Direction from the row's text
/// (sell before buy: "Verkauf" contains "kauf"), ISIN, units, price, fees and
/// taxes from the detail screen. Without ISIN or direction there is no
/// securities transaction. Value: units × price, else the cash amount
/// without fees and taxes. Best effort: the screen is not a documented API.
fn trade_security(row: &TradeRow, detail: &Value) -> Option<super::RemoteSecurityTransaction> {
    use crate::models::{SecurityDirection, SecurityTransaction};
    let k = &row.kind_text;
    let direction = if k.contains("dividend") || k.contains("ausschüttung") {
        SecurityDirection::Dividend
    } else if k.contains("verkauf") || k.contains("sell") {
        SecurityDirection::Sell
    } else if k.contains("kauf")
        || k.contains("sparplan")
        || k.contains("savings_plan")
        || k.contains("buy")
    {
        SecurityDirection::Buy
    } else {
        return None;
    };
    let d = parse_trade_detail(&row.icon, detail);
    let isin = d.isin?;
    let fees = d.fees_cents.unwrap_or(0);
    let taxes = d.taxes_cents.unwrap_or(0);
    let cash = row.amount_cents.abs();
    let quantity = d.quantity.unwrap_or(0.0);
    let by_price = d
        .price_cents
        .filter(|_| quantity > 0.0)
        .map(|p| (p as f64 * quantity).round() as i64);
    let value = by_price.unwrap_or(match direction {
        SecurityDirection::Buy => (cash - fees - taxes).max(0),
        SecurityDirection::Sell => cash + fees + taxes,
        _ => cash + taxes,
    });
    let price_cents = d.price_cents.unwrap_or(if quantity > 0.0 {
        (value as f64 / quantity).round() as i64
    } else {
        0
    });
    Some(super::RemoteSecurityTransaction {
        account_ref: "tr:depot".into(),
        external_id: row.id.clone(),
        cash_external_id: row.id.clone(),
        transaction: SecurityTransaction {
            booking_date: row.date.clone(),
            isin,
            name: row.title.clone(),
            direction,
            quantity,
            price_cents,
            currency: if row.currency.is_empty() {
                "EUR".into()
            } else {
                row.currency.clone()
            },
            value_cents: value,
            fees_cents: fees,
            taxes_cents: taxes,
            booking_text: row.kind_text.trim().to_string(),
            ..Default::default()
        },
    })
}

// --- Depot positions (compactPortfolioByType + instrument + ticker)

/// One position of the portfolio answer.
#[derive(Debug, Default, Clone, PartialEq)]
struct RawPosition {
    /// Instrument id as Trade Republic reports it: an ISIN, for some assets another key.
    isin: String,
    name: Option<String>,
    category: String,
    quantity: f64,
    /// Average purchase price per piece (bonds: see [`holding_of`]).
    average_buy_in: Option<f64>,
}

/// A JSON number, or a number sent as text ("40.745", "1.234,56").
fn json_as_f64(v: &Value) -> Option<f64> {
    if let Some(f) = v.as_f64() {
        return Some(f);
    }
    let text = v.as_str()?.trim();
    // plain decimals first: "40.745" is a price, not forty thousand
    text.parse::<f64>()
        .ok()
        .or_else(|| parse_decimal_text(text))
        .filter(|f| f.is_finite())
}

fn parse_position_item(item: &Value, default_category: &str) -> Option<RawPosition> {
    let isin = item["instrumentId"]
        .as_str()
        .or_else(|| item["isin"].as_str())
        .or_else(|| item["id"].as_str())
        .map(|s| s.trim().to_string())
        .or_else(|| item["icon"].as_str().and_then(isin_of))
        .filter(|s| !s.is_empty())?;
    let quantity = ["netSize", "shares", "size", "quantity", "units"]
        .iter()
        .find_map(|k| json_as_f64(&item[*k]))
        .unwrap_or(0.0);
    if quantity <= 0.0 {
        return None;
    }
    let average_buy_in = ["averageBuyIn", "buyIn", "averagePrice", "purchasePrice"]
        .iter()
        .find_map(|k| json_as_f64(&item[*k]));
    let name = ["name", "title", "shortName"]
        .iter()
        .find_map(|k| item[*k].as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let category = ["category", "type", "instrumentType"]
        .iter()
        .find_map(|k| item[*k].as_str())
        .unwrap_or(default_category)
        .to_string();
    Some(RawPosition {
        isin,
        name,
        category,
        quantity,
        average_buy_in,
    })
}

const DEFAULT_CATEGORY: &str = "stocksAndETFs";

/// The positions of a portfolio answer. `None` = the answer has none of the
/// known layouts: it says nothing about the depot (and must not empty it).
fn portfolio_positions(val: &Value) -> Option<Vec<RawPosition>> {
    fn category(v: &Value) -> &str {
        v["type"]
            .as_str()
            .or_else(|| v["categoryType"].as_str())
            .unwrap_or(DEFAULT_CATEGORY)
    }
    let mut out = Vec::new();
    if let Some(categories) = val["categories"].as_array() {
        for cat in categories {
            for pos in cat["positions"].as_array().into_iter().flatten() {
                out.extend(parse_position_item(pos, category(cat)));
            }
        }
    } else if let Some(positions) = val["positions"].as_array() {
        for pos in positions {
            out.extend(parse_position_item(pos, DEFAULT_CATEGORY));
        }
    } else {
        for item in val.as_array()? {
            if let Some(positions) = item["positions"].as_array() {
                for pos in positions {
                    out.extend(parse_position_item(pos, category(item)));
                }
            } else {
                out.extend(parse_position_item(item, DEFAULT_CATEGORY));
            }
        }
    }
    Some(out)
}

/// Securities account number of the session (`accountPairs`), the default product first.
fn securities_account_number(pairs: &Value) -> Option<String> {
    let mut accounts: Vec<&Value> = pairs["accounts"]
        .as_array()?
        .iter()
        .filter(|a| {
            a["securitiesAccountNumber"]
                .as_str()
                .is_some_and(|s| !s.is_empty())
        })
        .collect();
    accounts.sort_by_key(|a| a["productType"].as_str() != Some("DEFAULT"));
    accounts
        .first()
        .and_then(|a| a["securitiesAccountNumber"].as_str())
        .map(str::to_string)
}

/// What the `instrument` answer tells about a position.
#[derive(Debug, Default, Clone, PartialEq)]
struct InstrumentInfo {
    name: String,
    short_name: String,
    wkn: String,
    /// First exchange of `exchangeIds`: the ticker id is `<ISIN>.<exchange>`.
    exchange: Option<String>,
    /// `stock`, `fund`, `etf`, `crypto`, `bond`, `derivative` …
    type_id: String,
    /// `fundInfo.category` (`etf`).
    fund_category: String,
    /// `fundInfo.underlyingClass`: what the fund invests in ("Aktien", "Festverzinslich" …).
    underlying_class: String,
    /// `priceFactor`; only traced, to learn how bonds are quoted.
    price_factor: Option<f64>,
}

fn instrument_info(v: &Value) -> InstrumentInfo {
    let text = |v: &Value| v.as_str().unwrap_or_default().trim().to_string();
    InstrumentInfo {
        name: text(&v["name"]),
        short_name: text(&v["shortName"]),
        wkn: text(&v["wkn"]),
        exchange: v["exchangeIds"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        type_id: ["typeId", "type", "instrumentType"]
            .iter()
            .find_map(|k| v[*k].as_str())
            .unwrap_or_default()
            .trim()
            .to_lowercase(),
        fund_category: text(&v["fundInfo"]["category"]).to_lowercase(),
        underlying_class: text(&v["fundInfo"]["underlyingClass"]),
        price_factor: v["priceFactor"].as_f64(),
    }
}

/// `word` as a whole word of `text` ("ETC", not the "etc" in "Fletcher").
fn has_word(text: &str, word: &str) -> bool {
    text.split(|c: char| !c.is_alphanumeric())
        .any(|t| t.eq_ignore_ascii_case(word))
}

fn is_bond(category: &str, info: &InstrumentInfo) -> bool {
    let cat = category.to_lowercase();
    info.type_id == "bond" || cat.contains("bond") || cat.contains("anleihe")
}

fn is_crypto(category: &str, info: &InstrumentInfo) -> bool {
    let cat = category.to_lowercase();
    info.type_id == "crypto" || cat.contains("crypto") || cat.contains("krypto")
}

/// Asset class a fund invests in; an unknown label stays open for the name rules.
fn underlying_asset_class(label: &str) -> Option<AssetClass> {
    let l = label.trim().to_lowercase();
    if l.is_empty() {
        return None;
    }
    if let Some(known) = crate::asset_classification::from_labels(&l, "").asset_class {
        return Some(known);
    }
    let has = |parts: &[&str]| parts.iter().any(|p| l.contains(p));
    if has(&["multi", "misch"]) {
        Some(AssetClass::MultiAsset)
    } else if has(&["money", "geldmarkt"]) {
        Some(AssetClass::Cash)
    } else if has(&["bond", "anleihe", "renten", "fixed", "festverzins"]) {
        Some(AssetClass::Bond)
    } else if has(&["commodit", "rohstoff", "metal"]) {
        Some(AssetClass::Commodity)
    } else if has(&["immobil", "realestate", "real estate", "property"]) {
        Some(AssetClass::RealEstate)
    } else if has(&["crypto", "krypto"]) {
        Some(AssetClass::Crypto)
    } else if has(&["equit", "aktie", "stock"]) {
        Some(AssetClass::Equity)
    } else {
        None
    }
}

/// Classification from what Trade Republic says about the instrument, and
/// whether it is a directly held crypto asset. Only stated facts are imported:
/// the instrument type decides, never a substring of the name ("Netflix" is
/// no ETF), and a fund's asset class comes from its underlying class. What
/// stays `None` is left to the core's ISIN and name rules.
fn classify_tr_instrument(
    category: &str,
    info: &InstrumentInfo,
    name: &str,
) -> (ClassificationValues, bool) {
    let values = |asset_class, instrument| ClassificationValues {
        asset_class,
        instrument,
    };
    if is_crypto(category, info) {
        return (
            values(
                Some(AssetClass::Crypto),
                Some(InstrumentKind::DirectHolding),
            ),
            true,
        );
    }
    if is_bond(category, info) {
        return (
            values(Some(AssetClass::Bond), Some(InstrumentKind::SingleSecurity)),
            false,
        );
    }
    let cat = category.to_lowercase();
    if matches!(
        info.type_id.as_str(),
        "derivative" | "derivate" | "warrant" | "knockout" | "factor"
    ) || cat.contains("derivat")
    {
        return (values(None, Some(InstrumentKind::Derivative)), false);
    }
    let underlying = underlying_asset_class(&info.underlying_class);
    let named = |word: &str| has_word(name, word) || has_word(&info.name, word);
    if named("ETC") {
        return (
            values(
                underlying.or(Some(AssetClass::Commodity)),
                Some(InstrumentKind::EtcEtn),
            ),
            false,
        );
    }
    if named("ETN") {
        return (values(underlying, Some(InstrumentKind::EtcEtn)), false);
    }
    let classification = match info.type_id.as_str() {
        "etf" => values(underlying, Some(InstrumentKind::Etf)),
        "fund" if info.fund_category == "etf" => values(underlying, Some(InstrumentKind::Etf)),
        "fund" => values(underlying, Some(InstrumentKind::Fund)),
        "stock" => values(
            Some(AssetClass::Equity),
            Some(InstrumentKind::SingleSecurity),
        ),
        _ => values(None, None),
    };
    (classification, false)
}

/// Euro amount to cents, half away from zero.
fn to_cents(p: f64) -> i64 {
    if !p.is_finite() {
        return 0;
    }
    // the epsilon keeps .5 cases that binary floats store just below (40.745 * 100 = 4074.4999…)
    let eps = if p >= 0.0 { 1e-7 } else { -1e-7 };
    (p * 100.0 + eps).round() as i64
}

/// Prices of a `ticker` answer.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct Quote {
    last: Option<f64>,
    /// Previous close.
    pre: Option<f64>,
}

fn quote_of(tick: &Value) -> Quote {
    let price = |v: &Value| {
        json_as_f64(&v["price"])
            .or_else(|| json_as_f64(v))
            .filter(|p| *p > 0.0)
    };
    Quote {
        last: price(&tick["last"]).or_else(|| price(&tick["bid"])),
        pre: price(&tick["pre"]),
    }
}

/// Bonds are quoted in percent of the nominal (the quantity): 99.5 means 0.995 per unit.
const BOND_PRICE_FACTOR: f64 = 0.01;

/// The stored position. Values come from the unrounded price (quantity ×
/// price, then cents), so the depot total matches the bank's.
///
/// Without a quote the position is valued at cost (price = purchase price) and
/// reports no gain, instead of a value of zero.
fn holding_of(
    raw: &RawPosition,
    info: &InstrumentInfo,
    trade_name: Option<&str>,
    quote: Quote,
) -> RemoteHolding {
    let bond = is_bond(&raw.category, info);
    let scale = if bond { BOND_PRICE_FACTOR } else { 1.0 };
    let last = quote.last.map(|p| p * scale);
    let pre = quote.pre.map(|p| p * scale);
    let buy_in = raw.average_buy_in.filter(|p| *p > 0.0).map(|p| {
        // A bond's purchase price is in percent like its quote. Should it come per
        // unit already, it is close to the scaled quote and stays as it is.
        let per_unit = bond && last.is_some_and(|l| p < l * 10.0);
        if bond && !per_unit {
            p * BOND_PRICE_FACTOR
        } else {
            p
        }
    });

    let purchase_value_cents = buy_in.map(|p| to_cents(raw.quantity * p));
    let value_cents = last
        .map(|p| to_cents(raw.quantity * p))
        .or(purchase_value_cents)
        .unwrap_or(0);
    let prev_day_value_cents = pre.map(|p| to_cents(raw.quantity * p));
    // gains only against a real quote
    let gain = |base: Option<i64>| {
        let base = base.filter(|b| *b > 0)?;
        last?;
        Some((
            value_cents - base,
            (value_cents - base) as f64 / base as f64 * 100.0,
        ))
    };
    let purchase_gain = gain(purchase_value_cents);
    let day_gain = gain(prev_day_value_cents);

    let (classification, direct_crypto) =
        classify_tr_instrument(&raw.category, info, raw.name.as_deref().unwrap_or_default());
    // The core knows a directly held coin by its ticker in `wkn` with no ISIN.
    let (isin, wkn) = if direct_crypto && !is_isin(&raw.isin) {
        (String::new(), raw.isin.to_ascii_uppercase())
    } else {
        (raw.isin.clone(), info.wkn.clone())
    };
    let name = [
        raw.name.as_deref().unwrap_or_default(),
        &info.short_name,
        &info.name,
        trade_name.unwrap_or_default(),
        &raw.isin,
    ]
    .into_iter()
    .find(|s| !s.is_empty())
    .unwrap_or_default()
    .to_string();

    RemoteHolding {
        classification,
        direct_crypto,
        account_ref: DEPOT_REF.into(),
        holding: Holding {
            isin,
            wkn,
            name,
            quantity: raw.quantity,
            current_price_cents: last.or(buy_in).map(to_cents).unwrap_or(0),
            purchase_price_cents: buy_in.map(to_cents).unwrap_or(0),
            value_cents,
            prev_day_price_cents: pre.map(to_cents),
            prev_day_value_cents,
            profit_loss_purchase_cents: purchase_gain.map(|g| g.0),
            profit_loss_purchase_rel: purchase_gain.map(|g| g.1),
            profit_loss_prev_day_cents: day_gain.map(|g| g.0),
            profit_loss_prev_day_rel: day_gain.map(|g| g.1),
            // Trade Republic does not report how a position is held
            custody_type: String::new(),
            ..Default::default()
        },
    }
}

const DEPOT_REF: &str = "tr:depot";

/// Layout of an answer for the depot debug log: keys and value types only,
/// never a value. Keys that are identifiers themselves are masked.
fn shape(v: &Value, depth: u8) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(_) => "bool".into(),
        Value::Number(_) => "num".into(),
        Value::String(_) => "str".into(),
        Value::Array(a) if depth == 0 => format!("[{}]", a.len()),
        Value::Array(a) => format!(
            "[{}x {}]",
            a.len(),
            a.first().map(|x| shape(x, depth - 1)).unwrap_or_default()
        ),
        Value::Object(m) if depth == 0 => format!("{{{}}}", m.len()),
        Value::Object(m) => {
            let fields: Vec<String> = m
                .iter()
                .map(|(k, x)| {
                    let plain = k.len() <= 32
                        && !is_isin(k)
                        && k.chars().all(|c| c.is_ascii_alphabetic() || c == '_');
                    let key = if plain { k.as_str() } else { "<id>" };
                    format!("{key}:{}", shape(x, depth - 1))
                })
                .collect();
            format!("{{{}}}", fields.join(","))
        }
    }
}

/// A failed subscription request. `transport` = the connection itself failed
/// (send error, closed socket); otherwise the server refused or stayed silent
/// and the connection is still usable.
struct RequestError {
    transport: bool,
    error: CoreError,
}

const REQUEST_WAIT: std::time::Duration = std::time::Duration::from_secs(20);
const ACCOUNT_WAIT: std::time::Duration = std::time::Duration::from_secs(5);
const INSTRUMENT_WAIT: std::time::Duration = std::time::Duration::from_secs(10);
/// Instruments without a quote never answer: do not hold up the sync for them.
const TICKER_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Subscribes, waits up to `wait` for the first answer ("<id> A <json>") and unsubscribes.
async fn request_within<S>(
    ws: &mut S,
    id: u32,
    mut payload: Value,
    token: &str,
    wait: std::time::Duration,
) -> Result<Value, RequestError>
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + futures_util::Sink<Message>
        + Unpin,
    <S as futures_util::Sink<Message>>::Error: std::fmt::Display,
{
    let transport = |error| RequestError {
        transport: true,
        error,
    };
    let refused = |error| RequestError {
        transport: false,
        error,
    };
    if !token.is_empty() {
        payload["token"] = json!(token);
    }
    let kind = payload["type"].as_str().unwrap_or("?").to_string();
    ws.send(Message::text(format!("sub {id} {payload}")))
        .await
        .map_err(|e| transport(TradeRepublicConnector::ws_err(e)))?;
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let msg = match tokio::time::timeout_at(deadline, ws.next()).await {
            Err(_) => {
                let _ = ws.send(Message::text(format!("unsub {id}"))).await;
                return Err(refused(CoreError::Network {
                    message: trf("Trade Republic hat auf „{0}“ nicht geantwortet.", &[&kind]),
                }));
            }
            Ok(None) => {
                return Err(transport(CoreError::Network {
                    message: trf("WebSocket während „{0}“ geschlossen.", &[&kind]),
                }));
            }
            Ok(Some(Err(e))) => return Err(transport(TradeRepublicConnector::ws_err(e))),
            Ok(Some(Ok(msg))) => msg,
        };
        let Message::Text(text) = msg else { continue };
        let text = text.to_string();
        let mut parts = text.splitn(3, ' ');
        let (Some(rid), Some(code)) = (parts.next(), parts.next()) else {
            continue;
        };
        if rid != id.to_string() {
            continue;
        }
        let body = parts.next().unwrap_or("null");
        let _ = ws.send(Message::text(format!("unsub {id}"))).await;
        return match code {
            "A" => serde_json::from_str(body).map_err(|e| refused(e.into())),
            "E" => Err(refused(CoreError::Bank {
                code: format!("WS {kind}"),
                message: trf("Fehler-Frame: {0}", &[&snippet(body)]),
            })),
            _ => Ok(Value::Null),
        };
    }
}

/// [`request_within`] with the default wait; any failure is an error.
async fn request<S>(ws: &mut S, id: u32, payload: Value, token: &str) -> CoreResult<Value>
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + futures_util::Sink<Message>
        + Unpin,
    <S as futures_util::Sink<Message>>::Error: std::fmt::Display,
{
    request_within(ws, id, payload, token, REQUEST_WAIT)
        .await
        .map_err(|e| e.error)
}

/// The complete depot inventory, largest position first. `None` = it could not
/// be read reliably (no usable portfolio answer, or the connection failed on
/// the way): the stored positions and the depot value must stay as they are.
async fn fetch_portfolio<S>(
    ws: &mut S,
    sub_id: &mut u32,
    token: &str,
    trade_names: &std::collections::HashMap<String, String>,
) -> Option<Vec<RemoteHolding>>
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + futures_util::Sink<Message>
        + Unpin,
    <S as futures_util::Sink<Message>>::Error: std::fmt::Display,
{
    use crate::depot_debug::log;
    let mut next_id = || {
        *sub_id += 2;
        *sub_id
    };

    // The portfolio is asked per securities account; older sessions answer without it.
    let sec_acc_no = match request_within(
        ws,
        next_id(),
        json!({"type": "accountPairs"}),
        token,
        ACCOUNT_WAIT,
    )
    .await
    {
        Ok(pairs) => securities_account_number(&pairs),
        Err(e) if e.transport => {
            log("trade republic: accountPairs: connection failed, depot unchanged");
            return None;
        }
        Err(_) => None,
    };
    let mut attempts = vec![
        ("byType", json!({"type": "compactPortfolioByType"})),
        ("legacy", json!({"type": "compactPortfolio"})),
    ];
    if let Some(number) = &sec_acc_no {
        attempts.insert(
            0,
            (
                "byType+secAccNo",
                json!({"type": "compactPortfolioByType", "secAccNo": number}),
            ),
        );
    }
    let mut positions = None;
    for (label, payload) in attempts {
        match request_within(ws, next_id(), payload, token, REQUEST_WAIT).await {
            Ok(answer) => match portfolio_positions(&answer) {
                // only the current request may report an empty depot
                Some(found) if !found.is_empty() || label != "legacy" => {
                    log(&format!(
                        "trade republic: portfolio via {label}: positions={} shape={}",
                        found.len(),
                        shape(&answer, 5)
                    ));
                    positions = Some(found);
                    break;
                }
                _ => log(&format!(
                    "trade republic: portfolio via {label}: not usable, shape={}",
                    shape(&answer, 5)
                )),
            },
            Err(e) if e.transport => {
                log("trade republic: portfolio: connection failed, depot unchanged");
                return None;
            }
            Err(_) => log(&format!("trade republic: portfolio via {label}: no answer")),
        }
    }
    let positions = positions?;

    let mut holdings = Vec::with_capacity(positions.len());
    for (i, pos) in positions.iter().enumerate() {
        let info = match request_within(
            ws,
            next_id(),
            json!({"type": "instrument", "id": &pos.isin}),
            token,
            INSTRUMENT_WAIT,
        )
        .await
        {
            Ok(answer) => {
                if i == 0 {
                    log(&format!(
                        "trade republic: instrument shape={}",
                        shape(&answer, 2)
                    ));
                }
                instrument_info(&answer)
            }
            Err(e) if e.transport => {
                log("trade republic: instrument: connection failed, depot unchanged");
                return None;
            }
            Err(_) => InstrumentInfo::default(),
        };
        let exchange = info.exchange.as_deref().unwrap_or("LSX");
        let quote = match request_within(
            ws,
            next_id(),
            json!({"type": "ticker", "id": format!("{}.{}", pos.isin, exchange)}),
            token,
            TICKER_WAIT,
        )
        .await
        {
            Ok(answer) => {
                if i == 0 {
                    log(&format!(
                        "trade republic: ticker shape={}",
                        shape(&answer, 2)
                    ));
                }
                quote_of(&answer)
            }
            Err(e) if e.transport => {
                log("trade republic: ticker: connection failed, depot unchanged");
                return None;
            }
            Err(_) => Quote::default(),
        };
        // codes and flags only: no instrument id, name, quantity or price
        log(&format!(
            "trade republic: position {i}: category={} type={} fund={} underlying={} factor={:?} exchange={} isin_id={} last={} pre={} buy_in={}",
            pos.category,
            info.type_id,
            info.fund_category,
            info.underlying_class,
            info.price_factor,
            exchange,
            is_isin(&pos.isin),
            quote.last.is_some(),
            quote.pre.is_some(),
            pos.average_buy_in.is_some(),
        ));
        holdings.push(holding_of(
            pos,
            &info,
            trade_names.get(&pos.isin).map(String::as_str),
            quote,
        ));
    }
    holdings.sort_by_key(|h| std::cmp::Reverse(h.holding.value_cents));
    Some(holdings)
}

/// Connection state saved between syncs (inside the encrypted vault).
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct StoredSession {
    /// "tr_session=…; tr_refresh=…"
    cookies: SecretString,
    session_token: SecretString,
    phone: String,
    saved_unix: i64,
    /// Timeline item ids whose documents were already downloaded, oldest first.
    doc_items: Vec<String>,
    /// The securities history (trades older than the sync window) is read in batches, newest
    /// first: everything on or after this date has been read; `complete` = all of it.
    #[serde(
        rename = "trade_history_v3_before",
        skip_serializing_if = "Option::is_none"
    )]
    trade_history_before: Option<String>,
    #[serde(rename = "trade_history_v3_complete")]
    trade_history_complete: bool,
    /// Trades whose detail screen failed: read again next time, whatever the history cursor says
    /// (their securities activity would be missing for good).
    #[serde(
        rename = "trade_history_v3_retry",
        skip_serializing_if = "Vec::is_empty"
    )]
    trade_history_retry: Vec<String>,
    /// Older rows may no longer be in the current page range when a detail request is retried.
    #[serde(rename = "trade_retry_rows_v1", skip_serializing_if = "Vec::is_empty")]
    trade_retry_rows: Vec<TradeRow>,
    /// Start of the current history segment, separate from the day checkpoint within it.
    #[serde(
        rename = "transaction_history_cursor_v1",
        skip_serializing_if = "Option::is_none"
    )]
    transaction_history_cursor: Option<String>,
    /// Document candidates a pass did not get to (over the limit of one pass) or whose detail or
    /// download failed. They are tried again on the next syncs until they are stored, whatever the
    /// history cursor or the sync window say.
    #[serde(rename = "doc_pending_v1", skip_serializing_if = "Vec::is_empty")]
    doc_pending: Vec<DocumentCandidate>,
    /// The last listing of the account-level documents (`timelineActivityLog`) did not reach its end
    /// (a failed page, the page limit): the next sync lists on, even below the sync window, as long
    /// as this is set. Entries not listed cannot be in `doc_pending`.
    #[serde(rename = "doc_listing_open_v1")]
    doc_listing_open: bool,
    /// The Activity Log was listed to its very end once (with document retrieval on). Before that,
    /// no listing stops at the sync window: a history finished while retrieval was off has never
    /// listed the older statements (review of #1660).
    #[serde(rename = "doc_listing_complete_v1")]
    doc_listing_complete: bool,
    /// Next Activity Log page after a partial listing. A flag alone would repeat the same pages.
    #[serde(
        rename = "doc_listing_cursor_v1",
        skip_serializing_if = "Option::is_none"
    )]
    doc_listing_cursor: Option<String>,
}

/// Document ids kept in the state; the oldest are dropped first.
const MAX_DOC_ITEMS: usize = 2000;

/// Body of the PIN login; borrowed so the PIN is not copied before sending.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LoginRequest<'a> {
    phone_number: &'a str,
    pin: &'a str,
}

pub struct TradeRepublicConnector {
    jar: Arc<Jar>,
    client: Option<reqwest::Client>,
    process_id: String,
    session: StoredSession,
    challenge: Option<Challenge>,
}

impl TradeRepublicConnector {
    pub fn new() -> Self {
        TradeRepublicConnector {
            jar: Arc::new(Jar::default()),
            client: None,
            process_id: String::new(),
            session: StoredSession::default(),
            challenge: None,
        }
    }

    fn ws_err(e: impl std::fmt::Display) -> CoreError {
        CoreError::Network {
            message: format!("WebSocket: {e}"),
        }
    }

    fn client(&mut self) -> CoreResult<reqwest::Client> {
        if let Some(c) = &self.client {
            return Ok(c.clone());
        }
        let c = self.build_client(super::same_host_redirects())?;
        self.client = Some(c.clone());
        Ok(c)
    }

    /// Client for document downloads (GET without credentials in the body). Presigned
    /// document URLs may redirect to a storage host, so redirects to other hosts are
    /// followed here; reqwest drops the manual Cookie header on a cross-host redirect.
    fn document_client(&self) -> CoreResult<reqwest::Client> {
        self.build_client(reqwest::redirect::Policy::default())
    }

    fn build_client(&self, redirect: reqwest::redirect::Policy) -> CoreResult<reqwest::Client> {
        let mut h = HeaderMap::new();
        h.insert(USER_AGENT, HeaderValue::from_static(BROWSER_UA));
        h.insert(ORIGIN, HeaderValue::from_static(APP_ORIGIN));
        h.insert(
            REFERER,
            HeaderValue::from_static("https://app.traderepublic.com/"),
        );
        h.insert(ACCEPT, HeaderValue::from_static("application/json"));
        h.insert(
            "Accept-Language",
            HeaderValue::from_static("de-DE,de;q=0.9"),
        );
        let c = reqwest::Client::builder()
            .cookie_provider(self.jar.clone())
            .default_headers(h)
            .timeout(std::time::Duration::from_secs(30))
            .redirect(redirect)
            .referer(false)
            .build()?;
        Ok(c)
    }

    /// Reads status, Set-Cookie headers and body of a response.
    async fn read(resp: reqwest::Response) -> (u16, Vec<String>, String) {
        let status = resp.status().as_u16();
        let cookies = resp
            .headers()
            .get_all(SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok().map(|s| s.to_string()))
            .collect();
        let body = super::read_text_limited(resp).await.unwrap_or_default();
        (status, cookies, body)
    }

    fn http_error(step: &str, status: u16, body: &str) -> CoreError {
        let hint = match status {
            403 | 405 => tr(
                " – AWS-WAF-Schutz von Trade Republic. Bitte „Im Browser-Fenster anmelden“ verwenden.",
            ),
            426 => tr(
                " – CLIENT_VERSION_OUTDATED: Trade Republic verlangt einen neueren Client. Bitte „Im Browser-Fenster anmelden“ verwenden.",
            ),
            401 => tr(" – PIN, Code oder Sitzung ungültig."),
            429 => tr(" – zu viele Versuche, bitte etwas warten."),
            _ => "",
        };
        CoreError::Bank {
            code: format!("HTTP {status}"),
            message: trf(
                "Trade Republic {0}{1} Antwort: {2}",
                &[&step, &hint, &snippet(body)],
            ),
        }
    }

    fn code_challenge(&mut self, message: String, countdown: Option<u32>) -> Phase {
        let c = Challenge {
            kind: ChallengeKind::Code,
            title: "Trade Republic 2FA".into(),
            message,
            method_name: tr("App-Code (oder SMS)").into(),
            redirect_url: None,
            image: None,
            expires_in_seconds: countdown,
            poll_interval_seconds: 0,
        };
        self.challenge = Some(c.clone());
        Phase::Challenge(c)
    }

    fn advance_history(
        &mut self,
        before: Option<String>,
        segment_complete: bool,
        next_page: Option<String>,
    ) {
        // Advance the page cursor only once all date batches in this segment were processed.
        // A new segment starts a fresh date range, including when it begins on the same day.
        if segment_complete && next_page.is_some() {
            self.session.transaction_history_cursor = next_page;
            self.session.trade_history_before = None;
            self.session.trade_history_complete = false;
        } else {
            self.session.trade_history_before = before;
            self.session.trade_history_complete = segment_complete;
            if segment_complete {
                self.session.transaction_history_cursor = None;
            }
        }
    }

    fn state(&mut self) -> Option<SecretJson> {
        let session = &mut self.session;
        session.saved_unix = jiff::Timestamp::now().as_second();
        let old = session.doc_items.len().saturating_sub(MAX_DOC_ITEMS);
        session.doc_items.drain(..old);
        SecretJson::encode(session).ok()
    }

    /// Tries to refresh a stored session. Ok(true) = usable.
    async fn refresh_session(&mut self) -> CoreResult<bool> {
        let client = self.client()?;
        let resp = client
            .get(format!("{API}/api/v1/auth/web/session"))
            .header("Cookie", self.session.cookies.expose())
            .send()
            .await?;
        let (status, set, _body) = Self::read(resp).await;
        if (200..300).contains(&status) {
            self.session.cookies = merge_cookies(self.session.cookies.expose(), &set).into();
            return Ok(true);
        }
        // 401/403: expired. Anything else (endpoint moved …): try the WebSocket anyway.
        Ok(!(status == 401 || status == 403))
    }
}

/// Browser login needed (no WAF token). `expired`: a stored session was just
/// refused by Trade Republic.
fn browser_login_message(expired: bool) -> &'static str {
    if expired {
        tr(
            "Die Trade-Republic-Sitzung ist abgelaufen. Bitte im Browser-Fenster neu anmelden – die neue Sitzung wird danach gespeichert.",
        )
    } else {
        tr(
            "Trade Republic schützt die Anmeldung mit AWS WAF. Bitte im Browser-Fenster anmelden – die Sitzung wird danach gespeichert und wiederverwendet.",
        )
    }
}

/// PIN needed for a direct login. `expired`: as above.
fn pin_prompt_message(expired: bool) -> &'static str {
    if expired {
        tr(
            "Die Trade-Republic-Sitzung ist abgelaufen. Bitte die 4-stellige PIN eingeben (oder „Im Browser-Fenster anmelden“).",
        )
    } else {
        tr("Bitte die 4-stellige Trade-Republic-PIN eingeben (oder „Im Browser-Fenster anmelden“).")
    }
}

impl Default for TradeRepublicConnector {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BankConnector for TradeRepublicConnector {
    async fn begin(&mut self, ctx: &ConnectContext) -> CoreResult<Phase> {
        self.session = ctx.state.decode().unwrap_or_default();
        self.session.phone = normalize_phone(&ctx.login_name);
        // 1) Stored browser/API session (tr_session) -> no PIN, no 2FA.
        let had_session = self.session.cookies.expose().contains("tr_session=");
        if had_session && self.refresh_session().await? {
            return Ok(Phase::Ready);
        }
        // A stored session that Trade Republic no longer accepts: say so, so
        // the PIN / browser prompt below doesn't look like a new problem.
        let expired = had_session;
        // 2) Direct login only with a WAF token harvested by the browser
        //    (aws-waf-token cookie); a bare request gets HTTP 405 from AWS WAF.
        let waf = cookie_value(self.session.cookies.expose(), "aws-waf-token");
        if waf.is_none() {
            return Err(CoreError::BrowserLoginRequired {
                message: browser_login_message(expired).into(),
            });
        }
        if self.session.phone.len() < 8 {
            return Err(CoreError::InvalidInput {
                message: trf(
                    "Telefonnummer fehlt oder ist ungültig („{0}“). Bitte z. B. +4915112345678 eintragen.",
                    &[&ctx.login_name],
                ),
            });
        }
        if ctx.secret.is_empty() {
            return Err(CoreError::Authentication {
                message: pin_prompt_message(expired).into(),
            });
        }
        let client = self.client()?;
        let mut last = (0u16, String::new());
        for version in ["v1", "v2"] {
            let resp = client
                .post(format!("{API}/api/{version}/auth/web/login"))
                .header("x-aws-waf-token", waf.clone().unwrap_or_default())
                .header("Cookie", self.session.cookies.expose())
                .json(&LoginRequest {
                    phone_number: &self.session.phone,
                    pin: ctx.secret.expose(),
                })
                .send()
                .await?;
            let (status, set, text) = Self::read(resp).await;
            self.session.cookies = merge_cookies(self.session.cookies.expose(), &set).into();
            if (200..300).contains(&status) {
                let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                self.process_id = v["processId"].as_str().unwrap_or_default().to_string();
                if self.process_id.is_empty() {
                    return Err(Self::http_error(
                        tr("Login (keine processId)"),
                        status,
                        &text,
                    ));
                }
                let countdown = v["countdownInSeconds"].as_u64().map(|s| s as u32);
                return Ok(self.code_challenge(
                    tr("Geben Sie den 4-stelligen Code ein, der in der Trade-Republic-App angezeigt wird. Keine Push-Nachricht erhalten? „Code per SMS“ wählen.").into(),
                    countdown,
                ));
            }
            last = (status, text);
            if !(status == 404 || status == 426) {
                break; // v2 only as fallback for "not found" / "outdated client"
            }
        }
        if matches!(last.0, 403 | 405) {
            return Err(CoreError::BrowserLoginRequired {
                message: trf(
                    "AWS WAF hat den Login abgelehnt (HTTP {0}). Bitte im Browser-Fenster anmelden.",
                    &[&last.0],
                ),
            });
        }
        Err(Self::http_error("Login", last.0, &last.1))
    }

    async fn respond(
        &mut self,
        _ctx: &ConnectContext,
        response: ChallengeResponse,
    ) -> CoreResult<Phase> {
        let ChallengeResponse::Code(code) = response else {
            return Err(CoreError::InvalidInput {
                message: tr("Trade Republic benötigt den 4-stelligen Code").into(),
            });
        };
        let client = self.client()?;
        let code = code.expose().trim().to_string();
        if code.eq_ignore_ascii_case("sms") {
            let resp = client
                .post(format!(
                    "{API}/api/v1/auth/web/login/{}/sms",
                    self.process_id
                ))
                .send()
                .await?;
            let (status, _, text) = Self::read(resp).await;
            if !(200..300).contains(&status) {
                return Err(Self::http_error(tr("SMS-Anforderung"), status, &text));
            }
            return Ok(self.code_challenge(
                tr("Der Code wurde per SMS gesendet. Bitte die 4 Ziffern eingeben.").into(),
                None,
            ));
        }
        if !(code.len() == 4 && code.chars().all(|c| c.is_ascii_digit())) {
            return Err(CoreError::InvalidInput {
                message: tr("Der Code muss aus 4 Ziffern bestehen.").into(),
            });
        }
        let resp = client
            .post(format!(
                "{API}/api/v1/auth/web/login/{}/{}",
                self.process_id, code
            ))
            .send()
            .await?;
        let (status, set, text) = Self::read(resp).await;
        if !(200..300).contains(&status) {
            return Err(Self::http_error(tr("2FA-Bestätigung"), status, &text));
        }
        // HTTP 200 with an EMPTY body (or {}) is normal: the session comes as Set-Cookie.
        self.session.cookies = merge_cookies(self.session.cookies.expose(), &set).into();
        if let Ok(v) = serde_json::from_str::<Value>(&text)
            && let Some(t) = v["sessionToken"].as_str()
        {
            self.session.session_token = t.into();
        }
        if self.session.cookies.is_empty() {
            let url: url::Url = API.parse().map_err(|e| CoreError::Protocol {
                message: format!("{e}"),
            })?;
            if let Some(h) = self.jar.cookies(&url) {
                self.session.cookies = h.to_str().unwrap_or_default().into();
            }
        }
        if !self.session.cookies.expose().contains("tr_session")
            && self.session.session_token.is_empty()
        {
            return Err(CoreError::Authentication {
                message: trf(
                    "Trade Republic hat keine Sitzung (tr_session) gesetzt. HTTP {0}, Antwort: {1}",
                    &[&status, &snippet(&text)],
                ),
            });
        }
        Ok(Phase::Ready)
    }

    async fn fetch(&mut self, ctx: &ConnectContext) -> CoreResult<FetchResult> {
        let mut req = WS.into_client_request().map_err(Self::ws_err)?;
        {
            let h = req.headers_mut();
            if !self.session.cookies.is_empty() {
                h.insert(
                    "Cookie",
                    HeaderValue::from_str(self.session.cookies.expose()).map_err(Self::ws_err)?,
                );
            }
            h.insert("Origin", HeaderValue::from_static(APP_ORIGIN));
            h.insert("User-Agent", HeaderValue::from_static(BROWSER_UA));
        }
        let (mut ws, _) = tokio_tungstenite::connect_async(req).await.map_err(|e| {
            let detail = match &e {
                tokio_tungstenite::tungstenite::Error::Http(resp) => format!(
                    "HTTP {} {}",
                    resp.status(),
                    resp.body().as_ref().map(|b| snippet(&crate::text::decode_bank_bytes(b))).unwrap_or_default()
                ),
                other => other.to_string(),
            };
            if detail.starts_with("HTTP 401") || detail.starts_with("HTTP 403") {
                return CoreError::BrowserLoginRequired { message: trf("Trade-Republic-Sitzung abgelaufen ({0}). Bitte im Browser-Fenster neu anmelden.", &[&detail]) };
            }
            CoreError::Network { message: trf("WebSocket-Verbindung abgelehnt: {0}", &[&detail]) }
        })?;
        let hello = json!({"locale": "de", "platformId": "webtrading", "platformVersion": "safari - 18.0.0", "clientId": "app.traderepublic.com", "clientVersion": "1.2.0"});
        ws.send(Message::text(format!("connect 31 {hello}")))
            .await
            .map_err(Self::ws_err)?;
        // wait for "connected"
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            let msg = tokio::time::timeout_at(deadline, ws.next())
                .await
                .map_err(|_| CoreError::Network {
                    message: tr(
                        "Trade Republic hat „connect 31“ nicht bestätigt (Zeitüberschreitung).",
                    )
                    .into(),
                })?
                .ok_or_else(|| CoreError::Network {
                    message: tr(
                        "WebSocket wurde vor „connected“ geschlossen – Sitzung abgelaufen?",
                    )
                    .into(),
                })?
                .map_err(Self::ws_err)?;
            if let Message::Text(t) = msg {
                let t = t.to_string();
                if t.trim() == "connected" {
                    break;
                }
                return Err(CoreError::Bank {
                    code: "WS".into(),
                    message: trf("Unerwartete Antwort auf connect: {0}", &[&snippet(&t)]),
                });
            }
        }

        let session_token = self.session.session_token.clone();
        let token = session_token.expose();
        let mut out = FetchResult::default();
        let cash = match request(&mut ws, 1, json!({"type": "cash"}), token).await {
            Ok(v) => v,
            Err(_) => request(&mut ws, 3, json!({"type": "availableCash"}), token)
                .await
                .unwrap_or(Value::Null),
        };
        let items: Vec<Value> = match &cash {
            Value::Array(a) => a.clone(),
            Value::Object(_) => vec![cash.clone()],
            _ => vec![],
        };
        let eur = items
            .iter()
            .find(|c| c["currencyId"].as_str().unwrap_or("EUR") == "EUR")
            .cloned()
            .unwrap_or(Value::Null);
        let balance = if eur["amount"].is_null() {
            amount_cents(&eur["value"])
        } else {
            amount_cents(&eur["amount"])
        };
        let iban = ["iban", "accountNumber"]
            .iter()
            .find_map(|k| {
                eur[*k]
                    .as_str()
                    .filter(|s| s.len() > 10)
                    .map(|s| s.to_string())
            })
            .unwrap_or_default();
        out.accounts.push(RemoteAccount {
            external_ref: "tr:cash".into(),
            name: "Verrechnungskonto".into(),
            owner: String::new(),
            iban,
            account_number: self.session.phone.clone(),
            currency: "EUR".into(),
            kind: AccountKind::Checking,
            balance_cents: balance,
            balance_date: super::today(),
        });

        let mut sub_id = 2u32;
        let mut doc_candidates: Vec<DocumentCandidate> = Vec::new();
        let mut trade_rows: Vec<TradeRow> = Vec::new();
        // After document retrieval was switched on again: reach back to the day it was switched off.
        let window_start = window_start(ctx.documents.catchup_since(), &ctx.since);
        // bookings older than the sync window, read in the history pass (stored as checked)
        let mut older_bookings: Vec<RemoteTransaction> = Vec::new();
        // the history pass pages through the whole timeline (cheap); details are batched below
        // (trades to read again lie anywhere in the timeline: it is paged through to the end)
        // One account from its sidebar menu (#1124): `keep_only` cuts the result down to it after
        // this fetch. The history pass and the document queue wait for the next full sync instead
        // of moving on with data that is thrown away (a cash-only sync would drop the depot's
        // activities of a history batch for good, review of #1660).
        let single_account = ctx.only_account.is_some();
        let (history_done, resume) = history_scope(&self.session, single_account);
        let mut next_history_page = None;
        let mut seen = std::collections::HashSet::new();
        // While backfilling older pages, still refresh the recent window from the head.
        for recent_only in [true, false] {
            if recent_only && resume.is_none() {
                continue;
            }
            let mut cursor = if recent_only { None } else { resume.clone() };
            let mut pass = TimelinePass {
                recent_only,
                history_done,
                in_history: !recent_only && resume.is_some(),
            };
            let page_limit = if recent_only || history_done {
                200
            } else {
                600
            };
            'pages: for _ in 0..page_limit {
                let mut payload = json!({"type": "timelineTransactions"});
                if let Some(c) = &cursor {
                    payload["after"] = json!(c);
                }
                let Some(page) = pass.page(request(&mut ws, sub_id, payload, token).await)? else {
                    // Keep the failed page as the continuation, never its successor.
                    break 'pages;
                };
                sub_id += 2;
                for item in page["items"].as_array().cloned().unwrap_or_default() {
                    let date: String = item["timestamp"]
                        .as_str()
                        .unwrap_or("")
                        .chars()
                        .take(10)
                        .collect();
                    // Older than the sync window: only trades are read, once, so that the depot's
                    // activities cover the history (bookings and documents stay incremental).
                    let below_window = !date.is_empty() && date.as_str() < window_start.as_str();
                    if pass.stop_at_window(below_window) {
                        cursor = None;
                        break 'pages;
                    }
                    if item["amount"].is_null() || item["status"].as_str() == Some("CANCELED") {
                        continue;
                    }
                    let subtitle = item["subtitle"].as_str().unwrap_or_default().to_string();
                    let item_id = item["id"].as_str().unwrap_or_default().to_string();
                    if !item_id.is_empty() && !seen.insert(item_id.clone()) {
                        continue;
                    }
                    let kind_text = format!(
                        "{} {} {}",
                        subtitle,
                        item["eventType"].as_str().unwrap_or(""),
                        item["title"].as_str().unwrap_or("")
                    )
                    .to_lowercase();
                    if !item_id.is_empty()
                        && !below_window
                        && !self.session.doc_items.contains(&item_id)
                        && [
                            "sparplan",
                            "savings_plan",
                            "kauf",
                            "verkauf",
                            "order",
                            "trade",
                            "zinsen",
                            "interest",
                            "dividende",
                            "dividend",
                            "steuer",
                            "tax",
                            "saveback",
                            "round up",
                            "spare_change",
                            "ausschüttung",
                            "corporate_action",
                        ]
                        .iter()
                        .any(|k| kind_text.contains(k))
                    {
                        doc_candidates.push(DocumentCandidate::from_timeline_item(
                            &item,
                            item_id.clone(),
                            date.clone(),
                        ));
                    }
                    if !item_id.is_empty() && is_trade_kind(&kind_text) {
                        trade_rows.push(TradeRow {
                            id: item_id.clone(),
                            date: date.clone(),
                            title: item["title"].as_str().unwrap_or_default().to_string(),
                            kind_text: kind_text.clone(),
                            event_context: format!(
                                "{} {}",
                                subtitle,
                                item["eventType"].as_str().unwrap_or("")
                            )
                            .to_lowercase(),
                            amount_cents: amount_cents(&item["amount"]["value"]),
                            currency: item["amount"]["currency"]
                                .as_str()
                                .unwrap_or("EUR")
                                .to_string(),
                            icon: item["icon"].as_str().unwrap_or_default().to_string(),
                        });
                    }
                    let mut booking = RemoteTransaction {
                        account_ref: "tr:cash".into(),
                        external_id: item["id"].as_str().unwrap_or_default().to_string(),
                        booking_date: date.clone(),
                        value_date: date,
                        amount_cents: amount_cents(&item["amount"]["value"]),
                        currency: item["amount"]["currency"]
                            .as_str()
                            .unwrap_or("EUR")
                            .to_string(),
                        counterparty: item["title"].as_str().unwrap_or_default().to_string(),
                        counterparty_iban: String::new(),
                        purpose: subtitle.clone(),
                        booking_text: if subtitle.is_empty() {
                            item["eventType"].as_str().unwrap_or_default().to_string()
                        } else {
                            subtitle
                        },
                        ..Default::default()
                    };
                    if below_window {
                        booking.checked = true;
                        older_bookings.push(booking);
                        continue;
                    }
                    out.transactions.push(booking);
                }
                cursor = page["cursors"]["after"].as_str().map(|s| s.to_string());
                if cursor.is_none() {
                    break;
                }
            }
            if !recent_only {
                next_history_page = cursor;
            }
        }

        let transaction_candidates = doc_candidates.len();
        // --- Activity Log: timelineActivityLog
        // Covers non-transaction documents such as quarterly account/depot statements,
        // annual tax certificates, and corporate actions (which carry no cash movements).
        let mut act_cursor = self.session.doc_listing_cursor.clone();
        // whether the listing got to its end (or to the window, where an earlier pass had finished)
        let mut listing_complete = false;
        let activity_limit = if ctx.documents.enabled() && !single_account {
            100
        } else {
            0
        };
        'activity_pages: for _ in 0..activity_limit {
            let mut payload = json!({"type": "timelineActivityLog"});
            if let Some(c) = &act_cursor {
                payload["after"] = json!(c);
            }
            sub_id += 2;
            let page = match request(&mut ws, sub_id, payload, token).await {
                Ok(p) => p,
                // timelineActivityLog not available or failing: the listing stays open
                Err(_) => break,
            };
            for item in page["items"].as_array().cloned().unwrap_or_default() {
                let date: String = item["timestamp"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(10)
                    .collect();
                if activity_log_stops_at(&date, &window_start, &self.session) {
                    listing_complete = true;
                    break 'activity_pages;
                }
                let item_id = item["id"].as_str().unwrap_or_default().to_string();
                if item_id.is_empty()
                    || self.session.doc_items.contains(&item_id)
                    || doc_candidates
                        .iter()
                        .any(|candidate| candidate.id == item_id)
                {
                    continue;
                }
                let subtitle = item["subtitle"].as_str().unwrap_or_default().to_string();
                let title = item["title"].as_str().unwrap_or_default().to_string();
                let kind_text = format!(
                    "{} {} {}",
                    subtitle,
                    item["eventType"].as_str().unwrap_or(""),
                    title
                )
                .to_lowercase();

                let is_candidate = [
                    "sparplan",
                    "savings_plan",
                    "kauf",
                    "verkauf",
                    "order",
                    "trade",
                    "zinsen",
                    "interest",
                    "dividende",
                    "dividend",
                    "steuer",
                    "tax",
                    "ertragsaufstellung",
                    "auszug",
                    "statement",
                    "abschluss",
                    "rechnung",
                    "report",
                    "saveback",
                    "round up",
                    "spare_change",
                    "ausschüttung",
                    "corporate_action",
                    "kapitalmaßnahme",
                    "split",
                    "übertrag",
                    "transfer",
                    "einbuchung",
                    "ausbuchung",
                ]
                .iter()
                .any(|k| kind_text.contains(k))
                    || item["action"]["type"].as_str() == Some("timelineDetail")
                    || item["action"]["type"].as_str() == Some("timelineDetailV2");

                if is_candidate {
                    doc_candidates
                        .push(DocumentCandidate::from_timeline_item(&item, item_id, date));
                }
            }
            act_cursor = page["cursors"]["after"].as_str().map(|s| s.to_string());
            if act_cursor.is_none() {
                listing_complete = true;
                self.session.doc_listing_complete = true;
                break;
            }
        }
        // a listing that stopped early is picked up again, whatever the trade history says
        if ctx.documents.enabled() && !single_account {
            self.session.doc_listing_open = !listing_complete;
            self.session.doc_listing_cursor = if listing_complete { None } else { act_cursor };
        }

        // Account-level documents (statements, tax certificates) are rare: they go first, so
        // the per-sync limit never starves them behind the many booking receipts.
        doc_candidates.rotate_left(transaction_candidates);
        // This sync's trades (the recent ones and a batch of the history) and their detail
        // screens: the documents of the older ones are fetched from the same screens.
        let mut plan = plan_trades(
            &trade_rows,
            &window_start,
            self.session.trade_history_before.as_deref(),
            self.session.trade_history_complete || single_account,
        );
        let retry_rows = self.session.trade_retry_rows.clone();
        if !single_account {
            add_retries(
                &mut plan.selected,
                &trade_rows,
                &self.session.trade_history_retry,
            );
            // Old connection states stored only IDs. Keep unresolved ones until the full scan ends.
            if !plan.complete || next_history_page.is_some() {
                self.session
                    .trade_history_retry
                    .retain(|id| !plan.selected.iter().any(|row| &row.id == id));
            } else {
                self.session.trade_history_retry.clear();
            }
            for row in &retry_rows {
                if !plan.selected.iter().any(|selected| selected.id == row.id) {
                    plan.selected.push(row);
                }
            }
        }
        // The bookings of the history batch: the days from the oldest trade of this batch up to
        // where the last sync stopped (everything left, in the final batch).
        {
            let upper = self.session.trade_history_before.as_deref();
            let lower = if plan.complete {
                None
            } else {
                plan.before.as_deref()
            };
            out.transactions
                .extend(older_bookings.into_iter().filter(|t| {
                    !t.booking_date.is_empty()
                        && upper.is_none_or(|u| t.booking_date.as_str() <= u)
                        && lower.is_none_or(|l| t.booking_date.as_str() >= l)
                }));
        }
        for row in &plan.selected {
            let older = !row.date.is_empty() && row.date.as_str() < window_start.as_str();
            // a known document of a trade whose booking is imported now is only linked (no download)
            let linkable = out.transactions.iter().any(|t| t.external_id == row.id);
            if older
                && (!self.session.doc_items.contains(&row.id) || linkable)
                && !doc_candidates
                    .iter()
                    .any(|candidate| candidate.id == row.id)
            {
                doc_candidates.push(DocumentCandidate {
                    id: row.id.clone(),
                    date: row.date.clone(),
                    title: row.title.clone(),
                    event_context: row.event_context.clone(),
                });
            }
        }
        // Document retrieval switched off: no detail screens for documents, nothing remembered
        // (the securities page below fetches the trade screens it needs itself).
        let documents_off = !ctx.documents.enabled();
        // a single-account sync keeps the candidates for the next full one: the documents of the
        // other account would be remembered as downloaded and then thrown away by `keep_only`
        let mut doc_candidates = documents_for_fetch(
            doc_candidates,
            &mut self.session.doc_pending,
            !documents_off && !single_account,
        );
        let doc_limit = if self.session.trade_history_complete {
            50
        } else {
            50 + HISTORY_BATCH
        };
        // Documents not looked at (more than one pass takes, a failed request): kept in
        // `doc_pending`, a catch-up goes on.
        let beyond_limit: Vec<DocumentCandidate> = if doc_candidates.len() > doc_limit {
            doc_candidates.split_off(doc_limit)
        } else {
            Vec::new()
        };
        let mut docs_incomplete = !beyond_limit.is_empty();
        let mut still_open: Vec<DocumentCandidate> = Vec::new();
        // --- Beleg-Abruf: timelineDetailV2 -> "Dokumente" section -> PDF download
        let client = self.document_client()?;
        let mut details: std::collections::HashMap<String, Value> = Default::default();
        for candidate in doc_candidates {
            let item_id = &candidate.id;
            sub_id += 2;
            let Ok(detail) = request(
                &mut ws,
                sub_id,
                json!({"type": "timelineDetailV2", "id": item_id}),
                token,
            )
            .await
            else {
                docs_incomplete = true;
                still_open.push(candidate);
                continue;
            };
            details.insert(item_id.clone(), detail.clone());
            let known_item = self.session.doc_items.contains(item_id);
            let mut download_failed = false;
            for section in detail["sections"].as_array().cloned().unwrap_or_default() {
                let is_docs = section["type"].as_str() == Some("documents")
                    || section["title"]
                        .as_str()
                        .map(|t| t.contains("Dokument"))
                        .unwrap_or(false);
                if !is_docs {
                    continue;
                }
                for d in section["data"].as_array().cloned().unwrap_or_default() {
                    let url = d["action"]["payload"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string();
                    let Ok(parsed) = reqwest::Url::parse(&url) else {
                        continue;
                    };
                    if parsed.scheme() != "https" {
                        continue;
                    }
                    // Stored earlier: no second download, only the link to the booking. The booking can
                    // come with this fetch or with an earlier one (the document pass was full then);
                    // storing looks it up and links only a receipt that has none yet.
                    if known_item {
                        out.documents.push(crate::documents::RemoteDocument {
                            account_ref: "tr:cash".into(),
                            booking_external_id: Some(item_id.clone()),
                            title: String::new(),
                            doc_type: String::new(),
                            mime_type: String::new(),
                            file_name: String::new(),
                            bytes: Vec::new(),
                            source_url: url.split('?').next().unwrap_or(&url).to_string(),
                            date: candidate.date.clone(),
                        });
                        continue;
                    }
                    // Only send the session cookie to Trade Republic hosts; presigned
                    // CDN/S3 URLs get a cookie-free request. (reqwest also strips the
                    // Cookie header on cross-host redirects.)
                    let mut req = client.get(parsed.clone());
                    if is_trade_republic_host(&parsed) {
                        req = req.header("Cookie", self.session.cookies.expose());
                    }
                    let Ok(resp) = req.send().await else {
                        download_failed = true;
                        continue;
                    };
                    if !resp.status().is_success() {
                        download_failed = true;
                        continue;
                    }
                    let Ok(bytes) = super::read_body_limited(resp, super::MAX_DOC_BYTES).await
                    else {
                        download_failed = true;
                        continue;
                    };
                    if !bytes.starts_with(b"%PDF") {
                        // an error or login page with HTTP 200 is no document: try again later
                        download_failed = true;
                        continue;
                    }
                    let doc_title = d["title"].as_str().unwrap_or("Dokument").to_string();
                    let (doc_type, account_ref) = candidate.classify(&doc_title);
                    let doc_date = d["detail"]
                        .as_str()
                        .and_then(|s| crate::documents::dates_in(s).into_iter().next())
                        .unwrap_or_else(|| candidate.date.clone());
                    // strip the presigned query part for a stable de-dup key
                    let stable = url.split('?').next().unwrap_or(&url).to_string();

                    out.documents.push(crate::documents::RemoteDocument {
                        account_ref: account_ref.into(),
                        booking_external_id: Some(item_id.clone()),
                        title: format!("{doc_title} – {}", candidate.title),
                        doc_type: doc_type.into(),
                        mime_type: "application/pdf".into(),
                        file_name: format!("{}_{}.pdf", doc_date, doc_title.replace(' ', "_")),
                        bytes: bytes.to_vec(),
                        source_url: stable,
                        date: doc_date,
                    });
                }
            }
            // a failed download is retried by the next sync
            docs_incomplete |= download_failed;
            if download_failed {
                still_open.push(candidate);
            } else if !known_item {
                self.session.doc_items.push(candidate.id);
            }
        }
        if !documents_off && !single_account {
            self.session.doc_pending = pending_after_pass(beyond_limit, still_open);
        }
        out.documents_incomplete =
            docs_incomplete || (!documents_off && self.session.doc_listing_open);
        // --- Wertpapierseite (#314): units, price, fees and taxes of the newest
        // trades from their detail screens (cached from the document pass).
        // A trade whose screen fails is skipped; the cash booking stays.
        let mut failed_trades = Vec::new();
        for row in plan.selected {
            let detail = match details.remove(&row.id) {
                Some(d) => d,
                None => {
                    sub_id += 2;
                    match request(
                        &mut ws,
                        sub_id,
                        json!({"type": "timelineDetailV2", "id": row.id}),
                        token,
                    )
                    .await
                    {
                        Ok(d) => d,
                        Err(_) => {
                            failed_trades.push(row.clone());
                            continue;
                        }
                    }
                }
            };
            if let Some(t) = trade_security(row, &detail) {
                out.security_transactions.push(t);
            }
        }

        // --- Depotpositionen: portfolio, then instrument data and a quote per position.
        // Names of the trade rows fill in where the portfolio and the instrument have none.
        let mut trade_names: std::collections::HashMap<String, String> = Default::default();
        for row in &trade_rows {
            if let Some(isin) = isin_of(&row.icon).filter(|_| !row.title.is_empty()) {
                trade_names.entry(isin).or_insert_with(|| row.title.clone());
            }
        }
        let portfolio = fetch_portfolio(&mut ws, &mut sub_id, token, &trade_names).await;
        add_depot_account(
            &mut out,
            portfolio,
            ctx.since_by_account.contains_key(DEPOT_REF),
        );
        let _ = ws.close(None).await;
        // An open page cursor means this pass does not know the full history size.
        let history_total_known = resume.is_none() && next_history_page.is_none();
        if !single_account {
            self.advance_history(plan.before, plan.complete, next_history_page);
        }
        // the history is not complete while a trade is still to be read again
        // documents still open (downloads, a listing that stopped early) keep "Weiter abrufen" on
        out.history_incomplete = !self.session.trade_history_complete
            || !failed_trades.is_empty()
            || (!documents_off
                && (!self.session.doc_pending.is_empty() || self.session.doc_listing_open));
        if single_account {
            // the rows waiting for a retry stay; recent ones that failed now join them
            let mut rows = retry_rows.clone();
            for row in failed_trades {
                if !rows.iter().any(|r| r.id == row.id) {
                    rows.push(row);
                }
            }
            self.session.trade_retry_rows = rows;
        } else {
            self.session.trade_retry_rows = failed_trades;
        }
        out.history_remaining = plan.remaining as u32;
        if history_total_known && plan.remaining > 0 {
            out.history_total = trade_rows.len() as u32;
        }
        out.state = self.state();
        Ok(out)
    }
}

fn add_depot_account(
    out: &mut FetchResult,
    portfolio: Option<Vec<RemoteHolding>>,
    known_depot: bool,
) {
    let has_depot = portfolio.as_ref().is_some_and(|h| !h.is_empty())
        || !out.security_transactions.is_empty()
        || known_depot
        // A document filed at the depot is evidence of the account even if the bank did
        // not return positions or security transactions on this fetch.
        || out.documents.iter().any(|d| d.account_ref == DEPOT_REF);
    if has_depot {
        let balance_cents = match portfolio {
            Some(holdings) => {
                let total = holdings.iter().map(|h| h.holding.value_cents).sum();
                out.holdings = holdings;
                out.holdings_fetched.push(DEPOT_REF.into());
                total
            }
            None => {
                // not read: the stored positions and the depot value stay
                out.balances_unknown.push(DEPOT_REF.into());
                0
            }
        };
        out.accounts.push(RemoteAccount {
            external_ref: DEPOT_REF.into(),
            name: "Depot".into(),
            owner: String::new(),
            iban: String::new(),
            // no phone number here: it would show up as an account number
            account_number: String::new(),
            currency: "EUR".into(),
            kind: AccountKind::Brokerage,
            balance_cents,
            balance_date: super::today(),
        });
    }
}

/// True if the URL's host is `traderepublic.com` or a subdomain of it.
fn is_trade_republic_host(url: &reqwest::Url) -> bool {
    url.host_str()
        .map(|h| {
            let h = h.to_ascii_lowercase();
            h == "traderepublic.com" || h.ends_with(".traderepublic.com")
        })
        .unwrap_or(false)
}

/// Where the timeline's documents are read from: the sync window, or earlier back to the day document
/// retrieval was switched off.
fn window_start(catchup_since: Option<&str>, since: &str) -> String {
    catchup_since
        .filter(|d| *d < since)
        .unwrap_or(since)
        .to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn documents_reach_back_to_the_catchup_day_only_when_it_is_earlier() {
        assert_eq!(super::window_start(None, "2026-08-01"), "2026-08-01");
        assert_eq!(
            super::window_start(Some("2026-05-01"), "2026-08-01"),
            "2026-05-01"
        );
        assert_eq!(
            super::window_start(Some("2026-09-01"), "2026-08-01"),
            "2026-08-01"
        );
    }

    use super::*;

    #[test]
    fn generic_document_uses_the_event_to_determine_its_account() {
        for (event, expected) in [
            ("kapitalmaßnahme corporate_action apple", DEPOT_REF),
            ("corporate_action apple", DEPOT_REF),
            ("kauf trade apple", DEPOT_REF),
            ("zinsen interest", "tr:cash"),
            ("gebühren", "tr:cash"),
            ("order fee", "tr:cash"),
            ("steuer dividende apple", "tr:cash"),
        ] {
            let candidate = DocumentCandidate {
                id: "evt-1".into(),
                date: "2026-11-21".into(),
                title: "Apple".into(),
                event_context: event.into(),
            };
            assert_eq!(candidate.classify("Dokumente").1, expected, "{event}");
        }
    }

    #[test]
    fn an_instrument_name_cannot_route_a_purchase_to_the_cash_account() {
        for title in ["Interest Rate Hedged Bond ETF", "Deka Zins Plus"] {
            let candidate = DocumentCandidate::from_timeline_item(
                &json!({"title": title, "subtitle": "Kauf", "eventType": "trade"}),
                "evt-1".into(),
                "2026-11-21".into(),
            );
            assert_eq!(candidate.title, title);
            assert_eq!(candidate.classify("Dokumente"), ("abrechnung", DEPOT_REF));
        }
    }

    #[test]
    fn first_fetch_files_a_generic_trade_pdf_at_the_depot_without_a_portfolio() {
        use super::super::store_fetch;
        use crate::{models::ConnectorKind, vault::Vault};

        let dir = crate::test_support::TempDir::new("girocrab-tr-document-import");
        let v = Vault::open(
            dir.join("v.db").to_string_lossy().into_owned(),
            vec![3u8; 32],
        )
        .unwrap();
        let kind = ConnectorKind::TradeRepublic;
        let conn = v
            .add_bank_connection(kind, String::new(), "login".into())
            .unwrap();
        let cash = RemoteAccount {
            external_ref: "tr:cash".into(),
            name: "Verrechnungskonto".into(),
            kind: AccountKind::Checking,
            currency: "EUR".into(),
            owner: String::new(),
            iban: String::new(),
            account_number: String::new(),
            balance_cents: 0,
            balance_date: "2025-12-04".into(),
        };
        let candidate = DocumentCandidate {
            id: "trade-1".into(),
            date: "2025-12-04".into(),
            title: "Apple".into(),
            event_context: "kauf trade".into(),
        };
        let (doc_type, account_ref) = candidate.classify("Dokumente");
        assert_eq!((doc_type, account_ref), ("abrechnung", DEPOT_REF));
        let transaction = RemoteTransaction {
            account_ref: "tr:cash".into(),
            external_id: "trade-1".into(),
            booking_date: "2025-12-04".into(),
            value_date: "2025-12-04".into(),
            amount_cents: -100_00,
            currency: "EUR".into(),
            ..Default::default()
        };
        let mut fetched = FetchResult {
            accounts: vec![cash],
            transactions: vec![transaction],
            documents: vec![crate::documents::RemoteDocument {
                account_ref: account_ref.into(),
                booking_external_id: Some(candidate.id),
                doc_type: doc_type.into(),
                title: "Dokumente – Apple".into(),
                mime_type: "application/pdf".into(),
                file_name: "beleg.pdf".into(),
                source_url: "https://example.invalid/beleg.pdf".into(),
                date: candidate.date,
                bytes: crate::documents::simple_pdf(&["Wertpapierabrechnung".into()]),
            }],
            ..Default::default()
        };
        add_depot_account(&mut fetched, None, false);
        assert_eq!(fetched.balances_unknown, [DEPOT_REF]);
        assert!(fetched.holdings_fetched.is_empty());
        store_fetch(&v, kind, conn, fetched).unwrap();
        let refs: (String, String) = v
            .db()
            .unwrap()
            .query_row(
                "SELECT depot.external_ref, cash.external_ref
               FROM booking_documents d JOIN accounts depot ON depot.id = d.account_id
               JOIN transactions t ON t.id = d.booking_id JOIN accounts cash ON cash.id = t.account_id",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(refs, ("tr:depot".into(), "tr:cash".into()));
    }

    #[test]
    fn stored_session_keeps_its_json_shape() {
        let stored = json!({
            "cookies": "tr_session=abc", "session_token": "st", "phone": "+491701234567",
            "saved_unix": 7, "doc_items": ["a", "b"]
        });
        let session: StoredSession = SecretJson::from(stored.clone()).decode().unwrap();
        assert_eq!(session.cookies.expose(), "tr_session=abc");
        let saved = serde_json::to_value(&session).unwrap();
        for (key, value) in stored.as_object().unwrap() {
            assert_eq!(&saved[key], value);
        }
    }

    #[test]
    fn login_request_uses_the_api_field_names() {
        let body = LoginRequest {
            phone_number: "+491701234567",
            pin: "1234",
        };
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            json!({"phoneNumber": "+491701234567", "pin": "1234"})
        );
    }

    #[test]
    fn saved_state_keeps_the_newest_doc_items_in_order() {
        let mut c = TradeRepublicConnector::new();
        c.session.doc_items = (0..=MAX_DOC_ITEMS).map(|i| i.to_string()).collect();
        let saved: StoredSession = c.state().unwrap().decode().unwrap();
        assert_eq!(saved.doc_items.len(), MAX_DOC_ITEMS);
        assert_eq!(saved.doc_items.first().unwrap(), "1");
        assert_eq!(saved.doc_items.last().unwrap(), &MAX_DOC_ITEMS.to_string());
    }

    #[tokio::test]
    async fn login_client_does_not_follow_redirects_to_another_origin() {
        let client = TradeRepublicConnector::new().client().unwrap();
        crate::connectors::tests::redirects::assert_no_cross_origin_replay(&client).await;
    }

    #[tokio::test]
    async fn document_client_follows_redirects_to_another_origin() {
        let client = TradeRepublicConnector::new().document_client().unwrap();
        crate::connectors::tests::redirects::assert_follows_cross_origin_get(&client).await;
    }

    fn detail(extra_rows: serde_json::Value) -> Value {
        json!({"sections": [
            {"type": "header", "data": {"icon": "logos/US0378331005/v2"}},
            {"title": "Übersicht", "type": "table", "data": [
                {"title": "Gebühr", "detail": {"text": "1,00 €"}},
                {"title": "Steuer", "detail": {"text": "0,50 €"}},
                {"title": "Transaktion", "detail": {"action": {"payload": {"sections": [
                    {"type": "table", "data": extra_rows}
                ]}}}}
            ]}
        ]})
    }

    fn row(kind: &str, amount: i64) -> TradeRow {
        TradeRow {
            id: "evt-1".into(),
            date: "2026-11-05".into(),
            title: "Apple".into(),
            kind_text: kind.into(),
            event_context: kind.into(),
            amount_cents: amount,
            currency: "EUR".into(),
            icon: String::new(),
        }
    }

    #[test]
    fn parses_decimal_texts() {
        assert_eq!(parse_decimal_text("1.234,56 €"), Some(1234.56));
        assert_eq!(parse_decimal_text("0,5"), Some(0.5));
        assert_eq!(parse_decimal_text("12.5"), Some(12.5));
        assert_eq!(parse_decimal_text("1.234"), Some(1234.0));
        assert_eq!(parse_decimal_text("1,234.50"), Some(1234.5));
        assert_eq!(parse_decimal_text("−3,00 €"), Some(-3.0));
        assert_eq!(parse_decimal_text("– €"), None);
        assert_eq!(parse_decimal_text(""), None);
    }

    #[test]
    fn reads_trade_details_and_builds_the_securities_side() {
        use crate::models::SecurityDirection;
        let d = detail(json!([
            {"title": "Aktien", "detail": {"text": "2,5"}},
            {"title": "Preis", "detail": {"text": "160,00 €"}}
        ]));
        let t = trade_security(&row("kauf apple", -40_150), &d).unwrap();
        assert_eq!(
            (
                t.account_ref.as_str(),
                t.external_id.as_str(),
                t.cash_external_id.as_str()
            ),
            ("tr:depot", "evt-1", "evt-1")
        );
        let x = &t.transaction;
        assert_eq!(x.isin, "US0378331005");
        assert_eq!(x.direction, SecurityDirection::Buy);
        assert_eq!(
            (x.quantity, x.price_cents, x.value_cents),
            (2.5, 16_000, 40_000)
        );
        assert_eq!((x.fees_cents, x.taxes_cents), (100, 50));
        // without a price the value is the cash amount minus fees and taxes
        let d2 = detail(json!([{"title": "Aktien", "detail": {"text": "2,5"}}]));
        let b = trade_security(&row("kauf apple", -40_150), &d2)
            .unwrap()
            .transaction;
        assert_eq!((b.value_cents, b.price_cents), (40_000, 16_000));
        // "Verkauf" is a sell although it contains "kauf"; sells add fees back
        let s = trade_security(&row("verkaufsorder", 39_850), &d2)
            .unwrap()
            .transaction;
        assert_eq!(s.direction, SecurityDirection::Sell);
        assert_eq!(s.value_cents, 40_000);
        let div = trade_security(&row("dividende", 1_000), &detail(json!([])))
            .unwrap()
            .transaction;
        assert_eq!(div.direction, SecurityDirection::Dividend);
        // no ISIN anywhere: not a securities transaction
        assert!(trade_security(&row("kauf", -100), &json!({"sections": []})).is_none());
        assert!(trade_security(&row("zinsen", 100), &d2).is_none());
    }

    #[test]
    fn isin_comes_from_the_icon_or_the_header() {
        assert_eq!(
            isin_of("logos/IE00B4L5Y983/v2").as_deref(),
            Some("IE00B4L5Y983")
        );
        assert_eq!(isin_of("DE0007164600").as_deref(), Some("DE0007164600"));
        assert_eq!(isin_of("logos/company/v2"), None);
        assert_eq!(
            parse_trade_detail("logos/DE0007164600/v2", &json!({}))
                .isin
                .as_deref(),
            Some("DE0007164600")
        );
        let d = json!({"sections": [{"action": {"type": "instrumentDetail", "payload": "IE00B4L5Y983"}}]});
        assert_eq!(
            parse_trade_detail("", &d).isin.as_deref(),
            Some("IE00B4L5Y983")
        );
    }

    #[test]
    fn expired_session_is_named_in_the_login_prompts() {
        assert!(pin_prompt_message(true).contains("abgelaufen"));
        assert!(browser_login_message(true).contains("abgelaufen"));
        // no stored session: the usual prompts, without "expired"
        assert!(!pin_prompt_message(false).contains("abgelaufen"));
        assert!(browser_login_message(false).contains("AWS WAF"));
    }

    #[test]
    fn cookie_host_check() {
        let p = |s: &str| reqwest::Url::parse(s).unwrap();
        assert!(is_trade_republic_host(&p(
            "https://api.traderepublic.com/x"
        )));
        assert!(is_trade_republic_host(&p(
            "https://a.b.traderepublic.com/x"
        )));
        assert!(!is_trade_republic_host(&p("https://evil.com/x")));
        assert!(!is_trade_republic_host(&p(
            "https://traderepublic.com.evil.com/x"
        )));
        assert!(!is_trade_republic_host(&p(
            "https://eviltraderepublic.com/x"
        )));
        assert!(!is_trade_republic_host(&p(
            "https://bucket.s3.amazonaws.com/x?traderepublic.com"
        )));
    }

    #[test]
    fn normalises_phone_numbers() {
        assert_eq!(normalize_phone("0171 1234567"), "+491711234567");
        assert_eq!(normalize_phone("0049171/1234567"), "+491711234567");
        assert_eq!(normalize_phone("17012345678"), "+4917012345678");
        assert_eq!(normalize_phone("017012345678"), "+4917012345678");
        assert_eq!(normalize_phone("+49 170 1234 5678"), "+4917012345678");
        assert_eq!(normalize_phone("4917012345678"), "+4917012345678");
        assert_eq!(normalize_phone(""), "");
    }

    #[test]
    fn merges_set_cookie_headers() {
        let c = merge_cookies(
            "tr_refresh=old; other=1",
            &[
                "tr_session=abc; Path=/; HttpOnly; Secure".into(),
                "tr_refresh=new; Path=/".into(),
            ],
        );
        assert_eq!(c, "other=1; tr_session=abc; tr_refresh=new");
    }

    #[test]
    fn float_amounts_round_to_cents() {
        assert_eq!(amount_cents(&json!(-4.040000000000001)), -4_04);
        assert_eq!(amount_cents(&json!(-5.78)), -5_78);
        assert_eq!(amount_cents(&json!(47.32)), 4732);
    }

    // --- Depot positions

    fn position(id: &str, category: &str, quantity: f64, buy_in: Option<f64>) -> RawPosition {
        RawPosition {
            isin: id.into(),
            name: None,
            category: category.into(),
            quantity,
            average_buy_in: buy_in,
        }
    }

    fn info(type_id: &str) -> InstrumentInfo {
        InstrumentInfo {
            type_id: type_id.into(),
            ..Default::default()
        }
    }

    fn quote(last: f64, pre: Option<f64>) -> Quote {
        Quote {
            last: Some(last),
            pre,
        }
    }

    #[test]
    fn parses_compact_portfolio_by_type() {
        let v = json!({
            "categories": [
                {"type": "stocksAndETFs", "positions": [
                    {"instrumentId": "US0378331005", "name": "Apple Inc.", "netSize": "10.5", "averageBuyIn": "175.50"},
                    {"isin": "IE00B4L5Y983", "netSize": 40.0, "averageBuyIn": 82.20},
                    {"isin": "DE0007164600", "netSize": "0"}
                ]},
                {"type": "crypto", "positions": [
                    {"instrumentId": "XF000BTC0017", "name": "Bitcoin", "netSize": "0.15", "averageBuyIn": "50000.00"}
                ]},
                {"type": "bonds", "positions": []}
            ]
        });
        let positions = portfolio_positions(&v).unwrap();
        assert_eq!(positions.len(), 3); // the sold-out position is no holding
        assert_eq!(positions[0].isin, "US0378331005");
        assert_eq!(positions[0].name.as_deref(), Some("Apple Inc."));
        assert_eq!(positions[0].category, "stocksAndETFs");
        assert_eq!(positions[0].quantity, 10.5);
        assert_eq!(positions[0].average_buy_in, Some(175.50));
        assert_eq!(positions[1].isin, "IE00B4L5Y983");
        assert_eq!(positions[1].quantity, 40.0);
        assert_eq!(positions[2].category, "crypto");
    }

    #[test]
    fn portfolio_layouts_and_what_is_no_portfolio() {
        let flat = json!({"positions": [
            {"instrumentId": "DE0007164600", "netSize": "5", "averageBuyIn": "140,00"}
        ]});
        let positions = portfolio_positions(&flat).unwrap();
        assert_eq!(positions[0].quantity, 5.0);
        assert_eq!(positions[0].average_buy_in, Some(140.0));
        // an empty depot is an answer …
        assert_eq!(
            portfolio_positions(&json!({"categories": []})),
            Some(vec![])
        );
        // … an unknown layout is not: it must not empty the stored depot
        assert_eq!(portfolio_positions(&json!({"status": "ok"})), None);
        assert_eq!(portfolio_positions(&Value::Null), None);
    }

    #[test]
    fn numbers_as_text_keep_three_decimals() {
        assert_eq!(json_as_f64(&json!("40.745")), Some(40.745));
        assert_eq!(json_as_f64(&json!("18.000")), Some(18.0));
        assert_eq!(json_as_f64(&json!("73.256536")), Some(73.256536));
        assert_eq!(json_as_f64(&json!(40.745)), Some(40.745));
        assert_eq!(json_as_f64(&json!("1.234,56")), Some(1234.56));
        assert_eq!(json_as_f64(&json!("")), None);
    }

    #[test]
    fn securities_account_prefers_the_default_product() {
        let pairs = json!({"accounts": [
            {"productType": "CHILD", "securitiesAccountNumber": "222"},
            {"productType": "DEFAULT", "securitiesAccountNumber": "111"},
            {"productType": "DEFAULT"}
        ]});
        assert_eq!(securities_account_number(&pairs).as_deref(), Some("111"));
        assert_eq!(securities_account_number(&json!({"accounts": []})), None);
        assert_eq!(securities_account_number(&Value::Null), None);
    }

    #[test]
    fn holding_with_price_and_performance() {
        let mut raw = position("US0378331005", "stocksAndETFs", 10.0, Some(150.0));
        raw.name = Some("Apple".into());
        let mut inst = info("stock");
        inst.wkn = "865985".into();
        let rh = holding_of(&raw, &inst, None, quote(180.0, Some(175.0)));
        assert_eq!(rh.account_ref, "tr:depot");
        assert!(!rh.direct_crypto);
        let h = rh.holding;
        assert_eq!(h.isin, "US0378331005");
        assert_eq!(h.wkn, "865985");
        assert_eq!(h.name, "Apple");
        assert_eq!(h.purchase_price_cents, 15_000);
        assert_eq!(h.current_price_cents, 18_000);
        assert_eq!(h.value_cents, 180_000);
        assert_eq!(h.prev_day_price_cents, Some(17_500));
        assert_eq!(h.prev_day_value_cents, Some(175_000));
        assert_eq!(h.profit_loss_purchase_cents, Some(30_000));
        assert_eq!(h.profit_loss_purchase_rel, Some(20.0));
        assert_eq!(h.profit_loss_prev_day_cents, Some(5_000));
        assert_eq!(h.custody_type, "");
        assert_eq!(rh.classification.asset_class, Some(AssetClass::Equity));
        assert_eq!(
            rh.classification.instrument,
            Some(InstrumentKind::SingleSecurity)
        );
    }

    #[test]
    fn value_comes_from_the_unrounded_price() {
        // 73,256536 × 40,745 = 2.984,84 (rounding the price to 40,75 first gave 2.985,20)
        let raw = position("DE000A0F5UH1", "stocksAndETFs", 73.256536, Some(27.57));
        let h = holding_of(&raw, &info("fund"), None, quote(40.745, None)).holding;
        assert_eq!(h.current_price_cents, 4075);
        assert_eq!(h.value_cents, 298_484);
        assert_eq!(h.profit_loss_purchase_cents, Some(298_484 - 201_968));
        assert_eq!(h.prev_day_value_cents, None);
        assert_eq!(h.profit_loss_prev_day_cents, None);
    }

    #[test]
    fn bonds_are_quoted_in_percent_of_the_nominal() {
        // 1.000 nominal at 99,50 % is worth 995,00, not 99.500,00
        let raw = position("DE0001102580", "bonds", 1000.0, Some(98.5));
        let h = holding_of(&raw, &info("bond"), None, quote(99.5, Some(99.0))).holding;
        assert_eq!(h.value_cents, 99_500);
        assert_eq!(h.purchase_price_cents, 99); // 0,985 per unit, shown in cents
        assert_eq!(h.profit_loss_purchase_cents, Some(1_000));
        assert_eq!(h.prev_day_value_cents, Some(99_000));
        // a purchase price that already is per unit stays as it is
        let per_unit = position("DE0001102580", "bonds", 1000.0, Some(0.985));
        let h = holding_of(&per_unit, &info("bond"), None, quote(99.5, None)).holding;
        assert_eq!(h.profit_loss_purchase_cents, Some(1_000));
        // the category alone marks a bond when the instrument answer is missing
        let h = holding_of(&raw, &InstrumentInfo::default(), None, quote(99.5, None)).holding;
        assert_eq!(h.value_cents, 99_500);
    }

    #[test]
    fn without_a_quote_the_position_is_valued_at_cost_and_reports_no_gain() {
        let raw = position("US0378331005", "stocksAndETFs", 2.0, Some(150.0));
        let h = holding_of(&raw, &info("stock"), None, Quote::default()).holding;
        assert_eq!(h.value_cents, 30_000);
        assert_eq!(h.current_price_cents, 15_000);
        assert_eq!(h.profit_loss_purchase_cents, None);
        assert_eq!(h.profit_loss_purchase_rel, None);
        // neither quote nor purchase price: nothing to value
        let bare = position("US0378331005", "stocksAndETFs", 2.0, None);
        let h = holding_of(&bare, &info("stock"), None, Quote::default()).holding;
        assert_eq!((h.value_cents, h.purchase_price_cents), (0, 0));
    }

    #[test]
    fn name_falls_back_from_portfolio_to_instrument_to_trade_row() {
        let raw = position("US0378331005", "stocksAndETFs", 1.0, None);
        let mut inst = info("stock");
        let q = Quote::default();
        assert_eq!(
            holding_of(&raw, &inst, None, q).holding.name,
            "US0378331005"
        );
        assert_eq!(
            holding_of(&raw, &inst, Some("Apple (Kauf)"), q)
                .holding
                .name,
            "Apple (Kauf)"
        );
        inst.name = "Apple Inc.".into();
        assert_eq!(
            holding_of(&raw, &inst, Some("x"), q).holding.name,
            "Apple Inc."
        );
        inst.short_name = "Apple".into();
        assert_eq!(holding_of(&raw, &inst, Some("x"), q).holding.name, "Apple");
    }

    #[test]
    fn classification_follows_the_instrument_type_not_the_name() {
        let class = |category: &str, inst: &InstrumentInfo, name: &str| {
            let (c, crypto) = classify_tr_instrument(category, inst, name);
            (c.asset_class, c.instrument, crypto)
        };
        // "Netflix" contains "etf"
        assert_eq!(
            class("stocksAndETFs", &info("stock"), "Netflix"),
            (
                Some(AssetClass::Equity),
                Some(InstrumentKind::SingleSecurity),
                false
            )
        );
        // an ETF's asset class is its underlying class; unknown stays open for the name rules
        let mut etf = info("fund");
        etf.fund_category = "etf".into();
        assert_eq!(
            class("stocksAndETFs", &etf, "Core Euro Govt Bond"),
            (None, Some(InstrumentKind::Etf), false)
        );
        etf.underlying_class = "Bonds".into();
        assert_eq!(
            class("stocksAndETFs", &etf, "Core Euro Govt Bond"),
            (Some(AssetClass::Bond), Some(InstrumentKind::Etf), false)
        );
        // the labels Trade Republic sends
        etf.underlying_class = "Festverzinslich".into();
        assert_eq!(class("stocksAndETFs", &etf, "x").0, Some(AssetClass::Bond));
        etf.underlying_class = "Aktien".into();
        assert_eq!(
            class("stocksAndETFs", &etf, "x").0,
            Some(AssetClass::Equity)
        );
        etf.underlying_class = "equity".into();
        assert_eq!(
            class("stocksAndETFs", &etf, "MSCI World").0,
            Some(AssetClass::Equity)
        );
        assert_eq!(
            class("stocksAndETFs", &info("fund"), "Mischfonds").1,
            Some(InstrumentKind::Fund)
        );
        assert_eq!(
            class("stocksAndETFs", &info("fund"), "Physical Gold ETC"),
            (
                Some(AssetClass::Commodity),
                Some(InstrumentKind::EtcEtn),
                false
            )
        );
        assert_eq!(
            class("derivatives", &info("derivative"), "Call"),
            (None, Some(InstrumentKind::Derivative), false)
        );
        assert_eq!(
            class("bonds", &InstrumentInfo::default(), "Bund"),
            (
                Some(AssetClass::Bond),
                Some(InstrumentKind::SingleSecurity),
                false
            )
        );
        // nothing stated: nothing imported
        assert_eq!(
            class("stocksAndETFs", &InstrumentInfo::default(), "Netflix"),
            (None, None, false)
        );
    }

    #[test]
    fn direct_crypto_keeps_an_isin_and_moves_a_ticker_to_the_wkn() {
        let coin = position("BTC", "crypto", 0.5, Some(60_000.0));
        let rh = holding_of(&coin, &info("crypto"), None, quote(65_000.0, None));
        assert!(rh.direct_crypto);
        assert_eq!(rh.classification.asset_class, Some(AssetClass::Crypto));
        assert_eq!(
            rh.classification.instrument,
            Some(InstrumentKind::DirectHolding)
        );
        assert_eq!(
            (rh.holding.isin.as_str(), rh.holding.wkn.as_str()),
            ("", "BTC")
        );
        // the core recognises it as a direct holding with a stable key
        let class = crate::asset_classification::classify_asset(
            rh.holding.clone(),
            rh.classification.clone(),
            Default::default(),
            true,
        );
        assert_eq!(class.security_key.as_deref(), Some("crypto:BTC"));

        let with_isin = position("XF000BTC0017", "crypto", 0.5, None);
        let rh = holding_of(&with_isin, &info("crypto"), None, quote(65_000.0, None));
        assert_eq!(rh.holding.isin, "XF000BTC0017");
    }

    #[test]
    fn shape_shows_keys_and_types_but_no_values() {
        let v = json!({
            "categories": [{"type": "crypto", "positions": [
                {"isin": "US0378331005", "netSize": "10.5", "averageBuyIn": 175.5}
            ]}],
            "US0378331005": {"name": "Apple Inc."}
        });
        let s = shape(&v, 5);
        assert!(s.contains(
            "categories:[1x {positions:[1x {averageBuyIn:num,isin:str,netSize:str}],type:str}]"
        ));
        assert!(s.contains("<id>:{name:str}"));
        for secret in ["US0378331005", "Apple", "10.5", "175", "crypto"] {
            assert!(!s.contains(secret), "{secret} leaked into {s}");
        }
    }

    // --- Depot fetch against a scripted WebSocket

    use std::collections::VecDeque;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    enum Reply {
        Answer(Value),
        Refuse,
        /// No answer at all (the request runs into its timeout).
        Silent,
        /// The connection ends.
        Close,
    }

    struct FakeWs<F: FnMut(&Value) -> Reply + Unpin> {
        script: F,
        inbox: VecDeque<Message>,
        closed: bool,
        /// Request types in the order they were sent.
        sent: Vec<String>,
    }

    impl<F: FnMut(&Value) -> Reply + Unpin> FakeWs<F> {
        fn new(script: F) -> Self {
            Self {
                script,
                inbox: VecDeque::new(),
                closed: false,
                sent: Vec::new(),
            }
        }
    }

    impl<F: FnMut(&Value) -> Reply + Unpin> futures_util::Stream for FakeWs<F> {
        type Item = Result<Message, tokio_tungstenite::tungstenite::Error>;
        fn poll_next(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            match self.inbox.pop_front() {
                Some(msg) => Poll::Ready(Some(Ok(msg))),
                None if self.closed => Poll::Ready(None),
                // nothing will come: the request's timeout wakes the task
                None => Poll::Pending,
            }
        }
    }

    impl<F: FnMut(&Value) -> Reply + Unpin> futures_util::Sink<Message> for FakeWs<F> {
        type Error = std::io::Error;
        fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
        fn start_send(mut self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
            let text = item.to_text().unwrap_or_default().to_string();
            let mut parts = text.splitn(3, ' ');
            if parts.next() != Some("sub") {
                return Ok(());
            }
            let id = parts.next().unwrap_or_default().to_string();
            let payload: Value = serde_json::from_str(parts.next().unwrap_or("null")).unwrap();
            self.sent
                .push(payload["type"].as_str().unwrap_or_default().to_string());
            match (self.script)(&payload) {
                Reply::Answer(v) => self.inbox.push_back(Message::text(format!("{id} A {v}"))),
                Reply::Refuse => self.inbox.push_back(Message::text(format!("{id} E {{}}"))),
                Reply::Silent => {}
                Reply::Close => self.closed = true,
            }
            Ok(())
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
        fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
    }

    /// Two positions: a share with a quote and a fund.
    fn portfolio_answer() -> Value {
        json!({"categories": [{"type": "stocksAndETFs", "positions": [
            {"isin": "US0378331005", "netSize": "2", "averageBuyIn": "150.00"},
            {"isin": "IE00B4L5Y983", "netSize": "10", "averageBuyIn": "80.00"}
        ]}]})
    }

    /// A server that knows the account, the portfolio and every instrument and quote.
    fn healthy(payload: &Value) -> Reply {
        let id = payload["id"].as_str().unwrap_or_default();
        match payload["type"].as_str().unwrap_or_default() {
            "accountPairs" => Reply::Answer(json!({"accounts": [
                {"productType": "DEFAULT", "securitiesAccountNumber": "111"}
            ]})),
            "compactPortfolioByType" if payload["secAccNo"] == "111" => {
                Reply::Answer(portfolio_answer())
            }
            "instrument" if id == "US0378331005" => Reply::Answer(json!({
                "typeId": "stock", "shortName": "Apple", "wkn": "865985", "exchangeIds": ["LSX"]
            })),
            "instrument" => Reply::Answer(json!({
                "typeId": "fund", "shortName": "Core MSCI World", "exchangeIds": ["TDG"],
                "fundInfo": {"category": "etf", "underlyingClass": "equity"}
            })),
            "ticker" if id == "US0378331005.LSX" => {
                Reply::Answer(json!({"last": {"price": "180.00"}, "pre": {"price": "175.00"}}))
            }
            "ticker" if id == "IE00B4L5Y983.TDG" => {
                Reply::Answer(json!({"last": {"price": "90.555"}}))
            }
            _ => Reply::Refuse,
        }
    }

    async fn run<F: FnMut(&Value) -> Reply + Unpin>(
        script: F,
    ) -> (Option<Vec<RemoteHolding>>, Vec<String>) {
        let mut ws = FakeWs::new(script);
        let mut sub_id = 100;
        let names = std::collections::HashMap::new();
        let holdings = fetch_portfolio(&mut ws, &mut sub_id, "", &names).await;
        (holdings, ws.sent)
    }

    #[tokio::test(start_paused = true)]
    async fn depot_fetch_reads_positions_with_instrument_and_quote() {
        let (holdings, sent) = run(healthy).await;
        let holdings = holdings.unwrap();
        assert_eq!(
            sent,
            [
                "accountPairs",
                "compactPortfolioByType",
                "instrument",
                "ticker",
                "instrument",
                "ticker"
            ]
        );
        // largest value first: 10 × 90,555 = 905,55 before 2 × 180,00 = 360,00
        let fund = &holdings[0];
        assert_eq!(fund.holding.name, "Core MSCI World");
        assert_eq!(fund.holding.value_cents, 90_555);
        assert_eq!(fund.classification.instrument, Some(InstrumentKind::Etf));
        assert_eq!(fund.classification.asset_class, Some(AssetClass::Equity));
        let share = &holdings[1];
        assert_eq!(share.holding.wkn, "865985");
        assert_eq!(share.holding.value_cents, 36_000);
        assert_eq!(share.holding.profit_loss_prev_day_cents, Some(1_000));
    }

    #[tokio::test(start_paused = true)]
    async fn depot_fetch_falls_back_when_the_account_number_is_unknown_or_refused() {
        // no account pairs: the portfolio is asked without the number
        let (holdings, sent) = run(|p: &Value| match p["type"].as_str().unwrap() {
            "accountPairs" => Reply::Refuse,
            "compactPortfolioByType" if p["secAccNo"].is_null() => {
                Reply::Answer(portfolio_answer())
            }
            _ => healthy(p),
        })
        .await;
        assert_eq!(holdings.unwrap().len(), 2);
        assert_eq!(sent[..2], ["accountPairs", "compactPortfolioByType"]);

        // the current request refused twice: the old one still lists the positions
        let (holdings, sent) = run(|p: &Value| match p["type"].as_str().unwrap() {
            "compactPortfolioByType" => Reply::Refuse,
            "compactPortfolio" => Reply::Answer(json!({"positions": [
                {"instrumentId": "US0378331005", "netSize": "2", "averageBuyIn": "150.00"}
            ]})),
            _ => healthy(p),
        })
        .await;
        assert_eq!(holdings.unwrap().len(), 1);
        assert_eq!(
            sent[..4],
            [
                "accountPairs",
                "compactPortfolioByType",
                "compactPortfolioByType",
                "compactPortfolio"
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn depot_fetch_reports_an_empty_depot_only_from_the_current_request() {
        let (holdings, _) = run(|p: &Value| match p["type"].as_str().unwrap() {
            "compactPortfolioByType" => Reply::Answer(json!({"categories": []})),
            _ => healthy(p),
        })
        .await;
        assert_eq!(holdings.map(|h| h.len()), Some(0));

        // an unknown layout, then an empty answer of the old request: not a statement
        let (holdings, _) = run(|p: &Value| match p["type"].as_str().unwrap() {
            "compactPortfolioByType" => Reply::Answer(json!({"status": "ok"})),
            "compactPortfolio" => Reply::Answer(json!({"positions": []})),
            _ => healthy(p),
        })
        .await;
        assert!(holdings.is_none());

        // nothing answers at all
        let (holdings, _) = run(|p: &Value| match p["type"].as_str().unwrap() {
            "compactPortfolioByType" | "compactPortfolio" => Reply::Silent,
            _ => healthy(p),
        })
        .await;
        assert!(holdings.is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn a_silent_ticker_costs_its_timeout_and_values_the_position_at_cost() {
        let started = tokio::time::Instant::now();
        let (holdings, _) = run(|p: &Value| match p["type"].as_str().unwrap() {
            "ticker" if p["id"] == "US0378331005.LSX" => Reply::Silent,
            "instrument" if p["id"] == "IE00B4L5Y983" => Reply::Refuse,
            _ => healthy(p),
        })
        .await;
        assert_eq!(started.elapsed(), TICKER_WAIT);
        let holdings = holdings.unwrap();
        let share = holdings
            .iter()
            .find(|h| h.holding.isin == "US0378331005")
            .unwrap();
        assert_eq!(share.holding.value_cents, 30_000);
        assert_eq!(share.holding.profit_loss_purchase_cents, None);
        // the refused instrument: default exchange, no quote there, still listed
        let fund = holdings
            .iter()
            .find(|h| h.holding.isin == "IE00B4L5Y983")
            .unwrap();
        assert_eq!(fund.holding.value_cents, 80_000);
        assert_eq!(fund.classification, ClassificationValues::default());
    }

    #[tokio::test(start_paused = true)]
    async fn a_lost_connection_leaves_the_depot_untouched() {
        // at any step: no partial inventory with made-up prices
        for lost in [
            "accountPairs",
            "compactPortfolioByType",
            "instrument",
            "ticker",
        ] {
            let mut seen = 0;
            let (holdings, _) = run(|p: &Value| {
                if p["type"] == lost {
                    seen += 1;
                    // the second position's requests, where there are two
                    if seen == 2 || matches!(lost, "accountPairs" | "compactPortfolioByType") {
                        return Reply::Close;
                    }
                }
                healthy(p)
            })
            .await;
            assert!(holdings.is_none(), "connection lost during {lost}");
        }
    }

    fn trade(date: &str, id: &str) -> TradeRow {
        TradeRow {
            id: id.into(),
            date: date.into(),
            title: String::new(),
            kind_text: String::new(),
            event_context: String::new(),
            amount_cents: 0,
            currency: "EUR".into(),
            icon: String::new(),
        }
    }

    #[test]
    fn the_securities_history_state_is_kept_in_the_connection_state() {
        let mut c = TradeRepublicConnector::new();
        assert_eq!(
            c.state().unwrap().with_value(|v| v.clone())["trade_history_v3_complete"],
            json!(false)
        );
        assert!(c.session.trade_history_before.is_none());
        c.session.trade_history_before = Some("2024-05-01".into());
        c.session.trade_history_complete = true;
        assert_eq!(
            c.state().unwrap().with_value(|v| v.clone())["trade_history_v3_before"],
            json!("2024-05-01")
        );
        assert_eq!(
            c.state().unwrap().with_value(|v| v.clone())["trade_history_v3_complete"],
            json!(true)
        );
    }

    #[test]
    fn a_failed_trade_is_read_again_wherever_the_cursor_is() {
        let rows: Vec<TradeRow> = (0..3)
            .map(|i| trade("2020-01-10", &format!("old{i}")))
            .chain([trade("2026-11-10", "new")])
            .collect();
        // the history is complete: only the recent trade is selected …
        let mut plan = plan_trades(&rows, "2026-10-01", Some("2020-01-01"), true);
        assert_eq!(
            plan.selected
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            ["new"]
        );
        // … and the two that failed last time come back, once each
        let retry = vec!["old1".to_string(), "old2".to_string(), "new".to_string()];
        add_retries(&mut plan.selected, &rows, &retry);
        let ids: Vec<&str> = plan.selected.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["new", "old1", "old2"]);
        // the failed ones are part of the state, so a new session knows them
        let mut c = TradeRepublicConnector::new();
        c.session.trade_history_retry = retry;
        assert_eq!(
            c.state().unwrap().with_value(|v| v.clone())["trade_history_v3_retry"],
            json!(["old1", "old2", "new"])
        );
    }

    #[test]
    fn history_resumes_at_the_page_limit_after_finishing_the_current_date_batches() {
        let mut c = TradeRepublicConnector::new();
        c.advance_history(Some("2024-05-01".into()), false, Some("next".into()));
        assert!(c.session.transaction_history_cursor.is_none());
        assert_eq!(
            c.session.trade_history_before.as_deref(),
            Some("2024-05-01")
        );
        assert!(!c.session.trade_history_complete);

        c.advance_history(Some("2024-05-01".into()), true, Some("next".into()));
        assert_eq!(
            c.state().unwrap().with_value(|v| v.clone())["transaction_history_cursor_v1"],
            json!("next")
        );
        assert!(c.session.trade_history_before.is_none());
        assert!(!c.session.trade_history_complete);

        c.advance_history(None, true, None);
        assert!(c.session.transaction_history_cursor.is_none());
        assert!(c.session.trade_history_complete);
    }

    #[test]
    fn the_history_is_read_in_batches_newest_first() {
        // 5 recent trades and 1000 older ones, one per day, newest first
        let mut rows: Vec<TradeRow> = (0..5)
            .map(|i| trade("2026-11-10", &format!("r{i}")))
            .collect();
        let start = jiff::civil::date(2026, 1, 1);
        for i in 0..1000 {
            let d = crate::dates::iso(start - crate::dates::days(i));
            rows.push(trade(&d, &format!("h{i}")));
        }
        let since = "2026-11-01";
        let p1 = plan_trades(&rows, since, None, false);
        assert_eq!(p1.selected.len(), 5 + 400);
        assert!(!p1.complete);
        assert_eq!(p1.remaining, 600);
        let border = p1.before.clone().unwrap();
        assert_eq!(border, rows[5 + 399].date);
        let p2 = plan_trades(&rows, since, Some(&border), false);
        // the border day is read again, then the next 400
        assert!(p2.selected.iter().any(|r| r.id == "h399"));
        assert!(!p2.complete);
        let border2 = p2.before.clone().unwrap();
        let p3 = plan_trades(&rows, since, Some(&border2), false);
        assert!(p3.complete, "the rest fits into one batch");
        assert_eq!(p3.remaining, 0);
        // finished: only the recent trades from then on
        let p4 = plan_trades(&rows, since, p3.before.as_deref(), true);
        assert_eq!(p4.selected.len(), 5);
        assert!(p4.complete);
    }

    #[test]
    fn classifies_document_types_accurately() {
        let classify = |title: &str| crate::documents::type_from_title(&title.to_lowercase());

        assert_eq!(classify("Kontoauszug Q3 2026"), "kontoauszug");
        assert_eq!(classify("Depotauszug zum 30.09.2026"), "kontoauszug");
        assert_eq!(classify("Quartalsabschluss"), "kontoauszug");
        assert_eq!(classify("Jahressteuerbescheinigung 2025"), "steuer");
        assert_eq!(classify("Ertragsaufstellung 2025"), "steuer");
        assert_eq!(classify("Wertpapierabrechnung Kauf"), "abrechnung");
        assert_eq!(classify("Ausführungsbestätigung Sparplan"), "abrechnung");
        assert_eq!(classify("Zinsbeleg"), "beleg");
        assert_eq!(classify("Dividendenbeleg"), "abrechnung");
        assert_eq!(classify("Dividende Wahlweise"), "postbox");
        assert_eq!(classify("Kundenvereinbarung"), "postbox");
        // titles of real exports
        for (title, kind) in [
            ("Abrechnung Ausführung", "abrechnung"),
            ("Abrechnungsausführung", "abrechnung"),
            ("Ausführungsanzeige", "abrechnung"),
            ("Steuerbescheinigung", "steuer"),
            ("Vorabpauschale", "steuer"),
            ("Depotauszug", "kontoauszug"),
            ("Cryptoauszug", "kontoauszug"),
            ("Kosteninformation", "postbox"),
            ("Individuelle Kosteninformation", "postbox"),
            (
                "Kundeninformation Änderung PRIIPs-Basisinformationsblatt",
                "postbox",
            ),
            ("Kundenanschreiben", "postbox"),
            ("Bedingungsänderung - Ihre Zustimmung", "postbox"),
            ("Änderung der Vertragsunterlagen", "postbox"),
            ("Dokumente", "beleg"),
            ("Steuerreport", "steuer"),
            ("Ex-Post Kosteninformation", "postbox"),
            ("Löschbestätigung", "postbox"),
            ("Auftragsbestätigung", "abrechnung"),
            ("Prospekt", "postbox"),
            ("Hauptversammlung", "postbox"),
            ("Basisinformationsblatt", "postbox"),
            ("Preis- und Leistungsverzeichnis", "postbox"),
            ("Geschäftsbedingungen", "postbox"),
            ("NCP-AGB", "postbox"),
            ("Customer Agreement", "postbox"),
            ("Datenschutzhinweis", "postbox"),
            ("Hinweise zu den Handelsplätzen", "postbox"),
        ] {
            assert_eq!(classify(title), kind, "{title}");
        }
        // the generic "Dokumente" is what the timeline event says it is
        assert_eq!(
            refine_document_type("beleg", "bardividende ssp_corporate_action_cash"),
            "abrechnung"
        );
        assert_eq!(
            refine_document_type("beleg", "sparplan ausgeführt"),
            "abrechnung"
        );
        assert_eq!(refine_document_type("beleg", "steuerkorrektur"), "steuer");
        assert_eq!(refine_document_type("beleg", "zinsen"), "beleg");
        assert_eq!(refine_document_type("postbox", "kauf"), "postbox");
        assert!(is_securities_notice("dividende wahlweise"));
        assert!(!is_securities_notice("kundenvereinbarung"));
    }

    #[test]
    fn a_documents_account_follows_what_it_is_not_how_it_arrived() {
        let acc = document_account_ref;
        // confirmations and credits for securities: the depot
        for head in [
            "dividendenbeleg",
            "abrechnungsausf\u{fc}hrung",
            "abrechnung",
            "ausf\u{fc}hrungsanzeige",
            "wertpapierabrechnung sparplan",
        ] {
            assert_eq!(acc(head, "abrechnung"), Some("tr:depot"), "{head}");
        }
        // notices without a cash flow about a security: the depot as well
        assert_eq!(acc("dividende wahlweise", "postbox"), Some("tr:depot"));
        assert_eq!(acc("kapitalma\u{df}nahme", "postbox"), Some("tr:depot"));
        // statements: the account they are about
        assert_eq!(acc("kontoauszug", "kontoauszug"), Some("tr:cash"));
        assert_eq!(acc("depotauszug", "kontoauszug"), Some("tr:depot"));
        // interest and fees of the settlement account, even when typed as a settlement
        assert_eq!(acc("zinsabrechnung", "abrechnung"), Some("tr:cash"));
        assert_eq!(acc("kontogeb\u{fc}hren", "postbox"), Some("tr:cash"));
        // nothing says where tax papers and general information belong: they stay
        assert_eq!(acc("jahressteuerbescheinigung", "steuer"), None);
        assert_eq!(acc("kundenvereinbarung", "postbox"), None);
        // an older document just called "Dokumente": the type of its event decides
        assert_eq!(acc("dokumente", "abrechnung"), Some("tr:depot"));
        assert_eq!(acc("dokumente", "postbox"), None);
    }

    #[test]
    fn a_failed_history_page_keeps_the_recent_data_and_retries_its_cursor() {
        let failure = || Err::<&str, _>(CoreError::invalid("page request failed"));
        let mut pass = TimelinePass {
            recent_only: false,
            history_done: false,
            in_history: false,
        };
        assert!(pass.page(failure()).is_err());
        let mut read = vec![pass.page(Ok("recent")).unwrap().unwrap()];
        assert!(!pass.stop_at_window(false));
        read.push(pass.page(Ok("first older page")).unwrap().unwrap());
        assert!(!pass.stop_at_window(true));
        assert_eq!(pass.page(failure()).unwrap(), None);
        assert_eq!(read, ["recent", "first older page"]);

        let mut connector = TradeRepublicConnector::new();
        connector.advance_history(None, true, Some("failed-page".into()));
        assert!(!connector.session.trade_history_complete);
        assert_eq!(
            connector.session.transaction_history_cursor.as_deref(),
            Some("failed-page")
        );
        let mut recent = TimelinePass {
            recent_only: true,
            history_done: false,
            in_history: false,
        };
        assert!(recent.stop_at_window(true));
        assert!(recent.page(failure()).is_err());
    }

    /// Review of #1660: a sync of one account reads no history and resumes no cursor, so the
    /// rows `keep_only` drops afterwards are read by the next full sync; a full sync goes on.
    #[test]
    fn a_single_account_sync_leaves_the_history_where_it_is() {
        let mut session = StoredSession {
            transaction_history_cursor: Some("page-7".into()),
            ..Default::default()
        };
        assert_eq!(history_scope(&session, true), (true, None));
        assert_eq!(
            history_scope(&session, false),
            (false, Some("page-7".into()))
        );
        session.trade_history_complete = true;
        assert!(history_scope(&session, false).0);
        session.trade_history_retry = vec!["t1".into()];
        assert!(!history_scope(&session, false).0, "a retry keeps it going");
    }

    /// Review of #1660: until the Activity Log was listed to its end once, no listing stops at
    /// the sync window, whatever the trade history says.
    #[test]
    fn the_activity_log_stops_at_the_window_only_after_one_full_listing() {
        let mut session = StoredSession {
            trade_history_complete: true,
            ..Default::default()
        };
        let window = "2026-11-01";
        assert!(!activity_log_stops_at("2024-01-01", window, &session));
        session.doc_listing_complete = true;
        assert!(activity_log_stops_at("2024-01-01", window, &session));
        assert!(!activity_log_stops_at("2026-11-05", window, &session));
        assert!(!activity_log_stops_at("", window, &session));
        session.doc_listing_open = true;
        assert!(!activity_log_stops_at("2024-01-01", window, &session));
    }

    /// Review of #1660: candidates a pass did not reach get their try before the ones that failed.
    #[test]
    fn the_queue_puts_unreached_documents_before_failed_ones() {
        let candidate = |id: &str| DocumentCandidate {
            id: id.into(),
            date: "2023-05-01".into(),
            title: "Kontoauszug".into(),
            event_context: String::new(),
        };
        let queue = pending_after_pass(vec![candidate("new")], vec![candidate("broken")]);
        assert_eq!(queue, [candidate("new"), candidate("broken")]);
        // next pass: the unreached one is ahead of the one that keeps failing
        let mut pending = queue;
        let next = documents_for_fetch(Vec::new(), &mut pending, true);
        assert_eq!(next.first().map(|c| c.id.as_str()), Some("new"));
    }

    #[test]
    fn disabled_documents_keep_history_candidates_for_the_next_enabled_fetch() {
        let candidate = |id: &str| DocumentCandidate {
            id: id.into(),
            date: "2023-05-01".into(),
            title: "Sparplan".into(),
            event_context: "savings_plan".into(),
        };
        let mut connector = TradeRepublicConnector::new();
        connector.session.doc_pending.push(candidate("old"));
        let disabled = documents_for_fetch(
            vec![candidate("old"), candidate("next")],
            &mut connector.session.doc_pending,
            false,
        );
        assert!(disabled.is_empty());
        let state = connector.state().unwrap();
        let mut resumed: StoredSession = state.decode().unwrap();
        assert_eq!(resumed.doc_pending, [candidate("old"), candidate("next")]);
        let enabled = documents_for_fetch(Vec::new(), &mut resumed.doc_pending, true);
        assert_eq!(enabled, [candidate("old"), candidate("next")]);
        assert!(resumed.doc_pending.is_empty());
    }

    #[test]
    fn open_document_candidates_are_kept_in_the_state_between_syncs() {
        let mut c = TradeRepublicConnector::new();
        c.session.doc_pending = vec![DocumentCandidate {
            id: "item-1".into(),
            date: "2023-05-01".into(),
            title: "Kontoauszug".into(),
            event_context: "kontoauszug".into(),
        }];
        c.session.doc_listing_open = true;
        c.session.doc_listing_cursor = Some("after-page-100".into());
        let state = c.state().unwrap().with_value(|v| v.clone());
        assert_eq!(state["doc_listing_open_v1"], json!(true));
        assert_eq!(state["doc_listing_cursor_v1"], json!("after-page-100"));
        let back: Vec<DocumentCandidate> =
            serde_json::from_value(state["doc_pending_v1"].clone()).unwrap();
        assert_eq!(back, c.session.doc_pending);
        // the state of an older version has none: nothing pending
        let none: Vec<DocumentCandidate> =
            serde_json::from_value(json!({}).get("doc_pending_v1").cloned().unwrap_or_default())
                .unwrap_or_default();
        assert!(none.is_empty());
    }

    #[test]
    fn history_finishes_the_boundary_day_and_does_not_cap_recent_trades() {
        let mut rows: Vec<TradeRow> = (0..150)
            .map(|i| trade("2026-11-10", &format!("recent{i}")))
            .chain((0..450).map(|i| trade("2020-01-10", &format!("old{i}"))))
            .collect();
        rows.push(trade("2019-12-01", "last"));
        // a border day that fills a batch on its own takes the next day along, already in the
        // first batch: the 450 are not read a second time to reach the last trade
        let first = plan_trades(&rows, "2026-11-01", None, false);
        assert_eq!(first.selected.len(), 601);
        assert_eq!(first.remaining, 0);
        assert!(first.complete);
        // resumed on that day (a cursor of an older build), it still moves on
        let next = plan_trades(&rows, "2026-11-01", Some("2020-01-10"), false);
        assert_eq!(next.selected.len(), 601);
        assert!(next.complete);
        assert!(next.selected.iter().any(|r| r.id == "last"));
        let recent = plan_trades(&rows, "2026-11-01", None, true);
        assert_eq!(recent.selected.len(), 150);
    }

    #[test]
    fn a_documents_direction_comes_from_its_first_event_word() {
        assert_eq!(
            document_inflow(
                "Abrechnung Ausführung – Apple",
                "WERTPAPIERABRECHNUNG KAUF Apple"
            ),
            Some(false)
        );
        assert_eq!(
            document_inflow(
                "Abrechnung Ausführung – Apple",
                "Wertpapierabrechnung Verkauf Apple"
            ),
            Some(true)
        );
        assert_eq!(
            document_inflow("Wertpapierabrechnung Sparplan – JPM", ""),
            Some(false)
        );
        assert_eq!(document_inflow("Dividende – Apple", "Ertrag"), Some(true));
        // a sale whose boilerplate talks about purchases
        assert_eq!(
            document_inflow(
                "Abrechnung – Apple",
                "Verkauf Apple ... Kauf und Verkauf von Wertpapieren"
            ),
            Some(true)
        );
        assert_eq!(
            document_inflow("Kosteninformation – Apple", "Hinweise zu den Kosten"),
            None
        );
    }
}
