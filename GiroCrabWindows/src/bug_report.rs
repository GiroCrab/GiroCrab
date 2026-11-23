//! Diagnostics text for „Problem melden“ (#384, part 1), after the Mac
//! `BugReport.swift` and the Linux `bug_report.rs`: a header (version, system,
//! active view, connectors) plus the newest sync log lines. Only the log
//! messages carry user data; they pass `sanitize_report_text` with the vault's
//! known terms. The text is only returned, never sent anywhere.

use std::collections::{HashMap, HashSet};

use girocrab_core::report_sanitizer::sanitize_report_text;
use girocrab_core::sync_log::{SyncLogEntry, SyncLogFilter};
use girocrab_core::{BankDescriptor, ConnectorKind, Vault};

use crate::nav::Target;

#[derive(Debug, Clone, PartialEq)]
pub struct Info {
    pub app_version: String,
    pub system: String,
    pub architecture: String,
    pub language: String,
    pub view: &'static str,
    pub demo: bool,
    pub connectors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LogLine {
    pub head: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub text: String,
    /// The log was requested but could not be loaded together with the terms.
    pub log_unavailable: bool,
}

/// Fixed label per sidebar target: never an account or category name or id.
pub fn view_label(target: Target) -> &'static str {
    match target {
        Target::AllAccounts => "Alle Konten",
        Target::Account(_) => "Konto",
        Target::NewBookings => "Neue Buchungen",
        Target::Wealth => "Vermögen",
        Target::Depot => "Depot",
        Target::Planned => "Geplant",
        Target::Subscriptions => "Verträge & Abos",
        Target::Mandates => "Mandate",
        Target::Documents => "Dokumente",
        Target::Uncategorized => "Ohne Kategorie",
        Target::Category(_) => "Kategorie",
    }
}

/// The log lines with sanitized messages. The head (date, connector, source,
/// outcome) is left alone: an account named like its bank must not blank out
/// the connector column.
pub fn sanitized_log(lines: &[LogLine], known_terms: &[String]) -> Vec<String> {
    if lines.is_empty() {
        return Vec::new();
    }
    // One call for all messages (one regex build). The closing "." keeps a
    // trailing empty message from being dropped by the line split.
    let joined = lines
        .iter()
        .map(|l| l.message.as_str())
        .chain(std::iter::once("."))
        .collect::<Vec<_>>()
        .join("\n");
    let sanitized = sanitize_report_text(joined, known_terms.to_vec());
    let mut messages: Vec<&str> = sanitized.split('\n').collect();
    messages.pop();
    if messages.len() != lines.len() {
        // never expected; fall back to one call per message
        return lines
            .iter()
            .map(|l| {
                line(
                    &l.head,
                    &sanitize_report_text(l.message.clone(), known_terms.to_vec()),
                )
            })
            .collect();
    }
    lines
        .iter()
        .zip(messages)
        .map(|(l, m)| line(&l.head, m))
        .collect()
}

fn line(head: &str, message: &str) -> String {
    if message.is_empty() {
        head.to_string()
    } else {
        format!("{head} · {message}")
    }
}

/// The diagnostics block (header plus already sanitized log lines).
pub fn render(info: &Info, log: &[String]) -> String {
    let connectors = if info.connectors.is_empty() {
        "–".to_string()
    } else {
        info.connectors.join(", ")
    };
    let mut lines = vec![
        format!("GiroCrab {}", info.app_version),
        format!("{} · {}", info.system, info.architecture),
        format!(
            "Sprache: {} · Demo-Modus: {}",
            info.language,
            if info.demo { "ja" } else { "nein" }
        ),
        format!("Ansicht: {}", info.view),
        format!("Anbindungen: {connectors}"),
    ];
    if !log.is_empty() {
        lines.push(String::new());
        lines.push("Abruf-Protokoll (neueste zuerst):".to_string());
        lines.extend(log.iter().cloned());
    }
    lines.join("\n")
}

fn normalized(term: &str) -> String {
    term.trim().to_lowercase()
}

/// Trimmed, de-duplicated terms without bank/connector names (not personal).
fn public_terms_removed(terms: Vec<String>, banks: &HashSet<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    terms
        .into_iter()
        .map(|t| t.trim().to_string())
        .filter(|t| {
            let key = normalized(t);
            !key.is_empty() && !banks.contains(&key) && seen.insert(key)
        })
        .collect()
}

