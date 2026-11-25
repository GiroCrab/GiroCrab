---
name: GiroCrab
description: Native Banking-App, lokal verschlüsselt – ruhige, dichte Oberfläche für Konten, Buchungen und Vermögen
colors:
  canvas: "#F8FAFC"
  canvas-dark: "#0F172A"
  card: "#FFFFFF"
  card-dark: "#1E293B"
  sidebar: "#F3F4F6"
  sidebar-dark: "#0B1220"
  border: "#E2E8F0"
  border-dark: "#334155"
  text: "#0F172A"
  text-dark: "#F1F5F9"
  secondary: "#4B5A70"
  secondary-dark: "#A3B1C4"
  tertiary: "#64748B"
  tertiary-dark: "#8494AA"
  inflow: "#047857"
  inflow-dark: "#34D399"
  negative: "#B91C1C"
  negative-dark: "#F87171"
  warning: "#B45309"
  warning-dark: "#F59E0B"
  invest: "#1F7A5C"
  invest-dark: "#5EC79F"
  loan: "#64748B"
  loan-dark: "#94A3B8"
typography:
  large-title:
    fontFamily: "SF Pro Rounded, system-ui"
    fontSize: "26pt"
    fontWeight: 600
  title3:
    fontFamily: "SF Pro, system-ui"
    fontSize: "15pt"
    fontWeight: 600
  headline:
    fontFamily: "SF Pro, system-ui"
    fontSize: "13pt"
    fontWeight: 700
  body:
    fontFamily: "SF Pro, system-ui"
    fontSize: "13pt"
    fontWeight: 400
  callout:
    fontFamily: "SF Pro, system-ui"
    fontSize: "12pt"
    fontWeight: 400
  subheadline:
    fontFamily: "SF Pro, system-ui"
    fontSize: "11pt"
    fontWeight: 400
  caption:
    fontFamily: "SF Pro, system-ui"
    fontSize: "10pt"
    fontWeight: 400
rounded:
  inset: "6pt"
  control: "8pt"
  card: "12pt"
spacing:
  section-inset: "12pt"
  section-padding: "8pt"
  section-spacing: "4pt"
  card-gap: "4pt"
  card-spacing: "8pt"
  card-padding-compact: "12pt"
  card-padding: "16pt"
  block: "14pt"
components:
  flat-section:
    backgroundColor: "system"
    padding: "{spacing.section-padding} {spacing.section-inset}"
  card:
    backgroundColor: "{colors.card}"
    rounded: "{rounded.card}"
    padding: "{spacing.card-padding}"
---

# Design System: GiroCrab

Beschreibt den **heutigen Stand** der Apple-Oberfläche (`GiroCrabApp/`); Werte im Kopf sind macOS-Punktgrößen
bei Schriftgröße „Standard“. Quelle der Wahrheit bleibt der Code: Farben in
`GiroCrabUI/Formatting.swift` (`enum Theme`), Schrift in `GiroCrabUI/TextSize.swift`, Abstände in
`GiroCrabUI/ViewLayout.swift`. Linux spiegelt die Farben in `GiroCrabLinux/qml/Theme.qml`.
Wer hier etwas ändert, ändert zuerst den Code. Produktkontext: [`PRODUCT.md`](PRODUCT.md);
Audit-Ablauf: [`docs/ui-audit.md`](docs/ui-audit.md). Verbindliche technische Regeln (Schrift, Schwärzung,
Dialoge) stehen in [`CLAUDE.md`](CLAUDE.md); diese Datei verweist darauf und wiederholt sie nicht.

## Overview

Charakter **Operate**: ein ruhiges, dichtes Arbeitswerkzeug mit nativen Plattformmustern, ein blauer Akzent,
Farbe nur dort, wo sie etwas bedeutet. Der Mac ist die Hauptplattform: drei Spalten
(Seitenleiste, Buchungsliste, Inspektor), viel Information auf wenig Fläche, Bedienung auch per Tastatur.
Die Hauptspalten (Übersicht, Kennzahlen, Buchungsliste, Inspektor) sind flach wie klassische Mac-Apps:
Systemflächen, Trennlinien und klare Typografie statt Kacheln (#1666). Andere Ansichten (Vermögen, Depots,
Konto-Ansichten, Einrichtung) nutzen noch Karten auf `canvas` (Schiefer-Grautöne, „Daylight“ / „Nightfall“).
Auf dem iPhone dieselben Inhalte im Navigationsstapel, der Inspektor als Sheet.

