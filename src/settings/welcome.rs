//! First-launch welcome window and the guided setup walkthrough.

use super::*;

/// Apply a telemetry enable/disable across every surface at once: persisted
/// config, the shared consent cell gating Send-now, and the live Setup-page
/// switch — the latter guarded by `sync` so programmatic changes never
/// re-trigger the About page's opt-out nudge.
pub(crate) fn set_telemetry(
    enabled: bool,
    consent: &Rc<Cell<bool>>,
    share: Option<&adw::SwitchRow>,
    sync: &Rc<Cell<bool>>,
) {
    legion_core::config::update(|c| c.diagnostics.enabled = enabled);
    push_telemetry_to_daemon(enabled);
    consent.set(enabled);
    sync.set(true);
    if let Some(row) = share {
        row.set_active(enabled);
    }
    sync.set(false);
}

/// The root daemon keeps its own config and sends telemetry itself, so the
/// user's choice must reach it over IPC. Background thread: the GTK main
/// loop never waits on the socket. Best effort — logged on failure.
pub(crate) fn push_telemetry_to_daemon(enabled: bool) {
    std::thread::spawn(move || {
        use legion_core::comms::{send_command, DaemonCommand, DaemonResponse};
        match send_command(DaemonCommand::SetTelemetry(enabled)) {
            Ok(DaemonResponse::Ok) => log::info!("telemetry choice ({enabled}) applied to daemon"),
            Ok(other) => log::warn!("daemon rejected telemetry choice: {other:?}"),
            Err(e) => log::warn!("could not send telemetry choice to daemon: {e}"),
        }
    });
}

/// First-launch welcome: one compact window with the app mark, a single
/// sentence, the diagnostics switch and Start. Optional components are
/// offered later on the Health page. Any close path marks the welcome seen,
/// so it shows exactly once.
pub(crate) fn show_welcome_if_needed(
    parent: &impl glib::object::IsA<gtk::Widget>,
    consent: &Rc<Cell<bool>>,
    share_switch: Option<&adw::SwitchRow>,
    sync: &Rc<Cell<bool>>,
) {
    if legion_core::config::welcome_seen() {
        return;
    }

    let win = adw::Window::builder()
        .title("Legion Control")
        .modal(true)
        .default_width(440)
        .resizable(false)
        .build();
    let root = parent.as_ref().root();
    if let Some(host) = root.and_then(|r| r.downcast::<gtk::Window>().ok()) {
        win.set_transient_for(Some(&host));
    }

    // ── brand ──
    let mark = color_icon(include_bytes!("../../data/icons/app-mark.svg"), 64);
    mark.set_halign(Align::Center);
    let title = gtk::Label::new(Some("Legion Control"));
    title.add_css_class("title-1");
    title.set_halign(Align::Center);
    let tagline = gtk::Label::new(Some(
        "Unofficial tool for Lenovo Legion laptops. Not affiliated with Lenovo.",
    ));
    tagline.set_wrap(true);
    tagline.set_justify(gtk::Justification::Center);
    tagline.set_halign(Align::Center);

    // ── diagnostics ──
    let telemetry_row = adw::SwitchRow::builder()
        .title("Share anonymous diagnostics")
        .subtitle("Anonymous hardware stats. Opt out any time.")
        .active(legion_core::config::get().diagnostics.enabled)
        .build();
    tip(
        &telemetry_row,
        "One anonymized report per minute: hardware model, distro/kernel, sensors, \
         fan/battery stats, self-check results. NEVER included: hostname, username, \
         serials, MACs, IPs, key colors, custom profile names. \
         Change any time under Settings, Setup.",
    );
    let telemetry_group = pref_group("", None);
    telemetry_group.add(&telemetry_row);

    let consent_c0 = consent.clone();
    let share_c0 = share_switch.cloned();
    let sync_c0 = sync.clone();
    let win_c0 = win.clone();
    telemetry_row.connect_active_notify(move |row| {
        if row.is_active() {
            set_telemetry(true, &consent_c0, share_c0.as_ref(), &sync_c0);
            return;
        }
        // Opting out gets the standard nudge before anything changes.
        let row_c = row.clone();
        let consent_c = consent_c0.clone();
        let share_c = share_c0.clone();
        let sync_c = sync_c0.clone();
        let win_c = win_c0.clone();
        confirm_disable_telemetry(Some(win_c.upcast_ref::<gtk::Window>()), move |confirmed| {
            if confirmed {
                set_telemetry(false, &consent_c, share_c.as_ref(), &sync_c);
            } else {
                sync_c.set(true);
                row_c.set_active(true);
                sync_c.set(false);
            }
        });
    });

    // ── actions ──
    let privacy_btn = gtk::Button::with_label("Privacy policy");
    privacy_btn.add_css_class("pill-btn");
    tip(
        &privacy_btn,
        "What is sent and what is never collected. Opens GitHub in your browser.",
    );
    privacy_btn.connect_clicked(|_| {
        open_uri("https://github.com/encomjp/Lenovo-Legion-Control#alpha-telemetry-opt-out");
    });
    let start_btn = primary_button_tip("Start", Some("Open Legion Control"));
    let actions = gtk::Box::new(Orientation::Horizontal, 12);
    let spacer = gtk::Box::new(Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    actions.append(&privacy_btn);
    actions.append(&spacer);
    actions.append(&start_btn);

    // ── layout ──
    let page = gtk::Box::new(Orientation::Vertical, 16);
    page.set_margin_top(12);
    page.set_margin_bottom(24);
    page.set_margin_start(28);
    page.set_margin_end(28);
    page.append(&mark);
    page.append(&title);
    page.append(&tagline);
    page.append(&telemetry_group);
    page.append(&actions);

    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    header.set_show_title(false);
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&page));
    win.set_content(Some(&toolbar));

    // Any close path records the welcome as seen: it shows exactly once.
    win.connect_close_request(|_| {
        legion_core::config::mark_welcome_seen();
        glib::Propagation::Proceed
    });
    let win_start = win.clone();
    start_btn.connect_clicked(move |_| win_start.close());

    win.present();
}

