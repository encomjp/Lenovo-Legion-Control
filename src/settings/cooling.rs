//! Cooling — fan-curve editor, per-fan cards and manual control.

use super::curve_editor::CurveEditor;
use super::*;
use std::collections::HashMap;

pub(crate) fn build_cooling_overview_page(
    toast_overlay: &adw::ToastOverlay,
    apply_queue: &ApplyQueue,
    gate: &DaemonGate,
) -> gtk::Box {
    let page = page_lede("");
    // LOQ 15AHP10 / R8CN — tachometer locked behind yogafan
    if legion_core::fans::needs_yogafan_setup() {
        let locked = pref_group("Fans locked — driver required", None);
        tip(
            &locked,
            "lenovo_wmi_other binds but reads 0 RPM on this model; yogafan provides the real tachometer",
        );
        let banner = adw::Banner::new(
            "Fan sensors need the yogafan driver — install to unlock RPM reading and control",
        );
        banner.set_button_label(Some("Unlock fans"));
        locked.add(&banner);
        let hint = gtk::Label::new(Some(
            "On LOQ 15AHP10 (83JG) the EC tachometer is behind the yogafan ACPI driver. One password prompt loads it and enables it at boot.",
        ));
        hint.set_wrap(true);
        hint.add_css_class("dim-label");
        hint.set_xalign(0.0);
        hint.set_margin_start(12);
        hint.set_margin_end(12);
        locked.add(&hint);
        // Show grayed-out fan cards so layout stays stable
        for ch in &legion_core::fans::channels() {
            let card = fan_card(
                ch.id,
                &ch.title,
                ch.min_rpm as f64,
                ch.max_rpm as f64,
                toast_overlay,
                apply_queue,
            );
            card.set_sensitive(false);
            card.add_css_class("dim-label");
            locked.add(&card);
        }
        let overlay = toast_overlay.clone();
        let banner_c = banner.clone();
        let locked_c = locked.clone();
        banner.connect_button_clicked(move |_| {
            banner_c.set_sensitive(false);
            banner_c.set_title("Installing…");
            let overlay = overlay.clone();
            let banner_c = banner_c.clone();
            let locked_c = locked_c.clone();
            run_setup_helper("install-yogafan", move |result| match result {
                Ok(_) => {
                    toast_ok(&overlay, "Fans unlocked — reopen the Cooling page to see RPM");
                    banner_c.set_title("Installed — reopen this page");
                    locked_c.set_sensitive(false);
                }
                Err(e) => {
                    toast_error(&overlay, &e);
                    banner_c.set_sensitive(true);
                    banner_c.set_title(
                        "Fan sensors need the yogafan driver — install to unlock RPM reading and control",
                    );
                }
            });
        });
        page.append(&locked);
        // Also try silent daemon-side load in background
        std::thread::spawn(|| {
            let _ = legion_core::fans::ensure_yogafan_loaded();
        });
        gate.track(&locked);
        return page;
    }
    let channels = legion_core::fans::channels();
    let manual_supported = legion_core::fans::control_backend_name().is_some();
    if !manual_supported {
        let overview = pref_group("Fans overview", None);
        tip(&overview, "Each card exposes full tuning for that fan");
        for ch in &channels {
            overview.add(&fan_card(
                ch.id,
                &ch.title,
                ch.min_rpm as f64,
                ch.max_rpm as f64,
                toast_overlay,
                apply_queue,
            ));
        }
        if channels.is_empty() {
            overview.add(&property_row(
                "No fans detected",
                "Check the daemon and HWMon",
                Some("Fan channels missing — check legion-control service"),
            ));
        }
        let note = pref_group("Manual control unavailable", None);
        tip(
            &note,
            "This model exposes fan tachometers but no firmware write path for manual targets",
        );
        note.add(&property_row(
            "Managed by Lenovo EC firmware",
            "Manual fan control is not supported on this model (managed by Lenovo EC firmware)",
            Some("Fan RPM is shown live above; the EC firmware owns the fan curve"),
        ));
        page.append(&overview);
        page.append(&note);
        gate.track(&overview);
        gate.track(&note);
        return page;
    }

    // ── Fan curve (primary) ──
    let curve_box = build_fan_curve_section(&channels, toast_overlay, gate);
    page.append(&curve_box);

    // ── Advanced: manual speed (existing per-fan controls) ──
    let advanced = gtk::Expander::new(Some("Advanced"));
    advanced.add_css_class("section-expander");
    advanced.set_margin_top(6);
    let inner = gtk::Box::new(Orientation::Vertical, 12);
    inner.set_margin_top(10);
    let hint = gtk::Label::new(Some(
        "Fixed speeds only apply while no fan curve is running.",
    ));
    hint.set_wrap(true);
    hint.set_xalign(0.0);
    hint.add_css_class("dim-label");
    inner.append(&hint);
    let overview = pref_group("Fans overview", None);
    tip(&overview, "Each card exposes full tuning for that fan");
    for ch in &channels {
        overview.add(&fan_card(
            ch.id,
            &ch.title,
            ch.min_rpm as f64,
            ch.max_rpm as f64,
            toast_overlay,
            apply_queue,
        ));
    }
    if channels.is_empty() {
        overview.add(&property_row(
            "No fans detected",
            "Check the daemon and HWMon",
            Some("Fan channels missing — check legion-control service"),
        ));
    }
    let reset = pref_group("Automatic mode", None);
    let btn = secondary_button_tip(
        "All fans automatic",
        Some("Clears manual RPM on all detected fans — returns to the firmware fan curve"),
    );
    let queue = apply_queue.clone();
    let fan_ids: Vec<u8> = channels.iter().map(|c| c.id).collect();
    btn.connect_clicked(move |_| {
        for fan in &fan_ids {
            queue.set_fan(*fan, 0);
        }
    });
    let row = adw::ActionRow::builder()
        .title("Reset all")
        .activatable(false)
        .build();
    tip(&row, "Recommended after testing loud manual speeds");
    row.add_suffix(&btn);
    reset.add(&row);
    inner.append(&overview);
    inner.append(&reset);
    advanced.set_child(Some(&inner));
    page.append(&advanced);
    gate.track(&overview);
    gate.track(&reset);
    page
}

