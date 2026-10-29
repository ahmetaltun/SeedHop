import SwiftUI

struct BandwidthSettingsView: View {
    @EnvironmentObject var store: LibraryStore

    var body: some View {
        VStack(alignment:.leading,spacing:18) {
            VStack(alignment:.leading,spacing:6) {
                Text("Bandwidth limits").font(.system(size:21,weight:.semibold,design:.rounded))
                Text("Limits apply across all transfers. Use 0 for unlimited.")
                    .font(.system(size:11)).foregroundStyle(.secondary)
            }
            Label(store.bandwidthStatus,systemImage:store.alternativeLimitsEnabled ? "tortoise.fill" : "speedometer")
                .font(.system(size:12,weight:.medium)).fixedSize(horizontal:false,vertical:true)
                .padding(12).frame(maxWidth:.infinity,alignment:.leading)
                .background(Design.green.opacity(0.08),in:RoundedRectangle(cornerRadius:9))
                .accessibilityLabel(store.bandwidthStatus)
            if !store.connected {
                Text("Connect to the engine to change limits. Displayed limits are the last known values.")
                    .font(.system(size:11)).foregroundStyle(.secondary)
            }
            if let error = store.error { InlineNotice(title:"Could not change limits",message:error) }
            BandwidthProfileEditor(alternative:false)
            BandwidthProfileEditor(alternative:true)
            Text("Alternative limits override scheduled caps. Scheduled pauses still apply.")
                .font(.system(size:10)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true)
        }
    }
}

struct BandwidthDraft {
    static func bytes(_ text: String, direction: String) throws -> UInt32 {
        guard let value = UInt32(text.trimmingCharacters(in:.whitespacesAndNewlines)), value <= UInt32.max / 1000 else {
            throw UserFacingError(message:"\(direction) limit must be a whole number from 0 to 4,294,967.")
        }
        return value * 1000
    }
}

private struct BandwidthProfileEditor: View {
    @EnvironmentObject var store: LibraryStore
    let alternative: Bool
    @State private var down = ""
    @State private var up = ""
    @State private var error: String?
    @State private var saving = false
    private enum Field: Hashable { case download, upload }
    @FocusState private var field: Field?

    private var savedDown: UInt32 { alternative ? store.alternativeDownloadLimit : store.normalDownloadLimit }
    private var savedUp: UInt32 { alternative ? store.alternativeUploadLimit : store.normalUploadLimit }
    private var active: Bool { alternative ? store.alternativeLimitsEnabled : !store.alternativeLimitsEnabled && !store.scheduleActive }
    private var editable: Bool { store.canChangeBandwidth && (!alternative || store.alternativeLimitsAvailable) && !saving }
    private var changed: Bool { down != "\(savedDown / 1000)" || up != "\(savedUp / 1000)" }

    var body: some View {
        VStack(alignment:.leading,spacing:12) {
            HStack {
                Label(alternative ? "Alternative limits" : "Global limits",systemImage:alternative ? "tortoise" : "globe")
                    .font(.system(size:13,weight:.semibold))
                Spacer()
                Text(active ? (store.connected ? "Active" : "Last used") : alternative && !store.hasAlternativeLimits ? "Not set" : "Standby")
                    .font(.system(size:10,weight:.medium)).foregroundStyle(active ? Design.green : .secondary)
                    .padding(.horizontal,7).padding(.vertical,3)
                    .background(active ? Design.green.opacity(0.1) : Color.secondary.opacity(0.08),in:Capsule())
            }
            Text(alternative && !store.hasAlternativeLimits ? "No alternative limits saved" : "Saved: ↓ \(SizeText.limit(savedDown)) · ↑ \(SizeText.limit(savedUp))")
                .font(.system(size:11)).foregroundStyle(.secondary)
            HStack(spacing:20) {
                limitField("Download",image:"arrow.down",text:$down,focus:.download)
                limitField("Upload",image:"arrow.up",text:$up,focus:.upload)
            }.disabled(!editable)
            if alternative {
                Divider()
                Toggle("Use alternative limits",isOn:Binding(get:{store.alternativeLimitsEnabled},set:{store.toggleAlternativeLimits($0)}))
                    .toggleStyle(.switch).controlSize(.small)
                    .disabled(!store.canToggleAlternativeLimits || saving)
                    .accessibilityHint(store.alternativeLimitsHint)
                if !store.hasAlternativeLimits {
                    Label("Set and save a download or upload limit above 0 to enable the switch.",systemImage:"info.circle")
                        .font(.system(size:11)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true)
                }
            }
            HStack(alignment:.center,spacing:10) {
                if let error {
                    Text(error).font(.system(size:11)).foregroundStyle(.orange)
                        .fixedSize(horizontal:false,vertical:true).announceAccessibility(error)
                } else if changed {
                    Text("Unsaved changes").font(.system(size:10)).foregroundStyle(.secondary)
                }
                Spacer(minLength:0)
                Button(saving ? "Saving…" : alternative ? "Save alternative limits" : "Save global limits",action:save)
                    .disabled(!editable || !changed)
            }
        }.padding(16).frame(maxWidth:.infinity,alignment:.leading)
            .background(Design.card,in:RoundedRectangle(cornerRadius:11))
            .overlay(RoundedRectangle(cornerRadius:11).stroke(Design.line))
            .onAppear { down = "\(savedDown / 1000)"; up = "\(savedUp / 1000)" }
            .onChange(of:savedDown) { old, new in if down == "\(old / 1000)" { down = "\(new / 1000)" } }
            .onChange(of:savedUp) { old, new in if up == "\(old / 1000)" { up = "\(new / 1000)" } }
    }

    private func limitField(_ title: String, image: String, text: Binding<String>, focus: Field) -> some View {
        VStack(alignment:.leading,spacing:6) {
            Label(title,systemImage:image).font(.system(size:11)).foregroundStyle(.secondary)
            HStack(spacing:6) {
                TextField("0",text:text).textFieldStyle(.roundedBorder).multilineTextAlignment(.trailing)
                    .focused($field,equals:focus)
                    .accessibilityLabel("\(alternative ? "Alternative" : "Global") \(title.lowercased()) limit in kilobytes per second")
                    .accessibilityHint("Enter 0 for unlimited")
                    .onSubmit { if focus == .download { field = .upload } else { save() } }
                Text("KB/s").font(.system(size:11)).foregroundStyle(.secondary)
            }
        }.frame(maxWidth:.infinity)
    }

    private func save() {
        guard editable else { return }
        let download: UInt32, upload: UInt32
        do { download = try BandwidthDraft.bytes(down,direction:"Download") }
        catch { self.error = error.localizedDescription; field = .download; return }
        do { upload = try BandwidthDraft.bytes(up,direction:"Upload") }
        catch { self.error = error.localizedDescription; field = .upload; return }
        saving = true; error = nil
        Task {
            defer { saving = false }
            do {
                if alternative {
                    try await store.setAlternativeLimits(download:download,upload:upload,enabled:store.alternativeLimitsEnabled && (download > 0 || upload > 0))
                } else { try await store.setLimits(download:download,upload:upload) }
                down = "\(download / 1000)"; up = "\(upload / 1000)"
            } catch { self.error = error.localizedDescription }
        }
    }
}
