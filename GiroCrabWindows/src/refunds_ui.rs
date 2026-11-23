//! UUID endpoints and token undo; docs/ERSTATTUNGEN_UI.md.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use girocrab_core::refunds::{RefundCandidate, RefundLink, RefundMatch, RefundSuggestion};
use girocrab_core::{BookingFilter, Vault};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::{MainWindow, RefundRow, daily, is_current, money, with_state};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Pair {
    refund: i64,
    refund_part: String,
    original: i64,
    original_part: String,
}

impl From<&RefundLink> for Pair {
    fn from(l: &RefundLink) -> Self {
        Self {
            refund: l.refund_id,
            refund_part: l.refund_part.clone(),
            original: l.original_id,
            original_part: l.original_part.clone(),
        }
    }
}

#[derive(Clone)]
enum Edit {
    Link(Pair),
    Unlink(Pair),
    Restore(Pair, i64),
}

struct Part {
    uuid: String,
    amount: i64,
    label: String,
}
struct Loaded {
    booking: daily::Row,
    parts: Vec<Part>,
    own: String,
    from_refund: bool,
    open: i64,
    links: Vec<RefundLink>,
    linked_bookings: HashMap<i64, daily::Row>,
    suggestions: Vec<RefundSuggestion>,
    candidates: Vec<RefundCandidate>,
}

type EditResult = Result<(bool, Option<(Pair, i64)>), String>;

#[derive(Default)]
struct Ui {
    session: usize,
    book: Option<i64>,
    part: String,
    query: String,
    dialog: u64,
    request: u64,
    operation: u64,
    pending: bool,
    loaded: Option<Loaded>,
    actions: Vec<Option<Edit>>,
    undo: Option<(Pair, i64)>,
}

impl Ui {
    fn accepts(&self, session: usize, request: u64, book: i64) -> bool {
        self.session == session && self.request == request && self.book == Some(book)
    }
    fn invalidate(&mut self) {
        self.request = self.request.wrapping_add(1);
        self.loaded = None;
        self.actions.clear();
    }
}

thread_local! { static UI: RefCell<Ui> = RefCell::new(Ui::default()); }
fn with_ui<R>(f: impl FnOnce(&mut Ui) -> R) -> R {
    UI.with(|u| f(&mut u.borrow_mut()))
}
fn text(de: &'static str, en: &'static str) -> &'static str {
    if english() { en } else { de }
}
pub fn english() -> bool {
    girocrab_core::i18n::current() == girocrab_core::i18n::Lang::En
}
pub fn can_open(amount: i64, splits: u32) -> bool {
    amount != 0 || splits > 0
}

thread_local! { static CARD_REQUEST: std::cell::Cell<u64> = const { std::cell::Cell::new(0) }; }

fn card_rows(v: &Vault, id: i64) -> Result<Vec<RefundRow>, String> {
    let links = v.refund_links(id).map_err(|e| e.to_string())?;
    if !links.iter().any(|l| !l.inactive) {
        return Ok(Vec::new());
    }
    let books = v
        .list_transactions(BookingFilter {
            limit: 0,
            ..Default::default()
        })
        .map_err(|e| e.to_string())?
        .transactions;
    Ok(links
        .iter()
        .filter(|l| !l.inactive)
        .filter_map(|l| {
            let (other_id, part) = other(l, id);
            let b = books.iter().find(|b| b.id == other_id)?;
            let label = if l.refund_id == id {
                &l.original_label
            } else {
                &l.refund_label
            };
            Some(RefundRow {
                index: other_id as i32,
                title: if b.merchant_name.is_empty() {
                    &b.counterparty
                } else {
                    &b.merchant_name
                }
                .into(),
                detail: format!(
                    "{} · {}{}",
                    b.booking_date,
                    b.account_name,
                    if part.is_empty() {
                        String::new()
                    } else {
                        format!(" · {label}")
                    }
                )
                .into(),
                amount: money(l.allocated_cents, &b.currency).into(),
                ..Default::default()
            })
        })
        .collect())
}

pub fn refresh_card(win: &MainWindow) {
    let request = CARD_REQUEST.with(|r| {
        r.set(r.get().wrapping_add(1));
        r.get()
    });
    win.set_rf_card_rows(ModelRc::new(VecModel::from(Vec::<RefundRow>::new())));
    let Some((v, id)) = with_state(|s| Some((s.vault.upgrade()?, s.selected_booking?))) else {
        return;
    };
    let weak = win.as_weak();
    std::thread::spawn(move || {
        let result = card_rows(&v, id);
        let _ = weak.upgrade_in_event_loop(move |win| {
            if !is_current(&v)
                || CARD_REQUEST.with(|r| r.get()) != request
                || with_state(|s| s.selected_booking) != Some(id)
            {
                return;
            }
            match result {
                Ok(rows) => win.set_rf_card_rows(ModelRc::new(VecModel::from(rows))),
                Err(e) => win.set_sync_status(e.into()),
            }
        });
    });
}

