import SwiftUI
import AppKit
import UniformTypeIdentifiers

/// Paths organize presentation; protocol indices remain the identity of file commands.
final class TorrentFileNode: NSObject {
    let key: String
    let path: String
    let index: UInt32?
    var children: [TorrentFileNode] = []
    var indices = Set<UInt32>()
    var isFolder: Bool { index == nil }
    var name: String { (path as NSString).lastPathComponent }
    init(key: String, path: String, index: UInt32? = nil) {
        self.key = key; self.path = path; self.index = index
    }
}

struct TorrentFilePath: Equatable {
    let index: UInt32
    let path: String
    init(_ file: FileItem) { index = file.index; path = file.name }
}

/// Built only when paths/indices change, never for a progress-only snapshot.
final class TorrentFileTree {
    let paths: [TorrentFilePath]
    private(set) var roots: [TorrentFileNode] = []
    private(set) var nodes: [String:TorrentFileNode] = [:]
    init(files: [FileItem]) {
        paths = files.map(TorrentFilePath.init)
        for file in files {
            let components = file.name.split(separator:"/",omittingEmptySubsequences:true).map(String.init)
            var parent: TorrentFileNode?
            var path = ""
            for component in components.dropLast() {
                path = path.isEmpty ? component : path + "/" + component
                let key = "folder:" + path
                let folder: TorrentFileNode
                if let existing = nodes[key] { folder = existing }
                else {
                    folder = TorrentFileNode(key:key,path:path); nodes[key] = folder
                    if let parent { parent.children.append(folder) } else { roots.append(folder) }
                }
                folder.indices.insert(file.index); parent = folder
            }
            let node = TorrentFileNode(key:"file:\(file.index)",path:file.name,index:file.index)
            node.indices = [file.index]; nodes[node.key] = node
            if let parent { parent.children.append(node) } else { roots.append(node) }
        }
        // Stable folders-first ordering preserves metadata order within each group.
        roots = roots.filter(\.isFolder) + roots.filter { !$0.isFolder }
        for node in nodes.values where node.isFolder {
            node.children = node.children.filter(\.isFolder) + node.children.filter { !$0.isFolder }
        }
    }
    func matchingKeys(_ query: String) -> Set<String> {
        guard !query.isEmpty else { return Set(nodes.keys) }
        var result = Set<String>()
        for path in paths where path.path.localizedCaseInsensitiveContains(query) {
            result.insert("file:\(path.index)")
            var parent = (path.path as NSString).deletingLastPathComponent
            while !parent.isEmpty && parent != "." {
                result.insert("folder:" + parent)
                parent = (parent as NSString).deletingLastPathComponent
            }
        }
        return result
    }
    func targets(_ selection: Set<String>, query: String) -> Set<UInt32> {
        guard !selection.isEmpty else { return [] }
        let visible = query.isEmpty ? nil : matchingKeys(query)
        return selection.reduce(into:Set<UInt32>()) { result, key in
            guard visible?.contains(key) != false, let node = nodes[key], query.isEmpty || !node.isFolder else { return }
            result.formUnion(node.indices)
        }
    }
}

@MainActor final class TorrentFilesSession: ObservableObject {
    @Published var query = "" {
        didSet { if query != oldValue { selection.formIntersection(tree.matchingKeys(query)) } }
    }
    @Published var selection = Set<String>()
    @Published var expanded = Set<String>()
    @Published private(set) var tree: TorrentFileTree
    init(files: [FileItem]) { tree = TorrentFileTree(files:files) }
    func update(_ files: [FileItem]) {
        guard tree.paths != files.map(TorrentFilePath.init) else { return }
        tree = TorrentFileTree(files:files)
        selection.formIntersection(Set(tree.nodes.keys)); expanded.formIntersection(Set(tree.nodes.keys))
    }
}

@MainActor final class TorrentFilesSessions: ObservableObject {
    private var sessions: [String:TorrentFilesSession] = [:]
    func session(for torrent: TorrentItem) -> TorrentFilesSession {
        if let session = sessions[torrent.id] { return session }
        let session = TorrentFilesSession(files:torrent.files); sessions[torrent.id] = session
        return session
    }
}

