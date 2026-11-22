//! Bitcoin Core (own node): the balance of a wallet in the user's own node,
//! read over its local JSON-RPC interface, valued at the current BTC/EUR price.
//!
//! - Login field: the wallet name; empty = the only wallet the node has loaded.
//! - Authentication: the node's `.cookie` file in its data directory (written
//!   while the RPC server runs, `server=1`); a stored secret `user:password`
//!   (`rpcauth`) is used instead when there is one.
//! - The RPC address must be on this machine (loopback): the cookie never leaves it.
//! - Only the price comes from outside (a public ticker, no account, nothing
//!   about the wallet is sent).
//!
//! The wallet becomes one depot with one position "Bitcoin" (ticker BTC,
//! directly held crypto). No bookings: the node knows amounts, not purposes.

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{
    BankConnector, ChallengeResponse, ConnectContext, FetchResult, Phase, RemoteAccount,
    RemoteHolding,
};
use crate::error::{CoreError, CoreResult};
use crate::i18n::{tr, trf};
use crate::models::{AccountKind, Holding};

/// Settings keys.
pub(crate) const DATADIR_KEY: &str = "bitcoin_core.datadir";
pub(crate) const RPC_URL_KEY: &str = "bitcoin_core.rpc_url";
/// The node's default RPC address (mainnet).
pub(crate) const DEFAULT_RPC_URL: &str = "http://127.0.0.1:8332";
/// Public BTC/EUR ticker (last trade price).
const PRICE_URL: &str = "https://api.kraken.com/0/public/Ticker?pair=XBTEUR";
const SATS_PER_BTC: f64 = 100_000_000.0;

#[derive(Default)]
pub struct BitcoinCoreConnector {
    http: Option<reqwest::Client>,
    url: String,
    auth: (String, String),
    wallet: String,
}

impl BitcoinCoreConnector {
    pub fn new() -> Self {
        Self::default()
    }

    /// `wallet`: the wallet's endpoint (`/wallet/<name>`), else the node's.
    async fn rpc(&self, wallet: Option<&str>, method: &str) -> CoreResult<Value> {
        let http = self
            .http
            .as_ref()
            .ok_or_else(|| CoreError::protocol("Bitcoin Core: no session"))?;
        let body = json!({"jsonrpc": "1.0", "id": "girocrab", "method": method, "params": []});
        let mut url = reqwest::Url::parse(&self.url)
            .map_err(|_| CoreError::invalid(tr("Bitcoin Core: Die RPC-Adresse ist ungültig.")))?;
        if let Some(name) = wallet {
            url.path_segments_mut()
                .map_err(|_| CoreError::invalid(tr("Bitcoin Core: Die RPC-Adresse ist ungültig.")))?
                .push("wallet")
                .push(name);
        }
        let resp = http
            .post(url)
            .basic_auth(&self.auth.0, Some(&self.auth.1))
            .json(&body)
            .send()
            .await
            .map_err(|_| unreachable_node())?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(CoreError::Authentication {
                message: tr("Bitcoin Core: Zugang abgelehnt. Die .cookie-Datei ist veraltet oder Benutzer:Passwort stimmt nicht.").into(),
            });
        }
        // the node answers RPC errors (unknown wallet …) with HTTP 500 and a JSON body
        let v = super::read_json_or_null(resp).await;
        if v.is_null() {
            return Err(CoreError::protocol(tr(
                "Bitcoin Core: unerwartete Antwort des Knotens.",
            )));
        }
        rpc_result(v)
    }
}

fn unreachable_node() -> CoreError {
    CoreError::Network {
        message: tr("Bitcoin Core ist nicht erreichbar. Läuft der Knoten, und ist der RPC-Server eingeschaltet (server=1)?").into(),
    }
}

