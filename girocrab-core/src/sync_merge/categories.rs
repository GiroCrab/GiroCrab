//! Category merge: newest edit per field wins for each UUID.

use std::collections::{BTreeMap, HashMap};

use rusqlite::{Connection, params};

use super::{fill_missing_uuids, merge_deletion, survivor, take_newer};
use crate::error::CoreResult;

#[derive(Clone, PartialEq, Debug)]
struct Cat {
    role: Option<String>,
    id: i64,
    uuid: String,
    name: String,
    parent: Option<String>,
    symbol: String,
    color: String,
    budget: Option<i64>,
    order: i64,
    name_ms: i64,
    parent_ms: i64,
    symbol_ms: i64,
    color_ms: i64,
    budget_ms: i64,
    order_ms: i64,
    deleted_ms: Option<i64>,
    merged_into: Option<String>,
}

impl Cat {
    fn merge(&mut self, o: Cat) {
        // A peer can clear built-in provenance when importing into the same
        // UUID. Preserve custom provenance so its name remains literal.
        merge_category_role(&mut self.role, o.role);
        take_newer(&mut self.name, &mut self.name_ms, o.name, o.name_ms);
        take_newer(&mut self.parent, &mut self.parent_ms, o.parent, o.parent_ms);
        take_newer(&mut self.symbol, &mut self.symbol_ms, o.symbol, o.symbol_ms);
        take_newer(&mut self.color, &mut self.color_ms, o.color, o.color_ms);
        take_newer(&mut self.budget, &mut self.budget_ms, o.budget, o.budget_ms);
        take_newer(&mut self.order, &mut self.order_ms, o.order, o.order_ms);
        merge_deletion(
            &mut self.deleted_ms,
            &mut self.merged_into,
            o.deleted_ms,
            o.merged_into,
        );
    }
}

pub(super) fn merge_category_role(role: &mut Option<String>, other: Option<String>) {
    if *role != other {
        // Only the two transfer roles share semantics. All other conflicts
        // become custom. This rule is commutative, associative and idempotent,
        // so three-way device merges converge regardless of their order.
        *role = match (role.as_deref(), other.as_deref()) {
            (Some("Umbuchungen"), Some("import:transfers"))
            | (Some("import:transfers"), Some("Umbuchungen")) => Some("import:transfers".into()),
            _ => None,
        };
    }
}

