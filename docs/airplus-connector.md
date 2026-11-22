# AirPlus browser connector

<!-- sync:connector-note:start -->
> **Hinweis / Note:** Dieser Connector ist ein unabhängiges Community-Projekt und steht in keiner Verbindung zur AirPlus. Schnittstelle: inoffiziell. Bei inoffiziellen Schnittstellen kann die Nutzung gegen die Nutzungsbedingungen der Bank verstoßen.
> This connector is an independent community project and is not affiliated with AirPlus. Interface: unofficial. Using unofficial interfaces may violate the bank's terms of service.
<!-- sync:connector-note:end -->

Design page for `AirPlusUI.swift` and `girocrab-core/src/connectors/airplus.rs` (#1143).

Sign in through **Add bank → AirPlus Corporate Card**. Login (OpenID Connect) and the SMS TAN
happen in the embedded browser. Then open the booking overview of your card in the portal and
choose **Umsätze abrufen**.

## What is imported

- **Booked card bookings** (booking date, sales date, merchant, city, amount, currency, booking
  type). The portal's per-booking id (`AuthorizationNo`) is the de-duplication key, so repeated
  fetches never create duplicates.
- **Used credit limit** as the card's (negative) balance. If it cannot be read, the balance stays
  unknown instead of 0.
- Not imported yet: pending authorizations, statement PDFs, payments to the card.

## How it works

The portal is an Angular application that posts JSON to `/call/directive/…` and `/services/…`.
The page script (`AirPlusScripts.hook`) remembers the portal's own request for the card's booking
list (`GetAccountCardTransactionListTransaction`: URL, request headers, body) in a JavaScript
closure and replays it page by page (50 per page) with XMLHttpRequest, like the portal itself. Only whitelisted booking fields and two card
values (id, used limit) cross the bridge to the core; cookies, the request headers and personal
data (name, employer, birth date) stay in the page. The generic portal traffic recorder is off.

Interface: unofficial and undocumented. No FinTS, PSD2 or Enable Banking access to AirPlus cards
is known. The portal can change without notice.

## Open points (to verify with more real accounts)

- Sign of refunds and booking of foreign-currency amounts (only charges in EUR were observed).
- Meaning of the used limit for billed but unpaid amounts.
- How long a session lasts and how often a TAN is needed.
- The time of day in `AuthorizationDate` is always `00:00`; only the date is used.
- The terms of use of AirPlus may restrict automated access.

## Tests

`cargo test -q -p girocrab-core --lib airplus` (mapping, idempotent import, rejection of broken
responses) and `swift test -q --filter AirPlusTests` (page script in JavaScriptCore with invented
data). Fixtures contain only invented merchants, ids and amounts.
