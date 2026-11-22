//! Effective category and flow contributions. Bookings and user splits stay unchanged.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, OptionalExtension, params_from_iter};

use crate::error::{CoreError, CoreResult};
use crate::i18n::tr;
use crate::merchants::FlowKind;
use crate::models::*;
use crate::refunds::{Endpoint, Projection as Refunds};
use crate::vault::Vault;
use crate::vault::bookings::keeps;

mod reports;
pub use reports::{AnalyticsReport, PeriodCurrencyFlow, TagFlowTotal};

#[cfg(test)]
mod lint;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy)]
pub(crate) struct Row {
    pub booking_id: i64,
    pub category_id: Option<i64>,
    pub amount_cents: i64,
    pub flow: FlowKind,
    side: KpiSide,
}

pub(crate) struct Info {
    amount_cents: i64,
    pub account_id: i64,
    pub date: String,
    pub currency: String,
    merchant: Arc<str>,
    loan_account: Option<i64>,
}

type ScopedRows = HashMap<Option<Vec<i64>>, Arc<Vec<Row>>>;
type MatchingIds = HashMap<
    (
        Option<i64>,
        bool,
        bool,
        u8,
        Option<String>,
        Option<String>,
        Option<String>,
    ),
    String,
>;

pub(crate) struct Projection {
    base: Vec<Row>,
    pub info: HashMap<i64, Info>,
    parts: HashMap<Endpoint, (Option<i64>, FlowKind, KpiSide)>,
    parents: HashMap<i64, Option<i64>>,
    refunds: Arc<Refunds>,
    rows: Mutex<ScopedRows>,
    matching_ids: Mutex<MatchingIds>,
}

#[derive(Clone, Copy)]
pub(crate) enum CurrencyMode {
    Primary,
    Other,
    All,
}

fn primary_currency(db: &Connection, filter: &BookingFilter) -> CoreResult<String> {
    if let Some(account) = filter
        .account_ids
        .first()
        .filter(|id| filter.account_ids.iter().all(|other| other == *id))
    {
        let currency: Option<String> = db
            .query_row(
                "SELECT currency FROM accounts WHERE id=?1",
                [account],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(currency) = currency.filter(|c| !c.trim().is_empty()) {
            return Ok(currency.trim().to_ascii_uppercase());
        }
    }
    Ok("EUR".into())
}

#[derive(Clone, Copy)]
pub(crate) enum Side {
    Net,
    Expense,
    Income,
}

#[derive(Debug, Clone, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CategoryFlowTotal {
    pub category_id: Option<i64>,
    pub name: String,
    pub color_hex: String,
    pub cents: i64,
    pub booking_count: u32,
}

pub(crate) struct Selection {
    pub projection: Arc<Projection>,
    pub rows: Vec<Row>,
}

pub(crate) struct CategoryTotals {
    pub own: HashMap<Option<i64>, (i64, u32)>,
    pub subtree: HashMap<Option<i64>, (i64, u32)>,
}

#[derive(Default)]
pub(crate) struct Totals {
    pub income_cents: i64,
    pub expense_cents: i64,
    pub consumption_cents: i64,
    pub investment_cents: i64,
    pub perk_cents: i64,
    pub flow: SelectionFlow,
}

pub(crate) fn cents(value: i128) -> CoreResult<i64> {
    value
        .try_into()
        .map_err(|_| CoreError::invalid(tr("Die Teilbeträge sind zu groß.")))
}

pub(crate) fn spending(net: i64) -> CoreResult<i64> {
    Ok(cents(-i128::from(net))?.max(0))
}

