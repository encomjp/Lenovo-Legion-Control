#!/usr/bin/env python3
"""Hacky EC sensor reader for Lenovo Legion Pro 7 16AFR10H (83RU).

Three methods to read EC registers, in order of preference:
  1. ec_sys  — /sys/kernel/debug/ec/ec0/io (kernel-managed, safe)
  2. /dev/port — raw I/O ports 0x62/0x66 (bypasses kernel, use with care)
  3. fallback  — error if neither works

Discovered EC register map (SMCN20WW / ITE IT5508):
  EC[0xB0] — CPU temperature (°C, confirmed vs k10temp Tctl)
  EC[0xB4] — dGPU temperature (°C, confirmed vs nvidia-smi)
  EC[0x0F] — ~47°C (board/ambient)
  EC[0xB1] — ~42°C (chipset/IC)
  EC[0xB3] — ~45°C (unknown, possibly VRM)
  EC[0x03] — ~21°C (PCH/cold area)
  EC[0x04] — ~29°C (RAM area)

For ec_sys: sudo modprobe ec_sys write_support=0
For /dev/port: just run with sudo.
"""

import os
import struct
import time
import subprocess
import json
from pathlib import Path

HWMON_BASE = "/sys/class/hwmon"


def _read(path: str) -> str | None:
    try:
        with open(path) as f:
            return f.read().strip()
    except (OSError, PermissionError):
        return None


def _discover_hwmon() -> dict[str, list[int]]:
    mapping: dict[str, list[int]] = {}
    for d in sorted(Path(HWMON_BASE).iterdir()):
        name = _read(str(d / "name"))
        if name:
            idx = int(d.name.replace("hwmon", ""))
            mapping.setdefault(name, []).append(idx)
    return mapping


class EcReader:
    """Reads EC registers. Tries ec_sys first, falls back to /dev/port."""

    EC_SYS_PATH = "/sys/kernel/debug/ec/ec0/io"
    EC_CMD_PORT = 0x66
    EC_DATA_PORT = 0x62
    EC_READ_CMD = 0x80

    def __init__(self):
        self._method = None
        self._fd = None

        # Try ec_sys first (kernel-managed, safer)
        if os.path.exists(self.EC_SYS_PATH):
            self._fd = os.open(self.EC_SYS_PATH, os.O_RDONLY)
            self._method = "ec_sys"
            return

        # Fall back to /dev/port (raw I/O)
        try:
            self._fd = os.open("/dev/port", os.O_RDWR)
            self._method = "dev_port"
        except PermissionError:
            raise PermissionError(
                "Need root. Also try: sudo modprobe ec_sys write_support=0"
            )

    def read(self, port: int) -> int:
        if self._method == "ec_sys":
            os.lseek(self._fd, port & 0xFF, os.SEEK_SET)
            return struct.unpack("B", os.read(self._fd, 1))[0]
        return self._read_devport(port)

    def _read_devport(self, port: int) -> int:
        """Read single byte via classic ACPI EC protocol (ports 0x62/0x66)."""
        for _ in range(10000):
            os.lseek(self._fd, self.EC_CMD_PORT, os.SEEK_SET)
            if not (struct.unpack("B", os.read(self._fd, 1))[0] & 0x02):
                break
        os.lseek(self._fd, self.EC_CMD_PORT, os.SEEK_SET)
        os.write(self._fd, struct.pack("B", self.EC_READ_CMD))
        for _ in range(10000):
            os.lseek(self._fd, self.EC_CMD_PORT, os.SEEK_SET)
            if not (struct.unpack("B", os.read(self._fd, 1))[0] & 0x02):
                break
        os.lseek(self._fd, self.EC_DATA_PORT, os.SEEK_SET)
        os.write(self._fd, struct.pack("B", port & 0xFF))
        for _ in range(10000):
            os.lseek(self._fd, self.EC_CMD_PORT, os.SEEK_SET)
            if struct.unpack("B", os.read(self._fd, 1))[0] & 0x01:
                break
        os.lseek(self._fd, self.EC_DATA_PORT, os.SEEK_SET)
        return struct.unpack("B", os.read(self._fd, 1))[0]

    @property
    def method(self) -> str:
        return self._method

    def close(self):
        if self._fd is not None:
            os.close(self._fd)
            self._fd = None


# ─── Sensor Readers ───────────────────────────────────────────────────────────

