#!/usr/bin/env bash
# Legion Control — friendly installer for CachyOS / Arch, Ubuntu / Debian, Fedora.
#
# Usage:
#   ./install.sh              # deps + build + install + daemon
#   ./install.sh --help
#   ./install.sh --deps-only
#   ./install.sh --user       # install binaries to ~/.local (still needs sudo for daemon/udev)
#
set -euo pipefail

PREFIX_DEFAULT="/usr/local"
USER_PREFIX="${HOME}/.local"
PREFIX="$PREFIX_DEFAULT"
DO_DEPS=1
DO_BUILD=1
DO_INSTALL=1
DO_DAEMON=1
DO_UDEV=1
DO_DESKTOP=1
DO_DKMS=0
DO_RYZEN_SMU=0
DO_WIDGET=0
ASSUME_YES=0
DEPS_ONLY=0

# ─── colours ────────────────────────────────────────────────────────────────
if [[ -t 1 ]]; then
  C_RED=$'\033[1;31m'; C_GRN=$'\033[1;32m'; C_YEL=$'\033[1;33m'
  C_BLU=$'\033[1;34m'; C_CYN=$'\033[1;36m'; C_DIM=$'\033[2m'
  C_RST=$'\033[0m'; C_BLD=$'\033[1m'
else
  C_RED=; C_GRN=; C_YEL=; C_BLU=; C_CYN=; C_DIM=; C_RST=; C_BLD=
fi

say()  { printf '%s\n' "$*"; }
info() { printf '%s→%s %s\n' "$C_CYN" "$C_RST" "$*"; }
ok()   { printf '%s✓%s %s\n' "$C_GRN" "$C_RST" "$*"; }
warn() { printf '%s!%s %s\n' "$C_YEL" "$C_RST" "$*"; }
die()  { printf '%s✗%s %s\n' "$C_RED" "$C_RST" "$*" >&2; exit 1; }
banner() {
  printf '\n%s╔══════════════════════════════════════════════╗%s\n' "$C_BLD$C_BLU" "$C_RST"
  printf '%s║%s  Legion Control installer                    %s║%s\n' "$C_BLD$C_BLU" "$C_RST" "$C_BLD$C_BLU" "$C_RST"
  printf '%s║%s  Fans · profiles · Spectrum RGB · battery   %s║%s\n' "$C_BLD$C_BLU" "$C_RST" "$C_BLD$C_BLU" "$C_RST"
  printf '%s╚══════════════════════════════════════════════╝%s\n\n' "$C_BLD$C_BLU" "$C_RST"
}

usage() {
  cat <<'EOF'
Legion Control installer

Usage: ./install.sh [options]

Options:
  -h, --help         Show this help
  -y, --yes          Don't ask before installing packages (apt/dnf/pacman)
  --deps-only        Only install build dependencies, then exit
  --user             Install CLI/GUI to ~/.local/bin (daemon remains system-wide)
  --prefix DIR       Install prefix (default: /usr/local)
  --no-deps          Skip dependency installation
  --no-daemon        Don't install/enable the systemd daemon
  --no-udev          Don't install HID udev rules
  --with-dkms        Also try legion_hwmon DKMS (optional EC temps)
  --with-ryzen-smu    Install the bundled AMD Curve Optimizer DKMS driver (Ryzen 9000 / Granite Ridge)
  --widget           Install/update the KDE Plasma 6 widget for this user
  --skip-build       Skip cargo build (use existing target/release)

What gets installed:
  legion-cli, legion-daemon, legion-settings
  systemd service: legion-control  (root — fans / profile / charge limit)
  udev rules for Spectrum RGB (048d:c197)
  optional KDE Plasma 6 widget (no Plasma restart)

Supported package managers:
  pacman   — CachyOS, Arch, EndeavourOS, Manjaro
  apt      — Ubuntu, Debian, Pop!_OS, Linux Mint
  dnf      — Fedora, Nobara
EOF
}

