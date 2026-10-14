//! Legacy 4-zone RGB keyboards (ITE 048d:c955–c995): Legion 5 / 5 Pro
//! 2020–2024, IdeaPad Gaming 3, LOQ 2023–2024.
//!
//! EXPERIMENTAL — the packet layout is verified against four independent
//! reference implementations, not on hardware by this project:
//!
//! - 4JX/L5P-Keyboard-RGB `driver/src/lib.rs` (`build_payload`):
//!   `payload[0]=0xcc; payload[1]=0x16; [2]=effect; [3]=speed; [4]=brightness;
//!   [5..17]=rgb (Static/Breath only); [18]=1 for RightWave; [19]=1 for LeftWave`,
//!   33-byte buffer, `SPEED_RANGE 1..=4`, `BRIGHTNESS_RANGE 1..=2`, devices
//!   matched by usage page 0xff89 / usage 0x00cc.
//! - imShara/l5p-kbl `l5p_kbl.py`: `[204, 22, effect, speed, brightness] +
//!   12 colour bytes (zeros unless static/breath) + [0] + direction pair +
//!   13 zeros`; EFFECT static 1, breath 3, wave 4, hue 6; sent as a feature
//!   report (`wValue=0x03CC`); "off" = `[204, 22, 1] + 30 zeros`.
//! - LenovoLegionToolkit `Native.cs` `LENOVO_RGB_KEYBOARD_STATE`
//!   (Pack=1): Header[2], Effect, Speed, Brightness, Zone1..4Rgb[3], Padding,
//!   WaveLTR, WaveRTL, Unused[13] → byte 18 = WaveLTR, byte 19 = WaveRTL.
//! - OpenRGB `LenovoDevices4Zone.h` `KeyboardState`: `{0xCC,0x16}, effect,
//!   speed, brightness, zone0..3_rgb[3], wave_ltr, wave_rtl`, enums static 1 /
//!   breath 3 / wave 4 / smooth 6, brightness 1..2, speed 1..4.
//!
//! Wave direction naming: LLT and OpenRGB name byte 18 "LTR" and 4JX sets it
//! for `RightWave` (the wave travels to the right). We follow that majority:
//! [`WaveDir::Right`] = byte 18. l5p-kbl labels the same byte "rtl"; if the
//! direction turns out reversed on hardware only this mapping changes.
//!
//! Safety: a node is written only when its report descriptor positively
//! declares a 33-byte feature report with ID 0xCC on usage page 0xFF89 —
//! never by PID alone. 048d:c193 ("Lenovo Lighting") is never a target.

use std::fs::OpenOptions;
use std::io::Error;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};

/// Feature report length including the report-ID byte.
pub const REPORT_LEN: usize = 33;
/// Report ID (byte 0) and command (byte 1).
const REPORT_ID: u8 = 0xCC;
const CMD_SET: u8 = 0x16;
/// Vendor usage page declared by the 4-zone controller.
const USAGE_PAGE: u32 = 0xFF89;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FzEffect {
    Static,
    Breath,
    Wave,
    Smooth,
}

impl FzEffect {
    pub const ALL: [FzEffect; 4] = [Self::Static, Self::Breath, Self::Wave, Self::Smooth];

    /// Firmware effect code (byte 2).
    pub fn code(self) -> u8 {
        match self {
            Self::Static => 0x01,
            Self::Breath => 0x03,
            Self::Wave => 0x04,
            Self::Smooth => 0x06,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::Breath => "breath",
            Self::Wave => "wave",
            Self::Smooth => "smooth",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Static => "Static",
            Self::Breath => "Breath",
            Self::Wave => "Wave",
            Self::Smooth => "Smooth",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "static" => Some(Self::Static),
            "breath" | "breathing" => Some(Self::Breath),
            "wave" | "rainbow-wave" => Some(Self::Wave),
            "smooth" | "hue" => Some(Self::Smooth),
            _ => None,
        }
    }

