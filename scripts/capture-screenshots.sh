#!/usr/bin/env zsh
# Capture every Legion Control page headless — does not obscure your real desktop.
# Uses an isolated Xvfb :99 + openbox session, so your Wayland/KWin session stays untouched.
# The running `legion-settings --hidden` tray instance is briefly replaced and auto-restarted.
#
# Requirements: Xvfb, openbox, xdotool, xorg-xwd, ImageMagick (magick), cargo
# Usage: ./scripts/capture-screenshots.sh
set -e
OUTDIR="$(dirname "$0")/../docs/assets/screenshots"
mkdir -p "$OUTDIR"
echo "Output dir $OUTDIR"

if ! command -v Xvfb >/dev/null; then echo "Xvfb missing — install xorg-server-xvfb"; exit 1; fi
if ! command -v openbox >/dev/null; then echo "openbox missing — pacman -S openbox"; exit 1; fi
if ! command -v xdotool >/dev/null; then echo "xdotool missing"; exit 1; fi
if ! command -v magick >/dev/null; then echo "magick missing — install imagemagick"; exit 1; fi

echo "Building legion-settings..."
cargo build --bin legion-settings --quiet

echo "Killing hidden instance (will restart)..."
pkill -f "legion-settings --hidden" || true
sleep 1
pkill -f "Xvfb :99" || true
sleep 1

Xvfb :99 -screen 0 1400x900x24 -ac &
XVFB=$!
sleep 2
DISPLAY=:99 openbox --sm-disable &
OB=$!
sleep 1

typeset -A PAGES
PAGES=(
  "overview" "01-home-overview"
  "cpu-features" "02-cpu-features"
  "cpu-tuning" "03-cpu-tuning"
  "cpu-power" "04-cpu-power"
  "cooling-fans" "05-cooling"
  "lighting-keyboard" "06-lighting-keyboard"
  "lighting-front" "07-lighting-front"
  "lighting-rear" "08-lighting-rear"
  "lighting-logo" "09-lighting-logo"
  "lighting-more" "10-lighting-more"
  "battery-status" "11-battery"
  "profiles" "12-profiles"
  "about-setup" "13-settings-setup"
  "fix" "14-settings-fix"
  "about-hardware" "15-settings-hardware"
  "about-help" "16-settings-help"
)

BIN="$(dirname "$0")/../target/debug/legion-settings"
for PAGE in ${(k)PAGES}; do
  FILE="${PAGES[$PAGE]}"
  echo "=== $PAGE -> $FILE ==="
  DISPLAY=:99 WAYLAND_DISPLAY= GDK_BACKEND=x11 GSK_RENDERER=cairo LIBGL_ALWAYS_SOFTWARE=1 LEGION_PAGE=$PAGE timeout 10 "$BIN" > /tmp/legion_${FILE}.log 2>&1 &
  APP=$!
  sleep 5
  WIN=""
  for i in {1..10}; do
    WIN=$(DISPLAY=:99 xdotool search --onlyvisible --name "Legion Control" 2>/dev/null | head -n1)
    if [ -n "$WIN" ]; then break; fi
    sleep 0.5
  done
  if [ -z "$WIN" ]; then
    echo "WARN: no window for $PAGE"
    kill $APP 2>/dev/null || true
    pkill -f "legion-settings" || true
    sleep 1
    continue
  fi
  DISPLAY=:99 xwd -id $WIN -out /tmp/${FILE}.xwd
  magick /tmp/${FILE}.xwd "$OUTDIR/${FILE}.png"
  ls -lh "$OUTDIR/${FILE}.png"
  kill $APP 2>/dev/null || true
  pkill -f "legion-settings" || true
  sleep 1
done

kill $OB || true
kill $XVFB || true
wait $XVFB 2>/dev/null || true
# Only restart the tray instance if one was running before the capture —
# never launch the app on the user's desktop from a screenshot run.
if pgrep -f "legion-settings --hidden" >/dev/null 2>&1; then
  echo "Restarting hidden instance..."
  nohup "$BIN" --hidden > /tmp/legion_restart.log 2>&1 &
  sleep 1
  ps aux | grep legion-settings | grep -v grep | head
fi
echo "Done — images in $OUTDIR"
ls -lh "$OUTDIR"
