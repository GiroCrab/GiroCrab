# Abruf-Fenster („Alle abrufen“)

Gilt für macOS und iOS/iPadOS (`GiroCrabUI/SyncRunSheet.swift`, Zustand in `SyncRun.swift`,
Ablauf in `AppModel+Sync.swift`). Linux zeigt den Abruf mehrerer Bankzugänge noch mit
einzelnen Fenstern und der Angabe „Bankzugang 2 von 5“.

## Wann es erscheint

„Alle abrufen“ und „Konten abrufen“ einer Gruppe öffnen das Fenster, sobald mehr als ein
Bankzugang beteiligt ist. Es bleibt für den ganzen Lauf offen. Der Abruf eines einzelnen
Zugangs oder Kontos und die automatischen Abrufe im Hintergrund behalten ihre bisherigen
Fenster und Hinweise.

## Aufbau

- **Kopf:** „Abruf läuft“ mit „noch X von Y“ und einem Fortschrittsbalken über die erledigten
  Zugänge. Am Ende steht dort „Abruf abgeschlossen“ mit „4 abgerufen · 1 übersprungen ·
  1 mit Fehler“. Genannt wird nur, was vorkam.
- **Liste:** Jeder Zugang des Laufs erscheint in der Reihenfolge des Abrufs mit Logo, Name
  (`.sensitive()`) und einer Statuszeile. Sie scrollt, wenn sie mehr Platz braucht, als
  neben der Eingabe frei ist.
- **Eingabe:** Unter der Liste stehen PIN-Abfrage, fehlende Angaben oder TAN/Freigabe des
  laufenden Zugangs. Es sind dieselben Ansichten wie beim Einzelabruf, ohne eigene Angabe
  „Bankzugang X von Y“ und ohne eigenes „Alle abbrechen“. Läuft ein Zugang ohne Eingabe,
  steht dort „<Name> wird abgerufen …“. Am Ende folgt ein Satz zum Ergebnis.
- **Browser-Anmeldungen** (Trade Republic, FNZ, eine Bank, die eine verlangt) öffnen sich als
  Fenster über dem Abruf-Fenster. Ihr „Abbrechen“ überspringt den Zugang. Als abgerufen zählt
  er nur, wenn danach ein Abruf lief oder das Browser-Fenster einen Import gemeldet hat (FNZ);
  jedes andere Schließen ergibt „Browser-Fenster ohne Abruf geschlossen“.
- **Fußzeile:** Während des Laufs „Alle abbrechen“ links. Rechts steht „Überspringen“, solange
  keine Eingabe ein eigenes hat. Am Ende steht dort nur „Fertig“ (Standardtaste).

## Zustände eines Zugangs

| Zustand | Symbol | Statuszeile |
|---|---|---|
| wartet | leerer Kreis | „Wartet“ |
| läuft | Aktivitätsanzeige | „Wird abgerufen …“, „Wartet auf Freigabe“, „TAN erforderlich“, „Anmeldung erforderlich“, „Angaben fehlen“, „Anmeldung im Browser-Fenster“ |
| fertig | grüner Haken | „12 neue Buchungen“ und, falls vorhanden, was sonst der Hinweis nach dem Abruf sagt (neue Konten, unvollständige Historie, Postbox) |
| übersprungen | Vorspul-Symbol | Grund: „Von Ihnen übersprungen“, „Abruf abgebrochen“, „Wegen Sicherheitssperre übersprungen“, eigener Ablauf (Advanzia), Datei-Import, Bitcoin Core über iCloud |
| Fehler | rotes Kreuz | Fehlermeldung (auswählbar) |

## Fehler, Abbruch, Wiederholen

- Ein Fehler beendet nur seinen Zugang. Die Zeile zeigt ihn, und der Lauf geht ohne Halt mit
  dem nächsten Zugang weiter. Eine Fehlermeldung, die den Lauf anhält, gibt es nicht mehr.
- Fehler, auf die eine Eingabe antwortet (fehlende Angaben, Browser-Anmeldung, abgelaufene
  Trade-Republic-Sitzung), öffnen diese Eingabe im Fenster. Deren „Überspringen“ lässt den
  Zugang aus.
- „Erneut versuchen“ steht bei Fehlern und bei Zugängen, die die Nutzerin oder der Nutzer
  übersprungen hat. Während des Laufs reiht es den Zugang hinten ein, nach dem Ende startet
  es ihn sofort. Das Fenster läuft dann weiter.
- Was ein Fehler zusätzlich anbietet (eine andere FinTS-Registrierung, die eigene eintragen),
  erscheint nach dem Ende in der Zeile. Ein Klick schließt das Fenster und führt es aus.
- „Alle abbrechen“ beendet den laufenden Abruf und markiert alle offenen Zugänge als
  „Abruf abgebrochen“. Das Fenster bleibt mit dem Ergebnis stehen. Ein „Erneut versuchen“
  startet erst, wenn der Bankdialog des abgebrochenen Zugangs beendet ist. Ein Zugang, der
  gerade noch auf die Prüfung der Sicherheitssperre wartete, startet danach nicht mehr.
- Zusatzaktionen eines Fehlers behalten keine eingegebene PIN; ein erneuter Versuch fragt sie
  wie gewohnt ab, sofern sie nicht gespeichert ist.
- Eine Sicherheitssperre für einen Zugang überspringt ihn, eine Sperre für alle überspringt
  alle offenen Zugänge. Den Grund nennt die Zeile.
- Das Sperren des Tresors schließt das Fenster sofort.
- Das Fenster lässt sich vor dem Ende nicht schließen, weder per Esc noch per Wischgeste.

## Was es ersetzt

Pro Zugang gibt es keinen Hinweis (Toast) mehr. Auch der Sammelhinweis „Übersprungen: …“
am Ende entfällt. VoiceOver liest am Ende die Zusammenfassung vor. Die Kontenauswahl für
neue, doppelt gelieferte Konten erscheint erst nach „Fertig“.

## Datenschutz (Problem melden)

Das Fenster ist ein `.capturableSheet`. Namen der Zugänge und Detailzeilen (sie können
Kontonamen nennen) tragen `.sensitive()`, die Zahl neuer Buchungen `.sensitive(.amounts)`.
Status, Fehlermeldungen und Logos bleiben sichtbar.

## Prüfen

```
swift run GiroCrabMac --demo-seed --demo-sheet sync-run --screenshot /tmp/abruf.png
swift run GiroCrabMac --demo-seed --demo-sheet sync-run-done --screenshot /tmp/abruf-ende.png
swift run GiroCrabMac --demo-seed --demo-sync --redact --screenshot /tmp/abruf-tan.png
```
