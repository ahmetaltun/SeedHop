import SwiftUI
import UniformTypeIdentifiers

struct LibrarySearchAction { var focus: () -> Void; var focusTransfers: () -> Void; var toggleSidebar: (() -> Void)? = nil }
private struct LibrarySearchKey: FocusedValueKey { typealias Value = LibrarySearchAction }
extension FocusedValues {
    var librarySearch: LibrarySearchAction? {
        get { self[LibrarySearchKey.self] }
        set { self[LibrarySearchKey.self] = newValue }
    }
}

struct LibraryView: View {
    @EnvironmentObject var store: LibraryStore
    @Environment(\.openWindow) private var openWindow
    @State private var sortOrder = LibraryView.restoredSort()
    @State private var inspectorTab = 0
    @AppStorage("workspaceSidebarVisible",store:WorkspacePreferences.defaults) private var showSidebar = true
    @AppStorage("workspaceComfortableRows",store:WorkspacePreferences.defaults) private var comfortableRows = false
    @StateObject private var fileSessions = TorrentFilesSessions()
    @FocusState private var searchFocused: Bool
    @FocusState private var transfersFocused: Bool
    @State private var dropTargeted = false
    @State private var showNotice = false
    @AppStorage("transferColumns",store:WorkspacePreferences.defaults) private var columnCustomization = LibraryView.defaultColumns()
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    init(inspectorTab: Int? = nil) { _inspectorTab = State(initialValue:inspectorTab ?? WorkspacePreferences.defaults.integer(forKey:"workspaceDetailsTab")) }

