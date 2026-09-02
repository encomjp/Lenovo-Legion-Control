//! Unix socket IPC — CLI/GUI clients talk to the privileged legion-daemon.
//!
//! Variant order of existing commands/responses is frozen so older daemons
//! keep working for profile/fans/sensors. New Spectrum/charge commands are
//! appended at the end.

use bincode::Options;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Hard cap on IPC frame size (request or response). Prevents both
/// unbounded reads and bincode length-prefix allocation attacks.
pub const MAX_FRAME_BYTES: u64 = 4 * 1024 * 1024;

/// Bincode options used for ALL IPC frames. The byte limit makes
/// deserialization safe against hostile length prefixes: allocations are
/// clamped to the remaining limit instead of trusting the prefix.
pub fn bincode_opts() -> impl Options {
    bincode::options().with_limit(MAX_FRAME_BYTES)
}

/// Sanitize a client-supplied string for single-line logs: escape control
/// characters (prevents log forging via embedded newlines) and truncate.
pub fn sanitize_log(s: &str) -> String {
    let mut escaped = 0usize;
    let mut out = String::with_capacity(s.len().min(64));
    for c in s.chars().take(64) {
        match c {
            '\n' => {
                escaped += 1;
                out.push_str("\\n");
            }
            '\r' => {
                escaped += 1;
                out.push_str("\\r");
            }
            '\t' => out.push('\t'),
            c if c.is_control() => {
                escaped += 1;
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    let truncated = s.chars().count() > 64;
    if truncated {
        out.push('…');
    }
    // Trace only: runs once per client-supplied argument in a log label.
    log::trace!(
        "sanitize_log: in_len={} out_chars={} escaped={escaped} truncated={truncated}",
        s.chars().count(),
        out.chars().count()
    );
    out
}

#[derive(Debug, Serialize, Deserialize)]
pub enum DaemonCommand {
    // ── frozen ABI (do not reorder) ──
    GetSensors,
    GetProfile,
    SetProfile(String),
    GetFanRpm(u8),
    SetFanTarget(u8, u32),
    GetKbdBrightness,
    SetKbdBrightness(u8),
    SetRgbStatic(u8, u8, u8),
    GetBattery,
    SetConservation(bool),
    GetDeviceInfo,
    GetCameraPower,
    // ── appended (Gen 10 Spectrum + charge limits) ──
    SetRgbEffect {
        effect: String,
        r: u8,
        g: u8,
        b: u8,
        speed: u8,
    },
    SetRgbBrightness(u8),
    GetRgbBrightness,
    SetLogo(bool),
    SetChargeLimit(u32),
    GetChargeLimit,
    /// RAPL package watts (daemon must be root).
    GetCpuPower,
    /// Write firmware-attribute (e.g. ppt_pl1_spl) under lenovo-wmi-other.
    SetFwAttr {
        name: String,
        value: String,
    },
    /// SMT / hyperthreading on·off.
    GetSmt,
    SetSmt(bool),
    /// CPU frequency boost (turbo) on·off.
    GetBoost,
    SetBoost(bool),
    /// Spectrum RGB panic diagnose / autofix (HID + kernel USB faults).
    DiagnoseRgb,
    FixRgbPanic,
    /// Return recent in-memory log entries from the daemon.
    GetRecentLogs(usize),
    /// Change daemon log level at runtime.
    SetLogLevel(String),
    /// Read-only Curve Optimizer capability and per-core offsets.
    GetCurveOptimizer,
    /// Apply a temporary all-core Curve Optimizer offset.
    SetCurveOptimizer {
        offset: i16,
        acknowledge: bool,
    },
    /// Restore the exact baseline observed before Legion Control's first write.
    ResetCurveOptimizer,
    /// Restore the baseline with an explicit acknowledgement of the SMU write.
    ResetCurveOptimizerAcknowledged {
        acknowledge: bool,
    },
    /// Read delayed startup reapplication and recovery state.
    GetCurveOptimizerPersistence,
    /// Enable or disable delayed startup reapplication.
    SetCurveOptimizerPersistence {
        enabled: bool,
        offset: i16,
        acknowledge: bool,
    },
    GetThermal,
    SetThermal {
        enabled: bool,
        max_temp: u8,
        acknowledge: bool,
    },
    GetThermalStatus,
    GetDaemonVersion,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum DaemonResponse {
    // ── frozen ABI (do not reorder) ──
    Sensors(crate::sensors::SensorReadings),
    Profile(String),
    ProfileChoices(Vec<String>),
    FanRpm(u32),
    FanTarget(u32),
    KbdBrightness(u8),
    KbdMaxBrightness(u8),
    Battery {
        capacity: u32,
        status: String,
        voltage: f64,
        cycles: u32,
        conservation: bool,
    },
    DeviceInfo(crate::device::DeviceInfo),
    CameraPower(bool),
    Ok,
    Error(String),
    // ── appended ──
    RgbBrightness(u8),
    ChargeLimit(u32),
    CpuPower(f64),
    Smt {
        active: bool,
        control: String,
        logical_cpus: u32,
    },
    Boost(bool),
    /// RGB panic diagnosis (JSON-ish summary for CLI/GUI).
    RgbDiagnosis {
        health: String,
        summary: String,
        details: Vec<String>,
        fixable: bool,
    },
    RgbFixReport {
        steps: Vec<String>,
        errors: Vec<String>,
        health: String,
        summary: String,
    },
    /// Text block of recent log lines.
    RecentLogs(String),
    CurveOptimizer(crate::undervolt::CurveOptimizerStatus),
    CurveOptimizerPersistence(crate::undervolt::CurveOptimizerPersistence),
    Thermal(crate::thermal::ThermalConfig),
    ThermalStatus(crate::thermal::ThermalStatus),
    DaemonVersion(String),
}

/// System-wide socket used when the daemon runs as root (required for sysfs writes).
pub const SYSTEM_SOCKET: &str = "/run/legion-control.socket";

/// Per-user socket used by a non-root daemon. Only available when
/// XDG_RUNTIME_DIR is set — a predictable `/tmp` path would let any local
/// process pre-create the socket and impersonate the daemon.
fn user_socket() -> Option<PathBuf> {
    match std::env::var("XDG_RUNTIME_DIR") {
        Ok(dir) => {
            let path = PathBuf::from(dir).join("legion-control.socket");
            log::trace!("user_socket: XDG_RUNTIME_DIR → {}", path.display());
            Some(path)
        }
        Err(_) => {
            log::trace!("user_socket: XDG_RUNTIME_DIR unset — no per-user candidate");
            None
        }
    }
}

/// Path the daemon should bind.
/// Root → `/run/legion-control.socket` so profile/fan/battery writes work.
pub fn bind_socket_path() -> Result<PathBuf, String> {
    // SAFETY: geteuid() is a pure POSIX syscall with no memory safety requirements.
    if unsafe { libc::geteuid() } == 0 {
        log::debug!("bind_socket_path: euid=0 → {SYSTEM_SOCKET}");
        Ok(PathBuf::from(SYSTEM_SOCKET))
    } else {
        match user_socket() {
            Some(path) => {
                log::debug!("bind_socket_path: unprivileged → {}", path.display());
                Ok(path)
            }
            None => {
                let msg = "XDG_RUNTIME_DIR is not set — cannot pick a safe per-user socket path"
                    .to_string();
                log::warn!("bind_socket_path: {msg}");
                Err(msg)
            }
        }
    }
}

/// Candidate sockets for clients (system first, then per-user).
pub fn socket_candidates() -> Vec<PathBuf> {
    let mut candidates = vec![PathBuf::from(SYSTEM_SOCKET)];
    candidates.extend(user_socket());
    log::debug!(
        "socket candidates({}): [{}]",
        candidates.len(),
        candidates
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    candidates
}

/// Connect to the daemon, send a command, receive the response.
pub fn send_command(cmd: DaemonCommand) -> Result<DaemonResponse, String> {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    const IPC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

    let started = std::time::Instant::now();
    let label = cmd_label(&cmd);
    log::debug!("ipc → {label}");

    let mut last_err = String::from("No legion-control socket found");
    let mut perm_err: Option<String> = None;
    for path in socket_candidates() {
        let attempt_started = std::time::Instant::now();
        let mut stream = match UnixStream::connect(&path) {
            Ok(s) => {
                log::debug!(
                    "ipc {label}: connected to {} ({:?})",
                    path.display(),
                    attempt_started.elapsed()
                );
                s
            }
            Err(e) => {
                if e.kind() == std::io::ErrorKind::PermissionDenied {
                    perm_err = Some(format!("{}: {e}", path.display()));
                }
                last_err = format!("{}: {}", path.display(), e);
                match e.raw_os_error() {
                    Some(errno) => log::debug!(
                        "ipc {label}: connect failed on {} (errno {errno}, after {:?}): {e}",
                        path.display(),
                        attempt_started.elapsed()
                    ),
                    None => log::debug!(
                        "ipc {label}: connect failed on {} (after {:?}): {e}",
                        path.display(),
                        attempt_started.elapsed()
                    ),
                }
                continue;
            }
        };
        // Bound the wait so a wedged daemon cannot hang clients forever.
        if let Err(e) = stream.set_write_timeout(Some(IPC_TIMEOUT)) {
            log::warn!(
                "ipc {label}: set write timeout failed on {}: {e}",
                path.display()
            );
            return Err(format!("Set write timeout: {e}"));
        }
        if let Err(e) = stream.set_read_timeout(Some(IPC_TIMEOUT)) {
            log::warn!(
                "ipc {label}: set read timeout failed on {}: {e}",
                path.display()
            );
            return Err(format!("Set read timeout: {e}"));
        }
        log::trace!("ipc {label}: timeouts (write={IPC_TIMEOUT:?}, read={IPC_TIMEOUT:?})");

        let data = bincode_opts().serialize(&cmd).map_err(|e| {
            log::warn!("ipc {label}: serialize failed: {e}");
            format!("Serialize error: {e}")
        })?;
        log::trace!("ipc {label}: request frame {} B", data.len());

        // Manual write_all: tracks how many bytes actually left the process
        // so a failed write reports the exact partial count.
        let mut sent = 0usize;
        while sent < data.len() {
            match stream.write(&data[sent..]) {
                Ok(0) => {
                    log::warn!("ipc {label}: write stalled at {sent}/{} B", data.len());
                    return Err("Write error: made no progress".to_string());
                }
                Ok(n) => sent += n,
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    log::warn!(
                        "ipc {label}: write failed ({sent}/{} B sent): {e}",
                        data.len()
                    );
                    return Err(format!("Write error: {e}"));
                }
            }
        }
        log::debug!("ipc {label}: sent {sent} B");
        stream.shutdown(std::net::Shutdown::Write).ok();

        let mut buf = Vec::with_capacity(1024);
        stream
            .take(MAX_FRAME_BYTES)
            .read_to_end(&mut buf)
            .map_err(|e| {
                log::warn!("ipc {label}: read failed after {} B: {e}", buf.len());
                format!("Read error: {e}")
            })?;
        log::debug!("ipc {label}: received {} B", buf.len());

        let resp: DaemonResponse = bincode_opts().deserialize(&buf).map_err(|e| {
            log::warn!("ipc {label}: deserialize failed ({} B): {e}", buf.len());
            format!("Deserialize error: {e}")
        })?;
        log::trace!("ipc {label}: decoded {}", response_kind(&resp));

        let elapsed = started.elapsed();
        match &resp {
            DaemonResponse::Error(e) => log::warn!("ipc ← {label} error ({elapsed:?}): {e}"),
            DaemonResponse::Ok => log::debug!("ipc ← {label} ok ({elapsed:?})"),
            _ => log::debug!("ipc ← {label} ok ({elapsed:?}) ({})", response_kind(&resp)),
        }
        return Ok(resp);
    }

    let err_detail = perm_err.unwrap_or(last_err);
    let msg = format!(
        "{err_detail}. Start the daemon: sudo systemctl enable --now legion-control  (AppImage: open Settings → Fix → Enable daemon, or run: pkexec /usr/local/libexec/legion-control-setup enable-daemon)"
    );
    log::warn!("ipc ✗ {label} after {:?}: {msg}", started.elapsed());
    Err(msg)
}

/// Query the running daemon's version over IPC.
pub fn query_daemon_version() -> Result<String, String> {
    match send_command(DaemonCommand::GetDaemonVersion) {
        Ok(DaemonResponse::DaemonVersion(v)) => Ok(v),
        Ok(DaemonResponse::Error(e)) => Err(format!("daemon error (likely pre-v0.2.11): {e}")),
        Ok(other) => Err(format!("unexpected response: {other:?}")),
        Err(e) => Err(e),
    }
}

/// Short label for logs (avoids dumping large sensor payloads).
/// Client-supplied strings are sanitized (control chars escaped, truncated)
/// so a hostile client cannot forge log lines.
pub fn cmd_label(cmd: &DaemonCommand) -> String {
    let label = match cmd {
        DaemonCommand::GetSensors => "GetSensors".into(),
        DaemonCommand::GetProfile => "GetProfile".into(),
        DaemonCommand::SetProfile(name) => format!("SetProfile({})", sanitize_log(name)),
        DaemonCommand::GetFanRpm(f) => format!("GetFanRpm({f})"),
        DaemonCommand::SetFanTarget(f, rpm) => format!("SetFanTarget({f},{rpm})"),
        DaemonCommand::GetKbdBrightness => "GetKbdBrightness".into(),
        DaemonCommand::SetKbdBrightness(l) => format!("SetKbdBrightness({l})"),
        DaemonCommand::SetRgbStatic(r, g, b) => format!("SetRgbStatic(#{r:02x}{g:02x}{b:02x})"),
        DaemonCommand::GetBattery => "GetBattery".into(),
        DaemonCommand::SetConservation(on) => format!("SetConservation({on})"),
        DaemonCommand::GetDeviceInfo => "GetDeviceInfo".into(),
        DaemonCommand::GetCameraPower => "GetCameraPower".into(),
        DaemonCommand::SetRgbEffect {
            effect,
            r,
            g,
            b,
            speed,
        } => format!(
            "SetRgbEffect({},#{r:02x}{g:02x}{b:02x},spd={speed})",
            sanitize_log(effect)
        ),
        DaemonCommand::SetRgbBrightness(l) => format!("SetRgbBrightness({l})"),
        DaemonCommand::GetRgbBrightness => "GetRgbBrightness".into(),
        DaemonCommand::SetLogo(on) => format!("SetLogo({on})"),
        DaemonCommand::SetChargeLimit(p) => format!("SetChargeLimit({p}%)"),
        DaemonCommand::GetChargeLimit => "GetChargeLimit".into(),
        DaemonCommand::GetCpuPower => "GetCpuPower".into(),
        DaemonCommand::SetFwAttr { name, value } => {
            format!("SetFwAttr({}={})", sanitize_log(name), sanitize_log(value))
        }
        DaemonCommand::GetSmt => "GetSmt".into(),
        DaemonCommand::SetSmt(on) => format!("SetSmt({on})"),
        DaemonCommand::GetBoost => "GetBoost".into(),
        DaemonCommand::SetBoost(on) => format!("SetBoost({on})"),
        DaemonCommand::DiagnoseRgb => "DiagnoseRgb".into(),
        DaemonCommand::FixRgbPanic => "FixRgbPanic".into(),
        DaemonCommand::GetRecentLogs(n) => format!("GetRecentLogs({n})"),
        DaemonCommand::SetLogLevel(l) => format!("SetLogLevel({})", sanitize_log(l)),
        DaemonCommand::GetCurveOptimizer => "GetCurveOptimizer".into(),
        DaemonCommand::SetCurveOptimizer { offset, .. } => {
            format!("SetCurveOptimizer({offset})")
        }
        DaemonCommand::ResetCurveOptimizer => "ResetCurveOptimizer".into(),
        DaemonCommand::ResetCurveOptimizerAcknowledged { .. } => {
            "ResetCurveOptimizerAcknowledged".into()
        }
        DaemonCommand::GetCurveOptimizerPersistence => "GetCurveOptimizerPersistence".into(),
        DaemonCommand::SetCurveOptimizerPersistence {
            enabled, offset, ..
        } => format!("SetCurveOptimizerPersistence({enabled},{offset})"),
        DaemonCommand::GetThermal => "GetThermal".into(),
        DaemonCommand::SetThermal {
            enabled, max_temp, ..
        } => format!("SetThermal({enabled},{max_temp})"),
        DaemonCommand::GetThermalStatus => "GetThermalStatus".into(),
        DaemonCommand::GetDaemonVersion => "GetDaemonVersion".into(),
    };
    // Per-command classifier — trace only to keep hot paths quiet.
    log::trace!("cmd_label: {label}");
    label
}

/// Fixed command-kind label (no client data) — safe as a bounded map key
/// for per-command timing statistics.
pub fn cmd_kind(cmd: &DaemonCommand) -> &'static str {
    let kind = match cmd {
        DaemonCommand::GetSensors => "GetSensors",
        DaemonCommand::GetProfile => "GetProfile",
        DaemonCommand::SetProfile(_) => "SetProfile",
        DaemonCommand::GetFanRpm(_) => "GetFanRpm",
        DaemonCommand::SetFanTarget(_, _) => "SetFanTarget",
        DaemonCommand::GetKbdBrightness => "GetKbdBrightness",
        DaemonCommand::SetKbdBrightness(_) => "SetKbdBrightness",
        DaemonCommand::SetRgbStatic(_, _, _) => "SetRgbStatic",
        DaemonCommand::GetBattery => "GetBattery",
        DaemonCommand::SetConservation(_) => "SetConservation",
        DaemonCommand::GetDeviceInfo => "GetDeviceInfo",
        DaemonCommand::GetCameraPower => "GetCameraPower",
        DaemonCommand::SetRgbEffect { .. } => "SetRgbEffect",
        DaemonCommand::SetRgbBrightness(_) => "SetRgbBrightness",
        DaemonCommand::GetRgbBrightness => "GetRgbBrightness",
        DaemonCommand::SetLogo(_) => "SetLogo",
        DaemonCommand::SetChargeLimit(_) => "SetChargeLimit",
        DaemonCommand::GetChargeLimit => "GetChargeLimit",
        DaemonCommand::GetCpuPower => "GetCpuPower",
        DaemonCommand::SetFwAttr { .. } => "SetFwAttr",
        DaemonCommand::GetSmt => "GetSmt",
        DaemonCommand::SetSmt(_) => "SetSmt",
        DaemonCommand::GetBoost => "GetBoost",
        DaemonCommand::SetBoost(_) => "SetBoost",
        DaemonCommand::DiagnoseRgb => "DiagnoseRgb",
        DaemonCommand::FixRgbPanic => "FixRgbPanic",
        DaemonCommand::GetRecentLogs(_) => "GetRecentLogs",
        DaemonCommand::SetLogLevel(_) => "SetLogLevel",
        DaemonCommand::GetCurveOptimizer => "GetCurveOptimizer",
        DaemonCommand::SetCurveOptimizer { .. } => "SetCurveOptimizer",
        DaemonCommand::ResetCurveOptimizer => "ResetCurveOptimizer",
        DaemonCommand::ResetCurveOptimizerAcknowledged { .. } => "ResetCurveOptimizerAcknowledged",
        DaemonCommand::GetCurveOptimizerPersistence => "GetCurveOptimizerPersistence",
        DaemonCommand::SetCurveOptimizerPersistence { .. } => "SetCurveOptimizerPersistence",
        DaemonCommand::GetThermal => "GetThermal",
        DaemonCommand::SetThermal { .. } => "SetThermal",
        DaemonCommand::GetThermalStatus => "GetThermalStatus",
        DaemonCommand::GetDaemonVersion => "GetDaemonVersion",
    };
    // Per-command classifier — trace only to keep hot paths quiet.
    log::trace!("cmd_kind: {kind}");
    kind
}

pub fn response_kind(resp: &DaemonResponse) -> &'static str {
    match resp {
        DaemonResponse::Sensors(_) => "Sensors",
        DaemonResponse::Profile(_) => "Profile",
        DaemonResponse::ProfileChoices(_) => "ProfileChoices",
        DaemonResponse::FanRpm(_) => "FanRpm",
        DaemonResponse::FanTarget(_) => "FanTarget",
        DaemonResponse::KbdBrightness(_) => "KbdBrightness",
        DaemonResponse::KbdMaxBrightness(_) => "KbdMaxBrightness",
        DaemonResponse::Battery { .. } => "Battery",
        DaemonResponse::DeviceInfo(_) => "DeviceInfo",
        DaemonResponse::CameraPower(_) => "CameraPower",
        DaemonResponse::Ok => "Ok",
        DaemonResponse::Error(_) => "Error",
        DaemonResponse::RgbBrightness(_) => "RgbBrightness",
        DaemonResponse::ChargeLimit(_) => "ChargeLimit",
        DaemonResponse::CpuPower(_) => "CpuPower",
        DaemonResponse::Smt { .. } => "Smt",
        DaemonResponse::Boost(_) => "Boost",
        DaemonResponse::RgbDiagnosis { .. } => "RgbDiagnosis",
        DaemonResponse::RgbFixReport { .. } => "RgbFixReport",
        DaemonResponse::RecentLogs(_) => "RecentLogs",
        DaemonResponse::CurveOptimizer(_) => "CurveOptimizer",
        DaemonResponse::CurveOptimizerPersistence(_) => "CurveOptimizerPersistence",
        DaemonResponse::Thermal(_) => "Thermal",
        DaemonResponse::ThermalStatus(_) => "ThermalStatus",
        DaemonResponse::DaemonVersion(_) => "DaemonVersion",
    }
}

/// True when the command mutates hardware / firmware state.
pub fn cmd_is_write(cmd: &DaemonCommand) -> bool {
    let is_write = matches!(
        cmd,
        DaemonCommand::SetProfile(_)
            | DaemonCommand::SetFanTarget(_, _)
            | DaemonCommand::SetKbdBrightness(_)
            | DaemonCommand::SetRgbStatic(_, _, _)
            | DaemonCommand::SetConservation(_)
            | DaemonCommand::SetRgbEffect { .. }
            | DaemonCommand::SetRgbBrightness(_)
            | DaemonCommand::SetLogo(_)
            | DaemonCommand::SetChargeLimit(_)
            | DaemonCommand::SetFwAttr { .. }
            | DaemonCommand::SetSmt(_)
            | DaemonCommand::SetBoost(_)
            | DaemonCommand::SetCurveOptimizer { .. }
            | DaemonCommand::ResetCurveOptimizer
            | DaemonCommand::ResetCurveOptimizerAcknowledged { .. }
            | DaemonCommand::SetCurveOptimizerPersistence { .. }
            | DaemonCommand::SetThermal { .. }
            | DaemonCommand::FixRgbPanic
    );
    // Per-command classifier — trace only to keep hot paths quiet.
    log::trace!("cmd_is_write: {is_write}");
    is_write
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thermal_ipc_round_trip() {
        let cmd = DaemonCommand::SetThermal {
            enabled: true,
            max_temp: 90,
            acknowledge: false,
        };
        let bytes = bincode_opts().serialize(&cmd).unwrap();
        let back: DaemonCommand = bincode_opts().deserialize(&bytes).unwrap();
        assert!(matches!(
            back,
            DaemonCommand::SetThermal { max_temp: 90, .. }
        ));
    }

    #[test]
    fn sanitize_log_escapes_newlines_and_truncates() {
        assert_eq!(sanitize_log("balanced"), "balanced");
        assert_eq!(sanitize_log("foo\nbar"), "foo\\nbar");
        assert_eq!(sanitize_log("a\u{0001}b"), "a\\u0001b");
        let long = "x".repeat(100);
        let out = sanitize_log(&long);
        assert!(out.chars().count() == 65 && out.ends_with('…'));
    }

    #[test]
    fn sanitize_log_escapes_cr_and_tab() {
        assert_eq!(sanitize_log("a\rb"), "a\\rb");
        assert!(sanitize_log("a\tb").contains('\t'));
    }

    #[test]
    fn cmd_kind_is_bounded_and_independent_of_payload() {
        // Client-controlled strings must not create distinct map keys.
        for name in ["foo", "bar\nbaz", "x".repeat(200).as_str()] {
            assert_eq!(
                cmd_kind(&DaemonCommand::SetProfile(name.to_string())),
                "SetProfile"
            );
        }
        assert_eq!(
            cmd_kind(&DaemonCommand::SetFwAttr {
                name: "ppt_pl1_spl".into(),
                value: "9999".into()
            }),
            "SetFwAttr"
        );
        assert_eq!(
            cmd_kind(&DaemonCommand::SetLogLevel("trace\ninject".into())),
            "SetLogLevel"
        );
    }

    #[test]
    fn cmd_label_sanitizes_injected_newlines() {
        let label = cmd_label(&DaemonCommand::SetProfile("foo\nbar".into()));
        assert!(!label.contains('\n'));
        assert!(label.contains("\\n"));
    }

    #[test]
    fn ipc_frame_rejects_oversized_payload_via_limit() {
        // A normal command serializes well under the cap; a huge string must
        // fail to serialize when the bincode limit is enforced.
        let normal = DaemonCommand::GetSensors;
        assert!(bincode_opts().serialize(&normal).is_ok());
        let huge = DaemonCommand::SetProfile("x".repeat((MAX_FRAME_BYTES + 1) as usize));
        assert!(bincode_opts().serialize(&huge).is_err());
    }

    #[test]
    fn socket_candidates_never_includes_tmp() {
        // The /tmp fallback was removed to block daemon impersonation.
        // socket_candidates must only contain /run and (optionally) XDG_RUNTIME_DIR.
        let saved = std::env::var("XDG_RUNTIME_DIR").ok();
        unsafe {
            // SAFETY: single-threaded test — env is not mutated concurrently here.
            std::env::remove_var("XDG_RUNTIME_DIR");
        }
        let cands = socket_candidates();
        assert!(cands
            .iter()
            .any(|p| p == std::path::Path::new(SYSTEM_SOCKET)));
        assert!(!cands.iter().any(|p| p.starts_with("/tmp")));
        if let Some(v) = saved {
            unsafe { std::env::set_var("XDG_RUNTIME_DIR", v) };
        }
    }

    #[test]
    fn bind_socket_path_errors_without_xdg_when_unprivileged() {
        if unsafe { libc::geteuid() } == 0 {
            // Root always binds the system socket — no env dependency.
            assert_eq!(bind_socket_path().unwrap().to_str().unwrap(), SYSTEM_SOCKET);
            return;
        }
        let saved = std::env::var("XDG_RUNTIME_DIR").ok();
        unsafe { std::env::remove_var("XDG_RUNTIME_DIR") };
        assert!(bind_socket_path().is_err());
        if let Some(v) = saved {
            unsafe { std::env::set_var("XDG_RUNTIME_DIR", v) };
        }
    }

    /// The GUI and daemon evolve independently — every command/response
    /// variant must survive a wire round-trip byte-for-byte.
    #[test]
    fn daemon_commands_round_trip_over_the_wire() {
        let cmds = [
            DaemonCommand::GetSensors,
            DaemonCommand::GetProfile,
            DaemonCommand::SetProfile("custom".into()),
            DaemonCommand::GetFanRpm(2),
            DaemonCommand::SetFanTarget(1, 4400),
            DaemonCommand::SetFwAttr {
                name: "ppt_pl1_spl".into(),
                value: "80".into(),
            },
            DaemonCommand::SetChargeLimit(80),
            DaemonCommand::SetSmt(false),
            DaemonCommand::SetBoost(true),
            DaemonCommand::GetThermalStatus,
            DaemonCommand::SetThermal {
                enabled: true,
                max_temp: 92,
                acknowledge: false,
            },
            DaemonCommand::GetCurveOptimizer,
            DaemonCommand::SetCurveOptimizer {
                offset: -15,
                acknowledge: true,
            },
            DaemonCommand::ResetCurveOptimizerAcknowledged { acknowledge: true },
            DaemonCommand::GetCurveOptimizerPersistence,
            DaemonCommand::SetCurveOptimizerPersistence {
                enabled: true,
                offset: -15,
                acknowledge: true,
            },
            DaemonCommand::GetRecentLogs(100),
            DaemonCommand::SetLogLevel("debug".into()),
        ];
        for cmd in &cmds {
            let bytes = bincode_opts().serialize(cmd).expect("serialize");
            let back: DaemonCommand = bincode_opts().deserialize(&bytes).expect("deserialize");
            assert_eq!(bincode_opts().serialize(&back).unwrap(), bytes);
        }
    }

    #[test]
    fn daemon_responses_round_trip_over_the_wire() {
        let resps = [
            DaemonResponse::Ok,
            DaemonResponse::Error("Parse: byte 2".into()),
            DaemonResponse::Profile("performance".into()),
            DaemonResponse::FanRpm(4400),
            DaemonResponse::FanTarget(0),
            DaemonResponse::ChargeLimit(80),
            DaemonResponse::Smt {
                active: true,
                control: "on".into(),
                logical_cpus: 32,
            },
            DaemonResponse::ThermalStatus(crate::thermal::ThermalStatus {
                config: crate::thermal::ThermalConfig {
                    enabled: true,
                    max_temp: 92,
                },
                active: false,
                cpu_temp_mc: Some(82_000),
                cpu_temp_2_mc: Some(75_200),
                cur_max_freq: 5_460_527,
                restore_temp: 85,
            }),
            DaemonResponse::CurveOptimizer(crate::undervolt::CurveOptimizerStatus {
                available: true,
                reason: "probe".into(),
                codename: Some(23),
                driver_version: Some("0.1.7".into()),
                firmware_version: Some("4.98.26.0".into()),
                current: vec![-15; 16],
                boot_baseline: vec![-4; 16],
                previous: Some(-4),
                minimum: -30,
                maximum: 0,
                temporary_only: true,
            }),
        ];
        for resp in &resps {
            let bytes = bincode_opts().serialize(resp).expect("serialize");
            let back: DaemonResponse = bincode_opts().deserialize(&bytes).expect("deserialize");
            assert_eq!(bincode_opts().serialize(&back).unwrap(), bytes);
        }
    }

    /// Mirrors the live robustness probe. Short/garbage frames CAN decode
    /// into write commands with degenerate values — the daemon must reject
    /// those server-side. Pin both halves of that contract:
    /// - all-FF / plain junk / empty never decode at all;
    /// - the 2-byte frame [0x02, 0x00] decodes to SetProfile(""), which
    ///   profile::set refuses before any hardware write.
    #[test]
    fn garbage_frames_decode_only_into_degenerate_writes_that_are_rejected() {
        for raw in [&[0xff_u8; 64][..], &[1, 2, 3, 4, 5, 6, 7, 8][..], &[][..]] {
            let decoded: Result<DaemonCommand, _> = bincode_opts().deserialize(raw);
            assert!(decoded.is_err(), "{raw:?} should not decode into a command");
        }
        let decoded: DaemonCommand = bincode_opts()
            .deserialize(&[0x02, 0x00])
            .expect("known degenerate frame");
        match decoded {
            DaemonCommand::SetProfile(name) => {
                assert!(name.is_empty(), "expected the degenerate empty name");
                assert_eq!(
                    crate::profile::set(&name).unwrap_err(),
                    "empty profile name"
                );
            }
            other => panic!("unexpected decode: {}", cmd_label(&other)),
        }
    }
}
