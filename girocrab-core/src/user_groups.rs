//! User-defined account groups ("Kontogruppen") for the "Nach Gruppe"
//! sidebar view – e.g. "Privat", "Geschäft", "Kinder".
//!
//! Independent of the kind-based `account_groups` (the "Nach Kontoart" view),
//! which stay untouched. An account belongs to at most one user group
//! (`accounts.user_group_uuid`); accounts without one are listed under
//! "Ohne Gruppe".
//!
//! iCloud sync: groups are keyed by a random uuid. Every user edit stamps a
//! millisecond timestamp (`user_groups.updated_ms`, `accounts.placement_ms`,
//! `accounts.name_ms`); when merging two vaults the newer edit wins. Deleted
//! groups stay as tombstones (`deleted_ms`) so a merge can't bring them back.
//! Automatic placements use timestamp 0, so any explicit user choice wins.

use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection, OptionalExtension};

use crate::connectors::group_for;
use crate::error::{CoreError, CoreResult};
use crate::i18n::{tr, trf};
use crate::models::{Account, UserGroup};
use crate::vault::Vault;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS user_groups (
    uuid             TEXT    PRIMARY KEY,
    name             TEXT    NOT NULL,
    sf_symbol        TEXT    NOT NULL DEFAULT 'folder',
    sort_order       INTEGER NOT NULL DEFAULT 0,
    include_in_total INTEGER NOT NULL DEFAULT 1,
    updated_ms       INTEGER NOT NULL DEFAULT 0,
    deleted_ms       INTEGER                       -- tombstone, kept for sync
);";

/// New `accounts` columns (name, definition).
const ACCOUNT_COLUMNS: &[(&str, &str)] = &[
    ("user_group_uuid", "TEXT"),
    ("user_sort_order", "INTEGER NOT NULL DEFAULT 0"),
    ("placement_ms", "INTEGER NOT NULL DEFAULT 0"),
    ("name_ms", "INTEGER NOT NULL DEFAULT 0"),
];

/// Idempotent schema setup (also for brand-new vaults).
pub(crate) fn migrate(conn: &Connection) -> CoreResult<()> {
    conn.execute_batch(SCHEMA)?;
    for (col, def) in ACCOUNT_COLUMNS {
        let has: i64 = conn.query_row(
            "SELECT count(*) FROM pragma_table_info('accounts') WHERE name = ?1",
            [col],
            |r| r.get(0),
        )?;
        if has == 0 {
            conn.execute_batch(&format!("ALTER TABLE accounts ADD COLUMN {col} {def};"))?;
        }
    }
    Ok(())
}

pub(crate) fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn group_exists(conn: &Connection, uuid: &str) -> CoreResult<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM user_groups WHERE uuid = ?1 AND deleted_ms IS NULL",
            [uuid],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn not_found(uuid: &str) -> CoreError {
    CoreError::NotFound {
        what: trf("Gruppe {0}", &[&uuid]),
    }
}

/// Trimmed, non-empty name that no other live group uses (case-insensitive).
fn valid_name(conn: &Connection, name: &str, except: Option<&str>) -> CoreResult<String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(CoreError::InvalidInput {
            message: tr("Der Gruppenname darf nicht leer sein").into(),
        });
    }
    let clash: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM user_groups WHERE deleted_ms IS NULL AND lower(name) = lower(?1) AND uuid IS NOT ?2",
            params![name, except],
            |r| r.get(0),
        )
        .optional()?;
    if clash.is_some() {
        return Err(CoreError::InvalidInput {
            message: trf("Es gibt schon eine Gruppe „{0}“", &[&name]),
        });
    }
    Ok(name)
}

