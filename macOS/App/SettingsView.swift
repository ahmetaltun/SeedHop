import SwiftUI
import ServiceManagement
import UserNotifications

enum AppLanguagePreference {
    static let options: [(id: String, title: String)] = [
        ("en", "English"), ("de", "Deutsch"), ("es", "Español"), ("fr", "Français"),
        ("it", "Italiano"), ("pt-BR", "Português (Brasil)"), ("tr", "Türkçe"),
        ("ru", "Русский"), ("uk", "Українська"), ("pl", "Polski"),
        ("ja", "日本語"), ("ko", "한국어"), ("zh-Hans", "简体中文")
    ]
    static var available: [(id: String, title: String)] {
        options.filter { Bundle.main.localizations.contains($0.id) }
    }
    static var saved: String {
        let domain = WorkspacePreferences.isFixture ? "app.seedhop.workspace-fixtures" : (Bundle.main.bundleIdentifier ?? SeedHopIdentity.app)
        let value = (WorkspacePreferences.defaults.persistentDomain(forName:domain)?["AppleLanguages"] as? [String])?.first ?? ""
        return available.contains(where:{ $0.id == value }) ? value : ""
    }
    static func save(_ language: String) {
        // An absent app override lets macOS resolve its preferred languages normally.
        if language.isEmpty {
            WorkspacePreferences.defaults.removeObject(forKey:"AppleLanguages")
            let preferences = UserDefaults.standard.persistentDomain(forName:UserDefaults.globalDomain)?["AppleLanguages"] as? [String] ?? ["en"]
            let resolved = Bundle.preferredLocalizations(from:available.map(\.id),forPreferences:preferences).first ?? "en"
            AppLocalization.shared.use(resolved)
        } else if available.contains(where:{ $0.id == language }) {
            WorkspacePreferences.defaults.set([language],forKey:"AppleLanguages")
            AppLocalization.shared.use(language)
        }
    }
}

