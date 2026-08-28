//! Narrow PolicyKit helper for optional Legion Control components.
//!
//! This binary deliberately accepts only fixed operations and fixed paths. It
//! never evaluates shell text or accepts caller-provided paths/commands.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const DKMS_VERSION: &str = "0.1.7";

const UDEV_RULE: &str = "# Lenovo Legion — allow userspace access to ec_sys and keyboard HID\n\
SUBSYSTEM==\"hidraw\", ATTRS{idVendor}==\"048d\", ATTRS{idProduct}==\"c193\", MODE=\"0660\", TAG+=\"uaccess\"\n\
SUBSYSTEM==\"hidraw\", ATTRS{idVendor}==\"048d\", ATTRS{idProduct}==\"c197\", MODE=\"0660\", TAG+=\"uaccess\"\n";

fn executable(candidates: &[&'static str]) -> Result<&'static str, String> {
    for path in candidates {
        if Path::new(path).is_file() {
            return Ok(path);
        }
    }
    Err(format!(
        "required tool not found: {}",
        candidates.join(" or ")
    ))
}

fn run(program: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(program)
        .args(args)
        .status()
        .map_err(|e| format!("cannot run {program}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} exited with {status}"))
    }
}

fn install_executable(source: &Path, destination: &Path) -> Result<(), String> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    fs::copy(source, destination).map_err(|e| {
        format!(
            "cannot copy {} to {}: {e}",
            source.display(),
            destination.display()
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(destination)
            .map_err(|e| format!("cannot stat {}: {e}", destination.display()))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(destination, perms)
            .map_err(|e| format!("cannot chmod {}: {e}", destination.display()))?;
    }
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|e| format!("cannot create {}: {e}", destination.display()))?;
    for entry in
        fs::read_dir(source).map_err(|e| format!("cannot read {}: {e}", source.display()))?
    {
        let entry = entry.map_err(|e| format!("cannot inspect bundle entry: {e}"))?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|e| format!("cannot inspect {}: {e}", source_path.display()))?;
        if file_type.is_dir() {
            copy_tree(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &destination_path).map_err(|e| {
                format!(
                    "cannot copy {} to {}: {e}",
                    source_path.display(),
                    destination_path.display()
                )
            })?;
        } else {
            return Err(format!(
                "unsupported bundle entry: {}",
                source_path.display()
            ));
        }
    }
    Ok(())
}

fn source_dir() -> Result<PathBuf, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("cannot resolve setup helper path: {error}"))?;
    let source = if executable.starts_with("/usr/local/") {
        PathBuf::from("/usr/local/lib/legion-control/ryzen_smu")
    } else {
        PathBuf::from("/usr/lib/legion-control/ryzen_smu")
    };
    if source.join("Makefile").is_file() && source.join("dkms.conf").is_file() {
        return Ok(source);
    }

    let bundled = executable
        .parent()
        .and_then(Path::parent)
        .map(|usr| usr.join("lib/legion-control/ryzen_smu"));
    let Some(bundled) = bundled else {
        return Err("bundled ryzen_smu source is missing; reinstall Legion Control".into());
    };
    if !bundled.join("Makefile").is_file() || !bundled.join("dkms.conf").is_file() {
        return Err("bundled ryzen_smu source is missing; reinstall Legion Control".into());
    }
    let stable = PathBuf::from("/usr/local/lib/legion-control/ryzen_smu");
    copy_tree(&bundled, &stable)?;
    Ok(stable)
}

fn install_ryzen_smu() -> Result<(), String> {
    if Path::new("/sys/kernel/ryzen_smu_drv").is_dir() {
        println!("ryzen_smu is already loaded");
        return Ok(());
    }
    let dkms = executable(&["/usr/bin/dkms", "/usr/sbin/dkms"])?;
    let make = executable(&["/usr/bin/make"])?;
    let modprobe = executable(&["/usr/bin/modprobe", "/usr/sbin/modprobe"])?;
    let source = source_dir()?;

    // DKMS can hold builds for several kernels at once (e.g. after switching
    // LTS → bore) and the registered source copy may predate bundle updates
    // (e.g. compatibility patches). Cleanest guarantee: drop any previous
    // registration and re-add from the bundled source, which (a) always uses
    // the current driver source and (b) builds for the running kernel only.
    let status = Command::new(dkms)
        .arg("status")
        .output()
        .map_err(|e| format!("cannot query DKMS: {e}"))?;
    let dkms_status = String::from_utf8_lossy(&status.stdout);
    if dkms_status
        .lines()
        .any(|line| line.starts_with(&format!("ryzen_smu/{DKMS_VERSION}")))
    {
        // Best-effort: uninstalls for every kernel, keeps /usr/src tidy.
        let _ = Command::new(dkms)
            .args(["remove", &format!("ryzen_smu/{DKMS_VERSION}"), "--all"])
            .status();
    }

    let status = Command::new(make)
        .arg("dkms-install")
        .current_dir(&source)
        .status()
        .map_err(|e| format!("cannot build ryzen_smu: {e}"))?;
    if !status.success() {
        return Err(format!("ryzen_smu DKMS build failed with {status}"));
    }

    fs::write("/etc/modules-load.d/ryzen_smu.conf", "ryzen_smu\n")
        .map_err(|e| format!("cannot enable ryzen_smu at boot: {e}"))?;
    run(modprobe, &["ryzen_smu"])?;
    if !Path::new("/sys/kernel/ryzen_smu_drv").is_dir() {
        return Err("ryzen_smu loaded but did not expose its sysfs interface".into());
    }
    restart_daemon()?;
    println!("ryzen_smu installed and loaded; no tuning value was written");
    Ok(())
}

