# Changelog – GiroCrab für Linux

Die Linux-Oberfläche ist ein Community-Port der Mac-App
([#389](https://github.com/GiroCrab/GiroCrab/issues/389)). Sie folgt der Mac-App in eigenem
Tempo: Der Kern – Bankanbindungen, verschlüsselter Tresor, Kategorisierung, Auswertungen – ist
immer der aktuelle, die Oberfläche holt die Funktionen der Mac-App schrittweise nach. Aufbau,
Verhalten und Texte kommen dabei 1:1 aus der Mac-App, auf Deutsch und Englisch. In
Einstellungen › Sicherheit steht deshalb „Kern-Version“: Die Nummer gehört zum Kern, nicht zum
Funktionsumfang der Oberfläche.

## Nächster Stand (in Arbeit)

### Seitenleiste, Kategorien, Regeln

- **Seitenleiste mit Abschnitten:** „Neue Umsätze“ und „Vermögen“ stehen ganz oben. Darunter
  folgen einklappbare Abschnitte **Konten**, **Zahlungen** (Geplant, Lastschriften – mit Zählern),
  **Dokumente** und **Kategorien**; der Zustand bleibt über Neustarts erhalten. Alle Pfeile und
  Symbole liegen auf einer gemeinsamen Achse, Unterkategorien und Konten rücken einheitlich ein.
- **Konto-Zeilen** kompakter: Zahl neuer Buchungen am Kontosymbol, Kontoinhaber als Tooltip,
  Saldo in der zweiten Zeile; archivierte Konten zeigen keinen Saldo mehr („Archiviert – wird
  nicht mehr abgerufen“).
- **„+“ unten** öffnet ein Menü: Neues Konto, Neue Kontengruppe, Neue Kategorie (unter der
  gewählten), Neue Kategoriengruppe.
- **Konto bearbeiten …** im Kontextmenü: Name, Kontoinhaber, Kontoart und eine **Konto-Notiz**,
  die unter dem Kontonamen erscheint. „… abrufen“ nennt das Konto statt des Bankzugangs.
- Einheitliche Dialoge zum Archivieren und Löschen von Konten und Kategorien (Name im Titel).
  Konten-Auswertungen erscheinen nicht mehr bei Depots.
- **„Ohne Kategorie“** lässt sich umbenennen (Rechtsklick › Umbenennen …, „Standardbezeichnung“
  stellt den Namen zurück); die Bezeichnung gilt auch in Titel, Buchungsliste und Aufteilen.
- **Kategorie duplizieren** samt Unterkategorien („… Kopie“).
- **Aufgeklappte Kategorien** werden gemerkt, getrennt für Demo und eigenen Tresor.
- **Regeln:** auf mehrere Konten einschränken (Konten als Chips, „Konto hinzufügen“),
  Reihenfolge per Drag & Drop oder Kontextmenü („An den Anfang“, „Nach oben“, „Nach unten“,
  „Ans Ende“), Filter über der Liste mit `*` und `?` (z. B. „deu*b“), Rang vor jeder Regel.
  Das Feld „Priorität“ entfällt wie in der Mac-App: Es zählt die Reihenfolge der Liste.
- Bankzugang bearbeiten: Easybank und Alte Leipziger speichern das Passwort, die Volkswagen Bank
  die PIN im Tresor.

### Tresor sichern und wiederherstellen

- **Einstellungen › Sicherheit › „Sicherung“:** „Tresor sichern …“ speichert den eigenen Tresor
  als Datei „GiroCrab-Sicherung <Datum>.girocrab-backup“, verschlüsselt mit einem Passwort Ihrer
  Wahl (mindestens 8 Zeichen, zweimal eingegeben). Die Sicherung lässt sich auf jedem Rechner
  öffnen, auch ohne den Schlüsselbund dieses Rechners – aber nicht ohne das Passwort.
- **„Passwort vorschlagen“** erzeugt ein starkes Passwort (drei Sechsergruppen, rund 90 Bit aus
  dem Zufallsgenerator des Kernels) und trägt es in beide Felder ein. „Kopieren“ legt es für
  60 Sekunden in die Zwischenablage, ohne Eintrag im Verlauf der Zwischenablage.
- **„Tresor wiederherstellen …“** ersetzt alle Daten des eigenen Tresors durch eine Sicherung und
  zeigt danach den eigenen Tresor, auch wenn vorher die Demo zu sehen war.
- **„Vorhandene Daten importieren“ › Datenbank** bietet jetzt auch die GiroCrab-Sicherung an
  (Art: GiroCrab oder MoneyMoney). Bei leerem Tresor ist sie vorgewählt; enthält der Tresor schon
  Konten, fragt „Tresor ersetzen?“ nach.

### Belege & PostBox

- **Filter oben in der Symbolleiste:** „Alle Dokumente“, „Belege“ (selbst hinzugefügte
  Rechnungen, Kassenbons, Fotos) oder „PostBox“ (Dokumente der Bank) – unabhängig davon, ob ein
  Dokument einem Umsatz zugeordnet ist. Überschrift und Erklärung folgen der Auswahl, darunter
  steht die Zahl der Dokumente.
- **„Beleg hinzufügen …“** wählt PDFs oder Fotos per Dateidialog und sucht den passenden Umsatz,
  wie beim Hineinziehen.
- Jede Zeile nennt Herkunft und Konto („Beleg · Girokonto“) und den zugeordneten Umsatz oder
  „Ohne zugeordneten Umsatz“. „Nur ohne zugeordneten Umsatz“ ersetzt „Nur ohne Buchung“.
- „Belege & PostBox“ beginnt wieder mit „Alle Konten“, wenn man den Bereich verlässt und später
  zurückkehrt. Findet ein Filter nichts, führt „Alle Dokumente anzeigen“ zurück.
- **„Zuordnung aufheben“** (vorher „Verknüpfung lösen“) ist bei Dokumenten ohne Umsatz
  ausgegraut und bestätigt sich mit einem Hinweis.
- **„Im Dateimanager anzeigen“** im Kontextmenü und im Beleg-Menü der Detailspalte öffnet den
  Dateimanager (Dolphin, Nautilus, …) mit der entschlüsselten Kopie ausgewählt; ohne
  passenden Dateimanager öffnet sich der Ordner.

### Commerzbank mit HBCI-Schlüsseldatei (RAH-10)

- **Bank hinzufügen › „Commerzbank · Schlüsseldatei“** (unter „Nicht gefunden?“) übernimmt eine
  bestehende HBCI-Schlüsseldatei mit Profil 10 als Kopie in den verschlüsselten Tresor; die
  Originaldatei bleibt unverändert. Zuerst die FinTS-Produktregistrierung, dann Datei,
  Anzeigename und Passwort; „Datei prüfen“ prüft Passwort und Datei ohne Netz.
- Fehlen die öffentlichen Bankschlüssel, ruft „Öffentliche Bankschlüssel abrufen“ sie ab. Sie
  werden erst übernommen, wenn für beide Schlüssel der SHA-256-Prüfwert aus der offiziellen
  Commerzbank-Quelle eingetragen ist und übereinstimmt.
- Danach startet der erste Abruf. Beim Abruf fragt GiroCrab nach dem „Passwort der
  Schlüsseldatei“ statt nach einer PIN; vorerst nur Konten, Salden und Umsätze.

## Funktionsstand 0.7.0 (Kern 0.7.5)

Mit diesem Schritt kann die Linux-Oberfläche alles, was die Mac-App 0.7.0 kann. Was aus
0.7.1 bis 0.7.5 noch fehlt, steht am Ende.

### Seitenleiste: alle Konten im Blick

- **„Alle Konten“** steht jetzt ganz oben als Wurzel der Kontenliste. Die Zeile zeigt den
  Gesamtsaldo aller Konten, die zur Gesamtsumme zählen – bei mehreren Währungen eine Summe je
  Währung, untereinander, damit nichts abgeschnitten wird (Fremdwährungen mit Summe 0 entfallen).
  Ein Klick auf den Pfeil klappt den ganzen Kontenbaum weg, ein Klick auf die Zeile öffnet die
  Übersicht über alle Buchungen (#397).
- **Drei Ansichten** für die Konten: nach eigenen **Gruppen**, nach **Bank** oder nach
  **Kontoart**. Umschaltbar oben in der Seitenleiste, im Menü Darstellung › Konten anzeigen und
  in der Befehlspalette (Strg+K), die auch jede Bank direkt anspringt (#377).
- **Kontosuche:** Das Suchfeld über der Liste filtert nach Name, Kontoinhaber, Bank, IBAN oder
  Kontonummer (Leerzeichen in IBANs egal). Passt der Inhaber oder die IBAN, zeigt die Zeile genau
  das statt des Banknamens, damit der Treffer sofort erkennbar ist.
- **Konto-Zeilen** zeigen das Kürzel des Kontoinhabers und statt eines Punktes eine **Blase mit
  der Zahl neuer Buchungen** (#426).
- Neu im Kontextmenü eines Kontos: **„Kontoart ändern …“** (z. B. ein Girokonto, das eigentlich
  eine Kreditkarte ist) und **„Kontoinhaber …“** (#462, #280).
- **„Neue Umsätze“**: eine eigene Seite mit allen noch ungeprüften Buchungen, als eine Karte je
  Konto mit Kontostand und vorgemerkter Summe. Wer neue Buchungen abhaken will, muss nicht mehr
  Konto für Konto durchgehen (#352).
- **Kategorien** lassen sich als Ganzes einklappen, wenn man sie selten braucht (#408).
- Bedienung mit der **Tastatur**: ↑/↓ wechseln die Auswahl in der Seitenleiste, ←/→ klappen
  Kategorien mit Unterkategorien auf und zu.

### Gruppen, Farben, Kontoauswahl

- **„Gruppen bearbeiten …“** hat eine Suche über Gruppen, Konten, Kontoinhaber und IBANs mit
  Trefferzahl je Gruppe. Konten lassen sich per Ziehen auf eine Gruppe links legen (#377).
- **Farbwahl** für Konten, Gruppen und Kategorien: zusätzlich Gelb, Braun und Schiefer, dazu
  „Eigene Farben“ – alle Farben, die im Tresor schon benutzt werden, mit einem Klick wieder
  verwendbar und per Rechtsklick entfernbar (#466).
- Die Kontoauswahl in **Belege & PostBox** und im **Regel-Editor** ist gegliedert wie die
  Seitenleiste (nach Gruppe, Bank oder Kontoart) statt einer langen, flachen Liste (#412).

### Vermögen und Sachwerte

- Neue Seite **„Vermögen“**: Nettovermögen mit Vermögenswerten und Verbindlichkeiten (Kredite
  und Karten wahlweise abgezogen), die Zusammensetzung als Ring, der Cash-Verlauf über 3 Monate
  bis 5 Jahre, Wertpapiere mit Tagesveränderung und Ausschüttungen, Gebühren und Steuern.
  Umschaltbar zwischen Alles, Cash, Wertpapiere und Sachwerte (#348).
- **Sachwerte** wie Gold, Immobilien oder Fahrzeuge gibt es jetzt als Konto ohne Bankanbindung
  (Ablage › „Neues Sachwerte-Konto …“). Je Sachwert lassen sich Bewertungen und Mengen mit Datum
  erfassen und Verkäufe buchen; Goldbestände werden mit aktuellen Goldpreisen bewertet (#87, #345).

### Buchungen und Auswertungen

- **Export** der Umsätze als **PDF** (Kontoauszug-Layout, A4), **CSV** (für Tabellen,
  Semikolon, UTF-8) oder **CAMT.053** (ISO 20022, für andere Programme und Steuerberater) – für
  einen Zeitraum, ein oder mehrere Konten oder nur die markierten Buchungen. Ablage › „Umsätze
  exportieren …“, im Kontextmenü der Buchungen und in der Befehlspalette. CAMT.053 erstellt der
  Kern mit den echten Salden der Bank; Darlehen und Depots sind dafür gesperrt (#401, #372, #483).
- Das **Zeitraum-Menü** bietet rollierende Zeiträume (30 Tage, 90 Tage, 12 Monate), „Seit
  letztem Gehalt“, die letzten Monate, Quartale und Jahre beim Namen und einen eigenen Zeitraum
  (#382).
- **Budgets** lassen sich bearbeiten und entfernen; wer keine Budgets nutzt, blendet sie in
  Darstellung oder in den Einstellungen ganz aus (#430).
- **Splitbuchung:** Der Restbetrag wird angezeigt, „Rest zuweisen“ verteilt ihn, und ein Hinweis
  sagt, warum „Speichern“ noch ausgegraut ist (#383). Darlehen lassen sich nur ausgehenden
  Buchungen zuordnen.
- Eine eben geprüfte Buchung bleibt in der Liste stehen, bis man eine andere auswählt, statt
  unter dem Mauszeiger zu verschwinden (#470).
- Die **Händler-Historie** im Inspektor zeigt neben allen Konten auch die Zahlen für das eigene
  Konto (#299).
- Eine **Regel aus einer Einnahme** gilt nur für Einnahmen und kategorisiert nicht versehentlich
  auch die Überweisungen an denselben Empfänger (#295).
- **Amazon-Bestellungen** (Zugang „Amazon Bestellungen“): Buchungen zeigen die bestellten Artikel,
  „+N weitere“, Abo-Bestellungen und die Bestellnummer; ein Pfeil klappt alle Artikel auf. Im
  Inspektor sind die Artikel Links zur Produktseite, Erstattungen springen zur Bestellung
  (#333, #349, #354).

### Einstellungen, Hilfe und Darstellung

- **Schriftgröße** Standard, Groß oder Sehr groß für die ganze App – in Einstellungen ›
  Allgemein und mit Strg+Plus, Strg+Minus, Strg+0 (#343).
- **Erscheinungsbild** System, Hell oder Dunkel, unabhängig vom System (#388).
- **App zurücksetzen** (Einstellungen › Sicherheit): löscht nach einer Bestätigung Tresor,
  Schlüssel im Schlüsselbund und Einstellungen und startet GiroCrab neu wie beim ersten Start
  (#241).
- **Kontoauswahl beim Bankzugang:** Liefert ein neuer Bankzugang Konten, die ein anderer Zugang
  schon abruft, fragt GiroCrab, welche übernommen werden – doppelte Konten bleiben aus (#346).
- **Hilfe › Problem melden** (Strg+Alt+B): ein eigenes Fenster mit einem Screenshot des
  Hauptfensters, in dem Beträge, Namen, IBANs und andere persönliche Daten automatisch geschwärzt
  sind; weitere Stellen lassen sich von Hand schwärzen. Optional mit dem Abruf-Protokoll, ebenfalls
  bereinigt. Gesendet wird nichts automatisch: Ein Klick öffnet ein vorausgefülltes GitHub-Issue im
  Browser, das Bild liegt in der Zwischenablage (#301).
- Die **animierte Krabbe** der Mac-App begrüßt auf Startbild, Sperrbildschirm und
  Willkommenskarte. Unter Linux wirft sie beim Entsperren und Laden des Tresors die Münze; die
  Animation läuft im Render-Thread und steht deshalb auch beim Laden großer Tresore nicht still.

### PSD2-Banken (Enable Banking)

- **Revolut** und andere Banken, deren Freigabe ein zweites Fenster öffnet, funktionieren jetzt
  im GiroCrab-Anmeldefenster. Bisher wurde das zweite Fenster in das erste geladen und die Seite,
  die auf die Freigabe am Handy wartete, ging verloren („Autorisierung konnte nicht abgeschlossen
  werden“). Jetzt öffnet es sich als echtes Pop-up mit derselben Sitzung (#289).
- Das Anmeldefenster blieb weiß, stürzte nach einer erfolgreichen Freigabe ab und gab sich als
  „QtWebEngine“ zu erkennen, was manche Banken ablehnen – alles behoben.
- `girocrab --debug-consent` protokolliert den Anmeldeablauf im Terminal (nur Adressen und die
  Namen der Parameter, keine Codes oder Tokens) – für die Fehlersuche bei weiteren Banken.
- **Bank hinzufügen** bietet für PSD2 die heruntergeladene .pem-Schlüsseldatei an statt eines
  Textfelds; die Anleitung nennt auch, was in „Privacy URL“ und „Terms URL“ gehört.

### Weitere Korrekturen

- „Bearbeiten › Suchen“ setzt den Fokus ins Suchfeld; die Kontosuche verliert beim Tippen nicht
  mehr den Fokus.
- Bankdialoge, die aus den Einstellungen starten (z. B. „Neu verbinden“), erscheinen vorn statt
  hinter dem Einstellungsfenster.
- Scrollleisten sind im Dunkelmodus gut sichtbar; „Einstellungen › Allgemein“ ist groß genug.
- Namen in der Buchungsliste werden nicht mehr zu früh gekürzt („Tim Hoffma…“).
- Neue Regeln lassen sich wieder speichern (das Feld heißt im Kern jetzt `account_ids`); Regeln,
  die in der Mac-App auf mehrere Konten eingeschränkt wurden, zeigt Linux als solche an.

### Noch nicht in der Linux-Oberfläche (Mac 0.7.1 – 0.7.5)

Diese Funktionen der Mac-App folgen in den nächsten Schritten, unter anderem:
- Seiten **„Geplant“** (Termin- und Daueraufträge) und **„Lastschriften“** (Mandate, pain.008)
- **Sammelüberweisung** und **Umbuchung** zwischen eigenen Konten
- **Tags** für Buchungen, **Buchungsdichte** mit drei Ansichten, vorgemerkte Umsätze schraffiert
- anpassbare rechte Seitenleiste, Seitenleiste mit Abschnitten
- **Regeln für mehrere Konten** (Linux wählt vorerst ein Konto), Kategorie-Ansichten je Konto,
  Kategorien duplizieren, Anlageklassen in der Vermögensübersicht
