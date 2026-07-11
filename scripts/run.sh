#!/usr/bin/env bash
# Launch Grok Build Control Panel (dev or installed app).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# Ensure CLI tools are on PATH for this process tree.
export PATH="${HOME}/.grok/bin:${HOME}/.cargo/bin:${HOME}/.local/bin:/opt/homebrew/bin:/usr/local/bin:${PATH}"

APP_BUNDLE="${ROOT}/target/release/bundle/macos/Grok Build Control Panel.app"
INSTALLED="/Applications/Grok Build Control Panel.app"
BIN="${ROOT}/target/release/grok-build-control-panel"

if [[ "${1:-}" == "--dev" ]]; then
  exec cargo tauri dev
fi

if [[ -d "$INSTALLED" ]]; then
  echo "Opening installed app: $INSTALLED"
  exec open "$INSTALLED"
fi

if [[ -d "$APP_BUNDLE" ]]; then
  echo "Opening built app: $APP_BUNDLE"
  exec open "$APP_BUNDLE"
fi

if [[ -x "$BIN" ]]; then
  echo "Running release binary: $BIN"
  exec "$BIN"
fi

echo "No build found. Building app bundle…"
cargo tauri build --bundles app
exec open "$APP_BUNDLE"
