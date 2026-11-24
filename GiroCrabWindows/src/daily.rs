//! Daily-driver logic of the Windows app without Slint: booking filter, inspector selection,
//! privacy masking and SEPA transfer input validation. Everything here is pure so it can be
//! unit-tested; `main.rs` only wires it to the window.

use girocrab_core::{Booking, SepaTransferRequest};
use std::collections::HashMap;

/// Shown instead of amounts, balances, IBANs and other personal texts in privacy mode.
pub const MASK: &str = "••••••";

/// Privacy applies when the user switched it on or while the vault is locked.
pub fn effective_privacy(toggle: bool, unlocked: bool) -> bool {
    toggle || !unlocked
}

pub fn mask(text: &str, privacy: bool) -> String {
    if privacy && !text.is_empty() {
        MASK.to_string()
    } else {
        text.to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StatusFilter {
    #[default]
    All,
    Unchecked,
    Checked,
    Expenses,
    Income,
}

impl StatusFilter {
    pub fn from_index(i: i32) -> Self {
        match i {
            1 => Self::Unchecked,
            2 => Self::Checked,
            3 => Self::Expenses,
            4 => Self::Income,
            _ => Self::All,
        }
    }
}

/// Filter bar state. Applied in memory to the loaded rows, so typing never hits the vault.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub search: String,
    pub uncategorized_only: bool,
    pub status: StatusFilter,
    /// Earliest booking date (ISO `YYYY-MM-DD`) that still passes; empty = no time limit.
    pub from_date: String,
    /// Only bookings carrying a tag with this name; empty = any.
    pub tag: String,
    /// Only bookings with at least one attached document.
    pub with_documents: bool,
}

/// ISO date `days` days before today (UTC), for the time range selector.
pub fn date_days_ago(days: i64) -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let z = secs / 86_400 - days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Start of the current pay period (15th to 14th, #565): the most recent 15th.
pub fn payday_period_start() -> String {
    let today = date_days_ago(0);
    let (y, m, d) = (
        today[0..4].parse::<i32>().unwrap_or(1970),
        today[5..7].parse::<i32>().unwrap_or(1),
        today[8..10].parse::<i32>().unwrap_or(1),
    );
    let (y, m) = if d >= 15 {
        (y, m)
    } else if m == 1 {
        (y - 1, 12)
    } else {
        (y, m - 1)
    };
    format!("{y:04}-{m:02}-15")
}

/// Time range selector: 0 all, 1 last 30 days, 2 last 90 days, 3 last year, 4 pay period.
pub fn range_from(index: i32) -> String {
    match index {
        1 => date_days_ago(30),
        2 => date_days_ago(90),
        3 => date_days_ago(365),
        4 => payday_period_start(),
        _ => String::new(),
    }
}

/// The booking fields the list and the inspector need (copied once per load).
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: i64,
    pub account_name: String,
    pub booking_date: String,
    pub value_date: String,
    pub counterparty: String,
    pub counterparty_iban: String,
    pub purpose: String,
    pub category_id: Option<i64>,
    pub category_name: String,
    pub category_color_hex: String,
    pub amount_cents: i64,
    pub currency: String,
    pub checked: bool,
    pub note: String,
    /// Bank booking type ("Lastschrift", "Kartenzahlung"); the list shows it before the purpose.
    pub booking_text: String,
    /// Domains to try for the logo, best first (empty for private persons and unknown names).
    pub logo_domains: Vec<String>,
    pub monogram: String,
    pub tint_hex: String,
    pub recurring: bool,
    /// Grouping key of its recurring series (`series_key`), "" = not groupable.
    pub series_key: String,
    /// Counts as salary (flag, signal or marked sender).
    pub salary: bool,
    /// The user's decision: `Some(true)` salary, `Some(false)` not salary, `None` automatic.
    pub salary_flag: Option<bool>,
    /// The booking looks like salary by itself (purpose code, keyword).
    pub salary_signal: bool,
    /// The tags the booking carries.
    pub tags: Vec<girocrab_core::tags::BookingTag>,
    /// End-to-end id, mandate and creditor id, one labelled line each (empty ones left out).
    pub reference: String,
    /// Classified as a transfer between own accounts / settlement (#574).
    pub transfer: bool,
    /// Attached documents (paperclip).
    pub documents: u32,
}

impl From<&Booking> for Row {
    fn from(b: &Booking) -> Self {
        Row {
            id: b.id,
            account_name: b.account_name.clone(),
            booking_date: b.booking_date.clone(),
            value_date: b.value_date.clone(),
            counterparty: if b.merchant_name.is_empty() {
                b.counterparty.clone()
            } else {
                b.merchant_name.clone()
            },
            counterparty_iban: b.counterparty_iban.clone(),
            purpose: b.purpose.clone(),
            category_id: b.category_id,
            category_name: b.category_name.clone(),
            category_color_hex: b.category_color_hex.clone(),
            amount_cents: b.amount_cents,
            currency: b.currency.clone(),
            checked: b.checked,
            note: b.note.clone(),
            booking_text: b.booking_text.clone(),
            logo_domains: if b.merchant.is_person {
                Vec::new()
            } else if b.merchant.domain_candidates.is_empty() && !b.merchant.domain.is_empty() {
                vec![b.merchant.domain.clone()]
            } else {
                b.merchant.domain_candidates.clone()
            },
            monogram: b.merchant.monogram.clone(),
            tint_hex: b.merchant.color_hex.clone(),
            recurring: !b.recurrence_key.is_empty(),
            series_key: girocrab_core::recurring::series_key(b),
            salary: b.is_salary,
            salary_flag: b.salary_flag,
            salary_signal: girocrab_core::salary::salary_signal(b),
            tags: b.tags.clone(),
            reference: [
                ("Ende-zu-Ende-Referenz", &b.end_to_end_id),
                ("Mandatsreferenz", &b.mandate_id),
                ("Gläubiger-ID", &b.creditor_id),
            ]
            .iter()
            .filter(|(_, v)| !v.is_empty() && v.as_str() != "NOTPROVIDED")
            .map(|(l, v)| format!("{l}: {v}"))
            .collect::<Vec<_>>()
            .join("\n"),
            transfer: b.flow == girocrab_core::merchants::FlowKind::Transfer,
            documents: b.document_count,
        }
    }
}

impl Filter {
    pub fn matches(&self, r: &Row) -> bool {
        if self.uncategorized_only && r.category_id.is_some() {
            return false;
        }
        if !self.from_date.is_empty() && r.booking_date.as_str() < self.from_date.as_str() {
            return false;
        }
        if self.with_documents && r.documents == 0 {
            return false;
        }
        if !self.tag.is_empty() && !r.tags.iter().any(|t| t.name == self.tag) {
            return false;
        }
        let status_ok = match self.status {
            StatusFilter::All => true,
            StatusFilter::Unchecked => !r.checked,
            StatusFilter::Checked => r.checked,
            StatusFilter::Expenses => r.amount_cents < 0,
            StatusFilter::Income => r.amount_cents > 0,
        };
        if !status_ok {
            return false;
        }
        let hay = format!(
            "{} {} {} {}",
            r.counterparty, r.purpose, r.note, r.counterparty_iban
        )
        .to_lowercase();
        self.search
            .to_lowercase()
            .split_whitespace()
            .all(|t| hay.contains(t))
    }
}

/// Indices of the rows that pass the filter, in list order.
pub fn visible(rows: &[Row], f: &Filter) -> Vec<usize> {
    rows.iter()
        .enumerate()
        .filter(|(_, r)| f.matches(r))
        .map(|(i, _)| i)
        .collect()
}

/// The selection survives a filter change or reload only while its booking is still shown.
pub fn keep_selection(rows: &[Row], shown: &[usize], selected: Option<i64>) -> Option<i64> {
    let id = selected?;
    shown.iter().any(|&i| rows[i].id == id).then_some(id)
}

/// Income and expenses of the ticked bookings in one currency.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CurrencySum {
    pub currency: String,
    pub income: i64,
    /// Zero or negative.
    pub expense: i64,
}

