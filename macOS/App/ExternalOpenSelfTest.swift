#if SEEDHOP_DEVELOPMENT
import AppKit
import SwiftUI

@MainActor enum ExternalOpenSelfTest {
    /// Send real Launch Services events; coordinator-only tests cannot detect extra SwiftUI scenes.
    static func verifyWindowRouting(store: () -> LibraryStore?) async throws {
        func expect(_ condition: Bool, _ message: String) throws {
            if !condition { throw UserFacingError(message:message) }
        }
        func settle() async throws { try await Task.sleep(for:.milliseconds(500)) }
        func libraries() -> [NSWindow] { NSApp.windows.filter { $0.isVisible && $0.title == "SeedHop" } }
        func open(_ url: URL) async throws {
            let configuration = NSWorkspace.OpenConfiguration()
            configuration.activates = true
            let application = try await NSWorkspace.shared.open([url],withApplicationAt:Bundle.main.bundleURL,configuration:configuration)
            try expect(application.processIdentifier == ProcessInfo.processInfo.processIdentifier,"External URL launched another app process")
            try await settle()
        }
        try await settle()
        guard let store = store(), let library = libraries().first else {
            throw UserFacingError(message:"No library window for external URL verification")
        }
        try expect(libraries().count == 1,"External URL fixture started with multiple libraries")
        let first = URL(string:"magnet:?xt=urn:btih:0123456789012345678901234567890123456789")!
        let second = URL(string:"magnet:?xt=urn:btih:1123456789012345678901234567890123456789")!
        try await open(first)
        try expect(libraries().count == 1 && libraries().first === library,"Magnet link created another library window")
        try expect(store.showImport && store.incomingMagnet == first.absoluteString,"Magnet link did not reach existing library")
        try await open(second)
        try expect(libraries().count == 1 && libraries().first === library,"Repeated magnet link created another library window")
        try expect(store.incomingMagnet == first.absoluteString && store.externalOpenCoordinator?.pendingCount == 1,"Repeated magnet replaced active import or lost queued link")
        store.showImport = false
        try await settle()
        try expect(store.showImport && store.incomingMagnet == second.absoluteString,"Queued magnet did not reach existing library")
        store.showImport = false
        try await settle()
        library.miniaturize(nil)
        try await open(first)
        try expect(libraries().count == 1 && libraries().first === library && !library.isMiniaturized,"Magnet did not reveal minimized library")
        store.showImport = false
        try await settle()
        library.close()
        try await settle()
        try await open(second)
        try expect(libraries().count == 1 && store.showImport && store.incomingMagnet == second.absoluteString,"Magnet did not reopen exactly one library")
        store.showImport = false
        try await settle()
        print("PASS: Launch Services magnets reuse the existing library, queue during review, reveal minimized windows, and reopen one closed library")
    }

