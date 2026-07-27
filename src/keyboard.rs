//! Keyboard lighting — Lenovo Legion Gen 10 Spectrum (ITE 8258, 048d:c197).
//!
//! Protocol from:
//! - [legion-spectrum-control](https://github.com/alstergee/legion-spectrum-control) (Linux)
//! - [LenovoLegionToolkit](https://github.com/BartoszCichecki/LenovoLegionToolkit) (Windows reference)
//!
//! 960-byte HID Feature Reports, report ID `0x07`. Not the old 33-byte 4-zone protocol
//! used by L5P-Keyboard-RGB / older Legion HID tools.

use std::fs::OpenOptions;
use std::io::{Error, ErrorKind, Result as IoResult};
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use std::thread;

const REPORT_SIZE: usize = 960;
const VID: &str = "048d";
const PID: &str = "c197";
/// HID usage page marker that identifies the Spectrum responder interface.
const SPECTRUM_USAGE: &[u8] = &[0x06, 0x89, 0xff];

const OP_EFFECT_CHANGE: u8 = 0xCB;
const OP_GET_EFFECT: u8 = 0xCC;
const OP_GET_BRIGHTNESS: u8 = 0xCD;
const OP_BRIGHTNESS: u8 = 0xCE;
const OP_GET_PROFILE: u8 = 0xCA;
const OP_PROFILE_CHANGE: u8 = 0xC8;
const OP_GET_LOGO: u8 = 0xA5;
const OP_LOGO: u8 = 0xA6;

/// Special keycode = all Spectrum lights (keyboard + accents). Used by LLT / spectrum-ctl `all`.
const ALL_LIGHTS: u16 = 0x0065;
const ALL_LIGHTS_KEYS: &[u16] = &[ALL_LIGHTS];

/// Legion Pro 7 Gen 10 full-spectrum keyboard keycodes (from legion-spectrum-control).
/// Verified on 83RU — see `research/SPECTRUM-ZONES.md`.
const KEYBOARD_KEYS: &[u16] = &[
    0x0001, 0x0002, 0x0003, 0x0004, 0x0005, 0x0006, 0x0007, 0x0008, 0x0009, 0x000a, 0x000b, 0x000c,
    0x000d, 0x000e, 0x000f, 0x0010, 0x0011, 0x0012, 0x0013, 0x0014, 0x0016, 0x0017, 0x0018, 0x0019,
    0x001a, 0x001b, 0x001c, 0x001d, 0x001e, 0x001f, 0x0020, 0x0021, 0x0022, 0x0026, 0x0027, 0x0028,
    0x0029, 0x0038, 0x0040, 0x0042, 0x0043, 0x0044, 0x0045, 0x0046, 0x0047, 0x0048, 0x0049, 0x004a,
    0x004b, 0x004c, 0x004d, 0x004e, 0x004f, 0x0050, 0x0051, 0x0055, 0x0058, 0x0059, 0x005a, 0x005b,
    0x005c, 0x005d, 0x005f, 0x0068, 0x006a, 0x006d, 0x006e, 0x006f, 0x0070, 0x0071, 0x0072, 0x0073,
    0x0074, 0x0075, 0x0076, 0x0077, 0x0079, 0x007b, 0x007c, 0x007f, 0x0080, 0x0082, 0x0083, 0x0087,
    0x0088, 0x008d, 0x008e, 0x0090, 0x0092, 0x0096, 0x0097, 0x0098, 0x009a, 0x009b, 0x009c, 0x009d,
    0x009f, 0x00a1, 0x00a3, 0x00a5, 0x00a7,
];

/// Rear / back accent bar (18 LEDs) — verified independent of front bar.
const REAR_KEYS: &[u16] = &[
    0x03e9, 0x03ea, 0x03eb, 0x03ec, 0x03ed, 0x03ee, 0x03ef, 0x03f0, 0x03f1, 0x03f2, 0x03f3, 0x03f4,
    0x03f5, 0x03f6, 0x03f7, 0x03f8, 0x03f9, 0x03fa,
];

/// Front bar + side accents (10 LEDs) — verified independent of rear bar.
const FRONT_KEYS: &[u16] = &[
    0x01f5, 0x01f6, 0x01f7, 0x01f8, 0x01f9, 0x01fa, 0x01fb, 0x01fc, 0x01fd, 0x01fe,
];

/// Full perimeter = rear + front (spectrum-ctl `perimeter` zone).
const PERIMETER_KEYS: &[u16] = &[
    0x03e9, 0x03ea, 0x03eb, 0x03ec, 0x03ed, 0x03ee, 0x03ef, 0x03f0, 0x03f1, 0x03f2, 0x03f3, 0x03f4,
    0x03f5, 0x03f6, 0x03f7, 0x03f8, 0x03f9, 0x03fa, 0x01f5, 0x01f6, 0x01f7, 0x01f8, 0x01f9, 0x01fa,
    0x01fb, 0x01fc, 0x01fd, 0x01fe,
];

const LOGO_KEY: u16 = 0x05dd;
const LOGO_KEYS: &[u16] = &[LOGO_KEY];

