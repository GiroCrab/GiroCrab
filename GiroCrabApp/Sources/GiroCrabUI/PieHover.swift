import SwiftUI
import Charts
import GiroCrabCore

/// What a pie's legend shows next to the name: the share of the total or the amount (#889).
/// The hover card of a slice always shows name, amount and share, whatever is chosen here.
enum PieValueMode: String, CaseIterable, Identifiable {
    case share
    case amount

    /// Legend of the pies in the header (category, account and wealth overview).
    static let headerPieKey = "chart.headerPie.valueMode"

    var id: String { rawValue }

    var title: String {
        switch self {
        case .share: String(localized: "Anteil in Prozent")
        case .amount: String(localized: "Betrag")
        }
    }

    /// Share of `total` as text, e.g. `12,5 %`; a dash without a positive total.
    static func shareText(_ cents: Int64, of total: Int64) -> String {
        total > 0 ? DepotFormat.percent(Double(cents) / Double(total) * 100, digits: 1) : "–"
    }
}

/// Which slice of a pie lies under the pointer (#889). Pure geometry: `value` turns a clockwise angle from
/// twelve o'clock into the cumulative value Swift Charts uses for its sectors (`ChartProxy.value(atAngle:)`).
enum PieHit {
    static func id(at location: CGPoint, in frame: CGRect, innerRatio: CGFloat,
                   parts: [(id: String, cents: Int64)], value: (Angle) -> Double?) -> String? {
        let dx = location.x - frame.midX, dy = location.y - frame.midY
        let distance = hypot(dx, dy), radius = min(frame.width, frame.height) / 2
        guard radius > 0, distance >= radius * innerRatio, distance <= radius else { return nil }
        let radians = atan2(dx, -dy)
        let angle = Angle.radians(radians < 0 ? radians + 2 * .pi : radians)
        guard let v = value(angle), v.isFinite, v >= 0 else { return nil }
        var end = 0.0
        for part in parts where part.cents > 0 {
            end += Double(part.cents)
            if v < end { return part.id }
        }
        return nil
    }
}

/// Where a hover card goes: the pointer's place in the coordinates of the view that carries `hoverCardHost`.
struct HoverCardSpot: Equatable {
    var id: String
    var point: CGPoint
}

enum HoverCardSpace {
    static let name = "hoverCardSpace"
}

extension View {
    /// Draws `card` for `spot` at the mouse pointer, over everything inside this view and over the views that follow
    /// it (a card inside the chart's own overlay would be clipped at the chart's edge and covered by its neighbours).
    /// It lies right of and below the pointer and flips to the left or above where it would run over this view's
    /// edge. Takes no clicks. Everything that reports a `HoverCardSpot` sits inside this view.
    func hoverCardHost<Card: View>(_ spot: HoverCardSpot?, @ViewBuilder card: @escaping (String) -> Card) -> some View {
        coordinateSpace(name: HoverCardSpace.name)
            .overlay(alignment: .topLeading) {
                GeometryReader { geo in
                    if let spot {
                        HoverCardPlacement(point: spot.point, container: geo.size) { card(spot.id) }
                    }
                }
                .allowsHitTesting(false)
            }
            .zIndex(1)
    }

    /// A legend entry or other slice label: highlights its slice and shows the card at the pointer, like the chart.
    func sliceHover(id: String, hovered: Binding<String?>, spot: Binding<HoverCardSpot?>) -> some View {
        onContinuousHover(coordinateSpace: .named(HoverCardSpace.name)) { phase in
            switch phase {
            case .active(let point):
                if hovered.wrappedValue != id { hovered.wrappedValue = id }
                spot.wrappedValue = HoverCardSpot(id: id, point: point)
            case .ended:
                if hovered.wrappedValue == id { hovered.wrappedValue = nil }
                if spot.wrappedValue?.id == id { spot.wrappedValue = nil }
            }
        }
    }

