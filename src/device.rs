//! Device detection, model matching, and live capability probe.

use serde::{Deserialize, Serialize};

use crate::models::{self, ModelProfile};
use std::sync::OnceLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// Best display name (marketing when known).
    pub model: String,
    /// DMI machine type (e.g. `83RU`) when detectable.
    pub machine_type: String,
    /// Marketing / product family string.
    pub marketing_name: String,
    /// Matched series label from the profile DB.
    pub series: String,
    pub bios_version: String,
    /// First 4 characters of BIOS version (LLL-style key).
    pub bios_prefix: String,
    pub ec_chip: String,
    pub cpu_model: String,
    pub gpu_model: String,
    /// Generation guess from profile (`0` = unknown).
    pub gen: u8,
    pub profile_matched: bool,
    pub profile_source: String,
    pub profile_notes: String,
    pub capabilities: Capabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capabilities {
    pub fan_backend: String,
    pub fans: Vec<FanCapability>,
    pub lighting: String,
    pub peak_gpu_w: Option<u32>,
    pub peak_gpu_source: String,
    pub ppt_attrs: Vec<String>,
    pub platform_profiles: Vec<String>,
    pub has_custom_profile: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FanCapability {
    pub id: u8,
    pub title: String,
    pub min_rpm: u32,
    pub max_rpm: u32,
    pub current_rpm: u32,
}

/// Hardware identity is stable for the machine's uptime — cache it so
/// repeated detection never re-spawns `nvidia-smi` (up to 3 s per call).
/// `DeviceInfo.current_rpm` is therefore a detection-time snapshot, not live
/// telemetry; live RPM comes from `fans::read_rpm`.
pub fn detect() -> DeviceInfo {
    static DETECTED: OnceLock<DeviceInfo> = OnceLock::new();
    if let Some(cached) = DETECTED.get() {
        log::debug!("device detect: cache hit");
        return cached.clone();
    }
    let info = DETECTED.get_or_init(detect_uncached).clone();
    log::debug!("device detect: cache miss — full detection ran, result now cached");
    info
}

fn detect_uncached() -> DeviceInfo {
    log::debug!("device detect: full probe starting");
    // sys_vendor is informational only — read for logs, never used for decisions.
    let dmi_vendor = read_dmi("sys_vendor").unwrap_or_default();
    let dmi_name = read_dmi("product_name").unwrap_or_default();
    let dmi_version = read_dmi("product_version").unwrap_or_default();
    let dmi_family = read_dmi("product_family").unwrap_or_default();
    let dmi_sku = read_dmi("product_sku").unwrap_or_default();
    let bios = read_dmi("bios_version").unwrap_or_else(|| {
        log::debug!("device detect: dmi bios_version unreadable — using \"Unknown\"");
        "Unknown".into()
    });
    let bios_prefix = bios_prefix(&bios);

    let (machine_type, marketing) = classify_dmi(&dmi_name, &dmi_version, &dmi_family, &dmi_sku);
    log::debug!(
        "device detect: classify_dmi resolved machine_type={machine_type:?} marketing={marketing:?}"
    );

    let profile = models::lookup(&machine_type, &marketing, &bios_prefix);
    log::debug!(
        "device detect: profile {}",
        match profile {
            Some(p) => format!("matched: {} ({})", p.marketing, p.source),
            None => "unmatched".to_string(),
        }
    );

    let gpu = match crate::dgpu::smi_query("name") {
        Some(name) => {
            log::debug!("device detect: gpu detection via nvidia-smi → {name:?}");
            name
        }
        None => {
            log::debug!(
                "device detect: gpu detection via nvidia-smi returned nothing — using \"Unknown\""
            );
            "Unknown".into()
        }
    };
    let cpu = match read_cpu_model() {
        Some(c) => {
            log::debug!("device detect: cpu model → {c:?}");
            c
        }
        None => {
            log::debug!("device detect: cpu model unreadable — using \"Unknown\"");
            "Unknown".into()
        }
    };

    let capabilities = probe_capabilities(profile, &gpu);

    let (series, gen, notes, source, matched) = match profile {
        Some(p) => {
            log::debug!(
                "device detect: gen assignment {} from profile {}",
                p.gen,
                p.bios_prefix
            );
            (
                p.series.to_string(),
                p.gen,
                p.notes.to_string(),
                p.source.to_string(),
                true,
            )
        }
        None => {
            let guessed = guess_series(&marketing);
            log::debug!(
                "device detect: gen assignment 0 (no profile) — series guessed {guessed:?}"
            );
            (
                guessed.to_string(),
                0,
                String::new(),
                "unmatched — probed from sysfs only".into(),
                false,
            )
        }
    };

    let display = if !marketing.is_empty() && marketing != "Unknown" {
        log::debug!("device detect: display name ← marketing ({marketing})");
        marketing.clone()
    } else if !machine_type.is_empty() {
        log::debug!("device detect: display name ← machine type ({machine_type})");
        machine_type.clone()
    } else {
        log::debug!("device detect: display name — nothing usable, \"Unknown\"");
        "Unknown".into()
    };

    log::info!(
        "device detected: vendor={dmi_vendor:?} model={display:?} machine_type={machine_type:?} \
         series={series:?} gen={gen} gpu={gpu:?} cpu={cpu:?} bios={bios} ({bios_prefix}) \
         profile_matched={matched}"
    );

    DeviceInfo {
        model: display,
        machine_type: if machine_type.is_empty() {
            "Unknown".into()
        } else {
            machine_type
        },
        marketing_name: if marketing.is_empty() {
            "Unknown".into()
        } else {
            marketing
        },
        series,
        bios_version: bios,
        bios_prefix,
        ec_chip: detect_ec_chip(&capabilities.fan_backend),
        cpu_model: cpu,
        gpu_model: gpu,
        gen,
        profile_matched: matched,
        profile_source: source,
        profile_notes: notes,
        capabilities,
    }
}

/// Live capability probe (fans, lighting, PPT, peak TGP).
pub fn probe_capabilities(profile: Option<&'static ModelProfile>, gpu_name: &str) -> Capabilities {
    log::debug!("capability probe starting (gpu={gpu_name:?})");
    let (fan_backend, fans) = probe_fans(profile);
    log::debug!(
        "capability probe: fan backend {fan_backend:?} ({} fan channels)",
        fans.len()
    );
    let lighting = probe_lighting();
    log::debug!("capability probe: lighting {lighting:?}");
    let ppt_attrs = probe_ppt_attrs();
    let platform_profiles = crate::profile::choices();
    let has_custom = platform_profiles.iter().any(|p| p == "custom");
    log::trace!("capability probe: platform profiles {platform_profiles:?} (custom={has_custom})");

    let (peak_gpu_w, peak_gpu_source) = match crate::dgpu::read_power_max() {
        Some(w) => {
            log::debug!(
                "capability probe: peak GPU limit {} W via nvidia-smi power.max_limit",
                w.round() as u32
            );
            (Some(w.round() as u32), "nvidia-smi power.max_limit".into())
        }
        None => {
            match models::expected_tgp_from_gpu_name(gpu_name) {
                Some(w) => {
                    log::debug!("capability probe: peak GPU limit {w} W via PSREF heuristic for {gpu_name:?}");
                    (Some(w), "PSREF / GPU name heuristic".into())
                }
                None => {
                    log::debug!(
                    "capability probe: peak GPU limit unavailable (no smi limit, no heuristic hit)"
                );
                    (None, "unavailable".into())
                }
            }
        }
    };

    log::debug!(
        "capability probe done: fans={} lighting={lighting:?} ppt_attrs={} peak_gpu_w={peak_gpu_w:?}",
        fans.len(),
        ppt_attrs.len()
    );

    Capabilities {
        fan_backend,
        fans,
        lighting,
        peak_gpu_w,
        peak_gpu_source,
        ppt_attrs,
        platform_profiles,
        has_custom_profile: has_custom,
    }
}

fn probe_fans(profile: Option<&'static ModelProfile>) -> (String, Vec<FanCapability>) {
    if let Some(hw) = crate::sensors::hwmon_by_name("lenovo_wmi_other") {
        let fans = collect_fans_from_hwmon(&hw, profile);
        if !fans.is_empty() {
            // Some models (LOQ 15AHP10/83JG, LenovoLegionLinux #384) bind
            // lenovo_wmi_other but the EC reports 0 RPM — the real tachometer
            // lives in the yogafan hwmon. Prefer yogafan when it has nonzero
            // readings.
            if let Some(yw) = crate::sensors::hwmon_by_name("yogafan") {
                let yfans = collect_fans_from_hwmon(&yw, profile);
                if yfans.iter().any(|f| f.current_rpm > 0)
                    && fans.iter().all(|f| f.current_rpm == 0)
                {
                    log::debug!(
                        "fan probe: lenovo_wmi_other reads 0 RPM — yogafan backend at {} has live values",
                        yw.display()
                    );
                    return ("yogafan".into(), yfans);
                }
            }
            log::debug!(
                "fan probe: backend lenovo_wmi_other at {} ({} fans)",
                hw.display(),
                fans.len()
            );
            return ("lenovo_wmi_other".into(), fans);
        }
        log::trace!(
            "fan probe: lenovo_wmi_other present at {} but exposes no fan channels",
            hw.display()
        );
    } else {
        log::trace!("fan probe: no lenovo_wmi_other hwmon");
    }
    if let Some(hw) = crate::sensors::hwmon_by_name("legion_hwmon") {
        let fans = collect_fans_from_hwmon(&hw, profile);
        if !fans.is_empty() {
            log::debug!(
                "fan probe: backend legion_hwmon at {} ({} fans)",
                hw.display(),
                fans.len()
            );
            return ("legion_hwmon".into(), fans);
        }
        log::trace!(
            "fan probe: legion_hwmon present at {} but exposes no fan channels",
            hw.display()
        );
    } else {
        log::trace!("fan probe: no legion_hwmon hwmon");
    }

    // Profile fallbacks when no hwmon yet (daemon early boot, missing module).
    if let Some(p) = profile {
        log::debug!(
            "fan probe: no hwmon fans — using profile fallback ranges ({})",
            p.bios_prefix
        );
        let fans = p
            .fan_rpm_fallback
            .iter()
            .map(|(id, min, max)| FanCapability {
                id: *id,
                title: fan_title(*id).into(),
                min_rpm: *min,
                max_rpm: *max,
                current_rpm: 0,
            })
            .collect();
        return (format!("profile-fallback ({})", p.bios_prefix), fans);
    }

    log::warn!("fan probe: no hwmon and no profile — using static placeholder fan ranges");
    (
        "none".into(),
        vec![
            FanCapability {
                id: 1,
                title: "CPU fan".into(),
                min_rpm: 0,
                max_rpm: 5500,
                current_rpm: 0,
            },
            FanCapability {
                id: 2,
                title: "GPU fan".into(),
                min_rpm: 0,
                max_rpm: 5500,
                current_rpm: 0,
            },
        ],
    )
}

fn collect_fans_from_hwmon(
    hw: &std::path::Path,
    profile: Option<&'static ModelProfile>,
) -> Vec<FanCapability> {
    let mut ids = Vec::new();
    let disp = hw.display();
    match std::fs::read_dir(hw) {
        Ok(entries) => {
            for entry in entries {
                let entry = match entry {
                    Ok(e) => e,
                    Err(e) => {
                        log::trace!("fan probe: {disp} dir entry error: {e}");
                        continue;
                    }
                };
                let name = entry.file_name().to_string_lossy().to_string();
                if let Some(rest) = name.strip_prefix("fan") {
                    if let Some(num) = rest.strip_suffix("_input") {
                        if let Ok(id) = num.parse::<u8>() {
                            ids.push(id);
                        }
                    }
                }
            }
        }
        Err(e) => {
            log::debug!("fan probe: {disp} readdir failed ({e}) — treating as no fan channels")
        }
    }
    ids.sort_unstable();
    ids.dedup();
    log::trace!("fan probe: {disp} fan channels discovered: {ids:?}");

    ids.into_iter()
        .filter_map(|id| {
            let input = match read_u32(hw.join(format!("fan{id}_input"))) {
                Some(v) => v,
                None => {
                    log::trace!("fan probe: {disp} fan{id}_input unreadable — channel skipped");
                    return None;
                }
            };
            let min = read_u32(hw.join(format!("fan{id}_min"))).unwrap_or_else(|| {
                log::trace!("fan probe: {disp} fan{id}: no _min attr — profile/default fallback");
                profile
                    .and_then(|p| {
                        p.fan_rpm_fallback
                            .iter()
                            .find(|(fid, _, _)| *fid == id)
                            .map(|(_, min, _)| *min)
                    })
                    .unwrap_or(0)
            });
            let max = read_u32(hw.join(format!("fan{id}_max"))).unwrap_or_else(|| {
                log::trace!("fan probe: {disp} fan{id}: no _max attr — profile/default fallback");
                profile
                    .and_then(|p| {
                        p.fan_rpm_fallback
                            .iter()
                            .find(|(fid, _, _)| *fid == id)
                            .map(|(_, _, max)| *max)
                    })
                    .unwrap_or(5500)
            });
            let title = match read_file(hw.join(format!("fan{id}_label"))) {
                Some(l) => pretty_fan_label(&l, id),
                None => {
                    log::trace!("fan probe: {disp} fan{id}: no label attr — generic title");
                    fan_title(id).into()
                }
            };
            log::trace!(
                "fan probe: {disp} fan{id} {title:?}: {input} rpm, range {}–{}",
                min,
                max.max(min + 100)
            );
            Some(FanCapability {
                id,
                title,
                min_rpm: min,
                max_rpm: max.max(min + 100),
                current_rpm: input,
            })
        })
        .collect()
}

fn pretty_fan_label(raw: &str, id: u8) -> String {
    let t = raw.trim();
    if t.is_empty() {
        return fan_title(id).into();
    }
    // Keep short UI titles.
    let lower = t.to_ascii_lowercase();
    if lower.contains("cpu") {
        "CPU fan".into()
    } else if lower.contains("gpu") {
        "GPU fan".into()
    } else if lower.contains("aux") || lower.contains("system") || id == 4 {
        "Aux fan".into()
    } else {
        format!("{t} fan")
    }
}

pub fn fan_title(id: u8) -> &'static str {
    match id {
        1 => "CPU fan",
        2 => "GPU fan",
        3 => "Fan 3",
        4 => "Aux fan",
        _ => "Fan",
    }
}

