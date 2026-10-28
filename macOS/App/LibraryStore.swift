import SwiftUI
import AppKit
import ServiceManagement
import UserNotifications

enum LibraryFilter: String, CaseIterable, Identifiable {
    case all = "All transfers", downloading = "Downloading", seeding = "Seeding", paused = "Paused", queued = "Queued", checking = "Checking", attention = "Needs attention"
    var id: String { rawValue }
    var symbol: String {
        switch self { case .all:return "square.stack.3d.up"; case .downloading:return "arrow.down.circle"; case .seeding:return "arrow.up.circle"; case .paused:return "pause.circle"; case .queued:return "list.number"; case .checking:return "checkmark.shield"; case .attention:return "exclamationmark.circle" }
    }
    var emptyTitle: String {
        switch self {
        case .all: return "No transfers yet"
        case .downloading: return "No downloads in progress"
        case .seeding: return "No transfers seeding"
        case .paused: return "No paused transfers"
        case .queued: return "No queued transfers"
        case .checking: return "No transfers being checked"
        case .attention: return "Everything looks good"
        }
    }
    func includes(_ t: TorrentItem) -> Bool {
        switch self { case .all:return true; case .downloading:return t.isActive; case .seeding:return t.state == "seeding"; case .paused:return t.isPaused; case .queued:return t.state == "queued"; case .checking:return t.state == "checking"; case .attention:return t.state == "error" }
    }
}

@MainActor final class LibraryStore: ObservableObject {
    @Published var torrents: [TorrentItem] = [] { didSet { reindexTorrents() } }
    private var selectedIDs = Set<String>()
    var selection: Set<String> {
        get { selectedIDs }
        set {
            guard newValue != selectedIDs else { return }
            objectWillChange.send()
            selectedIDs = newValue
        }
    }
    @Published var filter: LibraryFilter = .all { didSet { if oldValue != filter { filteredTorrents = nil; reconcileVisibleSelection() } } }
    @Published var search = "" { didSet { if oldValue != search { filteredTorrents = nil; reconcileVisibleSelection() } } }
    @Published var trackerHost = "" { didSet { if oldValue != trackerHost { filteredTorrents = nil; reconcileVisibleSelection() } } }
    @Published var details: TorrentDetails?
    @Published var detailsError: String?
    @Published var allPeers = false
    @Published var peerRates: [String:PeerRate] = [:]
    var history: [String:[TrafficSample]] = [:]
    @Published var events: [String:[TransferEvent]] = [:]
    @Published private(set) var engineInstance = ""
    private var detailsContext = UUID()
    private var detailsInFlight = false
    private var peerBaselines: [String:(UInt64,UInt64,TimeInterval)] = [:]
    private var lastTrackerEvent: [String:UInt64] = [:]
    @Published var connected = false
    @Published var connecting = false
    @Published var switching = false
    @Published var statisticsWarning: String?
    @Published var error: String?
    @Published var showImport = false
    @Published var showInspector = WorkspacePreferences.defaults.object(forKey:"showTransferDetails") as? Bool ?? true { didSet { WorkspacePreferences.defaults.set(showInspector,forKey:"showTransferDetails") } }
    @Published var incomingMagnet = ""
    @Published var incomingFile: URL?
    @Published var showRemoval = false
    @Published private(set) var removalTargets: [TorrentItem] = []
    @Published private(set) var busyTransfers = Set<String>()
    @Published var fileOperation: FileOperationItem? { didSet { rebuildCounts() } }
    @Published var storageRecovery: StorageRecoveryItem? { didSet { rebuildCounts() } }
    @Published private(set) var resolvingStorageRecovery = false
    @Published private(set) var storageRequests = Set<String>()
    @Published private(set) var cancellingOperations = Set<String>()
    @Published var downloadLimit: UInt32 = 0
    @Published var uploadLimit: UInt32 = 0
    @Published var maxActiveDownloads: UInt32 = 0
    @Published var savingQueue = false
    @Published var peerEncryption: PeerEncryptionPolicy?
    @Published var savedPeerEncryption: PeerEncryptionPolicy?
    @Published private(set) var savingEncryption = false
    @Published var listenPort: UInt16 = 0
    @Published var backgroundEnabled = UserDefaults.standard.bool(forKey:"backgroundEnabled")
    @Published var resumeAtLogin = UserDefaults.standard.bool(forKey:"resumeAtLogin")
    @Published var downloadDirectory = UserDefaults.standard.string(forKey:"downloadDirectory") ?? FileManager.default.urls(for:.downloadsDirectory,in:.userDomainMask)[0].path
    @Published var completionNotifications = UserDefaults.standard.object(forKey:"completionNotifications") as? Bool ?? false
    let designPreview: Bool
    let client: any EngineTransport
    // Derived from the current snapshot, never a second source of transfer state.
    private var torrentIndices: [String:Int] = [:]
    private(set) var torrentGeneration: UInt64 = 0
    private var filterCounts: [LibraryFilter:Int] = [:]
    private var stateCounts: [String:Int] = [:]
    private var filteredTorrents: [TorrentItem]?
    private var polling: Task<Void,Never>?
    private var reconnect: Task<Void,Never>?

