# Snap (devmode-only reference)

The snap bundles UI + daemon together, but Legion Control needs privileged
hardware access (`/sys`, `ec_sys`, `/dev/hidraw*`) that has no dedicated
strict-confinement interface upstream. As a result this `snapcraft.yaml`
is intentionally `confinement: devmode` and **not store-ready** — it builds
and runs locally for testing.

```bash
snapcraft pack --destructive-mode   # needs LXD/multipass or --destructive-mode on Arch
sudo snap install --devmode legion-control_0.1.0_amd64.snap
```

For normal distribution use the native packages (`packaging/out/*.deb|*.rpm|*.pkg.tar.zst`),
the portable AppImage, or the Flatpak (UI) + host daemon instead.

The monorepo builds all native formats in containers (`packaging/build-all.sh`).

No `snapd` or `snapcraft` is available on this CachyOS host by default, so
this snap has not been built or run here.
