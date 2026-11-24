# Sicherheit / Security

[Deutsch](#deutsch) · [English](#english)

## Deutsch

### Sicherheitslücken melden

Bitte keine öffentlichen Issues für Schwachstellen. Nutze auf GitHub „Security › Report a vulnerability“
(privater Security Advisory) im Repository `GiroCrab/GiroCrab`. Bitte keine echten Finanzdaten, IBANs oder
Zugangsdaten mitschicken.

### Architektur im Überblick

GiroCrab ist eine lokale Anwendung: Ein Rust-Kern (`girocrab-core`) hält alle Daten, die Oberflächen
(SwiftUI, Qt, Slint, …) zeigen sie an. Es gibt kein GiroCrab-Konto und keinen GiroCrab-Server, der Finanzdaten sieht.
Dieses Dokument beschreibt den Stand des Branches `main`; ein externes Audit gab es bisher nicht.

| Bereich | Maßnahme |
|---|---|
| Daten in Ruhe | Der Tresor ist eine **SQLCipher-4**-Datenbank (AES-256). Speicherschutz (`cipher_memory_security`) fordert der Kern beim Öffnen an (nicht in Unit-Tests), prüft das Ergebnis aber nicht: Der Tresor öffnet auch ohne ihn. |
| Tresorschlüssel | Zufällige 32 Byte im **Schlüsselbund** (macOS/iOS), **Secret Service** (Linux) oder **Windows Credential Manager**. Einzelheiten unter „Schlüssel“. |
| Zugangsdaten | Gespeicherte PINs, Portal-Passwörter, API-Schlüssel und Sitzungen liegen **im verschlüsselten Tresor**, nicht einzeln im Schlüsselbund (Ausnahmen unter „Schlüssel“). Eine PIN zu speichern ist freiwillig. |
| Geheimnisse im Speicher | Der Rust-Kern hält Schlüssel und Geheimnisse an den vorgesehenen Stellen in `Zeroizing`-Containern, die beim Freigeben überschrieben werden. Für Zeichenketten der Oberflächen und der WebViews gilt das nicht. |
| Netzwerk | Verbindungen zu Banken nutzen **TLS mit Zertifikatsprüfung**, ohne Option zum Abschalten. Ausnahme: ein eigener Bitcoin-Core-Knoten, per `http` nur auf dem eigenen Rechner. Weiterleitungen siehe „Netzwerk“. |
| Oberfläche | `.sensitive()` verdeckt Personenbezogenes, Beträge und Bezeichnungen (Datenschutzmodus, Fehlerbericht-Screenshots). Texte für öffentliche Fehlerberichte laufen durch `sanitize_report_text`. |
| Lokale Schnittstelle | Nur macOS, standardmäßig aus, nur lesend. Siehe „Lokale Schnittstelle“. |
| Updates | macOS-Direktdownload: signierter Feed, signiertes Disk-Image und Developer-ID-Prüfung, siehe [`docs/auto-updates.md`](docs/auto-updates.md) und [`docs/release-integrity.md`](docs/release-integrity.md). Windows: nur ein Hinweis, der Download öffnet sich im Browser und ist nicht signiert ([`docs/windows.md`](docs/windows.md)). |

### Schlüssel

- **Tresorschlüssel:** Er wird als Rohschlüssel an SQLCipher übergeben. Es gibt keine Ableitung aus einem Passwort, weil
  der Schlüssel zufällig ist. macOS/iOS speichern ihn als `WhenUnlockedThisDeviceOnly` mit `userPresence`
  (Biometrie oder Gerätecode; auf dem Mac abschaltbar). Unsignierte Mac-Builds nutzen den Anmelde-Schlüsselbund, dort
  prüft nur die App die Biometrie. Unter Linux und Windows schützt die Anmeldung bzw. der Schlüsselbund des Systems.
- **iCloud-Schlüsselbund (Opt-in):** Synchronisierte Einträge können keine Biometrie-Bindung tragen. Der Tresorschlüssel
  liegt dann als `WhenUnlocked` im iCloud-Schlüsselbund; die Biometrie-Abfrage ist nur noch eine Prüfung der App
  ([#1798](https://github.com/GiroCrab/GiroCrab/issues/1798)). Mit demselben Opt-in werden eigene
  Enable-Banking-Anwendungen (ID und privater Schlüssel) als `AfterFirstUnlock` geteilt
  ([#1795](https://github.com/GiroCrab/GiroCrab/issues/1795)).
- **Weitere Schlüsselbund-Einträge (Apple):** das gespeicherte Passwort des Advanzia-Portals und der Stand der
  Sicherheitssperre, beide nur auf diesem Gerät.
- **Sicherungen mit Passwort:** SQLCipher leitet den Schlüssel per PBKDF2 aus dem Passwort ab (mindestens 8 Zeichen). Die
  Sicherung ist nur so stark wie das Passwort. Beim Wiederherstellen werden gespeicherte PINs und FinTS-Adressen aus der
  Sicherung nicht übernommen.
- **Abgleich über iCloud Drive oder einen Ordner (Opt-in):** Die Kopien sind mit dem Tresorschlüssel verschlüsselt und
  enthalten den ganzen Tresor, also auch gespeicherte PINs.
- **3DES:** nur zum Lesen externer FinTS-Schlüsseldateien (Profil RDH-10), nie für den Tresor
  ([#1796](https://github.com/GiroCrab/GiroCrab/issues/1796)).
- **Android:** Der Alpha-Build öffnet noch keinen eigenen Tresor.

### Netzwerk

Konnektoren, die Zugangsdaten senden, folgen Weiterleitungen nur bei gleichem Schema, Host und Port, FinTS folgt gar
keinen. Es gibt diese Ausnahmen:

- Der Dokumenten-Download bei Trade Republic folgt auch fremden Hosts, weil Dokumente auf Speicher-Hosts liegen können
  ([#1794](https://github.com/GiroCrab/GiroCrab/issues/1794)).
- Die Bankenliste von Enable Banking (mit einem eine Stunde gültigen Token) und der Bitcoin-Kurs (ohne Zugangsdaten) nutzen die
  Standardrichtlinie der HTTP-Bibliothek.
- WebViews folgen der Seite, siehe „Portal-WebViews“.

Neben den Endpunkten, die du selbst einrichtest, kann GiroCrab diese Adressen abrufen:

| Ziel | Zweck | Wann |
|---|---|---|
| `raw.githubusercontent.com` | signierte Sicherheitssperre | macOS/iOS/Linux: beim Start und vor Bankkontakten, höchstens alle 15 Minuten |
| `raw.githubusercontent.com` | signierte Mitteilungen | macOS/iOS/Linux: höchstens alle 6 Stunden; abschaltbar (Standard: an) |
| `github.com` | Update-Feed und Updates | nur macOS-Direktdownload |
| `api.github.com` | Liste der Releases | nur Windows: auf Knopfdruck oder, falls eingeschaltet, höchstens einmal am Tag (Standard: aus) |
| `api.frankfurter.dev` | Goldpreis | nur mit automatisch bewerteten Sachwerten; übertragen werden Metall und Zeitraum |
| `api.kraken.com` | Bitcoin-Kurs | nur mit Bitcoin-Core-Konto |
| `icons.duckduckgo.com` oder `www.google.com` | Händler-Logos | nur nach Auswahl (Standard: aus); übertragen wird die Händler-Domain |

Wie bei jeder Verbindung sieht das Ziel deine IP-Adresse.

### Portal-WebViews

Einige Zugänge (z. B. American Express, Amazon, FNZ) melden sich in einem eingebetteten Browser an. Er gehört nicht zum
Rust-Kern und unterliegt nicht dessen Lints.

- Der Hauptrahmen der Portal-Fenster lädt beliebige `https`-Seiten (Bank-Logins wechseln die Domain), aber kein `http`, `file`,
  `javascript`, `data` und keine App-Schemata. Eine Host-Allowlist für die Navigation gibt es nicht.
  Die Fenster für die PSD2-Zustimmung und den Trade-Republic-Login haben unter macOS/iOS keine eigene Navigationsregel.
- Mitgelesene Antworten nimmt die App unter macOS/iOS nur vom Hauptrahmen einer `https`-Seite auf einer Domain des Portals an. Antworten
  von Profil- und Anmelde-Endpunkten gehen nicht an den Import.
- Gespeicherte Passwörter füllt die App unter macOS/iOS nur auf den Domains des Portals bzw. seiner Anmeldeseite ein.
- macOS/iOS: Cookies liegen im WebView-Speicher des Systems und damit **außerhalb des Tresors**, sie überstehen einen
  Neustart. Linux nutzt ein flüchtiges Profil (`offTheRecord`) und legt Portal-Cookies je Zugang im Tresor ab.
- Offen: Prüfung aller Portal-Skripte und ein automatischer Check ([#1797](https://github.com/GiroCrab/GiroCrab/issues/1797)).

### Lokale Schnittstelle

[`docs/local-api.md`](docs/local-api.md): Unix-Socket ohne Netzwerkport (Ordner `0700`, Socket `0600`, andere Benutzer werden
abgewiesen), nur bei entsperrtem Tresor. Vom Zugangsschlüssel wird nur der Hash gespeichert. Konten und Felder werden
einzeln freigegeben; IBANs und Notizen sind standardmäßig aus. Ein Programm mit Zugriff (z. B. ein KI-Assistent) kann
die freigegebenen Daten an Dritte weitergeben.

### Was das Gerät verlässt

- Buchungen, Konten, Kategorien, Notizen und Dokumente verlassen das Gerät nur auf dein Zutun: Abruf oder Überweisung bei
  deiner Bank, Export, freigegebene Schnittstelle, Abgleich über iCloud Drive oder einen Ordner.
- Keine Telemetrie, keine Analyse-Dienste, keine Werbe-Kennungen.
- Fehlerberichte werden erst nach deiner Prüfung und bereinigt veröffentlicht.
- Exporte und der optionale Dokumentenordner sind unverschlüsselte Dateien.

### Bedrohungsmodell

**Geschützt gegen**
- Verlust oder Diebstahl des gesperrten Geräts bzw. eine Kopie der Tresordatei ohne den Schlüssel.
- Mitlesen oder Umleiten des Netzwerkverkehrs (TLS, Regeln für Weiterleitungen mit den genannten Ausnahmen).
- Veränderte Updates, Sicherheitssperren und Mitteilungen (Signaturen), im Rahmen der verlinkten Dokumente.
- Versehentliches Offenlegen in Screenshots, Logs und Fehlerberichten (`.sensitive()`, Sanitizer).

**Nur gemindert**
- Reste von Geheimnissen im Arbeitsspeicher, in Swap oder Core-Dumps (Zeroizing im Kern, SQLCipher-Speicherschutz, soweit aktiv).
- Passwort-Raten gegen eine entwendete Sicherung (PBKDF2, Mindestlänge).

**Nicht geschützt gegen (außerhalb des Modells)**
- Ein bereits kompromittiertes, entsperrtes System (Malware mit Zugriff auf Prozessspeicher oder Bildschirm).
- Eine kompromittierte Bank, ein kompromittiertes Portal oder gefälschte Zertifizierungsstellen im Systemspeicher.
- Physischer Zugriff auf ein entsperrtes Gerät, schwache Geräte-Passwörter.
- Ein übernommenes Apple-Konto, wenn der iCloud-Schlüsselbund den Tresorschlüssel teilt.
- Eine vom Nutzer freigegebene lokale Schnittstelle: Wer das Token hat, liest im vergebenen Umfang.

### Abhängigkeiten

Eingesetzte Bibliotheken und Lizenzen: [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).

---

## English

### Reporting a vulnerability

Please do not open public issues for vulnerabilities. Use GitHub “Security › Report a vulnerability” (private
security advisory) in `GiroCrab/GiroCrab`. Do not include real financial data, IBANs or credentials.

### Architecture overview

GiroCrab is a local application: a Rust core (`girocrab-core`) holds all data; the UIs (SwiftUI, Qt, Slint, …) display
it. There is no GiroCrab account and no GiroCrab server that sees financial data. This document describes the state of
the `main` branch; there has been no external audit so far.

| Area | Measure |
|---|---|
| Data at rest | The vault is a **SQLCipher 4** database (AES-256). The core requests memory security (`cipher_memory_security`) on opening (not in unit tests) but does not check the result: the vault also opens without it. |
| Vault key | 32 random bytes in the **Keychain** (macOS/iOS), **Secret Service** (Linux) or **Windows Credential Manager**. Details under “Keys”. |
| Credentials | Stored PINs, portal passwords, API keys and sessions are kept **in the encrypted vault**, not one by one in the keychain (exceptions under “Keys”). Storing a PIN is optional. |
| Secrets in memory | The Rust core holds keys and secrets in `Zeroizing` containers where intended, overwritten on drop. This does not apply to strings of the UIs and web views. |
| Network | Bank connections use **TLS with certificate validation**, with no opt-out. Exception: your own Bitcoin Core node, over `http` on the same computer only. For redirects see “Network”. |
| UI | `.sensitive()` covers personal data, amounts and labels (privacy mode, bug-report screenshots). Text for public bug reports passes through `sanitize_report_text`. |
| Local interface | macOS only, off by default, read-only. See “Local interface”. |
| Updates | macOS direct download: signed feed, signed disk image and Developer ID check, see [`docs/auto-updates.md`](docs/auto-updates.md) and [`docs/release-integrity.md`](docs/release-integrity.md) (German). Windows: a notice only; the download opens in the browser and is not signed ([`docs/windows.md`](docs/windows.md), German). |

### Keys

- **Vault key:** passed to SQLCipher as a raw key. It is not derived from a password, because it is random. macOS/iOS store
  it as `WhenUnlockedThisDeviceOnly` with `userPresence` (biometrics or device passcode; can be turned off on the Mac).
  Unsigned Mac builds use the login keychain, where only the app checks biometrics. On Linux and Windows the system
  login or keyring protects it.
- **iCloud Keychain (opt-in):** synchronised items cannot carry a biometric binding. The vault key is then kept as
  `WhenUnlocked` in iCloud Keychain, and the biometric prompt is only a check by the app
  ([#1798](https://github.com/GiroCrab/GiroCrab/issues/1798)). The same opt-in shares your own Enable Banking
  applications (ID and private key) as `AfterFirstUnlock`
  ([#1795](https://github.com/GiroCrab/GiroCrab/issues/1795)).
- **Other keychain items (Apple):** the stored password of the Advanzia portal and the state of the security lock, both on
  this device only.
- **Password backups:** SQLCipher derives the key from the password with PBKDF2 (at least 8 characters). A backup is only
  as strong as its password. Restoring one does not take over its stored PINs and FinTS addresses.
- **Sync through iCloud Drive or a folder (opt-in):** the copies are encrypted with the vault key and contain the whole
  vault, including stored PINs.
- **3DES:** only for reading external FinTS key files (profile RDH-10), never for the vault
  ([#1796](https://github.com/GiroCrab/GiroCrab/issues/1796)).
- **Android:** the alpha build does not open a vault of its own yet.

### Network

Connectors that send credentials follow redirects only with the same scheme, host and port; FinTS follows none. The
exceptions are:

- Trade Republic document downloads also follow other hosts, because documents may be on storage hosts
  ([#1794](https://github.com/GiroCrab/GiroCrab/issues/1794)).
- The Enable Banking bank list (with a token valid for one hour) and the Bitcoin price (no credentials) use the HTTP library's
  default policy.
- Web views follow the page, see “Portal web views”.

Besides the endpoints you set up yourself, GiroCrab can contact:

| Target | Purpose | When |
|---|---|---|
| `raw.githubusercontent.com` | signed security lock | macOS/iOS/Linux: at start and before contacting a bank, at most every 15 minutes |
| `raw.githubusercontent.com` | signed announcements | macOS/iOS/Linux: at most every 6 hours; can be turned off (default: on) |
| `github.com` | update feed and updates | macOS direct download only |
| `api.github.com` | list of releases | Windows only: on request or, if switched on, at most once a day (default: off) |
| `api.frankfurter.dev` | gold price | only with automatically valued tangible assets; sends the metal and date range |
| `api.kraken.com` | Bitcoin price | only with a Bitcoin Core account |
| `icons.duckduckgo.com` or `www.google.com` | merchant logos | only if chosen (default: off); sends the merchant domain |

As with any connection, the target sees your IP address.

### Portal web views

Some logins (e.g. American Express, Amazon, FNZ) run in an embedded browser. It is not part of the Rust core and not
covered by its lints.

- The main frame of the portal windows loads any `https` page (bank logins change domains), but no `http`, `file`, `javascript`, `data` or app
  schemes. There is no host allowlist for navigation.
  On macOS/iOS the windows for PSD2 consent and the Trade Republic login have no navigation rule of their own.
- On macOS/iOS the app accepts captured responses only from the main frame of an `https` page on one of the portal's domains. Responses
  of profile and login endpoints do not reach the import.
- On macOS/iOS stored passwords are filled in only on the domains of the portal or its login page.
- macOS/iOS: cookies are kept in the system's web view storage and thus **outside the vault**; they survive a restart.
  Linux uses a transient profile (`offTheRecord`) and keeps portal cookies per login in the vault.
- Open: review of all portal scripts and an automated check ([#1797](https://github.com/GiroCrab/GiroCrab/issues/1797)).

### Local interface

[`docs/local-api.md`](docs/local-api.md) (German): a Unix socket with no network port (folder `0700`, socket `0600`, other
users are refused), only while the vault is unlocked. Only a hash of the access key is stored. Accounts and fields are
shared individually; IBANs and notes are off by default. A program with access (e.g. an AI assistant) can pass the
shared data on to third parties.

### What leaves the device

- Transactions, accounts, categories, notes and documents leave the device only when you act: fetching or transferring at
  your bank, export, a shared interface, sync through iCloud Drive or a folder.
- No telemetry, no analytics services, no advertising identifiers.
- Bug reports are published only after your review and sanitised.
- Exports and the optional document folder are unencrypted files.

### Threat model

**Protected against**
- Loss or theft of the locked device, or a copy of the vault file without the key.
- Eavesdropping on or redirecting network traffic (TLS, redirect rules with the exceptions above).
- Tampered updates, security locks and announcements (signatures), within the limits of the linked documents.
- Accidental disclosure in screenshots, logs and bug reports (`.sensitive()`, sanitizer).

**Only mitigated**
- Leftover secrets in memory, swap or core dumps (Zeroizing in the core, SQLCipher memory security where active).
- Password guessing against a stolen backup (PBKDF2, minimum length).

**Out of scope**
- An already compromised, unlocked system (malware with access to process memory or the screen).
- A compromised bank or portal, or rogue certificate authorities in the system trust store.
- Physical access to an unlocked device, weak device passwords.
- A taken-over Apple account, if iCloud Keychain shares the vault key.
- A local interface the user enabled: whoever holds the token reads within the granted scope.

### Dependencies

Libraries and licenses: [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
