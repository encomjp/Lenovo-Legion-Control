//! Software fan-curve engine (runs inside the root daemon).
//!
//! A curve maps temperature to fan speed (8 points, linear in between) and is
//! stored per platform profile. While a curve is enabled for the *current*
//! profile the engine samples CPU (and awake dGPU) temperature every
//! [`TICK`], interpolates the curve, converts percent into RPM inside each
//! fan's `[min, max]` window and writes it through [`fans::set_target`].
//!
//! Safety rules (all enforced in [`Engine::tick`], tested with a mock `FanIo`):
//! * CPU or GPU at or above [`SAFETY_TEMP_C`] → every fan to its maximum,
//!   whatever the curve says.
//! * Any sensor read failure → hand all fans back to firmware
//!   (`fans::set_auto`); the engine resumes when sensors recover.
//! * Curve disabled / profile switched to one without a curve / daemon
//!   shutdown → `fans::set_auto`.
//! * Hysteresis: speed-ups follow the temperature immediately; speed-downs
//!   only after [`DOWN_CONFIRMATIONS`] consecutive readings at least
//!   [`DOWN_MARGIN_C`] below the value currently driving the curve.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::{config, dgpu, fans, profile, thermal};

/// Number of curve points.
pub const POINTS: usize = 8;
/// Curve point: (temperature °C, fan percent).
pub type CurvePoints = [(u8, u8); POINTS];

/// Lowest / highest temperature a point may sit at.
pub const TEMP_MIN_C: u8 = 20;
pub const TEMP_MAX_C: u8 = 100;
/// At or above this temperature (CPU or GPU) all fans run at maximum.
pub const SAFETY_TEMP_C: f64 = 90.0;
/// Control period.
pub const TICK: Duration = Duration::from_secs(2);
/// Consecutive lower readings required before the fans may slow down.
pub const DOWN_CONFIRMATIONS: u32 = 3;
/// A reading only counts as "lower" when this far below the held value.
pub const DOWN_MARGIN_C: f64 = 3.0;
/// Weights applied before taking the maximum of the sensors.
pub const CPU_WEIGHT: f64 = 1.0;
pub const GPU_WEIGHT: f64 = 1.0;
/// Upper bound on stored per-profile curves (config-bloat guard).
pub const MAX_CURVES: usize = 16;

/// One stored curve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FanCurve {
    pub points: CurvePoints,
    pub enabled: bool,
}

/// Snapshot returned over IPC (`GetFanCurves` / `SetFanCurve`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FanCurveStatus {
    /// Stored curves, sorted by profile name.
    pub curves: Vec<(String, FanCurve)>,
    /// Platform profile the daemon sees right now.
    pub current_profile: String,
    /// The engine currently owns the fans.
    pub active: bool,
    /// Temperature driving the curve (°C), last tick.
    pub temp_c: Option<f32>,
    /// Fan percent applied on the last tick.
    pub pct: Option<u8>,
    /// The ≥90 °C safety override is engaged.
    pub safety: bool,
    /// Last sensor / write fault (fans were handed back to firmware).
    pub fault: Option<String>,
}

/// Factory shape for a profile (mirrors the design mock-up presets).
pub fn default_points(profile: &str) -> CurvePoints {
    const TEMPS: [u8; POINTS] = [30, 40, 50, 60, 70, 80, 90, 100];
    let pct: [u8; POINTS] = match profile {
        "quiet" | "low-power" => [0, 0, 10, 22, 35, 52, 74, 100],
        "performance" | "balanced-performance" | "max-power" => [20, 30, 42, 55, 68, 82, 94, 100],
        "custom" => [10, 18, 28, 45, 60, 72, 88, 100],
        _ => [0, 15, 25, 38, 52, 68, 86, 100],
    };
    let mut out = [(0u8, 0u8); POINTS];
    for i in 0..POINTS {
        out[i] = (TEMPS[i], pct[i]);
    }
    out
}

