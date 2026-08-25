//! Anonymous diagnostics dump (alpha) + opt-in transport.
//!
//! PRIVACY CONTRACT — enforced by the unit test in this file:
//! The report is built from a field whitelist; nothing identifying is ever
//! collected. Excluded by construction: hostname, username, serial numbers,
//! MAC addresses, IP addresses, disk serials, per-key colour maps, custom
//! user strings. Included: hardware model/type/BIOS/CPU/GPU/EC identity,
//! distro+kernel, sensor readings, battery health stats (no serial), fan
//! states, thermal/CO configuration, a small settings digest, the daemon log
//! tail (sanitized at write time and *additionally* passed through a
//! home-path redactor here — see `redact_home_paths`) and the self-check
//! results.
//!
//! Endpoints — one resolution chain over two defaults
//! ([`resolve_endpoint`]): explicit override > configured endpoint >
//! [`DEFAULT_WAN_ENDPOINT`].
//!
//! * WAN alpha default ([`DEFAULT_WAN_ENDPOINT`]): HTTPS; TLS is terminated
//!   at the operator's reverse proxy, which additionally enforces an
//!   optional shared secret delivered by [`send`] as an
//!   `X-Legion-Telemetry-Key` header (env `LEGION_TELEMETRY_KEY`).
//! * Tailscale dev ([`DEFAULT_TAILSCALE_ENDPOINT`]): legacy plain-HTTP VPS
//!   listener reachable only inside the tailnet; still selectable via
//!   override or configured endpoint for development.
//!
//! Transport shells out to `curl` (present on every supported distro) to
//! avoid adding an HTTPS dependency; the payload goes through a brand-new
//! 0600 temp file (`create_new` — refuses pre-existing paths and symlinks)
//! that is removed on every code path, and stale leftovers older than an
//! hour are swept at the start of [`collect`].

use crate::selftest::{run_self_checks, SelfCheck};
use crate::{battery, config, device, fans, profile, sensors, thermal, undervolt};
use serde::Serialize;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, SystemTime};

/// WAN alpha collector — PLACEHOLDER host until DNS/CDN is chosen; swap in
/// the real hostname here after deploy. Served over HTTPS: TLS terminates at
/// the operator's reverse proxy, which also validates the optional shared
/// secret sent by [`send`] (env `LEGION_TELEMETRY_KEY`).
pub const DEFAULT_WAN_ENDPOINT: &str = "https://legion-telemetry.example.com/v1/diagnostics";

/// Legacy/dev collector on the operator's IONOS VPS — plain HTTP, reachable
/// only inside the tailnet. Value frozen for existing dev setups; select it
/// via override or configured endpoint.
pub const DEFAULT_TAILSCALE_ENDPOINT: &str = "http://127.0.0.1:8787/v1/diagnostics";

pub const REPORT_SCHEMA_VERSION: u32 = 1;

/// Response-body echo cap for error strings (diagnosable, never verbatim-huge).
const MAX_ERR_BODY_CHARS: usize = 300;
/// Stderr echo cap for error strings.
const MAX_ERR_STDERR_CHARS: usize = 200;
/// Age at which a leftover temp payload counts as stale.
const PAYLOAD_STALE_AFTER: Duration = Duration::from_secs(3600);

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

/// PRIVACY (defence in depth): scrub home-directory paths from free-form
/// log text before it is embedded in the report. Warn sites like config.rs
/// embed `path.display()`, so a failed config write leaks `$HOME` into the
/// ring buffer. Collapses to `~`: the literal HOME dir of this process,
/// any `/home/<user>` prefix (other users included) and any
/// `/run/user/<uid>` prefix. Applied to the daemon log tail only — every
/// other field is anonymous by construction.
fn redact_home_paths(text: &str) -> String {
    let mut out = text.to_string();
    // Most specific first: this process's actual HOME.
    if let Ok(home) = std::env::var("HOME") {
        let home = home.trim_end_matches('/');
        if home.len() > 1 {
            out = out.replace(home, "~");
        }
    }
    // Generic prefixes last: cover foreign homes and XDG runtime dirs.
    let out = rewrite_prefix_to_tilde(&out, "/run/user/");
    rewrite_prefix_to_tilde(&out, "/home/")
}

/// True when the character terminates a home-path token.
fn is_path_boundary(c: char) -> bool {
    c.is_whitespace() || matches!(c, ':' | ';' | ',' | '"' | '\'' | ')' | ']' | '}')
}

