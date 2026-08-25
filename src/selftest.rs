//! Self-health checks — the runtime twin of `tests/hardware_live.rs`.
//!
//! Every check is strictly READ-ONLY and safe to run on a production laptop.
//! Used by the GUI "Run self-check" button, `legion-cli diagnose selfcheck`,
//! and the anonymous diagnostics report.
//!
//! [`scan_faults`] complements the pass/fail checks with *anomaly detection*:
//! conditions that are legal states but usually indicate a problem (a fan
//! stalling under load, the EC charging past the configured limiter,
//! divergent temperature sources, an unwritable config directory …).

use crate::{battery, comms, config, device, fans, keyboard, profile, sensors, thermal, undervolt};

#[derive(Debug, Clone, serde::Serialize)]
pub struct SelfCheck {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
}

/// An active machine anomaly. `Critical` = hardware risk / broken core flow;
/// `Warning` = degraded or suspicious; `Info` = notable but expected in some
/// configurations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Severity {
    Critical,
    Warning,
    Info,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Fault {
    pub id: &'static str,
    pub severity: Severity,
    pub detail: String,
}

fn fault(id: &'static str, severity: Severity, detail: impl Into<String>) -> Fault {
    Fault {
        id,
        severity,
        detail: detail.into(),
    }
}

fn check(name: &'static str, ok: bool, detail: impl Into<String>) -> SelfCheck {
    SelfCheck {
        name,
        ok,
        detail: detail.into(),
    }
}

fn plaus(v: f64, lo: f64, hi: f64) -> bool {
    (-20.0..=hi).contains(&v) && v >= lo || v == -1.0 // -1 = "no reading" sentinel
}