/// DE QWERTZ Spectrum keycodes (live-probed 83RU).
const DE_KEYCODES: &[(&str, u16)] = &[
    ("esc", 0x0001),
    ("f1", 0x0002),
    ("f2", 0x0003),
    ("f3", 0x0004),
    ("f4", 0x0005),
    ("f5", 0x0006),
    ("f6", 0x0007),
    ("f7", 0x0008),
    ("f8", 0x0009),
    ("f9", 0x000a),
    ("f10", 0x000b),
    ("f11", 0x000c),
    ("f12", 0x000d),
    ("prtsc", 0x000f),
    ("insert", 0x000e),
    ("delete", 0x0010),
    ("home", 0x0011),
    ("end", 0x0012),
    ("pgup", 0x0013),
    ("pgdn", 0x0014),
    ("caret", 0x0016),
    ("1", 0x0017),
    ("2", 0x0018),
    ("3", 0x0019),
    ("4", 0x001a),
    ("5", 0x001b),
    ("6", 0x001c),
    ("7", 0x001d),
    ("8", 0x001e),
    ("9", 0x001f),
    ("0", 0x0020),
    ("eszett", 0x0021),
    ("acute_grave", 0x0022),
    ("backspace", 0x0038),
    ("tab", 0x0040),
    ("q", 0x0042),
    ("w", 0x0043),
    ("e", 0x0044),
    ("r", 0x0045),
    ("t", 0x0046),
    ("z", 0x0047),
    ("u", 0x0048),
    ("i", 0x0049),
    ("o", 0x004a),
    ("p", 0x004b),
    ("ue", 0x004c),
    ("plus", 0x004d),
    ("iso_angle", 0x004e),
    ("a", 0x006d),
    ("s", 0x006e),
    ("d", 0x0058),
    ("f", 0x0059),
    ("g", 0x005a),
    ("h", 0x0071),
    ("j", 0x0072),
    ("k", 0x005b),
    ("l", 0x005c),
    ("oe", 0x005d),
    ("ae", 0x005f),
    ("caps", 0x0055),
    ("lshift", 0x006a),
    ("y", 0x0082),
    ("x", 0x0083),
    ("c", 0x006f),
    ("v", 0x0070),
    ("b", 0x0087),
    ("n", 0x0088),
    ("m", 0x0073),
    ("comma", 0x0074),
    ("period", 0x0075),
    ("minus", 0x0076),
    ("rshift", 0x008d),
    ("enter", 0x0077),
    ("fn", 0x0080),
    ("lctrl", 0x007f),
    ("copilot", 0x009b),
    ("win", 0x0096),
    ("lalt", 0x0097),
    ("space", 0x0098),
    ("altgr", 0x009a),
    ("up", 0x009d),
    ("down", 0x009f),
    ("left", 0x009c),
    ("right", 0x00a1),
    ("numlock", 0x0026),
    ("numdiv", 0x0027),
    ("nummul", 0x0028),
    ("numsub", 0x0029),
    ("numadd", 0x0090),
    ("num7", 0x004f),
    ("num8", 0x0050),
    ("num9", 0x0051),
    ("num4", 0x0079),
    ("num5", 0x007b),
    ("num6", 0x007c),
    ("num1", 0x008e),
    ("num2", 0x0068),
    ("num3", 0x0092),
    ("num0", 0x00a3),
    ("numcomma", 0x00a5),
    ("numenter", 0x00a7),
];

