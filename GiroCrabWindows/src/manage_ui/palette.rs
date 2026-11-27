//! Command palette (Ctrl+K): entries, filter, selection index and window wiring.
//! Mac counterpart: `GiroCrabApp/Sources/GiroCrabUI/CommandPalette.swift`.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::{MainWindow, VaultSlot, context, run_command};
use crate::manage::{self, Command, Context};

/// Like the Mac: at most 14 hits.
const MAX_ROWS: usize = 14;

/// Fixed commands with the `Ctrl+<key>` shortcut they run through `manage::route`.
const COMMANDS: [(&str, char); 5] = [
    ("Suchen", 'f'),
    ("Konten abrufen", 'r'),
    ("Datei importieren", 'i'),
    ("Neue Überweisung", 'n'),
    ("Tresor sperren", 'l'),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    /// Runs like `Ctrl+<key>`, with the same gates (busy, transfer accounts, lock).
    Shortcut(char),
    Account(i64),
}

#[derive(Debug, Clone)]
struct Item {
    title: String,
    subtitle: String,
    /// Masked in privacy mode; the search still runs on the plain title.
    personal: bool,
    action: Action,
}

fn items(accounts: Vec<(i64, String, String)>) -> Vec<Item> {
    let commands = COMMANDS.iter().map(|&(title, key)| Item {
        title: title.into(),
        subtitle: format!("Befehl · Strg+{}", key.to_ascii_uppercase()),
        personal: false,
        action: Action::Shortcut(key),
    });
    let accounts = accounts.into_iter().map(|(id, name, bank)| Item {
        title: name,
        subtitle: format!("Konto · {bank}"),
        personal: true,
        action: Action::Account(id),
    });
    commands.chain(accounts).collect()
}

/// Indexes of the entries whose title or subtitle contains `query` (case-insensitive).
fn filter(items: &[Item], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    items
        .iter()
        .enumerate()
        .filter(|(_, it)| {
            q.is_empty()
                || it.title.to_lowercase().contains(&q)
                || it.subtitle.to_lowercase().contains(&q)
        })
        .map(|(i, _)| i)
        .take(MAX_ROWS)
        .collect()
}

/// Moves the selection by `delta`, wrapping at both ends; 0 for an empty list.
fn step(index: usize, delta: i32, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    (index as i64 + i64::from(delta)).rem_euclid(len as i64) as usize
}

/// The command a palette entry runs: shortcuts go through `manage::route`.
fn command(action: Action, ctx: &Context) -> Option<Command> {
    match action {
        Action::Shortcut(key) => manage::route(&key.to_string(), true, ctx),
        Action::Account(_) => None,
    }
}

#[derive(Default)]
struct State {
    items: Vec<Item>,
    shown: Vec<usize>,
    index: usize,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::default();
}

/// Counts palette sessions: an account list that arrives after the palette was closed or
/// reopened (or the vault locked) is dropped.
static SESSION: AtomicU64 = AtomicU64::new(0);

/// Adds the account entries loaded on a worker to the open palette, if it is still the
/// session that asked for them. Keeps the typed query and the entry selected meanwhile
/// (read here on the UI thread, so a newer arrow-key choice is never overwritten).
fn apply_accounts(win: &MainWindow, session: u64, list: Vec<(i64, String, String)>) {
    if SESSION.load(Ordering::SeqCst) != session || !win.get_cmd_open() {
        return;
    }
    let keep = STATE.with_borrow_mut(|s| {
        let keep = s.shown.get(s.index).map(|&i| s.items[i].action);
        s.items = items(list);
        keep
    });
    show(win, &win.get_cmd_query(), keep);
}

/// Row of `keep` in the refreshed list; 0 when nothing was selected or the entry is gone.
fn reselect(items: &[Item], shown: &[usize], keep: Option<Action>) -> usize {
    keep.and_then(|a| shown.iter().position(|&i| items[i].action == a))
        .unwrap_or(0)
}

