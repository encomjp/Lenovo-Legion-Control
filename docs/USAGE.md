# Legion Control usage guide

This guide covers the GTK application, `legion-cli`, the daemon that serves hardware-control requests, telemetry, saved profiles, cooling, battery charging, Spectrum RGB, diagnostics, logs, settings, and the optional AMD Curve Optimizer path.

The commands below assume the binaries are installed and available on `PATH`. From a source checkout, build them with:

```bash
cargo build --release
```

The binaries are then under `target/release/`:

- `target/release/legion-settings` — GTK4/libadwaita GUI
- `target/release/legion-cli` — command-line client
- `target/release/legion-daemon` — background hardware-control service

## 1. Start the daemon first

Most profile, fan, charge-limit, CPU-feature, and diagnostic operations use `legion-daemon`. The packaged system service runs it as root so it can write the relevant sysfs and firmware interfaces:

```bash
sudo systemctl enable --now legion-control
systemctl status legion-control
```

The service unit is defined in [`data/systemd/legion-control.system.service`](../data/systemd/legion-control.system.service). It starts `/usr/local/bin/legion-daemon` as a root system service. Its `info` logging level is configured by the unit environment.

For a per-user, non-root daemon, the repository also contains [`data/systemd/legion-control.service`](../data/systemd/legion-control.service), which starts `/usr/bin/legion-daemon` as the logged-in user. A non-root daemon can provide read access where available, but the daemon warns that platform-profile, fan, and conservation writes will fail.

The daemon listens on a Unix socket. A root daemon binds `/run/legion-control.socket`; a non-root process uses `$XDG_RUNTIME_DIR/legion-control.socket` when that variable is set, otherwise it falls back to `/tmp/legion-control.socket`. An empty `XDG_RUNTIME_DIR` value is still treated as a configured directory and can produce a relative socket path; unset it rather than setting it empty if you need the `/tmp` fallback. Clients try the system socket first and then the per-user socket. This behavior is implemented in [`src/comms.rs`](../src/comms.rs).

If a client cannot connect, it reports the socket error and suggests:

```text
Start the daemon: sudo systemctl enable --now legion-control
```

## 2. GTK GUI: `legion-settings`

Launch the GTK4/libadwaita application with:

```bash
legion-settings
```

The executable is [`src/settings/main.rs`](../src/settings/main.rs), and its Cargo binary name is declared in [`Cargo.toml`](../Cargo.toml). The window has these main areas:

- **Home** — CPU, GPU, battery, and fan telemetry; platform mode; Custom-mode power controls.
- **CPU** — boost, SMT, power-limit information, Curve Optimizer, and a five-minute stability test.
- **Cooling** — CPU fan (fan `1`), GPU fan (fan `2`), Aux/chassis fan (fan `4`), and reset-to-automatic control.
- **Lighting** — keyboard, front, rear, logo, and “More” controls, including per-key lighting.
- **Battery** — battery status/details and charge limit.
- **Fix** — speaker diagnostics/repair, Spectrum RGB recovery, and daemon logs.
- **Profiles** — named presets and restore-on-launch.
- **About** — setup, hardware, storage, and help pages.

If the daemon is offline, the GUI displays a service banner. Its **Start daemon** action tries `systemctl start legion-control`, then `run0 systemctl start legion-control`, then `pkexec systemctl start legion-control`, and waits for the control socket. The GUI disables controls that require the daemon while it is offline. Battery readouts and some local RGB operations can still work directly through `legion_core`.

### Telemetry in the GUI

The Home page polls approximately every two seconds. It reads CPU usage from `/proc/stat`, GPU utilization through the discrete-GPU helper, and sensor snapshots through the daemon when available. The displayed sources include:

- CPU temperature (EC CPU where available, otherwise CPU package temperature) and CPU busy percentage.
- Discrete GPU temperature and utilization, with GPU power when available.
- Fan RPM and target state for each detected fan.
- Battery percentage and status.
- CPU package power when the supported RAPL path is available.

