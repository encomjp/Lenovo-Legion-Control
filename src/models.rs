//! Known Legion / LOQ / IdeaPad Gaming model profiles.
//!
//! Sourced from:
//! - LenovoLegionLinux `legion-laptop.c` DMI allowlist (BIOS prefix → family)
//! - Gen 10 forks (ChaoticSi1ence / gluceri) for SMCN / Q7CN
//! - Lenovo PSREF peak TGP notes where useful
//!
//! Probe (sysfs) always wins for live limits; these entries fill marketing
//! names, generation, expected fan layout, and quirks when DMI matches.

use serde::{Deserialize, Serialize};

/// How many cooling fans this chassis typically exposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FanLayout {
    Two,
    Three,
    Unknown,
}

/// Static knowledge about a machine family / SKU.
#[derive(Debug, Clone, Copy)]
pub struct ModelProfile {
    /// Machine type (DMI product_name on modern Legion), e.g. `"83RU"`.
    pub machine_type: Option<&'static str>,
    /// BIOS version prefix (first 4 chars), e.g. `"SMCN"`.
    pub bios_prefix: &'static str,
    /// Marketing / family name shown to users.
    pub marketing: &'static str,
    /// Short series label.
    pub series: &'static str,
    /// Approximate generation (5–10). `0` = unknown / older.
    pub gen: u8,
    pub fans: FanLayout,
    /// Fallback RPM ranges `(fan_id, min, max)` when hwmon has no min/max.
    pub fan_rpm_fallback: &'static [(u8, u32, u32)],
    /// Notes / quirks (Custom mode, WMI, etc.).
    pub notes: &'static str,
    /// Where this entry was curated from.
    pub source: &'static str,
}

const FALLBACK_2FAN: &[(u8, u32, u32)] = &[(1, 0, 5500), (2, 0, 5500)];
const FALLBACK_3FAN_GEN10: &[(u8, u32, u32)] = &[(1, 1700, 5200), (2, 1700, 5400), (4, 1500, 6500)];
const FALLBACK_3FAN_GENERIC: &[(u8, u32, u32)] = &[(1, 0, 5500), (2, 0, 5500), (4, 0, 6000)];

