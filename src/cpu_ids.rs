//! SKU → pretty CPU name lookup.
//!
//! `data/cpu-ids.yaml` is embedded at compile time. Keys are marketing SKU
//! tokens from `/proc/cpuinfo` (`9955HX3D`, `14700HX`, `HX 370`), not CPUID —
//! family/model is shared across many laptop SKUs.

use std::sync::OnceLock;

const CPU_IDS_YAML: &str = include_str!("../data/cpu-ids.yaml");

fn rows() -> &'static Vec<(String, String)> {
    static ROWS: OnceLock<Vec<(String, String)>> = OnceLock::new();
    ROWS.get_or_init(|| {
        let mut rows = parse_cpu_ids(CPU_IDS_YAML);
        rows.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(&b.0)));
        rows
    })
}

/// Pretty marketing name for a `/proc/cpuinfo` model-name line.
pub fn pretty_name(raw: &str) -> Option<String> {
    let hay = raw.trim();
    if hay.is_empty() {
        return None;
    }
    for (sku, name) in rows() {
        if token_in(hay, sku) {
            return Some(name.clone());
        }
    }
    None
}

/// Strip trademark junk and "16-Core Processor" when no SKU row matches.
pub fn clean(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    for junk in ["(R)", "(r)", "(TM)", "(tm)", "®", "™"] {
        s = s.replace(junk, "");
    }
    s = s.replace("with Radeon Graphics", "");
    s = s.replace("with Radeon Vega Graphics", "");
    if let Some(idx) = s.find("-Core") {
        s = s[..idx].to_string();
        s = s
            .trim_end_matches(|c: char| c.is_ascii_digit())
            .trim_end()
            .to_string();
    }
    s = s.replace(" Processor", "");
    s = s.replace(" CPU @", " @");
    collapse_ws(&s)
}

/// YAML lookup, else cleaned cpuinfo string.
pub fn display_name(raw: &str) -> String {
    pretty_name(raw).unwrap_or_else(|| clean(raw))
}

fn token_in(haystack: &str, key: &str) -> bool {
    let hay = haystack.to_ascii_uppercase();
    let needle = key.to_ascii_uppercase();
    if needle.is_empty() {
        return false;
    }
    let bytes = hay.as_bytes();
    let n = needle.as_bytes();
    let mut start = 0;
    while start + n.len() <= bytes.len() {
        if hay[start..].starts_with(&needle) {
            let left_ok = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
            let end = start + n.len();
            let right_ok = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
            if left_ok && right_ok {
                return true;
            }
        }
        start += 1;
    }
    false
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse_cpu_ids(text: &str) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    for raw_line in text.lines() {
        let stripped = strip_comment(raw_line);
        let line = stripped.trim();
        if line.is_empty() || line.ends_with(':') && !line.contains('"') {
            continue;
        }
        let Some((id, name)) = line.split_once(':') else {
            continue;
        };
        let id = unquote(id.trim());
        let name = unquote(name.trim());
        if id.is_empty() || name.is_empty() {
            continue;
        }
        // Skip vendor section headers that slipped through (`amd:` already
        // filtered). Bare words without a value are not SKUs.
        if name.chars().all(|c| c.is_ascii_lowercase()) && name.len() < 8 {
            continue;
        }
        rows.push((id, name));
    }
    rows
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
    fn legion_gen10_amd() {
        assert_eq!(
            pretty_name("AMD Ryzen 9 9955HX3D 16-Core Processor").as_deref(),
            Some("AMD Ryzen 9 9955HX3D")
        );
        assert_eq!(
            pretty_name("AMD Ryzen 9 9955HX 16-Core Processor").as_deref(),
            Some("AMD Ryzen 9 9955HX")
        );
        assert_eq!(
            display_name("AMD Ryzen 9 9955HX3D 16-Core Processor"),
            "AMD Ryzen 9 9955HX3D"
        );
    }

    #[test]
    fn longer_sku_wins_over_prefix() {
        assert_eq!(
            pretty_name("AMD Ryzen 7 6800HS").as_deref(),
            Some("AMD Ryzen 7 6800HS")
        );
        assert_eq!(
            pretty_name("AMD Ryzen 7 6800H").as_deref(),
            Some("AMD Ryzen 7 6800H")
        );
        assert_eq!(pretty_name("Intel Core i5-15600H").as_deref(), None);
        assert_eq!(
            pretty_name("AMD Ryzen 5 5600H").as_deref(),
            Some("AMD Ryzen 5 5600H")
        );
    }

    #[test]
    fn intel_hx_and_ultra() {
        assert_eq!(
            pretty_name("Intel(R) Core(TM) i9-14900HX").as_deref(),
            Some("Intel Core i9-14900HX")
        );
        assert_eq!(
            pretty_name("Intel Core Ultra 9 275HX").as_deref(),
            Some("Intel Core Ultra 9 275HX")
        );
        assert_eq!(
            pretty_name("Intel(R) Core(TM) Ultra 7 255H").as_deref(),
            Some("Intel Core Ultra 7 255H")
        );
        assert_eq!(
            pretty_name("Intel Core Ultra 7 255HX").as_deref(),
            Some("Intel Core Ultra 7 255HX")
        );
    }

    #[test]
    fn strix_and_loq() {
        assert_eq!(
            pretty_name("AMD Ryzen AI 9 HX 370 w/ Radeon 890M").as_deref(),
            Some("AMD Ryzen AI 9 HX 370")
        );
        assert_eq!(
            pretty_name("AMD Ryzen 7 7840HS with Radeon 780M Graphics").as_deref(),
            Some("AMD Ryzen 7 7840HS")
        );
        assert_eq!(
            pretty_name("AMD Ryzen 7 250").as_deref(),
            Some("AMD Ryzen 7 250")
        );
    }

    #[test]
    fn clean_strips_trademark_and_core_count() {
        assert_eq!(
            clean("Intel(R) Core(TM) i7-14700HX"),
            "Intel Core i7-14700HX"
        );
        assert_eq!(
            clean("AMD Ryzen 9 9999HX 16-Core Processor"),
            "AMD Ryzen 9 9999HX"
        );
    }

    #[test]
    fn yaml_has_a_useful_laptop_set() {
        assert!(rows().len() >= 150, "got {} rows", rows().len());
    }
}
