# Quick Summary

## What works now (built-in kernel, no external programs):
- CPU temps (Tctl/Tccd1/Tccd2) — k10temp
- iGPU temp/voltage/power — amdgpu  
- dGPU temp/power/clocks — nvidia-smi only, no hwmon
- Both NVMe temps
- Both DDR5 RAM temps (SPD5118 via I2C)
- Ethernet + WiFi temps
- 3x Fan RPM + writable Fan Targets — lenovo_wmi_other
- Battery (V, capacity, conservation mode)
- Platform profiles (low-power/balanced/performance/max-power/custom) — lenovo_wmi_gamezone

## What's missing:
- **EC CPU temp** — different from k10temp, used by fan controller
- **EC GPU temp** — different from nvidia-smi/amdgpu
- **IC/board temp** — EC internal
- **VRM temps** — EC only
- **CPU power limits (PPT PL1/PL2)** — capdata01 reports no supported attrs for this model
- **Fan curve** (10-point) — EC registers F9F0-F9F9

## How to get them:
Only way is a **kernel module** — Linux WMI doesn't allow userspace method calls.

Option: **ChaoticSi1ence/legion-pro-7-16iax10h-linux** fork — already has `model_smcn` config for 83RU, exposes everything via hwmon. Conflicts with upstream `lenovo_wmi_*` on kernels 7.0+.

No pure-userspace / pure-sysfs path exists for EC temperatures on this hardware.