    /// Reports the slice under the pointer in `hovered` (nil outside the ring; the legend may set it too, to
    /// highlight), the pointer's place in `spot` (only while it is over a slice) and calls `tap` for a click on a
    /// slice. Goes on the pie `Chart`; the sectors need the same inner radius as the `SectorMark`s. The card itself
    /// is drawn by a `hoverCardHost` further up.
    func pieHover(parts: [(id: String, cents: Int64)], innerRatio: CGFloat, hovered: Binding<String?>,
                  spot: Binding<HoverCardSpot?>, tap: ((String) -> Void)? = nil) -> some View {
        chartOverlay { proxy in
            GeometryReader { geo in
                let hit: (CGPoint) -> String? = { location in
                    guard let plot = proxy.plotFrame else { return nil }
                    return PieHit.id(at: location, in: geo[plot], innerRatio: innerRatio, parts: parts) { angle in
                        proxy.value(atAngle: angle)
                    }
                }
                Color.clear.contentShape(Rectangle())
                    // the pointer in the host's coordinates, for the card; the slice is looked up in the chart's own
                    .onContinuousHover(coordinateSpace: .named(HoverCardSpace.name)) { phase in
                        switch phase {
                        case .active(let point):
                            let origin = geo.frame(in: .named(HoverCardSpace.name)).origin
                            let id = hit(CGPoint(x: point.x - origin.x, y: point.y - origin.y))
                            if hovered.wrappedValue != id { hovered.wrappedValue = id }
                            spot.wrappedValue = id.map { HoverCardSpot(id: $0, point: point) }
                        case .ended:
                            if hovered.wrappedValue != nil { hovered.wrappedValue = nil }
                            if spot.wrappedValue != nil { spot.wrappedValue = nil }
                        }
                    }
                    .onTapGesture { point in
                        if let tap, let id = hit(point) { tap(id) }
                    }
            }
        }
    }
}

/// Puts a card at `point` (right of and below), measured so it can flip left or up at the edge of `container`.
struct HoverCardPlacement<Content: View>: View {
    let point: CGPoint
    let container: CGSize
    @ViewBuilder var content: Content
    @State private var size = CGSize(width: 180, height: 70)
    private let gap: CGFloat = 14

    var body: some View {
        let x = point.x + gap + size.width > container.width ? point.x - gap - size.width : point.x + gap
        let y = point.y + gap + size.height > container.height ? point.y - gap - size.height : point.y + gap
        content
            .fixedSize()
            .onGeometryChange(for: CGSize.self) { $0.size } action: { size = $0 }
            .offset(x: max(x, 0), y: max(y, 0))
    }
}

/// Name, amount and share of one slice, as a card next to a small pie.
struct PieDetailCard: View {
    let label: String
    let cents: Int64
    let total: Int64
    var currency = "EUR"
    /// A name that is personal (a private person), not only a label.
    var personal = false
    @Environment(\.textScale) private var textScale

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label).sensitive(personal ? .personal : .labels).appFont(.caption, weight: .semibold).lineLimit(2)
            Text(Money.format(cents, currency: currency)).sensitive(.amounts).appFont(.caption, monospacedDigit: true)
            Text(PieValueMode.shareText(cents, of: total)).sensitive(.amounts)
                .appFont(.caption2, monospacedDigit: true).foregroundStyle(Theme.secondary)
        }
        .padding(.horizontal, 8).padding(.vertical, 6)
        .frame(maxWidth: 200 * textScale, alignment: .leading)
        .background(Theme.card, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(Theme.border, lineWidth: 1))
        .shadow(color: .black.opacity(0.12), radius: 5, y: 2)
    }
}

/// Share or amount in the legend of the header pies; sits right of the chart type picker (#889).
struct PieValueModeMenu: View {
    @AppStorage(PieValueMode.headerPieKey) private var mode: PieValueMode = .share

    var body: some View {
        Menu {
            Picker("Kreis zeigt", selection: $mode) {
                ForEach(PieValueMode.allCases) { Text($0.title).tag($0) }
            }
            .pickerStyle(.inline)
        } label: {
            Text(mode.title).appFont(.caption2).foregroundStyle(Theme.secondary)
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.visible)
        .fixedSize()
        .help("Kreis zeigt")
        .accessibilityLabel("Kreis zeigt")
        .accessibilityValue(mode.title)
    }
}
