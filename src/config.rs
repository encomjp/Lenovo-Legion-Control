//! Persistent app settings (`~/.config/legion-control/settings.json`).

use crate::keyboard::{RgbEffect, RgbZone};
use crate::thermal::ThermalConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const VERSION: u32 = 4;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneEffect {
    pub effect: String,
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub speed: u8,
    #[serde(default = "default_brightness")]
    pub brightness: u8,
}

impl Default for ZoneEffect {
    fn default() -> Self {
        Self {
            effect: "static".into(),
            r: 200,
            g: 16,
            b: 46,
            speed: 2,
            brightness: 9,
        }
    }
}

impl ZoneEffect {
    pub fn rgb_effect(&self) -> RgbEffect {
        let fx = if self.effect.eq_ignore_ascii_case("off") {
            RgbEffect::Static
        } else {
            RgbEffect::from_name(&self.effect).unwrap_or(RgbEffect::Static)
        };
        log::trace!(
            "config::ZoneEffect::rgb_effect(effect={:?}) — result={fx:?} (unknown names fall back to Static)",
            self.effect
        );
        fx
    }

    pub fn colors(&self) -> Vec<(u8, u8, u8)> {
        if self.effect.eq_ignore_ascii_case("off") {
            log::trace!("config::ZoneEffect::colors(effect=off) — returning black");
            return vec![(0, 0, 0)];
        }
        let fx = self.rgb_effect();
        let bri = self.brightness.min(9) as f64 / 9.0;
        let c = if fx.needs_color() || matches!(fx, RgbEffect::Static) {
            vec![(
                (self.r as f64 * bri).round() as u8,
                (self.g as f64 * bri).round() as u8,
                (self.b as f64 * bri).round() as u8,
            )]
        } else {
            Vec::new()
        };
        log::trace!(
            "config::ZoneEffect::colors(effect={:?}, brightness={}) — computed {} color(s)",
            self.effect,
            self.brightness,
            c.len()
        );
        c
    }

    pub fn is_off(&self) -> bool {
        self.effect.eq_ignore_ascii_case("off")
    }
}

/// Snapshot of power + lighting that can be saved as a named profile or last session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfile {
    #[serde(default = "default_platform_profile")]
    pub platform_profile: String,
    #[serde(default)]
    pub ppt: HashMap<String, u32>,
    #[serde(default)]
    pub fan1: u32,
    #[serde(default)]
    pub fan2: u32,
    #[serde(default)]
    pub fan4: u32,
    #[serde(default = "default_brightness")]
    pub brightness: u8,
    #[serde(default = "default_true")]
    pub logo_on: bool,
    #[serde(default = "default_lighting_mode")]
    pub lighting_mode: String,
    #[serde(default)]
    pub keyboard: ZoneEffect,
    #[serde(default)]
    pub front: ZoneEffect,
    #[serde(default)]
    pub rear: ZoneEffect,
    #[serde(default)]
    pub logo: ZoneEffect,
    #[serde(default)]
    pub per_key: HashMap<String, [u8; 3]>,
    #[serde(default = "default_charge_limit")]
    pub charge_limit: u32,
    #[serde(default = "default_keyboard_layout")]
    pub keyboard_layout: String,
    #[serde(default = "default_ui_r")]
    pub ui_r: u8,
    #[serde(default = "default_ui_g")]
    pub ui_g: u8,
    #[serde(default = "default_ui_b")]
    pub ui_b: u8,
}

fn default_platform_profile() -> String {
    "balanced".into()
}
fn default_brightness() -> u8 {
    9
}
fn default_true() -> bool {
    true
}
fn default_ui_r() -> u8 {
    200
}
fn default_ui_g() -> u8 {
    16
}
fn default_ui_b() -> u8 {
    46
}

