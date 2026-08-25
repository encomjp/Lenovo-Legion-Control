//! Spectrum RGB “panic” detection + recovery.
//!
//! When the ITE 8258 (`048d:c197`) HID path dies — missing hidraw, bad
//! permissions, ioctl failures, USB disconnects in the kernel log, or lights
//! stuck dark — users see a black keyboard (“RGB panic”). Soft HID resets
//! often recover; stubborn cases need a USB reset / hid rebind (root daemon).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const VID: &str = "048d";
const PID: &str = "c197";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    /// Device open + ioctl OK; brightness matches expectation (or lights off intentionally).
    Ok,
    /// Recoverable: permissions, soft HID glitch, brightness 0 with saved lights, kernel USB blip.
    SoftIssue,
    /// Device missing or ioctl dead after soft attempts — needs USB reset / rebind / replug.
    HardwareBroken,
    /// No Spectrum hardware on this machine.
    NotApplicable,
}

#[derive(Debug, Clone)]
pub struct Diagnosis {
    pub health: Health,
    pub summary: String,
    pub details: Vec<String>,
    pub fixable: bool,
    pub device_path: Option<PathBuf>,
    pub usb_sysfs: Option<PathBuf>,
    pub accessible: bool,
    pub ioctl_ok: bool,
    pub brightness: Option<u8>,
    pub kernel_faults: Vec<String>,
    pub permission_mode: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct FixReport {
    pub steps: Vec<String>,
    pub errors: Vec<String>,
    pub after: Diagnosis,
}

/// Probe Spectrum HID + recent kernel USB/HID faults.
pub fn diagnose() -> Diagnosis {
    let mut details = Vec::new();
    let device_path = find_spectrum_hidraw();
    let usb_sysfs = device_path.as_ref().and_then(|p| usb_sysfs_for_hidraw(p));
    let permission_mode = device_path
        .as_ref()
        .and_then(|p| fs::metadata(p).ok())
        .map(|m| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                m.permissions().mode() & 0o777
            }
            #[cfg(not(unix))]
            {
                0
            }
        });

    let kernel_faults = scan_kernel_rgb_faults();
    for f in &kernel_faults {
        details.push(format!("kernel: {f}"));
    }

    let Some(path) = device_path.clone() else {
        details.push("Spectrum HID 048d:c197 not found in /sys/class/hidraw".into());
        if !kernel_faults.is_empty() {
            details.push("Kernel recently reported USB/HID errors for this controller".into());
        }
        return Diagnosis {
            health: if kernel_faults.is_empty() {
                Health::NotApplicable
            } else {
                Health::HardwareBroken
            },
            summary: if kernel_faults.is_empty() {
                "No Spectrum RGB controller detected".into()
            } else {
                "Spectrum RGB missing after USB/HID fault — try USB reset / replug".into()
            },
            details,
            fixable: !kernel_faults.is_empty(),
            device_path: None,
            usb_sysfs: None,
            accessible: false,
            ioctl_ok: false,
            brightness: None,
            kernel_faults,
            permission_mode: None,
        };
    };

    details.push(format!("hidraw {}", path.display()));
    if let Some(mode) = permission_mode {
        details.push(format!("permissions {:o}", mode));
    }
    if let Some(usb) = &usb_sysfs {
        details.push(format!("usb {}", usb.display()));
    }

    let accessible = OpenOptionsCompat::can_open_rw(&path);
    if !accessible {
        details.push("Cannot open hidraw RDWR — udev/uaccess missing?".into());
    }

    let (ioctl_ok, brightness, ioctl_err) = probe_ioctl();
    match (ioctl_ok, brightness, &ioctl_err) {
        (true, Some(b), _) => details.push(format!("ioctl OK · brightness {b}/9")),
        (true, None, _) => details.push("ioctl OK · brightness unread".into()),
        (false, _, Some(e)) => details.push(format!("ioctl failed: {e}")),
        (false, _, None) => details.push("ioctl failed".into()),
    }

    let cfg = crate::config::get();
    let zone_wants_light = |z: &crate::config::ZoneEffect| {
        !z.is_off() && !(z.effect == "static" && z.r == 0 && z.g == 0 && z.b == 0)
    };
    let expect_on = cfg.brightness > 0
        && (zone_wants_light(&cfg.keyboard)
            || zone_wants_light(&cfg.front)
            || zone_wants_light(&cfg.rear)
            || zone_wants_light(&cfg.logo)
            || !cfg.per_key.is_empty());
    let dark_panic = ioctl_ok && brightness == Some(0) && expect_on;
    if dark_panic {
        details.push(
            "Brightness is 0 but saved lighting wants lights on — classic RGB panic / dark hang"
                .into(),
        );
    }

    let bad_perms = permission_mode.map(|m| m & 0o006 == 0).unwrap_or(false);
    if bad_perms {
        details.push("hidraw not world/group writable — userspace RGB blocked".into());
    }

    let (health, summary, fixable) = if !accessible || bad_perms {
        (
            Health::SoftIssue,
            "Spectrum RGB blocked by permissions".into(),
            true,
        )
    } else if !ioctl_ok {
        (
            Health::HardwareBroken,
            "Spectrum RGB HID not responding (ioctl dead)".into(),
            true,
        )
    } else if dark_panic {
        (
            Health::SoftIssue,
            "Spectrum RGB panic — lights stuck off".into(),
            true,
        )
    } else {
        let summary = if kernel_faults.is_empty() {
            "Spectrum RGB HID healthy".into()
        } else {
            "Spectrum RGB OK now (kernel logged earlier USB/HID faults this boot)".into()
        };
        (Health::Ok, summary, false)
    };

    Diagnosis {
        health,
        summary,
        details,
        fixable,
        device_path: Some(path),
        usb_sysfs,
        accessible,
        ioctl_ok,
        brightness,
        kernel_faults,
        permission_mode,
    }
}

