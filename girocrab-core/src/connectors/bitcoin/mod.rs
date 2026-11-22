//! Bitcoin wallet, watch-only from an extended public key (xpub / ypub / zpub).
//!
//! The login field holds the key; there is no password. The connector derives
//! the wallet's receive and change addresses (gap limit 20), asks an Esplora
//! server (default mempool.space, or the user's own one in the setting
//! [`SERVER_SETTING`]) for their transactions and reports one depot with a
//! single position "Bitcoin" (quantity in BTC, value in EUR at the current
//! price). Every wallet transaction becomes a depot entry ("Eingang" /
//! "Ausgang") valued at that day's price. Prices always come from
//! mempool.space; day prices are asked for by calendar day (UTC midnight),
//! never with a transaction's own block time.

pub mod keys;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap, HashSet};

use async_trait::async_trait;
use serde_json::Value;

use self::keys::{ExtendedPubKey, KeyError};
use super::{
    BankConnector, ChallengeResponse, ConnectContext, FetchResult, Phase, RemoteAccount,
    RemoteHolding, RemoteSecurityTransaction,
};
use crate::error::{CoreError, CoreResult};
use crate::i18n::tr;
use crate::models::{AccountKind, Holding, SecurityDirection, SecurityTransaction};
use crate::secret::SecretJson;

/// Setting with the user's own Esplora server (e.g. `https://node.local/api`).
pub const SERVER_SETTING: &str = "bitcoin.esplora_url";
/// Public Esplora API used when no own server is set.
pub const DEFAULT_SERVER: &str = "https://mempool.space/api";
/// Price API (current and day EUR prices); never sees an address or a block time.
const PRICE_SERVER: &str = "https://mempool.space/api";
/// Unused addresses in a row after which a chain counts as fully scanned (BIP44).
const GAP_LIMIT: u32 = 20;
/// Upper bound per chain, so a broken server cannot keep the scan going forever.
const MAX_ADDRESSES: u32 = 5_000;
/// Esplora returns confirmed transactions of an address in pages of 25.
const PAGE: usize = 25;
const SATS_PER_BTC: f64 = 100_000_000.0;

fn key_message(e: KeyError) -> String {
    match e {
        KeyError::Private => tr("Das ist ein privater Schlüssel. Bitte nur den erweiterten öffentlichen Schlüssel (xpub, ypub oder zpub) eintragen."),
        KeyError::Unsupported => tr("Dieses Schlüsselformat wird nicht unterstützt. Möglich sind xpub, ypub und zpub (Bitcoin-Mainnet, Einzelsignatur)."),
        KeyError::Malformed => tr("Der Schlüssel ist ungültig. Bitte den erweiterten öffentlichen Schlüssel (xpub, ypub oder zpub) vollständig eintragen."),
        KeyError::NotAccount => tr("Das ist kein Schlüssel eines Wallet-Kontos. Bitte den Konto-Schlüssel eintragen (z. B. m/84'/0'/0')."),
        KeyError::ChainLevel => tr("Das ist der Schlüssel einer einzelnen Adresskette. Bitte den Schlüssel des Wallet-Kontos eintragen (z. B. m/84'/0'/0'), sonst fehlen die Wechselgeld-Adressen."),
    }
    .to_string()
}

/// Checks the user's server URL: https anywhere, plain http only for hosts on
/// the own network (own node at home). Returns it without a trailing slash.
pub fn validate_server_url(url: &str) -> CoreResult<String> {
    let bad = || {
        CoreError::invalid(tr(
            "Die Adresse des Bitcoin-Servers ist ungültig. Erwartet wird z. B. https://mempool.space/api.",
        ))
    };
    let trimmed = url.trim().trim_end_matches('/');
    let parsed = url::Url::parse(trimmed).map_err(|_| bad())?;
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(bad());
    }
    let host = parsed.host_str().ok_or_else(bad)?.to_ascii_lowercase();
    match parsed.scheme() {
        "https" => {}
        "http" if is_local_host(&host) => {}
        _ => return Err(bad()),
    }
    Ok(trimmed.to_string())
}

