import SwiftUI
import UniformTypeIdentifiers

struct LibrarySearchAction { var focus: () -> Void; var focusTransfers: () -> Void }
private struct LibrarySearchKey: FocusedValueKey { typealias Value = LibrarySearchAction }
extension FocusedValues {
    var librarySearch: LibrarySearchAction? {
        get { self[LibrarySearchKey.self] }
        set { self[LibrarySearchKey.self] = newValue }
    }
}

struct LibraryView: View {
    @EnvironmentObject var store: LibraryStore
    @State private var sortOrder = [KeyPathComparator(\TorrentItem.name)]
    @State private var inspectorTab = 0
    @StateObject private var fileSessions = TorrentFilesSessions()
    @FocusState private var searchFocused: Bool
    @FocusState private var transfersFocused: Bool
    @State private var dropTargeted = false
    @AppStorage("transferColumns") private var columnCustomization = TableColumnCustomization<TorrentItem>()
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    init(inspectorTab: Int = 0) { _inspectorTab = State(initialValue:inspectorTab) }

    var body: some View {
        HSplitView {
            sidebar.frame(minWidth:190,idealWidth:210,maxWidth:260)
            content.frame(minWidth:520,maxWidth:.infinity,maxHeight:.infinity)
            if store.showInspector { inspector.frame(minWidth:250,idealWidth:280,maxWidth:360) }
        }
        .background(Design.canvas)
        .frame(minWidth:780,minHeight:560)
        .tint(Design.green)
        .toolbar {
            ToolbarItemGroup(placement:.navigation) {
                Button { store.beginImport() } label: { Label("Add torrent",systemImage:"plus") }.disabled(store.hasFileOperation).help("Add a torrent (⌘N)")
                Button { store.openFiles() } label: { Label("Open torrent file",systemImage:"folder.badge.plus") }.disabled(store.hasFileOperation).help("Open a torrent file (⌘O)")
            }
            ToolbarItemGroup(placement:.primaryAction) {
                Button { store.toggleSelected(paused:false) } label: { Label(store.resumeActionLabel,systemImage:store.resumeActionLabel == "Retry" ? "arrow.clockwise" : "play.fill") }.disabled(!store.canResumeSelection).help("Resume paused transfers or retry errors without changing saved pause intent")
                Button { store.toggleSelected(paused:true) } label: { Label("Pause",systemImage:"pause.fill") }.disabled(!store.canPauseSelection).help("Pause selected transfers")
                Button { store.requestRemoval() } label: { Label("Remove",systemImage:"trash") }.disabled(!store.canRemoveSelection).help("Remove selected transfers")
                Divider()
                Button { withAnimation(reduceMotion ? nil : .easeInOut(duration:0.18)) { store.showInspector.toggle() } } label: { Label("Toggle inspector",systemImage:"sidebar.right") }.help("Show or hide transfer details (⌥⌘I)")
            }
        }
        .searchable(text:$store.search,placement:.toolbar,prompt:"Search transfers")
        .searchFocused($searchFocused)
        .onChange(of:store.showRemoval) { old, new in if old && !new { focusTransfers() } }
        .onSubmit(of:.search) { focusTransfers() }
        .sheet(isPresented:$store.showImport,onDismiss:focusTransfers) { ImportView().environmentObject(store) }
        .confirmationDialog("Remove \(store.removalTargets.count == 1 ? "this transfer" : "\(store.removalTargets.count) transfers")?",isPresented:$store.showRemoval,titleVisibility:.visible) {
            Button("Remove from Library") { store.removeConfirmed(deleteFiles:false) }
            Button("Remove and Delete Downloaded Files",role:.destructive) { store.removeConfirmed(deleteFiles:true) }
        } message: {
            Text(store.removalTargets.count == 1 ? "“\(store.removalTargets.first?.name ?? "This transfer")” will be removed. Its files stay on your Mac unless you choose to delete them." : "These \(store.removalTargets.count) transfers will be removed. Their files stay on your Mac unless you choose to delete them.")
        }
        .focusedSceneValue(\.librarySearch,LibrarySearchAction(focus:{ searchFocused = true },focusTransfers:focusTransfers))
        .onDrop(of:[UTType.fileURL.identifier, UTType.url.identifier, UTType.plainText.identifier],isTargeted:$dropTargeted,perform:handleDrop)
        .overlay { if dropTargeted { RoundedRectangle(cornerRadius:8).strokeBorder(Design.green,lineWidth:2).padding(3).allowsHitTesting(false).accessibilityHidden(true) } }
    }
    private var sidebar: some View {
        VStack(alignment:.leading,spacing:0) {
            HStack(spacing:10) { BrandMark(); VStack(alignment:.leading,spacing:2) { Text("SeedHop").font(.system(size:20,weight:.semibold,design:.rounded)); Text("Room for good things.").font(.system(size:10)).foregroundStyle(.secondary) } }
                .padding(.horizontal,20).padding(.top,27).padding(.bottom,34)
            SectionEyebrow(title:"Library").padding(.horizontal,22).padding(.bottom,12)
            VStack(spacing:4) {
                ForEach(LibraryFilter.allCases) { filter in
                    let selected = store.filter == filter
                    Button { store.filter = filter } label: {
                        HStack(spacing:10) {
                            Image(systemName:filter.symbol).font(.system(size:14,weight:.medium)).frame(width:19)
                            Text(filter.rawValue).font(.system(size:12,weight:selected ? .semibold : .regular))
                            Spacer(minLength:0)
                            Text("\(store.count(in:filter))").font(.system(size:10,weight:.medium,design:.rounded)).monospacedDigit().foregroundStyle(selected ? Design.green : Design.secondary)
                        }.foregroundStyle(selected ? Design.green : Color.primary.opacity(0.75)).padding(.horizontal,12).frame(height:35)
                            .background(selected ? Design.green.opacity(0.09) : .clear,in:RoundedRectangle(cornerRadius:8))
                            .contentShape(Rectangle())
                    }.buttonStyle(.plain).accessibilityAddTraits(selected ? .isSelected : [])
                }
            }.padding(.horizontal,10)
            Spacer()
            VStack(alignment:.leading,spacing:12) {
                Divider()
                HStack(spacing:8) {
                    Image(systemName:"externaldrive").font(.system(size:15)).foregroundStyle(.secondary)
                    VStack(alignment:.leading,spacing:3) {
                        Text(URL(fileURLWithPath:store.downloadDirectory).lastPathComponent).font(.system(size:11,weight:.medium))
                        Text("Default download folder").font(.system(size:10)).foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button { NSWorkspace.shared.selectFile(nil,inFileViewerRootedAtPath:store.downloadDirectory) } label: { Image(systemName:"arrow.up.right") }.buttonStyle(.plain).foregroundStyle(.secondary).help("Show download folder in Finder").accessibilityLabel("Show download folder in Finder")
                }
                HStack(spacing:6) { Circle().fill(store.connected ? Design.green : Design.amber).frame(width:5,height:5); Text(store.switching ? "Switching engine…" : store.connected ? (store.backgroundEnabled ? "Running in background" : "Engine connected") : store.connecting ? "Connecting to engine…" : "Engine disconnected").font(.system(size:10)).foregroundStyle(.secondary) }.accessibilityElement(children:.combine)
            }.padding(18)
        }.frame(maxWidth:.infinity,maxHeight:.infinity).background(Design.sidebar)
    }
    private var content: some View {
        VStack(spacing:0) {
            if store.designPreview {
                HStack { Image(systemName:"paintpalette"); Text("DESIGN PREVIEW · SAMPLE DATA").tracking(1.2); Spacer() }.font(.system(size:9,weight:.semibold)).foregroundStyle(Design.green).padding(.horizontal,26).padding(.vertical,9).background(Design.green.opacity(0.06))
            }
            VStack(alignment:.leading,spacing:22) {
                HStack(alignment:.center) {
                    VStack(alignment:.leading,spacing:7) {
                        Text(store.filter == .all ? "Your library" : store.filter.rawValue).font(.system(size:27,weight:.semibold,design:.rounded)).tracking(-0.65)
                        Text(store.activitySummary)
                            .font(.system(size:12)).foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button { store.beginImport() } label: { Label("Add torrent",systemImage:"plus").font(.system(size:12,weight:.semibold)).padding(.horizontal,5).padding(.vertical,3) }
                        .buttonStyle(.borderedProminent).controlSize(.large).tint(Design.green).disabled(store.hasFileOperation)
                }
                if !store.torrents.isEmpty {
                    HStack(spacing:18) {
                        Metric(title:"Download",value:SizeText.rate(store.downloadRate),icon:"arrow.down")
                        Rectangle().fill(Design.line).frame(width:1,height:35)
                        Metric(title:"Upload",value:SizeText.rate(store.uploadRate),icon:"arrow.up")
                        Rectangle().fill(Design.line).frame(width:1,height:35)
                        Metric(title:"Downloaded",value:store.torrents.contains(where:{ $0.state == "checking" || $0.state == "error" || $0.id == store.fileOperation?.torrentId }) ? "—" : SizeText.bytes(store.totalDownloaded),icon:"checkmark.circle")
                    }.padding(18).background(Design.card.opacity(0.7),in:RoundedRectangle(cornerRadius:12)).overlay(RoundedRectangle(cornerRadius:12).stroke(Design.line,lineWidth:1))
                }
                if store.switching { HStack { ProgressView().controlSize(.small); Text("Moving transfers to the \(store.backgroundEnabled ? "foreground" : "background") engine…").font(.system(size:12)) }.padding(12) }
                if let recovery = store.storageRecovery { storageRecoveryNotice(recovery) }
                if let operation = store.fileOperation { fileOperationProgress(operation) }
                else if !store.storageRequests.isEmpty {
                    HStack(spacing:10) { ProgressView().controlSize(.small); Text("Preparing file change…").font(.system(size:12)) }
                }
                if let error = store.error {
                    InlineNotice(title:store.connected ? "Something needs attention" : "Let’s get you connected",message:error,retry:store.connected ? nil : { Task { store.error = nil; await store.start() } },dismiss:{ store.error = nil })
                }
            }.padding(26)
            if store.torrents.isEmpty && store.storageRecovery != nil {
                ContentUnavailableView("Retained files need attention",systemImage:"exclamationmark.circle",description:Text("Review the cleanup notice above. Your remaining files have been preserved."))
            }
            else if store.torrents.isEmpty { emptyState }
            else if store.visibleTorrents.isEmpty {
                if !store.search.isEmpty { ContentUnavailableView.search(text:store.search).frame(maxHeight:.infinity) }
                else {
                    ContentUnavailableView {
                        Label {
                            Text(store.filter == .attention && store.hasUnscopedRecovery ? "Retained files need attention" : store.filter.emptyTitle).foregroundStyle(.primary)
                        } icon: {
                            Image(systemName:store.filter == .attention ? (store.hasUnscopedRecovery ? "exclamationmark.circle" : "checkmark.circle") : store.filter.symbol).foregroundStyle(store.filter == .attention ? (store.hasUnscopedRecovery ? Design.amber : Design.green) : Design.secondary)
                        }
                    } description: {
                        Text(store.filter == .attention && store.hasUnscopedRecovery ? "Review the cleanup notice above. Your other transfers are still in the library." : "Your other transfers are still in the library.")
                    } actions: {
                        Button("Show all transfers") { store.filter = .all }
                    }.frame(maxHeight:.infinity)
                }
            }
            else { transferTable }
            Divider()
            HStack(spacing:9) {
                Text("\(store.visibleTorrents.count) transfers").font(.system(size:10)).foregroundStyle(.secondary)
                if !store.selection.isEmpty { Text("· \(store.selection.count) selected").font(.system(size:10)).foregroundStyle(.secondary) }
                if !store.busyTransfers.isEmpty { ProgressView().controlSize(.mini); Text("Saving changes…").font(.system(size:10)).foregroundStyle(.secondary) }
                Spacer()
                Image(systemName:"leaf").font(.system(size:10)).foregroundStyle(Design.green)
                Text("A little sharing goes a long way.").font(.system(size:10)).foregroundStyle(.secondary)
            }.padding(.horizontal,24).frame(height:34)
        }.background(Design.canvas)
    }
    private func fileOperationProgress(_ operation: FileOperationItem) -> some View {
        VStack(alignment:.leading,spacing:9) {
            HStack(alignment:.top,spacing:12) {
                VStack(alignment:.leading,spacing:4) {
                    Text(store.connected ? operation.title : "Last known: \(operation.title)").font(.system(size:12,weight:.semibold))
                    Text(store.torrents.first(where:{ $0.id == operation.torrentId })?.name ?? "Transfer files")
                        .font(.system(size:11)).lineLimit(1).truncationMode(.middle)
                }
                Spacer(minLength:8)
                if ["preparing", "copying"].contains(operation.phase) {
                    Button(operation.cancelRequested || store.cancellingOperations.contains(operation.id) ? "Cancelling…" : "Cancel") {
                        store.cancelFileOperation(operation)
                    }.disabled(!store.canCancelFileOperation(operation)).accessibilityLabel("Cancel file change")
                } else { ProgressView().controlSize(.small).accessibilityLabel(operation.title) }
            }
            if operation.phase == "copying" && operation.totalBytes > 0 {
                TransferProgress(value:Double(operation.completedBytes) / Double(max(operation.completedBytes,operation.totalBytes)))
                    .accessibilityLabel("Files copied")
                    .accessibilityValue("\(SizeText.bytes(operation.completedBytes)) of \(SizeText.bytes(operation.totalBytes))")
            }
            Text(store.connected ? operation.detail : "Reconnect to see current file-operation progress.").font(.system(size:10)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true)
        }.padding(14).frame(maxWidth:.infinity,alignment:.leading)
            .background(Design.card,in:RoundedRectangle(cornerRadius:10))
            .overlay(RoundedRectangle(cornerRadius:10).stroke(Design.line))
            .announceAccessibility(store.connected ? "File change. \(operation.title)" : nil,priority:.medium)
    }
    private func storageRecoveryNotice(_ notice: StorageRecoveryItem) -> some View {
        VStack(alignment:.leading,spacing:10) {
            Text("File cleanup needs attention").font(.system(size:12,weight:.semibold))
            Text(notice.message).font(.system(size:11)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true).textSelection(.enabled)
            HStack {
                Button(store.resolvingStorageRecovery ? "Checking…" : "Retry Cleanup") { store.resolveStorageRecovery(notice,keepCopies:false) }
                    .disabled(!store.canResolveStorageRecovery(notice))
                if notice.canKeepCopies {
                    Button("Keep Extra Copies") { store.resolveStorageRecovery(notice,keepCopies:true) }
                        .disabled(!store.canResolveStorageRecovery(notice))
                        .help("Keep all remaining files and finish tracking this cleanup.")
                }
                Spacer(minLength:0)
            }
            HStack {
                Button("Show Original Folder") { NSWorkspace.shared.selectFile(nil,inFileViewerRootedAtPath:notice.originalDirectory) }
                if notice.originalDirectory != notice.destinationDirectory {
                    Button("Show Destination") { NSWorkspace.shared.selectFile(nil,inFileViewerRootedAtPath:notice.destinationDirectory) }
                }
            }.font(.system(size:10))
        }.padding(14).frame(maxWidth:.infinity,alignment:.leading)
            .background(Design.amber.opacity(0.06),in:RoundedRectangle(cornerRadius:10))
            .overlay(RoundedRectangle(cornerRadius:10).stroke(Design.amber.opacity(0.25)))
            .announceAccessibility("File cleanup needs attention. \(notice.message)")
    }
    private var transferTable: some View {
        Table(store.visibleTorrents.sorted(using:sortOrder),selection:$store.selection,sortOrder:$sortOrder,columnCustomization:$columnCustomization) {
            TableColumn("Name",value:\.name) { torrent in
                let status = store.presentation(torrent)
                HStack(spacing:12) {
                    Image(systemName:Design.symbol(torrent.name)).font(.system(size:18,weight:.light)).foregroundStyle(Design.stateColor(torrent.state)).frame(width:36,height:39).background(Design.stateColor(torrent.state).opacity(0.065),in:RoundedRectangle(cornerRadius:8))
                    VStack(alignment:.leading,spacing:7) {
                        Text(torrent.name).font(.system(size:12,weight:.medium)).lineLimit(1)
                        HStack(spacing:7) {
                            if let progress = status.progress {
                                TransferProgress(value:progress,color:Design.stateColor(status.colorState)).frame(maxWidth:110)
                                Text("\(Int(progress * 100))% \(status.progressTitle.lowercased())").font(.system(size:9)).monospacedDigit().foregroundStyle(.secondary)
                            } else { Text(status.isError ? "Existing files will be checked on retry" : "File change in progress").font(.system(size:9)).foregroundStyle(.secondary).lineLimit(1) }
                            Text("· \(SizeText.bytes(torrent.totalBytes))").font(.system(size:9)).foregroundStyle(.secondary)
                        }
                    }
                }.padding(.vertical,10).accessibilityElement(children:.ignore).accessibilityLabel("\(torrent.name), \(status.accessibleLabel), \(status.progressDescription)")
            }.width(min:190,ideal:240).customizationID("name").disabledCustomizationBehavior(.visibility)
            TableColumn("Status") { StateBadge(torrent:$0,presentation:store.presentation($0)) }.width(min:118,ideal:140).customizationID("status")
            TableColumn("↓ Download",value:\.downloadRate) { Text(SizeText.rate(store.connected ? $0.downloadRate : 0)).font(.system(size:11)).monospacedDigit().foregroundStyle(.secondary) }.width(min:70,ideal:80).customizationID("download")
            TableColumn("↑ Upload",value:\.uploadRate) { Text(SizeText.rate(store.connected ? $0.uploadRate : 0)).font(.system(size:11)).monospacedDigit().foregroundStyle(.secondary) }.width(min:70,ideal:80).customizationID("upload")
            TableColumn("Peers",value:\.peers) { Text("\($0.peers)").font(.system(size:11)).monospacedDigit().foregroundStyle(.secondary) }.width(50).customizationID("peers").defaultVisibility(.hidden)
            TableColumn("Size",value:\.totalBytes) { Text(SizeText.bytes($0.totalBytes)).font(.system(size:11)).monospacedDigit().foregroundStyle(.secondary) }.width(75).customizationID("size").defaultVisibility(.hidden)
        }
        .tableStyle(.inset(alternatesRowBackgrounds:false))
        .focused($transfersFocused)
        .accessibilityLabel("Transfers")
        .contextMenu(forSelectionType:String.self) { ids in
            Button("Move to Front of Queue") { store.selection = ids; store.moveSelectedToFront() }.disabled(!canAct(on:ids) || ids.count != 1)
            Button("Move Files…") { store.selection = ids; store.chooseRelocation() }.disabled(!canAct(on:ids) || store.storageRecovery != nil || ids.count != 1 || store.torrents.contains { ids.contains($0.id) && $0.state == "checking" })
            Button("Force Recheck") { store.selection = ids; store.recheckSelected() }.disabled(!canAct(on:ids) || store.torrents.contains { ids.contains($0.id) && ($0.state == "checking" || store.hasPendingCleanup($0.id)) })
            Button(store.resumeLabel(for:store.torrents.filter { ids.contains($0.id) })) { store.selection = ids; store.toggleSelected(paused:false) }.disabled(!canAct(on:ids) || !store.torrents.contains { ids.contains($0.id) && store.canResumeOrRetry($0) })
            Button("Pause") { store.selection = ids; store.toggleSelected(paused:true) }.disabled(!canAct(on:ids) || !store.torrents.contains { ids.contains($0.id) && !$0.isPaused })
            Divider()
            Button("Show in Finder") { if let item = store.torrents.first(where:{ ids.contains($0.id) }) { NSWorkspace.shared.selectFile(nil,inFileViewerRootedAtPath:item.destination) } }.disabled(ids.isEmpty)
            Button("Remove…",role:.destructive) { store.selection = ids; store.requestRemoval() }.disabled(!canAct(on:ids) || ids.contains(where:store.hasPendingCleanup))
        } primaryAction: { ids in store.selection = ids; store.showInspector = true }
    }
    private var emptyState: some View {
        VStack(spacing:0) {
            Spacer()
            ZStack {
                Circle().fill(Design.green.opacity(0.035)).frame(width:174,height:174)
                Circle().stroke(Design.green.opacity(0.12),style:StrokeStyle(lineWidth:1,dash:[3,6])).frame(width:142,height:142)
                Image(systemName:"tray.and.arrow.down").font(.system(size:45,weight:.ultraLight)).foregroundStyle(Design.green)
                Image(systemName:"leaf.fill").font(.system(size:18)).foregroundStyle(Design.green).padding(9).background(Design.canvas,in:Circle()).offset(x:52,y:-48)
            }.accessibilityHidden(true)
            Text("Something good starts here.").font(.system(size:21,weight:.medium,design:.rounded)).padding(.top,22)
            Text("Drop a torrent file or paste a magnet link.\nWe’ll take care of the rest.").font(.system(size:12)).foregroundStyle(.secondary).multilineTextAlignment(.center).lineSpacing(5).padding(.top,10)
            HStack(spacing:12) {
                Button("Open torrent file…") { store.openFiles() }.buttonStyle(.borderedProminent).tint(Design.green)
                Button("Paste magnet link") { store.pasteMagnet() }.buttonStyle(.bordered)
            }.controlSize(.large).padding(.top,24)
            Text("BitTorrent v1 + v2 · Files stay on your Mac").font(.system(size:10)).foregroundStyle(.tertiary).padding(.top,17)
            Spacer(); Spacer().frame(height:45)
        }.frame(maxWidth:.infinity,maxHeight:.infinity)
    }
    private var inspector: some View {
        VStack(alignment:.leading,spacing:0) {
            HStack { SectionEyebrow(title:"Transfer details"); Spacer(); Button { store.showInspector = false } label: { Image(systemName:"xmark").font(.system(size:10)) }.buttonStyle(.plain).foregroundStyle(.secondary).accessibilityLabel("Close inspector") }.padding(20)
            Divider()
            if store.selected.count > 1 { multipleSelectionInspector }
            else if let item = store.selected.first {
                let status = store.presentation(item)
                if inspectorTab == 1 {
                    VStack(alignment:.leading,spacing:10) {
                        HStack(alignment:.top,spacing:8) {
                            Image(systemName:Design.symbol(item.name)).foregroundStyle(.secondary)
                            Text(item.name).font(.system(size:13,weight:.semibold)).lineLimit(2).help(item.name)
                                .accessibilityLabel(item.name)
                        }
                        StateBadge(torrent:item,presentation:status)
                        if let progress = status.progress { TransferProgress(value:progress).accessibilityLabel(status.progressTitle) }
                        inspectorTabs
                    }.padding(14)
                    Divider()
                    TorrentFilesView(torrent:item,session:fileSessions.session(for:item))
                        .frame(maxWidth:.infinity,maxHeight:.infinity)
                } else {
                ScrollView {
                    VStack(alignment:.leading,spacing:22) {
                        VStack(alignment:.leading,spacing:13) {
                            Image(systemName:Design.symbol(item.name)).font(.system(size:29,weight:.ultraLight)).foregroundStyle(Design.green).frame(width:60,height:65).background(Design.green.opacity(0.07),in:RoundedRectangle(cornerRadius:13))
                            Text(item.name).font(.system(size:16,weight:.semibold,design:.rounded)).textSelection(.enabled).fixedSize(horizontal:false,vertical:true)
                            StateBadge(torrent:item,presentation:status)
                            if !status.isError || status.hasRecovery {
                                Text(status.detail).font(.system(size:11)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true)
                            }
                        }
                        if let progress = status.progress {
                            VStack(alignment:.leading,spacing:10) {
                                HStack { Text(status.progressTitle).foregroundStyle(.secondary); Spacer(); Text("\(Int(progress * 100))%").fontWeight(.medium).monospacedDigit() }.font(.system(size:11))
                                TransferProgress(value:progress).accessibilityLabel(status.progressTitle)
                                Text("\(SizeText.bytes(status.completedBytes)) of \(SizeText.bytes(status.totalBytes))").font(.system(size:10)).foregroundStyle(.secondary)
                            }
                        }
                        if status.isQueued {
                            Button("Move to Front of Queue") { store.moveToFront(item) }.disabled(!store.canChangeTransfers || store.busyTransfers.contains(item.id))
                        }
                        if status.isError && !status.hasRecovery { transferFailure(item,status:status) }
                        inspectorTabs
                        if inspectorTab == 0 { overview(item) }
                        else {
                            VStack(alignment:.leading,spacing:10) { SectionEyebrow(title:"Trackers"); if item.trackers.isEmpty { Text("No trackers are listed for this transfer.").font(.system(size:11)).foregroundStyle(.secondary) }; ForEach(item.trackers,id:\.self) { Text(redactTracker($0)).font(.system(size:10,design:.monospaced)).textSelection(.enabled) } }
                        }
                    }.padding(20)
                }
                Spacer(minLength:0)
                }
                Divider()
                HStack {
                    Button { if item.state == "error" { store.retryTransfer(item) } else { store.toggleSelected(paused:!item.isPaused) } } label: { Label(item.state == "error" ? "Retry" : item.isPaused ? "Resume" : "Pause",systemImage:item.state == "error" ? "arrow.clockwise" : item.isPaused ? "play" : "pause").frame(maxWidth:.infinity) }.buttonStyle(.bordered).controlSize(.large).disabled(item.state == "error" ? !store.canRetryTransfer(item) : item.isPaused ? !store.canResumeSelection : !store.canPauseSelection)
                    Button { store.revealDestination(item) } label: { Image(systemName:"folder") }.controlSize(.large).help("Show the destination or its closest available parent in Finder").accessibilityLabel("Show transfer folder in Finder")
                }.padding(16)
            } else {
                Spacer()
                VStack(spacing:12) { Image(systemName:"sidebar.right").font(.system(size:29,weight:.ultraLight)); Text("A closer look").font(.system(size:15,weight:.medium,design:.rounded)); Text("Select a transfer to see its\nprogress, files, and details.").font(.system(size:11)).multilineTextAlignment(.center) }.foregroundStyle(.secondary).frame(maxWidth:.infinity)
                Spacer()
            }
        }.background(Design.card.opacity(0.55))
    }
    private func transferFailure(_ item: TorrentItem, status: TransferPresentation) -> some View {
        VStack(alignment:.leading,spacing:10) {
            Label(status.errorTitle,systemImage:"exclamationmark.circle.fill").font(.system(size:12,weight:.semibold)).foregroundStyle(Design.amber)
            Text(status.detail).font(.system(size:11)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true)
            HStack {
                Button("Retry") { store.retryTransfer(item) }.disabled(!store.canRetryTransfer(item))
                Button("Show Folder") { store.revealDestination(item) }.help("Open the destination or its closest available parent in Finder")
            }.controlSize(.small)
            if status.canMoveAfterError {
                Button("Move Files…") { store.chooseRelocation(item) }.controlSize(.small).disabled(!store.canChangeTransfers || store.busyTransfers.contains(item.id))
            }
            if let error = item.error {
                DisclosureGroup("Error details") { Text(error).font(.system(size:10,design:.monospaced)).foregroundStyle(.secondary).textSelection(.enabled).fixedSize(horizontal:false,vertical:true) }.font(.system(size:10))
            }
        }.padding(12).background(Design.amber.opacity(0.06),in:RoundedRectangle(cornerRadius:10))
            .announceAccessibility("\(item.name). \(status.errorTitle). \(status.detail)")
    }
    private func overview(_ t: TorrentItem) -> some View {
        let status = store.presentation(t)
        return VStack(alignment:.leading,spacing:17) {
            SectionEyebrow(title:"Activity")
            detail(status.progressTitle,status.progress == nil ? "—" : SizeText.bytes(status.completedBytes))
            detail("Uploaded",["downloading", "waiting", "seeding"].contains(t.state) ? SizeText.bytes(t.uploadedBytes) : "—")
            detail(store.connected ? "Connected peers" : "Last known peers","\(t.peers)")
            detail("Share ratio",status.progressTitle != "Downloaded" || t.downloadedBytes == 0 || !["downloading", "waiting", "seeding"].contains(t.state) ? "—" : String(format:"%.2f",Double(t.uploadedBytes)/Double(t.downloadedBytes)))
            Divider()
            SectionEyebrow(title:"Location")
            Label(URL(fileURLWithPath:t.destination).lastPathComponent,systemImage:"folder").font(.system(size:11))
            Text(t.destination.replacingOccurrences(of:NSHomeDirectory(),with:"~")).font(.system(size:10)).foregroundStyle(.secondary).textSelection(.enabled).fixedSize(horizontal:false,vertical:true)
            Divider()
            SectionEyebrow(title:"Info hash")
            Text(t.id).font(.system(size:9,design:.monospaced)).foregroundStyle(.secondary).textSelection(.enabled).lineLimit(3)
        }
    }
    private func detail(_ label: String, _ value: String) -> some View { HStack { Text(label).foregroundStyle(.secondary); Spacer(); Text(value).monospacedDigit() }.font(.system(size:11)) }
    private var inspectorTabs: some View {
        Picker("Details",selection:$inspectorTab) { Text("Overview").tag(0); Text("Files").tag(1); Text("Trackers").tag(2) }.pickerStyle(.segmented).labelsHidden()
    }
    private func redactTracker(_ raw: String) -> String {
        guard let url = URL(string:raw) else { return "Tracker" }
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
        VStack(alignment:.leading,spacing:22) {
            Image(systemName:"square.stack.3d.up").font(.system(size:30,weight:.ultraLight)).foregroundStyle(Design.green)
            Text("\(store.selected.count) transfers selected").font(.system(size:19,weight:.semibold,design:.rounded))
            Text("Actions apply to the selected transfers. Select one transfer to view its files and details.").font(.system(size:12)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true)
            Divider()
            detail("Total size",SizeText.bytes(store.selected.reduce(0) { $0 + $1.totalBytes }))
            detail("Downloaded",store.selected.contains(where:{ $0.state == "checking" || $0.state == "error" || $0.id == store.fileOperation?.torrentId }) ? "—" : SizeText.bytes(store.selected.reduce(0) { $0 + $1.downloadedBytes }))
            detail("Download",SizeText.rate(store.connected ? store.selected.reduce(0) { $0 + $1.downloadRate } : 0))
            detail("Upload",SizeText.rate(store.connected ? store.selected.reduce(0) { $0 + $1.uploadRate } : 0))
            Spacer()
            VStack(spacing:10) {
                Button { store.toggleSelected(paused:false) } label: { Label(store.resumeActionLabel,systemImage:store.resumeActionLabel == "Retry" ? "arrow.clockwise" : "play").frame(maxWidth:.infinity) }.disabled(!store.canResumeSelection)
                Button { store.toggleSelected(paused:true) } label: { Label("Pause transfers",systemImage:"pause").frame(maxWidth:.infinity) }.disabled(!store.canPauseSelection)
            }.controlSize(.large).buttonStyle(.bordered)
        }.padding(20).frame(maxWidth:.infinity,maxHeight:.infinity,alignment:.topLeading)
    }
}
