//! Hardware sensor reading via sysfs hwmon.
//!
//! Sources: k10temp (CPU), amdgpu (iGPU), legion_hwmon (EC CPU/GPU),
//! nvme (SSD), spd5118 (RAM), iwlwifi (WiFi), r8169 (Ethernet).

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SensorReadings {
    /// Main CPU temperature (k10temp Tctl).
    pub cpu_temp: f64,
    /// CPU die 1 temperature (k10temp Tccd1).
    pub cpu_temp_1: f64,
    /// CPU die 2 temperature (k10temp Tccd2).
    pub cpu_temp_2: f64,
    pub ec_cpu: f64,
    pub ec_gpu: f64,
    pub igpu_edge: f64,
    pub igpu_power: f64,
    pub dgpu_temp: f64,
    pub dgpu_power: f64,
    pub dgpu_clock: f64,
    pub ssd_composite: Vec<f64>,
    pub ram_temps: Vec<f64>,
    pub wifi_temp: f64,
    pub ethernet_temp: f64,
    pub fan1_rpm: u32,
    pub fan2_rpm: u32,
    pub fan4_rpm: u32,
    pub fan1_target: u32,
    pub fan2_target: u32,
    pub fan4_target: u32,
    pub profile: String,
    pub battery_pct: u32,
    pub battery_status: String,
    pub battery_voltage: f64,
    pub battery_cycles: u32,
    pub charge_type: String,
}

fn read_file(path: &Path) -> Option<String> {
    let res = fs::read_to_string(path).ok().map(|s| s.trim().to_string());
    if let Some(v) = &res {
        log::trace!("sensors::read_file: {} = {v:?}", path.display());
    } else {
        log::trace!("sensors::read_file: {} unreadable", path.display());
    }
    res
}

fn read_int(path: &Path) -> Option<i64> {
    let res = read_file(path).and_then(|s| s.parse().ok());
    if let Some(v) = &res {
        log::trace!("sensors::read_int: {} = {v}", path.display());
    } else {
        log::trace!("sensors::read_int: {} no integer value", path.display());
    }
    res
}

/// Discover hwmon devices by name, returning their sysfs paths.
pub fn find_hwmon(name: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let base = Path::new("/sys/class/hwmon");
    match fs::read_dir(base) {
        Ok(entries) => {
            for entry in entries.flatten() {
                let name_file = entry.path().join("name");
                if let Some(n) = read_file(&name_file) {
                    log::trace!(
                        "sensors::find_hwmon: dir {} name='{n}'",
                        entry.path().display()
                    );
                    if n == name {
                        log::debug!(
                            "sensors::find_hwmon: '{name}' matched at {}",
                            entry.path().display()
                        );
                        paths.push(entry.path());
                    }
                } else {
                    log::trace!(
                        "sensors::find_hwmon: dir {} has no readable name",
                        entry.path().display()
                    );
                }
            }
        }
        Err(e) => log::warn!("sensors::find_hwmon: {base:?} unreadable: {e}"),
    }
    log::debug!(
        "sensors::find_hwmon: '{name}' → {} match(es) {:?}",
        paths.len(),
        paths
    );
    paths
}

/// Get the first hwmon device matching a name.
pub fn hwmon_by_name(name: &str) -> Option<PathBuf> {
    find_hwmon(name).into_iter().next()
}

