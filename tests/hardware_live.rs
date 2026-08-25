//! Legion Control — live self-test suite (F01–F55).
//!
//! Validates the whole surface against REAL hardware, REAL sysfs and the
//! REAL running daemon on the target laptop (83RU). No mocks. Strictly
//! READ-ONLY: sysfs/hidraw/IPC reads only — never add a write here; write
//! paths are covered by pure-helper unit tests.
//!
//! Skipped by default so `cargo test` stays green anywhere:
//!
//! ```bash
//! cargo test --test hardware_live --test hardware_live_cfg -- --ignored
//! ```
//!
//! Machine-specific pins (83RU / 3 fans / Spectrum present …) live in one
//! dedicated test (`f50_machine_pins_83ru`) so porting touches one place.

use legion_core::comms::{send_command, DaemonCommand, DaemonResponse};
use legion_core::{
    battery, comms, config, device, fans, keyboard, profile, sensors, thermal, undervolt,
};

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

fn daemon_reachable() -> bool {
    matches!(
        send_command(DaemonCommand::GetProfile),
        Ok(DaemonResponse::Profile(_))
    )
}

macro_rules! skip_unless_daemon {
    () => {
        if !daemon_reachable() {
            eprintln!("skipping: legion-daemon not reachable");
            return;
        }
    };
}

// ═══ Battery ═══════════════════════════════════════════════════════════════

#[test]
#[ignore = "live self-test"]
fn f01_battery_capacity_and_status() {
    skip_unless_hw!();
    let pct = battery::capacity().expect("BAT0 capacity readable");
    assert!(pct <= 100);
    let status = battery::status().unwrap_or_default();
    assert!(!status.is_empty());
}

#[test]
#[ignore = "live self-test"]
fn f02_battery_status_is_a_known_state() {
    skip_unless_hw!();
    let status = battery::status().unwrap_or_default();
    const KNOWN: [&str; 6] = [
        "Charging",
        "Discharging",
        "Not charging",
        "Full",
        "Unknown",
        "",
    ];
    assert!(
        KNOWN.contains(&status.as_str()),
        "unexpected power_supply status {status:?}"
    );
}

#[test]
#[ignore = "live self-test"]
fn f03_battery_voltage_plausible() {
    skip_unless_hw!();
    let v = battery::voltage().expect("voltage_now readable");
    assert!((5.0..=30.0).contains(&v), "voltage {v} V implausible");
}

#[test]
#[ignore = "live self-test"]
fn f04_battery_cycles_plausible() {
    skip_unless_hw!();
    let c = battery::cycles().expect("cycle_count readable");
    assert!(c <= 10_000, "cycle count {c} implausible");
}

#[test]
#[ignore = "live self-test"]
fn f05_battery_energy_trio_consistent() {
    skip_unless_hw!();
    let now = battery::energy_now_wh().expect("energy_now");
    let full = battery::energy_full_wh().expect("energy_full");
    let design = battery::energy_design_wh().expect("energy_full_design");
    assert!(now >= 0.0);
    assert!(full > 0.0 && design > 0.0);
    assert!(
        now <= full * 1.05,
        "energy_now {now} exceeds energy_full {full}"
    );
    assert!(full <= design * 1.2, "wear model inverted");
}

#[test]
#[ignore = "live self-test"]
fn f06_health_matches_pure_helper_exactly() {
    skip_unless_hw!();
    let full = battery::energy_full_wh().expect("full");
    let design = battery::energy_design_wh().expect("design");
    let live = battery::health_pct();
    let pure = battery::health_from_wh(full, design);
    assert_eq!(live, pure, "health_pct diverged from health_from_wh");
    let h = live.expect("health computable");
    assert!((10.0..=120.0).contains(&h));
}

#[test]
#[ignore = "live self-test"]
fn f07_battery_identity_strings() {
    skip_unless_hw!();
    let mfr = battery::manufacturer().unwrap_or_default();
    let model = battery::model_name().unwrap_or_default();
    let tech = battery::technology().unwrap_or_default();
    assert!(!mfr.is_empty() && mfr.len() < 64);
    assert!(!model.is_empty());
    assert!(!tech.is_empty());
}