def read_sysfs_temps(hm: dict) -> dict:
    """Temperatures from existing hwmon (no /dev/port needed)."""
    result = {}

    # CPU (k10temp)
    for idx in hm.get("k10temp", []):
        base = Path(HWMON_BASE) / f"hwmon{idx}"
        for f in sorted(base.iterdir()):
            if f.name.endswith("_label"):
                lbl = _read(str(f))
                inp = str(f).replace("_label", "_input")
                val = _read(inp)
                if val and lbl:
                    result[f"cpu_{lbl}"] = int(val) / 1000

    # iGPU (amdgpu)
    for idx in hm.get("amdgpu", []):
        base = Path(HWMON_BASE) / f"hwmon{idx}"
        labels = {}
        for f in base.iterdir():
            if f.name.endswith("_label"):
                labels[f.name.replace("_label", "")] = _read(str(f))
        for f in base.iterdir():
            if f.name == "name" or f.name.endswith("_label"):
                continue
            val = _read(str(f))
            if val is None or val == "":
                continue
            key = f.name.replace("_input", "")
            lbl = labels.get(key, key)
            try:
                n = int(val)
            except ValueError:
                continue
            if f.name.startswith("temp"):
                result[f"igpu_{lbl}"] = n / 1000
            elif f.name.startswith("power"):
                result[f"igpu_{lbl}_W"] = n / 1_000_000

    # NVMe
    for idx in hm.get("nvme", []):
        base = Path(HWMON_BASE) / f"hwmon{idx}"
        for f in sorted(base.iterdir()):
            if f.name.startswith("temp") and f.name.endswith("_input") and not f.name.endswith("_alarm"):
                lbl_f = str(base / f.name.replace("_input", "_label"))
                lbl = _read(lbl_f) or f.name.replace("_input", "")
                result[f"nvme_hwmon{idx}_{lbl}"] = int(_read(str(f))) / 1000

    # RAM (spd5118)
    for i, idx in enumerate(hm.get("spd5118", [])):
        base = Path(HWMON_BASE) / f"hwmon{idx}"
        inp = _read(str(base / "temp1_input"))
        if inp:
            result[f"ram_dim_{i}"] = int(inp) / 1000

    # NIC
    for name, idxs in hm.items():
        if "r8169" in name:
            base = Path(HWMON_BASE) / f"hwmon{idxs[0]}"
            v = _read(str(base / "temp1_input"))
            if v:
                result["ethernet"] = int(v) / 1000
        if "iwlwifi" in name:
            base = Path(HWMON_BASE) / f"hwmon{idxs[0]}"
            v = _read(str(base / "temp1_input"))
            if v:
                result["wifi"] = int(v) / 1000

    return result


def read_dgpu() -> dict:
    """NVIDIA dGPU via nvidia-smi."""
    try:
        out = subprocess.check_output(
            ["/usr/bin/nvidia-smi",
             "--query-gpu=temperature.gpu,power.draw,clocks.gr,clocks.mem",
             "--format=csv,noheader,nounits"],
            timeout=5, text=True
        ).strip()
        parts = [p.strip() for p in out.split(",")]
        return {
            "dgpu_temp_C": float(parts[0]) if len(parts) > 0 and parts[0] != "[N/A]" else None,
            "dgpu_power_W": float(parts[1]) if len(parts) > 1 and parts[1] != "[N/A]" else None,
            "dgpu_clock_MHz": float(parts[2]) if len(parts) > 2 and parts[2] != "[N/A]" else None,
        }
    except Exception:
        return {"dgpu_error": "nvidia-smi unavailable"}


def read_ec_temps(ec: EcReader) -> dict:
    """Temperature sensors discovered via EC register analysis.

    These are the temperatures the EC uses for fan control.
    Different from the PCI/on-die sensors exposed by k10temp/amdgpu/nvidia.
    """
    return {
        "ec_cpu_temp_C": ec.read(0xB0),        # Confirmed = k10temp Tctl (±1°C)
        "ec_dgpu_temp_C": ec.read(0xB4),        # Confirmed = nvidia-smi GPU temp
        "ec_board_temp_C": ec.read(0x0F),       # Stable ~47°C, likely board
        "ec_chipset_temp_C": ec.read(0xB1),     # Stable ~42°C, likely IC/package
        "ec_unk_temp_C": ec.read(0xB3),         # Stable ~45°C, unknown
        "ec_pch_temp_C": ec.read(0x03),         # ~24°C, changes slightly under load
        "ec_ram_area_temp_C": ec.read(0x04),    # ~32°C, changes under load
    }


