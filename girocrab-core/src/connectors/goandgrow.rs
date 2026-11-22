//! Go & Grow (goandgrow.eu, formerly Bondora Go & Grow): balance and bookings
//! via the JSON API of the investor web app (unofficial, may change at any time).
//!
//!  GET  https://app.goandgrow.eu/en/login   -> redirects to the Keycloak login form
//!                                              (`form#kc-form-login` on sso.goandgrow.eu)
//!  POST <form action>                       username, password (exactly once: the
//!                                              session code in the action is single use)
//!                                              -> OIDC form_post page: a form to /signin-oidc
//!  POST https://app.goandgrow.eu/signin-oidc -> the app's session cookies
//!  GET  https://api.prd.goandgrow.eu/investor/api/v2/gogrow
//!                                           -> [{ Id, ReferenceNumber, Name, ClientValue, … }]
//!  GET  …/investor/api/v2/statements        -> [{ Type, Description, Amount, Date, Balance }]
//!
//! The API authenticates by cookie only. `Amount` is always positive, the
//! direction comes from `Type`. The statement list holds only the latest
//! ten bookings and no booking ids, so bookings get content ids and the
//! account has to be synced at least every ten bookings (≈ ten days).
//! Redirects are followed by hand and only between the Go & Grow hosts, so
//! the password and the session cookies never reach another host.

use async_trait::async_trait;
use serde_json::Value;
use zeroize::Zeroize;

use super::html::{FORM, INPUT, attr, attrs};
use super::{
    BankConnector, ChallengeResponse, ConnectContext, ContentIds, FetchResult, Phase,
    RemoteAccount, RemoteTransaction,
};
use crate::error::{CoreError, CoreResult};
use crate::i18n::{tr, trf};
use crate::models::AccountKind;
use crate::money::json_number_to_cents_rounded;

const APP: &str = "https://app.goandgrow.eu";
const API: &str = "https://api.prd.goandgrow.eu/investor/api/v2";
/// Hosts that may receive the password or the session cookies.
const HOSTS: &[&str] = &[
    "app.goandgrow.eu",
    "sso.goandgrow.eu",
    "sso.bondora.com",
    "api.prd.goandgrow.eu",
];
/// Account external refs: `goandgrow:<account id>`.
pub(crate) const REF_PREFIX: &str = "goandgrow:";
/// Counterparty of every booking, as the bookings imported from other
/// programs carry it, so the first sync adopts them.
const COUNTERPARTY: &str = "Go & Grow";
const MAX_REDIRECTS: usize = 10;

fn page_changed() -> CoreError {
    CoreError::protocol(tr(
        "Go & Grow: Anmeldeseite nicht erkannt (Seite geändert?).",
    ))
}

fn wrong_credentials() -> CoreError {
    CoreError::Authentication {
        message: tr(
            "Go & Grow: Anmeldung fehlgeschlagen. Bitte E-Mail-Adresse und Passwort prüfen.",
        )
        .into(),
    }
}

fn unexpected(status: u16) -> CoreError {
    CoreError::protocol(trf(
        "Go & Grow: Unerwartete Antwort der API (HTTP {0}).",
        &[&status],
    ))
}

/// Only https to the Go & Grow hosts.
fn ensure_allowed(url: &url::Url) -> CoreResult<()> {
    if url.scheme() == "https"
        && url.port().is_none()
        && url.host_str().is_some_and(|h| HOSTS.contains(&h))
    {
        return Ok(());
    }
    Err(CoreError::protocol(trf(
        "Go & Grow: Weiterleitung zu einer unbekannten Adresse ({0}) abgebrochen.",
        &[&url.origin().ascii_serialization()],
    )))
}

/// A redirect target: relative to `base`; plain `http` to one of the Go & Grow
/// hosts (the app sits behind a proxy and sometimes names its own address
/// with `http`) is taken as `https`, nothing is ever sent unencrypted.
fn redirect_target(base: &url::Url, location: &str) -> CoreResult<url::Url> {
    let mut next = base.join(location).map_err(|_| page_changed())?;
    if next.scheme() == "http" && next.host_str().is_some_and(|h| HOSTS.contains(&h)) {
        next.set_scheme("https").map_err(|_| page_changed())?;
        if next.port() == Some(80) {
            next.set_port(None).map_err(|_| page_changed())?;
        }
    }
    ensure_allowed(&next)?;
    Ok(next)
}

