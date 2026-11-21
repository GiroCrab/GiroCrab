use std::collections::{BTreeMap, HashSet};
use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::{Value, json};

use super::keys::{self, ExtendedPubKey, KeyError, ScriptKind};
use super::*;

// Test vectors of BIP44 / BIP49 / BIP84 for the mnemonic
// "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".
const XPUB: &str = "xpub6BosfCnifzxcFwrSzQiqu2DBVTshkCXacvNsWGYJVVhhawA7d4R5WSWGFNbi8Aw6ZRc1brxMyWMzG3DSSSSoekkudhUd9yLb6qx39T9nMdj";
const YPUB: &str = "ypub6Ww3ibxVfGzLrAH1PNcjyAWenMTbbAosGNB6VvmSEgytSER9azLDWCxoJwW7Ke7icmizBMXrzBx9979FfaHxHcrArf3zbeJJJUZPf663zsP";
const ZPUB: &str = "zpub6rFR7y4Q2AijBEqTUquhVz398htDFrtymD9xYYfG1m4wAcvPhXNfE3EfH1r1ADqtfSdVCToUG868RvUUkgDKf31mGDtKsAYz2oz2AGutZYs";

#[test]
fn bip84_zpub_derives_native_segwit_addresses() {
    let k = ExtendedPubKey::parse(ZPUB).unwrap();
    assert_eq!(k.kinds, vec![ScriptKind::P2wpkh]);
    let a = |chain, i| keys::address(ScriptKind::P2wpkh, &k.public_key(chain, i).unwrap());
    assert_eq!(a(0, 0), "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu");
    assert_eq!(a(0, 1), "bc1qnjg0jd8228aq7egyzacy8cys3knf9xvrerkf9g");
    assert_eq!(a(1, 0), "bc1q8c6fshw2dlwun7ekn9qwf37cu2rn755upcp6el");
}

#[test]
fn bip49_ypub_derives_nested_segwit_addresses() {
    let k = ExtendedPubKey::parse(YPUB).unwrap();
    assert_eq!(k.kinds, vec![ScriptKind::P2shP2wpkh]);
    let pk = k.public_key(0, 0).unwrap();
    assert_eq!(
        keys::address(ScriptKind::P2shP2wpkh, &pk),
        "37VucYSaXLCAsxYyAPfbSi9eh4iEcbShgf"
    );
}

#[test]
fn bip44_xpub_derives_legacy_addresses_and_scans_segwit_too() {
    let k = ExtendedPubKey::parse(XPUB).unwrap();
    assert_eq!(k.kinds, vec![ScriptKind::P2wpkh, ScriptKind::P2pkh]);
    let pk = k.public_key(0, 0).unwrap();
    assert_eq!(
        keys::address(ScriptKind::P2pkh, &pk),
        "1LqBGSKuX5yYUonjxT5qGfpUsXKYYWeabA"
    );
}

#[test]
fn private_testnet_and_broken_keys_are_rejected() {
    // BIP32 test vector 1, master private key
    let xprv = "xprv9s21ZrQH143K3QTDL4LXw2F7HEK3wJUD2nW2nRk4stbPy6cq3jPPqjiChkVvvNKmPGJxWUtg6LnF5kejMRNNU3TGtRBeJgk33yuGBxrMPHi";
    assert_eq!(ExtendedPubKey::parse(xprv), Err(KeyError::Private));
    // BIP32 test vector 1, master public key re-encoded as testnet tpub
    let mut raw = keys::base58check_decode(XPUB).unwrap();
    raw[..4].copy_from_slice(&[0x04, 0x35, 0x87, 0xCF]);
    let tpub = keys::base58check_encode(raw[0], &raw[1..]);
    assert_eq!(ExtendedPubKey::parse(&tpub), Err(KeyError::Unsupported));
    let mut broken = ZPUB.to_string();
    broken.replace_range(20..21, if &broken[20..21] == "a" { "b" } else { "a" });
    assert_eq!(ExtendedPubKey::parse(&broken), Err(KeyError::Malformed));
    assert_eq!(ExtendedPubKey::parse(""), Err(KeyError::Malformed));
    assert_eq!(
        ExtendedPubKey::parse("bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu"),
        Err(KeyError::Malformed)
    );
}

