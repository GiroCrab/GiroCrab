# 0005 – Rules for merging device vaults

**Status** · Proposed\
**Date** · 2026-11-24\
**Discussion** · TRACKING_ISSUE, [#1840](https://github.com/GiroCrab/GiroCrab/issues/1840), [#1869](https://github.com/GiroCrab/GiroCrab/issues/1869)

## Context

### How devices sync today

Every device keeps its own encrypted vault. To sync, a device reads the vault file of another
device and merges it into its own (`Vault::merge_other_vault` in
[`vault/merge.rs`](../../girocrab-core/src/vault/merge.rs)). There is no server that decides
what is right; each device merges on its own, whenever a file arrives. The merge has one step
per kind of data — connections and accounts, categories, rules, bookings, tags and others
([`sync_merge.rs`](../../girocrab-core/src/sync_merge.rs) and its submodules) — and they share
these building blocks:

- **Each editable field carries the time of its last edit** (`*_ms`, from the device clock).
  When both vaults have the same row, the merge keeps, field by field, the value that was
  edited last. Some fields state what wins when both times are equal: for a booking, a manual
  category beats none, the longer note wins, checked beats unchecked
  ([`sync_merge/bookings.rs`](../../girocrab-core/src/sync_merge/bookings.rs)). Elsewhere the
  larger value wins (`take_newer`).
- **Deleted rows stay as markers** (`deleted_ms`), so a deletion reaches other devices; a merge
  never brings a row back.
- **Changes that follow from another change carry no edit time.** When a category is deleted,
  its bookings lose their category without a `category_ms` stamp, so the uncategorising does
  not compete with edits on other devices (`soft_delete_categories`).
- **The same thing gets the same id on every device.** Categories get an id computed from their
  path at creation, tags from their name, rules from their content. Rows that still turn out
  to be duplicates, such as two same-named categories under the same parent, are combined
  (`merged_into`); their bookings move along.
- **Different app versions.** An older app refuses to open a vault written by a newer one
  (`schema::check_not_too_new`). A newer app first upgrades the other device's file to its own
  schema, then merges.

### What is missing

These building blocks are documented per step, but nothing states what a merge as a whole must
guarantee. Reviews therefore find the same kinds of problems again and again, each as a single
case:

- **Results that depend on the order of syncs.** Whether two same-named categories get
  combined depends on which device's file arrives first, and a later rename does not undo it
  ([#1869](https://github.com/GiroCrab/GiroCrab/issues/1869)). The review of category roles
  ([#1833](https://github.com/GiroCrab/GiroCrab/pull/1833)) found the same for a value the
  merge copied from one row into another: the copy stayed where an earlier merge had put it.
- **Unclear standing of values the app sets.** Applying rules stamps the booking's category
  with the current time, so it beats older manual edits from other devices; an import marks
  its categories as manual. Whether such values count like a user's own edit is open
  ([#1840](https://github.com/GiroCrab/GiroCrab/issues/1840)). Upgrades that seed values face
  the same question.
- **Upgrades that compute synced values from more than one row.** Each device upgrades its own
  vault at its own time. A step that looks at other rows (for example, where a category sits
  in the tree) can compute different values on two devices for the same row.
- **Edit times from the device clock.** A device whose clock runs ahead wins every conflict
  (#1840).

## Decision drivers

1. **All devices end up with the same data.** Once every device has merged the same files,
   they hold the same data, no matter in which order the files arrived.
2. **What the user did stays.** No merge, import or upgrade silently replaces a user's edit.
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
written next to the merge step, as the booking merge does today.

*Not allowed:* a result that depends on which device's file arrived first (#1869).

### 2. The merge only stores what someone did

Fields that the merge compares by edit time hold edits. The merge does not store a value in
such a field that it worked out itself from other rows. Values like that — which row a
duplicate belongs to, a value inherited from a parent, totals — are computed when the app
reads the data, or the change carries no edit time, as when deleting a category uncategorises
its bookings.

*Why:* a stored value stays even when the rows it came from change on another device later.
Which value stays then depends on the order of syncs.

### 3. Every stored value says where it came from

A value can come from a user's edit, from an action a user started (applying rules, an
import), or from the app on its own (an upgrade, starter data). The merge rule of each field
says how these rank against each other, and the value carries enough to tell them apart (as
`category_source` does for a booking's category). A value the app set on its own never beats
one of the other two, and the merge never carries it over to another row.

*Open for comment:* how an action the user started ranks against an older edit by hand
(#1840), and whether "set by the app" is marked by edit time `0` or by a field of its own.

### 4. Rows get the same id on every device

Something that two devices create independently should get the same id on both, computed from
what it is, as for categories, tags and rules today. Matching by name is only for rows where
that does not work, such as a renamed category, and must follow rule 1 as well.

### 5. An upgrade that changes synced data reads only the row, or says what else it reads

If an upgrade step computes a value from more than the row it changes, two devices can compute
different values for the same row. Such a step says in its documentation what it reads and
what happens when devices disagree, and its tests cover that case.

### 6. A merge never mixes two schema versions

This is already how the app works and stays so: an older app does not open a vault written by
a newer one, and a newer app upgrades the other device's file before merging it.

## Not covered

- **Clocks.** Replacing the device clock with one that cannot run ahead (hybrid logical
  clocks, with the device as tie-break) is left to #1840. Rule 1 holds either way.
- **How files reach other devices** (iCloud) and when a merge runs.
- **Individual fixes.** #1869 and the other items of the tracking issue decide their own
  details.

## Alternatives considered

- **Keep fixing cases as reviews find them.** Each fix is small, but every reviewer has to
  work out the rules again, and a fix for one case can bring back another. Fails driver 4.
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
- Places that break a rule today become tracked work instead of review findings. Known so far:
  combining same-named categories cannot be undone by a later rename (#1869), and the
  category merge stores values it worked out itself (the parent after resolving moves,
  `builtin_role` of a combined category). The tests below are expected to find more.
- Every place where the app sets synced values needs a marker of where the value came from
  (rule 3).
- Re-evaluate when the tests below find problems the current design cannot fix, or when #1840
  changes how edit times are taken.

## Confirmation

- **A test per kind of synced data:** several simulated devices make random changes, then
  merge in every order; all orders must give the same data. The first such test checks
  category roles (#1833); the tracking issue extends the approach to everything that syncs.
- **For an upgrade that changes how bookings are counted:** a comparison with an independent
  copy of the old behaviour on random data.
- **A review checklist** for changes under `sync_merge/` and for upgrade steps that write
  synced data.
