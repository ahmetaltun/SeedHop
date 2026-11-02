import SwiftUI
import Combine

@main struct SeedHopApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @StateObject private var store = SeedHopApp.makeStore()
    @State private var localization = AppLocalization.shared
    @MainActor private static func makeStore() -> LibraryStore {
        #if SEEDHOP_DEVELOPMENT
        if ProcessInfo.processInfo.arguments.contains("--verify-desktop-integration") || ProcessInfo.processInfo.arguments.contains("--verify-localization") || ProcessInfo.processInfo.arguments.contains("--verify-live-language") || ProcessInfo.processInfo.arguments.contains("--verify-menus") || ProcessInfo.processInfo.arguments.contains("--render-localizations") { return LibraryStore(designPreview:true) }
        if ProcessInfo.processInfo.arguments.contains("--verify-performance") { return PerformanceSelfTest.fixtureStore() }
        if ProcessInfo.processInfo.arguments.contains("--verify-interface") { return UXSelfTest.nativeFixture() }
        #endif
        return LibraryStore(designPreview:ProcessInfo.processInfo.arguments.contains("--verify-workspace-layout") || ProcessInfo.processInfo.arguments.contains("--verify-performance") || ProcessInfo.processInfo.arguments.contains("--design-preview") || ProcessInfo.processInfo.arguments.contains("--render-designs") || ProcessInfo.processInfo.arguments.contains("--verify-engine") || ProcessInfo.processInfo.arguments.contains("--verify-ux") || ProcessInfo.processInfo.arguments.contains("--verify-interface") || ProcessInfo.processInfo.arguments.contains("--verify-lifecycle") || ProcessInfo.processInfo.arguments.contains("--verify-storage-recovery"))
    }
    var body: some Scene {
        WindowGroup("SeedHop",id:"library",for:String.self) { _ in
            LibraryView().environmentObject(store)
                .modifier(ExternalOpenBridge(coordinator:delegate.externalOpens,store:store,attached:{ delegate.store = store }))
                .task { await store.start() }
        } defaultValue: { "library" }
        .defaultSize(width:1200,height:760)
        .windowToolbarStyle(.unified)
        .environment(\.locale,localization.locale)
        .commands {
            CommandGroup(replacing:.newItem) {
                Button(String(localized:"Add Torrent…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { store.beginImport() }.keyboardShortcut("n").disabled(store.hasFileOperation)
                Button(String(localized:"Open Torrent File…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { store.openFiles() }.keyboardShortcut("o").disabled(store.hasFileOperation)
                Button(String(localized:"Paste Magnet Link…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { store.pasteMagnet() }.keyboardShortcut("v",modifiers:[.command,.shift]).disabled(store.hasFileOperation)
            }
            SoftwareUpdateCommands()
            LibraryNavigationCommands(store:store)
            CommandMenu(String(localized:"Transfers",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) {
                Button(store.resumeSelectedLabel) { store.toggleSelected(paused:false) }.keyboardShortcut("r").disabled(!store.canResumeSelection)
                Button(String(localized:"Pause Selected",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { store.toggleSelected(paused:true) }.keyboardShortcut(".").disabled(!store.canPauseSelection)
                Toggle(String(localized:"Force Start",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),isOn:Binding(get:{ store.selectionForceStarted },set:{ store.setForceStarted(ids:store.selection,enabled:$0) }))
                    .disabled(!store.canSetForceStarted(ids:store.selection,enabled:!store.selectionForceStarted))
                Divider()
                Button(String(localized:"Force Reannounce",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { store.reannounceSelected() }
                    .disabled(!store.canReannounceSelection)
                    .help(String(localized:"Requests tracker updates when tracker limits allow.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                Button(String(localized:"Force Recheck",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { store.recheckSelected() }.disabled(!store.canRecheckSelection)
                Button(String(localized:"Move to Front of Queue",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { store.moveSelectedToFront() }.disabled(!store.canActOnSelection || store.selected.count != 1)
                Button(String(localized:"Move Files…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { store.chooseRelocation() }.disabled(!store.canMoveSelection)
                Button(String(localized:"Remove Selected…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { store.requestRemoval() }.keyboardShortcut(.delete).disabled(!store.canRemoveSelection)
            }
            CommandGroup(before:.appTermination) { Button(String(localized:"Quit and Stop Transfers",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { delegate.stopEverything = true; NSApp.terminate(nil) }.keyboardShortcut("q",modifiers:[.command,.option]) }
        }
        Window(String(localized:"Bandwidth Limits",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),id:"bandwidth") {
            ScrollView { BandwidthSettingsView().padding(24) }
                .frame(width:500,height:610).background(Design.canvas).tint(Design.green)
                .environmentObject(store)
        }.windowResizability(.contentSize).environment(\.locale,localization.locale)
        Settings { SettingsView().environmentObject(store) }.environment(\.locale,localization.locale)
    }
}

struct LibraryNavigationCommands: Commands {
    @ObservedObject var store: LibraryStore
    @FocusedValue(\.librarySearch) private var librarySearch
    @Environment(\.openWindow) private var openWindow
    var body: some Commands {
        CommandGroup(after:.textEditing) {
            Button(String(localized:"Find Transfer…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { librarySearch?.focus() }.keyboardShortcut("f").disabled(librarySearch == nil)
            Button(String(localized:"Focus Transfer List",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { librarySearch?.focusTransfers() }.keyboardShortcut("l").disabled(librarySearch == nil)
        }
        CommandGroup(replacing:.sidebar) {
            // AppKit reserves Option-Command-S for its hidden toggleSidebar: command.
            Button(String(localized:"Toggle Sidebar",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { librarySearch?.toggleSidebar?() }.keyboardShortcut("s",modifiers:[.command,.control]).disabled(librarySearch?.toggleSidebar == nil)
        }
        CommandGroup(after:.sidebar) {
            Button(String(localized:"Show Details",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { store.showInspector.toggle() }.keyboardShortcut("i",modifiers:[.command,.option]).disabled(librarySearch == nil)
        }
        CommandGroup(after:.windowArrangement) {
            // A matching value reveals the existing group window and preserves URL handling.
            Button(String(localized:"Show Library",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { openWindow(id:"library",value:"library") }.keyboardShortcut("0")
        }
    }
}

@MainActor final class AppDelegate: NSObject, NSApplicationDelegate {
    private let menuLocalization = NativeMenuLocalization()
    let externalOpens = ExternalOpenCoordinator()
    private let dockSpeeds = DockSpeedController()
    private var dockObservations = Set<AnyCancellable>()
    weak var store: LibraryStore? {
        didSet { if oldValue !== store { observeDock() } }
    }
    private func observeDock() {
        dockObservations.removeAll()
        guard let store, !store.designPreview, !WorkspacePreferences.isFixture else { return }
        store.objectWillChange.sink { [weak self] in
            Task { @MainActor [weak self] in self?.updateDock() }
        }.store(in:&dockObservations)
        store.$showDockSpeeds.dropFirst().sink { [weak store] enabled in
            if enabled { Task { @MainActor in await store?.refresh() } }
        }.store(in:&dockObservations)
        NotificationCenter.default.publisher(for:AppLocalization.didChange).sink { [weak self] _ in
            Task { @MainActor [weak self] in self?.updateDock() }
        }.store(in:&dockObservations)
        NSWorkspace.shared.notificationCenter.publisher(for:NSWorkspace.willSleepNotification).sink { [weak self] _ in
            Task { @MainActor [weak self] in self?.dockSpeeds.reset() }
        }.store(in:&dockObservations)
        NSWorkspace.shared.notificationCenter.publisher(for:NSWorkspace.didWakeNotification).sink { [weak self] _ in
            Task { @MainActor [weak self] in
                guard let self, let store = self.store, store.showDockSpeeds else { return }
                await store.refresh()
                self.updateDock()
            }
        }.store(in:&dockObservations)
        updateDock()
    }
    private func updateDock() {
        guard let store else { dockSpeeds.reset(); return }
        dockSpeeds.update(enabled:store.showDockSpeeds,connected:store.connected && !store.switching,
                          download:store.downloadRate,upload:store.uploadRate)
    }
    func applicationWillTerminate(_ notification: Notification) { dockSpeeds.reset() }

    var stopEverything = false
    private var quitting = false
    func applicationWillFinishLaunching(_ notification:Notification) {
        AppAppearance.shared.apply()
    }
    func applicationShouldTerminateAfterLastWindowClosed(_ sender:NSApplication) -> Bool { false }
    func applicationShouldTerminate(_ sender:NSApplication) -> NSApplication.TerminateReply {
        guard let store else { return SoftwareUpdate.shared.installing ? .terminateCancel : .terminateNow }
        guard !quitting else { return .terminateCancel }
        quitting = true
        Task {
            let ready = SoftwareUpdate.shared.installing
                ? await store.prepareForUpdate()
                : await store.prepareToQuit(stopEverything:stopEverything)
            quitting = false; stopEverything = false
            if ready { dockSpeeds.reset() }
            sender.reply(toApplicationShouldTerminate:ready)
        }
        return .terminateLater
    }
    func applicationDidFinishLaunching(_ notification:Notification) {
        menuLocalization.install()
        SoftwareUpdate.shared.installationFailed = { [weak self] in
            Task { await self?.store?.resumeAfterFailedUpdate() }
        }
        SoftwareUpdate.shared.start()
        #if SEEDHOP_DEVELOPMENT
        if ProcessInfo.processInfo.arguments.contains("--verify-desktop-integration") {
            Task {
                do {
                    try await ExternalOpenSelfTest.verifyWindowRouting(store:{ self.store })
                    try await FileAssociationSelfTest.run()
                    try DockSpeedSelfTest.run()
                    try await ExternalOpenSelfTest.run()
                    exit(0)
                } catch { fputs("Desktop integration verification failed: \(error)\n",stderr); exit(1) }
            }
            return
        }
        if ProcessInfo.processInfo.arguments.contains("--verify-menus") {
            Task {
                do { try await NativeMenuSelfTest.run(); exit(0) }
                catch { fputs("Menu verification failed: \(error)\n",stderr); exit(1) }
            }
            return
        }
        if ProcessInfo.processInfo.arguments.contains("--verify-live-language") {
            Task {
                do { try await LocalizationSelfTest.runLive(store:{ self.store }); exit(0) }
                catch { fputs("Live language verification failed: \(error)\n",stderr); exit(1) }
            }
            return
        }
        if ProcessInfo.processInfo.arguments.contains("--render-localizations") {
            DispatchQueue.main.asyncAfter(deadline:.now() + 0.7) {
                do { try DesignRenderer.renderLocalizations(); exit(0) }
                catch { fputs("Localization render failed: \(error)\n",stderr); exit(1) }
            }
            return
        }
        if ProcessInfo.processInfo.arguments.contains("--verify-localization") {
            do { try LocalizationSelfTest.run(); exit(0) }
            catch { fputs("Localization verification failed: \(error.localizedDescription)\n",stderr); exit(1) }
        }
        #endif
        #if SEEDHOP_VALIDATION
        if ProcessInfo.processInfo.arguments.contains("--verify-lifecycle") {
            Task { do { try await LifecycleSelfTest.run(); exit(0) } catch { fputs("Lifecycle verification failed: \(error.localizedDescription)\n",stderr); exit(1) } }
            return
        }
        #endif
        #if SEEDHOP_DEVELOPMENT
        if ProcessInfo.processInfo.arguments.contains("--verify-storage-recovery") {
            Task { do { try await EngineSelfTest.runStorageRecovery(); exit(0) } catch { fputs("Storage recovery verification failed: \(error.localizedDescription)\n",stderr); exit(1) } }
            return
        }
        if ProcessInfo.processInfo.arguments.contains("--verify-performance") {
            Task { do { try await PerformanceSelfTest.run(store:{ self.store }); exit(0) } catch { fputs("Performance verification failed: \(error.localizedDescription)\n",stderr); exit(1) } }
            return
        }
        if ProcessInfo.processInfo.arguments.contains("--verify-workspace-layout") {
            Task { do { try await InterfaceSelfTest.verifyWorkspaceLayout(); exit(0) } catch { fputs("Workspace layout verification failed: \(error.localizedDescription)\n",stderr); exit(1) } }
            return
        }
        if ProcessInfo.processInfo.arguments.contains("--verify-interface") {
            Task { do { try await InterfaceSelfTest.run(store:{ self.store }); exit(0) } catch { fputs("Interface verification failed: \(error.localizedDescription)\n",stderr); exit(1) } }
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
    func application(_ application:NSApplication,open urls:[URL]) { externalOpens.receive(urls) }
}
