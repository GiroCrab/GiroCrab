//! Pure texts and checks behind the "Import, Export & Einstellungen" dialog
//! (`ui/settings_import_dialog.slint`, wired in `settings_import_ui.rs`): file checks before an
//! import, the Amazon capture check, export rows, backup rules, settings lines and the
//! Abrufprotokoll. No vault access here, so everything is unit-testable.

use std::collections::HashSet;

use girocrab_core::export::{ExportCsvRow, esc};
use girocrab_core::iban::has_iban_shape;
use girocrab_core::importers::amazon_orders::{
    Capture, bookings, is_asin, parse_date, price_cents,
};
use girocrab_core::importers::moneymoney_db::{
    MoneyMoneyDbAccount, MoneyMoneyDbAccountChoice, MoneyMoneyDbImportProgress,
    MoneyMoneyDbImportResult,
};
use girocrab_core::importers::moneymoney_import::{
    MmRow, fingerprint, parse_camt_statement, parse_csv_export, parse_export,
    parse_mt940_statement, parse_xml_export, split_category,
};
use girocrab_core::sync_log::{SyncLogEntry, SyncLogOutcome, SyncLogSource};
use girocrab_core::{Booking, MoneyMoneyImportPreview, MoneyMoneyImportResult, SettingEntry};

/// One list line of the dialog.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Line {
    pub title: String,
    pub detail: String,
}

/// FinTS product registration choices, in menu order (stored values of the core).
pub const PRODUCT_MODES: [(&str, &str); 3] = [
    ("", "Noch nicht festgelegt"),
    ("builtin", "Eingebaute Registrierung"),
    ("own", "Eigene Produkt-ID"),
];

pub fn product_mode_names() -> Vec<String> {
    PRODUCT_MODES.iter().map(|m| m.1.to_string()).collect()
}

pub fn product_mode_index(mode: &str) -> i32 {
    PRODUCT_MODES.iter().position(|m| m.0 == mode).unwrap_or(0) as i32
}

pub fn product_mode_value(index: i32) -> &'static str {
    usize::try_from(index)
        .ok()
        .and_then(|i| PRODUCT_MODES.get(i))
        .map_or("", |m| m.0)
}

pub fn product_source_text(source: &str) -> String {
    match source {
        "builtin" => "Verwendet wird die eingebaute Registrierung.".into(),
        "own" => "Verwendet wird die eigene Produkt-ID.".into(),
        _ => "Es ist keine Registrierung verfügbar: eigene Produkt-ID eintragen.".into(),
    }
}

// ---- import file check ---------------------------------------------------------------------------

/// What a statement file contains, before the core's dry run.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FileCheck {
    pub format: &'static str,
    pub rows: usize,
    pub with_valid_iban: usize,
    /// Rows whose fingerprint already occurred earlier in the same file.
    pub repeated_in_file: usize,
    pub categories: Vec<String>,
}

/// Parses a statement with the parser of its format; unknown content falls back to detection.
pub fn parse_statement(name: &str, data: &[u8]) -> Result<(&'static str, Vec<MmRow>), String> {
    let lower = name.to_lowercase();
    let text = String::from_utf8_lossy(data);
    let (format, rows) = if lower.ends_with(".sta") || lower.ends_with(".mt940") {
        ("MT940", parse_mt940_statement(&text))
    } else if lower.ends_with(".csv") {
        ("CSV", parse_csv_export(&text).map_err(|e| e.to_string())?)
    } else if lower.ends_with(".xml") && text.contains("camt.053") {
        ("CAMT.053", parse_camt_statement(&text))
    } else if lower.ends_with(".xml") {
        ("XML", parse_xml_export(&text).map_err(|e| e.to_string())?)
    } else {
        ("Erkannt", parse_export(data).map_err(|e| e.to_string())?)
    };
    if rows.is_empty() {
        // The content may differ from the extension: let the core detect it.
        let rows = parse_export(data).map_err(|e| e.to_string())?;
        return Ok(("Erkannt", rows));
    }
    Ok((format, rows))
}

