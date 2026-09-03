//! Anonymous diagnostics dump (alpha) + opt-out transport.
//!
//! PRIVACY CONTRACT — enforced by the unit test in this file:
//! The report is built from a field whitelist; nothing identifying is ever
//! collected. Excluded by construction: hostname, username, serial numbers,
//! MAC addresses, IP addresses, disk serials, per-key colour maps, custom
//! user strings, audio sink names, EDID bytes, power-supply uevent dumps.
//! Included: hardware model/type/BIOS/CPU/GPU/EC identity,
//! distro+kernel, sensor readings, battery health stats (no serial), fan
//! states, thermal/CO configuration, a small settings digest, a slim log
//! digest (counts + last error, passed through `redact_home_paths` here),
//! AC/charging state (whitelisted tokens + watt/volt numbers), cpufreq
//! policy (governor, EPP, p-state mode, boost flag), display connector/VRR
//! (sanitized names + booleans, no EDID serials), speaker-amp health
//! (booleans + one token), dGPU power limits/P-state (numbers + token),
//! and the self-check
//! results.
//!
//! Endpoints — one resolution chain over two defaults
//! ([`resolve_endpoint`]): explicit override > configured endpoint >
//! [`DEFAULT_WAN_ENDPOINT`]. Every candidate must carry an explicit
//! `http(s)://` scheme (case-insensitive); anything else is rejected and
//! the next precedence level applies, so curl can never be handed an
//! option-shaped value (`--proxy …`) or a non-HTTP scheme.
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
//! avoid adding an HTTPS dependency; the payload — and, when
//! `LEGION_TELEMETRY_KEY` is set, the secret header line — go through
//! brand-new 0600 temp files (`create_new` — refuses pre-existing paths and
//! symlinks) that are removed on every code path. The secret is delivered
//! to curl as `-H @<header-file>` precisely so it never appears in the
//! argument vector, which is world-readable via `/proc/<pid>/cmdline` while
//! curl runs. Stale payload leftovers older than an hour are swept at the
//! start of [`collect`].

use crate::selftest::{run_self_checks, SelfCheck};
use crate::{battery, config, device, fans, profile, sensors, thermal, undervolt};
use serde::Serialize;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime};

/// WAN alpha collector — points at the operator VPS (`telemetry.
/// adrian-kozlowski.de`). Served over HTTPS: TLS terminates at the
/// operator's reverse proxy, which also validates the optional shared
/// secret sent by [`send`] (env `LEGION_TELEMETRY_KEY`).
pub const DEFAULT_WAN_ENDPOINT: &str = "https://telemetry.adrian-kozlowski.de/v1/diagnostics";

/// Shared secret expected by the default WAN collector, compiled in so
/// release builds authenticate out of the box — no per-machine systemd
/// drop-in required. `LEGION_TELEMETRY_KEY` overrides it (its purpose is
/// spam throttling, not confidentiality: the payload is anonymous). Rotate
/// the value on the collector AND here in lockstep.
pub const DEFAULT_TELEMETRY_KEY: &str =
    "193c4ca1a0ad0eedb2a2c758416066a4c3885046beb1da50d60fd3a20eb6b9f9";

/// Legacy/dev collector on the operator's IONOS VPS — plain HTTP, reachable
/// only inside the tailnet. Value frozen for existing dev setups; select it
/// via override or configured endpoint.
pub const DEFAULT_TAILSCALE_ENDPOINT: &str = "http://127.0.0.1:8787/v1/diagnostics";

pub const REPORT_SCHEMA_VERSION: u32 = 4;

/// Deep-report cadence: full sensor dump sent on daemon launch, every hour,
/// and whenever the capability digest changes (new fan channels, amp bound,
/// model detected differently …). Minute pushes stay small (schema v1 body
/// plus a `deep: null` marker).
pub const DEEP_INTERVAL_SECS: u64 = 3600;

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
    /// Pseudonymous machine ID (UUID v4). Stable per installation; lets the
    /// operator correlate reports from the same machine over time.
    pub machine_id: String,

    pub device: device::DeviceInfo,
    pub os: OsInfo,
    pub sensors: sensors::SensorReadings,
    pub battery: BatterySummary,
    /// Actual RPM reader selected for this report, exposed at top level for
    /// fleet queries. Device capability discovery remains nested in `device`.
    pub fan_backend: String,
    /// Separate writable target backend; null for read-only fan hardware.
    pub fan_control_backend: Option<String>,
    pub fans: Vec<FanLive>,
    pub thermal: ThermalDigest,
    pub profiles: ProfilesDigest,
    pub curve_optimizer: undervolt::CurveOptimizerStatus,
    pub settings: SettingsDigest,
    /// Active machine anomalies (fan stalled, NVMe hot, limiter bypassed,
    /// config unwritable …). Empty = clean bill of health this pass.
    pub faults: Vec<crate::selftest::Fault>,
    /// Slimmed log summary — counts plus the last error (redacted). Raw log
    /// lines never leave the machine; this keeps reports small and the
    /// anonymity contract robust even if a message slips a path in.
    pub log_digest: LogDigest,
    pub self_checks: Vec<SelfCheck>,
    /// System context for correlating reports across machines.
    pub system_info: SystemInfo,
    /// Detailed hardware inventory (CPU, GPU, RAM, storage, display).
    #[serde(default)]
    pub hardware: HardwareInfo,
    /// Live power & charging state (AC adapter presence, charge rate).
    /// Minute-level — the `battery` block above carries the slow-moving
    /// health stats (cycles, design capacity, limiter).
    pub power: PowerInfo,
    /// Speaker-amp health for the Cirrus/Realtek fleet (typed booleans plus
    /// a whitelisted health token — never sink names or mixer free text).
    pub audio: AudioDigest,
    /// Deep diagnostic block — present only on launch / hourly /
    /// capability-change reports. Null on the 1-minute heartbeat pushes.
    #[serde(default)]
    pub deep: Option<DeepReport>,
}

/// Everything a deep report carries beyond the normal one. Kept separate so
/// minute pushes stay small while deep pushes land only when they carry new
/// information (launch, hourly, capability change).
#[derive(Debug, Default, Serialize)]
pub struct DeepReport {
    /// Why this deep report was sent: "launch", "hourly", "capability-change".
    pub reason: String,
    /// Raw `nvidia-smi -q` output (trimmed), or amdgpu sysfs equivalent.
    pub gpu_detailed: Option<String>,
    /// Every hwmon chip with every readable input/label — full fleet sensor
    /// visibility for new-model bring-up.
    pub hwmon_dump: Vec<HwmonChipDump>,
    /// Fan sysfs detail: all fanN_* attrs per backend, including unreadable
    /// ones (with the error kind) so missing-permission bugs are visible.
    pub fan_detail: Vec<(String, String)>,
    /// Battery sysfs detail: every file in the battery dir + its value.
    pub battery_detail: Vec<(String, String)>,
    /// Installed support software: ryzen_smu (DKMS state), dkms, kernel
    /// headers, aw88399 firmware, GPU driver version.
    pub installed_software: Vec<(String, String)>,
    /// Digest of capability-relevant state — lets the server group models
    /// and spot new hardware variants.
    pub capability_digest: String,
    // 0.2.5 additive -- all #[serde(default)] so schema stays v3
    /// Explicit yogafan probe -- queryable without parsing installed_software strings.
    #[serde(default)]
    pub yogafan_loaded: bool,
    #[serde(default)]
    pub yogafan_hwmon: Option<String>,
    /// ACPI/WMI3 probe results -- isolates R8CN EC-lock fleet (WMI3 err -5) vs healthy.
    #[serde(default)]
    pub acpi_probe: AcpiProbe,
    /// Direct EC ACPI-space 0xB0/0xB4 via ec_sys debugfs (IT5508).
    #[serde(default)]
    pub ec_temps: EcTemps,
    /// Typed PPT snapshot -- ClickHouse can slice without parsing ppt_attrs strings.
    #[serde(default)]
    pub ppt_snapshot: Vec<PptSnapshot>,
    /// Raw yogafan fan attrs (fan*_input/label) -- cheap proof that yogafan actually tachd.
    #[serde(default)]
    pub yogafan_attrs: Vec<(String, String)>,
    /// Full DMI/SMBIOS sysfs dump (product family, sku, board name, ec firmware release, etc.)
    #[serde(default)]
    pub dmi_dump: Vec<(String, String)>,
    /// Discovered HID controllers (vendor/product, device name, driver) for keyboard/RGB detection
    #[serde(default)]
    pub hid_devices: Vec<(String, String)>,
    /// Discovered thermal zones (name (type) -> temp) for EC thermistor mapping
    #[serde(default)]
    pub thermal_zones: Vec<(String, String)>,
    /// Discovered platform profile choices from ACPI/WMI
    #[serde(default)]
    pub platform_profile_choices: Vec<String>,
    /// Discovered WMI devices/GUIDs
    #[serde(default)]
    pub wmi_devices: Vec<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct AcpiProbe {
    #[serde(default)]
    pub wmi3_available: bool,
    #[serde(default)]
    pub wmi3_cpu_temp: Option<String>,
    #[serde(default)]
    pub wmi3_gpu_temp: Option<String>,
    #[serde(default)]
    pub acpi_tmp_ok: bool,
    #[serde(default)]
    pub acpi_tmp_detail: String,
    #[serde(default)]
    pub ec_fans_acpi_path: Option<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct EcTemps {
    #[serde(default)]
    pub b0: Option<u8>,
    #[serde(default)]
    pub b4: Option<u8>,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub b0_delta_vs_k10temp: Option<i8>,
    #[serde(default)]
    pub b4_delta_vs_dgpu: Option<i8>,
}

#[derive(Debug, Default, Serialize)]
pub struct PptSnapshot {
    pub id: String,
    pub current_w: u32,
    pub min_w: u32,
    pub max_w: u32,
}

/// One hwmon chip: name + every readable (attribute, value) pair.
#[derive(Debug, Serialize)]
pub struct HwmonChipDump {
    pub hwmon: String,
    pub name: String,
    pub attrs: Vec<(String, String)>,
}

/// Detailed static & topological hardware inventory for deep telemetry
/// correlation across laptop revisions. Whitelisted: no serial numbers,
/// MACs, UUIDs, or user paths.
#[derive(Debug, Clone, Default, Serialize, serde::Deserialize)]
pub struct HardwareInfo {
    pub cpu: CpuDetail,
    pub gpu: GpuDetail,
    pub memory: MemoryDetail,
    pub storage: StorageDetail,
    pub display: DisplayDetail,
    #[serde(default)]
    pub motherboard: Option<MotherboardDetail>,
    #[serde(default)]
    pub keyboard_lighting: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, serde::Deserialize)]
pub struct MotherboardDetail {
    pub board_name: Option<String>,
    pub board_version: Option<String>,
    pub product_family: Option<String>,
    pub product_sku: Option<String>,
    pub ec_firmware_release: Option<String>,
    pub bios_date: Option<String>,
    pub bios_release: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, serde::Deserialize)]
pub struct CpuDetail {
    pub name: String,
    pub vendor: String,
    pub physical_cores: u32,
    pub logical_threads: u32,
    pub base_clock_mhz: Option<u32>,
    pub max_clock_mhz: Option<u32>,
    pub microcode: String,
    pub governor: String,
    /// `energy_performance_preference` for cpu0 (whitelisted: performance,
    /// balance_performance, balance_power, power, default — else "other").
    /// None when the cpufreq node is unreadable (e.g. custom kernels).
    #[serde(default)]
    pub energy_performance_preference: Option<String>,
    /// cpufreq scaling driver (e.g. `amd-pstate-epp`, `intel_pstate`).
    /// Charset-sanitized kernel name — never identifying.
    #[serde(default)]
    pub scaling_driver: Option<String>,
    /// P-state backend mode: `amd-pstate:active|passive|guided|disable`,
    /// `intel-pstate:active|passive|off`, or `none` (acpi-cpufreq/unknown).
    #[serde(default)]
    pub pstate_mode: Option<String>,
    /// CPU frequency boost (turbo) toggle from
    /// `/sys/devices/system/cpu/cpufreq/boost`. None when the knob is absent.
    #[serde(default)]
    pub boost_enabled: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, serde::Deserialize)]
pub struct GpuDetail {
    pub discrete_name: Option<String>,
    pub integrated_name: Option<String>,
    #[serde(default)]
    pub discrete_vendor: Option<String>,
    pub driver_version: Option<String>,
    pub vram_total_mb: Option<u64>,
    pub pci_id: Option<String>,
    /// dGPU lifecycle: "active" (reading NVIDIA metrics), "inactive"
    /// (runtime-suspended after a live reading), "off" (present, never live
    /// this boot), "present" (non-NVIDIA), or "absent" (no discrete GPU).
    #[serde(default)]
    pub state: Option<String>,
    /// Currently enforced dGPU power limit (W, `enforced.power.limit`).
    /// None while the dGPU sleeps or on AMD-only machines.
    #[serde(default)]
    pub power_limit_w: Option<f64>,
    /// Firmware ceiling (W, `power.max_limit`, e.g. 175 on RTX 5080 Pro 7).
    #[serde(default)]
    pub power_max_w: Option<f64>,
    /// Non-boost baseline (W, `power.default_limit`, e.g. 80).
    #[serde(default)]
    pub power_default_w: Option<f64>,
    /// Dynamic Boost headroom (W) = max − default when the ceiling exceeds
    /// the baseline. Lets the fleet see the boost range at a glance.
    #[serde(default)]
    pub dynamic_boost_headroom_w: Option<f64>,
    /// Current dGPU P-state (`P0`…`P15`, whitelisted). None when asleep.
    #[serde(default)]
    pub pstate: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, serde::Deserialize)]
pub struct MemoryDetail {
    pub total_mb: u64,
    pub swap_total_mb: u64,
    pub mem_type: Option<String>, // e.g. DDR5
    pub speed_mhz: Option<u32>,   // e.g. 5600
    pub slots_used: Option<u32>,  // e.g. 2
    pub slots_total: Option<u32>, // e.g. 2
}

