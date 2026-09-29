#!/usr/bin/env bash
# Build GLib / GTK4 / libadwaita from source into $PREFIX on an old base
# (Ubuntu 22.04) so the AppImage only needs glibc 2.35 on the host.
# The distro versions there are too old for our gtk4 v4_14 / adw v1_5 features.
set -euo pipefail

PREFIX="${PREFIX:-/opt/gnome}"
GLIB_VERSION="${GLIB_VERSION:-2.80.5}"
GTK_VERSION="${GTK_VERSION:-4.14.5}"
ADW_VERSION="${ADW_VERSION:-1.5.3}"
WAYLAND_VERSION="${WAYLAND_VERSION:-1.23.1}"
WAYLAND_PROTOCOLS_VERSION="${WAYLAND_PROTOCOLS_VERSION:-1.36}"

export PKG_CONFIG_PATH="$PREFIX/lib/x86_64-linux-gnu/pkgconfig:$PREFIX/lib/pkgconfig:$PREFIX/share/pkgconfig:${PKG_CONFIG_PATH:-}"
export LD_LIBRARY_PATH="$PREFIX/lib/x86_64-linux-gnu:$PREFIX/lib:${LD_LIBRARY_PATH:-}"
export PATH="$PREFIX/bin:$PATH"

SRC="$(mktemp -d)"
trap 'rm -rf "$SRC"' EXIT

fetch() { # name version
    local series="${2%.*}"
    curl -fsSL "https://download.gnome.org/sources/$1/$series/$1-$2.tar.xz" | tar -xJ -C "$SRC"
}

build() { # dir meson-args...
    local dir="$1"; shift
    meson setup "$SRC/$dir/_build" "$SRC/$dir" --prefix="$PREFIX" --buildtype=release \
        --wrap-mode=default "$@"
    meson compile -C "$SRC/$dir/_build"
    meson install -C "$SRC/$dir/_build"
}

fetch glib "$GLIB_VERSION"
build "glib-$GLIB_VERSION" -Dintrospection=disabled -Dtests=false -Dman-pages=disabled \
    -Ddocumentation=false -Dselinux=disabled

# GTK 4.14 needs wayland >= 1.21 / wayland-protocols >= 1.31 (jammy: 1.20 / 1.25).
fetch_fdo() { # project version
    curl -fsSL "https://gitlab.freedesktop.org/wayland/$1/-/releases/$2/downloads/$1-$2.tar.xz" | tar -xJ -C "$SRC"
}
fetch_fdo wayland "$WAYLAND_VERSION"
build "wayland-$WAYLAND_VERSION" -Ddocumentation=false -Dtests=false
fetch_fdo wayland-protocols "$WAYLAND_PROTOCOLS_VERSION"
build "wayland-protocols-$WAYLAND_PROTOCOLS_VERSION" -Dtests=false

fetch gtk "$GTK_VERSION"
build "gtk-$GTK_VERSION" -Dintrospection=disabled -Ddocumentation=false -Dman-pages=false \
    -Dbuild-demos=false -Dbuild-examples=false -Dbuild-tests=false -Dbuild-testsuite=false \
    -Dmedia-gstreamer=disabled -Dprint-cpdb=disabled -Dprint-cups=disabled \
    -Dvulkan=disabled -Dcloudproviders=disabled -Dsysprof=disabled -Dtracker=disabled \
    -Dcolord=disabled -Dx11-backend=true -Dwayland-backend=true

fetch libadwaita "$ADW_VERSION"
build "libadwaita-$ADW_VERSION" -Dintrospection=disabled -Dvapi=false -Dgtk_doc=false \
    -Dtests=false -Dexamples=false