/// One-line Gen 10 reminder shown only when a manual speed is set.
const GEN10_MANUAL_HINT: &str = "This speed can stay set until you switch back to Automatic.";

/// Show / clear the Gen 10 reminder on a fan's mode row. No-op below Gen 10.
fn gen10_manual_hint(row: &adw::SwitchRow, manual: bool, toast: Option<&adw::ToastOverlay>) {
    if !legion_core::fans::manual_target_may_persist() {
        return;
    }
    if manual {
        row.set_subtitle(GEN10_MANUAL_HINT);
        if let Some(t) = toast {
            toast_info(t, GEN10_MANUAL_HINT);
        }
    } else {
        row.set_subtitle("");
    }
}

fn profile_tab_label(name: &str) -> String {
    match name {
        "low-power" | "quiet" => "Quiet".into(),
        "balanced" => "Balanced".into(),
        "balanced-performance" => "Balanced+".into(),
        "performance" => "Performance".into(),
        "max-power" => "Max power".into(),
        "custom" => "Custom".into(),
        other => {
            let mut c = other.chars();
            c.next()
                .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
        }
    }
}

/// Everything the curve section updates after a daemon round trip.
struct CurveUi {
    editor: CurveEditor,
    profile: RefCell<String>,
    status: RefCell<Option<legion_core::fan_curve::FanCurveStatus>>,
    drafts: RefCell<HashMap<String, legion_core::fan_curve::CurvePoints>>,
    inflight: Cell<bool>,
    busy: Cell<bool>,
    headline: gtk::Label,
    state_label: gtk::Label,
    reason: gtk::Label,
    state_dot: Option<gtk::Widget>,
    apply: gtk::Button,
    reset: gtk::Button,
    factory: gtk::Button,
    cards: Vec<FanCardUi>,
    toast: adw::ToastOverlay,
}

