# Android-Parität

Stand der Android-App (`GiroCrabAndroid/`, `de.girocrab.app`) gegenüber dem Rust-Kern
(`girocrab-core/`) — was heute läuft, was fehlt und in welcher Reihenfolge es dazukommt.

Die Seite ist der dauerhafte Ort für diesen Plan; die einzelnen Arbeitsblöcke stehen als
übernehmbare Issues im Tracking-Issue **#1431**. Zahlen unten wurden gegen `main` (`364a184`,
24.11.2026) erhoben, Android-`versionName` ist 0.1.0; nach Umbauten im Client bitte nachziehen.

## Kurzfassung

Seit dem Sammel-MR #1688 hängt die App am Kern. Sie öffnet einen echten Tresor — ausschließlich
lesend — und zeigt acht Compose-Oberflächen (sieben Screens plus Buchungsdetail) mit Daten aus
`girocrab-core`.

- **Native Schicht.** `GiroCrabAndroid/scripts/build-rust-android.sh` übersetzt den Kern mit
  `cargo-ndk` für `arm64-v8a`, `armeabi-v7a` und `x86_64` und erzeugt die Kotlin-Bindings (UniFFI
  über JNA). `jniLibs` und `generated` werden gebaut, nicht eingecheckt. Die CI ruft dasselbe
  Skript auf und prüft, dass die gebaute APK `.so`-Dateien für alle drei ABIs enthält.
- **Tresor.** Die App legt keinen eigenen Tresor an: Sie spielt eine passwortgeschützte
  **Desktop-Sicherung** ein (SAF-Auswahl), erzeugt dabei einen eigenen 32-Byte-Schlüssel, umhüllt
  ihn AES-GCM im `AndroidKeyStore` und entsperrt über `BiometricPrompt` mit `CryptoObject`.
- **Datenabbildung.** `VaultSnapshotMapper` übersetzt Gruppen, Konten, Buchungen, Kategorien, Tags,
  Holdings und Sachwerte in die App-Modelle. Buchungen kommen als eine Seite à 1.000; eine längere
  Liste zeigt sichtbar „erste X von Y".
