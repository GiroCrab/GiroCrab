//! `SyncSession`: the login/fetch state machine exposed via FFI. A connector
//! can ask for a TAN or an app approval during `fetch` too (#1730,
//! [ADR 0002](../../../docs/adr/0002-challenges-during-fetch.md)).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::{Mutex as AsyncMutex, mpsc, oneshot};

use crate::error::{CoreError, CoreResult};
use crate::models::ConnectorKind;
use crate::secret::{SecretJson, SecretString};
use crate::vault::Vault;

use super::*;

/// Answer to a question a connector asked during `fetch` (#1730).
#[derive(Debug)]
pub enum PromptAnswer {
    /// The code the user typed.
    Code(SecretString),
    /// Ask the bank again whether the app approval happened.
    Poll,
    /// The user left this order out: the sync goes on without it.
    Skip,
}

type Question = (Phase, oneshot::Sender<PromptAnswer>);

/// Set when the user cancels the sync (#1730). Every fetch gets it in
/// [`ConnectContext::cancelled`], also an automatic one without a prompt: a
/// connector checks it before each order to the bank.
#[derive(Clone, Default)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    pub fn is_set(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    pub(crate) fn set(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// Lets a connector ask the user during `fetch` (#1730). `SyncSession` puts it
/// into [`ConnectContext::prompt`] for syncs the user started; automatic syncs
/// (`begin_quiet_sync`) get none and must not ask.
#[derive(Clone)]
pub struct FetchPrompt {
    questions: mpsc::UnboundedSender<Question>,
    cancelled: CancelFlag,
}

impl FetchPrompt {
    #[cfg(test)]
    pub(crate) fn channel() -> (Self, mpsc::UnboundedReceiver<Question>) {
        Self::cancelled_by(CancelFlag::default())
    }

    /// A prompt that asks nothing more once `cancelled` is set.
    pub(crate) fn cancelled_by(cancelled: CancelFlag) -> (Self, mpsc::UnboundedReceiver<Question>) {
        let (questions, rx) = mpsc::unbounded_channel();
        (
            FetchPrompt {
                questions,
                cancelled,
            },
            rx,
        )
    }

    /// Shows `question` (`Phase::Challenge` or `Phase::Pending`) and waits for
    /// the answer. `SessionClosed` once the user cancelled the sync.
    pub async fn ask(&self, question: Phase) -> CoreResult<PromptAnswer> {
        if self.cancelled.is_set() {
            return Err(CoreError::SessionClosed);
        }
        let (tx, rx) = oneshot::channel();
        self.questions
            .send((question, tx))
            .map_err(|_| CoreError::SessionClosed)?;
        rx.await.map_err(|_| CoreError::SessionClosed)
    }
}

/// The open bank dialog.
struct Active {
    connector: Box<dyn BankConnector>,
    ctx: ConnectContext,
}

/// `fetch` and `finish`, owning the connector and its context.
type FetchRun = Pin<Box<dyn Future<Output = FetchEnd> + Send>>;

/// How a fetch ended: its result, and for a failed one the connector state worth keeping
/// (`BankConnector::state_after_failed_fetch`, #1828).
type FetchEnd = (CoreResult<FetchResult>, Option<SecretJson>);

/// A fetch in progress. It runs only while a session call drives it, and
/// stops at the connector's questions until the user answers (no task of its
/// own: Windows drives the session with `block_on` on a one-thread runtime).
struct Fetching {
    run: FetchRun,
    questions: mpsc::UnboundedReceiver<Question>,
    /// Where the answer to the question shown goes.
    open: Option<oneshot::Sender<PromptAnswer>>,
    connection_id: i64,
    only_account: Option<String>,
}

enum State {
    /// Logging in: the connector answers challenges itself (`respond`).
    Login(Box<Active>),
    Fetching(Fetching),
}

/// How far a drive of the fetch got.
enum Driven {
    Asked(Question),
    Done(Box<FetchEnd>),
}

/// One sync run for one bank connection. Created by `Vault::begin_sync`.
#[derive(uniffi::Object)]
pub struct SyncSession {
    vault: Arc<Vault>,
    kind: ConnectorKind,
    /// The user started this sync and can answer questions during the fetch.
    interactive: AtomicBool,
    /// Set by `cancel()` before it waits for `state`: a step that holds the
    /// state (a TAN on its way, the rest of the fetch) sees it, sends nothing
    /// more to the bank and stores nothing.
    cancelled: CancelFlag,
    /// The login is done and the fetch runs (a question now is for one order).
    fetching: AtomicBool,
    /// `None` once the sync finished, failed or was cancelled, so every
    /// later step gets `SessionClosed`.
    state: AsyncMutex<Option<State>>,
    /// The pending full fetch (#1043) this sync reaches back for: done once
    /// its result is stored.
    history: Option<String>,
}