pub fn check_rows(format: &'static str, rows: &[MmRow]) -> FileCheck {
    let mut seen = HashSet::new();
    let mut repeated = 0;
    let mut cats: Vec<String> = vec![];
    for r in rows {
        let key = fingerprint(
            &r.booking_date,
            r.amount_cents,
            &r.iban,
            &r.name,
            &r.purpose,
        );
        if !seen.insert(key) {
            repeated += 1;
        }
        let path = if r.category_parts.is_empty() {
            split_category(&r.category)
        } else {
            r.category_parts.clone()
        };
        if !path.is_empty() {
            cats.push(path.join(" › "));
        }
    }
    cats.sort();
    cats.dedup();
    FileCheck {
        format,
        rows: rows.len(),
        with_valid_iban: rows.iter().filter(|r| has_iban_shape(&r.iban)).count(),
        repeated_in_file: repeated,
        categories: cats,
    }
}

pub fn file_check_text(c: &FileCheck) -> String {
    let mut s = format!(
        "{}: {} Zeilen, davon {} mit gültiger IBAN.",
        c.format, c.rows, c.with_valid_iban
    );
    if c.repeated_in_file > 0 {
        s.push_str(&format!(
            " {} Zeilen kommen in der Datei doppelt vor.",
            c.repeated_in_file
        ));
    }
    if !c.categories.is_empty() {
        s.push_str(&format!(" {} Kategorien erkannt.", c.categories.len()));
    }
    s
}

pub fn preview_text(p: &MoneyMoneyImportPreview, date: &dyn Fn(&str) -> String) -> String {
    let mut s = format!(
        "{} Zeilen: {} neu, {} bereits vorhanden",
        p.total_rows, p.new_bookings, p.duplicates_skipped_or_enriched
    );
    if let (Some(a), Some(b)) = (&p.date_range_start, &p.date_range_end) {
        s.push_str(&format!(", {} bis {}", date(a), date(b)));
    }
    if p.notes_count > 0 {
        s.push_str(&format!(", {} Notizen", p.notes_count));
    }
    s.push('.');
    s
}

pub fn import_result_text(r: &MoneyMoneyImportResult) -> String {
    format!(
        "Import: {} neu, {} bereits vorhanden, {} ergänzt{}.",
        r.imported,
        r.duplicates,
        r.enriched,
        if r.account_created {
            " (neues Konto angelegt)"
        } else {
            ""
        }
    )
}

/// The accounts a database import sends from here, without a target picker: each with its
/// suggested kind and target. One whose IBAN fits several accounts (#1993) stays out, as
/// neither a guess nor a new account is the user's choice; their number comes back for the result.
pub fn db_account_choices(
    accounts: &[MoneyMoneyDbAccount],
) -> (Vec<MoneyMoneyDbAccountChoice>, usize) {
    let choices = accounts
        .iter()
        .map(|a| MoneyMoneyDbAccountChoice {
            mm_id: a.mm_id,
            import: !a.target_needs_choice,
            kind: a.suggested_kind,
            target_account_id: a.existing_account_id,
        })
        .collect();
    let left_out = accounts.iter().filter(|a| a.target_needs_choice).count();
    (choices, left_out)
}

/// What the result adds for the accounts [`db_account_choices`] left out. A count, no names:
/// the line stays on screen when privacy mode is switched on (like the export status).
pub fn db_left_out_text(count: usize) -> String {
    match count {
        0 => String::new(),
        1 => " 1 Konto nicht importiert, weil mehrere Konten zur IBAN passen.".into(),
        n => format!(" {n} Konten nicht importiert, weil mehrere Konten zur IBAN passen."),
    }
}

pub fn db_result_text(r: &MoneyMoneyDbImportResult) -> String {
    format!(
        "Datenimport: {} neue Umsätze, {} Duplikate, {} ergänzt, {} Konten neu, {} zusammengeführt, {} Kategorien angelegt.",
        r.bookings_new,
        r.bookings_duplicates,
        r.bookings_enriched,
        r.accounts_created,
        r.accounts_merged,
        r.categories_created
    )
}

pub fn progress_text(p: &MoneyMoneyDbImportProgress) -> String {
    if p.account_index == 0 {
        return "Datenbank wird gelesen …".into();
    }
    if p.account_name.is_empty() {
        return "Wird abgeschlossen …".into();
    }
    format!(
        "Konto {} von {}: {} ({} von {} Umsätzen)",
        p.account_index, p.account_count, p.account_name, p.bookings_done, p.bookings_total
    )
}

// ---- Amazon capture ------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Default)]
pub struct AmazonCheck {
    pub orders: usize,
    pub cancelled: usize,
    pub items: usize,
    pub items_without_asin: usize,
    pub bad_dates: usize,
    pub bookings: usize,
    pub total_cents: i64,
    /// Orders whose summary rows carry a readable price.
    pub priced_orders: usize,
}

