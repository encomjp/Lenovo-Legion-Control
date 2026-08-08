# Troubleshooting

This guide is organized as **symptom → evidence → remedy**. Run read-only diagnostics first, then apply the smallest relevant fix. Commands below are the commands exposed by this repository or the paths used by its source; run them from any directory unless noted.

## First-pass diagnostics

| Symptom | Evidence | Remedy |
|---|---|---|
| Several features fail at once | `legion-cli info`, `legion-cli status`, and `systemctl status legion-control` show whether the device is detected, sensors respond, and the system service is active. | Capture the outputs and the recent service log before changing settings: `journalctl -u legion-control -n 50 --no-pager`. |
| You are unsure which hardware is supported | `lsusb \| grep 048d` identifies `048d:c197` (Spectrum RGB) and `048d:c193` (Lenovo Lighting). | The repository verifies the Legion Pro 7 16AFR10H (83RU) with Gen 10 Spectrum RGB. Other Gen 10 models are marked likely; older models use a different RGB protocol. |
| A command is unavailable | `command -v legion-cli`, `command -v legion-settings`, and `command -v legion-daemon` show which installation is on `PATH`. | Do not mix installations. Native packages own `/usr/bin`; the source installer owns `/usr/local/bin`. Remove or avoid the competing installation before troubleshooting path/service mismatches. |

## Daemon and IPC

### Symptom: `legion-cli` says it cannot connect to the daemon

**Evidence**

The client tries `/run/legion-control.socket`, then `$XDG_RUNTIME_DIR/legion-control.socket` (or `/tmp/legion-control.socket`). A root daemon binds the first path. Check both the service and socket:

```bash
systemctl status legion-control
systemctl is-active legion-control
ls -l /run/legion-control.socket "${XDG_RUNTIME_DIR:-/tmp}/legion-control.socket" 2>/dev/null
journalctl -u legion-control -n 50 --no-pager
```

**Remedy**

Start or enable the system service:

```bash
sudo systemctl enable --now legion-control
sudo systemctl restart legion-control
```

If the service starts and then stops, inspect the journal for the bind or hardware error. The daemon is intended to run as root; a non-root daemon warns that profile, fan, and conservation writes will fail. A manually launched non-root daemon uses a user socket and is not a substitute for the system service.

### Symptom: the daemon is active, but writes fail

**Evidence**

Read-only commands may work while profile, fan, or charge-limit writes return errors. Confirm the process and executable selected by the unit:

```bash
systemctl cat legion-control
systemctl status legion-control
readlink -f /proc/"$(systemctl show -p MainPID --value legion-control)"/exe
```

**Remedy**

Use the system service, not a user service or a foreground non-root daemon:

```bash
sudo systemctl stop legion-control
sudo systemctl disable --now legion-control.service 2>/dev/null || true
sudo systemctl daemon-reload
sudo systemctl enable --now legion-control
```

If you rebuilt from source, refresh the installed binaries and restart the service:

```bash
sudo systemctl stop legion-control
sudo cp target/release/legion-daemon /usr/local/bin/legion-daemon
sudo cp target/release/legion-cli /usr/local/bin/legion-cli
sudo systemctl start legion-control
```

### Symptom: logs do not show enough detail

**Evidence**

The daemon keeps a ring buffer and accepts runtime log-level changes. Check recent lines and the system journal:

```bash
legion-cli logs 50
journalctl -u legion-control -n 100 --no-pager
```

**Remedy**

Increase the running daemon's level through IPC, then reproduce the issue:

```bash
legion-cli set-log-level debug
journalctl -u legion-control -f
```

The system unit sets `LEGION_LOG=info`; changing that variable only in your shell does not change an already-running service. The daemon also supports `LEGION_LOG_FILE=1`, `LEGION_LOG_RING=500`, and `LEGION_LOG=json`; file logs are stored under the platform data directory, normally `~/.local/share/legion-control/`, when file logging is enabled.

## udev and HID

### Symptom: RGB reports permission denied or the keyboard is not found

**Evidence**

The installed rule must match the HID devices used by this project:

```bash
lsusb | grep 048d
ls -l /dev/hidraw*
udevadm info --query=property --name=/dev/hidraw0 2>/dev/null | grep -E 'ID_VENDOR_ID|ID_MODEL_ID|ID_SERIAL'
```

`data/udev/99-legion.rules` grants `0666` and `uaccess` to `048d:c193` and `048d:c197`. The Spectrum implementation searches `/sys/class/hidraw` for `048d:c197` and opens the matching `/dev/hidrawN` device.

**Remedy**

Install or reload the repository rule and retrigger HID devices:

```bash
sudo install -Dm644 data/udev/99-legion.rules /etc/udev/rules.d/99-legion.rules
sudo udevadm control --reload-rules
sudo udevadm trigger -s hidraw
```

Log out and back in, or reboot, after the first installation. If the device is absent from `lsusb`, a udev reload cannot fix detection; check the kernel USB/HID log and the physical device state:

```bash
dmesg | grep -iE 'hid|usb|048d|spectrum'
```

RGB commands use HID directly and can work without the daemon. Fans, profiles, and charge limits still require the root daemon.

## Sensors and platform profile

### Symptom: one or more temperatures are missing or displayed as zero

**Evidence**

The sensor reader discovers hwmon devices by their `name` file. It reads CPU data from `k10temp`, EC CPU/GPU data from `legion_hwmon`, iGPU data from `amdgpu`, storage from `nvme`, RAM from `spd5118`, Wi-Fi from `iwlwifi_1`, and Ethernet from `r8169` variants. The dGPU is queried with `/usr/bin/nvidia-smi`, not hwmon. Inspect the available sources directly:

```bash
legion-cli status
legion-cli info
for h in /sys/class/hwmon/hwmon*; do
  [ -r "$h/name" ] && printf '%s: %s\n' "$h" "$(cat "$h/name")"
done
ls -l /sys/class/hwmon
```

Unavailable dGPU values are represented internally as `-1.0`, so a missing dGPU value is not a measured `0°C`.

**Remedy**

Use the readings that are available and inspect the daemon log for hardware capability detection:

```bash
journalctl -u legion-control -b --no-pager | grep -E 'machine:|hardware:|legion_hwmon|sensors:'
legion-cli watch
```

`--with-dkms` optionally installs the repository's `driver/legion_hwmon.c` through DKMS. It is a fallback/EC sensor backend, not a requirement for every sensor. Do not assume a missing optional hwmon source is a daemon failure.

### Symptom: the displayed profile is `unknown` or differs from the profile command

**Evidence**

The aggregate sensor snapshot reads the legacy `/sys/firmware/acpi/platform_profile` path, while profile selection/current-profile logic also uses `/sys/class/platform-profile/*/profile`.

```bash
legion-cli profile
cat /sys/firmware/acpi/platform_profile 2>/dev/null || true
find /sys/class/platform-profile -maxdepth 2 -type f -name profile -print -exec cat {} \;
```

**Remedy**

Use `legion-cli profile` as the supported current-profile check and select a supported profile through the CLI:

```bash
legion-cli set-profile balanced
# quiet | balanced | performance | max-power | custom
```

## Fans

### Symptom: fan control is unavailable or a fan cannot be read

**Evidence**

The fan backend prefers `lenovo_wmi_other` and falls back to `legion_hwmon`. Fan IDs are `1` CPU, `2` GPU, and `4` Aux. Check the backend and channels:

```bash
legion-cli info
legion-cli fan
for h in /sys/class/hwmon/hwmon*; do
  [ -r "$h/name" ] || continue
  case "$(cat "$h/name")" in
    lenovo_wmi_other|legion_hwmon)
      printf '%s (%s)\n' "$h" "$(cat "$h/name")"
      ls "$h"/fan*_input "$h"/fan*_target "$h"/fan*_min "$h"/fan*_max 2>/dev/null || true
      ;;
  esac
done
```

**Remedy**

Run the root daemon and use the discovered fan IDs rather than assuming every channel exists:

```bash
sudo systemctl enable --now legion-control
legion-cli fan
legion-cli set-fan 1 3500
legion-cli set-fan 2 3000
legion-cli fan-auto
```