/// US ANSI labels → same physical LED codes as DE probes (Gen 10 Spectrum matrix).
/// Do not use upstream spectrum-ctl ANSI names blindly — they disagree with hardware.
const US_KEYCODES: &[(&str, u16)] = &[
    ("esc", 0x0001),
    ("f1", 0x0002),
    ("f2", 0x0003),
    ("f3", 0x0004),
    ("f4", 0x0005),
    ("f5", 0x0006),
    ("f6", 0x0007),
    ("f7", 0x0008),
    ("f8", 0x0009),
    ("f9", 0x000a),
    ("f10", 0x000b),
    ("f11", 0x000c),
    ("f12", 0x000d),
    ("prtsc", 0x000f),
    ("insert", 0x000e),
    ("delete", 0x0010),
    ("home", 0x0011),
    ("end", 0x0012),
    ("pgup", 0x0013),
    ("pgdn", 0x0014),
    ("tilde", 0x0016),
    ("1", 0x0017),
    ("2", 0x0018),
    ("3", 0x0019),
    ("4", 0x001a),
    ("5", 0x001b),
    ("6", 0x001c),
    ("7", 0x001d),
    ("8", 0x001e),
    ("9", 0x001f),
    ("0", 0x0020),
    ("minus", 0x0021),
    ("equals", 0x0022),
    ("backspace", 0x0038),
    ("tab", 0x0040),
    ("q", 0x0042),
    ("w", 0x0043),
    ("e", 0x0044),
    ("r", 0x0045),
    ("t", 0x0046),
    ("y", 0x0047),
    ("u", 0x0048),
    ("i", 0x0049),
    ("o", 0x004a),
    ("p", 0x004b),
    ("lbracket", 0x004c),
    ("rbracket", 0x004d),
    ("backslash", 0x004e),
    ("a", 0x006d),
    ("s", 0x006e),
    ("d", 0x0058),
    ("f", 0x0059),
    ("g", 0x005a),
    ("h", 0x0071),
    ("j", 0x0072),
    ("k", 0x005b),
    ("l", 0x005c),
    ("semicolon", 0x005d),
    ("quote", 0x005f),
    ("caps", 0x0055),
    ("lshift", 0x006a),
    ("z", 0x0082),
    ("x", 0x0083),
    ("c", 0x006f),
    ("v", 0x0070),
    ("b", 0x0087),
    ("n", 0x0088),
    ("m", 0x0073),
    ("comma", 0x0074),
    ("period", 0x0075),
    ("slash", 0x0076),
    ("rshift", 0x008d),
    ("enter", 0x0077),
    ("fn", 0x0080),
    ("lctrl", 0x007f),
    ("copilot", 0x009b),
    ("win", 0x0096),
    ("lalt", 0x0097),
    ("space", 0x0098),
    ("ralt", 0x009a),
    ("up", 0x009d),
    ("down", 0x009f),
    ("left", 0x009c),
    ("right", 0x00a1),
    ("numlock", 0x0026),
    ("numdiv", 0x0027),
    ("nummul", 0x0028),
    ("numsub", 0x0029),
    ("numadd", 0x0090),
    ("num7", 0x004f),
    ("num8", 0x0050),
    ("num9", 0x0051),
    ("num4", 0x0079),
    ("num5", 0x007b),
    ("num6", 0x007c),
    ("num1", 0x008e),
    ("num2", 0x0068),
    ("num3", 0x0092),
    ("num0", 0x00a3),
    ("numdot", 0x00a5),
    ("numenter", 0x00a7),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeyboardLayout {
    #[default]
    De,
    Us,
}

impl KeyboardLayout {
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_lowercase().as_str() {
            "de" | "de-de" | "qwertz" | "german" | "iso" => Some(Self::De),
            "us" | "en" | "en-us" | "ansi" | "qwerty" | "american" => Some(Self::Us),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::De => "de",
            Self::Us => "us",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::De => "DE QWERTZ",
            Self::Us => "US QWERTY",
        }
    }
}

/// Resolve a key name (DE or US) or `0xNNNN` hex to a Spectrum LED code.
pub fn keycode_by_name(name: &str) -> Option<u16> {
    let n = name.trim();
    if let Some(hex) = n.strip_prefix("0x").or_else(|| n.strip_prefix("0X")) {
        return u16::from_str_radix(hex, 16).ok();
    }
    DE_KEYCODES
        .iter()
        .chain(US_KEYCODES.iter())
        .find(|(k, _)| *k == n)
        .map(|(_, c)| *c)
}

pub fn layout_keycodes(layout: KeyboardLayout) -> &'static [(&'static str, u16)] {
    match layout {
        KeyboardLayout::De => DE_KEYCODES,
        KeyboardLayout::Us => US_KEYCODES,
    }
}

pub fn de_key_names() -> impl Iterator<Item = &'static str> {
    DE_KEYCODES.iter().map(|(n, _)| *n)
}

/// Stable map key for per-key colour storage (layout-independent).
pub fn color_key_for_code(code: u16) -> String {
    format!("0x{code:04x}")
}

// ioctl: HIDIOCSFEATURE(len) / HIDIOCGFEATURE(len) — same as spectrum-ctl.py
fn hid_ioc(dir: u32, nr: u32, size: usize) -> libc::c_ulong {
    let typ = u32::from(b'H');
    let req = (dir << 30) | ((size as u32) << 16) | (typ << 8) | nr;
    req as libc::c_ulong
}
fn hid_set_feature(size: usize) -> libc::c_ulong {
    hid_ioc(3, 0x06, size)
}
fn hid_get_feature(size: usize) -> libc::c_ulong {
    hid_ioc(3, 0x07, size)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RgbEffect {
    ScrewRainbow = 1,
    RainbowWave = 2,
    ColorChange = 3,
    ColorPulse = 4,
    ColorWave = 5,
    Smooth = 6,
    Rain = 7,
    Ripple = 8,
    Static = 11,
    Reactive = 12,
}

impl RgbEffect {
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_lowercase().as_str() {
            "screw-rainbow" | "screw" | "spiral" => Some(Self::ScrewRainbow),
            "rainbow-wave" | "rainbow" => Some(Self::RainbowWave),
            "color-change" | "change" => Some(Self::ColorChange),
            "color-pulse" | "pulse" | "breath" | "breathing" => Some(Self::ColorPulse),
            "color-wave" | "wave" => Some(Self::ColorWave),
            "smooth" | "spectrum" | "hue" => Some(Self::Smooth),
            "rain" => Some(Self::Rain),
            "ripple" => Some(Self::Ripple),
            "static" => Some(Self::Static),
            "reactive" | "type" => Some(Self::Reactive),
            "off" => None,
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::ScrewRainbow => "screw-rainbow",
            Self::RainbowWave => "rainbow-wave",
            Self::ColorChange => "color-change",
            Self::ColorPulse => "color-pulse",
            Self::ColorWave => "color-wave",
            Self::Smooth => "smooth",
            Self::Rain => "rain",
            Self::Ripple => "ripple",
            Self::Static => "static",
            Self::Reactive => "reactive",
        }
    }

    pub fn all_names() -> &'static [&'static str] {
        &[
            "static",
            "color-pulse",
            "color-wave",
            "rainbow-wave",
            "screw-rainbow",
            "smooth",
            "color-change",
            "rain",
            "ripple",
            "reactive",
        ]
    }

    pub fn needs_color(self) -> bool {
        matches!(
            self,
            Self::Static
                | Self::ColorPulse
                | Self::ColorWave
                | Self::Reactive
                | Self::Ripple
                | Self::Rain
        )
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Static => "Static",
            Self::ColorPulse => "Pulse",
            Self::ColorWave => "Wave",
            Self::RainbowWave => "Rainbow",
            Self::ScrewRainbow => "Spiral",
            Self::Smooth => "Smooth",
            Self::ColorChange => "Color change",
            Self::Rain => "Rain",
            Self::Ripple => "Ripple",
            Self::Reactive => "Reactive",
        }
    }
}

