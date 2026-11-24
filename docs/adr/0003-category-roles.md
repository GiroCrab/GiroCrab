# 0003 – Category roles

**Status** · Proposed\
**Date** · 2026-11-24\
**Discussion** · [#931](https://github.com/GiroCrab/GiroCrab/issues/931), [#1662](https://github.com/GiroCrab/GiroCrab/issues/1662), [#1833](https://github.com/GiroCrab/GiroCrab/pull/1833)

## Context

This ADR accompanies #1833. Where it describes current behaviour, it means the code after that
PR, so it is merged together with or after #1833.

The core classifies a booking (`FlowKind`) when it reads it, in `classify_by_kind`
([`merchants.rs`](../../girocrab-core/src/merchants.rs)) and `BookingFinisher`
([`vault/bookings.rs`](../../girocrab-core/src/vault/bookings.rs)), from several signals: text
keywords, bundled merchant kinds, account kinds, own IBANs, payment-service pairing, salary
marks, loan links and the category. Before #1833 the category contributed the `builtin_role`
of its top-level ancestor (#678), plus whether any ancestor was a transfer category. That
leaves gaps:

1. **Custom categories have no meaning.** A user cannot mark a category of their own as neutral
   (outlays, deposits, money passed on), investment or loan.
2. **Tree position decides.** A built-in investment or loan category moved under a custom
   folder loses its classification, because only the top-level ancestor counts.
3. **Front ends match names.** [`BookingDetailView.swift`](../../GiroCrabApp/Sources/GiroCrabUI/BookingDetailView.swift)
   and [`InspectorView.qml`](../../GiroCrabLinux/qml/InspectorView.qml) compare the category
   with the string `"Umbuchungen"`.

#931 plans further steps on top of this model (a role editor, transfer pairing between own
accounts, roles that beat other heuristics), and #1662 classifies the parts of split bookings.
They are decided there, not here.

## Decision drivers

1. **Meaning is explicit.** It does not depend on a category's name or its position in the
   tree, and it survives renaming and moving.
2. **Upgrading keeps the numbers.** Existing vaults show the same KPIs and cash flows after the
   migration; every exception is deliberate and listed below.
3. **A user's choice is kept.** Including the choice to inherit again, it survives iCloud sync
   and repeated imports.
4. **One contract.** The core resolves a category's role; macOS/iOS, Linux and Windows read it
   and do not re-implement it.

## Decision

### Every category has a financial role

[`CategoryRole`](../../girocrab-core/src/models.rs):

| Role | Meaning | Effect on `FlowKind` |
| --- | --- | --- |
| `Consumption` (default) | everyday spending | none of its own |
| `Income` | income | none yet (#931) |
| `Salary` | salary, pension | none yet (#931) |
| `Investment` | saving and investing | `Investment` |
| `Loan` | loans and debt | `Loan` |
| `Neutral` | neither income nor expense | `Transfer` |

The role is a signal in `classify_by_kind` that replaces the category names and `builtin_role`
checks there. How it ranks against merchant, account-kind and text signals stays as it was;
changing that is a later step of #931.

### Stored per category, inherited down the tree

- `categories.role` holds an explicit role; `NULL` means "inherit". A category's
  `effective_role` is the role of the nearest category on its way to the root that has one,
  else `Consumption`.
- [`Category`](../../girocrab-core/src/models.rs) carries both `role` and `effective_role`.
  Front ends read `effective_role` and never compare category names.

### `Neutral` belongs to the category, transfers to the booking

`Neutral` says that the bookings of a category do not enter the income and expense figures.
It replaces the special case `"Umbuchungen"`: the built-in category, imported transfer
categories (`import:transfers`) and imported categories hidden from charts
(`showInDiagrams = 0`) become `Neutral`. Recognising that two bookings are one transfer
between own accounts stays a property of the bookings, because only a booking has a
counterpart booking.

### Who set a role, and which one wins

- `categories.role_ms` stamps the last change of the role, like the other `*_ms` columns.
  `Vault::set_category_role` stamps it; `role = NULL` with `role_ms > 0` is an explicit
  "inherit", `role_ms = 0` a role nobody chose.
- The iCloud merge ([`sync_merge/categories.rs`](../../girocrab-core/src/sync_merge/categories.rs))
  takes `role` and `role_ms` together: the newer change wins.
- The migration and the importers set a role only where nobody chose one
  (`role IS NULL AND role_ms = 0`), and keep stamp `0`: what the core sets itself is no choice,
  so any user choice wins over it on sync.

### Existing vaults keep their classification

Before, only the `builtin_role` of a booking's top-level category counted, plus whether any
category on its way to the root was a transfer category. The update gives a category a role
only where its `builtin_role` decided the classification this way:

- Built-in categories at the top level, or below a deleted category, start with the role
  their `builtin_role` stood for.
- `Umbuchungen` and imported transfer categories start as `Neutral` at any depth.
- `Gehalt` starts as `Salary` wherever it is, except below a category that starts as
  `Investment`, `Loan` or `Neutral`.
- `ETF-Sparpläne` and `Aktien` already moved to the top level start as `Investment`: before,
  a keyword check on the top-level `builtin_role`, meant for imported category paths, matched
  their keys.

Every other category starts without a role and inherits from its parent. A built-in category
moved elsewhere thus keeps the classification its position gave it; `Gehalt` under
`Umbuchungen` stays `Neutral`, under `Sparen & Investieren` `Investment`. These roles are ones nobody chose (stamp `0`), so any user
choice wins over them. From then on only roles count: a moved category keeps its explicit
role, and one without inherits from its new parent. The schema step `category_roles` in
[`schema.rs`](../../girocrab-core/src/schema.rs) implements this.

## Not covered

- How a role ranks against merchant, account-kind and text signals, and the effects of
  `Income` and `Salary` (later steps of #931).
- Transfer pairing between own accounts (`transfer_flag`, `transfer_group`; #931).
- Split bookings: one category per part and its role per part (#1662).
- The role editor in the front ends.
- The local API: its `categories` method does not return the role yet.
- The general rules for sync stamps (`*_ms`), such as whether an import counts as a user
  choice (#1840). This ADR only applies them to `role_ms`.

## Alternatives considered

- **Keep `builtin_role` and name checks.** Custom categories cannot have a meaning, moving
  built-ins breaks their classification, front ends match German names (drivers 1 and 4).
- **A role only on top-level categories.** Close to the state before #1833: a sub-category
  cannot differ from its parent (an outlay category under "Freizeit"), and a moved built-in
  still loses its meaning (driver 1).
- **Copy the role to every category instead of inheriting.** Changing a parent's role would not
  reach its children, and a copied role could not be told apart from a chosen one (driver 3).
- **Every category starts as `Consumption`.** Simpler, but investments, loans and transfers of
  existing vaults would count as spending right after the update (driver 2).
- **Every built-in category starts with its role, wherever it is.** Gives moved built-ins
  their meaning back (driver 1), but changes the numbers of rearranged vaults on the update:
  a moved `Sparen & Investieren` would turn spending into investments, `Gehalt` under
  `Umbuchungen` transfers into income (driver 2).
- **A per-booking transfer flag instead of `Neutral`.** Cannot exclude whole categories
  (deposits, outlays, imported hidden categories) and keeps `"Umbuchungen"` as a special name
  (#931).

## Consequences

- `Category` gains `role` and `effective_role` across UniFFI and serde JSON
  (`--features serde`).
- Built-in categories moved before the update keep their old, position-dependent
  classification until the user sets a role: `Sparen & Investieren` under a custom folder
  stays `Consumption`. Here driver 2 wins over driver 1.
- Deliberate exceptions to driver 2:
  - The next database import turns categories hidden from charts there into `Neutral`.
  - An import that writes into a built-in top-level category keeps its role. Before, taking
    over the category's name also removed its classification.
  - Moving `ETF-Sparpläne` or `Aktien` to the top level after the update gives `Consumption`
    until the user sets a role; before, the keyword check made them investments.
- `Income` and `Salary` are stored, inherited and synced but change nothing yet. A role editor
  hides them or ships together with their effect.
- Re-evaluate this ADR when roles are to beat the other heuristics, when split parts are
  classified per part (#1662), or when a second axis beyond direction and role is needed.

## Confirmation

- `category_roles_backfill_from_builtin_role` in [`schema.rs`](../../girocrab-core/src/schema.rs)
  and the fixture vault of the new step (`cargo test -p girocrab-core --test schema_fixtures`).
- [`role_tests.rs`](../../girocrab-core/src/vault/categories/role_tests.rs):
  inheritance, sync merge (including a role edited on a category another device combined
  into a duplicate), moved built-ins, rearranged trees of the previous schema version, and
  the comparison with an independent copy of the old classification (KPIs and cash flow) on
  a demo vault and on the fixture of the previous schema version.
- Importer tests: `showInDiagrams = 0` becomes `Neutral`; an explicit "inherit" survives a
  re-import.
- [`scripts/lint-serde-feature.sh`](../../scripts/lint-serde-feature.sh) (CI) for the new fields.