fn is_local_host(host: &str) -> bool {
    if host == "localhost"
        || host.ends_with(".local")
        || host.ends_with(".lan")
        || host.ends_with(".home.arpa")
    {
        return true;
    }
    match host.trim_matches(['[', ']']).parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => {
            ip.is_private()
                || ip.is_loopback()
                || (ip.octets()[0] == 100 && (64..128).contains(&ip.octets()[1]))
        }
        Ok(std::net::IpAddr::V6(ip)) => ip.is_loopback() || (ip.segments()[0] & 0xfe00) == 0xfc00,
        Err(_) => false,
    }
}

/// Stable account reference that does not reveal the key. It comes from the
/// key itself, so the same wallet account entered as xpub, zpub or descriptor
/// is one account (a second login delivering it is held back, not doubled).
pub fn account_ref(key: &ExtendedPubKey) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(key.chain_code);
    h.update(key.key);
    format!("bitcoin-{}", hex::encode(&h.finalize()[..8]))
}

// ------------------------------------------------------------------ transactions

/// One transaction as far as the wallet is concerned.
#[derive(Debug, Clone, PartialEq)]
pub struct WalletTx {
    pub txid: String,
    /// Block time (Unix seconds); unconfirmed transactions are left out.
    pub time: i64,
    /// Satoshis received minus satoshis spent by the wallet (fee included).
    pub net_sats: i64,
}

/// Inputs and outputs of a confirmed Esplora transaction: `(address, sats)`;
/// an output script without an address (`OP_RETURN` …) has an empty one.
struct RawTx {
    txid: String,
    time: i64,
    inputs: Vec<(String, i64)>,
    outputs: Vec<(String, i64)>,
}

/// All bitcoins there will ever be, in satoshis: no amount can be larger.
const MAX_SATS: i64 = 21_000_000 * 100_000_000;
/// Block time of the genesis block: no confirmed transaction is older.
const GENESIS_TIME: i64 = 1_231_006_505;

fn incomplete() -> CoreError {
    CoreError::invalid(tr(
        "Der Bitcoin-Server hat unvollständige Transaktionsdaten geliefert.",
    ))
}

/// A confirmed transaction, `None` for an unconfirmed one. Missing amounts, a
/// missing `prevout` of a non-coinbase input or an impossible time are an error:
/// leaving such an input out would count the change as a deposit.
fn raw_tx(v: &Value) -> CoreResult<Option<RawTx>> {
    let txid = v
        .get("txid")
        .and_then(Value::as_str)
        .ok_or_else(incomplete)?
        .to_string();
    let status = v.get("status");
    // a missing or malformed flag is no reason to skip: the history would be incomplete
    match status
        .and_then(|s| s.get("confirmed"))
        .and_then(Value::as_bool)
    {
        Some(true) => {}
        Some(false) => return Ok(None),
        None => return Err(incomplete()),
    }
    let latest = jiff::Timestamp::now().as_second() + 86_400;
    let time = status
        .and_then(|s| s.get("block_time"))
        .and_then(Value::as_i64)
        .filter(|t| (GENESIS_TIME..=latest).contains(t))
        .ok_or_else(incomplete)?;
    let amount = |o: &Value| -> CoreResult<(String, i64)> {
        let sats = o
            .get("value")
            .and_then(Value::as_i64)
            .filter(|v| (0..=MAX_SATS).contains(v))
            .ok_or_else(incomplete)?;
        let address = o
            .get("scriptpubkey_address")
            .and_then(Value::as_str)
            .unwrap_or_default();
        Ok((address.to_string(), sats))
    };
    let mut inputs = Vec::new();
    for i in v
        .get("vin")
        .and_then(Value::as_array)
        .ok_or_else(incomplete)?
    {
        if i.get("is_coinbase").and_then(Value::as_bool) == Some(true) {
            continue; // newly mined coins have no previous output
        }
        inputs.push(amount(i.get("prevout").ok_or_else(incomplete)?)?);
    }
    let outputs = v
        .get("vout")
        .and_then(Value::as_array)
        .ok_or_else(incomplete)?
        .iter()
        .map(amount)
        .collect::<CoreResult<Vec<_>>>()?;
    Ok(Some(RawTx {
        txid,
        time,
        inputs,
        outputs,
    }))
}