    static func run() async throws {
        func expect(_ condition: Bool, _ message: String) throws {
            if !condition { throw UserFacingError(message:message) }
        }
        func settle() async { for _ in 0..<12 { await Task.yield() }; try? await Task.sleep(for:.milliseconds(20)) }
        let first = URL(fileURLWithPath:"/tmp/seedhop-first.torrent")
        let second = URL(string:"magnet:?xt=urn:btih:0123456789012345678901234567890123456789")!
        let third = URL(fileURLWithPath:"/tmp/seedhop-third.TORRENT")
        let coordinator = ExternalOpenCoordinator()
        coordinator.receive([first,second,first,URL(string:"https://example.com")!,third])
        try expect(coordinator.pendingCount == 3,"Cold-launch queue lost or duplicated requests")
        let store = LibraryStore(designPreview:true)
        store.connected = false
        var reveals = 0
        coordinator.reveal = { reveals += 1 }
        coordinator.attach(store)
        try expect(!store.showImport && coordinator.pendingCount == 3,"Cold launch attempted inspection before engine readiness")
        store.connected = true
        await settle()
        try expect(store.showImport && store.incomingFile == first,"Cold-launch request did not reach review")
        try expect(store.error != nil && reveals == 1,"Unsupported input hid pending requests")
        coordinator.receive([first,second])
        try expect(coordinator.pendingCount == 2,"Outstanding duplicate callbacks were queued twice")
        store.showImport = false
        await settle()
        try expect(!store.showImport,"Next import opened before prior sheet dismissal")
        coordinator.didDismissImport()
        await settle()
        try expect(store.showImport && store.incomingMagnet == second.absoluteString && store.incomingFile == nil,"Mixed queue lost order")
        store.fileOperation = FileOperationItem(id:"fixture",torrentId:"fixture",kind:"move",phase:"copying",completedBytes:0,totalBytes:1,cancelRequested:false)
        store.showImport = false
        coordinator.didDismissImport()
        await settle()
        try expect(!store.showImport && coordinator.pendingCount == 1,"Import bypassed file-operation conflict")
        store.fileOperation = nil
        await settle()
        try expect(store.showImport && store.incomingFile == third,"Import did not resume after operation")
        store.showImport = false
        coordinator.didDismissImport()
        await settle()
        store.beginImport()
        coordinator.receive([first])
        try expect(store.incomingFile == nil && coordinator.pendingCount == 1,"External request replaced manual import")
        store.showImport = false
        coordinator.didDismissImport()
        await settle()
        try expect(store.showImport && store.incomingFile == first,"External request lost after manual cancellation")
        let saved = store.showDockSpeeds
        store.showDockSpeeds = true
        try expect(store.shouldPollSnapshots && WorkspacePreferences.defaults.bool(forKey:"showDockSpeeds"),"Dock preference did not enable background sampling")
        store.showDockSpeeds = saved
        let down = store.torrents.reduce(UInt64(0)) { $0 + $1.downloadRate }
        store.search = "no matching transfer"
        try expect(store.downloadRate == down,"Dock aggregate depended on library filter")
        store.connected = false
        try expect(store.downloadRate == 0 && store.uploadRate == 0,"Disconnected aggregate retained stale rates")
        let unavailable = LibraryStore(client:UnavailableSnapshotFixture())
        unavailable.torrents = store.torrents
        unavailable.connected = true
        await unavailable.refresh()
        try expect(!unavailable.connected && unavailable.downloadRate == 0 && unavailable.error != nil,"Missing snapshot retained stale Dock rates")
        print("PASS: External opens buffer, deduplicate, serialize reviews, wait for conflicts, and preserve manual imports")
        print("PASS: Dock preference persistence, background polling, and unfiltered aggregates")
        try await verifyAssociationLayout()
    }

    private static func verifyAssociationLayout() async throws {
        let localization = AppLocalization.shared
        let originalLanguage = localization.language
        defer { localization.use(originalLanguage) }
        let args = ProcessInfo.processInfo.arguments
        let output = args.firstIndex(of:"--output").flatMap { index in args.indices.contains(index + 1) ? URL(fileURLWithPath:args[index + 1],isDirectory:true) : nil }
        for language in ["en","de"] {
            localization.use(language)
            for (name,appearance) in [("light",NSAppearance.Name.aqua),("dark",.darkAqua),("contrast",.accessibilityHighContrastDarkAqua)] {
                let window = NSWindow(contentRect:NSRect(x:0,y:0,width:518,height:230),styleMask:[],backing:.buffered,defer:false)
                window.isReleasedWhenClosed = false
                window.appearance = NSAppearance(named:appearance)
                let host = NSHostingView(rootView:FileAssociationSettings().padding(17).background(Design.canvas))
                window.contentView = host
                window.orderFront(nil)
                try? await Task.sleep(for:.milliseconds(100))
                host.layoutSubtreeIfNeeded()
                func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
                let controls = descendants(host).compactMap { $0 as? NSSwitch }
                guard controls.count == 2 else { window.close(); throw UserFacingError(message:"Association switches missing from native Settings") }
                let frames = controls.map { $0.convert($0.bounds,to:host) }
                guard frames.allSatisfy({ $0.maxX > host.bounds.width - 70 }), abs(frames[0].maxX - frames[1].maxX) < 1 else {
                    window.close(); throw UserFacingError(message:"Association switches do not align at the trailing edge: \(frames)")
                }
                if let output, let bitmap = host.bitmapImageRepForCachingDisplay(in:host.bounds) {
                    host.cacheDisplay(in:host.bounds,to:bitmap)
                    try bitmap.representation(using:.png,properties:[:])!.write(to:output.appendingPathComponent("associations-\(language)-\(name).png"))
                }
                window.close()
            }
        }
        print("PASS: Native association switches align at the trailing edge in English/German and light/dark/increased contrast")
    }
}

@MainActor private final class UnavailableSnapshotFixture: EngineTransport {
    var onInterruption: (() -> Void)?
    func connect(background: Bool) throws {}
    func invalidate() {}
    func send(_ request: EngineRequest) async throws -> EngineReply { EngineReply() }
}
#endif
