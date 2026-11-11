# Scalable Capital connector – implementation plan

Living document. Update the **Status** table and the **Change log** whenever work
on the connector progresses. Everything below was derived from the open-source
[`ScalableCapital/scalable-cli`](https://github.com/ScalableCapital/scalable-cli)
(v1.3.0, Apache-2.0, snapshot of 2026-11-10) and from GiroCrab's own connector
layer. Nothing has been verified against the live Scalable backend yet.

## Status

| Phase | Scope | State |
| --- | --- | --- |
| 0 | Feasibility study (this document) | ✅ done 2026-11-11 |
| 1 | DPoP signing + device-code login against the real endpoints | ⬜ not started |
| 2 | Session persistence, refresh, 2FA-on-login | ⬜ not started |
| 3 | Account discovery (broker portfolio, Overnight, Fixed-Term) | ⬜ not started |
| 4 | Transaction fetch + mapping, de-duplication | ⬜ not started |
| 5 | Wiring: `ConnectorKind`, descriptor, Swift UI, demo data | ⬜ not started |
| 6 | Tests, README/CONTRIBUTING update, release notes | ⬜ not started |

Blocking dependency for phases 1–4: a Scalable Capital account with
**Profile → Security → Agentic Investing** enabled, to validate the flow live.

## Goal and scope

Add Scalable Capital (Broker, Overnight savings, Fixed-Term savings) as a native
connector in `girocrab-core`, talking **only** to Scalable's own endpoints, read-only.

In scope:

- Device-code OAuth login with DPoP-bound tokens, as the official CLI does it.
- Accounts: broker portfolio(s) as `AccountKind::Brokerage`, Overnight savings as
  `AccountKind::Savings`, Fixed-Term accounts as `AccountKind::Savings`.
- Transactions: broker cash + security transactions, Overnight cash transactions.
- Balances: portfolio valuation, cash balance, Overnight total, Fixed-Term deposit.

Out of scope (now or ever):

- Trading, savings-plan changes, watchlists, price alerts (write operations).
- Crypto transaction stream (can be added later; separate cursor and types).
- Transaction documents (not exposed by the API).

## What the CLI tells us about the API

### Endpoints and OAuth client

| Item | Value (prod channel, `src/channel.rs`) |
| --- | --- |
| GraphQL | `https://de.scalable.capital/api/cli/graphql` |
| OAuth issuer | `https://secure.scalable.capital` |
| Device code | `{issuer}/oauth/device/code` |
| Token | `{issuer}/oauth/token` |
| Revoke | `{issuer}/oauth/revoke` |
| Audience | `https://de.scalable.capital/api-gateway` |
| Client ID | `yBM3BrpRgwSTJZRdJllvtD6jJEmyxWfE` (public client, no secret) |
| Scope | `offline_access openid email` |
| User-Agent | `scalable-cli/<version>` (CLI sends it; unknown whether the server checks it) |

The issuer is an Auth0-style tenant: standard RFC 8628 device flow with
`authorization_pending`, `slow_down`, `access_denied`, `expired_token` errors.

### Login flow (`src/auth.rs::login_with_device_code`)

1. Generate or load a P-256 signing key (DPoP key).
2. `POST /oauth/device/code` with form `client_id`, `audience`, `scope` and a
   `DPoP` header (proof for `POST` + that URL, no `ath`). Response:
   `device_code`, `user_code`, `verification_uri`, `verification_uri_complete`,
   `expires_in`, `interval` (default 5 s).
3. Show the user `verification_uri_complete` (or `verification_uri` + `user_code`).
4. Poll `POST /oauth/token` every `interval` seconds with
   `grant_type=urn:ietf:params:oauth:grant-type:device_code`, `device_code`,
   `client_id`, plus a fresh `DPoP` proof per request. On `slow_down` add 2 s.
5. Success returns `access_token`, `refresh_token`, `expires_in`, `token_type`
   (`DPoP`). The CLI verifies the JWT against the issuer's JWKS; GiroCrab can
   decode the claims unverified (the token came straight from the issuer over TLS).
6. Read the person ID from the access-token claim
   `https://de.scalable.capital/person_id` (fallback `…/personId`).
7. Post-login 2FA (GraphQL, same token):
   - `query Is2faOnLoginEnabled(input:{userId})` → `{enabled, hasApprovedSession}`.
   - If enabled and not approved: `mutation Start2faOnLogin(input:{userId,
     deviceName, deviceType})` → `mfaSessionId`; then poll
     `mutation Validate2faOnLogin(input:{userId, mfaSessionId})` → `status`
     (`APPROVED` / `DENY` / `TIMEOUT_RETRY`). The user approves in the Scalable app.

### DPoP (`src/dpop/mod.rs`)

Every HTTP request to the issuer **and** to GraphQL carries a `DPoP` header: a JWS
with header `{"typ":"dpop+jwt","alg":"ES256","jwk":{kty:"EC",crv:"P-256",x,y}}`
and claims `htm` (upper-case method), `htu` (URL without query/fragment), `iat`,
`jti` (random), optional `nonce`, and for GraphQL `ath` = base64url(SHA-256(access
token)). GraphQL uses `Authorization: DPoP <access_token>`.

Nonce handling: if the server answers 400/401 with a `DPoP-Nonce` header or a body
containing `use_dpop_nonce` / `invalid_dpop_proof`, retry **once** with that nonce.
The CLI only replays queries this way, never mutations.

Access tokens are bound to the key, so the private key must be persisted together
with the refresh token. The CLI's software backend (plain P-256 key in a file)
proves the server accepts any client-generated key; Secure Enclave is optional.

### Session limits (`public-docs/automation.md`)

- Access token: 20 min, refreshed via `grant_type=refresh_token` (rotating refresh token, DPoP-signed).
- Idle timeout: 24 h without an authenticated request.
- Hard maximum: 7 days from the original login, then a new device-code login.
- Disabling "Agentic Investing" in the web profile kills the session.
- Rate limits exist but are unpublished; errors come back as `rate_limited`.

### GraphQL operations needed

All queries take `$accountId` = person ID. Operation names matter: the backend
distinguishes allowed operations by name (the CLI blocks unnamed mutations).

| Purpose | Operation (file in CLI) | Key fields |
| --- | --- | --- |
| Identity | `WhoAmI(id)` → `personOverview` (`graphql.rs`) | `id`, `personalDetails.firstName/lastName` |
| Portfolio IDs | `ResolveBrokerIds(id)` → `account{id, brokerPortfolios{id}}` (`broker_commands.rs`) | portfolio IDs |
| Valuation | `BrokerOverview(accountId, portfolioId, includeYearToDate)` (`broker_queries.rs`) | `valuation.valuation`, `securitiesValuation`, `cryptoValuation`, `timestampUtc.time` |
| Cash | cash-breakdown query (`broker_queries.rs`, `BrokerLimits`) | `cash_balance`, `buying_power…` |
| Transactions | `BrokerTransactions(accountId, portfolioId, brokerInput, cryptoInput, includeCrypto)` | see below |
| Overnight accounts | `DiscoverOvernightAccounts(accountId)` (`overnight_queries.rs`) | `savingsAccounts{id, owners, personalizations.name, state}` |
| Overnight balance | `OvernightSummary(accountId, savingsAccountId)` | `totalAmount`, `interests.*` |
| Overnight transactions | `OvernightTransactions(accountId, savingsAccountId, input)` | same shape as broker cash transactions |
| Fixed-Term | `ListFixedTermAccounts(accountId)`, `GetFixedTermAccount(...)` (`fixed_term_queries.rs`) | deposit, term, rates, maturity |

`BrokerTransactionInput` (built in `broker_queries.rs` ~line 1400):
`{pageSize, cursor, status: [..], types: [..], searchTerm, fromTime, toTime}` with
times in **epoch seconds**. Response: `moreTransactions{cursor, total, transactions[]}`.

Transaction union members and fields:

- common: `__typename, id, currency, type, status, isCancellation, lastEventDateTime, description, custodian`
- `BrokerSecurityTransactionSummary`: `isin, securityTransactionType, quantity, amount, side, limitPrice, stopPrice`
- `BrokerCashTransactionSummary`: `relatedIsin, cashTransactionType, amount`
- `BrokerNonTradeSecurityTransactionSummary`: `isin, nonTradeSecurityTransactionType, quantity, amount`
- `BrokerEltifTransactionSummary`: `isin, securityTransactionType, eltifQuantity, amount, side`

Monetary values arrive as decimal strings or numbers; never trust floats, parse to
`i64` cents (see `trade_republic.rs::amount_cents` for the rounding helper).

## Design in GiroCrab

### Identity

- `ConnectorKind::ScalableCapital`, key `"scalable"` (`models.rs::as_str/all`).
- `BankDescriptor` in `connectors/mod.rs::descriptor()`: name "Scalable Capital",
  protocol "OAuth 2.0 Device-Flow + GraphQL (offizielle CLI-API)", `login_label`
  = e-mail (display only; the browser does the real login), `secret_label` empty,
  `required_settings: []`, note explaining the "Agentic Investing" switch and the
  7-day re-login.
- New file `girocrab-core/src/connectors/scalable.rs` (connector) and
  `girocrab-core/src/connectors/dpop.rs` (reusable DPoP signer).

### Connector phases

`begin(ctx)`:

1. If `ctx.state` holds a usable session (refresh token + DPoP key, not older than
   7 days): refresh if the access token expires within 60 s → `Phase::Ready`.
2. Otherwise start the device flow, keep `device_code`/`interval`/`expires_at` in
   memory and return `Phase::Challenge` with `ChallengeKind::DecoupledPush`,
   `redirect_url = verification_uri_complete`, `message` containing the user code,
   `poll_interval_seconds = interval`, `expires_in_seconds = expires_in`.

`respond(ctx, _)` (called by the UI poll loop):

1. Poll the token endpoint once. `Pending`/`SlowDown` → `Phase::Pending(challenge)`.
2. On success: decode person ID, run the 2FA-on-login check. If a second factor is
   required, return a second `DecoupledPush` challenge ("Freigabe in der
   Scalable-App") and keep polling `Validate2faOnLogin` on later `respond` calls.
3. → `Phase::Ready`.

`fetch(ctx)`:

1. `ResolveBrokerIds` → portfolio IDs; `DiscoverOvernightAccounts`; `ListFixedTermAccounts`.
2. Per portfolio: `BrokerOverview` + cash breakdown → `RemoteAccount` (Brokerage,
   balance = `valuation`), plus `BrokerTransactions` paged with `fromTime =
   ctx.since` at 00:00 UTC, `pageSize` 100, follow `cursor` until exhausted.
3. Per Overnight account: `OvernightSummary` → `RemoteAccount` (Savings, balance =
   `totalAmount`), `OvernightTransactions` paged the same way.
4. Per Fixed-Term account: `GetFixedTermAccount` → `RemoteAccount` (Savings,
   balance = deposit + accrued interest). No transactions.
5. Return `FetchResult` with `state = Some(updated session JSON)`.

### State JSON (persisted encrypted in the vault)

```json
{
  "dpop_key_hex": "<32-byte P-256 scalar, hex>",
  "access_token": "...",
  "access_expires_at": 1762900000,
  "refresh_token": "...",
  "login_at": 1762300000,
  "person_id": "...",
  "portfolio_ids": ["..."]
}
```

Never log any of these. The access token is kept only to avoid a refresh per sync.

### Mapping rules

| Scalable | GiroCrab `RemoteAccount` / `RemoteTransaction` |
| --- | --- |
| broker portfolio | `external_ref = "sc:portfolio:<id>"`, `kind = Brokerage`, `name = "Scalable Depot"`, `iban = ""`, `currency = "EUR"` |
| Overnight account | `external_ref = "sc:overnight:<id>"`, `kind = Savings`, name from `personalizations.name` or "Scalable Overnight" |
| Fixed-Term account | `external_ref = "sc:fixedterm:<id>"`, `kind = Savings`, name "Scalable Festgeld <term>" |
| transaction `id` | `external_id = "sc:<id>"` (stable → de-dup on re-sync) |
| `lastEventDateTime` | `booking_date` and `value_date` (date part, local to Europe/Berlin) |
| `amount` | `amount_cents`; sign: buys and withdrawals negative, sells, deposits, dividends, interest positive. Verify against live data; the CLI shows `side` for securities and `cashTransactionType` for cash |
| `description` | `purpose` |
| `securityTransactionType` / `cashTransactionType` / `nonTradeSecurityTransactionType` | `booking_text` |
| `isin` + `quantity` | appended to `purpose` ("ISIN …, 1,234 Stk") until GiroCrab has first-class security positions |
| `status` ≠ executed/settled, or `isCancellation` | skip (pending orders are not bookings) |

Open question: whether `amount` already carries the sign. The Trade Republic
connector had the same ambiguity; resolve with live data in phase 4.

### UI changes (Swift, `GiroCrabApp/Sources/GiroCrabUI`)

- `Formatting.swift`: `displayName` "Scalable Capital", `tintHex` `#0E2A47`
  (Scalable navy), `monogram` "SC".
- `ConnectionsUI.swift`: `keys` → `[]`; **not** in `usesPin`.
- `AppModel.swift` sync entry: treat `.scalableCapital` like a connector without a
  PIN that always goes through `begin()` (the connector decides whether a login is
  needed from its state). No web-login sheet.
- `TwoFactorSheet.swift`: the `.decoupledPush` branch must additionally render
  `c.redirectUrl` as an "Im Browser anmelden" button when present. Currently only
  the `.redirect` branch does that. This is the only UI feature gap.
- `BankPicker.swift`: add "Scalable Capital" to `suggestions` and to the dedicated-
  connector add form (no login input needed; the e-mail field is optional).
- `generated/` bindings: `ConnectorKind` is a UniFFI enum, so the xcframework must
  be rebuilt once (`./build-xcframework.sh`).

### Dependencies (`girocrab-core/Cargo.toml`)

- `p256 = { version = "0.13", features = ["ecdsa"] }` for ES256 signing and the
  public JWK. (`jsonwebtoken` is already present but only configured for RSA/HMAC;
  building the DPoP JWS by hand with `p256` + `base64` is ~60 lines and avoids
  pulling a second backend.)
- `rand` and `sha2` are already present. `sha2` is 0.10 here vs 0.11 in the CLI; the
  API used (`Sha256::digest`) is identical.

### Demo mode

Add a `DemoAccountSpec` for `ConnectorKind::ScalableCapital` in `demo.rs`
(one brokerage "Scalable Depot", one savings "Scalable Overnight") and make
`make_connector_for(kind, key, demo = true)` return the demo stub, as for Trade Republic.

## Implementation plan

Each step ends with `cargo check -q -p girocrab-core` (Rust) or `swift build -q`
(Swift only). Commit granularity: one commit per step.

### Phase 1 – DPoP + device-code login (Rust only, ~250 lines)

1. `connectors/dpop.rs`: `DpopKey::generate()`, `from_hex()`, `to_hex()`,
   `public_jwk()`, `proof(method, url, nonce, access_token) -> String`. Unit tests:
   header/claims shape, `ath` value for a known token, `htu` strips query.
2. `connectors/scalable.rs`: constants (endpoints, client ID, audience, scope),
   `oauth_post(form, key, nonce_retry)` helper with the single nonce retry,
   `request_device_code()`, `poll_device_token()`, `refresh_token()`.
3. `begin()` returning the `DecoupledPush` challenge; `respond()` polling.
4. Manual live test with a real account behind a feature flag / `cargo test --
   --ignored scalable_live` reading credentials from env. Record the exact
   responses (sanitised) in this document under **Live findings**.

### Phase 2 – Session, refresh, 2FA-on-login (~120 lines)

5. State struct ⇄ `ctx.state` JSON, 7-day and 24-hour expiry checks, refresh
   before every `fetch`, `FetchResult.state` round-trip.
6. 2FA-on-login: `Is2faOnLoginEnabled` → `Start2faOnLogin` → poll `Validate2faOnLogin`
   as a second `DecoupledPush` challenge. Use `deviceName = "GiroCrab"`, `deviceType = "CLI"`
   (the backend may only accept the CLI's values; verify live).
7. Error mapping: `access_denied`/`expired_token` → `CoreError::Authentication`;
   `rate_limited` → retryable error with the server's retry delay in the message.

### Phase 3 – Account discovery (~100 lines)

8. `graphql(query, operation_name, variables)` helper with DPoP + `ath` + nonce retry.
9. `ResolveBrokerIds`, `BrokerOverview`, cash breakdown → brokerage `RemoteAccount`.
10. `DiscoverOvernightAccounts` + `OvernightSummary` → savings accounts.
11. `ListFixedTermAccounts` + `GetFixedTermAccount` → savings accounts (balance only).

### Phase 4 – Transactions (~150 lines)

12. `BrokerTransactions` paging with `fromTime`, union-type parsing, mapping table above.
13. `OvernightTransactions` paging and mapping.
14. Sign/status rules confirmed with live data; unit tests from recorded JSON fixtures
    (sanitised) in `connectors/scalable.rs::tests`.

### Phase 5 – Wiring (~80 lines Rust, ~40 lines Swift)

15. `models.rs`: enum variant, `as_str`, `all`. `connectors/mod.rs`: descriptor,
    `make_connector_for`. `connections.rs`: no login normalisation needed.
16. `demo.rs` demo accounts; `make_connector_for(.., demo = true)` stub.
17. Rebuild xcframework; Swift: `Formatting.swift`, `ConnectionsUI.swift`,
    `AppModel.swift`, `BankPicker.swift`, `TwoFactorSheet.swift` (redirect button in
    the push branch).
18. Localisation strings (DE/EN) for the descriptor note and challenge texts.

### Phase 6 – Tests and docs

19. `cargo test -q -p girocrab-core scalable` green; `swift build -q` green;
    `cargo run -- --demo-seed` shows the demo Scalable accounts.
20. README (both languages): move Scalable Capital from "Help wanted" to supported
    with status "⚠️ Unterstützt, inoffizieller Client der offiziellen CLI-API".
    CONTRIBUTING: drop from the wanted list. Release notes entry.

## Risks and open questions

| # | Risk / question | Mitigation |
| --- | --- | --- |
| R1 | Scalable may restrict the public client ID to its own binary (User-Agent, DPoP key attestation, or token claims). Nothing in the CLI source suggests this, but it cannot be excluded. | Verify in phase 1 before investing further. Send the CLI's User-Agent format initially; switch to a GiroCrab UA once confirmed irrelevant. |
| R2 | Schema or JSON changes without notice (Scalable says so explicitly). | Parse defensively, never fail the whole sync on one unknown union member; log the operation name. |
| R3 | 7-day forced re-login and 24-hour idle timeout. | Surface a clear "Erneut anmelden" state in the UI; automatic refresh on every sync keeps the idle window alive for daily users. |
| R4 | Sign convention of `amount` unknown. | Phase 4 live check; add fixtures. |
| R5 | `Start2faOnLogin` may reject non-CLI `deviceName`/`deviceType`. | Try "GiroCrab"/"CLI" first, fall back to the CLI's literal values. |
| R6 | Terms of service: using the API outside the official CLI. | Read-only use of a user's own data with the user's explicit opt-in switch; document it in the connector note like the Trade Republic disclaimer. |
| Q1 | Does `BrokerOverview.valuation` include cash, or only securities? | Compare with `cash_balance` live; choose the figure that matches the app's "Gesamtwert". |
| Q2 | Does `lastEventDateTime` reflect booking or execution time? | Compare with the Scalable app's transaction dates. |

## Live findings

_Empty until phase 1 runs against the real backend. Record sanitised request and
response shapes here, including DPoP nonce behaviour and any UA/client checks._

## Change log

- 2026-11-11 – Initial feasibility study and plan written from scalable-cli v1.3.0 source.
