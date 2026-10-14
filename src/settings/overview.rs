//! Control center, the home page: health pill, power mode, live cards,
//! temperature history and quick toggles.
//!
//! Every IPC / sysfs / HID read or write happens off the GTK main thread
//! (`dispatch_async`, `run_daemon_command_async` or the poll worker).

use super::*;

/// Navigation callback: (stack page id, header title).
pub(crate) type NavFn = Rc<dyn Fn(&'static str, &'static str)>;

/// One quick-toggle tile: name on the left, switch on the right; the hint
/// is a tooltip. Starts hidden; the capability probe reveals it.
fn quick_tile(name: &str, hint: &str) -> (gtk::Box, gtk::Switch) {
    let tile = gtk::Box::new(Orientation::Horizontal, 8);
    tile.add_css_class("quick-tile");
    tile.set_visible(false);
    tip(&tile, hint);
    let n = gtk::Label::new(Some(name));
    n.add_css_class("quick-name");
    n.set_halign(Align::Start);
    n.set_xalign(0.0);
    n.set_hexpand(true);
    n.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let sw = gtk::Switch::new();
    sw.set_valign(Align::Center);
    sw.update_property(&[gtk::accessible::Property::Label(name)]);
    tile.append(&n);
    tile.append(&sw);
    (tile, sw)
}

/// A live row: name, thin usage bar and the current value. The secondary
/// detail (watts, load, per-fan rpm) is kept off-screen as a tooltip.
fn live_card(
    title: &str,
    tooltip: &str,
) -> (gtk::Box, gtk::Label, gtk::Label, gtk::Label, gtk::ProgressBar) {
    let row = gtk::Box::new(Orientation::Horizontal, 14);
    row.add_css_class("live-row");
    tip(&row, tooltip);
    let name = gtk::Label::new(Some(title));
    name.add_css_class("live-name");
    name.set_xalign(0.0);
    name.set_width_request(150);
    let bar = gtk::ProgressBar::new();
    bar.add_css_class("card-bar");
    bar.set_hexpand(true);
    bar.set_valign(Align::Center);
    let value = gtk::Label::new(Some("…"));
    value.add_css_class("value");
    value.add_css_class("numeric");
    value.set_xalign(1.0);
    value.set_width_chars(9);
    let detail = gtk::Label::new(None);
    {
        let row = row.clone();
        let base = tooltip.to_string();
        detail.connect_label_notify(move |l| {
            let d = l.text();
            row.set_tooltip_text(Some(&if d.is_empty() {
                base.clone()
            } else {
                format!("{d}\n{base}")
            }));
        });
    }
    row.append(&name);
    row.append(&bar);
    row.append(&value);
    (row, name, value, detail, bar)
}

/// Issue count from the read-only self-checks plus non-info faults.
pub(crate) fn health_issue_count() -> usize {
    let checks = legion_core::selftest::run_self_checks();
    let faults = legion_core::selftest::scan_faults();
    checks.iter().filter(|c| !c.ok).count()
        + faults
            .iter()
            .filter(|f| f.severity != legion_core::selftest::Severity::Info)
            .count()
}

/// Everything the capability probe for the quick toggles learns.
#[derive(Default)]
struct QuickProbe {
    kbd: Option<(bool, u8)>,
    fans_full: Option<bool>,
    fn_lock: Option<bool>,
    usb: Option<bool>,
}

/// Run a daemon switch command; on failure put the switch back.
fn daemon_switch(
    overlay: &adw::ToastOverlay,
    sw: &gtk::Switch,
    guard: &Rc<Cell<bool>>,
    label: &'static str,
    make: fn(bool) -> DaemonCommand,
) {
    let on = sw.is_active();
    let sw = sw.clone();
    let guard = guard.clone();
    let overlay = overlay.clone();
    run_daemon_command_async(make(on), move |result| {
        let revert = |msg: &str| {
            guard.set(true);
            sw.set_active(!on);
            guard.set(false);
            toast_error(&overlay, msg);
        };
        match result {
            Ok(DaemonResponse::Ok) => toast_ok(
                &overlay,
                &format!("{label} {}", if on { "on" } else { "off" }),
            ),
            Ok(DaemonResponse::Error(e)) => revert(&e),
            Err(e) if is_version_skew_error(&e) => revert("Service outdated, reinstall to update"),
            Err(e) => revert(&e),
            Ok(_) => revert("Unexpected response from service"),
        }
    });
}

