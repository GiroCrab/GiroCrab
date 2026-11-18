//! MoneyMoney twins: a fetch takes over an imported MoneyMoney account by
//! giving it the bank's `external_ref` (`adopt_account`). A device that did
//! not fetch still holds that account under its `moneymoney-db:` key, so the
//! merge used to add it as a second account with every MoneyMoney booking
//! again. The merge now lets such an account take over the bank's key, and
//! twins already in a vault are folded into the bank account.
//!
//! Two accounts are twins when one is a MoneyMoney account and the other a
//! bank account of the same bank login, their IBANs match (or, without IBAN,
//! kind and account number), and most of the MoneyMoney
//! account's bookings are in the bank account (same external id or import
//! fingerprint). Issue #94: MoneyMoney keeps closed accounts with the same
//! IBAN; `mm-` ids are content hashes, so a closed account shares the few
//! bookings MoneyMoney fetched again for its successor, but not most of its
//! own. Accounts without any booking (depots) are twins when name and
//! account number match too. A MoneyMoney account linked to no login (the
//! login was added after the import, e.g. Enable Banking) can be the twin of
//! a bank account of any login, but only by IBAN and currency.

use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;

use crate::error::CoreResult;

const MM_PREFIX: &str = "moneymoney-db:";

/// What decides whether two accounts can be the same one.
struct Ident {
    id: i64,
    iban: String,
    currency: String,
    kind: String,
    number: String,
    name: String,
}

const IDENT_COLUMNS: &str = "id, upper(replace(IFNULL(iban, ''), ' ', '')),
    upper(IFNULL(currency, '')), kind, IFNULL(account_number, ''), name";

fn ident(r: &rusqlite::Row) -> rusqlite::Result<Ident> {
    Ok(Ident {
        id: r.get(0)?,
        iban: r.get(1)?,
        currency: r.get(2)?,
        kind: r.get(3)?,
        number: r.get(4)?,
        name: r.get(5)?,
    })
}

fn same_account(a: &Ident, b: &Ident) -> bool {
    if a.iban.is_empty() != b.iban.is_empty() {
        return false;
    }
    if a.iban.is_empty() {
        return a.kind == b.kind && !a.number.is_empty() && a.number == b.number;
    }
    a.iban == b.iban && (a.currency.is_empty() || b.currency.is_empty() || a.currency == b.currency)
}

/// How well the bank account `bank` (in schema `bank_schema`) holds the
/// MoneyMoney account `mm` (in `mm_schema`): the number of its bookings found
/// there, if that is most of them. Two accounts without any booking and with
/// the same name score 0.
fn twin_score(
    conn: &Connection,
    mm_schema: &str,
    mm: &Ident,
    bank_schema: &str,
    bank: &Ident,
) -> CoreResult<Option<i64>> {
    // includes soft-deleted rows: tombstones still tell which bookings the
    // MoneyMoney account had; a booking a fetch took over keeps its fingerprint.
    // `b.import_fp != ''` lets SQLite use the partial index `tx_import_fp`.
    let (total, found): (i64, i64) = conn.query_row(
        &format!(
            "SELECT count(*), IFNULL(sum(
                    EXISTS (SELECT 1 FROM {bank_schema}.transactions b
                             WHERE b.account_id = ?2 AND b.external_id = m.external_id)
                 OR (m.import_fp != '' AND EXISTS (SELECT 1 FROM {bank_schema}.transactions b
                             WHERE b.account_id = ?2 AND b.import_fp = m.import_fp
                               AND b.import_fp != ''))), 0)
               FROM {mm_schema}.transactions m WHERE m.account_id = ?1"
        ),
        params![mm.id, bank.id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if total > 0 {
        return Ok((found * 2 > total).then_some(found));
    }
    // includes soft-deleted rows
    let bank_empty: bool = conn.query_row(
        &format!(
            "SELECT NOT EXISTS (SELECT 1 FROM {bank_schema}.transactions WHERE account_id = ?1)"
        ),
        [bank.id],
        |r| r.get(0),
    )?;
    Ok((bank_empty && mm.name == bank.name).then_some(0))
}

