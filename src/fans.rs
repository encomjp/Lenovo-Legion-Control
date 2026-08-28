//! Fan control via lenovo_wmi_other (preferred) or legion_hwmon.

use std::path::PathBuf;
use std::sync::OnceLock;

use crate::device::{self, FanCapability};
use crate::sensors::hwmon_by_name;

/// Resolve the fan sysfs backend once (stable for machine uptime) instead of
/// re-scanning `/sys/class/hwmon` on every read/write — the UI polls RPM
/// every 2 s per fan, and each old call locked the hwmon cache mutex and
/// cloned a `Vec<PathBuf>`.
fn fan_hwmon() -> Option<&'static (String, PathBuf)> {
    static BACKEND: OnceLock<Option<(String, PathBuf)>> = OnceLock::new();
    BACKEND
        .get_or_init(|| {
            if let Some(hw) = hwmon_by_name("lenovo_wmi_other") {
                // Some models (LOQ 15AHP10/83JG) bind lenovo_wmi_other but the
                // EC tachometer reads 0 — yogafan carries the live RPM there.
                if let Some(yw) = hwmon_by_name("yogafan") {
                    let any_live = (1..=4).any(|id| {
                        std::fs::read_to_string(yw.join(format!("fan{id}_input")))
                            .ok()
                            .and_then(|s| s.trim().parse::<u32>().ok())
                            .unwrap_or(0)
                            > 0
                    });
                    if any_live {
                        log::debug!(
                            "fans::fan_hwmon: lenovo_wmi_other reads 0 — using yogafan at {}",
                            yw.display()
                        );
                        return Some(("yogafan".into(), yw));
                    }
                }
                log::debug!(
                    "fans::fan_hwmon: backend lenovo_wmi_other at {}",
                    hw.display()
                );
                return Some(("lenovo_wmi_other".into(), hw));
            }
            if let Some(hw) = hwmon_by_name("legion_hwmon") {
                log::debug!("fans::fan_hwmon: backend legion_hwmon at {}", hw.display());
                return Some(("legion_hwmon".into(), hw));
            }
            // yogafan (kernel 7.1+): IdeaPad/Yoga/LOQ tachometer via ACPI EC.
            // On IdeaPad Gaming 3 (82K2) there is no lenovo_wmi_other at all —
            // this is the only fan source (read-only: no target/min/max attrs).
            if let Some(hw) = hwmon_by_name("yogafan") {
                log::debug!("fans::fan_hwmon: backend yogafan at {}", hw.display());
                return Some(("yogafan".into(), hw));
            }
            // EC direct fallback for old kernels (7.0) without yogafan or
            // for models where WMI reports 0 RPM. Reads EC registers via
            // ec_sys debugfs, as done by lenovo-fan-monitor (0xE3/0xE7 for
            // LOQ, 0x06 for IdeaPad). Only used when no hwmon exists.
            if let Some((name, _)) = ec_fallback_rpms() {
                log::info!("fans::fan_hwmon: EC fallback {name} available");
                return Some(("ec-fallback".into(), PathBuf::from(format!("/tmp/ec-fallback-{name}"))));
            }
            log::warn!(
                "fans::fan_hwmon: no fan backend (lenovo_wmi_other / legion_hwmon / yogafan) — fan control unavailable"
            );
            None
        })
        .as_ref()
}

fn fan_path(fan: u8, suffix: &str) -> Option<PathBuf> {
    let (_, hw) = fan_hwmon()?;
    let path = hw.join(format!("fan{fan}_{suffix}"));
    log::trace!("fans::fan_path: fan{fan}_{suffix} → {}", path.display());
    Some(path)
}

/// Shared sysfs u32 reader behind `read_rpm`/`read_target`/`read_min`/`read_max`.
/// Absent file → trace (routine probe, e.g. fan3 on a two-fan chassis);
/// unparsable content → warn (degraded hardware reporting).
fn read_fan_u32(fan: u8, caller: &str, path: &std::path::Path) -> Option<u32> {
    match std::fs::read_to_string(path) {
        Ok(s) => match s.trim().parse::<u32>() {
            Ok(v) => {
                log::trace!("fans::{caller}: fan{fan} → {v} ({})", path.display());
                Some(v)
            }
            Err(e) => {
                log::warn!(
                    "fans::{caller}: fan{fan} unparsable {:?} on {}: {e}",
                    s.trim(),
                    path.display()
                );
                None
            }
        },
        Err(e) => {
            log::trace!(
                "fans::{caller}: fan{fan} unavailable ({}): {e} (raw={})",
                path.display(),
                e.raw_os_error().unwrap_or(-1)
            );
            None
        }
    }
}

