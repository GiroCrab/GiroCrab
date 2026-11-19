# GiroCrab — Projektübersicht (Android-Strich)

OpenSpec-Baum für den Android-Teil von GiroCrab (`GiroCrabAndroid/`). Angelegt am 19.11.2026 gegen
Commit `6c2613e` (Workspace-Version 0.8.6, Android-`versionName` 0.1.0). Der Baum liegt bewusst neben
dem Code, damit Zitate auf Datei:Zeile im selben Checkout auflösen.

## Was dieses OpenSpec abdeckt

Nur die Android-Oberfläche und ihre Anbindung an `girocrab-core`. Der Rust-Kern, die macOS-/iOS-App,
die Qt-Oberfläche und der Windows-Client sind nicht Gegenstand; sie sind hier nur Vergleichsmaßstab.

## Fähigkeiten

- `android-parity` — der Android-Client: Build der nativen Schicht, Tresor-Lebenszyklus,
  Schlüsselablage, Datenabbildung, Schreibpfade, Demo-Betrieb. Ist-Stand: `specs/android-parity/spec.md`.

## Maßstab für „fertig" (Parität)

Referenz ist der macOS-/iOS-Client, weil er den Kern vollständig nutzt: der Kern exportiert
**91 Funktionen** (`#[uniffi::export]`, 2 Konstruktoren, `girocrab-core/src`), die Swift-App ruft
**~150 verschiedene `vault.`-Methoden** auf. Android ruft heute **zwei** (`openOwnVault`,
`demoSnapshot`) — und die erste liefert `null`.

## Externe Systeme, die Android später braucht

- Bankzugänge über `girocrab-core/src/connectors/` (FinTS, comdirect, Trade Republic, Enable Banking,
  PayPal, Portal-Importer) — alle über denselben FFI-Rand, kein eigener Android-Code.
- Lokale Schnittstelle (Unix-Socket) und MCP-Adapter: auf Android nicht vorgesehen (kein Unix-Socket
  im App-Sandbox-Modell), bleibt Desktop-exklusiv.

## Konventionen in diesem Baum

- Sprache Deutsch (Repo-Präzedenz: `CLAUDE.md`, Issues und Doku sind deutsch), Datei- und
  Funktionsnamen im Original.
- Live-Specs führen den Ist-Stand tool-nativ als `### Requirement: <Name>` mit `#### Scenario:`.
- Jede Änderung ist ein Change unter `changes/<id>/` mit `proposal.md`, `design.md`, `tasks.md` und
  einem Delta in `changes/<id>/specs/android-parity/spec.md` — `ADDED` für neue Anforderungen,
  `MODIFIED` für geänderte (Change 001 nutzt überwiegend `MODIFIED`, weil die Fähigkeit schon als
  Ist-Spec beschrieben ist).
- Zitate als `datei:zeile` beziehen sich auf Commit `6c2613e`; nach Umbauten nachziehen.

## Belege nachprüfen

Die Zitate im Ist-Stand lassen sich ohne Werkzeug gegenlesen (`git checkout 6c2613e`):

```sh
grep -c '#\[uniffi::export\]' -r girocrab-core/src                     # 91 Exporte
grep -rhoE 'vault\.[a-zA-Z]+\(' --include=*.swift GiroCrabApp/Sources | sort -u | wc -l   # ~150
grep -rhoE 'core\.[a-zA-Z]+\(' --include=*.kt GiroCrabAndroid/app/src/main/java | sort -u # 2 echte
ls GiroCrabAndroid/app/src/main/jniLibs scripts/build-rust-android.sh  # beide fehlen
grep -cE 'cargo|ndk|rust' .github/workflows/android.yml                # 0
```
