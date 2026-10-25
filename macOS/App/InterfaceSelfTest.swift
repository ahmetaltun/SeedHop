#if SEEDHOP_DEVELOPMENT
import SwiftUI
import AppKit

/// Exercises this process's fixture window and native menus, without system input.
@MainActor enum InterfaceSelfTest {
    static func run() async throws {
        try await settle()
        // macOS may restore a previous no-window state when this test is repeated.
        if !NSApp.windows.contains(where:{ $0.isVisible && $0.title == "SeedHop" }) {
            try await invoke("Show Library",key:"0")
            try await settle()
        }
        guard let window = NSApp.windows.first(where:{ $0.isVisible && $0.title == "SeedHop" }) else {
            throw UserFacingError(message:"No library fixture window")
        }
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps:true)
        try await settle()
        print("Native fixture state: active=\(NSApp.isActive), key=\(window.isKeyWindow), main=\(window.isMainWindow), responder=\(String(describing:window.firstResponder))")
        try await invoke("Find Transfer…",key:"f")
        try await settle()
        guard let editor = window.firstResponder as? NSTextView, editor.isFieldEditor,
              let field = editor.delegate as? NSTextField,
              field is NSSearchField else {
            throw UserFacingError(message:"Find did not focus the search field: \(String(describing:window.firstResponder))")
        }
        print("PASS: Command-F menu action focuses the native search field.")
        try await invoke("Focus Transfer List",key:"l")
        try await settle()
        guard window.firstResponder is NSTableView else {
            throw UserFacingError(message:"Focus Transfer List did not focus the native table: \(String(describing:window.firstResponder))")
        }
        print("PASS: Command-L menu action focuses the native transfer table.")
        let args = ProcessInfo.processInfo.arguments
        if let index = args.firstIndex(of:"--output"), args.indices.contains(index+1) {
            try capture(window,to:URL(fileURLWithPath:args[index+1]))
        }
        window.close()
        try await settle()
        try await invoke("Show Library",key:"0")
        try await settle()
        guard NSApp.windows.contains(where:{ $0.isVisible && $0.title == "SeedHop" }) else {
            throw UserFacingError(message:"Show Library did not reopen the closed library")
        }
        print("PASS: Command-0 menu action reopens the library after the last window closes.")
        print("Native key-event delivery, VoiceOver, live-transfer reopening, and scrolling remain manual acceptance checks.")
    }

    private static func invoke(_ title: String, key: String) async throws {
        func find(in menu: NSMenu) -> (NSMenu, Int)? {
            menu.delegate?.menuNeedsUpdate?(menu)
            menu.update()
            for (index,item) in menu.items.enumerated() {
                if item.title == title { return (menu,index) }
                if let submenu = item.submenu, let found = find(in:submenu) { return found }
            }
            return nil
        }
        guard let root = NSApp.mainMenu, let (menu,index) = find(in:root) else {
            throw UserFacingError(message:"Missing native menu action: \(title)")
        }
        menu.delegate?.menuWillOpen?(menu)
        defer { menu.delegate?.menuDidClose?(menu) }
        try await settle()
        menu.update()
        let item = menu.items[index]
        guard item.isEnabled, item.keyEquivalent == key, item.keyEquivalentModifierMask == .command else {
            throw UserFacingError(message:"Menu action disabled or shortcut incorrect: \(title), enabled=\(item.isEnabled), key=\(item.keyEquivalent), modifiers=\(item.keyEquivalentModifierMask.rawValue)")
        }
        menu.performActionForItem(at:index)
    }
    private static func settle() async throws { try await Task.sleep(for:.milliseconds(400)) }

    private static func capture(_ window: NSWindow, to url: URL) throws {
        guard let view = window.contentView?.superview,
              let bitmap = view.bitmapImageRepForCachingDisplay(in:view.bounds) else {
            throw UserFacingError(message:"Could not render the native navigation fixture")
        }
        view.cacheDisplay(in:view.bounds,to:bitmap)
        guard let png = bitmap.representation(using:.png,properties:[:]) else {
            throw UserFacingError(message:"Could not encode the native navigation fixture")
        }
        try png.write(to:url)
    }
}
#endif
