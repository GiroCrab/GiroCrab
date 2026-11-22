//! Category and tag edits through the local API (`set_category`, `set_tags`),
//! only while the user allows them (`ShareScope::allow_edits`).
//!
//! - Only bookings on shared accounts; any other id (unknown, deleted, on an
//!   unshared account) reads `not_found`, so the answer never tells which
//!   bookings exist elsewhere.
//! - A category set here counts as the user's own (`manual`, like a click in
//!   the app): rules no longer change it. A category the user set by hand is
//!   kept unless the request says `overwrite_manual`; one a rule set is
//!   replaced. Split bookings keep their parts' categories.
//! - Tags are added by name (created when missing) and removed by name.
//! - Every edit of a request runs in one transaction and stamps the booking
//!   like the app does, so the iCloud merge carries it to other devices.

use std::collections::{HashMap, HashSet};

use rusqlite::{params, params_from_iter};
use serde::{Deserialize, Serialize};

use super::{ApiError, ApiResult, MAX_PAGE, ShareScope, shared_account_ids};
use crate::error::CoreError;
use crate::vault::Vault;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApiTag {
    pub id: i64,
    pub name: String,
    /// Live bookings with this tag (all accounts).
    pub booking_count: u32,
}

/// The outcome for one requested booking.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApiEditResult {
    pub id: i64,
    /// `changed`, `unchanged`, `skipped_manual` (category set by the user, see
    /// `overwrite_manual`), `split` (the category belongs to the parts) or
    /// `not_found`.
    pub status: &'static str,
    /// Category and its source before the request (`set_category` only, not
    /// for `not_found`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_category_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_source: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApiCategoryEdit {
    pub category_id: Option<i64>,
    pub category_path: String,
    pub results: Vec<ApiEditResult>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApiTagEdit {
    /// Tags that did not exist and were created for `add`.
    pub created_tags: Vec<String>,
    pub results: Vec<ApiEditResult>,
}

impl ApiCategoryEdit {
    pub(super) fn changed_any(&self) -> bool {
        self.results.iter().any(|r| r.status == "changed")
    }
}

impl ApiTagEdit {
    pub(super) fn changed_any(&self) -> bool {
        !self.created_tags.is_empty() || self.results.iter().any(|r| r.status == "changed")
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct SetCategoryParams {
    transaction_ids: Vec<i64>,
    /// `null` (or missing) removes the category.
    category_id: Option<i64>,
    /// Also replace categories the user set by hand.
    overwrite_manual: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct SetTagsParams {
    transaction_ids: Vec<i64>,
    /// Tag names to add; a missing tag is created.
    add: Vec<String>,
    /// Tag names to remove; unknown names are ignored.
    remove: Vec<String>,
}

/// What an edit needs to know about a booking on a shared account.
struct Target {
    category_id: Option<i64>,
    source: &'static str,
    split: bool,
}

/// 1 to `MAX_PAGE` ids, each once, in request order.
fn check_ids(ids: Vec<i64>) -> ApiResult<Vec<i64>> {
    if ids.is_empty() {
        return Err(ApiError::invalid("transaction_ids must not be empty"));
    }
    if ids.len() > MAX_PAGE as usize {
        return Err(ApiError::invalid(format!(
            "at most {MAX_PAGE} transaction_ids per request"
        )));
    }
    let mut seen = HashSet::new();
    Ok(ids.into_iter().filter(|id| seen.insert(*id)).collect())
}

/// Live bookings among `ids` on the shared accounts.
fn targets(
    db: &rusqlite::Connection,
    scope: &ShareScope,
    ids: &[i64],
) -> ApiResult<HashMap<i64, Target>> {
    let shared: HashSet<i64> = shared_account_ids(db, scope)?.into_iter().collect();
    let ph = vec!["?"; ids.len()].join(",");
    let mut stmt = db
        .prepare(&format!(
            "SELECT t.id, t.account_id, t.category_id, t.category_source,
                    EXISTS (SELECT 1 FROM transaction_splits s WHERE s.transaction_id = t.id)
               FROM transactions t
              WHERE t.deleted_ms IS NULL AND t.id IN ({ph})"
        ))
        .map_err(CoreError::from)?;
    let rows = stmt
        .query_map(params_from_iter(ids), |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, bool>(4)?,
            ))
        })
        .map_err(CoreError::from)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(CoreError::from)?;
    Ok(rows
        .into_iter()
        .filter(|(_, account, ..)| shared.contains(account))
        .map(|(id, _, category_id, source, split)| {
            let source = match source.as_str() {
                "manual" => "manual",
                "rule" => "rule",
                _ => "none",
            };
            (
                id,
                Target {
                    category_id,
                    source,
                    split,
                },
            )
        })
        .collect())
}

