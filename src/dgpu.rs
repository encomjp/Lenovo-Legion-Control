//! NVIDIA dGPU monitoring via nvidia-smi subprocess.
//!
//! The NVIDIA GPU has no hwmon interface on Linux — we use nvidia-smi.
//! A 3-second timeout prevents hanging the daemon if the driver is
//! unresponsive. On timeout the child is SIGKILLed so the reaper thread
//! always finishes (no leaked threads or zombies).

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

const SMI_TIMEOUT: Duration = Duration::from_secs(3);

static SMI_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

fn find_nvidia_smi() -> Option<&'static PathBuf> {
    SMI_PATH
        .get_or_init(|| {
            for p in &[
                "/usr/bin/nvidia-smi",
                "/usr/local/bin/nvidia-smi",
                "/opt/bin/nvidia-smi",
            ] {
                let pb = PathBuf::from(p);
                if pb.exists() {
                    return Some(pb);
                }
            }
            // Try resolving from PATH
            if let Ok(path_var) = std::env::var("PATH") {
                for dir in std::env::split_paths(&path_var) {
                    let candidate = dir.join("nvidia-smi");
                    if candidate.exists() {
                        return Some(candidate);
                    }
                }
            }
            None
        })
        .as_ref()
}

/// Run nvidia-smi with a timeout. If the subprocess hasn't exited within
/// `SMI_TIMEOUT`, it is killed and `None` is returned.
fn smi_run(args: &[&str]) -> Option<String> {
    let smi_bin = find_nvidia_smi()?;
    let started = Instant::now();
    log::debug!(
        "nvidia-smi: spawning {} {}",
        smi_bin.display(),
        args.join(" ")
    );
    let child = match Command::new(smi_bin)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            log::debug!("nvidia-smi spawn failed: {e}");
            return None;
        }
    };
    let pid = child.id();
    log::trace!(
        "nvidia-smi: pid {pid} running — arming {}s timeout",
        SMI_TIMEOUT.as_secs()
    );
    let (tx, rx) = mpsc::channel();
    // Reaper thread: owns the child so wait() reaps it even after a timeout
    // kill — the thread always terminates once the process dies.
    thread::spawn(move || {
        if tx.send(child.wait_with_output()).is_err() {
            log::debug!(
                "nvidia-smi: reaper for pid {pid} finished but receiver is gone \
                 (timeout path already returned) — child reaped anyway"
            );
        }
    });
    match rx.recv_timeout(SMI_TIMEOUT) {
        Ok(Ok(output)) if output.status.success() => {
            let stdout_len = output.stdout.len();
            let stderr_len = output.stderr.len();
            let elapsed_ms = started.elapsed().as_millis();
            log::debug!(
                "nvidia-smi ok: exit=0, stdout {stdout_len} B, stderr {stderr_len} B, {elapsed_ms} ms"
            );
            let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if s.is_empty() {
                log::debug!("nvidia-smi: empty stdout — treated as no data");
                None
            } else {
                Some(s)
            }
        }
        Ok(Ok(output)) => {
            let elapsed_ms = started.elapsed().as_millis();
            log::debug!(
                "nvidia-smi failed: exit={:?}, stdout {} B, stderr {} B ({:?}), {elapsed_ms} ms",
                output.status.code(),
                output.stdout.len(),
                output.stderr.len(),
                String::from_utf8_lossy(&output.stderr).trim(),
            );
            None
        }
        Ok(Err(e)) => {
            log::debug!("nvidia-smi wait failed: {e}");
            None
        }
        Err(_) => {
            log::warn!(
                "nvidia-smi timed out after {}s — killing pid {pid}",
                SMI_TIMEOUT.as_secs()
            );
            // SAFETY: kill(pid, SIGKILL) is a plain syscall; the pid belongs
            // to our own child. The reaper thread then reaps it and exits.
            unsafe {
                let rc = libc::kill(pid as libc::pid_t, libc::SIGKILL);
                if rc != 0 {
                    log::debug!(
                        "nvidia-smi: SIGKILL to pid {pid} failed: {} (likely already exited)",
                        std::io::Error::last_os_error()
                    );
                }
            }
            log::debug!("nvidia-smi: SIGKILL sent to pid {pid} after timeout");
            None
        }
    }
}

pub fn smi_query(query: &str) -> Option<String> {
    let value = smi_run(&["--query-gpu", query, "--format=csv,noheader,nounits"]);
    match &value {
        Some(v) => log::debug!("nvidia-smi query {query:?} → {v:?}"),
        None => log::debug!("nvidia-smi query {query:?} → no data"),
    }
    value
}

/// Full `nvidia-smi -q` dump (trimmed of the noisy header). Returns None on
/// AMD-only machines. Cap ~16 KB — deep reports only, never minute pushes.
pub fn detailed_query() -> Option<String> {
    let raw = smi_run(&["-q"])?;
    // Trim the fixed header block (timestamp etc.) — keep from "Driver" on.
    let body = match raw.find("Driver Version") {
        Some(i) => &raw[i..],
        None => raw.as_str(),
    };
    const MAX: usize = 32 * 1024;
    let mut out: String = body.chars().take(MAX).collect();
    if body.len() > MAX {
        out.push_str("\n… [truncated]");
    }
    Some(out)
}

