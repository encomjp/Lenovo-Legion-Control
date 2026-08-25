//! Fan control via lenovo_wmi_other (preferred) or legion_hwmon.

use std::path::PathBuf;

use crate::device::{self, FanCapability};
use crate::sensors::hwmon_by_name;

fn fan_hwmon() -> Option<(String, PathBuf)> {
    if let Some(hw) = hwmon_by_name("lenovo_wmi_other") {
        return Some(("lenovo_wmi_other".into(), hw));
    }
    if let Some(hw) = hwmon_by_name("legion_hwmon") {
        return Some(("legion_hwmon".into(), hw));
    }
    None
}

fn fan_path(fan: u8, suffix: &str) -> Option<PathBuf> {
    let (_, hw) = fan_hwmon()?;
    Some(hw.join(format!("fan{fan}_{suffix}")))
}

pub fn read_rpm(fan: u8) -> Option<u32> {
    let path = fan_path(fan, "input")?;
    std::fs::read_to_string(&path).ok()?.trim().parse().ok()
}

pub fn read_target(fan: u8) -> Option<u32> {
    let path = fan_path(fan, "target")?;
    std::fs::read_to_string(&path).ok()?.trim().parse().ok()
}

pub fn read_min(fan: u8) -> Option<u32> {
    let path = fan_path(fan, "min")?;
    std::fs::read_to_string(&path).ok()?.trim().parse().ok()
}

pub fn read_max(fan: u8) -> Option<u32> {
    let path = fan_path(fan, "max")?;
    std::fs::read_to_string(&path).ok()?.trim().parse().ok()
}

/// Discovered fan channels with live min/max (falls back to model profile).
pub fn channels() -> Vec<FanCapability> {
    device::detect().capabilities.fans
}

/// Fan ids present on this machine (e.g. `[1, 2, 4]`).
pub fn ids() -> Vec<u8> {
    channels().into_iter().map(|f| f.id).collect()
}

/// Pure helper — the formatting contract for `target == 0` means "auto" on
/// this WMI driver. Used by `rpm_label`; exported so tests pin the contract.
pub fn format_rpm_label(target: u32, rpm: u32) -> String {
    if target == 0 {
        if rpm == 0 {
            "Auto".into()
        } else {
            format!("Auto · {rpm} rpm")
        }
    } else if rpm == 0 {
        format!("~{target} rpm")
    } else {
        format!("{rpm} rpm")
    }
}

/// UI-friendly RPM label. Auto mode often reports 0 on this WMI driver.
pub fn rpm_label(fan: u8) -> String {
    format_rpm_label(read_target(fan).unwrap_or(0), read_rpm(fan).unwrap_or(0))
}

/// Set fan target RPM. 0 = auto mode.
pub fn set_target(fan: u8, rpm: u32) -> std::io::Result<()> {
    let path = fan_path(fan, "target")
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "fan device not found"))?;
    if rpm == 0 {
        log::info!("fan {fan} → auto ({})", path.display());
    } else {
        log::info!("fan {fan} → {rpm} RPM ({})", path.display());
    }
    match std::fs::write(&path, format!("{rpm}")) {
        Ok(()) => Ok(()),
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

/// Set all discovered fans to auto mode.
pub fn set_auto() -> std::io::Result<()> {
    let mut last_err = None;
    for id in ids() {
        if let Err(e) = set_target(id, 0) {
            last_err = Some(e);
        }
    }
    match last_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_auto_covers_all_combos() {
        assert_eq!(format_rpm_label(0, 0), "Auto");
        assert_eq!(format_rpm_label(0, 1800), "Auto · 1800 rpm");
        assert_eq!(format_rpm_label(1500, 0), "~1500 rpm");
        assert_eq!(format_rpm_label(1500, 1400), "1400 rpm");
    }
}
