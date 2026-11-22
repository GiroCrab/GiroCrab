//! Explicitly reapplies a bank connection's document rules to stored PDFs.

use std::collections::HashSet;

use rusqlite::{Connection, params};

use crate::documents::{account_document_type, extract_pdf_text};
use crate::error::CoreResult;
use crate::models::{AccountKind, ConnectorKind};

use super::{RemoteAccount, comdirect_postbox, document_matching, persist, trade_republic};

fn is_comdirect(key: &str) -> bool {
    key == ConnectorKind::Comdirect.as_str() || persist::is_comdirect_fints(key)
}

pub(crate) fn resort(db: &Connection, connection_id: i64, key: &str) -> CoreResult<(u32, u32)> {
    let Some(kind) = ConnectorKind::from_str(key) else {
        return Ok((0, 0));
    };
    let comdirect = is_comdirect(key);
    let source_prefix = match kind {
        ConnectorKind::TradeRepublic => "https://".to_owned(),
        ConnectorKind::Fnz | ConnectorKind::Quirion => format!("{key}:"),
        _ if comdirect => "comdirect:postbox:".to_owned(),
        _ => return Ok((0, 0)),
    };
    // Deleting an old login doesn't establish the origin of its accountless PDFs.
    let claim_orphans = comdirect && sole_comdirect_login(db, connection_id)?;
    let mut documents = db.prepare(
        "SELECT d.id, d.title, d.doc_type, d.account_id, d.booking_id, d.source_url
           FROM booking_documents d
           LEFT JOIN accounts a ON a.id = d.account_id
           LEFT JOIN transactions t ON t.id = d.booking_id
           LEFT JOIN accounts b ON b.id = t.account_id
          WHERE (a.connection_id = ?1 AND a.deleted_ms IS NULL AND a.archived_unix IS NULL
                 OR (d.account_id IS NULL AND
                     (b.connection_id = ?1 OR (?2 AND d.booking_id IS NULL))))
            AND (d.booking_id IS NULL OR b.connection_id = ?1)",
    )?;
    let docs = documents
        .query_map(params![connection_id, claim_orphans], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, String>(5)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let accounts = accounts(db, connection_id)?;
    let account_refs: Vec<_> = accounts.iter().map(|(_, account, _)| account).collect();
    let rules = document_matching::rules_for_connection(db, connection_id, kind)?;
    let mut pdf = db.prepare("SELECT pdf_bytes FROM booking_documents WHERE id = ?1")?;
    let mut update =
        db.prepare("UPDATE booking_documents SET doc_type = ?1, account_id = ?2 WHERE id = ?3")?;
    let mut resorted = 0;
    let mut unlinked = HashSet::new();
    for (id, title, previous_type, previous_account, booking, source) in docs {
        if !source.starts_with(&source_prefix) {
            continue;
        }
        let (doc_type, account_ref) = if kind == ConnectorKind::TradeRepublic {
            trade_republic::stored_document_filing(&title, &previous_type)
        } else {
            let doc_type = if previous_type == "postbox" {
                account_document_type(&title)
            } else {
                &previous_type
            };
            let account_ref = if comdirect {
                let bytes: Vec<u8> = pdf.query_row([id], |row| row.get(0))?;
                comdirect_postbox::assign_account(&title, &extract_pdf_text(&bytes), &account_refs)
                    .map(|account| account.external_ref.as_str())
            } else {
                None
            };
            (doc_type, account_ref)
        };
        let account = account_ref
            .and_then(|key| {
                accounts
                    .iter()
                    .find(|(_, account, active)| *active && account.external_ref == key)
            })
            .map(|(id, _, _)| *id)
            .or(previous_account);
        if doc_type != previous_type || account != previous_account {
            resorted += update.execute(params![doc_type, account, id])? as u32;
        }
        if rules.is_some() && booking.is_none() && account.is_some() {
            unlinked.insert(id);
        }
    }
    let linked = if let Some(rules) = rules
        && !unlinked.is_empty()
    {
        document_matching::link_unlinked_by_content(
            db,
            connection_id,
            &rules,
            &unlinked,
            &HashSet::new(),
        )?
    } else {
        0
    };
    Ok((resorted, linked))
}

fn sole_comdirect_login(db: &Connection, connection_id: i64) -> CoreResult<bool> {
    let mut query = db.prepare("SELECT id, connector FROM bank_connections")?;
    let rows = query.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (id, key) = row?;
        if id != connection_id && is_comdirect(&key) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn accounts(db: &Connection, connection_id: i64) -> CoreResult<Vec<(i64, RemoteAccount, bool)>> {
    let mut query = db.prepare(
        "SELECT id, external_ref, kind, iban, account_number, archived_unix IS NULL
           FROM accounts WHERE connection_id = ?1 AND deleted_ms IS NULL",
    )?;
    let rows = query.query_map([connection_id], |row| {
        Ok((
            row.get(0)?,
            RemoteAccount {
                external_ref: row.get(1)?,
                kind: AccountKind::from_str(&row.get::<_, String>(2)?),
                iban: row.get(3)?,
                account_number: row.get(4)?,
                name: String::new(),
                owner: String::new(),
                currency: String::new(),
                balance_cents: 0,
                balance_date: String::new(),
            },
            row.get::<_, bool>(5)?,
        ))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}
