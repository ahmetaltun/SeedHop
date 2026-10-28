#if SEEDHOP_DEVELOPMENT
import SwiftUI
import AppKit
import Combine

/// Controlled model/native-layout measurements, not public-swarm or physical scrolling acceptance.
@MainActor enum PerformanceSelfTest {
    static func fixtureStore() -> LibraryStore {
        WorkspacePreferences.defaults.removePersistentDomain(forName:"app.seedhop.workspace-fixtures")
        let peer = PerformanceTransport()
        let store = LibraryStore(client:peer)
        store.torrents = peer.snapshot.torrents; store.connected = true
        store.backgroundEnabled = false; store.completionNotifications = false
        return store
    }
    static func fixture(_ count: Int, files: Int = 1) -> LibrarySnapshot {
        let torrents = (0..<count).map { index -> TorrentItem in
            let entries = (0..<files).map { file in FileItem(index:UInt32(file),name:String(format:"Sample files/file-%05d.bin",file),size:1_024_000,downloaded:512_000,selected:file % 7 != 6,priority:UInt8(file % 3)) }
            let state = files > 1 ? "downloading" : ["downloading","seeding","paused","waiting","queued"][index % 5]
            return TorrentItem(id:String(format:"%040d",index+1),name:String(format:"Sample transfer %06d",index),state:state,totalBytes:UInt64(files)*1_024_000,downloadedBytes:UInt64(files)*512_000,uploadedBytes:4096,downloadRate:state == "downloading" ? 100_000 : 0,uploadRate:state == "seeding" ? 20_000 : 0,peers:state == "paused" ? 0 : 3,destination:"/tmp/SeedHop-performance-fixture",files:entries,trackers:[],error:nil,queuePosition:UInt32(index+1),pausedIntent:state == "paused")
        }
        return LibrarySnapshot(protocolVersion:UInt32(SeedHopIdentity.protocolVersion),engineVersion:"Controlled performance fixture",instanceId:"performance-fixture",listenPort:0,downloadLimit:0,uploadLimit:0,maxActiveDownloads:0,torrents:torrents,peerEncryption:.allow,savedPeerEncryption:.allow)
    }
    private static func now() -> Double { ProcessInfo.processInfo.systemUptime }
    private static func stats(_ samples: [Double]) -> [String:Any] {
        let sorted = samples.sorted()
        return ["samplesMs":samples,"medianMs":sorted[sorted.count/2],"p95Ms":sorted[min(sorted.count-1,Int(ceil(Double(sorted.count)*0.95))-1)]]
    }
    private static func descendants(_ view: NSView?) -> [NSView] {
        guard let view else { return [] }; return [view] + view.subviews.flatMap { descendants($0) }
    }
    private static func flush(_ window: NSWindow) async throws {
        window.contentView?.layoutSubtreeIfNeeded()
        try await Task.sleep(for:.milliseconds(1))
        window.contentView?.layoutSubtreeIfNeeded()
    }
    private static func wait(_ window: NSWindow, _ condition: () -> Bool) async throws {
        let deadline = now()+20
        while !condition() {
            guard now()<deadline else { throw UserFacingError(message:"Performance fixture did not become ready") }
            try await flush(window)
        }
    }
    static func run(store provider: @MainActor () -> LibraryStore?) async throws {
        try await Task.sleep(for:.milliseconds(500))
        if !NSApp.windows.contains(where:{ $0.isVisible && $0.title == "SeedHop" }) {
            try await InterfaceSelfTest.invoke("Show Library",key:"0")
            try await Task.sleep(for:.milliseconds(500))
        }
        guard let store = provider(), let peer = store.client as? PerformanceTransport,
              let window = NSApp.windows.first(where:{ $0.isVisible && $0.title == "SeedHop" }) else { throw UserFacingError(message:"Performance fixture scene unavailable") }
        window.subtitle = "Performance fixture · sample data"
        window.setContentSize(NSSize(width:1200,height:800)); window.makeKeyAndOrderFront(nil); NSApp.activate(ignoringOtherApps:true)
        var results: [[String:Any]] = []
        for (count, fileCount) in [(100,1),(1000,1),(1,10000)] {
            print("Measuring sample library: \(count) transfers, \(fileCount) files per transfer")
            store.search = ""; store.filter = .all; store.selection = []; store.showInspector = true
            let base = fixture(count,files:fileCount)
            let encoded = try JSONEncoder().encode(base)
            peer.snapshot = try JSONDecoder().decode(LibrarySnapshot.self,from:encoded)
            await store.refresh()
            try await wait(window) { descendants(window.contentView).compactMap { $0 as? NSTableView }.contains { $0.numberOfRows == count } }
            try await flush(window)
            guard let table = descendants(window.contentView).compactMap({ $0 as? NSTableView }).first else { throw UserFacingError(message:"Native library table unavailable") }
            let target = base.torrents[count/2].id; store.selection = [target]
            try await flush(window)
            if fileCount > 1 {
                try await wait(window) { descendants(window.contentView).contains { $0 is NSSegmentedControl } }
                guard let picker = descendants(window.contentView).compactMap({ $0 as? NSSegmentedControl }).first(where: { $0.segmentCount == 5 && $0.label(forSegment:1) == "Files" }), let action = picker.action else { throw UserFacingError(message:"Native Files inspector control unavailable") }
                picker.selectedSegment = 1; NSApp.sendAction(action,to:picker.target,from:picker)
                try await flush(window)
                try await wait(window) { descendants(window.contentView).contains { $0.identifier?.rawValue == "torrent-files-outline" } }
                guard let outline = descendants(window.contentView).compactMap({ $0 as? NSOutlineView }).first(where:{ $0.identifier?.rawValue == "torrent-files-outline" }) else { throw UserFacingError(message:"Native file outline unavailable") }
                // Folder expansion is user state. Expand the fixture before measuring all files.
                let roots = (0..<outline.numberOfRows).compactMap { outline.item(atRow:$0) }
                for root in roots { outline.expandItem(root,expandChildren:true) }
                try await wait(window) { outline.numberOfRows == fileCount + 1 }
            }
            // Detail polling is independent of library snapshots. Settle its first reply
            // before checking whether identical library snapshots publish changes.
            if fileCount == 1 { try await wait(window) { store.details?.torrentId == target } }
            try await flush(window)
            var decode: [Double] = [], encode: [Double] = [], reads: [Double] = [], noop: [Double] = [], updates: [Double] = [], scrolling: [Double] = [], filtering: [Double] = []
            var publications = 0; let observer = store.objectWillChange.sink { publications += 1 }
            var checksum = 0
            for sample in 0..<17 {
                var start = now(); let value = try JSONDecoder().decode(LibrarySnapshot.self,from:encoded); let decoded = (now()-start)*1000
                start = now(); let bytes = try JSONEncoder().encode(value); let encodedMs = (now()-start)*1000; checksum += bytes.count
                peer.snapshot = value
                start = now(); await store.refresh(); try await flush(window); let applied = (now()-start)*1000
                start = now()
                checksum += store.visibleTorrents.sorted(using:[KeyPathComparator(\TorrentItem.name)]).count
                checksum += LibraryFilter.allCases.reduce(0) { $0 + store.count(in:$1) }
                checksum += store.activitySummary.count + store.selected.count
                let read = (now()-start)*1000
                if sample>=2 { decode.append(decoded); encode.append(encodedMs); noop.append(applied); reads.append(read) }
            }
            observer.cancel()
            let unchangedPublicationsPerSnapshot = Double(publications)/17
            table.scrollRowToVisible(count/2); try await flush(window); try await Task.sleep(for:.milliseconds(50))
            let libraryScroll = table.enclosingScrollView
            var fileScroll: NSScrollView?
            if fileCount > 1 {
                fileScroll = descendants(window.contentView).compactMap { $0 as? NSScrollView }.filter { $0 !== libraryScroll && ($0.documentView?.bounds.height ?? 0) > $0.contentView.bounds.height }.max { ($0.documentView?.bounds.height ?? 0) < ($1.documentView?.bounds.height ?? 0) }
                guard let fileScroll else { throw UserFacingError(message:"Large file inspector scroll view unavailable") }
                let height = fileScroll.documentView!.bounds.height-fileScroll.contentView.bounds.height
                fileScroll.contentView.scroll(to:NSPoint(x:0,y:height/2)); fileScroll.reflectScrolledClipView(fileScroll.contentView)
                try await flush(window); try await Task.sleep(for:.milliseconds(100))
            }
            var anchorsStable = true, selectionStable = true
            let profile = count == 1000 && ProcessInfo.processInfo.arguments.contains("--profile-performance")
            if profile { fputs("PROFILE changing 1000 transfers pid=\(ProcessInfo.processInfo.processIdentifier)\n",stderr) }
            for sample in 0..<(profile ? 250 : 11) {
                var value = try JSONDecoder().decode(LibrarySnapshot.self,from:encoded)
                for index in value.torrents.indices {
                    if value.torrents[index].state == "downloading" { value.torrents[index].downloadRate += UInt64(sample+1) }
                }
                if fileCount>1 { value.torrents[0].files[fileCount-1].downloaded += UInt64(sample+1) }
                peer.snapshot = value
                let before = (fileScroll ?? libraryScroll)?.contentView.bounds.origin.y ?? 0
                let start = now(); await store.refresh(); try await flush(window); let elapsed = (now()-start)*1000
                let after = (fileScroll ?? libraryScroll)?.contentView.bounds.origin.y ?? 0
                anchorsStable = anchorsStable && abs(before-after)<2
                selectionStable = selectionStable && store.selection == [target] && table.selectedRowIndexes == IndexSet(integer:count/2)
                if sample>=2 { updates.append(elapsed) }
            }
            for sample in 0..<11 {
                let start = now()
                if let fileScroll {
                    let height = fileScroll.documentView!.bounds.height-fileScroll.contentView.bounds.height
                    fileScroll.contentView.scroll(to:NSPoint(x:0,y:sample % 2 == 0 ? 0 : height*0.75)); fileScroll.reflectScrolledClipView(fileScroll.contentView)
                } else { table.scrollRowToVisible(sample % 2 == 0 ? 0 : count-1) }
                try await flush(window)
                if sample>=2 { scrolling.append((now()-start)*1000) }
            }
            var filterSelectionCorrect = true
            if count>1 {
                // A person types into an already-focused search field. Focus it
                // before timing so toolbar expansion is not attributed to filtering.
                try await InterfaceSelfTest.invoke("Find Transfer…",key:"f")
                try await Task.sleep(for:.milliseconds(300)); try await flush(window)
                let profileFilter = count == 1000 && ProcessInfo.processInfo.arguments.contains("--profile-filter")
                if profileFilter { fputs("PROFILE filter 1000 transfers pid=\(ProcessInfo.processInfo.processIdentifier)\n",stderr) }
                for sample in 0..<(profileFilter ? 250 : 11) {
                    store.search = ""; try await wait(window) { table.numberOfRows == count }; try await flush(window); store.selection = [target]
                    // Row count changes before AppKit finishes restoring selection and
                    // layout. Keep that previous operation outside the filter timing.
                    try await Task.sleep(for:.milliseconds(200)); try await flush(window)
                    guard table.selectedRowIndexes == IndexSet(integer:count/2) else {
                        let native = descendants(window.contentView).compactMap { $0 as? NSTableView }.map { "\(type(of:$0)) rows=\($0.numberOfRows) selected=\($0.selectedRowIndexes) frame=\($0.frame)" }
                        throw UserFacingError(message:"Restored table selection differs: native \(table.selectedRowIndexes), expected \(count/2), stored \(store.selection.count); tables \(native)")
                    }
                    var filterPublications = 0
                    let filterObserver = store.objectWillChange.sink { filterPublications += 1 }
                    let start = now(); store.search = "Sample transfer 00009"; try await wait(window) { table.numberOfRows == 10 }
                    if sample>=2 { filtering.append((now()-start)*1000) }
                    filterObserver.cancel()
                    if sample == 2 { print("Filter model publications: \(count) rows, \(filterPublications)") }
                    filterSelectionCorrect = filterSelectionCorrect && store.selection.isEmpty
                }
                store.search = ""; try await wait(window) { table.numberOfRows == count }
            }
            results.append(["transfers":count,"filesPerTransfer":fileCount,"payloadBytes":encoded.count,"jsonDecode":stats(decode),"jsonEncode":stats(encode),"modelReads":stats(reads),"unchangedSnapshotLayout":stats(noop),"unchangedPublicationsPerSnapshot":unchangedPublicationsPerSnapshot,"changingSnapshotLayout":stats(updates),"programmaticScrollLayout":stats(scrolling),"filterUntilNativeRows":filtering.isEmpty ? [:] : stats(filtering),"selectionStableDuringUpdates":selectionStable,"scrollAnchorStableDuringUpdates":anchorsStable,"filterClearsHiddenSelection":count>1 ? filterSelectionCorrect as Any : NSNull(),"checksum":checksum])
            print("Measured sample library: \(count)/\(fileCount), updates median \(stats(updates)["medianMs"]!), unchanged publications \(unchangedPublicationsPerSnapshot)")
        }
        var diagnosticResults: [[String:Any]] = []
        for (section,rows) in [("peers",1000),("trackers",100)] {
            let fixtureStore = LibraryStore(designPreview:true), item = PreviewFixtures.torrents[0]
            var value = PreviewFixtures.diagnostics(item,section:section,peerCount:section == "peers" ? rows : 0,trackerCount:section == "trackers" ? rows : 0)
            fixtureStore.details = value
            let diagnosticWindow = NSWindow(contentRect:NSRect(x:0,y:0,width:1200,height:500),styleMask:[.titled],backing:.buffered,defer:false)
            diagnosticWindow.isReleasedWhenClosed = false; diagnosticWindow.title = "Diagnostic performance fixture"
            diagnosticWindow.contentView = NSHostingView(rootView:TorrentDiagnosticsView(torrent:item,tab:section).environmentObject(fixtureStore))
            diagnosticWindow.orderFront(nil)
            try await wait(diagnosticWindow) { descendants(diagnosticWindow.contentView).compactMap { $0 as? NSTableView }.contains { $0.numberOfRows == rows } }
            guard let table = descendants(diagnosticWindow.contentView).compactMap({ $0 as? NSTableView }).first else { throw UserFacingError(message:"Diagnostic table unavailable") }
            var layouts:[Double] = [], scrolling:[Double] = [], decode:[Double] = []
            for sample in 0..<11 {
                value.observedAt += 1
                if section == "peers" { for index in value.peers.indices { value.peers[index].uploaded += UInt64(sample + 1) } }
                else { for index in value.trackers.indices { value.trackers[index].nextAnnounce = UInt64(sample) } }
                let bytes = try JSONEncoder().encode(value)
                var start = now(); let decoded = try JSONDecoder().decode(TorrentDetails.self,from:bytes); let decodeMs = (now()-start)*1000
                start = now(); fixtureStore.details = decoded; try await flush(diagnosticWindow); let layout = (now()-start)*1000
                start = now(); table.scrollRowToVisible(sample % 2 == 0 ? 0 : rows-1); try await flush(diagnosticWindow); let scroll = (now()-start)*1000
                if sample >= 2 { layouts.append(layout); scrolling.append(scroll); decode.append(decodeMs) }
            }
            diagnosticResults.append(["section":section,"rows":rows,"changingSnapshotLayout":stats(layouts),"programmaticScrollLayout":stats(scrolling),"jsonDecode":stats(decode)])
            diagnosticWindow.close()
        }
        let args = ProcessInfo.processInfo.arguments
        guard let at=args.firstIndex(of:"--output"),args.indices.contains(at+1) else { throw UserFacingError(message:"Pass --output for performance evidence") }
        let result: [String:Any] = ["fixture":true,"profile":"debug","profilingOnly":args.contains("--profile-performance") || args.contains("--profile-filter"),"cases":results,"diagnostics":diagnosticResults,"measurement":"Monotonic wall time through model work and native layout flush; includes a 1 ms yield, excludes physical frame presentation and XPC transport."]
        try JSONSerialization.data(withJSONObject:result,options:[.prettyPrinted,.sortedKeys]).write(to:URL(fileURLWithPath:args[at+1]))
        print("Recorded controlled performance measurements. Physical scrolling and real-library acceptance remain separate.")
    }
}

@MainActor private final class PerformanceTransport: EngineTransport {
    var onInterruption: (() -> Void)?
    var snapshot = PerformanceSelfTest.fixture(100)
    func connect(background: Bool) throws {}
    func invalidate() {}
    func send(_ request: EngineRequest) async throws -> EngineReply {
        if request.command == "torrentDetails", let item = snapshot.torrents.first(where:{ $0.id == request.id }) {
            var details = PreviewFixtures.diagnostics(item,section:request.section ?? "general",peerCount:0,trackerCount:0)
            details.instanceId = snapshot.instanceId
            return EngineReply(details:details)
        }
        guard request.command == "snapshot" else { throw UserFacingError(message:"Performance fixture accepts read-only requests.") }
        return EngineReply(snapshot:snapshot)
    }
}
#endif
