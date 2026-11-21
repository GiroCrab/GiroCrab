import Foundation
import GiroCrabCore

extension AppModel {
    // MARK: - Possible duplicates (bank booking vs. imported MoneyMoney row)

    func openDuplicates() {
        refreshDuplicates()
        showDuplicatesSheet = true
    }

    func refreshDuplicates() {
        guard let vault else { possibleDuplicates = []; return }
        runCore { possibleDuplicates = try vault.possibleDuplicates() }
    }

    /// One booking under the bank's id; `keepBank`: with the bank entry's texts,
    /// category and note, else the imported entry's (the other fills the gaps).
    func mergeDuplicate(_ pair: PossibleDuplicate, keepBank: Bool) {
        guard let vault else { return }
        runCore {
            try vault.mergePossibleDuplicate(
                bankBookingId: pair.bank.bookingId,
                importedBookingId: pair.imported.bookingId,
                keepBank: keepBank
            )
        }
        refreshDuplicates()
        reloadAll()
    }

    /// "Kein Doppel": the pair is not offered again.
    func dismissDuplicate(_ pair: PossibleDuplicate) {
        guard let vault else { return }
        runCore {
            try vault.dismissPossibleDuplicate(
                bankBookingId: pair.bank.bookingId,
                importedBookingId: pair.imported.bookingId
            )
        }
        refreshDuplicates()
    }
}
