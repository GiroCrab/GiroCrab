## MODIFIED Requirements

### Requirement: Native Schicht und Bindings

Der Client muss den Rust-Kern als `.so` für die drei konfigurierten ABIs laden und die
UniFFI-Kotlin-Bindings aus dem Kern erzeugen, und zwar in einem Schritt, den die CI selbst ausführt.

- **Ablauf:** `scripts/build-rust-android.sh` erzeugt per `cargo ndk` die drei `.so` nach
  `app/src/main/jniLibs/<abi>/` und ruft `uniffi-bindgen generate --language kotlin` nach
  `app/src/main/generated/`; Gradle konsumiert beides unverändert (`build.gradle.kts:37,39-40`).
- **Eingaben:** gepinnte Rust-Toolchain (`scripts/rust-toolchain.sh`), NDK aus dem Android-SDK,
  `cargo-ndk`, `uniffi-bindgen` in der Workspace-Version (`uniffi 0.32`).
- **Ergebnis:** `nativeAvailable` ist auf einem echten Gerät `true`; die Debug-APK enthält die `.so`
  für `arm64-v8a`, `armeabi-v7a`, `x86_64`.
- **Architektur:** UniFFI + JNA bleibt; kein handgeschriebener JNI-Code. Die CI baut die native
  Schicht vor `gradle assembleDebug` und schlägt fehl, wenn `jniLibs` danach leer ist.

#### Scenario: CI baut die native Schicht mit

- **GIVEN** ein Push, der `GiroCrabAndroid/**` berührt
- **WHEN** der Workflow `android.yml` läuft
- **THEN** enthält das erzeugte APK drei `.so`-Dateien und die Prüfung „`unzip -l` zählt
  `lib/*/libgirocrab_core.so`" ist grün

#### Scenario: Fehlende native Schicht bricht den Build

- **GIVEN** ein Build, in dem `cargo ndk` fehlschlägt
- **WHEN** Gradle die APK baut
- **THEN** bricht der Lauf ab, statt eine APK zu erzeugen, die still nur Demo-Daten anzeigt

#### Scenario: Erkennung der fehlenden Schicht

- **GIVEN** eine APK, in der `libgirocrab_core.so` für die Geräte-ABI fehlt
- **WHEN** der Client den Kern laden will
- **THEN** meldet er die fehlende native Schicht sichtbar und bleibt im Demo-Pfad, statt den Fehler
  zu verschlucken

### Requirement: Tresor öffnen und entsperren

Der Client muss einen echten Tresor öffnen, entsperren und wieder sperren können, mit demselben
Lebenszyklus wie der Desktop-Client.

- **Ablauf:** `CoreBridge.openOwnVault()` bildet das UniFFI-`Vault` auf den Client ab
  (`CoreBridge.kt:28`); `AppViewModel` übernimmt ihn über `adoptOwn()`
  (`AppViewModel.kt:69`), `VaultState.Own` ersetzt `VaultState.Demo`.
- **Eingaben:** Tresorschlüssel aus dem Keystore, Biometrie oder Geräte-PIN als Freigabe.
- **Ergebnis:** echte Konten und Buchungen; Sperren beim Wechsel in den Hintergrund.
- **Architektur:** Ein `Vault`-Objekt pro Prozess, Öffnen im Hintergrund-Thread
  (`Dispatchers.Default`), Fehler als sichtbare Meldung statt stillem Demo-Fallback.

#### Scenario: Entsperren zeigt Tresordaten

- **GIVEN** ein eigener Tresor und eine erfolgreiche Biometrie
- **WHEN** der Buchungsstrom erscheint
- **THEN** ist `VaultState.Own` gesetzt, der Demo-Banner fehlt, und die Buchungszahl entspricht dem
  Tresor

#### Scenario: Fehler beim Öffnen wird sichtbar

