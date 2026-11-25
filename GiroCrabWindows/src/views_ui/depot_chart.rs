//! Value history chart of one securities depot (#578): the series of
//! `Vault::depot_value_history` as path commands for the Slint `Path` element.

use std::fmt::Write;
use std::sync::Mutex;

use slint::{ModelRc, SharedString, VecModel};

use crate::MainWindow;
use crate::portfolio::{Period, today_iso};

/// Days asked for "Max"; the core starts the series at the first day with a value.
pub const MAX_DAYS: u32 = 365 * 30;
const W: f32 = 1000.0;
const H: f32 = 100.0;

/// One day of the series (from `DepotValuePoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct Point {
    pub date: String,
    pub value_cents: i64,
    pub covered: u32,
}

/// Series of the last loaded depot, keyed by its account id: written by the load worker,
/// read by `render` only for the depot it draws.
static SERIES: Mutex<Option<(i64, Vec<Point>)>> = Mutex::new(None);

pub fn store(account_id: i64, points: Vec<Point>) {
    *SERIES.lock().unwrap_or_else(|e| e.into_inner()) = Some((account_id, points));
}

/// Path in a 1000 × 100 viewbox plus the value range of the vertical axis.
#[derive(Debug, PartialEq)]
pub struct Chart {
    pub path: String,
    pub top_cents: i64,
    pub bottom_cents: i64,
}

/// Range selector: 0 = 1 M, 1 = 1 J, otherwise Max.
pub fn range_period(i: i32) -> Period {
    match i {
        0 => Period::Month,
        1 => Period::Year,
        _ => Period::All,
    }
}

/// The days from `start` (ISO, inclusive) on; the series is sorted by date.
pub fn in_range<'a>(points: &'a [Point], start: Option<&str>) -> &'a [Point] {
    match start {
        Some(s) => &points[points.partition_point(|p| p.date.as_str() < s)..],
        None => points,
    }
}

/// Line of the series; `None` for an empty one. A change in the number of covered depots
/// starts a new stroke (no false growth when a depot's first value comes in); a stroke of a
/// single day is drawn as a short dash.
pub fn chart(points: &[Point]) -> Option<Chart> {
    let lo = points.iter().map(|p| p.value_cents).min()?;
    let hi = points.iter().map(|p| p.value_cents).max()?;
    let (bottom, top) = if lo == hi {
        let pad = (hi.abs() / 10).max(100);
        (lo - pad, hi + pad)
    } else {
        (lo, hi)
    };
    let span = (top - bottom) as f32;
    let n = points.len();
    let x = |i: usize| {
        if n == 1 {
            W / 2.0
        } else {
            i as f32 * W / (n - 1) as f32
        }
    };
    let y = |v: i64| H - (v - bottom) as f32 / span * H;
    let mut path = String::new();
    let mut start = 0;
    while start < n {
        let mut end = start + 1;
        while end < n && points[end].covered == points[start].covered {
            end += 1;
        }
        if end - start == 1 {
            let (px, py) = (x(start), y(points[start].value_cents));
            let (a, b) = ((px - 4.0).max(0.0), (px + 4.0).min(W));
            let _ = write!(path, "M {a:.1} {py:.1} L {b:.1} {py:.1} ");
        } else {
            for (i, p) in points.iter().enumerate().take(end).skip(start) {
                let cmd = if i == start { 'M' } else { 'L' };
                let _ = write!(path, "{cmd} {:.1} {:.1} ", x(i), y(p.value_cents));
            }
        }
        start = end;
    }
    path.pop();
    Some(Chart {
        path,
        top_cents: top,
        bottom_cents: bottom,
    })
}

/// "Stand …" of the last day; the change only within the last stretch of equal coverage.
pub fn readout(
    points: &[Point],
    money: &dyn Fn(i64) -> String,
    date: &dyn Fn(&str) -> String,
) -> String {
    let Some(last) = points.last() else {
        return "Keine Werte im Zeitraum".into();
    };
    let from = points
        .iter()
        .rposition(|p| p.covered != last.covered)
        .map_or(0, |i| i + 1);
    let first = &points[from];
    let mut s = format!("Stand {}: {}", date(&last.date), money(last.value_cents));
    if first.date != last.date {
        let delta = last.value_cents - first.value_cents;
        let _ = write!(
            s,
            " · Veränderung seit {}: {}",
            date(&first.date),
            money(delta)
        );
    }
    s
}

/// The cached series if it belongs to `account_id` and that depot is the selected one; empty
/// otherwise, so a redraw of a retained depot never shows its curve beside another selection.
fn series_of(
    cached: Option<&(i64, Vec<Point>)>,
    account_id: i64,
    selected: Option<i64>,
) -> &[Point] {
    match cached {
        Some((id, p)) if *id == account_id && !is_foreign(Some(account_id), selected) => p,
        _ => &[],
    }
}

