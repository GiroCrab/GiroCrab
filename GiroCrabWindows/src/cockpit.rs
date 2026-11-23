//! Pure logic of the macOS-style cockpit above and inside the booking list: day grouping with
//! German weekday titles, the cashflow chart scale and the budget / top-spending lines.
//! No Slint here, so everything is unit-tested; `main.rs` maps the results to the window.

use girocrab_core::projection::CategoryFlowTotal;
use girocrab_core::{CategoryBudgetStatus, PeriodCashflow};

use crate::daily::Row;
use crate::portfolio::fmt_date;

const WEEKDAYS: [&str; 7] = [
    "Montag",
    "Dienstag",
    "Mittwoch",
    "Donnerstag",
    "Freitag",
    "Samstag",
    "Sonntag",
];
const MONTHS: [&str; 12] = [
    "Januar",
    "Februar",
    "März",
    "April",
    "Mai",
    "Juni",
    "Juli",
    "August",
    "September",
    "Oktober",
    "November",
    "Dezember",
];
const MONTHS_SHORT: [&str; 12] = [
    "Jan", "Feb", "Mär", "Apr", "Mai", "Jun", "Jul", "Aug", "Sep", "Okt", "Nov", "Dez",
];

/// `2026-11-13` (optionally followed by a time) -> (2026, 11, 13).
pub fn parse_iso(iso: &str) -> Option<(i32, u32, u32)> {
    let head = iso.get(..10)?;
    let mut it = head.split('-');
    let y: i32 = it.next()?.parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let d: u32 = it.next()?.parse().ok()?;
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then_some((y, m, d))
}

/// Days since 1970-01-01 (proleptic Gregorian, Howard Hinnant's algorithm).
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = i64::from(if m <= 2 { y - 1 } else { y });
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = i64::from((m + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = (yoe + era * 400) as i32 + i32::from(m <= 2);
    (y, m, d)
}

/// ISO date `n` days after today (UTC).
pub fn today_plus_days(n: i64) -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400) + n);
    fmt_date(y, m, d)
}

pub fn period_month(key: &str) -> Option<(i32, u32)> {
    let (year, month) = key.split_once('-')?;
    let (year, month) = (year.parse().ok()?, month.parse().ok()?);
    (key.len() == 7 && (1..=12).contains(&month)).then_some((year, month))
}

pub fn effective_account_currency(currency: &str) -> String {
    let currency = currency.trim().to_ascii_uppercase();
    if currency.is_empty() {
        "EUR".into()
    } else {
        currency
    }
}

pub fn current_year_month(vault: &girocrab_core::Vault) -> Result<(i32, u32), String> {
    vault
        .cashflow_by_period(girocrab_core::PeriodGranularity::Month, 1, None, false)
        .map_err(|e| e.to_string())?
        .last()
        .and_then(|p| period_month(&p.key))
        .ok_or_else(|| "Der Berichtsmonat konnte nicht ermittelt werden.".into())
}

/// `2026-11-13` -> "Freitag, 13. November"; unparsable input is returned unchanged.
pub fn day_title(iso: &str) -> String {
    let Some((y, m, d)) = parse_iso(iso) else {
        return iso.to_string();
    };
    let wd = (days_from_civil(y, m, d) + 3).rem_euclid(7) as usize;
    format!("{}, {d}. {}", WEEKDAYS[wd], MONTHS[m as usize - 1])
}

/// `2026-11-13` -> "November 2026"; unparsable input is returned unchanged.
pub fn month_title(iso: &str) -> String {
    let Some((y, m, _)) = parse_iso(iso) else {
        return iso.to_string();
    };
    format!("{} {y}", MONTHS[m as usize - 1])
}

/// One day of the booking list: the indices (into the shown rows) of its bookings and the net sum.
#[derive(Debug, PartialEq)]
pub struct DayBucket {
    pub date: String,
    pub indices: Vec<usize>,
    pub sum_cents: i64,
    pub currency: String,
}