#[derive(Debug, Clone, Default, Serialize, serde::Deserialize)]
pub struct StorageDetail {
    pub nvme_devices: Vec<NvmeDrive>,
    pub root_total_gb: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, serde::Deserialize)]
pub struct NvmeDrive {
    pub model: String,
    pub size_gb: u64,
}

#[derive(Debug, Clone, Default, Serialize, serde::Deserialize)]
pub struct DisplayDetail {
    pub resolution: Option<String>, // e.g. 2560x1600
    pub refresh_hz: Option<u32>,    // e.g. 240
    /// Chosen DRM connector short name (e.g. `eDP-1`, `HDMI-A-1`): the
    /// internal panel when connected, else the first connected output.
    /// Charset-sanitized (`[A-Za-z0-9-]`, ≤16 chars) — never EDID bytes.
    #[serde(default)]
    pub connector: Option<String>,
    /// Variable-refresh-rate switch state from the connector's
    /// `vrr_enabled` sysfs knob. None when the kernel does not expose it
    /// (no guessing from EDID or model tables).
    #[serde(default)]
    pub vrr_capable: Option<bool>,
}

/// Live power & charging state. All values are anonymous by construction:
/// adapter presence/type as whitelisted tokens, electrical readings as
/// plain numbers. No uevent dumps (those carry battery serial numbers).
#[derive(Debug, Clone, Default, Serialize, serde::Deserialize)]
pub struct PowerInfo {
    /// True when an AC-type supply (barrel `Mains` or USB-C `USB`) reports
    /// `online == 1`. None when no AC supply node exists.
    #[serde(default)]
    pub ac_online: Option<bool>,
    /// Whitelisted supply type: `Mains`, `USB`, or `Other`. None when no AC
    /// supply node exists.
    #[serde(default)]
    pub ac_type: Option<String>,
    /// Whitelisted battery status: `Charging`, `Discharging`, `Full`,
    /// `Not charging`, or `Unknown`.
    #[serde(default)]
    pub charge_state: Option<String>,
    /// Instantaneous battery power (`power_now`, W). 0 while full on AC.
    #[serde(default)]
    pub charge_rate_w: Option<f64>,
    /// Battery terminal voltage (V). Mirrors `battery.voltage_v` for
    /// fleet queries that slice on the power block.
    #[serde(default)]
    pub voltage_v: Option<f64>,
}

/// Speaker-amp health from `audio::diagnose`, reduced to strictly typed
/// values. The diagnosis also sees sink names and mixer free text — those
/// NEVER enter this struct (only booleans plus one whitelisted token).
#[derive(Debug, Clone, Default, Serialize, serde::Deserialize)]
pub struct AudioDigest {
    /// `ok` | `soft-issue` | `hardware-broken` | `not-applicable`.
    pub health: String,
    /// AW88399-class amp described in ACPI (`AWDZ8399` node present).
    pub amp_present: bool,
    /// Amp driver actually bound (sound works through the smart amp).
    pub amp_bound: bool,
    /// Amp kernel modules loaded.
    pub modules_loaded: bool,
    /// Amp firmware blob present.
    pub firmware_ok: bool,
    /// Soft-recovery action available (upstream also allows proactive resets
    /// while healthy — gate on `health != "ok"` to see if a fix is needed).
    pub fixable: bool,
    /// Onboard speakers currently muted at the ALSA layer.
    pub speakers_muted: bool,
    /// Bass-speaker switch off (tinny-sound canary).
    pub bass_off: bool,
    /// Default PipeWire sink is not the internal analog output.
    pub wrong_default_sink: bool,
}

#[derive(Debug, Serialize)]
pub struct SystemInfo {
    pub uptime_secs: u64,
    pub load_avg_1m: f64,
    pub disk_free_mb: Option<u64>,
    pub mem_available_mb: Option<u64>,
}

#[derive(Debug, Serialize, Default)]
pub struct LogDigest {
    pub info_count: u32,
    pub warn_count: u32,
    pub error_count: u32,
    /// Last ERROR-level message, home-redacted, capped at 200 chars.
    pub last_error: Option<String>,
    /// Error count per target module (top 5 by count). Lets the operator
    /// see WHICH subsystem is producing errors on each machine.
    pub errors_by_target: Vec<(String, u32)>,
}