/// The `result` of a JSON-RPC answer, or its `error` as a message.
fn rpc_result(v: Value) -> CoreResult<Value> {
    if let Some(err) = v.get("error").filter(|e| !e.is_null()) {
        let msg = err.get("message").and_then(Value::as_str).unwrap_or("");
        return Err(CoreError::protocol(trf(
            "Bitcoin Core meldet: {0}",
            &[&msg],
        )));
    }
    Ok(v.get("result").cloned().unwrap_or(Value::Null))
}

/// Only an address on this machine: the cookie (or password) must never leave it.
pub(crate) fn local_rpc_url(raw: &str) -> CoreResult<String> {
    let url = reqwest::Url::parse(raw.trim())
        .map_err(|_| CoreError::invalid(tr("Bitcoin Core: Die RPC-Adresse ist ungültig.")))?;
    let local = matches!(
        url.host(),
        Some(url::Host::Ipv4(ip)) if ip.is_loopback()
    ) || matches!(url.host(), Some(url::Host::Ipv6(ip)) if ip.is_loopback())
        || matches!(url.host(), Some(url::Host::Domain(d)) if d.eq_ignore_ascii_case("localhost"));
    if !local || !matches!(url.scheme(), "http" | "https") {
        return Err(CoreError::invalid(tr(
            "Bitcoin Core: Die RPC-Adresse muss auf diesem Rechner liegen (127.0.0.1 oder localhost).",
        )));
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

/// `user:password` from a `.cookie` file or the second login field.
pub(crate) fn parse_credentials(s: &str) -> Option<(String, String)> {
    let (user, pass) = s.trim().split_once(':')?;
    (!user.is_empty() && !pass.is_empty()).then(|| (user.to_string(), pass.to_string()))
}

/// The node's default data directory on this platform.
pub(crate) fn default_datadir() -> std::path::PathBuf {
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_default();
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Bitcoin")
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA")
            .map(std::path::PathBuf::from)
            .unwrap_or(home)
            .join("Bitcoin")
    } else {
        home.join(".bitcoin")
    }
}

/// The wallet to read: the one named, else the only one loaded.
pub(crate) fn pick_wallet(named: &str, loaded: &[String]) -> CoreResult<String> {
    let named = named.trim();
    if !named.is_empty() {
        // `listwallets` lists the loaded ones: a wallet not among them can't be read
        return if loaded.iter().any(|w| w == named) {
            Ok(named.to_string())
        } else {
            Err(CoreError::invalid(trf(
                "Bitcoin Core: Die Wallet „{0}“ ist im Knoten nicht geladen.",
                &[&named],
            )))
        };
    }
    match loaded {
        [only] => Ok(only.clone()),
        [] => Err(CoreError::invalid(tr(
            "Bitcoin Core: Im Knoten ist keine Wallet geladen.",
        ))),
        _ => Err(CoreError::invalid(tr(
            "Bitcoin Core: Der Knoten hat mehrere Wallets geladen. Bitte den Wallet-Namen eintragen.",
        ))),
    }
}

/// Satoshis the wallet holds from `getbalances`: confirmed, pending and immature,
/// its own (`mine`) and, in a legacy wallet with imported keys, the watch-only ones.
/// A balance the node must report but doesn't is an error, never zero.
pub(crate) fn wallet_sats(getbalances: &Value) -> CoreResult<i64> {
    let unexpected = || CoreError::protocol(tr("Bitcoin Core: unerwartete Antwort des Knotens."));
    let sum = |part: &Value| -> CoreResult<i64> {
        ["trusted", "untrusted_pending", "immature"]
            .iter()
            .map(|k| {
                part.get(*k)
                    .and_then(Value::as_f64)
                    .map(|btc| (btc * SATS_PER_BTC).round() as i64)
                    .ok_or_else(unexpected)
            })
            .sum()
    };
    let mine = getbalances.get("mine").ok_or_else(unexpected)?;
    let watch_only = match getbalances.get("watchonly") {
        Some(w) if !w.is_null() => sum(w)?,
        _ => 0,
    };
    Ok(sum(mine)? + watch_only)
}

/// Last trade price in euro cents from the ticker answer.
pub(crate) fn price_cents(ticker: &Value) -> CoreResult<i64> {
    let no_price = || {
        CoreError::protocol(tr(
            "Bitcoin-Kurs nicht abrufbar. Der Bestand wurde nicht aktualisiert.",
        ))
    };
    if ticker
        .get("error")
        .and_then(Value::as_array)
        .is_some_and(|e| !e.is_empty())
    {
        return Err(no_price());
    }
    let pair = ticker
        .get("result")
        .and_then(Value::as_object)
        .and_then(|r| r.values().next())
        .ok_or_else(no_price)?;
    let last = pair
        .get("c")
        .and_then(|c| c.get(0))
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|p| *p > 0.0)
        .ok_or_else(no_price)?;
    Ok((last * 100.0).round() as i64)
}

