import Foundation
import CoreGraphics
import CoreTransferable
import UniformTypeIdentifiers
import GiroCrabCore

/// EPC QR code ("GiroCode", EPC069-12 v002) for a SEPA credit transfer, for
/// accounts GiroCrab cannot send from (e.g. PSD2 / Enable Banking banks like
/// C24): the user scans it with the bank's app ("Fotoüberweisung").
/// Payload and modules come from the core (`payment_input.rs`).
enum GiroCode {
    /// The QR payload, or nil when IBAN, name or amount are unusable.
    static func payload(name: String, iban: String, bic: String = "", amountCents: Int64, purpose: String) -> String? {
        girocodePayload(name: name, iban: iban, bic: bic, amountCents: amountCents, purpose: purpose)
    }

    /// Pixels per module: drawn at about its own size, the code stays sharp even
    /// where `.interpolation(.none)` is ignored (screenshots, bug report capture,
    /// printing), and the exported PNG is large enough to scan.
    static let pixelsPerModule = 10

    /// QR image (error correction "M", as the EPC guideline requires), quiet zone included.
    static func image(for payload: String) -> CGImage? {
        guard let m = girocodeMatrix(payload: payload) else { return nil }
        let n = Int(m.width), scale = pixelsPerModule, side = n * scale
        var pixels = Data(capacity: side * side)
        for y in 0..<n {
            let row = m.modules[(y * n)..<((y + 1) * n)].flatMap { repeatElement(UInt8($0 ? 0 : 255), count: scale) }
            for _ in 0..<scale { pixels.append(contentsOf: row) }
        }
        guard let provider = CGDataProvider(data: pixels as CFData) else { return nil }
        return CGImage(width: side, height: side, bitsPerComponent: 8, bitsPerPixel: 8, bytesPerRow: side,
                       space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.none.rawValue),
                       provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)
    }

    /// The code as PNG for copy, save and share (the picture carries IBAN and amount).
    static func png(for payload: String) -> Data? {
        image(for: payload).flatMap(BugReport.png)
    }
}

/// The GiroCode picture handed to `ShareLink`, composed only when asked.
struct GiroCodeImage: Transferable {
    let payload: String

    static var transferRepresentation: some TransferRepresentation {
        DataRepresentation(exportedContentType: .png) { item in
            guard let png = GiroCode.png(for: item.payload) else { throw CocoaError(.fileWriteUnknown) }
            return png
        }
        .suggestedFileName("GiroCode.png")
    }
}