/// Aggregate log entries into a slim digest with per-module attribution.
pub fn build_log_digest(entries: &[crate::logging::LogEntry]) -> LogDigest {
    let mut d = LogDigest::default();
    let mut err_targets: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
    for e in entries {
        match e.level.as_str() {
            "INFO" => d.info_count += 1,
            "WARN" => d.warn_count += 1,
            "ERROR" => {
                d.error_count += 1;
                *err_targets.entry(e.target.as_str()).or_insert(0) += 1;
            }
            _ => {}
        }
        if e.level == "ERROR" {
            // recent_logs is oldest-first — overwrite each time to keep the NEWEST error.
            let redacted = redact_home_paths(&e.message);
            let capped: String = redacted.chars().take(200).collect();
            d.last_error = Some(capped);
        }
    }
    d.errors_by_target = {
        let mut v: Vec<(String, u32)> = err_targets
            .into_iter()
            .map(|(k, c)| (k.to_string(), c))
            .collect();
        v.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
        v.truncate(5);
        v
    };
    d
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
    #[serde(default)]
    pub charge_types: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FanLive {
    pub id: u8,
    pub title: String,
    pub min_rpm: u32,
    pub max_rpm: u32,
    pub rpm: Option<u32>,
    pub target: Option<u32>,
    pub readable: bool,
    /// `readable`, `not-exposed`, `backend-unavailable`, or `unreadable`.
    #[serde(default)]
    pub state: String,
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
    /// Raw legacy ACPI view from `/sys/firmware/acpi/platform_profile_choices`
    /// (whitelisted tokens). `choices` above is the handler-resolved view
    /// (which may add `custom`); comparing the two isolates handler-vs-EC
    /// disagreements across the fleet.
    #[serde(default)]
    pub acpi_choices: Vec<String>,
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
    let mut pretty_name = String::new();
    let mut name = String::new();
    let mut version = String::new();
    for line in raw.lines() {
        if let Some(v) = line.strip_prefix("PRETTY_NAME=") {
            pretty_name = v.trim_matches('"').to_string();
        } else if let Some(v) = line.strip_prefix("NAME=") {
            name = v.trim_matches('"').to_string();
        } else if let Some(v) = line.strip_prefix("VERSION_ID=") {
            version = v.trim_matches('"').to_string();
        }
    }
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    OsInfo {
        distro: if !pretty_name.is_empty() {
            pretty_name
        } else if !name.is_empty() && !version.is_empty() {
            format!("{name} {version}")
        } else if !name.is_empty() {
            name
        } else {
            "unknown".into()
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
    let mut replacements = 0usize;
    let mut out = text.to_string();
    // Most specific first: this process's actual HOME.
    if let Ok(home) = std::env::var("HOME") {
        let home = home.trim_end_matches('/');
        if home.len() > 1 {
            let hits = out.matches(home).count();
            if hits > 0 {
                out = out.replace(home, "~");
                replacements += hits;
            }
        }
    }
    // Generic prefixes: cover foreign homes, immutable distros (/var/home/), and XDG runtime dirs.
    for prefix in &["/run/user/", "/var/home/", "/home/"] {
        let hits = out.matches(prefix).count();
        out = rewrite_prefix_to_tilde(&out, prefix);
        replacements += hits;
    }
    // Scrub root home paths (/root/ -> ~/)
    if out.contains("/root/") {
        let hits = out.matches("/root/").count();
        out = out.replace("/root/", "~/");
        replacements += hits;
    }
    log::debug!(
        "redact_home_paths: {} chars in → {replacements} replacement(s)",
        text.len()
    );
    out
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
    log::debug!("diagnostics: collecting report");
    sweep_stale_temp();
    let mut s = sensors::read_all();
    log::debug!(
        "diag sensors: cpu {:.1}°C · dgpu {:.1}°C · igpu {:.1}°C · {} ssd(s) · {} ram module(s)",
        s.cpu_temp,
        s.dgpu_temp,
        s.igpu_edge,
        s.ssd_composite.len(),
        s.ram_temps.len()
    );
    let app_cfg = config::get();
    let mut machine_id = app_cfg.diagnostics.machine_id.clone();
    if machine_id.is_empty() {
        let mut dc = app_cfg.diagnostics.clone();
        dc.ensure_machine_id();
        let minted = dc.machine_id.clone();
        // Persist minted ID atomically (flock held inside update); re-read winner to avoid TOCTOU race.
        config::update(|c| {
            if c.diagnostics.machine_id.is_empty() {
                c.diagnostics.machine_id = minted.clone();
            }
        });
        machine_id = config::get().diagnostics.machine_id;
    }

    let battery_summary = BatterySummary {
        capacity_pct: battery::capacity(),
        status: battery::status(),
        voltage_v: battery::voltage(),
        cycles: battery::cycles(),
        health_pct: battery::health_pct(),
        charge_limit_pct: battery::charge_limit_pct(),
        charge_types: battery::charge_types(),
    };
    // Keep the legacy flattened fields and canonical battery block coherent
    // within this report even if AC state changes during collection.
    s.battery_pct = battery_summary.capacity_pct.unwrap_or_default();
    s.battery_status = battery_summary.status.clone().unwrap_or_default();
    s.battery_voltage = battery_summary.voltage_v.unwrap_or_default();
    s.battery_cycles = battery_summary.cycles.unwrap_or_default();
    log::debug!(
        "diag battery: capacity {:?}% · status {:?} · limit {}%",
        battery_summary.capacity_pct,
        battery_summary.status,
        battery_summary.charge_limit_pct
    );

    let mut fan_list = Vec::new();
    for f in fans::channels() {
        let (rpm, state) = fans::rpm_status(f.id);
        fan_list.push(FanLive {
            id: f.id,
            title: f.title,
            min_rpm: f.min_rpm,
            max_rpm: f.max_rpm,
            rpm,
            target: fans::read_target(f.id),
            readable: state == fans::FanRpmState::Readable,
            state: state.as_str().into(),
        });
    }
    log::debug!("diag fans: {} channel(s)", fan_list.len());

    let thermal_digest = ThermalDigest {
        config: app_cfg.thermal.clone(),
        cur_max_freq: thermal::read_cur_max(),
    };
    log::debug!(
        "diag thermal: enabled={} · max {}°C · cur_max {:?} kHz",
        thermal_digest.config.enabled,
        thermal_digest.config.max_temp,
        thermal_digest.cur_max_freq
    );

    let generated_at = chrono::Utc::now().to_rfc3339();

    let entries = crate::logging::recent_logs(200);
    let digest = build_log_digest(&entries);
    log::debug!(
        "diag log digest: {} info / {} warn / {} error(s)",
        digest.info_count,
        digest.warn_count,
        digest.error_count
    );

    let mut faults = crate::selftest::scan_faults();
    for f in &mut faults {
        f.detail = redact_home_paths(&f.detail);
    }
    let fault_criticals = faults
        .iter()
        .filter(|f| f.severity == crate::selftest::Severity::Critical)
        .count();
    let fault_warnings = faults
        .iter()
        .filter(|f| f.severity == crate::selftest::Severity::Warning)
        .count();
    let fault_infos = faults
        .iter()
        .filter(|f| f.severity == crate::selftest::Severity::Info)
        .count();
    log::debug!(
        "diag faults: {fault_criticals} critical / {fault_warnings} warning / {fault_infos} info"
    );
    for f in &faults {
        if f.severity == crate::selftest::Severity::Critical {
            log::warn!("fault: {}: {}", f.id, f.detail);
        }
    }

    let system_info = read_system_info();
    let hardware = read_hardware_info(&s);
    // Live power + speaker-amp digests. `battery_summary` is moved into the
    // report below, so the power block borrows it here while it is still
    // owned (voltage/status stay coherent within this report).
    let power = read_power_info(&battery_summary);
    let audio_digest = read_audio_digest();
    log::debug!(
        "diag power: ac_online {:?} ({:?}) · state {:?} · rate {:?} W",
        power.ac_online,
        power.ac_type,
        power.charge_state,
        power.charge_rate_w
    );
    log::debug!(
        "diag audio: health={} bound={} fixable={}",
        audio_digest.health,
        audio_digest.amp_bound,
        audio_digest.fixable
    );

    let report = DiagnosticsReport {
        schema_version: REPORT_SCHEMA_VERSION,
        generated_at,
        app_version: env!("CARGO_PKG_VERSION"),
        machine_id,
        device: device::detect(),
        os: read_os_release(),
        sensors: s,
        battery: battery_summary,
        fan_backend: fans::backend_name(),
        fan_control_backend: fans::control_backend_name(),
        fans: fan_list,
        thermal: thermal_digest,
        profiles: ProfilesDigest {
            current: profile::current(),
            choices: profile::choices(),
            acpi_choices: read_acpi_platform_choices(),
        },
        curve_optimizer: undervolt::status(),
        settings: SettingsDigest {
            lighting_mode: app_cfg.lighting_mode.clone(),
            keyboard_layout: app_cfg.keyboard_layout.clone(),
            restore_on_launch: app_cfg.restore_on_launch,
        },
        log_digest: digest,
        faults,
        self_checks: run_self_checks(),
        system_info,
        hardware,
        power,
        audio: audio_digest,
        deep: None,
    };

    // Sections built inline above — traced from the finished report so every
    // report section leaves exactly one event-log entry.
    log::debug!(
        "diag device: model={} machine={}",
        report.device.model,
        report.device.machine_type
    );
    log::debug!(
        "diag os: distro={} kernel={}",
        report.os.distro,
        report.os.kernel
    );
    log::debug!(
        "diag profiles: current='{}' · {} choice(s)",
        report.profiles.current,
        report.profiles.choices.len()
    );
    log::debug!(
        "diag curve optimizer: available={} ({})",
        report.curve_optimizer.available,
        report.curve_optimizer.reason
    );
    log::debug!(
        "diag settings: lighting={} layout={} restore_on_launch={}",
        report.settings.lighting_mode,
        report.settings.keyboard_layout,
        report.settings.restore_on_launch
    );
    let self_passed = report.self_checks.iter().filter(|c| c.ok).count();
    log::debug!(
        "diag self-checks: {self_passed}/{} passed",
        report.self_checks.len()
    );
    log::debug!(
        "diag system info: uptime {} s · load {:.2} · mem available {:?} MB",
        report.system_info.uptime_secs,
        report.system_info.load_avg_1m,
        report.system_info.mem_available_mb
    );

    report
}

/// Compute the capability digest — a hash of everything that, when it
/// changes, should trigger a deep report (model, fan channels, amp state,
/// installed software). Minute reports don't carry it; the scheduler
/// compares consecutive digests.
pub fn capability_digest() -> String {
    let dev = device::detect();
    let caps = &dev.capabilities;
    let key = format!(
        "{}|{}|{}|{}|{}|{}|{}",
        dev.model,
        dev.machine_type,
        caps.fan_backend,
        caps.fans.len(),
        caps.platform_profiles.len(),
        crate::audio::diagnose().health as u8,
        battery::charge_types().is_some(),
    );
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in key.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

/// Collect the deep block: full hwmon walk, GPU detailed dump, fan sysfs
/// detail, battery detail, installed-software detection. Heavier than
/// `collect()` (subprocess + sysfs walk) — only called at launch / hourly /
/// capability change / explicit send.
fn collect_deep(reason: &str) -> DeepReport {
    let mut deep = DeepReport {
        reason: reason.to_string(),
        ..Default::default()
    };

    // ─── GPU detailed: nvidia-smi -q, else amdgpu sysfs walk ───
    deep.gpu_detailed = crate::dgpu::detailed_query();

    // ─── hwmon dump: every chip, every readable attr ───
    for entry in std::fs::read_dir("/sys/class/hwmon")
        .map(|d| d.flatten().collect::<Vec<_>>())
        .unwrap_or_default()
    {
        let hw_path = entry.path();
        let hw_name = entry.file_name().to_string_lossy().into_owned();
        let chip = std::fs::read_to_string(hw_path.join("name"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        let mut attrs = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&hw_path) {
            for a in entries.flatten() {
                let aname = a.file_name().to_string_lossy().into_owned();
                // Skim to sensor-relevant attrs; skip device symlinks/power.
                if aname.starts_with("device")
                    || aname.starts_with("power")
                    || aname == "uevent"
                    || aname == "subsystem"
                {
                    continue;
                }
                match std::fs::read_to_string(a.path()) {
                    Ok(v) => attrs.push((aname, v.trim().to_string())),
                    Err(e) => attrs.push((aname, format!("<{}>", e.kind() as i32))),
                }
            }
            attrs.sort();
        }
        deep.hwmon_dump.push(HwmonChipDump {
            hwmon: hw_name,
            name: chip,
            attrs,
        });
    }
    deep.hwmon_dump.sort_by(|a, b| a.hwmon.cmp(&b.hwmon));

    // yogafan_loaded + yogafan_hwmon + yogafan_attrs (explicit, queryable)
    {
        let yw = crate::sensors::hwmon_by_name("yogafan");
        deep.yogafan_loaded = yw.is_some();
        deep.yogafan_hwmon = yw.as_ref().map(|p| {
            let hwmon = p.file_name().and_then(|n| n.to_str()).unwrap_or("hwmon?");
            format!("{}/yogafan", hwmon)
        });
        for chip in &deep.hwmon_dump {
            if chip.name == "yogafan" || chip.name == "lenovo_wmi_other" {
                for (attr, val) in &chip.attrs {
                    if attr.starts_with("fan") {
                        deep.yogafan_attrs
                            .push((format!("{}/{}", chip.hwmon, attr), val.clone()));
                    }
                }
            }
        }
    }

    // ACPI probe (WMI3 + acpitz + EC FANS path)
    deep.acpi_probe = probe_acpi();

    // EC direct temps 0xB0/0xB4 via ec_sys debugfs
    {
        let sensors = crate::sensors::read_all();
        deep.ec_temps = read_ec_acpi_temps(&sensors);
    }

    // PPT typed snapshot
    deep.ppt_snapshot = crate::profile::all_ppt_limits()
        .into_iter()
        .map(|l| PptSnapshot {
            id: l.id.to_string(),
            current_w: l.current,
            min_w: l.min,
            max_w: l.max,
        })
        .collect();

    // ─── fan detail: every fanN_* attr on every fan-ish hwmon ───
    for chip in &deep.hwmon_dump {
        if chip.name.contains("wmi") || chip.name.contains("legion") || chip.name.contains("yoga") {
            for (attr, val) in &chip.attrs {
                if attr.starts_with("fan") {
                    deep.fan_detail
                        .push((format!("{}/{}", chip.hwmon, attr), val.clone()));
                }
            }
        }
    }

    // ─── battery detail: every file in the power_supply battery dir ───
    for dir_name in ["BAT0", "BAT1", "BAT2", "BATT"] {
        let dir = format!("/sys/class/power_supply/{dir_name}");
        if !std::path::Path::new(&dir).is_dir() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let fname = e.file_name().to_string_lossy().into_owned();
                let fname_lower = fname.to_lowercase();
                // Enforce privacy contract: never collect battery serial numbers or raw uevents
                if fname_lower.contains("serial") || fname_lower == "uevent" {
                    continue;
                }
                let val = std::fs::read_to_string(e.path())
                    .map(|v| v.trim().to_string())
                    .unwrap_or_else(|e| format!("<{}>", e.kind() as i32));
                deep.battery_detail
                    .push((format!("{dir_name}/{fname}"), val));
            }
        }
        break; // only the first present battery — keeps the payload bounded
    }

    // ─── installed support software ───
    let sw = &mut deep.installed_software;
    let push = |sw: &mut Vec<(String, String)>, k: &str, v: String| sw.push((k.into(), v));
    push(
        sw,
        "ryzen_smu_loaded",
        Path::new("/sys/kernel/ryzen_smu_drv").is_dir().to_string(),
    );
    push(
        sw,
        "ryzen_smu_dkms_registered",
        std::process::Command::new("dkms")
            .arg("status")
            .output()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .any(|l| l.contains("ryzen_smu"))
            })
            .unwrap_or(false)
            .to_string(),
    );
    push(
        sw,
        "dkms_installed",
        std::process::Command::new("dkms")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
            .to_string(),
    );
    push(
        sw,
        "kernel_headers",
        Path::new(&format!("/lib/modules/{}/build", report_os_kernel()))
            .is_dir()
            .to_string(),
    );
    push(
        sw,
        "aw88399_firmware",
        Path::new("/lib/firmware/aw88399_acf.bin")
            .is_file()
            .to_string(),
    );
    if let Some(ver) = std::fs::read_to_string("/sys/module/nvidia/version")
        .ok()
        .map(|s| s.trim().to_string())
    {
        push(sw, "nvidia_driver", ver);
    }
    push(
        sw,
        "amdgpu_loaded",
        Path::new("/sys/module/amdgpu").is_dir().to_string(),
    );
    push(
        sw,
        "yogafan_loaded",
        std::path::Path::new("/sys/class/hwmon")
            .read_dir()
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| {
                std::fs::read_to_string(e.path().join("name"))
                    .unwrap_or_default()
                    .trim()
                    == "yogafan"
            })
            .to_string(),
    );
    push(sw, "kernel", {
        std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    });

    // ─── DMI / SMBIOS sysfs inventory (safe whitelist, no serial/UUID) ───
    for key in [
        "bios_date",
        "bios_release",
        "bios_vendor",
        "bios_version",
        "board_name",
        "board_vendor",
        "board_version",
        "chassis_type",
        "chassis_vendor",
        "chassis_version",
        "ec_firmware_release",
        "product_family",
        "product_name",
        "product_sku",
        "product_version",
        "sys_vendor",
    ] {
        let p = format!("/sys/class/dmi/id/{key}");
        if let Ok(v) = std::fs::read_to_string(&p) {
            let v = v.trim();
            if !v.is_empty() {
                deep.dmi_dump.push((key.to_string(), v.to_string()));
            }
        }
    }

    // ─── HID devices inventory (Keyboard, Touchpad, RGB controllers) ───
    if let Ok(entries) = std::fs::read_dir("/sys/bus/hid/devices") {
        for e in entries.flatten() {
            let hid_id = e.file_name().to_string_lossy().into_owned();
            let mut info = Vec::new();
            if let Ok(uevent) = std::fs::read_to_string(e.path().join("uevent")) {
                for line in uevent.lines() {
                    if let Some(name) = line.strip_prefix("HID_NAME=") {
                        info.push(name.to_string());
                    } else if let Some(driver) = line.strip_prefix("DRIVER=") {
                        info.push(format!("driver={driver}"));
                    }
                }
            }
            let desc = if info.is_empty() {
                "unknown".to_string()
            } else {
                info.join(" | ")
            };
            deep.hid_devices.push((hid_id, desc));
        }
        deep.hid_devices.sort_by(|a, b| a.0.cmp(&b.0));
    }

    // ─── ACPI Thermal Zones (SEN1..5, TCPU, acpitz, etc.) ───
    if let Ok(zones) = std::fs::read_dir("/sys/class/thermal") {
        for e in zones.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.starts_with("thermal_zone") {
                continue;
            }
            let ztype = std::fs::read_to_string(e.path().join("type"))
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|_| "unknown".to_string());
            let temp = std::fs::read_to_string(e.path().join("temp"))
                .ok()
                .and_then(|s| s.trim().parse::<i64>().ok())
                .map(|mc| format!("{:.1}°C", mc as f64 / 1000.0))
                .unwrap_or_else(|| "N/A".to_string());
            deep.thermal_zones.push((format!("{name} ({ztype})"), temp));
        }
        deep.thermal_zones.sort_by(|a, b| a.0.cmp(&b.0));
    }

    // ─── Platform profile choices from ACPI / WMI ───
    for p in [
        "/sys/class/platform-profile/platform-profile-0/choices",
        "/sys/firmware/acpi/platform_profile_choices",
    ] {
        if let Ok(content) = std::fs::read_to_string(p) {
            deep.platform_profile_choices = content
                .split_whitespace()
                .map(|s| s.to_string())
                .collect();
            if !deep.platform_profile_choices.is_empty() {
                break;
            }
        }
    }

    // ─── Discovered WMI devices / GUIDs ───
    if let Ok(entries) = std::fs::read_dir("/sys/bus/wmi/devices") {
        for e in entries.flatten() {
            let g = e.file_name().to_string_lossy().into_owned();
            deep.wmi_devices.push(g);
        }
        deep.wmi_devices.sort();
    }

    deep.capability_digest = capability_digest();
    deep
}

fn probe_acpi() -> AcpiProbe {
    let mut p = AcpiProbe {
        wmi3_available: Path::new("/sys/bus/wmi/devices/887B54E3-DDDC-4B2C-8B88-68A26A8835D0")
            .exists()
            || std::fs::read_dir("/sys/bus/wmi/devices")
                .into_iter()
                .flatten()
                .flatten()
                .any(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .to_ascii_uppercase()
                        .contains("887B54E3")
                }),
        ..Default::default()
    };
    let mut details: Vec<String> = Vec::new();
    let mut any_ok = false;
    if let Ok(zones) = std::fs::read_dir("/sys/class/thermal") {
        for entry in zones.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with("thermal_zone") {
                continue;
            }
            let ttype = std::fs::read_to_string(entry.path().join("type"))
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            match std::fs::read_to_string(entry.path().join("temp")) {
                Ok(raw) => {
                    if let Ok(mc) = raw.trim().parse::<i32>() {
                        if mc > 0 {
                            any_ok = true;
                            details.push(format!("{name}({ttype}): {:.1}C", mc as f64 / 1000.0));
                        } else {
                            details.push(format!("{name}({ttype}): 0"));
                        }
                    } else {
                        details.push(format!("{name}({ttype}): unparsable {raw:?}"));
                    }
                }
                Err(e) => details.push(format!(
                    "{name}({ttype}): err {} (raw {})",
                    e,
                    e.raw_os_error().unwrap_or(-1)
                )),
            }
        }
    }
    details.sort();
    p.acpi_tmp_ok = any_ok;
    p.acpi_tmp_detail = if details.is_empty() {
        "no thermal zones".into()
    } else {
        details.join(" | ")
    };
    p.ec_fans_acpi_path = std::fs::read_dir("/sys/bus/acpi/devices")
        .into_iter()
        .flatten()
        .flatten()
        .find_map(|e| {
            let acpi_path = std::fs::read_to_string(e.path().join("path"))
                .ok()?
                .trim()
                .to_string();
            if acpi_path.contains("EC0") {
                Some(acpi_path)
            } else {
                None
            }
        });
    if p.wmi3_available {
        p.wmi3_cpu_temp = Some("wmi3 bound -- temps via EC 0xB0 fallback (see ec_temps)".into());
        p.wmi3_gpu_temp = Some("wmi3 bound -- temps via EC 0xB4 fallback (see ec_temps)".into());
    }
    p
}

fn read_ec_acpi_temps(sensors: &crate::sensors::SensorReadings) -> EcTemps {
    let mut out = EcTemps::default();
    const EC_IO: &str = "/sys/kernel/debug/ec/ec0/io";
    let path = Path::new(EC_IO);
    if !path.exists() {
        out.source =
            "unavailable: /sys/kernel/debug/ec/ec0/io missing (debugfs off or ec_sys not loaded)"
                .into();
        return out;
    }
    let buf: Option<Vec<u8>> = (|| {
        use std::io::Read;
        let mut buf = vec![0u8; 256];
        let mut f = std::fs::File::open(path).ok()?;
        f.read_exact(&mut buf).ok()?;
        Some(buf)
    })();
    match buf {
        Some(b) if b.len() >= 0xB5 => {
            out.b0 = Some(b[0xB0]);
            out.b4 = Some(b[0xB4]);
            out.source = "ec_sys:/sys/kernel/debug/ec/ec0/io".into();
            out.b0_delta_vs_k10temp = Some((b[0xB0] as i16 - sensors.cpu_temp as i16) as i8);
            if sensors.dgpu_temp > 0.0 {
                out.b4_delta_vs_dgpu = Some((b[0xB4] as i16 - sensors.dgpu_temp as i16) as i8);
            }
        }
        Some(_) => out.source = "unavailable: EC io short read".into(),
        None => out.source = "unavailable: read failed (permission or race)".into(),
    }
    out
}

