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
    static BACKEND: OnceLock<(String, PathBuf)> = OnceLock::new();
    if let Some(backend) = BACKEND.get() {
        return Some(backend);
    }
    let discovered = discover_fan_hwmon()?;
    let _ = BACKEND.set(discovered);
    BACKEND.get()
}

fn has_fan_inputs(hw: &std::path::Path) -> bool {
    std::fs::read_dir(hw).is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with("fan") && name.ends_with("_input")
        })
    })
}

fn has_live_fan_input(hw: &std::path::Path) -> bool {
    (1..=4).any(|id| {
        std::fs::read_to_string(hw.join(format!("fan{id}_input")))
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(0)
            > 0
    })
}

fn has_fan_targets(hw: &std::path::Path) -> bool {
    std::fs::read_dir(hw).is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with("fan") && name.ends_with("_target")
        })
    })
}

fn fan_control_hwmon() -> Option<&'static (String, PathBuf)> {
    static BACKEND: OnceLock<(String, PathBuf)> = OnceLock::new();
    if let Some(backend) = BACKEND.get() {
        return Some(backend);
    }
    let discovered = ["lenovo_wmi_other", "legion_hwmon"]
        .into_iter()
        .find_map(|name| {
            hwmon_by_name(name)
                .filter(|hw| has_fan_targets(hw))
                .map(|hw| (name.to_string(), hw))
        })?;
    let _ = BACKEND.set(discovered);
    BACKEND.get()
}

fn discover_fan_hwmon() -> Option<(String, PathBuf)> {
    if let Some(hw) = hwmon_by_name("lenovo_wmi_other").filter(|hw| has_fan_inputs(hw)) {
        // Some models bind lenovo_wmi_other but its tachometer reads 0 while
        // yogafan carries live RPM values.
        if let Some(yw) = hwmon_by_name("yogafan").filter(|hw| has_fan_inputs(hw)) {
            // Match device::probe_fans exactly: switch only when yogafan has
            // a live reading and every WMI reading is zero/unreadable.
            if has_live_fan_input(&yw) && !has_live_fan_input(&hw) {
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
    if let Some(hw) = hwmon_by_name("legion_hwmon").filter(|hw| has_fan_inputs(hw)) {
        log::debug!("fans::fan_hwmon: backend legion_hwmon at {}", hw.display());
        return Some(("legion_hwmon".into(), hw));
    }
    if let Some(hw) = hwmon_by_name("yogafan").filter(|hw| has_fan_inputs(hw)) {
        log::debug!("fans::fan_hwmon: backend yogafan at {}", hw.display());
        return Some(("yogafan".into(), hw));
    }
    if let Some((name, _)) = ec_fallback_rpms() {
        log::info!("fans::fan_hwmon: EC fallback {name} available");
        return Some((
            "ec-fallback".into(),
            PathBuf::from(format!("/tmp/ec-fallback-{name}")),
        ));
    }
    log::warn!("fans::fan_hwmon: no RPM backend (lenovo_wmi_other / legion_hwmon / yogafan)");
    None
}

fn fan_control_path(fan: u8, suffix: &str) -> Option<PathBuf> {
    let (_, hw) = fan_control_hwmon()?;
    let path = hw.join(format!("fan{fan}_{suffix}"));
    log::trace!(
        "fans::fan_control_path: fan{fan}_{suffix} → {}",
        path.display()
    );
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FanRpmState {
    Readable,
    NotExposed,
    BackendUnavailable,
    Unreadable,
}

impl FanRpmState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Readable => "readable",
            Self::NotExposed => "not-exposed",
            Self::BackendUnavailable => "backend-unavailable",
            Self::Unreadable => "unreadable",
        }
    }
}

pub fn rpm_status(fan: u8) -> (Option<u32>, FanRpmState) {
    let Some((name, hw)) = fan_hwmon() else {
        return (None, FanRpmState::BackendUnavailable);
    };
    if name == "ec-fallback" {
        let Some(layout) = hw
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix("ec-fallback-"))
        else {
            return (None, FanRpmState::Unreadable);
        };
        let Some(rpms) = ec_fallback_rpms_for(layout) else {
            return (None, FanRpmState::Unreadable);
        };
        let index = match fan {
            1 => 0,
            2 | 4 => 1,
            _ => return (None, FanRpmState::NotExposed),
        };
        return match rpms.get(index).copied() {
            Some(rpm) => (Some(rpm), FanRpmState::Readable),
            None => (None, FanRpmState::NotExposed),
        };
    }

    let path = hw.join(format!("fan{fan}_input"));
    if !path.is_file() {
        return (None, FanRpmState::NotExposed);
    }
    match read_fan_u32(fan, "read_rpm", &path) {
        Some(rpm) => (Some(rpm), FanRpmState::Readable),
        None => (None, FanRpmState::Unreadable),
    }
}

