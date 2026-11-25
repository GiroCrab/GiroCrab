# Audit-Scope

Umfang für ein externes Sicherheitsaudit von GiroCrab. Das ist ein Vorschlag zum Vorbereiten: Es ist kein Audit beauftragt
und keine Förderung beantragt. Zusagen und Bedrohungsmodell stehen in [`SECURITY.md`](../SECURITY.md), vorhandene
Prüfungen und ihre Lücken im [Verification Plan](verification-plan.md).

*English summary: proposed scope for an external security audit; nothing has been commissioned or applied for. Priority 1
is the vault and its keys, the payment path, the network layer and the handling of credentials. Priority 2 is the
parsing of untrusted input, portal web views, sync and the local interface. Priority 3 is the supply chain and the
Linux and Windows front ends. Out of scope are banks and portals themselves, the Android alpha and denial of service.
GiroCrab is licensed under GPL-3.0-or-later.*

## Was geprüft werden soll

Leitfrage: Kann jemand ohne entsperrtes Gerät an Daten oder Zugangsdaten kommen, oder eine Zahlung anders ausführen
lassen, als sie angezeigt wurde?

Größen in Zeilen ohne Testdateien, gerundet (24.11.2026, Commit `86056966`); Tests innerhalb einer Datei zählen mit. Der
Rust-Kern hat rund 121 000 Zeilen, davon 55 000 in Konnektoren. Ein vollständiger Review ist deshalb nicht das Ziel.

### Priorität 1

| Bereich | Wo | Fragen |
|---|---|---|
| Tresor und Schlüssel | `girocrab-core/src/vault.rs` (800), `vault/backup.rs` (290), `GiroCrabUI/VaultKeyManager.swift` (460), `GiroCrabLinux/src/vault_key.rs` (140), `GiroCrabWindows/src/vault_key.rs` (120) | Schlüsselerzeugung und -weg, Rohschlüssel für SQLCipher, Schutzklassen, Verhalten ohne Biometrie, Sicherungen, zurückbleibende Dateien |
| Zahlungsweg | `connectors/transfer_session.rs` (350), `fints/transfer.rs` (810), `fints/sepa.rs` (720), `fints/standing_order.rs`, `fints/debit.rs`, Bestätigungsansichten der Oberflächen | Stimmt das Angezeigte mit dem Gesendeten überein? Empfängerüberprüfung, TAN-Abgleich, doppeltes Senden, Wiederholung nach Fehlern |
| Netzwerk und Anmeldung | `connectors/mod.rs`, `fints/dialog.rs` (940), `fints/rah10.rs`, `fints/keyfile.rs`, `fints/bank_keys.rs`, `connectors/dpop.rs` (180), `comdirect.rs`, `enable_banking.rs` (2700), `trade_republic.rs` (2800) | TLS, Weiterleitungen, Prüfung der Bankschlüssel, Zuordnung von Sitzungen und Tokens |
| Zugangsdaten | `vault/connections.rs`, `vault/settings.rs`, `GiroCrabUI/EnableBankingKeySync.swift`, `EnableBankingLoginSync.swift` | Ablage, Kopien im Speicher, Protokolle, iCloud-Schlüsselbund |

`connectors/…`, `fints/…` (unter `connectors/`) und `vault/…` liegen in `girocrab-core/src/`, `GiroCrabUI/` in `GiroCrabApp/Sources/`.

### Priorität 2

| Bereich | Wo | Fragen |
|---|---|---|
| Fremde Eingaben | `fints/syntax.rs`, `camt.rs`, `mt940.rs`, `mt535.rs`, `importers/` (11 600), `documents.rs` (2200), `connectors/html.rs` | Abstürze, Speicherverbrauch, falsche Beträge. Fuzzing gibt es bisher nicht |
| Portal-WebViews | `GiroCrabUI/PortalUI.swift` (1150), `ConsentBrowser.swift`, `AdvanziaScripts.swift`, die Portal-Ansichten (`*UI.swift`) mit ihren Skripten, `GiroCrabLinux/src/web_login.rs`, `GiroCrabLinux/qml/WebLoginSheet.qml` | Navigation ohne Host-Allowlist, Domain-Prüfung, eingefügte Skripte, Cookies außerhalb des Tresors |
| Abgleich | `vault/merge.rs`, `sync_merge.rs` und `sync_merge/` (3000), `GiroCrabUI/CloudSyncManager.swift` (640) | Veränderte oder alte Kopien, Rechte der Dateien, Inhalt der Kopien |
| Lokale Schnittstelle | `local_api.rs`, `local_api/server.rs` (zusammen 1700), `girocrab-mcp/` (660) | Rechte, Umfang der Freigabe, Umgehung der Maskierung |
| Signierte Dateien und Updates | `GiroCrabUI/BankingPolicy.swift` (680), `Announcements.swift`, `GiroCrabMac/Updater.swift`, `scripts/notarize.sh`, `scripts/make-appcast.sh` | Rückfall auf ältere Dateien, Schlüsselwechsel, Verhalten offline |
| Fehlerberichte | `report_sanitizer.rs` (480), `GiroCrabUI/BugReport.swift` | Umgehung der Bereinigung und Schwärzung |

