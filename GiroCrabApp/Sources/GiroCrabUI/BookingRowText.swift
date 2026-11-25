import SwiftUI
#if os(macOS)
import AppKit
#endif

/// Selectable booking text. Single clicks still select the booking on macOS;
/// dragging or clicking twice selects text using AppKit.
struct BookingRowText: View {
    let value: String
    let prominent: Bool
    let wraps: Bool
    let onSingleClick: (() -> Void)?

    #if os(macOS)
    @Environment(\.reportRedaction) private var redaction
    @Environment(\.textScale) private var scale
    #endif

    init(_ value: String, prominent: Bool = false, wraps: Bool = false, onSingleClick: (() -> Void)?) {
        self.value = value
        self.prominent = prominent
        self.wraps = wraps
        self.onSingleClick = onSingleClick
    }

    private var style: Font.TextStyle { prominent ? .body : .callout }
    private var weight: Font.Weight? { prominent ? .semibold : nil }
    private var color: Color { prominent ? Theme.text : Theme.secondary }

    var body: some View {
        let text = Text(value)
            .appFont(style, weight: weight)
            .foregroundStyle(color)
            .lineLimit(wraps ? nil : 1)
        Group {
            #if os(macOS)
            if let onSingleClick, redaction.allowsBookingCopy {
                text.hidden().overlay {
                    SelectableBookingText(
                        value: value, wraps: wraps,
                        font: style.appKitFont(scale: scale, weight: weight),
                        color: NSColor(color), onSingleClick: onSingleClick
                    )
                }
                .clipped()
            } else {
                text
            }
            #else
            text
            #endif
        }
        .sensitive()
    }
}

#if os(macOS)
private struct SelectableBookingText: NSViewRepresentable {
    let value: String
    let wraps: Bool
    let font: NSFont
    let color: NSColor
    let onSingleClick: () -> Void

    func makeNSView(context: Context) -> BookingTextView {
        let view = BookingTextView()
        view.isEditable = false
        view.isSelectable = true
        view.isRichText = false
        view.drawsBackground = false
        view.textContainerInset = .zero
        view.textContainer?.lineFragmentPadding = 0
        view.textContainer?.widthTracksTextView = true
        view.isHorizontallyResizable = false
        view.isVerticallyResizable = false
        return view
    }

    static func dismantleNSView(_ view: BookingTextView, coordinator: ()) {
        if let window = view.window, window.firstResponder === view {
            window.makeFirstResponder(nil)
        }
        view.isSelectable = false
        view.string = ""
        view.onSingleClick = nil
    }

    func updateNSView(_ view: BookingTextView, context: Context) {
        view.onSingleClick = onSingleClick
        view.textContainer?.maximumNumberOfLines = wraps ? 0 : 1
        view.textContainer?.lineBreakMode = wraps ? .byWordWrapping : .byTruncatingTail
        if view.string != value { view.string = value }
        if view.font != font { view.font = font }
        if view.textColor != color { view.textColor = color }
    }
}

private final class BookingTextView: NSTextView {
    var onSingleClick: (() -> Void)?
    private var trackingSelection = false

    // Ordinary clicks focus the list; native text selection takes focus for Cmd+C.
    override var acceptsFirstResponder: Bool { trackingSelection }

    override func hitTest(_ point: NSPoint) -> NSView? {
        if let event = NSApp.currentEvent {
            let rowModifiers: NSEvent.ModifierFlags = [.command, .shift, .control]
            if event.type == .rightMouseDown
                || (event.type == .leftMouseDown && !event.modifierFlags.isDisjoint(with: rowModifiers)) {
                return nil // row selection and the context menu
            }
        }
        return super.hitTest(point)
    }

