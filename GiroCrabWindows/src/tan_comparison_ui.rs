//! TAN-Abgleich bei der Freigabe einer Einzelüberweisung (#949). Vorbild: Mac `TanComparisonHint.swift`
//! und `visibleTanComparison` (`AppModel+Transfer.swift`). Der Snapshot wird beim Absenden eingefroren
//! und nur angezeigt, solange die Session auf die TAN des Auftrags selbst wartet.

use std::sync::{Mutex, MutexGuard};

/// Betrag und Empfänger-IBAN, eingefroren beim Absenden des Auftrags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TanComparison {
    /// Deutsch formatierter Betrag, z. B. `1.234,56 €`.
    pub amount: String,
    /// Gruppierte IBAN ohne die letzten 4 Zeichen.
    pub prefix: String,
    /// Die letzten 4 Zeichen der IBAN (hervorgehoben).
    pub end_chars: String,
}

impl TanComparison {
    /// `None` ohne Empfänger-IBAN. Leerzeichen werden ignoriert, Buchstaben großgeschrieben.
    pub fn new(iban: &str, amount_cents: impl Into<i128>) -> Option<Self> {
        let iban: Vec<char> = iban
            .chars()
            .filter(|c| !c.is_whitespace())
            .flat_map(char::to_uppercase)
            .collect();
        if iban.is_empty() {
            return None;
        }
        let split = iban.len().saturating_sub(4);
        Some(Self {
            amount: format_eur(amount_cents.into()),
            prefix: group(&iban[..split]),
            end_chars: iban[split..].iter().collect(),
        })
    }
}

fn group(chars: &[char]) -> String {
    chars
        .chunks(4)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_eur(cents: i128) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs = cents.unsigned_abs();
    let euros = (abs / 100).to_string();
    let mut grouped = String::with_capacity(euros.len() + euros.len() / 3);
    for (i, ch) in euros.chars().enumerate() {
        if i > 0 && (euros.len() - i).is_multiple_of(3) {
            grouped.push('.');
        }
        grouped.push(ch);
    }
    format!("{sign}{grouped},{:02} €", abs % 100)
}

type Gate = Box<dyn Fn() -> bool + Send>;

struct Armed {
    comparison: TanComparison,
    gate: Gate,
}

impl Armed {
    fn visible(&self) -> Option<TanComparison> {
        (self.gate)().then(|| self.comparison.clone())
    }
}

/// Ein Platz je Überweisung, gekennzeichnet durch den Worker-Thread, der `run_transfer`
/// ausführt (dort laufen auch alle Dialog-Rückfragen). Überlappende Überweisungen
/// (Sperren, Entsperren, neue Überweisung) ersetzen oder löschen sich so nicht gegenseitig.
static ARMED: Mutex<Vec<(std::thread::ThreadId, Armed)>> = Mutex::new(Vec::new());

fn armed() -> MutexGuard<'static, Vec<(std::thread::ThreadId, Armed)>> {
    ARMED.lock().unwrap_or_else(|e| e.into_inner())
}

/// Entschärft den Abgleich beim Verlassen von `run_transfer`, auch bei Fehlern.
/// Entfernt nur den Eintrag der eigenen Überweisung.
#[must_use]
pub struct ArmGuard(std::thread::ThreadId);

impl Drop for ArmGuard {
    fn drop(&mut self) {
        armed().retain(|(id, _)| *id != self.0);
    }
}

/// Schaltet den eingefrorenen Abgleich für die laufende Überweisung dieses Threads scharf.
/// `gate` ist `TransferSession::awaits_order_approval`: wahr nur bei der TAN des Auftrags,
/// nie beim Login.
pub fn arm(
    comparison: Option<TanComparison>,
    gate: impl Fn() -> bool + Send + 'static,
) -> ArmGuard {
    let me = std::thread::current().id();
    let mut slots = armed();
    slots.retain(|(id, _)| *id != me);
    if let Some(comparison) = comparison {
        slots.push((
            me,
            Armed {
                comparison,
                gate: Box::new(gate),
            },
        ));
    }
    ArmGuard(me)
}

