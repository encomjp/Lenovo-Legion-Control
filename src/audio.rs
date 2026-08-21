//! Legion Gen 10 speaker / AW88399 smart-amp diagnose + soft recovery.
//!
//! Many Pro 7 Gen 10 units (16AFR10H / 16IAX10H class) drive woofers through
//! Awinic AW88399 HDA side-codecs on I2C (`AWDZ8399`). Without the bridge
//! driver + firmware the amp never comes up and audio is quiet/tinny.
//! Soft failures (mute, stale UCM, wrong PipeWire sink) are recoverable here.
//! Missing kernel/firmware cannot be faked — we report that honestly.

use std::path::Path;
use std::process::Command;

/// Overall speaker-amp health after probing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    /// Amp present, bound, unmuted — looks fine.
    Ok,
    /// Amp stack OK but mute / volume / sink / UCM needs a soft reset.
    SoftIssue,
    /// ACPI amp is there (or expected) but driver/firmware/bind is broken.
    HardwareBroken,
    /// No smart-amp hardware detected on this machine.
    NotApplicable,
}

#[derive(Debug, Clone)]
pub struct Diagnosis {
    pub health: Health,
    pub summary: String,
    pub details: Vec<String>,
    /// Soft recovery (UCM / unmute / PipeWire / sink) is worth trying.
    pub fixable: bool,
    pub amp_acpi: bool,
    pub amp_bound: bool,
    pub amp_modules: bool,
    pub firmware_ok: bool,
    pub hda_card: Option<u32>,
    pub speakers_muted: bool,
    pub bass_off: bool,
    pub volume_low: bool,
    pub wrong_default_sink: bool,
    pub default_sink: Option<String>,
    pub internal_sink: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FixReport {
    pub steps: Vec<String>,
    pub errors: Vec<String>,
    pub after: Diagnosis,
}

/// Probe ALSA + sysfs + PipeWire for AW88399 / speaker state.
pub fn diagnose() -> Diagnosis {
    let amp_acpi = aw88399_acpi_present();
    let amp_bound = aw88399_bound();
    let amp_modules = aw88399_modules_loaded();
    let firmware_ok = aw88399_firmware_present();
    let hda_card = find_alc_hda_card();

    let mut speakers_muted = false;
    let mut bass_off = false;
    let mut volume_low = false;
    if let Some(card) = hda_card {
        // Only treat ALSA mute/bass as a fault when there is no external sink stealing the default.
        // Users who plug in USB headsets (SteelSeries etc.) keep Master muted on purpose — that is not a bug.
        let uses_external = pactl_default_sink().as_deref().is_some_and(|s| {
            s.starts_with("alsa_output.usb") || s.starts_with("bluez_") || s.contains("hdmi")
        });
        if !uses_external {
            speakers_muted = mixer_muted(card, "Speaker") || mixer_muted(card, "Master");
            bass_off = mixer_switch_off(card, "Bass Speaker");
            volume_low = mixer_pct(card, "Speaker").map(|p| p < 40).unwrap_or(false)
                || mixer_pct(card, "Master").map(|p| p < 40).unwrap_or(false);
        }
    }

    let internal_sink = find_internal_analog_sink();
    let default_sink = pactl_default_sink();
    let wrong_default_sink = match (&default_sink, &internal_sink) {
        (Some(def), Some(internal)) => {
            let is_external = def.starts_with("bluez_")
                || def.contains("hdmi")
                || def.starts_with("alsa_output.usb");
            def != internal && !is_external
        }
        (Some(def), None) => {
            def.starts_with("bluez_") || def.contains("hdmi") || def.starts_with("alsa_output.usb")
        }
        _ => false,
    };

    let soft = speakers_muted || bass_off || volume_low || wrong_default_sink;

    let (health, summary, details, fixable) = classify(
        amp_acpi,
        amp_bound,
        amp_modules,
        firmware_ok,
        hda_card,
        soft,
        speakers_muted,
        bass_off,
        volume_low,
        wrong_default_sink,
        &default_sink,
        &internal_sink,
    );

    Diagnosis {
        health,
        summary,
        details,
        fixable,
        amp_acpi,
        amp_bound,
        amp_modules,
        firmware_ok,
        hda_card,
        speakers_muted,
        bass_off,
        volume_low,
        wrong_default_sink,
        default_sink,
        internal_sink,
    }
}

#[allow(clippy::too_many_arguments)]
fn classify(
    amp_acpi: bool,
    amp_bound: bool,
    amp_modules: bool,
    firmware_ok: bool,
    hda_card: Option<u32>,
    soft: bool,
    speakers_muted: bool,
    bass_off: bool,
    volume_low: bool,
    wrong_default_sink: bool,
    default_sink: &Option<String>,
    internal_sink: &Option<String>,
) -> (Health, String, Vec<String>, bool) {
    let mut details = Vec::new();

    if amp_acpi {
        details.push("ACPI AWDZ8399 (AW88399) present".into());
    } else {
        details.push("No AWDZ8399 ACPI device in sysfs".into());
    }
    if amp_modules {
        details.push("AW88399 kernel modules loaded".into());
    } else if amp_acpi {
        details.push("AW88399 modules missing (need patched kernel)".into());
    }
    if amp_bound {
        details.push("AW88399 HDA side-codec bound".into());
    } else if amp_acpi {
        details.push("AW88399 present but NOT bound to HDA — amp dead".into());
    }
    if firmware_ok {
        details.push("Firmware aw88399_acf.bin found".into());
    } else if amp_acpi || amp_modules {
        details.push("Missing /lib/firmware/aw88399_acf.bin".into());
    }
    if let Some(c) = hda_card {
        details.push(format!("ALC / onboard HDA card: hw:{c}"));
    } else {
        details.push("No onboard Realtek/ALC HDA card found".into());
    }
    if speakers_muted {
        details.push("Speaker or Master is muted".into());
    }
    if bass_off {
        details.push("Bass Speaker switch is off".into());
    }
    if volume_low {
        details.push("Speaker/Master volume is very low".into());
    }
    if wrong_default_sink {
        details.push(format!(
            "Default sink is not onboard speakers ({})",
            default_sink.as_deref().unwrap_or("?")
        ));
        if let Some(s) = internal_sink {
            details.push(format!("Onboard sink available: {s}"));
        }
    } else if let Some(s) = default_sink {
        details.push(format!("Default sink: {s}"));
    }

    // Hardware broken: ACPI sees amp but stack incomplete.
    if amp_acpi && (!amp_modules || !amp_bound || !firmware_ok) {
        let mut why = Vec::new();
        if !firmware_ok {
            why.push("firmware");
        }
        if !amp_modules {
            why.push("kernel modules");
        }
        if !amp_bound {
            why.push("HDA bind");
        }
        let summary = format!("Smart amp not working — missing {}", why.join(" / "));
        details.push(
            "Soft reset cannot invent the AW88399 driver. Need aw88399_acf.bin + \
             CONFIG_SND_HDA_SCODEC_AW88399* (CachyOS patched kernel or \
             github.com/marco-giunta/legion-pro7-gen10-audio)."
                .into(),
        );
        return (Health::HardwareBroken, summary, details, soft);
    }

    // Known Gen10 smart-amp model with no ACPI — unusual; still report.
    if !amp_acpi && looks_like_gen10_smart_amp_model() {
        details.push(
            "This Legion Gen 10 model usually has AW88399; ACPI device missing — \
             check BIOS / kernel ACPI."
                .into(),
        );
        return (
            Health::HardwareBroken,
            "Expected AW88399 amp not detected".into(),
            details,
            false,
        );
    }

    if !amp_acpi {
        if hda_card.is_some() && soft {
            return (
                Health::SoftIssue,
                "Speakers need a soft reset (mute/sink)".into(),
                details,
                true,
            );
        }
        return (
            Health::NotApplicable,
            "No AW88399 smart amp detected".into(),
            details,
            soft,
        );
    }

    if soft {
        return (
            Health::SoftIssue,
            "Amp connected — mute, volume, or audio routing needs a reset".into(),
            details,
            true,
        );
    }

    (
        Health::Ok,
        "Smart amp connected and speakers look healthy".into(),
        details,
        true, // still allow proactive soft reset
    )
}

/// Run userspace recovery, then re-diagnose. Never claims hardware is fixed
/// when the amp stack is still broken.
pub fn troubleshoot() -> FixReport {
    let before = diagnose();
    let mut steps = Vec::new();
    let mut errors = Vec::new();

    if before.health == Health::NotApplicable && before.hda_card.is_none() {
        return FixReport {
            steps: vec!["Nothing to do — no onboard HDA card".into()],
            errors,
            after: before,
        };
    }

    if before.health == Health::HardwareBroken && !before.fixable {
        steps.push("Skipped soft reset — amp driver/firmware missing (would not help)".into());
        return FixReport {
            steps,
            errors,
            after: before,
        };
    }

    if let Some(card) = before.hda_card {
        let card_s = card.to_string();
        // UCM is optional — many Legion ALC287 cards have no UCM profile.
        match run_cmd("alsaucm", &["-c", &format!("hw:{card}"), "reset"]) {
            Ok(_) => {
                steps.push(format!("alsaucm reset hw:{card}"));
                match run_cmd("alsaucm", &["-c", &format!("hw:{card}"), "reload"]) {
                    Ok(_) => steps.push(format!("alsaucm reload hw:{card}")),
                    Err(e) => steps.push(format!("alsaucm reload skipped: {e}")),
                }
            }
            Err(e) => {
                let soft = e.contains("UCM is not supported")
                    || e.contains("No such device")
                    || e.contains("failed to import");
                if soft {
                    steps.push("alsaucm skipped (no UCM profile for this card)".into());
                } else {
                    errors.push(format!("alsaucm reset: {e}"));
                }
            }
        }

        for (ctl, extra) in [
            ("Master", &["100%", "unmute"][..]),
            ("Speaker", &["100%", "unmute"][..]),
            ("Bass Speaker", &["unmute"][..]),
            ("Headphone", &["unmute"][..]),
        ] {
            let mut args = vec!["sset", "-c", card_s.as_str(), ctl];
            args.extend_from_slice(extra);
            match run_cmd("amixer", &args) {
                Ok(_) => steps.push(format!("amixer {ctl} → {}", extra.join(" "))),
                Err(e) => {
                    if ctl != "Bass Speaker" && ctl != "Headphone" {
                        errors.push(format!("amixer {ctl}: {e}"));
                    }
                }
            }
        }
    }

    match run_cmd(
        "systemctl",
        &[
            "--user",
            "restart",
            "pipewire.service",
            "pipewire-pulse.service",
            "wireplumber.service",
        ],
    ) {
        Ok(_) => steps.push("restarted pipewire, pipewire-pulse, wireplumber".into()),
        Err(e) => errors.push(format!("systemctl restart audio: {e}")),
    }

    // PipeWire republishes sinks slowly after restart.
    let sink = wait_for_internal_sink(std::time::Duration::from_secs(5));
    if let Some(sink) = sink {
        match run_cmd("pactl", &["set-default-sink", &sink]) {
            Ok(_) => {
                steps.push(format!("default sink → {sink}"));
                let _ = run_cmd("pactl", &["set-sink-mute", &sink, "0"]);
                let _ = run_cmd("pactl", &["set-sink-volume", &sink, "80%"]);
            }
            Err(e) => errors.push(format!("set-default-sink: {e}")),
        }
    } else {
        errors.push("Onboard analog PipeWire sink did not appear after restart".into());
    }

    let after = diagnose();
    match after.health {
        Health::Ok => steps.push("Re-check: amp connected, speakers healthy".into()),
        Health::SoftIssue => steps.push(
            "Re-check: soft issues remain — unplug headsets or pick Speakers in sound settings"
                .into(),
        ),
        Health::HardwareBroken => steps.push(
            "Re-check: amp still broken — soft reset cannot fix missing driver/firmware".into(),
        ),
        Health::NotApplicable => {}
    }

    FixReport {
        steps,
        errors,
        after,
    }
}

fn aw88399_acpi_present() -> bool {
    let Ok(entries) = std::fs::read_dir("/sys/bus/i2c/devices") else {
        return false;
    };
    entries
        .flatten()
        .any(|e| e.file_name().to_string_lossy().contains("AWDZ8399"))
}

fn aw88399_bound() -> bool {
    let drv = Path::new("/sys/bus/i2c/drivers/aw88399-hda");
    if !drv.is_dir() {
        return false;
    }
    std::fs::read_dir(drv)
        .ok()
        .map(|d| {
            d.flatten().any(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                n.contains("AWDZ8399") || n.contains("aw88399")
            })
        })
        .unwrap_or(false)
}

