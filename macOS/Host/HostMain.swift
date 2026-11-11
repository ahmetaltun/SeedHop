import Foundation

@main enum HostMain {
    static func main() {
        let privateSession = Bundle.main.bundleIdentifier == SeedHopIdentity.privateEngine
        let background = !privateSession && ProcessInfo.processInfo.arguments.contains("--agent")
        let host = EngineHost(background:background,privateSession:privateSession)
        let listener = background ? NSXPCListener(machServiceName:SeedHopIdentity.background) : NSXPCListener.service()
        listener.delegate = host
        listener.resume()
        RunLoop.current.run()
        withExtendedLifetime(host) {}
    }
}