A target of `0` means automatic mode. In WMI auto mode, the hardware may report `0 RPM`; the project labels this `Auto` rather than treating it as a confirmed stopped fan. `--with-dkms` can add `legion_hwmon` as an optional backend, but it does not guarantee support on an unverified model.

## dGPU

### Symptom: dGPU shows `Off`, `—`, or unavailable

**Evidence**

The implementation invokes `/usr/bin/nvidia-smi` with a three-second timeout. Check the executable, driver visibility, and the repository's direct query:

```bash
command -v nvidia-smi
ls -l /usr/bin/nvidia-smi
/usr/bin/nvidia-smi -L
/usr/bin/nvidia-smi --query-gpu=temperature.gpu,power.draw,clocks.gr,utilization.gpu --format=csv,noheader,nounits
legion-cli status
```

**Remedy**

If the GPU is in Optimus/D3 sleep, run a workload that uses the dGPU and query again. A sleeping dGPU is expected to show `Off` or `—`; live values appear when it wakes. If `nvidia-smi` is missing, failing, or taking longer than the daemon's timeout, fix the NVIDIA driver/runtime first. Do not treat the device's power-limit heuristic as a live measurement: `legion-cli info` may use a model/PSREF fallback when `power.max_limit` is unavailable.

## Battery and charge limits

### Symptom: battery data or charge limit is missing

**Evidence**

The daemon reads `/sys/class/power_supply/BAT0`; the widget searches `BAT*`. Inspect the power-supply files and compare the CLI result:

```bash
ls -la /sys/class/power_supply/BAT0 2>/dev/null
for f in capacity status voltage_now cycle_count charge_types conservation_mode; do
  printf '%-18s' "$f"
  cat "/sys/class/power_supply/BAT0/$f" 2>/dev/null || printf '%s' unavailable
  printf '\n'
done
legion-cli battery
```

The 60% mode uses a discovered `conservation_mode` path (known Legion path first, then the `ideapad_acpi` driver); 80% uses `[Long_Life]` in `charge_types`; 100% uses `Standard` with conservation cleared.

**Remedy**

Use only the documented discrete limits:

```bash
legion-cli charge-limit 60
legion-cli charge-limit 80
legion-cli charge-limit 100
# Legacy boolean interface:
legion-cli conservation on
legion-cli conservation off
```

The implementation maps ranges to the three firmware modes, but `60`, `80`, and `100` are the supported, unambiguous inputs. These writes require the root daemon. If `BAT0` or the expected sysfs attributes do not exist, the project cannot infer or create a generic charge-limit interface for that model.

## RGB and keyboard lighting

### Symptom: keyboard is dark or RGB is stuck ("RGB panic")

**Evidence**

Run the built-in diagnosis and inspect kernel USB/HID errors:

```bash
legion-cli rgb-status
journalctl -k -b --no-pager | grep -iE 'hid|usb|048d|spectrum'
lsusb | grep 048d
```

The diagnosis checks Spectrum hidraw discovery, permissions, HID ioctl response, saved lighting/brightness state, and recent kernel USB/HID faults. `048d:c197` is the Gen 10 Spectrum device; the source does not claim the older four-zone protocol is compatible.

**Remedy**

Try the repository recovery ladder:

```bash
legion-cli rgb-fix
```

Recovery follows the repository's soft-to-hard ladder. Depending on the available permissions and device state, it can repair permissions, perform a soft lighting reset, and may attempt a USB sysfs reset and `hid-generic` rebind before reapplying lighting. Without sufficient permissions, those harder steps may fail. If it remains broken after the fix, replug the device if possible or reboot, then inspect the kernel log.

### Symptom: keyboard brightness says `spectrum (9)`

**Evidence**

Gen 10 Spectrum brightness uses a `0–9` range, while the standard keyboard LED fallback uses `0–2`:

```bash
legion-cli kbd
```

**Remedy**

Treat `spectrum (9)` as the Spectrum range, not a malformed standard LED value. Set it with:

```bash
legion-cli brightness 7
```

For a standard LED backend, the command is instead `legion-cli set-kbd 0|1|2`.

## KDE Plasma widget

### Symptom: widget says CLI not found or daemon offline

**Evidence**

