//! The QA review's tests of the refund links against the public `Vault` API (#714).
use super::*;
use crate::{models::*, offline::ManualBookingSetup, test_support::TempDir};

fn fixture() -> (TempDir, std::sync::Arc<Vault>, i64) {
    let d = TempDir::new("refund-qa");
    let v = Vault::open(d.join("v").to_string_lossy().into(), vec![7; 32]).unwrap();
    let a = v
        .create_offline_account("QA".into(), AccountKind::Checking, "EUR".into(), 0)
        .unwrap();
    (d, v, a)
}
fn add(v: &Vault, a: i64, amount: i64, date: &str, vendor: &str, purpose: &str) -> i64 {
    v.save_manual_booking(
        a,
        None,
        ManualBookingSetup {
            booking_date: date.into(),
            amount_cents: amount,
            counterparty: vendor.into(),
            purpose: purpose.into(),
            category_id: None,
            note: String::new(),
        },
    )
    .unwrap()
}
fn booking(v: &Vault, a: i64, amount: i64) -> i64 {
    add(v, a, amount, &crate::dates::today_iso(), "QA vendor", "")
}
fn allocations(v: &Vault, r: i64) -> Vec<(i64, Option<i64>)> {
    v.refund_links(r)
        .unwrap()
        .originals
        .into_iter()
        .map(|b| (b.id, b.share_cents))
        .collect()
}
fn categories(v: &Vault) -> (i64, i64) {
    let make = |s: &str| {
        v.create_category(s.into(), None, "tag".into(), "#abcdef".into())
            .unwrap()
    };
    (make("QA A"), make("QA B"))
}
fn split(v: &Vault, id: i64, a: i64, b: i64, sign: i64) {
    v.set_transaction_splits(
        id,
        vec![
            TransactionSplitInput {
                category_id: Some(a),
                amount_cents: sign * 6000,
                note: "A".into(),
            },
            TransactionSplitInput {
                category_id: Some(b),
                amount_cents: sign * 4000,
                note: "B".into(),
            },
        ],
    )
    .unwrap();
}
fn page(v: &Vault, cat: Option<i64>) -> BookingPage {
    v.list_transactions(BookingFilter {
        category_id: cat,
        ..Default::default()
    })
    .unwrap()
}

#[test]
fn qa_three_fill_middle_unlink_relink_and_exhaustion() {
    let (_d, v, a) = fixture();
    let r = booking(&v, a, 10000);
    let x = booking(&v, a, -2000);
    let y = booking(&v, a, -3000);
    let z = booking(&v, a, -8000);
    for o in [x, y, z] {
        v.link_refund(r, o, 0).unwrap();
    }
    assert_eq!(
        allocations(&v, r),
        vec![(x, Some(2000)), (y, Some(3000)), (z, Some(5000))]
    );
    let fourth = booking(&v, a, -1000);
    assert!(v.link_refund(r, fourth, 0).is_err());
    v.link_refund(r, z, 0).unwrap();
    v.unlink_refund(r, y, 0).unwrap();
    assert_eq!(allocations(&v, r), vec![(x, Some(2000)), (z, Some(8000))]);
    assert!(v.link_refund(r, y, 0).is_err());
    v.unlink_refund(r, z, 0).unwrap();
    v.link_refund(r, y, 0).unwrap();
    v.link_refund(r, z, 0).unwrap();
    assert_eq!(
        allocations(&v, r),
        vec![(x, Some(2000)), (y, Some(3000)), (z, Some(5000))]
    );
}

#[test]
fn qa_larger_refund_only_assigned_part_in_totals_and_insights() {
    let (_d, v, a) = fixture();
    let r = booking(&v, a, 10000);
    for n in [1000, 2000, 3000] {
        let o = booking(&v, a, -n);
        v.link_refund(r, o, 0).unwrap();
    }
    assert_eq!(v.refund_links(r).unwrap().open_cents, 4000);
    assert_eq!(
        (page(&v, None).income_cents, page(&v, None).expense_cents),
        (4000, 0)
    );
    let flow = v.selection_flow(vec![a], None, None).unwrap();
    assert_eq!((flow.income_cents, flow.consumption_cents), (4000, 0));
}

#[test]
fn qa_invalid_sign_self_other_refund_and_deleted() {
    let (_d, v, a) = fixture();
    let r = booking(&v, a, 5000);
    let income = booking(&v, a, 2000);
    let o = booking(&v, a, -10000);
    assert!(v.link_refund(r, r, 0).is_err());
    assert!(v.link_refund(r, income, 0).is_err());
    v.link_refund(income, o, 0).unwrap();
    assert!(v.link_refund(r, income, 0).is_err());
    v.link_refund(r, o, 0).unwrap();
    v.delete_manual_booking(o).unwrap();
    assert_eq!(v.refund_links(r).unwrap().open_cents, 5000);
    assert!(v.refund_links(r).unwrap().originals.is_empty());
    assert_eq!(page(&v, None).income_cents, 7000);
    assert!(v.link_refund(r, o, 0).is_err());
}

#[test]
fn qa_cross_currency_never_consumes_eur_capacity() {
    let (_d, v, a) = fixture();
    let usd = v
        .create_offline_account("USD".into(), AccountKind::Checking, "USD".into(), 0)
        .unwrap();
    let r = booking(&v, usd, 10000);
    let o = booking(&v, a, -3000);
    v.link_refund(r, o, 0).unwrap();
    assert_eq!(allocations(&v, r), vec![(o, None)]);
    assert_eq!(v.refund_links(r).unwrap().open_cents, 10000);
    assert_eq!(v.refund_links(o).unwrap().open_cents, 3000);
    assert_eq!(page(&v, None).expense_cents, -3000);
}

#[test]
fn qa_suggestions_ties_priority_duplicates_cap_and_search() {
    let (_d, v, a) = fixture();
    let r = add(&v, a, 10000, "2025-06-30", "QA vendor", "INV-123456");
    let same1 = add(&v, a, -10000, "2025-06-29", "QA vendor", "INV-123456");
    let same2 = add(&v, a, -10000, "2025-06-29", "Other", " ");
    let vendor = add(&v, a, -3000, "2025-06-28", "QA vendor", "");
    let reference = add(&v, a, -2000, "2025-06-30", "Other", "INV123456");
    let got = v.refund_suggestions(r).unwrap();
    assert_eq!(
        got.iter()
            .map(|s| (s.booking.id, s.reason))
            .collect::<Vec<_>>(),
        vec![
            (same1, RefundMatch::SameAmount),
            (same2, RefundMatch::SameAmount),
            (vendor, RefundMatch::SameMerchant)
        ]
    );
    assert_eq!(
        v.refund_candidates(r, "INV123456".into(), 100).unwrap()[0].id,
        reference
    );
    assert!(
        v.refund_candidates(r, "not present".into(), 100)
            .unwrap()
            .is_empty()
    );
}

/// "The last two months" of the same-merchant suggestions are a fixed 62 days (Simon's call).
#[test]
fn qa_vendor_window_is_62_days() {
    let (_d, v, a) = fixture();
    let r = add(&v, a, 10000, "2025-04-30", "QA vendor", "");
    let boundary = add(&v, a, -3000, "2025-02-27", "QA vendor", "");
    let too_old = add(&v, a, -2000, "2025-02-26", "QA vendor", "");
    let ids = v
        .refund_suggestions(r)
        .unwrap()
        .into_iter()
        .map(|s| s.booking.id)
        .collect::<Vec<_>>();
    assert!(ids.contains(&boundary));
    assert!(!ids.contains(&too_old), "{ids:?}");
}

