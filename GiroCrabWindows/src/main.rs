//! GiroCrab for Windows (MVP): Slint front end directly on `girocrab_core`.
//! Unlock / create vault, demo mode (`GIROCRAB_DEMO_SEED=1`), accounts with balances,
//! bookings of the selected account and a manual sync trigger.

mod policy;
mod vault_key;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use girocrab_core::{BookingFilter, SyncStep, Vault};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

slint::include_modules!();

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn vault_path() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("XDG_DATA_HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(
                std::env::var_os("HOME")
                    .or_else(|| std::env::var_os("USERPROFILE"))
                    .unwrap_or_default(),
            )
            .join(".local/share")
        })
        .join("GiroCrab");
    let _ = std::fs::create_dir_all(&base);
    base.join(if vault_key::bypassed() {
        "demo-vault.sqlite"
    } else {
        "vault.sqlite"
    })
}

fn open_vault(demo: bool) -> Result<Arc<Vault>, String> {
    let path = vault_path();
    if demo {
        // The demo vault is throw-away: always start from fresh demo data.
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }
    let allow_create = std::fs::metadata(&path).map_or(true, |m| m.len() == 0);
    let unlocked = vault_key::unlock(allow_create)?;
    let vault = Vault::open(path.display().to_string(), unlocked.key.to_vec())
        .map_err(|e| e.to_string())?;
    vault.seed_starter_categories().map_err(|e| e.to_string())?;
    if demo {
        vault.seed_demo_data().map_err(|e| e.to_string())?;
    }
    Ok(vault)
}

fn money(cents: i64, currency: &str) -> String {
    girocrab_core::format_cents(cents, currency.to_string())
}

fn account_rows(vault: &Vault) -> Result<(Vec<AccountRow>, String), String> {
    let accounts = vault.list_accounts().map_err(|e| e.to_string())?;
    let total: i64 = accounts
        .iter()
        .filter(|a| a.in_total && a.currency == "EUR")
        .map(|a| a.balance_cents)
        .sum();
    let rows = accounts
        .iter()
        .map(|a| AccountRow {
            id: a.id as i32,
            name: a.name.clone().into(),
            bank: a.bank_name.clone().into(),
            balance: money(a.balance_cents, &a.currency).into(),
            negative: a.balance_cents < 0,
        })
        .collect();
    Ok((rows, money(total, "EUR")))
}

fn booking_rows(vault: &Vault, account_id: i64) -> Result<Vec<BookingRow>, String> {
    let page = vault
        .list_transactions(BookingFilter {
            account_ids: vec![account_id],
            limit: 200,
            ..Default::default()
        })
        .map_err(|e| e.to_string())?;
    Ok(page
        .transactions
        .iter()
        .map(|b| BookingRow {
            date: b.booking_date.clone().into(),
            counterparty: b.counterparty.clone().into(),
            purpose: b.purpose.clone().into(),
            amount: money(b.amount_cents, &b.currency).into(),
            negative: b.amount_cents < 0,
        })
        .collect())
}

/// Manual sync: runs every connection to the end. Demo banks are simulated without network; a
/// real bank needs a PIN / TAN dialog, which the MVP does not have yet, so it is reported, not tried.
fn run_sync(vault: Arc<Vault>, demo: bool) -> Result<String, String> {
    if policy::apply(VERSION).all {
        return Err(
            "Abruf gesperrt: Diese Version ist durch die Sicherheitssperre gesperrt.".into(),
        );
    }
    let connections = vault.list_bank_connections().map_err(|e| e.to_string())?;
    if connections.is_empty() {
        return Ok("Keine Bankverbindung eingerichtet.".into());
    }
    if !demo {
        return Ok(
            "Der Bankabruf mit PIN und TAN folgt in einer späteren Version der Windows-App.".into(),
        );
    }
    let rt = tokio_runtime()?;
    let mut done = 0;
    for c in connections.iter().filter(|c| !c.all_accounts_archived) {
        let session = vault
            .clone()
            .begin_sync(c.id, String::new(), true)
            .map_err(|e| e.to_string())?;
        let step = rt.block_on(session.start()).map_err(|e| e.to_string())?;
        match step {
            SyncStep::Finished { .. } => done += 1,
            _ => rt.block_on(session.cancel()),
        }
    }
    Ok(format!("{done} Verbindung(en) abgerufen (Demo)."))
}

