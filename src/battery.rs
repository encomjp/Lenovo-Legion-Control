//! Battery info + charge limit modes for Lenovo Legion.
//!
//! Legion firmware exposes ONE charge limiter (ACPI GBMD bit 5) behind two
//! aliased interfaces: legacy `conservation_mode` and standardized
//! `charge_types` (kernel ≥6.14). They are not independent — either write
//! flips the same bit. We therefore write ONLY `charge_types` (single write,
//! then verify read-back) and treat `conservation_mode` as a read-only
//! fallback on machines lacking `charge_types`.
//!
//! The firmware threshold is fixed per generation (Legion Pro 7: ~75–80%
//! per Lenovo's manual; older IdeaPads: 55–60%) and cannot be changed from
//! Linux — "60%" and "80%" requests collapse onto the same limiter.
//!
//! Known failure mode (documented by Gen-10 owners and kernel Bug 221065):
//! the EC can silently drop or garble the state across AC plug events /
//! suspend. `reassert_configured_limit()` lets the daemon detect and repair
//! that.

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;

/// Charge limit requested through this process (discretized). 0 = unknown —
/// before any explicit request the watchdog must not touch anything.
static DESIRED_LIMIT: AtomicU32 = AtomicU32::new(0);

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
                log::trace!(
                    "battery::conservation_path — using known path {}",
                    known.display()
                );
                log::debug!(
                    "battery::conservation_path — conservation_mode found at known path {}",
                    known.display()
                );
                return Some(known.to_path_buf());
            }
            // Fallback: scan platform drivers for ideapad_acpi.
            match std::fs::read_dir("/sys/bus/platform/drivers/ideapad_acpi") {
                Ok(entries) => {
                    let mut scanned = 0usize;
                    for entry in entries.flatten() {
                        scanned += 1;
                        let candidate = entry.path().join("conservation_mode");
                        if candidate.exists() {
                            log::trace!(
                                "battery::conservation_path — discovered {} via ideapad_acpi scan",
                                candidate.display()
                            );
                            log::debug!(
                                "battery::conservation_path — conservation_mode found at {} after scanning {scanned} ideapad_acpi dir(s)",
                                candidate.display()
                            );
                            return Some(candidate);
                        }
                    }
                    log::debug!(
                        "battery::conservation_path — scanned {scanned} ideapad_acpi dir(s), none exposed conservation_mode"
                    );
                }
                Err(e) => {
                    log::debug!(
                        "battery::conservation_path — ideapad_acpi driver dir scan failed: {e}"
                    );
                }
            }
            log::debug!("battery::conservation_path — no conservation_mode found anywhere");
            None
        })
        .as_ref()
        .map(std::path::PathBuf::as_path)
}

fn read(path: &str) -> Option<String> {
    match std::fs::read_to_string(Path::new(path)) {
        Ok(s) => {
            let s = s.trim().to_string();
            log::trace!("battery::read — fetched {path} = {s:?}");
            Some(s)
        }
        Err(e) => {
            log::trace!("battery::read — {path} read returned None: {e}");
            match e.kind() {
                std::io::ErrorKind::NotFound => {
                    log::debug!("battery::read — {path}: file missing (does not exist)")
                }
                std::io::ErrorKind::PermissionDenied => {
                    log::debug!("battery::read — {path}: permission denied")
                }
                _ => log::debug!("battery::read — {path}: read failed: {e}"),
            }
            None
        }
    }
}

/// Parse a sysfs value, surfacing (not silently dropping) parse failures.
fn parse_sysfs<T: std::str::FromStr>(attr: &str, raw: String) -> Option<T> {
    match raw.parse::<T>() {
        Ok(v) => Some(v),
        Err(_) => {
            log::trace!("battery::parse_sysfs — {attr} returned None (raw={raw:?})");
            None
        }
    }
}