# ─── args ───────────────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help) usage; exit 0 ;;
    -y|--yes) ASSUME_YES=1; shift ;;
    --deps-only) DEPS_ONLY=1; shift ;;
    --user) PREFIX="$USER_PREFIX"; shift ;;
    --prefix) PREFIX="${2:?}"; shift 2 ;;
    --no-deps) DO_DEPS=0; shift ;;
    --no-daemon) DO_DAEMON=0; shift ;;
    --no-udev) DO_UDEV=0; shift ;;
    --with-dkms) DO_DKMS=1; shift ;;
    --with-ryzen-smu) DO_RYZEN_SMU=1; shift ;;
    --widget) DO_WIDGET=1; shift ;;
    --skip-build) DO_BUILD=0; shift ;;
    *) die "Unknown option: $1 (try --help)" ;;
  esac
done

# ─── find project root (Cargo.toml) ─────────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ -f "$SCRIPT_DIR/Cargo.toml" ]]; then
  ROOT="$SCRIPT_DIR"
elif [[ -f "$SCRIPT_DIR/lenovo-legion-tool/Cargo.toml" ]]; then
  ROOT="$SCRIPT_DIR/lenovo-legion-tool"
elif [[ -f "$SCRIPT_DIR/../Cargo.toml" ]]; then
  ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
else
  die "Cannot find Cargo.toml. Run from the Legion Control repo."
fi
cd "$ROOT"
ok "Project: $ROOT"

need_cmd() { command -v "$1" >/dev/null 2>&1; }

sudo_run() {
  if [[ "$(id -u)" -eq 0 ]]; then
    "$@"
  elif need_cmd sudo; then
    sudo "$@"
  else
    die "Need root for: $* (install sudo or re-run as root)"
  fi
}

confirm() {
  local msg="$1"
  if [[ "$ASSUME_YES" -eq 1 ]]; then
    return 0
  fi
  if [[ ! -t 0 ]]; then
    warn "Non-interactive shell — pass -y to allow package installs"
    return 1
  fi
  printf '%s?%s %s [Y/n] ' "$C_YEL" "$C_RST" "$msg"
  read -r ans || true
  [[ -z "$ans" || "$ans" =~ ^[Yy] ]]
}

# ─── detect distro family ───────────────────────────────────────────────────
detect_pm() {
  if need_cmd pacman; then echo pacman
  elif need_cmd apt-get; then echo apt
  elif need_cmd dnf; then echo dnf
  elif need_cmd zypper; then echo zypper
  else echo unknown
  fi
}

pretty_distro() {
  if [[ -f /etc/os-release ]]; then
    # shellcheck disable=SC1091
    . /etc/os-release
    echo "${PRETTY_NAME:-$NAME}"
  else
    echo "Linux"
  fi
}

PM="$(detect_pm)"
DISTRO="$(pretty_distro)"
banner
info "Detected: ${C_BLD}${DISTRO}${C_RST}  (${PM})"
info "Install prefix: ${C_BLD}${PREFIX}${C_RST}"

# ─── dependencies ───────────────────────────────────────────────────────────
install_deps_pacman() {
  local pkgs=(
    base-devel rust
    gtk4 libadwaita pkgconf
    hidapi systemd
  )
  info "Arch / CachyOS packages: ${pkgs[*]}"
  if confirm "Synchronize packages and install dependencies with pacman?"; then
    # Arch-based systems do not support partial upgrades. -Syu also makes a
    # fresh installation work when the package databases have not been synced.
    sudo_run pacman -Syu --needed --noconfirm "${pkgs[@]}"
  else
    warn "Skipped package install — build may fail without deps"
  fi
}

install_deps_apt() {
  local pkgs=(
    build-essential curl pkg-config
    libgtk-4-dev libadwaita-1-dev
    libglib2.0-dev libudev-dev
  )
  info "Ubuntu / Debian packages: ${pkgs[*]}"
  if confirm "Install packages with apt?"; then
    sudo_run apt-get update -y
    sudo_run apt-get install -y "${pkgs[@]}"
  else
    warn "Skipped package install — build may fail without deps"
  fi
}

install_deps_dnf() {
  local pkgs=(
    gcc gcc-c++ make curl pkgconf-pkg-config
    gtk4-devel libadwaita-devel
    glib2-devel systemd-devel
  )
  info "Fedora packages: ${pkgs[*]}"
  if confirm "Install packages with dnf?"; then
    sudo_run dnf install -y "${pkgs[@]}"
  else
    warn "Skipped package install — build may fail without deps"
  fi
}

