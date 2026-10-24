import SwiftUI
import AppKit
import ServiceManagement
import UserNotifications

enum LibraryFilter: String, CaseIterable, Identifiable {
    case all = "All transfers", downloading = "Downloading", seeding = "Seeding", paused = "Paused", attention = "Needs attention"
    var id: String { rawValue }
    var symbol: String {
        switch self { case .all:return "square.stack.3d.up"; case .downloading:return "arrow.down.circle"; case .seeding:return "arrow.up.circle"; case .paused:return "pause.circle"; case .attention:return "exclamationmark.circle" }
    }
    func includes(_ t: TorrentItem) -> Bool {
        switch self { case .all:return true; case .downloading:return t.isActive; case .seeding:return t.state == "seeding"; case .paused:return t.isPaused; case .attention:return t.state == "error" }
    }
}

@MainActor final class LibraryStore: ObservableObject {
    @Published var torrents: [TorrentItem] = []
    @Published var selection = Set<String>()
    @Published var filter: LibraryFilter = .all
    @Published var search = ""
    @Published var connected = false
    @Published var connecting = false
    @Published var switching = false
    @Published var error: String?
    @Published var showImport = false
    @Published var showInspector = true
    @Published var incomingMagnet = ""
    @Published var incomingFile: URL?
    @Published var showRemoval = false
    @Published var deleteFiles = false
    @Published var downloadLimit: UInt32 = 0
    @Published var uploadLimit: UInt32 = 0
    @Published var listenPort: UInt16 = 0
    @Published var backgroundEnabled = UserDefaults.standard.bool(forKey:"backgroundEnabled")
    @Published var resumeAtLogin = UserDefaults.standard.bool(forKey:"resumeAtLogin")
    @Published var downloadDirectory = UserDefaults.standard.string(forKey:"downloadDirectory") ?? FileManager.default.urls(for:.downloadsDirectory,in:.userDomainMask)[0].path
    @Published var completionNotifications = UserDefaults.standard.object(forKey:"completionNotifications") as? Bool ?? false
    let designPreview: Bool
    let client = EngineClient()
    private var polling: Task<Void,Never>?
    private var reconnect: Task<Void,Never>?

    init(designPreview: Bool = false) {
        self.designPreview = designPreview
        if designPreview { torrents = PreviewFixtures.torrents; selection = [torrents[0].id]; connected = true; return }
        client.onInterruption = { [weak self] in
            guard let self, !self.switching else { return }
            self.connected = false
            self.reconnect?.cancel()
            self.reconnect = Task { try? await Task.sleep(for:.seconds(2)); if !Task.isCancelled { await self.start() } }
        }
    }
    var visibleTorrents: [TorrentItem] { torrents.filter { filter.includes($0) && (search.isEmpty || $0.name.localizedCaseInsensitiveContains(search)) } }
    var selected: [TorrentItem] { torrents.filter { selection.contains($0.id) } }
    var downloadRate: UInt64 { torrents.reduce(0) { $0 + $1.downloadRate } }
    var uploadRate: UInt64 { torrents.reduce(0) { $0 + $1.uploadRate } }
    var totalDownloaded: UInt64 { torrents.reduce(0) { $0 + $1.downloadedBytes } }