pub fn read_rpm(fan: u8) -> Option<u32> {
    // EC fallback path: read directly from EC registers.
    if fan_hwmon().map(|(n, _)| n.as_str() == "ec-fallback").unwrap_or(false) {
        if let Some((_, rpms)) = ec_fallback_rpms() {
            let idx = match fan {
                1 => 0,
                2 | 4 => 1,
                _ => return None,
            };
            return rpms.get(idx).copied().filter(|&v| v > 0);
        }
        return None;
    }
    let path = fan_path(fan, "input")?;
    read_fan_u32(fan, "read_rpm", &path)
}

pub fn read_target(fan: u8) -> Option<u32> {
    let path = fan_path(fan, "target")?;
    read_fan_u32(fan, "read_target", &path)
}

/// Lowest settable RPM for a fan channel. Reserved for the custom fan-curve
/// feature (docs/superpowers/plans/2026-08-25-custom-fan-curves-plan.md).
pub fn read_min(fan: u8) -> Option<u32> {
    let path = fan_path(fan, "min")?;
    read_fan_u32(fan, "read_min", &path)
}

/// Highest settable RPM for a fan channel. Reserved for the custom fan-curve
/// feature (docs/superpowers/plans/2026-08-25-custom-fan-curves-plan.md).
pub fn read_max(fan: u8) -> Option<u32> {
    let path = fan_path(fan, "max")?;
    read_fan_u32(fan, "read_max", &path)
}

/// Discovered fan channels with live min/max (falls back to model profile).
pub fn channels() -> Vec<FanCapability> {
    let caps = device::detect().capabilities.fans;
    for c in &caps {
        log::debug!(
            "fans::channels: fan {} '{}' window {}..={} rpm",
            c.id,
            c.title,
            c.min_rpm,
            c.max_rpm
        );
    }
    log::debug!("fans::channels: {} channel(s) enumerated", caps.len());
    caps
}

/// Fan ids present on this machine (e.g. `[1, 2, 4]`).
pub fn ids() -> Vec<u8> {
    channels().into_iter().map(|f| f.id).collect()
}

/// Pure helper — the formatting contract for `target == 0` means "auto" on
/// this WMI driver. Used by `rpm_label`; exported so tests pin the contract.
pub fn format_rpm_label(target: u32, rpm: u32) -> String {
    let label = if target == 0 {
        if rpm == 0 {
            "Auto".into()
        } else {
            format!("Auto · {rpm} RPM")
        }
    } else if rpm == 0 {
        format!("~{target} RPM")
    } else {
        format!("{rpm} RPM")
    };
    log::debug!("fans::format_rpm_label(target={target}, rpm={rpm}) → '{label}'");
    label
}

/// UI-friendly RPM label. Auto mode often reports 0 on this WMI driver.
pub fn rpm_label(fan: u8) -> String {
    format_rpm_label(read_target(fan).unwrap_or(0), read_rpm(fan).unwrap_or(0))
}

/// Pure: clamp a requested RPM into an explicit `min..=max` window.
/// 0 = auto passes through untouched; an inverted window is normalised so
/// `clamp` can never panic.
pub fn clamp_target_with(min: u32, max: u32, rpm: u32) -> u32 {
    if rpm == 0 {
        return 0;
    }
    let (lo, hi) = if min <= max { (min, max) } else { (max, min) };
    rpm.clamp(lo, hi)
}

