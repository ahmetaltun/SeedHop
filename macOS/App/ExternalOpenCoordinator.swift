import SwiftUI
import Combine

/// Owns external requests from launch through the dismissal of each review sheet.
@MainActor final class ExternalOpenCoordinator {
    private weak var store: LibraryStore?
    private var observation: AnyCancellable?
    private var pending: [URL] = []
    private var outstanding = Set<String>()
    private var active: URL?
    private var waitingForDismissal = false
    private var unsupportedInput = false
    var reveal: (() -> Void)?
    var pendingCount: Int { pending.count }

    func attach(_ store: LibraryStore) {
        guard self.store !== store else { return }
        self.store = store
        store.externalOpenCoordinator = self
        observation = store.objectWillChange.sink { [weak self] in
            // Published notifications precede the state change.
            Task { @MainActor [weak self] in self?.advance() }
        }
        if !pending.isEmpty || unsupportedInput { reveal?() }
        advance()
    }

    func receive(_ urls: [URL]) {
        for url in urls {
            guard Self.supports(url) else { unsupportedInput = true; continue }
            if outstanding.insert(Self.key(url)).inserted { pending.append(url) }
        }
        if !urls.isEmpty { reveal?() }
        advance()
    }

    static func supports(_ url: URL) -> Bool {
        url.scheme?.lowercased() == "magnet" || (url.isFileURL && url.pathExtension.lowercased() == "torrent")
    }

    private static func key(_ url: URL) -> String {
        url.isFileURL ? url.standardizedFileURL.absoluteString : url.absoluteString
    }

    func didDismissImport() {
        if let active { outstanding.remove(Self.key(active)) }
        active = nil
        waitingForDismissal = false
        // Allow SwiftUI to finish removing the previous sheet before presenting another.
        Task { @MainActor [weak self] in
            await Task.yield()
            self?.advance()
        }
    }

    private func advance() {
        guard let store else { return }
        if unsupportedInput {
            unsupportedInput = false
            store.error = String(localized:"Choose a .torrent file or a magnet link to add a transfer.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        }
        if store.showImport { waitingForDismissal = true; return }
        guard !waitingForDismissal, store.connected, !store.hasFileOperation, !store.switching,
              !store.showRemoval, !pending.isEmpty else { return }
        let url = pending.removeFirst()
        active = url
        waitingForDismissal = true
        // The normal import sheet performs file reading and validation, including failures.
        if url.isFileURL { store.beginImport(file:url) }
        else { store.beginImport(magnet:url.absoluteString) }
    }
}

struct ExternalOpenBridge: ViewModifier {
    let coordinator: ExternalOpenCoordinator
    let store: LibraryStore
    let attached: () -> Void
    @Environment(\.openWindow) private var openWindow

    func body(content: Content) -> some View {
        content.onAppear {
            let revealWindow = openWindow
            coordinator.reveal = {
                revealWindow(id:"library",value:"library")
                NSApp.activate(ignoringOtherApps:true)
            }
            coordinator.attach(store)
            attached()
        }
        // Route external URLs to this library before SwiftUI creates another group window.
        .handlesExternalEvents(preferring:["*"],allowing:["*"])
        .onOpenURL { coordinator.receive([$0]) }
    }
}