impl CurrencySum {
    pub fn balance(&self) -> i64 {
        self.income + self.expense
    }
}

/// Sums of the ticked bookings (#878). Currencies are never added up (#569): EUR is the main
/// total, every other currency stays separate in `extra`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SelectionSummary {
    pub count: usize,
    pub main: CurrencySum,
    /// Other currencies, sorted by code.
    pub extra: Vec<CurrencySum>,
}

/// Sums the given (already loaded) rows; no vault access.
pub fn selection_summary<'a>(rows: impl IntoIterator<Item = &'a Row>) -> SelectionSummary {
    let mut s = SelectionSummary {
        main: CurrencySum {
            currency: "EUR".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    for r in rows {
        s.count += 1;
        let code = if r.currency.is_empty() {
            "EUR"
        } else {
            r.currency.as_str()
        };
        let slot = if code == "EUR" {
            &mut s.main
        } else if let Some(i) = s.extra.iter().position(|c| c.currency == code) {
            &mut s.extra[i]
        } else {
            s.extra.push(CurrencySum {
                currency: code.into(),
                ..Default::default()
            });
            let last = s.extra.len() - 1;
            &mut s.extra[last]
        };
        if r.amount_cents >= 0 {
            slot.income += r.amount_cents;
        } else {
            slot.expense += r.amount_cents;
        }
    }
    s.extra.sort_by(|a, b| a.currency.cmp(&b.currency));
    s
}

/// What the selection bar shows; amounts masked in privacy mode, the count stays visible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionTexts {
    pub count: String,
    pub income: String,
    pub expense: String,
    pub balance: String,
    /// `Zusätzlich: …` with the balance per other currency, "" when there is none.
    pub extra: String,
}

pub fn selection_texts(
    s: &SelectionSummary,
    privacy: bool,
    money: impl Fn(i64, &str) -> String,
) -> SelectionTexts {
    let m = |cents: i64, cur: &str| mask(&money(cents, cur), privacy);
    let extra = if s.extra.is_empty() {
        String::new()
    } else {
        let parts: Vec<String> = s
            .extra
            .iter()
            .map(|c| {
                if privacy {
                    format!("{} {MASK}", c.currency)
                } else {
                    money(c.balance(), &c.currency)
                }
            })
            .collect();
        format!("Zusätzlich: {}", parts.join(" · "))
    };
    SelectionTexts {
        count: format!("{} Buchungen", s.count),
        income: m(s.main.income, "EUR"),
        expense: m(s.main.expense, "EUR"),
        balance: m(s.main.balance(), "EUR"),
        extra,
    }
}

/// Running balance per booking and end-of-day balance of a one-account view (#577).
/// Empty (`currency` == "") for several accounts, groups, categories or mixed currencies,
/// so balances are never summed across currencies (#569).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Balances {
    /// The one currency of all loaded rows; empty = no balance column.
    pub currency: String,
    /// Booking id → balance after that booking, in cents.
    pub running: HashMap<i64, i64>,
    /// ISO day (`YYYY-MM-DD`) → balance at the end of that day, in cents.
    pub days: HashMap<String, i64>,
}

