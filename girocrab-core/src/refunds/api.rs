use super::store::{next_stamp, write};
use super::*;

fn key(refund: i64, refund_part: String, original: i64, original_part: String) -> Key {
    Key {
        refund: Endpoint(refund, refund_part),
        original: Endpoint(original, original_part),
    }
}

fn direct(state: &State, e: &Endpoint, positive: bool) -> CoreResult<i64> {
    let book = state
        .books
        .get(&e.0)
        .filter(|b| b.live)
        .ok_or_else(invalid)?;
    book.check_capacity()?;
    let amount = book.amount_of(&e.1).ok_or_else(invalid)?;
    if amount == 0 || (amount > 0) != positive {
        return Err(invalid());
    }
    Ok(amount)
}

#[uniffi::export]
impl Vault {
    pub fn link_refund(
        &self,
        refund_id: i64,
        refund_part: String,
        original_id: i64,
        original_part: String,
    ) -> CoreResult<()> {
        let db = self.db()?;
        let tx = db.unchecked_transaction()?;
        let state = State::load(&tx)?;
        let key = key(refund_id, refund_part, original_id, original_part);
        if state
            .rows
            .iter()
            .any(|r| r.key == key && r.deleted_ms.is_none())
        {
            return Ok(());
        }
        if refund_id == original_id {
            return Err(invalid());
        }
        let refund_cents = direct(&state, &key.refund, true)?;
        let original_cents = direct(&state, &key.original, false)?;
        if state.books[&refund_id].currency != state.books[&original_id].currency {
            return Err(invalid());
        }
        let projection = Projection::from_state(state)?;
        if projection.open(&key.refund) == 0
            || projection.open(&key.original) == 0
            || projection.allocations.iter().any(|a| {
                a.original.as_ref() == Some(&key.refund) || a.refund.as_ref() == Some(&key.original)
            })
        {
            return Err(invalid());
        }
        let ms = next_stamp(&tx)?;
        write(
            &tx,
            &Row {
                key,
                refund_cents,
                original_cents,
                created_ms: ms,
                changed_ms: ms,
                deleted_ms: None,
            },
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The returned token identifies this exact unlink, not merely the link's creation.
    pub fn unlink_refund(
        &self,
        refund_id: i64,
        refund_part: String,
        original_id: i64,
        original_part: String,
    ) -> CoreResult<i64> {
        let db = self.db()?;
        let tx = db.unchecked_transaction()?;
        let key = key(refund_id, refund_part, original_id, original_part);
        let mut row = State::load(&tx)?
            .rows
            .into_iter()
            .find(|r| r.key == key && r.deleted_ms.is_none())
            .ok_or_else(invalid)?;
        let token = next_stamp(&tx)?;
        row.changed_ms = token;
        row.deleted_ms = Some(token);
        write(&tx, &row)?;
        tx.commit()?;
        Ok(token)
    }

    /// Compare-and-set undo. Restoring retains the original allocation position.
    pub fn restore_refund(
        &self,
        refund_id: i64,
        refund_part: String,
        original_id: i64,
        original_part: String,
        token: i64,
    ) -> CoreResult<()> {
        let db = self.db()?;
        let tx = db.unchecked_transaction()?;
        let key = key(refund_id, refund_part, original_id, original_part);
        let mut state = State::load(&tx)?;
        let row = state
            .rows
            .iter_mut()
            .find(|r| r.key == key && r.deleted_ms.is_some() && r.changed_ms == token)
            .ok_or_else(invalid)?;
        row.changed_ms = next_stamp(&tx)?;
        row.deleted_ms = None;
        let restored = row.clone();
        if !resolve(&state)
            .iter()
            .any(|a| a.row.key == key && a.refund.is_some() && a.original.is_some())
        {
            return Err(invalid());
        }
        write(&tx, &restored)?;
        tx.commit()?;
        Ok(())
    }

    /// Live stored links in allocation order, including cheaply detected inactive links.
    /// Key fields address unlink/undo; resolved fields identify the current endpoints.
    pub fn refund_links(&self, transaction_id: i64) -> CoreResult<Vec<RefundLink>> {
        let db = self.db()?;
        let p = self.refund_projection(&db)?;
        Ok(p.allocations
            .iter()
            .filter(|a| {
                a.row.key.refund.0 == transaction_id || a.row.key.original.0 == transaction_id
            })
            .map(|a| {
                let label = |e: &Option<Endpoint>| {
                    e.as_ref()
                        .and_then(|e| p.state.books.get(&e.0).map(|b| b.label_of(&e.1)))
                        .unwrap_or_default()
                };
                RefundLink {
                    refund_id: a.row.key.refund.0,
                    refund_part: a.row.key.refund.1.clone(),
                    original_id: a.row.key.original.0,
                    original_part: a.row.key.original.1.clone(),
                    resolved_refund_part: a.refund.as_ref().map(|e| e.1.clone()),
                    resolved_original_part: a.original.as_ref().map(|e| e.1.clone()),
                    refund_label: label(&a.refund),
                    original_label: label(&a.original),
                    allocated_cents: a.cents,
                    refund_open_cents: a.refund.as_ref().map_or(0, |e| p.open(e)),
                    original_open_cents: a.original.as_ref().map_or(0, |e| p.open(e)),
                    created_ms: a.row.created_ms,
                    changed_ms: a.row.changed_ms,
                    inactive: a.refund.is_none() || a.original.is_none(),
                }
            })
            .collect())
    }
}
