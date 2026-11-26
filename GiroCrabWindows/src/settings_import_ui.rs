//! Slint wiring of the "Import, Export & Einstellungen" dialog (`ui/settings_import_dialog.slint`):
//! statement and MoneyMoney imports, Amazon orders, CSV / CAMT.053 export, encrypted backup,
//! settings and the Abrufprotokoll. The pure texts and checks live in `settings_import.rs`. Every
//! vault call, file dialog and file access runs on a worker thread; results come back through
//! `upgrade_in_event_loop`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use girocrab_core::export::{Camt053Request, ExportCsvBalanceLine, export_csv};
use girocrab_core::importers::moneymoney_db::{
    MoneyMoneyDbAccountChoice, MoneyMoneyDbImportOptions, MoneyMoneyDbImportProgress,
    MoneyMoneyImportObserver,
};
use girocrab_core::sync_log::{SyncLogEntry, SyncLogFilter};
use girocrab_core::{BookingFilter, ConnectorKind, SettingEntry, Vault};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::announcements::{self, Announcement};
use crate::settings_import::{self as view, Line};
use crate::{MainWindow, SettingsImportState, SiRow, german_date, money, privacy_on};

type VaultSlot = Arc<Mutex<Option<Arc<Vault>>>>;

const CAMT_VERSIONS: [&str; 2] = [
    "camt.053.001.02 (ältere Programme)",
    "camt.053.001.08 (aktuell)",
];

#[derive(Default)]
struct Ui {
    /// Import targets in menu order; `None` = new archive account.
    targets: Vec<Option<i64>>,
    /// Accounts in the export menu: (id, name).
    accounts: Vec<(i64, String)>,
    amazon_ids: Vec<i64>,
    file: Option<PathBuf>,
    db_path: Option<String>,
    amazon_json: Option<String>,
    settings: Vec<SettingEntry>,
    log: Vec<SyncLogEntry>,
    restore_armed: bool,
    clear_armed: bool,
    cancel: Option<Arc<AtomicBool>>,
    /// Announcements shown in the Mitteilungen tab (not dismissed).
    news: Vec<Announcement>,
}

thread_local! {
    static UI: RefCell<Ui> = RefCell::new(Ui::default());
    static SLOT: RefCell<Option<VaultSlot>> = const { RefCell::new(None) };
}

fn with_ui<R>(f: impl FnOnce(&mut Ui) -> R) -> R {
    UI.with(|u| f(&mut u.borrow_mut()))
}

fn vault_now() -> Option<Arc<Vault>> {
    SLOT.with(|s| s.borrow().as_ref().and_then(|s| s.lock().unwrap().clone()))
}

fn eur(c: i64) -> String {
    money(c, "EUR")
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn names(items: Vec<String>) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        items
            .into_iter()
            .map(SharedString::from)
            .collect::<Vec<_>>(),
    ))
}

fn rows(lines: Vec<Line>) -> ModelRc<SiRow> {
    ModelRc::new(VecModel::from(
        lines
            .into_iter()
            .map(|l| SiRow {
                title: l.title.into(),
                detail: l.detail.into(),
            })
            .collect::<Vec<_>>(),
    ))
}

/// Closes the dialog and forgets its state (and every typed password) when the vault locks.
pub fn on_lock(win: &MainWindow) {
    let gs = win.global::<SettingsImportState>();
    gs.set_open(false);
    gs.set_db_password(SharedString::new());
    gs.set_backup_password(SharedString::new());
    gs.set_backup_repeat(SharedString::new());
    gs.set_setting_value(SharedString::new());
    if let Some(c) = with_ui(|u| u.cancel.clone()) {
        c.store(true, Ordering::SeqCst);
    }
    with_ui(|u| *u = Ui::default());
}

fn set_status(win: &MainWindow, text: impl Into<SharedString>) {
    win.global::<SettingsImportState>().set_status(text.into());
}

/// Runs `work` on a worker thread; `done` and errors are shown on the UI thread.
fn spawn<T: Send + 'static>(
    win: &MainWindow,
    busy_text: &str,
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
    done: impl FnOnce(&MainWindow, T) + Send + 'static,
) {
    let gs = win.global::<SettingsImportState>();
    gs.set_busy(true);
    gs.set_status(busy_text.into());
    let weak = win.as_weak();
    std::thread::spawn(move || {
        let result = work();
        let _ = weak.upgrade_in_event_loop(move |win| {
            let gs = win.global::<SettingsImportState>();
            gs.set_busy(false);
            gs.set_show_progress(false);
            match result {
                Ok(v) => done(&win, v),
                Err(e) => set_status(&win, e),
            }
        });
    });
}

/// Reloads the booking list after a change of the vault's contents.
fn refresh_main(win: &MainWindow, v: &Arc<Vault>) {
    let sel = Some(win.get_selected() as i64).filter(|s| *s >= 0);
    crate::views_ui::invalidate_planned_count();
    crate::show(win, v, sel);
}

// ---- files -------------------------------------------------------------------------------------

#[cfg(windows)]
fn pick_file(title: &str, filter: &str, exts: &[&str]) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title(title)
        .add_filter(filter, exts)
        .pick_file()
}

#[cfg(not(windows))]
fn pick_file(_title: &str, _filter: &str, _exts: &[&str]) -> Option<PathBuf> {
    std::env::var_os("GIROCRAB_IMPORT_FILE").map(PathBuf::from)
}

#[cfg(windows)]
fn pick_dir(title: &str) -> Option<PathBuf> {
    rfd::FileDialog::new().set_title(title).pick_folder()
}

#[cfg(not(windows))]
fn pick_dir(_title: &str) -> Option<PathBuf> {
    std::env::var_os("GIROCRAB_IMPORT_FILE").map(PathBuf::from)
}

#[cfg(windows)]
fn save_file(title: &str, name: &str) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title(title)
        .set_file_name(name)
        .save_file()
}

#[cfg(not(windows))]
fn save_file(_title: &str, name: &str) -> Option<PathBuf> {
    std::env::var_os("GIROCRAB_EXPORT_DIR").map(|d| PathBuf::from(d).join(name))
}

