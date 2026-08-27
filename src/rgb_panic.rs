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

/// journalctl `-g` filter mirroring the device needles that
/// [`scan_kernel_rgb_faults`] matches on (parens/dots regex-escaped).
/// Filtering server-side lets us keep the 200-line network-cost cap without
/// the window ever being able to evict the relevant lines.
const KERNEL_FAULT_GREP: &str =
    r"(048d:c197|048D:C197|ITE Device\(8258\)|ITE Tech\. Inc\. ITE Device)";

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
    log::debug!("rgb-panic: diagnose: probing Spectrum HID {VID}:{PID}");
    let device_path = find_spectrum_hidraw();
    log::debug!(
        "rgb-panic: probe hidraw: {}",
        device_path
            .as_ref()
            .map(|p| format!("found {}", p.display()))
            .unwrap_or_else(|| "no matching node under /sys/class/hidraw".to_string())
    );
    let usb_sysfs = device_path.as_ref().and_then(|p| usb_sysfs_for_hidraw(p));
    log::debug!(
        "rgb-panic: probe usb sysfs: {}",
        usb_sysfs
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "unresolved".to_string())
    );
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

    log::debug!(
        "rgb-panic: probe permissions: {}",
        permission_mode
            .map(|m| format!("{m:04o}"))
            .unwrap_or_else(|| "unreadable".to_string())
    );

    let kernel_faults = scan_kernel_rgb_faults();
    log::debug!(
        "rgb-panic: probe kernel blob grep: {} fault line(s)",
        kernel_faults.len()
    );
    for f in &kernel_faults {
        log::debug!("rgb-panic: kernel fault evidence: {f}");
        details.push(format!("kernel: {f}"));
    }

    let Some(path) = device_path else {
        let health = if kernel_faults.is_empty() {
            Health::NotApplicable
        } else {
            Health::HardwareBroken
        };
        log::info!(
            "rgb-panic: classify: no hidraw node + {} kernel fault line(s) → {health:?}",
            kernel_faults.len()
        );
        if health == Health::HardwareBroken {
            log::warn!(
                "rgb-panic: device gone but kernel saw USB/HID faults — first evidence: {:?}",
                kernel_faults.first().map(|s| s.as_str())
            );
        }
        details.push("Spectrum HID 048d:c197 not found in /sys/class/hidraw".into());
        if !kernel_faults.is_empty() {
            details.push("Kernel recently reported USB/HID errors for this controller".into());
        }
        return Diagnosis {
            health,
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
    log::debug!(
        "rgb-panic: probe hidraw open RDWR {}: {}",
        path.display(),
        if accessible { "ok" } else { "denied/failed" }
    );
    if !accessible {
        log::warn!(
            "rgb-panic: cannot open {} RDWR — udev/uaccess missing?",
            path.display()
        );
        details.push("Cannot open hidraw RDWR — udev/uaccess missing?".into());
    }

    let (ioctl_ok, brightness, ioctl_err) = probe_ioctl();
    log::debug!(
        "rgb-panic: probe ioctl: ok={ioctl_ok} brightness={brightness:?} err={ioctl_err:?}"
    );
    match (ioctl_ok, brightness, &ioctl_err) {
        (true, Some(b), _) => details.push(format!("ioctl OK · brightness {b}/9")),
        (false, _, Some(e)) => details.push(format!("ioctl failed: {e}")),
        _ => details.push("ioctl failed".into()),
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
    log::debug!(
        "rgb-panic: brightness expectation: expect_on={expect_on} (cfg.brightness={} · zones/per-key evaluated), dark_panic={dark_panic}",
        cfg.brightness
    );
    if dark_panic {
        log::warn!(
            "rgb-panic: classify evidence: brightness 0 while saved lighting wants lights on — classic RGB panic / dark hang"
        );
        details.push(
            "Brightness is 0 but saved lighting wants lights on — classic RGB panic / dark hang"
                .into(),
        );
    }

    // The shipped udev rule wants 0660 + uaccess: group rw is the healthy
    // shape. World-writable is neither required nor desired here.
    let bad_perms = permission_mode.map(|m| m & 0o060 != 0o060).unwrap_or(false);
    if bad_perms {
        log::warn!(
            "rgb-panic: classify evidence: mode {} lacks group rw (expected udev 0660 + uaccess)",
            permission_mode
                .map(|m| format!("{m:04o}"))
                .unwrap_or_else(|| "?".to_string())
        );
        details.push(
            "hidraw lacks group rw (expected udev 0660 + uaccess) — userspace RGB blocked".into(),
        );
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
            "Spectrum RGB OK now (recent kernel log shows earlier USB/HID faults)".into()
        };
        (Health::Ok, summary, false)
    };

    if health == Health::Ok {
        log::info!(
            "rgb-panic: classify: {summary} (accessible={accessible} ioctl_ok={ioctl_ok} dark_panic={dark_panic} bad_perms={bad_perms} kernel_faults={})",
            kernel_faults.len()
        );
    } else {
        log::warn!(
            "rgb-panic: classify: {summary} (accessible={accessible} ioctl_ok={ioctl_ok} dark_panic={dark_panic} bad_perms={bad_perms} kernel_faults={})",
            kernel_faults.len()
        );
    }

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
    log::info!(
        "rgb-panic: troubleshoot entered — pre-health {:?} fixable={} kernel_faults={}",
        before.health,
        before.fixable,
        before.kernel_faults.len()
    );

    if before.health == Health::Ok && before.kernel_faults.is_empty() {
        log::info!("rgb-panic: troubleshoot: already healthy — no fix needed");
        steps.push("Already healthy — no fix needed".into());
        return FixReport {
            steps,
            errors,
            after: before,
        };
    }
    if before.health == Health::NotApplicable && before.device_path.is_none() {
        log::warn!("rgb-panic: troubleshoot: no Spectrum device to repair — aborting ladder");
        errors.push("No Spectrum device to repair".into());
        return FixReport {
            steps,
            errors,
            after: before,
        };
    }

    // 1) Permissions — target state is udev 0660 + uaccess, not world-writable.
    if let Some(path) = &before.device_path {
        if let Some(mode) = before.permission_mode {
            if mode & 0o060 != 0o060 {
                log::info!(
                    "rgb-panic: ladder 1/5 permissions: mode {mode:04o} lacks group rw — fixing"
                );
                match fix_permissions(path) {
                    Ok(detail) => {
                        log::info!("rgb-panic: ladder 1/5 permissions ok: {detail}");
                        steps.push(detail);
                    }
                    Err(e) => {
                        log::error!("rgb-panic: ladder 1/5 permissions failed: {e}");
                        errors.push(e);
                    }
                }
            } else {
                log::debug!(
                    "rgb-panic: ladder 1/5 permissions: mode {mode:04o} already group-rw — skipping"
                );
            }
        }
    }

    // 2) Soft HID lighting reset (when device answers, or after perm fix)
    match crate::keyboard::troubleshoot_lighting() {
        Ok(s) => {
            log::info!(
                "rgb-panic: ladder 2/5 soft lighting reset succeeded ({} sub-step(s))",
                s.len()
            );
            steps.push("Soft Spectrum lighting reset".into());
            steps.extend(s);
        }
        Err(e) => {
            log::warn!(
                "rgb-panic: ladder 2/5 soft lighting reset failed: {e} — escalating to hardware kicks"
            );
            errors.push(format!("Soft lighting reset failed: {e}"));
            // 3) USB reset
            if let Some(usb) = before.usb_sysfs.or_else(find_spectrum_usb_sysfs) {
                log::info!("rgb-panic: ladder 3/5 USB reset on {}", usb.display());
                hardware_kick(
                    &usb,
                    &mut steps,
                    &mut errors,
                    "Soft lighting reset after USB/rebind",
                    Duration::from_millis(500),
                );
            } else {
                log::warn!(
                    "rgb-panic: no USB sysfs path for Spectrum — cannot escalate without root daemon"
                );
                errors.push(
                    "No USB sysfs path for Spectrum — cannot reset without root daemon".into(),
                );
            }
        }
    }

    // If still dark panic after soft-only path, escalate
    let mid = diagnose();
    if mid.health != Health::Ok {
        log::warn!(
            "rgb-panic: mid-check health {:?} — considering escalated full reset",
            mid.health
        );
        if let Some(usb) = mid.usb_sysfs.or_else(find_spectrum_usb_sysfs) {
            if !steps.iter().any(|s| s.starts_with("USB reset")) {
                log::info!(
                    "rgb-panic: escalating: USB reset {} + HID rebind + re-light",
                    usb.display()
                );
                hardware_kick(
                    &usb,
                    &mut steps,
                    &mut errors,
                    "Re-applied lighting after escalated reset",
                    Duration::from_millis(400),
                );
            } else {
                log::debug!("rgb-panic: escalation skipped — a USB reset already ran this pass");
            }
        } else {
            log::warn!("rgb-panic: escalation wanted but no USB sysfs path available");
        }
    }

    let after = diagnose();
    log::info!(
        "rgb-panic: troubleshoot finished — post-health {:?}",
        after.health
    );
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

/// USB reset + HID rebind + soft re-light, recording steps/errors.
/// `rebind_sleep` differs between the ladder (500 ms) and the escalation
/// path (400 ms); `relight_label` keeps the two paths distinguishable.
fn hardware_kick(
    usb: &Path,
    steps: &mut Vec<String>,
    errors: &mut Vec<String>,
    relight_label: &str,
    rebind_sleep: Duration,
) {
    log::info!("rgb-panic: USB reset on {}", usb.display());
    match usb_reset(usb) {
        Ok(()) => {
            steps.push(format!("USB reset {}", usb.display()));
            std::thread::sleep(Duration::from_millis(800));
            match hid_rebind_spectrum() {
                Ok(msg) => {
                    log::info!("rgb-panic: HID rebind result: {msg}");
                    steps.push(msg);
                }
                Err(e) => {
                    log::warn!("rgb-panic: HID rebind after USB reset failed: {e}");
                    errors.push(format!("HID rebind: {e}"));
                }
            }
            std::thread::sleep(rebind_sleep);
            match crate::keyboard::troubleshoot_lighting() {
                Ok(s) => {
                    log::info!(
                        "rgb-panic: lighting reapplied after USB reset ({} sub-step(s))",
                        s.len()
                    );
                    steps.push(relight_label.into());
                    steps.extend(s);
                }
                Err(e) => {
                    log::warn!("rgb-panic: lighting still dead after USB reset: {e}");
                    errors.push(format!("Lighting still dead after USB reset: {e}"));
                }
            }
        }
        Err(e) => {
            log::error!("rgb-panic: USB reset failed: {e}");
            errors.push(e);
        }
    }
}

/// True when an auto-fix is warranted (daemon watchdog).
pub fn needs_autofix(d: &Diagnosis) -> bool {
    let warranted = matches!(d.health, Health::SoftIssue | Health::HardwareBroken) && d.fixable;
    log::debug!(
        "rgb-panic: needs_autofix={warranted} (health={:?} fixable={})",
        d.health,
        d.fixable
    );
    warranted
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

/// Bring the Spectrum hidraw node back into the state the packaged udev rule
/// designs for: 0660 + uaccess. Never widen it to world-writable — the root
/// watchdog auto-runs this, and 0666 on a /dev node is a hole, not a fix.
///
/// 1. Ask udev to re-apply its rules via `udevadm trigger` (fine when the
///    binary is missing — we fall through).
/// 2. Re-stat after 300 ms; if the node is still not group-accessible,
///    chmod 0660 directly.
/// 3. Report honestly which route was taken.
fn fix_permissions(path: &Path) -> Result<String, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fn mode_of(p: &Path) -> Option<u32> {
            fs::metadata(p).ok().map(|m| m.permissions().mode() & 0o777)
        }

        let dev = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());

        log::debug!(
            "rgb-panic: fix_permissions: {dev} current mode {}",
            mode_of(path)
                .map(|m| format!("{m:04o}"))
                .unwrap_or_else(|| "?".to_string())
        );

        // 1) Preferred route: let udev re-apply its own rules.
        log::debug!("rgb-panic: fix_permissions: attempting udevadm trigger for {dev}");
        match Command::new("udevadm")
            .args(["trigger", "-s", "hidraw", &format!("--name-match={dev}")])
            .output()
        {
            Ok(out) if out.status.success() => {
                log::debug!(
                    "rgb-panic: fix_permissions: udevadm trigger exit 0 — re-stat after settle delay"
                );
                // udev applies events asynchronously — give it a beat, then re-stat.
                std::thread::sleep(Duration::from_millis(300));
                if let Some(mode) = mode_of(path) {
                    if mode & 0o060 == 0o060 {
                        log::info!(
                            "rgb-panic: fix_permissions: udevadm route fixed {dev} → mode {mode:04o}"
                        );
                        return Ok(format!(
                            "udevadm trigger reapplied udev rules for {dev} (mode now {mode:04o})"
                        ));
                    }
                }
                log::debug!(
                    "rgb-panic: fix_permissions: udevadm route ineffective — still not group-rw; falling back to chmod"
                );
            }
            Ok(out) => log::debug!(
                "udevadm trigger exited {:?} for {dev}; falling back to chmod",
                out.status.code()
            ),
            Err(e) => log::debug!("udevadm unusable ({:?}); falling back to chmod", e.kind()),
        }

        // 2) Fallback: match the packaged udev rule's mode directly (0660, never 0666).
        let meta = fs::metadata(path).map_err(|e| {
            log::error!(
                "rgb-panic: fix_permissions: stat {} failed: {e}",
                path.display()
            );
            format!("stat {}: {e}", path.display())
        })?;
        let mut perms = meta.permissions();
        perms.set_mode(0o660);
        log::info!(
            "rgb-panic: fix_permissions: chmod {} → target mode 0660",
            path.display()
        );
        fs::set_permissions(path, perms).map_err(|e| {
            log::error!(
                "rgb-panic: fix_permissions: chmod {} failed: {e}",
                path.display()
            );
            format!("chmod {}: {e} (needs root daemon)", path.display())
        })?;
        log::info!(
            "rgb-panic: fix_permissions: post-fix stat {}: mode {}",
            path.display(),
            mode_of(path)
                .map(|m| format!("{m:04o}"))
                .unwrap_or_else(|| "unreadable".to_string())
        );
        Ok(format!(
            "chmod 0660 {} (udevadm trigger unavailable or ineffective)",
            path.display()
        ))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err("permissions fix only on Linux".into())
    }
}

