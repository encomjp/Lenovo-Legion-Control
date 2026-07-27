#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$ROOT/Cargo.toml" | head -n1)"
OUT="$ROOT/packaging/out"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$OUT"
rm -f "$OUT"/*.deb "$OUT"/*.rpm "$OUT"/*.pkg.tar.zst

SOURCE="$WORK/legion-control-$VERSION.tar.gz"
tar \
  --exclude='./target' \
  --exclude='./packaging/out' \
  --exclude='./.hermes' \
  --transform="s,^\.,legion-control-$VERSION," \
  -czf "$SOURCE" -C "$ROOT" .

printf '\n==> Building Debian package (Ubuntu 24.04)\n'
docker run --rm \
  -v "$SOURCE:/input/legion-control-$VERSION.tar.gz:ro" \
  -v "$OUT:/out" ubuntu:24.04 bash -lc "
    set -e
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    apt-get install -y -qq build-essential curl pkg-config dpkg-dev \
      libgtk-4-dev libadwaita-1-dev libglib2.0-dev libudev-dev >/dev/null
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable >/dev/null
    . /root/.cargo/env
    tar -xzf /input/legion-control-$VERSION.tar.gz -C /tmp
    cd /tmp/legion-control-$VERSION
    cargo build --release --locked
    chmod +x packaging/debian/build.sh packaging/debian/postinst packaging/debian/prerm packaging/debian/postrm
    OUT_DIR=/out packaging/debian/build.sh
  "

printf '\n==> Building RPM package (Fedora 42)\n'
docker run --rm \
  -v "$SOURCE:/input/legion-control-$VERSION.tar.gz:ro" \
  -v "$OUT:/out" fedora:42 bash -lc "
    set -e
    dnf install -y -q rpm-build cargo rust gtk4-devel libadwaita-devel \
      systemd-devel systemd-rpm-macros gcc gcc-c++ make >/dev/null
    mkdir -p /tmp/rpmbuild/{BUILD,BUILDROOT,RPMS,SOURCES,SPECS,SRPMS}
    cp /input/legion-control-$VERSION.tar.gz /tmp/rpmbuild/SOURCES/
    cp /input/legion-control-$VERSION.tar.gz /tmp/source.tar.gz
    tar -xzf /tmp/source.tar.gz -C /tmp
    cp /tmp/legion-control-$VERSION/packaging/rpm/legion-control.spec /tmp/rpmbuild/SPECS/
    rpmbuild -bb /tmp/rpmbuild/SPECS/legion-control.spec --define '_topdir /tmp/rpmbuild'
    cp /tmp/rpmbuild/RPMS/*/*.rpm /out/
  "

printf '\n==> Building Arch/CachyOS package\n'
docker run --rm \
  -v "$SOURCE:/input/legion-control-$VERSION.tar.gz:ro" \
  -v "$ROOT/packaging/arch:/packaging:ro" \
  -v "$OUT:/out" archlinux:latest bash -lc "
    set -e
    pacman -Syu --needed --noconfirm base-devel rust gtk4 libadwaita pkgconf systemd polkit >/dev/null
    useradd -m builder
    mkdir -p /build
    cp /input/legion-control-$VERSION.tar.gz /build/
    cp /packaging/PKGBUILD /packaging/legion-control.install /build/
    chown -R builder:builder /build
    runuser -u builder -- bash -lc 'cd /build && makepkg --noconfirm --cleanbuild'
    cp /build/*.pkg.tar.zst /out/
  "

printf '\nPackages built in %s:\n' "$OUT"
printf '  %s\n' "$OUT"/*.deb "$OUT"/*.rpm "$OUT"/*.pkg.tar.zst
