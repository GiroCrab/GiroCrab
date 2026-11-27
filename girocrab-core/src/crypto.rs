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
/// More than any coin supply a person holds; keeps quantity × price in range.
const MAX_QUANTITY: f64 = 1e12;
/// The largest value of one position (10 trillion EUR). Far below `i64::MAX`, so
/// the sum of an account and of all depots of a day cannot overflow either
/// (that would take about 9,000 such positions).
const MAX_VALUE_CENTS: i64 = 1_000_000_000_000_000;
/// The largest price per unit that still gives a value (one unit at [`MAX_VALUE_CENTS`]).
const MAX_PRICE_EUR: f64 = MAX_VALUE_CENTS as f64 / 100.0;

/// A price the app can value with, the one rule for fetch, storage, merge and
/// [`load`]: positive, finite, one unit within [`MAX_VALUE_CENTS`].
fn valid_price(eur: f64) -> bool {
    eur.is_finite() && eur > 0.0 && eur <= MAX_PRICE_EUR
}

/// A position of a crypto account with its last known price.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CryptoPosition {
    pub symbol: String,
    pub quantity: f64,
    /// Per unit in EUR cents; `None` = unknown.
    pub purchase_price_cents: Option<i64>,
    /// EUR per unit; `None` = no usable price: never fetched, or one the
    /// position cannot be valued with (beyond [`MAX_VALUE_CENTS`]).
    pub price_eur: Option<f64>,
    /// When the stored price was fetched, also when it is not usable.
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

/// A crypto account holding a ticker without a price: its value is unknown,
/// the depot history does not carry an earlier total forward
/// (`depot_values::series`).
pub(crate) fn is_incomplete(conn: &Connection, account_id: i64) -> CoreResult<bool> {
    Ok(is_crypto(conn, account_id)?
        && load(conn, account_id)?
            .iter()
            .any(|p| p.price_eur.is_none()))
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
    // the depot history keys a position by its name and folds these into the cash position
    // (`observations::position_key`): as tickers they would share one series
    if !matches!(
        crate::depot_values::observations::position_key("", &s),
        Some(k) if k.starts_with("name:")
    ) {
        return Err(CoreError::invalid(trf(
            "„{0}“ ist als Kürzel nicht möglich.",
            &[&s],
        )));
    }
    Ok(s)
}

/// Two devices that each added the same ticker to an account hold two live
/// positions after a merge. A merge keeps both (deleting one would make the
/// result depend on the merge order). Per account and ticker the newest row
/// decides (largest `(changed_ms, uuid)`, deleted rows included): live, it is
/// the position every device shows; deleted, the ticker is not held – a newer
/// deletion also hides older copies its device never saw. The result depends
/// only on the set of rows. A user edit of the ticker retires the others.
const NEWEST_OF_TICKER: &str = "NOT EXISTS (
    SELECT 1 FROM crypto_positions o
     WHERE o.account_id = p.account_id AND o.symbol = p.symbol AND o.id <> p.id
       AND (o.changed_ms, o.uuid) > (p.changed_ms, p.uuid))";

/// The held position `symbol` of `account_id` (the newest, see [`NEWEST_OF_TICKER`]).
fn position_id(conn: &Connection, account_id: i64, symbol: &str) -> CoreResult<i64> {
    let symbol = symbol.trim().to_ascii_uppercase();
    conn.query_row(
        &format!(
            "SELECT id FROM crypto_positions p WHERE account_id = ?1 AND symbol = ?2 AND deleted_ms IS NULL
                AND {NEWEST_OF_TICKER}"
        ),
        params![account_id, symbol],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| CoreError::NotFound {
        what: tr("Depotposition").into(),
    })
}

/// The user edits or deletes the position `id`: other live positions of its
/// ticker (merged in from another device) are retired with it. They keep their
/// stamp: on an equal stamp the deletion wins a merge, it never outranks `id`,
/// and a newer edit of that copy elsewhere still counts.
fn retire_twins(conn: &Connection, id: i64) -> CoreResult<()> {
    conn.execute(
        "UPDATE crypto_positions SET deleted_ms = ?2
          WHERE deleted_ms IS NULL AND id <> ?1
            AND (account_id, symbol) = (SELECT account_id, symbol FROM crypto_positions WHERE id = ?1)",
        params![id, now_ms()],
    )?;
    Ok(())
}

