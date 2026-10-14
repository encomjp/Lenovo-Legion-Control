//! GPU working mode (MUX): Hybrid ↔ dGPU only, as Lenovo Vantage /
//! Legion Space offer it.
//!
//! # Backend
//!
//! The only verified Linux path is the firmware's own GameZone WMI method
//! `\_SB.GZFD.WMAA` (GUID `887B54E3-DDDC-4B2C-8B88-68A26A8835D0`,
//! `LENOVO_GAMEZONE_DATA`) invoked through the `acpi_call` module
//! (`/proc/acpi/call`). Every call string lives in [`acpi`].
//!
//! The kernel's `lenovo-wmi-other` firmware-attributes `gpu_mode` and
//! `dgpu_enable` are deliberately **not** used as a MUX backend even when
//! present: in `drivers/platform/x86/lenovo/wmi-other.c` they are Other-Mode
//! features 0x0208 "Set the GPU mode by power limit" and 0x0207 "Set the
//! dedicated Nvidia GPU enabled status" (a dGPU hot-unplug, the same firmware
//! call as GameZone NotifyDGPUStatus), integer-typed with undocumented values
//! and writable only in the Custom thermal mode. Neither selects the MUX, so
//! they are only reported read-only ([`GpuModeStatus::kernel_attrs`]). The
//! kernel `lenovo-wmi-gamezone` driver implements only SmartFan (43/44/45)
//! and exposes no G-Sync/MUX control.
//!
//! # Semantics (cross-checked)
//!
//! * Method ids 40/41/42 = IsSupportGSync / GetGSyncStatus / SetGSyncStatus
//!   (kernel `Documentation/wmi/devices/lenovo-wmi-gamezone.rst` MOF,
//!   LenovoLegionLinux `legion-laptop.c` WMI_METHOD_ID_*GSYNC, Lenovo Legion
//!   Toolkit `WMI.LenovoGameZoneData.cs`, Centurion `legion_wmi.rs`).
//! * Raw G-Sync status 1 = dGPU direct (MUX to the NVIDIA GPU, hybrid off),
//!   0 = hybrid (LLT `HybridModeFeature.cs`: HybridMode Off ⇔ GSync On;
//!   Centurion DSDT notes). LenovoLegionLinux's sysfs `gsync` is *inverted*
//!   (1 = hybrid) — do not copy its values.
//! * SetGSyncStatus only queues an SMI (0xCA sub-command 0x26 / 0x25) and
//!   returns 0; the change happens at the next boot and GetGSyncStatus keeps
//!   describing the *running* mode until then (Centurion, decompiled DSDT of
//!   SMCN19WW/SMCN20WW + live test on a Legion Pro 7 16AFR10H). The firmware
//!   has no readable "pending" value, so the request is recorded in a
//!   volatile file under `/run` tagged with the boot id ([`PENDING_FILE`]).
//! * Running mode is read from the PCI bus: in dGPU-only mode the firmware
//!   hides the iGPU display function (Centurion `active_is_dgpu`).
//!
//! # Safety
//!
//! See [`check_switch`]: verified BIOS family only, explicit acknowledgement,
//! Hybrid refused without a usable iGPU kernel driver (black panel), refused
//! on low battery. BIOS setup (F2) → GPU mode is the recovery path.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// User-facing warning shown before any switch (CLI and GUI).
pub const RISK_TEXT: &str = "Changing the GPU mode rewires the display MUX in firmware. \
It takes effect only after a reboot. If the new mode has no working driver \
(for example Hybrid without the amdgpu/i915/xe driver) the internal screen \
stays black after the reboot. To recover, power on, press F2 to enter BIOS \
setup and change the GPU / graphics mode back, or use an external monitor on \
a port wired to the NVIDIA GPU.";

/// Volatile record of a mode requested during this boot. `/run` is a tmpfs,
/// so the record disappears at the reboot that applies it.
pub const PENDING_FILE: &str = "/run/legion-control-gpu-mode-pending";

/// Refuse switching while discharging below this charge (%): the change needs
/// a reboot and a dead battery mid-POST would leave the user guessing.
pub const MIN_BATTERY_PCT: u32 = 20;

/// BIOS families (first four characters of the BIOS version) where the exact
/// [`acpi`] call strings were verified against the decompiled DSDT and on
/// hardware. Add a family only with a cited source.
///
/// * `SMCN` — Legion Pro 7 16AFR10H (83RU/83RW): Centurion
///   (github.com/arabcian/centurion `docs/TECHNICAL.md`, "HW = decompiled
///   DSDT + live test on the 16AFR10H (SMCN19/20WW)";
///   `crates/centurion-helpers/src/legion_wmi.rs` GPU mode section).
pub const VERIFIED_BIOS_FAMILIES: &[&str] = &["SMCN"];

/// GPU working mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GpuMode {
    /// iGPU drives the internal panel, NVIDIA renders on demand (G-Sync off).
    Hybrid,
    /// MUX wires the panel straight to the NVIDIA GPU (G-Sync on).
    DgpuOnly,
}

impl GpuMode {
    pub fn as_str(self) -> &'static str {
        match self {
            GpuMode::Hybrid => "hybrid",
            GpuMode::DgpuOnly => "dgpu",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            GpuMode::Hybrid => "Hybrid",
            GpuMode::DgpuOnly => "dGPU only",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "hybrid" | "optimus" => Some(GpuMode::Hybrid),
            "dgpu" | "dgpu-only" | "dgpu_only" | "discrete" => Some(GpuMode::DgpuOnly),
            _ => None,
        }
    }
}

