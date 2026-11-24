# 0003 – Refund links and stable split identities

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
4. Leave every report unchanged: a link is documentation, not bookkeeping.
5. Make undo conditional.

## Decision

### Links are the only persisted source of refund truth

`refund_links` stores the natural key `(refund_id, refund_part, original_id,
original_part)`, signed endpoint amounts captured at link time, creation/change
stamps and a deletion stamp. A part is addressed by UUID; the empty string means
the whole booking only while that booking is unsplit. Endpoint sign, not booking
net, determines its role. Distinct bookings in the same currency are required;
link validation enforces the endpoint and chain rules.

Refund actions never manufacture or edit split lines, move a bank booking or
change its amount. Resolved endpoints, allocated cents and row markers
are derived, not a second persisted refund model.

### Split identities survive edits and sync

Schema step 39 adds a non-null split UUID and unique index, keeps presentation
order separately in `position`; step 40 creates `refund_links`. The
[backfill](../../girocrab-core/src/planning/split_ids.rs) derives UUID-v5 values
from booking sync identity, category UUID, signed amount, note and an occurrence
number among identical parts. Different local ids or ordering of distinct parts
therefore produce the same identities; indistinguishable duplicate parts receive
the same UUID set, not a promised identity for each old row position.

New parts receive UUID-v4 identities. Kept parts retain their UUIDs, including
when their content changes. Editors pass UUIDs back; sync preserves the winning
whole split vector's UUIDs and order, and twin folding preserves moved UUIDs.
This does not introduce per-part conflict merging. For manual bookings of offline
accounts, amount and split vector merge as one unit under `splits_ms`, with a
deterministic `(amount, vector)` tie-break. Amount-only edits stamp that same unit.
Bank amounts remain bank-owned; an incompatible incoming split vector is not applied.

Upgrade limitation: pre-upgrade amount-only edits have no recoverable edit stamp.
If those amounts diverged while `splits_ms` stayed equal, the first merge after
upgrade converges by deterministic `(amount, vector)` value, not by edit recency.
This can select an older amount. Bumping the stamp at upgrade would only replace
that deterministic choice with whichever device upgraded last.

### Schema cutover and recovery

Opening an older vault runs the pending steps through 41 (39: split UUIDs, 40: `refund_links`, 41: date stamp of manual bookings) in the migration transaction. A build
supporting only schema 38, 39 or 40 cannot open the migrated vault or merge its snapshots;
the version guard refuses them before writing. Reinstalling the older executable
does not undo the migration. There is no schema downgrade path.

Before upgrading, pause sync on every device and keep a pre-upgrade vault backup
outside the active sync location. Upgrade every syncing device to a compatible
build before resuming sync. During a staggered upgrade, older peers must remain
out of sync with the upgraded vault.

Recovery should use a compatible build with the upgraded vault to retain edits
made after migration. If an older build is unavoidable, stop sync on all devices,
preserve a separate copy of the upgraded vault, and restore the pre-upgrade backup
in an isolated location. That backup lacks edits made since it was taken; neither
reinstalling the app nor restoring the backup transfers those edits back. Do not
merge newer snapshots into the restored vault. Before resuming, move or archive the
upgraded snapshots out of the active sync location (or point every device at a
clean shared location for the restored vault). Resume shared sync only after all
devices use compatible builds and the same chosen vault history.

### One pure resolver, never widening

The [resolver](../../girocrab-core/src/refunds.rs) uses current booking/split state
and exact endpoint identities. An existing endpoint must still have the appropriate
nonzero sign. Missing UUIDs, including a whole-booking marker while the booking
is split, make the link inactive. There is no read-time amount matching.

Replacement happens in the local split-vector save: drop unless a same-amount
replacement exists in the same edit. A new part inherits a removed UUID only if
the edit removes exactly one part of that signed amount, that part is referenced,
and exactly one new part has the amount. All removed parts count, referenced or
not. Retained parts are not replacement candidates. Several removed parts or
several new candidates of that amount mean no inheritance. Otherwise the old UUID
disappears; a later edit creating that amount cannot revive it. The vector and
its identities sync as one unit; this does not write any link row.

Resolution never takes several replacement parts, spreads a link across them or
substitutes a larger parent amount. It does not infer a new user choice from
available capacity. Resolution writes nothing, including after local split edits,
sign changes or true booking/account deletion. The original link key, snapshots
and stamps remain unchanged; deleted or incompatible bookings make it inactive
at read time. Split edits never create a link tombstone or a successor link row.

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

Import deduplication and take-over paths apply the same row-ranking mechanism
when remapping actual booking identities before deleting the redundant booking.
The map addresses complete endpoints, including part UUIDs. Consolidating imported
part bookings explicitly maps each whole part booking to the split UUID created
from it, even when several parts have equal amounts. Other identity take-overs
preserve a surviving UUID first. When an endpoint disappears on the survivor,
the take-over persists a replacement only if exactly one survivor endpoint has
the old endpoint's signed amount. A whole-booking source uses the booking amount;
an unsplit survivor contributes one whole-booking endpoint. A split part never
moves to the whole booking of an unsplit survivor, even with an equal signed
amount: it stays inactive. Two equal matches,
no match, or an already missing source part leave the old part inactive. Link
snapshots are not used to resurrect a missing source. Imported-part consolidation
applies this rule to an existing whole-booking link on the bank booking too.
If distinct source rows would remap to the same complete destination identity,
the colliding remaps remain inactive. The fold proceeds without merging them.

