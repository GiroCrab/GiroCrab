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
    private var color: Color { prominent ? Theme.text : Theme.secondary }

    var body: some View {
        let text = Text(value)
            .appFont(style, weight: prominent ? .semibold : nil)
            .foregroundStyle(color)
            .lineLimit(wraps ? nil : 1)
        Group {
            #if os(macOS)
            if let onSingleClick, redaction.effective.isDisjoint(with: .personal) {
                text.hidden().overlay {
                    SelectableBookingText(
                        value: value, wraps: wraps,
                        font: .systemFont(ofSize: style.macSize(scale: scale), weight: prominent ? .semibold : .regular),
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
        anchorPreference(key: BookingNoteOccupiedBounds.self, value: .bounds) { [$0] }
        #else
        self
        #endif
    }
}

#if os(macOS)
private struct BookingNoteOccupiedBounds: PreferenceKey {
    static let defaultValue: [Anchor<CGRect>] = []
    static func reduce(value: inout [Anchor<CGRect>], nextValue: () -> [Anchor<CGRect>]) {
        value.append(contentsOf: nextValue())
    }
}

private struct BookingNoteOccupiedRects: PreferenceKey {
    static let defaultValue: [CGRect] = []
    static func reduce(value: inout [CGRect], nextValue: () -> [CGRect]) {
        value.append(contentsOf: nextValue())
    }
}

struct BookingNoteDoubleClick: ViewModifier {
    let openNote: () -> Void
    @State private var occupied: [CGRect] = []

    static func opensNote(at point: CGPoint, occupied: [CGRect], textClick: Bool) -> Bool {
        !textClick && !occupied.contains { $0.contains(point) }
    }

    func body(content: Content) -> some View {
        content
            .backgroundPreferenceValue(BookingNoteOccupiedBounds.self) { anchors in
                GeometryReader { proxy in
                    Color.clear.preference(key: BookingNoteOccupiedRects.self, value: anchors.map { proxy[$0] })
                }
                .allowsHitTesting(false)
            }
            .onPreferenceChange(BookingNoteOccupiedRects.self) { occupied = $0 }
            .contentShape(Rectangle())
            .simultaneousGesture(SpatialTapGesture(count: 2).onEnded { tap in
                if Self.opensNote(at: tap.location, occupied: occupied, textClick: ClickModifiers.bookingTextInClickSequence) {
                    openNote()
                }
            })
    }
}
#endif