/// Exact ACPI call strings for `/proc/acpi/call`, and their encoding.
///
/// acpi_call syntax (github.com/nix-community/acpi_call `acpi_call.c`
/// `parse_acpi_args`): `0xNN` = ACPI Integer, `{..}`/`bXX` = Buffer. Replies:
/// Integer as `0x%x`, Buffer as `{0x01, 0x00}`, failures as `Error: AE_…`,
/// and `not called` when nothing was evaluated.
///
/// WMAA takes (Arg0 instance, Arg1 method id, Arg2 input). The strings below
/// pass Arg2 as an **Integer**, exactly as Centurion does
/// (`wmaa()`: `"{path} 0x0 0x{method:x} 0x{arg:x}"`) and as verified on SMCN
/// firmware. The kernel WMI core passes a Buffer instead; Integer Arg2 is
/// only known to be accepted by the verified families, which is one reason
/// for [`VERIFIED_BIOS_FAMILIES`].
pub mod acpi {
    /// GameZone WMI GUID (`LENOVO_GAMEZONE_DATA`).
    pub const GAMEZONE_GUID: &str = "887B54E3-DDDC-4B2C-8B88-68A26A8835D0";
    /// Method object of the GameZone block on verified firmware.
    pub const WMAA_PATH: &str = "\\_SB.GZFD.WMAA";
    /// GameZone method 40 IsSupportGSync — DSDT returns constant 2 on SMCN.
    pub const IS_SUPPORT_GSYNC: u8 = 0x28;
    /// GameZone method 41 GetGSyncStatus — 1 = dGPU only, 0 = hybrid
    /// (running mode, not the pending one).
    pub const GET_GSYNC_STATUS: u8 = 0x29;
    /// GameZone method 42 SetGSyncStatus — 1 = dGPU only, 0 = hybrid;
    /// returns 0 when the SMI was queued. Applies at next boot.
    pub const SET_GSYNC_STATUS: u8 = 0x2A;

    /// `\_SB.GZFD.WMAA 0x0 0x<method> 0x<arg>` (Integer Arg2).
    pub fn call_string(method: u8, arg: u32) -> String {
        format!("{WMAA_PATH} 0x0 0x{method:x} 0x{arg:x}")
    }

    /// `\_SB.GZFD.WMAA 0x0 0x28 0x0`
    pub fn is_support_gsync() -> String {
        call_string(IS_SUPPORT_GSYNC, 0)
    }

    /// `\_SB.GZFD.WMAA 0x0 0x29 0x0`
    pub fn get_gsync_status() -> String {
        call_string(GET_GSYNC_STATUS, 0)
    }

    /// `\_SB.GZFD.WMAA 0x0 0x2a 0x1` (dGPU only) / `… 0x2a 0x0` (hybrid).
    pub fn set_gsync_status(mode: super::GpuMode) -> String {
        call_string(SET_GSYNC_STATUS, mode_to_arg(mode))
    }

    /// SetGSyncStatus argument: 1 = dGPU only (G-Sync on), 0 = hybrid.
    pub fn mode_to_arg(mode: super::GpuMode) -> u32 {
        match mode {
            super::GpuMode::Hybrid => 0,
            super::GpuMode::DgpuOnly => 1,
        }
    }

    /// GetGSyncStatus value → mode. Anything but 0/1 is not trusted.
    pub fn mode_from_status(v: u64) -> Option<super::GpuMode> {
        match v {
            0 => Some(super::GpuMode::Hybrid),
            1 => Some(super::GpuMode::DgpuOnly),
            _ => None,
        }
    }

    /// Parse one acpi_call reply into an integer.
    pub fn parse_reply(raw: &str) -> Result<u64, String> {
        let s = raw.trim_matches(|c: char| c == '\0' || c.is_whitespace());
        if s.starts_with("Error") {
            return Err(format!("acpi_call: {s}"));
        }
        if s.is_empty() || s == "not called" {
            return Err("acpi_call: method was not evaluated".into());
        }
        let parsed = match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
            Some(h) => u64::from_str_radix(h, 16).ok(),
            None => s.parse().ok(),
        };
        parsed.ok_or_else(|| format!("acpi_call: unexpected reply '{}'", truncate(s)))
    }

    fn truncate(s: &str) -> String {
        s.chars().take(40).collect()
    }

    /// Normalise an ACPI path from sysfs (`\_SB_.GZFD` → `\_SB.GZFD`):
    /// name segments are padded with `_` to four characters.
    pub fn normalize_path(p: &str) -> String {
        let p = p.trim();
        let (root, rest) = match p.strip_prefix('\\') {
            Some(r) => ("\\", r),
            None => ("", p),
        };
        let segs: Vec<&str> = rest
            .split('.')
            .map(|s| {
                let t = s.trim_end_matches('_');
                if t.is_empty() {
                    s
                } else {
                    t
                }
            })
            .collect();
        format!("{root}{}", segs.join("."))
    }
}

/// Everything the CLI/GUI need to show the GPU mode and decide what to offer.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GpuModeStatus {
    /// Firmware reports MUX support through a verified backend.
    pub supported: bool,
    /// BIOS family is on [`VERIFIED_BIOS_FAMILIES`].
    pub model_verified: bool,
    /// Human-readable model verdict (e.g. "SMCN20WW · verified").
    pub model_note: String,
    /// GameZone WMI block present with method object WMAA under `\_SB.GZFD`.
    pub gamezone_present: bool,
    /// `/proc/acpi/call` exists.
    pub acpi_call_loaded: bool,
    /// The running kernel has an acpi_call module that could be loaded.
    pub acpi_call_installed: bool,
    /// "acpi_call:\_SB.GZFD.WMAA" or "none".
    pub backend: String,
    /// Mode running this boot (PCI bus: iGPU visible ⇒ Hybrid).
    pub active: Option<GpuMode>,
    /// Raw firmware GetGSyncStatus interpretation (also the running mode).
    pub firmware: Option<GpuMode>,
    /// Mode requested during this boot (applies at the next boot).
    pub pending: Option<GpuMode>,
    /// Mode the machine will boot into next (pending, else active).
    pub configured: Option<GpuMode>,
    /// A reboot is needed for `configured` to become `active`.
    pub reboot_pending: bool,
    /// Kernel driver bound to the iGPU right now, if any.
    pub igpu_driver: Option<String>,
    /// Why switching to Hybrid would be refused right now (None = allowed).
    pub hybrid_block: Option<String>,
    /// Read-only kernel firmware-attributes `gpu_mode` / `dgpu_enable`
    /// (name=value). Reported only; they are not the MUX.
    pub kernel_attrs: Vec<String>,
    /// Extra explanation (errors, hints).
    pub detail: String,
}

