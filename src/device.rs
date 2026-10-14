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

/// Structured lighting capability — derived from the HID probe.
/// Drives the Lighting UI: `Spectrum` gets every tab (incl. per-key),
/// `FourZone` gets the 4-zone page (`crate::fourzone`), `White` the notice page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LightingKind {
    /// Spectrum RGB (960-byte report; 048d:c197 and any ITE c1xx/c6xx/c9xx
    /// whose report descriptor confirms it) — per-key painting + 4 zones + logo.
    Spectrum,
    /// Legacy 4-zone RGB HID (33-byte report, e.g. 048d:c965). Driven by
    /// `crate::fourzone` (experimental), which re-checks the descriptor
    /// before every write. 048d:c193 ("Lenovo
    /// Lighting") is a separate, undocumented device and is NOT classed here.
    FourZone,
    /// No RGB HID — white backlight via fn+Q / kbd_backlight sysfs.
    White,
}

impl LightingKind {
    /// Whether the per-key painter is supported.
    pub fn per_key(&self) -> bool {
        matches!(self, LightingKind::Spectrum)
    }

    /// Lighting tab names this hardware should show, in order.
    /// FourZone returns an empty list: it is a single page (no tabs), driven
    /// by `crate::fourzone` rather than the Spectrum write path.
    pub fn tabs(&self) -> &'static [&'static str] {
        match self {
            LightingKind::Spectrum => &["keyboard", "front", "rear", "logo", "more"],
            LightingKind::FourZone | LightingKind::White => &[],
        }
    }

    /// Short nav subtitle for the Lighting rail entry.
    pub fn nav_subtitle(&self) -> &'static str {
        match self {
            LightingKind::Spectrum => "Keyboard, front and rear bars, logo, per-key",
            LightingKind::FourZone => "4-zone RGB keyboard (experimental)",
            LightingKind::White => "White backlight brightness",
        }
    }
}

/// How an ITE 048d lighting HID was classified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IteClass {
    /// 960-byte Spectrum report (Gen 7+ / Gen 10, e.g. c197, c195).
    Spectrum,
    /// 192-byte "old" Spectrum report (Legion 5/7 Gen 5-6). Detected only.
    SpectrumLegacy,
    /// 33-byte 4-zone report (Legion 5 2020-2024, IdeaPad Gaming, LOQ).
    FourZone,
    /// 65-byte LOQ 24-zone report (e.g. c693). Detected only.
    Loq24,
    /// 048d:c193 "Lenovo Lighting" — undocumented secondary device, never written.
    LenovoLighting,
    /// ITE lighting-range PID whose report size matches nothing we know.
    Unrecognised,
}

impl IteClass {
    fn rank(self) -> u8 {
        match self {
            IteClass::Spectrum => 0,
            IteClass::LenovoLighting => 1,
            IteClass::FourZone => 2,
            IteClass::SpectrumLegacy | IteClass::Loq24 => 3,
            IteClass::Unrecognised => 4,
        }
    }

    fn lighting_kind(self) -> LightingKind {
        match self {
            IteClass::Spectrum => LightingKind::Spectrum,
            IteClass::FourZone => LightingKind::FourZone,
            _ => LightingKind::White,
        }
    }
}

/// True for ITE 048d lighting-controller PIDs: c1xx (Gen 10+), c6xx (LOQ,
/// Legion 5 Gen 11) and c9xx (everything older). `pid` is 4 hex digits.
pub fn is_ite_lighting_pid(pid: &str) -> bool {
    let p = pid.to_ascii_lowercase();
    p.len() == 4
        && p.bytes().all(|b| b.is_ascii_hexdigit())
        && (p.starts_with("c1") || p.starts_with("c6") || p.starts_with("c9"))
}

