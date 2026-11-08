#!/bin/sh
# 讓 kb-server 在登入時自動啟動（macOS launchd 使用者代理程式）。
#
#   scripts/install-launchd.sh             編譯 release、複製執行檔、寫入並載入 LaunchAgent
#   scripts/install-launchd.sh --print     只印出會寫入的 plist（不改任何東西）
#   scripts/install-launchd.sh --uninstall 停止並移除 LaunchAgent（執行檔與設定保留）
#
# 額外參數會原樣傳給 kb-server，例如：
#   scripts/install-launchd.sh -- --space ~/Documents/KnowledgeBase/private --member ellrin
# 沒給參數時，kb-server 照平常的順序找 space（config.toml > 旁邊的 ../private）。
#
# 執行檔複製到 ~/Library/Application Support/KnowledgeBase/bin/kb-server，
# 之後重新編譯不會換掉它（鑰匙圈的存取權限綁在執行檔上），要更新就再跑一次這個腳本。
# 紀錄檔：~/Library/Logs/KnowledgeBase/kb-server.log
# 尚未在實機上測試（not tested on real hardware）。
set -eu

LABEL="local.knowledgebase.kb-server"
root=$(cd "$(dirname "$0")/.." && pwd)
app_dir="$HOME/Library/Application Support/KnowledgeBase"
bin="$app_dir/bin/kb-server"
log_dir="$HOME/Library/Logs/KnowledgeBase"
plist="$HOME/Library/LaunchAgents/$LABEL.plist"
domain="gui/$(id -u)"

mode=install
case "${1:-}" in
  --print) mode=print; shift ;;
  --uninstall) mode=uninstall; shift ;;
  --) ;;
  "") ;;
  -h|--help) sed -n '2,15p' "$0"; exit 0 ;;
  *) echo "未知的參數：$1（用 --help 看用法）" >&2; exit 2 ;;
esac
[ "${1:-}" = "--" ] && shift

xml_escape() { printf '%s' "$1" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g'; }

render() {
  args=""
  for a in "$@"; do
    case "$a" in "~/"*) a="$HOME/${a#\~/}" ;; esac
    args="$args    <string>$(xml_escape "$a")</string>
"
  done
  # awk handles the multi-line @ARGS@ and paths with any character.
  KB_ARGS="$args" awk -v label="$LABEL" -v bin="$(xml_escape "$bin")" -v work="$(xml_escape "$root")" \
      -v log="$(xml_escape "$log_dir/kb-server.log")" '
    function lit(s) { gsub(/&/, "\\\\&", s); return s }
    { gsub(/@LABEL@/, lit(label)); gsub(/@BIN@/, lit(bin)); gsub(/@WORKDIR@/, lit(work)); gsub(/@LOG@/, lit(log))
      if ($0 == "@ARGS@  </array>") { printf "%s  </array>\n", ENVIRON["KB_ARGS"]; next }
      print }' "$root/scripts/launchd/kb-server.plist.template"
}

case "$mode" in
  print)
    render "$@"
    ;;
  uninstall)
    launchctl bootout "$domain/$LABEL" 2>/dev/null || true
    rm -f "$plist"
    echo "已移除 $plist（執行檔與設定保留在 $app_dir）"
    ;;
  install)
    echo "編譯 kb-server（release）…"
    (cd "$root" && cargo build --release -p kb-server)
    target="${CARGO_TARGET_DIR:-$root/target}"
    mkdir -p "$app_dir/bin" "$log_dir" "$HOME/Library/LaunchAgents"
    cp "$target/release/kb-server" "$bin.new" && mv "$bin.new" "$bin"
    render "$@" > "$plist.new"
    plutil -lint "$plist.new" >/dev/null
    mv "$plist.new" "$plist"
    launchctl bootout "$domain/$LABEL" 2>/dev/null || true
    launchctl bootstrap "$domain" "$plist"
    echo "已安裝並啟動：$LABEL"
    echo "  plist：$plist"
    echo "  紀錄：$log_dir/kb-server.log"
    echo "  開啟：http://127.0.0.1:7700"
    ;;
esac
