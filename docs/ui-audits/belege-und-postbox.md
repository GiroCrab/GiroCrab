# UI-Audit: Belege & Postbox

- **Ansicht:** Dokumentliste mit Filtern und Vorschau auf macOS und iPad sowie Vorschau-Sheet am iPhone.
- **Dateien:** `DocumentsUI.swift` und `AccountDocumentsUI.swift` in `GiroCrabUI/`.
- **Werkzeug:** Native Impeccable `critique` und `audit` anhand von SwiftUI-Code und Demo-Aufnahmen, dazu GiroCrab-Checkliste.
- **Nachweis:** macOS 27.2.1, iPhone 18 Pro und Pro Max sowie iPad mini (iOS 27.2), Deutsch. Standard, große Schrift und geschwärzte Varianten. iOS-Prüfkopie mit [#1919](https://github.com/GiroCrab/GiroCrab/pull/1919).

## Befunde

| ID | Priorität | Kriterium | Plattform | Beobachtung | Weg | Status |
|---|---|---|---|---|---|---|
| BEL-1 | P1 | Adaptivität | iPhone | Filter und Aktionen verdeckten bei „Accessibility XL“ die Liste. Die Filter scrollen jetzt mit den Dokumenten | Fix | `fixed` |
| BEL-2 | P2 | Lesbarkeit | iPhone | Bei „Accessibility XL“ brechen Datumsangaben mitten im Jahr um. Mehrere Bankdokumente lassen sich am gekürzten Titel kaum unterscheiden | Issue | `open` |

## Impeccable-Bewertung

| Ansicht | Datum / Stand | Critique | Technisches Audit | Kurzbewertung |
|---|---|---|---|---|
| Belege & Postbox | 25.11.2026, `main` @ `09f11987` | 24/36 | 12/20 | Dokumenttypen und ungelesene Einträge lassen sich gut unterscheiden. Der feste Kopf erschwert bei großer Schrift den Zugriff auf die Dokumente. Import, Export und ihre Fehlerzustände waren außerhalb dieses Ausschnitts. |

## Prüfnachweis

### Belege & Postbox

- **Demo-Einstieg:** `--demo-seed --view belege`. Auf dem iPhone zunächst Standardgröße, dann „Accessibility XL“ wählen.
- **Nachprüfung:** 25.11.2026, `main` @ `9ae40509`, BEL-1 und BEL-2. SwiftUI-Stand mit vorhandenem Demo-Core sowie iOS-Buildfix [#1919](https://github.com/GiroCrab/GiroCrab/pull/1919).
- **BEL-1:** Seit [#1895](https://github.com/GiroCrab/GiroCrab/pull/1895) scrollt der Kopf der iPhone-`List` mit. Auf dem iPhone 18 Pro mit „Accessibility XL“ bis „Visa Gold – Kreditkartenabrechnung“ scrollen und den Beleg öffnen. Liste und PDF sind erreichbar (`iphone-belege-main-gross-liste.png`, `iphone-belege-main-gross-pdf.png`).
- **BEL-2:** Im selben Listenbild wird aus dem Datum 21.11.2026 eine Zeile „21.11.202“ und eine Zeile „6“. `DocumentRow` setzt Datum und Titel nebeneinander. Beide sollen bei großer Schrift zusammenhängend lesbar bleiben.
- **Vorschau und Schwärzung:** Für den REWE-Beleg `--pick rewe` ergänzen. Mit `--redact=labels` werden die Mac-Liste und die Vorschau sowie iPhone-Sheet-Titel und PDF-Inhalt verdeckt (`mac-belege-vorschau-redact.png`, `iphone-belege-vorschau-redact.png`).