fn read_bytes(path: &PathBuf) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("Die Datei konnte nicht gelesen werden: {e}"))
}

fn write_bytes(path: &PathBuf, data: &[u8]) -> Result<(), String> {
    std::fs::write(path, data)
        .map_err(|e| format!("Die Datei konnte nicht geschrieben werden: {e}"))
}

/// Writes a new file into `dir` and never replaces one: if `name` was created in the meantime
/// (by another program), the next free `-N` name is used.
fn write_new(dir: &std::path::Path, name: &str, data: &[u8]) -> Result<(), String> {
    use std::io::Write;
    write_new_with(dir, name, |f| f.write_all(data))
}

/// `write_new` with the writing step passed in. A failed write removes the file it just created
/// (no half-written export next to a retry); files that existed before are never touched.
fn write_new_with(
    dir: &std::path::Path,
    name: &str,
    write: impl Fn(&mut std::fs::File) -> std::io::Result<()>,
) -> Result<(), String> {
    let failed = |e: std::io::Error| format!("Die Datei konnte nicht geschrieben werden: {e}");
    for i in 1..=100 {
        let path = dir.join(view::numbered_file_name(name, i));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
                let written = write(&mut f);
                // close before removing: Windows cannot delete an open file
                drop(f);
                return match written {
                    Ok(()) => Ok(()),
                    Err(e) => {
                        let _ = std::fs::remove_file(&path);
                        Err(failed(e))
                    }
                };
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(failed(e)),
        }
    }
    Err("Im Ordner ist kein freier Dateiname mehr verfügbar.".to_string())
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

// ---- progress ------------------------------------------------------------------------------------

struct Observer {
    weak: slint::Weak<MainWindow>,
    cancel: Arc<AtomicBool>,
}

impl MoneyMoneyImportObserver for Observer {
    fn progress(&self, p: MoneyMoneyDbImportProgress) -> bool {
        let text = view::progress_text(&p);
        let fraction = p.fraction.clamp(0.0, 1.0) as f32;
        let _ = self.weak.upgrade_in_event_loop(move |win| {
            let gs = win.global::<SettingsImportState>();
            gs.set_progress(fraction);
            gs.set_status(text.into());
        });
        !self.cancel.load(Ordering::SeqCst)
    }
}

// ---- wiring --------------------------------------------------------------------------------------

pub fn wire(win: &MainWindow, slot: VaultSlot) {
    SLOT.with(|s| *s.borrow_mut() = Some(slot));
    let gs = win.global::<SettingsImportState>();
    gs.set_camt_versions(names(CAMT_VERSIONS.iter().map(|s| s.to_string()).collect()));
    gs.set_mode_names(names(view::product_mode_names()));
    wire_open(win);
    wire_import(win);
    wire_export(win);
    wire_backup(win);
    wire_settings(win);
    wire_log(win);
    wire_news(win);
}

fn wire_open(win: &MainWindow) {
    let gs = win.global::<SettingsImportState>();

    // Belege & Postbox: switches the document retrieval of the connection shown there (#954).
    let weak = win.as_weak();
    win.on_doc_fetch_toggled(move |on| {
        let Some(win) = weak.upgrade() else {
            return;
        };
        // Show the stored state until the vault confirmed the change.
        win.set_doc_fetch_on(!on);
        // Privacy mode locks changes like the other actions of the Belege view.
        if crate::privacy_on(&win) {
            win.set_view_status("Im Privatmodus nicht möglich.".into());
            return;
        }
        let target = crate::views_ui::doc_connections::toggle_target();
        let (Some(id), Some(v)) = (target, vault_now()) else {
            return;
        };
        // Session of this request: a lock in between invalidates the completion.
        let epoch = crate::views_ui::doc_connections::epoch();
        spawn(
            &win,
            "Wird gespeichert …",
            // The error is handled below: the Documents view shows `view_status`, not the
            // status line of the settings dialog.
            move || Ok(v.set_documents_fetch(id, on).map_err(|e| e.to_string())),
            move |win, saved: Result<(), String>| {
                if crate::views_ui::doc_connections::epoch() != epoch {
                    return;
                }
                match saved {
                    Ok(()) => {
                        crate::views_ui::doc_connections::set_enabled(id, on);
                        // Only the switch of the connection that was saved; if another one is
                        // selected now, its own stored value stays visible.
                        if crate::views_ui::doc_connections::toggle_target() == Some(id) {
                            win.set_doc_fetch_on(on);
                        }
                    }
                    Err(e) => win.set_view_status(e.into()),
                }
            },
        );
    });

    let weak = win.as_weak();
    gs.on_request(move || {
        let (Some(win), Some(v)) = (weak.upgrade(), vault_now()) else {
            return;
        };
        if privacy_on(&win) {
            return;
        }
        let gs = win.global::<SettingsImportState>();
        gs.set_tab(0);
        gs.set_open(true);
        with_ui(|u| {
            u.restore_armed = false;
            u.clear_armed = false;
        });
        spawn(
            &win,
            "Wird geladen …",
            move || {
                let accounts = v.list_accounts().map_err(|e| e.to_string())?;
                let connections = v.list_bank_connections().map_err(|e| e.to_string())?;
                Ok((accounts, connections))
            },
            |win, (accounts, connections)| {
                let mut targets = vec![None];
                let mut labels = vec!["Neues Archivkonto".to_string()];
                for a in &accounts {
                    targets.push(Some(a.id));
                    labels.push(a.name.clone());
                }
                let amazon: Vec<_> = connections
                    .iter()
                    .filter(|c| c.connector == ConnectorKind::AmazonOrders)
                    .collect();
                let gs = win.global::<SettingsImportState>();
                gs.set_target_names(names(labels));
                gs.set_target(0);
                gs.set_acc_names(names(accounts.iter().map(|a| a.name.clone()).collect()));
                gs.set_acc(0);
                gs.set_amazon_names(names(
                    amazon.iter().map(|c| c.display_name.clone()).collect(),
                ));
                gs.set_amazon(0);
                gs.set_status(SharedString::new());
                with_ui(|u| {
                    u.targets = targets;
                    u.accounts = accounts.iter().map(|a| (a.id, a.name.clone())).collect();
                    u.amazon_ids = amazon.iter().map(|c| c.id).collect();
                });
            },
        );
    });

    let weak = win.as_weak();
    gs.on_tab_changed(move |tab| {
        let (Some(win), Some(v)) = (weak.upgrade(), vault_now()) else {
            return;
        };
        set_status(&win, "");
        with_ui(|u| {
            u.restore_armed = false;
            u.clear_armed = false;
        });
        let gs = win.global::<SettingsImportState>();
        gs.set_restore_armed(false);
        gs.set_clear_armed(false);
        match tab {
            3 => load_settings(&win, &v),
            4 => load_log(&win, &v),
            5 => refresh_news(&win, &v),
            _ => {}
        }
    });
}

