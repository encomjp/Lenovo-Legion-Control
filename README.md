# Legion Control

<p align="center"><img src="lenovo-legion-tool/docs/assets/made-in-europe.svg" alt="Made in Europe · for everyone" width="310"></p>

<p align="center"><sub>Made in the European Union · for everyone</sub></p>

Linux control app for **Lenovo Legion** laptops — fans, power modes, Spectrum RGB, battery, **CPU Tuning** (thermal throttle 70–98°C, Curve Optimizer, stability test — chips on top, hover tips) and tray autostart.

This is the **monorepo root**: research notes, kernel-driver PoCs, and the SVG icon set live here; the Rust application, installer, and GitHub-facing docs live in the nested repo **[`lenovo-legion-tool/`](./lenovo-legion-tool/)**.

## Alpha telemetry (opt-in)

Alpha builds can send **one anonymized JSON** report — hardware model/type/BIOS/CPU/GPU/EC, distro+kernel, sensors, fans, battery health, thermal/Curve-Optimizer settings, settings digest, sanitized daemon-log tail, self-check results — to the developer's collector ([`server/`](./server/README.md)). **Off by default**, enabled in Setup; endpoint configurable. Never included: hostname, username, serials, MAC/IP addresses.

```bash
./install.sh          # from this repo root (delegates to lenovo-legion-tool/install.sh)
# or
cd lenovo-legion-tool && ./install.sh
```

→ **[Full project README](./lenovo-legion-tool/README.md)** · **[Architecture](./lenovo-legion-tool/docs/ARCHITECTURE.md)** · **[Usage](./lenovo-legion-tool/docs/USAGE.md)**

Full change history: [CHANGELOG.md](CHANGELOG.md).

## Repo layout

```
lenovo-legion-control/              this repo (monorepo root, branch main)
├── README.md                       this file
├── install.sh                      thin wrapper → lenovo-legion-tool/install.sh
├── todo.md                         rolling task list
│
├── lenovo-legion-tool/             Rust app + nested git repo (branch master)
│   ├── src/                        shared lib + 4 binaries (daemon, cli, settings, setup-helper)
│   ├── data/                       icons, desktop entry, polkit, systemd units, udev rule
│   ├── docs/                       ARCHITECTURE, USAGE, INSTALLATION, HARDWARE-AND-HID,
│   │                               TROUBLESHOOTING, WIDGET, DEVELOPMENT
│   ├── driver/                     legion_hwmon kernel driver + DKMS
│   ├── kde-widget/                 optional KDE Plasma 6 QML widget
│   ├── packaging/                  .deb / .rpm / Arch PKGBUILD + build scripts
│   ├── scripts/                    enable-root-daemon.sh
│   ├── examples/                   test_async_rgb.rs
│   ├── third_party/ryzen_smu/      upstream Curve Optimizer driver (vendored)
│   ├── .hermes/plans/              session plans (tracked)
│   ├── .superpowers/sdd/           SDD task briefs + reports
│   ├── target/                     cargo build cache (~2.9 GB, gitignored)
│   └── .git/                       nested repo (github.com/encomjp/lenovo-legion-tool)
│
├── research/                       EC / hwmon / WMI research, sensor dumps, UI shots
│   ├── EC-RESEARCH.md              EC protocol + register research
│   ├── EC-REGISTER-MAP.md          EC register offset map
│   ├── SPECTRUM-ZONES.md           Spectrum RGB zone / effect notes
│   ├── SHORT.md                    short findings summary
│   ├── sensor-research-findings.md sensor-stack findings
│   ├── sensors-full.md / sensors-raw.txt   raw sensor captures
│   ├── RESEARCH-TOOLING.md          probe methods + reproduction commands
│   ├── ec-mode-dumps/              256 B EC RAM mode dumps (+ summary.json)
│   └── ui-shots/                   UX screenshots (~30 MB)
│
├── driver/                         kernel-driver PoCs (EC RAM, EC-WMI)
│   ├── legion_ec.c                 EC RAM via ioremap @ 0xFE00D400 (debugfs, read-only)
│   ├── legion_ec_wmi.c             WMI3/ACPI EC method fallback
│   └── Makefile                    build both; insmod + debugfs test target
│
├── icon-preview/                   SVG icon gallery (open index.html)
│
├── docs/superpowers/               plans + spec: KDE-native widget
│
├── .commandcode/                   local: taste + design/smell/review reports
├── .superpowers/                   local: brainstorm state
├── graphify-out/                   local: graphify index cache
├── .directory                      KDE folder-icon setting
└── sqlite_mcp_server.db            local: empty MCP db
```

## Reading order

1. **`lenovo-legion-tool/README.md`** — features, install, hardware support, guide index.
2. **`lenovo-legion-tool/docs/ARCHITECTURE.md`** — components, IPC, data flow, permissions.
3. **`research/`** — how the EC, hwmon, and Spectrum HID behaviors were worked out.
4. **`icon-preview/index.html`** — browse the two-color SVG icon set.

## Local working dirs

The dot-directories (`.commandcode/`, `.superpowers/`, `graphify-out/`, `.directory`, `sqlite_mcp_server.db`) are local tooling/metadata for this working copy. They are gitignored in the outer repo and are not part of the shipped project.
