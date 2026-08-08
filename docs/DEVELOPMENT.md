# Development guide

This guide describes the development and release checks for a checkout of the Legion Control repository.

## Toolchain and prerequisites

The Rust package uses edition 2021 and keeps its dependency lockfile in `Cargo.lock`. `Cargo.toml` defines the `legion_core` library and these binaries:

- `legion-cli` from `src/cli/main.rs`
- `legion-daemon` from `src/daemon/main.rs`
- `legion-settings` from `src/settings/main.rs`
- `legion-control-setup` from `src/setup-helper/main.rs`

The installer in `install.sh` requires Rust and Cargo `1.87.0` or newer. The GUI dependencies require GTK 4.14 or newer, libadwaita 1.5 or newer, `libudev`, and `pkg-config`. The installer documents Ubuntu 24.04+, Fedora 40+, Arch-family distributions, and openSUSE Tumbleweed as supported build environments. On openSUSE, the installer uses `zypper` to install `gcc`, `gcc-c++`, `make`, `curl`, `pkgconf`, `gtk4-devel`, and `libadwaita-devel`; Ubuntu 22.04 and Debian Bookworm do not provide the required GUI versions without the upgrade path implemented by `install.sh`.

Check the local toolchain before building:

```bash
rustc --version
cargo --version
pkg-config --modversion gtk4
pkg-config --modversion libadwaita-1
pkg-config --exists libudev
```

The repository does not provide a CI workflow or a project-specific wrapper for these checks. Run the commands below from the repository root.

## Build, format, lint, and test checks

