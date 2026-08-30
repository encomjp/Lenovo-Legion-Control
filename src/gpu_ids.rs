//! PCI-ID → pretty GPU name lookup.
//!
//! `data/gpu-ids.yaml` is embedded at compile time so the daemon never
//! depends on pci.ids or a live nvidia-smi query for the marketing string.

use std::collections::HashMap;
use std::sync::OnceLock;

const GPU_IDS_YAML: &str = include_str!("../data/gpu-ids.yaml");

const VENDORS: &[(&str, &str)] = &[("nvidia", "10de"), ("amd", "1002"), ("intel", "8086")];

fn db() -> &'static HashMap<String, String> {
    static DB: OnceLock<HashMap<String, String>> = OnceLock::new();
    DB.get_or_init(|| parse_gpu_ids(GPU_IDS_YAML))
}

/// Pretty marketing name for a PCI GPU, if we have a curated row.
///
/// `vendor` / `device` accept sysfs (`0x10de`) or bare hex (`10de`, `2c19`).
pub fn pretty_name(vendor: &str, device: &str) -> Option<String> {
    let key = pci_key(vendor, device)?;
    db().get(&key).cloned()
}

fn pci_key(vendor: &str, device: &str) -> Option<String> {
    let v = normalize_hex(vendor)?;
    let d = normalize_hex(device)?;
    Some(format!("{v}:{d}"))
}

fn normalize_hex(raw: &str) -> Option<String> {
    let hex = raw.trim().trim_start_matches("0x").trim_start_matches("0X");
    if hex.is_empty() || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(hex.to_ascii_lowercase())
}

fn parse_gpu_ids(text: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut vendor: Option<&str> = None;
    for raw_line in text.lines() {
        let stripped = strip_comment(raw_line);
        let line = stripped.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_suffix(':') {
            if !name.contains(' ') && !name.contains('\t') {
                vendor = VENDORS
                    .iter()
                    .find(|(label, pci)| *label == name || *pci == name.trim_start_matches("0x"))
                    .map(|(_, pci)| *pci);
                continue;
            }
        }
        let Some(vid) = vendor else {
            continue;
        };
        let Some((id, name)) = line.split_once(':') else {
            continue;
        };
        let id = id.trim().trim_matches('"').trim_matches('\'');
        let name = unquote(name.trim());
        if name.is_empty() {
            continue;
        }
        let Some(did) = normalize_hex(id) else {
            continue;
        };
        map.insert(format!("{vid}:{did}"), name);
    }
    map
}

fn strip_comment(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_quotes = false;
    for c in line.chars() {
        if c == '"' {
            in_quotes = !in_quotes;
            out.push(c);
            continue;
        }
        if c == '#' && !in_quotes {
            break;
        }
        out.push(c);
    }
    out
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_up_legion_gen10_nvidia() {
        assert_eq!(
            pretty_name("0x10de", "2C19").as_deref(),
            Some("NVIDIA GeForce RTX 5080 Laptop GPU")
        );
        assert_eq!(
            pretty_name("10de", "2c58").as_deref(),
            Some("NVIDIA GeForce RTX 5090 Laptop GPU")
        );
        assert_eq!(
            pretty_name("0x10DE", "2f18").as_deref(),
            Some("NVIDIA GeForce RTX 5070 Ti Laptop GPU")
        );
    }

    #[test]
    fn looks_up_obscure_and_older_lenovo_skus() {
        assert_eq!(
            pretty_name("10de", "1c90").as_deref(),
            Some("NVIDIA GeForce MX150")
        );
        assert_eq!(
            pretty_name("10de", "25a9").as_deref(),
            Some("NVIDIA GeForce RTX 2050")
        );
        assert_eq!(
            pretty_name("10de", "25ac").as_deref(),
            Some("NVIDIA GeForce RTX 3050 6GB Laptop GPU")
        );
        assert_eq!(
            pretty_name("10de", "2820").as_deref(),
            Some("NVIDIA GeForce RTX 4070 Laptop GPU")
        );
        assert_eq!(
            pretty_name("10de", "1618").as_deref(),
            Some("NVIDIA GeForce GTX 970M")
        );
    }

    #[test]
    fn looks_up_lenovo_amd_dgpu() {
        assert_eq!(
            pretty_name("0x1002", "73f0").as_deref(),
            Some("AMD Radeon RX 7600M XT")
        );
        assert_eq!(
            pretty_name("1002", "7340").as_deref(),
            Some("AMD Radeon RX 5500M")
        );
        assert_eq!(
            pretty_name("1002", "731f").as_deref(),
            Some("AMD Radeon RX 5600M")
        );
        assert_eq!(
            pretty_name("1002", "73ff").as_deref(),
            Some("AMD Radeon RX 6600M")
        );
        assert_eq!(
            pretty_name("1002", "7480").as_deref(),
            Some("AMD Radeon RX 7700S")
        );
    }

    #[test]
    fn looks_up_lenovo_amd_apu() {
        assert_eq!(
            pretty_name("1002", "13c0").as_deref(),
            Some("AMD Radeon 610M")
        );
        assert_eq!(
            pretty_name("1002", "15bf").as_deref(),
            Some("AMD Radeon 780M")
        );
        assert_eq!(
            pretty_name("1002", "1900").as_deref(),
            Some("AMD Radeon 780M")
        );
        assert_eq!(
            pretty_name("1002", "150e").as_deref(),
            Some("AMD Radeon 890M")
        );
        assert_eq!(
            pretty_name("1002", "1586").as_deref(),
            Some("AMD Radeon 8060S")
        );
        assert_eq!(
            pretty_name("1002", "164d").as_deref(),
            Some("AMD Radeon 680M")
        );
    }

    #[test]
    fn unknown_id_is_none() {
        assert_eq!(pretty_name("10de", "0000"), None);
        assert_eq!(pretty_name("10de", "not-hex"), None);
    }

    #[test]
    fn parser_ignores_comments_and_accepts_quotes() {
        let map = parse_gpu_ids(
            r#"
# heading
nvidia:
  # chip comment
  abcd: NVIDIA GeForce RTX 9999 Laptop GPU  # trailing
  "12ef": "NVIDIA GeForce GTX 1"
amd:
  1111: AMD Radeon RX 1
"#,
        );
        assert_eq!(
            map.get("10de:abcd").map(String::as_str),
            Some("NVIDIA GeForce RTX 9999 Laptop GPU")
        );
        assert_eq!(
            map.get("10de:12ef").map(String::as_str),
            Some("NVIDIA GeForce GTX 1")
        );
        assert_eq!(
            map.get("1002:1111").map(String::as_str),
            Some("AMD Radeon RX 1")
        );
    }

    #[test]
    fn yaml_has_a_useful_laptop_set() {
        assert!(db().len() >= 150, "got {} rows", db().len());
        assert!(db().keys().all(|k| k.contains(':')));
    }
}
