import SwiftUI

struct BandwidthSettingsView: View {
    @EnvironmentObject var store: LibraryStore

    var body: some View {
        VStack(alignment:.leading,spacing:18) {
            VStack(alignment:.leading,spacing:6) {
                Text(String(localized:"Bandwidth limits",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:21,weight:.semibold,design:.rounded))
                Text(String(localized:"Limits apply across all transfers. Use 0 for unlimited.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                    .font(.system(size:11)).foregroundStyle(.secondary)
            }
            Label(store.bandwidthStatus,systemImage:store.alternativeLimitsEnabled ? "tortoise.fill" : "speedometer")
                .font(.system(size:12,weight:.medium)).fixedSize(horizontal:false,vertical:true)
                .padding(12).frame(maxWidth:.infinity,alignment:.leading)
                .background(Design.green.opacity(0.08),in:RoundedRectangle(cornerRadius:9))
                .accessibilityLabel(store.bandwidthStatus)
            if !store.connected {
                Text(String(localized:"Connect to the engine to change limits. Displayed limits are the last known values.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                    .font(.system(size:11)).foregroundStyle(.secondary)
            }
            if let error = store.error { InlineNotice(title:String(localized:"Could not change limits",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),message:error) }
            BandwidthProfileEditor(alternative:false)
            BandwidthProfileEditor(alternative:true)
            Text(String(localized:"Alternative limits override scheduled caps. Scheduled pauses still apply.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                .font(.system(size:10)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true)
        }
    }
}

struct BandwidthDraft {
    static func bytes(_ text: String, direction: String) throws -> UInt32 {
        guard let value = LocaleText.wholeNumber(text) else {
            throw UserFacingError(message:String(localized:"Limit must be a whole number from 0 to 4,294,967.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
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
    private var changed: Bool { LocaleText.wholeNumber(down) != savedDown / 1000 || LocaleText.wholeNumber(up) != savedUp / 1000 }

    var body: some View {
        VStack(alignment:.leading,spacing:12) {
            HStack {
                Label(alternative ? String(localized:"Alternative limits",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Global limits",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),systemImage:alternative ? "tortoise" : "globe")
                    .font(.system(size:13,weight:.semibold))
                Spacer()
                Text(active ? (store.connected ? String(localized:"Active",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Last used",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) : alternative && !store.hasAlternativeLimits ? String(localized:"Not set",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Standby",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                    .font(.system(size:10,weight:.medium)).foregroundStyle(active ? Design.green : .secondary)
                    .padding(.horizontal,7).padding(.vertical,3)
                    .background(active ? Design.green.opacity(0.1) : Color.secondary.opacity(0.08),in:Capsule())
            }
            Text(alternative && !store.hasAlternativeLimits ? String(localized:"No alternative limits saved",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Saved: ↓ \(SizeText.limit(savedDown)) · ↑ \(SizeText.limit(savedUp))",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                .font(.system(size:11)).foregroundStyle(.secondary)
            HStack(spacing:20) {
                limitField(String(localized:"Download",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),image:"arrow.down",text:$down,focus:.download)
                limitField(String(localized:"Upload",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),image:"arrow.up",text:$up,focus:.upload)
            }.disabled(!editable)
            if alternative {
                Divider()
                Toggle(String(localized:"Use alternative limits",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),isOn:Binding(get:{store.alternativeLimitsEnabled},set:{store.toggleAlternativeLimits($0)}))
                    .toggleStyle(.switch).controlSize(.small)
                    .disabled(!store.canToggleAlternativeLimits || saving)
                    .accessibilityHint(store.alternativeLimitsHint)
                if !store.hasAlternativeLimits {
                    Label(String(localized:"Set and save a download or upload limit above 0 to enable the switch.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),systemImage:"info.circle")
                        .font(.system(size:11)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true)
                }
            }
            HStack(alignment:.center,spacing:10) {
                if let error {
                    Text(error).font(.system(size:11)).foregroundStyle(.orange)
                        .fixedSize(horizontal:false,vertical:true).announceAccessibility(error)
                } else if changed {
                    Text(String(localized:"Unsaved changes",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:10)).foregroundStyle(.secondary)
                }
                Spacer(minLength:0)
                Button(saving ? String(localized:"Saving…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : alternative ? String(localized:"Save alternative limits",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Save global limits",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),action:save)
                    .disabled(!editable || !changed)
            }
        }.padding(16).frame(maxWidth:.infinity,alignment:.leading)
            .background(Design.card,in:RoundedRectangle(cornerRadius:11))
            .overlay(RoundedRectangle(cornerRadius:11).stroke(Design.line))
            .onAppear { down = LocaleText.integer(savedDown / 1000); up = LocaleText.integer(savedUp / 1000) }
            .onChange(of:savedDown) { old, new in if LocaleText.wholeNumber(down) == old / 1000 { down = LocaleText.integer(new / 1000) } }
            .onChange(of:savedUp) { old, new in if LocaleText.wholeNumber(up) == old / 1000 { up = LocaleText.integer(new / 1000) } }
    }

    private func limitField(_ title: String, image: String, text: Binding<String>, focus: Field) -> some View {
        VStack(alignment:.leading,spacing:6) {
            Label(title,systemImage:image).font(.system(size:11)).foregroundStyle(.secondary)
            HStack(spacing:6) {
                TextField("0",text:text).textFieldStyle(.roundedBorder).multilineTextAlignment(.trailing)
                    .focused($field,equals:focus)
                    .accessibilityLabel(limitAccessibilityLabel(focus))
                    .accessibilityHint(String(localized:"Enter 0 for unlimited",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                    .onSubmit { if focus == .download { field = .upload } else { save() } }
                Text(String(localized:"KB/s",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:11)).foregroundStyle(.secondary)
            }
        }.frame(maxWidth:.infinity)
    }

    private func limitAccessibilityLabel(_ focus: Field) -> String {
        if alternative {
            return focus == .download ? String(localized:"Alternative download limit in kilobytes per second",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Alternative upload limit in kilobytes per second",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        }
        return focus == .download ? String(localized:"Global download limit in kilobytes per second",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Global upload limit in kilobytes per second",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
    }

    private func save() {
        guard editable else { return }
        let download: UInt32, upload: UInt32
        do { download = try BandwidthDraft.bytes(down,direction:String(localized:"Download",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) }
        catch { self.error = error.localizedDescription; field = .download; return }
        do { upload = try BandwidthDraft.bytes(up,direction:String(localized:"Upload",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) }
        catch { self.error = error.localizedDescription; field = .upload; return }
        saving = true; error = nil
        Task {
            defer { saving = false }
            do {
                if alternative {
                    try await store.setAlternativeLimits(download:download,upload:upload,enabled:store.alternativeLimitsEnabled && (download > 0 || upload > 0))
                } else { try await store.setLimits(download:download,upload:upload) }
                down = LocaleText.integer(download / 1000); up = LocaleText.integer(upload / 1000)
            } catch { self.error = error.localizedDescription }
        }
    }
}
