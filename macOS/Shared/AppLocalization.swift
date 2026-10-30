import Foundation
import Observation

/// Observable resources let existing views update without losing their identity or drafts.
@Observable final class AppLocalization {
    static let shared = AppLocalization()
    static let didChange = Notification.Name("app.seedhop.languageDidChange")
    private(set) var language: String
    private(set) var bundle: Bundle
    private(set) var locale: Locale

    private init() {
        let language = Bundle.main.preferredLocalizations.first ?? "en"
        self.language = language
        bundle = Self.resources(language) ?? .main
        locale = Self.displayLocale(language)
    }
    func use(_ language: String) {
        guard language != self.language, let resources = Self.resources(language) else { return }
        self.language = language
        bundle = resources
        locale = Self.displayLocale(language)
        NotificationCenter.default.post(name:Self.didChange,object:self)
    }
    private static func resources(_ language: String) -> Bundle? {
        Bundle.main.url(forResource:language,withExtension:"lproj").flatMap { Bundle(url:$0) }
    }
    private static func displayLocale(_ language: String) -> Locale {
        // Plural rules follow the selected language; regional number/time preferences stay regional.
        var components = Locale.Components(identifier:Locale.current.identifier)
        var selected = Locale.Language.Components(identifier:language)
        selected.region = components.languageComponents.region
        components.languageComponents = selected
        return Locale(components:components)
    }
}