    init(designPreview: Bool = false, client: (any EngineTransport)? = nil) {
        self.designPreview = designPreview
        self.client = client ?? EngineClient()
        if designPreview { peerEncryption = .allow; savedPeerEncryption = .allow; torrents = PreviewFixtures.torrents; reindexTorrents(); selection = [torrents[0].id]; details = PreviewFixtures.diagnostics(torrents[0],section:"general"); history[torrents[0].id] = PreviewFixtures.trafficHistory(); connected = true; return }
        self.client.onInterruption = { [weak self] in
            guard let self, !self.switching else { return }
            self.connected = false
            self.reconnect?.cancel()
            self.reconnect = Task { try? await Task.sleep(for:.seconds(2)); if !Task.isCancelled { await self.start() } }
        }
    }
    private func reindexTorrents() {
        torrentGeneration &+= 1
        torrentIndices.removeAll(keepingCapacity:true)
        torrentIndices.reserveCapacity(torrents.count)
        for index in torrents.indices { torrentIndices[torrents[index].id] = index }
        rebuildCounts()
    }
    private func rebuildCounts() {
        filteredTorrents = nil
        let operation = fileOperation, recovery = storageRecovery
        var counts: [LibraryFilter:Int] = [.all:torrents.count]
        var states: [String:Int] = [:]
        for item in torrents {
            let status = TransferPresentation(torrent:item,operation:operation,recovery:recovery)
            if status.isActive { counts[.downloading,default:0] += 1 }
            if status.isSeeding { counts[.seeding,default:0] += 1 }
            if status.isQueued { counts[.queued,default:0] += 1 }
            if status.isChecking { counts[.checking,default:0] += 1 }
            if item.isPaused { counts[.paused,default:0] += 1 }
            if status.needsAttention { counts[.attention,default:0] += 1 }
            if item.id != operation?.torrentId { states[item.state,default:0] += 1 }
        }
        filterCounts = counts; stateCounts = states
    }
    func presentation(_ item: TorrentItem) -> TransferPresentation {
        TransferPresentation(torrent:item,operation:fileOperation,recovery:storageRecovery,connected:connected)
    }
    func includes(_ item: TorrentItem, in filter: LibraryFilter) -> Bool {
        switch filter {
        case .queued: return presentation(item).isQueued
        case .checking: return presentation(item).isChecking
        case .attention: return presentation(item).needsAttention
        case .downloading: return presentation(item).isActive
        case .seeding: return presentation(item).isSeeding
        default: return filter.includes(item)
        }
    }
    func count(in filter: LibraryFilter) -> Int { filterCounts[filter,default:0] }
    var visibleTorrents: [TorrentItem] {
        let currentFilter = filter, query = search
        if currentFilter == .all && query.isEmpty && trackerHost.isEmpty { return torrents }
        if let filteredTorrents { return filteredTorrents }
        let result = torrents.filter { includes($0,in:currentFilter) && (query.isEmpty || $0.name.localizedCaseInsensitiveContains(query)) && (trackerHost.isEmpty || $0.trackers.contains { URL(string:$0)?.host?.lowercased() == trackerHost.lowercased() }) }
        filteredTorrents = result
        return result
    }
    var selected: [TorrentItem] {
        guard !selection.isEmpty else { return [] }
        // Keep the original snapshot order. Large/ambiguous selections retain
        // the original scan, including its behavior for unexpected duplicate IDs.
        if selection.count > torrents.count / 2 || torrentIndices.count != torrents.count {
            return torrents.filter { selection.contains($0.id) }
        }
        return selection.compactMap { torrentIndices[$0] }.sorted().map { torrents[$0] }
    }
    var downloadRate: UInt64 { connected ? torrents.reduce(0) { $0 + $1.downloadRate } : 0 }
    var uploadRate: UInt64 { connected ? torrents.reduce(0) { $0 + $1.uploadRate } : 0 }
    var totalDownloaded: UInt64 { torrents.reduce(0) { $0 + $1.downloadedBytes } }
    var canActOnSelection: Bool { canChangeTransfers && !selected.isEmpty && selected.allSatisfy { !busyTransfers.contains($0.id) } }
    func canResumeOrRetry(_ item: TorrentItem) -> Bool {
        item.state == "error" ? presentation(item).canRetry : item.isPaused && item.state != "checking"
    }
    var canResumeSelection: Bool { canActOnSelection && selected.contains(where:canResumeOrRetry) }
    var canPauseSelection: Bool { canActOnSelection && selected.contains { !$0.isPaused } }
    func hasPendingCleanup(_ id: String) -> Bool { storageRecovery.map { $0.torrentId.isEmpty || $0.torrentId == id } ?? false }
    var canRecheckSelection: Bool { canActOnSelection && selected.allSatisfy { $0.state != "checking" && !hasPendingCleanup($0.id) } }
    var canMoveSelection: Bool { canRecheckSelection && storageRecovery == nil && selected.count == 1 }
    var canRemoveSelection: Bool { canActOnSelection && selected.allSatisfy { !hasPendingCleanup($0.id) } }
    func resumeLabel(for items: [TorrentItem]) -> String {
        let retry = items.contains { $0.state == "error" && presentation($0).canRetry }
        let resume = items.contains { $0.state != "error" && canResumeOrRetry($0) }
        return retry ? (resume ? "Resume and Retry" : "Retry") : "Resume"
    }
    var resumeActionLabel: String { resumeLabel(for:selected) }
    var hasUnscopedRecovery: Bool { storageRecovery.map { notice in torrentIndices[notice.torrentId] == nil } ?? false }
    var activitySummary: String {
        if !connected { return "Showing the last update. Reconnect for live activity." }
        if torrents.isEmpty { return storageRecovery == nil ? "A fresh start for your next download." : "Retained files need attention." }
        var parts = [("downloading", "downloading"), ("waiting", "waiting"), ("queued", "queued"), ("checking", "checking"), ("seeding", "seeding"), ("paused", "paused")].compactMap { state, label -> String? in
            let count = stateCounts[state,default:0]
            return count == 0 ? nil : "\(count) \(label)"
        }
        if fileOperation != nil { parts.insert("1 file change",at:0) }
        let attention = count(in:.attention)
        if attention > 0 { parts.append("\(attention) need attention") }
        else if hasUnscopedRecovery { parts.append("File cleanup needs attention") }
        return parts.isEmpty ? "Waiting for a transfer update." : parts.joined(separator:" · ")
    }
    var hasFileOperation: Bool { fileOperation != nil || !storageRequests.isEmpty || resolvingStorageRecovery }
    var canChangeTransfers: Bool { connected && !switching && !designPreview && !hasFileOperation }

