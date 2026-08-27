//! Platform profile via ACPI / platform-profile class.
//!
//! Kernel rejects writing `custom` to the legacy aggregate path
//! `/sys/firmware/acpi/platform_profile` (see `platform_profile_store` in
//! `drivers/acpi/platform_profile.c`). Custom must be set on the class
//! handler: `/sys/class/platform-profile/*/profile` — same approach used by
//! LenovoLegionLinux after kernel 6.x/7.x.

use std::fs;
use std::path::{Path, PathBuf};

const LEGACY_PROFILE: &str = "/sys/firmware/acpi/platform_profile";
const LEGACY_CHOICES: &str = "/sys/firmware/acpi/platform_profile_choices";
const CLASS_DIR: &str = "/sys/class/platform-profile";

/// Scan a sysfs class dir for the first entry whose (optionally filtered)
/// sub-path satisfies `want`, with match/no-match debug logging.
fn find_class_attr(
    what: &str,
    root: &str,
    name_filter: impl Fn(&str) -> bool,
    sub: &[&str],
    want_dir: bool,
) -> Option<PathBuf> {
    let dir = match fs::read_dir(root) {
        Ok(d) => d,
        Err(e) => {
            log::debug!("profile: {what}: {root} unreadable: {e}");
            return None;
        }
    };
    let mut scanned = 0usize;
    for entry in dir.flatten() {
        let name = entry.file_name();
        let n = name.to_string_lossy();
        if !name_filter(&n) {
            continue;
        }
        scanned += 1;
        let mut path = entry.path();
        for part in sub {
            path = path.join(part);
        }
        if (want_dir && path.is_dir()) || (!want_dir && path.is_file()) {
            log::debug!("profile: {what} matched '{n}' → {}", path.display());
            return Some(path);
        }
    }
    log::debug!("profile: {what}: no match under {scanned} scanned entr(ies) in {root}");
    None
}

/// First platform-profile handler exposing `file` ("profile"/"choices").
fn handler_attr_path(file: &str) -> Option<PathBuf> {
    find_class_attr(
        &format!("handler_attr_path('{file}')"),
        CLASS_DIR,
        |_| true,
        &[file],
        false,
    )
}

fn handler_profile_path() -> Option<PathBuf> {
    handler_attr_path("profile")
}

fn handler_choices_path() -> Option<PathBuf> {
    handler_attr_path("choices")
}

pub fn current() -> String {
    // Prefer the Gamezone handler — matches what Fn+Q / LED actually use.
    if let Some(p) = handler_profile_path() {
        if let Ok(s) = fs::read_to_string(&p) {
            let t = s.trim();
            if !t.is_empty() {
                return t.to_string();
            }
        }
    }
    fs::read_to_string(LEGACY_PROFILE)
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

pub fn choices() -> Vec<String> {
    let path = handler_choices_path().unwrap_or_else(|| PathBuf::from(LEGACY_CHOICES));
    match fs::read_to_string(&path) {
        Ok(s) => {
            let parsed: Vec<String> = s.split_whitespace().map(String::from).collect();
            log::debug!(
                "profile: choices ← {} token(s) from {}",
                parsed.len(),
                path.display()
            );
            parsed
        }
        Err(e) => {
            log::debug!("profile: choices read failed on {}: {e}", path.display());
            Vec::new()
        }
    }
}

/// Set the platform profile. Writes the class handler when available so
/// `custom` works (legacy aggregate sysfs hard-rejects it).
pub fn set(profile: &str) -> Result<(), String> {
    let name = profile.trim();
    if name.is_empty() {
        log::debug!("profile: set rejected — empty profile name");
        return Err("empty profile name".into());
    }
    let allowed = choices();
    if !allowed.is_empty() && !allowed.iter().any(|c| c == name) {
        log::debug!(
            "profile: set '{name}' rejected — allowed: [{}]",
            allowed.join(", ")
        );
        return Err(format!(
            "Profile '{name}' not in choices: {}",
            allowed.join(", ")
        ));
    }

    // Always prefer the class device — required for custom.
    if let Some(path) = handler_profile_path() {
        return write_profile(&path, name);
    }

    // Legacy path cannot set custom (kernel returns -EINVAL).
    if name == "custom" {
        log::debug!(
            "profile: set 'custom' rejected — no class handler and the legacy \
             aggregate path hard-rejects custom"
        );
        return Err(
            "No platform-profile class handler; kernel rejects writing 'custom' \
             to /sys/firmware/acpi/platform_profile"
                .into(),
        );
    }
    write_profile(Path::new(LEGACY_PROFILE), name)
}

fn write_profile(path: &Path, name: &str) -> Result<(), String> {
    log::info!("profile → {name} via {}", path.display());
    fs::write(path, format!("{name}\n")).map_err(|e| {
        let ctx = format!(
            "profile::set '{name}' failed on {}: {e} (kind={:?}, raw={})",
            path.display(),
            e.kind(),
            e.raw_os_error().unwrap_or(-1)
        );
        log::warn!("{ctx}");
        ctx
    })?;
    log::debug!("profile write ok");
    Ok(())
}

// ─── Custom-mode PPT (lenovo-wmi-other firmware-attributes) ───────────────

const FW_ATTR_ROOT: &str = "/sys/class/firmware-attributes";

/// Unit a firmware attribute is expressed in — drives all user-facing labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitUnit {
    Watts,
    Celsius,
}