#[test]
fn the_key_never_shows_in_debug_output_or_the_account_ref() {
    let k = ExtendedPubKey::parse(ZPUB).unwrap();
    assert!(!format!("{k:?}").contains("zpub"));
    let r = account_ref(&k);
    let as_xpub = ExtendedPubKey::parse(&zpub_as_xpub()).unwrap();
    let descriptor = ExtendedPubKey::parse_input(&format!("wpkh({ZPUB}/<0;1>/*)")).unwrap();
    assert_eq!(
        r,
        account_ref(&as_xpub),
        "xpub and zpub of one account are one account"
    );
    assert_eq!(r, account_ref(&descriptor));
    assert!(r.starts_with("bitcoin-") && !r.contains(&ZPUB[4..20]));
    let other = ExtendedPubKey::parse(XPUB).unwrap();
    assert_ne!(r, account_ref(&other));
}

#[test]
fn hardened_children_cannot_be_derived() {
    let k = ExtendedPubKey::parse(ZPUB).unwrap();
    assert!(k.child(0x8000_0000).is_none());
}

/// A block time shortly after the genesis block (2011).
const T0: i64 = 1_300_000_000;

fn tx(txid: &str, time: Option<i64>, vin: &[(&str, i64)], vout: &[(&str, i64)]) -> Value {
    json!({
        "txid": txid,
        "status": match time { Some(t) => json!({"confirmed": true, "block_time": T0 + t}), None => json!({"confirmed": false}) },
        "vin": vin.iter().map(|(a, v)| json!({"prevout": {"scriptpubkey_address": a, "value": v}})).collect::<Vec<_>>(),
        "vout": vout.iter().map(|(a, v)| json!({"scriptpubkey_address": a, "value": v})).collect::<Vec<_>>(),
    })
}