/// Net effect of every confirmed transaction on the wallet `own`, oldest first.
/// A transaction listed several times (once per address, unconfirmed at first)
/// counts once, from a confirmed copy.
pub fn wallet_transactions(txs: &[Value], own: &HashSet<String>) -> CoreResult<Vec<WalletTx>> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for v in txs {
        let Some(t) = raw_tx(v)? else { continue };
        if !seen.insert(t.txid.clone()) {
            continue;
        }
        let received: i64 = t
            .outputs
            .iter()
            .filter(|(a, _)| own.contains(a))
            .map(|(_, v)| v)
            .sum();
        let spent: i64 = t
            .inputs
            .iter()
            .filter(|(a, _)| own.contains(a))
            .map(|(_, v)| v)
            .sum();
        if received != spent {
            out.push(WalletTx {
                txid: t.txid,
                time: t.time,
                net_sats: received - spent,
            });
        }
    }
    out.sort_by(|a, b| a.time.cmp(&b.time).then_with(|| a.txid.cmp(&b.txid)));
    Ok(out)
}

// ------------------------------------------------------------------ scanning

/// Where the transactions of an address come from (Esplora, or a fake in tests).
#[async_trait]
pub trait TxSource: Send + Sync {
    /// Confirmed and unconfirmed transactions of `address`; `after` = the last
    /// confirmed txid of the previous page.
    async fn address_txs(&self, address: &str, after: Option<&str>) -> CoreResult<Vec<Value>>;
}

/// Derives addresses of every script kind and chain until `GAP_LIMIT` unused
/// ones follow each other, and collects all their transactions.
pub async fn scan(
    key: &ExtendedPubKey,
    source: &dyn TxSource,
) -> CoreResult<(HashSet<String>, Vec<Value>)> {
    let mut own = HashSet::new();
    let mut txs = Vec::new();
    crate::depot_debug::log(&format!(
        "bitcoin: key depth {} kinds {:?}",
        key.depth, key.kinds
    ));
    // an xpub scans every script type it may stand for: a wallet can hold coins under both
    for &kind in &key.kinds {
        for chain in 0..2u32 {
            let (cc, k) = key
                .child(chain)
                .ok_or_else(|| CoreError::invalid(key_message(KeyError::Malformed)))?;
            let branch = ExtendedPubKey {
                chain_code: cc,
                key: k,
                kinds: vec![kind],
                depth: key.depth.saturating_add(1),
            };
            let (mut used, before) = (0u32, txs.len());
            let mut unused = 0;
            let mut index = 0u32;
            while unused < GAP_LIMIT && index < MAX_ADDRESSES {
                let Some(pk) = branch.child(index).map(|(_, pk)| pk) else {
                    index += 1; // invalid child (probability ~2^-127): skip it
                    continue;
                };
                let address = keys::address(kind, &pk);
                let found = address_history(source, &address).await?;
                if found.is_empty() {
                    unused += 1;
                } else {
                    unused = 0;
                    used += 1;
                    txs.extend(found);
                }
                own.insert(address);
                index += 1;
            }
            crate::depot_debug::log(&format!(
                "bitcoin: {kind:?} chain {chain}: {index} addresses, {used} used, {} tx entries",
                txs.len() - before
            ));
            if unused < GAP_LIMIT {
                // stopped by the cap, not by the gap: the rest of the chain is unknown
                return Err(CoreError::invalid(tr(
                    "Das Wallet nutzt mehr Adressen, als ein Abruf durchsucht (5000 je Kette). Der Saldo wäre unvollständig.",
                )));
            }
        }
    }
    Ok((own, txs))
}

