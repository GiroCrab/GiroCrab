//! „Problem melden …“ unter Werkzeuge (#384, Teil 2). Mac-Vorbild:
//! `GiroCrabApp/Sources/GiroCrabUI/BugReport.swift` (`issueBody`, `issueURL`).
//! Beschreibung und Diagnose gehen erst nach einem Klick hinaus und werden direkt
//! davor erneut über `sanitize_report_text` bereinigt (die Vorschau ist editierbar).
//! Die Bildschirmaufnahme mit Schwärzung gehört nicht zu diesem Teil.

use std::fmt::Write as _;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use girocrab_core::report_sanitizer::sanitize_report_text;
use slint::ComponentHandle;

use crate::MainWindow;

const NEW_ISSUE: &str = "https://github.com/GiroCrab/GiroCrab/issues/new";
/// Obergrenze der ganzen Issue-URL in UTF-8-Bytes (wie `maxURLLength` auf dem Mac).
pub(crate) const MAX_URL_LENGTH: usize = 7500;
const PLACEHOLDER: &str = "<!-- Beschreibe kurz das Problem: Was hast du getan, was hast du \
erwartet, was ist stattdessen passiert? Bitte keine echten IBANs, Namen oder Buchungsdaten. -->";
const LOCKED: &str = "Der Tresor ist gesperrt; Kopieren und Öffnen sind nicht möglich.";
const PRIVACY: &str = "Im Datenschutzmodus sind Kopieren und Öffnen gesperrt.";

/// Verwirft eine Bereinigung, die nach Schließen oder Sperren eintrifft.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Was der Dialog aus dem entsperrten Tresor braucht; `None` beim Aufrufer = gesperrt.
pub(crate) struct ReportSource {
    pub(crate) known_terms: Vec<String>,
    /// Abrufprotokoll, neueste zuerst (wird für die URL von hinten gekürzt).
    pub(crate) log_lines: Vec<String>,
}

pub(crate) fn install(ui: &MainWindow, source: Rc<dyn Fn() -> Option<ReportSource>>) {
    let weak = ui.as_weak();
    let src = source.clone();
    ui.on_report_request(move || {
        let Some(ui) = weak.upgrade() else { return };
        reset(&ui);
        ui.set_report_open(true);
        let Some(source) = src() else {
            ui.set_report_status(LOCKED.into());
            return;
        };
        ui.set_report_busy(true);
        let generation = GENERATION.load(Ordering::SeqCst);
        let raw = diagnostics_text(&ui.get_app_version(), &source.log_lines);
        // Tausende Begriffe zu kompilieren dauert: nicht im UI-Thread.
        let weak = ui.as_weak();
        std::thread::spawn(move || {
            let text = sanitize_report_text(raw, source.known_terms);
            let _ = weak.upgrade_in_event_loop(move |ui| {
                if GENERATION.load(Ordering::SeqCst) == generation {
                    ui.set_report_preview(text.into());
                    ui.set_report_busy(false);
                }
            });
        });
    });

    let weak = ui.as_weak();
    let src = source.clone();
    ui.on_report_copy(move || {
        let Some(ui) = weak.upgrade() else { return };
        let Some((description, diagnostics)) = release(&ui, src.as_ref()) else {
            return;
        };
        let status = match crate::copy_text_to_clipboard(&issue_body(&description, &diagnostics)) {
            Ok(()) => "Bericht bereinigt in die Zwischenablage kopiert.".to_string(),
            Err(e) => e,
        };
        ui.set_report_status(status.into());
    });

    let weak = ui.as_weak();
    ui.on_report_open_issue(move || {
        let Some(ui) = weak.upgrade() else { return };
        let Some((description, diagnostics)) = release(&ui, source.as_ref()) else {
            return;
        };
        let (url, shortened) = issue_url(&description, &diagnostics);
        let status = match crate::sync_flow::open_in_browser(&url) {
            Ok(()) if shortened => "Issue im Browser geöffnet. Die Diagnose wurde für den Link \
                gekürzt; „In Zwischenablage kopieren“ liefert den vollständigen Text."
                .to_string(),
            Ok(()) => "Issue im Browser geöffnet.".to_string(),
            Err(e) => e,
        };
        ui.set_report_status(status.into());
    });
}

/// Leert den Dialog und verwirft laufende Bereinigungen; auch beim Sperren aufrufen.
pub(crate) fn reset(ui: &MainWindow) {
    GENERATION.fetch_add(1, Ordering::SeqCst);
    ui.set_report_open(false);
    ui.set_report_busy(false);
    ui.set_report_description("".into());
    ui.set_report_preview("".into());
    ui.set_report_status("".into());
}

/// Gibt Beschreibung und Vorschau nur ohne Datenschutzmodus und bei entsperrtem
/// Tresor frei, bereinigt beide erneut und zeigt, was tatsächlich hinausgeht.
fn release(ui: &MainWindow, source: &dyn Fn() -> Option<ReportSource>) -> Option<(String, String)> {
    if ui.get_privacy() {
        ui.set_report_status(PRIVACY.into());
        return None;
    }
    let Some(source) = source() else {
        ui.set_report_status(LOCKED.into());
        return None;
    };
    let (description, diagnostics) = sanitized_parts(
        &ui.get_report_description(),
        &ui.get_report_preview(),
        &source.known_terms,
    );
    ui.set_report_description(description.as_str().into());
    ui.set_report_preview(diagnostics.as_str().into());
    Some((description, diagnostics))
}

pub(crate) fn sanitized_parts(
    description: &str,
    diagnostics: &str,
    known_terms: &[String],
) -> (String, String) {
    (
        sanitize_report_text(description.trim().to_string(), known_terms.to_vec()),
        sanitize_report_text(diagnostics.trim_end().to_string(), known_terms.to_vec()),
    )
}