pub fn amazon_check(json: &str) -> Result<AmazonCheck, String> {
    let capture: Capture = serde_json::from_str(json)
        .map_err(|_| "Das ist kein Bestell-Export (JSON).".to_string())?;
    if capture.orders.is_empty() {
        return Err("Keine Bestellungen erkannt.".into());
    }
    let items: Vec<_> = capture.orders.iter().flat_map(|o| &o.items).collect();
    let bks = bookings(&capture.orders);
    Ok(AmazonCheck {
        orders: capture.orders.len(),
        cancelled: capture.orders.iter().filter(|o| o.cancelled).count(),
        items: items.len(),
        items_without_asin: items.iter().filter(|i| !is_asin(&i.asin)).count(),
        bad_dates: capture
            .orders
            .iter()
            .filter(|o| parse_date(&o.date).is_none())
            .count(),
        bookings: bks.len(),
        total_cents: bks.iter().map(|b| b.cents).sum(),
        priced_orders: capture
            .orders
            .iter()
            .filter(|o| o.rows.iter().any(|r| price_cents(&r.value).is_some()))
            .count(),
    })
}

pub fn amazon_text(c: &AmazonCheck, eur: &dyn Fn(i64) -> String) -> String {
    let mut s = format!(
        "{} Bestellungen ({} storniert), {} Artikel, {} Buchungen über {}.",
        c.orders,
        c.cancelled,
        c.items,
        c.bookings,
        eur(c.total_cents)
    );
    if c.bad_dates > 0 {
        s.push_str(&format!(
            " {} Bestellungen ohne lesbares Datum.",
            c.bad_dates
        ));
    }
    if c.items_without_asin > 0 {
        s.push_str(&format!(
            " {} Artikel ohne Produkt-ID.",
            c.items_without_asin
        ));
    }
    s
}

// ---- export ----------------------------------------------------------------------------------------

/// Column switches of the CSV export tab.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CsvOptions {
    pub category: bool,
    pub note: bool,
    pub iban: bool,
    pub running_balance: bool,
}

/// Maps the switches onto the core's CSV columns (daily balance and tags are not offered here).
pub fn csv_columns(o: CsvOptions) -> girocrab_core::export::ExportCsvColumns {
    girocrab_core::export::ExportCsvColumns {
        category: o.category,
        note: o.note,
        daily_balance: false,
        running_balance: o.running_balance,
        iban: o.iban,
        tags: false,
    }
}

/// Comparison key for file names as NTFS sees them: each character uppercased on its own,
/// one to one (so `ς`, `σ` and `Σ` collide); multi-character mappings (`ß` → `SS`) are not applied.
fn ntfs_key(name: &str) -> String {
    name.chars()
        .map(|c| {
            let mut upper = c.to_uppercase();
            match (upper.next(), upper.next()) {
                (Some(u), None) => u,
                _ => c,
            }
        })
        .collect()
}

/// `name` for the first try (`i` < 2), then `stem-2.ext`, `stem-3.ext`, …
pub fn numbered_file_name(name: &str, i: u32) -> String {
    if i < 2 {
        return name.to_string();
    }
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem}-{i}.{ext}"),
        _ => format!("{name}-{i}"),
    }
}

/// One CSV file name per account, in the given order: cleaned account name, unique within the
/// batch (case-insensitive like NTFS, see `ntfs_key`) and not taken in the folder (`exists`);
/// clashes get `-2`, `-3`, … The fixed prefix rules out reserved device names (`CON`, `NUL`, …).
pub fn per_account_file_names(
    names: &[String],
    today: &str,
    exists: impl Fn(&str) -> bool,
) -> Vec<String> {
    let mut taken = std::collections::HashSet::new();
    names
        .iter()
        .map(|n| {
            let base = crate::lists::safe_file_name(&n.replace(['/', '\\'], "_"), "Konto");
            let stem = format!("girocrab-umsaetze-{base}-{today}");
            let mut name = format!("{stem}.csv");
            let mut i = 2;
            while taken.contains(&ntfs_key(&name)) || exists(&name) {
                name = format!("{stem}-{i}.csv");
                i += 1;
            }
            taken.insert(ntfs_key(&name));
            name
        })
        .collect()
}

