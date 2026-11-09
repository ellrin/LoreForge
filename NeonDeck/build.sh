#!/bin/zsh
# 用法：
#   ./build.sh          編譯到 build/NeonDeck.app
#   ./build.sh install  編譯 + 安裝到 ~/Applications + 啟動（桌面資料夾指向本專案的 desktop/，改檔即時生效）
set -euo pipefail
cd "$(dirname "$0")"

APP=build/NeonDeck.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

swiftc -O -swift-version 5 -target arm64-apple-macos14.0 \
  -framework IOKit -framework EventKit -framework WebKit \
  Sources/*.swift -o "$APP/Contents/MacOS/NeonDeck"

[[ -f Resources/AppIcon.icns ]] || swift scripts/make-icon.swift Resources/AppIcon.icns
cp Resources/Info.plist "$APP/Contents/Info.plist"
cp Resources/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
cp -R desktop "$APP/Contents/Resources/desktop"     # 內建範本（第一次在別台電腦啟動時會複製出去）
codesign --force --deep --sign - "$APP" >/dev/null
echo "✔ built $APP"

if [[ "${1:-}" == "install" ]]; then
  pkill -x NeonDeck 2>/dev/null || true
  mkdir -p ~/Applications
  rm -rf ~/Applications/NeonDeck.app
  cp -R "$APP" ~/Applications/
  defaults write com.local.neondeck deckRoot "$PWD/desktop"
  open ~/Applications/NeonDeck.app
  echo "✔ installed to ~/Applications/NeonDeck.app（桌面資料夾：$PWD/desktop）"
fi
