# Design — Android an den Kern anbinden

## Entscheidungen

Alle folgenden Punkte sind **Vorschläge des Beitragenden**, keine Vorgaben: die Wahl liegt bei den
Maintainern (offene Fragen am Ende). Das Delta beschreibt den angestrebten Zustand, keinen Beschluss —
wo eine Entscheidung offen ist, steht sie in den offenen Fragen und nicht als Fakt in der Spec.

### 1. Bindings: UniFFI + JNA, kein handgeschriebenes JNI

Der Kern deklariert 91 `#[uniffi::export]`-Funktionen. Ein handgeschriebener JNI-Rand wäre 91
Signaturen, die bei jeder Kern-Änderung driften. Gradle zieht JNA bereits
(`app/build.gradle.kts:62`, Kommentar „UniFFI uses JNA reflection"), die Klassennamen sind im
Kotlin-Bestand schon eingetragen (`uniffi.girocrab_core.Vault`, `…Girocrab_coreKt`,
`CoreBridge.kt:25,53`). Also: `uniffi-bindgen generate --language kotlin` als Build-Schritt.

**Verworfen:** JNI von Hand (Drift), Kotlin-Multiplatform-Ersatz für den Kern (verdoppelt die
Geschäftslogik — genau der Fehler, den `#663` im Core zusammengeführt hat).

### 2. Ein Build-Skript, ein Name

`build.gradle.kts:37` nennt bereits `scripts/build-rust-android.sh` als Erzeuger. Das Skript wird
angelegt statt einen zweiten Namen zu erfinden: `cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64`
gegen `girocrab-core`, danach `uniffi-bindgen` in `app/src/main/generated/`. Beide Ausgabepfade sind
in Gradle schon konfiguriert (`:37-40`), es ändert sich also nichts an der Modulstruktur.

Die CI ruft dasselbe Skript auf — der lokale und der CI-Build können nicht auseinanderlaufen. Der
`android.yml`-Job bekommt dafür NDK, `cargo-ndk`, `uniffi-bindgen` und einen Cache auf `~/.cargo`
plus `girocrab-core/target`; ohne Cache dauert der Rust-Teil je ABI Minuten.

### 3. Schlüssel: Keystore-umhüllter Schlüssel, Biometrie über `CryptoObject`

Der Tresorschlüssel ist 32 Byte. Er wird mit einem AES-GCM-Schlüssel aus dem `AndroidKeyStore`
umhüllt (`setUserAuthenticationRequired(true)`, `setInvalidatedByBiometricEnrollment(true)`), der
umhüllte Wert liegt in `filesDir`. Entsperrt wird über `BiometricPrompt` mit `CryptoObject` — ohne
erfolgreiche Krypto-Freigabe gibt es keinen lesbaren Schlüssel.

**Verworfen:** `EncryptedSharedPreferences` (von Google abgekündigt, kein Auth-Binding);
Klartext in `filesDir` (auf einem gerooteten Gerät oder im Backup lesbar); Schlüsselableitung aus
einem Nutzerpasswort (zwingt zu einem zweiten Geheimnis neben der Gerätesperre und wäre im Threat
Model von `SECURITY.md` nicht gedeckt).

Folge für `SECURITY.md`: die Zeile „Schlüssel liegen im Schlüsselbund (macOS/iOS) bzw. Secret
Service (Linux)" nennt Android und Windows nicht. Windows nutzt den Credential Manager; mit diesem
Change muss die Tabelle um Android (Keystore) ergänzt werden.

### 4. Abbildung: Snapshot für die Ansicht, Kern für die Logik

`VaultSnapshot` bleibt das einzige Format der Oberfläche (12 Listen, `Models.kt:209-222`). Kotlin
übersetzt nur; Kategorisierung, Regelanwendung, Saldenhistorie und Suche bleiben im Kern — sonst
entsteht eine zweite Wahrheit, die auf Desktop und Android auseinanderläuft.

Buchungen werden seitenweise geladen (`listTransactions` mit Cursor), weil ein realistischer Tresor
fünfstellig ist und ein Voll-Load im Snapshot auf einem Telefon Speicher kostet, den die VM nicht
hat.

### 5. Vier Stufen statt eines Ziels

Der Abstand zwischen zwei Aufrufen und 91 Exporten ist kein einzelner PR. Stufen: (1) native Schicht
+ Lesepfad, (2) Schreibpfade, (3) Konten anbinden über die bestehenden Konnektoren, (4)
Import/Export, Dokumente, Daueraufträge, Depot. Jede Stufe ist benutzbar und bekommt ein eigenes
Change; die Alpha-Kennzeichnung fällt erst nach Stufe 2.

## Risiken

- **OpenSSL für Android.** `rusqlite` ist mit `bundled-sqlcipher-vendored-openssl` gebaut
  (`girocrab-core/Cargo.toml:124`). Für Android muss `openssl-src` gegen den NDK übersetzt werden
  (`perl`, NDK-Clang, `CC_<target>`/`AR_<target>`); das ist der wahrscheinlichste Punkt, an dem der
  erste Build scheitert. Fallback: für Android auf `bundled-sqlcipher` ohne vendortes OpenSSL
  wechseln (System-OpenSSL entfällt auf Android, SQLCipher kann mit eigenem Krypto-Backend gebaut
  werden) — dann ist ein Feature-Flag im Crate nötig und der Desktop-Build muss unverändert bleiben.
- **R8 und JNA.** `buildTypes.release` hat `isMinifyEnabled = true`. UniFFI-Bindings und
  `com.sun.jna` müssen in `proguard-rules.pro` gehalten werden, sonst bricht nur der Release-Build —
  und zwar erst zur Laufzeit beim ersten `Vault`-Aufruf, nicht beim Kompilieren.
- **APK-Größe.** Drei ABIs plus OpenSSL plus JNA: ohne `abiFilters`-Split wächst die APK deutlich.
  Für die Alpha akzeptabel, für eine Play-Auslieferung ist ein App Bundle mit Splits vorzusehen.
- **Threading.** Öffnen und Abfragen laufen im Kern über eine tokio-Runtime; der Client muss sie
  hinter `Dispatchers.IO`/`Default` halten. Der heutige `withContext(Dispatchers.Default)`-Aufruf
  (`AppViewModel.kt:69`) ist richtig, aber ein blockierender Erstaufruf gehört nicht auf den
  Main-Thread.
- **Invalidierung des Keystore-Schlüssels.** Zwei getrennte Auslöser, die man nicht verwechseln darf:
  `setInvalidatedByBiometricEnrollment(true)` verwirft den Schlüssel, sobald eine **neue Biometrie
  registriert** wird (weiterer Finger, neues Gesicht); `setUserAuthenticationRequired(true)` macht ihn
  unbrauchbar, sobald die **Gerätesperre zurückgesetzt oder entfernt** wird (auch ein PIN-Wechsel).
  Beides ist gewollt — wer Gerätezugriff hat, soll den Tresor nicht durch die eigene Biometrie öffnen —
  kostet den Nutzer aber den Schlüssel: er braucht einen Weg zurück (Tresorschlüssel-Import wie auf dem
  Desktop), sonst ist der Tresor für ihn verloren. Das gehört in die Fehlermeldung.

## Offene Fragen an die Maintainer

1. Soll Android in denselben Release-Zug wie Desktop/iOS (Appcast/Signatur), oder bleibt es ein
   eigenes Alpha-Artefakt am Release dran?
2. Ist der Weg „Stufe 1 nur lesend" akzeptabel, oder soll Android erst erscheinen, wenn Schreiben
   (Stufe 2) steht?
3. Wird ein eigener `openssl`-Build für Android im Repo gepflegt oder als CI-Schritt ohne
   Repo-Artefakt?
