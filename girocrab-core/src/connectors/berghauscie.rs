//! Berghaus & Cie (wine investment portal): the wine depot with bottles, purchase price and
//! valuation. The data are read inside the embedded browser; only the selected depot fields
//! cross the bridge. The portal is the only interface (no FinTS/PSD2), see
//! `docs/berghauscie-connector.md`.
use std::{collections::HashSet, sync::Arc};

use serde_json::Value;

use super::{FetchResult, RemoteAccount, RemoteHolding, SyncSummary};
use crate::asset_classification::{AssetClass, ClassificationValues, InstrumentKind};
use crate::{
    error::{CoreError, CoreResult},
    i18n::tr,
    models::{AccountKind, ConnectorKind, Holding},
    vault::Vault,
};

/// Stable reference of the single securities account; it carries no personal number.
const ACCOUNT_REF: &str = "depot:berghauscie";

fn invalid() -> CoreError {
    CoreError::InvalidInput {
        message: tr("Berghaus: Unvollständige oder ungültige Portalantwort. Es wurden keine Daten übernommen.").into(),
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

fn number(v: &Value, key: &str) -> CoreResult<f64> {
    v[key]
        .as_f64()
        .filter(|n| n.is_finite() && n.abs() < 1e12)
        .ok_or_else(invalid)
}

/// Monetary values are rounded once at the boundary, never quantities.
fn cents_of(n: f64) -> i64 {
    (n * 100.0).round() as i64
}

/// One wine position: the bottles of one portal entry (cases x bottles per case), with the
/// purchase price and the valuation per bottle.
struct Position {
    lwin: String,
    name: String,
    bottles: f64,
    price: f64,
    purchase: f64,
    value: i64,
}

/// A position id is the portal's own number (digits only); it only keeps the list free of
/// duplicates. The LWIN (Liv-ex wine identifier) is shown as the position's WKN.
fn digits(item: &Value, key: &str, max: usize) -> CoreResult<String> {
    let s = item[key]
        .as_str()
        .map(str::to_owned)
        .or_else(|| item[key].as_u64().map(|n| n.to_string()))
        .ok_or_else(invalid)?;
    if s.is_empty() || s.len() > max || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    Ok(s)
}

fn positions(depot: &Value) -> CoreResult<Vec<Position>> {
    let items = depot["items"].as_array().ok_or_else(invalid)?;
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let id = digits(item, "id", 12)?;
        if !seen.insert(id.clone()) {
            return Err(invalid());
        }
        // an entry without an LWIN is allowed (the portal may not know one)
        let lwin = match item["lwin"].is_null() {
            true => String::new(),
            false => digits(item, "lwin", 12)?,
        };
        let name = text(item, "name")?.trim();
        if name.chars().count() > 200 {
            return Err(invalid());
        }
        date(item, "valueDate")?;
        let bottles = number(item, "bottles")?;
        let price = number(item, "price")?;
        let purchase = number(item, "purchasePrice")?;
        let value = cents_of(number(item, "value")?);
        // whole bottles only; a position without bottles is not a position
        if bottles < 1.0 || bottles.fract() != 0.0 || price < 0.0 || purchase < 0.0 || value < 0 {
            return Err(invalid());
        }
        // value = bottles x price; the page script rounds the price (size factor) to cents
        let expected = (bottles * price * 100.0).round() as i64;
        if (value - expected).abs() > 1 + (bottles * 0.5).ceil() as i64 {
            return Err(invalid());
        }
        out.push(Position {
            lwin,
            name: name.into(),
            bottles,
            price,
            purchase,
            value,
        });
    }
    Ok(out)
}

fn holding(p: &Position) -> Holding {
    let cost = cents_of(p.bottles * p.purchase);
    let (profit, rel) = match cost {
        0 => (None, None),
        _ => (
            Some(p.value - cost),
            Some((p.value - cost) as f64 / cost as f64 * 100.0),
        ),
    };
    Holding {
        isin: String::new(),
        wkn: p.lwin.clone(),
        name: p.name.clone(),
        quantity: p.bottles,
        current_price_cents: cents_of(p.price),
        purchase_price_cents: cents_of(p.purchase),
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
        bank_name: Some("Berghaus & Cie".into()),
        ..Default::default()
    };
    result.accounts.push(RemoteAccount {
        external_ref: ACCOUNT_REF.into(),
        name: tr("Weindepot").into(),
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
            classification: ClassificationValues {
                asset_class: Some(AssetClass::Other),
                instrument: Some(InstrumentKind::DirectHolding),
            },
            direct_crypto: false,
            account_ref: ACCOUNT_REF.into(),
            holding: holding(p),
        });
    }
    // an empty list clears positions that were delivered or sold since the last fetch
    result.holdings_fetched.push(ACCOUNT_REF.into());
    Ok(result)
}

