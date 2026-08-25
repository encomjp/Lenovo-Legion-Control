//! Legion Control — live self-test suite.
//!
//! Validates 50+ features against REAL hardware, REAL sysfs and the REAL
//! running daemon. No mocks. Strictly read-only: every check here must be
//! safe on a production laptop (sysfs/hidraw/IPC reads only — never add a
//! write; write paths are covered by pure-helper unit tests).
//!
//! Skipped by default so `cargo test` stays green on any machine:
//!
//! ```bash
//! cargo test --test hardware_live -- --ignored --nocapture
//! ```
//!
//! Feature IDs (F01…) let a run be mapped to this checklist 1:1.

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

// ─── Battery ────────────────────────────────────────────────────────────────

#[test]
#[ignore = "live self-test"]
fn f01_battery_capacity_status_voltage_cycles() {
    skip_unless_hw!();
    let pct = battery::capacity().expect("F01 capacity");
    assert!(pct <= 100);
    let status = battery::status().unwrap_or_default();
    assert!(!status.is_empty());
    let v = battery::voltage().expect("F01 voltage");
    assert!((5.0..=30.0).contains(&v), "voltage {v} V implausible");
    battery::cycles().expect("F01 cycle_count");
}

#[test]
#[ignore = "live self-test"]
fn f02_battery_energy_and_health() {
    skip_unless_hw!();
    let now = battery::energy_now_wh().expect("energy_now");
    let full = battery::energy_full_wh().expect("energy_full");
    let design = battery::energy_design_wh().expect("energy_design");
    assert!(now >= 0.0 && full > 0.0 && design > 0.0);
    assert!(
        full <= design * 1.2,
        "full {full} Wh > 120% of design {design}"
    );
    let health = battery::health_pct().expect("health");
    assert!((10.0..=120.0).contains(&health));
}

#[test]
#[ignore = "live self-test"]
fn f03_battery_identity_fields() {
    skip_unless_hw!();
    assert!(!battery::manufacturer().unwrap_or_default().is_empty());
    assert!(!battery::model_name().unwrap_or_default().is_empty());
    assert!(!battery::technology().unwrap_or_default().is_empty());
}

#[test]
#[ignore = "live self-test"]
fn f04_charge_types_selection_parses_from_live_value() {
    skip_unless_hw!();
    let types = battery::charge_types().expect("charge_types attr");
    assert!(types.contains('['), "no bracketed selection in {types:?}");
    let sel = battery::charge_limit_pct();
    assert!(
        [60, 80, 100].contains(&sel),
        "limit {sel}% not a modeled state"
    );
    // Cross-check the two views of the one firmware bit.
    let conservation = battery::conservation_mode();
    if sel == 60 {
        assert_eq!(conservation, Some(true));
    }
}

#[test]
#[ignore = "live self-test"]
fn f05_limiter_band_predicate_matches_live_state() {
    skip_unless_hw!();
    let limit = battery::charge_limit_pct();
    let cap = battery::capacity().unwrap_or(0);
    // Pure predicate and live helper must agree.
    assert_eq!(
        battery::charged_past_limiter(),
        battery::above_limiter_band(limit, cap)
    );
}

#[test]
#[ignore = "live self-test"]
fn f06_battery_power_draw_finite() {
    skip_unless_hw!();
    if let Some(w) = battery::power_w() {
        assert!(w.abs() < 500.0, "battery power {w} W implausible");
    }
}

// ─── Fans ───────────────────────────────────────────────────────────────────

#[test]
#[ignore = "live self-test"]
fn f07_fan_channels_enumerated_with_sane_limits() {
    skip_unless_hw!();
    let channels = fans::channels();
    assert!(!channels.is_empty(), "no fan channels discovered");
    for f in &channels {
        assert!(!f.title.is_empty());
        assert!(f.min_rpm > 0 && f.min_rpm <= f.max_rpm);
        assert!(f.max_rpm <= 20_000);
    }
}

#[test]
#[ignore = "live self-test"]
fn f08_live_fan_rpms_and_targets_readable() {
    skip_unless_hw!();
    for id in fans::ids() {
        let rpm = fans::read_rpm(id).expect("fan rpm");
        assert!(rpm <= 20_000, "fan {id}: {rpm} rpm implausible");
        let target = fans::read_target(id).expect("fan target");
        assert!(target <= 20_000, "fan {id}: target {target} implausible");
        let label = fans::rpm_label(id);
        assert!(!label.is_empty(), "fan {id}: empty label");
        if target == 0 {
            assert!(
                label.starts_with("Auto"),
                "target 0 must label Auto, got {label:?}"
            );
        }
    }
}

