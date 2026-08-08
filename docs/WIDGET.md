# KDE Plasma 6 widget

`kde-widget/` contains the Plasma 6 applet for Lenovo Legion telemetry and quick controls. The package is a `Plasma/Applet` with plugin ID `com.github.encomjp.legioncontrol`, a minimum Plasma API version of 6.0, and `package/contents/ui/main.qml` as its main script. These values come from [`kde-widget/package/metadata.json`](../kde-widget/package/metadata.json).

## Source layout

The widget source is organized as a KPackage under [`kde-widget/package/`](../kde-widget/package/):

```text
kde-widget/
├── CMakeLists.txt
├── install.sh
├── uninstall.sh
└── package/
    ├── metadata.json
    └── contents/
        ├── config/
        │   ├── config.qml
        │   ├── configGeneral.qml
        │   └── main.xml
        └── ui/
            ├── main.qml
            ├── BatteryBar.qml
            ├── Gauge.qml
            ├── MetricCard.qml
            ├── MonitorRow.qml
            ├── QuickControl.qml
            ├── SectionCard.qml
            ├── Sparkline.qml
            ├── legion-command.sh
            ├── legion-info.sh
            ├── legion-poll.sh
            ├── legion-settings.sh
            └── icons/
```

`kde-widget/CMakeLists.txt` installs the complete `package/` directory below `${KDE_INSTALL_DATADIR}/plasma/plasmoids/com.github.encomjp.legioncontrol`.

## Current Plasma layout

The root item in [`kde-widget/package/contents/ui/main.qml`](../kde-widget/package/contents/ui/main.qml) is a `PlasmoidItem` with two representations.

### Compact panel representation

The compact representation is a clickable `MouseArea`. It shows the CPU SVG icon and CPU temperature, and clicking it toggles the expanded representation. The CPU text is colored using the current CPU temperature: above 90 °C uses the negative theme color, above 75 °C uses the neutral theme color, and lower values use the normal text color.

Its tooltip can include CPU and dGPU temperatures, CPU fan speed, battery percentage, profile, and `Daemon offline`. The tooltip is visible while the compact item is hovered.

### Expanded representation

The expanded representation is a `QQC2.ScrollView` with a preferred width of 26 Kirigami grid units and minimum/maximum widths of 24/30 grid units. Its column contains, in order:

1. Optional circular CPU and dGPU temperature gauges.
2. A **System Monitor** `SectionCard` with CPU and dGPU `MonitorRow`s.
3. A **Battery** `SectionCard` with one `BatteryBar`.
4. A **Controls** `SectionCard` with four `QuickControl`s: Profile, CPU Fan, GPU Fan, and Charge Limit.

`SectionCard` supplies the rounded, translucent background and subtle border. `MonitorRow` renders an icon, label, temperature, optional secondary/tertiary values, and fan value. The dGPU row can show power in watts; unavailable/negative dGPU temperature is muted. `BatteryBar` shows percentage, charging/discharging watts when available, an animated fill bar, and a charge-limit label when the limit is not 100%.

`Gauge.qml` draws a 270-degree animated arc. In `main.qml`, the gauges use a 20–100 °C range and an 80-pixel size. `Sparkline.qml` supports up to 30 points and normalizes its polyline against the current sample range. `MetricCard.qml` can combine a value with a sparkline, but neither `MetricCard` nor `Sparkline` is currently instantiated by `main.qml`; do not describe them as visible in the current expanded layout.

## Polling and data contract

`main.qml` uses Plasma's executable `DataSource` to run [`legion-poll.sh`](../kde-widget/package/contents/ui/legion-poll.sh). The polling interval is `RefreshInterval * 1000`; the default is 2 seconds and the configuration restricts it to 1–10 seconds.

The poller searches for an executable `legion-cli` in this order:

```text
/usr/local/bin/legion-cli
/usr/bin/legion-cli
$HOME/.local/bin/legion-cli
```