/// `balances`: running balance per booking id (`BookingPage.running_balances`), empty when not wanted.
pub fn export_rows(
    list: &[Booking],
    balances: &std::collections::HashMap<i64, i64>,
) -> Vec<ExportCsvRow> {
    list.iter()
        .map(|b| ExportCsvRow {
            date: b.booking_date.chars().take(10).collect(),
            account_name: b.account_name.clone(),
            counterparty: b.counterparty.clone(),
            counterparty_iban: b.counterparty_iban.clone(),
            purpose: b.purpose.clone(),
            category: b.category_path.clone(),
            note: b.note.clone(),
            tags: crate::planning::tag_summary(&b.tags),
            amount_cents: b.amount_cents,
            currency: b.currency.clone(),
            day_balance_cents: None,
            balance_cents: balances.get(&b.id).copied(),
        })
        .collect()
}

/// `YYYY-MM-DD` as typed; empty = no limit. Anything else is an error.
pub fn parse_day(text: &str) -> Result<Option<String>, String> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(None);
    }
    let ok = t.len() == 10
        && t.char_indices().all(|(i, c)| match i {
            4 | 7 => c == '-',
            _ => c.is_ascii_digit(),
        })
        && matches!(t[5..7].parse::<u32>(), Ok(1..=12))
        && t[8..10].parse::<u32>().is_ok_and(|d| {
            let (y, m) = (
                t[..4].parse::<u32>().unwrap_or(0),
                t[5..7].parse::<u32>().unwrap_or(0),
            );
            let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
            let days = match m {
                2 if leap => 29,
                2 => 28,
                4 | 6 | 9 | 11 => 30,
                _ => 31,
            };
            (1..=days).contains(&d)
        });
    if ok {
        Ok(Some(t.to_string()))
    } else {
        Err(format!("„{t}“ ist kein Datum (JJJJ-MM-TT)."))
    }
}

/// Period menu of the export tab (`settings_import_dialog.slint`): 0 this month, 1 this year,
/// 2 all, 3 own Von/Bis, 4 the range of the main list.
pub const PERIOD_CUSTOM: i32 = 3;
pub const PERIOD_SELECTION: i32 = 4;

/// Maps the period menu onto the core's `date_from` / `date_to` (like the Mac export sheet).
/// `today` is `YYYY-MM-DD`; `list_from` is the start of the main list's range ("" = all).
/// An own range with Von after Bis (or a malformed day) is an error.
pub fn export_period(
    preset: i32,
    today: &str,
    from: &str,
    to: &str,
    list_from: &str,
) -> Result<(Option<String>, Option<String>), String> {
    match preset {
        0 => Ok((today.get(..7).map(|m| format!("{m}-01")), None)),
        1 => Ok((today.get(..4).map(|y| format!("{y}-01-01")), None)),
        PERIOD_CUSTOM => {
            let (from, to) = (parse_day(from)?, parse_day(to)?);
            match (&from, &to) {
                (Some(f), Some(t)) if f > t => Err(format!(
                    "„Von“ ({f}) liegt nach „Bis“ ({t}). Bitte den Zeitraum korrigieren."
                )),
                _ => Ok((from, to)),
            }
        }
        PERIOD_SELECTION => parse_day(list_from).map(|from| (from, Some(today.to_string()))),
        _ => Ok((None, None)),
    }
}

/// Hint below the period row; empty = the export buttons are enabled.
pub fn export_period_hint(preset: i32, from: &str, to: &str) -> String {
    if preset != PERIOD_CUSTOM {
        return String::new();
    }
    export_period(preset, "", from, to, "")
        .err()
        .unwrap_or_default()
}

/// CAMT version menu: 0 = `camt.053.001.02` (older programs), otherwise `camt.053.001.08`.
pub fn camt_version(index: i32) -> girocrab_core::export::CamtVersion {
    if index == 0 {
        girocrab_core::export::CamtVersion::V02
    } else {
        girocrab_core::export::CamtVersion::V08
    }
}

pub fn export_file_name(kind: &str, today: &str) -> String {
    match kind {
        "camt" => format!("girocrab-camt053-{today}.xml"),
        _ => format!("girocrab-umsaetze-{today}.csv"),
    }
}

/// Days since 1970-01-01 → (year, month, day), proleptic Gregorian.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// `2025-03-31T12:00:00+00:00` for the `CreDtTm` of a CAMT export.
pub fn iso_utc(unix_secs: i64) -> String {
    let (y, m, d) = civil(unix_secs.div_euclid(86_400));
    let s = unix_secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}+00:00",
        s / 3600,
        s % 3600 / 60,
        s % 60
    )
}

pub fn today(unix_secs: i64) -> String {
    iso_utc(unix_secs)[..10].to_string()
}