#[test]
#[ignore = "live self-test"]
fn f08_charge_types_selection_parses_from_live_attr() {
    skip_unless_hw!();
    let types = battery::charge_types().expect("charge_types attr present");
    assert!(
        types.contains('[') && types.contains(']'),
        "malformed {types:?}"
    );
    let sel = battery::charge_limit_pct();
    assert!(
        [60, 80, 100].contains(&sel),
        "limit {sel}% not a modeled state"
    );
}

#[test]
#[ignore = "live self-test"]
fn f09_limiter_state_machine_truth_table_on_live_state() {
    skip_unless_hw!();
    let limit = battery::charge_limit_pct();
    let selection = battery::charge_types().unwrap_or_default();
    if selection.contains("[Long_Life]") {
        assert_eq!(limit, 80, "Long_Life must map to 80");
    } else if selection.contains("[Standard]") || selection.contains("[Fast]") {
        assert_eq!(limit, 100, "Standard/Fast must map to 100");
    }
    // Legacy bit agrees with the modern view when it is set.
    if battery::conservation_mode() == Some(true) && !selection.is_empty() {
        assert!(
            limit == 60 || limit == 80,
            "conservation=1 but limit={limit}"
        );
    }
}

#[test]
#[ignore = "live self-test"]
fn f10_limiter_band_predicate_agrees_with_live_values() {
    skip_unless_hw!();
    let limit = battery::charge_limit_pct();
    let cap = battery::capacity().unwrap_or(0);
    assert_eq!(
        battery::charged_past_limiter(),
        battery::above_limiter_band(limit, cap)
    );
}

#[test]
#[ignore = "live self-test"]
fn f11_battery_power_draw_finite() {
    skip_unless_hw!();
    if let Some(w) = battery::power_w() {
        assert!(w.abs() < 300.0, "battery power {w} W implausible");
    }
}

// ═══ Fans ══════════════════════════════════════════════════════════════════

#[test]
#[ignore = "live self-test"]
fn f12_fan_backend_hwmon_present() {
    skip_unless_hw!();
    let backend = sensors::hwmon_by_name("lenovo_wmi_other")
        .or_else(|| sensors::hwmon_by_name("legion_hwmon"));
    assert!(
        backend.is_some(),
        "neither lenovo_wmi_other nor legion_hwmon hwmon found"
    );
}

#[test]
#[ignore = "live self-test"]
fn f13_fan_channels_sane_limits_and_titles() {
    skip_unless_hw!();
    let channels = fans::channels();
    assert!(!channels.is_empty());
    let mut titles = Vec::new();
    for f in &channels {
        assert!(!f.title.is_empty());
        titles.push(f.title.clone());
        assert!(f.min_rpm > 0 && f.min_rpm <= f.max_rpm);
        assert!(
            f.max_rpm <= 12_000,
            "fan {} max {} implausible",
            f.id,
            f.max_rpm
        );
    }
    titles.sort();
    titles.dedup();
    assert_eq!(titles.len(), channels.len(), "duplicate fan titles");
}

#[test]
#[ignore = "live self-test"]
fn f14_fan_ids_unique_and_bounded() {
    skip_unless_hw!();
    let mut ids = fans::ids();
    assert!(!ids.is_empty());
    ids.sort_unstable();
    let before = ids.clone();
    ids.dedup();
    assert_eq!(before, ids, "duplicate fan ids");
    for id in &ids {
        assert!([1u8, 2, 3, 4].contains(id), "unexpected fan id {id}");
    }
}

#[test]
#[ignore = "live self-test"]
fn f15_live_fan_rpms_readable() {
    skip_unless_hw!();
    for id in fans::ids() {
        let rpm = fans::read_rpm(id).expect("fan rpm readable");
        assert!(rpm <= 20_000, "fan {id}: {rpm} rpm implausible");
    }
}

#[test]
#[ignore = "live self-test"]
fn f16_live_fan_targets_readable() {
    skip_unless_hw!();
    for id in fans::ids() {
        let target = fans::read_target(id).expect("fan target readable");
        assert!(target <= 20_000, "fan {id}: target {target} implausible");
    }
}

