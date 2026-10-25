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
                Button { store.beginImport() } label: { Label("Add torrent",systemImage:"plus") }.help("Add a torrent (⌘N)")
                Button { store.openFiles() } label: { Label("Open torrent file",systemImage:"folder.badge.plus") }.help("Open a torrent file (⌘O)")
            }
            ToolbarItemGroup(placement:.primaryAction) {
                Button { store.toggleSelected(paused:false) } label: { Label("Resume",systemImage:"play.fill") }.disabled(!store.canResumeSelection).help("Resume selected paused transfers")
                Button { store.toggleSelected(paused:true) } label: { Label("Pause",systemImage:"pause.fill") }.disabled(!store.canPauseSelection).help("Pause selected transfers")
                Button { store.requestRemoval() } label: { Label("Remove",systemImage:"trash") }.disabled(!store.canActOnSelection).help("Remove selected transfers")
                Divider()
                Button { withAnimation(reduceMotion ? nil : .easeInOut(duration:0.18)) { store.showInspector.toggle() } } label: { Label("Toggle inspector",systemImage:"sidebar.right") }.help("Show or hide transfer details (⌥⌘I)")
            }
        }
        .searchable(text:$store.search,placement:.toolbar,prompt:"Search transfers")
        .searchFocused($searchFocused)
        .onSubmit(of:.search) { focusTransfers() }
        .sheet(isPresented:$store.showImport) { ImportView().environmentObject(store) }
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
                            Text("\(store.torrents.filter { filter.includes($0) }.count)").font(.system(size:10,weight:.medium,design:.rounded)).monospacedDigit().foregroundStyle(selected ? Design.green : Design.secondary)
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
                        Text(store.torrents.isEmpty ? "A fresh start for your next download." : !store.connected ? "Showing the last update. Reconnect for live activity." : "\(store.torrents.filter(\.isActive).count) downloading · \(store.torrents.filter { $0.state == "seeding" }.count) giving back")
                            .font(.system(size:12)).foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button { store.beginImport() } label: { Label("Add torrent",systemImage:"plus").font(.system(size:12,weight:.semibold)).padding(.horizontal,5).padding(.vertical,3) }
                        .buttonStyle(.borderedProminent).controlSize(.large).tint(Design.green)
                }
                if !store.torrents.isEmpty {
                    HStack(spacing:18) {
                        Metric(title:"Download",value:SizeText.rate(store.downloadRate),icon:"arrow.down")
                        Rectangle().fill(Design.line).frame(width:1,height:35)
                        Metric(title:"Upload",value:SizeText.rate(store.uploadRate),icon:"arrow.up")
                        Rectangle().fill(Design.line).frame(width:1,height:35)
                        Metric(title:"Downloaded",value:SizeText.bytes(store.totalDownloaded),icon:"checkmark.circle")
                    }.padding(18).background(Design.card.opacity(0.7),in:RoundedRectangle(cornerRadius:12)).overlay(RoundedRectangle(cornerRadius:12).stroke(Design.line,lineWidth:1))
                }
                if store.switching { HStack { ProgressView().controlSize(.small); Text("Moving transfers to the \(store.backgroundEnabled ? "foreground" : "background") engine…").font(.system(size:12)) }.padding(12) }
                if let error = store.error {
                    InlineNotice(title:store.connected ? "Something needs attention" : "Let’s get you connected",message:error,retry:store.connected ? nil : { Task { store.error = nil; await store.start() } },dismiss:{ store.error = nil })
                }
            }.padding(26)
            if store.torrents.isEmpty { emptyState }
            else if store.visibleTorrents.isEmpty {
                if !store.search.isEmpty { ContentUnavailableView.search(text:store.search).frame(maxHeight:.infinity) }
                else {
                    ContentUnavailableView {
                        Label {
                            Text(store.filter.emptyTitle).foregroundStyle(.primary)
                        } icon: {
                            Image(systemName:store.filter == .attention ? "checkmark.circle" : store.filter.symbol).foregroundStyle(store.filter == .attention ? Design.green : Design.secondary)
                        }
                    } description: {
                        Text("Your other transfers are still in the library.")
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
    private var transferTable: some View {
        Table(store.visibleTorrents.sorted(using:sortOrder),selection:$store.selection,sortOrder:$sortOrder,columnCustomization:$columnCustomization) {
            TableColumn("Name",value:\.name) { torrent in
                HStack(spacing:12) {
                    Image(systemName:Design.symbol(torrent.name)).font(.system(size:18,weight:.light)).foregroundStyle(Design.stateColor(torrent.state)).frame(width:36,height:39).background(Design.stateColor(torrent.state).opacity(0.065),in:RoundedRectangle(cornerRadius:8))
                    VStack(alignment:.leading,spacing:7) {
                        Text(torrent.name).font(.system(size:12,weight:.medium)).lineLimit(1)
                        HStack(spacing:7) {
                            TransferProgress(value:torrent.progress,color:Design.stateColor(torrent.state)).frame(maxWidth:110)
                            Text("\(Int(torrent.progress * 100))%").font(.system(size:9)).monospacedDigit().foregroundStyle(.secondary)
                            Text("· \(SizeText.bytes(torrent.totalBytes))").font(.system(size:9)).foregroundStyle(.secondary)
                        }
                    }
                }.padding(.vertical,10).accessibilityElement(children:.combine).accessibilityLabel("\(torrent.name), \(torrent.statusLabel), \(Int(torrent.progress * 100)) percent")
            }.width(min:190,ideal:240).customizationID("name").disabledCustomizationBehavior(.visibility)
            TableColumn("Status") { StateBadge(torrent:$0) }.width(min:100,ideal:112).customizationID("status")
            TableColumn("↓ Download",value:\.downloadRate) { Text(SizeText.rate($0.downloadRate)).font(.system(size:11)).monospacedDigit().foregroundStyle(.secondary) }.width(min:70,ideal:80).customizationID("download")
            TableColumn("↑ Upload",value:\.uploadRate) { Text(SizeText.rate($0.uploadRate)).font(.system(size:11)).monospacedDigit().foregroundStyle(.secondary) }.width(min:70,ideal:80).customizationID("upload")
            TableColumn("Peers",value:\.peers) { Text("\($0.peers)").font(.system(size:11)).monospacedDigit().foregroundStyle(.secondary) }.width(50).customizationID("peers").defaultVisibility(.hidden)
            TableColumn("Size",value:\.totalBytes) { Text(SizeText.bytes($0.totalBytes)).font(.system(size:11)).monospacedDigit().foregroundStyle(.secondary) }.width(75).customizationID("size").defaultVisibility(.hidden)
        }
        .tableStyle(.inset(alternatesRowBackgrounds:false))
        .focused($transfersFocused)
        .accessibilityLabel("Transfers")
        .contextMenu(forSelectionType:String.self) { ids in
            Button("Move to Front of Queue") { store.selection = ids; store.moveSelectedToFront() }.disabled(!canAct(on:ids) || ids.count != 1)
            Button("Move Files…") { store.selection = ids; store.chooseRelocation() }.disabled(!canAct(on:ids) || ids.count != 1 || store.torrents.contains { ids.contains($0.id) && $0.state == "checking" })
            Button("Force Recheck") { store.selection = ids; store.recheckSelected() }.disabled(!canAct(on:ids) || store.torrents.contains { ids.contains($0.id) && $0.state == "checking" })
            Button("Resume") { store.selection = ids; store.toggleSelected(paused:false) }.disabled(!canAct(on:ids) || !store.torrents.contains { ids.contains($0.id) && ($0.isPaused || $0.state == "error") })
            Button("Pause") { store.selection = ids; store.toggleSelected(paused:true) }.disabled(!canAct(on:ids) || !store.torrents.contains { ids.contains($0.id) && !$0.isPaused })
            Divider()
            Button("Show in Finder") { if let item = store.torrents.first(where:{ ids.contains($0.id) }) { NSWorkspace.shared.selectFile(nil,inFileViewerRootedAtPath:item.destination) } }.disabled(ids.isEmpty)
            Button("Remove…",role:.destructive) { store.selection = ids; store.requestRemoval() }.disabled(!canAct(on:ids))
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
                ScrollView {
                    VStack(alignment:.leading,spacing:22) {
                        VStack(alignment:.leading,spacing:13) {
                            Image(systemName:Design.symbol(item.name)).font(.system(size:29,weight:.ultraLight)).foregroundStyle(Design.green).frame(width:60,height:65).background(Design.green.opacity(0.07),in:RoundedRectangle(cornerRadius:13))
                            Text(item.name).font(.system(size:16,weight:.semibold,design:.rounded)).textSelection(.enabled).fixedSize(horizontal:false,vertical:true)
                            StateBadge(torrent:item)
                        }
                        VStack(alignment:.leading,spacing:10) {
                            HStack { Text("Progress").foregroundStyle(.secondary); Spacer(); Text("\(Int(item.progress * 100))%").fontWeight(.medium).monospacedDigit() }.font(.system(size:11))
                            TransferProgress(value:item.progress)
                            Text("\(SizeText.bytes(item.downloadedBytes)) of \(SizeText.bytes(item.totalBytes))").font(.system(size:10)).foregroundStyle(.secondary)
                        }
                        if let error = item.error { InlineNotice(title:"Transfer needs attention",message:error) }
                        Picker("Details",selection:$inspectorTab) { Text("Overview").tag(0); Text("Files").tag(1); Text("Trackers").tag(2) }.pickerStyle(.segmented).labelsHidden()
                        if inspectorTab == 0 { overview(item) }
                        else if inspectorTab == 1 { files(item) }
                        else {
                            VStack(alignment:.leading,spacing:10) { SectionEyebrow(title:"Trackers"); if item.trackers.isEmpty { Text("Peer discovery uses DHT.").font(.system(size:11)).foregroundStyle(.secondary) }; ForEach(item.trackers,id:\.self) { Text(redactTracker($0)).font(.system(size:10,design:.monospaced)).textSelection(.enabled) } }
                        }
                    }.padding(20)
                }
                Spacer(minLength:0)
                Divider()
                HStack {
                    Button { store.toggleSelected(paused:!item.isPaused && item.state != "error") } label: { Label(item.isPaused || item.state == "error" ? "Resume" : "Pause",systemImage:item.isPaused || item.state == "error" ? "play" : "pause").frame(maxWidth:.infinity) }.buttonStyle(.bordered).controlSize(.large).disabled(!store.canActOnSelection)
                    Button { NSWorkspace.shared.selectFile(nil,inFileViewerRootedAtPath:item.destination) } label: { Image(systemName:"folder") }.controlSize(.large).help("Show in Finder").accessibilityLabel("Show transfer folder in Finder")
                }.padding(16)
            } else {
                Spacer()
                VStack(spacing:12) { Image(systemName:"sidebar.right").font(.system(size:29,weight:.ultraLight)); Text("A closer look").font(.system(size:15,weight:.medium,design:.rounded)); Text("Select a transfer to see its\nprogress, files, and details.").font(.system(size:11)).multilineTextAlignment(.center) }.foregroundStyle(.secondary).frame(maxWidth:.infinity)
                Spacer()
            }
        }.background(Design.card.opacity(0.55))
    }
    private func overview(_ t: TorrentItem) -> some View {
        VStack(alignment:.leading,spacing:17) {
            SectionEyebrow(title:"Activity")
            detail("Downloaded",SizeText.bytes(t.downloadedBytes)); detail("Uploaded",SizeText.bytes(t.uploadedBytes)); detail("Connected peers","\(t.peers)")
            detail("Share ratio",t.downloadedBytes == 0 ? "—" : String(format:"%.2f",Double(t.uploadedBytes)/Double(t.downloadedBytes)))
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
    private func files(_ t: TorrentItem) -> some View {
        LazyVStack(alignment:.leading,spacing:12) {
            SectionEyebrow(title:"\(t.files.count) files")
            if store.busyTransfers.contains(t.id) { HStack { ProgressView().controlSize(.small); Text("Updating transfer…").font(.system(size:11)).foregroundStyle(.secondary) } }
            ForEach(t.files) { file in
                VStack(alignment:.leading,spacing:6) {
                Toggle(isOn:Binding(get:{file.selected},set:{ selected in var indices = Set(t.files.filter(\.selected).map(\.index)); if selected { indices.insert(file.index) } else { indices.remove(file.index) }; store.setFileSelection(t,indices:Array(indices)) })) {
                    VStack(alignment:.leading,spacing:4) { Text(file.name).font(.system(size:11)).lineLimit(2); Text(SizeText.bytes(file.size)).font(.system(size:10)).foregroundStyle(.secondary) }
                }.toggleStyle(.checkbox).disabled(!store.canChangeTransfers || store.busyTransfers.contains(t.id) || t.state == "checking" || (t.files.filter(\.selected).count == 1 && file.selected))
                HStack {
                    Menu {
                        Button("Low") { store.setPriority(t,file:file,priority:0) }
                        Button("Normal") { store.setPriority(t,file:file,priority:1) }
                        Button("High") { store.setPriority(t,file:file,priority:2) }
                    } label: {
                        Text(file.priority == 0 ? "Low" : file.priority == 2 ? "High" : "Normal").font(.system(size:11))
                    }.menuStyle(.borderlessButton).accessibilityLabel("Priority for \(file.name)").frame(width:95)
                    Spacer()
                    Button("Rename…") { store.requestRename(t,file:file) }.buttonStyle(.borderless).font(.system(size:10))
                }.padding(.leading,18).disabled(!store.canChangeTransfers || store.busyTransfers.contains(t.id) || t.state == "checking")
                }
            }
        }
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
        store.canChangeTransfers && !ids.isEmpty && store.busyTransfers.isDisjoint(with:ids)
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
            detail("Downloaded",SizeText.bytes(store.selected.reduce(0) { $0 + $1.downloadedBytes }))
            detail("Download",SizeText.rate(store.selected.reduce(0) { $0 + $1.downloadRate }))
            detail("Upload",SizeText.rate(store.selected.reduce(0) { $0 + $1.uploadRate }))
            Spacer()
            VStack(spacing:10) {
                Button { store.toggleSelected(paused:false) } label: { Label("Resume paused transfers",systemImage:"play").frame(maxWidth:.infinity) }.disabled(!store.canResumeSelection)
                Button { store.toggleSelected(paused:true) } label: { Label("Pause transfers",systemImage:"pause").frame(maxWidth:.infinity) }.disabled(!store.canPauseSelection)
            }.controlSize(.large).buttonStyle(.bordered)
        }.padding(20).frame(maxWidth:.infinity,maxHeight:.infinity,alignment:.topLeading)
    }
}
