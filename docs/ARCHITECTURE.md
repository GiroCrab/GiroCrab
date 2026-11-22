# Architecture notes

The reasons behind lasting decisions (alternatives, accepted trade-offs) are recorded as
[architecture decision records](adr/README.md).

## Schema & migrations

The vault is a SQLCipher database. Its schema is versioned in
`girocrab-core/src/schema.rs`: `PRAGMA user_version` counts the steps a vault has run,
and `migrate` runs the missing ones in a single transaction.

| Version | Step |
|---------|------|
| 1 | `BASELINE`: the complete schema of a new vault |
| 2–8 | empty – vaults from before #206 carry these numbers, reached through probes |
| 9 | `repair_to_v8`: the old probe-based setup, run once, so every vault ends up identical |
| 10 | opening balances (#212) |
| 11 | `sync_log`: the Abrufprotokoll (#238), local to the device |
| 12 | SEPA references of a booking (end-to-end id, mandate reference, creditor id) |
| 13 | tangible assets (`asset_items`, quantities, values, prices) |
| 14 | `security_transactions`: orders and distributions of depots, extra `holdings` columns |
| 15 | product ids of an order's items (#354) |
| 16 | asset classification (#351, `docs/asset-classification.md`) |
| 17 | planned transfers |
| 18 | standing orders |
| 19 | repair of step 15 for vaults of development builds |
| 20 | mandates and creditor ids |
| 21 | kind of a planned transfer (credit / debit) |
| 22 | return acknowledgement of a mandate |
| 23 | booking tags (#228) |
| 24 | a rule restricted to several accounts (#562) |
| 25 | index `transactions(amount, date)` |
| 26 | native currency of a depot position and its prices (#625) |
| 27 | built-in category and account-group identities (#678) |
| 28 | rules that set tags (#580): `rules.tag_uuids`, `rules.category_id` nullable |
| 29 | `depot_values`: value history of securities depots |
| 30 | `tx_date_id` also covers `deleted_ms`, `account_id` and `category_id` (counts and period sums from the index) |
| 31 | `booking_documents.seen_at`: read status of documents; the document list index carries the column, a partial index finds the unread ones |
| 32 | `accounts.note` / `note_ms`: the account note syncs like the name (#980); moves the setting `account.note.<id>` onto the account |
| 33 | `hidden_recipients`: IBANs left out of the transfer suggestions (#978); index `tx_recipients` over the outgoing bookings with an IBAN |
| 34 | per-account total inclusion and its sync stamp |
| 35 | merchant recognition switches and custom merchants |
| 36 | per-account bank label and its sync stamp |
| 37 | sidebar sections by account kind |
| 38 | stable split UUIDs with deterministic backfill, separate split position and `refund_links` (#714) |

The tables of securities depots are described in `docs/DEPOT_DATENMODELL.md`.

**Changing the schema**

1. Append a step to `STEPS` (SQL, or a hook for Rust logic such as backfills). Never edit or
   reorder an existing step: vaults in the wild have already run it.
2. Run `cargo run -q -p girocrab-core --example make_fixture_vault`. It writes
   `girocrab-core/tests/fixtures/vault-v{N}.girocrab`; commit it. Never regenerate or edit an
   older fixture – each stands for vaults written by that version.
3. `cargo test -q -p girocrab-core --test schema_fixtures` writes `fixtures/schema-v{N}.sql`
   (normalised `sqlite_master`) on first run. Review and commit it, too. Every fixture vault must
   open and end up with exactly the schema of a fresh vault.

**How a vault opens** (`Vault::migrate`)

1. Key check (`SELECT … FROM sqlite_master`); a wrong key is `CoreError::WrongKey`, which makes
   `Vault::open` try the next `cipher_compatibility` mode.
2. Downgrade guard: `user_version` above this build's `SCHEMA_VERSION` →
   `CoreError::SchemaTooNew`, before anything is written. The file stays untouched. The same
   applies to the peer file in an iCloud snapshot merge. Builds from before #206 have no guard.
3. `foreign_keys` / `journal_mode = WAL` (both no-ops inside a transaction).
4. All pending steps run in **one** transaction, which also sets `user_version`. If a step
   fails, nothing changes.

The snapshot export (`write_snapshot`) stamps `user_version = SCHEMA_VERSION` on the copy.

**Rules of thumb**

- Counts filter `deleted_ms IS NULL`: indexes used by count queries must cover that column.
- Index changes are a migration step, not a probe that runs on every open.
- Data repairs that must run on every open (e.g. `fill_missing_uuids`) are called from
  `Vault::open` directly, not from the schema steps.

## Vault modules

`Vault` is one object (one SQLCipher connection behind a mutex) with many feature modules. Each
module adds its own methods to it and reaches the database through `self.db()`.

- `vault.rs` only opens, keys, migrates and snapshots the vault (`open`, `rekey`,
  `write_snapshot`, `checkpoint`, `is_empty`) and holds the schema constants.
- Feature SQL lives in the module of its feature, never in `vault.rs`:

  | Module | Feature |
  |--------|---------|
  | `vault/accounts.rs` | bank logins, accounts, account groups, holdings, balance history |
  | `vault/bookings.rs` | booking list (filter, paging, enrichment), booking edits, CSV export |
  | `vault/categories.rs` | category tree |
  | `vault/rules.rs` | pattern and expression rules |
  | `vault/settings.rs` | settings and `KNOWN_SETTINGS` |
  | `vault/merge.rs` | merging a peer snapshot or another vault file |
  | `refunds.rs`, `refunds/` | UUID endpoint resolution, allocation, link API, suggestions and full-row link merge |
  | `projection.rs`, `projection/` | shared refund-aware category/flow contributions, reports and aggregation lint |
  | `loans.rs`, `planning.rs`, `documents.rs`, `insights.rs`, `connectors/*.rs`, `importers/*.rs`, … | their feature |

- A feature module has at most one `#[uniffi::export] impl Vault` (the Swift API) and one plain
  `impl Vault` (helpers taking `&Connection`, for use inside other transactions). Helpers are
  private unless another module calls them; prefer `pub(super)` inside `vault/` over `pub(crate)`.
- Free functions for Swift are exported only from `ffi.rs`, as wrappers that forward to the
  feature module (`scripts/lint-ffi-exports.sh`, CI). Elsewhere `#[uniffi::export]` marks only
  `impl` blocks of exported objects and callback traits.
- `unsafe` code lives only in `sys.rs` (safe wrappers around the libc calls std lacks): the
  core's `Cargo.toml` denies `unsafe_code` (every other workspace member forbids it) and
  `scripts/lint-no-obfuscation.sh` (CI) lets only that file lift it. The script also keeps
  processes, embedded files other than `data/` and dynamic loading out of the core and
  Base64/hex decoding out of the payment path. Two checks back its text patterns: `clippy.toml`
  bans process and loader calls by resolved path, and `--compiled` (CI) lists the files and
  environment variables the compiler read for the library.
- Modules under `vault/` are children of `vault`, so they see its private fields and helpers
  (`kept_key`, `migrate`). Other modules use `Vault::db()` only.
- Tests sit in the module whose code they exercise.
- A new feature gets a new module. Don't add another section to an existing file.

## Refund links and financial projection

Refund links are stored independently of bank bookings and user splits. Split
parts have durable UUIDs; their display position is not an endpoint identity.
`planning/split_ids.rs` implements migration 38. `planning.rs` preserves kept
UUIDs, and the split and twin merge paths retain them.

`refunds.rs` resolves endpoints and allocates capacity in sync-identity order;
`refunds/store.rs` merges complete link rows after exact booking-id mapping,
including import take-overs. Local edits and deletions keep original link keys
and stamps; resolution never writes derived retargets or tombstones.
`refunds/api.rs` exposes link/unlink and compare-and-set restore. Allocation is
cached by vault data revision, not persisted as additional financial truth.

`projection.rs` and `projection/reports.rs` supply financial totals to core
reports and front ends. Refund attribution uses the refund's account/date and
the original endpoint's category/classification. Currencies and permitted account
scopes stay separate. `projection/lint.rs` guards against reintroducing raw
category/split sums in core, Linux and Windows consumers; transaction rows are
for presentation, not a replacement report source.

See [ADR 0002](adr/0002-refund-links-and-projection.md) for the rules, rejected
alternatives and test evidence, and the [refund UI guide](ERSTATTUNGEN_UI.md)
for platform entry points and privacy behavior.

Manual bookings in offline accounts merge amount and split vector atomically
under `splits_ms`, with a deterministic value tie-break. Bank booking amounts
keep their existing merge semantics. No additional schema step is needed.
