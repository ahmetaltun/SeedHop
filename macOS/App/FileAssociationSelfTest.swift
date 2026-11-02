#if SEEDHOP_DEVELOPMENT
import Foundation

/// Default-handler coverage uses an in-memory system; it never changes Launch Services.
@MainActor enum FileAssociationSelfTest {
    static func run() async throws {
        let suite = "app.seedhop.association-tests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName:suite)!
        defer { defaults.removePersistentDomain(forName:suite) }
        let system = AssociationSystemFixture()
        let service = FileAssociationService(system:system,defaults:defaults)
        func expect(_ condition: Bool, _ message: String) throws {
            if !condition { throw UserFacingError(message:message) }
        }
        try expect(!service.isDefault(.torrent) && !service.isDefault(.magnet),"Association opt-in state was wrong")
        await service.change(.torrent,enabled:true)
        try expect(service.isDefault(.torrent) && !service.isDefault(.magnet),"Torrent change affected magnet default")
        await service.change(.torrent,enabled:false)
        try expect(system.current(.torrent) == system.other,"Previous torrent handler was not restored")
        await service.change(.magnet,enabled:true)
        try expect(service.isDefault(.magnet) && !service.isDefault(.torrent),"Magnet change affected torrent default")
        await service.change(.magnet,enabled:false)
        try expect(system.current(.magnet) == system.other,"Previous magnet handler was not restored")

        system.failure = true
        await service.change(.torrent,enabled:true)
        try expect(!service.isDefault(.torrent) && service.errors[.torrent] != nil && service.busy.isEmpty,"Failed change produced a false enabled state")
        system.failure = false
        system.ignoreChange = true
        await service.change(.torrent,enabled:true)
        try expect(!service.isDefault(.torrent) && service.errors[.torrent] != nil,"Unapplied change was not reported")
        system.ignoreChange = false

        await service.change(.torrent,enabled:true)
        system.available = [system.seedHop,system.replacement]
        await service.change(.torrent,enabled:false)
        try expect(service.replacement?.applications == [system.replacement] && service.isDefault(.torrent),"Missing previous handler did not offer compatible replacements")
        let calls = system.setCalls
        service.cancelReplacement()
        try expect(service.isDefault(.torrent) && system.setCalls == calls,"Cancelling replacement changed the default")
        await service.change(.torrent,enabled:false)
        await service.chooseReplacement(system.replacement)
        try expect(system.current(.torrent) == system.replacement && !service.isDefault(.torrent),"Selected replacement was not applied")

        // Resolve a moved previous installation by its persisted identity.
        await service.change(.torrent,enabled:true)
        let moved = AssociationApplication(url:URL(fileURLWithPath:"/Applications/Moved.app"),bundleIdentifier:system.replacement.bundleIdentifier,name:"Moved")
        system.available = [system.seedHop,moved]
        await service.change(.torrent,enabled:false)
        try expect(system.current(.torrent) == moved,"Moved handler could not be restored")

        system.handlers[.torrent] = system.seedHop
        system.available = [system.seedHop]
        service.refresh()
        await service.change(.torrent,enabled:false)
        try expect(service.isDefault(.torrent) && service.replacement == nil && service.errors[.torrent] != nil,"No replacement silently disabled the association")
        system.handlers[.torrent] = system.other
        service.refresh()
        try expect(!service.isDefault(.torrent),"External default change was not reflected")
        system.handlers[.torrent] = system.seedHop
        system.available = [system.seedHop,system.other]
        defaults.removeObject(forKey:"previousDefaultApplication.torrent")
        service.refresh()
        await service.change(.torrent,enabled:false)
        try expect(service.replacement != nil,"Existing default without history did not ask for replacement")
        system.handlers[.torrent] = system.other
        await service.chooseReplacement(system.other)
        try expect(system.current(.torrent) == system.other,"Chooser overwrote an external default change")
        print("PASS: file association enable, restore, replacement, refusal, cancellation, moved app, and external change")
    }
}

@MainActor private final class AssociationSystemFixture: FileAssociationSystem {
    let seedHop = AssociationApplication(url:URL(fileURLWithPath:"/Applications/SeedHop.app"),bundleIdentifier:"app.seedhop.mac",name:"SeedHop")
    let other = AssociationApplication(url:URL(fileURLWithPath:"/Applications/Other.app"),bundleIdentifier:"example.other",name:"Other")
    let replacement = AssociationApplication(url:URL(fileURLWithPath:"/Applications/Replacement.app"),bundleIdentifier:"example.replacement",name:"Replacement")
    var handlers: [FileAssociationKind:AssociationApplication] = [:]
    var available: [AssociationApplication] = []
    var failure = false
    var ignoreChange = false
    var setCalls = 0
    init() { handlers = [.torrent:other,.magnet:other]; available = [seedHop,other,replacement] }
    func current(_ kind: FileAssociationKind) -> AssociationApplication? { handlers[kind] }
    func applications(_ kind: FileAssociationKind) -> [AssociationApplication] { available }
    func installedApplication(_ saved: AssociationApplication, for kind: FileAssociationKind) -> AssociationApplication? {
        available.first { $0.url == saved.url && $0.bundleIdentifier == saved.bundleIdentifier } ?? available.first { $0.bundleIdentifier != nil && $0.bundleIdentifier == saved.bundleIdentifier }
    }
    func setDefault(_ application: AssociationApplication, for kind: FileAssociationKind) async throws {
        setCalls += 1
        if failure { throw UserFacingError(message:"Controlled association refusal") }
        if !ignoreChange { handlers[kind] = application }
    }
}
#endif
