//! Slint wiring for "Offline-Konto anlegen" (#679). Validation lives in `offline.rs`; the core
//! write runs on a worker thread and is published only while the same vault is still unlocked.

use std::sync::{Arc, Mutex};

use girocrab_core::{AccountKind, Vault};
use slint::{ComponentHandle, SharedString};

use crate::{MainWindow, daily, offline, privacy_on, show};

type VaultSlot = Arc<Mutex<Option<Arc<Vault>>>>;

fn vault_of(slot: &VaultSlot) -> Option<Arc<Vault>> {
    slot.lock().ok().and_then(|v| v.clone())
}

/// Clears every dialog property; also called when the vault is locked.
pub fn reset(win: &MainWindow) {
    win.set_offline_open(false);
    win.set_offline_busy(false);
    win.set_offline_name(SharedString::new());
    win.set_offline_currency("EUR".into());
    win.set_offline_opening(SharedString::new());
    win.set_offline_status(SharedString::new());
}

pub fn wire(win: &MainWindow, vault: &VaultSlot) {
    let (weak, slot) = (win.as_weak(), vault.clone());
    win.on_offline_new(move || {
        let Some(win) = weak.upgrade() else { return };
        if vault_of(&slot).is_none() {
            return;
        }
        reset(&win);
        win.set_offline_mask(privacy_on(&win));
        win.set_offline_open(true);
    });

    let (weak, slot) = (win.as_weak(), vault.clone());
    win.on_offline_save(move |name, currency, opening| {
        let (Some(win), Some(v)) = (weak.upgrade(), vault_of(&slot)) else {
            return;
        };
        if win.get_offline_busy() {
            return;
        }
        let form = match offline::validate(&name, &currency, &opening) {
            Ok(f) => f,
            Err(e) => {
                win.set_offline_status(e.into());
                return;
            }
        };
        win.set_offline_busy(true);
        win.set_offline_status(SharedString::new());
        let (weak, slot) = (win.as_weak(), slot.clone());
        std::thread::spawn(move || {
            let result = v
                .create_offline_account(
                    form.name.clone(),
                    AccountKind::Cash,
                    form.currency.clone(),
                    form.opening_cents,
                )
                .map_err(|e| e.to_string());
            let _ = weak.upgrade_in_event_loop(move |win| {
                // Locked or swapped meanwhile: `reset` already cleared the dialog.
                let Some(current) = vault_of(&slot) else {
                    return;
                };
                if !Arc::ptr_eq(&current, &v) {
                    return;
                }
                win.set_offline_busy(false);
                match result {
                    Ok(id) => {
                        let label = daily::mask(&form.name, privacy_on(&win));
                        reset(&win);
                        show(&win, &current, Some(id));
                        win.set_sync_status(format!("Offline-Konto „{label}“ angelegt.").into());
                        win.invoke_refocus();
                    }
                    Err(e) => win.set_offline_status(e.into()),
                }
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn created_account_is_listed_as_offline() {
        let path = std::env::temp_dir().join(format!(
            "girocrab-win-offline-{}.sqlite",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let v = Vault::open(path.display().to_string(), vec![9; 32]).unwrap();
        let form = offline::validate("Haushaltskasse", "eur", "1.234,56").unwrap();
        let id = v
            .create_offline_account(
                form.name.clone(),
                AccountKind::Cash,
                form.currency.clone(),
                form.opening_cents,
            )
            .unwrap();
        assert!(v.list_offline_account_ids().unwrap().contains(&id));
        let _ = std::fs::remove_file(&path);
    }
}
