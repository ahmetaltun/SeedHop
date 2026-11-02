import SwiftUI
import AppKit
import UniformTypeIdentifiers

/// The operating system remains the source of truth for default handlers.
enum FileAssociationKind: String, CaseIterable, Identifiable {
    case torrent, magnet
    var id: String { rawValue }
    @MainActor var title: String {
        switch self {
        case .torrent: String(localized:"Default app for .torrent files",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case .magnet: String(localized:"Default app for magnet links",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        }
    }
}

struct AssociationApplication: Codable, Equatable, Identifiable {
    let url: URL
    let bundleIdentifier: String?
    let name: String
    var id: URL { url }
}

@MainActor protocol FileAssociationSystem {
    var seedHop: AssociationApplication { get }
    func current(_ kind: FileAssociationKind) -> AssociationApplication?
    func applications(_ kind: FileAssociationKind) -> [AssociationApplication]
    func installedApplication(_ saved: AssociationApplication, for kind: FileAssociationKind) -> AssociationApplication?
    func setDefault(_ application: AssociationApplication, for kind: FileAssociationKind) async throws
}

@MainActor struct WorkspaceFileAssociationSystem: FileAssociationSystem {
    private let workspace = NSWorkspace.shared
    private var torrentType: UTType { UTType(importedAs:"org.bittorrent.torrent",conformingTo:.data) }
    private var magnetURL: URL { URL(string:"magnet:?xt=urn:btih:0123456789012345678901234567890123456789")! }
    var seedHop: AssociationApplication { application(Bundle.main.bundleURL) }
    private func application(_ url: URL) -> AssociationApplication {
        let name = FileManager.default.displayName(atPath:url.path)
        return AssociationApplication(url:url,bundleIdentifier:Bundle(url:url)?.bundleIdentifier,
            name:name.lowercased().hasSuffix(".app") ? String(name.dropLast(4)) : name)
    }
    func current(_ kind: FileAssociationKind) -> AssociationApplication? {
        let url = kind == .torrent ? workspace.urlForApplication(toOpen:torrentType) : workspace.urlForApplication(toOpen:magnetURL)
        return url.map(application)
    }
    func applications(_ kind: FileAssociationKind) -> [AssociationApplication] {
        let urls = kind == .torrent ? workspace.urlsForApplications(toOpen:torrentType) : workspace.urlsForApplications(toOpen:magnetURL)
        return urls.filter { FileManager.default.fileExists(atPath:$0.path) }.map(application)
    }
    func installedApplication(_ saved: AssociationApplication, for kind: FileAssociationKind) -> AssociationApplication? {
        let candidates = applications(kind)
        // Prefer the original copy when duplicate installations share a bundle identifier.
        if let exact = candidates.first(where:{ $0.url.standardizedFileURL == saved.url.standardizedFileURL && $0.bundleIdentifier == saved.bundleIdentifier }) { return exact }
        guard let identifier = saved.bundleIdentifier else { return nil }
        let installed = workspace.urlsForApplications(withBundleIdentifier:identifier)
        return installed.compactMap { url in candidates.first { $0.url.standardizedFileURL == url.standardizedFileURL } }.first
    }
    func setDefault(_ application: AssociationApplication, for kind: FileAssociationKind) async throws {
        // Previews and self-tests must never change the user's Launch Services defaults.
        guard !WorkspacePreferences.isFixture else {
            throw UserFacingError(message:String(localized:"Default apps cannot be changed in a preview or test.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
        }
        if kind == .torrent { try await workspace.setDefaultApplication(at:application.url,toOpen:torrentType) }
        else { try await workspace.setDefaultApplication(at:application.url,toOpenURLsWithScheme:"magnet") }
    }
}

@MainActor final class FileAssociationService: ObservableObject {
    struct Replacement: Identifiable {
        let kind: FileAssociationKind
        let applications: [AssociationApplication]
        var id: FileAssociationKind { kind }
    }
    @Published private(set) var handlers: [FileAssociationKind:AssociationApplication] = [:]
    @Published private(set) var busy = Set<FileAssociationKind>()
    @Published private(set) var errors: [FileAssociationKind:String] = [:]
    @Published private(set) var replacement: Replacement?
    private let system: any FileAssociationSystem
    private let defaults: UserDefaults
    init(system: (any FileAssociationSystem)? = nil, defaults: UserDefaults = WorkspacePreferences.defaults) {
        self.system = system ?? WorkspaceFileAssociationSystem(); self.defaults = defaults
        refresh()
    }
    func refresh() {
        var updated: [FileAssociationKind:AssociationApplication] = [:]
        for kind in FileAssociationKind.allCases { updated[kind] = system.current(kind) }
        handlers = updated
    }
    private func isSeedHop(_ app: AssociationApplication) -> Bool {
        app.url.standardizedFileURL == system.seedHop.url.standardizedFileURL ||
            (app.bundleIdentifier != nil && app.bundleIdentifier == system.seedHop.bundleIdentifier)
    }
    func isDefault(_ kind: FileAssociationKind) -> Bool { handlers[kind].map(isSeedHop) ?? false }
    func handlerName(_ kind: FileAssociationKind) -> String {
        handlers[kind]?.name ?? String(localized:"No default app",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
    }
    private func key(_ kind: FileAssociationKind) -> String { "previousDefaultApplication.\(kind.rawValue)" }
    func change(_ kind: FileAssociationKind, enabled: Bool) async {
        guard !busy.contains(kind), replacement == nil else { return }
        refresh(); errors[kind] = nil
        guard isDefault(kind) != enabled else { return }
        if enabled {
            let previous = handlers[kind]
            if let previous, !isSeedHop(previous), let data = try? JSONEncoder().encode(previous) { defaults.set(data,forKey:key(kind)) }
            else { defaults.removeObject(forKey:key(kind)) }
            await apply(system.seedHop,kind:kind)
        } else {
            let saved = defaults.data(forKey:key(kind)).flatMap { try? JSONDecoder().decode(AssociationApplication.self,from:$0) }
            if let saved, let installed = system.installedApplication(saved,for:kind), !isSeedHop(installed) {
                await apply(installed,kind:kind)
            } else {
                let candidates = system.applications(kind).filter { !isSeedHop($0) }
                if candidates.isEmpty {
                    errors[kind] = String(localized:"Install another compatible app before changing this default.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
                } else { replacement = Replacement(kind:kind,applications:candidates) }
            }
        }
    }
    func cancelReplacement() { replacement = nil; refresh() }
    func chooseReplacement(_ application: AssociationApplication) async {
        guard let request = replacement, request.applications.contains(application) else { return }
        replacement = nil
        // Another app may have claimed the association while the chooser was open.
        refresh()
        guard isDefault(request.kind) else { return }
        guard let installed = system.installedApplication(application,for:request.kind), !isSeedHop(installed) else {
            errors[request.kind] = String(localized:"This app is no longer available. Choose another app.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
            return
        }
        await apply(installed,kind:request.kind)
    }
    private func apply(_ application: AssociationApplication, kind: FileAssociationKind) async {
        busy.insert(kind)
        defer { busy.remove(kind); refresh() }
        do { try await system.setDefault(application,for:kind) }
        catch { errors[kind] = error.localizedDescription; return }
        refresh()
        if system.current(kind)?.url.standardizedFileURL != application.url.standardizedFileURL {
            errors[kind] = String(localized:"macOS did not change the default app. Try again.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        }
    }
}

struct FileAssociationSettings: View {
    @StateObject private var service = FileAssociationService()
    @State private var selectedApplication: URL?
    var body: some View {
        VStack(alignment:.leading,spacing:18) {
            ForEach(FileAssociationKind.allCases) { kind in
                if kind == .magnet { Divider() }
                VStack(alignment:.leading,spacing:6) {
                    Toggle(isOn:Binding(get:{ service.isDefault(kind) },set:{ enabled in Task { await service.change(kind,enabled:enabled) } })) {
                        VStack(alignment:.leading,spacing:4) {
                            Text(kind.title).font(.system(size:12,weight:.medium))
                            Text(String(localized:"Current default: \(service.handlerName(kind))",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                                .font(.system(size:11)).foregroundStyle(.secondary)
                        }.frame(maxWidth:.infinity,alignment:.leading)
                    }.disabled(service.busy.contains(kind) || service.replacement != nil)
                    if service.busy.contains(kind) { ProgressView().controlSize(.small).accessibilityLabel(String(localized:"Changing default app",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) }
                    if let error = service.errors[kind] { Text(error).font(.system(size:11)).foregroundStyle(.red).fixedSize(horizontal:false,vertical:true) }
                }
            }
            Text(String(localized:"Turning this off restores another default app. You can still use Open With SeedHop or add torrents in the app.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                .font(.system(size:11)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true)
        }
        .toggleStyle(.switch).controlSize(.small)
        .onAppear { service.refresh() }
        .onReceive(NotificationCenter.default.publisher(for:NSApplication.didBecomeActiveNotification)) { _ in service.refresh() }
        .sheet(item:Binding(get:{ service.replacement },set:{ if $0 == nil { service.cancelReplacement() } })) { request in
            VStack(alignment:.leading,spacing:16) {
                Text(String(localized:"Choose a replacement app",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.headline)
                Text(request.kind.title).foregroundStyle(.secondary)
                List(request.applications,selection:$selectedApplication) { app in
                    HStack(spacing:10) {
                        Image(nsImage:NSWorkspace.shared.icon(forFile:app.url.path)).resizable().frame(width:24,height:24)
                        VStack(alignment:.leading) {
                            Text(app.name)
                            Text(app.url.path).font(.caption).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                        }
                    }.tag(app.url)
                }.frame(height:200).accessibilityLabel(String(localized:"Compatible apps",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                HStack {
                    Spacer()
                    Button(String(localized:"Cancel",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { service.cancelReplacement() }.keyboardShortcut(.cancelAction)
                    Button(String(localized:"Use Selected App",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) {
                        if let app = request.applications.first(where:{ $0.url == selectedApplication }) { Task { await service.chooseReplacement(app) } }
                    }.disabled(selectedApplication == nil).keyboardShortcut(.defaultAction)
                }
            }.padding(24).frame(width:460)
                .onAppear { selectedApplication = nil }
        }
    }
}
