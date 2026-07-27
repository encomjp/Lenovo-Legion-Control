#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$ROOT/Cargo.toml" | head -n1)"
ARCH="${DEB_ARCH:-$(dpkg --print-architecture)}"
OUT="${OUT_DIR:-$ROOT/packaging/out}"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

for binary in legion-cli legion-daemon legion-settings legion-control-setup; do
    test -x "$ROOT/target/release/$binary" || {
        echo "Missing target/release/$binary; run cargo build --release --locked first" >&2
        exit 1
    }
done

install -Dm755 "$ROOT/target/release/legion-cli" "$STAGE/usr/bin/legion-cli"
install -Dm755 "$ROOT/target/release/legion-daemon" "$STAGE/usr/bin/legion-daemon"
install -Dm755 "$ROOT/target/release/legion-settings" "$STAGE/usr/bin/legion-settings"
install -Dm755 "$ROOT/target/release/legion-control-setup" "$STAGE/usr/libexec/legion-control-setup"
install -Dm644 "$ROOT/data/polkit/com.encomjp.legion-control.policy" \
    "$STAGE/usr/share/polkit-1/actions/com.encomjp.legion-control.policy"
mkdir -p "$STAGE/usr/lib/legion-control/ryzen_smu"
cp -a "$ROOT/third_party/ryzen_smu/." "$STAGE/usr/lib/legion-control/ryzen_smu/"
install -Dm644 "$ROOT/packaging/common/legion-control.service" \
    "$STAGE/usr/lib/systemd/system/legion-control.service"
install -Dm644 "$ROOT/data/udev/99-legion.rules" \
    "$STAGE/usr/lib/udev/rules.d/99-legion.rules"
install -Dm644 "$ROOT/data/gui/com.encomjp.legion-settings.desktop" \
    "$STAGE/usr/share/applications/com.encomjp.legion-settings.desktop"
install -Dm644 "$ROOT/data/icons/app-mark.svg" \
    "$STAGE/usr/share/icons/hicolor/scalable/apps/com.encomjp.legion-settings.svg"
install -Dm644 "$ROOT/data/icons/tray.svg" \
    "$STAGE/usr/share/icons/hicolor/scalable/status/com.encomjp.legion-settings-tray.svg"
install -Dm644 "$ROOT/README.md" "$STAGE/usr/share/doc/legion-control/README.md"

mkdir -p "$STAGE/DEBIAN"
cat > "$STAGE/DEBIAN/control" <<EOF
Package: legion-control
Version: $VERSION
Section: utils
Priority: optional
Architecture: $ARCH
Maintainer: europeanpepe <noreply@github.com>
Depends: libc6, libgtk-4-1 (>= 4.14), libadwaita-1-0 (>= 1.5), libudev1, systemd, policykit-1
Suggests: plasma-workspace, dkms, make, linux-headers-generic
Description: Lenovo Legion hardware control suite
 Controls fan profiles, platform power modes, battery charge limits,
 Spectrum RGB lighting, telemetry, and provides a GTK settings application.
EOF
for script in postinst prerm postrm; do
    install -m755 "$ROOT/packaging/debian/$script" "$STAGE/DEBIAN/$script"
done

mkdir -p "$OUT"
PACKAGE="$OUT/legion-control_${VERSION}_${ARCH}.deb"
dpkg-deb --root-owner-group --build "$STAGE" "$PACKAGE"
echo "$PACKAGE"
