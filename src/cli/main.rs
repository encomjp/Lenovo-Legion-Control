//! Legion CLI — command-line control for Lenovo Legion laptops.

use clap::{Parser, Subcommand};
use legion_core::{
    comms::{send_command, DaemonCommand, DaemonResponse},
    diagnostics, fans, selftest,
};

/// Print an operational-failure banner to stderr and exit 1 (stdout stays empty).
fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1);
}

/// Print a usage error to stderr and exit 2 (stdout stays empty).
fn usage_fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(2);
}

/// Unwrap a daemon response, failing with exit 1 on any error variant.
fn expect_ok(resp: Result<DaemonResponse, String>) -> Result<(), String> {
    match resp {
        Ok(DaemonResponse::Ok) => Ok(()),
        Ok(DaemonResponse::Error(e)) => fail(e),
        Err(e) => fail(e),
        _ => fail("unexpected response from service"),
    }
}

/// Send a command and fail with exit 1 unless the daemon answers Ok.
fn send_ok(cmd: DaemonCommand) -> Result<(), String> {
    expect_ok(send_command(cmd))
}

/// Show (no state) or set (on|off via the daemon) an ideapad_acpi toggle.
fn ideapad_toggle(
    name: &str,
    toggle: legion_core::ideapad::Toggle,
    state: Option<&str>,
    make_cmd: fn(bool) -> DaemonCommand,
) {
    if !legion_core::ideapad::supported(toggle) {
        fail(format!("{name} is not supported on this laptop"));
    }
    match state {
        None => match legion_core::ideapad::get(toggle) {
            Some(on) => println!("{name:<12} {}", if on { "on" } else { "off" }),
            None => fail(format!("cannot read {name}")),
        },
        Some(s) => {
            let on = match s.to_lowercase().as_str() {
                "on" | "true" | "1" => true,
                "off" | "false" | "0" => false,
                _ => usage_fail(format!("unknown state '{s}' (use on|off)")),
            };
            if let Err(e) = send_ok(make_cmd(on)) {
                fail(e);
            }
            println!("{name} → {}", if on { "on" } else { "off" });
        }
    }
}

/// Fetch the GPU working-mode status from the daemon (exit 1 on failure).
fn fetch_gpu_mode() -> legion_core::gpu_mode::GpuModeStatus {
    match send_command(DaemonCommand::GetGpuMode) {
        Ok(DaemonResponse::GpuMode(st)) => st,
        Ok(DaemonResponse::Error(e)) => fail(e),
        Err(e) if e.contains("variant index") || e.contains("Parse:") => {
            fail("the running legion-control service is too old for gpu-mode; restart/update it")
        }
        Err(e) => fail(e),
        _ => fail("unexpected response from service"),
    }
}

fn print_gpu_mode(st: &legion_core::gpu_mode::GpuModeStatus) {
    use legion_core::gpu_mode::GpuMode;
    let name = |m: Option<GpuMode>| m.map_or("unknown", |m| m.label());
    println!("GPU mode (MUX)");
    println!(
        "  supported      {}",
        if st.supported { "yes" } else { "no" }
    );
    println!("  backend        {}", st.backend);
    println!("  model          {}", st.model_note);
    println!("  active (boot)  {}", name(st.active));
    println!("  firmware       {}", name(st.configured));
    if st.reboot_pending {
        println!(
            "  pending        reboot required to apply {}",
            name(st.configured)
        );
    }
    println!(
        "  iGPU driver    {}",
        st.igpu_driver.as_deref().unwrap_or("none bound")
    );
    if !st.detail.is_empty() {
        println!("  note           {}", st.detail);
    }
}

/// `legion-cli gpu-mode [hybrid|dgpu] [--yes-i-understand]`.
fn gpu_mode_cmd(mode: Option<&str>, ack: bool) {
    use legion_core::gpu_mode::GpuMode;
    let Some(raw) = mode else {
        print_gpu_mode(&fetch_gpu_mode());
        return;
    };
    let Some(target) = GpuMode::parse(raw) else {
        usage_fail(format!("unknown GPU mode '{raw}' (use hybrid|dgpu)"));
    };
    if !ack {
        eprintln!("{}", legion_core::gpu_mode::RISK_TEXT);
        eprintln!();
        eprintln!(
            "error: re-run with --yes-i-understand to switch to {} (nothing was changed)",
            target.label()
        );
        std::process::exit(2);
    }
    match send_command(DaemonCommand::SetGpuMode {
        mode: target,
        acknowledge: true,
    }) {
        Ok(DaemonResponse::GpuMode(st)) => {
            print_gpu_mode(&st);
            println!();
            println!(
                "GPU mode set to {}. Reboot to apply. If the screen stays black, enter BIOS setup (F2) and change the GPU mode back.",
                target.label()
            );
        }
        Ok(DaemonResponse::Error(e)) => fail(e),
        Err(e) if e.contains("variant index") || e.contains("Parse:") => {
            fail("the running legion-control service is too old for gpu-mode; restart/update it")
        }
        Err(e) => fail(e),
        _ => fail("unexpected response from service"),
    }
}

#[derive(Parser)]
#[command(
    name = "legion-cli",
    about = "Lenovo Legion laptop control",
    version,
    propagate_version = true
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Show all sensor readings
    Status,
    /// Monitor sensors live (2s refresh)
    Watch,
    /// Show current platform profile
    Profile,
    /// Set platform profile
    SetProfile {
        /// Profile name: quiet/low-power, balanced, performance, max-power, custom
        name: String,
    },
    /// Show fan speeds
    Fan,
    /// Set fan target RPM (0 = auto)
    SetFan {
        /// Fan number: 1 (CPU), 2 (GPU), or 3/4 (Aux — fan3 on legion_hwmon)
        fan: u8,
        /// Target RPM (0 for auto)
        rpm: u32,
    },
    /// Set all fans to auto
    FanAuto,
    /// Show keyboard brightness
    Kbd,
    /// Set keyboard brightness (0-2)
    SetKbd {
        /// Brightness level: 0=off, 1=low, 2=high
        level: u8,
    },
    /// Set keyboard to static RGB color
    Rgb { r: u8, g: u8, b: u8 },
    /// Spectrum lighting effect (Gen 10)
    Effect {
        /// static, color-pulse, color-wave, rainbow-wave, screw-rainbow, smooth, rain, ripple, reactive, off
        name: String,
        /// Red (for color effects)
        #[arg(default_value_t = 200)]
        r: u8,
        #[arg(default_value_t = 16)]
        g: u8,
        #[arg(default_value_t = 46)]
        b: u8,
        #[arg(long, default_value_t = 2)]
        speed: u8,
        /// all, keyboard, front, rear, logo, chassis
        #[arg(long, default_value = "all")]
        zone: String,
    },
    /// Spectrum brightness 0-9
    Brightness { level: u8 },
    /// 4-zone RGB keyboard (Legion 5 2020-2024, IdeaPad Gaming, LOQ) — experimental
    Zones {
        #[command(subcommand)]
        action: ZonesCmd,
    },
    /// Show battery info
    Battery,
    /// Set charge limit: 60, 80, or 100
    ChargeLimit { pct: u32 },
    /// Set battery conservation mode (legacy → 60%)
    Conservation { state: String },
    /// Show device info
    Info,
    /// Show camera power state
    Camera,
    /// Diagnose onboard speakers / AW88399 smart amp
    Audio,
    /// Soft-reset speakers (UCM, unmute, PipeWire, default sink)
    AudioFix,
    /// Show SMT / hyperthreading state
    Smt,
    /// Enable or disable SMT (hyperthreading)
    SetSmt {
        /// on or off
        state: String,
    },
    /// Show CPU frequency boost state
    Boost,
    /// Enable or disable CPU boost (turbo)
    SetBoost {
        /// on or off
        state: String,
    },
    /// Diagnose Spectrum RGB panic (HID + kernel USB faults)
    RgbStatus,
    /// Auto-fix Spectrum RGB panic (soft reset → USB reset → rebind)
    RgbFix,
    /// Show Legion logo LED state
    Logo,
    /// Turn Legion logo LED on or off
    SetLogo {
        /// on or off
        state: String,
    },
    /// Show or set Fn Lock (ideapad_acpi): no argument shows, on|off sets
    FnLock {
        /// on or off
        state: Option<String>,
    },
    /// Show or set always-on USB charging (ideapad_acpi): no argument shows, on|off sets
    UsbCharging {
        /// on or off
        state: Option<String>,
    },
    /// Show recent daemon log lines
    Logs {
        /// Number of lines to fetch (default 50)
        #[arg(default_value_t = 50)]
        n: usize,
    },
    /// Set daemon log level (info, debug, trace, warn, error)
    SetLogLevel {
        /// Log level
        level: String,
    },
    /// Show AMD Curve Optimizer capability and current per-core offsets
    Undervolt,
    /// Apply a temporary all-core Curve Optimizer offset (reboot resets it)
    SetUndervolt {
        /// Negative Curve Optimizer offset, conservatively limited to -30..0
        #[arg(long, allow_hyphen_values = true)]
        offset: i16,
        /// Confirm that unstable undervolts can crash or corrupt work
        #[arg(long)]
        i_understand_instability_risk: bool,
    },
    /// Restore the firmware baseline observed before Legion Control's first write
    ResetUndervolt {
        /// Confirm the temporary SMU write
        #[arg(long)]
        i_understand_instability_risk: bool,
    },
    /// Software fan curve (per power mode): show | set | enable | disable
    FanCurve {
        #[command(subcommand)]
        action: Option<FanCurveCmd>,
    },
    /// Thermal throttle control
    Thermal {
        #[command(subcommand)]
        command: ThermalCmd,
    },
    /// Anonymous diagnostics (alpha) — on by default; turn off in Settings → Setup.
    Diagnose {
        #[command(subcommand)]
        action: DiagAction,
    },
    /// Show or set the GPU working mode (MUX): no argument shows, hybrid|dgpu sets.
    /// A change takes effect after a reboot; BIOS setup (F2) is the way back.
    GpuMode {
        /// hybrid (iGPU drives the panel, dGPU on demand) or dgpu (dGPU only)
        mode: Option<String>,
        /// Confirm you understand: reboot required, a wrong mode can leave the
        /// internal panel black, and the fix is BIOS setup (F2) → GPU mode
        #[arg(long)]
        yes_i_understand: bool,
    },
    /// Check GitHub for new releases of Legion Control
    CheckUpdate {
        /// Download and install the matching release (AppImage, deb, rpm, Arch, tarball)
        #[arg(long)]
        apply: bool,
        /// With --apply: install the latest release even if it is not newer
        /// than the running version (reinstall / roll back). Signatures are
        /// still required.
        #[arg(long, requires = "apply")]
        allow_downgrade: bool,
    },
}

