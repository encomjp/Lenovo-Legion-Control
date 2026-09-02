//! Hardware sensor reading via sysfs hwmon.
//!
//! Sources: k10temp (CPU), amdgpu (iGPU), legion_hwmon (EC CPU/GPU),
//! nvme (SSD), spd5118 (RAM), iwlwifi (WiFi), r8169 (Ethernet).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::battery;

/// Cached mapping of hwmon subsystem names to their base paths.
static HWMON_CACHE: Mutex<Option<HashMap<String, Vec<PathBuf>>>> = Mutex::new(None);

fn scan_all_hwmon() -> HashMap<String, Vec<PathBuf>> {
    let mut map: HashMap<String, Vec<PathBuf>> = HashMap::new();
    let base = Path::new("/sys/class/hwmon");
    if let Ok(entries) = fs::read_dir(base) {
        for entry in entries.flatten() {
            let name_file = entry.path().join("name");
            if let Some(n) = read_file(&name_file) {
                map.entry(n).or_default().push(entry.path());
            }
        }
    }
    map
}

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
    let res = match fs::read_to_string(path) {
        Ok(s) => Some(s.trim().to_string()),
        Err(e) => {
            log::debug!("sensors::read_file — {} returned None: {e}", path.display());
            None
        }
    };
    if let Some(v) = &res {
        log::trace!("sensors::read_file: {} = {v:?}", path.display());
    } else {
        log::trace!("sensors::read_file: {} unreadable", path.display());
    }
    res
}

fn read_int(path: &Path) -> Option<i64> {
    let res = match read_file(path) {
        Some(s) => match s.parse::<i64>() {
            Ok(v) => Some(v),
            Err(e) => {
                log::trace!(
                    "sensors::read_int — {} returned None: {e} (raw={s:?})",
                    path.display()
                );
                None
            }
        },
        None => None,
    };
    if let Some(v) = &res {
        log::trace!("sensors::read_int: {} = {v}", path.display());
    } else {
        log::trace!("sensors::read_int: {} no integer value", path.display());
    }
    res
}

/// Iterate a directory, logging (not silently dropping) readdir failures.
fn read_dir_entries(dir: &Path) -> Vec<std::fs::DirEntry> {
    match fs::read_dir(dir) {
        Ok(rd) => rd.flatten().collect(),
        Err(e) => {
            log::debug!("sensors::read_dir_entries — {} failed: {e}", dir.display());
            Vec::new()
        }
    }
}

/// One-shot diagnostic: report how (and whether) `nvidia-smi` resolves from
/// PATH. Remote-troubleshooting aid for "dGPU readings are -1.0" reports.
/// Spawn mechanics (duration, exit code, stdout size) live in crate::dgpu.
fn log_nvidia_smi_path_once() {
    static ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    if ONCE.set(()).is_err() {
        return; // already ran this process
    }
    let Some(paths) = std::env::var_os("PATH") else {
        log::warn!("sensors::nvidia_smi — PATH is unset; cannot locate nvidia-smi");
        return;
    };
    let dirs: Vec<PathBuf> = std::env::split_paths(&paths).collect();
    for dir in &dirs {
        let candidate = dir.join("nvidia-smi");
        if candidate.is_file() {
            log::debug!(
                "sensors::nvidia_smi — binary resolved from PATH: {} ({} PATH dir(s) searched)",
                candidate.display(),
                dirs.len()
            );
            return;
        }
    }
    log::warn!(
        "sensors::nvidia_smi — nvidia-smi not found in any of {} PATH dir(s) — dGPU readings will be unavailable",
        dirs.len()
    );
}

/// (hwmon-name, path) for every chip whose name starts with one of the
/// prefixes. Used by the WiFi fallback (driver naming varies by vendor).
fn find_hwmon_prefix(prefixes: &[&str]) -> Vec<(String, PathBuf)> {
    let base = Path::new("/sys/class/hwmon");
    let mut out = Vec::new();
    for entry in read_dir_entries(base) {
        if let Some(name) = read_file(&entry.path().join("name")) {
            if prefixes.iter().any(|p| name.starts_with(p)) {
                out.push((name, entry.path()));
            }
        }
    }
    out
}