    func start() async {
        guard !designPreview, !connecting, !switching else { return }
        connecting = true; defer { connecting = false }
        do {
            let service = SMAppService.agent(plistName:SeedHopIdentity.agentPlist)
            if backgroundEnabled && service.status != .enabled {
                backgroundEnabled = false; UserDefaults.standard.set(false,forKey:"backgroundEnabled")
                error = "Background access is unavailable. Transfers will run while SeedHop is open. You can enable background access in Settings."
            }
            try client.connect(background:backgroundEnabled)
            var request = EngineRequest(command:"connect"); request.destination = downloadDirectory
            let reply = try await client.send(request)
            if let snapshot = reply.snapshot { apply(snapshot) }
            connected = true
            beginPolling()
        } catch { connected = false; self.error = error.localizedDescription }
    }
    private func beginPolling() {
        polling?.cancel()
        polling = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for:.seconds(1))
                guard !Task.isCancelled, let self else { break }
                if NSApp.windows.contains(where:{ $0.isVisible && $0.title == "SeedHop" }) { await self.refresh() }
            }
        }
    }
    func refresh() async {
        guard !designPreview, connected, !switching else { return }
        do { if let snapshot = try await client.send(EngineRequest(command:"snapshot")).snapshot { apply(snapshot) } }
        catch { connected = false; self.error = error.localizedDescription }
    }
    private func apply(_ snapshot: LibrarySnapshot) {
        if completionNotifications, !torrents.isEmpty {
            var notified = Set(UserDefaults.standard.stringArray(forKey:"completedTorrents") ?? [])
            for item in snapshot.torrents where item.state == "seeding" && !notified.contains(item.id) {
                if let previous = torrents.first(where:{$0.id == item.id}), previous.downloadedBytes < previous.totalBytes {
                    let content = UNMutableNotificationContent(); content.title = "Your files are ready"; content.body = item.name; content.sound = .default
                    UNUserNotificationCenter.current().add(UNNotificationRequest(identifier:item.id,content:content,trigger:nil))
                    notified.insert(item.id)
                }
            }
            UserDefaults.standard.set(Array(notified),forKey:"completedTorrents")
        }
        // Avoid republishing identical rows; SwiftUI Table retains stable identity/selection.
        if torrents != snapshot.torrents { torrents = snapshot.torrents }
        let remainingSelection = selection.intersection(Set(torrents.map(\.id)))
        if selection != remainingSelection { selection = remainingSelection }
        if downloadLimit != snapshot.downloadLimit { downloadLimit = snapshot.downloadLimit }
        if uploadLimit != snapshot.uploadLimit { uploadLimit = snapshot.uploadLimit }
        if listenPort != snapshot.listenPort { listenPort = snapshot.listenPort }
    }
    func perform(_ request: EngineRequest) async throws -> EngineReply {
        guard !designPreview else { throw UserFacingError(message:"This is a design preview with sample data. Launch the app normally for real transfers.") }
        return try await client.send(request)
    }
    func toggleSelected(paused: Bool) {
        Task {
            do { for item in selected { var r = EngineRequest(command:"pause"); r.id = item.id; r.paused = paused; _ = try await perform(r) }; await refresh() }
            catch { self.error = error.localizedDescription }
        }
    }
    func removeSelected() {
        let items = selected; let deleting = deleteFiles
        Task { do { for item in items { var r = EngineRequest(command:"remove"); r.id = item.id; r.deleteFiles = deleting; _ = try await perform(r) }; selection.removeAll(); await refresh() } catch { self.error = error.localizedDescription } }
    }
    func setFileSelection(_ item: TorrentItem, indices: [UInt32]) {
        Task { do { var r = EngineRequest(command:"select"); r.id = item.id; r.selected = indices; _ = try await perform(r); await refresh() } catch { self.error = error.localizedDescription } }
    }
    func setLimits(download: UInt32, upload: UInt32) async throws {
        var r = EngineRequest(command:"limits"); r.downloadLimit = download; r.uploadLimit = upload
        _ = try await perform(r); await refresh()
    }
    func setBackground(_ enabled: Bool) async {
        guard !designPreview, !switching else { return }
        switching = true; polling?.cancel(); defer { switching = false; beginPolling() }
        let service = SMAppService.agent(plistName:SeedHopIdentity.agentPlist)
        var stoppedCurrent = false
        do {
            if enabled {
                if service.status == .notRegistered || service.status == .notFound { try service.register() }
                guard service.status == .enabled else { throw UserFacingError(message:"Allow SeedHop in System Settings → General → Login Items & Extensions, then try again.") }
            }
            _ = try await client.send(EngineRequest(command:"shutdown")); stoppedCurrent = true; client.invalidate(); connected = false
            if !enabled && service.status != .notRegistered { try await service.unregister() }
            apply(try await reconnectAfterHandoff(background:enabled))
            backgroundEnabled = enabled; connected = true
            UserDefaults.standard.set(enabled,forKey:"backgroundEnabled")
            var login = EngineRequest(command:"login"); login.resumeAtLogin = resumeAtLogin
            _ = try await client.send(login)
        } catch {
            self.error = error.localizedDescription
            // Registration denial must not disrupt a working foreground engine.
            guard stoppedCurrent else { return }
            // Stop the agent before foreground fallback. The OS lock remains authoritative.
            if service.status == .enabled { try? await service.unregister() }
            backgroundEnabled = false; UserDefaults.standard.set(false,forKey:"backgroundEnabled")
            do { apply(try await reconnectAfterHandoff(background:false)); connected = true }
            catch { self.error = error.localizedDescription; connected = false }
        }
    }
    private func reconnectAfterHandoff(background: Bool) async throws -> LibrarySnapshot {
        var failure: Error = UserFacingError(message:"The engine could not restart. Your library has been preserved.")
        for attempt in 0..<12 {
            do {
                try client.connect(background:background)
                var request = EngineRequest(command:"connect"); request.destination = downloadDirectory
                if let snapshot = try await client.send(request).snapshot { return snapshot }
            } catch { failure = error }
            client.invalidate()
            try await Task.sleep(for:.milliseconds(min(600,100 + attempt * 50)))
        }
        throw failure
    }
    func setLogin(_ enabled: Bool) async {
        do { var r = EngineRequest(command:"login"); r.resumeAtLogin = enabled; _ = try await perform(r); resumeAtLogin = enabled; UserDefaults.standard.set(enabled,forKey:"resumeAtLogin") }
        catch { self.error = error.localizedDescription }
    }
    func prepareToQuit(stopEverything: Bool) async -> Bool {
        polling?.cancel(); reconnect?.cancel()
        if !connected || designPreview { return true }
        if backgroundEnabled && !stopEverything { client.invalidate(); return true }
        do { var r = EngineRequest(command:"shutdown"); r.explicitStop = stopEverything; _ = try await client.send(r); client.invalidate(); return true }
        catch { self.error = error.localizedDescription; beginPolling(); return false }
    }
    func openFiles() {
        let panel = NSOpenPanel(); panel.allowedContentTypes = [.init(filenameExtension:"torrent")!]; panel.allowsMultipleSelection = false
        panel.begin { result in if result == .OK, let url = panel.url { self.incomingFile = url; self.incomingMagnet = ""; self.showImport = true } }
    }
    func pasteMagnet() { incomingMagnet = NSPasteboard.general.string(forType:.string) ?? ""; incomingFile = nil; showImport = true }
    func open(_ urls: [URL]) {
        guard let url = urls.first else { return }
        if url.scheme == "magnet" { incomingMagnet = url.absoluteString; incomingFile = nil } else { incomingFile = url; incomingMagnet = "" }
        showImport = true
    }
}
