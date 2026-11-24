//! Depot view: "Als CSV exportieren …" writes the active tab (positions or activities) with the
//! shown columns and order (rows from `portfolio::depot_csv`). It also runs in privacy mode as a
//! deliberate user action; file name and status line never carry the depot number.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use girocrab_core::Vault;
use slint::ComponentHandle;

use crate::MainWindow;

/// `view-mode` of the depot view (`MODE_DEPOT` in `views_ui.rs`).
const DEPOT_VIEW_MODE: i32 = 1;

thread_local! {
    /// CSV and file name of the depot tab on screen, rebuilt with every depot render.
    static SHOWN: RefCell<Option<(String, String)>> = const { RefCell::new(None) };
}

/// Called by the depot render with the unmasked CSV of the active tab.
pub fn remember(csv: String, file_name: String) {
    SHOWN.with(|s| *s.borrow_mut() = Some((csv, file_name)));
}

/// Drops the cached CSV: a depot switch starts, a load fails or the vault locks.
pub fn forget() {
    SHOWN.with(|s| *s.borrow_mut() = None);
}

/// Writes a new file into `dir` and never replaces one: a taken `name` gets the next free
/// `-N` name (`create_new` is atomic, no `exists()` race). A failed write removes the file it
/// just created; files that existed before are never touched.
fn write_new(dir: &Path, name: &str, data: &str) -> Result<PathBuf, String> {
    use std::io::Write;
    for i in 1..=100 {
        let path = dir.join(crate::settings_import::numbered_file_name(name, i));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
                let written = f.write_all(data.as_bytes());
                // close before removing: Windows cannot delete an open file
                drop(f);
                return match written {
                    Ok(()) => Ok(path),
                    Err(e) => {
                        let _ = std::fs::remove_file(&path);
                        Err(e.to_string())
                    }
                };
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("Im Ordner ist kein freier Dateiname mehr verfügbar.".to_string())
}

/// Downloads folder (falls back to home, then temp), like the booking export.
fn export_dir() -> PathBuf {
    let base = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    if base.join("Downloads").is_dir() {
        base.join("Downloads")
    } else {
        base
    }
}

pub fn wire(win: &MainWindow, vault: Arc<Mutex<Option<Arc<Vault>>>>) {
    let weak = win.as_weak();
    win.on_depot_export_csv(move || {
        let Some(win) = weak.upgrade() else { return };
        // Locked vault or another view: never write what an earlier render left behind.
        if vault.lock().unwrap().is_none() || win.get_view_mode() != DEPOT_VIEW_MODE {
            forget();
            return;
        }
        let Some((csv, name)) = SHOWN.with(|s| s.borrow().clone()) else {
            win.set_sync_status("Depot-Export: keine Daten geladen.".into());
            return;
        };
        match write_new(&export_dir(), &name, &csv) {
            Ok(path) => win.set_sync_status(format!("Exportiert: {}", path.display()).into()),
            Err(e) => win.set_sync_status(format!("Export fehlgeschlagen: {e}").into()),
        }
    });
}

#[cfg(test)]
mod tests {
    use crate::portfolio::{
        DepotTab, PositionLine, csv_amount, depot_csv_file_name, positions_csv,
    };

    fn line(
        name: &str,
        qty: f64,
        price: i64,
        cur: &str,
        value: i64,
        pl: Option<i64>,
    ) -> PositionLine {
        PositionLine {
            name: name.into(),
            quantity: qty,
            price_cents: price,
            price_currency: cur.into(),
            value_cents: value,
            pl_cents: pl,
        }
    }

    #[test]
    fn escapes_separator_quotes_and_formulas() {
        let csv = positions_csv(
            &[
                line("Fonds; \"Welt\"", 1.0, 10_000, "EUR", 10_000, None),
                line("=HYPERLINK(1)", 1.0, 100, "EUR", 100, None),
            ],
            "EUR",
        );
        assert!(csv.contains("\r\n\"Fonds; \"\"Welt\"\"\";1;100,00;EUR;100,00;\r\n"));
        assert!(csv.contains("\r\n'=HYPERLINK(1);1;1,00;EUR;1,00;\r\n"));
    }

    #[test]
    fn decimal_comma_without_grouping_and_dialect() {
        assert_eq!(csv_amount(-123_456), "-1234,56");
        assert_eq!(csv_amount(5), "0,05");
        assert_eq!(csv_amount(-5), "-0,05");
        assert_eq!(csv_amount(100_000_000), "1000000,00");
        let csv = positions_csv(&[line("ETF", 2.5, 4_000, "EUR", 10_000, Some(-250))], "EUR");
        assert!(
            csv.starts_with("\u{feff}Position;Menge;Kurs;Handelswährung;Wert (EUR);G/V (EUR)\r\n")
        );
        assert!(csv.contains("\r\nETF;2,5;40,00;EUR;100,00;-2,50\r\n"));
    }

    #[test]
    fn mixed_currencies_are_kept_per_row_and_never_summed() {
        let csv = positions_csv(
            &[
                line("Aktie US", 3.0, 18_950, "USD", 52_000, Some(4_000)),
                line("Fonds", 10.0, 10_000, "EUR", 100_000, Some(-2_550)),
            ],
            "EUR",
        );
        assert!(csv.contains("\r\nAktie US;3;189,50;USD;520,00;40,00\r\n"));
        assert!(csv.contains("\r\nFonds;10;100,00;EUR;1000,00;-25,50\r\n"));
        // Header plus exactly the two rows: no total line.
        assert_eq!(csv.split("\r\n").filter(|l| !l.is_empty()).count(), 3);
        assert!(!csv.contains("Summe"));
    }

    #[test]
    fn file_name_names_tab_and_date_only() {
        assert_eq!(
            depot_csv_file_name(DepotTab::Positions, "2026-11-23"),
            "GiroCrab-Depot-Positionen-2026-11-23.csv"
        );
        assert_eq!(
            depot_csv_file_name(DepotTab::Activities, "2026-11-23"),
            "GiroCrab-Depot-Aktivitaeten-2026-11-23.csv"
        );
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "girocrab-depot-export-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn same_day_export_never_replaces_an_earlier_file() {
        let dir = temp_dir("numbered");
        let name = "GiroCrab-Depot-Positionen-2026-11-23.csv";
        let first = super::write_new(&dir, name, "erste").unwrap();
        let second = super::write_new(&dir, name, "zweite").unwrap();
        let third = super::write_new(&dir, name, "dritte").unwrap();
        assert_eq!(first, dir.join(name));
        assert_eq!(
            second,
            dir.join("GiroCrab-Depot-Positionen-2026-11-23-2.csv")
        );
        assert_eq!(
            third,
            dir.join("GiroCrab-Depot-Positionen-2026-11-23-3.csv")
        );
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "erste");
        assert_eq!(std::fs::read_to_string(&second).unwrap(), "zweite");
        assert_eq!(std::fs::read_to_string(&third).unwrap(), "dritte");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn forget_drops_the_remembered_csv() {
        super::remember("a;b".into(), "x.csv".into());
        assert!(super::SHOWN.with(|s| s.borrow().is_some()));
        super::forget();
        assert!(super::SHOWN.with(|s| s.borrow().is_none()));
    }
}
