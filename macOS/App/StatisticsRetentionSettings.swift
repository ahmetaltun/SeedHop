import SwiftUI

#if SEEDHOP_DEVELOPMENT
struct StatisticsRetentionSettings: View {
    @EnvironmentObject var store: LibraryStore
    private var canEdit: Bool {
        store.statisticsRetention != nil && !WorkspacePreferences.isFixture && !store.connecting && !store.switching &&
            !store.savingStatisticsRetention && !store.clearingActivity && !store.hasFileOperation && !store.savingNetwork && !store.savingEncryption
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Label(String(localized:"Saved statistics",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale), systemImage: "chart.bar")
                .font(.system(size: 12, weight: .semibold))
            Toggle(String(localized:"Remember transfer statistics between launches",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale), isOn: Binding(
                get: { store.statisticsRetention?.rememberStatistics ?? true },
                set: { remember in
                    if remember { Task { await store.saveStatisticsRetention(true) } }
                    else { confirmForget() }
                }))
                .toggleStyle(.switch).controlSize(.small).disabled(!canEdit)
            Text(String(localized:"Turning this off clears this library’s saved totals and dates. Session upload and download counters remain available. Downloads and resume data are kept.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                .font(.system(size: 11)).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            Text(String(localized:"Earlier builds cannot open the library after this setting is changed.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                .font(.system(size: 10)).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            if store.statisticsRetention?.clearPending == true {
                Text(String(localized:"Statistics have not finished clearing. Check storage space and folder access, then retry.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                    .font(.system(size: 11)).fixedSize(horizontal: false, vertical: true)
                Button(String(localized:"Retry Clearing Statistics",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) {
                    Task { await store.saveStatisticsRetention(false) }
                }.disabled(!canEdit)
            }
            HStack {
                Button(String(localized:"Refresh",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) {
                    Task { await store.refreshStatisticsRetention() }
                }.disabled(WorkspacePreferences.isFixture || store.connecting || store.switching || store.savingStatisticsRetention)
                if store.savingStatisticsRetention { ProgressView().controlSize(.small) }
            }
        }.task { await store.refreshStatisticsRetention() }
    }

    private func confirmForget() {
        let alert = NSAlert()
        alert.messageText = String(localized:"Stop saving transfer statistics?",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        alert.informativeText = String(localized:"Saved totals and dates in this library will be cleared. Downloads and resume data are kept. Earlier builds will refuse to open this library.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        alert.addButton(withTitle: String(localized:"Stop Saving and Clear",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
        alert.addButton(withTitle: String(localized:"Cancel",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        Task { await store.saveStatisticsRetention(false) }
    }
}
#endif