/// Target surface for Spectrum effects (verified on 83RU).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RgbZone {
    /// Keyboard + front + rear + logo via special key `0x0065`.
    #[default]
    All,
    Keyboard,
    Front,
    Rear,
    Logo,
    /// Front + rear bars (no keyboard / logo).
    Chassis,
}

impl RgbZone {
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_lowercase().as_str() {
            "all" => Some(Self::All),
            "keyboard" | "keys" | "kb" => Some(Self::Keyboard),
            "front" => Some(Self::Front),
            "rear" | "back" => Some(Self::Rear),
            "logo" => Some(Self::Logo),
            "chassis" | "bars" | "perimeter" => Some(Self::Chassis),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Keyboard => "keyboard",
            Self::Front => "front",
            Self::Rear => "rear",
            Self::Logo => "logo",
            Self::Chassis => "chassis",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::All => "Everything",
            Self::Keyboard => "Keyboard",
            Self::Front => "Front bar",
            Self::Rear => "Rear bar",
            Self::Logo => "Lid logo",
            Self::Chassis => "Chassis bars",
        }
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::All,
            Self::Keyboard,
            Self::Chassis,
            Self::Front,
            Self::Rear,
            Self::Logo,
        ]
    }

    pub fn keys(self) -> &'static [u16] {
        match self {
            Self::All => ALL_LIGHTS_KEYS,
            Self::Keyboard => KEYBOARD_KEYS,
            Self::Front => FRONT_KEYS,
            Self::Rear => REAR_KEYS,
            Self::Logo => LOGO_KEYS,
            Self::Chassis => PERIMETER_KEYS,
        }
    }
}

fn find_spectrum_hidraw() -> Option<PathBuf> {
    let entries = std::fs::read_dir("/sys/class/hidraw").ok()?;
    let mut fallback = None;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let mut cur = std::fs::canonicalize(entry.path().join("device")).ok()?;
        let mut matched = false;
        for _ in 0..10 {
            let v = cur.join("idVendor");
            let p = cur.join("idProduct");
            if v.exists() && p.exists() {
                let vendor = std::fs::read_to_string(&v).ok()?.trim().to_lowercase();
                let product = std::fs::read_to_string(&p).ok()?.trim().to_lowercase();
                matched = vendor == VID && product == PID;
                break;
            }
            if !cur.pop() {
                break;
            }
        }
        if !matched {
            continue;
        }
        let path = PathBuf::from(format!("/dev/{name}"));
        let desc_path = entry.path().join("device/report_descriptor");
        if let Ok(desc) = std::fs::read(&desc_path) {
            if desc
                .windows(SPECTRUM_USAGE.len())
                .any(|w| w == SPECTRUM_USAGE)
            {
                return Some(path);
            }
        }
        fallback = Some(path);
    }
    fallback
}

struct SpectrumDevice {
    file: std::fs::File,
}

fn hid_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

