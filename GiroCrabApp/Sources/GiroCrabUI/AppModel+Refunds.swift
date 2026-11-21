import Foundation
import GiroCrabCore

/// Refund links (#714): a refund points at the booking it pays back. The link only documents
/// what belongs together; amounts, categories and balances stay as they are. A linked refund
/// counts as a reduced expense instead of income, so the sums and the analysis are read again. Layout and rules: `docs/ERSTATTUNGEN_UI.md`.
extension AppModel {
    /// Opens `RefundPickerSheet`: for an incoming booking the original is chosen, for an
    /// outgoing one its refunds.
    func openRefundPicker(for booking: Booking) {
        guard booking.amountCents != 0 else { return }
        refundPickerBooking = booking
    }

    /// Links `refundID` to `originalID`; a refund has one original, so an earlier link is replaced.
    func linkRefund(refundID: Int64, originalID: Int64) {
        guard let vault else { return }
        let previous = try? vault.refundLinks(transactionId: refundID).original?.id
        guard previous != originalID else { return }
        do { try vault.linkRefund(refundId: refundID, originalId: originalID) } catch { errorMessage = describe(error); return }
        if let um = windowUndoManager {
            um.registerUndo(withTarget: self) { m in
                if let previous { m.linkRefund(refundID: refundID, originalID: previous) } else { m.unlinkRefund(refundID: refundID) }
            }
            um.setActionName(String(localized: "Erstattung zuordnen"))
        }
        refundLinksChanged([refundID, originalID] + (previous.map { [$0] } ?? []))
    }

    /// Removes the link of `refundID` to its original; both bookings stay.
    func unlinkRefund(refundID: Int64) {
        guard let vault, let original = try? vault.refundLinks(transactionId: refundID).original?.id else { return }
        do { try vault.unlinkRefund(refundId: refundID) } catch { errorMessage = describe(error); return }
        if let um = windowUndoManager {
            um.registerUndo(withTarget: self) { m in m.linkRefund(refundID: refundID, originalID: original) }
            um.setActionName(String(localized: "Verknüpfung aufheben"))
        }
        refundLinksChanged([refundID, original])
    }

    /// The loaded rows show the new marker at once; sums, analysis and open link views read again.
    private func refundLinksChanged(_ ids: [Int64]) {
        guard let vault else { return }
        for id in Set(ids) {
            guard let links = try? vault.refundLinks(transactionId: id) else { continue }
            let kind: RefundLinkKind = links.original != nil ? .refund : (links.refunds.isEmpty ? .none : .original)
            patchLoadedRows([id]) { $0.refundLink = kind }
            if refundPickerBooking?.id == id { refundPickerBooking?.refundLink = kind }
        }
        refundLinksRevision += 1
        reloadDashboard()
        refreshAfterEdit()
    }
}