/// Quantity × price in cents, rounded after multiplying (tiny prices keep their
/// precision); a value beyond what cents can hold is an error, not a clamp.
fn value_cents(quantity: f64, price_eur: f64) -> CoreResult<i64> {
    let v = (quantity * price_eur * 100.0).round();
    if v.is_finite() && v.abs() <= MAX_VALUE_CENTS as f64 {
        Ok(v as i64)
    } else {
        Err(too_large())
    }
}

fn too_large() -> CoreError {
    CoreError::invalid(tr("Der Wert des Bestands ist zu groß."))
}

fn load(conn: &Connection, account_id: i64) -> CoreResult<Vec<CryptoPosition>> {
    let mut st = conn.prepare(&format!(
        "SELECT p.symbol, p.quantity, p.purchase_price_cents, c.price_eur, c.fetched_ms
           FROM crypto_positions p LEFT JOIN crypto_prices c ON c.symbol = p.symbol
          WHERE p.account_id = ?1 AND p.deleted_ms IS NULL AND {NEWEST_OF_TICKER} ORDER BY p.symbol"
    ))?;
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
        // a value cents cannot hold (e.g. a merged position meeting a price) counts as
        // unpriced: the account stays usable, its total unknown (see `is_incomplete`)
        match p
            .price_eur
            .filter(|price| valid_price(*price))
            .map(|price| value_cents(p.quantity, price))
        {
            Some(Ok(v)) => p.value_cents = v,
            _ => p.price_eur = None,
        }
    }
    Ok(out)
}

/// Writes the positions of the crypto account `account_id` into `holdings` and
/// its balance, and records them in the depot value history like a fetch: a
/// position no longer held gets its closing 0. A position without a price
/// shows with value 0; while there is one, the history records the priced
/// positions but no total (it would be too low).
///
/// `fresh` (after a price refresh): only these tickers got a new price. The
/// history then records only their positions, and a total only when every
/// position is among them: a cached price is no observation of today. An
/// empty set (a vault merge) records nothing.
pub(crate) fn materialize(
    conn: &Connection,
    account_id: i64,
    fresh: Option<&HashSet<String>>,
) -> CoreResult<()> {
    if !is_crypto(conn, account_id)? {
        return Ok(());
    }
    let positions = load(conn, account_id)?;
    // a position closes only when it is gone, not while it is held without a value
    let held: HashSet<String> = positions
        .iter()
        .filter_map(|p| crate::depot_values::observations::position_key("", &p.symbol))
        .collect();
    let previous: Vec<String> = crate::depot_values::positions::held_keys(conn, account_id)?
        .into_iter()
        .filter(|k| !held.contains(k))
        .collect();
    // built before the old holdings go: an error leaves them as they were
    let holdings: Vec<Holding> = positions
        .iter()
        .map(|p| {
            Ok(Holding {
                wkn: p.symbol.clone(),
                name: p.symbol.clone(),
                quantity: p.quantity,
                current_price_cents: p.price_eur.map_or(Ok(0), |e| value_cents(1.0, e))?,
                purchase_price_cents: p.purchase_price_cents.unwrap_or(0),
                value_cents: p.value_cents,
                ..Default::default()
            })
        })
        .collect::<CoreResult<_>>()?;
    conn.execute("DELETE FROM holdings WHERE account_id = ?1", [account_id])?;
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
    let total = positions
        .iter()
        .try_fold(0i64, |sum, p| sum.checked_add(p.value_cents))
        .ok_or_else(too_large)?;
    conn.execute(
        "UPDATE accounts SET balance_cents = ?1, balance_date = ?2 WHERE id = ?3",
        params![total, today_iso(), account_id],
    )?;
    let observed_now =
        |p: &CryptoPosition| p.price_eur.is_some() && fresh.is_none_or(|f| f.contains(&p.symbol));
    let priced: Vec<&Holding> = holdings
        .iter()
        .zip(&positions)
        .filter(|(_, p)| observed_now(p))
        .map(|(h, _)| h)
        .collect();
    let all_observed = positions.iter().all(observed_now);
    // a refresh sells nothing: a position left out of `priced` is not closed
    let previous = if fresh.is_some() {
        Vec::new()
    } else {
        previous
    };
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
        all_observed,
        observed,
    )
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
/// ticker, the newer fetch wins (on equal stamps the higher price). Two live
/// positions of one ticker both stay (see [`NEWEST_OF_TICKER`]). Then the
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
             SELECT symbol, price_eur, fetched_ms FROM other.crypto_prices
              WHERE price_eur > 0 AND price_eur <= ?1 -- `valid_price`, as a fetch checks it
             ON CONFLICT(symbol) DO UPDATE SET price_eur = excluded.price_eur, fetched_ms = excluded.fetched_ms
              WHERE excluded.fetched_ms > crypto_prices.fetched_ms
                 OR (excluded.fetched_ms = crypto_prices.fetched_ms AND excluded.price_eur > crypto_prices.price_eur)
             RETURNING symbol",
        )?
        .query_map([MAX_PRICE_EUR], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    touched.extend(holders_of(conn, &changed_symbols)?);
    // positions and balance follow the merged data; the history does not record them again:
    // the device that made the change or fetched the price recorded it, and its observations
    // come along with the merge (`depot_values::merge_attached`)
    let nothing_fresh = HashSet::new();
    for id in crypto_account_ids(conn)? {
        if touched.contains(&id) {
            materialize(conn, id, Some(&nothing_fresh))?;
        }
    }
    Ok(())
}

