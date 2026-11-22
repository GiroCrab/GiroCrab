# 0002 – Refund links, stable split identities and shared projection

**Status** · Proposed\
**Date** · 2026-11-22\
**Discussion** · [#714](https://github.com/GiroCrab/GiroCrab/issues/714)

## Context

A refund can cover part of an expense, several expenses or one part of a mixed-sign
split booking. Several refunds can also cover the same expense. Editing splits,
importing a matching bank booking and merging snapshots must not silently change
what the user linked or make allocation depend on device-local row ids.

This ADR records the implementation for #714, including the decision to keep the
complete winning link row during sync rather than taking the minimum creation
stamp. It is proposed for ADR review; it does not declare the feature released.
The [architecture notes](../ARCHITECTURE.md) locate the modules and migration;
the [refund UI guide](../ERSTATTUNGEN_UI.md) describes platform interactions.

## Decision drivers

1. Preserve bank bookings, user-authored splits and account balances.
2. Converge across snapshot merge orders and different local database ids.
3. Resolve changed endpoints conservatively, without expanding the user's intent.
4. Give every report and front end the same refund-aware financial totals.
5. Make undo conditional and keep scoped reports from revealing hidden accounts.

## Decision

### Links are the only persisted source of refund truth

`refund_links` stores the natural key `(refund_id, refund_part, original_id,
original_part)`, signed endpoint amounts captured at link time, creation/change
stamps and a deletion stamp. A part is addressed by UUID; the empty string means
the whole booking only while that booking is unsplit. Endpoint sign, not booking
net, determines its role. Distinct bookings in the same currency are required;
link validation enforces the endpoint and chain rules.

Refund actions never manufacture or edit split lines, move a bank booking or
change its amount. Resolved endpoints, allocated cents, row markers and report
contributions are derived, not a second persisted refund model.

### Split identities survive edits and sync

Schema step 38 adds a non-null split UUID and unique index, keeps presentation
order separately in `position`, and creates `refund_links`. The
[backfill](../../girocrab-core/src/planning/split_ids.rs) derives UUID-v5 values
from booking sync identity, category UUID, signed amount, note and an occurrence
number among identical parts. Different local ids or ordering of distinct parts
therefore produce the same identities; indistinguishable duplicate parts receive
the same UUID set, not a promised identity for each old row position.

New parts receive UUID-v4 identities. Kept parts retain their UUIDs, including
when their content changes. Editors pass UUIDs back; sync preserves the winning
whole split vector's UUIDs and order, and twin folding preserves moved UUIDs.
This does not introduce per-part conflict merging. An incoming nonempty vector
whose sum differs from the retained booking amount is not applied.

### One pure resolver, never widening

The [resolver](../../girocrab-core/src/refunds.rs) uses current booking/split state
and stored amount snapshots. An existing endpoint must still have the appropriate
nonzero sign. If its part is missing, including a whole booking that has since
been split, resolution requires a unique admissible same-signed-amount replacement
within that booking. Ambiguous, missing, conflicting or incompatible endpoints
make the link inactive. Existing resolved pairs are reserved so fallback cannot
duplicate a canonical pair.

Resolution never takes several replacement parts, spreads a link across them or
substitutes a larger parent amount. It does not infer a new user choice from
available capacity. Read-time resolution writes nothing. Local split edits,
sign-changing manual amount edits and booking deletion use the same resolver to
persist retargeting or tombstones. Snapshot merge does not perform that cleanup;
otherwise the merge order of booking, split and link state could change the result.

### Allocation is deterministic and revision-cached

For live resolved links, the core makes one capacity-consuming pass ordered by
`(created_ms, refund booking sync identity, refund_part, original booking sync
identity, original_part)`. Each link receives
`min(remaining refund capacity, remaining original capacity)`. Neither local
booking ids nor mutable change stamps break allocation ties. Capacity comes from
the selected endpoints, not the booking net; zero-net and mixed-sign split
bookings can therefore participate in either role through their parts.

The allocation cache is bound to the vault data revision. Booking, split and link
writes and snapshot merges invalidate derived state through revision tracking.
Repeated reports reuse allocation instead of independently assigning cents.

### Merge complete rows after exact identity mapping

[Link merge](../../girocrab-core/src/refunds/store.rs) maps both peer booking ids
through the exact booking map produced by the booking merge. It never assumes
that equal local integers identify the same booking. [Twin folding](../../girocrab-core/src/sync_merge/twins.rs)
also supplies explicit maps for moved bookings and unaffected endpoints; it does
not guess counterpart ids or discard their split UUIDs.

For a natural-key collision, choose the maximum complete row under the total rank
`(changed_ms, deleted_ms.is_some(), created_ms, refund_cents, original_cents,
deleted_ms)`. Keep every field of that winner, including `created_ms`. Tombstones
participate in the same rule, including collisions after twin remapping. Do not
resolve endpoints, generate timestamps or rewrite payloads during link merge.

### Undo is compare-and-set

[Unlink](../../girocrab-core/src/refunds/api.rs) returns the tombstone's change
stamp as its undo token. Restore succeeds only if the row is still deleted and
its current change stamp equals that token. It retains the original creation
stamp and allocation priority; a newer peer tombstone cannot be revived by an
older undo. Explicitly re-linking a tombstoned key instead gets a fresh creation
stamp rather than reviving its old priority.

### Every financial total uses the shared projection

The [projection](../../girocrab-core/src/projection.rs) consumes core allocation.
Allocated cents are removed from the refund endpoint's category and attributed
on the refund's date and account to the original endpoint's category. Their
expense/income effect follows the original **part's existing classification**,
not the parent booking net. Transfer and loan exclusions are preserved.
Unallocated positive cents remain income at their original location.

Category totals/history, budgets, cashflow, insights, selection flow, analytics
and local API summaries use that projection, including custom-category subtrees
and uncategorized contributions. Currencies remain separate (#569). Scoped
summaries resolve and allocate using only permitted endpoints, so hidden accounts
cannot affect visible allocation or leak refund metadata. Financial totals must
not be reconstructed from `list_transactions` presentation rows.

The [source lint](../../girocrab-core/src/projection/lint.rs) scans core, Linux and
Windows Rust sources for category/split aggregation outside the projection. It
is a guard against known patterns, not a proof that every possible aggregate is
caught. Front ends display core results; row-local presentation and bank-ledger
grouping are not alternative category-report implementations.

## Not covered

Manual booking amount sync, per-part split conflict merging and cross-currency
refund conversion. Suggestion ranking and platform layouts are UI/API policy,
not alternative sources of link or allocation truth.

## Alternatives considered

- **Synthetic refund splits** would reuse split-based reports but overwrite user
  structure and make derived state compete with user edits and peer snapshots.
- **Positional part ids** would be simpler to store, but reordering or deleting a
  part would redirect an existing link to different content.
- **Widening or merge-time endpoint repair** could retain more links, but would
  guess user intent and make results depend on which snapshot arrived first.
- **Taking `min(created_ms)` on collision** would preserve the earliest competing
  creation, but puts a re-linked key back at its old priority. For example, merging
  a tombstone created at 10 with a winning re-link created at 30 must not reset
  the winner to 10. Modifying a ranked winner's creation stamp also undermines
  the full-row merge rule's associativity argument.
- **Per-screen totals or allocation** would duplicate classification, currency and
  scope rules and let different reports disagree about the same refund.

## Consequences

User splits remain editable without becoming refund bookkeeping. Concurrent
snapshots converge under the same total row rank and identity-based allocation.
An ambiguous or invalidated link can become inactive instead of being guessed
back into existence; the UI omits it and may require an explicit new assignment.
Concurrent creations of the same natural key take the winning row's priority,
not necessarily the earliest creation seen on either peer.

New reports must integrate with the shared projection and its revision/scoping
rules. UUID migration, exact id maps, retained tombstones and caches add complexity
and storage cost. Revisit this decision if endpoints need a different durable
identity or scoped/currency semantics change, rather than adding UI-only repairs.

## Confirmation

[Split identity tests](../../girocrab-core/src/planning/split_ids.rs) cover
row-order/local-id-independent backfill and non-null/unique storage. The
[refund tests](../../girocrab-core/src/refunds/tests.rs) cover allocation, mixed
roles, conservative resolution, CAS undo, full-row LWW and generated two-peer
histories compared in both merge orders. Sync tests cover split-vector retention
and twin maps/collisions. [Projection tests](../../girocrab-core/src/projection/tests.rs)
cover report parity, classification, remainder, currency and privacy scope.
The aggregation lint has positive and negative detection fixtures. UI checks do
not replace these core invariants or establish event-loop race correctness.
