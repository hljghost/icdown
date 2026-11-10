#!/usr/bin/env bash
# Install the Omarchy bar plugin from this repo into ~/.config/omarchy/plugins.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="$ROOT/omarchy-plugin"
ID="$(jq -r .id "$SRC/manifest.json")"
DEST="${XDG_CONFIG_HOME:-$HOME/.config}/omarchy/plugins/$ID"

omarchy plugin validate "$SRC"

mkdir -p "$(dirname "$DEST")"
rm -rf "$DEST"
cp -a "$SRC" "$DEST"
chmod 755 "$DEST/status.py"

# Prefer enabling into the bar when omarchy-shell is available.
if omarchy plugin enable "$ID" --section right >/dev/null 2>&1; then
  echo "Enabled $ID on the right section of the bar."
else
  echo "Copied $ID to $DEST"
  echo "Enable with: omarchy plugin enable $ID --section right"
fi

omarchy-shell shell rescanPlugins >/dev/null 2>&1 || true
echo "Done. If the bar is running, the widget should hot-reload."