fn probe_lighting() -> String {
    let mut kinds = Vec::new();
    if usb_hid_present("048d", "c197") {
        log::debug!("lighting probe: Spectrum RGB HID (048d:c197) present");
        kinds.push("Spectrum RGB (048d:c197)");
    }
    if usb_hid_present("048d", "c193") {
        log::debug!("lighting probe: Lenovo Lighting HID (048d:c193) present");
        kinds.push("Lenovo Lighting (048d:c193)");
    }
    if usb_hid_present("048d", "c100") || usb_hid_present("048d", "c965") {
        log::debug!("lighting probe: 4-zone RGB HID (048d:c100/c965) present");
        kinds.push("4-zone RGB");
    }
    if kinds.is_empty() {
        log::debug!("lighting probe: none detected");
        "None detected".into()
    } else {
        kinds.join(" · ")
    }
}

fn probe_ppt_attrs() -> Vec<String> {
    let attrs: Vec<String> = crate::profile::all_ppt_limits()
        .into_iter()
        .map(|l| {
            format!(
                "{} (now {}, range {})",
                l.id,
                l.value_label(l.current),
                l.range_label()
            )
        })
        .collect();
    log::debug!("ppt probe: {} firmware attributes visible", attrs.len());
    log::trace!("ppt probe detail: {attrs:?}");
    attrs
}

