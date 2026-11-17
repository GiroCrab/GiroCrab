use super::*;
use crate::connectors::{store_fetch, FetchResult, RemoteAccount, RemoteHolding};
use crate::models::{AccountKind, ConnectorKind};
use crate::vault::Vault;

fn open(dir: &crate::test_support::TempDir) -> std::sync::Arc<Vault> {
    Vault::open(
        dir.join("v.db").to_string_lossy().to_string(),
        vec![5u8; 32],
    )
    .unwrap()
}

fn holding(isin: &str, name: &str, qty: f64, price: i64) -> Holding {
    Holding {
        isin: isin.into(),
        name: name.into(),
        quantity: qty,
        current_price_cents: price,
        value_cents: (qty * price as f64).round() as i64,
        ..Default::default()
    }
}

fn depot(v: &Vault) -> i64 {
    v.list_accounts()
        .unwrap()
        .into_iter()
        .find(|a| a.kind == AccountKind::Brokerage)
        .unwrap()
        .id
}

fn count(v: &Vault) -> i64 {
    v.db()
        .unwrap()
        .query_row("SELECT count(*) FROM depot_observations", [], |r| r.get(0))
        .unwrap()
}

/// The derived values of the depot, oldest first.
fn derived(v: &Vault, id: i64) -> Vec<(String, String, i64)> {
    v.db()
        .unwrap()
        .prepare(
            "SELECT value_date, source, value_cents FROM depot_values WHERE account_id = ?1
             ORDER BY value_date, source",
        )
        .unwrap()
        .query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn fetch(holdings: Vec<Holding>, balance: i64) -> FetchResult {
    FetchResult {
        accounts: vec![RemoteAccount {
            external_ref: "depot".into(),
            name: "depot".into(),
            owner: String::new(),
            iban: String::new(),
            account_number: "depot".into(),
            currency: "EUR".into(),
            kind: AccountKind::Brokerage,
            balance_cents: balance,
            balance_date: "2026-11-10".into(),
        }],
        holdings: holdings
            .into_iter()
            .map(|h| RemoteHolding {
                account_ref: "depot".into(),
                holding: h,
                ..Default::default()
            })
            .collect(),
        holdings_fetched: vec!["depot".into()],
        ..Default::default()
    }
}

fn connected(v: &Vault) -> i64 {
    v.add_bank_connection(ConnectorKind::Comdirect, String::new(), "l".into())
        .unwrap()
}

fn imported(position: &str, name: &str, currency: &str, day: &str, cents: i64) -> ObservedValue {
    ObservedValue {
        kind: if position.is_empty() {
            KIND_TOTAL
        } else {
            KIND_POSITION
        },
        position: position.into(),
        source_ref: String::new(),
        name: name.into(),
        currency: currency.into(),
        observed_ms: chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis(),
        value_date: day.into(),
        value_cents: cents,
    }
}

fn obs(date: &str, source: &str, ms: i64, cents: i64, confirmed: bool) -> Observed {
    Observed {
        day: day_key(date),
        source: source_of(source),
        observed_ms: ms,
        value_cents: cents,
        confirmed,
    }
}

#[test]
fn day_keys_order_like_the_text_and_round_trip() {
    assert_eq!(day_key("2026-11-07"), 20_261_107);
    assert_eq!(day_text(20_261_107), "2026-11-07");
    assert!(day_key("2026-11-07") < day_key("2026-12-01"));
    assert_eq!(day_key("07.11.2026"), 0);
    assert_eq!(day_key("2026-1x-07"), 0);
}

#[test]
fn position_keys() {
    assert_eq!(position_key(" ie00b4l5y983 ", "x").unwrap(), "IE00B4L5Y983");
    assert_eq!(position_key("", "Liquidität").unwrap(), "cash");
    assert_eq!(position_key("", "  Foo   Bar ").unwrap(), "name:foo bar");
    assert!(position_key(" ", " ").is_none());
}

#[test]
fn the_valuation_day_is_the_banks_unless_it_is_missing_or_ahead() {
    let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 15).unwrap();
    assert_eq!(
        valuation_day("2026-11-13", today),
        ("2026-11-13".into(), false)
    );
    assert_eq!(valuation_day("", today), ("2026-11-15".into(), true));
    assert_eq!(
        valuation_day("13.11.2026", today),
        ("2026-11-15".into(), true)
    );
    assert_eq!(
        valuation_day("2026-11-16", today),
        ("2026-11-15".into(), true)
    );
}

