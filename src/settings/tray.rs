//! StatusNotifierItem (system tray) for Legion Settings.
//!
//! * A background thread polls the daemon every ~2 s and pushes a
//!   [`TrayState`] into the tray with `Handle::update` (never the GTK thread).
//! * The menu is built from a pure model ([`build_menu`]) so it can be tested
//!   without D-Bus; [`to_ksni`] converts that model into ksni items.
//! * Menu actions are queued to a worker thread that talks to the daemon;
//!   failures become a desktop notification and a log line.

use crate::widgets::friendly_profile;
use ksni::menu::*;
use ksni::Tray;
use legion_core::comms::{send_command, DaemonCommand, DaemonResponse};
use legion_core::device::LightingKind;
use std::sync::mpsc;
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(2);
/// Slow-changing data (GPU mux mode) is re-read every N polls.
const SLOW_EVERY: u32 = 5;
const ICON_DEFAULT: &str = "com.encomjp.legion-settings-tray";
const CHARGE_LIMITS: [u32; 3] = [60, 80, 100];

#[derive(Debug)]
pub enum TrayCmd {
    Show,
    Quit,
}

// ───────────────────────── pure model ─────────────────────────

/// Keyboard backlight capability and current level.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Kbd {
    #[default]
    None,
    /// Spectrum RGB, brightness 0..=9 via `SetRgbBrightness`.
    Spectrum { level: u8 },
    /// White backlight, brightness 0..=max via `SetKbdBrightness`.
    White { level: u8, max: u8 },
}

/// Everything the tooltip and menu need. Values are pre-rounded so that
/// jitter in the readings does not cause a D-Bus update every poll.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TrayState {
    /// The service answered the last poll.
    pub connected: bool,
    pub profile: Option<String>,
    /// `profile::safe_choices()`.
    pub profile_choices: Vec<String>,
    pub cpu_temp: Option<i32>,
    pub cpu_watts: Option<u32>,
    pub gpu_present: bool,
    pub gpu_temp: Option<i32>,
    /// "Hybrid" / "dGPU only" when known.
    pub gpu_mode: Option<String>,
    pub fan_rpm: Option<u32>,
    /// `(fan id, max rpm)`; empty when fans are not controllable.
    pub fans: Vec<(u8, u32)>,
    pub fans_auto: bool,
    pub fans_full: bool,
    pub battery_pct: Option<u32>,
    pub charge_limit: Option<u32>,
    pub kbd: Kbd,
    /// `None` = unsupported.
    pub fn_lock: Option<bool>,
}

/// What a menu entry does when clicked.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Show,
    Quit,
    SetProfile(String),
    /// Auto (`full == false`, target 0) or maximum speed on every fan.
    Fans {
        full: bool,
        fans: Vec<(u8, u32)>,
    },
    KbdLevel {
        spectrum: bool,
        level: u8,
    },
    ChargeLimit(u32),
    FnLock(bool),
}

/// Backend-independent menu description.
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    Header(String),
    Item {
        label: String,
        action: Action,
    },
    Check {
        label: String,
        checked: bool,
        action: Action,
    },
    Radio {
        options: Vec<(String, Action)>,
        selected: Option<usize>,
    },
    Sub {
        label: String,
        entries: Vec<Entry>,
    },
    Separator,
}

/// Icon name for a platform profile. Unknown profiles keep the plain icon.
pub fn icon_for_profile(profile: Option<&str>) -> &'static str {
    match profile {
        Some("low-power") | Some("quiet") => "com.encomjp.legion-settings-tray-quiet",
        Some("balanced") | Some("balanced-performance") => {
            "com.encomjp.legion-settings-tray-balanced"
        }
        Some("performance") => "com.encomjp.legion-settings-tray-performance",
        Some("max-power") | Some("custom") => "com.encomjp.legion-settings-tray-extreme",
        _ => ICON_DEFAULT,
    }
}