/// The candidate with the best score, if any qualifies.
fn best<'a>(
    candidates: impl Iterator<Item = &'a Ident>,
    score: impl Fn(&Ident) -> CoreResult<Option<i64>>,
) -> CoreResult<Option<i64>> {
    let mut best: Option<(i64, i64)> = None;
    for c in candidates {
        if let Some(s) = score(c)? {
            if best.is_none_or(|(top, _)| s > top) {
                best = Some((s, c.id));
            }
        }
    }
    Ok(best.map(|(_, id)| id))
}

/// Merge: the other device's bank account `other_id` (not a MoneyMoney key,
/// no local row with its key) is the local MoneyMoney account a fetch over
/// there took over. That local account takes over the bank's key, as the
/// fetch did, and is returned.
pub(crate) fn adopt_on_merge(
    conn: &Connection,
    connection_id: i64,
    other_id: i64,
) -> CoreResult<Option<i64>> {
    let (bank, ext): (Ident, String) = conn.query_row(
        &format!("SELECT {IDENT_COLUMNS}, external_ref FROM other.accounts WHERE id = ?1"),
        [other_id],
        |r| Ok((ident(r)?, r.get(6)?)),
    )?;
    if ext.starts_with(MM_PREFIX) {
        return Ok(None);
    }
    let candidates: Vec<Ident> = conn
        .prepare(&format!(
            "SELECT {IDENT_COLUMNS}, connection_id FROM main.accounts
              WHERE (connection_id = ?1 OR connection_id IS NULL)
                AND external_ref LIKE 'moneymoney-db:%' AND deleted_ms IS NULL
              ORDER BY id"
        ))?
        .query_map([connection_id], |r| {
            Ok((ident(r)?, r.get::<_, Option<i64>>(6)?))
        })?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|(mm, login)| login.is_some() || !mm.iban.is_empty())
        .map(|(mm, _)| mm)
        .collect();
    let found = best(
        candidates.iter().filter(|mm| same_account(mm, &bank)),
        |mm| twin_score(conn, "main", mm, "other", &bank),
    )?;
    if let Some(id) = found {
        // linked to the login too, as `adopt_account` does
        conn.execute(
            "UPDATE accounts SET connection_id = ?3, external_ref = ?2 WHERE id = ?1",
            params![id, ext, connection_id],
        )?;
    }
    Ok(found)
}

/// The live bank account the MoneyMoney account `mm` (on login
/// `connection_id`, or none) is a twin of.
fn bank_twin(conn: &Connection, mm: &Ident, connection_id: Option<i64>) -> CoreResult<Option<i64>> {
    // unlinked: any login, but only by IBAN
    if connection_id.is_none() && mm.iban.is_empty() {
        return Ok(None);
    }
    let banks: Vec<Ident> = conn
        .prepare(&format!(
            "SELECT {IDENT_COLUMNS} FROM main.accounts
              WHERE connection_id IS NOT NULL AND (?1 IS NULL OR connection_id = ?1)
                AND external_ref NOT LIKE 'moneymoney-db:%' AND deleted_ms IS NULL
              ORDER BY id"
        ))?
        .query_map([connection_id], ident)?
        .collect::<Result<_, _>>()?;
    best(banks.iter().filter(|b| same_account(mm, b)), |bank| {
        twin_score(conn, "main", mm, "main", bank)
    })
}