fn usb_hid_present(vid: &str, pid: &str) -> bool {
    let want_vid = vid.to_ascii_lowercase();
    let want_pid = pid.to_ascii_lowercase();
    let Ok(entries) = std::fs::read_dir("/sys/bus/hid/devices") else {
        log::trace!("hid probe: /sys/bus/hid/devices unreadable");
        return false;
    };
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                log::trace!("hid probe: /sys/bus/hid/devices entry error: {e}");
                continue;
            }
        };
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        // HID_ID style in dirname: 0003:048D:C197.000B
        let parts: Vec<&str> = name.split(':').collect();
        if parts.len() >= 3 {
            let v = parts[1];
            let p = parts[2].split('.').next().unwrap_or("");
            if v == want_vid && p == want_pid {
                log::trace!(
                    "hid probe: {vid}:{pid} present at {}",
                    entry.file_name().to_string_lossy()
                );
                return true;
            }
        } else {
            log::trace!("hid probe: unexpected device dirname {name:?} — skipped");
        }
    }
    log::trace!("hid probe: {vid}:{pid} absent");
    false
}

/// Lenovo often puts machine type in `product_name` and marketing in `product_version`.
fn classify_dmi(
    product_name: &str,
    product_version: &str,
    product_family: &str,
    product_sku: &str,
) -> (String, String) {
    let mt_from_sku = extract_mt_from_sku(product_sku);

    let name_is_mt = looks_like_machine_type(product_name);
    let ver_is_mt = looks_like_machine_type(product_version);
    log::trace!(
        "classify_dmi: name={product_name:?} (mt={name_is_mt}) version={product_version:?} \
         (mt={ver_is_mt}) family={product_family:?} sku_mt={mt_from_sku:?}"
    );

    let machine_type = if name_is_mt {
        log::debug!("classify_dmi: machine type ← product_name ({product_name})");
        product_name.to_string()
    } else if ver_is_mt {
        log::debug!("classify_dmi: machine type ← product_version ({product_version})");
        product_version.to_string()
    } else {
        match &mt_from_sku {
            Some(mt) => log::debug!("classify_dmi: machine type ← product_sku ({mt})"),
            None => log::debug!("classify_dmi: no machine-type source matched — leaving empty"),
        }
        mt_from_sku.unwrap_or_default()
    };

    let marketing = if !product_family.is_empty()
        && !looks_like_machine_type(product_family)
        && product_family.len() > 4
    {
        log::debug!("classify_dmi: marketing ← product_family ({product_family})");
        product_family.to_string()
    } else if !product_version.is_empty() && !looks_like_machine_type(product_version) {
        log::debug!("classify_dmi: marketing ← product_version ({product_version})");
        product_version.to_string()
    } else if !product_name.is_empty() && !looks_like_machine_type(product_name) {
        log::debug!("classify_dmi: marketing ← product_name ({product_name})");
        product_name.to_string()
    } else {
        match marketing_from_sku(product_sku) {
            Some(m) => {
                log::debug!("classify_dmi: marketing ← product_sku _FM_ field ({m})");
                m
            }
            None => {
                log::debug!("classify_dmi: no marketing source matched — leaving empty");
                String::new()
            }
        }
    };

    (machine_type, marketing)
}

