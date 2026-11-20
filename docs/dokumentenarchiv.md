# Dokumentenarchiv: Belege verlinken statt doppelt ablegen

GiroCrab kann Belege in einem eigenen Dokumentenarchiv lassen und an der Buchung nur den Verweis speichern
(Issue #590, Diskussion #341). Unterstützt wird ein selbst betriebenes Archiv mit der REST-API von Paperless-ngx (API-Version 9).
Alles hier ist optional: ohne eingerichtetes Archiv sendet GiroCrab dafür keine einzige Anfrage.

## Stufen

1. **Link an der Buchung** (ohne Archivverbindung): eine `http(s)`-Adresse per „Link hinzufügen …" oder per Drag & Drop.
   Gespeichert werden Adresse, Titel und Datum, nie der Inhalt. Die Adresse wird nicht abgerufen.
2. **Archiv verbinden** (Einstellungen › Dokumente › Dokumentenarchiv): Adresse und Token. Danach lassen sich Dokumente
   des Archivs suchen und verlinken, verlinkte Belege werden in der App angezeigt (Vorschau, Quick Look), und zu
   Buchungen ohne Beleg gibt es Vorschläge. Der Filter „Nur ohne Beleg" zeigt, was noch fehlt, und
   „Aus dem Archiv zuordnen …" im selben Menü sieht das Archiv für alle Buchungen ohne Beleg des gewählten Zeitraums
   in einem Durchgang durch (siehe unten).
3. **Neue Belege ins Archiv verschieben** (Schalter): eine Datei, die an eine Buchung gehängt wird, wird hochgeladen
   und danach nur noch verlinkt. „Vorhandene Belege verschieben …" holt das für das nach, was schon im Tresor liegt.
4. **Dokumente der Banken ins Archiv verschieben** (Schalter): Kontoauszüge und Mitteilungen, die GiroCrab bei den
   Banken abruft, gehen denselben Weg und kommen im Archiv vorbelegt an (siehe unten).
5. **Buchungsdaten am Dokument vermerken** (Schalter): Datum, Betrag, Gegenpartei und Konto der Buchung stehen als
   Zusatzfelder am Dokument im Archiv.

Wer keine Schnittstelle einrichten möchte: „Dokumente automatisch in einen Ordner sichern" (Einstellungen ›
Dokumente) kopiert jedes Dokument in einen Ordner eigener Wahl, zum Beispiel in den Eingangsordner des Archivs. Das
Archiv ordnet die Dateien dann selbst ein; in GiroCrab bleiben sie zusätzlich im Tresor, einen Verweis gibt es so
nicht.

## Viele Belege auf einmal zuordnen

„Aus dem Archiv zuordnen …" stellt jeder Buchung ohne Beleg das Dokument gegenüber, das am besten passt. Strenger als
die Vorschläge an einer einzelnen Buchung: Der Betrag muss im Dokument stehen, das Datum in der Nähe liegen, und jede
Buchung und jedes Dokument kommt nur einmal vor. Passt ein Dokument gleich gut auf mehrere Buchungen (oder umgekehrt)
oder nennt das Dokument den Händler der Buchung nicht, ist die Zeile markiert und nicht vorausgewählt. Verlinkt wird erst, was Sie bestätigen. Ein Durchgang sieht bis zu
2000 Buchungen und 1000 Dokumente an; ein kürzerer Zeitraum findet den Rest.

## Was wohin geht

- Anfragen gehen ausschließlich an die eingetragene Adresse. Umleitungen werden nicht verfolgt, der Token wird
  nirgendwo anders hin gesendet und nur für die Adresse wiederverwendet, für die er eingegeben wurde.
- Unverschlüsselt (`http`) verbindet sich GiroCrab nur mit Adressen im eigenen Netz (privat, link-lokal, VPN);
  alles andere braucht `https`.
- Adresse und Token liegen im verschlüsselten Tresor dieses Geräts und werden nicht zwischen Geräten abgeglichen.
  Die Verweise selbst werden abgeglichen; ein anderes Gerät zeigt sie als einfache Links, bis es mit demselben
  Archiv verbunden ist.