impl GpuModeStatus {
    /// Verified model with the GameZone interface where only the acpi_call
    /// module is missing.
    pub fn needs_acpi_call(&self) -> bool {
        self.model_verified && self.gamezone_present && !self.acpi_call_loaded
    }
}

// ─── hardware probing (read-only sysfs/procfs) ─────────────────────────────

fn read_trim(p: impl AsRef<Path>) -> Option<String> {
    std::fs::read_to_string(p)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// BIOS family = first four alphanumerics of the BIOS version, uppercased.
pub fn bios_family(bios: &str) -> String {
    bios.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(4)
        .collect::<String>()
        .to_ascii_uppercase()
}

pub fn family_verified(family: &str) -> bool {
    VERIFIED_BIOS_FAMILIES.contains(&family)
}

/// Resolve the GameZone WMI method path from sysfs: parent ACPI device path +
/// ".WM" + object_id. `None` when the GUID is absent.
pub fn gamezone_method_path() -> Option<String> {
    let devs = std::fs::read_dir("/sys/bus/wmi/devices").ok()?;
    let mut names: Vec<String> = devs
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.to_ascii_uppercase().starts_with(acpi::GAMEZONE_GUID))
        .collect();
    names.sort();
    for n in names {
        let dev = Path::new("/sys/bus/wmi/devices").join(&n);
        let Some(obj) = read_trim(dev.join("object_id")) else {
            continue;
        };
        // …/PNP0C14:NN/wmi_bus/wmi_bus-PNP0C14:NN/<guid>-N → PNP0C14:NN
        let Ok(real) = std::fs::canonicalize(&dev) else {
            continue;
        };
        let Some(parent) = real.ancestors().nth(3) else {
            continue;
        };
        if let Some(acpi_dev) = read_trim(parent.join("firmware_node/path")) {
            return Some(format!("{}.WM{obj}", acpi::normalize_path(&acpi_dev)));
        }
    }
    None
}

/// Display-class PCI function seen on the bus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayFn {
    pub vendor: String,
    pub driver: Option<String>,
}

fn display_functions() -> Vec<DisplayFn> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir("/sys/bus/pci/devices") else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        if !read_trim(p.join("class")).is_some_and(|c| c.starts_with("0x03")) {
            continue;
        }
        let vendor = read_trim(p.join("vendor")).unwrap_or_default();
        let driver = std::fs::read_link(p.join("driver"))
            .ok()
            .and_then(|l| l.file_name().map(|n| n.to_string_lossy().into_owned()));
        out.push(DisplayFn { vendor, driver });
    }
    out
}

/// AMD (0x1002) or Intel (0x8086) display function = the iGPU on these
/// NVIDIA-dGPU machines.
pub fn is_igpu_vendor(vendor: &str) -> bool {
    matches!(vendor, "0x1002" | "0x8086")
}

/// iGPU kernel drivers for the CPU vendor (`/proc/cpuinfo` vendor_id).
pub fn igpu_drivers_for_cpu(cpu_vendor: &str) -> &'static [&'static str] {
    match cpu_vendor {
        "GenuineIntel" => &["i915", "xe"],
        _ => &["amdgpu"],
    }
}

fn cpu_vendor() -> String {
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("vendor_id"))
                .and_then(|l| l.split(':').nth(1))
                .map(|v| v.trim().to_string())
        })
        .unwrap_or_default()
}

fn kernel_release() -> String {
    read_trim("/proc/sys/kernel/osrelease").unwrap_or_default()
}

/// True when `<name>.ko*` appears in a modules.dep / modules.builtin text.
pub fn module_listed(list: &str, name: &str) -> bool {
    let needle = format!("/{name}.ko");
    list.lines().any(|l| {
        l.split(':')
            .next()
            .is_some_and(|path| path.contains(&needle) || path.starts_with(&needle[1..]))
    })
}

fn module_available(name: &str) -> bool {
    if Path::new("/sys/module").join(name).exists() {
        return true;
    }
    let base = Path::new("/lib/modules").join(kernel_release());
    ["modules.dep", "modules.builtin"]
        .iter()
        .any(|f| std::fs::read_to_string(base.join(f)).is_ok_and(|t| module_listed(&t, name)))
}

/// Collect `blacklist <name>` lines from modprobe.d configuration.
fn modprobe_blacklist() -> Vec<String> {
    let mut out = Vec::new();
    for dir in ["/etc/modprobe.d", "/usr/lib/modprobe.d", "/run/modprobe.d"] {
        let Ok(rd) = std::fs::read_dir(dir) else {
            continue;
        };
        for e in rd.flatten() {
            if let Ok(text) = std::fs::read_to_string(e.path()) {
                out.extend(parse_blacklist(&text));
            }
        }
    }
    out
}

/// Module names from `blacklist <name>` / `install <name> /bin/false` lines.
pub fn parse_blacklist(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            match (it.next(), it.next(), it.next()) {
                (Some("blacklist"), Some(m), _) => Some(m.to_string()),
                (Some("install"), Some(m), Some(cmd))
                    if cmd.ends_with("/false") || cmd.ends_with("/true") =>
                {
                    Some(m.to_string())
                }
                _ => None,
            }
        })
        .collect()
}