struct TorrentFilesView: View {
    @EnvironmentObject var store: LibraryStore
    let torrent: TorrentItem
    @ObservedObject var session: TorrentFilesSession
    @State private var showDetails = false
    @FocusState private var fileSearchFocused: Bool
    private var targets: Set<UInt32> { session.tree.targets(session.selection,query:session.query) }
    private var selectedFiles: [FileItem] {
        let indices = targets
        guard !indices.isEmpty else { return [] }
        return torrent.files.filter { indices.contains($0.index) }
    }
    private var canChange: Bool { store.canChangeTransfers && !store.busyTransfers.contains(torrent.id) && torrent.state != "checking" }
    private var included: Set<UInt32> { Set(torrent.files.filter(\.selected).map(\.index)) }
    static func priorityName(_ value: UInt8) -> String { value == 0 ? "Low" : value == 2 ? "High" : "Normal" }
    static func symbol(_ name: String) -> String {
        let ext = (name as NSString).pathExtension.lowercased()
        guard let type = UTType(filenameExtension:ext) else { return "doc" }
        if type.conforms(to:.movie) { return "film" }
        if type.conforms(to:.image) { return "photo" }
        if type.conforms(to:.audio) { return "music.note" }
        if type.conforms(to:.text) || ["srt","nfo"].contains(ext) { return "doc.text" }
        if type.conforms(to:.archive) { return "archivebox" }
        return "doc"
    }

    var body: some View {
        VStack(alignment:.leading,spacing:0) {
            VStack(alignment:.leading,spacing:9) {
                HStack {
                    Text("Files").font(.system(size:13,weight:.semibold))
                    Spacer()
                    Menu("Actions") { actions }.menuStyle(.borderlessButton).fixedSize()
                        .accessibilityLabel("File actions")
                }
                Text("\(included.count) of \(torrent.files.count) included · \(SizeText.bytes(torrent.files.filter(\.selected).reduce(UInt64(0)) { $0 + $1.size }))")
                    .font(.system(size:11)).foregroundStyle(.secondary)
                HStack(spacing:6) {
                    Image(systemName:"magnifyingglass").foregroundStyle(.secondary)
                    TextField("Search files",text:$session.query).textFieldStyle(.plain).accessibilityLabel("Search files in this transfer")
                        .focused($fileSearchFocused).onExitCommand { session.query = "" }
                    if !session.query.isEmpty {
                        Button { session.query = "" } label: { Image(systemName:"xmark.circle.fill") }
                            .buttonStyle(.plain).foregroundStyle(.secondary).accessibilityLabel("Clear file search")
                    }
                }.font(.system(size:12)).padding(7).background(.quaternary.opacity(0.35),in:RoundedRectangle(cornerRadius:6))
            }.padding(.horizontal,14).padding(.vertical,12)
            Divider()
            if torrent.files.isEmpty {
                emptyState(store.connected ? "File list unavailable" : "Files unavailable",detail:store.connected ? "Files will appear when metadata is available." : "Reconnect to load this transfer’s files.")
            } else if !session.query.isEmpty && session.tree.matchingKeys(session.query).isEmpty {
                emptyState("No matching files",detail:"Try another name or clear the search.",clearSearch:true)
            } else {
                TorrentFilesOutline(torrent:torrent,session:session,canChange:canChange,
                    include:{ indices, value in store.setFilesIncluded(torrentID:torrent.id,indices:indices,included:value) },
                    priority:{ indices, value in store.setPriorities(torrentID:torrent.id,indices:indices,priority:value) },
                    rename:{ file in store.requestRename(torrent,file:file) },canRename:canChange && store.storageRecovery == nil,
                    details:{ showDetails = true })
            }
            Divider()
            HStack(spacing:6) {
                if store.busyTransfers.contains(torrent.id) { ProgressView().controlSize(.mini) }
                Text(statusMessage ?? footerSummary).font(.system(size:11)).foregroundStyle(.secondary)
                    .lineLimit(1).truncationMode(.tail).help(statusMessage ?? footerSummary)
                    .accessibilityLabel(statusMessage ?? footerSummary)
                Spacer(minLength:0)
            }.frame(height:18).padding(.horizontal,14).padding(.vertical,8)
        }
        .background(Design.card).tint(Design.green)
        .onChange(of:torrent.files.map(TorrentFilePath.init),initial:true) { _, _ in session.update(torrent.files) }
        .onChange(of:session.selection) { _, _ in showDetails = false }
        .onChange(of:torrent.id) { _, _ in showDetails = false }
        .popover(isPresented:$showDetails) { details.padding(18).frame(width:290) }
    }