async fn address_history(source: &dyn TxSource, address: &str) -> CoreResult<Vec<Value>> {
    let mut all = source.address_txs(address, None).await?;
    let mut confirmed: Vec<String> = confirmed_ids(&all);
    let mut cursors = HashSet::new();
    // a full page of confirmed transactions: the next page starts after the last one
    // (some servers hand out larger pages); an empty page ends the list
    while confirmed.len() >= PAGE {
        let last = confirmed.last().cloned().unwrap_or_default();
        if !cursors.insert(last.clone()) {
            // the server sent a page again: going on would loop or miss transactions
            return Err(CoreError::invalid(tr(
                "Der Bitcoin-Server hat unerwartet geantwortet.",
            )));
        }
        let page = source.address_txs(address, Some(&last)).await?;
        let next = confirmed_ids(&page);
        if next.contains(&last) {
            return Err(CoreError::invalid(tr(
                "Der Bitcoin-Server hat unerwartet geantwortet.",
            )));
        }
        all.extend(page);
        confirmed = next;
    }
    Ok(all)
}

fn confirmed_ids(page: &[Value]) -> Vec<String> {
    page.iter()
        .filter(|t| t.pointer("/status/confirmed").and_then(Value::as_bool) == Some(true))
        .filter_map(|t| t.get("txid").and_then(Value::as_str).map(str::to_string))
        .collect()
}

struct Esplora {
    http: reqwest::Client,
    base: String,
}

/// When the next request of any Bitcoin sync in this process may go out, so
/// that parallel syncs together keep the public servers' rate limits.
struct Slots {
    /// Earliest time of the next request (`REQUEST_GAP` after the previous one).
    next: Option<tokio::time::Instant>,
    /// End of the pause after an "HTTP 429".
    cooldown: Option<tokio::time::Instant>,
}

static SLOTS: std::sync::Mutex<Slots> = std::sync::Mutex::new(Slots {
    next: None,
    cooldown: None,
});

/// Waits until a request may go out and reserves its slot, `gap` before the
/// next one. Slot and pause are checked and taken in one step: a request that
/// waited looks again, so a pause started meanwhile (or another request that
/// took the slot) holds it back, and requests leave `gap` apart after a pause.
async fn wait_for_slot(gap: std::time::Duration) {
    loop {
        let wait = {
            let mut s = SLOTS.lock().unwrap_or_else(|e| e.into_inner());
            let now = tokio::time::Instant::now();
            let start = [s.next, s.cooldown]
                .into_iter()
                .flatten()
                .fold(now, |a, b| a.max(b));
            if start <= now {
                s.next = Some(now + gap);
                None
            } else {
                Some(start)
            }
        };
        match wait {
            None => return,
            Some(t) => tokio::time::sleep_until(t).await,
        }
    }
}

/// After "HTTP 429" every Bitcoin sync of the process waits `pause`, also the
/// requests that already wait for their slot.
fn push_back_all(pause: std::time::Duration) {
    let until = tokio::time::Instant::now() + pause;
    let mut s = SLOTS.lock().unwrap_or_else(|e| e.into_inner());
    s.cooldown = Some(s.cooldown.map_or(until, |t| t.max(until)));
}

/// Pause between two requests to the Esplora server.
const REQUEST_GAP: std::time::Duration = std::time::Duration::from_millis(250);
/// Waits after "HTTP 429 Too Many Requests" (unless the server names one).
const BACKOFF_SECS: [u64; 5] = [2, 4, 8, 16, 30];

#[async_trait]
impl TxSource for Esplora {
    async fn address_txs(&self, address: &str, after: Option<&str>) -> CoreResult<Vec<Value>> {
        let url = match after {
            None => format!("{}/address/{address}/txs", self.base),
            Some(txid) => format!("{}/address/{address}/txs/chain/{txid}", self.base),
        };
        get_json(&self.http, &url)
            .await?
            .as_array()
            .cloned()
            .ok_or_else(|| CoreError::invalid(tr("Der Bitcoin-Server hat unerwartet geantwortet.")))
    }
}

