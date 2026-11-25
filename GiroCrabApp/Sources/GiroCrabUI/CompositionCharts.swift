import SwiftUI
import Charts
import GiroCrabCore

// The three drawings of a composition (ring, ranked bars, treemap). All of them get the same
// finished slices, total and currency: they compute no sums of their own, so switching the display
// never changes a number. Colours come with the slices (stable by business key). Amounts, shares
// and the drawings themselves are sensitive; names are labels.

private func shareText(_ cents: Int64, of total: Int64) -> String {
    total > 0 ? DepotFormat.percent(Double(cents) / Double(total) * 100, digits: 1) : "–"
}

private func detailText(_ s: DepotSlice, total: Int64, currency: String) -> String {
    let positions = s.count > 1 ? " · " + String(localized: "\(s.count) Positionen") : ""
    return "\(s.label) · \(Money.format(s.cents, currency: currency)) · \(shareText(s.cents, of: total))\(positions)"
}

/// The legend's hover: with a ring's bindings it highlights the slice and shows the card at the pointer, without
/// them (the shared picture) the entry keeps its plain tooltip.
private struct LegendSliceHover: ViewModifier {
    let id: String
    let help: String
    let hovered: Binding<String?>?
    let spot: Binding<HoverCardSpot?>?

    func body(content: Content) -> some View {
        if let hovered, let spot { content.sliceHover(id: id, hovered: hovered, spot: spot) } else { content.help(help) }
    }
}

/// Name, amount and share per slice; a long name wraps to a second line and is complete in the tooltip.
struct CompositionLegend: View {
    @Environment(\.textScale) private var textScale
    let slices: [DepotSlice]
    let total: Int64
    let currency: String
    var selected: String?
    var select: ((DepotSlice) -> Void)?
    /// Hover highlight and card of the ring this legend belongs to (#889).
    var hovered: Binding<String?>?
    var spot: Binding<HoverCardSpot?>?

    static func markerOpacity(id: String, selected: String?, hovered: String?) -> Double {
        if let selected {
            if selected == id { return 1 }
            return hovered == id ? 0.75 : 0.35
        }
        return hovered == nil || hovered == id ? 1 : 0.5
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            ForEach(slices) { s in
                let isSelected = selected == s.id
                Button { select?(s) } label: {
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Circle().fill(s.color).frame(width: 9, height: 9)
                            .opacity(Self.markerOpacity(id: s.id, selected: selected, hovered: hovered?.wrappedValue))
                            .accessibilityHidden(true)
                        Text(s.label).sensitive(s.personal ? .personal : .labels).lineLimit(2)
                            .foregroundStyle(s.unclassified ? Theme.secondary : Theme.text)
                        Spacer(minLength: 8)
                        Text(Money.format(s.cents, currency: currency)).sensitive(.amounts).foregroundStyle(Theme.secondary)
                        Text(shareText(s.cents, of: total)).sensitive(.amounts).foregroundStyle(Theme.secondary)
                            .frame(minWidth: 46 * textScale, alignment: .trailing)
                    }
                    .appFont(.caption, weight: isSelected ? .semibold : nil, monospacedDigit: true)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .modifier(LegendSliceHover(id: s.id, help: detailText(s, total: total, currency: currency), hovered: hovered, spot: spot))
                .accessibilityElement(children: .combine)
                .accessibilityAddTraits(isSelected ? [.isButton, .isSelected] : .isButton)
                .accessibilityHint(select == nil ? "" : String(localized: "Positionen dieser Gruppe anzeigen oder Filter aufheben"))
            }
        }
    }
}

/// Ring: segments proportional to the market value, the total in the middle, the legend beside or below.
struct CompositionRing: View {
    @Environment(\.textScale) private var textScale
    /// Dynamic Type (iOS): the hole holds the total, so the ring grows with the text, within reason.
    @ScaledMetric(relativeTo: .body) private var typeScale = 1.0
    private var ringSize: CGFloat { 168 * textScale * min(typeScale, 1.4) }
    let slices: [DepotSlice]
    let total: Int64
    let currency: String
    var selected: String?
    var select: ((DepotSlice) -> Void)?
    /// Slice under the pointer (macOS): highlighted in the ring and the legend, named in a card at the pointer (#889).
    @State private var hovered: String?
    @State private var spot: HoverCardSpot?