struct FanCardUi {
    id: u8,
    min: u32,
    max: u32,
    rpm: gtk::Label,
    bar: gtk::ProgressBar,
    source: gtk::Label,
}

impl CurveUi {
    /// Curve stored on the daemon for `profile` (factory shape if none).
    fn stored(&self, profile: &str) -> (legion_core::fan_curve::CurvePoints, bool) {
        if let Some(st) = self.status.borrow().as_ref() {
            if let Some((_, c)) = st.curves.iter().find(|(n, _)| n == profile) {
                return (c.points, c.enabled);
            }
        }
        (legion_core::fan_curve::default_points(profile), false)
    }

    fn dirty(&self) -> bool {
        let profile = self.profile.borrow().clone();
        self.editor.points() != self.stored(&profile).0
    }

    /// Refresh labels, buttons and fan cards from the current state.
    fn sync(&self) {
        let profile = self.profile.borrow().clone();
        let (_, enabled) = self.stored(&profile);
        let dirty = self.dirty();
        let st = self.status.borrow();
        let mut kind = "";
        let text = match st.as_ref() {
            None => "Connecting to the service…".to_string(),
            Some(st) => {
                let now = st.current_profile == profile;
                let base = if enabled && now && st.active {
                    let t = st.temp_c.map_or("—".into(), |t| format!("{t:.0} °C"));
                    let p = st.pct.map_or("—".into(), |p| format!("{p}%"));
                    if st.safety {
                        kind = "bad";
                        format!("Safety max at {t}: all fans at 100%")
                    } else {
                        kind = "ok";
                        format!("Following the curve: {t}, fans at {p}")
                    }
                } else if enabled && now {
                    match &st.fault {
                        Some(f) => format!("Firmware curve in control: {f}"),
                        None => "Curve enabled, starting…".to_string(),
                    }
                } else if enabled {
                    kind = "ok";
                    "Curve saved. It runs while this power mode is active".to_string()
                } else {
                    "Firmware curve in control".to_string()
                };
                if dirty {
                    kind = "warn";
                    format!("{base}. Unsaved changes")
                } else {
                    base
                }
            }
        };
        if let Some(dot) = &self.state_dot {
            for k in ["ok", "warn", "bad"] {
                dot.remove_css_class(k);
            }
            if !kind.is_empty() {
                dot.add_css_class(kind);
            }
        }
        self.state_label.set_text(&text);
        let online = st.is_some() && !self.busy.get();
        let can_factory =
            st.is_some() && self.editor.points() != legion_core::fan_curve::default_points(&profile);
        self.apply.set_sensitive(online && (dirty || !enabled));
        self.reset.set_sensitive(online && enabled);
        self.factory.set_sensitive(can_factory);
        // Say why a button is greyed out instead of leaving it a mystery.
        let mut why = Vec::new();
        if st.is_some() && !can_factory {
            why.push("Already the factory shape");
        }
        if st.is_some() && !enabled {
            why.push("No custom curve to reset");
        }
        self.reason.set_text(&why.join(" · "));
        self.reason.set_visible(!why.is_empty());
        self.headline
            .set_text(&format!("Fan curve for {}", profile_tab_label(&profile)));
        self.editor.set_now(
            st.as_ref()
                .filter(|s| s.current_profile == profile)
                .and_then(|s| s.temp_c),
        );
        // The status line already says when the firmware is in charge, so a
        // card only speaks up while the curve is actually driving its fan.
        let following = st.as_ref().filter(|s| s.active).and_then(|s| s.pct);
        for c in &self.cards {
            c.source.set_visible(following.is_some());
            if let Some(p) = following {
                c.source.set_text(&format!("Following the curve, {p}%"));
            }
        }
    }

