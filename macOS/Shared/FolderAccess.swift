import Foundation

#if SEEDHOP_SANDBOX
import Darwin
enum SandboxStorage {
    static var container: URL {
        get throws {
            guard let url = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier:SandboxConfiguration.group) else {
                throw FolderAccessFailure.unavailable
            }
            return url
        }
    }
    static var library: URL { get throws { try container.appendingPathComponent("Library",isDirectory:true) } }
}

enum FolderAccessFailure: Error {
    case unavailable, invalidGrant, damagedArchive, unsupportedArchive

    static func noticeCode(for error: Error) -> String {
        if let failure = error as? Self, case .unsupportedArchive = failure { return "host.folder_access_version" }
        return "host.folder_access"
    }
}

/// Each executable creates its own persistent bookmarks from the temporary grants
/// it receives. App-scoped bookmarks are never assumed portable between identities.
final class FolderAccessStore {
    static let application = FolderAccessStore(role: "application")
    private struct Record: Codable { var path: String; var bookmark: Data }
    private struct Archive: Codable { var version = 1; var records: [Record] }
    private struct Header: Decodable { var version: Int }
    private struct Inspection { var original: Data?; var records: [String: Record]; var damaged: Bool }
    private let role: String
    private let lock = NSRecursiveLock()
    private var loaded = false
    private var revision: UInt64 = 0
    private var records: [String: Record] = [:]
    private var renewals: [String: Record] = [:]
    private var active: [String: URL] = [:]
    private var archiveName: String { "\(role)-folder-access.json" }

    init(role: String) { self.role = role }
    deinit { for url in active.values { url.stopAccessingSecurityScopedResource() } }

    private func load() throws {
        guard !loaded else { return }
        let directory = try openDirectory(); defer { close(directory) }
        let inspection = try inspect(directory)
        guard !inspection.damaged else { throw FolderAccessFailure.damagedArchive }
        install(inspection.records, approved: [:])
    }

