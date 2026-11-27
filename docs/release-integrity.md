# Release-Integrität (Überwachung)

Ein geplanter Workflow (`.github/workflows/release-integrity.yml`, alle 6 Stunden
und manuell startbar) prüft, ob das, was unter
`https://github.com/<repo>/releases/latest/download/` (Appcast, Disk-Images) und
unter `https://raw.githubusercontent.com/<repo>/main/policy/` (Sicherheitssperre)
veröffentlicht ist, noch echt ist. Er braucht **keine Geheimnisse**, nur die öffentlichen Schlüssel aus dem
Repo, und läuft deshalb auch in Forks (gegen deren eigene Releases).

## Was geprüft wird

| Datei | Prüfung |
|---|---|
| `appcast.xml` | erreichbar; eingebettete Sparkle-Signatur gegen `scripts/sparkle-public-ed-key.txt` (während eines Schlüsselwechsels auch `scripts/sparkle-previous-public-ed-key.txt`); höchstens ein Eintrag pro Kanal |
| jeder Appcast-Eintrag | zeigt auf ein Release-Asset **dieses** Repos (stabiles Release, Beta-Pre-Release oder `nightly`), das existiert; Länge und `sparkle:edSignature` passen zum heruntergeladenen Disk-Image |
| `SHA256SUMS.txt` (Windows) | jede Zeile (`.exe`/`.msi`/`.zip`) verweist auf ein Asset des neuesten stabilen Releases, dessen SHA-256 passt; unbekannte Dateitypen, kaputte Zeilen und fehlende Assets sind Funde. Fehlt die Datei (noch kein Windows-Release), nur ein Hinweis |
| `banking-policy.json` + `.sig` | erreichbar; Signatur gegen `BankingPolicy.publicKeys`; `sequence` nie kleiner als beim letzten Lauf (gemerkt im Actions-Cache) |

