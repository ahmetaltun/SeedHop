import Foundation

@MainActor enum EngineSelfTest {
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
        // Wait for the initial hash before issuing the explicit recheck.
        try await waitForPaused(client, id:id)
        let content = try Data(contentsOf:fixture.appendingPathComponent("seedhop-fixture.bin"))
        try content.write(to:importedPath)
        var recheck = EngineRequest(command:"recheck"); recheck.id = id
        _ = try await client.send(recheck)
        try await waitForPaused(client, id:id)
        var priorities = EngineRequest(command:"priorities"); priorities.id = id; priorities.priorities = [2]
        _ = try await client.send(priorities)
        var queue = EngineRequest(command:"queue"); queue.maxActiveDownloads = 1
        _ = try await client.send(queue)
        var move = EngineRequest(command:"queueMove"); move.id = id; move.queuePosition = 1
        _ = try await client.send(move)
        var rename = EngineRequest(command:"rename"); rename.id = id; rename.fileIndex = 0; rename.fileName = "renamed-fixture.bin"
        _ = try await client.send(rename); try await waitForPaused(client,id:id)
        let moved = scratch.appendingPathComponent("moved")
        try FileManager.default.createDirectory(at:moved,withIntermediateDirectories:true)
        var relocate = EngineRequest(command:"relocate"); relocate.id = id; relocate.destination = moved.path
        _ = try await client.send(relocate); try await waitForPaused(client,id:id)
        guard try Data(contentsOf:moved.appendingPathComponent("renamed-fixture.bin")) == content,
              !FileManager.default.fileExists(atPath:importedPath.path) else { throw UserFacingError(message:"XPC file move changed bytes or retained its source") }
        _=try await client.send(EngineRequest(command:"shutdown")); client.invalidate()
        try await Task.sleep(for:.milliseconds(600))
        try client.connect(background:false)
        guard let restored=try await client.send(connect).snapshot,restored.torrents.count==1,restored.downloadLimit==250_000 else { throw UserFacingError(message:"Restore did not preserve library state") }
        try await waitForPaused(client,id:id)
        guard let item = try await client.send(EngineRequest(command:"snapshot")).snapshot?.torrents.first,
              item.id == id, item.files[0].priority == 2, item.files[0].name == "renamed-fixture.bin", item.destination == moved.path,
              restored.maxActiveDownloads == 1 else { throw UserFacingError(message:"Restoration lost file management settings") }
        var remove=EngineRequest(command:"remove"); remove.id=id; remove.deleteFiles=false
        _=try await client.send(remove)
        _=try await client.send(EngineRequest(command:"shutdown")); client.invalidate()
        print("PASS: XPC requirement, protocol v2, import, Force Recheck, priorities, queue, rename, relocation, exact bytes, restart, paused selection, removal, shutdown")
    }
    private static func waitForPaused(_ client: EngineClient, id: String) async throws {
        for _ in 0..<100 {
            if let item = try await client.send(EngineRequest(command:"snapshot")).snapshot?.torrents.first(where: { $0.id == id }), item.state == "paused", item.files.allSatisfy(\.selected) { return }
            try await Task.sleep(for:.milliseconds(50))
        }
        throw UserFacingError(message:"Recheck did not retain paused intent and file selection")
    }
}