/// Finds a live group by name (case-insensitive) or creates it.
/// Returns `(uuid, created)`.
pub(crate) fn find_or_create(
    conn: &Connection,
    name: &str,
    symbol: &str,
) -> CoreResult<(String, bool)> {
    if let Some(uuid) = conn
        .query_row(
            "SELECT uuid FROM user_groups WHERE deleted_ms IS NULL AND lower(name) = lower(?1)",
            [name.trim()],
            |r| r.get::<_, String>(0),
        )
        .optional()?
    {
        return Ok((uuid, false));
    }
    let name = valid_name(conn, name, None)?;
    let uuid = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO user_groups(uuid, name, sf_symbol, sort_order, updated_ms)
         VALUES (?1, ?2, ?3, (SELECT IFNULL(MAX(sort_order), -1) + 1 FROM user_groups WHERE deleted_ms IS NULL), ?4)",
        params![uuid, name, symbol, now_ms()],
    )?;
    Ok((uuid, true))
}

/// Puts an account at the end of a group (`None` = "Ohne Gruppe").
/// `stamp` = 0 for automatic placements, so an explicit user choice wins on sync.
pub(crate) fn assign(
    conn: &Connection,
    account_id: i64,
    uuid: Option<&str>,
    stamp: i64,
) -> CoreResult<()> {
    conn.execute(
        "UPDATE accounts SET user_group_uuid = ?1,
                user_sort_order = (SELECT IFNULL(MAX(user_sort_order), -1) + 1 FROM accounts
                                   WHERE user_group_uuid IS ?1 AND id != ?2),
                placement_ms = ?3
         WHERE id = ?2",
        params![uuid, account_id, stamp],
    )?;
    Ok(())
}

