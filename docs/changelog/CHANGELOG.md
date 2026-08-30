# Changelog

Single repo since 2026-08-25 (outer monorepo merged under `meta/`).
Newest first. Pre-attribution SHAs live on branch `backup/pre-attribution`.

Per-release GitHub notes: [`v0.2.0.md`](v0.2.0.md), [`v0.1.9.md`](v0.1.9.md), [`v0.1.8.md`](v0.1.8.md).
The app does not read this folder; in-app update text comes from the GitHub release body.

## Unreleased

- Pretty-print laptop GPU names from `data/gpu-ids.yaml` (PCI ID) when nvidia-smi is asleep or missing.
- Expand the map with Lenovo AMD APUs (610M–890M, 8060S) and discrete RX 5500M–7900M / 7700S.
- Pretty-print CPU names from `data/cpu-ids.yaml` (SKU token in `/proc/cpuinfo`).

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
