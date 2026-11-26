<!-- Titel und Beschreibung auf Deutsch, Präfix feat:/fix:/perf:/ci:/docs:. Checkliste: CONTRIBUTING.md.
     Title and description in German (English is fine if you don't speak German), prefix feat:/fix:/perf:/ci:/docs:.
     Checklist: CONTRIBUTING.md. -->

## Was sich ändert

<!-- Problem, Lösung, Issue (#…). / Problem, solution, issue (#…). -->

## Nachweis

<!-- Was lief, mit Ergebnis (Tests, Messwerte, Screenshot mit --redact); z. B. die Tabelle aus scripts/verify-fast.sh.
     Was nicht lief, als „nicht gelaufen“ nennen.
     What ran, with its result (tests, measurements, a --redact screenshot); e.g. the table of scripts/verify-fast.sh.
     Name what did not run as "not run". -->

## Zahlungspfad / Payment path

<!-- Nur nötig, wenn TransferSheetView.swift, payment_input.rs oder connectors/fints/sepa.rs, transfer.rs, debit.rs bzw. standing.rs geändert werden,
     oder wenn geänderte Zeilen in .rs/.swift/.kt/.qml/.slint-Dateien Zahlungsfelder wie creditor_iban, amount_cents oder pain.001 nennen (scripts/lint-transfer-integrity.sh).
     Den Text des Punkts nicht ändern: Die CI sucht genau ihn, angehakt.
     Only needed when one of those files changes, or when changed lines in .rs/.swift/.kt/.qml/.slint files name payment fields such as creditor_iban, amount_cents or pain.001.
     Keep the item's text: CI looks for exactly this item, ticked. -->
- [ ] Zahlungspfad geprüft / payment path reviewed: IBAN-/Betragsprüfung, Bestätigung und TAN-Ablauf unverändert oder getestet, keine Umgehung, keine echten Kontodaten

<!-- Nur bei KI-Unterstützung behalten und ausfüllen (docs/ai-transparency.md), sonst löschen.
     Only with AI help: keep and fill in (docs/ai-transparency.md), otherwise delete. -->
### 🤖 AI Process Reflection
- **Human-Steering:**
- **AI-Agency:**
- **Review-Focus:**
- [ ] **Security-Check:** <!-- Pflicht bei Änderungen an Vault/Secrets/SEPA/Konnektoren/Local-API/Workflows/scripts: abhaken ([x]) und begründen. Required for security-critical paths: tick and explain. -->
- **Perf-Check:**