    fn load_profile(&self, profile: &str) {
        let pts = self
            .drafts
            .borrow()
            .get(profile)
            .copied()
            .unwrap_or_else(|| self.stored(profile).0);
        *self.profile.borrow_mut() = profile.to_string();
        self.editor.set_points(pts);
        self.sync();
    }
}

fn fan_card_ui(ch: &legion_core::device::FanCapability) -> (gtk::Box, FanCardUi) {
    let card = gtk::Box::new(Orientation::Vertical, 6);
    card.add_css_class("panel");
    card.add_css_class("fan-card");
    card.set_hexpand(true);
    card.set_valign(Align::Fill);
    tip(&card, &format!("{} · {}–{} rpm", ch.title, ch.min_rpm, ch.max_rpm));
    let top = gtk::Box::new(Orientation::Horizontal, 8);
    let title = gtk::Label::new(Some(&ch.title));
    title.add_css_class("row-title");
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let rpm = gtk::Label::new(Some("— rpm"));
    rpm.add_css_class("numeric");
    rpm.add_css_class("fan-rpm");
    rpm.set_xalign(1.0);
    top.append(&title);
    top.append(&rpm);
    let bar = gtk::ProgressBar::new();
    bar.add_css_class("card-bar");
    bar.set_opacity(0.0);
    let source = gtk::Label::new(None);
    source.set_visible(false);
    source.add_css_class("row-sub");
    source.set_xalign(0.0);
    source.set_ellipsize(gtk::pango::EllipsizeMode::End);
    card.append(&top);
    card.append(&bar);
    card.append(&source);
    let ui = FanCardUi {
        id: ch.id,
        min: ch.min_rpm,
        max: ch.max_rpm,
        rpm,
        bar,
        source,
    };
    (card, ui)
}