- Fehlermeldungen und Protokolle enthalten weder Adresse noch Token noch Antworttexte des Archivs.
- Bei gesperrtem Tresor wird nichts gesendet; laufende Arbeit endet vor der nächsten Anfrage.

## Verschieben

Vor jedem Hochladen fragt GiroCrab das Archiv nach der Prüfsumme der Datei: Hat es genau diese Datei schon, wird
das vorhandene Dokument verlinkt und nichts hochgeladen (neuere Archivversionen nehmen dieselbe Datei sonst ein
zweites Mal auf). Verglichen wird mit der Datei, mit der das Dokument zuerst abgelegt wurde: Wurde es im Archiv
seither bearbeitet und hat dort eine neuere Fassung, bleibt es trotzdem dasselbe Dokument.

Eine Datei verlässt den Tresor erst, wenn das Archiv sie verarbeitet hat, das Dokument dort lesbar ist (nicht im
Papierkorb) und das Original Byte für Byte zurückkommt. Bis dahin bleibt sie wie gewohnt sichtbar. Ist unklar, ob
ein Upload angekommen ist (keine Antwort, Fehler eines Gateways), wird er nicht von selbst wiederholt: das Archiv
könnte die Datei sonst doppelt aufnehmen. Der Beleg zeigt dann einen Hinweis; „Erneut versuchen" oder „Im Tresor
behalten" entscheiden Sie.

Ist „Dokumente automatisch in einen Ordner sichern" eingeschaltet, wird eine Datei erst in den Ordner kopiert und
danach verschoben. Ist der Ordner nicht erreichbar, bleibt sie im Tresor.

## Dokumente der Banken

Ein Kontoauszug oder eine Mitteilung aus dem Postfach der Bank wird wie ein eigener Beleg verschoben. Der Verweis
bleibt, was die Datei war: ein Dokument dieses Kontos, mit seiner Art (Kontoauszug, Abrechnung, Steuerunterlage,
Bankmitteilung), als ungelesen markiert, bis es geöffnet wird.

- **Kein erneutes Ablegen.** Der Abruf erkennt ein Dokument an seiner Herkunft bei der Bank. Das Gerät, das ein
  Dokument verschoben hat, merkt sich diese Herkunft am Verweis: Solange er besteht, legt der Abruf das Dokument
  nicht wieder im Tresor ab. Wird der Verweis gelöscht, kommt das Dokument mit dem nächsten Abruf erneut, sofern
  die Bank es noch anbietet.
