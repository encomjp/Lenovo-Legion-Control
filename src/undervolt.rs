//! AMD Curve Optimizer access through the optional `ryzen_smu` kernel driver.
//!
//! The daemon is the only process that should call this module. Every write is
//! capability-gated by a successful read-only firmware probe, range-limited,
//! temporary (firmware resets it at reboot), and verified by reading every core.

use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

const DRIVER: &str = "/sys/kernel/ryzen_smu_drv";
const ARGS: &str = "/sys/kernel/ryzen_smu_drv/smu_args";
const CMD: &str = "/sys/kernel/ryzen_smu_drv/rsmu_cmd";
const GRANITE_RIDGE: u32 = 23;
const SET_ALL_CORE_CO: u32 = 0x07;
const GET_PER_CORE_CO: u32 = 0xD5;
const SMU_OK: u32 = 0x01;

/// Conservative first-release bounds. Negative values undervolt.
pub const MIN_OFFSET: i16 = -30;
pub const MAX_OFFSET: i16 = 0;
const PERSISTENCE_DIR: &str = "/var/lib/legion-control";
const PERSISTENCE_FILE: &str = "/var/lib/legion-control/curve-optimizer.json";
const ARMED_FILE: &str = "/var/lib/legion-control/curve-optimizer.armed";
const BOOT_BASELINE_FILE: &str = "/run/legion-control/curve-optimizer-baseline.json";
const HISTORY_FILE: &str = "/var/lib/legion-control/curve-optimizer-history.json";
const STARTUP_DELAY: Duration = Duration::from_secs(60);
const VALIDATION_WINDOW: Duration = Duration::from_secs(300);

fn offset_allowed(offset: i16) -> bool {
    (MIN_OFFSET..=MAX_OFFSET).contains(&offset)
}

static ACCESS: Mutex<()> = Mutex::new(());
static BASELINE: OnceLock<Vec<i16>> = OnceLock::new();

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CurveOptimizerStatus {
    pub available: bool,
    pub reason: String,
    pub codename: Option<u32>,
    pub driver_version: Option<String>,
    pub firmware_version: Option<String>,
    pub current: Vec<i16>,
    /// Values observed before Legion Control performs its first write.
    pub boot_baseline: Vec<i16>,
    /// Previous offset before the last successful Apply (restores with one click).
    #[serde(default)]
    pub previous: Option<i16>,
    pub minimum: i16,
    pub maximum: i16,
    pub temporary_only: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CurveOptimizerPersistence {
    pub enabled: bool,
    pub offset: i16,
    pub recovery_blocked: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistenceConfig {
    enabled: bool,
    offset: i16,
}

impl CurveOptimizerStatus {
    fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            available: false,
            reason: reason.into(),
            codename: None,
            driver_version: None,
            firmware_version: None,
            current: Vec::new(),
            boot_baseline: Vec::new(),
            previous: None,
            minimum: MIN_OFFSET,
            maximum: MAX_OFFSET,
            temporary_only: true,
        }
    }
}

fn read_trimmed(path: &str) -> Option<String> {
    fs::read_to_string(path).ok().map(|v| v.trim().to_string())
}

fn physical_core_count() -> usize {
    let mut cores = std::collections::BTreeSet::new();
    if let Ok(entries) = fs::read_dir("/sys/devices/system/cpu") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name
                .strip_prefix("cpu")
                .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            {
                continue;
            }
            let topology = entry.path().join("topology");
            let package =
                fs::read_to_string(topology.join("physical_package_id")).unwrap_or_default();
            let core = fs::read_to_string(topology.join("core_id")).unwrap_or_default();
            if !core.trim().is_empty() {
                cores.insert((package.trim().to_string(), core.trim().to_string()));
            }
        }
    }
    cores.len()
}

fn write_words(path: &str, words: &[u32; 6]) -> Result<(), String> {
    let mut bytes = [0u8; 24];
    for (index, value) in words.iter().enumerate() {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    OpenOptions::new()
        .write(true)
        .open(path)
        .and_then(|mut f| f.write_all(&bytes))
        .map_err(|e| format!("cannot write {path}: {e}"))
}

fn write_command(command: u32) -> Result<(), String> {
    OpenOptions::new()
        .write(true)
        .open(CMD)
        .and_then(|mut f| f.write_all(&command.to_le_bytes()))
        .map_err(|e| format!("cannot send SMU command 0x{command:02x}: {e}"))
}

fn read_command_status() -> Result<u32, String> {
    let mut bytes = [0u8; 4];
    OpenOptions::new()
        .read(true)
        .open(CMD)
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|e| format!("cannot read SMU response: {e}"))?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_result_words() -> Result<[u32; 6], String> {
    let mut bytes = [0u8; 24];
    OpenOptions::new()
        .read(true)
        .open(ARGS)
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|e| format!("cannot read SMU arguments: {e}"))?;
    let mut result = [0u32; 6];
    for (index, value) in result.iter_mut().enumerate() {
        let mut word = [0u8; 4];
        word.copy_from_slice(&bytes[index * 4..index * 4 + 4]);
        *value = u32::from_le_bytes(word);
    }
    Ok(result)
}

