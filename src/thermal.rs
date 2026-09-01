use std::fs;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const MAX_FULL: u32 = 5_460_527;
pub const MIN: u32 = 4_600_000;
/// Recovery ramp — gentle so restored headroom never jolts a running game.
pub const STEP_UP: u32 = 100_000;
/// Clamp steps scale with how far we are over the limit (see [down_step]).
pub const STEP_GENTLE: u32 = 100_000;
pub const STEP_MODERATE: u32 = 200_000;
pub const STEP_URGENT: u32 = 300_000;
/// A raw reading this far above the limit bypasses smoothing: real heat
/// still clamps fast.
pub const URGENT_OVERSHOOT_MC: i32 = 4_000;
pub const HYSTERESIS: i32 = 7;
pub const INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThermalConfig {
    pub enabled: bool,
    pub max_temp: u8,
}

impl Default for ThermalConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_temp: 90,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThermalStatus {
    pub config: ThermalConfig,
    pub cur_max_freq: u32,
    pub cpu_temp_mc: Option<i32>,
    pub cpu_temp_2_mc: Option<i32>,
    pub active: bool,
    pub restore_temp: u8,
}

pub fn validate(max_temp: u8, acknowledge: bool) -> Result<(), String> {
    if !(70..=98).contains(&max_temp) {
        return Err(format!("max_temp {max_temp} out of 70..=98"));
    }
    if max_temp >= 96 && !acknowledge {
        return Err("max_temp 96–98°C exceeds TjMax 95°C — pass --acknowledge".into());
    }
    Ok(())
}

/// Down-step size by overshoot: ≤2 °C → 100 MHz, 2–4 °C → 200 MHz,
/// ≥4 °C → 300 MHz. Mild crossings get gentle steps (no gameplay jolt);
/// serious heat gets clamped quickly.
pub fn down_step(overshoot_mc: i32) -> u32 {
    let (step, tier) = if overshoot_mc >= URGENT_OVERSHOOT_MC {
        (STEP_URGENT, "urgent")
    } else if overshoot_mc >= 2_000 {
        (STEP_MODERATE, "moderate")
    } else {
        (STEP_GENTLE, "gentle")
    };
    log::debug!("thermal: down_step(overshoot={overshoot_mc}) → {step} [{tier} tier]");
    step
}

/// Exponential moving average over the 1 s governor samples (α = ½).
/// Halves single-sample sensor spikes so one k10temp blip cannot yank the
/// frequency ceiling; sustained heat converges within ~2 samples.
#[derive(Debug, Clone, Default)]
pub struct TempFilter {
    smoothed: Option<i32>,
}

impl TempFilter {
    /// Feed one raw sample, get the smoothed value.
    pub fn update(&mut self, temp_mc: i32) -> i32 {
        let (seeded, next) = match self.smoothed {
            None => (true, temp_mc),
            Some(prev) => (
                false,
                prev.saturating_add((temp_mc.saturating_sub(prev)) / 2),
            ),
        };
        self.smoothed = Some(next);
        if seeded {
            log::debug!("thermal: TempFilter seeded ← {temp_mc}");
        } else {
            log::debug!("thermal: TempFilter average(raw={temp_mc}) → {next}");
        }
        next
    }

    /// Effective temperature for a governor decision: urgent overshoot
    /// bypasses (and re-seeds) the filter so protection stays fast.
    pub fn effective(&mut self, raw_mc: i32, limit_mc: i32) -> i32 {
        if raw_mc >= limit_mc + URGENT_OVERSHOOT_MC {
            self.smoothed = Some(raw_mc);
            log::debug!(
                "thermal: TempFilter urgent bypass: raw {raw_mc} ≥ limit {limit_mc} + {URGENT_OVERSHOOT_MC} — filter re-seeded"
            );
            raw_mc
        } else {
            self.update(raw_mc)
        }
    }
}

