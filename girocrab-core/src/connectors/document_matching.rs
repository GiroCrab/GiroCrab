//! Reclassifies stored documents and links unassigned PDFs to bookings by content.
//! Bank-specific rules supply account, subject, direction and amount extraction;
//! the matching and reprocessing pipeline is shared. See `docs/dokument-zuordnung.md`.

use std::sync::LazyLock;

use regex::Regex;
use rusqlite::{params, Connection};

use crate::documents::{amounts_in, dates_in, extract_pdf_text};
use crate::error::CoreResult;
use crate::models::ConnectorKind;

/// What the matching needs to know about one bank's documents.
pub(crate) struct DocumentMatchRules {
    /// Whether the bookings of an account (its `external_ref` and `kind`: `checking`, `brokerage`, …)
    /// are the ones documents belong to (the cash or settlement account, never the depot).
    pub booking_account: fn(external_ref: &str, kind: &str) -> bool,
    /// What identifies the subject of a stored document (security, merchant) in its title and text:
    /// a name, an ISIN, a WKN. Empty = leave the document alone. One of them must appear in the
    /// counterparty or purpose of the booking; they are tried in this order.
    pub subjects: fn(title: &str, text: &str) -> Vec<String>,
    /// Whether `subjects` reads the PDF text too. `false`: the title alone decides, and a document
    /// without a subject is skipped before its PDF is read.
    pub subjects_from_text: bool,
    /// The document types that may document a single booking. Anything else (statements, tax papers,
    /// cost and other information) belongs to an account or depot, whatever amounts and securities
    /// its text names.
    pub link_types: &'static [&'static str],
    /// Only a document whose direction is known (`inflow`) is linked: a security named in a text is
    /// no proof that it confirms a trade.
    pub direction_required: bool,
    /// The bookings of several accounts are candidates (settlement accounts of one login): a document
    /// is linked only when its text names the account (IBAN or number); with several accounts and no
    /// such evidence it stays open, a nearer booking of another account is no proof.
    pub account_from_text: bool,
    /// Whether the document is a credit (`Some(true)`: a sale, a dividend) or a debit (`Some(false)`:
    /// a purchase, a savings plan), from its title and text; `None` = not known. A booking of the
    /// other sign is no candidate: a purchase must not take the credit of an equal sale.
    pub inflow: fn(title: &str, text: &str) -> Option<bool>,
    /// The type of a stored document and, for one that belongs to an account (no booking), the
    /// `external_ref` of the account it belongs to (a depot or the settlement account); `None` keeps
    /// the account. Arguments: title, current type, account level. Used when stored documents are
    /// sorted anew (`reprocess_documents`), the same rules a connector applies on import.
    pub retype:
        fn(title: &str, current: &str, account_level: bool) -> (&'static str, Option<&'static str>),
    /// How far the booking date may be from a date in the PDF.
    pub slack_days: i64,
    /// Labels (lower case) after which the PDF states the settled amount ("ausmachender betrag"):
    /// only that amount may justify a link, not a price, a fee or a tax amount of the same text.
    /// Empty: the text must contain exactly one distinct amount. With labels, a text without one is not linked.
    pub amount_labels: &'static [&'static str],
    /// Where a stored document without a booking belongs, from its text and the accounts of the
    /// connection (`reprocess_documents` fixes documents an earlier version put elsewhere). `None`:
    /// the account stays as it is.
    pub account_for: Option<fn(title: &str, text: &str, accounts: &[StoredAccount]) -> Option<i64>>,
    /// Source URL prefix of documents that have neither account nor booking and may be claimed by
    /// this bank when the vault has a single login of it (their origin is not recorded otherwise).
    /// Empty: no such documents.
    pub orphan_source_prefix: &'static str,
}

/// A stored account of a connection, as a document's text can name it.
pub(crate) struct StoredAccount {
    pub id: i64,
    pub external_ref: String,
    pub kind: String,
    pub iban: String,
    pub account_number: String,
}

/// Sources that hand out account and depot documents only (FNZ, Quirion): the
/// documents are typed by their titles and stay with the account or depot named by the source. They
/// belong to a period, not to a booking, so there is no booking account to match against.
pub(crate) const PORTAL_STATEMENTS: DocumentMatchRules = DocumentMatchRules {
    booking_account: |_, _| false,
    subjects: |_, _| Vec::new(),
    subjects_from_text: false,
    link_types: &[],
    direction_required: false,
    account_from_text: false,
    amount_labels: &[],
    account_for: None,
    orphan_source_prefix: "",
    slack_days: 0,
    inflow: |_, _| None,
    // only what was stored as a plain PostBox notice is typed anew; a type that is known stays
    retype: |title, current, _| {
        if current == "postbox" {
            (crate::documents::account_document_type(title), None)
        } else {
            (static_type(current), None)
        }
    },
};

/// The `&'static str` of a document type name (`beleg` for anything unknown).
pub(crate) fn static_type(doc_type: &str) -> &'static str {
    match doc_type {
        "abrechnung" => "abrechnung",
        "kontoauszug" => "kontoauszug",
        "steuer" => "steuer",
        "postbox" => "postbox",
        _ => "beleg",
    }
}

