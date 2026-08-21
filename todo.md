# Legion Control — TODO

Last updated: 2026-08-19

## High priority

### Power-button / mode LED
- **Dropped for now** — not Spectrum; EC-tied to platform profile.
- Dumps: `research/ec-mode-dumps/`, notes in `research/SPECTRUM-ZONES.md`.

### Install / daemon
- Prefer `~/.local/bin/legion-*` until refreshed with `sudo ./scripts/enable-root-daemon.sh`.
- **CPU watts** need the new daemon (`GetCpuPower` RAPL). Restart daemon after install.

## Done this session

- Independent zones via persisted multi-effect packet (no blackout of others).
- Persist `~/.config/legion-control/settings.json` (zones, brightness, logo, per-key, UI).
- Quiet toasts (errors only).
- Lighting subtabs: Effects · Per-key · Look.
- Pretty DE QWERTZ per-key painter (click/drag).
- Fan Auto shows “Auto” instead of “0 rpm”.
- Richer battery details on Power page.
- CLI `effect … --zone keyboard|front|rear|logo|chassis`.
- **2026-08-20 Tuning batch:** CPU Tuning tab — Thermal throttle (70–98°C, governor, TjMax 95), Curve Optimizer (-30..0, startup persistence), Stability 5-min on one page; tooltips-only, chips on top, tray autostart `--hidden`, speaker false-positive fix for USB headset, Cooling collapsed to Fans.
- **UX Laws pass (2026-08-20 evening):** Glass made opaque (no YouTube bleed), thermal trough blue→red gradient + 95 TjMax tick + muted when off, `GHz` unify, tints consistent, `CPU → Tuning` chips on top inside card.

## Key paths

| Path | Role |
|------|------|
| `lenovo-legion-tool/src/settings/` | GTK4/libadwaita GUI |
| `lenovo-legion-tool/src/daemon/main.rs` | root `legion-daemon` (socket server) |
| `lenovo-legion-tool/src/cli/main.rs` | `legion-cli` client |
| `lenovo-legion-tool/src/keyboard.rs` | Spectrum HID / zones / per-key |
| `lenovo-legion-tool/src/config.rs` | Persistent settings (`settings.json`) |
| `lenovo-legion-tool/driver/` | `legion_hwmon` kernel driver + DKMS |
| `lenovo-legion-tool/kde-widget/` | Optional KDE Plasma 6 QML widget |
| `driver/` | EC-RAM / EC-WMI kernel PoCs |
| `research/` | EC, hwmon, WMI, and Spectrum research + dumps |
| `research/ui-shots/` | UX screenshots |
| `research/SPECTRUM-ZONES.md` | Zone / effect notes |
| `icon-preview/index.html` | SVG icon gallery |

## Layout

```
.                       monorepo root (branch main)
├── lenovo-legion-tool/ Rust app + nested git repo (branch master)
├── research/           research notes + dumps + UI shots
├── driver/             EC/WMI kernel PoCs
├── icon-preview/       SVG icon gallery
└── docs/superpowers/   KDE-native widget plans/specs
```