/// Soft → hard recovery ladder. Safe to call from GUI / CLI / daemon watchdog.
pub fn troubleshoot() -> FixReport {
    let mut steps = Vec::new();
    let mut errors = Vec::new();
    let before = diagnose();

    if before.health == Health::Ok && before.kernel_faults.is_empty() {
        steps.push("Already healthy — no fix needed".into());
        return FixReport {
            steps,
            errors,
            after: before,
        };
    }
    if before.health == Health::NotApplicable && before.device_path.is_none() {
        errors.push("No Spectrum device to repair".into());
        return FixReport {
            steps,
            errors,
            after: before,
        };
    }

    // 1) Permissions
    if let Some(path) = &before.device_path {
        if let Some(mode) = before.permission_mode {
            if mode & 0o006 == 0 {
                match fix_permissions(path) {
                    Ok(()) => steps.push(format!("Fixed permissions on {}", path.display())),
                    Err(e) => errors.push(e),
                }
            }
        }
    }

    // 2) Soft HID lighting reset (when device answers, or after perm fix)
    match crate::keyboard::troubleshoot_lighting() {
        Ok(s) => {
            steps.push("Soft Spectrum lighting reset".into());
            steps.extend(s);
        }
        Err(e) => {
            errors.push(format!("Soft lighting reset failed: {e}"));
            // 3) USB reset
            if let Some(usb) = before.usb_sysfs.clone().or_else(find_spectrum_usb_sysfs) {
                match usb_reset(&usb) {
                    Ok(()) => {
                        steps.push(format!("USB reset {}", usb.display()));
                        std::thread::sleep(Duration::from_millis(800));
                    }
                    Err(e) => errors.push(e),
                }
                // 4) hid rebind
                match hid_rebind_spectrum() {
                    Ok(msg) => {
                        steps.push(msg);
                        std::thread::sleep(Duration::from_millis(500));
                    }
                    Err(e) => errors.push(e),
                }
                // 5) Soft reset again after hardware kick
                match crate::keyboard::troubleshoot_lighting() {
                    Ok(s) => {
                        steps.push("Soft lighting reset after USB/rebind".into());
                        steps.extend(s);
                    }
                    Err(e) => errors.push(format!("Lighting still dead after USB reset: {e}")),
                }
            } else {
                errors.push(
                    "No USB sysfs path for Spectrum — cannot reset without root daemon".into(),
                );
            }
        }
    }

    // If still dark panic after soft-only path, escalate
    let mid = diagnose();
    if mid.health != Health::Ok {
        if let Some(usb) = mid.usb_sysfs.clone().or_else(find_spectrum_usb_sysfs) {
            if !steps.iter().any(|s| s.starts_with("USB reset")) {
                match usb_reset(&usb) {
                    Ok(()) => {
                        steps.push(format!("USB reset {}", usb.display()));
                        std::thread::sleep(Duration::from_millis(800));
                        match hid_rebind_spectrum() {
                            Ok(msg) => steps.push(msg),
                            Err(e) => {
                                log::warn!("HID rebind after USB reset failed: {e}");
                                errors.push(format!("HID rebind: {e}"));
                            }
                        }
                        std::thread::sleep(Duration::from_millis(400));
                        match crate::keyboard::troubleshoot_lighting() {
                            Ok(s) => {
                                steps.push("Re-applied lighting after escalated reset".into());
                                steps.extend(s);
                            }
                            Err(e) => errors.push(e),
                        }
                    }
                    Err(e) => errors.push(e),
                }
            }
        }
    }

    let after = diagnose();
    if after.health == Health::Ok {
        steps.push("Verified: Spectrum HID healthy".into());
    } else if after.health == Health::SoftIssue {
        errors.push("Still soft-broken — check Lighting tab / saved config".into());
    } else {
        errors.push(
            "Still broken — replug USB path or reboot; kernel driver may need attention".into(),
        );
    }

    FixReport {
        steps,
        errors,
        after,
    }
}

