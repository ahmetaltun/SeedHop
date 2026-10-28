import SwiftUI
import AppKit

/// The table owns observation inside its hosting graph. Selection-only changes
/// must invalidate native bindings even when the transfer rows remain identical.
struct TransferWorkspaceTable: View {
    @ObservedObject var store: LibraryStore
    @Binding var sortOrder: [KeyPathComparator<TorrentItem>]
    @Binding var columnCustomization: TableColumnCustomization<TorrentItem>
    let comfortableRows: Bool
    @FocusState.Binding var transfersFocused: Bool
    @StateObject private var nativeSelection = WorkspaceNativeSelection()

    var body: some View {
        let selected = store.selection
        let rows = store.visibleTorrents.sorted(using:sortOrder + [KeyPathComparator(\TorrentItem.id)])
        let indexes = IndexSet(rows.indices.filter { selected.contains(rows[$0].id) })
        return Table(rows,selection:Binding(get:{ selected },set:{ if !nativeSelection.synchronizing && store.selection != $0 { store.selection = $0 } }),sortOrder:$sortOrder,columnCustomization:$columnCustomization) {
            primaryColumns
            secondaryColumns
            optionalColumns
        }
        .tableStyle(.inset(alternatesRowBackgrounds:true))
        .environment(\.defaultMinListRowHeight,comfortableRows ? 40 : 28)
        .transaction { $0.animation = nil; $0.disablesAnimations = true }
        .font(.system(size:11)).monospacedDigit()
        .focused($transfersFocused)
        .accessibilityLabel("Transfers")
        .background(NativeTransferTableMarker(rowHeight:comfortableRows ? 40 : 28,rowIDs:rows.map(\.id),generation:store.torrentGeneration,selection:indexes,selectionBridge:nativeSelection).frame(width:1,height:1).allowsHitTesting(false))
        .contextMenu(forSelectionType:String.self) { ids in
            Button("Move to Front of Queue") { store.selection = ids; store.moveSelectedToFront() }.disabled(!canAct(on:ids) || ids.count != 1)
            Button("Move Files…") { store.selection = ids; store.chooseRelocation() }.disabled(!canAct(on:ids) || store.storageRecovery != nil || ids.count != 1 || store.torrents.contains { ids.contains($0.id) && $0.state == "checking" })
            Button("Force Recheck") { store.selection = ids; store.recheckSelected() }.disabled(!canAct(on:ids) || store.torrents.contains { ids.contains($0.id) && ($0.state == "checking" || store.hasPendingCleanup($0.id)) })
            Button(store.resumeLabel(for:store.torrents.filter { ids.contains($0.id) })) { store.selection = ids; store.toggleSelected(paused:false) }.disabled(!canAct(on:ids) || !store.torrents.contains { ids.contains($0.id) && store.canResumeOrRetry($0) })
            Button("Pause") { store.selection = ids; store.toggleSelected(paused:true) }.disabled(!canAct(on:ids) || !store.torrents.contains { ids.contains($0.id) && !$0.isPaused })
            Divider()
            Button("Show in Finder") { if let item = store.torrents.first(where:{ ids.contains($0.id) }) { NSWorkspace.shared.selectFile(nil,inFileViewerRootedAtPath:item.destination) } }.disabled(ids.isEmpty)
            Button("Remove…",role:.destructive) { store.selection = ids; store.requestRemoval() }.disabled(!canAct(on:ids) || ids.contains(where:store.hasPendingCleanup))
        } primaryAction: { ids in
            guard !ids.isEmpty else { return }
            store.selection = ids
            store.toggleSelected(paused:store.canPauseSelection)
        }
    }
    @TableColumnBuilder<TorrentItem, KeyPathComparator<TorrentItem>> private var primaryColumns: some TableColumnContent<TorrentItem, KeyPathComparator<TorrentItem>> {
            TableColumn("Name",value:\.name) { torrent in
                Text(torrent.name).font(.system(size:12,weight:.medium)).lineLimit(1).truncationMode(.middle)
                    .frame(height:comfortableRows ? 32 : 20)
                    .help(torrent.name).accessibilityLabel("\(torrent.name), \(store.presentation(torrent).accessibleLabel)")
            }.width(min:180,ideal:240).customizationID("name").disabledCustomizationBehavior(.visibility)
            TableColumn("Status",value:\.sortableStatus) { StateBadge(torrent:$0,presentation:store.presentation($0),verticalPadding:3) }.width(min:110,ideal:125).customizationID("status")
            TableColumn("Progress",value:\.progress) { torrent in
                let status = store.presentation(torrent)
                HStack(spacing:5) { if let value = status.progress { TransferProgress(value:value).frame(width:44); Text("\(Int(value * 100))%").monospacedDigit() } else { Text("—") } }.font(.system(size:11)).help(status.progressDescription)
            }.width(min:85,ideal:90).customizationID("progress")
            TableColumn("↓ Download",value:\.downloadRate) { Text(SizeText.rate(store.connected ? $0.downloadRate : 0)).font(.system(size:11)).monospacedDigit().foregroundStyle(.secondary) }.width(min:70,ideal:80).customizationID("download")
            TableColumn("↑ Upload",value:\.uploadRate) { Text(SizeText.rate(store.connected ? $0.uploadRate : 0)).font(.system(size:11)).monospacedDigit().foregroundStyle(.secondary) }.width(min:70,ideal:80).customizationID("upload")
    }
    @TableColumnBuilder<TorrentItem, KeyPathComparator<TorrentItem>> private var secondaryColumns: some TableColumnContent<TorrentItem, KeyPathComparator<TorrentItem>> {
            TableColumn("ETA",value:\.sortableETA) { Text($0.etaSeconds.map(Self.duration) ?? "—").help("Estimated time for selected files; unavailable while paused, checking, or without an estimate") }.width(min:60,ideal:75).customizationID("eta")
            TableColumn("Ratio",value:\.sortableRatio) { Text($0.shareRatio.map { String(format:"%.2f",$0) } ?? "—").help("Lifetime uploaded payload divided by downloaded payload. Unknown history and zero downloads have no ratio.") }.width(min:45,ideal:55).customizationID("ratio")
            TableColumn("Peers",value:\.peers) { Text("\($0.peers)").font(.system(size:11)).monospacedDigit().foregroundStyle(.secondary) }.width(min:45,ideal:50).customizationID("peers")
            TableColumn("Queue",value:\.queuePosition) { Text($0.state == "queued" ? String($0.queuePosition) : "—") }.width(min:45,ideal:55).customizationID("queue")
            TableColumn("Selected Size",value:\.totalBytes) { Text(SizeText.bytes($0.selectedBytes ?? $0.totalBytes)) }.width(min:70,ideal:90).customizationID("selectedSize").defaultVisibility(.hidden)
            TableColumn("Total Size",value:\.fullBytes) { Text(SizeText.bytes($0.fullBytes)) }.width(min:70,ideal:90).customizationID("size").defaultVisibility(.hidden)
            TableColumn("Remaining",value:\.remainingBytes) { Text(SizeText.bytes($0.remainingBytes)) }.width(min:70,ideal:90).customizationID("remaining").defaultVisibility(.hidden)
    }
    @TableColumnBuilder<TorrentItem, KeyPathComparator<TorrentItem>> private var optionalColumns: some TableColumnContent<TorrentItem, KeyPathComparator<TorrentItem>> {
            TableColumn("Downloaded Traffic",value:\.sortableDownloaded) { Text($0.networkDownloadedBytes.map(SizeText.bytes) ?? "—") }.width(min:90,ideal:115).customizationID("downloadedTraffic").defaultVisibility(.hidden)
            TableColumn("Uploaded Traffic",value:\.sortableUploaded) { Text($0.networkUploadedBytes.map(SizeText.bytes) ?? "—") }.width(min:85,ideal:110).customizationID("uploadedTraffic").defaultVisibility(.hidden)
            TableColumn("Added",value:\.sortableAdded) { Text($0.addedAt.map(Self.date) ?? "—") }.width(min:80,ideal:100).customizationID("added").defaultVisibility(.hidden)
            TableColumn("Completed",value:\.sortableCompleted) { Text($0.completedAt.map(Self.date) ?? "—") }.width(min:80,ideal:100).customizationID("completed").defaultVisibility(.hidden)
            TableColumn("Save Path",value:\.destination) { Text($0.destination).lineLimit(1).truncationMode(.middle).help($0.destination) }.width(min:140,ideal:220).customizationID("destination").defaultVisibility(.hidden)    }
    private func canAct(on ids: Set<String>) -> Bool {
        store.canChangeTransfers && !ids.isEmpty && ids.isSubset(of:Set(store.torrents.map(\.id))) && store.busyTransfers.isDisjoint(with:ids)
    }
    private static func duration(_ value: UInt64) -> String { value >= 3600 ? "\(value / 3600)h \(value % 3600 / 60)m" : "\(value / 60)m \(value % 60)s" }
    private static func date(_ value: UInt64) -> String { Date(timeIntervalSince1970:Double(value)).formatted(date:.abbreviated,time:.omitted) }
}