/// `(title, description)` for the hover tooltip.
pub fn format_tooltip(s: &TrayState) -> (String, String) {
    if !s.connected {
        return (
            "Legion Control".into(),
            "Service not reachable. Click to open".into(),
        );
    }
    let title = match &s.profile {
        Some(p) => format!("Legion Control, {}", friendly_profile(p)),
        None => "Legion Control".into(),
    };
    let mut lines = Vec::new();
    if let Some(t) = s.cpu_temp {
        lines.push(match s.cpu_watts {
            Some(w) => format!("CPU: {t} °C, {w} W"),
            None => format!("CPU: {t} °C"),
        });
    }
    if s.gpu_present {
        lines.push(match (s.gpu_temp, &s.gpu_mode) {
            (None, _) => "GPU: asleep".to_string(),
            (Some(t), Some(m)) => format!("GPU: {t} °C, {m}"),
            (Some(t), None) => format!("GPU: {t} °C"),
        });
    }
    if let Some(rpm) = s.fan_rpm {
        let mode = if s.fans_auto { "auto" } else { "manual" };
        lines.push(if rpm == 0 {
            "Fans: Idle".to_string()
        } else {
            format!("Fans: {rpm} rpm, {mode}")
        });
    }
    if let Some(p) = s.battery_pct {
        lines.push(match s.charge_limit {
            Some(l) => format!("Battery: {p}%, limit {l}%"),
            None => format!("Battery: {p}%"),
        });
    }
    (title, lines.join("\n"))
}

fn kbd_label(spectrum: bool, level: u8, max: u8) -> String {
    match level {
        0 => "Off".into(),
        l if spectrum => match l {
            3 => "Low".into(),
            6 => "Medium".into(),
            9 => "High".into(),
            n => format!("Level {n}"),
        },
        l if max == 2 => (if l == 1 { "Low" } else { "High" }).into(),
        l => format!("Level {l}"),
    }
}

/// Menu entries for a state. The daemon-unreachable menu only offers
/// "Open" and "Quit".
pub fn build_menu(s: &TrayState) -> Vec<Entry> {
    let mut out = Vec::new();
    if !s.connected {
        out.push(Entry::Header("Service not running".into()));
        out.push(Entry::Separator);
    } else {
        if !s.profile_choices.is_empty() {
            out.push(Entry::Header("Power mode".into()));
            out.push(Entry::Radio {
                options: s
                    .profile_choices
                    .iter()
                    .map(|c| (friendly_profile(c), Action::SetProfile(c.clone())))
                    .collect(),
                selected: s
                    .profile
                    .as_ref()
                    .and_then(|p| s.profile_choices.iter().position(|c| c == p)),
            });
            out.push(Entry::Separator);
        }
        if !s.fans.is_empty() {
            let value = if s.fans_auto {
                "Automatic"
            } else if s.fans_full {
                "Full speed"
            } else {
                "Manual"
            };
            out.push(Entry::Sub {
                label: format!("Fans: {value}"),
                entries: vec![Entry::Radio {
                    options: vec![
                        (
                            "Automatic".into(),
                            Action::Fans {
                                full: false,
                                fans: s.fans.clone(),
                            },
                        ),
                        (
                            "Full speed".into(),
                            Action::Fans {
                                full: true,
                                fans: s.fans.clone(),
                            },
                        ),
                    ],
                    selected: if s.fans_auto {
                        Some(0)
                    } else if s.fans_full {
                        Some(1)
                    } else {
                        None
                    },
                }],
            });
        }
        let kbd = match s.kbd {
            Kbd::Spectrum { level } => Some((true, level, 9u8, vec![0u8, 3, 6, 9])),
            Kbd::White { level, max } => Some((false, level, max, (0..=max.min(9)).collect())),
            Kbd::None => None,
        };
        if let Some((spectrum, level, max, levels)) = kbd {
            // Nearest listed level (Spectrum has 10 steps, we offer 4).
            let selected = levels
                .iter()
                .enumerate()
                .min_by_key(|(_, l)| (i16::from(**l) - i16::from(level)).abs())
                .map(|(i, _)| i);
            // Name the level instead of the raw firmware step ("On · 9").
            let summary = if level == 0 {
                "Off".to_string()
            } else if spectrum {
                ["Off", "Low", "Medium", "High"][selected.unwrap_or(3).min(3)].to_string()
            } else {
                format!("Level {level}")
            };
            out.push(Entry::Sub {
                label: format!("Keyboard light: {summary}"),
                entries: vec![Entry::Radio {
                    options: levels
                        .iter()
                        .map(|&l| {
                            (
                                kbd_label(spectrum, l, max),
                                Action::KbdLevel { spectrum, level: l },
                            )
                        })
                        .collect(),
                    selected,
                }],
            });
        }
        if let Some(limit) = s.charge_limit {
            out.push(Entry::Sub {
                label: format!("Charge limit: {limit}%"),
                entries: vec![Entry::Radio {
                    options: CHARGE_LIMITS
                        .iter()
                        .map(|&l| (format!("{l}%"), Action::ChargeLimit(l)))
                        .collect(),
                    selected: CHARGE_LIMITS.iter().position(|&l| l == limit),
                }],
            });
        }
        if let Some(on) = s.fn_lock {
            if !matches!(out.last(), Some(Entry::Separator) | None) {
                out.push(Entry::Separator);
            }
            out.push(Entry::Check {
                label: "Fn lock".into(),
                checked: on,
                action: Action::FnLock(!on),
            });
        }
        out.push(Entry::Separator);
    }
    out.push(Entry::Item {
        label: "Open Legion Control".into(),
        action: Action::Show,
    });
    out.push(Entry::Separator);
    out.push(Entry::Item {
        label: "Quit".into(),
        action: Action::Quit,
    });
    out
}

