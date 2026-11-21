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

#[test]
#[ignore = "open: \"two months\" is a fixed 62 days, not calendar months"]
fn qa_vendor_exact_two_calendar_months_boundary() {
    let (_d, v, a) = fixture();
    let r = add(&v, a, 10000, "2025-04-30", "QA vendor", "");
    let boundary = add(&v, a, -3000, "2025-02-28", "QA vendor", "");
    let too_old = add(&v, a, -2000, "2025-02-27", "QA vendor", "");
    let ids = v
        .refund_suggestions(r)
        .unwrap()
        .into_iter()
        .map(|s| s.booking.id)
        .collect::<Vec<_>>();
    assert!(ids.contains(&boundary));
    assert!(
        !ids.contains(&too_old),
        "calendar two-month window must exclude Feb 27: {ids:?}"
    );
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
        (7000, 3000),
        "A income/expense={}/{}, B={}/{}",
        pa.income_cents,
        pa.expense_cents,
        pb.income_cents,
        pb.expense_cents
    );
}

#[test]
#[ignore = "open product call: a refund does not net against the split original's categories"]
fn qa_split_original_parts_unchanged_and_category_expenses() {
    let (_d, v, a) = fixture();
    let (ca, cb) = categories(&v);
    let o = booking(&v, a, -10000);
    let r = booking(&v, a, 3000);
    split(&v, o, ca, cb, -1);
    v.link_refund(r, o, 0).unwrap();
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
    let cats = v.list_categories(None, None).unwrap();
    assert_eq!(
        cats.iter().find(|c| c.id == ca).unwrap().own_total_cents,
        6000
    );
    assert_eq!(
        cats.iter().find(|c| c.id == cb).unwrap().own_total_cents,
        4000
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