Beträge sind das zentrale Element. Sie sind gleich breit gesetzt, stehen rechts und werden nie gekürzt;
wenn der Platz knapp wird, kürzt sich der Name daneben.

## Colors

Jede Farbe hat vier Varianten: hell, dunkel und je eine für „Kontrast erhöhen“ (`ThemeShade`).
Alle Textfarben erreichen mindestens 4,5:1 auf `canvas` und `card`; `ThemeContrastTests` prüft das
für jede Variante. Ein neues Text-Token gehört in `Theme.textSpecs`, sonst prüft es niemand.
**Offen:** Die Systemflächen der Hauptspalten (`systemBackground`, `systemSurface`) prüft der Test nicht.

### Primary

Akzent: das App-Blau aus dem Asset-Katalog (`AccentColor`). Wählt jemand in den Systemeinstellungen einen
anderen Akzent, gilt dieser. Auswahlflächen (`Theme.selection`) sind der Akzent mit geringer Deckkraft.

### Neutral

Hauptspalten: Systemfarben `Theme.systemBackground` (Spaltenhintergrund), `Theme.systemSurface`
(Eingabe- und Diagrammflächen), `Theme.systemSeparator` (Trennlinien); sie folgen Hell/Dunkel und den
Bedienungshilfen des Systems. Übrige Ansichten: `canvas` (Hintergrund), `card` (Karten), `sidebar`,
`border` (1-pt-Haarlinie).
Text in drei Stufen: `text` → `secondary` (Beschriftungen) → `tertiary` (Metadaten, Symbole).
Die Stufen müssen unterscheidbar bleiben.

### Named Rules

- **Richtung statt Wertung:** Zuflüsse `inflow` (grün); Abflüsse in `text`-Farbe, nicht rot.
  Rot (`negative`) steht für negative Salden und Belastungen auf Bestellkonten, nicht für jede Ausgabe.
- **Kredite neutral:** Darlehen erscheinen in `loan` (grau), nicht als Warnung.
- **Bedeutung nie nur über Farbe:** Vorzeichen, Symbol oder Text tragen dieselbe Information.

## Typography

Systemschrift (SF Pro), gesetzt über `.appFont`, damit sie der Schriftgröße folgt
(macOS ×1 / ×1,15 / ×1,3, iOS Dynamic Type). Regeln und Ausnahmen: `CLAUDE.md` › Text size.

### Hierarchy

| Stil | macOS-Größe | Verwendung |
|---|---|---|
| `largeTitle` | 26 pt | Betrag einer Buchung im Inspektor |
| `title3` semibold | 15 pt | Händler-/Kontoname im Kopf |
| `headline` / `body` | 13 pt | Listeninhalte, Überschriften von Hinweisen |
| `callout` | 12 pt | Verwendungszweck, Erklärtexte |
| `subheadline` | 11 pt | Salden in der Seitenleiste, Kartentitel |
| `caption` | 10 pt | Datum, Metadaten, kleine Aktionen |

### Named Rules

- **Ziffern gleich breit:** Beträge, Salden, Kurse, Stückzahlen, Prozentwerte und Daten in Spalten mit
  `monospacedDigit: true`, damit sie ruhige Fluchtlinien bilden.
- **Format aus einer Hand:** Beträge nur über `Money.format` / `Money.signed` (Locale des Nutzers,
  echtes Minuszeichen U+2212, Währung nach Locale). Nie selbst zusammensetzen.
- **Vergleichbare Beträge, gleiche Stufe:** Summen und die Beträge, die sie zusammenfassen, stehen in derselben Größe.
- **Nie abschneiden:** Beträge bekommen `.fixedSize()`; der Name daneben kürzt sich. Kein `minimumScaleFactor`.

## Layout

- macOS: `NavigationSplitView` mit Seitenleiste, Buchungsliste (nach Tagen gruppiert) und Inspektor.
  Der Inspektor ist eine Folge flacher Abschnitte mit Trennlinien; Reihenfolge und Sichtbarkeit sind einstellbar.
