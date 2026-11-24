# 0005 – Rules for merging device vaults

**Status** · Proposed\
**Date** · 2026-11-24\
**Discussion** · TRACKING_ISSUE, [#1840](https://github.com/GiroCrab/GiroCrab/issues/1840), [#1869](https://github.com/GiroCrab/GiroCrab/issues/1869), [#1833](https://github.com/GiroCrab/GiroCrab/pull/1833)

## Context

### How devices sync today

Every device keeps its own encrypted vault. To sync, a device reads the vault file of another
device and merges it into its own (`Vault::merge_other_vault` in
[`vault/merge.rs`](../../girocrab-core/src/vault/merge.rs)). There is no server that decides
what is right; each device merges on its own, whenever a file arrives. The merge works like
this ([`sync_merge.rs`](../../girocrab-core/src/sync_merge.rs) and its submodules):

- **Each editable field carries the time of its last edit.** A category's name has
  `name_ms`, its parent `parent_ms`, and so on. When both vaults have the same category, the
  merge keeps, field by field, the value that was edited last (`take_newer`). If both times
  are equal, it keeps the larger value.
- **Deleted rows stay as markers.** A deleted row keeps its data with a deletion time
  (`deleted_ms`), so the deletion reaches other devices. A merge never brings it back.
- **Duplicates are combined.** If two devices created a category with the same name under
  the same parent, the merge keeps one and marks the other as merged into it
  (`merged_into`); the bookings of the second move to the first.
- **The same thing gets the same id on every device.** A category's id is computed from its
  path at creation, a tag's from its name, a rule's from its content. Two devices that create
  "Lebensmittel › Supermarkt" independently therefore agree that it is one category. A renamed
  category keeps its old id, so only then does the merge have to fall back to matching by
  name.
- **Different app versions.** An older app refuses to open a vault written by a newer one
  (`schema::check_not_too_new`). A newer app first upgrades the other device's file to its own
  schema, then merges.

### What went wrong

None of this says what a merge must guarantee. The reviews of
[#1833](https://github.com/GiroCrab/GiroCrab/pull/1833) (category roles) went through many
rounds. Each round found another case where two devices ended up with different data
depending on which one synced first, or where the upgrade changed how existing bookings were
counted. Most cases had one of three causes:

1. **The merge stored a value it had worked out itself.** When a duplicate category was
   combined, the merge copied its role into the remaining category. If a later sync assigned
   the duplicate to a different category, the copy stayed where it was. A randomized test with
   three devices found 112 of 2591 cases where the result depended on the order of syncs.
2. **Values the app set on its own looked like a user's choice.** The upgrade (schema step 42)
   gives built-in categories a role and stores it in the same field as a role the user picks,
   with time `0`. When two devices disagreed, the rule "on equal times keep the larger value"
   decided between two values that nobody had chosen.
3. **The upgrade read more than the row it changed.** Step 42 gives a category a role
   depending on where it sits in the tree. Each device upgrades on its own, at a different
   time; a device that upgrades after a sync has moved a category sees another tree than one
   that upgraded before.

The same problems exist outside roles. Whether two same-named categories get combined
depends on the order of syncs ([#1869](https://github.com/GiroCrab/GiroCrab/issues/1869)).
Edit times come from the device clock, so a device whose clock runs ahead wins
([#1840](https://github.com/GiroCrab/GiroCrab/issues/1840)).

Names from #1833 used below (`role_winners`, `role_ms`, schema step 42 and its tests) exist on
`main` once that PR is merged.

## Decision drivers

1. **All devices end up with the same data.** Once every device has merged the same files,
   they hold the same data, no matter in which order the files arrived.
2. **What the user chose stays.** No merge, import or upgrade replaces a user's choice with a
   value the app worked out.
3. **Upgrades do not change the numbers.** After an upgrade an existing vault shows the same
   income, expenses and balances as before, also after the next sync.
4. **Changes can be checked against rules.** A reviewer compares a change to written rules and
   tests, instead of finding problems case by case.

## Decision

Six rules for everything the devices sync. Where the code does not follow them yet, the
tracking issue lists the work.

### 1. The order of syncs does not matter

A device may receive the files of the others in any order, and it may receive the same file
twice. Every merge step must give the same result in each case. When two devices changed the
same field at exactly the same time, a fixed rule decides which value stays, and that rule is
written next to the merge step.

*Not allowed:* a result that depends on which device's file arrived first, as in #1869.

### 2. The merge only stores what someone did

Fields that the merge compares by edit time hold edits: something a user did, or what an
import brought in. The merge does not store a value in such a field that it worked out itself
from other rows. Values like that — which category a duplicate belongs to, the role a
category inherits, totals — are computed when the app reads the data.

*Why:* a stored value stays even when the rows it came from change on another device later.
Which value stays then depends on the order of syncs (cause 1 above).

*Example:* since #1833 the role of a combined duplicate stays on the duplicate; when reading,
it counts for the category it was combined into (`sync_merge::role_winners`).

### 3. What the app sets on its own never beats a user's choice

Upgrades, imports and the starter categories set values without anyone choosing them. Such a
value must be recognisable as "not chosen", lose against every choice a user makes on any
device, and not be carried over to other rows by the merge.

*Example:* step 42 gives a built-in `Sparen & Investieren` the role `Investment` with time
`0`, which marks it as not chosen. If another device combines this category as a duplicate
into another one, the role does not move along (#1833).

*Open for comment:* whether time `0` stays the marker, or such values move to a field of their
own that the merge never compares.

### 4. Rows get the same id on every device

Something that two devices create independently should get the same id on both, computed from
what it is (as categories, tags and rules do today). Matching by name is only for rows where
that does not work, such as a renamed category. That matching must follow rule 1 as well
(#1869).

### 5. An upgrade that changes synced data reads only the row, or says what else it reads

Each device upgrades its own vault, at its own time. If an upgrade step computes a value from
more than the row it changes — for example from where a category sits in the tree — two
devices can compute different values for the same row. Such a step says in its
documentation what it reads and what happens when devices disagree, and its tests cover
that case. Step 42 does this for the position in the tree.

### 6. A merge never mixes two schema versions

This is already how the app works and stays so: an older app does not open a vault written by
a newer one, and a newer app upgrades the other device's file before merging it.

## Not covered

- **Clocks.** Replacing the device clock with a clock that cannot run ahead (hybrid logical
  clocks, with the device as tie-break) is left to #1840. Rule 1 holds either way.
- **How files reach other devices** (iCloud) and when a merge runs.
- **Individual fixes.** #1869 and the other items of the tracking issue decide their own
  details.

## Alternatives considered

- **Keep fixing cases as reviews find them.** Each fix is small, but every reviewer has to
  work out the rules again, and a fix can bring back an earlier problem: in #1833 a filter
  added for one review finding made roles depend on the order of syncs again. Fails driver 4.
- **Use a library that merges data without conflicts by design (CRDT), or such an extension
  for SQLite.** It would give rule 1 automatically, but it replaces the merge of every kind of
  data, the file-based sync of encrypted vaults and the upgrade path of existing vaults at
  once. Worth revisiting if the tests below show that rule 1 cannot be kept with the current
  design.
- **Sync a list of edits instead of the data.** Devices would replay each other's edits in
  order instead of comparing their data. Every vault would need such a list and a way to keep
  it small; a larger change than the problems so far justify.

## Consequences

- Changes to the merge are reviewed against these rules; code that breaks one needs a comment
  that says why.
- Places that break a rule today become tracked work instead of review findings:
  - `combine_categories` combines duplicates in a way that a later rename on another device
    cannot undo (#1869), and copies `builtin_role` into the remaining category.
  - `resolve_parents` stores a parent it worked out itself.
- Every place where the app sets synced values on its own needs the "not chosen" marker of
  rule 3, not only roles.
- Re-evaluate when the tests below find problems the current design cannot fix, or when #1840
  changes how edit times are taken.

## Confirmation

- **A test per kind of synced data:** several simulated devices make random changes, then
  merge in every order; all orders must give the same data.
  `roles_do_not_depend_on_merge_order_on_random_states` in
  [`sync_merge/categories.rs`](../../girocrab-core/src/sync_merge/categories.rs) (#1833) is
  the first one; the tracking issue extends it to everything that syncs.
- **For an upgrade that changes how bookings are counted:** a comparison with an independent
  copy of the old behaviour on random data, as
  `category_roles_keep_v41_classification_on_random_trees` does for step 42.
- **A review checklist** for changes under `sync_merge/` and for upgrade steps that write
  synced data.
