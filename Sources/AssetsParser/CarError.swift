import Foundation

public enum CarError: LocalizedError, Sendable {
    case openFailed(String)
    case invalidMetadata(String)
    case decodeFailed(String)
    case unsupportedPreview

    public var errorDescription: String? {
        switch self {
        case .openFailed(let message): return message
        case .invalidMetadata(let message): return message
        case .decodeFailed(let message): return message
        case .unsupportedPreview: return "This entry has no previewable image data."
        }
    }
}
