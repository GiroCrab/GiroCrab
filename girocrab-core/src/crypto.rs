//! Crypto holdings (GiroCrab/GiroCrab#1836): coins the user holds outside a
//! bank, e.g. on a hardware wallet, kept by hand.
//!
//! A crypto account is a securities depot without a bank connection
//! (`kind = 'brokerage'`, `external_ref = "crypto-<uuid>"`). Its positions
//! (ticker, quantity, optional purchase price per unit) live in
//! `crypto_positions` and travel with the vault merge (newest edit wins, soft
//! deletes). The last known EUR price per ticker lives in `crypto_prices`.
//!
//! After every change ([`materialize`]) the positions are written into
//! `holdings` and the account balance, and the depot value history records
//! them like a bank fetch (`depot_values::record_fetch`). So the rest of the
//! app sees an ordinary depot: positions, gain, balance and value history.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension, params};

use crate::dates::{now_ms, today_iso};
use crate::error::{CoreError, CoreResult};
use crate::i18n::{tr, trf};
use crate::models::{AccountKind, Holding};
use crate::schema::has_table;
use crate::vault::Vault;

/// Schema step 46.
pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS crypto_positions (
    id                   INTEGER PRIMARY KEY,
    uuid                 TEXT    NOT NULL UNIQUE,     -- identity across devices (vault merge)
    account_id           INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    symbol               TEXT    NOT NULL,            -- ticker, upper case: BTC, ETH …
    quantity             REAL    NOT NULL,
    purchase_price_cents INTEGER,                     -- per unit in EUR; NULL = unknown
    changed_ms           INTEGER NOT NULL DEFAULT 0,  -- last edit (vault merge: newest wins)
    deleted_ms           INTEGER                      -- soft delete, so a merge can't bring it back
);
CREATE INDEX IF NOT EXISTS crypto_positions_account ON crypto_positions(account_id);
CREATE TABLE IF NOT EXISTS crypto_prices (
    symbol     TEXT    PRIMARY KEY,
    price_eur  REAL    NOT NULL,                      -- EUR per unit, unrounded (tiny prices)
    fetched_ms INTEGER NOT NULL
);
";

const EXTERNAL_REF_PREFIX: &str = "crypto-";
const MAX_NAME_CHARS: usize = 100;

/// A position of a crypto account with its last known price.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CryptoPosition {
    pub symbol: String,
    pub quantity: f64,
    /// Per unit in EUR cents; `None` = unknown.
    pub purchase_price_cents: Option<i64>,
    /// EUR per unit; `None` = never fetched.
    pub price_eur: Option<f64>,
    pub price_fetched_ms: Option<i64>,
    /// Quantity × price in EUR cents; 0 without a price.
    pub value_cents: i64,
}

pub(crate) fn is_crypto(conn: &Connection, account_id: i64) -> CoreResult<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM accounts WHERE id = ?1 AND connection_id IS NULL AND external_ref LIKE 'crypto-%'
                AND kind = 'brokerage' AND deleted_ms IS NULL",
            [account_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn require_crypto(conn: &Connection, account_id: i64) -> CoreResult<()> {
    if is_crypto(conn, account_id)? {
        Ok(())
    } else {
        Err(CoreError::NotFound {
            what: trf("Crypto-Bestand {0}", &[&account_id]),
        })
    }
}

/// `BTC`, `eth ` → `ETH`; letters and digits only, at most 10.
fn normalize_symbol(symbol: &str) -> CoreResult<String> {
    let s = symbol.trim().to_ascii_uppercase();
    if s.is_empty() || s.len() > 10 || !s.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(CoreError::invalid(tr(
            "Bitte ein Kürzel aus Buchstaben und Ziffern angeben (z. B. BTC).",
        )));
    }
    Ok(s)
}

/// The held position `symbol` of `account_id`.
fn position_id(conn: &Connection, account_id: i64, symbol: &str) -> CoreResult<i64> {
    let symbol = symbol.trim().to_ascii_uppercase();
    conn.query_row(
        "SELECT id FROM crypto_positions WHERE account_id = ?1 AND symbol = ?2 AND deleted_ms IS NULL",
        params![account_id, symbol],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| CoreError::NotFound {
        what: tr("Depotposition").into(),
    })
}

/// Quantity × price in cents, rounded after multiplying (tiny prices keep their
/// precision); a value beyond what cents can hold is an error, not a clamp.
fn value_cents(quantity: f64, price_eur: f64) -> CoreResult<i64> {
    let v = (quantity * price_eur * 100.0).round();
    if v.is_finite() && v.abs() < i64::MAX as f64 {
        Ok(v as i64)
    } else {
        Err(too_large())
    }
}

