//! Prüfansicht der Sammelüberweisung (CSV): mögliche Doppelte, Zeilenauswahl,
//! Summe je Währung (#569). Reine Logik plus Zustand des UI-Threads, kein Vault-Zugriff.

use crate::mandates_view::{Line, batch_line};
use girocrab_core::batch_csv::{BatchCsvImport, BatchCsvRow};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

/// `BatchCsvRow` trägt keine Währung: SEPA-Sammelaufträge sind in EUR.
pub const CURRENCY: &str = "EUR";
const MASK: &str = "•••";

pub type Fmt<'a> = &'a dyn Fn(i64, &str) -> String;

pub struct Review {
    pub imp: BatchCsvImport,
    pub selected: Vec<bool>,
}

thread_local! {
    static REVIEW: RefCell<Option<Review>> = const { RefCell::new(None) };
}

impl Review {
    /// Fehlerfreie Zeilen sind vorgewählt, fehlerhafte nie.
    pub fn new(imp: BatchCsvImport) -> Self {
        let selected = imp.rows.iter().map(|r| r.errors.is_empty()).collect();
        Self { imp, selected }
    }

    /// Schaltet eine fehlerfreie Zeile um; `false`, wenn nichts geändert wurde.
    pub fn toggle(&mut self, i: usize) -> bool {
        match self.imp.rows.get(i) {
            Some(r) if r.errors.is_empty() => {
                self.selected[i] = !self.selected[i];
                true
            }
            _ => false,
        }
    }

    /// Summe der ausgewählten Zeilen, je Währung getrennt.
    pub fn totals(&self) -> BTreeMap<String, i64> {
        sum_by_currency(
            self.imp
                .rows
                .iter()
                .zip(&self.selected)
                .filter(|(_, sel)| **sel)
                .map(|(r, _)| (CURRENCY, r.amount_cents)),
        )
    }
}

pub fn start(imp: BatchCsvImport) {
    REVIEW.with(|s| *s.borrow_mut() = Some(Review::new(imp)));
}

pub fn clear() {
    REVIEW.with(|s| *s.borrow_mut() = None);
}

pub fn toggle(i: usize) -> bool {
    REVIEW.with(|s| s.borrow_mut().as_mut().is_some_and(|r| r.toggle(i)))
}

pub fn with<R>(f: impl FnOnce(&Review) -> R) -> Option<R> {
    REVIEW.with(|s| s.borrow().as_ref().map(f))
}

/// Beträge nie über Währungen hinweg addieren (#569).
pub fn sum_by_currency<'a>(
    items: impl IntoIterator<Item = (&'a str, i64)>,
) -> BTreeMap<String, i64> {
    let mut m = BTreeMap::new();
    for (cur, cents) in items {
        *m.entry(cur.to_string()).or_insert(0) += cents;
    }
    m
}

fn key(r: &BatchCsvRow) -> String {
    let iban: String = r
        .iban
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_uppercase();
    format!("{iban}|{}|{}", r.amount_cents, r.purpose.trim())
}

/// Markiert alle Zeilen (auch fehlerhafte), deren IBAN, Betrag und Zweck mehrfach vorkommen.
pub fn duplicate_flags(rows: &[BatchCsvRow]) -> Vec<bool> {
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, r) in rows.iter().enumerate() {
        groups.entry(key(r)).or_default().push(i);
    }
    let mut flags = vec![false; rows.len()];
    for i in groups.values().filter(|g| g.len() > 1).flatten() {
        flags[*i] = true;
    }
    flags
}

/// Anzahl möglicher Doppelter wie in der Referenz: je Gruppe gleicher Zeilen alle
/// Wiederholungen, die erste Zeile zählt nicht mit. Fehlerhafte Zeilen zählen mit.
pub fn duplicate_count(rows: &[BatchCsvRow]) -> usize {
    repeats(rows.iter().map(key))
}

/// Zählt Schlüssel, die schon einmal vorkamen (Gruppengröße − 1 je Gruppe).
pub(super) fn repeats(keys: impl IntoIterator<Item = String>) -> usize {
    let mut seen = std::collections::HashSet::new();
    keys.into_iter().filter(|k| !seen.insert(k.clone())).count()
}

