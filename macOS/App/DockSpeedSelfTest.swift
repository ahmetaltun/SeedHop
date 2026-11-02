#if SEEDHOP_DEVELOPMENT
import Foundation
import AppKit

@MainActor enum DockSpeedSelfTest {
    static func run() throws {
        func expect(_ condition: Bool, _ message: String) throws {
            if !condition { throw UserFacingError(message:message) }
        }
        for (rate, expected) in [(UInt64(0),"—"),(999,"999B"),(1_000,"1K"),(678_000,"678K"),(999_950,"1M"),(10_400_000,"10.4M"),(UInt64.max,"18.4E")] {
            try expect(DockSpeedPresentation.compactRate(rate,locale:Locale(identifier:"en_US")) == expected,"Compact Dock rate lost units or rounding: \(rate)")
        }
        try expect(DockSpeedPresentation.compactRate(10_400_000,locale:Locale(identifier:"de_DE")) == "10,4M","Compact Dock rate ignored regional decimal separator")
        let accessible = DockSpeedPresentation.make(enabled:true,connected:true,download:10_400_000,upload:678_000)!
        try expect(accessible.accessibilityLabel.contains(SizeText.rate(10_400_000)) && accessible.accessibilityLabel.contains(SizeText.rate(678_000)),"Compact labels removed full spoken units")
        var renders: [DockSpeedPresentation?] = []
        let controller = DockSpeedController(render:{ renders.append($0) })
        controller.update(enabled:false,connected:true,download:1_000,upload:2_000)
        try expect(renders.isEmpty,"Disabled Dock speeds rendered")
        controller.update(enabled:true,connected:true,download:0,upload:0)
        try expect(renders.isEmpty,"Idle Dock speeds rendered")
        controller.update(enabled:true,connected:true,download:1_000,upload:2_000)
        try expect(renders.count == 1 && controller.presentation?.download == "↓\(DockSpeedPresentation.compactRate(1_000))" && controller.presentation?.upload == "↑\(DockSpeedPresentation.compactRate(2_000))","Dock rates lost direction or units")
        controller.update(enabled:true,connected:true,download:1_000,upload:2_000)
        try expect(renders.count == 1,"Unchanged Dock rates redrew")
        controller.update(enabled:true,connected:true,download:0,upload:2_000)
        try expect(renders.count == 2 && controller.presentation?.upload == "↑\(DockSpeedPresentation.compactRate(2_000))","Upload-only activity disappeared")
        controller.update(enabled:true,connected:false,download:1_000,upload:2_000)
        try expect(renders.count == 3 && controller.presentation == nil,"Disconnected Dock showed stale rates")
        controller.update(enabled:true,connected:false,download:1_000,upload:2_000)
        try expect(renders.count == 3,"Disconnected Dock repeatedly redrew")
        controller.update(enabled:true,connected:true,download:UInt64.max,upload:1)
        try expect(controller.presentation?.download == "↓\(DockSpeedPresentation.compactRate(UInt64.max))","Extreme Dock rate formatting changed")
        try expect(controller.presentation?.accessibilityLabel.isEmpty == false,"Dock rates lack accessibility text")
        controller.reset()
        try expect(renders.count == 5 && controller.presentation == nil,"Dock reset did not restore icon")
        controller.reset()
        try expect(renders.count == 5,"Dock reset redrew unnecessarily")
        controller.update(enabled:true,connected:true,download:1_000,upload:0)
        controller.update(enabled:false,connected:true,download:1_000,upload:0)
        try expect(renders.count == 7 && controller.presentation == nil,"Disabling Dock speeds did not clear tile")
        let args = ProcessInfo.processInfo.arguments
        if args.contains("--exercise-dock-tile") {
            let native = DockSpeedController()
            native.update(enabled:true,connected:true,download:10_400_000,upload:0)
            guard let content = NSApp.dockTile.contentView as? DockSpeedView,
                  content.bounds.width > 0, content.bounds.height > 0,
                  content.presentation?.download == "↓\(DockSpeedPresentation.compactRate(10_400_000))" else {
                throw UserFacingError(message:"Enabled speeds did not reach the native Dock tile")
            }
            native.update(enabled:false,connected:true,download:10_400_000,upload:0)
            try expect(NSApp.dockTile.contentView == nil,"Disabling speeds did not restore native Dock tile")
            print("PASS: Enabled speeds reach the real Dock tile; disabling restores the app icon")
        }
        if let index = args.firstIndex(of:"--output"), args.indices.contains(index + 1) {
            let directory = URL(fileURLWithPath:args[index + 1],isDirectory:true)
            try FileManager.default.createDirectory(at:directory,withIntermediateDirectories:true)
            for size in [32,64,128] {
                for (name,download,upload) in [("normal",UInt64(12_345_000),UInt64(678_000)),("large",UInt64.max,UInt64.max)] {
                    let view = DockSpeedView(frame:NSRect(x:0,y:0,width:size,height:size))
                    view.icon = NSApp.applicationIconImage
                    view.presentation = DockSpeedPresentation.make(enabled:true,connected:true,download:download,upload:upload)
                    let bitmap = NSBitmapImageRep(bitmapDataPlanes:nil,pixelsWide:size * 2,pixelsHigh:size * 2,bitsPerSample:8,samplesPerPixel:4,hasAlpha:true,isPlanar:false,colorSpaceName:.deviceRGB,bytesPerRow:0,bitsPerPixel:0)!
                    bitmap.size = NSSize(width:size,height:size)
                    NSGraphicsContext.saveGraphicsState()
                    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep:bitmap)
                    view.draw(view.bounds)
                    NSGraphicsContext.restoreGraphicsState()
                    try bitmap.representation(using:.png,properties:[:])!.write(to:directory.appendingPathComponent("dock-\(name)-\(size).png"))
                }
            }
        }
        print("PASS: Dock speed presentation, directions, idle, disconnect, boundaries, redraws, and reset")
    }
}
#endif
