//! Local test of the PayPal connector via API signature (classic NVP API, #156),
//! without the app and without a vault. Prints balances and bookings to stdout
//! (your own data, nothing is stored or sent anywhere but to PayPal).
//!
//! ```sh
//! read -s PAYPAL_API_PASSWORD; read -s PAYPAL_API_SIGNATURE; export PAYPAL_API_PASSWORD PAYPAL_API_SIGNATURE
//! PAYPAL_API_USERNAME=… [PAYPAL_SANDBOX=1] [SINCE=2026-01-01] \
//!   cargo run -q -p girocrab-core --example paypal_nvp
//! ```
use std::collections::HashMap;

use girocrab_core::connectors::paypal::PayPalConnector;
use girocrab_core::connectors::{BankConnector, ConnectContext, Phase};
use girocrab_core::money::format_cents;
use girocrab_core::secret::SecretString;

fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| {
        eprintln!("{key} fehlt");
        std::process::exit(2)
    })
}

#[tokio::main]
async fn main() {
    let mut settings = HashMap::new();
    for (k, e) in [
        ("paypal.api_username", "PAYPAL_API_USERNAME"),
        ("paypal.api_password", "PAYPAL_API_PASSWORD"),
        ("paypal.api_signature", "PAYPAL_API_SIGNATURE"),
    ] {
        settings.insert(k.to_string(), SecretString::new(env(e)));
    }
    if std::env::var("PAYPAL_SANDBOX").is_ok_and(|v| v == "1") {
        settings.insert(
            "paypal.environment".into(),
            SecretString::new("sandbox".into()),
        );
    }
    let since = std::env::var("SINCE").unwrap_or_else(|_| {
        (chrono::Utc::now().date_naive() - chrono::Duration::days(90)).to_string()
    });
    let ctx = ConnectContext {
        connection_id: 0,
        login_name: "PayPal".into(),
        secret: SecretString::default(),
        settings,
        state: serde_json::Value::Null,
        since,
        since_by_account: HashMap::new(),
    };
    let mut c = PayPalConnector::new();
    match c.begin(&ctx).await {
        Ok(Phase::Ready) => eprintln!("Anmeldung ok (seit {})", ctx.since),
        Ok(_) => unreachable!("NVP has no challenge"),
        Err(e) => {
            eprintln!("Anmeldung fehlgeschlagen: {e}");
            std::process::exit(1)
        }
    }
    let r = match c.fetch(&ctx).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Abruf fehlgeschlagen: {e}");
            std::process::exit(1)
        }
    };
    for a in &r.accounts {
        println!(
            "{}  {}  {}",
            a.external_ref,
            a.name,
            format_cents(a.balance_cents, a.currency.clone())
        );
    }
    println!("--- {} Umsätze", r.transactions.len());
    for t in &r.transactions {
        println!(
            "{}  {:>12}  {:<20}  {}  {}  [{}]",
            t.booking_date,
            format_cents(t.amount_cents, t.currency.clone()),
            t.booking_text,
            t.counterparty,
            t.purpose,
            t.external_id
        );
    }
}
