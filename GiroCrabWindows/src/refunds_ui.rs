//! Slint wiring for refund links (#714, `docs/ERSTATTUNGEN_UI.md`): a refund is linked to the
//! bookings it pays back, a booking to its refunds. The dialog lives in `ui/refunds_dialog.slint`.
//! Every vault call runs on a worker thread; results come back through `upgrade_in_event_loop`.

use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use girocrab_core::Vault;
use girocrab_core::refunds::{RefundBooking, RefundLinks, RefundMatch, RefundSuggestion};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::{
    MainWindow, RefundRow, daily, german_date, money, privacy_on, refresh_dashboard, reload,
    with_state,
};

type VaultSlot = Arc<Mutex<Option<Arc<Vault>>>>;

/// Dialog state on the UI thread.
#[derive(Default)]
struct Ui {
    /// The booking the dialog is open for: (id, amount in cents, currency).
    booking: Option<(i64, i64, String)>,
    /// Its counterparty, for the dialog's title.
    title: String,
    search: String,
    /// Linked right now, as (id, part of the split original): a click on one of them removes
    /// the link.
    linked: Vec<(i64, u32, u32)>,
    parts: Vec<u32>,
    own_part: u32,
    /// The part the rows and the side on screen were loaded for: while it is not `own_part`
    /// (a part was just picked), a click on a row would mix the new part with the old side.
    shown_part: u32,
    /// The side of the selected part, else of the booking (`Loaded::from_refund`).
    from_refund: bool,
}

thread_local! {
    static UI: RefCell<Ui> = RefCell::new(Ui::default());
}

fn with_ui<R>(f: impl FnOnce(&mut Ui) -> R) -> R {
    UI.with(|u| f(&mut u.borrow_mut()))
}

/// Closes the dialog and forgets its state when the vault locks.
pub fn on_lock(win: &MainWindow) {
    win.set_refunds_open(false);
    with_ui(|u| *u = Ui::default());
}

/// The booking the dialog is open for, in its head.
fn head(win: &MainWindow) {
    let Some(((_, cents, currency), title)) =
        with_ui(|u| Some((u.booking.clone()?, u.title.clone())))
    else {
        return;
    };
    let privacy = privacy_on(win);
    win.set_rf_title(daily::mask(&title, privacy).into());
    win.set_rf_amount(daily::mask(&money(cents, &currency), privacy).into());
}

/// Privacy mode was switched: an open dialog shows its names and amounts accordingly.
pub fn privacy_changed(win: &MainWindow, v: Arc<Vault>) {
    if win.get_refunds_open() {
        head(win);
        // what was drawn for the other mode goes at once, not when the new load arrives
        // (it may be slow or fail)
        win.set_rf_sums(SharedString::new());
        win.set_rf_hint(SharedString::new());
        win.set_rf_parts(ModelRc::new(VecModel::from(Vec::<SharedString>::new())));
        win.set_rf_rows(ModelRc::new(VecModel::from(Vec::new())));
        refresh(win, v);
    }
}

/// What the dialog shows for one booking.
struct Loaded {
    links: RefundLinks,
    suggestions: Vec<RefundSuggestion>,
    candidates: Vec<RefundBooking>,
    /// The parts of the booking: (position from 1, amount, label).
    parts: Vec<(u32, i64, String)>,
    /// The part the dialog links, 0 for a booking that is not split.
    own_part: u32,
    /// Whether that part (else the booking) pays back: a part of the other direction (a
    /// credit in a purchase) is linked on its own side.
    from_refund: bool,
    /// What that part (else the booking) has left for a further link.
    open: i64,
}

