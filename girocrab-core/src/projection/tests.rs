use super::*;
use crate::local_api::ShareScope;
use crate::test_support::TempDir;
use rusqlite::params;
use serde_json::{Value, json};

#[test]
fn round8_refund_rows_keep_each_original_attribution() {
    let f = Fixture::new();
    let a = f.add(-2500, Some(f.cost));
    let b = f.add(-2500, Some(f.cost));
    let r = f.add(5000, Some(f.income));
    f.link(r, "", a, "");
    f.link(r, "", b, "");
    let db = f.v.db().unwrap();
    let selection =
        f.v.projected_selection(&db, &BookingFilter::default(), None, CurrencyMode::All)
            .unwrap();
    let mut rows: Vec<_> = selection
        .rows
        .iter()
        .filter(|row| row.booking_id == r)
        .map(|row| (row.attribution_id, row.amount_cents))
        .collect();
    rows.sort();
    assert_eq!(rows, vec![(a, 2500), (b, 2500)]);
}

#[test]
fn round8_perk_refund_uses_metadata_outside_total_accounts() {
    let f = Fixture::new();
    let eligible =
        f.v.create_offline_account("Eligible".into(), AccountKind::Cash, "EUR".into(), 0)
            .unwrap();
    let perk =
        f.v.create_category("Round up".into(), None, "star".into(), "#ff0000".into())
            .unwrap();
    f.v.db()
        .unwrap()
        .execute(
            "UPDATE categories SET builtin_role='Perks' WHERE id=?1",
            [perk],
        )
        .unwrap();
    let original = f.add(-1000, Some(perk));
    let refund = f.add_on(eligible, 400, Some(f.income), "EUR", &f.date);
    f.v.db().unwrap().execute_batch(&format!("UPDATE accounts SET include_in_total=0 WHERE id={}; UPDATE transactions SET purpose='Round up' WHERE id={original}", f.account)).unwrap();
    f.link(refund, "", original, "");
    let dashboard = f.v.investment_dashboard(1).unwrap();
    assert_eq!(dashboard.roundup_count, 0);
    assert_eq!(dashboard.roundup_invested_cents, -400);
}

#[test]
fn round8_category_and_direction_match_one_contribution() {
    let f = Fixture::new();
    let original = f.add(-1000, Some(f.cost));
    let id = f.add(1000, None);
    let parts = f.split(id, &[(2000, Some(f.income)), (-1000, Some(f.cost))]);
    f.link(id, &parts[0], original, "");
    let filter = BookingFilter {
        category_id: Some(f.income),
        direction: BookingDirection::Expense,
        ..Default::default()
    };
    let page = f.v.list_transactions(filter.clone()).unwrap();
    assert!(page.transactions.is_empty());
    assert_eq!(f.v.count_transactions(filter).unwrap(), 0);
}

#[test]
fn round8_deleted_category_cannot_be_a_traversal_root() {
    let f = Fixture::new();
    f.add(-100, Some(f.cost));
    f.v.db()
        .unwrap()
        .execute("UPDATE categories SET deleted_ms=1 WHERE id=?1", [f.root])
        .unwrap();
    let filter = BookingFilter {
        category_id: Some(f.root),
        include_subcategories: true,
        ..Default::default()
    };
    assert_eq!(
        f.v.analytics_report(filter.clone(), false)
            .unwrap()
            .expense_cents,
        0
    );
    assert_eq!(f.v.count_transactions(filter).unwrap(), 0);
}

#[test]
fn round8_plan_total_excludes_neutral_positive_investment_parts() {
    let f = Fixture::new();
    let invest =
        f.v.create_category("Investment".into(), None, "star".into(), "#ff0000".into())
            .unwrap();
    f.v.db()
        .unwrap()
        .execute(
            "UPDATE categories SET builtin_role='Investitionen' WHERE id=?1",
            [invest],
        )
        .unwrap();
    let buy = f.add(-1000, Some(invest));
    f.split(buy, &[(-2000, Some(invest)), (1000, Some(invest))]);
    f.v.db()
        .unwrap()
        .execute(
            "UPDATE transactions SET purpose='Sparplan execution' WHERE id=?1",
            [buy],
        )
        .unwrap();
    assert_eq!(
        f.v.investment_dashboard(1).unwrap().plans[0].total_invested_cents,
        2000
    );
}

#[test]
fn round7b_realized_plan_total_uses_refund_projection_not_execution_amount() {
    let f = Fixture::new();
    let buy = f.add(-10000, Some(f.cost));
    let refund = f.add(3000, Some(f.income));
    f.v.db().unwrap().execute_batch(&format!("UPDATE categories SET builtin_role='Investitionen' WHERE id={}; UPDATE transactions SET purpose='Sparplan execution' WHERE id={buy}",f.cost)).unwrap();
    assert_eq!(
        f.v.investment_dashboard(1).unwrap().plans[0].total_invested_cents,
        10000
    );
    f.link(refund, "", buy, "");
    let d = f.v.investment_dashboard(1).unwrap();
    let plan = &d.plans[0];
    assert_eq!(plan.total_invested_cents, 7000);
    assert_eq!(plan.monthly_cents, 10000);
    assert_eq!(plan.executions[0].amount_cents, 10000);
    let remaining = f.add(7000, Some(f.income));
    f.link(remaining, "", buy, "");
    assert_eq!(
        f.v.investment_dashboard(1).unwrap().plans[0].total_invested_cents,
        0
    );
}
#[test]
fn round7b_linked_positive_split_keeps_its_unallocated_income() {
    let f = Fixture::new();
    let o = f.add(-300, Some(f.cost));
    let r = f.add(0, None);
    let p = f.split(r, &[(800, Some(f.income)), (-800, Some(f.cost))]);
    f.link(r, &p[0], o, "");
    let report =
        f.v.analytics_report(
            BookingFilter {
                direction: BookingDirection::Income,
                ..Default::default()
            },
            false,
        )
        .unwrap();
    assert_eq!(report.income_cents, 500);
}

#[test]
fn round7b_category_rollup_stops_at_a_deleted_parent() {
    let f = Fixture::new();
    let parent =
        f.v.create_category("Parent".into(), None, "tag".into(), "".into())
            .unwrap();
    f.v.delete_category(parent).unwrap();
    let child =
        f.v.create_category("Child".into(), Some(parent), "tag".into(), "".into())
            .unwrap();
    f.add(-100, Some(child));
    let flows =
        f.v.list_category_expenses(BookingFilter::default())
            .unwrap();
    assert!(
        flows
            .iter()
            .any(|r| r.category_id == Some(child) && r.cents == 100)
    );
    assert!(flows.iter().all(|r| r.category_id != Some(parent)));
}

#[test]
fn round7b_analytics_does_not_narrow_an_unused_investment_total() {
    let f = Fixture::new();
    let invest =
        f.v.create_category("Invest".into(), None, "tag".into(), "".into())
            .unwrap();
    f.v.db()
        .unwrap()
        .execute(
            "UPDATE categories SET builtin_role='Investitionen' WHERE id=?1",
            [invest],
        )
        .unwrap();
    f.add(i64::MAX, Some(invest));
    f.add(100, Some(invest));
    let report =
        f.v.analytics_report(BookingFilter::default(), false)
            .unwrap();
    assert_eq!((report.income_cents, report.expense_cents), (0, 0));
}

#[test]
fn round7_negative_saveback_is_not_roundup() {
    let f = Fixture::new();
    let id = f.add(-100, None);
    f.v.db()
        .unwrap()
        .execute(
            "UPDATE transactions SET counterparty='Saveback',purpose='Saveback' WHERE id=?1",
            [id],
        )
        .unwrap();
    let d = f.v.investment_dashboard(1).unwrap();
    assert_eq!(d.roundup_invested_cents, 0);
}