    override func mouseDown(with event: NSEvent) {
        guard let window else { return }
        ClickModifiers.handledBookingTextMouseDown()
        if event.clickCount > 1 {
            selectText(with: event, in: window)
            return
        }

        // Peek until we know whether this is a click or a drag, then let
        // NSTextView track the original drag with its queued events intact.
        let mask: NSEvent.EventTypeMask = [.leftMouseUp, .leftMouseDragged]
        let next = window.nextEvent(matching: mask, until: .distantFuture, inMode: .eventTracking, dequeue: false)
        if next?.type == .leftMouseDragged {
            selectText(with: event, in: window)
        } else if next?.type == .leftMouseUp {
            _ = window.nextEvent(matching: mask, until: .distantFuture, inMode: .eventTracking, dequeue: true)
            if window.firstResponder === self {
                window.makeFirstResponder(nil)
            }
            onSingleClick?()
        }
    }

    private func selectText(with event: NSEvent, in window: NSWindow) {
        trackingSelection = true
        window.makeFirstResponder(self)
        super.mouseDown(with: event)
        trackingSelection = false
    }
}
#endif

extension View {
    /// Excludes a visible label or control from the macOS row's note shortcut.
    func bookingNoteOccupied() -> some View {
        #if os(macOS)
        transformAnchorPreference(key: BookingRowClickAnchors.self, value: .bounds) { $0.append(($1, .occupied)) }
        #else
        self
        #endif
    }

    #if os(macOS)
    /// A click on the category picker must leave the row selection and density intact.
    func bookingCategoryTriggerBounds() -> some View {
        transformAnchorPreference(key: BookingRowClickAnchors.self, value: .bounds) { $0.append(($1, .category)) }
    }
    #endif
}

#if os(macOS)
enum BookingRowClickRegion: Equatable {
    case occupied
    case category
}

private struct BookingRowClickAnchors: PreferenceKey {
    static let defaultValue: [(Anchor<CGRect>, BookingRowClickRegion)] = []
    static func reduce(value: inout [(Anchor<CGRect>, BookingRowClickRegion)],
                       nextValue: () -> [(Anchor<CGRect>, BookingRowClickRegion)]) {
        value.append(contentsOf: nextValue())
    }
}

struct BookingRowClickRect: Equatable {
    let bounds: CGRect
    let region: BookingRowClickRegion
}

private struct BookingRowClickRects: PreferenceKey {
    static let defaultValue: [BookingRowClickRect] = []
    static func reduce(value: inout [BookingRowClickRect], nextValue: () -> [BookingRowClickRect]) {
        value.append(contentsOf: nextValue())
    }
}

struct BookingRowClicks: ViewModifier {
    let selectRow: (NSEvent.ModifierFlags) -> Void
    let openNote: () -> Void
    @State private var regions: [BookingRowClickRect] = []

    static func selectsRow(at point: CGPoint, regions: [BookingRowClickRect], textClick: Bool) -> Bool {
        !textClick && !regions.contains { $0.region == .category && $0.bounds.contains(point) }
    }

    static func opensNote(at point: CGPoint, regions: [BookingRowClickRect], textClick: Bool) -> Bool {
        !textClick && !regions.contains { $0.bounds.contains(point) }
    }

    func body(content: Content) -> some View {
        content
            .backgroundPreferenceValue(BookingRowClickAnchors.self) { anchors in
                GeometryReader { proxy in
                    Color.clear.preference(key: BookingRowClickRects.self, value: anchors.map {
                        BookingRowClickRect(bounds: proxy[$0.0], region: $0.1)
                    })
                }
                .allowsHitTesting(false)
            }
            .onPreferenceChange(BookingRowClickRects.self) { regions = $0 }
            .contentShape(Rectangle())
            .simultaneousGesture(SpatialTapGesture().onEnded { tap in
                if Self.selectsRow(at: tap.location, regions: regions, textClick: ClickModifiers.bookingTextHandledMouseDown) {
                    selectRow(ClickModifiers.atLastMouseDown)
                }
            })
            .simultaneousGesture(SpatialTapGesture(count: 2).onEnded { tap in
                if Self.opensNote(at: tap.location, regions: regions, textClick: ClickModifiers.bookingTextInClickSequence) {
                    openNote()
                }
            })
    }
}
#endif
