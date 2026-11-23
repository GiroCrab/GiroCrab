import SwiftUI

#if os(macOS)
import AppKit

/// macOS: the sidebar runs up to the window's top edge, but AppKit starts the detail column's
/// toolbar background and the line under it 2 pt left of the sidebar's edge, so they covered the
/// sidebar divider there and poked into the sidebar. This moves their start behind the divider:
/// the split view's divider, which runs the full window height, then shows through the title bar,
/// the sidebar is cut off along its whole right edge and the toolbar line starts at the divider.
/// AppKit lays the title bar out again on resizes and sidebar changes, so the trim is reapplied
/// whenever the background's frame or the split view changes. Both pieces are AppKit internals
/// found by shape; if AppKit draws them differently, nothing is changed.
private struct ToolbarBackgroundTrim: NSViewRepresentable {
    final class Probe: NSView {
        private weak var splitView: NSSplitView?
        private weak var background: NSView?
        private var observers: [NSObjectProtocol] = []
        /// The toolbar line's layers: AppKit moves a line back to its old start a moment after the
        /// window opens without changing any view frame, so the layers themselves are watched.
        private var lineObservers: [ObjectIdentifier: [NSKeyValueObservation]] = [:]
        private var applying = false

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            if window == nil {
                stopObserving()
            } else {
                // The split view and the title bar are in place once SwiftUI has laid out.
                DispatchQueue.main.async { [weak self] in self?.apply() }
            }
        }

        func apply() {
            guard !applying, let window, let titlebar = window.standardWindowButton(.closeButton)?.superview else { return }
            // Also after the probe left the window and came back: that removed the observers.
            if observers.isEmpty || splitView?.window !== window || background?.superview !== titlebar {
                splitView = Self.sidebarSplitView(in: window.contentView)
                background = Self.detailBackground(in: titlebar)
                observe(window)
            }
            guard let splitView, let sidebar = splitView.arrangedSubviews.first,
                  !sidebar.isHidden, !splitView.isSubviewCollapsed(sidebar) else { return }
            // Window coordinates: the title bar sits in its own window in full screen.
            let edge = sidebar.convert(NSPoint(x: sidebar.bounds.maxX, y: 0), to: nil).x
            let start = titlebar.convert(NSPoint(x: edge, y: 0), from: nil).x + splitView.dividerThickness
            applying = true
            defer { applying = false }
            if let background, let frame = Self.trimmed(background.frame, to: start) {
                background.frame = frame
            }
            // The toolbar line: an image layer of the title bar container, 1 pt below the title bar.
            if let container = titlebar.superview, let containerLayer = container.layer {
                let lineStart = container.convert(NSPoint(x: start, y: 0), from: titlebar).x
                // Not the backing layer of a subview: a layer the container draws itself.
                for layer in containerLayer.sublayers ?? [] where (layer.delegate as? NSView)?.layer !== layer
                    && layer.contents != nil && layer.frame.height <= 1.5 {
                    watchLine(layer)
                    guard let frame = Self.trimmed(layer.frame, to: lineStart) else { continue }
                    CATransaction.begin()
                    CATransaction.setDisableActions(true)
                    layer.frame = frame
                    CATransaction.commit()
                }
                // The sidebar's line (shown once the list is scrolled): AppKit places it under the
                // title bar as it was when the layer was made. Opened from the lock screen, whose
                // title bar has no toolbar, that is 10 pt too high, and AppKit does not move it when
                // the toolbar comes. Its bottom belongs on the title bar's bottom edge, like the
                // toolbar line's.
                if !container.isFlipped {
                    for layer in containerLayer.sublayers ?? [] where (layer.delegate as? NSView)?.layer !== layer
                        && layer.contents != nil && layer.frame.height <= 5 && layer.frame.minX < lineStart
                        && layer.frame.maxY > 0.25 && layer.frame.maxY < container.bounds.height {
                        watchLine(layer)
                        CATransaction.begin()
                        CATransaction.setDisableActions(true)
                        layer.frame.origin.y = -layer.frame.height
                        CATransaction.commit()
                    }
                }
            }
        }

        /// `frame` starting at `start` when AppKit let it begin a little left of it; nil otherwise
        /// (never widens anything, and leaves frames alone that start somewhere else entirely).
        private static func trimmed(_ frame: NSRect, to start: CGFloat) -> NSRect? {
            guard frame.minX < start - 0.25, frame.minX > start - 8, frame.maxX > start else { return nil }
            var out = frame
            out.size.width = frame.maxX - start
            out.origin.x = start
            return out
        }