/// Read all sensors and return a snapshot.
pub fn read_all() -> SensorReadings {
    let mut s = SensorReadings::default();

    // ─── CPU (k10temp) ───
    if let Some(hw) = hwmon_by_name("k10temp") {
        for entry in fs::read_dir(&hw).into_iter().flatten().flatten() {
            let fname = entry.file_name().to_string_lossy().to_string();
            if fname.ends_with("_label") {
                if let Some(label) = read_file(&entry.path()) {
                    let input_path = hw.join(fname.replace("_label", "_input"));
                    if let Some(val) = read_int(&input_path) {
                        let temp = val as f64 / 1000.0;
                        match label.as_str() {
                            "Tctl" => {
                                log::debug!("sensors::read_all: k10temp Tctl → cpu_temp={temp}");
                                s.cpu_temp = temp;
                            }
                            "Tccd1" => {
                                log::debug!("sensors::read_all: k10temp Tccd1 → cpu_temp_1={temp}");
                                s.cpu_temp_1 = temp;
                            }
                            "Tccd2" => {
                                log::debug!("sensors::read_all: k10temp Tccd2 → cpu_temp_2={temp}");
                                s.cpu_temp_2 = temp;
                            }
                            other => {
                                log::trace!("sensors::read_all: k10temp label '{other}' not mapped")
                            }
                        }
                    }
                }
            }
        }
    }

    // ─── EC (legion_hwmon) ───
    if let Some(hw) = hwmon_by_name("legion_hwmon") {
        for entry in fs::read_dir(&hw).into_iter().flatten().flatten() {
            let fname = entry.file_name().to_string_lossy().to_string();
            if fname.ends_with("_label") {
                if let Some(label) = read_file(&entry.path()) {
                    let input_path = hw.join(fname.replace("_label", "_input"));
                    if let Some(val) = read_int(&input_path) {
                        let temp = val as f64 / 1000.0;
                        match label.as_str() {
                            "EC CPU" => {
                                log::debug!("sensors::read_all: EC CPU → ec_cpu={temp}");
                                s.ec_cpu = temp;
                            }
                            "EC GPU" => {
                                log::debug!("sensors::read_all: EC GPU → ec_gpu={temp}");
                                s.ec_gpu = temp;
                            }
                            other => log::trace!(
                                "sensors::read_all: legion_hwmon label '{other}' not mapped"
                            ),
                        }
                    }
                }
            }
        }
    }

    // ─── iGPU (amdgpu) ───
    if let Some(hw) = hwmon_by_name("amdgpu") {
        if let Some(val) = read_int(&hw.join("temp1_input")) {
            s.igpu_edge = val as f64 / 1000.0;
            log::debug!("sensors::read_all: amdgpu igpu_edge={:.1}°C", s.igpu_edge);
        } else {
            log::trace!("sensors::read_all: amdgpu temp1_input unavailable");
        }
        if let Some(val) = read_int(&hw.join("power1_input")) {
            s.igpu_power = val as f64 / 1_000_000.0;
            log::debug!("sensors::read_all: amdgpu igpu_power={:.2} W", s.igpu_power);
        } else {
            log::trace!("sensors::read_all: amdgpu power1_input unavailable");
        }
    } else {
        log::trace!("sensors::read_all: amdgpu hwmon not found");
    }

    // ─── dGPU (nvidia-smi) ───
    // Use -1.0 as "unavailable" so the UI does not show "0.0°C" which looks
    // like a frozen sensor.
    let t_temp = std::time::Instant::now();
    let temp = crate::dgpu::read_temp();
    log::debug!(
        "sensors::read_all: nvidia-smi temperature.gpu took {} ms → {:?}",
        t_temp.elapsed().as_millis(),
        temp
    );
    if temp.is_none() {
        log::warn!(
            "sensors::read_all: dgpu_temp sentinel -1.0 — no reading (spawn failed/timeout/exit)"
        );
    }
    s.dgpu_temp = temp.unwrap_or(-1.0);
    let t_power = std::time::Instant::now();
    let power = crate::dgpu::read_power();
    log::debug!(
        "sensors::read_all: nvidia-smi power.draw took {} ms → {:?}",
        t_power.elapsed().as_millis(),
        power
    );
    if power.is_none() {
        log::warn!("sensors::read_all: dgpu_power sentinel -1.0 — no reading");
    }
    s.dgpu_power = power.unwrap_or(-1.0);
    let t_clock = std::time::Instant::now();
    let clock = crate::dgpu::read_clock();
    log::debug!(
        "sensors::read_all: nvidia-smi clocks.gr took {} ms → {:?}",
        t_clock.elapsed().as_millis(),
        clock
    );
    if clock.is_none() {
        log::warn!("sensors::read_all: dgpu_clock sentinel -1.0 — no reading");
    }
    s.dgpu_clock = clock.unwrap_or(-1.0);

    // ─── NVMe SSDs (prefer Composite label; fall back to temp1) ───
    let mut nvme_drives = 0usize;
    for hw in find_hwmon("nvme") {
        nvme_drives += 1;
        let mut composite = None;
        let mut fallback = None;
        for entry in fs::read_dir(&hw).into_iter().flatten().flatten() {
            let fname = entry.file_name().to_string_lossy().to_string();
            if fname.ends_with("_label") {
                if let Some(label) = read_file(&entry.path()) {
                    let input_path = hw.join(fname.replace("_label", "_input"));
                    if let Some(val) = read_int(&input_path) {
                        let temp = val as f64 / 1000.0;
                        if label.eq_ignore_ascii_case("Composite") {
                            log::trace!(
                                "sensors::read_all: nvme {} Composite={temp}",
                                hw.display()
                            );
                            composite = Some(temp);
                        } else if fallback.is_none() && fname.starts_with("temp") {
                            log::trace!(
                                "sensors::read_all: nvme {} fallback {fname}={temp}",
                                hw.display()
                            );
                            fallback = Some(temp);
                        }
                    }
                }
            }
        }
        let source = if composite.is_some() {
            "Composite"
        } else {
            "fallback"
        };
        if let Some(t) = composite.or(fallback) {
            log::debug!(
                "sensors::read_all: nvme drive {} → {source} temp wins: {t}",
                hw.display()
            );
            s.ssd_composite.push(t);
        } else if let Some(val) = read_int(&hw.join("temp1_input")) {
            s.ssd_composite.push(val as f64 / 1000.0);
            log::debug!(
                "sensors::read_all: nvme drive {} → temp1_input wins: {:.1}",
                hw.display(),
                val as f64 / 1000.0
            );
        } else {
            log::warn!(
                "sensors::read_all: nvme drive {} yielded no usable temperature",
                hw.display()
            );
        }
    }
    log::debug!(
        "sensors::read_all: nvme drives scanned: {nvme_drives}, ssd_composite readings: {}",
        s.ssd_composite.len()
    );

    // ─── RAM (spd5118) ───
    let mut spd_found = 0usize;
    for hw in find_hwmon("spd5118") {
        spd_found += 1;
        if let Some(val) = read_int(&hw.join("temp1_input")) {
            s.ram_temps.push(val as f64 / 1000.0);
            log::trace!(
                "sensors::read_all: spd5118 {} temp {:.1}°C",
                hw.display(),
                val as f64 / 1000.0
            );
        }
    }
    log::debug!(
        "sensors::read_all: RAM SPD temps: {} module(s) found, {} reading(s)",
        spd_found,
        s.ram_temps.len()
    );
    if spd_found > s.ram_temps.len() {
        log::warn!(
            "sensors::read_all: {} SPD module(s) found but unreadable",
            spd_found - s.ram_temps.len()
        );
    }

    // ─── WiFi ───
    if let Some(hw) = hwmon_by_name("iwlwifi_1") {
        if let Some(val) = read_int(&hw.join("temp1_input")) {
            s.wifi_temp = val as f64 / 1000.0;
            log::debug!("sensors::read_all: wifi_temp={:.1}°C", s.wifi_temp);
        } else {
            log::trace!("sensors::read_all: iwlwifi_1 temp1_input unavailable");
        }
    } else {
        log::trace!("sensors::read_all: iwlwifi_1 hwmon not found");
    }

    // ─── Ethernet ───
    for hw in find_hwmon("r8169") {
        if let Some(val) = read_int(&hw.join("temp1_input")) {
            s.ethernet_temp = val as f64 / 1000.0;
            log::debug!(
                "sensors::read_all: ethernet_temp={:.1}°C (r8169)",
                s.ethernet_temp
            );
        }
    }
    // Also try r8169_0_700:00 variant
    for entry in fs::read_dir("/sys/class/hwmon")
        .into_iter()
        .flatten()
        .flatten()
    {
        if let Some(name) = read_file(&entry.path().join("name")) {
            if name.contains("r8169") {
                log::trace!(
                    "sensors::read_all: r8169 variant '{name}' at {}",
                    entry.path().display()
                );
                if let Some(val) = read_int(&entry.path().join("temp1_input")) {
                    s.ethernet_temp = val as f64 / 1000.0;
                    log::debug!(
                        "sensors::read_all: ethernet_temp={:.1}°C ({name})",
                        s.ethernet_temp
                    );
                }
            }
        }
    }

    // ─── Fans ───
    if let Some(hw) = hwmon_by_name("lenovo_wmi_other") {
        log::debug!(
            "sensors::read_all: fan hwmon lenovo_wmi_other at {}",
            hw.display()
        );
        for fan_num in 1..=4 {
            if let Some(val) = read_int(&hw.join(format!("fan{}_input", fan_num))) {
                log::trace!("sensors::read_all: fan{fan_num}_input → {val} rpm");
                match fan_num {
                    1 => s.fan1_rpm = val as u32,
                    2 => s.fan2_rpm = val as u32,
                    4 => s.fan4_rpm = val as u32,
                    _ => {}
                }
            }
            if let Some(val) = read_int(&hw.join(format!("fan{}_target", fan_num))) {
                log::trace!("sensors::read_all: fan{fan_num}_target → {val}");
                match fan_num {
                    1 => s.fan1_target = val as u32,
                    2 => s.fan2_target = val as u32,
                    4 => s.fan4_target = val as u32,
                    _ => {}
                }
            }
        }
    } else {
        log::warn!("sensors::read_all: lenovo_wmi_other fan hwmon not found — no fan RPM readings");
    }

    // ─── Platform profile ───
    s.profile = read_file(Path::new("/sys/firmware/acpi/platform_profile"))
        .unwrap_or_else(|| "unknown".to_string());
    if s.profile == "unknown" {
        log::warn!("sensors::read_all: platform_profile unavailable → 'unknown'");
    } else {
        log::debug!("sensors::read_all: platform_profile='{}'", s.profile);
    }

    // ─── Battery ───
    let bat = Path::new("/sys/class/power_supply/BAT0");
    if let Some(val) = read_int(&bat.join("capacity")) {
        s.battery_pct = val as u32;
        log::trace!("sensors::read_all: BAT0 capacity={}%", s.battery_pct);
    } else {
        log::trace!("sensors::read_all: BAT0 capacity unavailable");
    }
    s.battery_status = read_file(&bat.join("status")).unwrap_or_default();
    log::trace!("sensors::read_all: BAT0 status='{}'", s.battery_status);
    if let Some(val) = read_int(&bat.join("voltage_now")) {
        s.battery_voltage = val as f64 / 1_000_000.0;
        log::trace!("sensors::read_all: BAT0 voltage={:.3} V", s.battery_voltage);
    }
    if let Some(val) = read_int(&bat.join("cycle_count")) {
        s.battery_cycles = val as u32;
        log::trace!("sensors::read_all: BAT0 cycle_count={}", s.battery_cycles);
    }
    s.charge_type = read_file(&bat.join("charge_types")).unwrap_or_default();
    log::trace!("sensors::read_all: BAT0 charge_types='{}'", s.charge_type);

    s
}