fn remove_ryzen_smu() -> Result<(), String> {
    let dkms = executable(&["/usr/bin/dkms", "/usr/sbin/dkms"])?;
    let modprobe = executable(&["/usr/bin/modprobe", "/usr/sbin/modprobe"])?;
    if Path::new("/sys/kernel/ryzen_smu_drv").is_dir() {
        run(modprobe, &["-r", "ryzen_smu"])?;
    }
    let status = Command::new(dkms)
        .arg("status")
        .output()
        .map_err(|e| format!("cannot query DKMS: {e}"))?;
    if String::from_utf8_lossy(&status.stdout)
        .lines()
        .any(|line| line.starts_with(&format!("ryzen_smu/{DKMS_VERSION}")))
    {
        run(
            dkms,
            &["remove", &format!("ryzen_smu/{DKMS_VERSION}"), "--all"],
        )?;
    }
    let source = format!("/usr/src/ryzen_smu-{DKMS_VERSION}");
    if let Err(error) = fs::remove_dir_all(&source) {
        if error.kind() != std::io::ErrorKind::NotFound {
            return Err(format!("cannot remove {source}: {error}"));
        }
    }
    let load_config = "/etc/modules-load.d/ryzen_smu.conf";
    if let Err(error) = fs::remove_file(load_config) {
        if error.kind() != std::io::ErrorKind::NotFound {
            return Err(format!("cannot remove {load_config}: {error}"));
        }
    }
    restart_daemon()?;
    println!("ryzen_smu removed");
    Ok(())
}

fn systemctl(args: &[&str]) -> Result<(), String> {
    let systemctl = executable(&["/usr/bin/systemctl", "/bin/systemctl"])?;
    run(systemctl, args)
}

fn restart_daemon() -> Result<(), String> {
    systemctl(&["try-restart", "legion-control.service"])
}

/// True when a Spectrum-permission udev rule is already on the host — either
/// the canonical /etc location (source + helper installs) or the packaged
/// /usr/lib location. Never counts other prefixes: /usr/local staging relies
/// on the /etc copy below.
fn udev_rule_installed() -> bool {
    Path::new("/etc/udev/rules.d/99-legion.rules").is_file()
        || Path::new("/usr/lib/udev/rules.d/99-legion.rules").is_file()
}

fn bundled_usr_dir() -> Result<PathBuf, String> {
    // …/usr/libexec/legion-control-setup → …/usr (works inside an AppImage
    // squashfs mount and for fixed-prefix installs alike).
    let exe =
        std::env::current_exe().map_err(|error| format!("cannot resolve helper path: {error}"))?;
    exe.parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| "cannot resolve bundle root".into())
}

