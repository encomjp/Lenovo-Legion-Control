# Changelog - 0.2.0 (2026-08-30)

## New

### In-app updates without opening GitHub

- **Update now** in Settings → Setup (and `legion-cli check-update --apply`) downloads the matching release asset for this install, verifies sha256, and installs it.
- AppImage: replaces the running file, then one password prompt restages the daemon after restart.
- Native packages: `.deb` via apt, `.rpm` via dnf/zypper/rpm, Arch `.pkg.tar.zst` via pacman — one PolicyKit prompt; package scripts restart the service.
- Portable `*-x86_64.tar.gz` (when shipped): installs binaries under `/usr/local`.
- Source-tree copies still use `git pull && ./install.sh`.

### GPU temperature sparkline on the Plasma widget

- The expanded GPU gauge now has the same history graph as CPU, so both cards match.
- History ignores offline/negative dGPU readings. Settings checkbox is “Show temperature sparklines”.

## Notes

- First launch of 0.2.0 is required before later versions can apply in-app (0.1.x still opens the release page).