/// Parse `"30:0,40:15,...,100:100"` (exactly 8 `temp:pct` pairs) and validate.
pub fn parse_points(spec: &str) -> Result<CurvePoints, String> {
    let parts: Vec<&str> = spec.split(',').map(str::trim).collect();
    if parts.len() != POINTS {
        return Err(format!(
            "expected {POINTS} comma-separated temp:pct pairs, got {}",
            parts.len()
        ));
    }
    let mut out = [(0u8, 0u8); POINTS];
    for (i, part) in parts.iter().enumerate() {
        let (t, p) = part
            .split_once(':')
            .ok_or_else(|| format!("point {}: '{part}' is not temp:pct", i + 1))?;
        let t: u8 = t
            .trim()
            .parse()
            .map_err(|_| format!("point {}: bad temperature '{t}'", i + 1))?;
        let p: u8 = p
            .trim()
            .parse()
            .map_err(|_| format!("point {}: bad percent '{p}'", i + 1))?;
        out[i] = (t, p);
    }
    validate(&out)?;
    Ok(out)
}

/// Profile keys are short kernel platform-profile names.
pub fn validate_profile_name(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name.len() <= 32
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(format!("invalid profile name {name:?}"))
    }
}

/// Curve rules: temperatures strictly increasing inside 20..=100, percent
/// 0..=100 and non-decreasing, last point 100 % at 90 °C or above.
pub fn validate(points: &CurvePoints) -> Result<(), String> {
    for (i, &(t, p)) in points.iter().enumerate() {
        if !(TEMP_MIN_C..=TEMP_MAX_C).contains(&t) {
            return Err(format!(
                "point {}: temperature {t} °C outside {TEMP_MIN_C}..={TEMP_MAX_C}",
                i + 1
            ));
        }
        if p > 100 {
            return Err(format!("point {}: {p} % is above 100", i + 1));
        }
        if i > 0 {
            let (pt, pp) = points[i - 1];
            if t <= pt {
                return Err(format!(
                    "point {}: temperature {t} °C must be above the previous {pt} °C",
                    i + 1
                ));
            }
            if p < pp {
                return Err(format!(
                    "point {}: {p} % is below the previous {pp} % (curve must not fall)",
                    i + 1
                ));
            }
        }
    }
    let (t, p) = points[POINTS - 1];
    if p != 100 || (t as f64) < SAFETY_TEMP_C {
        return Err(format!(
            "last point must be 100 % at {SAFETY_TEMP_C:.0} °C or above (got {p} % at {t} °C)"
        ));
    }
    Ok(())
}

/// Move point `idx` towards (`temp`, `pct`) while keeping the curve valid:
/// temperatures stay strictly increasing, percentages non-decreasing, and the
/// last point stays at 100 % and at or above 90 °C. Used by the GUI editor
/// (mouse and keyboard) so an edited curve always passes [`validate`].
pub fn constrain_point(points: &CurvePoints, idx: usize, temp: i32, pct: i32) -> (u8, u8) {
    let idx = idx.min(POINTS - 1);
    let last = idx == POINTS - 1;
    let t_lo = if idx == 0 {
        TEMP_MIN_C as i32
    } else {
        points[idx - 1].0 as i32 + 1
    };
    let t_hi = if last {
        TEMP_MAX_C as i32
    } else {
        points[idx + 1].0 as i32 - 1
    };
    let t_lo = if last { t_lo.max(SAFETY_TEMP_C as i32) } else { t_lo };
    let p_lo = if idx == 0 { 0 } else { points[idx - 1].1 as i32 };
    let p_hi = if last { 100 } else { points[idx + 1].1 as i32 };
    let p_lo = if last { 100 } else { p_lo };
    let t = temp.clamp(t_lo.min(t_hi), t_hi.max(t_lo));
    let p = pct.clamp(p_lo.min(p_hi), p_hi.max(p_lo));
    (t as u8, p as u8)
}

/// Piecewise-linear fan percent (0..=100) for `temp_c`. Flat before the first
/// and after the last point.
pub fn interpolate(points: &CurvePoints, temp_c: f64) -> f64 {
    let first = points[0];
    let last = points[POINTS - 1];
    if temp_c <= first.0 as f64 {
        return first.1 as f64;
    }
    if temp_c >= last.0 as f64 {
        return last.1 as f64;
    }
    for w in points.windows(2) {
        let (t0, p0) = (w[0].0 as f64, w[0].1 as f64);
        let (t1, p1) = (w[1].0 as f64, w[1].1 as f64);
        if temp_c <= t1 {
            let span = t1 - t0;
            if span <= 0.0 {
                return p1;
            }
            return p0 + (p1 - p0) * (temp_c - t0) / span;
        }
    }
    last.1 as f64
}