/// Length of the leading single path component of `s`
/// (`"4242/legion.sock"` → 4, `"alice x"` → 5).
fn single_component_len(s: &str) -> usize {
    s.char_indices()
        .find(|(_, c)| *c == '/' || is_path_boundary(*c))
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

/// Replace every occurrence of `prefix` + one path component with `~`,
/// keeping any remainder (`"/run/user/4242/x.sock"` → `"~/x.sock"`, bare
/// trailing `"/home/"` → `"~"`). Collapsing even the empty case guarantees
/// the invariant "no /home/ substring ever reaches the report".
fn rewrite_prefix_to_tilde(text: &str, prefix: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find(prefix) {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + prefix.len()..];
        out.push('~');
        rest = &after[single_component_len(after)..];
    }
    out.push_str(rest);
    out
}

/// Collect the full anonymous report. Read-only on the system; runtime is
/// typically <1 s, worst case ~15 s (subprocess timeouts inside the
/// self-checks). Also sweeps stale temp payloads — see [`sweep_stale_temp`].
pub fn collect() -> DiagnosticsReport {
    sweep_stale_temp();
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
        daemon_log_tail: redact_home_paths(&crate::logging::recent_logs_text(200)),
        self_checks: run_self_checks(),
    }
}

/// Endpoint resolution: explicit override > configured endpoint > WAN
/// default ([`DEFAULT_WAN_ENDPOINT`]). Both override and configured value
/// are trimmed first; a whitespace-only string is treated as unset. The
/// legacy Tailscale listener remains reachable by pointing the override or
/// configured endpoint at [`DEFAULT_TAILSCALE_ENDPOINT`].
pub fn resolve_endpoint(override_url: Option<&str>, cfg_endpoint: &str) -> String {
    let from_env = std::env::var("LEGION_TELEMETRY_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let override_url = override_url.map(str::trim).filter(|s| !s.is_empty());
    let cfg_endpoint = cfg_endpoint.trim();
    override_url
        .map(str::to_string)
        .or(from_env)
        .or_else(|| {
            if cfg_endpoint.is_empty() {
                None
            } else {
                Some(cfg_endpoint.to_string())
            }
        })
        .unwrap_or_else(|| DEFAULT_WAN_ENDPOINT.to_string())
}

/// Bound a string to `max_chars` characters (UTF-8 safe, no suffix marker
/// so the cap is exact). Used for error-string echoes only.
fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        s.chars().take(max_chars).collect()
    }
}

/// Write the JSON payload to a brand-new 0600 temp file. `create_new` fails
/// on pre-existing files *and* on symlinks (O_EXCL semantics), so a planted
/// link can never be followed; the mode is applied at creation (no
/// world-readable window) and re-applied afterwards as a guard against
/// exotic umasks stripping owner bits. The file is removed again here on
/// every failure, so the caller only sees a path when it exists.
fn create_temp_payload(json: &str) -> Result<std::path::PathBuf, String> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    // pid alone could collide between daemon/CLI/GUI sending concurrently.
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let tmp = std::env::temp_dir().join(format!("legion-diag-{}-{nanos}.json", std::process::id()));

    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|e| format!("temp file {}: {e}", tmp.display()))?;
    if let Err(e) = f.set_permissions(std::fs::Permissions::from_mode(0o600)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("temp chmod {}: {e}", tmp.display()));
    }
    if let Err(e) = f.write_all(json.as_bytes()) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("temp write: {e}"));
    }
    Ok(tmp)
}

/// Optional shared secret for the WAN collector, read per-send from the
/// environment (`LEGION_TELEMETRY_KEY`). `None` when unset or empty. The
/// value must never appear in logs or error strings — callers pass it only
/// into [`build_curl_args`], which embeds it in a single header argument.
fn telemetry_key_from_env() -> Option<String> {
    std::env::var("LEGION_TELEMETRY_KEY")
        .ok()
        .filter(|k| !k.is_empty())
}

/// Pure builder for the curl argument list used by [`send`]; factored out so
/// it is unit-testable (spawning curl itself is not). `key`, when present
/// and non-empty after trimming, contributes exactly one `-H
/// X-Legion-Telemetry-Key: …` pair; without a key the pair is omitted and no
/// blank header is ever sent. The key appears only in that one argument.
fn build_curl_args(endpoint: &str, tmp_path: &str, key: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "-sS".to_string(),
        "--max-time".to_string(),
        "15".to_string(),
        "-X".to_string(),
        "POST".to_string(),
        "-H".to_string(),
        "Content-Type: application/json".to_string(),
    ];
    if let Some(key) = key.map(str::trim).filter(|k| !k.is_empty()) {
        args.push("-H".to_string());
        args.push(format!("X-Legion-Telemetry-Key: {key}"));
    }
    args.extend([
        "--data-binary".to_string(),
        format!("@{tmp_path}"),
        "-w".to_string(),
        "\n%{http_code}".to_string(),
        endpoint.to_string(),
    ]);
    args
}

