# Verification Plan

Was GiroCrab zu seiner Sicherheit zusagt, womit das geprüft wird und was die Prüfung **nicht** abdeckt.
Die Zusagen selbst stehen in [`SECURITY.md`](../SECURITY.md).

*English summary: this page lists, for each security promise, the lint, test or CI step that checks it, how to run it
locally, and what it does not cover. There has been no external audit. A green check means the stated rule holds,
not that the app is free of vulnerabilities.*

## Wie die Prüfungen laufen

| Wann | Was |
|---|---|
| vor jedem Push (nach `git config core.hooksPath .githooks`) | `scripts/verify-fast.sh --lint-only`: alle `scripts/lint-*.sh`, Formatierung, Lizenzhinweise, Suche nach einer FinTS-Produkt-ID |
| jeder Pull Request | [`ci.yml`](../.github/workflows/ci.yml): die Lint-Schritte immer; Clippy, Tests und die Builds der Plattformen nur, wenn die Änderung sie betreffen kann (Job `changes`) |
| jeder Pull Request, jeder Push auf `main` und wöchentlich | `ci.yml`, Job `audit`: bekannte Meldungen der RustSec-Datenbank; der wöchentliche Lauf findet neue auch ohne Änderung im Repo |
| geplant alle 6 Stunden | [`release-integrity.yml`](../.github/workflows/release-integrity.yml): `appcast.xml` mit den Disk-Images seiner Einträge und `banking-policy.json` mit Signatur. Nicht enthalten: Windows-, Linux- und Android-Pakete sowie die Mitteilungen. GitHub kann geplante Läufe verspäten oder auslassen |

Für `main` sind die Checks `rust`, `swift` und `audit` vorgeschrieben (Stand 24.11.2026, nicht für Admins). Ein übersprungener Job zählt
dabei als bestanden. `scripts/lint-connectors.sh` läuft im Push-Hook, aber nicht als Schritt in `ci.yml`.

## Zusagen und Prüfungen

Befehle aus dem Wurzelverzeichnis des Repos. `cargo`-Befehle brauchen `GIROCRAB_SKIP_SWIFT=1`, wenn Xcode fehlt.

