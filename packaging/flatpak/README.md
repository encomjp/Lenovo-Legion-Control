# Flatpak (sandboxed settings UI)

The Flatpak contains **only the unprivileged settings application**. A
sandbox cannot hold root hardware access, so — exactly like the portable
AppImage — the privileged `legion-control` daemon lives on the **host** and
the UI talks to it over `/run/legion-control.socket`.

## What works out of the box

- Every page that reads the daemon: fans, profiles, charge limit, thermal
  governor, Curve Optimizer, logs, diagnostics
- Spectrum RGB / logo / per-key lighting — the UI writes HID directly, which
  works because `--device=all` plus the host `uaccess` udev rule grant the
  seat user access to the controller
- Update check and diagnostics upload (`--share=network`)
- System tray (`org.kde.StatusNotifierWatcher`)

## What the sandbox cannot do

- Run the PolicyKit setup helper (`pkexec` does not exist in the sandbox) —
  the app detects Flatpak and replaces the Enable/Install buttons with
  host-install instructions
- Install the KDE Plasma widget (host Plasma cannot see the sandboxed home) —
  the section is hidden

## Install the host daemon

Any native channel works and provides the same socket:

```bash
# Arch/CachyOS
sudo pacman -U packaging/out/legion-control-0.1.0-1-x86_64.pkg.tar.zst
# or the source installer
./install.sh --daemon
```

## Build

```bash
flatpak-builder --user --install --force-clean build \
    packaging/flatpak/com.encomjp.legion-settings.yml

# single-file bundle for sharing
flatpak build-bundle ~/.local/share/flatpak/repo \
    legion-control-0.1.0.flatpak com.encomjp.legion-settings
```

The manifest uses the local checkout (`type: dir`); swap in a git source for
CI builds.