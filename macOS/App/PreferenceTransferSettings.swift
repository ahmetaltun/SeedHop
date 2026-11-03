import SwiftUI
import UniformTypeIdentifiers

#if SEEDHOP_DEVELOPMENT
struct PreferenceTransferSettings: View {
    @EnvironmentObject var store: LibraryStore
    @State private var message: String?
    @State private var undoAvailable = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Label(String(localized:"Settings transfer",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale), systemImage: "square.and.arrow.up.on.square")
                .font(.system(size: 12, weight: .semibold))
            Text(String(localized:"Transfer appearance, language, and workspace layout. Imported settings apply after restarting SeedHop.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                .font(.system(size: 11)).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            HStack {
                Button(String(localized:"Export Settings…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale), action: exportSettings)
                Button(String(localized:"Import Settings…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale), action: importSettings)
                Spacer()
                Button(String(localized:"Undo Settings Import",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale), action: undoImport).disabled(!undoAvailable)
            }.disabled(WorkspacePreferences.isFixture || store.switching)
            if let message { Text(message).font(.system(size: 11)).fixedSize(horizontal: false, vertical: true) }
        }.onAppear { undoAvailable = PreferenceTransfer.canUndo() }
    }

    private func exportSettings() {
        do {
            let bytes = try PreferenceTransfer.exportData()
            let panel = NSSavePanel(); panel.allowedContentTypes = [.json]
            panel.nameFieldStringValue = "SeedHop Settings.json"
            panel.begin { response in
                guard response == .OK, let url = panel.url else { return }
                let scoped = url.startAccessingSecurityScopedResource()
                defer { if scoped { url.stopAccessingSecurityScopedResource() } }
                do {
                    try bytes.write(to: url, options: .atomic)
                    message = String(localized:"Settings exported.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
                } catch { showFailure() }
            }
        } catch { showFailure() }
    }

    private func importSettings() {
        let panel = NSOpenPanel(); panel.allowedContentTypes = [.json]; panel.allowsMultipleSelection = false
        panel.begin { response in
            guard response == .OK, let url = panel.url else { return }
            do {
                let archive = try PreferenceTransfer.inspect(url)
                let confirmation = NSAlert()
                confirmation.messageText = String(localized:"Import these settings?",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
                confirmation.informativeText = String(localized:"Your current settings will be saved for undo. Download folders, notifications, and background access require separate choices.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
                confirmation.addButton(withTitle: String(localized:"Continue",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                confirmation.addButton(withTitle: String(localized:"Cancel",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                guard confirmation.runModal() == .alertFirstButtonReturn else { return }
                try PreferenceTransfer.stage(archive)
                undoAvailable = PreferenceTransfer.canUndo()
                message = String(localized:"Settings saved. Restart SeedHop to apply the change.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
            } catch { showFailure() }
        }
    }

    private func undoImport() {
        do {
            let needsRestart = try PreferenceTransfer.requestUndo()
            undoAvailable = PreferenceTransfer.canUndo()
            message = needsRestart
                ? String(localized:"Settings saved. Restart SeedHop to apply the change.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
                : String(localized:"Settings import canceled.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        } catch { showFailure() }
    }

    private func showFailure() {
        message = String(localized:"Settings could not be updated. Your saved settings have been preserved.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
    }
}
#endif
