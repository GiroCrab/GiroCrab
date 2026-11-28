#if os(macOS)
import AppKit
import XCTest
@testable import GiroCrabUI

/// Einstellungen › Sicherheit: lock on putting the main window into the Dock, on hiding the app
/// and after a time without input (`MacAutoLock`).
@MainActor
final class MacAutoLockTests: XCTestCase {
    private var store: UserDefaults!
    private var clock = Date(timeIntervalSinceReferenceDate: 0)
    private var armed = true
    private var locks: [MacAutoLock.Reason] = []

    override func setUp() async throws {
        store = AppDefaults.makeStore(unitTests: true)
        clock = Date(timeIntervalSinceReferenceDate: 0)
        armed = true
        locks = []
    }

    private func makeAutoLock() -> MacAutoLock {
        MacAutoLock(store: store, now: { [unowned self] in clock }, isArmed: { [unowned self] in armed },
                    lock: { [unowned self] in locks.append($0) })
    }

    private func window() -> NSWindow {
        let window = NSWindow(contentRect: .init(x: 0, y: 0, width: 100, height: 100), styleMask: [.titled], backing: .buffered, defer: true)
        window.isReleasedWhenClosed = false
        return window
    }

    func testDefaultsAreOffWithFiveMinutes() {
        let autoLock = makeAutoLock()
        XCTAssertFalse(autoLock.lockOnMinimize)
        XCTAssertFalse(autoLock.lockWhenIdle)
        XCTAssertEqual(autoLock.idleInterval, 5 * 60)
        store.set(0, forKey: MacAutoLock.idleMinutesKey)
        XCTAssertEqual(MacAutoLock.idleMinutes(in: store), 5)
        store.set(10_000, forKey: MacAutoLock.idleMinutesKey)
        XCTAssertEqual(MacAutoLock.idleMinutes(in: store), MacAutoLock.idleMinutesRange.upperBound)
    }

    func testMinimizingTheMainWindowLocksOnlyWhenEnabled() {
        let autoLock = makeAutoLock()
        let main = window()
        autoLock.windowDidMiniaturize(main, mainWindows: [main])
        XCTAssertEqual(locks, [], "setting off")

        store.set(true, forKey: MacAutoLock.onMinimizeKey)
        autoLock.windowDidMiniaturize(window(), mainWindows: [main])
        XCTAssertEqual(locks, [], "another window (Hilfe, Einstellungen) minimized")
        let other = window()
        autoLock.windowDidMiniaturize(main, mainWindows: [main, other], isOnScreen: { $0 === other })
        XCTAssertEqual(locks, [], "a second main window still shows the vault")
        autoLock.windowDidMiniaturize(main, mainWindows: [main, other], isOnScreen: { _ in false })
        XCTAssertEqual(locks, [.minimized])
    }

    func testClosingTheMainWindowLocksOnlyWhenEnabled() {
        let autoLock = makeAutoLock()
        let main = window()
        autoLock.windowWillClose(main, mainWindows: [main])
        XCTAssertEqual(locks, [], "setting off")

        store.set(true, forKey: MacAutoLock.onMinimizeKey)
        autoLock.windowWillClose(window(), mainWindows: [main])
        XCTAssertEqual(locks, [], "another window (Hilfe, Einstellungen) closed")
        autoLock.windowWillClose(main, mainWindows: [main])
        XCTAssertEqual(locks, [.windowClosed])
    }

    func testAnotherOpenMainWindowKeepsTheVaultOpen() {
        store.set(true, forKey: MacAutoLock.onMinimizeKey)
        let autoLock = makeAutoLock()
        let closing = window(), other = window()
        autoLock.windowWillClose(closing, mainWindows: [closing, other], isShown: { $0 === other })
        XCTAssertEqual(locks, [])
        autoLock.windowWillClose(closing, mainWindows: [closing, other], isShown: { _ in false })
        XCTAssertEqual(locks, [.windowClosed], "the other one was already closed")
    }

    func testHidingTheAppLocksWithTheWindowSetting() async {
        let autoLock = makeAutoLock()
        autoLock.appHidden()
        XCTAssertEqual(locks, [])
        store.set(true, forKey: MacAutoLock.onMinimizeKey)
        autoLock.start(checkEvery: .seconds(3600))
        defer { autoLock.stop() }
        NotificationCenter.default.post(name: NSApplication.didHideNotification, object: nil)
        for _ in 0..<50 where locks.isEmpty { try? await Task.sleep(for: .milliseconds(10)) }
        XCTAssertEqual(locks, [.appHidden])
    }

    func testNothingLocksWhileDisarmed() {
        store.set(true, forKey: MacAutoLock.onMinimizeKey)
        store.set(true, forKey: MacAutoLock.idleKey)
        armed = false // demo, locked vault or Touch ID off
        let autoLock = makeAutoLock()
        let main = window()
        autoLock.windowWillClose(main, mainWindows: [main])
        autoLock.windowDidMiniaturize(main, mainWindows: [main])
        autoLock.appHidden()
        clock += 3600
        autoLock.checkIdle()
        XCTAssertEqual(locks, [])
    }

    func testIdleLocksAfterTheIntervalWithoutInput() {
        store.set(true, forKey: MacAutoLock.idleKey)
        store.set(2, forKey: MacAutoLock.idleMinutesKey)
        let autoLock = makeAutoLock()
        clock += 90
        autoLock.checkIdle()
        XCTAssertEqual(locks, [])
        autoLock.noteActivity() // a click resets the clock
        clock += 90
        autoLock.checkIdle()
        XCTAssertEqual(locks, [])
        clock += 30
        autoLock.checkIdle()
        XCTAssertEqual(locks, [.idle])
        clock += 1
        autoLock.checkIdle()
        XCTAssertEqual(locks, [.idle], "locks once, then counts afresh")
    }

    /// Locked for an hour, then unlocked: the interval starts at the unlock, not before.
    func testIdleClockRestartsWhileDisarmed() {
        store.set(true, forKey: MacAutoLock.idleKey)
        let autoLock = makeAutoLock()
        armed = false
        clock += 3600
        autoLock.checkIdle()
        armed = true
        clock += 60
        autoLock.checkIdle()
        XCTAssertEqual(locks, [])
    }

    func testIdleTimerFiresWithAShortInterval() async {
        store.set(true, forKey: MacAutoLock.idleKey)
        let fired = expectation(description: "idle lock")
        fired.assertForOverFulfill = false
        let autoLock = MacAutoLock(store: store, isArmed: { true }, lock: { reason in
            XCTAssertEqual(reason, .idle)
            fired.fulfill()
        })
        autoLock.idleIntervalOverride = 0.1
        autoLock.start(checkEvery: .milliseconds(20))
        defer { autoLock.stop() }
        await fulfillment(of: [fired], timeout: 5)
    }
}
#endif
