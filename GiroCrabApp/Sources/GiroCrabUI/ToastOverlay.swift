import SwiftUI

/// Draws `model.toast` at the bottom of a window (GiroCrab/GiroCrab#1145).
///
/// Every window that shares `model` carries it: on macOS the Settings window and
/// the sync log lie in front of the main window, so a notice raised there would
/// otherwise appear behind them. Only one window draws a notice on macOS: the one
/// that was key last (kept while the app is in the background). On iOS/iPadOS
/// scenes do not stack as modal utility windows, so every visible scene draws it.
extension View {
    func toastOverlay() -> some View { modifier(ToastOverlay()) }
}

/// The windows that can draw a notice, the one that was key last at the end.
@MainActor @Observable
final class ToastHosts {
    static let shared = ToastHosts()
    private(set) var order: [UUID] = []

    /// A new window goes behind the others until it becomes key.
    func add(_ id: UUID) { if !order.contains(id) { order.insert(id, at: 0) } }
    func bringToFront(_ id: UUID) {
        guard order.last != id else { return }
        order.removeAll { $0 == id }
        order.append(id)
    }
    func remove(_ id: UUID) { order.removeAll { $0 == id } }
}

/// What the toast shows. It is a plain string drawn over the window and may name an
/// account, a merchant or a file, so `.sensitive()` cannot cover it: like `WindowTitle`,
/// a bug report capture or privacy mode masks it. A toast set through `showToast` lists
/// its names (`hiding`), so only those become a placeholder (#1998); an unchecked toast
/// (`hiding == nil`) shows a neutral notice instead.
enum ToastText {
    static func shown(_ toast: String, hiding names: [String]? = nil, redaction: ReportRedaction) -> String {
        guard !redaction.isDisjoint(with: [.personal, .labels]) else { return toast }
        guard let names else { return String(localized: "Kein Klartext – Hinweis ausgeblendet") }
        // Longest first: "Muster" must not leave " GmbH" of "Muster GmbH" readable.
        return names.filter { !$0.isEmpty }.sorted { $0.count > $1.count }
            .reduce(toast) { $0.replacingOccurrences(of: $1, with: ReportRedaction.placeholder) }
    }
}

extension AppModel {
    /// Shows `text` as toast. `names` are every account, merchant or file name inside it;
    /// while personal data is hidden only they are masked (#1998).
    func showToast(_ text: String, hiding names: [String] = []) {
        toast = text
        toastHiding = names
    }
}

private struct ToastOverlay: ViewModifier {
    @Environment(AppModel.self) private var model
    @Environment(\.reportRedaction) private var redaction
    #if os(macOS)
    @State private var id = UUID()
    @Environment(\.controlActiveState) private var activeState
    private var isHost: Bool { ToastHosts.shared.order.last == id }
    #else
    private var isHost: Bool { true }
    #endif

    func body(content: Content) -> some View {
        content
            .overlay(alignment: .bottom) {
                if isHost, let toast = model.toast {
                    Label(ToastText.shown(toast, hiding: model.toastHiding, redaction: redaction), systemImage: "checkmark.circle.fill")
                        .padding(.horizontal, 16).padding(.vertical, 10)
                        .background(.regularMaterial, in: Capsule())
                        .shadow(color: .black.opacity(0.08), radius: 8, y: 2)
                        .padding(.bottom, 22)
                        .transition(Motion.slide(.bottom))
                        // id: a changed or extended notice gets its full time again
                        .task(id: toast) {
                            try? await Task.sleep(nanoseconds: 3_500_000_000)
                            // replaced meanwhile: the new notice's own task takes over
                            guard !Task.isCancelled else { return }
                            withAnimation { model.toast = nil }
                        }
                }
            }
            .animation(Motion.animation(.spring(duration: 0.3)), value: model.toast)
            #if os(macOS)
            .onAppear { ToastHosts.shared.add(id) }
            .onDisappear { ToastHosts.shared.remove(id) }
            .onChange(of: activeState, initial: true) { _, state in
                if state == .key { ToastHosts.shared.bringToFront(id) }
            }
            #endif
    }
}
