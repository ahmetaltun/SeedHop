import AppKit

/// A presentation value keeps Dock rendering independent from the transfer engine.
struct DockSpeedPresentation: Equatable {
    let download: String
    let upload: String
    let accessibilityLabel: String

    static func make(enabled: Bool, connected: Bool, download: UInt64, upload: UInt64) -> Self? {
        guard enabled, connected, download > 0 || upload > 0 else { return nil }
        let down = SizeText.rate(download), up = SizeText.rate(upload)
        return Self(download:"↓\(compactRate(download))", upload:"↑\(compactRate(upload))",
                    accessibilityLabel:String(localized:"Download: \(down). Upload: \(up).",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
    }

    /// Decimal byte units match the library; full units remain in the accessibility label.
    static func compactRate(_ value: UInt64, locale: Locale = .current) -> String {
        guard value > 0 else { return "—" }
        let units = ["B", "K", "M", "G", "T", "P", "E"]
        var amount = Double(value), unit = 0
        while amount >= 1000 && unit < units.count - 1 { amount /= 1000; unit += 1 }
        amount = (amount * 10).rounded() / 10
        if amount >= 1000 && unit < units.count - 1 { amount /= 1000; unit += 1 }
        return amount.formatted(.number.locale(locale).grouping(.never).precision(.fractionLength(0...1))) + units[unit]
    }

}

/// The application owns one controller. Tests inject a renderer and never touch the Dock.
@MainActor final class DockSpeedController {
    private let render: (DockSpeedPresentation?) -> Void
    private(set) var presentation: DockSpeedPresentation?

    init(render: ((DockSpeedPresentation?) -> Void)? = nil) {
        if let render { self.render = render }
        else {
            let renderer = DockSpeedRenderer()
            self.render = { renderer.render($0) }
        }
    }

    func update(enabled: Bool, connected: Bool, download: UInt64, upload: UInt64) {
        let next = DockSpeedPresentation.make(enabled:enabled,connected:connected,download:download,upload:upload)
        guard presentation != next else { return }
        presentation = next
        render(next)
    }

    func reset() {
        guard presentation != nil else { return }
        presentation = nil
        render(nil)
    }
}

@MainActor private final class DockSpeedRenderer {
    private var view: DockSpeedView?

    func render(_ presentation: DockSpeedPresentation?) {
        let tile = NSApp.dockTile
        guard let presentation else {
            tile.contentView = nil
            view = nil
            tile.display()
            return
        }
        let content = view ?? DockSpeedView(frame:NSRect(origin:.zero,size:tile.size))
        content.autoresizingMask = [.width,.height]
        content.icon = NSApp.applicationIconImage
        content.presentation = presentation
        content.setAccessibilityElement(true)
        content.setAccessibilityRole(.image)
        content.setAccessibilityLabel(presentation.accessibilityLabel)
        view = content
        tile.contentView = content
        tile.display()
    }
}

/// A translucent backdrop and white text keep speeds readable while preserving the icon.
@MainActor final class DockSpeedView: NSView {
    var icon: NSImage?
    var presentation: DockSpeedPresentation?

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        icon?.draw(in:bounds,from:.zero,operation:.sourceOver,fraction:1)
        guard let presentation else { return }
        let inset = bounds.width * 0.025
        let textArea = NSRect(x:inset,y:bounds.height * 0.025,width:bounds.width - 2 * inset,height:bounds.height * 0.5)
        NSColor.black.withAlphaComponent(0.7).setFill()
        NSBezierPath(roundedRect:textArea,xRadius:bounds.width * 0.06,yRadius:bounds.width * 0.06).fill()
        let lineHeight = textArea.height / 2
        for (index, text) in [presentation.upload,presentation.download].enumerated() {
            let row = NSRect(x:textArea.minX + inset,y:textArea.minY + CGFloat(index) * lineHeight,width:textArea.width - 2 * inset,height:lineHeight)
            // Scale long localized units down to fit without truncating the rate.
            var pointSize = bounds.height * 0.2
            let initialFont = NSFont.monospacedDigitSystemFont(ofSize:pointSize,weight:.bold)
            let width = (text as NSString).size(withAttributes:[.font:initialFont]).width
            if width > row.width { pointSize *= row.width / width }
            let font = NSFont.monospacedDigitSystemFont(ofSize:pointSize,weight:.bold)
            let style = NSMutableParagraphStyle(); style.alignment = .center
            let shadow = NSShadow()
            shadow.shadowColor = NSColor.black.withAlphaComponent(0.9)
            shadow.shadowBlurRadius = bounds.height * 0.012
            shadow.shadowOffset = NSSize(width:0,height:-bounds.height * 0.006)
            let attributes: [NSAttributedString.Key:Any] = [.font:font,.foregroundColor:NSColor.white,.paragraphStyle:style,.shadow:shadow]
            let size = (text as NSString).size(withAttributes:attributes)
            // Point drawing avoids word wrapping caused by fractional glyph widths at small Dock sizes.
            (text as NSString).draw(at:NSPoint(x:row.midX - size.width / 2,y:row.midY - size.height / 2),withAttributes:attributes)
        }
    }
}
