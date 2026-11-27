//! Statement PDFs a card portal hands out next to its bookings (Amex): the app
//! downloads them in the signed-in portal page and stores them here, each at
//! most once, as a statement of the card's account. A statement is known by its
//! account and closing date: the portal's own statement ids change with every
//! login.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use rusqlite::params;

use super::portal_import::account_ref;
use crate::error::CoreError;
use crate::vault::Vault;

/// One statement PDF from a card portal.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PortalStatement {
    /// Closing date of the statement, `YYYY-MM-DD`: with the account, what
    /// tells statements apart.
    pub date: String,
    pub bytes: Vec<u8>,
}

/// One entry of a portal's statement list: closing date, `YYYY-MM-DD`, and the
/// link to its PDF (absolute, or relative to the portal's origin).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PortalStatementItem {
    pub date: String,
    pub path: String,
}

/// One entry of a portal's statement list that names its PDF only by a document
/// id (Open Bank / Amazon Visa, live-checked at the real portal): the statements
/// response carries a closing date and a `documentId` (+ `version`) per entry,
/// the PDF itself answers a second request per statement
/// (`documents/:id/versions/:version` → `{ "documentUri": "…x.pdf" }`).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PortalStatementRef {
    pub date: String,
    pub document_id: String,
    /// The document's version from the same entry, `""` when the entry has none.
    pub version: String,
}

pub fn parse_portal_statement_refs(body: &str) -> Vec<PortalStatementRef> {
    serde_json::from_str::<serde_json::Value>(body)
        .map(|v| {
            let mut out = Vec::new();
            refs(&v, &mut out);
            out
        })
        .unwrap_or_default()
}

/// Every dated entry with its own document id onto `out`, once each — same
/// boundary as `entry`: the id is a direct field of the dated object, never
/// one of a nested entry (a movement's `date` names no statement).
fn refs(v: &serde_json::Value, out: &mut Vec<PortalStatementRef>) {
    if let Some(r) = statement_ref(v)
        && !out.contains(&r)
    {
        out.push(r);
    }
    match v {
        serde_json::Value::Object(m) => m.values().for_each(|v| refs(v, out)),
        serde_json::Value::Array(a) => a.iter().for_each(|v| refs(v, out)),
        _ => {}
    }
}

/// One list entry of the document-id kind: its closing date and its
/// `documentId` are direct fields of the same object.
fn statement_ref(v: &serde_json::Value) -> Option<PortalStatementRef> {
    let m = match v {
        serde_json::Value::Object(m) => m,
        _ => return None,
    };
    let date = own_date(m)?;
    let document_id = document_id(m)?;
    let version = m
        .iter()
        .filter(|(k, _)| field_name(k) == "version")
        .find_map(|(_, v)| v.as_str())
        .unwrap_or_default()
        .to_string();
    Some(PortalStatementRef {
        date,
        document_id: document_id.to_string(),
        version,
    })
}

/// The entry's `documentId` as a direct field of the object (nonempty, no
/// whitespace), like `statement_ref` reads it.
fn document_id(m: &serde_json::Map<String, serde_json::Value>) -> Option<&str> {
    m.iter()
        .filter(|(k, _)| field_name(k) == "documentid")
        .find_map(|(_, v)| v.as_str())
        .filter(|s| !s.is_empty() && !s.contains(char::is_whitespace))
}

/// The statements list of the Amazon-Visa portal (Open Bank), schema-agnostic:
/// its JSON schema is undocumented and changes, so every object that carries a
/// closing date and a PDF link is an entry (`{…, "date": "2026-10-15", "url":
/// "…x.pdf", …}`) — the date sits on the list item, the link often one nesting
/// level deeper (`files: [{"url": "…x.pdf"}]`). A date is always paired with a
/// PDF of the same entry, never with the link of a neighbour. Duplicate
/// `(date, path)` pairs and entries without a complete pair are left out.
pub fn parse_portal_statement_list(body: &str) -> Vec<PortalStatementItem> {
    serde_json::from_str::<serde_json::Value>(body)
        .map(|v| parse_statement_entries(&v))
        .unwrap_or_default()
}

/// The complete entries of an already-parsed response, once each.
fn parse_statement_entries(v: &serde_json::Value) -> Vec<PortalStatementItem> {
    let mut out: Vec<(String, String)> = Vec::new();
    collect(v, &mut out);
    out.into_iter()
        .map(|(date, path)| PortalStatementItem { date, path })
        .collect()
}

/// Every complete entry (date and PDF from the same object) onto `out`, once each.
fn collect(v: &serde_json::Value, out: &mut Vec<(String, String)>) {
    if let Some((date, path)) = entry(v)
        && !out.contains(&(date.clone(), path.clone()))
    {
        out.push((date, path));
    }
    match v {
        serde_json::Value::Object(m) => m.values().for_each(|v| collect(v, out)),
        serde_json::Value::Array(a) => a.iter().for_each(|v| collect(v, out)),
        _ => {}
    }
}