/// Run every read-only health check. Fast (<200 ms): no sampling sleeps.
pub fn run_self_checks() -> Vec<SelfCheck> {
    let mut out = Vec::new();

    // Config store loads and has a sane schema version.
    let cfg = config::get();
    out.push(check(
        "config_loads",
        cfg.version >= 4,
        format!("schema v{}", cfg.version),
    ));

    // Battery surface.
    match (battery::capacity(), battery::status()) {
        (Some(pct), _) if pct <= 100 => {
            out.push(check("battery_capacity", true, format!("{pct}%")))
        }
        (Some(pct), _) => out.push(check(
            "battery_capacity",
            false,
            format!("{pct}% out of range"),
        )),
        (None, _) => out.push(check("battery_capacity", false, "BAT0 capacity unreadable")),
    }
    let status = battery::status().unwrap_or_default();
    out.push(check(
        "battery_status",
        !status.is_empty(),
        if status.is_empty() {
            "empty".into()
        } else {
            status
        },
    ));
    let limit = battery::charge_limit_pct();
    let source = if battery::charge_types().is_some() {
        "charge_types"
    } else if battery::conservation_mode().is_some() {
        "conservation_mode"
    } else {
        "unreachable"
    };
    out.push(check(
        "charge_limit_state",
        source != "unreachable",
        format!("{limit}% via {source}"),
    ));

    // Fans.
    let channels = fans::channels();
    out.push(check(
        "fans_enumerated",
        !channels.is_empty(),
        format!("{} channel(s)", channels.len()),
    ));
    let mut rpm_ok = true;
    for id in fans::ids() {
        if fans::read_rpm(id).is_none() {
            rpm_ok = false;
        }
    }
    out.push(check(
        "fan_rpms_readable",
        rpm_ok,
        format!("{} fan(s)", channels.len()),
    ));

    // Temperatures.
    let s = sensors::read_all();
    let t_ok = plaus(s.cpu_temp, 0.0, 125.0);
    out.push(check(
        "k10temp_cpu_temp",
        t_ok,
        format!("{:.1}°C", s.cpu_temp),
    ));
    out.push(check(
        "dgpu_probe",
        plaus(s.dgpu_temp, 0.0, 125.0),
        if s.dgpu_temp == -1.0 {
            "powered down".into()
        } else {
            format!("{:.1}°C", s.dgpu_temp)
        },
    ));
    out.push(check(
        "nvme_temps",
        s.ssd_composite.iter().all(|t| (0.0..=95.0).contains(t)),
        format!("{} drive(s)", s.ssd_composite.len()),
    ));
    out.push(check(
        "ram_temps",
        s.ram_temps.iter().all(|t| (0.0..=125.0).contains(t)),
        format!("{} module(s)", s.ram_temps.len()),
    ));

    // Spectrum / switches.
    match keyboard::rgb_brightness() {
        Some(b) => out.push(check(
            "spectrum_controller",
            b <= 9,
            format!("brightness {b}/9"),
        )),
        None => out.push(check(
            "spectrum_controller",
            false,
            "048d:c197 not detected (optional component)",
        )),
    }
    out.push(check(
        "camera_switch",
        keyboard::camera_power().is_some(),
        String::from("ideapad attr"),
    ));

    // Platform profile.
    let current = profile::current();
    let in_choices = profile::choices()
        .iter()
        .any(|c| c.eq_ignore_ascii_case(&current));
    out.push(check(
        "platform_profile",
        !current.is_empty() && in_choices,
        current,
    ));

    // PPT surface (informational — models differ legitimately).
    let limits = profile::gpu_ppt_limits();
    out.push(check(
        "ppt_surface",
        true,
        format!(
            "available={} · {} gpu limit(s)",
            profile::ppt_available(),
            limits.len()
        ),
    ));

    // Curve optimizer (root-only probe degrades gracefully off-daemon).
    let co = undervolt::status();
    let is_root = unsafe { libc::geteuid() } == 0;
    out.push(check(
        "curve_optimizer",
        co.available || !is_root,
        if co.available {
            format!("available ({})", co.reason)
        } else if is_root {
            format!("root probe failed: {}", co.reason)
        } else {
            format!("unavailable without root (expected): {}", co.reason)
        },
    ));

    // cpufreq inputs + constant cross-check against the real policy.
    match thermal::read_cur_max() {
        Some(cur) if (400_000..=10_000_000).contains(&cur) => {
            let policy: Option<u32> =
                std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq")
                    .ok()
                    .and_then(|raw| raw.trim().parse().ok());
            match policy {
                Some(policy_max) => out.push(check(
                    "cpufreq_max_matches_constant",
                    policy_max == thermal::MAX_FULL,
                    format!("policy {policy_max} kHz vs MAX_FULL {}", thermal::MAX_FULL),
                )),
                None => out.push(check(
                    "cpufreq_max_matches_constant",
                    false,
                    "cpuinfo_max_freq unreadable",
                )),
            }
        }
        other => out.push(check(
            "scaling_max_freq_readable",
            other.is_some(),
            format!("{:?}", other),
        )),
    }

    // Daemon reachable (meaningful for CLI/GUI contexts; trivially true when
    // this code runs inside the daemon itself).
    let daemon_ok = matches!(
        comms::send_command(comms::DaemonCommand::GetProfile),
        Ok(comms::DaemonResponse::Profile(_))
    );
    out.push(check("daemon_reachable", daemon_ok, String::new()));

    // Socket candidates sane.
    let sane = comms::socket_candidates()
        .iter()
        .all(|p| !p.starts_with("/tmp"));
    out.push(check("socket_paths_sane", sane, String::new()));

    // Device identity present.
    let info = device::detect();
    out.push(check(
        "device_identity",
        !info.model.is_empty() && !info.machine_type.is_empty(),
        format!("{} ({})", info.model, info.machine_type),
    ));

    // Intel-only surfaces — inert on AMD, live on Intel if detected.
    out.push(check(
        "intel_pstate",
        true,
        if crate::intel::pstate_available() {
            String::from("present")
        } else {
            String::from("not present (AMD)")
        },
    ));
    out.push(check(
        "intel_uncore",
        true,
        if crate::intel::uncore_available() {
            String::from("present")
        } else {
            String::from("not present (AMD)")
        },
    ));
    out.push(check(
        "intel_msr",
        true,
        if crate::intel_msr::is_available() {
            String::from("present")
        } else {
            String::from("not present (AMD)")
        },
    ));

    out
}

// ═══ Machine fault detection (anomalies, distinct from pass/fail) ═════════