impl SpectrumDevice {
    fn open() -> Result<Self, String> {
        let path = find_spectrum_hidraw().ok_or_else(|| {
            "Spectrum HID (048d:c197) not found — is udev rule installed?".to_string()
        })?;
        // Blocking open — matches spectrum-ctl.py (no O_NONBLOCK).
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| {
                let msg = format!("Cannot open {}: {e}", path.display());
                log::warn!("spectrum: {msg}");
                msg
            })?;
        log::debug!("spectrum: opened {}", path.display());
        Ok(Self { file })
    }

    fn set_feature(&self, data: &[u8]) -> Result<(), String> {
        let mut buf = [0u8; REPORT_SIZE];
        let n = data.len().min(REPORT_SIZE);
        buf[..n].copy_from_slice(&data[..n]);
        // SAFETY: ioctl on a valid fd (self.file is owned/opened) with HID_SET_FEATURE
        // is a standard kernel interface. buf is stack-allocated and as_mut_ptr() points
        // to valid memory of REPORT_SIZE bytes.
        let ret = unsafe {
            libc::ioctl(
                self.file.as_raw_fd(),
                hid_set_feature(REPORT_SIZE),
                buf.as_mut_ptr(),
            )
        };
        if ret < 0 {
            let msg = format!("HIDIOCSFEATURE failed: {}", Error::last_os_error());
            log::warn!("spectrum: {msg}");
            return Err(msg);
        }
        Ok(())
    }

    fn get_feature(&self) -> Result<[u8; REPORT_SIZE], String> {
        let mut buf = [0u8; REPORT_SIZE];
        buf[0] = 0x07;
        // SAFETY: same ioctl on the same valid fd as set_feature; buf is stack-
        // allocated REPORT_SIZE bytes and the kernel writes exactly that many.
        let ret = unsafe {
            libc::ioctl(
                self.file.as_raw_fd(),
                hid_get_feature(REPORT_SIZE),
                buf.as_mut_ptr(),
            )
        };
        if ret < 0 {
            return Err(format!("HIDIOCGFEATURE failed: {}", Error::last_os_error()));
        }
        Ok(buf)
    }

    fn request(&self, op: u8, payload: &[u8]) -> Result<(), String> {
        // spectrum-ctl uses fixed size 0xC0 for all ops.
        let mut data = vec![0x07, op, 0xC0, 0x03];
        data.extend_from_slice(payload);
        self.set_feature(&data)
    }
}

fn build_effect(
    effect_no: u8,
    effect_type: u8,
    colors: &[(u8, u8, u8)],
    keycodes: &[u16],
    speed: u8,
    direction: u8,
    clockwise: u8,
) -> Vec<u8> {
    // spectrum-ctl color_mode: list if colors, else random (non-static) / none (static)
    let color_mode: u8 = if !colors.is_empty() {
        0x02
    } else if effect_type != 11 {
        0x01
    } else {
        0x00
    };

    let mut data = Vec::new();
    data.push(effect_no);
    data.extend_from_slice(&[
        0x06,
        0x01,
        effect_type,
        0x02,
        speed,
        0x03,
        clockwise,
        0x04,
        direction,
        0x05,
        color_mode,
        0x06,
        0x00,
    ]);
    data.push(colors.len() as u8);
    for (r, g, b) in colors {
        data.extend_from_slice(&[*r, *g, *b]);
    }
    data.push(keycodes.len() as u8);
    for kc in keycodes {
        data.extend_from_slice(&kc.to_le_bytes());
    }
    data
}

fn get_profile_raw(dev: &SpectrumDevice) -> Result<u8, String> {
    dev.request(OP_GET_PROFILE, &[])?;
    Ok(dev.get_feature()?[4].min(6))
}

fn set_profile_raw(dev: &SpectrumDevice, profile: u8) -> Result<(), String> {
    dev.request(OP_PROFILE_CHANGE, &[profile.min(6)])
}

fn get_brightness_raw(dev: &SpectrumDevice) -> Result<u8, String> {
    dev.request(OP_GET_BRIGHTNESS, &[])?;
    Ok(dev.get_feature()?[4])
}

fn set_brightness_raw(dev: &SpectrumDevice, level: u8) -> Result<(), String> {
    dev.request(OP_BRIGHTNESS, &[level.min(9)])
}

/// Active Spectrum lighting profile (0–6).
pub fn rgb_profile() -> Option<u8> {
    let _guard = hid_lock().lock().ok()?;
    let dev = SpectrumDevice::open().ok()?;
    get_profile_raw(&dev).ok()
}

pub fn set_rgb_profile(profile: u8) -> Result<(), String> {
    let _guard = hid_lock()
        .lock()
        .map_err(|_| "Spectrum HID lock poisoned".to_string())?;
    let dev = SpectrumDevice::open()?;
    set_profile_raw(&dev, profile)
}

fn send_effects(dev: &SpectrumDevice, profile: u8, effects: &[Vec<u8>]) -> Result<(), String> {
    let mut payload = vec![profile, 0x01, 0x01];
    for e in effects {
        payload.extend_from_slice(e);
    }
    let mut packet = vec![0x07, OP_EFFECT_CHANGE, 0xC0, 0x03];
    packet.extend_from_slice(&payload);
    // Match spectrum-ctl exactly — no profile hop after write.
    dev.set_feature(&packet)
}

fn zone_blob(effect_no: u8, layer: &crate::config::ZoneEffect, keys: &[u16]) -> Vec<u8> {
    let effect = if layer.is_off() {
        RgbEffect::Static
    } else {
        layer.rgb_effect()
    };
    let colors = layer.colors();
    let speed = layer.speed.clamp(1, 3);
    let clockwise = u8::from(matches!(effect, RgbEffect::ScrewRainbow));
    build_effect(effect_no, effect as u8, &colors, keys, speed, 0, clockwise)
}