fn too_large() -> CoreError {
    CoreError::invalid(tr("Der Wert des Bestands ist zu groß."))
}

fn load(conn: &Connection, account_id: i64) -> CoreResult<Vec<CryptoPosition>> {
    let mut st = conn.prepare(
        "SELECT p.symbol, p.quantity, p.purchase_price_cents, c.price_eur, c.fetched_ms
           FROM crypto_positions p LEFT JOIN crypto_prices c ON c.symbol = p.symbol
          WHERE p.account_id = ?1 AND p.deleted_ms IS NULL ORDER BY p.symbol",
    )?;
    let rows = st.query_map([account_id], |r| {
        Ok(CryptoPosition {
            symbol: r.get(0)?,
            quantity: r.get(1)?,
            purchase_price_cents: r.get(2)?,
            price_eur: r.get(3)?,
            price_fetched_ms: r.get(4)?,
            value_cents: 0,
        })
    })?;
    let mut out: Vec<CryptoPosition> = rows.collect::<Result<_, _>>()?;
    for p in &mut out {
        if let Some(price) = p.price_eur {
            p.value_cents = value_cents(p.quantity, price)?;
        }
    }
    Ok(out)
}

/// Writes the positions of the crypto account `account_id` into `holdings` and
/// its balance, and records them in the depot value history like a fetch: a
/// position no longer held gets its closing 0. A position without a price
/// shows with value 0; while there is one, the history records the priced
/// positions but no total (it would be too low).
pub(crate) fn materialize(conn: &Connection, account_id: i64) -> CoreResult<()> {
    if !is_crypto(conn, account_id)? {
        return Ok(());
    }
    let positions = load(conn, account_id)?;
    let previous: Vec<String> = crate::depot_values::positions::held_keys(conn, account_id)?
        .into_iter()
        .collect();
    conn.execute("DELETE FROM holdings WHERE account_id = ?1", [account_id])?;
    let holdings: Vec<Holding> = positions
        .iter()
        .map(|p| Holding {
            wkn: p.symbol.clone(),
            name: p.symbol.clone(),
            quantity: p.quantity,
            current_price_cents: p.price_eur.map_or(0, |e| (e * 100.0).round() as i64),
            purchase_price_cents: p.purchase_price_cents.unwrap_or(0),
            value_cents: p.value_cents,
            ..Default::default()
        })
        .collect();
    for h in &holdings {
        conn.execute(
            "INSERT INTO holdings(account_id, isin, wkn, name, quantity, current_price_cents, purchase_price_cents, value_cents)
             VALUES (?1, '', ?2, ?3, ?4, ?5, ?6, ?7)",
            params![account_id, h.wkn, h.name, h.quantity, h.current_price_cents, h.purchase_price_cents, h.value_cents],
        )?;
        conn.execute(
            "INSERT INTO holding_classifications(holding_id, direct_crypto) VALUES (?1, 1)",
            [conn.last_insert_rowid()],
        )?;
    }
    let all_priced = positions.iter().all(|p| p.price_eur.is_some());
    let total = positions
        .iter()
        .try_fold(0i64, |sum, p| sum.checked_add(p.value_cents))
        .ok_or_else(too_large)?;
    conn.execute(
        "UPDATE accounts SET balance_cents = ?1, balance_date = ?2 WHERE id = ?3",
        params![total, today_iso(), account_id],
    )?;
    let priced: Vec<&Holding> = holdings
        .iter()
        .zip(&positions)
        .filter(|(_, p)| p.price_eur.is_some())
        .map(|(h, _)| h)
        .collect();
    // each state its own observation, also for two changes within one millisecond
    let last: Option<i64> = conn.query_row(
        "SELECT MAX(observed_ms) FROM depot_observations WHERE account_id = ?1",
        [account_id],
        |r| r.get(0),
    )?;
    let observed = now_ms().max(last.map_or(0, |l| l + 1));
    crate::depot_values::record_fetch(
        conn,
        account_id,
        Some(priced.as_slice()),
        &previous,
        all_priced,
        observed,
    )
}

