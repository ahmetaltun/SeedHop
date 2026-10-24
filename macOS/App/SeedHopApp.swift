import SwiftUI

@main struct SeedHopApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @StateObject private var store = LibraryStore(designPreview:ProcessInfo.processInfo.arguments.contains("--design-preview") || ProcessInfo.processInfo.arguments.contains("--render-designs") || ProcessInfo.processInfo.arguments.contains("--verify-engine"))
    var body: some Scene {
        WindowGroup("SeedHop",id:"library") {
            LibraryView().environmentObject(store).onAppear { delegate.store = store }
                .task { await store.start() }
                .onOpenURL { store.open([$0]) }
        }
        .defaultSize(width:1200,height:760)
        .windowToolbarStyle(.unified)
        .commands {
            CommandGroup(replacing:.newItem) {
                Button("Add Torrent…") { store.incomingFile = nil; store.incomingMagnet = ""; store.showImport = true }.keyboardShortcut("n")
                Button("Open Torrent File…") { store.openFiles() }.keyboardShortcut("o")
                Button("Paste Magnet Link…") { store.pasteMagnet() }.keyboardShortcut("v",modifiers:[.command,.shift])
            }
            CommandMenu("Transfers") {
                Button("Resume Selected") { store.toggleSelected(paused:false) }.keyboardShortcut("r").disabled(store.selection.isEmpty)
                Button("Pause Selected") { store.toggleSelected(paused:true) }.keyboardShortcut(".").disabled(store.selection.isEmpty)
                Divider()
                Button("Remove Selected…") { store.showRemoval = true }.keyboardShortcut(.delete).disabled(store.selection.isEmpty)
                Button("Show Inspector") { store.showInspector.toggle() }.keyboardShortcut("i",modifiers:[.command,.option])
            }
            CommandGroup(before:.appTermination) { Button("Quit and Stop Transfers") { delegate.stopEverything = true; NSApp.terminate(nil) }.keyboardShortcut("q",modifiers:[.command,.option]) }
        }
        Settings { SettingsView().environmentObject(store) }
    }
}

@MainActor final class AppDelegate: NSObject, NSApplicationDelegate {
    weak var store: LibraryStore?
    var stopEverything = false
    private var quitting = false
    func applicationShouldTerminateAfterLastWindowClosed(_ sender:NSApplication) -> Bool { false }
    func applicationShouldTerminate(_ sender:NSApplication) -> NSApplication.TerminateReply {
        guard let store, !quitting else { return quitting ? .terminateCancel : .terminateNow }
        quitting = true
        Task { let ready = await store.prepareToQuit(stopEverything:stopEverything); quitting = false; stopEverything = false; sender.reply(toApplicationShouldTerminate:ready) }
        return .terminateLater
    }
    func applicationDidFinishLaunching(_ notification:Notification) {
        #if SEEDHOP_DEVELOPMENT
        if ProcessInfo.processInfo.arguments.contains("--verify-engine") {
            Task { do { try await EngineSelfTest.run(); exit(0) } catch { fputs("XPC verification failed: \(error.localizedDescription)\n",stderr); exit(1) } }
        }
        #endif
        if ProcessInfo.processInfo.arguments.contains("--render-designs") {
            DispatchQueue.main.asyncAfter(deadline:.now() + 0.7) {
                do { try DesignRenderer.render(); exit(0) } catch { fputs("Render failed: \(error)\n",stderr); exit(1) }
            }
        }
    }
    func application(_ application:NSApplication,open urls:[URL]) { store?.open(urls) }
}
