import Foundation
import GiroCrabCore

extension AppModel {
    // MARK: - Possible duplicates (bank booking vs. imported MoneyMoney row)

    func openDuplicates() {
        possibleDuplicates = []
        duplicatesPin = nil
        showDuplicatesSheet = true
        refreshDuplicates()
    }

    /// Loads the pairs off the main actor (the query reads every booking with the
    /// same amount). The list is bound to the vault it came from: a result from a
    /// vault that was locked or swapped for the demo meanwhile is dropped.
    func refreshDuplicates() {
        guard let pin = pinVault() else { closeDuplicates(); return }
        duplicatesTask?.cancel()
        duplicatesTask = Task { @MainActor in
            do {
                let pairs = try await pin.run { try $0.possibleDuplicates() }
                guard !Task.isCancelled, isCurrent(pin) else { return }
                possibleDuplicates = pairs
                duplicatesPin = pin
            } catch {
                guard !Task.isCancelled, isCurrent(pin) else { return }
                errorMessage = describe(error)
            }
        }
    }

    /// Clears the pairs and closes the sheet (vault locked or switched).
    func closeDuplicates() {
        duplicatesTask?.cancel()
        duplicatesTask = nil
        possibleDuplicates = []
        duplicatesPin = nil
        showDuplicatesSheet = false
    }

    /// The vault the shown pairs came from, if it is still the open one: booking
    /// ids from another vault must never reach a merge.
    private func duplicatesVault() -> VaultPin? {
        guard let pin = duplicatesPin, isCurrent(pin) else {
            closeDuplicates()
            return nil
        }
        return pin
    }

    /// One booking under the bank's id; `keepBank`: with the bank entry's texts,
    /// category and note, else the imported entry's (the other fills the gaps).
    func mergeDuplicate(_ pair: PossibleDuplicate, keepBank: Bool) {
        guard let pin = duplicatesVault() else { return }
        let bank = pair.bank.bookingId
        let imported = pair.imported.bookingId
        Task { @MainActor in
            do {
                try await pin.run {
                    try $0.mergePossibleDuplicate(
                        bankBookingId: bank, importedBookingId: imported, keepBank: keepBank)
                }
            } catch {
                if isCurrent(pin) { errorMessage = describe(error) }
            }
            guard isCurrent(pin) else { return }
            refreshDuplicates()
            reloadAll()
        }
    }

    /// "Kein Doppel": the pair is not offered again.
    func dismissDuplicate(_ pair: PossibleDuplicate) {
        guard let pin = duplicatesVault() else { return }
        let bank = pair.bank.bookingId
        let imported = pair.imported.bookingId
        Task { @MainActor in
            do {
                try await pin.run {
                    try $0.dismissPossibleDuplicate(bankBookingId: bank, importedBookingId: imported)
                }
            } catch {
                if isCurrent(pin) { errorMessage = describe(error) }
            }
            guard isCurrent(pin) else { return }
            refreshDuplicates()
        }
    }
}
