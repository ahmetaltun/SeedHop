#if SEEDHOP_DEVELOPMENT
import Foundation
import AppKit
import Combine

/// Deterministic presentation-model regressions; this does not certify native input or VoiceOver.
@MainActor enum UXSelfTest {
    static func run() async throws {
        var passed = 0
        func check(_ name: String, _ body: @MainActor () async throws -> Void) async throws {
            try await body(); passed += 1; print("PASS: \(name)")
        }

        try await check("Scheduling validates drafts and preserves saved state through rejected and delayed saves") {
            try expect(ScheduleDraft.minute("23:59") == 1439 && ScheduleDraft.minute("24:00") == nil && ScheduleDraft.minute("9:00") == nil,"Schedule times accepted invalid input")
            var draft = ScheduleDraft(ScheduleRule()); draft.down = "4294968"
            var rejected = false
            do { _ = try draft.rule() } catch { rejected = true }
            try expect(rejected,"Schedule bandwidth overflow was accepted")
            draft.suspend = true
            let suspendedRule = try draft.rule()
            try expect(suspendedRule.suspend,"Suspension unexpectedly requires bandwidth values")
            let (store, peer) = library(); await store.refresh()
            let original = store.schedule
            let value = ScheduleConfig(enabled:true,rules:[ScheduleRule()])
            peer.hold = ["schedule"]
            let save = Task { try await store.saveSchedule(value) }
            try await eventually { peer.pending.count == 1 }
            try expect(store.savingSchedule && store.schedule == original,"Schedule save changed state optimistically")
            peer.releaseFirst(error:UserFacingError(message:"Read-only library"))
            do { try await save.value } catch { }
            try expect(!store.savingSchedule && store.schedule == original,"Failed schedule save changed saved state")
            let retry = Task { try await store.saveSchedule(value) }
            try await eventually { peer.pending.count == 1 }; peer.releaseFirst()
            try await retry.value
            try expect(!store.savingSchedule && store.schedule == value,"Schedule retry did not save")
            store.torrents[0].state = "scheduled"
            try expect(store.presentation(store.torrents[0]).detail.contains("weekly schedule"),"Scheduled suspension is presented as queue starvation")
            store.torrents[0].downloadedBytes = store.torrents[0].totalBytes
            try expect(!store.torrents[0].isActive,"A suspended completed seeder was counted as an active download")
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
        try await check("File outline groups paths, filters ancestors, and retains stable identities") {
            var files = PreviewFixtures.files
            files.append(FileItem(index:4,name:"Subs/Italian/SDH.ita.HI.srt",size:98_000,downloaded:0,selected:false))
            files.append(FileItem(index:5,name:"Subs/日本語.srt",size:52_000,downloaded:0,selected:true))
            let session = TorrentFilesSession(files:files)
            try expect(session.tree.nodes["folder:Subs"]?.indices == [4,5],"Folder did not aggregate descendants")
            try expect(session.tree.matchingKeys("sdh") == ["file:4","folder:Subs","folder:Subs/Italian"],"Search lost ancestors or included unrelated files")
            session.selection = ["folder:Subs","file:0"]; session.expanded = ["folder:Subs"]
            session.query = "sdh"
            try expect(session.selection == ["folder:Subs"] && session.tree.targets(session.selection,query:session.query).isEmpty,"Filtered folder could mutate hidden descendants")
            session.query = ""; session.selection = ["file:4"]
            let original = session.tree
            files[4].downloaded = 100; session.update(files)
            try expect(session.tree === original && session.selection == ["file:4"],"Progress rebuilt the hierarchy or lost selection")
            files[4].name = "Subs/Italian/Renamed.srt"; session.update(files)
            try expect(session.tree !== original && session.selection == ["file:4"] && session.expanded == ["folder:Subs"],"Rename lost file identity or unrelated expansion")
            try expect(session.tree.targets(["folder:Subs","file:4"],query:"") == [4,5],"Overlapping folder/file selection duplicated targets")
        }
        try await check("Batch file commands use the current snapshot and protect the last included file") {
            let (store, peer) = library(); peer.hold = ["priorities","select"]
            let item = store.torrents[0]
            peer.torrents[0].files[1].priority = 0; peer.torrents[0].files[3].selected = false
            await store.refresh()
            store.setPriorities(torrentID:item.id,indices:[0,3],priority:2)
            try await eventually { peer.pending.count == 1 }
            try expect(peer.pending[0].request.priorities == [2,0,1,2],"Batch priority overwrote an untargeted current priority")
            store.selection = [store.torrents[1].id]
            peer.releaseFirst(); try await eventually { store.busyTransfers.isEmpty }
            try expect(!store.torrents[0].files[3].selected,"Priority included a skipped file")
            store.setFilesIncluded(torrentID:item.id,indices:[0,1,2],included:false)
            try expect(peer.pending.isEmpty && store.error == "Keep at least one file included.","Batch skip removed all included files")
            store.setFilesIncluded(torrentID:item.id,indices:[1,2],included:false)
            try await eventually { peer.pending.count == 1 }
            try expect(peer.pending[0].request.id == item.id && peer.pending[0].request.selected == [0],"Batch inclusion used a changed torrent selection")
            peer.releaseFirst(error:UserFacingError(message:"Could not save file choices"))
            try await eventually { store.busyTransfers.isEmpty }
            try expect(store.torrents[0].files.filter(\.selected).count == 3,"Failed batch changed confirmed inclusion")
            store.setFilesIncluded(torrentID:item.id,indices:[1,2],included:false)
            try await eventually { peer.pending.count == 1 }; peer.releaseFirst()
            try await eventually { store.busyTransfers.isEmpty }
            try expect(store.torrents[0].files.filter(\.selected).map(\.index) == [0],"Batch inclusion retry did not persist")
            peer.torrents[0].state = "checking"; await store.refresh()
            store.setPriorities(torrentID:item.id,indices:[0],priority:0)
            store.setFilesIncluded(torrentID:item.id,indices:[1],included:true)
            try expect(peer.pending.isEmpty,"Checking allowed a file mutation")
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

        try await check("File progress and cancellation retain exact targets across selection changes and retries") {
            let (store, peer) = library(); peer.hold = ["relocate", "cancelFileOperation"]
            let item = store.torrents[0], other = store.torrents[1].id
            var move = EngineRequest(command:"relocate"); move.id = item.id; move.destination = "/Volumes/Test/Transfers"
            store.updateTransfer(move)
            store.selection = [other]; store.showInspector = false
            try await eventually { peer.pending.count == 1 }
            let operation = FileOperationItem(id:UUID().uuidString,torrentId:item.id,kind:"relocate",phase:"copying",completedBytes:1024,totalBytes:4096,cancelRequested:false)
            peer.fileOperation = operation; await store.refresh()
            try expect(store.fileOperation == operation && !store.canChangeTransfers,"Progress was lost or conflicting actions remained available")
            store.toggleSelected(paused:true)
            try expect(peer.commands("pause").isEmpty,"A conflicting action was sent during a copy")
            var inspectionBlocked = false
            do { _ = try await store.perform(EngineRequest(command:"inspect")) }
            catch { inspectionBlocked = true }
            try expect(inspectionBlocked && peer.commands("inspect").isEmpty,"Import inspection waited behind an active copy")
            store.cancelFileOperation(operation); store.cancelFileOperation(operation)
            try await eventually { peer.pending.count == 2 }
            try expect(peer.pending[1].request.id == operation.id && peer.commands("cancelFileOperation").count == 1,"Cancellation targeted a torrent/selection or duplicated a request")
            peer.releaseFirst(at:1)
            try await eventually { store.cancellingOperations.isEmpty }
            try expect(store.fileOperation?.cancelRequested == true && store.fileOperation?.title == "Cancelling…","Cancellation acknowledgement was not reflected")
            peer.fileOperation = nil; peer.releaseFirst(error:CancellationError())
            try await eventually { store.busyTransfers.isEmpty }
            try expect(store.error == nil && store.selection == [other] && store.torrents[0].destination == item.destination && store.torrents[0].files == item.files,"Cancellation changed choices, selection, or surfaced as an error")
            store.updateTransfer(move)
            try await eventually { peer.pending.count == 1 }
            var retry = operation; retry.id = UUID().uuidString
            peer.fileOperation = retry; await store.refresh()
            store.cancelFileOperation(operation)
            try expect(peer.commands("cancelFileOperation").count == 1,"An old progress card cancelled a retry")
            peer.releaseFirst(); try await eventually { store.busyTransfers.isEmpty }
            try expect(store.fileOperation == nil && store.canChangeTransfers && store.selection == [other],"Finished operation left stale controls or changed selection")
        }

        try await check("Saving cannot be cancelled and cancellation failures remain retryable") {
            let (store, peer) = library(); peer.hold = ["cancelFileOperation"]
            let operation = FileOperationItem(id:UUID().uuidString,torrentId:store.torrents[0].id,kind:"rename",phase:"copying",completedBytes:1024,totalBytes:4096,cancelRequested:false)
            peer.fileOperation = operation; await store.refresh()
            store.cancelFileOperation(operation); try await eventually { peer.pending.count == 1 }
            peer.releaseFirst(error:UserFacingError(message:"Cancellation could not reach the engine."))
            try await eventually { store.cancellingOperations.isEmpty }
            try expect(store.error != nil && store.canCancelFileOperation(operation),"Failed cancellation could not be retried")
            peer.fileOperation?.phase = "saving"; await store.refresh()
            store.cancelFileOperation(operation)
            try expect(peer.commands("cancelFileOperation").count == 1 && store.fileOperation?.canCancel == false,"Stale copying state allowed cancellation during saving")
            try expect(store.fileOperation?.detail.contains("verification") == true,"Copy completion was presented as verification completion")
            peer.fileOperation = nil; await store.refresh()
            try expect(store.canChangeTransfers,"A completed external operation left controls disabled")
        }

        try await check("Recovery actions capture their record, preserve selection, and retry failures") {
            let (store, peer) = library(); peer.hold = ["retryStorageRecovery", "keepRecoveryCopies"]
            let item = store.torrents[0], other = store.torrents[1].id
            let notice = StorageRecoveryItem(id:UUID().uuidString,torrentId:item.id,activeLocation:"original",message:"Original location is active. Copies were retained.",originalDirectory:item.destination,destinationDirectory:"/Volumes/Example",canKeepCopies:true)
            peer.storageRecovery = notice; await store.refresh()
            peer.torrents[0].state = "checking"; await store.refresh()
            store.resolveStorageRecovery(notice,keepCopies:false)
            try expect(peer.commands("retryStorageRecovery").isEmpty,"Recovery ran before file verification finished")
            peer.torrents[0] = item; await store.refresh()
            store.resolveStorageRecovery(notice,keepCopies:false); store.resolveStorageRecovery(notice,keepCopies:false)
            store.selection = [other]; store.showInspector = false
            try await eventually { peer.pending.count == 1 }
            try expect(peer.pending[0].request.id == notice.id && store.resolvingStorageRecovery && !store.canChangeTransfers,"Recovery retargeted selection, duplicated a command, or left mutations enabled")
            peer.releaseFirst(error:UserFacingError(message:"Reconnect the destination and retry."))
            try await eventually { !store.resolvingStorageRecovery }
            try expect(store.storageRecovery == notice && store.error != nil && store.selection == [other],"Recovery failure dismissed its notice or changed selection")
            store.resolveStorageRecovery(notice,keepCopies:true); try await eventually { peer.pending.count == 1 }
            try expect(peer.pending[0].request.command == "keepRecoveryCopies" && peer.pending[0].request.id == notice.id,"Keep Copies did not capture the recovery record")
            peer.releaseFirst(); try await eventually { !store.resolvingStorageRecovery }
            try expect(store.storageRecovery == nil && store.torrents[0] == item && store.selection == [other] && store.error == nil,"Keeping copies changed transfer data or retained an old error")
            var newer = notice; newer.id = UUID().uuidString; peer.storageRecovery = newer; await store.refresh()
            store.resolveStorageRecovery(notice,keepCopies:true)
            try expect(peer.commands("keepRecoveryCopies").count == 1,"A stale notice dismissed a later recovery record")
        }

        try await check("Uncertain recovery stays visible until a snapshot confirms resolution") {
            let (store, peer) = library(); peer.hold = ["retryStorageRecovery"]
            let notice = StorageRecoveryItem(id:"unreadable",torrentId:"",activeLocation:"unknown",message:"Recovery record needs inspection.",originalDirectory:"/Example",destinationDirectory:"/Example",canKeepCopies:false)
            peer.storageRecovery = notice; await store.refresh()
            store.resolveStorageRecovery(notice,keepCopies:true)
            try expect(peer.commands("keepRecoveryCopies").isEmpty,"Uncertain cleanup could discard its record")
            store.resolveStorageRecovery(notice,keepCopies:false); try await eventually { peer.pending.count == 1 }
            peer.releaseFirst(reply:EngineReply()); try await eventually { !store.resolvingStorageRecovery }
            try expect(store.storageRecovery == notice,"Acknowledgement cleared an unresolved recovery notice")
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

        try await check("Transfer labels distinguish peers, queue positions, checks, and unknown progress") {
            var item = PreviewFixtures.torrents[0]
            item.state = "waiting"; item.peers = 0
            try expect(TransferPresentation(torrent:item).label == "Finding peers","No-peer state was inaccurate")
            item.peers = 3
            try expect(TransferPresentation(torrent:item).label == "Waiting for data","Connected idle peers appeared disconnected")
            item.state = "queued"; item.queuePosition = 4
            try expect(TransferPresentation(torrent:item).accessibleLabel == "Queued, position 4","Queue position was omitted")
            item.state = "checking"; item.pausedIntent = true
            let check = TransferPresentation(torrent:item)
            try expect(check.progressTitle == "Checked" && check.detail.contains("remain paused"),"Checking implied downloaded bytes or lost intent")
            item.state = "error"; item.errorKind = "storage_space"
            let failure = TransferPresentation(torrent:item)
            try expect(failure.progress == nil && failure.canRetry && failure.errorTitle == "Not enough disk space","Failure showed false progress or lacked recovery")
            item.errorKind = "storage_missing"
            try expect(!TransferPresentation(torrent:item).canMoveAfterError,"Unavailable original files offered relocation")
            let encoded = try JSONEncoder().encode(item)
            var old = try JSONSerialization.jsonObject(with:encoded) as! [String:Any]; old.removeValue(forKey:"errorKind")
            let decoded = try JSONDecoder().decode(TorrentItem.self,from:JSONSerialization.data(withJSONObject:old))
            try expect(decoded.errorKind == nil,"Older snapshots require the new optional error kind")
        }

        try await check("File operations and cleanup override stale transfer states and attention filters") {
            let (store, _) = library(); let id = store.torrents[0].id
            store.torrents[0].state = "error"
            store.fileOperation = FileOperationItem(id:"operation",torrentId:id,kind:"relocate",phase:"copying",completedBytes:10,totalBytes:20,cancelRequested:false)
            let moving = store.presentation(store.torrents[0])
            try expect(moving.label == "Moving files" && moving.progress == 0.5 && moving.progressTitle == "Copied" && !moving.needsAttention,"A move showed stale error or download progress")
            store.fileOperation = nil; store.torrents[0].state = "paused"
            store.storageRecovery = StorageRecoveryItem(id:"cleanup",torrentId:id,activeLocation:"original",message:"Review retained files",originalDirectory:"/tmp/original",destinationDirectory:"/tmp/copy",canKeepCopies:true)
            store.filter = .attention
            try expect(store.visibleTorrents.map(\.id) == [id] && store.count(in:.attention) == 1,"Cleanup was absent from attention")
            store.selection = [id]
            try expect(!store.canRemoveSelection && !store.canRecheckSelection && !store.canMoveSelection,"Cleanup allowed destructive file changes")
            store.torrents = []
            try expect(store.hasUnscopedRecovery && store.activitySummary.contains("attention"),"Orphan cleanup looked healthy")
        }

        try await check("Retry captures failed targets, suppresses duplicates, and preserves paused choices") {
            let (store, peer) = library(); peer.hold = ["retry"]
            peer.torrents[0].state = "error"; peer.torrents[0].pausedIntent = true
            peer.torrents[0].files[0].priority = 2
            await store.refresh(); let item = store.torrents[0], other = store.torrents[1].id
            store.selection = [item.id]; store.toggleSelected(paused:false); store.retryTransfer(item)
            store.selection = [other]
            try await eventually { peer.pending.count == 1 }
            try expect(peer.pending[0].request.command == "retry" && peer.pending[0].request.id == item.id,"Retry changed target or used unpause")
            peer.releaseFirst(error:UserFacingError(message:"Restore file access."))
            try await eventually { store.busyTransfers.isEmpty }
            try expect(store.torrents[0].state == "error" && store.error != nil,"Retry failure changed transfer state")
            store.retryTransfer(item)
            try await eventually { peer.pending.count == 1 }; peer.releaseFirst()
            try await eventually { store.busyTransfers.isEmpty }
            try expect(store.torrents[0].isPaused && store.torrents[0].files == item.files && store.selection == [other],"Retry lost pause intent, choices, or unrelated selection")
            store.retryTransfer(item)
            try expect(peer.commands("retry").count == 2,"Stale Retry reached a recovered transfer")
        }

        try await check("Mixed Resume and Retry uses separate commands and blocks checks or cleanup") {
            let (store, peer) = library()
            peer.torrents[0].state = "error"; peer.torrents[0].pausedIntent = false
            peer.torrents[1].state = "paused"; peer.torrents[1].pausedIntent = true
            await store.refresh(); store.selection = Set(store.torrents.prefix(2).map(\.id))
            try expect(store.resumeActionLabel == "Resume and Retry","Mixed action label hid Retry")
            store.toggleSelected(paused:false)
            try await eventually { store.busyTransfers.isEmpty }
            try expect(peer.commands("retry").count == 1 && peer.commands("pause").count == 1,"Mixed selection used incorrect commands")
            peer.torrents[0].state = "checking"; await store.refresh(); store.selection = [store.torrents[0].id]
            try expect(!store.canResumeSelection && !store.canRecheckSelection && !store.canMoveSelection,"Checking transfer accepted conflicting operations")
            peer.torrents[0].state = "error"
            peer.storageRecovery = StorageRecoveryItem(id:"unknown",torrentId:"",activeLocation:"original",message:"Review files",originalDirectory:"/tmp",destinationDirectory:"/tmp/copy",canKeepCopies:false)
            await store.refresh()
            try expect(!store.canRetryTransfer(store.torrents[0]) && !store.canRemoveSelection,"Unscoped cleanup did not protect transfer files")
        }

        try await check("Disconnected snapshots show stale status and hide live rates") {
            let (store, _) = library(); store.connected = false
            try expect(store.downloadRate == 0 && store.uploadRate == 0 && store.presentation(store.torrents[0]).label.hasPrefix("Last:"),"Offline snapshot implied live activity")
            try expect(store.activitySummary.contains("last update"),"Offline summary implied current activity")
        }

        try await check("Show Folder finds existing parents without creating a missing destination") {
            let directory = FileManager.default.temporaryDirectory.appendingPathComponent("status-"+UUID().uuidString)
            try FileManager.default.createDirectory(at:directory,withIntermediateDirectories:true)
            defer { try? FileManager.default.removeItem(at:directory) }
            let missing = directory.appendingPathComponent("unavailable/nested")
            try expect(LibraryStore.availableRevealDirectory(missing.path).standardizedFileURL == directory.standardizedFileURL,"Missing destination did not reveal its nearest existing parent")
            try expect(!FileManager.default.fileExists(atPath:missing.path),"Show Folder created a missing destination")
        }

        try await check("Encryption saves capture policy, report failure, and retain active connections until restart") {
            let (store, peer) = library(); await store.refresh(); peer.hold = ["peerEncryption"]
            store.saveEncryption(.require); store.saveEncryption(.disabled)
            try expect(store.savingEncryption && store.savedPeerEncryption == .allow,"Policy save was optimistic or not reserved")
            try await eventually { peer.pending.count == 1 }
            try expect(peer.pending[0].request.peerEncryption == .require,"Policy choice was overtaken")
            peer.releaseFirst(error:UserFacingError(message:"Could not save preference."))
            try await eventually { !store.savingEncryption }
            try expect(store.error != nil && store.savedPeerEncryption == .allow && !store.encryptionRestartPending,"Rejected policy save changed the saved choice")
            store.saveEncryption(.prefer); try await eventually { peer.pending.count == 1 }; peer.releaseFirst()
            try await eventually { !store.savingEncryption }
            try expect(store.peerEncryption == .allow && store.savedPeerEncryption == .prefer && store.encryptionRestartPending,"Saved policy pretended to apply live")
            store.saveEncryption(.allow); try await eventually { peer.pending.count == 1 }; peer.releaseFirst()
            try await eventually { !store.savingEncryption }
            try expect(!store.encryptionRestartPending && peer.commands("shutdown").isEmpty,"Restoring the active choice restarted transfers")
        }

        try await check("Policy restart preserves selection, choices, background mode, and login intent") {
            let (store, peer) = library(); await store.refresh()
            store.backgroundEnabled = true; store.resumeAtLogin = true
            store.selection = [store.torrents[0].id]; let before = store.torrents
            store.saveEncryption(.require); try await eventually { !store.savingEncryption }
            store.showImport = true; store.restartForEncryption()
            try expect(peer.commands("shutdown").isEmpty,"Restart interrupted an import")
            store.showImport = false
            store.fileOperation = FileOperationItem(id:"move",torrentId:before[0].id,kind:"relocate",phase:"copying",completedBytes:1,totalBytes:2,cancelRequested:false)
            try expect(!store.canSaveEncryption && !store.canRestartForEncryption,"Policy changes interrupted a file operation")
            store.fileOperation = nil; peer.hold = ["shutdown"]
            store.restartForEncryption(); store.restartForEncryption()
            try await eventually { peer.pending.count == 1 }
            try expect(store.switching && !store.canChangeTransfers && peer.pending[0].request.explicitStop == nil,"Restart changed pause intent or allowed conflicting actions")
            peer.releaseFirst(); try await eventually { !store.switching }
            try expect(store.connected && store.peerEncryption == .require && !store.encryptionRestartPending,"Restart failed to apply saved policy")
            try expect(store.torrents == before && store.selection == [before[0].id],"Restart lost choices or selection")
            try expect(peer.commands("shutdown").count == 1 && peer.connections == [true] && peer.commands("login").last?.resumeAtLogin == true,"Restart changed host mode or login intent")
        }

        try await check("Rejected restart leaves pending policy and remains retryable") {
            let (store, peer) = library(); await store.refresh()
            store.saveEncryption(.require); try await eventually { !store.savingEncryption }
            peer.hold = ["shutdown"]; store.restartForEncryption()
            try await eventually { peer.pending.count == 1 }
            peer.releaseFirst(error:UserFacingError(message:"Wait for the current operation."))
            try await eventually { !store.switching }
            try expect(store.connected && store.encryptionRestartPending && store.canRestartForEncryption && store.error != nil,"Failed shutdown hid the pending policy or disconnected a running engine")
            store.restartForEncryption(); try await eventually { peer.pending.count == 1 }; peer.releaseFirst()
            try await eventually { !store.switching }
            try expect(!store.encryptionRestartPending && store.error == nil,"Restart could not retry")
        }

        try await check("Failed reconnection preserves the pending policy and library for a later reconnect") {
            let (store, peer) = library(); await store.refresh(); store.backgroundEnabled = false
            let before = store.torrents; store.selection = [before[0].id]
            store.saveEncryption(.require); try await eventually { !store.savingEncryption }
            peer.failConnect = true; store.restartForEncryption()
            try await eventually(timeout:8) { !store.switching }
            try expect(!store.connected && store.error != nil && store.encryptionRestartPending && store.torrents == before && store.selection == [before[0].id],"Failed reconnection erased state or hid the pending preference")
            peer.failConnect = false; await store.start()
            try expect(store.connected && store.peerEncryption == .require && !store.encryptionRestartPending,"A later reconnect did not apply the saved policy")
        }

        try await check("Legacy and offline connection snapshots cannot claim an active port or editable policy") {
            let (store, peer) = library(); peer.peerEncryption = nil; peer.savedPeerEncryption = nil
            await store.refresh()
            try expect(!store.canSaveEncryption && !store.encryptionRestartPending,"An older engine implied configurable encryption")
            let snapshot = LibrarySnapshot(protocolVersion:2,engineVersion:"old",instanceId:"old",listenPort:1234,downloadLimit:0,uploadLimit:0,torrents:[])
            let old = try JSONDecoder().decode(LibrarySnapshot.self,from:JSONEncoder().encode(snapshot))
            try expect(old.peerEncryption == nil && old.savedPeerEncryption == nil,"Older snapshots require policy fields")
            store.connected = false; store.listenPort = 1234
            try expect(store.connectionPortDescription == "Unavailable while disconnected" && !store.canSaveEncryption,"Disconnected engine claimed an active listener")
        }

        try await check("Native import keyboard toggles capture valid rows and preserve other file choices") {
            let session = ImportSession(preview:PreviewFixtures.importPreview)
            session.selected = [0,2]
            session.toggleFiles([0,999])
            try expect(session.selected == [2],"Keyboard toggle changed unrelated files or accepted an unknown row")
            session.toggleFiles([0,1])
            try expect(session.selected == [0,1,2],"Keyboard toggle did not include the captured rows")
            session.toggleFiles([0,1]); session.toggleFiles([])
            try expect(session.selected == [2],"Keyboard deselection changed unrelated choices")
        }

        try await check("Accessibility announcements ignore identical updates and allow recurrence after clearing") {
            var gate = AccessibilityAnnouncementGate()
            try expect(gate.next(nil) == nil && gate.next("  ") == nil,"Empty status was announced")
            try expect(gate.next("  Disk unavailable.  ") == "Disk unavailable.","Error announcement was lost")
            try expect(gate.next("Disk unavailable.") == nil,"Unchanged error was repeatedly announced")
            try expect(gate.next("Check destination access.") == "Check destination access.","Updated guidance was suppressed")
            try expect(gate.next(nil) == nil && gate.next("Check destination access.") == "Check destination access.","A later recurrence was suppressed")
        }

        try await check("Unchanged snapshots publish nothing while changed policy still updates observers") {
            let (store, peer) = library(); await store.refresh()
            store.selection = [store.torrents[0].id]
            var publications = 0
            let observer = store.objectWillChange.sink { publications += 1 }
            defer { observer.cancel() }
            await store.refresh()
            try expect(publications == 0,"An identical snapshot invalidated the native library")
            peer.savedPeerEncryption = .require
            await store.refresh()
            try expect(publications > 0 && store.encryptionRestartPending && store.selection == [store.torrents[0].id],"Suppressing unchanged fields hid a real policy update or lost selection")
        }

        try await check("Derived counts and selected rows track reordering, row edits, operations, and recovery") {
            let (store, peer) = library()
            store.selection = [store.torrents[0].id,store.torrents[4].id,"unknown"]
            @MainActor func verify() throws {
                for filter in LibraryFilter.allCases {
                    try expect(store.count(in:filter) == store.torrents.filter { store.includes($0,in:filter) }.count,"Cached filter count disagreed with current state")
                }
                try expect(store.selected == store.torrents.filter { store.selection.contains($0.id) },"Indexed selection lost values or snapshot order")
            }
            try verify()
            peer.torrents.reverse(); peer.torrents[0].downloadRate += 100
            await store.refresh(); try verify()
            let id = store.torrents[0].id
            peer.torrents[0].state = "error"
            peer.fileOperation = FileOperationItem(id:"operation",torrentId:id,kind:"relocate",phase:"copying",completedBytes:1,totalBytes:2,cancelRequested:false)
            await store.refresh(); try verify()
            peer.fileOperation = nil
            peer.storageRecovery = StorageRecoveryItem(id:"cleanup",torrentId:id,activeLocation:"original",message:"Inspect copies",originalDirectory:"/tmp/old",destinationDirectory:"/tmp/new",canKeepCopies:true)
            await store.refresh(); try verify()
            store.torrents[0].state = "paused"; try verify()
            store.torrents.append(store.torrents[0]); store.selection = [id]; try verify()
            store.torrents = []; try verify()
            let preview = LibraryStore(designPreview:true)
            try expect(preview.count(in:.all) == PreviewFixtures.torrents.count && preview.selected.first?.id == PreviewFixtures.torrents.first?.id,"Design fixture indexes were not initialized")
        }

        try await check("Tracker host choices follow snapshots and retain a missing selected host") {
            let (store, peer) = library()
            peer.torrents[0].trackers = ["https://Tracker.Example/announce", "udp://tracker.example:80/announce", "invalid"]
            peer.torrents = [peer.torrents[0]]; await store.refresh()
            try expect(store.trackerHosts == ["tracker.example"],"Tracker hosts were not normalized or deduplicated")
            store.trackerHost = "missing.example"
            try expect(store.trackerHosts == ["missing.example", "tracker.example"],"Missing selected tracker disappeared")
            peer.torrents[0].trackers = ["https://new.example/announce"]; await store.refresh()
            try expect(store.trackerHosts == ["missing.example", "new.example"],"Tracker host cache survived a changed snapshot")
            store.torrents = []
            try expect(store.trackerHosts == ["missing.example"],"Deleted tracker hosts remained in choices")
        }

        try await check("Tracker and state filters clear hidden selection without changing transfer intent") {
            let (store, peer) = library(); peer.torrents[0].trackers = ["https://one.example.org/private?token=secret"]
            peer.torrents[1].trackers = ["udp://two.example.org:80/announce"]
            peer.torrents[0].state = "queued"; peer.torrents[1].state = "checking"; await store.refresh()
            let choices = store.torrents.map { $0.files }
            store.selection = [store.torrents[0].id]
            var filterPublications = 0
            let filterObserver = store.objectWillChange.sink { filterPublications += 1 }
            store.trackerHost = "two.example.org"
            filterObserver.cancel()
            try expect(filterPublications == 1,"Filtering and hidden-selection cleanup published separate intermediate states")
            try expect(store.selection.isEmpty && store.visibleTorrents.map(\.id) == [store.torrents[1].id],"Tracker filtering retained hidden targets or matched credentials")
            store.clearFilters(); store.filter = .queued
            try expect(store.visibleTorrents.allSatisfy { $0.state == "queued" },"Queue filter included nonqueued rows")
            store.filter = .checking
            try expect(store.visibleTorrents.allSatisfy { $0.state == "checking" } && store.torrents.map { $0.files } == choices,"Checking filter changed file choices")
            let checkingID = store.torrents[1].id
            peer.torrents[1].state = "paused"; await store.refresh()
            try expect(!store.visibleTorrents.contains { $0.id == checkingID },"Live state updates retained cached filter membership")
            store.clearFilters(); store.search = store.torrents[0].name
            let name = store.torrents[0].name
            try expect(store.visibleTorrents.contains { $0.name == name },"Name filter lost its match")
            peer.torrents[0].name = "Changed name"; await store.refresh()
            try expect(!store.visibleTorrents.contains { $0.name == name },"Renamed rows retained cached name matches")
        }
        try await check("Traffic ratio uses payload bytes and diagnostic export omits private values") {
            let (store, _) = library(); var item = store.torrents[0]
            item.networkDownloadedBytes = 100; item.networkUploadedBytes = 50; item.downloadedBytes = 900
            try expect(item.shareRatio == 0.5,"Ratio used verified file progress")
            item.networkDownloadedBytes = 0; try expect(item.shareRatio == nil,"Zero-download ratio claimed a value")
            item.destination = "/private/secret-folder"; item.trackers = ["https://user:pass@tracker.example/secret-passkey"]
            var value = PreviewFixtures.diagnostics(item,section:"trackers")
            value.peers = [PeerDetails(id:"secret-peer",endpoint:"192.0.2.123:51413",client:"client",state:"live",transport:"tcp",encrypted:false,incoming:false,downloaded:1,uploaded:2)]
            store.details = value
            let text = store.diagnosticReport(item)
            try expect(!text.isEmpty && !["secret-folder","secret-passkey","192.0.2.123",item.id,"user:pass"].contains(where:text.contains),"Diagnostic export exposed identifying data")
        }
        try await check("Late details cannot replace a new selection and hidden details stop polling") {
            let (store, peer) = library(); await store.refresh(); store.showInspector = true
            let item = store.torrents[0]; store.selection = [item.id]; peer.hold = ["torrentDetails"]
            let window = NSWindow(contentRect:NSRect(x:0,y:0,width:800,height:560),styleMask:[.titled],backing:.buffered,defer:false)
            window.isReleasedWhenClosed = false; window.title = "SeedHop"; window.orderFront(nil)
            defer { window.close() }
            let task = Task { await store.watchDetails(id:item.id,section:"peers",allPeers:false) }
            try await eventually { peer.pending.count == 1 }
            store.selection = [store.torrents[1].id]
            var value = PreviewFixtures.diagnostics(item,section:"peers"); value.instanceId = "fixture"
            peer.releaseFirst(reply:EngineReply(details:value)); task.cancel(); await task.value
            try expect(store.details == nil,"A late response populated the next transfer")
            store.selection = [item.id]; store.showInspector = false
            let before = peer.commands("torrentDetails").count
            await store.watchDetails(id:item.id,section:"peers",allPeers:false)
            try expect(peer.commands("torrentDetails").count == before,"Hidden details polled the engine")
            store.showInspector = true; peer.hold = []; peer.missingDetails = true
            let missing = Task { await store.watchDetails(id:item.id,section:"peers",allPeers:false) }
            try await eventually { store.detailsError != nil }; missing.cancel(); await missing.value
            try expect(store.connected && store.detailsError?.contains("did not return") == true,"Missing details disconnected the library or left a permanent loading state")
        }

        print("PASS: \(passed) UX presentation regressions using a controlled transport. Native pointer, keyboard, and VoiceOver walkthroughs remain manual.")
    }

    /// A real native window can exercise the existing controlled transport without starting peers.
    static var nativeRequests: [EngineRequest] = []
    static func nativeFixture() -> LibraryStore {
        nativeRequests = []
        let (store, peer) = library(); peer.captureNativeRequests = true
        peer.metadata = PreviewFixtures.importPreview
        return store
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
    private static func eventually(timeout: TimeInterval = 3, _ condition: () -> Bool) async throws {
        let deadline = Date(timeIntervalSinceNow:timeout)
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
    var captureNativeRequests = false
    var pending: [Pending] = []
    var hold = Set<String>()
    var torrents: [TorrentItem] = []
    var metadata: ImportPreview?
    var maxActiveDownloads: UInt32 = 0
    var fileOperation: FileOperationItem?
    var storageRecovery: StorageRecoveryItem?
    var peerEncryption: PeerEncryptionPolicy? = .allow
    var savedPeerEncryption: PeerEncryptionPolicy? = .allow
    var schedule = ScheduleConfig()
    var connections: [Bool] = []
    var stopped = false
    var failConnect = false
    var missingDetails = false
    func connect(background: Bool) throws {
        connections.append(background)
        if failConnect { throw UserFacingError(message:"Engine connection unavailable.") }
    }
    func invalidate() {}
    func commands(_ name: String) -> [EngineRequest] { requests.filter { $0.command == name } }
    func send(_ request: EngineRequest) async throws -> EngineReply {
        requests.append(request)
        if captureNativeRequests { UXSelfTest.nativeRequests.append(request) }
        if hold.contains(request.command) {
            return try await withCheckedThrowingContinuation { pending.append(Pending(request:request,continuation:$0)) }
        }
        return reply(to:request)
    }
    func releaseFirst(at index: Int = 0, reply override: EngineReply? = nil, error: Error? = nil) {
        let item = pending.remove(at:index)
        if let error { item.continuation.resume(throwing:error) }
        else { item.continuation.resume(returning:override ?? reply(to:item.request)) }
    }
    private func reply(to request: EngineRequest) -> EngineReply {
        switch request.command {
        case "connect", "snapshot":
            if request.command == "connect" && stopped { peerEncryption = savedPeerEncryption; stopped = false }
            return EngineReply(snapshot:LibrarySnapshot(protocolVersion:UInt32(SeedHopIdentity.protocolVersion),engineVersion:"test",instanceId:"fixture",listenPort:0,downloadLimit:0,uploadLimit:0,maxActiveDownloads:maxActiveDownloads,torrents:torrents,fileOperation:fileOperation,storageRecovery:storageRecovery,peerEncryption:peerEncryption,savedPeerEncryption:savedPeerEncryption,schedule:schedule))
        case "torrentDetails":
            guard !missingDetails, let item = torrents.first(where:{ $0.id == request.id }) else { return EngineReply() }
            var details = PreviewFixtures.diagnostics(item,section:request.section ?? "general")
            details.instanceId = "fixture"
            return EngineReply(details:details)
        case "shutdown": stopped = true
        case "schedule": schedule = request.schedule ?? schedule
        case "peerEncryption": savedPeerEncryption = request.peerEncryption
        case "retryStorageRecovery", "keepRecoveryCopies": storageRecovery = nil
        case "cancelFileOperation":
            let accepted = fileOperation?.id == request.id && fileOperation?.canCancel == true
            if accepted { fileOperation?.cancelRequested = true }
            return EngineReply(cancellationAccepted:accepted)
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
            fileOperation = nil
        case "rename":
            if let at = torrents.firstIndex(where: { $0.id == request.id }), let file = torrents[at].files.firstIndex(where: { $0.index == request.fileIndex }), let name = request.fileName { torrents[at].files[file].name = name }
            fileOperation = nil
        case "retry":
            if let at = torrents.firstIndex(where: { $0.id == request.id }), torrents[at].state == "error" {
                torrents[at].state = torrents[at].isPaused ? "paused" : "waiting"
                torrents[at].error = nil; torrents[at].errorKind = nil
            }
        case "pause":
            if let at = torrents.firstIndex(where: { $0.id == request.id }) { torrents[at].state = request.paused == true ? "paused" : "downloading" }
        default: break
        }
        return EngineReply()
    }
}
#endif
