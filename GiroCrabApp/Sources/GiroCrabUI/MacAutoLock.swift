#if os(macOS)
import AppKit
import Foundation

/// Einstellungen › Sicherheit › Tresor (macOS): locks the personal vault when the main
/// window goes into the Dock (minimize, ⌘M), the app is hidden (⌘H) or the last main
/// window closes while the app keeps running (Dock / menu bar option), and after a time without mouse or
/// keyboard input in the app. Armed only while the own vault is open and unlocking
/// asks for Touch ID; the demo is never locked. The settings stay on this device
/// (`AppDefaults`). Locking goes through `AppModel.lock()`, so it clears the same
/// state as the manual lock (QuickLook copies, bank dialogs, running loads).
@MainActor
final class MacAutoLock {
    /// "Sperren beim Ablegen im Dock" (also covers hiding the app and closing the window).
    static let onMinimizeKey = "girocrab.autoLock.onMinimize"
    /// "Sperren nach … Minuten Inaktivität".
    static let idleKey = "girocrab.autoLock.idle"
    static let idleMinutesKey = "girocrab.autoLock.idleMinutes"
    static let defaultIdleMinutes = 5
    static let idleMinutesRange = 1...240

    enum Reason: Equatable { case minimized, windowClosed, appHidden, idle }

    /// The instance the app runs (`install(for:)`); nil in unit tests.
    private(set) static var shared: MacAutoLock?

    /// Starts the app's instance once, for the app's one model.
    static func install(for model: AppModel) {
        guard shared == nil, !AppModel.runningUnitTests else { return }
        let autoLock = MacAutoLock(
            isArmed: { [weak model] in model.map(armed) ?? false },
            lock: { [weak model] _ in model?.lock() }
        )
        autoLock.start()
        shared = autoLock
    }

    /// The vault the auto lock may close: the own one, open, behind Touch ID.
    static func armed(_ model: AppModel) -> Bool {
        model.holdsRealData && model.vaultState == .open && VaultKeyManager.shared.requireBiometrics
    }

    /// The stored minutes, clamped to `idleMinutesRange` (unset: the default).
    static func idleMinutes(in store: UserDefaults) -> Int {
        let stored = store.integer(forKey: idleMinutesKey)
        guard stored > 0 else { return defaultIdleMinutes }
        return min(max(stored, idleMinutesRange.lowerBound), idleMinutesRange.upperBound)
    }

    private let store: UserDefaults
    private let now: () -> Date
    private let isArmed: () -> Bool
    private let lock: (Reason) -> Void
    /// Replaces the minutes setting (tests run with fractions of a second).
    var idleIntervalOverride: TimeInterval?
    private(set) var lastActivity: Date
    private var armedAtLastCheck = false
    private var eventMonitor: Any?
    private var observers: [NSObjectProtocol] = []
    private var ticker: Task<Void, Never>?

    init(store: UserDefaults = AppDefaults.store, now: @escaping () -> Date = Date.init,
         isArmed: @escaping () -> Bool, lock: @escaping (Reason) -> Void) {
        self.store = store
        self.now = now
        self.isArmed = isArmed
        self.lock = lock
        lastActivity = now()
    }

    var lockOnMinimize: Bool { store.bool(forKey: Self.onMinimizeKey) }
    var lockWhenIdle: Bool { store.bool(forKey: Self.idleKey) }
    var idleInterval: TimeInterval { idleIntervalOverride ?? TimeInterval(Self.idleMinutes(in: store) * 60) }

    /// A mouse or keyboard event reached the app.
    func noteActivity() { lastActivity = now() }

    /// The last window showing the vault is closing (the app keeps running only with
    /// the Dock or menu bar option; otherwise quitting closes the vault anyway).
    func mainWindowClosed() { lockIfArmed(.windowClosed, enabled: lockOnMinimize) }

    /// The app was hidden (⌘H, "GiroCrab ausblenden"): treated like putting it in the Dock.
    func appHidden() { lockIfArmed(.appHidden, enabled: lockOnMinimize) }

    /// `window` went into the Dock; locks when no other window shows the vault on screen.
    func windowDidMiniaturize(_ window: NSWindow, mainWindows: [NSWindow],
                              isOnScreen: (NSWindow) -> Bool = MacAutoLock.isOnScreen) {
        guard mainWindows.contains(window) else { return }
        guard !mainWindows.contains(where: { $0 !== window && isOnScreen($0) }) else { return }
        lockIfArmed(.minimized, enabled: lockOnMinimize)
    }

    /// Locks once the app saw no input for `idleInterval`. The interval starts afresh on
    /// the first check after arming (unlock, switching the setting on), not at the last
    /// check before it.
    func checkIdle() {
        let armedNow = lockWhenIdle && isArmed()
        defer { armedAtLastCheck = armedNow }
        guard armedNow, armedAtLastCheck else { noteActivity(); return }
        guard now().timeIntervalSince(lastActivity) >= idleInterval else { return }
        noteActivity()
        lock(.idle)
    }

    /// `window` is closing; locks when no other window shows the vault on screen (a
    /// minimized one does not count, the same rule as for minimizing).
    func windowWillClose(_ window: NSWindow, mainWindows: [NSWindow],
                         isOnScreen: (NSWindow) -> Bool = MacAutoLock.isOnScreen) {
        guard mainWindows.contains(window) else { return }
        guard !mainWindows.contains(where: { $0 !== window && isOnScreen($0) }) else { return }
        mainWindowClosed()
    }

    /// A main window that shows the vault right now: visible and not in the Dock.
    static func isOnScreen(_ window: NSWindow) -> Bool { window.isVisible && !window.isMiniaturized }

    private func lockIfArmed(_ reason: Reason, enabled: Bool) {
        guard enabled, isArmed() else { return }
        lock(reason)
    }

    /// Watches input, hiding, minimizing and window closes, and checks the idle time every `period`.
    func start(checkEvery period: Duration = .seconds(10)) {
        guard ticker == nil else { return }
        noteActivity()
        let input: NSEvent.EventTypeMask = [
            .keyDown, .flagsChanged, .leftMouseDown, .rightMouseDown, .otherMouseDown,
            .mouseMoved, .leftMouseDragged, .rightMouseDragged, .scrollWheel,
        ]
        eventMonitor = NSEvent.addLocalMonitorForEvents(matching: input) { [weak self] event in
            MainActor.assumeIsolated { self?.noteActivity() }
            return event
        }
        let center = NotificationCenter.default
        observers.append(center.addObserver(forName: NSApplication.didHideNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.appHidden() }
        })
        observers.append(center.addObserver(forName: NSWindow.willCloseNotification, object: nil, queue: .main) { [weak self] note in
            guard let window = note.object as? NSWindow else { return }
            MainActor.assumeIsolated { self?.windowWillClose(window, mainWindows: BugReport.mainWindows()) }
        })
        observers.append(center.addObserver(forName: NSWindow.didMiniaturizeNotification, object: nil, queue: .main) { [weak self] note in
            guard let window = note.object as? NSWindow else { return }
            MainActor.assumeIsolated { self?.windowDidMiniaturize(window, mainWindows: BugReport.mainWindows()) }
        })
        ticker = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: period)
                self?.checkIdle()
            }
        }
    }

    func stop() {
        ticker?.cancel()
        ticker = nil
        if let eventMonitor { NSEvent.removeMonitor(eventMonitor) }
        eventMonitor = nil
        observers.forEach(NotificationCenter.default.removeObserver)
        observers = []
    }
}
#endif