#[uniffi::export]
impl Vault {
    pub fn import_berghauscie_snapshot(
        self: Arc<Self>,
        connection_id: i64,
        snapshot_json: String,
    ) -> CoreResult<SyncSummary> {
        if self.portal_connection(connection_id)?.0 != ConnectorKind::BerghausCie {
            return Err(invalid());
        }
        crate::banking_gate::check_connector("berghauscie")?;
        let summary = super::store_fetch(
            &self,
            ConnectorKind::BerghausCie,
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
                "Bitte bei Berghaus & Cie im Browserfenster anmelden und die Daten abrufen.",
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
            message: tr("Bitte bei Berghaus & Cie im Browserfenster abrufen.").into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn item(id: &str, bottles: f64, price: f64, purchase: f64, value: f64) -> Value {
        json!({"id":id,"lwin":"1100001","name":"Testgut Cuvée 2020 (0,75 l)","bottles":bottles,
            "price":price,"purchasePrice":purchase,"valueDate":"2026-07-01","value":value})
    }

    fn fixture() -> Value {
        json!({"version":1,"asOf":"2026-11-23","depot":{"items":[
            item("101", 12.0, 200.0, 150.0, 2400.0),
            item("102", 2.0, 1580.0, 1549.0, 3160.0)]}})
    }

    #[test]
    fn maps_the_wines_and_the_total() {
        let r = snapshot(&fixture().to_string()).unwrap();
        assert_eq!(r.accounts.len(), 1);
        assert_eq!(r.accounts[0].kind, AccountKind::Brokerage);
        assert_eq!(r.accounts[0].balance_cents, 556_000);
        assert_eq!(r.accounts[0].external_ref, ACCOUNT_REF);
        assert_eq!(r.holdings.len(), 2);
        let h = &r.holdings[0].holding;
        assert_eq!(
            (h.isin.as_str(), h.wkn.as_str(), h.quantity),
            ("", "1100001", 12.0)
        );
        assert_eq!(
            (h.current_price_cents, h.purchase_price_cents),
            (20_000, 15_000)
        );
        assert_eq!(h.value_cents, 240_000);
        // 12 bottles at 150 EUR cost 1 800 EUR; they are worth 2 400 EUR
        assert_eq!(h.profit_loss_purchase_cents, Some(60_000));
        assert!((h.profit_loss_purchase_rel.unwrap() - 33.333).abs() < 0.01);
        assert_eq!(r.holdings_fetched, vec![ACCOUNT_REF.to_string()]);
    }

    #[test]
    fn an_empty_depot_clears_the_positions() {
        let r = snapshot(r#"{"version":1,"asOf":"2026-11-23","depot":{"items":[]}}"#).unwrap();
        assert!(r.holdings.is_empty());
        assert_eq!(r.accounts[0].balance_cents, 0);
        assert_eq!(r.holdings_fetched, vec![ACCOUNT_REF.to_string()]);
    }

    #[test]
    fn a_position_without_purchase_price_has_no_profit() {
        let mut f = fixture();
        f["depot"]["items"] = json!([item("101", 6.0, 10.0, 0.0, 60.0)]);
        let h = &snapshot(&f.to_string()).unwrap().holdings[0].holding;
        assert_eq!(
            (h.profit_loss_purchase_cents, h.profit_loss_purchase_rel),
            (None, None)
        );
    }

    #[test]
    fn rejects_anything_that_does_not_add_up() {
        let bad = |f: Value| assert!(snapshot(&f.to_string()).is_err());
        let with =
            |items: Vec<Value>| json!({"version":1,"asOf":"2026-11-23","depot":{"items":items}});
        bad(json!({"version":2,"asOf":"2026-11-23","depot":{"items":[]}}));
        bad(json!({"version":1,"asOf":"gestern","depot":{"items":[]}}));
        bad(json!({"version":1,"asOf":"2026-11-23","depot":{}}));
        // value does not match bottles x price
        bad(with(vec![item("1", 2.0, 10.0, 5.0, 100.0)]));
        // duplicate position, bad id, bad LWIN, partial or no bottles, negative price
        bad(with(vec![
            item("1", 1.0, 1.0, 1.0, 1.0),
            item("1", 1.0, 1.0, 1.0, 1.0),
        ]));
        bad(with(vec![item("a1", 1.0, 1.0, 1.0, 1.0)]));
        let mut odd = item("1", 1.0, 1.0, 1.0, 1.0);
        odd["lwin"] = json!("11-22");
        bad(with(vec![odd]));
        bad(with(vec![item("1", 0.0, 1.0, 1.0, 0.0)]));
        bad(with(vec![item("1", 1.5, 2.0, 1.0, 3.0)]));
        bad(with(vec![item("1", 1.0, -1.0, 1.0, -1.0)]));
        assert!(snapshot("not json").is_err());
    }

    #[test]
    fn an_unknown_lwin_is_allowed() {
        let mut one = item("1", 1.0, 10.0, 5.0, 10.0);
        one["lwin"] = Value::Null;
        let r =
            snapshot(&json!({"version":1,"asOf":"2026-11-23","depot":{"items":[one]}}).to_string())
                .unwrap();
        assert_eq!(r.holdings[0].holding.wkn, "");
    }

    #[test]
    fn the_snapshot_is_stored_and_replaced_by_the_next_one() {
        let dir = crate::test_support::TempDir::new("berghauscie");
        let vault =
            Vault::open(dir.join("vault.db").to_string_lossy().into(), vec![7; 32]).unwrap();
        let connection = vault
            .add_bank_connection(ConnectorKind::BerghausCie, "Test".into(), "x".into())
            .unwrap();
        let sum = vault
            .clone()
            .import_berghauscie_snapshot(connection, fixture().to_string())
            .unwrap();
        assert_eq!(sum.accounts_updated, 1);
        // the same snapshot again changes nothing; a smaller one removes the sold position
        let mut next = fixture();
        next["depot"]["items"] = json!([item("101", 12.0, 200.0, 150.0, 2400.0)]);
        vault
            .clone()
            .import_berghauscie_snapshot(connection, next.to_string())
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
                .import_berghauscie_snapshot(other, fixture().to_string())
                .is_err()
        );
    }
}
