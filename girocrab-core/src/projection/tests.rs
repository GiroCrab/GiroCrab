use super::*;
use crate::local_api::ShareScope;
use crate::test_support::TempDir;
use rusqlite::params;
use serde_json::{Value, json};

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
            (BookingDirection::Expense, -3000),
            (BookingDirection::Income, 1000),
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