    private var footerSummary: String {
        if !targets.isEmpty { return "\(targets.count) \(targets.count == 1 ? "file" : "files") selected" }
        if !session.query.isEmpty { return "\(torrent.files.filter { $0.name.localizedCaseInsensitiveContains(session.query) }.count) matching files" }
        return "\(torrent.files.count) files"
    }
    @ViewBuilder private var priorityActions: some View {
        let indices = targets
        ForEach([UInt8(0),1,2],id:\.self) { value in
            Button(Self.priorityName(value)) { store.setPriorities(torrentID:torrent.id,indices:indices,priority:value) }
        }
    }
    @ViewBuilder private var actions: some View {
        let indices = targets
        let renameFile = selectedFiles.first
        Button("Include in Download") { store.setFilesIncluded(torrentID:torrent.id,indices:indices,included:true) }
            .disabled(!canChange || targets.isEmpty || targets.isSubset(of:included))
        Button("Skip Download") { store.setFilesIncluded(torrentID:torrent.id,indices:indices,included:false) }
            .disabled(!canChange || targets.isDisjoint(with:included) || included.subtracting(targets).isEmpty)
        Menu("Priority") { priorityActions }.disabled(!canChange || targets.isEmpty)
        Divider()
        Button("Rename…") { if let file = renameFile { store.requestRename(torrent,file:file) } }
            .disabled(!canChange || store.storageRecovery != nil || targets.count != 1 || session.selection.contains(where:{ session.tree.nodes[$0]?.isFolder == true }))
        Button("Show File Details") { showDetails = true }.disabled(targets.count != 1)
    }
    private var statusMessage: String? {
        if !store.connected { return "Last known files · Reconnect to make changes." }
        if store.busyTransfers.contains(torrent.id) { return "Updating files…" }
        if torrent.state == "checking" { return "Checking files · Changes are temporarily unavailable." }
        if store.hasFileOperation { return "Wait for the current file change to finish." }
        if !session.query.isEmpty && session.selection.contains(where:{ session.tree.nodes[$0]?.isFolder == true }) { return "Clear search to change the whole folder." }
        return nil
    }
    private func emptyState(_ title: String, detail: String, clearSearch: Bool = false) -> some View {
        VStack(alignment:.leading,spacing:8) {
            Text(title).font(.system(size:13,weight:.medium))
            Text(detail).font(.system(size:11)).foregroundStyle(.secondary)
            if clearSearch { Button("Clear Search") { session.query = "" }.controlSize(.small) }
        }
            .padding(14).frame(maxWidth:.infinity,maxHeight:.infinity,alignment:.topLeading)
    }
    private var details: some View {
        VStack(alignment:.leading,spacing:10) {
            if let file = selectedFiles.first {
                Text("File Details").font(.headline)
                Text(file.name).font(.system(size:12)).textSelection(.enabled).fixedSize(horizontal:false,vertical:true)
                Text(SizeText.bytes(file.size)).font(.system(size:12))
                Text(file.selected ? "Included in download" : "Skipped")
                Text("Priority: \(Self.priorityName(file.priority))")
                if torrent.state == "checking" { Text("Checking files") }
                else if torrent.state == "error" { Text("File progress unavailable") }
                else { Text("\(SizeText.bytes(min(file.downloaded,file.size))) downloaded\(store.connected ? "" : " · Last known")") }
                if file.selected && included.count == 1 { Text("Keep at least one file included.").foregroundStyle(.secondary) }
            }
        }.font(.system(size:12))
    }
}

/// Native outline supplies selection, disclosure, keyboard navigation, and virtualization.
private struct TorrentFilesOutline: NSViewRepresentable {
    let torrent: TorrentItem
    @ObservedObject var session: TorrentFilesSession
    let canChange: Bool
    let include: (Set<UInt32>,Bool) -> Void
    let priority: (Set<UInt32>,UInt8) -> Void
    let rename: (FileItem) -> Void
    let canRename: Bool
    let details: () -> Void

