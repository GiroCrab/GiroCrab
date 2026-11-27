//! Protokoll des Windows-Clients, in der Form, die der macOS-Client mit `os.Logger` benutzt:
//! **Kategorie** je Bereich, **Stufe** je Zeile, Werte standardmäßig maskiert, und die Zeilen
//! verlassen das Programm (Konsole und Datei).
//!
//! macOS schreibt über `Logger(subsystem: "net.24unix.girocrab", category: "enablebanking-key")`
//! in das einheitliche Systemprotokoll und gibt nur einzeln geprüfte Werte mit `privacy: .public`
//! frei. Windows hat kein solches Sammelprotokoll; der nächste Weg ist die eigene Zeile auf stderr
//! und in `girocrab.log` neben dem Tresor — mit denselben Regeln: Kategorie, Stufe, maskiert.
//!
//! Stufenfilter: `GIROCRAB_LOG=error|warn|info|debug` (Standard `info`).
//!
//! Der Client ist ein Konsolen-Programm (`PE Subsystem 3`), deshalb fängt
//! `girocrab.exe 2>&1 | Tee-Object -FilePath …` jede Zeile mit. Ein Start über eine Verknüpfung
//! zeigt kein stderr — dafür ist die Datei da. Ein Panic in einem Arbeitsfaden hätte sonst gar
//! keine Spur: der Dialog bleibt auf „Bank wird eingerichtet …“ stehen.

use std::io::Write;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

use girocrab_core::report_sanitizer::sanitize_report_text;

static LOG: OnceLock<Log> = OnceLock::new();
/// Was maskiert wird, sobald der Tresor offen ist (Kontonamen, Inhaber, IBANs, Kennungen).
static TERMS: RwLock<Vec<String>> = RwLock::new(Vec::new());
static THRESHOLD: OnceLock<Level> = OnceLock::new();

/// Stufe einer Zeile, aufsteigend gesprochen.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Level {
    Error,
    Warn,
    Info,
    Debug,
}

impl Level {
    fn name(self) -> &'static str {
        match self {
            Level::Error => "ERROR",
            Level::Warn => "WARN",
            Level::Info => "INFO",
            Level::Debug => "DEBUG",
        }
    }

    fn from_name(name: &str) -> Option<Level> {
        match name.trim().to_ascii_lowercase().as_str() {
            "error" => Some(Level::Error),
            "warn" | "warning" => Some(Level::Warn),
            "info" => Some(Level::Info),
            "debug" => Some(Level::Debug),
            _ => None,
        }
    }
}

/// Schreibt Zeilen mit UTC-Zeitstempel an eine feste Datei.
pub struct Log {
    path: PathBuf,
}

impl Log {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        Self { path }
    }

    /// Hängt eine fertige Zeile an. Schreibfehler werden geschluckt: das Protokoll darf nie der
    /// Grund sein, dass die App stehen bleibt.
    pub fn write_line(&self, line: &str) {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = writeln!(file, "{line}");
        }
    }

    /// Panics landen in der Datei statt nur auf einem stderr, das ein Fensterstart nicht zeigt.
    pub fn install_panic_hook(&self) {
        let path = self.path.clone();
        std::panic::set_hook(Box::new(move |info| {
            let text = panic_line(info);
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
            {
                let _ = writeln!(file, "{text}");
            }
            // Zusätzlich auf stderr, falls jemand mit Konsole gestartet hat.
            eprintln!("{text}");
        }));
    }
}

/// Richtet das Protokoll einmalig ein (erster Aufruf gewinnt) und setzt den Panic-Hook.
pub fn init(path: impl Into<PathBuf>) -> &'static Log {
    LOG.get_or_init(|| {
        let log = Log::new(path);
        log.install_panic_hook();
        log
    })
}

/// Setzt die Begriffe, die auf Konsole und in der Datei maskiert werden. Aufruf nach dem
/// Entsperren; vorher wird unverändert geschrieben, weil es dann nichts zu verbergen gibt.
pub fn set_known_terms(terms: Vec<String>) {
    if let Ok(mut current) = TERMS.write() {
        *current = terms;
    }
}

pub fn has_known_terms() -> bool {
    TERMS.read().map(|t| !t.is_empty()).unwrap_or(false)
}

/// Schwelle aus `GIROCRAB_LOG`, sonst `info`.
fn threshold() -> Level {
    *THRESHOLD.get_or_init(|| {
        std::env::var("GIROCRAB_LOG")
            .ok()
            .as_deref()
            .and_then(Level::from_name)
            .unwrap_or(Level::Info)
    })
}

pub fn error(category: &str, message: &str) {
    emit(Level::Error, category, message);
}

pub fn warn(category: &str, message: &str) {
    emit(Level::Warn, category, message);
}

pub fn info(category: &str, message: &str) {
    emit(Level::Info, category, message);
}

pub fn debug(category: &str, message: &str) {
    emit(Level::Debug, category, message);
}