/// True when an auto-fix is warranted (daemon watchdog).
pub fn needs_autofix(d: &Diagnosis) -> bool {
    matches!(d.health, Health::SoftIssue | Health::HardwareBroken) && d.fixable
}

fn probe_ioctl() -> (bool, Option<u8>, Option<String>) {
    match crate::keyboard::rgb_brightness() {
        Some(b) => (true, Some(b), None),
        None => {
            // Distinguish missing device vs ioctl failure
            if find_spectrum_hidraw().is_none() {
                (false, None, Some("device gone".into()))
            } else {
                (false, None, Some("open or HIDIOCGFEATURE failed".into()))
            }
        }
    }
}

fn find_spectrum_hidraw() -> Option<PathBuf> {
    let entries = fs::read_dir("/sys/class/hidraw").ok()?;
    let mut fallback = None;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Ok(mut cur) = fs::canonicalize(entry.path().join("device")) else {
            continue;
        };
        let mut matched = false;
        for _ in 0..10 {
            let v = cur.join("idVendor");
            let p = cur.join("idProduct");
            if v.exists() && p.exists() {
                // An unreadable node (device mid-unplug) must not abort the
                // whole scan — stop walking this branch, try the next hidraw.
                match (fs::read_to_string(&v), fs::read_to_string(&p)) {
                    (Ok(vendor), Ok(product)) => {
                        matched = vendor.trim().to_lowercase() == VID
                            && product.trim().to_lowercase() == PID;
                    }
                    _ => break,
                }
                break;
            }
            if !cur.pop() {
                break;
            }
        }
        if matched {
            let path = PathBuf::from(format!("/dev/{name}"));
            // Prefer Spectrum usage page when present
            let desc_path = entry.path().join("device/report_descriptor");
            if let Ok(desc) = fs::read(&desc_path) {
                if desc.windows(3).any(|w| w == [0x06, 0x89, 0xff]) {
                    return Some(path);
                }
            }
            fallback = Some(path);
        }
    }
    fallback
}

fn usb_sysfs_for_hidraw(hidraw: &Path) -> Option<PathBuf> {
    let name = hidraw.file_name()?.to_str()?;
    let mut cur = fs::canonicalize(format!("/sys/class/hidraw/{name}/device")).ok()?;
    for _ in 0..16 {
        if cur.join("idVendor").exists() && cur.join("busnum").exists() {
            return Some(cur);
        }
        if !cur.pop() {
            break;
        }
    }
    None
}

fn find_spectrum_usb_sysfs() -> Option<PathBuf> {
    find_spectrum_hidraw().and_then(|p| usb_sysfs_for_hidraw(&p))
}

fn fix_permissions(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = fs::metadata(path).map_err(|e| format!("stat {}: {e}", path.display()))?;
        let mut perms = meta.permissions();
        perms.set_mode(0o666);
        fs::set_permissions(path, perms)
            .map_err(|e| format!("chmod {}: {e} (needs root daemon)", path.display()))?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err("permissions fix only on Linux".into())
    }
}

fn usb_reset(usb: &Path) -> Result<(), String> {
    let reset = usb.join("reset");
    if !reset.exists() {
        return Err(format!("no reset attribute at {}", reset.display()));
    }
    let mut f = fs::OpenOptions::new()
        .write(true)
        .open(&reset)
        .map_err(|e| format!("Cannot write {}: {e} (needs root daemon)", reset.display()))?;
    f.write_all(b"1")
        .map_err(|e| format!("USB reset failed: {e}"))?;
    Ok(())
}