/// The rules of a bank; `None` where the connector has not been taught to match by content.
pub(crate) fn rules_for(kind: ConnectorKind) -> Option<DocumentMatchRules> {
    match kind {
        ConnectorKind::TradeRepublic => Some(crate::connectors::trade_republic::DOCUMENT_MATCH),
        ConnectorKind::Comdirect => Some(COMDIRECT),
        _ => None,
    }
}

/// The rules of a stored connection: like [`rules_for`], but a FinTS login at comdirect (kind
/// `Fints`, the PostBox rides along) gets the comdirect rules too.
pub(crate) fn rules_for_connection(
    conn: &Connection,
    connection_id: i64,
    kind: ConnectorKind,
) -> CoreResult<Option<DocumentMatchRules>> {
    Ok(if is_comdirect_login(conn, connection_id)? {
        Some(COMDIRECT)
    } else {
        rules_for(kind)
    })
}

/// Whether this is the only recorded login of its bank, including deleted logins. Orphan documents
/// can outlive a deleted login: deleting it does not prove that its documents belong to another one.
fn sole_login_of_the_bank(conn: &Connection, connection_id: i64) -> CoreResult<bool> {
    let mut st = conn.prepare("SELECT id, connector FROM bank_connections")?;
    let rows: Vec<(i64, String)> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let comdirect = |key: &str| {
        key == ConnectorKind::Comdirect.as_str()
            || ConnectorKind::fints_blz(key)
                .is_some_and(|blz| crate::connectors::fints_directory::COMDIRECT_BLZ.contains(&blz))
    };
    let mine: Vec<&(i64, String)> = rows.iter().filter(|(_, k)| comdirect(k)).collect();
    Ok(matches!(mine.as_slice(), [(id, _)] if *id == connection_id))
}