/// Daemon commands needed to carry out an action (empty for Show/Quit).
pub fn commands_for(action: &Action) -> Vec<DaemonCommand> {
    match action {
        Action::Show | Action::Quit => vec![],
        Action::SetProfile(p) => vec![DaemonCommand::SetProfile(p.clone())],
        Action::Fans { full, fans } => fans
            .iter()
            .map(|&(id, max)| DaemonCommand::SetFanTarget(id, if *full { max } else { 0 }))
            .collect(),
        Action::KbdLevel {
            spectrum: true,
            level,
        } => {
            vec![DaemonCommand::SetRgbBrightness(*level)]
        }
        Action::KbdLevel {
            spectrum: false,
            level,
        } => {
            vec![DaemonCommand::SetKbdBrightness(*level)]
        }
        Action::ChargeLimit(l) => vec![DaemonCommand::SetChargeLimit(*l)],
        Action::FnLock(on) => vec![DaemonCommand::SetFnLock(*on)],
    }
}

// ───────────────────────── ksni glue ─────────────────────────

pub struct LegionTray {
    tx: mpsc::Sender<TrayCmd>,
    actions: mpsc::Sender<Action>,
    state: TrayState,
}

impl LegionTray {
    fn dispatch(&self, action: &Action) {
        match action {
            Action::Show => {
                let _ = self.tx.send(TrayCmd::Show);
            }
            Action::Quit => {
                let _ = self.tx.send(TrayCmd::Quit);
            }
            other => {
                let _ = self.actions.send(other.clone());
            }
        }
    }
}

fn to_ksni(entries: Vec<Entry>) -> Vec<MenuItem<LegionTray>> {
    entries
        .into_iter()
        .map(|e| match e {
            Entry::Separator => MenuItem::Separator,
            Entry::Header(label) => StandardItem {
                label,
                enabled: false,
                ..Default::default()
            }
            .into(),
            Entry::Item { label, action } => StandardItem {
                label,
                activate: Box::new(move |t: &mut LegionTray| t.dispatch(&action)),
                ..Default::default()
            }
            .into(),
            Entry::Check {
                label,
                checked,
                action,
            } => CheckmarkItem {
                label,
                checked,
                activate: Box::new(move |t: &mut LegionTray| t.dispatch(&action)),
                ..Default::default()
            }
            .into(),
            Entry::Radio { options, selected } => {
                let actions: Vec<Action> = options.iter().map(|(_, a)| a.clone()).collect();
                RadioGroup {
                    selected: selected.unwrap_or(usize::MAX),
                    select: Box::new(move |t: &mut LegionTray, i: usize| {
                        if let Some(a) = actions.get(i) {
                            t.dispatch(a);
                        }
                    }),
                    options: options
                        .into_iter()
                        .map(|(label, _)| RadioItem {
                            label,
                            ..Default::default()
                        })
                        .collect(),
                }
                .into()
            }
            Entry::Sub { label, entries } => SubMenu {
                label,
                submenu: to_ksni(entries),
                ..Default::default()
            }
            .into(),
        })
        .collect()
}