/// Import: the account a MoneyMoney key found is a folded twin (deleted,
/// its bank account live). The import then goes to the bank account
/// instead of reviving the twin.
pub(crate) fn folded_into(conn: &Connection, id: i64) -> CoreResult<Option<i64>> {
    let row: Option<(Ident, Option<i64>)> = conn
        .query_row(
            // includes soft-deleted rows
            &format!(
                "SELECT {IDENT_COLUMNS}, connection_id FROM main.accounts
                  WHERE id = ?1 AND deleted_ms IS NOT NULL
                    AND external_ref LIKE 'moneymoney-db:%'"
            ),
            [id],
            |r| Ok((ident(r)?, r.get(6)?)),
        )
        .optional()?;
    match row {
        Some((mm, connection_id)) => bank_twin(conn, &mm, connection_id),
        None => Ok(None),
    }
}

/// [`folded_into`] with the answers kept in `seen`: a merge asks once per account.
pub(crate) fn folded_account(
    conn: &Connection,
    account: i64,
    seen: &mut HashMap<i64, Option<i64>>,
) -> CoreResult<Option<i64>> {
    if let Some(bank) = seen.get(&account) {
        return Ok(*bank);
    }
    let bank = folded_into(conn, account)?;
    seen.insert(account, bank);
    Ok(bank)
}

