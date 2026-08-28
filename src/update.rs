//! GitHub release checker and update notification system for Legion Control.
//!
//! Queries the GitHub releases API asynchronously (via `curl` to avoid adding
//! heavy HTTP client crates) and parses semver tags to detect available updates.

use serde::{Deserialize, Serialize};
use std::process::Command;

pub const GITHUB_REPO: &str = "encomjp/Lenovo-Legion-Control";
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReleaseInfo {
    pub tag_name: String,
    pub version: String,
    pub name: String,
    pub body: String,
    pub html_url: String,
    pub published_at: String,
    pub is_newer: bool,
}

/// Check GitHub for the latest release of `encomjp/Lenovo-Legion-Control`.
/// Runs with a 5-second timeout and fails gracefully if offline.
pub fn check_latest_release() -> Result<ReleaseInfo, String> {
    let url = format!("https://api.github.com/repos/{GITHUB_REPO}/releases/latest");
    log::debug!("update: checking latest release from {url}");

    let output = Command::new("curl")
        .args([
            "-sL",
            "-S",
            "--max-time",
            "5",
            "-H",
            "User-Agent: legion-control-update-checker",
            "-H",
            "Accept: application/vnd.github.v3+json",
            &url,
        ])
        .output()
        .map_err(|e| format!("Failed to spawn curl: {e}"))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("curl error (exit {}): {err}", output.status));
    }

    #[derive(Deserialize)]
    struct GhRelease {
        tag_name: String,
        name: Option<String>,
        body: Option<String>,
        html_url: Option<String>,
        published_at: Option<String>,
    }

    // Handle repositories with no formal releases yet by checking tags/commit
    let body_str = String::from_utf8_lossy(&output.stdout);
    if body_str.contains("Not Found")
        || body_str.contains("\"message\"") && !body_str.contains("\"tag_name\"")
    {
        log::debug!("update: no formal releases found on GitHub repo — current version is latest");
        return Ok(ReleaseInfo {
            tag_name: format!("v{CURRENT_VERSION}"),
            version: CURRENT_VERSION.to_string(),
            name: format!("Version {CURRENT_VERSION}"),
            body: "Current release".into(),
            html_url: format!("https://github.com/{GITHUB_REPO}/releases"),
            published_at: "".into(),
            is_newer: false,
        });
    }

    let gh: GhRelease = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Failed to parse GitHub release JSON: {e}"))?;

    let version = gh.tag_name.trim_start_matches('v').to_string();
    let is_newer = is_version_newer(&version, CURRENT_VERSION);

    Ok(ReleaseInfo {
        tag_name: gh.tag_name,
        version: version.clone(),
        name: gh.name.unwrap_or_else(|| format!("Version {version}")),
        body: gh.body.unwrap_or_default(),
        html_url: gh
            .html_url
            .unwrap_or_else(|| format!("https://github.com/{GITHUB_REPO}/releases")),
        published_at: gh.published_at.unwrap_or_default(),
        is_newer,
    })
}

/// Compare two semver-like version strings (e.g. "0.2.0" > "0.1.0", "1.0.0" > "0.9.9").
/// Returns true if `remote` is strictly newer than `local`.
pub fn is_version_newer(remote: &str, local: &str) -> bool {
    let parse_nums = |s: &str| -> Vec<u64> {
        let clean = s.trim().trim_start_matches('v');
        clean
            .split('.')
            .filter_map(|part| {
                // Split off any prerelease tag like -alpha / -rc1
                let num_str = part.split('-').next().unwrap_or(part);
                num_str.parse::<u64>().ok()
            })
            .collect()
    };

    let r_nums = parse_nums(remote);
    let l_nums = parse_nums(local);

    for i in 0..r_nums.len().max(l_nums.len()) {
        let r_val = r_nums.get(i).copied().unwrap_or(0);
        let l_val = l_nums.get(i).copied().unwrap_or(0);
        if r_val > l_val {
            return true;
        }
        if r_val < l_val {
            return false;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_version_newer_comparisons() {
        assert!(is_version_newer("0.2.0", "0.1.0"));
        assert!(is_version_newer("1.0.0", "0.9.9"));
        assert!(is_version_newer("0.1.1", "0.1.0"));
        assert!(is_version_newer("v0.2.0", "0.1.0"));
        assert!(is_version_newer("0.2.0-rc1", "0.1.0"));

        assert!(!is_version_newer("0.1.0", "0.1.0"));
        assert!(!is_version_newer("0.1.0", "0.2.0"));
        assert!(!is_version_newer("0.0.9", "0.1.0"));
        assert!(!is_version_newer("v0.1.0", "0.1.0"));
    }
}