fn usb_reset(usb: &Path) -> Result<(), String> {
    log::info!(
        "rgb-panic: usb_reset: device {} — attempting port reset / re-authorization",
        usb.display()
    );
    let reset = usb.join("reset");
    if reset.exists() {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .open(&reset)
            .map_err(|e| format!("Cannot write {}: {e} (needs root daemon)", reset.display()))?;
        f.write_all(b"1")
            .map_err(|e| format!("USB reset failed: {e}"))?;
        log::info!("rgb-panic: usb_reset: {} accepted the reset", usb.display());
        return Ok(());
    }

    // Modern kernels / hubs often don't expose a 'reset' attribute directly on composite child nodes.
    // Fallback: toggle 'authorized' (0 -> 1) which forces USB kernel re-enumeration and port reset.
    let auth = usb.join("authorized");
    if auth.exists() {
        log::info!(
            "rgb-panic: toggling authorized attribute at {}",
            auth.display()
        );
        fs::write(&auth, b"0").map_err(|e| format!("authorized toggle (off) failed: {e}"))?;
        std::thread::sleep(Duration::from_millis(200));
        fs::write(&auth, b"1").map_err(|e| format!("authorized toggle (on) failed: {e}"))?;
        log::info!(
            "rgb-panic: USB re-authorization completed at {}",
            usb.display()
        );
        return Ok(());
    }

    log::debug!(
        "rgb-panic: usb_reset: neither reset nor authorized attribute found at {}",
        usb.display()
    );
    Err(format!(
        "no reset or authorization attribute at {}",
        usb.display()
    ))
}

