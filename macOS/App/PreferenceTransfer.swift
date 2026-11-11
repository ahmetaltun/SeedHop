import Foundation

#if SEEDHOP_DEVELOPMENT || SEEDHOP_SANDBOX
import Darwin

/// Portable UI settings only. Engine policy, paths, permissions and private
/// transfer information never enter this archive.
@MainActor enum PreferenceTransfer {
    struct SortRule: Codable { var field: String; var reverse: Bool }
    struct Values: Codable {
        var appearance: String
        var language: String
        var showDockSpeeds: Bool
        var showTransferDetails: Bool
        var sidebarVisible: Bool
        var comfortableRows: Bool
        var columnOrder: [String]
        var columnWidths: [String: Double]
        var sortOrder: [SortRule]
    }
    struct Archive: Codable {
        var format = "app.seedhop.preferences"
        var version = 1
        var values: Values
    }
    private enum Phase: String, Codable { case pending, applying, applied, restorePending, restored }
    private struct Transaction: Codable {
        var version = 1
        var domain: String
        var phase: Phase
        var archive: Archive
        var before: Data?
    }
    private enum Failure: Error { case invalidArchive, io, pendingChange }
    private static let fileName = "transaction.json"
    private static let keys: Set<String> = ["appearancePreference", "AppleLanguages", "showDockSpeeds", "showTransferDetails", "workspaceSidebarVisible", "workspaceComfortableRows", "workspaceNativeColumnOrder", "workspaceColumnWidths", "workspaceSortOrder", "workspaceSort", "workspaceSortReverse"]
    private static var domain: String { Bundle.main.bundleIdentifier ?? SeedHopIdentity.app }
    private static var prepared = false
    static private(set) var startupFailed = false

    private static func validate(_ archive: Archive) throws {
        let value = archive.values
        let columns = Set(TransferColumns.all.map(\.id))
        let fields = Set(LibraryView.sortFields.map(\.0))
        guard archive.format == "app.seedhop.preferences", archive.version == 1,
              AppearancePreference(rawValue: value.appearance) != nil,
              value.language.isEmpty || AppLanguagePreference.options.contains(where: { $0.id == value.language }),
              !value.columnOrder.isEmpty, value.columnOrder.count <= columns.count,
              Set(value.columnOrder).count == value.columnOrder.count, Set(value.columnOrder).isSubset(of: columns),
              value.columnWidths.count <= columns.count,
              value.columnWidths.allSatisfy({ columns.contains($0.key) && $0.value.isFinite && (40...4000).contains($0.value) }),
              !value.sortOrder.isEmpty, value.sortOrder.count <= fields.count,
              Set(value.sortOrder.map(\.field)).count == value.sortOrder.count,
              value.sortOrder.allSatisfy({ fields.contains($0.field) }) else { throw Failure.invalidArchive }
    }

    static func exportData() throws -> Data {
        guard !WorkspacePreferences.isFixture else { throw Failure.invalidArchive }
        let saved = UserDefaults.standard.persistentDomain(forName: domain) ?? [:]
        let archive = try portableSettings(from:saved)
        let encoder = JSONEncoder(); encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        let bytes = try encoder.encode(archive)
        guard bytes.count <= 65536 else { throw Failure.invalidArchive }
        return bytes
    }

    private static func portableSettings(from saved: [String:Any]) throws -> Archive {
        let columns = TransferColumns.all.map(\.id)
        let order = (saved[TransferColumns.orderKey] as? [String]) ?? columns
        let rawWidths = saved["workspaceColumnWidths"] as? [String: Double] ?? [:]
        var widths: [String: Double] = [:]
        for id in columns {
            if let value = rawWidths[id] ?? rawWidths[TransferColumns.legacyTitles[id] ?? ""], value.isFinite, (40...4000).contains(value) { widths[id] = value }
        }
        let sort = (saved["workspaceSortOrder"] as? [[String: Any]])?.compactMap { row -> SortRule? in
            guard let field = row["field"] as? String, let reverse = row["reverse"] as? Bool else { return nil }
            return SortRule(field: field, reverse: reverse)
        }
        let archive = Archive(values: Values(
            appearance: saved["appearancePreference"] as? String ?? "dark",
            language: (saved["AppleLanguages"] as? [String])?.first ?? "",
            showDockSpeeds: saved["showDockSpeeds"] as? Bool ?? true,
            showTransferDetails: saved["showTransferDetails"] as? Bool ?? true,
            sidebarVisible: saved["workspaceSidebarVisible"] as? Bool ?? true,
            comfortableRows: saved["workspaceComfortableRows"] as? Bool ?? false,
            columnOrder: order, columnWidths: widths,
            sortOrder: (sort?.isEmpty == false ? sort : nil) ?? [SortRule(field: saved["workspaceSort"] as? String ?? "queue", reverse: saved["workspaceSortReverse"] as? Bool ?? false)]))
        try validate(archive)
        return archive
    }