/// Inputs of the Hybrid black-screen gate (pure, unit-tested).
#[derive(Debug, Clone, Default)]
pub struct HybridGateInput {
    /// iGPU display function visible on the PCI bus (false in dGPU mode).
    pub igpu_present: bool,
    /// Driver bound to that function.
    pub igpu_driver: Option<String>,
    /// Acceptable iGPU drivers for this CPU.
    pub expected: Vec<String>,
    /// Of `expected`, the ones the running kernel can load or has built in.
    pub available: Vec<String>,
    /// `/proc/cmdline`.
    pub cmdline: String,
    /// modprobe.d blacklisted modules.
    pub blacklisted: Vec<String>,
}

/// Why the next boot in Hybrid would likely leave the panel black, or `None`.
pub fn hybrid_block_reason(i: &HybridGateInput) -> Option<String> {
    let words: Vec<&str> = i.cmdline.split_whitespace().collect();
    if words.contains(&"nomodeset") {
        return Some(
            "kernel command line has 'nomodeset': the iGPU driver cannot drive the panel in Hybrid"
                .into(),
        );
    }
    let usable: Vec<&String> = i
        .expected
        .iter()
        .filter(|d| {
            let on_cmdline_blacklist = words.iter().any(|w| {
                (w.starts_with("modprobe.blacklist=") || w.starts_with("module_blacklist="))
                    && w.split_once('=')
                        .is_some_and(|(_, v)| v.split(',').any(|m| m == d.as_str()))
            });
            let modeset_off = words.contains(&format!("{d}.modeset=0").as_str());
            !on_cmdline_blacklist && !modeset_off && !i.blacklisted.iter().any(|b| b == *d)
        })
        .collect();
    let want = i.expected.join("/");
    if i.igpu_present {
        return match &i.igpu_driver {
            Some(drv) if usable.contains(&drv) => None,
            Some(drv) => Some(format!(
                "the iGPU is bound to '{drv}', not a usable {want} driver"
            )),
            None => Some(format!(
                "no kernel driver ({want}) is bound to the iGPU: the panel would stay black in Hybrid"
            )),
        };
    }
    // dGPU mode hides the iGPU: require the driver in the running kernel.
    if usable.iter().any(|u| i.available.contains(u)) {
        None
    } else if i.expected.iter().any(|e| i.available.contains(e)) {
        Some(format!(
            "the {want} driver is blacklisted or disabled (nomodeset/modeset=0): the panel would stay black in Hybrid"
        ))
    } else {
        Some(format!(
            "the running kernel has no {want} driver: in Hybrid the iGPU drives the panel and it would stay black"
        ))
    }
}

fn hybrid_gate_input(displays: &[DisplayFn]) -> HybridGateInput {
    let igpu = displays.iter().find(|d| is_igpu_vendor(&d.vendor));
    let expected: Vec<String> = match igpu.map(|d| d.vendor.as_str()) {
        Some("0x8086") => vec!["i915".into(), "xe".into()],
        Some(_) => vec!["amdgpu".into()],
        None => igpu_drivers_for_cpu(&cpu_vendor())
            .iter()
            .map(|s| s.to_string())
            .collect(),
    };
    let available = expected
        .iter()
        .filter(|d| module_available(d))
        .cloned()
        .collect();
    HybridGateInput {
        igpu_present: igpu.is_some(),
        igpu_driver: igpu.and_then(|d| d.driver.clone()),
        expected,
        available,
        cmdline: read_trim("/proc/cmdline").unwrap_or_default(),
        blacklisted: modprobe_blacklist(),
    }
}

/// Mode running this boot from the PCI bus: an iGPU display function is
/// visible only in Hybrid. `None` when no NVIDIA dGPU is visible either.
pub fn active_from_displays(displays: &[DisplayFn]) -> Option<GpuMode> {
    let igpu = displays.iter().any(|d| is_igpu_vendor(&d.vendor));
    let dgpu = displays.iter().any(|d| d.vendor == "0x10de");
    match (igpu, dgpu) {
        (true, _) => Some(GpuMode::Hybrid),
        (false, true) => Some(GpuMode::DgpuOnly),
        _ => None,
    }
}

fn acpi_call_installed() -> bool {
    module_available("acpi_call")
}

fn boot_id() -> String {
    read_trim("/proc/sys/kernel/random/boot_id").unwrap_or_default()
}

/// `"<boot_id> <mode>"` → mode when the boot id matches (pure).
pub fn parse_pending(text: &str, current_boot: &str) -> Option<GpuMode> {
    let mut it = text.split_whitespace();
    let (b, m) = (it.next()?, it.next()?);
    if current_boot.is_empty() || b != current_boot {
        return None;
    }
    GpuMode::parse(m)
}

fn read_pending() -> Option<GpuMode> {
    parse_pending(&std::fs::read_to_string(PENDING_FILE).ok()?, &boot_id())
}

fn write_pending(mode: GpuMode) -> Result<(), String> {
    let tmp = format!("{PENDING_FILE}.tmp");
    std::fs::write(&tmp, format!("{} {}\n", boot_id(), mode.as_str()))
        .and_then(|_| std::fs::rename(&tmp, PENDING_FILE))
        .map_err(|e| format!("cannot record pending GPU mode in {PENDING_FILE}: {e}"))
}

fn kernel_attrs() -> Vec<String> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir("/sys/class/firmware-attributes") else {
        return out;
    };
    for e in rd.flatten() {
        if !e
            .file_name()
            .to_string_lossy()
            .starts_with("lenovo-wmi-other")
        {
            continue;
        }
        for a in ["gpu_mode", "dgpu_enable"] {
            if let Some(v) = read_trim(e.path().join("attributes").join(a).join("current_value")) {
                out.push(format!("{a}={v}"));
            }
        }
    }
    out
}

