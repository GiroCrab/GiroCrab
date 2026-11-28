//! Ayvens Bank (Ayvens Bank N.V., formerly LeasePlan Bank, sparen.ayvensbank.de): balance
//! and bookings of the savings accounts (Tagesgeld) via the JSON API of the online
//! banking's web app (Next.js with Auth.js; unofficial, may change at any time). Answer
//! shapes checked live 2026-11-26.
//!
//!  GET  /api/auth/csrf                    -> csrfToken (and its cookie)
//!  POST /api/auth/callback/credentials    form: username, password, csrfToken, callbackUrl,
//!                                         header `X-Auth-Return-Redirect: 1` -> {url}; an
//!                                         `error=` in it means refused, else the session cookie
//!  GET  /api/auth/session                 -> user.username (the OTP login needs it)
//!  POST /api/savings/send-sms             -> the bank sends the SMS code
//!  POST /api/auth/callback/otp-auth       form: username, otp, trustBrowser=false, csrfToken,
//!                                         callbackUrl; then the session reports user.isOtpVerified.
//!                                         No cookie outlives a sync, so every sync asks for the code
//!  GET  /api/savings/account-current      -> accounts[]: id, productNumber (IBAN), amount (balance)
//!  GET  /api/savings/transactions        -> [] of date ("2026-11-01T00:00:00.000Z"), amount
//!                                            (signed: withdrawals are negative), reference (the
//!                                            reference account's IBAN), description ("Zinsen");
//!                                            all bookings of the login, without an account
//!  GET  /api/savings/account-transactions?accountId=<id>
//!                                         -> the same per account, but empty for a live
//!                                            Tagesgeld account: only used with several accounts
//!  POST /api/savings/logout               -> best effort
//!
//! The bookings' `id` is not unique (all interest bookings share one, deposits carry the
//! reference IBAN), so bookings get content ids. Term deposits (Festgeld) are not read:
//! their answer shape is unknown. Only `https://sparen.ayvensbank.de` receives the password,
//! the SMS code and the session cookie.

use async_trait::async_trait;
use rust_decimal::Decimal;
use serde_json::Value;

use super::{
    BankConnector, Challenge, ChallengeKind, ChallengeResponse, ConnectContext, FetchResult, Phase,
    RemoteAccount, RemoteTransaction,
};
use crate::error::{CoreError, CoreResult};
use crate::i18n::{tr, trf};
use crate::models::AccountKind;
use crate::money::{cents_rounded, loose_decimal};

const BASE: &str = "https://sparen.ayvensbank.de";
/// Counterparty of bookings without a reference account (interest).
const BANK: &str = "Ayvens Bank";

fn unexpected(status: u16) -> CoreError {
    CoreError::protocol(trf(
        "Ayvens Bank: Unerwartete Antwort der Schnittstelle (HTTP {0}).",
        &[&status],
    ))
}

fn expired() -> CoreError {
    CoreError::Authentication {
        message: tr("Ayvens Bank: Sitzung abgelaufen. Bitte erneut abrufen.").into(),
    }
}

fn refused() -> CoreError {
    CoreError::Authentication {
        message: tr(
            "Ayvens Bank: Anmeldung fehlgeschlagen. Bitte Benutzername und Passwort prüfen.",
        )
        .into(),
    }
}

fn sms_challenge() -> Challenge {
    Challenge {
        kind: ChallengeKind::Code,
        title: "SMS-Code".into(),
        message: tr(
            "Bitte den SMS-Code eingeben, den die Ayvens Bank an die hinterlegte Mobilnummer gesendet hat.",
        )
        .into(),
        method_name: "SMS".into(),
        redirect_url: None,
        image: None,
        expires_in_seconds: None,
        poll_interval_seconds: 0,
    }
}

// ------------------------------------------------------------------ parsing

/// `true` when the `url` of an Auth.js callback answer reports a refusal.
fn callback_refused(v: &Value) -> bool {
    v["url"].as_str().is_none_or(|u| u.contains("error="))
}

/// A JSON amount (number or decimal string) in cents; `None` when missing or no number.
fn cents(v: &Value) -> Option<i64> {
    let d: Decimal = match v {
        Value::Number(n) => loose_decimal(&n.to_string())?,
        Value::String(s) if !s.trim().is_empty() => loose_decimal(s)?,
        _ => return None,
    };
    cents_rounded(d)
}