It runs `legion-cli status`, `fan`, `profile`, `kbd`, and `logo`. Battery values are read directly from the first `/sys/class/power_supply/BAT*` directory found. The helper prints newline-delimited `KEY=value` records, which `main.qml` parses.

### Records consumed by `main.qml`

| Record | Current use |
| --- | --- |
| `LEGION_OK=1` | Marks the daemon/CLI poll as successful. |
| `LEGION_DAEMON_OFFLINE=1` | Marks the poll as offline. |
| `LEGION_CLI_NOT_FOUND=1` | Marks the poll as offline. |
| `CPU_TEMP` | CPU temperature, compact display, CPU gauge, and CPU row. |
| `IGPU_TEMP` | Parsed by the helper but not handled by `main.qml`. |
| `DGPU_TEMP` | dGPU gauge and dGPU row. |
| `DGPU_POWER` | dGPU row secondary value in watts. |
| `FAN_CPU` | CPU row and CPU Fan control. `0` is displayed as `Auto`. |
| `FAN_GPU` | dGPU row and GPU Fan control. `0` is displayed as `Auto`. |
| `FAN_AUX` | Stored in `fanAux`; not rendered. |
| `BATTERY` | Battery percentage. |
| `BAT_STATUS` | Battery label and charging/discharging presentation. |
| `CHARGE_LIMIT` | Battery charge-limit label and Charge Limit control. `100` is stored as an empty display value. |
| `BAT_POWER` | Charging/discharging watts. |
| `PROFILE` | Compact tooltip and Profile control. |
| `KBD_BRIGHTNESS` | Stored in `kbdBrightness`; not rendered. |
| `LOGO` | Stored in `logoOn`; not rendered. |

The poller extracts CPU temperature from a `Tctl` line, dGPU temperature/power from lines containing `dGPU`, and fan values from lines such as `CPU fan: <RPM>`. `legion-info.sh` performs a one-shot `legion-cli info` call and supplies `CPU_NAME` and `GPU_NAME`; `main.qml` disconnects that source after receiving the result.

After a control write, `main.qml` suppresses selected polled values for 2.5 seconds to avoid immediately replacing the requested state with stale output. It schedules an additional one-shot poll after 800 ms; this does not restart or replace the regular refresh timer. CPU temperature history retains at most 30 samples, although the current visible layout does not attach that history to a `Sparkline`.

## Controls

The visible controls and their exact cycling behavior are implemented in `main.qml`:

- **Profile** cycles `quiet`, `balanced`, `performance`, `max-power`, and `custom`, then runs `legion-cli set-profile <profile>`.
- **CPU Fan** cycles `0`, `3000`, `3500`, `4000`, and `4500`, then runs `legion-cli set-fan 1 <rpm>`.
- **GPU Fan** uses the same RPM presets and runs `legion-cli set-fan 2 <rpm>`.
- **Charge Limit** cycles `100`, `80`, and `60`, then runs `legion-cli charge-limit <percentage>`.

The widget invokes [`legion-command.sh`](../kde-widget/package/contents/ui/legion-command.sh) through `bash`. That helper looks for the same three `legion-cli` locations and exits with status 127 if it cannot find the CLI. The CLI command definitions, including fan IDs 1 (CPU), 2 (GPU), and 4 (Aux), are in [`src/cli/main.rs`](../src/cli/main.rs).

Although the poller reads keyboard brightness and logo state, the current `main.qml` does not render controls for them. [`legion-settings.sh`](../kde-widget/package/contents/ui/legion-settings.sh) exists in the package but is not invoked by `main.qml`.

## Configuration

The configuration model is [`kde-widget/package/contents/config/main.xml`](../kde-widget/package/contents/config/main.xml). [`config.qml`](../kde-widget/package/contents/config/config.qml) exposes one **General** category backed by [`configGeneral.qml`](../kde-widget/package/contents/config/configGeneral.qml).

