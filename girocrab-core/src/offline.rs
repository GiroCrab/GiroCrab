//! Offline accounts and manual bookings (GiroCrab/GiroCrab#679, #81).
//!
//! An offline account has no bank connection (cash, a private loan given, a
//! second account kept by hand). Its bookings are typed in, edited and deleted
//! by the user, and its balance is `opening balance + Σ bookings`, the same
//! computed balance as imported accounts (`opening_balance.rs`). Everything
//! else (categories, rules, tags, sync merge, totals, history) already treats
//! these accounts like any other, because a manual booking is an ordinary row
//! of `transactions` with an `external_id` of its own (`manual-<uuid>`):
//! edits carry the per-field stamps and deletions are soft (`deleted_ms`), so
//! the vault merge converges as it does for bank bookings.
//!
//! Depots and loans have their own flows (Sachwerte, `loans.rs`) and are never
//! offline accounts in this sense; accounts of a bank connection are synced and
//! stay read-only.

use rusqlite::{Connection, OptionalExtension, params};

use crate::connectors::group_for;
use crate::dates::{now_ms, today_iso as today};
use crate::error::{CoreError, CoreResult};
use crate::i18n::{tr, trf};
use crate::models::AccountKind;
use crate::vault::Vault;

const EXTERNAL_REF_PREFIX: &str = "offline-";
const BOOKING_ID_PREFIX: &str = "manual-";
const MAX_NAME_CHARS: usize = 100;
const MAX_TEXT_CHARS: usize = 500;
const MAX_NOTE_CHARS: usize = 5000;

/// Accounts that take manual bookings (SQL condition on `a`).
pub(crate) const OFFLINE_SQL: &str =
    "a.deleted_ms IS NULL AND a.connection_id IS NULL AND a.kind NOT IN ('brokerage', 'loan')";

/// What the booking editor sends: a new booking or the new values of one.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ManualBookingSetup {
    /// `YYYY-MM-DD`.
    pub booking_date: String,
    /// Negative = money out. Never 0.
    pub amount_cents: i64,
    pub counterparty: String,
    pub purpose: String,
    /// `None` = uncategorised (the rules may still pick one).
    pub category_id: Option<i64>,
    pub note: String,
}

fn not_found(what: String) -> CoreError {
    CoreError::NotFound { what }
}

fn clean(text: &str, max: usize) -> CoreResult<String> {
    let t = text.trim();
    if t.chars().count() > max {
        return Err(CoreError::invalid(trf(
            "Der Text ist zu lang (höchstens {0} Zeichen).",
            &[&max],
        )));
    }
    Ok(t.to_string())
}

fn parse_date(date: &str) -> CoreResult<String> {
    jiff::civil::Date::strptime("%Y-%m-%d", date.trim())
        .map(crate::dates::iso)
        .map_err(|_| CoreError::invalid(trf("Ungültiges Datum: {0}", &[&date.trim()])))
}

fn require_offline(conn: &Connection, account_id: i64) -> CoreResult<()> {
    let ok: Option<bool> = conn
        .query_row(
            &format!("SELECT {OFFLINE_SQL} FROM accounts a WHERE a.id = ?1"),
            [account_id],
            |r| r.get(0),
        )
        .optional()?;
    match ok {
        Some(true) => Ok(()),
        Some(false) => Err(CoreError::invalid(tr(
            "Manuelle Buchungen sind nur in Offline-Konten möglich.",
        ))),
        None => Err(not_found(trf("Konto {0}", &[&account_id]))),
    }
}

/// The live booking's account id, which must be an offline account.
fn booking_account(conn: &Connection, booking_id: i64) -> CoreResult<i64> {
    let account: Option<i64> = conn
        .query_row(
            "SELECT account_id FROM transactions WHERE id = ?1 AND deleted_ms IS NULL",
            [booking_id],
            |r| r.get(0),
        )
        .optional()?;
    let account = account.ok_or_else(|| not_found(trf("Buchung {0}", &[&booking_id])))?;
    require_offline(conn, account)?;
    Ok(account)
}