fn other(l: &RefundLink, id: i64) -> (i64, &str) {
    if l.refund_id == id {
        (
            l.original_id,
            l.resolved_original_part.as_deref().unwrap_or(""),
        )
    } else {
        (l.refund_id, l.resolved_refund_part.as_deref().unwrap_or(""))
    }
}

fn load(v: &Vault, id: i64, own: &str, query: &str) -> Result<Loaded, String> {
    // Presentation data only; financial totals are never derived from these rows.
    let bookings = v
        .list_transactions(BookingFilter {
            limit: 0,
            ..Default::default()
        })
        .map_err(|e| e.to_string())?
        .transactions;
    let booking = bookings.iter().find(|b| b.id == id).ok_or_else(|| {
        text(
            "Die Buchung ist nicht mehr vorhanden.",
            "This transaction no longer exists.",
        )
        .to_string()
    })?;
    let cats = v.list_categories(None, None).map_err(|e| e.to_string())?;
    let parts = v
        .list_transaction_splits(id)
        .map_err(|e| e.to_string())?
        .into_iter()
        .enumerate()
        .filter(|(_, p)| p.amount_cents != 0)
        .map(|(i, p)| Part {
            uuid: p.uuid,
            amount: p.amount_cents,
            label: format!(
                "{} {} · {} · {} · {}",
                text("Teil", "Part"),
                i + 1,
                cats.iter()
                    .find(|c| Some(c.id) == p.category_id)
                    .map_or(text("Ohne Kategorie", "Uncategorized"), |c| c.path.as_str()),
                p.note,
                money(p.amount_cents, &booking.currency)
            ),
        })
        .collect::<Vec<_>>();
    let selected = parts
        .iter()
        .find(|p| p.uuid == own)
        .or_else(|| {
            parts
                .iter()
                .find(|p| (p.amount > 0) == (booking.amount_cents > 0))
        })
        .or(parts.first());
    let amount = selected.map_or(booking.amount_cents, |p| p.amount);
    let own = selected.map_or("", |p| p.uuid.as_str()).to_string();
    let from_refund = amount > 0;
    let links = v
        .refund_links(id)
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|l| {
            !l.inactive
                && ((l.refund_id == id && l.resolved_refund_part.as_deref() == Some(&own))
                    || (l.original_id == id && l.resolved_original_part.as_deref() == Some(&own)))
        })
        .collect::<Vec<_>>();
    let booking_by_id = bookings
        .iter()
        .map(|b| (b.id, b))
        .collect::<HashMap<_, _>>();
    let linked_bookings = links
        .iter()
        .filter_map(|l| {
            let other_id = other(l, id).0;
            booking_by_id
                .get(&other_id)
                .map(|b| (other_id, daily::Row::from(*b)))
        })
        .collect();
    let used = links
        .iter()
        .map(|l| i128::from(l.allocated_cents))
        .sum::<i128>();
    let open = i64::try_from((i128::from(amount).abs() - used).max(0))
        .map_err(|_| text("Betrag zu groß.", "Amount too large.").to_string())?;
    let shown = |c: &RefundCandidate| {
        links
            .iter()
            .any(|l| other(l, id) == (c.transaction_id, c.part.as_str()))
    };
    let suggestions = if amount == 0 {
        Vec::new()
    } else {
        v.refund_suggestions(id, own.clone())
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|s| !shown(&s.candidate))
            .take(3)
            .collect::<Vec<_>>()
    };
    let candidates = if amount == 0 {
        Vec::new()
    } else {
        v.refund_candidates(id, own.clone(), query.to_string(), 200)
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|c| {
                !shown(c)
                    && !suggestions.iter().any(|s| {
                        s.candidate.transaction_id == c.transaction_id && s.candidate.part == c.part
                    })
            })
            .collect()
    };
    Ok(Loaded {
        booking: daily::Row::from(booking),
        parts,
        own,
        from_refund,
        open,
        links,
        linked_bookings,
        suggestions,
        candidates,
    })
}

fn clear(win: &MainWindow) {
    win.set_rf_title(SharedString::new());
    win.set_rf_amount(SharedString::new());
    win.set_rf_sums(SharedString::new());
    win.set_rf_rows(ModelRc::new(VecModel::from(Vec::<RefundRow>::new())));
    win.set_rf_parts(ModelRc::new(VecModel::from(Vec::<SharedString>::new())));
}