#[test]
fn qa_split_refund_category_views_conserve_partial_income() {
    let (_d, v, a) = fixture();
    let (ca, cb) = categories(&v);
    let r = booking(&v, a, 10000);
    let o = booking(&v, a, -3000);
    split(&v, r, ca, cb, 1);
    v.link_refund(r, o, 0).unwrap();
    assert_eq!(page(&v, None).income_cents, 7000);
    let pa = page(&v, Some(ca));
    let pb = page(&v, Some(cb));
    assert_eq!(
        (
            pa.income_cents + pb.income_cents,
            pa.expense_cents + pb.expense_cents
        ),
        // what the refund pays back counts where the paid-back booking is (no category)
        (7000, 0),
        "A income/expense={}/{}, B={}/{}",
        pa.income_cents,
        pa.expense_cents,
        pb.income_cents,
        pb.expense_cents
    );
}

#[test]
fn qa_split_original_parts_unchanged_and_category_expenses() {
    let (_d, v, a) = fixture();
    let (ca, cb) = categories(&v);
    let o = booking(&v, a, -10000);
    let r = booking(&v, a, 3000);
    split(&v, o, ca, cb, -1);
    v.link_refund(r, o, 1).unwrap();
    assert_eq!(
        v.list_transaction_splits(o)
            .unwrap()
            .iter()
            .map(|s| s.amount_cents)
            .collect::<Vec<_>>(),
        vec![-6000, -4000]
    );
    assert_eq!(page(&v, None).expense_cents, -7000);
    assert_eq!(
        page(&v, Some(ca)).expense_cents + page(&v, Some(cb)).expense_cents,
        -7000,
        "linked split-original categories should net the assigned refund"
    );
    // the refund counts in the category of the part it was linked to
    assert_eq!(page(&v, Some(ca)).expense_cents, -3000);
    assert_eq!(page(&v, Some(cb)).expense_cents, -4000);
}

#[test]
fn qa_split_part_ids_are_a_separate_namespace_not_supported_targets() {
    let (_d, v, a) = fixture();
    let (ca, cb) = categories(&v);
    for _ in 0..3 {
        booking(&v, a, -1);
    }
    let o = booking(&v, a, -10000);
    let r = booking(&v, a, 3000);
    split(&v, o, ca, cb, -1);
    let part = v
        .list_transaction_splits(o)
        .unwrap()
        .into_iter()
        .find(|s| s.id != o && s.id != r)
        .unwrap();
    v.link_refund(r, part.id, 0).unwrap();
    assert_eq!(v.refund_links(o).unwrap().refunded_cents, None);
    assert_eq!(v.refund_links(part.id).unwrap().refunded_cents, Some(1));
    v.unlink_refund(r, part.id, 0).unwrap();
    // a split booking is linked by one of its parts
    assert!(v.link_refund(r, o, 0).is_err());
    v.link_refund(r, o, 1).unwrap();
    assert_eq!(v.refund_links(o).unwrap().refunded_cents, Some(3000));
}

fn api(v: &Vault, method: &str, params: serde_json::Value) -> serde_json::Value {
    serde_json::from_str(
        &v.local_api_request(
            &serde_json::json!({"v":1,"method":method,"params":params}).to_string(),
        ),
    )
    .unwrap()
}

#[test]
fn qa_split_refund_insights_cashflow_api_and_category_sums() {
    let (_d, v, a) = fixture();
    let (ca, cb) = categories(&v);
    let r = booking(&v, a, 10000);
    let o = booking(&v, a, -3000);
    split(&v, r, ca, cb, 1);
    v.link_refund(r, o, 0).unwrap();
    let f = v.selection_flow(vec![a], None, None).unwrap();
    assert_eq!((f.income_cents, f.consumption_cents), (7000, 0));
    let c = v
        .cashflow_by_period(PeriodGranularity::Month, 1, Some(vec![a]), false)
        .unwrap();
    assert_eq!((c[0].income_cents, c[0].expense_cents), (7000, 0));
    // the link to the whole of the split refund takes from its last part first
    let cats = v.list_categories(None, None).unwrap();
    assert_eq!(
        cats.iter().find(|c| c.id == ca).unwrap().own_total_cents,
        6000
    );
    assert_eq!(
        cats.iter().find(|c| c.id == cb).unwrap().own_total_cents,
        1000
    );
    v.set_local_api_scope(crate::local_api::ShareScope {
        enabled: true,
        account_ids: vec![a],
        ..Default::default()
    })
    .unwrap();
    let today = crate::dates::today_iso();
    let s = api(
        &v,
        "summary",
        serde_json::json!({"date_from":today,"date_to":today}),
    );
    assert_eq!(s["result"]["totals"][0]["income_cents"], 7000, "{s}");
    assert_eq!(s["result"]["totals"][0]["expense_cents"], 0, "{s}");
    println!("split refund API/category evidence: {s}");
    let rejected = api(
        &v,
        "link_refund",
        serde_json::json!({"refund_id":r,"original_id":o}),
    );
    assert!(
        rejected.get("error").is_some(),
        "local API is read-only: {rejected}"
    );
}