    func makeCoordinator() -> Coordinator { Coordinator(self) }
    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSScrollView(); scroll.hasVerticalScroller = true; scroll.autohidesScrollers = true
        scroll.drawsBackground = false; scroll.hasHorizontalScroller = true
        let outline = FilesNativeOutline()
        let column = NSTableColumn(identifier:NSUserInterfaceItemIdentifier("file"))
        column.title = "Name"; column.width = 400; column.minWidth = 170
        outline.addTableColumn(column); outline.outlineTableColumn = column
        for (identifier, title, width) in [("size", "Size", CGFloat(90)), ("progress", "Progress", CGFloat(100)), ("priority", "Priority", CGFloat(100))] {
            let column = NSTableColumn(identifier:NSUserInterfaceItemIdentifier(identifier))
            column.title = title; column.width = width; column.minWidth = width
            column.maxWidth = width + 60
            outline.addTableColumn(column)
        }
        outline.rowHeight = 28; outline.intercellSpacing = NSSize(width:0,height:0)
        outline.indentationPerLevel = 12; outline.columnAutoresizingStyle = .firstColumnOnlyAutoresizingStyle
        if !WorkspacePreferences.isFixture {
            outline.autosaveName = "torrent-file-columns"; outline.autosaveTableColumns = true
        }
        outline.allowsMultipleSelection = true; outline.allowsEmptySelection = true
        outline.backgroundColor = .clear; outline.style = .plain
        outline.setAccessibilityLabel("Torrent files")
        outline.identifier = NSUserInterfaceItemIdentifier("torrent-files-outline")
        outline.dataSource = context.coordinator; outline.delegate = context.coordinator
        outline.menu = NSMenu(); outline.menu?.delegate = context.coordinator
        outline.toggleInclusion = { [weak coordinator = context.coordinator] in coordinator?.toggleSelection() }
        scroll.documentView = outline; context.coordinator.outline = outline
        return scroll
    }
    func updateNSView(_ scroll: NSScrollView, context: Context) { context.coordinator.update(self) }

    @MainActor final class Coordinator: NSObject, NSOutlineViewDataSource, NSOutlineViewDelegate, NSMenuDelegate {
        var parent: TorrentFilesOutline
        weak var outline: FilesNativeOutline?
        private var tree: TorrentFileTree?
        private var query: String?
        private var torrentID: String?
        private var visible = Set<String>()
        private var rootChildren: [TorrentFileNode] = []
        private var folderChildren: [String:[TorrentFileNode]] = [:]
        private var files: [UInt32:FileItem] = [:]
        private var includedCount = 0
        private var syncing = false
        private var menuActions: [() -> Void] = []
        init(_ parent: TorrentFilesOutline) { self.parent = parent }

        func update(_ parent: TorrentFilesOutline) {
            self.parent = parent
            files = Dictionary(uniqueKeysWithValues:parent.torrent.files.map { ($0.index,$0) })
            includedCount = parent.torrent.files.filter(\.selected).count
            guard let outline else { return }
            syncing = true
            defer { syncing = false }
            if tree !== parent.session.tree || query != parent.session.query || torrentID != parent.torrent.id {
                tree = parent.session.tree; query = parent.session.query; torrentID = parent.torrent.id
                visible = parent.session.tree.matchingKeys(parent.session.query)
                rootChildren = parent.session.tree.roots.filter { visible.contains($0.key) }
                folderChildren = [:]
                for node in parent.session.tree.nodes.values where node.isFolder {
                    folderChildren[node.key] = node.children.filter { visible.contains($0.key) }
                }
                outline.reloadData()
                outline.collapseItem(nil,collapseChildren:true)
                let folders = parent.session.tree.nodes.values.filter { $0.isFolder && (parent.session.expanded.contains($0.key) || !parent.session.query.isEmpty) }
                    .sorted { $0.path.split(separator:"/").count < $1.path.split(separator:"/").count }
                for node in folders {
                    outline.expandItem(node)
                }
            }
            var selected = IndexSet()
            for key in parent.session.selection {
                if let node = parent.session.tree.nodes[key] { let row = outline.row(forItem:node); if row >= 0 { selected.insert(row) } }
            }
            let rows = outline.rows(in:outline.visibleRect)
            for row in rows.location..<(rows.location + rows.length) {
                guard let node = outline.item(atRow:row) as? TorrentFileNode else { continue }
                for column in outline.tableColumns.indices {
                    if let cell = outline.view(atColumn:column,row:row,makeIfNecessary:false) {
                        configureCell(cell,node:node,column:outline.tableColumns[column].identifier.rawValue)
                    }
                }
            }
            if outline.selectedRowIndexes != selected { outline.selectRowIndexes(selected,byExtendingSelection:false) }
        }
        private func children(_ item: Any?) -> [TorrentFileNode] {
            if let node = item as? TorrentFileNode { return folderChildren[node.key] ?? [] }
            return rootChildren
        }
        func outlineView(_ outlineView: NSOutlineView, numberOfChildrenOfItem item: Any?) -> Int { children(item).count }
        func outlineView(_ outlineView: NSOutlineView, child index: Int, ofItem item: Any?) -> Any { children(item)[index] }
        func outlineView(_ outlineView: NSOutlineView, isItemExpandable item: Any) -> Bool { (item as? TorrentFileNode)?.isFolder == true }
        func outlineView(_ outlineView: NSOutlineView, viewFor tableColumn: NSTableColumn?, item: Any) -> NSView? {
            guard let node = item as? TorrentFileNode else { return nil }
            let column = tableColumn?.identifier.rawValue ?? "file"
            let identifier = NSUserInterfaceItemIdentifier("file-" + column)
            let cell = outlineView.makeView(withIdentifier:identifier,owner:self) ?? {
                switch column {
                case "file": return FilesNativeCell()
                case "priority": return FilesPriorityCell()
                default: return FilesValueCell()
                }
            }()
            cell.identifier = identifier; configureCell(cell,node:node,column:column); return cell
        }
        private func configureCell(_ cell: NSView, node: TorrentFileNode, column: String) {
            if let cell = cell as? FilesNativeCell { configure(cell,node:node); return }
            let leaves = node.indices.compactMap { files[$0] }
            if let cell = cell as? FilesPriorityCell {
                let filteredFolder = node.isFolder && !parent.session.query.isEmpty
                cell.priority.configure(Set(leaves.map(\.priority)))
                cell.priority.isEnabled = parent.canChange && !filteredFolder
                cell.priority.setAccessibilityLabel("Priority for \(node.path)")
                cell.priority.toolTip = filteredFolder ? "Clear search to change the whole folder." : "Download priority for \(node.path)"
                let changePriority = parent.priority
                cell.priority.actionBlock = { value in changePriority(node.indices,value) }
                return
            }
            guard let cell = cell as? FilesValueCell else { return }
            let size = leaves.reduce(UInt64(0)) { $0 + $1.size }
            let value: String
            if column == "size" { value = SizeText.bytes(size) }
            else if parent.torrent.state == "checking" { value = "Checking" }
            else if parent.torrent.state == "error" { value = "—" }
            else if leaves.allSatisfy({ !$0.selected }) { value = "Skipped" }
            else if size > 0 {
                let downloaded = leaves.reduce(UInt64(0)) { $0 + min($1.downloaded,$1.size) }
                value = "\(Int(min(1,Double(downloaded) / Double(size)) * 100))%"
            } else { value = "—" }
            cell.value.stringValue = value
            cell.value.setAccessibilityLabel("\(column == "size" ? "Size" : "Progress") for \(node.path): \(value)\(parent.torrent.state == "error" && column == "progress" ? ", unavailable" : "")")
            cell.toolTip = node.path + "\n" + value
        }
        private func configure(_ cell: FilesNativeCell, node: TorrentFileNode) {
            let leaves = node.indices.compactMap { files[$0] }
            let included = leaves.filter(\.selected).count
            let value: NSControl.StateValue = included == 0 ? .off : included == leaves.count ? .on : .mixed
            let wouldEmpty = included > 0 && included == leaves.count && included == includedCount
            let filteredFolder = node.isFolder && !parent.session.query.isEmpty
            let help = filteredFolder ? "Clear search to change the whole folder." : wouldEmpty ? "Keep at least one file included." : "Include or skip this \(node.isFolder ? "folder" : "file")."
            let size = leaves.reduce(UInt64(0)) { $0 + $1.size }
            var metadata = SizeText.bytes(size)
            if node.isFolder { metadata = "\(leaves.count) \(leaves.count == 1 ? "file" : "files") · " + metadata }
            if included == 0 { metadata += " · Skipped" }
            else if included != leaves.count { metadata += " · Mixed" }
            else if parent.torrent.state == "checking" { metadata += " · Checking" }
            else if parent.torrent.state == "error" { metadata += " · Progress unavailable" }
            else if !node.isFolder, let file = leaves.first, file.size > 0 {
                metadata += " · \(Int(min(1,Double(file.downloaded)/Double(file.size)) * 100))%"
            }
            cell.name.stringValue = node.name
            cell.icon.image = NSImage(systemSymbolName:node.isFolder ? "folder" : TorrentFilesView.symbol(node.name),accessibilityDescription:nil)
            cell.check.state = value; cell.check.isEnabled = parent.canChange && !wouldEmpty && !filteredFolder
            cell.check.setAccessibilityLabel("Download \(node.path)")
            cell.check.setAccessibilityHelp(help); cell.check.toolTip = help
            cell.name.setAccessibilityLabel(node.path)
            cell.toolTip = node.path + "\n" + metadata
            cell.setAccessibilityLabel(node.path + ", " + metadata)
            cell.check.actionBlock = { [weak self] in self?.parent.include(node.indices,value != .on) }
        }
        func outlineViewSelectionDidChange(_ notification: Notification) {
            guard !syncing, let outline else { return }
            parent.session.selection = Set(outline.selectedRowIndexes.compactMap { (outline.item(atRow:$0) as? TorrentFileNode)?.key })
        }
        func outlineViewItemDidExpand(_ notification: Notification) {
            guard !syncing, parent.session.query.isEmpty, let node = notification.userInfo?["NSObject"] as? TorrentFileNode else { return }
            parent.session.expanded.insert(node.key)
        }
        func outlineViewItemDidCollapse(_ notification: Notification) {
            guard !syncing, parent.session.query.isEmpty, let node = notification.userInfo?["NSObject"] as? TorrentFileNode else { return }
            parent.session.expanded.remove(node.key)
        }
        func toggleSelection() {
            guard parent.canChange else { return }
            let indices = parent.session.tree.targets(parent.session.selection,query:parent.session.query)
            guard !indices.isEmpty else { return }
            parent.include(indices,!indices.allSatisfy { files[$0]?.selected == true })
        }
        func menuNeedsUpdate(_ menu: NSMenu) {
            menu.removeAllItems(); menuActions = []
            let include = parent.include, changePriority = parent.priority, rename = parent.rename, details = parent.details
            let indices = parent.session.tree.targets(parent.session.selection,query:parent.session.query)
            guard !indices.isEmpty else { return }
            let included = Set(files.values.filter(\.selected).map(\.index))
            if parent.canChange {
                if !indices.isSubset(of:included) { add("Include in Download",to:menu) { include(indices,true) } }
                if !indices.isDisjoint(with:included) && !included.subtracting(indices).isEmpty { add("Skip Download",to:menu) { include(indices,false) } }
                let priority = NSMenu(title:"Priority")
                for value in [UInt8(0),1,2] { add(TorrentFilesView.priorityName(value),to:priority) { changePriority(indices,value) } }
                let item = NSMenuItem(title:"Priority",action:nil,keyEquivalent:""); item.submenu = priority; menu.addItem(item)
                if indices.count == 1, let index = indices.first, let file = files[index], parent.canRename,
                   !parent.session.selection.contains(where:{ parent.session.tree.nodes[$0]?.isFolder == true }) {
                    menu.addItem(.separator()); add("Rename…",to:menu) { rename(file) }
                }
            }
            if indices.count == 1 { add("Show File Details",to:menu) { details() } }
        }
        private func add(_ title: String, to menu: NSMenu, action: @escaping () -> Void) {
            let item = NSMenuItem(title:title,action:#selector(invokeMenu(_:)),keyEquivalent:"")
            item.target = self; item.tag = menuActions.count; menuActions.append(action); menu.addItem(item)
        }
        @objc private func invokeMenu(_ item: NSMenuItem) { if menuActions.indices.contains(item.tag) { menuActions[item.tag]() } }
    }
}

