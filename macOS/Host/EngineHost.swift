import Foundation
import Security

struct RunIntent: Codable {
    var session: String
    var running: Bool
    var resumeAtLogin: Bool
    static var currentSession: String {
        var session: SecuritySessionId = 0
        guard SessionGetInfo(callerSecuritySession, &session, nil) == errSecSuccess else { return "unknown-\(ProcessInfo.processInfo.processIdentifier)" }
        var boot = timeval()
        var length = MemoryLayout<timeval>.size
        sysctlbyname("kern.boottime", &boot, &length, nil, 0)
        return "\(boot.tv_sec)-\(session)"
    }
}

final class EngineHost: NSObject, EngineServiceProtocol, NSXPCListenerDelegate {
    private let queue = DispatchQueue(label: "app.seedhop.host", qos: .userInitiated, attributes: .concurrent)
    private let lifecycle = NSLock()
    private var engine: EngineBridge?
    private var stopping = false
    private var connections: [NSXPCConnection] = []
    private let background: Bool
    private var runIntent = RunIntent(session: RunIntent.currentSession, running: false, resumeAtLogin: false)
    private var activity: NSObjectProtocol?
    private var libraryDirectory = SeedHopIdentity.library
    private var intentURL: URL { libraryDirectory.appendingPathComponent("run-intent.json") }

