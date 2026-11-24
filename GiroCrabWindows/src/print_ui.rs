//! Glue für Strg+P: sichtbare Buchungen der Liste übernehmen, HTML schreiben, Browser öffnen.
//! Datei schreiben und Browserstart laufen auf einem Worker-Thread, nie auf dem UI-Thread.

use crate::daily;
use crate::print::{self, Paper, Report, Row};

/// Die Zeilen der Liste (Filter, Suche und Richtung schon angewendet) als Druckzeilen.
pub fn rows_of(shown: &[&daily::Row]) -> Vec<Row> {
    shown
        .iter()
        .map(|r| Row {
            date: r.booking_date.clone(),
            counterparty: r.counterparty.clone(),
            purpose: r.purpose.clone(),
            amount_cents: r.amount_cents,
            currency: r.currency.clone(),
        })
        .collect()
}

/// Kontoname im Kopf: das eine Konto der Liste, sonst „Alle Konten“.
pub fn account_label(shown: &[&daily::Row]) -> String {
    let mut names = shown.iter().map(|r| r.account_name.as_str());
    match names.next() {
        Some(first) if names.all(|n| n == first) => first.to_string(),
        _ => "Alle Konten".to_string(),
    }
}

/// `privacy` ist der Wert zum Klickzeitpunkt (Privatmodus = Ausdruck verdeckt).
pub fn start(
    weak: slint::Weak<crate::MainWindow>,
    rows: Vec<Row>,
    account: String,
    from_date: String,
    privacy: bool,
) {
    std::thread::spawn(move || {
        let period = print::period_label(Some(from_date.as_str()).filter(|d| !d.is_empty()), None);
        let locale = std::env::var("LC_ALL")
            .or_else(|_| std::env::var("LANG"))
            .unwrap_or_default();
        let html = print::render_html(&Report {
            account,
            period,
            rows,
            privacy,
            paper: Paper::from_locale(&locale),
        });
        let result = print::write_temp(&html)
            .and_then(|path| print::open_in_browser(&path))
            .map_err(|e| e.to_string());
        let msg = match result {
            Ok(()) => "Druckansicht im Browser geöffnet.".to_string(),
            Err(e) => format!("Drucken fehlgeschlagen: {e}"),
        };
        let _ = weak.upgrade_in_event_loop(move |win| win.set_sync_status(msg.into()));
    });
}