/// Sample CPU package power via intel-rapl energy_uj (needs root / daemon).
pub fn sample_cpu_power_w() -> f64 {
    use std::sync::Mutex;
    use std::time::Instant;

    static PREV: Mutex<Option<(u64, Instant)>> = Mutex::new(None);
    const PATH: &str = "/sys/devices/virtual/powercap/intel-rapl/intel-rapl:0/energy_uj";

    use std::sync::atomic::{AtomicBool, Ordering};
    static DEGRADED_WARNED: AtomicBool = AtomicBool::new(false);
    let warn_once = |msg: String| {
        if !DEGRADED_WARNED.swap(true, Ordering::Relaxed) {
            log::warn!("{msg}");
        }
    };

    let Ok(raw) = fs::read_to_string(PATH) else {
        warn_once(format!(
            "sensors::sample_cpu_power_w: RAPL energy_uj unreadable at {PATH} — CPU power unavailable"
        ));
        return 0.0;
    };
    let Ok(energy) = raw.trim().parse::<u64>() else {
        warn_once(format!(
            "sensors::sample_cpu_power_w: RAPL energy_uj unparsable: {:?}",
            raw.trim()
        ));
        return 0.0;
    };
    log::trace!("sensors::sample_cpu_power_w: energy_uj={energy}");
    let now = Instant::now();
    let Ok(mut prev) = PREV.lock() else {
        warn_once("sensors::sample_cpu_power_w: RAPL state mutex poisoned".into());
        return 0.0;
    };
    let watts = if let Some((e0, t0)) = *prev {
        compute_cpu_power(e0, t0, energy, now)
    } else {
        0.0
    };
    *prev = Some((energy, now));
    log::debug!("sensors::sample_cpu_power_w: {watts:.3} W");
    watts
}