impl Default for UserProfile {
    fn default() -> Self {
        Self {
            platform_profile: default_platform_profile(),
            ppt: HashMap::new(),
            fan1: 0,
            fan2: 0,
            fan4: 0,
            brightness: default_brightness(),
            logo_on: true,
            lighting_mode: default_lighting_mode(),
            keyboard: ZoneEffect::default(),
            front: ZoneEffect::default(),
            rear: ZoneEffect::default(),
            logo: ZoneEffect::default(),
            per_key: HashMap::new(),
            charge_limit: 100,
            keyboard_layout: default_keyboard_layout(),
            ui_r: default_ui_r(),
            ui_g: default_ui_g(),
            ui_b: default_ui_b(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub version: u32,
    pub brightness: u8,
    pub logo_on: bool,
    /// `effects` = zone effects; `per-key` = painted keyboard map.
    #[serde(default = "default_lighting_mode")]
    pub lighting_mode: String,
    /// Last UI zone / effect / speed / colour picker.
    pub ui_zone: String,
    pub ui_effect: String,
    pub ui_speed: u8,
    pub ui_r: u8,
    pub ui_g: u8,
    pub ui_b: u8,
    pub keyboard: ZoneEffect,
    pub front: ZoneEffect,
    pub rear: ZoneEffect,
    pub logo: ZoneEffect,
    /// Per-key colours keyed by Spectrum key name (`esc`, `q`, …).
    #[serde(default)]
    pub per_key: HashMap<String, [u8; 3]>,
    #[serde(default = "default_charge_limit")]
    pub charge_limit: u32,
    /// Per-key painter layout: `de` or `us`.
    #[serde(default = "default_keyboard_layout")]
    pub keyboard_layout: String,
    /// Re-apply last session (profile / fans / lighting / charge) on launch.
    #[serde(default = "default_true")]
    pub restore_on_launch: bool,
    /// Last known power + lighting snapshot (updated as you change settings).
    #[serde(default)]
    pub last_session: UserProfile,
    /// Named user profiles (Save / Load in the GUI).
    #[serde(default)]
    pub profiles: HashMap<String, UserProfile>,
    #[serde(default)]
    pub active_profile: String,
    /// First-launch welcome dialog already shown.
    #[serde(default)]
    pub welcome_seen: bool,
    #[serde(default)]
    pub thermal: ThermalConfig,
    /// Optional anonymous diagnostics (alpha) — off unless the user opts in.
    #[serde(default)]
    pub diagnostics: DiagnosticsConfig,
}

/// Alpha telemetry settings. Enabled by default (opt-out): one anonymized
/// report is sent on schedule unless the user turns `enabled` off.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticsConfig {
    /// Telemetry is ON by default; set `false` to opt out. A user-facing
    /// warning offers opt-out wherever the setting is shown.
    #[serde(default = "default_diagnostics_enabled")]
    pub enabled: bool,
    /// Empty string = use the built-in default collector URL.
    #[serde(default)]
    pub endpoint: String,
    /// Auto-send interval in seconds; 0 = manual only. Env override:
    /// `LEGION_TELEMETRY_INTERVAL_SECS`. Rust default 60 s for NAT-friendly
    /// push cadence. `auto_period_hours` is legacy compat (hours → secs).
    #[serde(default = "default_auto_interval_secs")]
    pub auto_interval_secs: u32,
    /// RFC3339 timestamp of the last successful send (informational).
    #[serde(default)]
    pub last_sent: Option<String>,
    /// Pseudonymous machine ID (UUID v4) generated on first send. Lets
    /// the operator correlate reports from the same machine over time.
    #[serde(default)]
    pub machine_id: String,
    /// Legacy hours field — migrated to `auto_interval_secs` on load.
    #[serde(default)]
    pub auto_period_hours: u32,
}

impl Default for DiagnosticsConfig {
    /// Opt-out default: telemetry starts enabled at 60 s cadence.
    fn default() -> Self {
        Self {
            enabled: default_diagnostics_enabled(),
            endpoint: String::new(),
            auto_interval_secs: default_auto_interval_secs(),
            last_sent: None,
            machine_id: String::new(),
            auto_period_hours: 0,
        }
    }
}

impl DiagnosticsConfig {
    /// Generate a machine_id if one doesn't exist yet. Called on the first
    /// send so the ID is stable from the first report onward.
    pub fn ensure_machine_id(&mut self) {
        if self.machine_id.is_empty() {
            log::debug!("config::DiagnosticsConfig::ensure_machine_id — no machine_id present, generating one");
            use std::io::Read;
            let mut b = [0u8; 16];
            if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
                let _ = f.read_exact(&mut b);
            } else {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0);
                let pid = std::process::id() as u128;
                let val = now ^ (pid << 64);
                b = val.to_be_bytes();
            }
            b[6] = (b[6] & 0x0f) | 0x40;
            b[8] = (b[8] & 0x3f) | 0x80;
            self.machine_id = format!(
                "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
                b[0], b[1], b[2], b[3],
                b[4], b[5],
                b[6], b[7],
                b[8], b[9],
                b[10], b[11], b[12], b[13], b[14], b[15]
            );
            log::info!(
                "config::DiagnosticsConfig::ensure_machine_id — generated new machine_id {}",
                self.machine_id
            );
        } else {
            log::trace!(
                "config::DiagnosticsConfig::ensure_machine_id — machine_id already present, preserved"
            );
        }
    }
}

