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
//!
//! [`run_deployment_checks`] validates install state: user group membership,
//! daemon unit enabled, udev rule applied, socket permissions, binary path.

use crate::{battery, comms, config, device, fans, keyboard, profile, sensors, thermal, undervolt};
use std::os::unix::fs::PermissionsExt;

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
    log::debug!("self-checks: running");
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
        (None, _) => out.push(check(
            "battery_capacity",
            false,
            "battery capacity unreadable",
        )),
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
        format!("{} channel(s) via {}", channels.len(), fans::backend_name()),
    ));
    // Missing RPM support is a capability state, not a failed read. Only an
    // attribute that exists but cannot be read indicates a broken backend.
    let mut rpm_ok = true;
    let mut detail_parts: Vec<String> = Vec::new();
    for f in &channels {
        let (rpm, state) = fans::rpm_status(f.id);
        if state == fans::FanRpmState::Unreadable {
            rpm_ok = false;
        }
        match rpm {
            Some(rpm) => detail_parts.push(format!("fan{} {rpm} rpm", f.id)),
            None => {
                detail_parts.push(format!("fan{} {}", f.id, state.as_str()));
            }
        }
    }
    out.push(check("fan_rpms_readable", rpm_ok, detail_parts.join(" · ")));

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

    // cpufreq inputs + dynamic scaling policy check (bounds check against hardware capability).
    match thermal::read_cur_max() {
        Some(cur) if (400_000..=10_000_000).contains(&cur) => {
            let policy: Option<u32> =
                std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq")
                    .ok()
                    .and_then(|raw| raw.trim().parse().ok());
            match policy {
                Some(policy_max) => {
                    let sane = cur >= 400_000 && cur <= policy_max;
                    out.push(check(
                        "cpufreq_scaling_policy",
                        sane,
                        format!("cur_max {cur} kHz (policy max {policy_max} kHz)"),
                    ));
                }
                None => out.push(check(
                    "cpufreq_scaling_policy",
                    cur >= 400_000,
                    format!("cur_max {cur} kHz (policy max unreadable)"),
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

    let passed = out.iter().filter(|c| c.ok).count();
    log::debug!("self-checks: done — {passed}/{} passed", out.len());
    out
}

// ═══ Machine fault detection (anomalies, distinct from pass/fail) ═════════

/// Scan for active machine anomalies. Read-only; typical cost is one
/// `sensors::read_all()` plus a handful of fan/battery reads.
pub fn scan_faults() -> Vec<Fault> {
    log::debug!("fault scan: running");
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
    // Only fire when fan RPMs are actually READABLE and all read 0 — on
    // machines where the tachometer is unavailable (IdeaPad on kernels
    // without yogafan) every read returns None, which is "unknown", not
    // "fans off" (fleet false-positive at 81°C).
    let hottest = s.cpu_temp.max(s.dgpu_temp.max(-1.0));
    let fan_ids = fans::ids();
    let readings: Vec<Option<u32>> = fan_ids.iter().map(|id| fans::read_rpm(*id)).collect();
    let any_readable = readings.iter().any(|r| r.is_some());
    let any_airflow = readings.iter().any(|r| r.unwrap_or(0) > 0);
    if hottest >= 80.0 && any_readable && !any_airflow {
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
            if status == "Charging" && pct > limit && pct < 100 {
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
            // Cap threshold must respect THIS CPU's ceiling — MAX_FULL is the
            // 5.46 GHz reference (9955HX3D); a 4.28 GHz APU parked at its own
            // policy max is not "throttled" (IdeaPad fleet false-positive).
            let policy_max =
                std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq")
                    .ok()
                    .and_then(|s| s.trim().parse::<u32>().ok())
                    .unwrap_or(thermal::MAX_FULL);
            let cap_threshold = policy_max.saturating_sub(100_000);
            let restore_mc = (cfg.max_temp as i32 - 7) * 1000;
            // Only flag when the CPU is genuinely cool — not when the thermal
            // governor is holding a prior cap in the restore hysteresis band
            // (fleet false-positive ~82°C on Legion while fans spin normally).
            let cold_enough_mc = restore_mc - 15_000;
            if cur < cap_threshold && temp_mc(&s) < cold_enough_mc {
                out.push(fault(
                    "throttled_without_heat",
                    Severity::Warning,
                    format!(
                        "capped at {cur} kHz while CPU is {:.1}°C (< restore {:.0}°C)",
                        temp_mc(&s) as f64 / 1000.0,
                        restore_mc as f64 / 1000.0
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

    // ── iGPU temp anomaly ────────────────────────────────────────────────
    if s.igpu_edge >= 95.0 {
        out.push(fault(
            "igpu_overheat",
            Severity::Critical,
            format!("iGPU edge temp {:.1}°C — thermal emergency", s.igpu_edge),
        ));
    }

    // ── EC GPU temp divergence from nvidia-smi ───────────────────────────
    if s.ec_gpu > 1.0 && s.dgpu_temp > 50.0 && (s.ec_gpu - s.dgpu_temp).abs() > 15.0 {
        out.push(fault(
            "ec_gpu_temp_divergence",
            Severity::Warning,
            format!(
                "EC GPU {:.1}°C vs nvidia-smi {:.1}°C (>15° apart)",
                s.ec_gpu, s.dgpu_temp
            ),
        ));
    }

    // ── RAM overheat (DDR5 SPD5118 throttles at 85°C) ────────────────────
    for (i, t) in s.ram_temps.iter().enumerate() {
        if *t >= 80.0 {
            out.push(fault(
                "ram_overheat",
                Severity::Warning,
                format!("DIMM {} at {:.0}°C — SPD throttle imminent", i, t),
            ));
        }
    }

    // ── dGPU clock stuck at max while cool (possible driver issue) ──────
    if s.dgpu_clock > 2500.0 && s.dgpu_temp < 40.0 && s.dgpu_power < 10.0 {
        out.push(fault(
            "dgpu_clock_stuck",
            Severity::Info,
            format!(
                "dGPU at {} MHz with {:.1} W draw and {:.1}°C — possible stuck clocks",
                s.dgpu_clock, s.dgpu_power, s.dgpu_temp
            ),
        ));
    }

    // ── System uptime extremely long (kernel resource leak risk) ────────
    let uptime_secs = std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
        .unwrap_or(0.0);
    if uptime_secs > 90.0 * 24.0 * 3600.0 {
        out.push(fault(
            "system_uptime_extreme",
            Severity::Info,
            format!(
                "system up for {:.0} days — consider rebooting to reset hardware state",
                uptime_secs / 86400.0
            ),
        ));
    }

    // ── Memory pressure (available < 256 MB) ────────────────────────────
    if let Ok(meminfo) = std::fs::read_to_string("/proc/meminfo") {
        if let Some(line) = meminfo.lines().find(|l| l.starts_with("MemAvailable:")) {
            if let Some(kb) = line
                .split_whitespace()
                .nth(1)
                .and_then(|v| v.parse::<u64>().ok())
            {
                if kb < 262_144 {
                    out.push(fault(
                        "memory_pressure",
                        Severity::Warning,
                        format!("only {} MB available", kb / 1024),
                    ));
                }
            }
        }
    }

    let criticals = out
        .iter()
        .filter(|f| f.severity == Severity::Critical)
        .count();
    let warnings = out
        .iter()
        .filter(|f| f.severity == Severity::Warning)
        .count();
    let infos = out.iter().filter(|f| f.severity == Severity::Info).count();
    log::debug!("fault scan: done — {criticals} critical / {warnings} warning / {infos} info");
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

// ─── Deployment / install-state checks ───────────────────────────────────

/// Current user is a member of the named group (supplementary + effective).
fn user_in_group(group: &str) -> bool {
    let cname = match std::ffi::CString::new(group) {
        Ok(c) => c,
        Err(_) => {
            log::debug!("user_in_group('{group}'): not a valid group name");
            return false;
        }
    };
    let gr = unsafe { libc::getgrnam(cname.as_ptr()) };
    if gr.is_null() {
        log::debug!("user_in_group('{group}'): group not found");
        return false;
    }
    let gid = unsafe { (*gr).gr_gid };
    log::debug!("user_in_group('{group}'): found gid {gid}");
    let n = unsafe { libc::getgroups(0, std::ptr::null_mut()) };
    if n <= 0 {
        log::debug!("user_in_group('{group}'): no supplementary groups");
        return false;
    }
    let mut gids = vec![0u32; n as usize];
    let n = unsafe { libc::getgroups(n as i32, gids.as_mut_ptr() as *mut libc::gid_t) };
    let member = n > 0 && gids[..n as usize].contains(&gid);
    log::debug!("user_in_group('{group}'): member={member}");
    member
}

fn daemon_unit_enabled() -> Option<bool> {
    let out = std::process::Command::new("systemctl")
        .args(["is-enabled", "legion-control.service"])
        .output()
        .ok()?;
    let state = String::from_utf8_lossy(&out.stdout).trim().to_string();
    log::debug!("daemon_unit_enabled: systemctl reports '{state}'");
    Some(state == "enabled")
}

fn hidraw_spectrum_node() -> Option<(std::path::PathBuf, u32)> {
    let dir = std::fs::read_dir("/sys/class/hidraw").ok()?;
    let mut scanned = 0usize;
    for entry in dir.flatten() {
        scanned += 1;
        let uevent = entry.path().join("device/uevent");
        let content = std::fs::read_to_string(&uevent).unwrap_or_default();
        if content.contains("048D") && content.contains("C197") {
            let devnode = std::path::Path::new("/dev").join(entry.file_name());
            let mode = devnode.metadata().ok()?.permissions().mode();
            log::debug!(
                "hidraw_spectrum_node: match {} after scanning {scanned} node(s)",
                devnode.display()
            );
            return Some((devnode, mode & 0o777));
        }
    }
    log::debug!("hidraw_spectrum_node: no match in {scanned} node(s)");
    None
}

/// Validate install state: user group membership, daemon unit enabled,
/// udev rule applied, socket permissions, binary location. Read-only.
pub fn run_deployment_checks() -> Vec<SelfCheck> {
    log::debug!("deployment checks: running");
    let mut out = Vec::new();

    // User in `legion` group (socket access).
    let in_group = user_in_group("legion");
    let is_root = unsafe { libc::geteuid() } == 0;
    out.push(check(
        "user_in_legion_group",
        in_group || is_root,
        if is_root {
            String::from("running as root")
        } else if in_group {
            String::from("member of legion group")
        } else {
            String::from("not in legion group — sudo usermod -aG legion $USER")
        },
    ));

    // Daemon systemd unit enabled.
    match daemon_unit_enabled() {
        Some(true) => out.push(check("daemon_unit_enabled", true, "enabled")),
        Some(false) => out.push(check(
            "daemon_unit_enabled",
            false,
            "not enabled — systemctl enable legion-control",
        )),
        None => out.push(check(
            "daemon_unit_enabled",
            false,
            "systemctl unavailable".to_string(),
        )),
    }

    // Spectrum udev rule applied.
    match hidraw_spectrum_node() {
        Some((node, mode)) => {
            let accessible = mode & 0o640 == 0o640 || mode & 0o006 != 0;
            out.push(check(
                "spectrum_udev_rule",
                accessible,
                format!("{} mode {:o}", node.display(), mode),
            ));
        }
        None => out.push(check(
            "spectrum_udev_rule",
            true,
            "Spectrum controller absent (optional)".to_string(),
        )),
    }

    // Socket permissions sane (not world-writable).
    let sock = std::path::Path::new(comms::SYSTEM_SOCKET);
    if sock.exists() {
        let mode = sock.metadata().map(|m| m.permissions().mode()).unwrap_or(0);
        out.push(check(
            "socket_permissions",
            mode & 0o006 == 0,
            format!("mode {:o}", mode),
        ));
    } else {
        out.push(check(
            "socket_permissions",
            true,
            "socket absent (daemon off?)".to_string(),
        ));
    }

    // Binary location sanity.
    let bin_ok = std::path::Path::new("/usr/local/bin/legion-daemon").exists()
        || std::path::Path::new("/usr/bin/legion-daemon").exists();
    out.push(check(
        "binary_location",
        bin_ok,
        if bin_ok {
            "installed".to_string()
        } else {
            "legion-daemon binary not found".to_string()
        },
    ));

    // ryzen_smu module loaded (optional on supported AMD, not applicable on Intel).
    let is_amd = device::detect()
        .cpu_model
        .to_ascii_uppercase()
        .contains("AMD");
    if is_amd {
        let smu = std::path::Path::new("/sys/kernel/ryzen_smu_drv").exists();
        out.push(check(
            "ryzen_smu_module",
            smu,
            if smu {
                "loaded".to_string()
            } else {
                "not loaded (needed for Curve Optimizer only)".to_string()
            },
        ));
    } else {
        out.push(check(
            "ryzen_smu_module",
            true,
            "not applicable (Intel CPU)".to_string(),
        ));
    }

    let passed = out.iter().filter(|c| c.ok).count();
    log::debug!("deployment checks: done — {passed}/{} passed", out.len());
    out
}