#[derive(Debug, Clone, Subcommand)]
enum ZonesCmd {
    /// Show the detected 4-zone controller and the saved state (no write)
    Status,
    /// Static colours, zones left → right (#RRGGBB or RRGGBB)
    Static {
        c1: String,
        c2: String,
        c3: String,
        c4: String,
        /// 1 = low, 2 = high
        #[arg(long, default_value_t = 2)]
        brightness: u8,
    },
    /// Animated effect: breath (uses saved zone colours), wave, smooth
    Effect {
        /// breath, wave, smooth
        name: String,
        /// 1 (slowest) - 4 (fastest)
        #[arg(long, default_value_t = 2)]
        speed: u8,
        /// Wave direction: left or right
        #[arg(long, default_value = "right")]
        dir: String,
        /// 1 = low, 2 = high
        #[arg(long, default_value_t = 2)]
        brightness: u8,
    },
    /// Turn the 4-zone backlight off
    Off,
}

#[derive(Debug, Clone, Subcommand)]
enum DiagAction {
    /// Print the full ANONYMOUS diagnostics JSON (no upload)
    Dump {
        /// Include the deep block (hourly-report content, e.g. fan_evidence)
        #[arg(long)]
        deep: bool,
    },
    /// Run read-only self-health checks and print a pass/fail table
    Selfcheck,
    /// Scan for active machine anomalies (fan stalls, hot NVMe, limiter
    /// bypass, config unwritable …). Exit 1 on any CRITICAL fault.
    Faults,
    /// Collect and POST to the configured collector endpoint
    Send {
        /// Override collector URL (else Settings/default)
        #[arg(long)]
        endpoint: Option<String>,
    },
    /// Turn anonymous telemetry on (for this user and the root daemon)
    Enable,
    /// Turn anonymous telemetry off (for this user and the root daemon)
    Disable,
}

#[derive(Subcommand)]
enum FanCurveCmd {
    /// Show stored curves and live engine status (default)
    Show,
    /// Store a curve: 8 `temp:pct` pairs, e.g. 30:0,40:15,50:25,60:38,70:52,80:68,90:86,100:100
    Set {
        /// Platform profile (balanced, performance, custom, low-power, …)
        profile: String,
        /// 8 comma-separated temp:pct pairs (20–100 °C rising, 0–100 % not falling, last = 100 % at ≥90 °C)
        points: String,
        /// Store without enabling
        #[arg(long)]
        keep_disabled: bool,
    },
    /// Enable the stored (or factory) curve for a profile
    Enable { profile: String },
    /// Disable the curve for a profile (fans return to firmware control)
    Disable { profile: String },
}

#[derive(Subcommand)]
enum ThermalCmd {
    /// Show thermal throttle status
    Status,
    /// Set thermal throttle (max 70–98°C, 96–98 needs ack)
    Set {
        /// Max temperature 70–98 (default 90). Enables if --off not given.
        #[arg(long)]
        max_temp: Option<u8>,
        /// Disable throttle (enabled=false)
        #[arg(long)]
        off: bool,
        /// Explicitly enable (default when max_temp given)
        #[arg(long)]
        on: bool,
        /// Acknowledge 96–98°C exceeds TjMax 95°C
        #[arg(long)]
        acknowledge_high_temp: bool,
    },
}

