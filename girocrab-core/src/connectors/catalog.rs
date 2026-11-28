//! Bank catalog (`BankDescriptor` per `ConnectorKind`) and connector construction.

use crate::i18n::tr;
use crate::models::ConnectorKind;

use super::*;

/// The supported institutions with their public protocol facts.
pub fn bank_catalog() -> Vec<BankDescriptor> {
    ConnectorKind::all().into_iter().map(descriptor).collect()
}

pub fn descriptor(kind: ConnectorKind) -> BankDescriptor {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    match kind {
        ConnectorKind::Advanzia => advanzia::bank_descriptor(kind),
        ConnectorKind::Edenred => BankDescriptor {
            kind,
            name: "Edenred".into(),
            protocol: tr("Direkt-Login (Web-Session)").into(),
            bank_code: String::new(),
            sf_symbol: "fork.knife".into(),
            login_label: tr("Kartenbezeichnung").into(),
            secret_label: tr("(Anmeldung im Browser-Fenster)").into(),
            required_settings: vec![],
            note: tr("Edenred-Benefitkarten (z. B. Essenszuschuss, Sachbezug): Guthaben und Kartenumsätze aus nutzer.edenred.de. Anmeldung im eingebetteten Browser-Fenster wie gewohnt. Inoffiziell: liest die Daten, die das Portal selbst lädt, Änderungen sind möglich.").into(),
        },
        ConnectorKind::AirPlus => airplus::bank_descriptor(kind),
        ConnectorKind::VolksbankKasselGoettingen => BankDescriptor {
            kind,
            name: "Volksbank Kassel Göttingen".into(),
            protocol: "FinTS 3.0 PIN/TAN".into(),
            bank_code: "52090000".into(),
            sf_symbol: "building.columns.fill".into(),
            login_label: tr("VR-NetKey oder Alias").into(),
            secret_label: tr("Online-Banking-PIN").into(),
            required_settings: s(&["fints.product_id", "fints.url.52090000"]),
            note: tr("Freigabe in der SecureGo-plus-App (Decoupled, TAN-Prozess 4, HKTAN v7). Server: fints1.atruvia.de").into(),
        },
        ConnectorKind::Direkt1822 => BankDescriptor {
            kind,
            name: "1822direkt".into(),
            protocol: "FinTS 3.0 PIN/TAN".into(),
            bank_code: "50050201".into(),
            sf_symbol: "building.columns".into(),
            login_label: tr("Anmeldename / Kontonummer").into(),
            secret_label: tr("Online-Banking-PIN").into(),
            required_settings: s(&["fints.product_id", "fints.url.50050201"]),
            note: tr("Nur lesend über FinTS (Umsätze, Salden, Depot, Dokumente): 1822direkt hat SEPA-Überweisungen über FinTS seit 09/2025 abgeschaltet. pushTAN-App (Decoupled) oder chipTAN. Server: banking-hs6.s-fints-pt-hs.de").into(),
        },
        ConnectorKind::FinTsGeneric => BankDescriptor {
            kind,
            name: tr("FinTS-Bank").into(),
            protocol: "FinTS 3.0 PIN/TAN".into(),
            bank_code: String::new(),
            sf_symbol: "building.columns".into(),
            login_label: tr("Anmeldename / VR-NetKey / Kontonummer").into(),
            secret_label: tr("Online-Banking-PIN").into(),
            required_settings: s(&["fints.product_id"]),
            note: tr("Beliebige FinTS-3.0-Bank (PIN/TAN). BLZ und FinTS-URL stehen im Bankzugang.").into(),
        },
        ConnectorKind::Comdirect => BankDescriptor {
            kind,
            name: "comdirect".into(),
            protocol: tr("comdirect REST-API (OAuth2 + Session-TAN)").into(),
            bank_code: "20041111".into(),
            sf_symbol: "building.2".into(),
            login_label: tr("Zugangsnummer").into(),
            secret_label: "PIN".into(),
            required_settings: s(&["comdirect.client_id", "comdirect.client_secret"]),
            note: tr("Session-TAN per photoTAN-Push, photoTAN-Grafik oder mobileTAN.").into(),
        },
        ConnectorKind::PayPal => BankDescriptor {
            kind,
            name: "PayPal".into(),
            protocol: tr("PayPal Transaction Search REST-API").into(),
            bank_code: String::new(),
            sf_symbol: "p.circle".into(),
            login_label: tr("E-Mail (nur Anzeige)").into(),
            secret_label: tr("(nicht nötig – nutzt Ihre REST-App)").into(),
            required_settings: s(&["paypal.client_id", "paypal.client_secret"]),
            note: tr("OAuth2 Client Credentials (REST-App) oder API-Signatur (klassische NVP-API, unter Erweitert); Historie in 31-Tage-Fenstern.").into(),
        },
        ConnectorKind::TradeRepublic => BankDescriptor {
            kind,
            name: "Trade Republic".into(),
            protocol: tr("WebSocket-API + 2FA").into(),
            bank_code: String::new(),
            sf_symbol: "chart.line.uptrend.xyaxis.circle".into(),
            login_label: tr("Telefonnummer (+49…)").into(),
            secret_label: tr("4-stellige App-PIN").into(),
            required_settings: vec![],
            note: tr("Web-Login mit 4-stelligem Code aus der App/SMS. Inoffizielle Schnittstelle – kann sich jederzeit ändern.").into(),
        },
        ConnectorKind::AmericanExpress => BankDescriptor {
            kind,
            name: "American Express".into(),
            protocol: tr("Direkt-Login (Web-Session) oder CSV/PDF-Kontoauszug").into(),
            bank_code: String::new(),
            sf_symbol: "creditcard.fill".into(),
            login_label: tr("Kartenbezeichnung").into(),
            secret_label: tr("(Anmeldung im Browser-Fenster)").into(),
            required_settings: vec![],
            note: tr("Anmeldung im eingebetteten Browser (Benutzername, Passwort, 2FA wie gewohnt) – keine API-Schlüssel nötig. Optional: eigene PSD2-/Enable-Banking-API unter „Erweitert“.").into(),
        },
        ConnectorKind::AmazonVisa => BankDescriptor {
            kind,
            name: "Amazon Visa".into(),
            protocol: tr("Direkt-Login (Web-Session) oder CSV/PDF-Kontoauszug").into(),
            bank_code: String::new(),
            sf_symbol: "creditcard".into(),
            login_label: tr("Kartenbezeichnung").into(),
            secret_label: tr("(Anmeldung im Browser-Fenster)").into(),
            required_settings: vec![],
            note: tr("Herausgeber: Open Bank S.A. (Zinia). Anmeldung im eingebetteten Browser – keine API-Schlüssel nötig. Optional: eigene PSD2-/Enable-Banking-API unter „Erweitert“.").into(),
        },
        ConnectorKind::EquatePlus => BankDescriptor {
            kind,
            name: "EquatePlus".into(),
            protocol: tr("Direkt-Login (Web-Session)").into(),
            bank_code: String::new(),
            sf_symbol: "person.crop.rectangle.stack".into(),
            login_label: tr("Zugangsbezeichnung").into(),
            secret_label: tr("(Anmeldung im Browser-Fenster)").into(),
            required_settings: vec![],
            note: tr("Mitarbeiteraktien und Programme mit verfügbaren und gesperrten Beständen. Anmeldung im Browserfenster mit EquateAccess oder SMS. Inoffizielle Portal-Schnittstelle; Änderungen und Einschränkungen durch Nutzungsbedingungen sind möglich. PDF-Dokumente lassen sich optional laden; sie liegen unter „Belege & Postbox“ und werden nicht automatisch Umsätzen oder Aktivitäten zugeordnet.").into(),
        },
        ConnectorKind::Quirion => BankDescriptor {
            kind,
            name: "quirion".into(),
            protocol: tr("Direkt-Login (Web-Session)").into(),
            bank_code: "10110600".into(),
            sf_symbol: "chart.pie".into(),
            login_label: tr("E-Mail (nur Anzeige)").into(),
            secret_label: tr("(Anmeldung im Browser-Fenster)").into(),
            required_settings: vec![],
            note: tr("quirion: Depotpositionen, Liquiditätskonto, Verrechnungskonto und gebuchte Umsätze. Anmeldung im Browserfenster. Interne Portal-API; Änderungen sind möglich.").into(),
        },
        ConnectorKind::Sharinbox => BankDescriptor {
            kind,
            name: "Sharinbox (Société Générale)".into(),
            protocol: tr("Direkt-Login (Web-Session)").into(),
            bank_code: String::new(),
            sf_symbol: "chart.pie".into(),
            login_label: tr("Zugangsbezeichnung").into(),
            secret_label: tr("(Anmeldung im Browser-Fenster)").into(),
            required_settings: vec![],
            note: tr("Sharinbox (Société Générale): Mitarbeiteraktien mit Stückzahl und Wert, optional die PDF-Dokumente aus der Postbox. Anmeldung im Browserfenster mit E-Mail, Passwort und Authenticator-App. Interne Portal-Seiten; Änderungen sind möglich.").into(),
        },
        ConnectorKind::AxaShareplan => BankDescriptor {
            kind,
            name: "AXA Mitarbeiter Shareplan".into(),
            protocol: tr("Direkt-Login (Web-Session)").into(),
            bank_code: String::new(),
            sf_symbol: "chart.line.uptrend.xyaxis".into(),
            login_label: tr("Zugangsbezeichnung").into(),
            secret_label: tr("(Anmeldung im Browser-Fenster)").into(),
            required_settings: vec![],
            note: tr("AXA Mitarbeiter Shareplan: Anteile des Mitarbeiterbeteiligungsplans (PEEG) mit Stückzahl, Einstandspreis und Wert, eine Position je Plan. Anmeldung im Browserfenster mit SMS-Code. Interne Portal-Schnittstelle; Änderungen sind möglich.").into(),
        },
        ConnectorKind::UnionInvestment => BankDescriptor {
            kind,
            name: "Union Investment".into(),
            protocol: tr("Direkt-Login (Web-Session)").into(),
            bank_code: String::new(),
            sf_symbol: "chart.pie".into(),
            login_label: tr("Zugangsbezeichnung").into(),
            secret_label: tr("(Anmeldung im Browser-Fenster)").into(),
            required_settings: vec![],
            note: tr("Union Investment (UnionDepot): Depotwert, Fondspositionen und PDF-Postbox. Anmeldung im Browserfenster mit SMS-TAN. Interne Portal-Schnittstelle; Änderungen sind möglich.").into(),
        },
        ConnectorKind::BerghausCie => BankDescriptor {
            kind,
            name: "Berghaus & Cie (Weindepot)".into(),
            protocol: tr("Direkt-Login (Web-Session)").into(),
            bank_code: String::new(),
            sf_symbol: "wineglass".into(),
            login_label: tr("Zugangsbezeichnung").into(),
            secret_label: tr("(Anmeldung im Browser-Fenster)").into(),
            required_settings: vec![],
            note: tr("Berghaus & Cie: Weindepot mit Flaschen, Einstandspreis und aktueller Bewertung als Depotpositionen. Anmeldung im Browserfenster mit E-Mail und Passwort. Interne Portal-Schnittstelle; Änderungen sind möglich.").into(),
        },
        ConnectorKind::Easybank => BankDescriptor {
            kind,
            name: "easybank".into(),
            protocol: tr("Online-Banking-API (Direkt-Login)").into(),
            bank_code: String::new(),
            sf_symbol: "creditcard".into(),
            login_label: tr("Benutzername").into(),
            secret_label: tr("Passwort").into(),
            required_settings: vec![],
            note: tr("easybank (ehemals Barclays Deutschland): Kreditkarten und Umsätze der letzten zwei Jahre. Beim ersten Abruf fragt easybank eine SMS-TAN ab, danach erkennt die Bank GiroCrab wieder. Interne Portal-API; Änderungen sind möglich.").into(),
        },
        ConnectorKind::AlteLeipziger => BankDescriptor {
            kind,
            name: "Alte Leipziger".into(),
            protocol: tr("fin4u-Kundenportal (app.fin4u.de)").into(),
            bank_code: "50012800".into(),
            sf_symbol: "house.circle".into(),
            login_label: tr("fin4u-Benutzername (E-Mail)").into(),
            secret_label: tr("fin4u-Passwort").into(),
            required_settings: vec![],
            note: tr("Bausparverträge und Darlehen der Alte Leipziger über die Schnittstelle des fin4u-Portals. Inoffizielle Nutzung – kann sich ändern.").into(),
        },
        ConnectorKind::VolkswagenBank => BankDescriptor {
            kind,
            name: "Volkswagen Bank".into(),
            protocol: tr("Online-Banking-Portal (Direkt-Login, photoTAN/pushTAN)").into(),
            bank_code: String::new(),
            sf_symbol: "building.columns".into(),
            login_label: tr("Benutzername").into(),
            secret_label: tr("Passwort / PIN").into(),
            required_settings: vec![],
            note: tr("Volkswagen Bank: Konten, Salden und Umsätze; die Anmeldung wird per pushTAN in der Volkswagen-Bank-App freigegeben. photoTAN wird noch nicht unterstützt. Interne Portal-API; Änderungen sind möglich.").into(),
        },
        ConnectorKind::Uberspace => BankDescriptor {
            kind,
            name: "Uberspace".into(),
            protocol: tr("Web-Dashboard (Direkt-Login)").into(),
            bank_code: String::new(),
            sf_symbol: "server.rack".into(),
            login_label: tr("E-Mail-Adresse").into(),
            secret_label: tr("Passwort").into(),
            required_settings: vec![],
            note: tr("Uberspace-Hosting: Guthaben und Buchungen aller Asteroids, die zur E-Mail-Adresse des Dashboards gehören. Inoffiziell: liest die Seiten des Dashboards, Änderungen sind möglich.").into(),
        },
        ConnectorKind::BitcoinCore => BankDescriptor {
            kind,
            name: "Bitcoin Core".into(),
            protocol: tr("Eigener Knoten (lokale JSON-RPC-Schnittstelle)").into(),
            bank_code: String::new(),
            sf_symbol: "bitcoinsign.circle".into(),
            login_label: tr("Wallet-Name (leer = einzige geladene Wallet)").into(),
            secret_label: tr("(nicht nötig – nutzt die .cookie-Datei des Knotens)").into(),
            required_settings: vec![],
            note: tr("Bitcoin Core: der Bestand einer Wallet im eigenen Knoten als Depot mit der Position Bitcoin, bewertet zum aktuellen Euro-Kurs. Der Knoten muss beim Abruf laufen, mit eingeschaltetem RPC-Server (server=1); die Anmeldung erfolgt über seine .cookie-Datei. Über Adressen und Bestand verlässt nichts den Rechner, nur der Kurs wird öffentlich abgefragt.").into(),
        },
        ConnectorKind::Monefit => BankDescriptor {
            kind,
            name: "Monefit SmartSaver".into(),
            protocol: tr("Web-App (Direkt-Login)").into(),
            bank_code: String::new(),
            sf_symbol: "banknote".into(),
            login_label: tr("E-Mail-Adresse").into(),
            secret_label: tr("Passwort").into(),
            required_settings: vec![],
            note: tr("Monefit SmartSaver: Saldo und alle Umsätze des Sparkontos. Inoffiziell: nutzt die Schnittstelle der Web-App, Änderungen sind möglich. Eine Anmeldung mit zweitem Faktor wird noch nicht unterstützt.").into(),
        },
        ConnectorKind::GoAndGrow => BankDescriptor {
            kind,
            name: "Go & Grow".into(),
            protocol: tr("Web-App (Direkt-Login)").into(),
            bank_code: String::new(),
            sf_symbol: "leaf".into(),
            login_label: tr("E-Mail-Adresse").into(),
            secret_label: tr("Passwort").into(),
            required_settings: vec![],
            note: tr("Go & Grow (ehemals Bondora): Saldo und Umsätze. Die Schnittstelle liefert nur die letzten zehn Buchungen (Zinsen, Ein- und Auszahlungen); ältere lassen sich nur per Import übernehmen. Darum abrufen, bevor mehr als zehn neue Buchungen anfallen – bei täglichen Zinsen ohne weitere Buchungen also spätestens alle zehn Tage. Inoffiziell: nutzt die Schnittstelle der Web-App, Änderungen sind möglich.").into(),
        },
        ConnectorKind::Bhw => BankDescriptor {
            kind,
            name: "BHW Bausparkasse".into(),
            protocol: tr("myBHW-Portal (Direkt-Login, Einmal-Code per E-Mail oder SMS)").into(),
            bank_code: "25410200".into(),
            sf_symbol: "house".into(),
            login_label: tr("Vertragsnummer oder BHW-ID").into(),
            secret_label: tr("Vorname;TT.MM.JJJJ;PLZ").into(),
            required_settings: vec![],
            note: tr("BHW Bausparkasse: Bausparverträge und Darlehen mit Saldo und Umsätzen. Anmeldung mit der 10-stelligen Vertragsnummer oder der 11-stelligen BHW-ID; statt eines Passworts kommen Vorname, Geburtsdatum und Postleitzahl getrennt durch Semikolon in das zweite Feld (z. B. „Erika;31.12.1970;12345“). Jede Anmeldung wird mit einem Einmal-Code per E-Mail oder SMS bestätigt. Dokumente aus dem myBHW-Postfach lassen sich abrufen. Inoffiziell: nutzt die Schnittstelle des myBHW-Portals, Änderungen sind möglich.").into(),
        },
        ConnectorKind::HanseaticBank => BankDescriptor {
            kind,
            name: "Hanseatic Bank".into(),
            protocol: tr("Online-Banking (Direkt-Login, Freigabe in der App)").into(),
            bank_code: "20120700".into(),
            sf_symbol: "creditcard".into(),
            login_label: tr("Kundennummer").into(),
            secret_label: tr("Passwort").into(),
            required_settings: vec![],
            note: tr("Hanseatic Bank: Kreditkarten mit vorgemerkten Umsätzen sowie SparBriefe, Tages- und Festgeld mit Saldo und Umsätzen. Ältere Umsätze, für die die Bank eine zusätzliche Freigabe verlangt, werden nicht abgerufen. Die erste Anmeldung wird in der App der Hanseatic Bank (oder per smsTAN) bestätigt; danach merkt sich GiroCrab das Gerät. Kontoauszüge und weitere Dokumente aus der Postbox lassen sich abrufen; ungelesene Dokumente gelten danach bei der Bank als gelesen. Inoffiziell: nutzt die Schnittstelle des Online-Bankings, Änderungen sind möglich.").into(),
        },
        ConnectorKind::Kontist => BankDescriptor {
            kind,
            name: "Kontist".into(),
            protocol: tr("Kontist GraphQL-API (OAuth2 + Freigabe in der App)").into(),
            bank_code: String::new(),
            sf_symbol: "briefcase".into(),
            login_label: tr("E-Mail-Adresse").into(),
            secret_label: tr("Passwort").into(),
            required_settings: s(&["kontist.client_id"]),
            note: tr("Kontist: Geschäftskonto mit Saldo und Umsätzen über die offizielle API. Nötig ist ein eigener API-Client auf kontist.dev (Client Management), dessen Client-ID in den Einstellungen eingetragen wird. Die erste Anmeldung wird in der Kontist-App bestätigt, danach läuft der Abruf ohne Freigabe, solange Kontist die Anmeldung gelten lässt.").into(),
        },
        ConnectorKind::OpelDirektbank => BankDescriptor {
            kind,
            name: "Opel Direktbank".into(),
            protocol: tr("Online-Banking (Direkt-Login, TwoVerify-App)").into(),
            bank_code: "50025000".into(),
            sf_symbol: "banknote".into(),
            login_label: tr("Benutzerkennung").into(),
            secret_label: tr("Passwort").into(),
            required_settings: vec![],
            note: tr("Opel Direktbank: Tagesgeld- und Festgeldkonten mit Saldo und Umsätzen. Die Anmeldung wird in der TwoVerify-App bestätigt. Kontoauszüge und weitere Dokumente aus der Postbox lassen sich abrufen; ungelesene Dokumente gelten danach bei der Bank als gelesen. Inoffiziell: liest die Seiten des Online-Bankings, Änderungen sind möglich.").into(),
        },
        ConnectorKind::FordMoney => BankDescriptor {
            kind,
            name: "Ford Money".into(),
            protocol: tr("Online-Banking (Direkt-Login, Security-App)").into(),
            bank_code: "37021548".into(),
            sf_symbol: "banknote".into(),
            login_label: tr("Kundennummer").into(),
            secret_label: tr("Passwort").into(),
            required_settings: vec![],
            note: tr("Ford Money (Ford Bank): Tagesgeld- und Festgeldkonten mit Saldo und Umsätzen. Die Anmeldung wird in der Ford-Money-Security-App bestätigt. Kontoauszüge und weitere Dokumente aus der Postbox lassen sich abrufen; ungelesene Dokumente gelten danach bei Ford Money als gelesen. Inoffiziell: liest die Seiten des Online-Bankings, Änderungen sind möglich.").into(),
        },
        ConnectorKind::Bitcoin => BankDescriptor {
            kind,
            name: "Bitcoin-Wallet (xpub)".into(),
            protocol: tr("Bitcoin-Wallet (nur lesen, xpub/ypub/zpub)").into(),
            bank_code: "".into(),
            sf_symbol: "bitcoinsign.circle".into(),
            login_label: tr("Erweiterter öffentlicher Schlüssel (xpub, ypub oder zpub)").into(),
            secret_label: tr("(kein Passwort nötig)").into(),
            required_settings: vec![],
            note: tr("Nur lesen: Mit dem erweiterten öffentlichen Schlüssel des Wallet-Kontos (z. B. m/84'/0'/0') werden Empfangs- und Wechselgeld-Adressen abgeleitet; damit lässt sich nichts ausgeben. Die Umsätze kommen von mempool.space oder einem eigenen Esplora-Server (Einstellungen der Verbindung); dieser Server sieht alle Adressen des Wallets. Der Euro-Kurs kommt von mempool.space. Niemals den privaten Schlüssel (xprv) oder die Wiederherstellungswörter eingeben.").into(),
        },
        ConnectorKind::Finpension => BankDescriptor {
            kind,
            name: "Finpension".into(),
            protocol: tr("finpension Web-API (SMS-2FA)").into(),
            bank_code: "".into(),
            sf_symbol: "chart.pie".into(),
            login_label: tr("Handynummer (z. B. +41… oder +49…)").into(),
            secret_label: tr("Passwort").into(),
            required_settings: vec![],
            note: tr("Finpension (Schweiz): Säule 3a, Freizügigkeit und 1e. Synchronisiert Portfolios, Wertschriften und Cash-Bestände via finpension.ch. SMS-Code bei der Erstanmeldung erforderlich; die Sitzung wird verschlüsselt gespeichert.").into(),
        },
        ConnectorKind::Ffb => BankDescriptor {
            kind,
            name: "FFB (FIL Fondsbank)".into(),
            protocol: tr("Web-Kundenbereich (Direkt-Login)").into(),
            bank_code: "50021120".into(),
            sf_symbol: "chart.pie".into(),
            login_label: tr("Login / Benutzername").into(),
            secret_label: tr("Passwort / PIN").into(),
            required_settings: vec![],
            note: tr("FFB-Fondsdepot: Fondsbestände (auch VL-Sparverträge) und Depotumsätze wie Sparplankäufe, Erträge und Entgelte. Das Abwicklungskonto wird nicht abgerufen. Depot- und Kontoauszüge, Abrechnungen und Steuerunterlagen aus dem Postfach lassen sich abrufen; ungelesene Dokumente können danach bei der FFB als gelesen gelten. Nur Lesezugriff. Inoffiziell: liest die Seiten des Kundenbereichs, Änderungen sind möglich.").into(),
        },
        ConnectorKind::SchwaebischHall => BankDescriptor {
            kind,
            name: "Schwäbisch Hall".into(),
            protocol: tr("Direkt-Login (Web-Session)").into(),
            bank_code: "62220000".into(),
            sf_symbol: "house.circle".into(),
            login_label: tr("Bezeichnung (z. B. Kontoname)").into(),
            secret_label: tr("(Anmeldung im Browser-Fenster)").into(),
            required_settings: vec![],
            note: tr("Bausparkasse Schwäbisch Hall: Bausparverträge mit Saldo und Umsätzen, optional die PDF-Dokumente aus dem Postfach. Anmeldung im Browserfenster von „Mein Konto“. Interne Portal-API; Änderungen sind möglich.").into(),
        },
        ConnectorKind::Fnz => BankDescriptor {
            kind,
            name: "FNZ Bank".into(),
            protocol: tr("Direkt-Login (Zugangsnummer + PIN)").into(),
            bank_code: "70013000".into(),
            sf_symbol: "building.columns".into(),
            login_label: tr("Zugangsnummer").into(),
            secret_label: tr("PIN").into(),
            required_settings: vec![],
            note: tr(
                "FNZ Bank / ebase / finvesto: Kontosalden, Fondspositionen, Konto- und Depotumsätze sowie PDF-Postkorb einschließlich Archiv. Direkt-Login mit Zugangsnummer und PIN; wird „PIN im Tresor speichern“ abgewählt, gilt die PIN nur für diesen Abruf. Danach die Freigabe in der FNZsecure-App. Ohne Zugangsdaten öffnet sich das Browserfenster.",
            )
            .into(),
        },
        ConnectorKind::AmazonOrders => BankDescriptor {
            kind,
            name: "Amazon Bestellungen".into(),
            protocol: tr("Direkt-Login (Web-Session)").into(),
            bank_code: String::new(),
            sf_symbol: "shippingbox.fill".into(),
            login_label: tr("Kontobezeichnung").into(),
            secret_label: tr("(Anmeldung im Browser-Fenster)").into(),
            required_settings: vec![],
            note: tr("Liest Ihre Bestellungen aus dem Amazon-Kundenkonto (amazon.de). Anmeldung im eingebetteten Browser inkl. Captcha/2FA – keine Zugangsdaten werden gespeichert.").into(),
        },
        ConnectorKind::EnableBanking => BankDescriptor {
            kind,
            name: tr("Bank per PSD2 (Enable Banking)").into(),
            protocol: tr("PSD2 über Enable Banking (eigene Anwendung)").into(),
            bank_code: String::new(),
            sf_symbol: "building.columns.circle.fill".into(),
            login_label: tr("Bezeichnung (z. B. Kontoname)").into(),
            secret_label: tr("(Zustimmung im Browser)").into(),
            required_settings: s(&[
                "enablebanking.application_id",
                "enablebanking.private_key_pem",
                "enablebanking.redirect_url",
            ]),
            note: tr("Banken wie N26, Revolut oder C24 per PSD2. Benötigt eine kostenlose eigene Anwendung auf enablebanking.com (Application-ID, privater Schlüssel, Redirect-URL). Die Zustimmung erteilen Sie im Browser Ihrer Bank.").into(),
        },
        ConnectorKind::ScalableCapital => BankDescriptor {
            kind,
            name: "Scalable Capital".into(),
            protocol: tr("OAuth-Geräteanmeldung + GraphQL-API der offiziellen CLI").into(),
            bank_code: String::new(),
            sf_symbol: "chart.bar.xaxis".into(),
            login_label: tr("E-Mail (nur Anzeige)").into(),
            secret_label: tr("(nicht nötig – Anmeldung im Browser)").into(),
            required_settings: vec![],
            note: tr("Vorher im Browser auf scalable.capital (nicht in der Scalable-App) unter Profil → Sicherheit → „Agentic Investing“ den CLI-Zugriff einschalten. Die Anmeldung läuft im Browser und muss alle 7 Tage erneuert werden. Nur Lesezugriff.").into(),
        },
        ConnectorKind::Ibkr => BankDescriptor {
            kind,
            name: "Interactive Brokers (IBKR)".into(),
            protocol: tr("Flex Web Service (Token + Flex Query)").into(),
            bank_code: String::new(),
            sf_symbol: "chart.line.uptrend.xyaxis".into(),
            login_label: tr("Flex-Token").into(),
            secret_label: tr("Flex-Query-ID").into(),
            required_settings: vec![],
            note: tr("Im IBKR Client Portal unter Performance & Reports → Flex Queries eine Activity Flex Query (XML) mit Kontoinformationen, Offenen Positionen, Cash-Transaktionen und Trades anlegen und unter Flex Web Service Configuration ein Token erzeugen. Nur Lesezugriff.").into(),
        },
        ConnectorKind::Bunq => BankDescriptor {
            kind,
            name: "bunq".into(),
            protocol: tr("bunq Public API (eigener API-Schlüssel)").into(),
            bank_code: String::new(),
            sf_symbol: "building.columns.circle".into(),
            login_label: tr("E-Mail (nur Anzeige)").into(),
            secret_label: tr("API-Schlüssel").into(),
            required_settings: vec![],
            note: tr("API-Schlüssel in der bunq-App unter Profil → Einstellungen → Entwickler → API-Schlüssel erstellen und „Alle IP-Adressen erlauben“ aktivieren. Keine PSD2-Registrierung nötig.").into(),
        },
    }
}