fn ac_online() -> Option<bool> {
    let rd = std::fs::read_dir("/sys/class/power_supply").ok()?;
    let mut seen = false;
    for e in rd.flatten() {
        if read_trim(e.path().join("type")).as_deref() == Some("Mains") {
            seen = true;
            if read_trim(e.path().join("online")).as_deref() == Some("1") {
                return Some(true);
            }
        }
    }
    seen.then_some(false)
}

// ─── /proc/acpi/call (root only) ────────────────────────────────────────────

const ACPI_CALL: &str = "/proc/acpi/call";

/// Serialises write→read pairs inside this process; `flock` below covers
/// other acpi_call users (the module keeps one global result buffer).
static ACPI_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn acpi_call_raw(expr: &str) -> Result<String, String> {
    use std::io::{Read, Write};
    use std::os::unix::io::AsRawFd;
    let _g = ACPI_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(ACPI_CALL)
        .map_err(|e| format!("{ACPI_CALL}: {e}"))?;
    // SAFETY: flock on a valid, owned fd; released when `f` is dropped.
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(format!(
            "{ACPI_CALL} lock: {}",
            std::io::Error::last_os_error()
        ));
    }
    f.write_all(expr.as_bytes())
        .map_err(|e| format!("acpi_call write: {e}"))?;
    // One large read: acpi_call clears its result after the first read.
    let mut buf = vec![0u8; 4096];
    let n = f
        .read(&mut buf)
        .map_err(|e| format!("acpi_call read: {e}"))?;
    buf.truncate(n);
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn wmaa(expr: &str) -> Result<u64, String> {
    let out = acpi_call_raw(expr).and_then(|raw| acpi::parse_reply(&raw));
    log::debug!("gpu_mode: acpi_call '{expr}' → {out:?}");
    out
}

/// Firmware answers cached for the daemon lifetime: support is a DSDT
/// constant and GetGSyncStatus only changes across a reboot. Only successful
/// reads are cached.
static FW_CACHE: std::sync::Mutex<Option<(bool, Option<GpuMode>)>> = std::sync::Mutex::new(None);

fn firmware_read() -> Result<(bool, Option<GpuMode>), String> {
    let mut c = FW_CACHE.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(v) = *c {
        return Ok(v);
    }
    let supported = wmaa(&acpi::is_support_gsync())? > 0;
    let firmware = if supported {
        acpi::mode_from_status(wmaa(&acpi::get_gsync_status())?)
    } else {
        None
    };
    *c = Some((supported, firmware));
    Ok((supported, firmware))
}

// ─── status ────────────────────────────────────────────────────────────────

/// Read-only status without touching ACPI (any user; used by selftest).
pub fn status_passive() -> GpuModeStatus {
    let bios = read_trim("/sys/class/dmi/id/bios_version").unwrap_or_default();
    let family = bios_family(&bios);
    let model_verified = family_verified(&family);
    let path = gamezone_method_path();
    let gamezone_present = path.as_deref() == Some(acpi::WMAA_PATH);
    let displays = display_functions();
    let active = active_from_displays(&displays);
    let gate = hybrid_gate_input(&displays);
    let pending = read_pending();
    let configured = pending.or(active);
    let mut detail = Vec::new();
    if let Some(p) = path.as_deref().filter(|p| *p != acpi::WMAA_PATH) {
        detail.push(format!(
            "GameZone method at {p}, not the verified {}",
            acpi::WMAA_PATH
        ));
    }
    GpuModeStatus {
        supported: false,
        model_verified,
        model_note: if bios.is_empty() {
            "BIOS version unreadable · unverified on this model".into()
        } else if model_verified {
            format!("{bios} · verified ({family})")
        } else {
            format!("{bios} · unverified on this model")
        },
        gamezone_present,
        acpi_call_loaded: Path::new(ACPI_CALL).exists(),
        acpi_call_installed: acpi_call_installed(),
        backend: "none".into(),
        active,
        firmware: None,
        pending,
        configured,
        reboot_pending: matches!((pending, active), (Some(p), Some(a)) if p != a),
        igpu_driver: gate.igpu_driver.clone(),
        hybrid_block: hybrid_block_reason(&gate),
        kernel_attrs: kernel_attrs(),
        detail: detail.join("; "),
    }
}

/// Full status. Queries the firmware (support + G-Sync status, both reads)
/// only as root, on a verified model, with the verified method path and
/// acpi_call loaded.
pub fn status() -> GpuModeStatus {
    let mut st = status_passive();
    // SAFETY: geteuid() is a pure POSIX syscall.
    let root = unsafe { libc::geteuid() } == 0;
    if !(root && st.model_verified && st.gamezone_present && st.acpi_call_loaded) {
        if st.needs_acpi_call() {
            st.detail = if st.acpi_call_installed {
                "load the acpi_call module to manage the GPU mode".into()
            } else {
                "install your distribution's acpi_call package to manage the GPU mode".into()
            };
        }
        return st;
    }
    match firmware_read() {
        Ok((supported, firmware)) => {
            st.supported = supported;
            st.firmware = firmware;
            st.backend = format!("acpi_call:{}", acpi::WMAA_PATH);
            if !supported {
                st.detail = "firmware reports no GPU mode (G-Sync/MUX) support".into();
            }
        }
        Err(e) => st.detail = format!("firmware query failed: {e}"),
    }
    st
}