/// Resolve the battery sysfs directory. Legion models expose BAT0, but some
/// (LOQ 15AHP10 observed in fleet telemetry) enumerate as BAT1/BAT0 in a
/// different order or BATT — probe instead of hardcoding.
fn bat_dir() -> std::path::PathBuf {
    const BASE: &str = "/sys/class/power_supply";
    for name in ["BAT0", "BAT1", "BAT2", "BATT"] {
        let dir = std::path::PathBuf::from(format!("{BASE}/{name}"));
        if dir.join("capacity").is_file() || dir.join("status").is_file() {
            log::trace!("battery::bat_dir — using {}", dir.display());
            return dir;
        }
    }
    // Fall back to the historical constant so error messages stay stable.
    std::path::PathBuf::from(format!("{BASE}/BAT0"))
}

/// Read one battery sysfs attribute from the probed battery dir, trimmed,
/// with parse + trace logging.
fn read_attr<T: std::str::FromStr + std::fmt::Debug>(attr: &str) -> Option<T> {
    let r =
        read(&format!("{}/{attr}", bat_dir().display())).and_then(|s| parse_sysfs::<T>(attr, s));
    log::trace!("battery::{attr} — result={r:?}");
    r
}

pub fn capacity() -> Option<u32> {
    read_attr("capacity")
}

pub fn status() -> Option<String> {
    read_attr("status")
}

pub fn voltage() -> Option<f64> {
    read_attr::<i64>("voltage_now").map(|v| v as f64 / 1_000_000.0)
}

pub fn cycles() -> Option<u32> {
    read_attr("cycle_count")
}

pub fn power_w() -> Option<f64> {
    read_attr::<i64>("power_now").map(|v| v as f64 / 1_000_000.0)
}

pub fn energy_now_wh() -> Option<f64> {
    read_attr::<i64>("energy_now").map(|v| v as f64 / 1_000_000.0)
}

pub fn energy_full_wh() -> Option<f64> {
    read_attr::<i64>("energy_full").map(|v| v as f64 / 1_000_000.0)
}

pub fn energy_design_wh() -> Option<f64> {
    read_attr::<i64>("energy_full_design").map(|v| v as f64 / 1_000_000.0)
}

pub fn health_pct() -> Option<f64> {
    log::trace!("battery::health_pct()");
    let full = match energy_full_wh() {
        Some(v) => v,
        None => {
            log::debug!("battery::health_pct — energy_full unreadable, skipping health calc");
            return None;
        }
    };
    let design = match energy_design_wh() {
        Some(v) => v,
        None => {
            log::debug!(
                "battery::health_pct — energy_full_design unreadable, skipping health calc"
            );
            return None;
        }
    };
    let h = health_from_wh(full, design);
    log::debug!(
        "battery::health_pct — computed health={h:?}% (full={full:.2}Wh, design={design:.2}Wh)"
    );
    h
}

pub fn manufacturer() -> Option<String> {
    read_attr("manufacturer")
}

pub fn model_name() -> Option<String> {
    read_attr("model_name")
}

pub fn technology() -> Option<String> {
    read_attr("technology")
}

pub fn charge_types() -> Option<String> {
    read_attr("charge_types")
}

pub fn conservation_mode() -> Option<bool> {
    log::trace!("battery::conservation_mode()");
    let path = match conservation_path() {
        Some(p) => p,
        None => {
            log::debug!("battery::conservation_mode — conservation_mode file not found");
            return None;
        }
    };
    // Prefer ideapad conservation_mode when present
    if let Some(v) = read(&path.to_string_lossy()) {
        let on = v.trim() == "1";
        log::debug!(
            "battery::conservation_mode — read from {}: {on}",
            path.display()
        );
        return Some(on);
    }
    let types = match charge_types() {
        Some(t) => t,
        None => {
            log::debug!(
                "battery::conservation_mode — charge_types unreadable, legacy fallback impossible"
            );
            return None;
        }
    };
    let on = types.contains("[Long_Life]");
    log::debug!("battery::conservation_mode — fallback from charge_types={types:?}: {on}");
    Some(on)
}