/// (id, rpm, target) for each fan channel, from daemon or local sensors.
fn fan_readings(ids: &[u8], s: &legion_core::sensors::SensorReadings) -> Vec<(u8, u32, u32)> {
    let aux = legion_core::fans::aux_id_from(ids);
    ids.iter()
        .map(|id| {
            let (rpm, target) = match id {
                1 => (s.fan1_rpm, s.fan1_target),
                2 => (s.fan2_rpm, s.fan2_target),
                id if Some(*id) == aux => (s.fan4_rpm, s.fan4_target),
                _ => (
                    legion_core::fans::read_rpm(*id).unwrap_or(0),
                    legion_core::fans::read_target(*id).unwrap_or(0),
                ),
            };
            let target =
                legion_core::fans::effective_target(target, legion_core::fans::read_max(*id));
            (*id, rpm, target)
        })
        .collect()
}

/// Temperature bar fill: 30 °C empty, 100 °C full.
fn temp_fraction(c: f64) -> f64 {
    ((c - 30.0) / 70.0).clamp(0.0, 1.0)
}

fn fan_short_name(id: u8, aux: Option<u8>) -> String {
    match id {
        1 => "CPU".into(),
        2 => "GPU".into(),
        id if Some(id) == aux => "Aux".into(),
        id => format!("Fan {id}"),
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn build_overview(
    toast_overlay: &adw::ToastOverlay,
    _apply_queue: &ApplyQueue,
    gate: &DaemonGate,
    nav: &NavFn,
    mode_drop_slot: &Rc<RefCell<Option<adw::ComboRow>>>,
    profile_choices_slot: &Rc<RefCell<Vec<String>>>,
    ppt_group_slot: &Rc<RefCell<Option<adw::PreferencesGroup>>>,
    ppt_scales_slot: &PptScales,
    ppt_suppress_slot: &Rc<Cell<bool>>,
    trend_feed_slot: &Rc<RefCell<Option<Rc<dyn Fn(f64, f64)>>>>,
) -> gtk::Box {
    let page = page_lede("");
    let info = legion_core::device::detect();
    if !legion_core::device::vendor_supported() {
        let banner = adw::Banner::new(&info.profile_notes);
        banner.set_revealed(true);
        page.append(&banner);
    }

    // Home starts directly with the power mode: model/BIOS details live on
    // Settings, health on the sidebar footer.
    let set_machine_line = |_on_ac: Option<bool>| {};

    // ── Power mode (segmented, radio semantics) ─────────────────────────
    let blocked = legion_core::profile::blocked_choices();
    let blocked_note = blocked
        .iter()
        .map(|(c, why)| format!("{} is hidden: {why}", friendly_profile(c)))
        .collect::<Vec<_>>()
        .join("\n");
    let choices = legion_core::profile::safe_choices();
    *profile_choices_slot.borrow_mut() = choices.clone();
    let labels: Vec<String> = choices.iter().map(|c| friendly_profile(c)).collect();
    let label_refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
    let current = legion_core::profile::current();
    let active = choices.iter().position(|c| *c == current).unwrap_or(0) as u32;

    // The ComboRow is the logic holder shared with Profiles / Performance
    // (mode_drop_slot), it is never shown; the segmented control mirrors it.
    let drop = string_combo_row("Mode", "", &label_refs, active);
    *mode_drop_slot.borrow_mut() = Some(drop.clone());

    let power_box = gtk::Box::new(Orientation::Horizontal, 12);
    let power_title = gtk::Label::new(Some("Power mode"));
    power_title.add_css_class("row-title");
    power_title.set_halign(Align::Start);
    power_title.set_xalign(0.0);
    power_title.set_hexpand(true);
    power_box.append(&power_title);
    // The power-limit sliders live on Performance; Home only links to them
    // while Custom is active.
    let edit_limits = secondary_button_tip(
        "Edit power limits",
        Some("Opens Performance, where the Custom power limits are set"),
    );
    edit_limits.set_valign(Align::Center);
    edit_limits.set_visible(current == "custom");
    {
        let nav = nav.clone();
        edit_limits.connect_clicked(move |_| nav("cpu", "Performance"));
    }
    power_box.append(&edit_limits);
    let (seg_box, seg_buttons) = segmented(&labels, active);
    seg_box.set_halign(Align::End);
    tip(
        &seg_box,
        &if blocked_note.is_empty() {
            "Custom unlocks the CPU and GPU power sliders".to_string()
        } else {
            blocked_note.clone()
        },
    );
    for (b, c) in seg_buttons.iter().zip(choices.iter()) {
        tip(b, profile_tooltip(c));
    }
    power_box.append(&seg_box);
    page.append(&power_box);
    gate.track(&seg_box);
    // The mode blurb is only a tooltip now (one short line, no filler).
    let mode_hint = gtk::Label::new(Some(profile_blurb(&current)));

    let seg_guard = Rc::new(Cell::new(false));
    for (i, b) in seg_buttons.iter().enumerate() {
        let drop = drop.clone();
        let seg_guard = seg_guard.clone();
        b.connect_toggled(move |btn| {
            if !btn.is_active() || seg_guard.get() {
                return;
            }
            if drop.selected() != i as u32 {
                drop.set_selected(i as u32);
            }
        });
    }
    {
        // Any change of the hidden ComboRow (user click, declined
        // confirmation, Fn+Q poll, profile Load) re-syncs the segments.
        let seg_buttons = seg_buttons.clone();
        let seg_guard = seg_guard.clone();
        let choices = choices.clone();
        let hint = mode_hint.clone();
        let edit_limits = edit_limits.clone();
        drop.connect_selected_notify(move |d| {
            let idx = d.selected() as usize;
            edit_limits.set_visible(choices.get(idx).is_some_and(|c| c == "custom"));
            seg_guard.set(true);
            if let Some(b) = seg_buttons.get(idx) {
                b.set_active(true);
            }
            seg_guard.set(false);
            if let Some(name) = choices.get(idx) {
                hint.set_text(profile_blurb(name));
            }
        });
    }

    let overlay = toast_overlay.clone();
    let choices_c = choices.clone();
    let ppt_box_slot = ppt_group_slot.clone();
    let ppt_scales_slot_c = ppt_scales_slot.clone();
    let ppt_suppress_slot_c = ppt_suppress_slot.clone();
    let profile_guard = Rc::new(Cell::new(false));
    let last_ok = Rc::new(Cell::new(active));
    let profile_guard_n = profile_guard.clone();
    let last_ok_n = last_ok.clone();
    drop.connect_selected_notify(move |d| {
        if profile_guard_n.get() {
            return;
        }
        let idx = d.selected() as usize;
        if idx >= choices_c.len() {
            return;
        }
        let name = choices_c[idx].clone();

        let apply = {
            let name = name.clone();
            let overlay = overlay.clone();
            let ppt_box_slot = ppt_box_slot.clone();
            let ppt_scales_slot = ppt_scales_slot_c.clone();
            let ppt_suppress_slot = ppt_suppress_slot_c.clone();
            move || {
                if let Some(ppt_box) = ppt_box_slot.borrow().as_ref() {
                    apply_platform_profile(
                        &name,
                        &overlay,
                        ppt_box,
                        &ppt_scales_slot,
                        &ppt_suppress_slot,
                    );
                } else {
                    let overlay_e = overlay.clone();
                    run_daemon_command_async(
                        DaemonCommand::SetProfile(name.clone()),
                        move |result| match result {
                            Ok(DaemonResponse::Ok) => {
                                legion_core::config::remember_platform_profile(&name);
                                toast_ok(
                                    &overlay_e,
                                    &format!("Switched to {}", friendly_profile(&name)),
                                );
                            }
                            Ok(DaemonResponse::Error(e)) => toast_error(&overlay_e, &e),
                            Err(e) => toast_error(&overlay_e, &e),
                            _ => {}
                        },
                    );
                }
            }
        };

        // Max power and Custom ask for confirmation; declining restores the
        // previous selection (which re-syncs the segmented control).
        let confirm_kind = match name.as_str() {
            "max-power" => Some(true),
            "custom" => Some(false),
            _ => None,
        };
        if let (Some(is_max), true) = (confirm_kind, last_ok_n.get() != idx as u32) {
            let drop_r = d.clone();
            let choices_c = choices_c.clone();
            let profile_guard = profile_guard_n.clone();
            let last_ok = last_ok_n.clone();
            let prev = last_ok.get();
            let done = move |accepted: bool| {
                if !accepted {
                    profile_guard.set(true);
                    drop_r.set_selected(prev);
                    profile_guard.set(false);
                    return;
                }
                apply();
                let key = if is_max { "max-power" } else { "custom" };
                last_ok.set(
                    choices_c
                        .iter()
                        .position(|c| c == key)
                        .unwrap_or(prev as usize) as u32,
                );
            };
            if is_max {
                confirm_max_power(d, done);
            } else {
                confirm_custom_power(d, done);
            }
            return;
        }

        apply();
        last_ok_n.set(idx as u32);
    });

    // ── Live cards: CPU, GPU, Fans, Battery ──────────────────────────
    let cards = gtk::Box::new(Orientation::Vertical, 0);
    cards.add_css_class("panel");
    cards.add_css_class("live-panel");

    let (cpu_card, _cpu_t, cpu_v, cpu_d, cpu_bar) = live_card(
        "CPU temperature",
        &format!(
            "{}, package temperature, power and load",
            if info.cpu_model.is_empty() {
                "Processor"
            } else {
                &info.cpu_model
            }
        ),
    );
    let (gpu_card, _gpu_t, gpu_v, gpu_d, gpu_bar) = live_card(
        "GPU temperature",
        &format!(
            "{}, discrete GPU temperature, power and utilization",
            if info.gpu_model.is_empty() {
                "Discrete GPU"
            } else {
                &info.gpu_model
            }
        ),
    );
    cpu_bar.add_css_class("temp-bar");
    gpu_bar.add_css_class("temp-bar");
    let fan_channels = legion_core::fans::channels();
    let fan_ids: Vec<u8> = fan_channels.iter().map(|c| c.id).collect();
    let aux_fan = legion_core::fans::aux_id_from(&fan_ids);
    let fan_max_rpm = fan_channels
        .iter()
        .map(|c| c.max_rpm)
        .max()
        .filter(|m| *m > 0)
        .unwrap_or(6000) as f64;
    let (fan_card, fan_t, fan_v, fan_d, fan_bar) = live_card(
        "Fans",
        "Fastest fan. Idle means the fans are off until the laptop warms up",
    );
    let (bat_card, bat_t, bat_v, bat_d, bat_bar) =
        live_card("Battery", "Charge level and state");
    cards.append(&cpu_card);
    cards.append(&gpu_card);
    if !fan_channels.is_empty() {
        cards.append(&fan_card);
    }
    cards.append(&bat_card);
    page.append(&cards);

    // ── Temperature chart (service history, see history.rs) ─────────────
    let (chart_expander, history_feed) = history::build_history();
    trend_feed_slot.replace(Some(history_feed));

    // ── Quick toggles (only what this hardware supports) ────────────────
    let (kbd_tile, kbd_sw) = quick_tile("Keyboard light", "Turn the keyboard backlight on or off");
    let (fan_tile, fan_sw) = quick_tile("Fans full speed", "Run every fan at maximum for maximum airflow");
    let (fnl_tile, fnl_sw) = quick_tile("Fn lock", "F1 to F12 act as function keys without holding Fn");
    let (usb_tile, usb_sw) = quick_tile("Always-on USB", "Keep the USB port powered while the laptop is off or asleep");
    let quick_grid = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .max_children_per_line(2)
        .min_children_per_line(2)
        .homogeneous(true)
        .column_spacing(8)
        .row_spacing(8)
        .build();
    quick_grid.add_css_class("metric-grid");
    for t in [&kbd_tile, &fan_tile, &fnl_tile, &usb_tile] {
        quick_grid.append(t);
        // Hide the FlowBox slot too, or hidden tiles leave blank gaps.
        if let Some(slot) = t.parent() {
            slot.set_visible(false);
        }
    }
    let quick_wrap = gtk::Box::new(Orientation::Vertical, 6);
    quick_wrap.append(&quick_grid);
    quick_wrap.set_visible(false);
    page.append(&quick_wrap);
    gate.track(&quick_grid);

    let lighting_kind = info.capabilities.lighting_kind;
    let kbd_level = Rc::new(Cell::new(5u8));
    let kbd_guard = Rc::new(Cell::new(true));
    let fan_guard = Rc::new(Cell::new(true));
    let fnl_guard = Rc::new(Cell::new(true));
    let usb_guard = Rc::new(Cell::new(true));
    {
        let overlay = toast_overlay.clone();
        let guard = kbd_guard.clone();
        let level = kbd_level.clone();
        kbd_sw.connect_active_notify(move |sw| {
            if guard.get() {
                return;
            }
            let on = sw.is_active();
            let msg = if on {
                "Keyboard light on"
            } else {
                "Keyboard light off"
            };
            match lighting_kind {
                legion_core::device::LightingKind::Spectrum => {
                    legion_core::keyboard::set_rgb_brightness_async(if on {
                        level.get().clamp(1, 9)
                    } else {
                        0
                    });
                    toast_ok(&overlay, msg);
                }
                _ => {
                    let sw = sw.clone();
                    let guard = guard.clone();
                    let overlay = overlay.clone();
                    dispatch_async(
                        move || {
                            let mut st = legion_core::config::get().four_zone.to_state();
                            st.brightness = if on { st.brightness.max(2) } else { 0 };
                            legion_core::fourzone::apply_and_save(&st)
                        },
                        "Keyboard light stopped without a result",
                        move |res| match res {
                            Ok(()) => toast_ok(&overlay, msg),
                            Err(e) => {
                                guard.set(true);
                                sw.set_active(!on);
                                guard.set(false);
                                toast_error(&overlay, &e);
                            }
                        },
                    );
                }
            }
        });
    }
    {
        let overlay = toast_overlay.clone();
        let guard = fan_guard.clone();
        let channels: Vec<(u8, u32)> = fan_channels.iter().map(|c| (c.id, c.max_rpm)).collect();
        fan_sw.connect_active_notify(move |sw| {
            if guard.get() {
                return;
            }
            let on = sw.is_active();
            let sw = sw.clone();
            let guard = guard.clone();
            let overlay = overlay.clone();
            let channels = channels.clone();
            dispatch_async(
                move || {
                    let mut last_err = None;
                    for (id, max) in &channels {
                        let target = if on { *max } else { 0 };
                        match send_command(DaemonCommand::SetFanTarget(*id, target)) {
                            Ok(DaemonResponse::Ok) => {
                                legion_core::config::remember_fan(*id, target)
                            }
                            Ok(DaemonResponse::Error(e)) => last_err = Some(e),
                            Err(e) if is_version_skew_error(&e) => {
                                last_err = Some("Service outdated, reinstall to update".into())
                            }
                            Err(e) => last_err = Some(e),
                            Ok(_) => last_err = Some("Unexpected response from service".into()),
                        }
                    }
                    match last_err {
                        Some(e) => Err(e),
                        None => Ok(()),
                    }
                },
                "Fan request stopped without a result",
                move |res| match res {
                    Ok(()) => toast_ok(
                        &overlay,
                        if on {
                            "Fans at full speed"
                        } else {
                            "Fans back to automatic"
                        },
                    ),
                    Err(e) => {
                        guard.set(true);
                        sw.set_active(!on);
                        guard.set(false);
                        toast_error(&overlay, &e);
                    }
                },
            );
        });
    }
    {
        let overlay = toast_overlay.clone();
        let guard = fnl_guard.clone();
        fnl_sw.connect_active_notify(move |sw| {
            if !guard.get() {
                daemon_switch(&overlay, sw, &guard, "Fn lock", DaemonCommand::SetFnLock);
            }
        });
        let overlay = toast_overlay.clone();
        let guard = usb_guard.clone();
        usb_sw.connect_active_notify(move |sw| {
            if !guard.get() {
                daemon_switch(
                    &overlay,
                    sw,
                    &guard,
                    "Always-on USB",
                    DaemonCommand::SetUsbCharging,
                );
            }
        });
    }
    {
        let kbd_possible = matches!(
            lighting_kind,
            legion_core::device::LightingKind::Spectrum
                | legion_core::device::LightingKind::FourZone
        );
        let fans_possible = !fan_channels.is_empty();
        let fan_channels_p: Vec<(u8, u32)> =
            fan_channels.iter().map(|c| (c.id, c.max_rpm)).collect();
        let quick_wrap = quick_wrap.clone();
        dispatch_async(
            move || {
                use legion_core::ideapad::{get, Toggle};
                let mut p = QuickProbe::default();
                if kbd_possible {
                    p.kbd = match lighting_kind {
                        legion_core::device::LightingKind::Spectrum => {
                            legion_core::keyboard::rgb_brightness().map(|b| (b > 0, b))
                        }
                        _ => legion_core::fourzone::present().then(|| {
                            let b = legion_core::config::get().four_zone.brightness;
                            (b > 0, b)
                        }),
                    };
                }
                if fans_possible && legion_core::fans::control_backend_name().is_some() {
                    p.fans_full = Some(fan_channels_p.iter().all(|(id, max)| {
                        *max > 0
                            && legion_core::fans::read_target(*id)
                                .is_some_and(|t| t as f64 >= *max as f64 * 0.95)
                    }));
                }
                p.fn_lock = get(Toggle::FnLock);
                p.usb = get(Toggle::UsbCharging);
                Ok::<_, String>(p)
            },
            "Quick-toggle probe stopped without a result",
            move |res| {
                let Ok(p) = res else { return };
                let mut any = false;
                let mut reveal = |tile: &gtk::Box,
                                  sw: &gtk::Switch,
                                  guard: &Rc<Cell<bool>>,
                                  state: Option<bool>| {
                    if let Some(on) = state {
                        guard.set(true);
                        sw.set_active(on);
                        guard.set(false);
                        tile.set_visible(true);
                        if let Some(slot) = tile.parent() {
                            slot.set_visible(true);
                        }
                        any = true;
                    } else {
                        guard.set(false);
                    }
                };
                if let Some((_, level)) = p.kbd {
                    if level > 0 {
                        kbd_level.set(level);
                    }
                }
                reveal(&kbd_tile, &kbd_sw, &kbd_guard, p.kbd.map(|(on, _)| on));
                reveal(&fan_tile, &fan_sw, &fan_guard, p.fans_full);
                reveal(&fnl_tile, &fnl_sw, &fnl_guard, p.fn_lock);
                reveal(&usb_tile, &usb_sw, &usb_guard, p.usb);
                quick_wrap.set_visible(any);
            },
        );
    }

    page.append(&chart_expander);

    // ── Live poll ───────────────────────────────────────────────────────
    let mode_drop_poll = drop.clone();
    let choices_poll = choices.clone();
    let profile_guard_poll = profile_guard.clone();
    let last_ok_poll = last_ok.clone();
    let last_firmware_mode = Rc::new(RefCell::new(current));
    // Fn+Q can flip the EC into/out of Custom without touching this GUI —
    // reveal or hide the Custom sliders immediately on the next poll tick.
    let ppt_box_poll = ppt_group_slot.clone();
    let ppt_scales_poll = ppt_scales_slot.clone();
    let _ = legion_core::sensors::sample_cpu_usage_pct();

    struct BatteryPoll {
        pct: u32,
        status: String,
        health: Option<f64>,
        cycles: Option<u32>,
    }
    // Data collected off the main thread each tick. IPC and nvidia-smi can
    // block for seconds, they must never run inside the GTK main loop.
    struct DashboardPoll {
        cpu_pct: f64,
        gpu_pct: f64,
        firmware_mode: Option<String>,
        sensors: Option<legion_core::sensors::SensorReadings>,
        cpu_w: Option<f64>,
        local: Option<legion_core::sensors::SensorReadings>,
        battery: Option<BatteryPoll>,
    }
    let (dash_tx, dash_rx) = mpsc::channel::<DashboardPoll>();
    std::thread::Builder::new()
        .name("dashboard-poll".into())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_secs(2));
            let cpu_pct = legion_core::sensors::sample_cpu_usage_pct();
            let gpu_pct = legion_core::sensors::sample_gpu_usage_pct();
            let firmware_mode = match send_command(DaemonCommand::GetProfile) {
                Ok(DaemonResponse::Profile(mode)) => Some(mode),
                _ => None,
            };
            let sensors = match send_command(DaemonCommand::GetSensors) {
                Ok(DaemonResponse::Sensors(s)) => Some(s),
                _ => None,
            };
            let cpu_w = match send_command(DaemonCommand::GetCpuPower) {
                Ok(DaemonResponse::CpuPower(w)) if w > 0.5 => Some(w),
                _ => None,
            };
            let local = sensors.is_none().then(legion_core::sensors::read_all);
            let battery = legion_core::battery::capacity().map(|pct| BatteryPoll {
                pct,
                status: legion_core::battery::status().unwrap_or_default(),
                health: legion_core::battery::health_pct(),
                cycles: legion_core::battery::cycles(),
            });
            if dash_tx
                .send(DashboardPoll {
                    cpu_pct,
                    gpu_pct,
                    firmware_mode,
                    sensors,
                    cpu_w,
                    local,
                    battery,
                })
                .is_err()
            {
                break; // UI closed
            }
        })
        .ok();

    let cpu_card_c = cpu_card.clone();
    let gpu_card_c = gpu_card.clone();
    let bat_card_c = bat_card.clone();
    let trend_feed_poll = trend_feed_slot.clone();
    glib::timeout_add_local(Duration::from_millis(200), move || {
        while let Ok(poll) = dash_rx.try_recv() {
            if let Some(firmware_mode) = poll.firmware_mode {
                let changed = *last_firmware_mode.borrow() != firmware_mode;
                if changed {
                    *last_firmware_mode.borrow_mut() = firmware_mode.clone();
                    if let Some(index) = choices_poll.iter().position(|c| c == &firmware_mode) {
                        let index = index as u32;
                        profile_guard_poll.set(true);
                        mode_drop_poll.set_selected(index);
                        last_ok_poll.set(index);
                        profile_guard_poll.set(false);
                        legion_core::config::remember_platform_profile(&firmware_mode);
                    }
                    if let Some(ppt_box) = ppt_box_poll.borrow().as_ref() {
                        ppt_box.set_visible(
                            firmware_mode == "custom" && !ppt_scales_poll.borrow().is_empty(),
                        );
                    }
                }
            }

            // Battery card + header power line.
            if let Some(b) = &poll.battery {
                bat_v.set_text(&format!("{}%", b.pct));
                bat_bar.set_fraction(b.pct as f64 / 100.0);
                let mut d = b.status.clone();
                if let Some(c) = b.cycles {
                    d.push_str(&format!(", {c} cycles"));
                }
                bat_d.set_text(&d);
                if let Some(h) = b.health {
                    let (text, _) = legion_core::battery::health_display(h);
                    bat_d.set_text(&format!("{d}, health {text}"));
                }
                let _ = &bat_t;
                bat_card_c.remove_css_class("hot");
                bat_card_c.remove_css_class("warm");
                if b.status == "Discharging" {
                    if b.pct <= 20 {
                        bat_card_c.add_css_class("hot");
                    } else if b.pct <= 40 {
                        bat_card_c.add_css_class("warm");
                    }
                }
                set_machine_line(Some(b.status != "Discharging"));
            }

            let from_daemon = poll.sensors.is_some();
            if let Some(s) = poll.sensors.as_ref().or(poll.local.as_ref()) {
                let c = if s.ec_cpu > 0.0 { s.ec_cpu } else { s.cpu_temp };
                if c > 0.0 {
                    cpu_v.set_text(&format!("{c:.0} °C"));
                    tint_temp(&cpu_card_c, c);
                }
                cpu_bar.set_fraction(temp_fraction(c));
                cpu_d.set_text(&match poll.cpu_w {
                    Some(w) => format!("{w:.0} W, load {:.0}%", poll.cpu_pct),
                    None => format!("load {:.0}%", poll.cpu_pct),
                });

                let mut g = if s.dgpu_temp < 0.0 {
                    s.ec_gpu
                } else {
                    s.dgpu_temp.max(s.ec_gpu)
                };
                let mut gpu_power = s.dgpu_power;
                // Daemon cgroup can block NVML (nvidia-caps) even when the
                // GPU is awake, read nvidia-smi from this user process.
                if from_daemon && g <= 0.0 {
                    let local = legion_core::dgpu::read_metrics_batch();
                    if let Some(t) = local.temp.filter(|t| *t > 0.0) {
                        g = t;
                        if gpu_power < 0.0 {
                            gpu_power = local.power.unwrap_or(gpu_power);
                        }
                    }
                }
                if g > 0.0 {
                    gpu_v.set_text(&format!("{g:.0} °C"));
                    tint_temp(&gpu_card_c, g);
                } else {
                    gpu_v.set_text(if legion_core::dgpu::discrete_present() {
                        "Off"
                    } else {
                        "n/a"
                    });
                    tint_temp(&gpu_card_c, 0.0);
                }
                gpu_bar.set_fraction(temp_fraction(g));
                gpu_d.set_text(&if gpu_power >= 0.0 {
                    format!("{gpu_power:.0} W, load {:.0}%", poll.gpu_pct)
                } else {
                    format!("load {:.0}%", poll.gpu_pct)
                });

                let fans = fan_readings(&fan_ids, s);
                if !fans.is_empty() {
                    let top = fans.iter().map(|f| f.1).max().unwrap_or(0);
                    let manual = fans.iter().any(|f| f.2 > 0);
                    // Zero rpm is either the firmware's fan stop or a manual
                    // target that has not spun up yet; both read as "Idle",
                    // never as a bare "0 rpm" that looks like a broken fan.
                    let stopped = top == 0;
                    fan_t.set_text(if manual { "Fans (manual)" } else { "Fans" });
                    fan_v.set_text(&if stopped {
                        "Idle".to_string()
                    } else {
                        format!("{top} rpm")
                    });
                    fan_bar.set_fraction((top as f64 / fan_max_rpm).clamp(0.0, 1.0));
                    fan_bar.set_opacity(if stopped { 0.0 } else { 1.0 });
                    let mut d = fans
                        .iter()
                        .map(|(id, rpm, _)| format!("{} {rpm}", fan_short_name(*id, aux_fan)))
                        .collect::<Vec<_>>()
                        .join(", ");
                    if legion_core::fans::rpm_estimated() && top > 0 {
                        d.push_str(", est.");
                    }
                    if stopped {
                        d = "Fans are off and start automatically when warm".into();
                    }
                    fan_d.set_text(&d);
                }
                if from_daemon {
                    if let Some(feed) = trend_feed_poll.borrow().as_ref() {
                        feed(c.max(0.0), g.max(0.0));
                    }
                }
            }
        }
        glib::ControlFlow::Continue
    });

    page
}