// ─── First-launch guided setup ──────────────────────────────────────────────

/// Steps of the first-launch walkthrough started by the welcome dialog's
/// "First-time setup" response. Each step presents one `adw::AlertDialog`
/// and chains into the next via its response handler.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SetupStep {
    /// Probe the privileged control service over IPC (autoinstalls if missing).
    Daemon,
    /// Start-on-boot choice: login autostart + daemon systemd enable,
    /// optional AMD tuning backend (Curve Optimizer undervolt).
    Startup,
    /// Identify model, machine type, CPU, GPU, and fan channels.
    Hardware,
    /// Read-only self-checks plus the fault scan.
    SelfCheck,
    /// Summary; closing returns to the main view.
    Done,
}

impl SetupStep {
    /// 1-based position for dialog titles ("First-time setup (2/5) — …").
    fn number(self) -> usize {
        match self {
            SetupStep::Daemon => 1,
            SetupStep::Startup => 2,
            SetupStep::Hardware => 3,
            SetupStep::SelfCheck => 4,
            SetupStep::Done => 5,
        }
    }

    /// The step that follows this one (Done is its own successor).
    fn next(self) -> Self {
        match self {
            SetupStep::Daemon => SetupStep::Startup,
            SetupStep::Startup => SetupStep::Hardware,
            SetupStep::Hardware => SetupStep::SelfCheck,
            SetupStep::SelfCheck | SetupStep::Done => SetupStep::Done,
        }
    }
}

/// Everything a walkthrough step needs to present its dialog and reach the
/// next one. Cheap to clone — every response handler takes its own copy so
/// retry loops can re-enter [`SetupCtx::run`] freely.
#[derive(Clone)]
pub(crate) struct SetupCtx {
    win: Option<gtk::Window>,
    stack: adw::ViewStack,
    about_tabs: Option<adw::ViewStack>,
    consent: Rc<Cell<bool>>,
    share_switch: Option<adw::SwitchRow>,
    /// Suppresses the About page's opt-out nudge while this context flips the
    /// live switch programmatically (see [`SetupCtx::disable_telemetry`]).
    sync: Rc<Cell<bool>>,
}

