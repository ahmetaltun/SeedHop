import Foundation

enum SeedHopIdentity {
    #if SEEDHOP_VALIDATION
    static let app = ValidationConfiguration.app
    static var library: URL { URL(fileURLWithPath:ValidationConfiguration.root).appendingPathComponent("state", isDirectory:true) }
    #else
    static let app = "app.seedhop.mac"
    static var library: URL { FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("SeedHop", isDirectory: true) }
    #endif
    static let foreground = app + ".engine"
    static let background = app + ".agent"
    static let protocolVersion = 6
    static let agentPlist = background + ".plist"
}

struct FileItem: Codable, Identifiable, Equatable {
    var index: UInt32
    var name: String
    var size: UInt64
    var downloaded: UInt64
    var selected: Bool
    var priority: UInt8 = 1
    var id: UInt32 { index }
}
struct TorrentItem: Codable, Identifiable, Equatable {
    var id: String
    var name: String
    var state: String
    var totalBytes: UInt64
    var downloadedBytes: UInt64
    var uploadedBytes: UInt64
    var downloadRate: UInt64
    var uploadRate: UInt64
    var peers: UInt32
    var destination: String
    var files: [FileItem]
    var trackers: [String]
    var error: String?
    var queuePosition: UInt32 = 0
    var pausedIntent: Bool? = nil
    var errorKind: String? = nil
    var errorMessage: EngineMessage? = nil
    var networkDownloadedBytes: UInt64? = nil
    var networkUploadedBytes: UInt64? = nil
    var addedAt: UInt64? = nil
    var completedAt: UInt64? = nil
    var etaSeconds: UInt64? = nil
    var selectedBytes: UInt64? = nil
    var fullBytes: UInt64 { files.isEmpty ? totalBytes : files.reduce(0) { $0 + $1.size } }
    var remainingBytes: UInt64 { (selectedBytes ?? totalBytes) > downloadedBytes ? (selectedBytes ?? totalBytes) - downloadedBytes : 0 }
    var shareRatio: Double? { guard let down = networkDownloadedBytes, down > 0, let up = networkUploadedBytes else { return nil }; return Double(up) / Double(down) }
    var sortableStatus: String { statusLabel }
    var sortableRatio: Double { shareRatio ?? -1 }
    var sortableETA: UInt64 { etaSeconds ?? UInt64.max }
    var sortableAdded: UInt64 { addedAt ?? 0 }
    var sortableCompleted: UInt64 { completedAt ?? 0 }
    var sortableDownloaded: UInt64 { networkDownloadedBytes ?? 0 }
    var sortableUploaded: UInt64 { networkUploadedBytes ?? 0 }
    var progress: Double { totalBytes == 0 ? 0 : min(1, Double(downloadedBytes) / Double(totalBytes)) }
    var isPaused: Bool { pausedIntent ?? (state == "paused") }
    var isActive: Bool { ["downloading", "waiting", "checking", "queued"].contains(state) || (state == "scheduled" && progress < 1) }
    var statusLabel: String {
        switch state { case "downloading": return String(localized:"Downloading",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale); case "seeding": return String(localized:"Seeding",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale); case "paused": return String(localized:"Paused",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale); case "scheduled": return String(localized:"Scheduled suspension",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale); case "queued": return String(localized:"Queued",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale); case "checking": return String(localized:"Checking files",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale); case "error": return String(localized:"Needs attention",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale); case "waiting": return peers == 0 ? String(localized:"Finding peers",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Waiting for data",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale); default: return String(localized:"Updating status",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
    }
}
struct FileOperationItem: Codable, Identifiable, Equatable {
    var id: String
    var torrentId: String
    var kind: String
    var phase: String
    var completedBytes: UInt64
    var totalBytes: UInt64
    var cancelRequested: Bool
    var canCancel: Bool { ["preparing", "copying"].contains(phase) && !cancelRequested }
    var title: String {
        if cancelRequested && ["preparing", "copying", "restoring"].contains(phase) { return String(localized:"Cancelling…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
        switch phase {
        case "copying": return kind == "rename" ? String(localized:"Renaming file…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Moving files…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "saving": return String(localized:"Saving the new location…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "verifying": return String(localized:"Starting file verification…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "cleaning": return String(localized:"Removing original copies…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "restoring": return String(localized:"Restoring the original location…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        default: return String(localized:"Preparing files…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        }
    }
    var detail: String {
        if cancelRequested { return String(localized:"Your original files will stay in place.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
        if phase == "copying" { return String(localized:"\(SizeText.bytes(completedBytes)) of \(SizeText.bytes(totalBytes)) copied",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
        if phase == "preparing" { return String(localized:"Checking the destination and available space.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
        if phase == "restoring" { return String(localized:"Keeping your original files and transfer settings.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
        return String(localized:"Finishing this change. File verification is shown separately in the transfer list.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
    }
}
struct StorageRecoveryItem: Codable, Identifiable, Equatable {
    var id: String
    var torrentId: String
    var activeLocation: String
    var message: String
    var originalDirectory: String
    var destinationDirectory: String
    var canKeepCopies: Bool
    var messageDescriptor: EngineMessage? = nil
    var localizedMessage: String {
        (messageDescriptor ?? EngineMessage(code:activeLocation == "new" ? "recovery.new" : activeLocation == "original" ? "recovery.original" : "recovery.unknown",diagnostic:message)).displayText
    }
}
enum PeerEncryptionPolicy: String, Codable, CaseIterable, Identifiable {
    case disabled, allow, prefer, require
    var id: String { rawValue }
    var title: String {
        switch self { case .disabled: return String(localized:"Disabled",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale); case .allow: return String(localized:"Allow encrypted peers",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale); case .prefer: return String(localized:"Prefer encrypted peers",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale); case .require: return String(localized:"Require encryption",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
    }
    var detail: String {
        switch self {
        case .disabled: return String(localized:"Use plaintext peer connections only. Encrypted-only peers cannot connect.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case .allow: return String(localized:"Accept encrypted peers. Try plaintext first when connecting, then encryption if needed.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case .prefer: return String(localized:"Try encryption first. Plaintext connections are still allowed if encryption fails.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case .require: return String(localized:"Require encrypted peer payloads. Peers that only support plaintext cannot connect, which may reduce available peers.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        }
    }
}

struct ScheduleRule: Codable, Equatable {
    var days: UInt8 = 31
    var startMinute: UInt16 = 540
    var endMinute: UInt16 = 1020
    var downloadLimit: UInt32 = 500_000
    var uploadLimit: UInt32 = 100_000
    var suspend = false
}
struct ScheduleConfig: Codable, Equatable {
    var enabled = false
    var rules: [ScheduleRule] = []
}
struct LibrarySnapshot: Codable {
    var statisticsMessage: EngineMessage? = nil
    var schedulingMessage: EngineMessage? = nil
    var statisticsWarning: String? = nil
    var protocolVersion: UInt32
    var engineVersion: String
    var instanceId: String
    var listenPort: UInt16
    var downloadLimit: UInt32
    var uploadLimit: UInt32
    var maxActiveDownloads: UInt32 = 0
    var torrents: [TorrentItem]
    var fileOperation: FileOperationItem? = nil
    var storageRecovery: StorageRecoveryItem? = nil
    var peerEncryption: PeerEncryptionPolicy? = nil
    var savedPeerEncryption: PeerEncryptionPolicy? = nil
    var schedule: ScheduleConfig? = nil
    var scheduleActive: Bool? = nil
    var schedulingWarning: String? = nil
    var normalDownloadLimit: UInt32? = nil
    var normalUploadLimit: UInt32? = nil
    var alternativeDownloadLimit: UInt32? = nil
    var alternativeUploadLimit: UInt32? = nil
    var alternativeLimitsEnabled: Bool? = nil
}
struct ImportPreview: Codable {
    var id: String
    var name: String
    var totalBytes: UInt64
    var files: [FileItem]
    var torrentBytes: Data
}
struct EngineRequest: Codable {
    var version = SeedHopIdentity.protocolVersion
    var alternativeLimitsEnabled: Bool? = nil
    var command: String
    var requestId = UUID().uuidString
    var id: String?
    var magnet: String?
    var bytes: Data?
    var destination: String?
    var stateDirectory: String?
    var selected: [UInt32]?
    var paused: Bool?
    var reuse: Bool?
    var addToTop: Bool?
    var sequential: Bool?
    var firstLastPieces: Bool?
    var deleteFiles: Bool?
    var downloadLimit: UInt32?
    var uploadLimit: UInt32?
    var resumeAtLogin: Bool?
    var explicitStop: Bool?
    var priorities: [UInt8]?
    var maxActiveDownloads: UInt32?
    var queuePosition: UInt32?
    var fileIndex: UInt32?
    var fileName: String?
    var peerEncryption: PeerEncryptionPolicy?
    var section: String?
    var allPeers: Bool?
    var schedule: ScheduleConfig?
}
struct EngineReply: Codable {
    var version = SeedHopIdentity.protocolVersion
    var snapshot: LibrarySnapshot?
    var preview: ImportPreview?
    var id: String?
    var error: String?
    var errorMessage: EngineMessage? = nil
    var cancelled: Bool?
    var cancellationAccepted: Bool?
    var details: TorrentDetails?
    init(snapshot: LibrarySnapshot? = nil, preview: ImportPreview? = nil, id: String? = nil, error: String? = nil, errorMessage: EngineMessage? = nil, cancelled: Bool? = nil, cancellationAccepted: Bool? = nil, details: TorrentDetails? = nil) {
        self.snapshot = snapshot; self.preview = preview; self.id = id; self.error = error
        self.cancelled = cancelled; self.cancellationAccepted = cancellationAccepted; self.details = details
        self.errorMessage = errorMessage
    }
}
struct UserFacingError: LocalizedError {
    let message: String
    var errorDescription: String? { message }
}

@objc protocol EngineServiceProtocol {
    func send(_ request: Data, withReply reply: @escaping (Data) -> Void)
}

enum SizeText {
    static func bytes(_ value: UInt64) -> String {
        let formatter = ByteCountFormatter(); formatter.countStyle = .file
        return formatter.string(fromByteCount: Int64(clamping: value))
    }
    static func limit(_ value: UInt32) -> String { value == 0 ? String(localized:"Unlimited",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"\(LocaleText.integer(value / 1000)) KB/s",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
    static func rate(_ value: UInt64, limit: UInt32) -> String { rate(value) + (limit == 0 ? "" : " / " + self.limit(limit)) }
    static func rate(_ value: UInt64) -> String { value == 0 ? "—" : String(localized:"\(bytes(value))/s",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
}

struct PeerDetails: Codable, Identifiable, Equatable {
    var id: String
    var endpoint: String
    var client: String?
    var state: String
    var transport: String?
    var encrypted: Bool?
    var incoming: Bool?
    var downloaded: UInt64
    var uploaded: UInt64
}
struct TrackerDetails: Codable, Identifiable, Equatable {
    var id: String
    var address: String
    var endpoint: String?
    var hash: String
    var state: String
    var message: String?
    var lastAnnounce: UInt64?
    var nextAnnounce: UInt64?
    var peers: UInt64?
    var seeds: UInt64?
    var leechers: UInt64?
}
struct TorrentDetails: Codable, Equatable {
    var torrentId: String
    var instanceId: String
    var observedAt: UInt64
    var section: String
    var format: String
    var `private`: Bool
    var v1Hash: String?
    var v2Hash: String?
    var dht: Bool
    var pex: Bool
    var localDiscovery: Bool
    var sessionDownloaded: UInt64
    var sessionUploaded: UInt64
    var seedingSeconds: UInt64?
    var peers: [PeerDetails]
    var trackers: [TrackerDetails]
    var events: [DiagnosticEvent]? = nil
}
struct DiagnosticEvent: Codable, Equatable { var sequence: UInt64; var timestamp: UInt64; var message: String }
struct TrafficSample: Identifiable {
    var id: Date
    var download: UInt64
    var upload: UInt64
}
struct TransferEvent: Identifiable {
    var id = UUID()
    var date = Date()
    var message: String
}
struct PeerRate: Equatable {
    var download: UInt64 = 0
    var upload: UInt64 = 0
}

/// Keep render and acceptance-fixture preferences separate from the user's workspace.
enum WorkspacePreferences {
    static let isFixture = ProcessInfo.processInfo.arguments.contains { $0.hasPrefix("--verify-") || ["--render-designs","--design-preview"].contains($0) }
    static let defaults: UserDefaults = isFixture ? UserDefaults(suiteName:"app.seedhop.workspace-fixtures")! : .standard
}
