//! Fix page — speakers, Spectrum RGB recovery, udev rule, daemon logs.

use super::*;

pub(crate) fn udev_rule_installed() -> bool {
    for path in [
        "/etc/udev/rules.d/99-legion.rules",
        "/usr/lib/udev/rules.d/99-legion.rules",
        "/usr/local/lib/udev/rules.d/99-legion.rules",
    ] {
        if let Ok(content) = std::fs::read_to_string(path) {
            let lower = content.to_lowercase();
            if lower.contains("048d") && lower.contains("c197") && lower.contains("0660") {
                return true;
            }
        }
    }
    false
}

pub(crate) fn build_udev_permanent_section(
    toast_overlay: &adw::ToastOverlay,
) -> adw::PreferencesGroup {
    let group = pref_group("Permanent fix", None);

    let installed = udev_rule_installed();
    let (pill_text, pill_kind) = if installed {
        ("Installed", "ok")
    } else {
        ("Missing", "warn")
    };
    let pill = status_pill_tip(
        pill_text,
        pill_kind,
        Some(if installed {
            "udev rule 99-legion.rules is present and looks correct"
        } else {
            "Rule file missing, Auto-fix only lasts until reboot; install permanently below"
        }),
    );

    let status_row = adw::ActionRow::builder()
        .title("Keyboard access rule")
        .subtitle(if installed { "Present" } else { "Missing" })
        .activatable(false)
        .build();
    tip(
        &status_row,
        "Checks /etc/udev/rules.d/99-legion.rules (and /usr/lib/udev) for 048d:c197 with MODE 0660",
    );
    status_row.add_suffix(&pill);
    group.add(&status_row);

    let btn = secondary_button_tip(
        if installed {
            "Reinstall permanently"
        } else {
            "Install permanently"
        },
        Some("Writes /etc/udev/rules.d/99-legion.rules and reloads udev (needs admin password via PolicyKit)"),
    );
    let action = adw::ActionRow::builder()
        .title("Make fix permanent")
        .activatable(false)
        .build();
    tip(
        &action,
        "Runs legion-control-setup install-udev through pkexec; then re-checks the rule",
    );
    action.add_suffix(&btn);
    group.add(&action);

    let overlay = toast_overlay.clone();
    let pill_c = pill.clone();
    let status_c = status_row.clone();
    let btn_c = btn.clone();
    btn.connect_clicked(move |_| {
        btn_c.set_sensitive(false);
        btn_c.set_label("Installing…");
        let overlay = overlay.clone();
        let pill_c = pill_c.clone();
        let status_c = status_c.clone();
        let btn_c = btn_c.clone();
        run_setup_helper("install-udev", move |result| match result {
            Ok(msg) => {
                // Verify on disk after helper reports success
                let now_installed = udev_rule_installed();
                if now_installed {
                    set_pill(&pill_c, "Installed", "ok");
                    tip(
                        &pill_c,
                        "udev rule 99-legion.rules is present and looks correct",
                    );
                    status_c.set_subtitle("Present");
                    btn_c.set_label("Reinstall permanently");
                    let detail = if msg.is_empty() {
                        "Udev rule installed permanently".to_string()
                    } else {
                        msg
                    };
                    toast_ok(&overlay, &detail);
                } else {
                    set_pill(&pill_c, "Missing", "warn");
                    toast_error(
                        &overlay,
                        "Helper finished but rule still not found, check /etc/udev/rules.d/99-legion.rules",
                    );
                    btn_c.set_label("Install permanently");
                }
                btn_c.set_sensitive(true);
            }
            Err(error) => {
                toast_error(&overlay, &error);
                btn_c.set_label("Install permanently");
                btn_c.set_sensitive(true);
            }
        });
    });

    group
}

/// Compact Fix for embedding inside Settings — one scrollable page with
/// the diagnostics as stacked cards, no extra ViewSwitcher.
pub(crate) fn build_fix_compact(toast_overlay: &adw::ToastOverlay, gate: &DaemonGate) -> gtk::Box {
    let page = page_lede("");
    if let Some(group) = build_recommended_section(toast_overlay) {
        page.append(&group);
    }
    page.append(&build_selfcheck_section(toast_overlay));
    page.append(&build_speakers_section(toast_overlay));
    page.append(&build_lighting_reset_section(toast_overlay, gate));
    // Rarely needed tools stay folded away.
    let advanced_group = pref_group("", None);
    let advanced = adw::ExpanderRow::builder().title("Advanced").build();
    let inner = gtk::Box::new(Orientation::Vertical, 12);
    inner.set_margin_top(8);
    inner.append(&build_udev_permanent_section(toast_overlay));
    inner.append(&build_logs_section(toast_overlay, gate));
    advanced.add_row(&inner);
    advanced_group.add(&advanced);
    page.append(&advanced_group);
    page
}

