# 0006 – Localized monetary input and exact amounts

**Status** · Proposed\
**Date** · 2026-11-25\
**Discussion** · [#1937](https://github.com/GiroCrab/GiroCrab/issues/1937), [#1761](https://github.com/GiroCrab/GiroCrab/pull/1761)

## Context

With German regional settings, typing or pasting `12.50` into a transfer field
yields 1,250 cents through the Rust parser used on macOS and Linux. Android's
transfer form removes the period and constructs a draft for 125,000 cents. An
amount difference of this kind can pass validation because both results are
valid integers. [Issue #1937](https://github.com/GiroCrab/GiroCrab/issues/1937)

An amount the app already knows can also change when it takes a detour through
text. The split editor can calculate 10,000,000 cents from a preset, format it
as `1,00,000.00` under English (India), and disable Save when its parser rejects
the generated grouping. A mandate draft can turn a parse failure into a zero
default while saving an unrelated edit. These are different failures at the
same boundary between display text and amounts. [Split review](https://github.com/GiroCrab/GiroCrab/pull/1761#discussion_r4697292648) · [Mandate draft](../../GiroCrabApp/Sources/GiroCrabUI/DirectDebitTypes.swift)

## Decision drivers

1. A monetary edit has the same meaning across frontends using the same field
   rules and locale. The amount a user approves is the amount submitted.
2. An amount from storage, an import or a calculation keeps its exact value when
   it is shown in a form.
3. Users can enter and read amounts in the regional format of the field. An empty
   field, explicit zero and invalid input have distinct meanings.
4. Each form can adopt the contract independently. File and bank formats follow
   the rules of their own input sources.

## Proposed decision

1. **Presentation.** Frontends format values for the user's regional settings
   and keep the text the user is editing. A prefill from a saved value, a
   preset, an import or a calculation carries its exact amount in form state.
   Formatting that amount for display leaves the value available to the form.
2. **Input parsing.** A shared Rust parsing API accepts user-entered or pasted
   amount text with an explicit locale and field rules. It returns an exact
   value, an empty result, or a validation error. The field rules specify the
   accepted syntax, sign, scale and any rounding. If allowed interpretations
   would give different amounts, the field asks the user to clarify the input.
   Invalid input blocks the write. Clearing an optional value requires the user
   to empty the field or use a dedicated clear action. The parser is a pure
   function, so an editor can validate text without accessing the vault.
3. **Calculations and writes.** Business operations consume exact,
   locale-independent values. Bookings and payments already use integer cents,
   with currency supplied by the surrounding record or operation. A bridge
   that cannot carry the full integer range uses a lossless representation,
   such as an ASCII integer string. A payment review and submission use the same
   exact draft value, and an edit requires a fresh review. The core enforces its
   business rules at acceptance or save time. [Transfer and split input types](../../girocrab-core/src/models.rs)

The proposal covers every monetary input field on macOS, iOS, Linux, Windows
and Android. Percentage shares carry an exact decimal or ratio into the split
calculation, which applies the rounding rule. Bank responses, file imports,
GiroCodes and other structured inputs are interpreted according to their source
format and hand exact values to the form or operation. [Existing money parsers](../../girocrab-core/src/money.rs)

## Alternatives considered

- **Repair each form separately.** A form can emit text its current parser
  accepts. Applying that approach across the app leaves duplicated parsing rules
  and keeps the possibility of changing an already exact amount during prefill.
- **Parse with native locale APIs in each frontend.** This uses platform locale
  support and requires the same compatibility, rounding and error policy to be
  implemented and tested on each platform.
- **Change the existing Rust parser in place.** Its callers include connectors,
  CSV importers and search. A new interpretation of an accepted string could
  change imported amounts or search results. An opt-in form parser lets each
  migration verify its own accepted inputs. [Batch CSV example](../../girocrab-core/src/batch_csv.rs) · [Search example](../../girocrab-core/src/search_expr.rs)

## Consequences and migration

Start with split and budget forms to exercise exact prefills, localized editing
and shared validation. Prioritize transfers, standing orders and direct debits
once tests cover the amount shown at review and the value submitted or stored.
Then migrate the remaining money fields, including opening balances, manual
bookings, rules and assets. The end state is one input contract for all monetary
forms. Each migration costs a field-level compatibility review, especially for
previously accepted input and fields that intentionally round.

The draft leaves two implementation questions for discussion. Which alternate
formats should a field accept when text is pasted from another locale? Which
locale data and API shape should the shared parser use?

## Confirmation

- Start with a shared `de-DE` test matrix for `12,50`, `12.50`, `1.250,00`
  and ambiguous input such as `1,500`. Pin the accepted cents or validation
  error for each case. Include an `en-IN` grouped prefill to test that an exact
  value survives presentation.
- For each migrated form, verify that prefills and unrelated edits preserve
  exact values, invalid input cannot clear or zero an amount, and reviewed
  payments submit the same cents. Test the signed range where supported and
  bridge precision where the frontend uses JavaScript numbers.