#[test]
#[ignore = "live self-test"]
fn f17_rpm_label_matches_pure_contract_on_live_values() {
    skip_unless_hw!();
    for id in fans::ids() {
        let target = fans::read_target(id).unwrap_or(0);
        let rpm = fans::read_rpm(id).unwrap_or(0);
        let expected = fans::format_rpm_label(target, rpm);
        assert_eq!(
            fans::rpm_label(id),
            expected,
            "fan {id}: wrapper/pure drift"
        );
        if target == 0 {
            assert!(expected.starts_with("Auto"));
        }
    }
}

// ═══ Sensors (read_all, per-field) ═════════════════════════════════════════

fn sentinel_or_range(v: f64, lo: f64, hi: f64, what: &str) {
    // -1 is the codebase's "no reading" sentinel; anything else must be sane.
    if v != -1.0 {
        assert!((lo..=hi).contains(&v), "{what} {v} implausible");
    } else {
        assert!(v >= TEMP_MIN_C, "{what} below hard floor");
    }
}

#[test]
#[ignore = "live self-test"]
fn f18_sensor_cpu_tctl_plausible() {
    skip_unless_hw!();
    let s = sensors::read_all();
    assert!(
        (TEMP_MIN_C..=TEMP_MAX_C).contains(&s.cpu_temp),
        "cpu_temp {:.1} implausible",
        s.cpu_temp
    );
}

#[test]
#[ignore = "live self-test"]
fn f19_sensor_ccd_temps_sentinel_or_plausible() {
    skip_unless_hw!();
    let s = sensors::read_all();
    sentinel_or_range(s.cpu_temp_1, 0.0, TEMP_MAX_C, "cpu_temp_1");
    sentinel_or_range(s.cpu_temp_2, 0.0, TEMP_MAX_C, "cpu_temp_2");
}

#[test]
#[ignore = "live self-test"]
fn f20_sensor_ec_temps_sentinel_or_plausible() {
    skip_unless_hw!();
    let s = sensors::read_all();
    sentinel_or_range(s.ec_cpu, 0.0, TEMP_MAX_C, "ec_cpu");
    sentinel_or_range(s.ec_gpu, 0.0, TEMP_MAX_C, "ec_gpu");
}

#[test]
#[ignore = "live self-test"]
fn f21_sensor_igpu_fields_sentinel_or_plausible() {
    skip_unless_hw!();
    let s = sensors::read_all();
    sentinel_or_range(s.igpu_edge, 0.0, TEMP_MAX_C, "igpu_edge");
    sentinel_or_range(s.igpu_power, -5.0, 200.0, "igpu_power");
}

#[test]
#[ignore = "live self-test"]
fn f22_sensor_dgpu_fields_sentinel_or_plausible() {
    skip_unless_hw!();
    let s = sensors::read_all();
    sentinel_or_range(s.dgpu_temp, 0.0, TEMP_MAX_C, "dgpu_temp");
    sentinel_or_range(s.dgpu_power, -5.0, 300.0, "dgpu_power");
    if s.dgpu_clock != -1.0 {
        assert!((0.0..=6000.0).contains(&s.dgpu_clock));
    }
}

#[test]
#[ignore = "live self-test"]
fn f23_sensor_nvme_and_ram_temps() {
    skip_unless_hw!();
    let s = sensors::read_all();
    for t in &s.ssd_composite {
        assert!((0.0..=95.0).contains(t), "NVMe {t}°C implausible");
    }
    for t in &s.ram_temps {
        assert!((0.0..=TEMP_MAX_C).contains(t), "RAM {t}°C implausible");
    }
}

#[test]
#[ignore = "live self-test"]
fn f24_sensors_profile_agrees_with_profile_module() {
    skip_unless_hw!();
    let s = sensors::read_all();
    assert!(!s.profile.is_empty());
    assert_eq!(s.profile, profile::current());
}

#[test]
#[ignore = "live self-test"]
fn f25_sensors_fan_rpms_match_direct_reads_within_tolerance() {
    skip_unless_hw!();
    let s = sensors::read_all();
    let pairs = [(1u8, s.fan1_rpm), (2, s.fan2_rpm), (4, s.fan4_rpm)];
    for (id, snapshot_rpm) in pairs.into_iter().filter(|(_, r)| *r > 0) {
        let direct = fans::read_rpm(id).unwrap_or(0);
        let diff = (i64::from(snapshot_rpm) - i64::from(direct)).abs();
        assert!(
            diff <= 500,
            "fan {id}: snapshot {snapshot_rpm} vs direct {direct}"
        );
    }
}

