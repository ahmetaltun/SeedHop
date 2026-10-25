#if SEEDHOP_VALIDATION
import Foundation
import ServiceManagement

/// Real certificate-authenticated foreground/background handoffs in a separate
/// app/service namespace. Login policy cases simulate a changed login scope;
/// actual logout/login is still a target-system acceptance check.
@MainActor enum LifecycleSelfTest {
    static func run() async throws {
        let args = ProcessInfo.processInfo.arguments
        guard let at = args.firstIndex(of:"--verify-lifecycle"), args.indices.contains(at+1) else {
            throw UserFacingError(message:"Pass the generated fixture directory after --verify-lifecycle")
        }
        let fixture = URL(fileURLWithPath:args[at+1])
        let root = URL(fileURLWithPath:ValidationConfiguration.root)
        let downloads = root.appendingPathComponent("downloads")
        try FileManager.default.createDirectory(at:downloads,withIntermediateDirectories:true)
        let content = try Data(contentsOf:fixture.appendingPathComponent("seedhop-fixture.bin"))
        let bytes = try Data(contentsOf:fixture.appendingPathComponent("fixture.torrent"))
        guard let encoded = String(data:bytes,encoding:.isoLatin1), encoded.contains("seedhop-fixture.bin") else {
            throw UserFacingError(message:"Use seedhop-lab fixture output")
        }
        // Replace only the name bytes; torrent piece hashes remain untouched.
        var pausedBytes = bytes
        guard let nameRange = pausedBytes.range(of:Data("seedhop-fixture.bin".utf8)) else { throw UserFacingError(message:"Missing fixture name") }
        pausedBytes.replaceSubrange(nameRange,with:Data("pausedx-fixture.bin".utf8))
        try content.write(to:downloads.appendingPathComponent("seedhop-fixture.bin"))
        try content.write(to:downloads.appendingPathComponent("pausedx-fixture.bin"))
        let service = SMAppService.agent(plistName:SeedHopIdentity.agentPlist)
        let client = EngineClient()
        do {
            _ = try await connect(client,background:false)
            var add = EngineRequest(command:"add"); add.destination = downloads.path
            add.bytes = bytes; add.selected = [0]; add.paused = false; add.reuse = true
            guard let running = try await client.send(add).id else { throw UserFacingError(message:"Missing running transfer") }
            add.bytes = pausedBytes; add.paused = true
            guard let paused = try await client.send(add).id else { throw UserFacingError(message:"Missing paused transfer") }
            try await assertIntent(client,running:running,paused:paused)
            for cycle in 1...3 {
                try register(service)
                try await shutdown(client)
                _ = try await connect(client,background:true)
                try await assertIntent(client,running:running,paused:paused)
                try await shutdown(client)
                try await service.unregister()
                _ = try await connect(client,background:false)
                try await assertIntent(client,running:running,paused:paused)
                print("PASS: certificate-backed host handoff cycle \(cycle); running and paused intent retained")
            }
            try await shutdown(client)
            let intentURL = SeedHopIdentity.library.appendingPathComponent("run-intent.json")
            let original = try JSONSerialization.jsonObject(with:Data(contentsOf:intentURL)) as! [String:Any]
            for (resume, sameSession, shouldRestore) in [(false,false,false),(true,false,true),(false,true,true)] {
                var saved = original
                saved["session"] = sameSession ? original["session"] : "validation-previous-login"
                saved["running"] = true; saved["resumeAtLogin"] = resume
                try JSONSerialization.data(withJSONObject:saved).write(to:intentURL,options:.atomic)
                try register(service)
                try client.connect(background:true)
                var restored: LibrarySnapshot?
                // Off must remain idle; status is deliberately unable to start it.
                for _ in 0..<40 {
                    restored = try await client.send(EngineRequest(command:"validationStatus")).snapshot
                    if restored != nil { break }
                    try await Task.sleep(for:.milliseconds(100))
                }
                guard (restored != nil) == shouldRestore else { throw UserFacingError(message:"Automatic restoration ignored login policy") }
                if shouldRestore { try await assertIntent(client,running:running,paused:paused) }
                try await shutdown(client)
                try await service.unregister()
                print("PASS: simulated login scope, automatic resume=\(resume), same session=\(sameSession), restored=\(shouldRestore)")
            }
            for name in ["seedhop-fixture.bin", "pausedx-fixture.bin"] {
                guard try Data(contentsOf:downloads.appendingPathComponent(name)) == content else { throw UserFacingError(message:"Lifecycle fixture bytes changed") }
            }
            print("PASS: exact bytes retained. Actual logout/login, helper updates, and revoked permission remain separate acceptance checks.")
        } catch {
            client.invalidate()
            if service.status != .notRegistered { try? await service.unregister() }
            throw error
        }
    }
    private static func register(_ service: SMAppService) throws {
        if service.status == .notRegistered || service.status == .notFound { try service.register() }
        guard service.status == .enabled else { throw UserFacingError(message:"Allow the isolated SeedHop validation agent in Login Items & Extensions, then rerun this fixture.") }
    }
    private static func shutdown(_ client: EngineClient) async throws {
        _ = try await client.send(EngineRequest(command:"shutdown")); client.invalidate()
        try await Task.sleep(for:.milliseconds(600))
    }
    private static func connect(_ client: EngineClient, background: Bool) async throws -> LibrarySnapshot {
        var failure: Error = UserFacingError(message:"The validation host did not connect")
        for _ in 0..<30 {
            do {
                try client.connect(background:background)
                if let snapshot = try await client.send(EngineRequest(command:"connect")).snapshot { return snapshot }
            } catch { failure = error }
            client.invalidate(); try await Task.sleep(for:.milliseconds(200))
        }
        throw failure
    }
    private static func assertIntent(_ client: EngineClient, running: String, paused: String) async throws {
        for _ in 0..<100 {
            if let snapshot = try await client.send(EngineRequest(command:"snapshot")).snapshot,
               snapshot.torrents.count == 2,
               snapshot.torrents.first(where: { $0.id == running })?.state == "seeding",
               let stopped = snapshot.torrents.first(where: { $0.id == paused }),
               stopped.state == "paused", stopped.files.allSatisfy(\.selected) { return }
            try await Task.sleep(for:.milliseconds(50))
        }
        throw UserFacingError(message:"The host switch changed transfer identity, pause intent, file choices, or completion")
    }
}
#endif
