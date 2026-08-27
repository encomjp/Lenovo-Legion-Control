//! Intel-only extras — PState + Uncore. Gated on sysfs presence.
//!
//! Borrowed from bolekjar/lenovo-legion-linux-toolkit
//!   lenovo-legion-cli/src/sysfs_drivers/cpu_control.rs
//! Adapted: Option fields for missing files, path-bearing errors, no new deps.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const SYSFS_PSTATE: &str = "/sys/devices/system/cpu/intel_pstate/";
const SYSFS_UNCORE: &str = "/sys/devices/system/cpu/intel_uncore_frequency/";

fn read_val<T: std::str::FromStr>(path: &str) -> std::io::Result<T>
where
    T::Err: std::fmt::Display,
{
    let s = fs::read_to_string(path)
        .map_err(|e| std::io::Error::new(e.kind(), format!("{path}: {e}")))?;
    s.trim()
        .parse::<T>()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{path}: {e}")))
}

fn read_opt<T: std::str::FromStr>(path: &str) -> Option<T>
where
    T::Err: std::fmt::Display,
{
    match read_val(path) {
        Ok(v) => Some(v),
        Err(e) => {
            log::trace!("intel: read_opt {path}: {e}");
            None
        }
    }
}

// ── Intel PState ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct IntelPState {
    pub hwp_dynamic_boost: Option<bool>,
    pub max_perf_pct: Option<u32>,
    pub min_perf_pct: Option<u32>,
    pub no_turbo: Option<bool>,
    pub status: Option<String>,
}

pub fn pstate_available() -> bool {
    let available = Path::new(SYSFS_PSTATE).exists();
    log::debug!("intel: pstate_available → {available} ({SYSFS_PSTATE})");
    available
}

pub fn read_pstate() -> std::io::Result<IntelPState> {
    if !pstate_available() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("{SYSFS_PSTATE} not present (Intel PState not supported)"),
        ));
    }
    Ok(IntelPState {
        hwp_dynamic_boost: read_opt::<u8>(&format!("{SYSFS_PSTATE}hwp_dynamic_boost"))
            .map(|v| v != 0),
        max_perf_pct: read_opt(&format!("{SYSFS_PSTATE}max_perf_pct")),
        min_perf_pct: read_opt(&format!("{SYSFS_PSTATE}min_perf_pct")),
        no_turbo: read_opt::<u8>(&format!("{SYSFS_PSTATE}no_turbo")).map(|v| v != 0),
        status: fs::read_to_string(format!("{SYSFS_PSTATE}status"))
            .ok()
            .map(|s| s.trim().to_string()),
    })
}

/// Write a sysfs file, wrapping errors with the path for context.
fn write_sysfs(path: &str, value: impl std::fmt::Display) -> std::io::Result<()> {
    fs::write(path, value.to_string())
        .map_err(|e| std::io::Error::new(e.kind(), format!("{path}: {e}")))
}

pub fn set_pstate_hwp_dynamic_boost(on: bool) -> std::io::Result<()> {
    write_sysfs(
        &format!("{SYSFS_PSTATE}hwp_dynamic_boost"),
        if on { "1" } else { "0" },
    )
}
pub fn set_pstate_no_turbo(no_turbo: bool) -> std::io::Result<()> {
    write_sysfs(
        &format!("{SYSFS_PSTATE}no_turbo"),
        if no_turbo { "1" } else { "0" },
    )
}
pub fn set_pstate_max_pct(pct: u32) -> std::io::Result<()> {
    if pct > 100 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "max_perf_pct 0-100",
        ));
    }
    write_sysfs(&format!("{SYSFS_PSTATE}max_perf_pct"), pct)
}
pub fn set_pstate_min_pct(pct: u32) -> std::io::Result<()> {
    if pct > 100 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "min_perf_pct 0-100",
        ));
    }
    write_sysfs(&format!("{SYSFS_PSTATE}min_perf_pct"), pct)
}

// ── Intel Uncore ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct IntelUncorePackage {
    pub package: String,
    pub current_freq_khz: Option<u32>,
    pub initial_max_freq_khz: Option<u32>,
    pub initial_min_freq_khz: Option<u32>,
    pub max_freq_khz: Option<u32>,
    pub min_freq_khz: Option<u32>,
}

pub fn uncore_available() -> bool {
    let available = Path::new(SYSFS_UNCORE).exists();
    log::debug!("intel: uncore_available → {available} ({SYSFS_UNCORE})");
    available
}

pub fn uncore_packages() -> std::io::Result<Vec<IntelUncorePackage>> {
    if !uncore_available() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("{SYSFS_UNCORE} not present (Intel Uncore not supported)"),
        ));
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(SYSFS_UNCORE)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("package") {
            continue;
        }
        let base = format!("{SYSFS_UNCORE}{name}/");
        log::debug!("intel: uncore_packages: enumerating '{name}'");
        out.push(IntelUncorePackage {
            package: name,
            current_freq_khz: read_opt(&format!("{base}current_freq_khz")),
            initial_max_freq_khz: read_opt(&format!("{base}initial_max_freq_khz")),
            initial_min_freq_khz: read_opt(&format!("{base}initial_min_freq_khz")),
            max_freq_khz: read_opt(&format!("{base}max_freq_khz")),
            min_freq_khz: read_opt(&format!("{base}min_freq_khz")),
        });
    }
    log::debug!("intel: uncore_packages → {} package(s)", out.len());
    Ok(out)
}

fn set_uncore_freq(package: &str, khz: u32, suffix: &str) -> std::io::Result<()> {
    let path = format!("{SYSFS_UNCORE}{package}/{suffix}");
    if !Path::new(&path).exists() {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, path));
    }
    write_sysfs(&path, khz)
}
pub fn set_uncore_max(package: &str, khz: u32) -> std::io::Result<()> {
    set_uncore_freq(package, khz, "max_freq_khz")
}
pub fn set_uncore_min(package: &str, khz: u32) -> std::io::Result<()> {
    set_uncore_freq(package, khz, "min_freq_khz")
}

// ── Hybrid topology helpers (Intel 12th+ P/E split) ───────────────────────

pub fn hybrid_topology() -> Option<(BTreeSet<u32>, BTreeSet<u32>)> {
    let parse = |kind: &str, p: &str| -> Option<BTreeSet<u32>> {
        let s = match fs::read_to_string(p) {
            Ok(s) => s,
            Err(e) => {
                log::debug!("intel: hybrid_topology '{kind}' cpu list {p} unreadable: {e}");
                return None;
            }
        };
        let set: BTreeSet<u32> = s
            .trim()
            .split(',')
            .filter_map(|tok| {
                if let Some((lo, hi)) = tok.split_once('-') {
                    let lo: u32 = lo.parse().ok()?;
                    let hi: u32 = hi.parse().ok()?;
                    if lo <= hi && (hi - lo) < 1024 {
                        Some((lo..=hi).collect::<Vec<_>>())
                    } else {
                        None
                    }
                } else {
                    Some(vec![tok.parse().ok()?])
                }
            })
            .flatten()
            .collect();
        log::debug!(
            "intel: hybrid_topology '{kind}' cpus ← {} cpu(s): {set:?}",
            set.len()
        );
        Some(set)
    };
    let atom = parse("atom", "/sys/devices/cpu_atom/cpus")?;
    let core = parse("core", "/sys/devices/cpu_core/cpus")?;
    Some((atom, core))
}