- **GIVEN** ein Tresor, dessen Schlüssel nicht mehr passt
- **WHEN** die Entsperrung versucht wird
- **THEN** erscheint eine Fehlermeldung mit Ausweg (Schlüssel neu importieren), und die App fällt
  nicht wortlos auf Demo-Daten zurück

#### Scenario: Entsperren führt zu echten Daten

- **GIVEN** ein eigener Tresor auf dem Gerät und ein erfolgreicher Biometrie-Check
- **WHEN** der Buchungsstrom geöffnet wird
- **THEN** stammen Konten und Buchungen aus dem Tresor und `VaultState.Own` ist gesetzt

#### Scenario: Abbruch der Entsperrung

- **GIVEN** ein gesperrter Client
- **WHEN** die Biometrie abgebrochen wird
- **THEN** bleibt der Client gesperrt und zeigt keine Tresordaten

### Requirement: Schlüsselablage auf Android

Der Tresorschlüssel muss im Android-Keystore liegen und für das Entsperren an die Biometrie gebunden
sein.

- **Ablauf:** Ein AES-Schlüssel im `AndroidKeyStore` (`setUserAuthenticationRequired(true)`), der den
  Tresorschlüssel umhüllt; `BiometricPrompt` mit `CryptoObject` entsperrt ihn. Kein Klartext in
  `SharedPreferences` oder `filesDir`.
- **Eingaben:** Gerätesperre, Keystore-Eintrag, optional StrongBox.
- **Ergebnis:** Einrichtung beim ersten Start, Migration bei Gerätewechsel über den Export des
  Tresorschlüssels (derselbe Weg wie auf dem Desktop).
- **Architektur:** `allowBackup="false"` bleibt; jede unverschlüsselte Kopie des Schlüssels ist ein
  Fehler im Test `keystore-no-plaintext`.

#### Scenario: Keystore statt Datei

- **GIVEN** ein eingerichteter Android-Tresor
- **WHEN** das App-Datenverzeichnis kopiert und nach dem Schlüssel durchsucht wird
- **THEN** ist dort kein verwendbarer Schlüssel, und ohne Biometrie lässt sich der Tresor auf dem
  kopierten Stand nicht öffnen

#### Scenario: Biometrie-Sperre schützt den Schlüssel

- **GIVEN** ein Gerät mit hinterlegter Biometrie
- **WHEN** die Biometrie abgelehnt wird
- **THEN** bleibt der Tresor geschlossen, auch wenn die App wieder in den Vordergrund kommt

#### Scenario: Kein Schlüsselmaterial in Dateien

- **GIVEN** ein eingerichteter Android-Tresor
- **WHEN** das App-Datenverzeichnis kopiert wird
- **THEN** enthält die Kopie keinen verwendbaren Tresorschlüssel

### Requirement: Datenabbildung zwischen Kern und Oberfläche

Der Client muss die Kern-Daten in `VaultSnapshot` abbilden und die Buchungsliste seitenweise laden.

- **Ablauf:** Mapper für die 12 Listen in `VaultSnapshot` (`Models.kt:209-222`), beginnend mit dem
  Lesepfad: Konten, Buchungen, Kategorien, Analyse-Summen. Paging über `listTransactions` mit Cursor
  statt „alles laden".
- **Eingaben:** `Vault`-Objekt aus den Bindings.
- **Ergebnis:** Sieben Screens zeigen Tresordaten; die Analyse-Summen stimmen mit dem Desktop-Client
  überein.
- **Architektur:** Ein Snapshot je Reload, Buchungen separat nachgeladen; keine Geschäftslogik in
  Kotlin, die der Kern schon hat (Kategorisierung, Regelanwendung, Saldenhistorie).

#### Scenario: Buchungsliste mit Paging

- **GIVEN** ein Tresor mit 5.000 Buchungen
- **WHEN** der Buchungsstrom bis zum Ende gescrollt wird
- **THEN** werden Seiten nachgeladen, ohne dass der Speicher über die erste Seite hinaus wächst, und
  die letzte Seite endet mit der ältesten Buchung

