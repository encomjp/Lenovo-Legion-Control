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
    DETECTED.get_or_init(detect_uncached).clone()
}

fn detect_uncached() -> DeviceInfo {
    let dmi_name = read_dmi("product_name").unwrap_or_default();
    let dmi_version = read_dmi("product_version").unwrap_or_default();
    let dmi_family = read_dmi("product_family").unwrap_or_default();
    let dmi_sku = read_dmi("product_sku").unwrap_or_default();
    let bios = read_dmi("bios_version").unwrap_or_else(|| "Unknown".into());
    let bios_prefix = bios_prefix(&bios);

    let (machine_type, marketing) = classify_dmi(&dmi_name, &dmi_version, &dmi_family, &dmi_sku);

    let profile = models::lookup(&machine_type, &marketing, &bios_prefix);
    let gpu = crate::dgpu::smi_query("name").unwrap_or_else(|| "Unknown".into());
    let cpu = read_cpu_model().unwrap_or_else(|| "Unknown".into());

    let capabilities = probe_capabilities(profile, &gpu);

    let (series, gen, notes, source, matched) = match profile {
        Some(p) => (
            p.series.to_string(),
            p.gen,
            p.notes.to_string(),
            p.source.to_string(),
            true,
        ),
        None => (
            guess_series(&marketing).to_string(),
            0,
            String::new(),
            "unmatched — probed from sysfs only".into(),
            false,
        ),
    };

    let display = if !marketing.is_empty() && marketing != "Unknown" {
        marketing.clone()
    } else if !machine_type.is_empty() {
        machine_type.clone()
    } else {
        "Unknown".into()
    };

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
    let (fan_backend, fans) = probe_fans(profile);
    let lighting = probe_lighting();
    let ppt_attrs = probe_ppt_attrs();
    let platform_profiles = crate::profile::choices();
    let has_custom = platform_profiles.iter().any(|p| p == "custom");

    let (peak_gpu_w, peak_gpu_source) = match crate::dgpu::read_power_max() {
        Some(w) => (Some(w.round() as u32), "nvidia-smi power.max_limit".into()),
        None => match models::expected_tgp_from_gpu_name(gpu_name) {
            Some(w) => (Some(w), "PSREF / GPU name heuristic".into()),
            None => (None, "unavailable".into()),
        },
    };

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
            return ("lenovo_wmi_other".into(), fans);
        }
    }
    if let Some(hw) = crate::sensors::hwmon_by_name("legion_hwmon") {
        let fans = collect_fans_from_hwmon(&hw, profile);
        if !fans.is_empty() {
            return ("legion_hwmon".into(), fans);
        }
    }

    // Profile fallbacks when no hwmon yet (daemon early boot, missing module).
    if let Some(p) = profile {
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
    if let Ok(entries) = std::fs::read_dir(hw) {
        for entry in entries.flatten() {
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
    ids.sort_unstable();
    ids.dedup();

    ids.into_iter()
        .filter_map(|id| {
            let input = read_u32(hw.join(format!("fan{id}_input")))?;
            let min = read_u32(hw.join(format!("fan{id}_min"))).unwrap_or_else(|| {
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
                profile
                    .and_then(|p| {
                        p.fan_rpm_fallback
                            .iter()
                            .find(|(fid, _, _)| *fid == id)
                            .map(|(_, _, max)| *max)
                    })
                    .unwrap_or(5500)
            });
            let title = read_file(hw.join(format!("fan{id}_label")))
                .map(|l| pretty_fan_label(&l, id))
                .unwrap_or_else(|| fan_title(id).into());
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
        kinds.push("Spectrum RGB (048d:c197)");
    }
    if usb_hid_present("048d", "c193") {
        kinds.push("Lenovo Lighting (048d:c193)");
    }
    if usb_hid_present("048d", "c100") || usb_hid_present("048d", "c965") {
        kinds.push("4-zone RGB");
    }
    if kinds.is_empty() {
        "None detected".into()
    } else {
        kinds.join(" · ")
    }
}

fn probe_ppt_attrs() -> Vec<String> {
    crate::profile::all_ppt_limits()
        .into_iter()
        .map(|l| format!("{} ({}–{} W, now {} W)", l.id, l.min, l.max, l.current))
        .collect()
}

fn usb_hid_present(vid: &str, pid: &str) -> bool {
    let want_vid = vid.to_ascii_lowercase();
    let want_pid = pid.to_ascii_lowercase();
    let Ok(entries) = std::fs::read_dir("/sys/bus/hid/devices") else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        // HID_ID style in dirname: 0003:048D:C197.000B
        let parts: Vec<&str> = name.split(':').collect();
        if parts.len() >= 3 {
            let v = parts[1];
            let p = parts[2].split('.').next().unwrap_or("");
            if v == want_vid && p == want_pid {
                return true;
            }
        }
    }
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

    let machine_type = if name_is_mt {
        product_name.to_string()
    } else if ver_is_mt {
        product_version.to_string()
    } else {
        mt_from_sku.unwrap_or_default()
    };

    let marketing = if !product_family.is_empty()
        && !looks_like_machine_type(product_family)
        && product_family.len() > 4
    {
        product_family.to_string()
    } else if !product_version.is_empty() && !looks_like_machine_type(product_version) {
        product_version.to_string()
    } else if !product_name.is_empty() && !looks_like_machine_type(product_name) {
        product_name.to_string()
    } else {
        marketing_from_sku(product_sku).unwrap_or_default()
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
    if b.len() >= 4 {
        b[..4].to_ascii_uppercase()
    } else {
        b.to_ascii_uppercase()
    }
}

fn guess_series(marketing: &str) -> &'static str {
    let m = marketing.to_ascii_lowercase();
    if m.contains("loq") {
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
    }
}

fn read_dmi(field: &str) -> Option<String> {
    std::fs::read_to_string(format!("/sys/class/dmi/id/{field}"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn read_cpu_model() -> Option<String> {
    for line in std::fs::read_to_string("/proc/cpuinfo").ok()?.lines() {
        if line.starts_with("model name") {
            return line.split(':').nth(1).map(|s| s.trim().to_string());
        }
    }
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