impl Balances {
    /// Keeps the core balances only for one account whose rows all share one currency.
    pub fn for_scope(
        single_account: bool,
        rows: &[Row],
        running: HashMap<i64, i64>,
        days: HashMap<String, i64>,
    ) -> Self {
        let mut currencies = rows.iter().map(|r| r.currency.as_str());
        let first = currencies.next().unwrap_or_default();
        if !single_account || first.is_empty() || currencies.any(|c| c != first) {
            return Self::default();
        }
        Self {
            currency: first.to_string(),
            running,
            days,
        }
    }

    /// Search hits are a cut-out of the account: the column stays empty for them.
    fn active(&self, filter: &Filter) -> bool {
        !self.currency.is_empty() && filter.search.trim().is_empty()
    }

    fn text(
        &self,
        filter: &Filter,
        cents: Option<&i64>,
        privacy: bool,
        money: impl Fn(i64, &str) -> String,
    ) -> String {
        match cents {
            Some(&c) if self.active(filter) => mask(&money(c, &self.currency), privacy),
            _ => String::new(),
        }
    }

    /// Balance after booking `id`, masked in privacy mode; "" when the column stays empty.
    pub fn row(
        &self,
        filter: &Filter,
        id: i64,
        privacy: bool,
        money: impl Fn(i64, &str) -> String,
    ) -> String {
        self.text(filter, self.running.get(&id), privacy, money)
    }