        private func observe(_ window: NSWindow) {
            stopObserving()
            let center = NotificationCenter.default
            // Once now, once after AppKit has finished the layout pass that sent the notification.
            let refresh: (Notification) -> Void = { [weak self] _ in
                MainActor.assumeIsolated {
                    self?.apply()
                    DispatchQueue.main.async { self?.apply() }
                }
            }
            if let splitView {
                observers.append(center.addObserver(forName: NSSplitView.didResizeSubviewsNotification,
                                                    object: splitView, queue: .main, using: refresh))
            }
            // The sidebar's line appears with the first scroll: check again when the list scrolls.
            if let sidebar = splitView?.arrangedSubviews.first, let clip = Self.firstScrollView(in: sidebar)?.contentView {
                clip.postsBoundsChangedNotifications = true
                observers.append(center.addObserver(forName: NSView.boundsDidChangeNotification,
                                                    object: clip, queue: .main, using: refresh))
            }
            if let background {
                background.postsFrameChangedNotifications = true
                observers.append(center.addObserver(forName: NSView.frameDidChangeNotification,
                                                    object: background, queue: .main, using: refresh))
            }
            for name in [NSWindow.didResizeNotification, NSWindow.didEnterFullScreenNotification,
                         NSWindow.didExitFullScreenNotification, NSWindow.didBecomeKeyNotification] {
                observers.append(center.addObserver(forName: name, object: window, queue: .main, using: refresh))
            }
        }

        private func stopObserving() {
            observers.forEach(NotificationCenter.default.removeObserver)
            observers = []
            lineObservers.values.joined().forEach { $0.invalidate() }
            lineObservers = [:]
        }

        /// Trims again whenever AppKit moves or resizes the line layer. The trim itself also
        /// reports a change; the second pass then finds the line in place and leaves it.
        private func watchLine(_ layer: CALayer) {
            let id = ObjectIdentifier(layer)
            guard lineObservers[id] == nil else { return }
            let changed: @Sendable (CALayer, NSKeyValueObservedChange<CGPoint>) -> Void = { [weak self] _, _ in
                DispatchQueue.main.async { MainActor.assumeIsolated { self?.apply() } }
            }
            let resized: @Sendable (CALayer, NSKeyValueObservedChange<CGRect>) -> Void = { [weak self] _, _ in
                DispatchQueue.main.async { MainActor.assumeIsolated { self?.apply() } }
            }
            lineObservers[id] = [layer.observe(\.position, changeHandler: changed),
                                 layer.observe(\.bounds, changeHandler: resized)]
        }

        private static func firstScrollView(in view: NSView) -> NSScrollView? {
            if let scroll = view as? NSScrollView { return scroll }
            for sub in view.subviews {
                if let found = firstScrollView(in: sub) { return found }
            }
            return nil
        }

        /// The `NavigationSplitView`'s split view: the one with a sidebar item.
        private static func sidebarSplitView(in view: NSView?) -> NSSplitView? {
            guard let view else { return nil }
            if let split = view as? NSSplitView, let controller = split.delegate as? NSSplitViewController,
               controller.splitViewItems.first?.behavior == .sidebar {
                return split
            }
            for sub in view.subviews {
                if let found = sidebarSplitView(in: sub) { return found }
            }
            return nil
        }

        /// The title bar's background over the detail column: of AppKit's per-column
        /// backgrounds, the one that does not start at the window's left edge.
        private static func detailBackground(in titlebar: NSView) -> NSView? {
            titlebar.subviews
                .filter { $0.className == "NSTitlebarBackgroundView" && $0.frame.minX > 0 }
                .min { $0.frame.minX < $1.frame.minX }
        }
    }

    func makeNSView(context: Context) -> Probe { Probe() }
    func updateNSView(_ nsView: Probe, context: Context) { nsView.apply() }
}

extension View {
    /// Lets the sidebar divider run up through the toolbar to the window's top edge. No-op on iOS.
    func sidebarDividerToTop() -> some View {
        background(ToolbarBackgroundTrim().frame(width: 0, height: 0).accessibilityHidden(true))
    }
}
#else
extension View {
    func sidebarDividerToTop() -> some View { self }
}
#endif