fn main() {
    let cli = Cli::parse();
    legion_core::logging::init("legion-cli");
    log::debug!("cli command: {:?}", std::env::args().collect::<Vec<_>>());
    log::trace!("cmd {}", cmd_label(&cli.command));

    match cli.command {
        Commands::Status => {
            let resp = send_command(DaemonCommand::GetSensors);
            print_sensors(resp);
        }
        Commands::Watch => {
            let mut iteration = 0u64;
            loop {
                iteration += 1;
                log::trace!("watch: iteration {iteration}");
                print!("\x1B[2J\x1B[H");
                println!("Legion Control: live sensors (Ctrl+C to quit)\n");
                let resp = send_command(DaemonCommand::GetSensors);
                print_sensors(resp);
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
        }
        Commands::Profile => match send_command(DaemonCommand::GetProfile) {
            Ok(DaemonResponse::Profile(p)) => println!("{}", friendly_profile(&p)),
            Ok(DaemonResponse::Error(e)) => fail(e),
            Err(e) => fail(e),
            _ => fail("unexpected response from service"),
        },
        Commands::SetProfile { name } => {
            let name = normalize_profile(&name);
            if let Some(why) = legion_core::profile::blocked_reason(&name) {
                fail(format!("refusing to set '{name}': {why}"));
            }
            if name == "max-power" {
                eprintln!(
                    "warning: Max Power / Extreme can overheat this laptop and wear hardware.\n\
                     Prefer Performance unless you have strong cooling (e.g. Llano / Thermaltake Extreme).\n\
                     Thermal pads alone are often not enough for sustained Extreme."
                );
            }
            send_ok(DaemonCommand::SetProfile(name.clone())).ok();
            println!("profile → {}", friendly_profile(&name));
        }
        Commands::Fan => {
            let [f1, f2, aux] = fans::slot_ids();
            for (fan, name) in [(f1, "CPU"), (f2, "GPU"), (aux, "Aux")] {
                match send_command(DaemonCommand::GetFanRpm(fan)) {
                    Ok(DaemonResponse::FanRpm(r)) => println!("{name:>3} fan: {r:>5} RPM"),
                    Ok(DaemonResponse::Error(e)) => eprintln!("{name} fan: {e}"),
                    Err(e) => eprintln!("{name} fan: {e}"),
                    _ => {}
                }
            }
        }
        Commands::SetFan { fan, rpm } => {
            if !matches!(fan, 1..=4) {
                usage_fail(format!(
                    "invalid fan index '{fan}'. Valid fan channels: 1 (CPU), 2 (GPU), 3 or 4 (Aux)"
                ));
            }
            if rpm > 8000 {
                usage_fail(format!(
                    "RPM target '{rpm}' exceeds safety limit of 8000 RPM"
                ));
            }
            send_ok(DaemonCommand::SetFanTarget(fan, rpm)).ok();
            if rpm == 0 {
                println!("fan {fan} → auto");
            } else {
                println!("fan {fan} → {rpm} RPM");
                if fans::manual_target_may_persist() {
                    eprintln!("warning: {}", fans::GEN10_MANUAL_TARGET_WARNING);
                }
            }
        }
        Commands::FanAuto => {
            let mut errors: Vec<String> = Vec::new();
            for fan in fans::slot_ids() {
                log::debug!("fan-auto: fan {fan} → auto (attempt)");
                match send_command(DaemonCommand::SetFanTarget(fan, 0)) {
                    Ok(DaemonResponse::Ok) => log::debug!("fan-auto: fan {fan} ok"),
                    Ok(DaemonResponse::Error(e)) => {
                        log::warn!("fan-auto: fan {fan} refused: {e}");
                        errors.push(format!("fan {fan}: {e}"));
                    }
                    Err(e) => {
                        log::warn!("fan-auto: fan {fan} failed: {e}");
                        errors.push(format!("fan {fan}: {e}"));
                    }
                    _ => {
                        log::warn!("fan-auto: fan {fan} unexpected response");
                        errors.push(format!("fan {fan}: unexpected response"));
                    }
                }
            }
            if errors.is_empty() {
                println!("all fans → auto");
            } else {
                fail(format!("fans → auto failed: {}", errors.join("; ")));
            }
        }
        Commands::Kbd => match send_command(DaemonCommand::GetKbdBrightness) {
            Ok(DaemonResponse::KbdBrightness(b)) => {
                // Could be LED backlight (0-2) or Spectrum RGB brightness (0-9).
                let label = match b {
                    0 => "off",
                    1 => "low",
                    2 => "high",
                    3..=9 => "spectrum",
                    _ => "unknown",
                };
                println!("backlight: {label} ({b})");
            }
            Ok(DaemonResponse::Error(e)) => fail(e),
            Err(e) => fail(e),
            _ => fail("cannot read keyboard brightness"),
        },
        Commands::SetKbd { level } => {
            if level > 9 {
                usage_fail(format!("backlight level '{level}' out of range (0-2 for white/4-zone, 0-9 for Spectrum)"));
            }
            send_ok(DaemonCommand::SetKbdBrightness(level)).ok();
            println!("backlight → {level}");
        }
        Commands::Rgb { r, g, b } => match legion_core::keyboard::set_rgb_static(r, g, b) {
            Ok(()) => println!("rgb → #{r:02X}{g:02X}{b:02X}"),
            Err(e) => fail(e),
        },
        Commands::Effect {
            name,
            r,
            g,
            b,
            speed,
            zone,
        } => {
            let z = legion_core::keyboard::RgbZone::from_name(&zone).unwrap_or_else(|| {
                eprintln!("warn: unknown zone '{zone}', using all");
                legion_core::keyboard::RgbZone::All
            });
            if name.eq_ignore_ascii_case("off") {
                log::debug!("effect: 'off' matched");
                match legion_core::keyboard::set_rgb_effect_zone(
                    legion_core::keyboard::RgbEffect::Static,
                    0,
                    0,
                    0,
                    2,
                    9,
                    z,
                ) {
                    Ok(()) => println!("effect → off, {}", z.name()),
                    Err(e) => fail(e),
                }
            } else if let Some(fx) = legion_core::keyboard::RgbEffect::from_name(&name) {
                log::debug!("effect: '{name}' matched");
                match legion_core::keyboard::set_rgb_effect_zone(fx, r, g, b, speed, 9, z) {
                    Ok(()) => println!("effect → {name}, {}", z.name()),
                    Err(e) => fail(e),
                }
            } else {
                log::debug!("effect: '{name}' not found");
                usage_fail(format!(
                    "unknown effect '{}'. Try: {}",
                    name,
                    legion_core::keyboard::RgbEffect::all_names().join(", ")
                ));
            }
        }
        Commands::Brightness { level } => {
            if level > 9 {
                usage_fail(format!("brightness level '{level}' out of range (0-9)"));
            }
            match legion_core::keyboard::set_rgb_brightness(level) {
                Ok(()) => println!("brightness → {level}/9"),
                Err(e) => fail(format!("{e}. Run as root or ensure udev rules are loaded.")),
            }
        }
        Commands::Zones { action } => zones_cmd(action),
        Commands::Battery => match send_command(DaemonCommand::GetBattery) {
            Ok(DaemonResponse::Battery {
                capacity,
                status,
                voltage,
                cycles,
                conservation,
            }) => {
                let lim = match send_command(DaemonCommand::GetChargeLimit) {
                    Ok(DaemonResponse::ChargeLimit(p)) => p,
                    _ => {
                        if conservation {
                            60
                        } else {
                            100
                        }
                    }
                };
                println!("battery     {capacity}% ({status})");
                println!("voltage     {voltage:.2} V");
                println!("cycles      {cycles}");
                println!(
                    "limit       {lim}% ({})",
                    legion_core::battery::charge_limit_label(lim)
                );
            }
            Ok(DaemonResponse::Error(e)) => fail(e),
            Err(e) => fail(e),
            _ => fail("cannot read battery status from service"),
        },
        Commands::ChargeLimit { pct } => {
            let valid_pct = match pct {
                60 | 80 | 100 => pct,
                0..=69 => 60,
                70..=89 => 80,
                90..=100 => 100,
                _ => usage_fail(format!(
                    "charge limit '{pct}' invalid; must be 60, 80, or 100%"
                )),
            };
            send_ok(DaemonCommand::SetChargeLimit(valid_pct)).ok();
            println!("charge limit → {valid_pct}%");
        }
        Commands::Conservation { state } => {
            let on = match state.to_lowercase().as_str() {
                "on" | "true" | "1" => true,
                "off" | "false" | "0" => false,
                _ => usage_fail(format!("unknown state '{state}' (use on|off)")),
            };
            log::debug!("conservation: state '{state}' → {on}");
            send_ok(DaemonCommand::SetConservation(on)).ok();
            println!(
                "conservation → {}",
                if on { "on (~60%)" } else { "off (100%)" }
            );
        }
        Commands::Info => match send_command(DaemonCommand::GetDeviceInfo) {
            Ok(DaemonResponse::DeviceInfo(info)) => {
                let cli_ver = env!("CARGO_PKG_VERSION");
                match legion_core::comms::query_daemon_version() {
                    Ok(dver) if dver == cli_ver => {
                        println!("daemon      v{dver} (in sync with CLI v{cli_ver})");
                    }
                    Ok(dver) => {
                        println!("daemon      v{dver} (MISMATCH: CLI is v{cli_ver}: restart daemon!)");
                    }
                    Err(e) => {
                        println!("daemon      unknown / legacy ({e})");
                    }
                }
                println!("model       {}", info.model);
                println!("machine     {}", info.machine_type);
                println!("series      {}", info.series);
                println!(
                    "generation  {}",
                    if info.gen > 0 {
                        info.gen.to_string()
                    } else {
                        "unknown".into()
                    }
                );
                println!("bios        {}", info.bios_version);
                println!("bios_family {}", info.bios_prefix);
                println!(
                    "matched     {}",
                    if info.profile_matched {
                        format!("yes ({})", info.profile_source)
                    } else {
                        format!("no ({})", info.profile_source)
                    }
                );
                if !info.profile_notes.is_empty() {
                    println!("notes       {}", info.profile_notes);
                }
                println!("ec          {}", info.ec_chip);
                println!("cpu         {}", info.cpu_model);
                println!("gpu         {}", info.gpu_model);
                println!("fan_backend {}", info.capabilities.fan_backend);
                println!("lighting    {}", info.capabilities.lighting);
                match info.capabilities.peak_gpu_w {
                    Some(w) => println!("peak_gpu_w  {w} ({})", info.capabilities.peak_gpu_source),
                    None => println!("peak_gpu_w  n/a ({})", info.capabilities.peak_gpu_source),
                }
                for f in &info.capabilities.fans {
                    println!(
                        "fan{:<8} {}, {}–{} RPM, now {}",
                        f.id, f.title, f.min_rpm, f.max_rpm, f.current_rpm
                    );
                }
                if !info.capabilities.ppt_attrs.is_empty() {
                    println!("ppt_attrs:");
                    for a in &info.capabilities.ppt_attrs {
                        println!("  - {a}");
                    }
                }
                if !info.capabilities.platform_profiles.is_empty() {
                    println!(
                        "profiles    {}",
                        info.capabilities.platform_profiles.join(", ")
                    );
                }
                for (profile, why) in legion_core::profile::blocked_choices() {
                    println!("blocked     {profile}: {why}");
                }
            }
            Ok(DaemonResponse::Error(e)) => fail(e),
            Err(e) => fail(e),
            _ => fail("cannot get device info from service"),
        },
        Commands::Camera => match send_command(DaemonCommand::GetCameraPower) {
            Ok(DaemonResponse::CameraPower(killed)) => {
                println!(
                    "camera      {}",
                    if killed { "privacy kill active" } else { "on" }
                );
            }
            Ok(DaemonResponse::Error(e)) => fail(e),
            Err(e) => fail(e),
            _ => fail("cannot read camera power"),
        },
        Commands::Audio => {
            let d = legion_core::audio::diagnose();
            let tag = match d.health {
                legion_core::audio::Health::Ok => "ok",
                legion_core::audio::Health::SoftIssue => "soft-issue",
                legion_core::audio::Health::HardwareBroken => "broken",
                legion_core::audio::Health::NotApplicable => "n/a",
                legion_core::audio::Health::DriverMissing => "driver-missing",
            };
            println!("status      {tag}");
            println!("summary     {}", d.summary);
            println!("amp_acpi    {}", d.amp_acpi);
            println!("amp_bound   {}", d.amp_bound);
            println!("amp_modules {}", d.amp_modules);
            println!("firmware    {}", d.firmware_ok);
            println!(
                "hda_card    {}",
                d.hda_card
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "-".into())
            );
            println!("muted       {}", d.speakers_muted);
            println!("bass_off    {}", d.bass_off);
            println!("volume_low  {}", d.volume_low);
            println!("wrong_sink  {}", d.wrong_default_sink);
            if let Some(s) = &d.default_sink {
                println!("default     {s}");
            }
            if let Some(s) = &d.internal_sink {
                println!("internal    {s}");
            }
            for line in &d.details {
                println!("· {line}");
            }
            if matches!(
                d.health,
                legion_core::audio::Health::HardwareBroken
                    | legion_core::audio::Health::DriverMissing
            ) {
                std::process::exit(2);
            }
            if d.health == legion_core::audio::Health::SoftIssue {
                std::process::exit(1);
            }
        }
        Commands::AudioFix => {
            {
                let pre = legion_core::audio::diagnose();
                if pre.health == legion_core::audio::Health::DriverMissing
                    && pre.amp_acpi
                    && !pre.firmware_ok
                {
                    println!("AW88399 firmware missing; installing via PolicyKit helper...");
                    match legion_core::amp_firmware::install_via_helper() {
                        Ok(msg) => println!("ok  {msg}"),
                        Err(e) => eprintln!("err firmware install: {e}"),
                    }
                }
            }
            let report = legion_core::audio::troubleshoot();
            for s in &report.steps {
                println!("ok  {s}");
            }
            for e in &report.errors {
                eprintln!("err {e}");
            }
            let d = &report.after;
            let tag = match d.health {
                legion_core::audio::Health::Ok => "ok",
                legion_core::audio::Health::SoftIssue => "soft-issue",
                legion_core::audio::Health::HardwareBroken => "broken",
                legion_core::audio::Health::NotApplicable => "n/a",
                legion_core::audio::Health::DriverMissing => "driver-missing",
            };
            println!("status      {tag}");
            println!("summary     {}", d.summary);
            if matches!(
                d.health,
                legion_core::audio::Health::HardwareBroken
                    | legion_core::audio::Health::DriverMissing
            ) {
                std::process::exit(2);
            }
            if d.health == legion_core::audio::Health::SoftIssue || !report.errors.is_empty() {
                std::process::exit(1);
            }
        }
        Commands::Smt => match send_command(DaemonCommand::GetSmt) {
            Ok(DaemonResponse::Smt {
                active,
                control,
                logical_cpus,
            }) => {
                println!(
                    "smt         {} ({control})",
                    if active { "on" } else { "off" }
                );
                println!("logical     {logical_cpus}");
            }
            Ok(DaemonResponse::Error(e)) => fail(e),
            Err(e) => fail(e),
            _ => fail("unexpected response from service"),
        },
        Commands::SetSmt { state } => {
            let on = match state.to_lowercase().as_str() {
                "on" | "1" | "true" | "enable" => true,
                "off" | "0" | "false" | "disable" => false,
                _ => usage_fail("use on or off"),
            };
            log::debug!("set-smt: state '{state}' → {on}");
            if !on {
                let n = legion_core::cpu::logical_cpus().max(2);
                let half = (n / 2).max(1);
                eprintln!(
                    "warning: disabling SMT halves logical CPUs (about {n}→{half}). \
                     Helps some latency-sensitive games; hurts multi-threaded loads."
                );
            }
            send_ok(DaemonCommand::SetSmt(on)).ok();
            println!("smt → {}", if on { "on" } else { "off" });
            if let Ok(DaemonResponse::Smt {
                logical_cpus,
                active,
                ..
            }) = send_command(DaemonCommand::GetSmt)
            {
                println!(
                    "now         {}, {logical_cpus} logical CPUs",
                    if active { "on" } else { "off" }
                );
            }
        }
        Commands::Boost => match send_command(DaemonCommand::GetBoost) {
            Ok(DaemonResponse::Boost(on)) => {
                println!("boost       {}", if on { "on" } else { "off" });
            }
            Ok(DaemonResponse::Error(e)) => fail(e),
            Err(e) => fail(e),
            _ => fail("unexpected response from service"),
        },
        Commands::SetBoost { state } => {
            let on = match state.to_lowercase().as_str() {
                "on" | "1" | "true" | "enable" => true,
                "off" | "0" | "false" | "disable" => false,
                _ => usage_fail("use on or off"),
            };
            log::debug!("set-boost: state '{state}' → {on}");
            send_ok(DaemonCommand::SetBoost(on)).ok();
            println!("boost → {}", if on { "on" } else { "off" });
        }
        Commands::RgbStatus => match send_command(DaemonCommand::DiagnoseRgb) {
            Ok(DaemonResponse::RgbDiagnosis {
                health,
                summary,
                details,
                fixable,
            }) => {
                println!("status      {health}");
                println!("summary     {summary}");
                println!("fixable     {fixable}");
                for line in details {
                    println!("· {line}");
                }
                if health == "broken" {
                    std::process::exit(2);
                }
                if health == "soft-issue" {
                    std::process::exit(1);
                }
            }
            other => {
                if let Ok(DaemonResponse::Error(e)) | Err(e) = other {
                    eprintln!("note: daemon probe unavailable ({e}): local check");
                }
                rgb_status_local();
            }
        },
        Commands::RgbFix => match send_command(DaemonCommand::FixRgbPanic) {
            Ok(DaemonResponse::RgbFixReport {
                steps,
                errors,
                health,
                summary,
            }) => {
                for s in steps {
                    println!("ok  {s}");
                }
                for e in errors {
                    eprintln!("err {e}");
                }
                println!("status      {health}");
                println!("summary     {summary}");
                if health == "broken" {
                    std::process::exit(2);
                }
                if health == "soft-issue" {
                    std::process::exit(1);
                }
            }
            other => {
                eprintln!(
                    "note: using local soft fix (update/restart legion-control for USB reset)"
                );
                if let Ok(DaemonResponse::Error(e)) | Err(e) = other {
                    eprintln!("note: {e}");
                }
                rgb_fix_local();
            }
        },
        Commands::Logo => match legion_core::keyboard::logo_on() {
            Some(true) => println!("logo        on"),
            Some(false) => println!("logo        off"),
            None => eprintln!("error: cannot read logo state"),
        },
        Commands::SetLogo { state } => {
            let on = match state.to_lowercase().as_str() {
                "on" | "true" | "1" => true,
                "off" | "false" | "0" => false,
                _ => usage_fail(format!("unknown state '{state}' (use on|off)")),
            };
            log::debug!("set-logo: state '{state}' → {on}");
            send_ok(DaemonCommand::SetLogo(on)).ok();
            println!("logo → {}", if on { "on" } else { "off" });
        }
        Commands::FnLock { state } => ideapad_toggle(
            "fn-lock",
            legion_core::ideapad::Toggle::FnLock,
            state.as_deref(),
            DaemonCommand::SetFnLock,
        ),
        Commands::UsbCharging { state } => ideapad_toggle(
            "usb-charging",
            legion_core::ideapad::Toggle::UsbCharging,
            state.as_deref(),
            DaemonCommand::SetUsbCharging,
        ),
        Commands::Logs { n } => match send_command(DaemonCommand::GetRecentLogs(n)) {
            Ok(DaemonResponse::RecentLogs(text)) => {
                if text.is_empty() {
                    println!("(no log entries)");
                } else {
                    print!("{text}");
                }
            }
            Ok(DaemonResponse::Error(e)) => fail(e),
            Err(e) => fail(e),
            _ => fail("unexpected response from service"),
        },
        Commands::SetLogLevel { level } => {
            let valid = ["trace", "debug", "info", "warn", "error", "json"];
            if !valid.contains(&level.to_lowercase().as_str()) {
                usage_fail(format!(
                    "invalid log level '{level}'. Valid levels: {}",
                    valid.join(", ")
                ));
            }
            send_ok(DaemonCommand::SetLogLevel(level.clone())).ok();
            println!("log level → {level}");
        }
        Commands::Undervolt => match send_command(DaemonCommand::GetCurveOptimizer) {
            Ok(DaemonResponse::CurveOptimizer(status)) => print_curve_optimizer(&status),
            Ok(DaemonResponse::Error(e)) | Err(e) => fail(e),
            _ => fail("unexpected response from service"),
        },
        Commands::SetUndervolt {
            offset,
            i_understand_instability_risk,
        } => {
            if !(-30..=0).contains(&offset) {
                usage_fail(format!(
                    "offset '{offset}' is out of range. Curve Optimizer offsets must be between -30 and 0"
                ));
            }
            if !i_understand_instability_risk {
                eprintln!(
                    "error: pass --i-understand-instability-risk after testing a conservative value"
                );
                std::process::exit(2);
            }
            match send_command(DaemonCommand::SetCurveOptimizer {
                offset,
                acknowledge: true,
            }) {
                Ok(DaemonResponse::CurveOptimizer(status)) => print_curve_optimizer(&status),
                Ok(DaemonResponse::Error(e)) | Err(e) => fail(e),
                _ => fail("unexpected response from service"),
            }
        }
        Commands::ResetUndervolt {
            i_understand_instability_risk,
        } => {
            if !i_understand_instability_risk {
                eprintln!("error: pass --i-understand-instability-risk to confirm the SMU write");
                std::process::exit(2);
            }
            match send_command(DaemonCommand::ResetCurveOptimizerAcknowledged { acknowledge: true })
            {
                Ok(DaemonResponse::CurveOptimizer(status)) => print_curve_optimizer(&status),
                Ok(DaemonResponse::Error(e)) | Err(e) => fail(e),
                _ => fail("unexpected response from service"),
            }
        }
        Commands::FanCurve { action } => fan_curve_cmd(action.unwrap_or(FanCurveCmd::Show)),
        Commands::Thermal { command } => match command {
            ThermalCmd::Status => match send_command(DaemonCommand::GetThermalStatus) {
                Ok(DaemonResponse::ThermalStatus(s)) => print_thermal_status(&s),
                Ok(DaemonResponse::Error(e)) => fail(e),
                Err(e) => fail(e),
                _ => fail("unexpected response from service"),
            },
            ThermalCmd::Set {
                max_temp,
                off,
                on,
                acknowledge_high_temp,
            } => {
                if off && on {
                    eprintln!("error: --off and --on are mutually exclusive");
                    std::process::exit(2);
                }
                let enabled = !off;
                let effective_max = if let Some(v) = max_temp {
                    v
                } else {
                    match send_command(DaemonCommand::GetThermalStatus) {
                        Ok(DaemonResponse::ThermalStatus(s)) => s.config.max_temp,
                        _ => 90,
                    }
                };
                if let Err(e) = legion_core::thermal::validate(effective_max, acknowledge_high_temp)
                {
                    usage_fail(e);
                }
                match send_command(DaemonCommand::SetThermal {
                    enabled,
                    max_temp: effective_max,
                    acknowledge: acknowledge_high_temp,
                }) {
                    Ok(DaemonResponse::ThermalStatus(s)) => print_thermal_status(&s),
                    Ok(DaemonResponse::Error(e)) | Err(e) => fail(e),
                    _ => fail("unexpected response"),
                }
            }
        },
        Commands::Diagnose { action } => match action {
            DiagAction::Enable | DiagAction::Disable => {
                let on = matches!(action, DiagAction::Enable);
                legion_core::config::update(|c| c.diagnostics.enabled = on);
                println!("telemetry {} for this user", if on { "enabled" } else { "disabled" });
                match send_command(DaemonCommand::SetTelemetry(on)) {
                    Ok(DaemonResponse::Ok) => println!("daemon: telemetry {}", if on { "enabled" } else { "disabled" }),
                    Ok(DaemonResponse::Error(e)) | Err(e) => fail(format!("daemon not updated: {e}")),
                    _ => fail("daemon not updated: unexpected response"),
                }
            }
            DiagAction::Dump { deep } => {
                let report = if deep {
                    diagnostics::collect_with_deep("dump")
                } else {
                    diagnostics::collect()
                };
                let json = serde_json::to_string_pretty(&report)
                    .unwrap_or_else(|e| fail(format!("serialize diagnostics report: {e}")));
                log::debug!("diagnose dump: {} bytes", json.len());
                println!("{json}");
            }
            DiagAction::Selfcheck => {
                let checks = selftest::run_self_checks();
                let total = checks.len();
                let passed = checks.iter().filter(|c| c.ok).count();
                log::debug!("diagnose selfcheck: {passed}/{total} passed");
                for c in &checks {
                    let mark = if c.ok { "✓" } else { "✗" };
                    println!("{mark} {}: {}", c.name, c.detail);
                }
                println!("{passed}/{total} passed");
                if passed != total {
                    std::process::exit(1);
                }
            }
            DiagAction::Faults => {
                let faults = selftest::scan_faults();
                let criticals = faults
                    .iter()
                    .filter(|f| f.severity == legion_core::selftest::Severity::Critical)
                    .count();
                let warnings = faults
                    .iter()
                    .filter(|f| f.severity == legion_core::selftest::Severity::Warning)
                    .count();
                let infos = faults
                    .iter()
                    .filter(|f| f.severity == legion_core::selftest::Severity::Info)
                    .count();
                log::debug!(
                    "diagnose faults: {criticals} critical / {warnings} warning / {infos} info"
                );
                if faults.is_empty() {
                    println!("no active faults detected");
                    return;
                }
                for f in &faults {
                    let sev = match f.severity {
                        legion_core::selftest::Severity::Critical => "CRIT",
                        legion_core::selftest::Severity::Warning => "WARN",
                        legion_core::selftest::Severity::Info => "INFO",
                    };
                    println!("[{sev}] {}: {}", f.id, f.detail);
                }
                if criticals > 0 {
                    std::process::exit(1);
                }
            }
            DiagAction::Send { endpoint } => {
                log::debug!(
                    "diagnose send: {}",
                    if endpoint.is_some() {
                        "override endpoint supplied"
                    } else {
                        "endpoint from config/default"
                    }
                );
                match diagnostics::collect_and_send_deep(endpoint.as_deref(), "manual") {
                    Ok(resp) => println!("sent ✓ {}", resp.chars().take(200).collect::<String>()),
                    Err(e) => {
                        eprintln!("error: {e}");
                        std::process::exit(1);
                    }
                }
            }
        },
        Commands::GpuMode {
            mode,
            yes_i_understand,
        } => gpu_mode_cmd(mode.as_deref(), yes_i_understand),
        Commands::CheckUpdate {
            apply,
            allow_downgrade,
        } => match legion_core::update::check_latest_release() {
            Ok(info) => {
                println!("current version: {}", legion_core::update::CURRENT_VERSION);
                println!("latest release:  {} ({})", info.version, info.name);
                if let Some(asset) = legion_core::update::selected_asset(&info) {
                    println!("matching asset:   {} ({} bytes)", asset.name, asset.size);
                } else if let Some(asset) = &info.appimage {
                    println!("appimage:        {} ({} bytes)", asset.name, asset.size);
                }
                if info.is_newer || (apply && allow_downgrade) {
                    if info.is_newer {
                        println!(
                            "\nA new version of Legion Control is available: v{}",
                            info.version
                        );
                    } else {
                        println!(
                            "\n--allow-downgrade: installing v{} over v{}",
                            info.version,
                            legion_core::update::CURRENT_VERSION
                        );
                    }
                    if apply {
                        let opts = legion_core::update::UpdateOptions { allow_downgrade };
                        let kind = legion_core::update::detect_install_kind();
                        match legion_core::update::apply_update_with_options(
                            &info,
                            kind,
                            opts,
                            |phase, bytes, total| {
                                let label = match phase {
                                    legion_core::update::UpdatePhase::Downloading => "downloading",
                                    legion_core::update::UpdatePhase::Verifying => "verifying",
                                    legion_core::update::UpdatePhase::Building
                                    | legion_core::update::UpdatePhase::BuildingLog(_) => {
                                        "building"
                                    }
                                    legion_core::update::UpdatePhase::Installing => "installing",
                                };
                                if let Some(t) = total.filter(|t| *t > 0) {
                                    eprint!("\r{label}: {bytes}/{t} bytes");
                                } else {
                                    eprint!("\r{label}…");
                                }
                                let _ = std::io::Write::flush(&mut std::io::stderr());
                            },
                        ) {
                            Ok(outcome) => {
                                eprintln!();
                                println!("installed: {}", outcome.relaunch.display());
                                println!("Restart Legion Control to switch to the new version.");
                                if outcome.needs_daemon_restage {
                                    println!(
                                        "If the daemon is enabled, the next launch will refresh \
                                         it with one password prompt."
                                    );
                                }
                            }
                            Err(e) => fail(e),
                        }
                    } else if legion_core::update::can_apply(&info) {
                        println!(
                            "To install without a browser:\n  legion-cli check-update --apply"
                        );
                        println!("Or use Update now in Settings → Setup.");
                    } else {
                        println!("{}", legion_core::update::manual_update_hint());
                    }
                } else if apply {
                    println!("\nAlready up to date: nothing to apply.");
                } else {
                    println!("\nLegion Control is up to date.");
                }
            }
            Err(e) => fail(format!("failed to check for updates: {e}")),
        },
    }
}