// ------------------------------------------------------------------ parsing

/// A page form as the browser would submit it: absolute action and the
/// hidden, text and e-mail fields with their values.
#[derive(Debug, PartialEq)]
struct PageForm {
    action: url::Url,
    fields: Vec<(String, String)>,
}

impl PageForm {
    fn set(&mut self, name: &str, value: &str) {
        match self.fields.iter_mut().find(|(n, _)| n == name) {
            Some((_, v)) => *v = value.to_string(),
            None => self.fields.push((name.to_string(), value.to_string())),
        }
    }
}

/// The first POST form whose attributes satisfy `pick`, with its action
/// resolved against `page` and checked against the allowed hosts.
fn find_form(
    html: &str,
    page: &url::Url,
    pick: impl Fn(&[(String, String)], &url::Url) -> bool,
) -> Option<PageForm> {
    FORM.captures_iter(html).find_map(|f| {
        let a = attrs(&f[1]);
        if attr(&a, "method").is_some_and(|m| !m.eq_ignore_ascii_case("post")) {
            return None; // never put the password into a URL
        }
        let action = match attr(&a, "action").map(str::trim) {
            Some(act) if !act.is_empty() => page.join(act).ok()?,
            _ => page.clone(),
        };
        if !pick(&a, &action) || ensure_allowed(&action).is_err() {
            return None;
        }
        let fields = INPUT
            .captures_iter(&f[2])
            .filter_map(|i| {
                let ia = attrs(&i[1]);
                let name = attr(&ia, "name").filter(|n| !n.is_empty())?;
                let kind = attr(&ia, "type").unwrap_or("text").to_ascii_lowercase();
                matches!(kind.as_str(), "hidden" | "text" | "email").then(|| {
                    (
                        name.to_string(),
                        attr(&ia, "value").unwrap_or("").to_string(),
                    )
                })
            })
            .collect();
        Some(PageForm { action, fields })
    })
}

/// Keycloak's login form.
fn login_form(html: &str, page: &url::Url) -> Option<PageForm> {
    find_form(html, page, |a, _| attr(a, "id") == Some("kc-form-login"))
}

/// The OIDC form_post page Keycloak answers a successful login with.
fn signin_form(html: &str, page: &url::Url) -> Option<PageForm> {
    find_form(html, page, |_, action| {
        action.host_str() == Some("app.goandgrow.eu") && action.path().ends_with("/signin-oidc")
    })
}

/// Booking text and sign for a statement `Type`.
fn classify(kind: &str) -> Option<(i64, String)> {
    match kind {
        "Return" => Some((1, tr("Zinsen").into())),
        "Deposit" => Some((1, tr("Einzahlung").into())),
        "Withdrawal" | "WithdrawalRequest" | "PartialPayout" => Some((-1, tr("Auszahlung").into())),
        "Fee" => Some((-1, tr("Gebühr").into())),
        _ => None,
    }
}

/// One statement entry without its id; `None` without a usable date or amount.
fn parse_statement(t: &Value, account_ref: &str) -> Option<RemoteTransaction> {
    // "2026-07-08T00:00:00[.303]": the calendar day is the booking date
    let date = t["Date"].as_str()?.get(..10)?;
    crate::dates::parse_iso(date)?;
    let kind = t["Type"].as_str().unwrap_or_default();
    let (sign, booking_text) = classify(kind)?;
    let amount_cents = sign * json_number_to_cents_rounded(&t["Amount"]).ok()?.abs();
    let description = t["Description"].as_str().map(str::trim).unwrap_or_default();
    Some(RemoteTransaction {
        account_ref: account_ref.into(),
        booking_date: date.into(),
        value_date: date.into(),
        amount_cents,
        currency: "EUR".into(),
        counterparty: COUNTERPARTY.into(),
        purpose: if description.is_empty() {
            kind.to_string()
        } else {
            description.to_string()
        },
        booking_text,
        ..Default::default()
    })
}

