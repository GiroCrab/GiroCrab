# Trade Republic – Historie in Etappen

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
