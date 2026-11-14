#!/bin/sh
# Finder opens these types in the Lore apps on this Mac (no Word / PowerPoint):
#   .pdf → LoreReader, .pptx → SlideComposer, .docx and .tex → PaperComposer.
# Idempotent: a type already set is left alone. Apps not in /Applications are skipped.
# macOS may ask to confirm a change. Run after scripts/install-apps.sh, so the
# installed apps carry their file types (tauri.conf.json fileAssociations).
set -eu

[ "$(uname)" = Darwin ] || { echo "只用於 macOS"; exit 1; }
command -v swift >/dev/null 2>&1 || { echo "需要 swift（xcode-select --install）"; exit 1; }

# let Launch Services read each installed app's current Info.plist
LSREG=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
for app in LoreReader SlideComposer PaperComposer; do
  [ -d "/Applications/$app.app" ] && [ -x "$LSREG" ] && "$LSREG" -f "/Applications/$app.app" || true
done

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cat > "$tmp/set.swift" <<'SWIFT'
import AppKit
import UniformTypeIdentifiers

let pairs = [("pdf", "LoreReader"), ("pptx", "SlideComposer"), ("docx", "PaperComposer"), ("tex", "PaperComposer")]
let ws = NSWorkspace.shared
var failed = false
for (ext, app) in pairs {
    let url = URL(fileURLWithPath: "/Applications/\(app).app")
    guard FileManager.default.fileExists(atPath: url.path) else {
        print("略過  .\(ext)：\(app) 沒有安裝")
        continue
    }
    guard let type = UTType(filenameExtension: ext) else {
        print("略過  .\(ext)：系統不認得這個類型")
        continue
    }
    let current = ws.urlForApplication(toOpen: type)
    if current?.standardizedFileURL.path == url.standardizedFileURL.path {
        print("不變  .\(ext) → \(app)")
        continue
    }
    var done = false
    var err: Error?
    ws.setDefaultApplication(at: url, toOpen: type) { e in
        err = e
        done = true
    }
    // the answer comes back on the main queue: keep the run loop going until it does
    while !done { RunLoop.main.run(until: Date().addingTimeInterval(0.1)) }
    if let e = err {
        print("失敗  .\(ext) → \(app)：\(e.localizedDescription)")
        failed = true
    } else {
        let before = current.map { $0.deletingPathExtension().lastPathComponent } ?? "（無）"
        print("改成  .\(ext) → \(app)（原本 \(before)）")
    }
}
exit(failed ? 1 : 0)
SWIFT
swift "$tmp/set.swift"