/// The Go & Grow account of `/gogrow` (the first entry).
fn parse_account(v: &Value) -> CoreResult<RemoteAccount> {
    let a = v
        .as_array()
        .and_then(|l| l.first())
        .ok_or_else(|| CoreError::protocol(tr("Go & Grow: Kein Konto in der Antwort gefunden.")))?;
    let id = match &a["Id"] {
        Value::String(s) if !s.trim().is_empty() => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        _ => "account".into(),
    };
    // "6007939 Erika Mustermann GG1234567": the leading number identifies the account
    let number = a["ReferenceNumber"]
        .as_str()
        .and_then(|r| r.split_whitespace().next())
        .unwrap_or_default();
    Ok(RemoteAccount {
        external_ref: format!("{REF_PREFIX}{id}"),
        name: a["Name"].as_str().unwrap_or("Go & Grow").trim().to_string(),
        owner: String::new(),
        iban: String::new(),
        account_number: number.to_string(),
        currency: "EUR".into(),
        kind: AccountKind::Savings,
        balance_cents: json_number_to_cents_rounded(&a["ClientValue"])?,
        balance_date: super::today(),
    })
}

// ------------------------------------------------------------------ connector

#[derive(Default)]
pub struct GoAndGrowConnector {
    client: Option<reqwest::Client>,
}

impl GoAndGrowConnector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sends `req`, then follows redirects (as GET) between the allowed hosts.
    /// Returns the final URL and the page.
    async fn send_following(
        client: &reqwest::Client,
        req: reqwest::RequestBuilder,
    ) -> CoreResult<(url::Url, u16, String)> {
        let mut resp = req.send().await?;
        for _ in 0..MAX_REDIRECTS {
            if !resp.status().is_redirection() {
                break;
            }
            let location = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|l| l.to_str().ok())
                .ok_or_else(page_changed)?;
            let next = redirect_target(resp.url(), location)?;
            resp = client.get(next).send().await?;
        }
        if resp.status().is_redirection() {
            return Err(page_changed());
        }
        let url = resp.url().clone();
        ensure_allowed(&url)?;
        let status = resp.status().as_u16();
        Ok((url, status, super::read_text_limited(resp).await?))
    }

    async fn api(&self, path: &str) -> CoreResult<Value> {
        let client = self.client.as_ref().ok_or(CoreError::SessionClosed)?;
        let resp = client
            .get(format!("{API}{path}"))
            .header("Accept", "application/json")
            .header("Origin", APP)
            .header("Referer", format!("{APP}/"))
            .send()
            .await?;
        let status = resp.status().as_u16();
        // the API redirects to the login when the session is gone
        if matches!(status, 401 | 403) || resp.status().is_redirection() {
            return Err(CoreError::Authentication {
                message: tr("Go & Grow: Sitzung abgelaufen. Bitte erneut abrufen.").into(),
            });
        }
        let v = super::read_json_or_null(resp).await;
        if status != 200 || v.is_null() {
            return Err(unexpected(status));
        }
        Ok(v)
    }
}

#[async_trait]
impl BankConnector for GoAndGrowConnector {
    async fn begin(&mut self, ctx: &ConnectContext) -> CoreResult<Phase> {
        let login = ctx.login_name.trim();
        if login.is_empty() || ctx.secret.expose().is_empty() {
            return Err(CoreError::Authentication {
                message: tr("Go & Grow: E-Mail-Adresse und Passwort erforderlich.").into(),
            });
        }
        let client = super::client_builder(std::time::Duration::from_secs(60))
            .user_agent(super::BROWSER_UA)
            .cookie_store(true)
            .redirect(reqwest::redirect::Policy::none())
            .build()?;

        // 1. the app sends us to Keycloak's login form
        let (page, _, html) = Self::send_following(
            &client,
            client
                .get(format!("{APP}/en/login"))
                .header("Accept", "text/html,application/xhtml+xml"),
        )
        .await?;
        let mut form = login_form(&html, &page).ok_or_else(page_changed)?;
        form.set("username", login);
        form.set("password", ctx.secret.expose());
        if !form.fields.iter().any(|(n, _)| n == "credentialId") {
            form.set("credentialId", "");
        }

        // 2. submit it exactly once; the answer is the OIDC form_post page
        let sent =
            Self::send_following(&client, client.post(form.action.clone()).form(&form.fields))
                .await;
        for (_, v) in form.fields.iter_mut() {
            v.zeroize();
        }
        let (page, _, html) = sent?;
        let Some(signin) = signin_form(&html, &page) else {
            return Err(if login_form(&html, &page).is_some() {
                wrong_credentials()
            } else {
                page_changed()
            });
        };

        // 3. hand the tokens to the app, which sets its session cookies
        Self::send_following(
            &client,
            client.post(signin.action.clone()).form(&signin.fields),
        )
        .await?;
        self.client = Some(client);

        // 4. the API must know us now
        self.api("/gogrow").await.map_err(|e| match e {
            CoreError::Authentication { .. } => wrong_credentials(),
            other => other,
        })?;
        Ok(Phase::Ready)
    }

