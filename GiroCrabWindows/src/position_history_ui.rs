//! Price history of a depot position (#578): a double click or "Verlauf anzeigen …" in the
//! positions tab opens `position_value_history` in the position's trading currency.
//! Mac reference: `PositionHistorySheet.swift`.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use girocrab_core::Vault;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::portfolio::{self, DepotData, HistoryRow, PositionTarget};
use crate::{MainWindow, PositionHistory, PositionHistoryRow};

type DateFn = fn(&str) -> String;
type PrivacyFn = fn(&MainWindow) -> bool;
type VaultSlot = Arc<Mutex<Option<Arc<Vault>>>>;

thread_local! {
    static SLOT: RefCell<Option<VaultSlot>> = const { RefCell::new(None) };
    /// Positions of the table in row order; row `i` opens `TARGETS[i]`.
    static TARGETS: RefCell<Vec<PositionTarget>> = const { RefCell::new(Vec::new()) };
    /// Currency of Core's series and the rows computed from it.
    static LOADED: RefCell<Option<(String, Vec<HistoryRow>)>> = const { RefCell::new(None) };
    /// The position the dialog was opened for: a refresh that changes it reloads the dialog.
    static SHOWN: RefCell<Option<PositionTarget>> = const { RefCell::new(None) };
}

/// Bumped on every open, close and lock: a late answer of an older request is dropped.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Remembers the vault slot of the window (called once from `views_ui::wire`).
pub fn install(slot: VaultSlot) {
    SLOT.with(|s| *s.borrow_mut() = Some(slot));
}

fn vault_now() -> Option<Arc<Vault>> {
    SLOT.with(|s| s.borrow().as_ref().and_then(|s| s.lock().unwrap().clone()))
}

/// Called from `render_view`: `Some` for the depot positions tab, `None` everywhere else.
pub fn prepare(win: &MainWindow, data: Option<&DepotData>, date: DateFn, privacy: PrivacyFn) {
    let st = win.global::<PositionHistory>();
    let targets = data.map(portfolio::position_targets).unwrap_or_default();
    let same = TARGETS.with(|t| {
        let old = t.borrow();
        old.len() == targets.len() && old.iter().zip(&targets).all(|(a, b)| a.key == b.key)
    });
    if !same {
        st.set_selected(-1);
    }
    if data.is_none() {
        reset(win, false);
    }
    st.set_available(data.is_some());
    TARGETS.with(|t| *t.borrow_mut() = targets);
    let weak = win.as_weak();
    st.on_request(move |i| {
        if let Some(win) = weak.upgrade() {
            open(&win, i, date, privacy);
        }
    });
    let weak = win.as_weak();
    st.on_reset(move |forget| {
        if let Some(win) = weak.upgrade() {
            reset(&win, forget);
        }
    });
    // A refresh that changed quantity or trades of the shown position: its rows are stale,
    // so load them again (a vanished position closes the dialog).
    if data.is_some()
        && let Some(shown) = SHOWN.with(|s| s.borrow().clone())
    {
        let now = TARGETS.with(|t| {
            t.borrow()
                .iter()
                .position(|t| t.key == shown.key && t.account_id == shown.account_id)
        });
        match now {
            Some(i) => {
                if TARGETS.with(|t| t.borrow()[i] != shown) {
                    open(win, i as i32, date, privacy);
                }
            }
            None => reset(win, false),
        }
    }
    // Privacy changes come through `render_view`: redraw an open dialog with the new mode.
    if LOADED.with(|l| l.borrow().is_some()) {
        render(win, date, privacy(win));
    }
}

/// A reload of the depot view started: no new history action until the table is ready. The open
/// dialog and the shown position stay, so `prepare(Some(..))` can refresh them afterwards
/// (a sync of the same depot must not close the dialog).
pub fn loading(win: &MainWindow) {
    win.global::<PositionHistory>().set_available(false);
}

/// The scope changed (navigation) or the load failed: close the dialog and forget the position.
pub fn close(win: &MainWindow) {
    reset(win, false);
}