The core sensor reader is [`src/sensors.rs`](../src/sensors.rs). It discovers hwmon devices rather than assuming fixed hwmon numbering. Sources include `k10temp`, `amdgpu`, `legion_hwmon`, `nvme`, `spd5118`, `iwlwifi_1`, and `r8169`; discrete-GPU values come from [`src/dgpu.rs`](../src/dgpu.rs). Missing sources are represented as unavailable/default readings, not as a promise that every machine exposes every metric.

## 3. `legion-cli` command reference

`legion-cli` is defined in [`src/cli/main.rs`](../src/cli/main.rs). Run the built-in help to inspect the parser exposed by the current build:

```bash
legion-cli --help
```

The following sections use the exact command names and argument shapes from that parser.

### Sensors and device information

Show one sensor snapshot:

```bash
legion-cli status
```

The output includes the current profile, CPU/EC temperatures, iGPU and dGPU readings, fans, detected NVMe/RAM/network temperatures, and battery summary. `status` asks the daemon for `GetSensors`; the daemon throttles unchanged sensor log entries but returns a fresh reading to the client.

Refresh the snapshot every two seconds until interrupted:

```bash
legion-cli watch
```

Show the detected platform and capability information:

```bash
legion-cli info
```

`info` reports the model, machine type, series, generation when known, BIOS, EC, CPU/GPU, profile source/match, fan backend and ranges, lighting backend, peak GPU wattage source, and available platform profiles.

### Platform profiles and CPU features

Read or set the platform profile:

```bash
legion-cli profile
legion-cli set-profile quiet
legion-cli set-profile balanced
legion-cli set-profile performance
legion-cli set-profile max-power
legion-cli set-profile custom
```

`set-profile` accepts aliases: `quiet`, `low`, `low-power`, and `lowpower` map to `low-power`; `bal` maps to `balanced`; `perf` maps to `performance`; and `max`/`maxpower` map to `max-power`. The actual allowed choices come from the platform-profile interface. `max-power` prints a heat and hardware-wear warning before writing.

`custom` is supported only when a platform-profile class handler is available; the legacy aggregate sysfs path rejects writing `custom`. Custom-mode CPU/GPU power attributes are exposed by the GUI only when the firmware exposes usable ranges, and they are meaningful while the platform profile is `custom`.

Inspect and change CPU boost/turbo:

```bash
legion-cli boost
legion-cli set-boost on
legion-cli set-boost off
```

Inspect and change SMT/hyperthreading:

```bash
legion-cli smt
legion-cli set-smt on
legion-cli set-smt off
```

Accepted state values for `set-boost` and `set-smt` include `on`, `off`, `1`, `0`, `true`, and `false`; `set-smt` also accepts `enable` and `disable`. Disabling SMT prints a warning because it reduces the number of logical CPUs.

### Fans

List the three CLI fan channels:

```bash
legion-cli fan
```

The fixed channel IDs are fan `1` = CPU, fan `2` = GPU, and fan `4` = Aux. Set a target RPM with:

```bash
legion-cli set-fan 1 3500
legion-cli set-fan 2 3000
legion-cli set-fan 4 2500
```

Use `0` to return a channel to the firmware's automatic curve:

```bash
legion-cli set-fan 1 0
legion-cli fan-auto
```

The fan module prefers the `lenovo_wmi_other` hwmon backend and falls back to `legion_hwmon`; see [`src/fans.rs`](../src/fans.rs). On a machine that does not expose a channel, reads or writes can return an error. The GUI discovers the channels and their live min/max ranges. It warns before accepting a manual target at or above approximately 85% of the discovered maximum.

### Keyboard backlight and Spectrum RGB

For the standard keyboard brightness path, use levels `0` (off), `1` (low), or `2` (high):

```bash
legion-cli kbd
legion-cli set-kbd 0
legion-cli set-kbd 1
legion-cli set-kbd 2
```

