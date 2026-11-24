//! Druckansicht der Buchungsliste: eigenständiges HTML, das der Standardbrowser druckt.
//! Reine Logik ohne Slint- und Vault-Bezug, damit sie auf jeder Plattform testbar ist.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Platzhalter für verdeckte Werte im Privatmodus.
pub const MASK: &str = "••••••";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Paper {
    A4,
    Letter,
}

impl Paper {
    /// Letter für die Regionen US und CA (`en_US.UTF-8`, `fr-CA`), unabhängig von der Sprache, sonst A4.
    pub fn from_locale(locale: &str) -> Self {
        let base = locale.split(['.', '@']).next().unwrap_or("");
        let region = base.split(['_', '-']).skip(1).find(|p| p.len() == 2);
        match region {
            Some(r) if r.eq_ignore_ascii_case("US") || r.eq_ignore_ascii_case("CA") => {
                Paper::Letter
            }
            _ => Paper::A4,
        }
    }

    fn css(self) -> &'static str {
        match self {
            Paper::A4 => "A4",
            Paper::Letter => "Letter",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Row {
    pub date: String,
    pub counterparty: String,
    pub purpose: String,
    pub amount_cents: i64,
    pub currency: String,
}

#[derive(Clone, Debug)]
pub struct Report {
    pub account: String,
    pub period: String,
    pub rows: Vec<Row>,
    pub privacy: bool,
    pub paper: Paper,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Total {
    pub currency: String,
    pub income: i64,
    pub expense: i64,
}

impl Total {
    pub fn net(&self) -> i64 {
        self.income.saturating_add(self.expense)
    }
}

pub fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// `-1.234,56 EUR`
pub fn format_amount(cents: i64, currency: &str) -> String {
    let abs = cents.unsigned_abs();
    let digits = (abs / 100).to_string();
    let mut grouped = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push('.');
        }
        grouped.push(ch);
    }
    let sign = if cents < 0 { "-" } else { "" };
    format!("{sign}{grouped},{:02} {currency}", abs % 100)
}

/// Summen je Währung (#569): nie über Währungen hinweg addieren, EUR zuerst.
pub fn totals(rows: &[Row]) -> Vec<Total> {
    let mut map: BTreeMap<&str, (i64, i64)> = BTreeMap::new();
    for r in rows {
        let e = map.entry(r.currency.as_str()).or_default();
        if r.amount_cents >= 0 {
            e.0 = e.0.saturating_add(r.amount_cents);
        } else {
            e.1 = e.1.saturating_add(r.amount_cents);
        }
    }
    let mut out: Vec<Total> = map
        .into_iter()
        .map(|(c, (income, expense))| Total {
            currency: c.to_string(),
            income,
            expense,
        })
        .collect();
    out.sort_by_key(|t| t.currency != "EUR"); // stabil: EUR vorne, Rest alphabetisch
    out
}

pub fn period_label(from: Option<&str>, to: Option<&str>) -> String {
    match (from, to) {
        (Some(f), Some(t)) => format!("{f} bis {t}"),
        (Some(f), None) => format!("ab {f}"),
        (None, Some(t)) => format!("bis {t}"),
        (None, None) => "Gesamter Zeitraum".to_string(),
    }
}

fn shown(privacy: bool, text: &str) -> String {
    if privacy {
        MASK.to_string()
    } else {
        html_escape(text)
    }
}

/// Im Privatmodus bleibt der Währungscode sichtbar, nur der Betrag wird verdeckt.
fn money(privacy: bool, cents: i64, currency: &str) -> String {
    if privacy {
        format!("{MASK} {}", html_escape(currency))
    } else {
        html_escape(&format_amount(cents, currency))
    }
}

pub fn render_html(r: &Report) -> String {
    let p = r.privacy;
    let mut h = format!(
        "<!DOCTYPE html>\n<html lang=\"de\"><head><meta charset=\"utf-8\">\
<title>GiroCrab – Buchungen</title><style>\
@page {{ size: {}; margin: 15mm }}\
body{{font:10pt sans-serif;color:#000}}h1{{font-size:14pt;margin:0 0 4pt}}\
table{{border-collapse:collapse;width:100%;margin:8pt 0}}\
th,td{{border-bottom:1px solid #bbb;padding:2pt 4pt;text-align:left;vertical-align:top}}\
.n{{text-align:right;white-space:nowrap}}thead{{display:table-header-group}}\
tr{{page-break-inside:avoid}}</style></head><body>\n",
        r.paper.css()
    );
    h.push_str(&format!(
        "<h1>Buchungen</h1><p>Konto: {}<br>Zeitraum: {}<br>{} Buchungen</p>\n",
        shown(p, &r.account),
        html_escape(&r.period),
        r.rows.len()
    ));
    h.push_str(
        "<h2>Summen je Währung</h2><table><thead><tr><th>Währung</th>\
<th class=\"n\">Einnahmen</th><th class=\"n\">Ausgaben</th><th class=\"n\">Saldo</th></tr></thead><tbody>\n",
    );
    for t in totals(&r.rows) {
        let c = &t.currency;
        h.push_str(&format!(
            "<tr><td>{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td></tr>\n",
            html_escape(c),
            money(p, t.income, c),
            money(p, t.expense, c),
            money(p, t.net(), c)
        ));
    }
    h.push_str(
        "</tbody></table><table><thead><tr><th>Datum</th><th>Gegenpartei</th>\
<th>Verwendungszweck</th><th class=\"n\">Betrag</th></tr></thead><tbody>\n",
    );
    for row in &r.rows {
        h.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td class=\"n\">{}</td></tr>\n",
            html_escape(&row.date),
            shown(p, &row.counterparty),
            shown(p, &row.purpose),
            money(p, row.amount_cents, &row.currency)
        ));
    }
    h.push_str("</tbody></table></body></html>\n");
    h
}