fn send(command: u32, arg0: u32) -> Result<[u32; 6], String> {
    write_words(ARGS, &[arg0, 0, 0, 0, 0, 0])?;
    write_command(command)?;
    let status = read_command_status()?;
    if status != SMU_OK {
        return Err(format!(
            "SMU rejected command 0x{command:02x} (status 0x{status:02x})"
        ));
    }
    read_result_words()
}

fn read_core(ccd: u32, core: u32) -> Result<i16, String> {
    let mask = (ccd << 28) | ((core % 8) << 20);
    let words = send(GET_PER_CORE_CO, mask)?;
    Ok((words[0] as u16) as i16)
}

fn read_all_16() -> Result<Vec<i16>, String> {
    let mut values = Vec::with_capacity(16);
    for ccd in 0..2 {
        for core in 0..8 {
            values.push(read_core(ccd, core)?);
        }
    }
    Ok(values)
}

fn read_history() -> Option<i16> {
    serde_json::from_str::<i16>(&fs::read_to_string(HISTORY_FILE).ok()?).ok()
}

fn write_history(offset: i16) {
    let _ = fs::create_dir_all(PERSISTENCE_DIR);
    let _ = fs::write(
        HISTORY_FILE,
        serde_json::to_string(&offset).unwrap_or_else(|_| offset.to_string()),
    );
}

fn boot_baseline(current: &[i16]) -> Vec<i16> {
    BASELINE
        .get_or_init(|| {
            if let Ok(text) = fs::read_to_string(BOOT_BASELINE_FILE) {
                if let Ok(values) = serde_json::from_str::<Vec<i16>>(&text) {
                    if values.len() == 16 {
                        return values;
                    }
                }
                log::warn!("ignoring invalid boot baseline in {BOOT_BASELINE_FILE}");
            }

            let captured = current.to_vec();
            if let Some(parent) = Path::new(BOOT_BASELINE_FILE).parent() {
                if let Err(error) = fs::create_dir_all(parent) {
                    log::warn!("cannot create Curve Optimizer runtime state: {error}");
                    return captured;
                }
            }
            match serde_json::to_vec(&captured)
                .map_err(|error| error.to_string())
                .and_then(|data| {
                    fs::write(BOOT_BASELINE_FILE, data).map_err(|error| error.to_string())
                }) {
                Ok(()) => log::info!("captured Curve Optimizer boot baseline"),
                Err(error) => log::warn!("cannot persist Curve Optimizer boot baseline: {error}"),
            }
            captured
        })
        .clone()
}

fn validate_driver() -> Result<(u32, String, String), String> {
    let cpuinfo = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    if cpuinfo.contains("GenuineIntel") || cpuinfo.contains("Intel(R)") {
        return Err("Curve Optimizer is an AMD feature (not applicable on Intel CPUs)".into());
    }
    if !Path::new(DRIVER).is_dir() {
        return Err(
            "ryzen_smu driver is not loaded — open About → Setup and press 'Install' for AMD ryzen_smu (needs dkms + kernel headers)".into(),
        );
    }
    let codename = read_trimmed(&format!("{DRIVER}/codename"))
        .ok_or("ryzen_smu did not expose a CPU codename")?
        .parse::<u32>()
        .map_err(|_| "invalid ryzen_smu codename")?;
    if codename != GRANITE_RIDGE {
        return Err(format!(
            "Curve Optimizer writes are not validated for ryzen_smu codename {codename}"
        ));
    }
    let product_name = read_trimmed("/sys/class/dmi/id/product_name").unwrap_or_default();
    let product_version = read_trimmed("/sys/class/dmi/id/product_version").unwrap_or_default();
    let cpuinfo = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    if product_name != "83RU"
        || !product_version.contains("Legion Pro 7 16AFR10H")
        || !cpuinfo.contains("AMD Ryzen 9 9955HX3D")
    {
        return Err(format!(
            "Curve Optimizer writes are currently validated only on Legion Pro 7 16AFR10H (83RU) with Ryzen 9 9955HX3D; detected {product_name} / {product_version}"
        ));
    }
    let cores = physical_core_count();
    if cores != 16 {
        return Err(format!("Granite Ridge support is currently restricted to validated 16-core layouts (found {cores})"));
    }
    let driver = read_trimmed(&format!("{DRIVER}/drv_version")).unwrap_or_else(|| "unknown".into());
    let firmware = read_trimmed(&format!("{DRIVER}/version")).unwrap_or_else(|| "unknown".into());
    Ok((codename, driver, firmware))
}