/// One optional component this laptop would benefit from.
struct Recommendation {
    title: &'static str,
    subtitle: String,
    tooltip: String,
    /// Fixed PolicyKit setup operation behind the Install button.
    operation: &'static str,
    done_toast: &'static str,
    /// Install is not possible yet (missing build tools); the subtitle says why.
    blocked: bool,
}

/// Optional components that make sense for this machine and are missing.
fn missing_components() -> Vec<Recommendation> {
    let mut out = Vec::new();
    let caps = legion_core::device::detect().capabilities;

    // AMD tuning driver: only on the machine Curve Optimizer is validated on.
    if legion_core::undervolt::machine_validated()
        && !std::path::Path::new("/sys/kernel/ryzen_smu_drv").is_dir()
    {
        let missing = dkms_prereqs_missing();
        out.push(Recommendation {
            title: "AMD tuning driver",
            subtitle: match &missing {
                None => "Unlocks undervolting on the Performance page".to_string(),
                Some(cmd) => format!("Needs build tools first: {cmd}"),
            },
            tooltip: "Builds the bundled, pinned ryzen_smu driver through DKMS using PolicyKit"
                .into(),
            operation: "install-ryzen-smu",
            done_toast: "AMD tuning driver installed",
            blocked: missing.is_some(),
        });
    }

    // Background service: an update that only replaced the binaries left the
    // service's device sandbox behind (e.g. no NVIDIA access).
    if legion_core::service_unit::sandbox_outdated() {
        out.push(Recommendation {
            title: "Update the background service",
            subtitle: "Lets it read the graphics card temperature again".into(),
            tooltip: "Adds the device access this version needs to legion-control.service \
                      through a systemd drop-in, then restarts the service (admin password \
                      via PolicyKit)"
                .into(),
            operation: "refresh-service",
            done_toast: "Background service updated",
            blocked: false,
        });
    }

    // Speaker firmware: the woofer amp is present but has no firmware file.
    if legion_core::audio::aw88399_acpi_present() && !legion_core::audio::aw88399_firmware_present()
    {
        out.push(Recommendation {
            title: "Speaker firmware",
            subtitle: "Full bass from the built-in speakers. Restart afterwards".into(),
            tooltip: "Downloads and installs aw88399_acf.bin (checksum-verified, never \
                      overwrites existing firmware)"
                .into(),
            operation: "install-amp-firmware",
            done_toast: "Speaker firmware installed, restart to use it",
            blocked: false,
        });
    }

    // Keyboard access rule: lets the app drive the RGB keyboard after a reboot.
    if caps.lighting_kind == legion_core::device::LightingKind::Spectrum && !udev_rule_installed() {
        out.push(Recommendation {
            title: "Keyboard access rule",
            subtitle: "Keeps keyboard lighting working after a restart".into(),
            tooltip: "Writes /etc/udev/rules.d/99-legion.rules and reloads udev (admin password \
                      via PolicyKit)"
                .into(),
            operation: "install-udev",
            done_toast: "Keyboard access rule installed",
            blocked: false,
        });
    }
    out
}

