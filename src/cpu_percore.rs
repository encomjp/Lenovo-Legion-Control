//! Borrowed per-CPU controls — ported from LLLT `sysfs_drivers/cpu_control.rs`.
//!
//! Adapts LLLT topology discovery, per-CPU cpufreq and topology reads, and
//! privileged writes to this crate's style: `std::fs` only, `BTreeSet` for
//! stable ordering, `std::io::Error` with path-bearing messages (mirrors
//! `fans.rs:90`), and graceful handling of missing cpufreq dirs (offline cores)
//! and AMD `amd-pstate` quirks.
//!
//! Divergences / fixes vs LLLT are documented inline and in the module-level
//! report at the end of the file.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

// ─── Constants ────────────────────────────────────────────────────────────

const SYSFS_SYSTEM_CPU: &str = "/sys/devices/system/cpu/";

// ─── Helpers ──────────────────────────────────────────────────────────────

/// Path-bearing I/O error, same style as `fans.rs::set_target` ctx.
fn io_err(kind: std::io::ErrorKind, msg: String) -> std::io::Error {
    std::io::Error::new(kind, msg)
}

fn read_trim(path: &Path) -> std::io::Result<String> {
    fs::read_to_string(path)
        .map(|s| s.trim().to_string())
        .map_err(|e| {
            io_err(
                e.kind(),
                format!(
                    "reading {} failed: {e} (kind={:?})",
                    path.display(),
                    e.kind()
                ),
            )
        })
}

fn read_trim_opt(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

fn parse_u32_trim(s: &str, path: &Path) -> std::io::Result<u32> {
    s.trim().parse::<u32>().map_err(|e| {
        io_err(
            std::io::ErrorKind::InvalidData,
            format!(
                "parsing u32 from {} (value={:?}): {e}",
                path.display(),
                s.trim()
            ),
        )
    })
}

fn read_u32(path: &Path) -> std::io::Result<u32> {
    let s = read_trim(path)?;
    parse_u32_trim(&s, path)
}

fn read_u32_opt(path: &Path) -> Option<u32> {
    read_trim_opt(path).and_then(|s| s.parse::<u32>().ok())
}

fn read_bool01_opt(path: &Path) -> Option<bool> {
    match read_trim_opt(path)?.as_str() {
        "1" => Some(true),
        "0" => Some(false),
        other => other.parse::<u8>().ok().map(|v| v != 0),
    }
}

// ─── parse_cpu_range ────────────────────────────────────────────────────

/// Parse a Linux CPU range string like `"0-3,7,12-15"` into a sorted set.
///
/// Splits on `','` then on `'-'` (mirrors LLLT `utils::parse_cpu_range_value`
/// lines 7-44). Empty/whitespace-only input yields an empty set (matches LLLT
/// behaviour for the `offline` file on this 9955HX3D box, which is a lone
/// newline). Duplicate CPU ids are rejected with `ErrorKind::InvalidData`
/// (LLLT uses `Other` with `"CPU {} was not added !"` — we preserve the
/// message prefix for diagnostics).
///
/// # Errors
/// - `InvalidData` if a token is not a `u32`/`u32-u32` range, if a range has
///   more than one `-`, or if a CPU id appears twice.
pub fn parse_cpu_range(s: &str) -> std::io::Result<BTreeSet<u32>> {
    let mut out = BTreeSet::new();
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Ok(out);
    }
    for token in trimmed.split(',') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        // Split on '-' — exactly 1 or 2 parts are valid.
        let parts: Vec<&str> = token.split('-').collect();
        let (start, end) = match parts.as_slice() {
            [single] => {
                let v = single.trim().parse::<u32>().map_err(|e| {
                    io_err(
                        std::io::ErrorKind::InvalidData,
                        format!("parse_cpu_range: invalid value {:?}: {e}", single.trim()),
                    )
                })?;
                (v, v)
            }
            [a, b] => {
                let lo = a.trim().parse::<u32>().map_err(|e| {
                    io_err(
                        std::io::ErrorKind::InvalidData,
                        format!("parse_cpu_range: invalid range start {:?}: {e}", a.trim()),
                    )
                })?;
                let hi = b.trim().parse::<u32>().map_err(|e| {
                    io_err(
                        std::io::ErrorKind::InvalidData,
                        format!("parse_cpu_range: invalid range end {:?}: {e}", b.trim()),
                    )
                })?;
                if hi < lo {
                    return Err(io_err(
                        std::io::ErrorKind::InvalidData,
                        format!("parse_cpu_range: reversed range {lo}-{hi} in token {token:?}"),
                    ));
                }
                (lo, hi)
            }
            _ => {
                return Err(io_err(
                    std::io::ErrorKind::InvalidData,
                    format!("parse_cpu_range: malformed token {token:?} (too many '-')"),
                ))
            }
        };
        for cpu in start..=end {
            if !out.insert(cpu) {
                // Duplicate detection — same message prefix as LLLT.
                return Err(io_err(
                    std::io::ErrorKind::InvalidData,
                    format!("CPU {cpu} was not added ! (duplicate in {s:?})"),
                ));
            }
        }
    }
    Ok(out)
}