    init(background: Bool) {
        self.background = background
        super.init()
        if background, let bytes = try? Data(contentsOf: intentURL), let saved = try? JSONDecoder().decode(RunIntent.self, from: bytes) {
            runIntent = saved
            if saved.running && (saved.session == RunIntent.currentSession || saved.resumeAtLogin) {
                queue.async { do { _ = try self.start() } catch { NSLog("SeedHop: automatic restore failed: %@", error.localizedDescription) } }
            }
        }
    }
    private func persistIntent() throws {
        try FileManager.default.createDirectory(at: SeedHopIdentity.library, withIntermediateDirectories: true)
        let data = try JSONEncoder().encode(runIntent)
        try data.write(to: intentURL, options: [.atomic])
        let fd = open(intentURL.path, O_RDONLY)
        if fd >= 0 { defer { close(fd) }; guard fsync(fd) == 0 else { throw HostFailure(code:"host.background_save") } }
    }
    private func start(destination: String? = nil, stateDirectory: String? = nil) throws -> EngineBridge {
        lifecycle.lock(); defer { lifecycle.unlock() }
        if stopping { throw HostFailure(code:"host.stopping") }
        if let engine { return engine }
        #if SEEDHOP_DEVELOPMENT
        if let stateDirectory { libraryDirectory = URL(fileURLWithPath:stateDirectory) }
        #endif
        #if SEEDHOP_VALIDATION
        let downloads = URL(fileURLWithPath:ValidationConfiguration.root).appendingPathComponent("downloads").path
        #else
        let downloads = destination ?? UserDefaults.standard.string(forKey: "downloadDirectory") ?? FileManager.default.urls(for: .downloadsDirectory, in: .userDomainMask)[0].path
        #endif
        let core = try EngineBridge(stateDirectory: libraryDirectory.path, downloadDirectory: downloads, discovery: true)
        engine = core
        runIntent.session = RunIntent.currentSession; runIntent.running = true
        try persistIntent()
        activity = ProcessInfo.processInfo.beginActivity(options: [.userInitiatedAllowingIdleSystemSleep], reason: "Maintaining torrent transfers")
        return core
    }
    func listener(_ listener: NSXPCListener, shouldAcceptNewConnection connection: NSXPCConnection) -> Bool {
        do { connection.setCodeSigningRequirement(try Signing.requirement(at: Signing.containingApp)) }
        catch { NSLog("SeedHop: rejected connection: %@", error.localizedDescription); return false }
        connection.exportedInterface = NSXPCInterface(with: EngineServiceProtocol.self)
        connection.exportedObject = self
        lifecycle.lock(); connections.append(connection); lifecycle.unlock()
        connection.invalidationHandler = { [weak self, weak connection] in
            guard let self else { return }
            self.lifecycle.lock(); self.connections.removeAll { $0 === connection }; let empty = self.connections.isEmpty; self.lifecycle.unlock()
            if empty && !self.background { self.queue.async { self.stopForDisconnect() } }
        }
        connection.resume()
        return true
    }
    func send(_ data: Data, withReply reply: @escaping (Data) -> Void) {
        queue.async {
            let response: EngineReply
            do {
                let request = try JSONDecoder().decode(EngineRequest.self, from: data)
                guard request.version == SeedHopIdentity.protocolVersion else { throw HostFailure(code:"host.version_mismatch") }
                response = try self.handle(request)
            } catch BridgeError.Cancelled { response = EngineReply(cancelled:true) }
            catch let BridgeError.Operation(message, notice) { response = EngineReply(error:message,errorMessage:Self.message(notice)) }
            catch let failure as HostFailure { response = EngineReply(errorMessage:EngineMessage(code:failure.code)) }
            catch { response = EngineReply(errorMessage:EngineMessage(code:"engine.failed",diagnostic:error.localizedDescription)) }
            reply((try? JSONEncoder().encode(response)) ?? Data("{\"version\":6,\"errorMessage\":{\"code\":\"engine.failed\",\"arguments\":[],\"diagnostic\":\"Could not encode engine response.\"}}".utf8))
        }
    }
    private func handle(_ r: EngineRequest) throws -> EngineReply {
        #if SEEDHOP_VALIDATION
        // Observe restoration without the usual lazy start on an explicit Connect.
        if r.command == "validationStatus" {
            lifecycle.lock(); defer { lifecycle.unlock() }
            return EngineReply(snapshot:try engine.map { Self.snapshot(try $0.snapshot()) })
        }
        #endif
        if r.command == "shutdown" {
            lifecycle.lock(); stopping = true; let current = engine; lifecycle.unlock()
            if r.explicitStop == true, let current {
                for t in try current.snapshot().torrents where t.state != "paused" { try current.setPaused(id: t.id, paused: true) }
            }
            try current?.shutdown()
            lifecycle.lock(); defer { lifecycle.unlock() }; engine = nil
            runIntent.running = false
            try persistIntent()
            if let activity { ProcessInfo.processInfo.endActivity(activity); self.activity = nil }
            // Reply first; process exit is the final disk-ownership barrier.
            DispatchQueue.global().asyncAfter(deadline: .now() + 0.2) { exit(0) }
            return EngineReply()
        }
        let core = try start(destination: r.destination,stateDirectory:r.stateDirectory)
        switch r.command {
        case "connect", "snapshot": return EngineReply(snapshot: try JSONDecoder().decode(LibrarySnapshot.self, from:Data(core.snapshotJson().utf8)))
        case "torrentDetails": return EngineReply(details: try JSONDecoder().decode(TorrentDetails.self, from:Data(core.torrentDetails(id:r.id ?? "",section:r.section ?? "general",allPeers:r.allPeers ?? false).utf8)))
        case "inspect":
            let p = try core.inspect(requestId: r.requestId, magnet: r.magnet, bytes: r.bytes ?? Data())
            return EngineReply(preview: ImportPreview(id: p.id, name:p.name, totalBytes:p.totalBytes, files:p.files.map(Self.file), torrentBytes:Data(p.torrentBytes)))
        case "cancel": try core.cancel(requestId: r.id ?? r.requestId)
        case "cancelFileOperation": return EngineReply(cancellationAccepted:try core.cancelFileOperation(operationId:r.id ?? ""))
        case "retryStorageRecovery", "keepRecoveryCopies": try core.resolveStorageRecovery(id:r.id ?? "",keepCopies:r.command == "keepRecoveryCopies")
        case "add":
            let id = try core.addWithOptions(bytes:r.bytes ?? Data(), destination:r.destination ?? "", selected:r.selected ?? [], options:TorrentAddOptions(paused:r.paused ?? false, reuse:r.reuse ?? false, addToTop:r.addToTop ?? false, sequential:r.sequential ?? false, firstLastPieces:r.firstLastPieces ?? false))
            return EngineReply(id:id)
        case "pause": try core.setPaused(id:r.id ?? "", paused:r.paused ?? true)
        case "select": try core.selectFiles(id:r.id ?? "", indices:r.selected ?? [])
        case "recheck": try core.forceRecheck(id:r.id ?? "")
        case "retry": try core.retryTransfer(id:r.id ?? "")
        case "priorities": try core.setFilePriorities(id:r.id ?? "", priorities:Data(r.priorities ?? []))
        case "queue": try core.setQueue(maxActive:r.maxActiveDownloads ?? 0)
        case "queueMove": try core.moveInQueue(id:r.id ?? "", position:r.queuePosition ?? 1)
        case "relocate": try core.relocate(id:r.id ?? "", destination:r.destination ?? "")
        case "rename": try core.renameFile(id:r.id ?? "", index:r.fileIndex ?? UInt32.max, name:r.fileName ?? "")
        case "remove": try core.remove(id:r.id ?? "", deleteFiles:r.deleteFiles ?? false)
        case "peerEncryption":
            guard let policy = r.peerEncryption else { throw HostFailure(code:"host.encryption") }
            try core.setPeerEncryption(policy:policy.rawValue)
        case "schedule":
            guard let schedule = r.schedule else { throw HostFailure(code:"host.schedule") }
            try core.setSchedule(enabled:schedule.enabled,rules:schedule.rules.map { rule in
                ScheduleWindow(days:rule.days,startMinute:rule.startMinute,endMinute:rule.endMinute,downloadLimit:rule.downloadLimit,uploadLimit:rule.uploadLimit,suspend:rule.suspend)
            })
        case "alternativeLimits":
            guard let down = r.downloadLimit, let up = r.uploadLimit, let enabled = r.alternativeLimitsEnabled else { throw HostFailure(code:"host.bandwidth") }
            try core.setAlternativeLimits(download:down, upload:up, enabled:enabled)
        case "limits": try core.setLimits(download:r.downloadLimit ?? 0, upload:r.uploadLimit ?? 0)
        case "login":
            lifecycle.lock(); defer { lifecycle.unlock() }
            runIntent.resumeAtLogin = r.resumeAtLogin ?? false; try persistIntent()
        default: throw HostFailure(code:"host.unsupported")
        }
        return EngineReply()
    }
    private func stopForDisconnect() {
        lifecycle.lock(); guard !stopping else { lifecycle.unlock(); return }; lifecycle.unlock()
        do { _ = try handle(EngineRequest(command:"shutdown")) }
        catch { NSLog("SeedHop: shutdown failed: %@", error.localizedDescription) }
    }
    static func file(_ f: TransferFile) -> FileItem { FileItem(index:f.index,name:f.name,size:f.size,downloaded:f.downloaded,selected:f.selected,priority:f.priority) }
    static func message(_ notice: UserNotice) -> EngineMessage {
        EngineMessage(code:notice.code,arguments:notice.arguments.map { argument in
            switch argument { case .text(let value): return .text(value); case .number(let value): return .number(value) }
        },diagnostic:notice.diagnostic)
    }
    static func snapshot(_ s: EngineSnapshot) -> LibrarySnapshot {
        var snapshot = LibrarySnapshot(protocolVersion:s.protocolVersion, engineVersion:s.engineVersion, instanceId:s.instanceId,
            listenPort:s.listenPort,downloadLimit:s.downloadLimit,uploadLimit:s.uploadLimit,maxActiveDownloads:s.maxActiveDownloads,torrents:s.torrents.map { t in
                TorrentItem(id:t.id,name:t.name,state:t.state,totalBytes:t.totalBytes,downloadedBytes:t.downloadedBytes,
                    uploadedBytes:t.uploadedBytes,downloadRate:t.downloadRate,uploadRate:t.uploadRate,peers:t.peers,
                    destination:t.destination,files:t.files.map(file),trackers:t.trackers,error:t.error,queuePosition:t.queuePosition,pausedIntent:t.pausedIntent,errorKind:t.errorKind)
            },fileOperation:s.fileOperation.map { operation in
                FileOperationItem(id:operation.id,torrentId:operation.torrentId,kind:operation.kind,
                    phase:operation.phase,completedBytes:operation.completedBytes,totalBytes:operation.totalBytes,
                    cancelRequested:operation.cancelRequested)
            },storageRecovery:s.storageRecovery.map { notice in
                StorageRecoveryItem(id:notice.id,torrentId:notice.torrentId,activeLocation:notice.activeLocation,message:notice.message,
                    originalDirectory:notice.originalDirectory,destinationDirectory:notice.destinationDirectory,canKeepCopies:notice.canKeepCopies)
            })
        snapshot.schedule = ScheduleConfig(enabled:s.scheduleEnabled,rules:s.scheduleRules.map { rule in
            ScheduleRule(days:rule.days,startMinute:rule.startMinute,endMinute:rule.endMinute,downloadLimit:rule.downloadLimit,uploadLimit:rule.uploadLimit,suspend:rule.suspend)
        })
        snapshot.scheduleActive = s.scheduleActive
        snapshot.statisticsMessage = s.statisticsMessage.map(message)
        snapshot.schedulingMessage = s.schedulingMessage.map(message)
        snapshot.schedulingWarning = s.schedulingWarning
        snapshot.normalDownloadLimit = s.normalDownloadLimit
        snapshot.normalUploadLimit = s.normalUploadLimit
        snapshot.alternativeDownloadLimit = s.alternativeDownloadLimit
        snapshot.alternativeUploadLimit = s.alternativeUploadLimit
        snapshot.alternativeLimitsEnabled = s.alternativeLimitsEnabled
        snapshot.peerEncryption = PeerEncryptionPolicy(rawValue:s.peerEncryption)
        snapshot.savedPeerEncryption = PeerEncryptionPolicy(rawValue:s.savedPeerEncryption)
        return snapshot
    }
}

private struct HostFailure: Error { let code: String }