// ------------------------------------------------------------------ capability matrix

/// How GiroCrab talks to an institution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interface {
    /// `FinTS 3.0`
    Fints,
    /// `REST-API` (official bank/provider API)
    RestApi,
    /// `Enable Banking (PSD2)`
    EnableBanking,
    /// `Web-Portal` (embedded browser login, internal portal API)
    WebPortal,
    /// `Datei-Import` (export file chosen by the user, no login)
    FileImport,
}

/// What a connector can read or do. Only list what is implemented today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    /// Booked transactions.
    Transactions,
    /// Account balances.
    Balances,
    /// Depot / securities positions.
    Securities,
    /// Securities transactions of a depot (buys, sells, distributions as their own entries).
    SecurityTransactions,
    /// SEPA credit transfers (incl. SEPA Instant where the bank offers it).
    Transfers,
    /// Standing orders (not implemented by any connector yet).
    StandingOrders,
    /// PDF statements / post box documents.
    Documents,
}

/// Test status of a catalog entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Verified with real accounts.
    Verified,
    /// Implemented; feedback from real accounts wanted.
    TestingWanted,
    /// Works with known limitations (see `issues`).
    Limited,
}

/// One row of the public bank matrix (README, girocrab.de). Parsed by
/// `scripts/sync_docs.py`: keep the field order and literal style.
#[derive(Debug, Clone, Copy)]
pub struct MatrixEntry {
    pub name: &'static str,
    /// The connector behind this row; `None` for presets of a generic connector.
    pub kind: Option<ConnectorKind>,
    pub interface: Interface,
    /// Official interface of the bank/provider (`Some(false)`: unofficial, may conflict with
    /// its terms of service); `None` = not yet checked. Source of the notice in `docs/*-connector.md`.
    pub official: Option<bool>,
    pub capabilities: &'static [Capability],
    pub status: Status,
    /// GitHub issue numbers (feedback wanted / known limitation).
    pub issues: &'static [u32],
}