// ---- backup ------------------------------------------------------------------------------------------

pub fn validate_backup_password(pw: &str, repeat: &str, writing: bool) -> Result<(), String> {
    if pw.is_empty() {
        return Err("Bitte ein Passwort für die Sicherung angeben.".into());
    }
    // Same floor as the core's `write_backup` (and the Mac dialog): the backup is only
    // as strong as this password. Restoring takes any password, older backups may be shorter.
    if writing && pw.chars().count() < 8 {
        return Err("Das Passwort für die Sicherung muss mindestens 8 Zeichen lang sein.".into());
    }
    if writing && pw != repeat {
        return Err("Die beiden Passwörter stimmen nicht überein.".into());
    }
    Ok(())
}

// ---- settings --------------------------------------------------------------------------------------------

pub fn setting_lines(list: &[SettingEntry]) -> Vec<Line> {
    list.iter()
        .map(|s| Line {
            title: s.label.clone(),
            detail: format!(
                "{} · {}",
                s.group_title,
                if s.is_secret {
                    if s.has_value {
                        "gesetzt (verborgen)".to_string()
                    } else {
                        "nicht gesetzt".to_string()
                    }
                } else if s.value.is_empty() {
                    "leer".to_string()
                } else {
                    s.value.clone()
                }
            ),
        })
        .collect()
}

// ---- Abrufprotokoll ----------------------------------------------------------------------------------------

pub fn outcome_name(o: SyncLogOutcome) -> &'static str {
    match o {
        SyncLogOutcome::Ok => "Erfolgreich",
        SyncLogOutcome::Partial => "Teilweise",
        SyncLogOutcome::Error => "Fehler",
        SyncLogOutcome::Cancelled => "Abgebrochen",
    }
}

pub fn source_name(s: SyncLogSource) -> &'static str {
    match s {
        SyncLogSource::Manual => "Manuell",
        SyncLogSource::Automatic => "Automatisch",
        SyncLogSource::Statements => "Kontoauszüge",
    }
}

/// `dd.mm.yyyy hh:mm` (UTC) of a log timestamp.
pub fn stamp(ms: i64) -> String {
    let iso = iso_utc(ms.div_euclid(1000));
    format!(
        "{}.{}.{} {}",
        &iso[8..10],
        &iso[5..7],
        &iso[..4],
        &iso[11..16]
    )
}

pub fn log_lines(list: &[SyncLogEntry]) -> Vec<Line> {
    list.iter()
        .map(|e| {
            let mut detail = format!(
                "{} · {} · {}",
                stamp(e.timestamp_ms),
                outcome_name(e.outcome),
                source_name(e.source)
            );
            if matches!(e.outcome, SyncLogOutcome::Ok | SyncLogOutcome::Partial) {
                detail.push_str(&format!(" · {} neu", e.imported));
            }
            if !e.message.is_empty() {
                detail.push_str(&format!(" · {}", e.message));
            }
            Line {
                title: e.connection_name.clone(),
                detail,
            }
        })
        .collect()
}