/// Guided first-launch walkthrough behind the welcome dialog's "First-time
/// setup" response: five chained alert dialogs (service probe/autoinstall,
/// startup & tuning, hardware identity, self-check, summary). Every probe
/// runs on a `dispatch_async` worker thread; each dialog appears once its
/// result is in.
/// Kept for a future entry point (the welcome window no longer starts it).
#[allow(dead_code)]
pub(crate) fn run_guided_setup(
    stack: &adw::ViewStack,
    about_tabs: Option<&adw::ViewStack>,
    consent: &Rc<Cell<bool>>,
    share_switch: Option<&adw::SwitchRow>,
    sync: &Rc<Cell<bool>>,
) {
    // The main view stack lives inside the window — its root is the parent
    // every step dialog is presented on.
    let win = stack
        .root()
        .and_then(|root| root.downcast::<gtk::Window>().ok());
    SetupCtx {
        win,
        stack: stack.clone(),
        about_tabs: about_tabs.cloned(),
        consent: consent.clone(),
        share_switch: share_switch.cloned(),
        sync: sync.clone(),
    }
    .run(SetupStep::Daemon);
}

impl SetupCtx {
    fn run(self, step: SetupStep) {
        match step {
            SetupStep::Daemon => self.daemon_step(),
            SetupStep::Startup => self.startup_step(),
            SetupStep::Hardware => self.hardware_step(),
            SetupStep::SelfCheck => self.selfcheck_step(),
            SetupStep::Done => self.done_step(),
        }
    }

    /// Present a finished step dialog; each response id is mapped through
    /// `routes` into the next step (`None` leaves the walkthrough).
    fn present(
        &self,
        dialog: adw::AlertDialog,
        routes: impl Fn(&str) -> Option<SetupStep> + 'static,
    ) {
        let ctx = self.clone();
        dialog.connect_response(None, move |_, response| {
            if let Some(next) = routes(response) {
                ctx.clone().run(next);
            }
        });
        dialog.present(self.win.as_ref());
    }

    /// Failure variant of any step: explain what went wrong, offer Retry
    /// (re-runs the same step) or Continue anyway (skips ahead). Closing the
    /// dialog abandons the rest of the walkthrough — welcome-seen is already
    /// recorded, nothing is lost.
    fn retryable_failure(&self, step: SetupStep, topic: &str, problem: &str, hint: &str) {
        let dialog = status_step_dialog(
            step,
            topic,
            false,
            problem,
            hint,
            [("continue", "Continue anyway"), ("retry", "Retry")],
            "retry",
        );
        self.present(dialog, move |response| match response {
            "retry" => Some(step),
            "continue" => Some(step.next()),
            _ => None,
        });
    }

    /// Step 1 — is the privileged control service reachable? If not, offer
    /// one-click autoinstall via the PolicyKit helper (the same path the
    /// Settings → Setup Enable button uses). Retry re-probes; Enable runs
    /// `start_legion_control()` off-thread and advances on success.
    fn daemon_step(self) {
        // Connecting to the socket can block briefly — probe off-thread.
        dispatch_async(
            move || send_command(DaemonCommand::GetProfile).map(|_| ()),
            "Service probe stopped without a result",
            move |result| match result {
                Ok(()) => {
                    let dialog = status_step_dialog(
                        SetupStep::Daemon,
                        "Background service",
                        true,
                        "The background service is running",
                        "It handles fans, power modes and charge limits.",
                        [("continue", "Continue")],
                        "continue",
                    );
                    self.present(dialog, |_| Some(SetupStep::Startup));
                }
                Err(_) => {
                    let dialog = setup_step_dialog(
                        SetupStep::Daemon,
                        "Background service",
                        "The background service is not running.\n\n\
                         Enable it now? One system prompt will install and start it.\n\n\
                         You can continue without it. Detection and self-checks still work.",
                        [
                            ("enable", "Enable service"),
                            ("continue", "Continue anyway"),
                        ],
                        "enable",
                    );
                    dialog.set_response_appearance("enable", adw::ResponseAppearance::Suggested);
                    let ctx = self.clone();
                    let ctx_for_closure = ctx.clone();
                    dialog.connect_response(None, move |_, response| {
                        match response {
                            "enable" => {
                                let ctx2 = ctx_for_closure.clone();
                                // One authorized transaction — same helper the
                                // Settings page uses; shows the system password prompt.
                                dispatch_async(
                                    || start_legion_control().map(|_| ()),
                                    "Enable service stopped without a result",
                                    move |res| match res {
                                        Ok(()) => {
                                            let ok_dialog = status_step_dialog(
                                                SetupStep::Daemon,
                                                "Background service",
                                                true,
                                                "The background service is enabled and running",
                                                "",
                                                [("continue", "Continue")],
                                                "continue",
                                            );
                                            ctx2.present(ok_dialog, |_| Some(SetupStep::Startup));
                                        }
                                        Err(e) => ctx2.retryable_failure(
                                            SetupStep::Daemon,
                                            "Background service",
                                            "Couldn't enable the service",
                                            &e,
                                        ),
                                    },
                                );
                            }
                            "continue" => ctx_for_closure.clone().run(SetupStep::Startup),
                            _ => {}
                        }
                    });
                    dialog.present(ctx.win.as_ref());
                }
            },
        );
    }

