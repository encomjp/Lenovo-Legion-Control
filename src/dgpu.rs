//! NVIDIA dGPU monitoring via nvidia-smi subprocess.
//!
//! The NVIDIA GPU has no hwmon interface on Linux — we use nvidia-smi.
//! A 3-second timeout prevents hanging the daemon if the driver is
//! unresponsive.

use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const SMI_TIMEOUT: Duration = Duration::from_secs(3);

/// Run nvidia-smi with a timeout. If the subprocess hasn't exited within
/// `SMI_TIMEOUT`, `None` is returned.
fn smi_run(args: &[&str]) -> Option<String> {
    let (tx, rx) = mpsc::channel();
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    thread::spawn(move || {
        let output = Command::new("/usr/bin/nvidia-smi").args(&args).output();
        let _ = tx.send(output);
    });
    match rx.recv_timeout(SMI_TIMEOUT) {
        Ok(Ok(output)) if output.status.success() => {
            let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        }
        Ok(Ok(_)) => None,
        Ok(Err(e)) => {
            log::debug!("nvidia-smi spawn failed: {e}");
            None
        }
        Err(_) => {
            log::warn!("nvidia-smi timed out after {}s", SMI_TIMEOUT.as_secs());
            None
        }
    }
}

pub fn smi_query(query: &str) -> Option<String> {
    smi_run(&["--query-gpu", query, "--format=csv,noheader,nounits"])
}

pub fn read_temp() -> Option<f64> {
    smi_query("temperature.gpu")?.parse().ok()
}

pub fn read_power() -> Option<f64> {
    smi_query("power.draw")?.parse().ok()
}

pub fn read_clock() -> Option<f64> {
    smi_query("clocks.gr")?.parse().ok()
}

pub fn read_util() -> Option<f64> {
    smi_query("utilization.gpu")?.parse().ok()
}

/// NVIDIA driver maximum power limit (W) — e.g. 175 on RTX 5080 Legion Pro 7.
pub fn read_power_max() -> Option<f64> {
    smi_query("power.max_limit")?.parse().ok()
}

/// NVIDIA driver default / base power limit (W).
pub fn read_power_default() -> Option<f64> {
    smi_query("power.default_limit")?.parse().ok()
}

pub fn is_available() -> bool {
    smi_run(&["-L"]).is_some()
}
