//! Custom-mode power limits (daemon side).
//!
//! The firmware uses the user's power limits (lenovo-wmi-other firmware
//! attributes such as `ppt_pl1_spl`) only in Custom mode and overrides them
//! in every other mode. So:
//! * a write outside Custom is refused with [`SWITCH_FIRST`] instead of
//!   "succeeding" with no effect;
//! * every write is read back (`current_value`), retried once after
//!   [`SETTLE`], then reported as failed;
//! * accepted values are saved in the service's own settings
//!   (`custom_power_limits`) and written again whenever the mode becomes
//!   Custom (GUI, CLI, automatic switching, service start) and after resume,
//!   [`SETTLE`] after the mode change because the firmware may drop writes
//!   made right after one.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use crate::{config, profile};

/// Error text for a power-limit write outside Custom mode (GUI toast, CLI).
pub const SWITCH_FIRST: &str =
    "Switch to Custom mode first. Power limits only work in Custom mode.";
/// Wait after a mode change before writing, and before the single retry.
pub const SETTLE: Duration = Duration::from_millis(500);

/// Serialises IPC writes and re-applies so they never interleave.
static WRITE_LOCK: Mutex<()> = Mutex::new(());
/// Re-apply now if the mode is Custom (resume).
static KICK: AtomicBool = AtomicBool::new(false);

/// Firmware attribute access (mockable).
pub trait LimitIo {
    fn write(&self, attr: &str, value: u32) -> Result<(), String>;
    fn read(&self, attr: &str) -> Option<u32>;
    fn sleep(&self, d: Duration);
}

struct SysIo;

impl LimitIo for SysIo {
    fn write(&self, attr: &str, value: u32) -> Result<(), String> {
        profile::set_ppt(attr, value)
    }
    fn read(&self, attr: &str) -> Option<u32> {
        profile::read_ppt(attr)
    }
    fn sleep(&self, d: Duration) {
        std::thread::sleep(d)
    }
}

/// Write `value` and confirm it by reading `current_value` back; one retry
/// after [`SETTLE`]. An unreadable value after a successful write is taken
/// as accepted (nothing to compare against).
pub fn write_verified(io: &dyn LimitIo, attr: &str, value: u32) -> Result<(), String> {
    let mut last_read = None;
    for attempt in 0..2 {
        if attempt > 0 {
            io.sleep(SETTLE);
        }
        io.write(attr, value)?;
        match io.read(attr) {
            Some(v) if v == value => {
                if attempt > 0 {
                    log::info!("power-limits: {attr}={value} kept on the second try");
                }
                return Ok(());
            }
            None => {
                log::debug!("power-limits: {attr} unreadable after write: assuming accepted");
                return Ok(());
            }
            Some(v) => {
                log::warn!(
                    "power-limits: {attr} wrote {value}, reads {v} (try {}/2)",
                    attempt + 1
                );
                last_read = Some(v);
            }
        }
    }
    log::warn!(
        "power-limits: {attr}={value} not kept, reads {}",
        last_read.unwrap_or_default()
    );
    Err(format!(
        "The firmware did not keep {} at {value} {}. Try again.",
        profile::limit_label(attr),
        profile::limit_unit(attr).symbol()
    ))
}

/// Pure: should saved limits be written now? Only in Custom, and only when
/// the mode just became Custom (`previous` was something else or unknown,
/// e.g. at service start) or a re-apply was asked for (resume).
pub fn should_reapply(previous: Option<&str>, now: &str, kicked: bool) -> bool {
    now == "custom" && (kicked || previous != Some("custom"))
}

/// Pure: the saved limits that differ from what the firmware reports now
/// (unreadable counts as different). Unknown attributes are skipped.
pub fn limits_to_write(
    saved: &BTreeMap<String, u32>,
    current: impl Fn(&str) -> Option<u32>,
) -> Vec<(String, u32)> {
    saved
        .iter()
        .filter(|(id, _)| profile::is_known_fw_attr(id))
        .filter(|(id, v)| current(id) != Some(**v))
        .map(|(id, v)| (id.clone(), *v))
        .collect()
}

fn lock() -> std::sync::MutexGuard<'static, ()> {
    WRITE_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

/// Daemon side of `SetFwAttr`: Custom-mode check, settle wait after a
/// recent mode change, verified write, then save the value.
pub fn set_custom_limit(attr: &str, value: u32) -> Result<(), String> {
    let _g = lock();
    let mode = profile::current();
    if mode != "custom" {
        log::info!("power-limits: {attr}={value} refused: mode is '{mode}', not custom");
        return Err(SWITCH_FIRST.into());
    }
    if let Some(since) = profile::since_last_set() {
        if since < SETTLE {
            std::thread::sleep(SETTLE - since);
        }
    }
    write_verified(&SysIo, attr, value)?;
    let id = attr.to_string();
    config::update(move |c| {
        c.custom_power_limits.insert(id, value);
    });
    Ok(())
}

