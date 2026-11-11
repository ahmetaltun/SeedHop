import SwiftUI
import AppKit
import ServiceManagement
import UserNotifications

enum LibraryFilter: String, CaseIterable, Identifiable {
    case all = "All transfers", downloading = "Downloading", seeding = "Seeding", paused = "Paused", queued = "Queued", checking = "Checking", attention = "Needs attention"
    var id: String { rawValue }
    var title: String {
        switch self {
        case .all: return String(localized:"All transfers",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case .downloading: return String(localized:"Downloading",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case .seeding: return String(localized:"Seeding",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case .paused: return String(localized:"Paused",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case .queued: return String(localized:"Queued",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case .checking: return String(localized:"Checking",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case .attention: return String(localized:"Needs attention",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        }
    }
    var symbol: String {
        switch self { case .all:return "square.stack.3d.up"; case .downloading:return "arrow.down.circle"; case .seeding:return "arrow.up.circle"; case .paused:return "pause.circle"; case .queued:return "list.number"; case .checking:return "checkmark.shield"; case .attention:return "exclamationmark.circle" }
    }
    var emptyTitle: String {
        switch self {
        case .all: return String(localized:"No transfers yet",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case .downloading: return String(localized:"No downloads in progress",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case .seeding: return String(localized:"No transfers seeding",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case .paused: return String(localized:"No paused transfers",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case .queued: return String(localized:"No queued transfers",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case .checking: return String(localized:"No transfers being checked",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case .attention: return String(localized:"Everything looks good",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
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
    @Published private(set) var clearingActivity = false
    @Published private(set) var statisticsRetention: StatisticsRetentionInfo?
    @Published private(set) var savingStatisticsRetention = false
    private var activityEpoch: UInt64 = 0
    private var activityClearedThrough: UInt64?
    private static let activityRetention: TimeInterval = 15 * 60
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
    var externalOpenCoordinator: ExternalOpenCoordinator?
    @Published var showDockSpeeds = WorkspacePreferences.defaults.object(forKey:"showDockSpeeds") as? Bool ?? true {
        didSet { WorkspacePreferences.defaults.set(showDockSpeeds,forKey:"showDockSpeeds") }
    }
    var shouldPollSnapshots: Bool { showDockSpeeds || NSApp.windows.contains(where:{ $0.isVisible && $0.title == "SeedHop" }) }
    @Published var showImport = false
    @Published var showInspector = WorkspacePreferences.defaults.object(forKey:"showTransferDetails") as? Bool ?? true { didSet { WorkspacePreferences.defaults.set(showInspector,forKey:"showTransferDetails") } }
    @Published var incomingMagnet = ""
    @Published var incomingFile: URL?
    @Published var showRemoval = false
    @Published var removalDeletesFiles = false
    @Published private(set) var removalTargets: [TorrentItem] = []
    @Published private(set) var busyTransfers = Set<String>()
    @Published private(set) var reannouncing = Set<String>()
    @Published private(set) var reannounceNotice: String?
    @Published var fileOperation: FileOperationItem? { didSet { rebuildCounts() } }
    @Published var storageRecovery: StorageRecoveryItem? { didSet { rebuildCounts() } }
    @Published private(set) var resolvingStorageRecovery = false
    @Published private(set) var storageRequests = Set<String>()
    @Published private(set) var cancellingOperations = Set<String>()
    @Published var schedule: ScheduleConfig?
    @Published var alternativeDownloadLimit: UInt32 = 0
    @Published var alternativeUploadLimit: UInt32 = 0
    @Published var alternativeLimitsEnabled = false
    @Published var alternativeLimitsAvailable = false
    @Published var savingBandwidth = false
    @Published var scheduleActive = false
    @Published var schedulingWarning: String?
    @Published var normalDownloadLimit: UInt32 = 0
    @Published var normalUploadLimit: UInt32 = 0
    @Published var savingSchedule = false
    @Published var downloadLimit: UInt32 = 0
    @Published var uploadLimit: UInt32 = 0
    @Published var maxActiveDownloads: UInt32 = 0
    @Published var savingQueue = false
    @Published var peerEncryption: PeerEncryptionPolicy?
    @Published var savedPeerEncryption: PeerEncryptionPolicy?
    @Published private(set) var savingEncryption = false
    @Published var networkBlocked = false
    @Published var networkInterface: String?
    @Published var networkInfo: NetworkInfo?
    @Published var savingNetwork = false
    @Published var listenPort: UInt16 = 0
    @Published var backgroundEnabled = UserDefaults.standard.bool(forKey:"backgroundEnabled")
    @Published var resumeAtLogin = UserDefaults.standard.bool(forKey:"resumeAtLogin")
    @Published var downloadDirectory = UserDefaults.standard.string(forKey:"downloadDirectory") ?? FileManager.default.urls(for:.downloadsDirectory,in:.userDomainMask)[0].path
    @Published var notificationDetails = UserDefaults.standard.bool(forKey:"notificationDetails")
    @Published var completionNotifications = UserDefaults.standard.object(forKey:"completionNotifications") as? Bool ?? false
    private(set) var updatePrepared = false
    let designPreview: Bool
    let isPrivate: Bool
    let client: any EngineTransport
    // Derived from the current snapshot, never a second source of transfer state.
    private var torrentIndices: [String:Int] = [:]
    private(set) var torrentGeneration: UInt64 = 0
    private var filterCounts: [LibraryFilter:Int] = [:]
    private var stateCounts: [String:Int] = [:]
    private var filteredTorrents: [TorrentItem]?
    private var snapshotTrackerHosts: [String] = []
    private var polling: Task<Void,Never>?
    private var reconnect: Task<Void,Never>?
    // Notification bookkeeping lasts only as long as this app and active library.
    private var notifiedCompletions: Set<String> = []

    init(designPreview: Bool = false, client: (any EngineTransport)? = nil, isPrivate: Bool = false) {
        self.designPreview = designPreview
        self.isPrivate = isPrivate
        self.client = client ?? EngineClient(privateSession:isPrivate)
        if designPreview { peerEncryption = .allow; savedPeerEncryption = .allow; torrents = PreviewFixtures.torrents; reindexTorrents(); selection = [torrents[0].id]; details = PreviewFixtures.diagnostics(torrents[0],section:"general"); history[torrents[0].id] = PreviewFixtures.trafficHistory(); connected = true; return }
        self.client.onInterruption = { [weak self] in
            guard let self, !self.switching else { return }
            self.connected = false
            if self.isPrivate {
                self.error = EngineMessage(code:"host.stopping").localizedDescription
                return
            }
            self.reconnect?.cancel()
            self.reconnect = Task { try? await Task.sleep(for:.seconds(2)); if !Task.isCancelled { await self.start() } }
        }
    }
    private func reindexTorrents() {
        torrentGeneration &+= 1
        torrentIndices.removeAll(keepingCapacity:true)
        torrentIndices.reserveCapacity(torrents.count)
        for index in torrents.indices { torrentIndices[torrents[index].id] = index }
        snapshotTrackerHosts = Set(torrents.flatMap { $0.trackers.compactMap { URL(string:$0)?.host?.lowercased() } }).sorted()
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
        item.state == "error" ? presentation(item).canRetry : (item.isPaused || item.forceStarted == true) && item.state != "checking"
    }
    var canResumeSelection: Bool { canActOnSelection && selected.contains(where:canResumeOrRetry) }
    var canPauseSelection: Bool { canActOnSelection && selected.contains { !$0.isPaused || $0.forceStarted == true } }
    func canSetForceStarted(ids: Set<String>, enabled: Bool) -> Bool {
        let items = torrents.filter { ids.contains($0.id) }
        return canChangeTransfers && !ids.isEmpty && items.count == ids.count && busyTransfers.isDisjoint(with:ids)
            && (!enabled || items.allSatisfy { $0.state != "error" && $0.state != "checking" && !hasPendingCleanup($0.id) })
    }
    var selectionForceStarted: Bool { !selected.isEmpty && selected.allSatisfy { $0.forceStarted == true } }
    func hasPendingCleanup(_ id: String) -> Bool { storageRecovery.map { $0.torrentId.isEmpty || $0.torrentId == id } ?? false }
    func canReannounce(_ item: TorrentItem) -> Bool {
        canChangeTransfers && !engineInstance.isEmpty && item.canReannounce == true
            && ["downloading", "waiting", "seeding"].contains(item.state) && !item.isPaused
            && !hasPendingCleanup(item.id) && !busyTransfers.contains(item.id) && !reannouncing.contains(item.id)
    }
    var canReannounceSelection: Bool { selected.contains(where:canReannounce) }
    func canReannounceTracker(_ tracker: TrackerDetails, torrent: TorrentItem) -> Bool {
        canChangeTransfers && !engineInstance.isEmpty && ["downloading", "waiting", "seeding"].contains(torrent.state)
            && !torrent.isPaused && !hasPendingCleanup(torrent.id) && !busyTransfers.contains(torrent.id)
            && !reannouncing.contains(torrent.id) && tracker.reannounceTargetId != nil
            && tracker.reannouncePending != true && !["contacting", "resolving"].contains(tracker.state)
    }
    func reannounceSelected() { reannounce(items:selected) }
    func reannounceTorrent(_ torrent: TorrentItem) { reannounce(items:[torrent]) }
    func reannounceTrackers(torrent: TorrentItem, targetIds: Set<String>) {
        guard !targetIds.isEmpty else { return }
        reannounce(items:[torrent],targetIds:Array(targetIds).sorted())
    }
    private func reannounce(items: [TorrentItem], targetIds: [String]? = nil) {
        guard canChangeTransfers, !engineInstance.isEmpty else { return }
        let instance = engineInstance
        let eligible = items.filter { item in
            if targetIds != nil {
                return !item.isPaused && ["downloading", "waiting", "seeding"].contains(item.state)
                    && !hasPendingCleanup(item.id) && !busyTransfers.contains(item.id) && !reannouncing.contains(item.id)
            }
            return canReannounce(item)
        }
        guard !eligible.isEmpty else { return }
        let ids = Set(eligible.map(\.id))
        reannouncing.formUnion(ids); reannounceNotice = nil
        Task {
            defer { if engineInstance == instance { reannouncing.subtract(ids) } }
            var scheduled = 0, pending = 0, unavailable = 0, skipped = items.count - eligible.count
            var failure: String?
            for item in eligible {
                guard engineInstance == instance, connected, !switching else { return }
                do {
                    var request = EngineRequest(command:"reannounce")
                    request.id = item.id; request.targetIds = targetIds; request.engineInstanceId = instance
                    guard let summary = try await perform(request).reannounce else {
                        throw UserFacingError(message:String(localized:"Tracker updates are unavailable for this transfer.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                    }
                    scheduled += Int(summary.scheduled); pending += Int(summary.alreadyPending); unavailable += Int(summary.unavailable)
                } catch { skipped += 1; failure = error.localizedDescription }
            }
            guard engineInstance == instance else { return }
            reannounceNotice = String(localized:"Tracker updates: scheduled \(scheduled), pending \(pending), unavailable \(unavailable). Transfers skipped: \(skipped).",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
            if let failure { error = failure }
            await refresh()
        }
    }
    var canRecheckSelection: Bool { canActOnSelection && selected.allSatisfy { $0.state != "checking" && !hasPendingCleanup($0.id) } }
    var canMoveSelection: Bool { canRecheckSelection && storageRecovery == nil && selected.count == 1 }
    var canRemoveSelection: Bool { canActOnSelection && selected.allSatisfy { !hasPendingCleanup($0.id) } }
    func resumeLabel(for items: [TorrentItem]) -> String {
        let retry = items.contains { $0.state == "error" && presentation($0).canRetry }
        let resume = items.contains { $0.state != "error" && canResumeOrRetry($0) }
        return retry ? (resume ? String(localized:"Resume and Retry",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Retry",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) : String(localized:"Resume",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
    }
    var resumeActionLabel: String { resumeLabel(for:selected) }
    var selectionNeedsRetry: Bool { selected.contains { $0.state == "error" && presentation($0).canRetry } }
    var resumeSelectedLabel: String {
        let retry = selectionNeedsRetry
        let resume = selected.contains { $0.state != "error" && canResumeOrRetry($0) }
        return retry ? (resume ? String(localized:"Resume and Retry Selected",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Retry Selected",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) : String(localized:"Resume Selected",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
    }
    var hasUnscopedRecovery: Bool { storageRecovery.map { notice in torrentIndices[notice.torrentId] == nil } ?? false }
    var activitySummary: String {
        if !connected { return String(localized:"Showing the last update. Reconnect for live activity.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
        if torrents.isEmpty { return storageRecovery == nil ? String(localized:"A fresh start for your next download.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Retained files need attention.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
        var parts = ["downloading", "waiting", "queued", "scheduled", "checking", "seeding", "paused"].compactMap { state -> String? in
            let count = stateCounts[state,default:0]
            guard count > 0 else { return nil }
            switch state {
            case "downloading": return String(localized:"\(count) downloading",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
            case "waiting": return String(localized:"\(count) waiting",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
            case "queued": return String(localized:"\(count) queued",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
            case "scheduled": return String(localized:"\(count) scheduled",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
            case "checking": return String(localized:"\(count) checking",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
            case "seeding": return String(localized:"\(count) seeding",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
            default: return String(localized:"\(count) paused",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
            }
        }
        if fileOperation != nil { parts.insert(String(localized:"1 file change",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),at:0) }
        let attention = count(in:.attention)
        if attention > 0 { parts.append(String(localized:"\(attention) need attention",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) }
        else if hasUnscopedRecovery { parts.append(String(localized:"File cleanup needs attention",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) }
        return parts.isEmpty ? String(localized:"Waiting for a transfer update.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : parts.joined(separator:" · ")
    }
    var hasFileOperation: Bool { fileOperation != nil || !storageRequests.isEmpty || resolvingStorageRecovery }
    var canChangeTransfers: Bool { connected && !networkBlocked && !savingNetwork && !switching && !designPreview && !hasFileOperation }

    var encryptionRestartPending: Bool { networkBlocked || (peerEncryption != nil && savedPeerEncryption != nil && peerEncryption != savedPeerEncryption) }
    var canSaveEncryption: Bool { canChangeTransfers && !savingEncryption && savedPeerEncryption != nil }
    var canRestartForEncryption: Bool { connected && !switching && !designPreview && !hasFileOperation && encryptionRestartPending && !savingEncryption && !savingSchedule && !savingBandwidth && busyTransfers.isEmpty && !savingQueue && !showImport }
    var connectionPortDescription: String { !connected ? String(localized:"Unavailable while disconnected",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : listenPort == 0 ? String(localized:"No listening port reported",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(listenPort) }
    func refreshNetworkInfo() async {
        guard !designPreview, !switching else { return }
        do {
            let next = try await client.send(EngineRequest(command:"networkInfo")).networkInfo
            if networkInfo != next { networkInfo = next }
            if let info = networkInfo {
                let blocked = info.blocked && (info.active != nil || info.saved == nil || info.saved?.interface != nil)
                if networkBlocked != blocked { networkBlocked = blocked }
            }
        } catch { self.error = error.localizedDescription }
    }
    func saveNetworkPolicy(_ policy: NetworkPolicy) {
        guard !savingNetwork, !switching, !hasFileOperation, !showImport, !savingEncryption, !savingSchedule, !savingBandwidth, !savingQueue, busyTransfers.isEmpty else { return }
        savingNetwork = true; error = nil
        Task {
            defer { savingNetwork = false }
            do {
                var request = EngineRequest(command:"networkPolicy"); request.networkPolicy = policy
                networkInfo = try await client.send(request).networkInfo
                networkBlocked = networkInfo?.blocked ?? true
                if connected { restartForEncryption() }
                else { await start() }
            } catch {
                self.error = error.localizedDescription
                await refreshNetworkInfo()
                if let info = networkInfo { networkBlocked = info.blocked }
            }
        }
    }
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
        guard !isPrivate, !designPreview, !connected, !connecting, !switching, !clearingActivity, !savingStatisticsRetention else { return }
        #if SEEDHOP_SANDBOX && !SEEDHOP_VALIDATION
        guard SandboxLibraryMigration.isSetUp else { return }
        #endif
        connecting = true; defer { connecting = false }
        if UserDefaults.standard.bool(forKey:"updateRestoreBackground") {
            let service = SMAppService.agent(plistName:SeedHopIdentity.agentPlist)
            do {
                if service.status == .notRegistered || service.status == .notFound { try service.register() }
                guard service.status == .enabled else { throw UserFacingError(message:String(localized:"Allow SeedHop in System Settings → General → Login Items & Extensions, then try again.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) }
                backgroundEnabled = true
                UserDefaults.standard.removeObject(forKey:"updateRestoreBackground")
            } catch {
                // Keep the saved preference and marker so a later launch can retry.
                backgroundEnabled = false
                self.error = error.localizedDescription
            }
        }
        do {
            let service = SMAppService.agent(plistName:SeedHopIdentity.agentPlist)
            if backgroundEnabled && service.status != .enabled {
                backgroundEnabled = false; UserDefaults.standard.set(false,forKey:"backgroundEnabled")
                error = String(localized:"Background access is unavailable. Transfers will run while SeedHop is open. You can enable background access in Settings.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
            }
            try client.connect(background:backgroundEnabled)
            var request = EngineRequest(command:"connect"); request.destination = downloadDirectory
            let reply = try await client.send(request)
            if let snapshot = reply.snapshot { apply(snapshot) }
            connected = true
            beginPolling()
        } catch { connected = false; self.error = error.localizedDescription }
    }
    func privateProtection() async throws -> String {
        let transport: any EngineTransport
        if connected { transport = client }
        else {
            let offline = EngineClient()
            try offline.connect(background:false)
            transport = offline
        }
        defer { if transport !== client { transport.invalidate() } }
        guard let policy = try await transport.send(EngineRequest(command:"privateProtection")).privateProtection else {
            throw EngineMessage(code:"host.unsupported")
        }
        return policy
    }
    func startPrivate(folder: URL, protection: String) async throws -> Int32 {
        let scoped = folder.startAccessingSecurityScopedResource()
        defer { if scoped { folder.stopAccessingSecurityScopedResource() } }
        #if SEEDHOP_SANDBOX
        guard scoped else { throw EngineMessage(code:"host.folder_access") }
        #endif
        downloadDirectory = folder.path
        try client.connect(background:false)
        var request = EngineRequest(command:"connectPrivate")
        request.privateFolder = try folder.bookmarkData(options:[],includingResourceValuesForKeys:nil,relativeTo:nil)
        request.privateProtection = protection
        let reply = try await client.send(request)
        guard let pid = reply.privateProcessID, let snapshot = reply.snapshot else {
            throw EngineMessage(code:"host.unsupported")
        }
        apply(snapshot)
        connected = true
        beginPolling()
        return pid
    }
    func endPrivate(pid: Int32) async throws {
        switching = true
        defer { switching = false }
        polling?.cancel()
        if kill(pid, 0) == 0 || errno != ESRCH {
            // Even if the acknowledgement is lost, only a confirmed exit ends the session.
            _ = try? await client.send(EngineRequest(command:"shutdown"))
            try await Self.waitForEngineExit(pid)
        }
        client.invalidate()
        connected = false
    }
    #if SEEDHOP_SANDBOX
    func stopForLibraryMigration() async throws {
        guard connected else { return }
        guard canPrepareForUpdate else { throw EngineMessage(code:"host.requests_busy") }
        switching = true
        defer { switching = false }
        let reply = try await client.send(EngineRequest(command:"shutdown"))
        guard let pid = reply.stoppedProcessID else { throw EngineMessage(code:"host.stopping") }
        client.invalidate(); connected = false
        polling?.cancel(); reconnect?.cancel()
        try await Self.waitForEngineExit(pid)
    }
    #endif
    private func beginPolling() {
        polling?.cancel()
        polling = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for:.seconds(1))
                guard !Task.isCancelled, let self else { break }
                self.expireActivity()
                if self.isPrivate || self.shouldPollSnapshots { await self.refresh() }
            }
        }
    }
    func refresh() async {
        guard !designPreview, connected, !switching else { return }
        do {
            guard let snapshot = try await client.send(EngineRequest(command:"snapshot")).snapshot else {
                throw UserFacingError(message:String(localized:"Could not reach the torrent engine.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
            }
            apply(snapshot)
        }
        catch { connected = false; self.error = error.localizedDescription }
    }
    private func apply(_ snapshot: LibrarySnapshot) {
        let engineChanged = engineInstance != snapshot.instanceId
        if engineChanged {
            notifiedCompletions.removeAll()
            engineInstance = snapshot.instanceId; reannounceNotice = nil; reannouncing = []; detailsContext = UUID(); details = nil; peerBaselines = [:]; peerRates = [:]; lastTrackerEvent = [:]
        }
        let statisticsText = snapshot.statisticsMessage?.displayText ?? snapshot.statisticsWarning.map { EngineMessage(code:"statistics.save_failed",diagnostic:$0).displayText }
        if statisticsWarning != statisticsText { statisticsWarning = statisticsText }
        recordActivity(snapshot)

        let permitsNotifications = !isPrivate && !designPreview && !WorkspacePreferences.isFixture
        if permitsNotifications, UserDefaults.standard.object(forKey:"completedTorrents") != nil {
            // Older builds retained torrent identifiers indefinitely. Do not read
            // or migrate that history into the new session-only bookkeeping.
            UserDefaults.standard.removeObject(forKey:"completedTorrents")
        }
        notifiedCompletions.formIntersection(snapshot.torrents.map(\.id))
        if permitsNotifications, completionNotifications, !clearingActivity, !engineChanged, !torrents.isEmpty {
            for item in snapshot.torrents where item.state == "seeding" && !notifiedCompletions.contains(item.id) {
                if let previous = torrents.first(where:{$0.id == item.id}), previous.downloadedBytes < previous.totalBytes {
                    let content = UNMutableNotificationContent(); content.title = String(localized:"Your files are ready",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale); content.body = notificationDetails ? item.name : String(localized:"Open SeedHop to view the completed transfer.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale); content.sound = .default
                    UNUserNotificationCenter.current().add(UNNotificationRequest(identifier:item.id,content:content,trigger:nil))
                    notifiedCompletions.insert(item.id)
                }
            }
        }
        // Avoid republishing identical rows; SwiftUI Table retains stable identity/selection.
        if torrents != snapshot.torrents { torrents = snapshot.torrents }
        let remainingSelection = selection.intersection(Set(torrents.map(\.id)))
        if selection != remainingSelection { selection = remainingSelection }
        let schedulingText = snapshot.schedulingMessage?.displayText ?? snapshot.schedulingWarning.map { EngineMessage(code:"schedule.apply_failed",diagnostic:$0).displayText }
        if schedulingWarning != schedulingText { schedulingWarning = schedulingText }
        if schedule != snapshot.schedule { schedule = snapshot.schedule }
        if scheduleActive != (snapshot.scheduleActive ?? false) { scheduleActive = snapshot.scheduleActive ?? false }
        if alternativeLimitsAvailable != (snapshot.alternativeLimitsEnabled != nil) { alternativeLimitsAvailable = snapshot.alternativeLimitsEnabled != nil }
        if alternativeDownloadLimit != (snapshot.alternativeDownloadLimit ?? 0) { alternativeDownloadLimit = snapshot.alternativeDownloadLimit ?? 0 }
        if alternativeUploadLimit != (snapshot.alternativeUploadLimit ?? 0) { alternativeUploadLimit = snapshot.alternativeUploadLimit ?? 0 }
        if alternativeLimitsEnabled != (snapshot.alternativeLimitsEnabled ?? false) { alternativeLimitsEnabled = snapshot.alternativeLimitsEnabled ?? false }
        let normalDown = snapshot.normalDownloadLimit ?? snapshot.downloadLimit
        let normalUp = snapshot.normalUploadLimit ?? snapshot.uploadLimit
        if normalDownloadLimit != normalDown { normalDownloadLimit = normalDown }
        if normalUploadLimit != normalUp { normalUploadLimit = normalUp }
        if downloadLimit != snapshot.downloadLimit { downloadLimit = snapshot.downloadLimit }
        if uploadLimit != snapshot.uploadLimit { uploadLimit = snapshot.uploadLimit }
        if networkBlocked != (snapshot.networkBlocked ?? false) { networkBlocked = snapshot.networkBlocked ?? false }
        if networkInterface != snapshot.networkInterface { networkInterface = snapshot.networkInterface }
        if peerEncryption != snapshot.peerEncryption { peerEncryption = snapshot.peerEncryption }
        if savedPeerEncryption != snapshot.savedPeerEncryption { savedPeerEncryption = snapshot.savedPeerEncryption }
        if listenPort != snapshot.listenPort { listenPort = snapshot.listenPort }
        if maxActiveDownloads != snapshot.maxActiveDownloads { maxActiveDownloads = snapshot.maxActiveDownloads }
        if fileOperation != snapshot.fileOperation { fileOperation = snapshot.fileOperation }
        if storageRecovery != snapshot.storageRecovery { storageRecovery = snapshot.storageRecovery }
    }
    var trackerHosts: [String] {
        guard !trackerHost.isEmpty, !snapshotTrackerHosts.contains(trackerHost) else { return snapshotTrackerHosts }
        return (snapshotTrackerHosts + [trackerHost]).sorted()
    }
    func clearFilters() { search = ""; trackerHost = ""; filter = .all }
    private func expireActivity(at date: Date = Date()) {
        guard !designPreview, !WorkspacePreferences.isFixture else { return }
        let cutoff = date.addingTimeInterval(-Self.activityRetention)
        for id in Array(history.keys) {
            let samples = history[id] ?? []
            if let first = samples.first, let last = samples.last,
               first.id > cutoff, last.id <= date { continue }
            let retained = samples.filter { $0.id > cutoff && $0.id <= date }
            if retained.count != samples.count {
                objectWillChange.send()
                if retained.isEmpty { history.removeValue(forKey:id) }
                else { history[id] = retained }
            }
        }
        for id in Array(events.keys) {
            let rows = events[id] ?? []
            if let first = rows.first, let last = rows.last,
               first.date > cutoff, last.date <= date { continue }
            let retained = rows.filter { $0.date > cutoff && $0.date <= date }
            if retained.count != rows.count {
                if retained.isEmpty { events.removeValue(forKey:id) }
                else { events[id] = retained }
            }
        }
        if var current = details, let rows = current.events {
            current.events = rows.filter {
                let timestamp = Date(timeIntervalSince1970:Double($0.timestamp))
                return timestamp > cutoff && timestamp <= date
            }
            if current != details { details = current }
        }
    }
    private func recordActivity(_ snapshot: LibrarySnapshot) {
        guard !clearingActivity else { return }
        let date = Date()
        expireActivity(at:date)
        let liveIDs = Set(snapshot.torrents.map(\.id))
        for item in snapshot.torrents {
            if let previous = torrentIndices[item.id].map({ torrents[$0] }), previous.state != item.state || previous.errorKind != item.errorKind {
                appendEvent(item.id, message:item.statusLabel)
            }
            // Sample only real traffic changes or elapsed seconds, never every body read.
            if history[item.id]?.last.map({ date.timeIntervalSince($0.id) >= 1 }) ?? true {
                var samples = history[item.id] ?? []
                samples.append(TrafficSample(id:date,download:item.downloadRate,upload:item.uploadRate))
                samples.removeAll { date.timeIntervalSince($0.id) >= Self.activityRetention }
                if samples.count > 900 { samples.removeFirst(samples.count - 900) }
                history[item.id] = samples
            }
        }
        if snapshot.fileOperation != fileOperation, let operation = snapshot.fileOperation {
            appendEvent(operation.torrentId,message:operation.title)
        }
        if snapshot.storageRecovery != storageRecovery, let notice = snapshot.storageRecovery {
            appendEvent(notice.torrentId,message:String(localized:"File cleanup needs attention",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
        }
        if !Set(history.keys).isSubset(of:liveIDs) { history = history.filter { liveIDs.contains($0.key) } }
        if !Set(events.keys).isSubset(of:liveIDs) { events = events.filter { liveIDs.contains($0.key) } }
    }
    private func appendEvent(_ id: String, message: String) {
        guard !clearingActivity else { return }
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
                let eventEpoch = activityEpoch
                do {
                    var request = EngineRequest(command:"torrentDetails"); request.id = id; request.section = section; request.allPeers = allPeers
                    let reply = try await client.send(request)
                    guard let value = reply.details else { throw UserFacingError(message:String(localized:"The engine did not return transfer details. Try again.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) }
                    if !Task.isCancelled && !clearingActivity && eventEpoch == activityEpoch && detailsContext == token && instance == engineInstance && showInspector && selection == [id], value.torrentId == id && value.instanceId == instance && value.section == section {
                        applyDetails(value); if detailsError != nil { detailsError = nil }
                    }
                } catch {
                    if !Task.isCancelled && !clearingActivity && eventEpoch == activityEpoch && detailsContext == token && detailsError != error.localizedDescription { detailsError = error.localizedDescription }
                }
                detailsInFlight = false
            }
            do { try await Task.sleep(for:.seconds(1)) } catch { break }
        }
    }
    private func applyDetails(_ value: TorrentDetails) {
        var value = value
        let date = Date()
        expireActivity(at:date)
        let cutoff = date.addingTimeInterval(-Self.activityRetention)
        value.events = value.events?.filter {
            let timestamp = Date(timeIntervalSince1970:Double($0.timestamp))
            return timestamp > cutoff && timestamp <= date
        }
        if let cutoff = activityClearedThrough { value.events = value.events?.filter { $0.timestamp > cutoff } }
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
    #if SEEDHOP_DEVELOPMENT
    private func sendStatisticsRequest(_ request: EngineRequest) async throws -> EngineReply {
        if connected { return try await client.send(request) }
        // A temporary foreground helper can edit library policy without invoking
        // a background agent's automatic network restoration.
        let offline = EngineClient()
        try offline.connect(background: false)
        defer { offline.invalidate() }
        return try await offline.send(request)
    }

    func refreshStatisticsRetention() async {
        guard !designPreview, !WorkspacePreferences.isFixture, !switching, !connecting, !savingStatisticsRetention else { return }
        do {
            let value = try await sendStatisticsRequest(EngineRequest(command:"statisticsInfo")).statisticsRetention
            if statisticsRetention != value { statisticsRetention = value }
        } catch { statisticsRetention = nil; self.error = error.localizedDescription }
    }

    func saveStatisticsRetention(_ remember: Bool) async {
        guard !designPreview, !WorkspacePreferences.isFixture, !switching, !connecting,
              !savingStatisticsRetention, !clearingActivity, !hasFileOperation,
              !savingNetwork, !savingEncryption else { return }
        savingStatisticsRetention = true
        error = nil
        defer { savingStatisticsRetention = false }
        var request = EngineRequest(command:"statisticsRetention"); request.rememberStatistics = remember
        do {
            let value = try await sendStatisticsRequest(request).statisticsRetention
            if statisticsRetention != value { statisticsRetention = value }
        } catch {
            self.error = error.localizedDescription
            // A durable choice can precede a failed purge. Reflect the readable
            // pending state instead of showing an optimistic success or stale toggle.
            if let reply = try? await sendStatisticsRequest(EngineRequest(command:"statisticsInfo")) {
                statisticsRetention = reply.statisticsRetention
            } else { statisticsRetention = nil }
        }
        if connected { await refresh() }
    }

    var canClearLocalActivity: Bool {
        !designPreview && !WorkspacePreferences.isFixture && !clearingActivity &&
            !savingStatisticsRetention && !connecting && !switching && !hasFileOperation && busyTransfers.isEmpty
    }

    /// Clear volatile activity and app-owned presentation records. A failed engine
    /// acknowledgement is reported separately from successful local clearing.
    func clearLocalActivity() async -> Bool? {
        guard canClearLocalActivity else { return nil }
        let hasConnectedEngine = connected
        clearingActivity = true
        activityEpoch &+= 1
        activityClearedThrough = UInt64(max(0, Date().timeIntervalSince1970))
        history = [:]; events = [:]; details = nil; detailsError = nil
        peerRates = [:]; peerBaselines = [:]
        notifiedCompletions.removeAll()
        UserDefaults.standard.removeObject(forKey:"completedTorrents")
        reconnect?.cancel(); reconnect = nil
        NSDocumentController.shared.clearRecentDocuments(nil)
        UNUserNotificationCenter.current().removeAllDeliveredNotifications()
        UNUserNotificationCenter.current().removeAllPendingNotificationRequests()
        defer {
            activityEpoch &+= 1
            if !hasConnectedEngine {
                reconnect?.cancel(); reconnect = nil
            }
            clearingActivity = false
        }
        // An offline background connection can activate login restoration. Do
        // not open one for privacy clearing; report the engine state unconfirmed.
        guard hasConnectedEngine else { return false }
        do {
            _ = try await client.send(EngineRequest(command:"clearActivity"))
            return true
        } catch { return false }
    }
    #endif

    func diagnosticReport(_ torrent: TorrentItem) -> String {
        // Export only allowlisted fields. Remote strings, paths, hashes and endpoints stay out.
        let value = details?.torrentId == torrent.id ? details : nil
        let report = "SeedHop diagnostics\nState: \(torrent.state)\nEngine connected: \(connected)\nFormat: \(value?.format ?? "unknown")\nConnected peers: \(torrent.peers)\nDownload: \(SizeText.rate(connected ? torrent.downloadRate : 0))\nUpload: \(SizeText.rate(connected ? torrent.uploadRate : 0))\nTracker states: \(value?.trackers.map(\.state).joined(separator:", ") ?? "not sampled")\nPersonal paths, torrent identifiers, tracker URLs and peer endpoints omitted."
        return report
    }
    func copyDiagnostics(_ torrent: TorrentItem) {
        NSPasteboard.general.clearContents(); NSPasteboard.general.setString(diagnosticReport(torrent),forType:.string)
    }

    func perform(_ request: EngineRequest) async throws -> EngineReply {
        guard !designPreview else { throw UserFacingError(message:String(localized:"This is a design preview with sample data. Launch the app normally for real transfers.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) }
        if hasFileOperation && ["add", "inspect", "pause", "forceStart", "select", "priorities", "queue", "queueMove", "reannounce", "recheck", "retry", "remove", "limits", "peerEncryption"].contains(request.command) {
            throw UserFacingError(message:String(localized:"Wait for the current file change to finish, or cancel it, then try again.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
        }
        return try await client.send(request)
    }
    func toggleSelected(paused: Bool) {
        guard canActOnSelection else { return }
        let items = selected.filter { paused ? (!$0.isPaused || $0.forceStarted == true) : canResumeOrRetry($0) }
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
        removalDeletesFiles = false
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
    func saveSchedule(_ value: ScheduleConfig) async throws {
        guard canChangeTransfers, !savingSchedule else { throw UserFacingError(message:String(localized:"Wait for the engine, then save the schedule again.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) }
        savingSchedule = true
        defer { savingSchedule = false }
        var request = EngineRequest(command:"schedule"); request.schedule = value
        do { _ = try await perform(request); await refresh() }
        catch { await refresh(); throw error }
    }
    var hasAlternativeLimits: Bool { alternativeDownloadLimit > 0 || alternativeUploadLimit > 0 }
    var canToggleAlternativeLimits: Bool { canChangeBandwidth && alternativeLimitsAvailable && (hasAlternativeLimits || alternativeLimitsEnabled) }
    var bandwidthProfileTitle: String { alternativeLimitsEnabled ? String(localized:"Alternative",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : scheduleActive ? String(localized:"Scheduled",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Global",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
    var bandwidthStatus: String {
        let down = SizeText.limit(downloadLimit), up = SizeText.limit(uploadLimit)
        if !connected {
            if alternativeLimitsEnabled { return String(localized:"Last known alternative limits · ↓ \(down) · ↑ \(up)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
            return scheduleActive ? String(localized:"Last known scheduled limits · ↓ \(down) · ↑ \(up)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Last known global limits · ↓ \(down) · ↑ \(up)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        }
        if alternativeLimitsEnabled { return String(localized:"Alternative limits · ↓ \(down) · ↑ \(up)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
        return scheduleActive ? String(localized:"Scheduled limits · ↓ \(down) · ↑ \(up)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Global limits · ↓ \(down) · ↑ \(up)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
    }
    var alternativeLimitsHint: String {
        hasAlternativeLimits ? String(localized:"Alternative limits: download \(SizeText.limit(alternativeDownloadLimit)), upload \(SizeText.limit(alternativeUploadLimit)).",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Alternative limits are not set. Click the tortoise to set a download or upload limit.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
    }
    var canChangeBandwidth: Bool { canChangeTransfers && !savingBandwidth }
    func setAlternativeLimits(download: UInt32, upload: UInt32, enabled: Bool) async throws {
        guard canChangeBandwidth, alternativeLimitsAvailable else { throw UserFacingError(message:String(localized:"Wait for the engine, then apply alternative limits again.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) }
        guard !enabled || download > 0 || upload > 0 else { throw UserFacingError(message:String(localized:"Set an alternative download or upload limit before enabling alternative limits.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) }
        savingBandwidth = true
        defer { savingBandwidth = false }
        var request = EngineRequest(command:"alternativeLimits")
        request.downloadLimit = download; request.uploadLimit = upload; request.alternativeLimitsEnabled = enabled
        do { _ = try await perform(request); await refresh() }
        catch { await refresh(); throw error }
    }
    func toggleAlternativeLimits(_ enabled: Bool) {
        guard canToggleAlternativeLimits else { return }
        Task {
            do { try await setAlternativeLimits(download:alternativeDownloadLimit,upload:alternativeUploadLimit,enabled:enabled) }
            catch { self.error = error.localizedDescription }
        }
    }
    func setLimits(download: UInt32, upload: UInt32) async throws {
        guard canChangeBandwidth else { throw UserFacingError(message:String(localized:"Wait for the engine, then apply limits again.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) }
        savingBandwidth = true
        defer { savingBandwidth = false }
        guard !hasFileOperation else { throw UserFacingError(message:String(localized:"Wait for the current file change to finish, then apply limits again.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) }
        var r = EngineRequest(command:"limits"); r.downloadLimit = download; r.uploadLimit = upload
        _ = try await perform(r); await refresh()
    }
    func setPiecePreferences(ids: Set<String>, sequential: Bool? = nil, firstLastPieces: Bool? = nil) {
        guard canChangeTransfers, !ids.isEmpty, sequential != nil || firstLastPieces != nil,
              ids.isSubset(of:Set(torrents.map(\.id))), busyTransfers.isDisjoint(with:ids) else { return }
        busyTransfers.formUnion(ids); error = nil
        Task {
            defer { busyTransfers.subtract(ids) }
            var failures: [String] = []
            for id in ids.sorted() {
                var request = EngineRequest(command:"piecePreferences"); request.id = id
                request.sequential = sequential; request.firstLastPieces = firstLastPieces
                do { _ = try await perform(request) }
                catch { failures.append(error.localizedDescription) }
            }
            self.error = failures.isEmpty ? nil : failures.joined(separator:"\n")
            await refresh()
        }
    }
    func setForceStarted(ids: Set<String>, enabled: Bool) {
        guard canSetForceStarted(ids:ids,enabled:enabled) else { return }
        busyTransfers.formUnion(ids); error = nil
        Task {
            defer { busyTransfers.subtract(ids) }
            var failures: [String] = []
            for id in ids.sorted() {
                var request = EngineRequest(command:"forceStart"); request.id = id; request.forceStarted = enabled
                do { _ = try await perform(request) }
                catch { failures.append(error.localizedDescription) }
            }
            self.error = failures.isEmpty ? nil : failures.joined(separator:"\n")
            await refresh()
        }
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
        guard !next.isEmpty else { error = String(localized:"Keep at least one file included.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale); return }
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
        panel.canCreateDirectories = true; panel.prompt = String(localized:"Move Files",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale); panel.message = String(localized:"Choose a folder for \(item.name). Files will be verified after moving.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        panel.directoryURL = URL(fileURLWithPath:item.destination)
        panel.begin { response in
            guard response == .OK, let url = panel.url else { return }
            #if SEEDHOP_SANDBOX
            do { try FolderAccessStore.application.remember(url) }
            catch { self.error = EngineMessage(code:FolderAccessFailure.noticeCode(for:error)).localizedDescription; return }
            #endif
            var request = EngineRequest(command:"relocate"); request.id = item.id; request.destination = url.path
            self.updateTransfer(request)
        }
    }
    func requestRename(_ item: TorrentItem, file: FileItem) {
        guard canChangeTransfers, storageRecovery == nil, !busyTransfers.contains(item.id), item.state != "checking", let window = NSApp.keyWindow else { return }
        let previousFocus = window.firstResponder
        let alert = NSAlert(); alert.messageText = String(localized:"Rename File",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale); alert.informativeText = String(localized:"Choose a new name for \(URL(fileURLWithPath:file.name).lastPathComponent).",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        alert.addButton(withTitle:String(localized:"Rename",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).identifier = NSUserInterfaceItemIdentifier("rename-confirm")
        alert.addButton(withTitle:String(localized:"Cancel",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).identifier = NSUserInterfaceItemIdentifier("rename-cancel")
        let input = NSTextField(string:URL(fileURLWithPath:file.name).lastPathComponent)
        input.frame = NSRect(x:0,y:0,width:320,height:24); input.setAccessibilityLabel(String(localized:"New file name",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)); alert.accessoryView = input
        alert.window.initialFirstResponder = input
        alert.beginSheetModal(for:window) { response in
            NativeAccessibility.restore(previousFocus,in:window)
            guard response == .alertFirstButtonReturn else { return }
            let name = input.stringValue.trimmingCharacters(in:.whitespacesAndNewlines)
            guard !name.isEmpty, name != ".", name != "..", !name.contains("/"), !name.contains(":") else { self.error = String(localized:"Enter a file name without slashes or colons.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale); return }
            let parent = (file.name as NSString).deletingLastPathComponent
            var request = EngineRequest(command:"rename"); request.id = item.id; request.fileIndex = file.index
            request.fileName = parent.isEmpty ? name : parent + "/" + name
            self.updateTransfer(request)
        }
    }
    func setBackground(_ enabled: Bool) async {
        guard !designPreview, !switching, !hasFileOperation, !savingEncryption, !savingSchedule, !savingBandwidth else { return }
        switching = true; polling?.cancel(); defer { switching = false; beginPolling() }
        let service = SMAppService.agent(plistName:SeedHopIdentity.agentPlist)
        var stoppedCurrent = false
        do {
            if enabled {
                if service.status == .notRegistered || service.status == .notFound { try service.register() }
                guard service.status == .enabled else { throw UserFacingError(message:String(localized:"Allow SeedHop in System Settings → General → Login Items & Extensions, then try again.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) }
            }
            _ = try await client.send(EngineRequest(command:"shutdown")); stoppedCurrent = true; client.invalidate(); connected = false
            if !enabled && service.status != .notRegistered { try await service.unregister() }
            apply(try await reconnectAfterHandoff(background:enabled))
            backgroundEnabled = enabled; connected = true
            UserDefaults.standard.set(enabled,forKey:"backgroundEnabled")
            // An explicit user choice supersedes a pending update restoration.
            UserDefaults.standard.removeObject(forKey:"updateRestoreBackground")
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
        var failure: Error = UserFacingError(message:String(localized:"The engine could not restart. Your library has been preserved.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
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
    var canPrepareForUpdate: Bool {
        !designPreview && connected && !switching && !connecting && !hasFileOperation &&
        !showImport && !savingEncryption && !savingSchedule && !savingBandwidth &&
        !savingQueue && busyTransfers.isEmpty
    }
    static func waitForEngineExit(_ pid: Int32) async throws {
        guard pid > 1, pid != ProcessInfo.processInfo.processIdentifier else {
            throw UserFacingError(message:String(localized:"The engine could not stop for the update. Try again.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
        }
        for _ in 0..<100 {
            if kill(pid, 0) != 0 && errno == ESRCH { return }
            try await Task.sleep(for:.milliseconds(100))
        }
        throw UserFacingError(message:String(localized:"The engine could not stop for the update. Try again.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
    }
    func prepareForUpdate(
        unregisterAgent: () async throws -> Void = {
            let service = SMAppService.agent(plistName:SeedHopIdentity.agentPlist)
            if service.status != .notRegistered && service.status != .notFound { try await service.unregister() }
        },
        waitForExit: (Int32) async throws -> Void = LibraryStore.waitForEngineExit
    ) async -> Bool {
        if updatePrepared { return true }
        guard canPrepareForUpdate else {
            error = String(localized:"Finish the current operation before installing the update.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
            return false
        }
        switching = true; polling?.cancel(); reconnect?.cancel()
        do {
            // A normal shutdown preserves paused intent; explicitStop would pause all transfers.
            let reply = try await client.send(EngineRequest(command:"shutdown"))
            guard let pid = reply.stoppedProcessID else {
                throw UserFacingError(message:String(localized:"The engine could not stop for the update. Try again.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
            }
            client.invalidate(); connected = false
            try await waitForExit(pid)
            if backgroundEnabled {
                UserDefaults.standard.set(true,forKey:"updateRestoreBackground")
                UserDefaults.standard.synchronize()
            }
            // Successful shutdown exits with status zero, so KeepAlive does not respawn it.
            // Unregister before replacement to prevent a login or launchd restart of old code.
            try await unregisterAgent()
            updatePrepared = true
            return true
        } catch {
            self.error = error.localizedDescription
            await resumeAfterFailedUpdate()
            return false
        }
    }
    func resumeAfterFailedUpdate() async {
        guard switching || updatePrepared else { return }
        updatePrepared = false; switching = false
        client.invalidate(); connected = false
        await start()
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
        guard !hasFileOperation else { error = String(localized:"Wait for the current file change to finish, or cancel it, then add your torrent.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale); return }
        incomingMagnet = magnet; incomingFile = file; showImport = true
    }
    func pasteMagnet(from pasteboard: NSPasteboard = .general) {
        let link = (pasteboard.string(forType:.string) ?? "").trimmingCharacters(in:.whitespacesAndNewlines)
        beginImport(magnet:URLComponents(string:link)?.scheme?.lowercased() == "magnet" ? link : "")
    }
    func open(_ urls: [URL]) {
        let coordinator = externalOpenCoordinator ?? ExternalOpenCoordinator()
        coordinator.attach(self)
        coordinator.receive(urls)
    }
}
