import CoreGraphics
import ImageIO
import XCTest
@testable import AssetsParser

final class CarArchiveTests: XCTestCase {
    private static let repositoryRoot: URL = {
        var url = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { url.deleteLastPathComponent() }
        return url
    }()

    private func fixtureURL(_ relativePath: String) throws -> URL {
        let url = Self.repositoryRoot.appendingPathComponent(relativePath)
        try XCTSkipUnless(FileManager.default.fileExists(atPath: url.path), "缺少测试夹具 \(relativePath)")
        return url
    }

    func testOpenFromFileExposesDocumentInfoAndEntries() throws {
        let archive = try CarArchive(fileURL: fixtureURL("Tests/Assets.car"))

        XCTAssertEqual(archive.info.deploymentPlatform, "ios")
        XCTAssertGreaterThan(archive.info.coreUIVersion, 0)
        XCTAssertEqual(Int(archive.info.entryCount), archive.entries.count)
        XCTAssertFalse(archive.entries.isEmpty)
    }

    func testOpenFromDataMatchesOpenFromFile() throws {
        let url = try fixtureURL("Tests/Assets.car")
        let fromFile = try CarArchive(fileURL: url)
        let fromData = try CarArchive(data: Data(contentsOf: url))

        XCTAssertEqual(fromFile.entries, fromData.entries)
    }

    func testEveryRasterEntryDecodesToItsDeclaredSize() throws {
        let archive = try CarArchive(fileURL: fixtureURL("Tests/Assets.car"))
        let rasterEntries = archive.entries.filter { $0.preview == .rgba }
        XCTAssertFalse(rasterEntries.isEmpty)

        for entry in rasterEntries {
            let image = try archive.image(for: entry)
            XCTAssertEqual(image.width, entry.width, "\(entry.name)/\(entry.renditionName)")
            XCTAssertEqual(image.height, entry.height, "\(entry.name)/\(entry.renditionName)")
        }
    }

    func testDeepmapLaunchImagesDecode() throws {
        let archive = try CarArchive(fileURL: fixtureURL("Tests/Assets.car"))
        let launchImages = archive.entries.filter { $0.name.hasPrefix("LaunchImage") }
        XCTAssertFalse(launchImages.isEmpty)

        for entry in launchImages {
            XCTAssertNoThrow(try archive.image(for: entry), entry.renditionName)
        }
    }

    func testPNGExportHasPNGSignature() throws {
        let archive = try CarArchive(fileURL: fixtureURL("Tests/Assets.car"))
        let entry = try XCTUnwrap(archive.entries.first { $0.preview == .rgba })

        let png = try archive.pngData(for: entry)

        XCTAssertEqual(Array(png.prefix(8)), [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
    }

    func testColorEntriesExposeComponents() throws {
        let archive = try CarArchive(
            fileURL: fixtureURL("Tests/LCSign/com.huami.LoadController_1.4_decrypted/Payload/LoadController.app/Assets.car")
        )
        let colors = archive.entries.filter { $0.kind == .color }
        XCTAssertFalse(colors.isEmpty)

        for entry in colors {
            XCTAssertEqual(entry.preview, .color)
            XCTAssertNotNil(entry.color?.cgColor, entry.name)
            XCTAssertThrowsError(try archive.image(for: entry))
        }
    }

    func testAppIconEntriesAreSortedByClarity() throws {
        let archive = try CarArchive(fileURL: fixtureURL("Tests/Assets.car"))
        let entries = archive.appIconEntries(preferredNames: ["AppIcon"])

        let first = try XCTUnwrap(entries.first)
        XCTAssertTrue(entries.allSatisfy { $0.name == "AppIcon" })
        XCTAssertEqual(first.width * first.height, entries.map { $0.width * $0.height }.max())
        XCTAssertFalse(first.appearance.lowercased().contains("dark"))
    }

    func testAppIconFallsBackToAppIconPrefixWhenNameIsUnknown() throws {
        let archive = try CarArchive(fileURL: fixtureURL("Tests/Assets.car"))

        XCTAssertEqual(
            archive.appIconEntries(preferredNames: ["NoSuchIcon"]).map(\.index),
            archive.appIconEntries().map(\.index)
        )
        XCTAssertFalse(archive.appIconEntries().isEmpty)
    }

    func testBestAppIconPNGDataDecodesToLargestSize() throws {
        let archive = try CarArchive(fileURL: fixtureURL("Tests/Assets.car"))
        let best = try XCTUnwrap(archive.appIconEntries(preferredNames: ["AppIcon"]).first)

        let png = try XCTUnwrap(archive.bestAppIconPNGData(preferredNames: ["AppIcon"]))
        let source = try XCTUnwrap(CGImageSourceCreateWithData(png as CFData, nil))
        let image = try XCTUnwrap(CGImageSourceCreateImageAtIndex(source, 0, nil))

        XCTAssertEqual(image.width, best.width)
        XCTAssertEqual(image.height, best.height)
    }

    func testInvalidDataThrowsInsteadOfCrashing() {
        XCTAssertThrowsError(try CarArchive(data: Data(repeating: 0x42, count: 256)))
        XCTAssertThrowsError(try CarArchive(data: Data()))
        XCTAssertThrowsError(try CarArchive(fileURL: URL(fileURLWithPath: "/nonexistent/Assets.car")))
    }
}