/// Largest HID feature report in a report descriptor, in bytes, counting the
/// report-ID slot (so a 959-byte payload with ID 7 is 960, a 32-byte payload
/// with ID 0xCC is 33). Returns 0 when the descriptor has no feature items.
pub fn max_feature_report_len(desc: &[u8]) -> usize {
    let (mut size, mut count, mut id) = (0usize, 0usize, 0u8);
    let mut bits: Vec<(u8, usize)> = Vec::new();
    let mut i = 0;
    while i < desc.len() {
        let prefix = desc[i];
        if prefix == 0xFE {
            // Long item: [0xFE, len, tag, data..]
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
            (1, 0x7) => size = val,
            (1, 0x9) => count = val,
            (1, 0x8) => id = val as u8,
            (0, 0xB) => match bits.iter_mut().find(|(k, _)| *k == id) {
                Some((_, b)) => *b += size * count,
                None => bits.push((id, size * count)),
            },
            _ => {}
        }
        i += 1 + n;
    }
    bits.iter()
        .map(|(_, b)| b.div_ceil(8) + 1)
        .max()
        .unwrap_or(0)
}

/// Classify one ITE hidraw node by PID and (when readable) report descriptor.
/// Returns `None` for PIDs outside the ITE lighting ranges. Pure function so
/// the mapping is unit-testable without sysfs.
pub fn classify_ite(pid: &str, desc: Option<&[u8]>) -> Option<IteClass> {
    if !is_ite_lighting_pid(pid) {
        return None;
    }
    let pid = pid.to_ascii_lowercase();
    if pid == "c193" {
        return Some(IteClass::LenovoLighting);
    }
    // c197 is the known-good Spectrum PID: never demote it on descriptor quirks.
    if pid == "c197" {
        return Some(IteClass::Spectrum);
    }
    let class = match desc.map(max_feature_report_len) {
        Some(960) => IteClass::Spectrum,
        Some(192) => IteClass::SpectrumLegacy,
        Some(33) => IteClass::FourZone,
        Some(65) => IteClass::Loq24,
        // Descriptor unreadable: keep the historic PID-only behaviour.
        None if pid == "c965" => IteClass::FourZone,
        _ => IteClass::Unrecognised,
    };
    Some(class)
}

/// Map classified ITE devices (pid, class) to a lighting kind: Spectrum wins,
/// then FourZone; anything else (unknown, legacy, LOQ, c193) is `White`.
fn kind_from_ite(devs: &[(String, IteClass)]) -> LightingKind {
    devs.iter()
        .map(|(_, c)| c.lighting_kind())
        .min_by_key(|k| match k {
            LightingKind::Spectrum => 0,
            LightingKind::FourZone => 1,
            LightingKind::White => 2,
        })
        .unwrap_or(LightingKind::White)
}

/// Human label for the `capabilities.lighting` string.
fn ite_label(pid: &str, class: IteClass) -> String {
    match class {
        IteClass::Spectrum => format!("Spectrum RGB (048d:{pid})"),
        IteClass::LenovoLighting => format!("Lenovo Lighting (048d:{pid})"),
        IteClass::FourZone => format!("4-zone RGB (048d:{pid})"),
        IteClass::SpectrumLegacy => {
            format!("Legacy Spectrum 192 B (048d:{pid}) — detected, not supported")
        }
        IteClass::Loq24 => format!("LOQ 24-zone (048d:{pid}) — detected, not supported"),
        IteClass::Unrecognised => format!("Unrecognised ITE 048d:{pid}"),
    }
}

/// One entry per ITE PID (best class across its hidraw nodes), ordered
/// Spectrum first so the label string stays stable on c197 machines.
fn dedupe_ite(mut found: Vec<(String, IteClass)>) -> Vec<(String, IteClass)> {
    let mut best: Vec<(String, IteClass)> = Vec::new();
    for (pid, class) in found.drain(..) {
        match best.iter_mut().find(|(p, _)| *p == pid) {
            Some(slot) if class.rank() < slot.1.rank() => slot.1 = class,
            Some(_) => {}
            None => best.push((pid, class)),
        }
    }
    best.sort_by(|a, b| (a.1.rank(), &a.0).cmp(&(b.1.rank(), &b.0)));
    best
}