#[test]
fn qa_merge_conflicting_order_preserves_capacity_and_converges() {
    let (d, a, account) = fixture();
    let r = booking(&a, account, 10000);
    let x = booking(&a, account, -6000);
    let y = booking(&a, account, -6000);
    let b = Vault::open(d.join("peer").to_string_lossy().into(), vec![7; 32]).unwrap();
    b.merge_other_vault(a.file_path(), vec![7; 32]).unwrap();
    a.link_refund(r, x, 0).unwrap();
    a.link_refund(r, y, 0).unwrap();
    let list = page(&b, None).transactions;
    let br = list.iter().find(|t| t.amount_cents == 10000).unwrap().id;
    let mut originals = list
        .iter()
        .filter(|t| t.amount_cents == -6000)
        .map(|t| t.id)
        .collect::<Vec<_>>();
    originals.sort_unstable();
    b.link_refund(br, originals[1], 0).unwrap();
    b.link_refund(br, originals[0], 0).unwrap();
    a.merge_other_vault(b.file_path(), vec![7; 32]).unwrap();
    b.merge_other_vault(a.file_path(), vec![7; 32]).unwrap();
    let amounts = |v: &Vault, id| {
        allocations(v, id)
            .into_iter()
            .map(|(_, n)| n.unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(amounts(&a, r), vec![6000, 4000]);
    assert_eq!(amounts(&a, r), amounts(&b, br));
    assert_eq!(a.refund_links(r).unwrap().open_cents, 0);
    let extra = booking(&a, account, -1000);
    assert!(a.link_refund(r, extra, 0).is_err());
}

#[test]
fn qa_wording_consumption_partial_refund_remainder_is_income() {
    let (_d, v, a) = fixture();
    let r = add(
        &v,
        a,
        10000,
        &crate::dates::today_iso(),
        "QA vendor",
        "Rückerstattung Retoure Erstattung",
    );
    let o = booking(&v, a, -3000);
    let before = page(&v, None)
        .transactions
        .into_iter()
        .find(|t| t.id == r)
        .unwrap();
    println!("wording flow before linking: {:?}", before.flow);
    assert_eq!(before.flow, crate::merchants::FlowKind::Income);
    v.link_refund(r, o, 0).unwrap();
    let after = page(&v, None);
    assert_eq!((after.income_cents, after.expense_cents), (7000, 0));
}

#[test]
fn qa_exhausted_source_has_no_unlinkable_suggestions() {
    let (_d, v, a) = fixture();
    let r = booking(&v, a, 10000);
    let o = booking(&v, a, -10000);
    let extra = booking(&v, a, -10000);
    v.link_refund(r, o, 0).unwrap();
    assert!(v.link_refund(r, extra, 0).is_err());
    assert!(
        v.refund_suggestions(r).unwrap().is_empty(),
        "exhausted refund should not suggest another booking"
    );
}

fn totals(v: &Vault, cat: i64) -> (i64, u32) {
    let c = v
        .list_categories(None, None)
        .unwrap()
        .into_iter()
        .find(|c| c.id == cat)
        .unwrap();
    (c.own_total_cents, c.transaction_count)
}

/// What a refund pays back counts in the category of the booking it pays back, the rest in
/// the refund's own category: in the filtered list and in the category totals.
#[test]
fn assigned_refund_counts_in_the_category_of_its_original() {
    let (_d, v, a) = fixture();
    let (ca, cb) = categories(&v);
    let o = booking(&v, a, -3000);
    let r = booking(&v, a, 5000);
    v.set_transaction_category(o, Some(ca)).unwrap();
    v.set_transaction_category(r, Some(cb)).unwrap();
    assert_eq!(page(&v, Some(ca)).expense_cents, -3000);
    assert_eq!(totals(&v, ca).0, -3000);
    assert_eq!(totals(&v, cb).0, 5000);

    v.link_refund(r, o, 0).unwrap();
    let in_a = page(&v, Some(ca));
    assert_eq!((in_a.expense_cents, in_a.income_cents), (0, 0));
    // the refund is listed with the booking it pays back
    let mut ids: Vec<i64> = in_a.transactions.iter().map(|b| b.id).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![o, r]);
    let in_b = page(&v, Some(cb));
    assert_eq!((in_b.expense_cents, in_b.income_cents), (0, 2000));
    let all = page(&v, None);
    assert_eq!((all.expense_cents, all.income_cents), (0, 2000));
    assert_eq!(totals(&v, ca).0, 0);
    assert_eq!(totals(&v, cb).0, 2000);

    v.unlink_refund(r, o, 0).unwrap();
    assert_eq!(page(&v, Some(ca)).expense_cents, -3000);
    assert_eq!(page(&v, Some(cb)).income_cents, 5000);
    assert_eq!(totals(&v, ca), (-3000, 1));
    assert_eq!(totals(&v, cb), (5000, 1));
}

/// Two refunds on different parts of one split booking each count in their part's category;
/// a refund without a category counts there too.
#[test]
fn assigned_refunds_count_in_the_categories_of_their_parts() {
    let (_d, v, a) = fixture();
    let (ca, cb) = categories(&v);
    let o = booking(&v, a, -10000);
    let first = booking(&v, a, 1000);
    let second = booking(&v, a, 4000);
    split(&v, o, ca, cb, -1);
    v.link_refund(first, o, 1).unwrap();
    v.link_refund(second, o, 2).unwrap();
    assert_eq!(page(&v, Some(ca)).expense_cents, -5000);
    assert_eq!(page(&v, Some(cb)).expense_cents, 0);
    assert_eq!(totals(&v, ca).0, -5000);
    assert_eq!(totals(&v, cb).0, 0);
    let none = v
        .list_transactions(BookingFilter {
            uncategorized_only: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!((none.expense_cents, none.income_cents), (0, 0));
    let all = page(&v, None);
    assert_eq!((all.expense_cents, all.income_cents), (-5000, 0));
}

fn expenses(v: &Vault, accounts: Vec<i64>, from: Option<&str>, to: Option<&str>, cat: i64) -> i64 {
    v.list_category_expenses(from.map(Into::into), to.map(Into::into), accounts)
        .unwrap()
        .into_iter()
        .find(|c| c.id == cat)
        .unwrap()
        .own_total_cents
}

/// The chart of the expenses: what a refund pays back lowers the expenses of the paid-back
/// booking's category; the rest of the refund is income and no expense anywhere.
#[test]
fn category_expenses_are_less_what_refunds_pay_back() {
    let (_d, v, a) = fixture();
    let (ca, cb) = categories(&v);
    let o = booking(&v, a, -3000);
    let r = booking(&v, a, 1000);
    let unlinked = booking(&v, a, 700);
    v.set_transaction_category(o, Some(ca)).unwrap();
    v.set_transaction_category(r, Some(cb)).unwrap();
    // an incoming booking without a link lowers no expenses, like before
    v.set_transaction_category(unlinked, Some(ca)).unwrap();
    assert_eq!(expenses(&v, vec![], None, None, ca), -3000);

    v.link_refund(r, o, 0).unwrap();
    assert_eq!(expenses(&v, vec![], None, None, ca), -2000);
    assert_eq!(expenses(&v, vec![], None, None, cb), 0);

    v.unlink_refund(r, o, 0).unwrap();
    assert_eq!(expenses(&v, vec![], None, None, ca), -3000);
}

/// A refund in the category of the booking it pays back (the usual case): the sum of all
/// bookings in the category stays, its expenses go down.
#[test]
fn category_expenses_go_down_when_refund_and_booking_share_a_category() {
    let (_d, v, a) = fixture();
    let (ca, _) = categories(&v);
    let o = booking(&v, a, -3000);
    let r = booking(&v, a, 5000);
    v.set_transaction_category(o, Some(ca)).unwrap();
    v.set_transaction_category(r, Some(ca)).unwrap();
    let all = totals(&v, ca).0;
    v.link_refund(r, o, 0).unwrap();
    assert_eq!(totals(&v, ca).0, all);
    // only what the refund pays back counts: its rest of 20,00 is income
    assert_eq!(expenses(&v, vec![], None, None, ca), 0);
}

/// A refund on one part of a split booking lowers the expenses of that part's category only.
#[test]
fn category_expenses_of_a_split_booking_go_down_in_the_paid_back_part() {
    let (_d, v, a) = fixture();
    let (ca, cb) = categories(&v);
    let o = booking(&v, a, -10000);
    let r = booking(&v, a, 1000);
    split(&v, o, ca, cb, -1);
    v.link_refund(r, o, 1).unwrap();
    assert_eq!(expenses(&v, vec![], None, None, ca), -5000);
    assert_eq!(expenses(&v, vec![], None, None, cb), -4000);
}

/// What a refund pays back counts where the refund is: on its account and at its date.
#[test]
fn category_expenses_count_a_refund_on_its_account_and_date() {
    let (_d, v, a) = fixture();
    let other = v
        .create_offline_account("QA 2".into(), AccountKind::Checking, "EUR".into(), 0)
        .unwrap();
    let (ca, _) = categories(&v);
    let o = add(&v, a, -3000, "2026-03-10", "QA vendor", "");
    let r = add(&v, other, 1000, "2026-04-02", "QA vendor", "");
    v.set_transaction_category(o, Some(ca)).unwrap();
    v.link_refund(r, o, 0).unwrap();
    assert_eq!(expenses(&v, vec![], None, None, ca), -2000);
    assert_eq!(expenses(&v, vec![a], None, None, ca), -3000);
    assert_eq!(expenses(&v, vec![a, other], None, None, ca), -2000);
    let march = (Some("2026-03-01"), Some("2026-03-31"));
    assert_eq!(expenses(&v, vec![], march.0, march.1, ca), -3000);
    assert_eq!(
        expenses(&v, vec![], Some("2026-04-01"), Some("2026-04-30"), ca),
        1000
    );
}

/// The sidebar's sums under the list's "Ausgaben" / "Einnahmen" filter keep following the list
/// (#621): a category's sum is what the rows the list shows for it add up to.
#[test]
fn category_sums_under_a_direction_follow_the_list() {
    let (_d, v, a) = fixture();
    let (ca, _) = categories(&v);
    let o = booking(&v, a, -3000);
    let r = booking(&v, a, 1000);
    v.set_transaction_category(o, Some(ca)).unwrap();
    v.set_transaction_category(r, Some(ca)).unwrap();
    v.link_refund(r, o, 0).unwrap();
    for (direction, sum) in [
        (BookingDirection::All, -2000),
        (BookingDirection::Expense, -3000),
        (BookingDirection::Income, 1000),
    ] {
        let listed = v
            .list_transactions(BookingFilter {
                category_id: Some(ca),
                direction,
                ..Default::default()
            })
            .unwrap();
        let sidebar = v
            .list_categories_filtered(None, None, Vec::new(), None, direction)
            .unwrap()
            .into_iter()
            .find(|c| c.id == ca)
            .unwrap()
            .own_total_cents;
        assert_eq!(
            (sidebar, listed.expense_cents + listed.income_cents),
            (sum, sum),
            "{direction:?}"
        );
    }
}

fn part(category: i64, amount: i64) -> TransactionSplitInput {
    TransactionSplitInput {
        category_id: Some(category),
        amount_cents: amount,
        note: String::new(),
    }
}
fn refunds_of(v: &Vault, original: i64) -> Vec<(i64, u32, Option<i64>)> {
    v.refund_links(original)
        .unwrap()
        .refunds
        .into_iter()
        .map(|r| (r.id, r.part, r.share_cents))
        .collect()
}
fn all_expenses<const N: usize>(v: &Vault, categories: [i64; N]) -> [i64; N] {
    categories.map(|c| expenses(v, vec![], None, None, c))
}

/// Removing the split of a booking turns the links to its parts into links to the whole
/// booking: each refund is filled up to what the booking has left, not to its old part.
#[test]
fn removing_the_split_links_its_refunds_to_the_whole_booking() {
    let (_d, v, a) = fixture();
    let (ca, cb) = categories(&v);
    let o = booking(&v, a, -10000);
    let first = booking(&v, a, 3000);
    let second = booking(&v, a, 5000);
    split(&v, o, ca, cb, -1);
    v.link_refund(first, o, 2).unwrap();
    v.link_refund(second, o, 2).unwrap();
    // the part of 40,00 takes 30,00 and then 10,00
    assert_eq!(
        refunds_of(&v, o),
        vec![(first, 2, Some(3000)), (second, 2, Some(1000))]
    );

    v.set_transaction_splits(o, Vec::new()).unwrap();
    v.set_transaction_category(o, Some(ca)).unwrap();
    assert_eq!(
        refunds_of(&v, o),
        vec![(first, 0, Some(3000)), (second, 0, Some(5000))]
    );
    assert_eq!(allocations(&v, second), vec![(o, Some(5000))]);
    assert_eq!(v.refund_links(o).unwrap().refunded_cents, Some(8000));
    assert_eq!(all_expenses(&v, [ca, cb]), [-2000, 0]);
    // the link is one to the whole booking now: it is removed as such
    v.unlink_refund(second, o, 0).unwrap();
    assert_eq!(refunds_of(&v, o), vec![(first, 0, Some(3000))]);
}

/// A booking that is split differently spreads what its refunds pay back from its last part
/// to its first, each part filled until the refund runs out.
#[test]
fn changed_split_spreads_its_refunds_from_the_last_part_to_the_first() {
    let (_d, v, a) = fixture();
    let (ca, cb) = categories(&v);
    let cc = v
        .create_category("QA C".into(), None, "tag".into(), "#abcdef".into())
        .unwrap();
    let o = booking(&v, a, -10000);
    let r = booking(&v, a, 6000);
    let later = booking(&v, a, 2000);
    split(&v, o, ca, cb, -1);
    v.link_refund(r, o, 1).unwrap();
    assert_eq!(all_expenses(&v, [ca, cb, cc]), [0, -4000, 0]);

    v.set_transaction_splits(o, vec![part(ca, -5000), part(cb, -3000), part(cc, -2000)])
        .unwrap();
    assert_eq!(refunds_of(&v, o), vec![(r, 0, Some(6000))]);
    // 20,00 for the third part, 30,00 for the second, the last 10,00 for the first
    assert_eq!(all_expenses(&v, [ca, cb, cc]), [-4000, 0, 0]);
    // the parts the refund fills are used up for a further one
    let open: Vec<(u32, i64)> = v
        .refund_candidates(later, String::new(), 20)
        .unwrap()
        .into_iter()
        .filter(|b| b.id == o)
        .map(|b| (b.part, b.open_cents))
        .collect();
    assert_eq!(open, vec![(1, 4000), (2, 0), (3, 0)]);
    v.link_refund(later, o, 1).unwrap();
    assert_eq!(all_expenses(&v, [ca, cb, cc]), [-2000, 0, 0]);
    assert_eq!(page(&v, None).expense_cents, -2000);
}

/// A booking linked as a whole and split afterwards spreads the refund the same way.
#[test]
fn splitting_a_linked_booking_spreads_its_refund_from_the_last_part() {
    let (_d, v, a) = fixture();
    let (ca, cb) = categories(&v);
    let o = booking(&v, a, -10000);
    let r = booking(&v, a, 7000);
    v.set_transaction_category(o, Some(ca)).unwrap();
    v.link_refund(r, o, 0).unwrap();
    assert_eq!(all_expenses(&v, [ca, cb]), [-3000, 0]);

    split(&v, o, ca, cb, -1);
    assert_eq!(refunds_of(&v, o), vec![(r, 0, Some(7000))]);
    // 40,00 for the second part, 30,00 for the first
    assert_eq!(all_expenses(&v, [ca, cb]), [-3000, 0]);
    assert_eq!(page(&v, Some(cb)).expense_cents, 0);
    assert_eq!(page(&v, Some(ca)).expense_cents, -3000);
}

/// Another category or note for a part is no other split: the refund stays with its part.
#[test]
fn new_category_of_a_part_keeps_its_refund() {
    let (_d, v, a) = fixture();
    let (ca, cb) = categories(&v);
    let cc = v
        .create_category("QA C".into(), None, "tag".into(), "#abcdef".into())
        .unwrap();
    let o = booking(&v, a, -10000);
    let r = booking(&v, a, 1000);
    split(&v, o, ca, cb, -1);
    v.link_refund(r, o, 1).unwrap();

    v.set_transaction_splits(o, vec![part(cc, -6000), part(cb, -4000)])
        .unwrap();
    assert_eq!(refunds_of(&v, o), vec![(r, 1, Some(1000))]);
    assert_eq!(all_expenses(&v, [ca, cb, cc]), [0, -4000, -5000]);
}

fn third_category(v: &Vault) -> i64 {
    v.create_category("QA C".into(), None, "tag".into(), "#abcdef".into())
        .unwrap()
}
fn sums(p: &BookingPage) -> (i64, i64) {
    (p.income_cents, p.expense_cents)
}

/// A split income is linked by the part of it that is the refund: only that part pays back,
/// the other parts stay what they are.
#[test]
fn part_of_a_split_income_is_the_refund() {
    let (_d, v, a) = fixture();
    let (salary, refund) = categories(&v);
    let shop = third_category(&v);
    let income = booking(&v, a, 32000);
    let o = booking(&v, a, -5000);
    let other = booking(&v, a, -1000);
    v.set_transaction_category(o, Some(shop)).unwrap();
    v.set_transaction_splits(income, vec![part(salary, 30000), part(refund, 2000)])
        .unwrap();
    // the outgoing booking is offered the parts of the income, not the income as a whole;
    // a front end without a choice of the part gets the income as a whole
    assert_eq!(
        v.refund_candidates(o, String::new(), 20)
            .unwrap()
            .into_iter()
            .filter(|b| b.id == income)
            .map(|b| (b.refund_part, b.amount_cents))
            .collect::<Vec<_>>(),
        vec![(0, 32000)]
    );
    let offered: Vec<(u32, i64, i64)> = v
        .refund_part_candidates(o, false, String::new(), 20)
        .unwrap()
        .into_iter()
        .filter(|b| b.id == income)
        .map(|b| (b.refund_part, b.amount_cents, b.open_cents))
        .collect();
    assert_eq!(offered, vec![(1, 30000, 30000), (2, 2000, 2000)]);

    v.link_refund_parts(income, 2, o, 0).unwrap();
    let links = v.refund_links(income).unwrap();
    assert_eq!(
        links
            .originals
            .iter()
            .map(|b| (b.id, b.part, b.refund_part, b.share_cents))
            .collect::<Vec<_>>(),
        vec![(o, 0, 2, Some(2000))]
    );
    assert_eq!(links.originals[0].refund_part_label, "QA B");
    assert_eq!(
        (links.assigned_cents, links.open_cents),
        (Some(2000), 30000)
    );
    let back = v.refund_links(o).unwrap();
    assert_eq!(
        back.refunds
            .iter()
            .map(|r| (r.id, r.refund_part, r.amount_cents, r.share_cents))
            .collect::<Vec<_>>(),
        vec![(income, 2, 2000, Some(2000))]
    );
    assert_eq!(back.refunded_cents, Some(2000));
    // the salary part stays whole, the refund part is used up, the expense goes down
    assert_eq!(
        [salary, refund, shop].map(|c| totals(&v, c).0),
        [30000, 0, -3000]
    );
    assert_eq!(expenses(&v, vec![], None, None, shop), -3000);
    assert_eq!(sums(&page(&v, None)), (30000, -4000));
    assert_eq!(sums(&page(&v, Some(salary))), (30000, 0));
    assert_eq!(sums(&page(&v, Some(refund))), (0, 0));
    assert_eq!(sums(&page(&v, Some(shop))), (0, -3000));
    // the part is used up; the other part and the whole income still take a booking
    assert!(v.link_refund_parts(income, 2, other, 0).is_err());
    v.link_refund_parts(income, 1, other, 0).unwrap();
    assert_eq!(
        allocations(&v, income),
        vec![(o, Some(2000)), (other, Some(1000))]
    );
    // the earlier call removes the links of every part of the refund to that booking
    v.unlink_refund(income, other, 0).unwrap();
    v.unlink_refund_parts(income, 1, o, 0).unwrap();
    assert_eq!(allocations(&v, income), vec![(o, Some(2000))]);
    v.unlink_refund_parts(income, 2, o, 0).unwrap();
    assert!(allocations(&v, income).is_empty());
    assert_eq!(sums(&page(&v, None)), (32000, -6000));
}

/// Only a part on the income's side can be the refund, and a booking without parts has none.
#[test]
fn refund_part_must_be_an_incoming_part_of_the_income() {
    let (_d, v, a) = fixture();
    let (ca, cb) = categories(&v);
    let income = booking(&v, a, 10000);
    let plain = booking(&v, a, 4000);
    let o = booking(&v, a, -5000);
    v.set_transaction_splits(income, vec![part(ca, 12000), part(cb, -2000)])
        .unwrap();
    assert!(v.link_refund_parts(income, 2, o, 0).is_err());
    assert!(v.link_refund_parts(income, 3, o, 0).is_err());
    assert!(v.link_refund_parts(plain, 1, o, 0).is_err());
    // as a whole a split income is still linked (front ends without a choice of the part)
    v.link_refund(income, o, 0).unwrap();
    assert_eq!(allocations(&v, income), vec![(o, Some(5000))]);
    assert_eq!([ca, cb].map(|c| totals(&v, c).0), [7000, -2000]);
}

/// A part of a split income that matches the amount of a booking is suggested for it.
#[test]
fn part_of_a_split_income_is_suggested_by_its_amount() {
    let (_d, v, a) = fixture();
    let (ca, cb) = categories(&v);
    let o = add(&v, a, -2000, "2025-03-01", "Shop", "");
    let income = add(&v, a, 32000, "2025-03-20", "Employer", "");
    v.set_transaction_splits(income, vec![part(ca, 30000), part(cb, 2000)])
        .unwrap();
    let suggested: Vec<(i64, u32, RefundMatch)> = v
        .refund_part_suggestions(o, false)
        .unwrap()
        .into_iter()
        .map(|s| (s.booking.id, s.booking.refund_part, s.reason))
        .collect();
    assert_eq!(suggested, vec![(income, 2, RefundMatch::SameAmount)]);
    v.link_refund_parts(income, 2, o, 0).unwrap();
    assert!(v.refund_part_suggestions(o, false).unwrap().is_empty());
}

/// An income that is split differently or not at all any more keeps its links as links of the
/// whole income, like an outgoing booking.
#[test]
fn changed_split_of_an_income_links_it_as_a_whole() {
    let (_d, v, a) = fixture();
    let (ca, cb) = categories(&v);
    let income = booking(&v, a, 32000);
    let o = booking(&v, a, -5000);
    v.set_transaction_splits(income, vec![part(ca, 30000), part(cb, 2000)])
        .unwrap();
    v.link_refund_parts(income, 2, o, 0).unwrap();
    let own = |v: &Vault| -> Vec<(u32, Option<i64>)> {
        v.refund_links(income)
            .unwrap()
            .originals
            .iter()
            .map(|b| (b.refund_part, b.share_cents))
            .collect()
    };
    assert_eq!(own(&v), vec![(2, Some(2000))]);
    // another category for a part is no other split
    v.set_transaction_splits(income, vec![part(cb, 30000), part(ca, 2000)])
        .unwrap();
    assert_eq!(own(&v), vec![(2, Some(2000))]);
    // other amounts: the whole income pays back, taken from its last part first
    v.set_transaction_splits(income, vec![part(ca, 29000), part(cb, 3000)])
        .unwrap();
    assert_eq!(own(&v), vec![(0, Some(5000))]);
    assert_eq!([ca, cb].map(|c| totals(&v, c).0), [27000, 0]);
    v.set_transaction_splits(income, Vec::new()).unwrap();
    assert_eq!(own(&v), vec![(0, Some(5000))]);
}

/// The parts of a split refund keep every cent of what its links leave (no shares by
/// proportion that round down).
#[test]
fn split_refund_loses_no_cent_in_the_category_sums() {
    let (_d, v, a) = fixture();
    let (ca, cb) = categories(&v);
    let cc = third_category(&v);
    let o = booking(&v, a, -3000);
    let r = booking(&v, a, 10001);
    v.set_transaction_splits(r, vec![part(ca, 3334), part(cb, 3334), part(cc, 3333)])
        .unwrap();
    v.link_refund(r, o, 0).unwrap();
    assert_eq!([ca, cb, cc].map(|c| totals(&v, c).0), [3334, 3334, 333]);
    assert_eq!(
        [ca, cb, cc].map(|c| page(&v, Some(c)).income_cents),
        [3334, 3334, 333]
    );
    assert_eq!(sums(&page(&v, None)), (7001, 0));
}

/// A refund linked to a booking of another currency assigns nothing (#569): it stays income.
#[test]
fn refund_linked_across_currencies_stays_income() {
    let (_d, v, a) = fixture();
    let usd = v
        .create_offline_account("QA USD".into(), AccountKind::Checking, "USD".into(), 0)
        .unwrap();
    let o = booking(&v, a, -3000);
    let r = booking(&v, usd, 10000);
    let own = |v: &Vault| {
        sums(
            &v.list_transactions(BookingFilter {
                account_ids: vec![usd],
                ..Default::default()
            })
            .unwrap(),
        )
    };
    assert_eq!(own(&v), (10000, 0));
    v.link_refund(r, o, 0).unwrap();
    assert_eq!(allocations(&v, r), vec![(o, None)]);
    assert_eq!(own(&v), (10000, 0));
}

/// A linked booking that changes its side (an expense turned into an income) is no original
/// any more: its links are removed.
#[test]
fn booking_that_changes_its_side_loses_its_links() {
    let (_d, v, a) = fixture();
    let (ca, _) = categories(&v);
    let o = booking(&v, a, -3000);
    let r = booking(&v, a, 1000);
    v.set_transaction_category(o, Some(ca)).unwrap();
    v.link_refund(r, o, 0).unwrap();
    assert_eq!(sums(&page(&v, None)), (0, -2000));
    let edit = |amount: i64| {
        v.save_manual_booking(
            a,
            Some(o),
            ManualBookingSetup {
                booking_date: crate::dates::today_iso(),
                amount_cents: amount,
                counterparty: "QA vendor".into(),
                purpose: String::new(),
                category_id: Some(ca),
                note: String::new(),
            },
        )
        .unwrap();
    };
    // another amount on the same side keeps the link
    edit(-2500);
    assert_eq!(allocations(&v, r), vec![(o, Some(1000))]);
    edit(3000);
    assert!(allocations(&v, r).is_empty());
    assert!(v.refund_links(o).unwrap().refunds.is_empty());
    assert_eq!(sums(&page(&v, None)), (4000, 0));
    assert_eq!(expenses(&v, vec![], None, None, ca), 0);
    let row = page(&v, None)
        .transactions
        .into_iter()
        .find(|t| t.id == r)
        .unwrap();
    assert_eq!(row.refund_link, RefundLinkKind::None);
}

/// A split refund with an odd amount: what stays income adds up to the same cents in the
/// category views as in the unfiltered figures.
#[test]
fn odd_cent_split_refund_keeps_every_cent_in_category_views() {
    let (_d, v, a) = fixture();
    let (ca, cb) = categories(&v);
    let r = booking(&v, a, 10001);
    let o = booking(&v, a, -3000);
    v.set_transaction_splits(
        r,
        vec![
            TransactionSplitInput {
                category_id: Some(ca),
                amount_cents: 6000,
                note: "A".into(),
            },
            TransactionSplitInput {
                category_id: Some(cb),
                amount_cents: 4001,
                note: "B".into(),
            },
        ],
    )
    .unwrap();
    v.link_refund(r, o, 0).unwrap();
    assert_eq!(page(&v, None).income_cents, 7001);
    assert_eq!(
        page(&v, Some(ca)).income_cents + page(&v, Some(cb)).income_cents,
        7001
    );
    let cats = v.list_categories(None, None).unwrap();
    let own = |id: i64| cats.iter().find(|c| c.id == id).unwrap().own_total_cents;
    assert_eq!(own(ca) + own(cb), 7001);
}

/// A search also finds bookings on the other side of the date: an expense after the refund,
/// a refund before the expense. Without a search and in the suggestions they stay out.
#[test]
fn search_finds_bookings_on_either_side_of_the_date() {
    let (_d, v, a) = fixture();
    let r = add(&v, a, 5000, "2025-04-10", "QA vendor", "");
    let later = add(&v, a, -5000, "2025-04-20", "QA vendor", "");
    let ids = |id: i64, q: &str| -> Vec<i64> {
        v.refund_candidates(id, q.into(), 100)
            .unwrap()
            .into_iter()
            .map(|b| b.id)
            .collect()
    };
    assert!(ids(r, "").is_empty());
    assert!(v.refund_suggestions(r).unwrap().is_empty());
    assert_eq!(ids(r, "qa vendor"), vec![later]);
    assert!(ids(later, "").is_empty());
    assert_eq!(ids(later, "qa vendor"), vec![r]);
    v.link_refund(r, later, 0).unwrap();
    assert_eq!(v.refund_links(later).unwrap().refunded_cents, Some(5000));
}

/// A split refund with a part of the other sign (a fee taken off the refund), linked as a
/// whole: the list of every category adds up to what the sidebar shows for it, and the
/// categories add up to the whole.
#[test]
fn split_refund_with_a_fee_part_counts_alike_in_list_and_sidebar() {
    let (_d, v, a) = fixture();
    let (ca, cb) = categories(&v);
    let cc = third_category(&v);
    let r = booking(&v, a, 10000);
    let o = booking(&v, a, -10000);
    v.set_transaction_category(o, Some(cc)).unwrap();
    v.set_transaction_splits(r, vec![part(ca, 15000), part(cb, -5000)])
        .unwrap();
    let check = |v: &Vault, sidebar: [i64; 3]| {
        assert_eq!([ca, cb, cc].map(|c| totals(v, c).0), sidebar);
        let listed = [ca, cb, cc].map(|c| {
            let p = page(v, Some(c));
            p.income_cents + p.expense_cents
        });
        assert_eq!(listed, sidebar);
        let all = page(v, None);
        assert_eq!(
            all.income_cents + all.expense_cents,
            sidebar.iter().sum::<i64>()
        );
    };
    check(&v, [15000, -5000, -10000]);
    // the refund pays back from its incoming part; the fee stays where it is
    v.link_refund(r, o, 0).unwrap();
    check(&v, [5000, -5000, 0]);
    v.unlink_refund(r, o, 0).unwrap();
    v.link_refund_parts(r, 1, o, 0).unwrap();
    check(&v, [5000, -5000, 0]);
}

fn kinds(v: &Vault, ids: &[i64]) -> Vec<RefundLinkKind> {
    let rows = page(v, None).transactions;
    ids.iter()
        .map(|id| {
            rows.iter()
                .find(|t| t.id == *id)
                .unwrap()
                .refund_link
                .clone()
        })
        .collect()
}

/// An incoming part of a purchase (store credit that paid a part of it) is a refund: it pays
/// back the booking the credit was given for.
#[test]
fn store_credit_part_of_a_purchase_pays_back_a_booking() {
    let (_d, v, a) = fixture();
    let (shop, credit) = categories(&v);
    let returned = add(&v, a, -5000, "2026-01-10", "Shop", "");
    let purchase = add(&v, a, -5000, "2026-02-20", "Shop", "");
    v.set_transaction_category(returned, Some(shop)).unwrap();
    v.set_transaction_splits(purchase, vec![part(shop, -10000), part(credit, 5000)])
        .unwrap();
    assert_eq!([shop, credit].map(|c| totals(&v, c).0), [-15000, 5000]);
    assert_eq!(sums(&page(&v, None)), (0, -10000));

    // the returned booking is offered the credit part, the purchase the returned booking
    let offered: Vec<(i64, u32, i64)> = v
        .refund_part_candidates(returned, false, String::new(), 20)
        .unwrap()
        .into_iter()
        .map(|b| (b.id, b.refund_part, b.amount_cents))
        .collect();
    assert_eq!(offered, vec![(purchase, 2, 5000)]);
    assert_eq!(
        v.refund_part_candidates(purchase, true, String::new(), 20)
            .unwrap()
            .into_iter()
            .map(|b| (b.id, b.part))
            .collect::<Vec<_>>(),
        vec![(returned, 0)]
    );
    // without a choice of the part an outgoing booking pays nothing back
    assert!(
        v.refund_candidates(returned, String::new(), 20)
            .unwrap()
            .is_empty()
    );
    assert!(v.link_refund(purchase, returned, 0).is_err());
    assert!(v.link_refund_parts(purchase, 1, returned, 0).is_err());
    assert!(v.link_refund_parts(purchase, 2, purchase, 1).is_err());

    v.link_refund_parts(purchase, 2, returned, 0).unwrap();
    let links = v.refund_links(purchase).unwrap();
    assert_eq!(
        links
            .originals
            .iter()
            .map(|b| (b.id, b.refund_part, b.share_cents))
            .collect::<Vec<_>>(),
        vec![(returned, 2, Some(5000))]
    );
    assert_eq!(
        (
            links.refund_total_cents,
            links.refund_open_cents,
            links.original_total_cents
        ),
        (5000, 0, 10000)
    );
    assert_eq!(
        v.refund_links(returned)
            .unwrap()
            .refunds
            .iter()
            .map(|b| (b.id, b.refund_part, b.amount_cents, b.share_cents))
            .collect::<Vec<_>>(),
        vec![(purchase, 2, 5000, Some(5000))]
    );
    assert_eq!(
        kinds(&v, &[purchase, returned]),
        vec![RefundLinkKind::Refund, RefundLinkKind::Original]
    );
    // the credit counts where the returned booking is; the money that left stays the same
    assert_eq!([shop, credit].map(|c| totals(&v, c).0), [-10000, 0]);
    assert_eq!(all_expenses(&v, [shop, credit]), [-10000, 0]);
    assert_eq!(sums(&page(&v, None)), (0, -10000));
    assert_eq!(sums(&page(&v, Some(shop))), (0, -10000));
    assert_eq!(sums(&page(&v, Some(credit))), (0, 0));

    // the outgoing part of the same purchase can be paid back by a later refund
    let later = add(&v, a, 3000, "2026-03-01", "Shop", "");
    assert!(
        v.refund_candidates(later, String::new(), 20)
            .unwrap()
            .iter()
            .any(|b| b.id == purchase && b.part == 1 && b.amount_cents == -10000)
    );
    v.link_refund(later, purchase, 1).unwrap();
    let both = v.refund_links(purchase).unwrap();
    assert_eq!((both.originals.len(), both.refunds.len()), (1, 1));
    assert_eq!(both.original_open_cents, 7000);
    // the credit part itself is no amount that can be paid back
    assert!(v.link_refund(later, purchase, 2).is_err());

    // without its parts the purchase has nothing left that pays back: that link goes
    v.set_transaction_splits(purchase, Vec::new()).unwrap();
    assert!(v.refund_links(returned).unwrap().refunds.is_empty());
    assert_eq!(
        v.refund_links(purchase)
            .unwrap()
            .refunds
            .iter()
            .map(|b| (b.id, b.part, b.share_cents))
            .collect::<Vec<_>>(),
        vec![(later, 0, Some(3000))]
    );
}

/// An outgoing part of an income (a fee taken off it) is offered to a refund and can be paid
/// back; what pays it back is income, not a lower expense.
#[test]
fn fee_part_of_an_income_can_be_paid_back() {
    let (_d, v, a) = fixture();
    let (salary, fees) = categories(&v);
    let other = third_category(&v);
    let income = add(&v, a, 9500, "2026-01-31", "Employer", "");
    let back = add(&v, a, 500, "2026-02-15", "Employer", "");
    v.set_transaction_splits(income, vec![part(salary, 10000), part(fees, -500)])
        .unwrap();
    v.set_transaction_category(back, Some(other)).unwrap();
    assert_eq!(sums(&page(&v, None)), (10000, 0));

    // the selection of the refund lists the outgoing part of the income
    let offered: Vec<(i64, u32, i64, String)> = v
        .refund_candidates(back, String::new(), 20)
        .unwrap()
        .into_iter()
        .map(|b| (b.id, b.part, b.amount_cents, b.part_label))
        .collect();
    assert_eq!(offered, vec![(income, 2, -500, "QA B".to_string())]);
    assert_eq!(
        v.refund_part_suggestions(back, true)
            .unwrap()
            .into_iter()
            .map(|s| (s.booking.id, s.booking.part, s.reason))
            .collect::<Vec<_>>(),
        vec![(income, 2, RefundMatch::SameAmount)]
    );
    assert!(v.link_refund(back, income, 1).is_err());
    v.link_refund(back, income, 2).unwrap();
    assert_eq!(allocations(&v, back), vec![(income, Some(500))]);
    assert_eq!(
        kinds(&v, &[back, income]),
        vec![RefundLinkKind::Refund, RefundLinkKind::Original]
    );
    assert_eq!(v.refund_links(back).unwrap().refund_income_cents, 500);
    assert_eq!(
        [salary, fees, other].map(|c| totals(&v, c).0),
        [10000, 0, 0]
    );
    // income stays income: nothing here was an expense
    assert_eq!(sums(&page(&v, None)), (10000, 0));
    assert_eq!(sums(&page(&v, Some(fees))), (0, 0));
    assert_eq!(sums(&page(&v, Some(salary))), (10000, 0));
    assert_eq!(sums(&page(&v, Some(other))), (0, 0));
    assert_eq!(all_expenses(&v, [salary, fees, other]), [0, 0, 0]);

    v.unlink_refund(back, income, 2).unwrap();
    assert_eq!(
        [salary, fees, other].map(|c| totals(&v, c).0),
        [10000, -500, 500]
    );
    assert_eq!(sums(&page(&v, Some(fees))), (-500, 0));
}

/// Generated scenarios (splits with parts of both sides, links by part and as a whole, other
/// splits, removed links): whatever the links are, the views of the same bookings agree.
#[test]
fn views_agree_for_generated_links_and_splits() {
    struct Rng(u64);
    impl Rng {
        fn next(&mut self, n: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 33) % n
        }
    }
    let figures = |v: &Vault, cats: &[i64]| -> Vec<(i64, i64)> {
        let mut out = vec![sums(&page(v, None))];
        out.extend(cats.iter().map(|c| sums(&page(v, Some(*c)))));
        out
    };
    for seed in 0..40u64 {
        let mut rng = Rng(seed * 7919 + 714);
        let (_d, v, a) = fixture();
        let (ca, cb) = categories(&v);
        let cats = [ca, cb, third_category(&v)];
        let mut ids = Vec::new();
        let mut log = Vec::new();
        for i in 0..6u64 {
            let amount = (rng.next(90) as i64 + 10) * 100 * if i % 2 == 0 { -1 } else { 1 };
            let id = add(
                &v,
                a,
                amount,
                &format!("2026-0{}-1{}", rng.next(6) + 1, i),
                "QA",
                "",
            );
            if rng.next(4) > 0 {
                v.set_transaction_category(id, Some(cats[rng.next(3) as usize]))
                    .unwrap();
            }
            ids.push((id, amount));
        }
        let split = |v: &Vault, rng: &mut Rng, (id, amount): (i64, i64)| -> String {
            // two parts on the booking's side and often one of the other side
            let extra = if rng.next(2) == 0 {
                (rng.next(9) as i64 + 1) * 100
            } else {
                0
            };
            let gross = amount.abs() + extra;
            let first = gross / 3;
            let mut parts = vec![first, gross - first];
            if extra > 0 {
                parts.push(-extra);
            }
            let lines: Vec<TransactionSplitInput> = parts
                .iter()
                .map(|p| TransactionSplitInput {
                    category_id: (rng.next(4) > 0).then(|| cats[rng.next(3) as usize]),
                    amount_cents: p * amount.signum(),
                    note: String::new(),
                })
                .collect();
            v.set_transaction_splits(id, lines).unwrap();
            format!("split {id} {parts:?}")
        };
        for booking in ids.clone() {
            if rng.next(2) == 0 {
                log.push(split(&v, &mut rng, booking));
            }
        }
        let unlinked = figures(&v, &cats);
        let total: i64 = ids.iter().map(|(_, amount)| amount).sum();
        let mut links: Vec<(i64, u32, i64, u32)> = Vec::new();
        for step in 0..14 {
            let (r, o) = (ids[rng.next(6) as usize].0, ids[rng.next(6) as usize].0);
            let (rp, op) = (rng.next(4) as u32, rng.next(4) as u32);
            match rng.next(6) {
                0 => {
                    let booking = ids[rng.next(6) as usize];
                    if rng.next(3) == 0 {
                        v.set_transaction_splits(booking.0, Vec::new()).unwrap();
                        log.push(format!("unsplit {}", booking.0));
                    } else {
                        log.push(split(&v, &mut rng, booking));
                    }
                }
                1 if !links.is_empty() => {
                    let (r, rp, o, op) = links.remove(rng.next(links.len() as u64) as usize);
                    v.unlink_refund_parts(r, rp, o, op).unwrap();
                    log.push(format!("unlink {r}.{rp} -> {o}.{op}"));
                }
                _ => {
                    if v.link_refund_parts(r, rp, o, op).is_ok() {
                        links.push((r, rp, o, op));
                        log.push(format!("link {r}.{rp} -> {o}.{op}"));
                    }
                }
            }
            let context = || format!("seed {seed} step {step}: {log:?}");
            let all = page(&v, None);
            assert_eq!(all.income_cents + all.expense_cents, total, "{}", context());
            let listed: Vec<i64> = cats
                .iter()
                .map(|c| {
                    let p = page(&v, Some(*c));
                    p.income_cents + p.expense_cents
                })
                .collect();
            let sidebar: Vec<i64> = cats.iter().map(|c| totals(&v, *c).0).collect();
            assert_eq!(listed, sidebar, "list and sidebar, {}", context());
            let none = v
                .list_transactions(BookingFilter {
                    uncategorized_only: true,
                    ..Default::default()
                })
                .unwrap();
            assert_eq!(
                listed.iter().sum::<i64>() + none.income_cents + none.expense_cents,
                total,
                "the categories add up to the whole, {}",
                context()
            );
            // every link has the same share from both of its ends, within what its ends are
            for (id, _) in &ids {
                let own = v.refund_links(*id).unwrap();
                for o in &own.originals {
                    let share = o.share_cents.expect("one currency");
                    let back = v.refund_links(o.id).unwrap();
                    // at most what can be paid back of that booking: its outgoing parts
                    assert!(
                        share >= 0 && share <= back.original_total_cents,
                        "{}",
                        context()
                    );
                    let seen = back
                        .refunds
                        .iter()
                        .find(|r| r.id == *id && r.part == o.part && r.refund_part == o.refund_part)
                        .and_then(|r| r.share_cents);
                    assert_eq!(seen, Some(share), "both ends, {}", context());
                }
                assert!(
                    own.assigned_cents.unwrap_or(0) <= own.refund_total_cents,
                    "{}",
                    context()
                );
                assert!(
                    own.refunded_cents.unwrap_or(0) <= own.original_total_cents,
                    "{}",
                    context()
                );
                assert_eq!(
                    own.refund_open_cents,
                    own.refund_total_cents - own.assigned_cents.unwrap_or(0),
                    "{}",
                    context()
                );
            }
        }
        // without the links (the splits as they are now) nothing of them is left in the figures
        let ends: Vec<(i64, u32, i64, u32)> = ids
            .iter()
            .flat_map(|(id, _)| {
                v.refund_links(*id)
                    .unwrap()
                    .originals
                    .into_iter()
                    .map(|o| (*id, o.refund_part, o.id, o.part))
                    .collect::<Vec<_>>()
            })
            .collect();
        for (r, rp, o, op) in ends {
            v.unlink_refund_parts(r, rp, o, op).unwrap();
        }
        let plain = figures(&v, &cats);
        for (id, _) in &ids {
            let own = v.refund_links(*id).unwrap();
            assert!(
                own.originals.is_empty() && own.refunds.is_empty(),
                "seed {seed}"
            );
        }
        assert_eq!(plain[0], sums(&page(&v, None)));
        if log
            .iter()
            .all(|l| l.starts_with("link") || l.starts_with("unlink"))
        {
            assert_eq!(plain, unlinked, "seed {seed}: {log:?}");
        }
    }
}

/// Without a search term a refund is offered the bookings before it and a booking the
/// incomes after it; a search finds them on either side of the date (a refund booked before
/// the purchase it pays back).
#[test]
fn search_finds_bookings_before_and_after() {
    let (_d, v, a) = fixture();
    let early = add(&v, a, 2000, "2025-03-01", "Hotel Nord", "");
    let purchase = add(&v, a, -5000, "2025-03-10", "Hotel Nord", "");
    let late = add(&v, a, 1000, "2025-03-20", "Hotel Nord", "");
    let old = add(&v, a, -700, "2023-01-05", "Hotel Nord", "");
    let ids = |rows: Vec<RefundBooking>| -> Vec<i64> { rows.into_iter().map(|b| b.id).collect() };
    // from the purchase: only the income after it, with a search also the one before
    assert_eq!(
        ids(v.refund_candidates(purchase, String::new(), 20).unwrap()),
        vec![late]
    );
    assert_eq!(
        ids(v.refund_candidates(purchase, "hotel".into(), 20).unwrap()),
        vec![early, late]
    );
    // from the refund booked first: nothing before it, with a search the purchase after it
    // and the booking of years ago, nearest date first
    assert!(
        v.refund_candidates(early, String::new(), 20)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        ids(v.refund_candidates(early, "hotel".into(), 20).unwrap()),
        vec![purchase, old]
    );
    assert_eq!(
        ids(v
            .refund_part_candidates(early, true, "hotel".into(), 20)
            .unwrap()),
        vec![purchase, old]
    );
    // the suggestions keep to the dates
    assert!(v.refund_suggestions(early).unwrap().is_empty());
    v.link_refund(early, purchase, 0).unwrap();
    assert_eq!(allocations(&v, early), vec![(purchase, Some(2000))]);
}
