//! Inline analytics over the filtered booking rows: category breakdown and tag evaluation.

use crate::daily::Row;

pub fn expense_accounts(
    accounts: &[girocrab_core::Account],
    target: crate::nav::Target,
) -> Vec<i64> {
    if let crate::nav::Target::Account(id) = target {
        return vec![id];
    }
    use girocrab_core::AccountKind;
    let ids: Vec<_> = accounts
        .iter()
        .filter(|a| {
            a.in_total
                && matches!(
                    a.kind,
                    AccountKind::Checking
                        | AccountKind::Savings
                        | AccountKind::PaymentService
                        | AccountKind::CreditCard
                        | AccountKind::Cash
                )
        })
        .map(|a| a.id)
        .collect();
    if ids.is_empty() { vec![-1] } else { ids }
}

/// Like the account expense pie on macOS: top-level net expenses, including children.
pub fn net_expenses(
    categories: &[girocrab_core::Category],
    uncategorized: (i64, usize),
    n: usize,
) -> Vec<Slice> {
    let slices = categories
        .iter()
        .filter(|c| c.depth == 0 && c.total_cents < 0)
        .map(|c| Slice {
            name: c.name.clone(),
            color_hex: c.color_hex.clone(),
            cents: -c.total_cents,
            count: c.transaction_count as usize,
            share: 0.0,
        })
        .collect();
    let mut slices = finish(slices, usize::MAX);
    let (uncategorized_cents, uncategorized_count) = uncategorized;
    let limit = n
        .saturating_sub(usize::from(uncategorized_cents > 0))
        .max(1);
    if n > 0 && slices.len() > limit {
        let rest = slices.split_off(limit - 1);
        slices.push(Slice {
            name: "Sonstige".into(),
            color_hex: "#8e8e93".into(),
            cents: rest.iter().map(|s| s.cents).sum(),
            count: rest.iter().map(|s| s.count).sum(),
            share: rest.iter().map(|s| s.share).sum(),
        });
    }
    if uncategorized_cents > 0 {
        let total = slices.iter().map(|s| s.cents).sum::<i64>() + uncategorized_cents;
        for slice in &mut slices {
            slice.share = slice.cents as f32 / total as f32;
        }
        slices.push(Slice {
            name: "Ohne Kategorie".into(),
            color_hex: "#94A3B8".into(),
            cents: uncategorized_cents,
            count: uncategorized_count,
            share: uncategorized_cents as f32 / total as f32,
        });
    }
    slices
}

/// One bar of a breakdown: name, summed cents, booking count, share of the total (0..=1).
pub struct Slice {
    pub name: String,
    pub color_hex: String,
    pub cents: i64,
    pub count: usize,
    pub share: f32,
}

fn finish(mut v: Vec<Slice>, n: usize) -> Vec<Slice> {
    v.sort_by_key(|s| std::cmp::Reverse(s.cents));
    let total: i64 = v.iter().map(|s| s.cents).sum();
    v.truncate(n);
    for s in &mut v {
        s.share = if total > 0 {
            s.cents as f32 / total as f32
        } else {
            0.0
        };
    }
    v
}

fn add(v: &mut Vec<Slice>, name: &str, color: &str, cents: i64) {
    match v.iter_mut().find(|s| s.name == name) {
        Some(s) => {
            s.cents += cents;
            s.count += 1;
        }
        None => v.push(Slice {
            name: name.to_string(),
            color_hex: color.to_string(),
            cents,
            count: 1,
            share: 0.0,
        }),
    }
}

/// Top categories of expenses (`expenses`) or income, internal transfers excluded.
pub fn categories(rows: &[&Row], expenses: bool, n: usize) -> Vec<Slice> {
    let mut v = Vec::new();
    for r in rows.iter().filter(|r| !r.transfer) {
        if (r.amount_cents < 0) != expenses || r.amount_cents == 0 {
            continue;
        }
        let name = if r.category_name.is_empty() {
            "Ohne Kategorie"
        } else {
            &r.category_name
        };
        let cents = if expenses {
            r.amount_cents.abs()
        } else {
            income(r)
        };
        if cents != 0 {
            add(&mut v, name, &r.category_color_hex, cents);
        }
    }
    finish(v, n)
}

/// Net sum (income − expenses, in cents) per tag; share is relative to the sum of absolute values.
pub fn tags(rows: &[&Row], n: usize) -> Vec<Slice> {
    let mut v = Vec::new();
    for r in rows.iter().filter(|r| !r.transfer) {
        for t in &r.tags {
            add(&mut v, &t.name, &t.color_hex, r.amount_cents);
        }
    }
    let total: i64 = v.iter().map(|s| s.cents.abs()).sum();
    v.sort_by_key(|s| std::cmp::Reverse(s.cents.abs()));
    v.truncate(n);
    for s in &mut v {
        s.share = if total > 0 {
            s.cents.abs() as f32 / total as f32
        } else {
            0.0
        };
    }
    v
}

