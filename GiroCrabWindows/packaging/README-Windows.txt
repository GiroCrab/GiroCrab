GiroCrab für Windows
====================

Start: girocrab.exe doppelt anklicken (keine Installation nötig).

Beim ersten Start legt GiroCrab einen verschlüsselten Tresor an. Der Schlüssel liegt in der
Windows-Anmeldeinformationsverwaltung (Credential Manager) deines Benutzerkontos, der Tresor
selbst unter %LOCALAPPDATA%\GiroCrab\vault.sqlite. PIN und TAN werden nie gespeichert.

Windows-SmartScreen: Diese Version ist nicht mit einem Code-Signing-Zertifikat signiert.
Beim ersten Start kann "Der Computer wurde durch Windows geschützt" erscheinen:
"Weitere Informationen" > "Trotzdem ausführen". Prüfe vorher die SHA-256-Summe der
Download-Datei gegen die Angabe in den Release-Notes.

Tastenkürzel: Strg+F Suche, Strg+L Sperren, Strg+R oder F5 Abrufen, Strg+I Importieren,
Strg+N Überweisung, Esc Dialog schließen.

Wenn etwas nicht klappt
-----------------------
Neben dem Tresor liegt ein Protokoll: %LOCALAPPDATA%\GiroCrab\girocrab.log
Die erste Zeile nennt Version und Tresorpfad ("app: start v0.9.0 tresor=..."), danach steht jeder
Vorgang mit Zeitstempel darin. Kontonummern und Namen der Gegenparteien werden vor dem Schreiben
durch Platzhalter ersetzt. Mehr Ausführlichkeit für eine Sitzung:

    set GIROCRAB_LOG=debug
    girocrab.exe 2>&1 | more

Bitte an einen Fehlerbericht hängen: girocrab.log und die VERSION.txt aus diesem Ordner.

Ausführliche Anleitung: docs/windows.md im Quellcode-Repository
https://github.com/GiroCrab/GiroCrab
Lizenz: GPL-3.0-or-later. Lizenzen der verwendeten Bibliotheken: THIRD_PARTY_NOTICES.md

---

GiroCrab for Windows: run girocrab.exe, no installation needed. The encrypted vault is kept in
%LOCALAPPDATA%\GiroCrab, its key in the Windows Credential Manager. This build is unsigned:
SmartScreen may warn on first start ("More info" > "Run anyway"); verify the SHA-256 checksum
from the release notes first.
Troubleshooting: the log sits next to the vault (%LOCALAPPDATA%\GiroCrab\girocrab.log); attach it
and VERSION.txt to a report. Set GIROCRAB_LOG=debug for more detail (girocrab.exe 2>&1 | more).
