//! Depot positions edited by the user (#1987): the purchase price per unit of a
//! position, for depots whose bank sends none (FinTS / MT535).
//!
//! The value lives in `holdings.purchase_price_cents` like a fetched one. A later
//! fetch without a purchase price keeps it (#1812, `persist::kept_purchase_for`);
//! a purchase price the bank sends wins.

use rusqlite::params;

use crate::error::CoreError;
use crate::i18n::tr;
use crate::vault::Vault;

#[uniffi::export]
impl Vault {
    /// Sets the purchase price per unit (account currency, cents) of a depot
    /// position, found by ISIN, else by WKN. 0 removes it. Every stored row of
    /// the position gets it, so a fetch keeps it unambiguously. The bank's
    /// profit/loss against the purchase price is dropped (it would win over the
    /// new price), and so is the native purchase price of a position in a
    /// foreign currency (the value is entered in the account currency).
    pub fn set_holding_purchase_price(
        &self,
        account_id: i64,
        isin: String,
        wkn: String,
        purchase_price_cents: i64,
    ) -> Result<(), CoreError> {
        if purchase_price_cents < 0 {
            return Err(CoreError::invalid(tr(
                "Der Einstandskurs darf nicht negativ sein.",
            )));
        }
        let (isin, wkn) = (isin.trim().to_uppercase(), wkn.trim().to_uppercase());
        if isin.is_empty() && wkn.is_empty() {
            return Err(CoreError::invalid(tr(
                "Die Position hat weder ISIN noch WKN.",
            )));
        }
        let db = self.db()?;
        let n = db.execute(
            "UPDATE holdings SET purchase_price_cents = ?1,
                    native_purchase_price_cents = NULL,
                    profit_loss_purchase_cents = NULL, profit_loss_purchase_rel = NULL
              WHERE account_id = ?2
                AND (CASE WHEN ?3 <> '' THEN isin = ?3 ELSE isin = '' AND wkn = ?4 END)
                AND EXISTS (SELECT 1 FROM accounts a WHERE a.id = ?2 AND a.deleted_ms IS NULL)",
            params![purchase_price_cents, account_id, isin, wkn],
        )?;
        if n == 0 {
            return Err(CoreError::NotFound {
                what: tr("Depotposition").into(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::connectors::store_fetch;
    use crate::connectors::{FetchResult, RemoteAccount, RemoteHolding};
    use crate::models::{AccountKind, ConnectorKind, Holding};
    use crate::vault::Vault;

    /// A depot with one position as a FinTS fetch delivers it: no purchase price.
    fn fetch(isin: &str, wkn: &str, price: i64, purchase: i64) -> FetchResult {
        FetchResult {
            accounts: vec![RemoteAccount {
                external_ref: "0012345678".into(),
                name: "Depot".into(),
                owner: String::new(),
                iban: String::new(),
                account_number: "0012345678".into(),
                currency: "EUR".into(),
                kind: AccountKind::Brokerage,
                balance_cents: price * 2,
                balance_date: "2026-11-10".into(),
            }],
            holdings: vec![RemoteHolding {
                classification: Default::default(),
                direct_crypto: false,
                account_ref: "0012345678".into(),
                holding: Holding {
                    isin: isin.into(),
                    wkn: wkn.into(),
                    name: "X".into(),
                    quantity: 2.0,
                    current_price_cents: price,
                    purchase_price_cents: purchase,
                    value_cents: price * 2,
                    profit_loss_purchase_cents: (purchase > 0).then_some(price * 2 - purchase * 2),
                    ..Default::default()
                },
            }],
            ..Default::default()
        }
    }

    fn depot(name: &str) -> (crate::test_support::TempDir, std::sync::Arc<Vault>, i64) {
        let dir = crate::test_support::TempDir::new(name);
        let v = Vault::open(
            dir.join("v.db").to_string_lossy().to_string(),
            vec![3u8; 32],
        )
        .unwrap();
        let conn = v
            .add_bank_connection(ConnectorKind::Comdirect, String::new(), "login".into())
            .unwrap();
        (dir, v, conn)
    }

    #[test]
    fn a_purchase_price_set_by_hand_survives_a_fetch_without_one() {
        // synthetic data
        let (_dir, v, conn) = depot("girocrab-manual-purchase");
        store_fetch(
            &v,
            ConnectorKind::Comdirect,
            conn,
            fetch("DE0001234567", "", 100, 0),
        )
        .unwrap();
        let acc = v.list_accounts().unwrap().pop().unwrap().id;
        assert_eq!(v.list_holdings(acc).unwrap()[0].purchase_price_cents, 0);

        v.set_holding_purchase_price(acc, " de0001234567 ".into(), String::new(), 80)
            .unwrap();
        assert_eq!(v.list_holdings(acc).unwrap()[0].purchase_price_cents, 80);

        store_fetch(
            &v,
            ConnectorKind::Comdirect,
            conn,
            fetch("DE0001234567", "", 120, 0),
        )
        .unwrap();
        let h = &v.list_holdings(acc).unwrap()[0];
        assert_eq!(h.current_price_cents, 120, "the price is updated");
        assert_eq!(h.purchase_price_cents, 80, "the purchase price is kept");

        // a purchase price the bank sends wins
        store_fetch(
            &v,
            ConnectorKind::Comdirect,
            conn,
            fetch("DE0001234567", "", 130, 90),
        )
        .unwrap();
        assert_eq!(v.list_holdings(acc).unwrap()[0].purchase_price_cents, 90);
    }

    #[test]
    fn setting_drops_the_banks_profit_figure_and_zero_removes_the_price() {
        let (_dir, v, conn) = depot("girocrab-manual-purchase-pl");
        store_fetch(
            &v,
            ConnectorKind::Comdirect,
            conn,
            fetch("DE0001234567", "", 100, 70),
        )
        .unwrap();
        let acc = v.list_accounts().unwrap().pop().unwrap().id;
        assert!(
            v.list_holdings(acc).unwrap()[0]
                .profit_loss_purchase_cents
                .is_some()
        );

        v.set_holding_purchase_price(acc, "DE0001234567".into(), String::new(), 80)
            .unwrap();
        let h = &v.list_holdings(acc).unwrap()[0];
        assert_eq!(h.purchase_price_cents, 80);
        assert_eq!(
            h.profit_loss_purchase_cents, None,
            "the gain is computed from the new price"
        );

        v.set_holding_purchase_price(acc, "DE0001234567".into(), String::new(), 0)
            .unwrap();
        assert_eq!(v.list_holdings(acc).unwrap()[0].purchase_price_cents, 0);
    }

    #[test]
    fn a_position_without_isin_is_found_by_wkn_and_unknown_ones_are_errors() {
        let (_dir, v, conn) = depot("girocrab-manual-purchase-wkn");
        store_fetch(
            &v,
            ConnectorKind::Comdirect,
            conn,
            fetch("", "A0B1C2", 100, 0),
        )
        .unwrap();
        let acc = v.list_accounts().unwrap().pop().unwrap().id;

        v.set_holding_purchase_price(acc, String::new(), "a0b1c2".into(), 55)
            .unwrap();
        assert_eq!(v.list_holdings(acc).unwrap()[0].purchase_price_cents, 55);

        for (isin, wkn, cents) in [
            ("DE0009999999", "", 10), // not in the depot
            ("", "", 10),             // no key
            ("", "A0B1C2", -1),       // negative
        ] {
            assert!(
                v.set_holding_purchase_price(acc, isin.into(), wkn.into(), cents)
                    .is_err(),
                "{isin}/{wkn}/{cents}"
            );
        }
        assert!(
            v.set_holding_purchase_price(acc + 99, String::new(), "A0B1C2".into(), 10)
                .is_err(),
            "another account"
        );
    }
}
