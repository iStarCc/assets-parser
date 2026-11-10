import CarParserFFI
import CoreGraphics
import Foundation
import ImageIO

/// 已打开的 `.car` 资源目录。底层由 Rust `car-parser` 解析，所有访问通过内部锁串行化。
public final class CarArchive: @unchecked Sendable {
    public let info: CarArchiveInfo
    public let entries: [CarEntry]

    private let handle: OpaquePointer
    private let lock = NSLock()

    /// 从文件打开（内存映射，不会把整个文件读入内存）。
    public convenience init(fileURL: URL) throws {
        let handle = try Self.openHandle { error in
            fileURL.path.withCString { car_archive_open_path($0, error) }
        }
        try self.init(handle: handle)
    }

    /// 从内存数据打开（会复制一份数据）。
    public convenience init(data: Data) throws {
        let handle = try Self.openHandle { error in
            data.withUnsafeBytes { buffer in
                car_archive_open_bytes(buffer.bindMemory(to: UInt8.self).baseAddress, buffer.count, error)
            }
        }
        try self.init(handle: handle)
    }

    private init(handle: OpaquePointer) throws {
        self.handle = handle
        do {
            let decoder = JSONDecoder()
            let infoJSON = try Self.jsonData { car_archive_info_json(handle, $0) }
            let entriesJSON = try Self.jsonData { car_archive_entries_json(handle, $0) }
            self.info = try Self.decode(CarArchiveInfo.self, from: infoJSON, using: decoder)
            self.entries = try Self.decode([CarEntry].self, from: entriesJSON, using: decoder)
        } catch {
            car_archive_close(handle)
            throw error
        }
    }

    deinit {
        car_archive_close(handle)
    }

    // MARK: - 取回内容

    /// 取回条目的图像：RGBA 解码或由系统解码原始字节。
    public func image(for entry: CarEntry) throws -> CGImage {
        switch entry.preview {
        case .rgba:
            return try rgbaImage(for: entry)
        case .encoded:
            let data = try originalData(for: entry)
            guard let source = CGImageSourceCreateWithData(data as CFData, nil),
                  let image = CGImageSourceCreateImageAtIndex(source, 0, nil) else {
                throw CarError.decodeFailed("System image decoder rejected the \(entry.originalExtension ?? "encoded") data.")
            }
            return image
        case .color, .none:
            throw CarError.unsupportedPreview
        }
    }

    /// 取回条目的原始字节（JPEG/WEBP/HEIF/PDF/SVG/Data 等）。
    public func originalData(for entry: CarEntry) throws -> Data {
        var buffer = CarBuffer(data: nil, len: 0)
        try locked {
            try Self.checkSuccess { car_archive_entry_original(handle, entry.index, &buffer, $0) }
        }
        defer { car_buffer_free(buffer) }
        guard let base = buffer.data else { return Data() }
        return Data(bytes: base, count: buffer.len)
    }

    /// 将条目图像编码为 PNG（带正确的色彩空间）。
    public func pngData(for entry: CarEntry) throws -> Data {
        let image = try image(for: entry)
        let output = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(output, "public.png" as CFString, 1, nil) else {
            throw CarError.decodeFailed("Cannot create PNG encoder.")
        }
        CGImageDestinationAddImage(destination, image, nil)
        guard CGImageDestinationFinalize(destination) else {
            throw CarError.decodeFailed("PNG encoding failed.")
        }
        return output as Data
    }

    // MARK: - RGBA

    private func rgbaImage(for entry: CarEntry) throws -> CGImage {
        var raw = CarRgbaImage(width: 0, height: 0, pixels: CarBuffer(data: nil, len: 0))
        try locked {
            try Self.checkSuccess { car_archive_entry_rgba(handle, entry.index, &raw, $0) }
        }
        defer { car_buffer_free(raw.pixels) }

        let width = Int(raw.width)
        let height = Int(raw.height)
        guard width > 0, height > 0, let base = raw.pixels.data, raw.pixels.len == width * height * 4 else {
            throw CarError.decodeFailed("Decoded pixel buffer has an unexpected size.")
        }

        let colorSpaceName = entry.displayGamut == 1 ? CGColorSpace.displayP3 : CGColorSpace.sRGB
        guard let colorSpace = CGColorSpace(name: colorSpaceName),
              let provider = CGDataProvider(data: Data(bytes: base, count: raw.pixels.len) as CFData),
              let image = CGImage(
                  width: width,
                  height: height,
                  bitsPerComponent: 8,
                  bitsPerPixel: 32,
                  bytesPerRow: width * 4,
                  space: colorSpace,
                  bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.last.rawValue),
                  provider: provider,
                  decode: nil,
                  shouldInterpolate: true,
                  intent: .defaultIntent
              ) else {
            throw CarError.decodeFailed("Cannot create image from decoded pixels.")
        }
        return image
    }

    // MARK: - FFI 辅助

    private func locked<T>(_ body: () throws -> T) rethrows -> T {
        lock.lock()
        defer { lock.unlock() }
        return try body()
    }

    /// 执行返回句柄的 FFI 调用，失败（NULL）时抛出携带 Rust 错误描述的 `CarError`。
    private static func openHandle(_ call: (UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>) -> OpaquePointer?) throws -> OpaquePointer {
        var errorPointer: UnsafeMutablePointer<CChar>?
        if let result = call(&errorPointer) {
            return result
        }
        throw CarError.openFailed(consumeError(errorPointer))
    }

    /// 执行返回 JSON 字符串的 FFI 调用并释放 Rust 字符串。
    private static func jsonData(_ call: (UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>) -> UnsafeMutablePointer<CChar>?) throws -> Data {
        var errorPointer: UnsafeMutablePointer<CChar>?
        guard let text = call(&errorPointer) else {
            throw CarError.invalidMetadata(consumeError(errorPointer))
        }
        defer { car_string_free(text) }
        return Data(bytes: text, count: strlen(text))
    }

    private static func checkSuccess(_ call: (UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>) -> Bool) throws {
        var errorPointer: UnsafeMutablePointer<CChar>?
        guard call(&errorPointer) else {
            throw CarError.decodeFailed(consumeError(errorPointer))
        }
    }

    private static func consumeError(_ pointer: UnsafeMutablePointer<CChar>?) -> String {
        guard let pointer else { return "Unknown error." }
        defer { car_string_free(pointer) }
        return String(cString: pointer)
    }

    private static func decode<T: Decodable>(_ type: T.Type, from data: Data, using decoder: JSONDecoder) throws -> T {
        do {
            return try decoder.decode(type, from: data)
        } catch {
            throw CarError.invalidMetadata(error.localizedDescription)
        }
    }
}