pub fn reset(win: &MainWindow) {
    CARD_REQUEST.with(|r| r.set(r.get().wrapping_add(1)));
    win.set_rf_card_rows(ModelRc::new(VecModel::from(Vec::<RefundRow>::new())));
    with_ui(|u| *u = Ui::default());
    win.set_refunds_open(false);
    win.set_rf_edit_pending(false);
    win.set_rf_loading(false);
    win.set_rf_can_undo(false);
    clear(win);
}

pub fn privacy_changed(win: &MainWindow) {
    with_ui(|u| u.invalidate());
    clear(win);
}

fn render(win: &MainWindow) {
    let privacy = crate::privacy_on(win);
    let mask = |s: &str| daily::mask(s, privacy);
    with_ui(|u| {
        let Some(d) = &u.loaded else { return };
        win.set_rf_english(english());
        win.set_rf_from_refund(d.from_refund);
        win.set_rf_title(
            mask(&format!(
                "{} · {}",
                d.booking.counterparty, d.booking.purpose
            ))
            .into(),
        );
        win.set_rf_amount(mask(&money(d.booking.amount_cents, &d.booking.currency)).into());
        win.set_rf_sums(
            format!(
                "{}: {}",
                text("Noch offen", "Still open"),
                mask(&money(d.open, &d.booking.currency))
            )
            .into(),
        );
        win.set_rf_parts(ModelRc::new(VecModel::from(
            d.parts
                .iter()
                .map(|p| mask(&p.label).into())
                .collect::<Vec<SharedString>>(),
        )));
        win.set_rf_part_index(
            d.parts
                .iter()
                .position(|p| p.uuid == d.own)
                .map_or(0, |i| i as i32),
        );
        let mut rows = Vec::new();
        let mut actions = Vec::new();
        let mut push = |mut row: RefundRow, action: Option<Edit>| {
            row.index = rows.len() as i32;
            rows.push(row);
            actions.push(action);
        };
        if !d.links.is_empty() {
            push(
                RefundRow {
                    title: text("Verknüpfte Buchungen", "Linked transactions").into(),
                    ..Default::default()
                },
                None,
            );
            for l in &d.links {
                let label = if l.refund_id == d.booking.id {
                    &l.original_label
                } else {
                    &l.refund_label
                };
                let linked = d.linked_bookings.get(&other(l, d.booking.id).0);
                push(
                    RefundRow {
                        kind: 1,
                        title: mask(linked.map_or(label, |b| b.counterparty.as_str())).into(),
                        detail: mask(&linked.map_or_else(
                            || label.clone(),
                            |b| format!("{} · {} · {}", b.booking_date, label, b.purpose),
                        ))
                        .into(),
                        amount: mask(&money(l.allocated_cents, &d.booking.currency)).into(),
                        ..Default::default()
                    },
                    Some(Edit::Unlink(l.into())),
                );
            }
        }
        let mut candidate = |c: &RefundCandidate, reason: &str| {
            let pair = if d.from_refund {
                Pair {
                    refund: d.booking.id,
                    refund_part: d.own.clone(),
                    original: c.transaction_id,
                    original_part: c.part.clone(),
                }
            } else {
                Pair {
                    refund: c.transaction_id,
                    refund_part: c.part.clone(),
                    original: d.booking.id,
                    original_part: d.own.clone(),
                }
            };
            push(
                RefundRow {
                    kind: 2,
                    title: mask(if c.merchant_name.is_empty() {
                        &c.counterparty
                    } else {
                        &c.merchant_name
                    })
                    .into(),
                    detail: mask(&format!("{} · {}", c.booking_date, c.part_label)).into(),
                    reason: reason.into(),
                    amount: mask(&money(c.amount_cents, &c.currency)).into(),
                    share: format!(
                        "{}: {}",
                        text("Offen", "Open"),
                        mask(&money(c.open_cents, &c.currency))
                    )
                    .into(),
                    blocked: d.open <= 0 || c.open_cents <= 0,
                    ..Default::default()
                },
                Some(Edit::Link(pair)),
            );
        };
        if !d.suggestions.is_empty() {
            candidate(
                &d.suggestions[0].candidate,
                match d.suggestions[0].reason {
                    RefundMatch::SameAmount => text("Gleicher Betrag", "Same amount"),
                    RefundMatch::SameMerchant => text("Gleicher Händler", "Same merchant"),
                    RefundMatch::SameReference => text("Gleiche Referenz", "Same reference"),
                },
            );
            for s in d.suggestions.iter().skip(1) {
                candidate(
                    &s.candidate,
                    match s.reason {
                        RefundMatch::SameAmount => text("Gleicher Betrag", "Same amount"),
                        RefundMatch::SameMerchant => text("Gleicher Händler", "Same merchant"),
                        RefundMatch::SameReference => text("Gleiche Referenz", "Same reference"),
                    },
                );
            }
        }
        for c in &d.candidates {
            candidate(c, "");
        }
        let linked = d.links.len() + usize::from(!d.links.is_empty());
        if !d.suggestions.is_empty() {
            rows.insert(
                linked,
                RefundRow {
                    title: text("Vorschläge", "Suggestions").into(),
                    ..Default::default()
                },
            );
            actions.insert(linked, None);
        }
        if !d.candidates.is_empty() {
            let index = linked + d.suggestions.len() + usize::from(!d.suggestions.is_empty());
            rows.insert(
                index,
                RefundRow {
                    title: text("Weitere Buchungen", "Other transactions").into(),
                    ..Default::default()
                },
            );
            actions.insert(index, None);
        }
        for (i, row) in rows.iter_mut().enumerate() {
            row.index = i as i32;
        }
        u.actions = actions;
        win.set_rf_rows(ModelRc::new(VecModel::from(rows)));
        win.set_rf_can_undo(u.undo.is_some());
    });
}

