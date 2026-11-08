#!/bin/sh
# Development launcher for macOS.
#
# Debug builds are unsigned and change identity on every rebuild, so macOS
# asks for the login password each time they read the keychain, and "Always
# Allow" does not stick. This script lets Apple's own `security` tool hold the
# key instead (it never prompts for items it created) and passes it to the app
# through ANTHROPIC_API_KEY for this run only. Nothing is written to a file.
#
#   scripts/dev.sh                # desktop app
#   scripts/dev.sh pc-server      # browser mode, http://127.0.0.1:7878
#   scripts/dev.sh --reset-key    # forget the stored key and ask again

set -e
SVC=PaperComposer-dev
ACCT=anthropic-api-key

if [ "$1" = "--reset-key" ]; then
  security delete-generic-password -s "$SVC" -a "$ACCT" >/dev/null 2>&1 || true
  shift
fi

if ! key=$(security find-generic-password -s "$SVC" -a "$ACCT" -w 2>/dev/null); then
  echo "第一次使用：貼上 Claude API key（輸入時不會顯示，按 Enter 後需再輸入一次確認）"
  security add-generic-password -s "$SVC" -a "$ACCT" -w
  key=$(security find-generic-password -s "$SVC" -a "$ACCT" -w)
fi

cd "$(dirname "$0")/.."
ANTHROPIC_API_KEY="$key" exec cargo run -p "${1:-papercomposer}"
