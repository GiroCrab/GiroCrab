# Berghaus & Cie browser connector (wine depot)

<!-- sync:connector-note:start -->
> **Hinweis / Note:** Dieser Connector ist ein unabhängiges Community-Projekt und steht in keiner Verbindung zur Berghaus & Cie. Schnittstelle: inoffiziell. Bei inoffiziellen Schnittstellen kann die Nutzung gegen die Nutzungsbedingungen der Bank verstoßen.
> This connector is an independent community project and is not affiliated with Berghaus & Cie. Interface: unofficial. Using unofficial interfaces may violate the bank's terms of service.
<!-- sync:connector-note:end -->

The wine investment portal of Berghaus & Cie shows the bottles a customer holds in "Mein
Weindepot". Sign in through **Add bank → Berghaus & Cie (Weindepot)**. The login (e-mail and password)
happens in the embedded browser. After login, choose **Bestand abrufen**.

## What is imported

- One securities account ("Weindepot") with one position per portal entry: wine name (with
  vintage, and the bottle size if it is not 0.75 l), LWIN as WKN, number of bottles, purchase
  price per bottle and the current valuation per bottle.
- The portal counts **cases**: `amount` cases of `caskOf` bottles each. GiroCrab imports the
  bottles (cases × bottles per case). The purchase price is per bottle.
- The valuation is the latest "Winesearcher Europe average" value of the entry. It is the
  price of a 0.75 l bottle; a larger bottle is scaled by its volume (a 1.5 l bottle counts twice).
- The account balance is the sum of the position values; profit and loss compare the value
  with the purchase price. This reproduces the two totals at the top of "Mein Weindepot"
  ("Insgesamt investiert", "Depot-Wert").
- Entries whose latest status says sold, delivered, withdrawn, cancelled or returned are left
  out. Which status values the portal uses beyond "PURCHASED" is not documented; an unknown
  status counts as held.
- There are no documents in this portal, so no mailbox is read.
- Only EUR is accepted by design (the portal prices in EUR).

## Browser flow and technical validation

The portal is a single-page application that talks to a GraphQL service
(`bc-cms.herokuapp.com/graphql`). The page script remembers the authorization the portal itself
sends, in a closure of the page; it never reaches Swift, logs or the vault. The sign-in request
(with the password) is neither read nor replayed. The script then asks the service for the wine
depot only, with a fixed query that contains no names, addresses, invoices or images. Swift
receives only: position id, LWIN, wine name, bottles, prices, valuation date and value.
The generic portal traffic recorder is disabled for this connector.

Consistency checks before anything is stored: unique numeric ids, a valid LWIN (or none), whole
bottles, valid dates, and each value must match bottles × price. A position the script cannot
read (missing valuation, odd quantity) stops the fetch; a partly understood depot would
otherwise remove the positions left out. A failed check imports nothing.

## Limits

- The portal is not documented and can change without notice (GraphQL fields, status names).
- A position without a Winesearcher valuation stops the fetch with a message instead of being
  priced with a guess.
- The embedded browser is needed on every fetch; there is no background sync.
