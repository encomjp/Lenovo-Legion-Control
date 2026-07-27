# Native packages

Legion Control ships one native package containing the daemon, CLI, GTK app,
systemd unit, udev rules, desktop entry, icons, the PolicyKit setup helper,
pinned optional `ryzen_smu` source, and the KDE widget embedded in the settings
application.

The package owns `/usr/bin`; the source installer owns `/usr/local/bin`.
They should not be installed at the same time.

## Build all formats

```bash
./packaging/build-all.sh
```

Artifacts are written to `packaging/out/`:

- `legion-control_<version>_amd64.deb`
- `legion-control-<version>-1.<dist>.x86_64.rpm`
- `legion-control-<version>-1-x86_64.pkg.tar.zst`

The build uses clean Ubuntu, Fedora, and Arch containers. Package installation
reloads udev, enables/starts `legion-control.service`, and never restarts Plasma.
The About page installs/updates/removes the bundled Plasma widget per-user via
`kpackagetool6`, without root.

The AMD backend is opt-in. Packages do not install or load its DKMS module.
The app can request installation through the fixed PolicyKit helper after the
user authenticates. DKMS and matching kernel headers are weak/optional package
dependencies. Curve Optimizer writes remain daemon-only, range limited to
`-30..=0`, and read-back verified. Optional startup reapplication is delayed and
crash-loop guarded; the underlying firmware write itself still resets at reboot.