This exception runs only at the identity take-over boundary, not during reads,
ordinary snapshot merges or later split edits. It never selects a larger part,
combines parts or reallocates by available capacity. Signed equality and uniqueness
make the choice independent of part order and device-local ids. The persisted
endpoint change syncs as a complete row. Canonical twin/duplicate folds preserve
the original stamps and retain a moved tombstone at the old canonical part key,
so an unlink from a peer still holding that part can suppress its successor.
Booking merge installs the winning split vectors after merging all booking
deletions. A UUID transferred to a live survivor is not also restored on its
deleted former parent by a pre-fold peer snapshot.

A remap stamps the old key as a moved tombstone and writes the mapped row with
`moved_from` pointing to that old key, preserving creation/order. At read time,
the mapped row is inactive if that source key or a move ancestor holds a user-unlink tombstone
(not a moved tombstone) whose change stamp is at least the mapped row's stamp.
Undo of that exact user unlink makes the source row live again, reactivating the
mapped row; the deleted source booking keeps the old endpoint inactive. This
provenance is mapped through sync booking identities, not local integer order.
Canonical duplicate/twin folds remap complete keys and provenance without new
stamps, including tombstones and rows whose current endpoints are unaffected.
Physical cleanup clears provenance references without a surviving identity by FK.

For a natural-key collision, choose the maximum complete row under the total rank
`(changed_ms, deleted_ms.is_some(), created_ms, refund_cents, original_cents,
deleted_ms, user-tombstone priority, moved_from key snapshot, unlink_id)`.
Mapping availability does not participate in the rank. Already suppressed rows are
not moved again, so a new move stamp cannot override an earlier user unlink.
Equal logical rows retain mapped provenance when either copy has it.
The final key snapshot stores sync identities and parts at move time; merge does
not recompute its order from mutable bookings or local integer ids.
Keep every field of that winner, including `created_ms` and provenance. Tombstones
participate in the same rule, including collisions after twin remapping. Do not
resolve endpoints, generate timestamps or rewrite payloads during link merge.

### Undo is compare-and-set

[Unlink](../../girocrab-core/src/refunds/api.rs) persists and returns a fresh
UUID-v4 operation identity as its undo token (`unlink_id`). Restore succeeds
only if the row is still deleted, its nonempty operation identity equals that
token and it is not a moved marker. A restored row is active only while its
endpoints pass the resolver (e.g. a whole-booking link stays inactive while that
booking is split and becomes active again once it is unsplit). Restoring a link whose
endpoint is currently invalid returns an error and leaves the tombstone in place; the
same undo can be retried after the booking is unsplit. It retains the original creation
stamp and allocation priority; a newer peer tombstone cannot be revived by an
older undo. Explicitly re-linking a tombstoned key instead gets a fresh creation
stamp rather than reviving its old priority.

### Links are documentation only

A link changes no report. Category totals and history, budgets, cashflow, insights,
the selection sums and the local API count the refund booking where its own
category puts it, exactly as without the link. Netting a refund against its
purchase stays an explicit user decision: putting both bookings (or parts) into
the same category already does that today. Allocated cents are only shown: as
row markers, in the refund card and as the open capacity in the assignment
dialog (#714: "links must not change amounts, categories or balances").

## Not covered

Bank booking amount sync changes, per-part split conflict merging and cross-currency
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
  the full-row merge rule's associativity argument: with live A at creation 100,
  tombstone T and a later re-link L at creation 200, `A + T = T` and
  `T + L = L(200)`, while applying a later minimum against A would yield 100.
- **A shared refund-aware report projection** (allocated cents moved into the
  original part's category in every total) was implemented first. It only differs
  from assigning the same category when the user chose a different one, i.e. it
  would override the user's categorisation without being asked, and every new
  report would have to integrate with it. Kept on the branch
  `archive/714-refunds-with-projection` for a later decision.

## Consequences

User splits remain editable without becoming refund bookkeeping. Concurrent
snapshots converge under the same total row rank and identity-based allocation.
An ambiguous or invalidated link can become inactive instead of being guessed
back into existence; the UI omits it and may require an explicit new assignment.
Concurrent creations of the same natural key take the winning row's priority,
not necessarily the earliest creation seen on either peer.

Reports need no refund awareness. UUID migration, exact id maps, retained tombstones and caches add complexity
and storage cost. Revisit this decision if endpoints need a different durable
identity or scoped/currency semantics change, rather than adding UI-only repairs.

## Confirmation

[Split identity tests](../../girocrab-core/src/planning/split_ids.rs) cover
row-order/local-id-independent backfill and non-null/unique storage. The
[refund tests](../../girocrab-core/src/refunds/tests.rs) cover allocation, mixed
roles, conservative resolution, CAS undo, full-row LWW and generated two-peer
histories compared in both merge orders. Sync tests cover split-vector retention
and twin maps/collisions. UI checks do
not replace these core invariants or establish event-loop race correctness.