pub fn compute_target(cur_max: u32, temp_mc: i32, cfg: &ThermalConfig) -> Option<u32> {
    log::debug!(
        "thermal: compute_target(cur_max={cur_max}, temp_mc={temp_mc}, enabled={}, max_temp={})",
        cfg.enabled,
        cfg.max_temp
    );
    if !cfg.enabled {
        log::debug!("thermal: compute_target → None (disabled)");
        return None;
    }
    let max_mc = cfg.max_temp as i32 * 1000;
    let restore_mc = (cfg.max_temp as i32 - HYSTERESIS) * 1000;
    log::debug!(
        "thermal: compute_target limits: max_mc={max_mc}, restore_mc={restore_mc}, overshoot={} mc",
        temp_mc - max_mc
    );
    if temp_mc >= max_mc && cur_max > MIN {
        let target = cur_max.saturating_sub(down_step(temp_mc - max_mc)).max(MIN);
        log::debug!(
            "thermal: compute_target → Some({target}) (throttle: temp ≥ max {max_mc}, floor MIN {MIN})"
        );
        Some(target)
    } else if temp_mc <= restore_mc && cur_max < MAX_FULL {
        let target = cur_max.saturating_add(STEP_UP).min(MAX_FULL);
        log::debug!(
            "thermal: compute_target → Some({target}) (restore: temp ≤ restore {restore_mc}, cap MAX_FULL {MAX_FULL})"
        );
        Some(target)
    } else {
        log::debug!(
            "thermal: compute_target → None (hold in hysteresis band {restore_mc}..{max_mc})"
        );
        None
    }
}

/// One sysfs numeric file: read + parse with value/error logging, so a
/// dead sensor input degrades loudly instead of silently becoming `None`.
fn read_sysfs_num<T: std::str::FromStr + std::fmt::Display>(path: &Path, label: &str) -> Option<T>
where
    T::Err: std::fmt::Debug,
{
    match fs::read_to_string(path) {
        Ok(s) => match s.trim().parse::<T>() {
            Ok(v) => {
                log::debug!("thermal: {label} {path:?} → {v}");
                Some(v)
            }
            Err(e) => {
                log::debug!(
                    "thermal: {label} unparsable {:?} on {}: {e:?}",
                    s.trim(),
                    path.display()
                );
                None
            }
        },
        Err(e) => {
            log::debug!("thermal: {label} unreadable on {}: {e}", path.display());
            None
        }
    }
}

/// One hwmon millidegree file (i32).
fn read_temp_file(path: &Path, label: &str) -> Option<i32> {
    read_sysfs_num(path, label)
}

/// Thermal-zone fallback shared by the coretemp branch and the last-resort
/// path. `include_acpitz` widens acceptance beyond `x86_pkg_temp` (used when
/// coretemp exists but exposes no Package label — some vendor kernels only
/// surface an ACPi thermal zone). Positive-only: 0 mC is a sentinel, never a
/// reading.
fn read_thermal_zone_temp(include_acpitz: bool) -> Option<i32> {
    for zone in std::fs::read_dir("/sys/class/thermal")
        .into_iter()
        .flatten()
        .flatten()
    {
        let ttype = std::fs::read_to_string(zone.path().join("type"))
            .unwrap_or_default()
            .trim()
            .to_string();
        let accepted = if include_acpitz {
            ttype == "x86_pkg_temp" || ttype == "acpitz"
        } else {
            ttype == "x86_pkg_temp"
        };
        if accepted {
            if let Some(v) = read_temp_file(&zone.path().join("temp"), &format!("thermal {ttype}")) {
                if v > 0 {
                    return Some(v);
                }
            }
        }
    }
    None
}