fn print_curve_optimizer(status: &legion_core::undervolt::CurveOptimizerStatus) {
    println!(
        "curve optimizer: {}",
        if status.available {
            "available"
        } else {
            "unavailable"
        }
    );
    println!("status: {}", status.reason);
    log::debug!(
        "curve optimizer: available={} reason='{}'",
        status.available,
        status.reason
    );
    if let Some(codename) = status.codename {
        println!("ryzen_smu codename: {codename}");
        log::debug!("curve optimizer: ryzen_smu codename {codename}");
    }
    if let Some(driver) = &status.driver_version {
        println!("driver: {driver}");
        log::debug!("curve optimizer: driver {driver}");
    }
    if let Some(firmware) = &status.firmware_version {
        println!("smu firmware: {firmware}");
        log::debug!("curve optimizer: smu firmware {firmware}");
    }
    if !status.current.is_empty() {
        println!(
            "current: {}",
            status
                .current
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" ")
        );
        println!(
            "boot baseline: {}",
            status
                .boot_baseline
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" ")
        );
        if let Some(prev) = status.previous {
            println!("previous: {prev}");
            log::debug!("curve optimizer: previous {prev}");
        }
    }
    println!(
        "allowed temporary range: {}..={}",
        status.minimum, status.maximum
    );
    log::debug!(
        "curve optimizer: {} core offset(s); allowed range {}..={}",
        status.current.len(),
        status.minimum,
        status.maximum
    );
}