/// GET with a few retries when the connection drops (public servers close
/// busy connections now and then).
async fn send(http: &reqwest::Client, url: &str) -> CoreResult<reqwest::Response> {
    let mut last = None;
    for wait in [0u64, 2, 5, 10] {
        tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
        wait_for_slot(REQUEST_GAP).await;
        match http.get(url).send().await {
            Ok(r) => return Ok(r),
            Err(e) if e.is_connect() || e.is_timeout() || e.is_request() => last = Some(e),
            Err(e) => return Err(e.into()),
        }
    }
    Err(last.map(Into::into).unwrap_or_else(|| {
        CoreError::invalid(tr("Der Bitcoin-Server hat unerwartet geantwortet."))
    }))
}

/// One try of [`get_json`]: a final answer, or a connection that broke while
/// the answer was being read (worth asking again).
enum Attempt {
    Done(CoreResult<Value>),
    ReadFailed(CoreError),
}

async fn get_json(http: &reqwest::Client, url: &str) -> CoreResult<Value> {
    let mut attempt = 0;
    loop {
        match get_json_once(http, url).await {
            Attempt::Done(result) => return result,
            // `send` already retried the request itself; only a broken read is repeated
            Attempt::ReadFailed(e) if attempt >= 3 => return Err(e),
            Attempt::ReadFailed(_) => {
                attempt += 1;
                crate::depot_debug::log("bitcoin: connection lost while reading, retrying");
                tokio::time::sleep(std::time::Duration::from_secs(2 * attempt)).await;
            }
        }
    }
}

async fn get_json_once(http: &reqwest::Client, url: &str) -> Attempt {
    match get_json_try(http, url).await {
        Ok(v) => Attempt::Done(Ok(v)),
        Err(e) => e,
    }
}

async fn get_json_try(http: &reqwest::Client, url: &str) -> Result<Value, Attempt> {
    let done = |e: CoreError| Attempt::Done(Err(e));
    let mut resp = send(http, url).await.map_err(done)?;
    for wait in BACKOFF_SECS {
        if resp.status() != reqwest::StatusCode::TOO_MANY_REQUESTS {
            break;
        }
        let named = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|s| *s <= 60);
        let pause = std::time::Duration::from_secs(named.unwrap_or(wait));
        crate::depot_debug::log(&format!("bitcoin: HTTP 429, waiting {} s", pause.as_secs()));
        push_back_all(pause);
        resp = send(http, url).await.map_err(done)?;
    }
    let status = resp.status();
    if !status.is_success() {
        crate::depot_debug::log(&format!("bitcoin: HTTP {}", status.as_u16()));
        return Err(done(CoreError::invalid(format!(
            "{} (HTTP {})",
            tr("Der Bitcoin-Server hat die Anfrage abgelehnt"),
            status.as_u16()
        ))));
    }
    // An oversized answer stays oversized: it is final. A body that broke off
    // while being read is worth another try.
    let body = super::read_body_checked(resp, super::MAX_TEXT_BYTES)
        .await
        .map_err(|e| match e {
            super::BodyError::TooLarge(e) => done(e),
            super::BodyError::Read(e) => Attempt::ReadFailed(e),
        })?;
    serde_json::from_str(&crate::text::decode_bank_bytes(&body)).map_err(|_| {
        done(CoreError::invalid(tr(
            "Der Bitcoin-Server hat unerwartet geantwortet.",
        )))
    })
}

// ------------------------------------------------------------------ prices

/// EUR price of one BTC from mempool.space's `/v1/prices` answer.
pub fn current_eur_price(v: &Value) -> Option<f64> {
    v.get("EUR")?.as_f64().filter(|p| *p > 0.0)
}

/// EUR price from `/v1/historical-price` (`{"prices":[{"time":…,"EUR":…}]}`).
pub fn historical_eur_price(v: &Value) -> Option<f64> {
    v.get("prices")?
        .as_array()?
        .first()?
        .get("EUR")?
        .as_f64()
        .filter(|p| *p > 0.0)
}

