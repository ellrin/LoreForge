// 產生 App 圖示：Resources/logo.svg → Resources/AppIcon.icns（風格同 PaperComposer / SlideComposer / KnowledgeBase）
// 用法：swift scripts/make-icon.swift [輸出.icns]（build.sh 會在圖示不存在時自動執行）
// SVG 用 Quick Look（qlmanage）轉成 1024px，四角依圖示外形裁成透明，再用 iconutil 打包。
import AppKit

let out = URL(fileURLWithPath: CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "Resources/AppIcon.icns")
let svg = URL(fileURLWithPath: "Resources/logo.svg")
let fm = FileManager.default
let tmp = fm.temporaryDirectory.appendingPathComponent("neondeck-icon-\(getpid())")
try? fm.removeItem(at: tmp)
try fm.createDirectory(at: tmp, withIntermediateDirectories: true)
defer { try? fm.removeItem(at: tmp) }

func run(_ cmd: String, _ args: [String]) throws {
    let p = Process()
    p.executableURL = URL(fileURLWithPath: cmd)
    p.arguments = args
    p.standardOutput = FileHandle.nullDevice
    try p.run(); p.waitUntilExit()
    guard p.terminationStatus == 0 else { throw NSError(domain: cmd, code: Int(p.terminationStatus)) }
}

try run("/usr/bin/qlmanage", ["-t", "-s", "1024", "-o", tmp.path, svg.path])
guard let src = NSImage(contentsOf: tmp.appendingPathComponent("logo.svg.png")) else { fatalError("qlmanage 沒有產生 PNG") }

/// 依 logo.svg 的圓角方塊（x/y 32、寬高 960、圓角 216，1024 座標系）裁切，縮放到 px
func render(_ px: Int) -> Data {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: px, pixelsHigh: px, bitsPerSample: 8, samplesPerPixel: 4,
                               hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    let k = CGFloat(px) / 1024
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    NSGraphicsContext.current?.imageInterpolation = .high
    NSBezierPath(roundedRect: NSRect(x: 32 * k, y: 32 * k, width: 960 * k, height: 960 * k), xRadius: 216 * k, yRadius: 216 * k).addClip()
    src.draw(in: NSRect(x: 0, y: 0, width: CGFloat(px), height: CGFloat(px)))
    NSGraphicsContext.restoreGraphicsState()
    return rep.representation(using: .png, properties: [:])!
}

let iconset = tmp.appendingPathComponent("NeonDeck.iconset")
try fm.createDirectory(at: iconset, withIntermediateDirectories: true)
for s in [16, 32, 128, 256, 512] {
    try render(s).write(to: iconset.appendingPathComponent("icon_\(s)x\(s).png"))
    try render(s * 2).write(to: iconset.appendingPathComponent("icon_\(s)x\(s)@2x.png"))
}
try run("/usr/bin/iconutil", ["-c", "icns", iconset.path, "-o", out.path])
print("✔ \(out.path)")