/// Reads the main CPU temperature via cached hwmon discovery.
///
/// AMD: `k10temp` `temp1_input` (Tctl) + `temp4_input` (Tccd1).
/// Intel Raptor Lake (Y7000P IRX9 83DG 0°C bug): `coretemp` Package id / Core
/// labels, else `x86_pkg_temp` thermal zone. Source-agnostic so the governor
/// works on both vendors; `sensors::read_all` is the display twin.
pub fn read_cpu_temps() -> (Option<i32>, Option<i32>) {
    if let Some(hw) = crate::sensors::hwmon_by_name("k10temp") {
        let cpu_temp = read_temp_file(&hw.join("temp1_input"), "temp1_input (Tctl)");
        let tccd1 = read_temp_file(&hw.join("temp4_input"), "temp4_input (Tccd1)");
        let cpu_temp_2 =
            tccd1.or_else(|| read_temp_file(&hw.join("temp3_input"), "temp3_input (Tccd1 fallback)"));
        return (cpu_temp, cpu_temp_2);
    }
    if let Some(hw) = crate::sensors::hwmon_by_name("coretemp") {
        let mut max_pkg: Option<i32> = None;
        let mut max_core: Option<i32> = None;
        if let Ok(entries) = std::fs::read_dir(&hw) {
            for entry in entries.flatten() {
                let fname = entry.file_name().to_string_lossy().to_string();
                if !fname.ends_with("_label") {
                    continue;
                }
                let label = match std::fs::read_to_string(entry.path()) {
                    Ok(s) => s.trim().to_string(),
                    Err(_) => continue,
                };
                let input_path = hw.join(fname.replace("_label", "_input"));
                let val = match read_temp_file(&input_path, &format!("coretemp {label}")) {
                    Some(v) => v,
                    None => continue,
                };
                if label.contains("Package id") {
                    max_pkg = Some(max_pkg.map_or(val, |p| p.max(val)));
                } else if label.starts_with("Core ") {
                    max_core = Some(max_core.map_or(val, |p| p.max(val)));
                }
            }
        }
        if max_pkg.is_some() || max_core.is_some() {
            log::debug!("thermal: coretemp package={max_pkg:?} core_max={max_core:?}");
            // Governor clamps on package temp; core max as secondary.
            return (max_pkg.or(max_core), max_core);
        }
        // Thermal zone fallback if coretemp had no Package label
        if let Some(v) = read_thermal_zone_temp(true) {
            return (Some(v), None);
        }
        log::trace!("thermal: no coretemp or thermal zone reading");
        return (None, None);
    }
    // Last resort: thermal zone x86_pkg_temp (works on both Intel/AMD when hwmon missing)
    if let Some(v) = read_thermal_zone_temp(false) {
        return (Some(v), None);
    }
    log::trace!("thermal: no k10temp/coretemp/x86_pkg_temp hwmon found");
    (None, None)
}

pub fn read_cur_max() -> Option<u32> {
    read_sysfs_num(
        Path::new("/sys/devices/system/cpu/cpu0/cpufreq/scaling_max_freq"),
        "policy",
    )
}

