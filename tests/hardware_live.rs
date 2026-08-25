//! Read-only validation against REAL hardware — no mocks, no writes.
//!
//! These tests only run on the target machine and are skipped by default so
//! plain `cargo test` stays green anywhere:
//!
//! ```bash
//! cargo test --test hardware_live -- --ignored --nocapture
//! ```
//!
//! Contract: every test here must be safe to run on a production laptop —
//! sysfs/hidraw READS only. Never add a write (fans, RGB, charge limit) to
//! this file; write paths stay covered by pure-helper unit tests.

use legion_core::{battery, device, fans, sensors, thermal};

/// Loose plausibility bounds for live sensor values.
const TEMP_MIN_C: f64 = -20.0;
const TEMP_MAX_C: f64 = 125.0;

fn target_hardware_present() -> bool {
    std::path::Path::new("/sys/class/power_supply/BAT0").exists()
        && std::path::Path::new("/sys/class/hwmon").exists()
}

macro_rules! skip_unless_hw {
    () => {
        if !target_hardware_present() {
            eprintln!("skipping: no BAT0 / hwmon on this machine");
            return;
        }
    };
}

#[test]
#[ignore = "read-only live-hardware check"]
fn battery_reads_plausible_values() {
    skip_unless_hw!();
    let pct = battery::capacity().expect("BAT0 capacity readable");
    assert!(pct <= 100, "capacity {pct}% out of range");

    let status = battery::status().unwrap_or_default();
    assert!(!status.is_empty(), "BAT0 status empty");

    // Effective limit must be one of the firmware states we model.
    let limit = battery::charge_limit_pct();
    assert!(
        [60, 80, 100].contains(&limit),
        "charge_limit_pct() returned {limit}, expected 60/80/100"
    );

    if let Some(full) = battery::energy_full_wh() {
        assert!(full > 0.0);
        if let Some(design) = battery::energy_design_wh() {
            assert!(design > 0.0);
            let health = battery::health_pct().expect("health computable from full+design");
            assert!(
                (10.0..=120.0).contains(&health),
                "battery health {health}% implausible"
            );
        }
    }
}

#[test]
#[ignore = "read-only live-hardware check"]
fn fans_enumerate_and_read_live_rpms() {
    skip_unless_hw!();
    let channels = fans::channels();
    assert!(!channels.is_empty(), "no fan channels discovered");

    for f in channels {
        assert!(
            f.min_rpm <= f.max_rpm,
            "fan {}: min {} > max {}",
            f.id,
            f.min_rpm,
            f.max_rpm
        );
        let rpm = fans::read_rpm(f.id).expect("fan RPM readable");
        assert!(rpm <= 20_000, "fan {} reports implausible {rpm} rpm", f.id);
        assert!(!fans::rpm_label(f.id).is_empty());
    }
}

#[test]
#[ignore = "read-only live-hardware check"]
fn sensors_return_plausible_live_temperatures() {
    skip_unless_hw!();
    let s = sensors::read_all();
    assert!(
        (TEMP_MIN_C..=TEMP_MAX_C).contains(&s.cpu_tctl),
        "CPU temp {:.1}°C out of range",
        s.cpu_tctl
    );
    // dGPU may report a sentinel when powered down — only reject garbage.
    assert!(s.dgpu_temp >= TEMP_MIN_C, "dGPU temp implausible");
}

#[test]
#[ignore = "read-only live-hardware check"]
fn thermal_governor_inputs_readable_on_real_cpu() {
    skip_unless_hw!();
    let (tctl, tccd2) = thermal::read_thermal_temps();
    assert!(tctl.is_some() || tccd2.is_some(), "no k10temp temps");
    for t in [tctl, tccd2].into_iter().flatten() {
        let c = t as f64 / 1000.0;
        assert!((0.0..=TEMP_MAX_C).contains(&c), "k10temp {c}°C implausible");
    }
    let cur_max = thermal::read_cur_max().expect("scaling_max_freq readable");
    assert!(
        (400_000..=10_000_000).contains(&cur_max),
        "scaling_max_freq {cur_max} kHz implausible"
    );
}

#[test]
#[ignore = "read-only live-hardware check"]
fn device_identity_is_complete_and_cached() {
    skip_unless_hw!();
    let info = device::detect();
    assert!(!info.model.is_empty(), "DMI model empty");
    assert!(!info.machine_type.is_empty(), "machine type empty");
    assert!(!info.capabilities.fans.is_empty(), "no fan capabilities");
    // Second call must return the cached clone, not re-run detection.
    let again = device::detect();
    assert_eq!(info.machine_type, again.machine_type);
    assert_eq!(
        format!("{:?}", info.capabilities.fans),
        format!("{:?}", again.capabilities.fans)
    );
}
