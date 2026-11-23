import Foundation
import GiroCrabCore

struct RefundEndpointPair: Sendable, Equatable {
    let refundID: Int64
    let refundPart: String
    let originalID: Int64
    let originalPart: String

    init(bookingID: Int64, ownPart: String, paysBack: Bool, candidate: RefundCandidate) {
        refundID = paysBack ? bookingID : candidate.transactionId
        refundPart = paysBack ? ownPart : candidate.part
        originalID = paysBack ? candidate.transactionId : bookingID
        originalPart = paysBack ? candidate.part : ownPart
    }

    init(_ link: RefundLink) {
        refundID = link.refundId
        refundPart = link.refundPart
        originalID = link.originalId
        originalPart = link.originalPart
    }
}

struct RefundUndo: Sendable {
    let pair: RefundEndpointPair
    let token: String
    let pin: VaultPin
}

enum RefundHistoryAction: Sendable {
    case unlink(RefundEndpointPair)
    case restore(RefundUndo)
}

@MainActor
final class RefundHistoryStep {
    var task: Task<RefundHistoryAction?, Never>?

    init(_ action: RefundHistoryAction? = nil) {
        if let action { task = Task { action } }
    }
}

struct RefundLoadKey: Hashable {
    let vaultRevision: Int
    let bookingID: Int64
    let ownPart: String
    let revision: Int
}

struct RefundCandidateKey: Hashable {
    let context: RefundLoadKey
    let query: String
}

struct RefundSnapshot {
    let booking: Booking
    let parts: [TransactionSplit]
    let selectedPart: String
    let links: [RefundLink]
    let suggestions: [RefundSuggestion]
    var counterparts: [Int64: Booking] = [:]

    var amount: Int64 { parts.isEmpty ? booking.amountCents : parts.first { $0.uuid == selectedPart }?.amountCents ?? 0 }
    var paysBack: Bool { amount > 0 }
    var ownOpen: Int64 {
        let used = links.reduce(Int64(0)) { $0 + $1.allocatedCents }
        return max(0, abs(amount) - used)
    }
    func canLink(_ candidate: RefundCandidate) -> Bool { amount != 0 && ownOpen > 0 && candidate.openCents > 0 }

    func isUnlinked(_ candidate: RefundCandidate) -> Bool {
        !links.contains {
            (paysBack ? $0.originalId : $0.refundId) == candidate.transactionId &&
                (paysBack ? $0.resolvedOriginalPart : $0.resolvedRefundPart) == candidate.part
        }
    }
}

struct RefundPickerState {
    var loaded: (key: RefundLoadKey, snapshot: RefundSnapshot)?
    var loadedCandidates: (key: RefundCandidateKey, rows: [RefundCandidate])?

    func snapshot(for key: RefundLoadKey) -> RefundSnapshot? {
        loaded?.key == key ? loaded?.snapshot : nil
    }

    func candidates(for key: RefundCandidateKey) -> [RefundCandidate]? {
        snapshot(for: key.context) != nil && loadedCandidates?.key == key ? loadedCandidates?.rows : nil
    }
}

struct RefundCardSnapshot {
    let links: [RefundLink]
    let counterparts: [Int64: Booking]
}

struct RefundAllocationSummary {
    let assigned: Int64
    let open: Int64
    var total: Int64 { assigned + open }

    init(links: [RefundLink], paysBack: Bool) {
        assigned = links.reduce(0) { $0 + $1.allocatedCents }
        var endpoints: [String: Int64] = [:]
        for link in links {
            let part = paysBack ? link.resolvedRefundPart : link.resolvedOriginalPart
            endpoints[part ?? ""] = paysBack ? link.refundOpenCents : link.originalOpenCents
        }
        open = endpoints.values.reduce(0, +)
    }
}
