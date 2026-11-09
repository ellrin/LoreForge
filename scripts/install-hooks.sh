#!/bin/sh
# Points this repository at scripts/hooks (versioned hooks; overrides a global core.hooksPath
# for this repo only — scripts/hooks/commit-msg still runs the global commit-msg hook).
set -e
root=$(git rev-parse --show-toplevel)
git -C "$root" config core.hooksPath scripts/hooks
rm -f "$root/.git/hooks/pre-commit"
echo "Enabled LoreForge hooks (private/, private words, large files, secret scan)."
