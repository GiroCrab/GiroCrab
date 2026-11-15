//! Read-only local interface for external analysis tools (GitHub #136).
//!
//! A versioned JSON request/response layer over the open vault: accounts,
//! categories, bookings (keyset-paged) and period summaries. No transport here:
//! the app serves it while the vault is unlocked (local socket, MCP adapter).
//!
//! - Opt-in: off until the user stores an enabled [`ShareScope`]; the scope is
//!   read on every request, so revoking takes effect at once.
//! - Strictly read-only: the handler only knows the read methods below, nothing
//!   writes, no connection logins, credentials, keys or documents.
//! - Only the accounts in the scope are visible, also through search, loan views
//!   and sums. IBANs / account numbers and notes are off by default;
//!   counterparty and purpose can be switched off.
//!
//! The wire types (`Api*`) are their own, versioned structs: internal models
//! change between releases, `v1` stays stable.

use std::collections::{BTreeMap, HashSet};

use rusqlite::params_from_iter;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::CoreError;
use crate::merchants::FlowKind;
use crate::models::{kpi_side, AccountKind, Booking, BookingDirection, BookingFilter, KpiSide};
use crate::vault::Vault;

/// Version of the request/response format; a breaking change bumps it.
pub const API_VERSION: u32 = 1;

/// Settings key of the stored scope (JSON, inside the encrypted vault).
const SCOPE_SETTING: &str = "local_api.scope";

const DEFAULT_PAGE: u32 = 100;
const MAX_PAGE: u32 = 500;

/// What the user shares. Accounts are an explicit allow list: an account added
/// later is not shared until the user selects it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShareScope {
    pub enabled: bool,
    pub account_ids: Vec<i64>,
    /// Own and counterparty IBANs and account numbers; off = also masked inside texts.
    pub share_account_numbers: bool,
    pub share_counterparty: bool,
    /// Purpose and booking text.
    pub share_purpose: bool,
    pub share_notes: bool,
}

impl Default for ShareScope {
    fn default() -> Self {
        Self {
            enabled: false,
            account_ids: Vec::new(),
            share_account_numbers: false,
            share_counterparty: true,
            share_purpose: true,
            share_notes: false,
        }
    }
}

