//! Self-health checks — the runtime twin of `tests/hardware_live.rs`.
//!
//! Every check is strictly READ-ONLY and safe to run on a production laptop.
//! Used by the GUI "Run self-check" button, `legion-cli diagnose selfcheck`,
//! and the anonymous diagnostics report.

use crate::{battery, comms, config, device, fans, keyboard, profile, sensors, thermal, undervolt};

#[derive(Debug, Clone, serde::Serialize)]
pub struct SelfCheck {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
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
    out.push(check(
        "charge_limit_state",
        [60, 80, 100].contains(&limit),
        format!("{limit}%"),
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
    out.push(check(
        "curve_optimizer",
        !co.reason.is_empty(),
        if co.available {
            format!("available ({})", co.reason)
        } else {
            co.reason.clone()
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

    out
}