/// Read a sysfs file that contains a CPU range string and parse it.
fn parse_cpu_range_file(path: &Path) -> std::io::Result<BTreeSet<u32>> {
    let raw = fs::read_to_string(path).map_err(|e| {
        io_err(
            e.kind(),
            format!(
                "reading {} failed: {e} (kind={:?})",
                path.display(),
                e.kind()
            ),
        )
    })?;
    parse_cpu_range(&raw).map_err(|e| {
        // Attach path context to parse errors.
        io_err(
            e.kind(),
            format!("parsing {} (value={:?}): {e}", path.display(), raw.trim()),
        )
    })
}

// ─── CpuTopology ────────────────────────────────────────────────────────

/// System-wide CPU topology discovered from `/sys/devices/system/cpu/*`.
///
/// Borrowed from LLLT `Topology` (cpu_control.rs:43-52) but trimmed to the
/// four files that exist on every kernel (including this AMD box):
/// `kernel_max`, `offline`, `online`, `possible`, `present`.
/// LLLT's `atom`/`core` (hybrid Intel) are intentionally omitted — they do not
/// exist on AMD and would make `new()` spuriously fail.
#[derive(Debug, Clone, Default)]
pub struct CpuTopology {
    pub kernel_max: u32,
    pub offline: BTreeSet<u32>,
    pub online: BTreeSet<u32>,
    pub possible: BTreeSet<u32>,
    pub present: BTreeSet<u32>,
}

/// Field setter used by [`CpuTopology::refresh`]'s table-driven loop.
type TopologySetter = fn(&mut CpuTopology, BTreeSet<u32>);

impl CpuTopology {
    /// Discover topology from sysfs.
    ///
    /// Gated on `Path::exists` only for the integer `kernel_max` file (always
    /// present). Range files are read via [`parse_cpu_range`]; an empty
    /// `offline` file (this machine) yields an empty set rather than an error.
    pub fn new() -> std::io::Result<Self> {
        let mut topo = Self::default();
        topo.refresh()?;
        Ok(topo)
    }

    /// Re-read all fields from sysfs (useful for polling).
    pub fn refresh(&mut self) -> std::io::Result<&Self> {
        let km_path = Path::new(SYSFS_SYSTEM_CPU).join("kernel_max");
        self.kernel_max = read_u32(&km_path)?;

        // Table-driven to mirror LLLT's vec-of-closures style but without
        // boxing.
        let files: [(&str, TopologySetter); 4] = [
            ("offline", |t, v| t.offline = v),
            ("online", |t, v| t.online = v),
            ("possible", |t, v| t.possible = v),
            ("present", |t, v| t.present = v),
        ];
        for (name, setter) in files {
            let p = Path::new(SYSFS_SYSTEM_CPU).join(name);
            // `offline` may be empty; `parse_cpu_range_file` handles that.
            // If a file is genuinely missing we surface NotFound with path.
            if !p.exists() {
                return Err(io_err(
                    std::io::ErrorKind::NotFound,
                    format!("topology file not found: {}", p.display()),
                ));
            }
            let set = parse_cpu_range_file(&p)?;
            setter(self, set);
        }
        Ok(self)
    }

