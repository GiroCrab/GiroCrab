//! AXA Mitarbeiter Shareplan (employee savings plan portal): one depot position per fund of the
//! plan with quantity, average purchase price and value. The data are read inside the embedded
//! browser; only the selected position fields cross the bridge. The portal is the only
//! interface (no FinTS/PSD2), see `docs/axashareplan-connector.md`.
use std::{collections::HashSet, sync::Arc};

use serde_json::Value;

use super::{FetchResult, RemoteAccount, RemoteHolding, SyncSummary};
use crate::{
    error::{CoreError, CoreResult},
    i18n::tr,
    models::{AccountKind, ConnectorKind, Holding},
    vault::Vault,
};

/// Stable reference of the single securities account; it carries no personal number.
const ACCOUNT_REF: &str = "depot:axashareplan";

fn invalid() -> CoreError {
    CoreError::InvalidInput {
        message: tr("AXA Shareplan: Unvollständige oder ungültige Portalantwort. Es wurden keine Daten übernommen.").into(),
    }
}

fn text<'a>(v: &'a Value, key: &str) -> CoreResult<&'a str> {
    v[key]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(invalid)
}

fn date(v: &Value, key: &str) -> CoreResult<String> {
    let s = text(v, key)?.get(..10).ok_or_else(invalid)?;
    jiff::civil::Date::strptime("%Y-%m-%d", s).map_err(|_| invalid())?;
    Ok(s.into())
}

/// A whole number of cents (the portal reports minor units), never negative.
fn cents(v: &Value, key: &str) -> CoreResult<i64> {
    v[key]
        .as_i64()
        .filter(|n| (0..1_000_000_000_000).contains(n))
        .ok_or_else(invalid)
}

/// One position: the shares of one fund of the plan.
struct Position {
    isin: String,
    name: String,
    quantity: f64,
    price: i64,
    average: i64,
    invested: i64,
    value: i64,
}

/// The portal's instrument id (digits only); it only keeps the list free of duplicates.
fn instrument_id(item: &Value) -> CoreResult<String> {
    let s = item["id"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| item["id"].as_u64().map(|n| n.to_string()))
        .ok_or_else(invalid)?;
    if s.is_empty() || s.len() > 12 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    Ok(s)
}

/// The shape of an ISIN (two capital letters, nine capital letters or digits, one digit). The
/// plan's funds carry internal identifiers whose check digit is not the ISO one, so only the
/// shape is checked.
fn isin_shape(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 12
        && b[..2].iter().all(u8::is_ascii_uppercase)
        && b[2..11]
            .iter()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        && b[11].is_ascii_digit()
}

/// Cents of `quantity x unit price`, to compare with a reported total (the unit price is rounded
/// to a cent by the portal: up to half a cent per share, plus one for the total itself).
fn within(total: i64, quantity: f64, unit: i64) -> bool {
    let expected = (quantity * unit as f64).round() as i64;
    (total - expected).abs() <= 1 + (quantity * 0.5).ceil() as i64
}

fn positions(depot: &Value) -> CoreResult<Vec<Position>> {
    let items = depot["items"].as_array().ok_or_else(invalid)?;
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let id = instrument_id(item)?;
        if !seen.insert(id) {
            return Err(invalid());
        }
        // a fund without a usable identifier is allowed (the name still identifies it)
        let isin = match item["isin"].as_str() {
            _ if item["isin"].is_null() => String::new(),
            Some(s) if isin_shape(s) => s.to_string(),
            _ => return Err(invalid()),
        };
        let name = text(item, "name")?.trim();
        if name.chars().count() > 200 {
            return Err(invalid());
        }
        let quantity = item["quantity"]
            .as_f64()
            .filter(|q| q.is_finite() && *q > 0.0 && *q < 1e9)
            .ok_or_else(invalid)?;
        let price = cents(item, "priceCents")?;
        let average = cents(item, "averagePriceCents")?;
        let invested = cents(item, "investedCents")?;
        let value = cents(item, "valueCents")?;
        // value = shares x price and cost = shares x average price
        if !within(value, quantity, price) || !within(invested, quantity, average) {
            return Err(invalid());
        }
        out.push(Position {
            isin,
            name: name.into(),
            quantity,
            price,
            average,
            invested,
            value,
        });
    }
    Ok(out)
}

fn holding(p: &Position) -> Holding {
    let (profit, rel) = match p.invested {
        0 => (None, None),
        cost => (
            Some(p.value - cost),
            Some((p.value - cost) as f64 / cost as f64 * 100.0),
        ),
    };
    Holding {
        isin: p.isin.clone(),
        name: p.name.clone(),
        quantity: p.quantity,
        current_price_cents: p.price,
        purchase_price_cents: p.average,
        value_cents: p.value,
        profit_loss_purchase_cents: profit,
        profit_loss_purchase_rel: rel,
        ..Default::default()
    }
}

