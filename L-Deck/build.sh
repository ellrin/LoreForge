#!/bin/zsh
# 用法：
#   ./build.sh          編譯到 build/L-Deck.app
#   ./build.sh install  編譯 + 安裝到 ~/Applications + 啟動（桌面資料夾指向本專案的 desktop/，改檔即時生效）
set -euo pipefail
cd "$(dirname "$0")"

APP=build/L-Deck.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

swiftc -O -swift-version 5 -target arm64-apple-macos14.0 \
  -framework IOKit -framework EventKit -framework WebKit \
  Sources/*.swift -o "$APP/Contents/MacOS/L-Deck"

[[ -f Resources/AppIcon.icns ]] || swift scripts/make-icon.swift Resources/AppIcon.icns
cp Resources/Info.plist "$APP/Contents/Info.plist"
cp Resources/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
cp -R desktop "$APP/Contents/Resources/desktop"     # 內建範本（第一次在別台電腦啟動時會複製出去）
codesign --force --deep --sign - "$APP" >/dev/null
echo "✔ built $APP"

if [[ "${1:-}" == "install" ]]; then
  pkill -x L-Deck 2>/dev/null || true
  pkill -x NeonDeck 2>/dev/null || true   # 改名前的執行檔
  mkdir -p ~/Applications
  rm -rf ~/Applications/L-Deck.app
  cp -R "$APP" ~/Applications/
  defaults write com.local.neondeck deckRoot "$PWD/desktop"
  open ~/Applications/L-Deck.app
  echo "✔ installed to ~/Applications/L-Deck.app（桌面資料夾：$PWD/desktop）"
fi
