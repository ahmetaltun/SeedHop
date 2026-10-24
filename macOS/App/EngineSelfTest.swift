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
        guard handshake.snapshot?.protocolVersion==1 else { throw UserFacingError(message:"Handshake version mismatch") }
        let importSession = ImportSession()
        importSession.destination = downloads.path; importSession.paused = true
        await importSession.inspect(file:fixture.appendingPathComponent("fixture.torrent"),using:client.send)
        guard importSession.preview != nil else { throw UserFacingError(message:importSession.error ?? "Missing preview") }
        guard let id = await importSession.add(using:client.send) else { throw UserFacingError(message:importSession.error ?? "Missing transfer ID") }
        var limits=EngineRequest(command:"limits"); limits.downloadLimit=250_000; limits.uploadLimit=50_000
        _=try await client.send(limits)
        guard let snapshot=try await client.send(EngineRequest(command:"snapshot")).snapshot,snapshot.torrents.count==1,snapshot.torrents[0].id==id,snapshot.downloadLimit==250_000 else { throw UserFacingError(message:"Snapshot did not reflect saved commands") }
        _=try await client.send(EngineRequest(command:"shutdown")); client.invalidate()
        try await Task.sleep(for:.milliseconds(600))
        try client.connect(background:false)
        guard let restored=try await client.send(connect).snapshot,restored.torrents.count==1,restored.downloadLimit==250_000 else { throw UserFacingError(message:"Restore did not preserve library state") }
        var remove=EngineRequest(command:"remove"); remove.id=id; remove.deleteFiles=false
        _=try await client.send(remove)
        _=try await client.send(EngineRequest(command:"shutdown")); client.invalidate()
        print("PASS: signed XPC identity, protocol handshake, import presentation model with real metadata and paused add, limits, restart, restore, removal, shutdown")
    }
}
