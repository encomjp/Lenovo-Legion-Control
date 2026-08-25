# Research Tooling & Reproduction

How the measurements in this directory were taken — and how to reproduce them.
The original Python probes (`ec_sensors.py`, `read_sensors.py`) were retired:
everything they did is captured below with plain shell commands, and their
logic lives on in production code (`lenovo-legion-tool/src/sensors.rs`).

## 1. EC register dump (replaces `ec_sensors.py`)

Target: Lenovo Legion Pro 7 16AFR10H (83RU), ITE IT5508 EC, BIOS SMCN20WW+.

**Method A — kernel-managed (preferred):**

```bash
sudo modprobe ec_sys write_support=0
sudo dd if=/sys/kernel/debug/ec/ec0/io bs=256 count=1 | xxd
```

**Method B — raw I/O ports (bypasses kernel EC driver; use sparingly):**
Ports `0x66` = command, `0x62` = data; read command is `0x80 <addr>`.
Only useful when `ec_sys` cannot be loaded.

**Discovered register map:** see [EC-REGISTER-MAP.md](EC-REGISTER-MAP.md)
(highlights: `0xB0` CPU temp ≈ k10temp Tctl · `0xB4` dGPU temp ≈ nvidia-smi ·
`0xB1` chipset · `0x0F` ambient).

Captured evidence: `ec-mode-dumps/`, `ec_sensors_output.json`.

## 2. Full sensor sweep (replaces `read_sensors.py`)

Walk `/sys/class/hwmon/*`: read each `name`, then every `tempN_input` +
`tempN_label`; dGPU via `nvidia-smi --query-gpu=...` (no hwmon on NVIDIA).

This logic is now **production code**: `lenovo-legion-tool/src/sensors.rs`
(dynamic hwmon discovery, k10temp Tctl/Tccd mapping, NVMe Composite preference,
nvidia-smi fallback). Re-probe manually with:

```bash
grep -H . /sys/class/hwmon/*/name
grep -H . /sys/class/hwmon/*/temp*_label 2>/dev/null
grep -H . /sys/class/hwmon/*/temp*_input 2>/dev/null | sort
nvidia-smi --query-gpu=temperature.gpu,power.draw,clocks.gr --format=csv,noheader,nounits
```

Captured evidence: `sensors_output.json`, `sensors-raw.txt`,
`sensors-full.md`, `hwmon-names.txt`, `nvidia-sensors.txt`.

## 3. Artifact index

| File | Contents |
|------|----------|
| `EC-RESEARCH.md` / `EC-REGISTER-MAP.md` | EC protocol + per-register findings |
| `SPECTRUM-ZONES.md` | RGB zone/effect notes |
| `sensor-research-findings.md` | sensor-stack conclusions |
| `BATTERY-LIMITER-FINDINGS.md` | charge-limiter root cause + decision log |
| `ec-mode-dumps/` | per-power-mode 256 B EC RAM dumps |
| `*.json` / `*.txt` / `sensors-full.md` | raw measurement snapshots |
| `ui-shots/` | UX screenshots |