    var body: some View {
        presentedWorkspace
            // Notice ownership stays outside resizable hosting panes, so layout
            // changes cannot remount announcement gates or repeat speech.
            .announceAccessibility(store.statisticsWarning)
            .announceAccessibility(store.schedulingWarning)
            .announceAccessibility(store.error.map { String(localized:"Something needs attention. \($0)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) })
            .announceAccessibility(store.fileOperation.map { store.connected ? String(localized:"File change. \($0.title)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Last known file change. \($0.title)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) },priority:.medium)
            .announceAccessibility(store.storageRecovery.map { String(localized:"File cleanup needs attention. \($0.localizedMessage)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) })
            .focusedSceneValue(\.librarySearch,LibrarySearchAction(focus:{ searchFocused = true },focusTransfers:focusTransfers,toggleSidebar:{ showSidebar.toggle() }))
            .onDrop(of:[UTType.fileURL.identifier, UTType.url.identifier, UTType.plainText.identifier],isTargeted:$dropTargeted,perform:handleDrop)
            .overlay { if dropTargeted { RoundedRectangle(cornerRadius:8).strokeBorder(Design.green,lineWidth:2).padding(3).allowsHitTesting(false).accessibilityHidden(true) } }
    }
    private var layout: some View {
        Group {
            if showSidebar {
                WorkspaceSplitView(vertical:true,autosaveName:"SeedHop.Library.Sidebar",initialLeading:180,minLeading:150,minTrailing:520) {
                    sidebar.environmentObject(store)
                } trailing: { workspace.environmentObject(store) }
            } else { workspace }
        }
        .background(Design.canvas)
        .frame(minWidth:780,minHeight:560)
        .tint(Design.green)
    }
    private var searchableWorkspace: some View {
        layout.toolbar { libraryToolbar }
            .searchable(text:Binding(get:{store.search},set:{ if store.search != $0 { store.search = $0 } }),placement:.toolbar,prompt:String(localized:"Search transfers",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
            .searchFocused($searchFocused)
            .onChange(of:inspectorTab) { _, value in WorkspacePreferences.defaults.set(value,forKey:"workspaceDetailsTab") }
            .onChange(of:sortOrder) { _, value in Self.saveSort(value) }
            .onChange(of:store.showRemoval) { old, new in if old && !new { focusTransfers() } }
            .onSubmit(of:.search) { focusTransfers() }
    }
    private var presentedWorkspace: some View {
        searchableWorkspace
            .sheet(isPresented:$store.showImport,onDismiss:focusTransfers) { ImportView().environmentObject(store) }
            .confirmationDialog(removalTitle,isPresented:$store.showRemoval,titleVisibility:.visible) {
                Button(String(localized:"Remove from Library",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { store.removeConfirmed(deleteFiles:false) }
                Button(String(localized:"Remove and Delete Downloaded Files",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),role:.destructive) { store.removeConfirmed(deleteFiles:true) }
            } message: { Text(removalMessage) }
    }
    private var removalTitle: String { String(localized:"Remove \(store.removalTargets.count) transfers?",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
    private var removalMessage: String {
        store.removalTargets.count == 1 ? String(localized:"“\(store.removalTargets.first?.name ?? String(localized:"This transfer",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))” will be removed. Its files stay on your Mac unless you choose to delete them.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"These \(store.removalTargets.count) transfers will be removed. Their files stay on your Mac unless you choose to delete them.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
    }
    @ToolbarContentBuilder private var libraryToolbar: some ToolbarContent {

            ToolbarItemGroup(placement:.navigation) {
                Button { showSidebar.toggle() } label: { Label(String(localized:"Toggle sidebar",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),systemImage:"sidebar.left") }.help(String(localized:"Show or hide filters",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                Button { store.beginImport() } label: { Label(String(localized:"Add torrent",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),systemImage:"plus") }.disabled(store.hasFileOperation).help(String(localized:"Add a torrent (⌘N)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                Button { store.openFiles() } label: { Label(String(localized:"Open torrent file",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),systemImage:"folder.badge.plus") }.disabled(store.hasFileOperation).help(String(localized:"Open a torrent file (⌘O)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
            }
            ToolbarItemGroup(placement:.primaryAction) {
                Button { store.toggleSelected(paused:false) } label: { Label(store.resumeActionLabel,systemImage:store.selectionNeedsRetry ? "arrow.clockwise" : "play.fill") }.disabled(!store.canResumeSelection).help(String(localized:"Resume paused transfers or retry errors without changing saved pause intent",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                Button { store.toggleSelected(paused:true) } label: { Label(String(localized:"Pause",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),systemImage:"pause.fill") }.disabled(!store.canPauseSelection).help(String(localized:"Pause selected transfers",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                Button { store.requestRemoval() } label: { Label(String(localized:"Remove",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),systemImage:"trash") }.disabled(!store.canRemoveSelection).help(String(localized:"Remove selected transfers",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                Menu {
                    Toggle(String(localized:"Comfortable Rows",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),isOn:$comfortableRows)
                    Menu(String(localized:"Columns",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { columnChoices }
                    Button(String(localized:"Restore Default Columns",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { columnCustomization = Self.defaultColumns(); WorkspaceTablePersistence.restoreWidths() }
                } label: { Label(String(localized:"View options",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),systemImage:"tablecells") }.help(String(localized:"Choose row density; right-click a column heading to choose columns",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                Divider()
                Button { withAnimation(reduceMotion ? nil : .easeInOut(duration:0.18)) { store.showInspector.toggle() } } label: { Label(String(localized:"Toggle details",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),systemImage:"rectangle.bottomthird.inset.filled") }.help(String(localized:"Show or hide transfer details (⌥⌘I)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
            }
    }
    private static func defaultColumns() -> TableColumnCustomization<TorrentItem> {
        var columns = TableColumnCustomization<TorrentItem>()
        for id in ["selectedSize","size","remaining","downloadedTraffic","uploadedTraffic","added","completed","destination"] { columns[visibility:id] = .hidden }
        return columns
    }
    @ViewBuilder private var columnChoices: some View {
        ForEach(Self.columnOptions,id:\.id) { column in
            Toggle(column.title,isOn:Binding(get:{
                let visibility = columnCustomization[visibility:column.id]
                return visibility == .visible || (visibility == .automatic && column.visible)
            },set:{ columnCustomization[visibility:column.id] = $0 ? .visible : .hidden }))
        }
    }
    private struct ColumnOption: Identifiable { let id: String; let title: String; let visible: Bool }
    private static var columnOptions: [ColumnOption] { [
        ColumnOption(id:"status",title:String(localized:"Status",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),visible:true), ColumnOption(id:"progress",title:String(localized:"Progress",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),visible:true),
        ColumnOption(id:"download",title:String(localized:"Download",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),visible:true), ColumnOption(id:"upload",title:String(localized:"Upload",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),visible:true),
        ColumnOption(id:"peers",title:String(localized:"Peers",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),visible:true), ColumnOption(id:"eta",title:String(localized:"ETA",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),visible:true),
        ColumnOption(id:"ratio",title:String(localized:"Ratio",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),visible:true), ColumnOption(id:"queue",title:String(localized:"Queue",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),visible:true),
        ColumnOption(id:"selectedSize",title:String(localized:"Selected Size",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),visible:false), ColumnOption(id:"size",title:String(localized:"Total Size",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),visible:false),
        ColumnOption(id:"remaining",title:String(localized:"Remaining",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),visible:false), ColumnOption(id:"downloadedTraffic",title:String(localized:"Downloaded Traffic",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),visible:false),
        ColumnOption(id:"uploadedTraffic",title:String(localized:"Uploaded Traffic",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),visible:false), ColumnOption(id:"added",title:String(localized:"Added",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),visible:false),
        ColumnOption(id:"completed",title:String(localized:"Completed",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),visible:false), ColumnOption(id:"destination",title:String(localized:"Save Path",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),visible:false)
    ] }
    private var sidebar: some View {
        VStack(alignment:.leading,spacing:0) {
            SectionEyebrow(title:String(localized:"Transfers",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).padding(.horizontal,16).padding(.top,16).padding(.bottom,10)
            VStack(spacing:4) {
                ForEach(LibraryFilter.allCases) { filter in
                    let selected = store.filter == filter
                    Button { store.filter = filter } label: {
                        HStack(spacing:10) {
                            Image(systemName:filter.symbol).font(.system(size:14,weight:.medium)).frame(width:19)
                            Text(filter.title).font(.system(size:12,weight:selected ? .semibold : .regular)).lineLimit(1).minimumScaleFactor(0.85).help(filter.title)
                            Spacer(minLength:0)
                            Text("\(store.count(in:filter))").font(.system(size:10,weight:.medium,design:.rounded)).monospacedDigit().foregroundStyle(selected ? Design.green : Design.secondary)
                        }.foregroundStyle(selected ? Design.green : Color.primary.opacity(0.75)).padding(.horizontal,12).frame(height:35)
                            .background(selected ? Design.green.opacity(0.09) : .clear,in:RoundedRectangle(cornerRadius:8))
                            .contentShape(Rectangle())
                    }.buttonStyle(.plain).accessibilityAddTraits(selected ? .isSelected : [])
                }
            }.padding(.horizontal,10)
            Divider().padding(.vertical,12)
            VStack(alignment:.leading,spacing:8) {
                Text(String(localized:"Tracker",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:11,weight:.medium)).foregroundStyle(.secondary)
                Picker(String(localized:"Tracker host",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),selection:$store.trackerHost) {
                    Text(String(localized:"All trackers",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).tag("")
                    ForEach(store.trackerHosts,id:\.self) { Text($0).tag($0) }
                }.labelsHidden().controlSize(.small)
            }.padding(.horizontal,14)
            Spacer()
            VStack(alignment:.leading,spacing:12) {
                Divider()
                HStack(spacing:8) {
                    Image(systemName:"externaldrive").font(.system(size:15)).foregroundStyle(.secondary)
                    VStack(alignment:.leading,spacing:3) {
                        Text(URL(fileURLWithPath:store.downloadDirectory).lastPathComponent).font(.system(size:11,weight:.medium))
                        Text(String(localized:"Default download folder",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:10)).foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button { NSWorkspace.shared.selectFile(nil,inFileViewerRootedAtPath:store.downloadDirectory) } label: { Image(systemName:"arrow.up.right") }.buttonStyle(.plain).foregroundStyle(.secondary).help(String(localized:"Show download folder in Finder",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).accessibilityLabel(String(localized:"Show download folder in Finder",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                }
                HStack(spacing:6) { Circle().fill(store.connected ? Design.green : Design.amber).frame(width:5,height:5); Text(store.switching ? String(localized:"Switching engine…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : store.connected ? (store.backgroundEnabled ? String(localized:"Running in background",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Engine connected",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) : store.connecting ? String(localized:"Connecting to engine…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Engine disconnected",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:10)).foregroundStyle(.secondary) }.accessibilityElement(children:.combine)
            }.padding(18)
        }.frame(maxWidth:.infinity,maxHeight:.infinity).background(Design.sidebar)
    }
    private var content: some View {
        VStack(spacing:0) {
            if store.designPreview {
                HStack { Image(systemName:"paintpalette"); Text("DESIGN PREVIEW · SAMPLE DATA").tracking(1.2); Spacer() }.font(.system(size:9,weight:.semibold)).foregroundStyle(Design.green).padding(.horizontal,26).padding(.vertical,9).background(Design.green.opacity(0.06))
            }
            VStack(alignment:.leading,spacing:6) {
                if hasNotice { noticeStrip }

            }
            if store.torrents.isEmpty && store.storageRecovery != nil {
                ContentUnavailableView(String(localized:"Retained files need attention",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),systemImage:"exclamationmark.circle",description:Text(String(localized:"Review the cleanup notice above. Your remaining files have been preserved.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)))
            }
            else if store.torrents.isEmpty { emptyState }
            else if store.visibleTorrents.isEmpty {
                if !store.trackerHost.isEmpty {
                    ContentUnavailableView {
                        Label(String(localized:"No transfers for this tracker",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),systemImage:"network")
                    } description: { Text(String(localized:"Try another tracker or clear your filters.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) }
                    actions: { Button(String(localized:"Clear Filters",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { store.clearFilters() } }
                }
                else if !store.search.isEmpty { ContentUnavailableView.search(text:store.search).frame(maxHeight:.infinity) }
                else {
                    ContentUnavailableView {
                        Label {
                            Text(store.filter == .attention && store.hasUnscopedRecovery ? String(localized:"Retained files need attention",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : store.filter.emptyTitle).foregroundStyle(.primary)
                        } icon: {
                            Image(systemName:store.filter == .attention ? (store.hasUnscopedRecovery ? "exclamationmark.circle" : "checkmark.circle") : store.filter.symbol).foregroundStyle(store.filter == .attention ? (store.hasUnscopedRecovery ? Design.amber : Design.green) : Design.secondary)
                        }
                    } description: {
                        Text(store.filter == .attention && store.hasUnscopedRecovery ? String(localized:"Review the cleanup notice above. Your other transfers are still in the library.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Your other transfers are still in the library.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                    } actions: {
                        Button(String(localized:"Show all transfers",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { store.clearFilters() }
                    }.frame(maxHeight:.infinity)
                }
            }
            else { transferTable }

        }.background(Design.canvas)
    }
    private var statusBar: some View {
            HStack(spacing:9) {
                Text(String(localized:"\(store.visibleTorrents.count) transfers",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:10)).foregroundStyle(.secondary)
                if !store.selection.isEmpty { Text("· \(store.selection.count) selected").font(.system(size:10)).foregroundStyle(.secondary) }
                if !store.busyTransfers.isEmpty { ProgressView().controlSize(.mini); Text(String(localized:"Saving changes…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:10)).foregroundStyle(.secondary) }
                if !store.search.isEmpty || !store.trackerHost.isEmpty || store.filter != .all {
                    Text(activeFilterDescription).font(.system(size:10)).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                        .help(activeFilterDescription).accessibilityLabel(String(localized:"Active filters: \(activeFilterDescription)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).layoutPriority(-1)
                    Button(String(localized:"Clear Filters",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { store.clearFilters() }.controlSize(.mini).fixedSize()
                }
                Spacer(minLength:0)
                ViewThatFits(in:.horizontal) {
                    Text(String(localized:"↓ \(SizeText.rate(store.downloadRate,limit:store.downloadLimit))   ↑ \(SizeText.rate(store.uploadRate,limit:store.uploadLimit))",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                    Text(String(localized:"↓ \(SizeText.rate(store.downloadRate))   ↑ \(SizeText.rate(store.uploadRate))",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                }.monospacedDigit().help(String(localized:"\(store.bandwidthStatus). Speeds show current rate / limit.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                    .accessibilityLabel(String(localized:"Download \(SizeText.rate(store.downloadRate)), upload \(SizeText.rate(store.uploadRate)). \(store.bandwidthStatus)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                Text(store.connected ? store.bandwidthProfileTitle : String(localized:"Disconnected",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).foregroundStyle(store.alternativeLimitsEnabled ? Design.green : .secondary).help(store.bandwidthStatus)
                Button { openWindow(id:"bandwidth") } label: {
                    Image(systemName:store.alternativeLimitsEnabled ? "tortoise.fill" : "tortoise")
                        .padding(.horizontal,4).padding(.vertical,3)
                        .foregroundStyle(store.alternativeLimitsEnabled ? Design.green : .secondary)
                }.buttonStyle(.borderless).accessibilityLabel(String(localized:"Open bandwidth limits",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                    .help(String(localized:"\(store.alternativeLimitsHint) Click to edit bandwidth limits.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                Toggle(String(localized:"Alternative limits",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),isOn:Binding(get:{store.alternativeLimitsEnabled},set:{store.toggleAlternativeLimits($0)}))
                    .labelsHidden().toggleStyle(.switch).controlSize(.mini).fixedSize()
                    .disabled(!store.canToggleAlternativeLimits)
                    .accessibilityLabel(String(localized:"Alternative bandwidth limits",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                    .accessibilityValue(store.alternativeLimitsEnabled ? String(localized:"On",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Off",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                    .accessibilityHint(store.alternativeLimitsHint)
                    .help(store.alternativeLimitsHint)
            }.font(.system(size:11)).padding(.horizontal,10).frame(height:26)
    }
    private var activeFilterDescription: String {
        [store.filter == .all ? nil : store.filter.title,store.search.isEmpty ? nil : String(localized:"Name: \(store.search)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),store.trackerHost.isEmpty ? nil : String(localized:"Tracker: \(store.trackerHost)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)].compactMap { $0 }.joined(separator:" · ")
    }
    private var hasNotice: Bool { store.switching || store.storageRecovery != nil || store.fileOperation != nil || !store.storageRequests.isEmpty || store.error != nil || store.statisticsWarning != nil || store.schedulingWarning != nil }
    private var noticeTitle: String {
        if store.storageRecovery != nil { return String(localized:"File cleanup needs attention",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
        if let operation = store.fileOperation { return store.connected ? String(localized:"\(operation.title) · \(operation.detail)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Last known: \(operation.title) · \(operation.detail)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
        if let error = store.error { return error }
        if let warning = store.schedulingWarning { return warning }
        if let warning = store.statisticsWarning { return warning }
        return store.switching ? String(localized:"Switching transfer engine…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Preparing file change…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
    }
    private var noticeStrip: some View {
        HStack(spacing:8) {
            Image(systemName:store.error != nil || store.storageRecovery != nil ? "exclamationmark.circle" : "arrow.triangle.2.circlepath")
            Text(noticeTitle).lineLimit(1).truncationMode(.tail).help(noticeTitle)
            Spacer(minLength:4)
            if let operation = store.fileOperation, operation.canCancel {
                Button(String(localized:"Cancel",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { store.cancelFileOperation(operation) }.disabled(!store.canCancelFileOperation(operation))
            }
            Button(String(localized:"Review",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { showNotice = true }
        }.font(.system(size:11)).controlSize(.small).padding(.horizontal,10).frame(height:34)
            .background(Design.amber.opacity(0.06))
            .popover(isPresented:$showNotice) { noticeDetails.padding(12).frame(width:480) }
    }
    private var noticeDetails: some View {
        VStack(alignment:.leading,spacing:10) {
            if store.switching { Text(String(localized:"Switching transfer engine…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) }
            if let recovery = store.storageRecovery { storageRecoveryNotice(recovery) }
            if let operation = store.fileOperation { fileOperationProgress(operation) }
            if let warning = store.schedulingWarning { InlineNotice(title:String(localized:"Scheduling needs attention",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),message:warning) }
            if let warning = store.statisticsWarning { InlineNotice(title:String(localized:"Statistics need attention",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),message:warning) }
            if let error = store.error {
                InlineNotice(title:String(localized:"Something needs attention",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),message:error,retry:store.connected ? nil : { Task { store.error = nil; await store.start() } },dismiss:{ store.error = nil; showNotice = false })
            }
        }.environment(\.accessibilityAnnouncementsEnabled,false)
    }
    private func fileOperationProgress(_ operation: FileOperationItem) -> some View {
        VStack(alignment:.leading,spacing:9) {
            HStack(alignment:.top,spacing:12) {
                VStack(alignment:.leading,spacing:4) {
                    Text(store.connected ? operation.title : String(localized:"Last known: \(operation.title)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:12,weight:.semibold))
                    Text(store.torrents.first(where:{ $0.id == operation.torrentId })?.name ?? String(localized:"Transfer files",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                        .font(.system(size:11)).lineLimit(1).truncationMode(.middle)
                }
                Spacer(minLength:8)
                if ["preparing", "copying"].contains(operation.phase) {
                    Button(operation.cancelRequested || store.cancellingOperations.contains(operation.id) ? String(localized:"Cancelling…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Cancel",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) {
                        store.cancelFileOperation(operation)
                    }.disabled(!store.canCancelFileOperation(operation)).accessibilityLabel(String(localized:"Cancel file change",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                } else { ProgressView().controlSize(.small).accessibilityLabel(operation.title) }
            }
            if operation.phase == "copying" && operation.totalBytes > 0 {
                TransferProgress(value:Double(operation.completedBytes) / Double(max(operation.completedBytes,operation.totalBytes)))
                    .accessibilityLabel(String(localized:"Files copied",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                    .accessibilityValue(String(localized:"\(SizeText.bytes(operation.completedBytes)) of \(SizeText.bytes(operation.totalBytes))",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
            }
            Text(store.connected ? operation.detail : String(localized:"Reconnect to see current file-operation progress.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:10)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true)
        }.padding(14).frame(maxWidth:.infinity,alignment:.leading)
            .background(Design.card,in:RoundedRectangle(cornerRadius:10))
            .overlay(RoundedRectangle(cornerRadius:10).stroke(Design.line))

    }
    private func storageRecoveryNotice(_ notice: StorageRecoveryItem) -> some View {
        VStack(alignment:.leading,spacing:10) {
            Text(String(localized:"File cleanup needs attention",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:12,weight:.semibold))
            Text(notice.localizedMessage).font(.system(size:11)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true).textSelection(.enabled)
            HStack {
                Button(store.resolvingStorageRecovery ? String(localized:"Checking…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Retry Cleanup",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { store.resolveStorageRecovery(notice,keepCopies:false) }
                    .disabled(!store.canResolveStorageRecovery(notice))
                if notice.canKeepCopies {
                    Button(String(localized:"Keep Extra Copies",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { store.resolveStorageRecovery(notice,keepCopies:true) }
                        .disabled(!store.canResolveStorageRecovery(notice))
                        .help(String(localized:"Keep all remaining files and finish tracking this cleanup.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                }
                Spacer(minLength:0)
            }
            HStack {
                Button(String(localized:"Show Original Folder",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { NSWorkspace.shared.selectFile(nil,inFileViewerRootedAtPath:notice.originalDirectory) }
                if notice.originalDirectory != notice.destinationDirectory {
                    Button(String(localized:"Show Destination",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { NSWorkspace.shared.selectFile(nil,inFileViewerRootedAtPath:notice.destinationDirectory) }
                }
            }.font(.system(size:10))
        }.padding(14).frame(maxWidth:.infinity,alignment:.leading)
            .background(Design.amber.opacity(0.06),in:RoundedRectangle(cornerRadius:10))
            .overlay(RoundedRectangle(cornerRadius:10).stroke(Design.amber.opacity(0.25)))

    }
    private var transferTable: some View {
        TransferWorkspaceTable(store:store,sortOrder:$sortOrder,columnCustomization:$columnCustomization,comfortableRows:comfortableRows,transfersFocused:$transfersFocused)
    }
    private var emptyState: some View { LibraryEmptyState().environmentObject(store) }
    private var workspace: some View {
        VStack(spacing:0) {
            Group {
                if store.showInspector {
                    WorkspaceSplitView(vertical:false,autosaveName:"SeedHop.Library.Details",initialLeading:-280,minLeading:160,minTrailing:180) { content.environmentObject(store) } trailing: { inspector.environmentObject(store) }
                } else { content }
            }
            Divider(); statusBar
        }
    }
    private let detailSections = ["general","files","peers","trackers","activity"]
    private var inspector: some View {
        VStack(spacing:0) { detailsHeader; Divider(); detailsBody }.background(Design.card)
    }
    private var detailsHeader: some View {
        HStack(spacing:12) {
            Picker(String(localized:"Transfer details",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),selection:$inspectorTab) {
                Text(String(localized:"General",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).tag(0); Text(String(localized:"Files",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).tag(1); Text(String(localized:"Peers",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).tag(2); Text(String(localized:"Trackers",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).tag(3); Text(String(localized:"Activity",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).tag(4)
            }.pickerStyle(.segmented).labelsHidden().frame(maxWidth:500)
            Spacer(minLength:0)
            Text(store.selected.count == 1 ? store.selected[0].name : "\(store.selected.count) selected").font(.system(size:11)).lineLimit(1).truncationMode(.middle)
            Button { store.showInspector = false } label: { Image(systemName:"xmark") }.buttonStyle(.plain).accessibilityLabel(String(localized:"Hide transfer details",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
        }.padding(.horizontal,10).frame(height:34)
    }
    @ViewBuilder private var detailsBody: some View {
        if store.selected.count == 1, let item = store.selected.first {
            selectedDetails(item)
        } else if store.selected.count > 1 {
            ScrollView { multipleSelectionInspector }.frame(maxWidth:.infinity,maxHeight:.infinity)
        } else {
            VStack(spacing:6) {
                Text(String(localized:"Select a transfer",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:12,weight:.semibold))
                Text(String(localized:"Inspect files, peers, trackers, and activity here.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:11)).foregroundStyle(.secondary)
            }.frame(maxWidth:.infinity,maxHeight:.infinity).accessibilityElement(children:.combine)
        }
    }
    private func selectedDetails(_ item: TorrentItem) -> some View {
        let tab = detailSections[min(max(0,inspectorTab),4)]
        let identity = "\(item.id)|\(tab)|\(store.allPeers)|\(store.showInspector)|\(store.engineInstance)"
        return selectedDetailsContent(item,tab:tab).task(id:identity) {
            if store.designPreview { showPreviewDetails(item,section:tab) }
            else { await store.watchDetails(id:item.id,section:tab,allPeers:store.allPeers) }
        }
    }
    @ViewBuilder private func selectedDetailsContent(_ item: TorrentItem, tab: String) -> some View {
        if inspectorTab == 1 { TorrentFilesView(torrent:item,session:fileSessions.session(for:item)) }
        else {
            VStack(spacing:0) {
                if store.presentation(item).isError && !store.presentation(item).hasRecovery { transferFailure(item,status:store.presentation(item)).padding(8) }
                TorrentDiagnosticsView(torrent:item,tab:tab)
            }
        }
    }
    private func showPreviewDetails(_ item: TorrentItem, section: String) {
        store.details = PreviewFixtures.diagnostics(item,section:section)
        store.history[item.id] = PreviewFixtures.trafficHistory()
        store.events[item.id] = [TransferEvent(message:String(localized:"Sample transfer activity",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))]
    }
    private static func duration(_ value: UInt64) -> String { LocaleText.duration(value) }
    private static func date(_ value: UInt64) -> String { Date(timeIntervalSince1970:Double(value)).formatted(date:.abbreviated,time:.omitted) }
    static let sortFields: [(String, PartialKeyPath<TorrentItem>)] = [("name",\.name),("status",\.sortableStatus),("progress",\.progress),("download",\.downloadRate),("upload",\.uploadRate),("peers",\.peers),("eta",\.sortableETA),("ratio",\.sortableRatio),("queue",\.queuePosition),("size",\.fullBytes),("selected",\.totalBytes),("remaining",\.remainingBytes),("downloaded",\.sortableDownloaded),("uploaded",\.sortableUploaded),("added",\.sortableAdded),("completed",\.sortableCompleted),("path",\.destination)]
    private static func restoredSort() -> [KeyPathComparator<TorrentItem>] {
        if let saved = WorkspacePreferences.defaults.array(forKey:"workspaceSortOrder") as? [[String:Any]], !saved.isEmpty {
            return saved.map { comparator(field:$0["field"] as? String ?? "name",order:($0["reverse"] as? Bool ?? false) ? .reverse : .forward) }
        }
        return [comparator(field:WorkspacePreferences.defaults.string(forKey:"workspaceSort") ?? "name",order:WorkspacePreferences.defaults.bool(forKey:"workspaceSortReverse") ? .reverse : .forward)]
    }
    static func comparator(field: String, order: SortOrder) -> KeyPathComparator<TorrentItem> {
        switch field {
        case "status": return KeyPathComparator(\.sortableStatus,comparator:String.StandardComparator.localizedStandard,order:order)
        case "progress": return KeyPathComparator(\.progress,order:order)
        case "download": return KeyPathComparator(\.downloadRate,order:order)
        case "upload": return KeyPathComparator(\.uploadRate,order:order)
        case "peers": return KeyPathComparator(\.peers,order:order)
        case "eta": return KeyPathComparator(\.sortableETA,order:order)
        case "ratio": return KeyPathComparator(\.sortableRatio,order:order)
        case "queue": return KeyPathComparator(\.queuePosition,order:order)
        case "size": return KeyPathComparator(\.fullBytes,order:order)
        case "selected": return KeyPathComparator(\.totalBytes,order:order)
        case "remaining": return KeyPathComparator(\.remainingBytes,order:order)
        case "downloaded": return KeyPathComparator(\.sortableDownloaded,order:order)
        case "uploaded": return KeyPathComparator(\.sortableUploaded,order:order)
        case "added": return KeyPathComparator(\.sortableAdded,order:order)
        case "completed": return KeyPathComparator(\.sortableCompleted,order:order)
        case "path": return KeyPathComparator(\.destination,comparator:String.StandardComparator.localizedStandard,order:order)
        default: return KeyPathComparator(\.name,comparator:String.StandardComparator.localizedStandard,order:order)
        }
    }
    private static func saveSort(_ value: [KeyPathComparator<TorrentItem>]) {
        let descriptors: [[String:Any]] = value.compactMap { comparator in
            guard let field = sortFields.first(where:{ $0.1 == comparator.keyPath }) else { return nil }
            return ["field":field.0,"reverse":comparator.order == .reverse]
        }
        WorkspacePreferences.defaults.set(descriptors,forKey:"workspaceSortOrder")
    }
    private func inspectorSummary(_ item: TorrentItem, status: TransferPresentation) -> some View {
        VStack(alignment:.leading,spacing:10) {
            HStack(alignment:.top,spacing:10) {
                Image(systemName:Design.symbol(item.name)).font(.system(size:18,weight:.regular))
                    .foregroundStyle(Design.green).frame(width:36,height:36)
                    .background(Design.green.opacity(0.08),in:RoundedRectangle(cornerRadius:8))
                    .accessibilityHidden(true)
                Text(item.name).font(.system(size:13,weight:.semibold)).lineLimit(2)
                    .frame(maxWidth:.infinity,minHeight:36,alignment:.topLeading)
                    .textSelection(.enabled).help(item.name).accessibilityLabel(item.name)
            }
            HStack(alignment:.firstTextBaseline,spacing:8) {
                StateBadge(torrent:item,presentation:status)
                Spacer(minLength:0)
                if let progress = status.progress {
                    Text(LocaleText.percent(progress))
                        .font(.system(size:12,weight:.semibold)).monospacedDigit()
                }
            }
            if let progress = status.progress {
                VStack(alignment:.leading,spacing:6) {
                    TransferProgress(value:progress).accessibilityLabel(status.progressTitle)
                    HStack(spacing:4) {
                        Text(status.progressTitle)
                        Spacer(minLength:0)
                        Text(String(localized:"\(SizeText.bytes(status.completedBytes)) of \(SizeText.bytes(status.totalBytes))",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).monospacedDigit()
                    }.font(.system(size:10)).foregroundStyle(.secondary)
                }
            }
            if !status.isError || status.hasRecovery {
                Text(status.detail).font(.system(size:11)).foregroundStyle(.secondary)
                    .fixedSize(horizontal:false,vertical:true)
            }
            if status.isQueued {
                Button(String(localized:"Move to Front of Queue",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { store.moveToFront(item) }
                    .controlSize(.small).disabled(!store.canChangeTransfers || store.busyTransfers.contains(item.id))
            }
            if status.isError && !status.hasRecovery {
                // Keep recovery reachable without letting long diagnostics displace the tab content.
                ScrollView { transferFailure(item,status:status) }.frame(maxHeight:160)
            }
        }.padding(14)
    }
    private func trackers(_ item: TorrentItem) -> some View {
        VStack(alignment:.leading,spacing:10) {
            HStack {
                Text(String(localized:"Trackers",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:13,weight:.semibold))
                Spacer()
                Text("\(item.trackers.count)").font(.system(size:11)).monospacedDigit().foregroundStyle(.secondary)
            }
            if item.trackers.isEmpty {
                Text(String(localized:"No trackers are listed for this transfer.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:11)).foregroundStyle(.secondary)
            } else {
                ForEach(Array(item.trackers.enumerated()),id:\.offset) { _, tracker in
                    HStack(alignment:.top,spacing:8) {
                        Image(systemName:"network").foregroundStyle(.secondary).accessibilityHidden(true)
                        Text(redactTracker(tracker)).textSelection(.enabled).fixedSize(horizontal:false,vertical:true)
                    }.font(.system(size:11)).frame(maxWidth:.infinity,alignment:.leading)
                        .padding(10).background(Design.line.opacity(0.5),in:RoundedRectangle(cornerRadius:7))
                }
            }
        }
    }
    private func transferFailure(_ item: TorrentItem, status: TransferPresentation) -> some View {
        VStack(alignment:.leading,spacing:10) {
            Label(status.errorTitle,systemImage:"exclamationmark.circle.fill").font(.system(size:12,weight:.semibold)).foregroundStyle(Design.amber)
            Text(status.detail).font(.system(size:11)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true)
            HStack {
                Button(String(localized:"Retry",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { store.retryTransfer(item) }.disabled(!store.canRetryTransfer(item))
                Button(String(localized:"Show Folder",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { store.revealDestination(item) }.help(String(localized:"Open the destination or its closest available parent in Finder",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
            }.controlSize(.small)
            if status.canMoveAfterError {
                Button(String(localized:"Move Files…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { store.chooseRelocation(item) }.controlSize(.small).disabled(!store.canChangeTransfers || store.busyTransfers.contains(item.id))
            }
            if let error = item.error {
                DisclosureGroup(String(localized:"Error details",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { Text(error).font(.system(size:10,design:.monospaced)).foregroundStyle(.secondary).textSelection(.enabled).fixedSize(horizontal:false,vertical:true) }.font(.system(size:10))
            }
        }.padding(12).background(Design.amber.opacity(0.06),in:RoundedRectangle(cornerRadius:10))
            .announceAccessibility(String(localized:"\(item.name). \(status.errorTitle). \(status.detail)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
    }
    private func overview(_ t: TorrentItem) -> some View {
        let status = store.presentation(t)
        return VStack(alignment:.leading,spacing:10) {
            Text(String(localized:"Activity",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:13,weight:.semibold))
            detail(status.progressTitle,status.progress == nil ? "—" : SizeText.bytes(status.completedBytes))
            detail(String(localized:"Uploaded",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),["downloading", "waiting", "seeding"].contains(t.state) ? SizeText.bytes(t.uploadedBytes) : "—")
            detail(store.connected ? String(localized:"Connected peers",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Last known peers",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),"\(t.peers)")
            detail(String(localized:"Share ratio",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),t.shareRatio.map { $0.formatted(.number.precision(.fractionLength(2))) } ?? "—")
            Divider()
            Text(String(localized:"Location",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:12,weight:.semibold))
            Label(URL(fileURLWithPath:t.destination).lastPathComponent,systemImage:"folder").font(.system(size:11))
            Text(t.destination.replacingOccurrences(of:NSHomeDirectory(),with:"~")).font(.system(size:10)).foregroundStyle(.secondary).textSelection(.enabled).fixedSize(horizontal:false,vertical:true)
            Divider()
            Text(String(localized:"Info hash",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:12,weight:.semibold))
            Text(t.id).font(.system(size:9,design:.monospaced)).foregroundStyle(.secondary).textSelection(.enabled).lineLimit(3)
        }
    }
    private func detail(_ label: String, _ value: String) -> some View { HStack { Text(label).foregroundStyle(.secondary); Spacer(); Text(value).monospacedDigit() }.font(.system(size:11)) }
    private var inspectorTabs: some View {
        Picker(String(localized:"Details",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),selection:$inspectorTab) { Text(String(localized:"Overview",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).tag(0); Text(String(localized:"Files",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).tag(1); Text(String(localized:"Trackers",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).tag(2) }.pickerStyle(.segmented).labelsHidden()
    }
    private func redactTracker(_ raw: String) -> String {
        guard let url = URL(string:raw) else { return String(localized:"Tracker",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
        return "\(url.scheme ?? "https")://\(url.host ?? "tracker")" + (url.port.map { ":\($0)" } ?? "")
    }
    private func handleDrop(_ providers: [NSItemProvider]) -> Bool {
        guard let provider = providers.first else { return false }
        if provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier) || provider.hasItemConformingToTypeIdentifier(UTType.url.identifier) {
            _ = provider.loadObject(ofClass:URL.self) { url,_ in if let url { Task { @MainActor in store.open([url]) } } }; return true
        }
        _ = provider.loadObject(ofClass:String.self) { value,_ in if let value { Task { @MainActor in store.beginImport(magnet:value) } } }; return true
    }

    private func canAct(on ids: Set<String>) -> Bool {
        store.canChangeTransfers && !ids.isEmpty && ids.isSubset(of:Set(store.torrents.map(\.id))) && store.busyTransfers.isDisjoint(with:ids)
    }

    private func focusTransfers() {
        searchFocused = false
        transfersFocused = true
    }

    private var multipleSelectionInspector: some View {
        VStack(alignment:.leading,spacing:12) {
            Text(String(localized:"\(store.selected.count) transfers selected",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:13,weight:.semibold))
            Text(String(localized:"Actions apply to the selected transfers. Select one transfer to inspect its diagnostics.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:11)).foregroundStyle(.secondary)
            Divider()
            LazyVGrid(columns:[GridItem(.flexible()),GridItem(.flexible())],spacing:12) {
                detail(String(localized:"Selected size",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),SizeText.bytes(store.selected.reduce(0) { $0 + ($1.selectedBytes ?? $1.totalBytes) }))
                detail(String(localized:"Verified selected content",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),store.selected.contains(where:{ $0.state == "checking" || $0.state == "error" || $0.id == store.fileOperation?.torrentId }) ? "—" : SizeText.bytes(store.selected.reduce(0) { $0 + $1.downloadedBytes }))
                detail(String(localized:"Download rate",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),SizeText.rate(store.connected ? store.selected.reduce(0) { $0 + $1.downloadRate } : 0))
                detail(String(localized:"Upload rate",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),SizeText.rate(store.connected ? store.selected.reduce(0) { $0 + $1.uploadRate } : 0))
            }
            HStack(spacing:10) {
                Button { store.toggleSelected(paused:false) } label: { Label(store.resumeActionLabel,systemImage:store.selectionNeedsRetry ? "arrow.clockwise" : "play") }.disabled(!store.canResumeSelection)
                Button { store.toggleSelected(paused:true) } label: { Label(String(localized:"Pause transfers",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),systemImage:"pause") }.disabled(!store.canPauseSelection)
            }.controlSize(.small).buttonStyle(.bordered)
        }.padding(14).frame(maxWidth:.infinity,alignment:.topLeading)
    }
}

struct LibraryEmptyState: View {
    @EnvironmentObject var store: LibraryStore

    var body: some View {
        ViewThatFits(in:.vertical) {
            expanded.padding(20).fixedSize(horizontal:false,vertical:true)
            compact.padding(16).fixedSize(horizontal:false,vertical:true)
            ScrollView { compact.padding(16) }
        }.frame(maxWidth:.infinity,maxHeight:.infinity)
    }
    private var illustration: some View {
        ZStack {
            Circle().fill(Design.green.opacity(0.035)).frame(width:174,height:174)
            Circle().stroke(Design.green.opacity(0.12),style:StrokeStyle(lineWidth:1,dash:[3,6])).frame(width:142,height:142)
            Image(systemName:"tray.and.arrow.down").font(.system(size:45,weight:.ultraLight)).foregroundStyle(Design.green)
            Image(systemName:"leaf.fill").font(.system(size:18)).foregroundStyle(Design.green).padding(9).background(Design.canvas,in:Circle()).offset(x:52,y:-48)
        }.accessibilityHidden(true)
    }
    private var actions: some View {
        HStack(spacing:12) {
            Button(String(localized:"Open torrent file…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { store.openFiles() }.buttonStyle(.borderedProminent).tint(Design.green)
            Button(String(localized:"Paste magnet link",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { store.pasteMagnet() }.buttonStyle(.bordered)
        }
    }
    private var privacyNote: some View {
        Text(String(localized:"BitTorrent v1 + v2 · Files stay on your Mac",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:10)).foregroundStyle(.tertiary)
    }
    private var expanded: some View {
        VStack(spacing:0) {
            illustration
            Text(String(localized:"Something good starts here.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:21,weight:.medium,design:.rounded)).padding(.top,22)
            Text(String(localized:"Drop a torrent file or paste a magnet link.\nWe’ll take care of the rest.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:12)).foregroundStyle(.secondary).multilineTextAlignment(.center).lineSpacing(5).padding(.top,10)
            actions.controlSize(.large).padding(.top,24)
            privacyNote.padding(.top,17)
        }.multilineTextAlignment(.center)
    }
    private var compact: some View {
        HStack(spacing:18) {
            illustration.scaleEffect(0.4).frame(width:70,height:70)
            VStack(alignment:.leading,spacing:10) {
                Text(String(localized:"Something good starts here.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:17,weight:.medium,design:.rounded))
                Text(String(localized:"Drop a torrent file or paste a magnet link.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:12)).foregroundStyle(.secondary)
                actions.controlSize(.regular)
                privacyNote
            }
        }.frame(maxWidth:.infinity)
    }
}