fn require_category(conn: &Connection, category_id: Option<i64>) -> CoreResult<()> {
    let Some(id) = category_id else {
        return Ok(());
    };
    let found: i64 = conn.query_row(
        "SELECT count(*) FROM categories WHERE id = ?1 AND deleted_ms IS NULL",
        [id],
        |r| r.get(0),
    )?;
    if found == 0 {
        return Err(not_found(trf("Kategorie {0}", &[&id])));
    }
    Ok(())
}

/// Makes sure the account has an opening balance, so `refresh` keeps its
/// balance in step with the bookings. An account imported without one keeps
/// the balance it shows now: opening = balance − Σ bookings.
fn ensure_opening(conn: &Connection, account_id: i64) -> CoreResult<()> {
    if crate::opening_balance::load(conn, account_id)?.is_some() {
        return Ok(());
    }
    let (balance, sum): (i64, i64) = conn.query_row(
        "SELECT a.balance_cents,
                (SELECT IFNULL(SUM(t.amount_cents), 0) FROM transactions t
                  WHERE t.account_id = a.id AND t.deleted_ms IS NULL
                    AND (t.currency = '' OR upper(t.currency) = upper(a.currency)))
           FROM accounts a WHERE a.id = ?1",
        [account_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    crate::opening_balance::store(conn, account_id, Some(balance - sum), "")
}

fn account_currency(conn: &Connection, account_id: i64) -> CoreResult<String> {
    Ok(conn.query_row(
        "SELECT currency FROM accounts WHERE id = ?1",
        [account_id],
        |r| r.get(0),
    )?)
}

fn new_external_id() -> String {
    format!("{BOOKING_ID_PREFIX}{}", uuid::Uuid::new_v4())
}

/// Inserts one live booking (already validated) and returns its id.
pub(crate) fn insert_booking(
    conn: &Connection,
    account_id: i64,
    date: &str,
    s: &ManualBookingSetup,
    counterparty: &str,
    purpose: &str,
    note: &str,
) -> CoreResult<i64> {
    let ms = now_ms();
    let currency = account_currency(conn, account_id)?;
    conn.execute(
        "INSERT INTO transactions(account_id, external_id, booking_date, value_date, amount_cents, currency,
                                  counterparty, purpose, category_id, category_source, note, checked,
                                  category_ms, note_ms, checked_ms, text_ms)
         VALUES (?1, ?2, ?3, ?3, ?4, ?5, ?6, ?7, ?8,
                 CASE WHEN ?8 IS NULL THEN 'none' ELSE 'manual' END, ?9, 1, ?10, ?10, ?10, ?10)",
        params![
            account_id,
            new_external_id(),
            date,
            s.amount_cents,
            currency,
            counterparty,
            purpose,
            s.category_id,
            note,
            ms
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Normalises an account currency: three ASCII letters (ISO 4217), upper-cased; empty means EUR.
fn clean_currency(code: &str) -> CoreResult<String> {
    let code = code.trim();
    if code.is_empty() {
        return Ok("EUR".to_string());
    }
    if code.len() != 3 || !code.bytes().all(|b| b.is_ascii_alphabetic()) {
        return Err(CoreError::invalid(tr(
            "Die Währung muss ein dreistelliger ISO-Code sein (z. B. EUR).",
        )));
    }
    Ok(code.to_ascii_uppercase())
}

/// Sidebar symbol of a new offline account: one per kind, so a savings account is told from cash at a glance (#1049).
fn offline_symbol(kind: AccountKind) -> &'static str {
    match kind {
        AccountKind::Cash => "banknote",
        AccountKind::Savings => "percent",
        AccountKind::TimeDeposit => "lock.circle",
        _ => "pencil.and.list.clipboard",
    }
}

/// Creates the account row, places it in its sidebar group and stores the opening balance.
pub(crate) fn create_in(
    conn: &Connection,
    name: &str,
    kind: AccountKind,
    currency: &str,
    opening_cents: i64,
) -> CoreResult<i64> {
    let currency = clean_currency(currency)?;
    let name = clean(name, MAX_NAME_CHARS)?;
    let name = if name.is_empty() {
        tr("Bargeld").to_string()
    } else {
        name
    };
    let (group, symbol, order) = group_for(kind);
    let gid = Vault::ensure_group(conn, group, symbol, order)?;
    conn.execute(
        "INSERT INTO accounts(connection_id, group_id, external_ref, name, kind, sf_symbol, balance_cents,
                              balance_date, currency, sort_order)
         VALUES (NULL, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, (SELECT IFNULL(MAX(sort_order), 0) + 1 FROM accounts))",
        params![
            gid,
            format!("{EXTERNAL_REF_PREFIX}{}", uuid::Uuid::new_v4()),
            name,
            kind.as_str(),
            offline_symbol(kind),
            opening_cents,
            today(),
            currency
        ],
    )?;
    let id = conn.last_insert_rowid();
    crate::user_groups::place_new_account(conn, id)?;
    crate::opening_balance::store(conn, id, Some(opening_cents), "")?;
    Ok(id)
}

/// Recomputes the account balance and lets the rules categorise what is new.
fn finish(conn: &Connection, account_id: i64) -> CoreResult<()> {
    Vault::apply_rules_inner(conn, true)?;
    crate::opening_balance::refresh(conn, account_id)
}

fn validate(
    s: &ManualBookingSetup,
    unchanged_zero_split: bool,
) -> CoreResult<(String, String, String, String)> {
    if s.amount_cents == 0 && !unchanged_zero_split {
        return Err(CoreError::invalid(tr("Der Betrag darf nicht 0 sein.")));
    }
    Ok((
        parse_date(&s.booking_date)?,
        clean(&s.counterparty, MAX_TEXT_CHARS)?,
        clean(&s.purpose, MAX_TEXT_CHARS)?,
        clean(&s.note, MAX_NOTE_CHARS)?,
    ))
}

#[uniffi::export]
impl Vault {
    /// Creates an offline account (no bank connection) with a starting balance
    /// and returns its id. An empty name becomes "Bargeld". `kind` must not be a
    /// depot or loan: they have their own flows. `currency` is an ISO 4217 code
    /// (empty = EUR); bookings and balance use it, nothing is converted.
    pub fn create_offline_account(
        &self,
        name: String,
        kind: AccountKind,
        currency: String,
        opening_cents: i64,
    ) -> Result<i64, CoreError> {
        if matches!(kind, AccountKind::Brokerage | AccountKind::Loan) {
            return Err(CoreError::invalid(tr(
                "Depots und Darlehen werden nicht als Offline-Konto angelegt.",
            )));
        }
        let db = self.db()?;
        let tx = db.unchecked_transaction()?;
        let id = create_in(&tx, &name, kind, &currency, opening_cents)?;
        tx.commit()?;
        Ok(id)
    }

    /// Ids of the accounts that take manual bookings: live, without a bank
    /// connection, no depots or loans.
    pub fn list_offline_account_ids(&self) -> Result<Vec<i64>, CoreError> {
        let db = self.db()?;
        let mut stmt = db.prepare(&format!(
            "SELECT a.id FROM accounts a WHERE {OFFLINE_SQL} ORDER BY a.id"
        ))?;
        let ids = stmt
            .query_map([], |r| r.get(0))?
            .collect::<Result<Vec<i64>, _>>()?;
        Ok(ids)
    }

    /// Creates (`booking_id` = `None`) or changes a booking of an offline
    /// account and recomputes its balance. Returns the booking id.
    pub fn save_manual_booking(
        &self,
        account_id: i64,
        booking_id: Option<i64>,
        setup: ManualBookingSetup,
    ) -> Result<i64, CoreError> {
        self.save_manual_booking_with_splits(account_id, booking_id, setup, None)
    }

    /// Saves the booking and optional replacement splits atomically. `None` keeps its splits.
    /// Changing a split booking's amount requires nonempty parts summing to the new amount.
    pub fn save_manual_booking_with_splits(
        &self,
        account_id: i64,
        booking_id: Option<i64>,
        setup: ManualBookingSetup,
        splits: Option<Vec<crate::models::TransactionSplitInput>>,
    ) -> Result<i64, CoreError> {
        if let Some(parts) = &splits {
            crate::planning::validate_splits(setup.amount_cents, parts)?;
        }
        let db = self.db()?;
        let tx = db.unchecked_transaction()?;
        let unchanged_zero_split = if let Some(id) = booking_id {
            tx.query_row("SELECT amount_cents=0 AND EXISTS(SELECT 1 FROM transaction_splits WHERE transaction_id=?1) FROM transactions WHERE id=?1", [id], |r| r.get::<_,bool>(0)).optional()?.unwrap_or(false)
                && setup.amount_cents == 0
                // the exception holds only while the booking keeps its parts: an empty
                // replacement would leave a zero booking without splits (CodeRabbit #1851)
                && splits.as_ref().is_none_or(|parts| !parts.is_empty())
        } else {
            false
        };
        let (date, counterparty, purpose, note) = validate(&setup, unchanged_zero_split)?;
        require_offline(&tx, account_id)?;
        require_category(&tx, setup.category_id)?;
        ensure_opening(&tx, account_id)?;
        let mut amount_changed = false;
        let mut date_changed = false;
        let id = match booking_id {
            None => insert_booking(
                &tx,
                account_id,
                &date,
                &setup,
                &counterparty,
                &purpose,
                &note,
            )?,
            Some(id) => {
                if booking_account(&tx, id)? != account_id {
                    return Err(not_found(trf("Buchung {0}", &[&id])));
                }
                let (changed, has_splits): (bool, bool) = tx.query_row(
                    "SELECT amount_cents != ?2, EXISTS(SELECT 1 FROM transaction_splits WHERE transaction_id=?1)
                     FROM transactions WHERE id=?1",
                    params![id, setup.amount_cents], |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                amount_changed = changed;
                date_changed = tx.query_row("SELECT booking_date != ?2 OR COALESCE(value_date,'') != ?2 FROM transactions WHERE id=?1", params![id,date], |r| r.get(0))?;
                if amount_changed && has_splits && splits.as_ref().is_none_or(Vec::is_empty) {
                    return Err(CoreError::invalid(tr(
                        "Bitte passen Sie die Aufteilung zusammen mit dem Buchungsbetrag an.",
                    )));
                }
                let ms = now_ms();
                let old: (Option<i64>, String, String) = tx.query_row(
                    "SELECT category_id, note, counterparty || char(31) || purpose FROM transactions WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )?;
                tx.execute(
                    "UPDATE transactions SET booking_date = ?1, value_date = ?1, amount_cents = ?2 WHERE id = ?3",
                    params![date, setup.amount_cents, id],
                )?;
                if old.2 != format!("{counterparty}\u{1f}{purpose}") {
                    tx.execute(
                        // never below the stored stamp, also with a clock behind a peer's
                        "UPDATE transactions SET counterparty = ?1, purpose = ?2, text_ms = MAX(text_ms + 1, ?4) WHERE id = ?3",
                        params![counterparty, purpose, id, ms],
                    )?;
                }
                if old.1 != note {
                    tx.execute(
                        "UPDATE transactions SET note = ?1, note_ms = ?3 WHERE id = ?2",
                        params![note, id, ms],
                    )?;
                }
                if old.0 != setup.category_id {
                    tx.execute(
                        "UPDATE transactions SET category_id = ?1, rule_id = NULL, category_ms = ?3,
                                category_source = CASE WHEN ?1 IS NULL THEN 'none' ELSE 'manual' END
                          WHERE id = ?2",
                        params![setup.category_id, id, ms],
                    )?;
                }
                id
            }
        };
        if let Some(parts) = &splits {
            crate::planning::store_splits(&tx, id, parts)?;
        }
        // amount and parts sync as one unit (`splits_ms`), the date on its own (`date_ms`)
        if amount_changed {
            tx.execute(
                "UPDATE transactions SET splits_ms=MAX(splits_ms+1,?2) WHERE id=?1",
                params![id, now_ms()],
            )?;
            // rules can match amounts: a rule-assigned category is evaluated again by `finish`,
            // as on a receiving device (sync merge); manual categories and splits stay
            tx.execute(
                "UPDATE transactions SET category_id = NULL, rule_id = NULL, category_source = 'none'
                  WHERE id = ?1 AND category_source = 'rule'
                    AND NOT EXISTS (SELECT 1 FROM transaction_splits WHERE transaction_id = ?1)",
                [id],
            )?;
        }
        if date_changed {
            tx.execute(
                "UPDATE transactions SET date_ms=MAX(date_ms+1,?2) WHERE id=?1",
                params![id, now_ms()],
            )?;
        }
        finish(&tx, account_id)?;
        tx.commit()?;
        Ok(id)
    }

    /// Deletes a booking of an offline account (soft: other devices delete it
    /// too instead of bringing it back) and recomputes the balance.
    pub fn delete_manual_booking(&self, booking_id: i64) -> Result<(), CoreError> {
        let db = self.db()?;
        let tx = db.unchecked_transaction()?;
        let account_id = booking_account(&tx, booking_id)?;
        ensure_opening(&tx, account_id)?;
        tx.execute(
            "UPDATE transactions SET deleted_ms = ?1 WHERE id = ?2",
            params![now_ms(), booking_id],
        )?;
        crate::opening_balance::refresh(&tx, account_id)?;
        tx.commit()?;
        Ok(())
    }

    /// One-time copy of a booking into an offline account with the amount
    /// reversed and no category (an ATM withdrawal becomes a cash deposit).
    /// The copy is an ordinary manual booking afterwards. Returns its id.
    pub fn mirror_booking_to_offline_account(
        &self,
        booking_id: i64,
        account_id: i64,
    ) -> Result<i64, CoreError> {
        let db = self.db()?;
        let tx = db.unchecked_transaction()?;
        require_offline(&tx, account_id)?;
        let (source_account, date, cents, counterparty, purpose): (
            i64,
            String,
            i64,
            String,
            String,
        ) = tx
            .query_row(
                "SELECT account_id, booking_date, amount_cents, counterparty, purpose
                   FROM transactions WHERE id = ?1 AND deleted_ms IS NULL",
                [booking_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?
            .ok_or_else(|| not_found(trf("Buchung {0}", &[&booking_id])))?;
        if source_account == account_id {
            return Err(CoreError::invalid(tr(
                "Eine Buchung lässt sich nicht in ihr eigenes Konto spiegeln.",
            )));
        }
        if cents == 0 {
            return Err(CoreError::invalid(tr("Der Betrag darf nicht 0 sein.")));
        }
        ensure_opening(&tx, account_id)?;
        let setup = ManualBookingSetup {
            booking_date: date.clone(),
            amount_cents: -cents,
            counterparty: counterparty.clone(),
            purpose: purpose.clone(),
            category_id: None,
            note: String::new(),
        };
        let id = insert_booking(&tx, account_id, &date, &setup, &counterparty, &purpose, "")?;
        // no `finish`: the copy stays without a category, the rules are not run
        crate::opening_balance::refresh(&tx, account_id)?;
        tx.commit()?;
        Ok(id)
    }

    /// Books the difference between the account balance and `target_cents` as
    /// a correction booking dated today ("Saldo korrigieren"). Returns its id,
    /// or `None` when the balance already matches.
    pub fn correct_offline_balance(
        &self,
        account_id: i64,
        target_cents: i64,
    ) -> Result<Option<i64>, CoreError> {
        let db = self.db()?;
        let tx = db.unchecked_transaction()?;
        require_offline(&tx, account_id)?;
        ensure_opening(&tx, account_id)?;
        crate::opening_balance::refresh(&tx, account_id)?;
        let balance: i64 = tx.query_row(
            "SELECT balance_cents FROM accounts WHERE id = ?1",
            [account_id],
            |r| r.get(0),
        )?;
        let diff = target_cents - balance;
        if diff == 0 {
            return Ok(None);
        }
        let label = tr("Saldokorrektur").to_string();
        let setup = ManualBookingSetup {
            booking_date: today(),
            amount_cents: diff,
            counterparty: label.clone(),
            purpose: String::new(),
            category_id: None,
            note: String::new(),
        };
        let id = insert_booking(&tx, account_id, &today(), &setup, &label, "", "")?;
        crate::opening_balance::refresh(&tx, account_id)?;
        tx.commit()?;
        Ok(Some(id))
    }
}

#[cfg(test)]
mod tests;