/// Midnight (UTC) of the day of `unix` seconds.
pub fn day_start(unix: i64) -> i64 {
    unix - unix.rem_euclid(86_400)
}

/// The UTC calendar day of `unix` seconds (`YYYY-MM-DD`): the day whose price
/// is asked for and the key it is kept under.
pub fn price_day(unix: i64) -> String {
    crate::dates::iso(crate::dates::utc_date(
        jiff::Timestamp::from_second(day_start(unix)).unwrap_or_default(),
    ))
}

fn cents(sats: i64, eur_per_btc: f64) -> i64 {
    (sats as f64 / SATS_PER_BTC * eur_per_btc * 100.0).round() as i64
}

/// Stored state: EUR price per day (`YYYY-MM-DD` → cents per BTC), so a day's
/// price is asked for once.
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct State {
    #[serde(default)]
    day_prices: BTreeMap<String, i64>,
}

// ------------------------------------------------------------------ result

/// Builds the depot of one wallet: balance and position at `price_cents` per
/// BTC, one entry per transaction valued at its day's price (`day_prices`).
pub fn to_fetch(
    reference: &str,
    txs: &[WalletTx],
    price_cents: i64,
    day_prices: &BTreeMap<String, i64>,
) -> FetchResult {
    let sats: i64 = txs.iter().map(|t| t.net_sats).sum();
    let value = cents(sats, price_cents as f64 / 100.0);
    let mut out = FetchResult::default();
    out.accounts.push(RemoteAccount {
        external_ref: reference.to_string(),
        name: "Bitcoin-Wallet".into(),
        owner: String::new(),
        iban: String::new(),
        account_number: String::new(),
        currency: "EUR".into(),
        kind: AccountKind::Brokerage,
        balance_cents: value,
        balance_date: crate::dates::today_iso(),
    });
    out.balances_reported.push(reference.to_string());
    out.holdings_fetched.push(reference.to_string());
    if sats > 0 {
        out.holdings.push(RemoteHolding {
            classification: Default::default(),
            direct_crypto: true,
            account_ref: reference.to_string(),
            holding: Holding {
                isin: String::new(),
                wkn: "BTC".into(),
                name: "Bitcoin".into(),
                quantity: sats as f64 / SATS_PER_BTC,
                current_price_cents: price_cents,
                purchase_price_cents: 0,
                value_cents: value,
                ..Default::default()
            },
        });
    }
    for t in txs {
        let date = crate::dates::iso(crate::dates::local_date(
            jiff::Timestamp::from_second(t.time).unwrap_or_default(),
        ));
        let day = day_prices.get(&price_day(t.time)).copied().unwrap_or(0);
        let incoming = t.net_sats > 0;
        out.security_transactions.push(RemoteSecurityTransaction {
            account_ref: reference.to_string(),
            external_id: t.txid.clone(),
            cash_external_id: String::new(),
            transaction: SecurityTransaction {
                id: 0,
                account_id: 0,
                external_id: t.txid.clone(),
                booking_date: date.clone(),
                settlement_date: date,
                isin: String::new(),
                wkn: "BTC".into(),
                name: "Bitcoin".into(),
                direction: SecurityDirection::Other,
                quantity: t.net_sats.unsigned_abs() as f64 / SATS_PER_BTC,
                price_cents: day,
                currency: "EUR".into(),
                value_cents: cents(t.net_sats, day as f64 / 100.0),
                fees_cents: 0,
                taxes_cents: 0,
                accrued_interest_cents: 0,
                fx_rate: None,
                booking_text: if incoming {
                    tr("Eingang")
                } else {
                    tr("Ausgang")
                }
                .into(),
                cash_transaction_id: None,
            },
        });
    }
    out
}

// ------------------------------------------------------------------ connector

#[derive(Default)]
pub struct BitcoinConnector {
    key: Option<ExtendedPubKey>,
}