On Spectrum hardware, `kbd` can report Spectrum brightness values as `3`–`9`; use the dedicated Spectrum brightness command for that range:

```bash
legion-cli brightness 7
```

Set a static RGB color using decimal red, green, and blue values from `0` to `255`:

```bash
legion-cli rgb 200 16 46
```

Set a Spectrum effect:

```bash
legion-cli effect static 200 16 46 --zone keyboard
legion-cli effect color-pulse 0 120 255 --speed 2 --zone front
legion-cli effect rainbow-wave --zone rear
legion-cli effect rain 0 180 255 --speed 2
legion-cli effect off
```

The parser defaults effect colors to `200 16 46`, speed to `2`, and zone to `all`. Valid effect names are:

```text
static, color-pulse, color-wave, rainbow-wave, screw-rainbow,
 smooth, rain, ripple, reactive, off
```

Valid zones are `all`, `keyboard`, `front`, `rear`, `logo`, and `chassis`. An unknown zone is reported and treated as `all`; an unknown effect is rejected with the list of supported names. `off` is implemented as a black static effect for the selected zone.

The RGB implementation is [`src/keyboard.rs`](../src/keyboard.rs). It uses the Gen 10 Spectrum HID interface (`048d:c197`). The CLI's `rgb`, `effect`, and `brightness` commands use direct HID, while the GUI lighting/profile paths can also use direct HID through `legion_core` where implemented; the daemon separately exposes corresponding effect, brightness, logo, and profile IPC operations for the GUI and other clients.

Read and change the lid logo LED:

```bash
legion-cli logo
legion-cli set-logo on
legion-cli set-logo off
```

### Battery and charge limits

Show battery percentage, status, voltage, cycles, and effective charge limit:

```bash
legion-cli battery
```

The firmware supports discrete effective limits. Values from `0` through `69` select approximately 60%; `70` through `89` select 80%; and `90` or higher selects 100%. The normal explicit commands are:

```bash
legion-cli charge-limit 60
legion-cli charge-limit 80
legion-cli charge-limit 100
```

The legacy boolean command maps `on`/`true`/`1` to approximately 60% and every other value to 100%:

```bash
legion-cli conservation on
legion-cli conservation off
```

The mapping is implemented in [`src/battery.rs`](../src/battery.rs): 60% uses the conservation mode, 80% uses `Long_Life`, and 100% uses `Standard`. The GUI's Battery → Charge limit page exposes only the three supported choices. Battery readouts come from `/sys/class/power_supply/BAT0` and do not require the daemon, while changing the charge limit normally goes through the daemon.

### Diagnostics and repair

Diagnose the onboard speakers/AW88399 smart amplifier:

```bash
legion-cli audio
```

`audio` prints a health classification, summary, amplifier ACPI/binding/module state, firmware, HDA card, mute/bass/volume/sink checks, and details. It exits with status `1` for a soft issue and `2` for hardware-broken; a healthy or not-applicable result does not use those failure statuses.

Run the speaker soft-reset/troubleshooting sequence:

```bash
legion-cli audio-fix
```

The fix reports its steps and final health. It exits `1` for a remaining soft issue or reported error and `2` for hardware-broken.

Diagnose or repair Spectrum RGB panic conditions:

```bash
legion-cli rgb-status
legion-cli rgb-fix
```

The daemon-backed path reports health, summary, details, and whether the condition is fixable. If the daemon is unavailable or has an incompatible IPC variant, the CLI falls back to the local diagnostic/fix path; that recovery ladder may attempt USB reset/rebind when the process has the required permissions. A `broken` result exits `2`; a `soft-issue` result exits `1`.

The daemon also runs an RGB watchdog. After startup it periodically checks the Spectrum HID/kernel USB state and can perform the recovery sequence automatically when `rgb_panic::needs_autofix` says it is needed. The implementation is in [`src/rgb_panic.rs`](../src/rgb_panic.rs) and [`src/daemon/main.rs`](../src/daemon/main.rs).

