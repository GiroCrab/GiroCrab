//! EUR prices of crypto holdings (#1836) from the public Coinbase API, without
//! a key: `GET https://api.coinbase.com/v2/prices/<TICKER>-EUR/spot`. A request
//! names only the ticker, never quantities or purchase prices. There is no
//! automatic refresh: the app asks on the user's request ("Kurse aktualisieren")
//! and after a position was added or changed.

use rusqlite::{Connection, params};

use super::{crypto_account_ids, materialize, normalize_symbol};
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

/// Stores fetched prices and writes the crypto accounts again.
pub(super) fn store_prices(conn: &Connection, prices: &[(String, f64)]) -> CoreResult<()> {
    let now = now_ms();
    for (symbol, eur) in prices {
        conn.execute(
            "INSERT INTO crypto_prices(symbol, price_eur, fetched_ms) VALUES (?1, ?2, ?3)
             ON CONFLICT(symbol) DO UPDATE SET price_eur = excluded.price_eur, fetched_ms = excluded.fetched_ms",
            params![symbol, eur, now],
        )?;
    }
    for id in crypto_account_ids(conn)? {
        materialize(conn, id)?;
    }
    Ok(())
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
                Ok(Spot::Price(p)) => prices.push((symbol, p)),
                Ok(Spot::Unknown) => out.unknown.push(symbol),
                Err(e) => {
                    out.error.get_or_insert(e.to_string());
                }
            }
        }
        out.updated = prices.len() as u32;
        let db = self.db()?;
        let tx = db.unchecked_transaction()?;
        store_prices(&tx, &prices)?;
        tx.commit()?;
        Ok(out)
    }

    /// Fetches and stores the EUR price of one ticker, e.g. before a new
    /// position is saved. A ticker the price source does not know is an
    /// invalid input; a network problem is a network error (the position can
    /// still be saved, without a price).
    pub async fn fetch_crypto_price(&self, symbol: String) -> Result<f64, CoreError> {
        let symbol = normalize_symbol(&symbol)?;
        let price = match fetch_spot(&client()?, &symbol).await? {
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
        store_prices(&tx, &[(symbol, price)])?;
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
            store_prices(&db, &[("BTC".into(), 60_000.0)]).unwrap();
        }
        let value = |acc| v.list_crypto_positions(acc).unwrap()[0].value_cents;
        assert_eq!((value(a), value(b)), (6_000_000, 1_500_000));
        assert_eq!(
            v.list_holdings(b).unwrap()[0].current_price_cents,
            6_000_000
        );
    }
}
