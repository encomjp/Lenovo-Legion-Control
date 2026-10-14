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

/// Same answer as [`current`] without per-call debug logging, for loops
/// that poll every few hundred milliseconds (popup, auto switching, history).
pub fn current_quiet() -> String {
    let class = fs::read_dir(CLASS_DIR)
        .into_iter()
        .flatten()
        .flatten()
        .find_map(|e| {
            let s = fs::read_to_string(e.path().join("profile")).ok()?;
            let t = s.trim();
            (!t.is_empty()).then(|| t.to_string())
        });
    class.unwrap_or_else(|| {
        fs::read_to_string(LEGACY_PROFILE)
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "unknown".to_string())
    })
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

/// Known-dangerous platform profile writes, keyed by BIOS version prefix.
/// `(bios match, profile, reason)`. A match is a prefix of the DMI BIOS
/// version, so `"Q7CN"` covers every Q7CN build and `"LPCN65WW"` one build.
const DANGEROUS_PROFILES: &[(&str, &str, &str)] = &[
    (
        "Q7CN",
        "custom",
        "writing 'custom' hard-shuts-down the Legion Pro 7 16IAX10H (BIOS Q7CN)",
    ),
    (
        "LPCN65WW",
        "max-power",
        "'max-power' cuts power instantly on the Legion Pro 5 16ARX8 (BIOS LPCN65WW)",
    ),
];

/// DMI `product_version` values whose firmware has a broken max-power
/// entry. Mirrors `fwbug_list` in the upstream kernel
/// (drivers/platform/x86/lenovo/wmi-gamezone.c, matched on sys_vendor
/// "LENOVO" plus product_version), which drops max-power from
/// platform_profile_choices on these machines. Kept here so older kernels
/// and out-of-tree drivers that still list it get the same treatment.
const MAX_POWER_BROKEN_PRODUCTS: &[&str] = &[
    "Legion Go 8APU1",
    "Legion Go S 8APU1",
    "Legion Go S 8ARP1",
    "Legion Go 8ASP2",
    "Legion Go 8AHP2",
];

const MAX_POWER_BROKEN_REASON: &str =
    "the firmware on this model has a broken Max power entry (same list as the kernel)";

/// Pure: true when `(sys_vendor, product_version)` is on the kernel's
/// broken-max-power list. Exact match like the kernel's DMI_MATCH on a full
/// string, but tolerant of surrounding whitespace and vendor case.
pub fn max_power_broken_for(sys_vendor: &str, product_version: &str) -> bool {
    sys_vendor.trim().eq_ignore_ascii_case("LENOVO")
        && MAX_POWER_BROKEN_PRODUCTS
            .iter()
            .any(|p| *p == product_version.trim())
}

/// This machine is on the kernel's broken-max-power list.
pub fn max_power_broken() -> bool {
    let read = |f: &str| {
        fs::read_to_string(format!("/sys/class/dmi/id/{f}"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    };
    max_power_broken_for(&read("sys_vendor"), &read("product_version"))
}

/// Why writing `profile` is blocked on a machine with this BIOS version.
/// Pure so the deny-list is unit-testable without sysfs.
pub fn blocked_reason_for(bios_version: &str, profile: &str) -> Option<&'static str> {
    let bios = bios_version.trim().to_ascii_uppercase();
    DANGEROUS_PROFILES
        .iter()
        .find(|(b, p, _)| bios.starts_with(b) && *p == profile.trim())
        .map(|(_, _, why)| *why)
}

/// Pure: every reason a mode is withheld (BIOS deny-list, then the DMI
/// broken-max-power list).
pub fn withheld_reason(
    bios_version: &str,
    max_power_broken: bool,
    profile: &str,
) -> Option<&'static str> {
    blocked_reason_for(bios_version, profile)
        .or_else(|| (max_power_broken && profile.trim() == "max-power").then_some(MAX_POWER_BROKEN_REASON))
}

/// Pure: the firmware's choices minus every withheld mode, order kept.
pub fn filter_choices(choices: &[String], bios_version: &str, max_power_broken: bool) -> Vec<String> {
    choices
        .iter()
        .filter(|c| withheld_reason(bios_version, max_power_broken, c).is_none())
        .cloned()
        .collect()
}

fn dmi_bios_version() -> String {
    fs::read_to_string("/sys/class/dmi/id/bios_version")
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// Why `profile` must not be written on this machine, if it is withheld.
pub fn blocked_reason(profile: &str) -> Option<&'static str> {
    withheld_reason(&dmi_bios_version(), max_power_broken(), profile)
}