    /// Convenience: is a logical CPU id present in the `possible` set?
    pub fn contains(&self, cpu: u32) -> bool {
        self.possible.contains(&cpu)
    }
}

// ─── CpuFreq ────────────────────────────────────────────────────────────

/// Per-CPU frequency state, read from `/sys/devices/system/cpu/cpuN/cpufreq/*`.
///
/// Fields that may be absent (offline core, or `amd-pstate` without
/// `base_frequency`) are `Option`/`Vec` and a missing `cpufreq` directory is
/// **not** an error — the struct is returned with `None`/empty fields. This
/// satisfies the gating requirement: callers can iterate `topology.possible`
/// and always get `Ok`, even for offline cores that have no `cpufreq` dir.
///
/// LLLT divergence: `base_frequency` is optional (AMD lacks it) and
/// `scaling_setspeed` is read from its real file (LLLT bug read
/// `cpuinfo_max_freq` into `scaling_setspeed` — fixed here).
#[derive(Debug, Clone, Default)]
pub struct CpuFreq {
    pub cpu: u32,
    /// `None` if `/sys/.../cpuN/online` does not exist (cpu0 — always online).
    /// `Some(true/false)` otherwise.
    pub online: Option<bool>,
    pub available_governors: Vec<String>,
    pub governor: Option<String>,
    pub cur_freq: Option<u32>,
    pub min_freq: Option<u32>,
    pub max_freq: Option<u32>,

    // ── Extra optional fields preserved for AMD quirk / bug-fix fidelity ──
    /// `cpuinfo_max_freq` (kHz) if present.
    pub cpuinfo_max_freq: Option<u32>,
    /// `base_frequency` (kHz) — absent on `amd-pstate`, hence `Option`.
    pub base_frequency: Option<u32>,
    /// `scaling_setspeed` raw string (`<unsupported>` on amd-pstate) — not a
    /// frequency on this driver, correctly read from its own file.
    pub scaling_setspeed: Option<String>,
    /// `scaling_driver` (e.g. `amd-pstate-epp`).
    pub scaling_driver: Option<String>,
}

impl CpuFreq {
    /// Read frequency state for one CPU.
    ///
    /// Never errors solely because `cpufreq` is missing (offline core) — in
    /// that case all `Option` frequency fields remain `None` and
    /// `available_governors` is empty, but `online` is still populated when
    /// the `online` file exists.
    pub fn new(cpu: u32) -> std::io::Result<Self> {
        let mut out = Self {
            cpu,
            ..Self::default()
        };
        out.refresh()?;
        Ok(out)
    }

