# UI-Audit: Seitenleiste

- **Ansicht:** Navigation, Kontenbaum mit Salden, Kontensuche und Gruppen. Kategorien werden mit dem Navigationspfad betrachtet.
- **Dateien:** `SidebarView.swift` und `AllAccountsRoot.swift` in `GiroCrabUI/`.
- **Werkzeug:** Native Impeccable `critique` und `audit` anhand von SwiftUI-Code und Demo-Aufnahmen, dazu GiroCrab-Checkliste.
- **Nachweis:** macOS 27.2.1 und iPhone 18 Pro (iOS 27.2), Deutsch. Standard und große Schrift, Schwärzung auf Mac und iPhone. [Prüfbuild vom 25.11.2026](README.md#prüfbuilds).

## Befunde

| ID | Priorität | Kriterium | Plattform | Beobachtung | Weg | Status |
|---|---|---|---|---|---|---|
| SEI-1 | P2 | Orientierung | macOS, iPhone | Bei großer Schrift sind mehrere Konten fast nur am Saldo zu unterscheiden. Auf dem iPhone bleibt vom „Musterbank Girokonto“ ein „M“, bei der Kreditkarte nur das Kürzungszeichen | Fix | `fixed` |
| SEI-2 | P2 | Adaptivität | iPhone | Bei „Accessibility XL“ ragen die Symbole in „Neue Umsätze“ und „Vermögen“ in den Text | Fix | `fixed` |

## Impeccable-Bewertung

| Ansicht | Datum / Stand | Critique | Technisches Audit | Kurzbewertung |
|---|---|---|---|---|
| Seitenleiste | 25.11.2026, `main` @ `09f11987` | 26/36 | 13/20 | Die Gruppen und rechts ausgerichteten Salden erleichtern die Orientierung. Bei großer Schrift reichen die trotz Salden und Abzeichen verbleibenden Namensspalten nicht zur sicheren Auswahl. Die Pfeiltasten-Regressionsprüfung gehört zu [#1779](https://github.com/GiroCrab/GiroCrab/issues/1779) und dem offenen [PR #1883](https://github.com/GiroCrab/GiroCrab/pull/1883). |

## Prüfnachweis

### Seitenleiste

- **Demo-Einstieg:** `--demo-seed`, Konten nach Gruppen. Auf dem iPhone von „Alle Konten“ mit dem Zurück-Pfeil zur Seitenleiste wechseln. Standard und „Accessibility XL“ vergleichen.

- **Nachprüfung (27.11.2026):** SEI-1 und SEI-2 mit dem Fix auf Basis von `bfc5c262` verglichen. Auf macOS 27.2.1 bei „Sehr groß“ und schmalem Fenster sowie auf dem iPhone 18 Pro (iOS 27.2) bei „Accessibility XL“ sind Kontonamen und Salden lesbar. Auf dem iPhone bleiben die Symbole von „Neue Umsätze“ und „Vermögen“ vom Text getrennt. Das Girokonto ließ sich bei „Accessibility XL“ öffnen. Standardgröße auf beiden Plattformen sowie Dunkelmodus mit Schwärzung auf macOS geprüft.