    private var active: String? { hovered ?? selected }

    private var ring: some View {
        Chart(slices) { s in
            SectorMark(angle: .value("Wert", s.cents), innerRadius: .ratio(0.66), angularInset: 1.5)
                .cornerRadius(3)
                .foregroundStyle(s.color)
                .opacity(active == nil || active == s.id ? 1 : 0.3)
        }
        .chartLegend(.hidden)
        .pieHover(parts: slices.map { ($0.id, $0.cents) }, innerRatio: 0.66, hovered: $hovered, spot: $spot) { id in
            if let select, let s = slices.first(where: { $0.id == id }) { select(s) }
        }
        .frame(width: ringSize, height: ringSize)
        .sensitive()
        .overlay {
            VStack(spacing: 0) {
                Text(Money.format(total, currency: currency)).sensitive(.amounts)
                    .appFont(.callout, weight: .semibold, monospacedDigit: true)
                    .lineLimit(1).fixedSize()
                Text("Summe").appFont(.caption2).foregroundStyle(Theme.secondary)
            }
            .allowsHitTesting(false)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text("Kreisdiagramm der Zusammensetzung, \(slices.count) Segmente"))
    }

    var body: some View {
        ViewThatFits(in: .horizontal) {
            // a long legend (all positions unfolded) starts at the top; the ring is not centred beside it
            HStack(alignment: slices.count > 9 ? .top : .center, spacing: 28) {
                ring
                CompositionLegend(slices: slices, total: total, currency: currency, selected: selected, select: select,
                                  hovered: $hovered, spot: $spot)
                    .frame(maxWidth: 560 * textScale, alignment: .leading)
                Spacer(minLength: 0)
            }
            VStack(alignment: .leading, spacing: 14) {
                ring.frame(maxWidth: .infinity)
                CompositionLegend(slices: slices, total: total, currency: currency, selected: selected, select: select,
                                  hovered: $hovered, spot: $spot)
            }
        }
        .hoverCardHost(spot) { id in
            if let s = slices.first(where: { $0.id == id }) {
                PieDetailCard(label: s.label, cents: s.cents, total: total, currency: currency, personal: s.personal)
            }
        }
    }
}

/// Ranking: one bar per position or group on a common scale from zero, largest first. The rest
/// is shown on its own below, not as one huge bar that would flatten the single positions.
struct CompositionRankBars: View {
    @Environment(\.textScale) private var textScale
    /// All slices, largest first (not yet folded).
    let slices: [DepotSlice]
    let total: Int64
    let currency: String
    var selected: String?
    var select: ((DepotSlice) -> Void)?
    /// Entries listed before the rest is folded into one line.
    var keep = DepotComposition.maxRankedSlices