/// Build a full multi-effect packet from persisted zone (and optional per-key) state.
/// Zones stay independent — changing front does not black out the keyboard.
fn apply_config_on_dev(dev: &SpectrumDevice, cfg: &crate::config::AppConfig) -> Result<(), String> {
    if get_brightness_raw(dev)? < 1 {
        let level = cfg.brightness.max(1);
        set_brightness_raw(dev, level)?;
    }
    let profile = get_profile_raw(dev)?;
    let mut blobs = Vec::new();
    let mut n = 1u8;

    let push = |blobs: &mut Vec<Vec<u8>>, n: &mut u8, blob: Vec<u8>| {
        blobs.push(blob);
        *n = (*n).saturating_add(1);
    };

    if cfg.lighting_mode == "per-key" && !cfg.per_key.is_empty() {
        // Group painted keys by colour → one static effect per colour.
        let mut by_color: std::collections::BTreeMap<(u8, u8, u8), Vec<u16>> =
            std::collections::BTreeMap::new();
        for (name, rgb) in &cfg.per_key {
            if let Some(code) = keycode_by_name(name) {
                by_color
                    .entry((rgb[0], rgb[1], rgb[2]))
                    .or_default()
                    .push(code);
            }
        }
        // Unpainted keyboard keys stay dark so the map is readable.
        let painted: std::collections::HashSet<u16> =
            by_color.values().flat_map(|v| v.iter().copied()).collect();
        let rest: Vec<u16> = KEYBOARD_KEYS
            .iter()
            .copied()
            .filter(|c| !painted.contains(c))
            .collect();
        if !rest.is_empty() {
            let blob = build_effect(n, 11, &[(0, 0, 0)], &rest, 2, 0, 0);
            push(&mut blobs, &mut n, blob);
        }
        for ((r, g, b), keys) in by_color {
            if keys.is_empty() {
                continue;
            }
            let blob = build_effect(n, 11, &[(r, g, b)], &keys, 2, 0, 0);
            push(&mut blobs, &mut n, blob);
        }
    } else {
        let blob = zone_blob(n, &cfg.keyboard, KEYBOARD_KEYS);
        push(&mut blobs, &mut n, blob);
    }

    let blob = zone_blob(n, &cfg.front, FRONT_KEYS);
    push(&mut blobs, &mut n, blob);
    let blob = zone_blob(n, &cfg.rear, REAR_KEYS);
    push(&mut blobs, &mut n, blob);
    let blob = zone_blob(n, &cfg.logo, LOGO_KEYS);
    push(&mut blobs, &mut n, blob);

    send_effects(dev, profile, &blobs)
}

#[allow(clippy::too_many_arguments)]
fn apply_effect_on_dev(
    dev: &SpectrumDevice,
    effect: RgbEffect,
    r: u8,
    g: u8,
    b: u8,
    speed: u8,
    brightness: u8,
    zone: RgbZone,
) -> Result<(), String> {
    let name = if matches!(effect, RgbEffect::Static) && r == 0 && g == 0 && b == 0 {
        "off"
    } else {
        effect.name()
    };
    let cfg = crate::config::apply_zone_update(zone, name, r, g, b, speed, brightness);
    apply_config_on_dev(dev, &cfg)
}

// ─── Serial HID worker (coalescing queue) ───────────────────────────────────
//
// HID feature reports must not run on the GTK main thread, and concurrent
// opens of the same hidraw hang the UI when the user spams controls. One
// background thread owns the device; spam keeps only the latest command.

enum SpectrumJob {
    Effect {
        effect: RgbEffect,
        r: u8,
        g: u8,
        b: u8,
        speed: u8,
        brightness: u8,
        zone: RgbZone,
        done: Option<Sender<Result<(), String>>>,
    },
    /// Re-apply persisted config (zones / per-key) without changing it.
    Restore {
        done: Option<Sender<Result<(), String>>>,
    },
    Brightness {
        level: u8,
        done: Option<Sender<Result<(), String>>>,
    },
    Logo {
        on: bool,
        done: Option<Sender<Result<(), String>>>,
    },
}

fn spectrum_tx() -> &'static Sender<SpectrumJob> {
    static TX: OnceLock<Sender<SpectrumJob>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        match thread::Builder::new()
            .name("spectrum-hid".into())
            .spawn(move || spectrum_worker(rx))
        {
            Ok(_) => {}
            Err(e) => log::error!("failed to start spectrum HID worker: {e}"),
        }
        tx
    })
}

