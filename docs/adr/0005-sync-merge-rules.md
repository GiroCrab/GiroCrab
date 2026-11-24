# 0005 – Rules for merging device vaults

**Status** · Proposed\
**Date** · 2026-11-24\
**Discussion** · TRACKING_ISSUE, [#1840](https://github.com/GiroCrab/GiroCrab/issues/1840), [#1869](https://github.com/GiroCrab/GiroCrab/issues/1869), [#1833](https://github.com/GiroCrab/GiroCrab/pull/1833)

## Context

Every device keeps its own encrypted vault. `Vault::merge_other_vault`
([`vault/merge.rs`](../../girocrab-core/src/vault/merge.rs)) merges another device's file into
it: it migrates the other file to the current schema, then runs one merge step per entity
([`sync_merge.rs`](../../girocrab-core/src/sync_merge.rs) and its submodules). The building
blocks are documented there and this ADR describes them as they are:

- **Per-field stamps.** A synced field has a `*_ms` stamp from `dates::now_ms`; `take_newer`
  keeps the newer value, on equal stamps the larger value.
- **Tombstones.** `deleted_ms` marks a deleted row; a deletion is never undone by a merge.
  `merged_into` points from a combined duplicate to its survivor; bookings follow it.
- **Deterministic identities.** Categories get a UUIDv5 from their path, tags from their
  name, rules from their content, so two vaults that never met agree on them. After a rename
  or move a category keeps its uuid; a category created anew under the new path gets another
  one, and the merge combines same-named live siblings by name (`combine_categories`).
- **Version skew.** An older build refuses a vault written by a newer one
  (`schema::check_not_too_new`); a newer build migrates an older file before merging.

What is not written down is what a merge must guarantee. Reviews of
[#1833](https://github.com/GiroCrab/GiroCrab/pull/1833) (category roles) went through many
rounds that each found one more case where the result depended on the order in which devices
synced, or where an upgrade changed a vault's numbers. Most of them had one of three causes:

1. **Derived values written during the merge.** Copying a duplicate's role into its survivor
   (`forward_merged_roles`, removed in #1833) stuck to whichever survivor an earlier merge
   chose. A randomized test found 112 order-dependent cases out of 2591.
2. **Defaults stored like choices.** Schema step 42 seeds roles with stamp `0` in the same
   column as user choices. Equal stamps then fall back to comparing values (`Some` beats
   `None`), which decides between two defaults by accident.
3. **Migrations that read context.** Step 42 derives a role from a category's position in the
   tree. Devices that migrate before and after a sync see different positions.

The same structure shows outside roles: whether two same-named categories get combined
depends on the merge order ([#1869](https://github.com/GiroCrab/GiroCrab/issues/1869)), and
wall-clock stamps let a device with a fast clock win
([#1840](https://github.com/GiroCrab/GiroCrab/issues/1840)).

Names from #1833 used below (`role_winners`, `role_ms`, schema step 42 and its tests) exist on
`main` once that PR is merged.

## Decision drivers

1. **Devices converge.** Once every device has merged the same files, all hold the same data,
   whatever the order.
2. **A user's choice survives.** No merge, import or upgrade replaces something the user chose
   with something the app derived.
3. **Upgrades keep the numbers.** A migration does not change KPIs or cash flows of an existing
   vault, also not after the next sync.
4. **Reviewable.** A reviewer can check a merge change against stated rules and tests instead
   of finding cases one by one.

## Decision

Proposed rules for every synced entity. Where the code does not follow them yet, the tracking
issue lists the work.

### 1. Merging is order-independent

For every synced field the merge of two values is commutative, associative and idempotent.
Ties are broken deterministically and the rule is written next to the merge step. A merge
never stamps anything itself (as today).

### 2. Synced columns hold facts, not derived values

A synced, stamped column holds what a user or an import did. Values derived from other rows
— survivors, combined duplicates, a category's effective role, totals — are computed when
reading, or recomputed from the merged facts as a whole after each merge. A merge step does
not write a derived value as if someone had edited it. Example: since #1833 the role of a
combined duplicate stays on the duplicate and counts for the survivor when reading
(`sync_merge::role_winners`).

### 3. Defaults are not choices

A value the app sets on its own (migration, import, starter data) is marked as such and loses
against any user choice. It does not travel to other rows as if it were a choice (#1833: only
roles with `role_ms > 0` count for a survivor). Whether stamp `0` stays the marker or defaults
move to a column of their own is open for comment.

### 4. Identities are deterministic

A new synced entity gets a deterministic identity like the existing ones. Matching by name is
a fallback for rows created independently under the same name, and its merge rule is part of
rule 1 (#1869).

### 5. Migrations of synced data say what they read

A schema step that writes synced values documents which context it reads beyond the row
itself, and how the merge treats devices that migrated with different context (as step 42
does for the tree position). Prefer steps that read only the row.

### 6. Version skew stays as it is

Older builds refuse newer vaults; newer builds migrate older files before merging.

## Not covered

- **Clocks.** Replacing wall-clock stamps with hybrid logical clocks and a device id as
  tie-break is left to #1840; rule 1 holds with either.
- **The merge transport.** How files reach other devices (iCloud) and when a merge runs.
- **Individual fixes.** #1869 and the other items of the tracking issue decide their own
  details.

## Alternatives considered

- **Keep fixing cases as reviews find them.** Each fix is small, but every reviewer has to
  rediscover the rules, and a fix can reintroduce an earlier case (#1833: a filter added for
  one review finding made roles order-dependent again). Fails driver 4.
- **Adopt a CRDT library or a CRDT extension for SQLite.** Gives rule 1 by construction, but
  replaces the merge of every entity, the file-based sync of encrypted vaults and the
  migration path of existing vaults at once. Worth revisiting if the property tests show
  that rule 1 cannot be kept with the current design.
- **Sync an operation log instead of states.** Merges operations in causal order instead of
  comparing states. Needs a log in every vault and a way to compact it; a larger change than
  the problems so far justify.

## Consequences

- Merge changes are reviewed against the rules above; a violation needs a reason in the code.
- Known violations become tracked work instead of review findings: `combine_categories`
  writes combinations that a newer rename on another device cannot undo (#1869) and merges
  `builtin_role` into the survivor; `resolve_parents` writes derived parents.
- Defaults need a marker everywhere the app seeds synced data, not only for roles.
- Re-evaluate when the property tests find a class of violations the current design cannot
  fix, or when #1840 changes the stamps.

## Confirmation

- A property test per synced entity: random devices make random edits, then merge in every
  order; all orders must give the same data. `roles_do_not_depend_on_merge_order_on_random_states`
  in [`sync_merge/categories.rs`](../../girocrab-core/src/sync_merge/categories.rs) (#1833)
  is the first one; the tracking issue extends it to every entity.
- For a schema step that changes how data is classified: a comparison with an independent
  copy of the old behaviour on random data (as `category_roles_keep_v41_classification_on_random_trees`
  for step 42).
- Review checklist for changes under `sync_merge/` and to schema steps that write synced
  columns.
