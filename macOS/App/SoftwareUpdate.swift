import SwiftUI
import Sparkle

/// One updater for the app. Fixture launches must never contact the public feed.
@MainActor final class SoftwareUpdate: NSObject, ObservableObject, SPUUpdaterDelegate {
    static let shared = SoftwareUpdate()
    @Published private(set) var canCheck = false
    @Published private(set) var automaticChecks = false
    private var controller: SPUStandardUpdaterController?
    private var observations: [NSKeyValueObservation] = []
    private(set) var installing = false
    var installationFailed: (() -> Void)?

    func start() {
        guard controller == nil else { return }
        #if SEEDHOP_DEVELOPMENT
        return
        #else
        guard !ProcessInfo.processInfo.arguments.contains(where: {
            $0.hasPrefix("--verify-") || $0.hasPrefix("--render-") || $0 == "--design-preview"
        }) else { return }
        let controller = SPUStandardUpdaterController(startingUpdater:false, updaterDelegate:self, userDriverDelegate:nil)
        self.controller = controller
        observations = [
            controller.updater.observe(\.canCheckForUpdates, options:[.initial, .new]) { [weak self] updater, _ in
                Task { @MainActor in self?.canCheck = updater.canCheckForUpdates }
            },
            controller.updater.observe(\.automaticallyChecksForUpdates, options:[.initial, .new]) { [weak self] updater, _ in
                Task { @MainActor in self?.automaticChecks = updater.automaticallyChecksForUpdates }
            }
        ]
        controller.startUpdater()
        #endif
    }

    func check() { controller?.checkForUpdates(nil) }
    func setAutomaticChecks(_ enabled: Bool) {
        controller?.updater.automaticallyChecksForUpdates = enabled
    }
    func updater(_ updater: SPUUpdater, willInstallUpdate item: SUAppcastItem) { installing = true }
    func updater(_ updater: SPUUpdater, didAbortWithError error: Error) {
        guard installing else { return }
        installing = false
        installationFailed?()
    }
    func updater(_ updater: SPUUpdater, didFinishUpdateCycleFor updateCheck: SPUUpdateCheck, error: Error?) {
        guard installing else { return }
        installing = false
        installationFailed?()
    }
}

struct SoftwareUpdateCommands: Commands {
    @ObservedObject private var update = SoftwareUpdate.shared
    var body: some Commands {
        CommandGroup(after:.appInfo) {
            Button(String(localized:"Check for Updates…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { update.check() }
                .disabled(!update.canCheck)
        }
    }
}

struct SoftwareUpdateSettings: View {
    @ObservedObject private var update = SoftwareUpdate.shared
    var body: some View {
        VStack(alignment:.leading,spacing:12) {
            Toggle(String(localized:"Automatically check for updates",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),
                   isOn:Binding(get:{update.automaticChecks},set:{update.setAutomaticChecks($0)}))
                .toggleStyle(.switch).controlSize(.small).disabled(!update.canCheck)
            Button(String(localized:"Check for Updates…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { update.check() }
                .disabled(!update.canCheck)
        }
    }
}
