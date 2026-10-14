//! AW88399 smart-amp firmware (`aw88399_acf.bin`) fetch/verify/install logic.
//!
//! The blob is NOT in linux-firmware (absent from WHENCE) and is extracted from
//! the vendor's proprietary Windows audio driver, so it is not redistributable
//! under this repository's GPL. It is therefore never committed: the root
//! setup helper downloads it from a pinned upstream URL, verifies a pinned
//! SHA-256 and installs it. This module holds the pure, unit-testable parts.

use std::path::{Path, PathBuf};

/// Pinned upstream commit of marco-giunta/legion-pro7-gen10-audio.
pub const SOURCE_COMMIT: &str = "246ec0e5ebc2f101c75eb95ba1572c9acc8007b3";
/// Pinned download URL (commit-addressed, so the content cannot drift).
pub const SOURCE_URL: &str = "https://raw.githubusercontent.com/marco-giunta/legion-pro7-gen10-audio/246ec0e5ebc2f101c75eb95ba1572c9acc8007b3/firmware/aw88399/aw88399_acf.bin";
/// Pinned SHA-256 of the blob, lowercase hex.
pub const SOURCE_SHA256: &str = "1e927c9bca76d868181c0f81df2bccef3cf19c7d0910219f229360c87babd42c";
/// Exact size of the pinned blob; used to cap the download.
pub const SOURCE_SIZE: u64 = 152_416;
/// File name the kernel driver requests (`MODULE_FIRMWARE("aw88399_acf.bin")`).
pub const FIRMWARE_FILE: &str = "aw88399_acf.bin";
/// Install directory: the driver requests the bare name, so /lib/firmware root.
pub const INSTALL_DIR: &str = "/lib/firmware";

/// Decision for the install operation.
#[derive(Debug, PartialEq, Eq)]
pub enum Plan {
    /// No AWDZ8399 amp in ACPI: do nothing.
    NotApplicable,
    /// A firmware file already exists (never overwritten).
    AlreadyPresent,
    /// Download, verify and install to this path.
    Install(PathBuf),
}

/// Pure decision: install only on amp machines with no existing firmware.
pub fn plan(amp_acpi: bool, firmware_present: bool) -> Plan {
    if !amp_acpi {
        Plan::NotApplicable
    } else if firmware_present {
        Plan::AlreadyPresent
    } else {
        Plan::Install(destination(Path::new(INSTALL_DIR)))
    }
}

pub fn destination(dir: &Path) -> PathBuf {
    dir.join(FIRMWARE_FILE)
}

/// Extract the lowercase hex digest from `sha256sum` output (`<hex>  <name>`).
pub fn parse_sha256sum_output(out: &str) -> Option<String> {
    let tok = out.split_whitespace().next()?;
    (tok.len() == 64 && tok.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| tok.to_ascii_lowercase())
}

/// True when `digest` equals the pinned SHA-256.
pub fn digest_matches(digest: &str) -> bool {
    digest.trim().eq_ignore_ascii_case(SOURCE_SHA256)
}

/// Message shown after a successful install.
pub const REBOOT_HINT: &str = "Speaker firmware installed. Reboot, or reload the driver now: \
sudo modprobe -r snd_hda_scodec_aw88399_i2c && sudo modprobe snd_hda_scodec_aw88399_i2c";

/// Stable setup-helper locations (matching the polkit policy paths).
pub const HELPER_CANDIDATES: &[&str] = &[
    "/usr/libexec/legion-control-setup",
    "/usr/local/libexec/legion-control-setup",
    "/usr/lib/legion-control-setup",
];

/// Run `pkexec <helper> install-amp-firmware`; returns the helper's stdout.
pub fn install_via_helper() -> Result<String, String> {
    let helper = HELPER_CANDIDATES
        .iter()
        .find(|p| Path::new(p).is_file())
        .ok_or("Setup helper is missing; reinstall Legion Control from the current package")?;
    let out = std::process::Command::new("pkexec")
        .arg(helper)
        .arg("install-amp-firmware")
        .output()
        .map_err(|e| format!("Cannot start PolicyKit setup: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() {
            format!("Setup was cancelled or failed ({})", out.status)
        } else {
            err
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_selects_path_only_on_amp_without_firmware() {
        assert_eq!(plan(false, false), Plan::NotApplicable);
        assert_eq!(plan(false, true), Plan::NotApplicable);
        assert_eq!(plan(true, true), Plan::AlreadyPresent);
        assert_eq!(
            plan(true, false),
            Plan::Install(PathBuf::from("/lib/firmware/aw88399_acf.bin"))
        );
    }

    #[test]
    fn checksum_parse_and_verify() {
        let line = format!("{SOURCE_SHA256}  /lib/firmware/.part\n");
        let d = parse_sha256sum_output(&line).unwrap();
        assert!(digest_matches(&d));
        assert!(digest_matches(&SOURCE_SHA256.to_uppercase()));
        assert!(!digest_matches(&"0".repeat(64)));
        assert_eq!(parse_sha256sum_output("short  x"), None);
        assert_eq!(parse_sha256sum_output(""), None);
        assert_eq!(
            parse_sha256sum_output(&format!("{}zz  x", "a".repeat(62))),
            None
        );
    }

    #[test]
    fn pinned_url_is_https_and_commit_addressed() {
        assert!(SOURCE_URL.starts_with("https://"));
        assert!(SOURCE_URL.contains(SOURCE_COMMIT));
        assert!(SOURCE_URL.ends_with(FIRMWARE_FILE));
        assert_eq!(SOURCE_SHA256.len(), 64);
    }
}
