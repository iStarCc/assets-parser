import CoreGraphics
import Foundation

/// `.car` 文档信息。
public struct CarArchiveInfo: Decodable, Sendable, Equatable {
    public let storageVersion: UInt32
    public let coreUIVersion: UInt32
    public let schemaVersion: UInt32
    public let mainVersionString: String
    public let versionString: String
    public let authoringTool: String
    public let deploymentPlatform: String
    public let deploymentPlatformVersion: String
    public let renditionCount: UInt32
    public let entryCount: UInt32
}

public enum CarEntryKind: String, Decodable, Sendable {
    case image
    case color
    case document
    case rawData
    case other

    public init(from decoder: Decoder) throws {
        let raw = try decoder.singleValueContainer().decode(String.self)
        self = CarEntryKind(rawValue: raw) ?? .other
    }
}

/// 条目内容的获取方式。
public enum CarPreview: String, Decodable, Sendable {
    /// 解码为 RGBA8 像素。
    case rgba
    /// 原始字节可被系统（ImageIO）直接解码。
    case encoded
    /// 颜色条目，见 `CarEntry.color`。
    case color
    /// 无法预览。
    case none

    public init(from decoder: Decoder) throws {
        let raw = try decoder.singleValueContainer().decode(String.self)
        self = CarPreview(rawValue: raw) ?? .none
    }
}

public struct CarColor: Decodable, Sendable, Equatable {
    public let space: String
    public let components: [Double]

    /// 按目录中声明的色彩空间构造 `CGColor`。
    public var cgColor: CGColor? {
        let colorSpaceName: CFString
        switch space {
        case "display-p3": colorSpaceName = CGColorSpace.displayP3
        case "gray-gamma-2.2": colorSpaceName = CGColorSpace.genericGrayGamma2_2
        case "extended-srgb", "extended-linear-srgb": colorSpaceName = CGColorSpace.extendedSRGB
        case "extended-gray": colorSpaceName = CGColorSpace.extendedGray
        default: colorSpaceName = CGColorSpace.sRGB
        }
        guard let colorSpace = CGColorSpace(name: colorSpaceName),
              components.count == colorSpace.numberOfComponents + 1 else {
            return nil
        }
        return CGColor(colorSpace: colorSpace, components: components.map { CGFloat($0) })
    }
}

/// 目录中的一个 rendition（某资源名的一个具体变体）。
public struct CarEntry: Decodable, Sendable, Identifiable, Equatable {
    /// 在档案内的稳定序号，用于取回像素或原始数据。
    public let index: UInt32
    /// 资源名（facet），如 `AppIcon`。
    public let name: String
    /// rendition 名，通常是源文件名，如 `Icon-App-60x60@2x.png`。
    public let renditionName: String
    public let width: Int
    public let height: Int
    public let scale: Int
    /// `universal` / `phone` / `pad` / `tv` / `car` / `watch` / `marketing` / `mac` / `vision`，未知为空。
    public let idiom: String
    /// 外观键名（如 `UIAppearanceDark`），无外观变体为空。
    public let appearance: String
    /// 0 = sRGB，1 = Display P3。
    public let displayGamut: Int
    public let layout: String
    public let encoding: String
    public let kind: CarEntryKind
    public let preview: CarPreview
    /// 可导出的原始数据扩展名；为 nil 表示没有可导出的原始数据。
    public let originalExtension: String?
    public let color: CarColor?

    public var id: UInt32 { index }
}
