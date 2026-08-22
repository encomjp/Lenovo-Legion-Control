//! Legion Daemon — background service managing Lenovo Legion hardware.
//!
//! Listens on a Unix domain socket for commands from CLI/GUI.
//! Reads sensors, controls fans, manages keyboard backlight.

use legion_core::comms::{
    bincode_opts, bind_socket_path, cmd_is_write, cmd_kind, cmd_label,
    DaemonCommand, DaemonResponse, MAX_FRAME_BYTES,
};
use bincode::Options as _;
use legion_core::thermal::ThermalConfig;
use legion_core::{
    battery, config, cpu, device, fans, keyboard, logging, profile, rgb_panic, sensors, thermal,
    undervolt,
};

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

/// Maximum concurrently-handled client connections. Prevents thread-bombing
/// the daemon via connection spam.
const MAX_CLIENTS: usize = 32;

/// Shared thermal config updated on SetThermal and read by the governor.
static THERMAL_CONFIG: OnceLock<Arc<RwLock<ThermalConfig>>> = OnceLock::new();
/// Condvar pair used to wake the governor when SetThermal changes the config.
static THERMAL_NOTIFY: OnceLock<Arc<(Mutex<bool>, Condvar)>> = OnceLock::new();

/// Snapshot of last-seen sensor values for throttled logging.
#[derive(Debug, Clone, Default, PartialEq)]
struct SensorSnapshot {
    cpu_tctl: f64,
    dgpu_temp: f64,
    fan1_rpm: u32,
    fan2_rpm: u32,
    fan4_rpm: u32,
    profile: String,
}

/// Mutable state shared between the accept loop and client-handler threads.
#[derive(Default)]
struct ClientState {
    last_sensors: Option<Instant>,
    snapshot: SensorSnapshot,
    timings: HashMap<&'static str, (u64, u64)>, // kind → (count, total_ms)
}

/// Peer credentials for a connected Unix stream (SO_PEERCRED).
fn peer_uid(stream: &UnixStream) -> Option<u32> {
    use std::os::fd::AsRawFd;
    // SAFETY: getsockopt with SO_PEERCRED fills a fixed-size ucred struct;
    // fd is valid (owned by stream) and len matches the struct size.
    unsafe {
        let mut cred = libc::ucred {
            pid: 0,
            uid: 0,
            gid: 0,
        };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        if libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut libc::ucred as *mut libc::c_void,
            &mut len,
        ) == 0
        {
            Some(cred.uid)
        } else {
            None
        }
    }
}

/// GID of the `legion` system group, if it exists. Socket access is gated by
/// group membership (0660 root:legion); root always passes kernel checks.
fn legion_group_gid() -> Option<u32> {
    let name = std::ffi::CString::new("legion").ok()?;
    // SAFETY: getgrnam returns a pointer into static storage; we only read
    // gr_gid before any other libc call.
    unsafe {
        let gr = libc::getgrnam(name.as_ptr());
        if gr.is_null() {
            None
        } else {
            Some((*gr).gr_gid)
        }
    }
}

/// Try to exclusively lock `path` (flock). Returns the file (keeps the lock)
/// or None when another process holds it.
fn acquire_singleton_lock(path: &std::path::Path) -> Option<std::fs::File> {
    use std::os::fd::AsRawFd;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .map_err(|e| format!("cannot open {}: {e}", path.display()))
        .ok()?;
    // SAFETY: flock on a valid fd — plain POSIX call, no memory concerns.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc != 0 {
        return None;
    }
    Some(file)
}