/// Percent → RPM: 0 % is the fan's minimum settable speed (never `0`, which
/// the driver reads as "auto"), 100 % its maximum.
pub fn pct_to_rpm(min: u32, max: u32, pct: f64) -> u32 {
    let (lo, hi) = if min <= max { (min, max) } else { (max, min) };
    let p = pct.clamp(0.0, 100.0) / 100.0;
    let rpm = lo as f64 + (hi - lo) as f64 * p;
    fans::clamp_target_with(lo, hi, rpm.round() as u32)
}

/// Temperature hysteresis (see module docs).
#[derive(Debug, Clone, Default)]
pub struct Hysteresis {
    held: Option<f64>,
    lower_streak: u32,
    /// Highest of the lower readings in the current streak — the value the
    /// curve falls back to, so a slowdown never overshoots a recent reading.
    streak_max: f64,
}

impl Hysteresis {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Feed a raw reading, get the temperature the curve should use.
    pub fn update(&mut self, temp: f64) -> f64 {
        let Some(held) = self.held else {
            self.held = Some(temp);
            return temp;
        };
        if temp >= held {
            self.held = Some(temp);
            self.lower_streak = 0;
        } else if temp <= held - DOWN_MARGIN_C {
            if self.lower_streak == 0 {
                self.streak_max = temp;
            } else {
                self.streak_max = self.streak_max.max(temp);
            }
            self.lower_streak += 1;
            if self.lower_streak >= DOWN_CONFIRMATIONS {
                self.held = Some(self.streak_max);
                self.lower_streak = 0;
            }
        } else {
            // Only slightly lower: not a confirmed cool-down, restart count.
            self.lower_streak = 0;
        }
        self.held.unwrap_or(temp)
    }
}

/// Result of one control decision.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Decision {
    /// Raw combined sensor value (°C).
    pub temp_c: f64,
    /// Value after hysteresis that fed the curve.
    pub held_c: f64,
    /// Fan percent to apply.
    pub pct: f64,
    /// Safety override engaged.
    pub safety: bool,
}

/// Pure control step.
pub fn decide(
    hyst: &mut Hysteresis,
    points: &CurvePoints,
    cpu_c: f64,
    gpu_c: Option<f64>,
) -> Decision {
    let combined = match gpu_c {
        Some(g) => (cpu_c * CPU_WEIGHT).max(g * GPU_WEIGHT),
        None => cpu_c * CPU_WEIGHT,
    };
    let held = hyst.update(combined);
    // Safety looks at the raw sensors (not the held value) so it can never be
    // delayed, and at each sensor on its own so weights cannot mask a hot one.
    let safety = cpu_c >= SAFETY_TEMP_C || gpu_c.is_some_and(|g| g >= SAFETY_TEMP_C);
    let pct = if safety {
        100.0
    } else {
        interpolate(points, held)
    };
    Decision {
        temp_c: combined,
        held_c: held,
        pct,
        safety,
    }
}

/// One fan's controllable window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FanWindow {
    pub id: u8,
    pub min: u32,
    pub max: u32,
}

/// Hardware access used by the engine (mockable).
pub trait FanIo {
    /// (CPU °C, optional awake-dGPU °C); `Err` = sensor failure.
    fn temps(&self) -> Result<(f64, Option<f64>), String>;
    fn windows(&self) -> Vec<FanWindow>;
    fn set_rpm(&self, fan: u8, rpm: u32) -> std::io::Result<()>;
    fn set_auto(&self) -> std::io::Result<()>;
}

/// Real hardware.
pub struct SysIo;

