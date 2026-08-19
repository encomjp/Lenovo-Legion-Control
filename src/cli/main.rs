//! Legion CLI — command-line control for Lenovo Legion laptops.

use clap::{Parser, Subcommand};
use legion_core::comms::{send_command, DaemonCommand, DaemonResponse};

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

    match cli.command {
        Commands::Status => {
            let resp = send_command(DaemonCommand::GetSensors);
            print_sensors(resp);
        }
        Commands::Watch => loop {
            print!("\x1B[2J\x1B[H");
            println!("Legion Control — live sensors (Ctrl+C to quit)\n");
            let resp = send_command(DaemonCommand::GetSensors);
            print_sensors(resp);
            std::thread::sleep(std::time::Duration::from_secs(2));
        },
        Commands::Profile => match send_command(DaemonCommand::GetProfile) {
            Ok(DaemonResponse::Profile(p)) => println!("{}", friendly_profile(&p)),
            Ok(DaemonResponse::Error(e)) => eprintln!("error: {e}"),
            Err(e) => eprintln!("error: {e}"),
            _ => eprintln!("error: unexpected response"),
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
            match send_command(DaemonCommand::SetProfile(name.clone())) {
                Ok(DaemonResponse::Ok) => println!("profile → {}", friendly_profile(&name)),
                Ok(DaemonResponse::Error(e)) => eprintln!("error: {e}"),
                Err(e) => eprintln!("error: {e}"),
                _ => eprintln!("error: unexpected response"),
            }
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
            match send_command(DaemonCommand::SetFanTarget(fan, rpm)) {
                Ok(DaemonResponse::Ok) => {
                    if rpm == 0 {
                        println!("fan {fan} → auto");
                    } else {
                        println!("fan {fan} → {rpm} RPM");
                    }
                }
                Ok(DaemonResponse::Error(e)) => eprintln!("error: {e}"),
                Err(e) => eprintln!("error: {e}"),
                _ => eprintln!("error: unexpected response"),
            }
        }
        Commands::FanAuto => {
            for fan in [1u8, 2, 4] {
                send_command(DaemonCommand::SetFanTarget(fan, 0)).ok();
            }
            println!("all fans → auto");
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
            Ok(DaemonResponse::Error(e)) => eprintln!("error: {e}"),
            Err(e) => eprintln!("error: {e}"),
            _ => eprintln!("error: cannot read keyboard brightness"),
        },
        Commands::SetKbd { level } => match send_command(DaemonCommand::SetKbdBrightness(level)) {
            Ok(DaemonResponse::Ok) => println!("backlight → {level}"),
            Ok(DaemonResponse::Error(e)) => eprintln!("error: {e}"),
            Err(e) => eprintln!("error: {e}"),
            _ => eprintln!("error: unexpected response"),
        },
        Commands::Rgb { r, g, b } => match legion_core::keyboard::set_rgb_static(r, g, b) {
            Ok(()) => println!("rgb → #{r:02X}{g:02X}{b:02X}"),
            Err(e) => eprintln!("error: {e}"),
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
                    Err(e) => eprintln!("error: {e}"),
                }
            } else if let Some(fx) = legion_core::keyboard::RgbEffect::from_name(&name) {
                match legion_core::keyboard::set_rgb_effect_zone(fx, r, g, b, speed, 9, z) {
                    Ok(()) => println!("effect → {name} · {}", z.name()),
                    Err(e) => eprintln!("error: {e}"),
                }
            } else {
                eprintln!(
                    "error: unknown effect. Try: {}",
                    legion_core::keyboard::RgbEffect::all_names().join(", ")
                );
            }
        }
        Commands::Brightness { level } => match legion_core::keyboard::set_rgb_brightness(level) {
            Ok(()) => println!("brightness → {level}/9"),
            Err(e) => eprintln!("error: {e}"),
        },
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
            Ok(DaemonResponse::Error(e)) => eprintln!("error: {e}"),
            Err(e) => eprintln!("error: {e}"),
            _ => eprintln!("error: cannot read battery"),
        },
        Commands::ChargeLimit { pct } => match send_command(DaemonCommand::SetChargeLimit(pct)) {
            Ok(DaemonResponse::Ok) => {
                println!(
                    "charge limit → {}%",
                    match pct {
                        0..=69 => 60,
                        70..=89 => 80,
                        _ => 100,
                    }
                )
            }
            Ok(DaemonResponse::Error(e)) => eprintln!("error: {e}"),
            Err(e) => eprintln!("error: {e}"),
            _ => eprintln!("error: unexpected response"),
        },
        Commands::Conservation { state } => {
            let on = matches!(state.to_lowercase().as_str(), "on" | "true" | "1");
            match send_command(DaemonCommand::SetConservation(on)) {
                Ok(DaemonResponse::Ok) => {
                    println!(
                        "conservation → {}",
                        if on { "on (~60%)" } else { "off (100%)" }
                    )
                }
                Ok(DaemonResponse::Error(e)) => eprintln!("error: {e}"),
                Err(e) => eprintln!("error: {e}"),
                _ => eprintln!("error: unexpected response"),
            }
        }
        Commands::Info => match send_command(DaemonCommand::GetDeviceInfo) {
            Ok(DaemonResponse::DeviceInfo(info)) => {
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
            Ok(DaemonResponse::Error(e)) => eprintln!("error: {e}"),
            Err(e) => eprintln!("error: {e}"),
            _ => eprintln!("error: cannot get device info"),
        },
        Commands::Camera => match send_command(DaemonCommand::GetCameraPower) {
            Ok(DaemonResponse::CameraPower(killed)) => {
                println!(
                    "camera      {}",
                    if killed { "privacy kill active" } else { "on" }
                );
            }
            Ok(DaemonResponse::Error(e)) => eprintln!("error: {e}"),
            Err(e) => eprintln!("error: {e}"),
            _ => eprintln!("error: cannot read camera power"),
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
            Ok(DaemonResponse::Error(e)) => eprintln!("error: {e}"),
            Err(e) => eprintln!("error: {e}"),
            _ => eprintln!("error: unexpected response — update legion-daemon"),
        },
        Commands::SetSmt { state } => {
            let on = match state.to_lowercase().as_str() {
                "on" | "1" | "true" | "enable" => true,
                "off" | "0" | "false" | "disable" => false,
                _ => {
                    eprintln!("error: use on or off");
                    return;
                }
            };
            if !on {
                let n = legion_core::cpu::logical_cpus().max(2);
                let half = (n / 2).max(1);
                eprintln!(
                    "warning: disabling SMT halves logical CPUs (about {n}→{half}). \
                     Helps some latency-sensitive games; hurts multi-threaded loads."
                );
            }
            match send_command(DaemonCommand::SetSmt(on)) {
                Ok(DaemonResponse::Ok) => {
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
                Ok(DaemonResponse::Error(e)) => eprintln!("error: {e}"),
                Err(e) => eprintln!("error: {e}"),
                _ => eprintln!("error: unexpected response — update legion-daemon"),
            }
        }
        Commands::Boost => match send_command(DaemonCommand::GetBoost) {
            Ok(DaemonResponse::Boost(on)) => {
                println!("boost       {}", if on { "on" } else { "off" });
            }
            Ok(DaemonResponse::Error(e)) => eprintln!("error: {e}"),
            Err(e) => eprintln!("error: {e}"),
            _ => eprintln!("error: unexpected response — update legion-daemon"),
        },
        Commands::SetBoost { state } => {
            let on = match state.to_lowercase().as_str() {
                "on" | "1" | "true" | "enable" => true,
                "off" | "0" | "false" | "disable" => false,
                _ => {
                    eprintln!("error: use on or off");
                    return;
                }
            };
            match send_command(DaemonCommand::SetBoost(on)) {
                Ok(DaemonResponse::Ok) => {
                    println!("boost → {}", if on { "on" } else { "off" })
                }
                Ok(DaemonResponse::Error(e)) => eprintln!("error: {e}"),
                Err(e) => eprintln!("error: {e}"),
                _ => eprintln!("error: unexpected response — update legion-daemon"),
            }
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
            let on = matches!(state.to_lowercase().as_str(), "on" | "true" | "1");
            match send_command(DaemonCommand::SetLogo(on)) {
                Ok(DaemonResponse::Ok) => println!("logo → {}", if on { "on" } else { "off" }),
                Ok(DaemonResponse::Error(e)) => eprintln!("error: {e}"),
                Err(e) => eprintln!("error: {e}"),
                _ => eprintln!("error: unexpected response"),
            }
        }
        Commands::Logs { n } => match send_command(DaemonCommand::GetRecentLogs(n)) {
            Ok(DaemonResponse::RecentLogs(text)) => {
                if text.is_empty() {
                    println!("(no log entries)");
                } else {
                    print!("{text}");
                }
            }
            Ok(DaemonResponse::Error(e)) => eprintln!("error: {e}"),
            Err(e) => eprintln!("error: {e}"),
            _ => eprintln!("error: unexpected response"),
        },
        Commands::SetLogLevel { level } => {
            match send_command(DaemonCommand::SetLogLevel(level.clone())) {
                Ok(DaemonResponse::Ok) => println!("log level → {level}"),
                Ok(DaemonResponse::Error(e)) => eprintln!("error: {e}"),
                Err(e) => eprintln!("error: {e}"),
                _ => eprintln!("error: unexpected response"),
            }
        }
        Commands::Undervolt => match send_command(DaemonCommand::GetCurveOptimizer) {
            Ok(DaemonResponse::CurveOptimizer(status)) => print_curve_optimizer(&status),
            Ok(DaemonResponse::Error(e)) | Err(e) => eprintln!("error: {e}"),
            _ => eprintln!("error: unexpected response"),
        },
        Commands::SetUndervolt {
            offset,
            i_understand_instability_risk,
        } => {
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
                Ok(DaemonResponse::Error(e)) | Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
                _ => {
                    eprintln!("error: unexpected response");
                    std::process::exit(1);
                }
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
                Ok(DaemonResponse::Error(e)) | Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
                _ => {
                    eprintln!("error: unexpected response");
                    std::process::exit(1);
                }
            }
        }
        Commands::Thermal { command } => match command {
            ThermalCmd::Status => match send_command(DaemonCommand::GetThermalStatus) {
                Ok(DaemonResponse::ThermalStatus(s)) => print_thermal_status(&s),
                Ok(DaemonResponse::Error(e)) => eprintln!("error: {e}"),
                Err(e) => eprintln!("error: {e}"),
                _ => eprintln!("error: unexpected response"),
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
                if let Err(e) =
                    legion_core::thermal::validate(effective_max, acknowledge_high_temp)
                {
                    eprintln!("error: {e}");
                    std::process::exit(2);
                }
                match send_command(DaemonCommand::SetThermal {
                    enabled,
                    max_temp: effective_max,
                    acknowledge: acknowledge_high_temp,
                }) {
                    Ok(DaemonResponse::ThermalStatus(s)) => print_thermal_status(&s),
                    Ok(DaemonResponse::Error(e)) => {
                        eprintln!("error: {e}");
                        std::process::exit(1);
                    }
                    Err(e) => {
                        eprintln!("error: {e}");
                        std::process::exit(1);
                    }
                    _ => eprintln!("error: unexpected response"),
                }
            }
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
    if let Some(codename) = status.codename {
        println!("ryzen_smu codename: {codename}");
    }
    if let Some(driver) = &status.driver_version {
        println!("driver: {driver}");
    }
    if let Some(firmware) = &status.firmware_version {
        println!("smu firmware: {firmware}");
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
    }
    println!(
        "allowed temporary range: {}..={}",
        status.minimum, status.maximum
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
    let tctl = s
        .tctl_mC
        .map(|v| format!("{:.1}°C", v as f64 / 1000.0))
        .unwrap_or_else(|| "n/a".into());
    let tccd2 = s
        .tccd2_mC
        .map(|v| format!("{:.1}°C", v as f64 / 1000.0))
        .unwrap_or_else(|| "n/a".into());
    println!(
        "Thermal: {} · max {}°C (restore {}°C) · cur {} kHz · Tctl {} / Tccd2 {} · {}",
        on_off, s.config.max_temp, s.restore_temp, s.cur_max_freq, tctl, tccd2, state
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
        Ok(DaemonResponse::Sensors(s)) => {
            let cpu_power = match send_command(DaemonCommand::GetCpuPower) {
                Ok(DaemonResponse::CpuPower(w)) if w > 0.5 => Some(w),
                _ => None,
            };
            println!("┌─ Legion Sensors ─────────────────────────────────────┐");
            println!("│  Profile   {:<42} │", friendly_profile(&s.profile));
            println!("├─ CPU ────────────────────────────────────────────────┤");
            println!(
                "│  Tctl {:>5.1}°C   CCD1 {:>5.1}°C   CCD2 {:>5.1}°C       │",
                s.cpu_tctl, s.cpu_ccd1, s.cpu_ccd2
            );
            println!(
                "│  EC   {:>5.1}°C                                        │",
                s.ec_cpu
            );
            if let Some(w) = cpu_power {
                println!(
                    "│  CPU power {:>5.1} W                                  │",
                    w
                );
            }
            println!("├─ GPU ────────────────────────────────────────────────┤");
            println!(
                "│  iGPU {:>5.1}°C  {:>5.2} W                               │",
                s.igpu_edge, s.igpu_power
            );
            println!(
                "│  dGPU {:>5.1}°C  {:>5.1} W  {:>5.0} MHz                   │",
                s.dgpu_temp, s.dgpu_power, s.dgpu_clock
            );
            println!(
                "│  EC   {:>5.1}°C                                        │",
                s.ec_gpu
            );
            println!("├─ Fans ───────────────────────────────────────────────┤");
            println!(
                "│  CPU {:>5}   GPU {:>5}   Aux {:>5} rpm             │",
                s.fan1_rpm, s.fan2_rpm, s.fan4_rpm
            );
            println!("├─ Storage / Memory / Net ─────────────────────────────┤");
            for (i, t) in s.ssd_composite.iter().enumerate() {
                println!("│  SSD{i}  {t:>5.1}°C                                      │");
            }
            for (i, t) in s.ram_temps.iter().enumerate() {
                println!("│  RAM{i}  {t:>5.1}°C                                      │");
            }
            if s.wifi_temp > 0.0 {
                println!(
                    "│  Wi‑Fi {:>5.1}°C                                      │",
                    s.wifi_temp
                );
            }
            if s.ethernet_temp > 0.0 {
                println!(
                    "│  Eth   {:>5.1}°C                                      │",
                    s.ethernet_temp
                );
            }
            println!("├─ Battery ────────────────────────────────────────────┤");
            println!(
                "│  {:>3}%  {:<12}  {:<22} │",
                s.battery_pct, s.battery_status, s.charge_type
            );
            println!("└──────────────────────────────────────────────────────┘");
        }
        Ok(DaemonResponse::Error(e)) => eprintln!("error: {e}"),
        Err(e) => eprintln!("error: {e}"),
        _ => eprintln!("error: unexpected response type"),
    }
}
