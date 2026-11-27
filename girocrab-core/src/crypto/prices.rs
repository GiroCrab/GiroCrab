//! EUR prices of crypto holdings (#1836) from the public Coinbase API, without
//! a key: `GET https://api.coinbase.com/v2/prices/<TICKER>-EUR/spot`. A request
//! names only the ticker, never quantities or purchase prices. There is no
//! automatic refresh: the app asks on the user's request ("Kurse aktualisieren")
//! and after a position was added or changed.

use rusqlite::{Connection, params};

use std::collections::HashSet;

use super::{holders_of, materialize, normalize_symbol};
use crate::dates::now_ms;
use crate::error::{CoreError, CoreResult};
use crate::i18n::trf;
use crate::vault::Vault;

const SPOT_URL: &str = "https://api.coinbase.com/v2/prices";

/// What [`Vault::refresh_crypto_prices`] did.
#[derive(Debug, Clone, Default, PartialEq, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CryptoPriceRefresh {
    /// Tickers with a new price.
    pub updated: u32,
    /// Tickers the price source does not know.
    pub unknown: Vec<String>,
    /// The first network problem; the last known prices stay in use.
    pub error: Option<String>,
}

/// One ticker's spot price, or that the source does not know the ticker.
enum Spot {
    Price(f64),
    Unknown,
}

/// `{"data":{"amount":"56537.74","base":"BTC","currency":"EUR"}}` → 56537.74.
fn parse_spot(body: &str, symbol: &str) -> CoreResult<f64> {
    let bad = || CoreError::Network {
        message: trf("Unerwartete Antwort der Kurs-API für {0}", &[&symbol]),
    };
    let v: serde_json::Value = serde_json::from_str(body).map_err(|_| bad())?;
    let data = &v["data"];
    if data["currency"] != "EUR" || data["base"].as_str() != Some(symbol) {
        return Err(bad());
    }
    data["amount"]
        .as_str()
        .and_then(|a| a.parse::<f64>().ok())
        .filter(|p| p.is_finite() && *p > 0.0)
        .ok_or_else(bad)
}

async fn fetch_spot(client: &reqwest::Client, symbol: &str) -> CoreResult<Spot> {
    let resp = client
        .get(format!("{SPOT_URL}/{symbol}-EUR/spot"))
        .send()
        .await?;
    match resp.status().as_u16() {
        200 => {
            let body = crate::connectors::read_text_limited(resp).await?;
            parse_spot(&body, symbol).map(Spot::Price)
        }
        // an unknown pair: 404 (today), 400 (older answers)
        400 | 404 => Ok(Spot::Unknown),
        code => Err(CoreError::Network {
            message: trf("Kurs-API antwortet mit HTTP {0}", &[&code]),
        }),
    }
}

/// A fetched price: ticker, EUR per unit, when it was received (ms).
pub(super) type Quote = (String, f64, i64);

/// Stores fetched prices (a quote older than the stored one is dropped, e.g. a
/// bulk refresh that overlapped a single fetch) and writes again only the
/// crypto accounts holding a ticker that got a new price; their value history
/// records only those tickers (see [`materialize`]). Returns the prices taken.
pub(super) fn store_prices(conn: &Connection, quotes: &[Quote]) -> CoreResult<u32> {
    let mut fresh = HashSet::new();
    for (symbol, eur, received_ms) in quotes {
        let n = conn.execute(
            "INSERT INTO crypto_prices(symbol, price_eur, fetched_ms) VALUES (?1, ?2, ?3)
             ON CONFLICT(symbol) DO UPDATE SET price_eur = excluded.price_eur, fetched_ms = excluded.fetched_ms
              WHERE excluded.fetched_ms > crypto_prices.fetched_ms",
            params![symbol, eur, received_ms],
        )?;
        if n > 0 {
            fresh.insert(symbol.clone());
        }
    }
    let symbols: Vec<String> = fresh.iter().cloned().collect();
    for id in holders_of(conn, &symbols)? {
        materialize(conn, id, Some(&fresh))?;
    }
    Ok(fresh.len() as u32)
}