impl Projection {
    fn load(v: &Vault, db: &Connection) -> CoreResult<Self> {
        let refunds = v.refund_projection(db)?;
        let mut out = Self {
            base: Vec::new(),
            info: HashMap::new(),
            parts: HashMap::new(),
            parents: db
                .prepare("SELECT id,parent_id FROM categories WHERE deleted_ms IS NULL")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<_, _>>()?,
            refunds: refunds.clone(),
            rows: Mutex::new(HashMap::new()),
            matching_ids: Mutex::new(HashMap::new()),
        };
        let finisher = v.booking_finisher(db)?;
        let live_categories = Vault::category_paths(db)?;
        v.visit_bookings(db, &BookingFilter::default(), |_, b| {
            out.info.insert(
                b.id,
                Info {
                    amount_cents: b.amount_cents,
                    account_id: b.account_id,
                    date: b.booking_date.clone(),
                    currency: if b.currency.trim().is_empty() {
                        "EUR".into()
                    } else {
                        b.currency.trim().to_ascii_uppercase()
                    },
                    merchant: b.merchant.key.clone().into(),
                    loan_account: finisher.loan_account(b.id),
                },
            );
            let Some(book) = refunds.books().get(&b.id) else {
                return;
            };
            if book.parts.is_empty() {
                let side = kpi_side(b.flow, b.amount_cents);
                let category_id = b.category_id.filter(|id| live_categories.contains_key(id));
                out.parts
                    .insert(Endpoint(b.id, String::new()), (category_id, b.flow, side));
                out.base.push(Row {
                    booking_id: b.id,
                    category_id,
                    amount_cents: b.amount_cents,
                    flow: b.flow,
                    side,
                });
            } else {
                for p in &book.parts {
                    let mut part = b.clone();
                    part.amount_cents = p.amount;
                    part.category_id = p.category_id;
                    finisher.finish(&mut part, None);
                    let side = kpi_side(part.flow, p.amount);
                    let category_id = p.category_id.filter(|id| live_categories.contains_key(id));
                    out.parts.insert(
                        Endpoint(b.id, p.uuid.clone()),
                        (category_id, part.flow, side),
                    );
                    out.base.push(Row {
                        booking_id: b.id,
                        category_id,
                        amount_cents: p.amount,
                        flow: part.flow,
                        side,
                    });
                }
            }
        })?;
        Ok(out)
    }

    fn rows(&self, visible: Option<&[i64]>) -> CoreResult<Arc<Vec<Row>>> {
        let key = visible.map(|v| {
            let mut v = v.to_vec();
            v.sort_unstable();
            v.dedup();
            v
        });
        if let Some(rows) = self.rows.lock().unwrap().get(&key) {
            return Ok(rows.clone());
        }
        let allowed = key
            .as_ref()
            .map(|v| v.iter().copied().collect::<HashSet<_>>());
        let allocations = match &allowed {
            Some(accounts) => self.refunds.scoped_allocations(accounts)?,
            None => self.refunds.allocations.clone(),
        };
        let mut rows: Vec<Row> = self
            .base
            .iter()
            .filter(|r| {
                allowed
                    .as_ref()
                    .is_none_or(|a| a.contains(&self.info[&r.booking_id].account_id))
            })
            .map(|r| {
                let mut r = *r;
                if r.flow == FlowKind::Loan
                    && allowed.as_ref().is_some_and(|a| {
                        self.info[&r.booking_id]
                            .loan_account
                            .is_some_and(|id| !a.contains(&id))
                    })
                {
                    r.flow = FlowKind::Transfer;
                    r.side = KpiSide::Neutral;
                }
                r
            })
            .collect();
        for a in allocations.iter().filter(|a| a.cents > 0) {
            let (Some(refund), Some(original)) = (&a.refund, &a.original) else {
                continue;
            };
            let (
                Some(&(source_category, source_flow, source_side)),
                Some(&(target_category, target_flow, target_side)),
            ) = (self.parts.get(refund), self.parts.get(original))
            else {
                continue;
            };
            rows.push(Row {
                booking_id: refund.0,
                category_id: source_category,
                amount_cents: -a.cents,
                flow: source_flow,
                side: source_side,
            });
            rows.push(Row {
                booking_id: refund.0,
                category_id: target_category,
                amount_cents: a.cents,
                flow: target_flow,
                side: if target_flow == FlowKind::Loan {
                    KpiSide::Neutral
                } else {
                    target_side
                },
            });
        }
        let mut combined: HashMap<(i64, Option<i64>, FlowKind, u8), (Row, i128)> = HashMap::new();
        for r in rows {
            combined
                .entry((r.booking_id, r.category_id, r.flow, r.side as u8))
                .or_insert((r, 0))
                .1 += i128::from(r.amount_cents);
        }
        let rows = Arc::new(
            combined
                .into_values()
                .filter(|(_, n)| *n != 0)
                .map(|(mut row, n)| {
                    row.amount_cents = cents(n)?;
                    Ok(row)
                })
                .collect::<CoreResult<Vec<_>>>()?,
        );
        self.rows.lock().unwrap().insert(key, rows.clone());
        Ok(rows)
    }
}