#[test]
fn a_fetch_stores_the_total_and_positions_and_skips_an_unchanged_retrieval() {
    let dir = crate::test_support::TempDir::new("dobs-fetch");
    let v = open(&dir);
    let c = connected(&v);
    let data = || fetch(vec![holding("IE00B4L5Y983", "ETF", 2.0, 5_000)], 10_000);
    store_fetch(&v, ConnectorKind::Comdirect, c, data()).unwrap();
    assert_eq!(count(&v), 2, "total + one position");
    store_fetch(&v, ConnectorKind::Comdirect, c, data()).unwrap();
    assert_eq!(
        count(&v),
        2,
        "an unchanged fetch of the same day writes nothing"
    );
    store_fetch(
        &v,
        ConnectorKind::Comdirect,
        c,
        fetch(vec![holding("IE00B4L5Y983", "ETF", 2.0, 5_100)], 10_200),
    )
    .unwrap();
    assert_eq!(count(&v), 4, "a changed price is a new observation");
    let id = depot(&v);
    assert_eq!(
        derived(&v, id),
        vec![("2026-11-10".into(), "fetch".into(), 10_200)],
        "one value per valuation day, the latest observation"
    );
    let (assumed, confirmed, quantity, price): (bool, bool, f64, i64) = v
        .db()
        .unwrap()
        .query_row(
            "SELECT date_assumed, confirmed, quantity, price_cents FROM depot_observations
             WHERE kind = 'position' ORDER BY observed_ms DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert!(!assumed && confirmed);
    assert_eq!((quantity, price), (2.0, 5_100));
    let p = v.position_value_history(id, "IE00B4L5Y983".into()).unwrap();
    assert_eq!(p[0].date, "2026-11-10");
    assert_eq!(p[0].value_cents, 10_200);
}

#[test]
fn a_new_valuation_day_is_stored_even_if_nothing_changed() {
    let dir = crate::test_support::TempDir::new("dobs-day");
    let v = open(&dir);
    let c = connected(&v);
    let data = || fetch(vec![holding("IE00B4L5Y983", "ETF", 1.0, 100)], 100);
    store_fetch(&v, ConnectorKind::Comdirect, c, data()).unwrap();
    let id = depot(&v);
    v.db()
        .unwrap()
        .execute(
            "UPDATE accounts SET balance_date = '2026-11-11' WHERE id = ?1",
            [id],
        )
        .unwrap();
    let h = holding("IE00B4L5Y983", "ETF", 1.0, 100);
    let db = v.db().unwrap();
    let day = record_fetch(&db, id, Some(&[&h]), 4_000_000_000_000).unwrap();
    assert_eq!(day.as_deref(), Some("2026-11-11"));
    drop(db);
    assert_eq!(count(&v), 4, "new day: total and position again");
    let db = v.db().unwrap();
    assert!(record_fetch(&db, id, Some(&[&h]), 4_000_000_001_000)
        .unwrap()
        .is_none());
}

#[test]
fn a_position_gone_from_a_delivered_inventory_is_closed_with_zero() {
    let dir = crate::test_support::TempDir::new("dobs-sold");
    let v = open(&dir);
    let c = connected(&v);
    store_fetch(
        &v,
        ConnectorKind::Comdirect,
        c,
        fetch(
            vec![
                holding("IE00B4L5Y983", "ETF", 1.0, 100),
                holding("DE0000000001", "Alt AG", 1.0, 50),
            ],
            150,
        ),
    )
    .unwrap();
    store_fetch(
        &v,
        ConnectorKind::Comdirect,
        c,
        fetch(vec![holding("IE00B4L5Y983", "ETF", 1.0, 100)], 100),
    )
    .unwrap();
    let id = depot(&v);
    let sold = v.position_value_history(id, "DE0000000001".into()).unwrap();
    assert_eq!(
        sold.iter().map(|p| p.value_cents).collect::<Vec<_>>(),
        vec![0],
        "the sale on the same valuation day replaces the value; the series ends there"
    );
    let list = v.depot_value_positions(id).unwrap();
    assert_eq!(list.len(), 2);
    let alt = list.iter().find(|p| p.position == "DE0000000001").unwrap();
    assert_eq!((alt.last_value_cents, alt.held), (0, false));
    assert!(
        list.iter()
            .find(|p| p.position == "IE00B4L5Y983")
            .unwrap()
            .held
    );
}

#[test]
fn the_total_is_the_balance_or_else_the_sum_of_the_positions() {
    let dir = crate::test_support::TempDir::new("dobs-total");
    let v = open(&dir);
    let c = connected(&v);
    // no reported value: the depot is worth its positions
    store_fetch(
        &v,
        ConnectorKind::Comdirect,
        c,
        fetch(vec![holding("IE00B4L5Y983", "ETF", 2.0, 5_000)], 0),
    )
    .unwrap();
    let id = depot(&v);
    assert_eq!(derived(&v, id)[0].2, 10_000);
}

#[test]
fn custody_rows_of_one_isin_keep_quantity_times_price_equal_to_the_value() {
    let dir = crate::test_support::TempDir::new("dobs-custody");
    let v = open(&dir);
    let c = connected(&v);
    store_fetch(
        &v,
        ConnectorKind::Comdirect,
        c,
        fetch(
            vec![
                holding("IE00B4L5Y983", "ETF", 1.0, 100),
                holding("IE00B4L5Y983", "ETF", 3.0, 200),
            ],
            700,
        ),
    )
    .unwrap();
    let (qty, price, value): (f64, i64, i64) = v
        .db()
        .unwrap()
        .query_row(
            "SELECT quantity, price_cents, value_cents FROM depot_observations WHERE kind = 'position'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!((qty, value), (4.0, 700));
    assert_eq!(price, 175);
}

#[test]
fn a_fetch_does_not_close_positions_only_an_import_knows_by_name() {
    let dir = crate::test_support::TempDir::new("dobs-import-keys");
    let v = open(&dir);
    let c = connected(&v);
    let data = || fetch(vec![holding("IE00B4L5Y983", "ETF", 1.0, 100)], 100);
    store_fetch(&v, ConnectorKind::Comdirect, c, data()).unwrap();
    let id = depot(&v);
    {
        let db = v.db().unwrap();
        // imported: the cash balance (never in a fetch's list) and an ISIN no longer held
        record_import(
            &db,
            id,
            &[
                imported("cash", "Liquidität", "EUR", "2020-01-02", 500),
                imported("DE0000000001", "Alt AG", "EUR", "2020-01-02", 500),
            ],
        )
        .unwrap();
    }
    store_fetch(&v, ConnectorKind::Comdirect, c, data()).unwrap();
    let list = v.depot_value_positions(id).unwrap();
    let last = |key: &str| {
        list.iter()
            .find(|p| p.position == key)
            .unwrap()
            .last_value_cents
    };
    assert_eq!(last("cash"), 500, "an import-only name key is not closed");
    assert_eq!(
        last("DE0000000001"),
        0,
        "an ISIN missing from the inventory is closed"
    );
}

#[test]
fn a_position_in_another_currency_is_shown_in_its_own_and_left_out_of_the_total() {
    let dir = crate::test_support::TempDir::new("dobs-currency");
    let v = open(&dir);
    let c = connected(&v);
    let data = || fetch(vec![holding("IE00B4L5Y983", "ETF", 1.0, 100)], 100);
    store_fetch(&v, ConnectorKind::Comdirect, c, data()).unwrap();
    let id = depot(&v);
    {
        let db = v.db().unwrap();
        record_import(
            &db,
            id,
            &[
                imported("US0378331005", "Apple", "USD", "2020-01-02", 100),
                imported("US0378331005", "Apple", "USD", "2020-01-03", 110),
                imported("", "", "USD", "2020-01-03", 110),
            ],
        )
        .unwrap();
        crate::depot_values::rebuild(&db, id, None).unwrap();
    }
    let list = v.depot_value_positions(id).unwrap();
    let apple = list.iter().find(|p| p.position == "US0378331005").unwrap();
    assert_eq!(apple.currency, "USD");
    assert_eq!(
        list.iter()
            .find(|p| p.position == "IE00B4L5Y983")
            .unwrap()
            .currency,
        "EUR"
    );
    let series = v.position_value_history(id, "US0378331005".into()).unwrap();
    assert_eq!(
        series[..2]
            .iter()
            .map(|p| p.value_cents)
            .collect::<Vec<_>>(),
        [100, 110]
    );
    assert!(
        derived(&v, id).iter().all(|d| d.1 == "fetch"),
        "a total in USD stays out of an EUR depot"
    );
}

#[test]
fn imported_duplicates_give_the_largest_value_and_zeros_count_only_after_a_value() {
    let rows = [
        // before the first purchase: no point
        obs("2020-01-01", "import", 1, 0, true),
        // one ISIN in two source rows: the largest non-zero value
        obs("2020-01-02", "import", 2, 600, true),
        obs("2020-01-02", "import", 2, 0, true),
        obs("2020-01-02", "import", 2, 605, true),
        // sold: the first zero is a point, the ones after it are not
        obs("2020-01-03", "import", 3, 0, true),
        obs("2020-01-04", "import", 4, 0, true),
        // bought again
        obs("2020-01-05", "import", 5, 300, true),
    ];
    let got: Vec<(String, i64)> = day_values(&rows)
        .into_iter()
        .map(|d| (day_text(d.day), d.value_cents))
        .collect();
    assert_eq!(
        got,
        vec![
            ("2020-01-02".into(), 605),
            ("2020-01-03".into(), 0),
            ("2020-01-05".into(), 300)
        ]
    );
}

#[test]
fn a_fetch_takes_the_latest_observation_of_a_day_and_an_unconfirmed_zero_is_none() {
    let rows = [
        obs("2026-11-13", "fetch", 1, 100, false),
        obs("2026-11-13", "fetch", 2, 0, false),
        obs("2026-11-14", "fetch", 3, 0, false),
        obs("2026-11-15", "fetch", 4, 0, true),
    ];
    let got: Vec<(String, i64, i64)> = day_values(&rows)
        .into_iter()
        .map(|d| (day_text(d.day), d.value_cents, d.observed_ms))
        .collect();
    assert_eq!(
        got,
        vec![("2026-11-13".into(), 100, 1), ("2026-11-15".into(), 0, 4)]
    );
}

#[test]
fn a_fetch_beats_an_import_of_the_same_day_for_a_position() {
    let dir = crate::test_support::TempDir::new("dobs-rank");
    let v = open(&dir);
    let c = connected(&v);
    store_fetch(
        &v,
        ConnectorKind::Comdirect,
        c,
        fetch(vec![holding("IE00B4L5Y983", "ETF", 1.0, 100)], 100),
    )
    .unwrap();
    let id = depot(&v);
    {
        let db = v.db().unwrap();
        record_import(
            &db,
            id,
            &[
                imported("IE00B4L5Y983", "ETF", "EUR", "2026-11-09", 90),
                imported("IE00B4L5Y983", "ETF", "EUR", "2026-11-10", 95),
            ],
        )
        .unwrap();
        // importing the same rows again adds nothing
        assert_eq!(
            record_import(
                &db,
                id,
                &[imported("IE00B4L5Y983", "ETF", "EUR", "2026-11-10", 95)]
            )
            .unwrap(),
            0
        );
    }
    let p = v.position_value_history(id, "IE00B4L5Y983".into()).unwrap();
    assert_eq!(
        p[..2].iter().map(|p| p.value_cents).collect::<Vec<_>>(),
        [90, 100]
    );
}

#[test]
fn a_merge_unions_observations_and_derives_the_depot_again() {
    let dir = crate::test_support::TempDir::new("dobs-merge");
    let a = open(&dir);
    let c = connected(&a);
    store_fetch(
        &a,
        ConnectorKind::Comdirect,
        c,
        fetch(vec![holding("IE00B4L5Y983", "ETF", 1.0, 100)], 100),
    )
    .unwrap();
    let id = depot(&a);
    let db = a.db().unwrap();
    db.execute_batch("ATTACH DATABASE ':memory:' AS other;")
        .unwrap();
    // the same rows seen from "other", plus an older day only it saw
    db.execute_batch(
        "CREATE TABLE other.depot_observations AS SELECT * FROM main.depot_observations;
         INSERT INTO other.depot_observations(account_id, kind, position, currency, source,
                 value_date, observed_ms, source_ref, date_assumed, value_cents, confirmed, name)
             SELECT account_id, kind, position, currency, source, '2026-11-01', 1, source_ref,
                    date_assumed, 80, confirmed, name
             FROM main.depot_observations WHERE kind = 'total';",
    )
    .unwrap();
    let map: HashMap<i64, i64> = [(id, id)].into();
    crate::depot_values::merge_attached(&db, &map).unwrap();
    let n: i64 = db
        .query_row("SELECT count(*) FROM main.depot_observations", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(n, 3, "identical observations do not double");
    drop(db);
    assert_eq!(
        derived(&a, id),
        vec![
            ("2026-11-01".into(), "fetch".into(), 80),
            ("2026-11-10".into(), "fetch".into(), 100)
        ]
    );
    let db = a.db().unwrap();
    assert!(merge_attached(&db, &HashMap::new()).unwrap().is_empty());
}

#[test]
#[ignore = "timing at the size of a real imported history: cargo test --release -p girocrab-core --lib perf_of -- --ignored --nocapture"]
fn perf_of_a_real_sized_imported_history() {
    let dir = crate::test_support::TempDir::new("dobs-perf");
    let v = open(&dir);
    let c = connected(&v);
    store_fetch(&v, ConnectorKind::Comdirect, c, fetch(vec![], 100)).unwrap();
    let id = depot(&v);
    // 7 years of days, 223 positions of ~500 days each and a total every day: ~114k rows
    let start = chrono::NaiveDate::from_ymd_opt(2019, 1, 14).unwrap();
    let mut rows = Vec::new();
    for d in 0..2_550_i64 {
        let day = (start + chrono::Duration::days(d))
            .format("%Y-%m-%d")
            .to_string();
        rows.push(imported("", "", "EUR", &day, 1_000_000 + d));
    }
    for p in 0..223_i64 {
        let from = (p * 9) % 2_000;
        for d in from..from + 500 {
            let day = (start + chrono::Duration::days(d))
                .format("%Y-%m-%d")
                .to_string();
            let mut r = imported(&format!("DE{p:010}"), "Pos", "EUR", &day, 1_000 + d);
            r.source_ref = p.to_string();
            rows.push(r);
        }
    }
    let n = rows.len();
    let db = v.db().unwrap();
    let t = std::time::Instant::now();
    let tx = db.unchecked_transaction().unwrap();
    record_import(&tx, id, &rows).unwrap();
    tx.commit().unwrap();
    let t_import = t.elapsed();
    let t = std::time::Instant::now();
    let tx = db.unchecked_transaction().unwrap();
    crate::depot_values::rebuild(&tx, id, None).unwrap();
    tx.commit().unwrap();
    let t_rebuild = t.elapsed();
    drop(db);
    let held: Vec<Holding> = (0..50)
        .map(|p| holding(&format!("DE{p:010}"), "Pos", 1.0, 100))
        .collect();
    let t = std::time::Instant::now();
    store_fetch(&v, ConnectorKind::Comdirect, c, fetch(held, 5_000)).unwrap();
    let t_fetch = t.elapsed();
    let t = std::time::Instant::now();
    let list = v.depot_value_positions(id).unwrap();
    let t_list = t.elapsed();
    let t = std::time::Instant::now();
    let h = v.position_value_history(id, "DE0000000100".into()).unwrap();
    let t_one = t.elapsed();
    let t = std::time::Instant::now();
    let all = v.depot_value_history(vec![id], 3650).unwrap();
    let t_depot = t.elapsed();
    println!(
        "rows {n}: import {t_import:?}, rebuild {t_rebuild:?}, fetch {t_fetch:?}, positions ({}) {t_list:?}, one position ({}) {t_one:?}, depot history ({}) {t_depot:?}",
        list.len(), h.len(), all.len()
    );
}