    private static func openDirectory() throws -> (directory: Int32, lock: Int32) {
        let url = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("SeedHopPreferenceTransfer", isDirectory: true)
            .appendingPathComponent(domain, isDirectory: true)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let fd = open(url.path, O_RDONLY | O_DIRECTORY | O_NOFOLLOW)
        guard fd >= 0 else { throw Failure.io }
        var info = stat()
        guard fstat(fd, &info) == 0, info.st_uid == getuid(), fchmod(fd, 0o700) == 0 else { close(fd); throw Failure.io }
        let ownership = openat(fd, "transfer.lock", O_RDWR | O_CREAT | O_NOFOLLOW, 0o600)
        guard ownership >= 0 else { close(fd); throw Failure.io }
        var lockInfo = stat()
        guard fstat(ownership, &lockInfo) == 0, lockInfo.st_mode & S_IFMT == S_IFREG,
              lockInfo.st_nlink == 1, lockInfo.st_uid == getuid(),
              flock(ownership, LOCK_EX | LOCK_NB) == 0 else { close(ownership); close(fd); throw Failure.io }
        return (fd, ownership)
    }

    private static func read(_ fd: Int32, limit: Int) throws -> Data {
        var info = stat()
        guard fstat(fd, &info) == 0, info.st_mode & S_IFMT == S_IFREG,
              info.st_size <= limit else { throw Failure.invalidArchive }
        var result = Data(), buffer = [UInt8](repeating: 0, count: 8192)
        while true {
            let count = buffer.withUnsafeMutableBytes { Darwin.read(fd, $0.baseAddress, $0.count) }
            if count < 0, errno == EINTR { continue }
            guard count >= 0 else { throw Failure.io }
            if count == 0 { return result }
            result.append(contentsOf: buffer.prefix(count))
            guard result.count <= limit else { throw Failure.invalidArchive }
        }
    }

    static func inspect(_ url: URL) throws -> Archive {
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        let fd = open(url.path, O_RDONLY | O_NONBLOCK | O_NOFOLLOW)
        guard fd >= 0 else { throw Failure.io }; defer { close(fd) }
        let bytes = try read(fd, limit: 65536)
        #if SEEDHOP_SANDBOX
        if url.lastPathComponent == "app.seedhop.mac.plist" {
            guard let saved = try PropertyListSerialization.propertyList(from:bytes,options:[],format:nil) as? [String:Any] else {
                throw Failure.invalidArchive
            }
            return try portableSettings(from:saved)
        }
        #endif
        let archive = try JSONDecoder().decode(Archive.self, from:bytes)
        try validate(archive)
        return archive
    }

    private static func load(_ directory: Int32) throws -> Transaction? {
        let fd = openat(directory, fileName, O_RDONLY | O_NONBLOCK | O_NOFOLLOW)
        if fd < 0, errno == ENOENT { return nil }
        guard fd >= 0 else { throw Failure.io }; defer { close(fd) }
        var info = stat()
        guard fstat(fd, &info) == 0, info.st_uid == getuid(), info.st_nlink == 1 else { throw Failure.invalidArchive }
        let transaction = try JSONDecoder().decode(Transaction.self, from: read(fd, limit: 256 * 1024))
        guard transaction.version == 1, transaction.domain == domain,
              (transaction.before?.count ?? 0) <= 65536 else { throw Failure.invalidArchive }
        if [.applying, .applied, .restorePending].contains(transaction.phase), transaction.before == nil { throw Failure.invalidArchive }
        if transaction.phase == .pending, transaction.before != nil { throw Failure.invalidArchive }
        try validate(transaction.archive)
        return transaction
    }

    private static func save(_ transaction: Transaction, in directory: Int32) throws {
        let bytes = try JSONEncoder().encode(transaction)
        guard bytes.count <= 256 * 1024 else { throw Failure.invalidArchive }
        let temporary = ".preferences-\(UUID().uuidString)"
        let fd = openat(directory, temporary, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, 0o600)
        guard fd >= 0 else { throw Failure.io }; defer { close(fd) }
        try bytes.withUnsafeBytes { data in
            var offset = 0
            while offset < data.count {
                let count = write(fd, data.baseAddress!.advanced(by: offset), data.count - offset)
                if count < 0, errno == EINTR { continue }
                guard count > 0 else { throw Failure.io }
                offset += count
            }
        }
        guard fsync(fd) == 0, renameat(directory, temporary, directory, fileName) == 0, fsync(directory) == 0 else { throw Failure.io }
    }

