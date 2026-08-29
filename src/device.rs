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

    let gpu_inventory = gpu_inventory();
    let gpu = if gpu_inventory.discrete_vendor.as_deref() == Some("NVIDIA") {
        crate::dgpu::smi_query("name")
            .or_else(|| gpu_inventory.discrete_name.clone())
            .unwrap_or_else(|| "Unknown".into())
    } else {
        gpu_inventory
            .discrete_name
            .clone()
            .unwrap_or_else(|| "Unknown".into())
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

const GPU_VENDORS: [(&str, &str, &str); 3] = [
    ("0x10de", "10de", "NVIDIA"),
    ("0x1002", "1002", "AMD"),
    ("0x8086", "8086", "Intel"),
];

#[derive(Debug, Clone)]
struct PciGpu {
    slot: String,
    vendor: String,
    device: String,
    name: String,
}

#[derive(Debug, Clone, Default)]
pub struct GpuInventory {
    pub discrete_name: Option<String>,
    pub integrated_name: Option<String>,
    pub discrete_vendor: Option<String>,
    pub discrete_pci_id: Option<String>,
}

fn pci_gpus() -> Vec<PciGpu> {
    let mut cards = Vec::new();
    let Ok(entries) = std::fs::read_dir("/sys/bus/pci/devices") else {
        return cards;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let class = read_file(path.join("class")).unwrap_or_default();
        if !class.trim_start_matches("0x").starts_with("03") {
            continue;
        }
        let vendor = read_file(path.join("vendor"))
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !GPU_VENDORS
            .iter()
            .any(|(sysfs, _, _)| sysfs.eq_ignore_ascii_case(&vendor))
        {
            continue;
        }
        let device = read_file(path.join("device"))
            .unwrap_or_default()
            .trim_start_matches("0x")
            .to_ascii_uppercase();
        if device.is_empty() {
            continue;
        }
        let mut card = PciGpu {
            slot: entry.file_name().to_string_lossy().into_owned(),
            vendor,
            device,
            name: String::new(),
        };
        card.name = resolve_gpu_name(&card);
        cards.push(card);
    }
    cards
}

fn select_discrete_gpu(cards: &[PciGpu]) -> Option<&PciGpu> {
    // boot_vga is only the firmware console and can point at the dGPU in MUX
    // mode. Use positive vendor/marketing evidence; ambiguous controllers
    // remain unclassified rather than being reported in the wrong role.
    cards.iter().find(|card| looks_discrete_gpu(card))
}

fn looks_discrete_gpu(card: &PciGpu) -> bool {
    if card.vendor == "0x10de" {
        return true;
    }
    let name = card.name.to_ascii_lowercase();
    match card.vendor.as_str() {
        "0x1002" => ["radeon rx", "radeon pro", "firepro", "instinct"]
            .iter()
            .any(|marker| name.contains(marker)),
        "0x8086" => ["arc a", "arc b", "data center gpu"]
            .iter()
            .any(|marker| name.contains(marker)),
        _ => false,
    }
}

fn looks_integrated_gpu(card: &PciGpu) -> bool {
    let name = card.name.to_ascii_lowercase();
    match card.vendor.as_str() {
        "0x1002" => [
            "radeon graphics",
            "radeon vega",
            "radeon 6",
            "radeon 7",
            "radeon 8",
            "radeon 9",
        ]
        .iter()
        .any(|marker| name.contains(marker)),
        "0x8086" => ["uhd graphics", "iris", "arc graphics", "integrated graphics"]
            .iter()
            .any(|marker| name.contains(marker)),
        _ => false,
    }
}

fn resolve_gpu_name(card: &PciGpu) -> String {
    let (_, pci_vendor, label) = GPU_VENDORS
        .iter()
        .find(|(sysfs, _, _)| sysfs.eq_ignore_ascii_case(&card.vendor))
        .copied()
        .unwrap_or((card.vendor.as_str(), "unknown", "GPU"));
    if let Ok(output) = std::process::Command::new("lspci")
        .args(["-s", card.slot.as_str()])
        .output()
    {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            if let Some(name) = text
                .lines()
                .next()
                .and_then(|line| line.split_once(": "))
                .map(|(_, raw)| clean_gpu_name(raw))
                .filter(|name| {
                    !name.is_empty()
                        && name != "AMD/ATI"
                        && !name.to_ascii_lowercase().starts_with("device ")
                })
            {
                return name;
            }
        }
    }
    format!("{label} GPU ({pci_vendor}:{})", card.device)
}

fn gpu_inventory_uncached() -> GpuInventory {
    let cards = pci_gpus();
    let discrete = select_discrete_gpu(&cards);
    let integrated = cards
        .iter()
        .find(|card| {
            discrete.is_none_or(|dgpu| dgpu.slot != card.slot) && looks_integrated_gpu(card)
        });
    GpuInventory {
        discrete_name: discrete.map(|card| card.name.clone()),
        integrated_name: integrated.map(|card| card.name.clone()),
        discrete_vendor: discrete.map(|card| {
            GPU_VENDORS
                .iter()
                .find(|(sysfs, _, _)| sysfs.eq_ignore_ascii_case(&card.vendor))
                .map(|(_, _, label)| (*label).to_string())
                .unwrap_or_else(|| "Unknown".into())
        }),
        discrete_pci_id: discrete.map(|card| {
            format!(
                "{}:{}",
                card.vendor.trim_start_matches("0x"),
                card.device.to_ascii_lowercase()
            )
        }),
    }
}