fn load(
    v: &Vault,
    booking: i64,
    search: &str,
    cents: i64,
    own_part: u32,
    currency: &str,
) -> Result<Loaded, String> {
    let splits = v
        .list_transaction_splits(booking)
        .map_err(|e| e.to_string())?;
    let cats = v.list_categories(None, None).map_err(|e| e.to_string())?;
    let parts = splits
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let category = cats
                .iter()
                .find(|c| Some(c.id) == s.category_id)
                .map_or("", |c| c.name.as_str());
            (
                (i + 1) as u32,
                s.amount_cents,
                format!(
                    "Teil {} · {} · {} · {}",
                    i + 1,
                    category,
                    s.note,
                    money(s.amount_cents, currency)
                ),
            )
        })
        .collect::<Vec<_>>();
    // the picked part, else the first one on the booking's own side
    let own = parts
        .iter()
        .find(|p| p.0 == own_part)
        .or_else(|| parts.iter().find(|p| (p.1 > 0) == (cents > 0)))
        .or(parts.first());
    let own_part = own.map_or(0, |p| p.0);
    let from_refund = own.map_or(cents, |p| p.1) > 0;
    let links = v.refund_links(booking).map_err(|e| e.to_string())?;
    let suggestions = if search.trim().is_empty() {
        v.refund_part_suggestions(booking, from_refund, own_part)
            .map_err(|e| e.to_string())?
    } else {
        Vec::new()
    };
    let candidates = v
        .refund_part_candidates(booking, from_refund, search.to_string(), 200)
        .map_err(|e| e.to_string())?;
    let open = v
        .refund_part_open(booking, from_refund, own_part)
        .map_err(|e| e.to_string())?;
    Ok(Loaded {
        links,
        suggestions,
        candidates,
        parts,
        own_part,
        from_refund,
        open,
    })
}

fn header(title: String) -> RefundRow {
    RefundRow {
        title: title.into(),
        ..RefundRow::default()
    }
}

/// The rows of the list: linked bookings in the order they were linked (with their share), up to
/// three suggestions, then the other candidates. A row that takes no further link says why.
fn rows(l: &Loaded, from_refund: bool, searching: bool, privacy: bool) -> Vec<RefundRow> {
    let linked = if from_refund {
        &l.links.originals
    } else {
        &l.links.refunds
    };
    let used_up = l.open == 0;
    let part_used_up = used_up && l.parts.len() > 1 && l.own_part > 0;
    let cash = |c: i64, cur: &str| daily::mask(&money(c, cur), privacy);
    let row = |b: &RefundBooking, kind: i32, reason: &str| RefundRow {
        kind,
        id: b.id as i32,
        part: b.part as i32,
        refund_part: b.refund_part as i32,
        title: daily::mask(&b.display_name, privacy).into(),
        // a split booking is linked by one of its parts
        detail: [
            german_date(&b.booking_date),
            daily::mask(&b.account_name, privacy),
            if b.part > 0 {
                format!("Teil {}", b.part)
            } else {
                String::new()
            },
            daily::mask(&b.part_label, privacy),
            if b.refund_part > 0 {
                format!(
                    "aus Teil {} · {}",
                    b.refund_part,
                    daily::mask(&b.refund_part_label, privacy)
                )
            } else {
                String::new()
            },
        ]
        .into_iter()
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
        .into(),
        amount: cash(b.amount_cents, &b.currency).into(),
        share: match b.share_cents {
            Some(s) if kind == 1 => format!("Zugeordnet: {}", cash(s, &b.currency)).into(),
            _ => SharedString::new(),
        },
        share_empty: kind == 1 && b.share_cents == Some(0),
        reason: reason.into(),
        blocked: match (kind, used_up, b.open_cents == 0, from_refund) {
            (1, ..) => "",
            (_, true, _, true) if part_used_up => "Dieser Teil ist bereits vollständig zugeordnet.",
            (_, true, _, false) if part_used_up => "Dieser Teil ist bereits vollständig erstattet.",
            (_, true, _, true) => "Die Erstattung ist bereits vollständig zugeordnet.",
            (_, true, _, false) => "Die Buchung ist bereits vollständig erstattet.",
            (_, false, true, true) => "Diese Buchung ist bereits vollständig erstattet.",
            (_, false, true, false) => "Diese Erstattung ist bereits vollständig zugeordnet.",
            _ => "",
        }
        .into(),
    };
    let mut out = Vec::new();
    if !linked.is_empty() {
        out.push(header(if from_refund {
            format!("Verknüpfte ursprüngliche Buchungen ({})", linked.len())
        } else {
            format!("Verknüpfte Erstattungen ({})", linked.len())
        }));
        out.extend(linked.iter().map(|b| row(b, 1, "")));
    }
    let shown = |c: &RefundBooking| {
        linked
            .iter()
            .any(|b| (b.id, b.part, b.refund_part) == (c.id, c.part, c.refund_part))
    };
    let top: Vec<&RefundSuggestion> = l
        .suggestions
        .iter()
        .filter(|s| !shown(&s.booking))
        .collect();
    if !top.is_empty() {
        out.push(header("Vorschläge".into()));
        out.extend(top.iter().map(|s| {
            let reason = match s.reason {
                RefundMatch::SameAmount => "Gleicher Betrag",
                RefundMatch::SameMerchant => "Gleicher Händler",
                RefundMatch::SameReference => "Gleiche Referenz",
            };
            row(&s.booking, 2, reason)
        }));
    }
    let others: Vec<&RefundBooking> = l
        .candidates
        .iter()
        .filter(|c| {
            !shown(c)
                && !top.iter().any(|s| {
                    (s.booking.id, s.booking.part, s.booking.refund_part)
                        == (c.id, c.part, c.refund_part)
                })
        })
        .collect();
    if !others.is_empty() {
        out.push(header(
            if searching {
                "Suchergebnisse"
            } else {
                "Weitere Buchungen"
            }
            .into(),
        ));
        out.extend(others.iter().map(|b| row(b, 3, "")));
    }
    out
}

