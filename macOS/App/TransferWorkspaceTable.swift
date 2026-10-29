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

    var body: some View {
        NativeTransferGrid(store:store,rows:store.visibleTorrents.sorted(using:sortOrder + [KeyPathComparator(\TorrentItem.id)]),sortOrder:$sortOrder,customization:$columnCustomization,rowHeight:comfortableRows ? 40 : 28,focus:$transfersFocused)
            .focused($transfersFocused)
    }
}