pub fn refresh(win: &MainWindow, v: Arc<Vault>) {
    if !is_current(&v) || !win.get_refunds_open() {
        return;
    }
    let Some((book, own, query, request, session)) = with_ui(|u| {
        let book = u.book?;
        u.invalidate();
        Some((book, u.part.clone(), u.query.clone(), u.request, u.session))
    }) else {
        return;
    };
    clear(win);
    win.set_rf_loading(true);
    win.set_rf_status(SharedString::new());
    let weak = win.as_weak();
    std::thread::spawn(move || {
        let result = load(&v, book, &own, &query);
        let _ = weak.upgrade_in_event_loop(move |win| {
            if !is_current(&v)
                || !win.get_refunds_open()
                || !with_ui(|u| u.accepts(session, request, book))
            {
                return;
            }
            win.set_rf_loading(false);
            match result {
                Ok(data) => {
                    with_ui(|u| {
                        u.part = data.own.clone();
                        u.loaded = Some(data);
                    });
                    render(&win);
                }
                Err(e) => {
                    clear(&win);
                    win.set_rf_status(e.into());
                }
            }
        });
    });
}

fn open(win: &MainWindow, v: Arc<Vault>, book: i64) {
    with_ui(|u| {
        u.session = crate::session_id(&v);
        u.dialog = u.dialog.wrapping_add(1);
        u.book = Some(book);
        u.part.clear();
        u.query.clear();
        u.undo = None;
    });
    win.set_rf_english(english());
    win.set_refunds_open(true);
    win.set_rf_can_undo(false);
    refresh(win, v);
}

fn edit(v: &Vault, action: &Edit) -> EditResult {
    let pair = match action {
        Edit::Link(p) | Edit::Unlink(p) | Edit::Restore(p, _) => p,
    };
    let Pair {
        refund,
        refund_part,
        original,
        original_part,
    } = pair;
    let args = (
        *refund,
        refund_part.clone(),
        *original,
        original_part.clone(),
    );
    match action {
        Edit::Link(_) => {
            if v.refund_links(*refund)
                .map_err(|e| e.to_string())?
                .iter()
                .any(|l| {
                    !l.inactive
                        && l.original_id == *original
                        && l.resolved_refund_part.as_deref() == Some(refund_part)
                        && l.resolved_original_part.as_deref() == Some(original_part)
                })
            {
                return Ok((false, None));
            }
            v.link_refund(args.0, args.1, args.2, args.3)
                .map_err(|e| e.to_string())?;
            Ok((true, None))
        }
        Edit::Unlink(_) => {
            if !v
                .refund_links(*refund)
                .map_err(|e| e.to_string())?
                .iter()
                .any(|l| Pair::from(l) == *pair)
            {
                return Ok((false, None));
            }
            v.unlink_refund(args.0, args.1, args.2, args.3)
                .map(|token| (true, Some((pair.clone(), token))))
                .map_err(|e| e.to_string())
        }
        Edit::Restore(_, token) => v
            .restore_refund(args.0, args.1, args.2, args.3, *token)
            .map(|_| (true, None))
            .map_err(|e| e.to_string()),
    }
}