/// CPU busy percentage from `/proc/stat` (needs two samples; first call returns 0).
pub fn sample_cpu_usage_pct() -> f64 {
    use std::sync::Mutex;

    static PREV: Mutex<Option<(u64, u64)>> = Mutex::new(None);

    use std::sync::atomic::{AtomicBool, Ordering};
    static DEGRADED_WARNED: AtomicBool = AtomicBool::new(false);

    let Ok(raw) = fs::read_to_string("/proc/stat") else {
        if !DEGRADED_WARNED.swap(true, Ordering::Relaxed) {
            log::warn!(
                "sensors::sample_cpu_usage_pct: /proc/stat unreadable — CPU usage unavailable"
            );
        }
        return 0.0;
    };
    let line = raw.lines().next().unwrap_or("");
    log::trace!("sensors::sample_cpu_usage_pct: stat='{line}'");
    let mut parts = line.split_whitespace().skip(1);
    let mut vals = [0u64; 8];
    for v in vals.iter_mut() {
        *v = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    }
    // user nice system idle iowait irq softirq steal
    let idle = vals[3].saturating_add(vals[4]);
    let total: u64 = vals.iter().sum();

    let Ok(mut prev) = PREV.lock() else {
        if !DEGRADED_WARNED.swap(true, Ordering::Relaxed) {
            log::warn!("sensors::sample_cpu_usage_pct: state mutex poisoned");
        }
        return 0.0;
    };
    let pct = if let Some((p_idle, p_total)) = *prev {
        compute_cpu_usage(p_idle, p_total, idle, total)
    } else {
        0.0
    };
    *prev = Some((idle, total));
    let pct = pct.clamp(0.0, 100.0);
    log::debug!("sensors::sample_cpu_usage_pct: {pct:.1}%");
    pct
}

