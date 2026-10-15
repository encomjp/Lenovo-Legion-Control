//! Keeps the systemd sandbox of `legion-control.service` current.
//!
//! In-app updates of source installs (0.2.13 to 0.2.19 updaters) copy only
//! the binaries, so a host unit can lack device rules a newer daemon needs
//! (e.g. `char-nvidia` for dGPU temperatures, added in 0.2.20). The GUI
//! compares the *effective* rules (`systemctl show`) with the ones this build
//! ships and offers a one-click refresh. The refresh writes a drop-in, so
//! package-owned units in /usr/lib are never edited.

use std::collections::BTreeSet;
use std::process::Command;

/// The unit this build ships (source of truth for the device rules).
pub const UNIT_TEMPLATE: &str = include_str!("../data/systemd/legion-control.system.service");

/// Drop-in written by the setup helper's `refresh-service` operation.
pub const DROP_IN_PATH: &str =
    "/etc/systemd/system/legion-control.service.d/10-legion-device-access.conf";

/// `DeviceAllow=` values from unit text, e.g. `char-nvidia rw`.
pub fn device_rules(unit_text: &str) -> BTreeSet<String> {
    unit_text
        .lines()
        .filter_map(|l| l.trim().strip_prefix("DeviceAllow="))
        .map(|v| v.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|v| !v.is_empty())
        .collect()
}

/// Rules this build needs.
pub fn expected_rules() -> BTreeSet<String> {
    device_rules(UNIT_TEMPLATE)
}

/// Rules systemd applies right now; `None` when the service is not
/// installed or systemctl is unavailable.
pub fn effective_rules() -> Option<BTreeSet<String>> {
    let loaded = Command::new("systemctl")
        .args(["show", "legion-control.service", "-p", "LoadState", "--value"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "loaded")
        .unwrap_or(false);
    if !loaded {
        return None;
    }
    let out = Command::new("systemctl")
        .args(["show", "legion-control.service", "-p", "DeviceAllow"])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    Some(device_rules(&String::from_utf8_lossy(&out.stdout)))
}

/// Rules this build needs that the running service does not have.
pub fn missing_rules(expected: &BTreeSet<String>, effective: &BTreeSet<String>) -> Vec<String> {
    expected.difference(effective).cloned().collect()
}

/// True when the installed service lacks device access this build needs.
pub fn sandbox_outdated() -> bool {
    let Some(effective) = effective_rules() else {
        return false;
    };
    let missing = missing_rules(&expected_rules(), &effective);
    if !missing.is_empty() {
        log::info!("service_unit: host service lacks device rules {missing:?}");
    }
    !missing.is_empty()
}

/// Drop-in body: an empty assignment resets the list, then the full set.
pub fn drop_in_text() -> String {
    let mut s = String::from(
        "# Written by Legion Control (legion-control-setup refresh-service).\n\
         # Brings the device sandbox in line with the installed version.\n\
         [Service]\nDeviceAllow=\n",
    );
    for rule in expected_rules() {
        s.push_str("DeviceAllow=");
        s.push_str(&rule);
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_has_nvidia_rule() {
        assert!(expected_rules().contains("char-nvidia rw"));
    }

    #[test]
    fn parses_systemctl_show_output() {
        let show = "DeviceAllow=char-drm rw\nDeviceAllow=/dev/hidraw*  rw\n";
        let rules = device_rules(show);
        assert!(rules.contains("char-drm rw"));
        assert!(rules.contains("/dev/hidraw* rw"));
    }

    #[test]
    fn detects_missing_nvidia_rule() {
        let expected = device_rules("DeviceAllow=char-hidraw rw\nDeviceAllow=char-nvidia rw\n");
        let old_host = device_rules("DeviceAllow=char-hidraw rw\n");
        assert_eq!(missing_rules(&expected, &old_host), vec!["char-nvidia rw"]);
        assert!(missing_rules(&expected, &expected).is_empty());
    }

    #[test]
    fn drop_in_resets_then_lists_every_rule() {
        let text = drop_in_text();
        assert!(text.contains("[Service]\nDeviceAllow=\n"));
        assert_eq!(device_rules(&text), expected_rules());
    }
}
