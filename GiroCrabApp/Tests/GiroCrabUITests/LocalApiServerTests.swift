import XCTest
import GiroCrabCore
@testable import GiroCrabUI

/// The socket of the external analysis (#136) runs only for the own unlocked
/// vault, only when enabled with a token, and goes away on lock, revoke and demo.
@MainActor
final class LocalApiServerTests: XCTestCase {
    private var model: AppModel!
    private var dir: URL!

    override func setUp() async throws {
        dir = URL(fileURLWithPath: "/tmp/gc-\(UUID().uuidString.prefix(8))")
        AppModel.localApiSocketPathOverride = dir.appendingPathComponent("s.sock").path
        model = AppModel(options: LaunchOptions(arguments: ["--fresh-vault"], environment: [:]))
        XCTAssertEqual(model.vaultState, .open)
        XCTAssertTrue(model.holdsRealData)
        // The --fresh-vault file is wiped once per process: start every test from "off".
        await model.revokeLocalApiToken()
        await model.saveLocalApiScope(ShareScope(
            enabled: false, accountIds: [], shareAccountNumbers: false, shareCounterparty: true,
            sharePurpose: true, shareNotes: false, tokenSha256: ""))
        XCTAssertFalse(model.isLocalApiRunning)
    }

    override func tearDown() async throws {
        // Queued jobs may still start a server: let them finish first, while the
        // test path is set (a later one would bind the real socket path).
        for _ in 0..<3 {
            await Task.yield() // a job a didSet just spawned gets queued
            await model.localApiQueue?.value
        }
        model.stopLocalApiServer()
        model = nil
        AppModel.localApiSocketPathOverride = nil
        try? FileManager.default.removeItem(at: dir)
    }

    private var socketExists: Bool {
        FileManager.default.fileExists(atPath: AppModel.localApiSocketPath)
    }

