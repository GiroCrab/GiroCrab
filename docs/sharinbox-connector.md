# Sharinbox browser connector (Société Générale employee shares)

<!-- sync:connector-note:start -->
> **Hinweis / Note:** Dieser Connector ist ein unabhängiges Community-Projekt und steht in keiner Verbindung zur Société Générale. Schnittstelle: inoffiziell. Bei inoffiziellen Schnittstellen kann die Nutzung gegen die Nutzungsbedingungen der Bank verstoßen.
> This connector is an independent community project and is not affiliated with Société Générale. Interface: unofficial. Using unofficial interfaces may violate the bank's terms of service.
<!-- sync:connector-note:end -->

Employee share plans administered by Société Générale Securities Services are shown on
the **Sharinbox** portal. Sign in through **Add bank → Sharinbox (Société Générale)**. The login
(SG Connect: e-mail, password and authenticator app) happens in the embedded browser. After
login, choose **Bestand abrufen**.

The connector reads the **number of shares and their value** from the portal's start page and,
optionally, the **PDF documents** of the document list ("Meine Dokumente").

## What is imported

- One securities account ("Mitarbeiteraktien") with one position per security the start page
  lists: ISIN, name, quantity, price with its date, and value.
- Quantity and value of the securities are the available and the blocked balance added up.
- Share rights (Aktienrechte), i.e. claims to shares that are allotted later, appear as their own
  position "<security name> (noch nicht zugeteilt)" (with several securities: "Aktienrechte (noch
  nicht zugeteilt)") with the number the start page shows. The
  portal gives no ISIN or price for them. If the page lists exactly one security, the rights are
  valued at its price (assumption: one right becomes one share) and count towards the account
  total; with several securities they cannot be assigned and carry no value. Without such a
  section on the page, no position is created.
- The switch "Nicht zugeteilte Aktienrechte abrufen" (browser window and connection settings,
  default on) is stored with the connection. Switching it off removes the position at once and
  reduces the account total by its value; later fetches leave the rights out.
- The portal shows no purchase price, so no profit or loss is derived.
- Only EUR is accepted; anything else is rejected rather than guessed.
- Documents: every entry of the portal's document list (statements, transaction notices,
  purchase letters and also the general plan rules, flyers and newsletters). A PDF is stored once
  per document at the securities account; a second fetch loads only new ones. The switch
  "PDF-Postbox abrufen" in the browser window and "Dokumente abrufen" of the connection turn
  this off.

## Browser flow and technical validation

The portal signs in through SG Connect (OAuth 2.0) and then serves the start page
(`/SharinBox/connexion.do`) as HTML in the signed-in session. The page script reads that page
with a same-origin request and extracts the selected fields. No token or cookie reaches
Swift, logs or the vault; Swift receives only ISIN, name, quantity, price, price date,
currency and value. Names, e-mail addresses, account and holder numbers are not exported.
The generic portal traffic recorder is disabled for this connector.

Consistency checks before anything is stored: ISINs must be well formed and unique, the
price date a valid date, and each value must match quantity × price (the page shows the price
with three decimals). A failed check imports nothing.

Documents: the page script reads the list page (`/SharinBox/documentationAcces.do`): title, date and
document ID per entry. The ID must be a UUID because it is part of the download address
(`/SharinBox/sgdocs/download.do?idSgdocs=<ID>`); a file counts only if it starts with `%PDF-`.
Whether opening a document marks it as read in the portal has not been verified.

## Limits

- The page is parsed from its HTML. A change of the portal's page layout stops the fetch with a
  message; nothing is imported then.
- The embedded browser is needed on every fetch (authenticator app); there is no background
  sync.
- The portal is not documented and can change without notice.