fn rgb_status_local() {
    let d = legion_core::rgb_panic::diagnose();
    let tag = match d.health {
        legion_core::rgb_panic::Health::Ok => "ok",
        legion_core::rgb_panic::Health::SoftIssue => "soft-issue",
        legion_core::rgb_panic::Health::HardwareBroken => "broken",
        legion_core::rgb_panic::Health::NotApplicable => "n/a",
    };
    println!("status      {tag} (local: restart legion-control for full USB autofix)");
    println!("summary     {}", d.summary);
    println!("fixable     {}", d.fixable);
    for line in d.details {
        println!("· {line}");
    }
    if d.health == legion_core::rgb_panic::Health::HardwareBroken {
        std::process::exit(2);
    }
    if d.health == legion_core::rgb_panic::Health::SoftIssue {
        std::process::exit(1);
    }
}

fn print_thermal_status(s: &legion_core::thermal::ThermalStatus) {
    let on_off = if s.config.enabled { "on" } else { "off" };
    let state = if s.active { "throttling" } else { "idle" };
    let cpu_temp = s
        .cpu_temp_mc
        .map(|v| format!("{:.1}°C", v as f64 / 1000.0))
        .unwrap_or_else(|| "n/a".into());
    let cpu_temp_2 = s
        .cpu_temp_2_mc
        .map(|v| format!("{:.1}°C", v as f64 / 1000.0))
        .unwrap_or_else(|| "n/a".into());
    println!(
        "Thermal: {}, max {}°C (restore {}°C), cur {} kHz, CPU {} / CPU CCD 2 {}, {}",
        on_off, s.config.max_temp, s.restore_temp, s.cur_max_freq, cpu_temp, cpu_temp_2, state
    );
}

