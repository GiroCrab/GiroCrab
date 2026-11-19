# Tasks — Android an den Kern anbinden

Reihenfolge ist Abhängigkeitsreihenfolge: ohne 1.1 gibt es nichts zu mappen, ohne 1.3 keinen
Schlüssel zum Öffnen. Jede Aufgabe nennt den Beleg, der sie abschließt.

## 1. Native Schicht (Stufe 1, blockierend)

- [ ] 1.1 `scripts/build-rust-android.sh` anlegen: `cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -o app/src/main/jniLibs`, danach `uniffi-bindgen generate --language kotlin --out-dir app/src/main/generated`. Beleg: Skript läuft lokal durch, `app/src/main/jniLibs/*/libgirocrab_core.so` existiert (3 Dateien).
- [ ] 1.2 OpenSSL/SQLCipher für Android übersetzen (`openssl-src` gegen NDK), Umgebungsvariablen im Skript festnageln. Beleg: `cargo ndk` endet mit Exit 0 und die `.so` linkt (`llvm-readelf -d` zeigt keine fehlenden Symbole).
- [ ] 1.3 `scripts/build-rust-android.sh` aus `.github/workflows/android.yml` aufrufen (NDK, `cargo-ndk`, `uniffi-bindgen`, Cache auf `~/.cargo` und `girocrab-core/target`), vor `gradle assembleDebug`. Beleg: grüner Lauf mit Cache-Treffer; Laufzeit im Job-Log.
- [ ] 1.4 Prüfschritt in die CI: `unzip -l …/*.apk | grep -c 'lib/.*/libgirocrab_core.so'` muss 3 sein, sonst Fehlschlag. Beleg: absichtlich entfernte `.so` lässt den Job rot werden.
- [ ] 1.5 `proguard-rules.pro` um `uniffi.**` und `com.sun.jna.**` erweitern, Release-Build (`assembleRelease`) einmal bauen und auf einem Gerät starten. Beleg: Release-APK öffnet die Demo ohne `ClassNotFoundException`.

## 2. Tresor und Schlüssel (Stufe 1)

- [ ] 2.1 `VaultKeyStore` in Kotlin: AES-GCM-Schlüssel im `AndroidKeyStore`, umhüllter Tresorschlüssel in `filesDir`, `setUserAuthenticationRequired(true)`. Beleg: Unit-Test mit Robolectric/Instrumented-Test legt an, liest zurück, entsperrt per CryptoObject-Stub.
- [ ] 2.2 `BiometricPrompt` mit `CryptoObject` statt reiner Erfolgsprüfung verdrahten (`LockAndOnboarding.kt:61-92` umbauen). Beleg: Abbruch der Biometrie liefert keinen entsperrten Cipher.
- [ ] 2.3 `DefaultCoreBridge.openOwnVault()` implementieren: `Vault` aus den Bindings öffnen, `TODO(parity)` entfernen. Beleg: Instrumented-Test öffnet einen vorbereiteten Tresor und liest die Kontenzahl.
- [ ] 2.4 Fehlerpfade sichtbar machen (falscher Schlüssel, invalidierter Keystore, fehlende Datei) mit Ausweg „Schlüssel importieren". Beleg: drei Fälle mit sichtbarer Meldung, kein stiller Demo-Fallback.
- [ ] 2.5 Sperren beim Wechsel in den Hintergrund (`ON_STOP`), Wiedereintritt über Biometrie. Beleg: App-Wechsel zeigt Sperrbildschirm, `VaultState` bleibt `Own`.

## 3. Datenabbildung (Stufe 1)

- [ ] 3.1 Mapper `VaultSnapshot` für Konten, Gruppen, Buchungen, Kategorien, Tags. Beleg: Instrumented-Test vergleicht Anzahl und Summen gegen einen Fixture-Tresor.
- [ ] 3.2 Mapper für Regeln, Daueraufträge, Mandate, Holdings, Kredite, Sachwerte, Ankündigungen. Beleg: dieselbe Gegenprobe, alle 12 Listen belegt.
- [ ] 3.3 Buchungsliste mit Cursor-Paging über `listTransactions` statt Voll-Load. Beleg: Tresor mit 5.000 Buchungen, Speicherbedarf wächst nicht über eine Seite hinaus.
- [ ] 3.4 Analyse-Summen aus dem Kern (Saldenhistorie) statt Kotlin-Nachbau. Beleg: Monatssummen identisch zu dem, was der Desktop-Client für denselben Tresor anzeigt.
- [ ] 3.5 Demo-Pfad bleibt erhalten und wird nur noch genutzt, wenn kein Tresor existiert. Beleg: frisches Gerät → Demo mit Banner; Tresor vorhanden → `Own` ohne Banner.

## 4. Schreibpfade (Stufe 2)

- [ ] 4.1 Überweisung über den Kern (`begin_transfer`-Zustandsmaschine) statt Snapshot-Mutation (`AppViewModel.kt:139`). Beleg: Buchung nach Neustart vorhanden, Kontostand angepasst.
- [ ] 4.2 Kategorien, Regeln, Notizen, Einstellungen über die Kern-API. Beleg: je ein Instrumented-Test, der nach Neustart den geänderten Wert liest.
- [ ] 4.3 Bank-Fehler und TAN-Abbruch als sichtbare Meldung durchreichen. Beleg: abgebrochene TAN erzeugt Meldung, keine Buchung.
- [ ] 4.4 Reload-Disziplin: nach jeder Mutation Snapshot neu laden (Vorbild `AppModel+Loading.swift`). Beleg: zwei Geräte/Prozesse sehen denselben Stand.

## 5. Abschluss Stufe 1/2

- [ ] 5.1 `SECURITY.md`: Schlüsselablage-Zeile um Android (Keystore) und Windows (Credential Manager) ergänzen.
- [ ] 5.2 README-Alpha-Hinweis erst entfernen, wenn 4.1–4.4 abgenommen sind.
- [ ] 5.3 `docs/`-Seite „Android" mit Stand, Stufen und bekannten Grenzen (kein lokaler Socket, keine MCP-Anbindung auf Android).