/// The accounts holding one of `symbols`.
fn holders_of(conn: &Connection, symbols: &[String]) -> CoreResult<Vec<i64>> {
    if symbols.is_empty() {
        return Ok(Vec::new());
    }
    let mut st = conn.prepare(&format!(
        "SELECT DISTINCT account_id FROM crypto_positions p
          WHERE deleted_ms IS NULL AND {NEWEST_OF_TICKER} AND symbol IN (SELECT value FROM json_each(?1))"
    ))?;
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
        if quantity > MAX_QUANTITY {
            return Err(too_large());
        }
        if purchase_price_cents.is_some_and(|c| c < 0) {
            return Err(CoreError::invalid(tr(
                "Der Einstandskurs darf nicht negativ sein.",
            )));
        }
        // 0 means unknown, as everywhere for depot positions (`holdings.purchase_price_cents`)
        let purchase_price_cents = purchase_price_cents.filter(|c| *c > 0);
        let db = self.db()?;
        let tx = db.unchecked_transaction()?;
        require_crypto(&tx, account_id)?;
        let known: Option<f64> = tx
            .query_row(
                "SELECT price_eur FROM crypto_prices WHERE symbol = ?1",
                [&symbol],
                |r| r.get(0),
            )
            .optional()?;
        let previous = match previous_symbol {
            Some(p) => {
                let id = position_id(&tx, account_id, &p)?;
                retire_twins(&tx, id)?;
                Some(id)
            }
            None => None,
        };
        let old_quantity: Option<f64> = match previous {
            Some(id) => Some(tx.query_row(
                "SELECT quantity FROM crypto_positions WHERE id = ?1",
                [id],
                |r| r.get(0),
            )?),
            None => None,
        };
        // a new or changed quantity must give a value (a price that gives no per-unit value
        // counts as none, see `load`); an unchanged one may stay, e.g. to edit the purchase price
        if old_quantity != Some(quantity)
            && let Some(price) = known.filter(|p| valid_price(*p))
        {
            value_cents(quantity, price)?;
        }
        let taken: bool = tx.query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM crypto_positions p WHERE account_id = ?1 AND symbol = ?2
                    AND deleted_ms IS NULL AND id IS NOT ?3 AND {NEWEST_OF_TICKER})"
            ),
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
        materialize(&tx, account_id, None)?;
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
        retire_twins(&tx, id)?;
        tx.execute(
            "UPDATE crypto_positions SET deleted_ms = ?1, changed_ms = MAX(?1, changed_ms + 1) WHERE id = ?2",
            params![now_ms(), id],
        )?;
        materialize(&tx, account_id, None)?;
        tx.commit()?;
        Ok(())
    }
}

pub mod prices;

#[cfg(test)]
mod tests;
