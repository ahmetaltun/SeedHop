import SwiftUI

@main struct SeedHopApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @StateObject private var store = LibraryStore(designPreview:ProcessInfo.processInfo.arguments.contains("--design-preview") || ProcessInfo.processInfo.arguments.contains("--render-designs") || ProcessInfo.processInfo.arguments.contains("--verify-engine") || ProcessInfo.processInfo.arguments.contains("--verify-ux") || ProcessInfo.processInfo.arguments.contains("--verify-interface") || ProcessInfo.processInfo.arguments.contains("--verify-lifecycle"))
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
                Button("Add Torrent…") { store.beginImport() }.keyboardShortcut("n")
                Button("Open Torrent File…") { store.openFiles() }.keyboardShortcut("o")
                Button("Paste Magnet Link…") { store.pasteMagnet() }.keyboardShortcut("v",modifiers:[.command,.shift])
            }
            LibraryNavigationCommands()
            CommandMenu("Transfers") {
                Button("Resume Selected") { store.toggleSelected(paused:false) }.keyboardShortcut("r").disabled(!store.canResumeSelection)
                Button("Pause Selected") { store.toggleSelected(paused:true) }.keyboardShortcut(".").disabled(!store.canPauseSelection)
                Divider()
                Button("Force Recheck") { store.recheckSelected() }.disabled(!store.canRecheckSelection)
                Button("Move to Front of Queue") { store.moveSelectedToFront() }.disabled(!store.canActOnSelection || store.selected.count != 1)
                Button("Move Files…") { store.chooseRelocation() }.disabled(!store.canRecheckSelection || store.selected.count != 1)
                Button("Remove Selected…") { store.requestRemoval() }.keyboardShortcut(.delete).disabled(!store.canActOnSelection)
                Button("Show Inspector") { store.showInspector.toggle() }.keyboardShortcut("i",modifiers:[.command,.option])
            }
            CommandGroup(before:.appTermination) { Button("Quit and Stop Transfers") { delegate.stopEverything = true; NSApp.terminate(nil) }.keyboardShortcut("q",modifiers:[.command,.option]) }
        }
        Settings { SettingsView().environmentObject(store) }
    }
}

struct LibraryNavigationCommands: Commands {
    @FocusedValue(\.librarySearch) private var librarySearch
    @Environment(\.openWindow) private var openWindow
    var body: some Commands {
        CommandGroup(after:.textEditing) {
            Button("Find Transfer…") { librarySearch?.focus() }.keyboardShortcut("f").disabled(librarySearch == nil)
            Button("Focus Transfer List") { librarySearch?.focusTransfers() }.keyboardShortcut("l").disabled(librarySearch == nil)
        }
        CommandGroup(after:.windowArrangement) {
            Button("Show Library") { openWindow(id:"library") }.keyboardShortcut("0")
        }
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
        #if SEEDHOP_VALIDATION
        if ProcessInfo.processInfo.arguments.contains("--verify-lifecycle") {
            Task { do { try await LifecycleSelfTest.run(); exit(0) } catch { fputs("Lifecycle verification failed: \(error.localizedDescription)\n",stderr); exit(1) } }
            return
        }
        #endif
        #if SEEDHOP_DEVELOPMENT
        if ProcessInfo.processInfo.arguments.contains("--verify-interface") {
            Task { do { try await InterfaceSelfTest.run(); exit(0) } catch { fputs("Interface verification failed: \(error.localizedDescription)\n",stderr); exit(1) } }
            return
        }
        if ProcessInfo.processInfo.arguments.contains("--verify-ux") {
            Task { do { try await UXSelfTest.run(); exit(0) } catch { fputs("UX verification failed: \(error.localizedDescription)\n",stderr); exit(1) } }
            return
        }
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