Das Windows-Paket (Installer/ZIP) ist noch **nicht** Authenticode-signiert und nicht im Appcast. Die
Windows-App liest die GitHub-Releases-API (`GiroCrabWindows/src/updates.rs`), deshalb hängen
`release.yml` (Tags `v*`, Entwurf) und `nightly.yml` (versionierter Pre-Release `vX.Y.Z-nightly.YYYYMMDD`)
Installer, ZIP und `SHA256SUMS.txt` an das Release; vorher setzt `scripts/set-windows-version.sh` die Tag-Version in
`GiroCrabWindows/Cargo.toml`/`Cargo.lock` (#1778). Die Prüfsumme liegt im selben Release wie die Datei und erkennt daher
Beschädigung oder Austausch **einer** Datei, nicht aber einen Angreifer, der beide ersetzt.

Für den Appcast eignen sich Prüfsummen nicht: Jeder Beta- und Nightly-Lauf schreibt den Feed neu.
Geprüft werden deshalb Signaturen und der Bezug auf echte Releases.

Ein Teil wird übersprungen, solange sein Schlüssel nicht eingetragen ist
(Sparkle: `scripts/sparkle-public-ed-key.txt` fehlt; Banking-Policy: Platzhalter in
`BankingPolicy.publicKeys` oder Datei nicht vorhanden).

Wurde eine Datei **noch nie** veröffentlicht (kein Release mit In-App-Updates,
kein Windows-Release mit `SHA256SUMS.txt`, Policy noch nicht aktiviert), ist ein
404 nur ein Hinweis. Sobald ein Lauf sie einmal gesehen hat (gemerkt im
Actions-Cache), ist ein späteres 404 ein Fund.

Windows-Nightlies heißen `vX.Y.Z-nightly.JJJJMMTT` mit X.Y.Z = letzter Stable-
oder Beta-Tag plus eine Patch-Version. So bietet der Updater sie Nutzern dieser
Version an, und spätere Betas oder Stables von X.Y.Z sind wieder neuer.

## Schutzstatus von `main` und Release-Tags (Stand 2026-11-25, #1804)

Abgefragt über die GitHub-API (`gh api repos/GiroCrab/GiroCrab/branches/main/protection`
und `…/rulesets`). Der Stand kann sich ändern; maßgeblich sind die GitHub-Einstellungen.

| Ziel | Schutz | Status |
|---|---|---|
| `main` | Branch-Protection: kein Force-Push, kein Löschen | aktiv |
| `main` | Erforderliche Status-Checks: `rust`, `swift`, `audit` (aus `ci.yml`: Format, Lints, Clippy, Tests, `cargo audit`) | aktiv, nicht `strict` (Branch muss nicht aktuell sein) |
| `main` | Gilt auch für Administratoren (`enforce_admins`) | **nein**, Admins können den Schutz umgehen (z. B. `gh pr merge --admin`) |
| `main` | Signierte Commits erforderlich | nein |
| Release-Tags `v*` | Tag-Ruleset (kein Löschen, kein Überschreiben, kein Force-Push) | **noch nicht eingerichtet**: es gibt keine Rulesets im Repository |

Offen (Einstellung durch Repo-Admins, keine Code-Änderung): ein Tag-Ruleset für
`refs/tags/v*` mit den Regeln *Deletion*, *Update* und *Non-fast-forward*;
für `main` zusätzlich `enforce_admins`, sobald der Maintainer-Workflow es zulässt.

### Signatur und Notarisierung der Releases

Release-Pakete für macOS werden **lokal** auf dem Rechner des Maintainers mit
Developer ID signiert, notarisiert und geheftet (`scripts/notarize.sh --upload vX.Y.Z`);
`release.yml` baut sie bewusst nicht in der Cloud. Updates prüft die App zusätzlich über die
Sparkle-Signatur des Appcasts (`docs/auto-updates.md`). Die Windows- und Android-Pakete sind
Alpha-Builds: Das Windows-Paket ist nicht signiert, das Android-APK ist ein Debug-Build
(`assembleDebug`) und nur mit dem Debug-Schlüssel signiert; notarisiert ist keines von beiden.

Ob das Veröffentlichte noch dazu passt, prüft `scripts/verify-release-integrity.sh`
(Abschnitt „Lokal ausführen“); der geplante Workflow ruft es alle 6 Stunden auf.
Das ersetzt keinen Tag-Schutz: Es erkennt Abweichungen erst nach dem Veröffentlichen.

## Meldungen

Abweichungen landen in **einem** offenen Issue mit dem Label `release-integrity`
(wird bei Bedarf angelegt). Weitere Funde kommen als Kommentar dazu, statt bei
jedem Lauf ein neues Issue zu öffnen. Der Workflow-Lauf schlägt dann fehl. Das
Issue schließt man von Hand, sobald die Ursache geklärt ist.

Typische Ursachen und was zu tun ist:

* **Signatur stimmt nicht / Asset fehlt oder passt nicht:** Release-Assets
  wurden ersetzt. Audit-Log der Organisation prüfen (wer hat Assets geändert?).
  Bei Manipulation: Schlüsselwechsel und Sicherheitsmeldung, siehe
  `docs/auto-updates.md` („Schlüsselwechsel“) und `docs/banking-policy.md`.
* **`banking-policy.json` nicht erreichbar:** `policy/` wurde auf `main`
  gelöscht oder verschoben. Mit `git log -- policy/` den letzten Stand finden
  und wiederherstellen.
* **`sequence` gesunken:** Jemand hat eine ältere, korrekt signierte Datei
  eingespielt. Apps nehmen sie nicht an, aber der Commit selbst ist verdächtig
  (`git log -- policy/`).
* **Direkt nach einem Sparkle-Schlüsselwechsel:** Bis
  `scripts/sparkle-public-ed-key.txt` und `scripts/sparkle-previous-public-ed-key.txt`
  committet sind, meldet die Prüfung den alt signierten Feed. Erst committen,
  dann veröffentlichen.

## Lokal ausführen

```bash
scripts/verify-release-integrity.sh                         # gegen GiroCrab/GiroCrab
scripts/verify-release-integrity.sh --repo jacboy/GiroCrab  # gegen einen Fork
scripts/verify-release-integrity.sh --base http://127.0.0.1:8766 --state /tmp/s.json   # Testdaten
```

`--base` ersetzt `…/releases/latest/download`, die Policy-Adresse und das
erlaubte Präfix der Disk-Images durch einen lokalen Server oder Ordner. Ausgabe: `ok: …`-Zeilen,
`FINDING: …` bei Abweichungen; Exit-Code 1 bei Funden.

## Grenzen

* Die Prüfung erkennt Manipulationen **nach** dem Veröffentlichen, sie verhindert
  sie nicht. Davor schützen die Signaturen in der App selbst.
* Ein Angreifer mit Schreibzugriff auf das Repo könnte auch die öffentlichen
  Schlüssel im Repo ändern. Deshalb: geschützte Branches und Tags (Rulesets), 2FA,
  und Änderungen an `scripts/sparkle-*.txt` und `BankingPolicy.swift` im Review
  besonders prüfen.
* Jede Änderung an Release-Assets erscheint zusätzlich im Audit-Log der
  Organisation.
