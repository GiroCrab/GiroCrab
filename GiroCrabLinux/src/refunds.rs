use std::collections::HashMap;

use girocrab_core::{BookingFilter, Vault};
use serde::Deserialize;
use serde_json::{Value, json};

type R = Result<Value, String>;

pub fn seed(v: &Vault, args: &Value) -> R {
    use girocrab_core::offline::ManualBookingSetup;
    use girocrab_core::{AccountKind, TransactionSplitInput};
    let run = || -> Result<Value, girocrab_core::CoreError> {
        let account =
            v.create_offline_account("Erstattungen".into(), AccountKind::Cash, "EUR".into(), 0)?;
        let category =
            v.create_category("Bestellungen".into(), None, "cart".into(), "#2F6FDB".into())?;
        let date = args["date"].as_str().unwrap_or("2026-01-01");
        let booking = |amount, name: &str| {
            v.save_manual_booking(
                account,
                None,
                ManualBookingSetup {
                    booking_date: date.into(),
                    amount_cents: amount,
                    counterparty: name.into(),
                    purpose: "Bestellung RE714001".into(),
                    category_id: if amount > 0 { None } else { Some(category) },
                    note: String::new(),
                },
            )
        };
        let original = booking(-10000, "Musterladen")?;
        let refund = booking(6000, "Musterladen")?;
        for (amount, name) in [
            (-4000, "Musterladen"),
            (-3000, "Versandhaus"),
            (-7000, "Buchhandlung"),
            (-8000, "Kaufhaus"),
        ] {
            booking(amount, name)?;
        }
        v.set_transaction_splits(
            refund,
            vec![4000, 2000]
                .into_iter()
                .map(|amount| TransactionSplitInput {
                    uuid: None,
                    category_id: None,
                    amount_cents: amount,
                    note: "Rückzahlung".into(),
                })
                .collect(),
        )?;
        let part = v.list_transaction_splits(refund)?[0].uuid.clone();
        if args["linked"].as_bool().unwrap_or(false) {
            v.link_refund(refund, part.clone(), original, String::new())?;
        }
        Ok(
            json!({"account":account,"category":category,"original":original,"refund":refund,"part":part}),
        )
    };
    run().map_err(|e| e.to_string())
}

#[derive(Deserialize)]
struct Request {
    transaction_id: i64,
    #[serde(default)]
    own_part: String,
    #[serde(default)]
    query: String,
}

fn card_filter(
    v: &Vault,
    id: i64,
    links: &[girocrab_core::refunds::RefundLink],
) -> Result<Option<BookingFilter>, String> {
    let ids: std::collections::BTreeSet<_> = links
        .iter()
        .filter(|l| !l.inactive)
        .map(|l| {
            if l.refund_id == id {
                l.original_id
            } else {
                l.refund_id
            }
        })
        .collect();
    let dates = ids
        .into_iter()
        .map(|id| v.booking_date(id))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    Ok(dates
        .iter()
        .min()
        .zip(dates.iter().max())
        .map(|(from, to)| BookingFilter {
            date_from: Some(from.clone()),
            date_to: Some(to.clone()),
            limit: 0,
            ..Default::default()
        }))
}

pub fn card(v: &Vault, args: &Value) -> R {
    let req: Request = serde_json::from_value(args.clone()).map_err(|e| e.to_string())?;
    let links = v
        .refund_links(req.transaction_id)
        .map_err(|e| e.to_string())?;
    let Some(filter) = card_filter(v, req.transaction_id, &links)? else {
        return Ok(json!([]));
    };
    let bookings = v
        .list_transactions(filter)
        .map_err(|e| e.to_string())?
        .transactions
        .into_iter()
        .map(|b| (b.id, b))
        .collect::<HashMap<_, _>>();
    let mut rows = Vec::new();
    for link in links.into_iter().filter(|l| !l.inactive) {
        let (id, part, label) = if link.refund_id == req.transaction_id {
            (link.original_id, &link.original_part, &link.original_label)
        } else {
            (link.refund_id, &link.refund_part, &link.refund_label)
        };
        if let Some(b) = bookings.get(&id) {
            rows.push(json!({"booking":b,"part_label":if part.is_empty() { "" } else { label },"link":link}));
        }
    }
    Ok(json!(rows))
}