fn main() {
    logging::init("legion-daemon");
    // SAFETY: geteuid() is a pure POSIX syscall with no memory safety requirements.
    let euid = unsafe { libc::geteuid() };
    let pid = std::process::id();
    log::info!("starting (euid={euid}, pid={pid})");

    // signal_hook::flag sets the AtomicBool to *true* when the signal arrives.
    let shutdown = Arc::new(AtomicBool::new(false));
    for (sig, flag) in [
        (signal_hook::consts::SIGINT, shutdown.clone()),
        (signal_hook::consts::SIGTERM, shutdown.clone()),
    ] {
        if let Err(e) = signal_hook::flag::register(sig, flag) {
            // Without the handler the signal would kill us mid-write; refuse to run.
            log::error!("cannot register signal handler {sig}: {e}");
            std::process::exit(1);
        }
    }
    // SIGHUP reloads log config — use a separate flag so it doesn't shut down.
    let reload = Arc::new(AtomicBool::new(false));
    if let Err(e) = signal_hook::flag::register(signal_hook::consts::SIGHUP, reload.clone()) {
        log::warn!("SIGHUP reload unavailable: {e}");
    }

    let path = match bind_socket_path() {
        Ok(p) => p,
        Err(e) => {
            log::error!("cannot determine socket path: {e}");
            std::process::exit(1);
        }
    };

    // Single-instance guard: flock a pidfile next to the socket. Never
    // blindly delete a live socket — two daemons would fight over CPU freq.
    let pidfile_path = path.with_extension("pid");
    let _singleton = match acquire_singleton_lock(&pidfile_path) {
        Some(f) => f,
        None => {
            log::error!(
                "another legion-daemon appears to be running (lock held on {}) — exiting",
                pidfile_path.display()
            );
            // Non-zero exit so systemd Restart=on-failure retries once the
            // previous instance has released the singleton lock.
            std::process::exit(1);
        }
    };
    if path.exists() {
        log::debug!("removing stale socket {}", path.display());
        std::fs::remove_file(&path).ok();
    }

    if euid != 0 {
        log::warn!(
            "Running as non-root: platform profile, fans, and conservation writes will fail. \
             Install the system service: sudo systemctl enable --now legion-control"
        );
    }

    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            log::error!("Cannot bind {}: {}", path.display(), e);
            std::process::exit(1);
        }
    };

    // Socket permissions: root daemon → 0660 owned by group `legion` so only
    // root and legion-group members can issue privileged commands. Never
    // world-writable. Non-root daemon → umask default in XDG_RUNTIME_DIR.
    #[cfg(unix)]
    if euid == 0 {
        use std::os::unix::fs::PermissionsExt;
        if let Some(gid) = legion_group_gid() {
            // SAFETY: chown with a valid gid and NUL-terminated path — plain syscall.
            let cpath = match std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) {
                Ok(p) => p,
                Err(_) => std::ffi::CString::new("/run/legion-control.socket").unwrap(),
            };
            unsafe {
                libc::chown(cpath.as_ptr(), u32::MAX, gid);
            }
        }
        let mode = if legion_group_gid().is_some() {
            0o660
        } else {
            log::warn!(
                "group 'legion' does not exist — restricting socket to root only. \
                 Create it with: sudo groupadd -r legion && sudo usermod -aG legion $USER"
            );
            0o600
        };
        if let Err(e) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)) {
            log::warn!("failed to chmod socket {}: {e}", path.display());
        }
    }

    if let Err(e) = listener.set_nonblocking(true) {
        // Without nonblocking accept the shutdown flag is never checked — fatal.
        log::error!("Cannot set nonblocking accept: {e}");
        std::process::exit(1);
    }

    log::info!("Listening on {}", path.display());

    // ── hardware fingerprint ──
    let info = device::detect();
    log::info!(
        "machine: {} ({}) | BIOS {} | EC {} | CPU {} | GPU {} | gen={} | backend={} | fans={}",
        info.model,
        info.machine_type,
        info.bios_version,
        info.ec_chip,
        info.cpu_model,
        info.gpu_model,
        info.gen,
        info.capabilities.fan_backend,
        info.capabilities.fans.len()
    );
    log::info!(
        "hardware: profile={} ppt={} gpu_ppt={} fans_ok={} lighting={}",
        profile::current(),
        profile::ppt_available(),
        !profile::gpu_ppt_limits().is_empty(),
        fans::read_rpm(1).is_some(),
        info.capabilities.lighting,
    );
    let hwmon_loaded = std::path::Path::new("/sys/class/hwmon")
        .read_dir()
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| {
            std::fs::read_to_string(e.path().join("name"))
                .unwrap_or_default()
                .trim()
                == "legion_hwmon"
        });
    log::info!("legion_hwmon loaded={hwmon_loaded}");
    undervolt::start_persistence_worker();

    // ── thermal governor shared state ──
    // Validate the on-disk config before the governor uses it: a hand-edited
    // or corrupt settings.json must not drive raw frequency writes.
    let mut seeded_thermal = config::get().thermal.clone();
    if let Err(e) = thermal::validate(seeded_thermal.max_temp, true) {
        log::warn!(
            "invalid thermal config from disk ({}: {e}) — using defaults",
            seeded_thermal.max_temp
        );
        seeded_thermal = ThermalConfig::default();
    }
    let thermal_cfg = Arc::new(RwLock::new(seeded_thermal));
    THERMAL_CONFIG.set(thermal_cfg.clone()).ok();
    let thermal_notify: Arc<(Mutex<bool>, Condvar)> = Arc::new((Mutex::new(false), Condvar::new()));
    THERMAL_NOTIFY.set(thermal_notify.clone()).ok();

    // Watch Spectrum HID + kernel USB faults; soft/USB auto-fix when dark.
    let shutdown_w = shutdown.clone();
    if let Err(e) = std::thread::Builder::new()
        .name("rgb-watchdog".into())
        .spawn(move || rgb_watchdog(shutdown_w))
    {
        log::error!("failed to start rgb-watchdog thread: {e}");
    } else {
        log::info!("rgb-watchdog thread started");
    }

    // Thermal governor thread (alongside rgb-watchdog)
    let shutdown_t = shutdown.clone();
    let thermal_cfg_t = thermal_cfg.clone();
    let thermal_notify_t = thermal_notify.clone();
    if let Err(e) = std::thread::Builder::new()
        .name("thermal-governor".into())
        .spawn(move || thermal_governor(shutdown_t, thermal_cfg_t, thermal_notify_t))
    {
        log::error!("failed to start thermal-governor thread: {e}");
    } else {
        log::info!("thermal-governor thread started");
    }

    let client_state = Arc::new(Mutex::new(ClientState::default()));
    let active_clients = Arc::new(AtomicUsize::new(0));

    while !shutdown.load(Ordering::Relaxed) {
        // Check reload flag (set by SIGHUP) without blocking accept.
        if reload.swap(false, Ordering::Relaxed) {
            log::info!("SIGHUP received — reloading log filter");
            logging::reload_from_env();
        }
        match listener.accept() {
            Ok((stream, _)) => {
                // One thread per connection with a read timeout: a stalled
                // client must never wedge the accept loop or clean shutdown.
                if active_clients.load(Ordering::Relaxed) >= MAX_CLIENTS {
                    log::warn!("connection limit reached — dropping client");
                    continue;
                }
                let state = client_state.clone();
                let active = active_clients.clone();
                active.fetch_add(1, Ordering::Relaxed);
                let active_inner = active_clients.clone();
                if let Err(e) = std::thread::Builder::new()
                    .name("client-handler".into())
                    .spawn(move || {
                        handle_client(stream, &state);
                        active_inner.fetch_sub(1, Ordering::Relaxed);
                    })
                {
                    log::error!("failed to spawn client handler: {e}");
                    active.fetch_sub(1, Ordering::Relaxed);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => log::error!("Accept error: {e}"),
        }
    }

    // Give the governor a moment to notice the shutdown flag before we
    // restore hardware state (it ticks every ~100 ms).
    std::thread::sleep(Duration::from_millis(300));
    restore_hardware_on_shutdown();

    undervolt::clear_persistence_armed_on_clean_shutdown();
    std::fs::remove_file(&path).ok();
    log::info!("Daemon stopped");
}

/// Best-effort hardware cleanup on daemon exit: unthrottle CPUs and return
/// fans to auto so `systemctl stop` never leaves the machine capped.
fn restore_hardware_on_shutdown() {
    match thermal::write_all_cpus(thermal::MAX_FULL) {
        Ok(()) => log::info!("shutdown: restored scaling_max_freq to full speed"),
        Err(e) => log::warn!("shutdown: could not restore scaling_max_freq: {e}"),
    }
    if let Err(e) = fans::set_auto() {
        log::debug!("shutdown: fans back to auto failed: {e}");
    }
}

fn handle_client(mut stream: UnixStream, state: &Arc<Mutex<ClientState>>) {
    const CLIENT_READ_TIMEOUT: Duration = Duration::from_secs(5);
    if let Err(e) = stream.set_read_timeout(Some(CLIENT_READ_TIMEOUT)) {
        log::debug!("client set_read_timeout failed: {e}");
        return;
    }

    // Audit who connected. Kernel socket permissions (0660 root:legion)
    // already gate access; this makes abuse visible in the journal.
    if let Some(uid) = peer_uid(&stream) {
        log::debug!("client connected (uid={uid})");
    }

    let mut buf = Vec::new();
    if let Err(e) = std::io::Read::by_ref(&mut stream)
        .take(MAX_FRAME_BYTES)
        .read_to_end(&mut buf)
    {
        log::debug!("client read failed: {e}");
        return;
    }

    let cmd: DaemonCommand = match bincode_opts().deserialize(&buf) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("client sent unparsable command ({e})");
            let _ = send_response(&mut stream, DaemonResponse::Error(format!("Parse: {}", e)));
            return;
        }
    };

    let kind = cmd_kind(&cmd);
    let is_write = cmd_is_write(&cmd);
    let t0 = Instant::now();
    let response = {
        let mut st = match state.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        // Destructure through the guard: two &mut field borrows via deref_mut
        // would otherwise count as overlapping borrows of the guard itself.
        let ClientState {
            last_sensors,
            snapshot,
            ..
        } = &mut *st;
        process_command(cmd, last_sensors, snapshot)
    };
    let elapsed = t0.elapsed().as_millis() as u64;

    {
        let mut st = match state.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let (count, total) = st.timings.entry(kind).or_insert((0, 0));
        *count += 1;
        *total += elapsed;
        if elapsed > 100 && is_write {
            let avg = (*total).checked_div(*count).unwrap_or(0);
            log::warn!("cmd {kind} slow: {elapsed} ms (avg {avg} ms over {count} calls)");
        }
    }

    if let Err(e) = send_response(&mut stream, response) {
        log::debug!("client response write failed: {e}");
    }
}

