import Foundation

/// Display formatting follows regional preferences, independently of app language.
enum LocaleText {
    static func integer<T: BinaryInteger>(_ value: T, locale: Locale = .current) -> String {
        Int64(clamping:value).formatted(.number.locale(locale))
    }
    static func percent(_ value: Double) -> String {
        // Preserve truncation: an unfinished transfer must not round up to 100%.
        ((value * 100).rounded(.down) / 100).formatted(.percent.precision(.fractionLength(0)))
    }
    static func duration(_ seconds: UInt64) -> String {
        let formatter = DateComponentsFormatter()
        formatter.allowedUnits = seconds >= 3600 ? [.hour,.minute] : [.minute,.second]
        formatter.unitsStyle = .abbreviated
        formatter.maximumUnitCount = 2
        return formatter.string(from:TimeInterval(seconds)) ?? "—"
    }
    /// Accept plain digits or a correctly grouped regional integer. Never truncate fractions or trailing junk.
    static func wholeNumber(_ text: String, maximum: UInt32 = UInt32.max / 1000, locale: Locale = .current) -> UInt32? {
        let input = text.trimmingCharacters(in:.whitespacesAndNewlines)
        guard !input.isEmpty else { return nil }
        let formatter = NumberFormatter(); formatter.locale = locale; formatter.numberStyle = .decimal
        let separator = formatter.groupingSeparator ?? ","
        func normalize(_ value: String) -> String { value.replacingOccurrences(of:"\u{00a0}",with:" ").replacingOccurrences(of:"\u{202f}",with:" ") }
        let normalized = normalize(input), grouping = normalize(separator)
        let digits = normalized.replacingOccurrences(of:grouping,with:"")
        guard !digits.isEmpty, digits.allSatisfy({ $0.wholeNumberValue != nil }) else { return nil }
        let ascii = digits.compactMap(\.wholeNumberValue).map(String.init).joined()
        guard let value = UInt32(ascii), value <= maximum else { return nil }
        if normalized.contains(grouping) {
            // Some locales suppress grouping for four-digit output but still accept grouped input.
            let groups = normalized.components(separatedBy:grouping)
            let primary = formatter.groupingSize
            let secondary = formatter.secondaryGroupingSize == 0 ? primary : formatter.secondaryGroupingSize
            guard primary > 0, secondary > 0, groups.last?.count == primary,
                  let first = groups.first, (1...secondary).contains(first.count),
                  groups.dropFirst().dropLast().allSatisfy({ $0.count == secondary }) else { return nil }
        }
        return value
    }
}
