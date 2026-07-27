//! Persistent app settings (`~/.config/legion-control/settings.json`).

use crate::keyboard::{RgbEffect, RgbZone};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const VERSION: u32 = 3;

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

fn store() -> &'static Mutex<AppConfig> {
    static STORE: OnceLock<Mutex<AppConfig>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(load_from_disk()))
}

fn load_from_disk() -> AppConfig {
    let path = config_path();
    match fs::read_to_string(&path) {
        Ok(s) => match serde_json::from_str::<AppConfig>(&s) {
            Ok(parsed) => parsed,
            Err(e) => {
                log::warn!(
                    "config parse error ({}), using defaults: {e}",
                    path.display()
                );
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
    match serde_json::to_string_pretty(cfg) {
        Ok(s) => {
            if let Err(e) = fs::write(&path, s) {
                log::warn!("config write failed ({}): {e}", path.display());
            } else {
                log::debug!("config saved → {}", path.display());
            }
        }
        Err(e) => log::warn!("config serialize failed: {e}"),
    }
}

pub fn get() -> AppConfig {
    store().lock().map(|g| g.clone()).unwrap_or_default()
}

pub fn update(f: impl FnOnce(&mut AppConfig)) {
    if let Ok(mut g) = store().lock() {
        f(&mut g);
        g.version = VERSION;
        write_disk(&g);
    }
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

pub fn set_lighting_mode(mode: &str) {
    update(|cfg| {
        cfg.lighting_mode = mode.into();
        cfg.last_session.lighting_mode = mode.into();
    });
}

pub fn set_keyboard_layout(layout: &str) {
    update(|cfg| {
        cfg.keyboard_layout = layout.into();
        cfg.last_session.keyboard_layout = layout.into();
    });
}
