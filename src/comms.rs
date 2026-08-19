//! Unix socket IPC — CLI/GUI clients talk to the privileged legion-daemon.
//!
//! Variant order of existing commands/responses is frozen so older daemons
//! keep working for profile/fans/sensors. New Spectrum/charge commands are
//! appended at the end.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
}

/// System-wide socket used when the daemon runs as root (required for sysfs writes).
pub const SYSTEM_SOCKET: &str = "/run/legion-control.socket";

fn user_socket() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(dir).join("legion-control.socket")
    } else {
        PathBuf::from("/tmp/legion-control.socket")
    }
}

/// Path the daemon should bind.
/// Root → `/run/legion-control.socket` so profile/fan/battery writes work.
pub fn bind_socket_path() -> PathBuf {
    // SAFETY: geteuid() is a pure POSIX syscall with no memory safety requirements.
    if unsafe { libc::geteuid() } == 0 {
        PathBuf::from(SYSTEM_SOCKET)
    } else {
        user_socket()
    }
}

/// Candidate sockets for clients (system first, then per-user).
pub fn socket_candidates() -> Vec<PathBuf> {
    vec![PathBuf::from(SYSTEM_SOCKET), user_socket()]
}

/// Connect to the daemon, send a command, receive the response.
pub fn send_command(cmd: DaemonCommand) -> Result<DaemonResponse, String> {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    let label = cmd_label(&cmd);
    log::debug!("ipc → {label}");

    let mut last_err = String::from("No legion-control socket found");
    for path in socket_candidates() {
        let mut stream = match UnixStream::connect(&path) {
            Ok(s) => s,
            Err(e) => {
                last_err = format!("{}: {}", path.display(), e);
                log::debug!("ipc connect failed on {}: {e}", path.display());
                continue;
            }
        };

        let data = bincode::serialize(&cmd).map_err(|e| format!("Serialize error: {e}"))?;
        stream
            .write_all(&data)
            .map_err(|e| format!("Write error: {e}"))?;
        stream.shutdown(std::net::Shutdown::Write).ok();

        let mut buf = Vec::new();
        stream
            .read_to_end(&mut buf)
            .map_err(|e| format!("Read error: {e}"))?;

        let resp: DaemonResponse =
            bincode::deserialize(&buf).map_err(|e| format!("Deserialize error: {e}"))?;
        match &resp {
            DaemonResponse::Error(e) => log::warn!("ipc ← {label} error: {e}"),
            DaemonResponse::Ok => log::debug!("ipc ← {label} ok"),
            _ => log::debug!("ipc ← {label} ok ({})", response_kind(&resp)),
        }
        return Ok(resp);
    }

    let msg = format!("{last_err}. Start the daemon: sudo systemctl enable --now legion-control");
    log::warn!("ipc ✗ {label}: {msg}");
    Err(msg)
}

/// Short label for logs (avoids dumping large sensor payloads).
pub fn cmd_label(cmd: &DaemonCommand) -> String {
    match cmd {
        DaemonCommand::GetSensors => "GetSensors".into(),
        DaemonCommand::GetProfile => "GetProfile".into(),
        DaemonCommand::SetProfile(name) => format!("SetProfile({name})"),
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
        } => format!("SetRgbEffect({effect},#{r:02x}{g:02x}{b:02x},spd={speed})"),
        DaemonCommand::SetRgbBrightness(l) => format!("SetRgbBrightness({l})"),
        DaemonCommand::GetRgbBrightness => "GetRgbBrightness".into(),
        DaemonCommand::SetLogo(on) => format!("SetLogo({on})"),
        DaemonCommand::SetChargeLimit(p) => format!("SetChargeLimit({p}%)"),
        DaemonCommand::GetChargeLimit => "GetChargeLimit".into(),
        DaemonCommand::GetCpuPower => "GetCpuPower".into(),
        DaemonCommand::SetFwAttr { name, value } => format!("SetFwAttr({name}={value})"),
        DaemonCommand::GetSmt => "GetSmt".into(),
        DaemonCommand::SetSmt(on) => format!("SetSmt({on})"),
        DaemonCommand::GetBoost => "GetBoost".into(),
        DaemonCommand::SetBoost(on) => format!("SetBoost({on})"),
        DaemonCommand::DiagnoseRgb => "DiagnoseRgb".into(),
        DaemonCommand::FixRgbPanic => "FixRgbPanic".into(),
        DaemonCommand::GetRecentLogs(n) => format!("GetRecentLogs({n})"),
        DaemonCommand::SetLogLevel(l) => format!("SetLogLevel({l})"),
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
            enabled,
            max_temp,
            ..
        } => format!("SetThermal({enabled},{max_temp})"),
        DaemonCommand::GetThermalStatus => "GetThermalStatus".into(),
    }
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
    }
}

/// True when the command mutates hardware / firmware state.
pub fn cmd_is_write(cmd: &DaemonCommand) -> bool {
    matches!(
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
    )
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
        let bytes = bincode::serialize(&cmd).unwrap();
        let back: DaemonCommand = bincode::deserialize(&bytes).unwrap();
        assert!(matches!(back, DaemonCommand::SetThermal { max_temp: 90, .. }));
    }
}