fn send_response(stream: &mut UnixStream, resp: DaemonResponse) -> std::io::Result<()> {
    let data = bincode_opts()
        .serialize(&resp)
        .map_err(|e| std::io::Error::other(format!("Serialize: {}", e)))?;
    stream.write_all(&data)
}

fn build_thermal_status() -> thermal::ThermalStatus {
    let cfg = config::get().thermal;
    let (tctl, tccd2) = thermal::read_thermal_temps();
    let cur_max = thermal::read_cur_max().unwrap_or(0);
    let restore_temp = cfg.max_temp.saturating_sub(thermal::HYSTERESIS as u8);
    let active = cfg.enabled && cur_max != 0 && cur_max < thermal::MAX_FULL;
    thermal::ThermalStatus {
        config: cfg,
        cur_max_freq: cur_max,
        tctl_mc: tctl,
        tccd2_mc: tccd2,
        active,
        restore_temp,
    }
}

fn thermal_governor(
    shutdown: Arc<AtomicBool>,
    cfg: Arc<RwLock<ThermalConfig>>,
    notify: Arc<(Mutex<bool>, Condvar)>,
) {
    let mut warned_missing = false;
    // Sensor-spike smoothing carried across ticks (α=½ EMA, urgent bypass).
    let mut temp_filter = thermal::TempFilter::default();
    while !shutdown.load(Ordering::Relaxed) {
        let (tctl, tccd2) = thermal::read_thermal_temps();
        let cur_max_opt = thermal::read_cur_max();
        // Poison recovery: a panicked writer must not kill the governor and
        // freeze CPUs at their last throttled frequency.
        let (enabled, _max_temp) = {
            let g = cfg
                .read()
                .unwrap_or_else(|p| p.into_inner());
            (g.enabled, g.max_temp)
        };

        if tctl.is_none() && tccd2.is_none() && !warned_missing {
            log::warn!(
                "thermal governor: k10temp not found — status will show None temps, no freq writes"
            );
            warned_missing = true;
        } else if tctl.is_some() || tccd2.is_some() {
            warned_missing = false;
        }

        if !enabled {
            // Idle when disabled: wait up to 10s or until SetThermal notifies.
            let (lock, cvar) = &*notify;
            let guard = match lock.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            let (mut guard, _timeout) =
                match cvar.wait_timeout(guard, Duration::from_secs(10)) {
                    Ok(r) => r,
                    Err(p) => p.into_inner(),
                };
            // reset flag
            *guard = false;
            drop(guard);
            if shutdown.load(Ordering::Relaxed) {
                break;
            }
            continue;
        }

        // Enabled but no temps: cannot compute, sleep 1s
        if tctl.is_none() && tccd2.is_none() {
            // respect shutdown with short sleeps
            for _ in 0..10 {
                if shutdown.load(Ordering::Relaxed) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            if shutdown.load(Ordering::Relaxed) {
                break;
            }
            continue;
        }

        let temp_mc = match (tctl, tccd2) {
            (Some(a), Some(b)) => a.max(b),
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => unreachable!(),
        };

        let cur = match cur_max_opt {
            Some(v) => v,
            None => {
                log::warn!("thermal governor: cannot read scaling_max_freq — no throttle step");
                std::thread::sleep(thermal::INTERVAL);
                if shutdown.load(Ordering::Relaxed) {
                    break;
                }
                continue;
            }
        };

        let cfg_snapshot = cfg.read().unwrap_or_else(|p| p.into_inner()).clone();
        let limit_mc = cfg_snapshot.max_temp as i32 * 1000;
        let smooth_mc = temp_filter.effective(temp_mc, limit_mc);
        if let Some(target) = thermal::compute_target(cur, smooth_mc, &cfg_snapshot) {
            match thermal::write_all_cpus(target) {
                Ok(()) => {
                    log::info!(
                        "thermal governor: {}°C smoothed (raw {}°C, max {}°C, restore {}°C) cur {} → {} kHz",
                        smooth_mc as f64 / 1000.0,
                        temp_mc as f64 / 1000.0,
                        cfg_snapshot.max_temp,
                        cfg_snapshot
                            .max_temp
                            .saturating_sub(thermal::HYSTERESIS as u8),
                        cur,
                        target
                    );
                }
                Err(e) => {
                    log::warn!("thermal governor: write_all_cpus({}) failed: {e}", target);
                }
            }
        } else {
            log::trace!(
                "thermal governor: hold temp {:.1}°C (raw {:.1}°C) cur {} kHz (max {}°C)",
                smooth_mc as f64 / 1000.0,
                temp_mc as f64 / 1000.0,
                cur,
                cfg_snapshot.max_temp
            );
        }

        // Sleep 1s when enabled, respecting shutdown
        // Use segmented sleep so shutdown is noticed within ~100ms
        for _ in 0..10 {
            if shutdown.load(Ordering::Relaxed) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if shutdown.load(Ordering::Relaxed) {
            break;
        }
    }
    log::info!("thermal-governor thread stopped");
}

fn process_command(
    cmd: DaemonCommand,
    last_sensors: &mut Option<Instant>,
    snapshot: &mut SensorSnapshot,
) -> DaemonResponse {
    let label = cmd_label(&cmd);
    let write = cmd_is_write(&cmd);
    if write {
        log::info!("cmd {label}");
    } else {
        log::debug!("cmd {label}");
    }

    let t0 = Instant::now();
    let response = match cmd {
        DaemonCommand::GetSensors => {
            let s = sensors::read_all();
            let now = Instant::now();
            let due = match *last_sensors {
                Some(t) => now >= t + Duration::from_secs(10),
                None => true,
            };
            let changed = snapshot.cpu_tctl != s.cpu_tctl
                || snapshot.dgpu_temp != s.dgpu_temp
                || snapshot.fan1_rpm != s.fan1_rpm
                || snapshot.fan2_rpm != s.fan2_rpm
                || snapshot.fan4_rpm != s.fan4_rpm
                || snapshot.profile != s.profile;
            if due || changed {
                *last_sensors = Some(now);
                *snapshot = SensorSnapshot {
                    cpu_tctl: s.cpu_tctl,
                    dgpu_temp: s.dgpu_temp,
                    fan1_rpm: s.fan1_rpm,
                    fan2_rpm: s.fan2_rpm,
                    fan4_rpm: s.fan4_rpm,
                    profile: s.profile.clone(),
                };
                log::info!(
                    "sensors: CPU={:.1}°C dGPU={:.1}°C fans=[{}, {}, {}] profile={}",
                    s.cpu_tctl,
                    s.dgpu_temp,
                    s.fan1_rpm,
                    s.fan2_rpm,
                    s.fan4_rpm,
                    s.profile
                );
            } else {
                log::trace!("sensors unchanged — throttled");
            }
            DaemonResponse::Sensors(s)
        }
        DaemonCommand::GetProfile => DaemonResponse::Profile(profile::current()),
        DaemonCommand::SetProfile(name) => match profile::set(&name) {
            Ok(()) => DaemonResponse::Ok,
            Err(e) => DaemonResponse::Error(e),
        },
        DaemonCommand::GetFanRpm(fan) => match fans::read_rpm(fan) {
            Some(rpm) => DaemonResponse::FanRpm(rpm),
            None => DaemonResponse::Error(format!("Cannot read fan {}", fan)),
        },
        DaemonCommand::SetFanTarget(fan, rpm) => match fans::set_target(fan, rpm) {
            Ok(()) => DaemonResponse::Ok,
            Err(e) => DaemonResponse::Error(format!("Cannot set fan {}: {}", fan, e)),
        },
        DaemonCommand::GetKbdBrightness => {
            // Try standard LED backlight first, fall back to Spectrum RGB brightness.
            match keyboard::brightness() {
                Some(b) => DaemonResponse::KbdBrightness(b),
                None => match keyboard::rgb_brightness() {
                    Some(b) => DaemonResponse::KbdBrightness(b),
                    None => DaemonResponse::Error("Cannot read keyboard brightness".into()),
                },
            }
        }
        DaemonCommand::SetKbdBrightness(level) => {
            // Try standard LED backlight first, fall back to Spectrum RGB brightness.
            match keyboard::set_brightness(level) {
                Ok(()) => DaemonResponse::Ok,
                Err(_) => match keyboard::set_rgb_brightness(level) {
                    Ok(()) => DaemonResponse::Ok,
                    Err(e) => DaemonResponse::Error(format!("Cannot set brightness: {e}")),
                },
            }
        }
        DaemonCommand::SetRgbStatic(r, g, b) => match keyboard::set_rgb_static(r, g, b) {
            Ok(()) => DaemonResponse::Ok,
            Err(e) => DaemonResponse::Error(e),
        },
        DaemonCommand::SetRgbEffect {
            effect,
            r,
            g,
            b,
            speed,
        } => {
            if effect.eq_ignore_ascii_case("off") {
                match keyboard::set_rgb_off() {
                    Ok(()) => DaemonResponse::Ok,
                    Err(e) => DaemonResponse::Error(e),
                }
            } else {
                match keyboard::RgbEffect::from_name(&effect) {
                    Some(fx) => match keyboard::set_rgb_effect(fx, r, g, b, speed) {
                        Ok(()) => DaemonResponse::Ok,
                        Err(e) => DaemonResponse::Error(e),
                    },
                    None => DaemonResponse::Error(format!(
                        "Unknown effect '{effect}'. Try: {}",
                        keyboard::RgbEffect::all_names().join(", ")
                    )),
                }
            }
        }
        DaemonCommand::SetRgbBrightness(level) => match keyboard::set_rgb_brightness(level) {
            Ok(()) => DaemonResponse::Ok,
            Err(e) => DaemonResponse::Error(e),
        },
        DaemonCommand::GetRgbBrightness => match keyboard::rgb_brightness() {
            Some(b) => DaemonResponse::RgbBrightness(b),
            None => DaemonResponse::Error("Cannot read Spectrum brightness".into()),
        },
        DaemonCommand::SetLogo(on) => match keyboard::set_logo(on) {
            Ok(()) => DaemonResponse::Ok,
            Err(e) => DaemonResponse::Error(e),
        },
        DaemonCommand::GetBattery => DaemonResponse::Battery {
            capacity: battery::capacity().unwrap_or(0),
            status: battery::status().unwrap_or_default(),
            voltage: battery::voltage().unwrap_or(0.0),
            cycles: battery::cycles().unwrap_or(0),
            conservation: battery::charge_limit_pct() < 100,
        },
        DaemonCommand::SetConservation(on) => match battery::set_conservation(on) {
            Ok(()) => DaemonResponse::Ok,
            Err(e) => DaemonResponse::Error(format!("Cannot set conservation: {e}")),
        },
        DaemonCommand::SetChargeLimit(pct) => match battery::set_charge_limit_pct(pct) {
            Ok(()) => DaemonResponse::Ok,
            Err(e) => DaemonResponse::Error(format!("Cannot set charge limit: {e}")),
        },
        DaemonCommand::GetChargeLimit => DaemonResponse::ChargeLimit(battery::charge_limit_pct()),
        DaemonCommand::GetCpuPower => DaemonResponse::CpuPower(sensors::sample_cpu_power_w()),
        DaemonCommand::GetDeviceInfo => DaemonResponse::DeviceInfo(device::detect()),
        DaemonCommand::GetCameraPower => match keyboard::camera_power() {
            Some(on) => DaemonResponse::CameraPower(on),
            None => DaemonResponse::Error("Camera power not found".into()),
        },
        DaemonCommand::SetFwAttr { name, value } => {
            if name.starts_with("ppt_") || name.starts_with("gpu_nv_") {
                match value.trim().parse::<u32>() {
                    Ok(v) => match profile::set_ppt(&name, v) {
                        Ok(()) => DaemonResponse::Ok,
                        Err(e) => DaemonResponse::Error(e),
                    },
                    Err(_) => DaemonResponse::Error(format!("Invalid PPT value '{value}'")),
                }
            } else {
                DaemonResponse::Error(format!("Unsupported firmware attribute '{name}'"))
            }
        }
        DaemonCommand::GetSmt => {
            if !cpu::smt_available() {
                DaemonResponse::Error("SMT control not available".into())
            } else {
                DaemonResponse::Smt {
                    active: cpu::smt_active().unwrap_or(false),
                    control: cpu::smt_control().unwrap_or_else(|| "unknown".into()),
                    logical_cpus: cpu::logical_cpus() as u32,
                }
            }
        }
        DaemonCommand::SetSmt(on) => match cpu::set_smt(on) {
            Ok(()) => DaemonResponse::Ok,
            Err(e) => DaemonResponse::Error(e),
        },
        DaemonCommand::GetBoost => match cpu::boost_enabled() {
            Some(on) => DaemonResponse::Boost(on),
            None => DaemonResponse::Error("CPU boost not available".into()),
        },
        DaemonCommand::SetBoost(on) => match cpu::set_boost(on) {
            Ok(()) => DaemonResponse::Ok,
            Err(e) => DaemonResponse::Error(e),
        },
        DaemonCommand::DiagnoseRgb => {
            let d = rgb_panic::diagnose();
            DaemonResponse::RgbDiagnosis {
                health: rgb_health_str(d.health).into(),
                summary: d.summary,
                details: d.details,
                fixable: d.fixable,
            }
        }
        DaemonCommand::FixRgbPanic => {
            let report = rgb_panic::troubleshoot();
            DaemonResponse::RgbFixReport {
                steps: report.steps,
                errors: report.errors,
                health: rgb_health_str(report.after.health).into(),
                summary: report.after.summary,
            }
        }
        DaemonCommand::GetRecentLogs(n) => {
            let text = logging::recent_logs_text(n);
            DaemonResponse::RecentLogs(text)
        }
        DaemonCommand::SetLogLevel(level) => {
            let lvl = match level.to_ascii_lowercase().as_str() {
                "off" => log::LevelFilter::Off,
                "error" => log::LevelFilter::Error,
                "warn" => log::LevelFilter::Warn,
                "info" => log::LevelFilter::Info,
                "debug" => log::LevelFilter::Debug,
                "trace" => log::LevelFilter::Trace,
                _ => return DaemonResponse::Error(format!("Unknown log level '{level}'")),
            };
            logging::set_max_level(lvl);
            DaemonResponse::Ok
        }
        DaemonCommand::GetCurveOptimizer => DaemonResponse::CurveOptimizer(undervolt::status()),
        DaemonCommand::SetCurveOptimizer {
            offset,
            acknowledge,
        } => {
            if !acknowledge {
                DaemonResponse::Error(
                    "Curve Optimizer changes require explicit instability-risk acknowledgement"
                        .into(),
                )
            } else {
                match undervolt::set_all(offset) {
                    Ok(status) => DaemonResponse::CurveOptimizer(status),
                    Err(error) => DaemonResponse::Error(error),
                }
            }
        }
        DaemonCommand::ResetCurveOptimizer => DaemonResponse::Error(
            "Curve Optimizer reset requires an explicit instability-risk acknowledgement".into(),
        ),
        DaemonCommand::ResetCurveOptimizerAcknowledged { acknowledge } => {
            if !acknowledge {
                DaemonResponse::Error(
                    "Curve Optimizer reset requires an explicit instability-risk acknowledgement"
                        .into(),
                )
            } else {
                match undervolt::reset_to_baseline() {
                    Ok(status) => DaemonResponse::CurveOptimizer(status),
                    Err(error) => DaemonResponse::Error(error),
                }
            }
        }
        DaemonCommand::GetCurveOptimizerPersistence => {
            DaemonResponse::CurveOptimizerPersistence(undervolt::persistence_status())
        }
        DaemonCommand::SetCurveOptimizerPersistence {
            enabled,
            offset,
            acknowledge,
        } => {
            if !acknowledge {
                DaemonResponse::Error(
                    "Startup undervolt requires explicit instability-risk acknowledgement".into(),
                )
            } else {
                match undervolt::set_persistence(enabled, offset) {
                    Ok(status) => DaemonResponse::CurveOptimizerPersistence(status),
                    Err(error) => DaemonResponse::Error(error),
                }
            }
        }
        DaemonCommand::GetThermal => DaemonResponse::Thermal(config::get().thermal),
        DaemonCommand::GetThermalStatus => DaemonResponse::ThermalStatus(build_thermal_status()),
        DaemonCommand::SetThermal {
            enabled,
            max_temp,
            acknowledge,
        } => {
            if let Err(e) = thermal::validate(max_temp, acknowledge) {
                return DaemonResponse::Error(e);
            }
            let was_enabled = config::get().thermal.enabled;
            config::update(|c| c.thermal = ThermalConfig { enabled, max_temp });
            if let Some(shared) = THERMAL_CONFIG.get() {
                if let Ok(mut g) = shared.write() {
                    *g = ThermalConfig { enabled, max_temp };
                }
            }
            // Disabling a mid-throttle governor must not leave CPUs capped:
            // restore full speed immediately.
            if was_enabled && !enabled {
                match thermal::write_all_cpus(thermal::MAX_FULL) {
                    Ok(()) => log::info!("thermal governor disabled — restored full speed"),
                    Err(e) => log::warn!("thermal governor disable: restore failed: {e}"),
                }
            }
            if let Some(notify) = THERMAL_NOTIFY.get() {
                let (lock, cvar) = &**notify;
                if let Ok(mut flag) = lock.lock() {
                    *flag = true;
                    cvar.notify_one();
                }
            }
            if enabled && !was_enabled {
                match std::process::Command::new("systemctl")
                    .args(["disable", "--now", "cpu95-throttle.service"])
                    .status()
                {
                    Ok(st) if st.success() => {
                        log::info!("disabled legacy cpu95-throttle.service");
                    }
                    Ok(st) => {
                        log::warn!(
                            "systemctl disable --now cpu95-throttle.service failed: status {st}"
                        );
                    }
                    Err(e) => {
                        log::warn!("systemctl disable --now cpu95-throttle.service failed: {e}");
                    }
                }
            }
            DaemonResponse::ThermalStatus(build_thermal_status())
        }
    };

    let elapsed = t0.elapsed().as_millis() as u64;
    match &response {
        DaemonResponse::Error(e) => {
            log::warn!("cmd {label} → error: {e} ({elapsed} ms)")
        }
        DaemonResponse::Ok if write => log::info!("cmd {label} → ok ({elapsed} ms)"),
        _ if write => log::info!("cmd {label} → done ({elapsed} ms)"),
        _ => log::trace!("cmd {label} → ok ({elapsed} ms)"),
    }
    response
}

fn rgb_health_str(h: rgb_panic::Health) -> &'static str {
    match h {
        rgb_panic::Health::Ok => "ok",
        rgb_panic::Health::SoftIssue => "soft-issue",
        rgb_panic::Health::HardwareBroken => "broken",
        rgb_panic::Health::NotApplicable => "n/a",
    }
}

/// Periodic Spectrum panic detection — uses kernel log + HID ioctl probe.
fn rgb_watchdog(shutdown: Arc<AtomicBool>) {
    let mut cooldown_ticks = 0u32;
    // Delay first probe so boot USB settle finishes.
    for _ in 0..40 {
        if shutdown.load(Ordering::Relaxed) {
            return;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    while !shutdown.load(Ordering::Relaxed) {
        for _ in 0..30 {
            if shutdown.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        if shutdown.load(Ordering::Relaxed) {
            break;
        }
        if cooldown_ticks > 0 {
            cooldown_ticks -= 1;
            continue;
        }
        let d = rgb_panic::diagnose();
        if !rgb_panic::needs_autofix(&d) {
            continue;
        }
        log::warn!("RGB panic detected: {} — running auto-fix", d.summary);
        for line in &d.details {
            log::debug!("rgb: {line}");
        }
        let report = rgb_panic::troubleshoot();
        for s in &report.steps {
            log::info!("rgb-fix: {s}");
        }
        for e in &report.errors {
            log::warn!("rgb-fix: {e}");
        }
        log::info!(
            "rgb-fix done: {} ({})",
            report.after.summary,
            rgb_health_str(report.after.health)
        );
        // ~2 minutes before another autofix attempt.
        cooldown_ticks = 8;
    }
}