    var encryptionRestartPending: Bool { peerEncryption != nil && savedPeerEncryption != nil && peerEncryption != savedPeerEncryption }
    var canSaveEncryption: Bool { canChangeTransfers && !savingEncryption && savedPeerEncryption != nil }
    var canRestartForEncryption: Bool { canChangeTransfers && encryptionRestartPending && !savingEncryption && busyTransfers.isEmpty && !savingQueue && !showImport }
    var connectionPortDescription: String { !connected ? "Unavailable while disconnected" : listenPort == 0 ? "No listening port reported" : "\(listenPort)" }
    func saveEncryption(_ policy: PeerEncryptionPolicy) {
        guard canSaveEncryption else { return }
        savingEncryption = true; error = nil
        Task {
            defer { savingEncryption = false }
            do {
                var request = EngineRequest(command:"peerEncryption"); request.peerEncryption = policy
                _ = try await perform(request)
            } catch { self.error = error.localizedDescription }
            await refresh()
        }
    }
    func restartForEncryption() {
        guard canRestartForEncryption else { return }
        switching = true; polling?.cancel(); reconnect?.cancel(); error = nil
        let background = backgroundEnabled, loginAtRestart = resumeAtLogin
        Task {
            defer { switching = false; beginPolling() }
            do {
                // No explicitStop: shutdown persists each transfer's existing pause intent.
                _ = try await client.send(EngineRequest(command:"shutdown"))
                client.invalidate(); connected = false
                apply(try await reconnectAfterHandoff(background:background)); connected = true
                var login = EngineRequest(command:"login"); login.resumeAtLogin = loginAtRestart
                _ = try await client.send(login)
            } catch { self.error = error.localizedDescription }
        }
    }