The poller checks, in order, `/usr/local/bin/legion-cli`, `/usr/bin/legion-cli`, and `$HOME/.local/bin/legion-cli`. It emits `LEGION_CLI_NOT_FOUND=1` when none is executable and `LEGION_DAEMON_OFFLINE=1` when `status` returns no output.

```bash
for p in /usr/local/bin/legion-cli /usr/bin/legion-cli "$HOME/.local/bin/legion-cli"; do
  printf '%s: ' "$p"
  [ -x "$p" ] && echo present || echo missing
 done
systemctl is-active legion-control
/usr/local/bin/legion-cli status 2>/dev/null || /usr/bin/legion-cli status 2>/dev/null || "$HOME/.local/bin/legion-cli" status
```

**Remedy**

Install the CLI in one supported location, start the daemon, and reinstall/update the per-user widget if needed:

```bash
sudo systemctl enable --now legion-control
cd kde-widget
./install.sh
```

Add it from Plasma's widget picker. The widget is for KDE Plasma 6, polls every two seconds by default, and allows a 1–10 second refresh interval. Installing or updating it does not restart Plasma. Battery values are read directly from `/sys/class/power_supply/BAT*`, while status/fans/profile/keyboard/logo values come through the CLI.

### Symptom: widget is installed but does not appear or needs removal

**Evidence**

Check the package metadata and the package tool:

```bash
command -v kpackagetool6
kpackagetool6 --type Plasma/Applet -l | grep -i 'Legion\|encomjp'
```

**Remedy**

Install manually from the repository package or remove it with the repository script:

```bash
kpackagetool6 --type Plasma/Applet -i kde-widget/package
./kde-widget/uninstall.sh
```

The package ID is `com.github.encomjp.legioncontrol`. The widget requires Plasma 6 and a working `legion-cli`/daemon pair.

## Installation and mixed prefixes

### Symptom: service, GUI, and CLI appear to be different versions

**Evidence**

The source installer installs binaries under `/usr/local/bin` by default; `--user` puts CLI/GUI under `~/.local/bin` but keeps the daemon system-wide at `/usr/local/bin/legion-daemon`. Native packages own `/usr/bin`.

```bash
command -v legion-cli legion-daemon legion-settings
readlink -f "$(command -v legion-cli)"
readlink -f "$(command -v legion-daemon)"
systemctl cat legion-control
```

**Remedy**

Choose either native packages or the source installation and keep all three binaries from that installation. For a source rebuild:

```bash
cargo build --release
sudo install -Dm755 target/release/legion-cli /usr/local/bin/legion-cli
sudo install -Dm755 target/release/legion-daemon /usr/local/bin/legion-daemon
sudo install -Dm755 target/release/legion-settings /usr/local/bin/legion-settings
sudo systemctl restart legion-control
```

The source installer supports `--no-daemon`, `--no-udev`, `--with-dkms`, `--with-ryzen-smu`, `--widget`, `--deps-only`, and `--skip-build`; use `./install.sh --help` to confirm the option set before changing an existing installation.

## Debian and build dependencies

### Symptom: installation fails on Debian 12/Bookworm or an older Ubuntu

**Evidence**

The installer checks `pkg-config` versions and requires GTK 4.14+, libadwaita 1.5+, and libudev development files. The repository states that Debian 12/Bookworm and Ubuntu 22.04 ship older GTK/libadwaita versions.

```bash
. /etc/os-release
printf '%s\n' "$PRETTY_NAME"
pkg-config --modversion gtk4 2>/dev/null || true
pkg-config --modversion libadwaita-1 2>/dev/null || true
pkg-config --exists libudev && echo libudev-found || echo libudev-missing
```

**Remedy**

Use a supported release (Ubuntu 24.04+, Fedora 40+, or rolling Arch/CachyOS), or install the required development packages on a compatible system. The installer can offer to replace `bookworm` with `trixie` in `/etc/apt/sources.list` and matching files under `/etc/apt/sources.list.d`, making `.bak.bookworm` backups first. This is a broad system repository change, not a harmless per-application workaround; review and back up APT configuration and understand the upgrade before accepting it.

For Debian/Ubuntu dependency installation as implemented by `install.sh`:

```bash
sudo apt-get update
sudo apt-get install -y build-essential curl pkg-config libgtk-4-dev libadwaita-1-dev libglib2.0-dev libudev-dev
```

