//! Presentation of the core's financial analytics, independent of list paging.

use girocrab_core::projection::{AnalyticsReport, CategoryFlowTotal, TagFlowTotal};
use girocrab_core::{BookingDirection, BookingFilter, CoreError, Vault};

use crate::daily::{Filter, StatusFilter};

pub struct Slice {
    pub name: String,
    pub color_hex: String,
    pub cents: i64,
    pub count: usize,
    pub share: f32,
}

fn empty() -> AnalyticsReport {
    AnalyticsReport {
        currency: "EUR".into(),
        monthly: Vec::new(),
        income_cents: 0,
        expense_cents: 0,
        net_cents: 0,
        category_expenses: Vec::new(),
        category_income: Vec::new(),
        tags: Vec::new(),
    }
}

pub fn load(
    vault: &Vault,
    mut filter: BookingFilter,
    ui: &Filter,
) -> Result<AnalyticsReport, CoreError> {
    filter.uncategorized_only |= ui.uncategorized_only;
    if !ui.search.is_empty() {
        filter.search = Some(ui.search.clone());
    }
    if !ui.from_date.is_empty() {
        filter.date_from = Some(ui.from_date.clone());
    }
    let checked = match ui.status {
        StatusFilter::Unchecked => Some(false),
        StatusFilter::Checked => Some(true),
        _ => None,
    };
    if let Some(checked) = checked {
        if filter.checked.is_some_and(|c| c != checked) {
            return Ok(empty());
        }
        filter.checked = Some(checked);
    }
    filter.direction = match ui.status {
        StatusFilter::Expenses => BookingDirection::Expense,
        StatusFilter::Income => BookingDirection::Income,
        _ => BookingDirection::All,
    };
    if !ui.tag.is_empty() {
        let tags = vault.list_tags()?;
        let Some(tag) = tags.iter().find(|t| t.name == ui.tag) else {
            return Ok(empty());
        };
        filter.tag_ids = vec![tag.id];
    }
    vault.analytics_report(filter, ui.with_documents)
}

fn finish(mut slices: Vec<Slice>, n: usize, absolute: bool) -> Vec<Slice> {
    let magnitude = |s: &Slice| {
        if absolute {
            i128::from(s.cents).abs()
        } else {
            i128::from(s.cents)
        }
    };
    let total: i128 = slices.iter().map(&magnitude).sum();
    slices.sort_by_key(|s| std::cmp::Reverse(magnitude(s)));
    slices.truncate(n);
    for s in &mut slices {
        s.share = if total > 0 {
            magnitude(s) as f32 / total as f32
        } else {
            0.0
        };
    }
    slices
}

pub fn categories(totals: &[CategoryFlowTotal], n: usize) -> Vec<Slice> {
    finish(
        totals
            .iter()
            .filter(|t| t.cents > 0)
            .map(|t| Slice {
                name: t.name.clone(),
                color_hex: t.color_hex.clone(),
                cents: t.cents,
                count: t.booking_count as usize,
                share: 0.0,
            })
            .collect(),
        n,
        false,
    )
}

pub fn tags(totals: &[TagFlowTotal], n: usize) -> Vec<Slice> {
    finish(
        totals
            .iter()
            .map(|t| Slice {
                name: t.name.clone(),
                color_hex: t.color_hex.clone(),
                cents: t.cents,
                count: t.booking_count as usize,
                share: 0.0,
            })
            .collect(),
        n,
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn charts_present_core_totals_without_reclassifying_credits() {
        let totals = vec![
            CategoryFlowTotal {
                category_id: Some(1),
                name: "A".into(),
                color_hex: "#ff0000".into(),
                cents: 300,
                booking_count: 2,
            },
            CategoryFlowTotal {
                category_id: Some(2),
                name: "B".into(),
                color_hex: "#00ff00".into(),
                cents: 100,
                booking_count: 1,
            },
            CategoryFlowTotal {
                category_id: Some(3),
                name: "Credit".into(),
                color_hex: "#0000ff".into(),
                cents: -50,
                booking_count: 1,
            },
        ];
        let c = categories(&totals, 5);
        assert_eq!(c[0].name, "A");
        assert_eq!(c[0].cents, 300);
        assert_eq!(c[0].count, 2);
        assert!((c[0].share - 0.75).abs() < 1e-6);
        assert_eq!(c.len(), 2);
        assert!(categories(&[], 5).is_empty());
        let t = tags(
            &[TagFlowTotal {
                tag_id: 1,
                name: "x".into(),
                color_hex: "#00ff00".into(),
                cents: -400,
                booking_count: 2,
            }],
            8,
        );
        assert_eq!(t[0].cents, -400);
        assert_eq!(t[0].count, 2);
        assert_eq!(t[0].share, 1.0);
        assert!(tags(&[], 8).is_empty());
    }
}