/// Clamp a requested RPM into the channel's live min..max window
/// (0 = auto passes through untouched; unknown fans fall back to a sane
/// 0..=20_000 bound so garbage can't reach sysfs unchanged).
pub fn clamp_target(fan: u8, rpm: u32) -> u32 {
    let cap = channels().into_iter().find(|c| c.id == fan);
    if cap.is_none() {
        log::trace!("fans::clamp_target: fan{fan} unknown — fallback window 0..=20_000");
    }
    let (mut min, mut max) = cap.map_or((0, 20_000), |c| (c.min_rpm, c.max_rpm));
    // Live sysfs bounds win when present; capability profile is the fallback.
    if let Some(v) = read_min(fan) {
        min = v;
        log::trace!("fans::clamp_target: fan{fan} live sysfs min override {v}");
    }
    if let Some(v) = read_max(fan) {
        max = v;
        log::trace!("fans::clamp_target: fan{fan} live sysfs max override {v}");
    }
    let actual = clamp_target_with(min, max, rpm);
    if actual != rpm {
        log::info!(
            "fans::clamp_target: fan{fan} clamped requested {rpm} → {actual} (window {min}..={max})"
        );
    } else {
        log::trace!("fans::clamp_target: fan{fan} requested {rpm} within window {min}..={max}");
    }
    actual
}

/// Set fan target RPM. 0 = auto mode. Values outside the channel's window
/// are clamped before they reach sysfs.
pub fn set_target(fan: u8, rpm: u32) -> std::io::Result<()> {
    let requested = rpm;
    let rpm = clamp_target(fan, rpm);
    if rpm != requested {
        log::info!("fan {fan} requested {requested} → clamped {rpm}");
    }
    let path = match fan_path(fan, "target") {
        Some(p) => p,
        None => {
            let msg =
                format!("fans::set_target({fan},{rpm}): no fan backend/device for target write");
            log::warn!("{msg}");
            return Err(std::io::Error::new(std::io::ErrorKind::NotFound, msg));
        }
    };
    if rpm == 0 {
        log::info!("fan {fan} → auto ({})", path.display());
    } else {
        log::info!("fan {fan} → {rpm} RPM ({})", path.display());
    }
    match std::fs::write(&path, format!("{rpm}")) {
        Ok(()) => {
            log::debug!(
                "fans::set_target: sysfs write succeeded ({})",
                path.display()
            );
            Ok(())
        }
        Err(e) => {
            let ctx = format!(
                "fans::set_target({fan},{rpm}) failed on {}: {e} (kind={:?}, raw={})",
                path.display(),
                e.kind(),
                e.raw_os_error().unwrap_or(-1)
            );
            log::warn!("{ctx}");
            Err(std::io::Error::new(e.kind(), ctx))
        }
    }
}

/// EC direct fallback: read fan RPMs via ec_sys debugfs.
/// Returns (sensor_name, [cpu_rpm, gpu_rpm]) if readable and plausible.
/// Probes known EC layouts: LOQ 15IRH8 (0xE3/0xE7 16-bit LE) and
/// IdeaPad Gaming 3 (0x06 8-bit *100). Validates 500-7500 RPM range.
fn ec_fallback_rpms() -> Option<(String, Vec<u32>)> {
    use std::io::Read;
    const EC_IO: &str = "/sys/kernel/debug/ec/ec0/io";
    // Try to ensure ec_sys is loaded if the file doesn't exist.
    if !std::path::Path::new(EC_IO).exists() {
        let _ = std::process::Command::new("modprobe")
            .args(["ec_sys", "write_support=1"])
            .output();
        if !std::path::Path::new(EC_IO).exists() {
            return None;
        }
    }
    let mut buf = vec![0u8; 256];
    let mut f = std::fs::File::open(EC_IO).ok()?;
    f.read_exact(&mut buf).ok()?;
    // Probe LOQ layout (0xE3/0xE4 LE 16-bit CPU, 0xE7/0xE8 GPU)
    let loq_cpu = u16::from_le_bytes([buf[0xE3], buf[0xE4]]) as u32;
    let loq_gpu = u16::from_le_bytes([buf[0xE7], buf[0xE8]]) as u32;
    let loq_valid = (500..=7500).contains(&loq_cpu) || (500..=7500).contains(&loq_gpu);
    if loq_valid {
        return Some(("loq-ec".to_string(), vec![loq_cpu, loq_gpu]));
    }
    // Probe IdeaPad layout (0x06 8-bit *100, single fan or dual at 0x06/0x07)
    let idea_cpu = buf[0x06] as u32 * 100;
    let idea_gpu = buf[0x07] as u32 * 100;
    let idea_valid = (500..=6000).contains(&idea_cpu) || (500..=6000).contains(&idea_gpu);
    if idea_valid {
        // If only one fan has data, duplicate to second slot for UI.
        let gpu = if idea_gpu > 0 { idea_gpu } else { idea_cpu };
        return Some(("ideapad-ec".to_string(), vec![idea_cpu, gpu]));
    }
    // Legion alternative (0xC5E0/0xC5E2 via EC RAM, but ec_sys may not expose there)
    let leg_cpu = u16::from_le_bytes([buf[0xE0], buf[0xE1]]) as u32;
    let leg_gpu = u16::from_le_bytes([buf[0xE2], buf[0xE3]]) as u32;
    if (500..=7500).contains(&leg_cpu) || (500..=7500).contains(&leg_gpu) {
        return Some(("legion-ec".to_string(), vec![leg_cpu, leg_gpu]));
    }
    None
}

