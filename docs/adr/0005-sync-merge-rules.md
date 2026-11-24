# 0005 – Rules for merging device vaults

**Status** · Proposed\
**Date** · 2026-11-24\
**Discussion** · TRACKING_ISSUE, [#1840](https://github.com/GiroCrab/GiroCrab/issues/1840), [#1869](https://github.com/GiroCrab/GiroCrab/issues/1869)

## Context

Each device keeps its own vault and merges the vault files of other devices into it
(`Vault::merge_other_vault`, [`sync_merge.rs`](../../girocrab-core/src/sync_merge.rs)). There
is no server; every device merges on its own, in whatever order files arrive. Synced fields
carry the time of their last edit (`*_ms`), deletions stay as markers (`deleted_ms`), and rows
are matched by sync keys.

Each merge step documents its own rules, but no common guarantees are written down. Whether
a result may depend on the order of syncs, or how a value the app set compares to a user's
edit, is decided anew in each step and each review (#1840, #1869).

## Decision drivers

1. All devices hold the same data once they have merged the same files.
2. No merge, import or upgrade silently replaces a user's edit.
3. An upgrade does not change an existing vault's figures, also not after the next sync.
4. Merge changes can be checked against written rules and tests.

## Decision

These rules apply to every synced field.

1. **Order does not matter.** For any vault states `a`, `b`, `c`:
   `merge(a, b) = merge(b, a)`, `merge(merge(a, b), c) = merge(a, merge(b, c))` and
   `merge(a, a) = a`. When two values have the same edit time, an order on the values decides;
   the merge step documents it.
2. **Stamped fields hold edits.** A value the merge computes from other rows (a survivor, an
   inherited value, a total) is not stored as an edit. It is computed when reading, or
   stored without an edit time.
3. **Every value has an origin.** A stamped value records whether it comes from a user's
   edit, from an action the user started (applying rules, an import), or from the app itself
   (an upgrade, starter data). The merge step ranks these origins for its field. A value from
   the app itself ranks lowest and is never carried to another row.
4. **Identity comes from content.** A row created independently on two devices gets the same
   sync key on both, derived from what it is. Matching by name is a fallback and follows
   rule 1.
5. **Upgrades read the row they change.** A schema step that writes a synced field reads only
   that row. If it has to read other rows, its documentation states which, and how the merge
   resolves devices that computed different values; a test covers that case.
6. **No merge across schema versions.** A build refuses a vault from a newer schema
   (`schema::check_not_too_new`) and upgrades an older one before merging it.

## Not covered

- How edit times are taken (device clock or a logical clock): #1840.
- How vault files reach other devices and when a merge runs.

## Alternatives considered

- **Decide per merge step and per review.** Leaves driver 4 unmet; a fix for one case can
  break another.
- **A CRDT library or SQLite extension.** Gives rule 1 by construction, but replaces every
  merge step, the file-based sync of encrypted vaults and the upgrade path of existing vaults
  at once. Revisit if rule 1 cannot be kept with the current design.
- **Sync an operation log instead of states.** Needs a log and its compaction in every vault;
  larger than the known problems justify.

## Consequences

- Merge changes are reviewed against these rules; a deliberate exception is documented at
  the code that makes it.
- Existing code that breaks a rule is listed and fixed via TRACKING_ISSUE.
- Rule 3 needs an origin marker wherever the app writes synced values.

## Confirmation

- Per kind of synced data, a randomized test: simulated devices make random edits and merge
  in every order; all orders yield the same data (rule 1).
- A review checklist for changes under `sync_merge/` and for schema steps that write synced
  fields.
