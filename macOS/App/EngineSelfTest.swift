import Foundation

@MainActor enum EngineSelfTest {
    #if SEEDHOP_DEVELOPMENT
    static func runStorageRecovery() async throws {
        let args = ProcessInfo.processInfo.arguments
        guard let at = args.firstIndex(of:"--verify-storage-recovery"), args.indices.contains(at+1) else { throw UserFacingError(message:"Pass an isolated retained-copy fixture directory.") }
        let fixture = URL(fileURLWithPath:args[at+1])
        let source = fixture.appendingPathComponent("source"), destination = fixture.appendingPathComponent("destination")
        let before = try JSONDecoder().decode(LibrarySnapshot.self,from:Data(contentsOf:fixture.appendingPathComponent("before.json")))
        guard let expected = before.torrents.first else { throw UserFacingError(message:"Missing fixture transfer") }
        let bytes = try expected.files.map { try Data(contentsOf:source.appendingPathComponent($0.name)) }
        let retained = destination.appendingPathComponent(expected.files[0].name)
        let edited = try Data(contentsOf:retained)
        let client = EngineClient(); try client.connect(background:false)
        var connect = EngineRequest(command:"connect"); connect.stateDirectory = fixture.appendingPathComponent("state").path; connect.destination = source.path
        let initial = try await client.send(connect)
        guard let notice = initial.snapshot?.storageRecovery, notice.activeLocation == "original", notice.canKeepCopies else { throw UserFacingError(message:"XPC did not report retained copies and the active original location") }
        try await waitForPaused(client,id:expected.id,allFilesSelected:false)
        var retry = EngineRequest(command:"retryStorageRecovery"); retry.id = notice.id
        _ = try await client.send(retry)
        guard try await client.send(EngineRequest(command:"snapshot")).snapshot?.storageRecovery?.id == notice.id,
              try Data(contentsOf:retained) == edited else { throw UserFacingError(message:"Retry removed a changed file or dismissed unresolved recovery") }
        try await waitForPaused(client,id:expected.id,allFilesSelected:false)
        var keep = EngineRequest(command:"keepRecoveryCopies"); keep.id = notice.id
        _ = try await client.send(keep)
        try await waitForPaused(client,id:expected.id,allFilesSelected:false)
        guard let snapshot = try await client.send(EngineRequest(command:"snapshot")).snapshot,
              snapshot.storageRecovery == nil, let transfer = snapshot.torrents.first,
              transfer.id == expected.id, transfer.destination == expected.destination,
              transfer.files.map(\.selected) == expected.files.map(\.selected), transfer.files.map(\.priority) == expected.files.map(\.priority),
              try Data(contentsOf:retained) == edited else { throw UserFacingError(message:"Keep Copies changed intent or failed to resolve the record") }
        for (index, file) in expected.files.enumerated() {
            guard try Data(contentsOf:source.appendingPathComponent(file.name)) == bytes[index] else { throw UserFacingError(message:"Recovery changed original bytes") }
        }
        _ = try await client.send(EngineRequest(command:"shutdown")); client.invalidate()
        try await Task.sleep(for:.milliseconds(600)); try client.connect(background:false)
        guard try await client.send(connect).snapshot?.storageRecovery == nil else { throw UserFacingError(message:"Resolved recovery returned after restart") }
        try await waitForPaused(client,id:expected.id,allFilesSelected:false)
        _ = try await client.send(EngineRequest(command:"shutdown")); client.invalidate()
        print("PASS: packaged XPC recovery notice, active original location, retry preserves edited copy, Keep Copies preserves all bytes and choices, restart clears resolved notice")
    }
    #endif
    static func run() async throws {
        let args=ProcessInfo.processInfo.arguments
        guard let at=args.firstIndex(of:"--verify-engine"),args.indices.contains(at+1) else { throw UserFacingError(message:"Pass a fixture directory after --verify-engine") }
        let fixture=URL(fileURLWithPath:args[at+1])
        let scratch=fixture.appendingPathComponent("xpc-"+UUID().uuidString)
        let downloads=scratch.appendingPathComponent("downloads")
        try FileManager.default.createDirectory(at:downloads,withIntermediateDirectories:true)
        let client=EngineClient(); try client.connect(background:false)
        var connect=EngineRequest(command:"connect"); connect.destination=downloads.path; connect.stateDirectory=scratch.appendingPathComponent("state").path
        let handshake=try await client.send(connect)
        guard handshake.snapshot?.protocolVersion==UInt32(SeedHopIdentity.protocolVersion) else { throw UserFacingError(message:"Handshake version mismatch") }
        let importSession = ImportSession()
        importSession.destination = downloads.path; importSession.paused = true
        await importSession.inspect(file:fixture.appendingPathComponent("fixture.torrent"),using:client.send)
        guard let preview = importSession.preview, preview.files.count == 1, let importedFile = preview.files.first else { throw UserFacingError(message:importSession.error ?? "Expected a single-file fixture preview") }
        let importedPath = downloads.appendingPathComponent(importedFile.name)
        guard let id = await importSession.add(using:client.send) else { throw UserFacingError(message:importSession.error ?? "Missing transfer ID") }
        var limits=EngineRequest(command:"limits"); limits.downloadLimit=250_000; limits.uploadLimit=50_000
        _=try await client.send(limits)
        guard let snapshot=try await client.send(EngineRequest(command:"snapshot")).snapshot,snapshot.torrents.count==1,snapshot.torrents[0].id==id,snapshot.downloadLimit==250_000 else { throw UserFacingError(message:"Snapshot did not reflect saved commands") }
        var detailsRequest = EngineRequest(command:"torrentDetails"); detailsRequest.id = id; detailsRequest.section = "general"
        guard let details = try await client.send(detailsRequest).details,
              details.torrentId == id, details.instanceId == snapshot.instanceId, details.format == "v1",
              details.v1Hash == id, details.v2Hash == nil, details.sessionDownloaded == 0,
              snapshot.torrents[0].networkDownloadedBytes == 0 else { throw UserFacingError(message:"Typed XPC diagnostics lost identity or invented traffic") }
        // Wait for the initial hash before issuing the explicit recheck.
        try await waitForPaused(client, id:id)
        let content = try Data(contentsOf:fixture.appendingPathComponent("seedhop-fixture.bin"))
        try content.write(to:importedPath)
        var recheck = EngineRequest(command:"recheck"); recheck.id = id
        _ = try await client.send(recheck)
        try await waitForPaused(client, id:id)
        var priorities = EngineRequest(command:"priorities"); priorities.id = id; priorities.priorities = [2]
        _ = try await client.send(priorities)
        let beforeRetry = try await client.send(EngineRequest(command:"snapshot")).snapshot?.torrents.first
        var retry = EngineRequest(command:"retry"); retry.id = id
        _ = try await client.send(retry)
        guard let afterRetry = try await client.send(EngineRequest(command:"snapshot")).snapshot?.torrents.first,
              afterRetry.state == "paused", afterRetry.pausedIntent == true, afterRetry.files == beforeRetry?.files,
              afterRetry.errorKind == nil else { throw UserFacingError(message:"A stale Retry changed the paused transfer or its choices") }
        var queue = EngineRequest(command:"queue"); queue.maxActiveDownloads = 1
        _ = try await client.send(queue)
        var move = EngineRequest(command:"queueMove"); move.id = id; move.queuePosition = 1
        _ = try await client.send(move)
        try await verifyFileCancellation(client,id:id,source:importedPath,content:content,scratch:scratch)
        var rename = EngineRequest(command:"rename"); rename.id = id; rename.fileIndex = 0; rename.fileName = "renamed-fixture.bin"
        _ = try await client.send(rename); try await waitForPaused(client,id:id)
        let moved = scratch.appendingPathComponent("moved")
        try FileManager.default.createDirectory(at:moved,withIntermediateDirectories:true)
        var relocate = EngineRequest(command:"relocate"); relocate.id = id; relocate.destination = moved.path
        _ = try await client.send(relocate); try await waitForPaused(client,id:id)
        guard try Data(contentsOf:moved.appendingPathComponent("renamed-fixture.bin")) == content,
              !FileManager.default.fileExists(atPath:importedPath.path) else { throw UserFacingError(message:"XPC file move changed bytes or retained its source") }
        var policy = EngineRequest(command:"peerEncryption"); policy.peerEncryption = .prefer
        _ = try await client.send(policy)
        let pendingSettings = scratch.appendingPathComponent("state/connection-settings.pending")
        try FileManager.default.createDirectory(at:pendingSettings,withIntermediateDirectories:false)
        policy.peerEncryption = .require
        var rejected = false
        do { _ = try await client.send(policy) } catch { rejected = true }
        guard rejected, let unchanged = try await client.send(EngineRequest(command:"snapshot")).snapshot,
              unchanged.peerEncryption == .allow, unchanged.savedPeerEncryption == .prefer else {
            throw UserFacingError(message:"A rejected policy save changed active or saved settings")
        }
        try FileManager.default.removeItem(at:pendingSettings)
        _ = try await client.send(policy)
        guard let pending = try await client.send(EngineRequest(command:"snapshot")).snapshot,
              pending.peerEncryption == .allow, pending.savedPeerEncryption == .require else {
            throw UserFacingError(message:"Policy save did not distinguish active and next-start settings")
        }
        _=try await client.send(EngineRequest(command:"shutdown")); client.invalidate()
        try await Task.sleep(for:.milliseconds(600))
        try client.connect(background:false)
        guard let restored=try await client.send(connect).snapshot,restored.torrents.count==1,restored.downloadLimit==250_000 else { throw UserFacingError(message:"Restore did not preserve library state") }
        try await waitForPaused(client,id:id)
        guard let item = try await client.send(EngineRequest(command:"snapshot")).snapshot?.torrents.first,
              item.id == id, item.files[0].priority == 2, item.files[0].name == "renamed-fixture.bin", item.destination == moved.path,
              restored.maxActiveDownloads == 1, restored.peerEncryption == .require, restored.savedPeerEncryption == .require else { throw UserFacingError(message:"Restoration lost file management settings") }
        var remove=EngineRequest(command:"remove"); remove.id=id; remove.deleteFiles=false
        _=try await client.send(remove)
        _=try await client.send(EngineRequest(command:"shutdown")); client.invalidate()
        print("PASS: XPC requirement, protocol v\(SeedHopIdentity.protocolVersion), typed diagnostics, import, Force Recheck, stale Retry preserves paused choices, policy save failure and retry, policy activation after restart, priorities, queue, live file progress, move/rename cancellation, stale cancellation, rename, relocation, exact bytes, restart, paused selection, removal, shutdown")
    }
    private static func verifyFileCancellation(_ client: EngineClient, id: String, source: URL, content: Data, scratch: URL) async throws {
        let destination = scratch.appendingPathComponent("cancelled-move")
        try FileManager.default.createDirectory(at:destination,withIntermediateDirectories:true)
        let unrelated = destination.appendingPathComponent("unrelated.txt")
        try Data("keep".utf8).write(to:unrelated)
        for command in ["relocate", "rename"] {
            // A sparse extension keeps this isolated fixture cheap to create,
            // while providing enough logical copy work to observe over XPC.
            let file = try FileHandle(forWritingTo:source)
            defer { try? file.truncate(atOffset:UInt64(content.count)); try? file.close() }
            let copyLength: UInt64 = 512 * 1024 * 1024
            try file.truncate(atOffset:copyLength)
            var request = EngineRequest(command:command); request.id = id
            request.destination = destination.path; request.fileIndex = 0; request.fileName = "cancelled-rename.bin"
            var finished = false
            let change = Task { @MainActor [request] in
                defer { finished = true }
                return try await client.send(request)
            }
            var cancellationID: String?
            var observations = Set<String>()
            // Preparation can outlast a fixed number of fast XPC round trips.
            // Keep observing by elapsed time so the fixture can cancel actual copying.
            let observationDeadline = Date(timeIntervalSinceNow:120)
            while Date() < observationDeadline {
                let snapshot = try await client.send(EngineRequest(command:"snapshot")).snapshot
                if observations.count < 16 { observations.insert(snapshot?.fileOperation.map { "\($0.phase):\($0.completedBytes)" } ?? "idle") }
                if let operation = snapshot?.fileOperation,
                   operation.torrentId == id, operation.phase == "copying", operation.completedBytes > 0 {
                    guard operation.totalBytes == copyLength, operation.completedBytes <= operation.totalBytes else {
                        throw UserFacingError(message:"XPC copy progress did not reflect actual logical bytes")
                    }
                    var cancel = EngineRequest(command:"cancelFileOperation"); cancel.id = operation.id
                    if try await client.send(cancel).cancellationAccepted == true { cancellationID = operation.id; break }
                }
                if finished { break }
                try await Task.sleep(for:.milliseconds(5))
            }
            var cancelled = false
            do { _ = try await change.value }
            catch is CancellationError { cancelled = true }
            guard cancelled, let cancellationID else { throw UserFacingError(message:"XPC \(command) did not cancel during copying; observed \(observations.sorted().joined(separator:", "))") }
            try file.truncate(atOffset:UInt64(content.count))
            guard try Data(contentsOf:source) == content,
                  !FileManager.default.fileExists(atPath:destination.appendingPathComponent(source.lastPathComponent).path),
                  !FileManager.default.fileExists(atPath:source.deletingLastPathComponent().appendingPathComponent("cancelled-rename.bin").path),
                  try Data(contentsOf:unrelated) == Data("keep".utf8),
                  let snapshot = try await client.send(EngineRequest(command:"snapshot")).snapshot,
                  snapshot.fileOperation == nil,
                  let item = snapshot.torrents.first(where:{ $0.id == id }), item.isPaused,
                  item.files[0].selected, item.files[0].priority == 2, item.files[0].name == source.lastPathComponent else {
                throw UserFacingError(message:"XPC cancellation changed original files or transfer intent")
            }
            var stale = EngineRequest(command:"cancelFileOperation"); stale.id = cancellationID
            guard try await client.send(stale).cancellationAccepted == false else { throw UserFacingError(message:"Finished operation accepted stale cancellation") }
            print("PASS: packaged XPC \(command) progress and cancellation preserve exact bytes and paused choices")
        }
    }
    private static func waitForPaused(_ client: EngineClient, id: String, allFilesSelected: Bool = true) async throws {
        for _ in 0..<100 {
            if let item = try await client.send(EngineRequest(command:"snapshot")).snapshot?.torrents.first(where: { $0.id == id }), item.state == "paused", !allFilesSelected || item.files.allSatisfy(\.selected) { return }
            try await Task.sleep(for:.milliseconds(50))
        }
        throw UserFacingError(message:"Recheck did not retain paused intent and file selection")
    }
}