/// Minimal markup escaping: tooltip descriptions are rendered as rich text.
fn escape_markup(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

impl Tray for LegionTray {
    fn id(&self) -> String {
        "com.encomjp.legion-settings".into()
    }

    fn title(&self) -> String {
        "Legion Control".into()
    }

    fn icon_name(&self) -> String {
        icon_for_profile(
            self.state
                .profile
                .as_deref()
                .filter(|_| self.state.connected),
        )
        .into()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let (title, description) = format_tooltip(&self.state);
        ksni::ToolTip {
            icon_name: self.icon_name(),
            title,
            description: escape_markup(&description),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.tx.send(TrayCmd::Show);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        to_ksni(build_menu(&self.state))
    }
}

// ───────────────────────── workers ─────────────────────────

fn notify_error(summary: &str, body: &str) {
    log::warn!("tray: {summary}: {body}");
    // Best effort: no notification daemon / notify-send means log only.
    if let Err(e) = std::process::Command::new("notify-send")
        .args([
            "-a",
            "Legion Control",
            "-i",
            ICON_DEFAULT,
            "--",
            summary,
            body,
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|mut c| {
            // Reap in the background so we never leave zombies or block.
            std::thread::spawn(move || {
                let _ = c.wait();
            });
        })
    {
        log::debug!("tray: notify-send unavailable: {e}");
    }
}

/// Run one action's commands; stops at the first failure.
fn run_action(action: &Action) -> Result<(), String> {
    for cmd in commands_for(action) {
        if let DaemonResponse::Error(e) = send_command(cmd)? {
            return Err(e);
        }
    }
    Ok(())
}

/// Read the daemon once. `slow` also refreshes the cached slow-changing data.
struct Poller {
    tick: u32,
    device: Option<legion_core::device::DeviceInfo>,
    gpu_mode: Option<String>,
}

impl Poller {
    fn snapshot(&mut self) -> TrayState {
        let mut s = TrayState::default();
        let sensors = match send_command(DaemonCommand::GetSensors) {
            Ok(DaemonResponse::Sensors(r)) => r,
            _ => {
                self.device = None;
                return s;
            }
        };
        s.connected = true;

        if self.device.is_none() {
            if let Ok(DaemonResponse::DeviceInfo(d)) = send_command(DaemonCommand::GetDeviceInfo) {
                self.device = Some(d);
            }
        }
        if self.tick.is_multiple_of(SLOW_EVERY) {
            if let Ok(DaemonResponse::GpuMode(g)) = send_command(DaemonCommand::GetGpuMode) {
                self.gpu_mode = g.active.map(|m| match m {
                    legion_core::gpu_mode::GpuMode::Hybrid => "Hybrid".to_string(),
                    legion_core::gpu_mode::GpuMode::DgpuOnly => "dGPU only".to_string(),
                });
            }
        }
        self.tick = self.tick.wrapping_add(1);

        s.profile = match send_command(DaemonCommand::GetProfile) {
            Ok(DaemonResponse::Profile(p)) if !p.is_empty() => Some(p),
            _ if !sensors.profile.is_empty() => Some(sensors.profile.clone()),
            _ => None,
        };
        s.profile_choices = legion_core::profile::safe_choices();

        s.cpu_temp = (sensors.cpu_temp > 0.0).then(|| sensors.cpu_temp.round() as i32);
        s.cpu_watts = match send_command(DaemonCommand::GetCpuPower) {
            Ok(DaemonResponse::CpuPower(w)) if w > 0.0 => Some(w.round() as u32),
            _ => None,
        };

        s.gpu_present = self
            .device
            .as_ref()
            .map(|d| !d.gpu_model.trim().is_empty() && d.gpu_model != "Unknown")
            .unwrap_or(sensors.dgpu_temp > 0.0);
        s.gpu_temp = (sensors.dgpu_temp > 0.0).then(|| sensors.dgpu_temp.round() as i32);
        s.gpu_mode = self.gpu_mode.clone();

        if let Some(d) = &self.device {
            s.fans = d
                .capabilities
                .fans
                .iter()
                .map(|f| (f.id, f.max_rpm))
                .collect();
        }
        let rpms = [sensors.fan1_rpm, sensors.fan2_rpm, sensors.fan4_rpm];
        s.fan_rpm = rpms.iter().copied().max();
        let targets = [
            sensors.fan1_target,
            sensors.fan2_target,
            sensors.fan4_target,
        ];
        s.fans_auto = targets.iter().all(|t| *t == 0);
        s.fans_full = !s.fans_auto
            && !s.fans.is_empty()
            && s.fans
                .iter()
                .zip(targets.iter())
                .all(|(&(_, max), &t)| max > 0 && t >= max);

        s.battery_pct = match send_command(DaemonCommand::GetBattery) {
            Ok(DaemonResponse::Battery { capacity, .. }) => Some(capacity),
            _ => (sensors.battery_pct > 0).then_some(sensors.battery_pct),
        };
        s.charge_limit = match send_command(DaemonCommand::GetChargeLimit) {
            Ok(DaemonResponse::ChargeLimit(l)) => Some(l),
            _ => None,
        };

        if let Some(d) = &self.device {
            s.kbd = match d.capabilities.lighting_kind {
                LightingKind::Spectrum => match send_command(DaemonCommand::GetRgbBrightness) {
                    Ok(DaemonResponse::RgbBrightness(level)) => Kbd::Spectrum { level },
                    _ => Kbd::None,
                },
                LightingKind::White => {
                    let max = legion_core::keyboard::max_brightness().unwrap_or(0);
                    match send_command(DaemonCommand::GetKbdBrightness) {
                        Ok(DaemonResponse::KbdBrightness(level)) if max > 0 => {
                            Kbd::White { level, max }
                        }
                        _ => Kbd::None,
                    }
                }
                LightingKind::FourZone => Kbd::None,
            };
        }

        s.fn_lock = legion_core::ideapad::get(legion_core::ideapad::Toggle::FnLock);
        s
    }
}

fn start_workers(handle: ksni::blocking::Handle<LegionTray>, actions: mpsc::Receiver<Action>) {
    let (refresh_tx, refresh_rx) = mpsc::channel::<()>();

    // Poller: sleeps on the refresh channel so an action wakes it early.
    let poll_handle = handle.clone();
    std::thread::Builder::new()
        .name("tray-poll".into())
        .spawn(move || {
            let mut poller = Poller {
                tick: 0,
                device: None,
                gpu_mode: None,
            };
            let mut last: Option<TrayState> = None;
            loop {
                let snap = poller.snapshot();
                if last.as_ref() != Some(&snap) {
                    let pushed = snap.clone();
                    // Only the changed state goes over D-Bus; None = tray shut down.
                    if poll_handle.update(move |t| t.state = pushed).is_none() {
                        return;
                    }
                    last = Some(snap);
                }
                match refresh_rx.recv_timeout(POLL_INTERVAL) {
                    Ok(()) => {
                        // Coalesce bursts of actions into one refresh.
                        while refresh_rx.try_recv().is_ok() {}
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
                if poll_handle.is_closed() {
                    return;
                }
            }
        })
        .map_err(|e| log::warn!("tray: cannot start poll thread: {e}"))
        .ok();

    // Actions: IPC can take seconds, so it never runs in a menu callback.
    std::thread::Builder::new()
        .name("tray-actions".into())
        .spawn(move || {
            while let Ok(action) = actions.recv() {
                if let Err(e) = run_action(&action) {
                    notify_error("Legion Control", &e);
                }
                let _ = refresh_tx.send(());
            }
        })
        .map_err(|e| log::warn!("tray: cannot start action thread: {e}"))
        .ok();
}

pub fn spawn(tx: mpsc::Sender<TrayCmd>) {
    use ksni::blocking::TrayMethods;
    let (actions_tx, actions_rx) = mpsc::channel();
    let tray = LegionTray {
        tx,
        actions: actions_tx,
        state: TrayState::default(),
    };
    match tray.spawn() {
        Ok(handle) => start_workers(handle, actions_rx),
        Err(e) => log::warn!("system tray unavailable: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> TrayState {
        TrayState {
            connected: true,
            profile: Some("balanced".into()),
            profile_choices: vec!["low-power".into(), "balanced".into(), "performance".into()],
            cpu_temp: Some(64),
            cpu_watts: Some(45),
            gpu_present: true,
            gpu_temp: Some(48),
            gpu_mode: Some("Hybrid".into()),
            fan_rpm: Some(1800),
            fans: vec![(1, 5000), (2, 5200)],
            fans_auto: true,
            fans_full: false,
            battery_pct: Some(97),
            charge_limit: Some(100),
            kbd: Kbd::Spectrum { level: 9 },
            fn_lock: Some(false),
        }
    }

    #[test]
    fn tooltip_matches_design() {
        let (t, d) = format_tooltip(&state());
        assert_eq!(t, "Legion Control, Balanced");
        assert_eq!(
            d,
            "CPU: 64 °C, 45 W\nGPU: 48 °C, Hybrid\nFans: 1800 rpm, auto\nBattery: 97%, limit 100%"
        );
    }

    #[test]
    fn tooltip_asleep_manual_and_missing_values() {
        let mut s = state();
        s.gpu_temp = None;
        s.fans_auto = false;
        s.cpu_watts = None;
        s.charge_limit = None;
        let (_, d) = format_tooltip(&s);
        assert!(d.contains("CPU: 64 °C\n"));
        assert!(d.contains("GPU: asleep"));
        assert!(d.contains("1800 rpm, manual"));
        assert!(d.ends_with("Battery: 97%"));
        s.gpu_present = false;
        assert!(!format_tooltip(&s).1.contains("GPU"));
    }

    #[test]
    fn tooltip_fans_idle() {
        let mut s = state();
        s.fan_rpm = Some(0);
        assert!(format_tooltip(&s).1.contains("Fans: Idle"));
        s.fans_auto = false;
        assert!(format_tooltip(&s).1.contains("Fans: Idle"));
        assert!(!format_tooltip(&s).1.contains("  "));
    }

    #[test]
    fn menu_separates_fn_lock_and_quit() {
        let m = build_menu(&state());
        let fn_idx = m
            .iter()
            .position(|e| matches!(e, Entry::Check { .. }))
            .unwrap();
        assert_eq!(m[fn_idx - 1], Entry::Separator);
        // No two separators in a row.
        assert!(!m
            .windows(2)
            .any(|w| w[0] == Entry::Separator && w[1] == Entry::Separator));
    }

    #[test]
    fn tooltip_disconnected() {
        let (t, d) = format_tooltip(&TrayState::default());
        assert_eq!(t, "Legion Control");
        assert!(d.contains("not reachable"));
    }

    #[test]
    fn icon_mapping() {
        let n = |p| icon_for_profile(Some(p));
        assert!(n("low-power").ends_with("-quiet"));
        assert!(n("quiet").ends_with("-quiet"));
        assert!(n("balanced").ends_with("-balanced"));
        assert!(n("performance").ends_with("-performance"));
        assert!(n("max-power").ends_with("-extreme"));
        assert!(n("custom").ends_with("-extreme"));
        assert_eq!(n("weird"), ICON_DEFAULT);
        assert_eq!(icon_for_profile(None), ICON_DEFAULT);
    }

    #[test]
    fn menu_full_state() {
        let m = build_menu(&state());
        assert_eq!(m[0], Entry::Header("Power mode".into()));
        match &m[1] {
            Entry::Radio { options, selected } => {
                assert_eq!(options.len(), 3);
                assert_eq!(*selected, Some(1));
                assert_eq!(options[0].0, "Quiet");
                assert_eq!(options[2].1, Action::SetProfile("performance".into()));
            }
            other => panic!("expected radio, got {other:?}"),
        }
        let labels: Vec<&str> = m
            .iter()
            .filter_map(|e| match e {
                Entry::Sub { label, .. } => Some(label.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            labels,
            [
                "Fans: Automatic",
                "Keyboard light: High",
                "Charge limit: 100%"
            ]
        );
        assert!(m.contains(&Entry::Check {
            label: "Fn lock".into(),
            checked: false,
            action: Action::FnLock(true)
        }));
        assert_eq!(
            m[m.len() - 3],
            Entry::Item {
                label: "Open Legion Control".into(),
                action: Action::Show
            }
        );
        assert_eq!(m[m.len() - 2], Entry::Separator);
        assert_eq!(
            m[m.len() - 1],
            Entry::Item {
                label: "Quit".into(),
                action: Action::Quit
            }
        );
    }

    #[test]
    fn menu_hides_unsupported_features() {
        let mut s = state();
        s.fans.clear();
        s.kbd = Kbd::None;
        s.fn_lock = None;
        s.charge_limit = None;
        let m = build_menu(&s);
        assert!(!m
            .iter()
            .any(|e| matches!(e, Entry::Sub { .. } | Entry::Check { .. })));
    }

    #[test]
    fn menu_disconnected_only_open_and_quit() {
        let m = build_menu(&TrayState::default());
        assert!(m
            .iter()
            .all(|e| matches!(e, Entry::Header(_) | Entry::Separator | Entry::Item { .. })));
        assert_eq!(
            m.iter().filter(|e| matches!(e, Entry::Item { .. })).count(),
            2
        );
    }

    #[test]
    fn menu_unknown_profile_has_no_selection() {
        let mut s = state();
        s.profile = Some("custom".into());
        match &build_menu(&s)[1] {
            Entry::Radio { selected, .. } => assert_eq!(*selected, None),
            _ => panic!(),
        }
    }

    #[test]
    fn keyboard_levels_spectrum_and_white() {
        let find = |s: &TrayState| {
            build_menu(s).into_iter().find_map(|e| match e {
                Entry::Sub { label, entries } if label.starts_with("Keyboard") => Some(entries),
                _ => None,
            })
        };
        let mut s = state();
        s.kbd = Kbd::Spectrum { level: 5 };
        match &find(&s).unwrap()[0] {
            Entry::Radio { options, selected } => {
                let names: Vec<_> = options.iter().map(|o| o.0.as_str()).collect();
                assert_eq!(names, ["Off", "Low", "Medium", "High"]);
                assert_eq!(*selected, Some(2)); // 5 is nearest to 6
            }
            _ => panic!(),
        }
        s.kbd = Kbd::White { level: 1, max: 2 };
        match &find(&s).unwrap()[0] {
            Entry::Radio { options, selected } => {
                let names: Vec<_> = options.iter().map(|o| o.0.as_str()).collect();
                assert_eq!(names, ["Off", "Low", "High"]);
                assert_eq!(*selected, Some(1));
                assert_eq!(
                    options[1].1,
                    Action::KbdLevel {
                        spectrum: false,
                        level: 1
                    }
                );
            }
            _ => panic!(),
        }
        s.kbd = Kbd::Spectrum { level: 0 };
        assert!(build_menu(&s)
            .iter()
            .any(|e| matches!(e, Entry::Sub { label, .. } if label == "Keyboard light: Off")));
    }

    #[test]
    fn charge_limit_selection() {
        let mut s = state();
        s.charge_limit = Some(80);
        let sel = build_menu(&s).into_iter().find_map(|e| match e {
            Entry::Sub { entries, label } if label.starts_with("Charge") => match &entries[0] {
                Entry::Radio { selected, .. } => Some(*selected),
                _ => None,
            },
            _ => None,
        });
        assert_eq!(sel, Some(Some(1)));
    }

    #[test]
    fn fans_manual_and_full_labels() {
        let mut s = state();
        s.fans_auto = false;
        assert!(build_menu(&s)
            .iter()
            .any(|e| matches!(e, Entry::Sub { label, .. } if label == "Fans: Manual")));
        s.fans_full = true;
        assert!(build_menu(&s)
            .iter()
            .any(|e| matches!(e, Entry::Sub { label, .. } if label == "Fans: Full speed")));
    }

    #[test]
    fn action_commands() {
        assert!(commands_for(&Action::Show).is_empty());
        assert!(commands_for(&Action::Quit).is_empty());
        let c = commands_for(&Action::Fans {
            full: true,
            fans: vec![(1, 5000), (2, 5200)],
        });
        assert!(matches!(
            c[..],
            [
                DaemonCommand::SetFanTarget(1, 5000),
                DaemonCommand::SetFanTarget(2, 5200)
            ]
        ));
        let c = commands_for(&Action::Fans {
            full: false,
            fans: vec![(1, 5000)],
        });
        assert!(matches!(c[..], [DaemonCommand::SetFanTarget(1, 0)]));
        assert!(matches!(
            commands_for(&Action::KbdLevel {
                spectrum: true,
                level: 3
            })[..],
            [DaemonCommand::SetRgbBrightness(3)]
        ));
        assert!(matches!(
            commands_for(&Action::KbdLevel {
                spectrum: false,
                level: 1
            })[..],
            [DaemonCommand::SetKbdBrightness(1)]
        ));
        assert!(matches!(
            commands_for(&Action::ChargeLimit(80))[..],
            [DaemonCommand::SetChargeLimit(80)]
        ));
        assert!(matches!(
            commands_for(&Action::FnLock(true))[..],
            [DaemonCommand::SetFnLock(true)]
        ));
    }

    #[test]
    fn markup_is_escaped() {
        assert_eq!(escape_markup("a<b>&c"), "a&lt;b&gt;&amp;c");
    }
}