### Priorität 3

- **Lieferkette:** GitHub-Workflows, Abhängigkeiten (gut 350 Pakete in `Cargo.lock`), Zugriff auf Release und Signaturschlüssel,
  siehe [#1799](https://github.com/GiroCrab/GiroCrab/issues/1799)–[#1805](https://github.com/GiroCrab/GiroCrab/issues/1805).
- **Linux- und Windows-Oberfläche:** Abweichungen von der Swift-App, unsignierte Pakete.
- **macOS ohne Sandbox:** Was der Direktdownload darf und was sich einschränken ließe.

## Nicht im Umfang

- Banken, Portale und Zahlungsdienste selbst. **Keine Tests gegen deren Systeme** und nicht mit fremden Konten.
- SQLCipher, OpenSSL, Sparkle und andere Bibliotheken; nur ihre Einbindung.
- Die Android-Alpha (öffnet keinen eigenen Tresor) und Nightly-Builds.
- Denial of Service, Social Engineering, ein bereits kompromittiertes entsperrtes System.
- Fachliche Richtigkeit einzelner Konnektoren, dafür gibt es eigene Issues („[Connector-Audit]“).

## Bekannte Punkte

Diese sind offen und sollen nicht als neue Funde zählen. Hinweise zur Schwere oder Ausnutzbarkeit sind willkommen.

| Issue | Thema |
|---|---|
| [#1794](https://github.com/GiroCrab/GiroCrab/issues/1794) | Weiterleitungen beim Dokumenten-Download von Trade Republic |
| [#1795](https://github.com/GiroCrab/GiroCrab/issues/1795) | Schutzklasse der Enable-Banking-Einträge im iCloud-Schlüsselbund |
| [#1796](https://github.com/GiroCrab/GiroCrab/issues/1796) | 3DES beim Lesen von FinTS-Schlüsseldateien |
| [#1797](https://github.com/GiroCrab/GiroCrab/issues/1797) | Navigation und Skripte der Portal-WebViews |
| [#1798](https://github.com/GiroCrab/GiroCrab/issues/1798) | Biometrie-Bindung entfällt mit iCloud-Schlüsselbund |

## Rahmen

- **Stand:** ein fester Commit oder Release-Tag, vor Beginn vereinbart.
- **Daten:** nur der Demo-Tresor (`--demo-seed`), erfundene Testdaten und Testserver. Keine echten Finanzdaten.
- **Meldung:** vertraulich und einzeln, nicht als öffentliches Issue. Dafür muss die private Meldung auf GitHub eingeschaltet
  sein oder ein anderer Weg feststehen.
- **Ergebnis:** Bericht mit Schwere, betroffenem Code, Nachweis und Vorschlag; Nachprüfung der Korrekturen;
  Veröffentlichung, sobald Korrekturen ausgeliefert sind.
- **Vorarbeit:** Was der Verification Plan als Lücke nennt, möglichst vorher schließen, zuerst #1794 und #1797.

Reicht das Budget nur für einen Teil: Priorität 1 ganz, danach Portal-WebViews und fremde Eingaben.

## Mögliche Wege

Stand der verlinkten Seiten am 24.11.2026. Ob GiroCrab in Frage kommt, ist nicht geklärt.

| Weg | Was die Seite sagt | Einschätzung |
|---|---|---|
| [Sovereign Tech Resilience](https://www.sovereign.tech/programs/bug-resilience) ([Kriterien](https://www.sovereign.tech/programs/bug-resilience/criteria)) | Audits für grundlegende Open-Source-Bausteine; bewertet Relevanz, Verbreitung, Verwundbarkeit; Anträge auf Deutsch oder Englisch, laufend geprüft | Für eine Endnutzer-App unsicher. Eher für einen Teil wie den FinTS-Kern, der dafür erst herausgelöst werden müsste |
| [NLnet](https://nlnet.nl/themes/) | Der Commons Fund hat [keine Aufrufe mehr](https://nlnet.nl/commonsfund). Andere Fonds sind offen, 5000 bis 50 000 €. [Audits](https://nlnet.nl/NGI0/services/) gibt es als Unterstützung für viele Geförderte, je nach Programm und Kapazität | Nur über ein gefördertes Vorhaben, dessen Thema passt |
| [OSTIF](https://ostif.org/get-an-audit/) | Begleitet Audits vom Scope bis zum Bericht und hilft bei Geldgebern; erste Audits etwa 30 000 bis 200 000 US-Dollar | Passt fachlich, braucht Finanzierung |
| Community-Review | – | Ohne Budget möglich, ersetzt kein Audit. Einzelne Zeilen aus Priorität 1 eignen sich als abgegrenzte Aufgaben |

**Von den Maintainern zu entscheiden:** Ansprechperson, Weg und Budget, vertraulicher Meldeweg, Zeitfenster und wer Funde behebt.
