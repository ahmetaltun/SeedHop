import SwiftUI

struct NetworkPrivacySettings: View {
    @EnvironmentObject var store: LibraryStore
    @State private var selection = ""
    @State private var dnsServer = ""
    @State private var loaded = false
    @State private var allowLocalNetwork = false

    private var choices: [NetworkInterface] {
        (store.networkInfo?.interfaces ?? []).filter {
            $0.name.hasPrefix("utun") || $0.name.hasPrefix("ipsec")
        }
    }
    private var selected: NetworkInterface? { choices.first { $0.name == selection } }
    private var canSave: Bool {
        loaded && !store.savingStatisticsRetention && !store.savingNetwork && !store.switching && !store.hasFileOperation && !store.showImport
            && !store.savingEncryption && !store.savingSchedule && !store.savingBandwidth && !store.savingQueue
            && (selection.isEmpty || (selected != nil && !dnsServer.trimmingCharacters(in:.whitespacesAndNewlines).isEmpty))
    }

    var body: some View {
        VStack(alignment:.leading,spacing:12) {
            Label(String(localized:"Network privacy",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),systemImage:"network.badge.shield.half.filled")
                .font(.system(size:12,weight:.semibold))
            Text(!store.connected && !store.networkBlocked
                 ? String(localized:"The engine is stopped. Saved settings apply before transfers start.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
                 : store.networkBlocked
                 ? String(localized:"Transfers blocked. Review your connection and restart the engine.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
                 : store.networkInterface != nil
                 ? String(localized:"Torrent traffic uses the selected interface. This does not guarantee anonymity.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
                 : String(localized:"Direct connection. Your network address is visible to peers.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                .font(.system(size:11)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true)
            DropdownRow(String(localized:"Required connection",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) {
                Picker(String(localized:"Required connection",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),selection:$selection) {
                    Text(String(localized:"Direct connection",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).tag("")
                    ForEach(choices) { interface in Text(verbatim:interface.name).tag(interface.name) }
                    if !selection.isEmpty && selected == nil {
                        Text(String(localized:"Selected connection unavailable",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).tag(selection)
                    }
                }.accessibilityIdentifier("settings-required-network")
            }
            if loaded && store.networkInfo?.saved == nil {
                Text(String(localized:"Saved network settings could not be read. Transfers remain blocked. Apply a new choice to replace them.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                    .font(.system(size:11)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true)
            }
            if !selection.isEmpty {
                TextField(String(localized:"DNS server IP address",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),text:$dnsServer)
                    .textFieldStyle(.roundedBorder).accessibilityIdentifier("settings-network-dns")
                Text(String(localized:"Enter a DNS server reachable through your VPN. DNS uses this connection only, with no system fallback. Changing VPN addresses requires selecting the connection again.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                    .font(.system(size:10)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true)
            }
            Toggle(isOn:$allowLocalNetwork) {
                Text(String(localized:"Allow local trackers and peers",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                    .frame(maxWidth:.infinity,alignment:.leading)
            }.toggleStyle(.switch).controlSize(.small)
            Text(String(localized:"Keep this off for public torrents. Enable only when you trust torrents to contact devices and services on your local network.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                .font(.system(size:10)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true)
            HStack {
                Button(String(localized:"Refresh connections",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) {
                    Task { await store.refreshNetworkInfo() }
                }.disabled(store.switching || store.savingNetwork)
                Spacer()
                Button(store.savingNetwork
                       ? String(localized:"Saving preference…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
                       : String(localized:"Apply and restart engine",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) {
                    store.saveNetworkPolicy(NetworkPolicy(interface:selection.isEmpty ? nil : selected,
                        dnsServer:selection.isEmpty ? nil : dnsServer.trimmingCharacters(in:.whitespacesAndNewlines),allowLocalNetwork:allowLocalNetwork))
                }.disabled(!canSave).accessibilityIdentifier("settings-apply-network")
            }
            Text(String(localized:"Applying stops existing connections before saving. If the connection changes or disappears, transfers stay blocked until you review Settings and restart. Updates use a separate connection.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                .font(.system(size:10)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true)
        }
        .padding(17).frame(maxWidth:.infinity,alignment:.leading)
        .background(Design.card,in:RoundedRectangle(cornerRadius:11))
        .overlay(RoundedRectangle(cornerRadius:11).stroke(Design.line))
        .task {
            await store.refreshNetworkInfo()
            if let info = store.networkInfo {
                selection = info.saved?.interface?.name ?? ""; dnsServer = info.saved?.dnsServer ?? ""; allowLocalNetwork = info.saved?.allowLocalNetwork ?? false; loaded = true
            }
            while !Task.isCancelled {
                do { try await Task.sleep(for:.seconds(2)) } catch { break }
                await store.refreshNetworkInfo()
            }
        }
    }
}