// ---- import ---------------------------------------------------------------------------------------

fn target_id(win: &MainWindow) -> Option<i64> {
    let i = usize::try_from(win.global::<SettingsImportState>().get_target()).ok()?;
    with_ui(|u| u.targets.get(i).copied().flatten())
}

fn wire_import(win: &MainWindow) {
    let gs = win.global::<SettingsImportState>();

    // statement file: pick, check, then the core's dry run
    let weak = win.as_weak();
    gs.on_pick_file(move || {
        let (Some(win), Some(v)) = (weak.upgrade(), vault_now()) else {
            return;
        };
        let target = target_id(&win);
        win.global::<SettingsImportState>().set_file_ready(false);
        spawn(
            &win,
            "Datei wird geprüft …",
            move || {
                let path = pick_file(
                    "Kontoauszug wählen",
                    "CSV, XML, CAMT, MT940",
                    &["csv", "xml", "sta", "mt940", "txt"],
                )
                .ok_or("Import abgebrochen.".to_string())?;
                let data = read_bytes(&path)?;
                let name = file_name(&path);
                let (format, parsed) = view::parse_statement(&name, &data)?;
                let check = view::check_rows(format, &parsed);
                let preview = v
                    .preview_moneymoney_import(data, name, target)
                    .map_err(|e| e.to_string())?;
                let text = format!(
                    "{} {}",
                    view::file_check_text(&check),
                    view::preview_text(&preview, &german_date)
                );
                Ok((path, text))
            },
            |win, (path, text)| {
                with_ui(|u| u.file = Some(path));
                let gs = win.global::<SettingsImportState>();
                gs.set_file_info(text.into());
                gs.set_file_ready(true);
            },
        );
    });

    let weak = win.as_weak();
    gs.on_run_import(move || {
        let (Some(win), Some(v)) = (weak.upgrade(), vault_now()) else {
            return;
        };
        let Some(path) = with_ui(|u| u.file.clone()) else {
            return;
        };
        let target = target_id(&win);
        let vault = v.clone();
        spawn(
            &win,
            "Wird importiert …",
            move || {
                let data = read_bytes(&path)?;
                let r = v
                    .import_moneymoney_data(data, file_name(&path), target, true, true)
                    .map_err(|e| e.to_string())?;
                Ok(view::import_result_text(&r))
            },
            move |win, text| {
                refresh_main(win, &vault);
                let gs = win.global::<SettingsImportState>();
                gs.set_file_ready(false);
                gs.set_file_info(text.into());
            },
        );
    });

    // MoneyMoney database
    let weak = win.as_weak();
    gs.on_pick_db(move || {
        let (Some(win), Some(v)) = (weak.upgrade(), vault_now()) else {
            return;
        };
        let password = win
            .global::<SettingsImportState>()
            .get_db_password()
            .to_string();
        if password.is_empty() {
            set_status(&win, "Bitte zuerst das Passwort der Datenbank eintragen.");
            return;
        }
        spawn(
            &win,
            "Datenbank wird gelesen …",
            move || {
                let path = pick_file(
                    "Datenbank wählen",
                    "Datenbank",
                    &["moneymoney", "sqlite", "db"],
                )
                .or_else(|| pick_dir("Ordner mit Datenbank wählen"))
                .ok_or("Import abgebrochen.".to_string())?;
                let location = path.display().to_string();
                let p = v
                    .preview_moneymoney_database(location.clone(), password, None)
                    .map_err(|e| e.to_string())?;
                let text = format!("{} Konten gefunden.", p.accounts.len());
                Ok((location, text))
            },
            |win, (location, text)| {
                with_ui(|u| u.db_path = Some(location));
                let gs = win.global::<SettingsImportState>();
                gs.set_db_info(text.into());
                gs.set_db_ready(true);
            },
        );
    });

    let weak = win.as_weak();
    gs.on_run_db(move || {
        let (Some(win), Some(v)) = (weak.upgrade(), vault_now()) else {
            return;
        };
        let Some(location) = with_ui(|u| u.db_path.clone()) else {
            return;
        };
        let password = win
            .global::<SettingsImportState>()
            .get_db_password()
            .to_string();
        let cancel = Arc::new(AtomicBool::new(false));
        with_ui(|u| u.cancel = Some(cancel.clone()));
        let observer = Arc::new(Observer {
            weak: win.as_weak(),
            cancel,
        });
        let vault = v.clone();
        spawn(
            &win,
            "Datenbank wird gelesen …",
            move || {
                let preview = v
                    .preview_moneymoney_database(location.clone(), password.clone(), None)
                    .map_err(|e| e.to_string())?;
                let options = MoneyMoneyDbImportOptions {
                    accounts: preview
                        .accounts
                        .iter()
                        .map(|a| MoneyMoneyDbAccountChoice {
                            mm_id: a.mm_id,
                            import: true,
                            kind: a.suggested_kind,
                            target_account_id: a.existing_account_id,
                        })
                        .collect(),
                    import_categories: true,
                    import_notes: true,
                    create_connections: false,
                    import_groups: false,
                    import_statements: false,
                    statements_dir: None,
                    bank_choices: vec![],
                    import_transfer_templates: true,
                };
                let r = v
                    .import_moneymoney_database_with_progress(location, password, options, observer)
                    .map_err(|e| e.to_string())?;
                Ok(view::db_result_text(&r))
            },
            move |win, text| {
                refresh_main(win, &vault);
                let gs = win.global::<SettingsImportState>();
                gs.set_db_ready(false);
                gs.set_db_password(SharedString::new());
                gs.set_db_info(text.clone().into());
                gs.set_status(text.into());
                with_ui(|u| u.cancel = None);
            },
        );
        win.global::<SettingsImportState>().set_progress(0.0);
        win.global::<SettingsImportState>().set_show_progress(true);
    });

    let weak = win.as_weak();
    gs.on_cancel_db(move || {
        if let Some(c) = with_ui(|u| u.cancel.clone()) {
            c.store(true, Ordering::SeqCst);
        }
        if let Some(win) = weak.upgrade() {
            win.global::<SettingsImportState>().set_show_progress(false);
        }
    });

    // Amazon orders
    let weak = win.as_weak();
    gs.on_pick_amazon(move || {
        let Some(win) = weak.upgrade() else { return };
        win.global::<SettingsImportState>().set_amazon_ready(false);
        spawn(
            &win,
            "Datei wird geprüft …",
            move || {
                let path = pick_file("Bestell-Export wählen", "JSON", &["json", "txt"])
                    .ok_or("Import abgebrochen.".to_string())?;
                let text = String::from_utf8(read_bytes(&path)?)
                    .map_err(|_| "Die Datei ist kein Text (UTF-8).".to_string())?;
                let check = view::amazon_check(&text)?;
                Ok((text, view::amazon_text(&check, &eur)))
            },
            |win, (json, info)| {
                with_ui(|u| u.amazon_json = Some(json));
                let gs = win.global::<SettingsImportState>();
                gs.set_amazon_info(info.into());
                gs.set_amazon_ready(true);
            },
        );
    });

    let weak = win.as_weak();
    gs.on_run_amazon(move || {
        let (Some(win), Some(v)) = (weak.upgrade(), vault_now()) else {
            return;
        };
        let idx = usize::try_from(win.global::<SettingsImportState>().get_amazon()).ok();
        let (Some(json), Some(id)) = (
            with_ui(|u| u.amazon_json.clone()),
            idx.and_then(|i| with_ui(|u| u.amazon_ids.get(i).copied())),
        ) else {
            return;
        };
        let vault = v.clone();
        spawn(
            &win,
            "Bestellungen werden importiert …",
            move || {
                let s = v
                    .import_amazon_orders(id, json)
                    .map_err(|e| e.to_string())?;
                Ok(format!(
                    "Amazon-Import: {} neue Umsätze.",
                    s.transactions_imported
                ))
            },
            move |win, text| {
                refresh_main(win, &vault);
                let gs = win.global::<SettingsImportState>();
                gs.set_amazon_ready(false);
                gs.set_amazon_info(text.into());
            },
        );
    });
}