impl FanIo for SysIo {
    fn temps(&self) -> Result<(f64, Option<f64>), String> {
        let (a, b) = thermal::read_cpu_temps();
        let cpu_mc = [a, b].into_iter().flatten().max();
        let cpu = match cpu_mc {
            Some(mc) if mc > 0 => mc as f64 / 1000.0,
            _ => return Err("CPU temperature unreadable".into()),
        };
        let gpu = match dgpu::thermal_state() {
            dgpu::ThermalState::Temp(t) => Some(t),
            dgpu::ThermalState::NotApplicable | dgpu::ThermalState::Asleep => None,
            dgpu::ThermalState::Failed => {
                return Err("dGPU temperature unreadable while awake".into())
            }
        };
        Ok((cpu, gpu))
    }

    fn windows(&self) -> Vec<FanWindow> {
        fans::channels()
            .into_iter()
            .map(|c| FanWindow {
                id: c.id,
                min: fans::read_min(c.id).unwrap_or(c.min_rpm),
                max: fans::read_max(c.id).unwrap_or(c.max_rpm),
            })
            .collect()
    }

    fn set_rpm(&self, fan: u8, rpm: u32) -> std::io::Result<()> {
        // set_target re-clamps with the live window.
        fans::set_target(fan, fans::clamp_target(fan, rpm))
    }

    fn set_auto(&self) -> std::io::Result<()> {
        fans::set_auto()
    }
}

/// Engine state machine.
#[derive(Debug, Default)]
pub struct Engine {
    hyst: Hysteresis,
    /// We have written targets and firmware has not been handed control back.
    owned: bool,
    last_rpm: Vec<(u8, u32)>,
    status: LiveStatus,
}

/// Live part of [`FanCurveStatus`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LiveStatus {
    pub active: bool,
    pub temp_c: Option<f32>,
    pub pct: Option<u8>,
    pub safety: bool,
    pub fault: Option<String>,
}

impl Engine {
    pub fn status(&self) -> &LiveStatus {
        &self.status
    }

    pub fn owns_fans(&self) -> bool {
        self.owned
    }

    fn release(&mut self, io: &dyn FanIo, why: &str) {
        if self.owned {
            log::info!("fan-curve: handing fans back to firmware ({why})");
            if let Err(e) = io.set_auto() {
                log::warn!("fan-curve: set_auto failed: {e}");
            }
        }
        self.owned = false;
        self.last_rpm.clear();
        self.hyst.reset();
    }

    /// One control step. `curve` is the enabled curve for the current
    /// profile, `None` when there is none.
    pub fn tick(&mut self, io: &dyn FanIo, curve: Option<&CurvePoints>) {
        let Some(points) = curve else {
            self.release(io, "no active curve for this profile");
            self.status = LiveStatus::default();
            return;
        };
        let (cpu, gpu) = match io.temps() {
            Ok(t) => t,
            Err(e) => {
                if self.status.fault.as_deref() != Some(e.as_str()) {
                    log::warn!("fan-curve: sensor failure ({e}) — firmware control");
                }
                self.release(io, "sensor failure");
                self.status = LiveStatus {
                    fault: Some(e),
                    ..LiveStatus::default()
                };
                return;
            }
        };
        let d = decide(&mut self.hyst, points, cpu, gpu);
        if d.safety && !self.status.safety {
            log::warn!(
                "fan-curve: safety override — {:.1} °C ≥ {SAFETY_TEMP_C:.0} °C, all fans max",
                d.temp_c
            );
        }
        let windows = io.windows();
        if windows.is_empty() {
            self.release(io, "no fans");
            self.status = LiveStatus {
                fault: Some("no controllable fans".into()),
                ..LiveStatus::default()
            };
            return;
        }
        for w in &windows {
            let rpm = pct_to_rpm(w.min, w.max, d.pct);
            let unchanged = self.last_rpm.iter().any(|&(id, r)| id == w.id && r == rpm);
            if unchanged {
                continue;
            }
            // From here on we may have touched hardware.
            self.owned = true;
            if let Err(e) = io.set_rpm(w.id, rpm) {
                let msg = format!("fan {} write failed: {e}", w.id);
                log::warn!("fan-curve: {msg} — firmware control");
                self.release(io, "write failure");
                self.status = LiveStatus {
                    fault: Some(msg),
                    ..LiveStatus::default()
                };
                return;
            }
            self.last_rpm.retain(|&(id, _)| id != w.id);
            self.last_rpm.push((w.id, rpm));
        }
        self.owned = true;
        self.status = LiveStatus {
            active: true,
            temp_c: Some(d.temp_c as f32),
            pct: Some(d.pct.round() as u8),
            safety: d.safety,
            fault: None,
        };
    }
}

