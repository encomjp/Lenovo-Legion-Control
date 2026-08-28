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
        machine_type: None,
        bios_prefix: "J2CN",
        marketing: "Legion 5 Pro 16IAH7H",
        series: "Legion 5 Pro Gen 7",
        gen: 7,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "e.g. RTX 3070 Ti",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "J1CN",
        marketing: "Legion (J1CN)",
        series: "Legion Gen 7",
        gen: 7,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "KWCN",
        marketing: "Legion 7i Pro (2023)",
        series: "Legion 7 Gen 8",
        gen: 8,
        fans: FanLayout::Three,
        fan_rpm_fallback: FALLBACK_3FAN_GENERIC,
        notes: "e.g. Legion 7i Pro 2023",
        source: "LenovoLegionLinux legion-laptop.c",
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
        machine_type: None,
        bios_prefix: "LPCN",
        marketing: "Legion Pro 5 (2023) / R9000P",
        series: "Legion Pro 5 Gen 8",
        gen: 8,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "M3CN",
        marketing: "Legion Slim 5 16APH8 (2023)",
        series: "Legion Slim 5 Gen 8",
        gen: 8,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "M0CN",
        marketing: "Legion Slim 7 16IRH8 (2023)",
        series: "Legion Slim 7 Gen 8",
        gen: 8,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "e.g. RTX 4070 Intel",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "M1CN",
        marketing: "Legion Slim 7 16IRH8 AMD (2023)",
        series: "Legion Slim 7 Gen 8",
        gen: 8,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "e.g. Ryzen 7 7840HS + RTX 4060",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "M2CN",
        marketing: "Legion Slim 5 16IRH8 (2023)",
        series: "Legion Slim 5 Gen 8",
        gen: 8,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "e.g. RTX 4070",
        source: "LenovoLegionLinux legion-laptop.c",
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
        machine_type: None,
        bios_prefix: "FCCN",
        marketing: "IdeaPad Gaming 3 15ARH05",
        series: "IdeaPad Gaming",
        gen: 0,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "IdeaPad Gaming — partial Legion EC support",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "H3CN",
        marketing: "IdeaPad Gaming 3 (H3CN)",
        series: "IdeaPad Gaming",
        gen: 0,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "H4CN",
        marketing: "IdeaPad Gaming 3 15ARH05 (8K21)",
        series: "IdeaPad Gaming",
        gen: 0,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "JNCN",
        marketing: "IdeaPad Gaming 3 15ARH7 (2022)",
        series: "IdeaPad Gaming",
        gen: 0,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "",
        source: "LenovoLegionLinux legion-laptop.c",
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
        machine_type: None,
        bios_prefix: "LZCN",
        marketing: "LOQ 15IRH8",
        series: "LOQ",
        gen: 8,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "LOQ shares Legion WMI subset",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "NECN",
        marketing: "LOQ 15IRX9",
        series: "LOQ",
        gen: 9,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "NZCN",
        marketing: "LOQ 15AHP9",
        series: "LOQ",
        gen: 9,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "e.g. Ryzen 7 8845HS + RTX 4060",
        source: "LenovoLegionLinux legion-laptop.c",
    },
    ModelProfile {
        machine_type: Some("83JG"),
        bios_prefix: "R8CN",
        marketing: "LOQ 15AHP10",
        series: "LOQ",
        gen: 10,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "2025 LOQ (Ryzen 200-series). EC IT5508 locked: lenovo_wmi_other hwmon reads 0 RPM, ACPI temp paths error -5 (WMI3 works). RPM lives in yogafan hwmon (EC 0xFE/0xFF via \\_SB.PCI0.LPC0.EC0.FANS/FA2S, 16-bit). PPT: SPL 25-48 W, SPPT 35-43 W, FPPT 45-53 W.",
        source: "fleet telemetry (83JG) + LenovoLegionLinux #384/#453/#467 + kernel yogafan.c",
    },
    ModelProfile {
        machine_type: None,
        bios_prefix: "R3CN",
        marketing: "LOQ 15IRX10",
        series: "LOQ Gen 10",
        gen: 10,
        fans: FanLayout::Two,
        fan_rpm_fallback: FALLBACK_2FAN,
        notes: "e.g. Intel + RTX 5060 class",
        source: "LenovoLegionLinux legion-laptop.c",
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
            if p.machine_type == Some(mt) {
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
        } else if g.contains("5070") || g.contains("5060") || g.contains("4050") {
            Some(115)
        } else if g.contains("4070 ti") || g.contains("4070ti") {
            Some(150)
        } else if g.contains("4070") || g.contains("4060") {
            Some(140)
        } else {
            None
        };
    log::trace!("tgp heuristic: {gpu:?} → {tgp:?} W");
    tgp
}
