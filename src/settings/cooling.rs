//! Cooling — per-fan cards with automatic/manual control.

use super::*;

pub(crate) fn build_cooling_overview_page(
    toast_overlay: &adw::ToastOverlay,
    apply_queue: &ApplyQueue,
    gate: &DaemonGate,
) -> gtk::Box {
    let page = page_lede("");
    let overview = pref_group("Fans overview", None);
    tip(&overview, "Each card exposes full tuning for that fan");
    let channels = legion_core::fans::channels();
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
    let btn = primary_button_tip(
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
    page.append(&overview);
    page.append(&reset);
    gate.track(&overview);
    gate.track(&reset);
    page
}

pub(crate) fn fan_card(
    fan: u8,
    title: &str,
    min_rpm: f64,
    max_rpm: f64,
    _toast_overlay: &adw::ToastOverlay,
    apply_queue: &ApplyQueue,
) -> adw::PreferencesGroup {
    let sec_tip = match fan {
        1 => "CPU cooling fan",
        2 => "Discrete GPU cooling fan",
        4 => "Auxiliary chassis fan",
        _ => "Fan control for this channel",
    };
    let group = pref_group(title, None);
    tip(&group, sec_tip);

    let rpm_l = gtk::Label::new(Some(&legion_core::fans::rpm_label(fan)));
    rpm_l.add_css_class("numeric");
    rpm_l.add_css_class("fan-rpm");
    tip(
        &rpm_l,
        "Live RPM from the EC — may show Auto or 0 while firmware is driving the fan",
    );

    let auto = legion_core::fans::read_target(fan).unwrap_or(0) == 0;
    let sw = adw::SwitchRow::builder()
        .title(if auto { "Automatic" } else { "Manual" })
        .active(auto)
        .build();
    tip(
        &sw,
        "On = Automatic firmware curve · Off = Manual fixed RPM with the slider",
    );
    sw.add_suffix(&rpm_l);
    group.add(&sw);

    let scale = gtk::Scale::with_range(Orientation::Horizontal, min_rpm, max_rpm, 100.0);
    scale.set_draw_value(false);
    scale.set_digits(0);
    scale.set_hexpand(true);
    scale.set_width_request(180);
    let speed_val = gtk::Label::new(Some("—"));
    speed_val.add_css_class("numeric");
    speed_val.add_css_class("scale-value");
    speed_val.set_width_chars(5);
    speed_val.set_xalign(1.0);
    tip(
        &scale,
        &format!(
            "Target RPM for {title} · enabled only in Manual · about {min_rpm:.0}–{max_rpm:.0}"
        ),
    );
    let cur = legion_core::fans::read_target(fan).unwrap_or(0);
    if cur > 0 {
        scale.set_value(cur as f64);
        scale.set_sensitive(true);
        speed_val.set_text(&format!("~{cur}"));
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
    sw.connect_active_notify(move |s| {
        if suppressing_s.get() {
            return;
        }
        if s.is_active() {
            scale_s.set_sensitive(false);
            speed_val_s.set_text("—");
            sw_title.set_title("Automatic");
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
                            speed_val_r.set_text("—");
                            sw_title.set_title("Automatic");
                                suppressing.set(false);
                            return;
                        }
                        high_s.set(true);
                        scale_r.set_sensitive(true);
                        speed_val_r.set_text(&format!("~{rpm}"));
                        sw_title.set_title("Manual");
                                    queue.set_fan(fan, rpm);
                    },
                );
                return;
            }
            scale_s.set_sensitive(true);
            speed_val_s.set_text(&format!("~{rpm}"));
            sw_title.set_title("Manual");
            queue.set_fan(fan, rpm);
        }
    });

    let sw_sc = sw.clone();
    let suppressing_sc = suppressing.clone();
    let queue_sc = apply_queue.clone();
    let high_sc = high_accepted.clone();
    let speed_val_sc = speed_val.clone();
    scale.connect_value_changed(move |sc| {
        speed_val_sc.set_text(&format!("~{}", sc.value() as u32));
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

    // Sysfs read on a worker thread; only the label update touches GTK.
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

    group
}

// ─── Power ──────────────────────────────────────────────────────────────────