/// Curated profiles — match by machine type first, then BIOS prefix.
pub static MODEL_PROFILES: &[ModelProfile] = &[
    // ── Gen 10 Pro 7 (upstream WMI / forks) ─────────────────────────────
    ModelProfile {
        machine_type: Some("83RU"),
        bios_prefix: "SMCN",
        marketing: "Legion Pro 7 16AFR10H",
        series: "Legion Pro 7 Gen 10 (AMD)",
        gen: 10,
        fans: FanLayout::Three,
        fan_rpm_fallback: FALLBACK_3FAN_GEN10,
        notes: "Ryzen AI Max / 9955HX class · RTX 5070 Ti 140W or 5080 175W (PSREF) · \
                Spectrum RGB · lenovo_wmi_other on kernel 6.14+",
        source: "PSREF · ChaoticSi1ence Gen10 fork · live 83RU probe",
    },
    ModelProfile {
        machine_type: Some("83F5"),
        bios_prefix: "Q7CN",
        marketing: "Legion Pro 7 16IAX10H",
        series: "Legion Pro 7 Gen 10 (Intel)",
        gen: 10,
        fans: FanLayout::Three,
        fan_rpm_fallback: FALLBACK_3FAN_GEN10,
        notes: "Core Ultra 200HX · up to RTX 5090 · 3 fans (CPU/GPU/Aux) · WMI3",
        source: "LenovoLegionLinux #385 · ChaoticSi1ence",
    },
    ModelProfile {
        machine_type: Some("83KY"),
        bios_prefix: "Q7CN",
        marketing: "Legion 7i 16IAX10",
        series: "Legion 7 Gen 10 (Intel)",
        gen: 10,
        fans: FanLayout::Three,
        fan_rpm_fallback: FALLBACK_3FAN_GENERIC,
        notes: "Gen 10 7-series sibling · EC dumps in LLL #409",
        source: "LenovoLegionLinux #409",
    },
    // ── Gen 9 Legion Y7000P (Intel 14th Gen) ───────────────────────────
    ModelProfile {
        machine_type: Some("83DG"),
        bios_prefix: "NMCN",
        marketing: "Legion Y7000P IRX9",
        series: "Legion Y7000P Gen 9 (Intel)",
        gen: 9,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "Intel 14th Gen HX (i7-14700HX) · RTX 4060/4070 140W (PSREF) · Dual fan · WMI3 platform profiles",
        source: "fleet telemetry (83DG) · Lenovo PSREF",
    },
    // ── Gen 9 Legion 9 (Flagship Liquid-Cooled 3-Fan) ───────────────────
    ModelProfile {
        machine_type: Some("83G0"),
        bios_prefix: "NXCN",
        marketing: "Legion 9 16IRX9",
        series: "Legion 9 Gen 9 (Intel)",
        gen: 9,
        fans: FanLayout::Three,
        fan_rpm_fallback: FALLBACK_3FAN_GENERIC,
        notes: "Intel Core i9-14900HX · RTX 4080/4090 175W (PSREF) · Integrated liquid loop + 3 cooling fans",
        source: "LenovoLegionLinux #342 · Lenovo PSREF (83G0)",
    },
    // ── LenovoLegionLinux BIOS-prefix map (examples from driver comments) ─
    ModelProfile {
        machine_type: None,
        bios_prefix: "GKCN",
        marketing: "Legion 5 / 5 Pro / 7 (2021)",
        series: "Legion Gen 6",
        gen: 6,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "Classic legion-laptop EC map · Family e.g. 15ACH6H",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "EFCN",
        marketing: "Legion 5 / related (EFCN)",
        series: "Legion",
        gen: 6,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "Same EC family as GKCN in LLL",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "FSCN",
        marketing: "Legion (FSCN)",
        series: "Legion",
        gen: 6,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "HHCN",
        marketing: "Legion (HHCN)",
        series: "Legion",
        gen: 6,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "H1CN",
        marketing: "Legion (H1CN)",
        series: "Legion",
        gen: 6,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "JUCN",
        marketing: "Legion (JUCN)",
        series: "Legion",
        gen: 7,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "G9CN",
        marketing: "Legion (G9CN)",
        series: "Legion",
        gen: 6,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "God Mode often needs G9CN ≥24 on Windows LLT",
        source: "LenovoLegionLinux · Legion Toolkit notes",
    },
    ModelProfile {
        machine_type: Some("82RF"),
        bios_prefix: "J2CN",
        marketing: "Legion 5 Pro 16IAH7H",
        series: "Legion 5 Pro Gen 7 (Intel)",
        gen: 7,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "Intel 12th Gen (i7-12700H) · up to RTX 3070 Ti 150W · Dual fan",
        source: "LenovoLegionLinux legion-laptop.c · PSREF",
    },
    ModelProfile {
        machine_type: Some("82RB"),
        bios_prefix: "J1CN",
        marketing: "Legion 5 15IAH7 / 15IAH7H",
        series: "Legion 5 Gen 7 (Intel)",
        gen: 7,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "Intel 12th Gen · up to RTX 3060 140W · Dual fan",
        source: "LenovoLegionLinux legion-laptop.c · PSREF",
    },
    ModelProfile {
        machine_type: Some("82WR"),
        bios_prefix: "KWCN",
        marketing: "Legion Pro 7 16IRX8H (2023)",
        series: "Legion Pro 7 Gen 8 (Intel)",
        gen: 8,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "Intel 13th Gen HX · up to RTX 4090 175W · Dual fan (Coldfront 5.0)",
        source: "LenovoLegionLinux legion-laptop.c · PSREF",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "G8CN",
        marketing: "Legion 5 15IMH6 / Pro 5 16IRX9",
        series: "Legion 5",
        gen: 6,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "Also used for some Pro 5 16IRX9 (N0CN sibling map)",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: Some("83DF"),
        bios_prefix: "N0CN",
        marketing: "Legion Pro 5 16IRX9",
        series: "Legion Pro 5 Gen 9",
        gen: 9,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "Model 83DF",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: Some("82WM"),
        bios_prefix: "LPCN",
        marketing: "Legion Pro 5 16ARX8 (2023) / R9000P",
        series: "Legion Pro 5 Gen 8 (AMD)",
        gen: 8,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "Ryzen 7 7745HX / Ryzen 9 7945HX · up to RTX 4070 140W",
        source: "LenovoLegionLinux legion-laptop.c · PSREF",
    },
    ModelProfile {
        machine_type: Some("82Y9"),
        bios_prefix: "M3CN",
        marketing: "Legion Slim 5 16APH8 (2023)",
        series: "Legion Slim 5 Gen 8 (AMD)",
        gen: 8,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "Ryzen 7 7840HS · RTX 4060",
        source: "LenovoLegionLinux legion-laptop.c · PSREF",
    },
    ModelProfile {
        machine_type: Some("82Y3"),
        bios_prefix: "M0CN",
        marketing: "Legion Slim 7 16IRH8 (2023)",
        series: "Legion Slim 7 Gen 8 (Intel)",
        gen: 8,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "Intel 13th Gen · RTX 4070",
        source: "LenovoLegionLinux legion-laptop.c · PSREF",
    },
    ModelProfile {
        machine_type: Some("82Y4"),
        bios_prefix: "M1CN",
        marketing: "Legion Slim 7 16APH8 (2023)",
        series: "Legion Slim 7 Gen 8 (AMD)",
        gen: 8,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "Ryzen 7 7840HS + RTX 4060",
        source: "LenovoLegionLinux legion-laptop.c · PSREF",
    },
    ModelProfile {
        machine_type: Some("82YA"),
        bios_prefix: "M2CN",
        marketing: "Legion Slim 5 16IRH8 (2023)",
        series: "Legion Slim 5 Gen 8 (Intel)",
        gen: 8,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "Intel 13th Gen · RTX 4070",
        source: "LenovoLegionLinux legion-laptop.c · PSREF",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "M6CN",
        marketing: "Legion Slim / related (M6CN)",
        series: "Legion Slim",
        gen: 8,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: Some("83DH"),
        bios_prefix: "NRCN",
        marketing: "Legion Slim 5 16AHP9 (2024)",
        series: "Legion Slim 5 Gen 9",
        gen: 9,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "Model 83DH",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "KFCN",
        marketing: "Legion (KFCN)",
        series: "Legion",
        gen: 7,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "HACN",
        marketing: "Legion (HACN)",
        series: "Legion",
        gen: 7,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "K9CN",
        marketing: "Legion (K9CN)",
        series: "Legion",
        gen: 7,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "EUCN",
        marketing: "Legion (EUCN)",
        series: "Legion",
        gen: 6,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "DMCN",
        marketing: "Legion (DMCN)",
        series: "Legion",
        gen: 8,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "KHCN",
        marketing: "Legion (KHCN)",
        series: "Legion",
        gen: 8,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "JVCN",
        marketing: "Legion (JVCN)",
        series: "Legion",
        gen: 8,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "Shares map with KHCN in LLL",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "NSCN",
        marketing: "Legion (NSCN)",
        series: "Legion",
        gen: 7,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "K1CN",
        marketing: "Legion (K1CN)",
        series: "Legion",
        gen: 7,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    // Older / IdeaPad Gaming / LOQ
    ModelProfile {
        machine_type: None,
        bios_prefix: "BHCN",
        marketing: "Legion 5i / Y7000 2019 (PG0)",
        series: "Legion Gen 5",
        gen: 5,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "Older EC — limited fan control on Windows LLT",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "4GCN",
        marketing: "Legion Y720",
        series: "Legion Y",
        gen: 0,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "9VCN",
        marketing: "Legion Y7000p-1060",
        series: "Legion Y",
        gen: 0,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "JYCN",
        marketing: "Legion Y9000X",
        series: "Legion Y",
        gen: 0,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "BVCN",
        marketing: "Legion Y740-15IRH",
        series: "Legion Y",
        gen: 0,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "e.g. GTX 1660 class",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "8JCN",
        marketing: "Legion Y7000 (older)",
        series: "Legion Y",
        gen: 0,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: Some("82EY"),
        bios_prefix: "FCCN",
        marketing: "IdeaPad Gaming 3 15ARH05",
        series: "IdeaPad Gaming Gen 5 (AMD)",
        gen: 5,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "AMD Ryzen 4000H · GTX 1650 / 1650 Ti · IdeaPad Gaming — partial Legion EC support",
        source: "LenovoLegionLinux legion-laptop.c · PSREF",
    },
    ModelProfile {
        machine_type: Some("82K2"),
        bios_prefix: "H3CN",
        marketing: "IdeaPad Gaming 3 15ACH6",
        series: "IdeaPad Gaming Gen 6 (AMD)",
        gen: 6,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "AMD Ryzen 5000H · RTX 2050 (70W) / 3050 (85W) · Dual fan · EC fallback RPM (0x06/0x07)",
        source: "fleet telemetry (82K2) · LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "H4CN",
        marketing: "IdeaPad Gaming 3 15ARH05 (8K21)",
        series: "IdeaPad Gaming Gen 5 (AMD)",
        gen: 5,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: Some("82SB"),
        bios_prefix: "JNCN",
        marketing: "IdeaPad Gaming 3 15ARH7 (2022)",
        series: "IdeaPad Gaming Gen 7 (AMD)",
        gen: 7,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "Ryzen 6000H · RTX 3050 / 3050 Ti",
        source: "LenovoLegionLinux legion-laptop.c · PSREF",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "E9CN",
        marketing: "IdeaPad Gaming / related (E9CN)",
        series: "IdeaPad Gaming",
        gen: 0,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: Some("82XV"),
        bios_prefix: "LZCN",
        marketing: "LOQ 15IRH8",
        series: "LOQ Gen 8 (Intel)",
        gen: 8,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "Intel 13th Gen · RTX 3050 / 4050 / 4060 · LOQ shares Legion WMI subset",
        source: "LenovoLegionLinux legion-laptop.c · PSREF",
    },
    ModelProfile {
        machine_type: Some("83DV"),
        bios_prefix: "NECN",
        marketing: "LOQ 15IRX9",
        series: "LOQ Gen 9 (Intel)",
        gen: 9,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "Intel 13th/14th Gen HX · RTX 3050 / 4050 / 4060",
        source: "LenovoLegionLinux legion-laptop.c · PSREF",
    },
    ModelProfile {
        machine_type: Some("83DX"),
        bios_prefix: "NZCN",
        marketing: "LOQ 15AHP9",
        series: "LOQ Gen 9 (AMD)",
        gen: 9,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "Ryzen 7 8845HS · RTX 4050 / 4060",
        source: "LenovoLegionLinux legion-laptop.c · PSREF",
    },
    ModelProfile {
        machine_type: Some("83JG"),
        bios_prefix: "R8CN",
        marketing: "LOQ 15AHP10",
        series: "LOQ Gen 10 (AMD)",
        gen: 10,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "2025 LOQ (Ryzen 200-series). EC IT5508 locked: lenovo_wmi_other hwmon reads 0 RPM, ACPI temp paths error -5 (WMI3 works). RPM lives in yogafan hwmon (EC 0xFE/0xFF via \\_SB.PCI0.LPC0.EC0.FANS/FA2S, 16-bit). PPT: SPL 25-48 W, SPPT 35-43 W, FPPT 45-53 W.",
        source: "fleet telemetry (83JG) + LenovoLegionLinux #384/#453/#467 + kernel yogafan.c",
    },
    ModelProfile {
        machine_type: Some("83JE"),
        bios_prefix: "R3CN",
        marketing: "LOQ 15IRX10",
        series: "LOQ Gen 10 (Intel)",
        gen: 10,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "Intel Core 13th/14th Gen HX · RTX 5050 / 5060",
        source: "LenovoLegionLinux legion-laptop.c · PSREF",
    },
];

/// Match a profile: machine type → BIOS prefix → marketing substring.
pub fn lookup(
    machine_type: &str,
    marketing: &str,
    bios_prefix: &str,
) -> Option<&'static ModelProfile> {
    let mt = machine_type.trim();
    let mkt = marketing.trim();
    let bios = bios_prefix.trim().to_ascii_uppercase();
    log::debug!("model lookup: mt={mt:?} marketing={mkt:?} bios_prefix={bios:?}");

    // Tier 1: exact machine-type row.
    if !mt.is_empty() && mt != "Unknown" {
        for p in MODEL_PROFILES.iter() {
            if p.machine_type.is_some_and(|m| m.eq_ignore_ascii_case(mt)) {
                log::trace!(
                    "model lookup: row {} machine_type == {mt:?} → match",
                    p.bios_prefix
                );
                log::info!(
                    "model profile: machine type {mt} → {} ({} gen {}, source {})",
                    p.marketing,
                    p.series,
                    p.gen,
                    p.source
                );
                return Some(p);
            }
            log::trace!(
                "model lookup: row {} machine_type {:?} ≠ {mt:?} — no match",
                p.bios_prefix,
                p.machine_type
            );
        }
        log::debug!("model lookup: tier 1 exhausted — no row for machine type {mt:?}");
    } else {
        log::debug!("model lookup: machine type {mt:?} unusable — tier 1 skipped");
    }

    if bios.len() >= 4 {
        let prefix = &bios[..4];
        // Prefer MT-tagged entries for this BIOS family when marketing hints match.
        for p in MODEL_PROFILES.iter() {
            if !p.bios_prefix.eq_ignore_ascii_case(prefix) {
                log::trace!(
                    "model lookup: row {} bios prefix ≠ {prefix:?} — skipped (tier 2a)",
                    p.bios_prefix
                );
                continue;
            }
            log::trace!(
                "model lookup: row {} bios prefix matches {prefix:?} (MT-tagged: {})",
                p.bios_prefix,
                p.machine_type.is_some()
            );
            if p.machine_type.is_some()
                && !mkt.is_empty()
                && mkt
                    .to_ascii_lowercase()
                    .contains(&p.marketing.to_ascii_lowercase())
            {
                log::info!(
                    "model profile: bios prefix {prefix} + MT tag + marketing hint → {} (source {})",
                    p.marketing,
                    p.source
                );
                return Some(p);
            }
            let hint = p.marketing;
            log::trace!(
                "model lookup: tier-2a miss (mt_tagged={}, marketing={mkt:?}, hint={hint:?})",
                p.machine_type.is_some(),
            );
        }
        log::trace!("model lookup: no MT-tagged {prefix:?} row matched the marketing hint");

        for p in MODEL_PROFILES.iter() {
            if p.bios_prefix.eq_ignore_ascii_case(prefix) {
                log::info!(
                    "model profile: bios prefix {prefix} → {} ({} gen {}, source {})",
                    p.marketing,
                    p.series,
                    p.gen,
                    p.source
                );
                return Some(p);
            }
            log::trace!(
                "model lookup: row {} bios prefix ≠ {prefix:?} — no match",
                p.bios_prefix
            );
        }
        log::debug!("model lookup: tier 2 exhausted — no row for bios prefix {prefix:?}");
    } else {
        log::debug!("model lookup: bios prefix {bios:?} too short — tier 2 skipped");
    }

    // Fuzzy marketing match against known names.
    let mkt_l = mkt.to_ascii_lowercase();
    for p in MODEL_PROFILES.iter() {
        let name = p.marketing.to_ascii_lowercase();
        let substr_hit = !mkt_l.is_empty() && (mkt_l.contains(&name) || name.contains(&mkt_l));
        let alias_hit = (mkt_l.contains("16afr10h") && p.machine_type == Some("83RU"))
            || (mkt_l.contains("16iax10h") && p.machine_type == Some("83F5"));
        if substr_hit || alias_hit {
            log::debug!(
                "model lookup: fuzzy match {mkt:?} → {} ({})",
                p.marketing,
                if substr_hit {
                    "substring"
                } else {
                    "gen10 model alias"
                }
            );
            return Some(p);
        }
        log::trace!(
            "model lookup: row {} fuzzy no-match (marketing {name:?} vs input {mkt:?})",
            p.bios_prefix
        );
    }

    log::warn!("model lookup: no profile for mt={mt:?} marketing={mkt:?} bios_prefix={bios:?}");
    None
}