/// Filters by `query` and selects `keep` if it is still listed (a new query passes `None`).
fn show(win: &MainWindow, query: &str, keep: Option<Action>) {
    let (mut titles, mut subtitles, mut personal) = (Vec::new(), Vec::new(), Vec::new());
    let index = STATE.with_borrow_mut(|s| {
        s.shown = filter(&s.items, query);
        s.index = reselect(&s.items, &s.shown, keep);
        for &i in &s.shown {
            let it = &s.items[i];
            titles.push(SharedString::from(it.title.as_str()));
            subtitles.push(SharedString::from(it.subtitle.as_str()));
            personal.push(it.personal);
        }
        s.index
    });
    win.set_cmd_titles(ModelRc::new(VecModel::from(titles)));
    win.set_cmd_subtitles(ModelRc::new(VecModel::from(subtitles)));
    win.set_cmd_personal(ModelRc::new(VecModel::from(personal)));
    win.set_cmd_index(index as i32);
}

/// Closes the palette and forgets entries and query (also on lock).
fn close(win: &MainWindow) {
    SESSION.fetch_add(1, Ordering::SeqCst);
    STATE.with_borrow_mut(|s| *s = State::default());
    win.set_cmd_open(false);
    win.set_cmd_query(SharedString::new());
    show(win, "", None);
    win.invoke_refocus();
}