    /// Only Static and Breath carry per-zone colours (Wave/Smooth are
    /// firmware rainbows; every reference zeroes or ignores the colours).
    pub fn uses_colors(self) -> bool {
        matches!(self, Self::Static | Self::Breath)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaveDir {
    /// Wave travels left → right: byte 18 (LLT `WaveLTR`, OpenRGB `wave_ltr`,
    /// 4JX `RightWave`).
    Right,
    /// Wave travels right → left: byte 19 (LLT `WaveRTL`, 4JX `LeftWave`).
    Left,
}

impl WaveDir {
    pub fn name(self) -> &'static str {
        match self {
            Self::Right => "right",
            Self::Left => "left",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "right" | "ltr" => Some(Self::Right),
            "left" | "rtl" => Some(Self::Left),
            _ => None,
        }
    }
}

/// Complete 4-zone keyboard state. `brightness == 0` means off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FourZoneState {
    pub effect: FzEffect,
    /// 1 (slowest) ..= 4 (fastest).
    pub speed: u8,
    /// 0 = off, 1 = low, 2 = high.
    pub brightness: u8,
    /// Zones left → right.
    pub colors: [[u8; 3]; 4],
    pub direction: WaveDir,
}

impl Default for FourZoneState {
    fn default() -> Self {
        Self {
            effect: FzEffect::Static,
            speed: 1,
            brightness: 2,
            colors: [[200, 16, 46]; 4],
            direction: WaveDir::Right,
        }
    }
}

/// Build the 33-byte feature report (byte 0 = report ID 0xCC).
///
/// Off (`brightness == 0`) follows l5p-kbl exactly: `CC 16 01` + 30 zeros.
/// Otherwise follows 4JX `build_payload` byte-for-byte.
pub fn build_report(s: &FourZoneState) -> [u8; REPORT_LEN] {
    let mut p = [0u8; REPORT_LEN];
    p[0] = REPORT_ID;
    p[1] = CMD_SET;
    if s.brightness == 0 {
        p[2] = FzEffect::Static.code();
        return p;
    }
    p[2] = s.effect.code();
    p[3] = s.speed.clamp(1, 4);
    p[4] = s.brightness.clamp(1, 2);
    if s.effect.uses_colors() {
        for (i, c) in s.colors.iter().enumerate() {
            p[5 + i * 3..8 + i * 3].copy_from_slice(c);
        }
    }
    // p[17] = padding (0).
    if s.effect == FzEffect::Wave {
        match s.direction {
            WaveDir::Right => p[18] = 1,
            WaveDir::Left => p[19] = 1,
        }
    }
    p
}

/// Parse `#RRGGBB` / `RRGGBB` (ASCII only).
pub fn parse_color(s: &str) -> Option<[u8; 3]> {
    let t = s.trim().trim_start_matches('#');
    if !t.is_ascii() || t.len() != 6 {
        return None;
    }
    Some([
        u8::from_str_radix(&t[0..2], 16).ok()?,
        u8::from_str_radix(&t[2..4], 16).ok()?,
        u8::from_str_radix(&t[4..6], 16).ok()?,
    ])
}

/// Feature reports declared in a HID report descriptor, as
/// `(usage_page, report_id, total_len_including_id_byte)`.
fn feature_reports(desc: &[u8]) -> Vec<(u32, u8, usize)> {
    let (mut page, mut size, mut count, mut id) = (0u32, 0usize, 0usize, 0u8);
    // (page, id, bits)
    let mut out: Vec<(u32, u8, usize)> = Vec::new();
    let mut i = 0;
    while i < desc.len() {
        let prefix = desc[i];
        if prefix == 0xFE {
            i += 3 + desc.get(i + 1).copied().unwrap_or(0) as usize;
            continue;
        }
        let n = match prefix & 3 {
            3 => 4,
            n => n as usize,
        };
        let Some(data) = desc.get(i + 1..i + 1 + n) else {
            break;
        };
        let val = data
            .iter()
            .enumerate()
            .fold(0usize, |a, (k, b)| a | (*b as usize) << (8 * k));
        match (prefix >> 2 & 3, prefix >> 4) {
            (1, 0x0) => page = val as u32,
            (1, 0x7) => size = val,
            (1, 0x9) => count = val,
            (1, 0x8) => id = val as u8,
            (0, 0xB) => match out.iter_mut().find(|(_, k, _)| *k == id) {
                Some((_, _, b)) => *b += size * count,
                None => out.push((page, id, size * count)),
            },
            _ => {}
        }
        i += 1 + n;
    }
    out.into_iter()
        .map(|(p, id, bits)| (p, id, bits.div_ceil(8) + 1))
        .collect()
}