fn mutate(win: &MainWindow, v: Arc<Vault>, action: Edit) {
    let Some((operation, dialog, session)) = with_ui(|u| {
        if u.pending {
            return None;
        }
        u.pending = true;
        u.operation = u.operation.wrapping_add(1);
        u.invalidate();
        Some((u.operation, u.dialog, u.session))
    }) else {
        return;
    };
    win.set_rf_edit_pending(true);
    clear(win);
    win.set_rf_loading(true);
    win.set_rf_status(SharedString::new());
    let weak = win.as_weak();
    std::thread::spawn(move || {
        let result = edit(&v, &action);
        let _ = weak.upgrade_in_event_loop(move |win| {
            if !is_current(&v) || !with_ui(|u| u.session == session && u.operation == operation) {
                return;
            }
            with_ui(|u| u.pending = false);
            win.set_rf_edit_pending(false);
            match result {
                Ok((changed, undo)) => {
                    if changed {
                        with_ui(|u| {
                            if u.dialog == dialog {
                                u.undo = undo;
                            }
                        });
                        crate::reload(&win, v.clone());
                    }
                    refresh(&win, v);
                }
                Err(e) => {
                    if !with_ui(|u| u.dialog == dialog) {
                        refresh(&win, v);
                        return;
                    }
                    with_ui(|u| u.invalidate());
                    win.set_rf_loading(false);
                    clear(&win);
                    win.set_rf_status(e.into());
                }
            }
        });
    });
}

pub fn wire(win: &MainWindow, slot: Arc<Mutex<Option<Arc<Vault>>>>) {
    let weak = win.as_weak();
    let vault = slot.clone();
    win.on_refund_reveal(move |id| {
        let (Some(win), Some(v)) = (weak.upgrade(), vault.lock().unwrap().clone()) else {
            return;
        };
        with_state(|s| {
            s.filter = daily::Filter::default();
            s.selected_booking = Some(i64::from(id));
        });
        win.set_search_text(SharedString::new());
        win.set_uncategorized_only(false);
        win.set_status_index(0);
        win.set_range_index(0);
        win.set_tag_index(0);
        win.set_documents_only(false);
        crate::views_ui::select_target(&win, &v, crate::nav::Target::AllAccounts);
    });
    let (weak, s) = (win.as_weak(), slot.clone());
    win.on_refunds_request(move || {
        let (Some(win), Some(v)) = (weak.upgrade(), s.lock().unwrap().clone()) else {
            return;
        };
        let id = with_state(|st| {
            st.rows
                .iter()
                .find(|r| {
                    Some(r.id) == st.selected_booking && can_open(r.amount_cents, r.split_count)
                })
                .map(|r| r.id)
        });
        if let Some(id) = id {
            open(&win, v, id);
        }
    });
    let weak = win.as_weak();
    win.on_refund_close(move || {
        if let Some(win) = weak.upgrade() {
            with_ui(|u| {
                u.book = None;
                u.dialog = u.dialog.wrapping_add(1);
                u.invalidate();
                u.undo = None;
            });
            win.set_refunds_open(false);
            clear(&win);
            crate::manage_ui::refocus_later(&win);
        }
    });
    let (weak, s) = (win.as_weak(), slot.clone());
    win.on_refund_part_picked(move |index| {
        let (Some(win), Some(v)) = (weak.upgrade(), s.lock().unwrap().clone()) else {
            return;
        };
        if win.get_rf_loading() || win.get_rf_edit_pending() {
            return;
        }
        let part = with_ui(|u| {
            u.loaded
                .as_ref()?
                .parts
                .get(usize::try_from(index).ok()?)
                .map(|p| p.uuid.clone())
        });
        if let Some(part) = part {
            with_ui(|u| u.part = part);
            refresh(&win, v);
        }
    });
    let (weak, s) = (win.as_weak(), slot.clone());
    win.on_refund_search(move |query| {
        let (Some(win), Some(v)) = (weak.upgrade(), s.lock().unwrap().clone()) else {
            return;
        };
        let request = with_ui(|u| {
            u.query = query.to_string();
            u.invalidate();
            u.request
        });
        clear(&win);
        win.set_rf_loading(true);
        let weak = win.as_weak();
        slint::Timer::single_shot(Duration::from_millis(200), move || {
            if let Some(win) = weak.upgrade()
                && with_ui(|u| u.request == request)
            {
                refresh(&win, v);
            }
        });
    });
    let (weak, s) = (win.as_weak(), slot.clone());
    win.on_refund_toggle(move |index| {
        let (Some(win), Some(v)) = (weak.upgrade(), s.lock().unwrap().clone()) else {
            return;
        };
        if win.get_rf_loading() || win.get_rf_edit_pending() {
            return;
        }
        let action = with_ui(|u| {
            u.actions
                .get(usize::try_from(index).ok()?)
                .cloned()
                .flatten()
        });
        if let Some(action) = action {
            mutate(&win, v, action);
        }
    });
    let weak = win.as_weak();
    win.on_refund_undo_unlink(move || {
        let (Some(win), Some(v)) = (weak.upgrade(), slot.lock().unwrap().clone()) else {
            return;
        };
        if let Some((pair, token)) = with_ui(|u| u.undo.clone()) {
            mutate(&win, v, Edit::Restore(pair, token));
        }
    });
}