fn set_conservation_file(on: bool) -> std::io::Result<()> {
    log::trace!("battery::set_conservation_file(on={on})");
    let found = conservation_path().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "conservation_mode not found")
    });
    let path = match found {
        Ok(p) => p,
        Err(e) => {
            log::debug!("battery::set_conservation_file — conservation_mode missing: {e}");
            return Err(e);
        }
    };
    let res = std::fs::write(path, if on { "1" } else { "0" });
    match &res {
        Ok(()) => log::debug!(
            "battery::set_conservation_file — wrote {} to {}",
            if on { "1" } else { "0" },
            path.display()
        ),
        Err(e) => log::debug!(
            "battery::set_conservation_file — write to {} failed: {e}",
            path.display()
        ),
    }
    res
}

fn set_charge_type(val: &str) -> std::io::Result<()> {
    log::trace!("battery::set_charge_type(val={val})");
    let path = bat_dir().join("charge_types");
    let res = std::fs::write(&path, val);
    match &res {
        Ok(()) => log::debug!("battery::set_charge_type — wrote charge_types={val}"),
        Err(e) => log::debug!(
            "battery::set_charge_type — write to {} failed: {e}",
            path.display()
        ),
    }
    res
}

/// Parse the bracketed active selection out of a charge_types value
/// ("Fast [Standard] Long_Life" → "Standard"). Pure helper; exported for tests.
fn parse_selection(types: &str) -> Option<String> {
    let open = types.find('[')?;
    let close = types[open + 1..].find(']')? + open + 1;
    Some(types[open + 1..close].to_string())
}

fn selected_charge_type() -> Option<String> {
    let types = match charge_types() {
        Some(t) => t,
        None => {
            log::debug!("battery::selected_charge_type — charge_types unreadable, returning None");
            return None;
        }
    };
    let r = match parse_selection(&types) {
        Some(sel) => Some(sel),
        None => {
            log::debug!(
                "battery::selected_charge_type — no bracketed selection in charge_types={types:?}, returning None"
            );
            None
        }
    };
    log::trace!("battery::selected_charge_type — result={r:?}");
    r
}

/// Legacy boolean API — maps to ~60% conservation when on, Standard when off.
pub fn set_conservation(on: bool) -> std::io::Result<()> {
    log::trace!("battery::set_conservation(on={on})");
    let r = if on {
        set_charge_limit_pct(60)
    } else {
        set_charge_limit_pct(100)
    };
    log::debug!(
        "battery::set_conservation — delegated to set_charge_limit_pct, ok={}",
        r.is_ok()
    );
    r
}

/// Current effective charge limit percentage: 80 when the firmware limiter is
/// engaged, else 100. Legacy `conservation_mode`-only machines (no
/// `charge_types` attr) report 60 while conservation is on.
pub fn charge_limit_pct() -> u32 {
    log::trace!("battery::charge_limit_pct()");
    if let Some(sel) = selected_charge_type() {
        if sel.eq_ignore_ascii_case("Long_Life") {
            log::debug!("battery::charge_limit_pct — source=charge_types selection Long_Life → 80");
            return 80;
        }
        if sel.eq_ignore_ascii_case("Standard") || sel.eq_ignore_ascii_case("Fast") {
            log::debug!("battery::charge_limit_pct — source=charge_types selection {sel} → 100");
            return 100;
        }
        log::debug!(
            "battery::charge_limit_pct — unrecognized charge_types selection {sel:?}, falling through"
        );
    }
    if let Some(path) = conservation_path() {
        if read(&path.to_string_lossy()).as_deref() == Some("1") {
            log::debug!("battery::charge_limit_pct — source=legacy conservation_mode → 60");
            return 60;
        }
    }
    log::debug!("battery::charge_limit_pct — source=default → 100");
    100
}

/// Set charge limit. Any value < 100 engages the firmware limiter (one
/// feature: ~75–80% on current Legion firmware); ≥ 100 charges to full.
///
/// Writes the standardized `charge_types` switch only — `conservation_mode`
/// aliases the same firmware bit, and writing both in sequence was the old
/// self-undoing bug. Verifies the read-back selection and returns an error
/// when the EC did not accept the mode instead of reporting silent success.
pub fn set_charge_limit_pct(pct: u32) -> std::io::Result<()> {
    set_charge_limit_pct_inner(pct, true)
}

