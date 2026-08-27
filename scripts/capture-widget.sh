#!/usr/bin/env zsh
# Capture the KDE Plasma widget preview headless via plasmawindowed on Xvfb :99.
# Isolated display — the real desktop session is never touched.
# Requires: Xvfb, openbox, xdotool, xorg-xwd, ImageMagick (magick), kpackagetool6
# Usage: ./scripts/capture-widget.sh
set -e
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUTDIR="$ROOT/docs/assets/screenshots"
OUTFILE="19-widget"
mkdir -p "$OUTDIR"

if ! command -v Xvfb >/dev/null; then echo "Xvfb missing — install xorg-server-xvfb"; exit 1; fi
if ! command -v plasmawindowed >/dev/null; then echo "plasmawindowed missing — install Plasma 6"; exit 1; fi
if ! command -v magick >/dev/null; then echo "magick missing — install imagemagick"; exit 1; fi

echo "Installing/updating the widget package (per-user, no root)..."
kpackagetool6 --type Plasma/Applet -i "$ROOT/kde-widget/package" 2>/dev/null \
  || kpackagetool6 --type Plasma/Applet -u "$ROOT/kde-widget/package"

pkill -f "Xvfb :99" 2>/dev/null || true
pkill -f "plasmawindowed" 2>/dev/null || true
sleep 1

Xvfb :99 -screen 0 1400x900x24 -ac &
XVFB=$!
sleep 2
DISPLAY=:99 openbox --sm-disable &
OB=$!
sleep 1

echo "Launching plasmawindowed..."
DISPLAY=:99 WAYLAND_DISPLAY= QT_QPA_PLATFORM=xcb LIBGL_ALWAYS_SOFTWARE=1 \
  plasmawindowed com.github.encomjp.legioncontrol > /tmp/legion_widget.log 2>&1 &
APP=$!

WIN=""
for i in {1..20}; do
  WIN=$(DISPLAY=:99 xdotool search --onlyvisible --name "Legion" 2>/dev/null | head -n1)
  if [ -n "$WIN" ]; then break; fi
  sleep 1
done
if [ -z "$WIN" ]; then
  echo "WARN: no widget window found"
  kill $APP $OB $XVFB 2>/dev/null || true
  exit 1
fi
sleep 4   # let the widget paint live sensor data

DISPLAY=:99 xwd -id $WIN -out /tmp/${OUTFILE}.xwd
magick /tmp/${OUTFILE}.xwd "$OUTDIR/${OUTFILE}.png"
ls -lh "$OUTDIR/${OUTFILE}.png"

kill $APP $OB $XVFB 2>/dev/null || true
echo "Done — $OUTDIR/${OUTFILE}.png"