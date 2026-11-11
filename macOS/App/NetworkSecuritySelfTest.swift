#if SEEDHOP_VALIDATION
import Foundation

/// Exercises signed XPC using the validation namespace and temporary library only.
@MainActor enum NetworkSecuritySelfTest {
    static func run() async throws {
        func require(_ condition: Bool, _ message: String) throws {
            if !condition { throw UserFacingError(message:message) }
        }
        try FileManager.default.createDirectory(at:URL(fileURLWithPath:ValidationConfiguration.root).appendingPathComponent("downloads"),withIntermediateDirectories:true)
        let client = EngineClient()
        try client.connect(background:false)
        let initial = try await client.send(EngineRequest(command:"networkInfo"))
        try require(initial.networkInfo?.active == nil,"Reading preferences started the engine")
        guard let loopback = initial.networkInfo?.interfaces.first(where: { $0.name == "lo0" }) else {
            throw UserFacingError(message:"Loopback interface unavailable")
        }
        func save(_ policy: NetworkPolicy) async throws -> NetworkInfo {
            var request = EngineRequest(command:"networkPolicy"); request.networkPolicy = policy
            guard let result = try await client.send(request).networkInfo else {
                throw UserFacingError(message:"Missing network policy reply")
            }
            return result
        }
        func stop() async throws {
            var request = EngineRequest(command:"shutdown"); request.explicitStop = true
            let reply = try await client.send(request)
            guard let pid = reply.stoppedProcessID else { throw UserFacingError(message:"Missing process ownership barrier") }
            client.invalidate()
            for _ in 0..<100 {
                if kill(pid,0) != 0 && errno == ESRCH { return }
                try await Task.sleep(for:.milliseconds(100))
            }
            throw UserFacingError(message:"Engine process did not exit")
        }
        let protected = NetworkPolicy(interface:loopback,dnsServer:"1.1.1.1",allowLocalNetwork:true)
        let saved = try await save(protected)
        try require(saved.active == nil && saved.saved == protected,"Offline policy was not saved before startup")
        let started = try await client.send(EngineRequest(command:"connect"))
        try require(started.snapshot?.networkInterface == "lo0" && started.snapshot?.networkBlocked == false,"Startup ignored the selected interface")
        let direct = NetworkPolicy(allowLocalNetwork:true)
        let stopped = try await save(direct)
        try require(stopped.blocked && stopped.saved == direct,"Policy change did not stop active connections")
        let blocked = try await client.send(EngineRequest(command:"snapshot"))
        try require(blocked.snapshot?.networkBlocked == true,"Blocked snapshot was unavailable")
        do {
            var request = EngineRequest(command:"peerEncryption"); request.peerEncryption = .require
            _ = try await client.send(request)
            throw UserFacingError(message:"Blocked engine accepted a transfer-setting mutation")
        } catch let message as EngineMessage {
            try require(message.code == "network.blocked","Blocked engine returned the wrong failure")
        }
        try await stop()
        try client.connect(background:false)
        _ = try await save(protected)
        // Model interface disappearance between saving and the next launch.
        var missing = protected; missing.interface?.index = UInt32.max
        try JSONEncoder().encode(missing).write(to:SeedHopIdentity.library.appendingPathComponent("network-policy.json"),options:.atomic)
        do {
            _ = try await client.send(EngineRequest(command:"connect"))
            throw UserFacingError(message:"Unavailable interface silently fell back")
        } catch let message as EngineMessage {
            try require(message.code == "network.blocked","Unavailable interface returned the wrong failure")
        }
        _ = try await save(direct)
        let recovered = try await client.send(EngineRequest(command:"connect"))
        try require(recovered.snapshot?.networkInterface == nil && recovered.snapshot?.networkBlocked == false,"Offline settings could not recover blocked startup")
        try await stop()
        print("PASS: signed XPC policy save, bound startup, live stop, blocked status, mutation denial, missing-interface startup, offline recovery, and process exit")
    }
}
#endif