    var body: some View {
        // every slice is either listed or in the rest: with eleven positions all eleven are listed
        let (shown, rest) = DepotComposition.ranked(slices, keep: keep)
        let scale = max(shown.map(\.cents).max() ?? 1, 1)
        VStack(alignment: .leading, spacing: 8) {
            ForEach(shown) { s in
                let isSelected = selected == s.id
                Button { select?(s) } label: {
                    HStack(spacing: 10) {
                        Text(s.label).sensitive(s.personal ? .personal : .labels).lineLimit(2)
                            .foregroundStyle(s.unclassified ? Theme.secondary : Theme.text)
                            .frame(width: 150 * textScale, alignment: .leading)
                        GeometryReader { geo in
                            RoundedRectangle(cornerRadius: 3, style: .continuous).fill(s.color)
                                .frame(width: max(geo.size.width * CGFloat(s.cents) / CGFloat(scale), 2))
                        }
                        .frame(height: 12 * textScale)
                        .opacity(selected == nil || isSelected ? 1 : 0.35)
                        .sensitive()
                        Text(Money.format(s.cents, currency: currency)).sensitive(.amounts).foregroundStyle(Theme.secondary)
                            .frame(minWidth: 96 * textScale, alignment: .trailing)
                        Text(shareText(s.cents, of: total)).sensitive(.amounts).foregroundStyle(Theme.secondary)
                            .frame(minWidth: 46 * textScale, alignment: .trailing)
                    }
                    .appFont(.caption, weight: isSelected ? .semibold : nil, monospacedDigit: true)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help(detailText(s, total: total, currency: currency))
                .accessibilityElement(children: .combine)
                .accessibilityAddTraits(isSelected ? [.isButton, .isSelected] : .isButton)
            }
            if let rest {
                Divider()
                HStack(spacing: 10) {
                    Text(rest.label).foregroundStyle(Theme.secondary)
                        .frame(width: 150 * textScale, alignment: .leading)
                    Spacer(minLength: 0)
                    Text(Money.format(rest.cents, currency: currency)).sensitive(.amounts).foregroundStyle(Theme.secondary)
                        .frame(minWidth: 96 * textScale, alignment: .trailing)
                    Text(shareText(rest.cents, of: total)).sensitive(.amounts).foregroundStyle(Theme.secondary)
                        .frame(minWidth: 46 * textScale, alignment: .trailing)
                }
                .appFont(.caption, monospacedDigit: true)
                .help(String(localized: "Die kleineren Positionen stehen vollständig in der Positionsliste."))
                .accessibilityElement(children: .combine)
            }
        }
    }
}

/// Text colour for a tile: white on dark fills, near-black on light ones (WCAG relative luminance of the
/// resolved colour in the current appearance).
enum TileInk {
    static func luminance(of color: Color, in environment: EnvironmentValues) -> Double {
        let c = color.resolve(in: environment)
        func channel(_ v: Float) -> Double { let d = Double(v); return d <= 0.03928 ? d / 12.92 : pow((d + 0.055) / 1.055, 2.4) }
        return 0.2126 * channel(c.red) + 0.7152 * channel(c.green) + 0.0722 * channel(c.blue)
    }

    static func color(on fill: Color, in environment: EnvironmentValues) -> Color {
        luminance(of: fill, in: environment) > 0.4 ? Color(red: 0.11, green: 0.11, blue: 0.12) : .white
    }
}

/// Treemap: tile area proportional to the market value (squarified layout), tile colour = the colour of
/// the position or group in the ring and the bars. Name and "market value · share" sit in the tile's top
/// left corner as far as the tile has room: both lines, then the name, then only the share. Hovering
/// (macOS) or tapping a tile shows everything in a card in the bottom right corner.
struct CompositionTreemap: View {
    @Environment(\.textScale) private var textScale
    @Environment(\.self) private var environment
    /// Already folded to the tiles that are drawn.
    let slices: [DepotSlice]
    let total: Int64
    let currency: String
    var selected: String?
    var select: ((DepotSlice) -> Void)?
    /// A fixed height where the width is not measured (the shared picture), else derived from the width.
    var fixedHeight: CGFloat?
    @State private var hovered: String?
    @State private var width: CGFloat = 0
    /// Width of one digit of the small tile font (Dynamic Type on the iPhone, the app's text size on the Mac).
    @ScaledMetric(relativeTo: .caption2) private var digitWidth: CGFloat = 6.6
    private var charWidth: CGFloat { digitWidth * textScale }

    /// About 210 pt; narrower areas get less, so the tiles stay blocks.
    private var height: CGFloat { fixedHeight ?? min(210 * textScale, max(150 * textScale, width * 0.6)) }

    private var focus: DepotSlice? {
        let id = hovered ?? selected
        return slices.first { $0.id == id }
    }