/// Whether the connection is a FinTS login at comdirect (its connector key carries a comdirect BLZ).
fn is_comdirect_login(conn: &Connection, connection_id: i64) -> CoreResult<bool> {
    use rusqlite::OptionalExtension;
    let key: Option<String> = conn
        .query_row(
            "SELECT connector FROM bank_connections WHERE id = ?1",
            [connection_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(key.as_deref().is_some_and(|k| {
        ConnectorKind::fints_blz(k)
            .is_some_and(|blz| crate::connectors::fints_directory::COMDIRECT_BLZ.contains(&blz))
    }))
}

/// The rules for sorting the stored documents of a connector anew: those of the matching banks and the
/// statement portals. `None`: nothing to sort for this connector.
pub(crate) fn reprocess_rules_for(kind: ConnectorKind) -> Option<DocumentMatchRules> {
    match kind {
        ConnectorKind::Fnz | ConnectorKind::Quirion => Some(PORTAL_STATEMENTS),
        other => rules_for(other),
    }
}

/// [`reprocess_rules_for`] for a stored connection (see [`rules_for_connection`]).
pub(crate) fn reprocess_rules_for_connection(
    conn: &Connection,
    connection_id: i64,
    kind: ConnectorKind,
) -> CoreResult<Option<DocumentMatchRules>> {
    Ok(if is_comdirect_login(conn, connection_id)? {
        Some(COMDIRECT)
    } else {
        reprocess_rules_for(kind)
    })
}

/// comdirect (REST PostBox and FinTS login): documents are typed by their PostBox names like the other
/// portals and stay with the account or depot `comdirect_postbox::assign_account` found. A trade
/// confirmation or a dividend credit belongs to the depot and documents a booking on the settlement
/// account (the Girokonto): it is linked when amount, a date, the direction and an ISIN, a WKN or the
/// security's name agree, and left alone when that is ambiguous. Statements (Finanzreport), tax papers and
/// notices belong to a period: no single booking.
pub(crate) const COMDIRECT: DocumentMatchRules = DocumentMatchRules {
    booking_account: |_, kind| kind == "checking",
    subjects: comdirect_subjects,
    subjects_from_text: true,
    link_types: &["abrechnung"],
    direction_required: true,
    account_from_text: true,
    amount_labels: &[
        "zu ihren lasten vor steuern",
        "zu ihren gunsten vor steuern",
        "ausmachender betrag",
    ],
    account_for: Some(crate::connectors::comdirect_postbox::account_for_stored),
    orphan_source_prefix: "comdirect:postbox:",
    slack_days: 5,
    inflow: |title, text| inflow_in(title, text, 1500),
    retype: PORTAL_STATEMENTS.retype,
};

/// ISINs of the text (the first ones), then the WKN and the security's name in brackets from the title
/// (`"… St. WKN 847101 (COMINVEST FONDAK P) vom …"`).
fn comdirect_subjects(title: &str, text: &str) -> Vec<String> {
    static ISIN: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\b[A-Z]{2}[A-Z0-9]{9}[0-9]\b").unwrap());
    static WKN: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)\bWKN[ _:]*([A-Z0-9]{6})\b").unwrap());
    static NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\(([^()]{3,})\)").unwrap());
    let mut out: Vec<String> = Vec::new();
    let mut add = |s: &str| {
        if !out.iter().any(|o| o == s) {
            out.push(s.to_string());
        }
    };
    for m in ISIN.find_iter(text).take(3) {
        add(m.as_str());
    }
    if let Some(c) = WKN.captures(title) {
        add(&c[1]);
    }
    if let Some(c) = NAME.captures(title) {
        add(c[1].trim());
    }
    out
}

/// The direction of a document (`Some(true)` a credit: a sale, a dividend, interest; `Some(false)` a
/// debit: a purchase, a savings plan; `None` no event word), from the first event word in its title and
/// the first `text_chars` characters of its text. Boilerplate further down ("Kauf und Verkauf …") is not
/// read, so how far the head reaches is the bank's call. Shared by Trade Republic and comdirect.
pub(crate) fn inflow_in(title: &str, text: &str, text_chars: usize) -> Option<bool> {
    let head: String = title
        .chars()
        .chain(" ".chars())
        .chain(text.chars().take(text_chars))
        .collect::<String>()
        .to_lowercase();
    let mut first: Option<(usize, bool)> = None;
    let mut note = |at: usize, credit: bool| {
        if first.is_none_or(|(p, _)| at < p) {
            first = Some((at, credit));
        }
    };
    for (word, credit) in [
        ("verkauf", true),
        ("dividende", true),
        ("ertragsgutschrift", true),
        ("ausschüttung", true),
        ("zinsen", true),
        ("sparplan", false),
    ] {
        if let Some(at) = head.find(word) {
            note(at, credit);
        }
    }
    // "kauf" that is not the end of "verkauf"
    let mut from = 0;
    while let Some(at) = head[from..].find("kauf").map(|i| i + from) {
        if !head[..at].ends_with("ver") {
            note(at, false);
            break;
        }
        from = at + 4;
    }
    first.map(|(_, credit)| credit)
}

/// Reapplies the bank's type and document-account rules, then links unassigned documents by content.
/// Preserves manual account choices and existing booking links. Returns (changed, newly linked).
pub(crate) fn reprocess_documents(
    conn: &Connection,
    connection_id: i64,
    rules: &DocumentMatchRules,
) -> CoreResult<(u32, u32)> {
    use rusqlite::OptionalExtension;
    let account = |external_ref: &str| -> CoreResult<Option<i64>> {
        Ok(conn
            .query_row(
                "SELECT id FROM accounts WHERE connection_id = ?1 AND external_ref = ?2
                  AND deleted_ms IS NULL",
                params![connection_id, external_ref],
                |r| r.get(0),
            )
            .optional()?)
    };
    // documents without an account and booking whose source says they came from this bank: only
    // when the vault has a single login of it, otherwise their origin is not certain
    let claim_orphans =
        !rules.orphan_source_prefix.is_empty() && sole_login_of_the_bank(conn, connection_id)?;
    let orphan_prefix = if claim_orphans {
        format!("{}%", rules.orphan_source_prefix)
    } else {
        String::new()
    };
    let docs: Vec<(i64, String, String, Option<i64>, Option<i64>, bool)> = {
        let mut st = conn.prepare(
            "SELECT d.id, d.title, d.doc_type, d.booking_id, d.account_id, d.account_manual != 0 FROM booking_documents d
              WHERE d.account_id IN (SELECT id FROM accounts WHERE connection_id = ?1)
                 OR d.booking_id IN (SELECT t.id FROM transactions t JOIN accounts a ON a.id = t.account_id
                                      WHERE a.connection_id = ?1)
                 OR (?2 != '' AND d.account_id IS NULL AND d.booking_id IS NULL
                     AND d.source_url LIKE ?2)",
        )?;
        let rows = st.query_map(params![connection_id, orphan_prefix], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })?;
        rows.collect::<Result<_, _>>()?
    };
    // the accounts of the connection, to name by text; the first Girokonto is where an earlier
    // version put every document without a hint
    let stored: Vec<StoredAccount> = {
        let mut st = conn.prepare(
            "SELECT id, external_ref, kind, iban, account_number FROM accounts
              WHERE connection_id = ?1 AND deleted_ms IS NULL ORDER BY id",
        )?;
        let rows = st.query_map([connection_id], |r| {
            Ok(StoredAccount {
                id: r.get(0)?,
                external_ref: r.get(1)?,
                kind: r.get(2)?,
                iban: r.get(3)?,
                account_number: r.get(4)?,
            })
        })?;
        rows.collect::<Result<_, _>>()?
    };
    let first_girokonto = stored.iter().find(|a| a.kind == "checking").map(|a| a.id);
    let mut changed = 0u32;
    for (id, title, doc_type, booking_id, account_id, account_manual) in docs {
        let account_level = booking_id.is_none();
        let (new_type, target_ref) = (rules.retype)(&title, &doc_type, account_level);
        // The account follows what the document is, linked to a booking or not (a securities
        // confirmation belongs to the depot and documents a booking of the settlement account).
        // One the user chose stays, and so does one that sits on an account of another login.
        let ours = account_id.is_none_or(|a| stored.iter().any(|s| s.id == a));
        let mut target = match target_ref.filter(|_| !account_manual && ours) {
            Some(external_ref) => account(external_ref)?.or(account_id),
            None => account_id,
        };
        // a document that sits nowhere or on the old fallback account goes where its text points;
        // one the user put elsewhere stays
        if let (Some(account_for), true) = (rules.account_for, account_level && !account_manual) {
            if account_id.is_none() || account_id == first_girokonto {
                let bytes: Vec<u8> = conn.query_row(
                    "SELECT pdf_bytes FROM booking_documents WHERE id = ?1",
                    [id],
                    |r| r.get(0),
                )?;
                if let Some(found) = account_for(&title, &extract_pdf_text(&bytes), &stored) {
                    target = Some(found);
                }
            }
        }
        if new_type != doc_type || target != account_id {
            conn.execute(
                "UPDATE booking_documents SET doc_type = ?1, account_id = ?2 WHERE id = ?3",
                params![new_type, target, id],
            )?;
            changed += 1;
        }
    }
    let linked = link_unlinked_by_content(conn, connection_id, rules, None)?;
    Ok((changed, linked))
}