/// Batch snapshot of dGPU metrics in a single subprocess execution.
#[derive(Debug, Clone, Copy, Default)]
pub struct DgpuMetrics {
    pub temp: Option<f64>,
    pub power: Option<f64>,
    pub clock: Option<f64>,
    pub util: Option<f64>,
}

/// Query temperature, power, clock and utilization in a SINGLE nvidia-smi execution.
pub fn read_metrics_batch() -> DgpuMetrics {
    let raw = match smi_query("temperature.gpu,power.draw,clocks.gr,utilization.gpu") {
        Some(r) => r,
        None => return DgpuMetrics::default(),
    };
    let parts: Vec<&str> = raw.split(',').map(|s| s.trim()).collect();
    DgpuMetrics {
        temp: parts.first().and_then(|s| s.parse().ok()),
        power: parts.get(1).and_then(|s| s.parse().ok()),
        clock: parts.get(2).and_then(|s| s.parse().ok()),
        util: parts.get(3).and_then(|s| s.parse().ok()),
    }
}

/// True when this machine HAS a discrete NVIDIA GPU. Stable per boot —
/// resolved once from the PCI scan (dGPU = a GPU-vendor card that is not
/// boot_vga) plus nvidia-smi availability. Used to distinguish "no dGPU
/// hardware" (IdeaPad) from "dGPU powered down" (LOQ RTX 5050 idle).
pub fn discrete_present() -> bool {
    static PRESENT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *PRESENT.get_or_init(|| {
        // nvidia-smi resolvable → NVIDIA dGPU present (it only exists with
        // the driver stack installed).
        if find_nvidia_smi().is_some() {
            return true;
        }
        // AMD hybrid: second GPU-vendor card that is not boot_vga.
        let mut gpu_cards = 0;
        let mut non_boot = 0;
        if let Ok(entries) = std::fs::read_dir("/sys/class/drm") {
            for entry in entries.flatten() {
                let dev = entry.path().join("device");
                let vendor = std::fs::read_to_string(dev.join("vendor"))
                    .map(|v| v.trim().to_ascii_lowercase())
                    .unwrap_or_default();
                if vendor != "0x10de" && vendor != "0x1002" {
                    continue;
                }
                gpu_cards += 1;
                let boot = std::fs::read_to_string(dev.join("boot_vga"))
                    .map(|v| v.trim() == "1")
                    .unwrap_or(false);
                if !boot {
                    non_boot += 1;
                }
            }
        }
        gpu_cards >= 2 && non_boot >= 1
    })
}

/// Lifecycle label for the dGPU chip: Ok when reading metrics, Off when the
/// hardware exists but reports nothing (runtime-suspended), None when there
/// is no discrete GPU at all. Once a real reading is seen this boot, the
/// card is remembered as Present — a subsequent nvidia-smi failure then
/// reads "Inactive" (driver asleep), not "absent".
pub fn discrete_state() -> &'static str {
    static EVER_LIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !discrete_present() {
        return "absent";
    }
    let metrics = read_metrics_batch();
    if metrics.temp.is_some() {
        EVER_LIVE.store(true, std::sync::atomic::Ordering::Relaxed);
        return "active";
    }
    if EVER_LIVE.load(std::sync::atomic::Ordering::Relaxed) {
        "inactive"
    } else {
        // Never live this boot AND nvidia-smi absent → likely no dGPU or
        // driver unloaded; either way the card never reported in.
        "off"
    }
}

pub fn read_temp() -> Option<f64> {
    read_metrics_batch().temp
}

pub fn read_power() -> Option<f64> {
    read_metrics_batch().power
}

pub fn read_clock() -> Option<f64> {
    read_metrics_batch().clock
}

pub fn read_util() -> Option<f64> {
    read_metrics_batch().util
}

/// NVIDIA driver maximum power limit (W) — e.g. 175 on RTX 5080 Legion Pro 7.
pub fn read_power_max() -> Option<f64> {
    let raw = smi_query("power.max_limit")?;
    match raw.parse::<f64>() {
        Ok(v) => {
            log::debug!("gpu power max_limit: {v} W");
            Some(v)
        }
        Err(e) => {
            log::warn!("gpu power max_limit parse failed for {raw:?}: {e}");
            None
        }
    }
}

/// Pure helper: parse a single nvidia-smi CSV value. Trimmed whitespace,
/// empty string → None, parse failure → None. Extracted for tests.
#[allow(dead_code)]
pub(crate) fn parse_smi_value(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    t.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_smi_handles_whitespace_and_empty() {
        assert_eq!(parse_smi_value(" 53.0 "), Some(53.0));
        assert!(parse_smi_value("").is_none());
        assert!(parse_smi_value("   ").is_none());
        assert!(parse_smi_value("N/A").is_none());
    }
}
