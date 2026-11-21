//! Possible duplicates between a bank booking and an imported MoneyMoney row
//! that the automatic matching kept apart (different text, a day apart, a
//! duplicate from an older version). The user sees the pairs and merges or
//! dismisses each one; nothing is merged automatically.
//!
//! A pair: same account and amount, booking dates at most [`WINDOW_DAYS`]
//! apart, one row from the bank (not `mm-…`, not a reservation) and one
//! imported from MoneyMoney and not yet taken over (`mm-…`). Merging keeps the
//! imported row (its category, note and check mark) under the bank's id and
//! with the bank's texts – later fetches deliver them anyway – and removes the
//! bank row, as the automatic repair does (`merge_synced_twin`).

use rusqlite::{Connection, OptionalExtension, params};

use super::Vault;
use crate::error::{CoreError, CoreResult};
use crate::i18n::tr;

/// Days between the two booking dates of a pair.
const WINDOW_DAYS: i64 = 3;
/// Setting with the dismissed pairs, one `bank_external_id\tmm_external_id` per line.
const DISMISSED_KEY: &str = "duplicates.dismissed";

/// One side of a possible duplicate.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DuplicateSide {
    pub booking_id: i64,
    pub booking_date: String,
    pub counterparty: String,
    pub purpose: String,
    pub category_name: String,
    /// The category was assigned by hand (or came from MoneyMoney): it wins
    /// over a rule's on merge.
    pub category_by_hand: bool,
    pub note: String,
    pub tags: Vec<String>,
}

/// A bank booking and an imported MoneyMoney row that may be the same booking.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PossibleDuplicate {
    pub account_id: i64,
    pub account_name: String,
    /// The account's currency (ISO code), for formatting `amount_cents`.
    pub currency: String,
    pub amount_cents: i64,
    pub bank: DuplicateSide,
    pub imported: DuplicateSide,
}

/// Bank rows with what a merge could not carry over (splits, documents, loan
/// links) are left out: merging would lose them. Tags carry over.
const PAIRS_SQL: &str = "
    SELECT a.id, a.name, a.currency, b.amount_cents,
           b.id, b.booking_date, b.counterparty, b.purpose, IFNULL(bc.name, ''),
           b.category_id IS NOT NULL AND b.category_source = 'manual', b.note,
           (SELECT group_concat(g.name, char(31)) FROM transaction_tags x
              JOIN tags g ON g.id = x.tag_id AND g.deleted_ms IS NULL WHERE x.transaction_id = b.id),
           b.external_id,
           m.id, m.booking_date, m.counterparty, m.purpose, IFNULL(mc.name, ''),
           m.category_id IS NOT NULL AND m.category_source = 'manual', m.note,
           (SELECT group_concat(g.name, char(31)) FROM transaction_tags x
              JOIN tags g ON g.id = x.tag_id AND g.deleted_ms IS NULL WHERE x.transaction_id = m.id),
           m.external_id
      FROM transactions b
      JOIN accounts a ON a.id = b.account_id AND a.deleted_ms IS NULL
      JOIN transactions m ON m.account_id = b.account_id
                         AND m.amount_cents = b.amount_cents
                         AND m.id != b.id
                         AND m.deleted_ms IS NULL
                         AND m.external_id LIKE 'mm-%'
                         AND m.external_id NOT LIKE 'mm-switch-pending-%'
                         -- a range on the date, not abs(julianday(…)): the
                         -- (account, amount, date) lookup can use the index
                         AND m.booking_date BETWEEN date(b.booking_date, '-' || ?1 || ' days')
                                                AND date(b.booking_date, '+' || ?1 || ' days')
      LEFT JOIN categories bc ON bc.id = b.category_id
      LEFT JOIN categories mc ON mc.id = m.category_id
     WHERE b.deleted_ms IS NULL
       AND b.external_id NOT LIKE 'mm-%'
       -- a row that came from MoneyMoney and was taken over by the bank already
       -- carries its import key: it is the result of a match, not a candidate
       AND b.import_fp = ''
       AND b.external_id NOT LIKE 'pending-%'
       AND NOT EXISTS (SELECT 1 FROM transaction_splits s WHERE s.transaction_id = b.id)
       AND NOT EXISTS (SELECT 1 FROM booking_documents d WHERE d.booking_id = b.id)
       AND NOT EXISTS (SELECT 1 FROM loan_links l WHERE l.transaction_id = b.id)
     ORDER BY b.booking_date DESC, b.id, abs(julianday(m.booking_date) - julianday(b.booking_date)), m.id";

fn dismissed(conn: &Connection) -> CoreResult<Vec<String>> {
    Ok(Vault::setting_inner(conn, DISMISSED_KEY)?
        .map(|v| v.lines().map(str::to_string).collect())
        .unwrap_or_default())
}