// ---- export ---------------------------------------------------------------------------------------

type Period = (Option<String>, Option<String>);

/// Every export path (single CSV, one CSV per account, CAMT) reads its period through here,
/// so the privacy-mode block sits here and applies before any file dialog opens.
fn export_period(win: &MainWindow) -> Result<Period, String> {
    if privacy_on(win) {
        return Err("Im Privatmodus ist der Export gesperrt.".to_string());
    }
    let gs = win.global::<SettingsImportState>();
    let list_from = crate::daily::range_from(win.get_range_index());
    view::export_period(
        gs.get_export_period(),
        &view::today(now_secs()),
        &gs.get_date_from(),
        &gs.get_date_to(),
        &list_from,
    )
}

fn export_account(win: &MainWindow) -> Result<(i64, Period), String> {
    let gs = win.global::<SettingsImportState>();
    let (id, _) = usize::try_from(gs.get_acc())
        .ok()
        .and_then(|i| with_ui(|u| u.accounts.get(i).cloned()))
        .ok_or("Kein Konto vorhanden.".to_string())?;
    Ok((id, export_period(win)?))
}

/// Recomputes the hint below the period row; a non-empty hint disables both export buttons.
fn update_period_hint(win: &MainWindow) {
    let gs = win.global::<SettingsImportState>();
    let hint = view::export_period_hint(
        gs.get_export_period(),
        &gs.get_date_from(),
        &gs.get_date_to(),
    );
    gs.set_period_hint(hint.into());
}

/// One account's bookings in the period as CSV text (built by the core) and their count.
fn account_csv(
    v: &Vault,
    id: i64,
    (from, to): &Period,
    opts: view::CsvOptions,
    with_balance: bool,
) -> Result<(String, usize), String> {
    let page = v
        .list_transactions(BookingFilter {
            account_ids: vec![id],
            date_from: from.clone(),
            date_to: to.clone(),
            limit: 0,
            with_balances: opts.running_balance,
            ..Default::default()
        })
        .map_err(|e| e.to_string())?;
    let balances: HashMap<i64, i64> = page
        .running_balances
        .iter()
        .map(|b| (b.booking_id, b.balance_cents))
        .collect();
    let mut list = page.transactions;
    list.sort_by(|a, b| a.booking_date.cmp(&b.booking_date).then(a.id.cmp(&b.id)));
    let balance_lines = if with_balance {
        v.list_accounts()
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|a| a.id == id)
            .map(|a| ExportCsvBalanceLine {
                name: a.name,
                balance_cents: a.balance_cents,
                currency: a.currency,
                date: a.balance_date,
            })
            .collect()
    } else {
        vec![]
    };
    let rows = view::export_rows(&list, &balances);
    Ok((
        export_csv(rows, view::csv_columns(opts), balance_lines),
        list.len(),
    ))
}

