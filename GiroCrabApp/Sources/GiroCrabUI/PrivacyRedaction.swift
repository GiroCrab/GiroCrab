import SwiftUI

/// What the bug report screenshot hides (GiroCrab/GiroCrab#301). The main
/// window reads it from `AppModel.reportRedaction`; views mark their content
/// with `.sensitive()` / `.sensitive(.amounts)` / `.sensitive(.labels)`, so
/// the redaction is decided where the data is drawn instead of guessed
/// afterwards (OCR).
public struct ReportRedaction: OptionSet, Sendable, Hashable {
    public let rawValue: Int
    public init(rawValue: Int) { self.rawValue = rawValue }

    /// IBANs, account numbers, account names (banks often name an account
    /// after its holder), owners, counterparties, purposes, references, notes
    /// and documents. As a requested redaction it includes `.amounts`
    /// (`effective`): whoever hides names hides the money too.
    public static let personal = ReportRedaction(rawValue: 1 << 0)
    /// Category, group, budget and security names (off by default: they help
    /// to find a bug and say nothing about a person).
    public static let labels = ReportRedaction(rawValue: 1 << 1)
    /// Amounts, balances, sums, shares, chart axis values and readouts. Text
    /// that mixes both kinds is marked `.sensitive([.personal, .amounts])`.
    public static let amounts = ReportRedaction(rawValue: 1 << 2)

    /// The kinds that are actually hidden: `.personal` implies `.amounts`.
    public var effective: ReportRedaction {
        contains(.personal) ? union(.amounts) : self
    }

    /// `text`, or a placeholder while `kind` is hidden: for a name inside a
    /// longer text ("Konten auswählen – <Zugang>"), so the rest stays readable.
    func masking(_ text: String, _ kind: ReportRedaction = .personal) -> String {
        effective.isDisjoint(with: kind) || text.isEmpty ? text : "•••"
    }
}

private struct ReportRedactionKey: EnvironmentKey {
    static let defaultValue: ReportRedaction = []
}

extension EnvironmentValues {
    /// Set on the main window while a bug report screenshot is taken.
    var reportRedaction: ReportRedaction {
        get { self[ReportRedactionKey.self] }
        set { self[ReportRedactionKey.self] = newValue }
    }
}

extension View {
    /// Marks personal or financial content: replaced by a soft smudge while
    /// the bug report screenshot is taken, and `privacySensitive` for the
    /// system (widgets, locked screen). `kind` `.amounts` is for money,
    /// `.labels` for category and security names, which are only hidden on
    /// request.
    func sensitive(_ kind: ReportRedaction = .personal) -> some View {
        modifier(SensitiveContent(kind: kind))
    }
}

private struct SensitiveContent: ViewModifier {
    @Environment(\.reportRedaction) private var redaction
    let kind: ReportRedaction

    /// Fixed, so a smudge says nothing about the text below it.
    private static let smudge: [Gradient.Stop] = zip([0, 0.5, 0.3, 0.55, 0.25, 0.5, 0.35, 0.6, 0.3, 0.5, 0], 0...)
        .map { .init(color: Color.primary.opacity($0.0), location: Double($0.1) / 10) }
    private static let fade: [Gradient.Stop] = [
        .init(color: .clear, location: 0), .init(color: .black.opacity(0.6), location: 0.3), .init(color: .black, location: 0.5),
        .init(color: .black.opacity(0.6), location: 0.7), .init(color: .clear, location: 1),
    ]

    func body(content: Content) -> some View {
        let hidden = !redaction.effective.isDisjoint(with: kind)
        // The content is not drawn at all (opacity 0) and a blurred shape of
        // its size takes its place. It looks like blurred text, but unlike a
        // real blur nothing can be recovered from a screenshot, and it also
        // covers AppKit/UIKit-backed views (text editors, PDFView) and chart
        // marks, which `.blur` and `.redacted(reason:)` leave readable.
        // Opacity keeps the view's identity and state (scroll position,
        // editing); a hidden view takes no clicks and is not read aloud.
        content
            .privacySensitive(!kind.isDisjoint(with: [.personal, .amounts]))
            .opacity(hidden ? 0 : 1)
            .accessibilityHidden(hidden)
            .allowsHitTesting(!hidden)
            .overlay {
                if hidden {
                    // Uneven like the glyphs of blurred text and faded out
                    // towards every edge. Gradients instead of `.blur`: the
                    // in-process window bitmap of the bug report draws no
                    // layer filters, and a list full of blurs costs frames.
                    LinearGradient(stops: Self.smudge, startPoint: .leading, endPoint: .trailing)
                        .mask(LinearGradient(stops: Self.fade, startPoint: .top, endPoint: .bottom))
                        .accessibilityHidden(true)
                }
            }
    }
}