/// Enumerate hidraw nodes whose USB VID is 048d and PID is in the ITE
/// lighting ranges, classified by report descriptor.
fn scan_ite_hidraw() -> Vec<(String, IteClass)> {
    let Ok(entries) = std::fs::read_dir("/sys/class/hidraw") else {
        log::trace!("ite scan: /sys/class/hidraw unreadable");
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let Ok(dev) = std::fs::canonicalize(entry.path().join("device")) else {
            continue;
        };
        // HID_ID style dirname: 0003:048D:C197.0003
        let name = dev
            .file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let parts: Vec<&str> = name.split(':').collect();
        if parts.len() < 3 || parts[0] != "0003" || parts[1] != "048d" {
            continue;
        }
        let pid = parts[2].split('.').next().unwrap_or("").to_string();
        let desc = std::fs::read(dev.join("report_descriptor")).ok();
        if let Some(class) = classify_ite(&pid, desc.as_deref()) {
            log::debug!(
                "ite scan: {} 048d:{pid} → {class:?} (max feature report {} B)",
                entry.file_name().to_string_lossy(),
                desc.as_deref().map(max_feature_report_len).unwrap_or(0)
            );
            found.push((pid, class));
        }
    }
    dedupe_ite(found)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capabilities {
    pub fan_backend: String,
    pub fans: Vec<FanCapability>,
    pub lighting: String,
    pub lighting_kind: LightingKind,
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

/// True when DMI `sys_vendor` is readable and is not Lenovo. An unreadable
/// vendor (VM, container) is not treated as foreign hardware.
pub fn is_foreign_vendor(sys_vendor: &str) -> bool {
    let v = sys_vendor.trim();
    !v.is_empty() && !v.to_ascii_lowercase().starts_with("lenovo")
}

/// Cached vendor gate: false on non-Lenovo machines. Hardware writes (fans,
/// Spectrum HID) must check this before touching anything.
pub fn vendor_supported() -> bool {
    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SUPPORTED.get_or_init(|| !is_foreign_vendor(&read_dmi("sys_vendor").unwrap_or_default()))
}

/// Message shown by GUI/CLI when the vendor gate trips.
pub fn unsupported_vendor_message(sys_vendor: &str) -> String {
    format!(
        "Unsupported vendor \"{}\": Legion Control only supports Lenovo laptops, \
         so fan, lighting and power controls are disabled.",
        sys_vendor.trim()
    )
}

/// Device info for non-Lenovo hardware: no placeholder fans, no lighting,
/// no profile match, and a clear reason in `profile_source`.
fn unsupported_vendor_info(vendor: &str) -> DeviceInfo {
    let name = read_dmi("product_name").unwrap_or_else(|| "Unknown".into());
    let bios = read_dmi("bios_version").unwrap_or_else(|| "Unknown".into());
    let msg = unsupported_vendor_message(vendor);
    log::warn!("device detect: {msg}");
    DeviceInfo {
        model: name.clone(),
        machine_type: name.clone(),
        marketing_name: name,
        series: "Unsupported vendor".into(),
        bios_prefix: bios_prefix(&bios),
        bios_version: bios,
        ec_chip: "n/a".into(),
        cpu_model: read_cpu_model().unwrap_or_else(|| "Unknown".into()),
        gpu_model: "Unknown".into(),
        gen: 0,
        profile_matched: false,
        profile_source: format!("unsupported vendor ({})", vendor.trim()),
        profile_notes: msg,
        capabilities: Capabilities {
            fan_backend: "unsupported vendor".into(),
            fans: Vec::new(),
            lighting: "Unsupported vendor".into(),
            lighting_kind: LightingKind::White,
            peak_gpu_w: None,
            peak_gpu_source: "unavailable".into(),
            ppt_attrs: Vec::new(),
            platform_profiles: Vec::new(),
            has_custom_profile: false,
        },
    }
}

/// Model generation from DMI + the model table only — no GPU/capability
/// probe, so it is cheap for one-shot CLI paths. 0 = unmatched.
pub fn model_gen() -> u8 {
    static GEN: OnceLock<u8> = OnceLock::new();
    *GEN.get_or_init(|| {
        let bios = read_dmi("bios_version").unwrap_or_default();
        let (machine_type, marketing) = classify_dmi(
            &read_dmi("product_name").unwrap_or_default(),
            &read_dmi("product_version").unwrap_or_default(),
            &read_dmi("product_family").unwrap_or_default(),
            &read_dmi("product_sku").unwrap_or_default(),
        );
        models::lookup(&machine_type, &marketing, &bios_prefix(&bios)).map_or(0, |p| p.gen)
    })
}

fn detect_uncached() -> DeviceInfo {
    log::debug!("device detect: full probe starting");
    let dmi_vendor = read_dmi("sys_vendor").unwrap_or_default();
    if is_foreign_vendor(&dmi_vendor) {
        return unsupported_vendor_info(&dmi_vendor);
    }
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

    let model_match = models::lookup_match(&machine_type, &marketing, &bios_prefix);
    let profile = model_match.map(|m| m.profile);
    let family_only = model_match.is_some_and(|m| m.family_only);
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
                // Row notes describe a different machine type.
                if family_only { String::new() } else { p.notes.to_string() },
                if family_only {
                    format!("bios-prefix family ({}) · {}", p.bios_prefix, p.source)
                } else {
                    p.source.to_string()
                },
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
    let (lighting_kind, lighting) = probe_lighting();
    log::debug!("capability probe: lighting {lighting:?} (kind {lighting_kind:?})");
    let ppt_attrs = probe_ppt_attrs();
    // Known-dangerous profiles (see profile::DANGEROUS_PROFILES) are not offered.
    let platform_profiles = crate::profile::safe_choices();
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
        lighting_kind,
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
            "8050s",
            "8060s",
        ]
        .iter()
        .any(|marker| name.contains(marker)),
        "0x8086" => [
            "uhd graphics",
            "iris",
            "arc graphics",
            "integrated graphics",
        ]
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
    if let Some(pretty) = crate::gpu_ids::pretty_name(&card.vendor, &card.device) {
        return pretty;
    }
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
    let integrated = cards.iter().find(|card| {
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
            // lives in yogafan. Prefer yogafan whenever WMI is all-zero and
            // yogafan exposes channels; idle 0 rpm is a valid yogafan reading.
            let is_quirk_model = profile.is_some_and(|p| p.machine_type == Some("83JG") || p.bios_prefix == "R8CN")
                || read_dmi("product_name").as_deref() == Some("83JG")
                || read_dmi("bios_version").is_some_and(|b| b.starts_with("R8CN"));
            if is_quirk_model && fans.iter().all(|f| f.current_rpm == 0) {
                if let Some(yw) = crate::sensors::hwmon_by_name("yogafan") {
                    let yfans = collect_fans_from_hwmon(&yw, profile);
                    if !yfans.is_empty() {
                        log::debug!(
                            "fan probe: lenovo_wmi_other reads 0 RPM on quirk model — yogafan backend at {}",
                            yw.display()
                        );
                        return ("yogafan".into(), yfans);
                    }
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

    // EC debugfs RPM (daemon/root only): report the backend the RPM path
    // actually uses so `fan_backend` agrees with `fans::backend_name()`.
    // Channel ids/ranges still come from the profile/placeholder below.
    let ec_fallback = crate::fans::rpm_backend_name().filter(|name| name == "ec-fallback");

    // Profile fallbacks when no hwmon yet (daemon early boot, missing module).
    if let Some(p) = profile {
        log::debug!(
            "fan probe: no hwmon fans — using profile fallback ranges ({}){}",
            p.bios_prefix,
            if ec_fallback.is_some() {
                ", RPM via EC fallback"
            } else {
                ""
            }
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
        let backend =
            ec_fallback.unwrap_or_else(|| format!("profile-fallback ({})", p.bios_prefix));
        return (backend, fans);
    }

    log::warn!("fan probe: no hwmon and no profile — using static placeholder fan ranges");
    (
        ec_fallback.unwrap_or_else(|| "none".into()),
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
    } else if lower
        .strip_prefix("fan")
        .is_some_and(|n| n.trim().parse::<u8>().is_ok())
    {
        // legion_hwmon "Fan 1"/"Fan 3" carry no role — use the id's title
        // instead of "Fan 1 fan".
        fan_title(id).into()
    } else if lower.ends_with("fan") {
        t.into()
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

fn probe_lighting() -> (LightingKind, String) {
    // Debug/testing override — lets any UI branch be exercised on hardware
    // that doesn't have it: LEGION_LIGHTING_OVERRIDE=spectrum|fourzone|white
    if let Ok(ov) = std::env::var("LEGION_LIGHTING_OVERRIDE") {
        let kind = match ov.trim().to_ascii_lowercase().as_str() {
            "spectrum" => Some(LightingKind::Spectrum),
            "fourzone" | "4zone" => Some(LightingKind::FourZone),
            "white" => Some(LightingKind::White),
            _ => None,
        };
        if let Some(kind) = kind {
            log::warn!("lighting probe: override via LEGION_LIGHTING_OVERRIDE={ov:?} → {kind:?}");
            return (
                kind,
                format!("Override ({kind:?}) via LEGION_LIGHTING_OVERRIDE"),
            );
        }
        log::debug!("lighting probe: LEGION_LIGHTING_OVERRIDE={ov:?} not recognized — ignored");
    }

    let found = scan_ite_hidraw();
    let kind = kind_from_ite(&found);
    let labels: Vec<String> = found.iter().map(|(p, c)| ite_label(p, *c)).collect();
    if found.is_empty() {
        log::debug!("lighting probe: none detected → {kind:?}");
    } else {
        log::debug!("lighting probe: resolved {kind:?} from {labels:?}");
    }
    (
        kind,
        if labels.is_empty() {
            "None detected".into()
        } else {
            labels.join(" · ")
        },
    )
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
            return line.split(':').nth(1).map(|s| {
                let raw = s.trim();
                crate::cpu_ids::display_name(raw)
            });
        }
    }
    log::trace!("cpu model: /proc/cpuinfo has no \"model name\" line");
    None
}

fn detect_ec_chip(fan_backend: &str) -> String {
    match fan_backend {
        "lenovo_wmi_other" => "ITE (via lenovo_wmi_other)".into(),
        "legion_hwmon" => "ITE IT5508 (legion_hwmon)".into(),
        "ec-fallback" => "Unknown (EC debugfs fallback)".into(),
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

    /// Minimal descriptor: vendor page, one feature report `id` with `payload` bytes.
    fn desc_with_feature(id: u8, payload: u16) -> Vec<u8> {
        let mut d = vec![0x06, 0x89, 0xff, 0x09, 0x07, 0xa1, 0x01, 0x85, id];
        d.extend([
            0x75,
            0x08,
            0x96,
            payload as u8,
            (payload >> 8) as u8,
            0xb1,
            0x00,
            0xc0,
        ]);
        d
    }

    #[test]
    fn vendor_gate_only_rejects_readable_non_lenovo_vendors() {
        assert!(is_foreign_vendor("HP"));
        assert!(is_foreign_vendor("ASUSTeK COMPUTER INC."));
        assert!(!is_foreign_vendor("LENOVO"));
        assert!(!is_foreign_vendor("Lenovo\n"));
        assert!(!is_foreign_vendor(""), "unreadable vendor is not foreign");
        assert!(unsupported_vendor_message("HP").contains("Unsupported vendor"));
    }

    #[test]
    fn ite_pid_ranges() {
        for p in ["c197", "C195", "c193", "c693", "c965", "c100"] {
            assert!(is_ite_lighting_pid(p), "{p}");
        }
        for p in ["c53a", "5003", "c2", "c1970", "zzzz", "c8ff", "c0ff"] {
            assert!(!is_ite_lighting_pid(p), "{p}");
        }
    }

    #[test]
    fn feature_report_len_from_descriptor() {
        assert_eq!(max_feature_report_len(&desc_with_feature(7, 959)), 960);
        assert_eq!(max_feature_report_len(&desc_with_feature(7, 191)), 192);
        assert_eq!(max_feature_report_len(&desc_with_feature(0xcc, 32)), 33);
        assert_eq!(max_feature_report_len(&[]), 0);
        // Truncated item must not panic.
        assert_eq!(max_feature_report_len(&[0x96, 0xbf]), 0);
        // Real c197 hidraw descriptor head (0x5a: 16 B, 0x07: 959 B).
        let c197 = [
            0x06, 0x89, 0xff, 0x09, 0x10, 0xa1, 0x01, 0x85, 0x5a, 0x09, 0x01, 0x15, 0x00, 0x26,
            0xff, 0x00, 0x75, 0x08, 0x95, 0x10, 0xb1, 0x00, 0xc0, 0x06, 0x89, 0xff, 0x09, 0x07,
            0xa1, 0x01, 0x85, 0x07, 0x09, 0x01, 0x15, 0x00, 0x26, 0xff, 0x00, 0x75, 0x08, 0x96,
            0xbf, 0x03, 0xb1, 0x00, 0xc0,
        ];
        assert_eq!(max_feature_report_len(&c197), 960);
    }

    #[test]
    fn classify_ite_by_size_and_pid() {
        let spec = desc_with_feature(7, 959);
        // Spectrum is recognised by report size, not by PID alone.
        assert_eq!(classify_ite("c197", Some(&spec)), Some(IteClass::Spectrum));
        assert_eq!(classify_ite("c195", Some(&spec)), Some(IteClass::Spectrum));
        assert_eq!(classify_ite("c998", Some(&spec)), Some(IteClass::Spectrum));
        assert_eq!(
            classify_ite("c968", Some(&desc_with_feature(7, 191))),
            Some(IteClass::SpectrumLegacy)
        );
        assert_eq!(
            classify_ite("c985", Some(&desc_with_feature(0xcc, 32))),
            Some(IteClass::FourZone)
        );
        assert_eq!(
            classify_ite("c693", Some(&desc_with_feature(0, 64))),
            Some(IteClass::Loq24)
        );
        // Unknown size / unreadable descriptor → reported, not hidden as White.
        assert_eq!(
            classify_ite("c195", Some(&desc_with_feature(1, 10))),
            Some(IteClass::Unrecognised)
        );
        assert_eq!(classify_ite("c195", None), Some(IteClass::Unrecognised));
        // Historic PID-only fallbacks.
        assert_eq!(classify_ite("c197", None), Some(IteClass::Spectrum));
        assert_eq!(classify_ite("c965", None), Some(IteClass::FourZone));
        // c193 is a separate device even if its descriptor looks Spectrum-like.
        assert_eq!(
            classify_ite("c193", Some(&spec)),
            Some(IteClass::LenovoLighting)
        );
        // Not an ITE lighting PID.
        assert_eq!(classify_ite("c53a", Some(&spec)), None);
    }

    #[test]
    fn lighting_kind_and_label_from_ite() {
        let k = |v: &[(&str, IteClass)]| {
            kind_from_ite(
                &v.iter()
                    .map(|(p, c)| (p.to_string(), *c))
                    .collect::<Vec<_>>(),
            )
        };
        assert_eq!(k(&[]), LightingKind::White);
        assert_eq!(k(&[("c197", IteClass::Spectrum)]), LightingKind::Spectrum);
        assert_eq!(k(&[("c965", IteClass::FourZone)]), LightingKind::FourZone);
        // c193, unknown, legacy, LOQ never enable a write UI.
        for c in [
            IteClass::LenovoLighting,
            IteClass::Unrecognised,
            IteClass::SpectrumLegacy,
            IteClass::Loq24,
        ] {
            assert_eq!(k(&[("c195", c)]), LightingKind::White, "{c:?}");
        }
        assert_eq!(
            k(&[
                ("c193", IteClass::LenovoLighting),
                ("c197", IteClass::Spectrum)
            ]),
            LightingKind::Spectrum
        );
        assert_eq!(
            ite_label("c195", IteClass::Unrecognised),
            "Unrecognised ITE 048d:c195"
        );
        assert_eq!(
            ite_label("c197", IteClass::Spectrum),
            "Spectrum RGB (048d:c197)"
        );
        assert_eq!(
            ite_label("c193", IteClass::LenovoLighting),
            "Lenovo Lighting (048d:c193)"
        );
    }

    #[test]
    fn dedupe_keeps_best_class_per_pid_spectrum_first() {
        let out = dedupe_ite(vec![
            ("c193".into(), IteClass::LenovoLighting),
            ("c995".into(), IteClass::Unrecognised),
            ("c197".into(), IteClass::Spectrum),
            ("c995".into(), IteClass::FourZone),
        ]);
        assert_eq!(
            out,
            vec![
                ("c197".to_string(), IteClass::Spectrum),
                ("c193".to_string(), IteClass::LenovoLighting),
                ("c995".to_string(), IteClass::FourZone),
            ]
        );
    }

    #[test]
    fn lighting_kind_ui_surface() {
        assert!(LightingKind::Spectrum.per_key());
        assert!(!LightingKind::FourZone.per_key());
        assert!(!LightingKind::White.per_key());

        assert_eq!(
            LightingKind::Spectrum.tabs(),
            &["keyboard", "front", "rear", "logo", "more"]
        );
        // FourZone: write path not implemented yet → notice page, no tabs.
        assert!(LightingKind::FourZone.tabs().is_empty());
        assert!(LightingKind::White.tabs().is_empty());

        assert!(LightingKind::Spectrum.nav_subtitle().contains("per-key"));
        assert!(LightingKind::FourZone.nav_subtitle().contains("4-zone"));
        assert!(LightingKind::White.nav_subtitle().contains("White"));
    }

    #[test]
    fn detect_this_machine() {
        // Live-hardware assertion: skip on CI runners / non-Lenovo hosts.
        let vendor = std::fs::read_to_string("/sys/class/dmi/id/sys_vendor").unwrap_or_default();
        if vendor.trim() != "LENOVO" {
            eprintln!("skipping: not a Lenovo machine ({:?})", vendor.trim());
            return;
        }
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
        // legion_hwmon labels (83F6): never "Fan 1 fan".
        assert_eq!(pretty_fan_label("Fan 1", 1), "CPU fan");
        assert_eq!(pretty_fan_label("Fan 2", 2), "GPU fan");
        assert_eq!(pretty_fan_label("Fan 3", 3), "Fan 3");
        assert_eq!(pretty_fan_label("Chassis Fan", 9), "Chassis Fan");
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

        let amd_dgpu_only = [test_gpu("0000:03:00.0", "0x1002", "Radeon RX 7700S")];
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
    fn cpu_name_uses_sku_map() {
        assert_eq!(
            crate::cpu_ids::display_name("AMD Ryzen 9 9955HX3D 16-Core Processor"),
            "AMD Ryzen 9 9955HX3D"
        );
    }

    #[test]
    fn gpu_name_prefers_yaml_pci_map() {
        let card = PciGpu {
            slot: "0000:01:00.0".into(),
            vendor: "0x10de".into(),
            device: "2C19".into(),
            name: String::new(),
        };
        assert_eq!(
            resolve_gpu_name(&card),
            "NVIDIA GeForce RTX 5080 Laptop GPU"
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
        let dir =
            std::env::temp_dir().join(format!("legion-device-fan-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("fan2_input"), "not-a-number\n").unwrap();

        let fans = collect_fans_from_hwmon(&dir, None);
        assert_eq!(fans.len(), 1);
        assert_eq!(fans[0].id, 2);
        assert_eq!(fans[0].current_rpm, 0);

        std::fs::remove_dir_all(dir).unwrap();
    }
}
