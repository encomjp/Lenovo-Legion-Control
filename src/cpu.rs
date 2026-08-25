//! CPU topology knobs — SMT (hyperthreading) and frequency boost.
//!
//! Paths:
//! - `/sys/devices/system/cpu/smt/control` — `on` / `off` / `forceoff`
//! - `/sys/devices/system/cpu/cpufreq/boost` — `1` / `0` (AMD/Intel boost)

use std::fs;
use std::path::Path;

const SMT_CONTROL: &str = "/sys/devices/system/cpu/smt/control";
const SMT_ACTIVE: &str = "/sys/devices/system/cpu/smt/active";
const CPU_BOOST: &str = "/sys/devices/system/cpu/cpufreq/boost";

fn read_trim(path: &str) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Whether SMT (AMD SMT / Intel HT) is currently active.
/// Parse a sysfs knob that exposes "1"/"0" as a tri-state bool.
fn sysfs_bool(path: &str) -> Option<bool> {
    match read_trim(path)?.as_str() {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    }
}

pub fn smt_active() -> Option<bool> {
    sysfs_bool(SMT_ACTIVE)
}

/// Raw SMT control string (`on`, `off`, `forceoff`, `notsupported`, …).
pub fn smt_control() -> Option<String> {
    read_trim(SMT_CONTROL)
}

pub fn smt_available() -> bool {
    Path::new(SMT_CONTROL).exists()
        && !matches!(
            smt_control().as_deref(),
            Some("notsupported" | "notimplemented")
        )
}

/// Enable (`on`) or disable (`off`) SMT. Requires root (daemon).
pub fn set_smt(on: bool) -> Result<(), String> {
    if !smt_available() {
        return Err("SMT control is not available on this kernel/CPU".into());
    }
    let value = if on { "on" } else { "off" };
    log::info!("smt → {value}");
    fs::write(SMT_CONTROL, value).map_err(|e| {
        let msg = format!("Cannot set SMT to {value}: {e} (needs root legion-control service)");
        log::warn!("{msg}");
        msg
    })?;
    Ok(())
}

pub fn boost_available() -> bool {
    Path::new(CPU_BOOST).exists()
}

pub fn boost_enabled() -> Option<bool> {
    sysfs_bool(CPU_BOOST)
}

/// Toggle CPU frequency boost (turbo). Requires root (daemon).
pub fn set_boost(on: bool) -> Result<(), String> {
    if !boost_available() {
        return Err("CPU boost sysfs is not available".into());
    }
    let value = if on { "1" } else { "0" };
    log::info!("cpu boost → {}", if on { "on" } else { "off" });
    fs::write(CPU_BOOST, value).map_err(|e| {
        let msg = format!("Cannot set boost to {value}: {e} (needs root legion-control service)");
        log::warn!("{msg}");
        msg
    })?;
    Ok(())
}

/// Rough logical CPU count (online).
pub fn logical_cpus() -> usize {
    fs::read_dir("/sys/devices/system/cpu")
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| {
                    let s = e.file_name().to_string_lossy().into_owned();
                    if !(s.starts_with("cpu") && s[3..].chars().all(|c| c.is_ascii_digit())) {
                        return false;
                    }
                    let online = e.path().join("online");
                    if !online.exists() {
                        return true; // cpu0 often has no `online` file
                    }
                    fs::read_to_string(online)
                        .map(|v| v.trim() == "1")
                        .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0)
}

pub fn smt_summary() -> String {
    let n = logical_cpus();
    match (smt_active(), smt_control()) {
        (Some(true), _) => format!("On · {n} logical CPUs"),
        (Some(false), _) => format!("Off · {n} logical CPUs"),
        (_, Some(c)) => format!("{c} · {n} logical CPUs"),
        _ => format!("{n} logical CPUs"),
    }
}