/// Draws the chart of `account_id` while it is the `selected` depot; amounts go through `money`
/// (masked in privacy mode). Otherwise the chart stays hidden and the cached series is kept.
pub fn render(
    win: &MainWindow,
    account_id: i64,
    selected: Option<i64>,
    money: &dyn Fn(i64) -> String,
    date: &dyn Fn(&str) -> String,
) {
    let guard = SERIES.lock().unwrap_or_else(|e| e.into_inner());
    let all = series_of(guard.as_ref(), account_id, selected);
    let start = range_period(win.get_depot_chart_range()).start(&today_iso());
    let points = in_range(all, start.as_deref());
    let (path, ticks) = match chart(points) {
        Some(c) => {
            let mid = c.bottom_cents + (c.top_cents - c.bottom_cents) / 2;
            let ticks = [c.top_cents, mid, c.bottom_cents].map(|v| SharedString::from(money(v)));
            (c.path, ticks.to_vec())
        }
        None => (String::new(), Vec::new()),
    };
    win.set_depot_chart_shown(!all.is_empty());
    win.set_depot_chart_path(path.into());
    win.set_depot_chart_ticks(ModelRc::new(VecModel::from(ticks)));
    win.set_depot_chart_readout(readout(points, money, date).into());
}

/// Forgets the series and empties the chart (no depot data, e.g. after locking).
pub fn reset(win: &MainWindow) {
    *SERIES.lock().unwrap_or_else(|e| e.into_inner()) = None;
    win.set_depot_chart_shown(false);
    win.set_depot_chart_path(SharedString::new());
    win.set_depot_chart_ticks(ModelRc::default());
    win.set_depot_chart_readout(SharedString::new());
}

/// Whether the chart on screen belongs to another depot than the selected one: the cached
/// series is for `cached`, the view now shows `selected` (`None` = no single depot).
fn is_foreign(cached: Option<i64>, selected: Option<i64>) -> bool {
    selected.is_none() || cached != selected
}

/// Hides the chart while the selected depot loads if it still shows the curve of another one.
/// The series stays cached, so the same depot is drawn again without a vault call.
pub fn hide_unless_cached(win: &MainWindow, selected: Option<i64>) {
    let cached = SERIES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|(id, _)| *id);
    if is_foreign(cached, selected) {
        win.set_depot_chart_shown(false);
    }
}

/// Hides the chart (a failed load) without forgetting the cached series.
pub fn hide(win: &MainWindow) {
    win.set_depot_chart_shown(false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chart_of_another_depot_is_foreign_the_same_depot_is_kept() {
        assert!(!is_foreign(Some(3), Some(3)));
        assert!(is_foreign(Some(3), Some(4)));
        assert!(is_foreign(None, Some(4)));
        assert!(is_foreign(Some(3), None));
    }

    fn pt(date: &str, value_cents: i64, covered: u32) -> Point {
        Point {
            date: date.into(),
            value_cents,
            covered,
        }
    }

    #[test]
    fn redraw_shows_only_the_series_of_the_selected_depot() {
        let cached = (3, vec![pt("2026-11-01", 100, 1)]);
        assert_eq!(series_of(Some(&cached), 3, Some(3)).len(), 1);
        // Retained depot 3 redrawn while depot 4 (or no single depot) is selected: no curve.
        assert!(series_of(Some(&cached), 3, Some(4)).is_empty());
        assert!(series_of(Some(&cached), 3, None).is_empty());
        // The cached series of another depot is never drawn for this one.
        assert!(series_of(Some(&cached), 4, Some(4)).is_empty());
        assert!(series_of(None, 3, Some(3)).is_empty());
    }

    #[test]
    fn empty_series_has_no_chart() {
        assert_eq!(chart(&[]), None);
        assert!(in_range(&[], Some("2026-01-01")).is_empty());
        let s = readout(&[], &|c: i64| c.to_string(), &|d: &str| d.to_string());
        assert_eq!(s, "Keine Werte im Zeitraum");
    }

    #[test]
    fn scales_between_min_and_max() {
        let c = chart(&[pt("2026-11-01", 100, 1), pt("2026-11-02", 300, 1)]).unwrap();
        assert_eq!(c.path, "M 0.0 100.0 L 1000.0 0.0");
        assert_eq!((c.bottom_cents, c.top_cents), (100, 300));
    }

    #[test]
    fn single_point_is_a_centered_dash_with_padded_axis() {
        let c = chart(&[pt("2026-11-01", 5000, 1)]).unwrap();
        assert_eq!(c.path, "M 496.0 50.0 L 504.0 50.0");
        assert_eq!((c.bottom_cents, c.top_cents), (4500, 5500));
    }

    #[test]
    fn coverage_change_breaks_the_line() {
        let points = [
            pt("2026-11-01", 0, 1),
            pt("2026-11-02", 100, 1),
            pt("2026-11-03", 200, 2),
            pt("2026-11-04", 300, 2),
        ];
        let c = chart(&points).unwrap();
        assert_eq!(c.path.matches('M').count(), 2);
        assert!(c.path.starts_with("M 0.0 100.0 L 333.3 66.7 M 666.7"));
    }

    #[test]
    fn range_filter_and_readout_within_last_coverage() {
        let points = [
            pt("2026-10-01", 10, 1),
            pt("2026-11-01", 20, 2),
            pt("2026-11-20", 35, 2),
        ];
        assert_eq!(in_range(&points, None).len(), 3);
        assert_eq!(in_range(&points, Some("2026-10-23")).len(), 2);
        let s = readout(&points, &|c: i64| c.to_string(), &|d: &str| d.to_string());
        assert_eq!(s, "Stand 2026-11-20: 35 · Veränderung seit 2026-11-01: 15");
        assert_eq!(range_period(0), Period::Month);
        assert_eq!(range_period(1), Period::Year);
        assert_eq!(range_period(2), Period::All);
    }
}
