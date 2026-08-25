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
    health_from_wh(full, design)
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

/// Parse the bracketed active selection out of a charge_types value
/// ("Fast [Standard] Long_Life" → "Standard"). Pure helper; exported for tests.
fn parse_selection(types: &str) -> Option<String> {
    let open = types.find('[')?;
    let close = types[open + 1..].find(']')? + open + 1;
    Some(types[open + 1..close].to_string())
}

fn selected_charge_type() -> Option<String> {
    parse_selection(&charge_types()?)
}

/// Legacy boolean API — maps to ~60% conservation when on, Standard when off.
pub fn set_conservation(on: bool) -> std::io::Result<()> {
    if on {
        set_charge_limit_pct(60)
    } else {
        set_charge_limit_pct(100)
    }
}

/// Current effective charge limit percentage: 80 when the firmware limiter is
/// engaged, else 100. Legacy `conservation_mode`-only machines (no
/// `charge_types` attr) report 60 while conservation is on.
pub fn charge_limit_pct() -> u32 {
    if let Some(sel) = selected_charge_type() {
        if sel.eq_ignore_ascii_case("Long_Life") {
            return 80;
        }
        if sel.eq_ignore_ascii_case("Standard") || sel.eq_ignore_ascii_case("Fast") {
            return 100;
        }
    }
    if let Some(path) = conservation_path() {
        if read(&path.to_string_lossy()).as_deref() == Some("1") {
            return 60;
        }
    }
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
    let pct = discretize_limit(pct);
    let preserve = pct < 100;
    let want = if preserve { "Long_Life" } else { "Standard" };
    log::info!(
        "charge limit → {} ({want})",
        if preserve { "preserved" } else { "full" }
    );

    DESIRED_LIMIT.store(pct, Ordering::Relaxed);

    // Preferred path: standardized charge_types switch.
    if charge_types().is_some() {
        set_charge_type(want)?;
        // The EC may take a few hundred ms to reflect the new selection
        // (and kernel Bug 221065 can garble the first readback after AC
        // events) — poll the readback instead of trusting a single sample.
        for _ in 0..5 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            match selected_charge_type() {
                Some(t) if t.eq_ignore_ascii_case(want) => return Ok(()),
                _ => {}
            }
        }
        let msg = format!(
            "Firmware did not accept charge_types={want} (reads {:?}) — charging may be uncapped. AC plug/unplug or reboot usually clears this EC state",
            selected_charge_type().as_deref().unwrap_or("<unreadable>")
        );
        log::warn!("{msg}");
        return Err(std::io::Error::other(msg));
    }

    // Legacy fallback: machines without charge_types (older models/kernels).
    set_conservation_file(preserve)?;
    // Same verification contract as the charge_types path.
    match conservation_mode() {
        Some(on) if on == preserve => Ok(()),
        Some(on) => {
            let msg = format!(
                "Firmware did not accept conservation_mode={} (reads {on}) — charging may be uncapped",
                if preserve { 1 } else { 0 }
            );
            log::warn!("{msg}");
            Err(std::io::Error::other(msg))
        }
        None => {
            log::warn!("conservation_mode unreadable after write — cannot verify charge limit");
            Ok(())
        }
    }
}

/// Adopt the machine's current effective limiter state as the desired state,
/// so the watchdog maintains it across daemon restarts. Only seeds when a
/// limiter is actually engaged (booted with conservation on); otherwise the
/// watchdog stays passive until the user explicitly sets a limit.
pub fn seed_desired_from_effective() {
    let effective = charge_limit_pct();
    if effective < 100 {
        DESIRED_LIMIT.store(effective, Ordering::Relaxed);
        log::info!(
            "charge limiter watchdog seeded from effective state ({}%)",
            effective
        );
    }
}

/// Re-apply the last explicitly requested limit if the EC silently dropped
/// it (documented Gen-10 flakiness; kernel Bug 221065). Returns Ok(true)
/// when a repair was attempted. No-op until an explicit limit was set in
/// this daemon's lifetime.
pub fn reassert_configured_limit() -> std::io::Result<bool> {
    let want = DESIRED_LIMIT.load(Ordering::Relaxed);
    if want == 0 {
        return Ok(false);
    }
    let preserve = want < 100;
    let engaged = match selected_charge_type() {
        Some(sel) => sel.eq_ignore_ascii_case("Long_Life") == preserve,
        None => match conservation_mode() {
            Some(on) => on == preserve,
            // Neither interface readable — do not guess, do not spam.
            None => return Ok(false),
        },
    };
    if engaged {
        return Ok(false);
    }
    log::warn!(
        "charge limiter state silently cleared by firmware — re-applying {}%",
        want
    );
    set_charge_limit_pct(want)?;
    Ok(true)
}

pub fn charge_limit_label(pct: u32) -> &'static str {
    match pct {
        // 60 only exists as the legacy conservation_mode fallback on old
        // machines; current Legion firmware's limiter is ~75–80%.
        60 => "Conservation (~55–60%)",
        80 => "Preservation (~75–80%)",
        _ => "Full charge (100%)",
    }
}

/// Pure helper: map any requested percentage to the firmware limiter state
/// (< 100 ⇒ preserved, ≥ 100 ⇒ full). The single implementation behind
/// `set_charge_limit_pct`; exported so tests pin the mapping.
pub fn discretize_limit(pct: u32) -> u32 {
    match pct {
        0..=69 => 60,
        70..=89 => 80,
        _ => 100,
    }
}

/// Pure helper: compute health from two watt-hour readings. The single
/// implementation behind `health_pct()`; exported so tests exercise the
/// same math the sysfs path uses.
pub fn health_from_wh(full: f64, design: f64) -> Option<f64> {
    if design <= 0.0 {
        return None;
    }
    Some((full / design) * 100.0)
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
    fn health_from_wh_edge_cases() {
        assert_eq!(health_from_wh(50.0, 100.0).unwrap(), 50.0);
        assert_eq!(health_from_wh(100.0, 100.0).unwrap(), 100.0);
        assert!(health_from_wh(50.0, 0.0).is_none());
        assert!(health_from_wh(50.0, -1.0).is_none());
    }
}