    /// End-of-day balance for the day header, masked in privacy mode; "" when empty.
    pub fn day(
        &self,
        filter: &Filter,
        date: &str,
        privacy: bool,
        money: impl Fn(i64, &str) -> String,
    ) -> String {
        self.text(filter, self.days.get(date), privacy, money)
    }
}

/// What the inspector shows for a booking, already masked.
#[derive(Debug, Clone, PartialEq)]
pub struct Inspector {
    pub id: i64,
    pub counterparty: String,
    pub iban: String,
    pub purpose: String,
    pub booking_date: String,
    pub value_date: String,
    pub account: String,
    pub amount: String,
    pub note: String,
    pub checked: bool,
    pub category_id: Option<i64>,
    /// Personal data hidden: the inspector is read-only.
    pub read_only: bool,
    pub recurring: bool,
    pub reference: String,
    pub transfer_reason: String,
    pub booking_text: String,
}

pub fn inspect(r: &Row, privacy: bool, money: impl Fn(i64, &str) -> String) -> Inspector {
    Inspector {
        id: r.id,
        counterparty: mask(&r.counterparty, privacy),
        iban: mask(&r.counterparty_iban, privacy),
        purpose: mask(&r.purpose, privacy),
        booking_date: r.booking_date.clone(),
        value_date: if r.value_date.is_empty() {
            r.booking_date.clone()
        } else {
            r.value_date.clone()
        },
        account: r.account_name.clone(),
        amount: mask(&money(r.amount_cents, &r.currency), privacy),
        note: mask(&r.note, privacy),
        checked: r.checked,
        category_id: r.category_id,
        read_only: privacy,
        recurring: r.recurring,
        reference: mask(&r.reference, privacy),
        transfer_reason: if r.transfer {
            transfer_reason(r)
        } else {
            String::new()
        },
        booking_text: r.booking_text.clone(),
    }
}

