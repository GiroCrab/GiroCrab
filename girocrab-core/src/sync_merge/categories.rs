//! Category merge: newest edit per field wins for each UUID.

use std::collections::{BTreeMap, HashMap};

use rusqlite::{Connection, params};

use super::{fill_missing_uuids, merge_deletion, survivor, take_newer};
use crate::error::CoreResult;

#[derive(Clone, PartialEq, Debug)]
struct Cat {
    builtin_role: Option<String>,
    category_role: Option<String>,
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
    role_ms: i64,
    deleted_ms: Option<i64>,
    merged_into: Option<String>,
}

impl Cat {
    fn merge(&mut self, o: Cat) {
        // A peer can clear built-in provenance when importing into the same
        // UUID. Preserve custom provenance so its name remains literal.
        merge_category_role(&mut self.builtin_role, o.builtin_role);
        if self.role_ms == 0 && o.role_ms == 0 {
            // Nobody chose a role (stamp 0): each side holds the default schema step 44 or
            // an import derived from where the category was then. It goes with the parent
            // that wins, so a stale snapshot's default never lands on a newer position
            // (`Some` would otherwise beat `None` at the equal stamp). Same order as
            // `take_newer` on the parent; the role only breaks a full tie.
            if (o.parent_ms, &o.parent, &o.category_role)
                > (self.parent_ms, &self.parent, &self.category_role)
            {
                self.category_role = o.category_role;
            }
        } else {
            take_newer(
                &mut self.category_role,
                &mut self.role_ms,
                o.category_role,
                o.role_ms,
            );
        }
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
                c.order_ms, c.deleted_ms, c.merged_into, c.builtin_role, c.role, c.role_ms
         FROM {schema}.categories c LEFT JOIN {schema}.categories p ON p.id = c.parent_id
         ORDER BY c.id"
    );
    let rows = conn
        .prepare(&sql)?
        .query_map([], |r| {
            Ok(Cat {
                builtin_role: r.get(16)?,
                category_role: r.get(17)?,
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
                role_ms: r.get(18)?,
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

/// A category's role as stored, for [`role_winners`].
pub(crate) struct RoleEntry<'a> {
    pub uuid: &'a str,
    pub live: bool,
    pub merged_into: Option<&'a str>,
    pub role: Option<&'a str>,
    pub role_ms: i64,
}

/// The role that counts for each live category (#931): the newest of its own and the
/// chosen ones (`role_ms > 0`) of categories with an existing redirect (`merged_into`,
/// followed like the bookings, [`survivor`]), with the tie-break of [`take_newer`]. A role
/// nobody chose (stamp 0, seeded by schema step 44 or an import) described the duplicate's own
/// place and stays there. Resolved when reading, not written into the survivor on merge: a
/// written role would stay where an earlier merge put it, and the result would depend on
/// the order of syncs. Ordinary deletions count for nobody.
pub(crate) fn role_winners<'a>(rows: &[RoleEntry<'a>]) -> HashMap<&'a str, (Option<&'a str>, i64)> {
    let by_uuid: HashMap<&str, &RoleEntry> = rows.iter().map(|r| (r.uuid, r)).collect();
    let mut out: HashMap<&'a str, (Option<&'a str>, i64)> = rows
        .iter()
        .filter(|r| r.live)
        .map(|r| (r.uuid, (r.role, r.role_ms)))
        .collect();
    for r in rows
        .iter()
        .filter(|r| !r.live && r.merged_into.is_some() && r.role_ms > 0)
    {
        let Some(target) = survivor(r.uuid, |u| by_uuid.get(u).map(|e| (e.live, e.merged_into)))
        else {
            continue;
        };
        if let Some((role, ms)) = out.get_mut(target) {
            take_newer(role, ms, r.role, r.role_ms);
        }
    }
    out
}

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
                    deleted_ms = ?14, merged_into = ?15, builtin_role = ?16,
                    role = ?17, role_ms = ?18
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
                c.builtin_role,
                c.category_role,
                c.role_ms
            ],
        )?;
    }

    // whatever points at a deleted category follows its survivor or becomes
    // uncategorised (as a deletion does); derived, no stamps
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

#[cfg(test)]
mod forward_tests {
    use super::*;