/// One account of `/api/savings/account-current`; `None` without id or balance.
fn parse_account(a: &Value) -> Option<(String, RemoteAccount)> {
    let id = match &a["id"] {
        Value::String(s) if !s.trim().is_empty() => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    let balance_cents = cents(&a["amount"])?;
    let product = a["productNumber"].as_str().unwrap_or_default().trim();
    let is_iban = product.len() >= 15
        && product
            .get(..2)
            .is_some_and(|c| c.bytes().all(|b| b.is_ascii_uppercase()));
    Some((
        id.clone(),
        RemoteAccount {
            external_ref: format!("ayvens:{id}"),
            name: "Ayvens Tagesgeld".into(),
            owner: String::new(),
            iban: if is_iban {
                product.to_string()
            } else {
                String::new()
            },
            account_number: if is_iban {
                String::new()
            } else {
                product.to_string()
            },
            currency: "EUR".into(),
            kind: AccountKind::Savings,
            balance_cents,
            balance_date: super::today(),
        },
    ))
}

/// One booking of a booking list (`/api/savings/transactions`, or `account-transactions` with
/// several accounts); `None` without a usable date or amount.
fn parse_booking(account_ref: &str, t: &Value) -> Option<RemoteTransaction> {
    let date = t["date"]
        .as_str()?
        .trim()
        .get(..10)
        .filter(|d| crate::dates::parse_iso(d).is_some())?;
    let amount_cents = cents(&t["amount"])?;
    let reference = t["reference"].as_str().unwrap_or_default().trim();
    let description = t["description"].as_str().unwrap_or_default().trim();
    let booking_text = if !description.is_empty() {
        description.to_string()
    } else if amount_cents < 0 {
        tr("Auszahlung").into()
    } else {
        tr("Einzahlung").into()
    };
    Some(RemoteTransaction {
        account_ref: account_ref.into(),
        // no unique bank id: the vault derives a content id
        external_id: String::new(),
        booking_date: date.into(),
        value_date: date.into(),
        amount_cents,
        currency: "EUR".into(),
        counterparty: if reference.is_empty() {
            BANK.into()
        } else {
            String::new()
        },
        counterparty_iban: reference.into(),
        // part of the content id: never a translated text, so a booking keeps its id
        // when the app language changes (the bank's own texts are German too)
        purpose: if !description.is_empty() {
            description.into()
        } else if amount_cents < 0 {
            "Auszahlung".into()
        } else {
            "Einzahlung".into()
        },
        booking_text,
        ..Default::default()
    })
}

// ------------------------------------------------------------------ connector

pub struct AyvensConnector {
    base: String,
    client: Option<reqwest::Client>,
    csrf: String,
    /// The bank's user name from the session (the OTP login needs it).
    username: String,
}

impl Default for AyvensConnector {
    fn default() -> Self {
        Self {
            base: BASE.into(),
            client: None,
            csrf: String::new(),
            username: String::new(),
        }
    }
}

impl AyvensConnector {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    fn with_base(base: String) -> Self {
        Self {
            base,
            ..Self::default()
        }
    }

    fn client(&self) -> CoreResult<&reqwest::Client> {
        self.client.as_ref().ok_or(CoreError::SessionClosed)
    }

    async fn get(&self, path: &str) -> CoreResult<Value> {
        let resp = self
            .client()?
            .get(format!("{}{path}", self.base))
            .header("Accept", "application/json")
            .send()
            .await?;
        let status = resp.status();
        let v = super::read_json_or_null(resp).await;
        if matches!(status.as_u16(), 401 | 403) {
            return Err(expired());
        }
        if !status.is_success() {
            return Err(unexpected(status.as_u16()));
        }
        Ok(v)
    }

    /// POST of an Auth.js callback form; `Ok(false)` when the bank refused it.
    async fn callback(&self, provider: &str, fields: &[(&str, &str)]) -> CoreResult<bool> {
        let callback_url = format!("{}/", self.base);
        let mut form: Vec<(&str, &str)> = fields.to_vec();
        form.push(("csrfToken", &self.csrf));
        form.push(("callbackUrl", &callback_url));
        let resp = self
            .client()?
            .post(format!("{}/api/auth/callback/{provider}", self.base))
            .header("Accept", "application/json")
            .header("X-Auth-Return-Redirect", "1")
            .form(&form)
            .send()
            .await?;
        let status = resp.status();
        let v = super::read_json_or_null(resp).await;
        if status.is_server_error() || status.as_u16() == 429 {
            return Err(CoreError::Network {
                message: trf(
                    "Ayvens Bank: Der Anmeldedienst antwortet nicht (HTTP {0}). Bitte später erneut versuchen.",
                    &[&status.as_u16()],
                ),
            });
        }
        Ok(!callback_refused(&v))
    }

    /// The bookings in the list at `path` from `since` on, and how many the list held. An
    /// answer that is no list or holds an incomplete booking fails as a whole.
    async fn bookings(
        &self,
        path: &str,
        account_ref: &str,
        since: &str,
    ) -> CoreResult<(Vec<RemoteTransaction>, usize)> {
        let v = self.get(path).await?;
        let items = v.as_array().ok_or_else(|| unexpected(200))?;
        let mut out = Vec::new();
        for t in items {
            let b = parse_booking(account_ref, t).ok_or_else(|| {
                CoreError::invalid(tr(
                    "Ayvens Bank: Unvollständige Buchung in der Antwort. Es wurden keine Daten übernommen.",
                ))
            })?;
            if b.booking_date.as_str() >= since {
                out.push(b);
            }
        }
        Ok((out, items.len()))
    }

    /// `user` of the Auth.js session; `None` while not logged in.
    async fn session_user(&self) -> CoreResult<Option<Value>> {
        let v = self.get("/api/auth/session").await?;
        Ok(v.get("user").filter(|u| u.is_object()).cloned())
    }
}

#[async_trait]
impl BankConnector for AyvensConnector {
    async fn begin(&mut self, ctx: &ConnectContext) -> CoreResult<Phase> {
        let login = ctx.login_name.trim();
        if login.is_empty() || ctx.secret.expose().is_empty() {
            return Err(CoreError::Authentication {
                message: tr("Ayvens Bank: Benutzername und Passwort erforderlich.").into(),
            });
        }
        self.client = Some(
            super::client_builder(std::time::Duration::from_secs(60))
                .user_agent(super::BROWSER_UA)
                .cookie_store(true)
                .redirect(super::same_host_redirects())
                .build()?,
        );
        self.csrf = self.get("/api/auth/csrf").await?["csrfToken"]
            .as_str()
            .filter(|t| !t.is_empty())
            .ok_or_else(|| unexpected(200))?
            .to_string();
        if !self
            .callback(
                "credentials",
                &[("username", login), ("password", ctx.secret.expose())],
            )
            .await?
        {
            return Err(refused());
        }
        let user = self.session_user().await?.ok_or_else(refused)?;
        self.username = user["username"]
            .as_str()
            .filter(|u| !u.is_empty())
            .unwrap_or(login)
            .to_string();
        let resp = self
            .client()?
            .post(format!("{}/api/savings/send-sms", self.base))
            .header("Accept", "application/json")
            .json(&serde_json::json!({}))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(CoreError::protocol(trf(
                "Ayvens Bank: Der SMS-Code konnte nicht angefordert werden (HTTP {0}).",
                &[&resp.status().as_u16()],
            )));
        }
        Ok(Phase::Challenge(sms_challenge()))
    }

    async fn respond(
        &mut self,
        _ctx: &ConnectContext,
        response: ChallengeResponse,
    ) -> CoreResult<Phase> {
        let ChallengeResponse::Code(code) = response else {
            return Ok(Phase::Challenge(sms_challenge()));
        };
        let code = code.expose().trim().to_string();
        let username = self.username.clone();
        let accepted = self
            .callback(
                "otp-auth",
                &[
                    ("username", &username),
                    ("otp", &code),
                    ("trustBrowser", "false"),
                ],
            )
            .await?;
        let verified = accepted
            && self
                .session_user()
                .await?
                .is_some_and(|u| u["isOtpVerified"].as_bool() == Some(true));
        if !verified {
            return Err(CoreError::Authentication {
                message: tr(
                    "Ayvens Bank: Der SMS-Code wurde abgelehnt. Bitte den Abruf erneut starten.",
                )
                .into(),
            });
        }
        Ok(Phase::Ready)
    }

    async fn fetch(&mut self, ctx: &ConnectContext) -> CoreResult<FetchResult> {
        let current = self.get("/api/savings/account-current").await?;
        let list = current["accounts"]
            .as_array()
            .ok_or_else(|| unexpected(200))?;
        let several = list.len() > 1;
        let mut out = FetchResult::default();
        for a in list {
            let (id, account) = parse_account(a).ok_or_else(|| unexpected(200))?;
            let account_ref = account.external_ref.clone();
            let since = ctx.since_for(&account_ref).to_string();
            // the per-account list stayed empty for a live Tagesgeld account while
            // `/transactions` held its bookings; that list names no account, so it
            // serves a login with one account only
            let path = if several {
                format!("/api/savings/account-transactions?accountId={id}")
            } else {
                "/api/savings/transactions".to_string()
            };
            // one account's failed list must not cost the others theirs; the balance
            // stays. No promise that a later sync catches up: after the full fetch, or for
            // an account without stored bookings, its start date lies later; the period
            // setting of the login (macOS, iOS and Linux) asks for it again
            match self.bookings(&path, &account_ref, &since).await {
                Ok((bookings, 0)) if several => {
                    debug_assert!(bookings.is_empty());
                    out.warnings.push(trf(
                        "Ayvens Bank: Für Konto {0} kamen keine Umsätze. Bei mehreren Konten ist der Abruf noch nicht geprüft.",
                        &[&account_ref],
                    ));
                }
                Ok((bookings, _)) => out.transactions.extend(bookings),
                Err(e @ CoreError::Authentication { .. }) => return Err(e),
                Err(e) => out.warnings.push(trf(
                    "Ayvens Bank: Die Umsätze von Konto {0} konnten nicht geladen werden ({1}). Sie lassen sich in den Einstellungen des Zugangs unter „Umsätze“ › „Beim nächsten Abruf“ erneut abrufen.",
                    &[&account_ref, &e],
                )),
            }
            out.balances_reported.push(account_ref);
            out.accounts.push(account);
        }
        out.accounts_complete = true;
        Ok(out)
    }

    async fn finish(&mut self, _ctx: &ConnectContext) {
        if let Some(client) = self.client.take() {
            let _ = client
                .post(format!("{}/api/savings/logout", self.base))
                .header("Accept", "application/json")
                .send()
                .await;
        }
        self.csrf.clear();
        self.username.clear();
    }
}

#[cfg(test)]
mod tests;