impl Vault {
    pub(crate) fn financial_projection(&self, db: &Connection) -> CoreResult<Arc<Projection>> {
        let stamp = self.data_stamp(db);
        if db.is_autocommit()
            && let Some(p) = self.projection.get(stamp)
        {
            return Ok(p);
        }
        let p = Arc::new(Projection::load(self, db)?);
        if db.is_autocommit() {
            self.projection.put(stamp, p.clone());
        }
        Ok(p)
    }

    pub(crate) fn projected_selection(
        &self,
        db: &Connection,
        filter: &BookingFilter,
        visible: Option<&[i64]>,
        currency: CurrencyMode,
    ) -> CoreResult<Selection> {
        self.projected_selection_impl(db, filter, visible, currency, false)
    }

    pub(crate) fn projected_booking_selection(
        &self,
        db: &Connection,
        filter: &BookingFilter,
        visible: Option<&[i64]>,
        currency: CurrencyMode,
    ) -> CoreResult<Selection> {
        self.projected_selection_impl(db, filter, visible, currency, true)
    }

    pub(crate) fn projected_booking_ids(
        &self,
        db: &Connection,
        filter: &BookingFilter,
    ) -> CoreResult<String> {
        let projection = self.financial_projection(db)?;
        let key = (
            filter.category_id,
            filter.include_subcategories,
            filter.uncategorized_only,
            filter.scope as u8,
            filter.merchant_key.clone(),
            filter.date_from.clone(),
            filter.date_to.clone(),
        );
        if let Some(ids) = projection.matching_ids.lock().unwrap().get(&key) {
            return Ok(ids.clone());
        }
        let mut categories = HashSet::new();
        if let Some(id) = filter.category_id {
            categories.insert(id);
            if filter.include_subcategories {
                for &child in projection.parents.keys() {
                    let mut current = Some(child);
                    let mut visited = HashSet::new();
                    while let Some(c) = current {
                        if !visited.insert(c) {
                            break;
                        }
                        if c == id {
                            categories.insert(child);
                            break;
                        }
                        current = projection.parents.get(&c).copied().flatten();
                    }
                }
            }
        }
        let ids: HashSet<i64> = projection
            .rows(None)?
            .iter()
            .filter(|r| {
                (filter.category_id.is_none()
                    || r.category_id.is_some_and(|c| categories.contains(&c)))
                    && (!filter.uncategorized_only || r.category_id.is_none())
                    && filter
                        .date_from
                        .as_ref()
                        .is_none_or(|date| projection.info[&r.booking_id].date >= *date)
                    && filter
                        .date_to
                        .as_ref()
                        .is_none_or(|date| projection.info[&r.booking_id].date <= *date)
                    && keeps(filter, r.flow, &projection.info[&r.booking_id].merchant)
            })
            .map(|r| r.booking_id)
            .collect();
        let ids = format!(
            "[{}]",
            ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",")
        );
        projection
            .matching_ids
            .lock()
            .unwrap()
            .insert(key, ids.clone());
        Ok(ids)
    }

    fn projected_selection_impl(
        &self,
        db: &Connection,
        filter: &BookingFilter,
        visible: Option<&[i64]>,
        currency: CurrencyMode,
        loan_view: bool,
    ) -> CoreResult<Selection> {
        let projection = self.financial_projection(db)?;
        let mut plain = filter.clone();
        plain.category_id = None;
        plain.uncategorized_only = false;
        plain.scope = BookingScope::All;
        plain.direction = BookingDirection::All;
        let (cte, wh, args, _) = self.filter_clauses(db, &plain)?;
        let unfiltered = self.filter_clauses(db, &BookingFilter::default())?.1;
        let ids: Option<HashSet<i64>> = if wh == unfiltered && args.is_empty() {
            None
        } else {
            Some(db.prepare(&format!("{cte}SELECT t.id FROM transactions t JOIN accounts a ON a.id=t.account_id LEFT JOIN categories cat ON cat.id=t.category_id AND cat.deleted_ms IS NULL WHERE {wh}"))?.query_map(params_from_iter(args.iter().map(|a|a.as_ref())),|r|r.get(0))?.collect::<Result<_,_>>()?)
        };
        let categories: Option<HashSet<i64>> = if let Some(id) = filter.category_id {
            if filter.include_subcategories {
                Some(db.prepare("WITH RECURSIVE sub(id) AS (SELECT ?1 UNION SELECT c.id FROM categories c JOIN sub ON c.parent_id=sub.id WHERE c.deleted_ms IS NULL) SELECT id FROM sub")?.query_map([id],|r|r.get(0))?.collect::<Result<_,_>>()?)
            } else {
                Some(HashSet::from([id]))
            }
        } else {
            None
        };
        let allowed = if filter.respect_in_total {
            Some(crate::user_groups::total_account_ids(db)?)
        } else {
            None
        };
        let primary_currency = primary_currency(db, filter)?;
        let rows = projection
            .rows(visible)?
            .iter()
            .filter(|r| {
                let info = &projection.info[&r.booking_id];
                let primary = info.currency.eq_ignore_ascii_case(&primary_currency);
                ids.as_ref().is_none_or(|ids| ids.contains(&r.booking_id))
                    && match filter.direction {
                        BookingDirection::All => true,
                        BookingDirection::Income => info.amount_cents > 0,
                        BookingDirection::Expense => info.amount_cents < 0,
                    }
                    && (loan_view
                        || filter.account_ids.is_empty()
                        || filter.account_ids.contains(&info.account_id))
                    && categories
                        .as_ref()
                        .is_none_or(|c| r.category_id.is_some_and(|id| c.contains(&id)))
                    && (!filter.uncategorized_only || r.category_id.is_none())
                    && keeps(filter, r.flow, &info.merchant)
                    && allowed
                        .as_ref()
                        .is_none_or(|a| a.contains(&info.account_id))
                    && match currency {
                        CurrencyMode::Primary => primary,
                        CurrencyMode::Other => !primary,
                        CurrencyMode::All => true,
                    }
            })
            .copied()
            .collect();
        Ok(Selection { projection, rows })
    }

    pub(crate) fn projected_totals(
        &self,
        db: &Connection,
        filter: &BookingFilter,
        visible: Option<&[i64]>,
        currency: CurrencyMode,
    ) -> CoreResult<Totals> {
        self.projected_booking_selection(db, filter, visible, currency)?
            .totals()
    }

    pub(crate) fn projected_categories(
        &self,
        db: &Connection,
        filter: &BookingFilter,
        side: Side,
    ) -> CoreResult<CategoryTotals> {
        let selection = self.projected_selection(db, filter, None, CurrencyMode::Primary)?;
        let parents = &selection.projection.parents;
        let mut sums: HashMap<Option<i64>, (i128, HashSet<i64>)> = HashMap::new();
        let mut subtree: HashMap<Option<i64>, (i128, HashSet<i64>)> = HashMap::new();
        for r in selection.rows {
            if !side.keeps(&r) {
                continue;
            }
            let e = sums.entry(r.category_id).or_default();
            e.0 += i128::from(r.amount_cents);
            e.1.insert(r.booking_id);
            let mut category = r.category_id;
            for _ in 0..32 {
                let e = subtree.entry(category).or_default();
                e.0 += i128::from(r.amount_cents);
                e.1.insert(r.booking_id);
                match category.and_then(|id| parents.get(&id).copied()).flatten() {
                    Some(parent) => category = Some(parent),
                    None => break,
                }
            }
        }
        let finish = |sums: HashMap<Option<i64>, (i128, HashSet<i64>)>| {
            sums.into_iter()
                .map(|(c, (n, ids))| {
                    if matches!(side, Side::Expense) {
                        cents(-n)?;
                    }
                    Ok((c, (cents(n)?, ids.len() as u32)))
                })
                .collect::<CoreResult<HashMap<_, _>>>()
        };
        Ok(CategoryTotals {
            own: finish(sums)?,
            subtree: finish(subtree)?,
        })
    }

    pub(crate) fn projected_category_flows(
        &self,
        filter: &BookingFilter,
        side: Side,
    ) -> CoreResult<Vec<CategoryFlowTotal>> {
        let db = self.db()?;
        self.projected_selection(&db, filter, None, CurrencyMode::Primary)?
            .category_flows(&db, side)
    }
}