/// Curated institutions only (not the full FinTS directory). Bank-dependent
/// FinTS functions (securities, PDF statements) are offered where the bank's
/// BPD advertises them.
pub const BANK_MATRIX: &[MatrixEntry] = &[
    MatrixEntry {
        name: "Edenred",
        kind: Some(ConnectorKind::Edenred),
        interface: Interface::WebPortal,
        official: Some(false),
        capabilities: &[Capability::Transactions, Capability::Balances],
        status: Status::TestingWanted,
        issues: &[],
    },
    MatrixEntry {
        name: "Advanzia Bank",
        kind: Some(ConnectorKind::Advanzia),
        interface: Interface::WebPortal,
        official: Some(false),
        capabilities: &[Capability::Transactions, Capability::Balances],
        status: Status::TestingWanted,
        issues: &[177],
    },
    MatrixEntry {
        name: "Volksbank Kassel Göttingen",
        kind: Some(ConnectorKind::VolksbankKasselGoettingen),
        interface: Interface::Fints,
        official: Some(true),
        capabilities: &[
            Capability::Transactions,
            Capability::Balances,
            Capability::Securities,
            Capability::Transfers,
            Capability::Documents,
        ],
        status: Status::Verified,
        issues: &[],
    },
    MatrixEntry {
        name: "1822direkt",
        kind: Some(ConnectorKind::Direkt1822),
        interface: Interface::Fints,
        official: Some(true),
        capabilities: &[
            Capability::Transactions,
            Capability::Balances,
            Capability::Securities,
            Capability::Documents,
        ],
        status: Status::Verified,
        issues: &[],
    },
    MatrixEntry {
        name: "FinTS-Bank (BLZ + URL)",
        kind: Some(ConnectorKind::FinTsGeneric),
        interface: Interface::Fints,
        official: Some(true),
        capabilities: &[
            Capability::Transactions,
            Capability::Balances,
            Capability::Securities,
            Capability::Transfers,
            Capability::Documents,
        ],
        status: Status::Verified,
        issues: &[],
    },
    MatrixEntry {
        name: "ING",
        kind: None,
        interface: Interface::Fints,
        official: Some(true),
        capabilities: &[Capability::Transactions, Capability::Balances],
        status: Status::TestingWanted,
        issues: &[313],
    },
    MatrixEntry {
        name: "flatex (flatexDEGIRO Bank)",
        kind: None,
        interface: Interface::Fints,
        official: Some(true),
        capabilities: &[
            Capability::Transactions,
            Capability::Balances,
            Capability::Securities,
        ],
        status: Status::Verified,
        issues: &[],
    },
    MatrixEntry {
        name: "Consorsbank",
        kind: None,
        interface: Interface::Fints,
        official: Some(true),
        capabilities: &[
            Capability::Transactions,
            Capability::Balances,
            Capability::Securities,
        ],
        status: Status::Verified,
        issues: &[],
    },
    MatrixEntry {
        name: "Consorsbank (Online-Archiv, inoffiziell)",
        kind: None,
        interface: Interface::WebPortal,
        official: Some(false),
        capabilities: &[Capability::Documents],
        status: Status::Verified,
        issues: &[],
    },
    MatrixEntry {
        name: "Miles & More Kreditkarte (Deutsche Bank)",
        kind: None,
        interface: Interface::Fints,
        official: Some(true),
        capabilities: &[Capability::Transactions, Capability::Balances],
        status: Status::Verified,
        issues: &[],
    },
    MatrixEntry {
        name: "comdirect",
        kind: Some(ConnectorKind::Comdirect),
        interface: Interface::RestApi,
        official: Some(true),
        capabilities: &[
            Capability::Transactions,
            Capability::Balances,
            Capability::Securities,
            Capability::SecurityTransactions,
            Capability::Documents,
        ],
        status: Status::Verified,
        issues: &[],
    },
    MatrixEntry {
        name: "bunq",
        kind: Some(ConnectorKind::Bunq),
        interface: Interface::RestApi,
        official: Some(true),
        capabilities: &[
            Capability::Transactions,
            Capability::Balances,
            Capability::Transfers,
            Capability::Documents,
        ],
        status: Status::Verified,
        issues: &[],
    },
    MatrixEntry {
        name: "PayPal",
        kind: Some(ConnectorKind::PayPal),
        interface: Interface::RestApi,
        official: Some(true),
        capabilities: &[Capability::Transactions, Capability::Balances],
        status: Status::Limited,
        issues: &[156],
    },
    MatrixEntry {
        name: "Interactive Brokers (IBKR)",
        kind: Some(ConnectorKind::Ibkr),
        interface: Interface::RestApi,
        official: Some(true),
        capabilities: &[
            Capability::Transactions,
            Capability::Balances,
            Capability::Securities,
            Capability::SecurityTransactions,
        ],
        status: Status::TestingWanted,
        issues: &[],
    },
    MatrixEntry {
        name: "Scalable Capital",
        kind: Some(ConnectorKind::ScalableCapital),
        interface: Interface::RestApi,
        official: Some(false),
        capabilities: &[
            Capability::Transactions,
            Capability::Balances,
            Capability::Securities,
            Capability::SecurityTransactions,
        ],
        status: Status::TestingWanted,
        issues: &[45],
    },
    MatrixEntry {
        name: "Bank per PSD2 (Enable Banking)",
        kind: Some(ConnectorKind::EnableBanking),
        interface: Interface::EnableBanking,
        official: Some(true),
        capabilities: &[Capability::Transactions, Capability::Balances],
        status: Status::Limited,
        issues: &[],
    },
    MatrixEntry {
        name: "ING Business",
        kind: None,
        interface: Interface::EnableBanking,
        official: Some(true),
        capabilities: &[Capability::Transactions, Capability::Balances],
        status: Status::TestingWanted,
        issues: &[721],
    },
    MatrixEntry {
        name: "PayPal (DE)",
        kind: None,
        interface: Interface::EnableBanking,
        official: Some(true),
        capabilities: &[Capability::Transactions, Capability::Balances],
        status: Status::Verified,
        issues: &[],
    },
    MatrixEntry {
        name: "Bank Norwegian",
        kind: None,
        interface: Interface::EnableBanking,
        official: Some(true),
        capabilities: &[Capability::Transactions, Capability::Balances],
        status: Status::Verified,
        issues: &[],
    },
    MatrixEntry {
        name: "C24",
        kind: None,
        interface: Interface::EnableBanking,
        official: Some(true),
        capabilities: &[Capability::Transactions, Capability::Balances],
        status: Status::Verified,
        issues: &[],
    },
    MatrixEntry {
        name: "N26",
        kind: None,
        interface: Interface::EnableBanking,
        official: Some(true),
        capabilities: &[Capability::Transactions, Capability::Balances],
        status: Status::Verified,
        issues: &[],
    },
    MatrixEntry {
        name: "Revolut",
        kind: None,
        interface: Interface::EnableBanking,
        official: Some(true),
        capabilities: &[Capability::Transactions, Capability::Balances],
        status: Status::TestingWanted,
        issues: &[289],
    },
    MatrixEntry {
        name: "American Express",
        kind: Some(ConnectorKind::AmericanExpress),
        interface: Interface::EnableBanking,
        official: Some(true),
        capabilities: &[Capability::Transactions, Capability::Balances],
        status: Status::Verified,
        issues: &[],
    },
    MatrixEntry {
        name: "Amazon Visa",
        kind: Some(ConnectorKind::AmazonVisa),
        interface: Interface::EnableBanking,
        official: Some(true),
        capabilities: &[Capability::Transactions, Capability::Balances],
        status: Status::Verified,
        issues: &[],
    },
    MatrixEntry {
        name: "easybank (ehem. Barclays)",
        kind: Some(ConnectorKind::Easybank),
        interface: Interface::WebPortal,
        official: Some(false),
        capabilities: &[
            Capability::Transactions,
            Capability::Balances,
            Capability::Documents,
        ],
        status: Status::TestingWanted,
        issues: &[],
    },
    MatrixEntry {
        name: "Alte Leipziger (Bausparen)",
        kind: Some(ConnectorKind::AlteLeipziger),
        interface: Interface::WebPortal,
        official: Some(false),
        capabilities: &[
            Capability::Transactions,
            Capability::Balances,
            Capability::Documents,
        ],
        status: Status::Verified,
        issues: &[],
    },
    MatrixEntry {
        name: "Schwäbisch Hall (Bausparen)",
        kind: Some(ConnectorKind::SchwaebischHall),
        interface: Interface::WebPortal,
        official: Some(false),
        capabilities: &[
            Capability::Transactions,
            Capability::Balances,
            Capability::Documents,
        ],
        status: Status::TestingWanted,
        issues: &[290],
    },
    MatrixEntry {
        name: "Volkswagen Bank",
        kind: Some(ConnectorKind::VolkswagenBank),
        interface: Interface::WebPortal,
        official: Some(false),
        capabilities: &[Capability::Transactions, Capability::Balances],
        status: Status::Limited,
        issues: &[],
    },
    MatrixEntry {
        name: "Bitcoin Core",
        kind: Some(ConnectorKind::BitcoinCore),
        interface: Interface::RestApi,
        official: Some(true),
        capabilities: &[Capability::Balances, Capability::Securities],
        status: Status::TestingWanted,
        issues: &[],
    },
    MatrixEntry {
        name: "Monefit SmartSaver",
        kind: Some(ConnectorKind::Monefit),
        interface: Interface::WebPortal,
        official: Some(false),
        capabilities: &[Capability::Transactions, Capability::Balances],
        status: Status::TestingWanted,
        issues: &[],
    },
    MatrixEntry {
        name: "Go & Grow",
        kind: Some(ConnectorKind::GoAndGrow),
        interface: Interface::WebPortal,
        official: Some(false),
        capabilities: &[Capability::Transactions, Capability::Balances],
        status: Status::Verified,
        issues: &[],
    },
    MatrixEntry {
        name: "Uberspace",
        kind: Some(ConnectorKind::Uberspace),
        interface: Interface::WebPortal,
        official: Some(false),
        capabilities: &[Capability::Transactions, Capability::Balances],
        status: Status::Verified,
        issues: &[],
    },
    MatrixEntry {
        name: "Trade Republic",
        kind: Some(ConnectorKind::TradeRepublic),
        interface: Interface::WebPortal,
        official: Some(false),
        capabilities: &[
            Capability::Transactions,
            Capability::Balances,
            Capability::Securities,
            Capability::SecurityTransactions,
            Capability::Documents,
        ],
        status: Status::Limited,
        issues: &[],
    },
    MatrixEntry {
        name: "EquatePlus",
        kind: Some(ConnectorKind::EquatePlus),
        interface: Interface::WebPortal,
        official: Some(false),
        capabilities: &[Capability::Balances, Capability::Securities],
        status: Status::TestingWanted,
        issues: &[],
    },
    MatrixEntry {
        name: "Bitcoin-Wallet (xpub/ypub/zpub)",
        kind: Some(ConnectorKind::Bitcoin),
        interface: Interface::RestApi,
        official: Some(true),
        capabilities: &[
            Capability::Balances,
            Capability::Securities,
            Capability::SecurityTransactions,
        ],
        status: Status::TestingWanted,
        issues: &[],
    },
    MatrixEntry {
        name: "Finpension",
        kind: Some(ConnectorKind::Finpension),
        interface: Interface::WebPortal,
        official: Some(false),
        capabilities: &[
            Capability::Transactions,
            Capability::Balances,
            Capability::Securities,
        ],
        status: Status::TestingWanted,
        issues: &[300, 1068],
    },
    MatrixEntry {
        name: "BHW Bausparkasse",
        kind: Some(ConnectorKind::Bhw),
        interface: Interface::WebPortal,
        official: Some(false),
        capabilities: &[
            Capability::Transactions,
            Capability::Balances,
            Capability::Documents,
        ],
        status: Status::TestingWanted,
        issues: &[],
    },
    MatrixEntry {
        name: "Kontist",
        kind: Some(ConnectorKind::Kontist),
        interface: Interface::RestApi,
        official: Some(true),
        capabilities: &[Capability::Transactions, Capability::Balances],
        status: Status::Verified,
        issues: &[],
    },
    MatrixEntry {
        name: "Opel Direktbank",
        kind: Some(ConnectorKind::OpelDirektbank),
        interface: Interface::WebPortal,
        official: Some(false),
        capabilities: &[
            Capability::Transactions,
            Capability::Balances,
            Capability::Documents,
        ],
        status: Status::TestingWanted,
        issues: &[],
    },
    MatrixEntry {
        name: "Ford Money",
        kind: Some(ConnectorKind::FordMoney),
        interface: Interface::WebPortal,
        official: Some(false),
        capabilities: &[
            Capability::Transactions,
            Capability::Balances,
            Capability::Documents,
        ],
        status: Status::TestingWanted,
        issues: &[],
    },
    MatrixEntry {
        name: "FFB (FIL Fondsbank)",
        kind: Some(ConnectorKind::Ffb),
        interface: Interface::WebPortal,
        official: Some(false),
        capabilities: &[
            Capability::Transactions,
            Capability::Balances,
            Capability::Securities,
            Capability::SecurityTransactions,
            Capability::Documents,
        ],
        status: Status::TestingWanted,
        issues: &[],
    },
    MatrixEntry {
        name: "quirion",
        kind: Some(ConnectorKind::Quirion),
        interface: Interface::WebPortal,
        official: Some(false),
        capabilities: &[
            Capability::Transactions,
            Capability::Balances,
            Capability::Securities,
            Capability::Documents,
        ],
        status: Status::Verified,
        issues: &[],
    },
    MatrixEntry {
        name: "Sharinbox (Société Générale)",
        kind: Some(ConnectorKind::Sharinbox),
        interface: Interface::WebPortal,
        official: Some(false),
        capabilities: &[
            Capability::Balances,
            Capability::Securities,
            Capability::Documents,
        ],
        status: Status::TestingWanted,
        issues: &[],
    },
    MatrixEntry {
        name: "AXA Mitarbeiter Shareplan",
        kind: Some(ConnectorKind::AxaShareplan),
        interface: Interface::WebPortal,
        official: Some(false),
        capabilities: &[Capability::Balances, Capability::Securities],
        status: Status::TestingWanted,
        issues: &[],
    },
    MatrixEntry {
        name: "Union Investment",
        kind: Some(ConnectorKind::UnionInvestment),
        interface: Interface::WebPortal,
        official: Some(false),
        capabilities: &[
            Capability::Balances,
            Capability::Securities,
            Capability::Documents,
        ],
        status: Status::TestingWanted,
        issues: &[],
    },
    MatrixEntry {
        name: "Berghaus & Cie (Weindepot)",
        kind: Some(ConnectorKind::BerghausCie),
        interface: Interface::WebPortal,
        official: Some(false),
        capabilities: &[Capability::Balances, Capability::Securities],
        status: Status::TestingWanted,
        issues: &[],
    },
    MatrixEntry {
        name: "FNZ Bank",
        kind: Some(ConnectorKind::Fnz),
        interface: Interface::WebPortal,
        official: Some(false),
        capabilities: &[
            Capability::Balances,
            Capability::Transactions,
            Capability::Securities,
            Capability::SecurityTransactions,
            Capability::Documents,
        ],
        status: Status::Verified,
        issues: &[],
    },
    MatrixEntry {
        name: "Amazon Bestellungen",
        kind: Some(ConnectorKind::AmazonOrders),
        interface: Interface::WebPortal,
        official: Some(false),
        capabilities: &[Capability::Transactions],
        status: Status::TestingWanted,
        issues: &[],
    },
    MatrixEntry {
        name: "Hanseatic Bank",
        kind: Some(ConnectorKind::HanseaticBank),
        interface: Interface::WebPortal,
        official: Some(false),
        capabilities: &[
            Capability::Transactions,
            Capability::Balances,
            Capability::Documents,
        ],
        status: Status::TestingWanted,
        issues: &[],
    },
    MatrixEntry {
        name: "AirPlus Corporate Card",
        kind: Some(ConnectorKind::AirPlus),
        interface: Interface::WebPortal,
        official: Some(false),
        capabilities: &[Capability::Transactions, Capability::Balances],
        status: Status::TestingWanted,
        issues: &[],
    },
];

