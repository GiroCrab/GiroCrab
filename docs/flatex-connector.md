# flatex-Anbindung (flatexDEGIRO Bank)

<!-- sync:connector-note:start -->
> **Hinweis / Note:** Dieser Connector ist ein unabhängiges Community-Projekt und steht in keiner Verbindung zum Anbieter flatexDEGIRO Bank. Schnittstelle: offiziell. Bei inoffiziellen Schnittstellen kann die Nutzung gegen die Nutzungsbedingungen des Anbieters verstoßen.
> This connector is an independent community project and is not affiliated with flatexDEGIRO Bank. Interface: official. Using unofficial interfaces may violate the provider's terms of service.
<!-- sync:connector-note:end -->

Living document. Update the **Status** table whenever work on this connector
advances.

Research date: 2026-11-15, first live test with a real account: 2026-11-25.

## Status

| Phase | Scope | State |
| --- | --- | --- |
| 0 | Research: which interfaces exist | ✅ done 2026-11-15 |
| 1 | Live test with a real flatex account (login, TAN method, bookings, balances, depot, transfers) | ✅ 2026-11-25 for Girokonto and Depot – login without TAN, two TANs for `HKSAL`/`HKKAZ`, bookings (180 days), 11 depot positions via `HKWPD`; transfers untested, no PDF service in the BPD |
| 2 | Bank entry (preset, search words, capability matrix) | ✅ done 2026-11-15 – no dedicated `ConnectorKind`, the generic FinTS connector is used |
| 3 | README/website matrix: from "Test gesucht" to "Verifiziert" | ✅ done 2026-11-25 (transactions, balances and depot confirmed with a real account) |

## Summary

flatex (flatexDEGIRO Bank SE) is reachable over **FinTS 3.0 PIN/TAN** at
`https://banking.flatexdegiro.com/hbci`. GiroCrab's existing FinTS connector
(`girocrab-core/src/connectors/fints/`) is used unchanged – the work is the bank
entry: the institute is pinned in `connectors/fints_directory.rs` and offered as
a preset in `connectors/fints_presets.rs`.

| Interface | Usable by GiroCrab? | Why |
| --- | --- | --- |
| **FinTS 3.0 PIN/TAN**, `https://banking.flatexdegiro.com/hbci` | **Yes** | Customer credentials, no TPP registration. |
| Old address `https://konto.biw-bank.de/hbci/` | Same server, not used | The DK institute list still lists it for BLZ 10130800; GiroCrab pins the current host and migrates a connection that stored the old address (`RETIRED_URLS`). |
| PSD2 / XS2A | No | Requires a licensed TPP; no registration path for a private app. |
| Portal / Postfach | Not implemented | flatex keeps its statements in the web portal only. GiroCrab does have portal document sources for other banks (`consorsbank_archive.rs`, `comdirect_postbox.rs`), but that needs the portal's login, its approval flow and its endpoints – the FinTS interface itself offers no statement service, so a flatex post box module is a separate project. |

## Bank codes

flatex, DEGIRO and ViTrade are brands of the same institute. GiroCrab resolves all
three codes to the same FinTS server:

| BLZ | Note |
| --- | --- |
| 10130800 | Main code (also the preset in the add-account sheet) |
| 30330800 | Same institute – code of the IBAN decides |
| 76030800 | Same institute – code of the IBAN decides |

A name search for "flatex", "degiro", "vitrade" or the old name "biw" finds the
entry; typing a German IBAN resolves the bank code from characters 5–12.

## What has been verified

* The endpoint answers a FinTS kickoff: `POST https://banking.flatexdegiro.com/hbci`
  with `hbcu/1.6` returns a base64-wrapped dialog response
  (`HNHBK`, return codes `9110::Unbekannter Nachrichtenaufbau` and
  `9800::Dialoginitialisierung abgebrochen`) – the server speaks FinTS/HBCI.
  Re-checked 2026-11-25 with GiroCrab's own `User-Agent`; the legacy `konto.biw-bank.de`
  address answers the same way.
* Server software: `de.xcom.biw.tristan.biw-2.47.14` (the platform behind both
  addresses).

## Live test 2026-11-25 (real account: Girokonto + Depot)

Traced with `GIROCRAB_DEPOT_DEBUG=1`; every number below comes from that trace.

* Login with PIN only, no TAN (`HKIDN`/`HKVVB` → `3076`). The bank offers pushTAN
  (`PPTAN:flateXSecure`) and photoTAN/iTAN card (`CR#3:iTAN-Card`).
* `HKSPA` (Kontenliste) is refused on every sync (`9050 / 9380 Benutzer hat keine
  Auftragsberechtigung`); the accounts come from the UPD, so the sync is complete
  and the refusal is not reported (#1083, #1547).
* `HIPINS` marks `HKSAL` and `HKKAZ` as TAN-required – those are the two TANs asked
  for during the first sync. The depot (`HKWPD`) needs none.
* Bookings arrive over `HKKAZ`/MT940; `HIKAZS` v6 states a storage period of
  **180 days**, which is why the list reaches only to early June.
* Depot: `HKWPD` v5 returns an MT535 (3,270 bytes, 11 positions, values derivable),
  stored as one depot account with all positions resolved.
* **No PDFs:** the BPD contains no statement service at all – there is no `HIKAES`,
  `HIKEKA` or `HIKEKP` segment – so GiroCrab cannot request PDF Kontoauszüge over
  FinTS. They are only in the portal (see "Portal / Postfach" above).

## Still open

1. Transfers: `HKCCS` is advertised in the BPD and flagged TAN-required in `HIPINS`,
   but no order was sent through GiroCrab yet.
2. Depot activities: the bank lists `HKWDU` (`HIWDUS` v2/v4), which the generic FinTS
   connector does not read – buys, sells and distributions are not imported as their
   own bookings.
3. Standing orders: `HKCDB` is not in the account's UPD list, so none are read;
   `HKCDE`/`HKCDU` (create/change) are untested.

## Feedback

Please report results (TAN method, statement depth, depot, transfer support) in
the issue tracker. Transactions, balances and depot are ✅; the capability row
gains a ✓ as soon as transfers or documents are confirmed with a real account.
