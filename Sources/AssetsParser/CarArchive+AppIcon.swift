import Foundation

extension CarArchive {
    /// 默认的应用图标资源名。
    private static let defaultIconName = "AppIcon"

    /// 按清晰度从高到低返回应用图标候选条目。
    ///
    /// - Parameter iconNames: Info.plist 中声明的图标资源名（`CFBundleIconName`）。
    ///   没有任何同名条目时，回退为以 `AppIcon` 开头的资源。
    /// - Note: 默认外观（非深色/着色）优先，其次按像素面积降序；同面积保持目录内顺序。
    public func appIconEntries(preferredNames iconNames: [String] = []) -> [CarEntry] {
        let rasterEntries = entries.filter {
            $0.kind == .image && ($0.preview == .rgba || $0.preview == .encoded) && $0.width > 0 && $0.height > 0
        }

        let declaredNames = Set(iconNames)
        var candidates = rasterEntries.filter { declaredNames.contains($0.name) }
        if candidates.isEmpty {
            candidates = rasterEntries.filter {
                $0.name.range(of: Self.defaultIconName, options: [.anchored, .caseInsensitive]) != nil
            }
        }

        return candidates.enumerated()
            .sorted { lhs, rhs in
                let lhsDefault = Self.isDefaultAppearance(lhs.element)
                let rhsDefault = Self.isDefaultAppearance(rhs.element)
                if lhsDefault != rhsDefault { return lhsDefault }
                let lhsArea = lhs.element.width * lhs.element.height
                let rhsArea = rhs.element.width * rhs.element.height
                if lhsArea != rhsArea { return lhsArea > rhsArea }
                return lhs.offset < rhs.offset
            }
            .map(\.element)
    }

    /// 返回最清晰的应用图标 PNG；候选依次尝试，全部失败返回 nil。
    public func bestAppIconPNGData(preferredNames iconNames: [String] = []) -> Data? {
        for entry in appIconEntries(preferredNames: iconNames) {
            if let data = try? pngData(for: entry) {
                return data
            }
        }
        return nil
    }

    private static func isDefaultAppearance(_ entry: CarEntry) -> Bool {
        let appearance = entry.appearance.lowercased()
        return appearance.isEmpty || appearance.contains("any")
    }
}
