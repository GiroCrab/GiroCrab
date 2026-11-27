# Buchungsinspektor

Der Inspektor zeigt die Details der ausgewählten Buchung. Auf dem Mac und in breiten iOS-Fenstern steht er rechts neben der Buchungsliste. In schmalen iOS-Fenstern öffnet er sich als Sheet mit Buchungstitel und „Fertig“. Bei Bedienungshilfen-Schriftgrößen öffnet sich das Sheet in voller Höhe.

## Aufbau

| Abschnitt in Standardreihenfolge | Inhalt und Aktionen |
|---|---|
| Buchung | Händler, Konto, Zahlungsart, Gegenkonto, Betrag, Datum und Verwendungszweck. Der Stift neben dem Händlernamen öffnet den Editor für Gegenpartei und Verwendungszweck. „Geprüft“ ändert den Prüfstatus. |
| Notiz | Freitext zur Buchung. Das Feld speichert beim Verlassen den Entwurf für die Buchung, in der er begonnen wurde. |
| Tags | Zugewiesene Tags, Eingabefeld und Vorschläge zum Hinzufügen. |
| Gehalt / Rente | Einordnung von Zuflüssen als Gehalt oder Rente. |
| Wiederkehrend | Erkannte Serie oder Aktion zum Markieren der Buchung als wiederkehrend. |
| Kategorie | Aktuelle Kategorie, Vorschlag aus der Historie, Auswahl bis zur Unterkategorie, Splitbuchung und Regel. Auf dem Mac öffnen `C` und `R` aus der Buchungsliste die Kategorieauswahl und den Regeleditor. |
| Belege & Abrechnungen | Verknüpfte Dokumente und Aktionen zur Zuordnung. |
| Händler-Historie | Anzahl, Beträge und Zeitraum bisheriger Buchungen beim Händler. |
| SEPA & Bank-Rohdaten | Aufklappbare Originalangaben der Bank. |

Mit „Anpassen“ lassen sich Abschnitte verschieben, aus- und einblenden und auf die Standardreihenfolge zurücksetzen. Die Einstellung gilt pro Gerät. Bei Mehrfachauswahl zeigt der Inspektor Anzahl, Summen je Währung und Sammelaktionen.

Auf dem Mac öffnet ein Klick auf die Kategorie in der Buchungszeile eine scrollbare Liste mit Haupt- und Unterkategorien. Gehört die angeklickte Buchung zur Mehrfachauswahl, gilt die Änderung für alle ausgewählten Buchungen. „Ohne Kategorie“ entfernt die Zuordnung. In der Kompaktansicht öffnet der Klick auf die Kategorie die Auswahl und lässt die Zeilendetails in ihrem aktuellen Zustand.

## Darstellung und Prüfung

- Abschnitte sitzen auf Systemflächen und werden durch Trennlinien gegliedert. Hervorhebungen kennzeichnen eine Auswahl oder einen Vorschlag.
- IBANs sind gleichbreit gesetzt. Bei einem eigenen Gegenkonto bleibt dessen Name proportional gesetzt.
- Händlername, Kontoname, IBAN, Zweck, Notiz und Beträge sind sensibel. Auch der Sheet-Titel wird bei der Schwärzung berücksichtigt.
- Änderungen mit Demo-Daten auf dem Mac und iPhone prüfen. Dazu gehören Hell- und Dunkelmodus, ein schmales Fenster, große Schrift und „Problem melden“ mit Schwärzung.

Gestaltungsregeln stehen in [DESIGN.md](../DESIGN.md). Offene Befunde und Prüfschritte stehen im [UI-Audit des Buchungsinspektors](ui-audits/buchungen.md#buchungsinspektor).
