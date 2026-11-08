#!/bin/sh
# Installs the repository's git hooks into .git/hooks.
set -e
root=$(git rev-parse --show-toplevel)
cp "$root/scripts/pre-commit" "$root/.git/hooks/pre-commit"
chmod +x "$root/.git/hooks/pre-commit"
echo "Installed pre-commit secret scan."
