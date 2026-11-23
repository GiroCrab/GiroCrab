//! Full fetch of a login (#1043): period, window and when it is done.

use super::demo_vault;
use crate::connectors::*;
use crate::dates::{days, iso, today};
use crate::error::CoreError;
use crate::models::ConnectorKind;

fn date(s: &str) -> jiff::civil::Date {
    jiff::civil::Date::strptime("%Y-%m-%d", s).unwrap()
}

#[test]
fn full_fetch_starts_accounts_with_bookings_too() {
    let newest = vec![
        ("A".to_string(), Some("2026-11-01".to_string())),
        ("B".to_string(), Some("2024-01-01".to_string())),
        ("C".to_string(), None),
    ];
    let (since, per) = since_window(date("2026-11-12"), 90, &newest, Some(365));
    assert_eq!(per["A"], "2025-11-12", "not capped at its newest booking");
    // lagging behind further than the period: the period is what was asked for
    assert_eq!(per["B"], "2025-11-12");
    assert_eq!(per["C"], "2025-11-12");
    assert_eq!(since, "2025-11-12");
    // without bookings the period counts even when it is shorter than `days`
    let (since, per) = since_window(date("2026-11-12"), 730, &newest[2..], Some(90));
    assert_eq!(
        (since.as_str(), per["C"].as_str()),
        ("2026-08-14", "2026-08-14")
    );
    // a login not synced for longer than the period still closes its gap
    let stale = [("A".to_string(), Some("2026-01-01".to_string()))];
    let (since, _) = since_window(date("2026-11-12"), 730, &stale, Some(90));
    assert_eq!(since, "2025-12-22");
}

#[test]
fn period_is_capped_per_connector() {
    assert_eq!(HistoryRange::parse("max"), HistoryRange::Maximum);
    assert_eq!(
        HistoryRange::parse(" 365 "),
        HistoryRange::Days { days: 365 }
    );
    // a value from an older version, a typo or no days at all: the earlier default
    assert_eq!(HistoryRange::parse("viel"), HistoryRange::Days { days: 90 });
    assert_eq!(HistoryRange::parse("0"), HistoryRange::Days { days: 90 });
    // FinTS: the bank's storage period cuts "Maximum" further
    assert_eq!(
        HistoryRange::Maximum.days(ConnectorKind::FinTsGeneric),
        3650
    );
    // PSD2: an explicit start date beyond 90 days needs a fresh consent
    assert_eq!(HistoryRange::Maximum.days(ConnectorKind::EnableBanking), 90);
    // the start date never reaches the bank, or a refusal is tried again
    assert_eq!(
        HistoryRange::Maximum.days(ConnectorKind::TradeRepublic),
        3650
    );
    assert_eq!(HistoryRange::Maximum.days(ConnectorKind::Comdirect), 3650);
    // three years of transaction search
    assert_eq!(HistoryRange::Maximum.days(ConnectorKind::PayPal), 1090);
    let long = HistoryRange::Days { days: 5000 };
    assert_eq!(long.days(ConnectorKind::OpelDirektbank), 730);
    let short = HistoryRange::Days { days: 30 };
    assert_eq!(short.days(ConnectorKind::FinTsGeneric), 30);
}

#[test]
fn a_refused_start_date_is_tried_again_with_90_days() {
    let back = |n: i64| iso(today() - days(n));
    let bank = |code: &str| CoreError::Bank {
        code: code.into(),
        message: "Zeitraum ungültig".into(),
    };
    assert_eq!(
        history::retry_since(&back(3650), bank("400")).unwrap(),
        back(90)
    );
    // the bank's own code, or a GraphQL error with HTTP 200
    assert_eq!(
        history::retry_since(&back(365), bank("Error")).unwrap(),
        back(90)
    );
    assert_eq!(
        history::retry_since(&back(365), bank("200")).unwrap(),
        back(90)
    );
    // within 90 days a shorter period would not help
    assert!(matches!(
        history::retry_since(&back(30), bank("400")),
        Err(CoreError::Bank { .. })
    ));
    // not a refused request: the error stands
    for err in [
        bank("429"),
        bank("401"),
        bank("503"),
        CoreError::Protocol {
            message: "list incomplete".into(),
        },
        CoreError::Authentication {
            message: "abgelaufen".into(),
        },
        CoreError::Network {
            message: "offline".into(),
        },
    ] {
        assert!(history::retry_since(&back(3650), err).is_err());
    }
    let w = history::refused_warning("DE00ACC");
    assert!(w.starts_with("DE00ACC: ") && w.contains("90"), "{w}");
}