def read_fans(hm: dict) -> dict:
    """Fans from lenovo_wmi_other hwmon."""
    result = {}
    for idx in hm.get("lenovo_wmi_other", []):
        base = Path(HWMON_BASE) / f"hwmon{idx}"
        for f in sorted(base.iterdir()):
            if f.name.startswith("fan") and f.name.endswith("_input"):
                n = f.name.replace("_input", "")
                rpm = _read(str(f))
                target = _read(str(base / f"{n}_target"))
                result[n] = {
                    "rpm": int(rpm) if rpm else 0,
                    "target": int(target) if target else 0,
                }
    return result


def read_battery() -> dict:
    base = "/sys/class/power_supply/BAT0"
    v_raw = _read(f"{base}/voltage_now")
    voltage = round(int(v_raw) / 1_000_000, 3) if v_raw else None
    return {
        "battery_pct": int(_read(f"{base}/capacity") or 0),
        "battery_status": _read(f"{base}/status"),
        "battery_charge_type": _read(f"{base}/charge_types"),
        "battery_voltage_V": voltage,
        "battery_cycles": int(_read(f"{base}/cycle_count") or 0),
    }


def read_platform_profile() -> str:
    return _read("/sys/firmware/acpi/platform_profile") or "unknown"


# ─── Main ─────────────────────────────────────────────────────────────────────

def main():
    hm = _discover_hwmon()

    print("=" * 60)
    print(" Lenovo Legion Pro 7 16AFR10H (83RU) — Sensor Snapshot")
    print("=" * 60)

    # Sysfs temps
    sysfs = read_sysfs_temps(hm)
    print("\n─── Standard HWMON Sensors ───")
    for k, v in sorted(sysfs.items()):
        val = f"{v:.1f}°C" if "_W" not in k else f"{v:.3f}W"
        print(f"  {k:30s}: {val}")

    # dGPU
    dgpu = read_dgpu()
    print("\n─── dGPU (nvidia-smi) ───")
    for k, v in dgpu.items():
        print(f"  {k:30s}: {v}")

    # Hacky EC temps
    ec_method = "none"
    ec_temps = {}
    try:
        ec = EcReader()
        ec_method = ec.method
        ec_temps = read_ec_temps(ec)
        print(f"\n─── EC Temps ({ec_method}) ───")
        for k, v in sorted(ec_temps.items()):
            print(f"  {k:30s}: {v}°C")
        ec.close()
    except PermissionError:
        print("\n─── EC Temps ───")
        print("  ERROR: Need root. Run with sudo, or:")
        print("    sudo modprobe ec_sys write_support=0")
    except Exception as e:
        print(f"\n─── EC Temps ───")
        print(f"  ERROR: {e}")

    # Fans
    fans = read_fans(hm)
    print("\n─── Fans ───")
    for name, data in sorted(fans.items()):
        tgt = f" (target={data['target']})" if data["target"] else ""
        print(f"  {name:30s}: {data['rpm']} RPM{tgt}")

    # Battery
    bat = read_battery()
    print("\n─── Battery ───")
    for k, v in bat.items():
        print(f"  {k:30s}: {v}")

    # Profile
    print(f"\n─── Platform Profile: {read_platform_profile()}")

    # Summary
    print("\n" + "=" * 60)
    print(" Legend: EC temps are read directly from the Embedded Controller")
    print(" via ACPI I/O ports (0x62/0x66). These are the temperatures the")
    print(" EC uses for fan control. They differ from on-die (PCI) sensors.")
    print(" All EC reads are read-only — nothing is written to the EC.")
    print("=" * 60)

    # Export JSON
    output = {
        "sysfs_hwmon": sysfs,
        "dgpu_nvidia_smi": dgpu,
        "ec_temps_devport": ec_temps,
        "fans": fans,
        "battery": bat,
        "platform_profile": read_platform_profile(),
    }
    json_path = Path(__file__).parent / "ec_sensors_output.json"
    with open(json_path, "w") as f:
        json.dump(output, f, indent=2, default=str)
    print(f"\nJSON written to {json_path}")

    return output


if __name__ == "__main__":
    main()