/// "Erstattet: X von Y" for a booking, "Zugeordnet: X von Y · Noch offen: Z" for a refund.
fn sums(
    links: &RefundLinks,
    from_refund: bool,
    total: i64,
    currency: &str,
    privacy: bool,
) -> String {
    let cash = |c: i64| daily::mask(&money(c, currency), privacy);
    match (from_refund, links.assigned_cents, links.refunded_cents) {
        (true, Some(a), _) if a < total => format!(
            "Zugeordnet: {} von {} · Noch offen: {}",
            cash(a),
            cash(total),
            cash(total - a)
        ),
        (true, Some(a), _) => format!("Zugeordnet: {} von {}", cash(a), cash(total)),
        (false, _, Some(r)) => format!("Erstattet: {} von {}", cash(r), cash(total)),
        _ => String::new(),
    }
}

fn show(win: &MainWindow, loaded: Loaded) {
    let Some((_, _, currency)) = with_ui(|u| u.booking.clone()) else {
        return;
    };
    let (from_refund, privacy) = (loaded.from_refund, privacy_on(win));
    win.set_rf_from_refund(from_refund);
    let searching = with_ui(|u| !u.search.trim().is_empty());
    let linked = if from_refund {
        &loaded.links.originals
    } else {
        &loaded.links.refunds
    };
    let index = with_ui(|u| {
        u.linked = linked
            .iter()
            .map(|b| (b.id, b.part, b.refund_part))
            .collect();
        u.parts = loaded.parts.iter().map(|p| p.0).collect();
        u.own_part = loaded.own_part;
        u.shown_part = loaded.own_part;
        u.from_refund = from_refund;
        u.parts.iter().position(|p| *p == u.own_part).unwrap_or(0)
    });
    win.set_rf_parts(ModelRc::new(VecModel::from(
        loaded
            .parts
            .iter()
            .map(|(_, _, label)| SharedString::from(daily::mask(label, privacy)))
            .collect::<Vec<_>>(),
    )));
    win.set_rf_part_index(index as i32);
    let part_used_up = loaded.parts.len() > 1 && loaded.own_part > 0;
    let hint = match (loaded.open == 0 && !linked.is_empty(), from_refund) {
        (false, _) => "",
        (true, true) if part_used_up => {
            "Dieser Teil ist vollständig zugeordnet. Wählen Sie einen anderen Teil oder heben Sie eine Verknüpfung auf."
        }
        (true, false) if part_used_up => {
            "Dieser Teil ist vollständig erstattet. Wählen Sie einen anderen Teil oder heben Sie eine Verknüpfung auf."
        }
        (true, true) => {
            "Die Erstattung ist vollständig zugeordnet. Heben Sie eine Verknüpfung auf, um eine andere Buchung zu wählen."
        }
        (true, false) => {
            "Die Buchung ist vollständig erstattet. Heben Sie eine Verknüpfung auf, um eine andere Erstattung zu wählen."
        }
    };
    // of a split booking: its parts on the side the dialog shows
    let total = if from_refund {
        loaded.links.refund_total_cents
    } else {
        loaded.links.original_total_cents
    };
    win.set_rf_sums(sums(&loaded.links, from_refund, total, &currency, privacy).into());
    win.set_rf_hint(hint.into());
    win.set_rf_rows(ModelRc::new(VecModel::from(rows(
        &loaded,
        from_refund,
        searching,
        privacy,
    ))));
    win.set_rf_status(SharedString::new());
}