install_deps_zypper() {
  local pkgs=(
    gcc gcc-c++ make curl pkgconf
    gtk4-devel libadwaita-devel systemd-devel
  )
  info "openSUSE packages: ${pkgs[*]}"
  if confirm "Install packages with zypper?"; then
    sudo_run zypper install -y "${pkgs[@]}"
  else
    warn "Skipped package install — build may fail without deps"
  fi
}

ensure_rust() {
  local min_rust="1.87.0"
  local need_install=0
  if need_cmd cargo && need_cmd rustc; then
    local current
    current="$(rustc --version | awk '{print $2}')"
    if [[ "$(printf '%s\n%s\n' "$min_rust" "$current" | sort -V | head -n1)" == "$min_rust" ]]; then
      ok "Rust $current"
      return 0
    fi
    warn "System Rust $current is too old — requires $min_rust+"
    need_install=1
  else
    warn "Rust / cargo not found"
    need_install=1
  fi
  # In non-interactive mode or when the user confirms, install via rustup.
  if [[ "$ASSUME_YES" -eq 1 ]] || confirm "Install/update stable Rust via rustup (https://rustup.rs)?"; then
    if need_cmd rustup; then
      rustup update stable
      rustup default stable
    else
      curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    fi
    # shellcheck disable=SC1091
    source "$HOME/.cargo/env"
    export PATH="$HOME/.cargo/bin:$PATH"
    local installed
    installed="$(rustc --version | awk '{print $2}')"
    ok "Rust $installed (via rustup)"
  else
    die "Rust $min_rust+ is required. Update Rust and re-run."
  fi
}

