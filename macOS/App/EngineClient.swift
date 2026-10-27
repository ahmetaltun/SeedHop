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
    private(set) var background = false
    var onInterruption: (() -> Void)?

    func connect(background: Bool) throws {
        invalidate()
        let connection = background ? NSXPCConnection(machServiceName:SeedHopIdentity.background) : NSXPCConnection(serviceName:SeedHopIdentity.foreground)
        let root = Bundle.main.bundleURL.appendingPathComponent("Contents")
        let helper = background ? root.appendingPathComponent("MacOS/SeedHopAgent") : root.appendingPathComponent("XPCServices/SeedHopEngine.xpc")
        connection.setCodeSigningRequirement(try Signing.requirement(at:helper))
        connection.remoteObjectInterface = NSXPCInterface(with:EngineServiceProtocol.self)
        connection.interruptionHandler = { [weak self] in Task { @MainActor in self?.onInterruption?() } }
        connection.invalidationHandler = { [weak self] in Task { @MainActor in self?.onInterruption?() } }
        connection.resume()
        self.connection = connection
        self.background = background
    }
    func invalidate() {
        connection?.interruptionHandler = nil; connection?.invalidationHandler = nil
        connection?.invalidate(); connection = nil
    }
    func send(_ request: EngineRequest) async throws -> EngineReply {
        guard let connection else { throw UserFacingError(message:"The engine is not connected yet.") }
        let data = try JSONEncoder().encode(request)
        return try await withCheckedThrowingContinuation { continuation in
            let box = ReplyOnce(continuation)
            let timeout: TimeInterval = request.command == "inspect" ? 130 : (request.command == "shutdown" ? 40 : 15)
            // Startup may verify retained copies before the engine can reply.
            // These operations still fail through the XPC error handler.
            if !["connect", "relocate", "rename", "retryStorageRecovery", "keepRecoveryCopies"].contains(request.command) {
                DispatchQueue.global().asyncAfter(deadline:.now() + timeout) { box.fail(UserFacingError(message:"The engine took too long to respond. Your library has been preserved.")) }
            }
            let proxy = connection.remoteObjectProxyWithErrorHandler { error in box.fail(error) } as? EngineServiceProtocol
            guard let proxy else { box.fail(UserFacingError(message:"Could not reach the torrent engine.")); return }
            proxy.send(data) { response in
                do {
                    let reply = try JSONDecoder().decode(EngineReply.self,from:response)
                    guard reply.version == SeedHopIdentity.protocolVersion else { throw UserFacingError(message:"Restart SeedHop to finish updating its engine.") }
                    if let error = reply.error { throw UserFacingError(message:error) }
                    if reply.cancelled == true { throw CancellationError() }
                    box.succeed(reply)
                } catch { box.fail(error) }
            }
        }
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
