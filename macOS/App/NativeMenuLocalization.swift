import AppKit

/// SwiftUI refreshes app commands, but AppKit's standard menu titles keep their launch language.
@MainActor final class NativeMenuLocalization {
    private var observers: [NSObjectProtocol] = []
    private var scheduled = false
    private var updating = false
    private var trackingMenus: Set<ObjectIdentifier> = []
    private var keysByTitle: [String:String] = [:]
    // Match command identity before wording: system translations can differ from
    // our catalog. These names are inspected only; actions remain system-owned.
    private let keysByAction = [
        "performMiniaturize:":"Minimize", "miniaturizeAll:":"Minimize All",
        "performZoom:":"Zoom", "zoomAll:":"Zoom All",
        "_zoomFill:":"Fill", "_zoomCenter:":"Center",
        "_removeWindowFromStageManagerSet:":"Remove Window from Set",
        "arrangeInFront:":"Bring All to Front", "alternateArrangeInFront:":"Arrange in Front",
        "selectPreviousTab:":"Show Previous Tab", "selectNextTab:":"Show Next Tab",
        "moveTabToNewWindow:":"Move Tab to New Window", "mergeAllWindows:":"Merge All Windows",
        "orderFrontStandardAboutPanel:":"About %@", "hide:":"Hide %@",
        "hideOtherApplications:":"Hide Others", "unhideAllApplications:":"Show All",
        "terminate:":"Quit %@"
    ]
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
        for name in [NSMenu.didBeginTrackingNotification, NSMenu.didEndTrackingNotification] {
            observers.append(NotificationCenter.default.addObserver(forName:name,object:nil,queue:.main) { [weak self] notification in
                MainActor.assumeIsolated {
                    guard let self, let menu = notification.object as? NSMenu else { return }
                    if notification.name == NSMenu.didBeginTrackingNotification {
                        self.trackingMenus.insert(ObjectIdentifier(menu))
                        self.refresh()
                    } else {
                        self.trackingMenus.remove(ObjectIdentifier(menu))
                        self.scheduleRefresh()
                    }
                }
            })
        }
        for name in [AppLocalization.didChange, NSMenu.didAddItemNotification, NSMenu.didChangeItemNotification, NSApplication.didBecomeActiveNotification] {
            observers.append(NotificationCenter.default.addObserver(forName:name,object:nil,queue:.main) { [weak self] _ in
                MainActor.assumeIsolated {
                    guard let self else { return }
                    // Native delegates can recreate launch-language labels as a menu
                    // opens. Update leaf labels before display; never change its structure.
                    if self.trackingMenus.isEmpty { self.scheduleRefresh() }
                    else { self.refresh() }
                }
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
    private func key(for item: NSMenuItem) -> String? {
        if let action = item.action {
            if action == NSSelectorFromString("toggleFullScreen:") {
                let window = item.target as? NSWindow ?? NSApp.keyWindow
                return window?.styleMask.contains(.fullScreen) == true ? "Exit Full Screen" : "Enter Full Screen"
            }
            if let key = keysByAction[NSStringFromSelector(action)] { return key }
        }
        if let submenu = item.submenu {
            if submenu === NSApp.windowsMenu { return "Window" }
            if submenu === NSApp.helpMenu { return "Help" }
            if submenu === NSApp.servicesMenu { return "Services" }
            // Native tiling groups have no item identifiers on macOS 15.
            // Their child actions distinguish them without matching translated captions.
            let actions = Set(submenu.items.compactMap { $0.action.map(NSStringFromSelector) })
            if actions.contains("_zoomLeft:") && actions.contains("_zoomRight:") { return "Move & Resize" }
            if actions.contains("_tileLeft:") && actions.contains("_tileRight:") { return "Full Screen Tile" }
        }
        return keysByTitle[normalized(item.title)]
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
            for item in menu.items where !item.isSeparatorItem && !item.isHidden {
                // Window names and service-provider entries are content, not standard menu labels.
                if item.action == NSSelectorFromString("makeKeyAndOrderFront:") { continue }
                if let key = key(for:item) {
                    // SwiftUI owns its command labels and hidden shortcut mirrors.
                    // Leave those labels to its command updates.
                    // Settings is the standard application-menu command, not an app command.
                    if item.action == NSSelectorFromString("menuAction:"), key != "Settings…" { continue }
                    let localized = title(for:key)
                    if item.title != localized { item.title = localized }
                    // AppKit identifies standard menus by their internal titles when
                    // inserting full-screen, tab, and window-management commands.
                    // Localize the visible item only; preserve the submenu's title.
                }
                if let submenu = item.submenu, submenu !== NSApp.servicesMenu { update(submenu) }
            }
        }
        update(menu)
    }
}