/// Bank ids are unique only per account (`UNIQUE (account_id, external_id)`),
/// so the key names the account too.
fn pair_key(account_id: i64, bank_ext: &str, mm_ext: &str) -> String {
    format!("{account_id}\t{bank_ext}\t{mm_ext}")
}

/// The key the first version stored, without the account – still honoured.
fn legacy_pair_key(bank_ext: &str, mm_ext: &str) -> String {
    format!("{bank_ext}\t{mm_ext}")
}

fn possible_duplicates(conn: &Connection) -> CoreResult<Vec<PossibleDuplicate>> {
    let dismissed = dismissed(conn)?;
    let mut st = conn.prepare(PAIRS_SQL)?;
    let rows = st.query_map([WINDOW_DAYS], |r| {
        let side = |o: usize| -> rusqlite::Result<DuplicateSide> {
            Ok(DuplicateSide {
                booking_id: r.get(o)?,
                booking_date: r.get(o + 1)?,
                counterparty: r.get(o + 2)?,
                purpose: r.get(o + 3)?,
                category_name: r.get(o + 4)?,
                category_by_hand: r.get(o + 5)?,
                note: r.get(o + 6)?,
                tags: r
                    .get::<_, Option<String>>(o + 7)?
                    .map(|t| t.split('\u{1f}').map(str::to_string).collect())
                    .unwrap_or_default(),
            })
        };
        let pair = PossibleDuplicate {
            account_id: r.get(0)?,
            account_name: r.get(1)?,
            currency: r.get(2)?,
            amount_cents: r.get(3)?,
            bank: side(4)?,
            imported: side(13)?,
        };
        let (bank_ext, mm_ext): (String, String) = (r.get(12)?, r.get(21)?);
        let keys = [
            pair_key(pair.account_id, &bank_ext, &mm_ext),
            legacy_pair_key(&bank_ext, &mm_ext),
        ];
        Ok((pair, keys))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (pair, keys) = row?;
        if !keys.iter().any(|k| dismissed.contains(k)) {
            out.push(pair);
        }
    }
    Ok(out)
}

/// Merges one pair. The booking lives on in the imported row's place under
/// the bank's external id (the next fetch reports it by that id); the bank
/// row is removed. `keep_bank` decides the texts. Nothing the user entered is
/// lost: a category assigned by hand beats one from a rule, whichever side it
/// is on (a tie goes to the kept side); two different notes are joined; tags
/// of both stay. Without a category the rules run. Checks again that the two
/// still form a pair: the list may be stale.
fn merge(conn: &Connection, bank_id: i64, imported_id: i64, keep_bank: bool) -> CoreResult<()> {
    let still_a_pair = possible_duplicates(conn)?
        .iter()
        .any(|p| p.bank.booking_id == bank_id && p.imported.booking_id == imported_id);
    if !still_a_pair {
        return Err(CoreError::invalid(tr(
            "Die beiden Buchungen lassen sich nicht (mehr) zusammenführen.",
        )));
    }
    struct Side {
        ext: String,
        cat: Option<i64>,
        src: String,
        rule: Option<i64>,
        note: String,
        checked: i64,
    }
    impl Side {
        /// manual (by hand or from MoneyMoney) > rule > none
        fn rank(&self) -> u8 {
            match (self.cat, self.src.as_str()) {
                (None, _) => 0,
                (Some(_), "manual") => 2,
                _ => 1,
            }
        }
    }
    let read = |id: i64| -> CoreResult<Side> {
        Ok(conn.query_row(
            "SELECT external_id, category_id, category_source, rule_id, note, checked FROM transactions WHERE id = ?1",
            [id],
            |r| {
                Ok(Side {
                    ext: r.get(0)?,
                    cat: r.get(1)?,
                    src: r.get(2)?,
                    rule: r.get(3)?,
                    note: r.get(4)?,
                    checked: r.get(5)?,
                })
            },
        )?)
    };
    let bank = read(bank_id)?;
    let mm = read(imported_id)?;
    let (kept, other) = if keep_bank {
        (&bank, &mm)
    } else {
        (&mm, &bank)
    };
    let cat_from = if other.rank() > kept.rank() {
        other
    } else {
        kept
    };
    let note = match (kept.note.trim(), other.note.trim()) {
        (k, o) if o.is_empty() || k == o => k.to_string(),
        ("", o) => o.to_string(),
        (k, o) => format!("{k} · {o}"),
    };
    let now = crate::dates::now_ms();
    let tx = conn.unchecked_transaction()?;
    // (account, external_id) is unique: the bank row's id moves aside first
    tx.execute(
        "UPDATE transactions SET external_id = external_id || '#merged-' || id WHERE id = ?1",
        [bank_id],
    )?;
    // Each text field: the kept entry's when it has one, else the other's
    // (the bank's wording is what the next fetch reports, MoneyMoney's may be
    // a raw format like "Akto:… ABZR: …"). The salary override likewise.
    let pick = |f: &str| {
        format!(
            "{f} = CASE WHEN ?4 THEN (CASE WHEN b.{f} != '' THEN b.{f} ELSE transactions.{f} END)
                        ELSE (CASE WHEN transactions.{f} != '' THEN transactions.{f} ELSE b.{f} END) END"
        )
    };
    tx.execute(
        &format!(
            "UPDATE transactions SET external_id = ?2, {}, {}, {}, {},
                    salary_flag = CASE WHEN ?4 THEN COALESCE(b.salary_flag, transactions.salary_flag)
                                       ELSE COALESCE(transactions.salary_flag, b.salary_flag) END
               FROM (SELECT counterparty, purpose, counterparty_iban, booking_text, salary_flag
                       FROM transactions WHERE id = ?3) AS b
              WHERE transactions.id = ?1",
            pick("counterparty"),
            pick("purpose"),
            pick("counterparty_iban"),
            pick("booking_text"),
        ),
        params![imported_id, bank.ext, bank_id, keep_bank],
    )?;
    // tags of both: the kept booking gets the union
    tx.execute(
        "INSERT OR IGNORE INTO transaction_tags(transaction_id, tag_id)
         SELECT ?1, tag_id FROM transaction_tags WHERE transaction_id = ?2",
        params![imported_id, bank_id],
    )?;
    // Every merged field gets the merge time as its stamp, also where the value
    // stayed the imported one: sync keeps the newer stamp per field, and a
    // device still holding the bank row must not win back its own value.
    tx.execute(
        "UPDATE transactions
            SET category_id = ?2, category_source = ?3, rule_id = ?4, note = ?5,
                checked = MAX(checked, ?6),
                category_ms = ?7, note_ms = ?7, checked_ms = ?7, tags_ms = ?7, text_ms = ?7
          WHERE id = ?1",
        params![
            imported_id,
            cat_from.cat,
            cat_from.src,
            cat_from.rule,
            note,
            bank.checked,
            now
        ],
    )?;
    // the removed copy keeps the imported row's old id as a tombstone, so
    // another device still holding that row drops it on merge
    tx.execute(
        "UPDATE transactions SET external_id = ?2 WHERE id = ?1",
        params![bank_id, mm.ext],
    )?;
    crate::sync_merge::soft_delete_booking(&tx, bank_id)?;
    // still without a category: the rules may know the kept text
    Vault::apply_rules_to(&tx, &std::collections::HashSet::from([imported_id]))?;
    tx.commit()?;
    Ok(())
}

#[uniffi::export]
impl Vault {
    /// Pairs of a bank booking and an imported MoneyMoney row that may be the
    /// same booking (same account and amount, at most three days apart),
    /// newest first. Dismissed pairs are left out. A booking can appear in
    /// several pairs; merging one removes the others it was part of.
    pub fn possible_duplicates(&self) -> Result<Vec<PossibleDuplicate>, CoreError> {
        possible_duplicates(&*self.db()?)
    }

    /// Merges a pair from [`Vault::possible_duplicates`] into one booking
    /// under the bank's id. `keep_bank`: it keeps the bank entry's texts,
    /// category and note, else the imported MoneyMoney entry's; the other
    /// fills in what is missing.
    pub fn merge_possible_duplicate(
        &self,
        bank_booking_id: i64,
        imported_booking_id: i64,
        keep_bank: bool,
    ) -> Result<(), CoreError> {
        merge(
            &*self.db()?,
            bank_booking_id,
            imported_booking_id,
            keep_bank,
        )
    }

    /// "Kein Doppel": the pair is no longer offered.
    pub fn dismiss_possible_duplicate(
        &self,
        bank_booking_id: i64,
        imported_booking_id: i64,
    ) -> Result<(), CoreError> {
        let db = self.db()?;
        let ext = |id: i64| -> CoreResult<Option<(i64, String)>> {
            Ok(db
                .query_row(
                    "SELECT account_id, external_id FROM transactions WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?)
        };
        let (Some((account, b)), Some((_, m))) = (ext(bank_booking_id)?, ext(imported_booking_id)?)
        else {
            return Ok(());
        };
        let mut list = dismissed(&db)?;
        let key = pair_key(account, &b, &m);
        if !list.contains(&key) {
            list.push(key);
            Vault::set_setting_inner(&db, DISMISSED_KEY, &list.join("\n"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
