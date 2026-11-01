import SwiftUI

/// A bounded canvas keeps large torrents from creating thousands of SwiftUI views.
@MainActor struct TorrentPieceMapView: View {
    let map: PieceMap?
    let live: Bool
    @StateObject private var hoverDelay: PieceMapHoverDelay
    private var hover: PieceMapHover? { hoverDelay.hover }
    @Environment(\.colorSchemeContrast) private var contrast

    init(map: PieceMap?, live: Bool = true, hover: PieceMapHover? = nil) {
        self.map = map; self.live = live
        _hoverDelay = StateObject(wrappedValue:PieceMapHoverDelay(initialHover:hover))
    }

    private var progressTitle: String { String(localized:"Progress",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
    private var availabilityTitle: String { String(localized:"Availability",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
    private var progressHelp: String { String(localized:"Full torrent piece map, including unselected files. Green: verified. Blue: downloading. Gray: missing. Pieces run from left to right.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
    private var availabilityHelp: String { String(localized:"Copies across connected peers, excluding your files. Darker blue means more copies; gray means no peer has a piece. Paused transfers have no live availability.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
    private var gapHelp: String { String(localized:"Amber hatching marks ranges containing pieces missing locally with no connected source. Hover over a range for details.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
    private var progressValue: String {
        guard let map, map.totalPieces > 0 else { return "—" }
        return (Double(map.verifiedPieces) / Double(map.totalPieces)).formatted(.percent.precision(.fractionLength(1)).locale(AppLocalization.shared.locale))
    }
    private var availabilityValue: String {
        guard let value = map?.availability else { return "—" }
        return value.formatted(.number.precision(.fractionLength(3)).locale(AppLocalization.shared.locale)) + "×"
    }

    var body: some View {
        VStack(alignment:.leading,spacing:10) {
            Grid(alignment:.leading,horizontalSpacing:10,verticalSpacing:10) {
                row(progressTitle,value:progressValue,availability:false,help:progressHelp)
                row(availabilityTitle,value:availabilityValue,availability:true,help:availabilityHelp)
            }
            .zIndex(1)
            ViewThatFits(in:.horizontal) {
                HStack(spacing:12) { legend }
                VStack(alignment:.leading,spacing:4) { legend }
            }
            .font(.system(size:10)).foregroundStyle(.secondary)
            .accessibilityHidden(true)
        }
        .onDisappear { hoverDelay.cancel() }
        .onChange(of:map?.totalPieces) { _, _ in hoverDelay.cancel() }
    }

    @ViewBuilder private var legend: some View {
        key(Design.green,String(localized:"Verified",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
        key(Design.blue,String(localized:"Downloading",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
        key(Color(nsColor:.tertiaryLabelColor),String(localized:"Missing",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
        if map?.bins.contains(where:{ gapCount($0) > 0 }) == true {
            HStack(spacing:4) {
                Image(systemName:"exclamationmark.triangle.fill").foregroundStyle(Design.amber)
                Text(String(localized:"No source",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
            }.help(gapHelp)
        }
    }

    private func key(_ color: Color, _ title: String) -> some View {
        HStack(spacing:4) {
            RoundedRectangle(cornerRadius:2).fill(color).frame(width:7,height:7)
            Text(title)
        }
    }

    private func row(_ title: String, value: String, availability: Bool, help: String) -> some View {
        GridRow {
            Text(title).fontWeight(.medium).fixedSize().help(help + " " + gapHelp)
            bar(availability:availability)
            Text(value).monospacedDigit().foregroundStyle(.secondary)
                .fixedSize().gridColumnAlignment(.trailing)
        }
        .accessibilityElement(children:.ignore)
        .accessibilityLabel(title)
        .accessibilityValue(accessibleValue(value))
        .accessibilityHint(help + " " + gapHelp)
        .zIndex(hover?.availability == availability ? 1 : 0)
    }

    private func accessibleValue(_ value: String) -> String {
        let gaps = map?.bins.reduce(UInt64(0)) { $0 + UInt64(gapCount($1)) } ?? 0
        guard gaps > 0 else { return value }
        return value + ". " + String(localized:"No connected source",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) + ": " + LocaleText.integer(gaps)
    }

    private func gapCount(_ bin: PieceMapBin) -> UInt32 {
        live && map?.availability != nil ? bin.unavailablePieces ?? 0 : 0
    }

    private func bar(availability: Bool) -> some View {
        GeometryReader { geometry in
            Canvas { context, size in
                guard let map, !map.bins.isEmpty, !availability || map.availability != nil else { return }
                var offset = 0.0
                for bin in map.bins {
                    let x = offset / Double(max(1,map.totalPieces)) * size.width
                    offset += Double(bin.pieces)
                    let width = offset / Double(max(1,map.totalPieces)) * size.width - x
                    let rect = CGRect(x:x,y:0,width:width + 0.25,height:size.height)
                    if availability {
                        if bin.minimumAvailability > 0 {
                            let strength = min(1,0.35 + log2(Double(bin.minimumAvailability) + 1) * 0.15)
                            context.fill(Path(rect),with:.color(Design.blue.opacity(strength)))
                        }
                    } else if bin.pieces > 0 {
                        let verifiedHeight = size.height * Double(bin.verified) / Double(bin.pieces)
                        let activeHeight = size.height * Double(bin.downloading) / Double(bin.pieces)
                        context.fill(Path(CGRect(x:x,y:size.height - verifiedHeight,width:width + 0.25,height:verifiedHeight)),with:.color(Design.green))
                        context.fill(Path(CGRect(x:x,y:size.height - verifiedHeight - activeHeight,width:width + 0.25,height:activeHeight)),with:.color(Design.blue))
                    }
                    if gapCount(bin) > 0 {
                        var hatch = context
                        hatch.clip(to:Path(rect))
                        hatch.fill(Path(rect),with:.color(Design.amber.opacity(0.3)))
                        var lines = Path()
                        for stripe in stride(from:x - size.height,through:x + width,by:6) {
                            lines.move(to:CGPoint(x:stripe,y:size.height))
                            lines.addLine(to:CGPoint(x:stripe + size.height,y:0))
                        }
                        hatch.stroke(lines,with:.color(Design.amber),lineWidth:1.5)
                        hatch.stroke(Path(rect),with:.color(Design.amber),lineWidth:1)
                    }
                }
            }
            .background(Color(nsColor:.quaternaryLabelColor).opacity(contrast == .increased ? 1 : 0.5))
            .clipShape(RoundedRectangle(cornerRadius:3))
            .overlay(RoundedRectangle(cornerRadius:3).strokeBorder(Color(nsColor:.separatorColor),lineWidth:0.5))
            .contentShape(Rectangle())
            .onContinuousHover { phase in
                switch phase {
                case .active(let location):
                    let region = geometry.size.width > 0 ? map?.region(at:location.x / geometry.size.width) : nil
                    let next = region.map { PieceMapHover(index:$0.index,availability:availability) }
                    hoverDelay.update(next,location:location)
                case .ended:
                    hoverDelay.cancel(availability:availability)
                }
            }
            .overlay(alignment:.topLeading) {
                if let hover, hover.availability == availability, let map, let region = map.region(index:hover.index) {
                    let cardWidth = min(260,geometry.size.width)
                    let center = Double(region.firstPiece + region.lastPiece + 1) / 2 / Double(max(1,map.totalPieces)) * geometry.size.width
                    PieceMapHoverCard(region:region,availabilityKnown:live && map.availability != nil)
                        .frame(width:cardWidth)
                        .offset(x:max(0,min(geometry.size.width - cardWidth,center - cardWidth / 2)),y:22)
                        .allowsHitTesting(false)
                }
            }
        }
        .frame(minWidth:40,maxWidth:.infinity).frame(height:14)
        .accessibilityHidden(true)
    }
}

struct PieceMapHover: Equatable {
    var index: Int
    var availability: Bool
}

/// Show details only after the pointer rests; moving or leaving cancels the reveal.
@MainActor final class PieceMapHoverDelay: ObservableObject {
    @Published private(set) var hover: PieceMapHover?
    private let delay: Duration
    private var task: Task<Void,Never>?
    private var candidate: PieceMapHover?
    private var location: CGPoint?

    init(initialHover: PieceMapHover? = nil, delay: Duration = .milliseconds(150)) {
        hover = initialHover; self.delay = delay
    }

    func update(_ target: PieceMapHover?, location: CGPoint) {
        guard candidate != target || self.location != location else { return }
        cancel()
        candidate = target; self.location = location
        guard let target else { return }
        task = Task { [weak self, delay] in
            do { try await Task.sleep(for:delay) } catch { return }
            guard !Task.isCancelled else { return }
            self?.hover = target
        }
    }

    func cancel(availability: Bool? = nil) {
        if let availability, candidate?.availability != availability && hover?.availability != availability { return }
        task?.cancel(); task = nil
        candidate = nil; location = nil; hover = nil
    }

    deinit { task?.cancel() }
}

private struct PieceMapHoverCard: View {
    let region: PieceMapRegion
    let availabilityKnown: Bool

    var body: some View {
        VStack(alignment:.leading,spacing:7) {
            Text(String(localized:"Pieces \(LocaleText.integer(region.firstPiece + 1))–\(LocaleText.integer(region.lastPiece + 1))",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                .fontWeight(.semibold)
            metric(String(localized:"Verified",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),
                   (Double(region.bin.verified) / Double(max(1,region.bin.pieces))).formatted(.percent.precision(.fractionLength(1)).locale(AppLocalization.shared.locale)))
            metric(String(localized:"Downloading",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),LocaleText.integer(region.bin.downloading))
            metric(String(localized:"Minimum copies",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),availabilityKnown ? LocaleText.integer(region.bin.minimumAvailability) + "×" : "—")
            HStack(spacing:5) {
                if availabilityKnown, (region.bin.unavailablePieces ?? 0) > 0 {
                    Image(systemName:"exclamationmark.triangle.fill").foregroundStyle(Design.amber)
                }
                metric(String(localized:"No connected source",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),availabilityKnown ? region.bin.unavailablePieces.map { LocaleText.integer($0) } ?? "—" : "—")
            }
        }
        .font(.system(size:11)).padding(10)
        .background(.regularMaterial,in:RoundedRectangle(cornerRadius:7))
        .overlay(RoundedRectangle(cornerRadius:7).strokeBorder(Color(nsColor:.separatorColor),lineWidth:0.5))
        .shadow(color:.black.opacity(0.15),radius:5,y:2)
    }

    private func metric(_ title: String, _ value: String) -> some View {
        HStack(alignment:.firstTextBaseline,spacing:10) {
            Text(title).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true)
            Spacer(minLength:0)
            Text(value).monospacedDigit().fixedSize()
        }
    }
}