/// Watchdog variant: repair attempts are reported by the daemon only when
/// their state changes, not once per polling interval.
fn set_charge_limit_pct_quiet(pct: u32) -> std::io::Result<()> {
    set_charge_limit_pct_inner(pct, false)
}

fn set_charge_limit_pct_inner(pct: u32, announce: bool) -> std::io::Result<()> {
    log::trace!("battery::set_charge_limit_pct(pct={pct})");
    let pct = discretize_limit(pct);
    let preserve = pct < 100;
    let want = if preserve { "Long_Life" } else { "Standard" };
    if announce {
        log::info!(
            "charge limit → {} ({want})",
            if preserve { "preserved" } else { "full" }
        );
    } else {
        log::debug!(
            "charge limit repair → {} ({want})",
            if preserve { "preserved" } else { "full" }
        );
    }
    log::debug!(
        "battery::set_charge_limit_pct — request mapped to {pct} (preserve={preserve}, target={want})"
    );

    DESIRED_LIMIT.store(pct, Ordering::Relaxed);

    // Preferred path: standardized charge_types switch.
    if charge_types().is_some() {
        log::debug!("battery::set_charge_limit_pct — write path: charge_types={want}");
        if let Err(e) = set_charge_type(want) {
            log::debug!("battery::set_charge_limit_pct — charge_types write failed: {e}");
            return Err(e);
        }
        // The EC may take a few hundred ms to reflect the new selection
        // (and kernel Bug 221065 can garble the first readback after AC
        // events) — poll the readback instead of trusting a single sample.
        for attempt in 0..5 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            match selected_charge_type() {
                Some(t) if t.eq_ignore_ascii_case(want) => {
                    if announce {
                        log::info!(
                            "battery::set_charge_limit_pct — verified charge_types={want} on readback {} of 5",
                            attempt + 1
                        );
                    } else {
                        log::debug!(
                            "battery::set_charge_limit_pct — verified charge_types={want} on readback {} of 5",
                            attempt + 1
                        );
                    }
                    return Ok(());
                }
                _ => {
                    log::debug!(
                        "battery::set_charge_limit_pct — readback {} of 5 did not read {want} yet (attempt {})",
                        attempt + 1,
                        attempt
                    );
                }
            }
        }
        let last_readback = selected_charge_type();
        log::debug!(
            "battery::set_charge_limit_pct — verification exhausted after 5 retries, last readback={last_readback:?}"
        );
        let msg = format!(
            "Firmware did not accept charge_types={want} (reads {:?}) — charging may be uncapped. AC plug/unplug or reboot usually clears this EC state",
            selected_charge_type().as_deref().unwrap_or("<unreadable>")
        );
        if announce {
            log::warn!("{msg}");
        } else {
            log::debug!("{msg}");
        }
        return Err(std::io::Error::other(msg));
    }

    // Legacy fallback: machines without charge_types (older models/kernels).
    log::debug!(
        "battery::set_charge_limit_pct — write path: legacy conservation_mode={}",
        if preserve { 1 } else { 0 }
    );
    if let Err(e) = set_conservation_file(preserve) {
        log::debug!("battery::set_charge_limit_pct — conservation_mode write failed: {e}");
        return Err(e);
    }
    // Same verification contract as the charge_types path.
    match conservation_mode() {
        Some(on) if on == preserve => {
            if announce {
                log::info!(
                    "battery::set_charge_limit_pct — verified legacy conservation_mode={} on readback",
                    if preserve { 1 } else { 0 }
                );
            } else {
                log::debug!(
                    "battery::set_charge_limit_pct — verified legacy conservation_mode={} on readback",
                    if preserve { 1 } else { 0 }
                );
            }
            Ok(())
        }
        Some(on) => {
            let msg = format!(
                "Firmware did not accept conservation_mode={} (reads {on}) — charging may be uncapped",
                if preserve { 1 } else { 0 }
            );
            if announce {
                log::warn!("{msg}");
            } else {
                log::debug!("{msg}");
            }
            Err(std::io::Error::other(msg))
        }
        None => {
            if announce {
                log::warn!("conservation_mode unreadable after write — cannot verify charge limit");
            } else {
                log::debug!(
                    "conservation_mode unreadable after repair — cannot verify charge limit"
                );
            }
            Ok(())
        }
    }
}