struct SettingsView: View {
    @EnvironmentObject var store: LibraryStore
    @State private var tab = 0
    @State private var language = AppLanguagePreference.saved
    @State private var appearance = AppAppearance.shared
    private enum Field: Hashable { case folder }
    @FocusState private var focusedField: Field?
    init(tab: Int = 0) { _tab = State(initialValue:tab) }
    var body: some View {
        VStack(alignment:.leading,spacing:0) {
            HStack(spacing:12) { BrandMark(size:38); VStack(alignment:.leading,spacing:4) { Text(String(localized:"Make yourself at home.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:21,weight:.semibold,design:.rounded)); Text(String(localized:"A few thoughtful defaults. Your way from here.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:11)).foregroundStyle(.secondary) }; Spacer() }.padding(26)
            Picker(String(localized:"Settings section",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),selection:$tab) { Label(String(localized:"General",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),systemImage:"slider.horizontal.3").tag(0); Label(String(localized:"Transfers",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),systemImage:"arrow.down.arrow.up").tag(1); Label(String(localized:"About",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),systemImage:"leaf").tag(2) }.pickerStyle(.segmented).labelsHidden().padding(.horizontal,26).padding(.bottom,23)
            Divider()
            ScrollView {
                VStack(alignment:.leading,spacing:22) {
                    if let error = store.error { InlineNotice(title:String(localized:"Something needs attention",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),message:error) }
                    if tab == 0 {
                        group(String(localized:"Appearance",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) {
                            VStack(alignment:.leading,spacing:8) {
                                DropdownRow(String(localized:"Theme",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) {
                                    Picker(String(localized:"Theme",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),selection:Binding(get:{appearance.selection},set:{appearance.select($0)})) {
                                        Text(String(localized:"Follow macOS",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).tag(AppearancePreference.system)
                                        Text(String(localized:"Light",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).tag(AppearancePreference.light)
                                        Text(String(localized:"Dark",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).tag(AppearancePreference.dark)
                                    }
                                    .accessibilityIdentifier("settings-appearance")
                                }
                                Text(String(localized:"Appearance changes apply immediately.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                                    .font(.system(size:10)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true)
                            }
                        }
                        group(String(localized:"Language",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) {
                            VStack(alignment:.leading,spacing:8) {
                                DropdownRow(String(localized:"Display language",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) {
                                    Picker(String(localized:"Display language",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),selection:$language) {
                                        Text(String(localized:"Follow macOS",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).tag("")
                                        ForEach(AppLanguagePreference.available,id:\.id) { option in
                                            Text(verbatim:option.title).tag(option.id)
                                        }
                                    }
                                    .accessibilityIdentifier("settings-language")
                                }
                                .onChange(of:language) { _, value in AppLanguagePreference.save(value) }
                                Text(String(localized:"Language changes apply immediately.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                                    .font(.system(size:10)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true)
                            }
                        }
                        group(String(localized:"Download location",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) {
                            HStack(spacing:12) { Image(systemName:"folder").foregroundStyle(Design.green); VStack(alignment:.leading,spacing:4) { Text(URL(fileURLWithPath:store.downloadDirectory).lastPathComponent).font(.system(size:12,weight:.medium)); Text(store.downloadDirectory.replacingOccurrences(of:NSHomeDirectory(),with:"~")).font(.system(size:10)).foregroundStyle(.secondary) }; Spacer(); Button(String(localized:"Choose…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),action:chooseFolder).focused($focusedField,equals:.folder).accessibilityLabel(String(localized:"Choose default download folder",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) }
                        }
                        group(String(localized:"When you step away",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) {
                            VStack(alignment:.leading,spacing:18) {
                                Toggle(isOn:Binding(get:{store.backgroundEnabled},set:{ enabled in Task { await store.setBackground(enabled) } })) { settingText(String(localized:"Keep transfers running after Quit",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale), String(localized:"An optional background engine continues your downloads.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) }.disabled(store.switching || store.hasFileOperation || store.savingEncryption || store.savingSchedule)
                                Divider()
                                Toggle(isOn:Binding(get:{store.resumeAtLogin},set:{ enabled in Task { await store.setLogin(enabled) } })) { settingText(String(localized:"Resume transfers when I log in",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale), String(localized:"Previously paused transfers will stay paused.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) }.disabled(!store.backgroundEnabled || store.switching || store.savingEncryption)
                            }.toggleStyle(.switch).controlSize(.small)
                        }
                        group(String(localized:"Notifications",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) {
                            Toggle(isOn:Binding(get:{store.completionNotifications},set:{ enabled in
                                if enabled { UNUserNotificationCenter.current().requestAuthorization(options:[.alert,.sound]) { granted,_ in Task { @MainActor in store.completionNotifications = granted; UserDefaults.standard.set(granted,forKey:"completionNotifications") } } }
                                else { store.completionNotifications = false; UserDefaults.standard.set(false,forKey:"completionNotifications") }
                            })) { settingText(String(localized:"Let me know when a download finishes",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale), String(localized:"A quiet notification when your files are ready.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) }.toggleStyle(.switch).controlSize(.small)
                        }
                    } else if tab == 1 {
                        group(String(localized:"Peer encryption",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) {
                            VStack(alignment:.leading,spacing:12) {
                                DropdownRow(String(localized:"Connection policy",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) {
                                    Picker(String(localized:"Connection policy",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),selection:Binding(get:{store.savedPeerEncryption ?? .allow},set:{store.saveEncryption($0)})) {
                                        ForEach(PeerEncryptionPolicy.allCases) { policy in Text(policy.title).tag(policy) }
                                    }.disabled(!store.canSaveEncryption)
                                }
                                if let policy = store.savedPeerEncryption { Text(policy.detail).font(.system(size:11)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true) }
                                info(store.connected ? String(localized:"Active policy",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Last known policy",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale), store.peerEncryption?.title ?? String(localized:"Not reported",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                                if store.savingEncryption { Text(String(localized:"Saving preference…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:11)).foregroundStyle(.secondary) }
                                if store.encryptionRestartPending {
                                    Text(String(localized:"Saved for the next engine start. Restart briefly stops connections and rechecks files. Paused transfers stay paused; running transfers follow the queue.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:11)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true)
                                    Button(store.switching ? String(localized:"Restarting engine…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Restart engine to apply",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { store.restartForEncryption() }.disabled(!store.canRestartForEncryption)
                                } else { Text(String(localized:"Policy changes apply after restarting the engine.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:11)).foregroundStyle(.secondary) }
                                Text(String(localized:"Peer encryption is for BitTorrent interoperability. It does not hide your IP address or encrypt tracker and discovery traffic.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:10)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true)
                            }
                        }
                        group(String(localized:"Connection",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) {
                            VStack(alignment:.leading,spacing:13) {
                                info(String(localized:"Engine",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),store.switching ? String(localized:"Restarting",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : store.connected ? String(localized:"Connected",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Disconnected",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                                info(String(localized:"TCP and UDP listening port",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),store.connectionPortDescription)
                                info(String(localized:"Peer transports",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale), String(localized:"TCP and uTP (UDP)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                                info(String(localized:"Torrent formats",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale), String(localized:"BitTorrent v1, v2, and hybrid",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                                Text(String(localized:"A listening port does not confirm incoming reachability. Routers, firewalls, and the network can block connections.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:10)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true)
                            }
                        }
                        group(String(localized:"Download queue",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) {
                            VStack(alignment:.leading,spacing:12) {
                                settingText(String(localized:"Active downloads",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale), String(localized:"Completed transfers keep seeding. Paused transfers stay paused.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                                DropdownRow(String(localized:"Download at once",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) {
                                    Picker(String(localized:"Download at once",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),selection:Binding(get:{store.maxActiveDownloads},set:{store.setQueueLimit($0)})) {
                                        Text(String(localized:"Unlimited",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).tag(UInt32(0))
                                        ForEach([1,2,3,4,5,8,10],id:\.self) { value in Text("\(value)").tag(UInt32(value)) }
                                    }.disabled(!store.canChangeTransfers || store.savingQueue)
                                }
                            }
                        }
                        BandwidthSettingsView()
                        group(String(localized:"Weekly schedule",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { SchedulingSettingsView() }
                        Text(String(localized:"SeedHop rechecks saved files after an engine restart.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:11)).foregroundStyle(.secondary)

                    } else {
                        VStack(spacing:15) { BrandMark(size:72); Text("SeedHop").font(.system(size:30,weight:.semibold,design:.rounded)); Text("0.1.0 · Milestone 0").font(.system(size:12)).foregroundStyle(.secondary); Text(String(localized:"A thoughtful home for your torrents.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:13)); Text(String(localized:"Built with SwiftUI, AppKit, and Rust.\nFree and open source.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:11)).foregroundStyle(.secondary).multilineTextAlignment(.center).lineSpacing(5) }.frame(maxWidth:.infinity).padding(.vertical,20)
                    }
                }.padding(26)
            }
        }.frame(width:570,height:660).background(Design.canvas).tint(Design.green)
    }
    private func group<Content:View>(_ title: String,@ViewBuilder content: () -> Content) -> some View {
        VStack(alignment:.leading,spacing:11) { SectionEyebrow(title:title); content().padding(17).frame(maxWidth:.infinity,alignment:.leading).background(Design.card,in:RoundedRectangle(cornerRadius:11)).overlay(RoundedRectangle(cornerRadius:11).stroke(Design.line)) }
    }
    private func settingText(_ title:String,_ detail:String) -> some View { VStack(alignment:.leading,spacing:5) { Text(title).font(.system(size:12,weight:.medium)); Text(detail).font(.system(size:10)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true) } }
    private func info(_ title:String,_ value:String) -> some View { HStack { Text(title).foregroundStyle(.secondary); Spacer(); Text(value) }.font(.system(size:11)) }
    private func chooseFolder() { let panel = NSOpenPanel(); panel.canChooseFiles = false; panel.canChooseDirectories = true; panel.directoryURL = URL(fileURLWithPath:store.downloadDirectory); panel.begin { focusedField = .folder; if $0 == .OK, let url = panel.url { store.downloadDirectory = url.path; UserDefaults.standard.set(url.path,forKey:"downloadDirectory") } } }
}