    /// Step 2 — start on boot + optional AMD tuning backend (undervolt).
    /// One prompt, two switches. Apply also ENABLES the systemd unit so the
    /// daemon — not just the app — comes up on boot.
    fn startup_step(self) {
        let dialog = setup_step_dialog(
            SetupStep::Startup,
            "Startup & tuning",
            "Pick what Legion Control sets up now. You can change it any time under \
             Settings, Setup.",
            [("apply", "Apply"), ("skip", "Skip")],
            "apply",
        );

        let autostart_switch = adw::SwitchRow::builder()
            .title("Launch at login")
            .subtitle("App starts hidden to tray")
            .active(true)
            .build();
        tip(
            &autostart_switch,
            "Adds Legion Control to your desktop autostart (~/.config/autostart) and \
             turns on the legion-control service so it also starts on boot.",
        );
        // ryzen_smu is only useful where Curve Optimizer is validated, and its
        // DKMS build fails on fresh installs without dkms + kernel headers —
        // only pre-select it when it can actually succeed.
        let co_offered = legion_core::undervolt::machine_validated();
        let co_missing = dkms_prereqs_missing();
        let co_switch = adw::SwitchRow::builder()
            .title("Install AMD tuning backend")
            .subtitle(match &co_missing {
                None => "Curve Optimizer undervolt (ryzen_smu via DKMS)".to_string(),
                Some(cmd) => format!("Needs dkms + kernel headers first: {cmd}"),
            })
            .active(co_missing.is_none())
            .visible(co_offered)
            .build();
        tip(
            &co_switch,
            "Builds and loads the bundled ryzen_smu kernel module through DKMS \
             (one admin prompt). Needed for Curve Optimizer on the CPU page.",
        );

        let extra = gtk::Box::new(Orientation::Vertical, 2);
        extra.set_margin_top(8);
        extra.set_margin_bottom(8);
        extra.set_margin_start(12);
        extra.set_margin_end(12);
        extra.append(&autostart_switch);
        extra.append(&co_switch);
        dialog.set_extra_child(Some(&extra));

        let ctx = self.clone();
        let ctx_for_closure = ctx.clone();
        dialog.connect_response(None, move |_, response| {
            match response {
                "apply" => {
                    let want_autostart = autostart_switch.is_active();
                    let want_co = co_switch.is_active();
                    let ctx2 = ctx_for_closure.clone();
                    // Everything here can prompt (pkexec) or touch the disk —
                    // run off-thread so the dialog never freezes.
                    dispatch_async(
                        move || {
                            let mut report: Vec<String> = Vec::new();
                            if want_autostart {
                                set_autostart(true)?;
                                report.push("App will launch at login (hidden to tray)".into());
                                // Make sure the daemon also starts on boot: the
                                // helper's enable-daemon stages + `enable --now`.
                                let enabled = std::process::Command::new("systemctl")
                                    .args(["is-enabled", "legion-control"])
                                    .output()
                                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                                    .unwrap_or_default();
                                if enabled == "enabled" {
                                    report.push("Service already starts on boot".into());
                                } else {
                                    run_setup_helper_blocking("enable-daemon")?;
                                    report.push("Service now starts on boot".into());
                                }
                                if !daemon_ok() {
                                    start_legion_control()?;
                                }
                            }
                            if want_co && co_offered {
                                // Optional: a failed DKMS build must not undo the
                                // rest of the step — report it and carry on.
                                match run_setup_helper_blocking("install-ryzen-smu") {
                                    Ok(_) => report.push(
                                        "AMD tuning driver installed. Curve Optimizer is \
                                         on the Performance page"
                                            .into(),
                                    ),
                                    Err(e) => report.push(format!(
                                        "AMD tuning driver not installed (optional): {e}{}",
                                        dkms_prereqs_missing()
                                            .map(|cmd| format!("\n   Install the prerequisites with: {cmd}"))
                                            .unwrap_or_default()
                                    )),
                                }
                            }
                            Ok(report)
                        },
                        "Startup setup stopped without a result",
                        move |result| match result {
                            Ok(report) => {
                                let (headline, body) = if report.is_empty() {
                                    (
                                        "Nothing selected",
                                        "You can change this later under Settings, Setup."
                                            .to_string(),
                                    )
                                } else {
                                    ("Done", report.join("\n"))
                                };
                                let ok_dialog = status_step_dialog(
                                    SetupStep::Startup,
                                    "Startup & tuning",
                                    true,
                                    headline,
                                    &body,
                                    [("continue", "Continue")],
                                    "continue",
                                );
                                ctx2.present(ok_dialog, |_| Some(SetupStep::Hardware));
                            }
                            Err(e) => ctx2.retryable_failure(
                                SetupStep::Startup,
                                "Startup & tuning",
                                "Setup couldn't finish",
                                &e,
                            ),
                        },
                    );
                }
                "skip" => ctx_for_closure.clone().run(SetupStep::Hardware),
                _ => {}
            }
        });
        dialog.present(ctx.win.as_ref());
    }