fn client() -> CoreResult<reqwest::Client> {
    Ok(crate::connectors::client_builder(std::time::Duration::from_secs(15)).build()?)
}

#[uniffi::export(async_runtime = "tokio")]
impl Vault {
    /// Fetches the EUR prices of every ticker held in a crypto account. Network
    /// problems don't fail: the last known prices stay in use.
    pub async fn refresh_crypto_prices(&self) -> Result<CryptoPriceRefresh, CoreError> {
        let symbols: Vec<String> = {
            let db = self.db()?;
            let mut st = db.prepare(
                "SELECT DISTINCT p.symbol FROM crypto_positions p JOIN accounts a ON a.id = p.account_id
                  WHERE p.deleted_ms IS NULL AND a.deleted_ms IS NULL ORDER BY p.symbol",
            )?;
            st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?
        };
        let mut out = CryptoPriceRefresh::default();
        if symbols.is_empty() {
            return Ok(out);
        }
        let client = client()?;
        let mut prices = Vec::new();
        for symbol in symbols {
            match fetch_spot(&client, &symbol).await {
                Ok(Spot::Price(p)) => prices.push((symbol, p, now_ms())),
                Ok(Spot::Unknown) => out.unknown.push(symbol),
                Err(e) => {
                    out.error.get_or_insert(e.to_string());
                }
            }
        }
        let db = self.db()?;
        let tx = db.unchecked_transaction()?;
        out.updated = store_prices(&tx, &prices)?;
        tx.commit()?;
        Ok(out)
    }

    /// Fetches and stores the EUR price of one ticker, e.g. before a new
    /// position is saved. A ticker the price source does not know is an
    /// invalid input; a network problem is a network error (the position can
    /// still be saved, without a price).
    pub async fn fetch_crypto_price(&self, symbol: String) -> Result<f64, CoreError> {
        let symbol = normalize_symbol(&symbol)?;
        let spot = fetch_spot(&client()?, &symbol).await?;
        // stamped on receipt, not after waiting for the vault (see `store_prices`)
        let received_ms = now_ms();
        let price = match spot {
            Spot::Price(p) => p,
            Spot::Unknown => {
                return Err(CoreError::invalid(trf(
                    "Für das Kürzel {0} gibt es keinen Euro-Kurs. Bitte das Kürzel prüfen.",
                    &[&symbol],
                )));
            }
        };
        let db = self.db()?;
        let tx = db.unchecked_transaction()?;
        store_prices(&tx, &[(symbol, price, received_ms)])?;
        tx.commit()?;
        Ok(price)
    }
}

#[cfg(test)]
mod tests {
    // synthetic data
    use super::*;

    #[test]
    fn spot_answers_are_read_strictly() {
        let ok = r#"{"data":{"amount":"0.0000034","base":"SHIB","currency":"EUR"}}"#;
        assert_eq!(parse_spot(ok, "SHIB").unwrap(), 0.000_003_4);
        for body in [
            r#"{"data":{"amount":"1.5","base":"ETH","currency":"EUR"}}"#, // other ticker
            r#"{"data":{"amount":"1.5","base":"SHIB","currency":"USD"}}"#, // other currency
            r#"{"data":{"amount":"0","base":"SHIB","currency":"EUR"}}"#,  // no price
            r#"{"data":{"amount":"abc","base":"SHIB","currency":"EUR"}}"#,
            r#"{"error":"not found"}"#,
            "<html>",
        ] {
            assert!(parse_spot(body, "SHIB").is_err(), "{body}");
        }
    }

