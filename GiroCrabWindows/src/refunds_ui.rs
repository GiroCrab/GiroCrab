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
    search: String,
    /// Ids linked right now (a click on one of them removes the link).
    linked: Vec<i64>,
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

/// What the dialog shows for one booking.
struct Loaded {
    links: RefundLinks,
    suggestions: Vec<RefundSuggestion>,
    candidates: Vec<RefundBooking>,
}

fn load(v: &Vault, booking: i64, search: &str) -> Result<Loaded, String> {
    let links = v.refund_links(booking).map_err(|e| e.to_string())?;
    let suggestions = if search.trim().is_empty() {
        v.refund_suggestions(booking).map_err(|e| e.to_string())?
    } else {
        Vec::new()
    };
    let candidates = v
        .refund_candidates(booking, search.to_string(), 200)
        .map_err(|e| e.to_string())?;
    Ok(Loaded {
        links,
        suggestions,
        candidates,
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
    let used_up = l.links.open_cents == 0;
    let cash = |c: i64, cur: &str| daily::mask(&money(c, cur), privacy);
    let row = |b: &RefundBooking, kind: i32, reason: &str| RefundRow {
        kind,
        id: b.id as i32,
        title: daily::mask(&b.display_name, privacy).into(),
        detail: format!(
            "{} · {}",
            german_date(&b.booking_date),
            daily::mask(&b.account_name, privacy)
        )
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
    let shown = |id: i64| linked.iter().any(|b| b.id == id);
    let top: Vec<&RefundSuggestion> = l
        .suggestions
        .iter()
        .filter(|s| !shown(s.booking.id))
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
        .filter(|c| !shown(c.id) && !top.iter().any(|s| s.booking.id == c.id))
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
    let Some((_, cents, currency)) = with_ui(|u| u.booking.clone()) else {
        return;
    };
    let (from_refund, privacy) = (cents > 0, privacy_on(win));
    let searching = with_ui(|u| !u.search.trim().is_empty());
    let linked = if from_refund {
        &loaded.links.originals
    } else {
        &loaded.links.refunds
    };
    with_ui(|u| u.linked = linked.iter().map(|b| b.id).collect());
    let hint = match (
        loaded.links.open_cents == 0 && !linked.is_empty(),
        from_refund,
    ) {
        (false, _) => "",
        (true, true) => {
            "Die Erstattung ist vollständig zugeordnet. Heben Sie eine Verknüpfung auf, um eine andere Buchung zu wählen."
        }
        (true, false) => {
            "Die Buchung ist vollständig erstattet. Heben Sie eine Verknüpfung auf, um eine andere Erstattung zu wählen."
        }
    };
    win.set_rf_sums(sums(&loaded.links, from_refund, cents.abs(), &currency, privacy).into());
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
    let Some((booking, search)) = with_ui(|u| Some((u.booking.as_ref()?.0, u.search.clone())))
    else {
        return;
    };
    let weak = win.as_weak();
    std::thread::spawn(move || {
        let loaded = load(&v, booking, &search);
        let _ = weak.upgrade_in_event_loop(move |win| {
            // an answer for an earlier booking or search is dropped
            let current =
                with_ui(|u| u.booking.as_ref().map(|b| b.0) == Some(booking) && u.search == search);
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
        let privacy = privacy_on(&win);
        win.set_rf_from_refund(cents > 0);
        win.set_rf_title(daily::mask(&title, privacy).into());
        win.set_rf_amount(daily::mask(&money(cents, &currency), privacy).into());
        win.set_rf_sums(SharedString::new());
        win.set_rf_hint(SharedString::new());
        win.set_rf_status(SharedString::new());
        win.set_rf_rows(ModelRc::new(VecModel::from(Vec::<RefundRow>::new())));
        with_ui(|u| {
            *u = Ui {
                booking: Some((id, cents, currency)),
                ..Ui::default()
            }
        });
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
    win.on_refund_toggle(move |other| {
        let (Some(win), Some(v)) = (weak.upgrade(), vault()) else {
            return;
        };
        let other = i64::from(other);
        let Some((booking, cents, unlink)) = with_ui(|u| {
            let b = u.booking.as_ref()?;
            Some((b.0, b.1, u.linked.contains(&other)))
        }) else {
            return;
        };
        let (refund, original) = if cents > 0 {
            (booking, other)
        } else {
            (other, booking)
        };
        let weak = win.as_weak();
        std::thread::spawn(move || {
            let done = if unlink {
                v.unlink_refund(refund, original)
            } else {
                v.link_refund(refund, original)
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
                open_cents: open,
            },
            suggestions: vec![RefundSuggestion {
                booking: booking(3, -1599, None, 1599),
                reason: RefundMatch::SameAmount,
            }],
            candidates: vec![booking(3, -1599, None, 1599), booking(4, -500, None, 0)],
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
}