/// Value in cents of `sats` at `price_cents` per bitcoin.
pub(crate) fn value_cents(sats: i64, price_cents: i64) -> i64 {
    ((sats as i128 * price_cents as i128 + 50_000_000) / 100_000_000) as i64
}

/// The wallet as one depot with one Bitcoin position.
pub(crate) fn fetch_result(wallet: &str, sats: i64, price_cents: i64, date: &str) -> FetchResult {
    let account_ref = format!("bitcoin-core:{wallet}");
    let value = value_cents(sats, price_cents);
    let mut out = FetchResult::default();
    out.accounts.push(RemoteAccount {
        external_ref: account_ref.clone(),
        name: format!("Bitcoin ({wallet})"),
        owner: String::new(),
        iban: String::new(),
        account_number: String::new(),
        currency: "EUR".into(),
        kind: AccountKind::Brokerage,
        balance_cents: value,
        balance_date: date.to_string(),
    });
    if sats > 0 {
        out.holdings.push(RemoteHolding {
            classification: Default::default(),
            direct_crypto: true,
            account_ref: account_ref.clone(),
            holding: Holding {
                isin: String::new(),
                wkn: "BTC".into(),
                name: "Bitcoin".into(),
                quantity: sats as f64 / SATS_PER_BTC,
                current_price_cents: price_cents,
                value_cents: value,
                ..Default::default()
            },
        });
    }
    // an empty wallet is a complete inventory too
    out.holdings_fetched.push(account_ref.clone());
    out.balances_reported.push(account_ref);
    out
}

impl BitcoinCoreConnector {
    /// Logs in at the node and picks the wallet: `begin`'s work, shared with
    /// [`check_bitcoin_core`] before a login is added. Empty `datadir` /
    /// `rpc_url` mean the defaults; `secret` is `user:password` or empty (cookie).
    async fn connect(
        &mut self,
        datadir: &str,
        rpc_url: &str,
        secret: &str,
        wallet: &str,
    ) -> CoreResult<()> {
        let rpc_url = rpc_url.trim();
        self.url = local_rpc_url(if rpc_url.is_empty() {
            DEFAULT_RPC_URL
        } else {
            rpc_url
        })?;
        self.auth = match parse_credentials(secret) {
            Some(auth) => auth,
            None => {
                let dir = match datadir.trim() {
                    "" => default_datadir(),
                    d => std::path::PathBuf::from(d),
                };
                let cookie = std::fs::read_to_string(dir.join(".cookie")).map_err(|_| {
                    CoreError::Authentication {
                        message: trf(
                            "Bitcoin Core: Keine .cookie-Datei in {0}. Läuft der Knoten mit eingeschaltetem RPC-Server (server=1)?",
                            &[&dir.display().to_string()],
                        ),
                    }
                })?;
                parse_credentials(&cookie).ok_or_else(|| CoreError::Authentication {
                    message: tr("Bitcoin Core: Die .cookie-Datei ist unlesbar.").into(),
                })?
            }
        };
        // no proxy and no redirects: the credentials go to the node and nowhere else
        self.http = Some(
            super::client_builder(std::time::Duration::from_secs(60))
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
        );
        let loaded: Vec<String> = self
            .rpc(None, "listwallets")
            .await?
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|w| w.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        self.wallet = pick_wallet(wallet, &loaded)?;
        Ok(())
    }
}