fn hid_rebind_spectrum() -> Result<String, String> {
    // Find hid device symlink under the USB device and bounce hid-generic.
    let usb = find_spectrum_usb_sysfs().ok_or_else(|| "Spectrum USB path gone".to_string())?;
    let mut hid_id = None;
    if let Ok(rd) = fs::read_dir(&usb) {
        for e in rd.flatten() {
            let n = e.file_name();
            let n = n.to_string_lossy();
            // e.g. 3-2.4:1.0
            if n.contains(':') {
                let driver = e.path().join("driver");
                if driver.exists() {
                    hid_id = Some(n.into_owned());
                    break;
                }
            }
        }
    }
    // Also search one level of interfaces
    if hid_id.is_none() {
        if let Ok(rd) = fs::read_dir(&usb) {
            for iface in rd.flatten() {
                if let Ok(sub) = fs::read_dir(iface.path()) {
                    for e in sub.flatten() {
                        let n = e.file_name();
                        let n = n.to_string_lossy();
                        if n.contains(':') {
                            let driver = e.path().join("driver");
                            if driver.exists() {
                                hid_id = Some(n.into_owned());
                                break;
                            }
                        }
                    }
                }
                if hid_id.is_some() {
                    break;
                }
            }
        }
    }
    let id = hid_id.ok_or_else(|| "No HID interface id under Spectrum USB".to_string())?;

    let unbind = Path::new("/sys/bus/hid/drivers/hid-generic/unbind");
    let bind = Path::new("/sys/bus/hid/drivers/hid-generic/bind");
    if unbind.exists() {
        if let Err(e) = fs::write(unbind, &id) {
            log::warn!("failed to unbind HID {id} from hid-generic: {e}");
        }
    }
    // Some kernels expose only bind; try binding anyway.
    if bind.exists() {
        std::thread::sleep(Duration::from_millis(50));
        if let Err(e) = fs::write(bind, &id) {
            log::warn!("failed to rebind HID {id} to hid-generic: {e}");
        }
    }
    Ok(format!("Rebound hid-generic for {id}"))
}

/// Scan kernel log this boot for Spectrum USB/HID trouble.
pub fn scan_kernel_rgb_faults() -> Vec<String> {
    let mut out = Vec::new();
    let blob = kernel_log_blob();
    if blob.is_empty() {
        return out;
    }
    let needles = [
        "048d:c197",
        "048D:C197",
        "ITE Device(8258)",
        "ITE Tech. Inc. ITE Device",
    ];
    let fault_words = [
        "disconnect",
        "USB disconnect",
        "reset",
        "cannot",
        "fail",
        "error",
        "timed out",
        "timeout",
        "I/O error",
        "killed",
        "probe failed",
        "usb_submit",
        "overflow",
    ];
    for line in blob.lines() {
        let hit_dev = needles.iter().any(|n| line.contains(n));
        if !hit_dev {
            // Also catch generic hidraw lines paired with earlier context — skip.
            continue;
        }
        let bad = fault_words.iter().any(|w| line.to_lowercase().contains(w));
        if bad {
            let trimmed = line.trim();
            if !out.iter().any(|e: &String| e == trimmed) {
                out.push(trimmed.to_string());
            }
        }
    }
    // Cap noise
    out.truncate(8);
    out
}

fn kernel_log_blob() -> String {
    // Prefer journalctl (works without CAP_SYSLOG on many systems). Only the
    // most recent lines matter — the whole boot log would be a multi-MB
    // subprocess result on the GUI/watchdog hot path.
    let raw = if let Ok(o) = Command::new("journalctl")
        .args(["-k", "-b", "--no-pager", "-o", "cat", "-n", "200"])
        .output()
    {
        if o.status.success() {
            String::from_utf8_lossy(&o.stdout).into_owned()
        } else {
            String::new()
        }
    } else if let Ok(o) = Command::new("dmesg").output() {
        if o.status.success() {
            String::from_utf8_lossy(&o.stdout).into_owned()
        } else {
            String::new()
        }
    } else {
        String::new()
    };
    let mut lines: Vec<&str> = raw.lines().collect();
    if lines.len() > 200 {
        lines.drain(..lines.len() - 200);
    }
    lines.join("\n")
}

struct OpenOptionsCompat;
impl OpenOptionsCompat {
    fn can_open_rw(path: &Path) -> bool {
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .is_ok()
    }
}