/// Resolve `<iface>/driver` (a sysfs symlink) to the symlink path plus the
/// driver basename it points at.
fn iface_driver(iface: &Path) -> Option<(PathBuf, String)> {
    let link = iface.join("driver");
    let name = fs::read_link(&link)
        .ok()?
        .file_name()?
        .to_string_lossy()
        .into_owned();
    Some((link, name))
}

/// Bounce hid-generic for the Spectrum HID interface.
///
/// Only touches interfaces actually bound to `hid-generic`; refuses to
/// unbind/bind foreign drivers (usbhid etc.) blindly. Returns `Err`
/// summarizing every step that failed (with io error kinds) instead of
/// pretending success; on success the driver symlink is verified to have
/// reappeared and that is noted in the message.
fn hid_rebind_spectrum() -> Result<String, String> {
    // Find hid device symlink under the USB device and bounce hid-generic.
    let usb = find_spectrum_usb_sysfs().ok_or_else(|| {
        log::warn!("rgb-panic: hid_rebind: Spectrum USB path gone — cannot locate interfaces");
        "Spectrum USB path gone".to_string()
    })?;
    let mut hid_iface: Option<(String, PathBuf, String)> = None;
    if let Ok(rd) = fs::read_dir(&usb) {
        for e in rd.flatten() {
            let n = e.file_name();
            let n = n.to_string_lossy();
            // e.g. 3-2.4:1.0
            if n.contains(':') {
                if let Some((link, drv)) = iface_driver(&e.path()) {
                    log::debug!("rgb-panic: hid_rebind: interface {n} bound to driver {drv}");
                    hid_iface = Some((n.into_owned(), link, drv));
                    break;
                }
            }
        }
    }
    // Also search one level of interfaces
    if hid_iface.is_none() {
        if let Ok(rd) = fs::read_dir(&usb) {
            for iface in rd.flatten() {
                if let Ok(sub) = fs::read_dir(iface.path()) {
                    for e in sub.flatten() {
                        let n = e.file_name();
                        let n = n.to_string_lossy();
                        if n.contains(':') {
                            if let Some((link, drv)) = iface_driver(&e.path()) {
                                log::debug!(
                                    "rgb-panic: hid_rebind: interface {n} bound to driver {drv} (one level down)"
                                );
                                hid_iface = Some((n.into_owned(), link, drv));
                                break;
                            }
                        }
                    }
                }
                if hid_iface.is_some() {
                    break;
                }
            }
        }
    }
    let Some((id, driver_link, driver)) = hid_iface else {
        log::warn!(
            "rgb-panic: hid_rebind: no HID interface id found under {}",
            usb.display()
        );
        return Err("No HID interface id under Spectrum USB".into());
    };
    if driver != "hid-generic" {
        // Writing to unbind/bind for a foreign driver could drop hardware we
        // do not own (usbhid owns more than the lighting node) — refuse and
        // say what we saw instead of writing blindly.
        log::info!(
            "rgb-panic: hid_rebind: skipping {id} — bound to {driver}, not hid-generic (refusing foreign-driver bounce)"
        );
        return Ok(format!(
            "Skipped HID rebind: {id} is bound to {driver}, not hid-generic"
        ));
    }
    log::info!("rgb-panic: hid_rebind: bouncing {id}: unbind/bind via hid-generic");

    let unbind = Path::new("/sys/bus/hid/drivers/hid-generic/unbind");
    let bind = Path::new("/sys/bus/hid/drivers/hid-generic/bind");
    let mut failures: Vec<String> = Vec::new();

    if let Err(e) = fs::write(unbind, &id) {
        log::warn!("failed to unbind HID {id} from hid-generic: {e}");
        failures.push(format!("unbind {id}: {:?} ({e})", e.kind()));
    } else {
        log::debug!("rgb-panic: hid_rebind: unbound {id} from hid-generic");
    }
    std::thread::sleep(Duration::from_millis(50));
    if let Err(e) = fs::write(bind, &id) {
        log::warn!("failed to rebind HID {id} to hid-generic: {e}");
        failures.push(format!("bind {id}: {:?} ({e})", e.kind()));
    } else {
        log::debug!("rgb-panic: hid_rebind: bound {id} back to hid-generic");
    }

    if !failures.is_empty() {
        log::error!(
            "rgb-panic: hid_rebind: incomplete for {id}: {}",
            failures.join("; ")
        );
        return Err(format!("HID rebind incomplete: {}", failures.join("; ")));
    }

    // Verify the driver really came back before claiming success.
    match fs::read_link(&driver_link)
        .ok()
        .and_then(|t| t.file_name().map(|f| f.to_string_lossy().into_owned()))
    {
        Some(d) if d == "hid-generic" => {
            log::info!("rgb-panic: hid_rebind: driver symlink verified as hid-generic for {id}");
            Ok(format!(
                "Rebound hid-generic for {id} (driver symlink verified)"
            ))
        }
        other => {
            log::error!(
                "rgb-panic: hid_rebind: symlink verify failed for {id}: {:?}",
                other.as_deref()
            );
            Err(format!(
                "HID rebind incomplete: bind accepted for {id} but driver symlink is {}, not hid-generic",
                other.as_deref().unwrap_or("gone")
            ))
        }
    }
}