pub fn sheet(v: &Vault, args: &Value) -> R {
    let req: Request = serde_json::from_value(args.clone()).map_err(|e| e.to_string())?;
    // Proof rows supply current booking/link presentation, never financial aggregates.
    let booking = bookings_for(v, &[req.transaction_id])?
        .remove(&req.transaction_id)
        .ok_or_else(|| crate::i18n::tr("Die Buchung ist nicht mehr vorhanden."))?;
    let parts = v
        .list_transaction_splits(booking.id)
        .map_err(|e| e.to_string())?;
    if !req.own_part.is_empty() && !parts.iter().any(|p| p.uuid == req.own_part) {
        return Ok(
            json!({"booking":booking,"parts":parts,"own_part":req.own_part,"selection_invalid":true,"from_refund":false,"own_open":null,"linked":[],"suggestions":[],"candidates":[]}),
        );
    }
    let selected = parts
        .iter()
        .find(|p| p.uuid == req.own_part)
        .or_else(|| {
            parts
                .iter()
                .find(|p| (p.amount_cents > 0) == (booking.amount_cents > 0))
        })
        .or_else(|| parts.first());
    let (own_part, amount) = selected.map_or((String::new(), booking.amount_cents), |p| {
        (p.uuid.clone(), p.amount_cents)
    });
    let from_refund = amount > 0;
    let links = v.refund_links(booking.id).map_err(|e| e.to_string())?;
    let raw_candidates = if amount == 0 {
        Vec::new()
    } else {
        v.refund_candidates(booking.id, own_part.clone(), req.query, 200)
            .map_err(|e| e.to_string())?
    };
    let raw_suggestions = if amount == 0 {
        Vec::new()
    } else {
        v.refund_suggestions(booking.id, own_part.clone())
            .map_err(|e| e.to_string())?
    };
    let ids: Vec<_> = links
        .iter()
        .flat_map(|l| [l.refund_id, l.original_id])
        .chain(raw_candidates.iter().map(|c| c.transaction_id))
        .chain(raw_suggestions.iter().map(|s| s.candidate.transaction_id))
        .collect();
    let books = bookings_for(v, &ids)?;
    let accounts: HashMap<_, _> = v
        .list_account_groups()
        .map_err(|e| e.to_string())?
        .into_iter()
        .flat_map(|g| g.accounts)
        .map(|a| (a.id, a.name))
        .collect();
    let decorate = |mut row: Value| {
        let id = row["transaction_id"].as_i64().unwrap_or_default();
        row["account_name"] = json!(
            row["account_id"]
                .as_i64()
                .and_then(|id| accounts.get(&id))
                .cloned()
                .unwrap_or_default()
        );
        row["purpose"] = json!(
            books
                .get(&id)
                .map(|b| b.purpose.clone())
                .unwrap_or_default()
        );
        row
    };
    let mut current = Vec::new();
    let mut allocated = 0_i128;
    for link in links.iter().filter(|l| {
        !l.inactive
            && if from_refund {
                l.refund_id == booking.id && l.resolved_refund_part.as_deref() == Some(&own_part)
            } else {
                l.original_id == booking.id
                    && l.resolved_original_part.as_deref() == Some(&own_part)
            }
    }) {
        allocated += i128::from(link.allocated_cents);
        let (id, part, label, open) = if from_refund {
            (
                link.original_id,
                link.resolved_original_part.as_deref().unwrap_or_default(),
                &link.original_label,
                link.original_open_cents,
            )
        } else {
            (
                link.refund_id,
                link.resolved_refund_part.as_deref().unwrap_or_default(),
                &link.refund_label,
                link.refund_open_cents,
            )
        };
        let Some(b) = books.get(&id) else { continue };
        let cents = counterpart_amount(v, id, part, b.amount_cents)?;
        current.push(decorate(json!({"transaction_id":id,"part":part,"part_label":label,"merchant_name":b.merchant_name,"account_id":b.account_id,"booking_date":b.booking_date,"amount_cents":cents,"currency":b.currency,"open_cents":open,"allocated_cents":link.allocated_cents,"link":link})));
    }
    let open: i64 = (i128::from(amount).abs() - allocated)
        .max(0)
        .try_into()
        .map_err(|_| crate::i18n::tr("Die Teilbeträge sind zu groß."))?;
    let mut candidates = Vec::new();
    let mut suggestions = Vec::new();
    if amount != 0 && open > 0 {
        for c in raw_candidates {
            candidates.push(decorate(json!(c)));
        }
        for s in raw_suggestions {
            suggestions.push(json!({"candidate":decorate(json!(s.candidate)),"reason":s.reason}));
        }
    }
    Ok(
        json!({"booking":booking,"parts":parts,"own_part":own_part,"from_refund":from_refund,"own_open":open,"linked":current,"suggestions":suggestions,"candidates":candidates}),
    )
}