private final class FilesNativeOutline: NSOutlineView {
    var toggleInclusion: (() -> Void)?
    override func frameOfOutlineCell(atRow row: Int) -> NSRect {
        var frame = super.frameOfOutlineCell(atRow:row)
        frame.origin.y = rect(ofRow:row).minY + 13.5 - frame.height / 2
        return frame
    }
    override func keyDown(with event: NSEvent) {
        if event.keyCode == 49 && event.modifierFlags.intersection([.command,.control,.option,.shift]).isEmpty { toggleInclusion?() }
        else { super.keyDown(with:event) }
    }
    override func menu(for event: NSEvent) -> NSMenu? {
        let row = row(at:convert(event.locationInWindow,from:nil))
        guard row >= 0 else { return nil }
        if !isRowSelected(row) { selectRowIndexes(IndexSet(integer:row),byExtendingSelection:false) }
        return super.menu(for:event)
    }
}

private final class FilesCheckbox: NSButton {
    var actionBlock: (() -> Void)?
    init() {
        super.init(frame:.zero); setButtonType(.switch); title = ""; allowsMixedState = true
        target = self; action = #selector(invoke)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    @objc private func invoke() { actionBlock?() }
}

private final class FilesNativeCell: NSTableCellView {
    let check = FilesCheckbox()
    let name = NSTextField(labelWithString:"")
    let icon = NSImageView()
    override init(frame frameRect: NSRect) {
        super.init(frame:frameRect)
        textField = name; imageView = icon
        name.font = .systemFont(ofSize:12); name.lineBreakMode = .byTruncatingMiddle
        icon.contentTintColor = .secondaryLabelColor
        for view in [check,icon,name] { view.translatesAutoresizingMaskIntoConstraints = false; addSubview(view) }
        NSLayoutConstraint.activate([
            check.leadingAnchor.constraint(equalTo:leadingAnchor,constant:2),check.centerYAnchor.constraint(equalTo:centerYAnchor),check.widthAnchor.constraint(equalToConstant:18),
            icon.leadingAnchor.constraint(equalTo:check.trailingAnchor,constant:5),icon.centerYAnchor.constraint(equalTo:centerYAnchor),icon.widthAnchor.constraint(equalToConstant:16),icon.heightAnchor.constraint(equalToConstant:16),
            name.leadingAnchor.constraint(equalTo:icon.trailingAnchor,constant:6),name.trailingAnchor.constraint(equalTo:trailingAnchor,constant:-8),name.centerYAnchor.constraint(equalTo:centerYAnchor)
        ])
        name.setContentCompressionResistancePriority(.defaultLow,for:.horizontal)
    }
    convenience init() { self.init(frame:.zero) }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
}

private final class FilesValueCell: NSTableCellView {
    let value = NSTextField(labelWithString:"")
    override init(frame frameRect: NSRect) {
        super.init(frame:frameRect)
        textField = value
        value.font = .monospacedDigitSystemFont(ofSize:11,weight:.regular)
        value.textColor = .secondaryLabelColor; value.alignment = .right
        value.lineBreakMode = .byTruncatingTail
        value.translatesAutoresizingMaskIntoConstraints = false; addSubview(value)
        NSLayoutConstraint.activate([
            value.leadingAnchor.constraint(equalTo:leadingAnchor,constant:6),
            value.trailingAnchor.constraint(equalTo:trailingAnchor,constant:-10),
            value.centerYAnchor.constraint(equalTo:centerYAnchor)
        ])
    }
    convenience init() { self.init(frame:.zero) }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
}

private final class FilesPriorityCell: NSTableCellView {
    let priority = FilesPriorityPopup()
    override init(frame frameRect: NSRect) {
        super.init(frame:frameRect)
        priority.translatesAutoresizingMaskIntoConstraints = false; addSubview(priority)
        NSLayoutConstraint.activate([
            priority.leadingAnchor.constraint(equalTo:leadingAnchor,constant:6),
            priority.trailingAnchor.constraint(equalTo:trailingAnchor,constant:-6),
            priority.centerYAnchor.constraint(equalTo:centerYAnchor),
            priority.heightAnchor.constraint(equalToConstant:22)
        ])
    }
    convenience init() { self.init(frame:.zero) }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
}

private final class FilesPriorityPopup: NSPopUpButton {
    var actionBlock: ((UInt8) -> Void)?
    init() {
        super.init(frame:.zero,pullsDown:false)
        isBordered = false; font = .systemFont(ofSize:11); controlSize = .small
        target = self; action = #selector(invoke)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    func configure(_ priorities: Set<UInt8>) {
        let mixed = priorities.count != 1
        // Progress snapshots must not rebuild a menu the user is interacting with.
        if numberOfItems != (mixed ? 4 : 3) {
            removeAllItems()
            if mixed { addItem(withTitle:"Mixed"); lastItem?.tag = -1 }
            for value in [UInt8(0),1,2] {
                addItem(withTitle:TorrentFilesView.priorityName(value)); lastItem?.tag = Int(value)
            }
        }
        let tag = priorities.count == 1 ? Int(priorities.first!) : -1
        if selectedItem?.tag != tag { selectItem(withTag:tag) }
    }
    @objc private func invoke() {
        guard let value = selectedItem?.tag, (0...2).contains(value) else { return }
        actionBlock?(UInt8(value))
    }
}
