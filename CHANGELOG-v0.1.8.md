# Changelog — 0.1.8 (2026-08-29)

## Fixed

### Spectrum RGB: all effects failed with `HIDIOCGFEATURE returned short read: 5 < 960` (regression in 0.1.5)
- Commit `13a77ff` (0.1.5 hardening pass) added a strict check demanding all 960 bytes of every HID feature-reply. The ITE 048d:c197 firmware legitimately answers profile (`0xCA`), brightness (`0xCD`), and logo (`0xA5`) queries with 5-byte ACK frames (`07 <op> 01 00 <value>`), so *every* effect application — which always reads the active profile first — failed since 0.1.5.
- Now a reply is accepted when it carries the Spectrum report ID (`0x07`); only unusable replies (no report ID) are rejected. Verified live on 83RU: `effect static`, `rainbow-wave`, `brightness`, all zones.

### AppImage bootstrap never installed `legion-cli` / `legion-settings` on the host
- The one-click Enable staged only `legion-daemon`, the unit, polkit policy, and the setup helper. The KDE widget's scripts (`legion-poll.sh`, `legion-command.sh`, `legion-settings.sh`) searched only `PATH`, `~/.local/bin`, `/usr/local/bin`, `/usr/bin` — all empty, so the widget permanently reported **"Service offline"** (poll emitted `LEGION_CLI_NOT_FOUND=1`) with fan/profile/charge controls greyed out, and "Open Legion Control" answered *"Legion Settings is not installed"* — even though the daemon itself was running fine.
- Interim host repair: the current CLI/GUI are installed to `/usr/local/bin` so widget + launcher work immediately.

### HID permission after staged installs
- udevd kept a cached stale GID for the recreated `legion` group, leaving `/dev/hidraw*` on `root:root` (Spectrum `Permission denied` in the GUI log). Recreate-once workaround: `systemctl restart systemd-udevd` (done). The udev rule now also pins `GROUP="legion"` explicitly so future group recreations cannot wedge device permissions again (applied locally at `/etc/udev/rules.d/99-legion.rules`; shipped rule patched too).