# Change 001 — Android an den Kern anbinden (Stufen 1–2 von 4)

**Status:** Proposed (19.11.2026)
**Fähigkeit:** `android-parity`
**Umfang:** Stufe 1 (native Schicht, Tresor, Lesepfad) und Stufe 2 (Schreibpfade). Stufe 3
(Konten über die Konnektoren) und Stufe 4 (Import/Export, Dokumente, Daueraufträge, Depot) folgen
als eigene Changes — sie stehen hier nur als Zielbild.

## Why

Die Android-App sieht fertig aus (7 Compose-Screens, 6.247 Zeilen Kotlin, Sperrbildschirm mit
Biometrie, Demo-Banner), kann aber keinen Tresor öffnen: `openOwnVault()` gibt `null` zurück
(`CoreBridge.kt:30`), es gibt keine native Bibliothek (`app/src/main/jniLibs` fehlt,
`scripts/build-rust-android.sh` ebenfalls) und die CI baut nur Kotlin
(`.github/workflows/android.yml` enthält kein `cargo`/`ndk`). Der Client ist damit eine Demo-Hülle.

Der Abstand ist nicht „etwas Politur", sondern die komplette Datenschicht: der Kern exportiert 91
Funktionen, Swift nutzt ~150 verschiedene `vault.`-Methoden, Android zwei.

## What Changes

Nach diesem Change kann ein Android-Nutzer seinen echten Tresor öffnen, lesen und — in Stufe 2 —
schreiben; die native Schicht entsteht reproduzierbar im Build, und die APK ohne `.so` wird zum
Build-Fehler statt zur stillen Demo.

Beobachtbare Unterschiede zum Ist-Stand:

1. Die APK enthält `libgirocrab_core.so` für drei ABIs und die generierten Kotlin-Bindings; ein
   Build ohne native Schicht schlägt fehl.
2. `openOwnVault()` liefert einen echten Tresor; Entsperren führt zu `VaultState.Own` mit echten
   Konten, Buchungen und Analysen statt zu Demodaten.
3. Der Tresorschlüssel liegt im Android-Keystore und ist an Biometrie gebunden; ohne erfolgreiche
   Biometrie bleibt der Tresor geschlossen.
4. Änderungen (Überweisung, Kategorie, Regel, Notiz, Einstellung) gehen über den Kern und
   überleben den Neustart.
5. Alpha-Kennzeichnung und Version bleiben getrennt von der Desktop-Linie, bis Stufe 2 abgenommen ist.

Nicht in diesem Change: Konten anbinden (FinTS/PSD2), Import/Export, Dokumente, Daueraufträge,
Depot — das ist Stufe 3/4 und bekommt eigene Changes.

## Impact

- Betroffene Fähigkeit: `android-parity` (fünf Requirements geändert, eines ergänzt).
- Nutzer: Android-Nutzer erhalten erstmals echte Daten; Desktop und iOS sind nicht betroffen.
- Betrieb: die Android-CI wird langsamer (Rust-Build je ABI) und braucht NDK, `cargo-ndk` und
  `uniffi-bindgen`; ein Cache für `~/.cargo` und `target/` ist Pflicht, sonst wächst die Laufzeit
  auf ein Vielfaches.
- Risiko: SQLCipher bringt vendortes OpenSSL mit; das muss für drei Android-ABIs übersetzt werden
  (Details in `design.md`, Abschnitt „Risiken").

## Specs-Delta

- `specs/android-parity/spec.md` — `## MODIFIED Requirements`: Native Schicht und Bindings, Tresor
  öffnen und entsperren, Schlüsselablage auf Android, Datenabbildung, Schreibpfade.
- `## ADDED Requirements`: Parität in Stufen mit Abnahmegate.

## Archivierungs-Hinweis

Beim Archivieren zuerst dieses Change anwenden, dann `specs/android-parity/spec.md` aus dem Delta
neu komponieren (die Live-Spec führt den Ist-Stand und wird durch die MODIFIED-Fassungen ersetzt).
Die Requirement-Namen im Delta stimmen mit den Live-Headern überein; bei einer Umbenennung eines
Live-Requirements vorher den Namen hier nachziehen, sonst scheitert der Namens-Match.