    /// Step 3 — what machine is this?
    fn hardware_step(self) {
        // Full DMI + GPU probing may spawn nvidia-smi (up to ~3 s) — worker
        // thread keeps the main loop responsive.
        dispatch_async(
            || Ok::<_, String>(legion_core::device::detect()),
            "Hardware detection stopped without a result",
            move |result| match result {
                Ok(info) => {
                    let known = |value: &String| -> String {
                        if value.is_empty() {
                            "unknown".to_string()
                        } else {
                            value.clone()
                        }
                    };
                    let fan_count = info.capabilities.fans.len();
                    let plural = if fan_count == 1 { "" } else { "s" };
                    let body = format!(
                        "Model: {}\n\
                         Machine type: {}\n\
                         CPU: {}\n\
                         GPU: {}\n\
                         Fans: {} channel{}",
                        known(&info.model),
                        known(&info.machine_type),
                        known(&info.cpu_model),
                        known(&info.gpu_model),
                        fan_count,
                        plural,
                    );
                    let dialog = status_step_dialog(
                        SetupStep::Hardware,
                        "Your hardware",
                        true,
                        "Hardware detected",
                        &body,
                        [("continue", "Continue")],
                        "continue",
                    );
                    self.present(dialog, |_| Some(SetupStep::SelfCheck));
                }
                Err(error) => self.retryable_failure(
                    SetupStep::Hardware,
                    "Your hardware",
                    "Hardware detection failed",
                    &error,
                ),
            },
        );
    }

