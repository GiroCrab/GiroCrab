use super::*;

#[derive(Debug, Clone, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TagFlowTotal {
    pub tag_id: i64,
    pub name: String,
    pub color_hex: String,
    pub cents: i64,
    pub booking_count: u32,
}

#[derive(Debug, Clone, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PeriodCurrencyFlow {
    pub period: String,
    pub currency: String,
    pub income_cents: i64,
    pub expense_cents: i64,
}

#[derive(Debug, Clone, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AnalyticsReport {
    pub currency: String,
    pub monthly: Vec<PeriodCurrencyFlow>,
    pub income_cents: i64,
    pub expense_cents: i64,
    pub net_cents: i64,
    pub category_expenses: Vec<CategoryFlowTotal>,
    pub category_income: Vec<CategoryFlowTotal>,
    pub tags: Vec<TagFlowTotal>,
}

#[uniffi::export]
impl Vault {
    /// Financial analytics for the whole filter, independent of booking list paging.
    pub fn analytics_report(
        &self,
        filter: BookingFilter,
        documents_only: bool,
    ) -> CoreResult<AnalyticsReport> {
        let db = self.db()?;
        let mut selection =
            self.projected_booking_selection(&db, &filter, None, CurrencyMode::All)?;
        if documents_only {
            let ids: HashSet<i64> = db
                .prepare("SELECT DISTINCT booking_id FROM booking_documents")?
                .query_map([], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            selection.rows.retain(|r| ids.contains(&r.booking_id));
        }
        let mut monthly: BTreeMap<(String, String), (i128, i128)> = BTreeMap::new();
        for r in &selection.rows {
            let info = &selection.projection.info[&r.booking_id];
            let Some(period) = info.date.get(..7) else {
                continue;
            };
            let n = monthly
                .entry((period.into(), info.currency.clone()))
                .or_default();
            if Side::Income.keeps(r) {
                n.0 += r.amount_cents;
            }
            if Side::Expense.keeps(r) {
                n.1 -= r.amount_cents;
            }
        }
        let monthly = monthly
            .into_iter()
            .map(|((period, currency), (income, expense))| {
                Ok(PeriodCurrencyFlow {
                    period,
                    currency,
                    income_cents: cents(income)?,
                    expense_cents: {
                        cents(-expense)?;
                        cents(expense)?
                    },
                })
            })
            .collect::<CoreResult<Vec<_>>>()?;
        let currency = primary_currency(&db, &filter)?;
        selection.rows.retain(|r| {
            selection.projection.info[&r.booking_id]
                .currency
                .eq_ignore_ascii_case(&currency)
        });
        let sums = selection.totals()?;
        let names: HashMap<i64, (String, String)> = db
            .prepare("SELECT id,name,color_hex FROM tags WHERE deleted_ms IS NULL")?
            .query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?
            .collect::<Result<_, _>>()?;
        let tags = crate::tags::totals_of(&db, selection.shares()?)?
            .into_iter()
            .filter_map(|t| {
                let tag_id = t.tag_id?;
                let (name, color_hex) = names.get(&tag_id)?.clone();
                Some(TagFlowTotal {
                    tag_id,
                    name,
                    color_hex,
                    cents: t.sum_cents,
                    booking_count: t.booking_count,
                })
            })
            .collect();
        let category_expenses = selection.category_flows(&db, Side::Expense)?;
        let expense_cents = cents(category_expenses.iter().map(|c| i128::from(c.cents)).sum())?;
        Ok(AnalyticsReport {
            currency,
            monthly,
            income_cents: sums.income_cents,
            expense_cents,
            net_cents: cents(i128::from(sums.income_cents) - i128::from(expense_cents))?,
            category_expenses,
            category_income: selection.category_flows(&db, Side::Income)?,
            tags,
        })
    }
}
