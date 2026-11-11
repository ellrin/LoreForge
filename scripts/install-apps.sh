#!/bin/zsh
# 編譯 LoreForge 的 Mac App、安裝並放進 Dock。
#
#   scripts/install-apps.sh                     全部：PaperComposer SlideComposer KnowledgeBase LoreReader LoreEcho NeonDeck
#   scripts/install-apps.sh SlideComposer ...   只做指定的 App
#   scripts/install-apps.sh --no-dock ...       不動 Dock
#
# Tauri App（PaperComposer、SlideComposer、KnowledgeBase、LoreReader、LoreEcho）裝到 /Applications，
# NeonDeck 用它自己的 build.sh 裝到 ~/Applications。安裝前會先結束正在執行的舊版。
# Dock：每個 App 一個項目；名稱相同但指向別的位置（例如舊的編譯資料夾）的項目會被換掉，
# 其他項目不動；LoreEcho 是選單列小工具，不放進 Dock。
# 簽章是 ad-hoc（沒有 Developer ID），所以換版後第一次讀鑰匙圈時系統會再問一次。
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)

dock=1
apps=()
for a in "$@"; do
  case $a in
    --no-dock) dock=0 ;;
    -h|--help) sed -n '2,12p' "$0"; exit 0 ;;
    PaperComposer|SlideComposer|KnowledgeBase|LoreReader|LoreEcho|NeonDeck) apps+=$a ;;
    *) echo "不認得的 App：$a" >&2; exit 2 ;;
  esac
done
(( ${#apps} )) || apps=(PaperComposer SlideComposer KnowledgeBase LoreReader LoreEcho NeonDeck)

quit() { osascript -e "tell application id \"$1\" to quit" >/dev/null 2>&1 || true; }

installed=()
for a in $apps; do
  echo "== $a"
  if [[ $a == NeonDeck ]]; then
    (cd "$root/NeonDeck" && ./build.sh install)
    installed+="$HOME/Applications/NeonDeck.app"
    continue
  fi
  id=$(sed -n 's/.*"identifier": *"\([^"]*\)".*/\1/p' "$root/$a/apps/desktop/tauri.conf.json")
  (cd "$root/$a/apps/desktop" && cargo tauri build --bundles app)
  quit "$id"
  sleep 1
  rm -rf "/Applications/$a.app"
  cp -R "$root/$a/target/release/bundle/macos/$a.app" /Applications/
  echo "✔ /Applications/$a.app"
  [[ $a == LoreEcho ]] || installed+="/Applications/$a.app"   # menu-bar only, no Dock tile
done

if (( dock )); then
  # Dock 的設定是 plist：匯出、改 persistent-apps、匯入、重新啟動 Dock。
  defaults export com.apple.dock - | python3 -I -c '
import plistlib, sys, urllib.parse, os
apps = sys.argv[1:]
d = plistlib.loads(sys.stdin.buffer.read())
items = d.setdefault("persistent-apps", [])
def url(p): return "file://" + urllib.parse.quote(p.rstrip("/") + "/")
def name(it): return it.get("tile-data", {}).get("file-label", "")
def tile(label, u): return {"tile-type": "file-tile", "tile-data": {"file-label": label,
    "file-data": {"_CFURLString": u, "_CFURLStringType": 15}}}
for path in apps:
    label = os.path.basename(path)[:-4]
    want = url(path)
    out, found = [], False
    for it in items:
        u = it.get("tile-data", {}).get("file-data", {}).get("_CFURLString", "")
        if u == want and not found:
            found = True
        elif name(it) == label or u == want:
            # Same app at another place (an old build folder) or a duplicate:
            # the first one becomes a fresh tile in the same spot, the rest go.
            print(f"  Dock：換掉舊的 {label}（{urllib.parse.unquote(u)}）", file=sys.stderr)
            if found:
                continue
            it, found = tile(label, want), True
        out.append(it)
    if not found:
        out.append(tile(label, want))
        print(f"  Dock：加入 {label}", file=sys.stderr)
    items[:] = out
sys.stdout.buffer.write(plistlib.dumps(d))
' $installed | defaults import com.apple.dock -
  killall Dock
fi
echo "完成。"