/// Proof-view bank net per day and currency (the list is sorted by date).
pub fn group_by_day(rows: &[&Row]) -> Vec<DayBucket> {
    let mut out: Vec<DayBucket> = Vec::new();
    let mut positions = std::collections::HashMap::<(String, String), usize>::new();
    for (i, r) in rows.iter().enumerate() {
        let date = r.booking_date.get(..10).unwrap_or(&r.booking_date);
        let key = (date.to_string(), r.currency.clone());
        match positions.get(&key).copied() {
            Some(index) => {
                let day = &mut out[index];
                day.indices.push(i);
                day.sum_cents += r.amount_cents;
            }
            None => {
                positions.insert(key, out.len());
                out.push(DayBucket {
                    date: date.to_string(),
                    indices: vec![i],
                    sum_cents: r.amount_cents,
                    currency: r.currency.clone(),
                });
            }
        }
    }
    out
}

/// "+964,14 €" for income days, "-47,82 €" for expense days (`money` adds the minus itself).
pub fn day_sum_text(cents: i64, money: &dyn Fn(i64, &str) -> String, currency: &str) -> String {
    let text = money(cents, currency);
    if cents > 0 { format!("+{text}") } else { text }
}

/// "2026-03" -> "Mär", "2026-Q1" -> "Q1", "2026" -> "2026".
pub fn period_label(key: &str) -> String {
    if key.contains('Q') {
        return key.rsplit('-').next().unwrap_or(key).to_string();
    }
    match key.split_once('-') {
        Some((_, m)) => m
            .parse::<usize>()
            .ok()
            .and_then(|m| MONTHS_SHORT.get(m.wrapping_sub(1)))
            .map_or_else(|| key.to_string(), |s| (*s).to_string()),
        None => key.to_string(),
    }
}

/// German number label for the chart axis: "5.000", "2.500", "0".
pub fn axis_label(cents: i64) -> String {
    let euros = (cents / 100).to_string();
    let mut out = String::new();
    for (i, c) in euros.chars().enumerate() {
        if i > 0 && (euros.len() - i).is_multiple_of(3) && !out.ends_with('-') {
            out.push('.');
        }
        out.push(c);
    }
    out
}

/// Bar colour of a budget line: green below 80 %, amber up to 100 %, red above.
pub fn budget_color_hex(progress: f32) -> &'static str {
    if progress > 1.0 {
        "#f47067"
    } else if progress >= 0.8 {
        "#d29922"
    } else {
        "#3fb950"
    }
}

