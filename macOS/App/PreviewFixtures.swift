import SwiftUI

enum PreviewFixtures {
    static let files = [
        FileItem(index:0,name:"Big Buck Bunny — 4K.mp4",size:854_000_000,downloaded:540_000_000,selected:true),
        FileItem(index:1,name:"Behind the scenes.mp4",size:142_000_000,downloaded:22_000_000,selected:true),
        FileItem(index:2,name:"Artwork/Poster.png",size:8_400_000,downloaded:8_400_000,selected:true),
        FileItem(index:3,name:"LICENSE.txt",size:1_240,downloaded:1_240,selected:true)
    ]
    static let torrents: [TorrentItem] = [
        torrent("01","Big Buck Bunny — Open Movie", "downloading",1_004_401_240,570_401_240,8_400_000,540_000,24,files),
        torrent("02","Blender 4.5 · macOS Apple Silicon.dmg", "downloading",412_000_000,317_000_000,3_200_000,180_000,16),
        torrent("03","Debian 13.0 · arm64.iso", "seeding",3_210_000_000,3_210_000_000,0,1_420_000,12),
        torrent("04","Sintel — Original Soundtrack.flac", "seeding",284_000_000,284_000_000,0,240_000,8),
        torrent("05","Internet Archive · Botanical Atlas", "paused",728_000_000,193_000_000,0,0,0),
        torrent("06","Fedora Workstation · aarch64.iso", "waiting",2_480_000_000,0,0,0,0)
    ]
    private static func torrent(_ id:String,_ name:String,_ state:String,_ total:UInt64,_ downloaded:UInt64,_ down:UInt64,_ up:UInt64,_ peers:UInt32,_ files:[FileItem] = []) -> TorrentItem {
        var item = TorrentItem(id:String(repeating:"0",count:38)+id,name:name,state:state,totalBytes:total,downloadedBytes:downloaded,uploadedBytes:state == "seeding" ? total * 2 : 41_000_000,downloadRate:down,uploadRate:up,peers:peers,destination:NSHomeDirectory()+"/Downloads",files:files.isEmpty ? [FileItem(index:0,name:name,size:total,downloaded:downloaded,selected:true)] : files,trackers:["udp://tracker.example.org:1337/announce"],error:nil)
        item.networkDownloadedBytes = downloaded; item.networkUploadedBytes = state == "seeding" ? total * 2 : 41_000_000
        item.addedAt = 1_792_800_000; item.completedAt = state == "seeding" ? 1_792_850_000 : nil
        item.etaSeconds = down > 0 ? (total - downloaded) / down : nil; item.selectedBytes = total
        return item
    }
    static func diagnostics(_ item: TorrentItem, section: String, peerCount: Int = 6, trackerCount: Int = 3) -> TorrentDetails {
        var peers: [PeerDetails] = []
        for index in 0..<peerCount {
            let endpoint: String = index % 2 == 0 ? "192.0.2.\(index % 250 + 1):51413" : "[2001:db8::\(index + 1)]:51413"
            let client: String = ["qBittorrent","Transmission","SeedHop"][index % 3]
            let downloaded: UInt64 = UInt64(index + 1) * 1_000_000
            let uploaded: UInt64 = UInt64(index + 1) * 250_000
            peers.append(PeerDetails(id:"sample-peer-\(index)",endpoint:endpoint,client:client,state:"live",transport:index % 2 == 0 ? "tcp" : "uTP",encrypted:index % 3 == 0,incoming:index % 2 == 0,downloaded:downloaded,uploaded:uploaded))
        }
        var trackers: [TrackerDetails] = []
        for index in 0..<trackerCount {
            let scheme: String = index % 2 == 0 ? "https" : "udp"
            let address = "\(scheme)://tracker-\(index).example.org"
            let seeds: UInt64? = index == 2 ? nil : UInt64(index) * 3
            let leechers: UInt64? = index == 2 ? nil : UInt64(index)
            trackers.append(TrackerDetails(id:"sample-tracker-\(index)",address:address,endpoint:nil,hash:item.id,state:index == 1 ? "error" : "working",message:index == 1 ? "Tracker connection timed out" : nil,lastAnnounce:1_792_900_000,nextAnnounce:1_792_900_900,peers:UInt64(index) * 8,seeds:seeds,leechers:leechers))
        }
        return TorrentDetails(torrentId:item.id,instanceId:"sample",observedAt:1_792_900_000,section:section,format:"v1",private:false,v1Hash:item.id,v2Hash:nil,dht:true,pex:true,localDiscovery:false,sessionDownloaded:120_000_000,sessionUploaded:32_000_000,seedingSeconds:7200,peers:section == "peers" ? peers : [],trackers:["trackers","activity"].contains(section) ? trackers : [])
    }
    static func trafficHistory() -> [TrafficSample] {
        let now = Date()
        var samples: [TrafficSample] = []
        for index in 0..<60 {
            let timestamp = now.addingTimeInterval(Double(index) - 60)
            let download: UInt64 = 500_000 + UInt64(index) * 100_000
            let upload: UInt64 = 100_000 + UInt64(index) * 10_000
            samples.append(TrafficSample(id:timestamp,download:download,upload:upload))
        }
        return samples
    }
    static var importPreview: ImportPreview { ImportPreview(id:"sample",name:"Big Buck Bunny — Open Movie",totalBytes:1_004_401_240,files:files,torrentBytes:Data()) }
    static var filesInspector: TorrentItem {
        var item = torrents[0]
        item.files = [
            FileItem(index:0,name:"Big.Buck.Bunny.2026.1080p.WEBRip.x264.AAC5.1-[Open.Movie].mp4",size:1_950_000_000,downloaded:819_000_000,selected:true),
            FileItem(index:1,name:"Open Movie - Official site.jpg",size:39_000,downloaded:0,selected:false),
            FileItem(index:2,name:"Open Movie - Read me.txt",size:848,downloaded:0,selected:false),
            FileItem(index:3,name:"Subs/SDH.ita.HI.srt",size:98_000,downloaded:0,selected:false)
        ]
        return item
    }
}