fn wire_export(win: &MainWindow) {
    let gs = win.global::<SettingsImportState>();

    let weak = win.as_weak();
    gs.on_period_changed(move || {
        if let Some(win) = weak.upgrade() {
            update_period_hint(&win);
        }
    });

    let weak = win.as_weak();
    gs.on_export_csv(move || {
        let (Some(win), Some(v)) = (weak.upgrade(), vault_now()) else {
            return;
        };
        let gs = win.global::<SettingsImportState>();
        // `None` = one file per account in a chosen folder.
        let target = if gs.get_per_account() {
            export_period(&win).map(|p| (None, p))
        } else {
            export_account(&win).map(|(id, p)| (Some(id), p))
        };
        let (id, period) = match target {
            Ok(x) => x,
            Err(e) => return set_status(&win, e),
        };
        let opts = view::CsvOptions {
            category: gs.get_col_category(),
            note: gs.get_col_note(),
            iban: gs.get_col_iban(),
            running_balance: gs.get_col_running_balance(),
        };
        let with_balance = gs.get_col_balance();
        spawn(
            &win,
            "Wird exportiert …",
            move || {
                let today = view::today(now_secs());
                if let Some(id) = id {
                    let (csv, n) = account_csv(&v, id, &period, opts, with_balance)?;
                    let name = view::export_file_name("csv", &today);
                    let path = save_file("Umsätze exportieren", &name)
                        .ok_or("Export abgebrochen.".to_string())?;
                    write_bytes(&path, csv.as_bytes())?;
                    return Ok(format!("{n} Umsätze exportiert."));
                }
                // Current accounts, not the menu cache: an import in this dialog may have
                // created an archive account since it was opened.
                let accounts: Vec<(i64, String)> = v
                    .list_accounts()
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .map(|a| (a.id, a.name))
                    .collect();
                if accounts.is_empty() {
                    return Err("Kein Konto vorhanden.".to_string());
                }
                let dir = pick_dir("Ordner für die CSV-Dateien wählen")
                    .ok_or("Export abgebrochen.".to_string())?;
                let names: Vec<String> = accounts.iter().map(|(_, n)| n.clone()).collect();
                let files = view::per_account_file_names(&names, &today, |f| dir.join(f).exists());
                let mut total = 0;
                for ((id, _), name) in accounts.iter().zip(&files) {
                    let (csv, n) = account_csv(&v, *id, &period, opts, with_balance)?;
                    write_new(&dir, name, csv.as_bytes())?;
                    total += n;
                }
                // Counts only: no account names or paths in the status line (privacy mode).
                Ok(match files.len() {
                    1 => format!("1 Datei mit {total} Umsätzen gespeichert."),
                    k => format!("{k} Dateien mit {total} Umsätzen gespeichert."),
                })
            },
            set_status,
        );
    });

    let weak = win.as_weak();
    gs.on_export_camt(move || {
        let (Some(win), Some(v)) = (weak.upgrade(), vault_now()) else {
            return;
        };
        let (id, (from, to)) = match export_account(&win) {
            Ok(x) => x,
            Err(e) => return set_status(&win, e),
        };
        let version = view::camt_version(win.global::<SettingsImportState>().get_camt_version());
        spawn(
            &win,
            "Wird exportiert …",
            move || {
                let now = now_secs();
                let xml = v
                    .camt053_export(Camt053Request {
                        account_id: id,
                        date_from: from,
                        date_to: to,
                        booking_ids: None,
                        version,
                        created_at: view::iso_utc(now),
                    })
                    .map_err(|e| e.to_string())?;
                let name = view::export_file_name("camt", &view::today(now));
                let path = save_file("CAMT.053 speichern", &name)
                    .ok_or("Export abgebrochen.".to_string())?;
                write_bytes(&path, xml.as_bytes())?;
                Ok("CAMT.053 gespeichert.".to_string())
            },
            set_status,
        );
    });
}

// ---- backup ---------------------------------------------------------------------------------------

fn wire_backup(win: &MainWindow) {
    let gs = win.global::<SettingsImportState>();

    let weak = win.as_weak();
    gs.on_write_backup(move || {
        let (Some(win), Some(v)) = (weak.upgrade(), vault_now()) else {
            return;
        };
        let gs = win.global::<SettingsImportState>();
        let (pw, repeat) = (
            gs.get_backup_password().to_string(),
            gs.get_backup_repeat().to_string(),
        );
        if let Err(e) = view::validate_backup_password(&pw, &repeat, true) {
            return set_status(&win, e);
        }
        spawn(
            &win,
            "Sicherung wird geschrieben …",
            move || {
                let name = format!("girocrab-sicherung-{}.girocrab", view::today(now_secs()));
                let path = save_file("Sicherung speichern", &name)
                    .ok_or("Sicherung abgebrochen.".to_string())?;
                v.write_backup(path.display().to_string(), pw)
                    .map_err(|e| e.to_string())?;
                Ok("Sicherung geschrieben.".to_string())
            },
            |win, text| {
                let gs = win.global::<SettingsImportState>();
                gs.set_backup_password(SharedString::new());
                gs.set_backup_repeat(SharedString::new());
                set_status(win, text);
            },
        );
    });

    let weak = win.as_weak();
    gs.on_restore_backup(move || {
        let (Some(win), Some(v)) = (weak.upgrade(), vault_now()) else {
            return;
        };
        let gs = win.global::<SettingsImportState>();
        let pw = gs.get_backup_password().to_string();
        if let Err(e) = view::validate_backup_password(&pw, "", false) {
            return set_status(&win, e);
        }
        // Replacing the vault needs a second click.
        if !with_ui(|u| std::mem::replace(&mut u.restore_armed, true)) {
            gs.set_restore_armed(true);
            return set_status(
                &win,
                "Die aktuellen Daten werden ersetzt. Zum Bestätigen noch einmal klicken.",
            );
        }
        with_ui(|u| u.restore_armed = false);
        gs.set_restore_armed(false);
        let vault = v.clone();
        spawn(
            &win,
            "Sicherung wird eingespielt …",
            move || {
                let path = pick_file(
                    "Sicherung wählen",
                    "GiroCrab-Sicherung",
                    &["girocrab", "db", "sqlite"],
                )
                .ok_or("Wiederherstellung abgebrochen.".to_string())?;
                v.restore_backup(path.display().to_string(), pw)
                    .map_err(|e| e.to_string())?;
                Ok("Sicherung wiederhergestellt.".to_string())
            },
            move |win, text| {
                win.global::<SettingsImportState>()
                    .set_backup_password(SharedString::new());
                refresh_main(win, &vault);
                set_status(win, text);
            },
        );
    });
}

