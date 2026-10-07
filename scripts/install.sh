#!/usr/bin/env bash
# Explicit developer installation. Never replaces or re-signs an existing app.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [[ $# != 3 || "$1" != "--development" || "$2" != "--destination" ]]; then
  echo 'Usage: scripts/install.sh --development --destination /absolute/new/Bomb\ Code.app' >&2
  echo 'Builds a developer bundle; destination must not exist. See docs/release/MACOS_DISTRIBUTION.md.' >&2
  exit 2
fi
DEST="$3"

# Tauri may notarize automatically when these credentials are present. This
# developer installer must never transmit a build to Apple implicitly.
for name in APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID APPLE_API_KEY APPLE_API_KEY_PATH APPLE_API_ISSUER; do
  if [[ -n "${!name:-}" ]]; then
    echo 'ERROR: notarization environment is present; use the reviewed release workflow instead.' >&2
    exit 2
  fi
done

python3 - "$DEST" <<'PY'
import os
from pathlib import Path
import sys
path = Path(sys.argv[1])
if not path.is_absolute() or path.suffix != '.app' or os.path.lexists(path):
    sys.exit('ERROR: destination must be an absolute, new .app path')
if not path.parent.is_dir():
    sys.exit('ERROR: destination parent must already exist')
PY

export PATH="${HOME}/.grok/bin:${HOME}/.cargo/bin:${HOME}/.local/bin:/opt/homebrew/bin:/usr/local/bin:${PATH}"
if ! command -v grok >/dev/null 2>&1; then
  echo 'ERROR: grok CLI not found. Install Grok Build first.' >&2
  exit 1
fi
echo 'Building developer app bundle. This does not qualify a production release.'
cargo tauri build --bundles app
SRC="${CARGO_TARGET_DIR:-${ROOT}/target}/release/bundle/macos/Bomb Code.app"
codesign --verify --deep --strict "$SRC"

python3 - "$SRC" "$DEST" <<'PY'
import shutil
import sys
# copytree exclusively creates the new destination, including after a race.
# Existing installations and their signatures are never removed or replaced.
shutil.copytree(sys.argv[1], sys.argv[2], symlinks=True)
PY
codesign --verify --deep --strict "$DEST"
echo "Developer app copied with its existing signature: $DEST"
echo "Launch explicitly after selecting a QA profile: open \"$DEST\""
echo 'This installer does not launch the app or certify signing/notarization.'