#[test]
fn tag_counts_follow_projected_income_membership() {
    let f = Fixture::new();
    let o = f.add(-1000, Some(f.cost));
    let r = f.add(1000, Some(f.income));
    f.link(r, "", o, "");
    let tag = f.v.create_tag("Refund".into(), "#ff0000".into()).unwrap();
    f.v.set_booking_tags(r, vec![tag.id]).unwrap();
    let totals =
        f.v.tag_totals(BookingFilter {
            direction: BookingDirection::Income,
            ..Default::default()
        })
        .unwrap();
    assert!(totals.iter().all(|t| t.tag_id != Some(tag.id)));
}

#[test]
fn round7_loan_cashflow_keeps_linked_instalments() {
    let f = Fixture::new();
    let loan =
        f.v.create_offline_account("Loan".into(), AccountKind::Cash, "EUR".into(), 0)
            .unwrap();
    f.v.db()
        .unwrap()
        .execute("UPDATE accounts SET kind='loan' WHERE id=?1", [loan])
        .unwrap();
    let instalment = f.add(-10000, None);
    f.v.db()
        .unwrap()
        .execute(
            "INSERT INTO loan_links(transaction_id,loan_account_id,source) VALUES (?1,?2,'manual')",
            params![instalment, loan],
        )
        .unwrap();
    let flows =
        f.v.cashflow_by_period(PeriodGranularity::Month, 1, Some(vec![loan]), false)
            .unwrap();
    assert_eq!(flows[0].expense_cents, 10000);
}

#[test]
fn round7_untagged_split_does_not_overflow_unused_tag_share() {
    let f = Fixture::new();
    let b = f.add(1, None);
    let invest =
        f.v.create_category("Invest".into(), None, "chart.bar".into(), "#ff0000".into())
            .unwrap();
    f.v.db()
        .unwrap()
        .execute(
            "UPDATE categories SET builtin_role='Investitionen' WHERE id=?1",
            [invest],
        )
        .unwrap();
    let transfer = f.transfer_category();
    for (position, (amount, category)) in [(i64::MAX, f.income), (1, invest), (-i64::MAX, transfer)]
        .into_iter()
        .enumerate()
    {
        f.v.db().unwrap().execute("INSERT INTO transaction_splits(transaction_id,uuid,amount_cents,category_id,position) VALUES (?1,?2,?3,?4,?5)",params![b,uuid::Uuid::new_v4().to_string(),amount,category,position as i64]).unwrap();
    }
    assert!(
        f.v.analytics_report(BookingFilter::default(), false)
            .is_ok()
    );
}

#[test]
fn round7_blank_booking_currency_inherits_account() {
    let f = Fixture::new();
    let usd =
        f.v.create_offline_account("USD".into(), AccountKind::Cash, "USD".into(), 0)
            .unwrap();
    let id = f.add_on(usd, -100, Some(f.cost), "", &f.date);
    let page = f.v.list_transactions(BookingFilter::default()).unwrap();
    assert_eq!(
        page.transactions
            .iter()
            .find(|b| b.id == id)
            .unwrap()
            .currency,
        "USD"
    );
}

#[test]
fn round7_expense_proof_contains_cross_month_refund() {
    let f = Fixture::new();
    let o = f.add_on(f.account, -10000, Some(f.cost), "EUR", "2026-01-01");
    let r = f.add_on(f.account, 4000, Some(f.income), "EUR", "2026-02-01");
    f.link(r, "", o, "");
    let filter = BookingFilter {
        direction: BookingDirection::Expense,
        date_from: Some("2026-02-01".into()),
        ..Default::default()
    };
    let page = f.v.list_transactions(filter.clone()).unwrap();
    assert_eq!(page.expense_cents, 4000);
    assert_eq!(
        page.transactions.iter().map(|b| b.id).collect::<Vec<_>>(),
        vec![r]
    );
    assert_eq!(
        f.v.list_transactions(BookingFilter {
            limit: 6000,
            ..filter
        })
        .unwrap()
        .total_count,
        1
    );
}

#[test]
fn round7_dashboard_roundup_is_refund_adjusted() {
    let f = Fixture::new();
    let perk =
        f.v.create_category("Round up".into(), None, "star".into(), "#ff0000".into())
            .unwrap();
    f.v.db()
        .unwrap()
        .execute(
            "UPDATE categories SET builtin_role='Perks' WHERE id=?1",
            [perk],
        )
        .unwrap();
    let o = f.add(-1000, Some(perk));
    f.v.db()
        .unwrap()
        .execute(
            "UPDATE transactions SET purpose='Round up' WHERE id=?1",
            [o],
        )
        .unwrap();
    let r = f.add(400, Some(f.income));
    f.link(r, "", o, "");
    let dashboard = f.v.investment_dashboard(1).unwrap();
    assert_eq!(dashboard.roundup_count, 1);
    assert_eq!(dashboard.roundup_invested_cents, 600);
    assert_eq!(dashboard.perks_this_month_cents, 600);
}

#[test]
fn round6_expense_direction_keeps_refund_adjustments() {
    let f = Fixture::new();
    let o = f.add(-10000, Some(f.cost));
    let r = f.add(3000, Some(f.income));
    f.link(r, "", o, "");
    for (direction, expense, income) in [
        (BookingDirection::All, 7000, 0),
        (BookingDirection::Expense, 7000, 0),
        (BookingDirection::Income, 0, 0),
    ] {
        let report =
            f.v.analytics_report(
                BookingFilter {
                    direction,
                    ..Default::default()
                },
                false,
            )
            .unwrap();
        assert_eq!(
            (report.expense_cents, report.income_cents),
            (expense, income),
            "{direction:?}"
        );
    }
}

#[test]
fn round6_direction_keeps_the_base_of_a_linked_expense_part() {
    for amount in [0, 10000] {
        let f = Fixture::new();
        let original = f.add(amount, None);
        let parts = f.split(
            original,
            &[(-10000, Some(f.cost)), (10000 + amount, Some(f.income))],
        );
        let refund = f.add(3000, Some(f.income));
        f.link(refund, "", original, &parts[0]);
        for direction in [BookingDirection::All, BookingDirection::Expense] {
            let report =
                f.v.analytics_report(
                    BookingFilter {
                        direction,
                        ..Default::default()
                    },
                    false,
                )
                .unwrap();
            assert_eq!(
                report.expense_cents, 7000,
                "booking amount {amount}, {direction:?}"
            );
        }
    }
}

#[test]
fn round6_analytics_does_not_compute_the_unused_untagged_bucket() {
    let f = Fixture::new();
    f.add(i64::MAX, Some(f.income));
    let sale = f.add(100, None);
    let investment =
        f.v.create_category("Invest".into(), None, "chart.bar".into(), "#ff0000".into())
            .unwrap();
    f.v.db()
        .unwrap()
        .execute(
            "UPDATE categories SET builtin_role='Investitionen' WHERE id=?1",
            [investment],
        )
        .unwrap();
    f.v.db()
        .unwrap()
        .execute(
            "UPDATE transactions SET category_id=?1 WHERE id=?2",
            params![investment, sale],
        )
        .unwrap();
    assert!(
        f.v.analytics_report(BookingFilter::default(), false)
            .is_ok()
    );
    assert!(f.v.tag_totals(BookingFilter::default()).is_err());
}