impl Selection {
    fn category_flows(&self, db: &Connection, side: Side) -> CoreResult<Vec<CategoryFlowTotal>> {
        let parents = &self.projection.parents;
        let paths = Vault::category_paths(db)?;
        let mut sums: HashMap<Option<i64>, (i128, HashSet<i64>)> = HashMap::new();
        for r in &self.rows {
            if !side.keeps(r) {
                continue;
            }
            let mut category = r.category_id;
            for _ in 0..32 {
                match category.and_then(|id| parents.get(&id).copied()).flatten() {
                    Some(parent) => category = Some(parent),
                    None => break,
                }
            }
            let e = sums.entry(category).or_default();
            e.0 += i128::from(r.amount_cents);
            e.1.insert(r.booking_id);
        }
        let mut out = sums
            .into_iter()
            .map(|(category_id, (n, ids))| {
                let (name, color_hex) = category_id
                    .and_then(|id| paths.get(&id))
                    .map(|(name, _, color, _)| (name.clone(), color.clone()))
                    .unwrap_or_else(|| (tr("Ohne Kategorie").to_string(), "#8E8E93".into()));
                Ok(CategoryFlowTotal {
                    category_id,
                    name,
                    color_hex,
                    cents: cents(if matches!(side, Side::Expense) { -n } else { n })?,
                    booking_count: ids.len() as u32,
                })
            })
            .collect::<CoreResult<Vec<_>>>()?;
        out.sort_by(|a, b| {
            b.cents
                .cmp(&a.cents)
                .then_with(|| a.category_id.cmp(&b.category_id))
        });
        Ok(out)
    }
}

