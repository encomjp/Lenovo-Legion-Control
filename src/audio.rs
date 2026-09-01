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
    log::debug!("audio: diagnose: probing AW88399 amp stack, ALSA mixer, PipeWire sinks");
    let amp_acpi = aw88399_acpi_present();
    let amp_bound = aw88399_bound();
    let amp_modules = aw88399_modules_loaded();
    let firmware_ok = aw88399_firmware_present();
    let hda_card = find_alc_hda_card();
    log::debug!(
        "audio: probe amp stack: acpi={amp_acpi} bound={amp_bound} modules={amp_modules} firmware={firmware_ok} hda_card={hda_card:?}"
    );
    log::debug!(
        "audio: probe tools: alsaucm available={}",
        alsaucm_available()
    );

    let mut speakers_muted = false;
    let mut bass_off = false;
    let mut volume_low = false;
    if let Some(card) = hda_card {
        // Only treat ALSA mute/bass as a fault when there is no external sink stealing the default.
        // Users who plug in USB headsets (SteelSeries etc.) keep Master muted on purpose — that is not a bug.
        let default_now = pactl_default_sink();
        let uses_external = default_now.as_deref().is_some_and(|s| {
            s.starts_with("alsa_output.usb") || s.starts_with("bluez_") || s.contains("hdmi")
        });
        log::debug!(
            "audio: external-sink gate: uses_external={uses_external} (default sink '{}') — {}",
            default_now.as_deref().unwrap_or("<none>"),
            if uses_external {
                "onboard speakers not in play; skipping mute/bass/volume checks"
            } else {
                "onboard speakers in play; checking mixer state"
            }
        );
        if !uses_external {
            speakers_muted = mixer_muted(card, "Speaker") || mixer_muted(card, "Master");
            bass_off = mixer_switch_off(card, "Bass Speaker");
            volume_low = mixer_pct(card, "Speaker").map(|p| p < 40).unwrap_or(false)
                || mixer_pct(card, "Master").map(|p| p < 40).unwrap_or(false);
        }
    }
    log::debug!(
        "audio: mixer state: speakers_muted={speakers_muted} bass_off={bass_off} volume_low={volume_low}"
    );

    let internal_sink = find_internal_analog_sink();
    let default_sink = pactl_default_sink();
    log::debug!("audio: sink eval: default={default_sink:?} internal_analog={internal_sink:?}");
    let wrong_default_sink = match (&default_sink, &internal_sink) {
        (Some(def), Some(internal)) => {
            let is_external = def.starts_with("bluez_")
                || def.contains("hdmi")
                || def.starts_with("alsa_output.usb");
            log::debug!(
                "audio: wrong-default check: default='{def}' internal='{internal}' external_default={is_external} → wrong={}",
                def != internal && !is_external
            );
            def != internal && !is_external
        }
        (Some(def), None) => {
            let is_external = def.starts_with("bluez_")
                || def.contains("hdmi")
                || def.starts_with("alsa_output.usb");
            log::debug!(
                "audio: wrong-default check: no internal analog sink; default='{def}' external={is_external}"
            );
            is_external
        }
        _ => {
            log::debug!(
                "audio: wrong-default check: default or internal sink unknown → wrong=false"
            );
            false
        }
    };
    log::debug!(
        "audio: soft-issue inputs: muted={speakers_muted} bass_off={bass_off} volume_low={volume_low} wrong_default_sink={wrong_default_sink}"
    );

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

    match health {
        Health::Ok => log::info!(
            "audio: classify: {summary} (acpi={amp_acpi} bound={amp_bound} modules={amp_modules} fw={firmware_ok} card={hda_card:?})"
        ),
        Health::SoftIssue | Health::HardwareBroken => log::warn!(
            "audio: classify: {summary} (acpi={amp_acpi} bound={amp_bound} modules={amp_modules} fw={firmware_ok} card={hda_card:?} muted={speakers_muted} bass_off={bass_off} volume_low={volume_low} wrong_sink={wrong_default_sink})"
        ),
        Health::NotApplicable => log::info!(
            "audio: classify: {summary} (acpi={amp_acpi}, hda_card={hda_card:?})"
        ),
    }

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
            "Soft reset cannot invent the AW88399 driver. Need aw88399_acf.bin in \
             /lib/firmware + CONFIG_SND_HDA_SCODEC_AW88399* (kernel 7.3+ ships it; \
             on CachyOS 7.2 it is already built in — usually only the firmware \
             file is missing). Reference: \
             github.com/marco-giunta/legion-pro7-gen10-audio."
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
    log::info!(
        "audio: troubleshoot entered — pre-health {:?} fixable={} muted={} bass_off={} volume_low={} wrong_default_sink={}",
        before.health,
        before.fixable,
        before.speakers_muted,
        before.bass_off,
        before.volume_low,
        before.wrong_default_sink
    );

    if before.health == Health::NotApplicable {
        log::info!(
            "audio: troubleshoot: no AW88399 smart amp on this model (health NotApplicable) — speaker fix is Gen10-only, will be removed once kernel 7.3 ships"
        );
        return FixReport {
            steps: vec!["No AW88399 smart amp on this model — speaker fix is Gen10-only".into()],
            errors,
            after: before,
        };
    }

    if before.health == Health::HardwareBroken && !before.fixable {
        log::warn!(
            "audio: troubleshoot: skipping soft reset — amp driver/firmware missing (would not help)"
        );
        steps.push("Skipped soft reset — amp driver/firmware missing (would not help)".into());
        return FixReport {
            steps,
            errors,
            after: before,
        };
    }

    // Gate every destructive action on the *pre-state*, exactly like
    // diagnose() computes it: those flags are only set when the onboard
    // speakers are actually in play, so users of external sinks (USB
    // headset, BT, HDMI) keep their mixer levels, default sink, and running
    // services untouched instead of having them hijacked by a "fix".
    let mixer_needs_touch = before.speakers_muted || before.bass_off || before.volume_low;
    log::debug!(
        "audio: mixer gate: needs_touch={mixer_needs_touch} (flags are only set when no external sink is active)"
    );

    if let Some(card) = before.hda_card {
        let card_s = card.to_string();
        // UCM is optional — many Legion ALC287 cards have no UCM profile.
        log::debug!("audio: attempting alsaucm reset/reload on hw:{card} (UCM optional)");
        match run_cmd("alsaucm", &["-c", &format!("hw:{card}"), "reset"]) {
            Ok(_) => {
                log::info!("audio: alsaucm reset hw:{card} ok");
                steps.push(format!("alsaucm reset hw:{card}"));
                match run_cmd("alsaucm", &["-c", &format!("hw:{card}"), "reload"]) {
                    Ok(_) => {
                        log::info!("audio: alsaucm reload hw:{card} ok");
                        steps.push(format!("alsaucm reload hw:{card}"));
                    }
                    Err(e) => {
                        log::debug!("audio: alsaucm reload skipped on hw:{card}: {e}");
                        steps.push(format!("alsaucm reload skipped: {e}"));
                    }
                }
            }
            Err(e) => {
                let soft = e.contains("UCM is not supported")
                    || e.contains("No such device")
                    || e.contains("failed to import");
                if soft {
                    log::info!("audio: alsaucm skipped — no UCM profile for this card ({e})");
                    steps.push("alsaucm skipped (no UCM profile for this card)".into());
                } else {
                    log::warn!("audio: alsaucm reset failed: {e}");
                    errors.push(format!("alsaucm reset: {e}"));
                }
            }
        }

        if mixer_needs_touch {
            log::info!(
                "audio: applying amixer resets on card {card_s} (Master/Speaker/Bass Speaker/Headphone)"
            );
            for (ctl, extra) in [
                ("Master", &["100%", "unmute"][..]),
                ("Speaker", &["100%", "unmute"][..]),
                ("Bass Speaker", &["unmute"][..]),
                ("Headphone", &["unmute"][..]),
            ] {
                let mut args = vec!["sset", "-c", card_s.as_str(), ctl];
                args.extend_from_slice(extra);
                match run_cmd("amixer", &args) {
                    Ok(_) => {
                        log::info!("audio: amixer sset {ctl} → {} ok", extra.join(" "));
                        steps.push(format!("amixer {ctl} → {}", extra.join(" ")));
                    }
                    Err(e) => {
                        if ctl != "Bass Speaker" && ctl != "Headphone" {
                            log::warn!("audio: amixer sset {ctl} failed: {e}");
                            errors.push(format!("amixer {ctl}: {e}"));
                        } else {
                            log::debug!("audio: amixer sset {ctl} failed (optional control): {e}");
                        }
                    }
                }
            }
        } else {
            log::info!(
                "audio: gating amixer changes — mute/bass/volume looked fine pre-reset (external sink likely active); leaving user levels untouched"
            );
            steps.push(
                "skipped amixer changes — mute/bass/volume looked fine before the reset \
                 (onboard speakers not in play; leaving user levels untouched)"
                    .into(),
            );
        }
    }

    if before.health == Health::Ok {
        log::info!(
            "audio: gating service restart — audio stack was healthy pre-reset; skipping pipewire/wireplumber restart"
        );
        steps.push(
            "skipped pipewire/wireplumber restart — audio stack was healthy before the reset"
                .into(),
        );
    } else {
        log::info!("audio: restarting pipewire, pipewire-pulse, wireplumber (user services)");
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
            Ok(_) => {
                log::info!("audio: user audio services restarted");
                steps.push("restarted pipewire, pipewire-pulse, wireplumber".into());
            }
            Err(e) => {
                log::warn!("audio: systemctl restart of user audio services failed: {e}");
                errors.push(format!("systemctl restart audio: {e}"));
            }
        }
    }

    // PipeWire republishes sinks slowly after restart. Only steal the default
    // sink when diagnose() saw a wrong one to begin with.
    if before.wrong_default_sink {
        log::info!(
            "audio: default sink '{}' was wrong pre-reset — waiting up to 5s for onboard analog sink",
            before.default_sink.as_deref().unwrap_or("?")
        );
        let sink = wait_for_internal_sink(std::time::Duration::from_secs(5));
        if let Some(sink) = sink {
            log::info!("audio: onboard analog sink appeared: {sink} — setting as default");
            match run_cmd("pactl", &["set-default-sink", &sink]) {
                Ok(_) => {
                    log::info!("audio: default sink → {sink}");
                    steps.push(format!("default sink → {sink}"));
                    let _ = run_cmd("pactl", &["set-sink-mute", &sink, "0"]);
                    let _ = run_cmd("pactl", &["set-sink-volume", &sink, "80%"]);
                    log::debug!("audio: sink {sink} unmuted and set to 80% volume");
                }
                Err(e) => {
                    log::warn!("audio: set-default-sink {sink} failed: {e}");
                    errors.push(format!("set-default-sink: {e}"));
                }
            }
        } else {
            log::warn!(
                "audio: onboard analog PipeWire sink did not appear within 5s after restart"
            );
            errors.push("Onboard analog PipeWire sink did not appear after restart".into());
        }
    } else {
        log::info!(
            "audio: gating set-default-sink — '{}' already a sane default pre-reset (external sink kept in charge)",
            before.default_sink.as_deref().unwrap_or("current default")
        );
        steps.push(format!(
            "skipped set-default-sink — {} was already a sane default before the reset",
            before.default_sink.as_deref().unwrap_or("current default")
        ));
    }

    let after = diagnose();
    match after.health {
        Health::Ok => {
            log::info!("audio: re-check: amp connected, speakers healthy");
            steps.push("Re-check: amp connected, speakers healthy".into());
        }
        Health::SoftIssue => {
            log::warn!(
                "audio: re-check: soft issues remain — unplug headsets or pick Speakers in sound settings"
            );
            steps.push(
                "Re-check: soft issues remain — unplug headsets or pick Speakers in sound settings"
                    .into(),
            );
        }
        Health::HardwareBroken => {
            log::warn!(
                "audio: re-check: amp still broken — soft reset cannot fix missing driver/firmware"
            );
            steps.push(
                "Re-check: amp still broken — soft reset cannot fix missing driver/firmware".into(),
            );
        }
        Health::NotApplicable => {}
    }
    log::info!(
        "audio: troubleshoot finished — post-health {:?}",
        after.health
    );

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
    let name = std::fs::read_to_string("/sys/class/dmi/id/product_name")
        .unwrap_or_default()
        .to_uppercase();
    const HINTS: &[&str] = &[
        // Legion Pro 7 / Pro 7i Gen 10 (16AFR10H / 16IAX10H) — the reference
        // AW88399 models; R9000P ADR10(H) and Y9000P IAX10 share the amp per
        // marco-giunta/legion-pro7-gen10-audio; LOQ 15IRX10/15IAX10 match the
        // "Gen 10 IRX/IAX" naming. Checked against product_version AND
        // product_name (83JG-style DMI names differ between the two files).
        "16AFR10H", "16IAX10H", "16IRX10", "16IAX10", "16ARP10", "ADR10", "IAX10H", "83RU", "83JG",
        "R9000P", "Y9000P",
    ];
    HINTS.iter().any(|h| ver.contains(h) || name.contains(h))
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
    let out = match run_cmd("pactl", &["list", "short", "sinks"]) {
        Ok(out) => out,
        Err(e) => {
            log::debug!("audio: pactl list short sinks failed: {e}");
            return None;
        }
    };
    let line_count = out.lines().count();
    let picked = select_internal_sink(&out);
    log::debug!(
        "audio: pactl sinks: parsed {line_count} line(s) → internal analog candidate {}",
        picked.as_deref().unwrap_or("<none>")
    );
    picked
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
        log::debug!("audio: pactl get-default-sink returned empty");
        None
    } else {
        log::trace!("audio: pactl default sink: {s}");
        Some(s.to_string())
    }
}