/// A freshly created bank account joins the group most of its bank login's
/// other accounts are in (a family member's login stays with their group).
/// Otherwise it stays in "Ohne Gruppe".
pub(crate) fn place_new_account(conn: &Connection, account_id: i64) -> CoreResult<()> {
    let target: Option<String> = conn
        .query_row(
            "SELECT a.user_group_uuid FROM accounts a
               JOIN user_groups g ON g.uuid = a.user_group_uuid AND g.deleted_ms IS NULL
             WHERE a.id != ?1
               AND a.connection_id = (SELECT connection_id FROM accounts WHERE id = ?1)
             GROUP BY a.user_group_uuid ORDER BY count(*) DESC, MIN(a.id) LIMIT 1",
            [account_id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(uuid) = target {
        assign(conn, account_id, Some(&uuid), 0)?;
    }
    Ok(())
}

/// Merges user groups, account placements and account names from the
/// attached `other` vault (newest edit wins). `acc_map` maps other → local
/// account ids; accounts in `inserted` were just created by the merge and
/// take the other side's values unconditionally.
pub(crate) fn merge_attached(
    tx: &Connection,
    acc_map: &HashMap<i64, i64>,
    inserted: &HashSet<i64>,
) -> CoreResult<u32> {
    let mut changed = tx.execute(
        "INSERT INTO user_groups(uuid, name, sf_symbol, sort_order, include_in_total, updated_ms, deleted_ms)
         SELECT uuid, name, sf_symbol, sort_order, include_in_total, updated_ms, deleted_ms
         FROM other.user_groups WHERE true
         ON CONFLICT(uuid) DO UPDATE SET
            name = excluded.name, sf_symbol = excluded.sf_symbol, sort_order = excluded.sort_order,
            include_in_total = excluded.include_in_total, updated_ms = excluded.updated_ms,
            deleted_ms = excluded.deleted_ms
         WHERE excluded.updated_ms > user_groups.updated_ms",
        [],
    )? as u32;
    #[allow(clippy::type_complexity)]
    let rows: Vec<(i64, Option<String>, i64, i64, String, i64)> = tx
        .prepare("SELECT id, user_group_uuid, user_sort_order, placement_ms, name, name_ms FROM other.accounts")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?
        .collect::<Result<_, _>>()?;
    for (oid, uuid, order, placed, name, named) in rows {
        let Some(&id) = acc_map.get(&oid) else {
            continue;
        };
        let fresh = inserted.contains(&id);
        changed += tx.execute(
            "UPDATE accounts SET user_group_uuid = ?2, user_sort_order = ?3, placement_ms = ?4
             WHERE id = ?1 AND (?5 OR placement_ms < ?4)",
            params![id, uuid, order, placed, fresh],
        )? as u32;
        tx.execute(
            "UPDATE accounts SET name = ?2, name_ms = ?3 WHERE id = ?1 AND (?4 OR name_ms < ?3)",
            params![id, name, named, fresh],
        )?;
    }
    // Placements pointing at a group deleted on either side → "Ohne Gruppe".
    tx.execute(
        "UPDATE accounts SET user_group_uuid = NULL
         WHERE user_group_uuid IN (SELECT uuid FROM user_groups WHERE deleted_ms IS NOT NULL)",
        [],
    )?;
    Ok(changed)
}

/// Order of kinds inside a group: Giro → Karten → Depots → Darlehen.
fn kind_rank(a: &Account) -> i32 {
    group_for(a.kind).2
}

#[uniffi::export]
impl Vault {
    /// Sidebar data for the "Nach Gruppe" view: all user groups (also empty
    /// ones) in their order, then – if any account has no group – an
    /// "Ohne Gruppe" entry (`is_ungrouped`, empty uuid).
    pub fn list_user_groups(&self) -> Result<Vec<UserGroup>, CoreError> {
        let accounts = self.list_accounts()?;
        let order: HashMap<i64, i64> = {
            let db = self.db()?;
            let mut stmt = db.prepare("SELECT id, user_sort_order FROM accounts")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<Result<_, _>>()?
        };
        let groups: Vec<(String, String, String, i32, bool)> = {
            let db = self.db()?;
            let mut stmt = db.prepare(
                "SELECT uuid, name, sf_symbol, sort_order, include_in_total FROM user_groups
                 WHERE deleted_ms IS NULL ORDER BY sort_order, name COLLATE NOCASE",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?;
            rows.collect::<Result<_, _>>()?
        };
        let members = |uuid: Option<&str>| -> Vec<Account> {
            let mut v: Vec<Account> = accounts
                .iter()
                .filter(|a| a.user_group_uuid.as_deref() == uuid)
                .cloned()
                .collect();
            v.sort_by_key(|a| {
                (
                    order.get(&a.id).copied().unwrap_or(0),
                    kind_rank(a),
                    a.sort_order,
                    a.id,
                )
            });
            v
        };
        let sum = |v: &[Account]| {
            v.iter()
                .filter(|a| a.currency == "EUR")
                .map(|a| a.balance_cents)
                .sum()
        };
        let mut out: Vec<UserGroup> = groups
            .into_iter()
            .map(|(uuid, name, sf_symbol, sort_order, include_in_total)| {
                let accs = members(Some(&uuid));
                UserGroup {
                    balance_cents: sum(&accs),
                    uuid,
                    name,
                    sf_symbol,
                    sort_order,
                    include_in_total,
                    is_ungrouped: false,
                    accounts: accs,
                }
            })
            .collect();
        let rest = members(None);
        if !rest.is_empty() {
            out.push(UserGroup {
                uuid: String::new(),
                name: tr("Ohne Gruppe").into(),
                sf_symbol: "tray".into(),
                sort_order: i32::MAX,
                include_in_total: true,
                is_ungrouped: true,
                balance_cents: sum(&rest),
                accounts: rest,
            });
        }
        Ok(out)
    }

    /// Creates a group at the end of the list and returns its uuid.
    pub fn create_user_group(&self, name: String, sf_symbol: String) -> Result<String, CoreError> {
        let db = self.db()?;
        let name = valid_name(&db, &name, None)?;
        Ok(find_or_create(&db, &name, &sf_symbol)?.0)
    }

    /// Renames a group, changes its symbol and whether it counts towards the total.
    pub fn update_user_group(
        &self,
        uuid: String,
        name: String,
        sf_symbol: String,
        include_in_total: bool,
    ) -> Result<(), CoreError> {
        let db = self.db()?;
        if !group_exists(&db, &uuid)? {
            return Err(not_found(&uuid));
        }
        let name = valid_name(&db, &name, Some(&uuid))?;
        db.execute(
            "UPDATE user_groups SET name = ?2, sf_symbol = ?3, include_in_total = ?4, updated_ms = ?5 WHERE uuid = ?1",
            params![uuid, name, sf_symbol, include_in_total, now_ms()],
        )?;
        Ok(())
    }

    /// Deletes a group. Its accounts are kept and move to "Ohne Gruppe".
    pub fn delete_user_group(&self, uuid: String) -> Result<(), CoreError> {
        let db = self.db()?;
        if !group_exists(&db, &uuid)? {
            return Err(not_found(&uuid));
        }
        let now = now_ms();
        let tx = db.unchecked_transaction()?;
        tx.execute(
            "UPDATE user_groups SET deleted_ms = ?2, updated_ms = ?2 WHERE uuid = ?1",
            params![uuid, now],
        )?;
        let ids: Vec<i64> = tx
            .prepare(
                "SELECT id FROM accounts WHERE user_group_uuid = ?1 ORDER BY user_sort_order, id",
            )?
            .query_map([&uuid], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        for id in ids {
            assign(&tx, id, None, now)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Moves an account into a group (`None` = "Ohne Gruppe"), at `index`
    /// within that group (`None` = at the end).
    pub fn move_account_to_group(
        &self,
        account_id: i64,
        group_uuid: Option<String>,
        index: Option<u32>,
    ) -> Result<(), CoreError> {
        let db = self.db()?;
        if let Some(u) = &group_uuid {
            if !group_exists(&db, u)? {
                return Err(not_found(u));
            }
        }
        let mut ids: Vec<i64> = db
            .prepare(
                "SELECT id FROM accounts WHERE user_group_uuid IS ?1 AND id != ?2
                 ORDER BY user_sort_order, id",
            )?
            .query_map(params![group_uuid, account_id], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let at = index.map_or(ids.len(), |i| (i as usize).min(ids.len()));
        ids.insert(at, account_id);
        let tx = db.unchecked_transaction()?;
        let n = tx.execute(
            "UPDATE accounts SET user_group_uuid = ?2 WHERE id = ?1",
            params![account_id, group_uuid],
        )?;
        if n == 0 {
            return Err(CoreError::NotFound {
                what: trf("Konto {0}", &[&account_id]),
            });
        }
        renumber(&tx, &ids)?;
        tx.commit()?;
        Ok(())
    }

    /// Sets the order of all groups (uuids in the new order).
    pub fn reorder_user_groups(&self, uuids: Vec<String>) -> Result<(), CoreError> {
        let db = self.db()?;
        let now = now_ms();
        let tx = db.unchecked_transaction()?;
        for (i, u) in uuids.iter().enumerate() {
            tx.execute(
                "UPDATE user_groups SET sort_order = ?2, updated_ms = ?3
                 WHERE uuid = ?1 AND (sort_order != ?2)",
                params![u, i as i64, now],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Sets the order of the accounts inside one group (`None` = "Ohne Gruppe").
    /// Ids not in that group are ignored.
    pub fn reorder_accounts_in_group(
        &self,
        group_uuid: Option<String>,
        account_ids: Vec<i64>,
    ) -> Result<(), CoreError> {
        let db = self.db()?;
        let members: HashSet<i64> = db
            .prepare("SELECT id FROM accounts WHERE user_group_uuid IS ?1")?
            .query_map([&group_uuid], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let ids: Vec<i64> = account_ids
            .into_iter()
            .filter(|i| members.contains(i))
            .collect();
        let tx = db.unchecked_transaction()?;
        renumber(&tx, &ids)?;
        tx.commit()?;
        Ok(())
    }
}

/// Gives `ids` the sort orders 0, 1, 2 … and stamps changed placements.
fn renumber(tx: &Connection, ids: &[i64]) -> CoreResult<()> {
    let now = now_ms();
    for (i, id) in ids.iter().enumerate() {
        tx.execute(
            "UPDATE accounts SET user_sort_order = ?2, placement_ms = ?3 WHERE id = ?1",
            params![id, i as i64, now],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