fn default_lighting_mode() -> String {
    "effects".into()
}

fn default_charge_limit() -> u32 {
    100
}

fn default_auto_interval_secs() -> u32 {
    60
}

/// Opt-out default: telemetry is enabled unless the user disables it.
fn default_diagnostics_enabled() -> bool {
    true
}
fn default_keyboard_layout() -> String {
    "de".into()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: VERSION,
            brightness: 9,
            logo_on: true,
            lighting_mode: default_lighting_mode(),
            ui_zone: "all".into(),
            ui_effect: "static".into(),
            ui_speed: 2,
            ui_r: 200,
            ui_g: 16,
            ui_b: 46,
            keyboard: ZoneEffect::default(),
            front: ZoneEffect::default(),
            rear: ZoneEffect::default(),
            logo: ZoneEffect::default(),
            per_key: HashMap::new(),
            charge_limit: 100,
            keyboard_layout: default_keyboard_layout(),
            restore_on_launch: true,
            last_session: UserProfile::default(),
            profiles: HashMap::new(),
            active_profile: String::new(),
            welcome_seen: false,
            thermal: ThermalConfig::default(),
            diagnostics: DiagnosticsConfig::default(),
        }
    }
}

pub fn welcome_seen() -> bool {
    log::trace!("config::welcome_seen()");
    get().welcome_seen
}

pub fn mark_welcome_seen() {
    log::trace!("config::mark_welcome_seen()");
    update(|cfg| {
        cfg.welcome_seen = true;
        log::debug!("config::mark_welcome_seen — welcome_seen set to true");
    });
}

fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let mut h = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
            if h.as_os_str().is_empty() {
                log::trace!("config::config_path — XDG_CONFIG_HOME unset and HOME empty");
            }
            h.push(".config");
            h
        });
    let p = base.join("legion-control").join("settings.json");
    log::trace!("config::config_path — resolved to {}", p.display());
    p
}

fn lock_path() -> PathBuf {
    config_path().with_file_name(".settings.lock")
}

/// Run `f` while holding an exclusive advisory lock on a lockfile next to
/// settings.json. Serializes read-modify-write cycles between processes
/// (daemon, GUI, CLI) so concurrent updates cannot clobber each other.
fn with_config_lock<T>(f: impl FnOnce() -> T) -> T {
    let path = lock_path();
    log::trace!(
        "config::with_config_lock — acquiring lock at {}",
        path.display()
    );
    if let Some(parent) = path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            log::debug!(
                "config::with_config_lock — create_dir_all({}) failed: {e}",
                parent.display()
            );
        }
    }
    match fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
    {
        Ok(file) => {
            use std::os::fd::AsRawFd;
            // SAFETY: flock on a valid fd — plain POSIX call, no memory concerns.
            let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
            if rc != 0 {
                log::warn!(
                    "config lock failed ({}): {}",
                    path.display(),
                    std::io::Error::last_os_error()
                );
            } else {
                log::trace!(
                    "config::with_config_lock — lock acquired ({})",
                    path.display()
                );
            }
            let out = f();
            // SAFETY: unlocking the fd we locked above.
            unsafe {
                libc::flock(file.as_raw_fd(), libc::LOCK_UN);
            }
            log::trace!(
                "config::with_config_lock — lock released ({})",
                path.display()
            );
            out
        }
        Err(e) => {
            log::warn!("config lock open failed ({}): {e}", path.display());
            f()
        }
    }
}

fn store() -> &'static Mutex<AppConfig> {
    static STORE: OnceLock<Mutex<AppConfig>> = OnceLock::new();
    STORE.get_or_init(|| {
        log::trace!("config::store — initializing config store from disk");
        Mutex::new(with_config_lock(load_from_disk))
    })
}

