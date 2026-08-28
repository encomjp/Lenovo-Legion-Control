# Installing Legion Control

This guide covers every installation path implemented by this repository:

- **the portable AppImage — recommended** (works on every x86_64 Linux distribution);
- native Debian, RPM, and Arch packages built from [`packaging/`](../packaging/);
- the source installer, [`install.sh`](../install.sh);
- a manual source installation; and
- the optional KDE Plasma widget and AMD kernel modules.

> **Do not mix installation methods.** The AppImage and source installer stage the daemon under `/usr/local/bin`; native packages install it under `/usr/bin`. Pick one method and stay with it. If you switch, fully remove the previous one first (see [Uninstalling](#uninstalling)).

---

## Recommended: the AppImage

The AppImage is a single executable file. It bundles the settings GUI, CLI, and daemon and needs **no system dependencies** — no GTK/libadwaita install, no package manager, no build tools. It runs on essentially every current x86_64 Linux distribution (Ubuntu, Fedora, Arch/CachyOS, openSUSE, Debian, Mint, NixOS-with-FUSE, …).

### 1. Download and run

Grab `legion-control-<version>-x86_64.AppImage` from the [latest release](https://github.com/encomjp/lenovo-legion-tool/releases/latest), then:

```bash
chmod +x legion-control-0.1.1-x86_64.AppImage
./legion-control-0.1.1-x86_64.AppImage
```

You can run it from anywhere — `~/Downloads`, `~/Applications`, a USB stick. Nothing is installed until you ask for it.

**FUSE note:** AppImages need FUSE 2 at runtime. If your distribution does not ship it, install it once (`sudo apt install libfuse2`, `sudo dnf install fuse`, `sudo pacman -S libfuse2`) or run without FUSE:

```bash
./legion-control-0.1.1-x86_64.AppImage --appimage-extract-and-run
```

### 2. First launch

The welcome window appears once: it explains the anonymous telemetry (on by default, with a privacy-policy link) and offers a **guided setup** that walks through:

1. **Control service** — if the privileged daemon is not running, one click enables it. You get a single PolicyKit password prompt; the app stages `legion-daemon`, the `legion-control.service` unit, the PolicyKit policy, the setup helper, and the bundled `ryzen_smu` source onto the host (under `/usr/local/...`), then enables and starts the service.
2. **Startup & tuning** — optional (both default on): *Launch at login* writes the desktop autostart entry **and enables the systemd unit, so the daemon also starts on boot**, and *Install AMD tuning backend* builds the `ryzen_smu` DKMS module for Curve Optimizer undervolting.
3. **Hardware** — model, machine type, CPU/GPU, fan channels.
4. **Self-check** — read-only health checks.

### 3. What the AppImage does and does not install

Staged on the host by the one-click Enable (only what systemd needs):

- `/usr/local/bin/legion-daemon`
- `/usr/local/libexec/legion-control-setup`
- `/etc/systemd/system/legion-control.service`
- `/usr/share/polkit-1/actions/com.encomjp.legion-control.policy`
- `/usr/local/lib/legion-control/ryzen_smu/`

**Not** installed automatically: the HID udev rule for Spectrum RGB. Without it the daemon cannot open the lighting controller until you approve the rule. Install it permanently from **Settings → Fix → Permanent fix (udev) → Install permanently** (one PolicyKit prompt), then log out/in or run:

```bash
sudo udevadm control --reload-rules
sudo udevadm trigger -s hidraw
```

### 4. Starting on boot

Either accept **Launch at login** in the guided setup, or later toggle **CPU → Tuning → Launch at login**. It adds `~/.config/autostart/com.encomjp.legion-settings.desktop` (app starts hidden to tray) and enables `legion-control.service`, so both the app and the daemon come up on boot.

### 5. Updating

Replace the AppImage file with the newer one. The staged daemon is **not** overwritten automatically (systemd needs stable paths), so refresh it once:

```bash
sudo systemctl disable --now legion-control
sudo rm -f /usr/local/bin/legion-daemon /etc/systemd/system/legion-control.service
```

Then start the new AppImage and click **Enable** again (Settings → Setup, or the banner) — it re-stages the bundled daemon and helper and starts the service. The GUI detects a daemon/GUI version mismatch and tells you when this is needed.

### 6. Uninstalling the AppImage

```bash
sudo systemctl disable --now legion-control
sudo rm -f /usr/local/bin/legion-daemon /usr/local/libexec/legion-control-setup \
  /etc/systemd/system/legion-control.service \
  /usr/share/polkit-1/actions/com.encomjp.legion-control.policy \
  /etc/udev/rules.d/99-legion.rules
sudo rm -rf /usr/local/lib/legion-control
sudo systemctl daemon-reload
rm -rf ~/.config/legion-control          # settings (optional)
rm -f ~/.config/autostart/com.encomjp.legion-settings.desktop  # autostart (optional)
```

Then delete the AppImage file itself.

---

## Native packages

Native packages install the binaries under `/usr/bin`, the service under the distribution's systemd unit directory, udev rules under `/usr/lib/udev/rules.d/`, the PolicyKit helper, the desktop entry, icons, and the bundled `ryzen_smu` source. They enable/start `legion-control.service` and reload/trigger udev during installation. They do not install or load the optional DKMS modules automatically.

Build all formats from the repository root:

```bash
./packaging/build-all.sh
```

This removes old package artifacts from `packaging/out/`, creates a source archive, and builds in Docker containers. The outputs are written to [`packaging/out/`](../packaging/out/):

- `legion-control_<version>_amd64.deb`;
- `legion-control-<version>-1.<dist>.x86_64.rpm`; and
- `legion-control-<version>-1-x86_64.pkg.tar.zst`.

Install the package that matches the host distribution:

```bash
# Debian/Ubuntu
sudo apt install ./packaging/out/legion-control_*_amd64.deb

# Fedora/RPM-based systems
sudo dnf install ./packaging/out/legion-control-*.x86_64.rpm

# Arch/CachyOS
sudo pacman -U ./packaging/out/legion-control-*-x86_64.pkg.tar.zst
```

The native package service uses [`packaging/common/legion-control.service`](../packaging/common/legion-control.service), whose daemon path is `/usr/bin/legion-daemon`.

---

## Source installer

From the repository root:

```bash
./install.sh
```

The default flow installs dependencies, builds release binaries with Cargo, installs `legion-cli`, `legion-daemon`, and `legion-settings`, installs the PolicyKit setup helper and bundled `third_party/ryzen_smu` source, installs the desktop entry and icons, installs HID udev rules, and enables the root `legion-control` systemd service.

`legion-daemon` is a system-service binary, not a user-facing command. It provides the privileged hardware operations; the CLI and GTK settings application communicate with it. The installer uses [`data/systemd/legion-control.system.service`](../data/systemd/legion-control.system.service), whose `ExecStart` is `/usr/local/bin/legion-daemon`.

### Supported distributions and prerequisites

The current GTK build requires all of the following minimum versions:

- Rust and Cargo `1.87.0` or newer;
- GTK 4 `4.14` or newer;
- libadwaita `1.5` or newer; and
- the `libudev` development files.

A C toolchain and `pkg-config` are also required. The installer checks the GTK, libadwaita, and `libudev` development interfaces with `pkg-config`, and checks Rust with `rustc --version`.

The repository's documented target families are:

- Ubuntu 24.04 or newer;
- Fedora 40 or newer;
- Arch-family distributions, including CachyOS, Arch, EndeavourOS, and Manjaro; and
- openSUSE Tumbleweed when using the installer's `zypper` dependency path.

Ubuntu 22.04 does not provide the required GTK/libadwaita versions for this build. Debian 12 (Bookworm) is not a supported baseline for the current GUI build either: `install.sh` can offer to rewrite apt sources from Bookworm to Trixie to obtain the required libraries, but this is a system-source change, not native Debian 12 support. If accepted, the installer creates `.bak.bookworm` backups for sources it changes and installs the required development packages from Trixie. Review that behavior before accepting it.

(All of these requirements are already satisfied inside the AppImage — one more reason it is the recommended path.)

### Installer flags

Run `./install.sh --help` for the built-in list. The implemented flags are:

| Flag | Effect |
| --- | --- |
| `-h`, `--help` | Print installer help and exit. |
| `-y`, `--yes` | Do not ask before installing packages. This is required for package installation from a non-interactive shell. It also accepts automatic Rust setup and the installer's Debian source upgrade prompt. |
| `--deps-only` | Install/check build dependencies, then exit before building and installing. |
| `--user` | Install the CLI and GUI under `~/.local/bin`; the daemon is still installed system-wide when enabled. The installer also ensures a daemon copy at `/usr/local/bin/legion-daemon`. |
| `--prefix DIR` | Install the CLI, daemon, and GUI under `DIR/bin`. If `DIR` is not `~/.local`, the installer additionally copies the CLI and GUI to `~/.local/bin` for the user. This does not make the complete installation user-local: the daemon service still uses `/usr/local/bin/legion-daemon`, and PolicyKit files remain system-wide. |
| `--no-deps` | Skip package-manager installation, but still check native libraries and Rust. |
| `--no-daemon` | Do not install, enable, or start the systemd daemon. |
| `--no-udev` | Do not install or reload the HID udev rules. |
| `--with-dkms` | Attempt to build and install the optional `legion_hwmon` DKMS module from [`driver/`](../driver/). Missing DKMS or the driver directory causes this optional step to be skipped. |
| `--with-ryzen-smu` | Install and load the bundled AMD `ryzen_smu` DKMS driver through the PolicyKit setup helper. DKMS is required. |
| `--widget` | Install or update the KDE Plasma 6 widget for the desktop user. Plasma is not restarted. |
| `--skip-build` | Skip `cargo build --release` and use existing executable files under `target/release/`. The installer still requires `legion-cli`, `legion-daemon`, `legion-settings`, and `legion-control-setup` to exist and be executable. |

Useful combinations include:

```bash
# Install dependencies only.
./install.sh --deps-only

# Install the GUI and CLI for the current user, while keeping the daemon system-wide.
./install.sh --user

# Install without the daemon or udev rules.
./install.sh --no-daemon --no-udev

# Install the KDE widget as part of the source installation.
./install.sh --widget

# Use already-built release binaries.
./install.sh --skip-build
```

When using `--user`, ensure `~/.local/bin` is on `PATH` if the installer prints that warning:

```bash
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc
```

### Dependency packages installed by `install.sh`

The installer detects package managers in this order: `pacman`, `apt-get`, `dnf`, then `zypper`.

For the supported package families, its package lists are:

```bash
# Arch, CachyOS, EndeavourOS, or Manjaro
sudo pacman -Syu --needed base-devel rust gtk4 libadwaita pkgconf hidapi systemd

# Ubuntu or Debian-family systems
sudo apt-get update -y
sudo apt-get install -y build-essential curl pkg-config libgtk-4-dev libadwaita-1-dev libglib2.0-dev libudev-dev

# Fedora or Nobara
sudo dnf install -y gcc gcc-c++ make curl pkgconf-pkg-config gtk4-devel libadwaita-devel glib2-devel systemd-devel

# openSUSE path implemented by install.sh
sudo zypper install -y gcc gcc-c++ make curl pkgconf gtk4-devel libadwaita-devel
```

These are the commands implemented by the installer; on a non-interactive invocation, use `-y` with the installer so it is allowed to run its package-manager commands. On an unknown package manager, the installer does not guess package names: install GTK 4, libadwaita, `pkg-config`, `libudev` development files, and a C toolchain yourself, then use `--no-deps`.

If Rust is missing or older than `1.87.0`, the installer offers to install or update stable Rust with rustup. The relevant commands are:

```bash
rustup update stable
rustup default stable
```

If `rustup` is not installed, the installer uses:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
```

Reload the shell environment or source `$HOME/.cargo/env` before running Cargo yourself.

## Manual source installation

Use this path when you need to control each installed file yourself. It is the same system-service layout used by the source installer, so do not use it alongside a native package.

Install the development dependencies for your distribution, then ensure Rust `1.87.0` or newer is available. Build from the repository root:

```bash
cargo build --release
```

Install the three user-facing binaries, the source-install service, and the udev rules:

```bash
sudo cp target/release/legion-daemon /usr/local/bin/
sudo cp target/release/legion-cli /usr/local/bin/
sudo cp target/release/legion-settings /usr/local/bin/

sudo cp data/systemd/legion-control.system.service /etc/systemd/system/legion-control.service
sudo systemctl daemon-reload
sudo systemctl enable --now legion-control

sudo cp data/udev/99-legion.rules /etc/udev/rules.d/99-legion.rules
sudo udevadm control --reload-rules
sudo udevadm trigger
```

The manual sequence above does not install the PolicyKit setup helper, desktop entry, icons, or bundled `ryzen_smu` source. Those are installed by `install.sh`; copy them only if you also reproduce the corresponding paths and PolicyKit configuration from [`install.sh`](../install.sh).

## KDE Plasma widget

The widget requires KDE Plasma 6, `kpackagetool6`, an installed `legion-cli`, and an active `legion-control` daemon. Its scripts search `/usr/local/bin/legion-cli`, `/usr/bin/legion-cli`, and `~/.local/bin/legion-cli`; if you used a different prefix, ensure the installer also placed the user CLI copy in `~/.local/bin` or otherwise use one of those supported paths.

Install it for the current user:

```bash
cd kde-widget
chmod +x install.sh
./install.sh
```

From the repository root, the underlying package command is:

```bash
kpackagetool6 --type Plasma/Applet -i kde-widget/package
```

If the install operation fails (for example, because the package already exists), [`kde-widget/install.sh`](../kde-widget/install.sh) attempts an update with `-u`. No Plasma restart is performed. Add the widget from Plasma's widget picker: right-click the desktop, choose **Add Widgets**, and search for **Legion Control**. You can also install it from the app: **Settings → Setup → KDE Plasma widget → Install widget**.

Remove the per-user widget with:

```bash
./kde-widget/uninstall.sh
```

## Optional DKMS and `ryzen_smu`

### `legion_hwmon` DKMS module

`./install.sh --with-dkms` attempts to install the optional `legion_hwmon` module from [`driver/`](../driver/). The installer stages the driver under `/usr/src/legion-hwmon-0.1` and invokes DKMS add/build/install for `legion-hwmon/0.1`, then attempts `modprobe legion_hwmon`. The source tree and DKMS metadata should be treated as the authority for the effective module/version naming; the installer does not validate every DKMS result, and failures are generally optional warnings.

The module needs DKMS, a compiler, and headers matching the running kernel. The native packages only advertise DKMS and headers as optional dependencies; they do not install this module automatically.

### AMD Curve Optimizer backend

`ryzen_smu` is bundled as source but is opt-in. The easiest way to install it is **Settings → Setup → AMD tuning backend → Install**, or accept *Install AMD tuning backend* in the first-launch guided setup — both run the bundled pinned source through DKMS via the PolicyKit helper.

From the source installer:

```bash
./install.sh --with-ryzen-smu
```

The installer requires `dkms`, then calls the installed PolicyKit setup helper. The helper registers/builds/installs bundled version `0.1.7`, creates `/etc/modules-load.d/ryzen_smu.conf`, loads `ryzen_smu`, checks `/sys/kernel/ryzen_smu_drv`, and runs `systemctl try-restart`; this may leave an inactive daemon stopped. Secure Boot or missing running-kernel headers can prevent the sysfs interface from appearing.

For a standalone upstream-style installation, [`third_party/ryzen_smu/README.md`](../third_party/ryzen_smu/README.md) gives these prerequisites and commands:

```bash
# Ubuntu/Debian
sudo apt install dkms git build-essential linux-headers-$(uname -r)
git clone https://github.com/amkillam/ryzen_smu.git
cd ryzen_smu
sudo make dkms-install

# Arch, using an AUR helper
  yay -S ryzen_smu-dkms-git
```

The repository also supports a manual module build:

```bash
git clone https://github.com/amkillam/ryzen_smu.git
cd ryzen_smu
make
sudo insmod ryzen_smu.ko
```

The driver documentation warns that misuse can damage hardware. Treat Curve Optimizer and SMU writes as experimental and use them only when you understand the risk. After loading the module, verify it with:

```bash
dmesg
ls -lah /sys/kernel/ryzen_smu_drv
cat /sys/kernel/ryzen_smu_drv/version
cat /sys/kernel/ryzen_smu_drv/mp1_if_version
cat /sys/kernel/ryzen_smu_drv/codename
cat /sys/kernel/ryzen_smu_drv/drv_version
```

The setup helper removes the bundled module with `remove-ryzen-smu`; its underlying cleanup runs `dkms remove ryzen_smu/0.1.7 --all` and removes `/usr/src/ryzen_smu-0.1.7` and `/etc/modules-load.d/ryzen_smu.conf`. This is separate from uninstalling the main Legion Control package.

## Upgrades and rebuilding

### Native package upgrades

Use the normal package-manager upgrade operation for the installed package. The package lifecycle scripts reload systemd and udev and preserve the running service behavior:

- Debian's [`packaging/debian/postinst`](../packaging/debian/postinst) daemon-reloads, enables, and restarts or starts the service, then reloads/triggers udev.
- RPM `%post` enables/starts the service and reloads/triggers udev; `%postun` uses systemd's restart-aware post-uninstall handling.
- Arch's [`packaging/arch/legion-control.install`](../packaging/arch/legion-control.install) daemon-reloads, tries to restart the service, and reloads/triggers udev in `post_upgrade`.

To rebuild native packages from the current checkout:

```bash
./packaging/build-all.sh
```

### AppImage updates

See [Updating](#5-updating) in the AppImage section: replace the file, refresh the staged daemon, click Enable again.

### Source rebuilds

After rebuilding manually, replace the binaries in the prefix used by the installation. For the default `/usr/local` system installation:

```bash
sudo systemctl stop legion-control
sudo cp target/release/legion-daemon /usr/local/bin/legion-daemon
sudo cp target/release/legion-cli /usr/local/bin/legion-cli
sudo cp target/release/legion-settings /usr/local/bin/legion-settings
sudo systemctl start legion-control
```

For `--user`, replace the CLI and GUI in `~/.local/bin`; the system service still uses `/usr/local/bin/legion-daemon`, so refresh that daemon copy as well when it changed. For `--prefix DIR`, replace the binaries in `DIR/bin` and, because the installer also maintains user CLI/GUI copies when `DIR` is not `~/.local`, update those `~/.local/bin` copies too. Native package installs should be rebuilt and upgraded through the package workflow instead. If you changed the service or udev rules, repeat the corresponding reload commands from the manual installation section.

## Uninstalling

### Native packages

Remove the installed package with the package manager that installed it. The repository's removal scripts stop and disable `legion-control.service` and reload systemd metadata:

```bash
# Debian/Ubuntu
sudo apt remove legion-control

# Fedora/RPM-based systems
sudo dnf remove legion-control

# Arch/CachyOS
sudo pacman -R legion-control
```

Package removal does **not** document deletion of per-user settings, logs, the Plasma widget, or optional DKMS state. Remove those separately only if desired. Remove the widget with `./kde-widget/uninstall.sh` (as the desktop user), and remove the optional `ryzen_smu` module with the setup helper's `remove-ryzen-smu` operation when it was installed by that helper.

### AppImage

See [Uninstalling the AppImage](#6-uninstalling-the-appimage) for the exact staged-file list and removal commands.

### Source or manual installs

There is no source-installer uninstall flag or repository uninstall script. To remove a source/manual installation, first stop and disable the service if it was enabled, then remove the files belonging to the prefix used by that installation. For the standard `/usr/local` layout:

```bash
sudo systemctl disable --now legion-control
sudo rm -f /etc/systemd/system/legion-control.service
sudo systemctl daemon-reload

sudo rm -f /usr/local/bin/legion-cli
sudo rm -f /usr/local/bin/legion-daemon
sudo rm -f /usr/local/bin/legion-settings
sudo rm -f /etc/udev/rules.d/99-legion.rules
sudo udevadm control --reload-rules
sudo udevadm trigger
```

The full source-installer layout additionally includes the PolicyKit helper and policy, desktop entry/icons, and bundled source under `/usr/local/lib/legion-control/ryzen_smu`. The installer does not provide a removal routine for these paths, so inspect the paths in [`install.sh`](../install.sh) and remove only files belonging to this installation. Do not remove shared directories or user configuration you intend to keep.

For `--user`, remove the `~/.local/bin` CLI/GUI copies and the user desktop files. For a non-system `--prefix DIR`, remove `DIR/bin/legion-cli`, `DIR/bin/legion-daemon`, and `DIR/bin/legion-settings`, plus the additional `~/.local/bin` CLI/GUI copies and user desktop files created by the installer. The installer may also have created `/usr/local/bin/legion-daemon` for the system service; remove that only after the service has been stopped and no other source installation depends on it. For a system prefix such as `/usr`, remove that prefix's binaries instead of the `/usr/local` paths shown above.

## Verification and troubleshooting

After installation, verify the service, CLI, and GUI:

```bash
systemctl status legion-control
legion-cli status
legion-settings
```

The installer checks whether the service is active and prints a quick RGB test command; run the following commands to verify the installation:

```bash
legion-cli effect static 200 16 46 --zone keyboard
legion-cli brightness 7
```

RGB HID access comes from [`data/udev/99-legion.rules`](../data/udev/99-legion.rules). On a first installation, log out and in or reboot if the HID device was previously permission-denied. To reload the rules without rebooting:

```bash
sudo udevadm control --reload-rules
sudo udevadm trigger
```

For service failures, inspect the journal and restart the service:

```bash
sudo systemctl restart legion-control
journalctl -u legion-control -f
legion-cli logs 50
legion-cli set-log-level debug
```

If the daemon is running but a widget reports no data, confirm both prerequisites:

```bash
systemctl is-active legion-control
command -v legion-cli
```

For `ryzen_smu`, confirm the kernel interface exists before attempting Curve Optimizer operations:

```bash
ls -lah /sys/kernel/ryzen_smu_drv
cat /sys/kernel/ryzen_smu_drv/drv_version
```

The repository does not claim that every Lenovo Legion model or every optional kernel backend is supported. If a module, HID rule, or device interface is absent, check the service journal and the relevant source/backend documentation rather than forcing writes.

## Repository paths referenced by this guide

- [`packaging/appimage/AppRun`](../packaging/appimage/AppRun) and [`packaging/build-appimage.sh`](../packaging/build-appimage.sh) — the recommended AppImage.
- [`install.sh`](../install.sh) — source installer and flags.
- [`packaging/README.md`](../packaging/README.md) — native-package policy.
- [`packaging/build-all.sh`](../packaging/build-all.sh) — containerized package build.
- [`packaging/debian/`](../packaging/debian/) — Debian package and lifecycle scripts.
- [`packaging/rpm/legion-control.spec`](../packaging/rpm/legion-control.spec) — RPM build and lifecycle hooks.
- [`packaging/arch/PKGBUILD`](../packaging/arch/PKGBUILD) and [`packaging/arch/legion-control.install`](../packaging/arch/legion-control.install) — Arch package and hooks.
- [`packaging/common/legion-control.service`](../packaging/common/legion-control.service) — native-package service unit.
- [`data/systemd/legion-control.system.service`](../data/systemd/legion-control.system.service) — source-install service unit.
- [`kde-widget/README.md`](../kde-widget/README.md) — widget requirements and user installation.
- [`third_party/ryzen_smu/README.md`](../third_party/ryzen_smu/README.md) — upstream driver requirements and verification.
- [`driver/`](../driver/) — optional `legion_hwmon` DKMS source.

The AppImage stages its daemon until you remove it, the source installer has no built-in uninstall mode, and package removal does not clean every user or optional-backend artifact. Those are the main lifecycle limitations documented by the repository.