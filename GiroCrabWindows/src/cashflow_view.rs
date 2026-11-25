//! Cashflow-Diagramm über der Buchungsliste: Die echten Werte bleiben hier, die Anzeige
//! maskiert sie je nach Privatmodus. So stellt der Privatmodus-Schalter das Diagramm sofort
//! um, ohne auf das nächste Laden des Dashboards zu warten.

use crate::{BarGroup, MainWindow};
use slint::{ModelRc, SharedString, VecModel};
use std::cell::RefCell;

thread_local! {
    static LAST: RefCell<(Vec<BarGroup>, Vec<SharedString>)> =
        const { RefCell::new((Vec::new(), Vec::new())) };
}

/// Neue Diagrammwerte aus dem Dashboard übernehmen und anzeigen.
pub fn set(win: &MainWindow, bars: Vec<BarGroup>, ticks: Vec<SharedString>, privacy: bool) {
    LAST.with(|l| *l.borrow_mut() = (bars, ticks));
    render(win, privacy);
}

/// Diagramm aus den zuletzt geladenen Werten neu zeichnen (z. B. Privatmodus umgeschaltet).
pub fn render(win: &MainWindow, privacy: bool) {
    let (bars, ticks) = LAST.with(|l| {
        let l = l.borrow();
        masked(&l.0, &l.1, privacy)
    });
    win.set_cashflow_bars(ModelRc::new(VecModel::from(bars)));
    win.set_cashflow_ticks(ModelRc::new(VecModel::from(ticks)));
}

/// Im Privatmodus keine Beträge: Balken auf null, Achsenwerte als Platzhalter; Monate bleiben.
fn masked(
    bars: &[BarGroup],
    ticks: &[SharedString],
    privacy: bool,
) -> (Vec<BarGroup>, Vec<SharedString>) {
    if !privacy {
        return (bars.to_vec(), ticks.to_vec());
    }
    let bars = bars
        .iter()
        .map(|b| BarGroup {
            label: b.label.clone(),
            income: 0.0,
            expense: 0.0,
        })
        .collect();
    let ticks = ticks.iter().map(|_| SharedString::from("•")).collect();
    (bars, ticks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> (Vec<BarGroup>, Vec<SharedString>) {
        (
            vec![BarGroup {
                label: "Jan".into(),
                income: 1200.0,
                expense: 800.0,
            }],
            vec![SharedString::from("1.234.567 €")],
        )
    }

    #[test]
    fn privacy_hides_bar_values_and_ticks_but_keeps_labels() {
        let (bars, ticks) = sample();
        let (b, t) = masked(&bars, &ticks, true);
        assert_eq!((b[0].income, b[0].expense), (0.0, 0.0));
        assert_eq!(b[0].label.as_str(), "Jan");
        assert_eq!(t[0].as_str(), "•");
        assert!(t.iter().all(|t| !t.contains('€')));
    }

    #[test]
    fn privacy_off_shows_the_real_values_again() {
        let (bars, ticks) = sample();
        let (b, t) = masked(&bars, &ticks, false);
        assert_eq!((b[0].income, b[0].expense), (1200.0, 800.0));
        assert_eq!(t[0].as_str(), "1.234.567 €");
    }
}