pub(crate) fn tint_temp(chip: &gtk::Box, temp: f64) {
    chip.remove_css_class("hot");
    chip.remove_css_class("warm");
    if temp >= 90.0 {
        chip.add_css_class("hot");
    } else if temp >= 78.0 {
        chip.add_css_class("warm");
    }
}

pub(crate) fn apply_platform_profile(
    name: &str,
    overlay: &adw::ToastOverlay,
    ppt_box: &adw::PreferencesGroup,
    ppt_scales: &PptScales,
    ppt_suppress: &Rc<Cell<bool>>,
) {
    let overlay = overlay.clone();
    let ppt_box = ppt_box.clone();
    let ppt_scales = ppt_scales.clone();
    let ppt_suppress = ppt_suppress.clone();
    let name = name.to_string();
    run_daemon_command_async(
        DaemonCommand::SetProfile(name.clone()),
        move |result| match result {
            Ok(DaemonResponse::Ok) => {
                legion_core::config::remember_platform_profile(&name);
                let show = name == "custom" && !ppt_scales.borrow().is_empty();
                ppt_box.set_visible(show);
                if show {
                    ppt_suppress.set(true);
                    for lim in legion_core::profile::all_ppt_limits() {
                        for (id, scale, label) in ppt_scales.borrow().iter() {
                            if id == lim.id {
                                scale.set_value(lim.current as f64);
                                label.set_text(&lim.value_label(lim.current));
                            }
                        }
                    }
                    ppt_suppress.set(false);
                }
                toast_ok(
                    &overlay,
                    &format!("Switched to {}", friendly_profile(&name)),
                );
            }
            Ok(DaemonResponse::Error(e)) => toast_error(&overlay, &e),
            Err(e) => toast_error(&overlay, &e),
            _ => {}
        },
    );
}

