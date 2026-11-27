// synthetic data
use super::*;
use std::sync::Arc;

fn temp_vault() -> (Arc<Vault>, std::path::PathBuf) {
    let p = crate::test_support::thread_temp_path("crypto.db");
    let v = Vault::open(p.to_string_lossy().to_string(), vec![7u8; 32]).unwrap();
    (v, p)
}

/// A fetched price (the Coinbase request comes with the next step of #1836).
fn price(v: &Vault, symbol: &str, eur: f64) {
    v.db()
        .unwrap()
        .execute(
            "INSERT OR REPLACE INTO crypto_prices(symbol, price_eur, fetched_ms) VALUES (?1, ?2, ?3)",
            params![symbol, eur, now_ms()],
        )
        .unwrap();
}

fn balance(v: &Vault, acc: i64) -> i64 {
    v.list_accounts()
        .unwrap()
        .into_iter()
        .find(|a| a.id == acc)
        .unwrap()
        .balance_cents
}

/// Edits get their own millisecond, so "newer" is well defined in the merge test.
fn tick() {
    std::thread::sleep(std::time::Duration::from_millis(3));
}

#[test]
fn positions_show_as_depot_holdings_with_balance_and_history() {
    let (v, _p) = temp_vault();
    let acc = v.create_crypto_account(" Wallet ".into()).unwrap();
    assert_eq!(v.list_crypto_account_ids().unwrap(), vec![acc]);
    let a = v
        .list_accounts()
        .unwrap()
        .into_iter()
        .find(|a| a.id == acc)
        .unwrap();
    assert_eq!(
        (a.name.as_str(), a.kind),
        ("Wallet", AccountKind::Brokerage)
    );
    assert!(v.list_holdings(acc).unwrap().is_empty());

    price(&v, "BTC", 80_000.0);
    price(&v, "SHIB", 0.000_012_34);
    v.save_crypto_position(acc, None, "btc".into(), 0.5, Some(30_000_00))
        .unwrap();
    v.save_crypto_position(acc, None, " shib".into(), 10_000_000.0, None)
        .unwrap();

    let p = v.list_crypto_positions(acc).unwrap();
    assert_eq!(p.len(), 2);
    assert_eq!((p[0].symbol.as_str(), p[0].value_cents), ("BTC", 40_000_00));
    // a tiny price keeps its precision: 10 M × 0.00001234 € = 123.40 €
    assert_eq!((p[1].symbol.as_str(), p[1].value_cents), ("SHIB", 123_40));

    let h = v.list_holdings(acc).unwrap();
    assert_eq!(h.len(), 2);
    assert_eq!((h[0].wkn.as_str(), h[0].isin.as_str()), ("BTC", ""));
    assert_eq!(h[0].purchase_price_cents, 30_000_00);
    assert_eq!(h[0].current_price_cents, 80_000_00);
    assert_eq!(balance(&v, acc), 40_123_40);

    let history = v.depot_value_history(vec![acc], 30).unwrap();
    assert_eq!(history.last().unwrap().value_cents, 40_123_40);
}

#[test]
fn a_position_without_price_is_kept_with_value_zero() {
    let (v, _p) = temp_vault();
    let acc = v.create_crypto_account(String::new()).unwrap();
    v.save_crypto_position(acc, None, "ETH".into(), 2.0, None)
        .unwrap();
    let p = &v.list_crypto_positions(acc).unwrap()[0];
    assert_eq!((p.price_eur, p.value_cents), (None, 0));
    assert_eq!(v.list_holdings(acc).unwrap().len(), 1);
    // no total in the history while a price is missing
    assert!(
        v.depot_value_history(vec![acc], 30)
            .unwrap()
            .iter()
            .all(|d| d.covered_depots == 0)
    );

    // a change values it once the price is known
    price(&v, "ETH", 3_000.0);
    v.save_crypto_position(acc, Some("eth".into()), "ETH".into(), 3.0, None)
        .unwrap();
    assert_eq!(balance(&v, acc), 9_000_00);

    v.delete_crypto_position(acc, "ETH".into()).unwrap();
    assert!(v.list_crypto_positions(acc).unwrap().is_empty());
    assert!(v.list_holdings(acc).unwrap().is_empty());
    assert_eq!(balance(&v, acc), 0);
}

#[test]
fn invalid_input_is_rejected() {
    let (v, _p) = temp_vault();
    let acc = v.create_crypto_account("Wallet".into()).unwrap();
    v.save_crypto_position(acc, None, "BTC".into(), 1.0, None)
        .unwrap();
    for (symbol, quantity, purchase) in [
        ("", 1.0, None),         // no ticker
        ("BTC-EUR", 1.0, None),  // not a plain ticker
        ("ETH", 0.0, None),      // no quantity
        ("ETH", f64::NAN, None), // not a number
        ("ETH", 1.0, Some(-1)),  // negative purchase price
        ("btc", 2.0, None),      // already held
    ] {
        assert!(
            v.save_crypto_position(acc, None, symbol.into(), quantity, purchase)
                .is_err(),
            "{symbol}/{quantity}/{purchase:?}"
        );
    }
    assert!(v.create_crypto_account("x".repeat(101)).is_err());
    // only crypto accounts take positions
    let tangible = v.create_tangible_account("Gold".into()).unwrap();
    assert!(
        v.save_crypto_position(tangible, None, "ETH".into(), 1.0, None)
            .is_err()
    );
    assert!(v.list_crypto_positions(tangible).is_err());
    assert!(v.delete_crypto_position(acc, "DOGE".into()).is_err());
    assert!(v.delete_crypto_position(tangible, "BTC".into()).is_err());
    assert!(
        v.save_crypto_position(acc, Some("DOGE".into()), "DOGE".into(), 1.0, None)
            .is_err(),
        "nothing to change"
    );
}

