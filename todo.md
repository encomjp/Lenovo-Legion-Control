# Legion Control — TODO

Last updated: 2026-07-21

## High priority

### Power-button / mode LED
- **Dropped for now** — not Spectrum; EC-tied to platform profile.
- Dumps: `research/ec-mode-dumps/`, notes in `SPECTRUM-ZONES.md`.

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

## Key paths

| Path | Role |
|------|------|
| `lenovo-legion-tool/src/settings/` | GUI |
| `lenovo-legion-tool/src/keyboard.rs` | Spectrum HID / zones / per-key |
| `lenovo-legion-tool/src/config.rs` | Persistent settings |
| `research/ui-shots/` | UX screenshots |
| `research/SPECTRUM-ZONES.md` | Zone / effect notes |