impl LimitUnit {
    pub fn symbol(self) -> &'static str {
        match self {
            LimitUnit::Watts => "W",
            LimitUnit::Celsius => "°C",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PptLimit {
    pub id: &'static str,
    pub label: &'static str,
    pub unit: LimitUnit,
    pub current: u32,
    pub default: u32,
    pub min: u32,
    pub max: u32,
}

impl PptLimit {
    /// "`85–100 °C`"-style range label used by CLI + GUI alike.
    pub fn range_label(&self) -> String {
        format!("{}–{} {}", self.min, self.max, self.unit.symbol())
    }
    pub fn value_label(&self, value: u32) -> String {
        format!("{value} {}", self.unit.symbol())
    }
}

const PPT_IDS: &[(&str, &str, LimitUnit)] = &[
    ("ppt_pl1_spl", "Everyday power", LimitUnit::Watts),
    ("ppt_pl2_sppt", "Short boost", LimitUnit::Watts),
    ("ppt_pl3_fppt", "Peak burst", LimitUnit::Watts),
    ("ppt_cpu_cl", "CPU share", LimitUnit::Watts),
    // Firmware thermal cutoffs — INDEPENDENT of, and stacking with, the
    // software scaling_max_freq governor (Settings → Thermal).
    ("cpu_temp", "CPU thermal limit", LimitUnit::Celsius),
    ("gpu_temp", "GPU thermal limit", LimitUnit::Celsius),
];

/// NVIDIA GPU power knobs (Other Mode WMI). Same Custom-mode gate as CPU PPT.
/// Only attributes with a real firmware min/max range are exposed — some BIOS
/// builds list cTGP/PPAB but reject writes (EINVAL).
const GPU_PPT_IDS: &[(&str, &str, LimitUnit)] = &[
    ("gpu_nv_ac_offset", "GPU AC power target", LimitUnit::Watts),
    ("gpu_nv_ctgp", "GPU cTGP", LimitUnit::Watts),
    ("gpu_nv_ppab", "GPU PPAB", LimitUnit::Watts),
    ("gpu_nv_cpu_boost", "GPU↔CPU boost", LimitUnit::Watts),
];

fn fw_attr_dir(attr: &str) -> Option<PathBuf> {
    find_class_attr(
        &format!("fw_attr_dir('{attr}')"),
        FW_ATTR_ROOT,
        |n| n.starts_with("lenovo-wmi-other"),
        &["attributes", attr],
        true,
    )
}

fn read_u32_file(path: &Path) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn collect_limits(ids: &[(&'static str, &'static str, LimitUnit)]) -> Vec<PptLimit> {
    let mut out = Vec::new();
    for (id, label, unit) in ids {
        let Some(dir) = fw_attr_dir(id) else {
            log::debug!("profile: collect_limits '{id}' ({label}): attribute dir absent — skipped");
            continue;
        };
        let Some(current) = read_u32_file(&dir.join("current_value")) else {
            log::debug!(
                "profile: collect_limits '{id}' ({label}): current_value unreadable in {} — skipped",
                dir.display()
            );
            continue;
        };
        let default = read_u32_file(&dir.join("default_value")).unwrap_or(current);
        let min = read_u32_file(&dir.join("min_value")).unwrap_or(0);
        let max = read_u32_file(&dir.join("max_value")).unwrap_or(current);
        // Skip knobs the firmware exposes but does not actually range/tune.
        if max <= min {
            log::debug!(
                "profile: collect_limits '{id}' ({label}): degenerate range (min={min}, max={max}) — skipped"
            );
            continue;
        }
        log::debug!(
            "profile: collect_limits '{id}' ({label}): current={current} default={default} min={min} max={max}"
        );
        out.push(PptLimit {
            id,
            label,
            unit: *unit,
            current,
            default,
            min,
            max,
        });
    }
    out
}

/// CPU PPT knobs used in Custom mode (Other Mode WMI). Values are only applied by
/// firmware while platform profile is `custom`.
pub fn ppt_limits() -> Vec<PptLimit> {
    collect_limits(PPT_IDS)
}

/// NVIDIA GPU TDP knobs for Custom mode (writable firmware-attributes only).
pub fn gpu_ppt_limits() -> Vec<PptLimit> {
    collect_limits(GPU_PPT_IDS)
}

/// All Custom-mode firmware power attributes (CPU + GPU).
pub fn all_ppt_limits() -> Vec<PptLimit> {
    let mut out = ppt_limits();
    out.extend(gpu_ppt_limits());
    out
}

/// Single source of truth for which firmware attributes this app may write.
/// The daemon's SetFwAttr gate MUST use this (prefix checks caused a drift
/// where cpu_temp/gpu_temp passed the lib but were rejected by the daemon).
pub fn is_known_fw_attr(attr: &str) -> bool {
    let known = PPT_IDS.iter().any(|(id, _, _)| *id == attr)
        || GPU_PPT_IDS.iter().any(|(id, _, _)| *id == attr);
    log::trace!("profile: is_known_fw_attr('{attr}') → {known}");
    known
}

/// Unit for a firmware attribute; defaults to Watts for unknown ids
/// (unknown ids are rejected by set_ppt anyway).
pub fn limit_unit(attr: &str) -> LimitUnit {
    let found = PPT_IDS
        .iter()
        .find(|(id, _, _)| *id == attr)
        .or_else(|| GPU_PPT_IDS.iter().find(|(id, _, _)| *id == attr))
        .map(|(_, _, u)| *u);
    match found {
        Some(unit) => {
            log::trace!("profile: limit_unit('{attr}') → {unit:?}");
            unit
        }
        None => {
            log::trace!("profile: limit_unit('{attr}') unknown → Watts (default)");
            LimitUnit::Watts
        }
    }
}

pub fn set_ppt(attr: &str, value: u32) -> Result<(), String> {
    if !is_known_fw_attr(attr) {
        return Err(format!("Unknown PPT attribute '{attr}'"));
    }
    let dir = fw_attr_dir(attr).ok_or_else(|| format!("firmware-attribute '{attr}' not found"))?;
    let path = dir.join("current_value");
    log::info!("fw-attr {attr} → {value} ({})", path.display());

    // Lenovo's firmware temporarily returns EBUSY while changing into Custom
    // mode. Retry here so profile restore and slider changes do not fail just
    // because the WMI handler has not settled yet.
    const ATTEMPTS: usize = 8;
    for attempt in 0..ATTEMPTS {
        match fs::write(&path, format!("{value}\n")) {
            Ok(()) => {
                log::debug!(
                    "profile: fw-attr {attr}={value} written ok on attempt {}/{}",
                    attempt + 1,
                    ATTEMPTS
                );
                return Ok(());
            }
            Err(e) if e.raw_os_error() == Some(16) && attempt + 1 < ATTEMPTS => {
                log::debug!("fw-attr {attr} busy; retry {}/{}", attempt + 2, ATTEMPTS);
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            Err(e) => {
                let ctx = format!(
                    "profile::set_ppt {attr}={value} failed on {}: {e} (kind={:?}, raw={})",
                    path.display(),
                    e.kind(),
                    e.raw_os_error().unwrap_or(-1)
                );
                log::warn!("{ctx}");
                return Err(ctx);
            }
        }
    }
    // All paths inside the loop either return Ok or Err; this is unreachable
    // but the compiler can't prove it. Use Err as a safety net instead of panic.
    #[allow(unreachable_code)]
    {
        log::warn!("profile: fw-attr {attr}={value}: still busy after {ATTEMPTS} attempts");
        Err("retry loop exhausted without result".into())
    }
}

pub fn ppt_available() -> bool {
    let available = !all_ppt_limits().is_empty();
    log::debug!("profile: ppt_available → {available}");
    available
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_fw_attr_matches_declared_ids() {
        assert!(is_known_fw_attr("ppt_pl1_spl"));
        assert!(is_known_fw_attr("gpu_nv_ac_offset"));
        assert!(is_known_fw_attr("cpu_temp"), "thermal limits are writable");
        assert!(is_known_fw_attr("gpu_temp"));
        assert!(!is_known_fw_attr("unknown_attr"));
        assert!(!is_known_fw_attr(""));
    }

    /// The daemon gate and the discovery lists must never drift: every
    /// attribute we can discover must also pass the write gate.
    #[test]
    fn every_discoverable_limit_passes_the_daemon_gate() {
        for lim in all_ppt_limits() {
            assert!(
                is_known_fw_attr(lim.id),
                "{} discoverable but not writable via SetFwAttr",
                lim.id
            );
        }
        // Static lists too (covers attrs absent on this machine).
        for (id, _, _) in PPT_IDS.iter().chain(GPU_PPT_IDS.iter()) {
            assert!(is_known_fw_attr(id));
        }
    }

    #[test]
    fn set_ppt_rejects_unknown_attr_without_touching_sysfs() {
        let err = set_ppt("bogus_attr", 100).unwrap_err();
        assert!(err.contains("Unknown PPT attribute"), "err={err:?}");
    }

    #[test]
    fn limit_unit_maps_temps_to_celsius() {
        assert_eq!(limit_unit("cpu_temp"), LimitUnit::Celsius);
        assert_eq!(limit_unit("gpu_nv_ac_offset"), LimitUnit::Watts);
        assert_eq!(limit_unit("bogus"), LimitUnit::Watts);
    }
}
