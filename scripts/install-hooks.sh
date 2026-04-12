#!/usr/bin/env bash
# Point git at the checked-in .githooks/ directory. Run once per clone.

set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

git config core.hooksPath .githooks
chmod +x .githooks/*

echo "installed: git hooks now run from .githooks/"
echo "  pre-commit: validates doc samples when docs/*.md is staged"
echo "  bypass:     git commit --no-verify"