    fn cat(uuid: &str, merged_into: Option<&str>, role: Option<&str>, role_ms: i64) -> Cat {
        Cat {
            builtin_role: None,
            category_role: role.map(str::to_string),
            id: 0,
            uuid: uuid.into(),
            name: uuid.into(),
            parent: None,
            symbol: String::new(),
            color: String::new(),
            budget: None,
            order: 0,
            name_ms: 0,
            parent_ms: 0,
            symbol_ms: 0,
            color_ms: 0,
            budget_ms: 0,
            order_ms: 0,
            role_ms,
            deleted_ms: merged_into.map(|_| 1),
            merged_into: merged_into.map(str::to_string),
        }
    }

    /// The roles that count, by uuid, as `Vault::explicit_roles` resolves them.
    fn winners(state: &BTreeMap<String, Cat>) -> BTreeMap<String, (Option<String>, i64)> {
        let rows: Vec<RoleEntry> = state
            .values()
            .map(|c| RoleEntry {
                uuid: &c.uuid,
                live: c.deleted_ms.is_none(),
                merged_into: c.merged_into.as_deref(),
                role: c.category_role.as_deref(),
                role_ms: c.role_ms,
            })
            .collect();
        role_winners(&rows)
            .into_iter()
            .map(|(u, (r, ms))| (u.to_string(), (r.map(str::to_string), ms)))
            .collect()
    }

    /// A chain of duplicate merges longer than 32 links still gives the role to the
    /// survivor bookings move to (`cat_survivor`).
    #[test]
    fn role_follows_the_survivor_bookings_use_on_a_long_chain() {
        let n = 40;
        let mut state = BTreeMap::new();
        for i in 0..n {
            let (role, ms) = if i == 0 {
                (Some("neutral"), 500)
            } else {
                (None, 0)
            };
            let next = format!("t{:02}", i + 1);
            let c = cat(&format!("t{i:02}"), Some(&next), role, ms);
            state.insert(c.uuid.clone(), c);
        }
        let live = cat(&format!("t{n:02}"), None, Some("consumption"), 100);
        state.insert(live.uuid.clone(), live);
        assert_eq!(
            cat_survivor(&state, "t00"),
            Some(format!("t{n:02}").as_str())
        );
        assert_eq!(
            winners(&state)[&format!("t{n:02}")],
            (Some("neutral".to_string()), 500)
        );
    }

    #[test]
    fn only_a_chosen_role_follows_an_existing_redirect() {
        let mut state = [
            cat("x", None, Some("investment"), 0),
            cat("y", Some("x"), Some("neutral"), 0),
        ]
        .into_iter()
        .map(|c| (c.uuid.clone(), c))
        .collect::<BTreeMap<_, _>>();

        assert_eq!(winners(&state)["x"], (Some("investment".into()), 0));
        state.get_mut("y").unwrap().role_ms = 7;
        assert_eq!(winners(&state)["x"], (Some("neutral".into()), 7));
    }

    /// Merges `other` into `local` as `merge_categories` does for a vault of the current
    /// schema: upsert by uuid (newest stamp per field), then [`resolve_parents`].
    fn merge_into(
        local: &BTreeMap<String, Cat>,
        other: &BTreeMap<String, Cat>,
    ) -> BTreeMap<String, Cat> {
        let mut state = local.clone();
        for o in other.values() {
            match state.get_mut(&o.uuid) {
                Some(c) => c.merge(o.clone()),
                None => {
                    state.insert(o.uuid.clone(), Cat { id: 0, ..o.clone() });
                }
            }
        }
        resolve_parents(&mut state);
        state
    }

    /// Effective role of every live category, as `Vault::category_roles` resolves it.
    fn effective(state: &BTreeMap<String, Cat>) -> BTreeMap<String, String> {
        let won = winners(state);
        state
            .values()
            .filter(|c| c.deleted_ms.is_none())
            .map(|c| {
                let mut cur = Some(c.uuid.as_str());
                let mut role = "consumption".to_string();
                for _ in 0..=32 {
                    let Some(n) = cur
                        .and_then(|u| state.get(u))
                        .filter(|n| n.deleted_ms.is_none())
                    else {
                        break;
                    };
                    if let Some(r) = &won[&n.uuid].0 {
                        role = r.clone();
                        break;
                    }
                    cur = n.parent.as_deref();
                }
                (c.uuid.clone(), role)
            })
            .collect()
    }