| Setting | Type | Default | Effect |
| --- | --- | ---: | --- |
| `RefreshInterval` | `Int` | `2` | Polling interval in seconds, limited to 1–10. |
| `ShowGauges` | `Bool` | `true` | Shows or hides the circular temperature gauges. |
| `ShowSparklines` | `Bool` | `true` | Exposed in the configuration UI and root properties, but no current visible `Sparkline` instance is wired to it. |

The configuration page contains a refresh-interval spin box, checkboxes for circular gauges and the CPU temperature sparkline, and states that changes apply after closing the configuration dialog.

## Install, update, and remove

### Direct widget script

From the repository's widget directory:

```bash
cd kde-widget
chmod +x install.sh
./install.sh
```

[`kde-widget/install.sh`](../kde-widget/install.sh) tries:

```bash
kpackagetool6 --type Plasma/Applet -i kde-widget/package
```

The script uses the package path relative to its own directory at runtime. If installation fails, it retries with `-u` and reports an update. It does not restart Plasma; add **Legion Control** from Plasma's widget picker afterward.

To remove the package:

```bash
cd kde-widget
./uninstall.sh
```

The equivalent package command is:

```bash
kpackagetool6 --type Plasma/Applet -r com.github.encomjp.legioncontrol
```

The uninstall script reports an error if that package is not installed. Removing it also does not restart Plasma.

### Top-level installer

The repository installer exposes the optional widget operation:

```bash
./install.sh --widget
```

The `--widget` path in [`install.sh`](../install.sh) requires `kpackagetool6`, installs or updates `kde-widget/package`, and prints that the widget can be added from Plasma's widget picker. When the top-level installer is run as root, its source handles KPackage execution as the desktop user represented by `$SUDO_USER`.

### CMake installation

The CMake file has a package-install rule for the KDE data directory. With ECM and KF6 `Package` available, the source-defined build sequence is:

```bash
cd kde-widget
cmake -B build
cmake --build build
cmake --install build
```

The repository does not set a project-specific install prefix in `kde-widget/CMakeLists.txt`; use the KDE/CMake installation environment appropriate for the target system if the default prefix is not desired.

## Linting and validation

Run QML lint against the widget and configuration QML files:

```bash
cd kde-widget
qmllint package/contents/ui/*.qml package/contents/config/*.qml
```

The command was run in this checkout and returned exit status 0. It checks QML syntax and lint diagnostics; it does not prove that the widget can run inside a live Plasma shell, that `legion-cli` is installed, or that the daemon's output matches the poller's parsing expressions.

The package metadata is in [`kde-widget/package/metadata.json`](../kde-widget/package/metadata.json), including the package name `Legion Control` and plugin `com.github.encomjp.legioncontrol`.

## Debugging

First test the helper scripts independently of Plasma:

```bash
cd kde-widget
bash package/contents/ui/legion-poll.sh
bash package/contents/ui/legion-info.sh
bash package/contents/ui/legion-command.sh status
```

The poller prints the key/value contract described above. Empty output from `legion-cli status`, a missing CLI, or missing optional sensors can result in offline or incomplete values by design.

Check the daemon and CLI directly:

```bash
systemctl status legion-control
journalctl -u legion-control -e
legion-cli status
legion-cli fan
legion-cli profile
```

The widget has no project-specific debug switch and no `console.log()` calls. Runtime failures are primarily represented by `daemonOnline` becoming false and the compact tooltip showing `Daemon offline`. `main.qml` runs the helper through Plasma's executable `DataSource`, so standalone QML execution is not equivalent to running the applet in Plasma.

## Current verification limits

- `qmllint` was available and passed in this checkout; package metadata is available in `kde-widget/package/metadata.json`.
- Interactive Plasma testing was not performed here; a Plasma shell/widget picker and the `legion-cli`/daemon environment are required to verify live behavior.
- The visible implementation is the authority for the current UI. In particular, README-level claims about keyboard-brightness/logo controls or sparklines should not be used to infer controls that are absent from `main.qml`.

All paths and behavior in this guide are based on the files under [`kde-widget/`](../kde-widget/) and the CLI definitions in [`src/cli/main.rs`](../src/cli/main.rs).