/// POST the serialized report via curl. Returns the server response body on
/// 2xx. Every error carries its evidence: HTTP status (parsed numerically),
/// a response-body echo capped at `MAX_ERR_BODY_CHARS`, plus the curl exit
/// code and a trimmed stderr snippet (≤ `MAX_ERR_STDERR_CHARS`) whenever
/// curl itself failed — so transport problems are diagnosable from the
/// message alone. The temp payload is removed on every path. When
/// `LEGION_TELEMETRY_KEY` is set and non-empty, the request carries the
/// shared secret as an `X-Legion-Telemetry-Key` header (checked by the WAN
/// reverse proxy); the value itself never reaches any error message.
pub fn send(report: &DiagnosticsReport, endpoint: &str) -> Result<String, String> {
    let json = serde_json::to_string(report).map_err(|e| format!("serialize: {e}"))?;
    let tmp = create_temp_payload(&json)?;

    let outcome = (|| -> Result<String, String> {
        // Per-send env read: the secret flows into the argument list only
        // and is deliberately absent from every error string built below.
        let key = telemetry_key_from_env();
        let out = std::process::Command::new("curl")
            .args(build_curl_args(
                endpoint,
                &tmp.to_string_lossy(),
                key.as_deref(),
            ))
            .output()
            .map_err(|e| {
                format!(
                    "curl unavailable or failed to run ({e}) — install curl, \
                     or inspect locally with `legion-cli diagnose dump`"
                )
            })?;

        let exit_code = out.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let (body_raw, code_raw) = match stdout.rsplit_once('\n') {
            Some((b, c)) => (b.to_string(), c.trim().to_string()),
            None => (stdout.to_string(), String::new()),
        };
        // %{http_code} parsed numerically: unparsable output means curl died
        // before completing an HTTP round-trip (DNS, TLS, timeout, signal…).
        let http_code = code_raw.parse::<u16>().ok();
        let stderr_snip = truncate_chars(
            String::from_utf8_lossy(&out.stderr).trim(),
            MAX_ERR_STDERR_CHARS,
        );

        let transport_note = if out.status.success() {
            String::new()
        } else {
            let snip: &str = if stderr_snip.is_empty() {
                "<empty>"
            } else {
                stderr_snip.as_str()
            };
            format!(" [curl exit {exit_code}; stderr: {snip}]")
        };

        match (http_code, out.status.success()) {
            // Strict success: parsed 2xx AND clean curl exit (a timeout can
            // still emit a 2xx %{http_code} with a truncated body).
            (Some(code), true) if (200..300).contains(&code) => Ok(body_raw),
            _ => {
                let code_disp = match http_code {
                    Some(c) => c.to_string(),
                    None if code_raw.is_empty() => format!("curl_exit_{exit_code}"),
                    None => code_raw.clone(),
                };
                let body = body_raw.trim();
                Err(format!(
                    "HTTP {code_disp}: {}{transport_note}",
                    if body.is_empty() {
                        "no response body".to_string()
                    } else {
                        truncate_chars(body, MAX_ERR_BODY_CHARS)
                    }
                ))
            }
        }
    })();

    // Removed on EVERY path — success, HTTP error, curl failure.
    let _ = std::fs::remove_file(&tmp);
    outcome
}

/// Best-effort cleanup of `legion-diag-*.json` payload files older than one
/// hour in the system temp dir — leftovers from processes killed between
/// payload creation and the always-run removal in [`send`]. Called at the
/// top of [`collect`]; safe to call from anywhere, ignores all IO errors.
pub fn sweep_stale_temp() {
    sweep_older_than(
        &std::env::temp_dir(),
        SystemTime::now() - PAYLOAD_STALE_AFTER,
    );
}

