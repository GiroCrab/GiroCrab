// Render the background of the disk image window (scripts/make-dmg.sh): a calm
// light canvas, the install hint and an arrow from the app icon to the
// Applications link. Written as one TIFF holding a 1x and a 2x representation,
// so Finder draws it sharp on Retina displays.
//
// Usage: swift scripts/make-dmg-background.swift <out.tiff> <version>
//
// The window size and icon centres are the ones make-dmg.sh gives Finder;
// change both together.
import AppKit

// The window shows 660x400 pt; the extra height covers taller title bars and
// content areas of other macOS versions with the same canvas.
let size = NSSize(width: 660, height: 440)
let contentHeight: CGFloat = 400
let appCentre = NSPoint(x: 165, y: 196)
let linkCentre = NSPoint(x: 495, y: 196)
let iconHalf: CGFloat = 64  // icon size 128 in make-dmg.sh

// DESIGN.md tokens (canvas, text, tertiary) and the crab orange of the icon.
func rgb(_ hex: UInt32) -> NSColor {
    NSColor(srgbRed: CGFloat(hex >> 16 & 0xFF) / 255, green: CGFloat(hex >> 8 & 0xFF) / 255,
            blue: CGFloat(hex & 0xFF) / 255, alpha: 1)
}
let canvasTop = rgb(0xF8FAFC), canvasBottom = rgb(0xEDF1F6)
let textColor = rgb(0x0F172A), tertiary = rgb(0x64748B), crab = rgb(0xE8603C)

func fail(_ msg: String) -> Never {
    FileHandle.standardError.write(Data("make-dmg-background: \(msg)\n".utf8))
    exit(1)
}

func drawCentred(_ text: String, font: NSFont, color: NSColor, centreY: CGFloat) {
    let str = NSAttributedString(string: text, attributes: [.font: font, .foregroundColor: color])
    let s = str.size()
    str.draw(at: NSPoint(x: (size.width - s.width) / 2, y: centreY - s.height / 2))
}

func rounded(_ font: NSFont) -> NSFont {
    font.fontDescriptor.withDesign(.rounded).flatMap { NSFont(descriptor: $0, size: font.pointSize) } ?? font
}

func drawBackground(version: String) {
    NSGradient(starting: canvasTop, ending: canvasBottom)!
        .draw(in: NSRect(origin: .zero, size: size), angle: 90)

    drawCentred("GiroCrab auf „Programme“ ziehen, um es zu installieren",
                font: rounded(.systemFont(ofSize: 17, weight: .semibold)), color: textColor, centreY: 54)
    drawCentred("Drag GiroCrab onto Applications to install it",
                font: .systemFont(ofSize: 13), color: tertiary, centreY: 80)

    // Arrow between the two icons, with room for Finder's selection highlight.
    let gap: CGFloat = 34
    let start = NSPoint(x: appCentre.x + iconHalf + gap, y: appCentre.y)
    let end = NSPoint(x: linkCentre.x - iconHalf - gap, y: linkCentre.y)
    let shaft = NSBezierPath()
    shaft.move(to: start)
    shaft.line(to: NSPoint(x: end.x - 16, y: end.y))
    shaft.lineWidth = 5
    shaft.lineCapStyle = .round
    shaft.setLineDash([0.01, 13], count: 2, phase: 0)  // round dots
    crab.withAlphaComponent(0.55).setStroke()
    shaft.stroke()

    let head = NSBezierPath()
    head.move(to: NSPoint(x: end.x - 14, y: end.y - 15))
    head.line(to: end)
    head.line(to: NSPoint(x: end.x - 14, y: end.y + 15))
    head.lineWidth = 5
    head.lineCapStyle = .round
    head.lineJoinStyle = .round
    crab.setStroke()
    head.stroke()

    // Above the bottom edge: a path bar and a status bar (if the user shows
    // them) take ~50 pt of the window.
    drawCentred("Version \(version)", font: .systemFont(ofSize: 11), color: tertiary,
                centreY: contentHeight - 58)
}

func render(scale: Int, version: String) -> NSBitmapImageRep {
    guard let rep = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: Int(size.width) * scale, pixelsHigh: Int(size.height) * scale,
        bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
        colorSpaceName: .calibratedRGB, bytesPerRow: 0, bitsPerPixel: 0)?
        .retagging(with: .sRGB),
        let ctx = NSGraphicsContext(bitmapImageRep: rep)
    else { fail("cannot create \(scale)x bitmap") }
    rep.size = size  // points: the 2x rep is the same size at twice the pixels
    // Top-left origin, like Finder's icon positions.
    let flipped = NSGraphicsContext(cgContext: ctx.cgContext, flipped: true)
    flipped.cgContext.scaleBy(x: CGFloat(scale), y: -CGFloat(scale))
    flipped.cgContext.translateBy(x: 0, y: -size.height)
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = flipped
    drawBackground(version: version)
    NSGraphicsContext.restoreGraphicsState()
    return rep
}

let args = Array(CommandLine.arguments.dropFirst())
guard args.count == 2 else {
    fail("usage: swift scripts/make-dmg-background.swift <out.tiff> <version>")
}
let version = args[1]
let reps = [render(scale: 1, version: version), render(scale: 2, version: version)]
guard let tiff = NSBitmapImageRep.tiffRepresentationOfImageReps(in: reps, using: .lzw, factor: 0)
else { fail("cannot encode TIFF") }
do {
    try tiff.write(to: URL(fileURLWithPath: args[0]))
} catch {
    fail("cannot write \(args[0]): \(error.localizedDescription)")
}
print("make-dmg-background: \(args[0])")
