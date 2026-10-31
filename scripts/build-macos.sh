#!/usr/bin/env bash
# Universal build by default; --native builds one architecture. Strips Homebrew/MacPorts from PATH so only system libraries are linked.
set -euo pipefail
cd "$(dirname "$0")/../app"
export PATH="$(echo "$PATH" | tr ':' '\n' | grep -v -e '/opt/homebrew' -e '/opt/local' | paste -sd: -)"
rustup target add aarch64-apple-darwin x86_64-apple-darwin
npm ci
if [ "${1:-}" = "--native" ]; then npx tauri build; else npx tauri build --target universal-apple-darwin; fi
