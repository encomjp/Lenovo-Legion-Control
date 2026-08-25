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
        if self.effect.eq_ignore_ascii_case("off") {
            return RgbEffect::Static;
        }
        RgbEffect::from_name(&self.effect).unwrap_or(RgbEffect::Static)
    }

    pub fn colors(&self) -> Vec<(u8, u8, u8)> {
        if self.effect.eq_ignore_ascii_case("off") {
            return vec![(0, 0, 0)];
        }
        let fx = self.rgb_effect();
        let bri = self.brightness.min(9) as f64 / 9.0;
        if fx.needs_color() || matches!(fx, RgbEffect::Static) {
            vec![(
                (self.r as f64 * bri).round() as u8,
                (self.g as f64 * bri).round() as u8,
                (self.b as f64 * bri).round() as u8,
            )]
        } else {
            Vec::new()
        }
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

/// Alpha telemetry settings. Nothing is collected or sent unless `enabled`
/// is explicitly turned on by the user.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiagnosticsConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Empty string = use the built-in default collector URL.
    #[serde(default)]
    pub endpoint: String,
    /// Auto-send interval in hours; 0 = manual only.
    #[serde(default)]
    pub auto_period_hours: u32,
    /// RFC3339 timestamp of the last successful send (informational).
    #[serde(default)]
    pub last_sent: Option<String>,
    /// Pseudonymous machine ID (UUID v4) generated at first opt-in. Lets
    /// the operator correlate reports from the same machine over time.
    #[serde(default)]
    pub machine_id: String,
}

impl DiagnosticsConfig {
    /// Generate a machine_id if one doesn't exist yet. Called when the user
    /// opts in so the ID is stable from the first send onward.
    pub fn ensure_machine_id(&mut self) {
        if self.machine_id.is_empty() {
            // 16 bytes from /dev/urandom, formatted as canonical UUID v4.
            let mut b = [0u8; 16];
            if let Ok(raw) = std::fs::read("/dev/urandom") {
                for (i, byte) in raw.iter().take(16).enumerate() {
                    b[i] = *byte;
                }
            }
            b[6] = (b[6] & 0x0F) | 0x40;
            b[8] = (b[8] & 0x3F) | 0x80;
            self.machine_id = format!(
                "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
                b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
                b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
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
    get().welcome_seen
}

pub fn mark_welcome_seen() {
    update(|cfg| {
        cfg.welcome_seen = true;
    });
}

fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let mut h = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
            h.push(".config");
            h
        });
    base.join("legion-control").join("settings.json")
}

fn lock_path() -> PathBuf {
    config_path().with_file_name(".settings.lock")
}

/// Run `f` while holding an exclusive advisory lock on a lockfile next to
/// settings.json. Serializes read-modify-write cycles between processes
/// (daemon, GUI, CLI) so concurrent updates cannot clobber each other.
fn with_config_lock<T>(f: impl FnOnce() -> T) -> T {
    let path = lock_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
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
            }
            let out = f();
            // SAFETY: unlocking the fd we locked above.
            unsafe {
                libc::flock(file.as_raw_fd(), libc::LOCK_UN);
            }
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
    STORE.get_or_init(|| Mutex::new(with_config_lock(load_from_disk)))
}

fn load_from_disk() -> AppConfig {
    let path = config_path();
    match fs::read_to_string(&path) {
        Ok(s) => match serde_json::from_str::<AppConfig>(&s) {
            Ok(parsed) => parsed,
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
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => AppConfig::default(),
        Err(e) => {
            log::warn!("config read failed ({}): {e}", path.display());
            AppConfig::default()
        }
    }
}

fn write_disk(cfg: &AppConfig) {
    let path = config_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
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
            Ok(()) => log::debug!("config saved → {}", path.display()),
            Err(e) => {
                log::warn!("config rename failed ({}): {e}", path.display());
                let _ = fs::remove_file(&tmp);
            }
        },
        Err(e) => {
            log::warn!("config write failed ({}): {e}", tmp.display());
            let _ = fs::remove_file(&tmp);
        }
    }
}

pub fn get() -> AppConfig {
    store().lock().map(|g| g.clone()).unwrap_or_default()
}

pub fn update(f: impl FnOnce(&mut AppConfig)) {
    let mut g = match store().lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    with_config_lock(|| {
        // Re-read the on-disk state first: another process (daemon/GUI) may
        // have written since our cache was last updated. We are already
        // holding the config lock, so this read is serialized too.
        *g = load_from_disk();
        f(&mut g);
        g.version = VERSION;
        write_disk(&g);
    });
}

pub fn config_dir_display() -> String {
    config_path()
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "~/.config/legion-control".into())
}

/// Capture current lighting + remembered power fields into a UserProfile.
pub fn snapshot_user_profile() -> UserProfile {
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
    p
}

/// Write a profile’s lighting fields into the live AppConfig.
pub fn apply_profile_to_config(p: &UserProfile) {
    update(|cfg| {
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
    let name = name.trim();
    if name.is_empty() {
        return;
    }
    let snap = snapshot_user_profile();
    update(|cfg| {
        cfg.profiles.insert(name.to_string(), snap.clone());
        cfg.active_profile = name.to_string();
        cfg.last_session = snap;
    });
}

pub fn delete_named_profile(name: &str) {
    update(|cfg| {
        cfg.profiles.remove(name);
        if cfg.active_profile == name {
            cfg.active_profile.clear();
        }
    });
}

pub fn list_profile_names() -> Vec<String> {
    let mut names: Vec<String> = get().profiles.keys().cloned().collect();
    names.sort();
    names
}

pub fn get_named_profile(name: &str) -> Option<UserProfile> {
    get().profiles.get(name).cloned()
}

pub fn remember_platform_profile(name: &str) {
    update(|cfg| {
        cfg.last_session.platform_profile = name.to_string();
    });
}

pub fn remember_ppt(id: &str, watts: u32) {
    update(|cfg| {
        cfg.last_session.ppt.insert(id.to_string(), watts);
    });
}

pub fn remember_fan(fan: u8, rpm: u32) {
    update(|cfg| match fan {
        1 => cfg.last_session.fan1 = rpm,
        2 => cfg.last_session.fan2 = rpm,
        4 => cfg.last_session.fan4 = rpm,
        _ => {}
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
    get()
}

pub fn set_brightness(level: u8) {
    update(|cfg| {
        cfg.brightness = level.min(9);
        cfg.last_session.brightness = cfg.brightness;
    });
}

pub fn set_logo_on(on: bool) {
    update(|cfg| {
        cfg.logo_on = on;
        cfg.last_session.logo_on = on;
    });
}

pub fn set_charge_limit(pct: u32) {
    update(|cfg| {
        cfg.charge_limit = pct;
        cfg.last_session.charge_limit = pct;
    });
}

pub fn set_per_key_color(key: &str, r: u8, g: u8, b: u8) {
    update(|cfg| {
        cfg.lighting_mode = "per-key".into();
        cfg.per_key.insert(key.to_string(), [r, g, b]);
        cfg.last_session.lighting_mode = "per-key".into();
        cfg.last_session.per_key = cfg.per_key.clone();
    });
}

pub fn clear_per_key() {
    update(|cfg| {
        cfg.per_key.clear();
        cfg.lighting_mode = "effects".into();
        cfg.last_session.per_key.clear();
        cfg.last_session.lighting_mode = "effects".into();
    });
}

pub fn set_ui_color(r: u8, g: u8, b: u8) {
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
}
