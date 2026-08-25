//! Anonymous diagnostics dump (alpha) + opt-in transport.
//!
//! PRIVACY CONTRACT — enforced by the unit test in this file:
//! The report is built from a field whitelist; nothing identifying is ever
//! collected. Excluded by construction: hostname, username, serial numbers,
//! MAC addresses, IP addresses, disk serials, per-key colour maps, custom
//! user strings. Included: hardware model/type/BIOS/CPU/GPU/EC identity,
//! distro+kernel, sensor readings, battery health stats (no serial), fan
//! states, thermal/CO configuration, a small settings digest, the daemon log
//! tail (already sanitized at write time) and the self-check results.
//!
//! Transport shells out to `curl` (present on every supported distro) to
//! avoid adding an HTTPS dependency; the payload goes through a 0600 temp
//! file that is removed immediately afterwards.

use crate::selftest::{run_self_checks, SelfCheck};
use crate::{battery, config, device, fans, profile, sensors, thermal, undervolt};
use serde::Serialize;
use std::io::Write;

/// Default collector (IONOS VPS, Tailscale-internal during alpha).
/// Public rollout: front it with nginx + TLS and change this constant.
pub const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:8787/v1/diagnostics";

pub const REPORT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize)]
pub struct DiagnosticsReport {
    pub schema_version: u32,
    pub generated_at: String,
    pub app_version: &'static str,

    pub device: device::DeviceInfo,
    pub os: OsInfo,
    pub sensors: sensors::SensorReadings,
    pub battery: BatterySummary,
    pub fans: Vec<FanLive>,
    pub thermal: ThermalDigest,
    pub profiles: ProfilesDigest,
    pub curve_optimizer: undervolt::CurveOptimizerStatus,
    pub settings: SettingsDigest,
    pub daemon_log_tail: String,
    pub self_checks: Vec<SelfCheck>,
}

#[derive(Debug, Serialize)]
pub struct OsInfo {
    pub distro: String,
    pub kernel: String,
}

#[derive(Debug, Serialize)]
pub struct BatterySummary {
    pub capacity_pct: Option<u32>,
    pub status: Option<String>,
    pub voltage_v: Option<f64>,
    pub cycles: Option<u32>,
    pub health_pct: Option<f64>,
    /// Effective firmware limiter state: 60 / 80 / 100.
    pub charge_limit_pct: u32,
}

#[derive(Debug, Serialize)]
pub struct FanLive {
    pub id: u8,
    pub title: String,
    pub min_rpm: u32,
    pub max_rpm: u32,
    pub rpm: Option<u32>,
    pub target: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct ThermalDigest {
    pub config: thermal::ThermalConfig,
    pub cur_max_freq: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct ProfilesDigest {
    pub current: String,
    pub choices: Vec<String>,
}

/// Only preference *kinds* — never user content (no named profiles, no
/// per-key colours).
#[derive(Debug, Serialize)]
pub struct SettingsDigest {
    pub lighting_mode: String,
    pub keyboard_layout: String,
    pub restore_on_launch: bool,
}

fn read_os_release() -> OsInfo {
    let raw = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let mut name = String::new();
    let mut version = String::new();
    for line in raw.lines() {
        if let Some(v) = line.strip_prefix("NAME=") {
            name = v.trim_matches('"').to_string();
        }
        if let Some(v) = line.strip_prefix("VERSION_ID=") {
            version = v.trim_matches('"').to_string();
        }
    }
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    OsInfo {
        distro: if name.is_empty() {
            "unknown".into()
        } else if version.is_empty() {
            name
        } else {
            format!("{name} {version}")
        },
        kernel,
    }
}

/// Collect the full anonymous report. Read-only, <200 ms typical.
pub fn collect() -> DiagnosticsReport {
    let s = sensors::read_all();
    let cfg = config::get();

    let battery_summary = BatterySummary {
        capacity_pct: battery::capacity(),
        status: battery::status(),
        voltage_v: battery::voltage(),
        cycles: battery::cycles(),
        health_pct: battery::health_pct(),
        charge_limit_pct: battery::charge_limit_pct(),
    };

    let mut fan_list = Vec::new();
    for f in fans::channels() {
        fan_list.push(FanLive {
            id: f.id,
            title: f.title,
            min_rpm: f.min_rpm,
            max_rpm: f.max_rpm,
            rpm: fans::read_rpm(f.id),
            target: fans::read_target(f.id),
        });
    }

    let thermal_digest = ThermalDigest {
        config: cfg.thermal.clone(),
        cur_max_freq: thermal::read_cur_max(),
    };

    let generated_at = chrono::Utc::now().to_rfc3339();

    DiagnosticsReport {
        schema_version: REPORT_SCHEMA_VERSION,
        generated_at,
        app_version: env!("CARGO_PKG_VERSION"),
        device: device::detect(),
        os: read_os_release(),
        sensors: s,
        battery: battery_summary,
        fans: fan_list,
        thermal: thermal_digest,
        profiles: ProfilesDigest {
            current: profile::current(),
            choices: profile::choices(),
        },
        curve_optimizer: undervolt::status(),
        settings: SettingsDigest {
            lighting_mode: cfg.lighting_mode.clone(),
            keyboard_layout: cfg.keyboard_layout.clone(),
            restore_on_launch: cfg.restore_on_launch,
        },
        daemon_log_tail: crate::logging::recent_logs_text(200),
        self_checks: run_self_checks(),
    }
}

/// Endpoint resolution: explicit override > configured endpoint > default.
pub fn resolve_endpoint(override_url: Option<&str>, cfg_endpoint: &str) -> String {
    override_url
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            if cfg_endpoint.is_empty() {
                None
            } else {
                Some(cfg_endpoint.to_string())
            }
        })
        .unwrap_or_else(|| DEFAULT_ENDPOINT.to_string())
}