fn totals_text(t: &BTreeMap<String, i64>, fmt: Fmt) -> String {
    if t.is_empty() {
        return fmt(0, CURRENCY);
    }
    t.iter()
        .map(|(cur, c)| fmt(*c, cur))
        .collect::<Vec<_>>()
        .join("; ")
}

pub fn summary(r: &Review, private: bool, fmt: Fmt) -> String {
    let bad = r.imp.rows.iter().filter(|x| !x.errors.is_empty()).count();
    let sel = r.selected.iter().filter(|s| **s).count();
    let sum = if private {
        MASK.to_string()
    } else {
        totals_text(&r.totals(), fmt)
    };
    let mut s = format!(
        "{} Zeilen, {sel} ausgewählt (Summe {sum}), {bad} fehlerhaft.",
        r.imp.rows.len()
    );
    let d = duplicate_count(&r.imp.rows);
    if d > 0 {
        s.push_str(&format!(
            " {d} mögliche Doppelte (gleiche IBAN, Betrag und Zweck) – bitte prüfen."
        ));
    }
    for p in &r.imp.problems {
        s.push(' ');
        s.push_str(p);
    }
    s
}

pub fn lines(r: &Review, private: bool, fmt: Fmt) -> Vec<Line> {
    let eur = |c: i64| fmt(c, CURRENCY);
    let dups = duplicate_flags(&r.imp.rows);
    r.imp
        .rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let mut l = batch_line(row, &eur);
            l.checked = r.selected[i];
            if private {
                l.title = format!("Zeile {}", row.line);
                l.amount = MASK.into();
                if row.errors.is_empty() {
                    l.detail = MASK.into();
                }
            }
            if dups[i] {
                l.detail = format!("Mögliches Doppelt · {}", l.detail);
            }
            l
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use girocrab_core::batch_csv::parse_batch_csv;

    fn fmt(c: i64, cur: &str) -> String {
        format!("{},{:02} {cur}", c / 100, c % 100)
    }

    fn review() -> Review {
        Review::new(parse_batch_csv(
            "Name;IBAN;Betrag;Zweck\n\
             Erika;DE02120300000000202051;12,50;Miete\n\
             Erika;DE02120300000000202051;12,50;Miete\n\
             Erika;DE02120300000000202051;12,50;Mai\n\
             Bob;XX;abc;x\n"
                .into(),
        ))
    }

    #[test]
    fn duplicates_need_same_iban_amount_and_purpose() {
        let r = review();
        assert_eq!(duplicate_flags(&r.imp.rows), [true, true, false, false]);
        assert_eq!(duplicate_count(&r.imp.rows), 1);
        assert!(summary(&r, false, &fmt).contains("1 mögliche Doppelte"));
    }

    #[test]
    fn totals_never_mix_currencies() {
        let t = sum_by_currency([("EUR", 1250), ("CHF", 500), ("EUR", 50)]);
        assert_eq!(t.len(), 2);
        assert_eq!(t["EUR"], 1300);
        assert_eq!(t["CHF"], 500);
    }

    #[test]
    fn deselecting_changes_totals_and_faulty_rows_stay_out() {
        let mut r = review();
        assert_eq!(r.totals()["EUR"], 3750);
        assert!(r.toggle(1));
        assert_eq!(r.totals()["EUR"], 2500);
        assert!(!r.toggle(3), "fehlerhafte Zeile nicht wählbar");
        assert!(!r.toggle(99));
        assert!(summary(&r, false, &fmt).starts_with("4 Zeilen, 2 ausgewählt"));
    }

    #[test]
    fn private_mode_hides_personal_data_and_amounts() {
        let r = review();
        let s = summary(&r, true, &fmt);
        assert!(!s.contains("12,50") && !s.contains("37,50"));
        for l in lines(&r, true, &fmt) {
            let all = format!("{} {} {}", l.title, l.detail, l.amount);
            assert!(!all.contains("DE02") && !all.contains("Erika") && !all.contains("12,50"));
        }
    }
}