/// Read-only capability probe and current per-core values.
pub fn status() -> CurveOptimizerStatus {
    let _guard = match ACCESS.lock() {
        Ok(guard) => guard,
        Err(_) => {
            return CurveOptimizerStatus::unavailable("Curve Optimizer access lock is poisoned")
        }
    };
    let (codename, driver, firmware) = match validate_driver() {
        Ok(v) => v,
        Err(e) => return CurveOptimizerStatus::unavailable(e),
    };
    let current = match read_all_16() {
        Ok(values) => values,
        Err(e) => {
            return CurveOptimizerStatus::unavailable(format!(
                "read-only firmware probe failed: {e}"
            ))
        }
    };
    let baseline = boot_baseline(&current);
    let previous = read_history();
    CurveOptimizerStatus {
        available: true,
        reason: "Read-only firmware probe accepted".into(),
        codename: Some(codename),
        driver_version: Some(driver),
        firmware_version: Some(firmware),
        current,
        boot_baseline: baseline,
        previous,
        minimum: MIN_OFFSET,
        maximum: MAX_OFFSET,
        temporary_only: true,
    }
}

/// `Some(v)` iff every value in `values` equals `v` (empty slice → `None`).
fn uniform(values: &[i16]) -> Option<i16> {
    let first = *values.first()?;
    values.iter().all(|v| *v == first).then_some(first)
}

/// Apply one temporary all-core offset and verify all 16 cores by read-back.
pub fn set_all(offset: i16) -> Result<CurveOptimizerStatus, String> {
    if !offset_allowed(offset) {
        return Err(format!(
            "Curve Optimizer offset must be between {MIN_OFFSET} and {MAX_OFFSET}"
        ));
    }
    // The status call performs the mandatory read-only capability probe and captures baseline.
    let before = status();
    if !before.available {
        return Err(before.reason);
    }
    let previous_offset = uniform(&before.current).filter(|v| *v != offset);
    let _guard = ACCESS
        .lock()
        .map_err(|_| "Curve Optimizer access lock is poisoned")?;
    let encoded = u32::from(offset as u16);
    send(SET_ALL_CORE_CO, encoded)?;
    let readback = read_all_16()?;
    if uniform(&readback) != Some(offset) {
        return Err(format!("SMU read-back mismatch after apply: {readback:?}"));
    }
    drop(_guard);
    if let Some(prev) = previous_offset {
        write_history(prev);
    }
    if let Some(mut config) = load_persistence_config() {
        if config.enabled && config.offset != offset {
            config.enabled = false;
            if let Err(error) = write_persistence_config(&config) {
                log::warn!("could not disable stale startup undervolt: {error}");
            } else {
                log::info!("startup undervolt disabled because the active offset changed");
            }
        }
    }
    Ok(status())
}

/// Restore the exact per-core values captured before Legion Control's first write.
pub fn reset_to_baseline() -> Result<CurveOptimizerStatus, String> {
    let before = status();
    if !before.available {
        return Err(before.reason);
    }
    let baseline = before.boot_baseline;
    if baseline.len() != 16 {
        return Err("No complete boot baseline is available".into());
    }
    if uniform(&baseline).is_some() {
        return set_all(baseline[0]);
    }
    Err("Per-core baseline restore is not enabled; reboot safely restores firmware defaults".into())
}

fn load_persistence_config() -> Option<PersistenceConfig> {
    let text = fs::read_to_string(PERSISTENCE_FILE).ok()?;
    match serde_json::from_str(&text) {
        Ok(config) => Some(config),
        Err(error) => {
            log::warn!("cannot parse {PERSISTENCE_FILE}: {error}");
            None
        }
    }
}

