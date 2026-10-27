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
    static let protocolVersion = 2
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
    var progress: Double { totalBytes == 0 ? 0 : min(1, Double(downloadedBytes) / Double(totalBytes)) }
    var isPaused: Bool { pausedIntent ?? (state == "paused") }
    var isActive: Bool { ["downloading", "waiting", "checking", "queued"].contains(state) }
    var statusLabel: String {
        switch state { case "downloading": return "Downloading"; case "seeding": return "Seeding"; case "paused": return "Paused"; case "queued": return "Queued"; case "checking": return "Checking files"; case "error": return "Needs attention"; case "waiting": return peers == 0 ? "Finding peers" : "Waiting for data"; default: return "Updating status" }
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
        if cancelRequested && ["preparing", "copying", "restoring"].contains(phase) { return "Cancelling…" }
        switch phase {
        case "copying": return kind == "rename" ? "Renaming file…" : "Moving files…"
        case "saving": return "Saving the new location…"
        case "verifying": return "Starting file verification…"
        case "cleaning": return "Removing original copies…"
        case "restoring": return "Restoring the original location…"
        default: return "Preparing files…"
        }
    }
    var detail: String {
        if cancelRequested { return "Your original files will stay in place." }
        if phase == "copying" { return "\(SizeText.bytes(completedBytes)) of \(SizeText.bytes(totalBytes)) copied" }
        if phase == "preparing" { return "Checking the destination and available space." }
        if phase == "restoring" { return "Keeping your original files and transfer settings." }
        return "Finishing this change. File verification is shown separately in the transfer list."
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
}
enum PeerEncryptionPolicy: String, Codable, CaseIterable, Identifiable {
    case disabled, allow, prefer, require
    var id: String { rawValue }
    var title: String {
        switch self { case .disabled: return "Disabled"; case .allow: return "Allow encrypted peers"; case .prefer: return "Prefer encrypted peers"; case .require: return "Require encryption" }
    }
    var detail: String {
        switch self {
        case .disabled: return "Use plaintext peer connections only. Encrypted-only peers cannot connect."
        case .allow: return "Accept encrypted peers. Try plaintext first when connecting, then encryption if needed."
        case .prefer: return "Try encryption first. Plaintext connections are still allowed if encryption fails."
        case .require: return "Require encrypted peer payloads. Peers that only support plaintext cannot connect, which may reduce available peers."
        }
    }
}

struct LibrarySnapshot: Codable {
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
}
struct EngineReply: Codable {
    var version = SeedHopIdentity.protocolVersion
    var snapshot: LibrarySnapshot?
    var preview: ImportPreview?
    var id: String?
    var error: String?
    var cancelled: Bool?
    var cancellationAccepted: Bool?
    init(snapshot: LibrarySnapshot? = nil, preview: ImportPreview? = nil, id: String? = nil, error: String? = nil, cancelled: Bool? = nil, cancellationAccepted: Bool? = nil) {
        self.snapshot = snapshot; self.preview = preview; self.id = id; self.error = error
        self.cancelled = cancelled; self.cancellationAccepted = cancellationAccepted
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
    static func rate(_ value: UInt64) -> String { value == 0 ? "—" : bytes(value) + "/s" }
}
