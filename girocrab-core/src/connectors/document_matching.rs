//! Links bank documents to bookings by content using bank-specific rules.

use std::collections::{BTreeSet, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use rusqlite::{Connection, OptionalExtension, params};

use crate::documents::{amount_occurrences, amounts_in, dates_in, extract_pdf_text};
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
    /// is linked only when its text identifies one account (IBAN or number). A closer booking
    /// cannot resolve missing or ambiguous account evidence.
    pub account_from_text: bool,
    /// Whether the document is a credit (`Some(true)`: a sale, a dividend) or a debit (`Some(false)`:
    /// a purchase, a savings plan), from its title and text; `None` = not known. A booking of the
    /// other sign is no candidate: a purchase must not take the credit of an equal sale.
    pub inflow: fn(title: &str, text: &str) -> Option<bool>,
    /// How far the booking date may be from a date in the PDF.
    pub slack_days: i64,
    /// Groups of equivalent amount labels, in priority order. Use only the first group present
    /// in the PDF. With no groups, the PDF must contain exactly one distinct amount.
    pub amount_labels: &'static [&'static [&'static str]],
}

/// The rules of a bank; `None` where the connector has not been taught to match by content.
fn rules_for(kind: ConnectorKind) -> Option<DocumentMatchRules> {
    match kind {
        ConnectorKind::TradeRepublic => Some(crate::connectors::trade_republic::DOCUMENT_MATCH),
        ConnectorKind::Comdirect => Some(COMDIRECT),
        _ => None,
    }
}

/// The rules of a stored connection: like [`rules_for`], but a FinTS login at comdirect (kind
/// `FinTsGeneric`, the PostBox rides along) gets the comdirect rules too.
pub(crate) fn rules_for_connection(
    conn: &Connection,
    connection_id: i64,
    kind: ConnectorKind,
) -> CoreResult<Option<DocumentMatchRules>> {
    Ok(
        if kind == ConnectorKind::FinTsGeneric && is_comdirect_login(conn, connection_id)? {
            Some(COMDIRECT)
        } else {
            rules_for(kind)
        },
    )
}

/// Whether the connection is a FinTS login at comdirect (its connector key carries a comdirect BLZ).
fn is_comdirect_login(conn: &Connection, connection_id: i64) -> CoreResult<bool> {
    let key: Option<String> = conn
        .query_row(
            "SELECT connector FROM bank_connections WHERE id = ?1",
            [connection_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(key
        .as_deref()
        .is_some_and(super::persist::is_comdirect_fints))
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
        &["ausmachender betrag"],
        &[
            "zu ihren lasten vor steuern",
            "zu ihren gunsten vor steuern",
        ],
    ],
    slack_days: 5,
    inflow: |title, text| {
        inflow_in(
            title.split_once('(').map_or(title, |(head, _)| head),
            text,
            1500,
        )
    },
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

/// Letters and digits only, lower case: "JPM USD EM Bond (Dist)" and "jpm usd em bond dist" agree.
fn squash(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// A booking a stored document may belong to: (id, ISO date, cents, counterparty).
type MatchBooking = (i64, String, i64, String);

#[derive(Debug, PartialEq, Eq)]
enum BookingMatch {
    None,
    Unique(i64),
    Ambiguous,
}

/// The one booking a document belongs to by its content: the amount and a date of the text agree
/// (within `slack_days`), the booking's counterparty carries the subject and its sign fits the
/// direction of the document (`inflow`, when known). Several candidates: only the one with the
/// nearest date, and only if that is unambiguous.
#[cfg(test)]
fn match_by_content(
    text: &str,
    subject: &str,
    bookings: &[MatchBooking],
    slack_days: i64,
    inflow: Option<bool>,
) -> Option<i64> {
    match match_by_amounts(
        &amounts_in(text),
        &pdf_dates(text),
        subject,
        bookings,
        slack_days,
        inflow,
    ) {
        BookingMatch::Unique(id) => Some(id),
        BookingMatch::None | BookingMatch::Ambiguous => None,
    }
}

fn pdf_dates(text: &str) -> Vec<jiff::civil::Date> {
    dates_in(text)
        .iter()
        .filter_map(|date| crate::dates::parse_iso(date))
        .collect()
}

/// Finds the uniquely closest booking that fits the relevant amounts and subject.
fn match_by_amounts(
    amounts: &[i64],
    days: &[jiff::civil::Date],
    subject: &str,
    bookings: &[MatchBooking],
    slack_days: i64,
    inflow: Option<bool>,
) -> BookingMatch {
    let subject = squash(subject);
    if subject.len() < 3 {
        return BookingMatch::None;
    }
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
            && distance <= slack_days
        {
            best.push((distance, *id));
        }
    }
    best.sort();
    match best.as_slice() {
        [] => BookingMatch::None,
        [(_, id)] => BookingMatch::Unique(*id),
        [(a, id), (b, _), ..] if a < b => BookingMatch::Unique(*id),
        _ => BookingMatch::Ambiguous,
    }
}

/// The amount a text states after one of the labels: the first amount of the rest of the label's
/// line or, when the label heads a column (`IBAN  Valuta  Zu Ihren Lasten vor Steuern` and the
/// values below), of the next line. Dates are no amounts ("19.07.2023" holds a "19.07"). The settled
/// amount of a trade confirmation, not the price, a fee or a tax amount.
fn settled_amounts(text: &str, label_groups: &[&[&str]]) -> Vec<(i64, String)> {
    static DATE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\b\d{1,2}\.\d{1,2}\.(?:\d{4}|\d{2})\b").unwrap());
    // ASCII lower case keeps the byte positions of the original
    let lower = text.to_ascii_lowercase();
    let Some(labels) = label_groups
        .iter()
        .find(|labels| labels.iter().any(|&label| lower.contains(label)))
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for label in *labels {
        let mut from = 0;
        while let Some(at) = lower[from..].find(label).map(|i| i + from) {
            let first_amount = |line: &str| {
                let line = DATE.replace_all(line, " ");
                let (position, amount) = amount_occurrences(&line).next()?;
                let currencies = currency_occurrences(&line)
                    .into_iter()
                    .filter_map(|(start, _, currency)| (start == position).then_some(currency))
                    .collect::<Vec<_>>();
                Some((amount, currencies))
            };
            let mut lines = text[at + label.len()..].lines();
            let own = lines.next().unwrap_or("");
            let found = first_amount(own).or_else(|| lines.next().and_then(first_amount));
            from = at + label.len();
            if let Some((cents, currencies)) = found {
                let currency = match currencies.as_slice() {
                    [] => String::new(),
                    [currency] => currency.clone(),
                    _ => continue,
                };
                out.push((cents, currency));
            }
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

/// The settlement accounts (id, IBAN, account number) whose IBAN or number the text names. An IBAN
/// is the stronger hint. An explicit reference with no match returns an empty list;
/// a receipt with no reference returns `None`.
fn accounts_named_in(text: &str, accounts: &[(i64, String, String)]) -> Option<Vec<i64>> {
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
        return Some(by_iban);
    }
    const ACCOUNT_LABEL: &str =
        r"(?:verrechnungskonto|abrechnungskonto|girokonto|verrechnung\s+(?:über|ueber)\s+konto)";
    static ACCOUNT_NUMBER: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(&format!(
            r"(?i)\b{ACCOUNT_LABEL}(?:[ \t]+(?:nr\.?|nummer))?[ \t]*[:=]?\s*(\d{{6,}})\b"
        ))
        .unwrap()
    });
    static LABELLED_IBAN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(&format!(
            r"(?i)\b{ACCOUNT_LABEL}(?:[ \t]+\(?IBAN\)?)?[ \t]*[:=]?\s*DE(?:[ \t]*[0-9]){{20}}\b"
        ))
        .unwrap()
    });
    if LABELLED_IBAN.is_match(text) {
        return Some(Vec::new());
    }
    let numbers: Vec<&str> = ACCOUNT_NUMBER
        .captures_iter(text)
        .map(|label| label.get(1).unwrap().as_str().trim_start_matches('0'))
        .collect();
    if numbers.is_empty() {
        return None;
    }
    Some(
        accounts
            .iter()
            .filter(|(_, _, number)| {
                let digits: String = number.chars().filter(char::is_ascii_digit).collect();
                let normalized = digits.trim_start_matches('0');
                digits.len() >= 6 && !normalized.is_empty() && numbers.contains(&normalized)
            })
            .map(|(id, _, _)| *id)
            .collect(),
    )
}

const CURRENCIES: &str = "EUR|USD|GBP|CHF|JPY|CAD|AUD|SEK|NOK|DKK|PLN|CZK|HUF|\u{20ac}|\\$";

/// The amounts of a text together with the currency written next to them ("25,00 EUR", "EUR 25,00",
/// "25,00- EUR", "25,00 €"): (cents, ISO code). An amount without a currency beside it is not listed.
fn amounts_with_currency(text: &str) -> Vec<(i64, String)> {
    currency_occurrences(text)
        .into_iter()
        .map(|(_, cents, currency)| (cents, currency))
        .collect()
}

fn currency_occurrences(text: &str) -> Vec<(usize, i64, String)> {
    const AMOUNT: &str = r"\d{1,3}(?:\.\d{3})+,\d{2}|\d+,\d{2}|\d+\.\d{2}";
    static AFTER: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(&format!(
            r"(?:^|[^\d,.])({AMOUNT})-?[^\S\r\n]*({CURRENCIES})"
        ))
        .unwrap()
    });
    static BEFORE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(&format!(r"({CURRENCIES})[^\S\r\n]*-?({AMOUNT})")).unwrap());
    let code = |c: &str| match c {
        "\u{20ac}" => "EUR".to_string(),
        "$" => "USD".to_string(),
        other => other.to_string(),
    };
    let mut out = Vec::new();
    let mut add = |amount: regex::Match<'_>, currency: &str| {
        // a following digit means a longer number ("0,1361 USD" is no 0,13)
        if text[amount.end()..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit())
        {
            return;
        }
        if let Ok(cents) = crate::money::parse_cents(amount.as_str()) {
            let pair = (amount.start(), cents.abs(), code(currency));
            if cents != 0 && !out.contains(&pair) {
                out.push(pair);
            }
        }
    };
    let mut suffix_positions = Vec::new();
    for c in AFTER.captures_iter(text) {
        // A currency already used as a suffix cannot be the prefix of the next amount.
        suffix_positions.push(c.get(2).unwrap().start());
        add(c.get(1).unwrap(), &c[2]);
    }
    for c in BEFORE.captures_iter(text) {
        if !suffix_positions.contains(&c.get(1).unwrap().start()) {
            add(c.get(2).unwrap(), &c[1]);
        }
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
    stated.peek().is_none()
        || stated.any(|(_, cur)| cur.is_empty() || cur.eq_ignore_ascii_case(currency))
}

