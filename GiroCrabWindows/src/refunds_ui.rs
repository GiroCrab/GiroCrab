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
    /// The pair and the `changed_ms` of the link shown (compare-and-set unlink).
    Unlink(Pair, Option<i64>),
    Restore(Pair, String),
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
    search_matches: bool,
}

type EditResult = Result<(bool, Option<(Pair, String)>), String>;

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
    undo: Option<(Pair, String)>,
    edit_error: String,
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
    fn edit_failed(&mut self, error: String, restoring: bool) {
        self.invalidate();
        // A restore the core rejects as not possible has a stale token (a newer change won):
        // offering the same undo again cannot succeed. Other errors (e.g. a locked vault) keep it.
        if restoring
            && (error.contains("Diese Erstattungszuordnung ist nicht möglich")
                || error.contains("This refund assignment is not possible"))
        {
            self.undo = None;
        }
        self.edit_error = error;
    }
    fn edit_succeeded(&mut self, action: &Edit, undo: Option<(Pair, String)>) {
        if !matches!(action, Edit::Link(_)) {
            self.undo = undo;
        }
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
thread_local! { static REVEAL_REQUEST: std::cell::Cell<u64> = const { std::cell::Cell::new(0) }; }

pub fn cancel_reveal() {
    REVEAL_REQUEST.with(|r| r.set(r.get().wrapping_add(1)));
}

fn card_filters(v: &Vault, id: i64, links: &[RefundLink]) -> Result<Vec<BookingFilter>, String> {
    let ids: std::collections::BTreeSet<_> = links
        .iter()
        .filter(|l| !l.inactive)
        .map(|l| other(l, id).0)
        .collect();
    let dates = ids
        .into_iter()
        .map(|id| v.booking_date(id))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?
        .into_iter()
        .flatten()
        .collect::<std::collections::BTreeSet<_>>();
    Ok(dates
        .into_iter()
        .map(|date| BookingFilter {
            date_from: Some(date.clone()),
            date_to: Some(date),
            limit: 0,
            ..Default::default()
        })
        .collect())
}

fn card_rows(v: &Vault, id: i64) -> Result<Vec<RefundRow>, String> {
    let links = v.refund_links(id).map_err(|e| e.to_string())?;
    let filters = card_filters(v, id, &links)?;
    if filters.is_empty() {
        return Ok(Vec::new());
    }
    let ids: std::collections::BTreeSet<_> = links
        .iter()
        .filter(|l| !l.inactive)
        .map(|l| other(l, id).0)
        .collect();
    let mut books = HashMap::new();
    for filter in filters {
        for b in v
            .list_transactions(filter)
            .map_err(|e| e.to_string())?
            .transactions
        {
            if ids.contains(&b.id) {
                books.insert(b.id, b);
            }
        }
    }
    Ok(links
        .iter()
        .filter(|l| !l.inactive)
        .filter_map(|l| {
            let (other_id, part) = other(l, id);
            let b = books.get(&other_id)?;
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

fn rendered_card_rows(mut rows: Vec<RefundRow>, privacy: bool) -> Vec<RefundRow> {
    for row in &mut rows {
        row.title = daily::mask(&row.title, privacy).into();
        row.detail = daily::mask(&row.detail, privacy).into();
        row.amount = daily::mask(&row.amount, privacy).into();
    }
    rows
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
                Ok(rows) => win.set_rf_card_rows(ModelRc::new(VecModel::from(rendered_card_rows(
                    rows,
                    crate::privacy_on(&win),
                )))),
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
    let links_all = v.refund_links(id).map_err(|e| e.to_string())?;
    let ids: std::collections::HashSet<_> = std::iter::once(id)
        .chain(
            links_all
                .iter()
                .filter(|l| !l.inactive)
                .flat_map(|l| [l.refund_id, l.original_id]),
        )
        .collect();
    let dates = ids
        .iter()
        .map(|id| v.booking_date(*id))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?
        .into_iter()
        .flatten()
        .collect::<std::collections::BTreeSet<_>>();
    let mut bookings = Vec::new();
    for date in dates {
        bookings.extend(
            v.list_transactions(BookingFilter {
                date_from: Some(date.clone()),
                date_to: Some(date),
                limit: 0,
                ..Default::default()
            })
            .map_err(|e| e.to_string())?
            .transactions
            .into_iter()
            .filter(|b| ids.contains(&b.id)),
        );
    }
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
    let selected = parts.iter().find(|p| p.uuid == own).or_else(|| {
        if own.is_empty() {
            parts
                .iter()
                .find(|p| (p.amount > 0) == (booking.amount_cents > 0))
                .or(parts.first())
        } else {
            None
        }
    });
    let invalid = !own.is_empty() && selected.is_none();
    let amount = if invalid {
        0
    } else {
        selected.map_or(booking.amount_cents, |p| p.amount)
    };
    let own = if invalid {
        own
    } else {
        selected.map_or("", |p| p.uuid.as_str())
    }
    .to_string();
    let from_refund = amount > 0;
    let links = links_all
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
            .filter(|c| !shown(c))
            .collect::<Vec<_>>()
    };
    let search_matches = !candidates.is_empty();
    let candidates = candidates
        .into_iter()
        .filter(|c| {
            !suggestions.iter().any(|s| {
                s.candidate.transaction_id == c.transaction_id && s.candidate.part == c.part
            })
        })
        .collect();
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
        search_matches,
    })
}

fn clear(win: &MainWindow) {
    win.set_rf_search_empty(false);
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

pub fn privacy_changed(win: &MainWindow, vault: Arc<Vault>) {
    with_ui(|u| u.invalidate());
    win.set_rf_loading(false);
    clear(win);
    refresh_card(win);
    if win.get_refunds_open() {
        refresh(win, vault);
    }
}

fn search_empty(d: &Loaded, query: &str) -> bool {
    !query.trim().is_empty() && !d.search_matches
}

fn render(win: &MainWindow) {
    let privacy = crate::privacy_on(win);
    let mask = |s: &str| daily::mask(s, privacy);
    with_ui(|u| {
        let Some(d) = &u.loaded else { return };
        if !d.own.is_empty() && !d.parts.iter().any(|p| p.uuid == d.own) {
            win.set_rf_status(
                text(
                    if d.parts.is_empty() { "Die Aufteilung existiert nicht mehr. Schließen Sie die Zuordnung und öffnen Sie sie erneut, um die ganze Buchung zu wählen." } else { "Die gewählte Teilbuchung existiert nicht mehr. Bitte wählen Sie einen Teil." },
                    if d.parts.is_empty() { "The split no longer exists. Close and reopen the picker to select the whole booking." } else { "The selected split no longer exists. Please select a part." },
                )
                .into(),
            );
        }
        win.set_rf_search_empty(search_empty(d, &u.query));
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
                .map_or(-1, |i| i as i32),
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
                    Some(Edit::Unlink(l.into(), Some(l.changed_ms))),
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
    win.set_rf_status(with_ui(|u| u.edit_error.clone()).into());
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
        u.edit_error.clear();
    });
    win.set_rf_english(english());
    win.set_refunds_open(true);
    win.set_rf_can_undo(false);
    refresh(win, v);
}

fn edit(v: &Vault, action: &Edit) -> EditResult {
    let pair = match action {
        Edit::Link(p) | Edit::Unlink(p, _) | Edit::Restore(p, _) => p,
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
        // the core reports whether this call created the link (no undo for an existing one)
        Edit::Link(_) => v
            .link_refund(args.0, args.1, args.2, args.3)
            .map(|created| (created, None))
            .map_err(|e| e.to_string()),
        Edit::Unlink(_, changed_ms) => {
            if !v
                .refund_links(*refund)
                .map_err(|e| e.to_string())?
                .iter()
                .any(|l| Pair::from(l) == *pair)
            {
                return Ok((false, None));
            }
            v.unlink_refund(args.0, args.1, args.2, args.3, *changed_ms)
                .map(|token| (true, Some((pair.clone(), token))))
                .map_err(|e| e.to_string())
        }
        Edit::Restore(_, token) => v
            .restore_refund(args.0, args.1, args.2, args.3, token.clone())
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
        u.edit_error.clear();
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
                                u.edit_succeeded(&action, undo);
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
                    with_ui(|u| u.edit_failed(e, matches!(action, Edit::Restore(..))));
                    refresh(&win, v);
                }
            }
        });
    });
}

