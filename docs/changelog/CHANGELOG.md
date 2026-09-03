# Changelog

Single repo since 2026-08-25 (outer monorepo merged under `meta/`).
Newest first. Pre-attribution SHAs live on branch `backup/pre-attribution`.

Per-release GitHub notes: [`v0.2.13.md`](v0.2.13.md), [`v0.2.12.md`](v0.2.12.md), [`v0.2.11.md`](v0.2.11.md), [`v0.2.10.md`](v0.2.10.md), [`v0.2.4.md`](v0.2.4.md), [`v0.2.3.md`](v0.2.3.md), [`v0.2.2.md`](v0.2.2.md), [`v0.2.1.md`](v0.2.1.md), [`v0.2.0.md`](v0.2.0.md), [`v0.1.9.md`](v0.1.9.md), [`v0.1.8.md`](v0.1.8.md).
The app does not read this folder; in-app update text comes from the GitHub release body.

## Unreleased

- Catalog: promoted the 7 stub MTs into 9 real product rows (83EW/EG/EF split) with PSREF names + lighting enums; BIOS prefixes filled only from live/firmware-tracker sources (RYCN, NSCN, RLCN, NMCN, M3CN, MACN, M2CN); 83EF/83EG/83EX stay empty until live dmidecode; 83Q7 lighting set to sku_variant (1-zone RGB or 24-zone per Spec PDF).

- Lighting: structured `LightingKind` capability probe (`Spectrum` / `FourZone` / `White`) replaces the string-only `lighting` check.
- Lighting: capability-gated UI — Spectrum keeps all tabs; 4-zone and white-only boards collapse to a notice page with the backlight slider, instead of exposing tabs that write to nothing.
- Lighting: Lighting nav tooltip and startup Spectrum restore are now capability-driven; add `LEGION_LIGHTING_OVERRIDE=spectrum|fourzone|white` debug env to preview each branch.

