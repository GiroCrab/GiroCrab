import Foundation
import SwiftUI
import GiroCrabCore

// MARK: - Daueraufträge anlegen, ändern, löschen (#496)

extension AppModel {
    /// FinTS accounts a standing order can be placed on (HKCDE; not bunq or the other connectors).
    var standingOrderAccounts: [Account] {
        transferAccounts.filter { $0.connector != .bunq && bankingGate(forAccount: $0.id) == nil }
    }

    var canEditStandingOrders: Bool { canTransfer && !standingOrderAccounts.isEmpty && activeTransfer == nil }

    /// The stored order behind a „Geplant“ row (ids look like `standing-<row>-<date>`), if the bank allows changes.
    func standingOrder(for payment: PlannedPayment) -> StandingOrderInfo? {
        guard payment.kind == .standingOrder else { return nil }
        let parts = payment.id.split(separator: "-")
        guard parts.count >= 2, parts[0] == "standing", let row = Int64(parts[1]) else { return nil }
        return standingOrders.first { $0.id == row }
    }

    func newStandingOrder() {
        guard canEditStandingOrders else { return }
        var draft = StandingDraft()
        var selected: Int64?
        if case .account(let id) = sidebarSelection { selected = id }
        if let f = plannedAccountFilter { selected = f }
        let offered = standingOrderAccounts
        draft.accountID = offered.contains(where: { $0.id == selected }) ? selected : offered.first?.id
        draft.execDay = Calendar.current.component(.day, from: draft.firstDate)
        openStandingSheet(draft)
    }

    /// Outgoing bookings with an IBAN can become a standing order.
    func canMakeStandingOrder(from b: Booking) -> Bool {
        canEditStandingOrders && b.amountCents < 0 && validateIban(iban: b.counterpartyIban)
    }

    /// Opens the form filled from a booking (the booking's account when it can place orders).
    func newStandingOrder(from b: Booking) {
        guard canMakeStandingOrder(from: b) else { return }
        let offered = standingOrderAccounts
        let account = offered.contains(where: { $0.id == b.accountId }) ? b.accountId : offered.first?.id
        openStandingSheet(StandingDraft(booking: b, series: recurringSeries(for: b), accountID: account))
    }

    func editStandingOrder(_ order: StandingOrderInfo) {
        guard canEditStandingOrders, order.canChange else { return }
        openStandingSheet(StandingDraft(order: order, action: .change))
    }

    func deleteStandingOrder(_ order: StandingOrderInfo) {
        guard canEditStandingOrders, order.canDelete else { return }
        openStandingSheet(StandingDraft(order: order, action: .delete))
    }

    /// A template, a paste or an invoice names a recipient (#1475): that is an order to someone else,
    /// so it leaves Umbuchung first, as in the transfer form. A running order keeps its form.
    func fillStandingDraft(_ p: ParsedPayment, onlyEmpty: Bool = false) {
        guard activeTransfer == nil else { return }
        standingDraft.ownTransfer = false
        p.apply(to: &standingDraft, onlyEmpty: onlyEmpty)
    }

    /// Screenshots only (`--demo-sheet`): opens the sheet for a draft without the bank checks.
    func openStandingDemo(_ draft: StandingDraft) { openStandingSheet(draft) }

    /// Identifies the open sheet; a late result for an earlier one is dropped (`fillStandingDraft(_:ifToken:)`).
    var standingImportToken: UUID { standingDraftToken }

    /// The form goes to the review: results of imports that are still running are dropped from now on.
    func invalidateStandingImports() { standingDraftToken = UUID() }

    func fillStandingDraft(_ p: ParsedPayment, ifToken token: UUID) -> Bool {
        guard token == standingDraftToken, showStandingSheet else { return false }
        fillStandingDraft(p)
        return true
    }

    private func openStandingSheet(_ draft: StandingDraft) {
        standingDraftToken = UUID()
        standingDraft = draft
        standingLimits = nil
        transferError = nil
        showStandingSheet = true
        refreshStandingLimits()
    }

