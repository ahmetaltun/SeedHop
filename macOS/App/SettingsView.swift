import SwiftUI
import ServiceManagement
import UserNotifications

struct SettingsView: View {
    @EnvironmentObject var store: LibraryStore
    @State private var tab = 0
    @State private var down = ""
    @State private var up = ""
    @State private var limitError: String?
    @State private var saving = false
    private enum Field: Hashable { case download, upload, folder }
    @FocusState private var focusedField: Field?
    init(tab: Int = 0) { _tab = State(initialValue:tab) }
    var body: some View {
        VStack(alignment:.leading,spacing:0) {
            HStack(spacing:12) { BrandMark(size:38); VStack(alignment:.leading,spacing:4) { Text("Make yourself at home.").font(.system(size:21,weight:.semibold,design:.rounded)); Text("A few thoughtful defaults. Your way from here.").font(.system(size:11)).foregroundStyle(.secondary) }; Spacer() }.padding(26)
            Picker("Settings section",selection:$tab) { Label("General",systemImage:"slider.horizontal.3").tag(0); Label("Transfers",systemImage:"arrow.down.arrow.up").tag(1); Label("About",systemImage:"leaf").tag(2) }.pickerStyle(.segmented).labelsHidden().padding(.horizontal,26).padding(.bottom,23)
            Divider()
            ScrollView {
                VStack(alignment:.leading,spacing:22) {
                    if let error = store.error { InlineNotice(title:"Something needs attention",message:error) }
                    if tab == 0 {
                        group("Download location") {
                            HStack(spacing:12) { Image(systemName:"folder").foregroundStyle(Design.green); VStack(alignment:.leading,spacing:4) { Text(URL(fileURLWithPath:store.downloadDirectory).lastPathComponent).font(.system(size:12,weight:.medium)); Text(store.downloadDirectory.replacingOccurrences(of:NSHomeDirectory(),with:"~")).font(.system(size:10)).foregroundStyle(.secondary) }; Spacer(); Button("Choose…",action:chooseFolder).focused($focusedField,equals:.folder).accessibilityLabel("Choose default download folder") }
                        }
                        group("When you step away") {
                            VStack(alignment:.leading,spacing:18) {
                                Toggle(isOn:Binding(get:{store.backgroundEnabled},set:{ enabled in Task { await store.setBackground(enabled) } })) { settingText("Keep transfers running after Quit", "An optional background engine continues your downloads.") }.disabled(store.switching || store.hasFileOperation || store.savingEncryption || store.savingSchedule)
                                Divider()
                                Toggle(isOn:Binding(get:{store.resumeAtLogin},set:{ enabled in Task { await store.setLogin(enabled) } })) { settingText("Resume transfers when I log in", "Previously paused transfers will stay paused.") }.disabled(!store.backgroundEnabled || store.switching || store.savingEncryption)
                            }.toggleStyle(.switch).controlSize(.small)
                        }
                        group("Notifications") {
                            Toggle(isOn:Binding(get:{store.completionNotifications},set:{ enabled in
                                if enabled { UNUserNotificationCenter.current().requestAuthorization(options:[.alert,.sound]) { granted,_ in Task { @MainActor in store.completionNotifications = granted; UserDefaults.standard.set(granted,forKey:"completionNotifications") } } }
                                else { store.completionNotifications = false; UserDefaults.standard.set(false,forKey:"completionNotifications") }
                            })) { settingText("Let me know when a download finishes", "A quiet notification when your files are ready.") }.toggleStyle(.switch).controlSize(.small)
                        }
                    } else if tab == 1 {
                        group("Peer encryption") {
                            VStack(alignment:.leading,spacing:12) {
                                Picker("Connection policy",selection:Binding(get:{store.savedPeerEncryption ?? .allow},set:{store.saveEncryption($0)})) {
                                    ForEach(PeerEncryptionPolicy.allCases) { policy in Text(policy.title).tag(policy) }
                                }.disabled(!store.canSaveEncryption)
                                if let policy = store.savedPeerEncryption { Text(policy.detail).font(.system(size:11)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true) }
                                info(store.connected ? "Active policy" : "Last known policy", store.peerEncryption?.title ?? "Not reported")
                                if store.savingEncryption { Text("Saving preference…").font(.system(size:11)).foregroundStyle(.secondary) }
                                if store.encryptionRestartPending {
                                    Text("Saved for the next engine start. Restart briefly stops connections and rechecks files. Paused transfers stay paused; running transfers follow the queue.").font(.system(size:11)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true)
                                    Button(store.switching ? "Restarting engine…" : "Restart engine to apply") { store.restartForEncryption() }.disabled(!store.canRestartForEncryption)
                                } else { Text("Policy changes apply after restarting the engine.").font(.system(size:11)).foregroundStyle(.secondary) }
                                Text("Peer encryption is for BitTorrent interoperability. It does not hide your IP address or encrypt tracker and discovery traffic.").font(.system(size:10)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true)
                            }
                        }
                        group("Connection") {
                            VStack(alignment:.leading,spacing:13) {
                                info("Engine",store.switching ? "Restarting" : store.connected ? "Connected" : "Disconnected")
                                info("TCP and UDP listening port",store.connectionPortDescription)
                                info("Peer transports", "TCP and uTP (UDP)")
                                info("Torrent formats", "BitTorrent v1, v2, and hybrid")
                                Text("A listening port does not confirm incoming reachability. Routers, firewalls, and the network can block connections.").font(.system(size:10)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true)
                            }
                        }
                        group("Download queue") {
                            VStack(alignment:.leading,spacing:12) {
                                settingText("Active downloads", "Completed transfers keep seeding. Paused transfers stay paused.")
                                Picker("Download at once",selection:Binding(get:{store.maxActiveDownloads},set:{store.setQueueLimit($0)})) {
                                    Text("Unlimited").tag(UInt32(0))
                                    ForEach([1,2,3,4,5,8,10],id:\.self) { value in Text("\(value)").tag(UInt32(value)) }
                                }.disabled(!store.canChangeTransfers || store.savingQueue)
                            }
                        }
                        group("Bandwidth") {
                            VStack(alignment:.leading,spacing:16) {
                                settingText("Normal bandwidth limits", "Set a transfer limit in KB/s. Use 0 for unlimited. Schedule windows can replace these limits.")
                                if store.scheduleActive { Text("Effective caps during this window: download \(store.downloadLimit / 1000) KB/s, upload \(store.uploadLimit / 1000) KB/s.").font(.system(size:11)).foregroundStyle(.secondary) }
                                HStack { Label("Download",systemImage:"arrow.down"); Spacer(); TextField("Unlimited",text:$down).focused($focusedField,equals:.download).onSubmit { focusedField = .upload }.frame(width:90).accessibilityLabel("Download limit in kilobytes per second").accessibilityHint("Enter 0 for unlimited"); Text("KB/s").foregroundStyle(.secondary) }.font(.system(size:12))
                                HStack { Label("Upload",systemImage:"arrow.up"); Spacer(); TextField("Unlimited",text:$up).focused($focusedField,equals:.upload).onSubmit(saveLimits).frame(width:90).accessibilityLabel("Upload limit in kilobytes per second").accessibilityHint("Enter 0 for unlimited"); Text("KB/s").foregroundStyle(.secondary) }.font(.system(size:12))
                                HStack { if let limitError { Text(limitError).font(.system(size:10)).foregroundStyle(.orange).announceAccessibility("Bandwidth limit error. \(limitError)") }; Spacer(); Button(saving ? "Saving…" : "Apply limits",action:saveLimits).disabled(saving) }
                            }.textFieldStyle(.roundedBorder).disabled(!store.canChangeTransfers)
                        }
                        group("Weekly schedule") { SchedulingSettingsView() }
                        Text("SeedHop rechecks saved files after an engine restart.").font(.system(size:11)).foregroundStyle(.secondary)

                    } else {
                        VStack(spacing:15) { BrandMark(size:72); Text("SeedHop").font(.system(size:30,weight:.semibold,design:.rounded)); Text("0.1.0 · Milestone 0").font(.system(size:12)).foregroundStyle(.secondary); Text("A thoughtful home for your torrents.").font(.system(size:13)); Text("Built with SwiftUI, AppKit, and Rust.\nFree and open source.").font(.system(size:11)).foregroundStyle(.secondary).multilineTextAlignment(.center).lineSpacing(5) }.frame(maxWidth:.infinity).padding(.vertical,20)
                    }
                }.padding(26)
            }
        }.frame(width:570,height:660).background(Design.canvas).tint(Design.green)
        .onAppear { down = "\(store.normalDownloadLimit / 1000)"; up = "\(store.normalUploadLimit / 1000)" }
    }
    private func group<Content:View>(_ title: String,@ViewBuilder content: () -> Content) -> some View {
        VStack(alignment:.leading,spacing:11) { SectionEyebrow(title:title); content().padding(17).frame(maxWidth:.infinity,alignment:.leading).background(Design.card,in:RoundedRectangle(cornerRadius:11)).overlay(RoundedRectangle(cornerRadius:11).stroke(Design.line)) }
    }
    private func settingText(_ title:String,_ detail:String) -> some View { VStack(alignment:.leading,spacing:5) { Text(title).font(.system(size:12,weight:.medium)); Text(detail).font(.system(size:10)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true) } }
    private func info(_ title:String,_ value:String) -> some View { HStack { Text(title).foregroundStyle(.secondary); Spacer(); Text(value) }.font(.system(size:11)) }
    private func chooseFolder() { let panel = NSOpenPanel(); panel.canChooseFiles = false; panel.canChooseDirectories = true; panel.directoryURL = URL(fileURLWithPath:store.downloadDirectory); panel.begin { focusedField = .folder; if $0 == .OK, let url = panel.url { store.downloadDirectory = url.path; UserDefaults.standard.set(url.path,forKey:"downloadDirectory") } } }
    private func saveLimits() {
        guard !saving, store.canChangeTransfers else { return }
        guard let download = UInt32(down), download <= UInt32.max / 1000 else { limitError = "Download limit must be a whole number from 0 to 4,294,967."; focusedField = .download; return }
        guard let upload = UInt32(up), upload <= UInt32.max / 1000 else { limitError = "Upload limit must be a whole number from 0 to 4,294,967."; focusedField = .upload; return }
        saving = true; limitError = nil
        Task { defer { saving = false }; do { try await store.setLimits(download:download * 1000,upload:upload * 1000) } catch { limitError = error.localizedDescription } }
    }
}
