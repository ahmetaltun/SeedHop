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
    var body: some View {
        VStack(alignment:.leading,spacing:0) {
            HStack(spacing:12) { BrandMark(size:38); VStack(alignment:.leading,spacing:4) { Text("Make yourself at home.").font(.system(size:21,weight:.semibold,design:.rounded)); Text("A few thoughtful defaults. Your way from here.").font(.system(size:11)).foregroundStyle(.secondary) }; Spacer() }.padding(26)
            Picker("Settings section",selection:$tab) { Label("General",systemImage:"slider.horizontal.3").tag(0); Label("Transfers",systemImage:"arrow.down.arrow.up").tag(1); Label("About",systemImage:"leaf").tag(2) }.pickerStyle(.segmented).labelsHidden().padding(.horizontal,26).padding(.bottom,23)
            Divider()
            ScrollView {
                VStack(alignment:.leading,spacing:22) {
                    if tab == 0 {
                        group("Download location") {
                            HStack(spacing:12) { Image(systemName:"folder").foregroundStyle(Design.green); VStack(alignment:.leading,spacing:4) { Text(URL(fileURLWithPath:store.downloadDirectory).lastPathComponent).font(.system(size:12,weight:.medium)); Text(store.downloadDirectory.replacingOccurrences(of:NSHomeDirectory(),with:"~")).font(.system(size:10)).foregroundStyle(.secondary) }; Spacer(); Button("Choose…",action:chooseFolder) }
                        }
                        group("When you step away") {
                            VStack(alignment:.leading,spacing:18) {
                                Toggle(isOn:Binding(get:{store.backgroundEnabled},set:{ enabled in Task { await store.setBackground(enabled) } })) { settingText("Keep transfers running after Quit", "An optional background engine continues your downloads.") }.disabled(store.switching)
                                Divider()
                                Toggle(isOn:Binding(get:{store.resumeAtLogin},set:{ enabled in Task { await store.setLogin(enabled) } })) { settingText("Resume transfers when I log in", "Previously paused transfers will stay paused.") }.disabled(!store.backgroundEnabled || store.switching)
                            }.toggleStyle(.switch).controlSize(.small)
                        }
                        group("Notifications") {
                            Toggle(isOn:Binding(get:{store.completionNotifications},set:{ enabled in
                                if enabled { UNUserNotificationCenter.current().requestAuthorization(options:[.alert,.sound]) { granted,_ in Task { @MainActor in store.completionNotifications = granted; UserDefaults.standard.set(granted,forKey:"completionNotifications") } } }
                                else { store.completionNotifications = false; UserDefaults.standard.set(false,forKey:"completionNotifications") }
                            })) { settingText("Let me know when a download finishes", "A quiet notification when your files are ready.") }.toggleStyle(.switch).controlSize(.small)
                        }
                    } else if tab == 1 {
                        group("Bandwidth") {
                            VStack(alignment:.leading,spacing:16) {
                                settingText("Leave room for everything else", "Set a transfer limit in KB/s. Use 0 for unlimited.")
                                HStack { Label("Download",systemImage:"arrow.down"); Spacer(); TextField("Unlimited",text:$down).frame(width:90).accessibilityLabel("Download limit in kilobytes per second").accessibilityHint("Enter 0 for unlimited"); Text("KB/s").foregroundStyle(.secondary) }.font(.system(size:12))
                                HStack { Label("Upload",systemImage:"arrow.up"); Spacer(); TextField("Unlimited",text:$up).frame(width:90).accessibilityLabel("Upload limit in kilobytes per second").accessibilityHint("Enter 0 for unlimited"); Text("KB/s").foregroundStyle(.secondary) }.font(.system(size:12))
                                HStack { if let limitError { Text(limitError).font(.system(size:10)).foregroundStyle(.orange) }; Spacer(); Button(saving ? "Saving…" : "Apply limits",action:saveLimits).disabled(saving) }
                            }.textFieldStyle(.roundedBorder)
                        }
                        group("Connection") { VStack(spacing:13) { info("Engine",store.connected ? "Connected" : "Disconnected"); info("Incoming TCP port",store.listenPort == 0 ? "Assigned when connected" : "\(store.listenPort)"); info("Protocol", "BitTorrent v1 + v2") } }
                        Text("This preview re-verifies files after restart. Encrypted peers and advanced connection controls are on the roadmap.").font(.system(size:11)).foregroundStyle(.secondary).lineSpacing(4)
                    } else {
                        VStack(spacing:15) { BrandMark(size:72); Text("SeedHop").font(.system(size:30,weight:.semibold,design:.rounded)); Text("0.1.0 · Milestone 0").font(.system(size:12)).foregroundStyle(.secondary); Text("A thoughtful home for your torrents.").font(.system(size:13)); Text("Built with SwiftUI, AppKit, and Rust.\nFree and open source.").font(.system(size:11)).foregroundStyle(.secondary).multilineTextAlignment(.center).lineSpacing(5) }.frame(maxWidth:.infinity).padding(.vertical,20)
                    }
                    if let error = store.error { InlineNotice(title:"Something needs attention",message:error) }
                }.padding(26)
            }
        }.frame(width:570,height:560).background(Design.canvas).tint(Design.green)
        .onAppear { down = "\(store.downloadLimit / 1000)"; up = "\(store.uploadLimit / 1000)" }
    }
    private func group<Content:View>(_ title: String,@ViewBuilder content: () -> Content) -> some View {
        VStack(alignment:.leading,spacing:11) { SectionEyebrow(title:title); content().padding(17).frame(maxWidth:.infinity,alignment:.leading).background(Design.card,in:RoundedRectangle(cornerRadius:11)).overlay(RoundedRectangle(cornerRadius:11).stroke(Design.line)) }
    }
    private func settingText(_ title:String,_ detail:String) -> some View { VStack(alignment:.leading,spacing:5) { Text(title).font(.system(size:12,weight:.medium)); Text(detail).font(.system(size:10)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true) } }
    private func info(_ title:String,_ value:String) -> some View { HStack { Text(title).foregroundStyle(.secondary); Spacer(); Text(value) }.font(.system(size:11)) }
    private func chooseFolder() { let panel = NSOpenPanel(); panel.canChooseFiles = false; panel.canChooseDirectories = true; panel.directoryURL = URL(fileURLWithPath:store.downloadDirectory); panel.begin { if $0 == .OK, let url = panel.url { store.downloadDirectory = url.path; UserDefaults.standard.set(url.path,forKey:"downloadDirectory") } } }
    private func saveLimits() {
        guard let download = UInt32(down), let upload = UInt32(up), download <= UInt32.max / 1000, upload <= UInt32.max / 1000 else { limitError = "Enter a whole number between 0 and 4,294,967."; return }
        saving = true; limitError = nil
        Task { defer { saving = false }; do { try await store.setLimits(download:download * 1000,upload:upload * 1000) } catch { limitError = error.localizedDescription } }
    }
}