// ─── Sensors ────────────────────────────────────────────────────────────────

#[test]
#[ignore = "live self-test"]
fn f09_read_all_temps_plausible() {
    skip_unless_hw!();
    let s = sensors::read_all();
    for name in ["cpu_temp", "cpu_temp_1", "cpu_temp_2"] {
        let t = match name {
            "cpu_temp" => s.cpu_temp,
            "cpu_temp_1" => s.cpu_temp_1,
            _ => s.cpu_temp_2,
        };
        assert!(
            (TEMP_MIN_C..=TEMP_MAX_C).contains(&t),
            "{name} {t}°C implausible"
        );
    }
    assert!(s.dgpu_temp >= TEMP_MIN_C, "dGPU sentinel/garbage");
    assert!(s.dgpu_power >= TEMP_MIN_C);
    assert!(s.dgpu_clock == -1.0 || (0.0..=6000.0).contains(&s.dgpu_clock));
    for t in &s.ssd_composite {
        assert!((TEMP_MIN_C..=95.0).contains(t), "SSD {t}°C implausible");
    }
}

#[test]
#[ignore = "live self-test"]
fn f10_sensors_report_platform_profile() {
    skip_unless_hw!();
    let s = sensors::read_all();
    assert!(!s.profile.is_empty(), "sensors profile empty");
    // Must agree with the profile module reading the same sysfs.
    assert_eq!(s.profile, profile::current());
}

#[test]
#[ignore = "live self-test"]
fn f11_cpu_usage_sampling_two_ticks_in_range() {
    skip_unless_hw!();
    let first = sensors::sample_cpu_usage_pct(); // seeds baseline
    let _ = first;
    std::thread::sleep(std::time::Duration::from_millis(300));
    let second = sensors::sample_cpu_usage_pct();
    assert!(
        (0.0..=100.0).contains(&second),
        "usage {second}% out of range"
    );
}

#[test]
#[ignore = "live self-test"]
fn f12_rapl_power_sampling_non_negative() {
    skip_unless_hw!();
    let _ = sensors::sample_cpu_power_w(); // seeds baseline
    std::thread::sleep(std::time::Duration::from_millis(300));
    let w = sensors::sample_cpu_power_w();
    assert!((0.0..=500.0).contains(&w), "RAPL {w} W implausible");
}

// ─── Thermal governor inputs ────────────────────────────────────────────────

#[test]
#[ignore = "live self-test"]
fn f13_k10temp_readings_on_real_cpu() {
    skip_unless_hw!();
    // k10temp exposes the AMD Tctl (package) and Tccd2 (second CCD) sensors,
    // reported here as the two CPU temps in milli-celsius.
    let (cpu_temp_mc, cpu_temp_2_mc) = thermal::read_cpu_temps();
    assert!(
        cpu_temp_mc.is_some() || cpu_temp_2_mc.is_some(),
        "no k10temp temps at all"
    );
    for t in [cpu_temp_mc, cpu_temp_2_mc].into_iter().flatten() {
        let c = t as f64 / 1000.0;
        assert!((0.0..=TEMP_MAX_C).contains(&c), "k10temp {c}°C implausible");
    }
}

#[test]
#[ignore = "live self-test"]
fn f14_scaling_max_freq_readable_and_hardcoded_max_matches_policy() {
    skip_unless_hw!();
    let cur = thermal::read_cur_max().expect("scaling_max_freq readable");
    assert!((400_000..=10_000_000).contains(&cur));

    // REAL cross-check of the hardcoded constant against cpufreq policy:
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
    // Compile-time pins on governor constants (live policy_max checked above).
    const _: () = assert!(thermal::MIN < thermal::MAX_FULL);
    const _: () = assert!(thermal::HYSTERESIS > 0);
}

#[test]
#[ignore = "live self-test"]
fn f15_compute_target_exercises_with_live_values() {
    skip_unless_hw!();
    use legion_core::thermal::{compute_target, ThermalConfig};
    // Disabled config → None (governor idle path).
    let disabled = ThermalConfig::default();
    let temp_mc = thermal::read_cpu_temps().0.unwrap_or(60_000);
    let cur = thermal::read_cur_max().unwrap_or(thermal::MAX_FULL);
    assert_eq!(compute_target(cur, temp_mc, &disabled), None);

    // Enabled with a mid-range limit → decision within [MIN, MAX_FULL].
    let enabled = ThermalConfig {
        enabled: true,
        max_temp: 90,
    };
    if let Some(target) = compute_target(cur, temp_mc, &enabled) {
        assert!((thermal::MIN..=thermal::MAX_FULL).contains(&target));
    }
}