/// Matches changed documents and new bookings using the dates printed in each PDF.
pub(crate) fn link_unlinked_by_content(
    conn: &Connection,
    connection_id: i64,
    rules: &DocumentMatchRules,
    changed_document_ids: &HashSet<i64>,
    new_booking_ids: &HashSet<i64>,
) -> CoreResult<()> {
    let mut accounts = conn.prepare(
        "SELECT id, external_ref, kind, iban, account_number, archived_unix IS NULL
           FROM accounts WHERE connection_id = ?1 AND deleted_ms IS NULL",
    )?;
    let rows = accounts.query_map([connection_id], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, bool>(5)?,
        ))
    })?;
    let mut active = HashSet::new();
    let mut settlement = Vec::new();
    for row in rows {
        let (id, external_ref, kind, iban, number, is_active) = row?;
        if (rules.booking_account)(&external_ref, &kind) {
            // Archived account names still count as evidence against linking to a different account.
            settlement.push((id, iban, number));
            if is_active {
                active.insert(id);
            }
        }
    }
    if active.is_empty() {
        return Ok(());
    }
    let mut new_booking_dates = BTreeSet::new();
    let mut new_booking = conn.prepare(
        "SELECT account_id, booking_date FROM transactions WHERE id = ?1 AND deleted_ms IS NULL",
    )?;
    for id in new_booking_ids {
        let Some((account, date)) = new_booking
            .query_row([id], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .optional()?
        else {
            continue;
        };
        if active.contains(&account)
            && let Some(date) = crate::dates::parse_iso(&date)
        {
            new_booking_dates.insert(date);
        }
    }
    // The PostBox creation date can differ from the dates in the PDF. With a new
    // booking, even an older open PDF may contain the relevant transaction date.
    let mut pending = conn.prepare(
        "SELECT d.id, d.title, d.doc_type FROM booking_documents d
          JOIN accounts a ON a.id = d.account_id
         WHERE d.booking_id IS NULL AND a.connection_id = ?1
           AND a.archived_unix IS NULL AND a.deleted_ms IS NULL",
    )?;
    let docs = pending
        .query_map([connection_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut postings = conn.prepare(
        "SELECT t.id, t.booking_date, t.amount_cents, t.counterparty || ' ' || t.purpose,
                t.currency, t.account_id
           FROM transactions t JOIN accounts a ON a.id = t.account_id
          WHERE a.connection_id = ?1 AND t.deleted_ms IS NULL AND a.deleted_ms IS NULL
            AND a.archived_unix IS NULL
            AND t.external_id NOT LIKE 'pending-%' AND t.external_id NOT LIKE 'advanzia-hold-%'
            AND t.booking_date BETWEEN ?2 AND ?3",
    )?;
    let mut pdf = conn.prepare("SELECT pdf_bytes FROM booking_documents WHERE id = ?1")?;
    for (id, title, doc_type) in docs {
        let changed_document = changed_document_ids.contains(&id);
        if !changed_document && new_booking_dates.is_empty() {
            continue;
        }
        if !rules.link_types.contains(&doc_type.as_str()) {
            continue;
        }
        if !rules.subjects_from_text && (rules.subjects)(&title, "").is_empty() {
            continue;
        }
        let bytes: Vec<u8> = pdf.query_row([id], |row| row.get(0))?;
        let text = extract_pdf_text(&bytes);
        let days = pdf_dates(&text);
        let ranges: Vec<_> = days
            .iter()
            .map(|&date| {
                (
                    crate::dates::add_days(date, -rules.slack_days)
                        .unwrap_or(jiff::civil::Date::MIN),
                    crate::dates::add_days(date, rules.slack_days)
                        .unwrap_or(jiff::civil::Date::MAX),
                )
            })
            .collect();
        let (Some(from), Some(to)) = (
            ranges.iter().map(|&(from, _)| from).min(),
            ranges.iter().map(|&(_, to)| to).max(),
        ) else {
            continue;
        };
        if !changed_document
            && !ranges
                .iter()
                .any(|&(from, to)| new_booking_dates.range(from..=to).next().is_some())
        {
            continue;
        }
        let subjects = (rules.subjects)(&title, &text);
        if subjects.is_empty() {
            continue;
        }
        let inflow = (rules.inflow)(&title, &text);
        if rules.direction_required && inflow.is_none() {
            continue;
        }
        let named = if rules.account_from_text {
            match accounts_named_in(&text, &settlement).as_deref() {
                Some([account]) if active.contains(account) => Some(*account),
                None if settlement.len() <= 1 => None,
                _ => continue,
            }
        } else {
            None
        };
        let (amounts, currencies) = if rules.amount_labels.is_empty() {
            (unambiguous_amounts(&text), amounts_with_currency(&text))
        } else {
            let amounts = settled_amounts(&text, rules.amount_labels);
            (amounts.iter().map(|(cents, _)| *cents).collect(), amounts)
        };
        if amounts.is_empty() {
            continue;
        }
        let rows = postings.query_map(
            params![
                connection_id,
                crate::dates::iso(from),
                crate::dates::iso(to)
            ],
            |r| {
                Ok((
                    (r.get(0)?, r.get(1)?, r.get::<_, i64>(2)?, r.get(3)?),
                    r.get::<_, String>(4)?,
                    r.get::<_, i64>(5)?,
                ))
            },
        )?;
        let mut candidates = Vec::new();
        for row in rows {
            let (booking, currency, account) = row?;
            if active.contains(&account)
                && named.is_none_or(|named| named == account)
                && amounts.contains(&booking.2.abs())
                && currency_fits(&currencies, booking.2, &currency)
            {
                candidates.push(booking);
            }
        }
        // Only a missing match can fall back to a weaker security identifier.
        let found = subjects
            .iter()
            .map(|subject| {
                match_by_amounts(
                    &amounts,
                    &days,
                    subject,
                    &candidates,
                    rules.slack_days,
                    inflow,
                )
            })
            .find(|result| *result != BookingMatch::None);
        if let Some(BookingMatch::Unique(booking)) = found
            && (changed_document || new_booking_ids.contains(&booking))
        {
            // An occupied best match blocks a link instead of sending the document to a nearby
            // booking. Check at write time because an earlier PDF in this pass may have linked it.
            conn.execute(
                "UPDATE booking_documents SET booking_id = ?1
                  WHERE id = ?2 AND booking_id IS NULL
                    AND NOT EXISTS (SELECT 1 FROM booking_documents WHERE booking_id = ?1)",
                params![booking, id],
            )?;
        }
    }
    Ok(())
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
        let text = "WERTPAPIERABRECHNUNG SPARPLAN JPM USD EM Bond USD (Dist) 0,311177 Stk. 80,34 EUR \
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
    fn only_the_event_part_of_each_title_influences_direction() {
        let trade_republic = crate::connectors::trade_republic::DOCUMENT_MATCH.inflow;
        assert_eq!(trade_republic("Kauf – Dividenden ETF", ""), Some(false));
        assert_eq!(trade_republic("Verkauf – Sparplan ETF", ""), Some(true));
        let title = "Wertpapierabrechnung 3 St. WKN A1JX52 (Dividenden ETF)";
        assert_eq!((COMDIRECT.inflow)(title, "Wertpapierkauf"), Some(false));
        assert_eq!((COMDIRECT.inflow)(title, ""), None);
        assert_eq!(
            (COMDIRECT.inflow)("Wertpapierabrechnung Kauf (Dividenden ETF)", ""),
            Some(false)
        );
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
    }

    #[test]
    fn a_currency_suffix_cannot_also_describe_the_next_amount() {
        for separator in [" ", "\n"] {
            let bound = amounts_with_currency(&format!("100,00 USD{separator}92,00 EUR"));
            assert!(currency_fits(&bound, 92_00, "EUR"));
            assert!(!currency_fits(&bound, 92_00, "USD"));
        }
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
            settled_amounts(text, &[&["ausmachender betrag"]]),
            [(500_00, "EUR".to_owned()), (7_00, "EUR".to_owned())]
        );
        assert!(settled_amounts("Kurs 100,00 EUR", &[&["ausmachender betrag"]]).is_empty());
        assert!(settled_amounts("", &[&["ausmachender betrag"]]).is_empty());
        // comdirect: the label heads a column and the values stand below; a date is no amount at the line's end
        let trade = "Kurswert : EUR 24,57\nProvision : EUR 0,37\nIBAN Valuta Zu Ihren Lasten vor Steuern\nDE00 0000 0000 0000 0000 00 EUR 19.07.2023 EUR 24,94\nVerwahrungs-Art: X";
        assert_eq!(
            settled_amounts(trade, COMDIRECT.amount_labels),
            [(24_94, "EUR".to_owned())]
        );
        let dividend = "Bruttobetrag: USD 6,09\nzum Devisenkurs: EUR/USD 1,058500 EUR 5,75\nVerrechnung \u{fc}ber Konto (IBAN) Valuta Zu Ihren Gunsten vor Steuern\nDE00 0000 0000 0000 0000 00 EUR 29.09.2023 EUR 5,75";
        assert_eq!(
            settled_amounts(dividend, COMDIRECT.amount_labels),
            [(5_75, "EUR".to_owned())]
        );
        // a price is no candidate for a booking when the settled amount is known
        let bookings = vec![(1, "2025-12-04".to_string(), -100_00, "ETF ISIN".to_string())];
        assert_eq!(
            match_by_amounts(
                &[500_00],
                &pdf_dates("04.12.2025"),
                "ISIN",
                &bookings,
                5,
                Some(false)
            ),
            BookingMatch::None
        );
        assert_eq!(
            match_by_amounts(
                &[100_00],
                &pdf_dates("04.12.2025"),
                "ISIN",
                &bookings,
                5,
                Some(false)
            ),
            BookingMatch::Unique(1)
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
            Some(vec![2])
        );
        // the IBAN wins over a number that belongs to another account
        assert_eq!(
            accounts_named_in(
                "IBAN DE75 2004 1155 0765 4321 00 Depot 123456700",
                &accounts
            ),
            Some(vec![2])
        );
        assert!(accounts_named_in("Depotnummer 123456700", &accounts).is_none());
        assert_eq!(
            accounts_named_in("Verrechnungskonto Nr. 123456700", &accounts),
            Some(vec![1])
        );
        assert!(accounts_named_in("nichts", &accounts).is_none());
        assert_eq!(
            accounts_named_in("Verrechnungskonto Nr. 999999999", &accounts),
            Some(vec![])
        );
        // part of a longer number is no hit
        assert!(accounts_named_in("Vorgang 91234567001", &accounts).is_none());
    }

    #[test]
    fn settlement_account_numbers_can_have_leading_zeros() {
        let accounts = [
            (1, String::new(), "0000012345".to_owned()),
            (2, String::new(), "0000000000".to_owned()),
        ];
        assert_eq!(
            accounts_named_in("Verrechnungskonto Nr. 0000012345", &accounts),
            Some(vec![1])
        );
        assert_eq!(
            accounts_named_in("Verrechnungskonto Nr. 0000012346", &accounts),
            Some(vec![])
        );
        assert_eq!(
            accounts_named_in("Verrechnungskonto Nr. 0000000000", &accounts),
            Some(vec![])
        );
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