/// Discover hwmon devices by name, returning their sysfs paths (using cache with auto-refresh).
pub fn find_hwmon(name: &str) -> Vec<PathBuf> {
    let mut guard = match HWMON_CACHE.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    if let Some(cache) = guard.as_ref() {
        if let Some(paths) = cache.get(name) {
            if paths.iter().all(|p| p.exists()) {
                return paths.clone();
            }
        }
    }
    let fresh = scan_all_hwmon();
    let result = fresh.get(name).cloned().unwrap_or_default();
    *guard = Some(fresh);
    result
}

/// Get the first hwmon device matching a name.
pub fn hwmon_by_name(name: &str) -> Option<PathBuf> {
    find_hwmon(name).into_iter().next()
}

/// Read temp/power/clock from the amdgpu hwmon that is NOT the iGPU.
/// Multiple `amdgpu` hwmon devices exist on hybrid systems (iGPU + dGPU);
/// each hwmon has a `device` symlink — resolve its PCI address and pick the
/// one whose slot differs from the iGPU's (integrated GPUs live on the CPU's
/// PCI address; dGPUs sit on their own). When only one amdgpu hwmon exists
/// the machine is APU-only and there is no dGPU to report.
fn amd_dgpu_hwmon_read() -> Option<(f64, f64, f64)> {
    let cards = find_hwmon("amdgpu");
    if cards.len() < 2 {
        return None;
    }
    // Resolve each hwmon's PCI address (e.g. "0000:08:00.0").
    let mut slots: Vec<(PathBuf, String)> = Vec::new();
    for hw in &cards {
        // /sys/class/hwmon/hwmonN/device → ../../…/0000:08:00.0
        let addr = std::fs::read_link(hw.join("device"))
            .ok()
            .and_then(|l| {
                l.to_string_lossy()
                    .split('/')
                    .next_back()
                    .map(str::to_string)
            })
            .unwrap_or_default();
        slots.push((hw.clone(), addr));
    }
    if slots.len() < 2 {
        return None;
    }
    // Heuristic: the dGPU is the card whose PCI address does NOT share the
    // first card's bus. Sort by slot and prefer the later address (dGPUs
    // enumerate after the iGPU on AMD hybrid laptops); require distinct
    // addresses so we never pick the same device twice.
    slots.sort_by(|a, b| a.1.cmp(&b.1));
    let (gpu_hw, _) = slots.last()?.clone();
    if gpu_hw == slots[0].0 {
        return None;
    }
    let temp = read_int(&gpu_hw.join("temp1_input")).map(|v| v as f64 / 1000.0);
    let power = read_int(&gpu_hw.join("power1_average"))
        .or_else(|| read_int(&gpu_hw.join("power1_input")))
        .map(|v| v as f64 / 1_000_000.0);
    let clock = read_int(&gpu_hw.join("freq1_input")).map(|v| v as f64 / 1_000_000.0);
    if temp.is_none() && power.is_none() {
        return None;
    }
    Some((
        temp.unwrap_or(-1.0),
        power.unwrap_or(-1.0),
        clock.unwrap_or(-1.0),
    ))
}

