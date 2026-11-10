#!/usr/bin/env bash
# Install the marketplace plugin into ~/.config/omarchy/plugins.
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
# remove leftover legacy id
rm -rf "${XDG_CONFIG_HOME:-$HOME/.config}/omarchy/plugins/ghost.icdown"

if omarchy plugin enable "$ID" --section right >/dev/null 2>&1; then
  echo "Enabled $ID on the right section of the bar."
else
  echo "Copied $ID to $DEST"
  echo "Enable with: omarchy plugin enable $ID --section right"
fi
omarchy-shell shell rescanPlugins >/dev/null 2>&1 || true
echo "Marketplace install alternative:"
echo "  omarchy plugin add https://github.com/hljghost/omarchy-icdown-plugin.git --enable"
