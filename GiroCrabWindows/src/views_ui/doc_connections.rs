//! Belege & Postbox: bank connection filter and the "Dokumente abrufen" switch per connection
//! (#954, #1042). Mirrors `documentsFilterConnections` (`AppModel+Documents.swift`), the filter
//! in `DocumentsUI.swift` and the switch in `ConnectionsUI.swift`.
//!
//! The worker of the Belege view fills this state in [`refresh`]; the UI thread reads it in
//! [`view`]. It sits behind a mutex because the worker cannot reach the thread-local view state.
//! [`clear`] runs on lock and bumps an epoch, so a load still in flight cannot bring names back.

use std::collections::HashSet;
use std::sync::{Mutex, MutexGuard};

use super::{DocumentFilter, Vault};

const ALL: &str = "Alle Bankzugänge";
const WITHOUT_ACCOUNT: &str = "Ohne Konto";

/// One bank connection as the Belege view needs it.
struct DocConnection {
    id: i64,
    name: String,
    /// Only connections with accounts are offered in the filter (as on the Mac).
    has_accounts: bool,
    /// The bank delivers documents (`Vault::documents_fetch_supported`).
    supported: bool,
    enabled: bool,
    /// Ids of the documents fetched through this connection.
    docs: Vec<i64>,
}

struct Shared {
    epoch: u64,
    /// Counts the saved switch changes; a load that started before one must not overwrite it.
    saves: u64,
    conns: Vec<DocConnection>,
    /// Connection the shown switch belongs to.
    toggle: Option<i64>,
}

static SHARED: Mutex<Shared> = Mutex::new(Shared {
    epoch: 0,
    saves: 0,
    conns: Vec::new(),
    toggle: None,
});

fn lock() -> MutexGuard<'static, Shared> {
    SHARED.lock().unwrap_or_else(|p| p.into_inner())
}

enum Choice {
    All,
    Docs(HashSet<i64>),
    WithoutAccount,
}

/// What the Belege view shows for the current filter index.
pub(crate) struct DocView {
    /// Filter entries; empty = no filter (fewer than two connections with accounts).
    pub(crate) names: Vec<String>,
    /// The index, clamped to the entries.
    pub(crate) index: usize,
    /// `Some(enabled)` when the "Dokumente abrufen" switch is shown.
    pub(crate) toggle: Option<bool>,
    choice: Choice,
}

impl DocView {
    pub(crate) fn keeps(&self, doc_id: i64, has_account: bool) -> bool {
        match &self.choice {
            Choice::All => true,
            Choice::Docs(ids) => ids.contains(&doc_id),
            Choice::WithoutAccount => !has_account,
        }
    }
}

fn build(conns: &[DocConnection], index: usize) -> (DocView, Option<i64>) {
    let listed: Vec<&DocConnection> = conns.iter().filter(|c| c.has_accounts).collect();
    if listed.len() < 2 {
        let supported: Vec<&DocConnection> = conns.iter().filter(|c| c.supported).collect();
        let target = (supported.len() == 1).then(|| supported[0]);
        let view = DocView {
            names: Vec::new(),
            index: 0,
            toggle: target.map(|c| c.enabled),
            choice: Choice::All,
        };
        return (view, target.map(|c| c.id));
    }
    let index = if index > listed.len() + 1 { 0 } else { index };
    let mut names = vec![ALL.to_string()];
    names.extend(listed.iter().map(|c| c.name.clone()));
    names.push(WITHOUT_ACCOUNT.to_string());
    let (choice, target) = match index {
        0 => (Choice::All, None),
        i if i <= listed.len() => {
            let c = listed[i - 1];
            let ids = c.docs.iter().copied().collect();
            (Choice::Docs(ids), c.supported.then_some(c))
        }
        _ => (Choice::WithoutAccount, None),
    };
    let view = DocView {
        names,
        index,
        toggle: target.map(|c| c.enabled),
        choice,
    };
    (view, target.map(|c| c.id))
}

/// The current session epoch; bumped by [`clear`]. Take it before starting work that is stored later.
pub(crate) fn epoch() -> u64 {
    lock().epoch
}

/// Worker side: reads the connections, their switch state and document ids. `epoch` is the value
/// of [`epoch`] from before the load started; after a lock the result is dropped.
pub(crate) fn refresh(vault: &Vault, epoch: u64) -> Result<(), String> {
    // before the first read: a switch saved from here on is newer than this snapshot
    let saves = lock().saves;
    let accounts = vault.list_accounts().map_err(|e| e.to_string())?;
    let mut conns = Vec::new();
    for c in vault.list_bank_connections().map_err(|e| e.to_string())? {
        let filter = DocumentFilter {
            connection_id: Some(c.id),
            ..Default::default()
        };
        let docs = vault
            .list_all_documents(filter)
            .map_err(|e| e.to_string())?;
        conns.push(DocConnection {
            id: c.id,
            has_accounts: accounts.iter().any(|a| a.connection_id == Some(c.id)),
            supported: vault.documents_fetch_supported(c.id).unwrap_or(false),
            enabled: vault.documents_fetch_enabled(c.id).unwrap_or(true),
            docs: docs.into_iter().map(|d| d.id).collect(),
            name: c.display_name,
        });
    }
    store(epoch, saves, conns);
    Ok(())
}

