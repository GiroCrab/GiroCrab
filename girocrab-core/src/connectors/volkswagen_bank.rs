//! Volkswagen Bank online banking (`banking.volkswagenbank.de`), photoTAN and
//! pushTAN. No PSD2 access is wired in.
//!
//! **Status: scaffold.** The sign-in dialogue (two-phase `begin`/`respond`,
//! photoTAN image challenge, pushTAN polling) and the connector plumbing are
//! done and unit-tested against a scripted portal. The portal itself is a
//! single-page app whose endpoints have not been analysed yet, so
//! [`HttpPortal`] – the only place that talks to the bank – still returns a
//! clear "not finished" error. Fill in its four methods from a browser
//! network capture of a real sign-in (see `docs/volkswagen-bank-connector.md`);
//! nothing else in this file needs to change.
//!
//! Rules that carry over from the other portal connectors: money is `i64`
//! cents, debits are negative, secrets are never logged, and bookings without
//! a stable bank id leave `external_id` empty so `store_fetch` derives a
//! content id.
use async_trait::async_trait;

use super::{
    BankConnector, Challenge, ChallengeKind, ChallengeResponse, ConnectContext, FetchResult, Phase,
};
use crate::{
    error::{CoreError, CoreResult},
    i18n::{tr, trf},
};

const BASE: &str = "https://banking.volkswagenbank.de";

/// What the portal answered to a sign-in step.
#[derive(Debug)]
pub(crate) enum LoginStep {
    /// Signed in (client recognised, or the TAN was accepted).
    Authenticated,
    /// The portal wants a photoTAN: scan the image with the photoTAN app and
    /// type the code the app shows.
    PhotoTan { image: Vec<u8> },
    /// The portal sent a pushTAN: approve it in the banking app.
    PushTan,
    /// pushTAN not approved yet.
    PushPending,
}

/// The bank side of the connector. Everything that needs the portal's
/// endpoints lives behind this trait so the dialogue logic stays testable.
#[async_trait]
pub(crate) trait Portal: Send {
    /// User name + password → first sign-in step. `state` is the connection
    /// state saved by the last sync (e.g. a "known device" cookie).
    async fn start_login(
        &mut self,
        user: &str,
        password: &str,
        state: &serde_json::Value,
    ) -> CoreResult<LoginStep>;
    /// Ask whether the pushTAN was approved.
    async fn poll_push(&mut self) -> CoreResult<LoginStep>;
    /// Send the code typed from the photoTAN app.
    async fn submit_photo_tan(&mut self, tan: &str) -> CoreResult<LoginStep>;
    /// Accounts and bookings since `ctx.since` / `ctx.since_by_account`.
    async fn load(&mut self, ctx: &ConnectContext) -> CoreResult<FetchResult>;
    /// Sign out. Best effort.
    async fn logout(&mut self) {}
}

fn not_finished() -> CoreError {
    CoreError::Protocol {
        message: trf(
            "Volkswagen Bank: Die Anbindung an das Online-Banking ({0}) ist noch nicht fertiggestellt. Es wurden keine Daten abgerufen.",
            &[BASE],
        ),
    }
}

/// Talks to `banking.volkswagenbank.de`. Not implemented yet: every call
/// fails with [`not_finished`] before anything is sent.
#[derive(Default)]
struct HttpPortal;

#[async_trait]
impl Portal for HttpPortal {
    // TODO(portal): open the sign-in page, collect the cookies/tokens the SPA
    // sends, post user name + password, map the answer to a `LoginStep`
    // (photoTAN image → `PhotoTan`, pushTAN → `PushTan`, no second factor →
    // `Authenticated`). Restore a "known device" cookie from `state` if the
    // portal offers one, and surface a captcha as `CoreError::Authentication`.
    async fn start_login(
        &mut self,
        _user: &str,
        _password: &str,
        _state: &serde_json::Value,
    ) -> CoreResult<LoginStep> {
        Err(not_finished())
    }

    // TODO(portal): query the pending pushTAN; map "waiting" to `PushPending`
    // and "approved" to `Authenticated`; a rejection or timeout is a
    // `CoreError::Authentication`.
    async fn poll_push(&mut self) -> CoreResult<LoginStep> {
        Err(not_finished())
    }

    // TODO(portal): send the photoTAN code; a wrong code is a
    // `CoreError::Authentication`, never a retry loop.
    async fn submit_photo_tan(&mut self, _tan: &str) -> CoreResult<LoginStep> {
        Err(not_finished())
    }

    // TODO(portal): list the accounts (kind, IBAN, balance) and page through
    // the bookings. Debit = negative cents; use the booking date as
    // `booking_date`; leave `external_id` empty if the portal's ids are not
    // stable across sessions. Return the state to keep in `FetchResult::state`.
    async fn load(&mut self, _ctx: &ConnectContext) -> CoreResult<FetchResult> {
        Err(not_finished())
    }
}

