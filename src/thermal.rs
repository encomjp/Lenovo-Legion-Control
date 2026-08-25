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
    if overshoot_mc >= URGENT_OVERSHOOT_MC {
        STEP_URGENT
    } else if overshoot_mc >= 2_000 {
        STEP_MODERATE
    } else {
        STEP_GENTLE
    }
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
        let next = match self.smoothed {
            None => temp_mc,
            Some(prev) => (prev + temp_mc) / 2,
        };
        self.smoothed = Some(next);
        next
    }

    /// Effective temperature for a governor decision: urgent overshoot
    /// bypasses (and re-seeds) the filter so protection stays fast.
    pub fn effective(&mut self, raw_mc: i32, limit_mc: i32) -> i32 {
        if raw_mc >= limit_mc + URGENT_OVERSHOOT_MC {
            self.smoothed = Some(raw_mc);
            raw_mc
        } else {
            self.update(raw_mc)
        }
    }
}

pub fn compute_target(cur_max: u32, temp_mc: i32, cfg: &ThermalConfig) -> Option<u32> {
    if !cfg.enabled {
        return None;
    }
    let max_mc = cfg.max_temp as i32 * 1000;
    let restore_mc = (cfg.max_temp as i32 - HYSTERESIS) * 1000;
    if temp_mc >= max_mc && cur_max > MIN {
        Some(cur_max.saturating_sub(down_step(temp_mc - max_mc)).max(MIN))
    } else if temp_mc <= restore_mc && cur_max < MAX_FULL {
        Some(cur_max.saturating_add(STEP_UP).min(MAX_FULL))
    } else {
        None
    }
}

/// Reads the main CPU temperature (the AMD Tctl sensor, hwmon `temp1_input`)
/// and a per-CCD temperature (AMD Tccd1/Tccd2 sensors, hwmon `temp4_input`
/// with fallback to `temp3_input`).
pub fn read_cpu_temps() -> (Option<i32>, Option<i32>) {
    let base = Path::new("/sys/class/hwmon");
    if let Ok(entries) = fs::read_dir(base) {
        for entry in entries.flatten() {
            let name_path = entry.path().join("name");
            if let Ok(name) = fs::read_to_string(&name_path) {
                if name.trim() == "k10temp" {
                    let hw = entry.path();
                    let cpu_temp = fs::read_to_string(hw.join("temp1_input"))
                        .ok()
                        .and_then(|s| s.trim().parse::<i32>().ok());
                    let cpu_temp_2 = fs::read_to_string(hw.join("temp4_input"))
                        .ok()
                        .and_then(|s| s.trim().parse::<i32>().ok())
                        .or_else(|| {
                            fs::read_to_string(hw.join("temp3_input"))
                                .ok()
                                .and_then(|s| s.trim().parse::<i32>().ok())
                        });
                    return (cpu_temp, cpu_temp_2);
                }
            }
        }
    }
    (None, None)
}

pub fn read_cur_max() -> Option<u32> {
    fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_max_freq")
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
}

pub fn write_all_cpus(freq: u32) -> Result<(), String> {
    let base = Path::new("/sys/devices/system/cpu");
    let entries = fs::read_dir(base).map_err(|e| e.to_string())?;
    let mut found = false;
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
            if let Err(e) = fs::write(&p, freq.to_string()) {
                last_err = Some(format!("{}: {e}", p.display()));
            }
        }
    }
    if !found {
        return Err("no cpu*/cpufreq/scaling_max_freq found".into());
    }
    if let Some(e) = last_err {
        return Err(e);
    }
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
}
