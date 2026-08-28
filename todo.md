# Legion Control — TODO

Last updated: 2026-08-28

## Next up

### Ship v0.1.1
- Artifacts built at `888d14f` (AppImage, deb, rpm, Arch) in `lenovo-legion-tool/packaging/out/` + `packaged/`.
- Waiting on a fresh first-run validation of the new 5-step guided setup (welcome → Enable service autoinstall → Startup & tuning (autostart + daemon boot-enable + ryzen_smu) → Hardware → SelfCheck) on a wiped host.
- Then: `gh release create v0.1.1` + upload the four artifacts (AppImage, deb, rpm, Arch pkg).

### Fresh first-run validation checklist
- [ ] Welcome: telemetry ON default + privacy-policy link, red "Keep on" first.
- [ ] Guided setup step 1 Enable service autoinstalls + starts the daemon (one pkexec prompt).
- [ ] Startup & tuning: autostart entry + daemon `enabled` for boot; ryzen_smu install works.
- [ ] Settings hub (Setup / Fix / Hardware / Help) with symbolic tab icons — no image-missing tiles.
- [ ] Left rail: Home/CPU/Cooling/Lighting/Battery/Profiles/Settings.

## Future Features Backlog (Planned)

### 1. Telemetry Ingest & Webhook Alerts (Item 3)
- Real-time Discord / Telegram / Matrix notifications on Critical hardware faults (fan stall, continuous throttling, battery degradation).
- ClickHouse pre-aggregated Materialized Views for high-speed multi-month fleet telemetry analysis.
- One-click CSV/JSON export for filtered machine and telemetry views in the Web Portal.

### 2. Custom Temperature-to-RPM Fan Curves (Item 4)
- User-defined temperature-to-RPM curves and control loop in `legion-daemon` (see `docs/superpowers/plans/2026-08-25-custom-fan-curves-plan.md`).
- Multi-point fan curve interactive graph editor in GTK4 settings.

### 3. Granular Per-Core Curve Optimizer Tuning
- Expand the AMD tuning UI with an optional per-core offset table alongside the global all-core slider.

## High priority

### Power-button / mode LED
- **Dropped for now** — not Spectrum; EC-tied to platform profile.
- Dumps: `research/ec-mode-dumps/`, notes in `research/SPECTRUM-ZONES.md`.

## Done this session (2026-08-28)

- **v0.1.1 prep**: version bump (Cargo/spec/PKGBUILD), deb/rpm/Arch rebuilt in containers, AppImage rebuilt.
- **Distribution**: Flatpak and Snap dropped — AppImage is the recommended and sole distribution path; `docs/INSTALLATION.md` rewritten around it.
- **Settings hub**: About → Settings; Fix embedded as a compact tab (Setup / Fix / Hardware / Help); left rail decluttered.
- **Welcome revamp**: real window, telemetry ON by default with privacy-policy link and red "Keep on"; guided setup 5-step with daemon autoinstall (one pkexec transaction), autostart + daemon boot-enable, and ryzen_smu install.
- **UI fixes**: symbolic icons for all hub tabs (was image-missing tiles), stray badge dot gone, toast/label width fixes, sparkline legend.
- **Daemon/service**: `ProtectHome=read-only` restored; AppImage one-transaction pkexec bootstrap (stages daemon, helper, polkit, unit, ryzen_smu source).

## Done earlier this session

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