pub fn gpu_inventory() -> GpuInventory {
    static INVENTORY: OnceLock<GpuInventory> = OnceLock::new();
    INVENTORY.get_or_init(gpu_inventory_uncached).clone()
}

pub fn discrete_gpu_present() -> bool {
    gpu_inventory().discrete_name.is_some()
}

/// Trim marketing noise from an lspci GPU name.
fn clean_gpu_name(raw: &str) -> String {
    let without_rev = raw
        .trim()
        .split_once(" (rev ")
        .map(|(name, _)| name)
        .unwrap_or_else(|| raw.trim());
    let stripped = without_rev
        .strip_prefix("NVIDIA Corporation ")
        .or_else(|| without_rev.strip_prefix("Advanced Micro Devices, Inc. [AMD/ATI] "))
        .or_else(|| without_rev.strip_prefix("Intel Corporation "))
        .unwrap_or(without_rev)
        .trim_start_matches("AMD/ATI] ")
        .trim();

    // lspci often emits a chip codename followed by the useful marketing
    // name, e.g. "Cezanne [Radeon Vega Series]". Taking the final bracket
    // pair avoids spanning the vendor tag `[AMD/ATI]` and the model pair.
    if let Some(open) = stripped.rfind('[') {
        if let Some(close) = stripped[open + 1..].find(']') {
            let candidate = stripped[open + 1..open + 1 + close].trim();
            if !candidate.is_empty() && candidate != "AMD/ATI" {
                return candidate.to_string();
            }
        }
    }
    stripped.to_string()
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

    // yogafan (kernel 7.1+): the only fan source on IdeaPad Gaming/Yoga
    // (no lenovo_wmi_other at all) — read-only tachometer, no target/min/max.
    if let Some(hw) = crate::sensors::hwmon_by_name("yogafan") {
        let fans = collect_fans_from_hwmon(&hw, profile);
        if !fans.is_empty() {
            log::debug!(
                "fan probe: backend yogafan at {} ({} fans, read-only)",
                hw.display(),
                fans.len()
            );
            return ("yogafan".into(), fans);
        }
        log::trace!("fan probe: yogafan present but exposes no fan channels");
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
        .map(|id| {
            // The filename proves the channel is exposed. Preserve it even
            // when the first value read fails so diagnostics can report an
            // `unreadable` channel instead of silently dropping it.
            let input = read_u32(hw.join(format!("fan{id}_input"))).unwrap_or_else(|| {
                log::trace!("fan probe: {disp} fan{id}_input unreadable — keeping channel");
                0
            });
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
            FanCapability {
                id,
                title,
                min_rpm: min,
                max_rpm: max.max(min + 100),
                current_rpm: input,
            }
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

    fn test_gpu(slot: &str, vendor: &str, name: &str) -> PciGpu {
        PciGpu {
            slot: slot.into(),
            vendor: vendor.into(),
            device: "1234".into(),
            name: name.into(),
        }
    }

    #[test]
    fn discrete_gpu_selection_uses_pci_topology() {
        let apu_only = [test_gpu("0000:08:00.0", "0x1002", "Radeon Graphics")];
        assert!(select_discrete_gpu(&apu_only).is_none());
        let apu_without_boot_vga = [test_gpu("0000:08:00.0", "0x1002", "Radeon Vega Series")];
        assert!(select_discrete_gpu(&apu_without_boot_vga).is_none());

        let hybrid = [
            test_gpu("0000:08:00.0", "0x1002", "Radeon Graphics"),
            test_gpu("0000:01:00.0", "0x10de", "GeForce RTX GPU"),
        ];
        assert_eq!(
            select_discrete_gpu(&hybrid).map(|gpu| gpu.slot.as_str()),
            Some("0000:01:00.0")
        );

        let nvidia_only = [test_gpu("0000:01:00.0", "0x10de", "GeForce RTX GPU")];
        assert!(select_discrete_gpu(&nvidia_only).is_some());

        let amd_dgpu_only = [test_gpu(
            "0000:03:00.0",
            "0x1002",
            "Radeon RX 7700S",
        )];
        assert!(select_discrete_gpu(&amd_dgpu_only).is_some());

        let muxed_amd = [
            test_gpu("0000:08:00.0", "0x1002", "Radeon 780M"),
            test_gpu("0000:03:00.0", "0x1002", "Radeon RX 7700S"),
        ];
        assert_eq!(
            select_discrete_gpu(&muxed_amd).map(|gpu| gpu.slot.as_str()),
            Some("0000:03:00.0")
        );
    }

    #[test]
    fn gpu_name_cleanup_uses_model_not_vendor_tag() {
        assert_eq!(
            clean_gpu_name(
                "Advanced Micro Devices, Inc. [AMD/ATI] Cezanne [Radeon Vega Series / Radeon Vega Mobile Series] (rev c5)"
            ),
            "Radeon Vega Series / Radeon Vega Mobile Series"
        );
        assert_eq!(
            clean_gpu_name(
                "NVIDIA Corporation GB203M / GN22-X9 [GeForce RTX 5080 Max-Q / Mobile] (rev a1)"
            ),
            "GeForce RTX 5080 Max-Q / Mobile"
        );
    }

    #[test]
    fn fan_probe_keeps_exposed_unreadable_channel() {
        let dir = std::env::temp_dir().join(format!(
            "legion-device-fan-probe-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("fan2_input"), "not-a-number\n").unwrap();

        let fans = collect_fans_from_hwmon(&dir, None);
        assert_eq!(fans.len(), 1);
        assert_eq!(fans[0].id, 2);
        assert_eq!(fans[0].current_rpm, 0);

        std::fs::remove_dir_all(dir).unwrap();
    }
}