/// First closing date of an object's direct fields: a field whose (separator-free,
/// lower-cased) name names a statement/closing date (best of `statementDate`,
/// `closingDate`, `endDate`, `documentDate`, `date`), whatever the map order —
/// `serde_json` maps iterate alphabetically, e.g. a `created` would otherwise win
/// over `end_date` or `statementDate`. Mere metadata (`created`, `dueDate`) never
/// names a statement date: an object with only such fields carries no closing
/// date of its own — its PDF belongs to the enclosing entry.
fn own_date(m: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    const NAMED: [&str; 5] = [
        "statementdate",
        "closingdate",
        "enddate",
        "documentdate",
        "date",
    ];
    let mut best: Option<(usize, String)> = None;
    for (k, v) in m {
        let Some(date) = v.as_str().and_then(closing_date) else {
            continue;
        };
        let name = field_name(k);
        let Some(prio) = NAMED.iter().position(|n| *n == name) else {
            continue;
        };
        if best.as_ref().is_none_or(|(p, _)| prio < *p) {
            best = Some((prio, date));
        }
    }
    best.map(|(_, date)| date)
}

/// First PDF link within `v`, without crossing into an object that
/// carries its own closing date — that is another list entry, so its PDF
/// belongs to that entry, not to ours. A file object with only metadata
/// (`created`) is no entry of its own: its PDF belongs to the entry around it.
fn pdf_in(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::Object(m) if own_date(m).is_none() => m
            .values()
            .filter_map(|s| s.as_str())
            .find_map(pdf_link)
            .or_else(|| m.values().find_map(pdf_in)),
        serde_json::Value::Array(a) => a
            .iter()
            .find_map(|i| i.as_str().and_then(pdf_link))
            .or_else(|| a.iter().find_map(pdf_in)),
        _ => None,
    }
}

/// One list entry: its closing date is a direct field of the entry, the
/// PDF link comes from the entry itself (`files` array included, string
/// arrays as well as objects) — never from another entry. An entry without
/// a PDF and a PDF without a date emit nothing instead of pairing with
/// each other or with a neighbour.
fn entry(v: &serde_json::Value) -> Option<(String, String)> {
    let m = match v {
        serde_json::Value::Object(m) => m,
        _ => return None,
    };
    let date = own_date(m)?;
    let path = m
        .values()
        .filter_map(|v| v.as_str().and_then(pdf_link))
        .chain(m.values().filter_map(pdf_in))
        .next()?;
    Some((date, path))
}

/// The parsed entries of a captured response together with its shape:
/// `readable` is only true for a recognized statements list — including an
/// empty one. An unreadable or unrelated response (a balance summary that
/// only carries "statement" in its URL) is no list: the download must count
/// it as a failure instead of "nothing new". `rejected` counts its PDF links
/// that no dated entry claims — an incomplete retrieval, no "nothing new".
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PortalStatementListing {
    pub items: Vec<PortalStatementItem>,
    pub readable: bool,
    pub rejected: u32,
}

/// `parse_portal_statement_list` plus the response's shape: `readable` tells
/// a verifiably empty statement list (no failure) apart from an unrelated
/// captured response (a failure), `rejected` counts the captured PDFs without
/// a usable closing date (failures, like undated visible links).
pub fn parse_portal_statement_listing(body: &str) -> PortalStatementListing {
    let Some(v) = serde_json::from_str::<serde_json::Value>(body).ok() else {
        return PortalStatementListing {
            items: Vec::new(),
            readable: false,
            rejected: 0,
        };
    };
    let items = parse_statement_entries(&v);
    let readable = !items.is_empty() || looks_like_list(&v);
    // failures only for a recognized list: an unrelated response is one
    // failure as a whole, its PDFs are no statements of ours
    let rejected = if readable {
        let mut rejected = HashSet::new();
        rejected_entries(&v, false, &mut rejected);
        rejected.len() as u32
    } else {
        0
    };
    PortalStatementListing {
        items,
        readable,
        rejected,
    }
}

/// A recognized statements list: an array whose items are all objects — empty
/// (a real, empty list) or with an entry-shaped item (a statement-named field
/// date, a PDF link). Other answers (balances, notifications, errors) are not.
/// A bare empty array is a list only at the root or under a statement-named
/// field (`statements`): `{"error": "session expired", "details": []}` is no
/// list, no matter how empty.
fn looks_like_list(v: &serde_json::Value) -> bool {
    list_like(None, v)
}

fn list_like(field: Option<&str>, v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Object(m) => m
            .iter()
            .any(|(k, val)| list_like(Some(&field_name(k)), val)),
        serde_json::Value::Array(a) => {
            let all_objects = a.iter().all(|i| matches!(i, serde_json::Value::Object(_)));
            // a bare empty array is a list only at the root or under a
            // statement-named field — elsewhere it is no statements list
            let empty_list = a.is_empty() && field.is_none_or(|f| f.contains("statement"));
            // an entry-shaped item: a closing date, a document id (the
            // document-id kind of entry, dated or not), or a PDF where `entry`
            // would read one — directly or nested (a `files` array of links)
            let shaped = empty_list
                || a.iter().any(|i| match i {
                    serde_json::Value::Object(m) => {
                        own_date(m).is_some()
                            || document_id(m).is_some()
                            || m.values()
                                .filter_map(|s| s.as_str())
                                .any(|s| pdf_link(s).is_some())
                            || m.values().find_map(pdf_in).is_some()
                    }
                    _ => false,
                });
            (all_objects && shaped) || a.iter().any(|i| list_like(None, i))
        }
        _ => false,
    }
}

