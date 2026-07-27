//! Battery info + charge limit modes for Lenovo Legion.
//!
//! Hardware only supports discrete limits (not arbitrary %):
//! - 100% → charge_types = Standard, conservation_mode = 0
//! -  80% → charge_types = Long_Life (firmware preservation)
//! -  60% → ideapad conservation_mode = 1

use std::path::Path;
use std::sync::OnceLock;

const BAT0: &str = "/sys/class/power_supply/BAT0";

static CONSERVATION_PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();

/// Find conservation_mode sysfs path by scanning /sys (the PCI path varies
/// by model / BIOS, so hardcoding it is fragile). Cached via OnceLock.
fn conservation_path() -> Option<&'static std::path::Path> {
    CONSERVATION_PATH
        .get_or_init(|| {
            // Fast path: check the known Legion Pro 7 path first.
            let known = std::path::Path::new(
                "/sys/devices/pci0000:00/0000:00:14.3/PNP0C09:00/VPC2004:00/conservation_mode",
            );
            if known.exists() {
                return Some(known.to_path_buf());
            }
            // Fallback: scan platform drivers for ideapad_acpi.
            if let Ok(entries) = std::fs::read_dir("/sys/bus/platform/drivers/ideapad_acpi") {
                for entry in entries.flatten() {
                    let candidate = entry.path().join("conservation_mode");
                    if candidate.exists() {
                        return Some(candidate);
                    }
                }
            }
            None
        })
        .as_ref()
        .map(std::path::PathBuf::as_path)
}

fn read(path: &str) -> Option<String> {
    std::fs::read_to_string(Path::new(path))
        .ok()
        .map(|s| s.trim().to_string())
}

pub fn capacity() -> Option<u32> {
    read(&format!("{BAT0}/capacity"))?.parse().ok()
}

pub fn status() -> Option<String> {
    read(&format!("{BAT0}/status"))
}

pub fn voltage() -> Option<f64> {
    let v: i64 = read(&format!("{BAT0}/voltage_now"))?.parse().ok()?;
    Some(v as f64 / 1_000_000.0)
}

pub fn cycles() -> Option<u32> {
    read(&format!("{BAT0}/cycle_count"))?.parse().ok()
}

pub fn power_w() -> Option<f64> {
    let v: i64 = read(&format!("{BAT0}/power_now"))?.parse().ok()?;
    Some(v as f64 / 1_000_000.0)
}

pub fn energy_now_wh() -> Option<f64> {
    let v: i64 = read(&format!("{BAT0}/energy_now"))?.parse().ok()?;
    Some(v as f64 / 1_000_000.0)
}

pub fn energy_full_wh() -> Option<f64> {
    let v: i64 = read(&format!("{BAT0}/energy_full"))?.parse().ok()?;
    Some(v as f64 / 1_000_000.0)
}

pub fn energy_design_wh() -> Option<f64> {
    let v: i64 = read(&format!("{BAT0}/energy_full_design"))?.parse().ok()?;
    Some(v as f64 / 1_000_000.0)
}

pub fn health_pct() -> Option<f64> {
    let full = energy_full_wh()?;
    let design = energy_design_wh()?;
    if design <= 0.0 {
        return None;
    }
    Some((full / design) * 100.0)
}

pub fn manufacturer() -> Option<String> {
    read(&format!("{BAT0}/manufacturer"))
}

pub fn model_name() -> Option<String> {
    read(&format!("{BAT0}/model_name"))
}

pub fn technology() -> Option<String> {
    read(&format!("{BAT0}/technology"))
}

pub fn charge_types() -> Option<String> {
    read(&format!("{BAT0}/charge_types"))
}

pub fn conservation_mode() -> Option<bool> {
    let path = conservation_path()?;
    // Prefer ideapad conservation_mode when present
    if let Some(v) = read(&path.to_string_lossy()) {
        return Some(v.trim() == "1");
    }
    let types = charge_types()?;
    Some(types.contains("[Long_Life]"))
}

fn set_conservation_file(on: bool) -> std::io::Result<()> {
    let path = conservation_path().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "conservation_mode not found")
    })?;
    std::fs::write(path, if on { "1" } else { "0" })
}

fn set_charge_type(val: &str) -> std::io::Result<()> {
    std::fs::write(format!("{BAT0}/charge_types"), val)
}

/// Legacy boolean API — maps to ~60% conservation when on, Standard when off.
pub fn set_conservation(on: bool) -> std::io::Result<()> {
    if on {
        set_charge_limit_pct(60)
    } else {
        set_charge_limit_pct(100)
    }
}

/// Current effective charge limit percentage: 60, 80, or 100.
pub fn charge_limit_pct() -> u32 {
    if let Some(path) = conservation_path() {
        if read(&path.to_string_lossy()).as_deref() == Some("1") {
            return 60;
        }
    }
    if let Some(types) = charge_types() {
        if types.contains("[Long_Life]") {
            return 80;
        }
    }
    100
}

/// Set charge limit. Only 60 / 80 / 100 are valid on Legion firmware.
pub fn set_charge_limit_pct(pct: u32) -> std::io::Result<()> {
    let pct = match pct {
        0..=69 => 60,
        70..=89 => 80,
        _ => 100,
    };
    log::info!("charge limit → {pct}%");
    let result = match pct {
        60 => {
            // Classic conservation (~55–60%): set charge_type first, then
            // conservation. If conservation_mode is unavailable, the charge_type
            // alone still limits charging on some firmware.
            if let Err(e) = set_charge_type("Standard") {
                log::debug!("set_charge_type(Standard) for 60% mode: {e}");
            }
            set_conservation_file(true)
        }
        80 => {
            if let Err(e) = set_conservation_file(false) {
                log::debug!("clearing conservation for 80% mode: {e}");
            }
            set_charge_type("Long_Life")
        }
        _ => {
            if let Err(e) = set_conservation_file(false) {
                log::debug!("clearing conservation for 100% mode: {e}");
            }
            set_charge_type("Standard")
        }
    };
    if let Err(ref e) = result {
        log::warn!("charge limit {pct}% failed: {e}");
    }
    result
}

pub fn charge_limit_label(pct: u32) -> &'static str {
    match pct {
        60 => "Conservation (~60%)",
        80 => "Preservation (~80%)",
        _ => "Full charge (100%)",
    }
}