    private func openDirectory() throws -> Int32 {
        let container = try SandboxStorage.container
        try FileManager.default.createDirectory(at: container, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let fd = open(container.path, O_RDONLY | O_DIRECTORY | O_NOFOLLOW)
        guard fd >= 0 else { throw FolderAccessFailure.unavailable }
        var info = stat()
        guard fstat(fd, &info) == 0, info.st_uid == getuid() else { close(fd); throw FolderAccessFailure.unavailable }
        return fd
    }

    private func inspect(_ directory: Int32) throws -> Inspection {
        let fd = openat(directory, archiveName, O_RDONLY | O_NONBLOCK | O_NOFOLLOW)
        if fd < 0, errno == ENOENT { return Inspection(original: nil, records: [:], damaged: false) }
        guard fd >= 0 else { throw FolderAccessFailure.unavailable }; defer { close(fd) }
        var info = stat()
        guard fstat(fd, &info) == 0, info.st_mode & S_IFMT == S_IFREG, info.st_nlink == 1,
              info.st_uid == getuid(), info.st_size <= 4 * 1024 * 1024 else { throw FolderAccessFailure.unavailable }
        var bytes = Data(), buffer = [UInt8](repeating: 0, count: 8192)
        while true {
            let count = buffer.withUnsafeMutableBytes { read(fd, $0.baseAddress, $0.count) }
            if count < 0, errno == EINTR { continue }
            guard count >= 0 else { throw FolderAccessFailure.unavailable }
            if count == 0 { break }
            bytes.append(contentsOf: buffer.prefix(count))
            guard bytes.count <= 4 * 1024 * 1024 else { throw FolderAccessFailure.unavailable }
        }
        let header = try? JSONDecoder().decode(Header.self, from: bytes)
        let root = (try? JSONSerialization.jsonObject(with: bytes)) as? [String: Any]
        if root?["version"] != nil, header?.version != 1 {
            // A newer format is not corruption and must never be rewritten here.
            throw FolderAccessFailure.unsupportedArchive
        }
        guard header?.version == 1, let root,
              let entries = root["records"] as? [Any], entries.count <= 64 else {
            return Inspection(original: bytes, records: [:], damaged: true)
        }
        var recovered: [String: Record] = [:], ambiguous = Set<String>(), damaged = false
        for entry in entries {
            guard JSONSerialization.isValidJSONObject(entry),
                  let data = try? JSONSerialization.data(withJSONObject: entry),
                  let record = try? JSONDecoder().decode(Record.self, from: data),
                  record.path.hasPrefix("/"), record.path.utf8.count <= 4096, !record.path.utf8.contains(0),
                  !record.bookmark.isEmpty, record.bookmark.count <= 64 * 1024 else { damaged = true; continue }
            if ambiguous.contains(record.path) { damaged = true; continue }
            if let prior = recovered[record.path] {
                damaged = true
                if prior.bookmark != record.bookmark { recovered.removeValue(forKey: record.path); ambiguous.insert(record.path) }
            } else { recovered[record.path] = record }
        }
        return Inspection(original: bytes, records: recovered, damaged: damaged)
    }

    private func write(_ bytes: Data, name: String, directory: Int32, publish: Bool) throws {
        let temporary = publish ? ".folder-access-\(UUID().uuidString)" : name
        let fd = openat(directory, temporary, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, 0o600)
        guard fd >= 0 else { throw FolderAccessFailure.unavailable }; defer { close(fd) }
        try bytes.withUnsafeBytes { data in
            var offset = 0
            while offset < data.count {
                let count = Darwin.write(fd, data.baseAddress!.advanced(by: offset), data.count - offset)
                if count < 0, errno == EINTR { continue }
                guard count > 0 else { throw FolderAccessFailure.unavailable }
                offset += count
            }
        }
        guard fsync(fd) == 0 else { throw FolderAccessFailure.unavailable }
        if publish, renameat(directory, temporary, directory, name) != 0 { throw FolderAccessFailure.unavailable }
        guard fsync(directory) == 0 else { throw FolderAccessFailure.unavailable }
    }

    private func install(_ updated: [String: Record], approved: [String: URL]) {
        var restored = approved, reused = Set<String>(), refreshed: [String: Record] = [:]
        for (path, record) in updated where restored[path] == nil {
            if records[path]?.bookmark == record.bookmark || renewals[path]?.bookmark == record.bookmark, let url = active[path] {
                restored[path] = url; reused.insert(path)
                if records[path]?.bookmark == record.bookmark { refreshed[path] = renewals[path] }
                continue
            }
            var stale = false
            // A disconnected drive or revoked grant stays in the archive for
            // explicit reapproval. It cannot authorize startup through a path.
            guard let url = try? URL(resolvingBookmarkData: record.bookmark, options: [.withSecurityScope, .withoutUI], relativeTo: nil, bookmarkDataIsStale: &stale),
                  url.startAccessingSecurityScopedResource() else { continue }
            restored[path] = url
            if stale, let bookmark = try? url.bookmarkData(options: .withSecurityScope, includingResourceValuesForKeys: nil, relativeTo: nil),
               !bookmark.isEmpty, bookmark.count <= 64 * 1024 {
                refreshed[path] = Record(path: path, bookmark: bookmark)
            }
        }
        for (path, url) in active where !reused.contains(path) { url.stopAccessingSecurityScopedResource() }
        records = updated; renewals = refreshed; active = restored; loaded = true
    }

    /// Only actual approved URLs may rebuild a damaged archive. Reading settings
    /// or starting an engine never substitutes paths for lost bookmark authority.
    private func approve(_ urls: [URL]) throws {
        guard !urls.isEmpty, urls.count <= 64 else { throw FolderAccessFailure.invalidGrant }
        var approved: [String: URL] = [:], incoming: [String: Record] = [:]
        var committed = false
        defer { if !committed { for url in approved.values { url.stopAccessingSecurityScopedResource() } } }
        for url in urls {
            guard url.isFileURL else { throw FolderAccessFailure.invalidGrant }
            let path = url.standardizedFileURL.path
            guard path.utf8.count <= 4096, !path.utf8.contains(0) else { throw FolderAccessFailure.invalidGrant }
            if approved[path] != nil { continue }
            guard url.startAccessingSecurityScopedResource() else { throw FolderAccessFailure.unavailable }
            approved[path] = url
            guard try url.resourceValues(forKeys: [.isDirectoryKey]).isDirectory == true else { throw FolderAccessFailure.invalidGrant }
            let bookmark = try url.bookmarkData(options: .withSecurityScope, includingResourceValuesForKeys: nil, relativeTo: nil)
            guard !bookmark.isEmpty, bookmark.count <= 64 * 1024 else { throw FolderAccessFailure.invalidGrant }
            incoming[path] = Record(path: path, bookmark: bookmark)
        }
        let directory = try openDirectory(); defer { close(directory) }
        let ownership = openat(directory, "\(role)-folder-access.lock", O_RDWR | O_CREAT | O_NOFOLLOW, 0o600)
        guard ownership >= 0 else { throw FolderAccessFailure.unavailable }; defer { close(ownership) }
        var info = stat()
        guard fstat(ownership, &info) == 0, info.st_mode & S_IFMT == S_IFREG, info.st_nlink == 1,
              info.st_uid == getuid(), flock(ownership, LOCK_EX | LOCK_NB) == 0 else { throw FolderAccessFailure.unavailable }
        // Re-read under the process-wide lease even if this instance has a cache.
        let inspection = try inspect(directory)
        var updated = inspection.records.merging(incoming) { _, approved in approved }
        // Renew a stale bookmark only if its disk record still matches the one
        // that produced this scope. Never overwrite another writer's new grant.
        for (path, renewal) in renewals where incoming[path] == nil && inspection.records[path]?.bookmark == records[path]?.bookmark {
            updated[path] = renewal
        }
        guard updated.count <= 64 else { throw FolderAccessFailure.invalidGrant }
        let bytes = try JSONEncoder().encode(Archive(records: updated.values.sorted { $0.path < $1.path }))
        guard bytes.count <= 4 * 1024 * 1024 else { throw FolderAccessFailure.invalidGrant }
        if inspection.damaged, let original = inspection.original {
            try write(original, name: "\(role)-folder-access-damaged-\(UUID().uuidString).json", directory: directory, publish: false)
        }
        try write(bytes, name: archiveName, directory: directory, publish: true)
        install(updated, approved: approved)
        revision &+= 1; committed = true
    }

    func remember(_ url: URL) throws {
        lock.lock(); defer { lock.unlock() }
        try approve([url])
    }

    func generation() throws -> UInt64 {
        lock.lock(); defer { lock.unlock() }
        try load()
        return revision
    }

    func transferBookmarks() throws -> [Data] {
        lock.lock(); defer { lock.unlock() }
        try load()
        let bookmarks = try active.values.map { try $0.bookmarkData(options: [], includingResourceValuesForKeys: nil, relativeTo: nil) }
        guard bookmarks.allSatisfy({ $0.count <= 64 * 1024 }) else { throw FolderAccessFailure.invalidGrant }
        return bookmarks
    }

    func receive(_ bookmarks: [Data]) throws {
        lock.lock(); defer { lock.unlock() }
        guard bookmarks.count <= 64, bookmarks.allSatisfy({ $0.count <= 64 * 1024 }) else { throw FolderAccessFailure.invalidGrant }
        if bookmarks.isEmpty { try load(); return }
        var urls: [URL] = []
        for bookmark in bookmarks {
            var stale = false
            let url = try URL(resolvingBookmarkData: bookmark, options: [.withoutImplicitStartAccessing, .withoutUI], relativeTo: nil, bookmarkDataIsStale: &stale)
            urls.append(url)
        }
        // remember() balances this URL's explicit scope and creates a bookmark
        // owned by this receiving executable for autonomous background restore.
        // The same approval implementation commits a received batch only once.
        try approve(urls)
    }

    func requireAccess(to path: String) throws {
        lock.lock(); defer { lock.unlock() }
        try load()
        let target = URL(fileURLWithPath: path).resolvingSymlinksInPath().standardizedFileURL.path
        guard active.values.contains(where: {
            let root = $0.resolvingSymlinksInPath().standardizedFileURL.path
            return target == root || target.hasPrefix(root == "/" ? "/" : root + "/")
        }) else { throw FolderAccessFailure.unavailable }
    }
}
#endif