// ---- settings -------------------------------------------------------------------------------------

fn load_settings(win: &MainWindow, v: &Arc<Vault>) {
    let v = v.clone();
    spawn(
        win,
        "Wird geladen …",
        move || {
            let list = v.list_settings().map_err(|e| e.to_string())?;
            let mode = v.fints_product_id_mode().map_err(|e| e.to_string())?;
            let source = v.fints_product_id_source().map_err(|e| e.to_string())?;
            let prefs = update_prefs(&v);
            let refunds = v.net_refunds_in_stats().map_err(|e| e.to_string())?;
            Ok((list, mode, source, prefs, refunds))
        },
        |win, (list, mode, source, prefs, refunds)| {
            show_update_prefs(win, prefs);
            let gs = win.global::<SettingsImportState>();
            gs.set_net_refunds(refunds);
            gs.set_settings(rows(view::setting_lines(&list)));
            gs.set_sel_setting(-1);
            gs.set_mode(view::product_mode_index(&mode));
            gs.set_mode_source(view::product_source_text(&source).into());
            gs.set_status(SharedString::new());
            with_ui(|u| u.settings = list);
        },
    );
}

fn wire_settings(win: &MainWindow) {
    wire_updates(win);
    let gs = win.global::<SettingsImportState>();

    let weak = win.as_weak();
    gs.on_setting_select(move |i| {
        let Some(win) = weak.upgrade() else { return };
        let Some(entry) = usize::try_from(i)
            .ok()
            .and_then(|i| with_ui(|u| u.settings.get(i).cloned()))
        else {
            return;
        };
        let gs = win.global::<SettingsImportState>();
        gs.set_sel_setting(i);
        gs.set_setting_label(entry.label.into());
        gs.set_setting_help(entry.help.into());
        gs.set_setting_secret(entry.is_secret);
        // the core leaves a secret's value empty: it is never shown
        gs.set_setting_value(entry.value.into());
    });

    let weak = win.as_weak();
    gs.on_setting_save(move || {
        let (Some(win), Some(v)) = (weak.upgrade(), vault_now()) else {
            return;
        };
        let gs = win.global::<SettingsImportState>();
        let Some(key) = usize::try_from(gs.get_sel_setting())
            .ok()
            .and_then(|i| with_ui(|u| u.settings.get(i).map(|s| s.key.clone())))
        else {
            return;
        };
        let value = gs.get_setting_value().to_string();
        gs.set_setting_value(SharedString::new());
        let vault = v.clone();
        spawn(
            &win,
            "Wird gespeichert …",
            move || {
                v.set_setting(key.clone(), value)
                    .map_err(|e| e.to_string())?;
                // read back through the core: proves the value is stored
                v.get_setting(key).map_err(|e| e.to_string())?;
                Ok(())
            },
            move |win, ()| {
                load_settings(win, &vault);
                set_status(win, "Gespeichert.");
            },
        );
    });

    let weak = win.as_weak();
    gs.on_net_refunds_changed(move |on| {
        let Some(win) = weak.upgrade() else { return };
        let Some(v) = vault_now() else {
            win.global::<SettingsImportState>().set_net_refunds(!on);
            set_status(&win, "Tresor ist gesperrt.");
            return;
        };
        let vault = v.clone();
        spawn(
            &win,
            "Wird gespeichert …",
            move || Ok(save_net_refunds(&v, on)),
            move |win, (stored, saved)| {
                // the vault was locked or swapped meanwhile: its result belongs to no dialog
                if !crate::is_current(&vault) {
                    return;
                }
                win.global::<SettingsImportState>().set_net_refunds(stored);
                match saved {
                    // every total, budget and chart reads the setting: reload like a refund link change
                    Ok(()) => {
                        set_status(win, "Gespeichert.");
                        crate::reload(win, vault);
                    }
                    Err(e) => set_status(win, e),
                }
            },
        );
    });

    let weak = win.as_weak();
    gs.on_mode_changed(move |index| {
        let (Some(win), Some(v)) = (weak.upgrade(), vault_now()) else {
            return;
        };
        let mode = view::product_mode_value(index).to_string();
        spawn(
            &win,
            "Wird gespeichert …",
            move || {
                v.set_fints_product_id_mode(mode)
                    .map_err(|e| e.to_string())?;
                v.fints_product_id_source().map_err(|e| e.to_string())
            },
            |win, source| {
                win.global::<SettingsImportState>()
                    .set_mode_source(view::product_source_text(&source).into());
                set_status(win, "");
            },
        );
    });
}

/// Stores the switch and reads it back; the stored state is what the checkbox shows, also when
/// saving failed.
fn save_net_refunds(v: &Vault, on: bool) -> (bool, Result<(), String>) {
    let saved = v.set_net_refunds_in_stats(on).map_err(|e| e.to_string());
    let stored = v.net_refunds_in_stats().unwrap_or(!on);
    (stored, saved)
}

// ---- Abrufprotokoll ----------------------------------------------------------------------------------

fn load_log(win: &MainWindow, v: &Arc<Vault>) {
    let gs = win.global::<SettingsImportState>();
    let search = gs.get_log_search().trim().to_string();
    let filter = SyncLogFilter {
        connection_id: None,
        only_errors: gs.get_log_errors(),
        search: Some(search).filter(|s| !s.is_empty()),
        limit: 500,
    };
    let v = v.clone();
    spawn(
        win,
        "Wird geladen …",
        move || v.list_sync_log(filter).map_err(|e| e.to_string()),
        |win, list| {
            let gs = win.global::<SettingsImportState>();
            gs.set_log(rows(view::log_lines(&list)));
            gs.set_log_summary(format!("{} Einträge (Zeiten in UTC)", list.len()).into());
            gs.set_status(SharedString::new());
            with_ui(|u| u.log = list);
        },
    );
}

