import Foundation
import ServiceManagement

@MainActor protocol EngineTransport: AnyObject {
    var onInterruption: (() -> Void)? { get set }
    func connect(background: Bool) throws
    func invalidate()
    func send(_ request: EngineRequest) async throws -> EngineReply
}

@MainActor final class EngineClient: EngineTransport {
    private var connection: NSXPCConnection?
    private var connectionID: UUID?
    private let privateSession: Bool
    private(set) var background = false
    var onInterruption: (() -> Void)?
    #if SEEDHOP_SANDBOX
    private var folderGeneration: UInt64?
    #endif

    init(privateSession: Bool = false) { self.privateSession = privateSession }

    func connect(background: Bool) throws {
        invalidate()
        guard !privateSession || !background else { throw EngineMessage(code:"host.unsupported") }
        let service = privateSession ? SeedHopIdentity.privateEngine : SeedHopIdentity.foreground
        let connection = background ? NSXPCConnection(machServiceName:SeedHopIdentity.background) : NSXPCConnection(serviceName:service)
        let root = Bundle.main.bundleURL.appendingPathComponent("Contents")
        let helper = background ? root.appendingPathComponent("MacOS/SeedHopAgent") : root.appendingPathComponent(privateSession ? "XPCServices/SeedHopPrivateEngine.xpc" : "XPCServices/SeedHopEngine.xpc")
        connection.setCodeSigningRequirement(try Signing.requirement(at:helper))
        connection.remoteObjectInterface = NSXPCInterface(with:EngineServiceProtocol.self)
        let id = UUID()
        let interrupted: @Sendable () -> Void = { [weak self] in
            Task { @MainActor in
                guard let self, self.connectionID == id else { return }
                self.invalidate()
                self.onInterruption?()
            }
        }
        connection.interruptionHandler = interrupted
        connection.invalidationHandler = interrupted
        self.connection = connection
        connectionID = id
        self.background = background
        connection.resume()
    }
    func invalidate() {
        #if SEEDHOP_SANDBOX
        folderGeneration = nil
        #endif
        connectionID = nil
        connection?.interruptionHandler = nil; connection?.invalidationHandler = nil
        connection?.invalidate(); connection = nil
    }
    func send(_ request: EngineRequest) async throws -> EngineReply {
        guard let connection else { throw UserFacingError(message:String(localized:"The engine is not connected yet.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) }
        #if SEEDHOP_SANDBOX
        let acknowledgedGeneration = folderGeneration
        // Losing a folder grant must not prevent stopping the engine or changing
        // offline network settings. Those operations do not need payload access.
        let needsGrants = !privateSession && !["shutdown", "cancel", "cancelFileOperation", "networkInfo", "networkPolicy", "clearActivity", "statisticsInfo", "statisticsRetention", "privateProtection"].contains(request.command)
        #endif
        let encoded = try await Task.detached(priority:.userInitiated) {
            #if SEEDHOP_SANDBOX
            var outgoing = request
            var generation: UInt64?
            if needsGrants {
                do {
                    generation = try FolderAccessStore.application.generation()
                    if acknowledgedGeneration != generation { outgoing.folderGrants = try FolderAccessStore.application.transferBookmarks() }
                } catch { throw EngineMessage(code:FolderAccessFailure.noticeCode(for:error)) }
            }
            return (data:try JSONEncoder().encode(outgoing), generation:generation)
            #else
            return (data:try JSONEncoder().encode(request), generation:nil as UInt64?)
            #endif
        }.value
        try Task.checkCancellation()
        guard self.connection === connection else { throw CancellationError() }
        let bundle = AppLocalization.shared.bundle, locale = AppLocalization.shared.locale
        let result: EngineReply = try await withCheckedThrowingContinuation { continuation in
            let box = ReplyOnce(continuation)
            let timeout: TimeInterval = request.command == "inspect" ? 130 : (request.command == "shutdown" ? 40 : 15)
            // Startup may verify retained copies before the engine can reply.
            // These operations still fail through the XPC error handler.
            if !["connect", "relocate", "rename", "retryStorageRecovery", "keepRecoveryCopies"].contains(request.command) {
                DispatchQueue.global().asyncAfter(deadline:.now() + timeout) { box.fail(UserFacingError(message:String(localized:"The engine took too long to respond. Your library has been preserved.",bundle:bundle,locale:locale))) }
            }
            let proxy = connection.remoteObjectProxyWithErrorHandler { error in box.fail(error) } as? EngineServiceProtocol
            guard let proxy else { box.fail(UserFacingError(message:String(localized:"Could not reach the torrent engine.",bundle:bundle,locale:locale))); return }
            proxy.send(encoded.data) { response in
                do {
                    let reply = try JSONDecoder().decode(EngineReply.self,from:response)
                    guard reply.version == SeedHopIdentity.protocolVersion else { throw UserFacingError(message:String(localized:"Restart SeedHop to finish updating its engine.",bundle:bundle,locale:locale)) }
                    if let message = reply.errorMessage { throw message }
                    if let error = reply.error { throw EngineMessage(code:"engine.failed",diagnostic:error) }
                    if reply.cancelled == true { throw CancellationError() }
                    box.succeed(reply)
                } catch { box.fail(error) }
            }
        }
        try Task.checkCancellation()
        guard self.connection === connection else { throw CancellationError() }
        #if SEEDHOP_SANDBOX
        if let generation = encoded.generation { folderGeneration = generation }
        #endif
        return result
    }
}

private final class ReplyOnce: @unchecked Sendable {
    private let lock = NSLock()
    private var continuation: CheckedContinuation<EngineReply,Error>?
    init(_ continuation: CheckedContinuation<EngineReply,Error>) { self.continuation = continuation }
    func succeed(_ reply: EngineReply) { finish(.success(reply)) }
    func fail(_ error: Error) { finish(.failure(error)) }
    private func finish(_ result: Result<EngineReply,Error>) {
        lock.lock(); let pending = continuation; continuation = nil; lock.unlock()
        pending?.resume(with:result)
    }
}