/// Letters and digits only, lower case: "JPM USD EM Bond (Dist)" and "jpm usd em bond dist" agree.
fn squash(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// A booking a stored document may belong to: (id, ISO date, cents, counterparty).
pub(crate) type MatchBooking = (i64, String, i64, String);

/// The one booking a document belongs to by its content: the amount and a date of the text agree
/// (within `slack_days`), the booking's counterparty carries the subject and its sign fits the
/// direction of the document (`inflow`, when known). Several candidates: only the one with the
/// nearest date, and only if that is unambiguous.
#[cfg(test)]
pub(crate) fn match_by_content(
    text: &str,
    subject: &str,
    bookings: &[MatchBooking],
    slack_days: i64,
    inflow: Option<bool>,
) -> Option<i64> {
    match_by_amounts(
        &amounts_in(text),
        text,
        subject,
        bookings,
        slack_days,
        inflow,
    )
}

/// [`match_by_content`] with the amounts that count given (see `DocumentMatchRules::amount_labels`).
pub(crate) fn match_by_amounts(
    amounts: &[i64],
    text: &str,
    subject: &str,
    bookings: &[MatchBooking],
    slack_days: i64,
    inflow: Option<bool>,
) -> Option<i64> {
    let subject = squash(subject);
    if subject.len() < 3 {
        return None;
    }
    let days: Vec<jiff::civil::Date> = dates_in(text)
        .iter()
        .filter_map(|d| jiff::civil::Date::strptime("%Y-%m-%d", d).ok())
        .collect();
    let mut best: Vec<(i64, i64)> = Vec::new();
    for (id, date, cents, counterparty) in bookings {
        if !amounts.contains(&cents.abs()) || !squash(counterparty).contains(&subject) {
            continue;
        }
        if inflow.is_some_and(|credit| credit != (*cents > 0)) {
            continue;
        }
        let Ok(date) = jiff::civil::Date::strptime("%Y-%m-%d", date) else {
            continue;
        };
        if let Some(distance) = days
            .iter()
            .map(|d| crate::dates::days_between(*d, date).abs())
            .min()
        {
            if distance <= slack_days {
                best.push((distance, *id));
            }
        }
    }
    best.sort();
    match best.as_slice() {
        [(_, id)] => Some(*id),
        [(a, id), (b, _), ..] if a < b => Some(*id),
        _ => None,
    }
}

/// The amount a text states after one of the labels: the first amount of the rest of the label's
/// line or, when the label heads a column (`IBAN  Valuta  Zu Ihren Lasten vor Steuern` and the
/// values below), of the next line. Dates are no amounts ("19.07.2023" holds a "19.07"). The settled
/// amount of a trade confirmation, not the price, a fee or a tax amount.
fn settled_amounts(text: &str, labels: &[&str]) -> Vec<i64> {
    static DATE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\b\d{1,2}\.\d{1,2}\.(?:\d{4}|\d{2})\b").unwrap());
    // ASCII lower case keeps the byte positions of the original
    let lower = text.to_ascii_lowercase();
    let mut out: Vec<i64> = Vec::new();
    for label in labels {
        let mut from = 0;
        while let Some(at) = lower[from..].find(label).map(|i| i + from) {
            let first_amount =
                |line: &str| amounts_in(&DATE.replace_all(line, " ")).first().copied();
            let mut lines = text[at + label.len()..].lines();
            let own = lines.next().unwrap_or("");
            let found = first_amount(own).or_else(|| lines.next().and_then(first_amount));
            if let Some(amount) = found {
                if !out.contains(&amount) {
                    out.push(amount);
                }
            }
            from = at + label.len();
        }
    }
    out
}

/// A single money amount is usable without a settlement label. Dates contain decimal-looking
/// fragments too ("04.12.2025"), so remove them before deciding whether the amount is ambiguous.
fn unambiguous_amounts(text: &str) -> Vec<i64> {
    static DATE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\b(?:\d{1,2}\.\d{1,2}\.(?:\d{4}|\d{2})|20\d{2}-\d{2}-\d{2})\b").unwrap()
    });
    let amounts = amounts_in(&DATE.replace_all(text, " "));
    if amounts.len() == 1 {
        amounts
    } else {
        Vec::new()
    }
}

