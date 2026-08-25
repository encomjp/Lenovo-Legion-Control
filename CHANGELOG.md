# Legion Control — Changelog

Generated from the git histories of the **two private source repositories** that make up this monorepo. Entries are listed newest first; dates are commit dates (YYYY-MM-DD).

> **Note:** Short SHAs are **per-repo** — a SHA only resolves inside its own repository (run `git log` from that repo's root), not across the two.

# Legion Control (monorepo docs/research)

Branch `main` — research notes, kernel-driver PoCs, icon set, agent docs.

- 8ea3a3f 2026-08-25 feat(telemetry): IONOS collector + alpha docs
- dc37e7f 2026-08-25 docs(research): last python snippet → dd/xxd shell equivalent
- a1b2b6d 2026-08-25 docs(research): polish pass — fix discrepancies, drop python snippets, cross-link
- 601c49b 2026-08-25 docs(research): retire Python probes — methods compressed into RESEARCH-TOOLING.md
- f7ac172 2026-08-25 docs: plan for per-boot machine-id rotation on the CachyOS laptop
- 433611b 2026-08-25 docs(research): battery limiter findings + decision log (4-source synthesis)
- 2ef0854 2026-08-25 docs(plan): custom fan curves — Afterburner-style editor + daemon governor
- 12ab3bc 2026-08-25 chore: AGENTS.md — always-commit/rollback rule for agents
- 2a77e36 2026-08-22 chore: ignore local agent session dir
- 82f429b 2026-08-21 chore: snapshot before issue-fix sweep
- 52dd5ad 2026-08-20 chore: snapshot before Tuning UX A+B — chips grouping + Cooling overview (no removals)
- d8dfc1c 2026-08-19 plan: daemon-native thermal throttle B2 (70–98°C, 7°C hysteresis)
- 69c3fc0 2026-08-19 spec: daemon-native thermal throttle — B2 max-temp governor (70–98°C, hysteresis 7°C)
- 70e44b2 2026-08-10 Initial commit: Legion control research, driver, icons, docs

# legion-tool (app)

Branch `master` — the Rust application (daemon, CLI, settings GUI, KDE widget), packaging, and user docs.

- d57599b 2026-08-25 fix(selftest): disambiguate Into<String> for intel presence checks
- e240a0d 2026-08-25 feat(intel): gated PState + Uncore (hybrid topology)
- cbac1b7 2026-08-25 feat: borrow cpu_temp/gpu_temp, per-CPU controls, Intel MSR offsets
- e76b2b6 2026-08-25 feat(diagnostics): anonymous opt-in telemetry + self-check
- 934a580 2026-08-25 test: scale live self-test suite to 59 real-hardware checks (F01–F58)
- 86d3aee 2026-08-25 refactor: rename unfriendly k10temp identifiers (tctl/tccd1/tccd2)
- 733ff00 2026-08-25 test: expand live self-test suite to 32 real-hardware checks (F01–F32)
- 5b9da15 2026-08-25 perf: cache device identity, cap kernel-log slurp; add live-hardware tests
- f9bbf22 2026-08-25 feat(battery): detect and explain EC off-charging past the limiter
- 7ddc614 2026-08-25 chore: remove dead code — 14 unused pub fns + 2 unused deps
- cad276e 2026-08-25 chore: cargo fmt — repo-wide formatting drift (mechanical)
- 3d6ef02 2026-08-25 fix(battery): robust verification + watchdog seeding (validation findings)
- 31d71c7 2026-08-25 fix(battery): charge_types single-write + verification + EC-clear watchdog
- 8d939d7 2026-08-25 refactor: collapse duplicated code paths
- 91536ad 2026-08-25 docs: land pending socket-boundary rewrite (hidraw 0660+uaccess, no /tmp fallback)
- d2c1c2a 2026-08-22 fix(widget): instant controls + 40% lighter polling + offline state
- 32fde87 2026-08-22 fix(packaging): cross-distro install robustness (Ubuntu/Debian/Fedora/openSUSE)
- 63181a6 2026-08-22 ui(thermal): on/off switch moves to the group header, row removed
- eba1b4f 2026-08-22 feat(thermal): smooth governor — spike filter + proportional stepping
- c88a105 2026-08-22 test(comms): IPC protocol round-trips + garbage-frame contract
- ba68fe1 2026-08-22 test(settings): unit tests for CO status text and page-nav maps
- 2c57301 2026-08-22 fix(css): use margin-left for sibling pill spacing (margin-start is not a GTK CSS property)
- 7fdaa22 2026-08-22 ui: seven fixes from screenshot review
- abd3c1f 2026-08-22 ui: de-clutter copy + thermal temp control now fits its card
- 429af92 2026-08-22 redesign(ui): flat 8-row sidebar + horizontal hub tab bars
- 5930d6a 2026-08-22 fix(ui): inspection-fix sweep — tooltips, thermal sync, async IPC, a11y
- fd97ec2 2026-08-22 chore: snapshot before UI inspection-fix sweep
- 13a54ea 2026-08-21 refactor(ui): halve the sidebar — merged pages, internal switchers
- 98be891 2026-08-21 fix(daemon): exit non-zero on fatal startup so systemd Restart=on-failure retries
- 64cae2c 2026-08-21 fix(ryzen_smu): install backend works on any kernel + sidebar one-click nav
- a37b1c5 2026-08-21 fix(ui): robust header sync + profiles row layout (NN/g heuristics pass)
- 09c6168 2026-08-21 fix(ui): default-open Cooling/Battery sidebar + hide fan speed while auto
- f325c49 2026-08-21 test: expand coverage across the issue-fix sweep
- 458e911 2026-08-21 test: regression tests for fixed issues
- 773e8ee 2026-08-21 fix(ui): header sync, sidebar selection, battery prime, fan label, chip polish
- 61c7fb8 2026-08-21 fix: security & robustness sweep
- 44d8e28 2026-08-21 chore: snapshot before issue-fix sweep
- 35221ff 2026-08-20 fix(settings): autostart starts hidden to tray (--hidden)
- 0056de8 2026-08-20 refactor(settings): Garage Lab pass — Battery chips on top, Cooling/CPU Power tooltips-only
- b06b5e1 2026-08-20 fix(settings): Tuning — restore Current session subtitle + hover, Cooling overview (additive)
- e1882cf 2026-08-20 refactor(settings): Tuning — tooltips-only, chips on top, autostart option
- aee7c33 2026-08-20 refactor(settings): Tuning UX batch 1-5
- b7247b0 2026-08-20 refactor(settings): CPU Tuning = Thermal + Undervolt + Stability
- a414dad 2026-08-20 refactor(settings): CPU Tuning tab — thermal + undervolt together
- 6fddf26 2026-08-20 refactor(settings): move Thermal to CPU, match Garage Lab card + confirm_risk
- 6e63cc2 2026-08-19 docs: thermal throttle usage and architecture
- 2f59f5e 2026-08-19 feat(settings): Thermal Throttle card in Cooling
- 37ea1da 2026-08-19 feat(cli): legion-cli thermal status/set
- 0331062 2026-08-19 feat(daemon): thermal governor thread + Set/GetThermal handling
- 58560d3 2026-08-19 feat(comms): GetThermal/SetThermal/GetThermalStatus IPC
- b345d3f 2026-08-19 feat(config): AppConfig.thermal + VERSION 4 migration
- ea1ccb5 2026-08-19 feat(thermal): pure core — ThermalConfig, compute_target, validation
- dc07de1 2026-08-09 chore: save current state
- a91d3aa 2026-08-08 docs: add project guides, slim README, fix artifact ignores
- 0ed89a4 2026-07-31 chore: ignore generated build artifacts
- dacd067 2026-07-31 feat: improve Legion control widget and installer
- 0479ec8 2026-07-27 ux: UI copy cleanup and navigation improvements
- cdf9f71 2026-07-27 backup before UI copy/UX cleanup