fn sweep_older_than(dir: &Path, cutoff: SystemTime) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let is_payload = entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.starts_with("legion-diag-") && n.ends_with(".json"));
        if !is_payload {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let stale = meta.modified().map(|m| m <= cutoff).unwrap_or(false);
        if stale {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Opt-in state for diagnostics collection. GUI/background callers must
/// check this before sending anything autonomously; explicit sends go
/// through [`collect_and_send`], which treats the call itself as consent.
pub fn is_opted_in() -> bool {
    config::get().diagnostics.enabled
}

/// Convenience used by CLI/GUI: collect + send with the config-resolved
/// endpoint.
///
/// # Consent contract
///
/// Calling this function IS the consent. It deliberately does **not** gate
/// on [`is_opted_in`]: an explicit user action (button click, `legion-cli
/// diagnose send`) constitutes opt-in for that single send. Callers own the
/// consent decision — use [`is_opted_in`] only to decide whether automatic
/// or background sending may happen at all.
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
        // Idempotent bootstrap (OnceLock no-ops when already initialised);
        // init returns (), so a bare call is all we need.
        crate::logging::init("test");
        // Inject a config.rs-style warning that embeds $HOME plus generic
        // home/uid paths — exactly what a failed config write produces.
        let home = std::env::var("HOME").unwrap_or_else(|_| "/home/tester".to_string());
        log::warn!(
            "config save failed for {home}: fallback dump /home/redact-me/legion.conf \
             (socket /run/user/4242/legion.sock)"
        );

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

        // Home directory paths and XDG runtime dirs must not leak — the
        // injected warn line above is the canary riding in the log tail.
        assert!(!json.contains("/home/"), "home path leaked");
        assert!(!json.contains("/run/user/"), "XDG runtime dir leaked");
        if home.len() > 1 {
            assert!(!json.contains(&home), "raw HOME value leaked");
        }
        // The tail really made it through, redacted — not silently dropped.
        assert!(
            report.daemon_log_tail.contains("~/legion.conf")
                && report.daemon_log_tail.contains("~/legion.sock"),
            "injected markers missing/redacted wrongly: {:?}",
            report.daemon_log_tail
        );
    }

    #[test]
    fn endpoint_resolution_precedence() {
        assert_eq!(resolve_endpoint(Some("http://x"), ""), "http://x");
        assert_eq!(resolve_endpoint(None, "http://y"), "http://y");
        assert_eq!(resolve_endpoint(None, ""), DEFAULT_WAN_ENDPOINT);
        assert_eq!(resolve_endpoint(Some(""), "http://y"), "http://y");
    }

    /// Edge: a whitespace-only override is treated as unset (falls through
    /// to configured endpoint / default), and surrounding whitespace is
    /// trimmed off meaningful overrides.
    #[test]
    fn endpoint_resolution_whitespace_only_override_is_unset() {
        assert_eq!(resolve_endpoint(Some("   "), ""), DEFAULT_WAN_ENDPOINT);
        assert_eq!(resolve_endpoint(Some("\t\n "), "http://y"), "http://y");
        assert_eq!(resolve_endpoint(Some("  http://x  "), ""), "http://x");
        assert_eq!(
            resolve_endpoint(Some("  http://x  "), "http://y"),
            "http://x"
        );
    }

    /// The legacy/dev Tailscale endpoint must keep its historical value
    /// byte-for-byte so existing dev overrides/configs keep working.
    #[test]
    fn tailscale_default_endpoint_value_is_frozen() {
        assert_eq!(
            DEFAULT_TAILSCALE_ENDPOINT,
            "http://127.0.0.1:8787/v1/diagnostics"
        );
    }

    /// Argument builder without a secret: base flags retained, `@tmp`
    /// payload and trailing endpoint present, and no telemetry-key header.
    #[test]
    fn build_curl_args_without_key_omits_secret_header() {
        let args = build_curl_args("https://ep.example/v1/diagnostics", "/tmp/p.json", None);
        for flag in ["-sS", "--max-time", "15", "-X", "POST", "-w"] {
            assert!(args.iter().any(|a| a == flag), "flag {flag} missing");
        }
        assert!(args.contains(&"\n%{http_code}".to_string()));
        assert!(args
            .windows(2)
            .any(|w| w[0] == "-H" && w[1] == "Content-Type: application/json"));
        assert!(args
            .windows(2)
            .any(|w| w[0] == "--data-binary" && w[1] == "@/tmp/p.json"));
        assert_eq!(args.last().unwrap(), "https://ep.example/v1/diagnostics");
        assert!(
            !args.iter().any(|a| a.contains("X-Legion-Telemetry-Key")),
            "secret header without a key: {args:?}"
        );
    }

    /// Argument builder with a secret: exactly one `-H` pair carries the
    /// header, and the key appears in precisely one argument (nothing else
    /// echoes it — it must never leak into other args or error strings).
    #[test]
    fn build_curl_args_with_key_adds_single_header_pair() {
        let args = build_curl_args(
            "https://ep.example/v1/diagnostics",
            "/tmp/p.json",
            Some("s3cret"),
        );
        let header_positions: Vec<usize> = args
            .iter()
            .enumerate()
            .filter_map(|(i, a)| a.contains("X-Legion-Telemetry-Key").then_some(i))
            .collect();
        assert_eq!(
            header_positions.len(),
            1,
            "header not exactly once: {args:?}"
        );
        let pos = header_positions[0];
        assert_eq!(args[pos], "X-Legion-Telemetry-Key: s3cret");
        assert!(pos > 0 && args[pos - 1] == "-H", "header not in an -H pair");

        // Base shape intact around the inserted pair.
        for flag in ["-sS", "--max-time", "15", "-X", "POST", "-w"] {
            assert!(args.iter().any(|a| a == flag), "flag {flag} missing");
        }
        assert!(args
            .windows(2)
            .any(|w| w[0] == "--data-binary" && w[1] == "@/tmp/p.json"));
        assert_eq!(args.last().unwrap(), "https://ep.example/v1/diagnostics");
        // The key itself surfaces only inside that one header argument.
        assert_eq!(
            args.iter().filter(|a| a.contains("s3cret")).count(),
            1,
            "key echoed more than once: {args:?}"
        );
    }

    /// Blank secrets are dropped rather than sent as empty/blank headers.
    #[test]
    fn build_curl_args_drops_blank_keys() {
        for blank in [None, Some(""), Some("   ")] {
            let args = build_curl_args("https://ep/x", "/tmp/p", blank);
            assert!(!args.iter().any(|a| a.contains("X-Legion-Telemetry-Key")));
        }
    }

    /// Unit coverage for the log-tail redactor: literal $HOME, foreign
    /// `/home/<user>`, `/run/user/<uid>` and the bare-prefix case.
    #[test]
    fn redact_home_paths_scrubs_literal_home_user_and_uid_paths() {
        let home = std::env::var("HOME").unwrap_or_default();
        let mut input = String::from(
            "sock=/run/user/4242/legion.sock foreign=/home/alice/x bare=/home/ mid:\"/home/bob/y\"",
        );
        if home.len() > 1 {
            input.push_str(&format!(" own={}/cfg.toml", home.trim_end_matches('/')));
        }

        let out = redact_home_paths(&input);

        assert!(
            out.contains("sock=~/legion.sock"),
            "uid path not squashed: {out}"
        );
        assert!(
            out.contains("foreign=~/x"),
            "foreign home not squashed: {out}"
        );
        assert!(
            out.contains("mid:\"~/y\""),
            "quoted path not squashed: {out}"
        );
        assert!(out.contains("bare=~"), "bare prefix not collapsed: {out}");
        if home.len() > 1 {
            assert!(out.contains("own=~/cfg.toml"), "$HOME not squashed: {out}");
            assert!(!out.contains(&home), "raw HOME leaked: {out}");
        }
        assert!(!out.contains("/home/"), "/home/ survived: {out}");
        assert!(!out.contains("/run/user/"), "/run/user/ survived: {out}");
    }

    /// Error-echo caps: char-count bound, UTF-8 safe, passthrough below cap.
    #[test]
    fn truncate_chars_bounds_length_and_respects_utf8() {
        assert_eq!(truncate_chars("", 5), "");
        assert_eq!(truncate_chars("abc", 5), "abc");
        assert_eq!(truncate_chars("abcdef", 5).chars().count(), 5);
        let umlauts = "äöüäöü";
        assert_eq!(truncate_chars(umlauts, 3), "äöü");
        assert_eq!(truncate_chars(umlauts, 99), umlauts);
    }

    /// Sweeper removes only matching payload files past the cutoff; fresh
    /// files and non-.json bystanders survive.
    #[test]
    fn sweep_stale_temp_removes_only_old_payload_files() {
        let dir = std::env::temp_dir();
        let tag = std::process::id();
        let old = dir.join(format!("legion-diag-{tag}-sweep-old.json"));
        let fresh = dir.join(format!("legion-diag-{tag}-sweep-new.json"));
        let bystander = dir.join(format!("legion-diag-{tag}-sweep.txt"));
        std::fs::write(&old, "{}").unwrap();
        std::fs::write(&fresh, "{}").unwrap();
        std::fs::write(&bystander, "{}").unwrap();

        let now = SystemTime::now();
        // Cutoff an hour back → nothing is stale yet.
        sweep_older_than(&dir, now - Duration::from_secs(3600));
        assert!(old.exists(), "swept a fresh payload file");
        assert!(fresh.exists(), "swept a fresh payload file");

        // Cutoff in the future → every matching .json payload goes.
        sweep_older_than(&dir, now + Duration::from_secs(3600));
        assert!(!old.exists(), "stale payload survived sweep");
        assert!(!fresh.exists(), "future-cutoff sweep kept payload");
        assert!(bystander.exists(), "swept non-.json bystander");

        let _ = std::fs::remove_file(&bystander);
    }
}
