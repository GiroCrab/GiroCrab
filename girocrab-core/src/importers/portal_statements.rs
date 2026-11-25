//! Statement PDFs a card portal hands out next to its bookings (Amex): the app
//! downloads them in the signed-in portal page and stores them here, each at
//! most once, as a statement of the card's account. A statement is known by its
//! account and closing date: the portal's own statement ids change with every
//! login.

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

/// The statements list of the Amazon-Visa portal (Open Bank), schema-agnostic:
/// its JSON schema is undocumented and changes, so every object that carries a
/// closing date and a PDF link is an entry (`{…, "date": "2026-10-15", "url":
/// "…x.pdf", …}`) — the date sits on the list item, the link often one nesting
/// level deeper (`files: [{"url": "…x.pdf"}]`). A date is always paired with a
/// PDF of the same entry, never with the link of a neighbour. Duplicate
/// `(date, path)` pairs and entries without a complete pair are left out.
pub fn parse_portal_statement_list(body: &str) -> Vec<PortalStatementItem> {
    /// First closing date of an object's direct fields — a field whose (separator-free,
    /// lower-cased) name names a statement/closing date wins over any other date-like
    /// field, whatever the map order (`serde_json` maps iterate alphabetically, e.g. a
    /// `created` would otherwise win over `end_date` or `statementDate`).
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
            let name: String = k
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .map(|c| c.to_ascii_lowercase())
                .collect();
            let prio = NAMED.iter().position(|n| *n == name).unwrap_or(usize::MAX);
            if best.as_ref().is_none_or(|(p, _)| prio < *p) {
                best = Some((prio, date));
            }
        }
        best.map(|(_, date)| date)
    }
    /// First PDF link within `v`, without crossing into an object that
    /// carries its own closing date — that is another list entry, so its PDF
    /// belongs to that entry, not to ours.
    fn pdf_in(v: &serde_json::Value) -> Option<String> {
        match v {
            serde_json::Value::Object(m) if own_date(m).is_none() => m
                .values()
                .filter_map(|s| s.as_str())
                .find_map(pdf_link)
                .or_else(|| m.values().find_map(pdf_in)),
            serde_json::Value::Array(a) => a.iter().filter_map(pdf_in).next(),
            _ => None,
        }
    }
    /// One list entry: its closing date is a direct field of the entry, the
    /// PDF link comes from the entry itself (`files` array included) — never
    /// from another entry. An entry without a PDF and a PDF without a date
    /// emit nothing instead of pairing with each other or with a neighbour.
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
    let Some(v) = serde_json::from_str::<serde_json::Value>(body).ok() else {
        return Vec::new();
    };
    let mut out: Vec<(String, String)> = Vec::new();
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
    collect(&v, &mut out);
    out.into_iter()
        .map(|(date, path)| PortalStatementItem { date, path })
        .collect()
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

/// A path or URL ending in `.pdf` (the statement PDFs of the portals); anything
/// else (ids, tokens, HTML) is no link.
pub fn pdf_link(s: &str) -> Option<String> {
    let t = s.trim();
    (t.len() > 4 && t.to_lowercase().ends_with(".pdf") && !t.contains(char::is_whitespace))
        .then(|| t.to_string())
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
