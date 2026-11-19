# Android-Parität

## Purpose

Der Android-Client von GiroCrab (`GiroCrabAndroid/`, `de.girocrab.app`) zeigt die Banking-Oberfläche
in Jetpack Compose. Diese Fähigkeit beschreibt, was der Client **heute** kann: welche Daten er
darstellt, woher sie kommen und welche Teile der Anbindung an `girocrab-core` noch fehlen. Sie ist
der Ist-Stand für den Change `001-android-kern-paritaet`.

Maßstab ist der macOS-/iOS-Client: der Kern exportiert 91 Funktionen (`#[uniffi::export]`,
`girocrab-core/src`), Swift nutzt davon ~150 verschiedene `vault.`-Methoden, Android zwei.

## Requirements

### Requirement: Demo-Betrieb ohne nativen Kern

Der Client muss ohne geladene native Bibliothek vollständig bedienbar sein und dabei klar erkennbar
Demodaten zeigen.

- **Ablauf:** `AppViewModel` fragt beim Start `core.openOwnVault()`; ist das Ergebnis `null`, wird
  `core.demoSnapshot()` geladen (`GiroCrabAndroid/app/src/main/java/de/girocrab/app/model/AppViewModel.kt:69,86`).
- **Eingaben:** keine; Demodaten kommen aus Kotlin, nicht aus dem Kern (`model/CoreBridge.kt:92` ff.).
- **Ergebnis:** Zustand `VaultState.Demo`, Banner „Demo-Daten" über dem Buchungsstrom
  (`ui/screens/LockAndOnboarding.kt:176`), sechs realistische Konten über sieben Banken.
- **Architektur:** `DemoData` in Kotlin, `VaultSnapshot` als einziges Datenformat.

#### Scenario: Start ohne native Bibliothek

- **GIVEN** eine APK ohne `libgirocrab_core.so`
- **WHEN** die App startet
- **THEN** zeigt sie den Demo-Banner, gefüllte Konten, Buchungen und Analysen, und kein Bedienelement
  führt zu einem Absturz

#### Scenario: Demo überschreibt keine echten Daten

- **GIVEN** ein vorhandener eigener Tresor
- **WHEN** die Demo geöffnet wird
- **THEN** bleibt der eigene Stand im Speicher erhalten und ist nach dem Zurückschalten unverändert

### Requirement: Native Schicht und Bindings

Der Client muss den Rust-Kern als `.so` für die drei konfigurierten ABIs laden und die
UniFFI-Kotlin-Bindings einkompiliert haben.

- **Ablauf:** Gradle erwartet Bibliotheken unter `app/src/main/jniLibs` und Bindings unter
  `app/src/main/generated` (`GiroCrabAndroid/app/build.gradle.kts:37,39-40`), ABIs `arm64-v8a`,
  `armeabi-v7a`, `x86_64` (`:17`), JNA 5.15.0 für den Aufruf (`:62`).
- **Eingaben:** Ergebnis von `scripts/build-rust-android.sh` — dieses Skript existiert im Repo
  **nicht**; `app/src/main/jniLibs` fehlt ebenfalls.
- **Ergebnis:** heute keine native Schicht; `Class.forName("uniffi.girocrab_core.Vault")` schlägt fehl
  (`model/CoreBridge.kt:25`) und `nativeAvailable` bleibt `false`.
- **Architektur:** UniFFI + JNA (Reflection), kein eigener JNI-Code.

#### Scenario: Erkennung der fehlenden Schicht

- **GIVEN** eine APK ohne native Bibliothek
- **WHEN** `DefaultCoreBridge.nativeAvailable` ausgewertet wird
- **THEN** ist der Wert `false` und der Client bleibt im Demo-Pfad, ohne Fehlermeldung an den Nutzer

### Requirement: Tresor öffnen und entsperren

Der Client muss einen echten Tresor öffnen, entsperren und wieder sperren können.

- **Ablauf:** `AppViewModel` ruft `core.openOwnVault()` und übernimmt das Ergebnis über `adoptOwn()`.
- **Eingaben:** Tresorschlüssel aus der Plattform-Speicherung, Entsperrung per Biometrie oder
  Geräte-PIN.
