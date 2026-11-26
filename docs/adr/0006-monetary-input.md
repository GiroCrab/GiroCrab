# 0006 – Localized monetary input and exact amounts

**Status** · Proposed\
**Date** · 2026-11-25\
**Discussion** · [#1937](https://github.com/GiroCrab/GiroCrab/issues/1937), [#1761](https://github.com/GiroCrab/GiroCrab/pull/1761)

## Context

Monetary input crosses a boundary between text and exact amounts in every
frontend. Each frontend reads amount text with rules of its own, so the same
input can give different amounts, and the difference passes validation because
every result is a valid integer. An amount the app already knows can also change
when a form formats it and reads it back. [#1937](https://github.com/GiroCrab/GiroCrab/issues/1937) · [#1761](https://github.com/GiroCrab/GiroCrab/pull/1761#discussion_r4697292648)

In free text, `.` and `,` can each be a decimal or a grouping separator.
Guessing from the digits misreads a three-digit fraction by a factor of 1000.
The regional settings do not settle which key the user presses either. The iOS
decimal pad shows the separator of the active keyboard, not of the region, and
Android's on-screen keyboards are not bound by a field's key filter. A field
that accepts only the region's separator can leave a user without a decimal
key; a field that treats the other one as grouping turns `15.56` into
1556,00 €. [iOS decimal pad](https://developer.apple.com/forums/thread/107269) · [Android key filter](https://developer.android.com/reference/android/text/method/DigitsKeyListener)

## Decision drivers

1. A monetary edit has the same meaning across frontends using the same field
   rules. The amount a user approves is the amount submitted.
2. An amount from storage, an import or a calculation keeps its exact value when
   it is shown in a form.
3. Users can enter an amount with the decimal key their keyboard offers and read
   it in their regional format. Typed input the field cannot read is shown as an
   error, never dropped or silently reinterpreted. Pasted text is read as the
   user's own format reads it. An empty field, explicit zero and invalid input
   have distinct meanings.
4. Each form can adopt the contract independently. File and bank formats follow
   the rules of their own input sources.

## Decision

1. **Presentation.** A field shows its amount in the user's regional format,
   with grouping, while it does not have focus. While the user edits it, the
   field shows the amount without grouping (`1234,56`) and keeps the text the
   user typed. Grouping returns when focus leaves the field. A prefill from a
   saved value, a preset, an import or a calculation carries its exact amount in
   form state. The field submits that amount as long as its editing text equals
   the text formatted from it; once the text differs, the text is parsed.
2. **Typing.** The user never types grouping. A field accepts digits, `.` and
   `,`; the frontend refuses other keystrokes. Both `.` and `,` are the decimal
   separator, and it may stand at either end: `,5` is `50` cents and `5,` is
   `500` cents. A second separator or more decimals than the field's scale
   shows an error and blocks the write. The field never drops or moves a digit
   or separator it accepted. Where the field rules allow negative amounts, a
   sign `-` or `−` (U+2212, which the display formatters write) may precede
   the number. A percentage field also accepts a trailing `%`, with or without
   a space before it; its decimals are not limited, and the split calculation
   rounds the share. This rule does not depend on the regional settings:
   `12.50` and `12,50` are `1250` cents everywhere, and a German `1.500` is an
   error with too many decimals.
3. **Pasting.** Pasted text loses Unicode whitespace, including no-break
   spaces, and the symbol or ISO code of the field's currency. The rest takes
   the sign rule of typing and is first read strictly in the field's own
   format: the first group has one to three digits, later groups two or three,
   the last group before the decimal separator exactly three, and the decimals
   stay within the field's scale. `1.234,56` under `de-DE` is `123456` cents,
   and `1,00,000.00` under `en-IN` is read. A three-digit group is thousands as
   the own format reads it, so `12.500` under `de-DE` is `1250000` cents. Text
   the own format cannot read gets the typing rule, so `12.50` under `de-DE` is
   `1250` cents. Anything else is rejected with a message and leaves the field
   unchanged. A pasted amount appears in the field's editing form, so a pasted
   `12.500` shows as `12500` before anything is saved.
4. **Shared parser.** One pure Rust function implements rules 2 and 3 for every
   frontend, so an editor can validate text without accessing the vault. The
   frontend passes the decimal and grouping separator of the formatter it
   displays amounts with, so the core carries no locale data. The field rules
   specify sign, scale, percentages and any rounding. The function returns an
   exact value, an empty result or a validation error naming the problem.
   Invalid input blocks the write. Clearing an optional value requires the user
   to empty the field or use a dedicated clear action.
5. **Calculations and writes.** Business operations consume exact,
   locale-independent integer cents; the currency comes from the surrounding
   record or operation. A bridge that cannot carry the full integer range uses
   a lossless representation, such as an ASCII integer string. A payment review
   and submission use the same exact draft value, and an edit requires a fresh
   review. The core enforces its business rules at acceptance or save time.

The decision covers every monetary input field on macOS, iOS, Linux, Windows
and Android. Percentage shares carry an exact decimal or ratio into the split
calculation, which applies the rounding rule. Bank responses, file imports,
GiroCodes and other structured inputs are interpreted according to their source
format and hand exact values to the form or operation.

## Not covered

- Currencies whose minor unit is not a hundredth. Amount fields have a scale of
  two decimals, as for every amount the core stores in cents.
- Calculator expressions in amount fields.

## Alternatives considered

- **Guess grouping from the digits.** The core's text parser reads a separator
  followed by three digits as thousands; a variant could do so only for the
  field's own grouping separator. A German `1.500` would stay 1.500 €, but a
  `12.500` typed by habit from another format becomes `1250000` cents without
  notice (drivers 1 and 3). Asking the user in that case needs a choice in each
  frontend's field, and a German user who types thousands separators is better
  served by a visible error.
- **Accept only the region's decimal separator.** The platform keyboard may not
  offer it, which leaves the user without a decimal key (driver 3).
- **Fixed-decimal entry.** Digits shift in from the right, and no separator is
  typed. This needs a separate rule for pasted amounts and does not fit
  percentages in the split field. Editing inside the number and screen reader
  output become harder.
- **Group digits while typing.** Each toolkit would need exact cursor mapping
  as text is inserted and removed, and screen readers announce characters that
  the formatting then moves. Grouping on focus loss avoids both.
- **Pass a locale identifier and resolve it in the core.** The core would need
  locale data, such as CLDR through a new crate, with its license notice and
  binary size on every platform. Its separators could also disagree with the
  frontend formatter that wrote the prefill (driver 2).
- **Repair each form separately.** A form can emit text its current parser
  accepts. Applying that approach across the app leaves duplicated parsing rules
  and keeps the possibility of changing an already exact amount during prefill.
- **Parse with native locale APIs in each frontend.** The same rules would be
  implemented and tested on each platform (driver 1). Locale parsers reject or
  regroup the other separator, as Qt does for `12.50` under `de-DE`.
- **Change the existing Rust text parser in place.** Connectors, file importers
  and search also read amounts with it. A new interpretation of an accepted
  string could change imported amounts or search results (driver 4). A separate
  form parser lets each migration verify its own accepted inputs.

## Consequences

Forms migrate one at a time; the order is planned in the discussion issue. The
end state is one input contract for all monetary forms. Each migration costs a
field-level compatibility review, especially for previously accepted input and
fields that intentionally round.

Each frontend filters keystrokes, switches between editing and display text on
focus, and passes its two separators to the parser. Typed thousands separators
stop working: a typed `1.500` is an error instead of 1.500 €. Pasted text in
another format with a three-digit fraction, such as an English `12.500` under
`de-DE`, is read in the field's own format; the field shows the result before
anything is saved. Revisit the typing rule if users report the `1.500` error
often.

## Confirmation

- A core test matrix pins the cents or the error for each case.
  - Typed, under any format: `12,50` and `12.50` are `1250`; `,5` is `50` and
    `5,` is `500`; `−12,50` and `-12.50` are `-1250` where negatives are
    allowed and an error elsewhere; `1.500` and `12,500` have too many
    decimals; `1.234,56` has a second separator. In a percentage field,
    `33,3 %` and `33.3%` are 33.3 percent and `−10 %` is minus ten percent.
  - Pasted under `de-DE`: `1.234,56` is `123456`, `12.50` is `1250`, `1.500`
    is `150000`, `12.500` is `1250000`; `12,500` and `1,234.56` are rejected.
  - Pasted under `en-US`: `1,234.56` is `123456`, `1,500` is `150000`; `1.500`
    is rejected. Under `en-IN`: `1,00,000.00` is `10000000`. Under `fr-FR`,
    `1 234,56` with a narrow no-break space is `123456`.
- For each migrated form, verify that prefills and unrelated edits preserve
  exact values, including an `en-IN` prefill of `10000000` cents through focus
  and blur. Invalid input cannot clear or zero an amount, and reviewed payments
  submit the same cents. Test the signed range where supported and bridge
  precision where the frontend uses JavaScript numbers.
