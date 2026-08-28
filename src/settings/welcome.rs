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
    consent.set(enabled);
    sync.set(true);
    if let Some(row) = share {
        row.set_active(enabled);
    }
    sync.set(false);
}

/// First-launch welcome — a real window (not a cramped alert): brand header,
/// short intro, one compact telemetry opt-out switch, and horizontal actions.
/// Any close path marks the welcome seen, so it shows exactly once.
pub(crate) fn show_welcome_if_needed(
    parent: &impl glib::object::IsA<gtk::Widget>,
    stack: &adw::ViewStack,
    about_tabs: Option<&adw::ViewStack>,
    consent: &Rc<Cell<bool>>,
    share_switch: Option<&adw::SwitchRow>,
    sync: &Rc<Cell<bool>>,
) {
    if legion_core::config::welcome_seen() {
        return;
    }

    let win = adw::Window::builder()
        .title("Welcome to Legion Control")
        .modal(true)
        .default_width(600)
        .default_height(440)
        .build();
    let root = parent.as_ref().root();
    if let Some(host) = root.and_then(|r| r.downcast::<gtk::Window>().ok()) {
        win.set_transient_for(Some(&host));
    }

    // ── brand ──
    let brand = gtk::Box::new(Orientation::Horizontal, 16);
    brand.set_valign(Align::Center);
    brand.append(&color_icon(
        include_bytes!("../../data/icons/app-mark.svg"),
        48,
    ));
    let brand_text = gtk::Box::new(Orientation::Vertical, 2);
    let title = gtk::Label::new(Some("Welcome to Legion Control"));
    title.add_css_class("title-2");
    title.set_halign(Align::Start);
    title.set_wrap(true);
    title.set_xalign(0.0);
    let tagline = gtk::Label::new(Some(
        "Unofficial community tool for Lenovo Legion laptops — not affiliated with Lenovo. Use at your own risk.",
    ));
    tagline.add_css_class("page-sub");
    tagline.set_halign(Align::Start);
    tagline.set_wrap(true);
    tagline.set_xalign(0.0);
    brand_text.append(&title);
    brand_text.append(&tagline);
    brand.append(&brand_text);

    let intro = gtk::Label::new(Some(
        "Choose optional components now, or change everything later under Settings → Setup.",
    ));
    intro.set_halign(Align::Start);
    intro.set_wrap(true);
    intro.set_xalign(0.0);

// ── telemetry — one compact switch instead of a wall of buttons ──
    let telemetry_row = adw::SwitchRow::builder()
        .title("Share anonymous diagnostics")
        .subtitle("Anonymized hardware stats · opt out any time")
        .active(legion_core::config::get().diagnostics.enabled)
        .build();
    tip(
        &telemetry_row,
        "One anonymized report per minute: hardware model, distro/kernel, sensors, \
         fan/battery stats, self-check results. NEVER included: hostname, username, \
         serials, MACs, IPs, key colors, custom profile names. \
         Change any time under Settings → Setup.",
    );
    let telemetry_group = pref_group("Alpha telemetry", None);
    telemetry_group.add(&telemetry_row);
    let privacy_link = gtk::LinkButton::with_label(
        "https://github.com/encomjp/Lenovo-Legion-Control#alpha-telemetry-opt-out",
        "Privacy policy — what is sent and what is never collected",
    );
    privacy_link.add_css_class("flat");
    privacy_link.set_halign(Align::Start);
    privacy_link.set_margin_start(4);

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
    let actions = gtk::Box::new(Orientation::Horizontal, 12);
    actions.set_halign(Align::End);
    let later_btn = gtk::Button::with_label("Not now");
    later_btn.add_css_class("pill-btn");
    tip(&later_btn, "Skip for now — nothing else changes");
    let setup_btn = primary_button_tip(
        "First-time setup",
        Some("Guided walkthrough: service, startup & tuning, hardware, self-check"),
    );
    actions.append(&later_btn);
    actions.append(&setup_btn);

    // ── layout ──
    let page = gtk::Box::new(Orientation::Vertical, 22);
    page.set_margin_top(24);
    page.set_margin_bottom(24);
    page.set_margin_start(28);
    page.set_margin_end(28);
    page.append(&brand);
    page.append(&intro);

    // ── what's new — fetched from the GitHub release (silent on failure) ──
    // First-launch context: users see the latest changes without hunting
    // through the releases page.
    let page_whats_new = page.clone();
    {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(legion_core::update::check_latest_release());
        });
        glib::timeout_add_local(Duration::from_millis(300), move || {
            match rx.try_recv() {
                Ok(Ok(info)) => {
                    let highlights: String = info
                        .body
                        .lines()
                        .filter(|l| {
                            let t = l.trim_start();
                            t.starts_with("- ") && !t.starts_with("|")
                        })
                        .take(4)
                        .map(|l| format!("• {}", l.trim_start_matches('-').trim()))
                        .collect::<Vec<_>>()
                        .join("\n");
                    if !highlights.is_empty() {
                        let label = gtk::Label::new(Some(&format!(
                            "What's new in v{}\n{highlights}",
                            info.version
                        )));
                        label.set_halign(Align::Start);
                        label.set_wrap(true);
                        label.set_xalign(0.0);
                        label.set_margin_start(4);
                        label.add_css_class("page-sub");
                        page_whats_new.append(&label);
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(_)) | Err(_) => glib::ControlFlow::Break,
            }
        });
    }

        page.append(&telemetry_group);
    page.append(&privacy_link);
    page.append(&actions);

    let clamp = libadwaita::Clamp::builder().maximum_size(560).build();
    clamp.set_child(Some(&page));
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .child(&clamp)
        .build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&scroll));
    win.set_content(Some(&toolbar));

    // Any close path records the welcome as seen — it shows exactly once.
    win.connect_close_request(|_| {
        legion_core::config::mark_welcome_seen();
        glib::Propagation::Proceed
    });

    let win_later = win.clone();
    later_btn.connect_clicked(move |_| {
        legion_core::config::mark_welcome_seen();
        win_later.close();
    });
    let win_setup = win.clone();
    let stack_c = stack.clone();
    let about_tabs_c = about_tabs.cloned();
    let consent_c = consent.clone();
    let share_c = share_switch.cloned();
    let sync_c = sync.clone();
    setup_btn.connect_clicked(move |_| {
        legion_core::config::mark_welcome_seen();
        win_setup.close();
        run_guided_setup(
            &stack_c,
            about_tabs_c.as_ref(),
            &consent_c,
            share_c.as_ref(),
            &sync_c,
        );
    });

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
        let body = format!("⚠ {problem}\n\n{hint}");
        let dialog = setup_step_dialog(
            step,
            topic,
            &body,
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
                    let dialog = setup_step_dialog(
                        SetupStep::Daemon,
                        "Control service",
                        "✓ Control service is running.\n\n\
                         Fans, power profiles, and charge limits are handled \
                         by the privileged legion-control service.",
                        [("continue", "Continue")],
                        "continue",
                    );
                    self.present(dialog, |_| Some(SetupStep::Startup));
                }
                Err(_) => {
                    let dialog = setup_step_dialog(
                        SetupStep::Daemon,
                        "Control service",
                        "Control service is not running.\n\n\
                         Enable it now? One system prompt will install and start \
                         the privileged service.\n\n\
                         You can continue without it — detection and self-checks work.",
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
                                            let ok_dialog = setup_step_dialog(
                                                SetupStep::Daemon,
                                                "Control service",
                                                "✓ Control service enabled and running.",
                                                [("continue", "Continue")],
                                                "continue",
                                            );
                                            ctx2.present(ok_dialog, |_| Some(SetupStep::Startup));
                                        }
                                        Err(e) => ctx2.retryable_failure(
                                            SetupStep::Daemon,
                                            "Control service",
                                            "Could not enable service.",
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
            "Pick what Legion Control sets up now — change it any time under \
             Settings → Setup.",
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
            "Adds Legion Control to Desktop autostart (~/.config/autostart) and \
             enables the legion-control systemd unit so the hardware daemon also \
             starts on boot.",
        );
        let co_switch = adw::SwitchRow::builder()
            .title("Install AMD tuning backend")
            .subtitle("Curve Optimizer undervolt (ryzen_smu via DKMS)")
            .active(true)
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
                                    report.push("Daemon already enabled for boot".into());
                                } else {
                                    run_setup_helper_blocking("enable-daemon")?;
                                    report.push("Daemon enabled for boot".into());
                                }
                                if !daemon_ok() {
                                    start_legion_control()?;
                                }
                            }
                            if want_co {
                                run_setup_helper_blocking("install-ryzen-smu")?;
                                report.push(
                                    "AMD tuning backend installed — Curve Optimizer is \
                                     on the CPU page"
                                        .into(),
                                );
                            }
                            Ok(report)
                        },
                        "Startup setup stopped without a result",
                        move |result| match result {
                            Ok(report) => {
                                let body = if report.is_empty() {
                                    "Nothing selected — you can change this later \
                                     under Settings → Setup."
                                        .to_string()
                                } else {
                                    format!("✓ Done.\n\n{}", report.join("\n"))
                                };
                                let ok_dialog = setup_step_dialog(
                                    SetupStep::Startup,
                                    "Startup & tuning",
                                    &body,
                                    [("continue", "Continue")],
                                    "continue",
                                );
                                ctx2.present(ok_dialog, |_| Some(SetupStep::Hardware));
                            }
                            Err(e) => ctx2.retryable_failure(
                                SetupStep::Startup,
                                "Startup & tuning",
                                "Setup could not complete.",
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
                        "✓ Hardware detected.\n\n\
                         Model: {}\n\
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
                    let dialog = setup_step_dialog(
                        SetupStep::Hardware,
                        "Your hardware",
                        &body,
                        [("continue", "Continue")],
                        "continue",
                    );
                    self.present(dialog, |_| Some(SetupStep::SelfCheck));
                }
                Err(error) => self.retryable_failure(
                    SetupStep::Hardware,
                    "Your hardware",
                    "Hardware detection failed.",
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
                    let mut lines =
                        vec![format!("✓ Self-check finished: {passed} / {total} passed.")];

                    let criticals: Vec<_> = faults
                        .iter()
                        .filter(|f| f.severity == legion_core::selftest::Severity::Critical)
                        .collect();
                    if criticals.is_empty() {
                        lines.push("No critical faults found.".into());
                    } else {
                        lines.push(format!("⚠ {} critical fault(s):", criticals.len()));
                        const MAX_LISTED: usize = 6;
                        for fault in criticals.iter().take(MAX_LISTED) {
                            lines.push(format!("⚠ {}", fault.detail));
                        }
                        let hidden = criticals.len().saturating_sub(MAX_LISTED);
                        if hidden > 0 {
                            lines.push(format!("…and {hidden} more"));
                        }
                    }

                    let body = lines.join("\n");
                    let dialog = setup_step_dialog(
                        SetupStep::SelfCheck,
                        "Self-check",
                        &body,
                        [("continue", "Continue")],
                        "continue",
                    );
                    self.present(dialog, |_| Some(SetupStep::Done));
                }
                Err(error) => self.retryable_failure(
                    SetupStep::SelfCheck,
                    "Self-check",
                    "Self-check could not run.",
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
        let dialog = setup_step_dialog(
            SetupStep::Done,
            "All done",
            "✓ Setup complete.\n\nYou can change all of these later under Setup.",
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
        "First-time setup ({}/{}) — {topic}",
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
