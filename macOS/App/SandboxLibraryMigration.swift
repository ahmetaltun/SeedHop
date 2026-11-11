import Foundation

#if SEEDHOP_SANDBOX
import Darwin

/// Copy first, block the previous reader, then publish atomically. The original
/// session and payloads remain in place for an explicit, mutually exclusive rollback.
enum SandboxLibraryMigration {
    private static let receiptName = "sandbox-migration.json"
    private static let setupKey = "sandboxLibrarySetUp"
    static var isSetUp: Bool { UserDefaults.standard.bool(forKey:setupKey) }
    static func completeSetup() { UserDefaults.standard.set(true,forKey:setupKey) }
    static func resetSetup() { UserDefaults.standard.removeObject(forKey:setupKey) }
    static var legacyLibrary: URL? {
        guard let home = getpwuid(getuid())?.pointee.pw_dir else { return nil }
        return URL(fileURLWithPath:String(cString:home)).appendingPathComponent("Library/Application Support/SeedHop",isDirectory:true)
    }
    static var migratedDownloadDirectory: String? {
        guard let location = try? SandboxStorage.library.appendingPathComponent(receiptName),
              let data = try? Data(contentsOf:location) else { return nil }
        return (try? JSONDecoder().decode(Receipt.self,from:data))?.downloadDirectory
    }
    static func requiredFolders() -> [String] {
        guard let session = try? SandboxStorage.library.appendingPathComponent("session/session.json"),
              let bytes = try? Data(contentsOf:session),
              let root = (try? JSONSerialization.jsonObject(with:bytes)) as? [String:Any],
              let torrents = root["torrents"] as? [String:[String:Any]] else { return [] }
        return Set(torrents.values.compactMap { $0["output_folder"] as? String }).sorted()
    }
    private struct Receipt: Codable {
        var version = 1
        var id: String
        var source: String
        var stage: String
        var connectionSettings: Data?
        var downloadDirectory: String?
    }
    private enum Failure: Error { case invalidLibrary, busy, io }

    private static func directory(_ url: URL) throws -> Int32 {
        let fd = open(url.path, O_RDONLY | O_DIRECTORY | O_NOFOLLOW)
        guard fd >= 0 else { throw Failure.io }
        var info = stat()
        guard fstat(fd, &info) == 0, info.st_uid == getuid() else { close(fd); throw Failure.invalidLibrary }
        return fd
    }

    private static func lease(_ directory: Int32, create: Bool) throws -> Int32 {
        let fd = openat(directory, "engine.lock", O_RDWR | O_NOFOLLOW | (create ? O_CREAT : 0), 0o600)
        guard fd >= 0 else { throw Failure.io }
        var info = stat()
        guard fstat(fd, &info) == 0, info.st_mode & S_IFMT == S_IFREG, info.st_nlink == 1, info.st_uid == getuid() else { close(fd); throw Failure.invalidLibrary }
        guard flock(fd, LOCK_EX | LOCK_NB) == 0 else { close(fd); throw Failure.busy }
        return fd
    }

    private static func readFile(_ directory: Int32, _ name: String) throws -> Data? {
        let fd = openat(directory, name, O_RDONLY | O_NONBLOCK | O_NOFOLLOW)
        if fd < 0, errno == ENOENT { return nil }
        guard fd >= 0 else { throw Failure.io }
        defer { close(fd) }
        var info = stat()
        guard fstat(fd, &info) == 0, info.st_mode & S_IFMT == S_IFREG, info.st_nlink == 1,
              info.st_uid == getuid(), info.st_size <= 32 * 1024 * 1024 else { throw Failure.invalidLibrary }
        var result = Data(), buffer = [UInt8](repeating: 0, count: 65536)
        while true {
            let count = buffer.withUnsafeMutableBytes { read(fd, $0.baseAddress, $0.count) }
            if count < 0, errno == EINTR { continue }
            guard count >= 0 else { throw Failure.io }
            if count == 0 { break }
            result.append(contentsOf: buffer.prefix(count))
            guard result.count <= 32 * 1024 * 1024 else { throw Failure.invalidLibrary }
        }
        return result
    }

