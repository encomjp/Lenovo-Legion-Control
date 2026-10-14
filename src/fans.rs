//! Fan control via lenovo_wmi_other (preferred) or legion_hwmon.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::device::{self, FanCapability};
use crate::sensors::hwmon_by_name;

/// How long a failed backend discovery is trusted before re-probing.
const REPROBE_INTERVAL: Duration = Duration::from_secs(60);

/// Pure: re-run discovery only when no miss is cached or it has expired.
fn reprobe_due(last_miss: Option<Instant>, now: Instant) -> bool {
    last_miss.is_none_or(|t| now.saturating_duration_since(t) >= REPROBE_INTERVAL)
}

/// Resolve the fan sysfs backend once (stable for machine uptime) instead of
/// re-scanning `/sys/class/hwmon` on every read/write — the UI polls RPM
/// every 2 s per fan, and each old call locked the hwmon cache mutex and
/// cloned a `Vec<PathBuf>`. A miss is cached too (re-probed every 60 s) so
/// machines without any backend don't re-run discovery on every poll.
fn fan_hwmon() -> Option<&'static (String, PathBuf)> {
    static BACKEND: OnceLock<(String, PathBuf)> = OnceLock::new();
    static LAST_MISS: Mutex<Option<Instant>> = Mutex::new(None);
    if let Some(backend) = BACKEND.get() {
        return Some(backend);
    }
    let mut last_miss = LAST_MISS.lock().unwrap_or_else(|e| e.into_inner());
    if !reprobe_due(*last_miss, Instant::now()) {
        return None;
    }
    match discover_fan_hwmon() {
        Some(discovered) => {
            let _ = BACKEND.set(discovered);
            BACKEND.get()
        }
        None => {
            *last_miss = Some(Instant::now());
            None
        }
    }
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

fn is_yogafan_quirk_model() -> bool {
    let product = std::fs::read_to_string("/sys/class/dmi/id/product_name").unwrap_or_default();
    let bios = std::fs::read_to_string("/sys/class/dmi/id/bios_version").unwrap_or_default();
    product.trim() == "83JG" || bios.trim().starts_with("R8CN")
}

/// True when the tachometer is locked behind yogafan (LOQ 15AHP10 83JG / R8CN).
/// `lenovo_wmi_other` binds but reads 0 RPM until `yogafan` is loaded.
pub fn needs_yogafan_setup() -> bool {
    if !is_yogafan_quirk_model() {
        return false;
    }
    let Some(wmi) = hwmon_by_name("lenovo_wmi_other") else {
        return false;
    };
    if !has_fan_inputs(&wmi) {
        return false;
    }
    if has_live_fan_input(&wmi) {
        return false;
    }
    // WMI tach is 0 — yogafan must provide it
    hwmon_by_name("yogafan").is_none()
}