/// IBAN in blocks of four ("DE02 1203 …").
pub fn format_iban(iban: &str) -> String {
    let c: Vec<char> = iban.chars().filter(|c| !c.is_whitespace()).collect();
    c.chunks(4)
        .map(|b| b.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Header subtitle of an account: bank name and formatted IBAN (IBAN masked in privacy mode).
pub fn account_subtitle(bank: &str, iban: &str, privacy: bool) -> String {
    let iban = if iban.trim().is_empty() {
        String::new()
    } else {
        mask(&format_iban(iban), privacy)
    };
    [bank.trim().to_string(), iban]
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Bank line below an IBAN field (#1563): bank name of a German IBAN, empty when unknown.
/// Masked in privacy mode, because the bank narrows down the IBAN. Never fills in the BIC.
pub fn iban_bank_line(iban: &str, privacy: bool) -> String {
    let compact: String = iban.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.is_empty() {
        return String::new();
    }
    bank_line_text(
        girocrab_core::connectors::fints_directory::bank_name_for_iban(compact),
        privacy,
    )
}

/// Display text of a looked-up bank name (empty, trimmed, masked in privacy mode).
fn bank_line_text(name: Option<String>, privacy: bool) -> String {
    match name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()) {
        Some(n) => mask(&n, privacy),
        None => String::new(),
    }
}

#[cfg(test)]
mod bank_line_tests {
    use super::*;

    #[test]
    fn iban_bank_line_is_empty_for_blank_input() {
        assert_eq!(iban_bank_line("", false), "");
        assert_eq!(iban_bank_line("   \t ", true), "");
    }

    #[test]
    fn iban_bank_line_ignores_whitespace_in_unknown_iban() {
        assert_eq!(iban_bank_line("XX00 0000 0000", false), "");
        assert_eq!(iban_bank_line("XX00 0000 0000", true), "");
    }

    #[test]
    fn shows_bank_name_trimmed() {
        assert_eq!(
            bank_line_text(Some("  Sparkasse Test ".into()), false),
            "Sparkasse Test"
        );
    }

    #[test]
    fn unknown_or_blank_bank_gives_empty_line() {
        assert_eq!(bank_line_text(None, false), "");
        assert_eq!(bank_line_text(Some("   ".into()), true), "");
    }

    #[test]
    fn privacy_mode_masks_bank_name() {
        let shown = bank_line_text(Some("Sparkasse Test".into()), true);
        assert_ne!(shown, "Sparkasse Test");
        assert_eq!(shown, mask("Sparkasse Test", true));
    }

    #[test]
    fn empty_and_foreign_ibans_have_no_bank_line() {
        assert_eq!(iban_bank_line("", false), "");
        assert_eq!(iban_bank_line("   ", true), "");
        // Only German IBANs are resolved (core returns None otherwise).
        assert_eq!(iban_bank_line("AT611904300234573201", false), "");
    }
}

/// Why a booking counts as transfer, from its own texts.
pub fn transfer_reason(r: &Row) -> String {
    let t = format!("{} {} {}", r.counterparty, r.purpose, r.booking_text).to_lowercase();
    let why = if t.contains("kreditkarte") || t.contains("abrechnung") || t.contains("visa") {
        "Kartenabrechnung"
    } else if t.contains("aufladung") || t.contains("paypal") {
        "Aufladung / Zahlungsdienst"
    } else if t.contains("depot") || t.contains("sparplan") {
        "Übertrag auf ein Depot"
    } else {
        "Zahlung zwischen eigenen Konten"
    };
    format!("Als Umbuchung erkannt ({why}): zählt nicht als Einnahme oder Ausgabe.")
}

/// Raw form input of the transfer dialog.
#[derive(Debug, Clone, Default)]
pub struct TransferInput {
    pub account_id: i64,
    pub name: String,
    pub iban: String,
    pub amount: String,
    pub purpose: String,
    /// Echtzeitüberweisung.
    pub instant: bool,
    /// Wunschtermin (`TT.MM.JJJJ` or `JJJJ-MM-TT`), empty = sofort.
    pub date: String,
}

/// Normalises a desired execution date to ISO; `Ok(None)` for empty input.
pub fn parse_execution_date(text: &str, today: &str) -> Result<Option<String>, String> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(None);
    }
    let bad = || "Das Datum ist ungültig (TT.MM.JJJJ).".to_string();
    let (y, m, d) = if let Some((y, rest)) = t.split_once('-') {
        let (m, d) = rest.split_once('-').ok_or_else(bad)?;
        (y, m, d)
    } else {
        let mut it = t.split('.');
        let (d, m, y) = (it.next(), it.next(), it.next());
        match (d, m, y, it.next()) {
            (Some(d), Some(m), Some(y), None) => (y, m, d),
            _ => return Err(bad()),
        }
    };
    let (y, m, d): (i32, u32, u32) = (
        y.parse().map_err(|_| bad())?,
        m.parse().map_err(|_| bad())?,
        d.parse().map_err(|_| bad())?,
    );
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || y < 2000 {
        return Err(bad());
    }
    let iso = format!("{y:04}-{m:02}-{d:02}");
    if iso.as_str() < today {
        return Err("Der Ausführungstermin liegt in der Vergangenheit.".into());
    }
    Ok(Some(iso))
}

/// Checks the form like the core does, but with messages next to the right field and without
/// contacting anything. Returns the request for `begin_transfer`.
pub fn validate_transfer(i: &TransferInput) -> Result<SepaTransferRequest, String> {
    if i.account_id <= 0 {
        return Err("Bitte ein Absenderkonto wählen.".into());
    }
    let name = i.name.trim();
    if name.is_empty() {
        return Err("Der Empfängername fehlt.".into());
    }
    if name.chars().count() > 70 {
        return Err("Der Empfängername darf höchstens 70 Zeichen lang sein.".into());
    }
    if !girocrab_core::connectors::validate_iban(i.iban.clone()) {
        return Err("Die Empfänger-IBAN ist ungültig.".into());
    }
    let cents = girocrab_core::parse_amount_to_cents(i.amount.clone())
        .map_err(|_| "Der Betrag ist ungültig (z. B. 12,50).".to_string())?;
    if cents <= 0 {
        return Err("Der Betrag muss größer als 0 sein.".into());
    }
    let purpose = i.purpose.trim();
    if purpose.chars().count() > 140 {
        return Err("Der Verwendungszweck darf höchstens 140 Zeichen lang sein.".into());
    }
    let execution_date = parse_execution_date(&i.date, &crate::portfolio::today_iso())?;
    if i.instant && execution_date.is_some() {
        return Err(
            "Eine Echtzeitüberweisung wird sofort ausgeführt – bitte kein Datum angeben.".into(),
        );
    }
    Ok(SepaTransferRequest {
        account_id: i.account_id,
        recipient_name: name.to_string(),
        recipient_iban: i.iban.split_whitespace().collect::<String>().to_uppercase(),
        recipient_bic: None,
        amount_cents: cents,
        purpose: purpose.to_string(),
        instant: i.instant,
        execution_date,
    })
}

/// Import kind by file extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportKind {
    /// Encrypted MoneyMoney database (needs its password).
    MoneyMoneyDb,
    /// CSV / CAMT XML / MT940 export.
    Statement,
}