/// What an incoming row counts as income: of a refund linked to its bookings only what none
/// of them takes up, the rest lowers the expenses (`net_expenses`, #714).
fn income(r: &Row) -> i64 {
    r.refund_income_cents
        .map_or(r.amount_cents, |rest| rest.clamp(0, r.amount_cents))
}

/// (income, expenses, net) cents of the rows, transfers excluded. What a linked refund pays
/// back lowers the expenses instead of being income (#714), so both sums cover the same rows.
pub fn totals(rows: &[&Row]) -> (i64, i64, i64) {
    let incoming = || rows.iter().filter(|r| !r.transfer && r.amount_cents > 0);
    let inc: i64 = incoming().map(|r| income(r)).sum();
    let paid_back: i64 = incoming().map(|r| r.amount_cents - income(r)).sum();
    let spent: i64 = rows
        .iter()
        .filter(|r| !r.transfer && r.amount_cents < 0)
        .map(|r| -r.amount_cents)
        .sum();
    let exp = spent - paid_back;
    (inc, exp, inc - exp)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(cents: i64, cat: &str, tags: &[&str]) -> Row {
        let mut r = crate::cockpit::tests::row("2026-11-01", cents, cat);
        r.tags = tags
            .iter()
            .enumerate()
            .map(|(i, n)| girocrab_core::tags::BookingTag {
                id: i as i64,
                name: n.to_string(),
                color_hex: "#00ff00".into(),
            })
            .collect();
        r
    }

    #[test]
    fn breakdown_and_tags() {
        let rows = [
            row(-300, "A", &["x"]),
            row(-100, "B", &["x", "y"]),
            row(500, "A", &[]),
        ];
        let refs: Vec<&Row> = rows.iter().collect();
        let c = categories(&refs, true, 5);
        assert_eq!(c[0].name, "A");
        assert!((c[0].share - 0.75).abs() < 1e-6);
        let t = tags(&refs, 5);
        assert_eq!(t[0].name, "x");
        assert_eq!(t[0].count, 2);
        assert_eq!(t[0].cents, -400);
        assert_eq!(totals(&refs), (500, 400, 100));
    }

    /// QA v12 (#6): a linked refund is income only for what no booking takes up.
    #[test]
    fn a_linked_refund_counts_as_income_only_for_its_unassigned_rest() {
        let mut full = row(10000, "A", &[]);
        full.refund_income_cents = Some(0);
        let mut part = row(5000, "B", &[]);
        part.refund_income_cents = Some(2000);
        let rows = [full, part, row(10200, "C", &[])];
        let refs: Vec<&Row> = rows.iter().collect();
        // what the refunds pay back (10000 + 3000) lowers the expenses of the same rows
        assert_eq!(totals(&refs), (12200, -13000, 25200));
        let spent = row(-20000, "A", &[]);
        let with_expense: Vec<&Row> = rows.iter().chain([&spent]).collect();
        assert_eq!(totals(&with_expense), (12200, 7000, 5200));
        let c = categories(&refs, false, 5);
        assert_eq!(
            c.iter()
                .map(|s| (s.name.as_str(), s.cents))
                .collect::<Vec<_>>(),
            [("C", 10200), ("B", 2000)]
        );
    }

    #[test]
    fn net_expenses_excludes_children_income_and_empty_categories_and_folds_rest() {
        let cat = |id, depth, cents| girocrab_core::Category {
            id,
            parent_id: None,
            name: format!("Kategorie {id}"),
            sf_symbol: String::new(),
            color_hex: "#123456".into(),
            sort_order: 0,
            depth,
            path: String::new(),
            own_total_cents: cents,
            total_cents: cents,
            transaction_count: 1,
            budget_cents: None,
        };
        let cats = [
            cat(1, 0, -600),
            cat(2, 1, -600),
            cat(3, 0, -300),
            cat(4, 0, -100),
            cat(5, 0, 50),
            cat(6, 0, 0),
        ];
        let slices = net_expenses(&cats, (0, 0), 2);
        assert_eq!(slices.len(), 2);
        assert_eq!(slices[0].cents, 600);
        assert_eq!(slices[1].name, "Sonstige");
        assert_eq!(slices[1].cents, 400);
        assert!((slices.iter().map(|s| s.share).sum::<f32>() - 1.0).abs() < 1e-6);
        assert!(net_expenses(&[], (0, 0), 6).is_empty());
        let slices = net_expenses(&cats, (20, 2), 3);
        assert_eq!(slices.len(), 3);
        assert_eq!(slices[2].name, "Ohne Kategorie");
        assert_eq!(slices[2].count, 2);
        assert!((slices[2].share - 20.0 / 1020.0).abs() < 1e-6);
        assert!((slices.iter().map(|s| s.share).sum::<f32>() - 1.0).abs() < 1e-6);
    }
}