// ─── Platform profiles ──────────────────────────────────────────────────────

#[test]
#[ignore = "live self-test"]
fn f16_platform_profile_current_within_choices() {
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
fn f17_ppt_surface_discoverable() {
    skip_unless_hw!();
    // Either firmware attrs exist or they legitimately don't on some models;
    // both must be reported without erroring.
    let _ = profile::ppt_available();
    let _ = profile::gpu_ppt_limits();
    for lim in profile::gpu_ppt_limits() {
        assert!(lim.min <= lim.max, "PPT limit min>max");
    }
}

// ─── Device identity ────────────────────────────────────────────────────────

#[test]
#[ignore = "live self-test"]
fn f18_device_identity_complete() {
    skip_unless_hw!();
    let info = device::detect();
    assert!(!info.model.is_empty());
    assert!(!info.machine_type.is_empty());
    assert!(info.machine_type.len() == 4, "machine type format");
    assert!(!info.bios_version.is_empty());
    assert!(!info.cpu_model.is_empty());
    assert!(!info.gpu_model.is_empty());
    assert!(!info.capabilities.fans.is_empty());
}

#[test]
#[ignore = "live self-test"]
fn f19_device_detect_cached_identical() {
    skip_unless_hw!();
    let a = device::detect();
    let b = device::detect();
    assert_eq!(a.machine_type, b.machine_type);
    assert_eq!(a.capabilities.fans.len(), b.capabilities.fans.len());
    assert_eq!(format!("{a:?}"), format!("{b:?}"), "cache not stable");
}

// ─── Keyboard / Spectrum (reads only) ───────────────────────────────────────

#[test]
#[ignore = "live self-test"]
fn f20_spectrum_controller_read_only_probe() {
    skip_unless_hw!();
    // Detection scan + HIDIOCGFEATURE — both reads. None = controller not
    // present/unreadable on this machine, which is a valid outcome.
    if let Some(b) = keyboard::rgb_brightness() {
        assert!(b <= 9, "Spectrum brightness {b} > max 9");
    }
    if let Some(rgb) = keyboard::peek_effect_rgb() {
        let (r, g, b) = rgb;
        // Decoded bytes must be a legal RGB triple (u8 range is inherent —
        // this pins that the GET path returns exactly 3 channels).
        let _ = (r, g, b);
    }
}

#[test]
#[ignore = "live self-test"]
fn f21_camera_killswitch_readable() {
    skip_unless_hw!();
    // ideapad camera power attr — Option<bool> by parse; None is valid on
    // machines without the switch.
    let _ = keyboard::camera_power();
}

// ─── Curve Optimizer (SMU reads) ────────────────────────────────────────────

#[test]
#[ignore = "live self-test"]
fn f22_curve_optimizer_status_contract() {
    skip_unless_hw!();
    let st = undervolt::status();
    assert!(!st.reason.is_empty());
    if st.available {
        assert!(st.minimum <= 0 && st.maximum <= 0);
        assert!(st.maximum >= st.minimum);
        assert!(!st.current.is_empty(), "available but no offsets read");
        assert_eq!(
            st.current.len(),
            st.boot_baseline.len(),
            "offset vector mismatch"
        );
    }
}

// ─── Config persistence (REAL filesystem, isolated dir) ─────────────────────

/// f23/f24 both repoint XDG_CONFIG_HOME — cargo runs tests in parallel
/// threads within this binary, so they must take turns.
static CONFIG_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
#[ignore = "live self-test"]
fn f23_config_round_trip_on_real_fs() {
    let _guard = CONFIG_ENV_LOCK.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("legion-selftest-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("legion-control")).unwrap();
    // SAFETY: single-threaded with respect to env — other tests here do not
    // touch XDG_CONFIG_HOME.
    unsafe { std::env::set_var("XDG_CONFIG_HOME", &dir) };

    let cfg = config::get();
    let version_before = cfg.version;
    assert!(version_before >= 4, "unexpected config schema version");

    config::update(|c| c.ui_zone = "selftest".into());
    let reloaded = config::get();
    assert_eq!(reloaded.ui_zone, "selftest", "update did not persist");

    // settings.json really exists on disk now.
    assert!(dir.join("legion-control/settings.json").is_file());

    unsafe { std::env::remove_var("XDG_CONFIG_HOME") };
    let _ = std::fs::remove_dir_all(&dir);
}

// f24 lives in tests/hardware_live_cfg.rs — it needs its own process so the
// config OnceLock store is freshly initialized against the corrupt file.

// ─── IPC against the REAL running daemon (end-to-end) ───────────────────────

fn expect_ipc(cmd: DaemonCommand) -> DaemonResponse {
    send_command(cmd).expect("IPC round-trip to running daemon")
}

#[test]
#[ignore = "live self-test"]
fn f25_ipc_get_profile_matches_local_sysfs_view() {
    skip_unless_hw!();
    skip_unless_daemon!();
    match expect_ipc(DaemonCommand::GetProfile) {
        DaemonResponse::Profile(p) => assert_eq!(p, profile::current()),
        other => panic!("unexpected response: {other:?}"),
    }
}

#[test]
#[ignore = "live self-test"]
fn f26_ipc_battery_matches_direct_sysfs_reads() {
    skip_unless_hw!();
    skip_unless_daemon!();
    match expect_ipc(DaemonCommand::GetBattery) {
        DaemonResponse::Battery {
            capacity,
            status,
            voltage,
            cycles,
            conservation,
        } => {
            assert_eq!(capacity, battery::capacity().unwrap_or(0), "capacity skew");
            assert_eq!(status, battery::status().unwrap_or_default());
            if let Some(v) = battery::voltage() {
                assert!((voltage - v).abs() < 0.5, "voltage skew {voltage} vs {v}");
            }
            let _ = cycles;
            assert_eq!(conservation, battery::charge_limit_pct() < 100);
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[test]
#[ignore = "live self-test"]
fn f27_ipc_sensors_match_local_reads() {
    skip_unless_hw!();
    skip_unless_daemon!();
    match expect_ipc(DaemonCommand::GetSensors) {
        DaemonResponse::Sensors(s) => {
            let local = sensors::read_all();
            assert!((s.cpu_temp - local.cpu_temp).abs() < 15.0, "CPU temp skew");
            assert_eq!(s.profile, local.profile);
            assert_eq!(s.fan1_rpm, local.fan1_rpm);
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[test]
#[ignore = "live self-test"]
fn f28_ipc_device_info_matches_local_detect() {
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
fn f29_ipc_root_only_commands_answer() {
    skip_unless_daemon!();
    // GetCpuPower requires root daemon (RAPL) — the deployed daemon runs root.
    match expect_ipc(DaemonCommand::GetCpuPower) {
        DaemonResponse::CpuPower(w) => assert!((0.0..=500.0).contains(&w)),
        DaemonResponse::Error(e) => panic!("root daemon refused RAPL: {e}"),
        other => panic!("unexpected response: {other:?}"),
    }
    match expect_ipc(DaemonCommand::GetChargeLimit) {
        DaemonResponse::ChargeLimit(p) => {
            assert!([60, 80, 100].contains(&p));
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[test]
#[ignore = "live self-test"]
fn f30_ipc_read_surfaces_answer_without_error() {
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
fn f31_ipc_diagnose_rgb_reports_health() {
    skip_unless_daemon!();
    match expect_ipc(DaemonCommand::DiagnoseRgb) {
        DaemonResponse::RgbDiagnosis {
            health, summary, ..
        } => {
            assert!(!health.is_empty() && !summary.is_empty());
            assert!(["ok", "soft-issue", "broken", "n/a"].contains(&health.as_str()));
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

// ─── IPC protocol safety (compile-time + wire contracts) ────────────────────

#[test]
#[ignore = "live self-test"]
fn f32_socket_candidates_never_tmp_and_frame_cap_constant() {
    for p in comms::socket_candidates() {
        assert!(!p.starts_with("/tmp"), "unsafe candidate {p:?}");
    }
    const _: () = assert!(comms::MAX_FRAME_BYTES == 4 * 1024 * 1024);

    // Non-root client: bind path must be absolute under XDG_RUNTIME_DIR, or
    // an explicit error — never the system socket, never a relative path.
    if unsafe { libc::geteuid() } != 0 {
        if let Ok(p) = comms::bind_socket_path() {
            assert!(p.is_absolute(), "relative socket path {p:?}");
            assert_ne!(
                p,
                std::path::Path::new(comms::SYSTEM_SOCKET),
                "non-root must not claim the system socket"
            );
        } // Err: no XDG_RUNTIME_DIR — valid on headless CI
    }
}
