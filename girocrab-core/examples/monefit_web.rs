//! Local test of the Monefit SmartSaver connector, without the
//! app and without a vault. Prints the balance and the bookings to stdout
//! (your own data, nothing is stored or sent anywhere but to the provider).
//!
//! ```sh
//! IFS= read -r -s SAVINGS_PASSWORD
//! export SAVINGS_PASSWORD
//! CONNECTOR=monefit SAVINGS_LOGIN='erika@example.org' SINCE=2026-01-01 \
//!   cargo run -q -p girocrab-core --example monefit_web
//! ```
//!
//! Without `SINCE` the last 30 days are fetched.
use girocrab_core::connectors::monefit::MonefitConnector;
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
    let mut c: Box<dyn BankConnector> = match env("CONNECTOR").as_str() {
        "monefit" => Box::new(MonefitConnector::new()),
        other => {
            eprintln!("CONNECTOR={other}: erwartet monefit");
            std::process::exit(2)
        }
    };
    let since = std::env::var("SINCE").unwrap_or_else(|_| {
        let today = jiff::Timestamp::now()
            .to_zoned_datetime(jiff::tz::TimeZone::UTC)
            .date();
        (today - jiff::Span::new().days(30)).to_string()
    });
    let ctx = ConnectContext {
        connection_id: 0,
        login_name: env("SAVINGS_LOGIN"),
        secret: SecretString::new(env("SAVINGS_PASSWORD")),
        since,
        ..Default::default()
    };
    match c.begin(&ctx).await {
        Ok(Phase::Ready) => eprintln!("Anmeldung ok (seit {})", ctx.since),
        Ok(_) => {
            eprintln!("Unerwartete Rückfrage des Anbieters");
            std::process::exit(1)
        }
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
    c.finish(&ctx).await;
    for a in &r.accounts {
        println!(
            "Konto {} „{}“ Nr. {:?} {:?}: Saldo {}",
            a.external_ref,
            a.name,
            a.account_number,
            a.kind,
            format_cents(a.balance_cents, a.currency.clone())
        );
    }
    for t in &r.transactions {
        println!(
            "{}  {:>12}  {} | {} | {} | {}",
            t.booking_date,
            format_cents(t.amount_cents, t.currency.clone()),
            t.counterparty,
            t.purpose,
            t.booking_text,
            t.external_id
        );
    }
    eprintln!("{} Buchungen", r.transactions.len());
}
