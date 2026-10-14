#!/usr/bin/env bash
# Capture every Legion Control page off-screen for the README.
#
# Runs on a private Xvfb display with GTK forced to X11, so nothing appears on
# your desktop and your running tray instance is left alone. Uses a throwaway
# copy of your settings (diagnostics off, no welcome dialog).
#
# Requirements: Xvfb, openbox, xdotool, ImageMagick (magick)
# Usage: scripts/capture-screenshots.sh [light|dark]   (default: dark)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUTDIR="$ROOT/docs/assets/screenshots"
THEME="${1:-dark}"
SCHEME=default; [[ "$THEME" == dark ]] && SCHEME=prefer-dark
D=:119

for c in Xvfb openbox xdotool magick; do
  command -v "$c" >/dev/null || { echo "missing: $c"; exit 1; }
done

cargo build --release --bin legion-settings --quiet --manifest-path "$ROOT/Cargo.toml"
BIN="$ROOT/target/release/legion-settings"
mkdir -p "$OUTDIR"

CFG=$(mktemp -d); mkdir -p "$CFG/legion-control"
SRC="$HOME/.config/legion-control/settings.json"
if [[ -f "$SRC" ]]; then
  python3 - "$SRC" "$CFG/legion-control/settings.json" <<'EOF'
import json, sys
d = json.load(open(sys.argv[1]))
d.update(restore_on_launch=False, welcome_seen=True)
d.setdefault("diagnostics", {})["enabled"] = False
json.dump(d, open(sys.argv[2], "w"))
EOF
fi

Xvfb $D -screen 0 1400x900x24 -nolisten tcp >/dev/null 2>&1 & XV=$!
sleep 2
DISPLAY=$D openbox --sm-disable >/dev/null 2>&1 & OB=$!
sleep 1
trap 'kill $OB $XV 2>/dev/null || true; rm -rf "$CFG"' EXIT

PAGES=(
  "overview:01-home-overview"
  "cpu-features:02-cpu-features"
  "cpu-tuning:03-cpu-tuning"
  "cpu-power:04-cpu-power"
  "cooling-fans:05-cooling"
  "lighting-keyboard:06-lighting-keyboard"
  "lighting-more:10-lighting-more"
  "battery-status:11-battery"
  "profiles:12-profiles"
  "about-setup:13-settings-setup"
  "health:14-health"
  "about-hardware:15-settings-hardware"
  "about-help:16-settings-help"
)

for entry in "${PAGES[@]}"; do
  page=${entry%%:*}; file=${entry#*:}
  env -u WAYLAND_DISPLAY GDK_BACKEND=x11 DISPLAY=$D LEGION_PAGE=$page \
    DBUS_SESSION_BUS_ADDRESS=unix:path=/nonexistent ADW_DEBUG_COLOR_SCHEME=$SCHEME \
    XDG_CONFIG_HOME="$CFG" "$BIN" >/dev/null 2>&1 & APP=$!
  sleep 4
  DISPLAY=$D xdotool mousemove 1390 890 2>/dev/null || true   # no hover tooltips
  sleep 1
  DISPLAY=$D import -window root "$CFG/shot.png"
  magick "$CFG/shot.png" -trim +repage "$OUTDIR/$file.png"
  kill $APP 2>/dev/null || true; wait $APP 2>/dev/null || true
  echo "$page -> $file.png"
done
echo "Done: $OUTDIR"