/// True only when the descriptor declares the 4-zone protocol: a feature
/// report with ID 0xCC, 33 bytes long (incl. ID), on usage page 0xFF89.
pub fn is_fourzone_descriptor(desc: &[u8]) -> bool {
    feature_reports(desc)
        .iter()
        .any(|&(page, id, len)| page == USAGE_PAGE && id == REPORT_ID && len == REPORT_LEN)
}

/// Decide whether one hidraw node may be written. Pure for tests.
fn node_is_target(vendor: &str, pid: &str, desc: Option<&[u8]>) -> bool {
    let pid = pid.trim().to_ascii_lowercase();
    if !vendor.trim().eq_ignore_ascii_case("048d") || pid == "c193" {
        return false;
    }
    let Some(desc) = desc else {
        return false; // never by PID alone
    };
    crate::device::classify_ite(&pid, Some(desc)) == Some(crate::device::IteClass::FourZone)
        && is_fourzone_descriptor(desc)
}

/// Find the 4-zone hidraw node, if any (cheap sysfs scan, no writes).
pub fn find_hidraw() -> Option<PathBuf> {
    let entries = std::fs::read_dir("/sys/class/hidraw").ok()?;
    let mut nodes: Vec<_> = entries.flatten().collect();
    nodes.sort_by_key(|e| e.file_name());
    for entry in nodes {
        let name = entry.file_name().to_string_lossy().to_string();
        let Ok(dev) = std::fs::canonicalize(entry.path().join("device")) else {
            continue;
        };
        // HID_ID dirname: 0003:048D:C965.0003
        let id = dev
            .file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let parts: Vec<&str> = id.split(':').collect();
        if parts.len() < 3 || parts[0] != "0003" {
            continue;
        }
        let pid = parts[2].split('.').next().unwrap_or("");
        let desc = std::fs::read(dev.join("report_descriptor")).ok();
        if node_is_target(parts[1], pid, desc.as_deref()) {
            log::debug!("fourzone: {name} 048d:{pid} descriptor confirms 4-zone (33 B, 0xCC)");
            return Some(PathBuf::from(format!("/dev/{name}")));
        }
    }
    None
}

pub fn present() -> bool {
    find_hidraw().is_some()
}

/// Serializes every HIDIOCSFEATURE to the 4-zone controller in this process.
fn hid_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn write_report(report: &[u8; REPORT_LEN]) -> Result<(), String> {
    if !crate::device::vendor_supported() {
        return Err("unsupported vendor: 4-zone lighting is Lenovo-only".into());
    }
    let _g = hid_lock().lock().unwrap_or_else(|p| p.into_inner());
    let path = find_hidraw().ok_or_else(|| {
        "4-zone RGB controller not found (no 048d hidraw node with a 33-byte 0xCC feature report) — is the udev rule installed?".to_string()
    })?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|e| format!("Cannot open {}: {e}", path.display()))?;
    let mut buf = *report;
    log::debug!(
        "fourzone: HIDIOCSFEATURE {} {:02X?}",
        path.display(),
        &buf[..20]
    );
    // SAFETY: ioctl on an owned, open hidraw fd; buf is REPORT_LEN bytes and
    // the request encodes exactly that length.
    let ret = unsafe {
        libc::ioctl(
            file.as_raw_fd(),
            crate::keyboard::hid_set_feature(REPORT_LEN),
            buf.as_mut_ptr(),
        )
    };
    if ret < 0 {
        let msg = format!("HIDIOCSFEATURE failed: {}", Error::last_os_error());
        log::warn!("fourzone: {msg}");
        return Err(msg);
    }
    Ok(())
}

/// Send a state to the controller (blocking). Does not persist.
pub fn apply(state: &FourZoneState) -> Result<(), String> {
    write_report(&build_report(state))
}

/// Send and, on success, persist to `settings.json`.
pub fn apply_and_save(state: &FourZoneState) -> Result<(), String> {
    apply(state)?;
    crate::config::update(|c| c.four_zone = crate::config::FourZoneConfig::from_state(state));
    Ok(())
}

struct Job {
    state: FourZoneState,
    save: bool,
    done: Option<Sender<Result<(), String>>>,
}