fn spectrum_worker(rx: Receiver<SpectrumJob>) {
    use std::collections::HashMap;

    while let Ok(first) = rx.recv() {
        // Per-zone coalescing: rapid Front+Keyboard tweaks must not drop one zone.
        let mut effects: HashMap<RgbZone, (RgbEffect, u8, u8, u8, u8, u8)> = HashMap::new();
        let mut restore = false;
        let mut brightness: Option<u8> = None;
        let mut logo: Option<bool> = None;
        let mut waiters: Vec<Sender<Result<(), String>>> = Vec::new();

        let mut absorb = |job: SpectrumJob| match job {
            SpectrumJob::Effect {
                effect: e,
                r,
                g,
                b,
                speed,
                brightness: bri,
                zone,
                done,
            } => {
                restore = false;
                if zone == RgbZone::All {
                    effects.clear();
                    effects.insert(zone, (e, r, g, b, speed, bri));
                } else {
                    effects.remove(&RgbZone::All);
                    effects.insert(zone, (e, r, g, b, speed, bri));
                }
                if let Some(d) = done {
                    waiters.push(d);
                }
            }
            SpectrumJob::Restore { done } => {
                restore = true;
                effects.clear();
                if let Some(d) = done {
                    waiters.push(d);
                }
            }
            SpectrumJob::Brightness { level, done } => {
                brightness = Some(level);
                if let Some(d) = done {
                    waiters.push(d);
                }
            }
            SpectrumJob::Logo { on, done } => {
                logo = Some(on);
                if let Some(d) = done {
                    waiters.push(d);
                }
            }
        };

        absorb(first);
        while let Ok(job) = rx.try_recv() {
            absorb(job);
        }

        let result = (|| {
            let _guard = hid_lock()
                .lock()
                .map_err(|_| "Spectrum HID lock poisoned".to_string())?;
            let dev = SpectrumDevice::open()?;
            // If an effect/static is queued in the same batch, ignore brightness 0 —
            // the GTK scale starts at 0 and used to race with color applies, leaving
            // the keyboard dark even though colours were written.
            if let Some(level) = brightness {
                if !(!effects.is_empty() && level == 0) {
                    set_brightness_raw(&dev, level)?;
                    crate::config::set_brightness(level);
                }
            }
            if restore {
                let cfg = crate::config::get();
                apply_config_on_dev(&dev, &cfg)?;
            } else {
                // Stable order: All first (if any), then named zones.
                let mut zones: Vec<_> = effects.into_iter().collect();
                zones.sort_by_key(|(z, _)| match z {
                    RgbZone::All => 0,
                    RgbZone::Keyboard => 1,
                    RgbZone::Front => 2,
                    RgbZone::Rear => 3,
                    RgbZone::Chassis => 4,
                    RgbZone::Logo => 5,
                });
                for (zone, (e, r, g, b, speed, brightness)) in zones {
                    apply_effect_on_dev(&dev, e, r, g, b, speed, brightness, zone)?;
                }
            }
            if let Some(on) = logo {
                dev.request(OP_LOGO, &[u8::from(on)])?;
                crate::config::set_logo_on(on);
            }
            Ok(())
        })();

        for w in waiters {
            let _ = w.send(result.clone());
        }
    }
}

fn enqueue(job: SpectrumJob, wait: bool) -> Result<(), String> {
    if wait {
        let (done_tx, done_rx) = mpsc::channel();
        let job = match job {
            SpectrumJob::Effect {
                effect,
                r,
                g,
                b,
                speed,
                brightness,
                zone,
                ..
            } => SpectrumJob::Effect {
                effect,
                r,
                g,
                b,
                speed,
                brightness,
                zone,
                done: Some(done_tx),
            },
            SpectrumJob::Restore { .. } => SpectrumJob::Restore {
                done: Some(done_tx),
            },
            SpectrumJob::Brightness { level, .. } => SpectrumJob::Brightness {
                level,
                done: Some(done_tx),
            },
            SpectrumJob::Logo { on, .. } => SpectrumJob::Logo {
                on,
                done: Some(done_tx),
            },
        };
        spectrum_tx()
            .send(job)
            .map_err(|_| "Spectrum HID worker died".to_string())?;
        done_rx
            .recv()
            .map_err(|_| "Spectrum HID worker died".to_string())?
    } else {
        spectrum_tx()
            .send(job)
            .map_err(|_| "Spectrum HID worker died".to_string())
    }
}

/// Apply effect and wait until the HID worker finishes (CLI).
pub fn set_rgb_effect(effect: RgbEffect, r: u8, g: u8, b: u8, speed: u8) -> Result<(), String> {
    set_rgb_effect_zone(effect, r, g, b, speed, 9, RgbZone::All)
}

pub fn set_rgb_effect_zone(
    effect: RgbEffect,
    r: u8,
    g: u8,
    b: u8,
    speed: u8,
    brightness: u8,
    zone: RgbZone,
) -> Result<(), String> {
    enqueue(
        SpectrumJob::Effect {
            effect,
            r,
            g,
            b,
            speed,
            brightness,
            zone,
            done: None,
        },
        true,
    )
}

/// Queue effect without blocking the UI — spam keeps only the latest job.
pub fn set_rgb_effect_async(effect: RgbEffect, r: u8, g: u8, b: u8, speed: u8) {
    set_rgb_effect_zone_async(effect, r, g, b, speed, 9, RgbZone::All);
}

pub fn set_rgb_effect_zone_async(
    effect: RgbEffect,
    r: u8,
    g: u8,
    b: u8,
    speed: u8,
    brightness: u8,
    zone: RgbZone,
) {
    let _ = enqueue(
        SpectrumJob::Effect {
            effect,
            r,
            g,
            b,
            speed,
            brightness,
            zone,
            done: None,
        },
        false,
    );
}

/// Re-apply `~/.config/legion-control/settings.json` lighting to hardware.
pub fn restore_lighting_async() {
    let _ = enqueue(SpectrumJob::Restore { done: None }, false);
}

pub fn restore_lighting() -> Result<(), String> {
    enqueue(SpectrumJob::Restore { done: None }, true)
}

/// Paint one DE-layout key and refresh the full layer set (keeps bars/logo).
pub fn paint_key_async(key: &str, r: u8, g: u8, b: u8) {
    crate::config::set_per_key_color(key, r, g, b);
    restore_lighting_async();
}

