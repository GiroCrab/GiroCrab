// test: serial-defaults
import XCTest
@testable import GiroCrabUI

/// `swift test --parallel` runs several processes at once; with one shared
/// `UserDefaults.standard` they overwrote each other's settings, so tests like
/// `testViewChoiceAndCollapseStatePersist` failed at random. Each test process
/// keeps its settings in memory, the app keeps `.standard`.
final class AppDefaultsTests: XCTestCase {
    func testAppUsesTheStandardStore() {
        XCTAssertTrue(AppDefaults.makeStore(unitTests: false) === UserDefaults.standard)
    }

    func testThisTestRunIsIsolated() {
        XCTAssertFalse(AppDefaults.store === UserDefaults.standard)
        AppDefaults.store.set("x", forKey: "AppDefaultsTests.probe")
        XCTAssertNil(UserDefaults.standard.string(forKey: "AppDefaultsTests.probe"))
        AppDefaults.store.removeObject(forKey: "AppDefaultsTests.probe")
    }

    /// A read always sees the last write: the file-backed suite used before lost
    /// writes now and then on CI, or went back to an older value.
    func testEveryReadSeesTheLastWrite() {
        let store = AppDefaults.makeStore(unitTests: true)
        store.set([Int64(1), 2], forKey: "list")
        store.set([Int64(1)], forKey: "list")
        XCTAssertEqual(store.array(forKey: "list") as? [Int64], [1])
        store.removeObject(forKey: "list")
        XCTAssertNil(store.array(forKey: "list"))

        store.set(true, forKey: "flag")
        store.set(3, forKey: "count")
        store.set("byKind", forKey: "mode")
        store.set(["a"], forKey: "names")
        XCTAssertTrue(store.bool(forKey: "flag"))
        XCTAssertEqual(store.integer(forKey: "count"), 3)
        XCTAssertEqual(store.string(forKey: "mode"), "byKind")
        XCTAssertEqual(store.stringArray(forKey: "names"), ["a"])
        XCTAssertFalse(store.bool(forKey: "missing"))
        XCTAssertNil(store.object(forKey: "AppleLanguages"), "nothing from the global domain")
    }

    /// The model reloads when the running balance is switched (#577): key-value
    /// observation must work on the test store too.
    func testRunningBalanceSwitchIsObservable() {
        let store = AppDefaults.makeStore(unitTests: true)
        var seen: [Bool] = []
        let observer = store.observe(\.showRunningBalance, options: [.new]) { _, change in
            if let on = change.newValue { seen.append(on) }
        }
        store.set(true, forKey: ViewLayout.runningBalanceKey)
        store.set(false, forKey: ViewLayout.runningBalanceKey)
        observer.invalidate()
        XCTAssertEqual(seen, [true, false])
    }
}