    /// Step 4 — read-only health checks plus anomaly scan.
    fn selfcheck_step(self) {
        // Both probes are fast (<200 ms) but stay off-thread so a slow EC
        // read can never stall the main loop.
        dispatch_async(
            || {
                let checks = legion_core::selftest::run_self_checks();
                let faults = legion_core::selftest::scan_faults();
                Ok::<_, String>((checks, faults))
            },
            "Self-check stopped without a result",
            move |result| match result {
                Ok((checks, faults)) => {
                    let total = checks.len();
                    let passed = checks.iter().filter(|c| c.ok).count();
                    let mut lines: Vec<String> = Vec::new();

                    let criticals: Vec<_> = faults
                        .iter()
                        .filter(|f| f.severity == legion_core::selftest::Severity::Critical)
                        .collect();
                    if criticals.is_empty() {
                        lines.push("No serious problems found.".into());
                    } else {
                        lines.push(format!(
                            "{} serious problem{}:",
                            criticals.len(),
                            if criticals.len() == 1 { "" } else { "s" }
                        ));
                        const MAX_LISTED: usize = 6;
                        for fault in criticals.iter().take(MAX_LISTED) {
                            lines.push(fault.detail.to_string());
                        }
                        let hidden = criticals.len().saturating_sub(MAX_LISTED);
                        if hidden > 0 {
                            lines.push(format!("and {hidden} more"));
                        }
                    }

                    let body = lines.join("\n");
                    let dialog = status_step_dialog(
                        SetupStep::SelfCheck,
                        "Self-check",
                        criticals.is_empty(),
                        &format!("Self-check finished: {passed} of {total} passed"),
                        &body,
                        [("continue", "Continue")],
                        "continue",
                    );
                    self.present(dialog, |_| Some(SetupStep::Done));
                }
                Err(error) => self.retryable_failure(
                    SetupStep::SelfCheck,
                    "Self-check",
                    "Self-check couldn't run",
                    &error,
                ),
            },
        );
    }

    #[allow(dead_code)]
    /// Flip every telemetry surface off at once: persisted config, the
    /// shared consent cell gating Send-now, and the live Setup-page switch.
    fn disable_telemetry(&self) {
        legion_core::config::update(|c| c.diagnostics.enabled = false);
        push_telemetry_to_daemon(false);
        self.consent.set(false);
        self.sync.set(true);
        if let Some(row) = self.share_switch.as_ref() {
            row.set_active(false);
        }
        self.sync.set(false);
    }

    /// Step 5 — farewell. Close returns to the main view; the secondary
    /// button keeps the old Settings → Setup shortcut within reach.
    fn done_step(self) {
        let dialog = status_step_dialog(
            SetupStep::Done,
            "All done",
            true,
            "Setup complete",
            "You can change all of these later under Setup.",
            [("opensetup", "Open Setup"), ("done", "Close")],
            "done",
        );
        let stack = self.stack.clone();
        let about_tabs = self.about_tabs.clone();
        dialog.connect_response(None, move |_, response| {
            if response == "opensetup" {
                // "setup" is an INNER tab of the About hub, not an outer
                // stack child — open the hub first, then select the tab.
                stack.set_visible_child_name("about");
                if let Some(tabs) = about_tabs.as_ref() {
                    tabs.set_visible_child_name("setup");
                }
            } else {
                stack.set_visible_child_name("overview");
            }
        });
        dialog.present(self.win.as_ref());
    }
}

/// Build one standard walkthrough step dialog: numbered title, body text,
/// labeled response buttons (`suggested` is highlighted and bound to Enter).
pub(crate) fn setup_step_dialog(
    step: SetupStep,
    topic: &str,
    body: &str,
    responses: impl IntoIterator<Item = (&'static str, &'static str)>,
    suggested: &'static str,
) -> adw::AlertDialog {
    let title = format!(
        "Setup ({}/{}): {topic}",
        step.number(),
        SetupStep::Done.number()
    );
    let dialog = adw::AlertDialog::new(Some(title.as_str()), Some(body));
    for (id, label) in responses {
        dialog.add_response(id, label);
    }
    dialog.set_response_appearance(suggested, adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some(suggested));
    dialog
}

