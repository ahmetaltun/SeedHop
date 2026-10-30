import AppKit
import Observation

enum AppearancePreference: String, CaseIterable {
    case system, light, dark

    var appearance: NSAppearance? {
        switch self {
        case .system: return nil
        case .light: return NSAppearance(named:.aqua)
        case .dark: return NSAppearance(named:.darkAqua)
        }
    }
}

/// One application override keeps SwiftUI content and native windows in sync.
@MainActor @Observable final class AppAppearance {
    static let shared = AppAppearance(defaults:WorkspacePreferences.defaults)
    static let preferenceKey = "appearancePreference"
    private let defaults: UserDefaults
    private(set) var selection: AppearancePreference

    init(defaults: UserDefaults) {
        self.defaults = defaults
        selection = AppearancePreference(rawValue:defaults.string(forKey:Self.preferenceKey) ?? "") ?? .dark
    }

    func select(_ preference: AppearancePreference) {
        defaults.set(preference.rawValue,forKey:Self.preferenceKey)
        selection = preference
        apply()
    }

    func apply() {
        // nil restores native inheritance, including automatic macOS changes.
        NSApp.appearance = selection.appearance
    }
}