#[test]
fn vault_merge_brings_positions_and_keeps_deletions() {
    let (a, pa) = temp_vault();
    let acc = a.create_crypto_account("Wallet".into()).unwrap();
    price(&a, "BTC", 80_000.0);
    a.save_crypto_position(acc, None, "BTC".into(), 1.0, None)
        .unwrap();
    a.save_crypto_position(acc, None, "ETH".into(), 2.0, None)
        .unwrap();

    let (b, pb) = temp_vault();
    b.merge_other_vault(pa.to_string_lossy().to_string(), vec![7u8; 32])
        .unwrap();
    let bacc = b.list_crypto_account_ids().unwrap()[0];
    assert_eq!(b.list_crypto_positions(bacc).unwrap().len(), 2);
    assert_eq!(b.list_holdings(bacc).unwrap().len(), 2);
    assert_eq!(balance(&b, bacc), 80_000_00, "the price came along");

    // A deletes ETH, B changes BTC later: both survive a merge each way
    a.delete_crypto_position(acc, "ETH".into()).unwrap();
    tick();
    b.save_crypto_position(bacc, Some("BTC".into()), "BTC".into(), 1.5, None)
        .unwrap();
    b.merge_other_vault(pa.to_string_lossy().to_string(), vec![7u8; 32])
        .unwrap();
    a.merge_other_vault(pb.to_string_lossy().to_string(), vec![7u8; 32])
        .unwrap();
    for (v, acc) in [(&a, acc), (&b, bacc)] {
        let p = v.list_crypto_positions(acc).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!((p[0].symbol.as_str(), p[0].quantity), ("BTC", 1.5));
        assert_eq!(balance(v, acc), 120_000_00);
    }
}

/// The latest value the depot history holds for position `symbol`.
fn last_observed(v: &Vault, acc: i64, symbol: &str) -> i64 {
    v.db()
        .unwrap()
        .query_row(
            "SELECT value_cents FROM depot_observations WHERE account_id = ?1 AND kind = 'position'
               AND name = ?2 ORDER BY observed_ms DESC LIMIT 1",
            params![acc, symbol],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn a_removed_position_is_closed_in_the_history_while_another_lacks_a_price() {
    let (v, _p) = temp_vault();
    let acc = v.create_crypto_account("Wallet".into()).unwrap();
    price(&v, "BTC", 80_000.0);
    v.save_crypto_position(acc, None, "BTC".into(), 1.0, None)
        .unwrap();
    v.save_crypto_position(acc, None, "ETH".into(), 2.0, None)
        .unwrap();
    assert_eq!(last_observed(&v, acc, "BTC"), 80_000_00);
    v.delete_crypto_position(acc, "BTC".into()).unwrap();
    assert_eq!(last_observed(&v, acc, "BTC"), 0, "closing zero");
}

#[test]
fn the_same_ticker_added_on_two_devices_is_one_position_after_a_merge() {
    let (a, pa) = temp_vault();
    let acc = a.create_crypto_account("Wallet".into()).unwrap();
    let (b, pb) = temp_vault();
    b.merge_other_vault(pa.to_string_lossy().to_string(), vec![7u8; 32])
        .unwrap();
    let bacc = b.list_crypto_account_ids().unwrap()[0];
    price(&a, "BTC", 80_000.0);
    price(&b, "BTC", 80_000.0);
    a.save_crypto_position(acc, None, "BTC".into(), 1.0, None)
        .unwrap();
    tick();
    b.save_crypto_position(bacc, None, "BTC".into(), 2.0, None)
        .unwrap();
    for _ in 0..2 {
        a.merge_other_vault(pb.to_string_lossy().to_string(), vec![7u8; 32])
            .unwrap();
        b.merge_other_vault(pa.to_string_lossy().to_string(), vec![7u8; 32])
            .unwrap();
    }
    for (v, acc) in [(&a, acc), (&b, bacc)] {
        let p = v.list_crypto_positions(acc).unwrap();
        assert_eq!(p.len(), 1, "one BTC");
        assert_eq!(p[0].quantity, 2.0, "the newer one");
        assert_eq!(balance(v, acc), 160_000_00);
    }
}

#[test]
fn on_equal_stamps_a_deletion_wins_the_merge() {
    let (a, pa) = temp_vault();
    let acc = a.create_crypto_account("Wallet".into()).unwrap();
    a.save_crypto_position(acc, None, "BTC".into(), 1.0, None)
        .unwrap();
    let (b, _pb) = temp_vault();
    b.merge_other_vault(pa.to_string_lossy().to_string(), vec![7u8; 32])
        .unwrap();
    let bacc = b.list_crypto_account_ids().unwrap()[0];
    let stamp: i64 = a
        .db()
        .unwrap()
        .query_row("SELECT changed_ms FROM crypto_positions", [], |r| r.get(0))
        .unwrap();
    a.db()
        .unwrap()
        .execute(
            "UPDATE crypto_positions SET deleted_ms = ?1, changed_ms = ?1",
            [stamp],
        )
        .unwrap();
    b.merge_other_vault(pa.to_string_lossy().to_string(), vec![7u8; 32])
        .unwrap();
    assert!(b.list_crypto_positions(bacc).unwrap().is_empty());
}