/// Set all discovered fans to auto mode.
pub fn set_auto() -> std::io::Result<()> {
    let all_ids = ids();
    log::info!(
        "fans::set_auto: switching to auto on {} fan(s): {all_ids:?}",
        all_ids.len()
    );
    let mut last_err = None;
    let mut errors = 0usize;
    for id in all_ids {
        match set_target(id, 0) {
            Ok(()) => log::debug!("fans::set_auto: fan{id} → auto ok"),
            Err(e) => {
                log::warn!("fans::set_auto: fan{id} failed to switch to auto: {e}");
                errors += 1;
                last_err = Some(e);
            }
        }
    }
    match last_err {
        Some(e) => {
            log::warn!("fans::set_auto: {errors} fan(s) failed to switch to auto");
            Err(e)
        }
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_auto_covers_all_combos() {
        assert_eq!(format_rpm_label(0, 0), "Auto");
        assert_eq!(format_rpm_label(0, 1800), "Auto · 1800 RPM");
        assert_eq!(format_rpm_label(1500, 0), "~1500 RPM");
        assert_eq!(format_rpm_label(1500, 1400), "1400 RPM");
    }

    #[test]
    fn clamp_within_range_passes_through() {
        assert_eq!(clamp_target_with(1200, 5500, 3000), 3000);
        assert_eq!(clamp_target_with(1200, 5500, 1200), 1200);
        assert_eq!(clamp_target_with(1200, 5500, 5500), 5500);
    }

    #[test]
    fn clamp_below_min_snaps_to_min() {
        assert_eq!(clamp_target_with(1200, 5500, 0), 0); // 0 = auto, untouched
        assert_eq!(clamp_target_with(1200, 5500, 1), 1200);
        assert_eq!(clamp_target_with(1200, 5500, 1199), 1200);
    }

    #[test]
    fn clamp_above_max_snaps_to_max() {
        assert_eq!(clamp_target_with(1200, 5500, 5501), 5500);
        assert_eq!(clamp_target_with(0, 20_000, u32::MAX), 20_000);
    }

    #[test]
    fn clamp_zero_is_auto_passthrough() {
        assert_eq!(clamp_target_with(0, 5500, 0), 0);
        assert_eq!(clamp_target_with(1200, 5500, 0), 0);
    }

    #[test]
    fn clamp_unknown_fan_fallback_bound() {
        // Unknown fans resolve to the sane 0..=20_000 fallback window.
        assert_eq!(clamp_target_with(0, 20_000, 10_000), 10_000);
        assert_eq!(clamp_target_with(0, 20_000, 25_000), 20_000);
        assert_eq!(clamp_target_with(0, 20_000, 5), 5);
    }

    #[test]
    fn clamp_inverted_window_never_panics() {
        // Defensive: min > max (bad sysfs values) must not panic `u32::clamp`.
        assert_eq!(clamp_target_with(5500, 1200, 3000), 3000);
        assert_eq!(clamp_target_with(5500, 1200, 100), 1200);
        assert_eq!(clamp_target_with(5500, 1200, 9999), 5500);
    }
}