/// Everything personal the vault knows that might turn up in a log line: own
/// accounts, owners, IBANs and logins, and the counterparties of all bookings.
pub fn known_terms(
    v: &Vault,
    counterparties: &[String],
    catalog: &[BankDescriptor],
) -> Result<Vec<String>, String> {
    let accounts = v.list_accounts().map_err(|e| e.to_string())?;
    let connections = v.list_bank_connections().map_err(|e| e.to_string())?;
    let mut terms: Vec<String> = Vec::new();
    for a in &accounts {
        terms.extend([
            a.name.clone(),
            a.owner.clone(),
            a.iban.clone(),
            a.iban.replace(' ', ""),
            a.account_number.clone(),
        ]);
    }
    for c in &connections {
        terms.extend([c.display_name.clone(), c.login_name.clone()]);
    }
    terms.extend(counterparties.iter().cloned());
    let banks: HashSet<String> = catalog
        .iter()
        .map(|d| normalized(&d.name))
        .chain(accounts.iter().map(|a| normalized(&a.bank_name)))
        .collect();
    Ok(public_terms_removed(terms, &banks))
}

/// `Debug` case name in lower camel case (`ManualSync` → `manualSync`).
fn case_name<T: std::fmt::Debug>(v: &T) -> String {
    let name = format!("{v:?}");
    let mut chars = name.chars();
    chars
        .next()
        .map(|c| c.to_lowercase().chain(chars).collect())
        .unwrap_or_default()
}

fn connector_name(kind: &ConnectorKind, catalog: &[BankDescriptor]) -> String {
    match catalog.iter().find(|d| &d.kind == kind) {
        None => case_name(kind),
        Some(d) if d.protocol.is_empty() || d.protocol == d.name => d.name.clone(),
        Some(d) => format!("{} ({})", d.name, d.protocol),
    }
}

/// `YYYY-MM-DD HH:MM UTC` without a date crate (civil-from-days).
fn utc_minute(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let minutes = ms.rem_euclid(86_400_000) / 60_000;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        minutes / 60,
        minutes % 60
    )
}

fn log_lines(
    entries: &[SyncLogEntry],
    connectors: &HashMap<i64, ConnectorKind>,
    catalog: &[BankDescriptor],
) -> Vec<LogLine> {
    entries
        .iter()
        .map(|e| {
            // The connection's own name is the user's; the connector kind is enough.
            let kind = e
                .connection_id
                .and_then(|id| connectors.get(&id))
                .map(|k| connector_name(k, catalog))
                .unwrap_or_else(|| "–".to_string());
            let mut head = format!(
                "{} · {kind} · {} · {}",
                utc_minute(e.timestamp_ms),
                case_name(&e.source),
                case_name(&e.outcome)
            );
            if e.imported > 0 {
                head += &format!(" · +{}", e.imported);
            }
            LogLine {
                head,
                message: e.message.replace('\n', " "),
            }
        })
        .collect()
}

fn app_version() -> String {
    let core = girocrab_core::core_version();
    let app = crate::VERSION;
    if app.is_empty() || app == core {
        core
    } else {
        format!("{core} (App {app})")
    }
}

fn system_description() -> String {
    match std::env::consts::OS {
        "windows" => "Windows",
        "macos" => "macOS",
        "linux" => "Linux",
        other => other,
    }
    .to_string()
}

fn language() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .map(|v| v.split(['.', '@']).next().unwrap_or_default().to_string())
        .find(|v| !v.is_empty() && v != "C" && v != "POSIX")
        .unwrap_or_else(|| "de".to_string())
}