    #[test]
    fn stored_prices_value_every_account_holding_the_ticker() {
        let p = crate::test_support::thread_temp_path("crypto-prices.db");
        let v = Vault::open(p.to_string_lossy().to_string(), vec![7u8; 32]).unwrap();
        let a = v.create_crypto_account("A".into()).unwrap();
        let b = v.create_crypto_account("B".into()).unwrap();
        v.save_crypto_position(a, None, "BTC".into(), 1.0, None)
            .unwrap();
        v.save_crypto_position(b, None, "BTC".into(), 0.25, None)
            .unwrap();
        {
            let db = v.db().unwrap();
            store_prices(&db, &[("BTC".into(), 60_000.0, now_ms())]).unwrap();
        }
        let value = |acc| v.list_crypto_positions(acc).unwrap()[0].value_cents;
        assert_eq!((value(a), value(b)), (6_000_000, 1_500_000));
        assert_eq!(
            v.list_holdings(b).unwrap()[0].current_price_cents,
            6_000_000
        );
    }

    #[test]
    fn an_older_quote_is_dropped_and_only_holders_of_a_new_price_are_written() {
        let p = crate::test_support::thread_temp_path("crypto-quotes.db");
        let v = Vault::open(p.to_string_lossy().to_string(), vec![7u8; 32]).unwrap();
        let btc = v.create_crypto_account("A".into()).unwrap();
        let eth = v.create_crypto_account("B".into()).unwrap();
        v.save_crypto_position(btc, None, "BTC".into(), 1.0, None)
            .unwrap();
        v.save_crypto_position(eth, None, "ETH".into(), 1.0, None)
            .unwrap();
        let db = v.db().unwrap();
        let observations = |acc: i64| -> i64 {
            db.query_row(
                "SELECT count(*) FROM depot_observations WHERE account_id = ?1",
                [acc],
                |r| r.get(0),
            )
            .unwrap()
        };
        store_prices(&db, &[("ETH".into(), 3_000.0, 1_000)]).unwrap();
        let eth_before = observations(eth);
        store_prices(&db, &[("BTC".into(), 60_000.0, 2_000)]).unwrap();
        assert_eq!(
            observations(eth),
            eth_before,
            "ETH holder not written again"
        );
        // a bulk refresh that received its quote before the single fetch
        assert_eq!(
            store_prices(&db, &[("BTC".into(), 50_000.0, 1_500)]).unwrap(),
            0,
            "not taken"
        );
        let price: f64 = db
            .query_row(
                "SELECT price_eur FROM crypto_prices WHERE symbol = 'BTC'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(price, 60_000.0);
    }

    #[test]
    fn a_failed_ticker_is_not_recorded_with_its_cached_price() {
        let p = crate::test_support::thread_temp_path("crypto-mixed.db");
        let v = Vault::open(p.to_string_lossy().to_string(), vec![7u8; 32]).unwrap();
        let acc = v.create_crypto_account("A".into()).unwrap();
        v.save_crypto_position(acc, None, "BTC".into(), 1.0, None)
            .unwrap();
        v.save_crypto_position(acc, None, "ETH".into(), 1.0, None)
            .unwrap();
        let db = v.db().unwrap();
        store_prices(
            &db,
            &[
                ("BTC".into(), 60_000.0, 1_000),
                ("ETH".into(), 3_000.0, 1_000),
            ],
        )
        .unwrap();
        let count = |kind: &str, name: &str| -> i64 {
            db.query_row(
                "SELECT count(*) FROM depot_observations WHERE account_id = ?1 AND kind = ?2 AND name = ?3",
                params![acc, kind, name],
                |r| r.get(0),
            )
            .unwrap()
        };
        let (btc, eth, total) = (
            count("position", "BTC"),
            count("position", "ETH"),
            count("total", ""),
        );
        // the next refresh gets BTC, ETH times out
        store_prices(&db, &[("BTC".into(), 61_000.0, 2_000)]).unwrap();
        assert_eq!(count("position", "BTC"), btc + 1);
        assert_eq!(
            count("position", "ETH"),
            eth,
            "no observation from the cached price"
        );
        assert_eq!(count("total", ""), total, "no total with a cached price");
    }
}