/// Two devices that each added the same ticker to an account hold two live
/// positions after a merge: the newest edit stays (on equal stamps the larger
/// uuid, so every device keeps the same one), the others become tombstones
/// that a later merge carries along.
/// Returns the accounts it changed.
fn resolve_duplicate_tickers(conn: &Connection) -> CoreResult<Vec<i64>> {
    let mut st = conn.prepare(
        "UPDATE crypto_positions SET deleted_ms = ?1, changed_ms = MAX(changed_ms + 1, ?1)
          WHERE deleted_ms IS NULL AND EXISTS (
                SELECT 1 FROM crypto_positions o
                 WHERE o.account_id = crypto_positions.account_id AND o.symbol = crypto_positions.symbol
                   AND o.deleted_ms IS NULL AND o.id <> crypto_positions.id
                   AND (o.changed_ms, o.uuid) > (crypto_positions.changed_ms, crypto_positions.uuid))
          RETURNING account_id",
    )?;
    let rows = st.query_map([now_ms()], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Ids of all crypto accounts.
fn crypto_account_ids(conn: &Connection) -> CoreResult<Vec<i64>> {
    let mut st = conn.prepare(
        "SELECT id FROM accounts WHERE connection_id IS NULL AND external_ref LIKE 'crypto-%'
            AND kind = 'brokerage' AND deleted_ms IS NULL ORDER BY id",
    )?;
    let rows = st.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Takes over positions and prices from an attached vault (`other`): positions
/// by uuid, the newer edit wins and soft deletes travel along (on equal stamps
/// a deletion wins, then the larger values, so both devices agree); a price by
/// ticker, the newer fetch wins (on equal stamps the higher price). Then the
/// crypto accounts the merge changed are written again; an untouched one keeps
/// its state, so a merge records no valuation of its own.
pub(crate) fn merge_attached(conn: &Connection, acc_map: &HashMap<i64, i64>) -> CoreResult<()> {
    if !has_table(conn, "other", "crypto_positions")? {
        return Ok(());
    }
    let rows: Vec<(i64, i64)> = conn
        .prepare("SELECT id, account_id FROM other.crypto_positions")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let mut touched: HashSet<i64> = HashSet::new();
    for (oid, oacc) in rows {
        let Some(acc) = acc_map.get(&oacc) else {
            continue;
        };
        let n = conn.execute(
            "INSERT INTO crypto_positions(uuid, account_id, symbol, quantity, purchase_price_cents, changed_ms, deleted_ms)
             SELECT uuid, ?2, symbol, quantity, purchase_price_cents, changed_ms, deleted_ms
               FROM other.crypto_positions WHERE id = ?1
             ON CONFLICT(uuid) DO UPDATE SET symbol = excluded.symbol, quantity = excluded.quantity,
                    purchase_price_cents = excluded.purchase_price_cents,
                    changed_ms = excluded.changed_ms, deleted_ms = excluded.deleted_ms
              WHERE excluded.changed_ms > crypto_positions.changed_ms
                 OR (excluded.changed_ms = crypto_positions.changed_ms
                     AND (excluded.deleted_ms IS NOT NULL, excluded.quantity, IFNULL(excluded.purchase_price_cents, -1), excluded.symbol)
                       > (crypto_positions.deleted_ms IS NOT NULL, crypto_positions.quantity,
                          IFNULL(crypto_positions.purchase_price_cents, -1), crypto_positions.symbol))",
            params![oid, acc],
        )?;
        if n > 0 {
            touched.insert(*acc);
        }
    }
    let changed_symbols: Vec<String> = conn
        .prepare(
            "INSERT INTO crypto_prices(symbol, price_eur, fetched_ms)
             SELECT symbol, price_eur, fetched_ms FROM other.crypto_prices WHERE true
             ON CONFLICT(symbol) DO UPDATE SET price_eur = excluded.price_eur, fetched_ms = excluded.fetched_ms
              WHERE excluded.fetched_ms > crypto_prices.fetched_ms
                 OR (excluded.fetched_ms = crypto_prices.fetched_ms AND excluded.price_eur > crypto_prices.price_eur)
             RETURNING symbol",
        )?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    touched.extend(resolve_duplicate_tickers(conn)?);
    touched.extend(holders_of(conn, &changed_symbols)?);
    for id in crypto_account_ids(conn)? {
        if touched.contains(&id) {
            materialize(conn, id)?;
        }
    }
    Ok(())
}

/// The accounts holding one of `symbols`.
fn holders_of(conn: &Connection, symbols: &[String]) -> CoreResult<Vec<i64>> {
    if symbols.is_empty() {
        return Ok(Vec::new());
    }
    let mut st = conn.prepare(
        "SELECT DISTINCT account_id FROM crypto_positions
          WHERE deleted_ms IS NULL AND symbol IN (SELECT value FROM json_each(?1))",
    )?;
    let rows = st.query_map([serde_json::to_string(symbols)?], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[uniffi::export]
impl Vault {
    /// Creates an empty crypto account (a depot without a bank connection).
    /// An empty name becomes "Crypto". Returns the account id.
    pub fn create_crypto_account(&self, name: String) -> Result<i64, CoreError> {
        let name = name.trim();
        let name = if name.is_empty() { "Crypto" } else { name };
        if name.chars().count() > MAX_NAME_CHARS {
            return Err(CoreError::invalid(trf(
                "Der Name ist zu lang (höchstens {0} Zeichen).",
                &[&MAX_NAME_CHARS],
            )));
        }
        let db = self.db()?;
        let tx = db.unchecked_transaction()?;
        let (group, sym, order) = crate::connectors::group_for(AccountKind::Brokerage);
        let gid = Vault::ensure_group(&tx, group, sym, order)?;
        tx.execute(
            "INSERT INTO accounts(connection_id, group_id, external_ref, name, kind, sf_symbol, balance_date, sort_order)
             VALUES (NULL, ?1, ?2, ?3, 'brokerage', 'bitcoinsign.circle', ?4,
                     (SELECT IFNULL(MAX(sort_order),0)+1 FROM accounts))",
            params![gid, format!("{EXTERNAL_REF_PREFIX}{}", uuid::Uuid::new_v4()), name, today_iso()],
        )?;
        let id = tx.last_insert_rowid();
        crate::user_groups::place_new_account(&tx, id)?;
        tx.commit()?;
        Ok(id)
    }

    /// Ids of all crypto accounts.
    pub fn list_crypto_account_ids(&self) -> Result<Vec<i64>, CoreError> {
        crypto_account_ids(&*self.db()?)
    }

    /// The positions of a crypto account, by ticker.
    pub fn list_crypto_positions(&self, account_id: i64) -> Result<Vec<CryptoPosition>, CoreError> {
        let db = self.db()?;
        require_crypto(&db, account_id)?;
        load(&db, account_id)
    }

    /// Adds a position (`previous_symbol` = None) or changes the one held as
    /// `previous_symbol` (its ticker may change). A ticker is held once per
    /// account.
    pub fn save_crypto_position(
        &self,
        account_id: i64,
        previous_symbol: Option<String>,
        symbol: String,
        quantity: f64,
        purchase_price_cents: Option<i64>,
    ) -> Result<(), CoreError> {
        let symbol = normalize_symbol(&symbol)?;
        if !quantity.is_finite() || quantity <= 0.0 {
            return Err(CoreError::invalid(tr(
                "Die Stückzahl muss größer als 0 sein.",
            )));
        }
        if purchase_price_cents.is_some_and(|c| c < 0) {
            return Err(CoreError::invalid(tr(
                "Der Einstandskurs darf nicht negativ sein.",
            )));
        }
        let db = self.db()?;
        let tx = db.unchecked_transaction()?;
        require_crypto(&tx, account_id)?;
        let previous = match previous_symbol {
            Some(p) => Some(position_id(&tx, account_id, &p)?),
            None => None,
        };
        let taken: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM crypto_positions WHERE account_id = ?1 AND symbol = ?2
                AND deleted_ms IS NULL AND id IS NOT ?3)",
            params![account_id, symbol, previous],
            |r| r.get(0),
        )?;
        if taken {
            return Err(CoreError::invalid(trf(
                "{0} ist bereits in diesem Bestand.",
                &[&symbol],
            )));
        }
        match previous {
            Some(id) => tx.execute(
                "UPDATE crypto_positions SET symbol = ?1, quantity = ?2, purchase_price_cents = ?3,
                        changed_ms = MAX(?4, changed_ms + 1)
                  WHERE id = ?5",
                params![symbol, quantity, purchase_price_cents, now_ms(), id],
            )?,
            None => tx.execute(
                "INSERT INTO crypto_positions(uuid, account_id, symbol, quantity, purchase_price_cents, changed_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![uuid::Uuid::new_v4().to_string(), account_id, symbol, quantity, purchase_price_cents, now_ms()],
            )?,
        };
        materialize(&tx, account_id)?;
        tx.commit()?;
        Ok(())
    }

    /// Removes the position `symbol` (soft delete, so a vault merge does not
    /// bring it back).
    pub fn delete_crypto_position(&self, account_id: i64, symbol: String) -> Result<(), CoreError> {
        let db = self.db()?;
        let tx = db.unchecked_transaction()?;
        require_crypto(&tx, account_id)?;
        let id = position_id(&tx, account_id, &symbol)?;
        tx.execute(
            "UPDATE crypto_positions SET deleted_ms = ?1, changed_ms = MAX(?1, changed_ms + 1) WHERE id = ?2",
            params![now_ms(), id],
        )?;
        materialize(&tx, account_id)?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
