use rusqlite::{Connection, params};

use super::*;

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
    Ok(conn.prepare(&format!("SELECT refund_id,refund_part,original_id,original_part,refund_cents,original_cents,created_ms,changed_ms,deleted_ms FROM {schema}.refund_links"))?
        .query_map([],|r|Ok(Row{key:Key{refund:Endpoint(r.get(0)?,r.get(1)?),original:Endpoint(r.get(2)?,r.get(3)?)},refund_cents:r.get(4)?,original_cents:r.get(5)?,created_ms:r.get(6)?,changed_ms:r.get(7)?,deleted_ms:r.get(8)?}))?
        .collect::<Result<_,_>>()?)
}

pub(super) fn write(conn: &Connection, row: &Row) -> CoreResult<()> {
    conn.execute("INSERT INTO refund_links(refund_id,refund_part,original_id,original_part,refund_cents,original_cents,created_ms,changed_ms,deleted_ms)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
         ON CONFLICT(refund_id,refund_part,original_id,original_part) DO UPDATE SET
         refund_cents=excluded.refund_cents,original_cents=excluded.original_cents,created_ms=excluded.created_ms,changed_ms=excluded.changed_ms,deleted_ms=excluded.deleted_ms",
         params![row.key.refund.0,row.key.refund.1,row.key.original.0,row.key.original.1,row.refund_cents,row.original_cents,row.created_ms,row.changed_ms,row.deleted_ms])?;
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

/// Local edits preserve an old-key tombstone when an endpoint is retargeted.
pub(crate) fn reconcile(conn: &Connection, booking: i64) -> CoreResult<()> {
    let linked:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM refund_links WHERE deleted_ms IS NULL AND (refund_id=?1 OR original_id=?1))",[booking],|r|r.get(0))?;
    if !linked {
        return Ok(());
    }
    let state = State::load(conn)?;
    let resolutions = resolve(&state);
    for a in resolutions
        .into_iter()
        .filter(|a| a.row.key.refund.0 == booking || a.row.key.original.0 == booking)
    {
        let target = a
            .refund
            .zip(a.original)
            .map(|(refund, original)| Key { refund, original });
        if target.as_ref() == Some(&a.row.key) {
            continue;
        }
        let mut old = a.row;
        old.changed_ms = next_stamp(conn)?;
        old.deleted_ms = Some(old.changed_ms);
        write(conn, &old)?;
        if let Some(key) = target {
            let mut moved = old;
            moved.key = key;
            moved.deleted_ms = None;
            moved.changed_ms = next_stamp(conn)?;
            let current = load_rows(conn, "main")?
                .into_iter()
                .find(|r| r.key == moved.key);
            if current.is_none_or(|r| moved.rank() > r.rank()) {
                write(conn, &moved)?;
            }
        }
    }
    Ok(())
}

/// Plain full-row LWW. No resolver, allocation, timestamps or booking-state cleanup.
pub(crate) fn merge_attached(conn: &Connection, bookings: &HashMap<i64, i64>) -> CoreResult<()> {
    let mut local: HashMap<_, _> = load_rows(conn, "main")?
        .into_iter()
        .map(|r| (r.key.clone(), r))
        .collect();
    for mut other in load_rows(conn, "other")? {
        let (Some(&refund), Some(&original)) = (
            bookings.get(&other.key.refund.0),
            bookings.get(&other.key.original.0),
        ) else {
            continue;
        };
        other.key.refund.0 = refund;
        other.key.original.0 = original;
        if local
            .get(&other.key)
            .is_none_or(|r| other.rank() > r.rank())
        {
            write(conn, &other)?;
            local.insert(other.key.clone(), other);
        }
    }
    Ok(())
}
