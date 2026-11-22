use super::super::*;
use crate::test_support::TempDir;
use std::sync::Arc;

fn demo() -> (TempDir, Arc<Vault>) {
    let dir = TempDir::new("local-api-edit");
    let v = Vault::open(
        dir.join("v.db").to_string_lossy().to_string(),
        vec![7u8; 32],
    )
    .unwrap();
    v.seed_demo_data().unwrap();
    (dir, v)
}

fn account_id(v: &Vault, name: &str) -> i64 {
    v.list_accounts()
        .unwrap()
        .into_iter()
        .find(|a| a.name == name)
        .unwrap()
        .id
}

/// Shares `accounts`, with edits allowed when `edits`.
fn share(v: &Vault, accounts: &[i64], edits: bool) {
    v.set_local_api_scope(ShareScope {
        enabled: true,
        account_ids: accounts.to_vec(),
        allow_edits: edits,
        ..Default::default()
    })
    .unwrap();
}

fn call(v: &Vault, method: &str, params: Value) -> Value {
    let req = json!({ "v": 1, "method": method, "params": params }).to_string();
    serde_json::from_str(&v.local_api_request(&req)).unwrap()
}

fn ok(v: &Vault, method: &str, params: Value) -> Value {
    let resp = call(v, method, params);
    assert!(resp.get("error").is_none(), "{method}: {resp}");
    resp["result"].clone()
}

fn err_code(v: &Vault, method: &str, params: Value) -> String {
    call(v, method, params)["error"]["code"]
        .as_str()
        .unwrap()
        .to_string()
}

