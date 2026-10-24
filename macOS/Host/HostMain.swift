import Foundation

@main enum HostMain {
    static func main() {
        let background = ProcessInfo.processInfo.arguments.contains("--agent")
        let host = EngineHost(background:background)
        let listener = background ? NSXPCListener(machServiceName:SeedHopIdentity.background) : NSXPCListener.service()
        listener.delegate = host
        listener.resume()
        RunLoop.current.run()
        withExtendedLifetime(host) {}
    }
}
