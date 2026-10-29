#if SEEDHOP_DEVELOPMENT
import SwiftUI
import AppKit

/// Exercises this process's fixture window and native menus, without system input.
@MainActor enum InterfaceSelfTest {
    static func run(store provider: @MainActor () -> LibraryStore?) async throws {
        try await settle()
        // macOS may restore a previous no-window state when this test is repeated.
        if !NSApp.windows.contains(where:{ $0.isVisible && $0.title == "SeedHop" }) {
            try await invoke("Show Library",key:"0")
            try await settle()
        }
        guard let window = NSApp.windows.first(where:{ $0.isVisible && $0.title == "SeedHop" }) else {
            throw UserFacingError(message:"No library fixture window")
        }
        window.subtitle = "Native test fixture · sample data"
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps:true)
        try await settle()
        print("System accessibility settings: fullKeyboardAccess=\(NSApp.isFullKeyboardAccessEnabled), reduceMotion=\(NSWorkspace.shared.accessibilityDisplayShouldReduceMotion), increaseContrast=\(NSWorkspace.shared.accessibilityDisplayShouldIncreaseContrast)")
        print("Native fixture state: active=\(NSApp.isActive), key=\(window.isKeyWindow), main=\(window.isMainWindow), responder=\(String(describing:window.firstResponder))")
        try await invoke("Find Transfer…",key:"f")
        try await settle()
        guard let editor = window.firstResponder as? NSTextView, editor.isFieldEditor,
              let field = editor.delegate as? NSTextField,
              field is NSSearchField else {
            throw UserFacingError(message:"Find did not focus the search field: \(String(describing:window.firstResponder))")
        }
        print("PASS: Command-F menu action focuses the native search field.")
        try await invoke("Focus Transfer List",key:"l")
        try await settle()
        guard window.firstResponder is NSTableView else {
            throw UserFacingError(message:"Focus Transfer List did not focus the native table: \(String(describing:window.firstResponder))")
        }
        print("PASS: Command-L menu action focuses the native transfer table.")
        let args = ProcessInfo.processInfo.arguments
        if let index = args.firstIndex(of:"--output"), args.indices.contains(index+1) {
            try capture(window,to:URL(fileURLWithPath:args[index+1]))
        }
        window.close()
        try await settle()
        try await invoke("Show Library",key:"0")
        try await settle()
        guard NSApp.windows.contains(where:{ $0.isVisible && $0.title == "SeedHop" }) else {
            throw UserFacingError(message:"Show Library did not reopen the closed library")
        }
        print("PASS: Command-0 menu action reopens the library after the last window closes.")
        guard let store = provider(), let reopened = NSApp.windows.first(where: { $0.isVisible && $0.title == "SeedHop" }) else {
            throw UserFacingError(message:"Library fixture store unavailable")
        }
        reopened.makeKeyAndOrderFront(nil)
        try await invoke("Add Torrent…",key:"n"); try await settle()
        guard let sheet = reopened.attachedSheet,
              let editor = sheet.firstResponder as? NSTextView, editor.isFieldEditor,
              editor.delegate is NSTextField else {
            throw UserFacingError(message:"Import did not focus the magnet input: \(String(describing:reopened.attachedSheet?.firstResponder))")
        }
        print("PASS: Add Torrent opens a native sheet with its magnet input focused.")
        store.showImport = false; try await settle()
        guard reopened.attachedSheet == nil, reopened.firstResponder is NSTableView else {
            throw UserFacingError(message:"Closing import did not return focus to the transfer table: \(String(describing:reopened.firstResponder))")
        }
        print("PASS: Dismissing import returns focus to the native transfer table.")
        try await verifyControlledSheets(store:store,window:reopened)
        try await verifyFilesOutline()
        try await verifyAlternativeBandwidthSwitch(store:store,window:reopened)
        try await verifySchedulingSettings()
        try await verifyNativeTransferControls()
        try await verifyWorkspaceLayout()
        print("Native key-event delivery, VoiceOver speech, live-transfer reopening, and scrolling remain manual acceptance checks.")
    }

    static func verifyWorkspaceLayout() async throws {
        WorkspacePreferences.defaults.set(false,forKey:"workspaceComfortableRows")
        WorkspacePreferences.defaults.set(true,forKey:"workspaceSidebarVisible")
        WorkspacePreferences.defaults.removeObject(forKey:"workspaceDivider.SeedHop.Library.Sidebar")
        WorkspacePreferences.defaults.removeObject(forKey:"workspaceDivider.SeedHop.Library.Details")
        let store = LibraryStore(designPreview:true)
        store.torrents = (0..<30).map { index in var item = PreviewFixtures.torrents[index % PreviewFixtures.torrents.count]; item.id += "-\(index)"; return item }
        store.selection = [store.torrents[0].id]; store.showInspector = true
        let window = NSWindow(contentRect:NSRect(x:0,y:0,width:1280,height:800),styleMask:[.titled,.resizable],backing:.buffered,defer:false)
        window.isReleasedWhenClosed = false; window.title = "Workspace layout fixture"
        func mount() { window.contentView = NSHostingView(rootView:LibraryView(inspectorTab:0).environmentObject(store)) }
        mount(); window.makeKeyAndOrderFront(nil); defer { window.close() }
        try await settle()
        let fixtureRows = store.torrents
        store.torrents = []; store.selection = []; try await settle()
        store.torrents = [fixtureRows[0]]; store.selection = [fixtureRows[0].id]; try await settle()
        try verifyTransferNameRendering(in:window)
        store.torrents[0].downloadRate += 500; try await settle()
        try verifyTransferNameRendering(in:window)
        store.torrents = fixtureRows; try await settle()
        let nativeTables = descendants(window.contentView).compactMap { $0 as? NSTableView }
        guard let table = nativeTables.first(where:{ $0.numberOfRows == 30 && $0.tableColumns.first?.headerCell.stringValue == "Name" }), table.numberOfRows == 30 else { throw UserFacingError(message:"Workspace transfer table unavailable: \(nativeTables.map { "\($0.identifier?.rawValue ?? "nil"):\($0.numberOfRows):\($0.tableColumns.map { $0.identifier.rawValue + "=" + $0.headerCell.stringValue })" })") }
        print("Visible transfer columns: \(table.tableColumns.filter { !$0.isHidden }.map { $0.headerCell.stringValue })")
        print("Native transfer table: type=\(type(of:table)), automatic=\(table.usesAutomaticRowHeights), style=\(table.rowSizeStyle.rawValue), rowHeight=\(table.rowHeight), spacing=\(table.intercellSpacing)")
        let rowHeight = table.rect(ofRow:0).height
        let viewport = table.visibleRect
        let visible = (0..<table.numberOfRows).filter {
            let row = table.rect(ofRow:$0)
            return row.minY >= viewport.minY && row.maxY <= viewport.maxY
        }.count
        guard visible >= 12, abs(rowHeight - 28) < 1, abs(table.rect(ofRow:1).minY - table.rect(ofRow:0).minY - 28) < 1 else { throw UserFacingError(message:"Compact workspace shows \(visible) full rows at \(rowHeight) points") }
        guard !table.usesAutomaticRowHeights else { throw UserFacingError(message:"Native workspace did not retain fixed row heights") }
        WorkspacePreferences.defaults.set(true,forKey:"workspaceComfortableRows"); try await settle()
        guard abs(table.rect(ofRow:0).height - 40) < 1 else { throw UserFacingError(message:"Comfortable density did not produce 40-point native rows") }
        WorkspacePreferences.defaults.set(false,forKey:"workspaceComfortableRows"); try await settle()
        guard abs(table.rect(ofRow:0).height - 28) < 1 else { throw UserFacingError(message:"Compact density did not restore 28-point native rows") }
        let splits = descendants(window.contentView).compactMap { $0 as? NSSplitView }
        let splitFrames: [String] = splits.map { "vertical=\($0.isVertical) bounds=\($0.bounds) panes=\($0.subviews.map { $0.frame })" }
        print("Workspace split frames: \(splitFrames)")
        guard let sidebar = splits.first(where:{ $0.isVertical && $0.arrangedSubviews.count == 2 }),
              let details = splits.first(where:{ !$0.isVertical && $0.arrangedSubviews.count == 2 }),
              abs(sidebar.arrangedSubviews[0].frame.width - 180) < 2,
              abs(details.arrangedSubviews[1].frame.height - 280) < 2 else { throw UserFacingError(message:"Default workspace dividers do not preserve 180-point sidebar and 280-point details") }
        guard let name = table.tableColumns.first else { throw UserFacingError(message:"Name column unavailable") }
        sidebar.setPosition(210,ofDividerAt:0)
        details.setPosition(details.bounds.height - 320 - details.dividerThickness,ofDividerAt:0)
        try await settle()
        guard abs(sidebar.arrangedSubviews[0].frame.width - 210) < 2, abs(details.arrangedSubviews[1].frame.height - 320) < 2 else { throw UserFacingError(message:"Native divider changes did not retain the requested pane sizes") }
        guard let nativeCoordinator = table.dataSource as? NativeTransferGrid.Coordinator,
              let last = nativeCoordinator.rows.last,
              let source = store.torrents.firstIndex(where:{$0.id == last.id}),
              let upload = table.tableColumns.firstIndex(where:{$0.identifier.rawValue == "upload"}) else { throw UserFacingError(message:"Offscreen native rate fixture unavailable") }
        _ = table.rowView(atRow:29,makeIfNecessary:true)
        _ = table.view(atColumn:upload,row:29,makeIfNecessary:true)
        store.torrents[source].uploadRate = 12_345; try await settle()
        if let cached = table.view(atColumn:upload,row:29,makeIfNecessary:false) as? NSTableCellView {
            guard cached.textField?.stringValue == SizeText.rate(12_345) else { throw UserFacingError(message:"Offscreen cached native row retained an old rate") }
        }
        name.width = 275
        NotificationCenter.default.post(name:NSTableView.columnDidResizeNotification,object:table)
        try await settle()
        store.torrents[0].downloadRate += 500; try await settle()
        try verifyTransferNameRendering(in:window)
        table.scrollRowToVisible(29); try await settle()
        try verifyTransferNameRendering(in:window,row:29)
        table.scrollRowToVisible(0); try await settle()
        guard table.selectedRowIndexes == IndexSet(integer:0) else { throw UserFacingError(message:"Live transfer update lost the native selected-row highlight") }
        guard abs(name.width - 275) < 2 else { throw UserFacingError(message:"Live transfer update changed a resized column: \(name.width), saved \(String(describing:WorkspacePreferences.defaults.dictionary(forKey:"workspaceColumnWidths")?["Name"]))") }
        window.contentView = nil; mount(); try await settle()
        guard let restored = descendants(window.contentView).compactMap({ $0 as? NSTableView }).first(where:{ $0.numberOfRows == 30 && $0.tableColumns.first?.headerCell.stringValue == "Name" }),
              let restoredName = restored.tableColumns.first, abs(restoredName.width - 275) < 2 else { throw UserFacingError(message:"Native transfer column width did not persist across remount") }
        let restoredSplits = descendants(window.contentView).compactMap { $0 as? NSSplitView }
        guard let restoredSidebar = restoredSplits.first(where:{ $0.isVertical }), let restoredDetails = restoredSplits.first(where:{ !$0.isVertical }),
              abs(restoredSidebar.arrangedSubviews[0].frame.width - 210) < 2, abs(restoredDetails.arrangedSubviews[1].frame.height - 320) < 2 else { throw UserFacingError(message:"User pane sizes did not persist across remount") }
        WorkspacePreferences.defaults.set(true,forKey:"workspaceComfortableRows"); try await settle()
        guard abs(restored.rowHeight - 40) < 1, abs(restored.rect(ofRow:1).minY - restored.rect(ofRow:0).minY - 40) < 1 else { throw UserFacingError(message:"Comfortable density did not apply 40-point native rows") }
        try verifyTransferNameRendering(in:window)
        WorkspacePreferences.defaults.set(false,forKey:"workspaceComfortableRows"); try await settle()
        guard abs(restored.rowHeight - 28) < 1 else { throw UserFacingError(message:"Compact density did not restore 28-point native rows") }
        WorkspaceTablePersistence.restoreWidths()
        print("PASS: Native density switches between 28 and 40 points with visible text.")
        print("PASS: 1280×800 workspace shows \(visible) full 28-point rows; default dividers, changed pane sizes, and native column-width persistence pass.")
    }

    private static func verifyTransferNameRendering(in window: NSWindow, row: Int = 0) throws {
        guard let host = window.contentView,
              let table = descendants(host).compactMap({ $0 as? NSTableView }).first(where:{ $0.numberOfRows > row && $0.tableColumns.first?.headerCell.stringValue == "Name" }) else {
            throw UserFacingError(message:"First arriving transfer did not create a table")
        }
        // Inspect the name's pixels, excluding row borders. Row count and
        // accessibility values still pass when SwiftUI renders blank text.
        let rect = host.convert(table.frameOfCell(atColumn:0,row:row).insetBy(dx:20,dy:4),from:table)
        guard let bitmap = host.bitmapImageRepForCachingDisplay(in:rect) else { throw UserFacingError(message:"Could not render arriving transfer name") }
        host.cacheDisplay(in:rect,to:bitmap)
        var darkest: CGFloat = 1, lightest: CGFloat = 0
        for y in 0..<bitmap.pixelsHigh { for x in 0..<bitmap.pixelsWide {
            guard let color = bitmap.colorAt(x:x,y:y)?.usingColorSpace(.deviceRGB) else { continue }
            let brightness = (color.redComponent + color.greenComponent + color.blueComponent) / 3
            darkest = min(darkest,brightness); lightest = max(lightest,brightness)
        }}
        if lightest-darkest <= 0.15 {
            print("Blank viewport:",table.visibleRect,table.rows(in:table.visibleRect),String(describing:table.enclosingScrollView?.contentView.bounds))
            print("Blank name geometry:",table.tableColumns.map { ($0.headerCell.stringValue,$0.width) },table.frameOfCell(atColumn:0,row:row))
            if let cell = table.view(atColumn:0,row:row,makeIfNecessary:false) { print("Name cell:",cell.frame,cell.subviews.map { $0.frame }) }
        }
        guard lightest-darkest > 0.15 else { throw UserFacingError(message:"First arriving transfer has blank name text") }
        print("PASS: First arriving transfer name renders visible text after an empty library and live updates.")
    }

    private static func verifyAlternativeBandwidthSwitch(store: LibraryStore, window: NSWindow) async throws {
        await store.refresh()
        try await settle()
        guard let control = descendants(window.contentView).compactMap({ $0 as? NSSwitch }).first,
              !control.isEnabled, control.state == .off else { throw UserFacingError(message:"Unset alternative bandwidth switch was enabled") }
        // The tortoise is the last content button, immediately before the switch.
        guard let tortoise = descendants(window.contentView).compactMap({ $0 as? NSButton }).last else { throw UserFacingError(message:"Bandwidth settings button unavailable") }
        tortoise.performClick(nil)
        try await waitUntil("Tortoise did not open the bandwidth window") { NSApp.windows.contains { $0.isVisible && $0.title == "Bandwidth Limits" } }
        guard let settings = NSApp.windows.first(where:{ $0.isVisible && $0.title == "Bandwidth Limits" }) else { throw UserFacingError(message:"Bandwidth settings window unavailable") }
        defer { settings.close() }
        try await settle()
        let args = ProcessInfo.processInfo.arguments
        if let index = args.firstIndex(of:"--output"), args.indices.contains(index+1) {
            let url = URL(fileURLWithPath:args[index+1]).deletingPathExtension().appendingPathExtension("bandwidth-unset.png")
            try capture(settings,to:url)
        }
        try await store.setAlternativeLimits(download:12_000,upload:3_000,enabled:false)
        try await settle()
        guard control.isEnabled else { throw UserFacingError(message:"Saved alternative caps did not enable the switch") }
        control.performClick(nil)
        try await waitUntil("Alternative bandwidth switch did not enable limits") { store.alternativeLimitsEnabled }
        guard let request = UXSelfTest.nativeRequests.last(where:{$0.command == "alternativeLimits"}),
              request.alternativeLimitsEnabled == true, request.downloadLimit == 12_000, request.uploadLimit == 3_000 else { throw UserFacingError(message:"Bandwidth switch sent incorrect limits") }
        try await settle()
        if let index = args.firstIndex(of:"--output"), args.indices.contains(index+1) {
            let url = URL(fileURLWithPath:args[index+1]).deletingPathExtension().appendingPathExtension("bandwidth-active.png")
            try capture(settings,to:url)
        }
        control.performClick(nil)
        try await waitUntil("Alternative bandwidth switch did not restore global limits") { !store.alternativeLimitsEnabled }
        print("PASS: Unset alternative switch stays disabled, tortoise opens bandwidth settings, saved caps enable switching, and profiles stay synchronized.")
    }

    private static func verifySchedulingSettings() async throws {
        let store = UXSelfTest.nativeFixture()
        await store.refresh()
        let window = NSWindow(contentRect:NSRect(x:0,y:0,width:570,height:660),styleMask:[.titled],backing:.buffered,defer:false)
        window.isReleasedWhenClosed = false; window.title = "Weekly scheduling · native fixture"
        window.contentView = NSHostingView(rootView:ScrollView { SchedulingSettingsView().padding(24) }.environmentObject(store))
        window.makeKeyAndOrderFront(nil); defer { window.close() }
        try await settle()
        // SwiftUI renders button labels in separate hosting children rather
        // than NSButton.title. Exercise the three footer actions in source order;
        // the captured request below verifies the resulting action and payload.
        func footerButtons() -> [NSButton] { Array(descendants(window.contentView).compactMap { $0 as? NSButton }.suffix(3)) }
        let initial = footerButtons()
        guard initial.count == 3, initial[0].isEnabled else { throw UserFacingError(message:"Schedule footer controls unavailable") }
        initial[0].performClick(nil); try await settle()
        let edited = footerButtons()
        guard !UXSelfTest.nativeRequests.contains(where:{$0.command == "schedule"}), edited.count == 3, edited[2].isEnabled else {
            throw UserFacingError(message:"Editing a schedule sent a command before Save or hid Save")
        }
        let save = edited[2]
        save.performClick(nil)
        try await waitUntil("Schedule Save did not reach transport") { UXSelfTest.nativeRequests.contains { $0.command == "schedule" } }
        guard let sent = UXSelfTest.nativeRequests.last(where:{$0.command == "schedule"})?.schedule,
              !sent.enabled, sent.rules == [ScheduleRule()] else { throw UserFacingError(message:"Native schedule controls sent different rules") }
        try await settle()
        guard store.schedule == sent else { throw UserFacingError(message:"Saved schedule did not return through the store") }
        print("PASS: Native schedule Add and Save controls keep drafts local and send the expected weekly window.")
    }

    private static func verifyNativeTransferControls() async throws {
        let defaults = WorkspacePreferences.defaults
        let keys = ["transferColumns",TransferColumns.orderKey,"workspaceSortOrder","workspaceColumnWidths"]
        let saved = keys.map { defaults.object(forKey:$0) }
        defer { for (key,value) in zip(keys,saved) { if let value { defaults.set(value,forKey:key) } else { defaults.removeObject(forKey:key) } } }
        defaults.removeObject(forKey:TransferColumns.orderKey)
        let store = UXSelfTest.nativeFixture(); await store.refresh()
        let window = NSWindow(contentRect:NSRect(x:0,y:0,width:1200,height:800),styleMask:[.titled],backing:.buffered,defer:false)
        window.isReleasedWhenClosed = false; window.title = "Transfer controls · native fixture"
        func mount() { window.contentView = NSHostingView(rootView:LibraryView().environmentObject(store)) }
        func native() throws -> NativeTransfersTable {
            guard let table = descendants(window.contentView).compactMap({ $0 as? NativeTransfersTable }).first else { throw UserFacingError(message:"Native transfer controls unavailable") }
            return table
        }
        func invoke(_ item: NSMenuItem) throws {
            guard item.isEnabled, let action = item.action, NSApp.sendAction(action,to:item.target,from:item) else { throw UserFacingError(message:"Native menu action did not execute") }
        }
        mount(); window.makeKeyAndOrderFront(nil); defer { window.close() }
        try await settle()
        let table = try native()
        guard let coordinator = table.dataSource as? NativeTransferGrid.Coordinator,
              let row = coordinator.rows.firstIndex(where:{ !$0.isPaused }) else { throw UserFacingError(message:"Transfer action fixture has no running row") }
        let target = coordinator.rows[row].id
        table.selectRowIndexes(IndexSet(integer:row),byExtendingSelection:false); try await settle()
        guard store.selection == [target], let menu = table.menuProvider?([target]), let pause = menu.item(withTitle:"Pause") else { throw UserFacingError(message:"Native selection or context menu did not capture the row") }
        store.selection = [coordinator.rows.first(where:{$0.id != target})!.id]
        try invoke(pause)
        try await waitUntil("Context pause did not retain its captured torrent ID") { UXSelfTest.nativeRequests.contains { $0.command == "pause" && $0.id == target && $0.paused == true } }
        try await waitUntil("Context pause did not settle") { store.busyTransfers.isEmpty && store.torrents.first(where:{$0.id == target})?.isPaused == true }
        try await settle()
        guard let activate = table.doubleAction else { throw UserFacingError(message:"Native primary action unavailable") }
        NSApp.sendAction(activate,to:table.target,from:table)
        try await waitUntil("Primary activation did not resume the captured row") { UXSelfTest.nativeRequests.contains { $0.command == "pause" && $0.id == target && $0.paused == false } }
        try await settle()
        let beforeReturn = UXSelfTest.nativeRequests.filter { $0.command == "pause" && $0.id == target }.count
        guard let event = NSEvent.keyEvent(with:.keyDown,location:.zero,modifierFlags:.capsLock,timestamp:ProcessInfo.processInfo.systemUptime,windowNumber:window.windowNumber,context:nil,characters:"\r",charactersIgnoringModifiers:"\r",isARepeat:false,keyCode:36) else { throw UserFacingError(message:"Could not construct controlled Return event") }
        table.keyDown(with:event)
        try await waitUntil("Controlled Return with Caps Lock did not activate the selected transfer") { UXSelfTest.nativeRequests.filter { $0.command == "pause" && $0.id == target }.count > beforeReturn }
        try await settle()
        table.sortDescriptors = [NSSortDescriptor(key:"name",ascending:false)]; try await settle()
        let expected = store.visibleTorrents.sorted(using:[KeyPathComparator(\TorrentItem.name,order:.reverse),KeyPathComparator(\TorrentItem.id)])
        guard coordinator.rows.map(\.id) == expected.map(\.id), store.selection == [target],
              table.selectedRowIndexes.compactMap({ coordinator.rows.indices.contains($0) ? coordinator.rows[$0].id : nil }) == [target] else { throw UserFacingError(message:"Native sorting lost row order or selection identity") }
        guard let header = table.headerView?.menu else { throw UserFacingError(message:"Native column menu unavailable") }
        header.delegate?.menuNeedsUpdate?(header)
        guard let eta = header.item(withTitle:"ETA"), let name = header.item(withTitle:"Name"), !name.isEnabled else { throw UserFacingError(message:"Column menu allows hiding Name") }
        let wasHidden = table.tableColumns.first(where:{$0.identifier.rawValue == "eta"})!.isHidden
        try invoke(eta); try await settle()
        guard table.tableColumns.first(where:{$0.identifier.rawValue == "eta"})!.isHidden != wasHidden else { throw UserFacingError(message:"Native column menu did not update visibility") }
        table.moveColumn(0,toColumn:2); try await settle()
        let order = table.tableColumns.map { $0.identifier.rawValue }
        window.contentView = nil; mount(); try await settle()
        guard try native().tableColumns.map({ $0.identifier.rawValue }) == order else { throw UserFacingError(message:"Native column order did not persist across remount") }
        var legacy = TableColumnCustomization<TorrentItem>(); legacy[visibility:"destination"] = .hidden
        var object = try JSONSerialization.jsonObject(with:JSONEncoder().encode(legacy)) as! [String:Any]
        object["columnOrder"] = ["status","name","progress"].map { ["base":["explicit":["_0":$0]]] }
        let oldData = try JSONSerialization.data(withJSONObject:object,options:.sortedKeys)
        _ = try JSONDecoder().decode(TableColumnCustomization<TorrentItem>.self,from:oldData)
        defaults.set(oldData,forKey:"transferColumns"); defaults.removeObject(forKey:TransferColumns.orderKey)
        window.contentView = nil; mount(); try await settle()
        let migrated = try native()
        guard Array(migrated.tableColumns.prefix(3).map { $0.identifier.rawValue }) == ["status","name","progress"],
              migrated.tableColumns.first(where:{$0.identifier.rawValue == "destination"})?.isHidden == true,
              defaults.data(forKey:"transferColumns") == oldData else { throw UserFacingError(message:"Native migration changed legacy customization or lost order/visibility") }
        WorkspaceTablePersistence.restoreWidths(); try await settle()
        guard migrated.tableColumns.map({ $0.identifier.rawValue }) == TransferColumns.all.map(\.id) else { throw UserFacingError(message:"Restore Default Columns did not reset native order") }
        print("PASS: Native transfer selection, captured context actions, primary activation, sorting, visibility, order persistence, legacy preference preservation, and reset.")
    }

    private static func verifyFilesOutline() async throws {
        let store = UXSelfTest.nativeFixture(), item = store.torrents[0]
        let session = TorrentFilesSession(files:item.files)
        let window = NSWindow(contentRect:NSRect(x:0,y:0,width:780,height:520),styleMask:[.titled],backing:.buffered,defer:false)
        window.isReleasedWhenClosed = false; window.title = "Files inspector · native fixture"
        window.contentView = NSHostingView(rootView:FilesInterfaceFixture(store:store,session:session))
        window.makeKeyAndOrderFront(nil)
        defer { window.close() }
        try await settle()
        guard let outline = descendants(window.contentView).compactMap({ $0 as? NSOutlineView }).first else {
            throw UserFacingError(message:"Files inspector did not create a native outline")
        }
        guard let folder = session.tree.nodes["folder:Artwork"], let file = session.tree.nodes["file:0"] else {
            throw UserFacingError(message:"Files outline fixture nodes unavailable")
        }
        guard let viewport = outline.enclosingScrollView else { throw UserFacingError(message:"Files viewport unavailable") }
        let originalViewport = viewport.convert(viewport.bounds,to:nil)
        func expectStableViewport() throws {
            let current = viewport.convert(viewport.bounds,to:nil)
            guard abs(current.minY-originalViewport.minY) < 0.5, abs(current.height-originalViewport.height) < 0.5 else {
                throw UserFacingError(message:"File selection or status shifted the Files viewport")
            }
        }
        let initialRow = outline.row(forItem:file)
        guard let initialCell = outline.view(atColumn:0,row:initialRow,makeIfNecessary:true),
              let priorityColumn = outline.tableColumns.firstIndex(where:{ $0.identifier.rawValue == "priority" }),
              let priorityCell = outline.view(atColumn:priorityColumn,row:initialRow,makeIfNecessary:true),
              let popup = priorityCell.subviews.compactMap({ $0 as? NSPopUpButton }).first,
              let initialCheck = initialCell.subviews.compactMap({ $0 as? NSButton }).first(where:{ !($0 is NSPopUpButton) }),
              let name = initialCell.subviews.compactMap({ $0 as? NSTextField }).first(where:{ $0.stringValue == file.name }),
              let icon = initialCell.subviews.compactMap({ $0 as? NSImageView }).first,
              popup.isEnabled, session.selection.isEmpty,
              abs(initialCheck.frame.midY-name.frame.midY) < 0.5, abs(icon.frame.midY-name.frame.midY) < 0.5,
              outline.rowHeight == 28 else { throw UserFacingError(message:"Compact row controls are misaligned or priority requires row selection") }
        popup.selectItem(withTag:2)
        NSApp.sendAction(popup.action!,to:popup.target,from:popup); try await settle()
        guard UXSelfTest.nativeRequests.last(where:{ $0.command == "priorities" })?.priorities == [2,1,1,1], session.selection.isEmpty else {
            throw UserFacingError(message:"Row priority changed selection or targeted the wrong file")
        }
        try expectStableViewport()
        outline.expandItem(folder); try await settle()
        guard session.expanded.contains(folder.key), outline.numberOfRows == 5 else {
            throw UserFacingError(message:"Native expansion did not expose the folder's file")
        }
        let row = outline.row(forItem:file)
        outline.selectRowIndexes(IndexSet(integer:row),byExtendingSelection:false)
        window.makeFirstResponder(outline); try await settle()
        guard session.selection == [file.key], window.firstResponder === outline else {
            throw UserFacingError(message:"Native file selection did not retain keyboard focus")
        }
        try expectStableViewport()
        if let menu = outline.menu {
            menu.delegate?.menuNeedsUpdate?(menu)
            guard let rename = menu.items.firstIndex(where:{ $0.title == "Rename…" }) else { throw UserFacingError(message:"File context menu omitted Rename") }
            menu.performActionForItem(at:rename); try await settle()
            guard let sheet = window.attachedSheet,
                  let editor = sheet.firstResponder as? NSTextView, editor.isFieldEditor,
                  let input = editor.delegate as? NSTextField, input.stringValue == file.name,
                  let cancel = descendants(sheet.contentView).compactMap({ $0 as? NSButton }).first(where:{ $0.title == "Cancel" }) else {
                throw UserFacingError(message:"File outline Rename did not capture its selected file")
            }
            cancel.performClick(nil); try await settle()
            guard window.firstResponder === outline else { throw UserFacingError(message:"Rename did not restore outline focus") }
        }
        guard let space = NSEvent.keyEvent(with:.keyDown,location:.zero,modifierFlags:[],timestamp:0,windowNumber:window.windowNumber,context:nil,characters:" ",charactersIgnoringModifiers:" ",isARepeat:false,keyCode:49) else {
            throw UserFacingError(message:"Could not construct controlled Space event")
        }
        outline.keyDown(with:space); try await settle()
        guard let request = UXSelfTest.nativeRequests.last(where:{ $0.command == "select" }),
              request.id == item.id, request.selected == [1,2,3], session.selection == [file.key] else {
            throw UserFacingError(message:"Space changed row selection or submitted wrong download targets")
        }
        guard let cell = outline.view(atColumn:0,row:row,makeIfNecessary:true),
              let checkbox = descendants(cell).compactMap({ $0 as? NSButton }).first else {
            throw UserFacingError(message:"Native file checkbox unavailable")
        }
        checkbox.performClick(nil); try await settle()
        guard UXSelfTest.nativeRequests.last(where:{ $0.command == "select" })?.selected == [0,1,2,3], session.selection == [file.key] else {
            throw UserFacingError(message:"Native checkbox changed selection or submitted wrong inclusion")
        }
        let tree = session.tree
        let priorityMenuItem = popup.itemArray.first
        store.torrents[0].files[0].downloaded += 100; try await settle()
        guard session.tree === tree, outline.isItemExpanded(folder), window.firstResponder === outline,
              popup.itemArray.first === priorityMenuItem else {
            throw UserFacingError(message:"Progress update rebuilt the outline or lost expansion/focus")
        }
        session.query = "Poster"; try await settle()
        try expectStableViewport()
        guard outline.numberOfRows == 2, session.selection.isEmpty else {
            throw UserFacingError(message:"File search failed to retain ancestors or clear hidden selection")
        }
        session.query = ""; try await settle()
        guard outline.isItemExpanded(folder) else { throw UserFacingError(message:"Clearing search lost saved folder expansion") }
        store.torrents[0].files = store.torrents[0].files.map { file in var value = file; value.selected = file.index == 0; return value }
        try await settle()
        let finalRow = outline.row(forItem:file)
        guard let cell = outline.view(atColumn:0,row:finalRow,makeIfNecessary:true),
              let checkbox = descendants(cell).compactMap({ $0 as? NSButton }).first,
              !checkbox.isEnabled,
              descendants(cell).compactMap({ $0 as? NSTextField }).contains(where:{ $0.stringValue == file.name && $0.isEnabled }) else {
            throw UserFacingError(message:"Last included file did not retain readable text with a disabled checkbox")
        }
        print("PASS: Native file outline expands, selects, handles controlled Space and checkbox clicks, filters ancestors, and preserves focus across progress updates.")
        print("PASS: File context-menu Rename captures the selected filename and Cancel restores outline focus.")
        print("PASS: Last included file disables only its checkbox; the filename remains enabled.")
        outline.selectRowIndexes(IndexSet(integer:finalRow),byExtendingSelection:false); try await settle()
        try expectStableViewport()
        store.fileOperation = FileOperationItem(id:"files-layout",torrentId:item.id,kind:"rename",phase:"copying",completedBytes:1,totalBytes:100,cancelRequested:false)
        try await settle(); try expectStableViewport()
        store.fileOperation = nil; try await settle(); try expectStableViewport()
        print("PASS: 28-point rows align checkbox, icon, and filename; row priority works without selection.")
        print("PASS: Selection, search, last-file restriction, and operation status preserve the Files viewport frame.")
        for count in [1_000,10_000] {
            let files = PerformanceSelfTest.fixture(1,files:count).torrents[0].files
            let started = ProcessInfo.processInfo.systemUptime
            store.torrents[0].files = files; try await settle()
            if let folder = session.tree.roots.first { outline.expandItem(folder) }
            try await settle()
            let identity = session.tree
            store.torrents[0].files[0].downloaded += 1; try await settle()
            guard session.tree === identity, outline.numberOfRows == count + 1 else {
                throw UserFacingError(message:"Large file outline lost hierarchy or rebuilt it for byte updates")
            }
            let cells = descendants(outline).filter { $0 is NSTableCellView }.count
            guard cells < 100 else { throw UserFacingError(message:"Large file outline did not virtualize native cells") }
            let elapsed = Int((ProcessInfo.processInfo.systemUptime-started)*1000)
            guard elapsed < 5_000 else { throw UserFacingError(message:"\(count)-file outline exceeded the controlled 5 s budget: \(elapsed) ms including layout waits") }
            print("PASS: \(count)-file native outline, \(cells) materialized cells, \(elapsed) ms including three 400 ms layout waits.")
        }
    }

    private static func verifyControlledSheets(store: LibraryStore, window: NSWindow) async throws {
        window.subtitle = "Native test fixture · sample data"
        window.makeKeyAndOrderFront(nil); NSApp.activate(ignoringOtherApps:true)
        try await settle()
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("native-import-"+UUID().uuidString+".torrent")
        try Data([1,2,3]).write(to:file)
        defer { try? FileManager.default.removeItem(at:file) }
        store.beginImport(file:file)
        try await waitUntil("Metadata table did not appear; requests: \(UXSelfTest.nativeRequests.map(\.command))") {
            window.attachedSheet.map { descendants($0.contentView).contains { $0 is NSTableView } } ?? false
        }
        try await waitUntil("Metadata table did not receive native focus") { window.attachedSheet?.firstResponder is NSTableView }
        guard let sheet = window.attachedSheet else { throw UserFacingError(message:"Metadata fixture did not open an import sheet") }
        guard let focused = sheet.firstResponder as? NSTableView,
              descendants(sheet.contentView).contains(where:{ $0 === focused }) else {
            throw UserFacingError(message:"Metadata did not focus its native file table: \(String(describing:sheet.firstResponder))")
        }
        print("PASS: Resolved metadata moves native focus to file selection.")
        store.showImport = false; try await settle()
        guard let table = descendants(window.contentView).compactMap({ $0 as? NSTableView }).first else {
            throw UserFacingError(message:"Controlled transfer table unavailable")
        }
        window.makeFirstResponder(table)
        let item = store.torrents[0]
        store.requestRename(item,file:item.files[0]); try await settle()
        guard let rename = window.attachedSheet,
              let editor = rename.firstResponder as? NSTextView, editor.isFieldEditor,
              let input = editor.delegate as? NSTextField,
              input.stringValue == URL(fileURLWithPath:item.files[0].name).lastPathComponent else {
            throw UserFacingError(message:"Rename did not focus its captured file name")
        }
        guard let cancel = descendants(rename.contentView).compactMap({ $0 as? NSButton }).first(where:{ $0.title == "Cancel" }) else {
            throw UserFacingError(message:"Native rename Cancel button unavailable")
        }
        cancel.performClick(nil); try await settle()
        guard window.attachedSheet == nil, window.firstResponder === table else {
            throw UserFacingError(message:"Rename cancellation did not restore original native focus")
        }
        print("PASS: Rename focuses its captured name; native Cancel restores transfer-table focus.")
        var announcements: [String] = []
        var priorities: [NSAccessibilityPriorityLevel] = []
        NativeAccessibility.announcementObserver = { message, priority in announcements.append(message); priorities.append(priority) }
        defer { NativeAccessibility.announcementObserver = nil }
        window.makeKeyAndOrderFront(nil); NSApp.activate(ignoringOtherApps:true)
        try await waitUntil("Announcement fixture did not become active and key") { NSApp.isActive && NSApp.keyWindow === window }
        store.error = "Restore access to the destination."; try await settle()
        store.error = "Restore access to the destination."; try await settle()
        guard announcements == ["Something needs attention. Restore access to the destination."] else {
            throw UserFacingError(message:"Native error announcement was missing or duplicated: \(announcements)")
        }
        store.error = nil; try await settle()
        store.error = "Restore access to the destination."; try await settle()
        guard announcements.count == 2 else { throw UserFacingError(message:"Repeated error after clearing was not announced") }
        guard priorities == [.high,.high] else { throw UserFacingError(message:"Errors did not use high-priority announcements") }
        print("PASS: Native notice requests one high-priority announcement per error occurrence; speech output remains manual.")
        store.error = nil; try await settle()
        store.fileOperation = FileOperationItem(id:"native-move",torrentId:item.id,kind:"relocate",phase:"copying",completedBytes:1,totalBytes:100,cancelRequested:false)
        try await settle()
        let copyingCount = announcements.count
        store.fileOperation?.completedBytes = 50; try await settle()
        guard copyingCount == 3, announcements.count == copyingCount,
              announcements.last == "File change. Moving files…", priorities.last == .medium else {
            throw UserFacingError(message:"File copying did not announce its phase once or announced byte updates")
        }
        store.fileOperation?.cancelRequested = true; try await settle()
        guard announcements.count == 4, announcements.last == "File change. Cancelling…", priorities.last == .medium else {
            throw UserFacingError(message:"Cancellation phase was not announced")
        }
        store.fileOperation = nil
        store.storageRecovery = StorageRecoveryItem(id:"native-recovery",torrentId:item.id,activeLocation:"original",message:"Inspect retained copies.",originalDirectory:"/tmp/original",destinationDirectory:"/tmp/copies",canKeepCopies:true)
        try await settle()
        guard announcements.count == 5, announcements.last == "File cleanup needs attention. Inspect retained copies.", priorities.last == .high else {
            throw UserFacingError(message:"Recovery guidance was not announced")
        }
        print("PASS: File phases announce at medium priority without byte chatter; recovery guidance requests high priority.")
    }
    private static func waitUntil(_ failure: @autoclosure () -> String, _ condition: () -> Bool) async throws {
        let deadline = Date(timeIntervalSinceNow:5)
        while !condition() {
            guard Date() < deadline else { throw UserFacingError(message:failure()) }
            try await Task.sleep(for:.milliseconds(50))
        }
    }

    private static func descendants(_ view: NSView?) -> [NSView] {
        guard let view else { return [] }
        return [view] + view.subviews.flatMap { descendants($0) }
    }

    static func invoke(_ title: String, key: String) async throws {
        func find(in menu: NSMenu) -> (NSMenu, Int)? {
            menu.delegate?.menuNeedsUpdate?(menu)
            menu.update()
            for (index,item) in menu.items.enumerated() {
                if item.title == title { return (menu,index) }
                if let submenu = item.submenu, let found = find(in:submenu) { return found }
            }
            return nil
        }
        guard let root = NSApp.mainMenu, let (menu,index) = find(in:root) else {
            throw UserFacingError(message:"Missing native menu action: \(title)")
        }
        menu.delegate?.menuWillOpen?(menu)
        defer { menu.delegate?.menuDidClose?(menu) }
        try await settle()
        menu.update()
        let item = menu.items[index]
        guard item.isEnabled, item.keyEquivalent == key, item.keyEquivalentModifierMask == .command else {
            throw UserFacingError(message:"Menu action disabled or shortcut incorrect: \(title), enabled=\(item.isEnabled), key=\(item.keyEquivalent), modifiers=\(item.keyEquivalentModifierMask.rawValue), active=\(NSApp.isActive), keyWindow=\(NSApp.keyWindow?.title ?? "none")")
        }
        menu.performActionForItem(at:index)
    }
    private static func settle() async throws { try await Task.sleep(for:.milliseconds(400)) }

    private static func capture(_ window: NSWindow, to url: URL) throws {
        guard let view = window.contentView?.superview,
              let bitmap = view.bitmapImageRepForCachingDisplay(in:view.bounds) else {
            throw UserFacingError(message:"Could not render the native navigation fixture")
        }
        view.cacheDisplay(in:view.bounds,to:bitmap)
        guard let png = bitmap.representation(using:.png,properties:[:]) else {
            throw UserFacingError(message:"Could not encode the native navigation fixture")
        }
        try png.write(to:url)
    }
}

private struct FilesInterfaceFixture: View {
    @ObservedObject var store: LibraryStore
    @ObservedObject var session: TorrentFilesSession
    var body: some View {
        if let item = store.torrents.first { TorrentFilesView(torrent:item,session:session).environmentObject(store) }
    }
}
#endif