    /// Re-read from sysfs.
    pub fn refresh(&mut self) -> std::io::Result<&Self> {
        let cpu = self.cpu;
        let base = PathBuf::from(format!("{SYSFS_SYSTEM_CPU}cpu{cpu}"));
        let cpufreq = base.join("cpufreq");

        // ── online ──
        let online_path = base.join("online");
        self.online = if online_path.exists() {
            // File contains "0" or "1" with newline.
            read_bool01_opt(&online_path)
        } else {
            None // cpu0 has no online file → always online, signalled by None
        };

        // ── cpufreq gating ──
        if !cpufreq.exists() {
            // Offline cores have no cpufreq dir — not an error.
            self.available_governors.clear();
            self.governor = None;
            self.cur_freq = None;
            self.min_freq = None;
            self.max_freq = None;
            self.cpuinfo_max_freq = None;
            self.base_frequency = None;
            self.scaling_setspeed = None;
            self.scaling_driver = None;
            return Ok(self);
        }

        // Helper to read an optional u32 from cpufreq/<file>.
        let read_freq_opt = |name: &str| -> Option<u32> {
            let p = cpufreq.join(name);
            if !p.exists() {
                return None;
            }
            read_u32_opt(&p)
        };

        // available_governors: space-split, trimmed, filtered.
        let avail_path = cpufreq.join("scaling_available_governors");
        self.available_governors = if avail_path.exists() {
            read_trim_opt(&avail_path)
                .map(|s| {
                    s.split_whitespace()
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty())
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        self.governor = read_trim_opt(&cpufreq.join("scaling_governor")).filter(|s| !s.is_empty());
        self.cur_freq = read_freq_opt("scaling_cur_freq");
        self.min_freq = read_freq_opt("scaling_min_freq");
        self.max_freq = read_freq_opt("scaling_max_freq");
        self.cpuinfo_max_freq = read_freq_opt("cpuinfo_max_freq");
        self.base_frequency = read_freq_opt("base_frequency");
        // Fix LLLT bug: read actual scaling_setspeed, not cpuinfo_max_freq.
        self.scaling_setspeed =
            read_trim_opt(&cpufreq.join("scaling_setspeed")).filter(|s| !s.is_empty());
        self.scaling_driver =
            read_trim_opt(&cpufreq.join("scaling_driver")).filter(|s| !s.is_empty());

        Ok(self)
    }

    /// Whether this CPU's cpufreq directory exists.
    pub fn has_cpufreq(&self) -> bool {
        Path::new(&format!("{SYSFS_SYSTEM_CPU}cpu{}/cpufreq", self.cpu)).exists()
    }
}

/// Read `CpuFreq` for every CPU in `topology.possible`.
///
/// Convenience that never errors on a per-CPU basis — offline cores are
/// represented with `None` frequency fields.
pub fn all_cpu_freqs(topo: &CpuTopology) -> std::io::Result<Vec<CpuFreq>> {
    let mut out = Vec::with_capacity(topo.possible.len());
    for cpu in topo.possible.iter().copied() {
        out.push(CpuFreq::new(cpu)?);
    }
    Ok(out)
}

// ─── CpuCoreInfo ────────────────────────────────────────────────────────

/// Per-CPU topology masks from `/sys/devices/system/cpu/cpuN/topology/*`.
///
/// Minimal struct required by the task exposes `cpu`, `core_id`, and
/// `package_id`; extra AMD-relevant fields `cluster_id`/`die_id` are also
/// exposed as `Option` so callers that probe Intel hybrid details still work
/// without failing on AMD (where they may be sentinel `65535`).
#[derive(Debug, Clone, Default)]
pub struct CpuCoreInfo {
    pub cpu: u32,
    pub core_id: Option<u32>,
    pub package_id: Option<u32>,
    /// `cluster_id` if the file exists (hybrid Intel, or `65535` sentinel on AMD).
    pub cluster_id: Option<u32>,
    /// `die_id` if the file exists.
    pub die_id: Option<u32>,
}

impl CpuCoreInfo {
    pub fn new(cpu: u32) -> std::io::Result<Self> {
        let mut out = Self {
            cpu,
            ..Self::default()
        };
        out.refresh()?;
        Ok(out)
    }

    pub fn refresh(&mut self) -> std::io::Result<&Self> {
        let base = PathBuf::from(format!("{SYSFS_SYSTEM_CPU}cpu{}/topology", self.cpu));
        // Each file is optional — missing → None, not error. This handles
        // kernels where e.g. `cluster_id` only exists on hybrid parts.
        let read_opt = |name: &str| -> Option<u32> {
            let p = base.join(name);
            if !p.exists() {
                return None;
            }
            read_u32_opt(&p)
        };
        self.core_id = read_opt("core_id");
        self.package_id = read_opt("physical_package_id");
        self.cluster_id = read_opt("cluster_id");
        self.die_id = read_opt("die_id");
        Ok(self)
    }
}

/// Read `CpuCoreInfo` for every CPU in `topology.possible`.
pub fn all_core_infos(topo: &CpuTopology) -> std::io::Result<Vec<CpuCoreInfo>> {
    let mut out = Vec::with_capacity(topo.possible.len());
    for cpu in topo.possible.iter().copied() {
        out.push(CpuCoreInfo::new(cpu)?);
    }
    Ok(out)
}

// ─── Privileged actions (daemon-side) ───────────────────────────────────

fn cpufreq_dir(cpu: u32) -> PathBuf {
    PathBuf::from(format!("{SYSFS_SYSTEM_CPU}cpu{cpu}/cpufreq"))
}

fn ensure_cpufreq_exists(cpu: u32) -> std::io::Result<PathBuf> {
    let dir = cpufreq_dir(cpu);
    if !dir.exists() {
        return Err(io_err(
            std::io::ErrorKind::NotFound,
            format!(
                "cpu{cpu} cpufreq not present ({} missing — core offline or driver not loaded)",
                dir.display()
            ),
        ));
    }
    Ok(dir)
}

fn write_with_ctx(path: &Path, value: &str, ctx: &str) -> std::io::Result<()> {
    if !path.exists() {
        return Err(io_err(
            std::io::ErrorKind::NotFound,
            format!("{ctx}: file not found: {}", path.display()),
        ));
    }
    fs::write(path, value).map_err(|e| {
        io_err(
            e.kind(),
            format!(
                "{ctx} failed on {}: {e} (kind={:?}, raw={})",
                path.display(),
                e.kind(),
                e.raw_os_error().unwrap_or(-1)
            ),
        )
    })
}

/// Set `scaling_governor` for one CPU.
///
/// Validates `gov` against `scaling_available_governors` when that file is
/// present, mirroring LLLT's intent but surfacing a clear `InvalidInput` if
/// the governor is not offered. Requires root (to be called via daemon).
pub fn set_governor(cpu: u32, gov: &str) -> std::io::Result<()> {
    let dir = ensure_cpufreq_exists(cpu)?;
    let avail_path = dir.join("scaling_available_governors");
    if avail_path.exists() {
        if let Some(raw) = read_trim_opt(&avail_path) {
            let avail: BTreeSet<String> = raw.split_whitespace().map(|s| s.to_string()).collect();
            if !avail.contains(gov) {
                return Err(io_err(
                    std::io::ErrorKind::InvalidInput,
                    format!(
                        "governor {gov:?} not in available {} for cpu{cpu}: {:?}",
                        avail_path.display(),
                        avail
                    ),
                ));
            }
        }
    }
    let target = dir.join("scaling_governor");
    write_with_ctx(
        &target,
        gov,
        &format!("set_governor(cpu={cpu}, gov={gov:?})"),
    )
}

/// Set `scaling_min_freq` (kHz) for one CPU. Requires root (daemon).
pub fn set_freq_min(cpu: u32, khz: u32) -> std::io::Result<()> {
    let dir = ensure_cpufreq_exists(cpu)?;
    let target = dir.join("scaling_min_freq");
    write_with_ctx(
        &target,
        &khz.to_string(),
        &format!("set_freq_min(cpu={cpu}, khz={khz})"),
    )
}

/// Set `scaling_max_freq` (kHz) for one CPU. Requires root (daemon).
pub fn set_freq_max(cpu: u32, khz: u32) -> std::io::Result<()> {
    let dir = ensure_cpufreq_exists(cpu)?;
    let target = dir.join("scaling_max_freq");
    write_with_ctx(
        &target,
        &khz.to_string(),
        &format!("set_freq_max(cpu={cpu}, khz={khz})"),
    )
}

/// Online (`on=true`) or offline (`on=false`) a CPU via `.../cpuN/online`.
///
/// `cpu0` has no `online` file and is always online — this returns
/// `NotFound` with a clear message in that case, matching the gating rule.
pub fn set_online(cpu: u32, on: bool) -> std::io::Result<()> {
    let path = PathBuf::from(format!("{SYSFS_SYSTEM_CPU}cpu{cpu}/online"));
    if !path.exists() {
        return Err(io_err(
            std::io::ErrorKind::NotFound,
            format!(
                "cpu{cpu} online file not found ({} missing — cpu0 is always online and cannot be offlined)",
                path.display()
            ),
        ));
    }
    let value = if on { "1" } else { "0" };
    write_with_ctx(&path, value, &format!("set_online(cpu={cpu}, on={on})"))
}
