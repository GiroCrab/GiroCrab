# 0002 – Challenges during a fetch

**Status** · Proposed\
**Date** · 2026-11-23\
**Discussion** · #1730, #1083, #1627, #1690

## Context

[ADR 0001](0001-connector-architecture.md) describes a sync as `begin` → `respond` →
`fetch` → `finish`. A connector can ask for a TAN or an app approval only while logging in.
Once it returns `Phase::Ready`, [`SyncSession::advance`](../../girocrab-core/src/connectors/sync_session.rs)
runs `BankConnector::fetch` as one call to the end.

Some banks ask for strong customer authentication per order instead of at login:

- **flatex (#1083):** the login needs no TAN (`3076`), the depot (`HKWPD`) neither. `HKSAL`
  and `HKKAZ` with `HKTAN` process 4 are answered with `0030` and an `HITAN` challenge to type,
  for every clearing account on every sync. The next order invalidates the pending TAN
  (`3060 Die bereits reservierte TAN wurde entwertet`).
- **comdirect (#1627):** statements further back than 90 days need a TAN (`0030`).

[`FinTsConnector::request`](../../girocrab-core/src/connectors/fints/dialog.rs) turns a `0030`
into an error, and `fetch` reports it as a warning (#1382, #1690). An app approval (3955/3956)
during `fetch` is polled inside `request` for up to three minutes, and the front end shows
nothing. ADR 0001 asks to re-evaluate when an access path fits no `BankConnector` phase.
This is such a path, and other connectors can run into it too.

## Decision drivers

1. Bookings and balances that need a TAN mid-sync can be fetched when the user is there.
2. No regression for the banks that work today, FinTS ones in particular. The FinTS
   `fetch` keeps its structure (one loop over the accounts, errors collected per request).
3. Every front end keeps working without a change: macOS/iOS, Linux and Windows.
4. A sync without the user (`begin_quiet_sync`) never asks and causes no TAN pushes it
   can avoid.
5. Cancelling stops the sync without further orders to the bank. Skipping one TAN keeps
   what was fetched.

## Decision

- **A connector asks through the context.** `ConnectContext` gets
  `prompt: Option<FetchPrompt>`, set by `SyncSession` for interactive syncs and `None`
  for quiet ones. `FetchPrompt::ask(phase)` takes a `Phase::Challenge` or
  `Phase::Pending` and returns `PromptAnswer::Code(SecretString)`, `Poll` or `Skip`.
  After a cancel it returns `CoreError::SessionClosed` and `is_cancelled()` is true.
  One question is open at a time.
- **`SyncSession` drives the fetch step by step.** On `Phase::Ready` the session moves the
  connector and the context into one boxed future (`fetch`, then `finish`) and keeps it in
  its state. `start`, `submit_code` and `poll` drive that future until it ends or asks.
  A question becomes `SyncStep::NeedsChallenge` / `StillWaiting`, exactly like a login
  challenge. An answer is passed on, and driving continues. The session does not use
  `tokio::spawn`: Windows drives it with a `current_thread` runtime through `block_on`,
  and nothing has to run between two calls because the fetch waits for the user.
- **A cancel takes effect at once.** `cancel()` sets the session's cancel flag without waiting.
  Only then does it wait for a running step (a TAN on its way) to release the session state,
  and it ends the bank dialog. Every fetch gets the flag in `ConnectContext::cancelled`, also
  an automatic one without a prompt. The connector checks it before each order and after
  each answer, so it sends nothing more, and the session stores nothing once the fetch ends.
  The boundary is the last check of the flag before an action. A cancel that comes after
  it lets that one action start and run to its end: the order the connector was about to send,
  or the `store_fetch` of a finished fetch. A cancel cannot take back what the bank has
  received, and `store_fetch` writes accounts and bookings in one vault transaction, so a sync
  is either stored or not, never half. Every action after that one is left out.
- **FinTS asks in `request`, the one place that sends orders.**
  - `0030` with an `HITAN` task reference: ask with the challenge from `challenge_from`,
    then send the TAN with `HKTAN` process 2 (the `respond` code path, extracted into one
    helper). `Skip` keeps today's error, so `fetch` reports a warning.
  - 3955/3956: ask with `Phase::Pending`, then send one `HKTAN` process `S` for every `Poll`.
    The internal three-minute loop stays only for syncs without a prompt.
  - A cancelled prompt makes `request` fail before sending. The rest of `fetch` then runs
    through without contacting the bank, and `finish` ends the dialog.
- **Fewer TANs, a separate step in the same direction.** FinTS remembers in its state which
  orders the bank wanted a TAN for (`0030`). For such an `HKSAL` the balance comes from the
  statements' closing balance (camt `CLBD`, MT940 `:62F:`). `HKSAL` is sent only when there
  is no closing balance. A quiet sync does not send these orders at all. The balance is
  reported unknown and the stored one stays (ADR 0001, `balances_unknown`).
- **The UniFFI surface grows by one method.** `SyncSession::skip()` answers the open question
  with `Skip`. Front ends that do not call it yet treat "cancel" as today.

## Not covered

- Transfers and other orders (`TransferSession`). They already have their own challenge flow.
- Portal connectors whose fetch runs in the front end's browser.
- Which TAN method a bank offers. Picking an app approval instead of a typed TAN stays with
  the login (`3920`, `choose_tan_function` in `fints/bpd.rs`).

## Alternatives considered

- **A resumable `fetch`** that returns after every order and continues from a cursor. This
  is explicit and needs no future in the session. But the FinTS `fetch` is one long loop
  over the accounts (balance, depot, card, camt/MT940, standing orders, PDF statements).
  Splitting it into states puts every FinTS bank at risk for one bank's behaviour (driver 2),
  and every new request kind would have to handle resumption.
- **`tokio::spawn` plus channels.** The same contract with a background task. The task needs a
  runtime that keeps running between calls, which the Windows `block_on` model does not
  guarantee. Cancelling would leave a task to clean up.
- **Ask for the TAN in advance.** Not possible: the TAN belongs to the task reference in the
  bank's `HITAN` for one order.
- **Retry with 90 days on `0030`** (#1690 part B). This helps comdirect's long first fetch,
  but not flatex, which wants a TAN for every request.

## Consequences

- A sync can now ask for several TANs after the login. The front ends show them one after
  another. Windows' `DIALOG_TIMEOUT` (10 minutes for the whole dialog) has to allow for that.
- The FinTS dialog stays open while the user types. A bank that times it out turns the
  remaining requests into warnings, not into a lost sync.
- The challenge sheet should say what the TAN is for (balance or bookings, the account as
  personal data, `.sensitive()`). Without it, several prompts in a row look alike.
- Re-evaluate when a connector needs more than one open question at a time, or when a fetch
  has to keep running without the user while it waits.

## Confirmation

- Core tests in `connectors/tests/` and `fints/tests.rs` with a simulated bank: a TAN for the
  balance, for the statements and for two accounts in a row. Also skip, cancel without a
  further order, and a quiet sync without a question.
- The front-end loops over `SyncStep` (`AppModel+Sync.swift`, `GiroCrabWindows/src/sync_flow.rs`,
  `GiroCrabLinux/src/dispatch.rs`) are covered by their existing challenge tests. The Windows
  `sync_flow` fake gets a case with a challenge after a `submit_code`.
- A live test with flatex (#1083) and comdirect over 90 days (#1627) before #1730 is closed.