pub fn make_connector(kind: ConnectorKind, demo: bool) -> Box<dyn BankConnector> {
    make_connector_for(kind, kind.as_str(), demo)
}

/// Like `make_connector`, but with the stored connector key so generic FinTS
/// connections get their BLZ (`fints:<blz>`).
pub fn make_connector_for(kind: ConnectorKind, key: &str, demo: bool) -> Box<dyn BankConnector> {
    if demo {
        return Box::new(demo::DemoConnector::new(kind));
    }
    match kind {
        ConnectorKind::FinTsGeneric => Box::new(fints::FinTsConnector::for_key(key)),
        ConnectorKind::VolksbankKasselGoettingen => {
            Box::new(fints::FinTsConnector::new("52090000"))
        }
        ConnectorKind::Direkt1822 => Box::new(fints::FinTsConnector::new("50050201")),
        ConnectorKind::Comdirect => Box::new(comdirect::ComdirectConnector::new()),
        ConnectorKind::PayPal => Box::new(paypal::PayPalConnector::new()),
        ConnectorKind::TradeRepublic => Box::new(trade_republic::TradeRepublicConnector::new()),
        ConnectorKind::AmericanExpress => Box::new(enable_banking::EnableBankingConnector::new(
            "enablebanking.aspsp.amex",
            "American Express",
            "enablebanking.country.amex",
        )),
        ConnectorKind::AmazonVisa => Box::new(enable_banking::EnableBankingConnector::new(
            "enablebanking.aspsp.amazonvisa",
            "Open Bank",
            "enablebanking.country.amazonvisa",
        )),
        ConnectorKind::AmazonOrders => Box::new(crate::importers::amazon_orders::WebOnlyConnector),
        ConnectorKind::Fnz => Box::new(fnz::portal::PortalConnector::default()),
        ConnectorKind::Advanzia => Box::new(advanzia::WebOnlyConnector),
        ConnectorKind::Edenred => Box::new(crate::importers::portal_import::PortalOnlyConnector),
        ConnectorKind::AirPlus => Box::new(airplus::WebOnlyConnector),
        ConnectorKind::Ibkr => Box::new(ibkr::IbkrConnector::new()),
        ConnectorKind::Quirion => Box::new(quirion::WebOnlyConnector),
        ConnectorKind::Sharinbox => Box::new(sharinbox::WebOnlyConnector),
        ConnectorKind::AxaShareplan => Box::new(axashareplan::WebOnlyConnector),
        ConnectorKind::UnionInvestment => Box::new(unioninvestment::WebOnlyConnector),
        ConnectorKind::BerghausCie => Box::new(berghauscie::WebOnlyConnector),
        ConnectorKind::SchwaebischHall => Box::new(schwaebisch_hall::WebOnlyConnector),
        ConnectorKind::EquatePlus => Box::new(equateplus::WebOnlyConnector),
        ConnectorKind::Bunq => Box::new(bunq::BunqConnector::new()),
        ConnectorKind::Easybank => Box::new(easybank::EasybankConnector::new()),
        ConnectorKind::AlteLeipziger => Box::new(fin4u::Fin4uConnector::new()),
        ConnectorKind::VolkswagenBank => Box::new(volkswagen_bank::VolkswagenBankConnector::new()),
        ConnectorKind::Uberspace => Box::new(uberspace::UberspaceConnector::new()),
        ConnectorKind::BitcoinCore => Box::new(bitcoin_core::BitcoinCoreConnector::new()),
        ConnectorKind::Ffb => Box::new(ffb::FfbConnector::new()),
        ConnectorKind::FordMoney => Box::new(fordmoney::FordMoneyConnector::new()),
        ConnectorKind::OpelDirektbank => Box::new(opel::OpelDirektbankConnector::new()),
        ConnectorKind::HanseaticBank => Box::new(hanseatic::HanseaticConnector::new()),
        ConnectorKind::Bhw => Box::new(bhw::BhwConnector::new()),
        ConnectorKind::Finpension => Box::new(finpension::FinpensionConnector::new()),
        ConnectorKind::Bitcoin => Box::new(bitcoin::BitcoinConnector::new()),
        ConnectorKind::Kontist => Box::new(kontist::KontistConnector::new()),
        ConnectorKind::Monefit => Box::new(monefit::MonefitConnector::new()),
        ConnectorKind::GoAndGrow => Box::new(goandgrow::GoAndGrowConnector::new()),
        ConnectorKind::ScalableCapital => Box::new(scalable::ScalableConnector::new()),
        ConnectorKind::EnableBanking => {
            let (country, aspsp) = ConnectorKind::enable_banking_target(key).unwrap_or(("DE", ""));
            Box::new(enable_banking::EnableBankingConnector::for_aspsp(
                aspsp, country,
            ))
        }
    }
}