/// A booking a document may belong to, with the currency and account it sits on.
struct Candidate {
    booking: MatchBooking,
    currency: String,
    account: i64,
}

/// The settlement accounts (id, IBAN, account number) whose IBAN or number the text names. An IBAN
/// is the stronger hint: where one is named, numbers are not looked at.
fn accounts_named_in(text: &str, accounts: &[(i64, String, String)]) -> Vec<i64> {
    let compact: String = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_uppercase();
    let by_iban: Vec<i64> = accounts
        .iter()
        .filter(|(_, iban, _)| {
            let iban: String = iban.chars().filter(|c| !c.is_whitespace()).collect();
            iban.len() >= 15 && compact.contains(&iban.to_uppercase())
        })
        .map(|(id, _, _)| *id)
        .collect();
    if !by_iban.is_empty() {
        return by_iban;
    }
    let numbers: Vec<&str> = text
        .split(|c: char| !c.is_ascii_digit())
        .map(|n| n.trim_start_matches('0'))
        .filter(|n| !n.is_empty())
        .collect();
    accounts
        .iter()
        .filter(|(_, _, number)| {
            let digits: String = number.chars().filter(char::is_ascii_digit).collect();
            let digits = digits.trim_start_matches('0');
            digits.len() >= 6 && numbers.contains(&digits)
        })
        .map(|(id, _, _)| *id)
        .collect()
}

const CURRENCIES: &str = "EUR|USD|GBP|CHF|JPY|CAD|AUD|SEK|NOK|DKK|PLN|CZK|HUF|\u{20ac}|\\$";

/// The amounts of a text together with the currency written next to them ("25,00 EUR", "EUR 25,00",
/// "25,00- EUR", "25,00 €"): (cents, ISO code). An amount without a currency beside it is not listed.
fn amounts_with_currency(text: &str) -> Vec<(i64, String)> {
    const AMOUNT: &str = r"\d{1,3}(?:\.\d{3})+,\d{2}|\d+,\d{2}|\d+\.\d{2}";
    static AFTER: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(&format!(r"(?:^|[^\d,.])({AMOUNT})-?\s*({CURRENCIES})")).unwrap()
    });
    static BEFORE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(&format!(r"({CURRENCIES})\s*-?({AMOUNT})")).unwrap());
    let code = |c: &str| match c {
        "\u{20ac}" => "EUR".to_string(),
        "$" => "USD".to_string(),
        other => other.to_string(),
    };
    let mut out: Vec<(i64, String)> = Vec::new();
    let mut add = |amount: &str, currency: &str, end: usize| {
        // a following digit means a longer number ("0,1361 USD" is no 0,13)
        if text[end..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit())
        {
            return;
        }
        if let Ok(cents) = crate::money::parse_cents(amount) {
            let pair = (cents.abs(), code(currency));
            if cents != 0 && !out.contains(&pair) {
                out.push(pair);
            }
        }
    };
    for c in AFTER.captures_iter(text) {
        add(&c[1], &c[2], c.get(1).map_or(0, |m| m.end()));
    }
    for c in BEFORE.captures_iter(text) {
        add(&c[2], &c[1], c.get(2).map_or(0, |m| m.end()));
    }
    out
}

/// Whether a booking of `cents` in `currency` fits the amounts a document states: where the text
/// writes that amount next to a currency, it must be this one; an amount without any currency
/// beside it, or a booking without a currency, is not held against it.
fn currency_fits(bound: &[(i64, String)], cents: i64, currency: &str) -> bool {
    if currency.is_empty() {
        return true;
    }
    let mut stated = bound.iter().filter(|(c, _)| *c == cents.abs()).peekable();
    stated.peek().is_none() || stated.any(|(_, cur)| cur.eq_ignore_ascii_case(currency))
}

