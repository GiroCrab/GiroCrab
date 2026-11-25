---
name: girocrab-connector
description: End-to-end checklist for adding or modifying bank, broker, or card connectors in girocrab-core and client UIs. Use whenever touching girocrab-core/src/connectors/ or bank presets.
---

# GiroCrab Bank Connector Checklist

1. **Read ADR 0001 First:**
   - Follow `docs/adr/0001-connector-architecture.md` and mirror neighboring connectors in `girocrab-core/src/connectors/`.

2. **Four Mandatory Layers (No Core-Only Connector PRs):**
   - Core connector implementation (`girocrab-core/src/connectors/`) with strict HTTPS host validation and per-account/postbox error isolation.
   - Catalog & preset registration (`catalog.rs`, `bank_directory.rs`).
   - FFI & serde wiring (`#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]` on exported UniFFI records/enums).
   - Client setup UI & tests (`GiroCrabApp`, plus platform mappings in Windows/Linux if applicable).

3. **Offline Deterministic Tests & Lints:**
   - Write unit tests in `girocrab-core/src/connectors/tests/` using synthetic fixtures (never real IBANs, names, or live network calls; use `#[tokio::test(start_paused = true)]` instead of sleeps).
   - Run `bash scripts/lint-connectors.sh` and `bash scripts/lint-brand-names.sh` before committing.
