# PSD2 (Enable Banking) kostenlos für eigene Konten einrichten

Für Banken und Zahlungsdienste ohne FinTS-Zugang (z. B. **PayPal**, **N26**, **C24**, **Revolut**, **American Express**, **Amazon Visa** oder **Trade Republic** per PSD2) nutzt GiroCrab die Schnittstelle von [Enable Banking](https://enablebanking.com) (regulierter europäischer Kontoinformationsdienstleister / AISP).

Im sogenannten **Restricted Mode** ist Enable Banking für den Abruf **eigener Konten dauerhaft kostenlos** – deine Daten fließen dabei ohne fremdes GiroCrab-Backend direkt zwischen deinem Gerät und der Schnittstelle.

👉 **[Interaktive Schritt-für-Schritt-Anleitung öffnen (`girocrab.de`)](https://girocrab.de/girocrab_psd2_anleitung.html)**

---

## Schritt-für-Schritt-Anleitung (ca. 5 Minuten)

### 1. Kostenlos anmelden & Anwendung registrieren
Öffne [enablebanking.com/sign-in](https://enablebanking.com/sign-in/) (Anmeldung per E-Mail-Link, kein Passwort nötig – beim ersten Mal wird das Konto automatisch angelegt), wechsle zu [enablebanking.com/cp/applications](https://enablebanking.com/cp/applications) und fülle das Formular **Add a new application** wie folgt aus:

| Feld im Formular | Was du einträgst | Hintergrund |
| :--- | :--- | :--- |
| **1. Environment** | **`Production`** | Wichtig: *Sandbox* enthält nur Spielgeld-Testbanken. Echte Banken und PayPal funktionieren nur in *Production*. |
| **2. Private RSA key** | **`Generate in the browser (using SubtleCrypto) and export private key`** | Erzeugt deinen privaten Schlüssel lokal im Browser und lädt beim Speichern automatisch die Datei `<uuid>.pem` herunter. |
| **Application name** | z. B. `GiroCrab Personal` | Frei wählbar. |
| **Allowed redirect URLs** | `https://localhost/girocrab/callback` | **Exakt diese URL ohne Leerzeichen eintragen.** GiroCrab fängt die Weiterleitung im eingebauten Browserfenster automatisch ab (es läuft kein lokaler Server). |
| **Application description** | z. B. `Private Kontoverwaltung mit GiroCrab` | Kurzer Hinweis genügt. |
| **Email for data protection** | Deine eigene E-Mail-Adresse | Pflichtfeld im Formular. |
| **Privacy URL / Terms URL** | `https://girocrab.de` | Im Restricted Mode nicht geprüft; eine gültige `https://`-Adresse genügt. |

Klicke auf **Register**. Dein Browser lädt jetzt automatisch deine Schlüsseldatei **`<application-id>.pem`** in deinen `Downloads`-Ordner herunter.

> ⚠️ **Wichtig zur `.pem`-Datei:** Lege die Datei sicher ab (z. B. in einem eigenen Ordner unter *Dokumente*), gib sie niemandem weiter und **benenne sie nicht um** – GiroCrab liest die Application-ID direkt aus dem Dateinamen `<application-id>.pem`!

---

### 2. Eigene Konten freischalten („Activate by linking accounts“)
Nach dem Anlegen steht deine Anwendung zunächst auf **`Inactive`**.

1. Klicke bei deiner Anwendung auf den grünen Button **Activate by linking accounts** *(nicht auf „Request activation“ – das ist nur für kostenpflichtige Firmenverträge)*.
2. Wähle deine Bank bzw. PayPal aus:
   * **Country:** z. B. `Germany`
   * **ASPSP (Bank):** z. B. `PayPal`, `Trade Republic`, `N26`, `C24 Bank` oder `American Express`
   * **Usage type:** `personal` (bzw. `business` bei Geschäftskonten wie Finom)
3. Klicke auf **Link** und anschließend auf **Authentifizierung fortsetzen**.
4. Bestätige die Freigabe einmalig bei deiner Bank bzw. bei PayPal (z. B. Login + 2FA oder QR-Code in der Banking-App scannen → *„Enable Banking möchte auf deine Bank zugreifen“* → **Akzeptieren**).
5. Sobald das erste eigene Konto verknüpft ist, springt deine Anwendung im Control Panel von `Inactive` auf **`Active (Restricted)`**. Falls du weitere PSD2-Banken nutzen möchtest, wiederhole den Klick auf **Link** einfach für jede weitere Bank in derselben Anwendung.

---

### 3. Schlüsseldatei (`.pem`) in GiroCrab auswählen & anmelden
1. Öffne in GiroCrab **Bankverbindungen** → **Bankverbindung hinzufügen** und wähle deine Bank mit Schnittstelle **Enable Banking (PSD2)**.
   * **Hinweis für PayPal:** Wähle in GiroCrab **„PayPal (DE)“ über Enable Banking (PSD2)** *(nicht den als eingeschränkt markierten Eintrag „PayPal“ mit REST-API)*.
2. Klicke auf **„Schlüsseldatei (.pem) wählen …“** und wähle die heruntergeladene `<application-id>.pem` aus – GiroCrab übernimmt den privaten Schlüssel und die Application-ID automatisch aus dem Dateinamen.
3. Klicke optional auf **„Anwendung prüfen“**: GiroCrab bestätigt mit grünem Haken, dass *Production*, *Active*-Status und *Redirect-URL* passen.
4. Klicke auf **„Bei der Bank anmelden“** und bestätige im geöffneten Fenster den Lesezugriff auf Umsätze und Salden. GiroCrab fängt die Rückleitung auf `https://localhost/girocrab/callback` automatisch ab.
   *(Erscheint in einem externen Browser doch eine „Verbindung fehlgeschlagen“-Seite bei `localhost`, ist das normal: kopiere dann einfach die komplette Adresse aus der Browser-Adresszeile und füge sie in GiroCrab ein, falls danach gefragt wird.)*

---

## Wenn etwas nicht klappt

| Problem | Ursache & Lösung |
| :--- | :--- |
| **Keine Konten / `EB_NO_ACCOUNTS`** | Im Restricted Mode liefert Enable Banking nur Konten aus, die du in Schritt 2 im Control Panel aktiv verknüpft hast. Prüfe auf `enablebanking.com/cp/applications`, ob die Bank bzw. PayPal verknüpft ist – sonst Schritt 2 wiederholen (auch bei abgelaufener Zustimmung). |
| **„Anwendung prüfen“ schlägt fehl** | Typische Ursachen: *Sandbox* statt *Production* gewählt, Anwendung steht noch auf *Inactive*, `https://localhost/girocrab/callback` enthält Tippfehler/Leerzeichen oder die `.pem`-Datei wurde umbenannt. |
| **Zugriff klappt nach einiger Zeit nicht mehr** | PSD2-Zustimmungen laufen gesetzlich nach 90–180 Tagen ab. Melde die Bankverbindung in GiroCrab einfach einmal neu an. |

---

## Häufige Fragen

* **Wie oft muss ich die 2FA bestätigen?**
  Einmalig beim Verknüpfen und danach gesetzlich bedingt (PSD2-Richtlinie) nur noch alle **90 bis 180 Tage** zur Erneuerung der Einwilligung. Alle täglichen Kontoabrufe dazwischen laufen ohne erneute 2FA durch. Über PSD2 hat GiroCrab reinen Lesezugriff (keine Überweisungen).
* **Welche Konten liefert PSD2?**
  Nach PSD2 sind Anbieter verpflichtet, Zahlungskonten (Girokonten, Kreditkarten, PayPal) bereitzustellen. Ob Tagesgeld- oder Sparkonten mitgeliefert werden, entscheidet die jeweilige Bank; Wertpapier-Depotbestände sind über PSD2/XS2A grundsätzlich nicht enthalten (nutze für Depots in GiroCrab FinTS oder die direkten Broker-Connectoren).