    /// Three devices start from one category tree whose roles nobody chose (stamp 0) and
    /// change it on their own: new categories (including same-named siblings),
    /// renames, moves, deletions and role choices. Device A then merges B and C in either
    /// order. Where both orders give the same tree, they must give the same roles.
    #[test]
    fn roles_do_not_depend_on_merge_order_on_random_states() {
        let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = move |n: u64| {
            seed ^= seed >> 12;
            seed ^= seed << 25;
            seed ^= seed >> 27;
            seed.wrapping_mul(0x2545_f491_4f6c_dd1d) % n
        };
        const NAMES: [&str; 3] = ["Kaution", "kaution", "Depot"];
        const ROLES: [Option<&str>; 4] = [None, Some("investment"), Some("neutral"), Some("loan")];
        let (mut same_tree, mut failures) = (0, Vec::new());
        for case in 0..3000 {
            let mut base = BTreeMap::new();
            for i in 0..4 {
                let mut c = cat(&format!("u{i}"), None, ROLES[next(4) as usize], 0);
                c.name = NAMES[next(3) as usize].into();
                c.parent = (i > 0 && next(2) == 0).then(|| format!("u{}", next(i)));
                base.insert(c.uuid.clone(), c);
            }
            resolve_parents(&mut base);
            let mut device = |dev: u64| {
                let mut state = base.clone();
                for k in 0..3 {
                    // stamps of a device's own edits are unique: no two devices edit in the same ms
                    let ms = (100 * (k + 1) + dev) as i64;
                    let mut uuids: Vec<String> = state.keys().cloned().collect();
                    let u = uuids[next(uuids.len() as u64) as usize].clone();
                    match next(5) {
                        0 => {
                            let mut c =
                                cat(&format!("d{dev}k{k}"), None, ROLES[next(4) as usize], 0);
                            c.name = NAMES[next(3) as usize].into();
                            c.name_ms = ms;
                            c.parent = (next(2) == 0).then(|| u.clone());
                            c.parent_ms = ms;
                            if c.category_role.is_some() && next(2) == 0 {
                                c.role_ms = ms;
                            }
                            uuids.push(c.uuid.clone());
                            state.insert(c.uuid.clone(), c);
                        }
                        1 => {
                            let c = state.get_mut(&u).unwrap();
                            c.name = NAMES[next(3) as usize].into();
                            c.name_ms = ms;
                        }
                        2 => {
                            let p = uuids[next(uuids.len() as u64) as usize].clone();
                            let c = state.get_mut(&u).unwrap();
                            c.parent = (p != u && next(3) > 0).then_some(p);
                            c.parent_ms = ms;
                        }
                        3 => {
                            let c = state.get_mut(&u).unwrap();
                            if c.deleted_ms.is_none() {
                                c.deleted_ms = Some(ms);
                            }
                        }
                        _ => {
                            let c = state.get_mut(&u).unwrap();
                            c.category_role = ROLES[next(4) as usize].map(str::to_string);
                            c.role_ms = ms;
                        }
                    }
                    resolve_parents(&mut state);
                }
                state
            };
            let (a, b, c) = (device(1), device(2), device(3));
            let b_first = merge_into(&merge_into(&a, &b), &c);
            let c_first = merge_into(&merge_into(&a, &c), &b);
            let tree =
                |s: &BTreeMap<String, Cat>| -> Vec<(String, Option<String>, Option<String>, bool)> {
                    s.values()
                        .map(|c| {
                            (
                                c.uuid.clone(),
                                c.parent.clone(),
                                c.merged_into.clone(),
                                c.deleted_ms.is_some(),
                            )
                        })
                        .collect()
                };
            if tree(&b_first) != tree(&c_first) {
                continue;
            }
            same_tree += 1;
            let roles = winners;
            if roles(&b_first) != roles(&c_first) || effective(&b_first) != effective(&c_first) {
                failures.push(format!(
                    "case {case}:\n  b first {:?} {:?}\n  c first {:?} {:?}\n  tree {:?}",
                    roles(&b_first),
                    effective(&b_first),
                    roles(&c_first),
                    effective(&c_first),
                    tree(&b_first)
                ));
            }
        }
        assert!(
            same_tree > 1500,
            "only {same_tree} of 3000 cases end with the same tree"
        );
        assert!(
            failures.is_empty(),
            "{} of {same_tree} cases depend on the merge order; first:\n{}",
            failures.len(),
            failures[0]
        );
    }
}
