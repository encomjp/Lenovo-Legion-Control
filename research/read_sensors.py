#!/usr/bin/env python3
"""Sensor reader prototype for Lenovo Legion Pro 7 16AFR10H (83RU).

Reads all available hardware sensors via sysfs/hwmon. Uses nvidia-smi for dGPU
(NVIDIA has no hwmon).

All paths discovered dynamically — no hardcoded indices.
"""

import os
import re
import glob
import subprocess
import json
from pathlib import Path
from collections import defaultdict

HWMON_BASE = "/sys/class/hwmon"


def _read(path: str) -> str | None:
    try:
        with open(path) as f:
            return f.read().strip()
    except (OSError, PermissionError):
        return None


def _read_int(path: str) -> int | None:
    try:
        val = _read(path)
        return int(val) if val is not None else None
    except ValueError:
        return None


def _discover_hwmon() -> dict[str, list[int]]:
    """Map hwmon name -> list of indices (multiple devices can share names).

    Uses instance tracking: name plus a counter for duplicates.
    Returns { "k10temp": [5], "nvme": [2, 3], ... }
    """
    mapping: dict[str, list[int]] = defaultdict(list)
    for d in sorted(Path(HWMON_BASE).iterdir()):
        name = _read(str(d / "name"))
        if name:
            idx = int(d.name.replace("hwmon", ""))
            mapping[name].append(idx)
    return dict(mapping)


def _get_hwmon_attrs(idx: int) -> dict[str, str]:
    """All readable sysfs attrs for a hwmon device (skip power/, uevent)."""
    base = Path(HWMON_BASE) / f"hwmon{idx}"
    attrs = {}
    for entry in sorted(base.iterdir()):
        if entry.is_dir() or entry.name == "uevent":
            continue
        val = _read(str(entry))
        if val is not None:
            attrs[entry.name] = val
    return attrs


# ─── Helpers ──────────────────────────────────────────────────────────────────

def _label_for(attrs: dict[str, str], base_key: str) -> str:
    """Find the label for a given sensor key.

    Tries {base_key}_label, then falls back to the key name itself.
    """
    label_key = f"{base_key}_label"
    if label_key in attrs:
        return attrs[label_key]
    return base_key


def _parse_temp(attrs: dict[str, str], base_key: str) -> dict:
    raw = int(attrs.get(base_key, 0))
    key_clean = base_key.replace("_input", "")
    label = _label_for(attrs, key_clean)
    critical = attrs.get(key_clean + "_crit")
    maximum = attrs.get(key_clean + "_max")
    return {
        "label": label,
        "value_C": raw / 1000,
        **({"max_C": int(maximum) / 1000} if maximum else {}),
        **({"crit_C": int(critical) / 1000} if critical else {}),
    }


# ─── Sensor Readers ───────────────────────────────────────────────────────────

def read_cpu(hm: dict[str, list[int]]) -> dict:
    """k10temp: Tctl, Tccd1, Tccd2."""
    idxs = hm.get("k10temp", [])
    if not idxs:
        return {"error": "k10temp not found"}
    attrs = _get_hwmon_attrs(idxs[0])
    return {
        "driver": "k10temp",
        "sensors": {
            _label_for(attrs, k.replace("_input", "")): int(attrs[k]) / 1000
            for k in attrs
            if k.startswith("temp") and k.endswith("_input")
            and not k.endswith("_alarm")
        }
    }


def read_igpu(hm: dict[str, list[int]]) -> dict:
    """amdgpu: edge temp, voltages, PPT, sclk."""
    idxs = hm.get("amdgpu", [])
    if not idxs:
        return {"error": "amdgpu not found"}
    attrs = _get_hwmon_attrs(idxs[0])
    result: dict = {"driver": "amdgpu", "sensors": {}}
    for key, val in attrs.items():
        if key == "name" or key.endswith("_label"):
            continue
        numeric = int(val)
        clean = key.replace("_input", "")
        label = _label_for(attrs, clean)
        if key.startswith("temp"):
            result["sensors"][label] = f"{numeric / 1000:.1f}°C"
        elif key.startswith("in"):
            result["sensors"][label] = f"{numeric / 1000:.3f}V"
        elif key.startswith("power"):
            result["sensors"][label] = f"{numeric / 1_000_000:.3f}W"
        elif key.startswith("freq"):
            result["sensors"][label] = f"{numeric / 1_000_000:.0f}MHz"
    return result


