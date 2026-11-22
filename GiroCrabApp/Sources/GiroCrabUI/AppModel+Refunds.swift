import Foundation
import GiroCrabCore

extension AppModel {
    func openRefundPicker(for booking: Booking) {
        guard booking.splitCount > 0 || booking.amountCents != 0 else { return }
        refundPickerBooking = booking
    }

    func refundSnapshot(for key: RefundLoadKey, booking: Booking) async throws -> RefundSnapshot? {
        let generation = listGeneration
        guard key.vaultRevision == vaultRevision, key.revision == refundRevision, key.bookingID == booking.id else { return nil }
        let filter = BookingFilter(accountIds: [booking.accountId], categoryId: nil, includeSubcategories: true,
            uncategorizedOnly: false, search: nil, dateFrom: booking.bookingDate, dateTo: booking.bookingDate,
            limit: 0, offset: 0, scope: .all, merchantKey: nil, direction: .all, checked: nil, wholeDays: false)
        let snapshot = try await vaultOffMain { vault -> RefundSnapshot? in
            let rows = try vault.listTransactions(filter: filter).transactions
            guard let booking = rows.first(where: { $0.id == key.bookingID }) else { return nil }
            let parts = try vault.listTransactionSplits(transactionId: key.bookingID)
            let ownPart = parts.isEmpty ? "" : (parts.first { $0.uuid == key.ownPart && $0.amountCents != 0 } ?? parts.first { $0.amountCents != 0 })?.uuid ?? ""
            let amount = parts.isEmpty ? booking.amountCents : parts.first { $0.uuid == ownPart }?.amountCents ?? 0
            let links = try vault.refundLinks(transactionId: key.bookingID).filter {
                !$0.inactive && (amount > 0
                    ? $0.refundId == key.bookingID && $0.resolvedRefundPart == ownPart
                    : $0.originalId == key.bookingID && $0.resolvedOriginalPart == ownPart)
            }
            let suggestions = amount == 0 ? [] : try vault.refundSuggestions(transactionId: key.bookingID, ownPart: ownPart)
            let linked = Set(links.map { "\(amount > 0 ? $0.originalId : $0.refundId)|\(amount > 0 ? $0.resolvedOriginalPart ?? "" : $0.resolvedRefundPart ?? "")" })
            let unlinked: (RefundCandidate) -> Bool = { !linked.contains("\($0.transactionId)|\($0.part)") }
            return RefundSnapshot(booking: booking, parts: parts, selectedPart: ownPart, links: links,
                suggestions: Array(suggestions.filter { unlinked($0.candidate) }.prefix(3)))
        }
        guard !Task.isCancelled, generation == listGeneration, key.vaultRevision == vaultRevision, key.revision == refundRevision else { return nil }
        if let snapshot = snapshot ?? nil { return snapshot }
        if refundPickerBooking?.id == key.bookingID { refundPickerBooking = nil }
        return nil
    }

    func refundCandidates(for key: RefundCandidateKey, snapshot: RefundSnapshot) async throws -> [RefundCandidate]? {
        let generation = listGeneration
        guard key.context.vaultRevision == vaultRevision, key.context.revision == refundRevision,
              key.context.bookingID == snapshot.booking.id, key.context.ownPart == snapshot.selectedPart else { return nil }
        guard snapshot.amount != 0 else { return [] }
        let rows = try await vaultOffMain {
            try $0.refundCandidates(transactionId: key.context.bookingID, ownPart: key.context.ownPart, query: key.query, limit: 200)
        }
        guard !Task.isCancelled, generation == listGeneration, key.context.vaultRevision == vaultRevision,
              key.context.revision == refundRevision else { return nil }
        return rows?.filter { snapshot.isUnlinked($0) }
    }

    func refundCard(for key: RefundLoadKey) async throws -> RefundCardSnapshot? {
        let generation = listGeneration
        let result = try await vaultOffMain { vault in
            RefundCardSnapshot(links: try vault.refundLinks(transactionId: key.bookingID).filter { !$0.inactive })
        }
        guard !Task.isCancelled, generation == listGeneration, key.vaultRevision == vaultRevision, key.revision == refundRevision else { return nil }
        return result
    }

    func linkRefund(_ pair: RefundEndpointPair) async {
        guard !refundBusy, let pin = pinVault() else { return }
        refundBusy = true
        defer { refundBusy = false }
        do {
            try await pin.run { try $0.linkRefund(refundId: pair.refundID, refundPart: pair.refundPart, originalId: pair.originalID, originalPart: pair.originalPart) }
            guard isCurrent(pin) else { return }
            windowUndoManager?.registerUndo(withTarget: self) { model in
                guard model.isCurrent(pin) else { return }
                Task { await model.unlinkRefund(pair) }
            }
            windowUndoManager?.setActionName(String(localized: "Erstattung zuordnen"))
            refundUndo = nil
            refundLinksChanged()
        } catch { if isCurrent(pin) { errorMessage = describe(error) } }
    }