#[cfg(test)]
mod matrix_tests {
    use super::*;

    /// The other direction: a connector in the matrix that `ConnectorKind::all()`
    /// misses has no descriptor in `bank_catalog()` (the add form stays empty)
    /// and its stored connections are not recognised (`from_str`).
    #[test]
    fn every_matrix_connector_is_listed_in_all_kinds() {
        for e in BANK_MATRIX {
            let Some(kind) = e.kind else { continue };
            assert!(
                ConnectorKind::all().contains(&kind),
                "{}: {kind:?} missing in ConnectorKind::all()",
                e.name
            );
            assert_eq!(
                ConnectorKind::from_str(kind.as_str()),
                Some(kind),
                "{}",
                e.name
            );
            assert!(
                bank_catalog().iter().any(|d| d.kind == kind),
                "{}: no catalog entry",
                e.name
            );
        }
    }

    #[test]
    fn every_connector_kind_has_a_matrix_row() {
        for kind in ConnectorKind::all() {
            assert!(
                BANK_MATRIX.iter().any(|e| e.kind == Some(kind)),
                "no BANK_MATRIX entry for {kind:?}"
            );
        }
    }

    #[test]
    fn kinds_are_listed_once() {
        for (i, a) in BANK_MATRIX.iter().enumerate() {
            assert!(!a.name.trim().is_empty());
            if a.kind.is_some() {
                let dups = BANK_MATRIX[i + 1..].iter().filter(|b| b.kind == a.kind);
                assert_eq!(dups.count(), 0, "duplicate kind for {}", a.name);
            }
            let same_name = BANK_MATRIX[i + 1..].iter().filter(|b| b.name == a.name);
            assert_eq!(same_name.count(), 0, "duplicate name {}", a.name);
        }
    }