    private func reconcileVisibleSelection() {
        let remaining = selection.intersection(Set(visibleTorrents.map(\.id)))
        // The filter already published this mutation. Update hidden selection
        // in the same transaction instead of publishing an intermediate state.
        if remaining != selectedIDs { selectedIDs = remaining }
    }

    func revealTransfer(_ id: String) {
        filter = .all; search = ""; trackerHost = ""; selection = [id]
    }

    func start() async {
        guard !designPreview, !connected, !connecting, !switching else { return }
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
        if engineInstance != snapshot.instanceId {
            engineInstance = snapshot.instanceId; detailsContext = UUID(); details = nil; peerBaselines = [:]; peerRates = [:]; lastTrackerEvent = [:]
        }
        if statisticsWarning != snapshot.statisticsWarning { statisticsWarning = snapshot.statisticsWarning }
        recordActivity(snapshot)

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
        if peerEncryption != snapshot.peerEncryption { peerEncryption = snapshot.peerEncryption }
        if savedPeerEncryption != snapshot.savedPeerEncryption { savedPeerEncryption = snapshot.savedPeerEncryption }
        if listenPort != snapshot.listenPort { listenPort = snapshot.listenPort }
        if maxActiveDownloads != snapshot.maxActiveDownloads { maxActiveDownloads = snapshot.maxActiveDownloads }
        if fileOperation != snapshot.fileOperation { fileOperation = snapshot.fileOperation }
        if storageRecovery != snapshot.storageRecovery { storageRecovery = snapshot.storageRecovery }
    }
    var trackerHosts: [String] {
        var hosts = Set(torrents.flatMap { $0.trackers.compactMap { URL(string:$0)?.host?.lowercased() } })
        if !trackerHost.isEmpty { hosts.insert(trackerHost) }
        return hosts.sorted()
    }
    func clearFilters() { search = ""; trackerHost = ""; filter = .all }
    private func recordActivity(_ snapshot: LibrarySnapshot) {
        let date = Date()
        let liveIDs = Set(snapshot.torrents.map(\.id))
        for item in snapshot.torrents {
            if let previous = torrentIndices[item.id].map({ torrents[$0] }), previous.state != item.state || previous.errorKind != item.errorKind {
                appendEvent(item.id, message:item.statusLabel)
            }
            // Sample only real traffic changes or elapsed seconds, never every body read.
            if history[item.id]?.last.map({ date.timeIntervalSince($0.id) >= 1 }) ?? true {
                var samples = history[item.id] ?? []
                samples.append(TrafficSample(id:date,download:item.downloadRate,upload:item.uploadRate))
                samples.removeAll { date.timeIntervalSince($0.id) > 900 }
                if samples.count > 900 { samples.removeFirst(samples.count - 900) }
                history[item.id] = samples
            }
        }
        if snapshot.fileOperation != fileOperation, let operation = snapshot.fileOperation {
            appendEvent(operation.torrentId,message:operation.title)
        }
        if snapshot.storageRecovery != storageRecovery, let notice = snapshot.storageRecovery {
            appendEvent(notice.torrentId,message:"File cleanup needs attention")
        }
        if Set(history.keys) != liveIDs { history = history.filter { liveIDs.contains($0.key) }; events = events.filter { liveIDs.contains($0.key) } }
    }
    private func appendEvent(_ id: String, message: String) {
        var rows = events[id] ?? []; rows.append(TransferEvent(message:message))
        if rows.count > 200 { rows.removeFirst(rows.count - 200) }; events[id] = rows
    }
    func watchDetails(id: String, section: String, allPeers: Bool) async {
        let token = UUID(); detailsContext = token
        if details?.torrentId != id || details?.section != section { details = nil; detailsError = nil; peerRates = [:]; peerBaselines = [:] }
        while !Task.isCancelled && showInspector && selection == [id] {
            if connected && !switching && !detailsInFlight && NSApp.windows.contains(where:{ $0.isVisible && $0.title == "SeedHop" }) {
                detailsInFlight = true
                let instance = engineInstance
                do {
                    var request = EngineRequest(command:"torrentDetails"); request.id = id; request.section = section; request.allPeers = allPeers
                    let reply = try await client.send(request)
                    guard let value = reply.details else { throw UserFacingError(message:"The engine did not return transfer details. Try again.") }
                    if !Task.isCancelled && detailsContext == token && instance == engineInstance && showInspector && selection == [id], value.torrentId == id && value.instanceId == instance && value.section == section {
                        applyDetails(value); if detailsError != nil { detailsError = nil }
                    }
                } catch {
                    if !Task.isCancelled && detailsContext == token && detailsError != error.localizedDescription { detailsError = error.localizedDescription }
                }
                detailsInFlight = false
            }
            do { try await Task.sleep(for:.seconds(1)) } catch { break }
        }
    }
    private func applyDetails(_ value: TorrentDetails) {
        let now = ProcessInfo.processInfo.systemUptime
        var rates: [String:PeerRate] = [:], next: [String:(UInt64,UInt64,TimeInterval)] = [:]
        for peer in value.peers {
            if let previous = peerBaselines[peer.id], now > previous.2 && peer.downloaded >= previous.0 && peer.uploaded >= previous.1 {
                rates[peer.id] = PeerRate(download:UInt64(Double(peer.downloaded - previous.0) / (now - previous.2)),upload:UInt64(Double(peer.uploaded - previous.1) / (now - previous.2)))
            }
            next[peer.id] = (peer.downloaded,peer.uploaded,now)
        }
        peerBaselines = next; if peerRates != rates { peerRates = rates }
        for event in value.events ?? [] where event.sequence > lastTrackerEvent[value.torrentId,default:0] {
            var rows = events[value.torrentId] ?? []
            rows.append(TransferEvent(date:Date(timeIntervalSince1970:Double(event.timestamp)),message:event.message))
            rows.sort { $0.date < $1.date }
            if rows.count > 200 { rows.removeFirst(rows.count - 200) }
            events[value.torrentId] = rows; lastTrackerEvent[value.torrentId] = event.sequence
        }
        if details != value { details = value }
    }
    func diagnosticReport(_ torrent: TorrentItem) -> String {
        // Export only allowlisted fields. Remote strings, paths, hashes and endpoints stay out.
        let value = details?.torrentId == torrent.id ? details : nil
        let report = "SeedHop diagnostics\nState: \(torrent.statusLabel)\nEngine connected: \(connected)\nFormat: \(value?.format ?? "unknown")\nConnected peers: \(torrent.peers)\nDownload: \(SizeText.rate(connected ? torrent.downloadRate : 0))\nUpload: \(SizeText.rate(connected ? torrent.uploadRate : 0))\nTracker states: \(value?.trackers.map(\.state).joined(separator:", ") ?? "not sampled")\nPersonal paths, torrent identifiers, tracker URLs and peer endpoints omitted."
        return report
    }
    func copyDiagnostics(_ torrent: TorrentItem) {
        NSPasteboard.general.clearContents(); NSPasteboard.general.setString(diagnosticReport(torrent),forType:.string)
    }

