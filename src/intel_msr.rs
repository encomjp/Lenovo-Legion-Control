//! Intel MSR voltage offset via the `legion-intel-msr` sysfs driver.
//!
//! Borrowed from `lenovo-legion-cli/src/sysfs_drivers/offset_control.rs`:
//! - Base path `SYSFS_INTEL_MSR = "/sys/class/legion-intel-msr/intel-msr-0/"`
//!   (from offset_control.rs:6). On stock AMD (e.g. Legion Pro 7 16AFR10H
//!   9955HX3D) this path is absent — every helper returns `None` and
//!   `is_available()` is `false`.
//! - Per-plane files: `{plane}_offset` (i32, RW), `{plane}_offset_ctrl_supported`
//!   (u32 → bool), `{plane}_max_overvolt` (u32), `{plane}_max_undervolt` (u32).
//! - Five planes: `cpu`, `cache`, `gpu`, `analogio`, `uncore`.
//!
//! Gating: every read is `Path::exists` / `Option`-gated so an absent driver
//! never panics. `is_available()` additionally requires at least one plane to
//! report `supported == Some(true)`. Writes are bounds-checked against the
//! live `max_*` files (not hardcoded) and surface path-bearing `io::Error`s.
//!
//! No new dependencies — `fs::read_to_string` → `trim` → `parse`, and
//! `fs::write` with path in the error, mirroring `undervolt.rs`'s
//! `MIN..MAX` / ack-gate style but with per-plane dynamic limits.

use std::fs;
use std::io;
use std::path::Path;

/// Sysfs base exported by the `legion-intel-msr` kernel module.
/// Mirrors `offset_control.rs:6` exactly.
pub const SYSFS_INTEL_MSR: &str = "/sys/class/legion-intel-msr/intel-msr-0/";

/// Planes exposed by the driver. Order matches the task spec
/// `["cpu", "cache", "gpu", "analogio", "uncore"]`.
pub const PLANES: &[&str] = &["cpu", "cache", "gpu", "analogio", "uncore"];

/// Snapshot of one voltage plane. All sysfs reads are `Option`-gated:
/// `None` means the file was absent or unparseable (e.g. driver not loaded
/// on AMD).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plane {
    pub name: &'static str,
    pub offset_mv: Option<i32>,
    pub supported: Option<bool>,
    pub max_overvolt_mv: Option<u32>,
    pub max_undervolt_mv: Option<u32>,
}

// ---------------------------------------------------------------------------
// tiny sysfs helpers — return None if the file is missing or unparseable
// ---------------------------------------------------------------------------

fn read_i32(path: &str) -> Option<i32> {
    let v = fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse::<i32>().ok());
    if v.is_none() {
        log::trace!("intel_msr: read_i32 {path} → None (absent/unparseable)");
    }
    v
}

fn read_u32(path: &str) -> Option<u32> {
    let v = fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok());
    if v.is_none() {
        log::trace!("intel_msr: read_u32 {path} → None (absent/unparseable)");
    }
    v
}

fn path_for(plane: &str, suffix: &str) -> String {
    format!("{SYSFS_INTEL_MSR}{plane}_{suffix}")
}

// ---------------------------------------------------------------------------
// public API
// ---------------------------------------------------------------------------

/// Build a `Plane` snapshot for a single known plane name.
fn plane_snapshot(name: &'static str) -> Plane {
    let offset_mv = read_i32(&path_for(name, "offset"));
    let supported = read_u32(&path_for(name, "offset_ctrl_supported")).map(|v| v != 0);
    let max_overvolt_mv = read_u32(&path_for(name, "max_overvolt"));
    let max_undervolt_mv = read_u32(&path_for(name, "max_undervolt"));
    log::debug!(
        "intel_msr: plane '{name}' offset={offset_mv:?} supported={supported:?} \
         max_overvolt={max_overvolt_mv:?} max_undervolt={max_undervolt_mv:?}"
    );
    Plane {
        name,
        offset_mv,
        supported,
        max_overvolt_mv,
        max_undervolt_mv,
    }
}