/// Schreibt das HTML in eine neue, eindeutig benannte Datei im Temp-Verzeichnis.
/// Unter Unix nur für den Eigentümer lesbar (0600).
pub fn write_temp(html: &str) -> std::io::Result<PathBuf> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "girocrab-druck-{}-{nanos}.html",
        std::process::id()
    ));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&path)?;
    f.write_all(html.as_bytes())?;
    Ok(path)
}

/// Erster nicht leerer Wert in Reihenfolge (`LC_ALL` vor `LANG`); leere Variablen gelten als nicht gesetzt.
pub fn first_locale<I: IntoIterator<Item = Option<String>>>(values: I) -> Option<String> {
    values.into_iter().flatten().find(|v| !v.trim().is_empty())
}

/// Wert `name` vom Typ `REG_SZ` aus der Ausgabe von `reg query` (z. B. `LocaleName    REG_SZ    en-US`).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn reg_sz_value(output: &str, name: &str) -> Option<String> {
    output.lines().find_map(|l| {
        let mut parts = l.split_whitespace();
        if parts.next()? != name || parts.next()? != "REG_SZ" {
            return None;
        }
        parts.next().map(str::to_string)
    })
}

/// Öffnet die Datei im Standardprogramm für HTML (dem Browser). Kein Win32-Code nötig.
pub fn open_in_browser(path: &Path) -> std::io::Result<()> {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    // `status()` wartet auf den kurzen Starter-Prozess (kein Zombie, `clippy::zombie_processes`).
    let status = std::process::Command::new(program).arg(path).status()?;
    // `explorer` meldet auch bei Erfolg oft Exitcode 1: dort zählt nur ein Startfehler.
    if cfg!(windows) || status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "{program} beendet mit {status}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(cents: i64, cur: &str) -> Row {
        Row {
            date: "2026-03-01".into(),
            counterparty: "Müller & <Söhne>".into(),
            purpose: "Rechnung \"42\"".into(),
            amount_cents: cents,
            currency: cur.into(),
        }
    }

    fn report(privacy: bool, paper: Paper, rows: Vec<Row>) -> Report {
        Report {
            account: "Privatkonto".into(),
            period: "ab 2026-03-01".into(),
            rows,
            privacy,
            paper,
        }
    }

    #[test]
    fn escapes_html_in_user_fields() {
        assert_eq!(
            html_escape("<a href=\"x\">&'"),
            "&lt;a href=&quot;x&quot;&gt;&amp;&#39;"
        );
        let html = render_html(&report(false, Paper::A4, vec![row(-1000, "EUR")]));
        assert!(html.contains("Müller &amp; &lt;Söhne&gt;"));
        assert!(html.contains("Rechnung &quot;42&quot;"));
        assert!(!html.contains("<Söhne>"));
    }

    #[test]
    fn paper_sets_page_size() {
        assert_eq!(Paper::from_locale("en_US.UTF-8"), Paper::Letter);
        assert_eq!(Paper::from_locale("en-CA"), Paper::Letter);
        assert_eq!(Paper::from_locale("fr_CA"), Paper::Letter);
        assert_eq!(Paper::from_locale("fr-CA"), Paper::Letter);
        assert_eq!(Paper::from_locale("es-US"), Paper::Letter);
        assert_eq!(Paper::from_locale("de_DE.UTF-8"), Paper::A4);
        assert_eq!(Paper::from_locale("en_GB@euro"), Paper::A4);
        assert_eq!(Paper::from_locale("C"), Paper::A4);
        assert_eq!(Paper::from_locale(""), Paper::A4);
        assert!(render_html(&report(false, Paper::A4, vec![])).contains("size: A4"));
        assert!(render_html(&report(false, Paper::Letter, vec![])).contains("size: Letter"));
    }

    #[test]
    fn totals_are_separate_per_currency_with_eur_first() {
        let rows = [
            row(-500, "USD"),
            row(250_000, "EUR"),
            row(-12_345, "EUR"),
            row(100, "USD"),
        ];
        let t = totals(&rows);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].currency, "EUR");
        assert_eq!(
            (t[0].income, t[0].expense, t[0].net()),
            (250_000, -12_345, 237_655)
        );
        assert_eq!((t[1].currency.as_str(), t[1].net()), ("USD", -400));
        assert_eq!(format_amount(-123_456_789, "EUR"), "-1.234.567,89 EUR");
        assert_eq!(format_amount(5, "EUR"), "0,05 EUR");
    }

    #[test]
    fn privacy_mode_masks_personal_fields_and_amounts() {
        let html = render_html(&report(true, Paper::A4, vec![row(-4711, "EUR")]));
        assert!(html.contains(MASK));
        for secret in ["Müller", "Rechnung", "47,11", "Privatkonto"] {
            assert!(!html.contains(secret), "{secret} leaked");
        }
        assert!(html.contains("2026-03-01"));
        assert!(html.contains(&format!("{MASK} EUR")));
    }

    #[test]
    fn locale_skips_empty_values_and_reads_registry_output() {
        let s = |v: &str| Some(v.to_string());
        assert_eq!(first_locale([s(""), s("fr_CA.UTF-8")]), s("fr_CA.UTF-8"));
        assert_eq!(first_locale([s("en_US"), s("de_DE")]), s("en_US"));
        assert_eq!(first_locale([None, s("  ")]), None);
        let out = "\r\nHKEY_CURRENT_USER\\Control Panel\\International\r\n    LocaleName    REG_SZ    en-US\r\n\r\n";
        assert_eq!(reg_sz_value(out, "LocaleName"), s("en-US"));
        assert_eq!(reg_sz_value(out, "sCountry"), None);
        assert_eq!(reg_sz_value("", "LocaleName"), None);
    }

    #[test]
    fn period_label_and_temp_file() {
        assert_eq!(period_label(None, None), "Gesamter Zeitraum");
        assert_eq!(period_label(Some("2026-01-01"), None), "ab 2026-01-01");
        let p = write_temp("<html></html>").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "<html></html>");
        let _ = std::fs::remove_file(p);
    }
}
