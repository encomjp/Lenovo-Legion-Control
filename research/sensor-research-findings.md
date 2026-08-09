# Legion Pro 7 16AFR10H (83RU) — Sensor Research

## System Identity
- Model: Lenovo Legion Pro 7 16AFR10H (83RU)
- BIOS: SMCN20WW
- EC: ITE IT5508 (chip ID 0x5508)
- EC physical memory: `0xFE00D400` (via ACPI ERAM/ECB2)
- EC I/O ports: `0x62` (data), `0x66` (cmd)
- Platform: AMD Granite Ridge (Ryzen 9 9955HX3D + RTX 5080)

---

## What We Already Have (no external programs needed)

These are working via built-in kernel drivers:

| Sensor | Source | Path | Driver |
|--------|--------|------|--------|
| CPU Tctl/Tccd1/Tccd2 | PCI | hwmon5 (`k10temp`) | k10temp |
| iGPU edge temp | PCI | hwmon4 (`amdgpu`) | amdgpu |
| iGPU vddgfx/vddnb | PCI | hwmon4 | amdgpu |
| iGPU PPT power | PCI | hwmon4 | amdgpu |
| dGPU temp/power/clocks | PCI | nvidia-smi only | nvidia |
| NVMe temps (x2) | PCI | hwmon2, hwmon3 | nvme |
| DDR5 SPD temps (x2) | I2C | hwmon8, hwmon9 | spd5118 |
| Eth NIC temp | MDIO | hwmon10 | r8169 |
| WiFi temp | thermal | hwmon11 | iwlwifi |
| Fan RPM (fan1/2/4) | WMI | hwmon7 | lenovo_wmi_other |
| Fan target RPM | WMI | hwmon7 (writable) | lenovo_wmi_other |
| Battery V/capacity | ACPI | hwmon1 | ACPI battery |
| Platform profile | WMI | `/sys/firmware/acpi/platform_profile` | lenovo_wmi_gamezone |
| Conservation mode | ACPI | `/sys/class/power_supply/BAT0/charge_types` | ideapad ACPI |

Platform profiles available: `low-power balanced performance max-power custom`

---

## What We DON'T Have

### Missing Sensors (EC-internal, not exposed by upstream kernel)

| Sensor | Where it lives | Method to get it |
|--------|---------------|-----------------|
| **CPU temp (EC)** | EC registers | WMI3 GetCPUTemp / EC memory |
| **GPU temp (EC)** | EC registers | WMI3 GetGPUTemp / EC memory |
| **IC/board/chipset temp** | EC registers | EC memory read only |
| **VRM temps** | EC registers | EC memory read only |
| **GPU memory junction temp** | EC / GPU PMU | Not exposed by nvidia |
| **CPU power/current (PPT)** | WMI capdata01 | Needs capdata01 binding |
| **Fan curve (10-point)** | EC F9F0-F9F9 | WMI3 SET_TABLE / EC memory |
| **RGB keyboard state** | USB HID | HID feature report (048d:c197) |

---

## How To Get The Missing Sensors

### Method A: ChaoticSi1ence's legion-laptop Fork (Kernel Module)

The most complete solution. Already has `model_smcn` config for 83RU.

```bash
git clone https://github.com/ChaoticSi1ence/LenovoLegionLinux.git
cd LenovoLegionLinux/kernel_module
make
sudo insmod legion-laptop.ko wmi_dryrun=1
```

Provides via hwmon:
- CPU temp (EC reading, different from k10temp Tctl)
- GPU temp (EC reading)
- IC/board temp
- Fan 1/2/3 RPM and PWM curve points
- Full fan curve read/write via sysfs

**This is a kernel module, not an external program.** It reads EC via WMI3 and exposes via standard hwmon sysfs.

WARNING: Only use on kernels < 7.0, or manually blacklist `lenovo_wmi_*` first (they conflict for GUID `887B54E3`).

### Method B: Raw WMI Method Calls (Userspace via ACPI)

The Gamezone WMI GUID `887B54E3-DDDC-4B2C-8B88-68A26A8835D0` has these methods per the MOF:

| Method ID | Name | What |
|-----------|------|------|
| 18 | GetCPUTemp | EC CPU temperature (uint32) |
| 19 | GetGPUTemp | EC GPU temperature (uint32) |
| 43 | IsSupportSmartFan | Check fan curve support |
| 44 | SetSmartFanMode | Set thermal mode |
| 45 | GetSmartFanMode | Get current thermal mode |

To call these from userspace without a custom driver, you'd need either:
- `acpi_call` kernel module (not built-in) → `echo '\_SB_.GZFD.WMBD 18' > /proc/acpi/call`
- A small kernel driver calling `wmidev_evaluate_method()`

**Pure userspace call is NOT possible** — Linux WMI subsystem doesn't expose a char device for arbitrary method calls. You need a kernel driver.

### Method C: Direct EC Memory Read (Kernel Module)

The EC maps to physical CPU memory at `0xFE00D400`:
```bash
# Load ec_sys module to expose /sys/kernel/debug/ec/
sudo modprobe ec_sys write_support=0
xxd /sys/kernel/debug/ec/ec0/io | head -50
```

But ec_sys is not built-in, needs module load. And you'd need to reverse-engineer register offsets.

### Method D: USB HID for Keyboard

The per-key RGB keyboard (048d:c197) is pure USB HID:
- 33-byte HID feature reports for lighting state (read and write)
- 960-byte reports for full per-zone control
- No kernel driver needed — can use `/dev/hidraw*` directly:
```bash
ls /dev/hidraw*
# Read with: xxd /dev/hidrawX
```

But the keyboard backlight on/off/brightness is separate — that's the WMI white backlight (`platform::kbd_backlight` LED class), not the RGB zones.

---

## Current Upstream Kernel Status (7.1.4)

The following upstream Lenovo WMI drivers are loaded and bound:

| Driver | GUID | Status |
|--------|------|--------|
| lenovo_wmi_gamezone | 887B54E3-...-D0 | **Bound** — provides platform_profile |
| lenovo_wmi_other | DC2A8805-...-3B-7 | **Bound** — provides fan RPM hwmon |
| lenovo_wmi_capdata | 362A3AFE-...-E-17 | **Bound** — feeds data to other |
| lenovo_wmi_capdata | 7A8F5407-...-4-13 | **Bound** — capdata01 for PPT |
| lenovo_wmi_capdata | B642801B-...-1-19 | **Bound** — fan min/max data |
| lenovo_wmi_events | D320289E-...-F-21..24 | **Bound** — handles Fn+Q events |
| lenovo_wmi_hotkey_utilities | 362A3AFE-...-E-17 | **Loaded** — hotkey functions |

**BUT:** The upstream gamezone driver only exposes platform profiles — it does NOT expose GetCPUTemp/GetGPUTemp to hwmon. The upstream other driver exposes fan RPM but firmware_attributes directory is empty (no supported PPT attributes for this model).

---

## Conclusion

- **Pure userspace sysfs/HID** already covers: CPU/iGPU/dGPU/NVMe/RAM/NIC/WiFi temps, fan RPM, fan targets, battery, platform profiles
- **CPU/GPU/IC temps from EC** require a kernel driver — either the ChaoticSi1ence fork or adding `GetCPUTemp`/`GetGPUTemp` to the upstream `lenovo_wmi_gamezone`
- **No way to read EC WMI methods from pure userspace** — Linux WMI subsystem requires kernel drivers for method calls
- **Keyboard RGB** is the one truly "pure HID" path — `/dev/hidraw` can read/write directly