/// True iff the driver is present **and** at least one plane reports
/// `supported == Some(true)`.
///
/// On AMD (no `legion-intel-msr` driver) this is `false` without panicking:
/// `Path::exists` fails fast and `planes()` yields all-`None` entries.
pub fn is_available() -> bool {
    if !Path::new(SYSFS_INTEL_MSR).exists() {
        log::debug!("intel_msr: is_available → false ({SYSFS_INTEL_MSR} absent)");
        return false;
    }
    let available = planes().iter().any(|p| p.supported == Some(true));
    log::debug!("intel_msr: is_available → {available} (driver dir present)");
    available
}

/// Enumerate the five planes. Always returns five entries; each field is
/// `None` when the corresponding sysfs file is absent (AMD-absent case).
pub fn planes() -> Vec<Plane> {
    PLANES.iter().map(|&name| plane_snapshot(name)).collect()
}

/// Set the voltage offset for `plane` to `mv` millivolts.
///
/// Validation (mirrors `undervolt.rs` bounds-check pattern, but with dynamic
/// per-plane limits from sysfs):
/// 1. `plane` must be one of `PLANES` — else `NotFound`.
/// 2. `supported` must be `Some(true)` — else `NotFound` (not supported on
///    this hardware / plane disabled).
/// 3. If `max_overvolt` is present and `mv` is positive, `mv <= max_overvolt`
///    else `InvalidInput`. If `max_undervolt` is present and `mv` is
///    negative, `-mv <= max_undervolt` else `InvalidInput`.
/// 4. Write `mv` to `{plane}_offset` via `fs::write`; any I/O error is
///    surfaced with the full sysfs path in the message.
pub fn set_offset(plane: &str, mv: i32) -> io::Result<()> {
    log::debug!("intel_msr: set_offset plane='{plane}' {mv} mV");
    if !PLANES.contains(&plane) {
        log::debug!("intel_msr: set_offset '{plane}' rejected — unknown plane");
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "unknown Intel MSR plane '{plane}': expected one of {} (path {}{plane}_offset)",
                PLANES.join(", "),
                SYSFS_INTEL_MSR
            ),
        ));
    }

    // Gate on the live `supported` knob — absent or 0 means not supported.
    let supported_path = path_for(plane, "offset_ctrl_supported");
    let supported = read_u32(&supported_path).map(|v| v != 0);
    match supported {
        Some(true) => {}
        _ => {
            log::debug!(
                "intel_msr: set_offset plane '{plane}' rejected — offset control not supported \
                 (supported={supported:?}, path {supported_path})"
            );
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "plane '{plane}' not supported (supported={supported:?}, path {supported_path})"
                ),
            ));
        }
    }

    // Dynamic bounds from the driver's max_* files. Missing files mean
    // "no bound advertised" — we allow the write and let the kernel enforce.
    let max_overvolt = read_u32(&path_for(plane, "max_overvolt"));
    let max_undervolt = read_u32(&path_for(plane, "max_undervolt"));

    if let Some(max_ov) = max_overvolt {
        if mv > max_ov as i32 {
            log::debug!(
                "intel_msr: set_offset plane '{plane}' rejected — {mv} mV exceeds \
                 max_overvolt {max_ov} mV"
            );
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "offset {mv} mV exceeds max_overvolt {max_ov} mV for plane '{plane}' (path {}{plane}_offset)",
                    SYSFS_INTEL_MSR
                ),
            ));
        }
    }
    if let Some(max_uv) = max_undervolt {
        // `max_undervolt` is a magnitude (e.g. 200 means -200 mV is the limit).
        if mv < 0 && (-(mv as i64) as u32) > max_uv {
            log::debug!(
                "intel_msr: set_offset plane '{plane}' rejected — {mv} mV exceeds \
                 max_undervolt {max_uv} mV"
            );
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "offset {mv} mV exceeds max_undervolt {max_uv} mV for plane '{plane}' (path {}{plane}_offset)",
                    SYSFS_INTEL_MSR
                ),
            ));
        }
    }

    log::debug!(
        "intel_msr: set_offset plane '{plane}' bounds ok \
         (mv={mv}, max_overvolt={max_overvolt:?}, max_undervolt={max_undervolt:?})"
    );

    let offset_path = path_for(plane, "offset");
    match fs::write(&offset_path, mv.to_string()) {
        Ok(()) => {
            log::debug!("intel_msr: {offset_path} ← {mv} mV");
            Ok(())
        }
        Err(e) => {
            log::debug!("intel_msr: set_offset plane '{plane}' write failed on {offset_path}: {e}");
            Err(io::Error::new(
                e.kind(),
                format!("cannot write {offset_path}: {e}"),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn base_path_matches_upstream() {
        // Must mirror offset_control.rs:6 exactly.
        assert_eq!(SYSFS_INTEL_MSR, "/sys/class/legion-intel-msr/intel-msr-0/");
    }

    #[test]
    fn planes_constant_is_five_known_names() {
        assert_eq!(PLANES, &["cpu", "cache", "gpu", "analogio", "uncore"]);
        assert_eq!(PLANES.len(), 5);
    }

    #[test]
    fn plane_struct_fields_are_option_gated() {
        let p = Plane {
            name: "cpu",
            offset_mv: None,
            supported: None,
            max_overvolt_mv: None,
            max_undervolt_mv: None,
        };
        assert_eq!(p.name, "cpu");
        assert!(p.offset_mv.is_none());
        assert!(p.supported.is_none());
    }

    #[test]
    fn set_offset_rejects_unknown_plane_with_not_found() {
        let err = set_offset("bogus", -50).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        let msg = err.to_string();
        assert!(msg.contains("unknown Intel MSR plane"), "{msg}");
        assert!(msg.contains(SYSFS_INTEL_MSR), "{msg}");
    }

    #[test]
    fn set_offset_rejects_unsupported_plane_with_not_found() {
        // On this AMD box every plane is unsupported (driver absent) — any
        // known plane must still return NotFound, not panic, and the error
        // must mention the sysfs path.
        if is_available() {
            // Intel Legion with driver present — nothing to assert offline.
            return;
        }
        for plane in PLANES {
            let err = set_offset(plane, -10).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::NotFound, "plane {plane}: {err}");
            assert!(
                err.to_string().contains("not supported") || err.to_string().contains("unknown"),
                "plane {plane}: {}",
                err
            );
        }
    }

    #[test]
    fn planes_enumerates_five_entries_without_panic() {
        let ps = planes();
        assert_eq!(ps.len(), 5);
        for (plane, expected) in ps.iter().zip(PLANES.iter()) {
            assert_eq!(plane.name, *expected);
        }
    }

    // Live verification on AMD: `is_available()` must be false and every
    // field must be None/unsupported. Marked `ignored` so CI never fails
    // on Intel, but `cargo test -- --ignored` exercises it on this box.
    #[test]
    #[ignore]
    fn live_amd_reports_not_supported_without_panic() {
        assert!(
            !Path::new(SYSFS_INTEL_MSR).exists(),
            "this test is for AMD without the legion-intel-msr driver"
        );
        assert!(!is_available(), "AMD should report not available");
        for p in planes() {
            assert!(
                p.offset_mv.is_none(),
                "plane {} offset should be None on AMD",
                p.name
            );
            assert!(
                p.supported.is_none() || p.supported == Some(false),
                "plane {} supported should be None/false on AMD, got {:?}",
                p.name,
                p.supported
            );
            assert!(
                p.max_overvolt_mv.is_none(),
                "plane {} max_overvolt should be None",
                p.name
            );
            assert!(
                p.max_undervolt_mv.is_none(),
                "plane {} max_undervolt should be None",
                p.name
            );
        }
        // Even with the driver absent, set_offset must not panic — it must
        // return NotFound with a path-bearing message.
        let err = set_offset("cpu", -50).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(err.to_string().contains(SYSFS_INTEL_MSR));
    }

    #[test]
    fn read_helpers_handle_missing_files_gracefully() {
        // Non-existent sysfs path must yield None, not panic.
        assert!(read_i32("/nonexistent/path/offset").is_none());
        assert!(read_u32("/nonexistent/path/max_overvolt").is_none());
    }

    #[test]
    fn set_offset_error_messages_are_path_bearing() {
        let err = set_offset("unknown_plane", 0).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains(SYSFS_INTEL_MSR),
            "error should contain base path: {msg}"
        );
        assert!(
            msg.contains("unknown_plane"),
            "error should contain plane name: {msg}"
        );
    }
}