# ─── Debian Bookworm → Trixie auto-upgrade ─────────────────────────────────
upgrade_debian_to_trixie() {
  info "This will change Debian sources from bookworm → trixie"
  info "and install GTK 4.14+ / libadwaita 1.5+ from the Trixie repos."
  warn "A full system upgrade to Trixie is recommended for production use."
  warn "This installer only upgrades the libraries needed to build Legion Control."

  if ! confirm "Upgrade Debian sources to Trixie now?"; then
    die "Debian Bookworm GTK 4.8 / libadwaita 1.2 cannot build Legion Control.
  Upgrade manually:
    sudo sed -i 's/bookworm/trixie/g' /etc/apt/sources.list
    sudo apt update && sudo apt install libgtk-4-dev libadwaita-1-dev
  Or do a clean install of Debian Trixie."
  fi

  local upgraded=0

  # Handle legacy sources.list
  if [[ -f /etc/apt/sources.list ]]; then
    sudo_run cp /etc/apt/sources.list /etc/apt/sources.list.bak.bookworm
    sudo_run sed -i 's/bookworm/trixie/g' /etc/apt/sources.list
    ok "Updated /etc/apt/sources.list: bookworm → trixie"
    upgraded=1
  fi

  # Handle DEB822 .sources files and legacy .list files in sources.list.d
  if [[ -d /etc/apt/sources.list.d ]]; then
    for f in /etc/apt/sources.list.d/*.sources /etc/apt/sources.list.d/*.list; do
      [[ -f "$f" ]] || continue
      if grep -q 'bookworm' "$f" 2>/dev/null; then
        sudo_run cp "$f" "${f}.bak.bookworm"
        sudo_run sed -i 's/bookworm/trixie/g' "$f"
        info "Updated $(basename "$f"): bookworm → trixie"
        upgraded=1
      fi
    done
  fi

  if [[ "$upgraded" -eq 0 ]]; then
    die "No Debian apt sources found containing 'bookworm'. Cannot auto-upgrade."
  fi

  sudo_run apt-get update -qq
  ok "apt-get update done"

  sudo_run apt-get install -y -qq libgtk-4-dev libadwaita-1-dev libglib2.0-dev libudev-dev
  ok "Installed libgtk-4-dev + libadwaita-1-dev + deps from Trixie"
}

check_native_deps() {
  need_cmd pkg-config || die "pkg-config is required"

  # Detect specific distro for targeted upgrade advice.
  local distro_id=""
  if [[ -f /etc/os-release ]]; then
    # shellcheck disable=SC1091
    . /etc/os-release
    distro_id="${ID:-}"
  fi

  if ! pkg-config --atleast-version=4.14 gtk4 2>/dev/null; then
    local have
    have="$(pkg-config --modversion gtk4 2>/dev/null || echo "not found")"
    case "$distro_id" in
      debian)
        warn "GTK $have found, but 4.14+ is required (Debian Bookworm ships GTK 4.8)."
        upgrade_debian_to_trixie
        # Re-check after upgrade
        if ! pkg-config --atleast-version=4.14 gtk4 2>/dev/null; then
          die "GTK upgrade failed — $(pkg-config --modversion gtk4 2>/dev/null || echo "not found"). Check /etc/apt/sources.list."
        fi
        ;;
      ubuntu)
        die "GTK $have found, but 4.14+ is required.
  Ubuntu 22.04 ships GTK 4.6 — upgrade to Ubuntu 24.04 LTS (Noble) or newer."
        ;;
      *)
        die "GTK 4.14+ is required (found $have). Supported: Ubuntu 24.04+, Fedora 40+, Arch, openSUSE Tumbleweed."
        ;;
    esac
  fi

  if ! pkg-config --atleast-version=1.5 libadwaita-1 2>/dev/null; then
    local have
    have="$(pkg-config --modversion libadwaita-1 2>/dev/null || echo "not found")"
    case "$distro_id" in
      debian)
        # Already handled by upgrade_debian_to_trixie above (installs both)
        if ! pkg-config --atleast-version=1.5 libadwaita-1 2>/dev/null; then
          die "libadwaita $have found, but 1.5+ is required.
  Debian Trixie upgrade may have failed — check: apt-get install -t trixie libadwaita-1-dev"
        fi
        ;;
      ubuntu)
        die "libadwaita $have found, but 1.5+ is required.
  Ubuntu 22.04 ships libadwaita 1.1 — upgrade to Ubuntu 24.04 LTS or newer."
        ;;
      *)
        die "libadwaita 1.5+ is required (found $have). Supported: Ubuntu 24.04+, Fedora 40+, Arch, openSUSE Tumbleweed."
        ;;
    esac
  fi

  # Nothing currently links libudev (hidapi uses the linux-native backend);
  # missing -dev files only matter if that changes, so warn instead of dying.
  pkg-config --exists libudev \
    || warn "libudev development files not found — fine unless a future build links udev"
  ok "Native libraries: GTK $(pkg-config --modversion gtk4), libadwaita $(pkg-config --modversion libadwaita-1)"
}

if [[ "$DO_DEPS" -eq 1 ]]; then
  say ""
  info "Checking build dependencies…"
  case "$PM" in
    pacman) install_deps_pacman ;;
    apt)    install_deps_apt ;;
    dnf)    install_deps_dnf ;;
    zypper) install_deps_zypper ;;
    *)
      warn "Unknown package manager — install GTK4, libadwaita, pkg-config, and a C toolchain yourself"
      ;;
  esac
  check_native_deps
  ensure_rust
else
  check_native_deps
  ensure_rust
fi

if [[ "$DEPS_ONLY" -eq 1 ]]; then
  ok "Dependencies ready. Re-run without --deps-only to build & install."
  exit 0
fi

# ─── build ──────────────────────────────────────────────────────────────────
BIN_CLI="$ROOT/target/release/legion-cli"
BIN_DAEMON="$ROOT/target/release/legion-daemon"
BIN_GUI="$ROOT/target/release/legion-settings"
BIN_SETUP="$ROOT/target/release/legion-control-setup"

if [[ "$DO_BUILD" -eq 1 ]]; then
  say ""
  info "Building release binaries (this can take a few minutes)…"
  # Prefer cargo from PATH (rustup or distro)
  cargo build --release
  ok "Build finished"
else
  info "Skipping build (--skip-build)"
fi

[[ -x "$BIN_CLI" && -x "$BIN_DAEMON" && -x "$BIN_GUI" && -x "$BIN_SETUP" ]] \
  || die "Missing release binaries under target/release/ — build failed?"

# ─── install binaries ───────────────────────────────────────────────────────
say ""
info "Installing binaries to ${PREFIX}/bin …"
mkdir -p "${PREFIX}/bin" 2>/dev/null || true
if [[ "$PREFIX" == /usr* || "$PREFIX" == /usr ]]; then
  sudo_run install -Dm755 "$BIN_CLI"    "${PREFIX}/bin/legion-cli"
  sudo_run install -Dm755 "$BIN_DAEMON" "${PREFIX}/bin/legion-daemon"
  sudo_run install -Dm755 "$BIN_GUI"    "${PREFIX}/bin/legion-settings"
else
  install -Dm755 "$BIN_CLI"    "${PREFIX}/bin/legion-cli"
  install -Dm755 "$BIN_DAEMON" "${PREFIX}/bin/legion-daemon"
  install -Dm755 "$BIN_GUI"    "${PREFIX}/bin/legion-settings"
  # The system unit always uses /usr/local/bin, regardless of the user prefix.
  if [[ "$DO_DAEMON" -eq 1 ]]; then
    info "Installing daemon system-wide to /usr/local/bin"
    sudo_run install -Dm755 "$BIN_DAEMON" /usr/local/bin/legion-daemon
  fi
fi
ok "legion-cli · legion-daemon · legion-settings"

# The GTK setup buttons use a fixed, audited PolicyKit helper. Source installs
# own /usr/local even when the unprivileged GUI/CLI use --user.
SETUP_PREFIX=/usr/local
if [[ "$PREFIX" == /usr ]]; then
  SETUP_PREFIX=/usr
fi
info "Installing PolicyKit setup helper to ${SETUP_PREFIX}/libexec …"
sudo_run install -Dm755 "$BIN_SETUP" "${SETUP_PREFIX}/libexec/legion-control-setup"
sudo_run install -Dm644 "$ROOT/data/polkit/com.encomjp.legion-control.policy" \
  /usr/share/polkit-1/actions/com.encomjp.legion-control.policy
ok "PolicyKit helper installed"

# Keep ~/.local/bin copy for convenience when using --user or as fallback
if [[ "$PREFIX" != "$USER_PREFIX" ]]; then
  mkdir -p "${USER_PREFIX}/bin"
  install -Dm755 "$BIN_CLI"    "${USER_PREFIX}/bin/legion-cli"
  install -Dm755 "$BIN_GUI"    "${USER_PREFIX}/bin/legion-settings"
fi

# ─── udev ───────────────────────────────────────────────────────────────────
if [[ "$DO_UDEV" -eq 1 ]]; then
  say ""
  info "Installing Spectrum RGB udev rules…"
  sudo_run install -Dm644 "$ROOT/data/udev/99-legion.rules" /etc/udev/rules.d/99-legion.rules
  sudo_run udevadm control --reload-rules || true
  sudo_run udevadm trigger -s hidraw || true
  ok "udev rules installed (re-login if RGB was permission-denied before)"
fi

# ─── desktop entry ──────────────────────────────────────────────────────────
if [[ "$DO_DESKTOP" -eq 1 ]]; then
  DESKTOP_SRC="$ROOT/data/gui/com.encomjp.legion-settings.desktop"
  APP_ICON="$ROOT/data/icons/app-mark.svg"
  TRAY_ICON="$ROOT/data/icons/tray.svg"
  if [[ -f "$DESKTOP_SRC" ]]; then
    info "Installing desktop menu entry…"
    if [[ "$PREFIX" == /usr* ]]; then
      sudo_run install -Dm644 "$DESKTOP_SRC" \
        "${PREFIX}/share/applications/com.encomjp.legion-settings.desktop"
      sudo_run install -Dm644 "$APP_ICON" \
        "${PREFIX}/share/icons/hicolor/scalable/apps/com.encomjp.legion-settings.svg"
      sudo_run install -Dm644 "$TRAY_ICON" \
        "${PREFIX}/share/icons/hicolor/scalable/status/com.encomjp.legion-settings-tray.svg"
      if need_cmd gtk-update-icon-cache; then
        sudo_run gtk-update-icon-cache -f -t "${PREFIX}/share/icons/hicolor" || true
      fi
    else
      install -Dm644 "$DESKTOP_SRC" \
        "${HOME}/.local/share/applications/com.encomjp.legion-settings.desktop"
      install -Dm644 "$APP_ICON" \
        "${HOME}/.local/share/icons/hicolor/scalable/apps/com.encomjp.legion-settings.svg"
      install -Dm644 "$TRAY_ICON" \
        "${HOME}/.local/share/icons/hicolor/scalable/status/com.encomjp.legion-settings-tray.svg"
      if need_cmd gtk-update-icon-cache; then
        gtk-update-icon-cache -f -t "${HOME}/.local/share/icons/hicolor" || true
      fi
      # Point Exec at user binary if needed
      if [[ "$PREFIX" == "$USER_PREFIX" ]]; then
        sed -i "s|^Exec=legion-settings|Exec=${USER_PREFIX}/bin/legion-settings|" \
          "${HOME}/.local/share/applications/com.encomjp.legion-settings.desktop" || true
      fi
    fi
    ok "App menu: Legion Control"
  fi
fi

# ─── optional KDE Plasma widget ──────────────────────────────────────────────
if [[ "$DO_WIDGET" -eq 1 ]]; then
  say ""
  info "Installing KDE Plasma widget for the desktop user…"
  need_cmd kpackagetool6 \
    || die "kpackagetool6 is not installed. Install KDE Plasma 6 / KPackage first."

  WIDGET_PACKAGE="$ROOT/kde-widget/package"
  [[ -f "$WIDGET_PACKAGE/metadata.json" ]] || die "Widget package is missing"
  if [[ "$(id -u)" -eq 0 && -n "${SUDO_USER:-}" && "$SUDO_USER" != "root" ]]; then
    WIDGET_HOME="$(getent passwd "$SUDO_USER" | cut -d: -f6)"
    sudo -u "$SUDO_USER" HOME="$WIDGET_HOME" \
      kpackagetool6 --type Plasma/Applet -i "$WIDGET_PACKAGE" 2>/dev/null \
      || sudo -u "$SUDO_USER" HOME="$WIDGET_HOME" \
        kpackagetool6 --type Plasma/Applet -u "$WIDGET_PACKAGE"
  else
    kpackagetool6 --type Plasma/Applet -i "$WIDGET_PACKAGE" 2>/dev/null \
      || kpackagetool6 --type Plasma/Applet -u "$WIDGET_PACKAGE"
  fi
  ok "KDE widget installed — add it from Plasma's widget picker"
fi

# ─── systemd daemon ─────────────────────────────────────────────────────────
if [[ "$DO_DAEMON" -eq 1 ]]; then
  say ""
  info "Installing root daemon (fans / platform profile / charge limit)…"
  systemctl --user disable --now legion-control 2>/dev/null || true
  pkill -x legion-daemon 2>/dev/null || true
  # The daemon's IPC socket is 0660 root:legion — create the group and add
  # the invoking user so CLI/GUI/widget can talk to the daemon.
  sudo_run groupadd -r legion 2>/dev/null || true
  if [[ -n "${SUDO_USER:-}" && "$SUDO_USER" != "root" ]]; then
    if ! id -nG "$SUDO_USER" 2>/dev/null | grep -qw legion; then
      sudo_run usermod -aG legion "$SUDO_USER"
      warn "Added $SUDO_USER to group 'legion' — log out and back in for CLI/GUI access to take effect"
    fi
  fi
  sudo_run install -Dm644 "$ROOT/data/systemd/legion-control.system.service" \
    /etc/systemd/system/legion-control.service
  # Declarative group creation for future systems (groupadd above covers now).
  sudo_run install -Dm644 "$ROOT/data/sysusers.d/legion-control.conf" \
    /usr/lib/sysusers.d/legion-control.conf 2>/dev/null || true
  # Ensure ExecStart=/usr/local/bin/legion-daemon always exists, including
  # --prefix and --user installations.
  if [[ ! -x /usr/local/bin/legion-daemon ]]; then
    sudo_run install -Dm755 "$BIN_DAEMON" /usr/local/bin/legion-daemon
  fi
  sudo_run systemctl daemon-reload
  sudo_run systemctl enable --now legion-control
  sleep 0.4
  if systemctl is-active --quiet legion-control; then
    ok "Daemon running: systemctl status legion-control"
  else
    warn "Daemon may have failed — check: journalctl -u legion-control -e"
  fi
fi

# ─── optional DKMS ──────────────────────────────────────────────────────────
if [[ "$DO_DKMS" -eq 1 ]]; then
  say ""
  info "Trying optional legion_hwmon DKMS…"
  if need_cmd dkms && [[ -d "$ROOT/driver" ]]; then
    sudo_run mkdir -p /usr/src/legion-hwmon-0.1
    sudo_run cp "$ROOT/driver/legion_hwmon.c" "$ROOT/driver/Makefile" "$ROOT/driver/dkms.conf" \
      /usr/src/legion-hwmon-0.1/ 2>/dev/null || true
    sudo_run dkms add legion-hwmon/0.1 2>/dev/null || true
    sudo_run dkms build legion-hwmon/0.1 2>/dev/null || true
    sudo_run dkms install legion-hwmon/0.1 2>/dev/null || true
    sudo_run modprobe legion_hwmon 2>/dev/null || warn "legion_hwmon not loaded (optional)"
  else
    warn "dkms or driver/ missing — skipped"
  fi
fi

# ─── optional ryzen_smu DKMS (AMD Curve Optimizer) ──────────────────────────
if [[ "$DO_RYZEN_SMU" -eq 1 ]]; then
  say ""
  info "Installing bundled ryzen_smu DKMS driver (AMD Curve Optimizer)…"
  if ! need_cmd dkms; then
    die "dkms is required for ryzen_smu. Install dkms and re-run."
  fi
  SMU_SRC="$ROOT/third_party/ryzen_smu"
  if [[ ! -f "$SMU_SRC/Makefile" || ! -f "$SMU_SRC/dkms.conf" ]]; then
    die "Bundled ryzen_smu source is missing under third_party/ryzen_smu/"
  fi
  sudo_run mkdir -p "${SETUP_PREFIX}/lib/legion-control/ryzen_smu"
  sudo_run cp -a "$ROOT/third_party/ryzen_smu/." \
    "${SETUP_PREFIX}/lib/legion-control/ryzen_smu/"
  sudo_run "${SETUP_PREFIX}/libexec/legion-control-setup" install-ryzen-smu
  if [[ -d /sys/kernel/ryzen_smu_drv ]]; then
    ok "ryzen_smu loaded — no tuning value was written"
  else
    warn "ryzen_smu installed but sysfs interface not found — check Secure Boot and kernel headers"
  fi
fi

# ─── optional KDE Plasma 6 widget ──────────────────────────────────────────
if [[ "$DO_WIDGET" -eq 1 ]] || { [[ "$DO_WIDGET" -eq 0 ]] && [[ -n "${KDE_FULL_SESSION:-}" || "${XDG_CURRENT_DESKTOP:-}" == *"KDE"* ]] && need_cmd kpackagetool6; }; then
  WIDGET_DIR="$ROOT/kde-widget"
  if [[ -f "$WIDGET_DIR/package/metadata.json" ]] && need_cmd kpackagetool6; then
    say ""
    info "Installing / updating KDE Plasma 6 widget…"
    if kpackagetool6 --type Plasma/Applet -i "$WIDGET_DIR/package" 2>/dev/null; then
      ok "KDE Plasma 6 widget installed."
    else
      kpackagetool6 --type Plasma/Applet -u "$WIDGET_DIR/package" 2>/dev/null || true
      ok "KDE Plasma 6 widget updated."
    fi
    say "  ${C_DIM}Add 'Legion Control' from Plasma's widget picker.${C_RST}"
  fi
fi

# ─── PATH hint ──────────────────────────────────────────────────────────────
if [[ ":$PATH:" != *":${USER_PREFIX}/bin:"* ]] && [[ "$PREFIX" == "$USER_PREFIX" ]]; then
  warn "Add ${USER_PREFIX}/bin to your PATH, e.g.:"
  say "  echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.bashrc"
fi

# ─── done ───────────────────────────────────────────────────────────────────
say ""
printf '%s══════════════════════════════════════════════%s\n' "$C_GRN" "$C_RST"
ok "Legion Control is installed"
say ""
say "  ${C_BLD}GUI${C_RST}     legion-settings"
say "  ${C_BLD}CLI${C_RST}     legion-cli status"
say "  ${C_BLD}Daemon${C_RST}  systemctl status legion-control"
say ""
say "  ${C_DIM}RGB works without the daemon (HID)."
say "  Fans / power profile / charge limit need the daemon.${C_RST}"
say ""
say "Quick test:"
say "  legion-cli effect static 200 16 46 --zone keyboard"
say "  legion-cli brightness 7"
say ""