/// Human-readable booking type: "KARTENZAHLUNG" -> "Kartenzahlung", "UEBERWEISUNG" -> "Überweisung".
/// Text that already has lower-case letters is kept as the bank sent it.
pub fn booking_type_label(text: &str) -> String {
    let t = text.trim();
    if t.chars().any(char::is_lowercase) {
        return t.to_string();
    }
    t.split(' ')
        .map(|w| {
            let lower = w.to_lowercase();
            let lower = match lower.strip_prefix("ueber") {
                Some(rest) => format!("über{rest}"),
                None => lower,
            };
            let mut cs = lower.chars();
            cs.next().map_or_else(String::new, |f| {
                f.to_uppercase().collect::<String>() + cs.as_str()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// One bar pair of the chart: heights as a fraction (0..=1) of the axis maximum.
#[derive(Debug, PartialEq)]
pub struct Bars {
    pub label: String,
    pub income: f32,
    pub expense: f32,
}

/// Round axis maximum (1, 2, 2.5, 5 × 10^n euros) so that four grid lines read well.
fn nice_max(max_cents: i64) -> i64 {
    let euros = (max_cents / 100).max(1);
    let mut mag = 1i64;
    while mag * 10 <= euros {
        mag *= 10;
    }
    for step in [10, 20, 25, 50, 100] {
        let candidate = mag * step / 10;
        if candidate >= euros {
            return candidate * 100;
        }
    }
    mag * 10 * 100
}

/// Bars and the three axis labels (top to bottom, 3 grid lines) of the cashflow chart.
pub fn chart(periods: &[PeriodCashflow]) -> (Vec<Bars>, Vec<String>) {
    let max = periods
        .iter()
        .map(|p| p.income_cents.max(p.expense_cents))
        .max()
        .unwrap_or(0);
    let top = nice_max(max);
    let bars = periods
        .iter()
        .map(|p| Bars {
            label: period_label(&p.key),
            income: (p.income_cents.max(0) as f32 / top as f32).min(1.0),
            expense: (p.expense_cents.max(0) as f32 / top as f32).min(1.0),
        })
        .collect();
    let ticks = (0..3).map(|i| axis_label(top - top * i / 2)).collect();
    (bars, ticks)
}

/// A line of the budget card.
#[derive(Debug, PartialEq)]
pub struct Budget {
    pub name: String,
    pub currency: String,
    pub spent_cents: i64,
    /// `None` for the "top expenses" fallback without a limit.
    pub limit_cents: Option<i64>,
    pub progress: f32,
    pub color_hex: String,
}

/// Categories with a monthly budget, most used first.
pub fn budgets(status: &[CategoryBudgetStatus]) -> Vec<Budget> {
    let mut v: Vec<Budget> = status
        .iter()
        .filter(|s| s.budget_cents > 0)
        .map(|s| Budget {
            name: s.name.clone(),
            currency: "EUR".into(),
            spent_cents: s.spent_cents,
            limit_cents: Some(s.budget_cents),
            progress: (s.spent_cents.max(0) as f64 / s.budget_cents as f64) as f32,
            color_hex: s.color_hex.clone(),
        })
        .collect();
    v.sort_by(|a, b| b.progress.total_cmp(&a.progress));
    v
}

/// Bars over core expense totals, relative to the largest one.
pub fn top_spending(totals: &[CategoryFlowTotal], n: usize, currency: &str) -> Vec<Budget> {
    let mut sums: Vec<(String, String, i64)> = totals
        .iter()
        .filter(|t| t.cents > 0)
        .map(|t| (t.name.clone(), t.color_hex.clone(), t.cents))
        .collect();
    sums.sort_by_key(|s| std::cmp::Reverse(s.2));
    sums.truncate(n);
    let max = sums.first().map_or(1, |s| s.2).max(1);
    sums.into_iter()
        .map(|(name, color_hex, spent)| Budget {
            name,
            currency: currency.into(),
            spent_cents: spent,
            limit_cents: None,
            progress: spent as f32 / max as f32,
            color_hex,
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn blank_account_currency_matches_the_core_default() {
        assert_eq!(effective_account_currency(""), "EUR");
        assert_eq!(effective_account_currency("  "), "EUR");
        assert_eq!(effective_account_currency(" usd "), "USD");
    }

    #[test]
    fn reporting_month_uses_the_local_period_and_rejects_missing_periods() {
        assert_eq!(super::period_month("2026-02"), Some((2026, 2)));
        assert_eq!(super::period_month("2025-12"), Some((2025, 12)));
        for key in ["", "2026-00", "2026-13", "2026-02-01", "not-a-period"] {
            assert_eq!(super::period_month(key), None);
        }
    }
    pub(crate) fn row(date: &str, cents: i64, cat: &str) -> Row {
        Row {
            id: 1,
            account_name: "Giro".into(),
            booking_date: date.into(),
            value_date: String::new(),
            counterparty: "X".into(),
            counterparty_iban: String::new(),
            purpose: String::new(),
            category_id: None,
            category_name: cat.into(),
            category_color_hex: "#ff0000".into(),
            amount_cents: cents,
            currency: "EUR".into(),
            checked: false,
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
            refund_linked: false,
            refund_cents: 0,
            refunded_cents: 0,
            split_count: 0,
        }
    }

    #[test]
    fn german_day_titles() {
        assert_eq!(day_title("2026-11-13"), "Freitag, 13. November");
        assert_eq!(day_title("2024-02-29"), "Donnerstag, 29. Februar");
        assert_eq!(day_title("1970-01-01"), "Donnerstag, 1. Januar");
        assert_eq!(day_title("kaputt"), "kaputt");
    }

    #[test]
    fn civil_round_trip() {
        for (y, m, d) in [(2000, 2, 29), (2026, 11, 16), (1969, 12, 31)] {
            assert_eq!(civil_from_days(days_from_civil(y, m, d)), (y, m, d));
        }
    }

    #[test]
    fn groups_consecutive_days_and_sums() {
        let rows = [
            row("2026-11-13", -4165, ""),
            row("2026-11-13", -617, ""),
            row("2026-11-12", 96414, ""),
        ];
        let refs: Vec<&Row> = rows.iter().collect();
        let days = group_by_day(&refs);
        assert_eq!(days.len(), 2);
        assert_eq!(days[0].indices, vec![0, 1]);
        assert_eq!(days[0].sum_cents, -4782);
        assert_eq!(days[1].sum_cents, 96414);
        let money = |c: i64, _: &str| format!("{c}");
        assert_eq!(day_sum_text(96414, &money, "EUR"), "+96414");
        assert_eq!(day_sum_text(-4782, &money, "EUR"), "-4782");
    }

    #[test]
    fn proof_day_nets_keep_currencies_separate() {
        let a = row("2026-11-13", -100, "A");
        let mut b = row("2026-11-13", -200, "B");
        b.currency = "USD".into();
        let c = row("2026-11-13", -300, "C");
        let days = group_by_day(&[&a, &b, &c]);
        assert_eq!(days.len(), 2);
        assert_eq!((days[0].sum_cents, days[1].sum_cents), (-400, -200));
        assert_eq!(days[1].currency, "USD");
    }

    #[test]
    fn period_labels() {
        assert_eq!(period_label("2026-03"), "Mär");
        assert_eq!(period_label("2026-Q1"), "Q1");
        assert_eq!(period_label("2026"), "2026");
    }

    #[test]
    fn chart_scale_is_round_and_bars_fit() {
        let p = |k: &str, i, e| PeriodCashflow {
            key: k.into(),
            income_cents: i,
            expense_cents: e,
        };
        let (bars, ticks) = chart(&[p("2026-10", 250_000, 100_000), p("2026-11", 0, 0)]);
        assert_eq!(ticks, ["2.500", "1.250", "0"]);
        assert!((bars[0].income - 1.0).abs() < 1e-6);
        assert!((bars[0].expense - 0.4).abs() < 1e-6);
        assert_eq!(bars[1].income, 0.0);
        assert_eq!(nice_max(0), 100);
        assert_eq!(nice_max(123_400), 200_000);
        assert_eq!(axis_label(120_000), "1.200");
        assert_eq!(axis_label(50_000_000), "500.000");
        assert_eq!(axis_label(0), "0");
    }

    #[test]
    fn cashflow_chart_presents_core_periods() {
        let (bars, _) = chart(&[PeriodCashflow {
            key: "2026-11".into(),
            income_cents: 10000,
            expense_cents: 3000,
        }]);
        assert_eq!(bars.len(), 1);
        assert!(bars[0].income > bars[0].expense);
        assert!(bars[0].expense > 0.0);
        assert!(chart(&[]).0.is_empty());
    }

    #[test]
    fn booking_types_are_title_case() {
        assert_eq!(booking_type_label("KARTENZAHLUNG"), "Kartenzahlung");
        assert_eq!(booking_type_label("ÜBERWEISUNG"), "Überweisung");
        assert_eq!(booking_type_label("UEBERWEISUNG"), "Überweisung");
        assert_eq!(booking_type_label("LASTSCHRIFT"), "Lastschrift");
        assert_eq!(booking_type_label("Dauerauftrag"), "Dauerauftrag");
    }

    #[test]
    fn budget_colours_follow_the_thresholds() {
        assert_eq!(budget_color_hex(0.25), "#3fb950");
        assert_eq!(budget_color_hex(0.8), "#d29922");
        assert_eq!(budget_color_hex(1.0), "#d29922");
        assert_eq!(budget_color_hex(1.01), "#f47067");
    }

    #[test]
    fn top_spending_is_relative_to_the_largest() {
        let rows = [
            CategoryFlowTotal {
                category_id: Some(1),
                name: "Essen".into(),
                color_hex: "#ff0000".into(),
                cents: 4500,
                booking_count: 2,
            },
            CategoryFlowTotal {
                category_id: Some(2),
                name: "Reisen".into(),
                color_hex: "#00ff00".into(),
                cents: 1500,
                booking_count: 1,
            },
        ];
        let top = top_spending(&rows, 4, "USD");
        assert_eq!(top[0].currency, "USD");
        assert_eq!(
            crate::money(top[0].spent_cents, &top[0].currency),
            "45,00 $"
        );
        assert!(top_spending(&[], 4, "USD").is_empty());
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].name, "Essen");
        assert_eq!(top[0].spent_cents, 4500);
        assert!((top[1].progress - 1500.0 / 4500.0).abs() < 1e-6);
    }
}