/// The booking as the API reads it.
fn booking(v: &Vault, id: i64) -> Value {
    let mut params = json!({ "limit": 500 });
    loop {
        let page = ok(v, "transactions", params.clone());
        if let Some(b) = page["transactions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["id"] == id)
        {
            return b.clone();
        }
        params["cursor"] = page["next_cursor"].clone();
        assert!(!params["cursor"].is_null(), "booking {id} not served");
    }
}

/// Two bookings of `account` without category, and a category id.
fn uncategorized(v: &Vault, account: i64) -> (i64, i64, i64) {
    let page = ok(
        v,
        "transactions",
        json!({ "account_ids": [account], "uncategorized_only": true, "limit": 2 }),
    );
    let ids: Vec<i64> = page["transactions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids.len(), 2, "the demo has uncategorized bookings");
    let cats = ok(v, "categories", Value::Null);
    let cat = cats.as_array().unwrap()[0]["id"].as_i64().unwrap();
    (ids[0], ids[1], cat)
}

fn statuses(result: &Value) -> Vec<String> {
    result["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["status"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn edits_need_their_own_switch() {
    let (_d, v) = demo();
    let giro = account_id(&v, "Musterbank Girokonto");
    share(&v, &[giro], false);
    let (id, _, cat) = uncategorized(&v, giro);
    assert_eq!(ok(&v, "info", Value::Null)["allows_edits"], false);
    let set = json!({ "transaction_ids": [id], "category_id": cat });
    assert_eq!(err_code(&v, "set_category", set.clone()), "read_only");
    let tag = json!({ "transaction_ids": [id], "add": ["Dienstreise"] });
    assert_eq!(err_code(&v, "set_tags", tag), "read_only");
    // reading the tags is no edit
    assert!(ok(&v, "tags", Value::Null).is_array());
    share(&v, &[giro], true);
    assert_eq!(ok(&v, "info", Value::Null)["allows_edits"], true);
    assert_eq!(statuses(&ok(&v, "set_category", set)), ["changed"]);
}

#[test]
fn an_api_category_counts_as_manual_and_outlasts_the_rules() {
    let (_d, v) = demo();
    let giro = account_id(&v, "Musterbank Girokonto");
    share(&v, &[giro], true);
    let (id, _, cat) = uncategorized(&v, giro);
    assert_eq!(booking(&v, id)["category_source"], "none");
    let r = ok(
        &v,
        "set_category",
        json!({ "transaction_ids": [id], "category_id": cat }),
    );
    assert_eq!(r["category_id"], cat);
    assert!(!r["category_path"].as_str().unwrap().is_empty());
    assert_eq!(r["results"][0]["previous_source"], "none");
    v.apply_rules(false).unwrap();
    let b = booking(&v, id);
    assert_eq!(b["category_id"], cat);
    assert_eq!(b["category_source"], "manual");
    // the same again writes nothing
    let again = ok(
        &v,
        "set_category",
        json!({ "transaction_ids": [id], "category_id": cat }),
    );
    assert_eq!(statuses(&again), ["unchanged"]);
}

#[test]
fn manual_categories_stay_unless_overwritten_rule_ones_are_replaced() {
    let (_d, v) = demo();
    let giro = account_id(&v, "Musterbank Girokonto");
    share(&v, &[giro], true);
    let cats: Vec<i64> = ok(&v, "categories", Value::Null)
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_i64().unwrap())
        .collect();
    let (manual, _, _) = uncategorized(&v, giro);
    v.set_transaction_category(manual, Some(cats[0])).unwrap();
    let ruled = ok(
        &v,
        "transactions",
        json!({ "account_ids": [giro], "limit": 500 }),
    )["transactions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["category_source"] == "rule")
        .expect("the demo has rule-categorised bookings")
        .clone();
    let ruled_id = ruled["id"].as_i64().unwrap();
    let target = *cats
        .iter()
        .find(|c| Some(**c) != ruled["category_id"].as_i64() && **c != cats[0])
        .unwrap();
    let r = ok(
        &v,
        "set_category",
        json!({ "transaction_ids": [manual, ruled_id], "category_id": target }),
    );
    assert_eq!(statuses(&r), ["skipped_manual", "changed"]);
    assert_eq!(r["results"][0]["previous_source"], "manual");
    assert_eq!(r["results"][1]["previous_source"], "rule");
    assert_eq!(booking(&v, manual)["category_id"], cats[0]);
    assert_eq!(booking(&v, ruled_id)["category_source"], "manual");
    let forced = ok(
        &v,
        "set_category",
        json!({ "transaction_ids": [manual], "category_id": target, "overwrite_manual": true }),
    );
    assert_eq!(statuses(&forced), ["changed"]);
    assert_eq!(booking(&v, manual)["category_id"], target);
    // removing a manual category needs the override too
    let clear = json!({ "transaction_ids": [manual], "category_id": null });
    assert_eq!(statuses(&ok(&v, "set_category", clear)), ["skipped_manual"]);
    let clear = json!({ "transaction_ids": [manual], "overwrite_manual": true });
    assert_eq!(statuses(&ok(&v, "set_category", clear)), ["changed"]);
    assert_eq!(booking(&v, manual)["category_source"], "none");
}

#[test]
fn only_bookings_of_shared_accounts_and_valid_requests() {
    let (_d, v) = demo();
    let giro = account_id(&v, "Musterbank Girokonto");
    let other = v
        .list_accounts()
        .unwrap()
        .into_iter()
        .find(|a| a.id != giro)
        .unwrap()
        .id;
    share(&v, &[other], true);
    let (hidden, _, cat) = {
        share(&v, &[giro], true);
        uncategorized(&v, giro)
    };
    share(&v, &[other], true);
    let r = ok(
        &v,
        "set_category",
        json!({ "transaction_ids": [hidden, 99_999_999], "category_id": cat }),
    );
    assert_eq!(statuses(&r), ["not_found", "not_found"]);
    assert!(r["results"][0].get("previous_source").is_none());
    let r = ok(
        &v,
        "set_tags",
        json!({ "transaction_ids": [hidden], "add": ["Probe"] }),
    );
    assert_eq!(statuses(&r), ["not_found"]);
    // nothing written on the unshared account
    share(&v, &[giro], true);
    assert_eq!(booking(&v, hidden)["category_source"], "none");
    assert_eq!(booking(&v, hidden)["tags"], json!([]));
    for params in [
        json!({ "transaction_ids": [], "category_id": cat }),
        json!({ "transaction_ids": (1..=501).collect::<Vec<i64>>(), "category_id": cat }),
        json!({ "transaction_ids": [hidden], "category_id": 99_999_999 }),
        json!({ "transaction_ids": [hidden], "category": cat }),
    ] {
        assert_eq!(err_code(&v, "set_category", params), "invalid_params");
    }
    for params in [
        json!({ "transaction_ids": [hidden] }),
        json!({ "transaction_ids": [hidden], "add": ["   "] }),
    ] {
        assert_eq!(err_code(&v, "set_tags", params), "invalid_params");
    }
}

#[test]
fn tags_are_added_by_name_created_when_missing_and_removed() {
    let (_d, v) = demo();
    let giro = account_id(&v, "Musterbank Girokonto");
    share(&v, &[giro], true);
    let (a, b, _) = uncategorized(&v, giro);
    let r = ok(
        &v,
        "set_tags",
        json!({ "transaction_ids": [a, b, a], "add": ["Dienstreise", "dienstreise "] }),
    );
    assert_eq!(r["created_tags"], json!(["Dienstreise"]));
    assert_eq!(
        statuses(&r),
        ["changed", "changed"],
        "duplicates count once"
    );
    assert_eq!(booking(&v, a)["tags"], json!(["Dienstreise"]));
    let tags = ok(&v, "tags", Value::Null);
    let tag = tags
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "Dienstreise")
        .unwrap();
    assert_eq!(tag["booking_count"], 2);
    let r = ok(
        &v,
        "set_tags",
        json!({ "transaction_ids": [a], "add": ["Dienstreise"], "remove": ["unbekannt"] }),
    );
    assert_eq!(r["created_tags"], json!([]));
    assert_eq!(statuses(&r), ["unchanged"]);
    let r = ok(
        &v,
        "set_tags",
        json!({ "transaction_ids": [a], "remove": ["DIENSTREISE"] }),
    );
    assert_eq!(statuses(&r), ["changed"]);
    assert_eq!(booking(&v, a)["tags"], json!([]));
    assert_eq!(booking(&v, b)["tags"], json!(["Dienstreise"]));
}

#[test]
fn tag_counts_stay_within_the_shared_accounts() {
    let (_d, v) = demo();
    let giro = account_id(&v, "Musterbank Girokonto");
    let other = v
        .list_accounts()
        .unwrap()
        .into_iter()
        .find(|a| a.id != giro)
        .unwrap()
        .id;
    share(&v, &[other], true);
    let (hidden, _, _) = {
        share(&v, &[giro], true);
        uncategorized(&v, giro)
    };
    ok(
        &v,
        "set_tags",
        json!({ "transaction_ids": [hidden], "add": ["Nur Giro"] }),
    );
    let count = |v: &Vault| {
        ok(v, "tags", Value::Null)
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "Nur Giro")
            .unwrap()["booking_count"]
            .clone()
    };
    assert_eq!(count(&v), 1);
    // The giro account is no longer shared: the name stays, its use does not show.
    share(&v, &[other], false);
    assert_eq!(count(&v), 0);
    share(&v, &[], false);
    assert_eq!(count(&v), 0);
}

#[test]
fn the_same_category_from_a_rule_becomes_the_users_own() {
    let (_d, v) = demo();
    let giro = account_id(&v, "Musterbank Girokonto");
    share(&v, &[giro], true);
    let ruled = ok(
        &v,
        "transactions",
        json!({ "account_ids": [giro], "limit": 500 }),
    )["transactions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["category_source"] == "rule")
        .expect("the demo has rule-categorised bookings")
        .clone();
    let r = ok(
        &v,
        "set_category",
        json!({ "transaction_ids": [ruled["id"]], "category_id": ruled["category_id"] }),
    );
    assert_eq!(statuses(&r), ["changed"]);
    assert_eq!(r["results"][0]["previous_source"], "rule");
    let b = booking(&v, ruled["id"].as_i64().unwrap());
    assert_eq!(b["category_id"], ruled["category_id"]);
    assert_eq!(b["category_source"], "manual");
    // now it is the user's: the same request again writes nothing
    let r = ok(
        &v,
        "set_category",
        json!({ "transaction_ids": [ruled["id"]], "category_id": ruled["category_id"] }),
    );
    assert_eq!(statuses(&r), ["unchanged"]);
}

#[test]
fn a_tag_cannot_be_added_and_removed_at_once() {
    let (_d, v) = demo();
    let giro = account_id(&v, "Musterbank Girokonto");
    share(&v, &[giro], true);
    let (id, _, _) = uncategorized(&v, giro);
    let both = json!({ "transaction_ids": [id], "add": ["Probe"], "remove": [" probe "] });
    assert_eq!(err_code(&v, "set_tags", both), "invalid_params");
    assert_eq!(booking(&v, id)["tags"], json!([]));
}