    func perform(_ request: EngineRequest) async throws -> EngineReply {
        guard !designPreview else { throw UserFacingError(message:"This is a design preview with sample data. Launch the app normally for real transfers.") }
        if hasFileOperation && ["add", "inspect", "pause", "select", "priorities", "queue", "queueMove", "recheck", "retry", "remove", "limits", "peerEncryption"].contains(request.command) {
            throw UserFacingError(message:"Wait for the current file change to finish, or cancel it, then try again.")
        }
        return try await client.send(request)
    }
    func toggleSelected(paused: Bool) {
        guard canActOnSelection else { return }
        let items = selected.filter { paused ? !$0.isPaused : canResumeOrRetry($0) }
        let ids = Set(items.map(\.id))
        guard !ids.isEmpty else { return }
        error = nil
        busyTransfers.formUnion(ids)
        Task {
            defer { busyTransfers.subtract(ids) }
            do { for item in items {
                var r = EngineRequest(command:!paused && item.state == "error" ? "retry" : "pause")
                r.id = item.id; r.paused = paused; _ = try await perform(r)
            } }
            catch { self.error = error.localizedDescription }
            await refresh()
        }
    }
    func requestRemoval() {
        guard canRemoveSelection else { return }
        removalTargets = selected
        showRemoval = true
    }
    func recheckSelected() {
        guard canRecheckSelection else { return }
        let ids = selected.map(\.id)
        error = nil
        busyTransfers.formUnion(ids)
        Task {
            defer { busyTransfers.subtract(ids) }
            do { for id in ids { var r = EngineRequest(command:"recheck"); r.id = id; _ = try await perform(r) } }
            catch { self.error = error.localizedDescription }
            await refresh()
        }
    }
    func removeConfirmed(deleteFiles: Bool) {
        let items = removalTargets
        let ids = Set(items.map(\.id))
        guard canChangeTransfers, !ids.isEmpty, busyTransfers.isDisjoint(with:ids), items.allSatisfy({ !hasPendingCleanup($0.id) }) else { return }
        error = nil
        busyTransfers.formUnion(ids)
        Task {
            defer {
                busyTransfers.subtract(ids)
                if Set(removalTargets.map(\.id)) == ids { removalTargets = [] }
            }
            do { for item in items { var r = EngineRequest(command:"remove"); r.id = item.id; r.deleteFiles = deleteFiles; _ = try await perform(r); selection.remove(item.id) } }
            catch { self.error = error.localizedDescription }
            await refresh()
        }
    }
    func setFileSelection(_ item: TorrentItem, indices: [UInt32]) {
        guard canChangeTransfers, !busyTransfers.contains(item.id), !indices.isEmpty else { return }
        error = nil
        busyTransfers.insert(item.id)
        Task {
            defer { busyTransfers.remove(item.id) }
            do { var r = EngineRequest(command:"select"); r.id = item.id; r.selected = indices.sorted(); _ = try await perform(r) }
            catch { self.error = error.localizedDescription }
            await refresh()
        }
    }
    func setLimits(download: UInt32, upload: UInt32) async throws {
        guard !hasFileOperation else { throw UserFacingError(message:"Wait for the current file change to finish, then apply limits again.") }
        var r = EngineRequest(command:"limits"); r.downloadLimit = download; r.uploadLimit = upload
        _ = try await perform(r); await refresh()
    }
    func updateTransfer(_ request: EngineRequest) {
        guard let id = request.id, canChangeTransfers, !busyTransfers.contains(id), torrents.contains(where: { $0.id == id }) else { return }
        busyTransfers.insert(id); error = nil
        let storageChange = ["relocate", "rename"].contains(request.command)
        if storageChange { storageRequests.insert(id) }
        Task {
            defer { busyTransfers.remove(id); storageRequests.remove(id) }
            do { _ = try await perform(request) }
            catch is CancellationError { /* A confirmed cancellation preserves original files. */ }
            catch { self.error = error.localizedDescription }
            await refresh()
        }
    }
    func canRetryTransfer(_ item: TorrentItem) -> Bool {
        canChangeTransfers && !busyTransfers.contains(item.id) && torrents.contains { $0.id == item.id && presentation($0).canRetry }
    }
    func retryTransfer(_ item: TorrentItem) {
        guard canRetryTransfer(item) else { return }
        var request = EngineRequest(command:"retry"); request.id = item.id
        updateTransfer(request)
    }
    func canCancelFileOperation(_ operation: FileOperationItem) -> Bool {
        connected && !switching && !designPreview && fileOperation?.id == operation.id &&
            fileOperation?.canCancel == true && operation.canCancel && !cancellingOperations.contains(operation.id)
    }
    func cancelFileOperation(_ operation: FileOperationItem) {
        guard canCancelFileOperation(operation) else { return }
        let id = operation.id
        error = nil
        cancellingOperations.insert(id)
        Task {
            defer { cancellingOperations.remove(id) }
            do {
                var request = EngineRequest(command:"cancelFileOperation"); request.id = id
                _ = try await perform(request)
            } catch { self.error = error.localizedDescription }
            await refresh()
        }
    }
    func resolveStorageRecovery(_ notice: StorageRecoveryItem, keepCopies: Bool) {
        guard canResolveStorageRecovery(notice),
              !keepCopies || notice.canKeepCopies else { return }
        resolvingStorageRecovery = true; error = nil
        Task {
            defer { resolvingStorageRecovery = false }
            do {
                var request = EngineRequest(command:keepCopies ? "keepRecoveryCopies" : "retryStorageRecovery"); request.id = notice.id
                _ = try await perform(request)
            } catch { self.error = error.localizedDescription }
            await refresh()
        }
    }
    func canResolveStorageRecovery(_ notice: StorageRecoveryItem) -> Bool {
        connected && !switching && !designPreview && !hasFileOperation && storageRecovery?.id == notice.id &&
            !torrents.contains { $0.id == notice.torrentId && $0.state == "checking" }
    }
    func setPriority(_ item: TorrentItem, file: FileItem, priority: UInt8) {
        setPriorities(torrentID:item.id,indices:[file.index],priority:priority)
    }
    func setPriorities(torrentID: String, indices: Set<UInt32>, priority: UInt8) {
        guard priority <= 2, !indices.isEmpty,
              let item = torrents.first(where:{ $0.id == torrentID }), item.state != "checking",
              indices.isSubset(of:Set(item.files.map(\.index))) else { return }
        var request = EngineRequest(command:"priorities"); request.id = torrentID
        request.priorities = item.files.map { indices.contains($0.index) ? priority : $0.priority }
        updateTransfer(request)
    }
    func setFilesIncluded(torrentID: String, indices: Set<UInt32>, included: Bool) {
        guard !indices.isEmpty, let item = torrents.first(where:{ $0.id == torrentID }),
              item.state != "checking", indices.isSubset(of:Set(item.files.map(\.index))) else { return }
        let previous = Set(item.files.filter(\.selected).map(\.index))
        let next = included ? previous.union(indices) : previous.subtracting(indices)
        guard next != previous else { return }
        guard !next.isEmpty else { error = "Keep at least one file included."; return }
        setFileSelection(item,indices:Array(next))
    }
    func setQueueLimit(_ value: UInt32) {
        guard canChangeTransfers, !savingQueue else { return }
        savingQueue = true; error = nil
        Task {
            defer { savingQueue = false }
            do { var request = EngineRequest(command:"queue"); request.maxActiveDownloads = value; _ = try await perform(request) }
            catch { self.error = error.localizedDescription }
            await refresh()
        }
    }
    func moveSelectedToFront() {
        guard canActOnSelection, selected.count == 1 else { return }
        moveToFront(selected[0])
    }
    func moveToFront(_ item: TorrentItem) {
        var request = EngineRequest(command:"queueMove"); request.id = item.id; request.queuePosition = 1
        updateTransfer(request)
    }
    static func availableRevealDirectory(_ destination: String) -> URL {
        var folder = URL(fileURLWithPath:destination,isDirectory:true).standardizedFileURL
        while folder.path != "/" {
            var directory = ObjCBool(false)
            if FileManager.default.fileExists(atPath:folder.path,isDirectory:&directory), directory.boolValue { return folder }
            folder.deleteLastPathComponent()
        }
        return folder
    }
    func revealDestination(_ item: TorrentItem) {
        NSWorkspace.shared.selectFile(nil,inFileViewerRootedAtPath:Self.availableRevealDirectory(item.destination).path)
    }
    func chooseRelocation(_ target: TorrentItem? = nil) {
        guard let item = target ?? (selected.count == 1 ? selected.first : nil), canChangeTransfers,
              storageRecovery == nil, !busyTransfers.contains(item.id), item.state != "checking",
              torrents.contains(where: { $0.id == item.id }) else { return }
        let panel = NSOpenPanel(); panel.canChooseFiles = false; panel.canChooseDirectories = true
        panel.canCreateDirectories = true; panel.prompt = "Move Files"; panel.message = "Choose a folder for \(item.name). Files will be verified after moving."
        panel.directoryURL = URL(fileURLWithPath:item.destination)
        panel.begin { response in
            guard response == .OK, let url = panel.url else { return }
            var request = EngineRequest(command:"relocate"); request.id = item.id; request.destination = url.path
            self.updateTransfer(request)
        }
    }
    func requestRename(_ item: TorrentItem, file: FileItem) {
        guard canChangeTransfers, storageRecovery == nil, !busyTransfers.contains(item.id), item.state != "checking", let window = NSApp.keyWindow else { return }
        let previousFocus = window.firstResponder
        let alert = NSAlert(); alert.messageText = "Rename File"; alert.informativeText = "Choose a new name for \(URL(fileURLWithPath:file.name).lastPathComponent)."
        alert.addButton(withTitle:"Rename"); alert.addButton(withTitle:"Cancel")
        let input = NSTextField(string:URL(fileURLWithPath:file.name).lastPathComponent)
        input.frame = NSRect(x:0,y:0,width:320,height:24); input.setAccessibilityLabel("New file name"); alert.accessoryView = input
        alert.window.initialFirstResponder = input
        alert.beginSheetModal(for:window) { response in
            NativeAccessibility.restore(previousFocus,in:window)
            guard response == .alertFirstButtonReturn else { return }
            let name = input.stringValue.trimmingCharacters(in:.whitespacesAndNewlines)
            guard !name.isEmpty, name != ".", name != "..", !name.contains("/"), !name.contains(":") else { self.error = "Enter a file name without slashes or colons."; return }
            let parent = (file.name as NSString).deletingLastPathComponent
            var request = EngineRequest(command:"rename"); request.id = item.id; request.fileIndex = file.index
            request.fileName = parent.isEmpty ? name : parent + "/" + name
            self.updateTransfer(request)
        }
    }
    func setBackground(_ enabled: Bool) async {
        guard !designPreview, !switching, !hasFileOperation, !savingEncryption else { return }
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
        panel.begin { result in if result == .OK, let url = panel.url { self.beginImport(file:url) } }
    }
    func beginImport(magnet: String = "", file: URL? = nil) {
        guard !showImport else { return }
        guard !hasFileOperation else { error = "Wait for the current file change to finish, or cancel it, then add your torrent."; return }
        incomingMagnet = magnet; incomingFile = file; showImport = true
    }
    func pasteMagnet() { beginImport(magnet:NSPasteboard.general.string(forType:.string) ?? "") }
    func open(_ urls: [URL]) {
        guard let url = urls.first else { return }
        if url.scheme?.lowercased() == "magnet" { beginImport(magnet:url.absoluteString) }
        else if url.isFileURL && url.pathExtension.lowercased() == "torrent" { beginImport(file:url) }
        else { error = "Choose a .torrent file or a magnet link to add a transfer." }
    }
}