/// Self-check verdict from a passive status. Fails only when the next boot
/// is set to Hybrid while Hybrid would leave the panel black.
pub fn self_check(st: &GpuModeStatus) -> (bool, String) {
    let name = |m: Option<GpuMode>| m.map_or("unknown", |m| m.as_str());
    if !st.gamezone_present {
        return (true, "not applicable (no GameZone WMAA method)".into());
    }
    let mut parts = vec![
        format!("active {}", name(st.active)),
        st.model_note.clone(),
        format!(
            "acpi_call {}",
            if st.acpi_call_loaded {
                "loaded"
            } else if st.acpi_call_installed {
                "available"
            } else {
                "not installed"
            }
        ),
        format!(
            "iGPU driver {}",
            st.igpu_driver.as_deref().unwrap_or("none")
        ),
    ];
    if st.reboot_pending {
        parts.push(format!("pending {} after reboot", name(st.configured)));
    }
    if !st.kernel_attrs.is_empty() {
        parts.push(format!(
            "kernel attrs {} (not MUX)",
            st.kernel_attrs.join(",")
        ));
    }
    let doomed = st.reboot_pending && st.configured == Some(GpuMode::Hybrid);
    match (&st.hybrid_block, doomed) {
        (Some(why), true) => {
            parts.push(format!("NEXT BOOT HYBRID BUT {why}"));
            (false, parts.join(" · "))
        }
        _ => (true, parts.join(" · ")),
    }
}

// ─── switching ─────────────────────────────────────────────────────────────

/// Everything [`check_switch`] needs (pure, unit-tested).
#[derive(Debug, Clone)]
pub struct SwitchContext {
    pub model_verified: bool,
    pub model_note: String,
    pub gamezone_present: bool,
    pub acpi_call_loaded: bool,
    pub supported: bool,
    pub hybrid_block: Option<String>,
    pub ac_online: Option<bool>,
    pub battery_pct: Option<u32>,
}

/// Safety gates in order. `Ok(())` = the SetGSyncStatus call may be made.
pub fn check_switch(ctx: &SwitchContext, target: GpuMode, acknowledge: bool) -> Result<(), String> {
    if !acknowledge {
        return Err(format!(
            "GPU mode change requires explicit acknowledgement. {RISK_TEXT}"
        ));
    }
    if !ctx.model_verified {
        return Err(format!(
            "GPU mode switching is unverified on this model ({}); only BIOS families {} are allowed",
            ctx.model_note,
            VERIFIED_BIOS_FAMILIES.join(", ")
        ));
    }
    if !ctx.gamezone_present {
        return Err(format!(
            "GameZone WMI method {} not found on this machine",
            acpi::WMAA_PATH
        ));
    }
    if !ctx.acpi_call_loaded {
        return Err("acpi_call is not loaded (Settings → CPU → Features → Load acpi_call, or: sudo modprobe acpi_call)".into());
    }
    if !ctx.supported {
        return Err("the firmware does not report GPU mode (MUX) switching support".into());
    }
    if target == GpuMode::Hybrid {
        if let Some(why) = &ctx.hybrid_block {
            return Err(format!("refusing to switch to Hybrid: {why}"));
        }
    }
    if ctx.ac_online == Some(false) && ctx.battery_pct.is_some_and(|p| p < MIN_BATTERY_PCT) {
        return Err(format!(
            "battery below {MIN_BATTERY_PCT}% on battery power — plug in the charger first"
        ));
    }
    Ok(())
}

