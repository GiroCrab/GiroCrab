# Buchungen in andere Apps ziehen

Auf dem Mac lassen sich Buchungen aus der Liste in andere Programme ziehen, etwa in eine Belegverwaltung. Belege, die in GiroCrab an der Buchung hängen, gehen mit. Gezogen wird die Buchung unter dem Mauszeiger, oder die ganze Mehrfachauswahl, wenn die Buchung zu ihr gehört. Dieselbe Geste legt Buchungen weiterhin auf eine Kategorie in der Seitenleiste (#366).

Die Belegverwaltung [Receipts Space](https://receipts-app.com/) liest dieses Format: Auf ihr Fenster gezogene Buchungen werden dort zu Einträgen mit Datum, Bezahldatum, Betrag, Gegenseite, IBAN und Verwendungszweck, samt Beleg, wenn die Buchung in GiroCrab einen hat; sonst fehlt dort nur noch der Beleg.

## Für Entwickler empfangender Apps

Der Zug enthält zwei Darstellungen:

- `de.girocrab.transactions` (konform zu `public.json`): die Buchungen als JSON, Format unten.
- `public.utf8-plain-text`: ein interner Text der Form `girocrab-bookings:<id>,<id>` für GiroCrab selbst. Andere Apps sollten ihn ignorieren.

```json
{
  "version": 1,
  "creator": "GiroCrab 0.8.7",
  "transactions": [
    {
      "id": 4711,
      "account_id": 3,
      "account_name": "Girokonto",
      "account_iban": "DE02120300000000202051",
      "booking_date": "2026-03-14",
      "value_date": "2026-03-14",
      "amount_cents": -4599,
      "currency": "EUR",
      "pending": false,
      "counterparty": "Stadtwerke",
      "counterparty_iban": "DE89370400440532013000",
      "purpose": "Abschlag Strom",
      "booking_text": "Lastschrift",
      "category_path": "Wohnen › Energie",
      "merchant": "Stadtwerke",
      "note": "…",
      "end_to_end_id": "…",
      "mandate_id": "…",
      "creditor_id": "…",
      "documents": [
        { "file_name": "Rechnung.pdf", "mime_type": "application/pdf", "title": "Rechnung 03/2026", "data": "JVBERi0xLjQK…" }
      ]
    }
  ]
}
```

- Die Feldnamen entsprechen der Methode `transactions` der [lokalen Schnittstelle](local-api.md), ergänzt um `account_name`, `account_iban`, `end_to_end_id`, `mandate_id` und `creditor_id`. Eine App kann beides mit demselben Code lesen.
- Beträge in Cent, negativ = Ausgang. Daten als `YYYY-MM-DD`; fehlt das Valutadatum, steht dort das Buchungsdatum.
- Leere Texte fehlen im JSON. Unbekannte Felder sind zu ignorieren; eine inkompatible Änderung erhöht `version`.
- `id` ist die Buchungs-ID dieses Tresors auf diesem Mac. Sie bleibt für die Lebensdauer der Buchung gleich und taugt als Verweis (z. B. `GiroCrab-4711`), ein anderes Gerät vergibt aber eigene IDs.
- `documents` fehlt bei Buchungen ohne Beleg. `data` ist die Datei in Base64. Ein Zug trägt höchstens 100 MB an Belegen; was darüber hinausgeht, bleibt weg. Gelesen werden die Belege erst, wenn die empfangende App die JSON-Darstellung abruft, und nur solange der Tresor entsperrt ist.
- Vorgemerkte Buchungen (`pending: true`) können sich beim nächsten Abruf noch ändern oder wegfallen.