/// THE list of power modes the app offers: the firmware's
/// `platform_profile_choices` minus the modes withheld on this machine.
/// GUI (Home, Cooling, Profiles, automatic switching), tray, CLI and the
/// widget (through `legion-cli profile --with-choices`) all use this; `set`
/// enforces it. Never hard-code a mode list instead.
pub fn safe_choices() -> Vec<String> {
    filter_choices(&choices(), &dmi_bios_version(), max_power_broken())
}

/// `(profile, reason)` for every advertised choice that is blocked here.
pub fn blocked_choices() -> Vec<(String, &'static str)> {
    let bios = dmi_bios_version();
    let broken = max_power_broken();
    choices()
        .into_iter()
        .filter_map(|c| withheld_reason(&bios, broken, &c).map(|why| (c, why)))
        .collect()
}

/// Set the platform profile. Writes the class handler when available so
/// `custom` works (legacy aggregate sysfs hard-rejects it).
pub fn set(profile: &str) -> Result<(), String> {
    let name = profile.trim();
    if name.is_empty() {
        log::debug!("profile: set rejected: empty profile name");
        return Err("empty profile name".into());
    }
    if let Some(why) = blocked_reason(name) {
        log::warn!("profile: set '{name}' refused: {why}");
        return Err(format!(
            "Refusing to set '{name}': {why}. Choose another power mode."
        ));
    }
    let allowed = choices();
    if !allowed.is_empty() && !allowed.iter().any(|c| c == name) {
        log::debug!(
            "profile: set '{name}' rejected: allowed: [{}]",
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
            "profile: set 'custom' rejected: no class handler and the legacy \
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

/// When this process last wrote a power mode (see [`since_last_set`]).
static LAST_SET: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

/// Time since this process last changed the power mode, `None` if never.
/// The firmware may drop writes made right after a mode change, so power
/// limit writes wait until this is at least 500 ms.
pub fn since_last_set() -> Option<std::time::Duration> {
    LAST_SET
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .map(|t| t.elapsed())
}

fn write_profile(path: &Path, name: &str) -> Result<(), String> {
    log::info!("profile → {name} via {}", path.display());
    *LAST_SET.lock().unwrap_or_else(|p| p.into_inner()) = Some(std::time::Instant::now());
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
    ("gpu_nv_ac_offset", "GPU power on AC", LimitUnit::Watts),
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
            log::debug!("profile: collect_limits '{id}' ({label}): attribute dir absent: skipped");
            continue;
        };
        let Some(current) = read_u32_file(&dir.join("current_value")) else {
            log::debug!(
                "profile: collect_limits '{id}' ({label}): current_value unreadable in {}: skipped",
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
                "profile: collect_limits '{id}' ({label}): degenerate range (min={min}, max={max}): skipped"
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

/// Plain label for a firmware attribute ("Everyday power"); the id itself
/// for unknown ones.
pub fn limit_label(attr: &str) -> &str {
    PPT_IDS
        .iter()
        .chain(GPU_PPT_IDS.iter())
        .find(|(id, _, _)| *id == attr)
        .map_or(attr, |(_, label, _)| label)
}

/// Current `current_value` of an allow-listed firmware attribute.
pub fn read_ppt(attr: &str) -> Option<u32> {
    if !is_known_fw_attr(attr) {
        return None;
    }
    read_u32_file(&fw_attr_dir(attr)?.join("current_value"))
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
    fn deny_list_blocks_known_dangerous_profiles() {
        assert!(blocked_reason_for("Q7CN34WW", "custom").is_some());
        assert!(blocked_reason_for("q7cn34ww", " custom\n").is_some());
        assert!(blocked_reason_for("Q7CN34WW", "performance").is_none());
        assert!(blocked_reason_for("Q7CN34WW", "max-power").is_none());
        assert!(blocked_reason_for("LPCN65WW", "max-power").is_some());
        // Only the reported LPCN build is blocked, and only max-power.
        assert!(blocked_reason_for("LPCN65WW", "custom").is_none());
        assert!(blocked_reason_for("LPCN66WW", "max-power").is_none());
        // Unrelated machines and unreadable DMI are never blocked.
        assert!(blocked_reason_for("SMCN20WW", "custom").is_none());
        assert!(blocked_reason_for("", "custom").is_none());
    }

    #[test]
    fn max_power_broken_mirrors_kernel_dmi_list() {
        for p in [
            "Legion Go 8APU1",
            "Legion Go S 8APU1",
            "Legion Go S 8ARP1",
            "Legion Go 8ASP2",
            "Legion Go 8AHP2",
        ] {
            assert!(max_power_broken_for("LENOVO", p), "{p}");
            assert!(max_power_broken_for("lenovo\n", &format!("{p}\n")), "{p} trimmed");
        }
        // Vendor must match, product must match exactly.
        assert!(!max_power_broken_for("ASUS", "Legion Go 8APU1"));
        assert!(!max_power_broken_for("LENOVO", "Legion Pro 7 16AFR10H"));
        assert!(!max_power_broken_for("LENOVO", "Legion Go"));
        assert!(!max_power_broken_for("", ""));
    }

    #[test]
    fn filter_choices_drops_withheld_modes_only() {
        let all: Vec<String> = ["low-power", "balanced", "performance", "max-power", "custom"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        // This machine (83RU, SMCN): nothing withheld.
        assert_eq!(filter_choices(&all, "SMCN20WW", false), all);
        // Broken max-power entry: only max-power goes, order kept.
        assert_eq!(
            filter_choices(&all, "SMCN20WW", true),
            vec!["low-power", "balanced", "performance", "custom"]
        );
        // BIOS deny-list still applies on top.
        assert_eq!(
            filter_choices(&all, "Q7CN34WW", true),
            vec!["low-power", "balanced", "performance"]
        );
        assert!(withheld_reason("", true, "max-power").is_some());
        assert!(withheld_reason("", true, "performance").is_none());
        // Empty choices stay empty (caller falls back).
        assert!(filter_choices(&[], "", true).is_empty());
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

/// Automatic power-mode switching: AC / battery rule plus optional per-app
/// rules. Everything here except the sysfs / procfs readers is pure so the
/// daemon's decisions are unit-testable.
///
/// Battery findings (what the firmware does with platform_profile when the
/// charger is unplugged):
/// - Lenovo Legion / LOQ firmware cycles only Quiet and Balanced with Fn+Q
///   on battery. The EC caps package and GPU power on battery whatever the
///   mode, so Performance is accepted on most models but limited, and some
///   BIOSes reject it or drop back to Balanced on their own.
/// - Custom and Max power ("Extreme") are AC-only: their firmware power
///   limits need the adapter, and writes on battery are rejected (EINVAL /
///   EOPNOTSUPP) or silently reverted. They are never offered for battery.
/// - `platform_profile_choices` does not change with the power source, so it
///   cannot tell what works on battery. We therefore offer Quiet and
///   Balanced, plus Performance only when the firmware lists it, and when a
///   battery write fails the daemon logs it and falls back to Balanced.
pub mod auto {
    use crate::config::{AppRule, AutoModeConfig};
    use serde::{Deserialize, Serialize};

    /// Linux `comm` is truncated to 15 bytes (TASK_COMM_LEN - 1).
    const COMM_LEN: usize = 15;

    /// Live state reported to the GUI / CLI next to the stored config.
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
    pub struct AutoModeStatus {
        pub config: AutoModeConfig,
        /// `None` when no charger / battery information is readable.
        pub on_ac: Option<bool>,
        /// Process name of the app rule currently in force.
        pub active_app: Option<String>,
    }

    /// Modes that make sense on battery, in display order.
    pub fn battery_choices(choices: &[String]) -> Vec<String> {
        let has = |m: &str| choices.is_empty() || choices.iter().any(|c| c == m);
        let mut out: Vec<String> = Vec::new();
        for m in ["low-power", "balanced"] {
            if has(m) {
                out.push(m.into());
            }
        }
        // Performance only when the firmware advertises it at all.
        if choices.iter().any(|c| c == "performance") {
            out.push("performance".into());
        }
        out
    }

    /// True for modes the firmware never runs on battery.
    pub fn ac_only(mode: &str) -> bool {
        matches!(mode, "custom" | "max-power")
    }

    /// Validate a config before storing it. `choices` are the platform
    /// profile choices (empty = unknown, accept the common names).
    pub fn validate(cfg: &AutoModeConfig, choices: &[String]) -> Result<(), String> {
        let known = |m: &str| {
            if choices.is_empty() {
                matches!(
                    m,
                    "low-power" | "quiet" | "balanced" | "performance" | "max-power" | "custom"
                )
            } else {
                choices.iter().any(|c| c == m)
            }
        };
        if !known(&cfg.ac_mode) {
            return Err(format!("Unknown mode '{}' for plugged in", cfg.ac_mode));
        }
        if !battery_choices(choices).contains(&cfg.battery_mode) {
            return Err(format!(
                "Mode '{}' is not available on battery",
                cfg.battery_mode
            ));
        }
        if cfg.app_rules.len() > 32 {
            return Err("At most 32 app rules".into());
        }
        for r in &cfg.app_rules {
            let p = r.process.trim();
            if p.is_empty() || p.len() > 64 || p.contains('/') || p.chars().any(char::is_control) {
                return Err(format!("Invalid process name '{}'", r.process));
            }
            if !known(&r.mode) {
                return Err(format!("Unknown mode '{}' for {}", r.mode, r.process));
            }
        }
        Ok(())
    }

    /// Does a rule's process name match a `/proc/<pid>/comm` value?
    /// Case-insensitive; the rule is truncated like the kernel truncates comm.
    pub fn rule_matches(rule: &str, comm: &str) -> bool {
        let rule = rule.trim();
        if rule.is_empty() {
            return false;
        }
        let cut: String = rule.chars().take(COMM_LEN).collect();
        // Truncate by bytes as the kernel does, but stay on a char boundary.
        let cut = if cut.len() > COMM_LEN {
            let mut end = COMM_LEN;
            while !cut.is_char_boundary(end) {
                end -= 1;
            }
            cut[..end].to_string()
        } else {
            cut
        };
        comm.trim().eq_ignore_ascii_case(&cut)
    }

    /// First app rule that applies right now, if any.
    pub fn matching_rule<'a>(
        cfg: &'a AutoModeConfig,
        on_ac: bool,
        running: &[String],
    ) -> Option<&'a AppRule> {
        if !on_ac && !cfg.apps_on_battery {
            return None;
        }
        cfg.app_rules.iter().find(|r| {
            !(r.mode.is_empty() || (!on_ac && ac_only(&r.mode)))
                && running.iter().any(|c| rule_matches(&r.process, c))
        })
    }

    /// Mode the AC / battery rule asks for.
    pub fn power_rule_mode(cfg: &AutoModeConfig, on_ac: bool) -> String {
        if on_ac {
            cfg.ac_mode.clone()
        } else if ac_only(&cfg.battery_mode) {
            "balanced".into()
        } else {
            cfg.battery_mode.clone()
        }
    }

    /// Pure rule resolution: the mode that should be active for these inputs.
    /// `last_manual` is the mode to return to when no rule claims the moment
    /// (the mode active before an app rule started, or the current mode).
    pub fn resolve(
        cfg: &AutoModeConfig,
        on_ac: bool,
        running: &[String],
        last_manual: &str,
    ) -> String {
        if !cfg.enabled {
            return last_manual.to_string();
        }
        if let Some(rule) = matching_rule(cfg, on_ac, running) {
            return rule.mode.clone();
        }
        last_manual.to_string()
    }

    /// Transition-only switching state. `step` returns a mode to write only
    /// on a plug / unplug event, when an app rule starts or ends, or when
    /// switching was just turned on, so a manual change (Fn+Q, GUI) holds
    /// until the next such event.
    #[derive(Debug, Default, Clone)]
    pub struct Switcher {
        last_ac: Option<bool>,
        /// Process name of the rule in force.
        app: Option<String>,
        /// Mode to restore when the app rule ends.
        restore_to: Option<String>,
    }

    impl Switcher {
        pub fn active_app(&self) -> Option<&str> {
            self.app.as_deref()
        }

        pub fn step(
            &mut self,
            cfg: &AutoModeConfig,
            on_ac: bool,
            running: &[String],
            current: &str,
        ) -> Option<String> {
            if !cfg.enabled {
                *self = Switcher::default();
                return None;
            }
            let plug_event = self.last_ac != Some(on_ac);
            self.last_ac = Some(on_ac);
            let rule = matching_rule(cfg, on_ac, running);

            let target = match (rule, self.app.clone()) {
                (Some(r), None) => {
                    // App started: remember what to come back to.
                    self.restore_to = Some(if plug_event {
                        power_rule_mode(cfg, on_ac)
                    } else {
                        current.to_string()
                    });
                    self.app = Some(r.process.clone());
                    Some(r.mode.clone())
                }
                (Some(r), Some(prev)) => {
                    if plug_event {
                        // The power rule changed underneath the app.
                        self.restore_to = Some(power_rule_mode(cfg, on_ac));
                    }
                    if !prev.eq_ignore_ascii_case(&r.process) || plug_event {
                        self.app = Some(r.process.clone());
                        Some(r.mode.clone())
                    } else {
                        None
                    }
                }
                (None, Some(_)) => {
                    // App ended (or no longer applies, e.g. unplugged).
                    self.app = None;
                    let back = self.restore_to.take();
                    if plug_event {
                        Some(power_rule_mode(cfg, on_ac))
                    } else {
                        back.or_else(|| Some(power_rule_mode(cfg, on_ac)))
                    }
                }
                (None, None) => plug_event.then(|| power_rule_mode(cfg, on_ac)),
            };
            target.filter(|t| t != current)
        }
    }

    /// AC state from `(type, online, scope)` of each power_supply entry.
    /// Mains adapters decide; without one, a system-scope USB supply does.
    pub fn on_ac_from(supplies: &[(String, Option<bool>, String)]) -> Option<bool> {
        let mains: Vec<bool> = supplies
            .iter()
            .filter(|(t, _, _)| t == "Mains")
            .filter_map(|(_, o, _)| *o)
            .collect();
        if !mains.is_empty() {
            return Some(mains.iter().any(|o| *o));
        }
        let usb: Vec<bool> = supplies
            .iter()
            .filter(|(t, _, s)| t == "USB" && s != "Device")
            .filter_map(|(_, o, _)| *o)
            .collect();
        if !usb.is_empty() {
            return Some(usb.iter().any(|o| *o));
        }
        None
    }

    /// Read /sys/class/power_supply and decide whether a charger is plugged in.
    pub fn on_ac() -> Option<bool> {
        let root = std::path::Path::new("/sys/class/power_supply");
        let read = |p: std::path::PathBuf| {
            std::fs::read_to_string(p)
                .map(|s| s.trim().to_string())
                .unwrap_or_default()
        };
        let supplies: Vec<(String, Option<bool>, String)> = std::fs::read_dir(root)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| {
                let d = e.path();
                let online = match read(d.join("online")).as_str() {
                    "1" => Some(true),
                    "0" => Some(false),
                    _ => None,
                };
                (read(d.join("type")), online, read(d.join("scope")))
            })
            .collect();
        on_ac_from(&supplies)
    }

    /// `comm` of every running process. ~one small read per process; the
    /// daemon calls this every 5 s only while app rules exist.
    pub fn running_processes() -> Vec<String> {
        let mut out: Vec<String> = std::fs::read_dir("/proc")
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))
            })
            .filter_map(|e| std::fs::read_to_string(e.path().join("comm")).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn cfg() -> AutoModeConfig {
            AutoModeConfig {
                enabled: true,
                ac_mode: "performance".into(),
                battery_mode: "low-power".into(),
                app_rules: vec![AppRule {
                    process: "steam".into(),
                    mode: "max-power".into(),
                }],
                apps_on_battery: false,
            }
        }
        fn procs(p: &[&str]) -> Vec<String> {
            p.iter().map(|s| s.to_string()).collect()
        }

        #[test]
        fn battery_choices_follow_firmware() {
            let all = procs(&["low-power", "balanced", "performance", "max-power", "custom"]);
            assert_eq!(
                battery_choices(&all),
                procs(&["low-power", "balanced", "performance"])
            );
            let no_perf = procs(&["low-power", "balanced", "custom"]);
            assert_eq!(battery_choices(&no_perf), procs(&["low-power", "balanced"]));
            assert!(ac_only("custom") && ac_only("max-power") && !ac_only("balanced"));
        }

        #[test]
        fn resolve_rules() {
            let c = cfg();
            // No app: the last manual mode stays.
            assert_eq!(resolve(&c, true, &procs(&["bash"]), "balanced"), "balanced");
            // App on AC wins.
            assert_eq!(resolve(&c, true, &procs(&["steam"]), "balanced"), "max-power");
            // App rules are AC only unless "also on battery".
            assert_eq!(resolve(&c, false, &procs(&["steam"]), "low-power"), "low-power");
            let mut b = c.clone();
            b.apps_on_battery = true;
            // ...and even then never an AC-only mode on battery.
            assert_eq!(resolve(&b, false, &procs(&["steam"]), "low-power"), "low-power");
            b.app_rules[0].mode = "performance".into();
            assert_eq!(resolve(&b, false, &procs(&["Steam"]), "low-power"), "performance");
            // Disabled: nothing changes.
            let mut off = c.clone();
            off.enabled = false;
            assert_eq!(resolve(&off, true, &procs(&["steam"]), "quiet"), "quiet");
        }

        #[test]
        fn comm_truncation_matches() {
            assert!(rule_matches("steam", "steam"));
            assert!(rule_matches("Steam", "steam"));
            assert!(rule_matches("cyberpunk2077.exe", "cyberpunk2077.e"));
            assert!(!rule_matches("", "steam"));
            assert!(!rule_matches("steam", "steamwebhelper"));
        }

        #[test]
        fn switches_only_on_transitions() {
            let c = cfg();
            let mut s = Switcher::default();
            // First tick after enabling applies the power rule.
            assert_eq!(s.step(&c, true, &[], "balanced").as_deref(), Some("performance"));
            // Steady state: nothing, even if the user picked another mode.
            assert_eq!(s.step(&c, true, &[], "performance"), None);
            assert_eq!(s.step(&c, true, &[], "balanced"), None, "manual change holds");
            // Unplug: battery mode.
            assert_eq!(s.step(&c, false, &[], "balanced").as_deref(), Some("low-power"));
            // Manual Performance on battery holds until the next plug event.
            assert_eq!(s.step(&c, false, &[], "performance"), None);
            // Plug in.
            assert_eq!(s.step(&c, true, &[], "performance"), None, "already there");
            assert_eq!(s.step(&c, true, &[], "performance"), None);
        }

        #[test]
        fn app_rule_starts_and_restores() {
            let c = cfg();
            let mut s = Switcher::default();
            assert_eq!(s.step(&c, true, &[], "performance"), None);
            // User picks Balanced by hand, then starts steam.
            let steam = procs(&["steam"]);
            assert_eq!(s.step(&c, true, &steam, "balanced").as_deref(), Some("max-power"));
            assert_eq!(s.active_app(), Some("steam"));
            assert_eq!(s.step(&c, true, &steam, "max-power"), None);
            // Steam exits: back to the mode active before.
            assert_eq!(s.step(&c, true, &[], "max-power").as_deref(), Some("balanced"));
            assert_eq!(s.active_app(), None);
            // Steam runs, then the charger is pulled: the app rule is AC
            // only, so the battery mode applies.
            assert!(s.step(&c, true, &steam, "balanced").is_some());
            assert_eq!(s.step(&c, false, &steam, "max-power").as_deref(), Some("low-power"));
            assert_eq!(s.step(&c, false, &steam, "low-power"), None);
            // Plugged back in with steam still running: the app mode again.
            assert_eq!(s.step(&c, true, &steam, "low-power").as_deref(), Some("max-power"));
            // Steam exits: the AC rule (plug event happened meanwhile).
            assert_eq!(s.step(&c, true, &[], "max-power").as_deref(), Some("performance"));
        }

        #[test]
        fn disabling_resets_state() {
            let mut c = cfg();
            let mut s = Switcher::default();
            assert!(s.step(&c, true, &[], "balanced").is_some());
            c.enabled = false;
            assert_eq!(s.step(&c, false, &[], "balanced"), None);
            c.enabled = true;
            // Re-enabling applies the current power rule once.
            assert_eq!(s.step(&c, false, &[], "balanced").as_deref(), Some("low-power"));
        }

        #[test]
        fn ac_detection() {
            let e = |t: &str, o: Option<bool>, s: &str| (t.to_string(), o, s.to_string());
            assert_eq!(
                on_ac_from(&[e("Mains", Some(true), ""), e("Battery", None, "")]),
                Some(true)
            );
            assert_eq!(
                on_ac_from(&[e("Mains", Some(false), ""), e("USB", Some(true), "Device")]),
                Some(false)
            );
            assert_eq!(on_ac_from(&[e("USB", Some(true), "Device")]), None);
            assert_eq!(on_ac_from(&[e("USB", Some(true), "")]), Some(true));
            assert_eq!(on_ac_from(&[]), None);
        }

        #[test]
        fn validation() {
            let all = procs(&["low-power", "balanced", "performance", "max-power", "custom"]);
            assert!(validate(&cfg(), &all).is_ok());
            let mut bad = cfg();
            bad.battery_mode = "custom".into();
            assert!(validate(&bad, &all).is_err());
            let mut bad = cfg();
            bad.app_rules[0].process = "../x".into();
            assert!(validate(&bad, &all).is_err());
            let mut bad = cfg();
            bad.battery_mode = "performance".into();
            assert!(validate(&bad, &procs(&["low-power", "balanced"])).is_err());
        }
    }
}
