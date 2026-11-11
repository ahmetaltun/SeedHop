import SwiftUI

@MainActor final class PrivateSessionController: ObservableObject {
    @Published var store: LibraryStore?
    @Published private(set) var busy = false
    @Published var error: String?
    private var processID: Int32?

    func begin(from library: LibraryStore) {
        guard store == nil, !busy else { return }
        busy = true
        let panel = NSOpenPanel()
        panel.canChooseFiles = false; panel.canChooseDirectories = true
        panel.title = String(localized:"New Private Session…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        panel.message = String(localized:"Choose a download folder. Ending the session stops its transfers and discards its library records. Downloaded and partial files stay on disk.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        panel.begin { response in
            guard response == .OK, let folder = panel.url else { self.busy = false; return }
            Task {
                defer { self.busy = false }
                let workspace = LibraryStore(isPrivate:true)
                do {
                    let policy = try await library.privateProtection()
                    self.processID = try await workspace.startPrivate(folder:folder,protection:policy)
                    self.store = workspace
                } catch {
                    workspace.client.invalidate()
                    self.error = error.localizedDescription
                }
            }
        }
    }

    func end() async -> Bool {
        guard !busy else { return false }
        guard let store, let processID else { return true }
        busy = true
        defer { busy = false }
        do {
            try await store.endPrivate(pid:processID)
            self.processID = nil
            self.store = nil
            return true
        } catch {
            store.error = error.localizedDescription
            return false
        }
    }
}

struct PrivateSessionView: View {
    @ObservedObject var controller: PrivateSessionController
    @ObservedObject var store: LibraryStore

    var body: some View {
        VStack(alignment:.leading,spacing:16) {
            Label(String(localized:"Private Session",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),systemImage:"eye.slash")
                .font(.title2.weight(.semibold))
            if store.connected {
                Text(store.networkBlocked
                     ? String(localized:"Network blocked",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
                     : store.networkInterface ?? String(localized:"Direct connection",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                    .font(.subheadline).foregroundStyle(.secondary)
            }
            if let error = store.error { Text(error).foregroundStyle(.secondary).textSelection(.enabled) }
            HStack {
                Button(String(localized:"Add Torrent…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { store.beginImport() }
                Button(String(localized:"Resume Selected",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { store.toggleSelected(paused:false) }.disabled(!store.canResumeSelection)
                Button(String(localized:"Pause Selected",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { store.toggleSelected(paused:true) }.disabled(!store.canPauseSelection)
                Button(String(localized:"Remove Selected…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) {
                    store.requestRemoval()
                }.disabled(!store.canRemoveSelection)
            }.disabled(!store.connected || controller.busy)
            List(store.torrents,selection:$store.selection) { item in
                VStack(alignment:.leading,spacing:4) {
                    Text(item.name).lineLimit(1)
                    ProgressView(value:Double(item.downloadedBytes),total:Double(max(item.totalBytes,1)))
                    Text(item.statusLabel).font(.caption).foregroundStyle(.secondary)
                }.tag(item.id)
            }
            HStack {
                Spacer()
                if controller.busy { ProgressView().controlSize(.small) }
                Button(String(localized:"End Private Session",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) {
                    Task { _ = await controller.end() }
                }.disabled(controller.busy)
            }
        }.padding(24).frame(width:620,height:450)
            .interactiveDismissDisabled()
            .sheet(isPresented:$store.showImport) { ImportView(privateSession:true).environmentObject(store) }
            .confirmationDialog(String(localized:"Remove Selected…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),isPresented:$store.showRemoval) {
                Button(String(localized:"Remove",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { store.removeConfirmed(deleteFiles:false) }
            }
    }
}