fn aw88399_modules_loaded() -> bool {
    Path::new("/sys/module/snd_hda_scodec_aw88399").is_dir()
        || Path::new("/sys/module/snd_hda_scodec_aw88399_i2c").is_dir()
        || Path::new("/sys/module/snd_soc_aw88399").is_dir()
}

fn aw88399_firmware_present() -> bool {
    [
        "/lib/firmware/aw88399_acf.bin",
        "/usr/lib/firmware/aw88399_acf.bin",
        "/lib/firmware/awinic/aw88399_acf.bin",
        "/usr/lib/firmware/awinic/aw88399_acf.bin",
    ]
    .iter()
    .any(|p| Path::new(p).is_file())
}

fn looks_like_gen10_smart_amp_model() -> bool {
    let ver = std::fs::read_to_string("/sys/class/dmi/id/product_version")
        .unwrap_or_default()
        .to_uppercase();
    const HINTS: &[&str] = &["16AFR10H", "16IAX10H", "16IRX10", "16IAX10", "16ARP10"];
    HINTS.iter().any(|h| ver.contains(h))
}

/// Prefer the Realtek/ALC onboard card (not NVIDIA HDMI).
fn find_alc_hda_card() -> Option<u32> {
    // Prefer codec contents (ALC287 etc.) over vague card names.
    let mut best: Option<(u32, i32)> = None;
    for id in 0..8u32 {
        let codec =
            std::fs::read_to_string(format!("/proc/asound/card{id}/codec#0")).unwrap_or_default();
        let id_str =
            std::fs::read_to_string(format!("/proc/asound/card{id}/id")).unwrap_or_default();
        let text = format!("{codec}\n{id_str}").to_lowercase();
        let score = if text.contains("alc287") || text.contains("realtek") {
            100
        } else if text.contains("alc") {
            80
        } else if id_str.trim().eq_ignore_ascii_case("Generic") {
            40
        } else {
            continue;
        };
        if text.contains("nvidia") || text.contains("hdmi") {
            continue;
        }
        match best {
            Some((_, s)) if s >= score => {}
            _ => best = Some((id, score)),
        }
    }
    if best.is_some() {
        return best.map(|(id, _)| id);
    }

    // Fallback: /proc/asound/cards line pairs
    let cards = std::fs::read_to_string("/proc/asound/cards").ok()?;
    let lines: Vec<&str> = cards.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let id: u32 = match line.split_whitespace().next().and_then(|s| s.parse().ok()) {
            Some(id) => id,
            None => {
                i += 1;
                continue;
            }
        };
        let mut block = line.to_string();
        if i + 1 < lines.len()
            && !lines[i + 1]
                .trim()
                .starts_with(|c: char| c.is_ascii_digit())
        {
            block.push('\n');
            block.push_str(lines[i + 1]);
            i += 2;
        } else {
            i += 1;
        }
        let text = block.to_lowercase();
        if text.contains("nvidia") || text.contains("usb-audio") || text.contains("hdmi") {
            continue;
        }
        if text.contains("hd-audio generic") || text.contains("sof") {
            return Some(id);
        }
    }
    None
}