fn bookings_for(v: &Vault, ids: &[i64]) -> Result<HashMap<i64, girocrab_core::Booking>, String> {
    let dates = ids
        .iter()
        .map(|id| v.booking_date(*id))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?
        .into_iter()
        .flatten()
        .collect::<std::collections::BTreeSet<_>>();
    let mut books = HashMap::new();
    for date in dates {
        for b in v
            .list_transactions(BookingFilter {
                date_from: Some(date.clone()),
                date_to: Some(date),
                limit: 0,
                ..Default::default()
            })
            .map_err(|e| e.to_string())?
            .transactions
        {
            if ids.contains(&b.id) {
                books.insert(b.id, b);
            }
        }
    }
    Ok(books)
}

fn counterpart_amount(v: &Vault, id: i64, part: &str, whole: i64) -> Result<i64, String> {
    if part.is_empty() {
        return Ok(whole);
    }
    v.list_transaction_splits(id)
        .map_err(|e| e.to_string())?
        .iter()
        .find(|p| p.uuid == part)
        .map(|p| p.amount_cents)
        .ok_or_else(|| crate::i18n::tr("Diese Erstattungszuordnung ist nicht möglich."))
}

pub fn edit(v: &Vault, args: &Value) -> R {
    #[derive(Deserialize)]
    struct Edit {
        action: String,
        refund_id: i64,
        refund_part: String,
        original_id: i64,
        original_part: String,
        token: Option<String>,
    }
    let a: Edit = serde_json::from_value(args.clone()).map_err(|e| e.to_string())?;
    let token = match a.action.as_str() {
        "link" => {
            if v.refund_links(a.refund_id)
                .map_err(|e| e.to_string())?
                .iter()
                .any(|l| {
                    !l.inactive
                        && l.refund_id == a.refund_id
                        && l.original_id == a.original_id
                        && l.resolved_refund_part.as_deref() == Some(&a.refund_part)
                        && l.resolved_original_part.as_deref() == Some(&a.original_part)
                })
            {
                return Ok(json!({"changed":false}));
            }
            v.link_refund(a.refund_id, a.refund_part, a.original_id, a.original_part)
                .map_err(|e| e.to_string())?;
            None
        }
        "unlink" => Some(
            v.unlink_refund(a.refund_id, a.refund_part, a.original_id, a.original_part)
                .map_err(|e| e.to_string())?,
        ),
        "restore" => {
            v.restore_refund(
                a.refund_id,
                a.refund_part,
                a.original_id,
                a.original_part,
                a.token.ok_or_else(|| "Missing undo token".to_string())?,
            )
            .map_err(|e| e.to_string())?;
            None
        }
        _ => return Err("Unknown refund action".into()),
    };
    Ok(json!({"changed":true,"token":token}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn fixture() -> (Temp, std::sync::Arc<Vault>, Value) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "girocrab-refund-sheet-{}-{nonce}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        let v = Vault::open(path.join("v.db").to_string_lossy().into(), vec![7; 32]).unwrap();
        let data = seed(&v, &json!({})).unwrap();
        (Temp(path), v, data)
    }
    #[test]
    fn inspector_lookup_is_bounded_to_live_counterpart_days() {
        let (_dir, v, f) = fixture();
        assert!(
            card_filter(&v, f["refund"].as_i64().unwrap(), &[])
                .unwrap()
                .is_none()
        );
        edit(&v, &json!({"action":"link","refund_id":f["refund"],"refund_part":f["part"],"original_id":f["original"],"original_part":""})).unwrap();
        let id = f["refund"].as_i64().unwrap();
        let links = v.refund_links(id).unwrap();
        let add = |date: &str| {
            v.save_manual_booking(
                f["account"].as_i64().unwrap(),
                None,
                girocrab_core::offline::ManualBookingSetup {
                    booking_date: date.into(),
                    amount_cents: -10000,
                    counterparty: "Other day".into(),
                    purpose: String::new(),
                    category_id: None,
                    note: String::new(),
                },
            )
            .unwrap()
        };
        let earlier = add("2020-01-01");
        let later = add("2030-01-01");
        let mut expanded = links.clone();
        let mut second = links[0].clone();
        second.original_id = earlier;
        expanded.push(second);
        let filter = card_filter(&v, id, &expanded).unwrap().unwrap();
        assert_eq!(filter.date_from.as_deref(), Some("2020-01-01"));
        assert_eq!(filter.date_to.as_deref(), Some("2026-01-01"));
        assert!(
            !v.list_transactions(filter)
                .unwrap()
                .transactions
                .iter()
                .any(|b| b.id == later)
        );
        let reverse = card_filter(&v, links[0].original_id, &links)
            .unwrap()
            .unwrap();
        assert_eq!(reverse.date_from, v.booking_date(id).unwrap());
        expanded.iter_mut().for_each(|l| l.inactive = true);
        assert!(card_filter(&v, id, &expanded).unwrap().is_none());
        assert!(card(&v, &json!({})).is_err());
    }

    #[test]
    fn missing_counterpart_part_never_displays_the_booking_amount() {
        let (_dir, v, f) = fixture();
        let original = f["original"].as_i64().unwrap();
        let part = |uuid, amount_cents| girocrab_core::TransactionSplitInput {
            uuid,
            category_id: None,
            amount_cents,
            note: String::new(),
        };
        v.set_transaction_splits(original, vec![part(None, -4000), part(None, -6000)])
            .unwrap();
        let parts = v.list_transaction_splits(original).unwrap();
        edit(&v,&json!({"action":"link","refund_id":f["refund"],"refund_part":f["part"],"original_id":original,"original_part":parts[0].uuid})).unwrap();
        v.set_transaction_splits(
            original,
            vec![part(Some(parts[1].uuid.clone()), -9000), part(None, -1000)],
        )
        .unwrap();
        assert!(counterpart_amount(&v, original, &parts[0].uuid, -10000).is_err());
        assert_eq!(
            counterpart_amount(&v, original, "", -10000).unwrap(),
            -10000
        );
        assert_eq!(
            counterpart_amount(&v, original, &parts[1].uuid, -10000).unwrap(),
            -9000
        );
        assert!(
            sheet(
                &v,
                &json!({"transaction_id":f["refund"],"own_part":f["part"]})
            )
            .unwrap()["linked"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn round7_replaced_parts_return_current_choices_without_using_whole_amount() {
        let (_dir, v, f) = fixture();
        let id = f["refund"].as_i64().unwrap();
        v.set_transaction_splits(
            id,
            vec![4000, 2000]
                .into_iter()
                .map(|amount_cents| girocrab_core::TransactionSplitInput {
                    uuid: None,
                    amount_cents,
                    category_id: None,
                    note: String::new(),
                })
                .collect(),
        )
        .unwrap();
        let data = sheet(&v, &json!({"transaction_id":id,"own_part":f["part"]})).unwrap();
        assert_eq!(data["selection_invalid"], true);
        assert_eq!(data["parts"].as_array().unwrap().len(), 2);
        assert_eq!(data["own_part"], f["part"]);
        assert!(data["own_open"].is_null());
        assert!(data["candidates"].as_array().unwrap().is_empty());
        let recovered = sheet(
            &v,
            &json!({"transaction_id":id,"own_part":data["parts"][0]["uuid"]}),
        )
        .unwrap();
        assert_eq!(recovered["own_open"], 4000);
        v.set_transaction_splits(id, vec![]).unwrap();
        let whole = sheet(&v, &json!({"transaction_id":id,"own_part":f["part"]})).unwrap();
        assert_eq!(whole["selection_invalid"], true);
        assert!(whole["parts"].as_array().unwrap().is_empty());
        assert_eq!(
            sheet(&v, &json!({"transaction_id":id})).unwrap()["own_open"],
            6000
        );
    }

    #[test]
    fn split_sheet_roles_search_and_empty_error() {
        let (_dir, v, f) = fixture();
        let args = json!({"transaction_id":f["refund"],"own_part":f["part"]});
        let data = sheet(&v, &args).unwrap();
        assert_eq!(data["from_refund"], true);
        assert_eq!(data["own_open"], 4000);
        assert_eq!(data["parts"].as_array().unwrap().len(), 2);
        let other = sheet(
            &v,
            &json!({"transaction_id":f["refund"],"own_part":data["parts"][1]["uuid"]}),
        )
        .unwrap();
        assert_eq!(other["own_open"], 2000);
        assert_eq!(data["suggestions"].as_array().unwrap().len(), 3);
        assert!(!data["candidates"].as_array().unwrap().is_empty());
        let empty = sheet(
            &v,
            &json!({"transaction_id":f["refund"],"query":"no-such-merchant-xyz"}),
        )
        .unwrap();
        assert!(empty["candidates"].as_array().unwrap().is_empty());
        let reverse = sheet(&v, &json!({"transaction_id":f["original"]})).unwrap();
        assert_eq!(reverse["from_refund"], false);
        assert!(sheet(&v, &json!({"transaction_id":-1})).is_err());
        assert!(sheet(&v, &json!({})).is_err());
        assert_eq!(
            sheet(
                &v,
                &json!({"transaction_id":f["refund"],"own_part":"deleted-part"})
            )
            .unwrap()["selection_invalid"],
            true
        );
        assert!(bookings_for(&v, &[]).unwrap().is_empty());
        assert_eq!(
            bookings_for(&v, &[f["refund"].as_i64().unwrap()])
                .unwrap()
                .len(),
            1
        );
    }
    #[test]
    fn link_unlink_token_restore_and_noop_roundtrip() {
        let (_dir, v, f) = fixture();
        let mut pair = json!({"action":"link","refund_id":f["refund"],"refund_part":f["part"],"original_id":f["original"],"original_part":""});
        assert_eq!(edit(&v, &pair).unwrap()["changed"], true);
        assert_eq!(edit(&v, &pair).unwrap()["changed"], false);
        let args = json!({"transaction_id":f["refund"],"own_part":f["part"]});
        let linked = sheet(&v, &args).unwrap();
        assert_eq!(linked["own_open"], 0);
        assert_eq!(linked["linked"][0]["allocated_cents"], 4000);
        assert!(linked["candidates"].as_array().unwrap().is_empty());
        let stamp = linked["linked"][0]["link"]["created_ms"].clone();
        pair["action"] = json!("unlink");
        let token = edit(&v, &pair).unwrap()["token"].clone();
        assert!(
            sheet(&v, &args).unwrap()["linked"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        pair["action"] = json!("restore");
        assert!(edit(&v, &pair).is_err());
        pair["token"] = token;
        edit(&v, &pair).unwrap();
        assert_eq!(
            sheet(&v, &args).unwrap()["linked"][0]["link"]["created_ms"],
            stamp
        );
        pair["action"] = json!("invalid");
        assert!(edit(&v, &pair).is_err());
    }

    #[test]
    fn sheet_keeps_allocation_order_across_unlink_and_restore() {
        let (_dir, v, f) = fixture();
        let args = json!({"transaction_id":f["refund"],"own_part":f["part"]});
        let first = sheet(
            &v,
            &json!({"transaction_id":f["refund"],"own_part":f["part"],"query":"Versandhaus"}),
        )
        .unwrap()["candidates"][0]["transaction_id"]
            .clone();
        let mut pair = json!({"action":"link","refund_id":f["refund"],"refund_part":f["part"],"original_id":first,"original_part":""});
        edit(&v, &pair).unwrap();
        let mut second = pair.clone();
        second["original_id"] = f["original"].clone();
        edit(&v, &second).unwrap();
        pair["action"] = json!("unlink");
        let token = edit(&v, &pair).unwrap()["token"].clone();
        assert_eq!(
            sheet(&v, &args).unwrap()["linked"][0]["allocated_cents"],
            4000
        );
        pair["action"] = json!("restore");
        pair["token"] = token;
        edit(&v, &pair).unwrap();
        let data = sheet(&v, &args).unwrap();
        assert_eq!(data["linked"][0]["transaction_id"], first);
        assert_eq!(data["linked"][0]["allocated_cents"], 3000);
        assert_eq!(data["linked"][1]["allocated_cents"], 1000);
        pair["action"] = json!("unlink");
        edit(&v, &pair).unwrap();
        pair["action"] = json!("restore");
        assert!(edit(&v, &pair).is_err());
    }
}