fn build_fan_curve_section(
    channels: &[legion_core::device::FanCapability],
    toast: &adw::ToastOverlay,
    gate: &DaemonGate,
) -> gtk::Box {
    use legion_core::fan_curve;
    let root = gtk::Box::new(Orientation::Vertical, 10);

    // Mode selector (power profile whose curve is edited).
    let mut choices = legion_core::profile::safe_choices();
    if choices.is_empty() {
        choices = ["low-power", "balanced", "performance", "custom"]
            .iter()
            .map(|s| s.to_string())
            .collect();
    }
    let current = legion_core::profile::current();
    let initial = if choices.contains(&current) {
        current.clone()
    } else {
        choices[0].clone()
    };

    let header = gtk::Box::new(Orientation::Horizontal, 12);
    let headline = gtk::Label::new(None);
    headline.add_css_class("section-title");
    headline.set_xalign(0.0);
    headline.set_hexpand(true);
    headline.set_wrap(true);
    header.append(&headline);
    let modes = gtk::Box::new(Orientation::Horizontal, 0);
    modes.add_css_class("linked");
    modes.add_css_class("segmented");
    modes.set_valign(Align::Center);
    modes.update_property(&[gtk::accessible::Property::Label("Curve for power mode")]);
    let mut leader: Option<gtk::ToggleButton> = None;
    let mut mode_buttons: Vec<(String, gtk::ToggleButton)> = Vec::new();
    for name in &choices {
        let b = gtk::ToggleButton::with_label(&profile_tab_label(name));
        if let Some(l) = &leader {
            b.set_group(Some(l));
        } else {
            leader = Some(b.clone());
        }
        b.set_active(*name == initial);
        modes.append(&b);
        mode_buttons.push((name.clone(), b));
    }
    header.append(&modes);
    root.append(&header);

    // Editor + status + buttons.
    let editor = CurveEditor::new();
    let editor_frame = gtk::Box::new(Orientation::Vertical, 8);
    editor_frame.add_css_class("panel");
    editor_frame.set_hexpand(true);
    editor.area.set_margin_top(2);
    editor_frame.append(&editor.area);
    let (state_row, state_label) = status_line("", "");
    state_row.set_valign(Align::Center);
    let reason = gtk::Label::new(None);
    reason.add_css_class("row-sub");
    reason.set_xalign(0.0);
    reason.set_wrap(true);
    reason.set_margin_start(14);
    reason.set_visible(false);
    let state_col = gtk::Box::new(Orientation::Vertical, 2);
    state_col.set_valign(Align::Center);
    state_col.set_hexpand(true);
    state_col.append(&state_row);
    state_col.append(&reason);
    let buttons = gtk::Box::new(Orientation::Horizontal, 8);
    buttons.set_halign(Align::End);
    let factory = secondary_button_tip(
        "Factory shape",
        Some("Put this mode's points back to the built-in shape (not applied until you press Apply curve)"),
    );
    let reset = secondary_button_tip(
        "Reset to firmware",
        Some("Stop the software curve for this power mode and give the fans back to the firmware"),
    );
    let apply = primary_button_tip(
        "Apply curve",
        Some("Save this curve and use it to drive the fans"),
    );
    buttons.append(&factory);
    buttons.append(&reset);
    buttons.append(&apply);
    let action_row = gtk::Box::new(Orientation::Horizontal, 12);
    state_row.set_hexpand(true);
    action_row.append(&state_col);
    action_row.append(&buttons);
    editor_frame.append(&action_row);
    tip(
        &editor_frame,
        "Above 90 °C every fan goes to 100% regardless of the curve. Any sensor error hands the fans back to the firmware.",
    );
    let state_dot = state_row.first_child();

    // Fan cards.
    let cards_box = gtk::Box::new(Orientation::Horizontal, 10);
    cards_box.set_homogeneous(true);
    cards_box.set_valign(Align::Start);
    let mut card_uis = Vec::new();
    for ch in channels {
        let (w, ui) = fan_card_ui(ch);
        cards_box.append(&w);
        card_uis.push(ui);
    }
    root.append(&editor_frame);
    root.append(&cards_box);
    gate.track(&editor_frame);

    let ui = Rc::new(CurveUi {
        editor: editor.clone(),
        profile: RefCell::new(initial.clone()),
        status: RefCell::new(None),
        drafts: RefCell::new(HashMap::new()),
        inflight: Cell::new(false),
        busy: Cell::new(false),
        headline,
        state_label,
        reason,
        state_dot,
        apply: apply.clone(),
        reset: reset.clone(),
        factory: factory.clone(),
        cards: card_uis,
        toast: toast.clone(),
    });
    editor.set_points(fan_curve::default_points(&initial));
    ui.sync();

    // Edits update the draft for the selected mode and the dirty state.
    {
        let ui = ui.clone();
        editor.connect_changed(move || {
            let profile = ui.profile.borrow().clone();
            ui.drafts.borrow_mut().insert(profile, ui.editor.points());
            ui.sync();
        });
    }
    // Mode switch.
    for (name, b) in &mode_buttons {
        let ui = ui.clone();
        let name = name.clone();
        b.connect_toggled(move |b| {
            if b.is_active() {
                ui.load_profile(&name);
            }
        });
    }
    {
        let ui = ui.clone();
        factory.connect_clicked(move |_| {
            let profile = ui.profile.borrow().clone();
            ui.editor.set_points(fan_curve::default_points(&profile));
            ui.drafts.borrow_mut().insert(profile, ui.editor.points());
            ui.sync();
        });
    }
    // Apply / Reset (async IPC — GTK main thread never blocks).
    let send = {
        let ui = ui.clone();
        Rc::new(move |enabled: bool| {
            let profile = ui.profile.borrow().clone();
            let points = if enabled {
                ui.editor.points()
            } else {
                ui.stored(&profile).0
            };
            ui.busy.set(true);
            ui.sync();
            let ui2 = ui.clone();
            let p2 = profile.clone();
            run_daemon_command_async(
                DaemonCommand::SetFanCurve {
                    profile,
                    points,
                    enabled,
                },
                move |res| {
                    ui2.busy.set(false);
                    match res {
                        Ok(DaemonResponse::FanCurves(st)) => {
                            *ui2.status.borrow_mut() = Some(st);
                            if enabled {
                                ui2.drafts.borrow_mut().remove(&p2);
                                toast_ok(&ui2.toast, "Fan curve applied");
                            } else {
                                toast_ok(&ui2.toast, "Fans back on the firmware curve");
                            }
                        }
                        Ok(DaemonResponse::Error(e)) | Err(e) => toast_error(&ui2.toast, &e),
                        Ok(_) => toast_error(&ui2.toast, "Unexpected response from the daemon"),
                    }
                    ui2.sync();
                },
            );
        })
    };
    {
        let send = send.clone();
        apply.connect_clicked(move |_| send(true));
    }
    {
        let send = send.clone();
        reset.connect_clicked(move |_| send(false));
    }

    // Poll status + RPM every 2 s while the page is visible.
    let weak = editor.area.downgrade();
    let ui_poll = ui.clone();
    let tick = move || {
        let ui = ui_poll.clone();
        if ui.inflight.get() {
            return;
        }
        ui.inflight.set(true);
        let fans: Vec<u8> = ui.cards.iter().map(|c| c.id).collect();
        dispatch_async(
            move || {
                let st = match send_command(DaemonCommand::GetFanCurves)? {
                    DaemonResponse::FanCurves(s) => s,
                    DaemonResponse::Error(e) => {
                        return Err(format!(
                            "Fan curves unavailable ({e}) — update and restart legion-control"
                        ))
                    }
                    _ => return Err("Unexpected response from the daemon".into()),
                };
                let rpms: Vec<(u8, Option<u32>)> = fans
                    .iter()
                    .map(|f| (*f, legion_core::fans::read_rpm(*f)))
                    .collect();
                Ok((st, rpms))
            },
            "Fan status request stopped",
            move |res| {
                ui.inflight.set(false);
                match res {
                    Ok((st, rpms)) => {
                        *ui.status.borrow_mut() = Some(st);
                        for (id, rpm) in rpms {
                            if let Some(c) = ui.cards.iter().find(|c| c.id == id) {
                                match rpm {
                                    Some(r) => {
                                        // 0 rpm under firmware control is the EC's fan
                                        // stop, not a dead fan.
                                        c.rpm.set_text(&if r == 0 {
                                            "Idle".to_string()
                                        } else {
                                            format!("{r} rpm")
                                        });
                                        c.bar.set_opacity(if r == 0 { 0.0 } else { 1.0 });
                                        let span = c.max.saturating_sub(c.min).max(1) as f64;
                                        c.bar.set_fraction(
                                            (r.saturating_sub(c.min) as f64 / span).clamp(0.0, 1.0),
                                        );
                                    }
                                    None => c.rpm.set_text("— rpm"),
                                }
                            }
                        }
                        // Pick up CLI / other-client changes unless mid-edit.
                        let profile = ui.profile.borrow().clone();
                        if !ui.editor.is_dragging() && !ui.drafts.borrow().contains_key(&profile) {
                            ui.editor.set_points(ui.stored(&profile).0);
                        }
                        ui.sync();
                    }
                    Err(e) => {
                        ui.state_label.set_text(&e);
                        ui.apply.set_sensitive(false);
                        ui.reset.set_sensitive(false);
                    }
                }
            },
        );
    };
    tick();
    let tick = Rc::new(tick);
    glib::timeout_add_local(Duration::from_secs(2), move || {
        let Some(area) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if area.is_mapped() {
            tick();
        }
        glib::ControlFlow::Continue
    });

    root
}