    #[test]
    fn capabilities_are_non_empty_and_unique() {
        for e in BANK_MATRIX {
            assert!(!e.capabilities.is_empty(), "{} has no capabilities", e.name);
            for (i, c) in e.capabilities.iter().enumerate() {
                assert!(
                    !e.capabilities[i + 1..].contains(c),
                    "{} lists {c:?} twice",
                    e.name
                );
            }
        }
    }

    #[test]
    fn open_test_needs_and_limits_point_to_issues() {
        for name in ["ING", "Revolut", "PayPal"] {
            let e = BANK_MATRIX.iter().find(|e| e.name == name).expect(name);
            assert_ne!(e.status, Status::Verified, "{name}");
            assert!(!e.issues.is_empty(), "{name} needs a tracking issue");
        }
        for e in BANK_MATRIX.iter().filter(|e| e.status == Status::Verified) {
            assert!(e.issues.is_empty(), "{} is verified but has issues", e.name);
        }
    }

    #[test]
    fn direkt1822_fints_is_read_only() {
        // 1822direkt disabled HKCCS/HKIPZ over FinTS in 09/2025 (Discussion #452).
        let e = BANK_MATRIX
            .iter()
            .find(|e| e.kind == Some(ConnectorKind::Direkt1822))
            .unwrap();
        assert!(!e.capabilities.contains(&Capability::Transfers));
        assert!(e.capabilities.contains(&Capability::Transactions));
        assert!(
            descriptor(ConnectorKind::Direkt1822)
                .note
                .contains("09/2025")
        );
    }