pub(crate) const MAX_POWER_WARNING: &str = "\
Max power pushes the highest turbo the BIOS allows. Without strong \
cooling the laptop can overheat, throttle, or shut down.

Continue only if you accept the risk.";

pub(crate) const CUSTOM_POWER_WARNING: &str = "\
Custom mode unlocks manual CPU and GPU power limits. Raising them increases \
heat and fan noise. Inadequate cooling can cause throttling or shutdown.";

pub(crate) fn confirm_max_power(
    parent: &impl glib::object::IsA<gtk::Widget>,
    done: impl FnOnce(bool) + 'static,
) {
    confirm_risk(
        parent,
        "Max power can overheat this laptop",
        MAX_POWER_WARNING,
        "Use Max power anyway",
        done,
    );
}

pub(crate) fn confirm_custom_power(
    parent: &impl glib::object::IsA<gtk::Widget>,
    done: impl FnOnce(bool) + 'static,
) {
    confirm_risk(
        parent,
        "Custom power limits",
        CUSTOM_POWER_WARNING,
        "Use Custom",
        done,
    );
}

pub(crate) fn ensure_custom_then_ppt(
    overlay: &adw::ToastOverlay,
    drop: &adw::ComboRow,
    choices: &[String],
    queue: &ApplyQueue,
    id: &str,
    watts: u32,
) {
    let cur = legion_core::profile::current();
    if cur != "custom" {
        if let Some(idx) = choices.iter().position(|c| c == "custom") {
            drop.set_selected(idx as u32);
        }
        let overlay = overlay.clone();
        let queue = queue.clone();
        let id = id.to_string();
        run_daemon_command_async(DaemonCommand::SetProfile("custom".into()), move |result| {
            match result {
                Ok(DaemonResponse::Ok) => {
                    queue.set_fw_attr(&id, watts.to_string());
                    legion_core::config::remember_ppt(&id, watts);
                }
                Ok(DaemonResponse::Error(e)) => toast_error(&overlay, &e),
                Err(e) => toast_error(&overlay, &e),
                _ => {}
            }
        });
        return;
    }
    queue.set_fw_attr(id, watts.to_string());
    legion_core::config::remember_ppt(id, watts);
}