/// Before a Bitcoin Core login is added: is the node reachable with its RPC
/// server on, and is the wallet there? Returns the wallet that will be read.
/// Empty `datadir` / `rpc_url` mean the defaults, an empty `wallet` the only loaded one.
pub async fn check_bitcoin_core(
    datadir: String,
    rpc_url: String,
    wallet: String,
) -> Result<String, CoreError> {
    let mut c = BitcoinCoreConnector::new();
    c.connect(&datadir, &rpc_url, "", &wallet).await?;
    Ok(c.wallet)
}

#[async_trait]
impl BankConnector for BitcoinCoreConnector {
    async fn begin(&mut self, ctx: &ConnectContext) -> CoreResult<Phase> {
        let setting = |k: &str| {
            ctx.settings
                .get(k)
                .map(|s| s.expose().to_string())
                .unwrap_or_default()
        };
        self.connect(
            &setting(DATADIR_KEY),
            &setting(RPC_URL_KEY),
            ctx.secret.expose(),
            &ctx.login_name,
        )
        .await?;
        Ok(Phase::Ready)
    }

    async fn respond(
        &mut self,
        _ctx: &ConnectContext,
        _response: ChallengeResponse,
    ) -> CoreResult<Phase> {
        Ok(Phase::Ready)
    }

    async fn fetch(&mut self, _ctx: &ConnectContext) -> CoreResult<FetchResult> {
        let sats = wallet_sats(&self.rpc(Some(&self.wallet), "getbalances").await?)?;
        let resp = super::http_client()?
            .get(PRICE_URL)
            .send()
            .await
            .map_err(|_| price_unavailable())?;
        let ticker = super::read_json_or_null(resp).await;
        let price = price_cents(&ticker)?;
        Ok(fetch_result(&self.wallet, sats, price, &super::today()))
    }
}