impl SyncSession {
    pub(super) fn new(
        vault: Arc<Vault>,
        kind: ConnectorKind,
        connector: Box<dyn BankConnector>,
        ctx: ConnectContext,
        history: Option<String>,
    ) -> Self {
        Self {
            vault,
            kind,
            interactive: AtomicBool::new(true),
            cancelled: CancelFlag::default(),
            fetching: AtomicBool::new(false),
            state: AsyncMutex::new(Some(State::Login(Box::new(Active { connector, ctx })))),
            history,
        }
    }

    /// Automatic sync: nobody answers, so the fetch asks no questions.
    pub(super) fn set_quiet(&self) {
        self.interactive.store(false, Ordering::SeqCst);
    }

    pub(super) fn is_cancelled(&self) -> bool {
        self.cancelled.is_set()
    }

    /// Whether the fetch of this session may ask the user (tests of the
    /// `Vault::begin_*` entry points).
    #[cfg(test)]
    pub(super) fn is_interactive(&self) -> bool {
        self.interactive.load(Ordering::SeqCst)
    }

    /// Copy of the prepared login context (settings, PIN, connector state),
    /// e.g. to reuse the sync set-up for a transfer.
    pub(super) fn clone_context(&self) -> CoreResult<ConnectContext> {
        let st = self
            .state
            .try_lock()
            .map_err(|_| CoreError::SessionClosed)?;
        match st.as_ref() {
            Some(State::Login(a)) => Ok(a.ctx.clone()),
            _ => Err(CoreError::SessionClosed),
        }
    }

    /// Runs one login step and goes on to the fetch once the login is done.
    async fn login_step(
        &self,
        call: impl AsyncFnOnce(&mut Active) -> CoreResult<Phase>,
    ) -> CoreResult<SyncStep> {
        let mut st = self.state.lock().await;
        let Some(State::Login(active)) = st.as_mut() else {
            return Err(CoreError::SessionClosed);
        };
        let phase = call(active).await;
        let close = |st: &mut Option<State>| match st.take() {
            Some(State::Login(a)) => a,
            _ => unreachable!("checked above"),
        };
        // cancelled while the bank answered: no fetch, no challenge
        if self.is_cancelled() {
            let mut a = close(&mut st);
            a.connector.finish(&a.ctx).await;
            return Err(CoreError::SessionClosed);
        }
        match phase {
            Ok(Phase::Challenge(c)) => Ok(SyncStep::NeedsChallenge { challenge: c }),
            Ok(Phase::Pending(c)) => Ok(SyncStep::StillWaiting { challenge: c }),
            Ok(Phase::Ready) => {
                let fetching = self.start_fetch(*close(&mut st));
                *st = Some(State::Fetching(fetching));
                self.drive(&mut st).await
            }
            Err(e) => {
                let mut a = close(&mut st);
                a.connector.finish(&a.ctx).await;
                Err(e)
            }
            Ok(Phase::ConfirmPayee(_)) => {
                let mut a = close(&mut st);
                a.connector.finish(&a.ctx).await;
                Err(CoreError::protocol(
                    "payee confirmation outside of a transfer",
                ))
            }
        }
    }

    fn start_fetch(&self, active: Active) -> Fetching {
        let Active {
            mut connector,
            mut ctx,
        } = active;
        self.fetching.store(true, Ordering::SeqCst);
        let (prompt, questions) = FetchPrompt::cancelled_by(self.cancelled.clone());
        ctx.cancelled = self.cancelled.clone();
        // a quiet fetch gets no prompt: its question channel stays empty and closed
        if self.interactive.load(Ordering::SeqCst) {
            ctx.prompt = Some(prompt);
        }
        let (connection_id, only_account) = (ctx.connection_id, ctx.only_account.clone());
        let kind = self.kind;
        let run: FetchRun = Box::pin(async move {
            crate::depot_debug::log(&format!("sync advance: Ready, calling fetch for {kind:?}"));
            let fetched = connector.fetch(&ctx).await;
            // read before `finish` ends the bank dialog
            let kept = fetched
                .is_err()
                .then(|| connector.state_after_failed_fetch())
                .flatten();
            connector.finish(&ctx).await;
            (fetched, kept)
        });
        Fetching {
            run,
            questions,
            open: None,
            connection_id,
            only_account,
        }
    }