def read_dgpu() -> dict:
    """NVIDIA RTX 5080 via nvidia-smi subprocess."""
    NVIDIA_SMI = "/usr/bin/nvidia-smi"
    fields = "name,temperature.gpu,power.draw,clocks.gr,clocks.mem,power.limit"
    try:
        out = subprocess.check_output(
            [NVIDIA_SMI, "--query-gpu=" + fields, "--format=csv,noheader,nounits"],
            timeout=5, text=True, env={**os.environ, "PATH": os.environ.get("PATH", "")}
        ).strip()
        parts = [p.strip() for p in out.split(",")]
        return {
            "name": parts[0] if len(parts) > 0 else "?",
            "temperature_C": float(parts[1]) if len(parts) > 1 and parts[1] != "[N/A]" else None,
            "power_draw_W": float(parts[2]) if len(parts) > 2 and parts[2] != "[N/A]" else None,
            "clock_graphics_MHz": float(parts[3]) if len(parts) > 3 and parts[3] != "[N/A]" else None,
            "clock_memory_MHz": float(parts[4]) if len(parts) > 4 and parts[4] != "[N/A]" else None,
            "power_limit_W": parts[5] if len(parts) > 5 and parts[5] != "[N/A]" else None,
        }
    except Exception as e:
        return {"error": f"nvidia-smi failed: {e}"}


def read_nvme(hm: dict[str, list[int]]) -> list[dict]:
    """All NVMe drives."""
    results = []
    for idx in hm.get("nvme", []):
        attrs = _get_hwmon_attrs(idx)
        drive = {"hwmon_idx": idx}
        for key, val in attrs.items():
            if key == "name" or key.endswith("_label"):
                continue
            if key.startswith("temp") and key.endswith("_input"):
                label = _label_for(attrs, key.replace("_input", ""))
                clean = key.replace("_input", "")
                crit = attrs.get(f"{clean}_crit")
                maximum = attrs.get(f"{clean}_max")
                drive[label] = {
                    "value_C": int(val) / 1000,
                    **({"max_C": int(maximum) / 1000} if maximum else {}),
                    **({"crit_C": int(crit) / 1000} if crit else {}),
                }
        results.append(drive)
    return results


def read_ram(hm: dict[str, list[int]]) -> dict:
    """DDR5 SPD temps (spd5118 on I2C)."""
    result = {}
    for i, idx in enumerate(hm.get("spd5118", [])):
        attrs = _get_hwmon_attrs(idx)
        result[f"DIMM_{i}"] = {
            "temperature_C": int(attrs.get("temp1_input", 0)) / 1000,
            "max_C": int(attrs.get("temp1_max", 0)) / 1000,
            "crit_C": int(attrs.get("temp1_crit", 0)) / 1000,
        }
    return result


def read_fans(hm: dict[str, list[int]]) -> dict:
    """Fans via lenovo_wmi_other (WMI) + yogafan (legacy).

    Uses composite keys (source_fanN) to avoid collisions.
    """
    result = {}
    for driver in ["lenovo_wmi_other", "yogafan"]:
        for idx in hm.get(driver, []):
            attrs = _get_hwmon_attrs(idx)
            fan_nums = sorted({
                m.group(1) for key in attrs
                if (m := re.match(r"fan(\d+)_input", key))
            })
            for n in fan_nums:
                result[f"{driver}_fan{n}"] = {
                    "rpm": int(attrs.get(f"fan{n}_input", 0)),
                    "min_rpm": int(attrs.get(f"fan{n}_min", 0)),
                    "max_rpm": int(attrs.get(f"fan{n}_max", 0)),
                    "target_rpm": int(attrs.get(f"fan{n}_target", 0)),
                }
    return result