pub fn read_rpm(fan: u8) -> Option<u32> {
    rpm_status(fan).0
}

pub fn read_target(fan: u8) -> Option<u32> {
    let path = fan_control_path(fan, "target")?;
    read_fan_u32(fan, "read_target", &path)
}

/// Lowest settable RPM for a fan channel. Reserved for the custom fan-curve
/// feature (docs/superpowers/plans/2026-08-25-custom-fan-curves-plan.md).
pub fn read_min(fan: u8) -> Option<u32> {
    let path = fan_control_path(fan, "min")?;
    read_fan_u32(fan, "read_min", &path)
}

/// Highest settable RPM for a fan channel. Reserved for the custom fan-curve
/// feature (docs/superpowers/plans/2026-08-25-custom-fan-curves-plan.md).
pub fn read_max(fan: u8) -> Option<u32> {
    let path = fan_control_path(fan, "max")?;
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

pub fn backend_name() -> String {
    fan_hwmon()
        .map(|(name, _)| name.clone())
        .unwrap_or_else(|| device::detect().capabilities.fan_backend)
}

pub fn control_backend_name() -> Option<String> {
    fan_control_hwmon().map(|(name, _)| name.clone())
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
    let path = match fan_control_path(fan, "target") {
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
    let buf = ec_fallback_bytes()?;
    for layout in ["loq-ec", "ideapad-ec", "legion-ec"] {
        let rpms = ec_layout_rpms(layout, &buf)?;
        let ceiling = if layout == "ideapad-ec" { 6000 } else { 7500 };
        if rpms.iter().any(|rpm| (500..=ceiling).contains(rpm)) {
            return Some((layout.to_string(), rpms));
        }
    }
    None
}

fn ec_fallback_rpms_for(layout: &str) -> Option<Vec<u32>> {
    let buf = ec_fallback_bytes()?;
    ec_layout_rpms(layout, &buf)
}

fn ec_fallback_bytes() -> Option<Vec<u8>> {
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
    Some(buf)
}

fn ec_layout_rpms(layout: &str, buf: &[u8]) -> Option<Vec<u32>> {
    if buf.len() < 0xe9 {
        return None;
    }
    match layout {
        "loq-ec" => Some(vec![
            u16::from_le_bytes([buf[0xe3], buf[0xe4]]) as u32,
            u16::from_le_bytes([buf[0xe7], buf[0xe8]]) as u32,
        ]),
        "ideapad-ec" => {
            let cpu = buf[0x06] as u32 * 100;
            let gpu = buf[0x07] as u32 * 100;
            Some(vec![cpu, if gpu > 0 { gpu } else { cpu }])
        }
        "legion-ec" => Some(vec![
            u16::from_le_bytes([buf[0xe0], buf[0xe1]]) as u32,
            u16::from_le_bytes([buf[0xe2], buf[0xe3]]) as u32,
        ]),
        _ => None,
    }
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

    #[test]
    fn ec_layout_keeps_zero_rpm_as_a_valid_reading() {
        let buf = vec![0u8; 256];
        assert_eq!(ec_layout_rpms("loq-ec", &buf), Some(vec![0, 0]));
        assert_eq!(ec_layout_rpms("ideapad-ec", &buf), Some(vec![0, 0]));
        assert_eq!(ec_layout_rpms("legion-ec", &buf), Some(vec![0, 0]));
        assert_eq!(ec_layout_rpms("unknown", &buf), None);
    }

    #[test]
    fn read_only_backend_is_not_control_capable() {
        let dir =
            std::env::temp_dir().join(format!("legion-fan-backend-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("fan1_input"), "1200\n").unwrap();
        assert!(has_fan_inputs(&dir));
        assert!(has_live_fan_input(&dir));
        assert!(!has_fan_targets(&dir));

        std::fs::write(dir.join("fan1_target"), "0\n").unwrap();
        assert!(has_fan_targets(&dir));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