fn load_from_disk() -> AppConfig {
    let path = config_path();
    log::trace!("config::load_from_disk — reading {}", path.display());
    match fs::read_to_string(&path) {
        Ok(s) => match serde_json::from_str::<AppConfig>(&s) {
            Ok(mut parsed) => {
                log::debug!(
                    "config::load_from_disk — loaded {} ({} bytes)",
                    path.display(),
                    s.len()
                );
                // Migrate legacy auto_period_hours (hours → secs) if auto_interval_secs is still default and hours is non-zero.
                if parsed.diagnostics.auto_period_hours != 0
                    && parsed.diagnostics.auto_interval_secs == default_auto_interval_secs()
                {
                    let migrated = parsed.diagnostics.auto_period_hours.saturating_mul(3600);
                    log::info!(
                        "config migration: auto_period_hours {} → auto_interval_secs {}",
                        parsed.diagnostics.auto_period_hours,
                        migrated
                    );
                    parsed.diagnostics.auto_interval_secs = migrated;
                }
                parsed
            }
            Err(e) => {
                // A truncated/corrupt file must not silently wipe profiles and
                // per-key data: preserve it for manual recovery, then reset.
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "settings.json".into());
                let backup = path.with_file_name(format!(
                    "{name}.corrupt-{}",
                    chrono::Local::now().format("%Y%m%d-%H%M%S")
                ));
                match fs::rename(&path, &backup) {
                    Ok(()) => log::error!(
                        "config parse error — moved corrupt file to {}, using defaults: {e}",
                        backup.display()
                    ),
                    Err(re) => log::error!(
                        "config parse error ({}): {e} — using defaults (could not preserve file: {re})",
                        path.display()
                    ),
                }
                AppConfig::default()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::debug!(
                "config::load_from_disk — no settings file at {}, using defaults",
                path.display()
            );
            AppConfig::default()
        }
        Err(e) => {
            log::warn!("config read failed ({}): {e}", path.display());
            AppConfig::default()
        }
    }
}

fn write_disk(cfg: &AppConfig) {
    let path = config_path();
    log::trace!(
        "config::write_disk — writing version={} config to {}",
        cfg.version,
        path.display()
    );
    if let Some(parent) = path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            log::debug!(
                "config::write_disk — create_dir_all({}) failed: {e}",
                parent.display()
            );
        }
    }
    let s = match serde_json::to_string_pretty(cfg) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("config serialize failed: {e}");
            return;
        }
    };
    // Atomic write: temp file in the same directory + rename. A crash or
    // power loss mid-write can never leave a truncated settings.json behind.
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "settings.json".into());
    let tmp = path.with_file_name(format!("{name}.tmp-{}", std::process::id()));
    use std::io::Write;
    let write_result = fs::File::create(&tmp).and_then(|mut f| {
        f.write_all(s.as_bytes())?;
        f.sync_all()
    });
    match write_result {
        Ok(()) => match fs::rename(&tmp, &path) {
            Ok(()) => {
                log::info!(
                    "config::write_disk — settings written to {}",
                    path.display()
                );
                log::debug!("config saved → {}", path.display());
            }
            Err(e) => {
                log::warn!("config rename failed ({}): {e}", path.display());
                if let Err(re) = fs::remove_file(&tmp) {
                    log::trace!(
                        "config::write_disk — cleanup of {} returned None: {re}",
                        tmp.display()
                    );
                }
            }
        },
        Err(e) => {
            log::warn!("config write failed ({}): {e}", tmp.display());
            if let Err(re) = fs::remove_file(&tmp) {
                log::trace!(
                    "config::write_disk — cleanup of {} returned None: {re}",
                    tmp.display()
                );
            }
        }
    }
}

pub fn get() -> AppConfig {
    log::trace!("config::get()");
    match store().lock() {
        Ok(g) => g.clone(),
        Err(_) => {
            log::warn!("config::get — config mutex poisoned, unwrap_or_default fallback used");
            AppConfig::default()
        }
    }
}

pub fn update(f: impl FnOnce(&mut AppConfig)) {
    log::trace!("config::update()");
    let mut g = match store().lock() {
        Ok(g) => g,
        Err(p) => {
            log::warn!("config::update — config mutex poisoned, recovering inner state");
            p.into_inner()
        }
    };
    with_config_lock(|| {
        // Re-read the on-disk state first: another process (daemon/GUI) may
        // have written since our cache was last updated. We are already
        // holding the config lock, so this read is serialized too.
        *g = load_from_disk();
        log::trace!("config::update — invoking caller closure");
        f(&mut g);
        g.version = VERSION;
        log::debug!("config::update — closure applied, writing updated config");
        write_disk(&g);
    });
}

pub fn config_dir_display() -> String {
    let d = config_path()
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "~/.config/legion-control".into());
    log::trace!("config::config_dir_display — result={d:?}");
    d
}