/// POST the serialized report via curl. Returns the server response body
/// with the HTTP status prefixed on non-2xx.
pub fn send(report: &DiagnosticsReport, endpoint: &str) -> Result<String, String> {
    let json = serde_json::to_string(report).map_err(|e| format!("serialize: {e}"))?;

    let tmp = std::env::temp_dir().join(format!("legion-diag-{}.json", std::process::id()));
    {
        // 0600 so nobody on a multi-user box reads the payload mid-flight.
        let mut f = std::fs::File::create(&tmp)
            .and_then(|f| {
                use std::os::unix::fs::PermissionsExt;
                f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
                Ok(f)
            })
            .map_err(|e| format!("temp file: {e}"))?;
        f.write_all(json.as_bytes())
            .map_err(|e| format!("temp write: {e}"))?;
    }

    let result = std::process::Command::new("curl")
        .args([
            "-sS",
            "--max-time",
            "15",
            "-X",
            "POST",
            "-H",
            "Content-Type: application/json",
            "--data-binary",
            format!("@{}", tmp.display()).as_str(),
            "-w",
            "\n%{http_code}",
            endpoint,
        ])
        .output();

    let _ = std::fs::remove_file(&tmp);

    match result {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            let (body, code) = match text.rsplit_once('\n') {
                Some((b, c)) => (b.to_string(), c.trim().to_string()),
                None => (
                    text.to_string(),
                    format!("curl_exit_{}", out.status.code().unwrap_or(-1)),
                ),
            };
            if out.status.success() && code.starts_with('2') {
                Ok(body)
            } else {
                Err(format!(
                    "HTTP {code}: {}",
                    if body.is_empty() {
                        "no response body"
                    } else {
                        &*body
                    }
                ))
            }
        }
        Err(e) => Err(format!(
            "curl unavailable or failed ({e}) — install curl or send the dump manually"
        )),
    }
}

/// Convenience used by CLI/GUI: collect + send with config-resolved endpoint.
pub fn collect_and_send(override_url: Option<&str>) -> Result<String, String> {
    let cfg = config::get().diagnostics;
    let endpoint = resolve_endpoint(override_url, &cfg.endpoint);
    let report = collect();
    let resp = send(&report, &endpoint)?;
    config::update(|c| {
        c.diagnostics.last_sent = Some(chrono::Utc::now().to_rfc3339());
    });
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The privacy contract, enforced: whatever this machine's real
    /// hostname/username/MAC-like tokens are, they must not appear anywhere
    /// in the serialized report.
    #[test]
    fn collected_report_is_anonymous() {
        let report = collect();
        let json = serde_json::to_string(&report).expect("serializable");

        assert_eq!(report.schema_version, REPORT_SCHEMA_VERSION);
        assert!(!report.device.model.is_empty() || cfg!(not(target_os = "linux")));

        // Hostname must never appear (compare against live values if readable).
        for host_file in ["/etc/hostname", "/proc/sys/kernel/hostname"] {
            if let Ok(host) = std::fs::read_to_string(host_file) {
                let host = host.trim();
                if !host.is_empty() {
                    assert!(!json.contains(host), "leaked hostname {host:?}");
                }
            }
        }

        // Username from the environment must not appear.
        for var in ["USER", "LOGNAME", "SUDO_USER"] {
            if let Ok(user) = std::env::var(var) {
                assert!(!user.is_empty() && !json.contains(&user), "leaked username");
            }
        }

        // No MAC-address-shaped token anywhere.
        for chunk in json.split('"') {
            let colon_count = chunk.matches(':').count();
            if colon_count == 5 && chunk.len() == 17 {
                panic!("possible MAC address leaked: {chunk:?}");
            }
        }

        // Home directory paths must not leak.
        assert!(!json.contains("/home/"), "home path leaked");
    }

    #[test]
    fn endpoint_resolution_precedence() {
        assert_eq!(resolve_endpoint(Some("http://x"), ""), "http://x");
        assert_eq!(resolve_endpoint(None, "http://y"), "http://y");
        assert_eq!(resolve_endpoint(None, ""), DEFAULT_ENDPOINT);
        assert_eq!(resolve_endpoint(Some(""), "http://y"), "http://y");
    }
}
