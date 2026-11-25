# Abos & Verträge (GitHub #1787)

Sidebar › Zahlungen › **Abos & Verträge** (`SubscriptionsView`, `SidebarItem.subscriptions`); Details in der rechten Spalte
(`SubscriptionInspector`); Hinzufügen-Dialog `SubscriptionAddSheet`.

## Was zählt als Abo
- Erkannte wiederkehrende **Ausgaben und Einnahmen** (`RecurringSeries`, siehe #35), die der Nutzer markiert („Abo
  hinzufügen“ oder „Als Abo markieren“ im Fixkosten-Dialog), und **manuell angelegte** Einträge (Name, Art, Betrag,
  Turnus, ein Zahlungsdatum). Nichts wird automatisch als Abo eingestuft.
- **Eine Serie = ein Abo.** Markierung und Felder hängen am `seriesKey`, nicht am Empfänger. Zwei Verträge beim selben
  Anbieter (zwei Streaming-Tarife, mehrere In-App-Abos) sind nur dann getrennte Serien, wenn sich ihre Beträge deutlich unterscheiden (die Erkennung fasst ähnliche Beträge zusammen)
  (`amountSpecific`).
- Der Betrag trägt sein Vorzeichen: negativ = Ausgabe, positiv = Einnahme.

## Anzeige
- **Ausgaben und Einnahmen getrennt:** zwei Blöcke (Ausgaben zuerst). Die Kopfzeile zeigt beide nebeneinander, je
  Spalte Betrag / Monat über Betrag / Jahr (nur aktive Einträge, je Währung getrennt, #569).
- **Betragsbasis (Umschalter):** „Original“ zeigt jeden Eintrag in seinem eigenen Turnus („29,90 € / Quartal“),
  alternativ € pro Monat, Quartal, Halbjahr oder Jahr. Umgerechnet wird immer vom Jahresbetrag (`SubscriptionMath`);
  der Inspektor zeigt Original, Jahres- und Monatsbetrag.
- **Gruppierung:** nach Turnus (monatlich … individuell), nach **Aktiv / Inaktiv** oder gar nicht. Jede Gruppe zeigt
  ihre Summe in der gewählten Basis.
- **Aktiv / inaktiv:** inaktiv ist, was gekündigt ist *und* dessen Vertragsende erreicht ist (ohne bekanntes Ende:
  sofort). Ein gekündigter Vertrag, der noch läuft, bleibt aktiv.
- **Farben:** Frist in 0 bis 30 Tagen → Warnfarbe; rechtzeitig gekündigt, Vertrag läuft noch → Einnahmefarbe.
  **Durchgestrichen** wird erst, wenn der Vertrag beendet ist.
- **Icon:** das Händler-Signet der letzten Buchung (`MerchantSignet`); ohne Buchung (manuelle Einträge) eine
  Symbol-Kachel.
- **Sortieren:** Name, Betrag pro Jahr (nach Betrag, größte zuerst), Nächste Zahlung, Kündigungsfrist. **Filter:**
  Suche (Name, Notiz), Art (Ausgaben/Einnahmen), Turnus, Status (Aktiv, Inaktiv, Gekündigt, Frist in 30 Tagen), „Beendete ausblenden“. Der Hinzufügen-Dialog filtert zusätzlich nach Konto.
- „Buchungen im Konto anzeigen“ springt ins Konto und filtert auf alle Buchungen dieses Abos (Händler-Liste, alle Zeiträume). Ein Klick auf eine Zeile wählt den Eintrag; die **rechte Spalte** zeigt Turnus (änderbar), Vertragsfelder und **alle
  Buchungen** (Klick springt in die Buchungsliste). Die Steuerleiste bricht um, wenn die Mittelspalte schmal ist.
- Ändert sich der Turnus, kann die Serie einen anderen Schlüssel bekommen; Markierung und Felder wandern mit
  (`carrySubscription`).

## Vertragsfelder (alle optional)
Kündigungsfrist (Tage/Wochen/Monate), Vertragsende, Verlängerung um (Monate), Gekündigt, Gekündigt am, Probezeit, Notiz.
- „Kündigen bis“ = Vertragsende − Kündigungsfrist. Ohne Vertragsende gilt bei eingetragener Kündigungsfrist die nächste
  Zahlung als Ende (Verlängerung: der Turnus, falls keine eingetragen ist). „Gekündigt“ belegt das Vertragsende mit der
  nächsten Zahlung und „Gekündigt am“ mit heute vor.
- „Gekündigt am“ gegen die Frist: rechtzeitig oder nach Fristende (Hinweis im Inspektor).
- Probezeit: Enddatum einer kostenlosen Phase; Zeile „Probezeit bis … · danach Betrag“, Warnfarbe in den letzten 7 Tagen.
- Preiserhöhung: ist die letzte Buchung höher als die beiden davor, die gleich waren, steht „Preis erhöht: alt → neu“.
  Schwankende Rechnungen lösen das nicht aus.
- Ungekündigt mit Verlängerung: das Ende rollt um die Verlängerung weiter, solange das Ende **oder die Kündigungsfrist**
  verstrichen ist (wer die Frist verpasst hat, steckt im nächsten Zeitraum). Ohne Verlängerung gilt der Vertrag nach
  dem Ende als abgelaufen.
- Gekündigt: das Ende bleibt, es wird nicht weitergerollt – außer die Kündigung kam nach der Frist (Datum „Gekündigt am“):
  dann läuft ein weiterer Zeitraum (Verlängerung, sonst der Turnus) und die Kündigung gilt für dessen Ende.
- Am Tag des Vertragsendes ist der Vertrag noch aktiv; beendet ist er erst danach.
- Texte: „Kündigen bis … · noch n Tage“, „Kündigen nur noch heute“, „Kündigungsfrist am … verstrichen · Ende …“
  (Frist vorbei, Ende noch nicht erreicht), „Gekündigt · läuft bis …“, „Beendet am …“, „Vertrag abgelaufen am …“.

## Speicherung
Eine JSON-Einstellung `subscriptions.v2` im Tresor: `{ entries: { seriesKey: Felder }, manual: [ … ] }`. Kein
Schema-Schritt, kein Core-Umbau. `subscriptions.v1` (nur Notizen) wird beim Laden übernommen. Eine Serie, die nicht mehr
erkannt wird, verschwindet aus der Liste, behält aber ihre Felder.

Der Serienschlüssel hängt am typischen Betrag (`…#1299`): eine Preiserhöhung oder ein zweiter Vertrag desselben
Anbieters ändert ihn. Deshalb speichert jeder Eintrag zusätzlich Basisschlüssel, letzten Betrag und Konto;
`reconcileSubscriptionKeys()` ordnet einen verwaisten Eintrag der Serie mit demselben Basisschlüssel zu, wenn das
eindeutig ist (genau ein freier Kandidat, dessen Betrag höchstens die Hälfte vom letzten abweicht). Alles Unklare bleibt
unverändert liegen. Ist der gespeicherte Wert nicht lesbar (kaputtes JSON), wird nichts geschrieben.

**Kein iCloud-Abgleich:** `merge_other_vault` übernimmt `subscriptions.v2` nicht; die Abos gelten nur im Tresor,
in dem sie gepflegt wurden. Für einen späteren Abgleich (neueste Änderung gewinnt, Löschvermerke) ist ein eigener
Core-Schritt nötig.

## Performance
Die Buchungen der markierten Serien lädt (mehrere Klicks auf „Hinzufügen“ werden nacheinander und nur für das betroffene Konto geladen) `reloadSubscriptionBookings()` einmal pro Konto, abseits des Main-Threads, beim
Öffnen der Ansicht und beim Markieren (der Core reichert jede Zeile einer solchen Abfrage an; vier Jahre, höchstens
5000 Buchungen je Konto). Nicht pro Zeile aufrufen.

## Sensible Daten
Namen, Notizen und Buchungstexte `.sensitive()`, Beträge `.sensitive(.amounts)`; der Hinzufügen-Dialog ist eine
`capturableSheet`.