fn wire_log(win: &MainWindow) {
    let gs = win.global::<SettingsImportState>();

    let weak = win.as_weak();
    gs.on_log_reload(move || {
        if let (Some(win), Some(v)) = (weak.upgrade(), vault_now()) {
            load_log(&win, &v);
        }
    });

    let weak = win.as_weak();
    gs.on_log_clear(move || {
        let (Some(win), Some(v)) = (weak.upgrade(), vault_now()) else {
            return;
        };
        if !with_ui(|u| std::mem::replace(&mut u.clear_armed, true)) {
            win.global::<SettingsImportState>().set_clear_armed(true);
            return;
        }
        with_ui(|u| u.clear_armed = false);
        win.global::<SettingsImportState>().set_clear_armed(false);
        let vault = v.clone();
        spawn(
            &win,
            "Wird geleert …",
            move || v.clear_sync_log().map_err(|e| e.to_string()),
            move |win, ()| load_log(win, &vault),
        );
    });

    let weak = win.as_weak();
    gs.on_log_export(move || {
        let Some(win) = weak.upgrade() else { return };
        let list = with_ui(|u| u.log.clone());
        spawn(
            &win,
            "Wird gespeichert …",
            move || {
                let name = format!("girocrab-abrufprotokoll-{}.xml", view::today(now_secs()));
                let path = save_file("Abrufprotokoll speichern", &name)
                    .ok_or("Export abgebrochen.".to_string())?;
                write_bytes(&path, view::log_xml(&list).as_bytes())?;
                Ok(format!("{} Einträge gespeichert.", list.len()))
            },
            set_status,
        );
    });
}

// ---- announcements ----------------------------------------------------------------------------------

const DISMISSED_KEY: &str = "announcements.dismissed";

/// Loads the applicable, not yet dismissed announcements into the banner and the Mitteilungen tab.
pub fn refresh_news(win: &MainWindow, v: &Arc<Vault>) {
    let setting = v
        .get_setting(DISMISSED_KEY.into())
        .ok()
        .flatten()
        .unwrap_or_default();
    let dismissed = announcements::dismissed_ids(&setting);
    let list: Vec<Announcement> = announcements::current(
        env!("CARGO_PKG_VERSION"),
        &crate::daily::date_days_ago(0),
        "de",
    )
    .into_iter()
    .filter(|a| !dismissed.contains(&a.id))
    .collect();
    let gs = win.global::<SettingsImportState>();
    let first = list.first();
    gs.set_banner_title(first.map(|a| a.title.clone()).unwrap_or_default().into());
    gs.set_banner_important(first.is_some_and(|a| a.important));
    gs.set_news(rows(
        list.iter()
            .map(|a| Line {
                title: a.title.clone(),
                detail: a.summary.clone(),
            })
            .collect(),
    ));
    gs.set_sel_news(-1);
    gs.set_news_body(SharedString::new());
    gs.set_news_has_link(false);
    with_ui(|u| u.news = list);
    auto_check_updates(win, v);
}

// ---- updates ---------------------------------------------------------------------------------------

use crate::updates::{AUTO_KEY, CHANNEL_KEY, Channel, LAST_KEY};

/// At most one automatic check per app session (and per day, via `updates.last_check`).
static AUTO_CHECKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn update_prefs(v: &Vault) -> (Channel, bool, String) {
    let get = |k: &str| v.get_setting(k.into()).ok().flatten();
    (
        crate::updates::channel_setting(get(CHANNEL_KEY).as_deref(), env!("CARGO_PKG_VERSION")),
        crate::updates::auto_enabled(get(AUTO_KEY).as_deref()),
        get(LAST_KEY).unwrap_or_default(),
    )
}

fn show_update_prefs(win: &MainWindow, (channel, auto, last): (Channel, bool, String)) {
    let us = win.global::<crate::UpdateState>();
    us.set_channel(channel.index());
    us.set_auto(auto);
    us.set_last(if last.is_empty() {
        "Noch nie".into()
    } else {
        last.into()
    });
}

fn auto_check_updates(win: &MainWindow, v: &Arc<Vault>) {
    let (channel, auto, last) = update_prefs(v);
    show_update_prefs(win, (channel, auto, last.clone()));
    if !auto || last == crate::daily::date_days_ago(0) || AUTO_CHECKED.swap(true, Ordering::SeqCst)
    {
        return;
    }
    run_update_check(win, Some(v.clone()), channel, true);
}

/// Network on a worker thread, never on the UI thread; `quiet` = automatic check, which
/// reports only a found update.
fn run_update_check(win: &MainWindow, v: Option<Arc<Vault>>, channel: Channel, quiet: bool) {
    let us = win.global::<crate::UpdateState>();
    if us.get_checking() {
        return;
    }
    us.set_checking(true);
    us.set_notes(SharedString::new());
    us.set_has_download(false);
    if !quiet {
        us.set_result("Wird gesucht …".into());
    }
    let weak = win.as_weak();
    std::thread::spawn(move || {
        let current = env!("CARGO_PKG_VERSION");
        let result = crate::updates::check(&crate::updates::CurlFetcher, current, channel);
        let today = crate::daily::date_days_ago(0);
        if result.is_ok()
            && let Some(v) = &v
        {
            let _ = v.set_setting(LAST_KEY.into(), today.clone());
        }
        let _ = weak.upgrade_in_event_loop(move |win| {
            let us = win.global::<crate::UpdateState>();
            us.set_checking(false);
            if result.is_ok() {
                us.set_last(today.into());
            }
            match result {
                Ok(Some(u)) => {
                    us.set_result(
                        format!(
                            "GiroCrab {} ist verfügbar (installiert: {current}).",
                            u.version
                        )
                        .into(),
                    );
                    us.set_notes(u.notes.into());
                    us.set_has_download(!u.url.is_empty());
                    us.set_download_url(u.url.into());
                }
                Ok(None) if !quiet => {
                    us.set_result(format!("GiroCrab {current} ist aktuell.").into())
                }
                Err(e) if !quiet => us.set_result(e.into()),
                _ => {}
            }
        });
    });
}

fn save_update_setting(win: &MainWindow, key: &'static str, value: &'static str) {
    let Some(v) = vault_now() else {
        set_status(win, "Tresor ist gesperrt.");
        return;
    };
    spawn(
        win,
        "Wird gespeichert …",
        move || {
            v.set_setting(key.into(), value.into())
                .map_err(|e| e.to_string())
        },
        |win, ()| set_status(win, "Gespeichert."),
    );
}