/// Capture current lighting + remembered power fields into a UserProfile.
pub fn snapshot_user_profile() -> UserProfile {
    log::trace!("config::snapshot_user_profile()");
    let cfg = get();
    let mut p = cfg.last_session.clone();
    p.brightness = cfg.brightness;
    p.logo_on = cfg.logo_on;
    p.lighting_mode = cfg.lighting_mode.clone();
    p.keyboard = cfg.keyboard.clone();
    p.front = cfg.front.clone();
    p.rear = cfg.rear.clone();
    p.logo = cfg.logo.clone();
    p.per_key = cfg.per_key.clone();
    p.charge_limit = cfg.charge_limit;
    p.keyboard_layout = cfg.keyboard_layout.clone();
    p.ui_r = cfg.ui_r;
    p.ui_g = cfg.ui_g;
    p.ui_b = cfg.ui_b;
    log::debug!(
        "config::snapshot_user_profile — captured fields: brightness={}, logo_on={}, lighting_mode={:?}, charge_limit={}, keyboard_layout={:?}, per_key={} keys, zones(k/f/r/l)=({:?},{:?},{:?},{:?}), ui_rgb=({},{},{})",
        p.brightness,
        p.logo_on,
        p.lighting_mode,
        p.charge_limit,
        p.keyboard_layout,
        p.per_key.len(),
        p.keyboard.effect,
        p.front.effect,
        p.rear.effect,
        p.logo.effect,
        p.ui_r,
        p.ui_g,
        p.ui_b
    );
    p
}

/// Write a profile’s lighting fields into the live AppConfig.
pub fn apply_profile_to_config(p: &UserProfile) {
    log::trace!(
        "config::apply_profile_to_config(brightness={}, logo_on={}, lighting_mode={:?}, charge_limit={}, keyboard_layout={:?}, per_key={} keys)",
        p.brightness,
        p.logo_on,
        p.lighting_mode,
        p.charge_limit,
        p.keyboard_layout,
        p.per_key.len()
    );
    update(|cfg| {
        log::debug!(
            "config::apply_profile_to_config — applying profile fields (brightness={}, charge_limit={}, lighting_mode={:?}) to live config",
            p.brightness,
            p.charge_limit,
            p.lighting_mode
        );
        cfg.brightness = p.brightness;
        cfg.logo_on = p.logo_on;
        cfg.lighting_mode = p.lighting_mode.clone();
        cfg.keyboard = p.keyboard.clone();
        cfg.front = p.front.clone();
        cfg.rear = p.rear.clone();
        cfg.logo = p.logo.clone();
        cfg.per_key = p.per_key.clone();
        cfg.charge_limit = p.charge_limit;
        cfg.keyboard_layout = p.keyboard_layout.clone();
        cfg.ui_r = p.ui_r;
        cfg.ui_g = p.ui_g;
        cfg.ui_b = p.ui_b;
        cfg.last_session = p.clone();
    });
}

pub fn save_named_profile(name: &str) {
    log::trace!("config::save_named_profile(name={name:?})");
    let name = name.trim();
    if name.is_empty() {
        log::debug!("config::save_named_profile — empty profile name, ignoring");
        return;
    }
    let snap = snapshot_user_profile();
    log::debug!(
        "config::save_named_profile — snapshot for {name:?}: brightness={}, charge_limit={}, lighting_mode={:?}",
        snap.brightness,
        snap.charge_limit,
        snap.lighting_mode
    );
    update(|cfg| {
        cfg.profiles.insert(name.to_string(), snap.clone());
        cfg.active_profile = name.to_string();
        cfg.last_session = snap;
        log::info!(
            "config::save_named_profile — profile {name:?} saved and set active ({} profile(s) stored)",
            cfg.profiles.len()
        );
    });
}

pub fn delete_named_profile(name: &str) {
    log::trace!("config::delete_named_profile(name={name:?})");
    update(|cfg| {
        let removed = cfg.profiles.remove(name).is_some();
        if cfg.active_profile == name {
            cfg.active_profile.clear();
        }
        if removed {
            log::info!("config::delete_named_profile — deleted profile {name:?}");
        } else {
            log::debug!(
                "config::delete_named_profile — profile {name:?} not found, nothing deleted"
            );
        }
    });
}

pub fn list_profile_names() -> Vec<String> {
    log::trace!("config::list_profile_names()");
    let mut names: Vec<String> = get().profiles.keys().cloned().collect();
    names.sort();
    log::trace!(
        "config::list_profile_names — found {} profile(s)",
        names.len()
    );
    names
}

pub fn get_named_profile(name: &str) -> Option<UserProfile> {
    let r = get().profiles.get(name).cloned();
    log::trace!(
        "config::get_named_profile(name={name:?}) — found={}",
        r.is_some()
    );
    r
}

