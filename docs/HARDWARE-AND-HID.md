# Hardware and HID

This guide follows a user action to the Linux interface and, where the source establishes it, to the laptop hardware. It describes the interfaces implemented by Legion Control; it does not imply that every Legion model exposes the same interfaces.

## 1. What communicates with what

Legion Control has several clients and hardware paths:

- `legion-settings` is the GTK4/libadwaita desktop application.
- `legion-cli` is the command-line client.
- The optional KDE Plasma widget runs `legion-cli` and reads battery data directly from `/sys/class/power_supply/BAT*`.
- `legion-daemon` is normally the root `legion-control` system service.
- The daemon aggregates Linux interfaces including sysfs, hwmon, WMI-backed hwmon, NVIDIA telemetry, and HID. Selected RGB operations instead use a direct HID path from the CLI or GUI.

The overview diagram shows these boundaries: [SVG](assets/legion-control-overview.svg), [PNG](assets/legion-control-overview.png), and [DOT source](assets/legion-control-overview.dot). The optional `legion-control-setup` helper performs narrowly scoped setup actions through PolicyKit; it is not part of the normal daemon command path.

## 2. The normal control path through the daemon

For daemon-mediated operations, GUI and CLI clients serialize `DaemonCommand` values with `bincode` and send them over a Unix stream. The normal root-daemon socket is `/run/legion-control.socket`; the daemon serializes a `DaemonResponse` and sends it back over the same connection. The client writes one command, shuts down its write side, and the daemon reads until EOF before dispatching the command.

The daemon normally runs as a system service because profile, fan, conservation, firmware, and other hardware writes can require privileges. A non-root daemon can use a per-user runtime socket, but the source warns that the principal profile, fan, and conservation writes are expected to fail in that mode. The [Architecture Guide](ARCHITECTURE.md) documents socket selection, command boundaries, and permissions in more detail.

The socket is an IPC transport, not a general hardware protocol. The repository does not define a version field or capability negotiation; compatibility relies on preserving the existing serialized enum order and appending new variants.

## 3. Monitoring paths

The daemon combines readings from the interfaces that are present on the machine:

- **sysfs and hwmon:** sensor discovery reads `/sys/class/hwmon/*/name` and aggregates supported sources such as `k10temp`, `legion_hwmon`, `amdgpu`, `nvme`, `spd5118`, `iwlwifi_1`, and `r8169` variants.
- **WMI-backed hwmon:** fan discovery prefers `lenovo_wmi_other` and falls back to `legion_hwmon`. Fan readings and targets use hwmon files such as `fan1_input` and `fan1_target`; a target of `0` selects automatic mode.
- **Battery and platform profile:** battery data comes primarily from `/sys/class/power_supply/BAT0`; platform profile readings use Linux platform-profile paths.
- **Optional NVIDIA telemetry:** when available, the daemon invokes the absolute path `/usr/bin/nvidia-smi` for dGPU name, temperature, power, clocks, utilization, and power-limit data. Each call has a three-second response timeout. A sleeping or unavailable dGPU can therefore produce an unavailable value rather than a live measurement.
- **Optional AMD Curve Optimizer path:** when the optional `ryzen_smu` interface is installed and the project’s strict target-hardware checks pass, the daemon can use `/sys/kernel/ryzen_smu_drv` and its related files. This is an opt-in, hardware-gated path, not a requirement for ordinary telemetry or RGB.

The [Usage Guide](USAGE.md) lists the user-facing monitoring and control commands. The [Architecture Guide](ARCHITECTURE.md) provides the source map and the exact backend paths.

## 4. RGB and HID lighting

The Spectrum implementation in [`src/keyboard.rs`](../src/keyboard.rs) scans `/sys/class/hidraw`, follows the device links, and matches vendor ID `048d` and product ID `c197`. Where available, it also checks the report descriptor for the Spectrum usage. It then opens the matching `/dev/hidrawN` node read/write and sends HID feature reports through ioctl.

The source documents these transport facts for the Spectrum path:

- feature reports are 960 bytes;
- the report ID is `0x07`;
- Spectrum access is serialized with a process-local mutex;
- the implementation covers effects, zones, per-key maps, brightness, and logo controls.

The complete field-level report layout is intentionally **not** documented here. The repository source establishes the report size, report ID, device matching, and access behavior, but does not establish a complete packet-field or checksum specification suitable for a general hardware guide. Do not infer that layout from these transport facts.

The HID lighting flow is available as [SVG](assets/hid-lighting-flow.svg), [PNG](assets/hid-lighting-flow.png), and [DOT source](assets/hid-lighting-flow.dot); it shows the direct path to `hidraw`. The udev rule matches `048d:c193` and `048d:c197`, assigning mode `0666` and the `uaccess` tag. This enables the project’s direct non-root RGB path, while also making matching device nodes broadly accessible; treat the rule as a deliberate local security trade-off.

## 5. What `hidraw` means