fn looks_like_machine_type(s: &str) -> bool {
    let s = s.trim();
    // Modern Lenovo MT: 83RU, 82JQ, 15ACH6H is NOT mt — those are longer marketing codes.
    s.len() == 4
        && s.chars().next().is_some_and(|c| c.is_ascii_digit())
        && s.chars().all(|c| c.is_ascii_alphanumeric())
}

fn extract_mt_from_sku(sku: &str) -> Option<String> {
    // LENOVO_MT_83RU_BU_idea_FM_...
    if let Some(rest) = sku.strip_prefix("LENOVO_MT_") {
        let mt = rest.split('_').next()?;
        if looks_like_machine_type(mt) {
            return Some(mt.to_string());
        }
    }
    None
}

fn marketing_from_sku(sku: &str) -> Option<String> {
    if let Some(idx) = sku.find("_FM_") {
        let name = &sku[idx + 4..];
        if !name.is_empty() {
            return Some(name.replace('_', " "));
        }
    }
    None
}

fn bios_prefix(bios: &str) -> String {
    let b: String = bios.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let prefix = if b.len() >= 4 {
        b[..4].to_ascii_uppercase()
    } else {
        b.to_ascii_uppercase()
    };
    log::debug!("bios prefix: {bios:?} → {prefix:?}");
    prefix
}