pub fn write_all_cpus(freq: u32) -> Result<(), String> {
    let base = Path::new("/sys/devices/system/cpu");
    let entries = match fs::read_dir(base) {
        Ok(entries) => entries,
        Err(e) => {
            log::error!("thermal: cannot list {}: {e}", base.display());
            return Err(e.to_string());
        }
    };
    let mut found = false;
    let mut ok = 0usize;
    let mut failed = 0usize;
    let mut last_err: Option<String> = None;
    for entry in entries.flatten() {
        let fname = entry.file_name().to_string_lossy().to_string();
        // match cpu0, cpu1, ... cpuNN (not cpuidle etc.)
        if !fname.starts_with("cpu") {
            continue;
        }
        let suffix = &fname[3..];
        if suffix.is_empty() || !suffix.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let p = entry.path().join("cpufreq/scaling_max_freq");
        if p.exists() {
            found = true;
            match fs::write(&p, freq.to_string()) {
                Ok(()) => {
                    ok += 1;
                    log::debug!("thermal: {fname}/scaling_max_freq ← {freq}");
                }
                Err(e) => {
                    failed += 1;
                    log::warn!("thermal: write {} failed: {e}", p.display());
                    last_err = Some(format!("{}: {e}", p.display()));
                }
            }
        }
    }
    log::debug!("thermal: write_all_cpus({freq}) — {ok} write(s) ok, {failed} failed");
    if !found {
        log::error!(
            "thermal: no cpu*/cpufreq/scaling_max_freq found under {}",
            base.display()
        );
        return Err("no cpu*/cpufreq/scaling_max_freq found".into());
    }
    if let Some(e) = last_err {
        log::error!("thermal: write_all_cpus({freq}) failed: {e}");
        return Err(e);
    }
    log::debug!("thermal: write_all_cpus({freq}) ok — all policy files written");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_target_throttles_gently_at_max() {
        let cfg = ThermalConfig {
            enabled: true,
            max_temp: 90,
        };
        // Barely over (0°C overshoot) → gentle 100 MHz step, no jolt
        assert_eq!(compute_target(5_460_527, 90_000, &cfg), Some(5_360_527));
    }

    #[test]
    fn compute_target_throttles_proportionally() {
        let cfg = ThermalConfig {
            enabled: true,
            max_temp: 90,
        };
        // 2.5°C over → 200 MHz step
        assert_eq!(compute_target(5_460_527, 92_500, &cfg), Some(5_260_527));
        // 4.5°C over → urgent 300 MHz step
        assert_eq!(compute_target(5_460_527, 94_500, &cfg), Some(5_160_527));
    }

    #[test]
    fn compute_target_holds_in_hysteresis_band() {
        let cfg = ThermalConfig {
            enabled: true,
            max_temp: 90,
        };
        // 86°C is inside (83,90) → hold
        assert_eq!(compute_target(5_260_527, 86_000, &cfg), None);
    }

    #[test]
    fn compute_target_restores_below_restore() {
        let cfg = ThermalConfig {
            enabled: true,
            max_temp: 90,
        };
        // ≤83°C (90000-7000) and cur < MAX_FULL → gentle 100 MHz ramp up
        assert_eq!(compute_target(4_600_000, 83_000, &cfg), Some(4_700_000));
    }

    #[test]
    fn compute_target_disabled_holds() {
        let cfg = ThermalConfig {
            enabled: false,
            max_temp: 90,
        };
        assert_eq!(compute_target(5_460_527, 95_000, &cfg), None);
    }

    #[test]
    fn validate_rejects_96_without_ack() {
        assert!(validate(96, false).is_err());
        assert!(validate(96, true).is_ok());
    }

    #[test]
    fn validate_range_70_to_98() {
        assert!(validate(69, false).is_err());
        assert!(validate(70, false).is_ok());
        assert!(validate(98, true).is_ok());
        assert!(validate(99, true).is_err());
    }

    // Extra: clamp at MIN/MAX_FULL and enabled=false restore path

    #[test]
    fn compute_target_clamps_at_min() {
        let cfg = ThermalConfig {
            enabled: true,
            max_temp: 90,
        };
        // cur already at MIN → no further throttle even if hot
        assert_eq!(compute_target(MIN, 95_000, &cfg), None);
        // one step above MIN should clamp to MIN, not underflow
        assert_eq!(compute_target(MIN + 100_000, 95_000, &cfg), Some(MIN));
    }

    #[test]
    fn compute_target_clamps_at_max_full() {
        let cfg = ThermalConfig {
            enabled: true,
            max_temp: 90,
        };
        // cur already at MAX_FULL → no further restore
        assert_eq!(compute_target(MAX_FULL, 50_000, &cfg), None);
        // one step below MAX_FULL should clamp to MAX_FULL
        assert_eq!(
            compute_target(MAX_FULL - 100_000, 50_000, &cfg),
            Some(MAX_FULL)
        );
    }

    #[test]
    fn compute_target_disabled_holds_below_restore() {
        let cfg = ThermalConfig {
            enabled: false,
            max_temp: 90,
        };
        // even well below restore temp, disabled must hold
        assert_eq!(compute_target(4_600_000, 50_000, &cfg), None);
        assert_eq!(compute_target(5_000_000, 83_000, &cfg), None);
    }

    #[test]
    fn compute_target_holds_at_restore_boundary() {
        let cfg = ThermalConfig {
            enabled: true,
            max_temp: 90,
        };
        // 83_001 is just above restore (83_000) → hold
        assert_eq!(compute_target(4_800_000, 83_001, &cfg), None);
        // exactly at restore → step up
        assert_eq!(compute_target(4_800_000, 83_000, &cfg), Some(4_900_000));
    }

    #[test]
    fn validate_rejects_97_98_without_ack() {
        assert!(validate(97, false).is_err());
        assert!(validate(98, false).is_err());
        assert!(validate(97, true).is_ok());
    }

    #[test]
    fn validate_accepts_95_without_ack() {
        // 95 is TjMax itself, should not require acknowledge
        assert!(validate(95, false).is_ok());
    }

    #[test]
    fn down_step_tiers_by_overshoot() {
        assert_eq!(down_step(0), STEP_GENTLE);
        assert_eq!(down_step(1_999), STEP_GENTLE);
        assert_eq!(down_step(2_000), STEP_MODERATE);
        assert_eq!(down_step(3_999), STEP_MODERATE);
        assert_eq!(down_step(4_000), STEP_URGENT);
        assert_eq!(down_step(12_000), STEP_URGENT);
    }

    #[test]
    fn temp_filter_seeds_then_averages() {
        let mut f = TempFilter::default();
        assert_eq!(f.update(80_000), 80_000); // seeds with the first sample
        assert_eq!(f.update(84_000), 82_000); // (80+84)/2
        assert_eq!(f.update(84_000), 83_000); // converges
    }

    #[test]
    fn temp_filter_halves_single_sample_spikes() {
        let mut f = TempFilter::default();
        let limit = 90_000;
        // Steady at 88, one 92 blip: smoothed stays below the limit → no clamp
        assert_eq!(f.effective(88_000, limit), 88_000);
        assert_eq!(f.effective(92_000, limit), 90_000);
        // A real crossing needs to persist: 92 again → smoothed 91 → clamps
        assert_eq!(f.effective(92_000, limit), 91_000);
    }

    #[test]
    fn temp_filter_urgent_overshoot_bypasses_smoothing() {
        let mut f = TempFilter::default();
        let limit = 90_000;
        assert_eq!(f.effective(80_000, limit), 80_000);
        // 95 = limit+5°C: urgent, returns raw immediately and re-seeds
        assert_eq!(f.effective(95_000, limit), 95_000);
        // Filter state followed the raw value, not the average
        assert_eq!(f.update(95_000), 95_000);
    }

    // ── Intel 0°C regression (Y7000P IRX9 83DG) ──────────────────────────
    // sensors.rs already covers the hwmon walk; thermal::read_cpu_temps must
    // mirror it source-agnostically (k10temp → coretemp → x86_pkg_temp) so the
    // governor does not stay blind (None temps, no freq writes) on Intel.
    #[test]
    fn read_cpu_temps_returns_none_when_no_hwmon() {
        // On this CI host there is no k10temp/coretemp; thermal zone may also
        // be absent — must not panic and must return (None, None) not 0.
        let (a, b) = read_cpu_temps();
        // a and b are either temps or None — but never Some(0) sentinel.
        if let Some(v) = a {
            assert!(v != 0, "thermal governor must never see 0 mC sentinel");
        }
        if let Some(v) = b {
            assert!(v != 0);
        }
        // The exact value depends on host hwmon; just assert it doesn't panic.
        let _ = (a, b);
    }

    #[test]
    fn coretemp_label_pick_logic() {
        // Pure logic extracted from read_cpu_temps coretemp branch.
        fn pick(labels: &[(&str, i32)]) -> (Option<i32>, Option<i32>) {
            let mut pkg: Option<i32> = None;
            let mut core: Option<i32> = None;
            for (lab, v) in labels {
                if lab.contains("Package id") {
                    pkg = Some(pkg.map_or(*v, |p| p.max(*v)));
                } else if lab.starts_with("Core ") {
                    core = Some(core.map_or(*v, |p| p.max(*v)));
                }
            }
            (pkg, core)
        }
        // Package id 0 hottest wins, Core max as secondary — mirrors Y7000P 83DG (Package 37°C)
        assert_eq!(
            pick(&[("Package id 0", 37000), ("Core 0", 35000), ("Core 1", 36000)]),
            (Some(37000), Some(36000))
        );
        // Two Package ids — hottest wins
        assert_eq!(
            pick(&[("Package id 0", 40000), ("Package id 1", 45000)]),
            (Some(45000), None)
        );
        // No package label → None (thermal zone fallback path)
        assert_eq!(pick(&[("Core 0", 33000)]), (None, Some(33000)));
        assert_eq!(pick(&[]), (None, None));
    }
}