| Nr. | Zusage | Prüfung | Nicht abgedeckt |
|---|---|---|---|
| 1 | Ohne Schlüssel öffnet sich der Tresor nicht | `cargo test -q -p girocrab-core --test schema_fixtures`; der Test in `demo.rs` prüft einen falschen Schlüssel | Stärke der Verschlüsselung: Sie kommt von SQLCipher. Schutz des Speichers im Betrieb |
| 2 | Sicherungen brauchen das Passwort; Wiederherstellen übernimmt keine PINs und FinTS-Adressen | `cargo test -q -p girocrab-core --lib vault::backup` | Wie schwer ein Passwort zu erraten ist |
| 3 | FinTS nur über öffentliche `https`-Adressen | `cargo test -q -p girocrab-core --lib fints_presets` | Zertifikatsprüfung: Dafür gibt es keinen eigenen Test, sie kommt von der TLS-Bibliothek |
| 4 | Zugangsdaten folgen keiner Weiterleitung zu fremden Hosts | `cargo test -q -p girocrab-core --lib redirect` | Ob jeder Konnektor den richtigen Client nutzt, prüft kein Lint ([#1794](https://github.com/GiroCrab/GiroCrab/issues/1794)) |
| 5 | Antworten haben eine Obergrenze | `cargo test -q -p girocrab-core --lib body_limit`; Clippy verbietet unbegrenztes Lesen (`girocrab-core/clippy.toml`) | Antworten in WebViews |
| 6 | FinTS-Schlüsseldateien werden geprüft und begrenzt gelesen | `cargo test -q -p girocrab-core --lib keyfile` | Eignung von 3DES ([#1796](https://github.com/GiroCrab/GiroCrab/issues/1796)) |
| 7 | Der Kern startet keine Prozesse, lädt keinen Code nach und enthält `unsafe` nur in `sys.rs` | `scripts/lint-no-obfuscation.sh`, mit `--compiled` auch der Build; Clippy-Verbote | Oberflächen, Portal-Skripte und der Quelltext der Abhängigkeiten |
| 8 | Lokale Schnittstelle: nur lesend, nur mit Schlüssel, nur freigegebene Konten und Felder | `cargo test -q -p girocrab-core --lib local_api` (Unix); `cargo test -q -p girocrab-mcp` | Was ein Programm mit den Daten macht |
| 9 | Texte für Fehlerberichte sind bereinigt | `cargo test -q -p girocrab-core --lib report_sanitizer`; Swift `BugReportTests` | Unbekannte Formate; deshalb prüfst du den Bericht selbst |
| 10 | Dialoge im Screenshot gehen durch die Schwärzung | `scripts/lint-report-dialogs.sh` | Ob jede Ansicht ihre Texte mit `.sensitive()` markiert, prüft kein Lint |
| 11 | Ansichten greifen nur über `AppModel` auf den Tresor zu | `scripts/lint-vault-access.sh`, `scripts/lint-appmodel-internals.sh` | Dient vor allem Bedienbarkeit und Aufbau; keine Sicherheitsgrenze |
| 12 | Sicherheitssperre und Mitteilungen gelten nur mit gültiger Signatur und nie mit älterer Nummer | Swift `BankingPolicyTests`, `AnnouncementsTests`; `cargo test` in `GiroCrabLinux/` (braucht Qt) und `GiroCrabWindows/` | Linux und Windows haben weniger Tests als Swift |
| 13 | Keine bekannten Lücken in Rust-Abhängigkeiten | `cargo audit --deny warnings` (Ausnahmen mit Grund in `.cargo/audit.toml`) | Unbekannte Lücken; Swift, Gradle und die eigenen `Cargo.lock` von Linux und Windows |
| 14 | Direkte Abhängigkeiten stehen in den Lizenzhinweisen | `scripts/check-third-party-notices.sh` | Indirekte Abhängigkeiten, Richtigkeit der Lizenzangabe ([#1800](https://github.com/GiroCrab/GiroCrab/issues/1800)) |
| 15 | Portal-Skripte lesen die erwarteten Daten | `node --test` mit den Dateien aus dem Schritt „Test FNZ portal browser scripts“ in `ci.yml` | Domain-Prüfung und Navigation der WebViews haben keine eigenen Tests ([#1797](https://github.com/GiroCrab/GiroCrab/issues/1797)) |
| 16 | Schlüsselbund-Regeln (Biometrie, nur auf diesem Gerät) | Swift `KeychainBypassTests`, `KeychainSyncAvailabilityTests` (Einstellungen und Entwicklerschalter); die Schutzklassen selbst nur per Code-Review (`VaultKeyManager.swift`) | Das Verhalten des Schlüsselbunds auf einem echten Gerät ([#1795](https://github.com/GiroCrab/GiroCrab/issues/1795), [#1798](https://github.com/GiroCrab/GiroCrab/issues/1798)) |

Swift-Tests einzeln: `cd GiroCrabApp && swift test -q --filter BankingPolicyTests`.

## Signaturketten

| Was | Wer signiert | Wer prüft |
|---|---|---|
| macOS-App und Disk-Image | Developer ID, Notarisierung durch Apple; nur auf dem Mac des Maintainers (`scripts/notarize.sh`) | `notarize.sh` vor dem Hochladen, danach Gatekeeper |
| Update-Feed und Updates | Ed25519-Schlüssel für Sparkle | Sparkle vor dem Entpacken, dazu `release-integrity.yml` ([`auto-updates.md`](auto-updates.md)) |
| Sicherheitssperre und Mitteilungen | eigener Ed25519-Schlüssel, getrennt von Sparkle | die Apps; bei der Sicherheitssperre auch `release-integrity.yml` ([`banking-policy.md`](banking-policy.md)) |
| iOS | Apple (App Store, TestFlight) | Apple |
| Nightly aus der CI | nur ad hoc signiert | niemand; kein Ersatz für ein Release |
| Windows | **keine Signatur**, Prüfsummen in `SHA256SUMS.txt` ([`windows.md`](windows.md)) | du, per Prüfsumme |
| Linux (`.deb`, Flatpak aus dem Release) | **keine Signatur**, Prüfsummen in `SHA256SUMS-linux.txt` | du, per Prüfsumme |
| Android-Alpha | Debug-Build | niemand; nur zum Testen |

Prüfsummen liegen am selben Ort wie die Dateien. Sie zeigen Übertragungsfehler, aber keinen Angreifer, der beides
austauscht. Nicht jedes Release enthält alle Pakete: Bei v0.8.8 ist der Bau der Linux- und Windows-Pakete fehlgeschlagen.

### Selbst prüfen

Update-Feed und Sicherheitssperre, nur mit öffentlichen Schlüsseln aus dem Repo (lädt die Disk-Images, braucht macOS):

```bash
scripts/verify-release-integrity.sh --state build/integrity-state.json
```

Mit `--state` vergleicht das Skript mit früheren Läufen und meldet eine Sicherheitssperre mit kleinerer `sequence` sowie einen
verschwundenen Update-Feed. Das gelingt nur, wenn beides schon einmal gefunden und gespeichert wurde, und Schreibfehler meldet
es nicht. Ein Lauf ist nur vollständig, wenn

- die Ausgabe mit `==> all checks passed` endet (Exit-Code 0; sonst stehen die Abweichungen als `FINDING` in der Ausgabe),
- sie `ok: appcast.xml signature` und `ok: banking-policy.json sequence …` enthält (ohne gültigen öffentlichen Schlüssel
  überspringt das Skript die jeweilige Prüfung nur mit einem Hinweis) und
- in der Datei `policySequence` mit der ausgegebenen Nummer und `appcastSeen` als `true` steht.

Bewahre die Datei auf (`build/` wird beim Aufräumen leicht gelöscht).

Das heruntergeladene Disk-Image auf dem Mac:

```bash
spctl -a -vvv -t open --context context:primary-signature GiroCrab-macOS-arm64.dmg
```

```bash
xcrun stapler validate GiroCrab-macOS-arm64.dmg
```

Linux-Pakete, im Ordner mit den Dateien und `SHA256SUMS-linux.txt`:

```bash
sha256sum --check --ignore-missing SHA256SUMS-linux.txt
```

## Was fehlt

- Es gab kein externes Audit.
- Builds sind nicht reproduzierbar dokumentiert: Wer selbst baut, kann das Ergebnis nicht bitgenau mit dem Release vergleichen.
- Actions sind erst teilweise auf Commit-SHAs festgelegt ([#1802](https://github.com/GiroCrab/GiroCrab/issues/1802)),
  Dependabot ([#1801](https://github.com/GiroCrab/GiroCrab/issues/1801)), CODEOWNERS
  ([#1799](https://github.com/GiroCrab/GiroCrab/issues/1799)), SBOM
  ([#1805](https://github.com/GiroCrab/GiroCrab/issues/1805)) und Tag-Rulesets
  ([#1804](https://github.com/GiroCrab/GiroCrab/issues/1804)) stehen aus.
- Die Transparenzkarte an Pull Requests ist eine Selbstauskunft ([`ai-transparency.md`](ai-transparency.md),
  [#1803](https://github.com/GiroCrab/GiroCrab/issues/1803)).

## Pflege

Wer einen Lint, einen CI-Schritt oder eine Signaturregel ändert, ändert hier die passende Zeile. Eine neue
Zusage in `SECURITY.md` bekommt hier eine Zeile, auch wenn die Prüfung „keine“ ist.