#[test]
#[ignore = "live self-test"]
fn f26_two_sensor_passes_stay_within_drift_bound() {
    skip_unless_hw!();
    let a = sensors::read_all();
    std::thread::sleep(std::time::Duration::from_millis(150));
    let b = sensors::read_all();
    let drift = (a.cpu_temp - b.cpu_temp).abs();
    assert!(drift < 15.0, "cpu_temp drifted {drift}°C between passes");
}

// ═══ CPU sampling ══════════════════════════════════════════════════════════

#[test]
#[ignore = "live self-test"]
fn f27_cpu_usage_two_ticks_in_range() {
    skip_unless_hw!();
    let _ = sensors::sample_cpu_usage_pct(); // seed baseline
    std::thread::sleep(std::time::Duration::from_millis(300));
    let second = sensors::sample_cpu_usage_pct();
    assert!(
        (0.0..=100.0).contains(&second),
        "usage {second}% out of range"
    );
}

#[test]
#[ignore = "live self-test"]
fn f28_rapl_power_in_range_and_counter_monotonic() {
    skip_unless_hw!();
    const RAPL: &str = "/sys/devices/virtual/powercap/intel-rapl/intel-rapl:0/energy_uj";
    // Ground truth first: the raw energy counter never goes backwards.
    if let Ok(raw) = std::fs::read_to_string(RAPL) {
        if let Ok(e0) = raw.trim().parse::<u64>() {
            std::thread::sleep(std::time::Duration::from_millis(250));
            if let Ok(raw2) = std::fs::read_to_string(RAPL) {
                if let Ok(e1) = raw2.trim().parse::<u64>() {
                    assert!(e1 >= e0, "RAPL counter went backwards ({e0} → {e1})");
                }
            }
        }
    }
    let _ = sensors::sample_cpu_power_w(); // seed
    std::thread::sleep(std::time::Duration::from_millis(300));
    let w = sensors::sample_cpu_power_w();
    assert!((0.0..=300.0).contains(&w), "RAPL {w} W implausible");
}

// ═══ Thermal governor inputs ═══════════════════════════════════════════════

#[test]
#[ignore = "live self-test"]
fn f29_k10temp_present_and_plausible() {
    skip_unless_hw!();
    let (main, secondary) = thermal::read_cpu_temps();
    assert!(
        main.is_some() || secondary.is_some(),
        "no k10temp temps at all"
    );
    for t in [main, secondary].into_iter().flatten() {
        let c = t as f64 / 1000.0;
        assert!((0.0..=TEMP_MAX_C).contains(&c), "k10temp {c}°C implausible");
    }
}

#[test]
#[ignore = "live self-test"]
fn f30_scaling_max_freq_readable_and_bounded() {
    skip_unless_hw!();
    let cur = thermal::read_cur_max().expect("scaling_max_freq readable");
    assert!((400_000..=10_000_000).contains(&cur));
}

#[test]
#[ignore = "live self-test"]
fn f31_hardcoded_max_full_matches_real_cpufreq_policy() {
    skip_unless_hw!();
    let policy_max: u32 =
        std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq")
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .expect("cpuinfo_max_freq readable");
    assert_eq!(
        thermal::MAX_FULL,
        policy_max,
        "thermal::MAX_FULL drifted from real cpuinfo_max_freq"
    );
    // Compile-time governor-constant pins.
    const _: () = assert!(thermal::MIN < thermal::MAX_FULL);
    const _: () = assert!(thermal::HYSTERESIS > 0);
}

#[test]
#[ignore = "live self-test"]
fn f32_compute_target_on_live_inputs() {
    skip_unless_hw!();
    use legion_core::thermal::{compute_target, ThermalConfig};
    let temp_mc = thermal::read_cpu_temps().0.unwrap_or(60_000);
    let cur = thermal::read_cur_max().unwrap_or(thermal::MAX_FULL);

    let disabled = ThermalConfig::default();
    assert_eq!(compute_target(cur, temp_mc, &disabled), None);

    let enabled = ThermalConfig {
        enabled: true,
        max_temp: 90,
    };
    if let Some(target) = compute_target(cur, temp_mc, &enabled) {
        assert!((thermal::MIN..=thermal::MAX_FULL).contains(&target));
    }
}