impl Side {
    fn keeps(self, r: &Row) -> bool {
        match self {
            Self::Net => true,
            Self::Expense => r.flow != FlowKind::Loan && r.side == KpiSide::Expense,
            Self::Income => r.side == KpiSide::Income,
        }
    }
}

impl Selection {
    pub fn monthly_flows(&self) -> CoreResult<BTreeMap<String, MonthFlow>> {
        let mut months: BTreeMap<String, Vec<Row>> = BTreeMap::new();
        for r in &self.rows {
            if let Some(m) = self.projection.info[&r.booking_id].date.get(..7) {
                months.entry(m.into()).or_default().push(*r);
            }
        }
        months
            .into_iter()
            .map(|(month, rows)| {
                let sums = Selection {
                    projection: self.projection.clone(),
                    rows,
                }
                .totals()?;
                Ok((
                    month.clone(),
                    MonthFlow {
                        month,
                        consumption_cents: sums.flow.consumption_cents,
                        investment_cents: sums.flow.invested_cents,
                        income_cents: sums.flow.income_cents,
                    },
                ))
            })
            .collect()
    }

    pub fn cashflow(
        &self,
        granularity: PeriodGranularity,
    ) -> CoreResult<HashMap<String, (i64, i64)>> {
        let mut sums: HashMap<String, (i128, i128)> = HashMap::new();
        for r in &self.rows {
            let date = &self.projection.info[&r.booking_id].date;
            let key = match granularity {
                PeriodGranularity::Month => date.get(..7).map(str::to_string),
                PeriodGranularity::Quarter => date.get(..4).map(|y| {
                    format!(
                        "{y}-Q{}",
                        (date
                            .get(5..7)
                            .and_then(|m| m.parse::<u32>().ok())
                            .unwrap_or(1)
                            .clamp(1, 12)
                            - 1)
                            / 3
                            + 1
                    )
                }),
                PeriodGranularity::Year => date.get(..4).map(str::to_string),
            };
            let Some(key) = key else { continue };
            let e = sums.entry(key).or_default();
            match r.side {
                KpiSide::Income => e.0 += i128::from(r.amount_cents),
                KpiSide::Expense => e.1 -= i128::from(r.amount_cents),
                KpiSide::Neutral => {}
            }
        }
        sums.into_iter()
            .map(|(k, (i, e))| Ok((k, (cents(i)?, cents(e)?))))
            .collect()
    }

    pub fn currency_totals(&self) -> CoreResult<Vec<(String, Totals)>> {
        let mut selections: BTreeMap<String, Vec<Row>> = BTreeMap::new();
        for r in &self.rows {
            selections
                .entry(self.projection.info[&r.booking_id].currency.clone())
                .or_default()
                .push(*r);
        }
        selections
            .into_iter()
            .map(|(currency, rows)| {
                Ok((
                    currency,
                    Selection {
                        projection: self.projection.clone(),
                        rows,
                    }
                    .totals()?,
                ))
            })
            .collect()
    }

    pub fn category_currencies(&self) -> CoreResult<Vec<(Option<i64>, String, i64)>> {
        self.category_currency_sums(false)
    }