/// Adopt the machine's current effective limiter state as the desired state,
/// so the watchdog maintains it across daemon restarts. Only seeds when a
/// limiter is actually engaged (booted with conservation on); otherwise the
/// watchdog stays passive until the user explicitly sets a limit.
pub fn seed_desired_from_effective() {
    log::trace!("battery::seed_desired_from_effective()");
    let effective = charge_limit_pct();
    if effective < 100 {
        DESIRED_LIMIT.store(effective, Ordering::Relaxed);
        log::info!(
            "charge limiter watchdog seeded from effective state ({}%)",
            effective
        );
    } else {
        log::debug!(
            "battery::seed_desired_from_effective — limiter not engaged (effective={effective}%), watchdog stays passive"
        );
    }
}

/// Pure predicate: the firmware limiter is engaged AND the battery sits far
/// above its nominal band (~75–80%). This is the documented "EC charged while
/// the laptop was off/asleep" condition — the limiter bit is intact, the EC
/// just ignored it while the OS was gone.
pub fn above_limiter_band(limit_pct: u32, capacity_pct: u32) -> bool {
    let r = limit_pct < 100 && capacity_pct > 85;
    log::trace!(
        "battery::above_limiter_band(limit_pct={limit_pct}, capacity_pct={capacity_pct}) — result={r}"
    );
    r
}

/// Live check of `above_limiter_band` against current sysfs state.
pub fn charged_past_limiter() -> bool {
    log::trace!("battery::charged_past_limiter()");
    let limit = charge_limit_pct();
    let cap = capacity().unwrap_or(0);
    let r = above_limiter_band(limit, cap);
    log::debug!("battery::charged_past_limiter — limit={limit}%, capacity={cap}% → {r}");
    r
}

/// Re-apply the last explicitly requested limit if the EC silently dropped
/// it (documented Gen-10 flakiness; kernel Bug 221065). Returns Ok(true)
/// when a repair was attempted. No-op until an explicit limit was set in
/// this daemon's lifetime.
pub fn reassert_configured_limit() -> std::io::Result<bool> {
    log::trace!("battery::reassert_configured_limit()");
    let want = DESIRED_LIMIT.load(Ordering::Relaxed);
    if want == 0 {
        log::debug!(
            "battery::reassert_configured_limit — no explicit limit requested yet, skipping"
        );
        return Ok(false);
    }
    let preserve = want < 100;
    let engaged = match selected_charge_type() {
        Some(sel) => {
            let e = sel.eq_ignore_ascii_case("Long_Life") == preserve;
            log::debug!(
                "battery::reassert_configured_limit — charge_types selection {sel:?} vs want {want}%: engaged={e}"
            );
            e
        }
        None => match conservation_mode() {
            Some(on) => {
                let e = on == preserve;
                log::debug!(
                    "battery::reassert_configured_limit — conservation_mode={on} vs want {want}%: engaged={e}"
                );
                e
            }
            // Neither interface readable — do not guess, do not spam.
            None => {
                log::debug!(
                    "battery::reassert_configured_limit — neither charge_types nor conservation_mode readable, skipping"
                );
                return Ok(false);
            }
        },
    };
    if engaged {
        log::debug!(
            "battery::reassert_configured_limit — limiter still engaged at {want}%, nothing to repair"
        );
        return Ok(false);
    }
    log::debug!(
        "charge limiter state silently cleared by firmware — re-applying {}%",
        want
    );
    if let Err(e) = set_charge_limit_pct_quiet(want) {
        log::debug!("battery::reassert_configured_limit — re-apply failed: {e}");
        return Err(e);
    }
    log::debug!("battery::reassert_configured_limit — repaired limiter to {want}%");
    Ok(true)
}