    private static func writeFile(_ directory: Int32, _ name: String, _ data: Data) throws {
        let temporary = ".sandbox-\(UUID().uuidString)"
        let fd = openat(directory, temporary, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, 0o600)
        guard fd >= 0 else { throw Failure.io }
        defer { close(fd); _ = unlinkat(directory, temporary, 0) }
        try data.withUnsafeBytes { bytes in
            var offset = 0
            while offset < bytes.count {
                let count = write(fd, bytes.baseAddress!.advanced(by: offset), bytes.count - offset)
                if count < 0, errno == EINTR { continue }
                guard count > 0 else { throw Failure.io }
                offset += count
            }
        }
        guard fsync(fd) == 0, renameat(directory, temporary, directory, name) == 0, fsync(directory) == 0 else { throw Failure.io }
    }

    private static func barrier(_ id: String) throws -> Data {
        // Both previous settings readers reject version 3 before opening a session.
        try JSONSerialization.data(withJSONObject: ["version": 3, "peer_encryption": "allow", "sandboxMigrationId": id], options: [.sortedKeys])
    }

    static func copyLibrary(from source: URL) throws {
        let scoped = source.startAccessingSecurityScopedResource()
        guard scoped else { throw FolderAccessFailure.unavailable }
        defer { source.stopAccessingSecurityScopedResource() }
        let target = try SandboxStorage.library
        guard source.standardizedFileURL != target.standardizedFileURL,
              !FileManager.default.fileExists(atPath: target.path) else { throw Failure.invalidLibrary }
        let original = try directory(source); defer { close(original) }
        let ownership = try lease(original, create: false); defer { close(ownership) }
        guard try readFile(original, "layout-transaction.json") == nil,
              try readFile(original, receiptName) == nil else { throw Failure.invalidLibrary }
        let settings = try readFile(original, "connection-settings.json")
        if let settings {
            let value = try JSONSerialization.jsonObject(with: settings) as? [String: Any]
            guard let version = value?["version"] as? Int, [1, 2, 4].contains(version) else { throw Failure.invalidLibrary }
            if version == 4 {
                guard value?["remember_statistics"] is Bool, value?["network_policy_required"] is Bool,
                      value?["statistics_clear_pending"] as? Bool != true else { throw Failure.invalidLibrary }
            }
        }
        let id = UUID().uuidString
        let previousIntent = try readFile(original,"run-intent.json")
        let intent = previousIntent.flatMap { try? JSONSerialization.jsonObject(with:$0) as? [String:Any] }
        let previousDefault = (intent?["downloadDirectory"] as? String).flatMap { path in
            path.hasPrefix("/") && path.utf8.count <= 4096 && !path.utf8.contains(0) ? path : nil
        }
        let receipt = Receipt(id:id,source:source.standardizedFileURL.path,stage:".migration-\(id)",
                              connectionSettings:settings,downloadDirectory:previousDefault)
        let stage = try SandboxStorage.container.appendingPathComponent(receipt.stage,isDirectory:true)
        try FileManager.default.createDirectory(at: stage, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
        var recoveryRecorded = false
        defer { if !recoveryRecorded { try? FileManager.default.removeItem(at:stage) } }
        let output = try directory(stage); defer { close(output) }
        let allowed = Set(["connection-settings.json", "network-policy.json", "limits.json", "queue.json", "statistics.json", "schedule.json", "alternative-limits.json"])
        var total = 0
        for name in try FileManager.default.contentsOfDirectory(atPath: source.path).sorted() {
            if ["engine.lock", "run-intent.json", ".DS_Store"].contains(name) { continue }
            if name == "session" { continue }
            guard allowed.contains(name), let bytes = try readFile(original, name) else { throw Failure.invalidLibrary }
            total += bytes.count; guard total <= 256 * 1024 * 1024 else { throw Failure.invalidLibrary }
            try writeFile(output, name, bytes)
        }
        let session = openat(original, "session", O_RDONLY | O_DIRECTORY | O_NOFOLLOW)
        guard session >= 0 else { throw Failure.invalidLibrary }; defer { close(session) }
        guard let sessionBytes = try readFile(session, "session.json"),
              let value = try JSONSerialization.jsonObject(with: sessionBytes) as? [String: Any],
              value["torrents"] is [String: Any] else { throw Failure.invalidLibrary }
        guard mkdirat(output, "session", 0o700) == 0 else { throw Failure.io }
        let copiedSession = openat(output, "session", O_RDONLY | O_DIRECTORY | O_NOFOLLOW)
        guard copiedSession >= 0 else { throw Failure.io }; defer { close(copiedSession) }
        let names = try FileManager.default.contentsOfDirectory(atPath: source.appendingPathComponent("session").path)
        guard names.count <= 10000 else { throw Failure.invalidLibrary }
        for name in names.sorted() {
            let metadata = name.range(of: "^[0-9a-fA-F]{40,64}\\.(torrent|bitv)$", options: .regularExpression) != nil
            guard name == "session.json" || metadata, let bytes = try readFile(session, name) else { throw Failure.invalidLibrary }
            total += bytes.count; guard total <= 256 * 1024 * 1024 else { throw Failure.invalidLibrary }
            try writeFile(copiedSession, name, bytes)
        }
        let receiptBytes = try JSONEncoder().encode(receipt)
        try writeFile(output, receiptName, receiptBytes)
        // Persist recovery instructions before disabling the previous reader.
        recoveryRecorded = true
        try writeFile(original, receiptName, receiptBytes)
        try writeFile(original, "connection-settings.json", barrier(id))
        // Exclusive publication cannot overwrite a concurrently created library.
        guard renamex_np(stage.path, target.path, UInt32(RENAME_EXCL)) == 0 else { throw Failure.io }
        let parent = try directory(SandboxStorage.container); defer { close(parent) }
        guard fsync(parent) == 0 else { throw Failure.io }
    }

    static func restorePreviousLibrary(at source: URL) throws {
        guard source.startAccessingSecurityScopedResource() else { throw FolderAccessFailure.unavailable }
        defer { source.stopAccessingSecurityScopedResource() }
        let original = try directory(source); defer { close(original) }
        let originalLease = try lease(original, create: false); defer { close(originalLease) }
        guard let bytes = try readFile(original, receiptName) else { throw Failure.invalidLibrary }
        let receipt = try JSONDecoder().decode(Receipt.self, from: bytes)
        guard receipt.version == 1, receipt.source == source.standardizedFileURL.path,
              UUID(uuidString: receipt.id) != nil, receipt.stage == ".migration-\(receipt.id)" else { throw Failure.invalidLibrary }
        let current = try readFile(original, "connection-settings.json")
        guard current == (try barrier(receipt.id)) || current == receipt.connectionSettings else { throw Failure.invalidLibrary }
        let target = try SandboxStorage.library
        if FileManager.default.fileExists(atPath: target.path) {
            let migrated = try directory(target); defer { close(migrated) }
            let migratedLease = try lease(migrated, create: true); defer { close(migratedLease) }
            guard try readFile(migrated, receiptName) == bytes else { throw Failure.invalidLibrary }
            if let settings = try readFile(migrated, "connection-settings.json"), settings != (try barrier(receipt.id)) {
                try writeFile(migrated, "sandbox-rollback-settings.json", settings)
            }
            // Disable the migrated reader before re-enabling the original reader.
            try writeFile(migrated, "connection-settings.json", barrier(receipt.id))
            let archived = try SandboxStorage.container.appendingPathComponent("Library-rollback-\(receipt.id)")
            guard renamex_np(target.path, archived.path, UInt32(RENAME_EXCL)) == 0 else { throw Failure.io }
            let parent = try directory(SandboxStorage.container); defer { close(parent) }
            guard fsync(parent) == 0 else { throw Failure.io }
        }
        if let settings = receipt.connectionSettings { try writeFile(original, "connection-settings.json", settings) }
        else { guard unlinkat(original, "connection-settings.json", 0) == 0 || errno == ENOENT else { throw Failure.io } }
        guard fsync(original) == 0, unlinkat(original, receiptName, 0) == 0, fsync(original) == 0 else { throw Failure.io }
    }
}
#endif