- iOS: Navigationsstapel. Auf dem iPhone öffnet sich der Inspektor als Sheet (`.medium`, `.large`), bei Bedienungshilfen-Schriftgrößen in voller Höhe (`.large`).
- Abstände der Hauptspalten: `ViewLayout.sectionInset` (12 pt) seitlich, `sectionPadding` (8 pt) oben und
  unten, `sectionSpacing` (4 pt) zwischen Elementen; `cardGap` (4 pt) zwischen den Blöcken der Buchungsliste.
  Konto-Ansichten: `assetBlockSpacing` (14 pt), Karten dort 16 pt Innenabstand.
- **Feste Maße, die Text enthalten, wachsen mit** der Schriftgröße (`textScale`, `CLAUDE.md` › Text size).
- **Lange deutsche Texte:** Ansichten bleiben bei schmalen Fenstern und auf dem iPhone lesbar; lieber eine
  zweite Zeile als ein abgeschnittenes Wort.
- **Trefferflächen:** auf iOS mindestens 44 pt hoch (Listenzeilen, Tabellenzeilen, Diagramme zum Antippen).

## Elevation & Depth

Flach, ohne Schatten. In den Hauptspalten gliedern volle Trennlinien (`Divider()`) und Typografie die
Abschnitte; in den übrigen Ansichten trennen Flächenfarbe (`canvas` → `card`) und eine 1-pt-Haarlinie
(`border`). Sheets und Popover nutzen die Systemdarstellung.

Bewegung ist knapp und funktional (meist 0,15–0,25 s). Animationen laufen über
`Motion.animation(_:)` bzw. `Motion.slide(_:)` (`Formatting.swift`): Mit „Bewegung reduzieren“ erscheinen
Änderungen sofort oder blenden nur über.

## Shapes

- Abschnitte der Hauptspalten: ohne Rahmen und ohne Radius.
- Karten (übrige Ansichten): 12 pt, `.continuous` (`CardBackground`, `.card()`).
- Kleinere Flächen und Rahmen (Kacheln, Vorschlagsboxen, Felder): meist 5–8 pt.
- **Offen:** Im Code kommen heute rund zehn verschiedene Radien vor (4–16 pt). Ein festes Set ist noch
  nicht entschieden; neue Ansichten verwenden 12 / 8 / 6 pt.

## Components

### Flat Sections

`.flatSection()` (`Formatting.swift`): kein Hintergrund, kein Rahmen, `sectionInset` / `sectionPadding` als
Innenabstand. Abschnitte stehen untereinander und werden durch `Divider()` getrennt. Standard für Übersicht,
Buchungsliste, Inspektor und Planung.

### Cards / Containers

Nur noch in Ansichten außerhalb der Hauptspalten. `.card()` (`Formatting.swift`): `card`-Fläche, Radius 12 pt, Haarlinie, 16 pt Innenabstand
(`padding:` und `vertical:` überschreibbar). Karten werden nicht in Karten geschachtelt.

### Navigation

Seitenleiste: Zeilen mindestens `34 * scale` hoch; Salden rechts in `subheadline` mit
`monospacedDigit`, Name kürzt sich. Gruppen- und Bereichssummen in derselben Spalte.

### Sensible Inhalte

Nicht sichtbar, aber Teil jeder Komponente: Persönliches, Beträge und Bezeichnungen sind für „Problem melden“
markiert, auch in Titeln, Sheets und Popovern. Wie: `CLAUDE.md` › Sensitive UI.

## Do's and Don'ts

### Do:

- Bestehende Tokens (`Theme.*`, `ViewLayout.*`, `.flatSection()`, `.card()`) verwenden, bevor neue Werte entstehen.
- Jede geänderte Ansicht hell und dunkel, bei Schriftgröße „Sehr groß“, schmal und geschwärzt (`--redact`)
  ansehen, immer mit Demo-Daten.
- Fehlertexte so schreiben, dass sie den nächsten Schritt nennen; Codes nur in Klammern dahinter.
- Persönliches und Beträge für „Problem melden“ markieren, auch in Titeln, Sheets und Popovern
  (`CLAUDE.md` › Sensitive UI); prüfen mit `--redact`.
- Beträge nur über `Money.format` / `Money.signed`, mit `monospacedDigit` und `.fixedSize()`.

### Don't:

- Keine Verläufe, Glow-Effekte oder Farbe als Schmuck; einzige Ausnahme ist die dezente Fläche unter Verlaufslinien (`Theme.chartAreaFill`).
- Keine Karten in den Hauptspalten; keine Karte in einer Karte.
- Keine Animation ohne `Motion`.
- Keine Beträge verschiedener Währungen in einer Summe.
