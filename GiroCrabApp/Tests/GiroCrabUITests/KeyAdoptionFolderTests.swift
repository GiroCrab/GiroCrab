import XCTest
@testable import GiroCrabUI

/// No vault where the device looked (#1503): the paste sheet names the place and
/// offers „Ordner wählen …“; a picked folder without a vault is refused.
@MainActor
final class KeyAdoptionFolderTests: XCTestCase {
    private let fm = FileManager.default
    private var root: URL!
    private let snapshot = "tresor-01234567-89ab-cdef-0123-456789abcdef.girocrab"

    override func setUpWithError() throws {
        root = fm.temporaryDirectory.appendingPathComponent("adopt-folder-\(UUID().uuidString)")
        try fm.createDirectory(at: root, withIntermediateDirectories: true)
    }

    override func tearDown() { try? fm.removeItem(at: root) }

    private func dir(_ name: String, snapshot: Bool = false) throws -> URL {
        let url = root.appendingPathComponent(name, isDirectory: true)
        try fm.createDirectory(at: url, withIntermediateDirectories: true)
        if snapshot { try Data("v".utf8).write(to: url.appendingPathComponent(self.snapshot)) }
        return url
    }

    func test_container_message_points_to_the_icloud_drive_folder() {
        let text = KeyAdoptionState.notFoundMessage(location: .container)
        XCTAssertTrue(text.contains("iCloud Drive"), text)
        XCTAssertTrue(text.contains("„GiroCrab“"), text)
    }

    func test_folder_message_differs_from_container_message() {
        XCTAssertNotEqual(KeyAdoptionState.notFoundMessage(location: .folder), KeyAdoptionState.notFoundMessage(location: .container))
    }

    func test_folder_pick_offered_after_not_found_and_failure_only() {
        XCTAssertTrue(KeyAdoptionState.notFound("x", searched: nil).offersFolderPick)
        XCTAssertTrue(KeyAdoptionState.failed("Schlüssel passt nicht").offersFolderPick)
        for state in [KeyAdoptionState.idle, .working, .done] { XCTAssertFalse(state.offersFolderPick) }
    }

    func test_inspect_picked_folder() async throws {
        let withVault = try dir("GiroCrab", snapshot: true)
        let empty = try dir("Leer")
        let found = await ICloudFolderMigration.inspectPicked(withVault, wait: 0)
        XCTAssertEqual(found, .found)
        let parent = await ICloudFolderMigration.inspectPicked(root, wait: 0)
        XCTAssertEqual(parent, .subfolder("GiroCrab"))
        let none = await ICloudFolderMigration.inspectPicked(empty, wait: 0)
        XCTAssertEqual(none, .empty)
        XCTAssertNil(KeyAdoptionState.pickedFolderMessage(.found))
        XCTAssertNotNil(KeyAdoptionState.pickedFolderMessage(.empty))
        XCTAssertTrue(KeyAdoptionState.pickedFolderMessage(.subfolder("GiroCrab"))?.contains("„GiroCrab“") == true)
    }

    func test_picked_folder_without_vault_is_refused_and_not_stored() async throws {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        let cloud = CloudSyncManager.shared
        let chosenBefore = cloud.folderChosen
        let nameBefore = cloud.folderName
        let empty = try dir("Leer")
        model.adoptSyncKey(String(repeating: "ab", count: 32), folder: empty, wait: 0)
        XCTAssertEqual(model.keyAdoption, .working)
        for _ in 0..<100 where model.keyAdoption == .working { try await Task.sleep(for: .milliseconds(20)) }
        guard case .notFound(let message, let searched) = model.keyAdoption else { return XCTFail("expected .notFound, got \(model.keyAdoption)") }
        XCTAssertEqual(message, KeyAdoptionState.pickedFolderMessage(.empty))
        XCTAssertEqual(searched, ICloudFolderMigration.displayName(for: empty), "names the folder just picked, not the configured one")
        XCTAssertTrue(model.keyAdoption.offersFolderPick)
        XCTAssertEqual(cloud.folderChosen, chosenBefore, "a folder without a vault never becomes the sync folder")
        XCTAssertEqual(cloud.folderName, nameBefore)
        XCTAssertTrue(model.demoActive)
    }
}