pub fn import_kind(path: &std::path::Path) -> Option<ImportKind> {
    match path.extension()?.to_str()?.to_lowercase().as_str() {
        "moneymoney" => Some(ImportKind::MoneyMoneyDb),
        "csv" | "xml" | "sta" | "mt940" | "txt" => Some(ImportKind::Statement),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64, who: &str, purpose: &str, cents: i64, cat: Option<i64>, checked: bool) -> Row {
        Row {
            id,
            account_name: "Giro".into(),
            booking_date: "2026-11-01".into(),
            value_date: String::new(),
            counterparty: who.into(),
            counterparty_iban: "DE89370400440532013000".into(),
            purpose: purpose.into(),
            category_id: cat,
            category_name: String::new(),
            category_color_hex: String::new(),
            amount_cents: cents,
            currency: "EUR".into(),
            checked,
            note: String::new(),
            booking_text: String::new(),
            logo_domains: Vec::new(),
            monogram: String::new(),
            tint_hex: String::new(),
            recurring: false,
            series_key: String::new(),
            salary: false,
            salary_flag: None,
            salary_signal: false,
            tags: Vec::new(),
            reference: String::new(),
            transfer: false,
            documents: 0,
        }
    }

    fn fmt(cents: i64, cur: &str) -> String {
        format!("{cents} {cur}")
    }

    fn balance_maps() -> (HashMap<i64, i64>, HashMap<String, i64>) {
        (
            HashMap::from([(1, 10_000), (2, 7_650)]),
            HashMap::from([("2026-11-01".to_string(), 7_650)]),
        )
    }

    #[test]
    fn balances_map_onto_rows_and_day_headers() {
        let rows = vec![
            row(1, "A", "", -2350, None, false),
            row(2, "B", "", 100, None, false),
        ];
        let (running, days) = balance_maps();
        let b = Balances::for_scope(true, &rows, running, days);
        let f = Filter::default();
        assert_eq!(b.currency, "EUR");
        assert_eq!(b.row(&f, 1, false, fmt), "10000 EUR");
        assert_eq!(b.row(&f, 2, false, fmt), "7650 EUR");
        // A booking without a core balance (beyond the page) stays empty.
        assert_eq!(b.row(&f, 99, false, fmt), "");
        assert_eq!(b.day(&f, "2026-11-01", false, fmt), "7650 EUR");
        assert_eq!(b.day(&f, "2026-10-31", false, fmt), "");
    }

    #[test]
    fn balances_stay_empty_for_several_accounts_or_currencies() {
        let f = Filter::default();
        let rows = vec![
            row(1, "A", "", -2350, None, false),
            row(2, "B", "", 100, None, false),
        ];
        let (running, days) = balance_maps();
        let several = Balances::for_scope(false, &rows, running.clone(), days.clone());
        assert_eq!(several, Balances::default());
        assert_eq!(several.row(&f, 1, false, fmt), "");

        let mut mixed = rows;
        mixed[1].currency = "CHF".into();
        let mixed = Balances::for_scope(true, &mixed, running, days);
        assert!(mixed.currency.is_empty());
        assert_eq!(mixed.day(&f, "2026-11-01", false, fmt), "");
    }

    #[test]
    fn balances_hidden_for_search_and_masked_in_privacy() {
        let rows = vec![
            row(1, "A", "", -2350, None, false),
            row(2, "B", "", 100, None, false),
        ];
        let (running, days) = balance_maps();
        let b = Balances::for_scope(true, &rows, running, days);
        let search = Filter {
            search: "rewe".into(),
            ..Filter::default()
        };
        assert_eq!(b.row(&search, 1, false, fmt), "");
        assert_eq!(b.day(&search, "2026-11-01", false, fmt), "");
        let f = Filter::default();
        assert_eq!(b.row(&f, 1, true, fmt), MASK);
        assert_eq!(b.day(&f, "2026-11-01", true, fmt), MASK);
        // An empty column stays empty in privacy mode (no stray mask).
        assert_eq!(b.row(&f, 99, true, fmt), "");
    }

    fn sample() -> Vec<Row> {
        vec![
            row(
                1,
                "REWE Markt",
                "Einkauf Lebensmittel",
                -2350,
                Some(5),
                true,
            ),
            row(
                2,
                "Arbeitgeber GmbH",
                "Gehalt November",
                300000,
                None,
                false,
            ),
            row(3, "Stadtwerke", "Strom Abschlag", -8900, None, false),
        ]
    }

    fn row_in(id: i64, cents: i64, currency: &str) -> Row {
        Row {
            currency: currency.into(),
            ..row(id, "X", "", cents, None, false)
        }
    }

    fn plain(cents: i64, cur: &str) -> String {
        format!("{cents} {cur}")
    }

    #[test]
    fn selection_summary_splits_income_and_expenses() {
        let s = selection_summary(&sample());
        assert_eq!(s.count, 3);
        assert_eq!(s.main.currency, "EUR");
        assert_eq!(s.main.income, 300000);
        assert_eq!(s.main.expense, -11250);
        assert_eq!(s.main.balance(), 288750);
        assert!(s.extra.is_empty());
    }

    #[test]
    fn selection_summary_never_adds_currencies() {
        let rows = vec![
            row_in(1, 1000, "EUR"),
            row_in(2, 500, "USD"),
            row_in(3, -200, "USD"),
            row_in(4, -300, "CHF"),
            row_in(5, 100, ""),
        ];
        let s = selection_summary(&rows);
        assert_eq!(s.count, 5);
        assert_eq!((s.main.income, s.main.expense), (1100, 0));
        let codes: Vec<&str> = s.extra.iter().map(|c| c.currency.as_str()).collect();
        assert_eq!(codes, ["CHF", "USD"]);
        assert_eq!((s.extra[0].income, s.extra[0].expense), (0, -300));
        assert_eq!((s.extra[1].income, s.extra[1].expense), (500, -200));
        let t = selection_texts(&s, false, plain);
        assert_eq!(t.balance, "1100 EUR");
        assert_eq!(t.extra, "Zusätzlich: -300 CHF · 300 USD");
    }

    #[test]
    fn selection_summary_of_nothing_is_zero() {
        let s = selection_summary(&[] as &[Row]);
        assert_eq!(s.count, 0);
        assert_eq!(s.main.balance(), 0);
        assert!(s.extra.is_empty());
        let t = selection_texts(&s, false, plain);
        assert_eq!((t.count.as_str(), t.extra.as_str()), ("0 Buchungen", ""));
    }

    #[test]
    fn selection_texts_mask_amounts_in_privacy_mode() {
        let rows = vec![row_in(1, -2350, "EUR"), row_in(2, 900, "USD")];
        let t = selection_texts(&selection_summary(&rows), true, plain);
        assert_eq!(t.count, "2 Buchungen");
        assert_eq!(t.income, MASK);
        assert_eq!(t.expense, MASK);
        assert_eq!(t.balance, MASK);
        assert_eq!(t.extra, format!("Zusätzlich: USD {MASK}"));
    }

    #[test]
    fn search_matches_all_tokens_case_insensitive() {
        let rows = sample();
        let f = Filter {
            search: "rewe LEBENS".into(),
            ..Default::default()
        };
        assert_eq!(visible(&rows, &f), vec![0]);
        let f = Filter {
            search: "rewe gehalt".into(),
            ..Default::default()
        };
        assert!(visible(&rows, &f).is_empty());
    }

    #[test]
    fn uncategorized_and_status_filters_combine() {
        let rows = sample();
        let f = Filter {
            uncategorized_only: true,
            status: StatusFilter::Expenses,
            ..Default::default()
        };
        assert_eq!(visible(&rows, &f), vec![2]);
        let f = Filter {
            status: StatusFilter::Unchecked,
            ..Default::default()
        };
        assert_eq!(visible(&rows, &f), vec![1, 2]);
        assert_eq!(StatusFilter::from_index(4), StatusFilter::Income);
        assert_eq!(StatusFilter::from_index(99), StatusFilter::All);
    }

    #[test]
    fn selection_is_dropped_when_filtered_out() {
        let rows = sample();
        let f = Filter {
            search: "strom".into(),
            ..Default::default()
        };
        let shown = visible(&rows, &f);
        assert_eq!(keep_selection(&rows, &shown, Some(3)), Some(3));
        assert_eq!(keep_selection(&rows, &shown, Some(1)), None);
        assert_eq!(keep_selection(&rows, &shown, None), None);
    }

    #[test]
    fn privacy_masks_inspector_and_makes_it_read_only() {
        let mut r = row(2, "Arbeitgeber GmbH", "Gehalt", 300000, None, false);
        r.note = "privat".into();
        let money = |c: i64, cur: &str| format!("{c} {cur}");
        let open = inspect(&r, false, money);
        assert_eq!(open.amount, "300000 EUR");
        assert_eq!(open.value_date, "2026-11-01");
        assert!(!open.read_only);
        let hidden = inspect(&r, true, money);
        for s in [
            &hidden.amount,
            &hidden.counterparty,
            &hidden.iban,
            &hidden.purpose,
            &hidden.note,
        ] {
            assert_eq!(s, MASK);
        }
        assert!(hidden.read_only);
        // Dates stay visible; empty values are not turned into a mask.
        assert_eq!(hidden.booking_date, "2026-11-01");
        assert_eq!(mask("", true), "");
    }

    #[test]
    fn privacy_is_on_while_locked() {
        assert!(effective_privacy(false, false));
        assert!(effective_privacy(true, true));
        assert!(!effective_privacy(false, true));
    }

    fn input() -> TransferInput {
        TransferInput {
            account_id: 1,
            name: " Erika Muster ".into(),
            iban: "de89 3704 0044 0532 0130 00".into(),
            amount: "12,50".into(),
            purpose: "Miete".into(),
            instant: false,
            date: String::new(),
        }
    }

    #[test]
    fn valid_transfer_is_normalised() {
        let r = validate_transfer(&input()).unwrap();
        assert_eq!(r.recipient_name, "Erika Muster");
        assert_eq!(r.recipient_iban, "DE89370400440532013000");
        assert_eq!(r.amount_cents, 1250);
        assert!(!r.instant && r.execution_date.is_none());
    }

    #[test]
    fn execution_date_formats() {
        assert_eq!(parse_execution_date("", "2026-01-01"), Ok(None));
        assert_eq!(
            parse_execution_date("15.03.2027", "2026-01-01"),
            Ok(Some("2027-03-15".into()))
        );
        assert!(parse_execution_date("2020-01-01", "2026-01-01").is_err());
        assert!(parse_execution_date("32.13.2027", "2026-01-01").is_err());
    }

    #[test]
    fn invalid_transfer_inputs_are_rejected() {
        let bad = |f: &dyn Fn(&mut TransferInput)| {
            let mut i = input();
            f(&mut i);
            validate_transfer(&i).unwrap_err()
        };
        assert!(bad(&|i| i.account_id = 0).contains("Absender"));
        assert!(bad(&|i| i.name = "  ".into()).contains("Empfängername"));
        assert!(bad(&|i| i.iban = "DE89 3704 0044 0532 0130 01".into()).contains("IBAN"));
        assert!(bad(&|i| i.amount = "abc".into()).contains("Betrag"));
        assert!(bad(&|i| i.amount = "0,00".into()).contains("größer als 0"));
        assert!(bad(&|i| i.amount = "-5".into()).contains("größer als 0"));
        assert!(bad(&|i| i.purpose = "x".repeat(141)).contains("140"));
    }

    #[test]
    fn import_kind_by_extension() {
        use std::path::Path;
        assert_eq!(
            import_kind(Path::new("a.MoneyMoney")),
            Some(ImportKind::MoneyMoneyDb)
        );
        assert_eq!(
            import_kind(Path::new("x/umsaetze.csv")),
            Some(ImportKind::Statement)
        );
        assert_eq!(
            import_kind(Path::new("camt.xml")),
            Some(ImportKind::Statement)
        );
        assert_eq!(import_kind(Path::new("photo.png")), None);
        assert_eq!(import_kind(Path::new("noext")), None);
    }
}
