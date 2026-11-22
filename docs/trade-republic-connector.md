# Trade Republic – Historie in Etappen

Beim Abruf aktualisiert GiroCrab zuerst die jüngeren Buchungen und liest bei
Bedarf ältere Buchungen, Wertpapieraktivitäten und Belege nach. Die App zeigt
eine offene Historie an und bietet im Depot „Weiter abrufen“ an. Neu eingelesene
ältere Buchungen erscheinen als geprüft. Die Dokumente folgen denselben
[Ablage- und Matchingregeln](dokument-zuordnung.md) wie beim laufenden Abruf.

Pro Etappe werden regulär bis zu 400 ältere Wertpapieraktivitäten ausgewertet.
Mehrere Aktivitäten vom selben Tag bleiben in derselben Etappe. Der Tresor
speichert die Fortsetzungsmarken für die Buchungszeitleiste und das Activity Log
sowie fehlgeschlagene Anfragen für den nächsten Abruf.

„Historie neu aufbauen …“ in den Bankzugangseinstellungen setzt die
Fortsetzungsmarken zurück. Der nächste Abruf beginnt wieder am Anfang.
GiroCrab erkennt bereits gespeicherte Buchungen und Belege anhand ihrer IDs.
Ein ausgeschalteter Dokumentenabruf pausiert die Arbeit an den Belegen.
