//! ideapad_acpi on/off toggles: Fn Lock and always-on USB charging.
//!
//! Both live next to `camera_power` / `conservation_mode` under the
//! `VPC2004:00` platform device. Each is exposed only when the kernel driver
//! provides the attribute for this model, so every helper returns `None` /
//! an error when it is absent and callers hide the control.

use std::path::{Path, PathBuf};

/// One ideapad_acpi sysfs on/off attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggle {
    FnLock,
    UsbCharging,
}

impl Toggle {
    pub fn attr(self) -> &'static str {
        match self {
            Toggle::FnLock => "fn_lock",
            Toggle::UsbCharging => "usb_charging",
        }
    }
}

/// Directories that may hold the ideapad_acpi attributes, most specific first.
const KNOWN_DIRS: &[&str] = &[
    "/sys/bus/platform/devices/VPC2004:00",
    "/sys/bus/platform/drivers/ideapad_acpi/VPC2004:00",
    "/sys/devices/pci0000:00/0000:00:14.3/PNP0C09:00/VPC2004:00",
];

/// Parse a sysfs "0"/"1" attribute. Anything else is unknown.
pub fn parse_flag(raw: &str) -> Option<bool> {
    match raw.trim() {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    }
}

/// First `<dir>/<attr>` that exists, checking `dirs` in order and then every
/// child directory of `scan_root` (the ideapad_acpi driver dir).
pub fn resolve_in(dirs: &[PathBuf], scan_root: &Path, attr: &str) -> Option<PathBuf> {
    for d in dirs {
        let p = d.join(attr);
        if p.is_file() {
            return Some(p);
        }
    }
    let mut children: Vec<PathBuf> = std::fs::read_dir(scan_root)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .collect();
    children.sort();
    children
        .into_iter()
        .map(|c| c.join(attr))
        .find(|p| p.is_file())
}

/// Path of the attribute on this machine, if the driver exposes it.
pub fn path(t: Toggle) -> Option<PathBuf> {
    let dirs: Vec<PathBuf> = KNOWN_DIRS.iter().map(PathBuf::from).collect();
    let p = resolve_in(
        &dirs,
        Path::new("/sys/bus/platform/drivers/ideapad_acpi"),
        t.attr(),
    );
    log::trace!("ideapad::path({}) → {p:?}", t.attr());
    p
}

/// Current state, or `None` when unsupported / unreadable.
pub fn get(t: Toggle) -> Option<bool> {
    parse_flag(&std::fs::read_to_string(path(t)?).ok()?)
}

/// True when the attribute exists (read-only presence check).
pub fn supported(t: Toggle) -> bool {
    path(t).is_some()
}

/// Write `on` to `file`, read it back and fail when firmware ignored it.
pub fn write_verify(file: &Path, name: &str, on: bool) -> Result<(), String> {
    std::fs::write(file, if on { "1" } else { "0" })
        .map_err(|e| format!("cannot write {name} ({}): {e}", file.display()))?;
    match std::fs::read_to_string(file)
        .ok()
        .and_then(|s| parse_flag(&s))
    {
        Some(v) if v == on => Ok(()),
        Some(v) => Err(format!(
            "{name}: firmware ignored the write (asked {}, read back {})",
            u8::from(on),
            u8::from(v)
        )),
        None => Err(format!("{name}: written but read-back failed")),
    }
}

/// Set a toggle (root only) and verify by read-back.
pub fn set(t: Toggle, on: bool) -> Result<(), String> {
    let file = path(t).ok_or_else(|| format!("{} is not supported on this laptop", t.attr()))?;
    log::debug!("ideapad::set({}, {on}) at {}", t.attr(), file.display());
    let r = write_verify(&file, t.attr(), on);
    if let Err(e) = &r {
        log::warn!("ideapad::set — {e}");
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("legion-ideapad-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn parse_flag_accepts_only_zero_one() {
        assert_eq!(parse_flag("1\n"), Some(true));
        assert_eq!(parse_flag(" 0 "), Some(false));
        assert_eq!(parse_flag("2"), None);
        assert_eq!(parse_flag(""), None);
        assert_eq!(parse_flag("on"), None);
    }

    #[test]
    fn attr_names() {
        assert_eq!(Toggle::FnLock.attr(), "fn_lock");
        assert_eq!(Toggle::UsbCharging.attr(), "usb_charging");
    }

    #[test]
    fn resolve_prefers_known_dir_then_scans() {
        let root = tmp("resolve");
        let known = root.join("known");
        let scan = root.join("drv");
        let dev = scan.join("VPC2004:00");
        std::fs::create_dir_all(&known).unwrap();
        std::fs::create_dir_all(&dev).unwrap();
        assert_eq!(
            resolve_in(std::slice::from_ref(&known), &scan, "fn_lock"),
            None
        );
        std::fs::write(dev.join("fn_lock"), "0").unwrap();
        assert_eq!(
            resolve_in(std::slice::from_ref(&known), &scan, "fn_lock"),
            Some(dev.join("fn_lock"))
        );
        std::fs::write(known.join("fn_lock"), "1").unwrap();
        assert_eq!(
            resolve_in(std::slice::from_ref(&known), &scan, "fn_lock"),
            Some(known.join("fn_lock"))
        );
        assert_eq!(resolve_in(&[], &root.join("nope"), "x"), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn write_verify_roundtrip_and_missing() {
        let root = tmp("write");
        let f = root.join("usb_charging");
        std::fs::write(&f, "0").unwrap();
        assert!(write_verify(&f, "usb_charging", true).is_ok());
        assert_eq!(
            parse_flag(&std::fs::read_to_string(&f).unwrap()),
            Some(true)
        );
        assert!(write_verify(&f, "usb_charging", false).is_ok());
        assert!(write_verify(&root.join("absent/x"), "x", true).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
