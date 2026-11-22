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
    /// `part`: for a split booking the part the refund pays back (from 1), else 0. `refundPart`:
    /// for a split refund the part of it that pays back (from 1), else 0.
    func linkRefund(refundID: Int64, originalID: Int64, part: UInt32 = 0, refundPart: UInt32 = 0) {
        guard let vault else { return }
        let linked = (try? vault.refundLinks(transactionId: refundID).originals.contains { $0.id == originalID && $0.part == part && $0.refundPart == refundPart }) ?? false
        guard !linked else { return }
        do { try vault.linkRefundParts(refundId: refundID, refundPart: refundPart, originalId: originalID, originalPart: part) } catch { errorMessage = describe(error); return }
        if let um = windowUndoManager {
            um.registerUndo(withTarget: self) { m in m.unlinkRefund(refundID: refundID, originalID: originalID, part: part, refundPart: refundPart) }
            um.setActionName(String(localized: "Erstattung zuordnen"))
        }
        refundLinksChanged([refundID, originalID])
    }

    /// Removes the link of `refundID` to `originalID`; both bookings and their other links stay.
    func unlinkRefund(refundID: Int64, originalID: Int64, part: UInt32 = 0, refundPart: UInt32 = 0) {
        guard let vault else { return }
        do {
            // a link that is gone already is no error and leaves no undo step
            guard try vault.refundLinks(transactionId: refundID).originals.contains(where: { $0.id == originalID && $0.part == part && $0.refundPart == refundPart }) else { return }
        } catch { errorMessage = describe(error); return }
        do { try vault.unlinkRefundParts(refundId: refundID, refundPart: refundPart, originalId: originalID, originalPart: part) } catch { errorMessage = describe(error); return }
        if let um = windowUndoManager {
            um.registerUndo(withTarget: self) { m in m.linkRefund(refundID: refundID, originalID: originalID, part: part, refundPart: refundPart) }
            um.setActionName(String(localized: "Verknüpfung aufheben"))
        }
        refundLinksChanged([refundID, originalID])
    }

    /// Removes every link of the refund `refundID` (context menu of a refund), as one undo step.
    func unlinkRefundOriginals(refundID: Int64) {
        guard let vault else { return }
        let originals: [RefundBooking]
        do { originals = try vault.refundLinks(transactionId: refundID).originals } catch { errorMessage = describe(error); return }
        windowUndoManager?.beginUndoGrouping()
        for original in originals { unlinkRefund(refundID: refundID, originalID: original.id, part: original.part, refundPart: original.refundPart) }
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
            // like the core's rows: a booking that pays back is a refund, also an outgoing one
            // with a part that does
            let kind: RefundLinkKind = !links.originals.isEmpty ? .refund : (links.refunds.isEmpty ? .none : .original)
            let (covered, total) = kind == .refund ? (links.assignedCents, links.refundTotalCents) : (links.refundedCents, links.originalTotalCents)
            patchLoadedRows([id]) { row in
                row.refundLink = kind
                // like the core: no amount for a link that takes nothing
                row.refundPartCents = covered.flatMap { $0 > 0 && $0 < total ? $0 : nil }
                row.refundIncomeCents = links.refundIncomeCents
            }
            if refundPickerBooking?.id == id { refundPickerBooking?.refundLink = kind }
        }
        refundLinksRevision += 1
        reloadDashboard()
        refreshAfterEdit()
    }
}