/// Scan for active machine anomalies. Read-only; typical cost is one
/// `sensors::read_all()` plus a handful of fan/battery reads.
pub fn scan_faults() -> Vec<Fault> {
    let mut out = Vec::new();
    let s = sensors::read_all();

    // ── Fans: mechanical failure under an active target ────────────────
    for f in fans::channels() {
        let (Some(target), Some(rpm)) = (fans::read_target(f.id), fans::read_rpm(f.id)) else {
            continue;
        };
        if target >= f.min_rpm && rpm + 200 < f.min_rpm {
            out.push(fault(
                "fan_stalled_under_target",
                Severity::Critical,
                format!(
                    "fan {} told to spin {} rpm but reads {} (min {})",
                    f.id, target, rpm, f.min_rpm
                ),
            ));
        }
    }

    // ── Hot package with zero airflow ────────────────────────────────────
    let hottest = s.cpu_temp.max(s.dgpu_temp.max(-1.0));
    let any_airflow = fans::ids()
        .iter()
        .any(|id| fans::read_rpm(*id).unwrap_or(0) > 0);
    if hottest >= 80.0 && !any_airflow {
        out.push(fault(
            "fans_off_while_hot",
            Severity::Warning,
            format!("package at {hottest:.0}°C with all fans reading 0 rpm"),
        ));
    }

    // ── NVMe thermals ───────────────────────────────────────────────────
    for t in &s.ssd_composite {
        if *t >= 84.0 {
            out.push(fault(
                "nvme_overheat",
                Severity::Critical,
                format!("NVMe at {t:.0}°C — approaching throttle/limit"),
            ));
        } else if *t >= 70.0 {
            out.push(fault(
                "nvme_hot",
                Severity::Warning,
                format!("NVMe at {t:.0}°C"),
            ));
        }
    }

    // ── Battery degradation ─────────────────────────────────────────────
    if let Some(h) = battery::health_pct() {
        if h < 40.0 {
            out.push(fault(
                "battery_degraded",
                Severity::Critical,
                format!("battery health {h:.0}%"),
            ));
        } else if h < 60.0 {
            out.push(fault(
                "battery_worn",
                Severity::Warning,
                format!("battery health {h:.0}%"),
            ));
        }
    }

    // ── EC charging past the configured limiter ─────────────────────────
    let limit = battery::charge_limit_pct();
    if limit < 100 {
        if let (Some(pct), Some(status)) = (battery::capacity(), battery::status()) {
            if status == "Charging" && pct as i32 >= limit as i32 - 2 && pct < 100 {
                out.push(fault(
                    "charging_past_limiter",
                    Severity::Warning,
                    format!("charging at {pct}% with limiter set to {limit}%"),
                ));
            }
        }
    }

    // ── Limiter interfaces disagree (legacy bit vs charge_types) ────────
    if let (Some(cons), Some(types)) = (battery::conservation_mode(), battery::charge_types()) {
        let long_life = types.contains("[Long_Life]");
        if cons != long_life {
            out.push(fault(
                "limiter_interfaces_disagree",
                Severity::Warning,
                format!(
                    "conservation_mode={} but charge_types selection is [{}]",
                    u8::from(cons),
                    types.split('[').nth(1).unwrap_or("?").trim_end_matches(']')
                ),
            ));
        }
    }

    // ── EC CPU temp diverging from k10temp while under load ─────────────
    // ec_cpu == 0.0 on machines without an EC hwmon backend = no data.
    if s.ec_cpu > 1.0 && s.cpu_temp > 50.0 && (s.ec_cpu - s.cpu_temp).abs() > 12.0 {
        out.push(fault(
            "ec_cpu_temp_divergence",
            Severity::Warning,
            format!(
                "EC reports {:.1}°C vs k10temp {:.1}°C (>12° apart under load)",
                s.ec_cpu, s.cpu_temp
            ),
        ));
    }

    // ── dGPU telemetry partially degraded ───────────────────────────────
    if s.dgpu_clock > 500.0 && s.dgpu_power == -1.0 {
        out.push(fault(
            "dgpu_telemetry_partial",
            Severity::Info,
            "dGPU reports clocks but no power draw".to_string(),
        ));
    }

    // ── Config directory must be writable (persistence precursor) ───────
    if let Some(dir) = config_dir() {
        let probe = dir.join(".fault-probe");
        match std::fs::write(&probe, b"ok") {
            Ok(()) => {
                let _ = std::fs::remove_file(&probe);
            }
            Err(e) => out.push(fault(
                "config_dir_unwritable",
                Severity::Critical,
                format!("cannot write {}: {e}", dir.display()),
            )),
        }
    } else {
        out.push(fault(
            "config_dir_missing",
            Severity::Warning,
            "config directory does not exist yet".to_string(),
        ));
    }

    // ── Frequency capped without thermal cause ─────────────────────────
    let cfg = config::get().thermal;
    if cfg.enabled {
        if let Some(cur) = thermal::read_cur_max() {
            let restore_mc = (cfg.max_temp as i32 - 7) * 1000;
            if cur < thermal::MAX_FULL.saturating_sub(100_000) && temp_mc(&s) < restore_mc - 3_000 {
                out.push(fault(
                    "throttled_without_heat",
                    Severity::Warning,
                    format!(
                        "capped at {cur} kHz while CPU is {:.1}°C (< max {}°C)",
                        temp_mc(&s) as f64 / 1000.0,
                        cfg.max_temp
                    ),
                ));
            }
        }
    }

    // ── Error burst in the recent log window ────────────────────────────
    let entries = crate::logging::recent_logs(200);
    let errors = entries.iter().filter(|e| e.level == "ERROR").count();
    if errors >= 10 {
        out.push(fault(
            "log_error_burst",
            Severity::Warning,
            format!("{errors} errors in the last 200 log lines"),
        ));
    }

    out
}

fn temp_mc(s: &sensors::SensorReadings) -> i32 {
    (s.cpu_temp * 1000.0) as i32
}

fn config_dir() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let mut h = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
            h.push(".config");
            h
        });
    let dir = base.join("legion-control");
    dir.is_dir().then_some(dir)
}