pub fn log_xml(list: &[SyncLogEntry]) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<syncLog>\n");
    for e in list {
        out.push_str(&format!(
            "  <entry time=\"{}\" connection=\"{}\" source=\"{}\" outcome=\"{}\" imported=\"{}\">{}</entry>\n",
            esc(&stamp(e.timestamp_ms)),
            esc(&e.connection_name),
            source_name(e.source),
            outcome_name(e.outcome),
            e.imported,
            esc(&e.message)
        ));
    }
    out.push_str("</syncLog>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(date: &str, cents: i64, iban: &str, cat: &str) -> MmRow {
        MmRow {
            booking_date: date.into(),
            amount_cents: cents,
            iban: iban.into(),
            name: "Shop".into(),
            purpose: "Kauf".into(),
            category: cat.into(),
            ..Default::default()
        }
    }

    #[test]
    fn csv_switches_map_to_core_columns() {
        let off = csv_columns(CsvOptions::default());
        assert!(!off.category && !off.note && !off.iban && !off.running_balance);
        assert!(!off.daily_balance && !off.tags);
        let on = csv_columns(CsvOptions {
            category: true,
            note: true,
            iban: true,
            running_balance: true,
        });
        assert!(on.category && on.note && on.iban && on.running_balance);
        assert!(!on.daily_balance && !on.tags);
        let only_iban = csv_columns(CsvOptions {
            iban: true,
            ..Default::default()
        });
        assert!(only_iban.iban && !only_iban.running_balance && !only_iban.category);
    }

    #[test]
    fn same_named_accounts_get_unique_files() {
        let names: Vec<String> = vec!["Girokonto".into(), "Girokonto".into(), "girokonto".into()];
        assert_eq!(
            per_account_file_names(&names, "2025-03-31", |_| false),
            vec![
                "girocrab-umsaetze-Girokonto-2025-03-31.csv",
                "girocrab-umsaetze-Girokonto-2025-03-31-2.csv",
                "girocrab-umsaetze-girokonto-2025-03-31-3.csv",
            ]
        );
    }

    #[test]
    fn account_names_are_cleaned() {
        let names: Vec<String> = vec!["Giro/Spar: Haushalt".into(), "".into(), "...".into()];
        assert_eq!(
            per_account_file_names(&names, "2025-03-31", |_| false),
            vec![
                "girocrab-umsaetze-Giro_Spar_ Haushalt-2025-03-31.csv",
                "girocrab-umsaetze-Konto-2025-03-31.csv",
                "girocrab-umsaetze-Konto-2025-03-31-2.csv",
            ]
        );
    }

    #[test]
    fn existing_files_are_not_overwritten() {
        let names: Vec<String> = vec!["Giro".into()];
        let taken = "girocrab-umsaetze-Giro-2025-03-31.csv";
        assert_eq!(
            per_account_file_names(&names, "2025-03-31", |f| f == taken),
            vec!["girocrab-umsaetze-Giro-2025-03-31-2.csv"]
        );
    }

    #[test]
    fn batch_names_collide_like_ntfs() {
        // ς, σ and Σ uppercase to the same letter on NTFS
        let names: Vec<String> = vec!["Kontoσ".into(), "Kontoς".into(), "KontoΣ".into()];
        assert_eq!(
            per_account_file_names(&names, "2025-03-31", |_| false),
            vec![
                "girocrab-umsaetze-Kontoσ-2025-03-31.csv",
                "girocrab-umsaetze-Kontoς-2025-03-31-2.csv",
                "girocrab-umsaetze-KontoΣ-2025-03-31-3.csv",
            ]
        );
        // NTFS maps one to one: ß does not become SS
        let names: Vec<String> = vec!["Straße".into(), "STRASSE".into()];
        assert_eq!(
            per_account_file_names(&names, "2025-03-31", |_| false),
            vec![
                "girocrab-umsaetze-Straße-2025-03-31.csv",
                "girocrab-umsaetze-STRASSE-2025-03-31.csv",
            ]
        );
    }

    #[test]
    fn numbered_names() {
        assert_eq!(numbered_file_name("a.csv", 1), "a.csv");
        assert_eq!(numbered_file_name("a.csv", 2), "a-2.csv");
        assert_eq!(numbered_file_name("a-2.csv", 3), "a-2-3.csv");
        assert_eq!(numbered_file_name("ohne", 2), "ohne-2");
        assert_eq!(numbered_file_name(".csv", 2), ".csv-2");
    }

    #[test]
    fn dates_and_stamps() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00+00:00");
        assert_eq!(iso_utc(1_743_422_400), "2025-03-31T12:00:00+00:00");
        assert_eq!(today(1_743_422_400), "2025-03-31");
        assert_eq!(stamp(1_743_422_400_000), "31.03.2025 12:00");
        assert_eq!(iso_utc(-1), "1969-12-31T23:59:59+00:00");
    }

    #[test]
    fn day_input() {
        assert_eq!(parse_day("  "), Ok(None));
        assert_eq!(parse_day("2025-03-01"), Ok(Some("2025-03-01".into())));
        assert!(parse_day("01.03.2025").is_err());
        assert!(parse_day("2025-13-01").is_err());
        assert!(parse_day("2026-02-30").is_err());
        assert!(parse_day("2026-02-29").is_err());
        assert!(parse_day("2026-04-31").is_err());
        assert_eq!(parse_day("2024-02-29"), Ok(Some("2024-02-29".into())));
        assert!(parse_day("1900-02-29").is_err());
        assert_eq!(parse_day("2000-02-29"), Ok(Some("2000-02-29".into())));
        assert!(export_period_hint(PERIOD_CUSTOM, "2026-02-30", "").contains("kein Datum"));
    }

    #[test]
    fn export_period_presets() {
        let t = "2026-11-23";
        assert_eq!(
            export_period(0, t, "", "", ""),
            Ok((Some("2026-11-01".into()), None))
        );
        assert_eq!(
            export_period(1, t, "", "", ""),
            Ok((Some("2026-01-01".into()), None))
        );
        // Von/Bis only count for the own range, even when they are reversed.
        assert_eq!(
            export_period(2, t, "2026-04-01", "2026-03-31", ""),
            Ok((None, None))
        );
    }

    #[test]
    fn custom_period_validation() {
        let t = "2026-11-23";
        assert_eq!(
            export_period(PERIOD_CUSTOM, t, " 2026-02-01 ", "2026-03-31", ""),
            Ok((Some("2026-02-01".into()), Some("2026-03-31".into())))
        );
        assert_eq!(
            export_period(PERIOD_CUSTOM, t, "", "2026-03-31", ""),
            Ok((None, Some("2026-03-31".into())))
        );
        assert!(export_period(PERIOD_CUSTOM, t, "2026-03-31", "2026-03-31", "").is_ok());
        assert!(export_period(PERIOD_CUSTOM, t, "2026-04-01", "2026-03-31", "").is_err());
        assert!(export_period(PERIOD_CUSTOM, t, "31.03.2026", "", "").is_err());
        assert_eq!(
            export_period_hint(PERIOD_CUSTOM, "2026-02-01", "2026-03-31"),
            ""
        );
        assert!(
            export_period_hint(PERIOD_CUSTOM, "2026-04-01", "2026-03-31").contains("liegt nach")
        );
        assert_eq!(export_period_hint(2, "2026-04-01", "2026-03-31"), "");
    }

    #[test]
    fn selection_period_and_camt_version() {
        let t = "2026-11-23";
        assert_eq!(
            export_period(PERIOD_SELECTION, t, "x", "y", "2026-08-25"),
            Ok((Some("2026-08-25".into()), Some(t.into())))
        );
        assert_eq!(
            export_period(PERIOD_SELECTION, t, "", "", ""),
            Ok((None, Some(t.into())))
        );
        assert_eq!(camt_version(0), girocrab_core::export::CamtVersion::V02);
        assert_eq!(camt_version(1), girocrab_core::export::CamtVersion::V08);
    }

    #[test]
    fn file_check_counts() {
        let rows = vec![
            row("2025-01-01", -500, "DE89370400440532013000", "Essen / Café"),
            row("2025-01-01", -500, "DE89370400440532013000", "Essen / Café"),
            row("2025-01-02", 100, "kaputt", ""),
        ];
        let c = check_rows("CSV", &rows);
        assert_eq!(c.rows, 3);
        assert_eq!(c.with_valid_iban, 2);
        assert_eq!(c.repeated_in_file, 1);
        assert_eq!(c.categories, vec!["Essen › Café".to_string()]);
        let t = file_check_text(&c);
        assert!(t.contains("3 Zeilen") && t.contains("1 Zeilen kommen"));
    }

    #[test]
    fn statement_parsing_by_name() {
        assert!(parse_statement("x.csv", b"nonsense").is_err());
        let csv = "\"Datum\";\"Wertstellung\";\"Kategorie\";\"Name\";\"Verwendungszweck\";\"Konto\";\"Bank\";\"Betrag\";\"Währung\"\n\"01.02.2025\";\"01.02.2025\";\"\";\"Shop\";\"Kauf\";\"DE89370400440532013000\";\"\";\"-5,00\";\"EUR\"\n";
        let (f, rows) = parse_statement("auszug.csv", csv.as_bytes()).unwrap();
        assert!(f == "CSV" || f == "Erkannt");
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn amazon_capture() {
        assert!(amazon_check("kein json").is_err());
        assert!(amazon_check("{\"orders\":[]}").is_err());
        let json = r#"{"orders":[{"code":"028-1234567-1234567","date":"12. März 2025",
            "items":[{"title":"A","qty":1,"asin":"B012345678"},{"title":"B","asin":""}],
            "rows":[{"label":"Gesamtsumme","value":"45,98 €"}]}]}"#;
        let c = amazon_check(json).unwrap();
        assert_eq!((c.orders, c.items, c.items_without_asin), (1, 2, 1));
        assert_eq!(c.bad_dates, 0);
        assert_eq!(c.priced_orders, 1);
        assert_eq!(c.total_cents, -4598);
        assert!(amazon_text(&c, &|v| v.to_string()).contains("1 Bestellungen"));
    }

    #[test]
    fn backup_rules() {
        assert!(validate_backup_password("", "", true).is_err());
        assert!(validate_backup_password("geheim123", "geheim124", true).is_err());
        assert!(validate_backup_password("a", "", false).is_ok());
        // writing needs 8 characters, like the core
        assert!(validate_backup_password("a", "a", true).is_err());
        assert!(validate_backup_password("1234567", "1234567", true).is_err());
        assert!(validate_backup_password("geheim123", "geheim123", true).is_ok());
    }

    #[test]
    fn product_modes() {
        assert_eq!(product_mode_index("own"), 2);
        assert_eq!(product_mode_index("?"), 0);
        assert_eq!(product_mode_value(1), "builtin");
        assert_eq!(product_mode_value(-1), "");
        assert!(product_source_text("").contains("keine"));
    }

    #[test]
    fn settings_hide_secrets() {
        let e = |secret, has, value: &str| SettingEntry {
            key: "k".into(),
            label: "Label".into(),
            help: String::new(),
            group: "g".into(),
            group_title: "Gruppe".into(),
            value: value.into(),
            is_secret: secret,
            has_value: has,
        };
        let l = setting_lines(&[e(true, true, ""), e(false, true, "x"), e(false, false, "")]);
        assert!(l[0].detail.contains("verborgen") && !l[0].detail.contains(" x"));
        assert!(l[1].detail.ends_with("x"));
        assert!(l[2].detail.ends_with("leer"));
    }

    #[test]
    fn log_text_and_xml_escape() {
        let e = SyncLogEntry {
            id: 1,
            timestamp_ms: 1_743_422_400_000,
            connection_id: None,
            connection_name: "A & B".into(),
            source: SyncLogSource::Automatic,
            outcome: SyncLogOutcome::Error,
            message: "<boom>".into(),
            imported: 0,
        };
        let l = log_lines(std::slice::from_ref(&e));
        assert!(l[0].detail.contains("Fehler") && l[0].detail.contains("Automatisch"));
        let x = log_xml(&[e]);
        assert!(x.contains("A &amp; B") && x.contains("&lt;boom&gt;"));
    }

    #[test]
    fn progress_phases() {
        let mut p = MoneyMoneyDbImportProgress::default();
        assert!(progress_text(&p).contains("gelesen"));
        p.account_index = 2;
        p.account_count = 3;
        p.account_name = "Giro".into();
        assert!(progress_text(&p).contains("Konto 2 von 3"));
    }

    fn db_account(
        mm_id: i64,
        name: &str,
        existing: Option<i64>,
        needs_choice: bool,
    ) -> MoneyMoneyDbAccount {
        MoneyMoneyDbAccount {
            mm_id,
            name: name.into(),
            iban: "DE89370400440532013000".into(),
            account_number: String::new(),
            currency: "EUR".into(),
            bank_code: String::new(),
            bank_name: String::new(),
            owner: String::new(),
            group_name: String::new(),
            mm_bank_id: None,
            is_portfolio: false,
            suggested_kind: girocrab_core::AccountKind::Checking,
            existing_account_id: existing,
            existing_account_name: None,
            target_needs_choice: needs_choice,
            booking_count: 0,
            pending_count: 0,
            holding_count: 0,
            value_history_count: 0,
            notes_count: 0,
            first_date: None,
            last_date: None,
            statement_count: 0,
            computed_balance_cents: 0,
            balance_source: String::new(),
            closed: false,
            reused_account_id: None,
        }
    }

    #[test]
    fn an_account_needing_a_target_choice_is_left_out_and_counted() {
        // #1993: no picker here, so neither the guess nor a new account
        let (choices, left_out) = db_account_choices(&[
            db_account(1, "Wallet EUR", Some(7), false),
            db_account(2, "Wallet USD", None, true),
        ]);
        let sent: Vec<(i64, bool, Option<i64>)> = choices
            .iter()
            .map(|c| (c.mm_id, c.import, c.target_account_id))
            .collect();
        assert_eq!(sent, vec![(1, true, Some(7)), (2, false, None)]);
        assert_eq!(left_out, 1);
        // a count only: no account names in the status line (privacy mode)
        let text = db_left_out_text(left_out);
        assert_eq!(
            text,
            " 1 Konto nicht importiert, weil mehrere Konten zur IBAN passen."
        );
        assert!(!text.contains("Wallet"));
        assert!(db_left_out_text(2).starts_with(" 2 Konten"));
        assert_eq!(db_left_out_text(0), "");
    }
}
