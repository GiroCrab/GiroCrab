//! Conservative reconciliation inside the import transaction. Indexed seven-day
//! candidate buckets avoid comparing every hold with every stored booking.
use super::*;
struct Row {
    id: i64,
    ext: String,
    day: i32,
    imported_unix: i64,
    cents: i64,
    merchant: String,
    protected: bool,
}
type Key = (i32, i64, String);
fn key(row: &Row, day: i32) -> Key {
    (day, row.cents, row.merchant.clone())
}
fn could_settle(hold: &Row, post: &Row, reported: &HashSet<&str>) -> bool {
    // Older versions inserted same-day postings first when both arrived
    // together. Require the same import timestamp and a re-reported posting
    // before considering a match in that order.
    post.id > hold.id
        || (post.day == hold.day
            && post.imported_unix == hold.imported_unix
            && reported.contains(post.ext.as_str()))
}
fn buckets<'a>(
    rows: &'a [Row],
    pending: bool,
    settled: &HashSet<String>,
) -> HashMap<Key, Vec<&'a Row>> {
    let mut out: HashMap<Key, Vec<&Row>> = HashMap::new();
    for row in rows.iter().filter(|r| {
        compatibility::is_hold(&r.ext) == pending && (pending || !settled.contains(&r.ext))
    }) {
        out.entry(key(row, row.day)).or_default().push(row);
    }
    out
}
pub(crate) fn settle_holds(
    db: &Connection,
    accounts: &HashMap<String, i64>,
    data: &mut FetchResult,
) -> CoreResult<()> {
    let mut state = decode_state(data.state.as_ref())?;
    let reported: HashSet<_> = data
        .transactions
        .iter()
        .map(|t| t.external_id.as_str())
        .collect();
    for id in accounts.values() {
        let mut q=db.prepare("SELECT t.id,t.external_id,t.booking_date,t.amount_cents,t.counterparty,
            EXISTS(SELECT 1 FROM transaction_splits WHERE transaction_id=t.id) OR EXISTS(SELECT 1 FROM booking_documents WHERE booking_id=t.id),
            t.imported_unix
            FROM transactions t WHERE t.account_id=?1 AND t.deleted_ms IS NULL AND t.currency='EUR'
            AND (t.external_id LIKE 'pending-hold-portal-advanzia-%' OR t.external_id LIKE 'advanzia-hold-%' OR t.external_id LIKE 'advanzia-post-%') ORDER BY t.id")?;
        let rows = q
            .query_map([id], |r| {
                let date: String = r.get(2)?;
                let merchant: String = r.get(4)?;
                Ok(Row {
                    id: r.get(0)?,
                    ext: r.get(1)?,
                    day: jiff::civil::Date::strptime("%Y-%m-%d", &date)
                        .map_or(i32::MIN, crate::dates::day_number),
                    imported_unix: r.get(6)?,
                    cents: r.get(3)?,
                    merchant: merchant
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                        .to_lowercase(),
                    protected: r.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let holds = buckets(&rows, true, &state.settled_posts);
        let posts = buckets(&rows, false, &state.settled_posts);
        for hold in rows.iter().filter(|h| {
            compatibility::is_hold(&h.ext)
                && !h.protected
                && !h.merchant.is_empty()
                && h.day != i32::MIN
                && !reported.contains(h.ext.as_str())
        }) {
            let matches: Vec<_> = (-1..=5)
                .filter_map(|offset| posts.get(&key(hold, hold.day + offset)))
                .flat_map(|bucket| {
                    let start = bucket.partition_point(|p| p.day != hold.day && p.id <= hold.id);
                    bucket[start..].iter().copied()
                })
                .filter(|post| could_settle(hold, post, &reported))
                .take(2)
                .collect();
            if matches.len() != 1 {
                continue;
            }
            let post = matches[0];
            if post.protected || state.settled_posts.contains(&post.ext) {
                continue;
            }
            // Include still-open and protected holds in the reverse uniqueness check.
            let reverse: Vec<_> = (-5..=1)
                .filter_map(|offset| holds.get(&key(post, post.day + offset)))
                .flat_map(|bucket| {
                    let end = bucket.partition_point(|h| h.day == post.day || h.id < post.id);
                    bucket[..end].iter().copied()
                })
                .filter(|hold| could_settle(hold, post, &reported))
                .take(2)
                .collect();
            if reverse.len() == 1 && reverse[0].id == hold.id {
                // Preserve tag assignments when the provisional row becomes a posting.
                let tagged = db.execute(
                    "INSERT OR IGNORE INTO transaction_tags(transaction_id, tag_id)
                     SELECT ?2, tag_id FROM transaction_tags WHERE transaction_id = ?1",
                    params![hold.id, post.id],
                )?;
                if tagged > 0 {
                    db.execute(
                        "UPDATE transactions SET tags_ms = ?2 WHERE id = ?1",
                        params![post.id, crate::dates::now_ms()],
                    )?;
                }
                crate::importers::portal_import::hand_over(db, hold.id, post.id)?;
                state.settled_posts.insert(post.ext.clone());
            }
        }
    }
    state.target_account_id = None;
    state.import_aliases.clear();
    data.state = Some(encode_state(&state)?);
    Ok(())
}