fn write_persistence_config(config: &PersistenceConfig) -> Result<(), String> {
    fs::create_dir_all(PERSISTENCE_DIR)
        .map_err(|error| format!("cannot create {PERSISTENCE_DIR}: {error}"))?;
    let data = serde_json::to_vec_pretty(config)
        .map_err(|error| format!("cannot encode startup undervolt: {error}"))?;
    let temporary = PathBuf::from(format!("{PERSISTENCE_FILE}.tmp"));
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&temporary)
        .map_err(|error| format!("cannot write {}: {error}", temporary.display()))?;
    file.write_all(&data)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("cannot save startup undervolt: {error}"))?;
    fs::rename(&temporary, PERSISTENCE_FILE)
        .map_err(|error| format!("cannot activate startup undervolt: {error}"))
}

pub fn persistence_status() -> CurveOptimizerPersistence {
    let config = load_persistence_config().unwrap_or(PersistenceConfig {
        enabled: false,
        offset: 0,
    });
    CurveOptimizerPersistence {
        enabled: config.enabled,
        offset: config.offset,
        recovery_blocked: Path::new(ARMED_FILE).exists(),
    }
}

pub fn set_persistence(enabled: bool, offset: i16) -> Result<CurveOptimizerPersistence, String> {
    if !enabled {
        let _ = fs::remove_file(ARMED_FILE);
        write_persistence_config(&PersistenceConfig {
            enabled: false,
            offset,
        })?;
        return Ok(persistence_status());
    }
    if !offset_allowed(offset) {
        return Err(format!(
            "Startup offset must be between {MIN_OFFSET} and {MAX_OFFSET}"
        ));
    }
    if Path::new(ARMED_FILE).exists() {
        return Err(
            "Startup undervolt is recovery-blocked; disable it before enabling again".into(),
        );
    }
    let current = status();
    if !current.available {
        return Err(current.reason);
    }
    if uniform(&current.current) != Some(offset) {
        return Err("Apply and verify this offset before enabling it at startup".into());
    }
    write_persistence_config(&PersistenceConfig {
        enabled: true,
        offset,
    })?;
    Ok(persistence_status())
}

/// Reapply a validated offset after boot. An armed marker prevents crash loops:
/// if the machine or daemon exits before the validation window ends, the next
/// start disables persistence instead of applying the offset again.
pub fn start_persistence_worker() {
    let Some(mut config) = load_persistence_config() else {
        return;
    };
    if !config.enabled {
        return;
    }
    if Path::new(ARMED_FILE).exists() {
        config.enabled = false;
        if let Err(error) = write_persistence_config(&config) {
            log::error!("failed to disable recovery-blocked startup undervolt: {error}");
        }
        if let Err(error) = fs::remove_file(ARMED_FILE) {
            log::warn!("failed to clear recovered startup undervolt marker: {error}");
        }
        log::warn!("startup undervolt disabled after an unclean previous validation window");
        return;
    }

    std::thread::spawn(move || {
        std::thread::sleep(STARTUP_DELAY);
        if let Err(error) = fs::create_dir_all(PERSISTENCE_DIR) {
            log::error!("cannot prepare startup undervolt state: {error}");
            return;
        }
        if let Err(error) = fs::write(ARMED_FILE, format!("{}\n", config.offset)) {
            log::error!("cannot arm startup undervolt recovery: {error}");
            return;
        }
        match set_all(config.offset) {
            Ok(_) => log::info!(
                "startup Curve Optimizer offset {} applied; validating for {} seconds",
                config.offset,
                VALIDATION_WINDOW.as_secs()
            ),
            Err(error) => {
                log::error!("startup Curve Optimizer apply failed: {error}");
                let _ = reset_to_baseline();
                config.enabled = false;
                let _ = write_persistence_config(&config);
                let _ = fs::remove_file(ARMED_FILE);
                return;
            }
        }
        std::thread::sleep(VALIDATION_WINDOW);
        if let Err(error) = fs::remove_file(ARMED_FILE) {
            if error.kind() != std::io::ErrorKind::NotFound {
                log::warn!("cannot clear startup undervolt recovery marker: {error}");
            }
        } else {
            log::info!("startup Curve Optimizer validation window completed");
        }
    });
}

pub fn clear_persistence_armed_on_clean_shutdown() {
    if let Err(error) = fs::remove_file(ARMED_FILE) {
        if error.kind() != std::io::ErrorKind::NotFound {
            log::warn!("cannot clear startup undervolt recovery marker: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conservative_bounds_reject_overvolt_and_extreme_offsets() {
        assert!(offset_allowed(-30));
        assert!(offset_allowed(-4));
        assert!(offset_allowed(0));
        assert!(!offset_allowed(-31));
        assert!(!offset_allowed(1));
    }
}