fn rgb_fix_local() {
    let report = legion_core::rgb_panic::troubleshoot();
    for s in report.steps {
        println!("ok  {s}");
    }
    for e in report.errors {
        eprintln!("err {e}");
    }
    let tag = match report.after.health {
        legion_core::rgb_panic::Health::Ok => "ok",
        legion_core::rgb_panic::Health::SoftIssue => "soft-issue",
        legion_core::rgb_panic::Health::HardwareBroken => "broken",
        legion_core::rgb_panic::Health::NotApplicable => "n/a",
    };
    println!("status      {tag}");
    println!("summary     {}", report.after.summary);
    if report.after.health == legion_core::rgb_panic::Health::HardwareBroken {
        std::process::exit(2);
    }
    if report.after.health == legion_core::rgb_panic::Health::SoftIssue {
        std::process::exit(1);
    }
}

fn friendly_profile(name: &str) -> String {
    match name {
        "low-power" => "Quiet (low-power)".into(),
        "balanced" => "Balanced".into(),
        "performance" => "Performance".into(),
        "max-power" => "Max Power".into(),
        "custom" => "Custom".into(),
        "quiet" => "Quiet".into(),
        other => other.to_string(),
    }
}

fn normalize_profile(name: &str) -> String {
    match name.to_lowercase().as_str() {
        "quiet" | "low" | "low-power" | "lowpower" => "low-power".into(),
        "bal" | "balanced" => "balanced".into(),
        "perf" | "performance" => "performance".into(),
        "max" | "max-power" | "maxpower" => "max-power".into(),
        "custom" => "custom".into(),
        other => other.to_string(),
    }
}

