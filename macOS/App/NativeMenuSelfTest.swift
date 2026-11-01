#if SEEDHOP_DEVELOPMENT
import AppKit

/// Uses AppKit's tracking loop so native window commands receive their real lifecycle.
@MainActor enum NativeMenuSelfTest {
    private final class TrackingCancellation: NSObject {
        let menu: NSMenu
        let selection: String?
        var visibleTitles: [String] = []
        init(_ menu: NSMenu, selection: String?) { self.menu = menu; self.selection = selection }
        @objc func cancel(_ timer: Timer) {
            visibleTitles = menu.items.filter { !$0.isHidden }.map(\.title)
            if let selection, let index = menu.items.firstIndex(where:{ !$0.isHidden && $0.isEnabled && $0.title == selection }) {
                menu.performActionForItem(at:index)
            }
            menu.cancelTrackingWithoutAnimation()
        }
    }

    private static func settle() async throws { try await Task.sleep(for:.milliseconds(350)) }
    private static func expect(_ condition: Bool, _ message: String) throws {
        if !condition { throw UserFacingError(message:message) }
    }
    private static func menu(_ key: String) throws -> NSMenu {
        let title = NativeMenuLocalization().title(for:key)
        guard let menu = NSApp.mainMenu?.items.first(where:{ $0.title == title })?.submenu else {
            throw UserFacingError(message:"Missing standard menu: \(key)")
        }
        return menu
    }
    @discardableResult private static func track(_ menu: NSMenu, in window: NSWindow, selecting selection: String? = nil) -> [String] {
        let cancellation = TrackingCancellation(menu,selection:selection)
        let timer = Timer(timeInterval:0.15,target:cancellation,selector:#selector(TrackingCancellation.cancel(_:)),userInfo:nil,repeats:false)
        RunLoop.main.add(timer,forMode:.eventTracking)
        defer { timer.invalidate() }
        menu.popUp(positioning:nil,at:NSPoint(x:20,y:20),in:window.contentView)
        return cancellation.visibleTitles
    }
    private static func actions(_ menu: NSMenu) -> Set<String> {
        Set(menu.items.filter { !$0.isHidden && !$0.isSeparatorItem }.compactMap { $0.action.map(NSStringFromSelector) }
            .filter { !["menuAction:","submenuAction:","makeKeyAndOrderFront:"].contains($0) })
    }
    private static func focus(_ window: NSWindow) async throws {
        NSApp.activate(ignoringOtherApps:true)
        window.makeKeyAndOrderFront(nil)
        for _ in 0..<20 {
            if NSApp.isActive && NSApp.keyWindow === window { return }
            try await Task.sleep(for:.milliseconds(100))
        }
        throw UserFacingError(message:"Menu fixture could not focus \(window.title)")
    }
    private static func assertViewCommands(_ menu: NSMenu, enabled: Bool) throws {
        for key in ["s","i"] {
            let modifiers: NSEvent.ModifierFlags = key == "s" ? [.command,.control] : [.command,.option]
            guard let item = menu.items.first(where:{ !$0.isHidden && $0.keyEquivalent == key && $0.keyEquivalentModifierMask == modifiers }) else {
                throw UserFacingError(message:"View lost shortcut: \(key), language=\(AppLocalization.shared.language), items=\(menu.items.map(\.title))")
            }
            try expect(item.isEnabled == enabled,"View command has incorrect availability: \(key)")
        }
    }
    static func run() async throws {
        let originalLanguage = AppLocalization.shared.language
        defer { AppLocalization.shared.use(originalLanguage) }
        try await settle()
        try await InterfaceSelfTest.invoke("Show Library",key:"0")
        try await settle()
        guard let library = NSApp.windows.first(where:{ $0.isVisible && $0.title == "SeedHop" }) else {
            throw UserFacingError(message:"No library menu fixture")
        }
        // Discard secondary windows restored by a previous fixture run.
        for window in NSApp.windows where window !== library && window.isVisible { window.close() }
        try await focus(library)
        for _ in 0..<2 { try await InterfaceSelfTest.invoke("Show Library",key:"0") }
        try expect(NSApp.windows.filter { $0.isVisible && $0.title == "SeedHop" }.count == 1,"Show Library duplicated the library window")

        let initialView = try menu("View"), initialWindow = try menu("Window")
        let internalViewTitle = initialView.title, internalWindowTitle = initialWindow.title
        track(initialView,in:library); track(initialWindow,in:library)
        let nativeWindowActions = actions(initialWindow)
        try expect(nativeWindowActions.contains("performZoom:"),"Fixture has no native window commands")
        for language in Bundle.main.localizations.sorted() where language != "en-XA" {
            AppLocalization.shared.use(language)
            try await settle()
            try await focus(library)
            for _ in 0..<2 {
                let view = try menu("View"), windows = try menu("Window")
                try expect(view.title == internalViewTitle && windows.title == internalWindowTitle,"Localization changed internal menu titles: \(language)")
                let viewTitles = track(view,in:library), windowTitles = track(windows,in:library)
                let labels = NativeMenuLocalization()
                try expect(viewTitles.contains(labels.title(for:"Enter Full Screen")),"View label stayed in launch language while tracking: \(language)")
                try expect(windowTitles.contains(labels.title(for:"Minimize")),"Window label stayed in launch language while tracking: \(language), titles=\(windowTitles)")
                for key in ["Move & Resize","Full Screen Tile"] {
                    let markers = key == "Move & Resize" ? ["_zoomLeft:","_zoomRight:"] : ["_tileLeft:","_tileRight:"]
                    let present = windows.items.contains { item in
                        let actions = Set(item.submenu?.items.compactMap { $0.action.map(NSStringFromSelector) } ?? [])
                        return Set(markers).isSubset(of:actions)
                    }
                    if present { try expect(windowTitles.contains(labels.title(for:key)),"Native window group stayed in launch language: \(key), \(language)") }
                }
                try expect(actions(view).contains("toggleFullScreen:"),"View lost native full-screen command: \(language)")
                try expect(nativeWindowActions.isSubset(of:actions(windows)),"Window lost native commands: \(language)")
                try assertViewCommands(view,enabled:true)
            }
        }
        AppLocalization.shared.use("en")
        try await settle()
        let sidebarBefore = WorkspacePreferences.defaults.object(forKey:"workspaceSidebarVisible") as? Bool ?? true
        guard let event = NSEvent.keyEvent(with:.keyDown,location:.zero,modifierFlags:[.command,.control],timestamp:0,windowNumber:library.windowNumber,context:nil,characters:"s",charactersIgnoringModifiers:"s",isARepeat:false,keyCode:1) else {
            throw UserFacingError(message:"Could not create Sidebar shortcut event")
        }
        try expect(NSApp.mainMenu?.performKeyEquivalent(with:event) == true,"Sidebar shortcut was not handled")
        try await settle()
        try expect(WorkspacePreferences.defaults.bool(forKey:"workspaceSidebarVisible") != sidebarBefore,"Sidebar shortcut did not toggle the custom sidebar")
        try expect(NSApp.mainMenu?.performKeyEquivalent(with:event) == true,"Sidebar shortcut could not restore the sidebar")
        try await settle()
        try expect(WorkspacePreferences.defaults.bool(forKey:"workspaceSidebarVisible") == sidebarBefore,"Sidebar shortcut did not restore visibility")
        let windows = try menu("Window")
        let bandwidthTitles = track(windows,in:library,selecting:"Bandwidth Limits")
        try expect(bandwidthTitles.contains("Bandwidth Limits"),"Missing Bandwidth Limits command")
        try await settle()
        guard let bandwidthWindow = NSApp.windows.first(where:{ $0.isVisible && $0 !== library }) else {
            throw UserFacingError(message:"Bandwidth Limits did not open: \(NSApp.windows.map(\.title))")
        }
        try await focus(bandwidthWindow)
        try await settle()
        let bandwidthView = try menu("View")
        track(bandwidthView,in:bandwidthWindow)
        try assertViewCommands(bandwidthView,enabled:false)
        bandwidthWindow.close()
        guard let appMenu = NSApp.mainMenu?.items.first?.submenu else {
            throw UserFacingError(message:"No application menu")
        }
        let appTitles = track(appMenu,in:library,selecting:NativeMenuLocalization().title(for:"Settings…"))
        try expect(appTitles.contains(NativeMenuLocalization().title(for:"Settings…")),"Settings stayed in launch language while tracking")
        try await settle()
        guard let settings = NSApp.windows.first(where:{ $0.isVisible && $0 !== library }) else {
            throw UserFacingError(message:"Settings did not open")
        }
        try await focus(settings)
        try await settle()
        let settingsView = try menu("View")
        track(settingsView,in:settings)
        try assertViewCommands(settingsView,enabled:false)
        settings.close()
        try await focus(library)
        try await settle()
        let restoredView = try menu("View"), restoredWindows = try menu("Window")
        track(restoredView,in:library); track(restoredWindows,in:library)
        try assertViewCommands(restoredView,enabled:true)
        try expect(actions(restoredView).contains("toggleFullScreen:") && nativeWindowActions.isSubset(of:actions(restoredWindows)),"Native commands did not recover with library focus")
        library.close()
        try await settle()
        try await InterfaceSelfTest.invoke("Show Library",key:"0")
        try await settle()
        try expect(NSApp.windows.filter { $0.isVisible && $0.title == "SeedHop" }.count == 1,"Show Library did not reopen one library window")
        print("PASS: real View/Window tracking across all bundled languages preserves native commands and internal titles; view commands follow focus; Show Library reveals and reopens one window.")
    }
}
#endif