@MainActor enum DesignRenderer {
    static func render() throws {
        let args = ProcessInfo.processInfo.arguments
        WorkspacePreferences.defaults.removePersistentDomain(forName:"app.seedhop.workspace-fixtures")
        let output: URL
        if let index = args.firstIndex(of:"--output"), args.indices.contains(index+1) { output = URL(fileURLWithPath:args[index+1]) }
        else { output = URL(fileURLWithPath:FileManager.default.currentDirectoryPath).appendingPathComponent(".build/design") }
        try FileManager.default.createDirectory(at:output,withIntermediateDirectories:true)
        for (name,appearance) in [("light",NSAppearance.Name.aqua),("dark",.darkAqua)] {
            let store = LibraryStore(designPreview:true)
            try capture(LibraryView().environmentObject(store),size:NSSize(width:1240,height:780),appearance:appearance,to:output.appendingPathComponent("library-\(name).png"))
            store.showInspector = false
            try capture(LibraryView().environmentObject(store),size:NSSize(width:820,height:620),appearance:appearance,to:output.appendingPathComponent("compact-\(name).png"))
            store.showInspector = true; store.selection = [PreviewFixtures.torrents[0].id,PreviewFixtures.torrents[4].id]
            try capture(LibraryView().environmentObject(store),size:NSSize(width:1240,height:780),appearance:appearance,to:output.appendingPathComponent("selection-\(name).png"))
            store.showInspector = false; store.filter = .attention
            try capture(LibraryView().environmentObject(store),size:NSSize(width:1000,height:700),appearance:appearance,to:output.appendingPathComponent("filter-\(name).png"))
            store.filter = .all; store.trackerHost = "unavailable.example.org"
            try capture(LibraryView().environmentObject(store),size:NSSize(width:1000,height:700),appearance:appearance,to:output.appendingPathComponent("tracker-empty-\(name).png"))
            store.trackerHost = ""; store.connected = false
            store.error = "SeedHop couldn’t reach its engine. Your library has been preserved. Try reconnecting to continue."
            try capture(LibraryView().environmentObject(store),size:NSSize(width:1000,height:700),appearance:appearance,to:output.appendingPathComponent("recovery-\(name).png"))
            store.error = nil; store.connected = true
            store.torrents = []; store.selection = []
            try capture(LibraryView().environmentObject(store),size:NSSize(width:1000,height:700),appearance:appearance,to:output.appendingPathComponent("empty-\(name).png"))
            for (name,size) in [("full",NSSize(width:820,height:500)),("short",NSSize(width:820,height:250)),("minimum",NSSize(width:520,height:160))] {
                try capture(LibraryEmptyState().environmentObject(store).background(Design.canvas),size:size,appearance:appearance,to:output.appendingPathComponent("empty-pane-\(name)-\(appearance == .darkAqua ? "dark" : "light").png"))
            }
            try capture(ImportView().environmentObject(store),size:NSSize(width:600,height:432),appearance:appearance,to:output.appendingPathComponent("add-\(name).png"))
            try capture(ImportView(preview:PreviewFixtures.importPreview).environmentObject(store),size:NSSize(width:600,height:680),appearance:appearance,to:output.appendingPathComponent("files-\(name).png"))
            try capture(SettingsView().environmentObject(store),size:NSSize(width:570,height:660),appearance:appearance,to:output.appendingPathComponent("settings-\(name).png"))
            let controls = LibraryStore(designPreview:true)
            controls.torrents[0].files[0].priority = 2
            controls.maxActiveDownloads = 2
            try capture(LibraryView(inspectorTab:1).environmentObject(controls),size:NSSize(width:1240,height:780),appearance:appearance,to:output.appendingPathComponent("file-controls-\(name).png"))
            let inspectorSample = LibraryStore(designPreview:true)
            inspectorSample.torrents = [PreviewFixtures.filesInspector]
            inspectorSample.torrents[0].name = "Open Movie Collection (2026) [1080p] [WEBRip] [5.1] — Complete Edition"
            inspectorSample.selection = [inspectorSample.torrents[0].id]
            for tab in 0...4 {
                inspectorSample.details = PreviewFixtures.diagnostics(inspectorSample.torrents[0],section:["general","files","peers","trackers","activity"][tab])
                try capture(LibraryView(inspectorTab:tab).environmentObject(inspectorSample),size:NSSize(width:1000,height:620),appearance:appearance,to:output.appendingPathComponent("transfer-details-\(tab)-\(name).png"))
            }
            for width in [250,280,360] {
                let sample = LibraryStore(designPreview:true); sample.torrents = [PreviewFixtures.filesInspector]
                let item = sample.torrents[0], session = TorrentFilesSession(files:item.files)
                session.expanded = ["folder:Subs"]; session.selection = ["file:0"]
                try capture(TorrentFilesView(torrent:item,session:session).environmentObject(sample),size:NSSize(width:width,height:520),appearance:appearance,to:output.appendingPathComponent("files-inspector-\(width)-\(name).png"))
            }
            let fileSample = PreviewFixtures.filesInspector
            let searchedFiles = TorrentFilesSession(files:fileSample.files); searchedFiles.query = "ita"
            try capture(TorrentFilesView(torrent:fileSample,session:searchedFiles).environmentObject(controls),size:NSSize(width:280,height:520),appearance:appearance,to:output.appendingPathComponent("files-search-\(name).png"))
            searchedFiles.query = "no matching file"
            try capture(TorrentFilesView(torrent:fileSample,session:searchedFiles).environmentObject(controls),size:NSSize(width:250,height:520),appearance:appearance,to:output.appendingPathComponent("files-no-results-\(name).png"))
            controls.connected = false; searchedFiles.query = ""
            try capture(TorrentFilesView(torrent:fileSample,session:searchedFiles).environmentObject(controls),size:NSSize(width:280,height:520),appearance:appearance,to:output.appendingPathComponent("files-offline-\(name).png"))
            controls.connected = true
            try capture(TorrentFilesView(torrent:fileSample,session:searchedFiles).environmentObject(controls),size:NSSize(width:280,height:520),appearance:name == "dark" ? .accessibilityHighContrastDarkAqua : .accessibilityHighContrastAqua,to:output.appendingPathComponent("files-contrast-\(name).png"))
            try capture(SettingsView(tab:1).environmentObject(controls),size:NSSize(width:570,height:660),appearance:appearance,to:output.appendingPathComponent("queue-settings-\(name).png"))
            controls.fileOperation = FileOperationItem(id:"sample-move",torrentId:controls.torrents[0].id,kind:"relocate",phase:"copying",completedBytes:420_000_000,totalBytes:1_004_401_240,cancelRequested:false)
            try capture(LibraryView().environmentObject(controls),size:NSSize(width:1240,height:880),appearance:appearance,to:output.appendingPathComponent("move-progress-\(name).png"))
            controls.showInspector = false
            try capture(LibraryView().environmentObject(controls),size:NSSize(width:820,height:680),appearance:appearance,to:output.appendingPathComponent("move-compact-\(name).png"))
            controls.fileOperation?.phase = "saving"
            try capture(LibraryView().environmentObject(controls),size:NSSize(width:820,height:680),appearance:appearance,to:output.appendingPathComponent("move-saving-\(name).png"))
            controls.fileOperation = nil; controls.showInspector = true
            controls.storageRecovery = StorageRecoveryItem(id:"sample-recovery",torrentId:controls.torrents[0].id,activeLocation:"original",message:"The original location is still active. Extra copies were retained because the destination is unavailable. Reconnect the drive and retry cleanup, or inspect the folders and keep the extra copies.",originalDirectory:controls.downloadDirectory,destinationDirectory:"/Volumes/External/Transfers",canKeepCopies:true)
            try capture(LibraryView().environmentObject(controls),size:NSSize(width:1240,height:880),appearance:appearance,to:output.appendingPathComponent("move-recovery-\(name).png"))
            controls.showInspector = false; controls.storageRecovery?.activeLocation = "new"
            controls.storageRecovery?.message = "The new location is saved. Some original copies changed and were retained. Inspect both folders before retrying cleanup or keeping the extra copies."
            try capture(LibraryView().environmentObject(controls),size:NSSize(width:820,height:780),appearance:appearance,to:output.appendingPathComponent("move-recovery-compact-\(name).png"))
            let states = LibraryStore(designPreview:true)
            states.torrents[0].state = "checking"; states.torrents[0].pausedIntent = true
            states.torrents[1].state = "queued"; states.torrents[1].queuePosition = 3
            states.torrents[2].state = "waiting"; states.torrents[2].peers = 4
            states.torrents[3].state = "error"; states.torrents[3].errorKind = "storage_space"
            states.torrents[3].error = "Unable to write chunk: No space left on device (os error 28)"
            for index in 0...3 { states.torrents[index].downloadRate = 0; states.torrents[index].uploadRate = 0 }
            states.torrents[2].downloadedBytes /= 2
            states.torrents[3].downloadedBytes = 0; states.torrents[3].peers = 0
            states.selection = [states.torrents[3].id]
            try capture(LibraryView().environmentObject(states),size:NSSize(width:1240,height:880),appearance:appearance,to:output.appendingPathComponent("transfer-error-\(name).png"))
            states.showInspector = false
            try capture(LibraryView().environmentObject(states),size:NSSize(width:820,height:680),appearance:appearance,to:output.appendingPathComponent("transfer-states-compact-\(name).png"))
            states.connected = false
            try capture(LibraryView().environmentObject(states),size:NSSize(width:820,height:680),appearance:appearance,to:output.appendingPathComponent("transfer-stale-\(name).png"))
            controls.filter = .attention
            try capture(LibraryView().environmentObject(controls),size:NSSize(width:1000,height:780),appearance:appearance,to:output.appendingPathComponent("cleanup-attention-\(name).png"))
            controls.torrents = []; controls.selection = []
            try capture(LibraryView().environmentObject(controls),size:NSSize(width:1000,height:780),appearance:appearance,to:output.appendingPathComponent("cleanup-orphan-\(name).png"))
            let preferences = LibraryStore(designPreview:true)
            preferences.peerEncryption = .allow; preferences.savedPeerEncryption = .require
            preferences.listenPort = 51413
            try capture(SettingsView(tab:1).environmentObject(preferences),size:NSSize(width:570,height:660),appearance:appearance,to:output.appendingPathComponent("encryption-pending-\(name).png"))
            preferences.error = "The preference could not be saved. Check library storage access, then choose the policy again."
            try capture(SettingsView(tab:1).environmentObject(preferences),size:NSSize(width:570,height:660),appearance:appearance,to:output.appendingPathComponent("encryption-error-\(name).png"))
            preferences.error = nil
            preferences.connected = false
            try capture(SettingsView(tab:1).environmentObject(preferences),size:NSSize(width:570,height:660),appearance:appearance,to:output.appendingPathComponent("encryption-offline-\(name).png"))
            let contrastStore = LibraryStore(designPreview:true)
            let contrastAppearance: NSAppearance.Name = name == "dark" ? .accessibilityHighContrastDarkAqua : .accessibilityHighContrastAqua
            try capture(LibraryView().environmentObject(contrastStore),size:NSSize(width:1240,height:780),appearance:contrastAppearance,to:output.appendingPathComponent("contrast-library-\(name).png"))
            try capture(ImportView(preview:PreviewFixtures.importPreview).environmentObject(contrastStore),size:NSSize(width:600,height:680),appearance:contrastAppearance,to:output.appendingPathComponent("contrast-import-\(name).png"))
            try capture(SettingsView(tab:1).environmentObject(contrastStore),size:NSSize(width:570,height:660),appearance:contrastAppearance,to:output.appendingPathComponent("contrast-settings-\(name).png"))
            contrastStore.storageRecovery = StorageRecoveryItem(id:"contrast-recovery",torrentId:contrastStore.torrents[0].id,activeLocation:"original",message:"The original location is active. Reconnect the destination and retry cleanup, or inspect the retained copies.",originalDirectory:contrastStore.downloadDirectory,destinationDirectory:"/Volumes/External/Transfers",canKeepCopies:true)
            try capture(LibraryView().environmentObject(contrastStore),size:NSSize(width:1240,height:880),appearance:contrastAppearance,to:output.appendingPathComponent("contrast-recovery-\(name).png"))
            contrastStore.storageRecovery = nil
            contrastStore.showInspector = false
            try capture(LibraryView().environmentObject(contrastStore),size:NSSize(width:820,height:620),appearance:contrastAppearance,to:output.appendingPathComponent("contrast-compact-\(name).png"))
        }
        let workspace = LibraryStore(designPreview:true)
        workspace.torrents = (0..<30).map { index in var item = PreviewFixtures.torrents[index % PreviewFixtures.torrents.count]; item.id += "-\(index)"; return item }
        workspace.selection = [workspace.torrents[0].id]; workspace.showInspector = true
        workspace.history[workspace.torrents[0].id] = PreviewFixtures.trafficHistory()
        workspace.events[workspace.torrents[0].id] = [TransferEvent(message:"Sample tracker connection timed out"),TransferEvent(message:"Sample transfer downloading")]

        for (label, appearance) in [("light",NSAppearance.Name.aqua),("dark",.darkAqua),("contrast-light",.accessibilityHighContrastAqua),("contrast-dark",.accessibilityHighContrastDarkAqua)] {
            for (sizeName,size) in [("minimum",NSSize(width:780,height:560)),("desktop",NSSize(width:1280,height:800)),("wide",NSSize(width:1720,height:1000))] {
                for tab in 0...4 {
                    workspace.details = PreviewFixtures.diagnostics(workspace.torrents[0],section:["general","files","peers","trackers","activity"][tab])
                    try capture(LibraryView(inspectorTab:tab).environmentObject(workspace),size:size,appearance:appearance,to:output.appendingPathComponent("workspace-\(sizeName)-\(tab)-\(label).png"))
                }
            }
        }
        WorkspacePreferences.defaults.set(true,forKey:"workspaceComfortableRows")
        try capture(LibraryView(inspectorTab:0).environmentObject(workspace),size:NSSize(width:1280,height:800),appearance:.aqua,to:output.appendingPathComponent("workspace-comfortable-light.png"))
        WorkspacePreferences.defaults.set(false,forKey:"workspaceComfortableRows")
        print("Rendered workspace and existing fixtures to \(output.path); all data is explicitly labeled sample data")

    }
    private static func capture<V:View>(_ view:V,size:NSSize,appearance:NSAppearance.Name,to url:URL) throws {
        let window = NSWindow(contentRect:NSRect(origin:.zero,size:size),styleMask:[.borderless],backing:.buffered,defer:false)
        window.appearance = NSAppearance(named:appearance)
        let dark = appearance == .darkAqua || appearance == .accessibilityHighContrastDarkAqua
        let contrast = appearance == .accessibilityHighContrastAqua || appearance == .accessibilityHighContrastDarkAqua
        let host = NSHostingView(rootView:view.environment(\.accessibilityAnnouncementsEnabled,false).preferredColorScheme(contrast ? nil : dark ? .dark : .light))
        host.appearance = NSAppearance(named:appearance)
        host.frame = NSRect(origin:.zero,size:size); window.contentView = host
        host.layoutSubtreeIfNeeded()
        // SwiftUI task/layout updates are allowed to settle without capturing the user's screen.
        RunLoop.current.run(until:Date(timeIntervalSinceNow:0.15))
        host.layoutSubtreeIfNeeded()
        guard let bitmap = host.bitmapImageRepForCachingDisplay(in:host.bounds) else { throw UserFacingError(message:"Could not render design") }
        host.cacheDisplay(in:host.bounds,to:bitmap)
        guard let png = bitmap.representation(using:.png,properties:[:]) else { throw UserFacingError(message:"Could not encode design") }
        try png.write(to:url)
        window.contentView = nil
    }
}
