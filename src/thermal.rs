use std::fs;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const MAX_FULL: u32 = 5_460_527;
pub const MIN: u32 = 4_600_000;
pub const STEP: u32 = 200_000;
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
    pub tctl_mC: Option<i32>,
    pub tccd2_mC: Option<i32>,
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

pub fn compute_target(cur_max: u32, temp_mC: i32, cfg: &ThermalConfig) -> Option<u32> {
    if !cfg.enabled {
        return None;
    }
    let max_mC = cfg.max_temp as i32 * 1000;
    let restore_mC = (cfg.max_temp as i32 - HYSTERESIS) * 1000;
    if temp_mC >= max_mC && cur_max > MIN {
        Some(cur_max.saturating_sub(STEP).max(MIN))
    } else if temp_mC <= restore_mC && cur_max < MAX_FULL {
        Some(cur_max.saturating_add(STEP).min(MAX_FULL))
    } else {
        None
    }
}

pub fn read_thermal_temps() -> (Option<i32>, Option<i32>) {
    let base = Path::new("/sys/class/hwmon");
    if let Ok(entries) = fs::read_dir(base) {
        for entry in entries.flatten() {
            let name_path = entry.path().join("name");
            if let Ok(name) = fs::read_to_string(&name_path) {
                if name.trim() == "k10temp" {
                    let hw = entry.path();
                    let tctl = fs::read_to_string(hw.join("temp1_input"))
                        .ok()
                        .and_then(|s| s.trim().parse::<i32>().ok());
                    let tccd2 = fs::read_to_string(hw.join("temp4_input"))
                        .ok()
                        .and_then(|s| s.trim().parse::<i32>().ok())
                        .or_else(|| {
                            fs::read_to_string(hw.join("temp3_input"))
                                .ok()
                                .and_then(|s| s.trim().parse::<i32>().ok())
                        });
                    return (tctl, tccd2);
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
    fn compute_target_throttles_at_max() {
        let cfg = ThermalConfig {
            enabled: true,
            max_temp: 90,
        };
        // temp ≥ 90°C (90000 mC), cur > MIN → step down
        assert_eq!(compute_target(5_460_527, 90_000, &cfg), Some(5_260_527));
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
        // ≤83°C (90000-7000) and cur < MAX_FULL → step up
        assert_eq!(compute_target(4_600_000, 83_000, &cfg), Some(4_800_000));
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
        assert_eq!(compute_target(4_800_000, 83_000, &cfg), Some(5_000_000));
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
}