    /// Loads what the account's bank allows for the order being edited and fits the draft to it.
    func refreshStandingLimits() {
        guard let account = standingDraft.accountID else { return }
        let action = standingDraft.action
        Task {
            let limits = try? await vaultOffMain { try $0.standingOrderLimits(accountId: account, action: action) }
            // The sheet may have moved on to another account meanwhile.
            guard standingDraft.accountID == account, standingDraft.action == action else { return }
            standingLimits = limits ?? nil
            standingDraft.conform(to: standingLimits)
        }
    }

    func cancelStandingSheet() {
        standingDraftToken = UUID()
        showStandingSheet = false
        standingDraft.pin = ""
        transferError = nil
    }

    func startStandingOrder() {
        withFreshBankingPolicy { [self] in
            if let gate = bankingGate(forAccount: standingDraft.accountID) {
                transferError = gate.message
                return
            }
            startStandingOrderChecked()
        }
    }

    private func startStandingOrderChecked() {
        guard let vault, let accID = standingDraft.accountID, canSendTransfer(from: accID),
              let account = allAccounts.first(where: { $0.id == accID }),
              let connection = connections.first(where: { $0.id == account.connectionId }),
              let request = standingDraft.request(demo: isDemo) else { return }
        let stored = connectionStatus(connection.id)?.hasPin ?? false
        if !isDemo && !stored && standingDraft.pin.isEmpty {
            transferError = String(localized: "Bitte die Banking-PIN eingeben.")
            return
        }
        do {
            let session = try vault.beginStandingOrder(request: request, secret: isDemo ? "demo" : standingDraft.pin, demo: isDemo)
            transferError = nil
            transferBusy = true
            showStandingSheet = false
            activeTransferIsStanding = true
            activeTransfer = session
            // the TAN sheet asks to compare what was sent: amount and the end of the recipient IBAN (#949, #1475)
            activeTransferComparison = TanComparison(iban: request.recipientIban, amountCents: request.amountCents)
            Task {
                do {
                    self.handleTransfer(try await session.start(), connection: connection)
                } catch {
                    self.finishTransfer(error: error)
                }
            }
        } catch {
            transferError = describe(error)
        }
    }

    /// The bank accepted the order: the list shows the optimistic copy until the next fetch.
    func finishStandingOrder() {
        pollTask?.cancel()
        activeChallenge = nil
        activeTransfer = nil
        transferBusy = false
        activeTransferIsStanding = false
        let action = standingDraft.action
        let accountID = standingDraft.accountID
        standingDraft = StandingDraft()
        plannedVersion += 1
        switch action {
        case .create: toast = String(localized: "Dauerauftrag angelegt")
        case .change: toast = String(localized: "Dauerauftrag geändert")
        case .delete: toast = String(localized: "Dauerauftrag gelöscht")
        }
        reloadAll()
        if action == .create { fetchAfterNewStandingOrder(accountID) }
    }

    /// A new order has no order number until the bank reports it, and without one it can be neither
    /// changed nor deleted (#1475): fetch its account once now instead of at the next manual fetch.
    /// Only where that needs no PIN prompt out of the blue (a stored PIN); the demo has no bank to ask.
    private func fetchAfterNewStandingOrder(_ accountID: Int64?) {
        guard !isDemo, let account = allAccounts.first(where: { $0.id == accountID }),
              let cid = account.connectionId, connectionStatus(cid)?.hasPin == true else { return }
        requestSyncAccount(account)
    }

    /// The bank may have executed it: no retry, say so.
    func standingOrderOutcomeUnknown(reason: String) {
        pollTask?.cancel()
        activeChallenge = nil
        activeTransfer = nil
        transferBusy = false
        activeTransferIsStanding = false
        standingDraft = StandingDraft()
        plannedVersion += 1
        standingUnknown = reason
        reloadAll()
    }

    /// The user cancelled while the bank waited for the TAN: nothing was sent, back to the sheet.
    func cancelStandingOrderRun() {
        activeTransferIsStanding = false
        standingDraft.pin = ""
        transferError = String(localized: "Dauerauftrag abgebrochen – es wurde nichts gesendet, solange die TAN nicht bestätigt wurde.")
        showStandingSheet = true
    }

    /// An error before the bank accepted anything: back to the sheet, nothing is lost.
    func failStandingOrder() {
        activeTransferIsStanding = false
        standingDraft.pin = ""
        showStandingSheet = true
    }
}
