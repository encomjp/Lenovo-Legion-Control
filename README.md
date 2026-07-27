<div align="center">

# 🎮 Legion Control

### Fans, power modes, Spectrum RGB, battery tools, and a KDE Plasma widget — all in one place for Lenovo Legion laptops on Linux.

[![License: GPL-2.0](https://img.shields.io/badge/license-GPL--2.0-blue?style=flat-square)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-2021-orange?style=flat-square)](https://www.rust-lang.org/)
[![Legion red](https://img.shields.io/badge/accent-%23c8102e-c8102e?style=flat-square)](#)

Fan control · Power profiles · Spectrum RGB · Per-key painter · Battery health · System tray · KDE widget · CLI · Daemon

---

<a href="https://www.paypal.com/donate/?hosted_button_id=H4SCC24R8KS4A"><img src="https://img.shields.io/badge/%E2%98%95_Support_Development-PayPal-c8102e?style=for-the-badge&logo=paypal&logoColor=white" alt="Donate" height="36" /></a>

</div>

---

## ✨ Features

| | Feature | Description |
|---|---|---|
| 🌀 | **Fan Control** | Auto mode or manual RPM per fan (CPU, GPU, Aux) |
| ⚡ | **Power Profiles** | Quiet, Balanced, Performance, Max Power, or Custom PPT |
| 🚀 | **CPU Boost** | Toggle turbo boost on/off |
| 🌈 | **Spectrum RGB** | Independent keyboard · front · rear · logo · chassis zones |
| ✏️ | **Per-Key Painter** | Click-to-paint individual keys with custom colours (DE + US layouts) |
| 💡 | **Logo LED** | On/off toggle |
| 🔋 | **Battery Health** | Charge limit 60 / 80 / 100% with health %, cycles, voltage |
| 📊 | **System Monitor** | Live CPU/iGPU/dGPU temps, power draw, fan RPMs, NVMe/RAM temps |
| 🔔 | **System Tray** | KDE tray icon with sensor tooltip, close-to-tray |
| 🖥️ | **KDE Plasma Widget** | Animated gauges, sparklines, quick controls, battery bar |
| 🖥️ | **GTK4 GUI** | Modern libadwaita interface — overview, cooling, lighting, power, troubleshoot |
| ⌨️ | **CLI** | Full command-line control for scripting & automation |
| 🔄 | **Daemon** | Systemd root service auto-loads saved settings on startup |
| 📝 | **Logging** | Ring buffer, file rotation, runtime log-level switch, in-app log viewer |
| 🩺 | **Diagnostics** | Speaker/AW88399 smart amp check, RGB panic detection + auto-fix, audio soft-reset |
| 🔧 | **SMT Control** | Enable/disable hyperthreading |
| 🧪 | **AMD Curve Optimizer** | Capability-gated all-core offsets, guarded startup reapply, and a cancellable stability check |

---

## 📋 Supported Devices

Built for and verified on **Lenovo Legion Pro 7 16AFR10H (83RU)** with Gen 10 Spectrum RGB keyboard (ITE `048d:c197`).

| Model | Fans | Profiles | RGB | Per-Key | Charge Limit |
|-------|------|----------|-----|---------|-------------|
| Legion Pro 7 16AFR10H (Gen 10) | ✅ | ✅ | ✅ Spectrum | ✅ | ✅ 60/80/100% |
| Other Gen 10 Legion | ✅ likely | ✅ likely | ✅ if `048d:c197` | ✅ | ✅ likely |
| Older Legion (Gen 7-9) | ✅ likely | ✅ likely | ❌ different protocol | ❌ | ✅ likely |

**Check if your laptop has Spectrum RGB:**
```bash
lsusb | grep 048d
# 048d:c197 = Spectrum RGB keyboard
# 048d:c193 = Lenovo Lighting (logo LED)
```

---
## 📥 Installation

### Native packages

Build all three package formats in clean containers:

```bash
./packaging/build-all.sh
```

Install the package for your distribution:

```bash
# Ubuntu 24.04+
sudo apt install ./packaging/out/legion-control_*_amd64.deb

# Fedora 40+
sudo dnf install ./packaging/out/legion-control-*.x86_64.rpm

# CachyOS / Arch
sudo pacman -U ./packaging/out/legion-control-*-x86_64.pkg.tar.zst
```

Native packages own `/usr/bin`, install the systemd service and udev rules,
and enable/start the daemon in one package-manager transaction. Do not combine
a native package with a manual `/usr/local` source installation.

### Quick Install (CachyOS / Arch, Ubuntu 24.04+, Fedora 40+)

```bash
git clone https://github.com/encomjp/lenovo-legion-tool.git
cd lenovo-legion-tool
./install.sh
```

The installer:
1. Installs build dependencies (asks first — pass `-y` to skip prompts)
2. Builds release binaries with `cargo`
3. Installs `legion-daemon`, `legion-cli`, `legion-settings` to `/usr/local/bin/`
4. Sets up the systemd system service (`legion-control`)
5. Installs udev rules for HID access
6. Creates desktop entry for the GUI
7. Installs the narrowly scoped PolicyKit setup helper and pinned `ryzen_smu` source

**Minimum build versions:** Rust 1.87, GTK 4.14, and libadwaita 1.5. The
installer checks these versions and installs/updates stable Rust through
`rustup` when the distro Rust compiler is missing or too old. Ubuntu 22.04
and Debian 12 have older GTK/libadwaita releases and are not supported by the
current GUI build; Ubuntu 24.04+, Fedora 40+, and rolling CachyOS/Arch are.

```bash
./install.sh -y          # no package prompts
./install.sh --user      # binaries → ~/.local/bin
./install.sh --widget    # also install/update the Plasma widget
./install.sh --with-ryzen-smu # also install/load the optional AMD backend
./install.sh --help      # see all options
```

**After install:**
```bash
legion-settings                 # launch GUI
legion-cli status               # check sensors
systemctl status legion-control # verify daemon
```

> **Note:** Log out and back in (or reboot) after first install for udev HID rules to take effect. Spectrum RGB works without the daemon — it talks to HID directly.

### AMD Curve Optimizer (Optional)

On the validated Ryzen 9 9955HX3D / Granite Ridge platform, open
**Legion Settings → CPU → Undervolt**. If needed, install the bundled
`ryzen_smu` DKMS backend there or from **About → First-time setup**.
The GUI invokes only the fixed `legion-control-setup` helper through PolicyKit;
it never runs `sudo`, `run0`, arbitrary shell text, or a moving network source.

Tuning is deliberately limited to one all-core offset from `0` through `-30`.
Every apply is followed by per-core read-back verification. **Reset baseline**
restores the values first observed when the daemon started. Optional startup
reapplication waits 60 seconds and uses a persistent validation marker: an
interrupted validation window disables reapplication on the next daemon start.
The CPU menu also includes a cancellable five-minute all-thread stability check;
passing it is a quick confidence check, not proof of long-term stability.

The firmware on this model exposes no validated iGPU voltage control, so the app
does not offer an unsafe generic SMU/MSR iGPU slider.
Without explicit opt-in, no value is reapplied after reboot. Unstable offsets can
crash the machine or corrupt active work.

CLI equivalents:

```bash
legion-cli undervolt
legion-cli set-undervolt --offset -10 --i-understand-instability-risk
legion-cli reset-undervolt --i-understand-instability-risk
```

The optional backend needs DKMS, a C build toolchain, and headers for the
running kernel. Secure Boot may require signing/enrolling the DKMS module.

---

### KDE Plasma Widget (Optional)

The widget shows live telemetry and quick controls in your panel or desktop.

**Recommended:** open **Legion Settings → About → KDE Plasma widget**, then
click **Install widget**. The widget is bundled in the application and installs
per-user without root. The same page can update, preview, or remove it. Plasma
is never restarted automatically.

Command-line installation:

```bash
cd kde-widget
chmod +x install.sh
./install.sh
```

Or manually:
```bash
kpackagetool6 --type Plasma/Applet -i kde-widget/package
```

Then right-click your desktop or panel → **Add Widgets** → search **"Legion Control"** → drag to desired location.

**Widget features:**
- Animated circular temperature gauges (CPU + dGPU) with colour zones
- Real-time sparklines for CPU temperature history
- Metric cards: CPU, dGPU, fan RPMs with icons and sub-values
- Animated battery bar with charging pulse and charge-limit badge
- Click-to-cycle quick controls: profile, fan, KB brightness, logo, charge limit
- Daemon online/offline status indicator
- Compact panel mode with colour-coded CPU temp
- Configurable refresh interval, gauge toggle, sparkline toggle

**Uninstall widget:**
```bash
./kde-widget/uninstall.sh
```

---

### Manual Build from Source

Requires Rust, GTK4, libadwaita, and pkg-config.

**Arch / CachyOS:**
```bash
sudo pacman -Syu --needed base-devel rust gtk4 libadwaita pkgconf hidapi systemd
```

**Ubuntu 24.04+:**
```bash
sudo apt install build-essential curl pkg-config libgtk-4-dev libadwaita-1-dev libglib2.0-dev libudev-dev
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

**Fedora 40+:**
```bash
sudo dnf install gcc gcc-c++ make curl pkgconf-pkg-config gtk4-devel libadwaita-devel glib2-devel systemd-devel
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Then:
```bash
git clone https://github.com/encomjp/lenovo-legion-tool.git
cd lenovo-legion-tool
cargo build --release
```

**Install manually:**
```bash
# Copy binaries
sudo cp target/release/legion-daemon /usr/local/bin/
sudo cp target/release/legion-cli /usr/local/bin/
sudo cp target/release/legion-settings /usr/local/bin/

# Set up daemon
sudo cp data/systemd/legion-control.system.service /etc/systemd/system/legion-control.service
sudo systemctl daemon-reload
sudo systemctl enable --now legion-control

# Udev rules for HID access
sudo cp data/udev/99-legion.rules /etc/udev/rules.d/99-legion.rules
sudo udevadm control --reload-rules
sudo udevadm trigger
```

**After a rebuild, refresh the daemon:**
```bash
sudo systemctl stop legion-control
sudo cp target/release/legion-daemon /usr/local/bin/legion-daemon
sudo cp target/release/legion-cli /usr/local/bin/legion-cli
sudo cp target/release/legion-settings /usr/local/bin/legion-settings
sudo systemctl start legion-control
```

---

## ⌨️ CLI Cheatsheet

```bash
# ─── Sensors ───
legion-cli status              # all sensor readings
legion-cli info                 # device info (model, BIOS, EC, fans, GPU)
legion-cli watch                # live monitor (2s refresh)
legion-cli battery              # battery %, voltage, cycles, charge limit

# ─── Power ───
legion-cli profile              # current platform profile
legion-cli set-profile balanced # quiet | balanced | performance | max-power | custom
legion-cli boost                # CPU turbo boost state
legion-cli set-boost on         # on | off
legion-cli smt                  # hyperthreading state
legion-cli set-smt on           # on | off

# ─── Fans ───
legion-cli fan                  # all fan RPMs
legion-cli set-fan 1 3500       # fan 1 (CPU) → 3500 RPM (0 = auto)
legion-cli set-fan 2 3000       # fan 2 (GPU)
legion-cli fan-auto             # all fans to auto

# ─── Keyboard brightness ───
legion-cli kbd                  # current brightness
legion-cli set-kbd 2            # 0=off, 1=low, 2=high

# ─── Spectrum RGB ───
legion-cli brightness 7         # spectrum brightness 0-9
legion-cli effect static 200 16 46 --zone keyboard
legion-cli effect static 0 120 255 --zone front
legion-cli effect rainbow-wave --zone rear
legion-cli effect rain 0 180 255 --speed 2
legion-cli effect off

# ─── Logo LED ───
legion-cli logo                 # current state
legion-cli set-logo on          # on | off

# ─── Battery ───
legion-cli charge-limit 80      # 60 | 80 | 100
legion-cli conservation on      # legacy → 60%

# ─── Diagnostics ───
legion-cli audio                # speaker / AW88399 smart amp diagnose
legion-cli audio-fix            # soft-reset speakers
legion-cli rgb-status           # RGB panic detection
legion-cli rgb-fix              # auto-fix RGB panic (soft → USB → rebind)

# ─── Daemon logs ───
legion-cli logs 50              # fetch last 50 log lines
legion-cli set-log-level debug  # info | debug | trace | warn | error
```

**Zones:** `all` · `keyboard` · `front` · `rear` · `logo` · `chassis`

**Effects:** `static` · `color-pulse` · `color-wave` · `rainbow-wave` · `screw-rainbow` · `smooth` · `color-change` · `rain` · `ripple` · `reactive` · `off`

---

## 🏗️ Architecture

```
┌──────────────────┐    unix socket    ┌───────────────────┐
│ legion-settings  │ ────────────────► │  legion-daemon    │  (root)
│ legion-cli       │                   │  fans / profile   │
│ KDE widget       │                   │  charge limit     │
└────────┬─────────┘                   │  PPT / SMT / boost │
         │ HID                         └───────────────────┘
         ▼
┌──────────────────┐    sysfs / nvidia-smi
│ Spectrum RGB     │  048d:c197
│ Lenovo Lighting   │  048d:c193
└──────────────────┘
```

- **Daemon** (`legion-daemon`): root systemd service, manages fans, profiles, charge limit, PPT, SMT, boost. Exposes Unix socket at `/run/legion-control.socket`.
- **GUI** (`legion-settings`): GTK4/libadwaita app — overview, cooling, lighting, per-key painter, power, troubleshoot.
- **CLI** (`legion-cli`): wraps all daemon commands + direct RGB/HID calls.
- **KDE Widget**: pure QML plasmoid, polls `legion-cli` via shell script.
- **Settings**: stored in `~/.config/legion-control/settings.json`, restore on launch.
- **Logs**: ring buffer (500 entries) + optional file rotation (`~/.local/share/legion-control/daemon.log`, 7-day retention). Runtime log-level switch via `legion-cli set-log-level` or SIGHUP.

---

## 🧩 KDE Plasma Widget

A native KDE Plasma 6 plasmoid for quick access from your panel or desktop.

### Install

```bash
cd kde-widget
./install.sh
```

Or:
```bash
kpackagetool6 --type Plasma/Applet -i kde-widget/package
```

### What it shows

- **Compact (panel):** icon + colour-coded CPU temp
- **Expanded:**
  - Animated circular gauges (CPU + dGPU) with green/yellow/orange/red zones
  - Sparkline mini-charts for CPU temperature history
  - Metric cards: CPU, dGPU, fans (CPU/GPU/Aux) with power sub-values
  - Animated battery bar with charging pulse + charge limit badge
  - Click-to-cycle controls: Profile, CPU Fan, KB Brightness, Logo LED, Charge Limit
  - Daemon status dot (green/red)
  - "Open Settings" button

### Configure

Right-click widget → Configure:
- **Refresh interval** (1–10 seconds, default 2)
- **Show temperature gauges** (toggle)
- **Show sparklines** (toggle)

### Uninstall

```bash
./kde-widget/uninstall.sh
```

---

## 🔄 Service Management

The daemon runs as a **systemd system service** (root, for hardware access):

```bash
# Status
systemctl status legion-control

# Restart
sudo systemctl restart legion-control

# Stop / start
sudo systemctl stop legion-control
sudo systemctl start legion-control

# View logs
journalctl -u legion-control -f

# Or via CLI
legion-cli logs 50
legion-cli set-log-level debug
```

---

## 🛠️ Troubleshooting

### "Cannot connect to daemon"

```bash
systemctl status legion-control
sudo systemctl start legion-control
journalctl -u legion-control -n 20
```

### "Permission denied" on HID (RGB not working)

```bash
sudo udevadm control --reload-rules
sudo udevadm trigger
```

Log out and back in, or reboot.

### Keyboard brightness shows "spectrum (9)"

This is correct for Gen 10 models — the backlight is controlled via Spectrum RGB (0–9 range), not a standard LED class device.

### Fan shows "0 RPM" on Auto

Common WMI quirk — the UI and CLI show **Auto** instead of a fake zero reading.

### dGPU shows "Off" or "—"

The dGPU is in D3 sleep state (Optimus). It will show live data when it wakes under load.

### RGB stuck / keyboard dark ("RGB panic")

```bash
legion-cli rgb-status    # diagnose
legion-cli rgb-fix        # auto-fix: soft reset → USB reset → HID rebind
```

---

## 📁 Project Structure

```
lenovo-legion-tool/
├── src/
│   ├── lib.rs              # shared core crate (legion_core)
│   ├── comms.rs            # daemon IPC protocol (socket commands)
│   ├── config.rs           # settings.json load/save
│   ├── sensors.rs          # hwmon + nvidia-smi sensor reads
│   ├── fans.rs             # fan RPM read/write
│   ├── profile.rs          # ACPI platform profile + PPT
│   ├── keyboard.rs         # Spectrum RGB HID + per-key
│   ├── battery.rs          # charge limit / conservation mode
│   ├── audio.rs            # AW88399 smart amp diagnostics
│   ├── rgb_panic.rs        # RGB panic detection + recovery
│   ├── dgpu.rs             # nvidia-smi subprocess (with timeout)
│   ├── cpu.rs              # CPU boost / SMT control
│   ├── device.rs           # hardware detection / fingerprint
│   ├── logging.rs          # ring buffer + file rotation + runtime level
│   ├── models.rs           # GPU TGP classification
│   ├── cli/main.rs         # legion-cli (clap)
│   ├── daemon/main.rs      # legion-daemon (socket server)
│   └── settings/
│       ├── main.rs         # legion-settings GUI (GTK4)
│       ├── lighting.rs     # lighting tab
│       ├── perkey.rs       # per-key painter
│       ├── queue.rs        # apply queue
│       ├── tray.rs          # system tray
│       └── widgets.rs      # shared UI helpers
├── data/
│   ├── systemd/            # systemd unit file
│   ├── udev/               # udev rules
│   └── icons/              # 512×512 SVG icons
├── kde-widget/             # KDE Plasma 6 plasmoid
│   ├── package/
│   │   ├── metadata.json
│   │   └── contents/
│   │       ├── config/main.xml
│   │       └── ui/
│   │           ├── main.qml
│   │           ├── Gauge.qml
│   │           ├── Sparkline.qml
│   │           ├── MetricCard.qml
│   │           ├── QuickControl.qml
│   │           ├── BatteryBar.qml
│   │           └── legion-poll.sh
│   ├── CMakeLists.txt
│   ├── install.sh
│   └── uninstall.sh
├── Cargo.toml
├── install.sh
└── README.md
```

---

## ⚠️ Disclaimer

This is experimental community software. Use at your own risk. No warranty is provided.

| | |
|---|---|
| ❌ | Not affiliated with Lenovo |
| ❌ | Not responsible for hardware damage |
| ✅ | Works on my machine™ (Legion Pro 7 16AFR10H / RTX 5080) |

---

## 📄 License

**GPL-2.0-only** — see [LICENSE](LICENSE).

Spectrum protocol notes borrowed from community reverse-engineering ([legion-spectrum-control](https://github.com/alstergee/legion-spectrum-control), [LenovoLegionToolkit](https://github.com/BartoszCichecki/LenovoLegionToolkit)).

---

## 🤝 Contributing

PRs and issues welcome — especially other models, keymaps, and kernel quirks. For RGB bugs, include **model**, `lsusb | grep 048d`, and keyboard layout.

---

<div align="center">

<a href="https://www.paypal.com/donate/?hosted_button_id=H4SCC24R8KS4A"><img src="https://img.shields.io/badge/%E2%98%95_Support_Development-PayPal-c8102e?style=for-the-badge&logo=paypal&logoColor=white" alt="Donate" height="36" /></a>

**⭐ If this project helps you, give it a star!**

</div>