`hidraw` is Linux’s raw HID character-device interface. A node such as `/dev/hidraw3` represents one HID interface exposed by the kernel. Opening it does not mean that the device is a generic RGB device, nor does the node name identify a stable physical device: the number can change between boots or device enumeration events.

For this project, discovery starts from `/sys/class/hidraw` and then selects the matching Spectrum interface before opening its `/dev/hidrawN` node. Checking `lsusb` alone identifies USB IDs but does not prove that the matching hidraw interface is present, accessible, or answering feature-report requests.

## 6. Standard Linux behavior versus project-specific behavior

**Standard Linux behavior:** sysfs, hwmon, power-supply, platform-profile, `nvidia-smi`, udev, systemd, and hidraw are operating-system interfaces. Their presence and permissions depend on the kernel, drivers, firmware, distribution, service configuration, and the hardware that the host exposes.

**Project-specific behavior:** Legion Control chooses particular paths and names, matches Spectrum VID/PID `048d:c197`, uses 960-byte feature reports with report ID `0x07`, serializes Spectrum access within a process, and maps daemon commands to its Rust hardware modules. Its udev rule covers `048d:c193` and `048d:c197`; that is not a claim that every Lenovo lighting controller uses either ID or the same protocol.

Likewise, the project’s optional `nvidia-smi` and `ryzen_smu` integrations are conditional paths. An absent interface is not evidence that Linux or the laptop is universally unsupported, and a present interface is not by itself proof that every project write operation is safe or enabled.

## 7. Hardware support boundaries

The repository verifies the Lenovo Legion Pro 7 16AFR10H (machine type `83RU`) with a Gen 10 Spectrum RGB keyboard (`048d:c197`). Other Gen 10 Legion models are described as likely-compatible but are not verified by this project; older generations use different RGB protocols. Hardware interfaces vary by model, firmware, kernel, drivers, and installation method, so unavailable capabilities should be treated as unavailable rather than assumed.

Before expecting Spectrum support, inspect the controller:

```bash
lsusb | grep 048d
```

`048d:c197` is the Gen 10 Spectrum device used by the implementation. `048d:c193` is also covered by the udev rule as Lenovo Lighting, but the Spectrum report details in this guide apply only to the `c197` implementation. The project’s AMD Curve Optimizer write path is narrower still: source documentation describes strict checks for the validated target rather than a universal AMD capability.

## 8. Debugging checklist

Start with read-only checks and capture the output before changing hardware state:

```bash
systemctl status legion-control
legion-cli status
legion-cli info
lsusb | grep 048d
ls -l /dev/hidraw*
ls -l /run/legion-control.socket
journalctl -u legion-control -n 50 --no-pager
```

If RGB is missing or denied:

```bash
for hid in /dev/hidraw*; do
    [ -e "$hid" ] || continue
    udevadm info --query=property --name="$hid" 2>/dev/null \
        | grep -E 'ID_VENDOR_ID|ID_MODEL_ID|ID_SERIAL' \
        && printf 'node: %s\n' "$hid"
done
legion-cli rgb-status
journalctl -k -b --no-pager | grep -iE 'hid|usb|048d|spectrum'
```

If the rule was newly installed, reload and retrigger it as documented by the project:

```bash
sudo udevadm control --reload-rules
sudo udevadm trigger -s hidraw
```

If the daemon is active but writes fail, verify that the system service is the selected installation and that it is running with the expected privileges. If `/usr/bin/nvidia-smi` is relevant, check it directly; if Curve Optimizer is relevant, first confirm that `/sys/kernel/ryzen_smu_drv` exists. For symptom-specific remedies, use [Troubleshooting](TROUBLESHOOTING.md) rather than guessing a packet format or forcing a write on an unverified machine.

## 9. Related documentation

- [Installation](INSTALLATION.md) — source and package installation, service setup, udev rules, and optional `ryzen_smu`.
- [Usage](USAGE.md) — CLI, GUI, monitoring, cooling, battery, lighting, diagnostics, and safety notes.
- [Architecture](ARCHITECTURE.md) — components, IPC, persistence, permissions, data flow, and implementation caveats.
- [Widget](WIDGET.md) — KDE Plasma polling, direct battery sysfs reads, controls, and validation.
- [Troubleshooting](TROUBLESHOOTING.md) — evidence-first daemon, HID, sensor, dGPU, battery, and RGB recovery checks.
- [Development](DEVELOPMENT.md) — source map, hardware-sensitive testing, and contribution boundaries.
- [Overview diagram source](assets/legion-control-overview.dot) and [HID lighting diagram](assets/hid-lighting-flow.svg) — visual summaries of the documented paths.

For source-level details, start with [`src/comms.rs`](../src/comms.rs), [`src/daemon/main.rs`](../src/daemon/main.rs), [`src/keyboard.rs`](../src/keyboard.rs), [`src/sensors.rs`](../src/sensors.rs), [`src/dgpu.rs`](../src/dgpu.rs), [`src/undervolt.rs`](../src/undervolt.rs), and [`data/udev/99-legion.rules`](../data/udev/99-legion.rules).
