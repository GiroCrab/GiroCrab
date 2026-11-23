use rusqlite::{Connection, OptionalExtension, params};

use super::*;

pub(crate) fn migrate(conn: &Connection) -> CoreResult<()> {
    if !crate::schema::has_column(conn, "main", "refund_links", "moved")? {
        conn.execute_batch("ALTER TABLE refund_links ADD COLUMN moved INTEGER NOT NULL DEFAULT 0;
            ALTER TABLE refund_links ADD COLUMN moved_from_refund_id INTEGER REFERENCES transactions(id) ON DELETE SET NULL;
            ALTER TABLE refund_links ADD COLUMN moved_from_refund_part TEXT;
            ALTER TABLE refund_links ADD COLUMN moved_from_original_id INTEGER REFERENCES transactions(id) ON DELETE SET NULL;
            ALTER TABLE refund_links ADD COLUMN moved_from_original_part TEXT;
            ALTER TABLE refund_links ADD COLUMN moved_from_key TEXT NOT NULL DEFAULT '';")?;
    }
    Ok(())
}

pub(crate) fn migrate_unlink_ids(conn: &Connection) -> CoreResult<()> {
    if !crate::schema::has_column(conn, "main", "refund_links", "unlink_id")? {
        conn.execute_batch(
            "ALTER TABLE refund_links ADD COLUMN unlink_id TEXT NOT NULL DEFAULT '';",
        )?;
    }
    Ok(())
}

impl State {
    pub(super) fn load(conn: &Connection) -> CoreResult<Self> {
        let mut accounts = HashMap::new();
        let ids: Vec<i64> = conn
            .prepare("SELECT id FROM accounts")?
            .query_map([], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        for id in ids {
            accounts.insert(id, crate::sync_merge::account_key(conn, id)?);
        }
        let mut books:HashMap<i64,Book>=conn.prepare(
            "SELECT t.id,t.account_id,t.external_id,t.booking_date,t.amount_cents,
                    COALESCE(NULLIF(t.currency,''),NULLIF(a.currency,''),'EUR'),t.category_id,
                    t.counterparty,t.purpose,t.end_to_end_id,COALESCE(c.name,''),t.deleted_ms IS NULL AND a.deleted_ms IS NULL
             FROM transactions t JOIN accounts a ON a.id=t.account_id
             LEFT JOIN categories c ON c.id=t.category_id
             ")?
            .query_map([],|r| {
                let id=r.get(0)?;
                let account=r.get(1)?;
                let external:String=r.get(2)?;
                Ok((id,Book{id,live:r.get(11)?,account_id:account,identity:format!("{}|{external}",accounts[&account]),date:r.get(3)?,amount:r.get(4)?,
                    currency:r.get::<_,String>(5)?.to_ascii_uppercase(),category_id:r.get(6)?,counterparty:r.get(7)?,purpose:r.get(8)?,reference:r.get(9)?,label:r.get(10)?,parts:Vec::new()}))
            })?.collect::<Result<_,_>>()?;
        let mut stmt=conn.prepare("SELECT s.transaction_id,s.uuid,s.amount_cents,s.category_id,COALESCE(c.name,''),s.note FROM transaction_splits s LEFT JOIN categories c ON c.id=s.category_id ORDER BY s.position,s.id")?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            if let Some(b) = books.get_mut(&r.get::<_, i64>(0)?) {
                let category: String = r.get(4)?;
                let note: String = r.get(5)?;
                let label = if note.is_empty() {
                    category
                } else if category.is_empty() {
                    note
                } else {
                    format!("{category} · {note}")
                };
                b.parts.push(Part {
                    uuid: r.get(1)?,
                    amount: r.get(2)?,
                    category_id: r.get(3)?,
                    label,
                });
            }
        }
        Ok(Self {
            books,
            rows: load_rows(conn, "main")?,
        })
    }
}

fn load_rows(conn: &Connection, schema: &str) -> CoreResult<Vec<Row>> {
    Ok(conn.prepare(&format!("SELECT refund_id,refund_part,original_id,original_part,refund_cents,original_cents,created_ms,changed_ms,deleted_ms,moved,moved_from_refund_id,moved_from_refund_part,moved_from_original_id,moved_from_original_part,moved_from_key,unlink_id FROM {schema}.refund_links"))?
        .query_map([],|r| {
            let moved_from = match (r.get::<_,Option<i64>>(10)?,r.get::<_,Option<i64>>(12)?) {
                (Some(refund),Some(original)) => Some(Key { refund:Endpoint(refund,r.get(11)?),original:Endpoint(original,r.get(13)?) }),
                _ => None
            };
            Ok(Row{key:Key{refund:Endpoint(r.get(0)?,r.get(1)?),original:Endpoint(r.get(2)?,r.get(3)?)},refund_cents:r.get(4)?,original_cents:r.get(5)?,created_ms:r.get(6)?,changed_ms:r.get(7)?,deleted_ms:r.get(8)?,moved:r.get(9)?,moved_from,moved_from_key:r.get(14)?,unlink_id:r.get(15)?})
        })?.collect::<Result<_,_>>()?)
}

pub(super) fn write(conn: &Connection, row: &Row) -> CoreResult<()> {
    let source = row.moved_from.as_ref();
    conn.execute("INSERT INTO refund_links(refund_id,refund_part,original_id,original_part,refund_cents,original_cents,created_ms,changed_ms,deleted_ms,moved,moved_from_refund_id,moved_from_refund_part,moved_from_original_id,moved_from_original_part,moved_from_key,unlink_id)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
         ON CONFLICT(refund_id,refund_part,original_id,original_part) DO UPDATE SET
         refund_cents=excluded.refund_cents,original_cents=excluded.original_cents,created_ms=excluded.created_ms,changed_ms=excluded.changed_ms,deleted_ms=excluded.deleted_ms,
         moved=excluded.moved,moved_from_refund_id=excluded.moved_from_refund_id,moved_from_refund_part=excluded.moved_from_refund_part,moved_from_original_id=excluded.moved_from_original_id,moved_from_original_part=excluded.moved_from_original_part,moved_from_key=excluded.moved_from_key,unlink_id=excluded.unlink_id",
         params![row.key.refund.0,row.key.refund.1,row.key.original.0,row.key.original.1,row.refund_cents,row.original_cents,row.created_ms,row.changed_ms,row.deleted_ms,row.moved,
            source.map(|k|k.refund.0),source.map(|k|k.refund.1.as_str()),source.map(|k|k.original.0),source.map(|k|k.original.1.as_str()),row.moved_from_key,row.unlink_id])?;
    Ok(())
}

pub(super) fn next_stamp(conn: &Connection) -> CoreResult<i64> {
    let last: i64 = conn.query_row(
        "SELECT MAX(MAX(created_ms,changed_ms,IFNULL(deleted_ms,0))) FROM refund_links",
        [],
        |r| Ok(r.get::<_, Option<i64>>(0)?.unwrap_or(0)),
    )?;
    Ok(crate::dates::now_ms().max(last.checked_add(1).ok_or_else(invalid)?))
}

/// Plain full-row LWW. No resolver, allocation, timestamps or booking-state cleanup.
pub(crate) fn merge_attached(conn: &Connection, bookings: &HashMap<i64, i64>) -> CoreResult<()> {
    merge_mapped(conn, bookings, false)
}

pub(crate) fn merge_folded(conn: &Connection, bookings: &HashMap<i64, i64>) -> CoreResult<()> {
    merge_mapped(conn, bookings, true)
}

fn merge_mapped(conn: &Connection, bookings: &HashMap<i64, i64>, folded: bool) -> CoreResult<()> {
    let remap = |mut key: Key| {
        key.refund.0 = *bookings.get(&key.refund.0)?;
        key.original.0 = *bookings.get(&key.original.0)?;
        Some(key)
    };
    let mut rows = Vec::new();
    let mut direct = Vec::new();
    let mut remapped = HashSet::new();
    for mut row in load_rows(conn, "other")? {
        let source = row.key.clone();
        let Some(mapped) = remap(row.key.clone()) else {
            continue;
        };
        row.key = mapped;
        row.moved_from = row.moved_from.and_then(remap);
        if folded && row.deleted_ms.is_none() {
            let previous = row.key.clone();
            for (old, target) in [
                (&source.refund, &mut row.key.refund),
                (&source.original, &mut row.key.original),
            ] {
                if old.1.is_empty()
                    && is_split(conn, "other", old.0)?
                    && !is_split(conn, "main", target.0)?
                {
                    *target = old.clone();
                    continue;
                }
                if old.0 != target.0
                    && let Some(part) = survivor_part(conn, "other", old, target.0, true)?
                {
                    target.1 = part;
                }
            }
            if previous != row.key {
                remapped.insert(row.key.clone());
                rows.push(retire_part_key(conn, &mut row, previous)?);
            } else {
                direct.push(row);
                continue;
            }
        }
        rows.push(row);
    }
    if folded {
        let occupied: HashSet<_> = load_rows(conn, "main")?
            .into_iter()
            .chain(direct.iter().cloned())
            .filter(|r| r.deleted_ms.is_none())
            .map(|r| r.key)
            .collect();
        rows.retain(|r| {
            !(r.deleted_ms.is_none() && remapped.contains(&r.key) && occupied.contains(&r.key))
        });
        inactivate_collisions(&mut rows);
    }
    rows.extend(direct);
    merge_rows(conn, rows)
}

fn inactivate_collisions(rows: &mut Vec<Row>) {
    let mut counts = HashMap::new();
    for row in rows.iter().filter(|r| r.deleted_ms.is_none()) {
        *counts.entry(row.key.clone()).or_insert(0usize) += 1;
    }
    let sources: HashSet<_> = rows
        .iter()
        .filter(|r| r.deleted_ms.is_none() && counts[&r.key] > 1)
        .filter_map(|r| r.moved_from.clone())
        .collect();
    rows.retain(|r| !(r.deleted_ms.is_none() && counts[&r.key] > 1 && r.moved_from.is_some()));
    for row in rows
        .iter_mut()
        .filter(|r| sources.contains(&r.key) && r.moved)
    {
        row.deleted_ms = None;
        row.moved = false;
    }
}

fn survivor_part(
    conn: &Connection,
    schema: &str,
    source: &Endpoint,
    survivor: i64,
    keep_exact: bool,
) -> CoreResult<Option<String>> {
    if source.1.is_empty()
        && !(schema == "main" && source.0 == survivor)
        && is_split(conn, schema, source.0)?
    {
        return Ok(None);
    }
    let parts = conn
        .prepare_cached(
            "SELECT uuid,amount_cents FROM main.transaction_splits WHERE transaction_id=?1
        UNION ALL SELECT '',amount_cents FROM main.transactions WHERE id=?1
        AND NOT EXISTS(SELECT 1 FROM main.transaction_splits WHERE transaction_id=?1)",
        )?
        .query_map([survivor], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    if keep_exact && parts.iter().any(|(uuid, _)| *uuid == source.1) {
        return Ok(Some(source.1.clone()));
    }
    let amount: Option<i64> = if source.1.is_empty() {
        conn.query_row(
            &format!("SELECT amount_cents FROM {schema}.transactions WHERE id=?1"),
            [source.0],
            |r| r.get(0),
        )
        .optional()?
    } else {
        conn.query_row(&format!("SELECT amount_cents FROM {schema}.transaction_splits WHERE transaction_id=?1 AND uuid=?2"), params![source.0,source.1], |r| r.get(0)).optional()?
    };
    let Some(amount) = amount.filter(|n| *n != 0) else {
        return Ok(None);
    };
    let mut matches = parts.into_iter().filter(|(_, cents)| *cents == amount);
    let first = matches.next();
    Ok(first
        .filter(|_| matches.next().is_none())
        .map(|(uuid, _)| uuid))
}

fn is_split(conn: &Connection, schema: &str, id: i64) -> CoreResult<bool> {
    Ok(conn.query_row(
        &format!(
            "SELECT EXISTS(SELECT 1 FROM {schema}.transaction_splits WHERE transaction_id=?1)"
        ),
        [id],
        |r| r.get(0),
    )?)
}

fn retire_part_key(conn: &Connection, row: &mut Row, previous: Key) -> CoreResult<Row> {
    let mut retired = row.clone();
    retired.key = previous.clone();
    retired.deleted_ms = Some(row.changed_ms);
    retired.moved = true;
    row.moved_from_key = source_key(conn, &previous)?;
    row.moved_from = Some(previous);
    Ok(retired)
}

pub(crate) fn move_booking(conn: &Connection, from: i64, to: i64) -> CoreResult<()> {
    let affected: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM refund_links WHERE deleted_ms IS NULL AND (refund_id=?1 OR original_id=?1))",
        [from], |r| r.get(0),
    )?;
    if !affected {
        return Ok(());
    }
    let mut endpoints = HashMap::new();
    for row in load_rows(conn, "main")? {
        for e in [row.key.refund, row.key.original] {
            if e.0 == from {
                if from != to && e.1.is_empty() && is_split(conn, "main", from)? {
                    continue;
                }
                let part =
                    survivor_part(conn, "main", &e, to, true)?.unwrap_or_else(|| e.1.clone());
                endpoints.insert(e, Endpoint(to, part));
            }
        }
    }
    move_endpoints(conn, &endpoints)
}

pub(crate) fn fold_booking(
    conn: &Connection,
    from: i64,
    to: i64,
    keep_live: bool,
    source_had_splits: bool,
) -> CoreResult<()> {
    let remap = |key: &mut Key| {
        if key.refund.0 == from {
            key.refund.0 = to;
        }
        if key.original.0 == from {
            key.original.0 = to;
        }
    };
    let mut folded = Vec::new();
    let mut direct = Vec::new();
    for row in load_rows(conn, "main")? {
        let mut target = row.clone();
        let mut retargeted = false;
        if keep_live || target.deleted_ms.is_some() {
            remap(&mut target.key);
        }
        if target.deleted_ms.is_none() && source_had_splits && !is_split(conn, "main", to)? {
            if row.key.refund.0 == from && row.key.refund.1.is_empty() {
                target.key.refund = row.key.refund.clone();
            }
            if row.key.original.0 == from && row.key.original.1.is_empty() {
                target.key.original = row.key.original.clone();
            }
        }
        if let Some(source) = &mut target.moved_from {
            remap(source);
        }
        if target.deleted_ms.is_none() {
            let mut previous = target.key.clone();
            remap(&mut previous);
            for (old, endpoint) in [
                (&row.key.refund, &mut target.key.refund),
                (&row.key.original, &mut target.key.original),
            ] {
                if old.0 == from
                    && !(source_had_splits && old.1.is_empty())
                    && let Some(part) = survivor_part(conn, "main", old, to, keep_live)?
                {
                    *endpoint = Endpoint(to, part);
                }
            }
            if target.key != previous
                && (target.key.refund.1 != previous.refund.1
                    || target.key.original.1 != previous.original.1)
            {
                let snapshot = source_key(conn, &row.key)?;
                folded.push(retire_part_key(conn, &mut target, previous)?);
                target.moved_from_key = snapshot;
                retargeted = true;
            }
        }
        if target == row {
            continue;
        }
        conn.execute("DELETE FROM refund_links WHERE refund_id=?1 AND refund_part=?2 AND original_id=?3 AND original_part=?4",
            params![row.key.refund.0,row.key.refund.1,row.key.original.0,row.key.original.1])?;
        if retargeted {
            folded.push(target);
        } else {
            direct.push(target);
        }
    }
    let occupied: HashSet<_> = load_rows(conn, "main")?
        .into_iter()
        .chain(direct.iter().cloned())
        .filter(|r| r.deleted_ms.is_none())
        .map(|r| r.key)
        .collect();
    folded.retain(|r| r.deleted_ms.is_some() || !occupied.contains(&r.key));
    inactivate_collisions(&mut folded);
    folded.extend(direct);
    merge_rows(conn, folded)
}

pub(crate) fn move_endpoints(
    conn: &Connection,
    endpoints: &HashMap<Endpoint, Endpoint>,
) -> CoreResult<()> {
    let mut moved = Vec::new();
    let stamp = next_stamp(conn)?;
    let rows = load_rows(conn, "main")?;
    let targets: Vec<_> = rows
        .iter()
        .filter_map(|row| {
            let key = Key {
                refund: endpoints
                    .get(&row.key.refund)
                    .cloned()
                    .unwrap_or_else(|| row.key.refund.clone()),
                original: endpoints
                    .get(&row.key.original)
                    .cloned()
                    .unwrap_or_else(|| row.key.original.clone()),
            };
            if key == row.key || row.deleted_ms.is_some() || move_blocked(&rows, row) {
                return None;
            }
            Some((row, key))
        })
        .collect();
    let mut counts = HashMap::new();
    for (_, key) in &targets {
        *counts.entry(key.clone()).or_insert(0usize) += 1;
    }
    for (row, key) in targets {
        let mut row = row.clone();
        let mut target = row.clone();
        target.key = key;
        target.changed_ms = stamp;
        target.moved = false;
        target.moved_from = Some(row.key.clone());
        target.moved_from_key = source_key(conn, &row.key)?;
        let destination = rows.iter().find(|r| r.key == target.key);
        if counts[&target.key] == 1
            && destination.is_none_or(|d| {
                d.deleted_ms.is_some() && (d.moved || d.changed_ms < row.changed_ms)
            })
        {
            moved.push(target);
        }
        row.changed_ms = stamp;
        row.deleted_ms = Some(stamp);
        row.moved = true;
        write(conn, &row)?;
    }
    merge_rows(conn, moved)
}

fn source_key(conn: &Connection, key: &Key) -> CoreResult<String> {
    Ok(serde_json::json!([
        crate::sync_merge::booking_key(conn, key.refund.0)?,
        key.refund.1,
        crate::sync_merge::booking_key(conn, key.original.0)?,
        key.original.1
    ])
    .to_string())
}

fn merge_rows(conn: &Connection, rows: impl IntoIterator<Item = Row>) -> CoreResult<()> {
    let mut local: HashMap<_, _> = load_rows(conn, "main")?
        .into_iter()
        .map(|r| (r.key.clone(), r))
        .collect();
    for other in rows {
        let wins = match local.get(&other.key) {
            None => true,
            Some(current) => {
                other.rank() > current.rank()
                    || (other.rank() == current.rank()
                        && current.moved_from.is_none()
                        && other.moved_from.is_some())
            }
        };
        if wins {
            write(conn, &other)?;
            local.insert(other.key.clone(), other);
        }
    }
    Ok(())
}
