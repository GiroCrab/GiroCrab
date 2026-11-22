# 0001 – Bank connector architecture

**Status** · Proposed\
**Date** · 2026-11-21\
**Discussion** · [#1402](https://github.com/GiroCrab/GiroCrab/issues/1402)

## Context

Banks differ in login, fetch windows and the data they deliver. GiroCrab has to
absorb these differences and still keep accounts and bookings consistent across
repeated fetches and front ends. New connectors are usually modelled on an
existing one, which carries bank- and platform-specific choices that are easily
copied as a general rule, by people and by AI agents alike.

This ADR documents the existing state. Its rules come from past bugs and the
connector audits, e.g. a failed fetch reporting a balance of 0 (#369), a generic
cleanup rule deleting open card holds (#1111) and the audits of Trade Republic
(#1079) and EquatePlus (#1065).

## Decision drivers

1. Keep the default data path between the user's device and the bank.
   For PSD2, use the user's own application with the provider.
2. Repeated fetches never create duplicates or lose real bookings.
3. A partial or failed fetch never overwrites known data with guesses.
4. Send credentials and session secrets only to the intended bank or
   authorised provider endpoints. Keep them out of logs and fixtures.
5. Each front end uses the same core rules for supported access paths. A
   single login failure must allow the remaining fetches to continue.

## Decision

- **Start from the template.** A new connector begins with
  [`connectors/template.rs`](../../girocrab-core/src/connectors/template.rs) and the steps in
  [CONTRIBUTING.md](../../CONTRIBUTING.md). Existing connectors are examples of a protocol
  (FinTS, PSD2, web API), not templates.
- **The connector owns the bank protocol:** endpoints, login phases, fetch windows and the
  mapping onto the shared types in [`connectors/types.rs`](../../girocrab-core/src/connectors/types.rs)
  (`BankConnector`: `begin` → `respond` → `fetch` → `finish`). Amounts are `i64` cents in
  their own currency. Direct HTTP requests that carry credentials use the
  [shared HTTP clients](../../girocrab-core/src/connectors/mod.rs) with same-origin
  redirects, or no redirects for FinTS. PSD2 can use Enable Banking under the
  user's own application. Interactive portal logins run in the front end, where
  the browser may hold the session. If a connector needs to pass session data
  to the core, scope it to that connection and store it in the encrypted vault.
- **The core owns storage and identity.** Normalised account and booking snapshots use
  [`store_fetch`](../../girocrab-core/src/connectors/persist.rs), which handles account
  selection, text repair, date normalisation, balances, pending entries and documents.
  A connector delivers stable account and booking ids; without a bank id it leaves
  `external_id` empty and the core derives a content id that counts identical bookings
  (`ContentIds`). A balance that was not read is reported as unknown, never as `0`.
  Bank-specific steps in `store_fetch` (Advanzia, EquatePlus) are justified exceptions.
- **The caller handles failures.** The connector signals authentication and
  approval through the supported phases or reports a failure. The front end
  handles the prompt and the queue. On macOS, “Sync all” excludes browser-only
  access paths. If a queued connection needs a login or fails, the batch can
  wait for the user. Skipping that connection continues with the next one.
  [macOS sync flow](../../GiroCrabApp/Sources/GiroCrabUI/AppModel+Sync.swift)
- **Done means end to end:** core, catalog ([`catalog.rs`](../../girocrab-core/src/connectors/catalog.rs),
  PSD2 presets in [`bank_directory.rs`](../../girocrab-core/src/connectors/bank_directory.rs)),
  FFI and app setup with tests.

## Not covered

Payment orders (`send_transfer`, standing orders, direct debits) and file importers
(`girocrab-core/src/importers/`). They share parts of this design and get their own ADR
when needed.

## Alternatives considered

- **A hosted XS2A aggregator as the default path** would reduce per-bank work.
  It would also add a third party to the default data path, conflicting with
  driver 1. The [XS2A evaluation](../evaluations/yaxi-xs2a.md) recommends
  keeping the current access paths and leaves a future opt-in integration open.
- **Scripted plugins loaded at runtime** (#251) would reuse existing community scripts,
  but bypass the shared storage path and secret handling (drivers 2–4). Native Rust
  connectors via the bank's own API, FinTS or PSD2 have priority.
- **Storage per connector** is quick for one bank, but every connector would have to secure
  identity, balances and repeatability itself, and a gap would show up for one bank only.
- **One login flow for all banks** would force browser sessions, FinTS approvals and API
  logins into the same shape.

## Consequences

- Changes to a shared storage rule affect the connectors that use it (#369, #1111).
  Explain new exceptions in the PR and cover the affected paths with tests.
- Bank-specific limits (account kinds, fetch windows, platform differences) are written down
  in the connector's doc in `docs/`, not only in code.
- Re-evaluate when an access path fits no `BankConnector` phase or several connectors need
  the same exception in `store_fetch`.

## Confirmation

The [connector lint](../../scripts/lint-connectors.sh) checks for references in shared
Core and UI code. Use synthetic or anonymised fixtures to test each connector's mapping.
The connector audit records live results for repeated fetches, balances and currency,
error isolation in “Sync all” and test coverage.