pub(super) fn wire(win: &MainWindow, vault: VaultSlot) {
    let weak = win.as_weak();
    win.on_cmd_request(move || {
        let Some(win) = weak.upgrade() else { return };
        let session = SESSION.fetch_add(1, Ordering::SeqCst) + 1;
        // Commands are available at once; the accounts follow from a worker, because
        // `list_accounts` waits on the vault connection, which a running sync holds.
        STATE.with_borrow_mut(|s| {
            *s = State {
                items: items(Vec::new()),
                ..State::default()
            }
        });
        win.set_cmd_query(SharedString::new());
        show(&win, "", None);
        win.set_cmd_open(true);
        // `try_lock` only on the slot: never blocks the UI thread.
        let handle = vault.try_lock().ok().and_then(|g| g.as_ref().cloned());
        let Some(handle) = handle else { return };
        let weak = weak.clone();
        std::thread::spawn(move || {
            let list: Vec<(i64, String, String)> = handle
                .list_accounts()
                .unwrap_or_default()
                .into_iter()
                .map(|a| (a.id, a.name, a.bank_name))
                .collect();
            let _ = weak.upgrade_in_event_loop(move |win| apply_accounts(&win, session, list));
        });
    });
    let weak = win.as_weak();
    win.on_cmd_search(move |query| {
        if let Some(win) = weak.upgrade() {
            show(&win, &query, None);
        }
    });
    let weak = win.as_weak();
    win.on_cmd_move(move |delta| {
        let Some(win) = weak.upgrade() else { return };
        let index = STATE.with_borrow_mut(|s| {
            s.index = step(s.index, delta, s.shown.len());
            s.index
        });
        win.set_cmd_index(index as i32);
    });
    let weak = win.as_weak();
    win.on_cmd_run(move |row| {
        let Some(win) = weak.upgrade() else { return };
        // -1 = the row selected with the arrow keys
        let action = STATE.with_borrow(|s| {
            let pick = usize::try_from(row).unwrap_or(s.index);
            s.shown.get(pick).map(|&i| s.items[i].action)
        });
        close(&win);
        match action {
            Some(Action::Account(id)) => win.invoke_select_account(id as i32),
            Some(action) => {
                if let Some(cmd) = command(action, &context(&win)) {
                    run_command(&win, cmd);
                }
            }
            None => {}
        }
    });
    let weak = win.as_weak();
    win.on_cmd_close(move || {
        if let Some(win) = weak.upgrade() {
            close(&win);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Context {
        Context {
            unlocked: true,
            can_transfer: true,
            ..Context::default()
        }
    }

    fn sample() -> Vec<(i64, String, String)> {
        vec![
            (1, "Girokonto".into(), "Musterbank".into()),
            (2, "Tagesgeld".into(), "Musterbank".into()),
        ]
    }

    #[test]
    fn palette_commands_map_to_their_routes() {
        let c = ctx();
        let expected = [
            Command::FocusSearch,
            Command::Refresh,
            Command::Import,
            Command::NewTransfer,
            Command::Lock,
        ];
        for (&(_, key), want) in COMMANDS.iter().zip(expected) {
            assert_eq!(command(Action::Shortcut(key), &c), Some(want));
            let routed = manage::route(&key.to_string(), true, &c);
            assert_eq!(command(Action::Shortcut(key), &c), routed);
        }
        assert_eq!(command(Action::Account(7), &c), None);
    }

    #[test]
    fn palette_keeps_the_shortcut_gates() {
        let busy = Context {
            busy: true,
            ..ctx()
        };
        assert_eq!(command(Action::Shortcut('r'), &busy), None);
        assert_eq!(command(Action::Shortcut('l'), &busy), Some(Command::Lock));
        let no_transfer = Context {
            can_transfer: false,
            ..ctx()
        };
        assert_eq!(command(Action::Shortcut('n'), &no_transfer), None);
        let locked = Context {
            unlocked: false,
            ..ctx()
        };
        assert_eq!(command(Action::Shortcut('f'), &locked), None);
    }

    #[test]
    fn ctrl_k_opens_the_palette_and_collides_with_nothing() {
        let c = ctx();
        assert_eq!(manage::route("k", true, &c), Some(Command::Palette));
        assert_eq!(manage::route("k", false, &c), None);
        for key in "abcdefghijlmnopqrstuvwxyz".chars() {
            assert_ne!(
                manage::route(&key.to_string(), true, &c),
                Some(Command::Palette)
            );
        }
        let mut keys: Vec<char> = COMMANDS.iter().map(|&(_, k)| k).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), COMMANDS.len());
        assert!(!keys.contains(&'k'));
    }

    #[test]
    fn selection_wraps_and_survives_an_empty_list() {
        assert_eq!(step(0, 1, 3), 1);
        assert_eq!(step(2, 1, 3), 0);
        assert_eq!(step(0, -1, 3), 2);
        assert_eq!(step(0, 1, 1), 0);
        assert_eq!(step(0, 1, 0), 0);
        assert_eq!(step(4, -1, 0), 0);
    }

    #[test]
    fn search_runs_on_plain_names_and_caps_the_list() {
        let all = items(sample());
        let hits = filter(&all, "  GIRO ");
        assert_eq!(hits.len(), 1);
        assert!(all[hits[0]].personal);
        assert_eq!(all[hits[0]].action, Action::Account(1));
        assert!(filter(&all, "zzz").is_empty());
        assert!(all[..COMMANDS.len()].iter().all(|it| !it.personal));
        let many = items(
            (0..30)
                .map(|i| (i, format!("Konto {i}"), "Bank".into()))
                .collect(),
        );
        assert_eq!(filter(&many, "").len(), MAX_ROWS);
    }

    #[test]
    fn palette_offers_all_commands_while_accounts_are_pending() {
        let pending = items(Vec::new());
        assert_eq!(pending.len(), COMMANDS.len());
        assert!(pending.iter().all(|it| !it.personal));
        assert_eq!(filter(&pending, "").len(), COMMANDS.len());
        // the late account list replaces the entries and keeps the commands in front
        let full = items(sample());
        assert_eq!(full.len(), COMMANDS.len() + 2);
        assert_eq!(full[COMMANDS.len()].action, Action::Account(1));
    }

    #[test]
    fn late_accounts_keep_the_selected_entry() {
        let pending = items(Vec::new());
        let shown = filter(&pending, "");
        // arrow keys moved to "Neue Überweisung" before the accounts arrived
        let keep = Some(pending[shown[3]].action);
        assert_eq!(keep, Some(Action::Shortcut('n')));
        let full = items(sample());
        let now = filter(&full, "");
        assert_eq!(
            full[now[reselect(&full, &now, keep)]].action,
            Action::Shortcut('n')
        );
        // entry no longer listed, or nothing selected: back to the first row
        let narrowed = filter(&full, "giro");
        assert_eq!(reselect(&full, &narrowed, keep), 0);
        assert_eq!(reselect(&full, &now, None), 0);
        assert_eq!(reselect(&full, &[], keep), 0);
    }
}
