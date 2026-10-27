import Foundation
import Combine

/// Owns a single import sheet. A cancelled lookup can never update a later attempt.
@MainActor final class ImportSession: ObservableObject {
    typealias Send = @MainActor (EngineRequest) async throws -> EngineReply

    @Published private(set) var preview: ImportPreview?
    @Published var magnet = ""
    @Published var selected = Set<UInt32>()
    @Published var destination: String
    @Published var paused = false
    @Published var reuse = false
    @Published private(set) var resolving = false
    @Published private(set) var adding = false
    @Published private(set) var error: String?
    @Published private(set) var sourceFile: URL?
    private var lookupID: String?

    init(preview: ImportPreview? = nil) {
        self.preview = preview
        selected = Set(preview?.files.map(\.index) ?? [])
        destination = UserDefaults.standard.string(forKey:"downloadDirectory") ?? FileManager.default.urls(for:.downloadsDirectory,in:.userDomainMask)[0].path
    }

    /// Native file-table keyboard actions affect captured rows, never another selection.
    func toggleFiles(_ indices: Set<UInt32>) {
        guard !adding, !resolving, let preview else { return }
        let targets = indices.intersection(Set(preview.files.map(\.index)))
        guard !targets.isEmpty else { return }
        if targets.isSubset(of:selected) { selected.subtract(targets) }
        else { selected.formUnion(targets) }
    }

    func resolve(using send: @escaping Send) async {
        guard !resolving, !adding, preview == nil else { return }
        let link = magnet.trimmingCharacters(in:.whitespacesAndNewlines)
        guard URLComponents(string:link)?.scheme?.lowercased() == "magnet" else {
            error = "Paste a magnet link beginning with magnet:, or open a .torrent file."
            return
        }
        sourceFile = nil
        let id = beginLookup()
        defer { finishLookup(id) }
        do {
            var request = EngineRequest(command:"inspect"); request.requestId = id; request.magnet = link
            try accept(try await send(request).preview, for:id)
        } catch { if lookupID == id { self.error = error.localizedDescription } }
    }

    func inspect(file: URL, using send: @escaping Send) async {
        guard !resolving, !adding, preview == nil else { return }
        sourceFile = file
        let id = beginLookup()
        defer { finishLookup(id) }
        do {
            // Reading even a bounded torrent file must not block the UI on a slow drive.
            let bytes = try await Task.detached(priority:.userInitiated) {
                let handle = try FileHandle(forReadingFrom:file)
                defer { try? handle.close() }
                let maximum = 32 * 1024 * 1024
                func oversized() -> UserFacingError { UserFacingError(message:"The torrent metadata exceeds 32 MB. Choose a smaller .torrent file.") }
                // Query the open file, avoiding URL resource caches when the user retries.
                guard try handle.seekToEnd() <= UInt64(maximum) else { throw oversized() }
                try handle.seek(toOffset:0)
                var bytes = Data()
                while let chunk = try handle.read(upToCount:min(65_536,maximum + 1 - bytes.count)), !chunk.isEmpty {
                    bytes.append(chunk)
                    guard bytes.count <= maximum else { throw oversized() }
                }
                return bytes
            }.value
            guard lookupID == id else { return }
            var request = EngineRequest(command:"inspect"); request.requestId = id; request.bytes = bytes
            try accept(try await send(request).preview, for:id)
        } catch { if lookupID == id { self.error = error.localizedDescription } }
    }

    func cancel(using send: @escaping Send) {
        guard !adding, let id = lookupID else { return }
        lookupID = nil; resolving = false
        var request = EngineRequest(command:"cancel"); request.id = id
        Task { _ = try? await send(request) }
    }

    func add(using send: @escaping Send) async -> String? {
        guard !adding, !resolving, let preview, !selected.isEmpty else { return nil }
        adding = true; error = nil
        defer { adding = false }
        var request = EngineRequest(command:"add")
        request.bytes = preview.torrentBytes; request.selected = selected.sorted()
        request.destination = destination; request.paused = paused; request.reuse = reuse
        do {
            guard let id = try await send(request).id else { throw UserFacingError(message:"The engine did not confirm this transfer. Check your library before trying again.") }
            return id
        } catch { self.error = error.localizedDescription; return nil }
    }

    private func beginLookup() -> String {
        let id = UUID().uuidString
        lookupID = id; resolving = true; error = nil
        return id
    }
    private func finishLookup(_ id: String) {
        if lookupID == id { lookupID = nil; resolving = false }
    }
    private func accept(_ value: ImportPreview?, for id: String) throws {
        guard lookupID == id else { return }
        guard let value else { throw UserFacingError(message:"The file list could not be read. Try another torrent file or magnet link.") }
        preview = value; selected = Set(value.files.map(\.index))
    }
}
