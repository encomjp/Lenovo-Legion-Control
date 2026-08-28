//! Fix page — speakers, Spectrum RGB recovery, udev rule, daemon logs.

use super::*;

pub(crate) fn build_fix_audio_page(toast_overlay: &adw::ToastOverlay) -> gtk::Box {
    let page = page_lede("");
    page.append(&build_speakers_section(toast_overlay));
    page
}

/// One "Fix" destination with an internal switcher instead of three sidebar
/// rows — keeps the rail short while all diagnostics stay one click away.
pub(crate) fn build_fix_page(
    toast_overlay: &adw::ToastOverlay,
    gate: &DaemonGate,
    initial: Option<&str>,
) -> gtk::Box {
    let page = page_lede("");

    let inner = adw::ViewStack::new();
    inner.set_vexpand(true);
    inner.add_titled(
        &page_shell(&build_fix_audio_page(toast_overlay)),
        Some("fix-audio"),
        "Speakers",
    );
    inner.add_titled(
        &page_shell(&build_fix_lighting_page(toast_overlay, gate)),
        Some("fix-lighting"),
        "RGB Fix",
    );
    inner.add_titled(
        &page_shell(&build_fix_logs_page(toast_overlay, gate)),
        Some("fix-logs"),
        "Logs",
    );
    if let Some(id) = initial {
        if inner.child_by_name(id).is_some() {
            inner.set_visible_child_name(id);
        }
    }

    // Same horizontal tab bar as the CPU/About/Lighting hubs.
    let switcher = adw::ViewSwitcher::new();
    switcher.set_stack(Some(&inner));
    switcher.set_policy(adw::ViewSwitcherPolicy::Wide);
    switcher.set_halign(Align::Center);
    tip(
        &switcher,
        "Speaker audio issues · stuck or dark Spectrum RGB · recent control-service output",
    );
    let bar = gtk::Box::new(Orientation::Vertical, 0);
    bar.add_css_class("hub-bar");
    bar.append(&switcher);
    page.append(&bar);
    page.append(&inner);
    page
}