### Logs and runtime log level

Fetch recent daemon log lines; the default is 50:

```bash
legion-cli logs
legion-cli logs 100
```

Change the running daemon's maximum level:

```bash
legion-cli set-log-level error
legion-cli set-log-level warn
legion-cli set-log-level info
legion-cli set-log-level debug
legion-cli set-log-level trace
```

The daemon accepts `off` as well. Unknown levels are rejected. The in-memory ring defaults to 500 entries and is capped at 2,000. The GUI's Fix → Service logs page fetches 100 entries, lets you copy them, and toggles between `info` and `debug`.

Logging behavior is implemented in [`src/logging.rs`](../src/logging.rs). By default entries are written to stderr and the ring buffer. For the root system service, stderr is normally collected by journald; use `journalctl -u legion-control`. Set `LEGION_LOG_FILE=1` for a rotated JSON log under the process user's platform data directory, normally `~/.local/share/legion-control/`; for the root daemon this is often `/root/.local/share/legion-control/`, not the desktop user's log directory. Files older than seven days are cleaned up. `LEGION_LOG_RING` changes the ring size within the supported 100–2,000 range. The GUI's own logs are separate user-process logs.

`SIGHUP` reloads the daemon's inherited environment log filter; it does not change the service unit's environment. Prefer the IPC command for a running service:

```bash
legion-cli set-log-level debug
```

To make the daemon reread its inherited filter after changing the service environment, send `SIGHUP`:

```bash
sudo systemctl kill --signal=SIGHUP legion-control
```

## 4. Settings and named profiles

The GUI persists settings at:

```text
~/.config/legion-control/settings.json
```

`XDG_CONFIG_HOME` changes the base directory. The settings model and path handling are in [`src/config.rs`](../src/config.rs). The file contains the current lighting state, per-key colors, keyboard layout (`de` or `us`), charge limit, last-session power/fan values, named profiles, active profile, UI color, and the `restore_on_launch` flag.

In **Profiles**:

1. Enter a name and choose **Save current**.
2. The snapshot records the current platform profile, discovered Custom-mode PPT values, fan targets, lighting, charge limit, and related session settings.
3. Select a saved name and choose **Load** to apply it now.
4. Choose **Delete** to remove it from the settings file.
5. Enable **Restore last session on launch** to re-apply saved session values such as fans, charge limit, and lighting when the GUI starts. The saved platform profile is recorded in the profile data but is not restored by this launch path.

The GUI also stores zone effects and per-key maps. Lighting changes are saved in the same settings file, while applying them sends the relevant Spectrum HID operation. The per-key editor supports DE QWERTZ and US QWERTY layouts; selecting a layout changes the editor mapping, not the physical LED matrix.

The repository's settings queue coalesces pending fan and firmware-attribute writes before sending them to the daemon. This means rapidly moving a GUI slider does not imply one IPC write per pointer event.

## 5. Optional AMD Curve Optimizer safety path

This feature is intentionally narrower than a general-purpose undervolting tool. It is implemented in [`src/undervolt.rs`](../src/undervolt.rs), exposed through daemon commands in [`src/comms.rs`](../src/comms.rs), and guarded by [`src/cli/main.rs`](../src/cli/main.rs).

### Availability and safety conditions

The read-only capability probe must succeed before a write is enabled. The current validated target is:

- `ryzen_smu` loaded at `/sys/kernel/ryzen_smu_drv`.
- Granite Ridge codename `23`.
- Product name `83RU`.
- Product version containing `Legion Pro 7 16AFR10H`.
- AMD Ryzen 9 9955HX3D in `/proc/cpuinfo`.
- A validated 16-physical-core layout.

Other hardware receives an unavailable status rather than an unvalidated SMU write. The supported temporary all-core range is `-30..=0`; negative values are the undervolt direction. Every apply writes all cores and reads all cores back. Firmware resets the temporary value at reboot.

Check availability and current/baseline values:

```bash
legion-cli undervolt
```

