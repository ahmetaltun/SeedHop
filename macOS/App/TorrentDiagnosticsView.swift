import SwiftUI
import AppKit
import Charts

struct TorrentDiagnosticsView: View {
    @EnvironmentObject var store: LibraryStore
    let torrent: TorrentItem
    let tab: String
    @State private var peerSort = [KeyPathComparator(\DiagnosticPeerRow.endpoint)]
    @State private var trackerSort = [KeyPathComparator(\DiagnosticTrackerRow.address)]

    private var details: TorrentDetails? { store.details.flatMap { $0.torrentId == torrent.id ? $0 : nil } }
    private var currentSection: Bool { details?.section == tab }
    private var status: TransferPresentation { store.presentation(torrent) }

    var body: some View {
        VStack(spacing:0) {
            if !store.connected {
                notice(String(localized:"Last known information. Reconnect to see current activity.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),symbol:"clock")
            } else if let error = store.detailsError {
                notice(String(localized:"Details could not refresh. Showing the last observation. \(error)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),symbol:"exclamationmark.circle")
            }
            switch tab {
            case "peers": peers
            case "trackers": trackers
            case "activity": activity
            default: general
            }
        }.font(.system(size:11)).frame(maxWidth:.infinity,maxHeight:.infinity)
    }

    private var general: some View {
        ScrollView {
            VStack(alignment:.leading,spacing:14) {
                HStack(alignment:.top) {
                    VStack(alignment:.leading,spacing:4) {
                        Text(status.label).font(.system(size:12,weight:.semibold))
                        Text(status.detail).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true)
                    }
                    Spacer()
                    Button(String(localized:"Show in Finder",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { store.revealDestination(torrent) }
                        .accessibilityLabel(String(localized:"Show destination for \(torrent.name) in Finder",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                }
                LazyVGrid(columns:[GridItem(.flexible(),alignment:.leading),GridItem(.flexible(),alignment:.leading),GridItem(.flexible(),alignment:.leading)],alignment:.leading,spacing:14) {
                    metric(String(localized:"Selected size",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),SizeText.bytes(torrent.selectedBytes ?? torrent.totalBytes))
                    metric(String(localized:"Full torrent size",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),SizeText.bytes(torrent.fullBytes))
                    metric(status.progressMetricTitle,status.progress == nil ? "—" : SizeText.bytes(status.completedBytes))
                    metric(String(localized:"Remaining selected bytes",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),status.isChecking || status.isError ? "—" : SizeText.bytes(torrent.remainingBytes))
                    metric(String(localized:"Estimated time remaining",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),duration(torrent.etaSeconds))
                    metric(store.connected ? String(localized:"Connected peers",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Last known peers",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),String(torrent.peers))
                    metric(String(localized:"Downloaded traffic · lifetime",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),bytes(torrent.networkDownloadedBytes))
                    metric(String(localized:"Uploaded traffic · lifetime",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),bytes(torrent.networkUploadedBytes))
                    metric(String(localized:"Share ratio · lifetime",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),torrent.shareRatio.map { $0.formatted(.number.precision(.fractionLength(2))) } ?? "—")
                    metric(String(localized:"Downloaded traffic · engine session",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),details.map { SizeText.bytes($0.sessionDownloaded) } ?? "—")
                    metric(String(localized:"Uploaded traffic · engine session",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),details.map { SizeText.bytes($0.sessionUploaded) } ?? "—")
                    metric(String(localized:"Seeding duration",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),duration(details?.seedingSeconds))
                    metric(String(localized:"Added",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),timestamp(torrent.addedAt))
                    metric(String(localized:"Completed",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),timestamp(torrent.completedAt))
                    metric(String(localized:"Torrent format",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),details.map { $0.private ? String(localized:"\($0.format.uppercased()) · Private",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"\($0.format.uppercased()) · Public",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) } ?? "—")
                }
                Text(String(localized:"Verified content and network traffic are separate measurements. Historical values are unavailable for transfers added before statistics were recorded.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                    .font(.system(size:10)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true)
                Divider()
                LabeledContent(String(localized:"Destination",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { Text(torrent.destination).textSelection(.enabled).lineLimit(2).truncationMode(.middle) }
                if let hash = details?.v1Hash { hashRow(String(localized:"BitTorrent v1 info hash",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),hash) }
                if let hash = details?.v2Hash { hashRow(String(localized:"BitTorrent v2 info hash",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),hash) }
                if details == nil { Text(store.connected ? String(localized:"Additional metadata is loading…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Additional metadata is unavailable while disconnected.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).foregroundStyle(.secondary) }
            }.padding(14)
        }
    }

    private var peers: some View {
        VStack(spacing:0) {
            HStack {
                Toggle(String(localized:"Show known connection attempts",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),isOn:$store.allPeers).toggleStyle(.checkbox)
                Spacer()
                Text(details.map { String(localized:"\($0.peers.filter { $0.state == "live" }.count) connected",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) } ?? String(localized:"Peers not sampled",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).foregroundStyle(.secondary)
            }.padding(.horizontal,12).padding(.vertical,7)
            if currentSection, let details, !details.peers.isEmpty {
                peerTable(details.peers)
            } else {
                empty(String(localized:"No peer records",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),detail:!currentSection && store.connected ? String(localized:"Loading peer information…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : torrent.isPaused ? String(localized:"This transfer is paused. Live peer information is unavailable.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"No peer connections are reported. This does not establish a tracker or network failure.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
            }
            Text(String(localized:"Rates use observed payload-counter changes. Encryption describes peer payloads and does not hide your IP address.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                .font(.system(size:10)).foregroundStyle(.secondary).padding(.horizontal,12).padding(.vertical,6)
        }
    }

    private func peerTable(_ peers: [PeerDetails]) -> some View {
        let rows = peers.map { DiagnosticPeerRow(peer:$0,rate:store.peerRates[$0.id]) }.sorted(using:peerSort)
        return Table(rows,sortOrder:$peerSort) {
            TableColumn(String(localized:"Endpoint",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),value:\.endpoint) { Text($0.endpoint).textSelection(.enabled) }.width(min:140,ideal:170)
            TableColumn(String(localized:"Client",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),value:\.client) { Text($0.client).lineLimit(1) }.width(min:95,ideal:120)
            TableColumn(String(localized:"State",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),value:\.state) { Text($0.state) }.width(min:80,ideal:90)
            TableColumn(String(localized:"Transport",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),value:\.transport) { Text($0.transport) }.width(75)
            TableColumn(String(localized:"Encryption",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),value:\.encryption) { Text($0.encryption) }.width(85)
            TableColumn(String(localized:"Download",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),value:\.download) { Text($0.rate.map { rate($0.download) } ?? "—").monospacedDigit() }.width(90)
            TableColumn(String(localized:"Upload",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),value:\.upload) { Text($0.rate.map { rate($0.upload) } ?? "—").monospacedDigit() }.width(90)
            TableColumn(String(localized:"Downloaded",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),value:\.downloaded) { Text(SizeText.bytes($0.downloaded)).monospacedDigit() }.width(95)
            TableColumn(String(localized:"Uploaded",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),value:\.uploaded) { Text(SizeText.bytes($0.uploaded)).monospacedDigit() }.width(95)
        }.tableStyle(.inset(alternatesRowBackgrounds:true)).accessibilityLabel(String(localized:"Peer connections for \(torrent.name)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
    }

    private var trackers: some View {
        VStack(spacing:0) {
            HStack(spacing:18) {
                discovery("DHT",details?.dht)
                discovery("PEX",details?.pex)
                Text(String(localized:"Local discovery: \(details == nil ? "Unknown" : "Not supported")",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).foregroundStyle(.secondary)
                Spacer()
            }.padding(.horizontal,12).padding(.vertical,8)
            if currentSection, let details, !details.trackers.isEmpty {
                trackerTable(details.trackers)
            } else {
                empty(String(localized:"No tracker records",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),detail:!currentSection && store.connected ? String(localized:"Loading tracker information…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"No trackers are reported for this torrent. Discovery methods are shown separately above.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
            }
            Text(String(localized:"Discovery labels describe enabled methods, not network health. Tracker swarm counts may overlap and are not added together.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                .font(.system(size:10)).foregroundStyle(.secondary).padding(.horizontal,12).padding(.vertical,6)
        }
    }

    private func trackerTable(_ trackers: [TrackerDetails]) -> some View {
        let rows = trackers.map { DiagnosticTrackerRow($0,v1Hash:details?.v1Hash,v2Hash:details?.v2Hash) }.sorted(using:trackerSort)
        return Table(rows,sortOrder:$trackerSort) {
            TableColumn(String(localized:"Tracker",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),value:\.address) { Text($0.address).textSelection(.enabled).lineLimit(1).help($0.address) }.width(min:165,ideal:210)
            TableColumn(String(localized:"Format",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),value:\.format) { Text($0.format) }.width(65)
            TableColumn(String(localized:"State",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),value:\.state) { Text($0.state) }.width(min:80,ideal:90)
            TableColumn(String(localized:"Last result",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),value:\.message) { Text($0.message).lineLimit(1).help($0.message) }.width(min:170,ideal:230)
            TableColumn(String(localized:"Last announce",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),value:\.lastSort) { Text(timestamp($0.tracker.lastAnnounce)).monospacedDigit() }.width(145)
            TableColumn(String(localized:"Next announce",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),value:\.nextSort) { Text(timestamp($0.tracker.nextAnnounce)).monospacedDigit() }.width(145)
            TableColumn(String(localized:"Peers returned",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),value:\.peersSort) { Text(count($0.tracker.peers)).monospacedDigit() }.width(100)
            TableColumn(String(localized:"Seeds reported",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),value:\.seedsSort) { Text(count($0.tracker.seeds)).monospacedDigit() }.width(110)
            TableColumn(String(localized:"Leechers reported",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),value:\.leechersSort) { Text(count($0.tracker.leechers)).monospacedDigit() }.width(125)
            TableColumn(String(localized:"Wire hash",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),value:\.hash) { Text($0.hash).font(.system(size:10,design:.monospaced)).lineLimit(1).truncationMode(.middle).textSelection(.enabled).help(String(localized:"Announced wire identity: \($0.hash)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) }.width(min:100,ideal:140)
        }.tableStyle(.inset(alternatesRowBackgrounds:true)).accessibilityLabel(String(localized:"Tracker observations for \(torrent.name)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
    }

    private var activity: some View {
        let samples = store.history[torrent.id] ?? []
        let events = store.events[torrent.id] ?? []
        return VStack(spacing:0) {
            HStack {
                Text(String(localized:"Traffic · last 15 minutes in this app session",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).foregroundStyle(.secondary)
                Spacer()
                Button(String(localized:"Copy Diagnostics",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { store.copyDiagnostics(torrent) }
                    .help(String(localized:"Copy diagnostics with tracker credentials, peer endpoints, and personal paths removed",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
            }.padding(.horizontal,12).padding(.vertical,7)
            if samples.isEmpty {
                Text(String(localized:"Traffic history appears as live snapshots arrive.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).foregroundStyle(.secondary).frame(maxWidth:.infinity).frame(height:90)
            } else {
                Chart {
                    ForEach(samples) { sample in
                        LineMark(x:.value(String(localized:"Time",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),sample.id),y:.value(String(localized:"Bytes per second",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),Double(sample.download)),series:.value(String(localized:"Direction",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),String(localized:"Download",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)))
                            .foregroundStyle(by:.value(String(localized:"Direction",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),String(localized:"Download",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)))
                        LineMark(x:.value(String(localized:"Time",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),sample.id),y:.value(String(localized:"Bytes per second",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),Double(sample.upload)),series:.value(String(localized:"Direction",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),String(localized:"Upload",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)))
                            .foregroundStyle(by:.value(String(localized:"Direction",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),String(localized:"Upload",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)))
                    }
                }
                .chartForegroundStyleScale([String(localized:"Download",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale):Design.blue,String(localized:"Upload",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale):Design.green])
                .chartYAxis {
                    AxisMarks(position:.leading) { value in
                        AxisGridLine()
                        AxisValueLabel {
                            if let value = value.as(Double.self) {
                                let amount = UInt64(max(0,min(value,1e18)))
                                Text(rate(amount))
                            }
                        }
                    }
                }
                .frame(height:95).padding(.horizontal,14).padding(.bottom,8)
                .accessibilityLabel(String(localized:"Download and upload traffic history for \(torrent.name)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
            }
            Divider()
            if events.isEmpty { empty(String(localized:"No recorded events",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),detail:String(localized:"Transfer changes, tracker failures, recovery, and file operations appear here.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) }
            else {
                List(events.reversed()) { event in
                    HStack(alignment:.top,spacing:12) {
                        Text(event.date,format:.dateTime.hour().minute().second()).monospacedDigit().foregroundStyle(.secondary).frame(width:75,alignment:.leading)
                        Text(event.message).textSelection(.enabled).frame(maxWidth:.infinity,alignment:.leading)
                    }.accessibilityElement(children:.combine)
                }.listStyle(.plain).accessibilityLabel(String(localized:"Recent transfer events",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
            }
        }
    }

    private func metric(_ label: String, _ value: String) -> some View {
        VStack(alignment:.leading,spacing:3) {
            Text(label).foregroundStyle(.secondary)
            Text(value).monospacedDigit().textSelection(.enabled)
        }.accessibilityElement(children:.combine)
    }
    private func hashRow(_ label: String, _ hash: String) -> some View {
        HStack {
            Text(label).foregroundStyle(.secondary)
            Text(hash).font(.system(size:10,design:.monospaced)).textSelection(.enabled).lineLimit(1).truncationMode(.middle).help(hash)
            Spacer(minLength:0)
            Button(String(localized:"Copy",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(hash,forType:.string) }.accessibilityLabel(String(localized:"Copy \(label)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
        }
    }
    private func discovery(_ label: String, _ enabled: Bool?) -> some View {
        Text(enabled.map { $0 ? String(localized:"\(label): Enabled",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"\(label): Disabled",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) } ?? String(localized:"\(label): Unknown",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).foregroundStyle(.secondary)
    }
    private func notice(_ text: String, symbol: String) -> some View {
        Label(text,systemImage:symbol).foregroundStyle(.secondary).frame(maxWidth:.infinity,alignment:.leading)
            .padding(.horizontal,12).padding(.vertical,6).background(Design.amber.opacity(0.06))
    }
    private func empty(_ title: String, detail: String) -> some View {
        VStack(spacing:6) { Text(title).fontWeight(.medium); Text(detail).foregroundStyle(.secondary).multilineTextAlignment(.center) }
            .padding(18).frame(maxWidth:.infinity,maxHeight:.infinity)
    }
    private func bytes(_ value: UInt64?) -> String { value.map(SizeText.bytes) ?? "—" }
    private func count(_ value: UInt64?) -> String { value.map { $0.formatted() } ?? "—" }
    private func rate(_ value: UInt64) -> String { value == 0 ? "0 B/s" : SizeText.rate(value) }
    private func duration(_ seconds: UInt64?) -> String {
        guard let seconds else { return "—" }
        return LocaleText.duration(seconds)
    }
    private func timestamp(_ seconds: UInt64?) -> String {
        seconds.map { Date(timeIntervalSince1970:TimeInterval($0)).formatted(date:.abbreviated,time:.shortened) } ?? "—"
    }
}

private struct DiagnosticPeerRow: Identifiable {
    let peer: PeerDetails
    let rate: PeerRate?
    var id: String { peer.id }
    var endpoint: String { peer.endpoint }
    var client: String { peer.client ?? "—" }
    var state: String {
        let label: String
        switch peer.state {
        case "live": label = String(localized:"Connected",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "connecting": label = String(localized:"Connecting",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "queued": label = String(localized:"Queued",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "dead": label = String(localized:"Disconnected",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "not_needed", "not needed": label = String(localized:"Not needed",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        default: label = String(localized:"Unknown",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        }
        return peer.incoming.map { $0 ? String(localized:"\(label) · Incoming",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"\(label) · Outgoing",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) } ?? label
    }
    var transport: String { peer.transport.map { $0.lowercased() == "utp" ? "uTP" : $0.uppercased() } ?? "—" }
    var encryption: String { peer.encrypted.map { $0 ? String(localized:"Encrypted",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Plaintext",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) } ?? String(localized:"Unknown",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
    var download: UInt64 { rate?.download ?? 0 }
    var upload: UInt64 { rate?.upload ?? 0 }
    var downloaded: UInt64 { peer.downloaded }
    var uploaded: UInt64 { peer.uploaded }
}

private struct DiagnosticTrackerRow: Identifiable {
    let tracker: TrackerDetails
    let format: String
    init(_ tracker: TrackerDetails, v1Hash: String?, v2Hash: String?) {
        self.tracker = tracker
        format = tracker.hash == v1Hash ? "v1" : v2Hash.map { String($0.prefix(40)) == tracker.hash } == true ? "v2" : "—"
    }
    var id: String { tracker.id }
    var address: String { tracker.address + (tracker.endpoint.map { " · " + $0 } ?? "") }
    var state: String {
        switch tracker.state {
        case "not contacted": return String(localized:"Not contacted",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "contacting": return String(localized:"Contacting",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "resolving": return String(localized:"Resolving",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "warning": return String(localized:"Warning",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "error": return String(localized:"Needs attention",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "working": return String(localized:"Working",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        default: return String(localized:"Unknown",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        }
    }
    var message: String { tracker.message ?? "—" }
    var hash: String { tracker.hash }
    var lastSort: UInt64 { tracker.lastAnnounce ?? 0 }
    var nextSort: UInt64 { tracker.nextAnnounce ?? 0 }
    var peersSort: UInt64 { tracker.peers ?? 0 }
    var seedsSort: UInt64 { tracker.seeds ?? 0 }
    var leechersSort: UInt64 { tracker.leechers ?? 0 }
}
