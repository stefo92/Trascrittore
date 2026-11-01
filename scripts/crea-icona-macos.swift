// Disegna l'icona di Trascrittore.app (onda sonora che diventa righe di testo) e la salva come
// .icns. Usato da scripts/build-macos.sh; richiede soltanto gli Xcode Command Line Tools.
//
// Uso: swift scripts/crea-icona-macos.swift <file.icns>
import AppKit

let output = URL(fileURLWithPath: CommandLine.arguments[1])
let iconset = FileManager.default.temporaryDirectory
    .appendingPathComponent("Trascrittore-\(ProcessInfo.processInfo.processIdentifier).iconset")
try? FileManager.default.removeItem(at: iconset)
try! FileManager.default.createDirectory(at: iconset, withIntermediateDirectories: true)

func render(_ pixels: Int) -> Data {
    let rep = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: pixels, pixelsHigh: pixels, bitsPerSample: 8,
        samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
        bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    let s = CGFloat(pixels)

    // Riquadro arrotondato secondo la griglia delle icone di macOS (824 su 1024).
    let body = NSRect(x: s * 0.0977, y: s * 0.0977, width: s * 0.8046, height: s * 0.8046)
    let shape = NSBezierPath(roundedRect: body, xRadius: s * 0.18, yRadius: s * 0.18)
    NSGradient(
        starting: NSColor(calibratedRed: 0.16, green: 0.45, blue: 0.90, alpha: 1),
        ending: NSColor(calibratedRed: 0.35, green: 0.22, blue: 0.72, alpha: 1))!
        .draw(in: shape, angle: -90)

    if let symbol = NSImage(systemSymbolName: "waveform", accessibilityDescription: nil) {
        let config = NSImage.SymbolConfiguration(pointSize: s * 0.36, weight: .semibold)
            .applying(NSImage.SymbolConfiguration(paletteColors: [.white]))
        if let image = symbol.withSymbolConfiguration(config) {
            let size = image.size
            image.draw(in: NSRect(
                x: (s - size.width) / 2, y: s * 0.47, width: size.width, height: size.height))
        }
    }
    // Righe di testo sotto l'onda.
    NSColor.white.withAlphaComponent(0.9).setFill()
    for (index, width) in [0.50, 0.38].enumerated() {
        let w = s * CGFloat(width)
        let h = s * 0.045
        let y = s * 0.33 - CGFloat(index) * s * 0.09
        NSBezierPath(roundedRect: NSRect(x: (s - w) / 2, y: y, width: w, height: h),
                     xRadius: h / 2, yRadius: h / 2).fill()
    }
    NSGraphicsContext.restoreGraphicsState()
    return rep.representation(using: .png, properties: [:])!
}

for base in [16, 32, 128, 256, 512] {
    for scale in [1, 2] {
        let name = scale == 1 ? "icon_\(base)x\(base).png" : "icon_\(base)x\(base)@2x.png"
        try! render(base * scale).write(to: iconset.appendingPathComponent(name))
    }
}

let iconutil = Process()
iconutil.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
iconutil.arguments = ["-c", "icns", "-o", output.path, iconset.path]
try! iconutil.run()
iconutil.waitUntilExit()
try? FileManager.default.removeItem(at: iconset)
exit(iconutil.terminationStatus)