fn snapshot(json: &str) -> CoreResult<FetchResult> {
    let root: Value = serde_json::from_str(json).map_err(|_| invalid())?;
    if root["version"].as_u64() != Some(1) {
        return Err(invalid());
    }
    let as_of = date(&root, "asOf")?;
    let positions = positions(&root["depot"])?;
    let total: i64 = positions.iter().map(|p| p.value).sum();
    let mut result = FetchResult {
        bank_name: Some("AXA Mitarbeiter Shareplan".into()),
        ..Default::default()
    };
    result.accounts.push(RemoteAccount {
        external_ref: ACCOUNT_REF.into(),
        name: tr("Mitarbeiter Shareplan").into(),
        owner: String::new(),
        iban: String::new(),
        account_number: String::new(),
        currency: "EUR".into(),
        kind: AccountKind::Brokerage,
        balance_cents: total,
        balance_date: as_of,
    });
    for p in &positions {
        result.holdings.push(RemoteHolding {
            classification: Default::default(),
            direct_crypto: false,
            account_ref: ACCOUNT_REF.into(),
            holding: holding(p),
        });
    }
    // an empty list clears positions that were sold or transferred since the last fetch
    result.holdings_fetched.push(ACCOUNT_REF.into());
    Ok(result)
}

#[uniffi::export]
impl Vault {
    pub fn import_axashareplan_snapshot(
        self: Arc<Self>,
        connection_id: i64,
        snapshot_json: String,
    ) -> CoreResult<SyncSummary> {
        if self.portal_connection(connection_id)?.0 != ConnectorKind::AxaShareplan {
            return Err(invalid());
        }
        crate::banking_gate::check_connector("axashareplan")?;
        let summary = super::store_fetch(
            &self,
            ConnectorKind::AxaShareplan,
            connection_id,
            snapshot(&snapshot_json)?,
        )?;
        let _ = self.checkpoint();
        Ok(summary)
    }
}

/// Placeholder connector: the login and the page reads run in the embedded browser.
pub struct WebOnlyConnector;

