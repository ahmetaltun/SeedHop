import Foundation
import Security

struct RunIntent: Codable {
    var session: String
    var running: Bool
    var resumeAtLogin: Bool
    var downloadDirectory: String? = nil
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
    private let requestSlots = DispatchSemaphore(value:8)
    private var engine: EngineBridge?
    private var stopping = false
    private var connections: [NSXPCConnection] = []
    private let background: Bool
    private let privateSession: Bool
    private var privateFolder: URL?
    private var runIntent = RunIntent(session: RunIntent.currentSession, running: false, resumeAtLogin: false)
    private var activity: NSObjectProtocol?
    private var fixtureDirectory: URL?
    private var libraryDirectory: URL { get throws { try fixtureDirectory ?? SeedHopIdentity.library } }
    #if SEEDHOP_SANDBOX
    private lazy var folderAccess = FolderAccessStore(role: background ? "background" : "foreground")
    #endif
    private var intentURL: URL { get throws { try libraryDirectory.appendingPathComponent("run-intent.json") } }

    init(background: Bool, privateSession: Bool = false) {
        self.background = background
        self.privateSession = privateSession
        super.init()
        if background, let bytes = try? Data(contentsOf: intentURL), let saved = try? JSONDecoder().decode(RunIntent.self, from: bytes) {
            runIntent = saved
            if saved.running && (saved.session == RunIntent.currentSession || saved.resumeAtLogin) {
                queue.async { do { _ = try self.start() } catch { NSLog("SeedHop: automatic restore failed.") } }
            }
        }
    }
    private func persistIntent() throws {
        let intentURL = try self.intentURL
        try FileManager.default.createDirectory(at:intentURL.deletingLastPathComponent(),withIntermediateDirectories:true)
        let data = try JSONEncoder().encode(runIntent)
        try data.write(to: intentURL, options: [.atomic])
        let fd = open(intentURL.path, O_RDONLY)
        if fd >= 0 { defer { close(fd) }; guard fsync(fd) == 0 else { throw HostFailure(code:"host.background_save") } }
    }
    private func start(destination: String? = nil, stateDirectory: String? = nil) throws -> EngineBridge {
        lifecycle.lock(); defer { lifecycle.unlock() }
        if stopping { throw HostFailure(code:"host.stopping") }
        if let engine { return engine }
        #if SEEDHOP_DEVELOPMENT && !SEEDHOP_SANDBOX
        if let stateDirectory { fixtureDirectory = URL(fileURLWithPath:stateDirectory) }
        #endif
        #if SEEDHOP_SANDBOX
        guard let downloads = destination ?? runIntent.downloadDirectory else { throw HostFailure(code:"host.folder_access") }
        #elseif SEEDHOP_VALIDATION
        let downloads = URL(fileURLWithPath:ValidationConfiguration.root).appendingPathComponent("downloads").path
        #else
        let downloads = destination ?? UserDefaults.standard.string(forKey: "downloadDirectory") ?? FileManager.default.urls(for: .downloadsDirectory, in: .userDomainMask)[0].path
        #endif
        #if SEEDHOP_VALIDATION
        let allowLocalNetwork = true
        let discovery = false
        #else
        let allowLocalNetwork = false
        let discovery = true
        #endif
        #if SEEDHOP_SANDBOX
        do {
            try folderAccess.requireAccess(to: downloads)
            let sessionURL = try libraryDirectory.appendingPathComponent("session/session.json")
            if FileManager.default.fileExists(atPath: sessionURL.path) {
                let session = try JSONSerialization.jsonObject(with: Data(contentsOf: sessionURL)) as? [String: Any]
                guard let session, let torrents = session["torrents"] as? [String: [String: Any]] else { throw FolderAccessFailure.unavailable }
                for torrent in torrents.values {
                    guard let folder = torrent["output_folder"] as? String else { throw FolderAccessFailure.unavailable }
                    try folderAccess.requireAccess(to: folder)
                }
            }
        } catch { throw HostFailure(code:FolderAccessFailure.noticeCode(for:error)) }
        #endif
        let core = try EngineBridge(stateDirectory: libraryDirectory.path, downloadDirectory: downloads, discovery: discovery, allowLocalNetwork:allowLocalNetwork)
        runIntent.session = RunIntent.currentSession; runIntent.running = true
        runIntent.downloadDirectory = downloads
        do { try persistIntent() }
        catch { try? core.shutdown(); throw error }
        engine = core
        activity = ProcessInfo.processInfo.beginActivity(options: [.userInitiatedAllowingIdleSystemSleep], reason: "Maintaining torrent transfers")
        return core
    }
    private func privateEngine(for request: EngineRequest) throws -> EngineBridge {
        lifecycle.lock(); defer { lifecycle.unlock() }
        guard !stopping else { throw HostFailure(code:"host.stopping") }
        if request.command != "connectPrivate", let engine { return engine }
        guard request.command == "connectPrivate", engine == nil,
              let protection = request.privateProtection, protection.utf8.count <= 8192,
              let bookmark = request.privateFolder, !bookmark.isEmpty, bookmark.count <= 65536 else {
            throw HostFailure(code:"host.unsupported")
        }
        var stale = false
        let folder = try URL(resolvingBookmarkData:bookmark,options:[.withoutUI,.withoutImplicitStartAccessing],relativeTo:nil,bookmarkDataIsStale:&stale)
        guard folder.isFileURL, !stale else { throw HostFailure(code:"host.folder_access") }
        let scoped = folder.startAccessingSecurityScopedResource()
        #if SEEDHOP_SANDBOX
        guard scoped else { throw HostFailure(code:"host.folder_access") }
        #endif
        do {
            let core = try EngineBridge.newPrivate(downloadDirectory:folder.path,protection:protection,discovery:true)
            if scoped { privateFolder = folder }
            engine = core
            activity = ProcessInfo.processInfo.beginActivity(options:[.userInitiatedAllowingIdleSystemSleep],reason:"Maintaining torrent transfers")
            return core
        } catch {
            if scoped { folder.stopAccessingSecurityScopedResource() }
            throw error
        }
    }
    func listener(_ listener: NSXPCListener, shouldAcceptNewConnection connection: NSXPCConnection) -> Bool {
        do { connection.setCodeSigningRequirement(try Signing.requirement(at: Signing.containingApp)) }
        catch { NSLog("SeedHop: rejected connection."); return false }
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
        // Torrent metadata is bounded to 32 MiB; allow its base64 envelope and options.
        guard data.count <= 48 * 1024 * 1024 else {
            reply((try? JSONEncoder().encode(EngineReply(errorMessage:EngineMessage(code:"input.invalid")))) ?? Data()); return
        }
        guard requestSlots.wait(timeout:.now()) == .success else {
            reply((try? JSONEncoder().encode(EngineReply(errorMessage:EngineMessage(code:"host.requests_busy")))) ?? Data()); return
        }
        queue.async {
            defer { self.requestSlots.signal() }
            let response: EngineReply
            do {
                let request = try JSONDecoder().decode(EngineRequest.self, from: data)
                guard request.version == SeedHopIdentity.protocolVersion else { throw HostFailure(code:"host.version_mismatch") }
                response = try self.handle(request)
            } catch BridgeError.Cancelled { response = EngineReply(cancelled:true) }
            catch let BridgeError.Operation(message, notice) { response = EngineReply(error:message,errorMessage:Self.message(notice)) }
            catch let failure as HostFailure { response = EngineReply(errorMessage:EngineMessage(code:failure.code)) }
            catch { response = EngineReply(errorMessage:EngineMessage(code:"engine.failed",diagnostic:error.localizedDescription)) }
            reply((try? JSONEncoder().encode(response)) ?? Data("{\"version\":\(SeedHopIdentity.protocolVersion),\"errorMessage\":{\"code\":\"engine.failed\",\"arguments\":[],\"diagnostic\":\"Could not encode engine response.\"}}".utf8))
        }
    }
    private func handle(_ r: EngineRequest) throws -> EngineReply {
        if privateSession {
            guard ["connectPrivate","snapshot","inspect","cancel","add","pause","recheck","retry","remove","shutdown"].contains(r.command),
                  r.deleteFiles != true, r.reuse != true else { throw HostFailure(code:"host.unsupported") }
        } else if r.command == "privateProtection" {
            lifecycle.lock(); defer { lifecycle.unlock() }
            guard !stopping else { throw HostFailure(code:"host.stopping") }
            var reply = EngineReply()
            reply.privateProtection = try engine?.privateProtection() ?? offlinePrivateProtection(stateDirectory:libraryDirectory.path)
            return reply
        } else if r.command == "connectPrivate" {
            throw HostFailure(code:"host.unsupported")
        }
        #if SEEDHOP_SANDBOX
        // Serialized with engine startup, including the background restore path.
        if !privateSession, let grants = r.folderGrants {
            lifecycle.lock(); defer { lifecycle.unlock() }
            do { try folderAccess.receive(grants) }
            catch { throw HostFailure(code:FolderAccessFailure.noticeCode(for:error)) }
        }
        if !privateSession, ["add", "relocate"].contains(r.command) {
            lifecycle.lock(); defer { lifecycle.unlock() }
            do {
                guard let destination = r.destination else { throw FolderAccessFailure.unavailable }
                try folderAccess.requireAccess(to: destination)
            } catch { throw HostFailure(code:FolderAccessFailure.noticeCode(for:error)) }
        }
        #endif
        #if SEEDHOP_VALIDATION
        // Observe restoration without the usual lazy start on an explicit Connect.
        if r.command == "validationStatus" {
            lifecycle.lock(); defer { lifecycle.unlock() }
            return EngineReply(snapshot:try engine.map { Self.snapshot(try $0.snapshot()) })
        }
        #endif
        #if SEEDHOP_DEVELOPMENT
        if r.command == "statisticsInfo" || r.command == "statisticsRetention" {
            lifecycle.lock(); defer { lifecycle.unlock() }
            guard !stopping else { throw HostFailure(code:"host.stopping") }
            let json: String
            if r.command == "statisticsRetention" {
                guard let remember = r.rememberStatistics else { throw HostFailure(code:"host.unsupported") }
                if let engine { json = try engine.setStatisticsRetention(remember: remember) }
                else { json = try saveStatisticsPreferences(stateDirectory: libraryDirectory.path, remember: remember) }
            } else {
                json = try engine?.statisticsPreferences() ?? statisticsPreferences(stateDirectory: libraryDirectory.path)
            }
            return EngineReply(statisticsRetention: try JSONDecoder().decode(StatisticsRetentionInfo.self, from: Data(json.utf8)))
        }
        if r.command == "clearActivity" {
            lifecycle.lock(); defer { lifecycle.unlock() }
            guard !stopping else { throw HostFailure(code:"host.stopping") }
            // Clearing local records must not lazily start a network session.
            try engine?.clearActivity()
            return EngineReply()
        }
        #endif
        if r.command == "shutdown" {
            lifecycle.lock(); stopping = true; let current = engine; lifecycle.unlock()
            if r.explicitStop == true, let current {
                for t in try current.snapshot().torrents where t.state != "paused" || t.forceStarted { try current.setPaused(id: t.id, paused: true) }
            }
            do { try current?.shutdown() }
            catch {
                if privateSession { DispatchQueue.global().asyncAfter(deadline:.now() + 0.2) { exit(1) } }
                throw error
            }
            lifecycle.lock(); defer { lifecycle.unlock() }; engine = nil
            runIntent.running = false
            // A preferences-only foreground helper never owned the engine's run
            // intent. Do not create a library or overwrite a background owner's intent.
            if !privateSession, current != nil || background { try persistIntent() }
            privateFolder?.stopAccessingSecurityScopedResource(); privateFolder = nil
            if let activity { ProcessInfo.processInfo.endActivity(activity); self.activity = nil }
            // Reply first; process exit is the final disk-ownership barrier.
            DispatchQueue.global().asyncAfter(deadline: .now() + 0.2) { exit(0) }
            return EngineReply(stoppedProcessID:ProcessInfo.processInfo.processIdentifier)
        }
        if r.command == "networkInfo" || r.command == "networkPolicy" {
            lifecycle.lock(); defer { lifecycle.unlock() }
            if r.command == "networkPolicy" {
                guard let policy = r.networkPolicy else { throw HostFailure(code:"host.unsupported") }
                let json = String(decoding:try JSONEncoder().encode(policy),as:UTF8.self)
                if let engine { try engine.setNetworkPolicy(policy:json) }
                else { try saveNetworkPreferences(stateDirectory:libraryDirectory.path,policy:json) }
            }
            let json = try engine?.networkInfo() ?? networkPreferences(stateDirectory:libraryDirectory.path)
            return EngineReply(networkInfo:try JSONDecoder().decode(NetworkInfo.self,from:Data(json.utf8)))
        }
        let core = try privateSession ? privateEngine(for:r) : start(destination:r.destination,stateDirectory:r.stateDirectory)
        switch r.command {
        case "connectPrivate":
            var reply = EngineReply(snapshot:try Self.snapshot(core.snapshot()))
            reply.privateProcessID = ProcessInfo.processInfo.processIdentifier
            return reply
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
        case "forceStart":
            guard let enabled = r.forceStarted else { throw HostFailure(code:"host.unsupported") }
            try core.setForceStarted(id:r.id ?? "", enabled:enabled)
        case "select": try core.selectFiles(id:r.id ?? "", indices:r.selected ?? [])
        case "reannounce":
            return EngineReply(reannounce:try JSONDecoder().decode(ReannounceSummary.self, from:Data(core.forceReannounce(id:r.id ?? "",targetIds:r.targetIds,instanceId:r.engineInstanceId ?? "").utf8)))
        case "recheck": try core.forceRecheck(id:r.id ?? "")
        case "retry": try core.retryTransfer(id:r.id ?? "")
        case "piecePreferences": try core.setPiecePreferences(id:r.id ?? "", sequential:r.sequential, firstLastPieces:r.firstLastPieces)
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
        lifecycle.lock()
        guard connections.isEmpty, privateSession || (!stopping && engine != nil) else { lifecycle.unlock(); return }
        if privateSession && stopping { lifecycle.unlock(); return }
        stopping = true
        lifecycle.unlock()
        do { _ = try handle(EngineRequest(command:"shutdown")) }
        catch {
            if privateSession { exit(1) }
            NSLog("SeedHop: shutdown failed.")
        }
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
                    destination:t.destination,files:t.files.map(file),trackers:t.trackers,canReannounce:t.canReannounce,error:t.error,queuePosition:t.queuePosition,pausedIntent:t.pausedIntent,forceStarted:t.forceStarted,errorKind:t.errorKind)
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
        snapshot.networkBlocked = s.networkBlocked
        snapshot.networkInterface = s.networkInterface
        snapshot.peerEncryption = PeerEncryptionPolicy(rawValue:s.peerEncryption)
        snapshot.savedPeerEncryption = PeerEncryptionPolicy(rawValue:s.savedPeerEncryption)
        return snapshot
    }
}

private struct HostFailure: Error { let code: String }