/// Beim Sperren des Tresors: kein Betrag und keine IBAN-Reste dürfen den Tresor überleben.
pub fn disarm_all() {
    armed().clear();
}

/// Vor jeder Challenge aufgerufen: `Some` nur während der Freigabe des Auftrags
/// der aufrufenden Überweisung.
pub fn current() -> Option<TanComparison> {
    let me = std::thread::current().id();
    armed()
        .iter()
        .find(|(id, _)| *id == me)
        .and_then(|(_, a)| a.visible())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comparison_is_scoped_to_its_own_transfer_thread() {
        std::thread::spawn(|| {
            let cmp = TanComparison::new("DE89 3704 0044 0532 0130 00", 1250_i64).unwrap();
            let _guard = arm(Some(cmp), || true);
            std::thread::spawn(|| {
                assert!(
                    current().is_none(),
                    "fremde Überweisung sieht keinen Abgleich"
                );
                let other = arm(
                    TanComparison::new("DE02 1203 0000 0000 2020 51", 100_i64),
                    || true,
                );
                assert_eq!(current().unwrap().end_chars, "2051");
                drop(other);
                assert!(current().is_none());
            })
            .join()
            .unwrap();
            // das Ende der anderen Überweisung löscht diesen Abgleich nicht
            assert_eq!(current().unwrap().end_chars, "3000");
        })
        .join()
        .unwrap();
    }

    #[test]
    fn closed_gate_hides_comparison_and_guard_disarms() {
        std::thread::spawn(|| {
            let cmp = TanComparison::new("DE89 3704 0044 0532 0130 00", 1250_i64);
            let guard = arm(cmp.clone(), || false);
            assert!(current().is_none());
            drop(guard);
            let guard = arm(cmp, || true);
            assert!(current().is_some());
            drop(guard);
            assert!(current().is_none());
        })
        .join()
        .unwrap();
    }

    #[test]
    fn groups_prefix_and_keeps_last_four_chars() {
        let c = TanComparison::new("DE89 3704 0044 0532 0130 00", 1250_i64).unwrap();
        assert_eq!(c.prefix, "DE89 3704 0044 0532 01");
        assert_eq!(c.end_chars, "3000");
        assert_eq!(c.amount, "12,50 €");
    }

    #[test]
    fn end_chars_with_letters_and_short_or_empty_iban() {
        let c = TanComparison::new("mt84malt011000012345mtlcast001s", 5_i64).unwrap();
        assert_eq!(c.prefix, "MT84 MALT 0110 0001 2345 MTLC AST");
        assert_eq!(c.end_chars, "001S");
        assert_eq!(c.amount, "0,05 €");
        let short = TanComparison::new("AB1", 0_i64).unwrap();
        assert_eq!(
            (short.prefix.as_str(), short.end_chars.as_str()),
            ("", "AB1")
        );
        assert_eq!(TanComparison::new("  ", 100_i64), None);
    }

    #[test]
    fn formats_several_thousands_separators() {
        assert_eq!(format_eur(123_456_700), "1.234.567,00 €");
        assert_eq!(format_eur(100_000), "1.000,00 €");
        assert_eq!(format_eur(-99), "-0,99 €");
    }

    #[test]
    fn comparison_only_while_order_awaits_approval() {
        let cmp = TanComparison::new("DE89370400440532013000", 1250_i64).unwrap();
        let login = Armed {
            comparison: cmp.clone(),
            gate: Box::new(|| false),
        };
        assert_eq!(login.visible(), None);
        let order = Armed {
            comparison: cmp.clone(),
            gate: Box::new(|| true),
        };
        assert_eq!(order.visible(), Some(cmp));
    }

    // einziger Test, der den globalen Platz benutzt (keine Parallelitäts-Kollision)
    #[test]
    fn guard_disarms_after_transfer() {
        let cmp = TanComparison::new("DE89370400440532013000", 500_i64);
        {
            let _armed = arm(cmp.clone(), || true);
            assert_eq!(current(), cmp);
        }
        assert_eq!(current(), None);
    }
}