    #[test]
    fn interface_matches_protocol_of_catalog_entry() {
        for e in BANK_MATRIX {
            let Some(kind) = e.kind else { continue };
            let proto = descriptor(kind).protocol;
            assert_eq!(
                proto.starts_with("FinTS"),
                e.interface == Interface::Fints,
                "{}: interface vs protocol {proto:?}",
                e.name
            );
        }
    }

    /// The README column "Depot-Aktivitäten" comes from this: exactly the connectors that fill
    /// `FetchResult::security_transactions` (search for it in `connectors/` when one is added).
    #[test]
    fn security_transactions_are_listed_for_the_connectors_that_deliver_them() {
        let mut kinds: Vec<String> = BANK_MATRIX
            .iter()
            .filter(|e| e.capabilities.contains(&Capability::SecurityTransactions))
            .map(|e| format!("{:?}", e.kind.expect("a connector")))
            .collect();
        kinds.sort();
        assert_eq!(
            kinds,
            [
                "Bitcoin",
                "Comdirect",
                "Ffb",
                "Fnz",
                "Ibkr",
                "ScalableCapital",
                "TradeRepublic"
            ]
        );
        for e in BANK_MATRIX
            .iter()
            .filter(|e| e.capabilities.contains(&Capability::SecurityTransactions))
        {
            assert!(
                e.capabilities.contains(&Capability::Securities),
                "{}: activities without positions",
                e.name
            );
        }
    }

