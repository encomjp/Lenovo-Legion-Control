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
        "Choose optional components now, or change everything later under About → Setup.",
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
         Change any time under About → Setup.",
    );
    let telemetry_group = pref_group("Alpha telemetry", None);
    telemetry_group.add(&telemetry_row);
    let privacy_link = gtk::LinkButton::with_label(
        "https://github.com/encomjp/lenovo-legion-tool#alpha-telemetry-opt-out",
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
        Some("Guided walkthrough: service, hardware, self-check, telemetry"),
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
    /// Probe the privileged control service over IPC.
    Daemon,
    /// Identify model, machine type, CPU, GPU, and fan channels.
    Hardware,
    /// Read-only self-checks plus the fault scan.
    SelfCheck,
    /// Opt-out choice for anonymous diagnostics.
    Telemetry,
    /// Summary; closing returns to the main view.
    Done,
}

impl SetupStep {
    /// 1-based position for dialog titles ("First-time setup (2/5) — …").
    fn number(self) -> usize {
        match self {
            SetupStep::Daemon => 1,
            SetupStep::Hardware => 2,
            SetupStep::SelfCheck => 3,
            SetupStep::Telemetry => 4,
            SetupStep::Done => 5,
        }
    }

    /// The step that follows this one (Done is its own successor).
    fn next(self) -> Self {
        match self {
            SetupStep::Daemon => SetupStep::Hardware,
            SetupStep::Hardware => SetupStep::SelfCheck,
            SetupStep::SelfCheck => SetupStep::Telemetry,
            SetupStep::Telemetry | SetupStep::Done => SetupStep::Done,
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
/// setup" response: five chained alert dialogs (service probe, hardware
/// identity, self-check, telemetry opt-in, summary). Every probe runs on a
/// `dispatch_async` worker thread; each dialog appears once its result is in.
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
            SetupStep::Hardware => self.hardware_step(),
            SetupStep::SelfCheck => self.selfcheck_step(),
            SetupStep::Telemetry => self.telemetry_step(),
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

    /// Step 1 — is the privileged control service reachable?
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
                    self.present(dialog, |_| Some(SetupStep::Hardware));
                }
                Err(_) => self.retryable_failure(
                    SetupStep::Daemon,
                    "Control service",
                    "Control service is not running.",
                    "Run:\nsudo systemctl enable --now legion-control\n\n\
                     You can continue without it — detection and self-checks work.",
                ),
            },
        );
    }

    /// Step 2 — what machine is this?
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

    /// Step 3 — read-only health checks plus anomaly scan.
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
                    self.present(dialog, |_| Some(SetupStep::Telemetry));
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

    /// Step 4 — telemetry opt-in. ON by default; Keep is the suggested
    /// red/destructive top button and links to the privacy policy.
    fn telemetry_step(self) {
        let dialog = setup_step_dialog(
            SetupStep::Telemetry,
            "Anonymous diagnostics",
            "ON by default: one anonymized report per minute.\n\n\
             Included: hardware model, distro/kernel, sensors,\n\
             fan/battery stats, self-check results.\n\
             Never: hostname · username · serials · MACs · IPs · key colors.\n\n\
             Turn it off now to opt out, or later under Setup → Alpha diagnostics.",
            [("keep", "Keep on"), ("optout", "Opt out")],
            "keep",
        );
        // Keep ON — red (destructive) and still the default / top button.
        dialog.set_response_appearance("keep", adw::ResponseAppearance::Destructive);
        let policy = gtk::LinkButton::with_label(
            "https://github.com/encomjp/lenovo-legion-tool#alpha-telemetry-opt-out",
            "Privacy policy",
        );
        policy.add_css_class("flat");
        let extra = gtk::Box::new(Orientation::Horizontal, 0);
        extra.set_halign(Align::Center);
        extra.append(&policy);
        dialog.set_extra_child(Some(&extra));
        let ctx = self.clone();
        self.present(dialog, move |response| match response {
            "optout" => {
                // Nudge before actually opting out — telemetry stays on unless
                // they explicitly confirm. Route to Done only after the nudge settles.
                let ctx2 = ctx.clone();
                let win = ctx2.win.clone();
                confirm_disable_telemetry(win.as_ref(), move |confirmed| {
                    if confirmed {
                        ctx2.disable_telemetry();
                    }
                    ctx2.run(SetupStep::Done);
                });
                None
            }
            _ => Some(SetupStep::Done),
        });
    }

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
    /// button keeps the old About → Setup shortcut within reach.
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