fn reveal_count(v: &Vault, id: i64) -> Result<Option<u32>, String> {
    let Some(day) = v.booking_date(id).map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    v.count_transactions(BookingFilter {
        date_from: Some(day),
        ..Default::default()
    })
    .map(Some)
    .map_err(|e| e.to_string())
}

pub fn wire(win: &MainWindow, slot: Arc<Mutex<Option<Arc<Vault>>>>) {
    let weak = win.as_weak();
    let vault = slot.clone();
    win.on_refund_reveal(move |id| {
        let (Some(win), Some(v)) = (weak.upgrade(), vault.lock().unwrap().clone()) else {
            return;
        };
        let request = REVEAL_REQUEST.with(|r| {
            r.set(r.get().wrapping_add(1));
            r.get()
        });
        let weak = win.as_weak();
        std::thread::spawn(move || {
            let result = reveal_count(&v, i64::from(id));
            let _ = weak.upgrade_in_event_loop(move |win| {
                if !is_current(&v) || REVEAL_REQUEST.with(|r| r.get()) != request {
                    return;
                }
                match result {
                    Ok(Some(count)) => {
                        with_state(|s| {
                            s.filter = daily::Filter::default();
                            s.selected_booking = Some(i64::from(id));
                            s.reveal_limit = count;
                        });
                        win.set_search_text(SharedString::new());
                        win.set_uncategorized_only(false);
                        win.set_status_index(0);
                        win.set_range_index(0);
                        win.set_tag_index(0);
                        win.set_documents_only(false);
                        crate::views_ui::select_reveal_target(
                            &win,
                            &v,
                            crate::nav::Target::AllAccounts,
                        );
                    }
                    Ok(None) => {
                        win.set_sync_status(
                            text(
                                "Die verknüpfte Buchung ist nicht mehr verfügbar.",
                                "The linked transaction is no longer available.",
                            )
                            .into(),
                        );
                        refresh_card(&win);
                    }
                    Err(e) => win.set_sync_status(e.into()),
                }
            });
        });
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

    #[test]
    fn linking_another_pair_preserves_the_unlink_undo_token() {
        let pair = Pair {
            refund: 1,
            refund_part: String::new(),
            original: 2,
            original_part: String::new(),
        };
        let mut ui = Ui {
            undo: Some((pair.clone(), "token".into())),
            ..Default::default()
        };
        ui.edit_succeeded(&Edit::Link(pair.clone()), None);
        assert_eq!(ui.undo.as_ref().unwrap().1, "token");
        ui.edit_succeeded(
            &Edit::Unlink(pair.clone(), None),
            Some((pair.clone(), "next".into())),
        );
        assert_eq!(ui.undo.as_ref().unwrap().1, "next");
        ui.edit_succeeded(&Edit::Restore(pair, "next".into()), None);
        assert!(ui.undo.is_none());
    }

    #[test]
    fn stale_selected_uuid_never_selects_a_different_endpoint() {
        let (_dir, v, f) = fixture();
        let initial = load(&v, f.refund, "", "").unwrap();
        assert!(!initial.own.is_empty());
        let got = load(&v, f.refund, "removed-uuid", "").unwrap();
        assert_eq!(got.own, "removed-uuid");
        assert!(got.candidates.is_empty() && got.suggestions.is_empty() && got.links.is_empty());
        assert!(!got.parts.is_empty());
        v.set_transaction_splits(f.refund, Vec::new()).unwrap();
        let unsplit = load(&v, f.refund, "removed-uuid", "").unwrap();
        assert_eq!(unsplit.own, "removed-uuid");
        assert!(
            unsplit.parts.is_empty()
                && unsplit.candidates.is_empty()
                && unsplit.suggestions.is_empty()
        );
    }

    #[test]
    fn inspector_card_masks_names_details_and_amounts() {
        let (_dir, v, f) = fixture();
        v.link_refund(f.refund, f.part.clone(), f.original, String::new())
            .unwrap();
        let rows = rendered_card_rows(card_rows(&v, f.refund).unwrap(), true);
        assert!(!rows.is_empty());
        for row in rows {
            assert_eq!(row.title.as_str(), daily::mask("private", true));
            assert_eq!(row.detail.as_str(), daily::mask("private", true));
            assert_eq!(row.amount.as_str(), daily::mask("private", true));
        }
        assert!(rendered_card_rows(Vec::new(), true).is_empty());
        let visible = rendered_card_rows(card_rows(&v, f.refund).unwrap(), false);
        assert_ne!(visible[0].title.as_str(), daily::mask("private", true));
        let source = include_str!("refunds_ui.rs");
        let changed = source
            .split("pub fn privacy_changed(")
            .nth(1)
            .unwrap()
            .split("fn search_empty")
            .next()
            .unwrap();
        assert!(changed.contains("refresh_card(win)"));
    }

    #[test]
    fn refund_overlay_stays_below_transfer_and_tan_prompts() {
        let source = include_str!("../ui/main.slint");
        let refund = source.find("if refunds-open: Dialog").unwrap();
        let transfer = source.find("if transfer-open: Rectangle").unwrap();
        let tan = source.find("if tan-open: Rectangle").unwrap();
        assert!(refund < transfer && transfer < tan);
    }

    #[test]
    fn round5_retryable_restore_error_preserves_undo() {
        let mut ui = Ui {
            undo: Some((
                Pair {
                    refund: 1,
                    refund_part: String::new(),
                    original: 2,
                    original_part: String::new(),
                },
                "token".into(),
            )),
            ..Default::default()
        };
        ui.edit_failed("database is busy".into(), true);
        assert!(ui.undo.is_some());
    }

    #[test]
    fn failed_edits_keep_the_error_and_undo_during_reload() {
        let mut ui = Ui {
            undo: Some((
                Pair {
                    refund: 1,
                    refund_part: String::new(),
                    original: 2,
                    original_part: String::new(),
                },
                "unlink-operation".into(),
            )),
            ..Default::default()
        };
        ui.edit_failed("unrelated failure".into(), false);
        assert!(ui.undo.is_some());
        ui.edit_failed("stale token".into(), true);
        ui.invalidate();
        assert_eq!(ui.edit_error, "stale token");
        assert!(ui.undo.is_some());
        assert!(ui.loaded.is_none());
    }

    #[test]
    fn reveal_intent_is_independent_of_sidebar_reload_requests() {
        cancel_reveal();
        let request = REVEAL_REQUEST.with(|r| r.get());
        CARD_REQUEST.with(|r| r.set(r.get().wrapping_add(1)));
        assert_eq!(REVEAL_REQUEST.with(|r| r.get()), request);
        cancel_reveal();
        assert_ne!(REVEAL_REQUEST.with(|r| r.get()), request);
    }

    #[test]
    fn list_reload_does_not_cancel_reveal_or_stale_analytics() {
        let main = include_str!("main.rs");
        let filter = main
            .split("fn apply_filter(")
            .nth(1)
            .unwrap()
            .split("\nfn ")
            .next()
            .unwrap();
        assert!(!filter.contains("cancel_reveal"));
        let views = include_str!("views_ui.rs");
        let normal = views
            .split("pub fn select_target(")
            .nth(1)
            .unwrap()
            .split("pub fn select_reveal_target(")
            .next()
            .unwrap();
        assert!(normal.contains("st.reveal_limit = 0"));
        let reveal = views
            .split("pub fn select_reveal_target(")
            .nth(1)
            .unwrap()
            .split("\npub fn ")
            .next()
            .unwrap();
        assert!(!reveal.contains("reveal_limit = 0"));
    }

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
    fn round7_daily_refund_amount_keeps_both_directions() {
        let (_dir, v, f) = fixture();
        let mut b = v
            .list_transactions(BookingFilter {
                limit: 0,
                ..Default::default()
            })
            .unwrap()
            .transactions
            .into_iter()
            .find(|b| b.id == f.refund)
            .unwrap();
        b.refund_cents = 400;
        b.refunded_cents = 200;
        let row = daily::Row::from(&b);
        let label = crate::refund_amount(&row);
        assert!(label.contains(&crate::money(400, &b.currency)));
        assert!(label.contains(&crate::money(200, &b.currency)));
        b.refund_cents = 0;
        assert_eq!(
            crate::refund_amount(&daily::Row::from(&b)),
            crate::money(200, &b.currency)
        );
        b.refunded_cents = 0;
        assert!(crate::refund_amount(&daily::Row::from(&b)).is_empty());
    }

    #[test]
    fn round7_reveal_survives_newer_rows_between_count_and_load() {
        let (_dir, v, f) = fixture();
        let count = reveal_count(&v, f.original).unwrap().unwrap();
        v.save_manual_booking(
            f.account,
            None,
            girocrab_core::offline::ManualBookingSetup {
                booking_date: "2030-01-01".into(),
                amount_cents: -1,
                counterparty: "Newer".into(),
                purpose: String::new(),
                category_id: None,
                note: String::new(),
            },
        )
        .unwrap();
        let rows = crate::load_reveal_rows(
            &v,
            crate::nav::Target::AllAccounts,
            count,
            Some(f.original),
            Default::default(),
        )
        .unwrap();
        assert!(rows.0.0.iter().any(|r| r.id == f.original));
        assert!(!rows.1);
        assert!(
            crate::load_reveal_rows(
                &v,
                crate::nav::Target::AllAccounts,
                count,
                Some(-1),
                Default::default()
            )
            .unwrap()
            .1
        );
        assert!(
            !crate::load_reveal_rows(
                &v,
                crate::nav::Target::AllAccounts,
                0,
                None,
                Default::default()
            )
            .unwrap()
            .0
            .0
            .is_empty()
        );
    }

    #[test]
    fn round7_matching_suggestion_is_not_an_empty_search() {
        let (_dir, v, f) = fixture();
        let initial = load(&v, f.refund, &f.part, "").unwrap();
        let query = &initial.suggestions[0].candidate.counterparty;
        let loaded = load(&v, f.refund, &f.part, query).unwrap();
        assert!(!search_empty(&loaded, query));
    }

    #[test]
    fn round8_missing_reveal_is_not_a_read_failure() {
        let (_dir, v, _) = fixture();
        assert!(
            crate::load_reveal_rows(
                &v,
                crate::nav::Target::AllAccounts,
                1,
                Some(-1),
                Default::default()
            )
            .is_ok()
        );
    }

    #[test]
    fn round6_empty_search_preserves_suggestions_and_has_feedback() {
        let (_dir, v, demo) = fixture();
        let loaded = load(&v, demo.refund, "", "nothing-matches-xyz").unwrap();
        assert!(!loaded.suggestions.is_empty());
        assert!(loaded.candidates.is_empty());
        let render = include_str!("refunds_ui.rs")
            .split("fn render(win:")
            .nth(1)
            .unwrap()
            .split("pub fn refresh(")
            .next()
            .unwrap();
        assert!(render.contains("set_rf_search_empty(search_empty(d, &u.query))"));
        assert!(
            include_str!("../ui/refunds_dialog.slint")
                .contains("if root.search-empty && !root.loading && root.rows.length > 0")
        );
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
    fn inspector_lookup_is_bounded_to_live_counterpart_days() {
        let (_dir, v, f) = fixture();
        assert!(card_filters(&v, f.refund, &[]).unwrap().is_empty());
        v.link_refund(f.refund, f.part, f.original, String::new())
            .unwrap();
        let mut links = v.refund_links(f.refund).unwrap();
        let filters = card_filters(&v, f.refund, &links).unwrap();
        assert_eq!(filters.len(), 1);
        let filter = &filters[0];
        assert_eq!(filter.date_from.as_deref(), Some("2026-01-14"));
        assert_eq!(filter.date_to, filter.date_from);
        let mut second = links[0].clone();
        second.original_id = v
            .save_manual_booking(
                f.account,
                None,
                girocrab_core::offline::ManualBookingSetup {
                    booking_date: "2020-01-01".into(),
                    amount_cents: -10000,
                    counterparty: "Earlier".into(),
                    purpose: String::new(),
                    category_id: None,
                    note: String::new(),
                },
            )
            .unwrap();
        links.push(second);
        let filters = card_filters(&v, f.refund, &links).unwrap();
        assert_eq!(filters.len(), 2);
        assert_eq!(filters[0].date_from.as_deref(), Some("2020-01-01"));
        assert_eq!(filters[1].date_from.as_deref(), Some("2026-01-14"));
        assert!(filters.iter().all(|f| f.date_from == f.date_to));
        links.iter_mut().for_each(|l| l.inactive = true);
        assert!(card_filters(&v, f.refund, &links).unwrap().is_empty());
        assert!(card_rows(&v, -1).unwrap().is_empty());
    }

    #[test]
    fn refund_modal_escape_and_privacy_use_safe_presentation_paths() {
        let panel = include_str!("../ui/refunds_dialog.slint");
        let escape = panel
            .split("if event.text ==")
            .nth(1)
            .unwrap()
            .split('\n')
            .next()
            .unwrap();
        assert!(escape.contains("root.shortcut("));
        assert!(!escape.contains("root.close("));
        let main = include_str!("main.rs");
        assert!(main.contains("progress: if privacy { 0.0 } else { s.share }"));
        let logos = main
            .split("logos::load(vault, wanted,")
            .nth(1)
            .unwrap()
            .split("/// Fills the cashflow")
            .next()
            .unwrap();
        assert!(logos.contains("render_booking_rows(&win)"));
        assert!(!logos.contains("render_bookings(&win)"));
    }

    #[test]
    fn inspector_jump_reaches_beyond_the_default_page() {
        let (_dir, v, f) = fixture();
        assert_eq!(reveal_count(&v, -1).unwrap(), None);
        for _ in 0..crate::ROW_LIMIT {
            v.save_manual_booking(
                f.account,
                None,
                girocrab_core::offline::ManualBookingSetup {
                    booking_date: "2027-01-01".into(),
                    amount_cents: -1,
                    counterparty: "Later".into(),
                    purpose: String::new(),
                    category_id: None,
                    note: String::new(),
                },
            )
            .unwrap();
        }
        let count = reveal_count(&v, f.original).unwrap().unwrap();
        assert!(count > crate::ROW_LIMIT);
        assert!(
            !crate::load_rows(
                &v,
                crate::nav::Target::AllAccounts,
                crate::ROW_LIMIT,
                Default::default()
            )
            .unwrap()
            .0
            .iter()
            .any(|r| r.id == f.original)
        );
        assert!(
            crate::load_rows(
                &v,
                crate::nav::Target::AllAccounts,
                count,
                Default::default()
            )
            .unwrap()
            .0
            .iter()
            .any(|r| r.id == f.original)
        );
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
            .unlink_refund(f.refund, f.part.clone(), f.original, String::new(), None)
            .unwrap();
        assert!(card_rows(&v, f.original).unwrap().is_empty());
        v.restore_refund(f.refund, f.part, f.original, String::new(), token)
            .unwrap();
        assert_eq!(card_rows(&v, f.original).unwrap().len(), 1);
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
        let (_, Some((p, token))) = edit(&v, &Edit::Unlink(p, None)).unwrap() else {
            panic!("token")
        };
        assert_eq!(
            load(&v, f.refund, &f.part, "").unwrap().links[0].allocated_cents,
            4000
        );
        edit(&v, &Edit::Restore(p.clone(), token.clone())).unwrap();
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
        edit(&v, &Edit::Unlink(p.clone(), None)).unwrap();
        assert!(!edit(&v, &Edit::Unlink(p.clone(), None)).unwrap().0);
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