/// Whether the `alsaucm` binary is reachable on `$PATH` (diagnostics only).
fn alsaucm_available() -> bool {
    match std::env::var_os("PATH") {
        Some(paths) => std::env::split_paths(&paths).any(|dir| dir.join("alsaucm").is_file()),
        None => false,
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

/// Pure helper — the scoring in `find_internal_analog_sink`. The single
/// implementation behind the pactl path; exported so tests exercise the
/// same ranking production uses.
pub(crate) fn score_sink_name(name: &str) -> Option<i32> {
    let lname = name.to_lowercase();
    if lname.contains("hdmi") || lname.starts_with("bluez_") {
        return None;
    }
    if lname.contains("usb-") || lname.contains("dock") {
        return None;
    }
    if lname.contains("analog-stereo") && lname.contains("pci-") {
        Some(100)
    } else if lname.contains("analog") && !lname.contains("usb") {
        Some(50)
    } else {
        None
    }
}

/// Pure helper: pick best sink from a `pactl list short sinks` output —
/// used by production (`find_internal_analog_sink`) and tests alike.
pub(crate) fn select_internal_sink(pactl_output: &str) -> Option<String> {
    let mut cands: Vec<(i32, String)> = Vec::new();
    for line in pactl_output.lines() {
        let Some(name) = line.split_whitespace().nth(1) else {
            continue;
        };
        if let Some(score) = score_sink_name(name) {
            log::trace!("audio: sink candidate '{name}' scored {score}");
            cands.push((score, name.to_string()));
        }
    }
    cands.sort_by_key(|(s, _)| std::cmp::Reverse(*s));
    // Stable sort: equal scores keep pactl listing order — first listed wins ties.
    let winner = cands.first().cloned();
    match &winner {
        Some((score, name)) => log::debug!(
            "audio: internal sink selected: '{name}' (score {score}, {} candidate(s); ties broken by pactl order)",
            cands.len()
        ),
        None => log::debug!("audio: no internal analog sink among pactl entries"),
    }
    winner.map(|(_, n)| n)
}

fn amixer_get(card: u32, control: &str) -> Option<String> {
    run_cmd("amixer", &["-c", &card.to_string(), "sget", control]).ok()
}

fn run_cmd(bin: &str, args: &[&str]) -> Result<String, String> {
    log::trace!("audio: exec: {bin} {}", args.join(" "));
    let out = Command::new(bin).args(args).output().map_err(|e| {
        log::debug!("audio: exec {bin} spawn failed: {e}");
        format!("{bin}: {e}")
    })?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if !out.status.success() {
        let msg = if stderr.is_empty() {
            let t = stdout.trim().to_string();
            if t.is_empty() {
                format!("{bin} failed")
            } else {
                t
            }
        } else {
            stderr
        };
        log::debug!("audio: exec {bin} exited {:?}: {msg}", out.status.code());
        return Err(msg);
    }
    log::trace!("audio: exec {bin} ok");
    Ok(stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sink_scoring_prefers_pci_analog_stereo() {
        assert_eq!(
            score_sink_name("alsa_output.pci-0000_00_1f.3.analog-stereo"),
            Some(100)
        );
        assert_eq!(
            score_sink_name("alsa_output.pci-0000_04_00.1.analog-stereo"),
            Some(100)
        );
        assert_eq!(score_sink_name("alsa_output.pci.analog"), Some(50));
        assert!(score_sink_name("alsa_output.hdmi-stereo").is_none());
        assert!(score_sink_name("bluez_output.XX_analog").is_none());
        assert!(score_sink_name("alsa_output.usb-Dock.analog-stereo").is_none());
    }

    #[test]
    fn sink_selection_skips_malformed_and_picks_best() {
        let pactl = "\
0\talsa_output.hdmi-stereo\tmodule-x\n\
malformed-line-without-tabs\n\
1\talsa_output.pci-0000_00_1f.3.analog-stereo\tmodule-y\n\
2\talsa_output.pci-analog\tmodule-z\n";
        assert_eq!(
            select_internal_sink(pactl),
            Some("alsa_output.pci-0000_00_1f.3.analog-stereo".into())
        );
    }

    #[test]
    fn sink_selection_returns_none_when_only_hdmi() {
        let pactl = "0\talsa_output.hdmi-stereo\tmodule-x\n";
        assert!(select_internal_sink(pactl).is_none());
    }

    #[test]
    fn sink_selection_empty_is_none() {
        assert!(select_internal_sink("").is_none());
        assert!(select_internal_sink("\n\n").is_none());
    }

    // ── Gen10-only gating (83JG LOQ, 83DG Y7000P fleet) ─────────────────
    #[test]
    fn classify_not_applicable_when_no_aw88399() {
        // Fleet 83DG Y7000P IRX9 + 83JG LOQ: no AWDZ8399 ACPI device, onboard
        // HDA card present (hw:0), soft flags from mute/volume/sink. On most
        // models this is NotApplicable; on Gen10 DMI (83RU etc.) the same
        // inputs surface as HardwareBroken (expected amp missing) — either way
        // the speaker fix must stay disabled (UI grey, daemon early-exit).
        let (health, summary, details, fixable) = classify(
            false, true, true, true, Some(0), true, true, false, true, false, &None, &None,
        );
        assert!(
            matches!(health, Health::NotApplicable | Health::HardwareBroken),
            "health={health:?}"
        );
        assert!(details.iter().any(|d| d.contains("No AWDZ8399")));
        if health == Health::NotApplicable {
            assert!(summary.contains("No AW88399"));
            // fixable mirrors soft even for N/A (UI shows N/A pill muted) — daemon
            // troubleshoot() must early-exit regardless.
            assert!(fixable);
        } else {
            // HardwareBroken path for Gen10 DMI with missing ACPI: expected amp not detected
            assert!(summary.contains("Expected") || summary.contains("Smart amp"));
            assert!(!fixable);
        }
    }

    #[test]
    fn classify_hardware_broken_when_amp_acpi_without_modules() {
        let (health, summary, _, fixable) = classify(
            true, false, false, false, Some(0), false, false, false, false, false, &None, &None,
        );
        assert_eq!(health, Health::HardwareBroken);
        assert!(summary.contains("Smart amp not working"));
        assert!(!fixable);
    }

    #[test]
    fn classify_soft_issue_when_amp_ok_but_muted() {
        let (health, summary, _, fixable) = classify(
            true, true, true, true, Some(0), true, true, false, false, false, &None, &None,
        );
        assert_eq!(health, Health::SoftIssue);
        assert!(fixable);
        assert!(summary.contains("Amp connected"));
    }

    #[test]
    fn classify_ok_when_amp_and_no_soft() {
        let (health, _, _, fixable) = classify(
            true, true, true, true, Some(0), false, false, false, false, false, &None, &None,
        );
        assert_eq!(health, Health::Ok);
        assert!(fixable);
    }
}
