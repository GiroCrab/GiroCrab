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
                suggestions: Array(suggestions.filter { unlinked($0.candidate) }.prefix(3)),
                counterparts: try Self.refundCounterparts(in: vault, links: links, bookingID: key.bookingID))
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
        guard key.vaultRevision == vaultRevision, key.revision == refundRevision else { return nil }
        let result = try await vaultOffMain { vault in
            let links = try vault.refundLinks(transactionId: key.bookingID).filter { !$0.inactive }
            return RefundCardSnapshot(links: links,
                counterparts: try Self.refundCounterparts(in: vault, links: links, bookingID: key.bookingID))
        }
        guard !Task.isCancelled, generation == listGeneration, key.vaultRevision == vaultRevision, key.revision == refundRevision else { return nil }
        return result
    }

    nonisolated private static func refundCounterparts(in vault: Vault, links: [RefundLink], bookingID: Int64) throws -> [Int64: Booking] {
        let ids = Set(links.map { $0.refundId == bookingID ? $0.originalId : $0.refundId })
        return try refundBookings(in: vault, ids: ids)
    }

    nonisolated private static func refundBookings(in vault: Vault, ids: Set<Int64>) throws -> [Int64: Booking] {
        let dates = Set(try ids.compactMap { try vault.bookingDate(bookingId: $0) })
        var bookings: [Int64: Booking] = [:]
        for date in dates.sorted() {
            let filter = BookingFilter(accountIds: [], categoryId: nil, includeSubcategories: true,
                uncategorizedOnly: false, search: nil, dateFrom: date, dateTo: date,
                limit: 0, offset: 0, scope: .all, merchantKey: nil, direction: .all, checked: nil, wholeDays: false)
            for booking in try vault.listTransactions(filter: filter).transactions where ids.contains(booking.id) {
                bookings[booking.id] = booking
            }
        }
        return bookings
    }

    func linkRefund(_ pair: RefundEndpointPair) async {
        guard !refundBusy, let pin = pinVault() else { return }
        refundHistoryPending += 1
        refundBusy = true
        defer { refundHistoryPending -= 1; refundBusy = refundHistoryPending > 0 }
        do {
            try await pin.run { try $0.linkRefund(refundId: pair.refundID, refundPart: pair.refundPart, originalId: pair.originalID, originalPart: pair.originalPart) }
            guard isCurrent(pin) else { return }
            registerRefundHistory(RefundHistoryStep(.unlink(pair)), pin: pin)
            windowUndoManager?.setActionName(String(localized: "Erstattung zuordnen"))
            refundUndo = nil
            await refundLinksChanged()
        } catch { if isCurrent(pin) { errorMessage = describe(error) } }
    }

    func unlinkRefund(_ pair: RefundEndpointPair) async {
        guard !refundBusy, let pin = pinVault() else { return }
        refundHistoryPending += 1
        refundBusy = true
        defer { refundHistoryPending -= 1; refundBusy = refundHistoryPending > 0 }
        do {
            let token = try await pin.run { try $0.unlinkRefund(refundId: pair.refundID, refundPart: pair.refundPart, originalId: pair.originalID, originalPart: pair.originalPart) }
            guard isCurrent(pin) else { return }
            let undo = RefundUndo(pair: pair, token: token, pin: pin)
            refundUndo = undo
            registerRefundHistory(RefundHistoryStep(.restore(undo)), pin: pin)
            windowUndoManager?.setActionName(String(localized: "Verknüpfung aufheben"))
            await refundLinksChanged()
        } catch { if isCurrent(pin) { errorMessage = describe(error) } }
    }

    func restoreRefund(_ undo: RefundUndo) async {
        guard !refundBusy, isCurrent(undo.pin) else { return }
        refundHistoryPending += 1
        refundBusy = true
        defer { refundHistoryPending -= 1; refundBusy = refundHistoryPending > 0 }
        do {
            let pair = undo.pair
            try await undo.pin.run { try $0.restoreRefund(refundId: pair.refundID, refundPart: pair.refundPart, originalId: pair.originalID, originalPart: pair.originalPart, token: undo.token) }
            guard isCurrent(undo.pin) else { return }
            registerRefundHistory(RefundHistoryStep(.unlink(pair)), pin: undo.pin)
            windowUndoManager?.setActionName(String(localized: "Erstattung zuordnen"))
            refundUndo = nil
            await refundLinksChanged()
        } catch { if isCurrent(undo.pin) { errorMessage = describe(error) } }
    }

    private func registerRefundHistory(_ step: RefundHistoryStep, pin: VaultPin, manager: UndoManager? = nil) {
        guard let manager = manager ?? windowUndoManager else { return }
        manager.registerUndo(withTarget: step) { [step, weak self, weak manager] _ in
            guard let self, self.isCurrent(pin) else { return }
            let inverse = RefundHistoryStep()
            self.registerRefundHistory(inverse, pin: pin, manager: manager)
            let previous = self.refundHistoryTask
            self.refundHistoryPending += 1
            self.refundBusy = true
            inverse.task = Task { @MainActor [weak self] in
                await previous?.value
                guard let self else { return nil }
                defer {
                    self.refundHistoryPending -= 1
                    self.refundBusy = self.refundHistoryPending > 0
                }
                guard let action = await step.task?.value, self.isCurrent(pin) else {
                    manager?.removeAllActions(withTarget: inverse)
                    return nil
                }
                do {
                    let next: RefundHistoryAction
                    switch action {
                    case .unlink(let pair):
                        let token = try await pin.run {
                            try $0.unlinkRefund(refundId: pair.refundID, refundPart: pair.refundPart,
                                originalId: pair.originalID, originalPart: pair.originalPart)
                        }
                        guard self.isCurrent(pin) else { return nil }
                        let undo = RefundUndo(pair: pair, token: token, pin: pin)
                        self.refundUndo = undo
                        next = .restore(undo)
                    case .restore(let undo):
                        try await pin.run {
                            try $0.restoreRefund(refundId: undo.pair.refundID, refundPart: undo.pair.refundPart,
                                originalId: undo.pair.originalID, originalPart: undo.pair.originalPart, token: undo.token)
                        }
                        guard self.isCurrent(pin) else { return nil }
                        self.refundUndo = nil
                        next = .unlink(undo.pair)
                    }
                    await self.refundLinksChanged()
                    return next
                } catch {
                    manager?.removeAllActions(withTarget: inverse)
                    if self.isCurrent(pin) { self.errorMessage = describe(error) }
                    return nil
                }
            }
            self.refundHistoryTask = Task { _ = await inverse.task?.value }
        }
    }

    private func refundLinksChanged() async {
        refundRevision &+= 1
        invalidateTotals()
        financialReportRevision &+= 1
        reloadDashboard()
        refreshSelectedBookingHistory()
        if let id = stickyBooking?.id, let pin = pinVault() {
            let generation = listGeneration
            let fresh = try? await pin.run { try Self.refundBookings(in: $0, ids: [id])[id] }
            if isCurrent(pin), generation == listGeneration, stickyBooking?.id == id, let fresh {
                stickyBooking = fresh
                if let index = bookings.firstIndex(where: { $0.id == id }) { bookings[index] = fresh }
            }
        }
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