fn tokio_runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())
}

fn show(win: &MainWindow, vault: &Vault, selected: Option<i64>) {
    match account_rows(vault) {
        Ok((rows, total)) => {
            let first = rows.first().map(|r| r.id as i64);
            win.set_accounts(ModelRc::new(VecModel::from(rows)));
            win.set_total(total.into());
            let sel = selected.or(first);
            win.set_selected(sel.map_or(-1, |s| s as i32));
            let bookings = sel.map(|s| booking_rows(vault, s)).unwrap_or(Ok(vec![]));
            match bookings {
                Ok(b) => win.set_bookings(ModelRc::new(VecModel::from(b))),
                Err(e) => win.set_message(e.into()),
            }
            win.set_unlocked(true);
        }
        Err(e) => win.set_message(e.into()),
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let win = MainWindow::new()?;
    let demo_mode = vault_key::demo_seed_requested();
    let vault: Arc<Mutex<Option<Arc<Vault>>>> = Arc::new(Mutex::new(None));
    let demo_active = Arc::new(Mutex::new(demo_mode));
    win.set_first_run(std::fs::metadata(vault_path()).map_or(true, |m| m.len() == 0));

    let open = {
        let (weak, vault, demo_active) = (win.as_weak(), vault.clone(), demo_active.clone());
        move |demo: bool| {
            let Some(win) = weak.upgrade() else { return };
            policy::apply(VERSION);
            match open_vault(demo) {
                Ok(v) => {
                    *demo_active.lock().unwrap() = demo;
                    show(&win, &v, None);
                    *vault.lock().unwrap() = Some(v);
                    win.set_message(SharedString::new());
                }
                Err(e) => win.set_message(e.into()),
            }
        }
    };
    {
        let open = open.clone();
        win.on_unlock(move || open(vault_key::demo_seed_requested()));
    }
    {
        // "Demo-Modus" button: same as GIROCRAB_DEMO_SEED=1, but without touching the key store.
        let open = open.clone();
        win.on_demo(move || {
            std::env::set_var("GIROCRAB_DEMO_SEED", "1");
            open(true)
        });
    }
    {
        let (weak, vault) = (win.as_weak(), vault.clone());
        win.on_select_account(move |id| {
            let (Some(win), Some(v)) = (weak.upgrade(), vault.lock().unwrap().clone()) else {
                return;
            };
            win.set_selected(id);
            match booking_rows(&v, id as i64) {
                Ok(b) => win.set_bookings(ModelRc::new(VecModel::from(b))),
                Err(e) => win.set_sync_status(e.into()),
            }
        });
    }
    {
        let (weak, vault, demo_active) = (win.as_weak(), vault.clone(), demo_active.clone());
        win.on_sync(move || {
            let (Some(win), Some(v)) = (weak.upgrade(), vault.lock().unwrap().clone()) else {
                return;
            };
            let demo = *demo_active.lock().unwrap();
            win.set_busy(true);
            win.set_sync_status("Abruf läuft …".into());
            let weak = win.as_weak();
            // The core call blocks (vault mutex, network): never on the UI thread.
            std::thread::spawn(move || {
                let result = run_sync(v.clone(), demo);
                let _ = weak.upgrade_in_event_loop(move |win| {
                    let selected = Some(win.get_selected() as i64).filter(|s| *s >= 0);
                    show(&win, &v, selected);
                    win.set_busy(false);
                    win.set_sync_status(result.unwrap_or_else(|e| e).into());
                });
            });
        });
    }

    if demo_mode {
        open(true);
    }
    win.run()
}