/// "Recommended for this laptop": shown only while an optional component that
/// fits this machine is missing. Each row reuses the setup helper operation
/// behind the matching button elsewhere in the app.
pub(crate) fn build_recommended_section(
    toast_overlay: &adw::ToastOverlay,
) -> Option<adw::PreferencesGroup> {
    let items = missing_components();
    if items.is_empty() {
        return None;
    }
    let group = pref_group("Recommended for this laptop", None);
    let remaining = Rc::new(Cell::new(items.len()));
    for item in items {
        let row = adw::ActionRow::builder()
            .title(item.title)
            .subtitle(item.subtitle.as_str())
            .activatable(false)
            .build();
        tip(&row, &item.tooltip);
        let btn = secondary_button_tip("Install", Some(&item.tooltip));
        btn.set_valign(Align::Center);
        btn.set_sensitive(!item.blocked);
        row.add_suffix(&btn);
        group.add(&row);

        let overlay = toast_overlay.clone();
        let group_c = group.clone();
        let row_c = row.clone();
        let remaining = remaining.clone();
        let (operation, done_toast) = (item.operation, item.done_toast);
        btn.connect_clicked(move |btn| {
            btn.set_sensitive(false);
            btn.set_label("Installing…");
            let overlay = overlay.clone();
            let group = group_c.clone();
            let row = row_c.clone();
            let btn = btn.clone();
            let remaining = remaining.clone();
            run_setup_helper(operation, move |result| match result {
                Ok(_) => {
                    group.remove(&row);
                    remaining.set(remaining.get().saturating_sub(1));
                    if remaining.get() == 0 {
                        group.set_visible(false);
                    }
                    toast_ok(&overlay, done_toast);
                }
                Err(error) => {
                    btn.set_label("Install");
                    btn.set_sensitive(true);
                    toast_error(&overlay, &error);
                }
            });
        });
    }
    Some(group)
}

pub(crate) fn build_lighting_reset_section(
    toast_overlay: &adw::ToastOverlay,
    gate: &DaemonGate,
) -> adw::PreferencesGroup {
    use legion_core::rgb_panic::{self, Health};

    let diag0 = rgb_panic::diagnose();
    let group = pref_group(
        if diag0.health == Health::Ok {
            "Keyboard lighting"
        } else {
            "Keyboard lighting issue"
        },
        None,
    );
    let (pill_text, pill_kind) = rgb_pill(diag0.health);
    let pill = status_pill_tip(pill_text, pill_kind, Some(rgb_pill_tooltip(diag0.health)));

    let status_row = adw::ActionRow::builder()
        .title(rgb_title(diag0.health, &diag0.summary))
        .subtitle(rgb_short_help(diag0.health))
        .activatable(false)
        .build();
    tip(
        &status_row,
        "Scans hidraw 048d:c197, ioctl health, brightness vs saved config, and kernel USB/HID errors",
    );
    pill.set_width_request(PILL_COL_W);
    status_row.add_suffix(&pill);
    group.add(&status_row);

    let expander = adw::ExpanderRow::builder()
        .title("Technical details")
        .build();
    tip(
        &expander,
        "Expand for HID path, permissions, ioctl health, and kernel USB/HID log hits",
    );
    let details = gtk::Label::new(Some(&diag0.details.join("\n")));
    details.add_css_class("detail-body");
    details.set_halign(Align::Start);
    details.set_wrap(true);
    details.set_xalign(0.0);
    details.set_selectable(true);
    details.set_margin_start(12);
    details.set_margin_end(12);
    details.set_margin_top(4);
    details.set_margin_bottom(8);
    tip(
        &details,
        "Raw diagnostic lines, selectable so you can copy them into a bug report",
    );
    expander.add_row(&details);
    group.add(&expander);

    let btn = secondary_button_tip(
        rgb_button_label(diag0.health),
        Some(
            "Soft lighting reset → fix permissions → USB reset → hid-generic rebind → restore saved look (daemon)",
        ),
    );
    btn.set_width_request(BTN_COL_W);
    status_row.add_suffix(&btn);
    gate.track(&btn);

    let overlay = toast_overlay.clone();
    let pill_c = pill.clone();
    let status_c = status_row.clone();
    let details_c = details.clone();
    let expander_c = expander.clone();
    let btn_c = btn.clone();
    btn.connect_clicked(move |_| {
        set_busy(&btn_c, true, "Check again");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let report = match send_command(DaemonCommand::FixRgbPanic) {
                Ok(DaemonResponse::RgbFixReport {
                    steps,
                    errors,
                    health,
                    summary,
                }) => {
                    let health = match health.as_str() {
                        "ok" => Health::Ok,
                        "soft-issue" => Health::SoftIssue,
                        "broken" => Health::HardwareBroken,
                        _ => Health::NotApplicable,
                    };
                    Ok((steps, errors, health, summary))
                }
                Ok(DaemonResponse::Error(e)) => Err(e),
                Err(e) if is_version_skew_error(&e) => {
                    let r = rgb_panic::troubleshoot();
                    Ok((r.steps, r.errors, r.after.health, r.after.summary))
                }
                Err(e) => Err(e),
                _ => Err("Unexpected daemon response".into()),
            };
            let _ = tx.send(report);
        });

        let overlay = overlay.clone();
        let pill_c = pill_c.clone();
        let status_c = status_c.clone();
        let details_c = details_c.clone();
        let expander_c = expander_c.clone();
        let btn_c = btn_c.clone();
        glib::timeout_add_local(Duration::from_millis(200), move || match rx.try_recv() {
            Ok(Ok((steps, errors, health, summary))) => {
                let (pt, pk) = rgb_pill(health);
                set_pill(&pill_c, pt, pk);
                tip(&pill_c, rgb_pill_tooltip(health));
                status_c.set_title(&rgb_title(health, &summary));
                status_c.set_subtitle(rgb_short_help(health));
                let mut body = steps.to_vec();
                if !errors.is_empty() {
                    body.push(String::new());
                    body.push("Problems:".into());
                    body.extend(errors.iter().cloned());
                }
                details_c.set_text(&body.join("\n"));
                expander_c.set_expanded(true);
                match health {
                    Health::Ok => toast_ok(&overlay, "Keyboard lighting is working"),
                    Health::SoftIssue => {
                        toast_error(&overlay, "Partially fixed, check Lighting tab")
                    }
                    Health::HardwareBroken => {
                        toast_error(&overlay, "Still broken, try reboot or USB replug")
                    }
                    Health::NotApplicable => toast_ok(&overlay, &summary),
                }
                set_busy(&btn_c, false, rgb_button_label(health));
                glib::ControlFlow::Break
            }
            Ok(Err(e)) => {
                set_pill(&pill_c, "Failed", "bad");
                details_c.set_text(&e);
                expander_c.set_expanded(true);
                toast_error(&overlay, &e);
                set_busy(&btn_c, false, "Check again");
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => {
                set_busy(&btn_c, false, "Check again");
                toast_error(&overlay, "RGB fix failed");
                glib::ControlFlow::Break
            }
        });
    });

    group
}

