# 0005 – Rules for merging device vaults

**Status** · Proposed\
**Date** · 2026-11-24\
**Discussion** · [#1870](https://github.com/GiroCrab/GiroCrab/issues/1870), [#1840](https://github.com/GiroCrab/GiroCrab/issues/1840), [#1869](https://github.com/GiroCrab/GiroCrab/issues/1869)

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

These rules apply to all synced data.

1. **Order does not matter.** The merge is a function of the synced state of the two vaults:
   sync keys and synced values, not local row ids. For any states `a`, `b`, `c`:
   `merge(a, b) = merge(b, a)`, `merge(merge(a, b), c) = merge(a, merge(b, c))` and
   `merge(a, a) = a`. Equal edit times are decided by an order on synced values or sync keys
   that the merge step documents, never by local ids or iteration order. A merge writes no
   new edit times.
2. **Stamped fields hold edits.** A value the merge computes from other rows (a survivor, an
   inherited value, a total) is not stored as an edit. It is computed when reading, or stored
   without an edit time. The same holds for cascades and cleanup after a deletion: they carry
   no edit time, and no live row is left pointing at a deleted one.
3. **Deletions are markers.** A synced row is never removed from the vault, or a peer brings it
   back. Each kind of data states its lifecycle: either a deletion is final (the latest
   `deleted_ms` wins), or the latest of creation and deletion wins (`join_life`).
4. **Coupled fields merge together.** Fields bound by an invariant (an amount and its split
   parts, `deleted_ms` and `merged_into`, a parent that must not form a cycle) share one edit
   time, or the merge repairs a violation deterministically. A peer value that breaks an
   invariant against a field it cannot change is repaired or rejected the same way on every
   device.
5. **Every value has an origin.** A synced value records whether it comes from a user's edit,
   from an action the user started (applying rules, an import), or from the app itself (an
   upgrade, starter data). Where two origins suffice, edit time `0` marks a value from the
   app; more need a marker such as `category_source`. The merge step ranks the origins for its
   field. A value from the app itself ranks lowest and is never carried to another row.
6. **Identity comes from content and then stays.** A row created independently on two devices
   gets the same sync key on both, derived from what it is when it is created. Edits do not
   change it. Matching by name is a fallback and follows rule 1.
7. **Upgrades read the row they change.** A schema step that writes a synced field reads only
   that row and its own identity chain (its parent, account or connection), never unrelated
   rows, counts or insertion order. If it has to, its documentation states what it reads and
   how the merge resolves devices that computed different values; a test covers that case.
8. **No merge across schema versions.** A build refuses a vault from a newer schema
   (`schema::check_not_too_new`) and upgrades an older one before merging it.
9. **A peer is untrusted input.** Its data passes the same validation as a local edit, in a
   way that keeps rule 1. A merge runs in one transaction. A merge that deletes a connection
   also removes its secrets and session state.

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
- Existing code that breaks a rule is listed and fixed via [#1870](https://github.com/GiroCrab/GiroCrab/issues/1870).
- Values the app writes to synced fields need an origin (rule 5).

## Confirmation

- Per kind of synced data, a randomized test: simulated devices make random edits and merge
  in every order; all orders yield the same data (rules 1–4).
- Per schema step that writes synced fields, an upgrade-and-merge test: one device upgrades
  and then edits, another edits and then upgrades; merged in both orders they hold the same
  data and the figures from before the upgrade (rules 7 and 8, driver 3).
- A review checklist for changes under `sync_merge/` and for schema steps that write synced
  fields.