fn not_found(id: i64) -> ApiEditResult {
    ApiEditResult {
        id,
        status: "not_found",
        previous_category_id: None,
        previous_source: None,
    }
}

impl Vault {
    /// All live tags, in list order.
    pub(super) fn api_tags(&self) -> ApiResult<Vec<ApiTag>> {
        Ok(self
            .list_tags()?
            .into_iter()
            .map(|t| ApiTag {
                id: t.id,
                name: t.name,
                booking_count: t.booking_count,
            })
            .collect())
    }

    pub(super) fn api_set_category(
        &self,
        scope: &ShareScope,
        p: SetCategoryParams,
    ) -> ApiResult<ApiCategoryEdit> {
        let ids = check_ids(p.transaction_ids)?;
        let db = self.db()?;
        let paths = Vault::category_paths(&db)?;
        let category_path = match p.category_id {
            None => String::new(),
            Some(id) => paths
                .get(&id)
                .map(|(_, path, _, _)| path.clone())
                .ok_or_else(|| ApiError::invalid(format!("unknown category: {id}")))?,
        };
        let targets = targets(&db, scope, &ids)?;
        let tx = db.unchecked_transaction().map_err(CoreError::from)?;
        let now = crate::dates::now_ms();
        let mut update = tx
            .prepare(
                "UPDATE transactions SET category_id = ?1, rule_id = NULL,
                        category_source = CASE WHEN ?1 IS NULL THEN 'none' ELSE 'manual' END,
                        category_ms = ?3
                  WHERE id = ?2",
            )
            .map_err(CoreError::from)?;
        let mut results = Vec::with_capacity(ids.len());
        for id in ids {
            let Some(t) = targets.get(&id) else {
                results.push(not_found(id));
                continue;
            };
            let status = if t.split {
                "split"
            } else if t.category_id == p.category_id {
                // Same category (or none): nothing to write, whoever set it.
                "unchanged"
            } else if t.source == "manual" && !p.overwrite_manual {
                "skipped_manual"
            } else {
                update
                    .execute(params![p.category_id, id, now])
                    .map_err(CoreError::from)?;
                "changed"
            };
            results.push(ApiEditResult {
                id,
                status,
                previous_category_id: t.category_id,
                previous_source: Some(t.source),
            });
        }
        drop(update);
        tx.commit().map_err(CoreError::from)?;
        Ok(ApiCategoryEdit {
            category_id: p.category_id,
            category_path,
            results,
        })
    }

    pub(super) fn api_set_tags(
        &self,
        scope: &ShareScope,
        p: SetTagsParams,
    ) -> ApiResult<ApiTagEdit> {
        let ids = check_ids(p.transaction_ids)?;
        if p.add.is_empty() && p.remove.is_empty() {
            return Err(ApiError::invalid("add or remove must name a tag"));
        }
        let db = self.db()?;
        let targets = targets(&db, scope, &ids)?;
        let live: Vec<i64> = ids
            .iter()
            .copied()
            .filter(|id| targets.contains_key(id))
            .collect();
        let tx = db.unchecked_transaction().map_err(CoreError::from)?;
        // A bad name (empty, too long) is the caller's mistake, not an internal error.
        let (created_tags, changed) = crate::tags::edit_tags_by_name(&tx, &live, &p.add, &p.remove)
            .map_err(|e| match e {
                CoreError::InvalidInput { message } => ApiError::invalid(message),
                other => ApiError::from(other),
            })?;
        tx.commit().map_err(CoreError::from)?;
        let results = ids
            .into_iter()
            .map(|id| {
                if !targets.contains_key(&id) {
                    return not_found(id);
                }
                ApiEditResult {
                    id,
                    status: if changed.contains(&id) {
                        "changed"
                    } else {
                        "unchanged"
                    },
                    previous_category_id: None,
                    previous_source: None,
                }
            })
            .collect();
        Ok(ApiTagEdit {
            created_tags,
            results,
        })
    }
}

#[cfg(test)]
mod tests;