pub(crate) fn build_logs_section(
    toast_overlay: &adw::ToastOverlay,
    gate: &DaemonGate,
) -> adw::PreferencesGroup {
    use legion_core::comms::{send_command, DaemonCommand, DaemonResponse};

    let group = pref_group("Service logs", None);

    let text_view = gtk::TextView::new();
    text_view.set_editable(false);
    text_view.set_monospace(false);
    text_view.set_wrap_mode(gtk::WrapMode::WordChar);
    text_view.set_top_margin(6);
    text_view.set_bottom_margin(6);
    text_view.set_left_margin(8);
    text_view.set_right_margin(8);
    let buffer = text_view.buffer();
    buffer.set_text("Fetching recent service output…");

    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&text_view));
    scroll.set_vexpand(false);
    scroll.set_min_content_height(120);
    scroll.set_max_content_height(160);
    group.add(&scroll);

    let fetch_btn = secondary_button_tip(
        "Fetch logs",
        Some("Fetch the last 100 log lines from the background service"),
    );
    let copy_btn = secondary_button_tip("Copy", Some("Copy log text to clipboard"));
    let level_btn = secondary_button_tip(
        "Verbose",
        Some("Switch between normal and detailed logging"),
    );

    let btn_row = gtk::Box::new(Orientation::Horizontal, 8);
    btn_row.set_margin_top(4);
    btn_row.set_margin_bottom(8);
    btn_row.append(&fetch_btn);
    btn_row.append(&copy_btn);
    btn_row.append(&level_btn);
    group.add(&btn_row);

    // Fetch logs
    let overlay = toast_overlay.clone();
    let buf = buffer.clone();
    fetch_btn.connect_clicked(move |_| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let text = match send_command(DaemonCommand::GetRecentLogs(100)) {
                Ok(DaemonResponse::RecentLogs(t)) => {
                    if t.is_empty() {
                        "(no log entries)".into()
                    } else {
                        t
                    }
                }
                Ok(DaemonResponse::Error(e)) => format!("service error: {e}"),
                Err(e) => format!("ipc error: {e}"),
                _ => "Unexpected response".into(),
            };
            let _ = tx.send(text);
        });
        let o = overlay.clone();
        let b = buf.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || match rx.try_recv() {
            Ok(text) => {
                b.set_text(&text);
                o.add_toast(adw::Toast::new("Logs fetched"));
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => {
                o.add_toast(adw::Toast::new("Failed to fetch logs"));
                glib::ControlFlow::Break
            }
        });
    });

    // Auto-fetch once shortly after startup so opening the Logs tab shows
    // content immediately; Fetch stays as the manual refresh.
    {
        let buf_auto = buffer.clone();
        glib::timeout_add_local_once(Duration::from_millis(600), move || {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let text = match send_command(DaemonCommand::GetRecentLogs(100)) {
                    Ok(DaemonResponse::RecentLogs(t)) if !t.is_empty() => t,
                    Ok(DaemonResponse::RecentLogs(_)) => "(no log entries)".into(),
                    Ok(DaemonResponse::Error(e)) => format!("service error: {e}"),
                    Err(e) => format!("ipc error: {e}"),
                    _ => "Unexpected response".into(),
                };
                let _ = tx.send(text);
            });
            let b = buf_auto.clone();
            glib::timeout_add_local(Duration::from_millis(100), move || match rx.try_recv() {
                Ok(text) => {
                    b.set_text(&text);
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(_) => glib::ControlFlow::Break,
            });
        });
    }

    // Copy to clipboard
    let buf_copy = buffer.clone();
    copy_btn.connect_clicked(move |_| {
        if let Some(display) = gtk::gdk::Display::default() {
            let (start, end) = buf_copy.bounds();
            let text = buf_copy.text(&start, &end, false);
            display.clipboard().set_text(&text);
        }
    });

    // Toggle verbose / quiet
    let level_btn_c = level_btn.clone();
    let overlay_for_level = toast_overlay.clone();
    level_btn.connect_clicked(move |btn| {
        // Button shows "Verbose" when daemon is at info → clicking switches to debug.
        // Button shows "Quiet" when daemon is at debug → clicking switches back to info.
        let switching_to_debug = btn.label().is_some_and(|l| l == "Verbose");
        let new_level = if switching_to_debug { "debug" } else { "info" };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let r = send_command(DaemonCommand::SetLogLevel(new_level.into()));
            let _ = tx.send(r);
        });
        let btn_c = level_btn_c.clone();
        let overlay = overlay_for_level.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || match rx.try_recv() {
            Ok(Ok(DaemonResponse::Ok)) => {
                btn_c.set_label(if switching_to_debug {
                    "Quiet"
                } else {
                    "Verbose"
                });
                overlay.add_toast(adw::Toast::new(if switching_to_debug {
                    "Debug logging on"
                } else {
                    "Info logging on"
                }));
                glib::ControlFlow::Break
            }
            Ok(Ok(DaemonResponse::Error(e))) => {
                overlay.add_toast(adw::Toast::new(&format!("Failed: {e}")));
                glib::ControlFlow::Break
            }
            _ => {
                overlay.add_toast(adw::Toast::new("Daemon unreachable"));
                glib::ControlFlow::Break
            }
        });
    });

    gate.track(&fetch_btn);
    gate.track(&level_btn);
    group
}