/// Scan recent kernel logs for Spectrum USB/HID trouble: the current boot via
/// journalctl, else whatever is in the kernel ring buffer via dmesg.
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

/// Run a command and return its stdout on success; `None` when the spawn
/// failed or the command exited non-zero, so callers can fall through to
/// the next log source (e.g. journalctl → dmesg).
fn capture_stdout(name: &str, cmd: &mut Command) -> Option<String> {
    match cmd.output() {
        Ok(o) if o.status.success() => {
            let blob = String::from_utf8_lossy(&o.stdout).into_owned();
            log::debug!(
                "rgb-panic: {name} exit 0 · {} line(s) · {} bytes",
                blob.lines().count(),
                blob.len()
            );
            Some(blob)
        }
        Ok(o) => {
            log::warn!(
                "rgb-panic: {name} exited {:?} — falling through",
                o.status.code()
            );
            None
        }
        Err(e) => {
            log::debug!("rgb-panic: {name} spawn failed ({e})");
            None
        }
    }
}

fn kernel_log_blob() -> String {
    // Prefer journalctl (works without CAP_SYSLOG on many systems). `-g`
    // narrows to Spectrum-related lines server-side and `-n 200` keeps the
    // subprocess result small on the GUI/watchdog hot path — together, the
    // cap can no longer evict the lines we care about. The dmesg fallback
    // stays untrimmed: the kernel ring buffer bounds its size already.
    log::debug!(
        "rgb-panic: kernel_log_blob: spawning journalctl -k -b -g <spectrum needles> -n 200"
    );
    if let Some(blob) = capture_stdout(
        "journalctl",
        Command::new("journalctl").args([
            "-k",
            "-b",
            "--no-pager",
            "-o",
            "cat",
            "-g",
            KERNEL_FAULT_GREP,
            "-n",
            "200",
        ]),
    ) {
        return blob;
    }
    log::debug!("rgb-panic: kernel_log_blob: spawning dmesg fallback");
    capture_stdout("dmesg", &mut Command::new("dmesg")).unwrap_or_default()
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