fn load_categories(conn: &Connection, schema: &str) -> CoreResult<Vec<Cat>> {
    // includes soft-deleted rows
    let sql = format!(
        "SELECT c.id, c.uuid, c.name, p.uuid, c.sf_symbol, c.color_hex, c.budget_cents,
                c.sort_order, c.name_ms, c.parent_ms, c.symbol_ms, c.color_ms, c.budget_ms,
                c.order_ms, c.deleted_ms, c.merged_into, c.builtin_role
         FROM {schema}.categories c LEFT JOIN {schema}.categories p ON p.id = c.parent_id
         ORDER BY c.id"
    );
    let rows = conn
        .prepare(&sql)?
        .query_map([], |r| {
            Ok(Cat {
                role: r.get(16)?,
                id: r.get(0)?,
                uuid: r.get(1)?,
                name: r.get(2)?,
                parent: r.get(3)?,
                symbol: r.get(4)?,
                color: r.get(5)?,
                budget: r.get(6)?,
                order: r.get(7)?,
                name_ms: r.get(8)?,
                parent_ms: r.get(9)?,
                symbol_ms: r.get(10)?,
                color_ms: r.get(11)?,
                budget_ms: r.get(12)?,
                order_ms: r.get(13)?,
                deleted_ms: r.get(14)?,
                merged_into: r.get(15)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

fn cat_survivor<'a>(state: &'a BTreeMap<String, Cat>, uuid: &'a str) -> Option<&'a str> {
    survivor(uuid, |u| {
        state
            .get(u)
            .map(|c| (c.deleted_ms.is_none(), c.merged_into.as_deref()))
    })
}

/// A parent that was combined into another moves its children to the
/// survivor; a live child of a deleted parent moves to the top level (a
/// deleted child keeps it). A cycle from crossing moves is cut at its oldest
/// move (smallest `parent_ms`, then uuid): that child goes to the top level.
/// Derived: no stamps.
fn resolve_parents(state: &mut BTreeMap<String, Cat>) {
    let all: Vec<String> = state.keys().cloned().collect();
    for u in &all {
        let c = &state[u];
        let parent = c
            .parent
            .as_deref()
            .and_then(|p| match cat_survivor(state, p) {
                Some(s) => Some(s),
                None if c.deleted_ms.is_some() => Some(p),
                None => None,
            })
            .filter(|p| *p != u.as_str())
            .map(str::to_string);
        state.get_mut(u).unwrap().parent = parent;
    }
    let live: Vec<String> = state
        .values()
        .filter(|c| c.deleted_ms.is_none())
        .map(|c| c.uuid.clone())
        .collect();
    loop {
        let mut cycle: Option<Vec<String>> = None;
        for u in &live {
            let mut seen = vec![u.clone()];
            let mut cur = state[u].parent.clone();
            while let Some(p) = cur {
                if p == *u {
                    cycle = Some(seen);
                    break;
                }
                if seen.len() > state.len() {
                    break;
                }
                cur = state.get(&p).and_then(|c| c.parent.clone());
                seen.push(p);
            }
            if cycle.is_some() {
                break;
            }
        }
        let Some(cycle) = cycle else { break };
        // the older move loses (ties: lower uuid); its stamp stays
        let oldest = cycle
            .iter()
            .min_by_key(|u| (state[*u].parent_ms, (*u).clone()))
            .unwrap()
            .clone();
        state.get_mut(&oldest).unwrap().parent = None;
    }
}

/// Merges `other.categories` into this vault: upsert by uuid, newest stamp per
/// field, deletions win. Returns other id -> local id (rows with an existing
/// redirect map to their survivor; a deleted row without a survivor maps to its
/// own deleted local row).
pub(crate) fn merge_categories(
    conn: &Connection,
    stats: &mut crate::vault::MergeStats,
) -> CoreResult<HashMap<i64, i64>> {
    fill_missing_uuids(conn)?;
    let local = load_categories(conn, "main")?;
    let other = load_categories(conn, "other")?;
    let original: HashMap<String, Cat> =
        local.iter().map(|c| (c.uuid.clone(), c.clone())).collect();
    let mut state: BTreeMap<String, Cat> =
        local.iter().map(|c| (c.uuid.clone(), c.clone())).collect();

    for o in &other {
        match state.get_mut(&o.uuid) {
            Some(c) => c.merge(o.clone()),
            None => {
                state.insert(o.uuid.clone(), Cat { id: 0, ..o.clone() });
            }
        }
    }
    resolve_parents(&mut state);

    // Allocate all local IDs before assigning parent edges.
    let mut ids: HashMap<String, i64> = state.values().map(|c| (c.uuid.clone(), c.id)).collect();
    let changed: Vec<&Cat> = state
        .values()
        .filter(|c| original.get(&c.uuid) != Some(c))
        .collect();
    for c in &changed {
        if c.id == 0 {
            conn.execute(
                "INSERT INTO categories(uuid, name) VALUES (?1, ?2)",
                params![c.uuid, c.name],
            )?;
            ids.insert(c.uuid.clone(), conn.last_insert_rowid());
            if c.deleted_ms.is_none() {
                stats.categories_added += 1;
            }
        }
    }
    for c in &changed {
        let parent = c.parent.as_ref().and_then(|p| ids.get(p));
        conn.execute(
            "UPDATE categories SET name = ?2, parent_id = ?3, sf_symbol = ?4, color_hex = ?5,
                    budget_cents = ?6, sort_order = ?7, name_ms = ?8, parent_ms = ?9,
                    symbol_ms = ?10, color_ms = ?11, budget_ms = ?12, order_ms = ?13,
                    deleted_ms = ?14, merged_into = ?15, builtin_role = ?16
             WHERE id = ?1",
            params![
                ids[&c.uuid],
                c.name,
                parent,
                c.symbol,
                c.color,
                c.budget,
                c.order,
                c.name_ms,
                c.parent_ms,
                c.symbol_ms,
                c.color_ms,
                c.budget_ms,
                c.order_ms,
                c.deleted_ms,
                c.merged_into,
                c.role
            ],
        )?;
    }

    // whatever points at a deleted category follows its survivor or becomes
    // uncategorised (as a deletion does); derived, no stamps. Redirects from
    // builds before v46, which combined same-named categories by themselves, are
    // followed too (#1833: edge case of mixed versions on three or more devices,
    // see `older_snapshot_redirect_is_followed`).
    for c in state.values().filter(|c| c.deleted_ms.is_some()) {
        let id = ids[&c.uuid];
        match cat_survivor(&state, &c.uuid) {
            Some(s) => {
                let s = ids[s];
                conn.execute(
                    "UPDATE transactions SET category_id = ?2 WHERE category_id = ?1",
                    [id, s],
                )?;
                conn.execute(
                    "UPDATE transaction_splits SET category_id = ?2 WHERE category_id = ?1",
                    [id, s],
                )?;
                conn.execute(
                    "UPDATE rules SET category_id = ?2 WHERE category_id = ?1",
                    [id, s],
                )?;
            }
            None => {
                conn.execute(
                    "UPDATE transactions SET category_id = NULL, rule_id = NULL, category_source = 'none'
                     WHERE category_id = ?1",
                    [id],
                )?;
                conn.execute(
                    "UPDATE transaction_splits SET category_id = NULL WHERE category_id = ?1",
                    [id],
                )?;
            }
        }
    }

    Ok(other
        .iter()
        .map(|o| {
            let target = cat_survivor(&state, &o.uuid).unwrap_or(o.uuid.as_str());
            (o.id, ids[target])
        })
        .collect())
}