    /// Drives the fetch until it asks the user or ends; stores its result
    /// unless the sync was cancelled meanwhile.
    async fn drive(&self, st: &mut Option<State>) -> CoreResult<SyncStep> {
        loop {
            let Some(State::Fetching(f)) = st.as_mut() else {
                return Err(CoreError::SessionClosed);
            };
            let driven = tokio::select! {
                biased;
                done = &mut f.run => Driven::Done(Box::new(done)),
                Some(q) = f.questions.recv() => Driven::Asked(q),
            };
            match driven {
                // cancelled meanwhile: the question ends with `SessionClosed`
                Driven::Asked(_) if self.is_cancelled() => {}
                Driven::Asked((Phase::Challenge(c), reply)) => {
                    f.open = Some(reply);
                    return Ok(SyncStep::NeedsChallenge { challenge: c });
                }
                Driven::Asked((Phase::Pending(c), reply)) => {
                    f.open = Some(reply);
                    return Ok(SyncStep::StillWaiting { challenge: c });
                }
                // nothing the user can answer during a fetch
                Driven::Asked((_, reply)) => {
                    let _ = reply.send(PromptAnswer::Skip);
                }
                Driven::Done(fetched) => {
                    let Some(State::Fetching(f)) = st.take() else {
                        unreachable!("checked above")
                    };
                    // `finish` already ended the bank dialog
                    if self.is_cancelled() {
                        return Err(CoreError::SessionClosed);
                    }
                    let (fetched, kept_state) = *fetched;
                    let mut data = match fetched {
                        Ok(data) => data,
                        Err(e) => {
                            // the sync failed, what the connector learned about the bank on the
                            // way stays (#1828); best effort, the failure is what gets reported
                            if let Some(state) = kept_state
                                && let Ok(db) = self.vault.db()
                            {
                                let _ = crate::vault::connections::store_state(
                                    &db,
                                    f.connection_id,
                                    &state,
                                );
                            }
                            return Err(e);
                        }
                    };
                    if let Some(only) = &f.only_account {
                        data.keep_only(only);
                    }
                    let summary = store_fetch(&self.vault, self.kind, f.connection_id, data)?;
                    // Done once stored, also with warnings (an account that failed, a list cut
                    // at its page limit): kept, every sync would ask for the whole period again,
                    // and for a TAN, without ever finishing. The warnings say what is missing;
                    // `request_full_history` asks for the period again.
                    if let Some(history) = &self.history {
                        let db = self.vault.db()?;
                        super::history::finish(&db, f.connection_id, history)?;
                    }
                    return Ok(SyncStep::Finished { summary });
                }
            }
        }
    }

    /// Passes `answer` to the question shown and drives the fetch on. Without
    /// an open question (a late poll) it just goes on driving.
    async fn answer(&self, answer: PromptAnswer) -> CoreResult<SyncStep> {
        let mut st = self.state.lock().await;
        let Some(State::Fetching(f)) = st.as_mut() else {
            return Err(CoreError::SessionClosed);
        };
        if let Some(open) = f.open.take() {
            let _ = open.send(answer);
        }
        self.drive(&mut st).await
    }

    async fn fetching(&self) -> bool {
        matches!(*self.state.lock().await, Some(State::Fetching(_)))
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl SyncSession {
    /// Phase 1: log in. Returns a challenge or finishes the sync.
    pub async fn start(&self) -> Result<SyncStep, CoreError> {
        self.login_step(async |s| s.connector.begin(&s.ctx).await)
            .await
    }

    /// Phase 2a: submit the code / redirect URL the user typed, at login or
    /// for an order during the fetch.
    pub async fn submit_code(&self, code: String) -> Result<SyncStep, CoreError> {
        let code = SecretString::new(code);
        if self.fetching().await {
            return self.answer(PromptAnswer::Code(code)).await;
        }
        let code = ChallengeResponse::Code(code);
        self.login_step(async |s| s.connector.respond(&s.ctx, code).await)
            .await
    }

    /// Phase 2b: ask whether the decoupled push approval happened.
    pub async fn poll(&self) -> Result<SyncStep, CoreError> {
        if self.fetching().await {
            return self.answer(PromptAnswer::Poll).await;
        }
        self.login_step(async |s| s.connector.respond(&s.ctx, ChallengeResponse::Poll).await)
            .await
    }

    /// Whether the login is done and the sync is fetching: a challenge from now
    /// on is for one order of the fetch, which [`skip`](Self::skip) can leave out.
    pub fn is_fetching(&self) -> bool {
        self.fetching.load(Ordering::SeqCst)
    }

    /// Leaves out the order a TAN was asked for during the fetch (#1730): it is
    /// reported with the sync, the rest is fetched and stored. A login
    /// challenge cannot be skipped (`InvalidInput`): cancel the sync instead.
    pub async fn skip(&self) -> Result<SyncStep, CoreError> {
        if !self.fetching().await {
            return Err(CoreError::invalid(
                "only a question during the fetch can be skipped",
            ));
        }
        self.answer(PromptAnswer::Skip).await
    }

    /// Cancels the sync and closes the bank dialog. The flag is set before
    /// waiting for a running step (a TAN on its way), so the fetch sends no
    /// further order and stores nothing. An action that already passed its
    /// last check of the flag still runs to its end: the one order about to
    /// be sent, or the storing of a finished fetch (ADR 0002).
    pub async fn cancel(&self) {
        self.cancelled.set();
        match self.state.lock().await.take() {
            Some(State::Login(mut a)) => a.connector.finish(&a.ctx).await,
            Some(State::Fetching(mut f)) => {
                // the open question ends with `SessionClosed`
                drop(f.open.take());
                // let the fetch run out: its requests fail before sending, and
                // `finish` ends the bank dialog
                loop {
                    tokio::select! {
                        biased;
                        _ = &mut f.run => break,
                        Some(q) = f.questions.recv() => drop(q),
                    }
                }
            }
            None => {}
        }
    }

    pub fn bank_name(&self) -> String {
        descriptor(self.kind).name
    }
}