fn wire_updates(win: &MainWindow) {
    let us = win.global::<crate::UpdateState>();

    let weak = win.as_weak();
    us.on_channel_changed(move |i| {
        if let Some(win) = weak.upgrade() {
            save_update_setting(&win, CHANNEL_KEY, Channel::from_index(i).key());
        }
    });

    let weak = win.as_weak();
    us.on_auto_changed(move |on| {
        if let Some(win) = weak.upgrade() {
            save_update_setting(&win, AUTO_KEY, if on { "1" } else { "0" });
        }
    });

    let weak = win.as_weak();
    us.on_check_now(move || {
        let Some(win) = weak.upgrade() else { return };
        let channel = Channel::from_index(win.global::<crate::UpdateState>().get_channel());
        run_update_check(&win, vault_now(), channel, false);
    });

    let weak = win.as_weak();
    us.on_open_download(move || {
        let Some(win) = weak.upgrade() else { return };
        let us = win.global::<crate::UpdateState>();
        if let Err(e) = crate::updates::open_download(&us.get_download_url()) {
            us.set_result(e.into());
        }
    });
}

fn dismiss_news(win: &MainWindow, v: &Arc<Vault>, id: &str) {
    let setting = v
        .get_setting(DISMISSED_KEY.into())
        .ok()
        .flatten()
        .unwrap_or_default();
    match v.set_setting(
        DISMISSED_KEY.into(),
        announcements::with_dismissed(&setting, id),
    ) {
        Ok(()) => refresh_news(win, v),
        Err(e) => set_status(win, e.to_string()),
    }
}

fn wire_news(win: &MainWindow) {
    let gs = win.global::<SettingsImportState>();

    let weak = win.as_weak();
    gs.on_news_select(move |i| {
        let Some(win) = weak.upgrade() else { return };
        let gs = win.global::<SettingsImportState>();
        let item = usize::try_from(i)
            .ok()
            .and_then(|i| with_ui(|u| u.news.get(i).cloned()));
        gs.set_sel_news(if item.is_some() { i } else { -1 });
        gs.set_news_body(
            item.as_ref()
                .map(|a| announcements::plain(&a.body))
                .unwrap_or_default()
                .into(),
        );
        gs.set_news_has_link(item.is_some_and(|a| !a.link.is_empty()));
    });

    let weak = win.as_weak();
    gs.on_news_dismiss(move || {
        let (Some(win), Some(v)) = (weak.upgrade(), vault_now()) else {
            return;
        };
        let sel = win.global::<SettingsImportState>().get_sel_news();
        let id = usize::try_from(sel)
            .ok()
            .and_then(|i| with_ui(|u| u.news.get(i).map(|a| a.id.clone())));
        if let Some(id) = id {
            dismiss_news(&win, &v, &id);
        }
    });

    let weak = win.as_weak();
    gs.on_news_open_link(move || {
        let Some(win) = weak.upgrade() else { return };
        let sel = win.global::<SettingsImportState>().get_sel_news();
        let link = usize::try_from(sel)
            .ok()
            .and_then(|i| with_ui(|u| u.news.get(i).map(|a| a.link.clone())));
        if let Some(link) = link
            && let Err(e) =
                crate::sync_flow::safe_https_url(&link).and_then(crate::sync_flow::open_in_browser)
        {
            set_status(&win, e);
        }
    });

    let weak = win.as_weak();
    gs.on_banner_dismiss(move || {
        let (Some(win), Some(v)) = (weak.upgrade(), vault_now()) else {
            return;
        };
        if let Some(id) = with_ui(|u| u.news.first().map(|a| a.id.clone())) {
            dismiss_news(&win, &v, &id);
        }
    });

    let weak = win.as_weak();
    gs.on_banner_show(move || {
        let Some(win) = weak.upgrade() else { return };
        let gs = win.global::<SettingsImportState>();
        gs.set_tab(5);
        gs.set_open(true);
        gs.invoke_tab_changed(5);
        gs.invoke_news_select(0);
    });
}

#[cfg(test)]
mod tests {
    use super::{write_new, write_new_with};

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir =
            std::env::temp_dir().join(format!("girocrab-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn refund_switch_persists_in_the_vault() {
        let dir = temp_dir("refund-switch");
        let path = dir.join("v.db").display().to_string();
        let v = girocrab_core::Vault::open(path.clone(), vec![7; 32]).unwrap();
        assert!(!v.net_refunds_in_stats().unwrap(), "off by default");
        assert_eq!(super::save_net_refunds(&v, true), (true, Ok(())));
        drop(v);
        let v = girocrab_core::Vault::open(path, vec![7; 32]).unwrap();
        assert!(v.net_refunds_in_stats().unwrap());
        assert_eq!(super::save_net_refunds(&v, false), (false, Ok(())));
        drop(v);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_write_leaves_no_partial_file() {
        use std::io::Write;
        let dir = temp_dir("write-fail");
        std::fs::write(dir.join("vorher.csv"), b"alt").unwrap();
        let r = write_new_with(&dir, "a.csv", |f| {
            f.write_all(b"teil")?;
            Err(std::io::Error::other("Datenträger voll"))
        });
        assert!(r.is_err());
        assert!(!dir.join("a.csv").exists());
        assert!(!dir.join("a-2.csv").exists());
        // files written before stay untouched
        assert_eq!(std::fs::read(dir.join("vorher.csv")).unwrap(), b"alt");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn per_account_write_never_replaces_a_file() {
        let dir = temp_dir("write-new");
        // appeared after the names were planned
        std::fs::write(dir.join("a.csv"), b"fremd").unwrap();
        write_new(&dir, "a.csv", b"neu").unwrap();
        assert_eq!(std::fs::read(dir.join("a.csv")).unwrap(), b"fremd");
        assert_eq!(std::fs::read(dir.join("a-2.csv")).unwrap(), b"neu");
        // a free name is written directly
        write_new(&dir, "b.csv", b"frei").unwrap();
        assert_eq!(std::fs::read(dir.join("b.csv")).unwrap(), b"frei");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
