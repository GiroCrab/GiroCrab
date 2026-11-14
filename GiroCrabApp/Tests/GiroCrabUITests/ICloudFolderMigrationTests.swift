import XCTest
@testable import GiroCrabUI

/// iCloud Drive folder: new installs use "GiroCrab"; the former folder is copied once, never touched.
final class ICloudFolderMigrationTests: XCTestCase {
    private var root: URL!
    private let fm = FileManager.default
    private var old: URL { root.appendingPathComponent(ICloudFolderMigration.legacyFolderName) }
    private var new: URL { root.appendingPathComponent(ICloudFolderMigration.folderName) }
    private let snapshot = "tresor-01234567-89ab-cdef-0123-456789abcdef.girocrab"

    override func setUpWithError() throws {
        root = fm.temporaryDirectory.appendingPathComponent("icloud-mig-\(UUID().uuidString)")
        try fm.createDirectory(at: root, withIntermediateDirectories: true)
    }

    override func tearDown() { try? fm.removeItem(at: root) }

    private func put(_ name: String, in dir: URL, _ text: String = "x") throws {
        try fm.createDirectory(at: dir, withIntermediateDirectories: true)
        try Data(text.utf8).write(to: dir.appendingPathComponent(name))
    }

    private func requireSnapshotName() throws {
        try put(snapshot, in: root.appendingPathComponent("probe"))
        try XCTSkipUnless(SyncFolder.containsVault(root.appendingPathComponent("probe")), "snapshot name pattern changed")
        try fm.removeItem(at: root.appendingPathComponent("probe"))
    }

    func test_fresh_install_uses_new_folder() {
        let r = ICloudFolderMigration.resolve(cloudDocs: root)
        XCTAssertEqual(r.url.lastPathComponent, "GiroCrab")
        XCTAssertFalse(r.conflict)
        XCTAssertTrue(fm.fileExists(atPath: new.path))
        XCTAssertFalse(fm.fileExists(atPath: old.path))
    }

    func test_migration_copies_and_keeps_old() throws {
        try requireSnapshotName()
        try put(snapshot, in: old, "data")
        let r = ICloudFolderMigration.resolve(cloudDocs: root)
        XCTAssertEqual(r.url.lastPathComponent, "GiroCrab")
        XCTAssertFalse(r.conflict)
        XCTAssertEqual(try String(contentsOf: new.appendingPathComponent(snapshot), encoding: .utf8), "data")
        XCTAssertEqual(try String(contentsOf: old.appendingPathComponent(snapshot), encoding: .utf8), "data")
        // no staging leftovers
        XCTAssertEqual(try fm.contentsOfDirectory(atPath: root.path).sorted(), ["GiroCrab", "moneymoney"])
    }

    func test_migration_into_empty_existing_target() throws {
        try requireSnapshotName()
        try put(snapshot, in: old)
        try put(".DS_Store", in: new)
        let r = ICloudFolderMigration.resolve(cloudDocs: root)
        XCTAssertEqual(r.url, new)
        XCTAssertTrue(fm.fileExists(atPath: new.appendingPathComponent(snapshot).path))
    }

    func test_target_with_foreign_files_keeps_old_and_flags_conflict() throws {
        try requireSnapshotName()
        try put(snapshot, in: old)
        try put("foreign.txt", in: new)
        let r = ICloudFolderMigration.resolve(cloudDocs: root)
        XCTAssertEqual(r.url, old)
        XCTAssertTrue(r.conflict)
        XCTAssertTrue(fm.fileExists(atPath: new.appendingPathComponent("foreign.txt").path))
        XCTAssertTrue(fm.fileExists(atPath: old.appendingPathComponent(snapshot).path))
        XCTAssertFalse(fm.fileExists(atPath: new.appendingPathComponent(snapshot).path))
    }

    func test_target_already_migrated_is_used_without_notice() throws {
        try requireSnapshotName()
        try put(snapshot, in: old, "old")
        try put(snapshot, in: new, "new")
        let r = ICloudFolderMigration.resolve(cloudDocs: root)
        XCTAssertEqual(r, .init(url: new, conflict: false))
        XCTAssertEqual(try String(contentsOf: new.appendingPathComponent(snapshot), encoding: .utf8), "new")
    }

    func test_missing_old_folder_uses_new() throws {
        try put("foreign.txt", in: new)
        let r = ICloudFolderMigration.resolve(cloudDocs: root)
        XCTAssertEqual(r, .init(url: new, conflict: false))
    }

    func test_old_folder_without_vault_is_ignored() throws {
        try put("notes.txt", in: old)
        XCTAssertEqual(ICloudFolderMigration.resolve(cloudDocs: root).url, new)
    }

    func test_ui_name_hides_old_folder() {
        XCTAssertEqual(ICloudFolderMigration.displayName(for: old), "iCloud Drive")
        XCTAssertEqual(ICloudFolderMigration.displayName(for: new), "GiroCrab")
    }
}