- **Vorbelegt.** GiroCrab kennt Bank, Art und Konto und gibt sie beim Hochladen mit: die Bank als Absender, die Art
  als Dokumenttyp, den Kontonamen als Tag. Gibt es im Archiv schon einen Eintrag dieses Namens (Groß- und
  Kleinschreibung egal), wird er verwendet; was fehlt, legt GiroCrab an, ohne Zuordnungsregel, damit das Archiv es
  nicht von sich aus auf andere Dokumente setzt. Darf der Token das nicht, geht die Datei ohne diese Angabe hoch.
  Abschaltbar („Bank, Dokumentart und Konto mitgeben"). Eigene Belege bekommen keine Vorbelegung.
- **Mehrere Geräte.** Jedes Gerät holt die Dokumente der Banken selbst, und der Schalter gilt je Gerät. Die
  gemerkte Herkunft bleibt auf dem Gerät, das verschoben hat (sie enthält Nummern, die nur dort gelten). Ein
  anderes Gerät legt sein eigenes Exemplar deshalb weiterhin im Tresor ab und zeigt, solange es diese Datei hält,
  die Datei statt des Verweises. Verschiebt es sein Exemplar ebenfalls und ist die Datei dieselbe, findet das
  Archiv sie über die Prüfsumme: Es wird nichts hochgeladen, und das Konto behält einen Verweis. Liefert die Bank
  jedem Gerät eine leicht andere Datei, liegt das Dokument danach zweimal im Archiv.
- Der Schalter gilt für Dokumente, die von nun an eintreffen. Ältere nimmt „Vorhandene Belege verschieben …" mit,
  solange der Schalter an ist; ein einzelnes Dokument lässt sich immer über sein Menü verschieben.
- **Im Tresor behalten** gilt dauerhaft: Eine so markierte Datei wird weder nach dem nächsten Abruf noch mit
  „Vorhandene Belege verschieben …" erneut eingereiht, sondern nur, wenn Sie genau diese Datei verschieben.

## Buchungsdaten im Archiv

Vier eigene Zusatzfelder, die GiroCrab bei Bedarf anlegt: `Buchungsdatum (GiroCrab)` (Datum),
`Buchungsbetrag (GiroCrab)` (Betrag mit Währung und Vorzeichen der Buchung), `Gegenpartei (GiroCrab)` und
`Buchungskonto (GiroCrab)` (Text). Andere Felder, auch gleich klingende eigene, werden nie gelesen, geändert oder
entfernt.

- Hängt ein Dokument an mehreren Buchungen: frühestes Datum, Summe der Beträge (bei einer Währung, sonst kein
  Betrag), alle Gegenparteien und Konten.
- Wird der Verweis umgehängt, ändern sich die Felder mit; wird der letzte Verweis entfernt, werden die vier Felder
  wieder vom Dokument genommen. Lehnt das Archiv das ab, zeigt die App es an.
- Beim Ausschalten bleibt stehen, was schon im Archiv steht.
- Werte, die im Archiv von Hand geändert werden, bemerkt GiroCrab nicht; sie bleiben, bis sich die Buchungsdaten
  des Dokuments ändern.
- Der Token braucht dafür das Recht, Zusatzfelder anzulegen und Dokumente zu ändern.

## Plattformen

Mac und iPhone/iPad teilen sich die Oberfläche (Einstellungen › Dokumente). Auf dem Mac ist sie gegen ein
Test-Archiv ausprobiert, für iOS bisher nur übersetzt. Linux zeigt verlinkte Belege als Links, die im Browser
öffnen, ebenso Windows; eine Oberfläche für die Archivverbindung gibt es dort noch nicht.

## Für Entwickler

Ausprobiert gegen Paperless-ngx 3.4.2 (Docker): `GIROCRAB_LIVE_ARCHIVE=http://127.0.0.1:8000
GIROCRAB_LIVE_ARCHIVE_TOKEN=… cargo test -p girocrab-core --lib live_archive -- --ignored --nocapture` fährt den
ganzen Weg gegen eine echte Instanz (lädt eine erfundene Rechnung hoch und löscht sie wieder; Testinstanz verwenden).

Kern: `girocrab-core/src/document_links.rs` (Verweise, Abgleich), `archive.rs` (Verbindung, Suche, Zuordnung),
`archive/moves.rs` (Verschieben), `archive/facts.rs` (Buchungsdaten), `archive/client.rs` (einziger HTTP-Client
dafür). Schema-Schritt 32 (Verweise) und 33 bis 35 (Archiv). Tests gegen einen lokalen Skript-Server in
`archive/tests.rs`.

**Weitere Archive anbinden.** Die Archiv-Logik kennt keine Adresse und kein Antwortformat: Alles, was eine
bestimmte Software ausmacht, liegt hinter dem Trait `Backend` in `archive/backend.rs` (wie `BankConnector` bei den
Banken). `archive/paperless.rs` ist die bisher einzige Umsetzung. Ein weiteres Archiv ist ein Modul daneben, das
`Backend` umsetzt, plus eine Variante in `Kind` (Kennung, Name, Adressen der Dokumente). Am Tresor ändert sich dafür
nichts: Ein Verweis speichert die Kennung des Archivs, die Dokumentnummer und die Adresse, nicht die Art. Die Art
steht bei der Verbindung (`archive.kind`). Grenze heute: Dokumentnummern sind Ganzzahlen; ein Archiv mit anderen
Kennungen braucht dafür eine Abbildung.