#[test]
fn round6_split_category_presentation_follows_saved_positions() {
    let f = Fixture::new();
    let b = f.add(-100, None);
    f.split(b, &[(-40, Some(f.cost)), (-60, Some(f.income))]);
    let mut parts = f.v.list_transaction_splits(b).unwrap();
    parts.reverse();
    f.v.set_transaction_splits(
        b,
        parts
            .iter()
            .map(|p| TransactionSplitInput {
                uuid: Some(p.uuid.clone()),
                category_id: p.category_id,
                amount_cents: p.amount_cents,
                note: p.note.clone(),
            })
            .collect(),
    )
    .unwrap();
    let rows = f.v.list_transactions(BookingFilter::default()).unwrap();
    let b = rows.transactions.iter().find(|r| r.id == b).unwrap();
    let db = f.v.db().unwrap();
    let names = parts
        .iter()
        .map(|p| {
            db.query_row(
                "SELECT name FROM categories WHERE id=?1",
                [p.category_id],
                |r| r.get::<_, String>(0),
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(b.category_name, names.join(", "));
}

#[test]
fn round6_dashboard_primary_plan_totals_never_mix_currencies() {
    let f = Fixture::new();
    let usd =
        f.v.create_offline_account("USD".into(), AccountKind::Cash, "USD".into(), 0)
            .unwrap();
    f.add(-10000, Some(f.cost));
    f.add_on(usd, -10000, Some(f.cost), "USD", &f.date);
    f.v.db().unwrap().execute_batch(&format!("UPDATE categories SET builtin_role='Investitionen' WHERE id={}; UPDATE transactions SET counterparty='Security',purpose='Sparplan execution'",f.cost)).unwrap();
    for inherited in [false, true] {
        if inherited {
            f.v.db()
                .unwrap()
                .execute("UPDATE transactions SET currency=''", [])
                .unwrap();
        }
        let dashboard = f.v.investment_dashboard(1).unwrap();
        assert_eq!(dashboard.plans.len(), 1, "inherited currency: {inherited}");
        assert_eq!(dashboard.plans[0].total_invested_cents, 10000);
        assert_eq!(dashboard.plans[0].executions[0].amount_cents, 10000);
        assert_eq!(dashboard.monthly[0].investment_cents, 10000);
    }
}

#[test]
fn round6_unlinked_expense_direction_keeps_mixed_split_totals() {
    let f = Fixture::new();
    let expense = f.add(-100, None);
    f.split(expense, &[(-200, Some(f.cost)), (100, Some(f.income))]);
    let all =
        f.v.analytics_report(BookingFilter::default(), false)
            .unwrap();
    let expenses =
        f.v.analytics_report(
            BookingFilter {
                direction: BookingDirection::Expense,
                ..Default::default()
            },
            false,
        )
        .unwrap();
    assert_eq!(
        (expenses.income_cents, expenses.expense_cents),
        (all.income_cents, all.expense_cents)
    );
    let income =
        f.v.analytics_report(
            BookingFilter {
                direction: BookingDirection::Income,
                ..Default::default()
            },
            false,
        )
        .unwrap();
    assert_eq!((income.income_cents, income.expense_cents), (0, 0));
}

#[test]
fn round5_api_summary_sorts_minimum_signed_category_total() {
    let f = Fixture::new();
    let transfer = f.transfer_category();
    f.add(-i64::MAX, Some(transfer));
    f.add(-1, Some(transfer));
    let r = f.add(1, Some(transfer));
    let o = f.add(-1, Some(f.cost));
    f.link(r, "", o, "");
    f.share(&[f.account]);
    assert_eq!(f.summary()["by_category"][0]["net_cents"], json!(i64::MIN));
}

#[test]
fn round5_invested_ignores_sale_proceeds_but_includes_refunds() {
    let f = Fixture::new();
    let investment =
        f.v.create_category("Invest".into(), None, "chart.bar".into(), "#ff0000".into())
            .unwrap();
    f.v.db()
        .unwrap()
        .execute(
            "UPDATE categories SET builtin_role='Investitionen' WHERE id=?1",
            [investment],
        )
        .unwrap();
    let purchase = f.add(-1000, Some(investment));
    f.add(1500, Some(investment));
    let r = f.add(200, None);
    f.link(r, "", purchase, "");
    assert_eq!(
        f.v.selection_flow_for_totals(vec![], None, None)
            .unwrap()
            .invested_cents,
        800
    );
}

#[test]
fn scoped_loan_refund_offsets_only_a_shared_repayment() {
    let f = Fixture::new();
    let original = f.add(-500, Some(f.cost));
    let refund = f.add(200, None);
    let loan =
        f.v.create_manual_loan(crate::loans::LoanSetup {
            name: "Loan".into(),
            original_cents: None,
            anchor_date: f.date.clone(),
            anchor_balance_cents: 10000,
            interest_rate_ppm: None,
            match_expr: String::new(),
        })
        .unwrap();
    f.v.set_booking_loan(original, Some(loan)).unwrap();
    f.link(refund, "", original, "");
    let db = f.v.db().unwrap();
    for (accounts, expected) in [(vec![f.account], 0), (vec![f.account, loan], -300)] {
        let selection =
            f.v.projected_selection(
                &db,
                &BookingFilter::default(),
                Some(&accounts),
                CurrencyMode::Primary,
            )
            .unwrap();
        assert_eq!(selection.totals().unwrap().expense_cents, expected);
        assert!(
            selection
                .category_flows(&db, Side::Expense)
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn tag_analytics_skip_transfers_but_keep_projected_refund_credits() {
    let f = Fixture::new();
    let transfer = f.transfer_category();
    let original = f.add(-1000, Some(f.cost));
    let refund = f.add(400, Some(transfer));
    let own_transfer = f.add(-50000, Some(transfer));
    let tag = f.v.create_tag("Trip".into(), "#ff0000".into()).unwrap();
    for id in [original, refund, own_transfer] {
        f.v.set_booking_tags(id, vec![tag.id]).unwrap();
    }
    f.link(refund, "", original, "");
    let report =
        f.v.analytics_report(BookingFilter::default(), false)
            .unwrap();
    assert_eq!(report.tags[0].cents, -600);
    assert_eq!(report.tags[0].booking_count, 2);
}

#[test]
fn supplementary_guard_inherits_account_currency() {
    let f = Fixture::new();
    let usd =
        f.v.create_offline_account("USD".into(), AccountKind::Cash, "USD".into(), 0)
            .unwrap();
    f.add_on(usd, -300, Some(f.cost), "", &f.date);
    let totals = f.v.flow_other_currencies(BookingFilter::default()).unwrap();
    assert_eq!(totals.len(), 1);
    assert_eq!(totals[0].currency, "USD");
    assert_eq!(totals[0].consumption_cents, 300);
}

#[test]
fn investment_dashboard_does_not_total_future_months() {
    let f = Fixture::new();
    let future = f.add_on(f.account, -100, Some(f.cost), "EUR", "9999-01-01");
    f.split(
        future,
        &[
            (-i64::MAX, Some(f.cost)),
            (i64::MAX, Some(f.income)),
            (-100, Some(f.cost)),
        ],
    );
    assert!(f.v.investment_dashboard(1).is_ok());
}

#[test]
fn category_match_cache_separates_date_ranges_and_revisions() {
    let f = Fixture::new();
    let first = f.add_on(f.account, -1000, Some(f.cost), "EUR", "2020-01-01");
    let second = f.add_on(f.account, -2000, Some(f.cost), "EUR", "2020-02-01");
    let ids = |from: Option<&str>, to: Option<&str>| {
        let db = f.v.db().unwrap();
        serde_json::from_str::<Vec<i64>>(
            &f.v.projected_booking_ids(
                &db,
                &BookingFilter {
                    category_id: Some(f.root),
                    include_subcategories: true,
                    date_from: from.map(str::to_string),
                    date_to: to.map(str::to_string),
                    ..Default::default()
                },
            )
            .unwrap(),
        )
        .unwrap()
        .into_iter()
        .collect::<HashSet<_>>()
    };
    assert_eq!(ids(None, None), HashSet::from([first, second]));
    assert_eq!(ids(Some(""), Some("")), HashSet::from([first, second]));
    assert_eq!(ids(None, Some("2020-01-31")), HashSet::from([first]));
    assert_eq!(ids(Some("2020-02-01"), None), HashSet::from([second]));
    assert!(ids(Some("2021-01-01"), None).is_empty());
    assert_eq!(ids(None, None), HashSet::from([first, second]));
    let third = f.add_on(f.account, -3000, Some(f.cost), "EUR", "2020-01-15");
    assert_eq!(ids(None, Some("2020-01-31")), HashSet::from([first, third]));
}

#[test]
fn category_rollups_reach_deep_roots_and_stop_at_cycles() {
    let f = Fixture::new();
    let mut leaf = f.root;
    for depth in 0..40 {
        leaf =
            f.v.create_category(
                format!("Depth {depth}"),
                Some(leaf),
                "cart".into(),
                "#ff0000".into(),
            )
            .unwrap();
    }
    f.add_on(f.account, -1000, Some(leaf), "EUR", "2020-01-01");
    let filter = BookingFilter::default();
    let db = f.v.db().unwrap();
    let totals = f.v.projected_categories(&db, &filter, Side::Net).unwrap();
    assert_eq!(totals.subtree[&Some(f.root)], (-1000, 1));
    let selection =
        f.v.projected_selection(&db, &filter, None, CurrencyMode::All)
            .unwrap();
    assert!(selection.category_currency_subtrees().unwrap().contains(&(
        Some(f.root),
        "EUR".into(),
        -1000
    )));
    db.execute(
        "UPDATE categories SET parent_id=?1 WHERE id=?2",
        params![leaf, f.root],
    )
    .unwrap();
    let totals = f.v.projected_categories(&db, &filter, Side::Net).unwrap();
    assert_eq!(totals.subtree[&Some(f.root)], (-1000, 1));
}

struct Fixture {
    _dir: TempDir,
    v: Arc<Vault>,
    account: i64,
    root: i64,
    cost: i64,
    income: i64,
    year: i32,
    month: u32,
    date: String,
}

#[test]
fn zero_and_cancelled_parts_keep_category_membership() {
    let f = Fixture::new();
    let zero = f.add(0, Some(f.cost));
    let transfer = f.transfer_category();
    let mixed = f.add(0, None);
    f.split(mixed, &[(-500, Some(transfer)), (500, Some(transfer))]);
    for (id, category) in [(zero, f.cost), (mixed, transfer)] {
        let filter = BookingFilter {
            category_id: Some(category),
            ..Default::default()
        };
        assert_eq!(f.v.count_transactions(filter.clone()).unwrap(), 1);
        assert_eq!(
            f.v.list_transactions(filter).unwrap().transactions[0].id,
            id
        );
    }
}

#[test]
fn foreign_currency_tag_counts_do_not_add_foreign_amounts() {
    let f = Fixture::new();
    let id = f.add_on(f.account, -1200, Some(f.cost), "USD", &f.date);
    let tag = f.v.create_tag("Foreign".into(), String::new()).unwrap().id;
    f.v.set_booking_tags(id, vec![tag]).unwrap();
    let totals = f.v.tag_totals(BookingFilter::default()).unwrap();
    let row = totals.iter().find(|row| row.tag_id == Some(tag)).unwrap();
    assert_eq!((row.booking_count, row.sum_cents), (1, 0));
}

#[test]
fn review_empty_currency_uses_the_account_and_explicit_eur_stays_eur() {
    let f = Fixture::new();
    f.v.db()
        .unwrap()
        .execute(
            "UPDATE accounts SET currency='USD' WHERE id=?1",
            [f.account],
        )
        .unwrap();
    let original = f.add_on(f.account, -1000, Some(f.cost), "USD", &f.date);
    let refund = f.add_on(f.account, 400, Some(f.income), "", &f.date);
    f.add_on(f.account, -200, Some(f.cost), "EUR", &f.date);
    f.v.link_refund(refund, "".into(), original, "".into())
        .unwrap();
    let db = f.v.db().unwrap();
    let selection =
        f.v.projected_selection(&db, &BookingFilter::default(), None, CurrencyMode::All)
            .unwrap();
    let totals = selection.currency_totals().unwrap();
    assert_eq!(
        totals
            .iter()
            .find(|(c, _)| c == "USD")
            .unwrap()
            .1
            .expense_cents,
        -600
    );
    assert_eq!(
        totals
            .iter()
            .find(|(c, _)| c == "EUR")
            .unwrap()
            .1
            .expense_cents,
        -200
    );
    drop(db);
    f.share(&[f.account]);
    let summary = f.summary();
    let dollars = summary["totals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["currency"] == "USD")
        .unwrap();
    assert_eq!(dollars["booking_count"], 2);
    assert_eq!(dollars["expense_cents"], 600);
    for ids in [vec![f.account], vec![f.account, f.account]] {
        let totals =
            f.v.flow_other_currencies(BookingFilter {
                account_ids: ids.clone(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(totals.len(), 1);
        assert_eq!(totals[0].currency, "EUR");
        assert_eq!(
            f.v.category_other_currencies(f.cost, ids, None, None)
                .unwrap()[0]
                .cents,
            -200
        );
    }
}

#[test]
fn review_tag_counts_ignore_the_totals_account_flag() {
    let f = Fixture::new();
    let id = f.add(-100, Some(f.cost));
    let tag = f.v.create_tag("Count".into(), String::new()).unwrap().id;
    f.v.set_booking_tags(id, vec![tag]).unwrap();
    f.v.db()
        .unwrap()
        .execute(
            "UPDATE accounts SET include_in_total=0 WHERE id=?1",
            [f.account],
        )
        .unwrap();
    let rows =
        f.v.tag_totals(BookingFilter {
            respect_in_total: true,
            ..Default::default()
        })
        .unwrap();
    let row = rows.iter().find(|r| r.tag_id == Some(tag)).unwrap();
    assert_eq!((row.booking_count, row.sum_cents), (1, 0));
}

#[test]
fn review_unselected_overflow_does_not_poison_reports_and_remaining_zero_net_parts_keep_membership()
{
    let f = Fixture::new();
    let bad = f.add(0, None);
    f.split(
        bad,
        &[
            (-i64::MAX, Some(f.cost)),
            (i64::MAX, Some(f.income)),
            (-100, Some(f.cost)),
            (100, Some(f.income)),
        ],
    );
    f.add_on(f.account, -200, Some(f.cost), "EUR", "2020-01-01");
    let report =
        f.v.analytics_report(
            BookingFilter {
                date_from: Some("2020-01-01".into()),
                date_to: Some("2020-01-01".into()),
                ..Default::default()
            },
            false,
        )
        .unwrap();
    assert_eq!(report.expense_cents, 200);
    assert!(
        f.v.analytics_report(BookingFilter::default(), false)
            .is_err()
    );
    let f = Fixture::new();
    let r = f.add(100, Some(f.income));
    let o = f.add(-100, Some(f.cost));
    f.split(
        r,
        &[
            (100, Some(f.income)),
            (200, Some(f.cost)),
            (-200, Some(f.cost)),
        ],
    );
    let part = f.v.list_transaction_splits(r).unwrap()[0].uuid.clone();
    f.v.link_refund(r, part, o, "".into()).unwrap();
    let filter = BookingFilter {
        category_id: Some(f.cost),
        ..Default::default()
    };
    assert!(
        f.v.list_transactions(filter)
            .unwrap()
            .transactions
            .iter()
            .any(|b| b.id == r)
    );
}

#[test]
fn uncategorized_takes_precedence_over_a_stale_category_filter() {
    let f = Fixture::new();
    let id = f.add(-1200, None);
    f.add(-300, Some(f.cost));
    let filter = BookingFilter {
        category_id: Some(f.cost),
        uncategorized_only: true,
        ..Default::default()
    };
    assert_eq!(
        f.v.list_transactions(filter.clone()).unwrap().transactions[0].id,
        id
    );
    assert_eq!(f.v.count_transactions(filter.clone()).unwrap(), 1);
    let db = f.v.db().unwrap();
    assert_eq!(
        f.v.projected_totals(&db, &filter, None, CurrencyMode::Primary)
            .unwrap()
            .expense_cents,
        -1200
    );
}

#[test]
fn projected_matching_cache_is_bounded() {
    let f = Fixture::new();
    f.add(-1200, Some(f.cost));
    let db = f.v.db().unwrap();
    for year in 1900..1980 {
        f.v.projected_booking_ids(
            &db,
            &BookingFilter {
                category_id: Some(f.cost),
                date_from: Some(format!("{year}-01-01")),
                ..Default::default()
            },
        )
        .unwrap();
    }
    assert!(
        f.v.financial_projection(&db)
            .unwrap()
            .matching_ids
            .lock()
            .unwrap()
            .len()
            <= 64
    );
}

#[test]
fn euro_bookings_on_a_dollar_account_have_supplementary_totals() {
    let f = Fixture::new();
    f.v.db()
        .unwrap()
        .execute(
            "UPDATE accounts SET currency='USD' WHERE id=?1",
            [f.account],
        )
        .unwrap();
    f.add(-1200, Some(f.cost));
    let filter = BookingFilter {
        account_ids: vec![f.account],
        ..Default::default()
    };
    let flow = f.v.flow_other_currencies(filter).unwrap();
    assert_eq!(flow.len(), 1);
    assert_eq!(flow[0].currency, "EUR");
    let totals =
        f.v.category_other_currency_totals(
            None,
            None,
            vec![f.account],
            None,
            BookingDirection::All,
            None,
            None,
            vec![],
            false,
        )
        .unwrap();
    assert!(
        totals
            .iter()
            .any(|row| row.currency == "EUR" && row.cents == -1200)
    );
}

#[test]
fn loan_proof_rows_do_not_expand_explicit_total_accounts() {
    let f = Fixture::new();
    let payment = f.add(-500, Some(f.cost));
    let loan =
        f.v.create_manual_loan(crate::loans::LoanSetup {
            name: "Loan".into(),
            original_cents: None,
            anchor_date: f.date.clone(),
            anchor_balance_cents: 10000,
            interest_rate_ppm: None,
            match_expr: String::new(),
        })
        .unwrap();
    f.v.set_booking_loan(payment, Some(loan)).unwrap();
    let db = f.v.db().unwrap();
    db.execute(
        "UPDATE accounts SET include_in_total=1 WHERE id IN (?1,?2)",
        params![loan, f.account],
    )
    .unwrap();
    drop(db);
    let filter = BookingFilter {
        account_ids: vec![loan],
        respect_in_total: true,
        ..Default::default()
    };
    assert_eq!(f.v.count_transactions(filter.clone()).unwrap(), 1);
    let db = f.v.db().unwrap();
    let totals =
        f.v.projected_totals(&db, &filter, None, CurrencyMode::Primary)
            .unwrap();
    assert_eq!((totals.income_cents, totals.expense_cents), (0, 0));
}

#[test]
fn review_supplementary_totals_include_exact_loan_reference_proof_rows() {
    let f = Fixture::new();
    let loan =
        f.v.create_manual_loan(crate::loans::LoanSetup {
            name: "Loan".into(),
            original_cents: None,
            anchor_date: f.date.clone(),
            anchor_balance_cents: 10000,
            interest_rate_ppm: None,
            match_expr: String::new(),
        })
        .unwrap();
    let other =
        f.v.create_offline_account("USD".into(), AccountKind::Cash, "USD".into(), 0)
            .unwrap();
    let expense = f.add_on(other, -300, Some(f.cost), "USD", &f.date);
    let filter = BookingFilter {
        account_ids: vec![loan, f.account],
        ..Default::default()
    };
    assert!(
        f.v.flow_other_currencies(filter.clone())
            .unwrap()
            .is_empty()
    );
    let db = f.v.db().unwrap();
    db.execute(
        "UPDATE accounts SET account_number='123456789' WHERE id=?1",
        [loan],
    )
    .unwrap();
    db.execute(
        "UPDATE transactions SET purpose='Loan 123456789' WHERE id=?1",
        [expense],
    )
    .unwrap();
    drop(db);
    let totals = f.v.flow_other_currencies(filter).unwrap();
    assert_eq!(totals.len(), 1);
    assert_eq!(
        (totals[0].currency.as_str(), totals[0].consumption_cents),
        ("USD", 300)
    );
}

#[test]
fn review_scoped_uncategorized_keeps_transfers_but_not_allocated_loan_refunds() {
    let f = Fixture::new();
    let original = f.add(-500, Some(f.cost));
    let refund = f.add(500, None);
    let transfer = f.add(200, None);
    let loan =
        f.v.create_manual_loan(crate::loans::LoanSetup {
            name: "Loan".into(),
            original_cents: None,
            anchor_date: f.date.clone(),
            anchor_balance_cents: 10000,
            interest_rate_ppm: None,
            match_expr: String::new(),
        })
        .unwrap();
    for id in [refund, transfer] {
        f.v.set_booking_loan(id, Some(loan)).unwrap();
    }
    f.link(refund, "", original, "");
    f.share(&[f.account]);
    let response: Value = serde_json::from_str(&f.v.local_api_request(
        &json!({"v":1,"method":"transactions","params":{"uncategorized_only":true}}).to_string(),
    ))
    .unwrap();
    let rows = response["result"]["transactions"].as_array().unwrap();
    assert!(rows.iter().any(|r| r["id"] == transfer));
    assert!(!rows.iter().any(|r| r["id"] == refund));
    let db = f.v.db().unwrap();
    let ids: Vec<i64> = serde_json::from_str(
        &f.v.projected_booking_ids_visible(
            &db,
            &BookingFilter {
                uncategorized_only: true,
                ..Default::default()
            },
            Some(&[f.account]),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(ids.contains(&transfer));
    assert!(!ids.contains(&refund));
}

impl Fixture {
    fn new() -> Self {
        let dir = TempDir::new("projection");
        let v = Vault::open(dir.join("v.db").to_string_lossy().into(), vec![7; 32]).unwrap();
        let account = v
            .create_offline_account("Wallet".into(), AccountKind::Cash, "EUR".into(), 0)
            .unwrap();
        let root = v
            .create_category("Costs".into(), None, "cart".into(), "#ff0000".into())
            .unwrap();
        let cost = v
            .create_category("Child".into(), Some(root), "cart".into(), "#ff0000".into())
            .unwrap();
        let income = v
            .create_category("Credits".into(), None, "plus".into(), "#00ff00".into())
            .unwrap();
        let today = crate::dates::today();
        let year = i32::from(today.year());
        let month = today.month() as u32;
        Self {
            _dir: dir,
            v,
            account,
            root,
            cost,
            income,
            year,
            month,
            date: format!("{year:04}-{month:02}-01"),
        }
    }
    fn add(&self, amount: i64, category: Option<i64>) -> i64 {
        self.add_on(self.account, amount, category, "EUR", &self.date)
    }
    fn add_on(
        &self,
        account: i64,
        amount: i64,
        category: Option<i64>,
        currency: &str,
        date: &str,
    ) -> i64 {
        let db = self.v.db().unwrap();
        db.execute("INSERT INTO transactions(account_id,external_id,booking_date,amount_cents,currency,category_id,counterparty) VALUES (?1,?2,?3,?4,?5,?6,'Shop')",params![account,uuid::Uuid::new_v4().to_string(),date,amount,currency,category]).unwrap();
        db.last_insert_rowid()
    }
    fn link(&self, refund: i64, part: &str, original: i64, original_part: &str) {
        self.v
            .link_refund(refund, part.into(), original, original_part.into())
            .unwrap();
    }
    fn transfer_category(&self) -> i64 {
        let id = self
            .v
            .create_category(
                "Transfers".into(),
                None,
                "arrow.left.arrow.right".into(),
                "#ff0000".into(),
            )
            .unwrap();
        self.v
            .db()
            .unwrap()
            .execute(
                "UPDATE categories SET builtin_role='Umbuchungen' WHERE id=?1",
                [id],
            )
            .unwrap();
        id
    }
    fn split(&self, id: i64, parts: &[(i64, Option<i64>)]) -> Vec<String> {
        self.v
            .set_transaction_splits(
                id,
                parts
                    .iter()
                    .map(|(amount, category)| TransactionSplitInput {
                        uuid: None,
                        category_id: *category,
                        amount_cents: *amount,
                        note: String::new(),
                    })
                    .collect(),
            )
            .unwrap();
        self.v
            .list_transaction_splits(id)
            .unwrap()
            .into_iter()
            .map(|p| p.uuid)
            .collect()
    }
    fn share(&self, accounts: &[i64]) {
        self.v
            .set_local_api_scope(ShareScope {
                enabled: true,
                account_ids: accounts.to_vec(),
                ..Default::default()
            })
            .unwrap();
    }
    fn summary(&self) -> Value {
        let request = json!({"v":1,"method":"summary","params":{"date_from":format!("{:04}-{:02}-01",self.year,self.month),"date_to":format!("{:04}-{:02}-28",self.year,self.month)}});
        let response: Value =
            serde_json::from_str(&self.v.local_api_request(&request.to_string())).unwrap();
        assert!(response.get("error").is_none(), "{response}");
        response["result"].clone()
    }
    fn category_total(&self, id: i64) -> i64 {
        self.v
            .list_categories(None, None)
            .unwrap()
            .iter()
            .find(|c| c.id == id)
            .unwrap()
            .total_cents
    }
    fn spending(&self) -> i64 {
        self.v
            .selection_flow(vec![], None, None)
            .unwrap()
            .consumption_cents
    }
}

#[test]
fn every_report_entry_point_uses_the_same_refund_adjusted_fixture() {
    let f = Fixture::new();
    let original = f.add(-1000, Some(f.cost));
    let refund = f.add(400, Some(f.income));
    let baseline = f.spending();
    assert_eq!(baseline, 1000);
    f.link(refund, "", original, "");
    f.v.set_category_budget(f.root, Some(1000)).unwrap();
    let tag = f.v.create_tag("Trip".into(), "#ff0000".into()).unwrap();
    f.v.set_booking_tags(original, vec![tag.id]).unwrap();
    f.v.set_booking_tags(refund, vec![tag.id]).unwrap();
    assert_eq!(f.category_total(f.root), -600);
    assert_eq!(f.category_total(f.cost), -600);
    assert_eq!(f.category_total(f.income), 0);
    let a =
        f.v.list_categories_filtered(None, None, vec![], None, BookingDirection::All)
            .unwrap();
    assert_eq!(a.iter().find(|c| c.id == f.root).unwrap().total_cents, -600);
    let a =
        f.v.list_categories_filtered_amount(
            None,
            None,
            vec![],
            None,
            BookingDirection::All,
            None,
            None,
            vec![tag.id],
            false,
        )
        .unwrap();
    assert_eq!(a.iter().find(|c| c.id == f.root).unwrap().total_cents, -600);
    let expenses =
        f.v.list_category_expenses(BookingFilter::default())
            .unwrap();
    assert_eq!(
        expenses
            .iter()
            .find(|c| c.category_id == Some(f.root))
            .unwrap()
            .cents,
        600
    );
    assert_eq!(
        f.v.category_history(f.root, vec![], None, None).unwrap()[0].cents,
        -600
    );
    assert_eq!(
        f.v.category_history_by_unit(f.root, vec![], None, None, 4)
            .unwrap()[0]
            .cents,
        -600
    );
    assert_eq!(
        f.v.list_category_budget_progress(f.year, f.month).unwrap()[0].spent_cents,
        600
    );
    assert_eq!(
        f.v.cashflow_by_period(PeriodGranularity::Month, 1, None, false)
            .unwrap()[0]
            .expense_cents,
        600
    );
    assert_eq!(
        f.v.investment_dashboard(1).unwrap().monthly[0].consumption_cents,
        600
    );
    for limit in [1, 6000] {
        let p =
            f.v.list_transactions(BookingFilter {
                limit,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(p.expense_cents, -600);
        assert_eq!(p.income_cents, 0);
        assert_eq!(p.flow.consumption_cents, 600);
    }
    assert_eq!(f.spending(), 600);
    assert_eq!(
        f.v.selection_flow_for_totals(vec![], None, None)
            .unwrap()
            .consumption_cents,
        600
    );
    let filter = BookingFilter {
        category_id: Some(f.root),
        include_subcategories: true,
        ..Default::default()
    };
    let tagged = f.v.tag_totals(filter.clone()).unwrap();
    let tagged = tagged.iter().find(|t| t.tag_id == Some(tag.id)).unwrap();
    assert_eq!(tagged.sum_cents, -600);
    assert_eq!(tagged.booking_count, 2);
    let report = f.v.analytics_report(filter.clone(), false).unwrap();
    assert_eq!(report.monthly[0].expense_cents, 600);
    assert_eq!(report.expense_cents, 600);
    assert_eq!(report.tags[0].cents, -600);
    assert_eq!(report.category_expenses[0].cents, 600);
    let p = f.v.list_transactions(filter).unwrap();
    assert_eq!(p.total_count, 2);
    assert_eq!(p.expense_cents, -600);
    f.share(&[f.account]);
    let summary = f.summary();
    assert_eq!(summary["totals"][0]["expense_cents"], 600);
    assert_eq!(summary["totals"][0]["income_cents"], 0);
    assert_eq!(
        summary["by_category"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["category_id"] == f.cost)
            .unwrap()["net_cents"],
        -600
    );
    f.v.unlink_refund(refund, String::new(), original, String::new())
        .unwrap();
    assert_eq!(f.spending(), baseline);
}

mod previous_pr_regressions {
    use super::*;

    #[test]
    fn new_category_of_a_part_keeps_its_refund() {
        let f = Fixture::new();
        let other =
            f.v.create_category("Other".into(), None, "tag".into(), "#abcdef".into())
                .unwrap();
        let original = f.add(-10000, None);
        let refund = f.add(1000, None);
        let parts = f.split(original, &[(-6000, Some(f.cost)), (-4000, Some(f.income))]);
        f.link(refund, "", original, &parts[0]);
        let replacement = f.split(original, &[(-6000, Some(other)), (-4000, Some(f.income))]);
        assert_eq!(replacement[0], parts[0]);
        assert_eq!(
            [f.cost, f.income, other].map(|id| f.category_total(id)),
            [0, -4000, -5000]
        );
        assert_eq!(f.v.refund_links(refund).unwrap()[0].allocated_cents, 1000);
    }

    #[test]
    fn odd_cent_split_refund_keeps_every_cent_in_category_views() {
        let f = Fixture::new();
        let refund = f.add(10001, None);
        let original = f.add(-3000, None);
        let parts = f.split(refund, &[(6000, Some(f.cost)), (4001, Some(f.income))]);
        f.link(refund, &parts[0], original, "");
        let page = |category_id| {
            f.v.list_transactions(BookingFilter {
                category_id,
                ..Default::default()
            })
            .unwrap()
        };
        assert_eq!(page(None).income_cents, 7001);
        assert_eq!(
            page(Some(f.cost)).income_cents + page(Some(f.income)).income_cents,
            7001
        );
        assert_eq!(f.category_total(f.cost) + f.category_total(f.income), 7001);
    }

    #[test]
    fn category_sums_under_a_direction_follow_the_list() {
        let f = Fixture::new();
        let original = f.add(-3000, Some(f.cost));
        let refund = f.add(1000, Some(f.cost));
        f.link(refund, "", original, "");
        for (direction, sum) in [
            (BookingDirection::All, -2000),
            (BookingDirection::Expense, -2000),
            (BookingDirection::Income, 0),
        ] {
            let page =
                f.v.list_transactions(BookingFilter {
                    category_id: Some(f.cost),
                    direction,
                    ..Default::default()
                })
                .unwrap();
            let sidebar =
                f.v.list_categories_filtered(None, None, vec![], None, direction)
                    .unwrap()
                    .into_iter()
                    .find(|c| c.id == f.cost)
                    .unwrap()
                    .own_total_cents;
            assert_eq!(
                (sidebar, page.income_cents + page.expense_cents),
                (sum, sum),
                "{direction:?}"
            );
        }
    }

    #[test]
    fn linked_refund_counts_once_per_category() {
        let f = Fixture::new();
        let first = f.add(-3000, Some(f.cost));
        let second = f.add(-2000, Some(f.cost));
        let refund = f.add(6000, Some(f.cost));
        let totals = |id| {
            let c =
                f.v.list_categories(None, None)
                    .unwrap()
                    .into_iter()
                    .find(|c| c.id == id)
                    .unwrap();
            (c.own_total_cents, c.transaction_count)
        };
        assert_eq!(totals(f.cost), (1000, 3));
        f.link(refund, "", first, "");
        f.link(refund, "", second, "");
        assert_eq!(totals(f.cost), (1000, 3));
        f.v.set_transaction_category(refund, Some(f.income))
            .unwrap();
        assert_eq!(totals(f.cost), (0, 3));
        assert_eq!(
            f.v.list_transactions(BookingFilter {
                category_id: Some(f.cost),
                ..Default::default()
            })
            .unwrap()
            .transactions
            .len(),
            3
        );
        assert_eq!(totals(f.income), (1000, 1));
    }

    #[test]
    fn qa_wording_consumption_partial_refund_remainder_is_income() {
        let f = Fixture::new();
        let refund = f.add(10000, None);
        let original = f.add(-3000, None);
        f.v.db()
            .unwrap()
            .execute(
                "UPDATE transactions SET purpose='Rückerstattung Retoure Erstattung' WHERE id=?1",
                [refund],
            )
            .unwrap();
        let before = f.v.list_transactions(BookingFilter::default()).unwrap();
        assert_eq!(
            before
                .transactions
                .iter()
                .find(|b| b.id == refund)
                .unwrap()
                .flow,
            crate::merchants::FlowKind::Income
        );
        f.link(refund, "", original, "");
        let after = f.v.list_transactions(BookingFilter::default()).unwrap();
        assert_eq!((after.income_cents, after.expense_cents), (7000, 0));
    }
}

#[test]
fn expense_parts_of_zero_net_and_income_originals_are_expenses() {
    for income in [1000, 1500] {
        let f = Fixture::new();
        let original = f.add(income - 1000, None);
        let parts = f.split(original, &[(-1000, Some(f.cost)), (income, Some(f.income))]);
        let refund = f.add(700, None);
        let before = f.v.list_transaction_splits(original).unwrap();
        f.link(refund, "", original, &parts[0]);
        let flow = f.v.selection_flow(vec![], None, None).unwrap();
        assert_eq!(flow.consumption_cents, 300);
        assert_eq!(flow.income_cents, income);
        assert_eq!(f.category_total(f.cost), -300);
        assert_eq!(f.category_total(f.income), income);
        assert_eq!(
            format!("{before:?}"),
            format!("{:?}", f.v.list_transaction_splits(original).unwrap())
        );
    }
}

#[test]
fn full_positive_split_capacity_is_not_capped_at_the_refund_booking_net() {
    for debit in [-800, -700, -900] {
        let f = Fixture::new();
        let original = f.add(-1000, Some(f.cost));
        let refund = f.add(800 + debit, None);
        let parts = f.split(refund, &[(800, None), (debit, Some(f.cost))]);
        f.link(refund, &parts[0], original, "");
        let flow = f.v.selection_flow(vec![], None, None).unwrap();
        assert_eq!(flow.consumption_cents, 1000 - debit - 800);
        assert_eq!(flow.income_cents, 0);
        assert_eq!(f.v.refund_links(refund).unwrap()[0].allocated_cents, 800);
    }
}

#[test]
fn unallocated_positive_cents_stay_income_and_uncategorized_costs_remain_expenses() {
    let f = Fixture::new();
    let original = f.add(-1000, None);
    let refund = f.add(1500, None);
    f.link(refund, "", original, "");
    let flow = f.v.selection_flow(vec![], None, None).unwrap();
    assert_eq!(flow.consumption_cents, 0);
    assert_eq!(flow.income_cents, 500);
    let p =
        f.v.list_transactions(BookingFilter {
            uncategorized_only: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(p.income_cents, 500);
    assert_eq!(p.expense_cents, 0);
    f.add(-200, None);
    let a =
        f.v.list_category_expenses(BookingFilter::default())
            .unwrap();
    assert_eq!(
        a.iter().find(|c| c.category_id.is_none()).unwrap().cents,
        200
    );
    assert_eq!(
        f.v.analytics_report(BookingFilter::default(), false)
            .unwrap()
            .expense_cents,
        200
    );
}

#[test]
fn destination_credit_is_independent_of_a_transfer_classified_refund() {
    let f = Fixture::new();
    let original = f.add(-1000, Some(f.cost));
    let transfer_category = f.transfer_category();
    let refund = f.add(400, Some(transfer_category));
    f.v.db()
        .unwrap()
        .execute(
            "UPDATE transactions SET purpose='Umbuchung' WHERE id=?1",
            [refund],
        )
        .unwrap();
    f.link(refund, "", original, "");
    assert_eq!(f.spending(), 600);
    assert_eq!(
        f.v.list_transactions(BookingFilter::default())
            .unwrap()
            .transactions
            .iter()
            .find(|b| b.id == refund)
            .unwrap()
            .flow,
        FlowKind::Transfer
    );
    assert_eq!(
        f.v.analytics_report(BookingFilter::default(), false)
            .unwrap()
            .expense_cents,
        600
    );
}

#[test]
fn currencies_and_account_date_attribution_stay_separate() {
    let f = Fixture::new();
    let original = f.add(-1000, Some(f.cost));
    let refund = f.add(400, None);
    f.link(refund, "", original, "");
    let usd =
        f.v.create_offline_account("Dollar".into(), AccountKind::Cash, "USD".into(), 0)
            .unwrap();
    let o = f.add_on(usd, -2000, Some(f.cost), "USD", &f.date);
    let r = f.add_on(usd, 800, None, "USD", &f.date);
    f.link(r, "", o, "");
    assert_eq!(f.spending(), 600);
    assert_eq!(f.category_total(f.root), -600);
    assert_eq!(
        f.v.category_other_currencies(f.root, vec![], None, None)
            .unwrap()[0]
            .cents,
        -1200
    );
    let other =
        f.v.category_other_currency_totals(
            None,
            None,
            vec![],
            None,
            BookingDirection::All,
            None,
            None,
            vec![],
            false,
        )
        .unwrap();
    assert_eq!(
        other
            .iter()
            .find(|c| c.category_id == f.root)
            .unwrap()
            .cents,
        -1200
    );
    assert_eq!(
        f.v.selection_flow_other_currencies(vec![], None, None, false)
            .unwrap()[0]
            .consumption_cents,
        1200
    );
    assert_eq!(
        f.v.selection_flow(vec![usd], None, None)
            .unwrap()
            .consumption_cents,
        1200
    );
    assert!(
        f.v.selection_flow_other_currencies(vec![usd], None, None, false)
            .unwrap()
            .is_empty()
    );
    f.share(&[f.account, usd]);
    let summary = f.summary();
    assert_eq!(summary["totals"].as_array().unwrap().len(), 2);
    assert_eq!(
        summary["totals"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["currency"] == "USD")
            .unwrap()["expense_cents"],
        1200
    );
    let later = format!("{:04}-{:02}-15", f.year, f.month);
    let euro =
        f.v.create_offline_account("Other euro".into(), AccountKind::Cash, "EUR".into(), 0)
            .unwrap();
    f.v.db()
        .unwrap()
        .execute(
            "UPDATE transactions SET account_id=?1,booking_date=?2 WHERE id=?3",
            params![euro, later, refund],
        )
        .unwrap();
    assert_eq!(
        f.v.category_history_by_unit(f.root, vec![euro], Some(later), None, 10)
            .unwrap()[0]
            .cents,
        400
    );
    assert_eq!(
        f.v.selection_flow(vec![f.account], None, None)
            .unwrap()
            .consumption_cents,
        1000
    );
}

#[test]
fn transfers_and_loans_never_enter_expense_category_mode() {
    let f = Fixture::new();
    let transfer_category = f.transfer_category();
    let transfer = f.add(-700, Some(transfer_category));
    let loan_payment = f.add(-500, Some(f.cost));
    let loan =
        f.v.create_manual_loan(crate::loans::LoanSetup {
            name: "Loan".into(),
            original_cents: None,
            anchor_date: f.date.clone(),
            anchor_balance_cents: 10000,
            interest_rate_ppm: None,
            match_expr: String::new(),
        })
        .unwrap();
    f.v.set_booking_loan(loan_payment, Some(loan)).unwrap();
    let db = f.v.db().unwrap();
    db.execute(
        "UPDATE accounts SET iban='DE89370400440532013000' WHERE id=?1",
        [f.account],
    )
    .unwrap();
    db.execute(
        "UPDATE transactions SET counterparty_iban='DE89370400440532013000' WHERE id=?1",
        [transfer],
    )
    .unwrap();
    drop(db);
    assert_eq!(
        f.v.analytics_report(BookingFilter::default(), false)
            .unwrap()
            .expense_cents,
        0
    );
    assert!(
        f.v.list_category_expenses(BookingFilter::default())
            .unwrap()
            .is_empty()
    );
    f.share(&[f.account]);
    assert_eq!(f.summary()["totals"][0]["expense_cents"], 0);
}

#[test]
fn scoped_allocations_ignore_hidden_competitors_even_after_global_cache_warmup() {
    let f = Fixture::new();
    let hidden =
        f.v.create_offline_account("Hidden".into(), AccountKind::Cash, "EUR".into(), 0)
            .unwrap();
    let hidden_category =
        f.v.create_category(
            "Private category".into(),
            None,
            "lock".into(),
            "#ff0000".into(),
        )
        .unwrap();
    let o = f.add(-1000, Some(f.cost));
    let hidden_o = f.add_on(hidden, -1000, Some(hidden_category), "EUR", &f.date);
    let r = f.add(1000, None);
    f.link(r, "", hidden_o, "");
    f.v.db().unwrap().execute("INSERT INTO refund_links(refund_id,original_id,refund_cents,original_cents,created_ms,changed_ms) VALUES (?1,?2,1000,-1000,?3,?3)",params![r,o,i64::MAX-100]).unwrap();
    f.spending();
    f.v.list_categories(None, None).unwrap();
    f.share(&[f.account]);
    let before = f.summary();
    assert_eq!(before["totals"][0]["expense_cents"], 0);
    assert_eq!(before["totals"][0]["income_cents"], 0);
    assert!(!before.to_string().contains("Private"));
    f.v.db()
        .unwrap()
        .execute(
            "UPDATE transactions SET amount_cents=-50 WHERE id=?1",
            [hidden_o],
        )
        .unwrap();
    f.spending();
    assert_eq!(f.summary(), before);
    let response: Value = serde_json::from_str(
        &f.v.local_api_request(&json!({"method":"transactions","params":{}}).to_string()),
    )
    .unwrap();
    assert!(response.get("error").is_none(), "{response}");
    assert!(!response.to_string().contains("refund_cents"));
    assert!(!response.to_string().contains("refund_marker"));
}

#[test]
fn projection_cache_is_revision_bound_and_scoped_views_are_read_only() {
    let f = Fixture::new();
    f.add(-100, Some(f.cost));
    let db = f.v.db().unwrap();
    let stamp = f.v.data_stamp(&db);
    let a = f.v.financial_projection(&db).unwrap();
    let b = f.v.financial_projection(&db).unwrap();
    assert!(Arc::ptr_eq(&a, &b));
    let x = a.rows(Some(&[f.account])).unwrap();
    let y = a.rows(Some(&[f.account, f.account])).unwrap();
    assert!(Arc::ptr_eq(&x, &y));
    assert_eq!(stamp, f.v.data_stamp(&db));
    db.execute("UPDATE transactions SET amount_cents=-200", [])
        .unwrap();
    let c = f.v.financial_projection(&db).unwrap();
    assert!(!Arc::ptr_eq(&a, &c));
    f.v.data_replaced();
    assert!(!Arc::ptr_eq(&c, &f.v.financial_projection(&db).unwrap()));
}

#[test]
fn reports_handle_empty_document_selection_and_overflow_as_errors() {
    let f = Fixture::new();
    let empty =
        f.v.analytics_report(BookingFilter::default(), false)
            .unwrap();
    assert_eq!(empty.expense_cents, 0);
    assert!(empty.category_expenses.is_empty());
    assert!(empty.tags.is_empty());
    let booking = f.add(-100, Some(f.cost));
    assert_eq!(
        f.v.analytics_report(BookingFilter::default(), true)
            .unwrap()
            .expense_cents,
        0
    );
    f.v.attach_document_to_booking(booking, "receipt.jpg".into(), vec![0xff, 0xd8])
        .unwrap();
    assert_eq!(
        f.v.analytics_report(BookingFilter::default(), true)
            .unwrap()
            .expense_cents,
        100
    );
    f.add(-i64::MAX, Some(f.cost));
    assert!(
        f.v.analytics_report(BookingFilter::default(), false)
            .is_err()
    );
}

#[test]
fn subtree_counts_deduplicate_one_refund_across_sibling_categories() {
    let f = Fixture::new();
    let sibling =
        f.v.create_category(
            "Sibling".into(),
            Some(f.root),
            "cart".into(),
            "#00ff00".into(),
        )
        .unwrap();
    let a = f.add(-400, Some(f.cost));
    let b = f.add(-600, Some(sibling));
    let r = f.add(500, None);
    f.link(r, "", a, "");
    f.link(r, "", b, "");
    let cats = f.v.list_categories(None, None).unwrap();
    let root = cats.iter().find(|c| c.id == f.root).unwrap();
    assert_eq!(root.total_cents, -500);
    assert_eq!(root.own_total_cents, 0);
    assert_eq!(root.transaction_count, 3);
    let expense =
        f.v.list_category_expenses(BookingFilter::default())
            .unwrap();
    assert_eq!(expense[0].booking_count, 3);
}

#[test]
fn foreign_currency_within_one_account_is_not_combined_with_its_primary_currency() {
    let f = Fixture::new();
    f.add(-100, Some(f.cost));
    f.add_on(f.account, -300, Some(f.cost), "USD", &f.date);
    assert_eq!(
        f.v.selection_flow(vec![f.account], None, None)
            .unwrap()
            .consumption_cents,
        100
    );
    let other =
        f.v.category_other_currencies(f.root, vec![f.account], None, None)
            .unwrap();
    assert_eq!(
        (&other[0].currency, other[0].cents),
        (&"USD".to_string(), -300)
    );
    let report =
        f.v.analytics_report(BookingFilter::default(), false)
            .unwrap();
    assert_eq!(report.currency, "EUR");
    assert_eq!(report.expense_cents, 100);
    assert_eq!(report.monthly.len(), 2);
    assert_eq!(
        f.v.selection_flow_other_currencies(vec![f.account], None, None, false)
            .unwrap()[0]
            .consumption_cents,
        300
    );
}

#[test]
fn gross_same_sign_split_totals_return_an_error_instead_of_overflowing() {
    let f = Fixture::new();
    let id = f.add(0, None);
    f.split(
        id,
        &[
            (-i64::MAX, Some(f.cost)),
            (i64::MAX, Some(f.income)),
            (-100, Some(f.cost)),
            (100, Some(f.income)),
        ],
    );
    assert!(f.v.list_categories(None, None).is_err());
    assert!(
        f.v.analytics_report(BookingFilter::default(), false)
            .is_err()
    );
    assert!(spending(i64::MIN).is_err());
}

#[test]
fn tag_totals_report_overflow_instead_of_clamping() {
    let f = Fixture::new();
    f.add(i64::MAX, Some(f.income));
    f.add(100, Some(f.income));
    assert!(f.v.tag_totals(BookingFilter::default()).is_err());
}

#[test]
fn local_summary_counts_and_sums_use_the_same_canonical_currency() {
    let f = Fixture::new();
    f.add(-200, Some(f.cost));
    f.add_on(f.account, -100, Some(f.cost), "eur", &f.date);
    f.share(&[f.account]);
    let summary = f.summary();
    let totals = summary["totals"].as_array().unwrap();
    assert_eq!(totals.len(), 1);
    assert_eq!(totals[0]["currency"], "EUR");
    assert_eq!(totals[0]["expense_cents"], 300);
    assert_eq!(totals[0]["booking_count"], 2);
}