- **Offen.** Schreibpfade (Stufe 2, #1430): Eigene Bearbeitungen im `Own`-Zustand liegen nur im
  Snapshot und überleben keinen harten Lock. Dazu 3.2b (Regeln, Daueraufträge, Mandate, Kredite), die Ankündigungen aus der
  signierten Policy-Datei, 3.3 Cursor-Paging, 3.4 Analysesummen und das Lesen des Snapshots außerhalb des
  Main-Threads.
- **Alpha.** Debug-APK als CI-Artefakt; die Release-APK baut derselbe Job nur als
  R8-Smoke-Test und lädt sie nicht hoch. 58 instrumentierte Tests im Job `android-instrumented`. Das Alpha-Label fällt nach Stufe 2.

## Ist-Stand im Einzelnen

**Native Schicht.** Das Build-Skript baut standardmäßig drei ABIs im Release-Profil; `--debug`,
`--abi`, `--bindings-only` und `--check` decken den Entwicklungsalltag ab. Die Kotlin-Bindings
entstehen aus einem Debug-Build des Hosts, nicht aus der Android-Bibliothek: `profile.release` setzt
`strip = "symbols"`, damit findet `uniffi-bindgen` keine Metadaten in der `.so`. Die Metadaten sind
quellabgeleitet, die Bindings gehören also zur Release-Bibliothek. Gradle liest `src/main/jniLibs`
und `src/main/generated` (`app/build.gradle.kts:38-40`), die ABIs stehen in `abiFilters` (`:17`), JNA
kommt als `aar` (`:63`).

**Schlüssel und Entsperren.** `VaultKeyStore` umhüllt die 32 Byte mit einem AES-GCM-Schlüssel aus
dem `AndroidKeyStore` (`setUserAuthenticationRequired(true)`,
`setInvalidatedByBiometricEnrollment(true)`). `VaultUnlocker` liefert ein `UnlockResult`
(`NoVaultOnDevice`, `KeyInvalidated`, `VaultMissing`, `Unavailable`, `Refused`, `SoftReleased`),
genau einmal je Anfrage; der Prompt ist strikt biometrisch. `VaultMissing` und `KeyInvalidated`
führen zum Import statt in eine Sackgasse. Der Sperrbildschirm fragt also nicht mehr nur Biometrie ab,
sondern schaltet über das `CryptoObject` einen lesbaren Schlüssel frei.

**Import.** Die Auswahl läuft über das Storage Access Framework mit einer eigenen Cachekopie je
Auswahl; ein Abbruch rollt zurück, und es läuft immer nur ein Import — auch über Activity-Neustarts
hinweg. Der umhüllte Schlüssel wird **zuletzt** geschrieben, damit ein abgebrochener Lauf beim
nächsten Start als „nicht eingerichtet" gilt und nicht als „fertig". Ein Null-Byte-Tresor zählt als
fehlend (der Kern würde sonst eine leere Datenbank anlegen). Ein nutzbarer Schlüssel **mit**
Tresordatei wird nie überschrieben.

**Hintergrundwechsel.** `AppUiState.backgroundLocked` sperrt das Fenster, nicht den Tresor:
`VaultUnlocker.releaseSoftLock()` prüft den Schlüssel ohne zweites `open`, `VaultState` bleibt `Own`.

**Datenabbildung.** `VaultSnapshot` hat zwölf Listen; gefüllt werden heute sieben: Gruppen, Konten,
Buchungen, Kategorien, Tags, Holdings und Sachwerte. Die restlichen vier (Regeln, Daueraufträge,
Mandate, Kredite) gehören zu 3.2b; Ankündigungen stammen nicht aus dem Tresor, sondern aus der
signierten `policy/announcements.json` und werden im Own-Snapshot noch nicht geladen. `bookingsTotal` kommt aus `page.totalCount`, die
Seite ist bewusst auf 1.000 begrenzt und ohne `wholeDays`, damit der Kern sie nicht über die Grenze
hinaus verlängert. Holdings übernehmen die Währung ihres Kontos als Kurslabel, die Kostenbasis
ergibt sich aus Wert minus Gewinn, gerundet wird auf Cent; Sachwerte tragen ihr Konto, damit das
Vermögen sie nicht doppelt zählt. `includeInTotal`, Notizen, Tags, `documentCount` und `isTransfer`
werden übernommen. Kategorisierung, Regelanwendung, Saldenhistorie und Suche bleiben im Kern — der
Mapper entscheidet nichts über Geld.

**Schreibpfade.** Keine. Die Oberfläche mutiert weiterhin nur den Snapshot im Speicher.

## Stufenplan

Vier Stufen statt „Android wie macOS" als ein Ziel. Jede Stufe ist für sich benutzbar.

**Stufe 1 — native Schicht und Lesepfad.** *Erledigt bis auf 3.2b, 3.3 und 3.4 (#1427, #1428,
#1429 über #1688).* Build-Skript,
NDK/UniFFI in der CI, CI-Gate gegen APKs ohne `.so`, Keystore-umhüllter Tresorschlüssel,
`openOwnVault()` über die Bindings, Mapper auf `VaultSnapshot`, Import der Desktop-Sicherung.

**Stufe 2 — Schreibpfade.** *Offen (#1430).* Überweisungen über die Kern-Zustandsmaschine statt
Snapshot-Mutation, Kategorien/Regeln/Notizen/Einstellungen über die Kern-API, sichtbare Bank- und
TAN-Fehler, Reload-Disziplin nach jeder Mutation, und Persistenz eigener Bearbeitungen über einen
harten Lock hinweg. Ergebnis: die Alpha-Kennzeichnung fällt.

**Stufe 3 — Konten.** *Offen.*
Bankzugänge über die bestehenden Konnektoren in `girocrab-core/src/connectors/` (FinTS, comdirect,
Trade Republic, Enable Banking, PayPal, Portal-Importer), alles über denselben FFI-Rand — kein eigener
Android-Code. Die Aufgaben 3.1 bis 3.4 gehören dagegen zu Stufe 1c (Issue #1429) und stehen dort.

**Stufe 4 — Rest.** Import/Export, Dokumente, Daueraufträge, Depot.

Nicht vorgesehen: lokale Schnittstelle (Unix-Socket) und MCP-Adapter. Beides ist heute eine
macOS-Funktion (`docs/local-api.md`: „Nur auf dem Mac") und der MCP-Adapter hängt an genau diesem
Socket; für Android ist beides nicht geplant. Technisch ginge ein `LocalServerSocket`, aber ein
zweiter, abweichender Zugang neben dem Desktop wäre eine eigene Schnittstelle mit eigenem
Berechtigungsmodell — das ist eine Produktentscheidung, keine Grenze der Plattform.

## Entschieden

Die drei offenen Fragen aus dem ersten Entwurf sind beantwortet (Antwort des Maintainers im
geschlossenen PR #1375):

1. **Ablage.** Kein Top-Level-`openspec/`-Baum im Repo. Der Plan steht als diese Seite unter
   `docs/`, die Arbeitsblöcke als Issues (siehe `CONTRIBUTING.md` › „Größere Vorhaben &
   Vorab-Spezifikation").
2. **Release-Zug.** Android bleibt in Stufe 1–2 ein eigenes Alpha-APK-Artefakt; macOS hängt am
   Sparkle-Appcast, iOS an App Store/TestFlight. Das Alpha-Label fällt nach Abschluss von Stufe 2.
3. **Stufe 1 nur lesend.** Ja — erst Build-Pipeline, Keystore und Lesepfad stabil, dann Schreibpfade.

## Offene Entscheidungen

**Wiederherstellung für einen auf dem Gerät angelegten Tresor.** Der Import deckt nur Tresore, deren
Schlüssel auf dem Desktop liegt; Android legt keinen eigenen an. Für einen rein mobilen Tresor braucht
es entweder eine beim Einrichten erzeugte Schlüsselkopie, die der Nutzer außerhalb der App verwahrt,
oder die ausdrückliche Ansage „keine Wiederherstellung". Bis dahin darf keine Fehlermeldung einen
Import anbieten, den es nicht gibt.

**Keystore-Invalidierung.** Zwei getrennte Auslöser: `setInvalidatedByBiometricEnrollment(true)`
verwirft den Schlüssel bei **neuer Biometrie** (weiterer Finger, neues Gesicht).
`setUserAuthenticationRequired(true)` bindet ihn an die Gerätesperre: ein **normaler PIN-/Muster-Wechsel
mit der alten PIN** überträgt die User-SID und lässt den Schlüssel nutzbar; unbrauchbar wird er, wenn
die Sperre **ohne die alte PIN** gesetzt oder entfernt wird — Werksreset, Entfernen der
Bildschirmsperre, administratives *untrusted enroll* (AOSP, *Authentication* › Enrollment). Beides ist
gewollt, kostet den Nutzer aber den Schlüssel und damit einen erneuten Import der Desktop-Sicherung.

## Design-Entscheidungen

**Bindings: UniFFI + JNA, kein handgeschriebenes JNI.** Ein handgeschriebener JNI-Rand bräuchte eine
Signatur pro exportierter Funktion (heute **103 `#[uniffi::export]`-Blöcke**, rund 300 Signaturen), die
bei jeder Kern-Änderung mitdriftet. Also `uniffi-bindgen generate --language kotlin` als Build-Schritt.
Verworfen: JNI von Hand, Kotlin-Multiplatform-Ersatz für den Kern (verdoppelt die Geschäftslogik).

**Ein Build-Skript, ein Name.** `app/build.gradle.kts:39` nennt `src/main/jniLibs`, das Skript füllt
genau diesen Pfad; die CI ruft dasselbe Skript auf, damit lokaler und CI-Build nicht auseinanderlaufen.

**Schlüssel: Keystore-umhüllt, Biometrie über `CryptoObject`.** Der 32-Byte-Tresorschlüssel wird mit
einem AES-GCM-Schlüssel aus dem `AndroidKeyStore` umhüllt, der umhüllte Wert liegt in `filesDir`.
Entsperrt wird über `BiometricPrompt` mit `CryptoObject` — ohne Krypto-Freigabe gibt es keinen lesbaren
Schlüssel. Verworfen: `EncryptedSharedPreferences` (abgekündigt, kein Auth-Binding), Klartext in
`filesDir`, Schlüsselableitung aus einem Nutzerpasswort (zweites Geheimnis neben der Gerätesperre).

**Snapshot für die Ansicht, Kern für die Logik.** `VaultSnapshot` bleibt das einzige Format der
Oberfläche; Kategorisierung, Regelanwendung, Saldenhistorie und Suche bleiben im Kern. Sonst entsteht
eine zweite Wahrheit, die zwischen Desktop und Android auseinanderläuft.

## Risiken

- **R8 und JNA.** `buildTypes.release` hat `isMinifyEnabled = true`. UniFFI-Bindings und
  `com.sun.jna` müssen in `proguard-rules.pro` gehalten werden, sonst bricht nur der Release-Build —
  und erst zur Laufzeit beim ersten `Vault`-Aufruf. Deshalb baut die CI zusätzlich die Release-APK als
  Smoke-Test.
- **APK-Größe.** Drei ABIs plus vendortes OpenSSL plus JNA ergeben eine Debug-APK von **88 MB**
  (Artefakt 47,8 MB gepackt). Für die Alpha akzeptabel, für eine Play-Auslieferung ein App Bundle mit
  Splits.
- **Threading.** Öffnen und Abfragen laufen im Kern über eine tokio-Runtime. Das Lesen des Snapshots
  läuft noch im Main-Thread — der eigene Schritt, der die Main-Zustellung mitbauen muss.
- **Keystore-Invalidierung.** Siehe „Offene Entscheidungen": Der Schlüssel geht bei neuer Biometrie und
  bei einer ohne alte PIN geänderten Sperre verloren; der Weg zurück ist der Import der Sicherung.

## Aufgaben

Die 23 Aufgaben standen mit Belegkriterium in vier übernehmbaren Blöcken; Stand heute:

- **Stufe 1a — native Schicht** (5 Aufgaben): erledigt. → Issue #1427
- **Stufe 1b — Tresor und Schlüssel** (6 Aufgaben): erledigt. → Issue #1428
- **Stufe 1c — Datenabbildung** (5 Aufgaben): erledigt bis auf 3.2b, 3.3 und 3.4. → Issue #1429
- **Stufe 2 — Schreibpfade** (4 Aufgaben): offen. → Issue #1430
- **Abschluss Stufe 1/2:** `SECURITY.md` nennt Android jetzt mit Schlüsselablage und Import; der
  Alpha-Hinweis im README fällt nach Stufe 2; diese Seite wird bei Abschluss von Stufe 2 erneut auf den
  dann aktuellen Stand gezogen.

## Belege nachprüfen

```sh
# 103 Blöcke; ein Block auf einem impl exportiert alle seine pub fn
grep -rho '#\[uniffi::export\]' girocrab-core/src | wc -l

# Native Schicht: Skript vorhanden, CI baut drei ABIs und prüft sie in der APK
ls GiroCrabAndroid/scripts/build-rust-android.sh
GiroCrabAndroid/scripts/build-rust-android.sh --check      # ohne NDK: erwarteter Fehlschlag
unzip -l GiroCrabAndroid/app/build/outputs/apk/debug/app-debug.apk | grep -c 'lib/.*/libgirocrab_core.so'   # 3

# CI-Gate gegen APKs ohne native Bibliothek
grep -n 'test "\$count" -eq 3' .github/workflows/android.yml

# Wie viele Kernmethoden die Oberflächen nutzen
git grep -hoE '\bvault\.[a-zA-Z]+\(' -- GiroCrabApp/Sources | sort -u | wc -l   # 165 (Swift)
```

## Herkunft

Der Plan entstand als OpenSpec-Baum in PR #1375 (geschlossen, weil reine Planungsbäume nicht ins Repo
gehören) und liegt hier in der von den Maintainern gewünschten Form: Prosa unter `docs/`, Arbeitsblöcke
als Issues. Erstellt mit Unterstützung eines KI-Agenten (Hermes, Nous Research) auf Basis der Antwort
des Maintainers; der Ist-Stand wurde aus dem Code gelesen, nicht aus dem Changelog. Diesen Stand hat
nach dem Merge von #1688 derselbe Agent aus dem gemergten Code nachgezogen.
