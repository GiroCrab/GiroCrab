# UI-Audit: Seitenleiste

- **Ansicht:** Navigation, Kontenbaum mit Salden, Kontensuche und Gruppen. Kategorien werden mit dem Navigationspfad betrachtet.
- **Dateien:** `SidebarView.swift` und `AllAccountsRoot.swift` in `GiroCrabUI/`.
- **Werkzeug:** Native Impeccable `critique` und `audit` anhand von SwiftUI-Code und Demo-Aufnahmen, dazu GiroCrab-Checkliste.
- **Nachweis:** macOS 27.2.1 und iPhone 18 Pro (iOS 27.2), Deutsch. Standard und große Schrift, Schwärzung auf Mac und iPhone. [Prüfbuild vom 25.11.2026](README.md#prüfbuilds).

## Befunde

| ID | Priorität | Kriterium | Plattform | Beobachtung | Weg | Status |
|---|---|---|---|---|---|---|
| SEI-1 | P2 | Orientierung | macOS, iPhone | Bei großer Schrift sind mehrere Konten fast nur am Saldo zu unterscheiden. Auf dem iPhone bleibt vom „Musterbank Girokonto“ ein „M“, bei der Kreditkarte nur das Kürzungszeichen | Issue | `open` |
| SEI-2 | P2 | Adaptivität | iPhone | Bei „Accessibility XL“ ragen die Symbole in „Neue Umsätze“ und „Vermögen“ in den Text | Issue | `open` |

## Impeccable-Bewertung

| Ansicht | Datum / Stand | Critique | Technisches Audit | Kurzbewertung |
|---|---|---|---|---|
| Seitenleiste | 25.11.2026, `main` @ `09f11987` | 26/36 | 13/20 | Die Gruppen und rechts ausgerichteten Salden erleichtern die Orientierung. Bei großer Schrift reichen die trotz Salden und Abzeichen verbleibenden Namensspalten nicht zur sicheren Auswahl. Die Pfeiltasten-Regressionsprüfung gehört zu [#1779](https://github.com/GiroCrab/GiroCrab/issues/1779) und dem offenen [PR #1883](https://github.com/GiroCrab/GiroCrab/pull/1883). |

## Prüfnachweis

### Seitenleiste

- **Demo-Einstieg:** `--demo-seed`, Konten nach Gruppen. Auf dem iPhone von „Alle Konten“ mit dem Zurück-Pfeil zur Seitenleiste wechseln. Standard und „Accessibility XL“ vergleichen.

#### SEI-1 · Kontonamen werden auf einzelne Zeichen gekürzt

- **Ort:** `SidebarView.swift` (`AccountRow`) und `AllAccountsRoot.swift` (`AllAccountsRow`).
- **Beobachtung:** Auf dem schmalen Mac werden Tagesgeldnamen gekürzt. Am iPhone bleibt bei „Accessibility XL“ vom Girokonto „M“ und bei der Kreditkarte nur das Kürzungszeichen.
- **Vorschlag:** Bei Platzmangel Namen und Saldo untereinander anordnen oder dem Namen eine zweite Zeile geben.
- **Prüfung:** Auf Mac und iPhone die ersten Konten bei großer Schrift vergleichen und das Girokonto auswählen. Namen und Salden bleiben zuordenbar. [Bildbelege (ZIP)][bildbelege] · `mac-sidebar-gross.png`, `iphone-sidebar-gross.png`.

#### SEI-2 · Seitensymbole ragen in den Text

- **Ort:** `SidebarView.swift`, Navigationszeilen mit 20-pt-Symbolspalte.
- **Beobachtung:** Auf dem iPhone bei „Accessibility XL“ ragen die Symbole in „Neue Umsätze“ und „Vermögen“ in die Beschriftung.
- **Vorschlag:** Die reservierte Symbolspalte mit der Schrift skalieren und einen festen sichtbaren Abstand zum Text vorsehen.
- **Prüfung:** Dieselben Navigationspunkte bei Standard und „Accessibility XL“ vergleichen. Beide Symbole bleiben vom Text getrennt. [Bildbelege (ZIP)][bildbelege] · `iphone-sidebar-gross.png`.

[bildbelege]: https://github.com/user-attachments/files/35449133/PR-Bildbelege.zip