/// The booking that stands for `booking` once its account is a folded twin
/// (deleted, its bank account live): the bank account's copy with the same
/// external id, else its partner by fingerprint. Identical bookings share a
/// fingerprint; like `merge_bookings`, the ones without a key match on the
/// other side are paired in external-id order, the i-th with the i-th.
/// `None`: `booking` stands for itself (no folded twin, or no copy). Linked
/// receipts (#590) follow it, so a reference made on a device that had not
/// folded yet still reaches the live booking. The answer only depends on
/// the bookings of the two accounts, which are the same on every device
/// after a merge.
pub(crate) fn folded_booking(
    conn: &Connection,
    booking: i64,
    seen: &mut HashMap<i64, Option<i64>>,
) -> CoreResult<Option<i64>> {
    type Text = (String, String, i64, String, String, String);
    const TEXT: &str =
        "import_fp, booking_date, amount_cents, counterparty_iban, counterparty, purpose";
    let text = |r: &rusqlite::Row, at: usize| -> rusqlite::Result<Text> {
        Ok((
            r.get(at)?,
            r.get(at + 1)?,
            r.get(at + 2)?,
            r.get(at + 3)?,
            r.get(at + 4)?,
            r.get(at + 5)?,
        ))
    };
    let fingerprint = |(fp, date, cents, iban, name, purpose): &Text| {
        super::bookings::match_fingerprint(fp.clone(), date, *cents, iban, name, purpose)
    };
    let row: Option<(i64, String, Text)> = conn
        .query_row(
            // includes soft-deleted rows: the twin's bookings went with it
            &format!("SELECT account_id, external_id, {TEXT} FROM transactions WHERE id = ?1"),
            [booking],
            |r| Ok((r.get(0)?, r.get(1)?, text(r, 2)?)),
        )
        .optional()?;
    let Some((account, ext, own)) = row else {
        return Ok(None);
    };
    let Some(bank) = folded_account(conn, account, seen)? else {
        return Ok(None);
    };
    let by_key: Option<i64> = conn
        .query_row(
            // includes soft-deleted rows: a deleted copy keeps the reference
            "SELECT id FROM transactions WHERE account_id = ?1 AND external_id = ?2",
            params![bank, ext],
            |r| r.get(0),
        )
        .optional()?;
    if by_key.is_some() {
        return Ok(by_key);
    }
    let wanted = fingerprint(&own);
    // bookings of `of` with this fingerprint that `other` does not hold under
    // the same external id, in external-id order
    let unpaired = |of: i64, other: i64| -> CoreResult<Vec<i64>> {
        Ok(conn
            .prepare(&format!(
                // includes soft-deleted rows
                "SELECT t.id, {TEXT} FROM transactions t
                  WHERE t.account_id = ?1
                    AND (t.import_fp = ?3 OR (t.booking_date = ?4 AND t.amount_cents = ?5))
                    AND NOT EXISTS (SELECT 1 FROM transactions x
                                     WHERE x.account_id = ?2 AND x.external_id = t.external_id)
                  ORDER BY t.external_id, t.id"
            ))?
            .query_map(params![of, other, wanted, own.1, own.2], |r| {
                Ok((r.get::<_, i64>(0)?, text(r, 1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|(_, t)| fingerprint(t) == wanted)
            .map(|(id, _)| id)
            .collect())
    };
    let Some(rank) = unpaired(account, bank)?
        .iter()
        .position(|id| *id == booking)
    else {
        return Ok(None);
    };
    Ok(unpaired(bank, account)?.get(rank).copied())
}

/// Folds every live MoneyMoney twin into its bank account: its bookings
/// merge into the bank account the way a device merge does (matched by
/// external id, else import fingerprint; newest edit per field wins), rules,
/// documents and loan links move over, and the twin is soft-deleted, so the other
/// devices drop it on merge. Runs outside a transaction (it attaches a
/// scratch database); each twin is folded all or nothing. Returns the count.
pub(crate) fn fold_twins(conn: &Connection) -> CoreResult<u32> {
    let twins: Vec<(Ident, Option<i64>)> = conn
        .prepare(&format!(
            "SELECT {IDENT_COLUMNS}, connection_id FROM main.accounts
              WHERE external_ref LIKE 'moneymoney-db:%' AND deleted_ms IS NULL
              ORDER BY id"
        ))?
        .query_map([], |r| Ok((ident(r)?, r.get(6)?)))?
        .collect::<Result<_, _>>()?;
    let mut pairs = Vec::new();
    for (mm, connection_id) in &twins {
        if let Some(bank) = bank_twin(conn, mm, *connection_id)? {
            pairs.push((mm.id, bank));
        }
    }
    if pairs.is_empty() {
        return Ok(0);
    }
    conn.execute_batch("ATTACH DATABASE ':memory:' AS other KEY ''")?;
    let res = pairs
        .iter()
        .try_for_each(|&(twin, bank)| fold(conn, twin, bank));
    let detached = conn.execute_batch("DETACH DATABASE other");
    res?;
    detached?;
    Ok(pairs.len() as u32)
}

fn fold(conn: &Connection, twin: i64, bank: i64) -> CoreResult<()> {
    conn.execute_batch("SAVEPOINT fold_twin")?;
    match fold_inner(conn, twin, bank) {
        Ok(()) => Ok(conn.execute_batch("RELEASE fold_twin")?),
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK TO fold_twin; RELEASE fold_twin");
            Err(e)
        }
    }
}

fn fold_inner(conn: &Connection, twin: i64, bank: i64) -> CoreResult<()> {
    // the twin as "the other device": its account row, bookings and splits
    conn.execute_batch(&format!(
        "DROP TABLE IF EXISTS other.accounts;
         DROP TABLE IF EXISTS other.transactions;
         DROP TABLE IF EXISTS other.transaction_splits;
         DROP TABLE IF EXISTS other.tags;
         DROP TABLE IF EXISTS other.transaction_tags;
         CREATE TABLE other.accounts AS SELECT * FROM main.accounts WHERE id = {twin};
         CREATE TABLE other.transactions AS SELECT * FROM main.transactions WHERE account_id = {twin};
         CREATE TABLE other.transaction_splits AS SELECT * FROM main.transaction_splits
             WHERE transaction_id IN (SELECT id FROM other.transactions);
         CREATE TABLE other.tags AS SELECT * FROM main.tags;
         CREATE TABLE other.transaction_tags AS SELECT * FROM main.transaction_tags
             WHERE transaction_id IN (SELECT id FROM other.transactions);"
    ))?;
    // same vault: categories and rules map onto themselves
    let identity = |table: &str| -> CoreResult<HashMap<i64, i64>> {
        Ok(conn
            .prepare(&format!(
                "SELECT id FROM main.{table} WHERE deleted_ms IS NULL"
            ))?
            .query_map([], |r| r.get(0))?
            .map(|id| id.map(|id: i64| (id, id)))
            .collect::<Result<_, _>>()?)
    };
    let cat_map = identity("categories")?;
    let rule_map = identity("rules")?;
    let acc_map = HashMap::from([(twin, bank)]);
    let mut stats = crate::vault::MergeStats::default();
    // twin booking -> its copy in the bank account, as the merge paired them
    let booking_map = super::merge_bookings(conn, &acc_map, &cat_map, &rule_map, &mut stats)?;
    // stamped: a device still holding the rule on the twin's key loses
    conn.execute(
        "UPDATE rules SET account_id = ?2, updated_ms = max(updated_ms + 1, ?3) WHERE account_id = ?1",
        params![twin, bank, crate::dates::now_ms()],
    )?;
    conn.execute(
        "UPDATE OR IGNORE rule_accounts SET account_id = ?2 WHERE account_id = ?1",
        params![twin, bank],
    )?;
    conn.execute("DELETE FROM rule_accounts WHERE account_id = ?1", [twin])?;
    // documents and manual loan links follow each booking to the bank
    // account's copy (same external id, else import fingerprint)
    let counterpart = "(SELECT b.id FROM transactions b, transactions m
                         WHERE m.id = {col} AND b.account_id = ?2
                           AND (b.external_id = m.external_id
                                OR (m.import_fp != '' AND b.import_fp = m.import_fp))
                         ORDER BY b.external_id = m.external_id DESC, b.deleted_ms IS NOT NULL, b.id
                         LIMIT 1)";
    conn.execute(
        &format!(
            "UPDATE booking_documents SET booking_id = IFNULL({}, booking_id)
              WHERE booking_id IN (SELECT id FROM transactions WHERE account_id = ?1)",
            counterpart.replace("{col}", "booking_documents.booking_id")
        ),
        params![twin, bank],
    )?;
    conn.execute(
        "UPDATE booking_documents SET account_id = ?2 WHERE account_id = ?1",
        params![twin, bank],
    )?;
    // linked receipts (#590) follow the exact pairing of the merge above.
    // Stamped, like the rules: a device still holding the reference on the
    // twin's booking loses and takes over this booking through its own
    // booking map, instead of pairing identical bookings anew.
    let now = crate::dates::now_ms();
    let rev = crate::document_links::new_rev();
    let linked: Vec<i64> = conn
        .prepare(
            "SELECT DISTINCT booking_id FROM document_links
              WHERE booking_id IN (SELECT id FROM transactions WHERE account_id = ?1)",
        )?
        .query_map([twin], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    for old in linked {
        if let Some(copy) = booking_map.get(&old) {
            conn.execute(
                "UPDATE document_links
                    SET booking_id = ?2, updated_ms = max(updated_ms + 1, ?3), rev = ?4
                  WHERE booking_id = ?1",
                params![old, copy, now, rev],
            )?;
        }
    }
    conn.execute(
        "UPDATE document_links
            SET account_id = ?2, updated_ms = max(updated_ms + 1, ?3), rev = ?4
          WHERE account_id = ?1 AND deleted_ms IS NULL",
        params![twin, bank, now, rev],
    )?;
    conn.execute(
        &format!(
            "UPDATE OR IGNORE loan_links SET transaction_id = {}
              WHERE transaction_id IN (SELECT id FROM transactions WHERE account_id = ?1)
                AND {} IS NOT NULL",
            counterpart.replace("{col}", "loan_links.transaction_id"),
            counterpart.replace("{col}", "loan_links.transaction_id")
        ),
        params![twin, bank],
    )?;
    // the imported depot history belongs to the bank's account from now on (#578)
    crate::depot_values::move_account(conn, twin, bank)?;
    super::soft_delete_account(conn, twin)?;
    Ok(())
}