/// Rohtext der Diagnose (wird vor der Anzeige bereinigt).
pub(crate) fn diagnostics_text(version: &str, log_lines: &[String]) -> String {
    let mut out = vec![
        format!("GiroCrab {version} (Windows-App)"),
        format!(
            "System: {} {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
    ];
    if !log_lines.is_empty() {
        out.push(String::new());
        out.push("Abruf-Protokoll (neueste zuerst):".into());
        out.extend(log_lines.iter().cloned());
    }
    out.join("\n")
}

fn longest_backtick_run(s: &str) -> usize {
    let (mut longest, mut run) = (0, 0);
    for c in s.chars() {
        run = if c == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    longest
}

/// Markdown-Text des Issues wie `issueBody` auf dem Mac (ohne Screenshot-Abschnitt).
pub(crate) fn issue_body(description: &str, diagnostics: &str) -> String {
    let description = description.trim();
    // Länger als jede Backtick-Folge: "```" in einer Zeile darf den Block nicht beenden.
    let fence = "`".repeat(3.max(longest_backtick_run(diagnostics) + 1));
    [
        "### Was ist passiert?",
        "",
        if description.is_empty() {
            PLACEHOLDER
        } else {
            description
        },
        "",
        "### Diagnose (automatisch bereinigt)",
        "",
        &format!("{fence}text"),
        diagnostics.trim_end(),
        &fence,
    ]
    .join("\n")
}

/// Prozent-Kodierung für einen Query-Wert: alles außer `A-Za-z0-9-._~`,
/// also auch `+` (GitHub liest ein rohes `+` als Leerzeichen).
fn encode_query(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

fn url_for(body: &str) -> String {
    format!("{NEW_ISSUE}?title=%5BBUG%5D%20&body={}", encode_query(body))
}

/// Vorbefüllte `issues/new`-URL aus bereits bereinigtem Text. Kürzt die Diagnose
/// zeilenweise von hinten, notfalls die Beschreibung, bis `MAX_URL_LENGTH` passt.
/// Liefert auch, ob gekürzt wurde.
pub(crate) fn issue_url(description: &str, diagnostics: &str) -> (String, bool) {
    let mut lines: Vec<&str> = diagnostics.lines().collect();
    let mut shortened = false;
    loop {
        let url = url_for(&issue_body(description, &lines.join("\n")));
        if url.len() <= MAX_URL_LENGTH {
            return (url, shortened);
        }
        if lines.pop().is_none() {
            break;
        }
        shortened = true;
    }
    let mut chars: Vec<char> = description.trim().chars().collect();
    loop {
        chars.truncate(chars.len().saturating_sub((chars.len() / 10).max(1)));
        let url = url_for(&issue_body(&chars.iter().collect::<String>(), ""));
        if url.len() <= MAX_URL_LENGTH || chars.is_empty() {
            return (url, true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_puts_description_above_fenced_diagnostics() {
        let body = issue_body("  Abruf hängt  ", "GiroCrab 1.0\nSystem: windows x86_64\n");
        assert!(body.starts_with("### Was ist passiert?\n\nAbruf hängt\n\n### Diagnose"));
        assert!(body.ends_with("```text\nGiroCrab 1.0\nSystem: windows x86_64\n```"));
        assert!(issue_body("   ", "x").contains(PLACEHOLDER));
    }

    #[test]
    fn fence_outgrows_backticks_in_diagnostics() {
        let body = issue_body("", "Fehler: ```` @team #12");
        assert!(body.contains("\n`````text\n"));
        assert!(body.ends_with("\n`````"));
    }

    #[test]
    fn issue_url_carries_only_sanitized_text() {
        let terms = vec!["Erika Mustermann".to_string()];
        let (desc, diag) = sanitized_parts(
            "Konto von Erika Mustermann fehlt",
            "IBAN DE89370400440532013000 bei erika mustermann",
            &terms,
        );
        let (url, shortened) = issue_url(&desc, &diag);
        assert!(url.starts_with(&format!("{NEW_ISSUE}?title=%5BBUG%5D%20&body=")));
        assert!(!shortened);
        assert!(!url.to_lowercase().contains("mustermann"));
        assert!(!url.contains("DE89370400440532013000"));
        assert!(url.contains("%5BREDACTED%5D"));
    }

    #[test]
    fn issue_url_drops_trailing_lines_then_cuts_description() {
        let diag: String = (0..2000)
            .map(|i| format!("Zeile {i}: Abruf fehlgeschlagen\n"))
            .collect();
        let (url, shortened) = issue_url("Kurz", &diag);
        assert!(shortened && url.len() <= MAX_URL_LENGTH);
        assert!(url.contains("Zeile%200%3A"));
        assert!(!url.contains("Zeile%201999%3A"));
        let (url, shortened) = issue_url(&"ä".repeat(5000), "");
        assert!(shortened && url.len() <= MAX_URL_LENGTH);
    }

    #[test]
    fn query_encoding_escapes_plus_and_separators() {
        assert_eq!(encode_query("a+b & c/ü"), "a%2Bb%20%26%20c%2F%C3%BC");
    }

    #[test]
    fn diagnostics_text_adds_log_section_only_with_lines() {
        let plain = diagnostics_text("1.2.3", &[]);
        assert!(plain.starts_with("GiroCrab 1.2.3 (Windows-App)\nSystem: "));
        assert!(!plain.contains("Abruf-Protokoll"));
        let with_log = diagnostics_text("1.2.3", &["Abruf OK".to_string()]);
        assert!(with_log.ends_with("Abruf-Protokoll (neueste zuerst):\nAbruf OK"));
    }
}