/// PSREF-style peak TGP guess from GPU marketing string (W). Probe still preferred.
pub fn expected_tgp_from_gpu_name(gpu: &str) -> Option<u32> {
    let g = gpu.to_ascii_lowercase();
    let tgp =
        if g.contains("5090") || g.contains("5080") || g.contains("4090") || g.contains("4080") {
            Some(175) // Pro 7 Gen 10 laptop SKUs commonly 175W class
        } else if g.contains("5070 ti") || g.contains("5070ti") {
            Some(140)
        } else if g.contains("5070") || g.contains("5060") {
            Some(115)
        } else if g.contains("5050") {
            Some(100) // LOQ 15 Gen 10 RTX 5050
        } else if g.contains("4070 ti") || g.contains("4070ti") {
            Some(150)
        } else if g.contains("4070") || g.contains("4060") {
            Some(140)
        } else if g.contains("4050") {
            Some(115)
        } else if g.contains("3080 ti") || g.contains("3080ti") {
            Some(175)
        } else if g.contains("3080") {
            Some(165)
        } else if g.contains("3070 ti") || g.contains("3070ti") {
            Some(150)
        } else if g.contains("3070") {
            Some(140)
        } else if g.contains("3060") {
            Some(130)
        } else if g.contains("3050 ti") || g.contains("3050ti") || g.contains("3050") {
            Some(95)
        } else if g.contains("2080") {
            Some(150)
        } else if g.contains("2070") {
            Some(115)
        } else if g.contains("2060") {
            Some(115)
        } else if g.contains("2050") {
            Some(70)
        } else if g.contains("1660 ti") || g.contains("1660ti") || g.contains("1660") {
            Some(80)
        } else if g.contains("1650 ti") || g.contains("1650ti") || g.contains("1650") {
            Some(50)
        } else if g.contains("radeon") {
            // Radeon RX 7000M/8000M mobile SKUs (LOQ/Legion AMD dGPUs).
            // PSREF peak-power bands: 7800M/8800M ≈ 180 W, 7700M/8700M
            // ≈ 145 W, 7600M/8600M ≈ 90 W, 7400M/890M ≈ 65 W.
            if g.contains("7800m") || g.contains("8800m") || g.contains("7900m") {
                Some(180)
            } else if g.contains("7700m") || g.contains("8700m") {
                Some(145)
            } else if g.contains("7600m") || g.contains("8600m") {
                Some(90)
            } else if g.contains("7400m") || g.contains("890m") || g.contains("8050m") {
                Some(65)
            } else {
                None
            }
        } else {
            None
        };
    log::trace!("tgp heuristic: {gpu:?} → {tgp:?} W");
    tgp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lookup_specific_models() {
        // 83RU (Legion Pro 7 16AFR10H)
        let p = lookup("83RU", "Legion Pro 7 16AFR10H", "SMCN").expect("83RU");
        assert_eq!(p.gen, 10);
        assert_eq!(p.fans, FanLayout::Three);

        // 83DG (Legion Y7000P IRX9)
        let p = lookup("83DG", "Legion Y7000P IRX9", "NMCN").expect("83DG");
        assert_eq!(p.gen, 9);
        assert_eq!(p.fans, FanLayout::Two);

        // 82K2 (IdeaPad Gaming 3 15ACH6)
        let p = lookup("82K2", "IdeaPad Gaming 3 15ACH6", "H3CN").expect("82K2");
        assert_eq!(p.gen, 6);
        assert_eq!(p.fans, FanLayout::Two);

        // 83JG (LOQ 15AHP10)
        let p = lookup("83JG", "LOQ 15AHP10", "R8CN").expect("83JG");
        assert_eq!(p.gen, 10);
        assert_eq!(p.fans, FanLayout::Two);

        // 83G0 (Legion 9 16IRX9)
        let p = lookup("83G0", "Legion 9 16IRX9", "NXCN").expect("83G0");
        assert_eq!(p.gen, 9);
        assert_eq!(p.fans, FanLayout::Three);

        // 82WR (Legion Pro 7 16IRX8H)
        let p = lookup("82WR", "Legion Pro 7 16IRX8H", "KWCN").expect("82WR");
        assert_eq!(p.gen, 8);
        assert_eq!(p.fans, FanLayout::Two);

        // 83DV (LOQ 15IRX9)
        let p = lookup("83DV", "LOQ 15IRX9", "NECN").expect("83DV");
        assert_eq!(p.gen, 9);
        assert_eq!(p.fans, FanLayout::Two);

        // Lowercase machine_type edge case (e.g. "83dg" or "83ru")
        let p_lower = lookup("83dg", "Legion Y7000P IRX9", "NMCN").expect("lowercase 83dg");
        assert_eq!(p_lower.gen, 9);
        let p_lower2 = lookup("83ru", "Legion Pro 7 16AFR10H", "SMCN").expect("lowercase 83ru");
        assert_eq!(p_lower2.gen, 10);
    }

    #[test]
    fn test_expected_tgp_heuristic() {
        assert_eq!(
            expected_tgp_from_gpu_name("NVIDIA GeForce RTX 5080 Laptop GPU"),
            Some(175)
        );
        assert_eq!(
            expected_tgp_from_gpu_name("NVIDIA GeForce RTX 5050 Laptop GPU"),
            Some(100)
        );
        assert_eq!(
            expected_tgp_from_gpu_name("NVIDIA GeForce RTX 4070 Laptop GPU"),
            Some(140)
        );
        assert_eq!(expected_tgp_from_gpu_name("GeForce RTX 2050"), Some(70));
        assert_eq!(
            expected_tgp_from_gpu_name("NVIDIA GeForce RTX 3060 Laptop GPU"),
            Some(130)
        );
        assert_eq!(expected_tgp_from_gpu_name("Unknown GPU"), None);
    }
}
