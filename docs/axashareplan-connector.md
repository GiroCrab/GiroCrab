# AXA Mitarbeiter Shareplan browser connector

<!-- sync:connector-note:start -->
> **Hinweis / Note:** Dieser Connector ist ein unabhängiges Community-Projekt und steht in keiner Verbindung zur AXA. Schnittstelle: inoffiziell. Bei inoffiziellen Schnittstellen kann die Nutzung gegen die Nutzungsbedingungen der Bank verstoßen.
> This connector is an independent community project and is not affiliated with AXA. Interface: unofficial. Using unofficial interfaces may violate the bank's terms of service.
<!-- sync:connector-note:end -->

The employee savings plan portal of AXA (`espace-salarie.axa-mon-epargne-entreprises.com`) shows
the funds of the share plan (PEEG): the shares an employee bought in each year's plan, with their value
and gain. Sign in through **Add bank → AXA Mitarbeiter Shareplan**. The login (credentials and an
SMS code) happens in the embedded browser. After login, choose **Bestand abrufen**.

## What is imported

- One securities account ("Mitarbeiter Shareplan") with **one position per fund** the portal
  lists for the plan (for example one per plan year): name, identifier, number of shares,
  average purchase price, current price, invested amount and value.
- Profit and loss are derived from the invested amount and the value.
- The account balance is the sum of the position values.
- Only EUR is accepted; anything else is rejected rather than guessed.
- Positions with no shares left (sold or transferred to a depot) are not imported; the next
  fetch removes them from the depot.
- Documents and transactions are not imported.

## Browser flow and technical validation

After sign-in the portal's own page script talks to its data interface under
`/<version>/api/b2b/`. The connector's page script remembers the headers the portal sends there
(in a JavaScript closure that never reaches Swift, logs or the vault; the sign-in requests under
`/api/auth/` are never read) and then sends two read queries with fixed bodies from the signed-in
page: the portfolio of type `mutual-fund` (`/profile/portfolio`) and the names and identifiers of
its instruments (`/instruments`). The portal uses POST for these filters; nothing is changed.

Swift receives only fund id, name, identifier, number of shares, prices and amounts. Names,
e-mail addresses, phone numbers, account and contract numbers are not requested or exported.
The generic portal traffic recorder is disabled for this connector.

The portal reports amounts as whole cents. Consistency checks before anything is stored: ids
unique, the identifier shaped like an ISIN (the funds use internal identifiers whose check
digit is not the ISO one), value = shares × price and invested = shares × average price (within
the cent rounding of the price). A failed check imports nothing.

## Limits

- Only the plan type **PEEG** (funds of type `mutual-fund`) is recognized and imported; this is
  what the connector was built and tested with. Other plan types or instrument types (for
  example directly held shares after a transfer) are not imported. They can be requested with
  an issue and, with sample data, be added.
- More than 499 positions are rejected rather than cut off.
- The embedded browser is needed on every fetch (SMS code); there is no background sync.
- The portal is not documented and can change without notice (the interface path carries its
  release number).