    async fn respond(&mut self, _ctx: &ConnectContext, _r: ChallengeResponse) -> CoreResult<Phase> {
        Ok(Phase::Ready)
    }

    async fn fetch(&mut self, ctx: &ConnectContext) -> CoreResult<FetchResult> {
        let account = parse_account(&self.api("/gogrow").await?)?;
        let statements = self.api("/statements").await?;
        let since = ctx.since_for(&account.external_ref).to_string();
        let mut ids = ContentIds::default();
        let mut out = FetchResult::default();
        let Some(statements) = statements.as_array() else {
            return Err(unexpected(200));
        };
        for t in statements {
            let Some(mut b) = parse_statement(t, &account.external_ref) else {
                return Err(CoreError::invalid(tr(
                    "Go & Grow: Unvollständige Buchung in der Antwort. Es wurden keine Daten übernommen.",
                )));
            };
            b.external_id = ids.next(&b);
            if b.booking_date.as_str() >= since.as_str() {
                out.transactions.push(b);
            }
        }
        out.balances_reported.push(account.external_ref.clone());
        out.accounts.push(account);
        out.accounts_complete = true;
        Ok(out)
    }

    async fn finish(&mut self, _ctx: &ConnectContext) {
        self.client = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Invented pages and entries in the shape of the real ones; no real data.
    const KEYCLOAK: &str = r#"<html><body>
      <form id="kc-form-login" onsubmit="return true;" action="https://sso.goandgrow.eu/realms/goandgrow/login-actions/authenticate?session_code=abc&amp;execution=x1&amp;client_id=investor-system" method="post">
        <input tabindex="1" id="username" name="username" value="" type="text" autofocus>
        <input tabindex="2" id="password" name="password" type="password">
        <input type="checkbox" id="rememberMe" name="rememberMe">
        <input type="hidden" id="id-hidden-input" name="credentialId">
        <input tabindex="4" name="login" id="kc-login" type="submit" value="Sign In">
      </form></body></html>"#;

    const FORM_POST: &str = r#"<HTML><BODY Onload="document.forms[0].submit()">
      <FORM METHOD="POST" ACTION="https://app.goandgrow.eu/signin-oidc">
        <INPUT TYPE="HIDDEN" NAME="code" VALUE="c0de">
        <INPUT TYPE="HIDDEN" NAME="id_token" VALUE="ey.tok">
        <INPUT TYPE="HIDDEN" NAME="state" VALUE="st&amp;1">
        <INPUT TYPE="HIDDEN" NAME="session_state" VALUE="s5">
      </FORM></BODY></HTML>"#;

    fn sso_page() -> url::Url {
        url::Url::parse(
            "https://sso.goandgrow.eu/realms/goandgrow/protocol/openid-connect/auth?x=1",
        )
        .unwrap()
    }

    #[test]
    fn keycloak_form_resolves_action_and_keeps_fields() {
        let mut f = login_form(KEYCLOAK, &sso_page()).unwrap();
        assert_eq!(f.action.host_str(), Some("sso.goandgrow.eu"));
        assert!(f.action.as_str().contains("session_code=abc&execution=x1"));
        f.set("username", "erika@example.org");
        f.set("password", "geheim");
        assert_eq!(
            f.fields,
            vec![
                ("username".to_string(), "erika@example.org".to_string()),
                ("credentialId".into(), String::new()),
                ("password".into(), "geheim".into()),
            ]
        );
    }

    #[test]
    fn signin_form_is_found_on_the_form_post_page_only() {
        let f = signin_form(FORM_POST, &sso_page()).unwrap();
        assert_eq!(f.action.as_str(), "https://app.goandgrow.eu/signin-oidc");
        assert_eq!(f.fields.len(), 4);
        assert_eq!(f.fields[2], ("state".to_string(), "st&1".to_string()));
        assert!(signin_form(KEYCLOAK, &sso_page()).is_none());
        assert!(login_form(FORM_POST, &sso_page()).is_none());
    }

    #[test]
    fn forms_to_other_hosts_or_via_get_are_ignored() {
        let other = KEYCLOAK.replace("sso.goandgrow.eu", "evil.example.org");
        assert!(login_form(&other, &sso_page()).is_none());
        let get = KEYCLOAK.replace(r#"method="post""#, r#"method="get""#);
        assert!(login_form(&get, &sso_page()).is_none());
        let http = FORM_POST.replace("https://app", "http://app");
        assert!(signin_form(&http, &sso_page()).is_none());
    }

    #[test]
    fn http_redirects_to_own_hosts_become_https_others_are_refused() {
        let base = url::Url::parse("https://app.goandgrow.eu/signin-oidc").unwrap();
        assert_eq!(
            redirect_target(&base, "http://app.goandgrow.eu/en")
                .unwrap()
                .as_str(),
            "https://app.goandgrow.eu/en"
        );
        assert_eq!(
            redirect_target(&base, "http://app.goandgrow.eu:80/en")
                .unwrap()
                .as_str(),
            "https://app.goandgrow.eu/en"
        );
        assert_eq!(
            redirect_target(&base, "/en?x=1").unwrap().as_str(),
            "https://app.goandgrow.eu/en?x=1"
        );
        assert!(redirect_target(&base, "http://evil.example.org/").is_err());
        assert!(redirect_target(&base, "https://app.goandgrow.eu.evil.example.org/").is_err());
        assert!(redirect_target(&base, "http://app.goandgrow.eu:8080/").is_err());
    }

    #[test]
    fn statements_get_signs_texts_and_days() {
        let r = "goandgrow:1";
        let ret = parse_statement(
            &json!({"Type": "Return", "Description": "Go & Grow Zinsen", "Amount": 6.56,
                    "Date": "2026-07-08T00:00:00.303", "Balance": 41123.81}),
            r,
        )
        .unwrap();
        assert_eq!(
            (
                ret.booking_date.as_str(),
                ret.amount_cents,
                ret.purpose.as_str()
            ),
            ("2026-07-08", 656, "Go & Grow Zinsen")
        );
        assert_eq!(ret.counterparty, "Go & Grow");
        let out = parse_statement(
            &json!({"Type": "PartialPayout", "Description": null, "Amount": 1000, "Date": "2026-07-09T00:00:00"}),
            r,
        )
        .unwrap();
        assert_eq!(
            (out.amount_cents, out.purpose.as_str()),
            (-100000, "PartialPayout")
        );
        assert!(parse_statement(&json!({"Type": "Return", "Amount": 1}), r).is_none());
        assert!(parse_statement(&json!({"Type": "Return", "Date": "2026-07-09"}), r).is_none());
        assert!(parse_statement(&json!({"Amount": 1, "Date": "2026-07-09"}), r).is_none());
        assert!(parse_statement(&json!({"Type": "Mystery", "Amount": 1, "Date": "2026-07-09"}), r).is_none());
    }

    #[test]
    fn identical_bookings_of_one_fetch_get_different_ids() {
        let t = json!({"Type": "Return", "Description": "Go & Grow Zinsen", "Amount": 1.0, "Date": "2026-07-08T00:00:00"});
        let mut ids = ContentIds::default();
        let a = parse_statement(&t, "goandgrow:1").unwrap();
        let b = a.clone();
        assert_ne!(ids.next(&a), ids.next(&b));
    }

    #[test]
    fn account_takes_id_number_and_balance() {
        let a = parse_account(
            &json!([{"Id": "a1b2", "ReferenceNumber": "1234567 Erika Mustermann GG7654321",
                                        "Name": "Go & Grow", "ClientValue": 41130.38}]),
        )
        .unwrap();
        assert_eq!(a.external_ref, "goandgrow:a1b2");
        assert_eq!(a.account_number, "1234567");
        assert_eq!(a.balance_cents, 4_113_038);
        assert_eq!(a.kind, AccountKind::Savings);
        assert!(parse_account(&json!([])).is_err());
    }
}