pub fn charge_limit_label(pct: u32) -> &'static str {
    let label = match pct {
        // 60 only exists as the legacy conservation_mode fallback on old
        // machines; current Legion firmware's limiter is ~75–80%.
        60 => "Conservation (~55–60%)",
        80 => "Preservation (~75–80%)",
        _ => "Full charge (100%)",
    };
    log::trace!("battery::charge_limit_label(pct={pct}) — result={label}");
    label
}

/// Pure helper: map any requested percentage to the firmware limiter state
/// (< 100 ⇒ preserved, ≥ 100 ⇒ full). The single implementation behind
/// `set_charge_limit_pct`; exported so tests pin the mapping.
pub fn discretize_limit(pct: u32) -> u32 {
    let d = match pct {
        0..=69 => 60,
        70..=89 => 80,
        _ => 100,
    };
    log::trace!("battery::discretize_limit(pct={pct}) — result={d}");
    d
}

/// Pure helper: compute health from two watt-hour readings. The single
/// implementation behind `health_pct()`; exported so tests exercise the
/// same math the sysfs path uses.
pub fn health_from_wh(full: f64, design: f64) -> Option<f64> {
    log::trace!("battery::health_from_wh(full={full}, design={design})");
    if design <= 0.0 {
        log::debug!("battery::health_from_wh — design={design} <= 0, returning None");
        return None;
    }
    let h = (full / design) * 100.0;
    log::debug!("battery::health_from_wh — computed {h:.1}%");
    Some(h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discretize_maps_to_supported_limits() {
        assert_eq!(discretize_limit(0), 60);
        assert_eq!(discretize_limit(59), 60);
        assert_eq!(discretize_limit(69), 60);
        assert_eq!(discretize_limit(70), 80);
        assert_eq!(discretize_limit(89), 80);
        assert_eq!(discretize_limit(90), 100);
        assert_eq!(discretize_limit(100), 100);
        assert_eq!(discretize_limit(999), 100);
    }

    #[test]
    fn label_matches_discretized_value() {
        for pct in [0, 60, 69, 70, 80, 89, 90, 100, 200] {
            let d = discretize_limit(pct);
            let label = charge_limit_label(d);
            assert!(!label.is_empty(), "empty label for {d}");
            // Supported labels only.
            assert!(
                label.contains("60") || label.contains("80") || label.contains("100"),
                "unexpected label {label:?} for {d}"
            );
        }
        assert_eq!(charge_limit_label(60), "Conservation (~55–60%)");
        assert_eq!(charge_limit_label(80), "Preservation (~75–80%)");
        assert_eq!(charge_limit_label(100), "Full charge (100%)");
    }

    #[test]
    fn selection_parser_extracts_bracketed_type() {
        assert_eq!(
            parse_selection("Fast [Standard] Long_Life").as_deref(),
            Some("Standard")
        );
        assert_eq!(parse_selection("[Long_Life]").as_deref(), Some("Long_Life"));
        assert_eq!(parse_selection("Fast Standard Long_Life"), None);
        assert_eq!(parse_selection(""), None);
        // Unterminated bracket must not panic.
        assert_eq!(parse_selection("Fast [Standard"), None);
    }

    #[test]
    fn above_limiter_band_detects_off_charging() {
        // Limiter engaged, battery far above the ~75–80% band → the EC
        // charged while the laptop was off/asleep.
        assert!(above_limiter_band(80, 98));
        assert!(above_limiter_band(60, 86));
        // Inside or below the band → normal.
        assert!(!above_limiter_band(80, 80));
        assert!(!above_limiter_band(80, 85));
        assert!(!above_limiter_band(80, 40));
        // Limiter off → never flagged, whatever the capacity.
        assert!(!above_limiter_band(100, 98));
        assert!(!above_limiter_band(100, 100));
    }

    #[test]
    fn health_from_wh_edge_cases() {
        assert_eq!(health_from_wh(50.0, 100.0).unwrap(), 50.0);
        assert_eq!(health_from_wh(100.0, 100.0).unwrap(), 100.0);
        assert!(health_from_wh(50.0, 0.0).is_none());
        assert!(health_from_wh(50.0, -1.0).is_none());
    }
}
