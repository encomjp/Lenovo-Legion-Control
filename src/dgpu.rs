//! NVIDIA dGPU monitoring via nvidia-smi subprocess.
//!
//! The NVIDIA GPU has no hwmon interface on Linux — we use nvidia-smi.
//! A 3-second timeout prevents hanging the daemon if the driver is
//! unresponsive. On timeout the child is SIGKILLed so the reaper thread
//! always finishes (no leaked threads or zombies).

use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const SMI_TIMEOUT: Duration = Duration::from_secs(3);
const SMI_BIN: &str = "/usr/bin/nvidia-smi";

/// Run nvidia-smi with a timeout. If the subprocess hasn't exited within
/// `SMI_TIMEOUT`, it is killed and `None` is returned.
fn smi_run(args: &[&str]) -> Option<String> {
    let started = Instant::now();
    log::debug!("nvidia-smi: spawning {SMI_BIN} {}", args.join(" "));
    let child = match Command::new(SMI_BIN)
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
        let _ = tx.send(child.wait_with_output());
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
                libc::kill(pid as libc::pid_t, libc::SIGKILL);
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

pub fn read_temp() -> Option<f64> {
    let raw = smi_query("temperature.gpu")?;
    match raw.parse::<f64>() {
        Ok(v) => {
            log::trace!("gpu temperature: {v} °C");
            Some(v)
        }
        Err(e) => {
            log::warn!("gpu temperature parse failed for {raw:?}: {e}");
            None
        }
    }
}

pub fn read_power() -> Option<f64> {
    let raw = smi_query("power.draw")?;
    match raw.parse::<f64>() {
        Ok(v) => {
            log::trace!("gpu power draw: {v} W");
            Some(v)
        }
        Err(e) => {
            log::warn!("gpu power draw parse failed for {raw:?}: {e}");
            None
        }
    }
}

pub fn read_clock() -> Option<f64> {
    let raw = smi_query("clocks.gr")?;
    match raw.parse::<f64>() {
        Ok(v) => {
            log::trace!("gpu core clock: {v} MHz");
            Some(v)
        }
        Err(e) => {
            log::warn!("gpu core clock parse failed for {raw:?}: {e}");
            None
        }
    }
}

pub fn read_util() -> Option<f64> {
    let raw = smi_query("utilization.gpu")?;
    match raw.parse::<f64>() {
        Ok(v) => {
            log::trace!("gpu utilization: {v} %");
            Some(v)
        }
        Err(e) => {
            log::warn!("gpu utilization parse failed for {raw:?}: {e}");
            None
        }
    }
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
