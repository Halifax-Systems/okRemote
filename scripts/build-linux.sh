#!/usr/bin/env bash
# Needs: libwebkit2gtk-4.1-dev libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev libdbus-1-dev pkg-config
set -euo pipefail
cd "$(dirname "$0")/../app"
export APPIMAGE_EXTRACT_AND_RUN=1 WEBKIT_DISABLE_DMABUF_RENDERER=1
npm ci
npx tauri build
