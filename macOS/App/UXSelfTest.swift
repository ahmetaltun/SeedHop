#if SEEDHOP_DEVELOPMENT
import Foundation

/// Deterministic presentation-model regressions; this does not certify native input or VoiceOver.
@MainActor enum UXSelfTest {
    static func run() async throws {
        var passed = 0
        func check(_ name: String, _ body: @MainActor () async throws -> Void) async throws {
            try await body(); passed += 1; print("PASS: \(name)")
        }

        try await check("New import clears old input and preserves an open draft") {
            let store = LibraryStore(designPreview:true)
            store.incomingFile = URL(fileURLWithPath:"/tmp/old.torrent"); store.incomingMagnet = "old"
            store.beginImport()
            try expect(store.incomingFile == nil && store.incomingMagnet.isEmpty && store.showImport,"New import reused an earlier source")
            store.beginImport(magnet:"replacement")
            try expect(store.incomingMagnet.isEmpty,"An open draft was overwritten")
            store.showImport = false; store.open([URL(string:"https://example.org/file")!])
            try expect(!store.showImport && store.error != nil,"An unsupported dropped URL opened an import")
        }

        try await check("Explicit filters clear hidden selection; snapshots preserve identity") {
            let (store, peer) = library()
            let first = store.torrents[0]
            store.selection = [first.id]; store.filter = .paused
            try expect(store.selection.isEmpty,"Filter left an invisible actionable selection")
            store.filter = .all; store.selection = [first.id]; store.search = "Blender"
            try expect(store.selection.isEmpty,"Search left an invisible actionable selection")
            store.search = ""; store.selection = [first.id]
            peer.torrents[0].downloadRate += 1_000
            await store.refresh()
            try expect(store.selection == [first.id],"A live rate update lost selection")
        }

        try await check("Newly added transfer is revealed outside the previous filter") {
            let (store, _) = library()
            store.filter = .paused; store.search = "does not match"
            let id = store.torrents[0].id; store.revealTransfer(id)
            try expect(store.filter == .all && store.search.isEmpty && store.selection == [id],"Added transfer remained hidden")
        }

        try await check("Pause captures its targets and suppresses duplicate in-flight commands") {
            let (store, peer) = library(); peer.hold = ["pause"]
            let first = store.torrents[0].id, other = store.torrents[1].id
            store.selection = [first]; store.toggleSelected(paused:true)
            try expect(store.busyTransfers == [first],"Action did not reserve its targets synchronously")
            store.selection = [other]
            try await eventually { peer.pending.count == 1 }
            try expect(peer.pending[0].request.id == first,"Pause followed a later selection")
            store.selection = [first]; store.toggleSelected(paused:true)
            store.selection = [other]
            peer.releaseFirst()
            try await eventually { store.busyTransfers.isEmpty }
            try expect(peer.commands("pause").count == 1 && store.selection == [other],"Duplicate command or unrelated selection loss")
        }

        try await check("Recheck captures targets, preserves choices, and allows retry after failure") {
            let (store, peer) = library(); peer.hold = ["recheck"]
            let first = store.torrents[0], other = store.torrents[1].id
            store.selection = [first.id]; store.recheckSelected(); store.recheckSelected()
            store.selection = [other]
            try await eventually { peer.pending.count == 1 }
            try expect(peer.pending[0].request.id == first.id,"Recheck followed a later selection")
            peer.releaseFirst(error:UserFacingError(message:"Reconnect the drive and retry."))
            try await eventually { store.busyTransfers.isEmpty }
            try expect(store.error != nil && store.selection == [other],"Recheck failure was hidden or changed selection")
            store.selection = [first.id]; store.recheckSelected()
            try await eventually { peer.pending.count == 1 }
            peer.releaseFirst()
            try await eventually { store.busyTransfers.isEmpty }
            try expect(store.torrents[0].files == first.files && store.torrents[0].state == first.state && store.error == nil,"Recheck changed choices or pause state")
            peer.torrents[0].state = "checking"; await store.refresh()
            store.recheckSelected()
            try expect(peer.commands("recheck").count == 2,"A verifying transfer accepted a duplicate recheck")
        }

        try await check("Removal confirmation retains original targets and keeps files by default") {
            let (store, peer) = library(); peer.hold = ["remove"]
            let first = store.torrents[0].id, other = store.torrents[1].id
            store.selection = [first]; store.requestRemoval(); store.selection = [other]
            store.removeConfirmed(deleteFiles:false)
            try await eventually { peer.pending.count == 1 }
            let command = peer.pending[0].request
            try expect(command.id == first && command.deleteFiles == false,"Confirmation acted on a changed selection or deleted files")
            store.showRemoval = false; store.requestRemoval()
            peer.releaseFirst()
            try await eventually { store.busyTransfers.isEmpty }
            try expect(store.selection == [other] && !store.torrents.contains { $0.id == first },"Removal lost an unrelated selection or failed to refresh")
            try expect(store.removalTargets.map(\.id) == [other],"A completed removal cleared a newer confirmation")
            store.removeConfirmed(deleteFiles:false)
            try await eventually { peer.pending.count == 1 }
            try expect(peer.pending[0].request.id == other,"The later confirmation lost its targets")
            peer.releaseFirst()
            try await eventually { store.busyTransfers.isEmpty }
        }

        try await check("File choices save once, surface errors, and remain retryable") {
            let (store, peer) = library(); peer.hold = ["select"]
            let item = store.torrents[0]
            store.setFileSelection(item,indices:[])
            try expect(peer.commands("select").isEmpty && store.busyTransfers.isEmpty,"An empty selection was submitted")
            store.setFileSelection(item,indices:[1,0]); store.setFileSelection(item,indices:[2,0])
            try await eventually { peer.pending.count == 1 }
            try expect(peer.pending[0].request.selected == [0,1],"File choices were not captured or a stale update overtook them")
            peer.releaseFirst(error:UserFacingError(message:"The destination is unavailable."))
            try await eventually { store.busyTransfers.isEmpty }
            try expect(store.error == "The destination is unavailable.","Save failure was hidden")
            store.setFileSelection(item,indices:[0,2])
            try await eventually { peer.pending.count == 1 }
            peer.releaseFirst()
            try await eventually { store.busyTransfers.isEmpty }
            try expect(store.torrents[0].files.filter(\.selected).map(\.index) == [0,2],"Retry did not persist the new choices")
            try expect(store.error == nil,"A successful retry retained the old failure notice")
        }

        try await check("Priorities preserve choices and serialize per transfer") {
            let (store, peer) = library(); peer.hold = ["priorities"]
            let item = store.torrents[0], file = store.torrents[0].files[0]
            store.setPriority(item,file:file,priority:2); store.setPriority(item,file:file,priority:0)
            try await eventually { peer.pending.count == 1 }
            try expect(peer.pending[0].request.priorities?.first == 2,"Priority choices were overtaken")
            peer.releaseFirst(error:UserFacingError(message:"Could not save priorities"))
            try await eventually { store.busyTransfers.isEmpty }
            try expect(store.torrents[0].files == item.files && store.error != nil,"Failed priority save changed file choices")
            store.setPriority(item,file:file,priority:2)
            try await eventually { peer.pending.count == 1 }; peer.releaseFirst()
            try await eventually { store.busyTransfers.isEmpty }
            try expect(store.torrents[0].files[0].priority == 2 && store.torrents[0].files.map(\.selected) == item.files.map(\.selected),"Priority changed selection")
        }
        try await check("Queue limits report failure, retry, and capture reorder targets") {
            let (store, peer) = library(); peer.hold = ["queue", "queueMove"]
            store.setQueueLimit(1); store.setQueueLimit(2)
            try await eventually { peer.pending.count == 1 }
            peer.releaseFirst(error:UserFacingError(message:"Could not save queue"))
            try await eventually { !store.savingQueue }
            try expect(store.maxActiveDownloads == 0 && store.error != nil,"Queue failure was hidden")
            store.setQueueLimit(2); try await eventually { peer.pending.count == 1 }; peer.releaseFirst()
            try await eventually { !store.savingQueue }
            try expect(store.maxActiveDownloads == 2,"Queue retry failed")
            let first = store.torrents[0].id, other = store.torrents[1].id
            store.selection = [first]; store.moveSelectedToFront(); store.selection = [other]
            try await eventually { peer.pending.count == 1 }
            try expect(peer.pending[0].request.id == first && peer.pending[0].request.queuePosition == 1,"Queue action followed changed selection")
            peer.releaseFirst(); try await eventually { store.busyTransfers.isEmpty }
            try expect(store.selection == [other],"Queue action lost another selection")
        }
        try await check("File move and rename failures preserve state and remain retryable") {
            let (store, peer) = library(); peer.hold = ["rename", "relocate"]
            let item = store.torrents[0], other = store.torrents[1].id
            var rename = EngineRequest(command:"rename"); rename.id = item.id; rename.fileIndex = item.files[0].index; rename.fileName = "renamed.bin"
            store.updateTransfer(rename); store.updateTransfer(rename); store.selection = [other]
            try await eventually { peer.pending.count == 1 }
            peer.releaseFirst(error:UserFacingError(message:"A file already exists"))
            try await eventually { store.busyTransfers.isEmpty }
            try expect(store.torrents[0].files == item.files && store.error != nil,"Rename failure altered paths")
            store.updateTransfer(rename); try await eventually { peer.pending.count == 1 }; peer.releaseFirst()
            try await eventually { store.busyTransfers.isEmpty }
            var move = EngineRequest(command:"relocate"); move.id = item.id; move.destination = "/Volumes/Test/Transfers"
            store.updateTransfer(move); try await eventually { peer.pending.count == 1 }; peer.releaseFirst()
            try await eventually { store.busyTransfers.isEmpty }
            try expect(store.selection == [other] && store.torrents[0].destination == move.destination && store.torrents[0].files[0].name == "renamed.bin","Storage action targeted changed selection or failed to refresh")
            try expect(store.torrents[0].state == item.state && store.torrents[0].files.map(\.selected) == item.files.map(\.selected),"Storage action changed pause intent or file choices")
        }

        try await check("Cancelled metadata cannot change a new attempt") {
            let peer = UXTransport(); peer.hold = ["inspect"]
            let session = ImportSession(); session.magnet = magnet
            let first = Task { await session.resolve(using:peer.send) }
            try await eventually { peer.pending.count == 1 }
            let firstID = peer.pending[0].request.requestId
            session.cancel(using:peer.send)
            let second = Task { await session.resolve(using:peer.send) }
            try await eventually { peer.pending.count == 2 && peer.commands("cancel").count == 1 }
            try expect(peer.pending[1].request.requestId != firstID && peer.commands("cancel")[0].id == firstID,"Cancellation targeted the wrong lookup")
            peer.releaseFirst(error:UserFacingError(message:"Old request failed")); await first.value
            try expect(session.resolving && session.error == nil && session.preview == nil,"An old error overwrote the active lookup")
            peer.releaseFirst(reply:EngineReply(preview:PreviewFixtures.importPreview)); await second.value
            try expect(!session.resolving && session.preview != nil && session.selected.count == 4,"New metadata was lost")
        }

        try await check("Cancelled successful metadata is discarded") {
            let peer = UXTransport(); peer.hold = ["inspect"]
            let session = ImportSession(); session.magnet = magnet
            let lookup = Task { await session.resolve(using:peer.send) }
            try await eventually { peer.pending.count == 1 }
            session.cancel(using:peer.send)
            peer.releaseFirst(reply:EngineReply(preview:PreviewFixtures.importPreview)); await lookup.value
            try expect(session.preview == nil && !session.resolving,"A dismissed lookup published its result")
        }

        try await check("Invalid links fail locally with recovery guidance") {
            let peer = UXTransport(); let session = ImportSession()
            session.magnet = "https://example.org/file"; await session.resolve(using:peer.send)
            try expect(peer.requests.isEmpty && session.error?.contains("magnet:") == true && !session.resolving,"Invalid link entered metadata resolution")
        }

        try await check("Missing metadata is an explicit retryable error") {
            let peer = UXTransport(); let session = ImportSession(); session.magnet = magnet
            await session.resolve(using:peer.send)
            try expect(session.preview == nil && session.error != nil && !session.resolving,"Missing metadata was accepted silently")
            peer.metadata = PreviewFixtures.importPreview; await session.resolve(using:peer.send)
            try expect(session.preview != nil && session.error == nil,"Retry retained an old error")
        }

        try await check("Add captures selected files and paused intent; duplicate adds are blocked") {
            let peer = UXTransport(); peer.hold = ["add"]
            let session = ImportSession(preview:PreviewFixtures.importPreview)
            session.selected = []; let empty = await session.add(using:peer.send)
            try expect(empty == nil && peer.requests.isEmpty,"Empty file selection was added")
            session.selected = [2,0]; session.paused = true; session.reuse = true; session.destination = "/tmp/chosen-folder"
            let add = Task { await session.add(using:peer.send) }
            try await eventually { peer.pending.count == 1 }
            let duplicate = await session.add(using:peer.send)
            session.cancel(using:peer.send)
            let request = peer.pending[0].request
            try expect(session.adding && duplicate == nil && request.selected == [0,2] && request.paused == true && request.reuse == true && request.destination == "/tmp/chosen-folder","Add choices changed or duplicate was accepted")
            try expect(peer.commands("cancel").isEmpty,"A committed add was treated as cancellable metadata")
            peer.releaseFirst(reply:EngineReply(id:"added"))
            let id = await add.value
            try expect(id == "added" && !session.adding,"Completed add remained busy")
        }

        try await check("Add failure preserves choices for recovery") {
            let peer = UXTransport(); peer.hold = ["add"]
            let session = ImportSession(preview:PreviewFixtures.importPreview); session.selected = [1]; session.paused = true
            let add = Task { await session.add(using:peer.send) }
            try await eventually { peer.pending.count == 1 }
            peer.releaseFirst(error:UserFacingError(message:"Choose a writable folder."))
            _ = await add.value
            try expect(!session.adding && session.error == "Choose a writable folder." && session.selected == [1] && session.paused,"Failed add lost the user's choices")
        }

        try await check("Torrent-file failures retain source and reject oversized metadata before XPC") {
            let directory = FileManager.default.temporaryDirectory.appendingPathComponent("seedhop-ux-"+UUID().uuidString)
            try FileManager.default.createDirectory(at:directory,withIntermediateDirectories:true)
            defer { try? FileManager.default.removeItem(at:directory) }
            let file = directory.appendingPathComponent("oversized.torrent")
            _ = FileManager.default.createFile(atPath:file.path,contents:Data())
            let handle = try FileHandle(forWritingTo:file); try handle.truncate(atOffset:33 * 1024 * 1024); try handle.close()
            let peer = UXTransport(); let session = ImportSession()
            await session.inspect(file:file,using:peer.send)
            try expect(session.error?.contains("32 MB") == true && session.sourceFile == file && peer.requests.isEmpty,"Oversized file was sent or its source was lost")
            let bytes = Data([1,2,3]); try bytes.write(to:file)
            peer.metadata = PreviewFixtures.importPreview; await session.inspect(file:file,using:peer.send)
            try expect(peer.commands("inspect").first?.bytes == bytes && session.preview != nil && session.error == nil,"File retry failed to send the current bytes")
        }

        print("PASS: \(passed) UX presentation regressions using a controlled transport. Native pointer, keyboard, and VoiceOver walkthroughs remain manual.")
    }

