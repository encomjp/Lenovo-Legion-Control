# Embedded Controller (EC) Sensor Documentation

> Related: [EC-REGISTER-MAP.md](EC-REGISTER-MAP.md) · [RESEARCH-TOOLING.md](RESEARCH-TOOLING.md) · [sensor-research-findings.md](sensor-research-findings.md)

## Lenovo Legion Pro 7 16AFR10H (83RU) — BIOS SMCN20WW — ITE IT5508

---

## EC Architecture

The ITE IT5508 Embedded Controller manages sensors, fans, power, and
keyboard backlight. It has two addressable memory regions:

| Region | Size | Access Method | Contents |
|--------|------|---------------|----------|
| ACPI Space | 256 B (0x00-0xFF) | Port I/O or ec_sys | Temps, battery, status, flags |
| Extended RAM | ~1.5 KB (0xC400+) | MMIO at 0xFE00D400 | Fan RPM, fan curve, power limits |

### Block Diagram
```
CPU Host
  │
  ├─ Port I/O 0x62/0x66 ──► EC ACPI Space (0x00-0xFF)
  │                           ├─ Temps (CPU, GPU, board, chipset)
  │                           ├─ Battery info
  │                           ├─ Status/control flags
  │                           └─ Power mode
  │
  ├─ MMIO 0xFE00D400 ────► EC Extended RAM (0xC400+)
  │    [BLOCKED by            ├─ Fan RPM (3 fans)
  │     STRICT_DEVMEM]        ├─ Fan curve (10-point)
  │                           ├─ CPU/GPU power limits
  │                           └─ Overclock settings
  │
  ├─ WMI (GUID 887B54E3) ──► WMI3 Methods
  │    [needs kernel driver]  ├─ GetCPUTemp (method 18)
  │                           ├─ GetGPUTemp (method 19)
  │                           ├─ GetFanSpeed (fan RPM)
  │                           └─ SetSmartFanMode
  │
  └─ USB HID (048d:c197) ──► Keyboard RGB + EC control
       /dev/hidraw12           └─ 960-byte feature reports
```

---

## Confirmed EC Register Map (ACPI Space 0x00-0xFF)

Discovered via cross-referencing `/dev/port` EC reads with known-good
hwmon/NVIDIA sensors under CPU load.

```
Offset  Size  Name              Confirmed  Notes
─────────────────────────────────────────────────────
0x00    1     EC status          ✓         Bitfield (ACPI protocol)
0x03    1     PCH/ambient temp   ✓         ~21-27°C, changes under load
0x04    1     RAM area temp      ✓         ~29-35°C, changes under load
0x06    1     Unknown temp       ?         ~24-26°C, slight changes
0x0E    1     Unknown            ?         0x1D (=29) relatively stable
0x0F    1     Board temp         ✓         ~47°C, very stable
0x30-0x31 2   Battery current?   ?         Changes with charge state
0x50-0x53 4   Fan PWM?           ?         Changes between power modes
0x80-0x8E —   Battery strings    ✓         "SMP 2024 L24M4PC1" at 0x90
0xB0    1     **CPU temperature** ✓         Matches k10temp Tctl (±1°C)
0xB1    1     Chipset/IC temp    ✓         ~41-42°C, stable
0xB3    1     Unknown temp       ?         ~45°C, very stable
0xB4    1     **dGPU temperature** ✓        Matches nvidia-smi GPU temp
0xC0    1     Battery max power? ?         0x70=112, stable
0xC3    1     Battery temp?      ?         0x27=39, matches SPD RAM temp
0xC4    1     Unknown            ?         0x18=24
0xC7    1     Unknown            ?         ~67°C (sometimes matches Tccd)
0xC9    1     Unknown            ?         ~60°C
0xCE    1     Unknown            ?         0x64=100 (constant)
0xF0    1     Power mode?        ?         0x03 = performance mode?
0xF5    1     Unknown status     ?         Changes
```

### Verification Data
```
Tctl=66.5°C → EC[0xB0]=66  ✓ (CPU temp confirmed)
dGPU=51°C   → EC[0xB4]=50  ✓ (dGPU temp confirmed)

Constant values at idle:
  EC[0x0F]=47  (board temp)
  EC[0xB1]=41  (chipset temp)
  EC[0xB3]=45  (unknown, possibly VRM)
  EC[0x03]=21  (cold area)
  EC[0x04]=29  (RAM area)
```

---

## Access Methods Comparison

### Method 1: ec_sys (built-in kernel module) — RECOMMENDED
```bash
sudo modprobe ec_sys write_support=0
xxd -l 256 /sys/kernel/debug/ec/ec0/io
```
- Pros: Kernel-managed locking, safe concurrent access
- Cons: 256-byte limit, needs debugfs mounted

> hwmon/hidraw indices (`hwmon7`, `hidraw12`, …) in this file are boot-specific
> examples — enumerate by `name`, never hardcode the index.

### Method 2: /dev/port (raw I/O) — HACKY
```bash
# Handshake: write 0x80 to port 0x66, write addr to port 0x62, read 0x62
```
- Pros: Works without loading any module
- Cons: Bypasses kernel locking, possible race with ACPI EC driver