impl BitcoinConnector {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl BankConnector for BitcoinConnector {
    async fn begin(&mut self, ctx: &ConnectContext) -> CoreResult<Phase> {
        let key = ExtendedPubKey::parse_input(&ctx.login_name).map_err(|e| {
            crate::depot_debug::log(&format!("bitcoin: key rejected: {e:?}"));
            CoreError::invalid(key_message(e))
        })?;
        self.key = Some(key);
        Ok(Phase::Ready)
    }

    async fn respond(&mut self, ctx: &ConnectContext, _r: ChallengeResponse) -> CoreResult<Phase> {
        self.begin(ctx).await
    }

    async fn fetch(&mut self, ctx: &ConnectContext) -> CoreResult<FetchResult> {
        let key = match &self.key {
            Some(k) => k.clone(),
            None => ExtendedPubKey::parse_input(&ctx.login_name)
                .map_err(|e| CoreError::invalid(key_message(e)))?,
        };
        let base = match ctx
            .settings
            .get(SERVER_SETTING)
            .map(|s| s.expose().trim().to_string())
        {
            Some(own) if !own.is_empty() => validate_server_url(&own)?,
            _ => DEFAULT_SERVER.to_string(),
        };
        let http = super::http_client_same_host()?;
        let (own, raw) = scan(
            &key,
            &Esplora {
                http: http.clone(),
                base,
            },
        )
        .await?;
        let txs = wallet_transactions(&raw, &own)?;
        let sats: i64 = txs.iter().map(|t| t.net_sats).sum();
        if sats < 0 {
            // more spent than received: part of the history is missing, so no figure is right
            return Err(CoreError::invalid(tr(
                "Die Wallet-Historie ist unvollständig: Es wurde mehr ausgegeben als empfangen. Bitte den Schlüssel des Wallet-Kontos (z. B. m/84'/0'/0') prüfen.",
            )));
        }
        crate::depot_debug::log(&format!(
            "bitcoin: {} addresses, {} wallet transactions",
            own.len(),
            txs.len()
        ));

        let mut state: State = if ctx.state.is_empty() {
            State::default()
        } else {
            ctx.state.decode().unwrap_or_default()
        };
        let price =
            current_eur_price(&get_json(&http, &format!("{PRICE_SERVER}/v1/prices")).await?)
                .ok_or_else(|| {
                    CoreError::invalid(tr("Der aktuelle Bitcoin-Kurs ist nicht verfügbar."))
                })?;
        let mut warnings = Vec::new();
        let mut by_day: HashMap<String, i64> = HashMap::new();
        for t in &txs {
            let date = price_day(t.time);
            if state.day_prices.contains_key(&date) || by_day.contains_key(&date) {
                continue;
            }
            // the day, not the block time: exact times would tell the price server
            // which transactions are the wallet's
            let url = format!(
                "{PRICE_SERVER}/v1/historical-price?currency=EUR&timestamp={}",
                day_start(t.time)
            );
            match get_json(&http, &url).await.ok().as_ref().and_then(historical_eur_price) {
                Some(p) => {
                    by_day.insert(date, (p * 100.0).round() as i64);
                }
                None if warnings.is_empty() => warnings.push(tr("Für einzelne Bitcoin-Umsätze ist kein Tageskurs verfügbar; sie stehen ohne Euro-Wert in der Liste.").to_string()),
                None => {}
            }
        }
        state.day_prices.extend(by_day);
        if txs.is_empty() {
            warnings.push(if key.depth == 3 {
                tr("Keine Umsätze gefunden. Bitte prüfen, ob der Schlüssel zum richtigen Wallet-Konto gehört.").to_string()
            } else {
                tr("Keine Umsätze gefunden. Der Schlüssel ist kein Konto-Schlüssel (z. B. m/84'/0'/0'); viele Wallets nutzen nur Adressen unterhalb eines solchen.").to_string()
            });
        }
        let mut out = to_fetch(
            &account_ref(&key),
            &txs,
            (price * 100.0).round() as i64,
            &state.day_prices,
        );
        out.state = SecretJson::encode(&state).ok();
        out.warnings = warnings;
        out.accounts_complete = true;
        Ok(out)
    }
}
