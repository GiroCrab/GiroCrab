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

    /// Links `refundID` to `originalID`; its other links stay (a refund can pay back several bookings).
    func linkRefund(refundID: Int64, originalID: Int64) {
        guard let vault else { return }
        let linked = (try? vault.refundLinks(transactionId: refundID).originals.contains { $0.id == originalID }) ?? false
        guard !linked else { return }
        do { try vault.linkRefund(refundId: refundID, originalId: originalID) } catch { errorMessage = describe(error); return }
        if let um = windowUndoManager {
            um.registerUndo(withTarget: self) { m in m.unlinkRefund(refundID: refundID, originalID: originalID) }
            um.setActionName(String(localized: "Erstattung zuordnen"))
        }
        refundLinksChanged([refundID, originalID])
    }

    /// Removes the link of `refundID` to `originalID`; both bookings and their other links stay.
    func unlinkRefund(refundID: Int64, originalID: Int64) {
        guard let vault,
              (try? vault.refundLinks(transactionId: refundID).originals.contains { $0.id == originalID }) == true else { return }
        do { try vault.unlinkRefund(refundId: refundID, originalId: originalID) } catch { errorMessage = describe(error); return }
        if let um = windowUndoManager {
            um.registerUndo(withTarget: self) { m in m.linkRefund(refundID: refundID, originalID: originalID) }
            um.setActionName(String(localized: "Verknüpfung aufheben"))
        }
        refundLinksChanged([refundID, originalID])
    }

    /// Removes every link of the refund `refundID` (context menu of a refund), as one undo step.
    func unlinkRefundOriginals(refundID: Int64) {
        guard let vault, let originals = try? vault.refundLinks(transactionId: refundID).originals else { return }
        windowUndoManager?.beginUndoGrouping()
        for original in originals { unlinkRefund(refundID: refundID, originalID: original.id) }
        windowUndoManager?.endUndoGrouping()
    }

    private func refundLinksChanged(_ ids: [Int64]) {
        guard let vault else { return }
        // a link shifts the shares of the bookings around it too: refresh those markers as well
        var affected = Set(ids)
        for id in ids {
            guard let links = try? vault.refundLinks(transactionId: id) else { continue }
            affected.formUnion((links.originals + links.refunds).map(\.id))
        }
        for id in affected {
            guard let links = try? vault.refundLinks(transactionId: id) else { continue }
            let kind: RefundLinkKind = !links.originals.isEmpty ? .refund : (links.refunds.isEmpty ? .none : .original)
            let covered = kind == .refund ? links.assignedCents : links.refundedCents
            patchLoadedRows([id]) { row in
                let part = covered.flatMap { $0 < abs(row.amountCents) ? $0 : nil }
                row.refundLink = kind
                row.refundPartCents = part
                row.refundIncomeCents = kind == .refund ? part.map { row.amountCents - $0 } ?? 0 : 0
            }
            if refundPickerBooking?.id == id { refundPickerBooking?.refundLink = kind }
        }
        refundLinksRevision += 1
        reloadDashboard()
        refreshAfterEdit()
    }
}