#### Scenario: Summen stimmen mit dem Desktop überein

- **GIVEN** derselbe Tresor auf Android und auf dem Desktop
- **WHEN** beide die Ausgaben eines Monats anzeigen
- **THEN** sind die Summen identisch

#### Scenario: Lesepfad gegen echten Tresor

- **GIVEN** ein geöffneter Tresor mit Buchungen
- **WHEN** der Buchungsstrom und die Analyse neu geladen werden
- **THEN** stimmen Anzahl und Summen mit dem Tresor überein (Gegenprobe über die Konsole des
  Desktop-Clients)

### Requirement: Schreibpfade

Der Client muss Änderungen über den Kern schreiben.

- **Ablauf:** Überweisung anlegen, Kategorie ändern, Regel speichern, Notiz setzen, Einstellung
  ändern — jeweils über die Kern-API, mit Rückmeldung und Fehlerbehandlung.
- **Eingaben:** Nutzereingaben der sieben Screens.
- **Ergebnis:** Änderungen überleben den Neustart; die Demo bleibt die einzige Ausnahme und ist als
  solche gekennzeichnet.
- **Architektur:** Kein Schreiben in den Snapshot; der Snapshot wird nach jeder Mutation neu geladen
  (wie `reload()` in `AppModel+Loading.swift` auf macOS).

#### Scenario: Überweisung überlebt den Neustart

- **GIVEN** ein entsperrter Tresor
- **WHEN** eine Überweisung gebucht und die App neu gestartet wird
- **THEN** steht die Buchung im Buchungsstrom und der Kontostand ist um den Betrag verringert

#### Scenario: Fehler der Bank wird angezeigt

- **GIVEN** ein Bankzugang, dessen TAN-Abfrage abbricht
- **WHEN** die Überweisung abgeschickt wird
- **THEN** erscheint die Fehlermeldung des Kerns und die Buchung wird nicht als ausgeführt angezeigt

#### Scenario: Neustart nach einer Demo-Überweisung

- **GIVEN** eine im Demo-Modus gebuchte Überweisung
- **WHEN** die App beendet und neu gestartet wird
- **THEN** ist die Buchung verschwunden und der Hinweis im Demo-Banner bleibt korrekt

## ADDED Requirements

### Requirement: Parität in Stufen mit Abnahmegate

Der Android-Client muss in vier Stufen ausgeliefert werden, jede mit einem prüfbaren Gate, statt
„Android wie macOS" als ein Ziel zu behandeln.

- **Ablauf:** Stufe 1 native Schicht + Lesepfad; Stufe 2 Schreibpfade (Überweisung, Kategorien,
  Regeln, Einstellungen); Stufe 3 Konten anbinden (FinTS/PSD2 über die bestehenden Konnektoren);
  Stufe 4 Import/Export, Dokumente, Daueraufträge, Depot.
- **Eingaben:** der Bestand an Kernfunktionen (91 Exporte).
- **Ergebnis:** Nach jeder Stufe ist die App benutzbar; die nächste Stufe ist eine eigene
  Auslieferung und kein Big Bang.
- **Architektur:** Alpha-Kennzeichnung bleibt bis Stufe 2 abgeschlossen ist; die
  `versionName`-Linie bleibt von der Desktop-Version getrennt (heute 0.1.0 gegen 0.8.6).

#### Scenario: Stufe 1 ist ohne Schreibzugriff nützlich

- **GIVEN** Stufe 1 ist ausgeliefert
- **WHEN** ein Nutzer seinen Tresor öffnet
- **THEN** kann er Konten, Buchungen und Analysen lesen, und jeder Schreibversuch ist sichtbar als
  „in Arbeit" gekennzeichnet statt still zu scheitern

#### Scenario: Kein Rückschritt der Alpha-Kennzeichnung

- **GIVEN** Stufe 2 ist noch nicht fertig
- **WHEN** eine APK veröffentlicht wird
- **THEN** trägt sie weiter den Alpha-Hinweis im README und in der App
