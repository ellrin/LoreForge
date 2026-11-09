// 產生 App 圖示：Resources/AppIcon.icns
// 用法：swift scripts/make-icon.swift （build.sh 會在圖示不存在時自動執行）
import AppKit

let out = URL(fileURLWithPath: CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "Resources/AppIcon.icns")
let iconset = FileManager.default.temporaryDirectory.appendingPathComponent("NeonDeck.iconset")
try? FileManager.default.removeItem(at: iconset)
try FileManager.default.createDirectory(at: iconset, withIntermediateDirectories: true)

func color(_ hex: UInt32, _ a: CGFloat = 1) -> NSColor {
    NSColor(srgbRed: CGFloat(hex >> 16 & 255) / 255, green: CGFloat(hex >> 8 & 255) / 255, blue: CGFloat(hex & 255) / 255, alpha: a)
}

/// 在 1024 座標系畫圖，再縮放到指定像素
func render(_ px: Int) -> Data {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: px, pixelsHigh: px, bitsPerSample: 8,
                               samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
                               bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    let ctx = NSGraphicsContext(bitmapImageRep: rep)!
    NSGraphicsContext.current = ctx
    let g = ctx.cgContext
    g.scaleBy(x: CGFloat(px) / 1024, y: CGFloat(px) / 1024)

    // 圓角方塊底（macOS 圖示格線：824 見方、留 100 邊）
    let body = NSBezierPath(roundedRect: NSRect(x: 100, y: 100, width: 824, height: 824), xRadius: 185, yRadius: 185)
    NSGraphicsContext.saveGraphicsState()
    let shadow = NSShadow()
    shadow.shadowColor = NSColor.black.withAlphaComponent(0.5)
    shadow.shadowBlurRadius = 24
    shadow.shadowOffset = NSSize(width: 0, height: -12)
    shadow.set()
    NSGradient(colors: [color(0x1A0D33), color(0x05030B)])!.draw(in: body, angle: -90)
    NSGraphicsContext.restoreGraphicsState()
    body.addClip()

    // 背景紫色光暈
    NSGradient(colors: [color(0xC04BFF, 0.45), color(0xC04BFF, 0)])!
        .draw(fromCenter: NSPoint(x: 512, y: 180), radius: 0, toCenter: NSPoint(x: 512, y: 180), radius: 620, options: [])

    // 斜帶：14° 平行四邊形
    let k = tan(14 * CGFloat.pi / 180)
    func band(_ x: CGFloat, _ w: CGFloat, _ c: NSColor) {
        let p = NSBezierPath()
        let lo: CGFloat = 60, hi: CGFloat = 964
        p.move(to: NSPoint(x: x + k * (lo - 512), y: lo))
        p.line(to: NSPoint(x: x + w + k * (lo - 512), y: lo))
        p.line(to: NSPoint(x: x + w + k * (hi - 512), y: hi))
        p.line(to: NSPoint(x: x + k * (hi - 512), y: hi))
        p.close()
        c.setFill(); p.fill()
    }
    // 紫色擠出陰影 → 黃色主帶 → 紫窄帶 → 藍細線
    for i in stride(from: 6, through: 1, by: -1) {
        band(300 + CGFloat(i) * 9, 230, color(0x6E20B8).blended(withFraction: CGFloat(6 - i) / 8, of: color(0xD35CFF))!)
    }
    band(300, 230, color(0xFFE600))
    band(590, 70, color(0xC04BFF))
    band(700, 16, color(0x2DE2FF))

    // 霓虹描邊
    color(0xFFE600, 0.5).setStroke()
    body.lineWidth = 10
    body.stroke()

    NSGraphicsContext.restoreGraphicsState()
    return rep.representation(using: .png, properties: [:])!
}

for (name, px) in [("16x16", 16), ("16x16@2x", 32), ("32x32", 32), ("32x32@2x", 64), ("128x128", 128),
                   ("128x128@2x", 256), ("256x256", 256), ("256x256@2x", 512), ("512x512", 512), ("512x512@2x", 1024)] {
    try render(px).write(to: iconset.appendingPathComponent("icon_\(name).png"))
}
let p = Process()
p.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
p.arguments = ["-c", "icns", iconset.path, "-o", out.path]
try p.run(); p.waitUntilExit()
print(p.terminationStatus == 0 ? "✔ \(out.path)" : "iconutil failed")
