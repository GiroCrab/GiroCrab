# Trade Republic – Historie in Etappen

<!-- sync:connector-note:start -->
> **Hinweis / Note:** Dieser Connector ist ein unabhängiges Community-Projekt und steht in keiner Verbindung zum Anbieter Trade Republic. Schnittstelle: inoffiziell. Bei inoffiziellen Schnittstellen kann die Nutzung gegen die Nutzungsbedingungen des Anbieters verstoßen.
> This connector is an independent community project and is not affiliated with Trade Republic. Interface: unofficial. Using unofficial interfaces may violate the provider's terms of service.
<!-- sync:connector-note:end -->

Beim Abruf aktualisiert GiroCrab zuerst die jüngeren Buchungen und liest bei
Bedarf ältere Buchungen, Wertpapieraktivitäten und Belege nach. Die App zeigt
eine offene Historie an und bietet im Depot „Weiter abrufen“ an. Neu eingelesene
ältere Buchungen erscheinen als geprüft. Die Dokumente folgen denselben
[Ablage- und Matchingregeln](dokument-zuordnung.md) wie beim laufenden Abruf.

Pro Etappe werden regulär bis zu 400 ältere Wertpapieraktivitäten ausgewertet.
Mehrere Aktivitäten vom selben Tag bleiben in derselben Etappe. Hat ein Tag allein
400 oder mehr Aktivitäten, nimmt die Etappe den nächsten Tag mit. Der Tresor
speichert die Fortsetzungsmarken für die Buchungszeitleiste und das Activity Log
sowie fehlgeschlagene Anfragen für den nächsten Abruf. Offene Belege, die ein
Abruf nicht erreicht hat, kommen beim nächsten Mal vor den fehlgeschlagenen dran.

Das Activity Log wird einmal bis zu seinem Ende gelesen, danach nur noch bis zum
Abrufzeitraum. Ein Abruf nur eines Kontos (Kontomenü in der Seitenleiste)
aktualisiert die jüngeren Buchungen, setzt die Historie und die offenen Belege
aber nicht fort. Das übernimmt der nächste Abruf des ganzen Bankzugangs.

„Historie neu aufbauen …“ in den Bankzugangseinstellungen setzt die
Fortsetzungsmarken zurück. Der nächste Abruf beginnt wieder am Anfang und liest
auch das Activity Log noch einmal ganz. GiroCrab erkennt bereits gespeicherte
Buchungen und Belege anhand ihrer IDs. Ein ausgeschalteter Dokumentenabruf
pausiert die Arbeit an den Belegen.