- Catalog (`data/model-capabilities.json`): fix 83KY BIOS prefix Q7CN → RXCN (live LLL #355/#409 + kernel DMI match; Q7CN belongs to 83F5); correct the 82N6 known-gaps note (plain Legion 7, alt-spectrum is only 83G0/83AG); add rows for 83Q7 (T2CN, Gen 11) and the toolkit-mapped MTs 83F1, 83EW/83EG/83EF, 83LT, 83FD, 83EX, 82Y9/82Y5/82YA as unknown-capability stubs pending the PSREF pass.

## 0.2.13 - 2026-09-03

Full notes: [`v0.2.13.md`](v0.2.13.md). Fixes GitHub Issue #3 (Y7000P 16IRX9 / 83DG).

- Lighting: white-backlight slider writes route through the daemon (`SetKbdBrightness`) with `toast_error` on failure; integer-step snapping with Off/50%/100% marks and labels.
- Cooling: read-only fan cards + EC-managed note on models without a fan-target write backend (`control_backend_name().is_none()`); "All fans automatic" reset gated the same way.
- CPU → Power: Custom-watts sliders are live while in Custom mode (wired to `ensure_custom_then_ppt`); guide text reflects the active state.
- Home: Fn+Q into Custom mode immediately reveals the Custom power sliders (`ppt_box.set_visible(firmware_mode == "custom" && ...)`).
- Telemetry schema version 4 support; collector accepts schemas 1–4.
- KDE widget redesign.

## 0.2.11 - 2026-09-02

- AppImage Staging: Always restage bundled daemon and helper upon `enable-daemon`, preventing daemon version skew.
- IPC Socket: Mode 0666 socket permissions and automatic `legion` group creation in `setup-helper` to prevent client permission denied errors.
- IPC Error Reporting: Preserve `PermissionDenied` error details in `comms.rs` instead of masking with fallback socket `NotFound`.
- Hardware Profiles: Add official mapping for Legion Y7000P IRX9 (`83DG`, `NMCN`), correct Pro 7 16IRX8H fan layout, and hide Curve Optimizer on Intel systems.
- Telemetry: Collect motherboard DMI details, keyboard lighting kind, battery charge types, and deep hardware inventories; uncap server-side retention.

## 0.2.10 - 2026-09-01

- Fix Intel `coretemp` CPU temperature handling, including Package/Core readings and thermal-zone fallback.
- Gate speaker troubleshooting on `NotApplicable` for models without the AW88399 smart amp.
- Collapse lighting controls to white-only mode on hardware that exposes only white backlighting.
- Add catalog coverage for Legion 82GN/81Y8/82B3 and 82K2/82MJ, plus broader Legion hardware research.

## 0.2.4 - 2026-08-30

- Source `/usr/local` installs can use **Update now** (git tag + rebuild, or the portable tarball).
- Update dialog is a wider two-tab window (Release / What's new) instead of a Later-only notes dump.

## 0.2.3 - 2026-08-30

- Prefer yogafan for RPM when `lenovo_wmi_other` is bound but locked at 0 (LOQ 83JG idle fans).
- Includes unpublished 0.2.2: nvidia-caps DeviceAllow, GPU card fallback, metric-chip ellipsis, CLI `—` for missing temps.

## 0.2.2 - 2026-08-30

- Fix daemon NVML/nvidia-smi: `DeviceAllow=char-nvidia-caps` (GPU card was "—" after boot).
- Home GPU chip falls back to user nvidia-smi if the daemon cannot read the dGPU.
- Stop ellipsizing short metric-card titles.
- CLI shows `—` for missing dGPU/EC temps instead of `-1.0°C`.

## 0.2.1 - 2026-08-30

- Fix daemon keyboard RGB: `DeviceAllow=char-hidraw rw` in systemd unit (glob alone blocked hidraw).
- Fix `throttled_without_heat` false positive in thermal governor hysteresis band.
- Pretty-print CPU/GPU names from YAML ID maps (`cpu-ids.yaml`, `gpu-ids.yaml`).

## 0.2.0 - 2026-08-30

- In-app updates for AppImage, `.deb`, `.rpm`, Arch packages, and portable tarball — no browser.
- Plasma widget GPU card now has a temperature sparkline matching the CPU card.

## 0.1.9 - 2026-08-30

- Fix dGPU detection by scanning PCI display controllers instead of inferring hardware from installed driver tools or the first DRM card.
- Keep AMD/Intel integrated GPUs out of the discrete-GPU field and clean multi-bracket `lspci` names correctly.
- Associate NVIDIA driver versions only with a detected NVIDIA dGPU; report dGPU vendor and PCI ID for fleet triage.
- Treat an unavailable fan RPM capability as informational while still failing genuinely unreadable exposed attributes.
- Add separate RPM/control fan backends plus per-fan readability/state to telemetry schema v3.
- Reuse the battery module's BAT0/BAT1/BAT2/BATT probe in flattened sensor telemetry.

- a22ce4c 2026-08-25 fix(ppt): unit-aware limits end-to-end + unified fw-attr gate
- 7933630 2026-08-25 docs: single-repo AGENTS.md — always-commit rule + meta/ layout
- df3643f 2026-08-25 Add 'meta/' from commit '07ed72840caecd91345e3009c2105e0b45c3842e'
- 07ed728 2026-08-25 docs: add CHANGELOG from both repos, link from telemetry section
- c0c85aa 2026-08-25 chore(clippy): factor complex topology setter type
- e0e93c6 2026-08-25 fix(selftest): disambiguate Into<String> for intel presence checks
- 71edbf3 2026-08-25 feat(intel): gated PState + Uncore (hybrid topology)
- be30ca3 2026-08-25 feat: borrow cpu_temp/gpu_temp, per-CPU controls, Intel MSR offsets
- 74598c4 2026-08-25 feat(telemetry): IONOS collector + alpha docs
- e4129bc 2026-08-25 feat(diagnostics): anonymous opt-in telemetry + self-check
- b8c673a 2026-08-25 docs(research): last python snippet → dd/xxd shell equivalent
- 9827346 2026-08-25 docs(research): polish pass — fix discrepancies, drop python snippets, cross-link
- a85af9f 2026-08-25 docs(research): retire Python probes — methods compressed into RESEARCH-TOOLING.md
- ff231ed 2026-08-25 test: scale live self-test suite to 59 real-hardware checks (F01–F58)
- dc227ff 2026-08-25 refactor: rename unfriendly k10temp identifiers (tctl/tccd1/tccd2)
- d866419 2026-08-25 test: expand live self-test suite to 32 real-hardware checks (F01–F32)
- 833eeca 2026-08-25 perf: cache device identity, cap kernel-log slurp; add live-hardware tests
- ffe37e5 2026-08-25 docs: plan for per-boot machine-id rotation on the CachyOS laptop
- d430855 2026-08-25 feat(battery): detect and explain EC off-charging past the limiter
- 9e4ea83 2026-08-25 chore: remove dead code — 14 unused pub fns + 2 unused deps
- aef5437 2026-08-25 chore: cargo fmt — repo-wide formatting drift (mechanical)
- c183973 2026-08-25 fix(battery): robust verification + watchdog seeding (validation findings)
- bd036e6 2026-08-25 docs(research): battery limiter findings + decision log (4-source synthesis)
- 6edf38f 2026-08-25 fix(battery): charge_types single-write + verification + EC-clear watchdog
- 3391b8d 2026-08-25 refactor: collapse duplicated code paths
- 419a67c 2026-08-25 docs: land pending socket-boundary rewrite (hidraw 0660+uaccess, no /tmp fallback)
- e3b9451 2026-08-25 docs(plan): custom fan curves — Afterburner-style editor + daemon governor
- ec508dc 2026-08-25 chore: AGENTS.md — always-commit/rollback rule for agents
- ce2d37a 2026-08-22 chore: ignore local agent session dir
- 57e39a1 2026-08-22 fix(widget): instant controls + 40% lighter polling + offline state
- b1fd2a1 2026-08-22 fix(packaging): cross-distro install robustness (Ubuntu/Debian/Fedora/openSUSE)
- 3c28b2a 2026-08-22 ui(thermal): on/off switch moves to the group header, row removed
- cb6ded6 2026-08-22 feat(thermal): smooth governor — spike filter + proportional stepping
- 1c57dc5 2026-08-22 test(comms): IPC protocol round-trips + garbage-frame contract
- 62870e9 2026-08-22 test(settings): unit tests for CO status text and page-nav maps
- cfd106e 2026-08-22 fix(css): use margin-left for sibling pill spacing (margin-start is not a GTK CSS property)
- 22672bc 2026-08-22 ui: seven fixes from screenshot review
- a5bdfed 2026-08-22 ui: de-clutter copy + thermal temp control now fits its card
- 82a5be7 2026-08-22 redesign(ui): flat 8-row sidebar + horizontal hub tab bars
- aaed745 2026-08-22 fix(ui): inspection-fix sweep — tooltips, thermal sync, async IPC, a11y
- c8d0692 2026-08-22 chore: snapshot before UI inspection-fix sweep
- ff6708a 2026-08-21 refactor(ui): halve the sidebar — merged pages, internal switchers
- 2de4293 2026-08-21 fix(daemon): exit non-zero on fatal startup so systemd Restart=on-failure retries
- 8ac6d94 2026-08-21 fix(ryzen_smu): install backend works on any kernel + sidebar one-click nav
- fb084db 2026-08-21 fix(ui): robust header sync + profiles row layout (NN/g heuristics pass)
- 57e662d 2026-08-21 fix(ui): default-open Cooling/Battery sidebar + hide fan speed while auto
- 4192266 2026-08-21 test: expand coverage across the issue-fix sweep
- 00d2d7e 2026-08-21 test: regression tests for fixed issues
- 46d932d 2026-08-21 fix(ui): header sync, sidebar selection, battery prime, fan label, chip polish
- 831eeb3 2026-08-21 fix: security & robustness sweep
- 75c2b68 2026-08-21 chore: snapshot before issue-fix sweep
- 26a02f8 2026-08-21 chore: snapshot before issue-fix sweep
- 7094ed4 2026-08-20 fix(settings): autostart starts hidden to tray (--hidden)
- 0ac5d67 2026-08-20 refactor(settings): Garage Lab pass — Battery chips on top, Cooling/CPU Power tooltips-only
- 74d0e4f 2026-08-20 fix(settings): Tuning — restore Current session subtitle + hover, Cooling overview (additive)
- badd236 2026-08-20 chore: snapshot before Tuning UX A+B — chips grouping + Cooling overview (no removals)
- 1d53257 2026-08-20 refactor(settings): Tuning — tooltips-only, chips on top, autostart option
- cfbfe48 2026-08-20 refactor(settings): Tuning UX batch 1-5
- 4cc73d7 2026-08-20 refactor(settings): CPU Tuning = Thermal + Undervolt + Stability
- 81aaa0a 2026-08-20 refactor(settings): CPU Tuning tab — thermal + undervolt together
- ebaef52 2026-08-20 refactor(settings): move Thermal to CPU, match Garage Lab card + confirm_risk
- 7d664bb 2026-08-19 docs: thermal throttle usage and architecture
- f90704c 2026-08-19 feat(settings): Thermal Throttle card in Cooling
- ab99726 2026-08-19 feat(cli): legion-cli thermal status/set
- 559cf03 2026-08-19 feat(daemon): thermal governor thread + Set/GetThermal handling
- 8907396 2026-08-19 feat(comms): GetThermal/SetThermal/GetThermalStatus IPC
- 62ec640 2026-08-19 feat(config): AppConfig.thermal + VERSION 4 migration
- 68e05b2 2026-08-19 feat(thermal): pure core — ThermalConfig, compute_target, validation
- 9ce628e 2026-08-19 plan: daemon-native thermal throttle B2 (70–98°C, 7°C hysteresis)
- 4a4c633 2026-08-19 spec: daemon-native thermal throttle — B2 max-temp governor (70–98°C, hysteresis 7°C)
- 3574092 2026-08-10 Initial commit: Legion control research, driver, icons, docs
- 20f0ecf 2026-08-09 chore: save current state
- e1d691b 2026-08-08 docs: add project guides, slim README, fix artifact ignores
- 1ba43ab 2026-07-31 chore: ignore generated build artifacts
- d7d8c5c 2026-07-31 feat: improve Legion control widget and installer
- 16c4d88 2026-07-27 ux: UI copy cleanup and navigation improvements
- a5b80c5 2026-07-27 backup before UI copy/UX cleanup
