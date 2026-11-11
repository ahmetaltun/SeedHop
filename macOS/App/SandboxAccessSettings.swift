import SwiftUI

#if SEEDHOP_SANDBOX
struct SandboxAccessSettings: View {
    @EnvironmentObject var store: LibraryStore
    var onReady: (() -> Void)? = nil
    @State private var busy = false
    @State private var message: String?
    @State private var folders: [String] = []

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Label(String(localized:"Folder access",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale), systemImage: "folder.badge.lock")
                .font(.system(size: 12, weight: .semibold))
            Text(String(localized:"Approve the folders SeedHop can use. Import your previous library before choosing download folders.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                .font(.system(size: 11)).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            Text(String(localized:"Choose a folder again to repair damaged access records. Recoverable approvals are kept, and damaged data is backed up first.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                .font(.system(size: 10)).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            HStack {
                Button(String(localized:"Grant Folder Access…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { chooseFolder() }
                Spacer()
                Menu(String(localized:"Library",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) {
                    Button(String(localized:"Import Existing Library…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { chooseLibrary(rollback: false) }.disabled(store.connected)
                    Button(String(localized:"Restore Previous Library…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { chooseLibrary(rollback: true) }
                }.disabled(store.connecting || store.switching)
            }.disabled(busy)
            ForEach(folders,id:\.self) { folder in
                Button(folder) { chooseFolder(at:URL(fileURLWithPath:folder)) }
                    .lineLimit(1).truncationMode(.middle).disabled(busy)
            }
            if busy { ProgressView().controlSize(.small) }
            if let message { Text(message).font(.system(size: 11)).fixedSize(horizontal: false, vertical: true) }
        }
        .onAppear(perform:refreshFolders)
    }

    private func refreshFolders() {
        folders = SandboxLibraryMigration.requiredFolders().filter {
            (try? FolderAccessStore.application.requireAccess(to:$0)) == nil
        }
    }

    private func chooseFolder(at directory: URL? = nil) {
        let panel = NSOpenPanel(); panel.canChooseFiles = false; panel.canChooseDirectories = true
        panel.directoryURL = directory ?? SandboxLibraryMigration.migratedDownloadDirectory.map { URL(fileURLWithPath:$0) }
        panel.begin { response in
            guard response == .OK, let url = panel.url else { return }
            do {
                try FolderAccessStore.application.remember(url)
                // A first approved root also supplies the stopped engine's default.
                if !SandboxLibraryMigration.isSetUp, let previous = SandboxLibraryMigration.migratedDownloadDirectory,
                   (try? FolderAccessStore.application.requireAccess(to:previous)) != nil {
                    store.downloadDirectory = previous; UserDefaults.standard.set(previous,forKey:"downloadDirectory")
                } else if (try? FolderAccessStore.application.requireAccess(to:store.downloadDirectory)) == nil {
                    store.downloadDirectory = url.path; UserDefaults.standard.set(url.path,forKey:"downloadDirectory")
                }
                refreshFolders()
                if folders.isEmpty {
                    SandboxLibraryMigration.completeSetup()
                    onReady?()
                    Task { await store.start() }
                }
            } catch { message = EngineMessage(code:FolderAccessFailure.noticeCode(for:error)).localizedDescription }
        }
    }

    private func chooseLibrary(rollback: Bool) {
        let panel = NSOpenPanel(); panel.canChooseFiles = false; panel.canChooseDirectories = true
        panel.directoryURL = SandboxLibraryMigration.legacyLibrary?.deletingLastPathComponent()
        panel.begin { response in
            guard response == .OK, let url = panel.url else { return }
            // Show the source selection before requesting the final migration choice.
            let confirmation = NSAlert()
            confirmation.messageText = rollback
                ? String(localized:"Restore the previous library?",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
                : String(localized:"Copy this library into SeedHop?",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
            confirmation.informativeText = String(localized:"Only one library can run. Import blocks the previous library; restoring it blocks the imported copy. Downloads stay in their current folders.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
            confirmation.addButton(withTitle: String(localized:"Continue",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
            confirmation.addButton(withTitle: String(localized:"Cancel",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
            guard confirmation.runModal() == .alertFirstButtonReturn else { return }
            busy = true
            Task {
                defer { busy = false }
                do {
                    if rollback { try await store.stopForLibraryMigration() }
                    try await Task.detached(priority: .userInitiated) {
                        if rollback { try SandboxLibraryMigration.restorePreviousLibrary(at: url) }
                        else { try SandboxLibraryMigration.copyLibrary(from: url) }
                    }.value
                    if rollback { SandboxLibraryMigration.resetSetup() }
                    refreshFolders()
                    message = String(localized:"Library updated. Choose its download folders before starting transfers.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
                } catch {
                    message = String(localized:"The library could not be updated. Close both engines, finish pending file changes, and retry. Your saved files have been preserved.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
                }
            }
        }
    }

}

struct SandboxFirstRun: ViewModifier {
    @ObservedObject var store: LibraryStore
    @State private var showSetup = false

    func body(content: Content) -> some View {
        content
            .onAppear { showSetup = !store.designPreview && !SandboxLibraryMigration.isSetUp }
            .sheet(isPresented:$showSetup) {
                ScrollView {
                    SandboxAccessSettings(onReady:{ showSetup = false }).environmentObject(store)
                        .padding(24)
                }.frame(width:510,height:350).interactiveDismissDisabled()
            }
    }
}
#endif