pub fn remember_platform_profile(name: &str) {
    log::trace!("config::remember_platform_profile(name={name:?})");
    update(|cfg| {
        cfg.last_session.platform_profile = name.to_string();
    });
}

pub fn remember_ppt(id: &str, watts: u32) {
    log::trace!("config::remember_ppt(id={id}, watts={watts})");
    update(|cfg| {
        cfg.last_session.ppt.insert(id.to_string(), watts);
    });
}

pub fn remember_fan(fan: u8, rpm: u32) {
    log::trace!("config::remember_fan(fan={fan}, rpm={rpm})");
    update(|cfg| match fan {
        1 => cfg.last_session.fan1 = rpm,
        2 => cfg.last_session.fan2 = rpm,
        4 => cfg.last_session.fan4 = rpm,
        _ => log::debug!("config::remember_fan — unknown fan id {fan}, ignored"),
    });
}

/// Update one or more zones from a UI/CLI apply, then return the full layer set.
pub fn apply_zone_update(
    zone: RgbZone,
    effect_name: &str,
    r: u8,
    g: u8,
    b: u8,
    speed: u8,
    brightness: u8,
) -> AppConfig {
    log::trace!(
        "config::apply_zone_update(zone={:?}, effect={:?}, rgb=({r},{g},{b}), speed={speed}, brightness={brightness})",
        zone,
        effect_name
    );
    let layer = ZoneEffect {
        effect: effect_name.to_string(),
        r,
        g,
        b,
        speed: speed.clamp(1, 3),
        brightness: brightness.min(9),
    };
    update(|cfg| {
        cfg.lighting_mode = "effects".into();
        cfg.ui_zone = zone.name().into();
        cfg.ui_effect = effect_name.into();
        cfg.ui_speed = layer.speed;
        cfg.ui_r = r;
        cfg.ui_g = g;
        cfg.ui_b = b;
        match zone {
            RgbZone::All => {
                cfg.keyboard = layer.clone();
                cfg.front = layer.clone();
                cfg.rear = layer.clone();
                cfg.logo = layer;
            }
            RgbZone::Keyboard => cfg.keyboard = layer,
            RgbZone::Front => cfg.front = layer,
            RgbZone::Rear => cfg.rear = layer,
            RgbZone::Logo => cfg.logo = layer,
            RgbZone::Chassis => {
                cfg.front = layer.clone();
                cfg.rear = layer;
            }
        }
        cfg.last_session.keyboard = cfg.keyboard.clone();
        cfg.last_session.front = cfg.front.clone();
        cfg.last_session.rear = cfg.rear.clone();
        cfg.last_session.logo = cfg.logo.clone();
        cfg.last_session.lighting_mode = cfg.lighting_mode.clone();
        cfg.last_session.ui_r = cfg.ui_r;
        cfg.last_session.ui_g = cfg.ui_g;
        cfg.last_session.ui_b = cfg.ui_b;
    });
    log::debug!("config::apply_zone_update — zone update persisted, returning fresh config");
    get()
}

pub fn set_brightness(level: u8) {
    log::trace!("config::set_brightness(level={level})");
    update(|cfg| {
        cfg.brightness = level.min(9);
        cfg.last_session.brightness = cfg.brightness;
    });
}

pub fn set_logo_on(on: bool) {
    log::trace!("config::set_logo_on(on={on})");
    update(|cfg| {
        cfg.logo_on = on;
        cfg.last_session.logo_on = on;
    });
}

pub fn set_charge_limit(pct: u32) {
    log::trace!("config::set_charge_limit(pct={pct})");
    update(|cfg| {
        cfg.charge_limit = pct;
        cfg.last_session.charge_limit = pct;
    });
    log::debug!("config::set_charge_limit — charge limit setting persisted as {pct}%");
}

pub fn set_per_key_color(key: &str, r: u8, g: u8, b: u8) {
    log::trace!("config::set_per_key_color(key={key}, rgb=({r},{g},{b}))");
    update(|cfg| {
        cfg.lighting_mode = "per-key".into();
        cfg.per_key.insert(key.to_string(), [r, g, b]);
        cfg.last_session.lighting_mode = "per-key".into();
        cfg.last_session.per_key = cfg.per_key.clone();
    });
}

pub fn clear_per_key() {
    log::trace!("config::clear_per_key()");
    update(|cfg| {
        cfg.per_key.clear();
        cfg.lighting_mode = "effects".into();
        cfg.last_session.per_key.clear();
        cfg.last_session.lighting_mode = "effects".into();
    });
}