    func unlinkRefund(_ pair: RefundEndpointPair) async {
        guard !refundBusy, let pin = pinVault() else { return }
        refundBusy = true
        defer { refundBusy = false }
        do {
            let token = try await pin.run { try $0.unlinkRefund(refundId: pair.refundID, refundPart: pair.refundPart, originalId: pair.originalID, originalPart: pair.originalPart) }
            guard isCurrent(pin) else { return }
            let undo = RefundUndo(pair: pair, token: token, pin: pin)
            refundUndo = undo
            windowUndoManager?.registerUndo(withTarget: self) { model in
                guard model.isCurrent(pin) else { return }
                Task { await model.restoreRefund(undo) }
            }
            windowUndoManager?.setActionName(String(localized: "Verknüpfung aufheben"))
            refundLinksChanged()
        } catch { if isCurrent(pin) { errorMessage = describe(error) } }
    }

    func restoreRefund(_ undo: RefundUndo) async {
        guard !refundBusy, isCurrent(undo.pin) else { return }
        refundBusy = true
        defer { refundBusy = false }
        do {
            let pair = undo.pair
            try await undo.pin.run { try $0.restoreRefund(refundId: pair.refundID, refundPart: pair.refundPart, originalId: pair.originalID, originalPart: pair.originalPart, token: undo.token) }
            guard isCurrent(undo.pin) else { return }
            windowUndoManager?.registerUndo(withTarget: self) { model in
                guard model.isCurrent(undo.pin) else { return }
                Task { await model.unlinkRefund(pair) }
            }
            windowUndoManager?.setActionName(String(localized: "Erstattung zuordnen"))
            refundUndo = nil
            refundLinksChanged()
        } catch { if isCurrent(undo.pin) { errorMessage = describe(error) } }
    }

    private func refundLinksChanged() {
        refundRevision &+= 1
        invalidateTotals()
        financialReportRevision &+= 1
        reloadDashboard()
        startSelectionLoad(keepingShown: true)
    }

    func prepareRefundDemo(_ name: String) async {
        guard demoActive, let pin = pinVault() else { return }
        do {
            let result = try await pin.run { vault -> (account: Int64, selected: Int64) in
                let account = try vault.createOfflineAccount(name: String(localized: "Erstattungen"), kind: .checking, currency: "EUR", openingCents: 0)
                let category = try vault.createCategory(name: String(localized: "Einkäufe"), parentId: nil, sfSymbol: "bag", colorHex: "#4C9A2A")
                let date = String(ISO8601DateFormatter().string(from: Date()).prefix(10))
                func booking(_ amount: Int64, _ purpose: String) throws -> Int64 {
                    try vault.saveManualBooking(accountId: account, bookingId: nil,
                        setup: ManualBookingSetup(bookingDate: date, amountCents: amount, counterparty: "Beispielhandel",
                            purpose: purpose, categoryId: category, note: ""))
                }
                let first = try booking(-1500, String(localized: "Rückgabe"))
                _ = try booking(-5000, String(localized: "Bestellung"))
                _ = try booking(-3500, String(localized: "Bestellung"))
                _ = try booking(-4000, String(localized: "Bestellung"))
                _ = try booking(-9000, String(localized: "Bestellung"))
                let negative = try booking(-7500, String(localized: "Einkauf"))
                let refund = try booking(6000, String(localized: "Teilerstattung"))
                try vault.setTransactionSplits(transactionId: refund, splits: [
                    TransactionSplitInput(uuid: nil, categoryId: category, amountCents: 8000, note: String(localized: "Rückzahlung")),
                    TransactionSplitInput(uuid: nil, categoryId: category, amountCents: -2000, note: String(localized: "Neuer Einkauf")),
                ])
                let parts = try vault.listTransactionSplits(transactionId: refund)
                guard let part = parts.first(where: { $0.amountCents > 0 }) else { throw CancellationError() }
                try vault.linkRefund(refundId: refund, refundPart: part.uuid, originalId: first, originalPart: "")
                let second = try booking(2500, String(localized: "Weitere Teilerstattung"))
                try vault.linkRefund(refundId: second, refundPart: "", originalId: negative, originalPart: "")
                return (account, name == "refund-negative" ? negative : refund)
            }
            guard demoActive, isCurrent(pin) else { return }
            reloadAccounts()
            sidebarSelection = .account(result.account)
            reloadAll()
            selectedBookingID = result.selected
            if name != "refund-linked", let row = booking(withID: result.selected) { openRefundPicker(for: row) }
        } catch { if isCurrent(pin), demoActive { errorMessage = describe(error) } }
    }
}