    static func stage(_ archive: Archive) throws {
        guard !WorkspacePreferences.isFixture else { throw Failure.invalidArchive }
        try validate(archive)
        let ownership = try openDirectory(); let directory = ownership.directory
        defer { close(ownership.lock); close(directory) }
        if let current = try load(directory), [.pending, .applying, .restorePending].contains(current.phase) { throw Failure.pendingChange }
        try save(Transaction(domain: domain, phase: .pending, archive: archive), in: directory)
    }

    static func requestUndo() throws -> Bool {
        guard !WorkspacePreferences.isFixture else { throw Failure.invalidArchive }
        let ownership = try openDirectory(); let directory = ownership.directory
        defer { close(ownership.lock); close(directory) }
        guard var transaction = try load(directory) else { throw Failure.invalidArchive }
        if transaction.phase == .pending, transaction.before == nil {
            transaction.phase = .restored
            try save(transaction, in: directory)
            return false
        }
        guard [.applied, .applying].contains(transaction.phase), transaction.before != nil else { throw Failure.invalidArchive }
        transaction.phase = .restorePending
        try save(transaction, in: directory)
        return true
    }

    static func canUndo() -> Bool {
        guard !WorkspacePreferences.isFixture else { return false }
        do {
            let ownership = try openDirectory(); let directory = ownership.directory
            defer { close(ownership.lock); close(directory) }
            guard let transaction = try load(directory) else { return false }
            return [.pending, .applying, .applied].contains(transaction.phase)
        } catch { return false }
    }

    /// Commit before models read defaults. A pending transaction survives process
    /// interruption; its original backup is never replaced during replay.
    static func prepareAtLaunch() {
        guard !prepared, !WorkspacePreferences.isFixture else { return }
        prepared = true
        do {
            let ownership = try openDirectory(); let directory = ownership.directory
            defer { close(ownership.lock); close(directory) }
            guard var transaction = try load(directory), [.pending, .applying, .restorePending].contains(transaction.phase) else { return }
            let defaults = UserDefaults.standard
            var merged = defaults.persistentDomain(forName: domain) ?? [:]
            if transaction.phase == .pending {
                let original = merged.filter { keys.contains($0.key) }
                transaction.before = try PropertyListSerialization.data(fromPropertyList: original, format: .binary, options: 0)
                guard transaction.before!.count <= 65536 else { throw Failure.invalidArchive }
                transaction.phase = .applying
                try save(transaction, in: directory)
            }
            let restore = transaction.phase == .restorePending
            let replacement: [String: Any]
            if restore {
                guard let before = transaction.before,
                      let values = try PropertyListSerialization.propertyList(from: before, options: [], format: nil) as? [String: Any],
                      Set(values.keys).isSubset(of: keys) else { throw Failure.invalidArchive }
                replacement = values
            } else {
                let v = transaction.archive.values
                var values: [String: Any] = ["appearancePreference": v.appearance, "showDockSpeeds": v.showDockSpeeds, "showTransferDetails": v.showTransferDetails,
                    "workspaceSidebarVisible": v.sidebarVisible, "workspaceComfortableRows": v.comfortableRows,
                    "workspaceNativeColumnOrder": v.columnOrder, "workspaceColumnWidths": v.columnWidths,
                    "workspaceSortOrder": v.sortOrder.map { ["field": $0.field, "reverse": $0.reverse] as [String: Any] }]
                if !v.language.isEmpty { values["AppleLanguages"] = [v.language] }
                replacement = values
            }
            for key in keys { merged.removeValue(forKey: key) }
            merged.merge(replacement) { _, new in new }
            defaults.setPersistentDomain(merged, forName: domain)
            guard defaults.synchronize() else { throw Failure.io }
            transaction.phase = restore ? .restored : .applied
            try save(transaction, in: directory)
            // Bundle may have cached the previous language before this commit.
            let languages = (replacement["AppleLanguages"] as? [String]) ??
                (defaults.persistentDomain(forName: UserDefaults.globalDomain)?["AppleLanguages"] as? [String]) ?? ["en"]
            let language = Bundle.preferredLocalizations(from: Bundle.main.localizations, forPreferences: languages).first ?? "en"
            AppLocalization.shared.use(language)
        } catch { startupFailed = true }
    }
}
#endif