fn print_sensors(resp: Result<DaemonResponse, String>) {
    match resp {
        Ok(DaemonResponse::Sensors(mut s)) => {
            // Daemon cgroup can block NVML (nvidia-caps) even when the
            // GPU is awake — read nvidia-smi from this user process.
            // Same fallback as the GUI app (settings/overview.rs).
            if s.dgpu_temp < 0.0 {
                let local = legion_core::dgpu::read_metrics_batch();
                if let Some(t) = local.temp.filter(|t| *t > 0.0) {
                    s.dgpu_temp = t;
                    if s.dgpu_power < 0.0 {
                        s.dgpu_power = local.power.unwrap_or(s.dgpu_power);
                    }
                    if s.dgpu_clock < 0.0 {
                        s.dgpu_clock = local.clock.unwrap_or(s.dgpu_clock);
                    }
                }
            }
            let cpu_power = match send_command(DaemonCommand::GetCpuPower) {
                Ok(DaemonResponse::CpuPower(w)) if w > 0.5 => Some(w),
                _ => None,
            };
            println!("┌─ Legion Sensors ─────────────────────────────────────┐");
            println!("│  Profile   {:<42} │", friendly_profile(&s.profile));
            log::trace!("sensors: profile '{}'", friendly_profile(&s.profile));
            println!("├─ CPU ────────────────────────────────────────────────┤");
            println!(
                "│  CPU {:>5.1}°C   CCD1 {:>5.1}°C   CCD2 {:>5.1}°C        │",
                s.cpu_temp, s.cpu_temp_1, s.cpu_temp_2
            );
            log::trace!(
                "sensors: cpu {:.1}°C ccd1 {:.1}°C ccd2 {:.1}°C",
                s.cpu_temp,
                s.cpu_temp_1,
                s.cpu_temp_2
            );
            let ec_cpu = if s.ec_cpu < 1.0 {
                "    —".to_string()
            } else {
                format!("{:>5.1}°C", s.ec_cpu)
            };
            println!("│  EC   {ec_cpu}                                        │");
            log::trace!("sensors: ec cpu {:.1}°C", s.ec_cpu);
            if let Some(w) = cpu_power {
                println!(
                    "│  CPU power {:>5.1} W                                  │",
                    w
                );
                log::trace!("sensors: cpu power {w:.1} W");
            }
            println!("├─ GPU ────────────────────────────────────────────────┤");
            println!(
                "│  iGPU {:>5.1}°C  {:>5.2} W                               │",
                s.igpu_edge, s.igpu_power
            );
            log::trace!("sensors: igpu {:.1}°C {:.2} W", s.igpu_edge, s.igpu_power);
            let dgpu_temp = if s.dgpu_temp < 0.0 {
                "    —".to_string()
            } else {
                format!("{:>5.1}°C", s.dgpu_temp)
            };
            let dgpu_power = if s.dgpu_power < 0.0 {
                "    —".to_string()
            } else {
                format!("{:>5.1} W", s.dgpu_power)
            };
            let dgpu_clock = if s.dgpu_clock < 0.0 {
                "    —".to_string()
            } else {
                format!("{:>5.0} MHz", s.dgpu_clock)
            };
            println!("│  dGPU {dgpu_temp}  {dgpu_power}  {dgpu_clock}                   │");
            log::trace!(
                "sensors: dgpu {:.1}°C {:.1} W {} MHz",
                s.dgpu_temp,
                s.dgpu_power,
                s.dgpu_clock
            );
            let ec_gpu = if s.ec_gpu < 1.0 {
                "    —".to_string()
            } else {
                format!("{:>5.1}°C", s.ec_gpu)
            };
            println!("│  EC   {ec_gpu}                                        │");
            log::trace!("sensors: ec gpu {:.1}°C", s.ec_gpu);
            println!("├─ Fans ───────────────────────────────────────────────┤");
            println!(
                "│  CPU {:>5}   GPU {:>5}   Aux {:>5} rpm             │",
                s.fan1_rpm, s.fan2_rpm, s.fan4_rpm
            );
            if fans::rpm_estimated() && s.fan1_rpm.max(s.fan2_rpm).max(s.fan4_rpm) > 0 {
                println!("│  (estimated: unverified EC fallback layout)       │");
            }
            log::trace!(
                "sensors: fans cpu {} gpu {} aux {} rpm",
                s.fan1_rpm,
                s.fan2_rpm,
                s.fan4_rpm
            );
            println!("├─ Storage / Memory / Net ─────────────────────────────┤");
            for (i, t) in s.ssd_composite.iter().enumerate() {
                println!("│  SSD{i}  {t:>5.1}°C                                      │");
                log::trace!("sensors: ssd{i} {t:.1}°C");
            }
            for (i, t) in s.ram_temps.iter().enumerate() {
                println!("│  RAM{i}  {t:>5.1}°C                                      │");
                log::trace!("sensors: ram{i} {t:.1}°C");
            }
            if s.wifi_temp > 0.0 {
                println!(
                    "│  Wi‑Fi {:>5.1}°C                                      │",
                    s.wifi_temp
                );
                log::trace!("sensors: wifi {:.1}°C", s.wifi_temp);
            }
            if s.ethernet_temp > 0.0 {
                println!(
                    "│  Eth   {:>5.1}°C                                      │",
                    s.ethernet_temp
                );
                log::trace!("sensors: ethernet {:.1}°C", s.ethernet_temp);
            }
            println!("├─ Battery ────────────────────────────────────────────┤");
            println!(
                "│  {:>3}%  {:<12}  {:<22} │",
                s.battery_pct, s.battery_status, s.charge_type
            );
            log::trace!(
                "sensors: battery {}% status '{}' type '{}'",
                s.battery_pct,
                s.battery_status,
                s.charge_type
            );
            println!("└──────────────────────────────────────────────────────┘");
        }
        Ok(DaemonResponse::Error(e)) => eprintln!("error: {e}"),
        Err(e) => eprintln!("error: {e}"),
        _ => eprintln!("error: unexpected response type"),
    }
}

/// Dispatch descriptor for the event log — one entry per `Commands` match
/// arm naming the subcommand plus its key parameters. CLI args are all
/// user-supplied and non-secret, so they are safe to log verbatim.
fn cmd_label(cmd: &Commands) -> String {
    match cmd {
        Commands::Status => "status".into(),
        Commands::Watch => "watch".into(),
        Commands::Profile => "profile".into(),
        Commands::SetProfile { name } => format!("set-profile name={name}"),
        Commands::Fan => "fan".into(),
        Commands::SetFan { fan, rpm } => format!("set-fan fan={fan} rpm={rpm}"),
        Commands::FanAuto => "fan-auto".into(),
        Commands::Kbd => "kbd".into(),
        Commands::SetKbd { level } => format!("set-kbd level={level}"),
        Commands::Rgb { r, g, b } => format!("rgb #{r:02X}{g:02X}{b:02X}"),
        Commands::Effect {
            name,
            r,
            g,
            b,
            speed,
            zone,
        } => format!("effect name={name} rgb=#{r:02X}{g:02X}{b:02X} speed={speed} zone={zone}"),
        Commands::Brightness { level } => format!("brightness level={level}"),
        Commands::Zones { action } => format!("zones {action:?}"),
        Commands::Battery => "battery".into(),
        Commands::ChargeLimit { pct } => format!("charge-limit pct={pct}"),
        Commands::Conservation { state } => format!("conservation state={state}"),
        Commands::Info => "info".into(),
        Commands::Camera => "camera".into(),
        Commands::Audio => "audio".into(),
        Commands::AudioFix => "audio-fix".into(),
        Commands::Smt => "smt".into(),
        Commands::SetSmt { state } => format!("set-smt state={state}"),
        Commands::Boost => "boost".into(),
        Commands::SetBoost { state } => format!("set-boost state={state}"),
        Commands::RgbStatus => "rgb-status".into(),
        Commands::RgbFix => "rgb-fix".into(),
        Commands::Logo => "logo".into(),
        Commands::SetLogo { state } => format!("set-logo state={state}"),
        Commands::FnLock { state } => format!("fn-lock state={state:?}"),
        Commands::UsbCharging { state } => format!("usb-charging state={state:?}"),
        Commands::Logs { n } => format!("logs n={n}"),
        Commands::SetLogLevel { level } => format!("set-log-level level={level}"),
        Commands::Undervolt => "undervolt".into(),
        Commands::SetUndervolt {
            offset,
            i_understand_instability_risk,
        } => format!("set-undervolt offset={offset} ack={i_understand_instability_risk}"),
        Commands::ResetUndervolt {
            i_understand_instability_risk,
        } => format!("reset-undervolt ack={i_understand_instability_risk}"),
        Commands::FanCurve { action } => match action {
            None | Some(FanCurveCmd::Show) => "fan-curve show".into(),
            Some(FanCurveCmd::Set { profile, .. }) => format!("fan-curve set {profile}"),
            Some(FanCurveCmd::Enable { profile }) => format!("fan-curve enable {profile}"),
            Some(FanCurveCmd::Disable { profile }) => format!("fan-curve disable {profile}"),
        },
        Commands::Thermal { command } => format!("thermal {}", thermal_cmd_label(command)),
        Commands::Diagnose { action } => format!("diagnose {}", diag_action_label(action)),
        Commands::GpuMode {
            mode,
            yes_i_understand,
        } => format!("gpu-mode mode={mode:?} ack={yes_i_understand}"),
        Commands::CheckUpdate {
            apply,
            allow_downgrade,
        } => format!("check-update apply={apply} allow_downgrade={allow_downgrade}"),
    }
}

