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
    var progress: Double { totalBytes == 0 ? 0 : min(1, Double(downloadedBytes) / Double(totalBytes)) }
    var isPaused: Bool { pausedIntent ?? (state == "paused") }
    var isActive: Bool { ["downloading", "waiting", "checking", "queued"].contains(state) }
    var statusLabel: String {
        switch state { case "downloading": return "Downloading"; case "seeding": return "Seeding"; case "paused": return "Paused"; case "queued": return "Queued"; case "checking": return "Verifying files"; case "error": return "Needs attention"; default: return "Waiting for peers" }
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
}
struct EngineReply: Codable {
    var version = SeedHopIdentity.protocolVersion
    var snapshot: LibrarySnapshot?
    var preview: ImportPreview?
    var id: String?
    var error: String?
    init(snapshot: LibrarySnapshot? = nil, preview: ImportPreview? = nil, id: String? = nil, error: String? = nil) {
        self.snapshot = snapshot; self.preview = preview; self.id = id; self.error = error
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