Apply a temporary all-core offset only after testing a conservative value and explicitly acknowledging the risk:

```bash
legion-cli set-undervolt \
  --offset -10 \
  --i-understand-instability-risk
```

The flag is mandatory. Without it, the CLI exits with status `2` and performs no write. Unstable values can crash the machine or corrupt active work.

Restore the captured firmware baseline:

```bash
legion-cli reset-undervolt \
  --i-understand-instability-risk
```

A baseline is captured before Legion Control's first write after daemon startup. If the baseline is not a uniform all-core value, the implementation does not attempt a per-core restore; rebooting safely restores firmware defaults.

The GUI's CPU → Undervolt page exposes the same capability-gated range, Apply, Reset, and status read-back. It can optionally enable **Apply after startup**, but this is not enabled by default. When enabled, the daemon waits 60 seconds after startup, applies the validated offset, and keeps a recovery marker during a 300-second validation window. If the prior validation was interrupted, the next start disables the startup setting instead of retrying indefinitely. The GUI's CPU → Stability test is a separate cancellable five-minute, all-thread memory/CPU stress check; a pass is only a quick confidence check, not proof of long-term stability.

No CLI command is provided for an arbitrary iGPU voltage or unrestricted SMU/MSR tuning.

## 6. Thermal Throttle governor (`scaling_max_freq` clamp)

Daemon-native replacement for the external `cpu-throttle-95.sh` + `cpu95-throttle.service`. The `legion-daemon` thermal governor watches `k10temp` (`Tctl` + `Tccd2`, `max` of the two) and clamps `cpu*/cpufreq/scaling_max_freq` between `5_460_527` kHz (full) and `4_600_000` kHz in `200_000` kHz steps, polling every `1s`. It throttles when `temp ≥ max_temp` and restores only when `temp ≤ max_temp − 7°C` (fixed `7°C` hysteresis, `restore = max − 7`). The hardware TjMax `95°C` remains the failsafe if the daemon stops. Persistence is `AppConfig.thermal` in `settings.json` (`VERSION 4`, `#[serde(default)]` so old files migrate to `enabled=false, max_temp=90`).

> **Deprecation:** `cpu95-throttle.service` is superseded. On the first `SetThermal(enabled=true)` the daemon best-effort runs `systemctl disable --now cpu95-throttle.service` (warn-only on failure) to avoid double-clamping. The bash file may remain on disk but should stay disabled.

Valid `max_temp` is `70–98` (`default 90`). `96–98°C` exceeds TjMax `95°C` and requires explicit acknowledgement.

### CLI: `legion-cli thermal`

```bash
# Show live status (config + live temps/freq + idle/throttling)
legion-cli thermal status
# Thermal: on · max 90°C (restore 83°C) · cur 5460527 kHz · Tctl 68.4°C / Tccd2 64.2°C · idle

# Enable (or re-configure) — enables if currently off
legion-cli thermal set --max-temp 85

# Expert 96–98°C — requires acknowledgement
legion-cli thermal set --max-temp 98 --acknowledge-high-temp

# Disable
legion-cli thermal set --off

# Explicit enable with a value
legion-cli thermal set --on --max-temp 90
```

`status` calls `GetThermalStatus` and prints `on|off`, `max` and `restore`, `cur` `scaling_max_freq`, `Tctl`/`Tccd2`, and `idle` vs `throttling`. `set` validates locally (`70..=98`, ack for `96–98`) then sends `SetThermal { enabled, max_temp, acknowledge }`; without `--max-temp` it reuses the current `max_temp` from `GetThermalStatus` (default `90` if the daemon is unreachable). `--off` and `--on` are mutually exclusive; `--max-temp` without `--off` implies `enabled=true`.

CLI implementation is in [`src/cli/main.rs`](../src/cli/main.rs); validation and stepping math live in [`src/thermal.rs`](../src/thermal.rs) (`validate`, `compute_target`).

### GUI: Cooling → Thermal Throttle