/// Write every saved limit that the firmware lost. No-op outside Custom.
/// Returns how many were written.
pub fn reapply_saved(reason: &str) -> Result<usize, String> {
    let _g = lock();
    if profile::current() != "custom" {
        return Ok(0);
    }
    let saved = config::get().custom_power_limits;
    let todo = limits_to_write(&saved, profile::read_ppt);
    if todo.is_empty() {
        log::debug!(
            "power-limits: {reason}: {} saved limit(s) already in place",
            saved.len()
        );
        return Ok(0);
    }
    let mut errors = Vec::new();
    for (id, v) in &todo {
        if let Err(e) = write_verified(&SysIo, id, *v) {
            errors.push(e);
        }
    }
    if errors.is_empty() {
        log::info!(
            "power-limits: {reason}: re-applied {} saved limit(s)",
            todo.len()
        );
        Ok(todo.len())
    } else {
        Err(errors.join("; "))
    }
}

/// Ask the worker to re-apply now if the mode is Custom (resume).
pub fn kick() {
    KICK.store(true, Ordering::Relaxed);
}

/// Daemon worker: watches the power mode (0.5 s) and re-applies saved
/// limits [`SETTLE`] after it becomes Custom, or after [`kick`].
pub fn run(shutdown: &AtomicBool) {
    let mut previous: Option<String> = None;
    while !shutdown.load(Ordering::Relaxed) {
        let now = profile::current_quiet();
        let kicked = KICK.swap(false, Ordering::Relaxed);
        if should_reapply(previous.as_deref(), &now, kicked)
            && !config::get().custom_power_limits.is_empty()
        {
            std::thread::sleep(SETTLE);
            let reason = if kicked {
                "resume"
            } else {
                "mode is now Custom"
            };
            match reapply_saved(reason) {
                Ok(_) => {}
                Err(e) => log::warn!("power-limits: {reason}: {e}"),
            }
        }
        previous = Some(now);
        std::thread::sleep(Duration::from_millis(500));
    }
    log::info!("power-limits thread stopped");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// `accept_after`: writes before this count are dropped by the firmware.
    struct Mock {
        writes: RefCell<u32>,
        accept_after: u32,
        value: RefCell<Option<u32>>,
        default: u32,
        unreadable: bool,
    }

    impl Mock {
        fn new(accept_after: u32) -> Self {
            Mock {
                writes: RefCell::new(0),
                accept_after,
                value: RefCell::new(None),
                default: 45,
                unreadable: false,
            }
        }
    }

    impl LimitIo for Mock {
        fn write(&self, _attr: &str, value: u32) -> Result<(), String> {
            *self.writes.borrow_mut() += 1;
            if *self.writes.borrow() > self.accept_after {
                *self.value.borrow_mut() = Some(value);
            }
            Ok(())
        }
        fn read(&self, _attr: &str) -> Option<u32> {
            if self.unreadable {
                return None;
            }
            Some(self.value.borrow().unwrap_or(self.default))
        }
        fn sleep(&self, _d: Duration) {}
    }

    #[test]
    fn verified_write_accepts_first_try() {
        let io = Mock::new(0);
        assert!(write_verified(&io, "ppt_pl1_spl", 80).is_ok());
        assert_eq!(*io.writes.borrow(), 1);
    }

    #[test]
    fn verified_write_retries_once_after_a_dropped_write() {
        let io = Mock::new(1);
        assert!(write_verified(&io, "ppt_pl1_spl", 80).is_ok());
        assert_eq!(*io.writes.borrow(), 2);
    }

    #[test]
    fn verified_write_reports_failure_after_the_retry() {
        let io = Mock::new(99);
        let e = write_verified(&io, "ppt_pl1_spl", 80).unwrap_err();
        assert_eq!(
            e,
            "The firmware did not keep Everyday power at 80 W. Try again."
        );
        assert_eq!(*io.writes.borrow(), 2, "exactly one retry");
    }

    #[test]
    fn verified_write_trusts_an_unreadable_value() {
        let mut io = Mock::new(99);
        io.unreadable = true;
        assert!(write_verified(&io, "ppt_pl1_spl", 80).is_ok());
    }

    #[test]
    fn reapply_only_when_custom_starts_or_on_request() {
        assert!(
            should_reapply(None, "custom", false),
            "service start in Custom"
        );
        assert!(
            should_reapply(Some("balanced"), "custom", false),
            "switched to Custom"
        );
        assert!(
            !should_reapply(Some("custom"), "custom", false),
            "steady Custom"
        );
        assert!(
            should_reapply(Some("custom"), "custom", true),
            "resume in Custom"
        );
        assert!(
            !should_reapply(Some("custom"), "performance", true),
            "never outside Custom"
        );
        assert!(!should_reapply(None, "balanced", false));
    }

    #[test]
    fn only_lost_and_known_limits_are_rewritten() {
        let mut saved = BTreeMap::new();
        saved.insert("ppt_pl1_spl".to_string(), 80);
        saved.insert("ppt_pl2_sppt".to_string(), 100);
        saved.insert("cpu_temp".to_string(), 95);
        saved.insert("bogus".to_string(), 1);
        let current = |id: &str| match id {
            "ppt_pl1_spl" => Some(80),  // still in place
            "ppt_pl2_sppt" => Some(65), // firmware reset it
            _ => None,                  // unreadable
        };
        assert_eq!(
            limits_to_write(&saved, current),
            vec![
                ("cpu_temp".to_string(), 95),
                ("ppt_pl2_sppt".to_string(), 100)
            ]
        );
        assert!(limits_to_write(&BTreeMap::new(), current).is_empty());
    }
}
