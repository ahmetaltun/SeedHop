import SwiftUI
import AppKit

enum Design {
    static let green = Color(nsColor: NSColor(name:nil) { $0.bestMatch(from:[.darkAqua,.aqua]) == .darkAqua ? NSColor(red:0.39,green:0.82,blue:0.65,alpha:1) : NSColor(red:0.10,green:0.43,blue:0.32,alpha:1) })
    static let canvas = Color(nsColor:NSColor(name:nil) { $0.bestMatch(from:[.darkAqua,.aqua]) == .darkAqua ? NSColor(red:0.105,green:0.12,blue:0.115,alpha:1) : NSColor(red:0.985,green:0.985,blue:0.975,alpha:1) })
    static let sidebar = Color(nsColor:NSColor(name:nil) { $0.bestMatch(from:[.darkAqua,.aqua]) == .darkAqua ? NSColor(red:0.08,green:0.095,blue:0.09,alpha:1) : NSColor(red:0.945,green:0.956,blue:0.942,alpha:1) })
    static let line = Color(nsColor:NSColor(name:nil) { appearance in
        let highContrast = appearance.bestMatch(from:[.aqua,.darkAqua,.accessibilityHighContrastAqua,.accessibilityHighContrastDarkAqua])
        if highContrast == .accessibilityHighContrastAqua || highContrast == .accessibilityHighContrastDarkAqua { return .separatorColor }
        return NSColor.labelColor.withAlphaComponent(0.075)
    })
    static let secondary = Color(nsColor:.secondaryLabelColor)
    static let card = Color(nsColor:.controlBackgroundColor)
    static let blue = Color(nsColor:NSColor(name:nil) { $0.bestMatch(from:[.darkAqua,.aqua]) == .darkAqua ? NSColor(red:0.48,green:0.72,blue:1,alpha:1) : NSColor(red:0.12,green:0.36,blue:0.66,alpha:1) })
    static let amber = Color(nsColor:NSColor(name:nil) { $0.bestMatch(from:[.darkAqua,.aqua]) == .darkAqua ? NSColor(red:1,green:0.73,blue:0.38,alpha:1) : NSColor(red:0.60,green:0.31,blue:0.06,alpha:1) })
    static let radius: CGFloat = 12
    static func stateColor(_ state: String) -> Color {
        switch state { case "seeding": return green; case "downloading": return blue; case "error": return amber; default: return secondary }
    }
    static func symbol(_ name: String) -> String {
        let ext = URL(fileURLWithPath:name).pathExtension.lowercased()
        if ["mp4","mkv","mov","webm"].contains(ext) { return "film" }
        if ["zip","gz","xz","dmg","iso"].contains(ext) { return "shippingbox" }
        if ["flac","mp3","wav","aiff"].contains(ext) { return "waveform" }
        if ["pdf","epub"].contains(ext) { return "book.closed" }
        return "folder"
    }
}

struct BrandMark: View {
    var size: CGFloat = 34
    var body: some View {
        Image(systemName:"leaf.fill").font(.system(size:size * 0.53,weight:.medium))
            .foregroundStyle(Design.green).frame(width:size,height:size)
            .background(Design.green.opacity(0.11),in:RoundedRectangle(cornerRadius:size * 0.29))
            .accessibilityHidden(true)
    }
}
struct StateBadge: View {
    let torrent: TorrentItem
    var body: some View {
        HStack(spacing:5) { Circle().fill(Design.stateColor(torrent.state)).frame(width:5,height:5); Text(torrent.statusLabel).font(.system(size:11,weight:.medium)) }
            .foregroundStyle(Design.stateColor(torrent.state)).padding(.horizontal,8).padding(.vertical,5)
            .background(Design.stateColor(torrent.state).opacity(0.075),in:Capsule())
            .accessibilityElement(children:.ignore)
            .accessibilityLabel(torrent.statusLabel)
    }
}
struct SectionEyebrow: View {
    let title: String
    var body: some View { Text(title.uppercased()).font(.system(size:10,weight:.semibold)).tracking(1.4).foregroundStyle(.secondary) }
}
struct TransferProgress: View {
    let value: Double
    var color: Color = Design.green
    var body: some View {
        GeometryReader { proxy in
            ZStack(alignment:.leading) {
                Capsule().fill(color.opacity(0.10))
                Capsule().fill(color.opacity(0.8)).frame(width:proxy.size.width * min(1,max(0,value)))
            }
        }.frame(height:4).accessibilityElement(children:.ignore).accessibilityLabel("Progress").accessibilityValue("\(Int(value * 100)) percent")
    }
}
struct Metric: View {
    let title: String
    let value: String
    let icon: String
    var body: some View {
        VStack(alignment:.leading,spacing:9) {
            Label(title,systemImage:icon).font(.system(size:11)).foregroundStyle(.secondary)
            Text(value).font(.system(size:21,weight:.medium,design:.rounded)).monospacedDigit()
        }
        .frame(maxWidth:.infinity,alignment:.leading)
    }
}
struct InlineNotice: View {
    let title: String
    let message: String
    var retry: (() -> Void)?
    var dismiss: (() -> Void)?
    var body: some View {
        HStack(alignment:.top,spacing:12) {
            Image(systemName:"exclamationmark.circle.fill").foregroundStyle(.orange)
            VStack(alignment:.leading,spacing:4) { Text(title).font(.system(size:12,weight:.semibold)); Text(message).font(.system(size:12)).foregroundStyle(.secondary).textSelection(.enabled) }
            Spacer(minLength:4)
            if let retry { Button("Retry",action:retry).controlSize(.small) }
            if let dismiss { Button(action:dismiss) { Image(systemName:"xmark") }.buttonStyle(.plain).accessibilityLabel("Dismiss message") }
        }.padding(14).background(Color.orange.opacity(0.07),in:RoundedRectangle(cornerRadius:10))
    }
}