fn worker_tx() -> &'static Sender<Job> {
    static TX: OnceLock<Sender<Job>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        if let Err(e) = std::thread::Builder::new()
            .name("fourzone-hid".into())
            .spawn(move || worker(rx))
        {
            log::error!("fourzone: worker spawn failed: {e}");
        }
        tx
    })
}

fn worker(rx: Receiver<Job>) {
    while let Ok(mut job) = rx.recv() {
        // Coalesce: only the newest queued state is sent (slider spam).
        let mut waiters: Vec<Sender<Result<(), String>>> = job.done.take().into_iter().collect();
        let mut save = job.save;
        while let Ok(mut next) = rx.try_recv() {
            waiters.extend(next.done.take());
            save |= next.save;
            job = next;
        }
        let res = if save {
            apply_and_save(&job.state)
        } else {
            apply(&job.state)
        };
        if let Err(e) = &res {
            log::warn!("fourzone: apply failed: {e}");
        }
        for w in waiters {
            let _ = w.send(res.clone());
        }
    }
}

/// Queue a state without blocking (GUI). `save` persists on success.
pub fn apply_async(state: FourZoneState, save: bool) {
    let _ = worker_tx().send(Job {
        state,
        save,
        done: None,
    });
}

/// Queue a state and invoke `on_done` on the worker thread with the result.
pub fn apply_async_then(
    state: FourZoneState,
    save: bool,
    on_done: impl FnOnce(Result<(), String>) + Send + 'static,
) {
    let (tx, rx) = mpsc::channel();
    if worker_tx()
        .send(Job {
            state,
            save,
            done: Some(tx),
        })
        .is_err()
    {
        on_done(Err("4-zone HID worker died".into()));
        return;
    }
    std::thread::spawn(move || {
        on_done(
            rx.recv()
                .unwrap_or_else(|_| Err("4-zone HID worker died".into())),
        )
    });
}