pub fn clear_per_key_async() {
    crate::config::clear_per_key();
    restore_lighting_async();
}

/// Soft-reset Spectrum when lighting is stuck/glitched: max brightness, clear
/// per-key paints, static white, logo on, then re-apply saved config.
pub fn troubleshoot_lighting() -> Result<Vec<String>, String> {
    let mut steps = Vec::new();
    set_rgb_brightness(9)?;
    steps.push("Spectrum brightness → 9".into());
    crate::config::clear_per_key();
    steps.push("Cleared per-key colour map".into());
    set_rgb_static(255, 255, 255)?;
    steps.push("Static white on all zones".into());
    match set_logo(true) {
        Ok(()) => steps.push("Logo LED on".into()),
        Err(e) => steps.push(format!("Logo LED skipped: {e}")),
    }
    restore_lighting()?;
    steps.push("Re-applied saved lighting config".into());
    Ok(steps)
}

/// Static color — Spectrum effect 11.
pub fn set_rgb_static(r: u8, g: u8, b: u8) -> Result<(), String> {
    set_rgb_effect(RgbEffect::Static, r, g, b, 2)
}

pub fn set_rgb_static_async(r: u8, g: u8, b: u8) {
    set_rgb_effect_async(RgbEffect::Static, r, g, b, 2);
}

/// Turn Spectrum lighting off (brightness 0).
pub fn set_rgb_off() -> Result<(), String> {
    set_rgb_brightness(0)
}

pub fn set_rgb_off_async() {
    set_rgb_brightness_async(0);
}

/// Spectrum brightness 0–9 (reads still hit the device directly; rare/fast).
pub fn rgb_brightness() -> Option<u8> {
    let _guard = hid_lock().lock().ok()?;
    let dev = SpectrumDevice::open().ok()?;
    get_brightness_raw(&dev).ok()
}

pub fn set_rgb_brightness(level: u8) -> Result<(), String> {
    enqueue(SpectrumJob::Brightness { level, done: None }, true)
}

pub fn set_rgb_brightness_async(level: u8) {
    let _ = enqueue(SpectrumJob::Brightness { level, done: None }, false);
}

pub fn logo_on() -> Option<bool> {
    let _guard = hid_lock().lock().ok()?;
    let dev = SpectrumDevice::open().ok()?;
    dev.request(OP_GET_LOGO, &[]).ok()?;
    Some(dev.get_feature().ok()?[4] == 1)
}

pub fn set_logo(on: bool) -> Result<(), String> {
    enqueue(SpectrumJob::Logo { on, done: None }, true)
}

pub fn set_logo_async(on: bool) {
    let _ = enqueue(SpectrumJob::Logo { on, done: None }, false);
}

/// Read back RGB of the first effect colour (debug / GUI toast).
pub fn peek_effect_rgb() -> Option<(u8, u8, u8)> {
    let _guard = hid_lock().lock().ok()?;
    let dev = SpectrumDevice::open().ok()?;
    let profile = get_profile_raw(&dev).ok()?;
    dev.request(OP_GET_EFFECT, &[profile]).ok()?;
    let resp = dev.get_feature().ok()?;
    let ncolors = *resp.get(21)?;
    if ncolors == 0 {
        return None;
    }
    Some((resp[22], resp[23], resp[24]))
}

// ─── White backlight (WMI LED — rare on Gen 10 RGB models) ───

const KBD_LED_CANDIDATES: &[&str] = &[
    "/sys/class/leds/platform::kbd_backlight",
    "/sys/class/leds/tpacpi::kbd_backlight",
];

fn find_kbd_led() -> Option<PathBuf> {
    for p in KBD_LED_CANDIDATES {
        let path = PathBuf::from(p);
        if path.join("brightness").exists() {
            return Some(path);
        }
    }
    None
}

pub fn brightness() -> Option<u8> {
    let led = find_kbd_led()?;
    std::fs::read_to_string(led.join("brightness"))
        .ok()?
        .trim()
        .parse()
        .ok()
}

pub fn set_brightness(level: u8) -> IoResult<()> {
    let led = find_kbd_led().ok_or_else(|| {
        Error::new(
            ErrorKind::NotFound,
            "No white keyboard backlight LED (RGB Spectrum only on this model)",
        )
    })?;
    let max = max_brightness().unwrap_or(2);
    std::fs::write(led.join("brightness"), format!("{}", level.min(max)))
}

pub fn max_brightness() -> Option<u8> {
    let led = find_kbd_led()?;
    std::fs::read_to_string(led.join("max_brightness"))
        .ok()?
        .trim()
        .parse()
        .ok()
}

pub fn has_white_backlight() -> bool {
    find_kbd_led().is_some()
}

/// Camera privacy kill-switch (ideapad).
pub fn camera_power() -> Option<bool> {
    let known = "/sys/devices/pci0000:00/0000:00:14.3/PNP0C09:00/VPC2004:00/camera_power";
    if let Ok(val) = std::fs::read_to_string(known) {
        return Some(val.trim() == "1");
    }
    None
}