pub fn set_ui_color(r: u8, g: u8, b: u8) {
    log::trace!("config::set_ui_color(rgb=({r},{g},{b}))");
    update(|cfg| {
        cfg.ui_r = r;
        cfg.ui_g = g;
        cfg.ui_b = b;
        cfg.last_session.ui_r = r;
        cfg.last_session.ui_g = g;
        cfg.last_session.ui_b = b;
    });
}

pub fn set_keyboard_layout(layout: &str) {
    log::trace!("config::set_keyboard_layout(layout={layout})");
    update(|cfg| {
        cfg.keyboard_layout = layout.into();
        cfg.last_session.keyboard_layout = layout.into();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Serialise tests that mutate process-wide env vars.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_isolated_config_dir(f: impl FnOnce(PathBuf)) {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("legion-test-{}-{}", std::process::id(), {
            use std::sync::atomic::{AtomicU64, Ordering};
            static CTR: AtomicU64 = AtomicU64::new(0);
            CTR.fetch_add(1, Ordering::Relaxed)
        }));
        let prev = std::env::var_os("XDG_CONFIG_HOME");
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &dir) };
        let _ = fs::create_dir_all(dir.join("legion-control"));
        f(dir.clone());
        let _ = fs::remove_dir_all(&dir);
        match prev {
            Some(v) => unsafe { std::env::set_var("XDG_CONFIG_HOME", v) },
            None => unsafe { std::env::remove_var("XDG_CONFIG_HOME") },
        }
    }

    #[test]
    fn write_disk_is_atomic_and_readable() {
        with_isolated_config_dir(|base| {
            let cfg = AppConfig {
                brightness: 7,
                charge_limit: 80,
                ..Default::default()
            };
            write_disk(&cfg);
            let loaded = load_from_disk();
            assert_eq!(loaded.brightness, 7);
            assert_eq!(loaded.charge_limit, 80);
            // No .tmp left behind.
            let cfg_dir = base.join("legion-control");
            for e in fs::read_dir(&cfg_dir).unwrap() {
                let name = e.unwrap().file_name().to_string_lossy().to_string();
                assert!(!name.contains(".tmp-"), "tmp file left behind: {name}");
            }
        });
    }

    #[test]
    fn corrupt_file_is_preserved_not_silently_lost() {
        with_isolated_config_dir(|base| {
            let path = base.join("legion-control").join("settings.json");
            let _ = fs::create_dir_all(path.parent().unwrap());
            fs::write(&path, "{ not valid json").unwrap();
            let loaded = load_from_disk();
            assert_eq!(loaded.brightness, AppConfig::default().brightness);
            assert!(!path.exists(), "corrupt file should have been moved away");
            let cfg_dir = base.join("legion-control");
            let mut found_backup = false;
            for e in fs::read_dir(&cfg_dir).unwrap() {
                let name = e.unwrap().file_name().to_string_lossy().to_string();
                if name.contains(".corrupt-") {
                    found_backup = true;
                }
            }
            assert!(found_backup, "corrupt backup not created");
        });
    }

    #[test]
    fn missing_file_returns_defaults() {
        with_isolated_config_dir(|_| {
            let loaded = load_from_disk();
            assert_eq!(loaded.version, AppConfig::default().version);
        });
    }

    #[test]
    fn zone_effect_off_detection() {
        let off_lower = ZoneEffect {
            effect: "off".into(),
            ..Default::default()
        };
        assert!(off_lower.is_off());
        let off_upper = ZoneEffect {
            effect: "OFF".into(),
            ..Default::default()
        };
        assert!(off_upper.is_off());
        let on = ZoneEffect {
            effect: "static".into(),
            ..Default::default()
        };
        assert!(!on.is_off());
    }

    #[test]
    fn zone_effect_colors_respect_off_and_brightness() {
        let mut z = ZoneEffect {
            effect: "off".into(),
            r: 255,
            g: 0,
            b: 0,
            speed: 2,
            brightness: 9,
        };
        assert_eq!(z.colors(), vec![(0, 0, 0)]);
        z.effect = "rainbow-wave".into();
        // Rainbow does not need a color → empty even with bright value.
        assert!(z.colors().is_empty());
        z.effect = "static".into();
        z.r = 200;
        z.g = 0;
        z.b = 0;
        z.brightness = 9;
        assert_eq!(z.colors(), vec![(200, 0, 0)]);
        // Dimmed via brightness scaler.
        z.brightness = 0;
        assert_eq!(z.colors(), vec![(0, 0, 0)]);
    }

    #[test]
    fn zone_effect_rgb_effect_fallback_is_static() {
        let z = ZoneEffect {
            effect: "not-a-real-effect".into(),
            ..Default::default()
        };
        assert_eq!(z.rgb_effect(), crate::keyboard::RgbEffect::Static);
    }

    /// Regression: legacy `auto_period_hours` must migrate into
    /// `auto_interval_secs` on load (hours → seconds) when the interval is
    /// still the default, so old configs stop being ignored.
    #[test]
    fn auto_period_hours_migrates_to_interval_seconds() {
        with_isolated_config_dir(|_| {
            // A legacy config keeps the serde-default interval (60 s) and only
            // bumps auto_period_hours.
            let mut cfg = AppConfig::default();
            cfg.diagnostics.auto_interval_secs = default_auto_interval_secs();
            cfg.diagnostics.auto_period_hours = 2;
            write_disk(&cfg);
            let loaded = load_from_disk();
            assert_eq!(loaded.diagnostics.auto_interval_secs, 2 * 3600);
        });
    }

    /// A legacy hours value of 0 (unset) must NOT trigger migration and must
    /// leave the default interval untouched.
    #[test]
    fn auto_period_hours_zero_keeps_default_interval() {
        with_isolated_config_dir(|_| {
            let mut cfg = AppConfig::default();
            cfg.diagnostics.auto_interval_secs = default_auto_interval_secs();
            cfg.diagnostics.auto_period_hours = 0;
            write_disk(&cfg);
            let loaded = load_from_disk();
            assert_eq!(
                loaded.diagnostics.auto_interval_secs,
                default_auto_interval_secs()
            );
        });
    }

    /// A config that already set auto_interval_secs must not be clobbered by
    /// the legacy hours field — explicit modern config wins.
    #[test]
    fn auto_interval_secs_takes_precedence_over_legacy_hours() {
        with_isolated_config_dir(|_| {
            let mut cfg = AppConfig::default();
            cfg.diagnostics.auto_interval_secs = 30;
            cfg.diagnostics.auto_period_hours = 3;
            write_disk(&cfg);
            let loaded = load_from_disk();
            assert_eq!(loaded.diagnostics.auto_interval_secs, 30);
        });
    }

    /// Opt-out contract: telemetry must default to ENABLED at the 60 s
    /// cadence, and an explicit opt-out (enabled:false) must survive a
    /// round-trip. New installs report by default; users turn it off.
    #[test]
    fn diagnostics_defaults_to_enabled_opt_out() {
        let d = DiagnosticsConfig::default();
        assert!(d.enabled, "telemetry should default to enabled (opt-out)");
        assert_eq!(d.auto_interval_secs, default_auto_interval_secs());
        with_isolated_config_dir(|_| {
            // A config written from the default round-trips to enabled:true.
            write_disk(&AppConfig::default());
            let loaded = load_from_disk();
            assert!(loaded.diagnostics.enabled);
            assert_eq!(
                loaded.diagnostics.auto_interval_secs,
                default_auto_interval_secs()
            );
        });
        with_isolated_config_dir(|_| {
            // An explicit opt-out persists and is honored on reload.
            let mut cfg = AppConfig::default();
            cfg.diagnostics.enabled = false;
            write_disk(&cfg);
            let loaded = load_from_disk();
            assert!(!loaded.diagnostics.enabled, "opt-out must be preserved");
        });
    }

    /// ensure_machine_id must mint a canonical UUID v4 (version nibble 4,
    /// variant 8/9/a/b) exactly once and preserve an existing id thereafter.
    #[test]
    fn ensure_machine_id_mints_valid_v4_and_preserves_existing() {
        let mut cfg = DiagnosticsConfig::default();
        assert!(cfg.machine_id.is_empty());
        cfg.ensure_machine_id();
        let id = cfg.machine_id.clone();
        let b = id.as_bytes();
        assert_eq!(b.len(), 36);
        assert_eq!(&b[14], &b'4', "version nibble must be 4: {id}");
        let variant = b[19];
        assert!(
            (b'8'..=b'b').contains(&variant),
            "variant must be 8/9/a/b: {id}"
        );
        assert_eq!(&b[8], &b'-');
        assert_eq!(&b[13], &b'-');
        assert_eq!(&b[18], &b'-');
        assert_eq!(&b[23], &b'-');
        // Stable: a second call preserves the existing id.
        cfg.ensure_machine_id();
        assert_eq!(cfg.machine_id, id);
    }
}