struct Demo {
    account: i64,
    original: i64,
    refund: i64,
    part: String,
}

fn seed(v: &Vault) -> Result<Demo, String> {
    use girocrab_core::offline::ManualBookingSetup;
    use girocrab_core::{AccountKind, TransactionSplitInput};
    let run = || -> Result<Demo, girocrab_core::CoreError> {
        let account =
            v.create_offline_account("Erstattungen".into(), AccountKind::Cash, "EUR".into(), 0)?;
        let category =
            v.create_category("Bestellungen".into(), None, "cart".into(), "#2F6FDB".into())?;
        let booking = |amount, name: &str| {
            v.save_manual_booking(
                account,
                None,
                ManualBookingSetup {
                    booking_date: "2026-01-14".into(),
                    amount_cents: amount,
                    counterparty: name.into(),
                    purpose: "Bestellung RE714001".into(),
                    category_id: if amount < 0 { Some(category) } else { None },
                    note: String::new(),
                },
            )
        };
        let original = booking(-10000, "Musterladen")?;
        for (amount, name) in [
            (-4000, "Musterladen"),
            (-3000, "Versandhaus"),
            (-7000, "Buchhandlung"),
            (-8000, "Kaufhaus"),
        ] {
            booking(amount, name)?;
        }
        let refund = booking(6000, "Musterladen")?;
        v.set_transaction_splits(
            refund,
            vec![4000, 2000]
                .into_iter()
                .map(|amount| TransactionSplitInput {
                    uuid: None,
                    category_id: None,
                    amount_cents: amount,
                    note: "Rückzahlung".into(),
                })
                .collect(),
        )?;
        let part = v.list_transaction_splits(refund)?[0].uuid.clone();
        Ok(Demo {
            account,
            original,
            refund,
            part,
        })
    };
    run().map_err(|e| e.to_string())
}

