# SeedHop app icon

Source artwork: [`macOS/Resources/AppIcon.png`](../macOS/Resources/AppIcon.png).
Generated with the built-in image generation tool on 2026-10-24, with a transparent background.

`make build` uses `scripts/Icon.swift` to render and package the ten standard
macOS PNG representations into
`.build/SeedHop.app/Contents/Resources/AppIcon.icns`. No image generation service
is needed to build the app.

## Generation prompt

```text
Use case: logo-brand
Asset type: production macOS app icon for SeedHop, a native torrent client.
Primary request: Create one polished app icon using the existing SeedHop identity: a pale mint seed/leaf with a curved vein and two small trailing seed dots, suggesting a gentle hop and peer transfer, on a deep forest green rounded square.
Style/medium: refined macOS icon, a softly sculpted mint leaf with restrained dimensional shading, subtle surface depth, crisp recognizable silhouette. Quiet, warm, precise. No glass effects.
Composition/framing: square 1024 by 1024 canvas, single centered icon. Rounded square occupies 94 percent of canvas with transparent corners outside it. Leaf fills about 55 percent of icon; large simple shapes legible at 16 pixels. Green tile softly lit from upper left, generous padding.
Color palette: deep forest green background, warm pale mint leaf, green curved vein.
Constraints: no text, no letters, no numbers, no watermark, no mockup, no extra icons, no border around canvas. Actual transparent exterior; opaque green icon tile.
```
