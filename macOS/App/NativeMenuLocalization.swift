import AppKit

/// SwiftUI refreshes app commands, but AppKit's standard menu titles keep their launch language.
@MainActor final class NativeMenuLocalization {
    private var observers: [NSObjectProtocol] = []
    private var scheduled = false
    private var updating = false
    private var keysByTitle: [String:String] = [:]
    private let applicationName = Bundle.main.object(forInfoDictionaryKey:"CFBundleName") as? String ?? "SeedHop"

    func install() {
        guard observers.isEmpty else { return }
        let table = AppLocalization.shared.bundle.path(forResource:"NativeMenus",ofType:"strings")
        // The catalog is compiled to a native string table; no system framework resources are read at runtime.
        let source = table.flatMap { path -> [String:String]? in
            guard let data = try? Data(contentsOf:URL(fileURLWithPath:path)) else { return nil }
            return (try? PropertyListSerialization.propertyList(from:data,options:[],format:nil)) as? [String:String]
        } ?? [:]
        let keys = source.keys.sorted()
        for language in Bundle.main.localizations {
            guard let url = Bundle.main.url(forResource:language,withExtension:"lproj"), let bundle = Bundle(url:url) else { continue }
            for key in keys {
                let title = formatted(bundle.localizedString(forKey:key,value:key,table:"NativeMenus"))
                keysByTitle[normalized(title)] = key
            }
        }
        for name in [AppLocalization.didChange, NSMenu.didAddItemNotification, NSMenu.didChangeItemNotification, NSMenu.didBeginTrackingNotification, NSApplication.didBecomeActiveNotification] {
            observers.append(NotificationCenter.default.addObserver(forName:name,object:nil,queue:.main) { [weak self] _ in
                MainActor.assumeIsolated { self?.scheduleRefresh() }
            })
        }
        scheduleRefresh()
    }

    private func normalized(_ title: String) -> String { title.replacingOccurrences(of:"...",with:"…") }
    private func formatted(_ value: String) -> String {
        value.contains("%@") || value.contains("%1$@") ? String(format:value,locale:AppLocalization.shared.locale,arguments:[applicationName]) : value
    }
    func title(for key: String) -> String {
        formatted(AppLocalization.shared.bundle.localizedString(forKey:key,value:key,table:"NativeMenus"))
    }
    private func scheduleRefresh() {
        guard !updating, !scheduled else { return }
        scheduled = true
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.scheduled = false
            self.refresh()
        }
    }
    func refresh() {
        guard !updating, let menu = NSApp.mainMenu else { return }
        updating = true; defer { updating = false }
        func update(_ menu: NSMenu) {
            for item in menu.items where !item.isSeparatorItem {
                // Window names and service-provider entries are content, not standard menu labels.
                if item.target is NSWindow || item.action == NSSelectorFromString("makeKeyAndOrderFront:") { continue }
                if let key = keysByTitle[normalized(item.title)] {
                    let localized = title(for:key)
                    if item.title != localized { item.title = localized }
                    if let submenu = item.submenu, submenu.title != localized { submenu.title = localized }
                }
                if let submenu = item.submenu, submenu !== NSApp.servicesMenu { update(submenu) }
            }
        }
        update(menu)
    }
}
