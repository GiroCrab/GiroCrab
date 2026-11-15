use super::*;
use crate::test_support::TempDir;
use std::sync::Arc;

fn demo() -> (TempDir, Arc<Vault>) {
    let dir = TempDir::new("local-api");
    let v = Vault::open(
        dir.join("v.db").to_string_lossy().to_string(),
        vec![5u8; 32],
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

fn share(v: &Vault, accounts: &[i64], edit: impl FnOnce(&mut ShareScope)) {
    let mut scope = ShareScope {
        enabled: true,
        account_ids: accounts.to_vec(),
        ..Default::default()
    };
    edit(&mut scope);
    v.set_local_api_scope(&scope).unwrap();
}

fn call(v: &Vault, method: &str, params: Value) -> Value {
    let req = json!({ "v": 1, "method": method, "params": params }).to_string();
    let resp: Value = serde_json::from_str(&v.local_api_request(&req)).unwrap();
    assert_eq!(resp["v"], 1);
    resp
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

/// Every booking of `params` across all pages of `limit`.
fn all_pages(v: &Vault, mut params: Value, limit: u32) -> Vec<Value> {
    params["limit"] = json!(limit);
    let mut out = Vec::new();
    loop {
        let page = ok(v, "transactions", params.clone());
        out.extend(page["transactions"].as_array().unwrap().iter().cloned());
        match page["next_cursor"].as_str() {
            Some(c) => params["cursor"] = json!(c),
            None => return out,
        }
    }
}

#[test]
fn off_until_enabled_and_revocable() {
    let (_d, v) = demo();
    assert_eq!(err_code(&v, "info", Value::Null), "disabled");
    let giro = account_id(&v, "Musterbank Girokonto");
    share(&v, &[giro], |_| {});
    assert_eq!(ok(&v, "info", Value::Null)["account_count"], 1);
    share(&v, &[giro], |s| s.enabled = false);
    assert_eq!(err_code(&v, "accounts", Value::Null), "disabled");
}

#[test]
fn only_read_methods_and_this_version() {
    let (_d, v) = demo();
    share(&v, &[account_id(&v, "Musterbank Girokonto")], |_| {});
    for method in [
        "set_transaction_category",
        "update_transaction_note",
        "list_bank_connections",
    ] {
        assert_eq!(err_code(&v, method, Value::Null), "unknown_method");
    }
    let resp: Value =
        serde_json::from_str(&v.local_api_request(r#"{"v":2,"method":"info"}"#)).unwrap();
    assert_eq!(resp["error"]["code"], "unsupported_version");
    let resp: Value = serde_json::from_str(&v.local_api_request("not json")).unwrap();
    assert_eq!(resp["error"]["code"], "invalid_params");
    assert_eq!(
        err_code(&v, "transactions", json!({ "accout_ids": [1] })),
        "invalid_params",
        "unknown parameters are rejected, not ignored"
    );
}

#[test]
fn accounts_are_an_allow_list_without_numbers_by_default() {
    let (_d, v) = demo();
    let giro = account_id(&v, "Musterbank Girokonto");
    let savings = account_id(&v, "Tagesgeld Plus");
    share(&v, &[giro], |_| {});
    let accounts = ok(&v, "accounts", Value::Null);
    let accounts = accounts.as_array().unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0]["id"], giro);
    assert!(accounts[0].get("iban").is_none());
    assert_eq!(
        err_code(&v, "transactions", json!({ "account_ids": [savings] })),
        "not_shared"
    );
    assert_eq!(
        err_code(&v, "transactions", json!({ "account_ids": [999_999] })),
        "not_shared",
        "an unknown id reads like an unshared one"
    );
    share(&v, &[giro], |s| s.share_account_numbers = true);
    let accounts = ok(&v, "accounts", Value::Null);
    assert_eq!(accounts[0]["iban"], "DE89370400440532013000");
}

#[test]
fn bookings_stay_in_shared_accounts_and_hide_ibans_and_notes() {
    let (_d, v) = demo();
    let giro = account_id(&v, "Musterbank Girokonto");
    let booking = v
        .list_transactions(BookingFilter {
            account_ids: vec![giro],
            limit: 1,
            ..Default::default()
        })
        .unwrap()
        .transactions[0]
        .id;
    v.update_transaction_note(booking, "Geheime Notiz".into())
        .unwrap();
    {
        let db = v.db().unwrap();
        db.execute(
            "UPDATE transactions SET counterparty_iban = 'DE02120300000000202051',
                    purpose = 'Miete an DE02 1203 0000 0000 2020 51' WHERE id = ?1",
            [booking],
        )
        .unwrap();
    }
    share(&v, &[giro], |_| {});
    let rows = all_pages(&v, json!({}), 50);
    assert!(!rows.is_empty());
    assert!(rows.iter().all(|t| t["account_id"] == giro));
    let text = serde_json::to_string(&rows).unwrap();
    assert!(!text.contains("DE02"), "IBAN leaked: {text}");
    assert!(!text.contains("Geheime Notiz"));
    let row = rows.iter().find(|t| t["id"] == booking).unwrap();
    assert_eq!(row["purpose"], "Miete an [IBAN]");

    share(&v, &[giro], |s| {
        s.share_account_numbers = true;
        s.share_notes = true;
    });
    let rows = all_pages(&v, json!({}), 50);
    let row = rows.iter().find(|t| t["id"] == booking).unwrap();
    assert_eq!(row["counterparty_iban"], "DE02120300000000202051");
    assert_eq!(row["note"], "Geheime Notiz");

    share(&v, &[giro], |s| {
        s.share_counterparty = false;
        s.share_purpose = false;
    });
    let rows = all_pages(&v, json!({}), 50);
    assert!(rows.iter().all(|t| t.get("counterparty").is_none()
        && t.get("purpose").is_none()
        && t.get("booking_text").is_none()
        && t["merchant"] == ""));
}

#[test]
fn loan_view_does_not_pull_in_other_accounts() {
    let (_d, v) = demo();
    let giro = account_id(&v, "Musterbank Girokonto");
    let loan = account_id(&v, "Immobilien-Darlehen");
    let linked = v
        .list_transactions(BookingFilter {
            account_ids: vec![giro],
            limit: 1,
            ..Default::default()
        })
        .unwrap()
        .transactions[0]
        .id;
    v.db()
        .unwrap()
        .execute(
            "INSERT OR REPLACE INTO loan_links(transaction_id, loan_account_id, source) VALUES (?1, ?2, 'manual')",
            [linked, loan],
        )
        .unwrap();
    // The app's loan view shows the Girokonto booking …
    let app = v
        .list_transactions(BookingFilter {
            account_ids: vec![loan],
            ..Default::default()
        })
        .unwrap();
    assert!(app.transactions.iter().any(|t| t.id == linked));
    // … the interface only the bookings on the loan account, also with both shared.
    share(&v, &[giro, loan], |_| {});
    let rows = all_pages(&v, json!({ "account_ids": [loan] }), 50);
    assert!(rows.iter().all(|t| t["account_id"] == loan), "{rows:?}");
    share(&v, &[loan], |_| {});
    let rows = all_pages(&v, json!({}), 50);
    assert!(rows.iter().all(|t| t["account_id"] == loan), "{rows:?}");
    let summary = ok(
        &v,
        "summary",
        json!({ "date_from": "1900-01-01", "date_to": "2999-12-31" }),
    );
    let count: u64 = summary["totals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["booking_count"].as_u64().unwrap())
        .sum();
    assert_eq!(count as usize, rows.len());
}

#[test]
fn keyset_pages_match_the_booking_list() {
    let (_d, v) = demo();
    let giro = account_id(&v, "Musterbank Girokonto");
    let card = account_id(&v, "Visa Gold Kreditkarte");
    share(&v, &[giro, card], |_| {});
    let expected: Vec<i64> = v
        .list_transactions(BookingFilter {
            account_ids: vec![giro, card],
            direction: BookingDirection::Expense,
            ..Default::default()
        })
        .unwrap()
        .transactions
        .iter()
        .map(|t| t.id)
        .collect();
    assert!(expected.len() > 20);
    let got: Vec<i64> = all_pages(&v, json!({ "direction": "expense" }), 7)
        .iter()
        .map(|t| t["id"].as_i64().unwrap())
        .collect();
    assert_eq!(got, expected);
}

#[test]
fn filters_and_cursor_are_validated() {
    let (_d, v) = demo();
    share(&v, &[account_id(&v, "Musterbank Girokonto")], |_| {});
    assert_eq!(
        err_code(&v, "transactions", json!({ "date_from": "01.02.2026" })),
        "invalid_params"
    );
    assert_eq!(
        err_code(&v, "transactions", json!({ "cursor": "x" })),
        "invalid_params"
    );
    assert_eq!(
        err_code(&v, "transactions", json!({ "direction": "sideways" })),
        "invalid_params"
    );
    assert_eq!(err_code(&v, "summary", json!({})), "invalid_params");
    let page = ok(&v, "transactions", json!({ "limit": 100_000 }));
    assert!(page["transactions"].as_array().unwrap().len() <= MAX_PAGE as usize);
}

#[test]
fn search_cannot_probe_hidden_fields() {
    let (_d, v) = demo();
    let giro = account_id(&v, "Musterbank Girokonto");
    share(&v, &[giro], |_| {});
    ok(&v, "transactions", json!({ "search": "Miete" }));
    assert_eq!(
        err_code(&v, "transactions", json!({ "search": "DE89" })),
        "not_shared"
    );
    share(&v, &[giro], |s| s.share_purpose = false);
    assert_eq!(
        err_code(&v, "transactions", json!({ "search": "Miete" })),
        "not_shared"
    );
    share(&v, &[giro], |s| s.share_account_numbers = true);
    ok(&v, "transactions", json!({ "search": "DE89" }));
}

#[test]
fn summary_matches_the_app_tiles() {
    let (_d, v) = demo();
    let giro = account_id(&v, "Musterbank Girokonto");
    share(&v, &[giro], |_| {});
    let (from, to) = ("2000-01-01", "2999-12-31");
    let page = v
        .list_transactions(BookingFilter {
            account_ids: vec![giro],
            date_from: Some(from.into()),
            date_to: Some(to.into()),
            ..Default::default()
        })
        .unwrap();
    let s = ok(&v, "summary", json!({ "date_from": from, "date_to": to }));
    let totals = s["totals"].as_array().unwrap();
    assert_eq!(totals.len(), 1, "demo Girokonto is EUR only");
    assert_eq!(totals[0]["currency"], "EUR");
    assert_eq!(totals[0]["income_cents"], page.income_cents);
    assert_eq!(totals[0]["expense_cents"], -page.expense_cents);
    assert_eq!(totals[0]["booking_count"], page.total_count);
    let net: i64 = s["by_category"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["net_cents"].as_i64().unwrap())
        .sum();
    let all: i64 = page.transactions.iter().map(|t| t.amount_cents).sum();
    assert_eq!(net, all, "split parts add up to the bookings");
}

#[test]
fn categories_are_the_tree() {
    let (_d, v) = demo();
    share(&v, &[account_id(&v, "Musterbank Girokonto")], |_| {});
    let cats = ok(&v, "categories", Value::Null);
    let cats = cats.as_array().unwrap();
    assert!(!cats.is_empty());
    let ids: HashSet<i64> = cats.iter().map(|c| c["id"].as_i64().unwrap()).collect();
    for c in cats {
        if let Some(p) = c["parent_id"].as_i64() {
            assert!(ids.contains(&p));
            assert!(c["depth"].as_u64().unwrap() >= 1);
        }
    }
}