/// Discrete GPU utilization % via nvidia-smi (0 if unavailable).
pub fn sample_gpu_usage_pct() -> f64 {
    let t0 = std::time::Instant::now();
    let util = crate::dgpu::read_util();
    log::debug!(
        "sensors::sample_gpu_usage_pct: nvidia-smi utilization.gpu took {} ms → {:?}",
        t0.elapsed().as_millis(),
        util
    );
    if util.is_none() {
        log::warn!("sensors::sample_gpu_usage_pct: no dGPU utilization reading → 0.0");
    }
    util.unwrap_or(0.0).clamp(0.0, 100.0)
}

/// Pure helper: compute CPU usage from two snapshots. The single
/// implementation behind `sample_cpu_usage_pct`; exported so tests exercise
/// the same math the /proc path uses.
pub(crate) fn compute_cpu_usage(p_idle: u64, p_total: u64, c_idle: u64, c_total: u64) -> f64 {
    let d_total = c_total.saturating_sub(p_total);
    let d_idle = c_idle.saturating_sub(p_idle);
    if d_total == 0 {
        0.0
    } else {
        ((1.0 - d_idle as f64 / d_total as f64) * 100.0).clamp(0.0, 100.0)
    }
}

/// Pure helper: compute watts from two RAPL energy samples (microjoules +
/// instants). The single implementation behind `sample_cpu_power_w`.
pub(crate) fn compute_cpu_power(
    e0: u64,
    t0: std::time::Instant,
    e1: u64,
    t1: std::time::Instant,
) -> f64 {
    let dt = t1.duration_since(t0).as_secs_f64();
    if dt >= 0.2 && e1 >= e0 {
        (e1 - e0) as f64 / dt / 1_000_000.0
    } else {
        0.0
    }
}

/// Pure helper: pick best SSD temp from ranked sources (Composite > fallback > temp1).
#[allow(dead_code)]
pub(crate) fn select_ssd_temp(
    composite: Option<f64>,
    fallback: Option<f64>,
    temp1: Option<f64>,
) -> Option<f64> {
    composite.or(fallback).or(temp1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_cpu_usage_zero_and_clamp() {
        // No elapsed time → 0.
        assert_eq!(compute_cpu_usage(100, 1000, 100, 1000), 0.0);
        // All non-idle → 100.
        assert_eq!(compute_cpu_usage(0, 0, 0, 100), 100.0);
        // 80% idle → 20% busy (allow fp slop).
        let v = compute_cpu_usage(0, 0, 80, 100);
        assert!((v - 20.0).abs() < 1e-9, "v={v}");
    }

    #[test]
    fn compute_cpu_power_needs_time_and_monotonic() {
        let t0 = std::time::Instant::now();
        // Too short → 0 regardless of energy delta.
        let t1 = t0 + std::time::Duration::from_millis(100);
        assert_eq!(compute_cpu_power(0, t0, 1_000_000, t1), 0.0);
        // Enough time + monotonic → watts.
        let t2 = t0 + std::time::Duration::from_millis(500);
        let w = compute_cpu_power(0, t0, 500_000, t2);
        assert!(w > 0.9 && w < 1.1, "w={w}");
        // Energy went backwards (counter wrap) → 0.
        assert_eq!(compute_cpu_power(1_000_000, t0, 0, t2), 0.0);
    }

    #[test]
    fn select_ssd_temp_priority() {
        assert_eq!(
            select_ssd_temp(Some(40.0), Some(41.0), Some(42.0)),
            Some(40.0)
        );
        assert_eq!(select_ssd_temp(None, Some(41.0), Some(42.0)), Some(41.0));
        assert_eq!(select_ssd_temp(None, None, Some(42.0)), Some(42.0));
        assert_eq!(select_ssd_temp(None, None, None), None);
    }
}
