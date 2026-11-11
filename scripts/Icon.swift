import AppKit
import Foundation

// Render the standard macOS icon sizes and store their PNG data in an ICNS file.
guard CommandLine.arguments.count == 3 else {
    fatalError("Usage: swift scripts/Icon.swift <output.icns> <source.png>")
}
let output = URL(fileURLWithPath: CommandLine.arguments[1])
let source = URL(fileURLWithPath: CommandLine.arguments[2])
guard let image = NSImage(contentsOf: source) else {
    fatalError("Cannot load app icon: \(source.path)")
}
func appendChunk(_ type: String, payload: Data, to data: inout Data) {
    data.append(contentsOf: type.utf8)
    var length = UInt32(payload.count + 8).bigEndian
    withUnsafeBytes(of: &length) { data.append(contentsOf: $0) }
    data.append(payload)
}

var representations = Data()
for (type, size) in [
    ("icp4", 16), ("ic11", 32),
    ("icp5", 32), ("ic12", 64),
    ("ic07", 128), ("ic13", 256),
    ("ic08", 256), ("ic14", 512),
    ("ic09", 512), ("ic10", 1024),
] {
    let bitmap = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size,
        bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true,
        isPlanar: false, colorSpaceName: .deviceRGB,
        bytesPerRow: 0, bitsPerPixel: 0
    )!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: bitmap)
    NSGraphicsContext.current!.imageInterpolation = .high
    image.draw(in: NSRect(x: 0, y: 0, width: size, height: size))
    NSGraphicsContext.restoreGraphicsState()
    guard let png = bitmap.representation(using: .png, properties: [:]) else {
        fatalError("Cannot encode the \(size)-pixel app icon")
    }
    appendChunk(type, payload: png, to: &representations)
}

var icon = Data()
appendChunk("icns", payload: representations, to: &icon)
try icon.write(to: output, options: .atomic)