/// Core's series of the position and its rows; the currency is the series' one.
fn load_history(v: &Vault, target: &PositionTarget) -> Result<(String, Vec<HistoryRow>), String> {
    let series: Vec<portfolio::SeriesInfo> = v
        .depot_value_positions(target.account_id)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|p| (p.position, p.name, p.currency, p.held))
        .collect();
    let Some((key, _, currency, _)) =
        portfolio::resolve_series(&series, &target.isin, &target.name)
    else {
        return Ok((target.currency.clone(), Vec::new()));
    };
    let points = v
        .position_value_history(target.account_id, key.clone())
        .map_err(|e| e.to_string())?;
    let points: Vec<(String, i64)> = points
        .into_iter()
        .map(|p| (p.date, p.value_cents))
        .collect();
    Ok((
        currency.clone(),
        portfolio::position_history_rows(&points, &target.trades, target.quantity),
    ))
}

fn open(win: &MainWindow, index: i32, date: DateFn, privacy: PrivacyFn) {
    let target = usize::try_from(index)
        .ok()
        .and_then(|i| TARGETS.with(|t| t.borrow().get(i).cloned()));
    let (Some(target), Some(v)) = (target, vault_now()) else {
        return;
    };
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    LOADED.with(|l| *l.borrow_mut() = None);
    SHOWN.with(|s| *s.borrow_mut() = Some(target.clone()));
    let st = win.global::<PositionHistory>();
    st.set_selected(index);
    // The security name is a label: it stays visible in privacy mode.
    st.set_title(target.name.clone().into());
    // The currency is known once Core's series is resolved.
    st.set_subtitle(SharedString::new());
    st.set_status("Verlauf wird geladen …".into());
    st.set_rows(ModelRc::default());
    st.set_open(true);
    let weak = win.as_weak();
    std::thread::spawn(move || {
        let result = load_history(&v, &target);
        let _ = weak.upgrade_in_event_loop(move |win| {
            if GENERATION.load(Ordering::SeqCst) != generation || vault_now().is_none() {
                return;
            }
            let st = win.global::<PositionHistory>();
            match result {
                Ok((currency, rows)) => {
                    st.set_status(if rows.is_empty() {
                        "Für diese Position ist noch kein Verlauf gespeichert.".into()
                    } else {
                        SharedString::new()
                    });
                    st.set_subtitle(format!("Wert und Stückzahl in {currency}").into());
                    LOADED.with(|l| *l.borrow_mut() = Some((currency, rows)));
                    render(&win, date, privacy(&win));
                }
                Err(e) => st.set_status(e.into()),
            }
        });
    });
}

fn render(win: &MainWindow, date: DateFn, privacy: bool) {
    let rows: Vec<PositionHistoryRow> = LOADED.with(|l| {
        let loaded = l.borrow();
        let Some((currency, rows)) = loaded.as_ref() else {
            return Vec::new();
        };
        rows.iter()
            .map(|r| {
                let (d, value, quantity) = portfolio::history_texts(r, currency, privacy, &date);
                PositionHistoryRow {
                    date: d.into(),
                    value: value.into(),
                    quantity: quantity.into(),
                    share: if privacy { 0.0 } else { r.share },
                    traded: r.quantity_change.is_some() && !privacy,
                }
            })
            .collect()
    });
    win.global::<PositionHistory>()
        .set_rows(ModelRc::new(VecModel::from(rows)));
}

/// Closes the dialog; `forget` (lock) also drops the remembered positions and the selection.
fn reset(win: &MainWindow, forget: bool) {
    GENERATION.fetch_add(1, Ordering::SeqCst);
    SHOWN.with(|s| *s.borrow_mut() = None);
    LOADED.with(|l| *l.borrow_mut() = None);
    let st = win.global::<PositionHistory>();
    st.set_open(false);
    st.set_title(SharedString::new());
    st.set_subtitle(SharedString::new());
    st.set_status(SharedString::new());
    st.set_rows(ModelRc::default());
    if forget {
        TARGETS.with(|t| t.borrow_mut().clear());
        st.set_selected(-1);
        st.set_available(false);
    }
}