Check Rust as well:

```bash
rustc --version
cargo --version
```

The installer requires Rust 1.87+ and may install/update stable Rust through `rustup` if the available compiler is missing or too old.

### Symptom: a native package conflicts with a source install

**Evidence**

Native package artifacts are built with:

```bash
./packaging/build-all.sh
```

and written under `packaging/out/`. The package README explicitly distinguishes `/usr/bin` (native) from `/usr/local/bin` (source).

```bash
ls -l packaging/out 2>/dev/null
rpm -q legion-control 2>/dev/null || true
dpkg-query -W legion-control 2>/dev/null || true
```

**Remedy**

Use one distribution method. Native Debian packages run their post-install hook to reload systemd, enable/restart or start `legion-control.service`, and reload/trigger hidraw udev rules. If the package is installed, use its `/usr/bin` binaries and service rather than copying source-built binaries into `/usr/local/bin`.

## Optional AMD Curve Optimizer tuning

### Symptom: AMD tuning is unavailable or `--with-ryzen-smu` does not expose the interface

**Evidence**

The optional backend requires DKMS, a C toolchain, matching kernel headers, and (with Secure Boot) possibly module signing/enrollment. Check the driver interface and hardware identity:

```bash
command -v dkms
uname -r
ls -ld /sys/kernel/ryzen_smu_drv
cat /sys/class/dmi/id/product_name 2>/dev/null
cat /sys/class/dmi/id/product_version 2>/dev/null
awk -F: '/model name/ {print $2; exit}' /proc/cpuinfo
legion-cli undervolt
```

**Remedy**

Install the optional backend explicitly:

```bash
./install.sh --with-ryzen-smu
```

The validated write path is deliberately narrow: product name `83RU`, product version containing `Legion Pro 7 16AFR10H`, AMD Ryzen 9 9955HX3D, Granite Ridge codename, and exactly 16 physical cores. A different machine may have an upstream `ryzen_smu` capability but is rejected by Legion Control's validated write gate. The native package includes the source but does not install/load the DKMS module by default.

### Symptom: an undervolt apply is rejected or the machine becomes unstable

**Evidence**

The write path accepts one all-core offset from `-30` through `0`, requires explicit risk acknowledgement, and reads back every core after applying. Inspect current status:

```bash
legion-cli undervolt
```

**Remedy**

Use a conservative tested value and the explicit acknowledgement:

```bash
legion-cli set-undervolt --offset -10 --i-understand-instability-risk
legion-cli reset-undervolt --i-understand-instability-risk
```

Offsets are temporary and normally reset at reboot. Reset restores the captured boot baseline only when that baseline is uniform; otherwise the implementation advises rebooting to restore firmware defaults. Unstable offsets can crash the machine or corrupt active work. Optional startup reapplication waits 60 seconds, validates for five minutes, and disables itself after an unclean validation window; passing the check is not proof of long-term stability.

## Useful source paths

- Daemon and service behavior: `src/daemon/main.rs`, `src/comms.rs`, `data/systemd/legion-control.system.service`, `packaging/common/legion-control.service`
- Sensor and hardware access: `src/sensors.rs`, `src/fans.rs`, `src/dgpu.rs`, `src/battery.rs`, `src/profile.rs`
- HID/RGB: `data/udev/99-legion.rules`, `src/keyboard.rs`, `src/rgb_panic.rs`
- CLI commands: `src/cli/main.rs`
- Installer and optional backends: `install.sh`, `driver/`, `third_party/ryzen_smu/`, `src/undervolt.rs`
- Widget: `kde-widget/README.md`, `kde-widget/install.sh`, `kde-widget/package/contents/ui/legion-poll.sh`, `kde-widget/package/contents/ui/main.qml`
- Native packages and Debian hook: `packaging/README.md`, `packaging/build-all.sh`, `packaging/debian/build.sh`, `packaging/debian/postinst`

If a command here reports a path or capability that does not exist on the machine, treat that as evidence about the kernel, firmware, driver, model, or installation rather than creating a new generic interface. This project is verified on the hardware listed in `README.md`; behavior on other Legion generations is not guaranteed.