/// Switch the MUX (root daemon only). Returns the refreshed status.
pub fn set(target: GpuMode, acknowledge: bool) -> Result<GpuModeStatus, String> {
    let st = status();
    let ctx = SwitchContext {
        model_verified: st.model_verified,
        model_note: st.model_note.clone(),
        gamezone_present: st.gamezone_present,
        acpi_call_loaded: st.acpi_call_loaded,
        supported: st.supported,
        hybrid_block: st.hybrid_block.clone(),
        ac_online: ac_online(),
        battery_pct: crate::battery::capacity(),
    };
    if let Err(e) = check_switch(&ctx, target, acknowledge) {
        log::warn!("gpu_mode: refused switch to {}: {e}", target.as_str());
        return Err(e);
    }
    if st.configured == Some(target) {
        log::info!(
            "gpu_mode: {} already selected — no firmware call",
            target.as_str()
        );
        return Ok(st);
    }
    let expr = acpi::set_gsync_status(target);
    log::info!("gpu_mode: switching to {} via '{expr}'", target.as_str());
    match wmaa(&expr) {
        Ok(0) => {}
        Ok(v) => {
            return Err(format!(
                "the firmware rejected the request (SetGSyncStatus returned {v})"
            ))
        }
        Err(e) => return Err(e),
    }
    let note = if st.active == Some(target) {
        // Back to the running mode: the earlier request is cancelled.
        std::fs::remove_file(PENDING_FILE)
            .or_else(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    Ok(())
                } else {
                    Err(e)
                }
            })
            .err()
            .map(|e| format!("cannot clear {PENDING_FILE}: {e}"))
    } else {
        write_pending(target).err()
    };
    let mut out = status();
    if let Some(n) = note {
        log::warn!("gpu_mode: {n}");
        out.detail = n;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_strings_are_exact() {
        assert_eq!(acpi::is_support_gsync(), "\\_SB.GZFD.WMAA 0x0 0x28 0x0");
        assert_eq!(acpi::get_gsync_status(), "\\_SB.GZFD.WMAA 0x0 0x29 0x0");
        assert_eq!(
            acpi::set_gsync_status(GpuMode::DgpuOnly),
            "\\_SB.GZFD.WMAA 0x0 0x2a 0x1"
        );
        assert_eq!(
            acpi::set_gsync_status(GpuMode::Hybrid),
            "\\_SB.GZFD.WMAA 0x0 0x2a 0x0"
        );
    }

    #[test]
    fn method_ids_match_gamezone_mof() {
        assert_eq!(acpi::IS_SUPPORT_GSYNC, 40);
        assert_eq!(acpi::GET_GSYNC_STATUS, 41);
        assert_eq!(acpi::SET_GSYNC_STATUS, 42);
    }

    #[test]
    fn status_values_decode_and_round_trip() {
        assert_eq!(acpi::mode_from_status(0), Some(GpuMode::Hybrid));
        assert_eq!(acpi::mode_from_status(1), Some(GpuMode::DgpuOnly));
        assert_eq!(acpi::mode_from_status(2), None);
        for m in [GpuMode::Hybrid, GpuMode::DgpuOnly] {
            assert_eq!(acpi::mode_from_status(acpi::mode_to_arg(m) as u64), Some(m));
        }
    }

    #[test]
    fn parse_reply_handles_acpi_call_formats() {
        assert_eq!(acpi::parse_reply("0x2"), Ok(2));
        assert_eq!(acpi::parse_reply("0x0\0\0\n"), Ok(0));
        assert_eq!(acpi::parse_reply("0X1"), Ok(1));
        assert_eq!(acpi::parse_reply("7"), Ok(7));
        assert!(acpi::parse_reply("Error: AE_NOT_FOUND").is_err());
        assert!(acpi::parse_reply("not called").is_err());
        assert!(acpi::parse_reply("").is_err());
        assert!(acpi::parse_reply("{0x01, 0x00}").is_err());
        assert!(acpi::parse_reply("0xzz").is_err());
    }

    #[test]
    fn acpi_path_normalisation() {
        assert_eq!(acpi::normalize_path("\\_SB_.GZFD"), "\\_SB.GZFD");
        assert_eq!(
            acpi::normalize_path("\\_SB_.PCI0.LPC0.EC0_"),
            "\\_SB.PCI0.LPC0.EC0"
        );
        assert_eq!(
            format!("{}.WM{}", acpi::normalize_path("\\_SB_.GZFD\n"), "AA"),
            acpi::WMAA_PATH
        );
    }

    #[test]
    fn mode_parse_and_labels() {
        assert_eq!(GpuMode::parse("Hybrid"), Some(GpuMode::Hybrid));
        assert_eq!(GpuMode::parse("dgpu"), Some(GpuMode::DgpuOnly));
        assert_eq!(GpuMode::parse("dGPU-only"), Some(GpuMode::DgpuOnly));
        assert_eq!(GpuMode::parse("igpu"), None);
        assert_eq!(GpuMode::DgpuOnly.as_str(), "dgpu");
        assert_eq!(
            GpuMode::parse(GpuMode::Hybrid.as_str()),
            Some(GpuMode::Hybrid)
        );
    }

    #[test]
    fn bios_family_allow_list() {
        assert_eq!(bios_family("SMCN20WW"), "SMCN");
        assert_eq!(bios_family("smcn19ww"), "SMCN");
        assert!(family_verified(&bios_family("SMCN20WW")));
        assert!(!family_verified(&bios_family("Q7CN23WW")));
        assert!(!family_verified(&bios_family("")));
    }

    fn ctx() -> SwitchContext {
        SwitchContext {
            model_verified: true,
            model_note: "SMCN20WW · verified (SMCN)".into(),
            gamezone_present: true,
            acpi_call_loaded: true,
            supported: true,
            hybrid_block: None,
            ac_online: Some(true),
            battery_pct: Some(90),
        }
    }

    #[test]
    fn gate_requires_acknowledgement_first() {
        let e = check_switch(&ctx(), GpuMode::DgpuOnly, false).unwrap_err();
        assert!(e.contains("acknowledgement") && e.contains("F2"));
        assert!(check_switch(&ctx(), GpuMode::DgpuOnly, true).is_ok());
        assert!(check_switch(&ctx(), GpuMode::Hybrid, true).is_ok());
    }

    #[test]
    fn gate_refuses_unverified_model_and_missing_backend() {
        let mut c = ctx();
        c.model_verified = false;
        c.model_note = "Q7CN23WW · unverified on this model".into();
        assert!(check_switch(&c, GpuMode::DgpuOnly, true)
            .unwrap_err()
            .contains("unverified"));
        let mut c = ctx();
        c.acpi_call_loaded = false;
        assert!(check_switch(&c, GpuMode::DgpuOnly, true)
            .unwrap_err()
            .contains("acpi_call"));
        let mut c = ctx();
        c.gamezone_present = false;
        assert!(check_switch(&c, GpuMode::DgpuOnly, true).is_err());
        let mut c = ctx();
        c.supported = false;
        assert!(check_switch(&c, GpuMode::DgpuOnly, true)
            .unwrap_err()
            .contains("support"));
    }

    #[test]
    fn gate_blocks_only_hybrid_without_igpu_driver() {
        let mut c = ctx();
        c.hybrid_block = Some("no amdgpu".into());
        assert!(check_switch(&c, GpuMode::Hybrid, true)
            .unwrap_err()
            .contains("Hybrid"));
        // dGPU-only never depends on the iGPU driver.
        assert!(check_switch(&c, GpuMode::DgpuOnly, true).is_ok());
    }

    #[test]
    fn gate_low_battery_only_when_discharging() {
        let mut c = ctx();
        c.ac_online = Some(false);
        c.battery_pct = Some(15);
        assert!(check_switch(&c, GpuMode::DgpuOnly, true)
            .unwrap_err()
            .contains("battery"));
        c.battery_pct = Some(20);
        assert!(check_switch(&c, GpuMode::DgpuOnly, true).is_ok());
        c.ac_online = Some(true);
        c.battery_pct = Some(5);
        assert!(check_switch(&c, GpuMode::DgpuOnly, true).is_ok());
    }

    fn amd_gate() -> HybridGateInput {
        HybridGateInput {
            igpu_present: true,
            igpu_driver: Some("amdgpu".into()),
            expected: vec!["amdgpu".into()],
            available: vec!["amdgpu".into()],
            cmdline: "root=UUID=x quiet".into(),
            blacklisted: vec![],
        }
    }

    #[test]
    fn hybrid_gate_bound_driver_passes() {
        assert_eq!(hybrid_block_reason(&amd_gate()), None);
    }

    #[test]
    fn hybrid_gate_igpu_without_driver_blocks() {
        let mut g = amd_gate();
        g.igpu_driver = None;
        assert!(hybrid_block_reason(&g)
            .unwrap()
            .contains("no kernel driver"));
        g.igpu_driver = Some("vfio-pci".into());
        assert!(hybrid_block_reason(&g).unwrap().contains("vfio-pci"));
    }

    #[test]
    fn hybrid_gate_in_dgpu_mode_needs_module_in_kernel() {
        let mut g = amd_gate();
        g.igpu_present = false;
        g.igpu_driver = None;
        assert_eq!(hybrid_block_reason(&g), None);
        g.available.clear();
        assert!(hybrid_block_reason(&g).unwrap().contains("no amdgpu"));
    }

    #[test]
    fn hybrid_gate_honours_blacklists_and_nomodeset() {
        let mut g = amd_gate();
        g.cmdline = "quiet nomodeset".into();
        assert!(hybrid_block_reason(&g).unwrap().contains("nomodeset"));
        let mut g = amd_gate();
        g.cmdline = "modprobe.blacklist=nouveau,amdgpu".into();
        assert!(hybrid_block_reason(&g).is_some());
        let mut g = amd_gate();
        g.cmdline = "amdgpu.modeset=0".into();
        assert!(hybrid_block_reason(&g).is_some());
        let mut g = amd_gate();
        g.igpu_present = false;
        g.igpu_driver = None;
        g.blacklisted = vec!["amdgpu".into()];
        assert!(hybrid_block_reason(&g).unwrap().contains("blacklisted"));
        // Blacklisting an unrelated module is fine.
        let mut g = amd_gate();
        g.blacklisted = vec!["nouveau".into()];
        g.cmdline = "modprobe.blacklist=nouveau".into();
        assert_eq!(hybrid_block_reason(&g), None);
    }

    #[test]
    fn hybrid_gate_intel_accepts_i915_or_xe() {
        let g = HybridGateInput {
            igpu_present: true,
            igpu_driver: Some("xe".into()),
            expected: vec!["i915".into(), "xe".into()],
            available: vec!["xe".into()],
            ..Default::default()
        };
        assert_eq!(hybrid_block_reason(&g), None);
        assert_eq!(igpu_drivers_for_cpu("GenuineIntel"), &["i915", "xe"]);
        assert_eq!(igpu_drivers_for_cpu("AuthenticAMD"), &["amdgpu"]);
    }

    #[test]
    fn blacklist_parsing() {
        let t = "# comment\nblacklist nouveau\ninstall amdgpu /bin/false\noptions amdgpu x=1\nblacklist\n";
        assert_eq!(parse_blacklist(t), vec!["nouveau", "amdgpu"]);
    }

    #[test]
    fn module_listing_detection() {
        let dep = "kernel/drivers/gpu/drm/amd/amdgpu/amdgpu.ko.zst: kernel/x.ko\nextra/acpi_call.ko.zst:\n";
        assert!(module_listed(dep, "amdgpu"));
        assert!(module_listed(dep, "acpi_call"));
        assert!(!module_listed(dep, "i915"));
        assert!(!module_listed("kernel/drivers/foo/xe_other.ko:", "xe"));
        assert!(module_listed("xe.ko:", "xe"));
    }

    #[test]
    fn active_mode_from_pci_bus() {
        let amd = DisplayFn {
            vendor: "0x1002".into(),
            driver: Some("amdgpu".into()),
        };
        let nv = DisplayFn {
            vendor: "0x10de".into(),
            driver: Some("nvidia".into()),
        };
        assert_eq!(
            active_from_displays(&[nv.clone(), amd.clone()]),
            Some(GpuMode::Hybrid)
        );
        assert_eq!(active_from_displays(&[nv]), Some(GpuMode::DgpuOnly));
        assert_eq!(active_from_displays(&[]), None);
    }

    #[test]
    fn pending_record_is_boot_scoped() {
        assert_eq!(parse_pending("abc dgpu\n", "abc"), Some(GpuMode::DgpuOnly));
        assert_eq!(parse_pending("abc hybrid", "abc"), Some(GpuMode::Hybrid));
        assert_eq!(parse_pending("old dgpu", "abc"), None);
        assert_eq!(parse_pending("abc", "abc"), None);
        assert_eq!(parse_pending(" dgpu", ""), None);
    }

    #[test]
    fn self_check_flags_doomed_hybrid_boot() {
        let mut st = GpuModeStatus {
            gamezone_present: true,
            model_verified: true,
            model_note: "SMCN20WW · verified (SMCN)".into(),
            active: Some(GpuMode::Hybrid),
            configured: Some(GpuMode::Hybrid),
            igpu_driver: Some("amdgpu".into()),
            ..Default::default()
        };
        let (ok, d) = self_check(&st);
        assert!(ok && d.contains("active hybrid"));
        st.active = Some(GpuMode::DgpuOnly);
        st.pending = Some(GpuMode::Hybrid);
        st.reboot_pending = true;
        st.hybrid_block = Some("no amdgpu".into());
        let (ok, d) = self_check(&st);
        assert!(!ok && d.contains("NEXT BOOT HYBRID"));
        st.gamezone_present = false;
        assert!(self_check(&st).0);
    }

    #[test]
    fn needs_acpi_call_only_on_verified_gamezone() {
        let mut st = GpuModeStatus {
            model_verified: true,
            gamezone_present: true,
            ..Default::default()
        };
        assert!(st.needs_acpi_call());
        st.acpi_call_loaded = true;
        assert!(!st.needs_acpi_call());
        st.acpi_call_loaded = false;
        st.model_verified = false;
        assert!(!st.needs_acpi_call());
    }
}