/// Header and sanitized log lines. Reads the vault (up to 5000 payee names):
/// call it on the worker thread, never on the UI thread. Log entries and payee
/// names load together, or the log is left out: without the names a log line
/// could carry a payee into the issue. A locked vault (`None`) gives the header only.
pub fn prepare(v: Option<&Vault>, view: Target, include_log: bool, demo: bool) -> Report {
    let catalog = girocrab_core::bank_catalog();
    let connections = v
        .and_then(|v| v.list_bank_connections().ok())
        .unwrap_or_default();
    let by_connection: HashMap<i64, ConnectorKind> =
        connections.iter().map(|c| (c.id, c.connector)).collect();
    let mut connectors: Vec<String> = connections
        .iter()
        .map(|c| connector_name(&c.connector, &catalog))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    connectors.sort();
    let info = Info {
        app_version: app_version(),
        system: system_description(),
        architecture: std::env::consts::ARCH.to_string(),
        language: language(),
        view: view_label(view),
        demo,
        connectors,
    };
    let mut log = Vec::new();
    let mut log_unavailable = false;
    if include_log && let Some(v) = v {
        let loaded = (|| {
            let filter = SyncLogFilter {
                connection_id: None,
                only_errors: false,
                search: None,
                limit: 50,
            };
            let entries = v.list_sync_log(filter).map_err(|e| e.to_string())?;
            let names = v.counterparty_names(5000).map_err(|e| e.to_string())?;
            let terms = known_terms(v, &names, &catalog)?;
            Ok::<_, String>((entries, terms))
        })();
        match loaded {
            Ok((entries, terms)) => {
                log = sanitized_log(&log_lines(&entries, &by_connection, &catalog), &terms)
            }
            Err(_) => log_unavailable = true,
        }
    }
    Report {
        text: render(&info, &log),
        log_unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(view: Target) -> Info {
        Info {
            app_version: "0.9.0".into(),
            system: "Windows".into(),
            architecture: "x86_64".into(),
            language: "de".into(),
            view: view_label(view),
            demo: true,
            connectors: vec!["Musterbank (FinTS)".into()],
        }
    }

    /// Same path as the demo start in `main.rs` (`GIROCRAB_DEMO_SEED=1`).
    fn demo_vault(path: &std::path::Path) -> std::sync::Arc<Vault> {
        let _ = std::fs::remove_file(path);
        let v = Vault::open(path.display().to_string(), vec![7; 32]).expect("vault");
        v.seed_demo_data().expect("demo data");
        v
    }

    #[test]
    fn demo_vault_names_ibans_and_amounts_leave_the_log() {
        let path = std::env::temp_dir().join(format!("gc-bugreport-{}.db", std::process::id()));
        let v = demo_vault(&path);
        let names = v.counterparty_names(5000).expect("names");
        let terms = known_terms(&v, &names, &girocrab_core::bank_catalog()).expect("terms");
        let account = v
            .list_accounts()
            .expect("accounts")
            .into_iter()
            .find(|a| !a.iban.is_empty() && a.name.trim().chars().count() >= 3)
            .expect("demo account with IBAN");
        let lines = [LogLine {
            head: "2026-11-20 08:15 UTC · Musterbank (FinTS) · manual · failed".into(),
            message: format!(
                "Überweisung von {} ({}) über 1.234,56 EUR abgelehnt: PIN gesperrt",
                account.name, account.iban
            ),
        }];
        let text = render(&info(Target::AllAccounts), &sanitized_log(&lines, &terms));
        drop(v);
        let _ = std::fs::remove_file(&path);
        assert!(!text.contains(account.name.trim()), "{text}");
        assert!(!text.contains(&account.iban), "{text}");
        assert!(!text.contains(&account.iban.replace(' ', "")), "{text}");
        assert!(!text.contains("1.234,56"), "{text}");
        assert!(text.contains("Musterbank (FinTS)"), "{text}");
        assert!(text.contains("PIN gesperrt"), "{text}");
    }

    #[test]
    fn bank_names_are_not_redacted_terms() {
        let banks: HashSet<String> = ["paypal".to_string()].into();
        let terms = vec![
            "PayPal".to_string(),
            " Erika Muster ".to_string(),
            "erika muster".to_string(),
            "  ".to_string(),
        ];
        assert_eq!(public_terms_removed(terms, &banks), vec!["Erika Muster"]);
        let log = sanitized_log(
            &[LogLine {
                head: "h".into(),
                message: "PayPal: Erika Muster nicht erreichbar".into(),
            }],
            &["Erika Muster".to_string()],
        );
        assert!(log[0].contains("PayPal"));
        assert!(log[0].contains("nicht erreichbar"));
        assert!(!log[0].contains("Erika"));
    }

    #[test]
    fn header_names_the_view_but_never_an_id() {
        let text = render(&info(Target::Account(4711)), &[]);
        assert!(text.contains("Ansicht: Konto"));
        assert!(text.contains("Demo-Modus: ja"));
        assert!(!text.contains("4711"));
        assert!(!text.contains("Abruf-Protokoll"));
        assert_eq!(view_label(Target::Category(9)), "Kategorie");
    }

    #[test]
    fn locked_vault_gives_header_only() {
        let report = prepare(None, Target::Depot, true, false);
        assert!(!report.log_unavailable);
        assert!(report.text.contains("Ansicht: Depot"));
        assert!(report.text.contains("Anbindungen: –"));
        assert!(!report.text.contains("Abruf-Protokoll"));
    }

    #[test]
    fn utc_minute_handles_epoch_and_leap_day() {
        assert_eq!(utc_minute(0), "1970-01-01 00:00 UTC");
        assert_eq!(utc_minute(1_709_164_800_000), "2024-02-29 00:00 UTC");
        assert_eq!(utc_minute(1_709_251_199_000), "2024-02-29 23:59 UTC");
    }
}
