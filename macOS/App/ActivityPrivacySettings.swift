import SwiftUI

#if SEEDHOP_DEVELOPMENT
struct ActivityPrivacySettings: View {
    @EnvironmentObject var store: LibraryStore
    @State private var message: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Label(String(localized:"Local activity",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale), systemImage: "hand.raised")
                .font(.system(size: 12, weight: .semibold))
            Text(String(localized:"Clear recent activity, traffic graphs, SeedHop recent items, and notifications. Downloads and saved library records are kept.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                .font(.system(size: 11)).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            HStack {
                Button(String(localized:"Clear Local Activity…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale), action: confirmClear)
                    .disabled(!store.canClearLocalActivity)
                if store.clearingActivity { ProgressView().controlSize(.small) }
            }
            if let message { Text(message).font(.system(size: 11)).fixedSize(horizontal: false, vertical: true) }
        }
    }

    private func confirmClear() {
        let alert = NSAlert()
        alert.messageText = String(localized:"Clear local activity?",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        alert.informativeText = String(localized:"This clears the activity timeline, traffic graphs, SeedHop recent items, and notifications. Running transfers can create new activity. This does not erase downloaded files or saved transfer statistics.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        alert.addButton(withTitle: String(localized:"Clear",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
        alert.addButton(withTitle: String(localized:"Cancel",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        Task {
            guard let complete = await store.clearLocalActivity() else { return }
            message = complete
                ? String(localized:"Local activity cleared. Running transfers may create new activity.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
                : String(localized:"Visible activity and SeedHop notifications were cleared. Engine history could not be cleared; reconnect and try again.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        }
    }
}
#endif