### Method 3: /dev/mem (MMIO) — BLOCKED
```bash
# EC RAM at 0xFE00D400 — returns 0xFF (blocked by STRICT_DEVMEM)
# ECB2 at 0xFF00D520 — works (ACPI region, not DRAM)
xxd -s 0xFF00D520 -l 256 /dev/mem
```
- Pros: Could access full EC RAM (if not blocked)
- Cons: CONFIG_STRICT_DEVMEM=y blocks DRAM access

### Method 4: WMI3 (needs kernel driver)
- Gamezone GUID `887B54E3-DDDC-4B2C-8B88-68A26A8835D0`
- Methods 18 (GetCPUTemp), 19 (GetGPUTemp), fan speed via OtherModeFeature
- Upstream `lenovo_wmi_gamezone` only does platform profiles, not temps
- ChaoticSi1ence fork exposes everything via hwmon

---

## How to Bypass STRICT_DEVMEM (for Extended EC RAM)

### Option A: iomem=relaxed (reboot, safe)
Add to kernel cmdline in `/etc/default/grub`:
```
GRUB_CMDLINE_LINUX="... iomem=relaxed"
```
Then: `sudo grub-mkconfig -o /boot/grub/grub.cfg && reboot`
This allows `/dev/mem` access to I/O memory regions (not DRAM).

### Option B: kretprobe bypass (needs one-time module)
Tiny kernel module that hooks `devmem_is_allowed()` to always return 1.
Then `/dev/mem` can read 0xFE00D400 directly.

### Option C: Rebuild kernel
`CONFIG_STRICT_DEVMEM=n` — no restrictions on /dev/mem at all.

### Option D: ec_sys with expanded size
Patch ec_sys.c to increase EC_SPACE_SIZE beyond 256 (the EC protocol
supports it via burst mode for multi-byte reads). Requires kernel rebuild.

---

## Fan RPM (Extended RAM Only)

Fan RPM lives in the extended EC RAM (0xC400+ region), NOT in the
ACPI 256-byte space. On Gen 10 (SMCN/ITE IT5508), this data is read
via WMI3 method `OtherMethodFeature_FAN_SPEED` rather than direct EC
memory access.

The upstream kernel's `lenovo_wmi_other` driver handles this correctly
— it exposes fan1/2/4 RPM and writable targets via hwmon. The hwmon
paths are:
```
/sys/class/hwmon/hwmon7/fan1_input   # CPU fan RPM
/sys/class/hwmon/hwmon7/fan2_input   # GPU fan RPM
/sys/class/hwmon/hwmon7/fan4_input   # Auxiliary fan RPM
/sys/class/hwmon/hwmon7/fan1_target  # Target RPM (0=auto)
```

These work fine already — no EC hacking needed for fan RPM.

---

## Keyboard RGB (USB HID)

The per-key RGB backlight uses USB HID device 048d:c197 (hidraw instance
varies per boot). Two report sizes on the same device:

- **960-byte feature reports** (report ID `0x07`) — full effect packets /
  per-zone control. See `SPECTRUM-ZONES.md` and production
  `lenovo-legion-tool/src/keyboard.rs`.
- **33-byte reports** — lighting-state query/set (`0xCC` sub-command shape):

```
byte 0: report ID 0xCC
byte 1: 0x16      # sub-command
byte 2: effect    # 0x01=static, 0x03=breath, 0x04=wave, 0x06=smooth
byte 3: speed     # 1-4
byte 4: brightness # 1-2
bytes 5-16: rgb   # 12 bytes for 4 zones (RGB x4)
```

White keyboard backlight (on/off/brightness levels 0/1/2) uses WMI,
already exposed at `/sys/class/leds/platform::kbd_backlight/brightness`.

---

## Summary

| Sensor | Accessible? | Method | Path |
|--------|------------|--------|------|
| EC CPU temp | **YES** | ec_sys or /dev/port | EC[0xB0] |
| EC dGPU temp | **YES** | ec_sys or /dev/port | EC[0xB4] |
| EC board temp | **YES** | ec_sys or /dev/port | EC[0x0F] |
| EC chipset temp | **YES** | ec_sys or /dev/port | EC[0xB1] |
| EC PCH temp | **YES** | ec_sys or /dev/port | EC[0x03] |
| Fan RPM | **YES** | Existing hwmon | /sys/class/hwmon/hwmon7/fan*_input |
| Fan targets | **YES** | Existing hwmon | /sys/class/hwmon/hwmon7/fan*_target |
| Fan curve | NO | Needs kernel module | EC F9F0-F9F9 / WMI3 SET_TABLE |
| CPU power limits | NO | Needs kernel module | Capdata01 / WMI |
| VRM temps | LIKELY NO | Extended EC RAM | Not in 0x00-0xFF space |
| GPU VRAM temp | NO | Not exposed by EC or NVIDIA | — |
| Keyboard RGB | **YES** | USB HID /dev/hidraw | 33-byte feature reports |
