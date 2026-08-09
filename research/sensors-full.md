# Lenovo Legion 83RU — Sensor Map (Full)

## System
- Model: Lenovo 83RU (Legion series)
- BIOS: LNVNB161216
- Kernel: 7.1.4-1-cachyos-bore
- OS: CachyOS (Arch-based)

---

## HWMON Devices

| hwmon | Name | Path | Type |
|-------|------|------|------|
| hwmon0 | ADP0 | platform/ACPI0003:00/power_supply/ADP0 | AC Adapter |
| hwmon1 | BAT0 | platform/.../power_supply/BAT0 | Battery |
| hwmon2 | nvme | pci/0000:03:00.0/nvme/nvme1 | NVMe (Nextorage 2TB) |
| hwmon3 | nvme | pci/0000:02:00.0/nvme/nvme0 | NVMe (Samsung 1TB) |
| hwmon4 | amdgpu | pci/0000:08:00.0 | AMD iGPU |
| hwmon5 | k10temp | pci/0000:00:18.3 | CPU temp |
| hwmon6 | yogafan | platform/yogafan | Fan (legacy) |
| hwmon7 | **lenovo_wmi_other** | wmi/.../DC2A8805-...CD3B-7 | **WMI Fan Control** |
| hwmon8 | spd5118 | i2c-10/10-0050 | DDR5 SPD temp |
| hwmon9 | spd5118 | i2c-10/10-0051 | DDR5 SPD temp |
| hwmon10 | r8169 | mdio/r8169-0-700:00 | Ethernet NIC temp |
| hwmon11 | iwlwifi_1 | virtual/thermal/thermal_zone0 | WiFi temp |
| hwmon12-14 | ucsi_source_psy_* | platform/USBC000:00 | USB-C PD sources |
| hwmon15 | hidpp_battery_0 | usb1/.../046D:C53A | Logitech mouse battery |

---

## Sensor Details

### CPU Temperature (k10temp / hwmon5)
- **Tctl** (temp1): CPU control temperature (~57°C idle)
- **Tccd1** (temp3): CCD1 temperature (~52°C)
- **Tccd2** (temp4): CCD2 temperature (~54°C)

### AMD iGPU (amdgpu / hwmon4)
- **vddgfx** (in0): GPU core voltage (1.31V)
- **vddnb** (in1): NB voltage (0.844V)
- **edge** (temp1): GPU edge temp (51°C)
- **PPT** (power1): GPU power draw (14mW idle, scales)
- **sclk** (freq1): GPU clock (600MHz idle)

### NVIDIA RTX 5080 Max-Q (nvidia-smi only, no hwmon)
- **GPU Temp**: 45°C idle
- **Power Draw**: ~17W avg
- **Power Limit**: 175W max
- **Clocks**: Graphics 337MHz / Memory 810MHz (idle)
- **Max Clocks**: 3090MHz GPU / 14001MHz Memory

### NVMe Temperatures
- **Samsung PM9C1a** (hwmon3): Composite ~34°C, Sensor1 ~37°, Sensor2 ~34°
- **Nextorage NE1N** (hwmon2): Composite ~39°C

### WMI Fan Control (lenovo_wmi_other / hwmon7)
**THIS IS THE KEY DEVICE FOR FAN CONTROL**

| Attribute | Fan1 | Fan2 | Fan4 |
|-----------|------|------|------|
| Input (current) | 1800 RPM | 1800 RPM | 2500 RPM |
| Min | 1700 RPM | 1700 RPM | 1500 RPM |
| Max | 5200 RPM | 5400 RPM | 6500 RPM |
| Target | 0 | 0 | 0 |
| Divider | 100 | 100 | 100 |

> **fan1_target, fan2_target, fan4_target are WRITABLE** — setting them controls fan speed (0 = auto).

### Other Temps
- **DDR5 SPD** (hwmon8/hwmon9): ~39–43°C (RAM modules via I2C)
- **Ethernet** (hwmon10): 35°C (RTL8125)
- **WiFi** (hwmon11): 32°C (Intel AX210)

### Battery (BAT0 / hwmon1)
- Model: SMP L24M4PC1 (Li-poly)
- Design capacity: 99.9 Wh
- Full capacity: 99.9 Wh
- Voltage: 17.27V
- Cycles: 8
- Status: Not charging (100%)

---

## Cooling Devices (32 total)
These map to CPU/GPU cooling states via thermal framework.

---

## WMI Methods Available
Multiple Lenovo WMI GUIDs present:
- `887B54E2-DDDC-4B2C-8B88-68A26A8835D0-9` — GameZone
- `887B54E3-DDDC-4B2C-8B88-68A26A8835D0-4` — GameZone (alt)
- `DC2A8805-3A8C-41BA-A6F7-092E0089CD3B-7` — **Fan control (exposed as hwmon)**
- `8C5B9127-ECD4-4657-980F-851019F99CA5-8` — Capability data
- `362A3AFE-3D96-4665-8530-96DAD5BB300E-17` — Hotkey utilities
- And many more (platform profile, LEDs, etc.)