pub(crate) fn build_fix_lighting_page(
    toast_overlay: &adw::ToastOverlay,
    gate: &DaemonGate,
) -> gtk::Box {
    let page = page_lede("");
    page.append(&build_lighting_reset_section(toast_overlay, gate));
    page.append(&build_udev_permanent_section(toast_overlay));
    page
}

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
    let group = pref_group("Permanent fix (udev)", None);

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
            "Rule file missing — Auto-fix only lasts until reboot; install permanently below"
        }),
    );

    let status_row = adw::ActionRow::builder()
        .title("Udev rule 99-legion.rules")
        .subtitle(if installed { "Present" } else { "Missing" })
        .activatable(false)
        .build();
    tip(
        &status_row,
        "Checks /etc/udev/rules.d/99-legion.rules (and /usr/lib/udev) for 048d:c197 with MODE 0660",
    );
    status_row.add_suffix(&pill);
    group.add(&status_row);

    let btn = primary_button_tip(
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
                        "Helper finished but rule still not found — check /etc/udev/rules.d/99-legion.rules",
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

pub(crate) fn build_fix_logs_page(
    toast_overlay: &adw::ToastOverlay,
    gate: &DaemonGate,
) -> gtk::Box {
    let page = page_lede("");
    page.append(&build_logs_section(toast_overlay, gate));
    page
}

/// Compact Fix for embedding inside Settings — one scrollable page with
/// the three diagnostics as stacked cards, no extra ViewSwitcher.
pub(crate) fn build_fix_compact(toast_overlay: &adw::ToastOverlay, gate: &DaemonGate) -> gtk::Box {
    let page = page_lede("");
    page.append(&build_speakers_section(toast_overlay));
    page.append(&build_lighting_reset_section(toast_overlay, gate));
    page.append(&build_udev_permanent_section(toast_overlay));
    page.append(&build_logs_section(toast_overlay, gate));
    page
}

pub(crate) fn build_lighting_reset_section(
    toast_overlay: &adw::ToastOverlay,
    gate: &DaemonGate,
) -> adw::PreferencesGroup {
    use legion_core::rgb_panic::{self, Health};

    let group = pref_group("Keyboard lighting issue", None);

    let diag0 = rgb_panic::diagnose();
    let (pill_text, pill_kind) = rgb_pill(diag0.health);
    let pill = status_pill_tip(pill_text, pill_kind, Some(rgb_pill_tooltip(diag0.health)));

    let status_row = adw::ActionRow::builder()
        .title(&diag0.summary)
        .subtitle(rgb_short_help(diag0.health))
        .activatable(false)
        .build();
    tip(
        &status_row,
        "Scans hidraw 048d:c197, ioctl health, brightness vs saved config, and kernel USB/HID errors",
    );
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
        "Raw diagnostic lines — selectable so you can copy them into a bug report",
    );
    expander.add_row(&details);
    group.add(&expander);

    let btn = primary_button_tip(
        match diag0.health {
            Health::Ok => "Re-check RGB",
            Health::SoftIssue => "Repair lighting",
            Health::HardwareBroken => "USB reset & restore",
            Health::NotApplicable => "Check RGB",
        },
        Some(
            "Soft lighting reset → fix permissions → USB reset → hid-generic rebind → restore saved look (daemon)",
        ),
    );
    let action = adw::ActionRow::builder()
        .title("Auto-fix")
        .activatable(false)
        .build();
    tip(
        &action,
        "Needs updated legion-control service for USB reset privileges",
    );
    action.add_suffix(&btn);
    group.add(&action);
    gate.track(&btn);

    let overlay = toast_overlay.clone();
    let pill_c = pill.clone();
    let status_c = status_row.clone();
    let details_c = details.clone();
    let expander_c = expander.clone();
    let btn_c = btn.clone();
    btn.connect_clicked(move |_| {
        set_busy(&btn_c, true, "Repair lighting");
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
                status_c.set_title(&summary);
                status_c.set_subtitle(rgb_short_help(health));
                let mut body = steps.iter().map(|s| format!("· {s}")).collect::<Vec<_>>();
                if !errors.is_empty() {
                    body.push(String::new());
                    body.push("Problems:".into());
                    body.extend(errors.iter().map(|s| format!("· {s}")));
                }
                details_c.set_text(&body.join("\n"));
                expander_c.set_expanded(true);
                match health {
                    Health::Ok => toast_ok(&overlay, "Spectrum RGB healthy"),
                    Health::SoftIssue => {
                        toast_error(&overlay, "Partially fixed — check Lighting tab")
                    }
                    Health::HardwareBroken => {
                        toast_error(&overlay, "Still broken — try reboot or USB replug")
                    }
                    Health::NotApplicable => toast_ok(&overlay, &summary),
                }
                let idle = match health {
                    Health::Ok => "Re-check RGB",
                    Health::SoftIssue => "Repair lighting",
                    Health::HardwareBroken => "USB reset & restore",
                    Health::NotApplicable => "Check RGB",
                };
                set_busy(&btn_c, false, idle);
                glib::ControlFlow::Break
            }
            Ok(Err(e)) => {
                set_pill(&pill_c, "Failed", "bad");
                details_c.set_text(&e);
                expander_c.set_expanded(true);
                toast_error(&overlay, &e);
                set_busy(&btn_c, false, "Repair lighting");
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => {
                set_busy(&btn_c, false, "Repair lighting");
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

    let group = pref_group("Daemon logs", None);

    let text_view = gtk::TextView::new();
    text_view.set_editable(false);
    text_view.set_monospace(true);
    text_view.set_wrap_mode(gtk::WrapMode::WordChar);
    text_view.set_top_margin(6);
    text_view.set_bottom_margin(6);
    text_view.set_left_margin(8);
    text_view.set_right_margin(8);
    let buffer = text_view.buffer();
    buffer.set_text("Fetching recent daemon output…");

    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&text_view));
    scroll.set_vexpand(true);
    scroll.set_min_content_height(200);
    scroll.set_max_content_height(400);
    group.add(&scroll);

    let fetch_btn = primary_button_tip(
        "Fetch logs",
        Some("Query the last 100 log lines from the daemon"),
    );
    let copy_btn = primary_button_tip("Copy", Some("Copy log text to clipboard"));
    let level_btn = primary_button_tip(
        "Verbose",
        Some("Toggle daemon between info and debug logging"),
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
                Ok(DaemonResponse::Error(e)) => format!("daemon error: {e}"),
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
                    Ok(DaemonResponse::Error(e)) => format!("daemon error: {e}"),
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

pub(crate) fn rgb_pill(health: legion_core::rgb_panic::Health) -> (&'static str, &'static str) {
    use legion_core::rgb_panic::Health;
    match health {
        Health::Ok => ("OK", "ok"),
        Health::SoftIssue => ("Panic", "warn"),
        Health::HardwareBroken => ("Not responding", "bad"),
        Health::NotApplicable => ("N/A", "muted"),
    }
}

pub(crate) fn rgb_pill_tooltip(health: legion_core::rgb_panic::Health) -> &'static str {
    use legion_core::rgb_panic::Health;
    match health {
        Health::Ok => "Spectrum HID answering — brightness and ioctl look fine",
        Health::SoftIssue => {
            "Lights stuck off, permissions, or recent kernel USB blip — auto-fix should help"
        }
        Health::HardwareBroken => {
            "HID ioctl dead or device missing — needs USB reset (daemon) or replug"
        }
        Health::NotApplicable => "No 048d:c197 Spectrum controller on this machine",
    }
}

pub(crate) fn rgb_short_help(health: legion_core::rgb_panic::Health) -> &'static str {
    use legion_core::rgb_panic::Health;
    match health {
        Health::Ok => {
            "Controller healthy. Daemon still watches kernel HID faults in the background."
        }
        Health::SoftIssue => "Lighting not responding — run Auto-fix to restore it.",
        Health::HardwareBroken => {
            "HID not responding — Auto-fix will USB-reset and rebind hid-generic."
        }
        Health::NotApplicable => "No Spectrum RGB hardware detected.",
    }
}

// ─── About ──────────────────────────────────────────────────────────────────