// ─── daemon glue ────────────────────────────────────────────────────────────

static LIVE: OnceLock<Mutex<LiveStatus>> = OnceLock::new();
static POKE: AtomicBool = AtomicBool::new(false);

fn live() -> &'static Mutex<LiveStatus> {
    LIVE.get_or_init(|| Mutex::new(LiveStatus::default()))
}

/// Wake the engine now (curve changed over IPC).
pub fn poke() {
    POKE.store(true, Ordering::Relaxed);
}

/// True while the engine owns the fans (manual targets are refused then).
pub fn is_active() -> bool {
    live().lock().map(|g| g.active).unwrap_or_else(|p| p.into_inner().active)
}

/// The enabled curve for `profile`, from the root config.
pub fn active_curve_for(profile: &str) -> Option<CurvePoints> {
    let cfg = config::get();
    let c = cfg.fan_curves.get(profile)?;
    (c.enabled && validate(&c.points).is_ok()).then_some(c.points)
}

/// Snapshot for IPC.
pub fn status() -> FanCurveStatus {
    let live = live()
        .lock()
        .map(|g| g.clone())
        .unwrap_or_else(|p| p.into_inner().clone());
    FanCurveStatus {
        curves: config::get().fan_curves.into_iter().collect(),
        current_profile: profile::current(),
        active: live.active,
        temp_c: live.temp_c,
        pct: live.pct,
        safety: live.safety,
        fault: live.fault,
    }
}

/// Validate and persist a curve (daemon side of `SetFanCurve`).
pub fn store(profile_name: &str, points: CurvePoints, enabled: bool) -> Result<(), String> {
    validate_profile_name(profile_name)?;
    validate(&points)?;
    let known = profile::choices();
    if !known.is_empty() && !known.iter().any(|c| c == profile_name) {
        return Err(format!(
            "profile '{profile_name}' not in choices: {}",
            known.join(", ")
        ));
    }
    if !config::get().fan_curves.contains_key(profile_name)
        && config::get().fan_curves.len() >= MAX_CURVES
    {
        return Err(format!("at most {MAX_CURVES} curves can be stored"));
    }
    let key = profile_name.to_string();
    config::update(move |c| {
        c.fan_curves.insert(key, FanCurve { points, enabled });
    });
    poke();
    Ok(())
}