/// Like [`setup_step_dialog`], but leads with a symbolic status icon and a
/// bold headline instead of a text glyph; `rest` is the plain detail text.
pub(crate) fn status_step_dialog(
    step: SetupStep,
    topic: &str,
    ok: bool,
    headline: &str,
    rest: &str,
    responses: impl IntoIterator<Item = (&'static str, &'static str)>,
    suggested: &'static str,
) -> adw::AlertDialog {
    let dialog = setup_step_dialog(step, topic, "", responses, suggested);
    let col = gtk::Box::new(Orientation::Vertical, 10);
    let head = gtk::Box::new(Orientation::Horizontal, 8);
    head.set_halign(Align::Center);
    let icon = gtk::Image::from_icon_name(if ok {
        "emblem-ok-symbolic"
    } else {
        "dialog-warning-symbolic"
    });
    icon.add_css_class(if ok { "success" } else { "warning" });
    let title = gtk::Label::new(Some(headline));
    title.add_css_class("heading");
    title.set_wrap(true);
    head.append(&icon);
    head.append(&title);
    col.append(&head);
    if !rest.is_empty() {
        let detail = gtk::Label::new(Some(rest));
        detail.set_wrap(true);
        detail.set_justify(gtk::Justification::Center);
        detail.set_max_width_chars(48);
        col.append(&detail);
    }
    dialog.set_extra_child(Some(&col));
    dialog
}

/// `Some(install command)` when the ryzen_smu DKMS build cannot work yet
/// because `dkms` or the running kernel's headers are missing; `None` when
/// both are present. The command matches the distro and kernel flavour.
pub(crate) fn dkms_prereqs_missing() -> Option<String> {
    let release = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let has_dkms = ["/usr/bin/dkms", "/usr/sbin/dkms"]
        .iter()
        .any(|p| std::path::Path::new(p).exists());
    let has_headers = std::path::Path::new(&format!("/lib/modules/{release}/build")).exists();
    if has_dkms && has_headers {
        return None;
    }
    let os = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    Some(dkms_install_hint(&os, &release))
}

/// Distro-specific `dkms` + headers install command (pure; unit-tested).
fn dkms_install_hint(os_release: &str, kernel: &str) -> String {
    let ids: String = os_release
        .lines()
        .filter(|l| l.starts_with("ID=") || l.starts_with("ID_LIKE="))
        .map(|l| l.split_once('=').map(|(_, v)| v.trim_matches('"').to_ascii_lowercase()).unwrap_or_default())
        .collect::<Vec<_>>()
        .join(" ");
    if ids.contains("arch") {
        let headers = if kernel.contains("cachyos") {
            "linux-cachyos-headers"
        } else if kernel.contains("-zen") {
            "linux-zen-headers"
        } else if kernel.contains("-lts") {
            "linux-lts-headers"
        } else {
            "linux-headers"
        };
        format!("sudo pacman -S dkms {headers}")
    } else if ids.contains("debian") || ids.contains("ubuntu") {
        format!("sudo apt install dkms linux-headers-{kernel}")
    } else if ids.contains("fedora") || ids.contains("rhel") {
        format!("sudo dnf install dkms kernel-devel-{kernel}")
    } else if ids.contains("suse") {
        "sudo zypper install dkms kernel-devel".to_string()
    } else {
        "install dkms and the headers for your running kernel".to_string()
    }
}

#[cfg(test)]
mod dkms_hint_tests {
    use super::dkms_install_hint;

    #[test]
    fn hints_match_distro_and_kernel_flavour() {
        assert_eq!(
            dkms_install_hint("ID=cachyos\nID_LIKE=arch\n", "7.2.10-1-cachyos"),
            "sudo pacman -S dkms linux-cachyos-headers"
        );
        assert_eq!(dkms_install_hint("ID=arch\n", "6.19.2-zen1-1-zen"), "sudo pacman -S dkms linux-zen-headers");
        assert_eq!(dkms_install_hint("ID=arch\n", "6.19.2-arch1-1"), "sudo pacman -S dkms linux-headers");
        assert_eq!(
            dkms_install_hint("ID=ubuntu\nID_LIKE=debian\n", "6.8.0-45-generic"),
            "sudo apt install dkms linux-headers-6.8.0-45-generic"
        );
        assert_eq!(
            dkms_install_hint("ID=fedora\n", "6.19.2-200.fc44.x86_64"),
            "sudo dnf install dkms kernel-devel-6.19.2-200.fc44.x86_64"
        );
        assert!(dkms_install_hint("ID=gentoo\n", "6.1").contains("headers"));
    }
}