    private func awaitRunning(_ running: Bool, file: StaticString = #filePath, line: UInt = #line) async throws {
        for _ in 0..<300 {
            if model.isLocalApiRunning == running { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("server running = \(model.isLocalApiRunning), expected \(running)", file: file, line: line)
    }

    private func enable() async throws {
        let loaded = await model.loadLocalApiScope()
        var scope = try XCTUnwrap(loaded)
        scope.enabled = true
        await model.saveLocalApiScope(scope)
    }

    func testStartsOnlyWhenEnabledWithAToken() async throws {
        try await enable()
        try await Task.sleep(for: .milliseconds(100))
        XCTAssertFalse(model.isLocalApiRunning, "no token yet")
        let token = await model.createLocalApiToken()
        XCTAssertEqual(token?.hasPrefix("gcapi_"), true)
        try await awaitRunning(true)
        XCTAssertTrue(socketExists)

        await model.revokeLocalApiToken()
        try await awaitRunning(false)
        XCTAssertFalse(socketExists)
    }

    func testLockAndDemoStopTheSocket() async throws {
        try await enable()
        _ = await model.createLocalApiToken()
        try await awaitRunning(true)
        model.showDemo()
        try await awaitRunning(false)
        XCTAssertFalse(socketExists, "never served for the demo")
        model.showOwnVault()
        try await awaitRunning(true)
        model.lock()
        XCTAssertFalse(model.isLocalApiRunning)
        XCTAssertFalse(socketExists)
    }

    func testTurningOffStopsTheSocket() async throws {
        try await enable()
        _ = await model.createLocalApiToken()
        try await awaitRunning(true)
        let loaded = await model.loadLocalApiScope()
        var scope = try XCTUnwrap(loaded)
        scope.enabled = false
        await model.saveLocalApiScope(scope)
        try await awaitRunning(false)
    }

    func testAStartErrorIsKeptForTheStatus() async throws {
        // Longer than sun_path (104 bytes): bind fails.
        AppModel.localApiSocketPathOverride = dir.appendingPathComponent(String(repeating: "x", count: 120)).path
        try await enable()
        _ = await model.createLocalApiToken()
        for _ in 0..<300 where model.localApiStartError == nil {
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertNotNil(model.localApiStartError)
        XCTAssertFalse(model.isLocalApiRunning)
        model.lock()
        XCTAssertNil(model.localApiStartError)
    }

    func testQuickSavesKeepTheNewest() async throws {
        let loaded = await model.loadLocalApiScope()
        var scope = try XCTUnwrap(loaded)
        scope.enabled = true
        // Switches, not account ids: the fresh vault may hold no accounts (the
        // core keeps only live ones).
        var saves: [Task<Bool, Never>] = []
        for (notes, purpose) in [(true, false), (false, false), (true, true)] {
            scope.shareNotes = notes
            scope.sharePurpose = purpose
            let copy = scope
            saves.append(Task { await model.saveLocalApiScope(copy) })
        }
        for s in saves { _ = await s.value }
        let stored = await model.loadLocalApiScope()
        XCTAssertEqual(stored?.shareNotes, true)
        XCTAssertEqual(stored?.sharePurpose, true)
    }

    func testOnThenQuicklyOffLeavesTheSocketOff() async throws {
        _ = await model.createLocalApiToken()
        let loaded = await model.loadLocalApiScope()
        var scope = try XCTUnwrap(loaded)
        scope.enabled = true
        let on = scope
        scope.enabled = false
        let off = scope
        // Tasks made on the main actor start in order (async let would not).
        let first = Task { await model.saveLocalApiScope(on) }
        let second = Task { await model.saveLocalApiScope(off) }
        _ = await (first.value, second.value)
        try await Task.sleep(for: .milliseconds(300))
        XCTAssertFalse(model.isLocalApiRunning)
        XCTAssertFalse(socketExists)
    }

    func testQueuedSavesLandInTheOwnVaultAcrossTheDemo() async throws {
        let loaded = await model.loadLocalApiScope()
        var scope = try XCTUnwrap(loaded)
        scope.enabled = true
        scope.shareNotes = true
        let save = Task { await model.saveLocalApiScope(scope) }
        await Task.yield() // queued while the own vault is shown
        model.showDemo()
        XCTAssertFalse(model.isLocalApiRunning)
        await save.value
        model.showOwnVault()
        let stored = await model.loadLocalApiScope()
        XCTAssertEqual(stored?.shareNotes, true)
    }

    func testAFailedStartIsRetried() async throws {
        let good = AppModel.localApiSocketPath
        AppModel.localApiSocketPathOverride = dir.appendingPathComponent(String(repeating: "x", count: 120)).path
        try await enable()
        _ = await model.createLocalApiToken()
        for _ in 0..<300 where model.localApiStartError == nil {
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertNotNil(model.localApiStartError)
        AppModel.localApiSocketPathOverride = good // the obstacle is gone
        model.retryLocalApiStart()
        try await awaitRunning(true)
        XCTAssertNil(model.localApiStartError)
    }

    func testASaveThatCannotRunSaysSo() async throws {
        let loaded = await model.loadLocalApiScope()
        let scope = try XCTUnwrap(loaded)
        model.showDemo()
        let saved = await model.saveLocalApiScope(scope)
        XCTAssertFalse(saved)
        model.showOwnVault()
    }

    func testANormalUnlockStartsTheSocket() async throws {
        try await enable()
        _ = await model.createLocalApiToken()
        try await awaitRunning(true)
        let v = try XCTUnwrap(model.ownVault)
        model.lock()
        XCTAssertFalse(model.isLocalApiRunning)
        // The unlock path of the lock screen, with no settings call afterwards.
        model.vaultState = .unlocking
        let unlock = Task { await model.adoptOpenedOwnVault(v, isEmpty: false) }
        await Task.yield()
        XCTAssertNotEqual(model.vaultState, .open)
        XCTAssertFalse(model.isLocalApiRunning, "not behind the lock screen")
        await unlock.value
        XCTAssertEqual(model.vaultState, .open)
        try await awaitRunning(true)
    }

    func testANewKeyIsShownUntilItIsNoLongerValid() async throws {
        let first = await model.createLocalApiToken()
        XCTAssertNotNil(first)
        XCTAssertEqual(model.localApiNewToken, first)
        let second = await model.createLocalApiToken()
        XCTAssertEqual(model.localApiNewToken, second, "the newest one")
        await model.revokeLocalApiToken()
        XCTAssertNil(model.localApiNewToken, "revoked")
        _ = await model.createLocalApiToken()
        model.lock()
        XCTAssertNil(model.localApiNewToken, "never shown again after locking")
    }

    func testTheModelHoldsWhatIsStored() async throws {
        let loaded = await model.loadLocalApiScope()
        var scope = try XCTUnwrap(loaded)
        scope.shareNotes = true
        model.updateLocalApiScope(scope)
        XCTAssertEqual(model.localApiScope?.shareNotes, true, "shown at once")
        _ = await model.loadLocalApiScope() // queued behind the save
        XCTAssertEqual(model.localApiScope?.shareNotes, true, "and stored")
        model.showDemo()
        XCTAssertNil(model.localApiScope, "nothing while the demo is shown")
        model.showOwnVault()
    }

    #if os(macOS)
    /// The exit codes of `girocrab-mcp --check` (girocrab-mcp/src/lib.rs,
    /// `Problem::exit_code`) and what the pane makes of them, with a stand-in
    /// script for the helper (runs in CI, where the helper is not built).
    func testEveryCheckExitCodeIsMapped() async throws {
        let cases: [(Int32, LocalApiCheck)] = [
            (0, .works), (3, .notReachable), (4, .reachableWithoutKey), (5, .keyRejected),
            (6, .turnedOff), (7, .locked), (8, .busy), (1, .failed("something odd")),
        ]
        let script = dir.appendingPathComponent("fake-helper")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { AppModel.localApiHelperOverride = nil }
        for (code, expected) in cases {
            try "#!/bin/sh\necho 'something odd' >&2\nexit \(code)\n".write(to: script, atomically: true, encoding: .utf8)
            try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: script.path)
            AppModel.localApiHelperOverride = script
            let result = await model.checkLocalApiConnection()
            XCTAssertEqual(result, expected, "exit \(code)")
        }
    }

    func testTheConnectionTestRunsTheHelperAgainstTheSocket() async throws {
        // Tests/GiroCrabUITests/<file> → the repository root.
        let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let built = root.appendingPathComponent("target/release/girocrab-mcp")
        try XCTSkipUnless(FileManager.default.isExecutableFile(atPath: built.path),
                          "girocrab-mcp not built (cargo build --release -p girocrab-mcp)")
        AppModel.localApiHelperOverride = built
        defer { AppModel.localApiHelperOverride = nil }
        try await enable()
        _ = await model.createLocalApiToken()
        try await awaitRunning(true)
        let works = await model.checkLocalApiConnection()
        XCTAssertEqual(works, .works)
        model.forgetNewLocalApiToken() // pane closed: no key at hand any more
        let reachable = await model.checkLocalApiConnection()
        XCTAssertEqual(reachable, .reachableWithoutKey)
        model.lock()
        let gone = await model.checkLocalApiConnection()
        XCTAssertEqual(gone, .notReachable)
    }

    func testTheClientEntryIsValidJSONWithPathAndKey() throws {
        let helper = URL(fileURLWithPath: "/Applications/Giro \"Crab\".app/Contents/Helpers/girocrab-mcp")
        let entry = AppModel.localApiClientConfig(helper: helper, token: "gcapi_abc")
        let json = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(entry.utf8)) as? [String: Any])
        let server = try XCTUnwrap((json["mcpServers"] as? [String: Any])?["girocrab"] as? [String: Any])
        XCTAssertEqual(server["command"] as? String, helper.path)
        XCTAssertEqual((server["env"] as? [String: String])?["GIROCRAB_API_TOKEN"], "gcapi_abc")
        XCTAssertTrue(AppModel.localApiClientConfig(helper: helper, token: nil).contains("gcapi_…"))
    }
    #endif
}