Run the normal Rust checks:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test
cargo clippy --all-targets
cargo build --release
```

Packaging recipes use locked builds. When reproducing a package build locally, use the corresponding locked commands:

```bash
cargo build --release --locked
cargo test --all-targets --locked
```

`cargo test` is not entirely hardware-independent. `src/device.rs` contains `detect_this_machine`, a test that reads the host machine identity, fan capabilities, and GPU information and expects Legion-like hardware. Treat a failure there as an environment or hardware result first; do not weaken the test merely to make an unsupported host pass.

### QML checks

The Plasma widget QML files are under `kde-widget/package`. Validate them with the Qt/KDE modules available on the development machine:

```bash
find kde-widget/package -type f -name '*.qml' -print0 | xargs -0 qmllint
```

The widget imports `QtQuick`, `QtQuick.Controls`, `QtQuick.Layouts`, `QtQuick.Shapes`, `org.kde.plasma.plasmoid`, `org.kde.kirigami`, and `org.kde.plasma.plasma5support`. `qmllint` may therefore report import or type errors when the local Plasma/Kirigami QML module paths are incomplete. There is no repository-specific `qmllint` configuration.

### Shell syntax checks

Check Bash scripts with Bash, not `sh`:

```bash
bash -n install.sh
bash -n kde-widget/install.sh
bash -n kde-widget/uninstall.sh
bash -n kde-widget/package/contents/ui/legion-poll.sh
bash -n kde-widget/package/contents/ui/legion-command.sh
bash -n kde-widget/package/contents/ui/legion-info.sh
bash -n kde-widget/package/contents/ui/legion-settings.sh
bash -n packaging/build-all.sh
bash -n packaging/debian/build.sh
bash -n scripts/enable-root-daemon.sh
```

The Debian maintainer scripts are POSIX shell scripts:

```bash
sh -n packaging/debian/postinst packaging/debian/prerm packaging/debian/postrm
```

The repository has no ShellCheck configuration. ShellCheck is useful when installed, but it is not currently a required repository check.

## Source map

The Rust hardware and application layers are organized as follows:

- `src/lib.rs` registers the public core modules and describes the hardware abstraction.
- `src/comms.rs` defines the Unix-socket protocol and daemon command classification.
- `src/device.rs` detects model, BIOS, capabilities, and the hardware fingerprint.
- `src/sensors.rs` reads hwmon, sysfs, power, and `nvidia-smi` telemetry.
- `src/fans.rs` reads and writes fan state.
- `src/profile.rs` handles ACPI platform profiles and PPT.
- `src/battery.rs` handles charge limits and conservation mode.
- `src/keyboard.rs` implements the Spectrum RGB HID protocol and per-key lighting.
- `src/rgb_panic.rs` contains RGB diagnostics, USB reset, and HID rebind handling.
- `src/audio.rs` contains AW88399 and smart-amp diagnostics.
- `src/cpu.rs` handles CPU boost and SMT controls.
- `src/dgpu.rs` integrates with `nvidia-smi`.
- `src/models.rs` classifies models and GPU/TGP capabilities.
- `src/undervolt.rs` contains optional AMD Curve Optimizer support.
- `src/config.rs` stores application settings and lighting configuration.
- `src/logging.rs` implements the ring buffer, file rotation, and runtime log level.
- `src/cli/main.rs` handles CLI arguments and subcommands.
- `src/daemon/main.rs` implements the root daemon and Unix-socket server.
- `src/settings/main.rs` implements the GTK4/libadwaita settings application.
- `src/settings/lighting.rs` contains the lighting UI.
- `src/settings/perkey.rs` contains the per-key painter.
- `src/settings/queue.rs` contains the UI apply queue.
- `src/settings/tray.rs` contains the system tray.
- `src/settings/widgets.rs` contains shared GTK widgets.
- `src/setup-helper/main.rs` contains the fixed PolicyKit setup helper.

For system integration, use `data/systemd/` for source-install service units, `data/udev/99-legion.rules` for HID access rules, `data/polkit/` for PolicyKit policy, `data/gui/` for the desktop entry, and `data/icons/` for application and tray icons. Optional kernel backends live in `driver/` and `third_party/ryzen_smu/`. `examples/test_async_rgb.rs` is a manual RGB hardware exercise, not an automated test.

When changing the code, add a Rust module to `src/` and register it in `src/lib.rs` when it is part of the core library. Add CLI behavior in `src/cli/main.rs`, settings pages and widgets in `src/settings/`, system integration files in `data/`, and backend changes in the relevant hardware module rather than duplicating sysfs, HID, or daemon protocol logic.

## Packaging

The all-format packaging entry point is:

```bash
./packaging/build-all.sh
```

The script reads the version from `Cargo.toml`, creates a temporary source archive, excludes `target/`, `packaging/out/`, and `.hermes/`, and builds in clean containers:

- Ubuntu 24.04 produces the Debian package.
- Fedora 42 produces the RPM.
- Arch latest produces the `pkg.tar.zst` package.

Outputs are written to `packaging/out/` with names of the form:

```text
packaging/out/legion-control_<version>_amd64.deb
packaging/out/legion-control-<version>-1.<dist>.x86_64.rpm
packaging/out/legion-control-<version>-1-x86_64.pkg.tar.zst
```

Package-specific staging is implemented in `packaging/debian/build.sh`, `packaging/rpm/legion-control.spec`, and `packaging/arch/PKGBUILD`. The packages include the daemon, CLI, GTK application, setup helper, systemd unit, udev rules, PolicyKit policy, desktop entry, icons, and pinned optional `ryzen_smu` source. The Arch recipe runs `cargo test --all-targets --locked` during its package check.

The native packages install binaries under `/usr/bin`; the source installer normally installs them under `/usr/local/bin`. `packaging/README.md` explicitly warns not to install both styles at the same time. Packages do not automatically install or load the optional `ryzen_smu` DKMS module. The optional backend requires DKMS and matching kernel headers and may require Secure Boot signing.

For a source build and install, `install.sh` checks native dependencies, ensures Rust 1.87+, builds release binaries with `cargo build --release`, and installs the binaries and selected integration files. Review its options and prefix behavior before running it on a development machine.

## KDE Plasma widget workflow

The widget package is `kde-widget/package`. Its package ID is `com.github.encomjp.legioncontrol`, its main file is `kde-widget/package/contents/ui/main.qml`, and `kde-widget/package/metadata.json` declares Plasma API minimum version 6.0.

Install or update it for the current user with:

```bash
cd kde-widget
chmod +x install.sh
./install.sh
```

The installer tries `kpackagetool6 --type Plasma/Applet -i package` first and falls back to update mode if installation fails. It does not restart Plasma. Add “Legion Control” from Plasma’s widget picker after installation.

Equivalent direct commands from the repository root are:

```bash
kpackagetool6 --type Plasma/Applet -i kde-widget/package
kpackagetool6 --type Plasma/Applet -u kde-widget/package
kpackagetool6 --type Plasma/Applet -r com.github.encomjp.legioncontrol
```

The repository script for removal is:

```bash
./kde-widget/uninstall.sh
```

`kde-widget/CMakeLists.txt` is an installation mechanism for the package under `${KDE_INSTALL_DATADIR}/plasma/plasmoids/com.github.encomjp.legioncontrol`; it does not define a C++ plugin. The widget’s polling and command contract is implemented by `kde-widget/package/contents/ui/legion-poll.sh` and `legion-command.sh`. The poller looks for `legion-cli` in `/usr/local/bin`, `/usr/bin`, and `$HOME/.local/bin`, and emits telemetry/status keys such as `LEGION_OK`, `LEGION_DAEMON_OFFLINE`, `CPU_TEMP`, `DGPU_TEMP`, `FAN_CPU`, `BATTERY`, `CHARGE_LIMIT`, and `PROFILE`.

`plasmawindowed` can be used as an optional manual visual smoke test when available, but the repository does not define a complete `plasmawindowed` invocation or an automated widget test.

## Hardware-sensitive testing

The project reads and, for write commands, changes hardware and system state through paths and interfaces including `/sys/class/hwmon`, `/sys/class/power_supply`, `/sys/firmware/acpi/platform_profile`, `/sys/devices/...`, `/sys/kernel/ryzen_smu_drv`, `/sys/bus/hid`, `/sys/class/hidraw`, USB HID feature reports, `nvidia-smi`, systemd, and udev.

On a supported machine, begin with read-only diagnostics:

```bash
legion-cli status
legion-cli info
legion-cli fan
legion-cli battery
systemctl status legion-control
journalctl -u legion-control -n 20
lsusb | grep 048d
```

Do not run hardware-writing commands automatically in CI or on an unsupported laptop. These include:

```text
legion-cli set-profile ...
legion-cli set-fan ...
legion-cli fan-auto
legion-cli charge-limit ...
legion-cli set-boost ...
legion-cli set-smt ...
legion-cli effect ...
legion-cli brightness ...
legion-cli rgb-fix
legion-cli set-undervolt ...
legion-cli reset-undervolt ...
```

Fan writes change cooling behavior; charge-limit writes change battery policy; RGB reset can USB-reset or rebind the HID device; and Curve Optimizer values can destabilize or crash the system. `src/undervolt.rs` limits accepted offsets to `-30..=0`, but the limit does not make an unstable value safe. Never run `examples/test_async_rgb.rs` as an unattended test.

The primary validated machine is the Lenovo Legion Pro 7 16AFR10H (machine type `83RU`) with Gen 10 Spectrum RGB keyboard USB ID `048d:c197`. Other Gen 10 Legion models are described as likely-compatible, while older generations use different RGB protocols. Include the laptop model and machine type, BIOS, kernel, distribution, GPU driver, `lsusb`, and exact CLI or service output when reporting hardware behavior.

## Ignored and generated artifacts

The repository’s `.gitignore` ignores:

- `/target/` for Cargo build output
- `*.qmlc` for Qt/QML compiled cache files
- `*.rpm`, `*.deb`, and `*.AppImage` for local package artifacts
- `.env` and `.env.*`, except `.env.example`
- `.DS_Store`, `*.swp`, and `*.swo` editor metadata

Do not add these generated or machine-local files to a contribution. `packaging/build-all.sh` also excludes `target/`, `packaging/out/`, and `.hermes/` from its temporary source archive. Note that existing package files under `packaging/out/` may be tracked despite the general `*.rpm` and `*.deb` ignore patterns; check Git state rather than assuming every package output is ignored.

## Contribution checklist

Before opening a pull request, run the checks relevant to the files changed:

- [ ] Run `cargo fmt --all -- --check`.
- [ ] Run `cargo check --workspace --all-targets`.
- [ ] Run `cargo test`, and record any host/hardware-sensitive failure rather than hiding it.
- [ ] Run `cargo clippy --all-targets` and `cargo build --release` for Rust changes.
- [ ] Run the QML `qmllint` command and shell syntax checks for widget, installer, or packaging script changes.
- [ ] For widget changes, install, update, and remove the package with `kpackagetool6`; inspect it manually on Plasma 6 when available.
- [ ] For service, udev, PolicyKit, or packaging changes, inspect the staged paths and package hooks and verify the `/usr/bin` versus `/usr/local/bin` installation distinction.
- [ ] Do not run hardware-writing commands unless the machine is supported, the effect is understood, and a rollback plan exists.
- [ ] Keep generated output and local environment files out of the change. Treat tracked files in `packaging/out/` deliberately.
- [ ] Run `git diff --check`.
- [ ] Run `git status --short` and confirm that only intended files are present.
- [ ] Include hardware and exact command output when a hardware-specific change or bug report requires it.

There is no repository CI workflow currently documented, so local verification remains important. This guide intentionally does not claim a fully automated Plasma visual test or a hardware-independent test suite where the source does not provide one.

## Uncertainty and environment notes

The commands above are grounded in the current repository scripts and manifests. Results can still vary with installed GTK/KDE development modules, kernel interfaces, GPU drivers, systemd state, and the physical laptop. In particular, `qmllint` depends on local Plasma/Kirigami module discovery, `cargo test` includes machine detection, and optional AMD tuning depends on DKMS and kernel headers. There is no checked-in `qmllint` or ShellCheck configuration and no CI workflow to substitute for these local checks.