    /// Consorsbank documents come from the unofficial Online-Archiv (portal JSON API, own app
    /// approval), not from FinTS: the official FinTS row must not claim them.
    #[test]
    fn consorsbank_documents_are_listed_as_unofficial_online_archive() {
        let fints = BANK_MATRIX
            .iter()
            .find(|e| e.name == "Consorsbank")
            .expect("Consorsbank FinTS row");
        assert_eq!(fints.interface, Interface::Fints);
        assert_eq!(fints.official, Some(true));
        assert!(!fints.capabilities.contains(&Capability::Documents));

        let archive = BANK_MATRIX
            .iter()
            .find(|e| e.name == "Consorsbank (Online-Archiv, inoffiziell)")
            .expect("Consorsbank Online-Archiv row");
        assert_eq!(archive.interface, Interface::WebPortal);
        assert_eq!(archive.official, Some(false));
        assert_eq!(archive.capabilities, &[Capability::Documents]);
    }

    /// Enable Banking syncs (`fetch_card`, accounts) deliver no documents. The document gates in
    /// `persist.rs` match the matrix by kind only, so such a row must never list `Documents`.
    #[test]
    fn enable_banking_rows_list_no_documents() {
        for e in BANK_MATRIX
            .iter()
            .filter(|e| e.interface == Interface::EnableBanking)
        {
            assert!(
                !e.capabilities.contains(&Capability::Documents),
                "{}: Enable Banking delivers no documents",
                e.name
            );
        }
    }
}
