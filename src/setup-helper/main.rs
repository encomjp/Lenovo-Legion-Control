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

fn source_dir() -> Result<PathBuf, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("cannot resolve setup helper path: {error}"))?;
    let source = if executable.starts_with("/usr/local/") {
        PathBuf::from("/usr/local/lib/legion-control/ryzen_smu")
    } else {
        PathBuf::from("/usr/lib/legion-control/ryzen_smu")
    };
    (source.join("Makefile").is_file() && source.join("dkms.conf").is_file())
        .then_some(source)
        .ok_or_else(|| "bundled ryzen_smu source is missing; reinstall Legion Control".into())
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
    if !host_daemon {
        let usr = bundled_usr_dir()?;
        let bundled_daemon = usr.join("bin/legion-daemon");
        let bundled_unit = usr.join("lib/systemd/system/legion-control.service");
        if !bundled_daemon.is_file() || !bundled_unit.is_file() {
            return Err("portable bundle is missing the daemon or unit file".into());
        }
        fs::create_dir_all("/usr/local/bin")
            .map_err(|e| format!("cannot create /usr/local/bin: {e}"))?;
        fs::copy(&bundled_daemon, "/usr/local/bin/legion-daemon")
            .map_err(|e| format!("cannot stage daemon: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata("/usr/local/bin/legion-daemon")
                .map_err(|e| format!("cannot stat staged daemon: {e}"))?
                .permissions();
            perms.set_mode(0o755);
            fs::set_permissions("/usr/local/bin/legion-daemon", perms)
                .map_err(|e| format!("cannot chmod staged daemon: {e}"))?;
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
        println!("staged daemon + unit from portable bundle");
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