def read_nic(hm: dict[str, list[int]]) -> dict:
    """Ethernet (any r8169*) + WiFi (iwlwifi_1) temperatures."""
    result = {}
    for name, idxs in hm.items():
        if "r8169" in name:
            attrs = _get_hwmon_attrs(idxs[0])
            result["ethernet"] = {
                "name": name,
                "temperature_C": int(attrs.get("temp1_input", 0)) / 1000,
                "max_C": int(attrs.get("temp1_max", 0)) / 1000,
            }
        if "iwlwifi" in name:
            attrs = _get_hwmon_attrs(idxs[0])
            result["wifi"] = {
                "name": name,
                "temperature_C": int(attrs.get("temp1_input", 0)) / 1000,
            }
    return result


def read_battery() -> dict:
    """BAT0 via power_supply sysfs."""
    base = "/sys/class/power_supply/BAT0"
    return {
        "manufacturer": _read(f"{base}/manufacturer"),
        "model": _read(f"{base}/model_name"),
        "technology": _read(f"{base}/technology"),
        "status": _read(f"{base}/status"),
        "capacity_pct": _read_int(f"{base}/capacity"),
        "voltage_V": round(v / 1_000_000, 3) if (v := _read_int(f"{base}/voltage_now")) else None,
        "power_W": round(p / 1_000_000, 3) if (p := _read_int(f"{base}/power_now")) else None,
        "energy_now_Wh": round(e / 1_000_000, 1) if (e := _read_int(f"{base}/energy_now")) else None,
        "energy_full_Wh": round(f / 1_000_000, 1) if (f := _read_int(f"{base}/energy_full")) else None,
        "energy_design_Wh": round(d / 1_000_000, 1) if (d := _read_int(f"{base}/energy_full_design")) else None,
        "cycle_count": _read_int(f"{base}/cycle_count"),
        "charge_types": _read(f"{base}/charge_types"),
    }


def read_platform_profile() -> dict:
    return {
        "current": _read("/sys/firmware/acpi/platform_profile"),
        "choices": (_read("/sys/firmware/acpi/platform_profile_choices") or "").split(),
    }


def read_cooling_summary() -> dict:
    """Count cooling devices by type."""
    counts: dict[str, int] = defaultdict(int)
    total = 0
    for d in sorted(Path("/sys/class/thermal").glob("cooling_device*")):
        typ = _read(str(d / "type")) or "unknown"
        cur = _read_int(str(d / "cur_state"))
        mx = _read_int(str(d / "max_state"))
        counts[f"{typ}"] = counts.get(typ, 0) + 1
        total += 1
    counts["TOTAL"] = total
    return dict(counts)


def read_ec_hid() -> dict:
    """Find ITE EC HID devices (048d:c193 lighting, 048d:c197 EC).

    Parses vendor/product from modalias since there's no id/ subdir on hidraw.
    """
    devices = []
    for hidraw in sorted(Path("/dev").glob("hidraw*")):
        modalias = _read(f"/sys/class/hidraw/{hidraw.name}/device/modalias")
        if not modalias:
            continue
        m = re.search(r"v([0-9A-F]{8})p([0-9A-F]{8})", modalias, re.IGNORECASE)
        if not m:
            continue
        vendor_full, product_full = m.group(1).upper(), m.group(2).upper()
        vendor = vendor_full[-4:]  # last 4 hex digits
        product = product_full[-4:]
        if vendor != "048D":
            continue
        name_map = {
            "C193": "ITE Lenovo Lighting (keyboard white backlight)",
            "C197": "ITE Device 8258 (EC HID — keyboard RGB / fan control)",
        }
        devices.append({
            "hidraw": str(hidraw),
            "vendor": f"0x{vendor}",
            "product": f"0x{product}",
            "name": name_map.get(product, f"ITE {vendor}:{product}"),
            "modalias": modalias,
        })
    return {"devices": devices, "count": len(devices)}


def read_missing_info() -> dict:
    """Sensors only reachable via EC/WMI kernel driver."""
    return {
        "EC_CPU_temp": "WMI3 GetCPUTemp (method 18) — gamezone GUID 887B54E3-... needs kernel driver",
        "EC_GPU_temp": "WMI3 GetGPUTemp (method 19) — gamezone GUID 887B54E3-... needs kernel driver",
        "IC_board_temp": "EC memory read at 0xFE00D400 — needs legion-laptop or ec_sys module",
        "VRM_temps": "EC memory only — needs kernel module",
        "CPU_power_limits_PPT": "capdata01 PPT attrs — not supported on 83RU/SMCN upstream yet",
        "fan_curve_10point": "EC registers F9F0-F9F9 via WMI3 SET_TABLE — needs kernel module",
        "gpu_memory_junction": "Not exposed by NVIDIA driver at all",
    }


