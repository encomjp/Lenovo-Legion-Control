//! Legion CLI — command-line control for Lenovo Legion laptops.

use clap::{Parser, Subcommand};
use legion_core::{
    comms::{send_command, DaemonCommand, DaemonResponse},
    diagnostics, selftest,
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
        /// Fan number: 1 (CPU), 2 (GPU), or 4 (Aux)
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
    /// Thermal throttle control
    Thermal {
        #[command(subcommand)]
        command: ThermalCmd,
    },
    /// Anonymous diagnostics (alpha) — off unless enabled in Settings.
    Diagnose {
        #[command(subcommand)]
        action: DiagAction,
    },
    /// Check GitHub for new releases of Legion Control
    CheckUpdate {
        /// Download and install the matching release (AppImage, deb, rpm, Arch, tarball)
        #[arg(long)]
        apply: bool,
    },
}

#[derive(Debug, Clone, Subcommand)]
enum DiagAction {
    /// Print the full ANONYMOUS diagnostics JSON (no upload)
    Dump,
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
                println!("Legion Control — live sensors (Ctrl+C to quit)\n");
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
            for (fan, name) in [(1u8, "CPU"), (2, "GPU"), (4, "Aux")] {
                match send_command(DaemonCommand::GetFanRpm(fan)) {
                    Ok(DaemonResponse::FanRpm(r)) => println!("{name:>3} fan: {r:>5} RPM"),
                    Ok(DaemonResponse::Error(e)) => eprintln!("{name} fan: {e}"),
                    Err(e) => eprintln!("{name} fan: {e}"),
                    _ => {}
                }
            }
        }
        Commands::SetFan { fan, rpm } => {
            if !matches!(fan, 1 | 2 | 4) {
                usage_fail(format!(
                    "invalid fan index '{fan}'. Valid fan channels: 1 (CPU), 2 (GPU), 4 (Aux)"
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
            }
        }
        Commands::FanAuto => {
            let mut errors: Vec<String> = Vec::new();
            for fan in [1u8, 2, 4] {
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
                    Ok(()) => println!("effect → off · {}", z.name()),
                    Err(e) => fail(e),
                }
            } else if let Some(fx) = legion_core::keyboard::RgbEffect::from_name(&name) {
                log::debug!("effect: '{name}' matched");
                match legion_core::keyboard::set_rgb_effect_zone(fx, r, g, b, speed, 9, z) {
                    Ok(()) => println!("effect → {name} · {}", z.name()),
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
                        println!("daemon      v{dver} (MISMATCH: CLI is v{cli_ver} — restart daemon!)");
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
                        "fan{:<8} {} · {}–{} RPM · now {}",
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
            if d.health == legion_core::audio::Health::HardwareBroken {
                std::process::exit(2);
            }
            if d.health == legion_core::audio::Health::SoftIssue {
                std::process::exit(1);
            }
        }
        Commands::AudioFix => {
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
            };
            println!("status      {tag}");
            println!("summary     {}", d.summary);
            if d.health == legion_core::audio::Health::HardwareBroken {
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
                    "now         {} · {logical_cpus} logical CPUs",
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
                    eprintln!("note: daemon probe unavailable ({e}) — local check");
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
            DiagAction::Dump => {
                let json = serde_json::to_string_pretty(&diagnostics::collect())
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
                    println!("{mark} {} — {}", c.name, c.detail);
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
                    println!("[{sev}] {} — {}", f.id, f.detail);
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
        Commands::CheckUpdate { apply } => match legion_core::update::check_latest_release() {
            Ok(info) => {
                println!("current version: {}", legion_core::update::CURRENT_VERSION);
                println!("latest release:  {} ({})", info.version, info.name);
                if let Some(asset) = legion_core::update::selected_asset(&info) {
                    println!("matching asset:   {} ({} bytes)", asset.name, asset.size);
                } else if let Some(asset) = &info.appimage {
                    println!("appimage:        {} ({} bytes)", asset.name, asset.size);
                }
                if info.is_newer {
                    println!(
                        "\nA new version of Legion Control is available: v{}",
                        info.version
                    );
                    if apply {
                        match legion_core::update::apply_update(&info, |phase, bytes, total| {
                            let label = match phase {
                                legion_core::update::UpdatePhase::Downloading => "downloading",
                                legion_core::update::UpdatePhase::Verifying => "verifying",
                                legion_core::update::UpdatePhase::Building
                                | legion_core::update::UpdatePhase::BuildingLog(_) => "building",
                                legion_core::update::UpdatePhase::Installing => "installing",
                            };
                            if let Some(t) = total.filter(|t| *t > 0) {
                                eprint!("\r{label}: {bytes}/{t} bytes");
                            } else {
                                eprint!("\r{label}…");
                            }
                            let _ = std::io::Write::flush(&mut std::io::stderr());
                        }) {
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
                    println!("\nAlready up to date — nothing to apply.");
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
    println!("status      {tag} (local — restart legion-control for full USB autofix)");
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
        "Thermal: {} · max {}°C (restore {}°C) · cur {} kHz · CPU {} / CPU CCD 2 {} · {}",
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
        Commands::Thermal { command } => format!("thermal {}", thermal_cmd_label(command)),
        Commands::Diagnose { action } => format!("diagnose {}", diag_action_label(action)),
        Commands::CheckUpdate { apply } => format!("check-update apply={apply}"),
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
        DiagAction::Dump => "dump".into(),
        DiagAction::Selfcheck => "selfcheck".into(),
        DiagAction::Faults => "faults".into(),
        DiagAction::Send { endpoint } => format!(
            "send override={}",
            if endpoint.is_some() { "set" } else { "unset" }
        ),
    }
}
