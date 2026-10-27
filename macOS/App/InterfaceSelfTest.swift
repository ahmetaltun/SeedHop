#if SEEDHOP_DEVELOPMENT
import SwiftUI
import AppKit

/// Exercises this process's fixture window and native menus, without system input.
@MainActor enum InterfaceSelfTest {
    static func run(store provider: @MainActor () -> LibraryStore?) async throws {
        try await settle()
        // macOS may restore a previous no-window state when this test is repeated.
        if !NSApp.windows.contains(where:{ $0.isVisible && $0.title == "SeedHop" }) {
            try await invoke("Show Library",key:"0")
            try await settle()
        }
        guard let window = NSApp.windows.first(where:{ $0.isVisible && $0.title == "SeedHop" }) else {
            throw UserFacingError(message:"No library fixture window")
        }
        window.subtitle = "Native test fixture · sample data"
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps:true)
        try await settle()
        print("System accessibility settings: fullKeyboardAccess=\(NSApp.isFullKeyboardAccessEnabled), reduceMotion=\(NSWorkspace.shared.accessibilityDisplayShouldReduceMotion), increaseContrast=\(NSWorkspace.shared.accessibilityDisplayShouldIncreaseContrast)")
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
        guard let store = provider(), let reopened = NSApp.windows.first(where: { $0.isVisible && $0.title == "SeedHop" }) else {
            throw UserFacingError(message:"Library fixture store unavailable")
        }
        reopened.makeKeyAndOrderFront(nil)
        try await invoke("Add Torrent…",key:"n"); try await settle()
        guard let sheet = reopened.attachedSheet,
              let editor = sheet.firstResponder as? NSTextView, editor.isFieldEditor,
              editor.delegate is NSTextField else {
            throw UserFacingError(message:"Import did not focus the magnet input: \(String(describing:reopened.attachedSheet?.firstResponder))")
        }
        print("PASS: Add Torrent opens a native sheet with its magnet input focused.")
        store.showImport = false; try await settle()
        guard reopened.attachedSheet == nil, reopened.firstResponder is NSTableView else {
            throw UserFacingError(message:"Closing import did not return focus to the transfer table: \(String(describing:reopened.firstResponder))")
        }
        print("PASS: Dismissing import returns focus to the native transfer table.")
        try await verifyControlledSheets(store:store,window:reopened)
        print("Native key-event delivery, VoiceOver speech, live-transfer reopening, and scrolling remain manual acceptance checks.")
    }

    private static func verifyControlledSheets(store: LibraryStore, window: NSWindow) async throws {
        window.subtitle = "Native test fixture · sample data"
        window.makeKeyAndOrderFront(nil); NSApp.activate(ignoringOtherApps:true)
        try await settle()
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("native-import-"+UUID().uuidString+".torrent")
        try Data([1,2,3]).write(to:file)
        defer { try? FileManager.default.removeItem(at:file) }
        store.beginImport(file:file)
        try await waitUntil("Metadata table did not appear; requests: \(UXSelfTest.nativeRequests.map(\.command))") {
            window.attachedSheet.map { descendants($0.contentView).contains { $0 is NSTableView } } ?? false
        }
        try await waitUntil("Metadata table did not receive native focus") { window.attachedSheet?.firstResponder is NSTableView }
        guard let sheet = window.attachedSheet else { throw UserFacingError(message:"Metadata fixture did not open an import sheet") }
        guard let focused = sheet.firstResponder as? NSTableView,
              descendants(sheet.contentView).contains(where:{ $0 === focused }) else {
            throw UserFacingError(message:"Metadata did not focus its native file table: \(String(describing:sheet.firstResponder))")
        }
        print("PASS: Resolved metadata moves native focus to file selection.")
        store.showImport = false; try await settle()
        guard let table = descendants(window.contentView).compactMap({ $0 as? NSTableView }).first else {
            throw UserFacingError(message:"Controlled transfer table unavailable")
        }
        window.makeFirstResponder(table)
        let item = store.torrents[0]
        store.requestRename(item,file:item.files[0]); try await settle()
        guard let rename = window.attachedSheet,
              let editor = rename.firstResponder as? NSTextView, editor.isFieldEditor,
              let input = editor.delegate as? NSTextField,
              input.stringValue == URL(fileURLWithPath:item.files[0].name).lastPathComponent else {
            throw UserFacingError(message:"Rename did not focus its captured file name")
        }
        guard let cancel = descendants(rename.contentView).compactMap({ $0 as? NSButton }).first(where:{ $0.title == "Cancel" }) else {
            throw UserFacingError(message:"Native rename Cancel button unavailable")
        }
        cancel.performClick(nil); try await settle()
        guard window.attachedSheet == nil, window.firstResponder === table else {
            throw UserFacingError(message:"Rename cancellation did not restore original native focus")
        }
        print("PASS: Rename focuses its captured name; native Cancel restores transfer-table focus.")
        var announcements: [String] = []
        var priorities: [NSAccessibilityPriorityLevel] = []
        NativeAccessibility.announcementObserver = { message, priority in announcements.append(message); priorities.append(priority) }
        defer { NativeAccessibility.announcementObserver = nil }
        window.makeKeyAndOrderFront(nil)
        store.error = "Restore access to the destination."; try await settle()
        store.error = "Restore access to the destination."; try await settle()
        guard announcements == ["Something needs attention. Restore access to the destination."] else {
            throw UserFacingError(message:"Native error announcement was missing or duplicated: \(announcements)")
        }
        store.error = nil; try await settle()
        store.error = "Restore access to the destination."; try await settle()
        guard announcements.count == 2 else { throw UserFacingError(message:"Repeated error after clearing was not announced") }
        guard priorities == [.high,.high] else { throw UserFacingError(message:"Errors did not use high-priority announcements") }
        print("PASS: Native notice requests one high-priority announcement per error occurrence; speech output remains manual.")
        store.error = nil; try await settle()
        store.fileOperation = FileOperationItem(id:"native-move",torrentId:item.id,kind:"relocate",phase:"copying",completedBytes:1,totalBytes:100,cancelRequested:false)
        try await settle()
        let copyingCount = announcements.count
        store.fileOperation?.completedBytes = 50; try await settle()
        guard copyingCount == 3, announcements.count == copyingCount,
              announcements.last == "File change. Moving files…", priorities.last == .medium else {
            throw UserFacingError(message:"File copying did not announce its phase once or announced byte updates")
        }
        store.fileOperation?.cancelRequested = true; try await settle()
        guard announcements.count == 4, announcements.last == "File change. Cancelling…", priorities.last == .medium else {
            throw UserFacingError(message:"Cancellation phase was not announced")
        }
        store.fileOperation = nil
        store.storageRecovery = StorageRecoveryItem(id:"native-recovery",torrentId:item.id,activeLocation:"original",message:"Inspect retained copies.",originalDirectory:"/tmp/original",destinationDirectory:"/tmp/copies",canKeepCopies:true)
        try await settle()
        guard announcements.count == 5, announcements.last == "File cleanup needs attention. Inspect retained copies.", priorities.last == .high else {
            throw UserFacingError(message:"Recovery guidance was not announced")
        }
        print("PASS: File phases announce at medium priority without byte chatter; recovery guidance requests high priority.")
    }
    private static func waitUntil(_ failure: @autoclosure () -> String, _ condition: () -> Bool) async throws {
        let deadline = Date(timeIntervalSinceNow:5)
        while !condition() {
            guard Date() < deadline else { throw UserFacingError(message:failure()) }
            try await Task.sleep(for:.milliseconds(50))
        }
    }

    private static func descendants(_ view: NSView?) -> [NSView] {
        guard let view else { return [] }
        return [view] + view.subviews.flatMap { descendants($0) }
    }

    static func invoke(_ title: String, key: String) async throws {
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