pub fn demo(win: &MainWindow, v: Arc<Vault>) {
    let Ok(mode) = std::env::var("GIROCRAB_REFUND_DEMO") else {
        return;
    };
    if !["split", "reverse", "linked"].contains(&mode.as_str()) {
        return;
    }
    let weak = win.as_weak();
    std::thread::spawn(move || {
        let result = seed(&v).and_then(|f| {
            if mode == "linked" {
                v.link_refund(f.refund, f.part.clone(), f.original, String::new())
                    .map_err(|e| e.to_string())?;
            }
            Ok(f)
        });
        let _ = weak.upgrade_in_event_loop(move |win| {
            if !is_current(&v) || !win.get_demo_active() {
                return;
            }
            match result {
                Ok(f) => {
                    crate::views_ui::refresh_sidebar(&win, &v);
                    crate::views_ui::select_target(
                        &win,
                        &v,
                        crate::nav::Target::Account(f.account),
                    );
                    if mode != "linked" {
                        open(
                            &win,
                            v,
                            if mode == "reverse" {
                                f.original
                            } else {
                                f.refund
                            },
                        );
                    }
                }
                Err(e) => win.set_sync_status(e.into()),
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn fixture() -> (Temp, Arc<Vault>, Demo) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = Temp(std::env::temp_dir().join(format!(
            "girocrab-win-refund-{}-{time}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )));
        std::fs::create_dir(&dir.0).unwrap();
        let v = Vault::open(dir.0.join("v.db").to_string_lossy().into(), vec![7; 32]).unwrap();
        let f = seed(&v).unwrap();
        (dir, v, f)
    }

    #[test]
    fn request_tokens_reject_old_reads_and_sessions() {
        let mut u = Ui {
            session: 1,
            book: Some(5),
            ..Default::default()
        };
        assert!(u.accepts(1, 0, 5));
        u.invalidate();
        assert!(!u.accepts(1, 0, 5));
        assert!(u.accepts(1, 1, 5));
        assert!(!u.accepts(2, 1, 5));
        assert!(!u.accepts(1, 1, 6));
        u.book = None;
        assert!(!u.accepts(1, 1, 5));
        assert!(can_open(0, 2));
        assert!(!can_open(0, 0));
        assert!(can_open(-1, 0));
    }

    #[test]
    fn inspector_links_include_every_part_and_both_directions() {
        let (_dir, v, f) = fixture();
        assert!(card_rows(&v, f.refund).unwrap().is_empty());
        v.link_refund(f.refund, f.part.clone(), f.original, String::new())
            .unwrap();
        let rows = card_rows(&v, f.refund).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].index, f.original as i32);
        assert!(rows[0].detail.contains("Erstattungen"));
        assert!(!rows[0].amount.is_empty());
        let reverse = card_rows(&v, f.original).unwrap();
        assert_eq!(reverse[0].index, f.refund as i32);
        assert!(reverse[0].detail.contains("Rückzahlung"));
        let token = v
            .unlink_refund(f.refund, f.part.clone(), f.original, String::new())
            .unwrap();
        assert!(card_rows(&v, f.original).unwrap().is_empty());
        v.restore_refund(f.refund, f.part, f.original, String::new(), token)
            .unwrap();
        assert_eq!(card_rows(&v, f.original).unwrap().len(), 1);
    }

    #[test]
    fn a_linked_refund_counts_as_income_only_for_its_unassigned_rest() {
        let (_dir, v, _) = fixture();
        let account = v
            .create_offline_account(
                "Analytics".into(),
                girocrab_core::AccountKind::Cash,
                "EUR".into(),
                0,
            )
            .unwrap();
        let categories = ["A", "B", "C"].map(|name| {
            v.create_category(name.into(), None, "tag".into(), "#123456".into())
                .unwrap()
        });
        let add = |amount, category, date: &str| {
            v.save_manual_booking(
                account,
                None,
                girocrab_core::offline::ManualBookingSetup {
                    booking_date: date.into(),
                    amount_cents: amount,
                    counterparty: "QA vendor".into(),
                    purpose: String::new(),
                    category_id: Some(category),
                    note: String::new(),
                },
            )
            .unwrap()
        };
        let first = add(-10000, categories[0], "2025-01-01");
        let second = add(-3000, categories[1], "2025-01-01");
        let full = add(10000, categories[0], "2025-02-01");
        let partial = add(5000, categories[1], "2025-02-01");
        add(10200, categories[2], "2025-02-01");
        v.link_refund(full, String::new(), first, String::new())
            .unwrap();
        v.link_refund(partial, String::new(), second, String::new())
            .unwrap();
        let report = || {
            crate::analytics::load(
                &v,
                BookingFilter {
                    account_ids: vec![account],
                    date_from: Some("2025-02-01".into()),
                    date_to: Some("2025-02-28".into()),
                    ..Default::default()
                },
                &daily::Filter::default(),
            )
            .unwrap()
        };
        let r = report();
        assert_eq!(
            (r.income_cents, r.expense_cents, r.net_cents),
            (12200, -13000, 25200)
        );
        let slices = crate::analytics::categories(&r.category_income, 5);
        assert_eq!(
            slices
                .iter()
                .map(|s| (s.name.as_str(), s.cents))
                .collect::<Vec<_>>(),
            [("C", 10200), ("B", 2000)]
        );
        add(-20000, categories[0], "2025-02-01");
        let r = report();
        assert_eq!(
            (r.income_cents, r.expense_cents, r.net_cents),
            (12200, 7000, 5200)
        );
    }

    #[test]
    fn candidates_keep_distinct_income_parts_and_linked_rows_keep_the_exact_part() {
        let (_dir, v, _) = fixture();
        let account = v
            .create_offline_account(
                "Parts".into(),
                girocrab_core::AccountKind::Cash,
                "EUR".into(),
                0,
            )
            .unwrap();
        let add = |amount, name: &str| {
            v.save_manual_booking(
                account,
                None,
                girocrab_core::offline::ManualBookingSetup {
                    booking_date: "2025-02-10".into(),
                    amount_cents: amount,
                    counterparty: name.into(),
                    purpose: String::new(),
                    category_id: None,
                    note: String::new(),
                },
            )
            .unwrap()
        };
        let original = add(-300, "Purchase");
        let refund = add(200, "Distinct income");
        v.set_transaction_splits(
            refund,
            ["First", "Second"]
                .map(|note| girocrab_core::TransactionSplitInput {
                    uuid: None,
                    category_id: None,
                    amount_cents: 100,
                    note: note.into(),
                })
                .to_vec(),
        )
        .unwrap();
        let parts = v.list_transaction_splits(refund).unwrap();
        v.link_refund(refund, parts[0].uuid.clone(), original, String::new())
            .unwrap();
        let x = load(&v, original, "", "Distinct income").unwrap();
        assert_eq!(x.links.len(), 1);
        assert_eq!(x.links[0].refund_part, parts[0].uuid);
        assert_eq!(x.candidates.len(), 1);
        assert_eq!(x.candidates[0].transaction_id, refund);
        assert_eq!(x.candidates[0].part, parts[1].uuid);
    }

    #[test]
    fn uuid_selection_suggestions_search_and_errors() {
        let (_dir, v, f) = fixture();
        let d = load(&v, f.refund, "", "").unwrap();
        assert!(d.from_refund);
        assert_eq!(d.open, 4000);
        assert_eq!(d.own, f.part);
        assert_eq!(d.suggestions.len(), 3);
        let other = load(&v, f.refund, &d.parts[1].uuid, "").unwrap();
        assert_eq!(other.open, 2000);
        let empty = load(&v, f.refund, "", "nothing-matches-xyz").unwrap();
        assert!(empty.candidates.is_empty());
        assert!(!load(&v, f.original, "", "").unwrap().from_refund);
        assert!(load(&v, -1, "", "").is_err());
        v.set_transaction_splits(
            f.refund,
            vec![-1000, 7000]
                .into_iter()
                .map(|amount| girocrab_core::TransactionSplitInput {
                    uuid: None,
                    category_id: None,
                    amount_cents: amount,
                    note: String::new(),
                })
                .collect(),
        )
        .unwrap();
        let parts = v.list_transaction_splits(f.refund).unwrap();
        let negative = parts.iter().find(|p| p.amount_cents < 0).unwrap();
        let mixed = load(&v, f.refund, &negative.uuid, "").unwrap();
        assert!(!mixed.from_refund);
        assert_eq!(mixed.own, negative.uuid);
    }

    #[test]
    fn unlink_restore_keeps_priority_and_rejects_stale_token() {
        let (_dir, v, f) = fixture();
        let first = v
            .refund_candidates(f.refund, f.part.clone(), "Versandhaus".into(), 20)
            .unwrap()[0]
            .transaction_id;
        let p = Pair {
            refund: f.refund,
            refund_part: f.part.clone(),
            original: first,
            original_part: String::new(),
        };
        assert!(edit(&v, &Edit::Link(p.clone())).unwrap().0);
        assert!(!edit(&v, &Edit::Link(p.clone())).unwrap().0);
        let mut p2 = p.clone();
        p2.original = f.original;
        edit(&v, &Edit::Link(p2)).unwrap();
        let (_, Some((p, token))) = edit(&v, &Edit::Unlink(p)).unwrap() else {
            panic!("token")
        };
        assert_eq!(
            load(&v, f.refund, &f.part, "").unwrap().links[0].allocated_cents,
            4000
        );
        edit(&v, &Edit::Restore(p.clone(), token)).unwrap();
        let d = load(&v, f.refund, &f.part, "").unwrap();
        assert_eq!(d.links[0].original_id, first);
        assert_eq!(d.linked_bookings[&first].counterparty, "Versandhaus");
        assert_eq!(d.linked_bookings[&first].booking_date, "2026-01-14");
        assert_eq!(d.links[0].allocated_cents, 3000);
        assert_eq!(d.links[1].allocated_cents, 1000);
        let other = load(&v, f.refund, &d.parts[1].uuid, "").unwrap();
        assert!(
            other
                .candidates
                .iter()
                .chain(other.suggestions.iter().map(|s| &s.candidate))
                .any(|c| c.transaction_id == f.original)
        );
        assert!(d.candidates.is_empty());
        edit(&v, &Edit::Unlink(p.clone())).unwrap();
        assert!(!edit(&v, &Edit::Unlink(p.clone())).unwrap().0);
        assert!(edit(&v, &Edit::Restore(p, token)).is_err());
    }

    #[test]
    fn reloading_reads_current_sign_and_omits_dropped_links() {
        let (_dir, v, f) = fixture();
        v.link_refund(f.refund, f.part, f.original, String::new())
            .unwrap();
        v.save_manual_booking(
            f.account,
            Some(f.original),
            girocrab_core::offline::ManualBookingSetup {
                booking_date: "2026-01-14".into(),
                amount_cents: 10000,
                counterparty: "Musterladen".into(),
                purpose: String::new(),
                category_id: None,
                note: String::new(),
            },
        )
        .unwrap();
        let d = load(&v, f.original, "", "").unwrap();
        assert!(d.from_refund);
        assert!(d.links.is_empty());
        assert!(d.linked_bookings.is_empty());
        assert_eq!(d.booking.amount_cents, 10000);
        let bad = Pair {
            refund: -1,
            refund_part: String::new(),
            original: f.original,
            original_part: String::new(),
        };
        assert!(edit(&v, &Edit::Link(bad)).is_err());
    }
}