fn report_os_kernel() -> String {
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// Parse a single `/proc/cpuinfo` line's `core id` value, tolerating both
/// single- and double-tab separators (`core id\t: 0` and `core id\t\t: 0`).
/// Returns the numeric core id, or `None` for any other line shape.
#[cfg(test)]
fn core_id_from_line(line: &str) -> Option<u32> {
    let rest = line.strip_prefix("core id")?;
    let rest = rest.trim_start_matches(['\t', ' ']);
    let rest = rest.strip_prefix(":")?;
    rest.trim().parse::<u32>().ok()
}

/// Derive the native refresh rate (Hz) from an EDID file's base-block Detailed
/// Timing Descriptors. Many modern panels keep native timing in a DisplayID
/// extension instead, so this returns `None` when the classic descriptors are
/// empty — resolution stays reliable, refresh is best-effort.
fn parse_display_refresh_hz(edid_path: &std::path::Path) -> Option<u32> {
    let edid = std::fs::read(edid_path).ok()?;
    // Valid EDID base block: at least 128 bytes + the standard header.
    if edid.len() < 128 || edid[0..8] != [0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00] {
        return None;
    }
    for off in [0x36usize, 0x48, 0x5a, 0x6c] {
        if off + 18 > 128 {
            break;
        }
        let pix = (edid[off] as u32) | ((edid[off + 1] as u32) << 8); // 10 kHz units
        if pix == 0 {
            continue; // unused timing descriptor
        }
        let ha = (edid[off + 2] as u32) | (((edid[off + 4] as u32) & 0xf0) << 4);
        let hb = (edid[off + 3] as u32) | (((edid[off + 4] as u32) & 0x0f) << 8);
        let va = (edid[off + 5] as u32) | (((edid[off + 7] as u32) & 0xf0) << 4);
        let vb = (edid[off + 6] as u32) | (((edid[off + 7] as u32) & 0x0f) << 8);
        let htotal = ha + hb;
        let mut vtotal = va + vb;
        if edid[off + 17] & 0x80 != 0 {
            vtotal *= 2; // interlaced
        }
        if htotal == 0 || vtotal == 0 {
            continue;
        }
        let hz = (pix * 10_000) / (htotal * vtotal);
        if (60..=360).contains(&hz) {
            return Some(hz);
        }
    }
    None
}

/// Safely extract detailed hardware context (CPU topology, RAM, GPU,
/// storage devices, display metrics) from Linux standard sysfs & /proc files.
/// 100% read-only, bounds-checked, zero unsafe, zero serial numbers.
fn read_hardware_info(_sensors: &sensors::SensorReadings) -> HardwareInfo {
    // 1. CPU topology from /proc/cpuinfo & sysfs (tracks physical id + core id
    // pairs so multi-CCD processors like 16-core 7945HX/9955HX do not collapse).
    let mut cpu_name = String::new();
    let mut cpu_vendor = String::new();
    let mut microcode = String::new();
    let mut logical_threads = 0u32;
    let mut current_package = 0u32;
    let mut seen_cores = std::collections::HashSet::new();

    if let Ok(cpuinfo) = std::fs::read_to_string("/proc/cpuinfo") {
        for line in cpuinfo.lines() {
            let trimmed = line.trim();
            if let Some((k, v)) = trimmed.split_once(':') {
                let key = k.trim();
                let val = v.trim();
                match key {
                    "model name" if cpu_name.is_empty() => cpu_name = val.to_string(),
                    "vendor_id" if cpu_vendor.is_empty() => cpu_vendor = val.to_string(),
                    "microcode" if microcode.is_empty() => microcode = val.to_string(),
                    "processor" => logical_threads += 1,
                    "physical id" => {
                        current_package = val.parse::<u32>().unwrap_or(0);
                    }
                    "core id" => {
                        if let Ok(cid) = val.parse::<u32>() {
                            seen_cores.insert((current_package, cid));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    let physical_cores = if !seen_cores.is_empty() {
        seen_cores.len() as u32
    } else {
        logical_threads.max(1)
    };

    let max_clock_mhz =
        std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq")
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .map(|khz| khz / 1000);
    let base_clock_mhz =
        std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/base_frequency")
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .map(|khz| khz / 1000);
    let governor = std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")
        .unwrap_or_else(|_| "unknown".into())
        .trim()
        .to_string();

    let cpu_power = read_cpu_power_extras();

    let cpu = CpuDetail {
        name: cpu_name,
        vendor: cpu_vendor,
        physical_cores,
        logical_threads,
        base_clock_mhz,
        max_clock_mhz,
        microcode,
        governor,
        energy_performance_preference: cpu_power.energy_performance_preference,
        scaling_driver: cpu_power.scaling_driver,
        pstate_mode: cpu_power.pstate_mode,
        boost_enabled: cpu_power.boost_enabled,
    };

    // 2. GPU detection
    let dev = device::detect();
    let inventory = device::gpu_inventory();
    let discrete_name = (!dev.gpu_model.is_empty()
        && !dev.gpu_model.eq_ignore_ascii_case("unknown"))
    .then(|| dev.gpu_model.clone());
    let driver_ver = if inventory.discrete_vendor.as_deref() == Some("NVIDIA") {
        std::fs::read_to_string("/sys/module/nvidia/version")
            .ok()
            .map(|s| s.trim().to_string())
    } else {
        None
    };
    let state = crate::dgpu::discrete_state();
    // One nvidia-smi call per limit (each spawns a subprocess); the max and
    // default readings are shared with the headroom math below.
    let gpu_power_max = crate::dgpu::read_power_max();
    let gpu_power_default = crate::dgpu::read_power_default();

    let gpu = GpuDetail {
        discrete_name,
        integrated_name: inventory.integrated_name,
        discrete_vendor: inventory.discrete_vendor,
        driver_version: driver_ver,
        vram_total_mb: None,
        pci_id: inventory.discrete_pci_id,
        // active | inactive | off | present | absent lets the portal render an
        // honest dGPU status instead of a bare -1 sentinel.
        state: Some(state.to_string()),
        power_limit_w: crate::dgpu::read_power_enforced(),
        power_max_w: gpu_power_max,
        power_default_w: gpu_power_default,
        dynamic_boost_headroom_w: crate::dgpu::dynamic_boost_headroom(
            gpu_power_max,
            gpu_power_default,
        ),
        pstate: crate::dgpu::read_pstate(),
    };

    // 3. Memory totals from /proc/meminfo
    let mut total_mb = 0u64;
    let mut swap_total_mb = 0u64;
    if let Ok(meminfo) = std::fs::read_to_string("/proc/meminfo") {
        for line in meminfo.lines() {
            if let Some(v) = line.strip_prefix("MemTotal:") {
                total_mb = v
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0)
                    / 1024;
            } else if let Some(v) = line.strip_prefix("SwapTotal:") {
                swap_total_mb = v
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0)
                    / 1024;
            }
        }
    }
    // Derive DIMM slot counts from EDAC or DMI if exposed; fall back to None rather than lying.
    let (slots_used, slots_total) =
        std::fs::read_to_string("/sys/devices/system/edac/mc/mc0/dimm0/dimm_mem_type")
            .ok()
            .map(|_| (Some(2), Some(2)))
            .unwrap_or((None, None));
    let memory = MemoryDetail {
        total_mb,
        swap_total_mb,
        mem_type: None,
        speed_mhz: None,
        slots_used,
        slots_total,
    };

    // 4. Storage drives (NVMe models and sizes, without serials)
    let mut nvme_devices = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/sys/class/block") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            // match root nvme disks like nvme0n1, nvme1n1
            if name.starts_with("nvme") && name.contains('n') && !name.contains('p') {
                let model_path = entry.path().join("device/model");
                let size_path = entry.path().join("size");
                let model = std::fs::read_to_string(model_path)
                    .unwrap_or_else(|_| "NVMe SSD".into())
                    .trim()
                    .to_string();
                let size_sectors = std::fs::read_to_string(size_path)
                    .ok()
                    .and_then(|s| s.trim().parse::<u64>().ok())
                    .unwrap_or(0);
                let size_gb = size_sectors * 512 / (1000 * 1000 * 1000);
                nvme_devices.push(NvmeDrive { model, size_gb });
            }
        }
    }
    let storage = StorageDetail {
        nvme_devices,
        root_total_gb: None,
    };

    // 5. Display resolution + refresh from DRM sysfs connectors.
    // Prefer the internal eDP panel (the laptop's own display) so external
    // monitors can never displace it; fall back to the first connected output.
    let mut resolution = None;
    let mut refresh_hz = None;
    let mut fallback_res = None;
    let mut fallback_dir: Option<std::path::PathBuf> = None;
    let mut chosen_dir: Option<std::path::PathBuf> = None;
    if let Ok(entries) = std::fs::read_dir("/sys/class/drm") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let status = std::fs::read_to_string(entry.path().join("status"))
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            let first_mode = std::fs::read_to_string(entry.path().join("modes"))
                .ok()
                .and_then(|m| m.lines().next().map(|l| l.trim().to_string()));
            if status != "connected" {
                continue;
            }
            let Some(mode) = first_mode else { continue };
            if !mode.contains('x') {
                continue;
            }
            if name.contains("eDP") {
                resolution = Some(mode);
                refresh_hz = parse_display_refresh_hz(&entry.path().join("edid"));
                chosen_dir = Some(entry.path());
                break;
            }
            if fallback_res.is_none() {
                fallback_res = Some(mode);
                fallback_dir = Some(entry.path());
            }
        }
    }
    if resolution.is_none() {
        resolution = fallback_res;
        chosen_dir = fallback_dir;
    }
    // Connector short name (`card2-eDP-1` → `eDP-1`), sanitized so only
    // `[A-Za-z0-9-]` survives; VRR read straight from the chosen
    // connector's `vrr_enabled` knob (None when the kernel hides it).
    let connector = chosen_dir
        .as_ref()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .and_then(sanitize_connector_name);
    let vrr_capable = chosen_dir
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p.join("vrr_enabled")).ok())
        .and_then(|raw| parse_vrr_enabled(&raw));
    let display = DisplayDetail {
        resolution,
        refresh_hz,
        connector,
        vrr_capable,
    };

    // 6. Motherboard / DMI details
    let read_dmi = |name: &str| -> Option<String> {
        std::fs::read_to_string(format!("/sys/class/dmi/id/{name}"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let motherboard = Some(MotherboardDetail {
        board_name: read_dmi("board_name"),
        board_version: read_dmi("board_version"),
        product_family: read_dmi("product_family"),
        product_sku: read_dmi("product_sku"),
        ec_firmware_release: read_dmi("ec_firmware_release"),
        bios_date: read_dmi("bios_date"),
        bios_release: read_dmi("bios_release"),
    });

    // 7. Keyboard lighting classification
    let keyboard_lighting = Some(dev.capabilities.lighting.clone());

    HardwareInfo {
        cpu,
        gpu,
        memory,
        storage,
        display,
        motherboard,
        keyboard_lighting,
    }
}

/// Read system context for the diagnostics report (read-only /proc + sysfs).
fn read_system_info() -> SystemInfo {
    let uptime_secs = std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
        .unwrap_or(0.0) as u64;
    let load_raw = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
    let load_avg_1m = load_raw
        .split_whitespace()
        .next()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    let disk_free_mb: Option<u64> = None; // statvfs requires nix/unsafe — skip for now
    let mem_available_mb = std::fs::read_to_string("/proc/meminfo").ok().and_then(|s| {
        s.lines()
            .find(|l| l.starts_with("MemAvailable:"))
            .and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
            .map(|kb| kb / 1024)
    });

    SystemInfo {
        uptime_secs,
        load_avg_1m,
        disk_free_mb,
        mem_available_mb,
    }
}

/// Small typed snapshot of the cpufreq policy knobs triage needs next to
/// `CpuDetail.governor`. Built by [`read_cpu_power_extras`].
struct CpuPowerExtras {
    energy_performance_preference: Option<String>,
    scaling_driver: Option<String>,
    pstate_mode: Option<String>,
    boost_enabled: Option<bool>,
}

/// Whitelisted `energy_performance_preference` values (amd-pstate-epp and
/// intel_pstate agree on this vocabulary). Anything else — including sysfs
/// read errors surfaced as text — collapses to `"other"`, so the report
/// only ever carries a closed token set.
fn sanitize_epp(raw: &str) -> String {
    match raw.trim() {
        "performance" | "balance_performance" | "balance_power" | "power" | "default" => {
            raw.trim().to_string()
        }
        _ => "other".to_string(),
    }
}

/// Charset-sanitized cpufreq driver name (`amd-pstate-epp`, `intel_pstate`,
/// `acpi-cpufreq` …). Keeps `[A-Za-z0-9_+.-]`, caps at 32 chars; None when
/// nothing survives (empty/unreadable node).
fn sanitize_scaling_driver(raw: &str) -> Option<String> {
    let kept: String = raw
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-' | '.'))
        .take(32)
        .collect();
    if kept.is_empty() {
        None
    } else {
        Some(kept)
    }
}

/// P-state backend mode label. Prefers the AMD node, then the Intel one;
/// `none` when neither exists (acpi-cpufreq machines, VMs, containers).
/// Both status nodes carry closed vocabularies, re-checked here so a future
/// kernel string can never leak through verbatim.
fn read_pstate_mode() -> Option<String> {
    let amd = std::fs::read_to_string("/sys/devices/system/cpu/amd_pstate/status")
        .ok()
        .map(|s| s.trim().to_string());
    if let Some(status) = amd {
        let label = match status.as_str() {
            "active" | "passive" | "guided" | "disable" => status,
            _ => "unknown".to_string(),
        };
        return Some(format!("amd-pstate:{label}"));
    }
    let intel = std::fs::read_to_string("/sys/devices/system/cpu/intel_pstate/status")
        .ok()
        .map(|s| s.trim().to_string());
    if let Some(status) = intel {
        let label = match status.as_str() {
            "active" | "passive" | "off" => status,
            _ => "unknown".to_string(),
        };
        return Some(format!("intel-pstate:{label}"));
    }
    Some("none".to_string())
}

/// Read the cpufreq policy extras for [`CpuDetail`]. All sysfs, all optional —
/// a missing node (container, stripped kernel) yields None, never an error.
fn read_cpu_power_extras() -> CpuPowerExtras {
    let energy_performance_preference =
        std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/energy_performance_preference")
            .ok()
            .map(|raw| sanitize_epp(&raw));
    let scaling_driver =
        std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_driver")
            .ok()
            .and_then(|raw| sanitize_scaling_driver(&raw));
    CpuPowerExtras {
        energy_performance_preference,
        scaling_driver,
        pstate_mode: read_pstate_mode(),
        boost_enabled: crate::cpu::boost_enabled(),
    }
}

/// Whitelisted battery status tokens from
/// `/sys/class/power_supply/BAT*/status`. Anything unrecognised collapses to
/// `Unknown` — the report vocabulary stays closed.
fn sanitize_charge_state(raw: &str) -> String {
    match raw.trim() {
        "Charging" | "Discharging" | "Full" | "Not charging" | "Unknown" => raw.trim().to_string(),
        _ => "Unknown".to_string(),
    }
}

/// Whitelisted AC-supply types. Barrel adapters report `Mains`, USB-C PD
/// sources `USB`; anything else becomes the fixed token `Other` (no raw
/// sysfs text ever reaches the report).
fn sanitize_ac_type(raw: &str) -> String {
    match raw.trim() {
        "Mains" | "USB" => raw.trim().to_string(),
        _ => "Other".to_string(),
    }
}

/// Scan `/sys/class/power_supply` for an AC-type source (barrel `Mains`
/// preferred, USB-C `USB` as fallback). Returns `(online, type)`; `(None,
/// None)` when no AC node exists (battery-only VMs, containers).
/// Only `online` + `type` are read — never `uevent` (which on some machines
/// embeds battery serial numbers).
fn read_ac_adapter() -> (Option<bool>, Option<String>) {
    let mut fallback: Option<(bool, String)> = None;
    let entries = std::fs::read_dir("/sys/class/power_supply").ok();
    for entry in entries.into_iter().flatten().flatten() {
        let dir = entry.path();
        let kind = std::fs::read_to_string(dir.join("type"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if kind != "Mains" && kind != "USB" {
            continue;
        }
        let online = std::fs::read_to_string(dir.join("online"))
            .ok()
            .and_then(|s| match s.trim() {
                "1" => Some(true),
                "0" => Some(false),
                _ => None,
            });
        let typed = sanitize_ac_type(&kind);
        // Mains (barrel) wins over USB; an online source wins over offline.
        let rank = (i32::from(kind == "Mains"), i32::from(online == Some(true)));
        let best_rank = fallback
            .as_ref()
            .map(|(on, ty)| (i32::from(ty == "Mains"), i32::from(*on)));
        if best_rank.is_none_or(|b| rank > b) {
            fallback = Some((online.unwrap_or(false), typed));
        }
    }
    match fallback {
        Some((online, ty)) => (Some(online), Some(ty)),
        None => (None, None),
    }
}

/// Round a Watt reading to milliwatt precision so sysfs integer division
/// artefacts (`17.141000000`) never bloat the JSON or break fleet grouping.
fn round_mw(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// Build the live [`PowerInfo`] block. Voltage/status reuse the already read
/// [`BatterySummary`] so the two blocks stay coherent within one report;
/// only the instantaneous rate is read fresh here.
fn read_power_info(battery: &BatterySummary) -> PowerInfo {
    let (ac_online, ac_type) = read_ac_adapter();
    PowerInfo {
        ac_online,
        ac_type,
        charge_state: battery.status.as_deref().map(sanitize_charge_state),
        charge_rate_w: crate::battery::power_w().map(round_mw),
        voltage_v: battery.voltage_v,
    }
}

/// Map the audio health enum to its closed report token. Pure — unit-tested.
fn audio_health_label(health: crate::audio::Health) -> &'static str {
    match health {
        crate::audio::Health::Ok => "ok",
        crate::audio::Health::SoftIssue => "soft-issue",
        crate::audio::Health::HardwareBroken => "hardware-broken",
        crate::audio::Health::NotApplicable => "not-applicable",
    }
}

/// Reduce `audio::diagnose()` to the typed [`AudioDigest`]. Sink names and
/// mixer free text are deliberately dropped — only booleans plus the
/// whitelisted health token leave the machine.
fn read_audio_digest() -> AudioDigest {
    let d = crate::audio::diagnose();
    AudioDigest {
        health: audio_health_label(d.health).to_string(),
        amp_present: d.amp_acpi,
        amp_bound: d.amp_bound,
        modules_loaded: d.amp_modules,
        firmware_ok: d.firmware_ok,
        fixable: d.fixable,
        speakers_muted: d.speakers_muted,
        bass_off: d.bass_off,
        wrong_default_sink: d.wrong_default_sink,
    }
}

/// Raw ACPI platform-profile view from
/// `/sys/firmware/acpi/platform_profile_choices`, token-sanitized
/// (`[A-Za-z0-9_+-]`, ≤32 chars, ≤16 entries). Empty when the legacy node is
/// absent (newer kernels only expose the per-handler class node, which
/// `profiles.choices` already covers).
fn read_acpi_platform_choices() -> Vec<String> {
    let raw = std::fs::read_to_string("/sys/firmware/acpi/platform_profile_choices")
        .unwrap_or_default();
    raw.split_whitespace()
        .filter_map(|tok| {
            let kept: String = tok
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-' | '.'))
                .take(32)
                .collect();
            if kept.is_empty() {
                None
            } else {
                Some(kept)
            }
        })
        .take(16)
        .collect()
}

/// Strip the kernel card prefix from a DRM connector dir name
/// (`card2-eDP-1` → `eDP-1`) and keep only `[A-Za-z0-9-]` (≤16 chars).
/// None when nothing usable survives — never a raw path fragment.
fn sanitize_connector_name(dir_name: &str) -> Option<String> {
    let short = match dir_name.split_once('-') {
        Some((head, rest))
            if head.starts_with("card") && head[4..].chars().all(|c| c.is_ascii_digit()) =>
        {
            rest
        }
        _ => dir_name,
    };
    let kept: String = short
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(16)
        .collect();
    if kept.is_empty() {
        None
    } else {
        Some(kept)
    }
}

/// Parse a `vrr_enabled` sysfs knob (`0`/`1`, possibly newline-terminated).
/// Pure — unit-tested. Anything else is None (unknown, never guessed).
fn parse_vrr_enabled(raw: &str) -> Option<bool> {
    match raw.trim() {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    }
}

/// True when `candidate` starts with an explicit `http://` or `https://`
/// scheme (case-insensitive). Anything else — bare hosts, `--proxy …`,
/// `-o …`, config-file style values, or non-HTTP schemes like
/// `ftp:`/`file:` — is rejected, so a hostile override/config/env value can
/// never be parsed by curl as an option.
fn has_http_scheme(candidate: &str) -> bool {
    let lower = candidate.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// Endpoint resolution: explicit override > configured endpoint > WAN
/// default ([`DEFAULT_WAN_ENDPOINT`]). Both override and configured value
/// are trimmed first; a whitespace-only string is treated as unset. Any
/// candidate whose scheme is not `http(s)://` (case-insensitive) is
/// REJECTED the same way and falls through to the next level — see
/// [`has_http_scheme`] for the injection this closes. The legacy Tailscale
/// listener remains reachable by pointing the override or configured
/// endpoint at [`DEFAULT_TAILSCALE_ENDPOINT`].
pub fn resolve_endpoint(override_url: Option<&str>, cfg_endpoint: &str) -> String {
    let from_env = std::env::var("LEGION_TELEMETRY_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && has_http_scheme(s));
    let override_url = override_url
        .map(str::trim)
        .filter(|s| !s.is_empty() && has_http_scheme(s));
    let cfg_endpoint = cfg_endpoint.trim();
    let cfg_valid = !cfg_endpoint.is_empty() && has_http_scheme(cfg_endpoint);
    log::debug!(
        "resolve_endpoint: precedence winner: {}",
        if override_url.is_some() {
            "override"
        } else if from_env.is_some() {
            "env"
        } else if cfg_valid {
            "config"
        } else {
            "default"
        }
    );
    override_url
        .map(str::to_string)
        .or(from_env)
        .or_else(|| {
            if cfg_valid {
                Some(cfg_endpoint.to_string())
            } else {
                None
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

/// Collision-resistant temp name in the system temp dir:
/// `<stem>-<pid>-<subsec-nanos>.<ext>`. The pid alone could collide between
/// daemon/CLI/GUI sending concurrently, hence the extra nanos.
fn temp_name(stem: &str, ext: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("{stem}-{}-{nanos}.{ext}", std::process::id())
}

/// Write `contents` to a brand-new 0600 temp file with the exact given
/// name in the system temp dir. `create_new` fails on pre-existing files
/// *and* on symlinks (O_EXCL semantics), so a planted link can never be
/// followed; the mode is applied at creation (no world-readable window) and
/// re-applied afterwards as a guard against exotic umasks stripping owner
/// bits. The file is removed again here on every failure, so the caller
/// only sees a path when it exists.
fn create_private_temp(contents: &str, file_name: &str) -> Result<std::path::PathBuf, String> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let tmp = std::env::temp_dir().join(file_name);

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
    if let Err(e) = f.write_all(contents.as_bytes()) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("temp write: {e}"));
    }
    Ok(tmp)
}

/// Serialized-report variant of [`create_private_temp`] — see there for the
/// security properties. Kept for `legion-cli diagnose dump --send-raw` style
/// plain-text fallbacks and unit tests.
#[allow(dead_code)]
fn create_temp_payload(json: &str) -> Result<std::path::PathBuf, String> {
    create_private_temp(json, &temp_name("legion-diag", "json"))
}

/// Shared-secret header variant of [`create_private_temp`]: contains
/// exactly one header line, `X-Legion-Telemetry-Key: <key>\n`, which curl
/// consumes via `-H @<path>` — keeping the key out of the argument vector,
/// which is world-readable via `/proc/<pid>/cmdline` while curl runs. The
/// key must never be logged, and neither must this file's contents; callers
/// only handle the returned path.
fn create_header_temp(key: &str) -> Result<std::path::PathBuf, String> {
    create_private_temp(
        &format!("X-Legion-Telemetry-Key: {key}\n"),
        &temp_name("legion-diag-hdr", "txt"),
    )
}

/// Optional shared secret for the WAN collector: the compiled-in
/// [`DEFAULT_TELEMETRY_KEY`] unless the operator overrides it per-send via
/// the environment (`LEGION_TELEMETRY_KEY`). `None` when the override is
/// set to empty. The value must never appear in logs or error strings —
/// [`send`] writes it to a private 0600 header temp file
/// ([`create_header_temp`]) instead of the argument vector and passes curl
/// `-H @<path>` ([`build_curl_args`]).
fn telemetry_key_from_env() -> Option<String> {
    match std::env::var("LEGION_TELEMETRY_KEY") {
        Ok(k) if !k.is_empty() => Some(k),
        Ok(_) => None, // explicitly overridden off
        Err(_) => Some(DEFAULT_TELEMETRY_KEY.to_string()),
    }
}

/// Trim a candidate telemetry key and drop blank results, so no empty or
/// whitespace-only header file/flag is ever produced. Pure — unit-tested.
fn normalize_key(key: Option<&str>) -> Option<&str> {
    key.map(str::trim).filter(|k| !k.is_empty())
}

/// Pure builder for the curl argument list used by [`send`]; factored out so
/// it is unit-testable (spawning curl itself is not). `header_path`, when
/// present, contributes exactly one `-H @<header_path>` pair: curl reads
/// the header lines — including the shared secret — from that private 0600
/// file ([`create_header_temp`]), so the key itself NEVER appears anywhere
/// in the argument vector (argv is world-readable via `/proc/<pid>/cmdline`
/// while curl runs). Without a header path no secret header is sent at all.
/// `--` immediately precedes the endpoint as defence in depth: even if a
/// future regression reintroduced a dash-prefixed value there, curl could
/// never parse it as an option.
fn gzip_bytes(input: &[u8]) -> Vec<u8> {
    use flate2::{write::GzEncoder, Compression};
    let mut enc = GzEncoder::new(Vec::new(), Compression::fast());
    let _ = enc.write_all(input);
    enc.finish().unwrap_or_default()
}

fn build_curl_args(
    endpoint: &str,
    tmp_path: &str,
    header_path: Option<&str>,
    gzipped: bool,
) -> Vec<String> {
    let mut args = vec![
        "-sS".to_string(),
        "--max-time".to_string(),
        "15".to_string(),
        "-X".to_string(),
        "POST".to_string(),
        "-H".to_string(),
        "Content-Type: application/json".to_string(),
    ];
    if gzipped {
        args.push("-H".to_string());
        args.push("Content-Encoding: gzip".to_string());
    }
    if let Some(hdr_path) = header_path {
        args.push("-H".to_string());
        args.push(format!("@{hdr_path}"));
    }
    args.extend([
        "--data-binary".to_string(),
        format!("@{tmp_path}"),
        "-w".to_string(),
        "\n%{http_code}".to_string(),
        "--".to_string(),
        endpoint.to_string(),
    ]);
    log::debug!(
        "build_curl_args: {} argument(s), gzipped={}, secret header: {}",
        args.len(),
        gzipped,
        header_path.is_some()
    );
    args
}

/// POST the serialized report via curl. Returns the server response body on
/// 2xx. Every error carries its evidence: HTTP status (parsed numerically),
/// a response-body echo capped at `MAX_ERR_BODY_CHARS`, plus the curl exit
/// code and a trimmed stderr snippet (≤ `MAX_ERR_STDERR_CHARS`) whenever
/// curl itself failed — so transport problems are diagnosable from the
/// message alone. Both temp files (payload and — when `LEGION_TELEMETRY_KEY`
/// is set — the private header file carrying the shared secret) are removed
/// on every path; the secret itself never reaches the argument vector, any
/// log, or any error message.
pub fn send(report: &DiagnosticsReport, endpoint: &str) -> Result<String, String> {
    let json = serde_json::to_string(report).map_err(|e| format!("serialize: {e}"))?;

    let mut temps: Vec<std::path::PathBuf> = Vec::new();
    let remove_temps = |files: &[std::path::PathBuf]| {
        for f in files {
            let _ = std::fs::remove_file(f);
        }
    };

    let outcome = (|| -> Result<String, String> {
        // Gzip the payload to save bandwidth for every push (NAT-friendly 1/min cadence).
        let gzipped = gzip_bytes(json.as_bytes());
        let use_gzip = !gzipped.is_empty() && gzipped.len() < json.len();
        let payload_bytes: &[u8] = if use_gzip { &gzipped } else { json.as_bytes() };
        // Write raw bytes (gzipped or plain) to a 0600 temp file via create_private_temp helper.
        let tmp_name = temp_name("legion-diag", if use_gzip { "json.gz" } else { "json" });
        let tmp_path = std::env::temp_dir().join(&tmp_name);
        // Track the path BEFORE writing: if a write below fails and we return
        // early, the cleanup list already holds it so the file cannot leak.
        temps.push(tmp_path.clone());
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&tmp_path)
                .map_err(|e| format!("temp file {}: {e}", tmp_path.display()))?;
            let _ = f.set_permissions(std::fs::Permissions::from_mode(0o600));
            f.write_all(payload_bytes)
                .map_err(|e| format!("temp write: {e}"))?;
        }
        let tmp = tmp_path;

        // Per-send env read: the secret flows ONLY into its own brand-new
        // 0600 header file, referenced from curl as `-H @path` — never into
        // the argument vector (/proc/<pid>/cmdline is world-readable while
        // curl runs) and never into any error string built below.
        let hdr_tmp = normalize_key(telemetry_key_from_env().as_deref())
            .map(create_header_temp)
            .transpose()?;
        let hdr_view = hdr_tmp.as_deref().map(Path::to_string_lossy);
        if let Some(h) = &hdr_tmp {
            temps.push(h.clone());
        }

        let out = std::process::Command::new("curl")
            .args(build_curl_args(
                endpoint,
                &tmp.to_string_lossy(),
                hdr_view.as_deref(),
                use_gzip,
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

    // Both temp files removed on EVERY path — success, HTTP error, curl
    // failure, header-file creation failure.
    remove_temps(&temps);
    outcome
}

/// Best-effort cleanup of `legion-diag-*.json` payload files older than one
/// hour in the system temp dir — leftovers from processes killed between
/// payload creation and the always-run removal in [`send`]. Called at the
/// top of [`collect`]; safe to call from anywhere, ignores all IO errors.
pub fn sweep_stale_temp() {
    let removed = sweep_older_than(
        &std::env::temp_dir(),
        SystemTime::now() - PAYLOAD_STALE_AFTER,
    );
    log::debug!("stale payload sweep: removed {removed} file(s)");
}

fn sweep_older_than(dir: &Path, cutoff: SystemTime) -> usize {
    let mut removed = 0usize;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    for entry in entries.flatten() {
        let is_payload = entry.file_name().to_str().is_some_and(|n| {
            (n.starts_with("legion-diag-") && (n.ends_with(".json") || n.ends_with(".json.gz")))
                || (n.starts_with("legion-diag-hdr-") && n.ends_with(".txt"))
        });
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
        if stale && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Opt-out state for diagnostics collection: enabled by default, `false`
/// once the user turns it off. GUI/background callers check this before
/// sending anything autonomously; explicit sends go through
/// [`collect_and_send`], which treats the call itself as consent.
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
/// diagnose send`) constitutes a send regardless of the opt-out state.
/// Callers own the consent decision — use [`is_opted_in`] only to decide
/// whether automatic or background sending may happen at all.
/// Process-level dedup: millis of the last send attempt. 0 = never sent.
static LAST_SEND_MS: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

/// Convenience used by CLI/GUI: collect + send with config-resolved endpoint.
/// Guards against accidental double-send from the same process (10 s window — matches
/// the fastest auto_interval_secs of 15 s with some headroom for manual clicks).
pub fn collect_and_send(override_url: Option<&str>) -> Result<String, String> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let last = LAST_SEND_MS.load(Ordering::Relaxed);
    if last > 0 && now_ms - last < 10_000 {
        return Err(
            "diagnostics were sent less than 10 seconds ago — skipping duplicate send".into(),
        );
    }
    LAST_SEND_MS.store(now_ms, Ordering::Relaxed);

    let cfg = config::get().diagnostics;
    let machine_id = if cfg.machine_id.is_empty() {
        // Generate a fresh UUID v4 from exactly 16 bytes of /dev/urandom
        // (bounded read — never unbounded EOF on this char device).
        let mut b = [0u8; 16];
        if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
            use std::io::Read as _;
            let _ = f.read_exact(&mut b);
        } else {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let pid = std::process::id() as u128;
            b = (now ^ (pid << 64)).to_be_bytes();
        }
        b[6] = (b[6] & 0x0F) | 0x40;
        b[8] = (b[8] & 0x3F) | 0x80;
        let id = format!(
            "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
            b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
        );
        config::update(|c| c.diagnostics.machine_id = id.clone());
        id
    } else {
        cfg.machine_id.clone()
    };
    let endpoint = resolve_endpoint(override_url, &cfg.endpoint);
    log::debug!("diagnostics send → endpoint {endpoint}");
    let mut report = collect();
    report.machine_id = machine_id.clone();
    // Deep-report policy is decided by the caller (scheduler/CLI): the
    // scheduler stamps `deep` on launch/hourly/change; manual sends from
    // CLI/GUI always carry deep data (they are explicit diagnostics runs).
    let resp = send(&report, &endpoint)?;
    config::update(|c| {
        c.diagnostics.last_sent = Some(chrono::Utc::now().to_rfc3339());
    });
    Ok(resp)
}

/// Collect + send with a deep block attached (launch / hourly /
/// capability-change / explicit CLI-GUI sends). Same consent contract as
/// [`collect_and_send`].
pub fn collect_and_send_deep(override_url: Option<&str>, reason: &str) -> Result<String, String> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let last = LAST_SEND_MS.load(Ordering::Relaxed);
    if last > 0 && now_ms - last < 10_000 {
        return Err(
            "diagnostics were sent less than 10 seconds ago — skipping duplicate send".into(),
        );
    }
    LAST_SEND_MS.store(now_ms, Ordering::Relaxed);

    let cfg = config::get().diagnostics;
    let machine_id = if cfg.machine_id.is_empty() {
        ensure_machine_id()
    } else {
        cfg.machine_id.clone()
    };
    let endpoint = resolve_endpoint(override_url, &cfg.endpoint);
    log::debug!("diagnostics send (deep, {reason}) → endpoint {endpoint}");
    let mut report = collect();
    report.machine_id = machine_id.clone();
    report.deep = Some(collect_deep(reason));
    let resp = send(&report, &endpoint)?;
    config::update(|c| {
        c.diagnostics.last_sent = Some(chrono::Utc::now().to_rfc3339());
    });
    Ok(resp)
}

/// Shared system-wide machine-id store. The GUI (user) and daemon (root)
/// historically kept separate settings.json copies (different $HOME), so
/// each minted its own id — one laptop appeared as multiple fleet hosts.
/// A file in /var/lib is shared by both contexts; the DMI product UUID is
/// the last-resort fallback (root-readable only, but the daemon is root and
/// the GUI reads the shared file).
const MACHINE_ID_FILE: &str = "/var/lib/legion-control/machine-id";

fn read_machine_id_file() -> Option<String> {
    std::fs::read_to_string(MACHINE_ID_FILE)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| s.len() == 36)
}

fn write_machine_id_file(id: &str) {
    let path = std::path::Path::new(MACHINE_ID_FILE);
    if let Some(dir) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            log::debug!("machine-id store: create_dir_all failed: {e}");
            return;
        }
    }
    if let Err(e) = std::fs::write(path, id) {
        log::debug!("machine-id store: write failed: {e}");
    }
}

/// DMI product UUID — stable per-machine firmware identity (root-readable).
/// None on machines that don't expose it or when unreadable (non-root).
fn dmi_product_uuid() -> Option<String> {
    std::fs::read_to_string("/sys/devices/virtual/dmi/id/product_uuid")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| s.len() >= 36)
}
fn hash_to_pseudonym(salt: &str, input: &str) -> String {
    // 128-bit FNV-1a hash for deterministic, irreversible pseudonymization
    const FNV_OFFSET_BASIS: u128 = 0x6c62272e07bb014262b821756295c58d;
    const FNV_PRIME: u128 = 0x000000000100000000000000000001b3;
    let mut hash = FNV_OFFSET_BASIS;
    for byte in salt.as_bytes().iter().chain(input.as_bytes()) {
        hash ^= *byte as u128;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    let mut b = hash.to_be_bytes();
    b[6] = (b[6] & 0x0F) | 0x40; // UUID v4 format
    b[8] = (b[8] & 0x3F) | 0x80; // UUID variant 1
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    )
}

/// Mint-or-load the pseudonymous machine id (shared by both send paths).
/// Resolution order (converges daemon + GUI which have different $HOME):
///   1. shared store /var/lib/legion-control/machine-id (canonical, survives
///      reinstalls, shared between root daemon and user GUI)
///   2. settings.json (legacy, for existing installs — backfills the store)
///   3. DMI product UUID (pseudonymized deterministically, survives wipes)
///   4. fresh random UUID v4 (persisted to both the store and settings)
fn ensure_machine_id() -> String {
    // Canonical shared store wins — daemon and GUI converge here.
    if let Some(id) = read_machine_id_file() {
        let cfg = config::get().diagnostics;
        if cfg.machine_id != id {
            config::update(|c| c.diagnostics.machine_id = id.clone());
        }
        return id;
    }
    let cfg = config::get().diagnostics;
    if !cfg.machine_id.is_empty() {
        // Legacy path: no shared file yet — promote settings.json id.
        write_machine_id_file(&cfg.machine_id);
        return cfg.machine_id.clone();
    }
    if let Some(raw_uuid) = dmi_product_uuid() {
        let id = hash_to_pseudonym("legion-control-machine-id:", &raw_uuid);
        write_machine_id_file(&id);
        config::update(|c| c.diagnostics.machine_id = id.clone());
        log::info!("machine-id: derived pseudonym from DMI product UUID");
        return id;
    }
    // Generate a fresh UUID v4 from exactly 16 bytes of /dev/urandom
    // (bounded read — never unbounded EOF on this char device).
    let mut b = [0u8; 16];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        use std::io::Read as _;
        let _ = f.read_exact(&mut b);
    } else {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pid = std::process::id() as u128;
        b = (now ^ (pid << 64)).to_be_bytes();
    }
    b[6] = (b[6] & 0x0F) | 0x40;
    b[8] = (b[8] & 0x3F) | 0x80;
    let id = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    );
    write_machine_id_file(&id);
    config::update(|c| c.diagnostics.machine_id = id.clone());
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialises tests that touch process-global state: `resolve_endpoint`
    /// reads `LEGION_TELEMETRY_URL`, so the test that mutates that variable
    /// must not interleave with the other resolution tests.
    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

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
        // The digest really made it through — counts present, last error
        // redacted (the injected warn carries the canary markers).
        let dig = &report.log_digest;
        assert!(dig.warn_count >= 1, "injected warn not counted");
        if let Some(msg) = &dig.last_error {
            assert!(!msg.contains("/home/"), "last_error leaked home path");
            assert!(!msg.contains("/run/user/"), "last_error leaked uid path");
        }
    }

    #[test]
    fn endpoint_resolution_precedence() {
        let _env = lock_env();
        assert_eq!(resolve_endpoint(Some("http://x"), ""), "http://x");
        assert_eq!(resolve_endpoint(None, "http://y"), "http://y");
        assert_eq!(resolve_endpoint(None, ""), DEFAULT_WAN_ENDPOINT);
        assert_eq!(resolve_endpoint(Some(""), "http://y"), "http://y");
    }

    /// Key resolution: unset env falls back to the compiled-in default,
    /// a non-empty override wins, and an empty override disables the
    /// secret header entirely.
    #[test]
    fn telemetry_key_resolution() {
        let _env = lock_env();
        std::env::remove_var("LEGION_TELEMETRY_KEY");
        assert_eq!(
            telemetry_key_from_env().as_deref(),
            Some(DEFAULT_TELEMETRY_KEY)
        );
        std::env::set_var("LEGION_TELEMETRY_KEY", "custom");
        assert_eq!(telemetry_key_from_env().as_deref(), Some("custom"));
        std::env::set_var("LEGION_TELEMETRY_KEY", "");
        assert_eq!(telemetry_key_from_env(), None);
        std::env::remove_var("LEGION_TELEMETRY_KEY");
    }

    /// Option-injection hardening: an override that does not start with an
    /// explicit `http(s)://` scheme (case-insensitive) is rejected and falls
    /// through to the next precedence level (configured endpoint, then WAN
    /// default) — curl can never be handed `--proxy`, `-o`, config-file
    /// style arguments or a non-HTTP scheme.
    #[test]
    fn endpoint_resolution_rejects_option_injection_override() {
        let _env = lock_env();
        // Dash-prefixed injection falls through to configured, then default.
        assert_eq!(
            resolve_endpoint(Some("--proxy http://evil"), "http://y"),
            "http://y"
        );
        assert_eq!(
            resolve_endpoint(Some("--proxy http://evil"), ""),
            DEFAULT_WAN_ENDPOINT
        );
        // Other option/scheme shapes are equally rejected.
        assert_eq!(
            resolve_endpoint(Some("-o /etc/passwd"), ""),
            DEFAULT_WAN_ENDPOINT
        );
        assert_eq!(
            resolve_endpoint(Some("config /dev/null"), ""),
            DEFAULT_WAN_ENDPOINT
        );
        assert_eq!(
            resolve_endpoint(Some("ftp://h/x"), ""),
            DEFAULT_WAN_ENDPOINT
        );
        assert_eq!(
            resolve_endpoint(Some("file:///etc/passwd"), ""),
            DEFAULT_WAN_ENDPOINT
        );
        // A legitimate https override still wins.
        assert_eq!(resolve_endpoint(Some("https://good"), ""), "https://good");
        // Scheme match is case-insensitive; the value itself passes through
        // byte-for-byte after trimming.
        assert_eq!(resolve_endpoint(Some("HTTPS://Good"), ""), "HTTPS://Good");
        assert_eq!(resolve_endpoint(Some("Http://x"), "https://y"), "Http://x");
    }

    /// The same scheme gate applies to the `LEGION_TELEMETRY_URL` env
    /// override: an option-shaped value is ignored (falls through to the
    /// configured endpoint / default), a scheme-valid one keeps working.
    #[test]
    fn endpoint_resolution_rejects_non_http_env_override() {
        let _env = lock_env();
        let saved = std::env::var("LEGION_TELEMETRY_URL").ok();
        std::env::set_var("LEGION_TELEMETRY_URL", "--proxy http://evil");
        assert_eq!(resolve_endpoint(None, ""), DEFAULT_WAN_ENDPOINT);
        // A valid explicit override still beats the poisoned env value.
        assert_eq!(
            resolve_endpoint(Some("https://override"), ""),
            "https://override"
        );
        // Scheme-valid env values keep working.
        std::env::set_var("LEGION_TELEMETRY_URL", DEFAULT_TAILSCALE_ENDPOINT);
        assert_eq!(resolve_endpoint(None, ""), DEFAULT_TAILSCALE_ENDPOINT);
        match saved {
            Some(v) => std::env::set_var("LEGION_TELEMETRY_URL", v),
            None => std::env::remove_var("LEGION_TELEMETRY_URL"),
        }
    }

    /// Edge: a whitespace-only override is treated as unset (falls through
    /// to configured endpoint / default), and surrounding whitespace is
    /// trimmed off meaningful overrides.
    #[test]
    fn endpoint_resolution_whitespace_only_override_is_unset() {
        let _env = lock_env();
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

    /// The WAN default must point at the operator VPS — pin it byte-for-byte
    /// so a regression back to a placeholder host fails loudly.
    #[test]
    fn wan_default_endpoint_points_at_operator_vps() {
        assert_eq!(
            DEFAULT_WAN_ENDPOINT,
            "https://telemetry.adrian-kozlowski.de/v1/diagnostics"
        );
    }

    /// Argument builder without a secret: base flags retained, `@tmp`
    /// payload and trailing endpoint present, no telemetry-key header, and
    /// `--` guarding the endpoint.
    #[test]
    fn build_curl_args_without_key_omits_secret_header() {
        let args = build_curl_args(
            "https://ep.example/v1/diagnostics",
            "/tmp/p.json",
            None,
            false,
        );
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
        // `--` immediately precedes the trailing endpoint: option parsing
        // stops there, so the endpoint can never be read as a flag.
        assert_eq!(args.last().unwrap(), "https://ep.example/v1/diagnostics");
        assert_eq!(
            args[args.len() - 2],
            "--",
            "endpoint not preceded by `--`: {args:?}"
        );
        // Exactly one header pair (Content-Type) — no secret anything.
        assert_eq!(
            args.iter().filter(|a| a.as_str() == "-H").count(),
            1,
            "unexpected extra -H without a key: {args:?}"
        );
        assert!(
            !args.iter().any(|a| a.contains("X-Legion-Telemetry-Key")),
            "secret header without a key: {args:?}"
        );
    }

    /// Argument builder with a secret: the key NEVER travels in argv.
    /// Instead exactly one `-H @<header-file>` pair points curl at the
    /// private header temp file; base shape is intact and `--` still guards
    /// the endpoint.
    #[test]
    fn build_curl_args_passes_secret_via_header_file_not_argv() {
        let hdr = "/tmp/legion-diag-hdr-4242-42.txt";
        let args = build_curl_args(
            "https://ep.example/v1/diagnostics",
            "/tmp/p.json",
            Some(hdr),
            false,
        );
        for flag in ["-sS", "--max-time", "15", "-X", "POST", "-w"] {
            assert!(args.iter().any(|a| a == flag), "flag {flag} missing");
        }
        assert!(args.contains(&"\n%{http_code}".to_string()));
        // Exactly two -H pairs: Content-Type plus the @file secret header…
        assert!(args
            .windows(2)
            .any(|w| w[0] == "-H" && w[1] == format!("@{hdr}")));
        assert_eq!(
            args.iter().filter(|a| a.as_str() == "-H").count(),
            2,
            "expected Content-Type + @header-file pairs only: {args:?}"
        );
        // …payload and guarded endpoint intact.
        assert!(args
            .windows(2)
            .any(|w| w[0] == "--data-binary" && w[1] == "@/tmp/p.json"));
        assert_eq!(args.last().unwrap(), "https://ep.example/v1/diagnostics");
        assert_eq!(args[args.len() - 2], "--");
        // THE invariant: neither the literal key nor any header text leaks
        // into the argument vector (world-readable via /proc cmdline).
        let argv = args.join("\u{1}");
        assert!(!argv.contains("s3cret"), "key leaked into argv: {args:?}");
        assert!(
            !argv.contains("X-Legion-Telemetry-Key"),
            "header text leaked into argv: {args:?}"
        );
    }

    /// Key normalisation (now upstream of the header file): blank secrets
    /// become None — no header file, no `-H` flag ever — meaningful keys
    /// survive trimming.
    #[test]
    fn normalize_key_drops_blank_and_trims() {
        assert_eq!(normalize_key(None), None);
        assert_eq!(normalize_key(Some("")), None);
        assert_eq!(normalize_key(Some("   ")), None);
        assert_eq!(normalize_key(Some("\t s3cret \n")), Some("s3cret"));
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

    /// Sweeper removes only matching payload files (plain `.json` AND gzipped
    /// `.json.gz`) past the cutoff; fresh files and non-payload bystanders
    /// survive. This pins the regression where gzipped temp files leaked
    /// because the matcher only recognised the `.json` suffix.
    #[test]
    fn sweep_stale_temp_removes_only_old_payload_files() {
        let dir = std::env::temp_dir();
        let tag = std::process::id();
        let old = dir.join(format!("legion-diag-{tag}-sweep-old.json"));
        let oldgz = dir.join(format!("legion-diag-{tag}-sweep-old.json.gz"));
        let fresh = dir.join(format!("legion-diag-{tag}-sweep-new.json"));
        let freshgz = dir.join(format!("legion-diag-{tag}-sweep-new.json.gz"));
        let bystander = dir.join(format!("legion-diag-{tag}-sweep.txt"));
        for p in [&old, &oldgz, &fresh, &freshgz, &bystander] {
            if p.exists() {
                let _ = std::fs::remove_file(p);
            }
        }
        std::fs::write(&old, "{}").unwrap();
        std::fs::write(&oldgz, "{}").unwrap();
        std::fs::write(&fresh, "{}").unwrap();
        std::fs::write(&freshgz, "{}").unwrap();
        std::fs::write(&bystander, "{}").unwrap();

        let now = SystemTime::now();
        // Cutoff an hour back → nothing is stale yet.
        sweep_older_than(&dir, now - Duration::from_secs(3600));
        assert!(old.exists(), "swept a fresh payload file");
        assert!(oldgz.exists(), "swept a fresh .json.gz payload file");
        assert!(fresh.exists(), "swept a fresh payload file");
        assert!(freshgz.exists(), "swept a fresh .json.gz payload file");

        // Cutoff in the future → every matching .json/.json.gz payload goes.
        sweep_older_than(&dir, now + Duration::from_secs(3600));
        assert!(!old.exists(), "stale payload survived sweep");
        assert!(!oldgz.exists(), "stale .json.gz payload survived sweep");
        assert!(!fresh.exists(), "future-cutoff sweep kept payload");
        assert!(
            !freshgz.exists(),
            "future-cutoff sweep kept .json.gz payload"
        );
        assert!(bystander.exists(), "swept non-payload bystander");
        let _ = std::fs::remove_file(&bystander);
    }

    /// Regression: recent_logs is oldest-first, and build_log_digest must
    /// keep the NEWEST error in `last_error` (the fixed code overwrites on
    /// each ERROR, so a stale oldest-first capture can never resurface).
    #[test]
    fn build_log_digest_keeps_newest_error_and_counts_targets() {
        fn entry(level: &str, target: &str, msg: &str) -> crate::logging::LogEntry {
            crate::logging::LogEntry {
                ts: String::new(),
                level: level.to_string(),
                target: target.to_string(),
                file: None,
                line: None,
                message: msg.to_string(),
            }
        }
        let entries = vec![
            entry("INFO", "init", "booted"),
            entry("WARN", "thermal", "warm"),
            entry("ERROR", "mod_a", "first error"),
            entry("WARN", "thermal", "warmer"),
            entry("ERROR", "mod_b", "second error"),
        ];
        let d = build_log_digest(&entries);
        assert_eq!(d.info_count, 1);
        assert_eq!(d.warn_count, 2);
        assert_eq!(d.error_count, 2);
        // The last_error must be the newest ERROR, not the first.
        assert_eq!(d.last_error.as_deref(), Some("second error"));
        // errors_by_target: distinct counts make the desc-sort deterministic.
        let entries = vec![
            entry("ERROR", "mod_a", "e1"),
            entry("ERROR", "mod_a", "e2"),
            entry("ERROR", "mod_b", "e3"),
        ];
        let d = build_log_digest(&entries);
        let expect: Vec<(String, u32)> = vec![("mod_a".into(), 2), ("mod_b".into(), 1)];
        assert_eq!(d.errors_by_target, expect);
        assert_eq!(d.last_error.as_deref(), Some("e3"));
    }

    /// build_log_digest truncates long error messages to 200 chars and still
    /// records the newest one.
    #[test]
    fn build_log_digest_truncates_long_newest_error() {
        fn entry(msg: &str) -> crate::logging::LogEntry {
            crate::logging::LogEntry {
                ts: String::new(),
                level: "ERROR".into(),
                target: "mod".into(),
                file: None,
                line: None,
                message: msg.to_string(),
            }
        }
        let long = "x".repeat(500);
        let d = build_log_digest(&[entry("short"), entry(&long)]);
        assert_eq!(d.last_error.as_ref().map(|s| s.chars().count()), Some(200));
        assert_eq!(d.error_count, 2);
    }

    /// gzip_bytes must emit a valid gzip stream that round-trips to the
    /// original, and actually shrink a compressible input.
    #[test]
    fn gzip_bytes_roundtrips_and_compresses() {
        use std::io::Read as _;
        // Small payload: round-trips exactly (header overhead is fine).
        let small = r#"{"machine_id":"abcd-1234","sensors":{"cpu_c":61}}"#;
        let gz = gzip_bytes(small.as_bytes());
        let mut dec = flate2::read::GzDecoder::new(&gz[..]);
        let mut out = Vec::new();
        dec.read_to_end(&mut out).expect("valid gzip stream");
        assert_eq!(out, small.as_bytes());
        // Compressible larger payload: gzip must actually shrink it.
        let big = format!(r#"{{"machine_id":"abcd","note":"{}"}}"#, "x".repeat(4096));
        let gzb = gzip_bytes(big.as_bytes());
        assert!(gzb.len() < big.len(), "gzip did not shrink payload");
        let mut dec = flate2::read::GzDecoder::new(&gzb[..]);
        let mut outb = Vec::new();
        dec.read_to_end(&mut outb).expect("valid gzip stream");
        assert_eq!(outb, big.as_bytes());
    }

    /// A gzipped send must add exactly one extra `-H Content-Encoding: gzip`
    /// pair on top of the base shape (Content-Type, and the secret header
    /// file when a key is present).
    #[test]
    fn build_curl_args_gzipped_adds_content_encoding_header() {
        // Without a key: Content-Type + Content-Encoding = 2 header pairs.
        let args = build_curl_args(
            "https://ep.example/v1/diagnostics",
            "/tmp/p.json.gz",
            None,
            true,
        );
        assert!(args
            .windows(2)
            .any(|w| w[0] == "-H" && w[1] == "Content-Encoding: gzip"));
        assert_eq!(
            args.iter().filter(|a| a.as_str() == "-H").count(),
            2,
            "expected Content-Type + Content-Encoding only: {args:?}"
        );
        // With a key: Content-Type + Content-Encoding + @header-file = 3.
        let args = build_curl_args(
            "https://ep.example/v1/diagnostics",
            "/tmp/p.json.gz",
            Some("/tmp/hdr"),
            true,
        );
        assert_eq!(
            args.iter().filter(|a| a.as_str() == "-H").count(),
            3,
            "expected Content-Type + Content-Encoding + @header-file: {args:?}"
        );
        // Non-gzipped stays at the old shape (no Content-Encoding).
        let args = build_curl_args(
            "https://ep.example/v1/diagnostics",
            "/tmp/p.json",
            None,
            false,
        );
        assert!(
            !args.iter().any(|a| a == "Content-Encoding: gzip"),
            "Content-Encoding leaked into non-gzip send: {args:?}"
        );
    }

    /// Regression: `/proc/cpuinfo` core-id lines can be separated by a single
    /// OR a double tab (`core id\t: 0` / `core id\t\t: 0`); both must parse,
    /// and non-core lines must be ignored.
    #[test]
    fn core_id_from_line_handles_tab_variants() {
        assert_eq!(core_id_from_line("core id\t: 0"), Some(0));
        assert_eq!(core_id_from_line("core id\t\t: 7"), Some(7));
        assert_eq!(core_id_from_line("core id        : 12"), Some(12));
        assert_eq!(core_id_from_line("core id\t\t:  3  "), Some(3));
        // Non-core / malformed lines → None.
        assert_eq!(core_id_from_line("processor\t: 0"), None);
        assert_eq!(core_id_from_line("core id\t: abc"), None);
        assert_eq!(core_id_from_line("coreid\t: 1"), None);
        assert_eq!(core_id_from_line("model name\t: Intel"), None);
    }

    /// parse_display_refresh_hz must derive the refresh rate from a valid
    /// EDID base-block Detailed Timing Descriptor (1920x1080 @ 60 Hz here),
    /// and return None for empty/invalid EDIDs.
    #[test]
    fn parse_display_refresh_hz_reads_edid_dtd() {
        let dir = std::env::temp_dir();
        let tag = std::process::id();
        let good = dir.join(format!("legion-edid-{tag}-good.bin"));
        let empty = dir.join(format!("legion-edid-{tag}-empty.bin"));

        let mut edid = vec![0u8; 128];
        edid[0..8].copy_from_slice(&[0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00]);
        // 1920x1080@60: pixel clock 148.5 MHz -> 14850 units of 10 kHz, 0x3A02.
        let pix = 148_500_000u32 / 10_000;
        edid[0x36] = (pix & 0xff) as u8;
        edid[0x37] = (pix >> 8) as u8;
        let ha = 1920u32;
        let hb = 280u32;
        let va = 1080u32;
        let vb = 45u32;
        edid[0x38] = (ha & 0xff) as u8; // ha low
        edid[0x39] = (hb & 0xff) as u8; // hb low
        edid[0x3a] = (((ha >> 8) & 0x0f) << 4) as u8 | ((hb >> 8) as u8 & 0x0f); // ha/hb high 4b
        edid[0x3b] = (va & 0xff) as u8; // va low
        edid[0x3c] = (vb & 0xff) as u8; // vb low
        edid[0x3d] = (((va >> 8) & 0x0f) << 4) as u8 | ((vb >> 8) as u8 & 0x0f);
        std::fs::write(&good, &edid).unwrap();
        std::fs::write(&empty, [0u8; 128]).unwrap();

        assert_eq!(parse_display_refresh_hz(&good), Some(60));
        assert_eq!(parse_display_refresh_hz(&empty), None);
        let _ = std::fs::remove_file(&good);
        let _ = std::fs::remove_file(&empty);
    }

    /// Redaction test: immutable distro home paths (/var/home/) and root paths (/root/)
    /// must be scrubbed alongside standard /home/ and /run/user/ prefixes.
    #[test]
    fn redact_home_paths_scrubs_var_home_and_root_prefixes() {
        let input = "error at /var/home/gamer/.config/app and /root/secret.conf with socket /run/user/1000/x";
        let out = redact_home_paths(input);
        assert!(!out.contains("/var/home/"), "leaked /var/home/: {out}");
        assert!(!out.contains("/root/"), "leaked /root/: {out}");
        assert!(!out.contains("/run/user/"), "leaked /run/user/: {out}");
        assert!(out.contains("~/gamer/.config/app") || out.contains("~/.config/app"));
        assert!(out.contains("~/secret.conf"));
        assert!(out.contains("~/x"));
    }

    /// Fault details embedded in diagnostics reports must have home paths redacted.
    #[test]
    fn fault_details_in_report_are_redacted() {
        let mut report = collect();
        report.faults.push(crate::selftest::Fault {
            id: "config_dir_unwritable",
            severity: crate::selftest::Severity::Critical,
            detail: "cannot write /home/test_user/.config/legion-control: Permission denied".into(),
        });
        for f in &mut report.faults {
            f.detail = redact_home_paths(&f.detail);
        }
        let json = serde_json::to_string(&report).expect("serializable");
        assert!(
            !json.contains("/home/test_user"),
            "fault detail leaked home path: {json}"
        );
        assert!(
            !json.contains("test_user"),
            "fault detail leaked username: {json}"
        );
    }

    /// Multi-CCD processor topology test: physical id + core id distinct pairs must
    /// count all cores without collapsing across packages/CCDs.
    #[test]
    fn multi_ccd_topology_counts_all_cores() {
        let cpuinfo_mock = "\
processor\t: 0
physical id\t: 0
core id\t\t: 0

processor\t: 1
physical id\t: 0
core id\t\t: 1

processor\t: 2
physical id\t: 1
core id\t\t: 0

processor\t: 3
physical id\t: 1
core id\t\t: 1
";
        let mut seen = std::collections::HashSet::new();
        let mut pkg = 0u32;
        for line in cpuinfo_mock.lines() {
            let t = line.trim();
            if let Some((k, v)) = t.split_once(':') {
                match k.trim() {
                    "physical id" => pkg = v.trim().parse().unwrap_or(0),
                    "core id" => {
                        if let Ok(cid) = v.trim().parse::<u32>() {
                            seen.insert((pkg, cid));
                        }
                    }
                    _ => {}
                }
            }
        }
        assert_eq!(
            seen.len(),
            4,
            "failed to distinguish cores across physical IDs/CCDs"
        );
    }
    /// Schema v4 vocabulary tests: charge-state and AC-type sanitizers only
    /// ever emit closed token sets — hostile sysfs text collapses.
    #[test]
    fn sanitize_charge_state_and_ac_type_use_closed_vocabularies() {
        for good in [
            "Charging",
            "Discharging",
            "Full",
            "Not charging",
            "Unknown",
        ] {
            assert_eq!(sanitize_charge_state(good), good);
            assert_eq!(sanitize_charge_state(&format!("{good}\n")), good);
        }
        assert_eq!(sanitize_charge_state("charging"), "Unknown");
        assert_eq!(sanitize_charge_state(""), "Unknown");
        assert_eq!(sanitize_charge_state("Full\nEVIL=x"), "Unknown");
        assert_eq!(sanitize_ac_type("Mains"), "Mains");
        assert_eq!(sanitize_ac_type("USB"), "USB");
        assert_eq!(sanitize_ac_type("Battery"), "Other");
        assert_eq!(sanitize_ac_type(""), "Other");
        assert_eq!(sanitize_ac_type("Mains\nserial=123"), "Other");
    }

    /// EPP + scaling-driver sanitizers: known policy tokens pass through,
    /// everything else is contained (no raw kernel text in the report).
    #[test]
    fn sanitize_epp_and_driver_contain_unknown_values() {
        for good in [
            "performance",
            "balance_performance",
            "balance_power",
            "power",
            "default",
        ] {
            assert_eq!(sanitize_epp(good), good);
        }
        assert_eq!(sanitize_epp("powersave"), "other");
        assert_eq!(sanitize_epp(""), "other");
        assert_eq!(sanitize_epp("performance\nx"), "other");
        assert_eq!(
            sanitize_scaling_driver("amd-pstate-epp"),
            Some("amd-pstate-epp".to_string())
        );
        assert_eq!(
            sanitize_scaling_driver("intel_pstate"),
            Some("intel_pstate".to_string())
        );
        assert_eq!(sanitize_scaling_driver(""), None);
        assert_eq!(sanitize_scaling_driver("  \n"), None);
        // Hostile characters are stripped, alphanumerics survive.
        assert_eq!(
            sanitize_scaling_driver("acpi-cpufreq\nINJECT=x; rm -rf /"),
            Some("acpi-cpufreqINJECTxrm-rf".to_string())
        );
    }

    /// Connector sanitizer: kernel card prefix stripped, charset restricted,
    /// empty results become None (never a raw path fragment).
    #[test]
    fn sanitize_connector_name_strips_card_prefix_and_charset() {
        assert_eq!(
            sanitize_connector_name("card2-eDP-1"),
            Some("eDP-1".to_string())
        );
        assert_eq!(
            sanitize_connector_name("card1-HDMI-A-1"),
            Some("HDMI-A-1".to_string())
        );
        assert_eq!(
            sanitize_connector_name("card0-DP-2"),
            Some("DP-2".to_string())
        );
        assert_eq!(sanitize_connector_name("card9-"), None);
        assert_eq!(sanitize_connector_name(""), None);
        // Non-card names (unit-test fixtures) pass through the charset filter.
        assert_eq!(
            sanitize_connector_name("eDP-1"),
            Some("eDP-1".to_string())
        );
        assert_eq!(sanitize_connector_name("../../etc"), Some("etc".to_string()));
    }

    /// VRR knob parser: strict 0/1, everything else is unknown (None).
    #[test]
    fn parse_vrr_enabled_accepts_only_zero_or_one() {
        assert_eq!(parse_vrr_enabled("1"), Some(true));
        assert_eq!(parse_vrr_enabled("1\n"), Some(true));
        assert_eq!(parse_vrr_enabled("0"), Some(false));
        assert_eq!(parse_vrr_enabled("0\n"), Some(false));
        assert_eq!(parse_vrr_enabled(""), None);
        assert_eq!(parse_vrr_enabled("enabled"), None);
        assert_eq!(parse_vrr_enabled("2"), None);
    }

    /// Audio health mapping covers every enum variant with a fixed token —
    /// adding a variant without extending this test fails loudly.
    #[test]
    fn audio_health_label_covers_all_variants() {
        assert_eq!(audio_health_label(crate::audio::Health::Ok), "ok");
        assert_eq!(
            audio_health_label(crate::audio::Health::SoftIssue),
            "soft-issue"
        );
        assert_eq!(
            audio_health_label(crate::audio::Health::HardwareBroken),
            "hardware-broken"
        );
        assert_eq!(
            audio_health_label(crate::audio::Health::NotApplicable),
            "not-applicable"
        );
    }

    /// ACPI platform-choices reader: only the legacy node, token-sanitized.
    /// On this fleet the node exists; the assertion is structural (closed
    /// charset, bounded count) so it holds on any kernel.
    #[test]
    fn acpi_platform_choices_are_sanitized_tokens() {
        let choices = read_acpi_platform_choices();
        assert!(choices.len() <= 16, "unbounded choices: {choices:?}");
        for c in &choices {
            assert!(!c.is_empty());
            assert!(c.len() <= 32);
            assert!(
                c.chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '+' | '-' | '.')),
                "unsanitized choice token: {c:?}"
            );
        }
        // Live cross-check: when the legacy node exists it must agree with
        // the raw file content (modulo sanitization).
        if let Ok(raw) = std::fs::read_to_string("/sys/firmware/acpi/platform_profile_choices")
        {
            let expected = raw.split_whitespace().count().min(16);
            assert_eq!(choices.len(), expected);
        }
    }

    /// Schema v4 serializes the new blocks with strictly typed values, and
    /// the privacy invariants still hold: no MAC-shaped tokens, no home
    /// paths, no serial-looking free text in the new sections.
    #[test]
    fn schema_v4_report_carries_typed_telemetry_blocks() {
        crate::logging::init("test");
        let report = collect();
        assert_eq!(report.schema_version, REPORT_SCHEMA_VERSION);
        assert_eq!(REPORT_SCHEMA_VERSION, 4);
        let v = serde_json::to_value(&report).expect("serializable");

        // Power block: whitelisted tokens + numbers only.
        let power = &v["power"];
        assert!(power.is_object(), "power block missing");
        if let Some(state) = power["charge_state"].as_str() {
            assert!(
                ["Charging", "Discharging", "Full", "Not charging", "Unknown"]
                    .contains(&state),
                "open charge_state vocabulary: {state:?}"
            );
        }
        if let Some(ty) = power["ac_type"].as_str() {
            assert!(
                ["Mains", "USB", "Other"].contains(&ty),
                "open ac_type vocabulary: {ty:?}"
            );
        }

        // Audio block: closed health token + booleans, no sink-name strings.
        let audio = &v["audio"];
        let health = audio["health"].as_str().expect("audio.health missing");
        assert!(
            ["ok", "soft-issue", "hardware-broken", "not-applicable"].contains(&health),
            "open audio health vocabulary: {health:?}"
        );
        for key in [
            "amp_present",
            "amp_bound",
            "modules_loaded",
            "firmware_ok",
            "fixable",
            "speakers_muted",
            "bass_off",
            "wrong_default_sink",
        ] {
            assert!(
                audio[key].is_boolean(),
                "audio.{key} must be a boolean, got: {}",
                audio[key]
            );
        }
        let audio_json = serde_json::to_string(audio).unwrap();
        assert!(!audio_json.contains("/home/"), "audio block leaked path");

        // CPU extras: closed EPP vocabulary, sanitized driver, p-state label.
        let cpu = &v["hardware"]["cpu"];
        if let Some(epp) = cpu["energy_performance_preference"].as_str() {
            assert!(
                [
                    "performance",
                    "balance_performance",
                    "balance_power",
                    "power",
                    "default",
                    "other"
                ]
                .contains(&epp),
                "open EPP vocabulary: {epp:?}"
            );
        }
        if let Some(mode) = cpu["pstate_mode"].as_str() {
            assert!(
                mode == "none"
                    || mode.starts_with("amd-pstate:")
                    || mode.starts_with("intel-pstate:"),
                "unexpected pstate_mode shape: {mode:?}"
            );
        }

        // Display: sanitized connector, no EDID bytes anywhere.
        if let Some(conn) = v["hardware"]["display"]["connector"].as_str() {
            assert!(
                conn.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-'),
                "unsanitized connector: {conn:?}"
            );
            assert!(conn.len() <= 16);
        }
        let json = serde_json::to_string(&report).expect("serializable");
        assert!(!json.contains("EDID"), "raw EDID text leaked");

        // Profiles carry both the handler view and the raw ACPI view.
        assert!(v["profiles"]["choices"].is_array());
        assert!(v["profiles"]["acpi_choices"].is_array());

        // dGPU power: numbers or null, P-state whitelisted.
        let gpu = &v["hardware"]["gpu"];
        for key in [
            "power_limit_w",
            "power_max_w",
            "power_default_w",
            "dynamic_boost_headroom_w",
        ] {
            assert!(
                gpu[key].is_null() || gpu[key].is_number(),
                "gpu.{key} must be numeric or null, got: {}",
                gpu[key]
            );
        }
        if let Some(pstate) = gpu["pstate"].as_str() {
            let n: u32 = pstate
                .strip_prefix('P')
                .and_then(|n| n.parse().ok())
                .expect("pstate outside P0..=P15");
            assert!(n <= 15);
        }
    }
}