In `legion-settings` **Cooling**, the **Thermal Throttle** card (`Clamp scaling_max_freq when hot`) contains:

1. **Enable** `GtkSwitch` bound to `thermal.enabled` — toggles `SetThermal` immediately.
2. **Max temp** `GtkScale` `70–98` step `1` with `"{n}°C"` value label; sensitive only when enabled. Moves update a read-only `Restore at {max−7}°C` label. Value changes are debounced `140 ms` before `SetThermal`.
3. **Warning row** hidden until `scale ≥96` — `GtkLabel` with CSS class `warning` text `"96–98°C exceeds TjMax (95°C) — instability risk"` and a `GtkCheckButton "I understand"` (`acknowledge`). `SetThermal` for `≥96` only succeeds when checked.
4. **Live status row** — two chips `Tctl` / `Tccd2` and `max_freq` (e.g. `68.4°C / 64.2°C · 5.46 GHz`) tinted via `tint_temp` (`≥90 red, ≥78 amber`), plus `Throttling` vs `Idle`. Polls `GetThermalStatus` every `2s`; errors surface as a transient `Toast`.

Page load reads `GetThermal` once to populate the switch/scale; the poll keeps chips live. The queue reuses the existing `140 ms` coalescing pattern from [`src/settings/queue.rs`](../src/settings/queue.rs).

## 7. Troubleshooting checklist

1. Confirm the service is running:
   ```bash
   systemctl status legion-control
   ```
2. Confirm the CLI can reach it:
   ```bash
   legion-cli profile
   ```
3. Inspect hardware capabilities:
   ```bash
   legion-cli info
   ```
4. Inspect recent daemon output:
   ```bash
   legion-cli logs 100
   ```
5. For a missing Spectrum controller, check the USB ID:
   ```bash
   lsusb | grep 048d
   ```
6. If the daemon binary was rebuilt or the IPC enum changed, restart the service so CLI and daemon use the same build:
   ```bash
   sudo systemctl restart legion-control
   ```

Hardware support is capability-driven. Unsupported hwmon channels, missing battery attributes, unavailable NVIDIA tools, absent platform-profile interfaces, and unsupported Curve Optimizer hardware are reported as unavailable or errors rather than silently treated as supported.

## Source map

The behavior documented here is implemented primarily by:

- [`src/cli/main.rs`](../src/cli/main.rs) — command names, arguments, output, aliases, warnings, and exit statuses.
- [`src/comms.rs`](../src/comms.rs) — daemon command/response protocol and Unix-socket selection.
- [`src/daemon/main.rs`](../src/daemon/main.rs) — IPC dispatch, root-service behavior, sensor logging, and RGB watchdog.
- [`src/settings/main.rs`](../src/settings/main.rs) — GTK pages, telemetry polling, profiles, fan UI, battery UI, diagnostics, logs, and Curve Optimizer UI.
- [`src/config.rs`](../src/config.rs) — persistent settings and named profiles.
- [`src/sensors.rs`](../src/sensors.rs) — hwmon/sysfs telemetry.
- [`src/profile.rs`](../src/profile.rs) — platform profiles and Custom-mode power attributes.
- [`src/fans.rs`](../src/fans.rs) — fan channel discovery and RPM targets.
- [`src/battery.rs`](../src/battery.rs) — 60/80/100% firmware charge-limit mapping.
- [`src/keyboard.rs`](../src/keyboard.rs) — Spectrum HID lighting, zones, effects, brightness, and logo.
- [`src/logging.rs`](../src/logging.rs) — stderr, ring-buffer, optional rotated-file, and runtime filtering.
- [`src/undervolt.rs`](../src/undervolt.rs) — validated temporary Curve Optimizer access and recovery-guarded startup reapplication.
- [`data/systemd/legion-control.system.service`](../data/systemd/legion-control.system.service) — packaged root service.

If a machine exposes a different firmware backend or lacks one of the described sysfs/HID interfaces, the exact available controls and readings can differ.