// MARK: - Wire types (v1)

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApiInfo {
    pub api_version: u32,
    pub core_version: String,
    pub account_count: u32,
    pub shares_account_numbers: bool,
    pub shares_counterparty: bool,
    pub shares_purpose: bool,
    pub shares_notes: bool,
    /// Newest successful bank sync of a shared account (Unix seconds).
    pub last_sync_unix: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApiAccount {
    pub id: i64,
    pub name: String,
    pub bank_name: String,
    /// `checking`, `savings`, `credit_card`, … (`AccountKind::as_str`).
    pub kind: String,
    pub currency: String,
    pub balance_cents: i64,
    pub balance_date: String,
    pub archived: bool,
    /// Last successful sync of the account's bank connection (Unix seconds).
    pub last_sync_unix: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iban: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_number: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApiCategory {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    /// Full path, segments joined with ` › `.
    pub path: String,
    pub depth: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApiTransaction {
    /// Stable for the life of the booking.
    pub id: i64,
    pub account_id: i64,
    pub booking_date: String,
    pub value_date: String,
    /// Negative = outflow.
    pub amount_cents: i64,
    pub currency: String,
    pub category_id: Option<i64>,
    pub category_path: String,
    /// Recognised merchant ("" = none).
    pub merchant: String,
    /// `consumption`, `income`, `investment`, `perk`, `transfer`, `loan`.
    pub flow: String,
    pub payment_method: String,
    /// Not booked yet: the bank can still change or drop it.
    pub pending: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counterparty: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counterparty_iban: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub booking_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApiTransactionPage {
    pub transactions: Vec<ApiTransaction>,
    /// Pass as `cursor` for the next (older) page; `None` = last page.
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApiCurrencyTotals {
    pub currency: String,
    /// Same definition as the app's income / expense tiles: transfers between
    /// own accounts are neutral, refunds reduce the expenses.
    pub income_cents: i64,
    /// Positive amount of money spent.
    pub expense_cents: i64,
    pub booking_count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApiCategoryTotal {
    pub category_id: Option<i64>,
    pub category_path: String,
    pub currency: String,
    /// Net sum (split bookings count with their parts).
    pub net_cents: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApiSummary {
    pub date_from: String,
    pub date_to: String,
    pub totals: Vec<ApiCurrencyTotals>,
    pub by_category: Vec<ApiCategoryTotal>,
}

// MARK: - Requests

#[derive(Debug, Deserialize)]
struct Request {
    #[serde(default)]
    v: Option<u32>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct TransactionParams {
    account_ids: Vec<i64>,
    date_from: Option<String>,
    date_to: Option<String>,
    category_id: Option<i64>,
    include_subcategories: Option<bool>,
    uncategorized_only: bool,
    /// `income`, `expense` or `all` (default).
    direction: Option<String>,
    search: Option<String>,
    cursor: Option<String>,
    limit: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct SummaryParams {
    account_ids: Vec<i64>,
    date_from: String,
    date_to: String,
}

/// Error codes of the response (`error.code`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiErrorCode {
    Disabled,
    UnsupportedVersion,
    UnknownMethod,
    InvalidParams,
    NotShared,
    Internal,
}

impl ApiErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ApiErrorCode::Disabled => "disabled",
            ApiErrorCode::UnsupportedVersion => "unsupported_version",
            ApiErrorCode::UnknownMethod => "unknown_method",
            ApiErrorCode::InvalidParams => "invalid_params",
            ApiErrorCode::NotShared => "not_shared",
            ApiErrorCode::Internal => "internal",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    pub code: ApiErrorCode,
    pub message: String,
}

impl ApiError {
    fn new(code: ApiErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self::new(ApiErrorCode::InvalidParams, message)
    }
}

impl From<CoreError> for ApiError {
    /// Vault errors stay inside the app: their text can name stored data.
    fn from(_: CoreError) -> Self {
        Self::new(ApiErrorCode::Internal, "internal error")
    }
}

type ApiResult<T> = Result<T, ApiError>;

// MARK: - Vault

impl Vault {
    /// The stored scope; default (disabled, nothing shared) when none is stored.
    pub fn local_api_scope(&self) -> Result<ShareScope, CoreError> {
        let stored = self.get_setting(SCOPE_SETTING.into())?;
        Ok(stored
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default())
    }

    pub fn set_local_api_scope(&self, scope: &ShareScope) -> Result<(), CoreError> {
        let json = serde_json::to_string(scope).map_err(|e| CoreError::InvalidInput {
            message: e.to_string(),
        })?;
        self.set_setting(SCOPE_SETTING.into(), json)
    }

    /// Answers one JSON request (`{"v":1,"method":"…","params":{…}}`) with
    /// `{"v":1,"result":…}` or `{"v":1,"error":{"code":"…","message":"…"}}`.
    pub fn local_api_request(&self, request: &str) -> String {
        let response = match self.local_api_dispatch(request) {
            Ok(result) => json!({ "v": API_VERSION, "result": result }),
            Err(e) => json!({
                "v": API_VERSION,
                "error": { "code": e.code.as_str(), "message": e.message },
            }),
        };
        response.to_string()
    }

    fn local_api_dispatch(&self, request: &str) -> ApiResult<Value> {
        let req: Request = serde_json::from_str(request)
            .map_err(|e| ApiError::invalid(format!("malformed request: {e}")))?;
        if req.v.is_some_and(|v| v != API_VERSION) {
            return Err(ApiError::new(
                ApiErrorCode::UnsupportedVersion,
                format!("supported version: {API_VERSION}"),
            ));
        }
        let scope = self.local_api_scope()?;
        if !scope.enabled {
            return Err(ApiError::new(
                ApiErrorCode::Disabled,
                "external access is turned off in GiroCrab",
            ));
        }
        fn params<T: for<'de> Deserialize<'de> + Default>(v: Value) -> ApiResult<T> {
            if v.is_null() {
                return Ok(T::default());
            }
            serde_json::from_value(v).map_err(|e| ApiError::invalid(e.to_string()))
        }
        let value = match req.method.as_str() {
            "info" => to_value(self.api_info(&scope)?),
            "accounts" => to_value(self.api_accounts(&scope)?),
            "categories" => to_value(self.api_categories()?),
            "transactions" => to_value(self.api_transactions(&scope, params(req.params)?)?),
            "summary" => to_value(self.api_summary(&scope, params(req.params)?)?),
            other => {
                return Err(ApiError::new(
                    ApiErrorCode::UnknownMethod,
                    format!("unknown method: {other}"),
                ))
            }
        };
        Ok(value)
    }

    /// Shared accounts that still exist, in sidebar order.
    fn shared_accounts(&self, scope: &ShareScope) -> ApiResult<Vec<ApiAccount>> {
        let wanted: HashSet<i64> = scope.account_ids.iter().copied().collect();
        let db = self.db()?;
        let mut stmt = db
            .prepare(
                "SELECT a.id, a.name, a.bank_name, a.kind, a.currency, a.balance_cents, a.balance_date,
                        a.archived_unix IS NOT NULL, c.last_sync_unix, a.iban, a.account_number
                   FROM accounts a
                   LEFT JOIN bank_connections c ON c.id = a.connection_id AND c.deleted_ms IS NULL
                  WHERE a.deleted_ms IS NULL
                  ORDER BY a.sort_order, a.id",
            )
            .map_err(CoreError::from)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(ApiAccount {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    bank_name: r.get(2)?,
                    kind: AccountKind::from_str(&r.get::<_, String>(3)?)
                        .as_str()
                        .to_string(),
                    currency: r.get(4)?,
                    balance_cents: r.get(5)?,
                    balance_date: r.get(6)?,
                    archived: r.get(7)?,
                    last_sync_unix: r.get(8)?,
                    iban: r.get::<_, Option<String>>(9)?,
                    account_number: r.get::<_, Option<String>>(10)?,
                })
            })
            .map_err(CoreError::from)?;
        let mut out = Vec::new();
        for row in rows {
            let mut a = row.map_err(CoreError::from)?;
            if !wanted.contains(&a.id) {
                continue;
            }
            if scope.share_account_numbers {
                a.iban = a.iban.filter(|s| !s.is_empty());
                a.account_number = a.account_number.filter(|s| !s.is_empty());
            } else {
                a.iban = None;
                a.account_number = None;
            }
            out.push(a);
        }
        Ok(out)
    }

    fn api_info(&self, scope: &ShareScope) -> ApiResult<ApiInfo> {
        let accounts = self.shared_accounts(scope)?;
        Ok(ApiInfo {
            api_version: API_VERSION,
            core_version: crate::core_version(),
            account_count: accounts.len() as u32,
            shares_account_numbers: scope.share_account_numbers,
            shares_counterparty: scope.share_counterparty,
            shares_purpose: scope.share_purpose,
            shares_notes: scope.share_notes,
            last_sync_unix: accounts.iter().filter_map(|a| a.last_sync_unix).max(),
        })
    }

    fn api_accounts(&self, scope: &ShareScope) -> ApiResult<Vec<ApiAccount>> {
        self.shared_accounts(scope)
    }

    /// The category tree. Category names are no account data and are shared whole.
    fn api_categories(&self) -> ApiResult<Vec<ApiCategory>> {
        let db = self.db()?;
        let paths = Vault::category_paths(&db)?;
        let mut stmt = db
            .prepare("SELECT id, parent_id FROM categories WHERE deleted_ms IS NULL")
            .map_err(CoreError::from)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?))
            })
            .map_err(CoreError::from)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(CoreError::from)?;
        let mut out: Vec<ApiCategory> = rows
            .into_iter()
            .filter_map(|(id, parent_id)| {
                let (name, path, _, _) = paths.get(&id)?;
                Some(ApiCategory {
                    id,
                    parent_id,
                    name: crate::i18n::display_name(name),
                    depth: path.matches(" › ").count() as u32,
                    path: crate::i18n::display_path(path),
                })
            })
            .collect();
        out.sort_by(|a, b| a.path.cmp(&b.path).then(a.id.cmp(&b.id)));
        Ok(out)
    }

    /// Requested accounts, all of them in the scope (an unknown id reads as not
    /// shared, so the answer never tells which accounts exist); empty = all shared.
    fn requested_accounts(&self, scope: &ShareScope, requested: &[i64]) -> ApiResult<Vec<i64>> {
        let shared: Vec<i64> = self
            .shared_accounts(scope)?
            .into_iter()
            .map(|a| a.id)
            .collect();
        if requested.is_empty() {
            return Ok(shared);
        }
        if let Some(id) = requested.iter().find(|id| !shared.contains(id)) {
            return Err(ApiError::new(
                ApiErrorCode::NotShared,
                format!("account {id} is not shared"),
            ));
        }
        Ok(requested.to_vec())
    }

    fn api_transactions(
        &self,
        scope: &ShareScope,
        p: TransactionParams,
    ) -> ApiResult<ApiTransactionPage> {
        let accounts = self.requested_accounts(scope, &p.account_ids)?;
        let search = check_search(scope, p.search)?;
        let direction = match p.direction.as_deref() {
            None | Some("all") => BookingDirection::All,
            Some("income") => BookingDirection::Income,
            Some("expense") => BookingDirection::Expense,
            Some(other) => return Err(ApiError::invalid(format!("unknown direction: {other}"))),
        };
        // Accounts only as the hard limit: v1 returns the bookings *on* an account,
        // not the app's loan view (bookings of other accounts that pay the loan).
        let filter = BookingFilter {
            category_id: p.category_id,
            include_subcategories: p.include_subcategories.unwrap_or(true),
            uncategorized_only: p.uncategorized_only,
            search,
            date_from: check_date(p.date_from, "date_from")?,
            date_to: check_date(p.date_to, "date_to")?,
            direction,
            ..Default::default()
        };
        let after = p.cursor.as_deref().map(parse_cursor).transpose()?;
        let limit = p.limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);
        // One extra row tells whether another page follows.
        let mut rows = self.booking_page_after(
            &filter,
            &accounts,
            after.as_ref().map(|(d, id)| (d.as_str(), *id)),
            limit + 1,
        )?;
        let next_cursor = if rows.len() > limit as usize {
            rows.truncate(limit as usize);
            rows.last().map(|t| format!("{}:{}", t.booking_date, t.id))
        } else {
            None
        };
        Ok(ApiTransactionPage {
            transactions: rows.iter().map(|t| api_transaction(scope, t)).collect(),
            next_cursor,
        })
    }

    fn api_summary(&self, scope: &ShareScope, p: SummaryParams) -> ApiResult<ApiSummary> {
        let accounts = self.requested_accounts(scope, &p.account_ids)?;
        let from = check_date(Some(p.date_from), "date_from")?
            .ok_or_else(|| ApiError::invalid("date_from is required"))?;
        let to = check_date(Some(p.date_to), "date_to")?
            .ok_or_else(|| ApiError::invalid("date_to is required"))?;
        if from > to {
            return Err(ApiError::invalid("date_from is after date_to"));
        }
        // Totals: the enriched bookings (flow decides the KPI side), like the
        // app's tiles and `cashflow_by_period`.
        let filter = BookingFilter {
            date_from: Some(from.clone()),
            date_to: Some(to.clone()),
            ..Default::default()
        };
        let mut totals: BTreeMap<String, ApiCurrencyTotals> = BTreeMap::new();
        for t in self.booking_page_after(&filter, &accounts, None, 0)? {
            let e = totals
                .entry(t.currency.clone())
                .or_insert_with(|| ApiCurrencyTotals {
                    currency: t.currency.clone(),
                    income_cents: 0,
                    expense_cents: 0,
                    booking_count: 0,
                });
            e.booking_count += 1;
            match kpi_side(t.flow, t.amount_cents) {
                KpiSide::Income => e.income_cents += t.amount_cents,
                KpiSide::Expense => e.expense_cents -= t.amount_cents,
                KpiSide::Neutral => {}
            }
        }
        // Per category: one split-aware SQL sum over the same bookings.
        let db = self.db()?;
        let paths = Vault::category_paths(&db)?;
        let ph = vec!["?"; accounts.len()].join(",");
        let sql = format!(
            "SELECT category_id, currency, SUM(amount_cents) FROM (
                SELECT t.category_id AS category_id, t.currency AS currency, t.amount_cents AS amount_cents
                  FROM transactions t
                 WHERE t.deleted_ms IS NULL AND t.account_id IN ({ph}) AND t.booking_date BETWEEN ? AND ?
                   AND NOT EXISTS (SELECT 1 FROM transaction_splits s WHERE s.transaction_id = t.id)
                UNION ALL
                SELECT s.category_id, t.currency, s.amount_cents
                  FROM transaction_splits s JOIN transactions t ON t.id = s.transaction_id
                 WHERE t.deleted_ms IS NULL AND t.account_id IN ({ph}) AND t.booking_date BETWEEN ? AND ?
             ) GROUP BY category_id, currency"
        );
        let mut args: Vec<rusqlite::types::Value> = Vec::new();
        for _ in 0..2 {
            args.extend(accounts.iter().map(|id| (*id).into()));
            args.push(from.clone().into());
            args.push(to.clone().into());
        }
        let by_category: Vec<ApiCategoryTotal> = if accounts.is_empty() {
            Vec::new()
        } else {
            let mut stmt = db.prepare(&sql).map_err(CoreError::from)?;
            let rows = stmt
                .query_map(params_from_iter(args.iter()), |r| {
                    Ok((
                        r.get::<_, Option<i64>>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                })
                .map_err(CoreError::from)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(CoreError::from)?;
            let mut out: Vec<ApiCategoryTotal> = rows
                .into_iter()
                .map(|(category_id, currency, net_cents)| ApiCategoryTotal {
                    category_path: category_id
                        .and_then(|c| paths.get(&c))
                        .map(|(_, path, _, _)| crate::i18n::display_path(path))
                        .unwrap_or_default(),
                    category_id,
                    currency,
                    net_cents,
                })
                .collect();
            out.sort_by(|a, b| {
                b.net_cents
                    .abs()
                    .cmp(&a.net_cents.abs())
                    .then_with(|| a.category_path.cmp(&b.category_path))
            });
            out
        };
        Ok(ApiSummary {
            date_from: from,
            date_to: to,
            totals: totals.into_values().collect(),
            by_category,
        })
    }
}

fn to_value<T: Serialize>(v: T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

fn flow_str(flow: FlowKind) -> &'static str {
    match flow {
        FlowKind::Consumption => "consumption",
        FlowKind::Income => "income",
        FlowKind::Investment => "investment",
        FlowKind::Perk => "perk",
        FlowKind::Transfer => "transfer",
        FlowKind::Loan => "loan",
    }
}

/// `YYYY-MM-DD` or nothing ("" = nothing).
fn check_date(date: Option<String>, name: &str) -> ApiResult<Option<String>> {
    match date.filter(|d| !d.is_empty()) {
        None => Ok(None),
        Some(d) => chrono::NaiveDate::parse_from_str(&d, "%Y-%m-%d")
            .map(|_| Some(d))
            .map_err(|_| ApiError::invalid(format!("{name} must be YYYY-MM-DD"))),
    }
}

/// `YYYY-MM-DD:<id>` as handed out in `next_cursor`.
fn parse_cursor(cursor: &str) -> ApiResult<(String, i64)> {
    let bad = || ApiError::invalid("invalid cursor");
    let (date, id) = cursor.split_once(':').ok_or_else(bad)?;
    let date = check_date(Some(date.to_string()), "cursor")
        .map_err(|_| bad())?
        .ok_or_else(bad)?;
    Ok((date, id.parse().map_err(|_| bad())?))
}

/// The booking search also matches fields the scope may hide (purpose,
/// counterparty, IBANs), so it is only allowed when its answer cannot reveal
/// them: text needs counterparty and purpose shared, a term with digits (which
/// also matches IBANs and account numbers) needs account numbers shared.
fn check_search(scope: &ShareScope, search: Option<String>) -> ApiResult<Option<String>> {
    let Some(q) = search.filter(|s| !s.trim().is_empty()) else {
        return Ok(None);
    };
    if !(scope.share_counterparty && scope.share_purpose) {
        return Err(ApiError::new(
            ApiErrorCode::NotShared,
            "search needs counterparty and purpose to be shared",
        ));
    }
    if !scope.share_account_numbers && q.chars().any(|c| c.is_ascii_digit()) {
        return Err(ApiError::new(
            ApiErrorCode::NotShared,
            "search terms with digits need account numbers to be shared",
        ));
    }
    Ok(Some(q))
}

/// Text field as shared: `None` when switched off, IBANs masked unless account
/// numbers are shared.
fn shared_text(scope: &ShareScope, on: bool, text: &str) -> Option<String> {
    if !on {
        return None;
    }
    if scope.share_account_numbers {
        Some(text.to_string())
    } else {
        Some(crate::sync_log::replace_ibans(text, |_| "[IBAN]".into()))
    }
}

fn api_transaction(scope: &ShareScope, t: &Booking) -> ApiTransaction {
    ApiTransaction {
        id: t.id,
        account_id: t.account_id,
        booking_date: t.booking_date.clone(),
        value_date: t.value_date.clone(),
        amount_cents: t.amount_cents,
        currency: t.currency.clone(),
        category_id: t.category_id,
        category_path: t.category_path.clone(),
        // The merchant is read from counterparty and purpose.
        merchant: if scope.share_counterparty && scope.share_purpose {
            t.merchant_name.clone()
        } else {
            String::new()
        },
        flow: flow_str(t.flow).into(),
        payment_method: t.payment_method.clone(),
        pending: t.is_pending,
        counterparty: shared_text(scope, scope.share_counterparty, &t.counterparty),
        counterparty_iban: Some(t.counterparty_iban.clone())
            .filter(|i| scope.share_account_numbers && !i.is_empty()),
        purpose: shared_text(scope, scope.share_purpose, &t.purpose),
        booking_text: shared_text(scope, scope.share_purpose, &t.booking_text),
        note: shared_text(scope, scope.share_notes, &t.note).filter(|n| !n.is_empty()),
    }
}

#[cfg(test)]
mod tests;
