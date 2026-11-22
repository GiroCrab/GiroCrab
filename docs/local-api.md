# Externe Analyse: lokale Schnittstelle (#136)

Für Entwicklung, Bündelung und lokale Tests siehe [local-api-entwicklung.md](local-api-entwicklung.md).

GiroCrab kann ausgewählte Konten, Buchungen, Kategorien und Summen für Programme auf demselben Mac
lesbar machen, etwa für eigene Auswertungen oder einen KI-Assistenten. Die Schnittstelle ist
standardmäßig aus und lesend: keine Zahlungen, keine Zugangsdaten. Ändern lassen sich nur Kategorien
und Tags von Buchungen, und das nur, wenn es eigens erlaubt ist (siehe „Änderungen“).

## Einschalten

Einstellungen › Analyse:

1. „Lesenden Zugriff für Programme auf diesem Mac erlauben“ einschalten.
2. Unter „Geteilte Konten“ die Konten auswählen. Neue Konten sind erst geteilt, wenn sie hier
   ausgewählt werden; gelöschte Konten fallen aus der Auswahl heraus.
3. Unter „Geteilte Felder“ festlegen, ob Empfänger/Auftraggeber, Verwendungszweck/Buchungstext,
   IBANs/Kontonummern und Notizen mitgehen. IBANs/Kontonummern und Notizen sind standardmäßig aus;
   dann werden IBANs und Ziffernfolgen ab sechs Stellen auch in Texten und Kontonamen unkenntlich
   gemacht.
4. Optional unter „Änderungen“: „Kategorien und Tags ändern erlauben“ (standardmäßig aus).
5. „Zugangsschlüssel erzeugen“ und den Schlüssel im Programm eintragen. Er wird nur einmal angezeigt;
   gespeichert wird nur sein Hash. „Zugriff widerrufen“, ein neuer Schlüssel oder das Ausschalten des Zugriffs sperren den alten sofort; zum erneuten Einschalten braucht es einen neuen Schlüssel.

Programme wie KI-Assistenten können geteilte Daten an einen Cloud-Dienst übertragen. Teilen Sie nur,
was dort verarbeitet werden darf.

## Wann der Zugriff besteht

Nur solange GiroCrab läuft, der eigene Tresor entsperrt ist und angezeigt wird (nie mit den
Demo-Daten, nicht solange noch der Sperrbildschirm zu sehen ist), die Schnittstelle eingeschaltet ist und
ein Zugangsschlüssel existiert. Nur auf dem Mac. Beim Sperren, beim
Wechsel zur Demo und beim Beenden wird der Socket geschlossen und seine Datei entfernt (bleibt sie nach
einem Absturz liegen, erkennt der nächste Start sie über `girocrab.sock.lock` als verwaist und ersetzt sie).

## Verbindung

- Unix-Domain-Socket, kein Netzwerkport:
  `~/Library/Application Support/GiroCrab/local-api/girocrab.sock`
  (Ordner `0700`, Socket `0600`; Verbindungen anderer Benutzer werden sofort geschlossen).
- Eine JSON-Anfrage pro Zeile, eine JSON-Antwort pro Zeile; mehrere Anfragen pro Verbindung. Eine
  letzte Anfrage ohne Zeilenende wird noch beantwortet, bevor die Verbindung endet.
- Höchstens 4 Verbindungen, 64 KiB pro Anfrage; eine Anfrage muss innerhalb von 10 Sekunden vollständig
  ankommen, Verbindungen ohne Anfrage enden nach einer Minute.
- Anfragen werden nacheinander beantwortet; die nächste beginnt erst nach einer Pause so lang wie die
  vorige (höchstens 0,25 s): Die App bleibt bedienbar, auch wenn ein Programm ständig abfragt.
- `girocrab.sock.lock` neben dem Socket legt fest, welche GiroCrab-Instanz ihn bereitstellt.