#[test]
fn a_new_login_is_marked_for_a_full_fetch() {
    let (v, path) = demo_vault();
    let first = v
        .add_bank_connection(ConnectorKind::Comdirect, String::new(), "one".into())
        .unwrap();
    assert_eq!(
        v.pending_full_history(first).unwrap(),
        Some(HistoryRange::Maximum),
        "default: as far back as the bank delivers"
    );
    v.set_setting("sync.days_back".into(), "365".into())
        .unwrap();
    let second = v
        .add_fints_connection(
            "12030000".into(),
            "https://fints.example.com/fints".into(),
            String::new(),
            "two".into(),
            String::new(),
        )
        .unwrap();
    assert_eq!(
        v.pending_full_history(second).unwrap(),
        Some(HistoryRange::Days { days: 365 })
    );
    // the same login added again while it exists: nothing is marked anew
    {
        let db = v.db().unwrap();
        history::finish(&db, first, "max").unwrap();
    }
    let again = v
        .add_bank_connection(ConnectorKind::Comdirect, String::new(), "one".into())
        .unwrap();
    assert_eq!(again, first);
    assert_eq!(v.pending_full_history(first).unwrap(), None);
    // deleted: forgotten; added again: a new full fetch
    v.request_full_history(first, HistoryRange::Days { days: 30 })
        .unwrap();
    v.delete_bank_connection(first).unwrap();
    assert_eq!(v.pending_full_history(first).unwrap(), None);
    assert!(matches!(
        v.request_full_history(first, HistoryRange::Maximum),
        Err(CoreError::NotFound { .. })
    ));
    let revived = v
        .add_bank_connection(ConnectorKind::Comdirect, String::new(), "one".into())
        .unwrap();
    assert_eq!(revived, first);
    assert_eq!(
        v.pending_full_history(first).unwrap(),
        Some(HistoryRange::Days { days: 365 })
    );
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn only_a_full_user_sync_reaches_back_and_finishes_it() {
    let (v, path) = demo_vault();
    let conn = v
        .list_bank_connections()
        .unwrap()
        .into_iter()
        .find(|c| c.connector == ConnectorKind::VolksbankKasselGoettingen)
        .unwrap();
    let giro = v
        .list_accounts()
        .unwrap()
        .into_iter()
        .find(|a| a.name == "Musterbank Girokonto" && a.connection_id == Some(conn.id))
        .unwrap();
    let back = |n: i64| iso(today() - days(n));
    let since = |s: &SyncSession| s.clone_context().unwrap().since;
    // an existing login goes on from its newest bookings
    let incremental = since(&v.clone().begin_sync(conn.id, "pin".into(), true).unwrap());
    assert!(incremental > back(400));
    v.request_full_history(conn.id, HistoryRange::Days { days: 400 })
        .unwrap();
    // neither an automatic sync nor one of a single account reaches back
    let quiet = v
        .clone()
        .begin_quiet_sync(conn.id, "pin".into(), true)
        .unwrap();
    assert_eq!(since(&quiet), incremental);
    let account = v
        .clone()
        .begin_account_sync(giro.id, "pin".into(), true)
        .unwrap();
    assert!(since(&account) > back(400));
    // a full sync the user started does, for every account
    let s = v.clone().begin_sync(conn.id, "pin".into(), true).unwrap();
    let ctx = s.clone_context().unwrap();
    assert_eq!(ctx.since, back(400));
    assert!(ctx.since_by_account.values().all(|d| *d <= back(400)));
    let mut step = s.start().await.unwrap();
    while !matches!(step, SyncStep::Finished { .. }) {
        step = s.poll().await.unwrap();
    }
    // stored: the login goes on incrementally
    assert_eq!(v.pending_full_history(conn.id).unwrap(), None);
    let next = since(&v.clone().begin_sync(conn.id, "pin".into(), true).unwrap());
    assert!(next > back(400));
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_full_fetch_asks_enable_banking_for_the_longest_history_again() {
    use crate::secret::SecretJson;
    use serde_json::{Value, json};
    let legacy = json!(["uid", "Giro", "DE00", "EUR"]);
    let state = SecretJson::encode(&json!({"session_id": "s", "accounts": [
        {"uid": "u1", "history_done": true},
        {"uid": "u2", "history_done": false},
        legacy,
    ]}))
    .unwrap();
    let reset: Value = history::state_for_full_fetch(state)
        .unwrap()
        .decode()
        .unwrap();
    assert_eq!(reset["accounts"][0]["history_done"], false);
    assert_eq!(reset["accounts"][1]["history_done"], false);
    assert_eq!(
        (&reset["session_id"], &reset["accounts"][2]),
        (&json!("s"), &legacy)
    );
    // nothing to ask again: the state stays as it is, an empty one too
    let other = json!({"system_id": "x", "accounts": [{"uid": "u", "history_done": false}]});
    let out = history::state_for_full_fetch(SecretJson::encode(&other).unwrap()).unwrap();
    assert_eq!(out.decode::<Value>().unwrap(), other);
    let empty = history::state_for_full_fetch(SecretJson::default()).unwrap();
    assert!(empty.is_empty());
}

#[test]
fn a_pending_full_fetch_can_be_called_off() {
    let (v, path) = demo_vault();
    let id = v
        .add_bank_connection(ConnectorKind::Comdirect, String::new(), "one".into())
        .unwrap();
    assert!(v.pending_full_history(id).unwrap().is_some());
    v.cancel_full_history(id).unwrap();
    assert_eq!(v.pending_full_history(id).unwrap(), None);
    let _ = std::fs::remove_file(path);
}