# ─── Main ─────────────────────────────────────────────────────────────────────

def main():
    hm = _discover_hwmon()
    print(f"=== HWMON Devices ({sum(len(v) for v in hm.values())} total) ===")
    for name, idxs in sorted(hm.items()):
        print(f"  {name}: hwmon{','.join(str(i) for i in idxs)}")
    print()

    all_sensors: dict = {}

    # CPU
    all_sensors["cpu"] = read_cpu(hm)
    print("=== CPU (k10temp) ===")
    for label, temp in all_sensors["cpu"].get("sensors", {}).items():
        print(f"  {label}: {temp:.1f}°C")

    # iGPU
    all_sensors["igpu"] = read_igpu(hm)
    print("\n=== iGPU (amdgpu) ===")
    for label, val in all_sensors["igpu"].get("sensors", {}).items():
        print(f"  {label}: {val}")

    # dGPU
    all_sensors["dgpu"] = read_dgpu()
    print("\n=== dGPU (nvidia-smi) ===")
    if "error" in all_sensors["dgpu"]:
        print(f"  ERROR: {all_sensors['dgpu']['error']}")
    else:
        for k, v in all_sensors["dgpu"].items():
            print(f"  {k}: {v}")

    # NVMe
    all_sensors["nvme"] = read_nvme(hm)
    print("\n=== NVMe ===")
    for drive in all_sensors["nvme"]:
        idx = drive.pop("hwmon_idx")
        for label, data in drive.items():
            if isinstance(data, dict):
                print(f"  hwmon{idx} {label}: {data['value_C']:.1f}°C")

    # RAM
    all_sensors["ram"] = read_ram(hm)
    print("\n=== RAM (SPD5118) ===")
    for label, data in all_sensors["ram"].items():
        print(f"  {label}: {data['temperature_C']:.1f}°C")

    # Fans
    all_sensors["fans"] = read_fans(hm)
    print("\n=== Fans ===")
    for label, data in sorted(all_sensors["fans"].items()):
        target = f" target={data['target_rpm']}" if data['target_rpm'] else " (auto)"
        print(f"  {label}: {data['rpm']} RPM"
              f" [min={data['min_rpm']} max={data['max_rpm']}]{target}")

    # NIC
    all_sensors["nic"] = read_nic(hm)
    print("\n=== Network Temps ===")
    for label, data in all_sensors["nic"].items():
        print(f"  {label}: {data['temperature_C']:.1f}°C ({data['name']})")

    # Battery
    all_sensors["battery"] = read_battery()
    print("\n=== Battery ===")
    for k, v in all_sensors["battery"].items():
        print(f"  {k}: {v}")

    # Platform profile
    all_sensors["profile"] = read_platform_profile()
    print("\n=== Platform Profile ===")
    print(f"  current: {all_sensors['profile']['current']}")
    print(f"  choices: {all_sensors['profile']['choices']}")

    # Cooling summary
    all_sensors["cooling"] = read_cooling_summary()
    print("\n=== Cooling Devices ===")
    for typ, count in sorted(all_sensors["cooling"].items()):
        print(f"  {typ}: {count}")

    # EC HID
    all_sensors["ec_hid"] = read_ec_hid()
    print("\n=== EC HID Devices ===")
    if not all_sensors["ec_hid"]["devices"]:
        print("  (none found — may need root)")
    for dev in all_sensors["ec_hid"]["devices"]:
        print(f"  {dev['hidraw']}: {dev['name']} ({dev['vendor']}:{dev['product']})")

    # Missing
    all_sensors["missing"] = read_missing_info()
    print("\n=== Missing (EC/WMI3 — need kernel module) ===")
    for k, v in all_sensors["missing"].items():
        print(f"  {k}: {v}")

    print()

    # Write JSON output for programmatic use
    json_path = Path(__file__).parent / "sensors_output.json"
    with open(json_path, "w") as f:
        json.dump(all_sensors, f, indent=2, default=str)
    print(f"Full JSON written to {json_path}")

    return all_sensors


if __name__ == "__main__":
    main()
