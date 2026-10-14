//! Background services that must run while the GUI is closed:
//! automatic power-mode switching and the 1-hour temperature history.

use legion_core::comms::{mode_code, TempHistory, TempSample};
use legion_core::profile::auto::{self, AutoModeStatus, Switcher};
use legion_core::{config, fan_curve, power_limits, profile, thermal};

use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// History cadence and length: 720 × 5 s = 1 hour.
pub const HISTORY_INTERVAL_S: u16 = 5;
pub const HISTORY_LEN: usize = 720;

static HISTORY: Mutex<VecDeque<TempSample>> = Mutex::new(VecDeque::new());
/// Process name of the app rule in force (for status queries).
static ACTIVE_APP: Mutex<Option<String>> = Mutex::new(None);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Push one sample, dropping the oldest beyond `cap`.
fn push_capped(buf: &mut VecDeque<TempSample>, s: TempSample, cap: usize) {
    buf.push_back(s);
    while buf.len() > cap {
        buf.pop_front();
    }
}

pub fn history() -> TempHistory {
    TempHistory {
        interval_s: HISTORY_INTERVAL_S,
        samples: lock(&HISTORY).iter().copied().collect(),
    }
}

pub fn auto_status() -> AutoModeStatus {
    AutoModeStatus {
        config: config::get().auto_mode,
        on_ac: auto::on_ac(),
        active_app: lock(&ACTIVE_APP).clone(),
    }
}

/// Cheap readings only: hwmon for the CPU, and the dGPU readers that never
/// wake a runtime-suspended card (-1 when asleep or missing).
fn sample_now() -> TempSample {
    let (a, b) = thermal::read_cpu_temps();
    let cpu = a.or(b).map(|mc| mc as f32 / 1000.0).unwrap_or(0.0);
    let gpu = legion_core::dgpu::read_metrics_batch()
        .temp
        .or_else(legion_core::sensors::amd_dgpu_temp_c)
        .filter(|t| *t > 0.0)
        .map(|t| t as f32)
        .unwrap_or(-1.0);
    TempSample {
        cpu,
        gpu,
        mode: mode_code(&profile::current_quiet()),
    }
}

pub fn temp_history_worker(shutdown: Arc<AtomicBool>) {
    let slice = Duration::from_millis(250);
    loop {
        let s = sample_now();
        push_capped(&mut lock(&HISTORY), s, HISTORY_LEN);
        if super::sleep_interruptible(&shutdown, Duration::from_secs(HISTORY_INTERVAL_S as u64), slice) {
            break;
        }
    }
    log::info!("temp-history thread stopped");
}

/// Write `target`; on battery a refused write falls back to Balanced (the
/// firmware may reject Performance there, see `profile::auto`).
fn apply(target: &str, on_ac: bool) {
    match profile::set(target) {
        Ok(()) => {
            log::info!(
                "auto-mode: {} → {target}",
                if on_ac { "plugged in" } else { "on battery" }
            );
            fan_curve::poke();
        }
        Err(e) if !on_ac && target != "balanced" => {
            log::warn!("auto-mode: firmware refused '{target}' on battery ({e}): using balanced");
            match profile::set("balanced") {
                Ok(()) => fan_curve::poke(),
                Err(e) => log::warn!("auto-mode: balanced on battery failed too: {e}"),
            }
        }
        Err(e) => log::warn!("auto-mode: cannot set '{target}': {e}"),
    }
}

/// AC is checked every 2 s, processes every 5 s (only while app rules
/// exist). Writes happen only on transitions (see `auto::Switcher`).
pub fn auto_mode_worker(shutdown: Arc<AtomicBool>) {
    const TICK: Duration = Duration::from_secs(1);
    let mut sw = Switcher::default();
    let mut tick: u64 = 0;
    let mut running: Vec<String> = Vec::new();
    let mut last_on_ac: Option<bool> = None;
    let mut warned_no_ac = false;
    loop {
        let cfg = config::get().auto_mode;
        if !cfg.enabled {
            sw = Switcher::default();
            *lock(&ACTIVE_APP) = None;
            running.clear();
        } else if tick.is_multiple_of(2) || tick.is_multiple_of(5) {
            if tick.is_multiple_of(5) {
                running = if cfg.app_rules.is_empty() {
                    Vec::new()
                } else {
                    auto::running_processes()
                };
            }
            match auto::on_ac() {
                Some(on_ac) => {
                    warned_no_ac = false;
                    if last_on_ac.is_some_and(|l| l != on_ac) {
                        log::info!("auto-mode: charger {}", if on_ac { "plugged in" } else { "unplugged" });
                    }
                    last_on_ac = Some(on_ac);
                    let current = profile::current_quiet();
                    if let Some(target) = sw.step(&cfg, on_ac, &running, &current) {
                        apply(&target, on_ac);
                    }
                    *lock(&ACTIVE_APP) = sw.active_app().map(str::to_string);
                }
                None if !warned_no_ac => {
                    log::warn!("auto-mode: no charger state in /sys/class/power_supply: idle");
                    warned_no_ac = true;
                }
                None => {}
            }
        }
        tick = tick.wrapping_add(1);
        if super::sleep_interruptible(&shutdown, TICK, Duration::from_millis(250)) {
            break;
        }
    }
    log::info!("auto-mode thread stopped");
}

/// Keeps Custom-mode power limits applied (see `legion_core::power_limits`).
pub fn power_limits_worker(shutdown: Arc<AtomicBool>) {
    power_limits::run(&shutdown);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_keeps_last_hour() {
        let mut buf = VecDeque::new();
        for i in 0..(HISTORY_LEN + 50) {
            push_capped(
                &mut buf,
                TempSample {
                    cpu: i as f32,
                    gpu: -1.0,
                    mode: 2,
                },
                HISTORY_LEN,
            );
        }
        assert_eq!(buf.len(), HISTORY_LEN);
        assert_eq!(buf.front().unwrap().cpu, 50.0);
        assert_eq!(buf.back().unwrap().cpu, (HISTORY_LEN + 49) as f32);
        assert_eq!(HISTORY_LEN as u32 * HISTORY_INTERVAL_S as u32, 3600);
    }
}
