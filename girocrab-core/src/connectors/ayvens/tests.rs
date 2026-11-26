use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use serde_json::json;

use super::*;
use crate::secret::SecretString;

// Invented data in the shape of the API; no real data.

#[test]
fn booking_takes_day_sign_and_reference_account() {
    let deposit = parse_booking(
        "ayvens:1",
        &json!({"id": "DE02120300000000202051", "date": "2026-10-01T00:00:00.000Z",
                "reference": "DE02120300000000202051", "description": "", "amount": 2500}),
    )
    .unwrap();
    assert_eq!(deposit.booking_date, "2026-10-01");
    assert_eq!(deposit.amount_cents, 250_000);
    assert_eq!(deposit.counterparty_iban, "DE02120300000000202051");
    assert_eq!(deposit.counterparty, "");
    assert_eq!(deposit.purpose, "Einzahlung");
    assert_eq!(deposit.booking_text, "Einzahlung");
    assert!(deposit.external_id.is_empty(), "content id from the vault");

    let interest = parse_booking(
        "ayvens:1",
        &json!({"id": "Interest-0000000000000", "date": "2026-10-31T00:00:00.000Z",
                "reference": "", "description": "Zinsen", "amount": 4.17}),
    )
    .unwrap();
    assert_eq!(interest.amount_cents, 417);
    assert_eq!(interest.counterparty, "Ayvens Bank");
    assert_eq!(interest.purpose, "Zinsen");

    let withdrawal = parse_booking(
        "ayvens:1",
        &json!({"date": "2026-11-02T00:00:00.000Z", "reference": "DE02120300000000202051",
                "description": "", "amount": -100.5}),
    )
    .unwrap();
    assert_eq!(withdrawal.amount_cents, -10_050);
    assert_eq!(withdrawal.purpose, "Auszahlung");
    assert_eq!(withdrawal.booking_text, "Auszahlung");
}

#[test]
fn purpose_does_not_follow_the_app_language() {
    // the purpose goes into the content id: the same booking must keep it in English
    let t = json!({"date": "2026-10-01T00:00:00.000Z", "reference": "DE02120300000000202051",
                   "description": "", "amount": 2500});
    let de = parse_booking("a", &t).unwrap();
    crate::i18n::with_language(crate::i18n::Lang::En, || {
        let en = parse_booking("a", &t).unwrap();
        assert_eq!(en.content_key(), de.content_key());
        assert_eq!(en.booking_text, "Deposit", "the booking text is translated");
    });
}

#[test]
fn booking_without_date_or_amount_is_rejected() {
    assert!(parse_booking("a", &json!({"date": "", "amount": 1})).is_none());
    assert!(parse_booking("a", &json!({"date": "2026-10-01T00:00:00.000Z"})).is_none());
    assert!(
        parse_booking(
            "a",
            &json!({"date": "2026-10-01T00:00:00.000Z", "amount": "x"})
        )
        .is_none()
    );
}

#[test]
fn account_without_balance_is_rejected_never_zero() {
    assert!(
        parse_account(&json!({"id": "1234567", "productNumber": "NL91ABNA0417164300"})).is_none()
    );
    let (id, a) = parse_account(
        &json!({"id": "1234567", "productNumber": "NL91ABNA0417164300",
                                         "interestRate": 2.3, "amount": 0, "accruedInterest": 0.0}),
    )
    .unwrap();
    assert_eq!(id, "1234567");
    assert_eq!(a.balance_cents, 0, "a balance the bank reports as 0 is 0");
    assert_eq!(a.iban, "NL91ABNA0417164300");
    assert_eq!(a.external_ref, "ayvens:1234567");
}

#[test]
fn refused_callback_is_detected() {
    assert!(callback_refused(
        &json!({"url": "https://sparen.ayvensbank.de/api/auth/error?error=CredentialsSignin"})
    ));
    assert!(callback_refused(&json!(null)));
    assert!(!callback_refused(
        &json!({"url": "https://sparen.ayvensbank.de/"})
    ));
}

// ------------------------------------------------------------------ mock bank

/// Reads one HTTP request (headers and `Content-Length` body).
fn read_request(s: &mut std::net::TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = s.read(&mut chunk).unwrap_or(0);
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        let text = String::from_utf8_lossy(&buf);
        if let Some(end) = text.find("\r\n\r\n") {
            let len = text[..end]
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                })
                .unwrap_or(0);
            if buf.len() >= end + 4 + len {
                break;
            }
        }
    }
    String::from_utf8_lossy(&buf).to_string()
}