    var body: some View {
        GeometryReader { geo in
            // more than a few tiles form rows and columns, not one row of strips
            let rects = Treemap.layout(slices.map { Double($0.cents) }, in: CGRect(origin: .zero, size: geo.size),
                                       preferredAspect: slices.count > 4 ? 2.4 : nil)
            ZStack(alignment: .topLeading) {
                ForEach(slices.indices, id: \.self) { i in
                    tile(slices[i], rects[i].insetBy(dx: 1.5, dy: 1.5))
                }
            }
            .overlay(alignment: .bottomTrailing) {
                if let focus { detailCard(focus).padding(8).allowsHitTesting(false) }
            }
        }
        .frame(height: height)
        .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width = $0 }
        .sensitive()
    }

    private func detailCard(_ s: DepotSlice) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(s.label).sensitive(s.personal ? .personal : .labels).appFont(.subheadline, weight: .semibold).lineLimit(2)
            Text(Money.format(s.cents, currency: currency)).sensitive(.amounts).appFont(.callout, monospacedDigit: true)
            Text("Anteil \(shareText(s.cents, of: total))").sensitive(.amounts).appFont(.caption, monospacedDigit: true).foregroundStyle(Theme.secondary)
            if s.count > 1 {
                Text("\(s.count) Positionen").appFont(.caption2).foregroundStyle(Theme.tertiary)
            }
        }
        .padding(10)
        .frame(maxWidth: 260 * textScale, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 8, style: .continuous).fill(Theme.card))
        .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(Theme.border, lineWidth: 1))
        .shadow(color: .black.opacity(0.12), radius: 6, y: 2)
    }

    private func tile(_ s: DepotSlice, _ r: CGRect) -> some View {
        let dimmed = selected != nil && selected != s.id
        let ink = dimmed ? Color.primary : TileInk.color(on: s.color, in: environment)
        // what fits is judged by the length of the text (scaled with the font), not by fixed widths
        let inner = r.width - 16
        let share = shareText(s.cents, of: total)
        let amountLine = "\(Money.format(s.cents, currency: currency)) · \(share)"
        let fullFits = Double(inner) >= Double(amountLine.count) * Double(charWidth)
        let shareFits = Double(inner) >= Double(share.count) * Double(charWidth)
        let nameFits = r.width > 56 * textScale && r.height > 26 * textScale
        let both = fullFits && nameFits && r.height > 44 * textScale
        let nameOnly = !both && nameFits
        let shareOnly = !both && !nameOnly && shareFits && r.height > 18 * textScale
        let summary = detailText(s, total: total, currency: currency)
        return RoundedRectangle(cornerRadius: 6, style: .continuous)
            .fill(s.color.opacity(dimmed ? 0.35 : 1))
            .frame(width: max(r.width, 0), height: max(r.height, 0))
            .overlay(alignment: .topLeading) {
                VStack(alignment: .leading, spacing: 1) {
                    if both {
                        Text(s.label).appFont(.caption, weight: .bold).lineLimit(1)
                        Text("\(Money.format(s.cents, currency: currency)) · \(shareText(s.cents, of: total))")
                            .appFont(.caption2, monospacedDigit: true).lineLimit(1)
                    } else if nameOnly {
                        Text(s.label).appFont(.caption, weight: .bold).lineLimit(1)
                        if shareFits && r.height > 44 * textScale {
                            Text(share).appFont(.caption2, monospacedDigit: true).lineLimit(1)
                        }
                    } else if shareOnly {
                        Text(shareText(s.cents, of: total)).appFont(.caption2, weight: .semibold, monospacedDigit: true).lineLimit(1)
                    }
                }
                .foregroundStyle(ink)
                .padding(8)
                .frame(width: max(r.width, 0), alignment: .leading)
            }
            .position(x: r.midX, y: r.midY)
            .onHover { inside in
                if inside { hovered = s.id } else if hovered == s.id { hovered = nil }
            }
            .onTapGesture { select?(s) }
            .help(summary)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(Text(summary))
            .accessibilityAddTraits(.isButton)
    }
}
