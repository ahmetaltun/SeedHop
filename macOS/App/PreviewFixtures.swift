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
        TorrentItem(id:String(repeating:"0",count:38)+id,name:name,state:state,totalBytes:total,downloadedBytes:downloaded,uploadedBytes:state == "seeding" ? total * 2 : 41_000_000,downloadRate:down,uploadRate:up,peers:peers,destination:NSHomeDirectory()+"/Downloads",files:files.isEmpty ? [FileItem(index:0,name:name,size:total,downloaded:downloaded,selected:true)] : files,trackers:["udp://tracker.example.org:1337/announce"],error:nil)
    }
    static var importPreview: ImportPreview { ImportPreview(id:"sample",name:"Big Buck Bunny — Open Movie",totalBytes:1_004_401_240,files:files,torrentBytes:Data()) }
}

@MainActor enum DesignRenderer {
    static func render() throws {
        let args = ProcessInfo.processInfo.arguments
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
            store.filter = .all; store.connected = false
            store.error = "SeedHop couldn’t reach its engine. Your library has been preserved. Try reconnecting to continue."
            try capture(LibraryView().environmentObject(store),size:NSSize(width:1000,height:700),appearance:appearance,to:output.appendingPathComponent("recovery-\(name).png"))
            store.error = nil; store.connected = true
            store.torrents = []; store.selection = []
            try capture(LibraryView().environmentObject(store),size:NSSize(width:1000,height:700),appearance:appearance,to:output.appendingPathComponent("empty-\(name).png"))
            try capture(ImportView().environmentObject(store),size:NSSize(width:600,height:432),appearance:appearance,to:output.appendingPathComponent("add-\(name).png"))
            try capture(ImportView(preview:PreviewFixtures.importPreview).environmentObject(store),size:NSSize(width:600,height:680),appearance:appearance,to:output.appendingPathComponent("files-\(name).png"))
            try capture(SettingsView().environmentObject(store),size:NSSize(width:570,height:560),appearance:appearance,to:output.appendingPathComponent("settings-\(name).png"))
            let contrastStore = LibraryStore(designPreview:true)
            let contrastAppearance: NSAppearance.Name = name == "dark" ? .accessibilityHighContrastDarkAqua : .accessibilityHighContrastAqua
            try capture(LibraryView().environmentObject(contrastStore),size:NSSize(width:1240,height:780),appearance:contrastAppearance,to:output.appendingPathComponent("contrast-library-\(name).png"))
            contrastStore.showInspector = false
            try capture(LibraryView().environmentObject(contrastStore),size:NSSize(width:820,height:620),appearance:contrastAppearance,to:output.appendingPathComponent("contrast-compact-\(name).png"))
        }
        print("Rendered 22 fixture screens to \(output.path); library fixtures are explicitly labeled sample data")
    }
    private static func capture<V:View>(_ view:V,size:NSSize,appearance:NSAppearance.Name,to url:URL) throws {
        let window = NSWindow(contentRect:NSRect(origin:.zero,size:size),styleMask:[.borderless],backing:.buffered,defer:false)
        window.appearance = NSAppearance(named:appearance)
        let dark = appearance == .darkAqua || appearance == .accessibilityHighContrastDarkAqua
        let contrast = appearance == .accessibilityHighContrastAqua || appearance == .accessibilityHighContrastDarkAqua
        let host = NSHostingView(rootView:view.preferredColorScheme(contrast ? nil : dark ? .dark : .light))
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