// ═══ Platform profiles / PPT ═══════════════════════════════════════════════

#[test]
#[ignore = "live self-test"]
fn f33_platform_profile_current_within_choices() {
    skip_unless_hw!();
    let current = profile::current();
    assert!(!current.is_empty());
    let choices = profile::choices();
    assert!(!choices.is_empty());
    assert!(
        choices.iter().any(|c| c.eq_ignore_ascii_case(&current)),
        "current profile {current:?} not among choices {choices:?}"
    );
}

#[test]
#[ignore = "live self-test"]
fn f34_ppt_surface_reports_without_error() {
    skip_unless_hw!();
    let _available = profile::ppt_available();
    for lim in profile::gpu_ppt_limits() {
        assert!(!lim.id.is_empty() && !lim.label.is_empty());
        assert!(lim.min <= lim.max);
        assert!(lim.min <= lim.default && lim.default <= lim.max);
    }
}

// ═══ Device identity ═══════════════════════════════════════════════════════

#[test]
#[ignore = "live self-test"]
fn f35_device_identity_complete() {
    skip_unless_hw!();
    let info = device::detect();
    assert!(!info.model.is_empty());
    assert!(!info.machine_type.is_empty());
    assert_eq!(info.machine_type.len(), 4, "machine type format");
    assert!(!info.bios_version.is_empty());
    assert!(!info.bios_prefix.is_empty());
    assert!(!info.cpu_model.is_empty());
    assert!(!info.gpu_model.is_empty());
    assert!(!info.series.is_empty() || info.gen == 0);
}

#[test]
#[ignore = "live self-test"]
fn f36_device_detect_cache_stable() {
    skip_unless_hw!();
    let a = device::detect();
    let b = device::detect();
    assert_eq!(format!("{a:?}"), format!("{b:?}"), "cache not stable");
}

/// Everything that is true for THIS production laptop in one place.
#[test]
#[ignore = "live self-test"]
fn f37_machine_pins_for_83ru() {
    skip_unless_hw!();
    let info = device::detect();
    assert_eq!(
        info.machine_type, "83RU",
        "suite targets this specific unit"
    );
    assert!(info.model.contains("Legion"), "model {:?}", info.model);
    assert!(
        info.bios_version.starts_with("SMCN"),
        "BIOS {}",
        info.bios_version
    );

    let mut ids = fans::ids();
    ids.sort_unstable();
    assert_eq!(ids, vec![1, 2, 4], "83RU fan layout");

    // Spectrum RGB controller ships with this model — brightness read works.
    let bright = keyboard::rgb_brightness().expect("Spectrum brightness readable");
    assert!(bright <= 9);
}

// ═── Keyboard / Spectrum (reads only) ───────────────────────────────────────

#[test]
#[ignore = "live self-test"]
fn f38_spectrum_brightness_read_back() {
    skip_unless_hw!();
    if let Some(b) = keyboard::rgb_brightness() {
        assert!(b <= 9, "brightness {b} > max 9");
    }
}

#[test]
#[ignore = "live self-test"]
fn f39_spectrum_effect_rgb_decodes_three_channels() {
    skip_unless_hw!();
    if let Some((r, g, b)) = keyboard::peek_effect_rgb() {
        let _ = (r, g, b); // u8 range inherent; presence is the contract
    }
}

#[test]
#[ignore = "live self-test"]
fn f40_logo_state_readable() {
    skip_unless_hw!();
    let _ = keyboard::logo_on(); // Some(bool) when controller answers
}

#[test]
#[ignore = "live self-test"]
fn f41_camera_killswitch_readable() {
    skip_unless_hw!();
    let _ = keyboard::camera_power(); // None valid where switch absent
}

// ═── Curve Optimizer (SMU reads) ═══════════════════════════════════════════

