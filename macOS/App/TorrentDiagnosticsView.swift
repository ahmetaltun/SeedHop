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
                notice("Last known information. Reconnect to see current activity.",symbol:"clock")
            } else if let error = store.detailsError {
                notice("Details could not refresh. Showing the last observation. " + error,symbol:"exclamationmark.circle")
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
                    Button("Show in Finder") { store.revealDestination(torrent) }
                        .accessibilityLabel("Show destination for \(torrent.name) in Finder")
                }
                LazyVGrid(columns:[GridItem(.flexible(),alignment:.leading),GridItem(.flexible(),alignment:.leading),GridItem(.flexible(),alignment:.leading)],alignment:.leading,spacing:14) {
                    metric("Selected size",SizeText.bytes(torrent.selectedBytes ?? torrent.totalBytes))
                    metric("Full torrent size",SizeText.bytes(torrent.fullBytes))
                    metric("\(status.progressTitle) bytes",status.progress == nil ? "—" : SizeText.bytes(status.completedBytes))
                    metric("Remaining selected bytes",status.isChecking || status.isError ? "—" : SizeText.bytes(torrent.remainingBytes))
                    metric("Estimated time remaining",duration(torrent.etaSeconds))
                    metric(store.connected ? "Connected peers" : "Last known peers",String(torrent.peers))
                    metric("Downloaded traffic · lifetime",bytes(torrent.networkDownloadedBytes))
                    metric("Uploaded traffic · lifetime",bytes(torrent.networkUploadedBytes))
                    metric("Share ratio · lifetime",torrent.shareRatio.map { $0.formatted(.number.precision(.fractionLength(2))) } ?? "—")
                    metric("Downloaded traffic · engine session",details.map { SizeText.bytes($0.sessionDownloaded) } ?? "—")
                    metric("Uploaded traffic · engine session",details.map { SizeText.bytes($0.sessionUploaded) } ?? "—")
                    metric("Seeding duration",duration(details?.seedingSeconds))
                    metric("Added",timestamp(torrent.addedAt))
                    metric("Completed",timestamp(torrent.completedAt))
                    metric("Torrent format",details.map { "\($0.format.uppercased()) · \($0.private ? "Private" : "Public")" } ?? "—")
                }
                Text("Verified content and network traffic are separate measurements. Historical values are unavailable for transfers added before statistics were recorded.")
                    .font(.system(size:10)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true)
                Divider()
                LabeledContent("Destination") { Text(torrent.destination).textSelection(.enabled).lineLimit(2).truncationMode(.middle) }
                if let hash = details?.v1Hash { hashRow("BitTorrent v1 info hash",hash) }
                if let hash = details?.v2Hash { hashRow("BitTorrent v2 info hash",hash) }
                if details == nil { Text("Additional metadata is \(store.connected ? "loading…" : "unavailable while disconnected.")").foregroundStyle(.secondary) }
            }.padding(14)
        }
    }

    private var peers: some View {
        VStack(spacing:0) {
            HStack {
                Toggle("Show known connection attempts",isOn:$store.allPeers).toggleStyle(.checkbox)
                Spacer()
                Text(details.map { "\($0.peers.filter { $0.state == "live" }.count) connected" } ?? "Peers not sampled").foregroundStyle(.secondary)
            }.padding(.horizontal,12).padding(.vertical,7)
            if currentSection, let details, !details.peers.isEmpty {
                peerTable(details.peers)
            } else {
                empty("No peer records",detail:!currentSection && store.connected ? "Loading peer information…" : torrent.isPaused ? "This transfer is paused. Live peer information is unavailable." : "No peer connections are reported. This does not establish a tracker or network failure.")
            }
            Text("Rates use observed payload-counter changes. Encryption describes peer payloads and does not hide your IP address.")
                .font(.system(size:10)).foregroundStyle(.secondary).padding(.horizontal,12).padding(.vertical,6)
        }
    }

    private func peerTable(_ peers: [PeerDetails]) -> some View {
        let rows = peers.map { DiagnosticPeerRow(peer:$0,rate:store.peerRates[$0.id]) }.sorted(using:peerSort)
        return Table(rows,sortOrder:$peerSort) {
            TableColumn("Endpoint",value:\.endpoint) { Text($0.endpoint).textSelection(.enabled) }.width(min:140,ideal:170)
            TableColumn("Client",value:\.client) { Text($0.client).lineLimit(1) }.width(min:95,ideal:120)
            TableColumn("State",value:\.state) { Text($0.state) }.width(min:80,ideal:90)
            TableColumn("Transport",value:\.transport) { Text($0.transport) }.width(75)
            TableColumn("Encryption",value:\.encryption) { Text($0.encryption) }.width(85)
            TableColumn("Download",value:\.download) { Text($0.rate.map { rate($0.download) } ?? "—").monospacedDigit() }.width(90)
            TableColumn("Upload",value:\.upload) { Text($0.rate.map { rate($0.upload) } ?? "—").monospacedDigit() }.width(90)
            TableColumn("Downloaded",value:\.downloaded) { Text(SizeText.bytes($0.downloaded)).monospacedDigit() }.width(95)
            TableColumn("Uploaded",value:\.uploaded) { Text(SizeText.bytes($0.uploaded)).monospacedDigit() }.width(95)
        }.tableStyle(.inset(alternatesRowBackgrounds:true)).accessibilityLabel("Peer connections for \(torrent.name)")
    }

    private var trackers: some View {
        VStack(spacing:0) {
            HStack(spacing:18) {
                discovery("DHT",details?.dht)
                discovery("PEX",details?.pex)
                Text("Local discovery: \(details == nil ? "Unknown" : "Not supported")").foregroundStyle(.secondary)
                Spacer()
            }.padding(.horizontal,12).padding(.vertical,8)
            if currentSection, let details, !details.trackers.isEmpty {
                trackerTable(details.trackers)
            } else {
                empty("No tracker records",detail:!currentSection && store.connected ? "Loading tracker information…" : "No trackers are reported for this torrent. Discovery methods are shown separately above.")
            }
            Text("Discovery labels describe enabled methods, not network health. Tracker swarm counts may overlap and are not added together.")
                .font(.system(size:10)).foregroundStyle(.secondary).padding(.horizontal,12).padding(.vertical,6)
        }
    }

    private func trackerTable(_ trackers: [TrackerDetails]) -> some View {
        let rows = trackers.map { DiagnosticTrackerRow($0,v1Hash:details?.v1Hash,v2Hash:details?.v2Hash) }.sorted(using:trackerSort)
        return Table(rows,sortOrder:$trackerSort) {
            TableColumn("Tracker",value:\.address) { Text($0.address).textSelection(.enabled).lineLimit(1).help($0.address) }.width(min:165,ideal:210)
            TableColumn("Format",value:\.format) { Text($0.format) }.width(65)
            TableColumn("State",value:\.state) { Text($0.state) }.width(min:80,ideal:90)
            TableColumn("Last result",value:\.message) { Text($0.message).lineLimit(1).help($0.message) }.width(min:170,ideal:230)
            TableColumn("Last announce",value:\.lastSort) { Text(timestamp($0.tracker.lastAnnounce)).monospacedDigit() }.width(145)
            TableColumn("Next announce",value:\.nextSort) { Text(timestamp($0.tracker.nextAnnounce)).monospacedDigit() }.width(145)
            TableColumn("Peers returned",value:\.peersSort) { Text(count($0.tracker.peers)).monospacedDigit() }.width(100)
            TableColumn("Seeds reported",value:\.seedsSort) { Text(count($0.tracker.seeds)).monospacedDigit() }.width(110)
            TableColumn("Leechers reported",value:\.leechersSort) { Text(count($0.tracker.leechers)).monospacedDigit() }.width(125)
            TableColumn("Wire hash",value:\.hash) { Text($0.hash).font(.system(size:10,design:.monospaced)).lineLimit(1).truncationMode(.middle).textSelection(.enabled).help("Announced wire identity: \($0.hash)") }.width(min:100,ideal:140)
        }.tableStyle(.inset(alternatesRowBackgrounds:true)).accessibilityLabel("Tracker observations for \(torrent.name)")
    }

    private var activity: some View {
        let samples = store.history[torrent.id] ?? []
        let events = store.events[torrent.id] ?? []
        return VStack(spacing:0) {
            HStack {
                Text("Traffic · last 15 minutes in this app session").foregroundStyle(.secondary)
                Spacer()
                Button("Copy Diagnostics") { store.copyDiagnostics(torrent) }
                    .help("Copy diagnostics with tracker credentials, peer endpoints, and personal paths removed")
            }.padding(.horizontal,12).padding(.vertical,7)
            if samples.isEmpty {
                Text("Traffic history appears as live snapshots arrive.").foregroundStyle(.secondary).frame(maxWidth:.infinity).frame(height:90)
            } else {
                Chart {
                    ForEach(samples) { sample in
                        LineMark(x:.value("Time",sample.id),y:.value("Bytes per second",Double(sample.download)),series:.value("Direction","Download"))
                            .foregroundStyle(by:.value("Direction","Download"))
                        LineMark(x:.value("Time",sample.id),y:.value("Bytes per second",Double(sample.upload)),series:.value("Direction","Upload"))
                            .foregroundStyle(by:.value("Direction","Upload"))
                    }
                }
                .chartForegroundStyleScale(["Download":Design.blue,"Upload":Design.green])
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
                .accessibilityLabel("Download and upload traffic history for \(torrent.name)")
            }
            Divider()
            if events.isEmpty { empty("No recorded events",detail:"Transfer changes, tracker failures, recovery, and file operations appear here.") }
            else {
                List(events.reversed()) { event in
                    HStack(alignment:.top,spacing:12) {
                        Text(event.date,format:.dateTime.hour().minute().second()).monospacedDigit().foregroundStyle(.secondary).frame(width:75,alignment:.leading)
                        Text(event.message).textSelection(.enabled).frame(maxWidth:.infinity,alignment:.leading)
                    }.accessibilityElement(children:.combine)
                }.listStyle(.plain).accessibilityLabel("Recent transfer events")
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
            Button("Copy") { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(hash,forType:.string) }.accessibilityLabel("Copy \(label)")
        }
    }
    private func discovery(_ label: String, _ enabled: Bool?) -> some View {
        Text("\(label): \(enabled.map { $0 ? "Enabled" : "Disabled" } ?? "Unknown")").foregroundStyle(.secondary)
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
        if seconds >= 86400 { return "\(seconds / 86400)d \((seconds % 86400) / 3600)h" }
        if seconds >= 3600 { return "\(seconds / 3600)h \((seconds % 3600) / 60)m" }
        return "\(seconds / 60)m \(seconds % 60)s"
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
        case "live": label = "Connected"
        case "connecting": label = "Connecting"
        case "queued": label = "Queued"
        case "dead": label = "Disconnected"
        case "not_needed", "not needed": label = "Not needed"
        default: label = "Unknown"
        }
        return label + (peer.incoming.map { $0 ? " · incoming" : " · outgoing" } ?? "")
    }
    var transport: String { peer.transport.map { $0.lowercased() == "utp" ? "uTP" : $0.uppercased() } ?? "—" }
    var encryption: String { peer.encrypted.map { $0 ? "Encrypted" : "Plaintext" } ?? "Unknown" }
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
    var state: String { tracker.state.capitalized }
    var message: String { tracker.message ?? "—" }
    var hash: String { tracker.hash }
    var lastSort: UInt64 { tracker.lastAnnounce ?? 0 }
    var nextSort: UInt64 { tracker.nextAnnounce ?? 0 }
    var peersSort: UInt64 { tracker.peers ?? 0 }
    var seedsSort: UInt64 { tracker.seeds ?? 0 }
    var leechersSort: UInt64 { tracker.leechers ?? 0 }
}