/// A field's name, alphanumerics kept, lower-cased (whole words within it
/// decide, e.g. `visaStatements`).
fn field_name(k: &str) -> String {
    k.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// The PDF links of `v` that no dated entry claims (`under_entry` when beneath
/// a dated one): each unidentified statement is a failure, like an undated
/// visible link — one link once, and PDFs of a dated entry belong to its date.
/// String items of arrays are PDF links too (`pdf_in` reads them as well).
/// A direct `documentId` without a closing date is an unidentified statement
/// as well: it counts once, like an undated PDF link — but only when the
/// object carries no PDF of its own (that one is counted instead, so an
/// entry is never two failures).
fn rejected_entries(v: &serde_json::Value, under_entry: bool, out: &mut HashSet<String>) {
    match v {
        serde_json::Value::Object(m) => {
            let under = under_entry || own_date(m).is_some();
            for path in m.values().filter_map(|s| s.as_str()).filter_map(pdf_link) {
                if !under {
                    out.insert(path.to_string());
                }
            }
            let has_pdf = m
                .values()
                .filter_map(|s| s.as_str())
                .find_map(pdf_link)
                .is_some()
                || m.values().find_map(pdf_in).is_some();
            if !under && !has_pdf && let Some(id) = document_id(m) {
                out.insert(id.to_string());
            }
            m.values().for_each(|val| rejected_entries(val, under, out));
        }
        serde_json::Value::Array(a) => {
            for item in a.iter().filter_map(|i| i.as_str()) {
                if !under_entry && let Some(path) = pdf_link(item) {
                    out.insert(path.to_string());
                }
            }
            a.iter()
                .for_each(|val| rejected_entries(val, under_entry, out));
        }
        _ => {}
    }
}

/// A plain closing date (`YYYY-MM-DD`, optionally with a time suffix, or
/// `DD.MM.YYYY`, the day must exist), normalized to `YYYY-MM-DD`.
fn closing_date(s: &str) -> Option<String> {
    static ISO: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^(\d{4})-(\d{2})-(\d{2})([T ].*)?$").unwrap());
    static DE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^(\d{1,2})\.(\d{1,2})\.(\d{4})$").unwrap());
    if let Some(c) = ISO.captures(s) {
        return crate::dates::ymd(c[1].parse().ok()?, c[2].parse().ok()?, c[3].parse().ok()?)
            .map(|_| format!("{}-{}-{}", &c[1], &c[2], &c[3]));
    }
    let c = DE.captures(s)?;
    crate::dates::ymd(c[3].parse().ok()?, c[2].parse().ok()?, c[1].parse().ok()?)
        .map(crate::dates::iso)
}

/// A path or URL whose path part ends in `.pdf`, query or fragment included
/// (`/S1.pdf?download=1`, the statement PDFs of the portals); anything else
/// (ids, tokens, HTML) is no link. The link comes back as it is, download
/// keeps the query. The rule matches the visible-links one in the page script.
pub fn pdf_link(s: &str) -> Option<String> {
    static PDF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\.pdf(\?|#|$)").unwrap());
    let t = s.trim();
    (t.len() > 4 && !t.contains(char::is_whitespace) && PDF.is_match(t)).then(|| t.to_string())
}

/// `source_url` of a stored portal statement.
fn source_of(account: i64, date: &str) -> String {
    format!("{SOURCE_PREFIX}{account}:{date}")
}

const SOURCE_PREFIX: &str = "portal-statement:";

/// `DD.MM.YYYY` of an ISO date (the date as it is, if it is no ISO date).
fn german_date(date: &str) -> String {
    match date.split('-').collect::<Vec<_>>().as_slice() {
        [y, m, d] if y.len() == 4 => format!("{d}.{m}.{y}"),
        _ => date.to_string(),
    }
}

/// The account a card's statements belong to, as the booking import maps the
/// card (#243): the card's own account (key from the card switcher), else the
/// account whose number ends in the card's digits (at least four, e.g. an
/// account taken over from an import), else the login's first card account.
fn card_account(
    db: &rusqlite::Connection,
    connection_id: i64,
    base_ref: &str,
    card_key: &str,
    card_number: &str,
) -> Result<Option<i64>, CoreError> {
    let own = format!("{base_ref}-{card_key}");
    let digits = |s: &str| s.chars().filter(char::is_ascii_digit).collect::<String>();
    let number = digits(card_number);
    let mut st = db.prepare(
        "SELECT id, external_ref, IFNULL(account_number, '') FROM accounts
          WHERE connection_id = ?1 AND deleted_ms IS NULL AND archived_unix IS NULL
          ORDER BY id",
    )?;
    let rows = st
        .query_map([connection_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .iter()
        .find(|(_, r, _)| !card_key.is_empty() && *r == own)
        .or_else(|| {
            rows.iter()
                .find(|(_, _, n)| number.len() >= 4 && digits(n).ends_with(&number))
        })
        .or_else(|| rows.iter().find(|(_, r, _)| r == base_ref))
        .or_else(|| rows.first())
        .map(|(id, _, _)| *id))
}

#[uniffi::export]
impl Vault {
    /// Closing dates of the portal statements the card (`card_key`, its digits
    /// `card_number`) already has (see [`Self::import_portal_statements`]): the app downloads only the
    /// others.
    pub fn portal_statement_dates(
        &self,
        connection_id: i64,
        card_key: String,
        card_number: String,
    ) -> Result<Vec<String>, CoreError> {
        let (kind, _) = self.portal_connection(connection_id)?;
        let base_ref = account_ref(kind, connection_id);
        let db = self.db()?;
        let Some(account) = card_account(&db, connection_id, &base_ref, &card_key, &card_number)?
        else {
            return Ok(Vec::new());
        };
        let mut st = db.prepare(
            "SELECT doc_date FROM booking_documents WHERE account_id = ?1 AND source_url LIKE ?2",
        )?;
        let dates = st
            .query_map(params![account, format!("{SOURCE_PREFIX}%")], |r| {
                r.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(dates)
    }

    /// Stores statement PDFs of a portal login with the account of the card
    /// `card_key` names (a hash of the portal's card token, "" = unknown;
    /// `card_number` its digits from the card switcher, "" = unknown).
    /// Returns how many were new; a statement stored before, or no PDF, is
    /// left out.
    pub fn import_portal_statements(
        &self,
        connection_id: i64,
        card_key: String,
        card_number: String,
        statements: Vec<PortalStatement>,
    ) -> Result<u32, CoreError> {
        let (kind, name) = self.portal_connection(connection_id)?;
        let base_ref = account_ref(kind, connection_id);
        let db = self.db()?;
        let Some(account) = card_account(&db, connection_id, &base_ref, &card_key, &card_number)?
        else {
            return Ok(0);
        };
        let tx = db.unchecked_transaction()?;
        let mut added = 0;
        for s in &statements {
            if !s.bytes.starts_with(b"%PDF") || s.date.len() != 10 {
                continue;
            }
            let title = format!("Kontoauszug {name} {}", german_date(&s.date));
            let file_name = format!("{}_Kontoauszug.pdf", s.date);
            let stored = crate::documents::insert_doc(
                &tx,
                None,
                Some(account),
                &title,
                "kontoauszug",
                "application/pdf",
                &file_name,
                &s.bytes,
                &source_of(account, &s.date),
                &s.date,
            )?;
            added += u32::from(stored.is_some());
        }
        tx.commit()?;
        Ok(added)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ConnectorKind;

    #[test]
    fn statements_are_stored_once_with_the_card_account() {
        let dir = crate::test_support::TempDir::new("girocrab-portal-statements");
        let v = Vault::open(
            dir.join("v.db").to_string_lossy().to_string(),
            vec![5u8; 32],
        )
        .unwrap();
        let conn = v
            .add_bank_connection(ConnectorKind::AmericanExpress, String::new(), "Amex".into())
            .unwrap();
        // the card account comes from a booking import
        let tx = r#"{"transactions":[{"charge_date":"2026-11-03","amount":10,"description":"REWE","identifier":"A1"}]}"#;
        v.clone()
            .import_portal_capture(conn, vec![tx.into()], "[]".into())
            .unwrap();
        let pdf = |date: &str| PortalStatement {
            date: date.into(),
            bytes: format!("%PDF-1.4 {date}").into_bytes(),
        };
        let dates = || {
            let mut d = v
                .portal_statement_dates(conn, String::new(), String::new())
                .unwrap();
            d.sort();
            d
        };
        assert!(dates().is_empty());
        let added = v
            .import_portal_statements(
                conn,
                String::new(),
                String::new(),
                vec![
                    pdf("2026-10-25"),
                    pdf("2026-09-25"),
                    // no PDF: left out
                    PortalStatement {
                        date: "2026-08-25".into(),
                        bytes: b"{\"error\":1}".to_vec(),
                    },
                ],
            )
            .unwrap();
        assert_eq!(added, 2);
        assert_eq!(dates(), vec!["2026-09-25", "2026-10-25"]);
        // fetched again after a new login (the portal's ids change): nothing new
        let again = v
            .import_portal_statements(conn, String::new(), String::new(), vec![pdf("2026-10-25")])
            .unwrap();
        assert_eq!(again, 0);
        assert_eq!(dates().len(), 2);
        let (account, title, doc_type): (Option<i64>, String, String) = v
            .db()
            .unwrap()
            .query_row(
                "SELECT account_id, title, doc_type FROM booking_documents WHERE doc_date = '2026-10-25'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert!(account.is_some());
        assert_eq!(title, "Kontoauszug American Express 25.10.2026");
        assert_eq!(doc_type, "kontoauszug");
    }

    #[test]
    fn a_card_reaches_the_account_with_its_digits() {
        let dir = crate::test_support::TempDir::new("girocrab-portal-statements-digits");
        let v = Vault::open(
            dir.join("v.db").to_string_lossy().to_string(),
            vec![5u8; 32],
        )
        .unwrap();
        let conn = v
            .add_bank_connection(ConnectorKind::AmericanExpress, String::new(), "Amex".into())
            .unwrap();
        // the login's first card and a card taken over from an import (another ref)
        for (r, number) in [("portal-x", "11111"), ("mm:business", "XXXX-XXXXXX-22222")] {
            let id = v
                .create_offline_account(
                    "Karte".into(),
                    crate::models::AccountKind::CreditCard,
                    "EUR".into(),
                    0,
                )
                .unwrap();
            v.db()
                .unwrap()
                .execute(
                    "UPDATE accounts SET connection_id = ?1, external_ref = ?2, account_number = ?3 WHERE id = ?4",
                    rusqlite::params![conn, r, number, id],
                )
                .unwrap();
        }
        let db = v.db().unwrap();
        let id_of = |r: &str| -> i64 {
            db.query_row(
                "SELECT id FROM accounts WHERE external_ref = ?1",
                [r],
                |x| x.get(0),
            )
            .unwrap()
        };
        let base = account_ref(ConnectorKind::AmericanExpress, conn);
        assert_eq!(
            card_account(&db, conn, &base, "kkkk", "••••22222").unwrap(),
            Some(id_of("mm:business"))
        );
        // too few digits, or none: the first card account
        assert_eq!(
            card_account(&db, conn, &base, "kkkk", "222").unwrap(),
            Some(id_of("portal-x"))
        );
        assert_eq!(
            card_account(&db, conn, &base, "", "").unwrap(),
            Some(id_of("portal-x"))
        );
    }
}

#[cfg(test)]
mod statement_list_tests {
    use super::*;
    use crate::models::ConnectorKind;

    /// Amazon Visa / Open Bank: the schema-agnostic list parser reads closing
    /// date and PDF link out of invented list items (invented values only).
    #[test]
    fn statement_list_reads_date_and_pdf_whenever_it_looks_like_one() {
        let body = r#"{"hasMore":false,"statements":[
            {"id":"S1","statementDate":"2026-10-15","files":[{"format":"PDF","url":"/api/cards/C1/statements/S1.pdf"}]},
            {"id":"S2","statementDate":"15.09.2026","files":[{"format":"HTML","url":"/api/cards/C1/statements/S2"}]},
            {"id":"S3","documentDate":"2026-08-14T00:00:00Z","url":"https://docs.example.net/statement/S3.pdf"}
        ]}"#;
        let items = parse_portal_statement_list(body);
        assert_eq!(
            items,
            vec![
                PortalStatementItem {
                    date: "2026-10-15".into(),
                    path: "/api/cards/C1/statements/S1.pdf".into()
                },
                PortalStatementItem {
                    date: "2026-08-14".into(),
                    path: "https://docs.example.net/statement/S3.pdf".into()
                },
            ]
        );
    }

    /// An incomplete entry never borrows the date or the PDF of a neighbour
    /// (finding of the PR review): the list reads only complete entries.
    #[test]
    fn a_date_never_pairs_with_the_pdf_of_another_entry() {
        // first entry without a PDF, second without a date: nothing emitted
        let body = r#"{"statements":[
            {"id":"S1","statementDate":"2026-10-15"},
            {"id":"S2","url":"/api/cards/C1/statements/S2.pdf"}]}"#;
        assert!(parse_portal_statement_list(body).is_empty());
        // the complete entry is read — and no pair of the incomplete ones
        // ("date of the first, PDF of the second") is emitted
        let body = r#"{"statements":[
            {"id":"S1","statementDate":"2026-10-15"},
            {"id":"S2","statementDate":"2026-09-14","url":"/api/cards/C1/statements/S2.pdf"}]}"#;
        assert_eq!(
            parse_portal_statement_list(body),
            vec![PortalStatementItem {
                date: "2026-09-14".into(),
                path: "/api/cards/C1/statements/S2.pdf".into()
            }]
        );
    }

    /// `serde_json` maps iterate alphabetically: a field whose name names the
    /// statement's date wins over any other date-like field — `created` or
    /// `dueDate` would otherwise be read before `end_date`/`statementDate`
    /// (invented values only).
    #[test]
    fn a_statement_named_date_wins_over_other_date_fields() {
        let body = r#"{"statements":[
            {"id":"S1","dueDate":"2026-10-01","statementDate":"2026-09-30","url":"/a/1.pdf"},
            {"id":"S2","created":"2026-10-01","end_date":"2026-10-31","url":"/a/2.pdf"}]}"#;
        assert_eq!(
            parse_portal_statement_list(body),
            vec![
                PortalStatementItem {
                    date: "2026-09-30".into(),
                    path: "/a/1.pdf".into()
                },
                PortalStatementItem {
                    date: "2026-10-31".into(),
                    path: "/a/2.pdf".into()
                },
            ]
        );
    }

    /// Not a list (error page, login body): nothing found, no panic.
    #[test]
    fn anything_else_parses_to_nothing() {
        for body in [
            "",
            "kein JSON",
            r#"{"error":"Hinweis vom Portal"}"#,
            r#"[]"#,
        ] {
            assert!(parse_portal_statement_list(body).is_empty(), "{body}");
        }
    }

    /// Duplicates (same statement named twice) collapse by date and path.
    #[test]
    fn duplicate_entries_are_left_out() {
        let body = r#"{"statements":[{"date":"2026-10-15","url":"/a/1.pdf"},
                                     {"date":"2026-10-15","url":"/a/1.pdf"},
                                     {"date":"2026-10-15","url":"/a/2.pdf"}]}"#;
        assert_eq!(parse_portal_statement_list(body).len(), 2);
    }

    /// A `files` array of plain PDF paths carries the entry's PDF as well as an
    /// object array; the entry's own date is used either way.
    #[test]
    fn a_string_array_carries_the_pdf_of_its_entry() {
        let body = r#"{"statements":[
            {"date":"2026-10-15","files":["/api/cards/C1/statements/S1.pdf"]},
            {"date":"2026-09-14","files":[{"url":"/api/cards/C1/statements/S2.pdf"}]}]}"#;
        assert_eq!(
            parse_portal_statement_list(body),
            vec![
                PortalStatementItem {
                    date: "2026-10-15".into(),
                    path: "/api/cards/C1/statements/S1.pdf".into()
                },
                PortalStatementItem {
                    date: "2026-09-14".into(),
                    path: "/api/cards/C1/statements/S2.pdf".into()
                },
            ]
        );
    }

    /// A nested file object with only metadata (`created`) is no entry of its
    /// own: its PDF belongs to the enclosing statement's closing date — the
    /// `created` does not turn it into a separate entry.
    #[test]
    fn a_file_objects_created_metadata_stays_with_the_enclosing_date() {
        let body = r#"{"statements":[
            {"statementDate":"2026-10-31","file":{"created":"2026-10-01","url":"/api/statements/S.pdf"}}]}"#;
        assert_eq!(
            parse_portal_statement_list(body),
            vec![PortalStatementItem {
                date: "2026-10-31".into(),
                path: "/api/statements/S.pdf".into()
            }]
        );
    }

    /// Readability: a recognized (also empty) statements list reads, unrelated
    /// responses (balances on a statement URL, error pages) do not.
    #[test]
    fn readability_tells_an_empty_list_from_an_unrelated_answer() {
        // entries → readable
        let listed = parse_portal_statement_listing(
            r#"{"statements":[{"date":"2026-10-15","url":"/a/1.pdf"}]}"#,
        );
        assert!(listed.readable);
        assert_eq!(listed.items.len(), 1);
        // a real, empty list → readable, no items
        for body in [
            r#"{"statements":[]}"#,
            r#"{"hasMore":false,"statements":[]}"#,
            r#"{"visaStatements":[]}"#,
            "[]",
        ] {
            assert!(parse_portal_statement_listing(body).readable, "{body}");
        }
        // balances and error bodies carry no statements list
        for body in [
            r#"{"balances":[{"currency":"EUR","total":19.99}]}"#,
            r#"{"error":"Hinweis vom Portal"}"#,
            "kein JSON",
        ] {
            let l = parse_portal_statement_listing(body);
            assert!(!l.readable && l.items.is_empty(), "{body}");
        }
        // an empty array is a statements list only at the root or under a
        // statement-named field: an error with an empty details list is none
        let l = parse_portal_statement_listing(r#"{"error":"session expired","details":[]}"#);
        assert!(
            !l.readable && l.items.is_empty() && l.rejected == 0,
            "{l:?}"
        );
        // entries whose PDF has no usable closing date: captured but cannot be
        // identified — an incomplete retrieval, no "nothing new"
        let l = parse_portal_statement_listing(r#"{"statements":[{"url":"/S1.pdf"}]}"#);
        assert!(l.readable && l.items.is_empty() && l.rejected == 1, "{l:?}");
        // two undated entries in one wrapper: each unidentified statement
        // counts, the wrapper does not swallow them into one
        let l =
            parse_portal_statement_listing(r#"{"statements":[{"url":"/a.pdf"},{"url":"/b.pdf"}]}"#);
        assert!(l.readable && l.items.is_empty() && l.rejected == 2, "{l:?}");
        // PDF links as plain strings inside a `files` array: unidentified
        // entries count rejected as well
        let l = parse_portal_statement_listing(
            r#"{"statements":[{"date":"2026-10-15","url":"/ok.pdf"},{"files":["/missing.pdf"]}]}"#,
        );
        assert!(l.items.len() == 1 && l.rejected == 1, "{l:?}");
        // entries with only nested PDF strings, no dated sibling: the listing
        // reads, its two unidentified statements count rejected
        let l = parse_portal_statement_listing(
            r#"{"statements":[{"files":["/a.pdf"]},{"files":["/b.pdf"]}]}"#,
        );
        assert!(l.readable && l.items.is_empty() && l.rejected == 2, "{l:?}");
        // a PDF URL with a query downloads: date and link pair to one item
        let l = parse_portal_statement_listing(
            r#"{"statements":[{"date":"2026-10-15","url":"/S1.pdf?download=1"}]}"#,
        );
        assert!(l.items.len() == 1 && l.rejected == 0, "{l:?}");
        assert_eq!(
            parse_portal_statement_listing(
                r#"{"statements":[{"date":"2026-10-15","url":"/S1.pdf?download=1"}]}"#
            )
            .items
            .first()
            .map(|i| i.path.clone()),
            Some("/S1.pdf?download=1".into())
        );
        // a dated entry next to a rejected one: the one won, the other failed
        let l = parse_portal_statement_listing(
            r#"{"statements":[{"date":"2026-10-15","url":"/a/1.pdf"},{"url":"/S2.pdf"}]}"#,
        );
        assert!(l.items.len() == 1 && l.rejected == 1, "{l:?}");
        // a file object with only metadata belongs to its enclosing entry:
        // its PDF is attributed to the entry's date, nothing rejected
        let l = parse_portal_statement_listing(
            r#"{"statements":[{"date":"2026-10-15","files":[{"created":"2026-10-20","url":"/a/1.pdf"}]}]}"#,
        );
        assert!(l.items.len() == 1 && l.rejected == 0, "{l:?}");
        // a direct entry with a documentId but no closing date is an
        // unidentified statement too: counted rejected, not silently dropped
        // (the dated id entry is a ref of its own, no item, not rejected)
        let l = parse_portal_statement_listing(
            r#"{"statements":[{"statementDate":"2026-10-31","documentId":"d0"},{"documentId":"d1"}]}"#,
        );
        assert!(l.readable && l.items.is_empty() && l.rejected == 1, "{l:?}");
        // a listing with only undated document ids: a recognized list whose
        // every entry is an unidentified statement — readable, all rejected
        let l = parse_portal_statement_listing(
            r#"{"statements":[{"documentId":"d1"},{"documentId":"d2"}]}"#,
        );
        assert!(l.readable && l.items.is_empty() && l.rejected == 2, "{l:?}");
        // an undated entry whose PDF the listing carries anyway counts its
        // PDF, not its documentId — one statement, one failure
        let l = parse_portal_statement_listing(
            r#"{"statements":[{"statementDate":"2026-10-31","url":"/ok.pdf"},{"documentId":"d1","url":"/missing.pdf"}]}"#,
        );
        assert!(l.items.len() == 1 && l.rejected == 1, "{l:?}");
    }

    /// The statements list of the real Amazon-Visa portal (Open Bank), captured
    /// live and anonymized (two entries, one booking each): its entries name
    /// their PDF only by `documentId` (+ `version`) — no PDF links at all.
    const VISA_STATEMENTS_SAMPLE: &str = r#"{
 "metadata": {
  "hasPendingRecords": false
 },
 "elements": [
  {
   "id": "C000000000001-20240924",
   "cardContract": "C000000000001",
   "statementDate": "2024-09-24",
   "paymentDueDate": "2024-09-30",
   "countryCode": "ES",
   "billRef": "99999999999999999999",
   "postponePurchaseMonthCourse": 0,
   "statementStatus": "PAID",
   "composeData": {
    "documentDate": "2024-09-25",
    "product": "OPEN CREDIT CLIENTES",
    "iban": "DE02300209000106531065",
    "settlementPeriod": "2024-09-24",
    "cycleDate": "2024-09-24",
    "paymentDueDate": "2024-09-30",
    "customerId": "1000000001",
    "amountPayConsumptionMonth": 835.39,
    "availableBalance": 1164.61,
    "claimCommissionsDebtPositions": 0.0,
    "consumptionExpenseMonthCourse": 0.0,
    "creditLimit": 2000.0,
    "currency": "EUR",
    "currentBalance": 835.39,
    "currentCycleInterest": 0.0,
    "currentCyclePrincipal": 835.39,
    "interestResolutionAmount": 0.0,
    "outstandingBalance": 835.39,
    "overlimit": 0.0,
    "paymentAmount": 835.39,
    "paymentMethod": "FULL_ACCOUNT_BALANCE",
    "penaltyInterestAmount": 0.0,
    "previousBalance": 0.0,
    "tae": 20.12,
    "tin": 0.0,
    "totalInterest": 0,
    "totalPendingConsumptionAfterPayment": 0.0,
    "totalSettlementAmount": 835.39,
    "unpaidBalance": 0.0,
    "cmsAccountId": "9999999999999999999",
    "movementsMonth": [
     {
      "date": "2024-09-04",
      "settlementDate": "2024-09-04",
      "referenceNumber": "09999999980904000470454",
      "authorizationCode": "EB4EB9",
      "cardNumber": "9999999999999999999",
      "description": "INCENTIVE CAMPAIGN",
      "amount": 25.0,
      "planNumber": "10000",
      "transactionType": "C"
     }
    ],
    "deferPurchase": []
   },
   "documentId": "AbCdEfGhIjKlMnOpQrStUvWxYz0123456789ab",
   "version": "1"
  },
  {
   "id": "C000000000001-20241024",
   "cardContract": "C000000000001",
   "statementDate": "2024-10-24",
   "paymentDueDate": "2024-10-29",
   "countryCode": "ES",
   "billRef": "99999999999999999999",
   "postponePurchaseMonthCourse": 0,
   "statementStatus": "PAID",
   "composeData": {
    "documentDate": "2024-10-25",
    "product": "OPEN CREDIT CLIENTES",
    "iban": "DE02300209000106531065",
    "settlementPeriod": "2024-10-24",
    "cycleDate": "2024-10-24",
    "paymentDueDate": "2024-10-29",
    "customerId": "1000000001",
    "amountPayConsumptionMonth": 1660.86,
    "availableBalance": 303.76,
    "claimCommissionsDebtPositions": 0.0,
    "consumptionExpenseMonthCourse": 0.0,
    "creditLimit": 2000.0,
    "currency": "EUR",
    "currentBalance": 1660.86,
    "currentCycleInterest": 0.0,
    "currentCyclePrincipal": 1660.86,
    "interestResolutionAmount": 0.0,
    "outstandingBalance": 1660.86,
    "overlimit": 0.0,
    "paymentAmount": 1660.86,
    "paymentMethod": "FULL_ACCOUNT_BALANCE",
    "penaltyInterestAmount": 0.0,
    "previousBalance": 835.39,
    "tae": 19.42,
    "tin": 0.0,
    "totalInterest": 0,
    "totalPendingConsumptionAfterPayment": 0.0,
    "totalSettlementAmount": 1660.86,
    "unpaidBalance": 0.0,
    "cmsAccountId": "9999999999999999999",
    "movementsMonth": [
     {
      "date": "2024-09-30",
      "settlementDate": "2024-09-30",
      "referenceNumber": "09999999980930000693712",
      "authorizationCode": "92F939",
      "cardNumber": "9999999999999999999",
      "description": "PAGO DE RECIBO CONTRATO TARJETA",
      "amount": 835.39,
      "planNumber": "10000",
      "transactionType": "C"
     }
    ],
    "deferPurchase": []
   },
   "documentId": "ZzYyXwWvVuUtSrRqQpPoOnNmLk9j8i7h6g5f4e3d",
   "version": "1"
  }
 ]
}"#;

    /// The live schema pairs each entry's closing date with its own document
    /// id — the bookings inside an entry carry dates of their own, they name
    /// no entry. The listing parser reads the same response as a readable
    /// list without items: the PDFs answer a second request per statement
    /// (`documentUri`), the refs parser prepares it.
    #[test]
    fn the_open_bank_list_pairs_dates_with_document_ids() {
        let refs = parse_portal_statement_refs(VISA_STATEMENTS_SAMPLE);
        assert_eq!(
            refs,
            vec![
                PortalStatementRef {
                    date: "2024-09-24".into(),
                    document_id: "AbCdEfGhIjKlMnOpQrStUvWxYz0123456789ab".into(),
                    version: "1".into(),
                },
                PortalStatementRef {
                    date: "2024-10-24".into(),
                    document_id: "ZzYyXwWvVuUtSrRqQpPoOnNmLk9j8i7h6g5f4e3d".into(),
                    version: "1".into(),
                },
            ]
        );
        // the listing parser reads the same response as a readable list —
        // without PDF links, no items, nothing rejected
        let l = parse_portal_statement_listing(VISA_STATEMENTS_SAMPLE);
        assert!(l.readable && l.items.is_empty() && l.rejected == 0, "{l:?}");
        // an entry without a version still reads; an entry without an id
        // names no document and stays out
        let r = parse_portal_statement_refs(
            r#"[{"statementDate":"2026-10-15","documentId":"D1"},{"statementDate":"2026-11-15","documentId":""}]"#,
        );
        assert_eq!(
            r,
            vec![PortalStatementRef {
                date: "2026-10-15".into(),
                document_id: "D1".into(),
                version: String::new(),
            }]
        );
    }

    /// A closing date is only a day that exists (29.02.2024 is leap year,
    /// 29.02.2025 is not) — in the core as in the page script's date reading.
    #[test]
    fn closing_dates_accept_only_existing_calendar_days() {
        for good in ["2024-02-29", "2024-02-29T12:00:00Z", "29.02.2024"] {
            assert_eq!(closing_date(good).as_deref(), Some("2024-02-29"), "{good}");
        }
        for bad in ["2025-02-29", "29.02.2025", "31.04.2026", "2026-04-31"] {
            assert_eq!(closing_date(bad), None, "{bad}");
        }
    }

    /// The statement PDFs land with the card account of the visa login, once each.
    #[test]
    fn amazon_visa_statements_are_stored_once_with_the_card_account() {
        let dir = crate::test_support::TempDir::new("girocrab-visa-statements");
        let v = Vault::open(
            dir.join("v.db").to_string_lossy().to_string(),
            vec![5u8; 32],
        )
        .unwrap();
        let conn = v
            .add_bank_connection(
                ConnectorKind::AmazonVisa,
                String::new(),
                "Amazon Visa".into(),
            )
            .unwrap();
        let tx = r#"{"elements":[{"id":"T1","date":"2026-11-03","description":"SHOP",
            "amounts":[{"currency":"EUR","totalAmount":-19.99}]}]}"#;
        v.clone()
            .import_portal_capture(conn, vec![tx.into()], "[]".into())
            .unwrap();
        let body = r#"{"statements":[{"id":"S1","date":"2026-10-15","url":"/cards/C/statements/S1.pdf"}]}"#;
        let items = parse_portal_statement_list(body);
        let statements = items
            .iter()
            .map(|i| PortalStatement {
                date: i.date.clone(),
                bytes: format!("%PDF-1.4 {}", i.path).into_bytes(),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            v.import_portal_statements(conn, String::new(), String::new(), statements.clone())
                .unwrap(),
            1
        );
        // the same list again: stored once
        assert_eq!(
            v.import_portal_statements(conn, String::new(), String::new(), statements)
                .unwrap(),
            0
        );
        assert_eq!(
            v.portal_statement_dates(conn, String::new(), String::new())
                .unwrap(),
            vec!["2026-10-15"]
        );
        let account: Option<i64> = v
            .db()
            .unwrap()
            .query_row(
                "SELECT account_id FROM booking_documents WHERE source_url LIKE 'portal-statement:%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let ref_: String = v
            .db()
            .unwrap()
            .query_row(
                "SELECT external_ref FROM accounts WHERE id = ?1",
                [account.unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ref_, account_ref(ConnectorKind::AmazonVisa, conn));
    }
}