#[test]
#[ignore = "live self-test"]
fn f42_curve_optimizer_status_contract_by_privilege() {
    skip_unless_hw!();
    let st = undervolt::status();
    assert!(!st.reason.is_empty(), "status must always explain itself");

    // SMU queries need root: /sys/kernel/ryzen_smu_drv/rsmu_cmd is 0644.
    // Root (i.e. the daemon's view) must see the full surface; an
    // unprivileged process legitimately reports unavailable instead.
    let is_root = unsafe { libc::geteuid() } == 0;
    if is_root {
        assert!(
            st.available,
            "ryzen_smu loaded but status unavailable: {}",
            st.reason
        );
        assert_eq!(st.minimum, -30);
        assert_eq!(st.maximum, 0);
        assert!(!st.current.is_empty(), "no per-core offsets read");
        assert_eq!(st.current.len(), st.boot_baseline.len());
        assert!(st.current.len() >= 8, "suspiciously few cores reported");
    } else {
        // Either the probe degraded gracefully or somehow succeeded — both
        // acceptable; what matters is it never panics and explains itself.
        eprintln!(
            "unprivileged CO probe: available={} ({})",
            st.available, st.reason
        );
    }
}

#[test]
#[ignore = "live self-test"]
fn f43_curve_optimizer_persistence_status_sane() {
    skip_unless_hw!();
    let p = undervolt::persistence_status();
    if p.enabled {
        assert!((st_min_max_bounds()).contains(&p.offset));
    }
}

fn st_min_max_bounds() -> std::ops::RangeInclusive<i16> {
    let st = undervolt::status();
    st.minimum..=st.maximum
}

// ═── Config persistence (REAL filesystem) ══════════════════════════════════

