import AppKit
import Foundation

// Vector artwork stays sharp at both Finder display scales.
let output = URL(fileURLWithPath: CommandLine.arguments[1])
try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
let width: CGFloat = 720
let height: CGFloat = 460
func rect(_ x: CGFloat, _ y: CGFloat, _ w: CGFloat, _ h: CGFloat) -> NSRect {
    NSRect(x: x, y: height - y - h, width: w, height: h)
}
func color(_ r: CGFloat, _ g: CGFloat, _ b: CGFloat) -> NSColor {
    NSColor(srgbRed: r, green: g, blue: b, alpha: 1)
}
let ink = color(0.10, 0.20, 0.17)
let secondary = color(0.32, 0.41, 0.37)
let mint = color(0.19, 0.49, 0.36)
func text(_ value: String, _ x: CGFloat, _ y: CGFloat, _ w: CGFloat,
          _ size: CGFloat, _ weight: NSFont.Weight, _ foreground: NSColor) {
    (value as NSString).draw(in: rect(x, y, w, size * 1.6), withAttributes: [
        .font: NSFont.systemFont(ofSize: size, weight: weight), .foregroundColor: foreground
    ])
}
func render(scale: Int, preview: Bool = false) throws {
    let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: Int(width) * scale,
        pixelsHigh: Int(height) * scale, bitsPerSample: 8, samplesPerPixel: 4,
        hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
        bytesPerRow: 0, bitsPerPixel: 0)!
    bitmap.size = NSSize(width: width, height: height)
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: bitmap)
    NSGradient(starting: color(0.93, 0.98, 0.95), ending: color(0.99, 0.99, 0.97))!
        .draw(in: NSRect(x: 0, y: 0, width: width, height: height), angle: -60)
    text("S E E D H O P", 48, 34, 500, 12, .semibold, mint)
    text("Welcome to SeedHop", 46, 63, 640, 34, .semibold, ink)
    text("Drag SeedHop into Applications to install.", 48, 115, 640, 16, .regular, secondary)

    let panel = NSBezierPath(roundedRect: rect(36, 173, 648, 196), xRadius: 24, yRadius: 24)
    NSColor.white.withAlphaComponent(0.78).setFill(); panel.fill()
    color(0.83, 0.90, 0.85).setStroke(); panel.lineWidth = 1; panel.stroke()
    let arrow = NSBezierPath()
    arrow.move(to: NSPoint(x: 323, y: height - 256))
    arrow.line(to: NSPoint(x: 397, y: height - 256))
    arrow.move(to: NSPoint(x: 385, y: height - 244))
    arrow.line(to: NSPoint(x: 397, y: height - 256))
    arrow.line(to: NSPoint(x: 385, y: height - 268))
    mint.setStroke(); arrow.lineWidth = 2.5; arrow.lineCapStyle = .round; arrow.lineJoinStyle = .round; arrow.stroke()
    text("Then open SeedHop from Applications.", 48, 402, 500, 13, .regular, secondary)
    text("macOS 15+ · Apple Silicon", 48, 425, 500, 11, .regular, secondary)

    if preview {
        // Preview only: Finder supplies the real, draggable icons in the disk image.
        let app = URL(fileURLWithPath: CommandLine.arguments[2])
        let appIcon = NSImage(contentsOf: app.appendingPathComponent("Contents/Resources/AppIcon.icns"))!
        appIcon.draw(in: rect(148, 204, 104, 104))
        NSWorkspace.shared.icon(forFile: "/Applications").draw(in: rect(468, 204, 104, 104))
        for (label, x) in [("SeedHop", CGFloat(200)), ("Applications", CGFloat(520))] {
            let attributes: [NSAttributedString.Key: Any] = [
                .font: NSFont.systemFont(ofSize: 14), .foregroundColor: ink
            ]
            let size = (label as NSString).size(withAttributes: attributes)
            (label as NSString).draw(at: NSPoint(x: x - size.width / 2, y: height - 333), withAttributes: attributes)
        }
    }
    NSGraphicsContext.restoreGraphicsState()
    let name = preview ? "preview.png" : (scale == 1 ? "background.png" : "background@2x.png")
    try bitmap.representation(using: .png, properties: [:])!.write(to: output.appendingPathComponent(name))
}
try render(scale: 1)
try render(scale: 2)
if CommandLine.arguments.count > 2 { try render(scale: 2, preview: true) }