fn guess_series(marketing: &str) -> &'static str {
    let m = marketing.to_ascii_lowercase();
    let series = if m.contains("loq") {
        "LOQ"
    } else if m.contains("ideapad") {
        "IdeaPad Gaming"
    } else if m.contains("slim") {
        "Legion Slim"
    } else if m.contains("pro 7") || m.contains("legion 7") {
        "Legion 7 / Pro 7"
    } else if m.contains("pro 5") || m.contains("legion 5") {
        "Legion 5 / Pro 5"
    } else if m.contains("legion") {
        "Legion"
    } else {
        "Unknown"
    };
    log::debug!("series guess: {marketing:?} → {series:?}");
    series
}

fn read_dmi(field: &str) -> Option<String> {
    let path = format!("/sys/class/dmi/id/{field}");
    log::trace!("dmi: reading {path}");
    let value = std::fs::read_to_string(&path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    match &value {
        Some(v) => log::debug!("dmi {field} = {v:?}"),
        None => log::debug!("dmi {field}: unreadable or empty"),
    }
    value
}

fn read_cpu_model() -> Option<String> {
    let text = match std::fs::read_to_string("/proc/cpuinfo") {
        Ok(t) => t,
        Err(e) => {
            log::trace!("cpu model: /proc/cpuinfo unreadable: {e}");
            return None;
        }
    };
    for line in text.lines() {
        if line.starts_with("model name") {
            return line.split(':').nth(1).map(|s| s.trim().to_string());
        }
    }
    log::trace!("cpu model: /proc/cpuinfo has no \"model name\" line");
    None
}

fn detect_ec_chip(fan_backend: &str) -> String {
    match fan_backend {
        "lenovo_wmi_other" => "ITE (via lenovo_wmi_other)".into(),
        "legion_hwmon" => "ITE IT5508 (legion_hwmon)".into(),
        other if other.starts_with("profile-fallback") => format!("Unknown ({other})"),
        _ => "Unknown".into(),
    }
}

fn read_file(path: impl AsRef<std::path::Path>) -> Option<String> {
    std::fs::read_to_string(path.as_ref())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn read_u32(path: impl AsRef<std::path::Path>) -> Option<u32> {
    read_file(path)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_this_machine() {
        let info = detect();
        assert!(
            info.machine_type == "83RU" || info.model.to_ascii_lowercase().contains("legion"),
            "unexpected model: {} / {}",
            info.machine_type,
            info.model
        );
        assert!(!info.capabilities.fans.is_empty(), "expected fan channels");
        assert!(
            info.capabilities.peak_gpu_w.is_some() || info.gpu_model == "Unknown",
            "expected peak GPU or missing nvidia"
        );
    }

    #[test]
    fn looks_like_machine_type_cases() {
        assert!(looks_like_machine_type("83RU"));
        assert!(looks_like_machine_type(" 82JQ "));
        assert!(!looks_like_machine_type("15ACH6H"));
        assert!(!looks_like_machine_type("Legion"));
        assert!(!looks_like_machine_type("ABC"));
        assert!(!looks_like_machine_type(""));
    }

    #[test]
    fn extract_mt_from_sku_cases() {
        assert_eq!(
            extract_mt_from_sku("LENOVO_MT_83RU_BU_idea_FM_Legion Pro 7"),
            Some("83RU".into())
        );
        assert_eq!(extract_mt_from_sku("LENOVO_MT_15ACH6_FOO"), None);
        assert_eq!(extract_mt_from_sku(""), None);
        assert_eq!(extract_mt_from_sku("FOO_MT_83RU"), None);
    }

    #[test]
    fn marketing_from_sku_cases() {
        assert_eq!(
            marketing_from_sku("LENOVO_MT_83RU_BU_idea_FM_Legion Pro 7"),
            Some("Legion Pro 7".into())
        );
        assert_eq!(marketing_from_sku("LENOVO_MT_83RU_BU_idea"), None);
        assert_eq!(marketing_from_sku(""), None);
    }

    #[test]
    fn bios_prefix_filters_and_uppercases() {
        assert_eq!(bios_prefix("N3CN28WW"), "N3CN");
        assert_eq!(bios_prefix("n3cn28ww"), "N3CN");
        assert_eq!(bios_prefix("A-B.C!"), "ABC");
        assert_eq!(bios_prefix("AB"), "AB");
    }

    #[test]
    fn guess_series_variants() {
        assert_eq!(guess_series("Legion Pro 7 16IAX7H"), "Legion 7 / Pro 7");
        assert_eq!(guess_series("Legion Pro 5 16ACH6"), "Legion 5 / Pro 5");
        assert_eq!(guess_series("Legion Slim 7"), "Legion Slim");
        assert_eq!(guess_series("LOQ 15"), "LOQ");
        assert_eq!(guess_series("IdeaPad Gaming 3"), "IdeaPad Gaming");
        assert_eq!(guess_series("ThinkPad X1"), "Unknown");
    }

    #[test]
    fn pretty_fan_label_cases() {
        assert_eq!(pretty_fan_label("CPU Fan", 1), "CPU fan");
        assert_eq!(pretty_fan_label("GPU-Fan", 2), "GPU fan");
        assert_eq!(pretty_fan_label("System", 4), "Aux fan");
        assert_eq!(pretty_fan_label("", 2), "GPU fan");
        assert_eq!(pretty_fan_label("Chassis", 9), "Chassis fan");
    }

    #[test]
    fn fan_title_known_ids() {
        assert_eq!(fan_title(1), "CPU fan");
        assert_eq!(fan_title(2), "GPU fan");
        assert_eq!(fan_title(4), "Aux fan");
    }
}