fn fan_curve_status() -> legion_core::fan_curve::FanCurveStatus {
    match send_command(DaemonCommand::GetFanCurves) {
        Ok(DaemonResponse::FanCurves(s)) => s,
        Ok(DaemonResponse::Error(e)) | Err(e) => fail(e),
        _ => fail("unexpected response from service"),
    }
}

fn print_fan_curves(st: &legion_core::fan_curve::FanCurveStatus) {
    println!("profile now : {}", st.current_profile);
    let live = if st.active {
        let temp = st.temp_c.map_or("?".into(), |t| format!("{t:.1} °C"));
        let pct = st.pct.map_or("?".into(), |p| format!("{p} %"));
        format!(
            "curve active: {temp} → {pct}{}",
            if st.safety { " (SAFETY: all fans max)" } else { "" }
        )
    } else if let Some(f) = &st.fault {
        format!("firmware control: fault: {f}")
    } else {
        "firmware control".into()
    };
    println!("engine      : {live}");
    if st.curves.is_empty() {
        println!("no curves stored (factory shapes are used when you `enable` a profile)");
    }
    for (name, c) in &st.curves {
        let pts: Vec<String> = c.points.iter().map(|(t, p)| format!("{t}:{p}")).collect();
        println!(
            "{name:<22} {}  {}",
            if c.enabled { "enabled " } else { "disabled" },
            pts.join(",")
        );
    }
}

fn fan_curve_cmd(action: FanCurveCmd) {
    use legion_core::fan_curve as fc;
    let send = |profile: String, points: fc::CurvePoints, enabled: bool| {
        match send_command(DaemonCommand::SetFanCurve {
            profile: profile.clone(),
            points,
            enabled,
        }) {
            Ok(DaemonResponse::FanCurves(st)) => {
                println!(
                    "fan curve for '{profile}' {}",
                    if enabled { "enabled" } else { "disabled (firmware control)" }
                );
                print_fan_curves(&st);
            }
            Ok(DaemonResponse::Error(e)) | Err(e) => fail(e),
            _ => fail("unexpected response from service"),
        }
    };
    let stored = |profile: &str| {
        fan_curve_status()
            .curves
            .into_iter()
            .find(|(n, _)| n == profile)
            .map_or_else(|| fc::default_points(profile), |(_, c)| c.points)
    };
    match action {
        FanCurveCmd::Show => print_fan_curves(&fan_curve_status()),
        FanCurveCmd::Set {
            profile,
            points,
            keep_disabled,
        } => {
            if let Err(e) = fc::validate_profile_name(&profile) {
                usage_fail(e);
            }
            match fc::parse_points(&points) {
                Ok(p) => send(profile, p, !keep_disabled),
                Err(e) => usage_fail(e),
            }
        }
        FanCurveCmd::Enable { profile } => {
            if let Err(e) = fc::validate_profile_name(&profile) {
                usage_fail(e);
            }
            let p = stored(&profile);
            send(profile, p, true);
        }
        FanCurveCmd::Disable { profile } => {
            if let Err(e) = fc::validate_profile_name(&profile) {
                usage_fail(e);
            }
            let p = stored(&profile);
            send(profile, p, false);
        }
    }
}

/// `legion-cli zones …` — 4-zone RGB (experimental). Writes go straight to
/// the hidraw node (udev uaccess), like the Spectrum commands.
fn zones_cmd(action: ZonesCmd) {
    use legion_core::fourzone::{self, FzEffect, WaveDir};
    let check_bri = |b: u8| {
        if !(1..=2).contains(&b) {
            usage_fail(format!(
                "brightness '{b}' out of range (1-2; use `zones off` for off)"
            ));
        }
        b
    };
    let mut state = legion_core::config::get().four_zone.to_state();
    let summary = match action {
        ZonesCmd::Status => {
            match fourzone::find_hidraw() {
                Some(p) => println!("controller   {} (4-zone, 33-byte report 0xCC)", p.display()),
                None => {
                    println!("controller   not found (no 048d hidraw with a 4-zone descriptor)")
                }
            }
            let c = legion_core::config::get().four_zone;
            let cols: Vec<String> = c
                .colors
                .iter()
                .map(|[r, g, b]| format!("#{r:02X}{g:02X}{b:02X}"))
                .collect();
            println!(
                "saved        effect={} speed={} brightness={} dir={} colours={}",
                c.effect,
                c.speed,
                c.brightness,
                c.direction,
                cols.join(" ")
            );
            println!(
                "report       {:02X?}",
                fourzone::build_report(&c.to_state())
            );
            return;
        }
        ZonesCmd::Static {
            c1,
            c2,
            c3,
            c4,
            brightness,
        } => {
            for (i, c) in [c1, c2, c3, c4].iter().enumerate() {
                state.colors[i] = fourzone::parse_color(c).unwrap_or_else(|| {
                    usage_fail(format!("zone {} colour '{c}' is not #RRGGBB", i + 1))
                });
            }
            state.effect = FzEffect::Static;
            state.brightness = check_bri(brightness);
            "static".to_string()
        }
        ZonesCmd::Effect {
            name,
            speed,
            dir,
            brightness,
        } => {
            let fx = match FzEffect::from_name(&name) {
                Some(fx) if fx != FzEffect::Static => fx,
                _ => usage_fail(format!(
                    "unknown effect '{name}'. Try: breath, wave, smooth (or `zones static …`)"
                )),
            };
            if !(1..=4).contains(&speed) {
                usage_fail(format!("speed '{speed}' out of range (1-4)"));
            }
            state.effect = fx;
            state.speed = speed;
            state.direction = WaveDir::from_name(&dir)
                .unwrap_or_else(|| usage_fail(format!("unknown direction '{dir}' (left|right)")));
            state.brightness = check_bri(brightness);
            fx.name().to_string()
        }
        ZonesCmd::Off => {
            state.brightness = 0;
            "off".to_string()
        }
    };
    match fourzone::apply_and_save(&state) {
        Ok(()) => println!("zones → {summary} (experimental 4-zone)"),
        Err(e) => fail(e),
    }
}

/// Dispatch descriptor for `thermal` subcommands.
fn thermal_cmd_label(cmd: &ThermalCmd) -> String {
    match cmd {
        ThermalCmd::Status => "status".into(),
        ThermalCmd::Set {
            max_temp,
            off,
            on,
            acknowledge_high_temp,
        } => {
            format!("set max_temp={max_temp:?} off={off} on={on} ack_high={acknowledge_high_temp}")
        }
    }
}

/// Dispatch descriptor for `diagnose` subactions.
fn diag_action_label(action: &DiagAction) -> String {
    match action {
        DiagAction::Dump { .. } => "dump".into(),
        DiagAction::Selfcheck => "selfcheck".into(),
        DiagAction::Faults => "faults".into(),
        DiagAction::Enable => "enable".into(),
        DiagAction::Disable => "disable".into(),
        DiagAction::Send { endpoint } => format!(
            "send override={}",
            if endpoint.is_some() { "set" } else { "unset" }
        ),
    }
}