/// Re-apply the saved state (launch restore). No-op without the hardware.
pub fn restore_async() {
    if !present() {
        log::debug!("fourzone: restore skipped — no 4-zone controller");
        return;
    }
    apply_async(crate::config::get().four_zone.to_state(), false);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(effect: FzEffect, speed: u8, bri: u8, dir: WaveDir) -> FourZoneState {
        FourZoneState {
            effect,
            speed,
            brightness: bri,
            colors: [
                [0x11, 0x22, 0x33],
                [0x44, 0x55, 0x66],
                [0x77, 0x88, 0x99],
                [0xAA, 0xBB, 0xCC],
            ],
            direction: dir,
        }
    }

    fn hex(s: &str) -> Vec<u8> {
        s.split_whitespace()
            .map(|b| u8::from_str_radix(b, 16).unwrap())
            .collect()
    }

    // Expected bytes are what 4JX `build_payload` / l5p-kbl `build_control_string`
    // produce for the same inputs (see module docs for the reference code).
    #[test]
    fn static_matches_reference() {
        let got = build_report(&state(FzEffect::Static, 1, 2, WaveDir::Right));
        let want = hex("CC 16 01 01 02 11 22 33 44 55 66 77 88 99 AA BB CC \
             00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00");
        assert_eq!(got.to_vec(), want);
        assert_eq!(got.len(), 33);
    }

    #[test]
    fn breath_keeps_colors_and_speed() {
        let got = build_report(&state(FzEffect::Breath, 3, 1, WaveDir::Left));
        let want = hex("CC 16 03 03 01 11 22 33 44 55 66 77 88 99 AA BB CC \
             00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00");
        assert_eq!(got.to_vec(), want);
    }

    #[test]
    fn wave_right_sets_byte18_and_zeroes_colors() {
        // 4JX RightWave → payload[18] = 1; LLT/OpenRGB WaveLTR/wave_ltr at 18.
        let got = build_report(&state(FzEffect::Wave, 4, 2, WaveDir::Right));
        let want = hex("CC 16 04 04 02 00 00 00 00 00 00 00 00 00 00 00 00 \
             00 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00");
        assert_eq!(got.to_vec(), want);
    }

    #[test]
    fn wave_left_sets_byte19() {
        let got = build_report(&state(FzEffect::Wave, 2, 2, WaveDir::Left));
        assert_eq!(got[18], 0);
        assert_eq!(got[19], 1);
        assert_eq!(&got[..5], &[0xCC, 0x16, 0x04, 0x02, 0x02]);
    }

    #[test]
    fn smooth_has_no_colors_or_direction() {
        let got = build_report(&state(FzEffect::Smooth, 2, 1, WaveDir::Left));
        let want = hex("CC 16 06 02 01 00 00 00 00 00 00 00 00 00 00 00 00 \
             00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00");
        assert_eq!(got.to_vec(), want);
    }

    #[test]
    fn off_matches_l5p_kbl() {
        // l5p-kbl: off → [204, 22, 1] + [0] * 30
        let got = build_report(&state(FzEffect::Wave, 4, 0, WaveDir::Right));
        let mut want = vec![0xCC, 0x16, 0x01];
        want.extend([0u8; 30]);
        assert_eq!(got.to_vec(), want);
    }

    #[test]
    fn speed_and_brightness_are_clamped() {
        let got = build_report(&state(FzEffect::Breath, 0, 9, WaveDir::Right));
        assert_eq!(got[3], 1);
        assert_eq!(got[4], 2);
        let got = build_report(&state(FzEffect::Breath, 99, 1, WaveDir::Right));
        assert_eq!(got[3], 4);
    }

    #[test]
    fn names_roundtrip() {
        for fx in FzEffect::ALL {
            assert_eq!(FzEffect::from_name(fx.name()), Some(fx));
        }
        assert_eq!(FzEffect::from_name("off"), None);
        assert_eq!(WaveDir::from_name("LEFT"), Some(WaveDir::Left));
        assert_eq!(WaveDir::from_name("ltr"), Some(WaveDir::Right));
        assert_eq!(WaveDir::from_name("up"), None);
    }

    #[test]
    fn parse_color_table() {
        assert_eq!(parse_color("#FF8000"), Some([255, 128, 0]));
        assert_eq!(parse_color("00ff7f"), Some([0, 255, 127]));
        assert_eq!(parse_color("FF80"), None);
        assert_eq!(parse_color("GG0000"), None);
        assert_eq!(parse_color("äää"), None);
    }

    /// Vendor collection on `page` with one feature report `id` of `bytes`.
    fn desc(page: u16, id: u8, bytes: u8) -> Vec<u8> {
        let p = page.to_le_bytes();
        vec![
            0x06, p[0], p[1], // Usage Page
            0x09, 0xCC, // Usage 0xCC
            0xA1, 0x01, // Collection (Application)
            0x85, id, // Report ID
            0x75, 0x08, // Report Size 8
            0x95, bytes, // Report Count
            0x09, 0x01, // Usage
            0xB1, 0x02, // Feature
            0xC0,
        ]
    }

    #[test]
    fn descriptor_gate() {
        assert!(is_fourzone_descriptor(&desc(0xFF89, 0xCC, 32)));
        // Wrong page, wrong ID, wrong size: all rejected.
        assert!(!is_fourzone_descriptor(&desc(0xFF00, 0xCC, 32)));
        assert!(!is_fourzone_descriptor(&desc(0xFF89, 0x07, 32)));
        assert!(!is_fourzone_descriptor(&desc(0xFF89, 0xCC, 191)));
        assert!(!is_fourzone_descriptor(&[]));
    }

    #[test]
    fn node_target_requires_descriptor() {
        let good = desc(0xFF89, 0xCC, 32);
        assert!(node_is_target("048d", "c965", Some(&good)));
        assert!(node_is_target("048D", "C995", Some(&good)));
        // Never by PID alone.
        assert!(!node_is_target("048d", "c965", None));
        // c193 is never driven, whatever its descriptor says.
        assert!(!node_is_target("048d", "c193", Some(&good)));
        // Spectrum-sized descriptor on a c9xx PID is not a 4-zone target.
        assert!(!node_is_target(
            "048d",
            "c975",
            Some(&desc(0xFF89, 0x07, 0xFF))
        ));
        // Foreign vendor / non-ITE PID.
        assert!(!node_is_target("1234", "c965", Some(&good)));
        assert!(!node_is_target("048d", "a965", Some(&good)));
    }
}