fn price_unavailable() -> CoreError {
    CoreError::Network {
        message: tr("Bitcoin-Kurs nicht abrufbar. Der Bestand wurde nicht aktualisiert.").into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_local_rpc_addresses_are_accepted() {
        assert_eq!(
            local_rpc_url("http://127.0.0.1:8332/").unwrap(),
            "http://127.0.0.1:8332"
        );
        assert!(local_rpc_url("http://localhost:8332").is_ok());
        assert!(local_rpc_url("http://[::1]:8332").is_ok());
        for bad in [
            "http://192.168.1.5:8332",
            "http://node.example.org:8332",
            "ftp://127.0.0.1",
            "kein url",
        ] {
            assert!(local_rpc_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn credentials_come_from_cookie_or_field() {
        assert_eq!(
            parse_credentials("__cookie__:abc123\n"),
            Some(("__cookie__".into(), "abc123".into()))
        );
        assert_eq!(parse_credentials(""), None);
        assert_eq!(parse_credentials("nur-ein-teil"), None);
        assert_eq!(parse_credentials(":x"), None);
    }

    #[test]
    fn the_only_loaded_wallet_is_taken_unless_one_is_named() {
        let one = vec!["haushalt".to_string()];
        assert_eq!(pick_wallet("", &one).unwrap(), "haushalt");
        // a named wallet must be one the node has loaded
        let two = vec!["haushalt".to_string(), "spar".to_string()];
        assert_eq!(pick_wallet(" spar ", &two).unwrap(), "spar");
        assert!(pick_wallet("spar", &one).is_err());
        assert!(pick_wallet("", &[]).is_err());
        assert!(pick_wallet("", &["a".into(), "b".into()]).is_err());
    }

    #[test]
    fn wallet_balance_counts_pending_and_immature() {
        let v =
            json!({"mine": {"trusted": 0.5, "untrusted_pending": 0.0012, "immature": 0.00000001}});
        assert_eq!(wallet_sats(&v).unwrap(), 50_120_001);
        assert!(wallet_sats(&json!({})).is_err());
        // amounts the node must report are never read as zero
        assert!(wallet_sats(&json!({"mine": {}})).is_err());
        assert!(wallet_sats(&json!({"mine": null})).is_err());
        assert!(wallet_sats(&json!({"mine": {"trusted": 0.5, "immature": 0.0}})).is_err());
        let zero = json!({"mine": {"trusted": 0.0, "untrusted_pending": 0.0, "immature": 0.0}});
        assert_eq!(wallet_sats(&zero).unwrap(), 0);
    }

    /// A watch-only wallet (e.g. a hardware wallet's keys imported): its funds count too.
    #[test]
    fn watch_only_funds_count() {
        let v = json!({
            "mine": {"trusted": 0.0, "untrusted_pending": 0.0, "immature": 0.0},
            "watchonly": {"trusted": 0.25, "untrusted_pending": 0.0, "immature": 0.0}
        });
        assert_eq!(wallet_sats(&v).unwrap(), 25_000_000);
        let broken = json!({
            "mine": {"trusted": 0.0, "untrusted_pending": 0.0, "immature": 0.0},
            "watchonly": {"trusted": 0.25}
        });
        assert!(wallet_sats(&broken).is_err());
    }

    #[test]
    fn ticker_price_in_cents() {
        let v = json!({"error": [], "result": {"XXBTZEUR": {"c": ["81234.56789", "0.01"]}}});
        assert_eq!(price_cents(&v).unwrap(), 8_123_457);
        assert!(price_cents(&json!({"error": ["EQuery:Unknown asset pair"]})).is_err());
        assert!(price_cents(&json!({"error": [], "result": {}})).is_err());
    }

    #[test]
    fn value_is_quantity_times_price() {
        // 0.5 BTC at 80,000.00 € = 40,000.00 €
        assert_eq!(value_cents(50_000_000, 8_000_000), 4_000_000);
        // one satoshi rounds to the cent
        assert_eq!(value_cents(1, 8_000_000), 0);
    }

    /// No node set up: the check before adding refuses with the folder it looked in.
    #[tokio::test]
    async fn check_without_cookie_names_the_folder() {
        let dir = crate::test_support::TempDir::new("girocrab-btc-nocookie");
        let err = check_bitcoin_core(
            dir.path().display().to_string(),
            String::new(),
            String::new(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CoreError::Authentication { .. }), "{err:?}");
        assert!(err.to_string().contains(".cookie"), "{err}");
    }

    /// A remote RPC address is refused before the cookie is even read.
    #[tokio::test]
    async fn check_refuses_a_remote_node() {
        let err = check_bitcoin_core(
            String::new(),
            "http://192.168.1.5:8332".into(),
            String::new(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CoreError::InvalidInput { .. }), "{err:?}");
    }

    #[test]
    fn wallet_becomes_a_depot_with_one_direct_crypto_position() {
        let r = fetch_result("haushalt", 50_000_000, 8_000_000, "2026-11-21");
        assert_eq!(r.accounts.len(), 1);
        let a = &r.accounts[0];
        assert_eq!(
            (a.kind, a.balance_cents),
            (AccountKind::Brokerage, 4_000_000)
        );
        assert_eq!(a.external_ref, "bitcoin-core:haushalt");
        let h = &r.holdings[0];
        assert!(h.direct_crypto);
        assert_eq!((h.holding.wkn.as_str(), h.holding.quantity), ("BTC", 0.5));
        assert_eq!(
            r.holdings_fetched,
            vec!["bitcoin-core:haushalt".to_string()]
        );
        // an empty wallet: no position, but a complete (empty) inventory
        let empty = fetch_result("haushalt", 0, 8_000_000, "2026-11-21");
        assert!(empty.holdings.is_empty());
        assert_eq!(empty.holdings_fetched.len(), 1);
    }
}