pub(crate) fn fan_card(
    fan: u8,
    title: &str,
    min_rpm: f64,
    max_rpm: f64,
    toast_overlay: &adw::ToastOverlay,
    apply_queue: &ApplyQueue,
) -> adw::PreferencesGroup {
    let sec_tip = match fan {
        1 => "CPU cooling fan",
        2 => "Discrete GPU cooling fan",
        3 | 4 => "Auxiliary chassis fan",
        _ => "Fan control for this channel",
    };
    let group = pref_group(title, None);
    tip(&group, sec_tip);

    // Models without a fan-target write backend (e.g. Y7000P 16IRX9 / 83DG)
    // are EC-owned: show RPM read-only instead of a manual switch/slider
    // that could only ever fail with "no fan backend/device for target write".
    if legion_core::fans::control_backend_name().is_none() {
        let rpm_l = gtk::Label::new(Some(&legion_core::fans::rpm_label(fan)));
        rpm_l.add_css_class("numeric");
        rpm_l.add_css_class("fan-rpm");
        tip(
            &rpm_l,
            "Live speed from the firmware. May read Auto or 0 while it drives the fan",
        );
        let mode_row = adw::ActionRow::builder()
            .title("Auto (EC controlled)")
            .activatable(false)
            .build();
        tip(
            &mode_row,
            "The EC firmware owns the fan curve on this model — no manual target write path is exposed",
        );
        mode_row.add_suffix(&rpm_l);
        group.add(&mode_row);
        let note_row = adw::ActionRow::builder()
            .title("Manual control unavailable")
            .subtitle("Manual fan control is not supported on this model (managed by Lenovo EC firmware)")
            .activatable(false)
            .build();
        tip(
            &note_row,
            "Fan tachometer is read-only here; the EC firmware drives the curve",
        );
        group.add(&note_row);
        // Keep the RPM label live even in read-only mode.
        glib::timeout_add_local(Duration::from_secs(2), move || {
            let (tx, rx) = mpsc::channel();
            let fan = fan;
            std::thread::spawn(move || {
                let _ = tx.send(legion_core::fans::rpm_label(fan));
            });
            let rpm_l_c = rpm_l.clone();
            glib::timeout_add_local(Duration::from_millis(50), move || match rx.try_recv() {
                Ok(text) => {
                    rpm_l_c.set_text(&text);
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(_) => glib::ControlFlow::Break,
            });
            glib::ControlFlow::Continue
        });
        return group;
    }

    let auto = legion_core::fans::display_target(fan) == 0;
    let sw = adw::SwitchRow::builder()
        .title(if auto { "Automatic" } else { "Manual" })
        .active(auto)
        .build();
    tip(
        &sw,
        "On = Automatic firmware curve · Off = Manual fixed RPM with the slider",
    );
    group.add(&sw);
    if !auto {
        // Already manual when the page opens: keep the reminder visible.
        gen10_manual_hint(&sw, true, None);
    }

    let scale = gtk::Scale::with_range(Orientation::Horizontal, min_rpm, max_rpm, 100.0);
    scale.set_draw_value(false);
    scale.set_digits(0);
    scale.set_hexpand(true);
    scale.set_width_request(180);
    let speed_val = gtk::Label::new(Some("Auto"));
    speed_val.add_css_class("numeric");
    speed_val.add_css_class("scale-value");
    speed_val.set_width_chars(9);
    speed_val.set_xalign(1.0);
    tip(
        &scale,
        &format!(
            "Target RPM for {title} · enabled only in Manual · about {min_rpm:.0}–{max_rpm:.0}"
        ),
    );
    let cur = legion_core::fans::display_target(fan);
    if cur > 0 {
        scale.set_value(cur as f64);
        scale.set_sensitive(true);
        speed_val.set_text(&format!("~{cur} rpm"));
    } else {
        scale.set_value(min_rpm);
        scale.set_sensitive(false);
    }

    let speed_row = adw::ActionRow::builder()
        .title("Speed")
        .activatable(false)
        .build();
    tip(
        &speed_row,
        "Automatic follows temperature · Manual holds a fixed speed until you change it",
    );
    speed_row.add_suffix(&scale);
    speed_row.add_suffix(&speed_val);
    group.add(&speed_row);

    let high_accepted = Rc::new(Cell::new(false));
    let scale_s = scale.clone();
    let speed_val_s = speed_val.clone();
    let sw_title = sw.clone();
    let queue = apply_queue.clone();
    let suppressing = Rc::new(Cell::new(false));
    let suppressing_s = suppressing.clone();
    let high_s = high_accepted.clone();
    let toast_s = toast_overlay.clone();
    sw.connect_active_notify(move |s| {
        if suppressing_s.get() {
            return;
        }
        if s.is_active() {
            scale_s.set_sensitive(false);
            speed_val_s.set_text("Auto");
            sw_title.set_title("Automatic");
            gen10_manual_hint(&sw_title, false, None);
            queue.set_fan(fan, 0);
        } else {
            let rpm = scale_s.value() as u32;
            let warn_rpm = (max_rpm * 0.85).round() as u32;
            if rpm >= warn_rpm && !high_s.get() {
                let scale_r = scale_s.clone();
                let speed_val_r = speed_val_s.clone();
                let sw_r = s.clone();
                let sw_title = sw_title.clone();
                let queue = queue.clone();
                let suppressing = suppressing_s.clone();
                let high_s = high_s.clone();
                let toast_r = toast_s.clone();
                confirm_risk(
                    s,
                    "Very high fan speed",
                    &format!(
                        "Manual {rpm} RPM is near the maximum for this fan (~{max_rpm:.0}).\n\n\
                         This holds the fan near its maximum speed and increases noise and power use."
                    ),
                    "Keep manual",
                    move |ok| {
                        if !ok {
                            suppressing.set(true);
                            sw_r.set_active(true);
                            scale_r.set_sensitive(false);
                            speed_val_r.set_text("Auto");
                            sw_title.set_title("Automatic");
                                suppressing.set(false);
                            return;
                        }
                        high_s.set(true);
                        scale_r.set_sensitive(true);
                        speed_val_r.set_text(&format!("~{rpm} rpm"));
                        sw_title.set_title("Manual");
                        gen10_manual_hint(&sw_title, true, Some(&toast_r));
                        queue.set_fan(fan, rpm);
                    },
                );
                return;
            }
            scale_s.set_sensitive(true);
            speed_val_s.set_text(&format!("~{rpm} rpm"));
            sw_title.set_title("Manual");
            gen10_manual_hint(&sw_title, true, Some(&toast_s));
            queue.set_fan(fan, rpm);
        }
    });

    let sw_sc = sw.clone();
    let suppressing_sc = suppressing.clone();
    let queue_sc = apply_queue.clone();
    let high_sc = high_accepted.clone();
    let speed_val_sc = speed_val.clone();
    scale.connect_value_changed(move |sc| {
        speed_val_sc.set_text(&format!("~{} rpm", sc.value() as u32));
        if sw_sc.is_active() || suppressing_sc.get() {
            return;
        }
        let rpm = sc.value() as u32;
        let warn_rpm = (max_rpm * 0.85).round() as u32;
        if rpm >= warn_rpm && !high_sc.get() {
            let sc_r = sc.clone();
            let queue = queue_sc.clone();
            let suppressing = suppressing_sc.clone();
            let high_sc = high_sc.clone();
            let safe = (max_rpm * 0.7).round() as u32;
            confirm_risk(
                sc,
                "Very high fan speed",
                &format!(
                    "Manual {rpm} RPM is near the maximum for this fan (~{max_rpm:.0}).\n\n\
                     This holds the fan near its maximum speed and increases noise and power use."
                ),
                "Use high speed",
                move |ok| {
                    if !ok {
                        suppressing.set(true);
                        sc_r.set_value(safe as f64);
                        suppressing.set(false);
                        queue.set_fan(fan, safe);
                        return;
                    }
                    high_sc.set(true);
                    queue.set_fan(fan, rpm);
                },
            );
            return;
        }
        queue_sc.set_fan(fan, rpm);
    });

    group
}

// ─── Power ──────────────────────────────────────────────────────────────────