#[test]
fn wallet_transactions_net_received_against_spent_and_skip_unconfirmed() {
    let own: HashSet<String> = ["mine1", "mine2", "change"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let txs = vec![
        tx(
            "b",
            Some(2_000),
            &[("mine1", 50_000)],
            &[("shop", 30_000), ("change", 19_000)],
        ),
        tx(
            "a",
            Some(1_000),
            &[("faucet", 80_000)],
            &[("mine1", 50_000), ("mine2", 25_000)],
        ),
        tx(
            "a",
            Some(1_000),
            &[("faucet", 80_000)],
            &[("mine1", 50_000), ("mine2", 25_000)],
        ),
        tx("c", None, &[("x", 9)], &[("mine2", 9)]),
        tx("d", Some(3_000), &[("mine2", 25_000)], &[("mine1", 24_500)]),
    ];
    let w = wallet_transactions(&txs, &own).unwrap();
    let summary: Vec<(&str, i64)> = w.iter().map(|t| (t.txid.as_str(), t.net_sats)).collect();
    // a twice listed tx counts once; the self-transfer costs its fee only
    assert_eq!(summary, vec![("a", 75_000), ("b", -31_000), ("d", -500)]);
}

#[derive(Default)]
struct Fake {
    by_address: std::collections::HashMap<String, Vec<Value>>,
    asked: Mutex<Vec<String>>,
    /// Size of the first page (a server with larger pages); `None` = 25.
    first_page: Option<usize>,
    /// Every address has a transaction (a wallet beyond the address cap).
    all_used: bool,
    /// The server ignores the cursor and sends the first page again.
    ignore_cursor: bool,
}

#[async_trait]
impl TxSource for Fake {
    async fn address_txs(&self, address: &str, after: Option<&str>) -> CoreResult<Vec<Value>> {
        self.asked.lock().unwrap().push(address.to_string());
        if self.all_used {
            return Ok(vec![tx(address, Some(1), &[("x", 1)], &[(address, 1)])]);
        }
        let all = self.by_address.get(address).cloned().unwrap_or_default();
        let after = if self.ignore_cursor { None } else { after };
        Ok(match after {
            None => all
                .into_iter()
                .take(self.first_page.unwrap_or(PAGE))
                .collect(),
            Some(last) => {
                let pos = all
                    .iter()
                    .position(|t| t["txid"] == last)
                    .map_or(all.len(), |p| p + 1);
                all.into_iter().skip(pos).take(PAGE).collect()
            }
        })
    }
}

#[tokio::test]
async fn scan_follows_the_gap_limit_and_pages() {
    let k = ExtendedPubKey::parse(ZPUB).unwrap();
    let addr = |chain, i| keys::address(ScriptKind::P2wpkh, &k.public_key(chain, i).unwrap());
    let mut fake = Fake::default();
    fake.by_address.insert(
        addr(0, 0),
        vec![tx("t0", Some(10), &[("x", 1)], &[(&addr(0, 0), 1)])],
    );
    // index 19 is used: the scan must reach it and then look 20 further
    let many: Vec<Value> = (0..30)
        .map(|i| {
            tx(
                &format!("p{i}"),
                Some(100 + i),
                &[("x", 1)],
                &[(&addr(0, 19), 1_000)],
            )
        })
        .collect();
    fake.by_address.insert(addr(0, 19), many);
    let (own, txs) = scan(&k, &fake).await.unwrap();
    assert_eq!(txs.len(), 31, "both pages of the busy address are read");
    assert!(own.contains(&addr(0, 39)) && !own.contains(&addr(0, 40)));
    assert!(
        own.contains(&addr(1, 19)) && !own.contains(&addr(1, 20)),
        "change chain: 20 unused"
    );
    let w = wallet_transactions(&txs, &own).unwrap();
    assert_eq!(w.iter().map(|t| t.net_sats).sum::<i64>(), 30_001);
}

#[test]
fn to_fetch_reports_a_depot_with_a_bitcoin_position_and_valued_entries() {
    let txs = vec![
        WalletTx {
            txid: "a".into(),
            time: 1_700_000_000,
            net_sats: 150_000_000,
        },
        WalletTx {
            txid: "b".into(),
            time: 1_700_100_000,
            net_sats: -50_000_000,
        },
    ];
    let day_a = crate::dates::iso(crate::dates::local_date(
        jiff::Timestamp::from_second(1_700_000_000).unwrap(),
    ));
    let mut prices = BTreeMap::new();
    prices.insert(day_a.clone(), 3_400_000); // 34 000 € per BTC
    let out = to_fetch("bitcoin-x", &txs, 9_000_000, &prices);
    let acc = &out.accounts[0];
    assert_eq!(
        (acc.kind, acc.currency.as_str(), acc.balance_cents),
        (AccountKind::Brokerage, "EUR", 9_000_000)
    );
    let h = &out.holdings[0];
    assert!(h.direct_crypto);
    assert_eq!(
        (
            h.holding.wkn.as_str(),
            h.holding.quantity,
            h.holding.value_cents
        ),
        ("BTC", 1.0, 9_000_000)
    );
    let crate::asset_classification::AssetClassification { asset_class, .. } =
        crate::asset_classification::classify_asset(
            h.holding.clone(),
            Default::default(),
            Default::default(),
            h.direct_crypto,
        );
    assert_eq!(asset_class, crate::asset_classification::AssetClass::Crypto);
    let e = &out.security_transactions;
    assert_eq!(e[0].transaction.booking_date, day_a);
    assert_eq!(
        (e[0].transaction.quantity, e[0].transaction.value_cents),
        (1.5, 5_100_000)
    );
    assert_eq!(e[0].transaction.booking_text, "Eingang");
    // no day price: the entry stays, without a euro value
    assert_eq!(
        (e[1].transaction.quantity, e[1].transaction.value_cents),
        (0.5, 0)
    );
    assert_eq!(e[1].transaction.booking_text, "Ausgang");
    assert_eq!(out.balances_reported, vec!["bitcoin-x".to_string()]);
    assert_eq!(out.holdings_fetched, vec!["bitcoin-x".to_string()]);
}

#[test]
fn an_empty_wallet_is_a_depot_without_positions() {
    let out = to_fetch("bitcoin-x", &[], 9_000_000, &BTreeMap::new());
    assert_eq!(out.accounts[0].balance_cents, 0);
    assert!(out.holdings.is_empty());
    assert_eq!(
        out.holdings_fetched,
        vec!["bitcoin-x".to_string()],
        "a fully spent wallet clears its position"
    );
}

#[test]
fn price_answers_are_read() {
    assert_eq!(
        current_eur_price(&json!({"time": 1, "USD": 99000, "EUR": 91234.5})),
        Some(91234.5)
    );
    assert_eq!(current_eur_price(&json!({"USD": 1})), None);
    assert_eq!(
        historical_eur_price(&json!({"prices": [{"time": 1, "EUR": 34000}]})),
        Some(34000.0)
    );
    assert_eq!(historical_eur_price(&json!({"prices": []})), None);
}

#[test]
fn server_urls_must_be_https_or_on_the_own_network() {
    assert_eq!(
        validate_server_url("https://mempool.space/api/").unwrap(),
        "https://mempool.space/api"
    );
    assert!(validate_server_url("http://node.local:3006/api").is_ok());
    assert!(validate_server_url("https://node.example/api#mainnet").is_err());
    assert!(validate_server_url("http://192.168.1.20:3002").is_ok());
    assert!(
        validate_server_url("http://100.101.1.2/api").is_ok(),
        "Tailscale range"
    );
    assert!(validate_server_url("http://[::1]:3000").is_ok());
    assert!(validate_server_url("http://mempool.space/api").is_err());
    assert!(validate_server_url("https://user:pw@mempool.space/api").is_err());
    assert!(validate_server_url("ftp://192.168.1.2").is_err());
    assert!(validate_server_url("mempool.space").is_err());
    assert!(validate_server_url("https://mempool.space/api?x=1").is_err());
}

#[test]
fn base58check_round_trips_and_detects_typos() {
    let raw = keys::base58check_decode(ZPUB).unwrap();
    assert_eq!(keys::base58check_encode(raw[0], &raw[1..]), ZPUB);
    assert_eq!(
        keys::base58check_encode(0x00, &[0u8; 20]),
        "1111111111111111111114oLvT2"
    );
}

/// Live check against mempool.space with the public BIP84 test wallet
/// (`cargo test -p girocrab-core --lib bitcoin::tests::live -- --ignored`).
#[tokio::test]
#[ignore = "network"]
async fn live_scan_of_the_public_test_wallet() {
    let ctx = ConnectContext {
        connection_id: 1,
        login_name: ZPUB.into(),
        secret: crate::secret::SecretString::new(String::new()),
        settings: Default::default(),
        state: SecretJson::from_text(String::new()),
        since: "2000-01-01".into(),
        since_by_account: Default::default(),
        documents: crate::connectors::Documents::Off,
        skipped_accounts: Default::default(),
        only_account: None,
    };
    let mut c = BitcoinConnector::new();
    assert!(matches!(c.begin(&ctx).await.unwrap(), Phase::Ready));
    let out = c.fetch(&ctx).await.unwrap();
    let h = out.holdings.first();
    eprintln!(
        "entries {} balance {} ct, BTC {:?}, price {:?} ct, warnings {:?}",
        out.security_transactions.len(),
        out.accounts[0].balance_cents,
        h.map(|h| h.holding.quantity),
        h.map(|h| h.holding.current_price_cents),
        out.warnings
    );
    assert!(!out.security_transactions.is_empty());
}

/// The BIP84 account key with xpub version bytes, as several wallets export it.
fn zpub_as_xpub() -> String {
    let mut raw = keys::base58check_decode(ZPUB).unwrap();
    raw[..4].copy_from_slice(&[0x04, 0x88, 0xB2, 0x1E]);
    keys::base58check_encode(raw[0], &raw[1..])
}

#[tokio::test]
async fn an_xpub_of_a_native_segwit_wallet_finds_its_bc1q_history() {
    let k = ExtendedPubKey::parse(&zpub_as_xpub()).unwrap();
    let segwit = keys::address(ScriptKind::P2wpkh, &k.public_key(0, 0).unwrap());
    assert_eq!(segwit, "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu");
    let mut fake = Fake::default();
    fake.by_address.insert(
        segwit.clone(),
        vec![tx("s", Some(5), &[("x", 1)], &[(&segwit, 7_000)])],
    );
    let (own, txs) = scan(&k, &fake).await.unwrap();
    assert_eq!(
        wallet_transactions(&txs, &own)
            .unwrap()
            .iter()
            .map(|t| t.net_sats)
            .sum::<i64>(),
        7_000
    );
}

#[test]
fn output_descriptors_are_accepted_and_set_the_script_type() {
    let x = zpub_as_xpub();
    let k = ExtendedPubKey::parse_input(&format!(
        "wpkh([73c5da0a/84h/0h/0h]{x}/<0;1>/*)#{}",
        keys::descriptor_checksum(&format!("wpkh([73c5da0a/84h/0h/0h]{x}/<0;1>/*)")).unwrap()
    ))
    .unwrap();
    assert_eq!((k.kinds.clone(), k.depth), (vec![ScriptKind::P2wpkh], 3));
    assert_eq!(
        keys::address(ScriptKind::P2wpkh, &k.public_key(0, 0).unwrap()),
        "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu"
    );
    let k = ExtendedPubKey::parse_input(&format!("sh(wpkh({x}/0/*))")).unwrap();
    assert_eq!(k.kinds, vec![ScriptKind::P2shP2wpkh]);
    assert_eq!(
        ExtendedPubKey::parse_input(&format!("  {x}/0/*  "))
            .unwrap()
            .kinds,
        vec![ScriptKind::P2wpkh, ScriptKind::P2pkh]
    );
    assert_eq!(
        ExtendedPubKey::parse_input(&format!("tr([73c5da0a/86h/0h/0h]{x}/0/*)")),
        Err(KeyError::Unsupported)
    );
}

#[test]
fn a_key_of_a_single_address_chain_is_rejected() {
    // the receive chain m/84'/0'/0'/0 as its own extended key (depth 4): its
    // change chain is out of reach, every payment would look like a full loss
    let acct = ExtendedPubKey::parse(ZPUB).unwrap();
    let (cc, k) = acct.child(0).unwrap();
    let mut raw = keys::base58check_decode(ZPUB).unwrap();
    raw[4] = 4;
    raw[13..45].copy_from_slice(&cc);
    raw[45..78].copy_from_slice(&k);
    let text = keys::base58check_encode(raw[0], &raw[1..]);
    assert_eq!(
        ExtendedPubKey::parse_input(&text),
        Err(KeyError::ChainLevel)
    );
}

#[tokio::test]
async fn an_xpub_scans_segwit_and_legacy() {
    let k = ExtendedPubKey::parse(XPUB).unwrap();
    let legacy = keys::address(ScriptKind::P2pkh, &k.public_key(0, 0).unwrap());
    let segwit = keys::address(ScriptKind::P2wpkh, &k.public_key(0, 0).unwrap());
    // legacy-only wallet: found after the empty SegWit scan
    let mut fake = Fake::default();
    fake.by_address.insert(
        legacy.clone(),
        vec![tx("l", Some(1), &[("x", 1)], &[(&legacy, 3_000)])],
    );
    let (own, txs) = scan(&k, &fake).await.unwrap();
    assert_eq!(wallet_transactions(&txs, &own).unwrap().len(), 1);
    // a wallet with coins under both types: both count
    fake.by_address.insert(
        segwit.clone(),
        vec![tx("s", Some(2), &[("x", 1)], &[(&segwit, 4_000)])],
    );
    let (own, txs) = scan(&k, &fake).await.unwrap();
    let w = wallet_transactions(&txs, &own).unwrap();
    assert_eq!(w.iter().map(|t| t.net_sats).sum::<i64>(), 7_000);
}

#[tokio::test]
async fn larger_pages_are_followed_too() {
    let k = ExtendedPubKey::parse(ZPUB).unwrap();
    let addr = keys::address(ScriptKind::P2wpkh, &k.public_key(0, 0).unwrap());
    let many: Vec<Value> = (0..80)
        .map(|i| tx(&format!("q{i}"), Some(100 + i), &[("x", 1)], &[(&addr, 10)]))
        .collect();
    let mut fake = Fake {
        first_page: Some(50),
        ..Default::default()
    };
    fake.by_address.insert(addr.clone(), many);
    let (own, txs) = scan(&k, &fake).await.unwrap();
    assert_eq!(
        wallet_transactions(&txs, &own).unwrap().len(),
        80,
        "50 + 25 + 5"
    );
}

#[test]
fn day_prices_are_asked_for_by_calendar_day() {
    assert_eq!(day_start(1_700_000_000), 1_699_920_000);
    assert_eq!(day_start(1_699_920_000), 1_699_920_000);
    assert_eq!(day_start(1_699_920_000 + 86_399), 1_699_920_000);
}

#[test]
fn a_negative_total_reports_no_position() {
    let txs = vec![WalletTx {
        txid: "z".into(),
        time: 1,
        net_sats: -5,
    }];
    assert!(
        to_fetch("bitcoin-x", &txs, 9_000_000, &BTreeMap::new())
            .holdings
            .is_empty()
    );
}

#[test]
fn a_confirmed_copy_counts_although_an_unconfirmed_one_came_first() {
    let own: HashSet<String> = ["mine".to_string()].into();
    let unconfirmed = tx("t", None, &[("x", 1)], &[("mine", 900)]);
    let confirmed = tx("t", Some(5), &[("x", 1)], &[("mine", 900)]);
    let w = wallet_transactions(&[unconfirmed, confirmed.clone(), confirmed], &own).unwrap();
    assert_eq!(w.iter().map(|t| t.net_sats).collect::<Vec<_>>(), vec![900]);
}

#[test]
fn incomplete_transaction_data_is_an_error_but_coinbase_inputs_are_fine() {
    let own: HashSet<String> = ["mine".to_string()].into();
    let mut no_prevout = tx(
        "p",
        Some(5),
        &[("mine", 1_000)],
        &[("mine", 600), ("shop", 300)],
    );
    no_prevout["vin"][0]
        .as_object_mut()
        .unwrap()
        .remove("prevout");
    assert!(
        wallet_transactions(&[no_prevout], &own).is_err(),
        "the change would count as a deposit"
    );
    let mut no_value = tx("v", Some(5), &[("x", 1)], &[("mine", 600)]);
    no_value["vout"][0].as_object_mut().unwrap().remove("value");
    assert!(wallet_transactions(&[no_value], &own).is_err());
    let mut too_old = tx("o", Some(5), &[("x", 1)], &[("mine", 600)]);
    too_old["status"]["block_time"] = json!(1_000);
    assert!(wallet_transactions(&[too_old], &own).is_err());
    let mut mined = tx("m", Some(5), &[], &[("mine", 625_000_000)]);
    mined["vin"] = json!([{"is_coinbase": true}]);
    // an output without an address (OP_RETURN) is fine too
    mined["vout"]
        .as_array_mut()
        .unwrap()
        .push(json!({"value": 0}));
    let w = wallet_transactions(&[mined], &own).unwrap();
    assert_eq!(w[0].net_sats, 625_000_000);
}

#[tokio::test]
async fn a_wallet_beyond_the_address_cap_is_not_reported_as_complete() {
    let k = ExtendedPubKey::parse(ZPUB).unwrap();
    let fake = Fake {
        all_used: true,
        ..Default::default()
    };
    assert!(scan(&k, &fake).await.is_err());
}

#[tokio::test]
async fn a_server_that_repeats_a_page_is_an_error() {
    let k = ExtendedPubKey::parse(ZPUB).unwrap();
    let addr = keys::address(ScriptKind::P2wpkh, &k.public_key(0, 0).unwrap());
    let many: Vec<Value> = (0..30)
        .map(|i| tx(&format!("r{i}"), Some(100 + i), &[("x", 1)], &[(&addr, 10)]))
        .collect();
    let mut fake = Fake {
        ignore_cursor: true,
        ..Default::default()
    };
    fake.by_address.insert(addr, many);
    assert!(scan(&k, &fake).await.is_err());
}

#[test]
fn descriptor_checksums_paths_and_depths_are_checked() {
    // BIP380 test vector
    assert_eq!(
        keys::descriptor_checksum("raw(deadbeef)").as_deref(),
        Some("89f8spxm")
    );
    let body = format!("wpkh({ZPUB}/<0;1>/*)");
    let sum = keys::descriptor_checksum(&body).unwrap();
    assert!(ExtendedPubKey::parse_input(&format!("{body}#{sum}")).is_ok());
    // the wrapper changed, the checksum no longer fits
    let changed = format!("pkh({ZPUB}/<0;1>/*)#{sum}");
    assert_eq!(
        ExtendedPubKey::parse_input(&changed),
        Err(KeyError::Malformed)
    );
    // paths the scan does not derive
    for path in ["/<2;3>/*", "/5/*", "/0h/*", "/0/0"] {
        assert_eq!(
            ExtendedPubKey::parse_input(&format!("wpkh({ZPUB}{path})")),
            Err(KeyError::Unsupported),
            "{path}"
        );
    }
    for path in ["", "/<0;1>/*", "/0/*", "/1/*"] {
        assert!(
            ExtendedPubKey::parse_input(&format!("wpkh({ZPUB}{path})")).is_ok(),
            "{path}"
        );
    }
    // a coin-type key (depth 2) cannot reach its accounts
    let mut raw = keys::base58check_decode(ZPUB).unwrap();
    raw[4] = 2;
    let coin_type = keys::base58check_encode(raw[0], &raw[1..]);
    assert_eq!(
        ExtendedPubKey::parse_input(&coin_type),
        Err(KeyError::NotAccount)
    );
}

#[test]
fn the_price_day_is_the_utc_day_for_request_and_cache() {
    // 23:30 UTC on 14 Nov 2023 is already the 15th in Berlin; the price is the 14th's
    let t = 1_700_004_600;
    assert_eq!(price_day(t), "2023-11-14");
    assert_eq!(price_day(day_start(t)), "2023-11-14");
}