    private static let magnet = "magnet:?xt=urn:btih:"+String(repeating:"a",count:40)
    private static func library() -> (LibraryStore, UXTransport) {
        let peer = UXTransport(); peer.torrents = PreviewFixtures.torrents
        let store = LibraryStore(client:peer); store.torrents = peer.torrents; store.connected = true; store.completionNotifications = false
        return (store,peer)
    }
    private static func expect(_ condition: @autoclosure () -> Bool, _ message: String) throws {
        if !condition() { throw UserFacingError(message:message) }
    }
    private static func eventually(_ condition: () -> Bool) async throws {
        let deadline = Date(timeIntervalSinceNow:3)
        while !condition() {
            guard Date() < deadline else { throw UserFacingError(message:"Timed out waiting for a presentation action") }
            try await Task.sleep(for:.milliseconds(10))
        }
    }
}

@MainActor private final class UXTransport: EngineTransport {
    struct Pending { let request: EngineRequest; let continuation: CheckedContinuation<EngineReply, Error> }
    var onInterruption: (() -> Void)?
    var requests: [EngineRequest] = []
    var pending: [Pending] = []
    var hold = Set<String>()
    var torrents: [TorrentItem] = []
    var metadata: ImportPreview?
    var maxActiveDownloads: UInt32 = 0
    func connect(background: Bool) throws {}
    func invalidate() {}
    func commands(_ name: String) -> [EngineRequest] { requests.filter { $0.command == name } }
    func send(_ request: EngineRequest) async throws -> EngineReply {
        requests.append(request)
        if hold.contains(request.command) {
            return try await withCheckedThrowingContinuation { pending.append(Pending(request:request,continuation:$0)) }
        }
        return reply(to:request)
    }
    func releaseFirst(reply override: EngineReply? = nil, error: Error? = nil) {
        let item = pending.removeFirst()
        if let error { item.continuation.resume(throwing:error) }
        else { item.continuation.resume(returning:override ?? reply(to:item.request)) }
    }
    private func reply(to request: EngineRequest) -> EngineReply {
        switch request.command {
        case "snapshot":
            return EngineReply(snapshot:LibrarySnapshot(protocolVersion:UInt32(SeedHopIdentity.protocolVersion),engineVersion:"test",instanceId:"fixture",listenPort:0,downloadLimit:0,uploadLimit:0,maxActiveDownloads:maxActiveDownloads,torrents:torrents))
        case "inspect": return EngineReply(preview:metadata)
        case "remove": torrents.removeAll { $0.id == request.id }
        case "select":
            if let at = torrents.firstIndex(where: { $0.id == request.id }) {
                for index in torrents[at].files.indices { torrents[at].files[index].selected = request.selected?.contains(torrents[at].files[index].index) == true }
            }
        case "priorities":
            if let at = torrents.firstIndex(where: { $0.id == request.id }), let priorities = request.priorities {
                for index in torrents[at].files.indices { torrents[at].files[index].priority = priorities[index] }
            }
        case "queue": maxActiveDownloads = request.maxActiveDownloads ?? 0
        case "queueMove":
            if let at = torrents.firstIndex(where: { $0.id == request.id }) { torrents[at].queuePosition = request.queuePosition ?? 1 }
        case "relocate":
            if let at = torrents.firstIndex(where: { $0.id == request.id }), let destination = request.destination { torrents[at].destination = destination }
        case "rename":
            if let at = torrents.firstIndex(where: { $0.id == request.id }), let file = torrents[at].files.firstIndex(where: { $0.index == request.fileIndex }), let name = request.fileName { torrents[at].files[file].name = name }
        case "pause":
            if let at = torrents.firstIndex(where: { $0.id == request.id }) { torrents[at].state = request.paused == true ? "paused" : "downloading" }
        default: break
        }
        return EngineReply()
    }
}
#endif