/// Fixed widths so pills and buttons line up across Health rows.
pub(crate) const PILL_COL_W: i32 = 104;
pub(crate) const BTN_COL_W: i32 = 112;

pub(crate) fn rgb_button_label(health: legion_core::rgb_panic::Health) -> &'static str {
    use legion_core::rgb_panic::Health;
    match health {
        Health::SoftIssue => "Repair",
        Health::HardwareBroken => "Reset USB",
        Health::Ok | Health::NotApplicable => "Check again",
    }
}

fn rgb_title(health: legion_core::rgb_panic::Health, summary: &str) -> String {
    if health == legion_core::rgb_panic::Health::Ok {
        "Keyboard lighting is working".to_string()
    } else {
        summary.to_string()
    }
}

pub(crate) fn rgb_pill(health: legion_core::rgb_panic::Health) -> (&'static str, &'static str) {
    use legion_core::rgb_panic::Health;
    match health {
        Health::Ok => ("Working", "ok"),
        Health::SoftIssue => ("Needs repair", "warn"),
        Health::HardwareBroken => ("Not responding", "bad"),
        Health::NotApplicable => ("N/A", "muted"),
    }
}

pub(crate) fn rgb_pill_tooltip(health: legion_core::rgb_panic::Health) -> &'static str {
    use legion_core::rgb_panic::Health;
    match health {
        Health::Ok => "Spectrum HID answering, brightness and ioctl look fine",
        Health::SoftIssue => {
            "Lights stuck off, permissions, or recent kernel USB blip, auto-fix should help"
        }
        Health::HardwareBroken => {
            "HID ioctl dead or device missing, needs USB reset (daemon) or replug"
        }
        Health::NotApplicable => "No 048d:c197 Spectrum controller on this machine",
    }
}