/// Loads the dialog's content for the open booking on a worker thread.
fn refresh(win: &MainWindow, v: Arc<Vault>) {
    let Some(((booking, cents, currency), search, own_part)) =
        with_ui(|u| Some((u.booking.clone()?, u.search.clone(), u.own_part)))
    else {
        return;
    };
    let weak = win.as_weak();
    std::thread::spawn(move || {
        let loaded = load(&v, booking, &search, cents, own_part, &currency);
        let _ = weak.upgrade_in_event_loop(move |win| {
            // an answer for another vault session, an earlier booking, search or part is dropped
            if !crate::is_current(&v) {
                return;
            }
            let current = with_ui(|u| {
                u.booking.as_ref().map(|b| b.0) == Some(booking)
                    && u.search == search
                    && u.own_part == own_part
            });
            if !current {
                return;
            }
            match loaded {
                Ok(l) => show(&win, l),
                Err(e) => win.set_rf_status(e.into()),
            }
        });
    });
}

pub fn wire(win: &MainWindow, slot: VaultSlot) {
    let vault_of = move || slot.lock().unwrap().clone();

    // suggestions, the open amount and the side follow the picked part
    let (weak, vault) = (win.as_weak(), vault_of.clone());
    win.on_refund_part_picked(move |index| {
        with_ui(|u| u.own_part = u.parts.get(index as usize).copied().unwrap_or(0));
        if let (Some(win), Some(v)) = (weak.upgrade(), vault()) {
            // the rows on screen belong to the part shown before: a click on one is refused,
            // so they go until the new ones are there (and stay away if the load fails)
            win.set_rf_sums(SharedString::new());
            win.set_rf_hint(SharedString::new());
            win.set_rf_rows(ModelRc::new(VecModel::from(Vec::<RefundRow>::new())));
            refresh(&win, v);
        }
    });

    let (weak, vault) = (win.as_weak(), vault_of.clone());
    win.on_refunds_request(move || {
        let (Some(win), Some(v)) = (weak.upgrade(), vault()) else {
            return;
        };
        let picked = with_state(|st| {
            let id = st.selected_booking?;
            let r = st.rows.iter().find(|r| r.id == id)?;
            Some((
                id,
                r.counterparty.clone(),
                r.amount_cents,
                r.currency.clone(),
            ))
        });
        let Some((id, title, cents, currency)) = picked else {
            return;
        };
        if cents == 0 {
            return;
        }
        win.set_rf_from_refund(cents > 0);
        win.set_rf_sums(SharedString::new());
        win.set_rf_hint(SharedString::new());
        win.set_rf_status(SharedString::new());
        win.set_rf_rows(ModelRc::new(VecModel::from(Vec::<RefundRow>::new())));
        win.set_rf_parts(ModelRc::new(VecModel::from(Vec::<SharedString>::new())));
        with_ui(|u| {
            *u = Ui {
                booking: Some((id, cents, currency)),
                title,
                ..Ui::default()
            }
        });
        head(&win);
        win.set_refunds_open(true);
        refresh(&win, v);
    });

    let (weak, vault) = (win.as_weak(), vault_of.clone());
    win.on_refund_search(move |text| {
        let (Some(win), Some(v)) = (weak.upgrade(), vault()) else {
            return;
        };
        with_ui(|u| u.search = text.to_string());
        refresh(&win, v);
    });

    let (weak, vault) = (win.as_weak(), vault_of);
    win.on_refund_toggle(move |other, part, refund_part| {
        let (Some(win), Some(v)) = (weak.upgrade(), vault()) else {
            return;
        };
        let other = i64::from(other);
        let part = u32::try_from(part).unwrap_or(0);
        let refund_part = u32::try_from(refund_part).unwrap_or(0);
        let Some((booking, from_refund, unlink, own_part)) = with_ui(|u| {
            let b = u.booking.as_ref()?;
            if u.own_part != u.shown_part {
                return None;
            }
            Some((
                b.0,
                u.from_refund,
                u.linked.contains(&(other, part, refund_part)),
                u.own_part,
            ))
        }) else {
            return;
        };
        let (refund, original) = if from_refund {
            (booking, other)
        } else {
            (other, booking)
        };
        let weak = win.as_weak();
        std::thread::spawn(move || {
            let (part, refund_part) = if unlink {
                (part, refund_part)
            } else if from_refund {
                (part, own_part)
            } else {
                (own_part, refund_part)
            };
            let done = if unlink {
                v.unlink_refund_parts(refund, refund_part, original, part)
            } else {
                v.link_refund_parts(refund, refund_part, original, part)
            }
            .map_err(|e| e.to_string());
            let _ = weak.upgrade_in_event_loop(move |win| match done {
                Ok(()) => {
                    refresh(&win, v.clone());
                    // the sign in the rows and the sums: only the assigned part lowers the expenses
                    reload(&win, v.clone());
                    refresh_dashboard(&win, v);
                }
                Err(e) => win.set_rf_status(e.into()),
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn booking(id: i64, cents: i64, share: Option<i64>, open: i64) -> RefundBooking {
        RefundBooking {
            id,
            account_id: 1,
            account_name: "Giro".into(),
            booking_date: "2026-11-01".into(),
            amount_cents: cents,
            currency: "EUR".into(),
            display_name: format!("Händler {id}"),
            counterparty: String::new(),
            purpose: String::new(),
            share_cents: share,
            open_cents: open,
            part: 0,
            part_label: String::new(),
            refund_part: 0,
            refund_part_label: String::new(),
        }
    }

    fn loaded(open: i64) -> Loaded {
        Loaded {
            links: RefundLinks {
                originals: vec![
                    booking(1, -299, Some(299), 0),
                    booking(2, -2490, Some(1300), 1190),
                ],
                refunds: Vec::new(),
                refunded_cents: None,
                assigned_cents: Some(1599 - open),
                refund_total_cents: 1599,
                refund_open_cents: open,
                original_total_cents: 0,
                original_open_cents: 0,
                refund_income_cents: open,
            },
            suggestions: vec![RefundSuggestion {
                booking: booking(3, -1599, None, 1599),
                reason: RefundMatch::SameAmount,
            }],
            candidates: vec![booking(3, -1599, None, 1599), booking(4, -500, None, 0)],
            parts: Vec::new(),
            own_part: 0,
            from_refund: true,
            open,
        }
    }

    #[test]
    fn linked_rows_come_first_in_link_order_with_their_share() {
        let r = rows(&loaded(0), true, false, false);
        let kinds: Vec<i32> = r.iter().map(|x| x.kind).collect();
        assert_eq!(kinds, [0, 1, 1, 0, 2, 0, 3]);
        assert_eq!(
            r[1].share.as_str(),
            format!("Zugeordnet: {}", money(299, "EUR"))
        );
        assert_eq!(
            r[2].share.as_str(),
            format!("Zugeordnet: {}", money(1300, "EUR"))
        );
        assert_eq!(r[4].reason.as_str(), "Gleicher Betrag");
    }

    #[test]
    fn a_used_up_refund_blocks_every_unlinked_row() {
        let r = rows(&loaded(0), true, false, false);
        assert!(r[1].blocked.is_empty() && r[2].blocked.is_empty());
        assert!(!r[4].blocked.is_empty() && !r[6].blocked.is_empty());
    }

    #[test]
    fn with_room_left_only_a_fully_refunded_booking_is_blocked() {
        let r = rows(&loaded(100), true, false, false);
        assert!(r[4].blocked.is_empty());
        assert_eq!(
            r[6].blocked.as_str(),
            "Diese Buchung ist bereits vollständig erstattet."
        );
    }

    #[test]
    fn sums_never_exceed_the_amount() {
        let l = loaded(100);
        let text = sums(&l.links, true, 1599, "EUR", false);
        assert!(text.starts_with("Zugeordnet: ") && text.contains("Noch offen: "));
        assert!(!sums(&l.links, true, 1599, "EUR", true).contains(&money(1599, "EUR")));
    }

    #[test]
    fn candidates_keep_distinct_income_parts_and_linked_rows_keep_the_exact_part() {
        let mut l = loaded(100);
        let mut first = booking(9, 100, Some(100), 0);
        first.refund_part = 1;
        let mut second = first.clone();
        second.refund_part = 2;
        l.links.refunds = vec![first.clone()];
        l.suggestions.clear();
        l.candidates = vec![first, second];
        let r = rows(&l, false, false, false);
        assert_eq!(r.iter().filter(|r| r.kind == 1).count(), 1);
        assert_eq!(r.iter().find(|r| r.kind == 1).unwrap().refund_part, 1);
        assert_eq!(r.iter().find(|r| r.kind == 3).unwrap().refund_part, 2);
    }

    #[test]
    fn demo_split_income_links_only_the_selected_part_and_net_expenses_refresh() {
        let path = std::env::temp_dir().join(format!(
            "girocrab-refund-parts-{}-{:?}.sqlite",
            std::process::id(),
            std::thread::current().id()
        ));
        let v = Vault::open(path.display().to_string(), vec![7; 32]).unwrap();
        v.seed_demo_data().unwrap();
        let bookings = v
            .list_transactions(girocrab_core::BookingFilter {
                limit: 2000,
                ..Default::default()
            })
            .unwrap()
            .transactions;
        let income = bookings.iter().find(|b| b.amount_cents == 280000).unwrap();
        let rent = bookings.iter().find(|b| b.amount_cents == -115000).unwrap();
        let total = || {
            crate::analytics::net_expenses(
                &v.list_category_expenses(None, None, vec![]).unwrap(),
                6,
            )
            .iter()
            .map(|s| s.cents)
            .sum::<i64>()
        };
        let before = total();
        v.set_transaction_splits(
            income.id,
            vec![270000, 10000]
                .into_iter()
                .map(|amount_cents| girocrab_core::TransactionSplitInput {
                    category_id: income.category_id,
                    amount_cents,
                    note: String::new(),
                })
                .collect(),
        )
        .unwrap();
        let loaded = load(&v, income.id, "", 280000, 0, "EUR").unwrap();
        assert_eq!(loaded.parts.iter().map(|p| p.0).collect::<Vec<_>>(), [1, 2]);
        v.link_refund_parts(income.id, 2, rent.id, 0).unwrap();
        assert_eq!(before - total(), 10000);
        let loaded = load(&v, income.id, "", 280000, 0, "EUR").unwrap();
        assert_eq!(loaded.links.originals[0].refund_part, 2);
        // QA v12: from the expense the split income is offered by its parts, never as a whole;
        // its used-up part 2 is shown as such and part 1 can still be linked
        let from_rent = load(&v, rent.id, "Gehalt", -115000, 0, "EUR").unwrap();
        let offered: Vec<_> = from_rent
            .candidates
            .iter()
            .chain(&from_rent.links.refunds)
            .filter(|c| c.id == income.id)
            .map(|c| c.refund_part)
            .collect();
        assert!(!offered.contains(&0) && offered.contains(&1), "{offered:?}");
        // the picked part 2 of the income is used up while part 1 is not
        let part2 = load(&v, income.id, "", 280000, 2, "EUR").unwrap();
        assert_eq!((part2.own_part, part2.open), (2, 0));
        assert!(part2.suggestions.is_empty());
        assert!(
            rows(&part2, true, false, false)
                .iter()
                .filter(|r| r.kind == 3)
                .all(|r| r.blocked.contains("Teil"))
        );
        assert!(load(&v, income.id, "", 280000, 1, "EUR").unwrap().open > 0);
        v.unlink_refund_parts(income.id, 2, rent.id, 0).unwrap();
        assert_eq!(total(), before);
        // a credit in a purchase pays back: the dialog changes its side with the part
        v.set_transaction_splits(
            rent.id,
            vec![-120000, 5000]
                .into_iter()
                .map(|amount_cents| girocrab_core::TransactionSplitInput {
                    category_id: rent.category_id,
                    amount_cents,
                    note: String::new(),
                })
                .collect(),
        )
        .unwrap();
        let credit = load(&v, rent.id, "", -115000, 2, "EUR").unwrap();
        assert!(credit.from_refund && credit.open == 5000);
        assert!(
            !load(&v, rent.id, "", -115000, 0, "EUR")
                .unwrap()
                .from_refund
        );
        v.set_transaction_splits(rent.id, Vec::new()).unwrap();
        assert!(load(&v, -1, "", 1, 0, "EUR").is_err());
        drop(v);
        let _ = std::fs::remove_file(path);
    }
}