/// Engine thread body. Returns after handing the fans back on shutdown.
pub fn run(shutdown: &AtomicBool) {
    let io = SysIo;
    let mut engine = Engine::default();
    let slice = Duration::from_millis(100);
    while !shutdown.load(Ordering::Relaxed) {
        let prof = profile::current();
        let curve = active_curve_for(&prof);
        engine.tick(&io, curve.as_ref());
        if let Ok(mut g) = live().lock() {
            *g = engine.status().clone();
        }
        // Interruptible sleep; a poke (config change) ends it early.
        let mut waited = Duration::ZERO;
        while waited < TICK && !shutdown.load(Ordering::Relaxed) {
            if POKE.swap(false, Ordering::Relaxed) {
                break;
            }
            std::thread::sleep(slice);
            waited += slice;
        }
    }
    if engine.owns_fans() {
        log::info!("fan-curve: daemon stopping — fans back to firmware");
        if let Err(e) = io.set_auto() {
            log::warn!("fan-curve: shutdown set_auto failed: {e}");
        }
    }
    if let Ok(mut g) = live().lock() {
        *g = LiveStatus::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn pts(pct: [u8; POINTS]) -> CurvePoints {
        let t = [30u8, 40, 50, 60, 70, 80, 90, 100];
        let mut o = [(0, 0); POINTS];
        for i in 0..POINTS {
            o[i] = (t[i], pct[i]);
        }
        o
    }

    fn balanced() -> CurvePoints {
        pts([0, 15, 25, 38, 52, 68, 86, 100])
    }

    #[test]
    fn defaults_are_valid_for_every_profile() {
        for p in [
            "quiet",
            "low-power",
            "balanced",
            "balanced-performance",
            "performance",
            "max-power",
            "custom",
            "weird",
        ] {
            validate(&default_points(p)).unwrap_or_else(|e| panic!("{p}: {e}"));
        }
    }

    #[test]
    fn interpolation_hits_points_midpoints_and_ends() {
        let c = balanced();
        assert_eq!(interpolate(&c, 20.0), 0.0);
        assert_eq!(interpolate(&c, 30.0), 0.0);
        assert_eq!(interpolate(&c, 35.0), 7.5);
        assert_eq!(interpolate(&c, 40.0), 15.0);
        assert_eq!(interpolate(&c, 65.0), 45.0);
        assert_eq!(interpolate(&c, 90.0), 86.0);
        assert_eq!(interpolate(&c, 95.0), 93.0);
        assert_eq!(interpolate(&c, 100.0), 100.0);
        assert_eq!(interpolate(&c, 120.0), 100.0);
    }

    #[test]
    fn pct_to_rpm_stays_in_fan_window() {
        assert_eq!(pct_to_rpm(1700, 5200, 0.0), 1700, "0 % is min, never auto(0)");
        assert_eq!(pct_to_rpm(1700, 5200, 100.0), 5200);
        assert_eq!(pct_to_rpm(1700, 5200, 50.0), 3450);
        assert_eq!(pct_to_rpm(1700, 5200, 250.0), 5200);
        assert_eq!(pct_to_rpm(1700, 5200, -5.0), 1700);
        assert_eq!(pct_to_rpm(5200, 1700, 100.0), 5200, "inverted window normalised");
        for pct in 0..=100 {
            let r = pct_to_rpm(1500, 6500, pct as f64);
            assert!((1500..=6500).contains(&r));
        }
    }

    #[test]
    fn hysteresis_rises_immediately() {
        let mut h = Hysteresis::default();
        assert_eq!(h.update(50.0), 50.0);
        assert_eq!(h.update(51.0), 51.0);
        assert_eq!(h.update(70.0), 70.0);
    }

    #[test]
    fn hysteresis_falls_only_after_three_confirmed_readings() {
        let mut h = Hysteresis::default();
        h.update(70.0);
        assert_eq!(h.update(66.0), 70.0);
        assert_eq!(h.update(65.0), 70.0);
        // Third consecutive ≥3 °C-lower reading → fall to the highest of them.
        assert_eq!(h.update(64.0), 66.0);
    }

    #[test]
    fn hysteresis_ignores_small_dips_and_resets_streak() {
        let mut h = Hysteresis::default();
        h.update(70.0);
        assert_eq!(h.update(66.0), 70.0);
        assert_eq!(h.update(65.0), 70.0);
        assert_eq!(h.update(69.0), 70.0, "within 3 °C: streak restarts");
        assert_eq!(h.update(65.0), 70.0);
        assert_eq!(h.update(65.0), 70.0);
        assert_eq!(h.update(65.0), 65.0);
        // A rise in the middle of a streak also restarts it.
        let mut h = Hysteresis::default();
        h.update(70.0);
        h.update(60.0);
        h.update(60.0);
        assert_eq!(h.update(71.0), 71.0);
        assert_eq!(h.update(60.0), 71.0);
    }

    #[test]
    fn safety_overrides_curve_for_cpu_and_gpu() {
        let low = pts([0, 0, 0, 0, 0, 0, 0, 100]);
        let mut h = Hysteresis::default();
        let d = decide(&mut h, &low, 89.9, Some(50.0));
        assert!(!d.safety);
        let d = decide(&mut h, &low, 90.0, None);
        assert!(d.safety && d.pct == 100.0);
        let mut h = Hysteresis::default();
        let d = decide(&mut h, &low, 40.0, Some(91.0));
        assert!(d.safety && d.pct == 100.0);
        assert_eq!(d.temp_c, 91.0, "weighted max picks the hotter sensor");
    }

    #[test]
    fn constrain_point_keeps_curve_valid() {
        let base = balanced();
        // Interior: bounded by neighbours.
        assert_eq!(constrain_point(&base, 3, 0, 0), (51, 25));
        assert_eq!(constrain_point(&base, 3, 200, 200), (69, 52));
        // First point can go down to 20 °C and 0 %.
        assert_eq!(constrain_point(&base, 0, -5, -5), (20, 0));
        // Last stays at 100 % and >= 90 °C.
        assert_eq!(constrain_point(&base, 7, 50, 10), (91, 100));
        assert_eq!(constrain_point(&base, 7, 500, 10), (100, 100));
        // Any single-point drag from anywhere leaves a valid curve.
        for idx in 0..POINTS {
            for t in [-10, 0, 35, 63, 99, 150] {
                for p in [-10, 0, 33, 99, 150] {
                    let mut c = base;
                    c[idx] = constrain_point(&c, idx, t, p);
                    validate(&c).unwrap_or_else(|e| panic!("idx {idx} t {t} p {p}: {e}"));
                }
            }
        }
    }

    #[test]
    fn parse_points_round_trips_and_rejects_garbage() {
        let p = parse_points("30:0,40:15,50:25,60:38,70:52,80:68,90:86,100:100").unwrap();
        assert_eq!(p, balanced());
        assert!(parse_points("30:0,40:15").is_err());
        assert!(parse_points("30:0,40:15,50:25,60:38,70:52,80:68,90:86,100").is_err());
        assert!(parse_points("30:0,40:15,50:25,60:38,70:52,80:68,90:86,100:99").is_err());
        assert!(parse_points("30:0,40:15,50:25,60:38,70:52,80:68,90:86,300:100").is_err());
    }

    #[test]
    fn validation_accepts_defaults_and_rejects_bad_curves() {
        assert!(validate(&balanced()).is_ok());
        let mut p = balanced();
        p[3].0 = p[2].0; // not strictly increasing
        assert!(validate(&p).is_err());
        let mut p = balanced();
        p[0].0 = 19;
        assert!(validate(&p).is_err());
        let mut p = balanced();
        p[4].1 = 10; // falls
        assert!(validate(&p).is_err());
        let mut p = balanced();
        p[2].1 = 101;
        assert!(validate(&p).is_err());
        let mut p = balanced();
        p[7].1 = 99; // last must be 100 %
        assert!(validate(&p).is_err());
        let mut p = balanced();
        p[6].0 = 85;
        p[7].0 = 89; // last below 90 °C
        assert!(validate(&p).is_err());
        let mut p = balanced();
        p[7].0 = 90;
        p[6].0 = 80;
        p[5].0 = 70;
        p[4].0 = 60;
        p[3].0 = 50;
        p[2].0 = 40;
        p[1].0 = 30;
        p[0].0 = 20;
        assert!(validate(&p).is_ok());
        assert!(validate_profile_name("balanced-performance").is_ok());
        assert!(validate_profile_name("").is_err());
        assert!(validate_profile_name("Bad\nName").is_err());
        assert!(validate_profile_name(&"x".repeat(33)).is_err());
    }

    // ── engine with a mock ──

    struct Mock {
        temps: RefCell<Result<(f64, Option<f64>), String>>,
        writes: RefCell<Vec<(u8, u32)>>,
        autos: RefCell<u32>,
        fail_write: bool,
    }

    impl Mock {
        fn new(cpu: f64) -> Self {
            Mock {
                temps: RefCell::new(Ok((cpu, None))),
                writes: RefCell::new(vec![]),
                autos: RefCell::new(0),
                fail_write: false,
            }
        }
    }

    impl FanIo for Mock {
        fn temps(&self) -> Result<(f64, Option<f64>), String> {
            self.temps.borrow().clone()
        }
        fn windows(&self) -> Vec<FanWindow> {
            vec![
                FanWindow { id: 1, min: 1700, max: 5200 },
                FanWindow { id: 2, min: 1700, max: 5400 },
                FanWindow { id: 4, min: 1500, max: 6500 },
            ]
        }
        fn set_rpm(&self, fan: u8, rpm: u32) -> std::io::Result<()> {
            if self.fail_write {
                return Err(std::io::Error::other("boom"));
            }
            self.writes.borrow_mut().push((fan, rpm));
            Ok(())
        }
        fn set_auto(&self) -> std::io::Result<()> {
            *self.autos.borrow_mut() += 1;
            Ok(())
        }
    }

    #[test]
    fn engine_writes_per_fan_rpm_and_skips_unchanged() {
        let io = Mock::new(65.0);
        let c = balanced();
        let mut e = Engine::default();
        e.tick(&io, Some(&c));
        // 65 °C → 45 %
        assert_eq!(
            *io.writes.borrow(),
            vec![(1, 3275), (2, 3365), (4, 3750)]
        );
        assert!(e.status().active && e.status().pct == Some(45));
        e.tick(&io, Some(&c));
        assert_eq!(io.writes.borrow().len(), 3, "no rewrite when unchanged");
    }

    #[test]
    fn engine_safety_forces_max_on_every_fan() {
        let io = Mock::new(95.0);
        let low = pts([0, 0, 0, 0, 0, 0, 0, 100]);
        let mut e = Engine::default();
        e.tick(&io, Some(&low));
        assert_eq!(*io.writes.borrow(), vec![(1, 5200), (2, 5400), (4, 6500)]);
        assert!(e.status().safety);
    }

    #[test]
    fn engine_sensor_failure_hands_back_to_firmware_and_recovers() {
        let io = Mock::new(60.0);
        let c = balanced();
        let mut e = Engine::default();
        e.tick(&io, Some(&c));
        *io.temps.borrow_mut() = Err("CPU temperature unreadable".into());
        e.tick(&io, Some(&c));
        assert_eq!(*io.autos.borrow(), 1);
        assert!(!e.owns_fans() && !e.status().active);
        assert!(e.status().fault.is_some());
        e.tick(&io, Some(&c));
        assert_eq!(*io.autos.borrow(), 1, "no repeated set_auto while faulted");
        *io.temps.borrow_mut() = Ok((60.0, None));
        io.writes.borrow_mut().clear();
        e.tick(&io, Some(&c));
        assert_eq!(io.writes.borrow().len(), 3, "rewrites after recovery");
        assert!(e.status().fault.is_none());
    }

    #[test]
    fn engine_releases_when_curve_goes_away() {
        let io = Mock::new(60.0);
        let c = balanced();
        let mut e = Engine::default();
        e.tick(&io, Some(&c));
        e.tick(&io, None);
        assert_eq!(*io.autos.borrow(), 1);
        e.tick(&io, None);
        assert_eq!(*io.autos.borrow(), 1, "idle engine never touches fans");
    }

    #[test]
    fn engine_write_failure_falls_back_to_firmware() {
        let mut io = Mock::new(60.0);
        io.fail_write = true;
        let mut e = Engine::default();
        e.tick(&io, Some(&balanced()));
        assert_eq!(*io.autos.borrow(), 1);
        assert!(e.status().fault.is_some());
    }

    #[test]
    fn engine_slows_down_only_after_confirmation() {
        let io = Mock::new(80.0);
        let c = balanced();
        let mut e = Engine::default();
        e.tick(&io, Some(&c));
        let first = io.writes.borrow().len();
        for _ in 0..2 {
            *io.temps.borrow_mut() = Ok((70.0, None));
            e.tick(&io, Some(&c));
        }
        assert_eq!(io.writes.borrow().len(), first, "held for two readings");
        *io.temps.borrow_mut() = Ok((70.0, None));
        e.tick(&io, Some(&c));
        assert!(io.writes.borrow().len() > first, "third reading slows down");
    }

    #[test]
    fn status_and_curve_serialize_over_ipc() {
        use bincode::Options;
        let st = FanCurveStatus {
            curves: vec![(
                "balanced".into(),
                FanCurve {
                    points: balanced(),
                    enabled: true,
                },
            )],
            current_profile: "balanced".into(),
            active: true,
            temp_c: Some(64.5),
            pct: Some(45),
            safety: false,
            fault: None,
        };
        let bytes = crate::comms::bincode_opts().serialize(&st).unwrap();
        let back: FanCurveStatus = crate::comms::bincode_opts().deserialize(&bytes).unwrap();
        assert_eq!(back, st);
    }
}