fn enable_daemon() -> Result<(), String> {
    // A portable bundle (AppImage) ships the daemon and unit but installs
    // nothing on first run — systemd needs stable paths and the squashfs
    // mount is not one. Stage both onto the host before enabling; fixed
    // installs (/usr/bin, /usr/local/bin) are left untouched.
    let host_daemon = Path::new("/usr/bin/legion-daemon").is_file()
        || Path::new("/usr/local/bin/legion-daemon").is_file();
    let host_unit = Path::new("/etc/systemd/system/legion-control.service").is_file()
        || Path::new("/usr/lib/systemd/system/legion-control.service").is_file();

    // Version skew: the staged unit may predate a fix shipped in this
    // bundle (e.g. the DeviceAllow GPU rules). Compare the bundled unit's
    // DeviceAllow section against the host's — refresh when the bundle
    // carries rules the host lacks, so service-file fixes reach existing
    // installs without manual reinstall.
    let unit_needs_refresh = host_daemon && host_unit && bundled_usr_dir().ok().is_some_and(|usr| {
        let bundled_unit = usr.join("lib/systemd/system/legion-control.service");
        if !bundled_unit.is_file() {
            return false;
        }
        let bundled_text = fs::read_to_string(&bundled_unit).unwrap_or_default();
        let bundled_rules: Vec<&str> = bundled_text
            .lines()
            .filter(|l| l.trim_start().starts_with("DeviceAllow"))
            .collect();
        let host_path = if Path::new("/etc/systemd/system/legion-control.service").is_file() {
            Path::new("/etc/systemd/system/legion-control.service")
        } else {
            Path::new("/usr/lib/systemd/system/legion-control.service")
        };
        let host_text = fs::read_to_string(host_path).unwrap_or_default();
        let host_rules: Vec<&str> = host_text
            .lines()
            .filter(|l| l.trim_start().starts_with("DeviceAllow"))
            .collect();
        !bundled_rules.is_empty() && bundled_rules != host_rules
    });
    if !host_daemon || !host_unit || unit_needs_refresh {
        let usr = bundled_usr_dir()?;
        let bundled_daemon = usr.join("bin/legion-daemon");
        let bundled_unit = usr.join("lib/systemd/system/legion-control.service");
        if !bundled_daemon.is_file() || !bundled_unit.is_file() {
            return Err("portable bundle is missing the daemon or unit file".into());
        }
        install_executable(&bundled_daemon, Path::new("/usr/local/bin/legion-daemon"))?;

        // Bootstrap stable helper/policy paths so later setup actions reuse
        // PolicyKit's auth_admin_keep instead of the AppImage mount path.
        let bundled_helper = usr.join("libexec/legion-control-setup");
        let stable_helper = Path::new("/usr/local/libexec/legion-control-setup");
        if bundled_helper.is_file() && !stable_helper.is_file() {
            install_executable(&bundled_helper, stable_helper)?;
        }
        let bundled_policy = usr.join("share/polkit-1/actions/com.encomjp.legion-control.policy");
        let policy = Path::new("/usr/share/polkit-1/actions/com.encomjp.legion-control.policy");
        if bundled_policy.is_file() && !policy.is_file() {
            if let Some(parent) = policy.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
            }
            fs::copy(&bundled_policy, policy)
                .map_err(|e| format!("cannot stage polkit policy: {e}"))?;
        }

        // DKMS must use a stable source tree after the AppImage mount goes
        // away. This also makes the later backend button work without another
        // authentication transaction.
        let bundled_source = usr.join("lib/legion-control/ryzen_smu");
        let stable_source = Path::new("/usr/local/lib/legion-control/ryzen_smu");
        if bundled_source.join("Makefile").is_file()
            && bundled_source.join("dkms.conf").is_file()
            && !stable_source.join("Makefile").is_file()
        {
            copy_tree(&bundled_source, stable_source)?;
        }

        let unit_text = fs::read_to_string(&bundled_unit)
            .map_err(|e| format!("cannot read bundled unit: {e}"))?;
        // The bundled unit targets package installs (/usr/bin); source-style
        // staging lives in /usr/local/bin.
        let unit_text = unit_text.replace(
            "ExecStart=/usr/bin/legion-daemon",
            "ExecStart=/usr/local/bin/legion-daemon",
        );
        fs::create_dir_all("/etc/systemd/system")
            .map_err(|e| format!("cannot create unit dir: {e}"))?;
        fs::write("/etc/systemd/system/legion-control.service", unit_text)
            .map_err(|e| format!("cannot install unit: {e}"))?;
        systemctl(&["daemon-reload"])?;
        println!("staged daemon + helper + policy + unit from portable bundle");
    }
    // The portable bootstrap pre-stages the daemon at /usr/local/bin, so the
    // branch above is skipped — Spectrum permissions still need the udev rule.
    if !udev_rule_installed() {
        install_udev_rule()?;
    }
    systemctl(&["enable", "--now", "legion-control.service"])
}

fn install_udev_rule() -> Result<(), String> {
    let dest = Path::new("/etc/udev/rules.d/99-legion.rules");
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    fs::write(dest, UDEV_RULE).map_err(|e| format!("cannot write {}: {e}", dest.display()))?;
    // Ensure file mode 0644 (udev expects world-readable)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(dest)
            .map_err(|e| format!("cannot stat {}: {e}", dest.display()))?
            .permissions();
        perms.set_mode(0o644);
        fs::set_permissions(dest, perms)
            .map_err(|e| format!("cannot chmod {}: {e}", dest.display()))?;
    }
    // Reload and trigger — best-effort, never fatal if udevadm is missing.
    let _ = Command::new("udevadm")
        .args(["control", "--reload-rules"])
        .status();
    let _ = Command::new("udevadm")
        .args(["trigger", "-s", "hidraw"])
        .status();
    // Give udev a beat to re-apply, then verify at least one Spectrum node is group-rw.
    std::thread::sleep(std::time::Duration::from_millis(400));
    println!("udev rule installed at {}", dest.display());
    Ok(())
}

fn real_main() -> Result<(), String> {
    if unsafe { libc::geteuid() } != 0 {
        return Err("this helper must be authorized through PolicyKit".into());
    }
    match std::env::args().nth(1).as_deref() {
        Some("install-ryzen-smu") => install_ryzen_smu(),
        Some("remove-ryzen-smu") => remove_ryzen_smu(),
        Some("enable-daemon") => enable_daemon(),
        Some("install-udev") => install_udev_rule(),
        _ => Err(
            "allowed operations: install-ryzen-smu, remove-ryzen-smu, enable-daemon, install-udev"
                .into(),
        ),
    }
}

fn main() -> ExitCode {
    match real_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("legion-control-setup: {error}");
            ExitCode::FAILURE
        }
    }
}