#[async_trait::async_trait]
impl super::BankConnector for WebOnlyConnector {
    async fn begin(&mut self, _ctx: &super::ConnectContext) -> CoreResult<super::Phase> {
        Err(CoreError::BrowserLoginRequired {
            message: tr(
                "Bitte beim AXA Mitarbeiter Shareplan im Browserfenster anmelden und die Daten abrufen.",
            )
            .into(),
        })
    }
    async fn respond(
        &mut self,
        ctx: &super::ConnectContext,
        _response: super::ChallengeResponse,
    ) -> CoreResult<super::Phase> {
        self.begin(ctx).await
    }
    async fn fetch(&mut self, _ctx: &super::ConnectContext) -> CoreResult<FetchResult> {
        Err(CoreError::BrowserLoginRequired {
            message: tr("Bitte beim AXA Mitarbeiter Shareplan im Browserfenster abrufen.").into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// Prices in cents: `price` is the market price, `avg` the average purchase price per share.
    fn item(id: &str, quantity: f64, price: i64, avg: i64, invested: i64, value: i64) -> Value {
        json!({"id":id,"isin":"QS0000000001","name":"Testfonds 2022","quantity":quantity,
            "priceCents":price,"averagePriceCents":avg,"investedCents":invested,"valueCents":value})
    }

    fn fixture() -> Value {
        json!({"version":1,"asOf":"2026-11-24","depot":{"items":[
            item("11", 200.0, 7000, 2000, 400_000, 1_400_000),
            item("12", 100.0, 5000, 4000, 400_000, 500_000)]}})
    }

    #[test]
    fn maps_the_positions_and_the_total() {
        let r = snapshot(&fixture().to_string()).unwrap();
        assert_eq!(r.accounts.len(), 1);
        assert_eq!(r.accounts[0].kind, AccountKind::Brokerage);
        assert_eq!(r.accounts[0].balance_cents, 1_900_000);
        assert_eq!(r.accounts[0].external_ref, ACCOUNT_REF);
        assert_eq!(r.holdings.len(), 2);
        let h = &r.holdings[0].holding;
        assert_eq!((h.isin.as_str(), h.quantity), ("QS0000000001", 200.0));
        assert_eq!(
            (h.current_price_cents, h.purchase_price_cents),
            (7000, 2000)
        );
        assert_eq!(h.value_cents, 1_400_000);
        // invested 4 000 EUR, worth 14 000 EUR
        assert_eq!(h.profit_loss_purchase_cents, Some(1_000_000));
        assert!((h.profit_loss_purchase_rel.unwrap() - 250.0).abs() < 0.001);
        assert_eq!(r.holdings_fetched, vec![ACCOUNT_REF.to_string()]);
    }

    #[test]
    fn fractional_shares_and_cent_rounding_are_tolerated() {
        // 150.4321 shares at 50.00 EUR: the exact product is 7 521.605, the portal says 7 521.61
        let mut f = fixture();
        f["depot"]["items"] = json!([item("1", 150.4321, 5000, 1800, 270_778, 752_161)]);
        assert!(snapshot(&f.to_string()).is_ok());
    }

    #[test]
    fn an_empty_depot_clears_the_positions() {
        let r = snapshot(r#"{"version":1,"asOf":"2026-11-24","depot":{"items":[]}}"#).unwrap();
        assert!(r.holdings.is_empty());
        assert_eq!(r.accounts[0].balance_cents, 0);
        assert_eq!(r.holdings_fetched, vec![ACCOUNT_REF.to_string()]);
    }

    #[test]
    fn a_position_without_cost_has_no_profit_and_no_isin_is_allowed() {
        let mut one = item("1", 10.0, 1000, 0, 0, 10_000);
        one["isin"] = Value::Null;
        let r =
            snapshot(&json!({"version":1,"asOf":"2026-11-24","depot":{"items":[one]}}).to_string())
                .unwrap();
        let h = &r.holdings[0].holding;
        assert_eq!(h.isin, "");
        assert_eq!(
            (h.profit_loss_purchase_cents, h.profit_loss_purchase_rel),
            (None, None)
        );
    }

    #[test]
    fn rejects_anything_that_does_not_add_up() {
        let bad = |f: Value| assert!(snapshot(&f.to_string()).is_err(), "{f}");
        let with =
            |items: Vec<Value>| json!({"version":1,"asOf":"2026-11-24","depot":{"items":items}});
        bad(json!({"version":2,"asOf":"2026-11-24","depot":{"items":[]}}));
        bad(json!({"version":1,"asOf":"gestern","depot":{"items":[]}}));
        bad(json!({"version":1,"asOf":"2026-11-24","depot":{}}));
        // value or cost do not match shares x price
        bad(with(vec![item("1", 10.0, 1000, 500, 5000, 20_000)]));
        bad(with(vec![item("1", 10.0, 1000, 500, 9000, 10_000)]));
        // duplicate or bad id, bad identifier shape, no or negative shares, negative amounts
        bad(with(vec![
            item("1", 1.0, 100, 100, 100, 100),
            item("1", 1.0, 100, 100, 100, 100),
        ]));
        bad(with(vec![item("a1", 1.0, 100, 100, 100, 100)]));
        let mut odd = item("1", 1.0, 100, 100, 100, 100);
        odd["isin"] = json!("not-an-isin");
        bad(with(vec![odd]));
        bad(with(vec![item("1", 0.0, 100, 100, 0, 0)]));
        bad(with(vec![item("1", -1.0, 100, 100, -100, -100)]));
        bad(with(vec![item("1", 1.0, -100, 100, 100, -100)]));
        let mut fractional_cents = item("1", 1.0, 100, 100, 100, 100);
        fractional_cents["valueCents"] = json!(100.5);
        bad(with(vec![fractional_cents]));
        assert!(snapshot("not json").is_err());
    }

    #[test]
    fn the_snapshot_is_stored_and_replaced_by_the_next_one() {
        let dir = crate::test_support::TempDir::new("axashareplan");
        let vault =
            Vault::open(dir.join("vault.db").to_string_lossy().into(), vec![7; 32]).unwrap();
        let connection = vault
            .add_bank_connection(ConnectorKind::AxaShareplan, "Test".into(), "x".into())
            .unwrap();
        let sum = vault
            .clone()
            .import_axashareplan_snapshot(connection, fixture().to_string())
            .unwrap();
        assert_eq!(sum.accounts_updated, 1);
        // the same snapshot again changes nothing; a smaller one removes the sold position
        let mut next = fixture();
        next["depot"]["items"] = json!([item("11", 200.0, 7000, 2000, 400_000, 1_400_000)]);
        vault
            .clone()
            .import_axashareplan_snapshot(connection, next.to_string())
            .unwrap();
        let rows: i64 = vault
            .db()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM holdings", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1);
        // another connector kind is refused
        let other = vault
            .add_bank_connection(ConnectorKind::Sharinbox, "Test".into(), "x".into())
            .unwrap();
        assert!(
            vault
                .clone()
                .import_axashareplan_snapshot(other, fixture().to_string())
                .is_err()
        );
    }
}
