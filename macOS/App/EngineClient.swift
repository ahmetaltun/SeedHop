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
        connection.interruptionHandler = { [weak self] in Task { @MainActor in self?.onInterruption?() } }
        connection.invalidationHandler = { [weak self] in Task { @MainActor in self?.onInterruption?() } }
        connection.resume()
        self.connection = connection
        self.background = background
    }
    func invalidate() {
        #if SEEDHOP_SANDBOX
        folderGeneration = nil
        #endif
        connection?.interruptionHandler = nil; connection?.invalidationHandler = nil
        connection?.invalidate(); connection = nil
    }
    func send(_ request: EngineRequest) async throws -> EngineReply {
        guard let connection else { throw UserFacingError(message:String(localized:"The engine is not connected yet.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) }
        #if SEEDHOP_SANDBOX
        var request = request
        let generation: UInt64?
        // Losing a folder grant must not prevent stopping the engine or changing
        // offline network settings. Those operations do not need payload access.
        if privateSession || ["shutdown", "cancel", "cancelFileOperation", "networkInfo", "networkPolicy", "clearActivity", "statisticsInfo", "statisticsRetention", "privateProtection"].contains(request.command) {
            generation = nil
        } else {
            do {
                generation = try FolderAccessStore.application.generation()
                if folderGeneration != generation { request.folderGrants = try FolderAccessStore.application.transferBookmarks() }
            } catch { throw EngineMessage(code:FolderAccessFailure.noticeCode(for:error)) }
        }
        #endif
        let data = try JSONEncoder().encode(request)
        let result: EngineReply = try await withCheckedThrowingContinuation { continuation in
            let box = ReplyOnce(continuation)
            let timeout: TimeInterval = request.command == "inspect" ? 130 : (request.command == "shutdown" ? 40 : 15)
            // Startup may verify retained copies before the engine can reply.
            // These operations still fail through the XPC error handler.
            if !["connect", "relocate", "rename", "retryStorageRecovery", "keepRecoveryCopies"].contains(request.command) {
                DispatchQueue.global().asyncAfter(deadline:.now() + timeout) { box.fail(UserFacingError(message:String(localized:"The engine took too long to respond. Your library has been preserved.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))) }
            }
            let proxy = connection.remoteObjectProxyWithErrorHandler { error in box.fail(error) } as? EngineServiceProtocol
            guard let proxy else { box.fail(UserFacingError(message:String(localized:"Could not reach the torrent engine.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))); return }
            proxy.send(data) { response in
                do {
                    let reply = try JSONDecoder().decode(EngineReply.self,from:response)
                    guard reply.version == SeedHopIdentity.protocolVersion else { throw UserFacingError(message:String(localized:"Restart SeedHop to finish updating its engine.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) }
                    if let message = reply.errorMessage { throw message }
                    if let error = reply.error { throw EngineMessage(code:"engine.failed",diagnostic:error) }
                    if reply.cancelled == true { throw CancellationError() }
                    box.succeed(reply)
                } catch { box.fail(error) }
            }
        }
        #if SEEDHOP_SANDBOX
        if let generation, self.connection === connection { folderGeneration = generation }
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