/// Links the documents of a connection that have no booking, by their content. `window`: only
/// documents and bookings dated inside it (plus a margin), for syncs; `None` = all. Returns how
/// many documents were linked.
pub(crate) fn link_unlinked_by_content(
    conn: &Connection,
    connection_id: i64,
    rules: &DocumentMatchRules,
    window: Option<(&str, &str)>,
) -> CoreResult<u32> {
    let widen = |d: &str, back: bool| -> Option<String> {
        let d = jiff::civil::Date::strptime("%Y-%m-%d", d).ok()?;
        crate::dates::add_days(d, if back { -10 } else { 10 }).map(crate::dates::iso)
    };
    let (from, to) = match window {
        Some((a, b)) => (
            widen(a, true).unwrap_or_default(),
            widen(b, false).unwrap_or_else(|| "9999-12-31".into()),
        ),
        None => (String::new(), "9999-12-31".into()),
    };
    // counterparty and purpose both carry the subject of a trade
    let bookings: Vec<Candidate> = {
        let mut st = conn.prepare(
            "SELECT t.id, t.booking_date, t.amount_cents, t.counterparty || ' ' || t.purpose,
                    a.external_ref, a.kind, t.currency, a.id
               FROM transactions t JOIN accounts a ON a.id = t.account_id
              WHERE a.connection_id = ?1 AND t.deleted_ms IS NULL AND a.deleted_ms IS NULL
                AND t.booking_date >= ?2 AND t.booking_date <= ?3",
        )?;
        let rows = st.query_map(params![connection_id, from, to], |r| {
            Ok((
                (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?),
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, i64>(7)?,
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|(_, external_ref, kind, _, _)| (rules.booking_account)(external_ref, kind))
            .map(|(booking, _, _, currency, account)| Candidate {
                booking,
                currency,
                account,
            })
            .collect()
    };
    if bookings.is_empty() {
        crate::depot_debug::log(
            "document matching: no booking of a matching account in the window",
        );
        return Ok(0);
    }
    // the accounts bookings may sit on, with what a document text can name them by
    let settlement: Vec<(i64, String, String)> = {
        let mut st = conn.prepare(
            "SELECT id, external_ref, kind, iban, account_number FROM accounts
              WHERE connection_id = ?1 AND deleted_ms IS NULL",
        )?;
        let rows = st.query_map([connection_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|(_, external_ref, kind, _, _)| (rules.booking_account)(external_ref, kind))
            .map(|(id, _, _, iban, number)| (id, iban, number))
            .collect()
    };
    let docs: Vec<(i64, String, String)> = {
        let mut st = conn.prepare(
            "SELECT d.id, d.title, d.doc_type FROM booking_documents d
              WHERE d.booking_id IS NULL
                AND d.account_id IN (SELECT id FROM accounts WHERE connection_id = ?1)
                AND d.doc_date >= ?2 AND d.doc_date <= ?3",
        )?;
        let rows = st.query_map(params![connection_id, from, to], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
        rows.collect::<Result<_, _>>()?
    };
    let mut linked = 0;
    // why documents stay open, as counts only (`GIROCRAB_DEPOT_DEBUG=1`; never a title, a number or a name)
    let mut why = [0u32; 9];
    let docs_seen = docs.len();
    for (id, title, doc_type) in docs {
        if !rules.link_types.contains(&doc_type.as_str()) {
            why[0] += 1;
            continue;
        }
        if !rules.subjects_from_text && (rules.subjects)(&title, "").is_empty() {
            why[1] += 1;
            continue;
        }
        let bytes: Vec<u8> = conn.query_row(
            "SELECT pdf_bytes FROM booking_documents WHERE id = ?1",
            [id],
            |r| r.get(0),
        )?;
        let text = extract_pdf_text(&bytes);
        if text.trim().is_empty() {
            why[2] += 1;
        }
        let subjects = (rules.subjects)(&title, &text);
        if subjects.is_empty() {
            why[3] += 1;
            continue;
        }
        let inflow = (rules.inflow)(&title, &text);
        if rules.direction_required && inflow.is_none() {
            why[4] += 1;
            continue;
        }
        // the accounts the text names; several candidate accounts and none named: no evidence
        let named: Option<Vec<i64>> = if rules.account_from_text {
            let named = accounts_named_in(&text, &settlement);
            if named.is_empty() && settlement.len() > 1 {
                why[5] += 1;
                continue;
            }
            (!named.is_empty()).then_some(named)
        } else {
            None
        };
        // only bookings whose amount the document states in the booking's currency
        let bound = amounts_with_currency(&text);
        let candidates: Vec<MatchBooking> = bookings
            .iter()
            .filter(|c| named.as_ref().is_none_or(|n| n.contains(&c.account)))
            .filter(|c| currency_fits(&bound, c.booking.2, &c.currency))
            .map(|c| c.booking.clone())
            .collect();
        // the amounts that may justify a link: the settled amount where the bank's documents have one
        let amounts = if rules.amount_labels.is_empty() {
            // Without a bank-specific settlement field, a price, fee or gross amount cannot be
            // distinguished from the actual cash movement. Only an unambiguous amount is safe.
            unambiguous_amounts(&text)
        } else {
            settled_amounts(&text, rules.amount_labels)
        };
        if amounts.is_empty() {
            why[6] += 1;
            continue;
        }
        let found = subjects.iter().find_map(|subject| {
            match_by_amounts(
                &amounts,
                &text,
                subject,
                &candidates,
                rules.slack_days,
                inflow,
            )
        });
        if found.is_none() {
            why[if candidates.is_empty() { 7 } else { 8 }] += 1;
        }
        if let Some(booking) = found {
            conn.execute(
                "UPDATE booking_documents SET booking_id = ?1 WHERE id = ?2 AND booking_id IS NULL",
                params![booking, id],
            )?;
            linked += 1;
        }
    }
    crate::depot_debug::log(&format!(
        "document matching: {docs_seen} documents without booking, {linked} linked; open because: \
         type not eligible {}, no subject in title {}, no readable text {} (counted apart), \
         no subject at all {}, direction unknown {}, no account named {}, no settled amount {}, \
         no booking candidate {}, no unique match {}",
        why[0], why[1], why[2], why[3], why[4], why[5], why[6], why[7], why[8]
    ));
    Ok(linked)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_do_not_make_an_unstructured_amount_ambiguous() {
        assert_eq!(
            unambiguous_amounts("25,00 EUR am 04.12.2025 und 2025-12-04"),
            [2500]
        );
        assert!(unambiguous_amounts("Kurs 100,00 EUR, Gesamt 500,00 EUR am 04.12.2025").is_empty());
    }

    #[test]
    fn a_document_is_matched_to_its_booking_by_amount_date_and_name() {
        let text =
            "WERTPAPIERABRECHNUNG SPARPLAN JPM USD EM Bond USD (Dist) 0,311177 Stk. 80,34 EUR \
                    25,00 EUR BUCHUNG 2025-12-04 -25,00 EUR 02.12.2025";
        let b = |id, date: &str, cents, cp: &str| (id, date.to_string(), cents, cp.to_string());
        let bookings = vec![
            b(1, "2025-12-04", -25_00, "JPM USD EM Bond USD (Dist)"),
            b(2, "2025-12-04", -25_00, "Daikin Industries"),
            b(3, "2025-12-04", -26_00, "JPM USD EM Bond USD (Dist)"),
            b(4, "2025-09-04", -25_00, "JPM USD EM Bond USD (Dist)"),
        ];
        assert_eq!(
            match_by_content(text, "JPM USD EM Bond USD (Dist)", &bookings, 5, None),
            Some(1)
        );
        // another security, other amount, too far away: nothing
        assert_eq!(match_by_content(text, "Intel", &bookings, 5, None), None);
        assert_eq!(
            match_by_content(text, "JPM USD EM Bond USD (Dist)", &bookings[2..], 5, None),
            None
        );
        // two equally near bookings are ambiguous
        let twice = vec![
            b(5, "2025-12-04", -25_00, "JPM USD EM Bond USD (Dist)"),
            b(6, "2025-12-04", -25_00, "JPM USD EM Bond USD (Dist)"),
        ];
        assert_eq!(
            match_by_content(text, "JPM USD EM Bond USD (Dist)", &twice, 5, None),
            None
        );
    }

    #[test]
    fn the_direction_comes_from_the_first_event_word_for_every_bank() {
        // comdirect: "Wertpapierkauf" deep in the head, a sale, a dividend credit
        let boilerplate =
            "Ihren Auftrag haben wir gemaess unseren Geschaeftsbedingungen ausgefuehrt. ".repeat(6);
        let kauf = format!("{boilerplate} GESCHAEFTSABRECHNUNG Wertpapierkauf");
        assert_eq!(inflow_in("Abrechnung", &kauf, 1500), Some(false));
        // the head Trade Republic reads (400 characters) does not reach that far
        assert_eq!(inflow_in("Abrechnung", &kauf, 400), None);
        let verkauf = format!("{boilerplate} GESCHAEFTSABRECHNUNG Wertpapierverkauf");
        assert_eq!(inflow_in("Abrechnung", &verkauf, 1500), Some(true));
        assert_eq!(
            inflow_in("Dividendengutschrift 10 St.", "", 1500),
            Some(true)
        );
        assert_eq!(inflow_in("Ertragsgutschrift 10 St.", "", 1500), Some(true));
        assert_eq!(inflow_in("Kosteninformation", "", 1500), None);
    }

    #[test]
    fn comdirect_names_the_security_by_isin_wkn_and_name() {
        let subjects = comdirect_subjects(
            "Wertpapierabrechnung Kauf 12,5 St. WKN A1JX52 (VANGUARD TEST FONDS) vom 04.12.2025",
            "Wertpapier ISIN IE00B3RBWM25 und noch IE00BK5BQT80",
        );
        assert_eq!(
            subjects,
            [
                "IE00B3RBWM25",
                "IE00BK5BQT80",
                "A1JX52",
                "VANGUARD TEST FONDS"
            ]
        );
        // a title without WKN and brackets and a text without ISIN: no subject, the document is left alone
        assert!(comdirect_subjects("Finanzreport Nr. 12", "Depot 1 Girokonto 2").is_empty());
    }

    #[test]
    fn comdirect_matches_against_the_settlement_account_only() {
        assert!((COMDIRECT.booking_account)(
            "DE02200411550123456700",
            "checking"
        ));
        assert!(!(COMDIRECT.booking_account)("123456700", "brokerage"));
        assert!(!(COMDIRECT.booking_account)(
            "4871999999990001",
            "credit_card"
        ));
        // only trade confirmations may document a booking: no cost information, statement or tax paper
        assert_eq!(COMDIRECT.link_types, ["abrechnung"]);
        const { assert!(COMDIRECT.direction_required && COMDIRECT.account_from_text) };
        // trade documents are typed by their names, a known type stays
        assert_eq!(
            (COMDIRECT.retype)("Finanzreport Nr. 1", "postbox", true).0,
            "kontoauszug"
        );
        assert_eq!(
            (COMDIRECT.retype)("Finanzreport Nr. 1", "steuer", true).0,
            "steuer"
        );
    }

    #[test]
    fn an_amount_is_tied_to_the_currency_written_next_to_it() {
        let bound = amounts_with_currency(
            "JPM USD EM Bond Kurswert 100,00 USD Wechselkurs 1,0870 Ausmachender Betrag EUR 92,00 \
             Ausschuettung USD 0,1361 pro Anteil Gebuehr 1,50- \u{20ac}",
        );
        assert!(bound.contains(&(100_00, "USD".to_string())));
        assert!(bound.contains(&(92_00, "EUR".to_string())));
        assert!(bound.contains(&(1_50, "EUR".to_string())));
        // "0,1361 USD" is no amount of 0,13
        assert!(!bound.iter().any(|(c, _)| *c == 13));
        // the document names 92,00 only in EUR: a USD booking of 92,00 is no candidate
        assert!(currency_fits(&bound, -92_00, "EUR"));
        assert!(!currency_fits(&bound, -92_00, "USD"));
        assert!(currency_fits(&bound, -100_00, "USD"));
        // an amount the text states without a currency, or a booking without one, is not held against it
        assert!(currency_fits(&bound, -7_77, "USD"));
        assert!(currency_fits(&bound, -92_00, ""));
        // "JPM USD" in the name does not make a 25,00 EUR amount a USD amount
        let jpm = amounts_with_currency("JPM USD EM Bond 25,00 EUR");
        assert!(!currency_fits(&jpm, -25_00, "USD"));
        assert!(currency_fits(&jpm, -25_00, "EUR"));
    }

    #[test]
    fn only_the_amount_after_the_label_is_the_settled_one() {
        let text = "Kurs 100,00 EUR\nProvision 1,50 EUR\nAusmachender Betrag  500,00- EUR\nAUSMACHENDER BETRAG 7,00 EUR";
        assert_eq!(
            settled_amounts(text, &["ausmachender betrag"]),
            [500_00, 7_00]
        );
        assert!(settled_amounts("Kurs 100,00 EUR", &["ausmachender betrag"]).is_empty());
        assert!(settled_amounts("", &["ausmachender betrag"]).is_empty());
        // comdirect: the label heads a column and the values stand below; a date is no amount at the line's end
        let trade = "Kurswert : EUR 24,57\nProvision : EUR 0,37\nIBAN Valuta Zu Ihren Lasten vor Steuern\nDE00 0000 0000 0000 0000 00 EUR 19.07.2023 EUR 24,94\nVerwahrungs-Art: X";
        assert_eq!(settled_amounts(trade, COMDIRECT.amount_labels), [24_94]);
        let dividend = "Bruttobetrag: USD 6,09\nzum Devisenkurs: EUR/USD 1,058500 EUR 5,75\nVerrechnung \u{fc}ber Konto (IBAN) Valuta Zu Ihren Gunsten vor Steuern\nDE00 0000 0000 0000 0000 00 EUR 29.09.2023 EUR 5,75";
        assert_eq!(settled_amounts(dividend, COMDIRECT.amount_labels), [5_75]);
        // a price is no candidate for a booking when the settled amount is known
        let bookings = vec![(1, "2025-12-04".to_string(), -100_00, "ETF ISIN".to_string())];
        assert_eq!(
            match_by_amounts(&[500_00], "04.12.2025", "ISIN", &bookings, 5, Some(false)),
            None
        );
        assert_eq!(
            match_by_amounts(&[100_00], "04.12.2025", "ISIN", &bookings, 5, Some(false)),
            Some(1)
        );
    }

    #[test]
    fn a_text_names_the_settlement_account_by_iban_before_number() {
        let accounts = vec![
            (
                1,
                "DE02200411550123456700".to_string(),
                "123456700".to_string(),
            ),
            (
                2,
                "DE75200411550765432100".to_string(),
                "765432100".to_string(),
            ),
        ];
        assert_eq!(
            accounts_named_in("Verrechnungskonto DE75 2004 1155 0765 4321 00", &accounts),
            [2]
        );
        // the IBAN wins over a number that belongs to another account
        assert_eq!(
            accounts_named_in(
                "IBAN DE75 2004 1155 0765 4321 00 Depot 123456700",
                &accounts
            ),
            [2]
        );
        assert_eq!(accounts_named_in("Depotnummer 123456700", &accounts), [1]);
        assert!(accounts_named_in("nichts", &accounts).is_empty());
        // part of a longer number is no hit
        assert!(accounts_named_in("Vorgang 91234567001", &accounts).is_empty());
    }

    #[test]
    fn a_purchase_does_not_take_the_credit_of_an_equal_sale() {
        let text = "WERTPAPIERABRECHNUNG KAUF JPM USD EM Bond USD (Dist) 25,00 EUR 02.12.2025";
        let b = |id, date: &str, cents, cp: &str| (id, date.to_string(), cents, cp.to_string());
        let bookings = vec![
            // the sale's credit is nearer in time, the purchase's debit two days off
            b(1, "2025-12-02", 25_00, "JPM USD EM Bond USD (Dist)"),
            b(2, "2025-12-04", -25_00, "JPM USD EM Bond USD (Dist)"),
        ];
        let subject = "JPM USD EM Bond USD (Dist)";
        // without the direction the nearer credit would win
        assert_eq!(match_by_content(text, subject, &bookings, 5, None), Some(1));
        assert_eq!(
            match_by_content(text, subject, &bookings, 5, Some(false)),
            Some(2)
        );
        assert_eq!(
            match_by_content(text, subject, &bookings[..1], 5, Some(false)),
            None
        );
        assert_eq!(
            match_by_content(text, subject, &bookings[..1], 5, Some(true)),
            Some(1)
        );
    }
}