/// Publishes a load. After a lock (`epoch` changed) it is dropped; if a switch was saved
/// since the load began (`saves` changed), the cached switch states win over the snapshot.
fn store(epoch: u64, saves: u64, mut conns: Vec<DocConnection>) -> bool {
    let mut s = lock();
    let current = s.epoch == epoch;
    if current {
        if s.saves != saves {
            for c in conns.iter_mut() {
                if let Some(old) = s.conns.iter().find(|o| o.id == c.id) {
                    c.enabled = old.enabled;
                }
            }
        }
        s.conns = conns;
    }
    current
}

/// UI side: the filter entries and switch for `index`; remembers the switch's connection.
pub(crate) fn view(index: usize) -> DocView {
    let mut s = lock();
    let (view, target) = build(&s.conns, index);
    s.toggle = target;
    view
}

pub(crate) fn toggle_target() -> Option<i64> {
    lock().toggle
}

pub(crate) fn set_enabled(id: i64, on: bool) {
    let mut s = lock();
    s.saves = s.saves.wrapping_add(1);
    if let Some(c) = s.conns.iter_mut().find(|c| c.id == id) {
        c.enabled = on;
    }
}

/// On lock: forget names and ids, drop the results of loads still running.
pub(crate) fn clear() {
    let mut s = lock();
    s.epoch = s.epoch.wrapping_add(1);
    s.conns.clear();
    s.toggle = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn(id: i64, accounts: bool, supported: bool, docs: &[i64]) -> DocConnection {
        DocConnection {
            id,
            name: format!("Zugang {id}"),
            has_accounts: accounts,
            supported,
            enabled: true,
            docs: docs.to_vec(),
        }
    }

    #[test]
    fn filter_appears_from_two_connections_with_accounts() {
        let (v, target) = build(&[conn(1, true, true, &[10]), conn(2, false, false, &[])], 0);
        assert!(v.names.is_empty());
        assert_eq!((v.toggle, target), (Some(true), Some(1)));
        let (v, _) = build(&[conn(1, true, true, &[10]), conn(2, true, false, &[])], 0);
        assert_eq!(
            v.names,
            ["Alle Bankzugänge", "Zugang 1", "Zugang 2", "Ohne Konto"]
        );
    }

    #[test]
    fn choices_keep_the_matching_documents() {
        let conns = [conn(1, true, true, &[10, 11]), conn(2, true, false, &[12])];
        let (all, _) = build(&conns, 0);
        assert!(all.keeps(12, true) && all.keeps(13, false));
        let (first, target) = build(&conns, 1);
        assert!(first.keeps(10, true) && first.keeps(11, false) && !first.keeps(12, true));
        assert_eq!(target, Some(1));
        let (none, _) = build(&conns, 3);
        assert!(none.keeps(13, false) && !none.keeps(10, true));
    }

    #[test]
    fn switch_only_for_a_supported_connection_and_index_clamps() {
        let conns = [conn(1, true, true, &[]), conn(2, true, false, &[])];
        let (v, target) = build(&conns, 2);
        assert_eq!((v.toggle, target), (None, None));
        let (v, target) = build(&conns, 9);
        assert_eq!((v.index, target), (0, None));
        let (v, _) = build(&[conn(1, true, true, &[]), conn(2, true, true, &[])], 0);
        assert_eq!(v.toggle, None);
    }

    /// The tests below change the shared state: one at a time.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn serial() -> MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    #[test]
    fn clear_changes_the_epoch() {
        let _g = serial();
        let before = epoch();
        clear();
        assert_ne!(epoch(), before);
    }

    #[test]
    fn set_enabled_only_touches_the_given_connection() {
        let _g = serial();
        clear();
        let saves = lock().saves;
        assert!(store(
            epoch(),
            saves,
            vec![conn(1, true, true, &[]), conn(2, true, true, &[])]
        ));
        set_enabled(1, false);
        let s = lock();
        assert_eq!(s.conns.len(), 2);
        assert!(!s.conns[0].enabled);
        assert!(s.conns[1].enabled);
    }

    #[test]
    fn load_finishing_after_lock_is_dropped() {
        let _g = serial();
        let (epoch, saves) = {
            let s = lock();
            (s.epoch, s.saves)
        };
        clear();
        assert!(!store(epoch, saves, vec![conn(1, true, true, &[])]));
        assert!(lock().conns.is_empty());
        assert!(lock().toggle.is_none());
    }

    #[test]
    fn load_started_before_a_save_keeps_the_saved_switch() {
        let _g = serial();
        clear();
        let e = epoch();
        let before = lock().saves;
        assert!(store(e, before, vec![conn(1, true, true, &[])]));
        // a load starts (reads `saves`), then the switch is saved, then the load publishes
        let started = lock().saves;
        set_enabled(1, false);
        assert!(store(
            e,
            started,
            vec![conn(1, true, true, &[]), conn(2, true, true, &[])]
        ));
        let s = lock();
        assert_eq!(s.conns.len(), 2);
        assert!(!s.conns[0].enabled);
        assert!(s.conns[1].enabled);
    }

    #[test]
    fn load_without_a_save_replaces_the_cache() {
        let _g = serial();
        clear();
        let e = epoch();
        let saves = lock().saves;
        assert!(store(e, saves, vec![conn(1, true, true, &[])]));
        set_enabled(1, false);
        let saves = lock().saves;
        // a load that began after the save reads the saved value from the database
        assert!(store(e, saves, vec![conn(1, true, true, &[])]));
        assert!(lock().conns[0].enabled);
    }
}