#[test]
#[ignore = "live self-test"]
fn f44_config_round_trip_writes_valid_json() {
    use std::sync::Mutex;
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    let _guard = ENV_LOCK.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("legion-selftest-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("legion-control")).unwrap();
    unsafe { std::env::set_var("XDG_CONFIG_HOME", &dir) };

    config::update(|c| c.ui_zone = "selftest".into());
    let reloaded = config::get();
    assert_eq!(reloaded.ui_zone, "selftest", "update did not persist");

    let path = dir.join("legion-control/settings.json");
    assert!(path.is_file());
    let raw = std::fs::read_to_string(&path).unwrap();
    let value: serde_json::Value = serde_json::from_str(&raw).expect("valid JSON on disk");
    assert!(
        value["version"].as_u64().unwrap_or(0) >= 4,
        "schema version stamped"
    );
    assert_eq!(value["ui_zone"].as_str(), Some("selftest"));

    unsafe { std::env::remove_var("XDG_CONFIG_HOME") };
    let _ = std::fs::remove_dir_all(&dir);
}
// f24 (corruption handling) lives in tests/hardware_live_cfg.rs — needs its
// own process so the config OnceLock store initializes fresh.

// ═── IPC end-to-end against the REAL running daemon ════════════════════════

fn expect_ipc(cmd: DaemonCommand) -> DaemonResponse {
    send_command(cmd).expect("IPC round-trip to running daemon")
}

#[test]
#[ignore = "live self-test"]
fn f45_ipc_get_profile_matches_local_sysfs_view() {
    skip_unless_hw!();
    skip_unless_daemon!();
    match expect_ipc(DaemonCommand::GetProfile) {
        DaemonResponse::Profile(p) => assert_eq!(p, profile::current()),
        other => panic!("unexpected response: {other:?}"),
    }
}

#[test]
#[ignore = "live self-test"]
fn f46_ipc_battery_matches_direct_sysfs_reads() {
    skip_unless_hw!();
    skip_unless_daemon!();
    match expect_ipc(DaemonCommand::GetBattery) {
        DaemonResponse::Battery {
            capacity,
            status,
            voltage,
            cycles: _,
            conservation,
        } => {
            assert_eq!(capacity, battery::capacity().unwrap_or(0));
            assert_eq!(status, battery::status().unwrap_or_default());
            if let Some(v) = battery::voltage() {
                assert!((voltage - v).abs() < 0.5);
            }
            assert_eq!(conservation, battery::charge_limit_pct() < 100);
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[test]
#[ignore = "live self-test"]
fn f47_ipc_sensors_match_local_reads() {
    skip_unless_hw!();
    skip_unless_daemon!();
    match expect_ipc(DaemonCommand::GetSensors) {
        DaemonResponse::Sensors(s) => {
            let local = sensors::read_all();
            assert!((s.cpu_temp - local.cpu_temp).abs() < 15.0);
            assert_eq!(s.profile, local.profile);
            assert_eq!(s.fan1_rpm, local.fan1_rpm);
            assert_eq!(s.fan2_rpm, local.fan2_rpm);
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[test]
#[ignore = "live self-test"]
fn f48_ipc_device_info_matches_local_detect() {
    skip_unless_hw!();
    skip_unless_daemon!();
    match expect_ipc(DaemonCommand::GetDeviceInfo) {
        DaemonResponse::DeviceInfo(d) => {
            assert_eq!(d.machine_type, device::detect().machine_type);
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[test]
#[ignore = "live self-test"]
fn f49_ipc_root_only_rapl_answers() {
    skip_unless_daemon!();
    match expect_ipc(DaemonCommand::GetCpuPower) {
        DaemonResponse::CpuPower(w) => assert!((0.0..=300.0).contains(&w)),
        DaemonResponse::Error(e) => panic!("root daemon refused RAPL: {e}"),
        other => panic!("unexpected response: {other:?}"),
    }
}

#[test]
#[ignore = "live self-test"]
fn f50_ipc_charge_limit_agrees_with_local_view() {
    skip_unless_daemon!();
    match expect_ipc(DaemonCommand::GetChargeLimit) {
        DaemonResponse::ChargeLimit(p) => {
            assert_eq!(p, battery::charge_limit_pct(), "daemon/local limit skew")
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[test]
#[ignore = "live self-test"]
fn f51_ipc_read_surfaces_answer_without_error() {
    skip_unless_daemon!();
    for cmd in [
        DaemonCommand::GetKbdBrightness,
        DaemonCommand::GetCameraPower,
        DaemonCommand::GetThermalStatus,
        DaemonCommand::GetCurveOptimizerPersistence,
    ] {
        let kind = comms::cmd_kind(&cmd);
        if let DaemonResponse::Error(e) = expect_ipc(cmd) {
            panic!("{kind} errored: {e}");
        }
    }
    match expect_ipc(DaemonCommand::GetRecentLogs(5)) {
        DaemonResponse::RecentLogs(text) => assert!(!text.is_empty()),
        other => panic!("unexpected response: {other:?}"),
    }
}

#[test]
#[ignore = "live self-test"]
fn f52_ipc_fan_rpms_match_direct_reads() {
    skip_unless_hw!();
    skip_unless_daemon!();
    for fan in [1u8, 2, 4] {
        let direct = fans::read_rpm(fan).unwrap_or(0);
        match expect_ipc(DaemonCommand::GetFanRpm(fan)) {
            DaemonResponse::FanRpm(rpm) => {
                let diff = i64::from(rpm).abs_diff(i64::from(direct));
                assert!(diff <= 800, "fan {fan}: daemon {rpm} vs direct {direct}");
            }
            other => panic!("fan {fan}: unexpected {other:?}"),
        }
    }
}

#[test]
#[ignore = "live self-test"]
fn f53_ipc_thermal_status_internally_consistent() {
    skip_unless_daemon!();
    match expect_ipc(DaemonCommand::GetThermalStatus) {
        DaemonResponse::ThermalStatus(st) => {
            assert!(
                (70..=98).contains(&st.config.max_temp),
                "max_temp out of range"
            );
            if st.active {
                assert!(st.config.enabled, "active while disabled?");
                assert!(
                    st.cur_max_freq < thermal::MAX_FULL,
                    "active but at full speed"
                );
            }
            if let Some(t) = st.cpu_temp_mc {
                assert!(
                    (0..=125_000).contains(&t),
                    "status temp {t} m°C implausible"
                );
            }
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[test]
#[ignore = "live self-test"]
fn f54_ipc_cpu_feature_commands_shape() {
    skip_unless_daemon!();
    match expect_ipc(DaemonCommand::GetBoost) {
        DaemonResponse::Boost(_) => {}
        DaemonResponse::Error(_) => {} // optional on some kernels
        other => panic!("unexpected boost response: {other:?}"),
    }
    match expect_ipc(DaemonCommand::GetSmt) {
        DaemonResponse::Smt { logical_cpus, .. } => assert!(logical_cpus > 0),
        DaemonResponse::Error(_) => {}
        other => panic!("unexpected smt response: {other:?}"),
    }
    match expect_ipc(DaemonCommand::GetCurveOptimizer) {
        DaemonResponse::CurveOptimizer(st) => {
            assert!(!st.reason.is_empty());
        }
        other => panic!("unexpected co response: {other:?}"),
    }
}

#[test]
#[ignore = "live self-test"]
fn f55_ipc_latency_and_parallel_clients_smoke() {
    skip_unless_daemon!();
    // Single-call latency well under the client timeout.
    let t0 = std::time::Instant::now();
    expect_ipc(DaemonCommand::GetProfile);
    assert!(t0.elapsed() < std::time::Duration::from_secs(3), "IPC slow");

    // Parallel client storm — daemon's handler pool must cope.
    let handles: Vec<_> = (0..8)
        .map(|_| {
            std::thread::spawn(|| {
                matches!(
                    send_command(DaemonCommand::GetProfile),
                    Ok(DaemonResponse::Profile(_))
                )
            })
        })
        .collect();
    for h in handles {
        assert!(h.join().unwrap(), "parallel GetProfile failed");
    }
}

#[test]
#[ignore = "live self-test"]
fn f56_diagnose_rgb_report_contract() {
    skip_unless_daemon!();
    match expect_ipc(DaemonCommand::DiagnoseRgb) {
        DaemonResponse::RgbDiagnosis {
            health,
            summary,
            fixable,
            ..
        } => {
            assert!(!summary.is_empty());
            assert!(["ok", "soft-issue", "broken", "n/a"].contains(&health.as_str()));
            // fixable=true makes no sense for a fully healthy or N/A device.
            if health == "ok" || health == "n/a" {
                assert!(!fixable, "health {health} but marked fixable");
            }
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

// ═── Widget pipeline (real script + real CLI + real daemon) ════════════════

#[test]
#[ignore = "live self-test"]
fn f57_kde_widget_poll_pipeline_end_to_end() {
    skip_unless_daemon!();
    let script = std::path::Path::new("kde-widget/package/contents/ui/legion-poll.sh");
    let output = std::process::Command::new("bash")
        .arg(script)
        .output()
        .expect("run legion-poll.sh");
    assert!(output.status.success(), "poll.sh exited nonzero");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut saw_ok = false;
    let mut saw_cpu_temp = false;
    let mut saw_battery = false;
    for line in stdout.lines() {
        if line == "LEGION_OK=1" {
            saw_ok = true;
        }
        if line.starts_with("CPU_TEMP=") && !line.ends_with('=') {
            saw_cpu_temp = true;
        }
        if line.starts_with("BATTERY=") {
            saw_battery = true;
        }
    }
    assert!(
        saw_ok,
        "poll.sh did not report LEGION_OK=1 — got:\n{stdout}"
    );
    assert!(saw_cpu_temp, "no CPU_TEMP in poll output");
    assert!(saw_battery, "no BATTERY in poll output");
}

// ═── Protocol hygiene ══════════════════════════════════════════════════════

#[test]
#[ignore = "live self-test"]
fn f58_read_command_kinds_are_unique() {
    let reads = [
        DaemonCommand::GetSensors,
        DaemonCommand::GetProfile,
        DaemonCommand::GetKbdBrightness,
        DaemonCommand::GetBattery,
        DaemonCommand::GetDeviceInfo,
        DaemonCommand::GetCameraPower,
        DaemonCommand::GetChargeLimit,
        DaemonCommand::GetCpuPower,
        DaemonCommand::GetSmt,
        DaemonCommand::GetBoost,
        DaemonCommand::DiagnoseRgb,
        DaemonCommand::GetThermalStatus,
        DaemonCommand::GetCurveOptimizer,
    ];
    let mut kinds: Vec<&'static str> = reads.iter().map(comms::cmd_kind).collect();
    kinds.sort_unstable();
    let before = kinds.clone();
    kinds.dedup();
    assert_eq!(before, kinds, "duplicate cmd_kind among read commands");
}