pub(crate) fn rgb_short_help(health: legion_core::rgb_panic::Health) -> &'static str {
    use legion_core::rgb_panic::Health;
    match health {
        Health::Ok => {
            "Watched in the background; repaired automatically if it stops responding."
        }
        Health::SoftIssue => "Lighting is not responding. Press Repair to restore it.",
        Health::HardwareBroken => {
            "The keyboard controller is not responding. Reset USB to bring it back."
        }
        Health::NotApplicable => "No Spectrum RGB hardware detected.",
    }
}

// ─── About ──────────────────────────────────────────────────────────────────

/// Health → Self-check: read-only local checks plus the anomaly scan, with
/// copy-friendly results.
pub(crate) fn build_selfcheck_section(toast_overlay: &adw::ToastOverlay) -> adw::PreferencesGroup {
    let group = pref_group("Self-check", None);
    let check_row = adw::ActionRow::builder()
        .title("Run read-only checks")
        .activatable(false)
        .build();
    let run_btn = gtk::Button::with_label("Run");
    run_btn.set_valign(Align::Center);
    run_btn.add_css_class("pill-btn");
    run_btn.set_halign(Align::Center);
    tip(
        &run_btn,
        "Runs read-only local checks, nothing is written or sent",
    );
    check_row.add_suffix(&run_btn);
    group.add(&check_row);

    // Results land here once the first run completes; hidden until then.
    let results = gtk::Label::new(None);
    results.add_css_class("detail-body");
    results.set_halign(Align::Start);
    results.set_wrap(true);
    results.set_xalign(0.0);
    results.set_selectable(true);
    results.set_margin_start(12);
    results.set_margin_end(12);
    results.set_margin_top(4);
    results.set_margin_bottom(8);
    tip(
        &results,
        "One line per check, selectable so you can copy it into a bug report",
    );
    let expander = adw::ExpanderRow::builder()
        .title("Self-check results")
        .visible(false)
        .build();
    expander.add_row(&results);
    group.add(&expander);

    {
        let overlay = toast_overlay.clone();
        let run_btn_connect = run_btn.clone();
        let run_btn_closure = run_btn.clone();
        let expander_closure = expander.clone();
        let results_closure = results.clone();
        run_btn_connect.connect_clicked(move |_| {
            run_btn_closure.set_sensitive(false);
            run_btn_closure.set_label("Running…");
            let overlay = overlay.clone();
            let run_btn = run_btn_closure.clone();
            let expander = expander_closure.clone();
            let results = results_closure.clone();
            dispatch_async(
                move || {
                    Ok::<_, String>((
                        legion_core::selftest::run_self_checks(),
                        legion_core::selftest::scan_faults(),
                    ))
                },
                "Self-check stopped without a result",
                move |result| {
                    run_btn.set_sensitive(true);
                    run_btn.set_label("Run");
                    match result {
                        Ok((checks, faults)) => {
                            let total = checks.len();
                            let passed = checks.iter().filter(|c| c.ok).count();
                            let mut lines: Vec<String> = checks
                                .iter()
                                .map(|c| {
                                    format!(
                                        "{} {}: {}",
                                        if c.ok { "OK" } else { "Problem" },
                                        c.name,
                                        c.detail
                                    )
                                })
                                .collect();
                            for fault in &faults {
                                lines.push(format!("Warning {:?}: {}", fault.severity, fault.detail));
                            }
                            results.set_text(&lines.join("\n"));
                            expander.set_title(&format!(
                                "Self-check results ({passed}/{total} passed)"
                            ));
                            expander.set_expanded(true);
                            expander.set_visible(true);
                        }
                        Err(error) => toast_error(&overlay, &error),
                    }
                },
            );
        });
    }

    group
}
