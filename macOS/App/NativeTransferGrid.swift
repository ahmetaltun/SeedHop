import SwiftUI
import AppKit

/// Stable app IDs own native order. Keep the original Codable customization
/// intact for visibility and rollback to the previous interface.
@MainActor enum TransferColumns {
    struct Column {
        let id: String
        let title: String
        let width: CGFloat
        let minimum: CGFloat
        let sort: String
        var optional = false
    }
    static var all: [Column] { [
        .init(id:"name",title:String(localized:"Name",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),width:240,minimum:180,sort:"name"),
        .init(id:"status",title:String(localized:"Status",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),width:125,minimum:110,sort:"status"),
        .init(id:"progress",title:String(localized:"Progress",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),width:90,minimum:85,sort:"progress"),
        .init(id:"download",title:String(localized:"↓ Download",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),width:80,minimum:70,sort:"download"),
        .init(id:"upload",title:String(localized:"↑ Upload",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),width:80,minimum:70,sort:"upload"),
        .init(id:"eta",title:String(localized:"ETA",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),width:75,minimum:60,sort:"eta"),
        .init(id:"ratio",title:String(localized:"Ratio",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),width:55,minimum:45,sort:"ratio"),
        .init(id:"peers",title:String(localized:"Peers",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),width:50,minimum:45,sort:"peers"),
        .init(id:"queue",title:String(localized:"Queue",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),width:55,minimum:45,sort:"queue"),
        .init(id:"selectedSize",title:String(localized:"Selected Size",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),width:90,minimum:70,sort:"selected",optional:true),
        .init(id:"size",title:String(localized:"Total Size",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),width:90,minimum:70,sort:"size",optional:true),
        .init(id:"remaining",title:String(localized:"Remaining",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),width:90,minimum:70,sort:"remaining",optional:true),
        .init(id:"downloadedTraffic",title:String(localized:"Downloaded Traffic",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),width:115,minimum:90,sort:"downloaded",optional:true),
        .init(id:"uploadedTraffic",title:String(localized:"Uploaded Traffic",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),width:110,minimum:85,sort:"uploaded",optional:true),
        .init(id:"added",title:String(localized:"Added",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),width:100,minimum:80,sort:"added",optional:true),
        .init(id:"completed",title:String(localized:"Completed",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),width:100,minimum:80,sort:"completed",optional:true),
        .init(id:"destination",title:String(localized:"Save Path",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),width:220,minimum:140,sort:"path",optional:true)
    ] }
    static let orderKey = "workspaceNativeColumnOrder"
    // English headings are legacy persistence keys, never display strings.
    static let legacyTitles = ["name":"Name", "status":"Status", "progress":"Progress", "download":"↓ Download", "upload":"↑ Upload", "eta":"ETA", "ratio":"Ratio", "peers":"Peers", "queue":"Queue", "selectedSize":"Selected Size", "size":"Total Size", "remaining":"Remaining", "downloadedTraffic":"Downloaded Traffic", "uploadedTraffic":"Uploaded Traffic", "added":"Added", "completed":"Completed", "destination":"Save Path"]
    static func initialWidth(_ column: Column) -> CGFloat {
        max(column.width,ceil((column.title as NSString).size(withAttributes:[.font:NSFont.systemFont(ofSize:11)]).width) + 26)
    }
    static func order(_ customization: TableColumnCustomization<TorrentItem>) -> [String] {
        let canonical = all.map(\.id)
        var saved = WorkspacePreferences.defaults.stringArray(forKey:orderKey)
        if saved == nil, let data = try? JSONEncoder().encode(customization),
           let object = try? JSONSerialization.jsonObject(with:data) as? [String:Any],
           let legacy = object["columnOrder"] as? [[String:Any]] {
            // Read recognized existing Codable data without changing it. Unknown
            // layouts remain intact; missing columns retain canonical order.
            saved = legacy.compactMap { (($0["base"] as? [String:Any])?["explicit"] as? [String:Any])?["_0"] as? String }
        }
        var result: [String] = []
        for id in (saved ?? []) + canonical where canonical.contains(id) && !result.contains(id) { result.append(id) }
        if WorkspacePreferences.defaults.stringArray(forKey:orderKey) == nil {
            WorkspacePreferences.defaults.set(result,forKey:orderKey)
        }
        return result
    }
    static func visible(_ column: Column, in customization: TableColumnCustomization<TorrentItem>) -> Bool {
        if column.id == "name" { return true }
        switch customization[visibility:column.id] {
        case .visible: return true
        case .hidden: return false
        default: return !column.optional
        }
    }
}