/// A mock of the online banking: user `erika` / `geheim`, SMS code `123456`.
fn serve() -> (String, Arc<Mutex<Vec<String>>>) {
    serve_with(false)
}

/// `two_accounts`: a second account whose list fails, and per-account lists.
fn serve_with(two_accounts: bool) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let log = Arc::new(Mutex::new(Vec::new()));
    let log2 = log.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { break };
            let req = read_request(&mut s);
            let first = req.lines().next().unwrap_or("").to_string();
            log2.lock().unwrap().push(first.clone());
            let cookies = req
                .lines()
                .find_map(|l| {
                    l.strip_prefix("cookie: ")
                        .or_else(|| l.strip_prefix("Cookie: "))
                })
                .unwrap_or("")
                .to_string();
            let logged_in = cookies.contains("session=1");
            let otp = cookies.contains("otp=1");
            let body = req.split("\r\n\r\n").nth(1).unwrap_or("");
            let mut set_cookie = "";
            let (status, json) = if first.starts_with("GET /api/auth/csrf") {
                set_cookie = "csrf=1; Path=/";
                ("200 OK", json!({"csrfToken": "tok"}))
            } else if first.starts_with("POST /api/auth/callback/credentials") {
                if body.contains("username=erika")
                    && body.contains("password=geheim")
                    && body.contains("csrfToken=tok")
                {
                    set_cookie = "session=1; Path=/";
                    ("200 OK", json!({"url": "http://localhost/"}))
                } else {
                    (
                        "200 OK",
                        json!({"url": "http://localhost/api/auth/error?error=CredentialsSignin&code=credentials"}),
                    )
                }
            } else if first.starts_with("GET /api/auth/session") {
                if logged_in {
                    (
                        "200 OK",
                        json!({"user": {"username": "erika", "isOtpVerified": otp}}),
                    )
                } else {
                    ("200 OK", json!(null))
                }
            } else if first.starts_with("POST /api/savings/send-sms") && logged_in {
                ("200 OK", json!({}))
            } else if first.starts_with("POST /api/auth/callback/otp-auth") && logged_in {
                if body.contains("otp=123456") && body.contains("username=erika") {
                    set_cookie = "otp=1; Path=/";
                    ("200 OK", json!({"url": "http://localhost/"}))
                } else {
                    (
                        "200 OK",
                        json!({"url": "http://localhost/api/auth/error?error=CredentialsSignin"}),
                    )
                }
            } else if !(logged_in && otp) {
                ("401 Unauthorized", json!({}))
            } else if first.starts_with("GET /api/savings/account-current") {
                let mut accounts = vec![
                    json!({"id": "1234567", "productNumber": "NL91ABNA0417164300",
                    "interestRate": 2.3, "amount": 2504.17, "accruedInterest": 1.2}),
                ];
                if two_accounts {
                    accounts.push(
                        json!({"id": "7654321", "productNumber": "NL02ABNA0123456789",
                        "interestRate": 2.3, "amount": 100, "accruedInterest": 0}),
                    );
                }
                ("200 OK", json!({"accounts": accounts}))
            } else if first.starts_with("GET /api/savings/account-transactions?accountId=7654321") {
                ("500 Internal Server Error", json!({}))
            } else if first.starts_with("GET /api/savings/account-transactions") {
                // empty for the Tagesgeld account, as seen live
                ("200 OK", json!([]))
            } else if first.starts_with("GET /api/savings/transactions") {
                (
                    "200 OK",
                    json!([
                        {"id": "Interest-0000000000000", "date": "2026-10-31T00:00:00.000Z", "reference": "", "description": "Zinsen", "amount": 4.17},
                        {"id": "Interest-0000000000000", "date": "2026-09-30T00:00:00.000Z", "reference": "", "description": "Zinsen", "amount": 4.17},
                        {"id": "DE02120300000000202051", "date": "2026-09-01T00:00:00.000Z", "reference": "DE02120300000000202051", "description": "", "amount": 2500}
                    ]),
                )
            } else if first.starts_with("POST /api/savings/logout") {
                ("200 OK", json!({}))
            } else {
                ("404 Not Found", json!({}))
            };
            let body = json.to_string();
            let cookie_line = if set_cookie.is_empty() {
                String::new()
            } else {
                format!("Set-Cookie: {set_cookie}\r\n")
            };
            let _ = write!(
                s,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{cookie_line}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (base, log)
}