```
→ {"v":1,"token":"gcapi_…","method":"transactions","params":{"date_from":"2026-01-01","limit":50}}
← {"v":1,"result":{"transactions":[…],"next_cursor":"2026-03-14:4711"}}
```

Fehler: `{"v":1,"error":{"code":"…","message":"…"}}` mit `disabled`, `unauthorized`, `locked`,
`unsupported_version`, `unknown_method`, `invalid_params`, `not_shared`, `read_only` (Änderungen sind
nicht erlaubt), `busy` (zu viele Verbindungen, später erneut versuchen), `internal`. Ohne gültigen Schlüssel verrät keine Antwort etwas über die
Freigabe oder die Daten: Anfragen mit gültigem Format erhalten `unauthorized`, auch wenn die Freigabe
ausgeschaltet ist. Fehlerhafte oder zu große Anfragen, eine fremde Version, `busy` und `locked` werden vor der
Schlüsselprüfung beantwortet.

## Methoden (Version 1)

| Methode | Parameter | Ergebnis |
|---|---|---|
| `info` | – | Version, Anzahl geteilter Konten, geteilte Felder, `allows_edits`, letzter Abruf |
| `accounts` | – | geteilte Konten mit Saldo, Art, Währung, letztem Abruf |
| `categories` | – | Kategorienbaum (`id`, `parent_id`, `name`, `path`, `depth`) |
| `tags` | – | Tags (`id`, `name`, `booking_count`: nur Buchungen geteilter Konten) |
| `transactions` | `account_ids`, `date_from`, `date_to`, `category_id`, `include_subcategories`, `uncategorized_only`, `direction` (`income`/`expense`/`all`), `search`, `cursor`, `limit` (1–500, Standard 100; größere Werte werden auf 500 begrenzt) | Buchungen, neueste zuerst, mit `next_cursor` |
| `summary` | `date_from`, `date_to` (höchstens drei Jahre), `account_ids` | Einnahmen/Ausgaben je Währung, Netto-Summen je Kategorie |
| `set_category` | `transaction_ids` (1–500), `category_id` (`null` entfernt sie), `overwrite_manual` | je Buchung `status`, vorherige Kategorie und Quelle |
| `set_tags` | `transaction_ids` (1–500), `add`, `remove` (Tag-Namen) | angelegte Tags, je Buchung `status` |

Daten im Format `YYYY-MM-DD`, Beträge in Cent (`amount_cents`, negativ = Ausgang). Buchungs-IDs bleiben
für die Lebensdauer einer Buchung gleich. Die Suche steht nur zur Verfügung, wenn Empfänger,
Verwendungszweck und Kontonummern geteilt sind, weil sie auch diese Felder durchsucht.

Jede Buchung nennt `category_source`: `none`, `rule` (von einer Regel gesetzt, die sie wieder ändern
kann) oder `manual` (vom Nutzer gesetzt), dazu ihre `tags` als Namen.

## Änderungen

Nur mit „Kategorien und Tags ändern erlauben“, sonst antworten `set_category` und `set_tags` mit
`read_only`. Beide wirken nur auf Buchungen geteilter Konten; jede andere ID (unbekannt, gelöscht, nicht
geteilt) meldet `not_found`. Alle Änderungen einer Anfrage laufen in einer Transaktion und gehen wie
Änderungen in der App per iCloud an die anderen Geräte.

- `set_category`: Eine so gesetzte Kategorie gilt als vom Nutzer gesetzt (`manual`), Regeln ändern sie
  nicht mehr (nur „Speichern & anwenden › Alle Buchungen“ einer Regel kann sie ersetzen). Das gilt auch,
  wenn eine Regel schon dieselbe Kategorie vergeben hatte: Sie wird dann übernommen (`changed`). Eine von Hand gesetzte Kategorie bleibt (`skipped_manual`), außer mit
  `"overwrite_manual": true`; eine von einer Regel gesetzte wird ersetzt. Aufgeteilte Buchungen melden
  `split` (die Kategorie gehört zu den Teilen). Sonst `changed` oder `unchanged`.
