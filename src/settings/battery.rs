//! Power page — Fn Lock / USB, live battery stats, charge limit, EC advisory.

use super::*;

pub(crate) const OFF_CHARGE_HINT: &str = "Battery charged past the limit while the laptop was off, EC behavior. It settles to ~80% with use; unplug AC when off to prevent it.";

/// Plain-words power flow. sysfs `power_now` is an unsigned magnitude, so the
/// direction comes from the battery status.
pub(crate) fn power_text(status: &str, watts: f64) -> String {
    let w = watts.abs();
    if w < 0.05 {
        return match status {
            "Not charging" | "Full" => "Plugged in".to_string(),
            _ => "Idle".to_string(),
        };
    }
    match status {
        "Charging" => format!("Charging {w:.0} W"),
        "Discharging" => format!("Draining {w:.1} W"),
        "Not charging" | "Full" => "Plugged in".to_string(),
        _ => format!("{w:.1} W"),
    }
}

/// Manufacturer / model / chemistry without stray separators.
fn cell_text(mfr: &str, model: &str, tech: &str) -> String {
    let name = format!("{mfr} {model}");
    let parts: Vec<&str> = [name.trim(), tech.trim()]
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        "Not reported".to_string()
    } else {
        parts.join(", ")
    }
}

pub(crate) fn build_battery_pages(
    toast_overlay: &adw::ToastOverlay,
    gate: &DaemonGate,
) -> gtk::Box {
    let status_page = page_lede("");

    let switches = build_ideapad_switches(toast_overlay, gate);

    // Chips on top — 3×2: Capacity · Voltage · Power · Health · Cycles · Limit
    let chips = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .max_children_per_line(3)
        .min_children_per_line(3)
        .homogeneous(true)
        .column_spacing(10)
        .row_spacing(10)
        .build();
    chips.add_css_class("metric-grid");
    let (cap_chip, cap_v, cap_d) = metric_chip_tip("Capacity", None);
    tip(
        &cap_chip,
        "Battery charge level from sysfs BAT0/capacity, 0–100%",
    );
    let (volt_chip, volt_v, volt_d) = metric_chip_tip("Voltage", None);
    tip(
        &volt_chip,
        "Pack voltage in volts, drops as the pack empties",
    );
    let (power_chip, power_v, power_d) = metric_chip_tip("Power", None);
    tip(
        &power_chip,
        "Watts in or out right now, positive while charging, drain while on battery",
    );
    let (health_chip, health_v, health_d) = metric_chip_tip("Health", None);
    tip(
        &health_chip,
        "Wear vs design capacity, 100% is a fresh pack",
    );
    let (cycles_chip, cycles_v, cycles_d) = metric_chip_tip("Cycles", None);
    tip(
        &cycles_chip,
        "Charge cycle count from the battery gauge, grows with use",
    );
    let (limit_chip, limit_v, limit_d) = metric_chip_tip("Limit", None);
    tip(
        &limit_chip,
        "Charge cap (60/80/100%), set it in the rows below",
    );
    // Capacity is the big readout above, voltage lives under Details and the
    // limit is the selector: only three chips stay on the surface.
    power_d.set_visible(false);
    chips.append(&health_chip);
    chips.append(&cycles_chip);
    chips.append(&power_chip);

    // Summary: big percentage + state on the left, charge limit on the right.
    let summary = gtk::Box::new(Orientation::Horizontal, 16);
    summary.add_css_class("panel");
    let pct_box = gtk::Box::new(Orientation::Vertical, 0);
    pct_box.set_hexpand(true);
    pct_box.set_valign(Align::Center);
    let big_pct = gtk::Label::new(Some("n/a"));
    big_pct.add_css_class("battery-pct");
    big_pct.set_xalign(0.0);
    let big_state = gtk::Label::new(None);
    big_state.add_css_class("battery-status");
    big_state.set_xalign(0.0);
    pct_box.append(&big_pct);
    pct_box.append(&big_state);
    summary.append(&pct_box);
    {
        let big_pct = big_pct.clone();
        cap_v.connect_label_notify(move |l| big_pct.set_text(&l.text()));
        let big_state = big_state.clone();
        cap_d.connect_label_notify(move |l| big_state.set_text(&l.text()));
    }
    let limit_col = gtk::Box::new(Orientation::Vertical, 6);
    limit_col.set_valign(Align::Center);
    limit_col.set_halign(Align::End);
    let limit_title = gtk::Label::new(Some("Stop charging at"));
    limit_title.add_css_class("row-title");
    limit_title.set_xalign(1.0);
    limit_col.append(&limit_title);
    summary.append(&limit_col);
    status_page.append(&summary);
    status_page.append(&chips);
    status_page.append(&switches);

    let stats = pref_group("", None);
    tip(
        &stats,
        "Extra battery stats, expand when you need them: voltage, power, energy, health, cycles, pack identity",
    );
    let details_exp = adw::ExpanderRow::builder()
        .title("Details")
        .subtitle("")
        .build();
    tip(
        &details_exp,
        "Tap to expand voltage, power draw, energy, health, cycles, and pack identity",
    );
    let detail_meta = [
        (
            "Voltage",
            "Battery pack voltage in volts, drops as the pack empties",
        ),
        (
            "Power",
            "Watts in or out right now, positive while charging, drain while on battery",
        ),
        (
            "Energy",
            "Watt-hours left / full design capacity of this pack",
        ),
        ("Health", "Wear level vs design capacity"),
        (
            "Cycles",
            "Charge cycle count reported by the battery (when available)",
        ),
        ("Cell", "Manufacturer, model, and chemistry of the pack"),
    ];
    let detail_rows: Vec<adw::ActionRow> = detail_meta
        .into_iter()
        .map(|(title, tip_text)| {
            let row = property_row(title, "n/a", Some(tip_text));
            tip(&row, tip_text);
            details_exp.add_row(&row);
            row
        })
        .collect();
    stats.add(&details_exp);
    status_page.append(&stats);

    let cur = legion_core::battery::charge_limit_pct();
    let limit_labels: Vec<String> = [60u32, 80, 100].iter().map(|p| format!("{p}%")).collect();
    let (pills, limit_buttons) = segmented(
        &limit_labels,
        [60u32, 80, 100].iter().position(|p| *p == cur).map_or(u32::MAX, |i| i as u32),
    );
    pills.set_halign(Align::End);
    tip(
        &pills,
        "60% conserve, 80% long life, 100% full. Caps how far the battery charges while plugged in; needs the legion-control service",
    );
    let guard = Rc::new(Cell::new(false));
    let prev_pct = Rc::new(Cell::new(cur));
    for (btn, pct) in limit_buttons.iter().zip([60u32, 80, 100]) {
        let tip_text = match pct {
            60 => {
                "Conservation (~60%): stops charging early so the pack sits cooler, best if the laptop stays plugged in most of the time"
            }
            80 => {
                "Long life (~80%): more runtime than 60%, gentler than always charging to 100%"
            }
            _ => {
                "Full charge (100%): maximum capacity, use before travel or long unplugged sessions"
            }
        };
        tip(btn, tip_text);
        let overlay = toast_overlay.clone();
        let guard_b = guard.clone();
        let prev = prev_pct.clone();
        let btns = limit_buttons.clone();
        btn.connect_toggled(move |b| {
            if !b.is_active() || guard_b.get() {
                return;
            }
            let before = prev.get();
            prev.set(pct);
            let overlay = overlay.clone();
            let guard_b = guard_b.clone();
            let prev = prev.clone();
            let btns = btns.clone();
            apply_charge_limit(pct, move |result| match result {
                Ok(()) => {
                    legion_core::config::set_charge_limit(pct);
                    toast_ok(&overlay, &format!("Charge limit set to {pct}%"));
                }
                Err(e) => {
                    prev.set(before);
                    guard_b.set(true);
                    match [60u32, 80, 100].iter().position(|p| *p == before) {
                        Some(i) => btns[i].set_active(true),
                        None => btns.iter().for_each(|b| b.set_active(false)),
                    }
                    guard_b.set(false);
                    toast_error(&overlay, &e);
                }
            });
        });
    }
    limit_col.append(&pills);
    gate.track(&pills);

    let volt_l = detail_rows[0].clone();
    let pow_l = detail_rows[1].clone();
    let en_l = detail_rows[2].clone();
    let health_l = detail_rows[3].clone();
    let cycles_l = detail_rows[4].clone();
    let cell_l = detail_rows[5].clone();

    // Prime chips immediately — otherwise first frame shows "—" until the 3 s tick.
    {
        if let Some(pct) = legion_core::battery::capacity() {
            cap_v.set_text(&format!("{pct}%"));
            cap_d.set_text(&legion_core::battery::status().unwrap_or_default());
        }
        if let Some(v) = legion_core::battery::voltage() {
            volt_v.set_text(&format!("{v:.2} V"));
        }
        if let Some(p) = legion_core::battery::power_w() {
            power_v.set_text(&power_text(
                &legion_core::battery::status().unwrap_or_default(),
                p,
            ));
        }
        if let Some(h) = legion_core::battery::health_pct() {
            let (text, _) = legion_core::battery::health_display(h);
            health_v.set_text(&text);
            health_d.set_text("of design capacity");
            tip(&health_chip, &health_tip(h));
        }
        if let Some(c) = legion_core::battery::cycles() {
            cycles_v.set_text(&format!("{c}"));
            cycles_d.set_text("charge cycles");
        }
        limit_v.set_text(&format!("{}%", legion_core::battery::charge_limit_pct()));
        limit_d.set_text("charge cap");
    }
    // Keep chips + rows in sync every 3 s
    let cap_v_c = cap_v.clone();
    let cap_d_c = cap_d.clone();
    let volt_v_c = volt_v.clone();
    let volt_d_c = volt_d.clone();
    let power_v_c = power_v.clone();
    let power_d_c = power_d.clone();
    let health_v_c = health_v.clone();
    let health_d_c = health_d.clone();
    let cap_chip_c = cap_chip.clone();
    let volt_chip_c = volt_chip.clone();
    let power_chip_c = power_chip.clone();
    let health_chip_c = health_chip.clone();
    let cycles_v_c = cycles_v.clone();
    let cycles_d_c = cycles_d.clone();
    let limit_v_c = limit_v.clone();
    let limit_d_c = limit_d.clone();
    // All sysfs reads happen on a worker thread; only widget updates run on
    // the GTK loop (same pattern as the Overview poller).
    #[derive(Default)]
    struct BatterySnapshot {
        pct: Option<u32>,
        status: Option<String>,
        voltage: Option<f64>,
        power_w: Option<f64>,
        energy_now: Option<f64>,
        energy_full: Option<f64>,
        health: Option<f64>,
        cycles: Option<u32>,
        limit: u32,
        mfr: String,
        model: String,
        tech: String,
    }
    let (snap_tx, snap_rx) = mpsc::channel();
    std::thread::spawn(move || loop {
        if !page_polling("battery-status") {
            std::thread::sleep(Duration::from_secs(3));
            continue;
        }
        let snap = BatterySnapshot {
            pct: legion_core::battery::capacity(),
            status: legion_core::battery::status(),
            voltage: legion_core::battery::voltage(),
            power_w: legion_core::battery::power_w(),
            energy_now: legion_core::battery::energy_now_wh(),
            energy_full: legion_core::battery::energy_full_wh(),
            health: legion_core::battery::health_pct(),
            cycles: legion_core::battery::cycles(),
            limit: legion_core::battery::charge_limit_pct(),
            mfr: legion_core::battery::manufacturer().unwrap_or_default(),
            model: legion_core::battery::model_name().unwrap_or_default(),
            tech: legion_core::battery::technology().unwrap_or_default(),
        };
        if snap_tx.send(snap).is_err() {
            return; // GUI is gone
        }
        std::thread::sleep(Duration::from_secs(3));
    });
    // Keep the EC advisory on the Battery page instead of repeatedly adding a
    // global toast while the snapshot poll runs.
    let off_charge_hint = gtk::Label::new(Some(OFF_CHARGE_HINT));
    off_charge_hint.add_css_class("hint");
    off_charge_hint.set_halign(Align::Fill);
    off_charge_hint.set_hexpand(true);
    off_charge_hint.set_wrap(true);
    off_charge_hint.set_xalign(0.0);
    off_charge_hint.set_visible(false);
    status_page.append(&off_charge_hint);
    ticker_local(Duration::from_millis(300), move || {
        match snap_rx.try_recv() {
            Ok(s) => {
                if !off_charge_hint.is_visible()
                    && s.pct
                        .is_some_and(|p| legion_core::battery::above_limiter_band(s.limit, p))
                {
                    off_charge_hint.set_visible(true);
                }
                if let Some(pct) = s.pct {
                    cap_v_c.set_text(&format!("{pct}%"));
                    cap_d_c.set_text(s.status.as_deref().unwrap_or_default());
                    // Tint capacity chip: warm when discharging low, hot when very low? Use level thresholds.
                    cap_chip_c.remove_css_class("hot");
                    cap_chip_c.remove_css_class("warm");
                    if pct <= 20 {
                        cap_chip_c.add_css_class("hot");
                    } else if pct <= 40 {
                        cap_chip_c.add_css_class("warm");
                    }
                }
                if let Some(v) = s.voltage {
                    volt_l.set_subtitle(&format!("{v:.2} V"));
                    volt_v_c.set_text(&format!("{v:.2} V"));
                    volt_d_c.set_text("");
                    volt_chip_c.remove_css_class("hot");
                    volt_chip_c.remove_css_class("warm");
                } else {
                    volt_v_c.set_text("n/a");
                    volt_d_c.set_text("no sensor");
                }
                if let Some(p) = s.power_w {
                    let st = s.status.as_deref().unwrap_or_default();
                    let txt = power_text(st, p);
                    pow_l.set_subtitle(&txt);
                    power_v_c.set_text(&txt);
                    power_d_c.set_text("");
                    power_chip_c.remove_css_class("hot");
                    power_chip_c.remove_css_class("warm");
                    if p.abs() > 45.0 {
                        power_chip_c.add_css_class("warm");
                    }
                } else {
                    pow_l.set_subtitle("n/a");
                    power_v_c.set_text("n/a");
                    power_d_c.set_text("no sensor");
                }
                if let (Some(n), Some(f)) = (s.energy_now, s.energy_full) {
                    en_l.set_subtitle(&format!("{n:.1} / {f:.1} Wh"))
                }
                if let Some(h) = s.health {
                    let (text, _) = legion_core::battery::health_display(h);
                    health_l.set_subtitle(&format!("{text} of design capacity"));
                    tip(&health_l, &health_tip(h));
                    tip(&health_chip_c, &health_tip(h));
                    health_v_c.set_text(&text);
                    health_d_c.set_text("of design capacity");
                    health_chip_c.remove_css_class("hot");
                    health_chip_c.remove_css_class("warm");
                    if h < 70.0 {
                        health_chip_c.add_css_class("hot");
                    } else if h < 85.0 {
                        health_chip_c.add_css_class("warm");
                    }
                } else {
                    health_v_c.set_text("n/a");
                    health_d_c.set_text("no sensor");
                }
                if let Some(c) = s.cycles {
                    cycles_l.set_subtitle(&format!("{c}"));
                    cycles_v_c.set_text(&format!("{c}"));
                    cycles_d_c.set_text("charge cycles");
                } else {
                    cycles_v_c.set_text("n/a");
                    cycles_d_c.set_text("no sensor");
                }
                limit_v_c.set_text(&format!("{}%", s.limit));
                limit_d_c.set_text("charge cap");
                cell_l.set_subtitle(&cell_text(&s.mfr, &s.model, &s.tech));
                glib::ControlFlow::Continue
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => glib::ControlFlow::Break,
        }
    });

    status_page
}

fn health_tip(raw: f64) -> String {
    format!(
        "Wear vs design capacity. Raw reading {raw:.1}%{}",
        if raw > 100.0 { " (shown as 100%)" } else { "" }
    )
}

// ─── Troubleshoot ───────────────────────────────────────────────────────────

// ─── ideapad_acpi switches (Fn Lock, always-on USB charging) ────────────────

/// Group with the Fn Lock and USB-charging switches. Everything starts hidden;
/// support and state are probed on a worker thread and rows only appear for
/// attributes the kernel driver actually exposes on this laptop.
fn build_ideapad_switches(
    toast_overlay: &adw::ToastOverlay,
    gate: &DaemonGate,
) -> adw::PreferencesGroup {
    use legion_core::ideapad::Toggle;

    let group = pref_group("Keyboard &amp; USB", None);
    group.set_visible(false);
    gate.track(&group);

    type Spec = (
        Toggle,
        &'static str,
        &'static str,
        &'static str,
        fn(bool) -> DaemonCommand,
    );
    let specs: [Spec; 2] = [
        (
            Toggle::FnLock,
            "Fn lock",
            "F1 to F12 act as function keys without holding Fn",
            "Same as Fn+Esc. Needs the legion-control service to change.",
            DaemonCommand::SetFnLock,
        ),
        (
            Toggle::UsbCharging,
            "Always-on USB charging",
            "Keep the USB port powered while the laptop is off or asleep",
            "Firmware may ignore this on some models; you will get an error if it does.",
            DaemonCommand::SetUsbCharging,
        ),
    ];

    let mut rows: Vec<(Toggle, adw::SwitchRow, Rc<Cell<bool>>)> = Vec::new();
    for (toggle, title, subtitle, tip_text, make_cmd) in specs {
        let row = adw::SwitchRow::builder()
            .title(title)
            .subtitle(subtitle)
            .visible(false)
            .build();
        tip(&row, tip_text);
        let guard = Rc::new(Cell::new(false));
        let guard_n = guard.clone();
        let overlay = toast_overlay.clone();
        row.connect_active_notify(move |row| {
            let guard = guard_n.clone();
            if guard.get() {
                return;
            }
            let on = row.is_active();
            let row = row.clone();
            let overlay = overlay.clone();
            run_daemon_command_async(make_cmd(on), move |result| {
                let revert = |msg: &str| {
                    guard.set(true);
                    row.set_active(!on);
                    guard.set(false);
                    toast_error(&overlay, msg);
                };
                match result {
                    Ok(DaemonResponse::Ok) => toast_ok(
                        &overlay,
                        &format!("{title} {}", if on { "on" } else { "off" }),
                    ),
                    Ok(DaemonResponse::Error(e)) => revert(&e),
                    Err(e) if is_version_skew_error(&e) => {
                        revert("Service outdated, reinstall to update")
                    }
                    Err(e) => revert(&e),
                    Ok(_) => revert("Unexpected response from service"),
                }
            });
        });
        group.add(&row);
        rows.push((toggle, row, guard));
    }

    // Probe sysfs off the GTK thread; reveal only supported rows.
    let kinds: Vec<Toggle> = rows.iter().map(|(t, _, _)| *t).collect();
    let group_c = group.clone();
    dispatch_async(
        move || {
            Ok(kinds
                .into_iter()
                .map(|t| (t, legion_core::ideapad::get(t)))
                .collect::<Vec<_>>())
        },
        "Keyboard & USB probe stopped without a result",
        move |res: Result<Vec<(Toggle, Option<bool>)>, String>| {
            let Ok(states) = res else { return };
            for (toggle, state) in states {
                let Some(on) = state else { continue };
                if let Some((_, row, guard)) = rows.iter().find(|(t, _, _)| *t == toggle) {
                    // Programmatic sync must not trigger a write.
                    guard.set(true);
                    row.set_active(on);
                    guard.set(false);
                    row.set_visible(true);
                    group_c.set_visible(true);
                }
            }
        },
    );
    group
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_text_plain_words() {
        assert_eq!(power_text("Discharging", 0.0), "Idle");
        assert_eq!(power_text("Charging", 45.2), "Charging 45 W");
        assert_eq!(power_text("Discharging", 9.84), "Draining 9.8 W");
        assert_eq!(power_text("Not charging", 0.0), "Plugged in");
        assert_eq!(power_text("Full", 3.0), "Plugged in");
    }

    #[test]
    fn cell_text_has_no_bare_separator() {
        assert_eq!(cell_text("", "", ""), "Not reported");
        assert_eq!(cell_text("", "", "Li-poly"), "Li-poly");
        assert_eq!(cell_text("LGC", "L23", "Li-poly"), "LGC L23, Li-poly");
    }
}
