// synthetic data
use super::*;
use std::sync::Arc;

fn temp_vault() -> (Arc<Vault>, std::path::PathBuf) {
    let p = crate::test_support::thread_temp_path("crypto.db");
    let v = Vault::open(p.to_string_lossy().to_string(), vec![7u8; 32]).unwrap();
    (v, p)
}

/// A fetched price, written directly (no network in tests).
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

/// The next edit gets a later stamp: waits until the clock has moved past the
/// current millisecond (not a fixed sleep, so a stalled clock cannot tie them).
fn tick() {
    let now = now_ms();
    while now_ms() <= now {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
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

#[test]
fn a_value_beyond_cents_is_refused() {
    let (v, _p) = temp_vault();
    let acc = v.create_crypto_account("Wallet".into()).unwrap();
    price(&v, "BTC", 60_000.0);
    // above 10 trillion EUR for one position
    assert!(
        v.save_crypto_position(acc, None, "BTC".into(), 2e8, None)
            .is_err()
    );
    v.save_crypto_position(acc, None, "BTC".into(), 1e8, None)
        .unwrap();
    // above any holding even without a price
    assert!(
        v.save_crypto_position(acc, None, "DOGE".into(), 1e13, None)
            .is_err()
    );
    assert_eq!(v.list_crypto_positions(acc).unwrap().len(), 1);
}

#[test]
fn a_merge_without_crypto_changes_records_no_valuation() {
    let (a, pa) = temp_vault();
    let acc = a.create_crypto_account("Wallet".into()).unwrap();
    price(&a, "BTC", 80_000.0);
    a.save_crypto_position(acc, None, "BTC".into(), 1.0, None)
        .unwrap();
    let (b, pb) = temp_vault();
    b.merge_other_vault(pa.to_string_lossy().to_string(), vec![7u8; 32])
        .unwrap();
    let count = || -> i64 {
        a.db()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM depot_observations WHERE account_id = ?1",
                [acc],
                |r| r.get(0),
            )
            .unwrap()
    };
    a.merge_other_vault(pb.to_string_lossy().to_string(), vec![7u8; 32])
        .unwrap();
    // without any history a write of the merge would show, even one with today's values
    for v in [&a, &b] {
        v.db()
            .unwrap()
            .execute_batch("DELETE FROM depot_observations; DELETE FROM depot_values;")
            .unwrap();
    }
    tick();
    a.merge_other_vault(pb.to_string_lossy().to_string(), vec![7u8; 32])
        .unwrap();
    assert_eq!(count(), 0);
}

#[test]
fn equal_price_stamps_end_with_the_same_price_on_both_devices() {
    let (a, pa) = temp_vault();
    let (b, pb) = temp_vault();
    for (v, eur) in [(&a, 100.0), (&b, 101.0)] {
        v.db()
            .unwrap()
            .execute(
                "INSERT INTO crypto_prices(symbol, price_eur, fetched_ms) VALUES ('BTC', ?1, 5)",
                [eur],
            )
            .unwrap();
    }
    let stored = |v: &Vault| -> f64 {
        v.db()
            .unwrap()
            .query_row("SELECT price_eur FROM crypto_prices", [], |r| r.get(0))
            .unwrap()
    };
    // the lower incoming price is refused, the higher one taken
    b.merge_other_vault(pa.to_string_lossy().to_string(), vec![7u8; 32])
        .unwrap();
    assert_eq!(stored(&b), 101.0);
    a.merge_other_vault(pb.to_string_lossy().to_string(), vec![7u8; 32])
        .unwrap();
    assert_eq!(stored(&a), 101.0);
}

#[test]
fn duplicate_cleanup_ends_the_same_in_every_merge_order() {
    // A adds BTC (x); C copies it; B adds BTC (y); C edits x last
    let (a, pa) = temp_vault();
    let acc = a.create_crypto_account("Wallet".into()).unwrap();
    let (b, pb) = temp_vault();
    b.merge_other_vault(pa.to_string_lossy().to_string(), vec![7u8; 32])
        .unwrap();
    a.save_crypto_position(acc, None, "BTC".into(), 1.0, None)
        .unwrap();
    let (c, pc) = temp_vault();
    c.merge_other_vault(pa.to_string_lossy().to_string(), vec![7u8; 32])
        .unwrap();
    let (bacc, cacc) = (
        b.list_crypto_account_ids().unwrap()[0],
        c.list_crypto_account_ids().unwrap()[0],
    );
    tick();
    b.save_crypto_position(bacc, None, "BTC".into(), 2.0, None)
        .unwrap();
    tick();
    c.save_crypto_position(cacc, Some("BTC".into()), "BTC".into(), 3.0, None)
        .unwrap();

    let merge = |v: &Vault, from: &std::path::PathBuf| {
        v.merge_other_vault(from.to_string_lossy().to_string(), vec![7u8; 32])
            .unwrap()
    };
    merge(&a, &pb);
    merge(&c, &pb);
    merge(&a, &pc);
    merge(&b, &pa);
    merge(&c, &pa);
    for (v, acc) in [(&a, acc), (&b, bacc), (&c, cacc)] {
        let p = v.list_crypto_positions(acc).unwrap();
        assert_eq!(p.len(), 1, "one live BTC");
        assert_eq!(p[0].quantity, 3.0, "the newest edit");
    }
}

#[test]
fn a_price_beyond_cents_from_another_vault_is_not_taken() {
    let (a, pa) = temp_vault();
    a.db()
        .unwrap()
        .execute(
            "INSERT INTO crypto_prices(symbol, price_eur, fetched_ms) VALUES ('BTC', 1e17, 5), ('ETH', 1e14, 5)",
            [],
        )
        .unwrap();
    let (b, _pb) = temp_vault();
    b.merge_other_vault(pa.to_string_lossy().to_string(), vec![7u8; 32])
        .unwrap();
    let n: i64 = b
        .db()
        .unwrap()
        .query_row("SELECT count(*) FROM crypto_prices", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
    // one stored anyway counts as no price: the position is kept, unvalued
    let acc = a.create_crypto_account("Wallet".into()).unwrap();
    a.save_crypto_position(acc, None, "BTC".into(), 0.001, None)
        .unwrap();
    let p = a.list_crypto_positions(acc).unwrap();
    assert_eq!((p[0].price_eur, p[0].value_cents), (None, 0));
    assert_eq!(a.list_holdings(acc).unwrap()[0].current_price_cents, 0);
}

#[test]
fn a_renamed_twin_survives_in_every_merge_order() {
    // A: x (BTC); B: y (BTC), added separately; C: y renamed to ETH
    let (a, pa) = temp_vault();
    let acc = a.create_crypto_account("Wallet".into()).unwrap();
    let (b, pb) = temp_vault();
    b.merge_other_vault(pa.to_string_lossy().to_string(), vec![7u8; 32])
        .unwrap();
    let bacc = b.list_crypto_account_ids().unwrap()[0];
    a.save_crypto_position(acc, None, "BTC".into(), 1.0, None)
        .unwrap();
    tick();
    b.save_crypto_position(bacc, None, "BTC".into(), 2.0, None)
        .unwrap();
    let (c, pc) = temp_vault();
    c.merge_other_vault(pb.to_string_lossy().to_string(), vec![7u8; 32])
        .unwrap();
    let cacc = c.list_crypto_account_ids().unwrap()[0];
    tick();
    c.save_crypto_position(cacc, Some("BTC".into()), "ETH".into(), 2.0, None)
        .unwrap();
    // two copies of A merge B and C in opposite orders
    let (a2, _pa2) = temp_vault();
    a2.merge_other_vault(pa.to_string_lossy().to_string(), vec![7u8; 32])
        .unwrap();
    let a2acc = a2.list_crypto_account_ids().unwrap()[0];
    for from in [&pb, &pc] {
        a.merge_other_vault(from.to_string_lossy().to_string(), vec![7u8; 32])
            .unwrap();
    }
    for from in [&pc, &pb] {
        a2.merge_other_vault(from.to_string_lossy().to_string(), vec![7u8; 32])
            .unwrap();
    }
    let held = |v: &Vault, acc| -> Vec<(String, f64)> {
        v.list_crypto_positions(acc)
            .unwrap()
            .into_iter()
            .map(|p| (p.symbol, p.quantity))
            .collect()
    };
    let expected = vec![("BTC".to_string(), 1.0), ("ETH".to_string(), 2.0)];
    assert_eq!(held(&a, acc), expected);
    assert_eq!(held(&a2, a2acc), expected);
}

#[test]
fn editing_a_merged_twin_retires_the_other() {
    let (a, pa) = temp_vault();
    let acc = a.create_crypto_account("Wallet".into()).unwrap();
    let (b, pb) = temp_vault();
    b.merge_other_vault(pa.to_string_lossy().to_string(), vec![7u8; 32])
        .unwrap();
    let bacc = b.list_crypto_account_ids().unwrap()[0];
    b.save_crypto_position(bacc, None, "BTC".into(), 2.0, None)
        .unwrap();
    tick();
    a.save_crypto_position(acc, None, "BTC".into(), 1.0, None)
        .unwrap();
    a.merge_other_vault(pb.to_string_lossy().to_string(), vec![7u8; 32])
        .unwrap();
    // the newest shows; editing it works and leaves one row
    assert_eq!(a.list_crypto_positions(acc).unwrap()[0].quantity, 1.0);
    a.save_crypto_position(acc, Some("BTC".into()), "BTC".into(), 5.0, None)
        .unwrap();
    let live: i64 = a
        .db()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM crypto_positions WHERE deleted_ms IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(live, 1);
    a.delete_crypto_position(acc, "BTC".into()).unwrap();
    assert!(a.list_crypto_positions(acc).unwrap().is_empty());
}

#[test]
fn a_partial_balance_is_no_value_in_the_history() {
    let (v, _p) = temp_vault();
    let acc = v.create_crypto_account("Wallet".into()).unwrap();
    v.save_crypto_position(acc, None, "ETH".into(), 1.0, None)
        .unwrap();
    price(&v, "BTC", 80_000.0);
    v.save_crypto_position(acc, None, "BTC".into(), 1.0, None)
        .unwrap();
    assert_eq!(balance(&v, acc), 80_000_00, "shown: the priced part");
    assert!(
        v.depot_value_history(vec![acc], 30)
            .unwrap()
            .iter()
            .all(|d| d.covered_depots == 0),
        "ETH has no price: no depot value"
    );
}

#[test]
fn a_newer_deletion_hides_older_copies_in_both_merge_directions() {
    let (a, pa) = temp_vault();
    let acc = a.create_crypto_account("Wallet".into()).unwrap();
    let (b, pb) = temp_vault();
    b.merge_other_vault(pa.to_string_lossy().to_string(), vec![7u8; 32])
        .unwrap();
    let bacc = b.list_crypto_account_ids().unwrap()[0];
    // A: x (BTC); B: y (BTC, newer); A sees both and shows y
    a.save_crypto_position(acc, None, "BTC".into(), 1.0, None)
        .unwrap();
    tick();
    b.save_crypto_position(bacc, None, "BTC".into(), 2.0, None)
        .unwrap();
    a.merge_other_vault(pb.to_string_lossy().to_string(), vec![7u8; 32])
        .unwrap();
    assert_eq!(a.list_crypto_positions(acc).unwrap()[0].quantity, 2.0);
    // B deletes BTC without ever having seen x
    tick();
    b.delete_crypto_position(bacc, "BTC".into()).unwrap();
    a.merge_other_vault(pb.to_string_lossy().to_string(), vec![7u8; 32])
        .unwrap();
    b.merge_other_vault(pa.to_string_lossy().to_string(), vec![7u8; 32])
        .unwrap();
    assert!(a.list_crypto_positions(acc).unwrap().is_empty());
    assert!(b.list_crypto_positions(bacc).unwrap().is_empty());
    // the ticker can be added again
    b.save_crypto_position(bacc, None, "BTC".into(), 3.0, None)
        .unwrap();
    assert_eq!(b.list_crypto_positions(bacc).unwrap()[0].quantity, 3.0);
}

#[test]
fn an_unpriced_ticker_ends_the_carried_total() {
    let (v, _p) = temp_vault();
    let acc = v.create_crypto_account("Wallet".into()).unwrap();
    price(&v, "BTC", 80_000.0);
    v.save_crypto_position(acc, None, "BTC".into(), 1.0, None)
        .unwrap();
    // that total was recorded yesterday
    let yesterday = crate::dates::iso(crate::dates::today() - crate::dates::days(1));
    v.db()
        .unwrap()
        .execute("UPDATE depot_values SET value_date = ?1", [&yesterday])
        .unwrap();
    v.save_crypto_position(acc, None, "ETH".into(), 1.0, None)
        .unwrap();
    let h = v.depot_value_history(vec![acc], 30).unwrap();
    let day = |d: &str| h.iter().find(|p| p.date == d).unwrap().covered_depots;
    assert_eq!(day(&yesterday), 1);
    assert_eq!(
        day(&today_iso()),
        0,
        "ETH has no price: today's value is unknown"
    );
}

#[test]
fn a_huge_position_meeting_a_price_leaves_the_account_usable() {
    let (a, pa) = temp_vault();
    let acc = a.create_crypto_account("Wallet".into()).unwrap();
    // no price yet: the quantity passes (within the cap)
    a.save_crypto_position(acc, None, "BTC".into(), 1e12, None)
        .unwrap();
    let (b, _pb) = temp_vault();
    price(&b, "BTC", 1e10);
    b.merge_other_vault(pa.to_string_lossy().to_string(), vec![7u8; 32])
        .unwrap();
    let bacc = b.list_crypto_account_ids().unwrap()[0];
    let p = b.list_crypto_positions(bacc).unwrap();
    assert_eq!(
        (p[0].price_eur, p[0].value_cents),
        (None, 0),
        "counts as unpriced"
    );
    assert_eq!(b.list_holdings(bacc).unwrap().len(), 1);
    // and it can still be corrected
    b.save_crypto_position(bacc, Some("BTC".into()), "BTC".into(), 1.0, None)
        .unwrap();
    assert_eq!(balance(&b, bacc), 1_000_000_000_000);
}

#[test]
fn an_edit_against_a_newer_deletion_ends_the_same_on_both_devices() {
    // A holds x and y (BTC, merged); C renames y to ETH; A, not knowing that, deletes BTC
    // later. Two edits of one position conflict: the newer one (the deletion) wins on both.
    let (a, pa) = temp_vault();
    let acc = a.create_crypto_account("Wallet".into()).unwrap();
    let (b, pb) = temp_vault();
    b.merge_other_vault(pa.to_string_lossy().to_string(), vec![7u8; 32])
        .unwrap();
    let bacc = b.list_crypto_account_ids().unwrap()[0];
    a.save_crypto_position(acc, None, "BTC".into(), 1.0, None)
        .unwrap();
    tick();
    b.save_crypto_position(bacc, None, "BTC".into(), 2.0, None)
        .unwrap();
    a.merge_other_vault(pb.to_string_lossy().to_string(), vec![7u8; 32])
        .unwrap();
    let (c, pc) = temp_vault();
    c.merge_other_vault(pa.to_string_lossy().to_string(), vec![7u8; 32])
        .unwrap();
    let cacc = c.list_crypto_account_ids().unwrap()[0];
    tick();
    c.save_crypto_position(cacc, Some("BTC".into()), "ETH".into(), 2.0, None)
        .unwrap();
    tick();
    a.delete_crypto_position(acc, "BTC".into()).unwrap();
    a.merge_other_vault(pc.to_string_lossy().to_string(), vec![7u8; 32])
        .unwrap();
    c.merge_other_vault(pa.to_string_lossy().to_string(), vec![7u8; 32])
        .unwrap();
    for (v, id) in [(&a, acc), (&c, cacc)] {
        let p: Vec<String> = v
            .list_crypto_positions(id)
            .unwrap()
            .into_iter()
            .map(|p| p.symbol)
            .collect();
        assert!(p.is_empty(), "{p:?}");
    }
}

#[test]
fn a_held_position_without_a_value_is_not_closed_and_zero_cost_means_unknown() {
    let (v, _p) = temp_vault();
    let acc = v.create_crypto_account("Wallet".into()).unwrap();
    price(&v, "BTC", 80_000.0);
    price(&v, "ETH", 3_000.0);
    v.save_crypto_position(acc, None, "BTC".into(), 1.0, Some(0))
        .unwrap();
    assert_eq!(
        v.list_crypto_positions(acc).unwrap()[0].purchase_price_cents,
        None
    );
    // BTC's price no longer gives a value in cents; then ETH is edited
    v.db()
        .unwrap()
        .execute(
            "UPDATE crypto_prices SET price_eur = 1e14 WHERE symbol = 'BTC'",
            [],
        )
        .unwrap();
    v.save_crypto_position(acc, None, "ETH".into(), 1.0, None)
        .unwrap();
    assert_eq!(
        last_observed(&v, acc, "BTC"),
        80_000_00,
        "no closing 0: still held"
    );
}

#[test]
fn a_partial_balance_is_not_carried_back_in_the_balance_history() {
    let (v, _p) = temp_vault();
    let acc = v.create_crypto_account("Wallet".into()).unwrap();
    v.save_crypto_position(acc, None, "ETH".into(), 1.0, None)
        .unwrap();
    price(&v, "BTC", 80_000.0);
    v.save_crypto_position(acc, None, "BTC".into(), 1.0, None)
        .unwrap();
    assert_eq!(balance(&v, acc), 80_000_00);
    let h = v.balance_history(vec![acc], 30).unwrap();
    assert!(
        h.iter().all(|d| d.balance_cents == 0),
        "no value of earlier days from a partial balance"
    );
}

#[test]
fn an_unpriced_ticker_keeps_the_last_total_in_the_balance_history_without_coverage() {
    let (v, _p) = temp_vault();
    let acc = v.create_crypto_account("Wallet".into()).unwrap();
    price(&v, "BTC", 80_000.0);
    v.save_crypto_position(acc, None, "BTC".into(), 1.0, None)
        .unwrap();
    let yesterday = crate::dates::iso(crate::dates::today() - crate::dates::days(1));
    v.db()
        .unwrap()
        .execute("UPDATE depot_values SET value_date = ?1", [&yesterday])
        .unwrap();
    v.save_crypto_position(acc, None, "ETH".into(), 1.0, None)
        .unwrap();
    // the balance chart does not drop to 0 …
    let b = v.balance_history(vec![acc], 30).unwrap();
    assert_eq!(b.last().unwrap().balance_cents, 80_000_00);
    // … while the depot history does not count today as valued
    let d = v.depot_value_history(vec![acc], 30).unwrap();
    assert_eq!(
        d.iter()
            .find(|p| p.date == today_iso())
            .unwrap()
            .covered_depots,
        0
    );
}

#[test]
fn cash_words_are_no_tickers_and_an_unchanged_quantity_may_stay() {
    let (v, _p) = temp_vault();
    let acc = v.create_crypto_account("Wallet".into()).unwrap();
    for t in ["CASH", "liquidity", "Liquiditaet"] {
        assert!(
            v.save_crypto_position(acc, None, t.into(), 1.0, None)
                .is_err(),
            "{t}"
        );
    }
    // a newer price makes the position unvaluable: its purchase price can still be edited
    v.save_crypto_position(acc, None, "BTC".into(), 1e9, None)
        .unwrap();
    price(&v, "BTC", 1e7);
    v.save_crypto_position(acc, Some("BTC".into()), "BTC".into(), 1e9, Some(100))
        .unwrap();
    assert!(
        v.save_crypto_position(acc, Some("BTC".into()), "BTC".into(), 2e9, None)
            .is_err(),
        "a changed quantity must give a value"
    );
}

#[test]
fn a_merged_price_updates_the_balance_but_records_no_observation() {
    let (a, pa) = temp_vault();
    let acc = a.create_crypto_account("Wallet".into()).unwrap();
    price(&a, "BTC", 80_000.0);
    a.save_crypto_position(acc, None, "BTC".into(), 1.0, None)
        .unwrap();
    let (b, pb) = temp_vault();
    b.merge_other_vault(pa.to_string_lossy().to_string(), vec![7u8; 32])
        .unwrap();
    a.merge_other_vault(pb.to_string_lossy().to_string(), vec![7u8; 32])
        .unwrap();
    let count = || -> i64 {
        a.db()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM depot_observations WHERE account_id = ?1",
                [acc],
                |r| r.get(0),
            )
            .unwrap()
    };
    let before = count();
    // B received a newer price (written directly: no observation on B either)
    b.db()
        .unwrap()
        .execute(
            "UPDATE crypto_prices SET price_eur = 90000, fetched_ms = fetched_ms + 1000",
            [],
        )
        .unwrap();
    a.merge_other_vault(pb.to_string_lossy().to_string(), vec![7u8; 32])
        .unwrap();
    assert_eq!(balance(&a, acc), 90_000_00);
    assert_eq!(count(), before);
}