- **Ergebnis:** heute nicht vorhanden — `DefaultCoreBridge.openOwnVault()` gibt immer `null` zurück,
  daneben steht `// TODO(parity)` (`model/CoreBridge.kt:28,30`).
- **Architektur:** Die Sperrbildschirm-Logik (`ui/screens/LockAndOnboarding.kt:61-92`) prüft heute nur
  die Biometrie; sie schaltet keinen Schlüssel frei.

#### Scenario: Entsperren führt zu echten Daten

- **GIVEN** ein eigener Tresor auf dem Gerät und ein erfolgreicher Biometrie-Check
- **WHEN** der Buchungsstrom geöffnet wird
- **THEN** stammen Konten und Buchungen aus dem Tresor und `VaultState.Own` ist gesetzt

#### Scenario: Abbruch der Entsperrung

- **GIVEN** ein gesperrter Client
- **WHEN** die Biometrie abgebrochen wird
- **THEN** bleibt der Client gesperrt und zeigt keine Tresordaten

### Requirement: Schlüsselablage auf Android

Der Tresorschlüssel muss über den Android-Keystore geschützt sein, nicht in einer Datei oder einem
Preference-Eintrag liegen.

- **Ablauf:** Schlüssel erzeugen/lesen, an Biometrie binden (`BiometricPrompt` mit `CryptoObject`).
- **Eingaben:** Gerätesperre, Keystore-Eintrag.
- **Ergebnis:** heute nicht vorhanden — im Kotlin-Bestand gibt es weder `AndroidKeyStore` noch
  `EncryptedSharedPreferences`; die App speichert nur die Tresordatei in `filesDir`
  (`MainActivity.kt:127`), die es ohne Kern aber nicht gibt.
- **Architektur:** `allowBackup="false"`, app-privater Speicher, minimale Berechtigungen
  (`AndroidManifest.xml`).

#### Scenario: Kein Schlüsselmaterial in Dateien

- **GIVEN** ein eingerichteter Android-Tresor
- **WHEN** das App-Datenverzeichnis kopiert wird
- **THEN** enthält die Kopie keinen verwendbaren Tresorschlüssel

### Requirement: Datenabbildung zwischen Kern und Oberfläche

Der Client muss die Kern-Daten in `VaultSnapshot` abbilden, ohne die Oberfläche umzubauen.

- **Ablauf:** Mapping der 91 Kernfunktionen auf die 12 Listen in `VaultSnapshot`: Gruppen, Konten,
  Buchungen, Kategorien, Tags, Regeln, Daueraufträge, Mandate, Holdings, Kredite, Sachwerte,
  Ankündigungen (`model/Models.kt:209-222`).
- **Eingaben:** `Vault`-Objekt aus den UniFFI-Bindings.
- **Ergebnis:** heute vollständig offen (`TODO(parity)`); die sieben Screens lesen ausschließlich den
  Demo-Snapshot.
- **Architektur:** Ein Snapshot pro Reload; Paging der Buchungsliste (`listTransactions` im Kern) ist
  nicht angebunden.

#### Scenario: Lesepfad gegen echten Tresor

- **GIVEN** ein geöffneter Tresor mit Buchungen
- **WHEN** der Buchungsstrom und die Analyse neu geladen werden
- **THEN** stimmen Anzahl und Summen mit dem Tresor überein (Gegenprobe über die Konsole des
  Desktop-Clients)

### Requirement: Schreibpfade

Der Client muss Änderungen über den Kern schreiben, nicht nur im Speicher.

- **Ablauf:** Zahlungen, Kategorien, Regeln, Notizen und Einstellungen laufen über die Kern-API.
- **Eingaben:** Nutzereingaben der sieben Screens.
- **Ergebnis:** heute in-memory — `AppViewModel` „bucht" eine Überweisung, indem es den Snapshot
  verändert (`model/AppViewModel.kt:139`); nach einem Neustart ist die Änderung weg (Demo-Banner sagt
  das ausdrücklich).
- **Architektur:** keine Transaktionsgrenzen, keine Fehlerrückmeldung aus dem Kern.

#### Scenario: Neustart nach einer Demo-Überweisung

- **GIVEN** eine im Demo-Modus gebuchte Überweisung
- **WHEN** die App beendet und neu gestartet wird
- **THEN** ist die Buchung verschwunden und der Hinweis im Demo-Banner bleibt korrekt