struct NativeTransferGrid: NSViewRepresentable {
    private let language = AppLocalization.shared.language
    @ObservedObject var store: LibraryStore
    let rows: [TorrentItem]
    @Binding var sortOrder: [KeyPathComparator<TorrentItem>]
    @Binding var customization: TableColumnCustomization<TorrentItem>
    let rowHeight: CGFloat
    var focus: FocusState<Bool>.Binding
    func makeCoordinator() -> Coordinator { Coordinator(self) }
    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSScrollView()
        scroll.hasVerticalScroller = true; scroll.hasHorizontalScroller = true; scroll.autohidesScrollers = true; scroll.drawsBackground = false
        let table = NativeTransfersTable()
        table.identifier = NSUserInterfaceItemIdentifier("transfers-table")
        table.style = .inset; table.usesAlternatingRowBackgroundColors = true
        table.usesAutomaticRowHeights = false; table.rowSizeStyle = .custom; table.intercellSpacing = NSSize(width:0,height:0)
        table.columnAutoresizingStyle = .noColumnAutoresizing; table.allowsMultipleSelection = true; table.allowsEmptySelection = true
        table.allowsColumnReordering = true; table.setAccessibilityLabel(String(localized:"Transfers",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
        table.delegate = context.coordinator; table.dataSource = context.coordinator
        table.target = context.coordinator; table.doubleAction = #selector(Coordinator.activate)
        table.menuProvider = { [weak coordinator = context.coordinator] in coordinator?.contextMenu($0) }
        table.focusChanged = { [weak coordinator = context.coordinator, weak table] focused in
            DispatchQueue.main.async { [weak coordinator, weak table] in
                guard let coordinator, let table, let window = table.window else { return }
                let actual = window.firstResponder === table
                if actual == focused, coordinator.parent.focus.wrappedValue != actual { coordinator.parent.focus.wrappedValue = actual }
            }
        }
        table.restoreColumnOrder = { [weak coordinator = context.coordinator, weak table] in
            guard let coordinator, let table else { return }
            WorkspacePreferences.defaults.set(TransferColumns.all.map(\.id),forKey:TransferColumns.orderKey)
            coordinator.applyOrder(TransferColumns.all.map(\.id),to:table)
        }
        for spec in TransferColumns.all {
            let column = NSTableColumn(identifier:NSUserInterfaceItemIdentifier(spec.id))
            column.title = spec.title; column.minWidth = spec.minimum; column.maxWidth = 20_000; column.width = TransferColumns.initialWidth(spec)
            column.resizingMask = .userResizingMask
            column.sortDescriptorPrototype = NSSortDescriptor(key:spec.sort,ascending:true)
            table.addTableColumn(column)
        }
        let headerMenu = NSMenu(); headerMenu.delegate = context.coordinator; headerMenu.autoenablesItems = false
        table.headerView?.menu = headerMenu
        scroll.documentView = table; context.coordinator.table = table
        context.coordinator.update(self)
        WorkspaceTablePersistence.install(table)
        return scroll
    }
    func updateNSView(_ scroll: NSScrollView, context: Context) { context.coordinator.update(self) }

    @MainActor final class Coordinator: NSObject, NSTableViewDataSource, NSTableViewDelegate, NSMenuDelegate {
        var parent: NativeTransferGrid
        weak var table: NativeTransfersTable?
        var rows: [TorrentItem] = []
        private var updating = false
        private var lastConnected: Bool?
        private var lastOperation: FileOperationItem?
        private var lastRecovery: StorageRecoveryItem?
        private var lastLanguage: String?
        private var requestedFocus = false
        init(_ parent: NativeTransferGrid) { self.parent = parent }
        func update(_ value: NativeTransferGrid) {
            parent = value
            guard let table else { return }
            updating = true; defer { updating = false }
            let changedIDs = rows.map(\.id) != value.rows.map(\.id)
            let changed = rows != value.rows || lastConnected != value.store.connected || lastOperation != value.store.fileOperation || lastRecovery != value.store.storageRecovery || lastLanguage != value.language
            lastLanguage = value.language
            table.setAccessibilityLabel(String(localized:"Transfers",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
            rows = value.rows; lastConnected = value.store.connected; lastOperation = value.store.fileOperation; lastRecovery = value.store.storageRecovery
            if table.rowHeight != value.rowHeight { table.rowHeight = value.rowHeight }
            for column in table.tableColumns {
                if let spec = TransferColumns.all.first(where:{$0.id == column.identifier.rawValue}) {
                    if column.title != spec.title { column.title = spec.title }
                    let hidden = !TransferColumns.visible(spec,in:value.customization)
                    if column.isHidden != hidden { column.isHidden = hidden }
                }
            }
            applyOrder(TransferColumns.order(value.customization),to:table)
            let descriptors = value.sortOrder.compactMap { sort -> NSSortDescriptor? in
                guard let field = LibraryView.sortFields.first(where:{$0.1 == sort.keyPath}) else { return nil }
                return NSSortDescriptor(key:field.0,ascending:sort.order == .forward)
            }
            if table.sortDescriptors != descriptors { table.sortDescriptors = descriptors }
            if changedIDs { table.reloadData() }
            else if changed {
                // AppKit can keep materialized rows outside the viewport. Update
                // those too so scrolling cannot reveal old rates or error labels.
                var available = IndexSet()
                table.enumerateAvailableRowViews { _, row in
                    if rows.indices.contains(row) { available.insert(row) }
                }
                if !available.isEmpty { table.reloadData(forRowIndexes:available,columnIndexes:IndexSet(integersIn:0..<table.numberOfColumns)) }
            }
            let selection = IndexSet(rows.indices.filter { value.store.selection.contains(rows[$0].id) })
            if table.selectedRowIndexes != selection { table.selectRowIndexes(selection,byExtendingSelection:false) }
            if value.focus.wrappedValue && !requestedFocus {
                DispatchQueue.main.async { [weak table] in if let table { table.window?.makeFirstResponder(table) } }
            }
            requestedFocus = value.focus.wrappedValue
        }
        func applyOrder(_ ids: [String], to table: NSTableView) {
            let wasUpdating = updating; updating = true; defer { updating = wasUpdating }
            for (destination,id) in ids.enumerated() {
                if let index = table.tableColumns.firstIndex(where:{$0.identifier.rawValue == id}), index != destination { table.moveColumn(index,toColumn:destination) }
            }
        }
        func numberOfRows(in tableView: NSTableView) -> Int { rows.count }
        func tableViewSelectionDidChange(_ notification: Notification) {
            guard !updating, let table else { return }
            let ids = Set(table.selectedRowIndexes.compactMap { rows.indices.contains($0) ? rows[$0].id : nil })
            if parent.store.selection != ids { parent.store.selection = ids }
        }
        func tableView(_ tableView: NSTableView, sortDescriptorsDidChange oldDescriptors: [NSSortDescriptor]) {
            guard !updating else { return }
            let order = tableView.sortDescriptors.compactMap { descriptor -> KeyPathComparator<TorrentItem>? in
                guard let key = descriptor.key else { return nil }
                return LibraryView.comparator(field:key,order:descriptor.ascending ? .forward : .reverse)
            }
            if parent.sortOrder != order { parent.sortOrder = order }
        }
        func tableViewColumnDidMove(_ notification: Notification) {
            guard !updating, let table else { return }
            WorkspacePreferences.defaults.set(table.tableColumns.map { $0.identifier.rawValue },forKey:TransferColumns.orderKey)
        }
        func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
            guard rows.indices.contains(row), let column = tableColumn else { return nil }
            let id = column.identifier.rawValue, item = rows[row], status = parent.store.presentation(item)
            if id == "progress" {
                let cell = (tableView.makeView(withIdentifier:column.identifier,owner:self) as? NativeProgressCell) ?? NativeProgressCell(frame:.zero)
                cell.identifier = column.identifier; cell.configure(status); return cell
            }
            let cell = (tableView.makeView(withIdentifier:column.identifier,owner:self) as? NativeTransferCell) ?? NativeTransferCell(frame:.zero)
            cell.identifier = column.identifier
            cell.label.font = id == "name" ? .systemFont(ofSize:12,weight:.medium) : id == "status" ? .systemFont(ofSize:11,weight:.medium) : .monospacedDigitSystemFont(ofSize:11,weight:.regular)
            cell.label.lineBreakMode = (id == "name" || id == "destination") ? .byTruncatingMiddle : .byTruncatingTail
            cell.secondary = ["download","upload","peers"].contains(id)
            cell.statusTint = id == "status" ? NSColor(Design.stateColor(status.colorState)) : nil
            var text = "—", help: String?
            switch id {
            case "name": text = item.name; help = item.name
            case "status": text = status.label; help = status.detail
            case "download": text = SizeText.rate(parent.store.connected ? item.downloadRate : 0)
            case "upload": text = SizeText.rate(parent.store.connected ? item.uploadRate : 0)
            case "peers": text = String(item.peers)
            case "eta": text = item.etaSeconds.map(Self.duration) ?? "—"; help = String(localized:"Estimated time for selected files; unavailable while paused, checking, or without an estimate",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
            case "ratio": text = item.shareRatio.map { $0.formatted(.number.precision(.fractionLength(2))) } ?? "—"; help = String(localized:"Lifetime uploaded payload divided by downloaded payload. Unknown history and zero downloads have no ratio.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
            case "queue": text = item.state == "queued" ? String(item.queuePosition) : "—"
            case "selectedSize": text = SizeText.bytes(item.selectedBytes ?? item.totalBytes)
            case "size": text = SizeText.bytes(item.fullBytes)
            case "remaining": text = SizeText.bytes(item.remainingBytes)
            case "downloadedTraffic": text = item.networkDownloadedBytes.map(SizeText.bytes) ?? "—"
            case "uploadedTraffic": text = item.networkUploadedBytes.map(SizeText.bytes) ?? "—"
            case "added": text = item.addedAt.map(Self.date) ?? "—"
            case "completed": text = item.completedAt.map(Self.date) ?? "—"
            case "destination": text = item.destination; help = item.destination
            default: break
            }
            cell.label.stringValue = text; cell.toolTip = help
            cell.label.setAccessibilityLabel(id == "name" ? String(localized:"\(item.name), \(status.accessibleLabel)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : id == "status" ? status.accessibleLabel : text)
            cell.updateColor(); return cell
        }
        private static func duration(_ value: UInt64) -> String { LocaleText.duration(value) }
        private static func date(_ value: UInt64) -> String { Date(timeIntervalSince1970:Double(value)).formatted(date:.abbreviated,time:.omitted) }
        @objc func activate() {
            guard let table, !table.selectedRowIndexes.isEmpty else { return }
            let ids = Set(table.selectedRowIndexes.compactMap { rows.indices.contains($0) ? rows[$0].id : nil })
            guard canAct(ids) else { return }
            parent.store.selection = ids; parent.store.toggleSelected(paused:parent.store.canPauseSelection)
        }
        private func canAct(_ ids: Set<String>) -> Bool {
            parent.store.canChangeTransfers && !ids.isEmpty && ids.isSubset(of:Set(parent.store.torrents.map(\.id))) && parent.store.busyTransfers.isDisjoint(with:ids)
        }
        func contextMenu(_ ids: Set<String>) -> NSMenu {
            let menu = NSMenu(); menu.autoenablesItems = false
            let items = parent.store.torrents.filter { ids.contains($0.id) }, allowed = canAct(ids)
            func add(_ title: String, _ action: String, _ enabled: Bool) {
                let item = NSMenuItem(title:title,action:#selector(menuAction(_:)),keyEquivalent:"")
                item.target = self; item.representedObject = ["action":action,"ids":Array(ids)] as [String:Any]; item.isEnabled = enabled; menu.addItem(item)
            }
            add(String(localized:"Move to Front of Queue",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),"queue",allowed && ids.count == 1)
            add(String(localized:"Move Files…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),"move",allowed && parent.store.storageRecovery == nil && ids.count == 1 && !items.contains { $0.state == "checking" })
            add(String(localized:"Force Recheck",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),"recheck",allowed && !items.contains { $0.state == "checking" || parent.store.hasPendingCleanup($0.id) })
            add(parent.store.resumeLabel(for:items),"resume",allowed && items.contains { parent.store.canResumeOrRetry($0) })
            add(String(localized:"Pause",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),"pause",allowed && items.contains { !$0.isPaused })
            menu.addItem(.separator()); add(String(localized:"Show in Finder",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),"finder",!ids.isEmpty)
            add(String(localized:"Remove…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),"remove",allowed && !ids.contains(where:parent.store.hasPendingCleanup))
            return menu
        }
        @objc private func menuAction(_ item: NSMenuItem) {
            guard let value = item.representedObject as? [String:Any], let ids = value["ids"] as? [String], let action = value["action"] as? String else { return }
            let targets = Set(ids)
            if action == "finder" {
                if let target = parent.store.torrents.first(where:{ targets.contains($0.id) }) { NSWorkspace.shared.selectFile(nil,inFileViewerRootedAtPath:target.destination) }; return
            }
            guard canAct(targets) else { return }
            parent.store.selection = targets
            switch action {
            case "queue": parent.store.moveSelectedToFront()
            case "move": parent.store.chooseRelocation()
            case "recheck": parent.store.recheckSelected()
            case "resume": parent.store.toggleSelected(paused:false)
            case "pause": parent.store.toggleSelected(paused:true)
            case "remove": parent.store.requestRemoval()
            default: break
            }
        }
        func menuNeedsUpdate(_ menu: NSMenu) {
            menu.removeAllItems()
            for spec in TransferColumns.all {
                let item = NSMenuItem(title:spec.title,action:#selector(toggleColumn(_:)),keyEquivalent:"")
                item.target = self; item.representedObject = spec.id; item.isEnabled = spec.id != "name"
                item.state = TransferColumns.visible(spec,in:parent.customization) ? .on : .off; menu.addItem(item)
            }
        }
        @objc private func toggleColumn(_ item: NSMenuItem) {
            guard let id = item.representedObject as? String, let spec = TransferColumns.all.first(where:{$0.id == id}), id != "name" else { return }
            parent.customization[visibility:id] = TransferColumns.visible(spec,in:parent.customization) ? .hidden : .visible
        }
    }
}

final class NativeTransfersTable: NSTableView {
    var menuProvider: ((Set<String>) -> NSMenu?)?
    var focusChanged: ((Bool) -> Void)?
    var restoreColumnOrder: (() -> Void)?
    override func becomeFirstResponder() -> Bool { let value = super.becomeFirstResponder(); if value { focusChanged?(true) }; return value }
    override func resignFirstResponder() -> Bool { let value = super.resignFirstResponder(); if value { focusChanged?(false) }; return value }
    override func keyDown(with event: NSEvent) {
        if [36,76].contains(event.keyCode) && event.modifierFlags.intersection([.command,.control,.option,.shift]).isEmpty {
            if let doubleAction { NSApp.sendAction(doubleAction,to:target,from:self) }; return
        }
        super.keyDown(with:event)
    }
    override func menu(for event: NSEvent) -> NSMenu? {
        let row = self.row(at:convert(event.locationInWindow,from:nil))
        if row >= 0 && !selectedRowIndexes.contains(row) { selectRowIndexes(IndexSet(integer:row),byExtendingSelection:false) }
        guard let coordinator = dataSource as? NativeTransferGrid.Coordinator else { return nil }
        let ids = Set(selectedRowIndexes.compactMap { coordinator.rows.indices.contains($0) ? coordinator.rows[$0].id : nil })
        return menuProvider?(ids)
    }
}

private class NativeTransferCell: NSTableCellView {
    let label = NSTextField(labelWithString:"")
    var secondary = false
    var statusTint: NSColor? { didSet { needsDisplay = true } }
    override var backgroundStyle: NSView.BackgroundStyle { didSet { updateColor() } }
    override init(frame frameRect: NSRect) {
        super.init(frame:frameRect); textField = label
        label.translatesAutoresizingMaskIntoConstraints = false; label.maximumNumberOfLines = 1
        addSubview(label)
        let inset = label.leadingAnchor.constraint(equalTo:leadingAnchor,constant:2)
        inset.identifier = "transfer-text-inset"
        NSLayoutConstraint.activate([inset,label.trailingAnchor.constraint(equalTo:trailingAnchor,constant:-2),label.centerYAnchor.constraint(equalTo:centerYAnchor)])
    }
    required init?(coder: NSCoder) { fatalError(String(localized:"init(coder:) has not been implemented",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) }
    func updateColor() {
        label.textColor = backgroundStyle == .emphasized ? .alternateSelectedControlTextColor : statusTint ?? (secondary ? .secondaryLabelColor : .labelColor)
        constraints.first(where: { $0.identifier == "transfer-text-inset" })?.constant = statusTint == nil ? 2 : 18
    }
    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        guard let tint = statusTint else { return }
        let rect = NSRect(x:2,y:(bounds.height-20)/2,width:min(bounds.width-4,label.intrinsicContentSize.width+26),height:20)
        tint.withAlphaComponent(0.075).setFill(); NSBezierPath(roundedRect:rect,xRadius:10,yRadius:10).fill()
        tint.setFill(); NSBezierPath(ovalIn:NSRect(x:10,y:(bounds.height-5)/2,width:5,height:5)).fill()
    }
}
private final class NativeProgressCell: NativeTransferCell {
    private let bar = NativeTransferProgress()
    override init(frame frameRect: NSRect) {
        super.init(frame:frameRect)
        NSLayoutConstraint.deactivate(constraints.filter { $0.firstAttribute == .leading && $0.firstItem === label })
        bar.translatesAutoresizingMaskIntoConstraints = false; addSubview(bar)
        NSLayoutConstraint.activate([bar.leadingAnchor.constraint(equalTo:leadingAnchor,constant:2),bar.widthAnchor.constraint(equalToConstant:44),bar.heightAnchor.constraint(equalToConstant:4),bar.centerYAnchor.constraint(equalTo:centerYAnchor),label.leadingAnchor.constraint(equalTo:bar.trailingAnchor,constant:5)])
        label.font = .monospacedDigitSystemFont(ofSize:11,weight:.regular)
    }
    required init?(coder: NSCoder) { fatalError(String(localized:"init(coder:) has not been implemented",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) }
    func configure(_ status: TransferPresentation) {
        bar.value = status.progress; bar.isHidden = status.progress == nil
        label.stringValue = status.progress.map { LocaleText.percent($0) } ?? "—"
        toolTip = status.progressDescription; label.setAccessibilityLabel(status.progressDescription)
    }
}
private final class NativeTransferProgress: NSView {
    var value: Double? { didSet { needsDisplay = true } }
    override func draw(_ dirtyRect: NSRect) {
        NSColor.separatorColor.setFill(); NSBezierPath(roundedRect:bounds,xRadius:2,yRadius:2).fill()
        if let value {
            NSColor(Design.green).setFill(); NSBezierPath(roundedRect:NSRect(x:0,y:0,width:bounds.width * CGFloat(min(1,max(0,value))),height:bounds.height),xRadius:2,yRadius:2).fill()
        }
    }
}