- `set_tags`: Tags nach Namen, ohne Unterschied von Groß- und Kleinschreibung. Ein Tag aus `add`, den es
  noch nicht gibt, wird angelegt (`created_tags`); unbekannte Namen in `remove` werden übergangen. Derselbe
  Tag in `add` und `remove` ist ein Fehler (`invalid_params`).

Nach einer Änderung lädt GiroCrab die Anzeige neu.

## MCP-Adapter (`girocrab-mcp`)

Für KI-Assistenten und andere MCP-Clients gibt es einen kleinen Adapter, den der Client selbst startet
und über stdio anspricht (JSON-RPC 2.0, eine Nachricht pro Zeile). Er leitet jeden Tool-Aufruf als
Anfrage an den Socket weiter; den Tresor öffnet er nie und er hängt nicht vom Core ab.

Der Adapter liegt der App bei: `GiroCrab.app/Contents/Helpers/girocrab-mcp` (signiert und notarisiert
mit der App). Einstellungen › Analyse › „Für KI-Assistenten“ zeigt den fertigen Eintrag für die
MCP-Konfiguration mit diesem Pfad, direkt nach dem Erzeugen eines Schlüssels auch mit dem Schlüssel, und
kopiert ihn auf Knopfdruck (mit Schlüssel wird die Zwischenablage nach 60 Sekunden geleert).
„Verbindung testen“ startet das Hilfsprogramm so, wie es der Client tut, und meldet, ob Socket und
Schlüssel passen. In Entwickler-Builds: `cargo build --release -p girocrab-mcp` (→ `target/release/`).

Eintrag in der Konfiguration des MCP-Clients (Form je nach Client):

```json
{
  "mcpServers": {
    "girocrab": {
      "command": "/Pfad/zu/girocrab-mcp",
      "env": { "GIROCRAB_API_TOKEN": "gcapi_…" }
    }
  }
}
```

Verbindung von der Kommandozeile prüfen: `girocrab-mcp --check` (mit `--json` als eine JSON-Zeile mit
`problem` und `message`; Exit-Codes und Namen nennt `girocrab-mcp --help`).

Den Schlüssel nie als Argument übergeben (die Befehlszeile ist für andere Programme sichtbar):
`GIROCRAB_API_TOKEN` im Client-Eintrag oder `--token-file <Datei>` (Datei nur für Sie lesbar). Der
Socket-Pfad lässt sich mit `--socket` oder `GIROCRAB_API_SOCKET` ändern.

| Tool | Methode | Hinweis |
|---|---|---|
| `get_info` | `info` | |
| `list_accounts` | `accounts` | Ergebnis als `items` in `structuredContent` |
| `list_categories` | `categories` | Ergebnis als `items` in `structuredContent` |
| `list_transactions` | `transactions` | gleiche Parameter wie die Methode |
| `get_summary` | `summary` | `date_from` und `date_to` sind Pflicht |
| `list_tags` | `tags` | Ergebnis als `items` in `structuredContent` |
| `set_category` | `set_category` | nur mit erlaubten Änderungen |
| `set_tags` | `set_tags` | nur mit erlaubten Änderungen |

Alle Tools außer `set_category` und `set_tags` sind als nur lesend markiert (`readOnlyHint`); die beiden
tragen `destructiveHint: true` (eine Kategorie kann ersetzt werden) und `idempotentHint: true`. Beschreibungen und Eingabeschemas liefert
`tools/list`. Ist GiroCrab gesperrt, ausgeschaltet oder nicht gestartet, sind alle Verbindungen belegt
(`busy`) oder passt bzw. fehlt der Schlüssel, liefert der Tool-Aufruf einen lesbaren Fehler (`isError`),
den der Assistent weitergeben kann. Unterstützte MCP-Versionen: 2025-06-18, 2025-03-26, 2024-11-05.