fn photo_tan_challenge(image: Vec<u8>) -> Challenge {
    Challenge {
        kind: ChallengeKind::ImageCode,
        title: "Volkswagen Bank photoTAN".into(),
        message: tr(
            "Bitte die Grafik mit der photoTAN-App scannen und den angezeigten Code eingeben.",
        )
        .into(),
        method_name: "photoTAN".into(),
        redirect_url: None,
        image: Some(image),
        expires_in_seconds: None,
        poll_interval_seconds: 0,
    }
}

fn push_tan_challenge() -> Challenge {
    Challenge {
        kind: ChallengeKind::DecoupledPush,
        title: "Volkswagen Bank pushTAN".into(),
        message: tr("Bitte den Auftrag in der Volkswagen-Bank-App per pushTAN freigeben.").into(),
        method_name: "pushTAN".into(),
        redirect_url: None,
        image: None,
        expires_in_seconds: None,
        poll_interval_seconds: 3,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Idle,
    AwaitingPhotoTan,
    AwaitingPush,
    Ready,
}

pub struct VolkswagenBankConnector {
    portal: Box<dyn Portal>,
    stage: Stage,
}

impl VolkswagenBankConnector {
    pub fn new() -> Self {
        Self::with_portal(Box::<HttpPortal>::default())
    }

    fn with_portal(portal: Box<dyn Portal>) -> Self {
        Self {
            portal,
            stage: Stage::Idle,
        }
    }

    /// Moves the dialogue forward and tells the app what to show next.
    fn advance(&mut self, step: LoginStep) -> Phase {
        match step {
            LoginStep::Authenticated => {
                self.stage = Stage::Ready;
                Phase::Ready
            }
            LoginStep::PhotoTan { image } => {
                self.stage = Stage::AwaitingPhotoTan;
                Phase::Challenge(photo_tan_challenge(image))
            }
            LoginStep::PushTan => {
                self.stage = Stage::AwaitingPush;
                Phase::Challenge(push_tan_challenge())
            }
            LoginStep::PushPending => {
                self.stage = Stage::AwaitingPush;
                Phase::Pending(push_tan_challenge())
            }
        }
    }
}

impl Default for VolkswagenBankConnector {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BankConnector for VolkswagenBankConnector {
    async fn begin(&mut self, ctx: &ConnectContext) -> CoreResult<Phase> {
        if ctx.login_name.trim().is_empty() || ctx.secret.expose().is_empty() {
            return Err(CoreError::Authentication {
                message: tr("Benutzername und Passwort erforderlich").into(),
            });
        }
        self.stage = Stage::Idle;
        let step = self
            .portal
            .start_login(ctx.login_name.trim(), ctx.secret.expose(), &ctx.state)
            .await?;
        Ok(self.advance(step))
    }

    async fn respond(
        &mut self,
        _ctx: &ConnectContext,
        response: ChallengeResponse,
    ) -> CoreResult<Phase> {
        let step = match (self.stage, response) {
            (Stage::AwaitingPush, ChallengeResponse::Poll) => self.portal.poll_push().await?,
            (Stage::AwaitingPhotoTan, ChallengeResponse::Code(tan)) => {
                let tan = tan.expose().trim().to_string();
                if tan.is_empty() {
                    return Err(CoreError::InvalidInput {
                        message: tr("Bitte die photoTAN eingeben.").into(),
                    });
                }
                self.portal.submit_photo_tan(&tan).await?
            }
            _ => {
                return Err(CoreError::InvalidInput {
                    message: tr("Volkswagen Bank: Unerwartete Antwort auf die TAN-Abfrage.").into(),
                })
            }
        };
        Ok(self.advance(step))
    }

    async fn fetch(&mut self, ctx: &ConnectContext) -> CoreResult<FetchResult> {
        if self.stage != Stage::Ready {
            return Err(CoreError::SessionClosed);
        }
        self.portal.load(ctx).await
    }

    async fn finish(&mut self, _ctx: &ConnectContext) {
        self.portal.logout().await;
        self.stage = Stage::Idle;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;
    use crate::secret::SecretString;

    /// Answers each call with the next scripted step.
    struct Scripted {
        steps: VecDeque<LoginStep>,
        loaded: bool,
        logged_out: bool,
    }

    impl Scripted {
        fn new(steps: Vec<LoginStep>) -> Self {
            Self {
                steps: steps.into(),
                loaded: false,
                logged_out: false,
            }
        }
        fn next(&mut self) -> CoreResult<LoginStep> {
            self.steps.pop_front().ok_or(CoreError::SessionClosed)
        }
    }

    #[async_trait]
    impl Portal for Scripted {
        async fn start_login(
            &mut self,
            _user: &str,
            _password: &str,
            _state: &serde_json::Value,
        ) -> CoreResult<LoginStep> {
            self.next()
        }
        async fn poll_push(&mut self) -> CoreResult<LoginStep> {
            self.next()
        }
        async fn submit_photo_tan(&mut self, tan: &str) -> CoreResult<LoginStep> {
            if tan == "000000" {
                return Err(CoreError::Authentication {
                    message: "wrong tan".into(),
                });
            }
            self.next()
        }
        async fn load(&mut self, _ctx: &ConnectContext) -> CoreResult<FetchResult> {
            self.loaded = true;
            Ok(FetchResult::default())
        }
        async fn logout(&mut self) {
            self.logged_out = true;
        }
    }

    fn ctx(user: &str, password: &str) -> ConnectContext {
        ConnectContext {
            connection_id: 1,
            login_name: user.into(),
            secret: SecretString::new(password.into()),
            settings: Default::default(),
            state: serde_json::Value::Null,
            since: "2026-01-01".into(),
            since_by_account: Default::default(),
        }
    }

    fn connector(steps: Vec<LoginStep>) -> VolkswagenBankConnector {
        VolkswagenBankConnector::with_portal(Box::new(Scripted::new(steps)))
    }

    #[tokio::test]
    async fn photo_tan_flow_shows_the_image_then_accepts_the_code() {
        let mut c = connector(vec![
            LoginStep::PhotoTan {
                image: vec![1, 2, 3],
            },
            LoginStep::Authenticated,
        ]);
        let ctx = ctx("demo", "geheim");
        let Phase::Challenge(ch) = c.begin(&ctx).await.unwrap() else {
            panic!("expected a challenge");
        };
        assert_eq!(ch.kind, ChallengeKind::ImageCode);
        assert_eq!(ch.image, Some(vec![1, 2, 3]));
        assert_eq!(ch.method_name, "photoTAN");
        let done = c
            .respond(
                &ctx,
                ChallengeResponse::Code(SecretString::new(" 123456 ".into())),
            )
            .await
            .unwrap();
        assert!(matches!(done, Phase::Ready));
        assert!(c.fetch(&ctx).await.is_ok());
    }

    #[tokio::test]
    async fn push_tan_flow_polls_until_approved() {
        let mut c = connector(vec![
            LoginStep::PushTan,
            LoginStep::PushPending,
            LoginStep::Authenticated,
        ]);
        let ctx = ctx("demo", "geheim");
        let Phase::Challenge(ch) = c.begin(&ctx).await.unwrap() else {
            panic!("expected a challenge");
        };
        assert_eq!(ch.kind, ChallengeKind::DecoupledPush);
        assert!(ch.image.is_none() && ch.poll_interval_seconds > 0);
        assert!(matches!(
            c.respond(&ctx, ChallengeResponse::Poll).await.unwrap(),
            Phase::Pending(_)
        ));
        assert!(matches!(
            c.respond(&ctx, ChallengeResponse::Poll).await.unwrap(),
            Phase::Ready
        ));
    }

    #[tokio::test]
    async fn trusted_device_skips_the_second_factor() {
        let mut c = connector(vec![LoginStep::Authenticated]);
        let ctx = ctx("demo", "geheim");
        assert!(matches!(c.begin(&ctx).await.unwrap(), Phase::Ready));
        assert!(c.fetch(&ctx).await.is_ok());
    }

    #[tokio::test]
    async fn rejects_missing_credentials_and_wrong_answers() {
        let mut c = connector(vec![]);
        for (u, p) in [("", "x"), ("  ", "x"), ("demo", "")] {
            assert!(matches!(
                c.begin(&ctx(u, p)).await,
                Err(CoreError::Authentication { .. })
            ));
        }
        let ctx = ctx("demo", "geheim");
        // answering before a challenge exists
        assert!(matches!(
            c.respond(&ctx, ChallengeResponse::Poll).await,
            Err(CoreError::InvalidInput { .. })
        ));
        // a photoTAN challenge cannot be polled; an empty or wrong code fails
        let mut c = connector(vec![LoginStep::PhotoTan { image: vec![0] }]);
        c.begin(&ctx).await.unwrap();
        assert!(matches!(
            c.respond(&ctx, ChallengeResponse::Poll).await,
            Err(CoreError::InvalidInput { .. })
        ));
        assert!(matches!(
            c.respond(
                &ctx,
                ChallengeResponse::Code(SecretString::new("  ".into()))
            )
            .await,
            Err(CoreError::InvalidInput { .. })
        ));
        assert!(matches!(
            c.respond(
                &ctx,
                ChallengeResponse::Code(SecretString::new("000000".into()))
            )
            .await,
            Err(CoreError::Authentication { .. })
        ));
    }

    #[tokio::test]
    async fn fetch_needs_a_finished_login_and_finish_signs_out() {
        let mut c = connector(vec![LoginStep::PushTan]);
        let ctx = ctx("demo", "geheim");
        assert!(matches!(c.fetch(&ctx).await, Err(CoreError::SessionClosed)));
        c.begin(&ctx).await.unwrap();
        assert!(matches!(c.fetch(&ctx).await, Err(CoreError::SessionClosed)));
        c.finish(&ctx).await;
        assert_eq!(c.stage, Stage::Idle);
    }

    #[tokio::test]
    async fn the_real_portal_is_honest_about_not_being_finished() {
        let mut c = VolkswagenBankConnector::new();
        let err = c.begin(&ctx("demo", "geheim")).await.err().unwrap();
        let CoreError::Protocol { message } = err else {
            panic!("expected a protocol error");
        };
        assert!(message.contains("Volkswagen Bank") && message.contains(BASE));
    }
}
