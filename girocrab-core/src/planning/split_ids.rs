use std::collections::HashMap;

use rusqlite::{Connection, params};
use uuid::Uuid;

use crate::error::CoreResult;

pub(crate) fn migrate(conn: &Connection) -> CoreResult<()> {
    if !crate::schema::has_column(conn, "main", "transaction_splits", "uuid")? {
        backfill(conn)?;
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS split_tx ON transaction_splits(transaction_id);
         CREATE INDEX IF NOT EXISTS split_cat ON transaction_splits(category_id);
         CREATE UNIQUE INDEX IF NOT EXISTS split_uuid ON transaction_splits(uuid);
         CREATE TABLE IF NOT EXISTS refund_links (
            id INTEGER PRIMARY KEY,
            refund_id INTEGER NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
            refund_part TEXT NOT NULL DEFAULT '',
            original_id INTEGER NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
            original_part TEXT NOT NULL DEFAULT '',
            refund_cents INTEGER NOT NULL,
            original_cents INTEGER NOT NULL,
            created_ms INTEGER NOT NULL,
            changed_ms INTEGER NOT NULL,
            deleted_ms INTEGER,
            UNIQUE(refund_id, refund_part, original_id, original_part)
         );
         CREATE INDEX IF NOT EXISTS refund_links_original ON refund_links(original_id, deleted_ms);",
    )?;
    Ok(())
}

fn backfill(conn: &Connection) -> CoreResult<()> {
    let rows = conn
        .prepare(
            "SELECT s.id,s.transaction_id,s.category_id,s.amount_cents,s.note,c.uuid
         FROM transaction_splits s LEFT JOIN categories c ON c.id=s.category_id ORDER BY s.id",
        )?
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<String>>(5)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    conn.execute_batch(
        "CREATE TABLE transaction_splits_v38 (
            id INTEGER PRIMARY KEY,
            transaction_id INTEGER NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
            category_id INTEGER REFERENCES categories(id) ON DELETE SET NULL,
            amount_cents INTEGER NOT NULL,
            note TEXT NOT NULL DEFAULT '',
            uuid TEXT NOT NULL,
            position INTEGER NOT NULL
         );",
    )?;
    let mut identities = HashMap::new();
    let mut occurrences = HashMap::new();
    let mut positions = HashMap::new();
    for (id, booking, category, amount, note, category_uuid) in rows {
        if let std::collections::hash_map::Entry::Vacant(entry) = identities.entry(booking) {
            entry.insert(crate::sync_merge::booking_key(conn, booking)?);
        }
        let key = (
            identities[&booking].clone(),
            category_uuid,
            amount,
            note.clone(),
        );
        // Identical parts are indistinguishable; their UUID set is always 0..count,
        // regardless of where other content occurs in the stored row order.
        let occurrence = occurrences.entry(key.clone()).or_insert(0u64);
        let content =
            serde_json::to_vec(&("girocrab.split.v1", &key, *occurrence)).expect("split identity");
        let uuid = Uuid::new_v5(&Uuid::NAMESPACE_OID, &content).to_string();
        *occurrence += 1;
        let position = positions.entry(booking).or_insert(0i64);
        conn.execute(
            "INSERT INTO transaction_splits_v38(id,transaction_id,category_id,amount_cents,note,uuid,position)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![id, booking, category, amount, note, uuid, *position],
        )?;
        *position += 1;
    }
    conn.execute_batch(
        "DROP TABLE transaction_splits;
         ALTER TABLE transaction_splits_v38 RENAME TO transaction_splits;",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy(reverse: bool) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys=ON;
             CREATE TABLE bank_connections(id INTEGER PRIMARY KEY, connector TEXT, login_name TEXT, bank_code TEXT);
             CREATE TABLE accounts(id INTEGER PRIMARY KEY, connection_id INTEGER, external_ref TEXT);
             CREATE TABLE transactions(id INTEGER PRIMARY KEY, account_id INTEGER, external_id TEXT);
             CREATE TABLE categories(id INTEGER PRIMARY KEY, uuid TEXT);
             CREATE TABLE transaction_splits(id INTEGER PRIMARY KEY, transaction_id INTEGER NOT NULL REFERENCES transactions(id),
                category_id INTEGER REFERENCES categories(id), amount_cents INTEGER NOT NULL, note TEXT NOT NULL DEFAULT '');
             INSERT INTO accounts VALUES (1,NULL,'account');
             INSERT INTO transactions VALUES (1,1,'external');
             INSERT INTO categories VALUES (1,'category-a'),(2,'category-b');",
        ).unwrap();
        let mut parts = vec![
            (Some(1), -20, "a"),
            (Some(2), -30, "b"),
            (Some(1), -20, "a"),
            (None, 10, "credit"),
        ];
        if reverse {
            parts.reverse();
            conn.execute_batch("UPDATE accounts SET id=91; UPDATE transactions SET id=82,account_id=91; UPDATE categories SET id=id+10;").unwrap();
        }
        for (category, amount, note) in parts {
            conn.execute(
                "INSERT INTO transaction_splits(transaction_id,category_id,amount_cents,note) VALUES (?1,?2,?3,?4)",
                params![if reverse { 82 } else { 1 }, category.map(|c| if reverse { c+10 } else { c }), amount, note],
            ).unwrap();
        }
        conn
    }

    fn identities(conn: &Connection) -> Vec<(Option<String>, i64, String, String)> {
        conn.prepare("SELECT c.uuid,s.amount_cents,s.note,s.uuid FROM transaction_splits s
                      LEFT JOIN categories c ON c.id=s.category_id ORDER BY c.uuid,s.amount_cents,s.note,s.uuid")
            .unwrap().query_map([], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))
            .unwrap().collect::<Result<_,_>>().unwrap()
    }

    #[test]
    fn backfill_is_content_based_across_row_order_and_local_ids() {
        let a = legacy(false);
        let b = legacy(true);
        migrate(&a).unwrap();
        migrate(&b).unwrap();
        assert_eq!(identities(&a), identities(&b));
        let before = identities(&a);
        migrate(&a).unwrap();
        assert_eq!(identities(&a), before);
        let uuids: std::collections::HashSet<_> = identities(&a).into_iter().map(|r| r.3).collect();
        assert_eq!(uuids.len(), 4);
        assert!(
            uuids
                .iter()
                .all(|s| Uuid::parse_str(s).unwrap().get_version_num() == 5)
        );
        assert_eq!(
            a.query_row(
                "SELECT note FROM transaction_splits ORDER BY position LIMIT 1",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "a"
        );
        assert_eq!(
            b.query_row(
                "SELECT note FROM transaction_splits ORDER BY position LIMIT 1",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "credit"
        );
    }

    #[test]
    fn migrated_storage_requires_unique_non_null_uuids() {
        let conn = legacy(false);
        migrate(&conn).unwrap();
        assert!(conn.execute("INSERT INTO transaction_splits(transaction_id,amount_cents,position) VALUES (1,1,4)", []).is_err());
        assert!(conn.execute("INSERT INTO transaction_splits(transaction_id,amount_cents,position,uuid) VALUES (1,1,4,NULL)", []).is_err());
        assert!(conn.execute("INSERT INTO transaction_splits(transaction_id,amount_cents,position,uuid) SELECT 1,1,4,uuid FROM transaction_splits LIMIT 1", []).is_err());
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM refund_links", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