/// Best-effort load of yogafan (daemon is root, no pkexec needed).
pub fn ensure_yogafan_loaded() -> bool {
    if hwmon_by_name("yogafan").is_some() {
        return true;
    }
    let _ = std::process::Command::new("modprobe")
        .args(["yogafan"])
        .output();
    // Give hwmon a moment to appear
    for _ in 0..10 {
        if hwmon_by_name("yogafan").is_some() {
            let _ = std::fs::write("/etc/modules-load.d/legion-yogafan.conf", "yogafan\n");
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    hwmon_by_name("yogafan").is_some()
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
        // LOQ 83JG (and similar): lenovo_wmi_other binds but the EC tach
        // is locked at 0. Real RPM is yogafan. Idle 0 rpm is valid there,
        // so do not wait for a nonzero yogafan sample before switching.
        if is_yogafan_quirk_model() && !has_live_fan_input(&hw) {
            if let Some(yw) = hwmon_by_name("yogafan").filter(|hw| has_fan_inputs(hw)) {
                log::debug!(
                    "fans::fan_hwmon: lenovo_wmi_other reads 0 on quirk model — using yogafan at {}",
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
        log::info!(
            "fans::fan_hwmon: EC fallback {name} available ({})",
            if dmi_ec_layout().is_some_and(|c| c.verified) {
                "fleet-verified layout"
            } else {
                "unverified layout — RPM shown as estimated"
            }
        );
        return Some((
            "ec-fallback".into(),
            PathBuf::from(format!("/tmp/ec-fallback-{name}")),
        ));
    }
    log::warn!(
        "fans::fan_hwmon: no RPM backend (lenovo_wmi_other / legion_hwmon / yogafan) — re-probe in {}s",
        REPROBE_INTERVAL.as_secs()
    );
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
        if EC_LAYOUT_REJECTED.load(Ordering::Relaxed) {
            return (None, FanRpmState::BackendUnavailable);
        }
        let Some(rpms) = ec_fallback_rpms_for(layout) else {
            return (None, FanRpmState::Unreadable);
        };
        let index = match fan {
            1 => 0,
            2 => 1,
            4 if !ids().contains(&2) => 1,
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
    rpm_backend_name().unwrap_or_else(|| device::detect().capabilities.fan_backend)
}

/// Live RPM backend only (no capability-profile fallback). Safe to call from
/// `device::probe_fans` — it never re-enters `device::detect`.
pub fn rpm_backend_name() -> Option<String> {
    fan_hwmon().map(|(name, _)| name.clone())
}

/// True when shown RPM may come from an EC-fallback layout that has not been
/// verified on this BIOS family. The EC debugfs path is the only non-hwmon
/// RPM source, so this is decidable without root (the GUI cannot read
/// debugfs itself but displays the daemon's numbers).
pub fn rpm_estimated() -> bool {
    let hwmon_tach = ["lenovo_wmi_other", "legion_hwmon", "yogafan"]
        .into_iter()
        .any(|name| hwmon_by_name(name).is_some_and(|hw| has_fan_inputs(&hw)));
    !hwmon_tach && !dmi_ec_layout().is_some_and(|c| c.verified)
}

/// Active EC-fallback layout name ("loq-ec" …), `None` for any other backend.
pub fn ec_layout_name() -> Option<String> {
    let (name, hw) = fan_hwmon()?;
    if name != "ec-fallback" {
        return None;
    }
    hw.file_name()?
        .to_str()?
        .strip_prefix("ec-fallback-")
        .map(str::to_string)
}

/// True once the constant-reading detector latched and rejected the layout.
pub fn ec_layout_rejected() -> bool {
    EC_LAYOUT_REJECTED.load(Ordering::Relaxed)
}

/// Pure: why a layout was chosen, as a short token for fleet queries.
/// `bios-family:<prefix>:verified|unverified`, `product-version:unverified`
/// or `probe:legacy-order` (unknown family, first plausible layout wins).
fn ec_layout_reason_for(bios_prefix: &str, product_version: &str) -> String {
    match ec_layout_for_family(bios_prefix, product_version) {
        Some(c) => {
            let verdict = if c.verified { "verified" } else { "unverified" };
            if ec_layout_for_family(bios_prefix, "").is_some() {
                format!("bios-family:{bios_prefix}:{verdict}")
            } else {
                format!("product-version:{verdict}")
            }
        }
        None => "probe:legacy-order".to_string(),
    }
}

/// Why the active EC layout was chosen (see [`ec_layout_reason_for`]).
pub fn ec_layout_reason() -> String {
    let bios = std::fs::read_to_string("/sys/class/dmi/id/bios_version").unwrap_or_default();
    let version = std::fs::read_to_string("/sys/class/dmi/id/product_version").unwrap_or_default();
    let prefix: String = bios.trim().chars().take(4).collect();
    ec_layout_reason_for(&prefix, version.trim())
}

/// Every known layout decoded from one EC snapshot, unfiltered: no plausibility
/// ceiling and no rejection, so the server can see which layout tracks CPU
/// temperature. Read-only; `None` when the EC debugfs is not readable.
pub fn ec_candidate_rpms() -> Option<Vec<(&'static str, Vec<u32>)>> {
    let buf = ec_fallback_bytes()?;
    Some(ec_candidates_from(&buf))
}

fn ec_candidates_from(buf: &[u8]) -> Vec<(&'static str, Vec<u32>)> {
    ["loq-ec", "ideapad-ec", "legion-ec"]
        .into_iter()
        .filter_map(|l| ec_layout_rpms(l, buf).map(|r| (l, r)))
        .collect()
}

pub fn control_backend_name() -> Option<String> {
    fan_control_hwmon().map(|(name, _)| name.clone())
}

/// Fan ids present on this machine (e.g. `[1, 2, 4]`).
pub fn ids() -> Vec<u8> {
    channels().into_iter().map(|f| f.id).collect()
}

/// Pure: the third ("aux") channel among `ids`. lenovo_wmi_other exposes it
/// as fan4, LenovoLegionLinux legion_hwmon as fan3 — prefer 4, else the
/// first id ≥ 3.
pub fn aux_id_from(ids: &[u8]) -> Option<u8> {
    if ids.contains(&4) {
        return Some(4);
    }
    ids.iter().copied().find(|id| *id >= 3)
}

/// Real hardware id of the third fan on this machine, if it has one.
pub fn aux_id() -> Option<u8> {
    aux_id_from(&ids())
}

/// Hardware ids behind the three flattened fan slots. The `fan4_*` fields in
/// `SensorReadings`, `config::UserProfile::fan4` and the CLI's "Aux" column
/// keep their historical names for telemetry/IPC compatibility, but mean
/// "third fan" — fan3 on legion_hwmon, fan4 on lenovo_wmi_other. Without a
/// third channel the slot stays 4 (absent → reads/writes are no-ops/errors).
pub fn slot_ids() -> [u8; 3] {
    [1, 2, aux_id().unwrap_or(4)]
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

/// Pure: a target above the channel's live max is not a real request —
/// 83F6 legion_hwmon reports `fan1_target=9600` with `fan1_max=5100` while
/// the EC drives the fan. Treat it as 0 (auto/unknown).
pub fn effective_target(target: u32, max: Option<u32>) -> u32 {
    match max {
        Some(max) if max > 0 && target > max => 0,
        _ => target,
    }
}

/// Target for display/saving: `read_target` with out-of-range values as auto.
pub fn display_target(fan: u8) -> u32 {
    let raw = read_target(fan).unwrap_or(0);
    let target = effective_target(raw, read_max(fan));
    if target != raw {
        log::trace!("fans::display_target: fan{fan} target {raw} above max — shown as auto");
    }
    target
}

/// UI-friendly RPM label. Auto mode often reports 0 on this WMI driver.
/// Unverified EC-fallback readings are marked "(est.)".
pub fn rpm_label(fan: u8) -> String {
    let rpm = read_rpm(fan).unwrap_or(0);
    let label = format_rpm_label(display_target(fan), rpm);
    if rpm > 0 && rpm_estimated() {
        format!("{label} (est.)")
    } else {
        label
    }
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
    if !device::vendor_supported() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "unsupported vendor: fan control is Lenovo-only",
        ));
    }
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

/// EC-fallback layout for a BIOS family. `verified` = fleet telemetry showed
/// the layout tracking load on that family; anything else is best-effort and
/// its RPM is flagged as estimated (`rpm_estimated`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EcLayoutChoice {
    layout: &'static str,
    verified: bool,
}

/// Pure: pick the EC layout from DMI instead of the first layout that happens
/// to hold a plausible-looking byte pair. Fleet data showed "loq-ec" reading
/// a constant ~3900-4300 rpm (not a tachometer) on 82K2/82MJ (H3CN), 82YA
/// (M2CN) and 83DG (NMCN). `None` = unknown family → legacy probe order.
fn ec_layout_for_family(bios_prefix: &str, product_version: &str) -> Option<EcLayoutChoice> {
    let choice = |layout, verified| Some(EcLayoutChoice { layout, verified });
    match bios_prefix {
        // 82RF Legion 5 Pro 16IAH7H: 0x06/0x07 ×100 tracks load (400 → 4332 rpm).
        "J2CN" => choice("ideapad-ec", true),
        "LZCN" => choice("loq-ec", false),
        "H3CN" | "H4CN" | "FCCN" | "JNCN" => choice("ideapad-ec", false),
        // Legion Gen 6-8 BIOS families.
        "EFCN" | "EUCN" | "FSCN" | "G8CN" | "G9CN" | "GKCN" | "H1CN" | "HACN" | "HHCN" | "J1CN"
        | "JUCN" | "JVCN" | "K1CN" | "K9CN" | "KFCN" | "KHCN" | "KWCN" | "LPCN" | "M0CN"
        | "M1CN" | "M2CN" | "M3CN" | "M6CN" | "DMCN" | "NSCN" => choice("legion-ec", false),
        _ if product_version.contains("LOQ 15IRH8") => choice("loq-ec", false),
        _ if product_version.contains("IdeaPad Gaming") => choice("ideapad-ec", false),
        _ => None,
    }
}

/// DMI-derived layout, read once (no `device::detect` — this runs inside it).
fn dmi_ec_layout() -> Option<EcLayoutChoice> {
    static CHOICE: OnceLock<Option<EcLayoutChoice>> = OnceLock::new();
    *CHOICE.get_or_init(|| {
        let bios = std::fs::read_to_string("/sys/class/dmi/id/bios_version").unwrap_or_default();
        let version =
            std::fs::read_to_string("/sys/class/dmi/id/product_version").unwrap_or_default();
        let prefix: String = bios.trim().chars().take(4).collect();
        let choice = ec_layout_for_family(&prefix, version.trim());
        log::debug!(
            "fans::dmi_ec_layout: bios {prefix:?} / {:?} → {choice:?}",
            version.trim()
        );
        choice
    })
}

/// Pure: try the family's layout (or the legacy order when unknown) and
/// return the first whose values look like a tachometer (any value in
/// 500..=ceiling).
fn select_ec_layout(
    choice: Option<EcLayoutChoice>,
    buf: &[u8],
) -> Option<(&'static str, Vec<u32>)> {
    const LEGACY: [&str; 3] = ["loq-ec", "ideapad-ec", "legion-ec"];
    let chosen;
    let candidates: &[&'static str] = match choice {
        Some(c) => {
            chosen = [c.layout];
            &chosen
        }
        None => &LEGACY,
    };
    for &layout in candidates {
        let rpms = ec_layout_rpms(layout, buf)?;
        let ceiling = if layout == "ideapad-ec" { 6000 } else { 7500 };
        if rpms.iter().any(|rpm| (500..=ceiling).contains(rpm)) {
            return Some((layout, rpms));
        }
    }
    None
}

/// EC direct fallback: read fan RPMs via ec_sys debugfs.
/// Returns (sensor_name, [cpu_rpm, gpu_rpm]) if readable and plausible.
/// Layouts: LOQ 15IRH8 (0xE3/0xE7 16-bit LE), IdeaPad Gaming 3 (0x06 8-bit
/// ×100) and Legion (0xE0/0xE2 16-bit LE), chosen by BIOS family.
fn ec_fallback_rpms() -> Option<(String, Vec<u32>)> {
    let buf = ec_fallback_bytes()?;
    select_ec_layout(dmi_ec_layout(), &buf).map(|(layout, rpms)| (layout.to_string(), rpms))
}

/// Latched once the constant-reading detector rejects the EC layout.
static EC_LAYOUT_REJECTED: AtomicBool = AtomicBool::new(false);

/// Detects an EC-fallback layout that reads a non-tachometer constant: both
/// fans within ±10 rpm of each other, the value itself within a 300 rpm band,
/// across ≥3 consecutive samples while the CPU temperature moved by more than
/// 15 °C. Real fans diverge or ramp with that much thermal change.
#[derive(Debug, Default)]
struct EcConstancy {
    streak: u32,
    temp_lo: f64,
    temp_hi: f64,
    rpm_lo: u32,
    rpm_hi: u32,
}

impl EcConstancy {
    const fn new() -> Self {
        Self {
            streak: 0,
            temp_lo: 0.0,
            temp_hi: 0.0,
            rpm_lo: 0,
            rpm_hi: 0,
        }
    }

    fn restart(&mut self, lo: u32, hi: u32, cpu_temp: f64) {
        self.streak = 1;
        (self.temp_lo, self.temp_hi) = (cpu_temp, cpu_temp);
        (self.rpm_lo, self.rpm_hi) = (lo, hi);
    }

    /// Feed one sample; returns true when the layout should be rejected.
    fn observe(&mut self, fan1: u32, fan2: u32, cpu_temp: f64) -> bool {
        let suspicious = fan1 > 0 && fan2 > 0 && fan1.abs_diff(fan2) <= 10 && cpu_temp > 0.0;
        if !suspicious {
            self.streak = 0;
            return false;
        }
        let (lo, hi) = (fan1.min(fan2), fan1.max(fan2));
        if self.streak == 0 || self.rpm_lo.min(lo).abs_diff(self.rpm_hi.max(hi)) > 300 {
            // First sample, or the value is moving like a real tachometer.
            self.restart(lo, hi, cpu_temp);
            return false;
        }
        self.streak += 1;
        self.temp_lo = self.temp_lo.min(cpu_temp);
        self.temp_hi = self.temp_hi.max(cpu_temp);
        self.rpm_lo = self.rpm_lo.min(lo);
        self.rpm_hi = self.rpm_hi.max(hi);
        self.streak >= 3 && self.temp_hi - self.temp_lo > 15.0
    }
}

/// Feed the EC-fallback plausibility check with one sensor sample (called
/// from `sensors::read_all`). No-op unless the RPM backend is ec-fallback.
pub fn observe_ec_sample(fan1: u32, fan2: u32, cpu_temp: f64) {
    static STATE: Mutex<EcConstancy> = Mutex::new(EcConstancy::new());
    if EC_LAYOUT_REJECTED.load(Ordering::Relaxed)
        || !fan_hwmon().is_some_and(|(name, _)| name == "ec-fallback")
    {
        return;
    }
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if state.observe(fan1, fan2, cpu_temp) {
        EC_LAYOUT_REJECTED.store(true, Ordering::Relaxed);
        log::warn!(
            "fans: EC-fallback RPM rejected — fans read {fan1}/{fan2} rpm, constant while CPU moved {:.0}–{:.0} °C (not a tachometer)",
            state.temp_lo,
            state.temp_hi
        );
    }
}

fn ec_fallback_rpms_for(layout: &str) -> Option<Vec<u32>> {
    let buf = ec_fallback_bytes()?;
    // 83JG (IT5508) uses 0xFE/0xFF via yogafan, not 0xE3/0xE7 — garbage is 0xFF fill (65535)
    if buf.iter().filter(|&&b| b == 0xFF).count() > 200 {
        return None;
    }
    let rpms = ec_layout_rpms(layout, &buf)?;
    let ceiling = if layout == "ideapad-ec" { 6000 } else { 7500 };
    if rpms.iter().any(|rpm| *rpm > ceiling) {
        return None;
    }
    Some(rpms)
}

fn ec_fallback_bytes() -> Option<Vec<u8>> {
    use std::io::Read;
    const EC_IO: &str = "/sys/kernel/debug/ec/ec0/io";
    // Try to ensure ec_sys is loaded if the file doesn't exist. Deliberately
    // WITHOUT write_support=1: this fallback only READS tachometer bytes, and
    // write_support would grant every root process EC-RAM write access
    // system-wide — far more privilege than an RPM read needs. At most one
    // modprobe per process (Fedora ships no ec_sys; it would respawn forever).
    static MODPROBE_TRIED: AtomicBool = AtomicBool::new(false);
    if !std::path::Path::new(EC_IO).exists() {
        if MODPROBE_TRIED.swap(true, Ordering::Relaxed) {
            return None;
        }
        let _ = std::process::Command::new("modprobe")
            .args(["ec_sys"])
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

/// User-facing caveat for manual fan targets on Gen 10 EC firmware.
pub const GEN10_MANUAL_TARGET_WARNING: &str =
    "manual fan targets may persist until EC reset on some Gen 10 firmware";

/// Pure: generations whose EC may treat a manual target as one-way. On Gen 10
/// (83F5 / Q7CN, EC 0x5508) writing 0 reportedly does not hand the fan back
/// to the EC curve — only a full EC reset does (VitaminDB). Unverified on
/// other Gen 10 units, so this only drives warnings, never write behaviour.
pub fn gen_may_latch_manual_target(gen: u8) -> bool {
    gen >= 10
}

/// True on machines where `GEN10_MANUAL_TARGET_WARNING` applies.
pub fn manual_target_may_persist() -> bool {
    gen_may_latch_manual_target(device::model_gen())
}

/// Set all discovered fans to auto mode (writes target 0 to every fan).
pub fn set_auto() -> std::io::Result<()> {
    let all_ids = ids();
    log::info!(
        "fans::set_auto: switching to auto on {} fan(s): {all_ids:?} (target 0 = EC curve)",
        all_ids.len()
    );
    if manual_target_may_persist() {
        log::warn!(
            "fans::set_auto: Gen 10 EC — {GEN10_MANUAL_TARGET_WARNING}; target 0 is written but the EC may not resume its own curve"
        );
    }
    let mut last_err = None;
    let mut errors = 0usize;
    for id in all_ids {
        // Never re-write 0 to a fan that is already on auto: on Gen 10 EC
        // firmware each write can restart the fan-stop hold, so the old
        // unconditional write on every daemon start/stop kept fans at 0 rpm
        // well past their normal spin-up temperature.
        if read_target(id) == Some(0) {
            log::debug!("fans::set_auto: fan{id} already auto — no write");
            continue;
        }
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
    fn target_above_max_is_auto() {
        assert_eq!(effective_target(9600, Some(5100)), 0); // 83F6 garbage
        assert_eq!(effective_target(5100, Some(5100)), 5100);
        assert_eq!(effective_target(3000, Some(5100)), 3000);
        assert_eq!(effective_target(9600, None), 9600, "no live max → keep");
        assert_eq!(effective_target(9600, Some(0)), 9600, "bogus max → keep");
        assert_eq!(
            format_rpm_label(effective_target(9600, Some(5100)), 2200),
            "Auto · 2200 RPM"
        );
    }

    #[test]
    fn manual_target_warning_is_gen10_and_newer() {
        assert!(!gen_may_latch_manual_target(0));
        assert!(!gen_may_latch_manual_target(9));
        assert!(gen_may_latch_manual_target(10));
        assert!(gen_may_latch_manual_target(11));
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
    fn aux_id_resolves_third_channel_per_backend() {
        assert_eq!(aux_id_from(&[1, 2, 4]), Some(4)); // lenovo_wmi_other
        assert_eq!(aux_id_from(&[1, 2, 3]), Some(3)); // legion_hwmon (83F6)
        assert_eq!(aux_id_from(&[1, 2, 3, 4]), Some(4));
        assert_eq!(aux_id_from(&[1, 2]), None);
        assert_eq!(aux_id_from(&[]), None);
    }

    #[test]
    fn negative_discovery_is_cached_for_the_reprobe_interval() {
        let now = Instant::now();
        assert!(reprobe_due(None, now));
        assert!(!reprobe_due(Some(now), now));
        assert!(!reprobe_due(Some(now), now + Duration::from_secs(59)));
        assert!(reprobe_due(Some(now), now + REPROBE_INTERVAL));
    }

    #[test]
    fn ec_layout_follows_bios_family() {
        let pick = |b, v| ec_layout_for_family(b, v).map(|c| (c.layout, c.verified));
        assert_eq!(pick("J2CN", ""), Some(("ideapad-ec", true)));
        assert_eq!(pick("LZCN", ""), Some(("loq-ec", false)));
        assert_eq!(
            pick("H3CN", "IdeaPad Gaming 3 15ACH6"),
            Some(("ideapad-ec", false))
        );
        assert_eq!(pick("M2CN", ""), Some(("legion-ec", false)));
        assert_eq!(pick("KWCN", ""), Some(("legion-ec", false)));
        assert_eq!(pick("XXCN", "LOQ 15IRH8"), Some(("loq-ec", false)));
        assert_eq!(
            pick("XXCN", "IdeaPad Gaming 3 15IHU6"),
            Some(("ideapad-ec", false))
        );
        assert_eq!(pick("NMCN", "Legion Y7000P IRX9"), None);
    }

    #[test]
    fn family_layout_never_falls_through_to_loq_bytes() {
        // loq-ec bytes hold a plausible ~3900 pair; ideapad-ec bytes are 0.
        let mut buf = vec![0u8; 256];
        buf[0xe3..0xe5].copy_from_slice(&3900u16.to_le_bytes());
        buf[0xe7..0xe9].copy_from_slice(&3905u16.to_le_bytes());
        let ideapad = Some(EcLayoutChoice {
            layout: "ideapad-ec",
            verified: false,
        });
        assert_eq!(select_ec_layout(ideapad, &buf), None);
        // Unknown family keeps the legacy first-plausible probe.
        assert_eq!(
            select_ec_layout(None, &buf),
            Some(("loq-ec", vec![3900, 3905]))
        );
        buf[0x06] = 25;
        assert_eq!(
            select_ec_layout(ideapad, &buf),
            Some(("ideapad-ec", vec![2500, 2500]))
        );
    }

    #[test]
    fn constant_ec_pair_is_rejected_when_temperature_moves() {
        let mut d = EcConstancy::default();
        assert!(!d.observe(3921, 3925, 40.0));
        assert!(!d.observe(3930, 3926, 50.0));
        assert!(
            d.observe(3918, 3920, 70.0),
            "3 samples, 30 °C swing, flat pair"
        );
    }

    #[test]
    fn constant_ec_pair_needs_temperature_swing_and_streak() {
        let mut d = EcConstancy::default();
        for _ in 0..10 {
            assert!(!d.observe(3900, 3900, 45.0), "steady temp is not evidence");
        }
        let mut d = EcConstancy::default();
        assert!(!d.observe(3900, 3900, 40.0));
        assert!(!d.observe(3900, 3900, 70.0));
        assert!(
            !d.observe(2400, 4000, 75.0),
            "diverging fans reset the streak"
        );
        assert!(!d.observe(3900, 3900, 40.0));
        assert!(!d.observe(3900, 3900, 70.0));
    }

    #[test]
    fn ramping_or_idle_fans_are_not_rejected() {
        let mut d = EcConstancy::default();
        assert!(!d.observe(1900, 1900, 40.0));
        assert!(!d.observe(2900, 2900, 60.0));
        assert!(!d.observe(3900, 3900, 80.0), "value ramps with temperature");
        let mut d = EcConstancy::default();
        for t in [40.0, 60.0, 80.0, 95.0] {
            assert!(!d.observe(0, 0, t), "fan-stop zero is not a constant pair");
        }
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

    #[test]
    fn idle_zero_rpm_is_not_a_live_reading() {
        let dir = std::env::temp_dir().join(format!("legion-fan-idle-zero-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("fan1_input"), "0\n").unwrap();
        std::fs::write(dir.join("fan2_input"), "0\n").unwrap();
        assert!(has_fan_inputs(&dir));
        assert!(
            !has_live_fan_input(&dir),
            "WMI/yogafan idle 0 must not count as a live tachometer"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn layout_reason_tokens() {
        assert_eq!(ec_layout_reason_for("J2CN", ""), "bios-family:J2CN:verified");
        assert_eq!(ec_layout_reason_for("H3CN", ""), "bios-family:H3CN:unverified");
        assert_eq!(
            ec_layout_reason_for("ZZZZ", "LOQ 15IRH8"),
            "product-version:unverified"
        );
        assert_eq!(ec_layout_reason_for("ZZZZ", "x"), "probe:legacy-order");
    }

    #[test]
    fn candidates_cover_every_layout_unfiltered() {
        let mut buf = vec![0u8; 256];
        buf[0xe0] = 0x10;
        buf[0xe1] = 0x0e; // legion fan1 = 3600
        buf[0xe3] = 0xff;
        buf[0xe4] = 0xff; // loq fan1 = 65535, must NOT be filtered out
        buf[0x06] = 40; // ideapad = 4000
        let c = ec_candidates_from(&buf);
        let names: Vec<_> = c.iter().map(|(n, _)| *n).collect();
        assert_eq!(names, ["loq-ec", "ideapad-ec", "legion-ec"]);
        assert_eq!(c[0].1[0], 65535);
        assert_eq!(c[1].1, vec![4000, 4000]);
        assert_eq!(c[2].1[0], 3600);
        assert!(ec_candidates_from(&buf[..16]).is_empty(), "short buffer");
    }
}
