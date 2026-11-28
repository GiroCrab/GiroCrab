# UI-Audit: Belege & Postbox

- **Ansicht:** Dokumentliste mit Filtern und Vorschau auf macOS und iPad sowie Vorschau-Sheet am iPhone.
- **Dateien:** `DocumentsUI.swift` und `AccountDocumentsUI.swift` in `GiroCrabUI/`.
- **Werkzeug:** Native Impeccable `critique` und `audit` anhand von SwiftUI-Code und Demo-Aufnahmen, dazu GiroCrab-Checkliste.
- **Nachweis:** macOS 27.2.1, iPhone 18 Pro und Pro Max sowie iPad mini (iOS 27.2), Deutsch. Standard, große Schrift und geschwärzte Varianten. [Prüfbuild vom 25.11.2026](README.md#prüfbuilds).

## Befunde

| ID | Priorität | Kriterium | Plattform | Beobachtung | Weg | Status |
|---|---|---|---|---|---|---|
| BEL-1 | P1 | Adaptivität | iPhone | Filter und Aktionen verdeckten bei „Accessibility XL“ die Liste. Die Filter scrollen jetzt mit den Dokumenten | Fix | `fixed` |
| BEL-2 | P2 | Lesbarkeit | iPhone | Bei „Accessibility XL“ brechen Datumsangaben mitten im Jahr um. Mehrere Bankdokumente lassen sich am gekürzten Titel kaum unterscheiden | Fix | `fixed` |

## Impeccable-Bewertung

| Ansicht | Datum / Stand | Critique | Technisches Audit | Kurzbewertung |
|---|---|---|---|---|
| Belege & Postbox | 25.11.2026, `main` @ `09f11987` | 24/36 | 12/20 | Dokumenttypen und ungelesene Einträge lassen sich gut unterscheiden. Der feste Kopf erschwert bei großer Schrift den Zugriff auf die Dokumente. Import, Export und ihre Fehlerzustände waren außerhalb dieses Ausschnitts. |

## Prüfnachweis

### Belege & Postbox

- **Demo-Einstieg:** `--demo-seed --view belege`. Auf dem iPhone zunächst Standardgröße, dann „Accessibility XL“ wählen.
- **Nachprüfung:** 25.11.2026, BEL-1 und BEL-2 auf dem iPhone 18 Pro (iOS 27.2), Deutsch, hell, „Accessibility XL“. SwiftUI und Core aus `9ae40509`, xcframework frisch im Release-Profil gebaut. [App-Commit 9ae40509](https://github.com/GiroCrab/GiroCrab/commit/9ae40509829c58b2c8532b53d87f26882154c1ce) · [Swift-Patch](README.md#prüfbuilds)
- **BEL-1:** Seit [#1895](https://github.com/GiroCrab/GiroCrab/pull/1895) scrollt der Kopf der iPhone-`List` mit. Auf dem iPhone 18 Pro mit „Accessibility XL“ bis „Visa Gold – Kreditkartenabrechnung“ scrollen und den Beleg öffnen. Liste und PDF sind erreichbar.

- **Vorschau und Schwärzung:** Für den REWE-Beleg `--pick rewe` ergänzen. Mit `--redact=labels` werden die Mac-Liste und die Vorschau sowie iPhone-Sheet-Titel und PDF-Inhalt verdeckt.

#### BEL-2 · Datum und Dokumenttitel sind schwer lesbar

- **Ort:** `DocumentsUI.swift`, `DocumentRow`.
- **Beobachtung:** Beim Musterbank-Kontoauszug am iPhone bei „Accessibility XL“ steht „20.11.202“ über „6“. Neben dem Datum endet der Titel bei „bank –…“.
- **Vorschlag:** Das Datum bei Platzmangel in die Metadatenzeile verschieben und dem Titel mehr Breite geben.
- **Prüfung:** Wie bei BEL-1 in die Liste scrollen. Datum und Titel sind zusammenhängend lesbar. Mehrere Musterbank-Dokumente lassen sich unterscheiden.

- **Nachprüfung:** 27.11.2026, iPhone 18 Pro (Simulator, iOS 27.0), Deutsch, hell, „Accessibility XL“. Vollständige App vor (`068fde32`) und nach dem Fix (`bfdd9434`), mit Demodaten. Titel und Datum des Musterbank-Kontoauszugs sind nachher vollständig lesbar. Bildpaar in [#2021](https://github.com/GiroCrab/GiroCrab/pull/2021).