fn ctx(login: &str, password: &str, since: &str) -> ConnectContext {
    ConnectContext {
        connection_id: 1,
        login_name: login.into(),
        secret: SecretString::new(password.into()),
        since: since.into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn login_with_sms_code_then_fetch() {
    let (base, log) = serve();
    let mut c = AyvensConnector::with_base(base);
    let ctx = ctx("erika", "geheim", "2026-09-15");
    let phase = c.begin(&ctx).await.unwrap();
    assert!(matches!(&phase, Phase::Challenge(ch) if ch.kind == ChallengeKind::Code));
    let phase = c
        .respond(
            &ctx,
            ChallengeResponse::Code(SecretString::new("123456".into())),
        )
        .await
        .unwrap();
    assert!(matches!(phase, Phase::Ready));
    let out = c.fetch(&ctx).await.unwrap();
    assert_eq!(out.accounts.len(), 1);
    assert_eq!(out.accounts[0].balance_cents, 250_417);
    assert_eq!(out.balances_reported, vec!["ayvens:1234567".to_string()]);
    // the deposit of 2026-09-01 lies before `since`
    assert_eq!(out.transactions.len(), 2);
    assert!(out.transactions.iter().all(|t| t.purpose == "Zinsen"));
    c.finish(&ctx).await;
    let log = log.lock().unwrap().clone();
    assert!(
        log.iter()
            .any(|r| r.starts_with("POST /api/savings/send-sms"))
    );
    assert!(
        log.iter()
            .any(|r| r.starts_with("POST /api/savings/logout"))
    );
}

#[tokio::test]
async fn wrong_password_is_an_authentication_error_without_sms() {
    let (base, log) = serve();
    let mut c = AyvensConnector::with_base(base);
    let err = c
        .begin(&ctx("erika", "falsch", "2026-01-01"))
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::Authentication { .. }), "{err:?}");
    assert!(!log.lock().unwrap().iter().any(|r| r.contains("send-sms")));
}

#[tokio::test]
async fn wrong_sms_code_is_an_authentication_error() {
    let (base, _log) = serve();
    let mut c = AyvensConnector::with_base(base);
    let ctx = ctx("erika", "geheim", "2026-01-01");
    c.begin(&ctx).await.unwrap();
    let err = c
        .respond(
            &ctx,
            ChallengeResponse::Code(SecretString::new("000000".into())),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::Authentication { .. }), "{err:?}");
    assert!(
        c.fetch(&ctx).await.is_err(),
        "no data without the second factor"
    );
}

#[tokio::test]
async fn one_failed_account_keeps_the_others() {
    let (base, _log) = serve_with(true);
    let mut c = AyvensConnector::with_base(base);
    let ctx = ctx("erika", "geheim", "2026-01-01");
    c.begin(&ctx).await.unwrap();
    c.respond(
        &ctx,
        ChallengeResponse::Code(SecretString::new("123456".into())),
    )
    .await
    .unwrap();
    let out = c.fetch(&ctx).await.unwrap();
    // both balances, no bookings: one list is empty, the other failed
    assert_eq!(out.accounts.len(), 2);
    assert_eq!(out.balances_reported.len(), 2);
    assert!(out.transactions.is_empty());
    assert_eq!(out.warnings.len(), 2, "{:?}", out.warnings);
    assert!(
        out.warnings
            .iter()
            .any(|w| w.contains("ayvens:1234567") && w.contains("keine Umsätze"))
    );
    assert!(
        out.warnings
            .iter()
            .any(|w| w.contains("ayvens:7654321") && w.contains("nicht geladen"))
    );
}

#[tokio::test]
async fn a_failed_list_in_the_full_fetch_points_to_the_period_setting() {
    // stored with warnings the full fetch counts as done: later syncs do not reach back
    let (base, _log) = serve_with(true);
    let mut c = AyvensConnector::with_base(base);
    let ctx = ConnectContext {
        full_fetch: true,
        ..ctx("erika", "geheim", "2016-01-01")
    };
    c.begin(&ctx).await.unwrap();
    c.respond(
        &ctx,
        ChallengeResponse::Code(SecretString::new("123456".into())),
    )
    .await
    .unwrap();
    let out = c.fetch(&ctx).await.unwrap();
    assert_eq!(out.accounts.len(), 2);
    let failed = out
        .warnings
        .iter()
        .find(|w| w.contains("ayvens:7654321"))
        .unwrap();
    assert!(failed.contains("Beim nächsten Abruf"), "{failed}");
    assert!(!failed.contains("holt sie nach"), "{failed}");
}