fn find_internal_analog_sink() -> Option<String> {
    let out = run_cmd("pactl", &["list", "short", "sinks"]).ok()?;
    let mut candidates: Vec<(i32, String)> = Vec::new();
    for line in out.lines() {
        // A malformed line must not abort sink discovery — skip it.
        let Some(name) = line.split_whitespace().nth(1) else {
            continue;
        };
        let lname = name.to_lowercase();
        if lname.contains("hdmi") || lname.starts_with("bluez_") {
            continue;
        }
        if lname.contains("usb-") || lname.contains("dock") {
            continue;
        }
        let score = if lname.contains("analog-stereo") && lname.contains("pci-") {
            100
        } else if lname.contains("analog") && !lname.contains("usb") {
            50
        } else {
            continue;
        };
        candidates.push((score, name.to_string()));
    }
    candidates.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    candidates.into_iter().map(|(_, n)| n).next()
}

fn wait_for_internal_sink(budget: std::time::Duration) -> Option<String> {
    let start = std::time::Instant::now();
    loop {
        if let Some(s) = find_internal_analog_sink() {
            return Some(s);
        }
        if start.elapsed() >= budget {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

fn pactl_default_sink() -> Option<String> {
    let out = run_cmd("pactl", &["get-default-sink"]).ok()?;
    let s = out.trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

fn mixer_muted(card: u32, control: &str) -> bool {
    let Some(out) = amixer_get(card, control) else {
        return false;
    };
    for line in out.lines() {
        if line.contains("Playback") && line.contains("[off]") {
            return true;
        }
    }
    false
}

fn mixer_switch_off(card: u32, control: &str) -> bool {
    let Some(out) = amixer_get(card, control) else {
        return false;
    };
    let mut saw = false;
    let mut any_on = false;
    for line in out.lines() {
        if line.contains("Playback") {
            saw = true;
            if line.contains("[on]") {
                any_on = true;
            }
        }
    }
    saw && !any_on
}

fn mixer_pct(card: u32, control: &str) -> Option<u32> {
    let out = amixer_get(card, control)?;
    for line in out.lines() {
        if !line.contains("Playback") {
            continue;
        }
        for part in line.split('[') {
            if let Some(n) = part.split(']').next() {
                if let Some(num) = n.strip_suffix('%') {
                    if let Ok(v) = num.trim().parse::<u32>() {
                        return Some(v);
                    }
                }
            }
        }
    }
    None
}

fn amixer_get(card: u32, control: &str) -> Option<String> {
    run_cmd("amixer", &["-c", &card.to_string(), "sget", control]).ok()
}

fn run_cmd(bin: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(bin)
        .args(args)
        .output()
        .map_err(|e| format!("{bin}: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if !out.status.success() {
        return Err(if stderr.is_empty() {
            let t = stdout.trim().to_string();
            if t.is_empty() {
                format!("{bin} failed")
            } else {
                t
            }
        } else {
            stderr
        });
    }
    Ok(stdout)
}