/// Kurzform für Zeilen ohne eigenen Bereich.
pub fn note(message: &str) {
    emit(Level::Info, "app", message);
}

fn emit(level: Level, category: &str, message: &str) {
    if level > threshold() {
        return;
    }
    let terms = TERMS.read().map(|t| t.clone()).unwrap_or_default();
    let text = format_line(level, category, message, &terms);
    // Konsole zuerst: sie ist der Weg, der auch ohne Datei sofort etwas zeigt.
    eprintln!("{text}");
    if let Some(log) = LOG.get() {
        log.write_line(&text);
    }
}

/// Eine Zeile im Zuschnitt des einheitlichen Protokolls: Zeit, Stufe, Kategorie, Meldung —
/// die Meldung vorher bereinigt.
fn format_line(level: Level, category: &str, message: &str, terms: &[String]) -> String {
    let message = sanitized(message, terms);
    format!("{} {:<5} {}: {message}", stamp(), level.name(), category)
}

/// Bereinigt eine Zeile mit den bekannten Begriffen. Ohne Begriffe bleibt sie unverändert.
fn sanitized(line: &str, terms: &[String]) -> String {
    if terms.is_empty() {
        return line.to_string();
    }
    sanitize_report_text(line.to_string(), terms.to_vec())
}

/// Zeile für einen Panic. Die Meldung wird gekürzt, weil sie mitunter Werte aus dem Aufruf trägt.
fn panic_line(info: &std::panic::PanicHookInfo<'_>) -> String {
    let location = info
        .location()
        .map(|l| format!("{}:{}", l.file(), l.line()))
        .unwrap_or_else(|| "unbekannt".to_string());
    let message = info
        .payload()
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_default();
    let message: String = message.chars().take(300).collect();
    let backtrace = std::backtrace::Backtrace::force_capture();
    format!(
        "{} PANIC  panic: {location}: {message}\n{backtrace}",
        stamp()
    )
}

/// `JJJJ-MM-TT hh:mm:ss.mmm` aus der Systemzeit, ohne zusätzliche Abhängigkeit.
fn stamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let (days, rest) = (now.as_secs() / 86_400, now.as_secs() % 86_400);
    let (year, month, day) = civil_from_days(days as i64);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}.{:03}Z",
        rest / 3_600,
        (rest / 60) % 60,
        rest % 60,
        now.subsec_millis()
    )
}

/// Tage seit 1970-01-01 → (Jahr, Monat, Tag). Nach Hinnant, `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_date_conversion_matches_known_days() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(20_000), (2024, 10, 4));
    }

    #[test]
    fn a_line_carries_time_level_and_category() {
        let line = format_line(Level::Info, "bank-add", "choice=0 demo", &[]);
        assert!(line.contains(" INFO  bank-add: choice=0 demo"), "{line}");
        assert!(line.starts_with("20") && line.contains('Z'), "{line}");
    }

    #[test]
    fn known_terms_are_masked_before_the_line_leaves_the_process() {
        let line = "bank_add: Konto Girokonto, IBAN DE02120300000000202051, Login 1234";
        let clean = format_line(
            Level::Info,
            "bank-add",
            line,
            &[
                "Girokonto".into(),
                "DE02120300000000202051".into(),
                "1234".into(),
            ],
        );
        assert!(!clean.contains("Girokonto"), "unerwartet: {clean}");
        assert!(
            !clean.contains("DE02120300000000202051"),
            "unerwartet: {clean}"
        );
        assert!(clean.contains("bank_add"), "Kontext muss bleiben: {clean}");
    }

    #[test]
    fn without_known_terms_a_line_passes_unchanged() {
        assert_eq!(
            format_line(Level::Info, "bank-add", "choice=0 demo", &[]),
            format!("{} INFO  bank-add: choice=0 demo", stamp())
        );
    }

    #[test]
    fn levels_order_from_error_to_debug_and_read_from_the_environment() {
        assert!(
            Level::Error < Level::Warn && Level::Warn < Level::Info && Level::Info < Level::Debug
        );
        assert_eq!(Level::from_name("debug"), Some(Level::Debug));
        assert_eq!(Level::from_name(" WARN "), Some(Level::Warn));
        assert_eq!(Level::from_name("laut"), None);
        // Standard ist info: debug fällt raus, error bleibt.
        assert!(Level::Debug > Level::Info && Level::Error <= Level::Info);
    }

    #[test]
    fn a_line_lands_in_the_file_with_a_timestamp() {
        let path = std::env::temp_dir().join(format!(
            "girocrab-diag-{}-{}.log",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let log = Log::new(&path);
        log.write_line(&format_line(Level::Info, "bank-add", "worker start", &[]));
        let text = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(
            text.contains("bank-add: worker start"),
            "unerwartet: {text}"
        );
        assert!(
            text.starts_with("20") && text.contains(" INFO "),
            "Zeitstempel oder Stufe fehlt: {text}"
        );
    }
}
