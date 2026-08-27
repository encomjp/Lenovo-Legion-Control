#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$ROOT/Cargo.toml" | head -n1)"
OUT="${1:-$ROOT/../packaged}"
APPIMAGETOOL="${APPIMAGETOOL:-$(command -v appimagetool || true)}"

if [[ -z "$APPIMAGETOOL" ]]; then
    echo "appimagetool not found; set APPIMAGETOOL=/path/to/appimagetool" >&2
    exit 1
fi

cargo build --release --locked --manifest-path "$ROOT/Cargo.toml"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
APPDIR="$WORK/LegionControl.AppDir"
mkdir -p "$OUT"
mkdir -p "$APPDIR/usr/bin" \
    "$APPDIR/usr/lib/systemd/system" \
    "$APPDIR/usr/lib/udev/rules.d" \
    "$APPDIR/usr/lib/legion-control/ryzen_smu" \
    "$APPDIR/usr/libexec" \
    "$APPDIR/usr/share/applications" \
    "$APPDIR/usr/share/icons/hicolor/scalable/apps" \
    "$APPDIR/usr/share/icons/hicolor/scalable/status" \
    "$APPDIR/usr/share/polkit-1/actions"

for binary in legion-cli legion-daemon legion-settings legion-control-setup; do
    install -Dm755 "$ROOT/target/release/$binary" "$APPDIR/usr/$(
        [[ "$binary" == legion-control-setup ]] && printf 'libexec' || printf 'bin'
    )/$binary"
done
install -Dm755 "$ROOT/packaging/appimage/AppRun" "$APPDIR/AppRun"
install -Dm644 "$ROOT/data/gui/com.encomjp.legion-settings.desktop" \
    "$APPDIR/com.encomjp.legion-settings.desktop"
install -Dm644 "$ROOT/data/gui/com.encomjp.legion-settings.desktop" \
    "$APPDIR/usr/share/applications/com.encomjp.legion-settings.desktop"
install -Dm644 "$ROOT/data/icons/app-mark.svg" \
    "$APPDIR/usr/share/icons/hicolor/scalable/apps/com.encomjp.legion-settings.svg"
install -Dm644 "$ROOT/data/icons/tray.svg" \
    "$APPDIR/usr/share/icons/hicolor/scalable/status/com.encomjp.legion-settings-tray.svg"
ln -s "usr/share/icons/hicolor/scalable/apps/com.encomjp.legion-settings.svg" \
    "$APPDIR/.DirIcon"
install -Dm644 "$ROOT/data/polkit/com.encomjp.legion-control.policy" \
    "$APPDIR/usr/share/polkit-1/actions/com.encomjp.legion-control.policy"
install -Dm644 "$ROOT/packaging/common/legion-control.service" \
    "$APPDIR/usr/lib/systemd/system/legion-control.service"
install -Dm644 "$ROOT/data/udev/99-legion.rules" \
    "$APPDIR/usr/lib/udev/rules.d/99-legion.rules"
cp -a "$ROOT/third_party/ryzen_smu/." "$APPDIR/usr/lib/legion-control/ryzen_smu/"

PACKAGE="$OUT/legion-control-${VERSION}-x86_64.AppImage"
rm -f "$PACKAGE"
"$APPIMAGETOOL" --appimage-extract-and-run "$APPDIR" "$PACKAGE"
chmod +x "$PACKAGE"
printf '%s\n' "$PACKAGE"