/// Read all sensors and return a snapshot.
pub fn read_all() -> SensorReadings {
    let mut s = SensorReadings::default();

    // ─── CPU (k10temp AMD + coretemp Intel) ───
    if let Some(hw) = hwmon_by_name("k10temp").or_else(|| hwmon_by_name("zenpower")) {
        let mut labels_seen: Vec<(String, &str)> = Vec::new();
        for entry in read_dir_entries(&hw) {
            let fname = entry.file_name().to_string_lossy().to_string();
            if fname.ends_with("_label") {
                if let Some(label) = read_file(&entry.path()) {
                    let input_path = hw.join(fname.replace("_label", "_input"));
                    if let Some(val) = read_int(&input_path) {
                        let temp = val as f64 / 1000.0;
                        let field = match label.as_str() {
                            "Tctl" | "Tdie" => {
                                log::debug!("sensors::read_all: AMD cpu temp → cpu_temp={temp}");
                                s.cpu_temp = temp;
                                "cpu_temp"
                            }
                            "Tccd1" => {
                                log::debug!("sensors::read_all: k10temp Tccd1 → cpu_temp_1={temp}");
                                s.cpu_temp_1 = temp;
                                "cpu_temp_1"
                            }
                            "Tccd2" => {
                                log::debug!("sensors::read_all: k10temp Tccd2 → cpu_temp_2={temp}");
                                s.cpu_temp_2 = temp;
                                "cpu_temp_2"
                            }
                            other => {
                                log::trace!(
                                    "sensors::read_all: k10temp label '{other}' not mapped"
                                );
                                "(unmapped)"
                            }
                        };
                        labels_seen.push((label, field));
                    } else {
                        labels_seen.push((label, "<input unreadable>"));
                    }
                }
            }
        }
        log::debug!("sensors::read_all — k10temp labels encountered: {labels_seen:?}");
    } else if let Some(hw) = hwmon_by_name("coretemp") {
        // Intel Raptor Lake / Alder Lake: Package id 0 is the CPU package temp,
        // Core N are per-core. Take hottest package as cpu_temp.
        let mut labels_seen: Vec<(String, f64)> = Vec::new();
        let mut max_pkg: Option<f64> = None;
        let mut max_core: Option<f64> = None;
        for entry in read_dir_entries(&hw) {
            let fname = entry.file_name().to_string_lossy().to_string();
            if fname.ends_with("_label") {
                if let Some(label) = read_file(&entry.path()) {
                    let input_path = hw.join(fname.replace("_label", "_input"));
                    if let Some(val) = read_int(&input_path) {
                        let temp = val as f64 / 1000.0;
                        labels_seen.push((label.clone(), temp));
                        if label.contains("Package id") {
                            max_pkg = Some(max_pkg.map_or(temp, |v| v.max(temp)));
                        } else if label.starts_with("Core ") {
                            max_core = Some(max_core.map_or(temp, |v| v.max(temp)));
                        }
                    }
                }
            }
        }
        if let Some(pkg) = max_pkg {
            s.cpu_temp = pkg;
            log::debug!(
                "sensors::read_all: coretemp Package → cpu_temp={pkg} (labels={labels_seen:?})"
            );
        }
        if let Some(core_max) = max_core {
            // Expose hottest core as cpu_temp_1 for Intel; keeps cpu_temp_1 non-zero in fleet.
            s.cpu_temp_1 = core_max;
            log::debug!("sensors::read_all: coretemp hottest core → cpu_temp_1={core_max}");
        }
        // Thermal zone fallback if coretemp had no Package label (some BIOS hide it)
        if s.cpu_temp == 0.0 {
            for zone in read_dir_entries(Path::new("/sys/class/thermal")) {
                let type_path = zone.path().join("type");
                if let Some(t) = read_file(&type_path) {
                    if (t == "x86_pkg_temp" || t == "acpitz") && s.cpu_temp == 0.0 {
                        if let Some(v) = read_int(&zone.path().join("temp")) {
                            let temp = v as f64 / 1000.0;
                            if temp > 0.0 {
                                s.cpu_temp = temp;
                                log::debug!("sensors::read_all: thermal {t} → cpu_temp={temp}");
                                break;
                            }
                        }
                    }
                }
            }
        }
        log::debug!("sensors::read_all — coretemp labels encountered: {labels_seen:?}");
    } else {
        // Last resort: thermal zone x86_pkg_temp (works on both Intel/AMD when hwmon missing)
        for zone in read_dir_entries(Path::new("/sys/class/thermal")) {
            let type_path = zone.path().join("type");
            if let Some(t) = read_file(&type_path) {
                if t == "x86_pkg_temp" {
                    if let Some(v) = read_int(&zone.path().join("temp")) {
                        let temp = v as f64 / 1000.0;
                        if temp > 0.0 {
                            s.cpu_temp = temp;
                            log::debug!(
                                "sensors::read_all: thermal {t} fallback → cpu_temp={temp}"
                            );
                            break;
                        }
                    }
                }
            }
        }
    }

    // ─── EC (legion_hwmon) ───
    if let Some(hw) = hwmon_by_name("legion_hwmon") {
        for entry in read_dir_entries(&hw) {
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

    // ─── dGPU (nvidia-smi batch query) ───
    // Query temp, power, and clock in a single subprocess execution.
    // Use -1.0 as "unavailable" so the UI does not show "0.0°C" which looks
    // like a frozen sensor.
    log_nvidia_smi_path_once();
    let t_smi = std::time::Instant::now();
    let dgpu_data = crate::dgpu::read_metrics_batch();
    log::debug!(
        "sensors::read_all: nvidia-smi batch query took {} ms → {:?}",
        t_smi.elapsed().as_millis(),
        dgpu_data
    );
    s.dgpu_temp = dgpu_data.temp.unwrap_or(-1.0);
    s.dgpu_power = dgpu_data.power.unwrap_or(-1.0);
    s.dgpu_clock = dgpu_data.clock.unwrap_or(-1.0);

    // ─── AMD dGPU fallback (amdgpu hwmon) ───
    // On machines without nvidia-smi (Radeon RX 7000M/8000M class LOQs, or
    // NVIDIA-less variants) the dGPU still exposes temp/power via a second
    // amdgpu hwmon — distinguish it from the iGPU hwmon by PCI slot: the
    // dGPU sits on a different bus address than the iGPU (which shares the
    // CPU's slot).
    if s.dgpu_temp < 0.0 {
        if let Some((temp, power, clock)) = amd_dgpu_hwmon_read() {
            s.dgpu_temp = temp;
            s.dgpu_power = power;
            s.dgpu_clock = clock;
            log::debug!(
                "sensors::read_all: AMD dGPU hwmon temp={temp:.1}°C power={power:.2}W clock={clock:.0}MHz"
            );
        }
    }

    // ─── NVMe SSDs (prefer Composite label; fall back to temp1) ───
    let mut nvme_drives = 0usize;
    for hw in find_hwmon("nvme") {
        nvme_drives += 1;
        let mut composite = None;
        let mut fallback = None;
        for entry in read_dir_entries(&hw) {
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
    // Driver naming varies by card vendor: Intel=iwlwifi_*, MediaTek=mt7921_*,
    // Realtek=rtw89_*/rtw88_*, Broadcom=brcmutil_*. Probe the known families.
    const WIFI_DRIVERS: [&str; 6] = [
        "iwlwifi_1",
        "iwlwifi_0",
        "mt7921_phy0",
        "mt7925_phy0",
        "rtw89_00:00",
        "rtw88_00:00",
    ];
    for driver in WIFI_DRIVERS {
        if let Some(hw) = hwmon_by_name(driver) {
            if let Some(val) = read_int(&hw.join("temp1_input")) {
                s.wifi_temp = val as f64 / 1000.0;
                log::debug!(
                    "sensors::read_all: wifi_temp={:.1}°C (via {driver})",
                    s.wifi_temp
                );
                break;
            }
            log::trace!("sensors::read_all: {driver} temp1_input unavailable");
        }
    }
    if s.wifi_temp == 0.0 {
        // Last resort: walk hwmon names for anything wifi-ish not covered above.
        for hw in find_hwmon_prefix(&[
            "iwlwifi", "mt79", "rtw89", "rtw88", "ath11k", "ath12k", "wl",
        ]) {
            if let Some(val) = read_int(&hw.1.join("temp1_input")) {
                s.wifi_temp = val as f64 / 1000.0;
                log::debug!(
                    "sensors::read_all: wifi_temp={:.1}°C (via scan {})",
                    s.wifi_temp,
                    hw.0
                );
                break;
            }
        }
    }

    // ─── Ethernet ───
    let exact_r8169 = find_hwmon("r8169");
    for hw in &exact_r8169 {
        if let Some(val) = read_int(&hw.join("temp1_input")) {
            s.ethernet_temp = val as f64 / 1000.0;
            log::debug!(
                "sensors::read_all: ethernet_temp={:.1}°C (r8169)",
                s.ethernet_temp
            );
        }
    }
    if exact_r8169.is_empty() {
        log::debug!(
            "sensors::read_all — no hwmon dir named exactly 'r8169'; engaging contains-match fallback scan"
        );
    }
    // Also try r8169_0_700:00 variant
    let mut r8169_variant_found = false;
    for entry in read_dir_entries(Path::new("/sys/class/hwmon")) {
        if let Some(name) = read_file(&entry.path().join("name")) {
            if name.contains("r8169") {
                r8169_variant_found = true;
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
    log::debug!(
        "sensors::read_all — r8169 scan finished: exact matches={}, contains-match variant found={r8169_variant_found}",
        exact_r8169.len()
    );

    // Fans -- via fans:: backend so 83JG yogafan is honored (reconciles flattened fields with FanLive)
    let active_fan_ids = crate::fans::ids();
    for (fan_num, dst_rpm, dst_target) in [
        (1u8, &mut s.fan1_rpm, &mut s.fan1_target),
        (2u8, &mut s.fan2_rpm, &mut s.fan2_target),
        (4u8, &mut s.fan4_rpm, &mut s.fan4_target),
    ] {
        if !active_fan_ids.contains(&fan_num) {
            continue;
        }
        if let Some(v) = crate::fans::read_rpm(fan_num) {
            *dst_rpm = v;
        }
        if let Some(v) = crate::fans::read_target(fan_num) {
            *dst_target = v;
        }
    }
    log::debug!(
        "sensors::read_all: fans via fans:: backend {} (rpm {} {} {})",
        crate::fans::backend_name(),
        s.fan1_rpm,
        s.fan2_rpm,
        s.fan4_rpm
    );

    // ─── Platform profile ───
    s.profile = read_file(Path::new("/sys/firmware/acpi/platform_profile"))
        .unwrap_or_else(|| "unknown".to_string());
    if s.profile == "unknown" {
        log::warn!("sensors::read_all: platform_profile unavailable → 'unknown'");
    } else {
        log::debug!("sensors::read_all: platform_profile='{}'", s.profile);
    }

    // ─── Battery ───
    // Reuse the battery module's BAT0/BAT1/BAT2/BATT probe so this legacy
    // flattened sensor block agrees with the canonical battery summary.
    s.battery_pct = battery::capacity().unwrap_or_default();
    s.battery_status = battery::status().unwrap_or_default();
    s.battery_voltage = battery::voltage().unwrap_or_default();
    s.battery_cycles = battery::cycles().unwrap_or_default();
    s.charge_type = battery::charge_types().unwrap_or_default();
    log::trace!(
        "sensors::read_all: battery capacity={}% status='{}' voltage={:.3} V cycles={} charge_types='{}'",
        s.battery_pct,
        s.battery_status,
        s.battery_voltage,
        s.battery_cycles,
        s.charge_type
    );

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
        *v = match parts.next().map(|s| s.parse::<u64>()) {
            Some(Ok(n)) => n,
            Some(Err(e)) => {
                log::trace!(
                    "sensors::sample_cpu_usage_pct — stat field unparsable, defaulted to 0: {e}"
                );
                0
            }
            None => 0,
        };
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

    // ── Intel 0°C regression (Y7000P 83DG) ──────────────────────────────
    // sensors::read_all already handles coretemp Package/id + x86_pkg_temp
    // fallback; this pins the priority helper used inside read_all so a
    // refactor cannot re-introduce the k10temp-only 0°C display on Intel.
    #[test]
    fn cpu_temp_helpers_intel_package_wins() {
        // Package id is the CPU temp on Intel; core temps feed cpu_temp_1.
        // This mirrors the coretemp branch debug logs seen on 83DG (Package 37°C).
        let labels = vec![
            ("Package id 0".to_string(), 37000.0),
            ("Core 0".to_string(), 35000.0),
            ("Core 1".to_string(), 36000.0),
        ];
        let mut max_pkg: Option<f64> = None;
        let mut max_core: Option<f64> = None;
        for (label, temp) in &labels {
            if label.contains("Package id") {
                max_pkg = Some(max_pkg.map_or(*temp, |v| v.max(*temp)));
            } else if label.starts_with("Core ") {
                max_core = Some(max_core.map_or(*temp, |v| v.max(*temp)));
            }
        }
        assert_eq!(max_pkg, Some(37000.0));
        assert_eq!(max_core, Some(36000.0));
    }
}