    pub fn category_currency_subtrees(&self) -> CoreResult<Vec<(Option<i64>, String, i64)>> {
        self.category_currency_sums(true)
    }

    fn category_currency_sums(
        &self,
        ancestors: bool,
    ) -> CoreResult<Vec<(Option<i64>, String, i64)>> {
        let mut sums: BTreeMap<(Option<i64>, String), i128> = BTreeMap::new();
        for r in &self.rows {
            let currency = &self.projection.info[&r.booking_id].currency;
            let mut category = r.category_id;
            for _ in 0..32 {
                *sums.entry((category, currency.clone())).or_default() +=
                    i128::from(r.amount_cents);
                if !ancestors {
                    break;
                }
                match category
                    .and_then(|id| self.projection.parents.get(&id).copied())
                    .flatten()
                {
                    Some(parent) => category = Some(parent),
                    None => break,
                }
            }
        }
        sums.into_iter()
            .map(|((id, currency), n)| Ok((id, currency, cents(n)?)))
            .collect()
    }

    pub fn currency_amounts(&self) -> CoreResult<Vec<CurrencyAmount>> {
        let mut sums: BTreeMap<String, i128> = BTreeMap::new();
        for r in &self.rows {
            *sums
                .entry(self.projection.info[&r.booking_id].currency.clone())
                .or_default() += i128::from(r.amount_cents);
        }
        sums.into_iter()
            .map(|(currency, n)| {
                Ok(CurrencyAmount {
                    currency,
                    cents: cents(n)?,
                })
            })
            .collect()
    }

    pub fn totals(&self) -> CoreResult<Totals> {
        let (mut income, mut expense, mut consumption, mut investment, mut perk, mut invested) =
            (0i128, 0i128, 0i128, 0i128, 0i128, 0i128);
        for r in &self.rows {
            let n = i128::from(r.amount_cents);
            match r.side {
                KpiSide::Income => income += n,
                KpiSide::Expense => expense += n,
                KpiSide::Neutral => {}
            }
            match r.flow {
                FlowKind::Consumption => consumption += n,
                FlowKind::Investment => {
                    investment += n;
                    invested -= n;
                }
                FlowKind::Perk => {
                    perk += n;
                    if r.side == KpiSide::Expense {
                        invested -= n
                    }
                }
                _ => {}
            }
        }
        Ok(Totals {
            income_cents: cents(income)?,
            expense_cents: cents(expense)?,
            consumption_cents: cents(consumption)?,
            investment_cents: cents(investment)?,
            perk_cents: cents(perk)?,
            flow: SelectionFlow {
                income_cents: cents(income)?,
                consumption_cents: cents(-consumption)?,
                invested_cents: cents(invested)?,
            },
        })
    }

    pub fn shares(&self) -> CoreResult<Vec<(i64, i64)>> {
        let mut sums: HashMap<i64, i128> = HashMap::new();
        for r in &self.rows {
            *sums.entry(r.booking_id).or_default() += i128::from(r.amount_cents);
        }
        sums.into_iter()
            .map(|(id, n)| Ok((id, cents(n)?)))
            .collect()
    }

    pub fn history(&self, key_length: usize) -> CoreResult<Vec<CategoryPeriodSum>> {
        let mut sums: BTreeMap<String, i128> = BTreeMap::new();
        for r in &self.rows {
            if let Some(key) = self.projection.info[&r.booking_id].date.get(..key_length) {
                *sums.entry(key.into()).or_default() += i128::from(r.amount_cents);
            }
        }
        sums.into_iter()
            .map(|(period, n)| {
                Ok(CategoryPeriodSum {
                    period,
                    cents: cents(n)?,
                })
            })
            .collect()
    }

    pub fn currencies(&self) -> CoreResult<Vec<CurrencyFlow>> {
        let mut selections: BTreeMap<String, Vec<Row>> = BTreeMap::new();
        for r in &self.rows {
            selections
                .entry(self.projection.info[&r.booking_id].currency.clone())
                .or_default()
                .push(*r);
        }
        selections
            .into_iter()
            .map(|(currency, rows)| {
                let f = Selection {
                    projection: self.projection.clone(),
                    rows,
                }
                .totals()?
                .flow;
                Ok(CurrencyFlow {
                    currency,
                    consumption_cents: f.consumption_cents,
                    invested_cents: f.invested_cents,
                    income_cents: f.income_cents,
                })
            })
            .collect()
    }
}
