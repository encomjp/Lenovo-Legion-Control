//! Home dashboard — metric chips, power mode, custom PPT sliders, trend.

use super::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_overview(
    toast_overlay: &adw::ToastOverlay,
    apply_queue: &ApplyQueue,
    gate: &DaemonGate,
    mode_drop_slot: &Rc<RefCell<Option<adw::ComboRow>>>,
    profile_choices_slot: &Rc<RefCell<Vec<String>>>,
    ppt_group_slot: &Rc<RefCell<Option<adw::PreferencesGroup>>>,
    ppt_scales_slot: &PptScales,
    ppt_suppress_slot: &Rc<Cell<bool>>,
    trend_feed_slot: &Rc<RefCell<Option<Rc<dyn Fn(f64, f64)>>>>,
) -> gtk::Box {
    let page = page_lede("");

    let metrics = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .max_children_per_line(3)
        .min_children_per_line(1)
        .homogeneous(true)
        .column_spacing(12)
        .row_spacing(12)
        .build();
    metrics.add_css_class("metric-grid");

    let (cpu_chip, cpu_v, cpu_d) =
        metric_chip_tip("CPU", Some("Package temperature (°C) and busy percentage"));
    let (gpu_chip, gpu_v, gpu_d) =
        metric_chip_tip("GPU", Some("Discrete GPU temperature (°C) and utilization"));
    let (bat_chip, bat_v, bat_d) = metric_chip_tip(
        "Battery",
        Some("Charge level · subtitle shows the current power mode"),
    );

    metrics.append(&cpu_chip);
    metrics.append(&gpu_chip);
    metrics.append(&bat_chip);

    let fan_channels = legion_core::fans::channels();
    let mut fan_metric_labels: Vec<(u8, gtk::Label, gtk::Label)> = Vec::new();
    for ch in &fan_channels {
        let tip_txt = match ch.id {
            1 => "CPU fan speed in RPM (Auto = firmware curve)",
            2 => "GPU fan speed in RPM (Auto = firmware curve)",
            4 => "Auxiliary chassis fan speed in RPM",
            _ => "Fan speed in RPM (Auto = firmware curve)",
        };
        let short = match ch.id {
            1 => "Fan · CPU".to_string(),
            2 => "Fan · GPU".to_string(),
            4 => "Fan · Aux".to_string(),
            _ => format!("Fan · {}", ch.id),
        };
        let (chip, v, d) = metric_chip_tip(&short, Some(tip_txt));
        metrics.append(&chip);
        fan_metric_labels.push((ch.id, v, d));
    }
    page.append(&metrics);

    let power = pref_group("Power mode", None);
    tip(
        &power,
        "Quiet / Balanced / Performance / Max / Custom — Custom unlocks CPU PPT and GPU AC power sliders below",
    );

    let choices = legion_core::profile::choices();
    *profile_choices_slot.borrow_mut() = choices.clone();
    let labels: Vec<String> = choices.iter().map(|c| friendly_profile(c)).collect();
    let label_refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
    let current = legion_core::profile::current();
    let active = choices.iter().position(|c| *c == current).unwrap_or(0) as u32;

    let drop = string_combo_row("Mode", "", &label_refs, active);
    tip(&drop, profile_tooltip(&current));
    power.add(&drop);
    *mode_drop_slot.borrow_mut() = Some(drop.clone());

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
        tip(d, profile_tooltip(&name));

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

        if name == "max-power" && last_ok_n.get() != idx as u32 {
            let drop_r = d.clone();
            let choices_c = choices_c.clone();
            let profile_guard = profile_guard_n.clone();
            let last_ok = last_ok_n.clone();
            let prev = last_ok.get();
            confirm_max_power(d, move |accepted| {
                if !accepted {
                    profile_guard.set(true);
                    drop_r.set_selected(prev);
                    let prev_name = choices_c
                        .get(prev as usize)
                        .cloned()
                        .unwrap_or_else(|| "balanced".into());
                    tip(&drop_r, profile_tooltip(&prev_name));
                    profile_guard.set(false);
                    return;
                }
                apply();
                last_ok.set(
                    choices_c
                        .iter()
                        .position(|c| c == "max-power")
                        .unwrap_or(prev as usize) as u32,
                );
            });
            return;
        }

        if name == "custom" && last_ok_n.get() != idx as u32 {
            let drop_r = d.clone();
            let choices_c = choices_c.clone();
            let profile_guard = profile_guard_n.clone();
            let last_ok = last_ok_n.clone();
            let prev = last_ok.get();
            confirm_custom_power(d, move |accepted| {
                if !accepted {
                    profile_guard.set(true);
                    drop_r.set_selected(prev);
                    let prev_name = choices_c
                        .get(prev as usize)
                        .cloned()
                        .unwrap_or_else(|| "balanced".into());
                    tip(&drop_r, profile_tooltip(&prev_name));
                    profile_guard.set(false);
                    return;
                }
                apply();
                last_ok.set(
                    choices_c
                        .iter()
                        .position(|c| c == "custom")
                        .unwrap_or(prev as usize) as u32,
                );
            });
            return;
        }

        apply();
        last_ok_n.set(idx as u32);
    });
    page.append(&power);
    gate.track(&power);

    // Custom PPT / GPU sliders — shown on Home when Mode is Custom.
    attach_custom_ppt_group(
        &page,
        toast_overlay,
        apply_queue,
        gate,
        &drop,
        &choices,
        &current,
        ppt_group_slot,
        ppt_scales_slot,
        ppt_suppress_slot,
    );

    // Temperature trend — a 5-minute CPU/GPU sparkline, giving Home the
    // same at-a-glance history the KDE widget already has.
    {
        const HISTORY_CAP: usize = 150; // 2 s poll → ~5 min
        const MIN_T: f64 = 30.0;
        const MAX_T: f64 = 100.0;
        let history: Rc<RefCell<std::collections::VecDeque<(f64, f64)>>> =
            Rc::new(RefCell::new(std::collections::VecDeque::new()));

        let (sec_trend, trend_card) = section_tip("Temperature · last 5 min", None);
        tip(
            &trend_card,
            "CPU (red) and GPU (amber) package temperature, sampled every 2 s",
        );
        let area = gtk::DrawingArea::new();
        area.set_height_request(88);
        area.set_hexpand(true);
        let area_hist = history.clone();
        area.set_draw_func(move |_, cr, w, h| {
            let w = w as f64;
            let h = h as f64;
            if w < 10.0 || h < 10.0 {
                return;
            }
            // Horizontal guides at 40/55/70/85 °C.
            cr.set_line_width(1.0);
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.05);
            for t in [40.0, 55.0, 70.0, 85.0] {
                let y = h - (t - MIN_T) / (MAX_T - MIN_T) * h;
                cr.move_to(0.0, y);
                cr.line_to(w, y);
            }
            let _ = cr.stroke();

            // Legend — which trace is which (CPU red, GPU amber).
            cr.select_font_face(
                "Sans",
                gtk::cairo::FontSlant::Normal,
                gtk::cairo::FontWeight::Normal,
            );
            cr.set_font_size(10.0);
            cr.set_source_rgba(0.784, 0.063, 0.180, 0.95);
            cr.move_to(w - 56.0, 12.0);
            let _ = cr.show_text("CPU");
            cr.set_source_rgba(0.851, 0.596, 0.102, 0.95);
            cr.move_to(w - 22.0, 12.0);
            let _ = cr.show_text("GPU");

            let hist = area_hist.borrow();
            if hist.len() >= 2 {
                // Right-aligned: the line grows leftward into the fixed window.
                let dx = w / (HISTORY_CAP as f64 - 1.0);
                let x0 = w - (hist.len() as f64 - 1.0) * dx;
                let y_of = |t: f64| h - ((t.clamp(MIN_T, MAX_T) - MIN_T) / (MAX_T - MIN_T)) * h;
                for (idx, r, g, b) in [(0usize, 0.784, 0.063, 0.180), (1, 0.851, 0.596, 0.102)] {
                    cr.set_source_rgba(r, g, b, 0.95);
                    cr.set_line_width(1.6);
                    let mut first = true;
                    for (i, point) in hist.iter().enumerate() {
                        let t = if idx == 0 { point.0 } else { point.1 };
                        let x = x0 + i as f64 * dx;
                        let y = y_of(t);
                        if first {
                            cr.move_to(x, y);
                            first = false;
                        } else {
                            cr.line_to(x, y);
                        }
                    }
                    let _ = cr.stroke();
                }
            } else {
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.30);
                cr.select_font_face(
                    "Sans",
                    gtk::cairo::FontSlant::Normal,
                    gtk::cairo::FontWeight::Normal,
                );
                cr.set_font_size(11.0);
                cr.move_to(10.0, h / 2.0 + 4.0);
                let _ = cr.show_text("collecting samples…");
            }
        });
        trend_card.append(&area);
        page.append(&sec_trend);

        // Feed the history from the dashboard poll (values mirror the chips).
        let history_feed = history.clone();
        trend_feed_slot.replace(Some(Rc::new(move |cpu: f64, gpu: f64| {
            let mut hist = history_feed.borrow_mut();
            hist.push_back((cpu, gpu));
            while hist.len() > HISTORY_CAP {
                hist.pop_front();
            }
            area.queue_draw();
        })));
    }

    if let Some(pct) = legion_core::battery::capacity() {
        bat_v.set_text(&format!("{pct}%"));
        bat_d.set_text(&legion_core::battery::status().unwrap_or_default());
    }

    let cpu_chip_c = cpu_chip.clone();
    let gpu_chip_c = gpu_chip.clone();
    let trend_feed_poll = trend_feed_slot.clone();
    let mode_drop_poll = drop.clone();
    let choices_poll = choices.clone();
    let profile_guard_poll = profile_guard.clone();
    let last_ok_poll = last_ok.clone();
    let last_firmware_mode = Rc::new(RefCell::new(current));
    let _ = legion_core::sensors::sample_cpu_usage_pct();

    // Data collected off the main thread each tick. IPC and nvidia-smi can
    // block for seconds — they must never run inside the GTK main loop.
    struct DashboardPoll {
        cpu_pct: f64,
        gpu_pct: f64,
        firmware_mode: Option<String>,
        sensors: Option<legion_core::sensors::SensorReadings>,
        cpu_w: Option<f64>,
        local: Option<legion_core::sensors::SensorReadings>,
        local_battery: Option<(u32, String)>,
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
            let (local, local_battery) = if sensors.is_none() {
                (
                    Some(legion_core::sensors::read_all()),
                    legion_core::battery::capacity()
                        .map(|pct| (pct, legion_core::battery::status().unwrap_or_default())),
                )
            } else {
                (None, None)
            };
            if dash_tx
                .send(DashboardPoll {
                    cpu_pct,
                    gpu_pct,
                    firmware_mode,
                    sensors,
                    cpu_w,
                    local,
                    local_battery,
                })
                .is_err()
            {
                break; // UI closed
            }
        })
        .ok();

    // Drain poll results on the main thread and update the widgets.
    glib::timeout_add_local(Duration::from_millis(200), move || {
        while let Ok(poll) = dash_rx.try_recv() {
            // Fn+Q changes happen outside this process.  Poll the daemon's
            // hardware-authoritative profile and update the row without allowing
            // the programmatic selection change to emit a SetProfile command.
            if let Some(firmware_mode) = poll.firmware_mode {
                let changed = *last_firmware_mode.borrow() != firmware_mode;
                if changed {
                    *last_firmware_mode.borrow_mut() = firmware_mode.clone();
                    if let Some(index) = choices_poll.iter().position(|c| c == &firmware_mode) {
                        let index = index as u32;
                        profile_guard_poll.set(true);
                        mode_drop_poll.set_selected(index);
                        mode_drop_poll.set_subtitle(profile_blurb(&firmware_mode));
                        tip(&mode_drop_poll, profile_tooltip(&firmware_mode));
                        last_ok_poll.set(index);
                        profile_guard_poll.set(false);
                        legion_core::config::remember_platform_profile(&firmware_mode);
                    }
                }
            }

            if let Some(s) = poll.sensors {
                let c = if s.ec_cpu > 0.0 { s.ec_cpu } else { s.cpu_temp };
                cpu_v.set_text(&format!("{c:.0} °C"));
                tint_temp(&cpu_chip_c, c);
                cpu_d.set_text(&match poll.cpu_w {
                    Some(w) => format!("{:.0}% · {w:.0} W", poll.cpu_pct),
                    None => format!("{:.0}%", poll.cpu_pct),
                });

                let g = if s.dgpu_temp < 0.0 {
                    s.ec_gpu
                } else {
                    s.dgpu_temp.max(s.ec_gpu)
                };
                if g > 0.0 {
                    gpu_v.set_text(&format!("{g:.0} °C"));
                    tint_temp(&gpu_chip_c, g);
                } else {
                    // No dGPU present (Radeon-only LOQ) or powered down with
                    // no EC reading: show N/A instead of a bogus "0 °C".
                    gpu_v.set_text("N/A");
                    tint_temp(&gpu_chip_c, 0.0);
                }
                if s.dgpu_power >= 0.0 {
                    gpu_d.set_text(&format!("{:.0}% · {:.0} W", poll.gpu_pct, s.dgpu_power));
                } else {
                    gpu_d.set_text(&format!("{:.0}%", poll.gpu_pct));
                }

                set_fan_metrics_from_sensors(&fan_metric_labels, &s);
                if let Some(feed) = trend_feed_poll.borrow().as_ref() {
                    feed(c.max(0.0), g.max(0.0));
                }
                bat_v.set_text(&format!("{}%", s.battery_pct));
                bat_d.set_text(&friendly_profile(&s.profile));
            } else {
                if let Some((pct, status)) = &poll.local_battery {
                    bat_v.set_text(&format!("{pct}%"));
                    bat_d.set_text(status);
                }
                if let Some(local) = &poll.local {
                    let c = if local.ec_cpu > 0.0 {
                        local.ec_cpu
                    } else {
                        local.cpu_temp
                    };
                    if c > 0.0 {
                        cpu_v.set_text(&format!("{c:.0} °C"));
                        tint_temp(&cpu_chip_c, c);
                        cpu_d.set_text(&format!("{:.0}%", poll.cpu_pct));
                    }
                    let g = if local.dgpu_temp < 0.0 {
                        local.ec_gpu
                    } else {
                        local.dgpu_temp.max(local.ec_gpu)
                    };
                    if g > 0.0 {
                        gpu_v.set_text(&format!("{g:.0} °C"));
                        tint_temp(&gpu_chip_c, g);
                        gpu_d.set_text(&format!("{:.0}%", poll.gpu_pct));
                    }
                    set_fan_metrics_from_sensors(&fan_metric_labels, local);
                }
            }
        }
        glib::ControlFlow::Continue
    });

    page
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn attach_custom_ppt_group(
    page: &gtk::Box,
    toast_overlay: &adw::ToastOverlay,
    apply_queue: &ApplyQueue,
    gate: &DaemonGate,
    drop: &adw::ComboRow,
    choices: &[String],
    current: &str,
    ppt_group_slot: &Rc<RefCell<Option<adw::PreferencesGroup>>>,
    ppt_scales_slot: &PptScales,
    ppt_suppress_slot: &Rc<Cell<bool>>,
) {
    let caps = legion_core::device::detect().capabilities;
    let peak_tgp = caps.peak_gpu_w.unwrap_or(175);
    let peak_src = caps.peak_gpu_source;
    let ppt_group = pref_group("Custom power limits", None);
    tip(
        &ppt_group,
        &format!(
            "CPU PPT + GPU AC processing-power target. Peak GPU TGP is {peak_tgp} W ({peak_src}) in Performance/Max — \
             the AC offset attribute max is set by the BIOS (often 130 W), not the absolute TGP."
        ),
    );

    let peak_row = adw::ActionRow::builder()
        .title("Peak GPU TGP (this laptop)")
        .subtitle(format!("{peak_tgp} W · {peak_src}"))
        .activatable(false)
        .build();
    tip(
        &peak_row,
        &format!(
            "Detected peak is {peak_tgp} W ({peak_src}). Performance and Max Power unlock that ceiling. \
             Custom mode uses separate firmware attributes; the AC power target max is BIOS-limited."
        ),
    );
    ppt_group.add(&peak_row);

    ppt_scales_slot.borrow_mut().clear();
    for lim in legion_core::profile::all_ppt_limits() {
        let row = adw::ActionRow::builder()
            .title(lim.label)
            .activatable(false)
            .build();
        let ppt_tip = match lim.id {
            "ppt_pl1_spl" => format!(
                "Sustained power limit (SPL) — long-term CPU watts · range {}",
                lim.range_label()
            ),
            "ppt_pl2_sppt" => format!(
                "Slow boost (SPPT) — medium burst · range {}",
                lim.range_label()
            ),
            "ppt_pl3_fppt" => format!(
                "Peak burst (FPPT) — short turbo · range {}",
                lim.range_label()
            ),
            "ppt_cpu_cl" => format!(
                "CPU cross-load share · range {}",
                lim.range_label()
            ),
            "cpu_temp" => format!(
                "CPU thermal cutoff — firmware throttles near this temperature · \
                 independent of, and stacking with, the software Thermal governor · \
                 range {} (°C, not watts)",
                lim.range_label()
            ),
            "gpu_temp" => format!(
                "GPU thermal cutoff — firmware throttles near this temperature · \
                 independent of, and stacking with, the software Thermal governor · \
                 range {} (°C, not watts)",
                lim.range_label()
            ),
            "gpu_nv_ac_offset" => format!(
                "GPU AC total processing power target (BIOS attribute). Firmware range {}. \
                 This is not the absolute {peak_tgp} W NVIDIA TGP — Performance/Max already use {peak_tgp} W.",
                lim.range_label()
            ),
            "gpu_nv_ctgp" => format!(
                "GPU configurable TGP (cTGP) · range {}",
                lim.range_label()
            ),
            "gpu_nv_ppab" => format!(
                "GPU PPAB boost · range {}",
                lim.range_label()
            ),
            "gpu_nv_cpu_boost" => format!(
                "GPU↔CPU dynamic boost share · range {}",
                lim.range_label()
            ),
            _ => format!("{} · {}", lim.label, lim.range_label()),
        };
        tip(&row, &ppt_tip);

        let val = gtk::Label::new(Some(&lim.value_label(lim.current)));
        val.add_css_class("dim-label");
        val.add_css_class("numeric");
        val.add_css_class("scale-value");
        tip(&val, &ppt_tip);

        let adj = gtk::Adjustment::new(
            lim.current as f64,
            lim.min as f64,
            lim.max as f64,
            1.0,
            5.0,
            0.0,
        );
        let scale = gtk::Scale::new(Orientation::Horizontal, Some(&adj));
        scale.set_draw_value(false);
        scale.set_hexpand(true);
        scale.set_width_request(160);
        tip(&scale, &ppt_tip);
        row.add_suffix(&scale);
        row.add_suffix(&val);
        ppt_group.add(&row);

        let overlay = toast_overlay.clone();
        let queue = apply_queue.clone();
        let id = lim.id.to_string();
        let lim_max = lim.max;
        let unit_sym = lim.unit.symbol();
        let unit_celsius = lim.unit == legion_core::profile::LimitUnit::Celsius;
        let lim_label = lim.label.to_string();
        let val_l = val.clone();
        let drop_c = drop.clone();
        let choices_c = choices.to_vec();
        let suppress = ppt_suppress_slot.clone();
        let ppt_warned = Rc::new(Cell::new(false));
        let scale_c = scale.clone();
        scale.connect_value_changed(move |s| {
            if suppress.get() {
                return;
            }
            let v = s.value().round() as u32;
            val_l.set_text(&format!("{v} {unit_sym}"));
            let warn_at = (lim_max as f64 * 0.92).round() as u32;
            if v >= warn_at && !ppt_warned.get() {
                ppt_warned.set(true);
                let scale_r = scale_c.clone();
                let suppress_r = suppress.clone();
                let prev = (warn_at.saturating_sub(5)).max(lim_max / 2);
                let overlay = overlay.clone();
                let queue = queue.clone();
                let id = id.clone();
                let lim_label = lim_label.clone();
                let choices_c = choices_c.clone();
                let drop_c = drop_c.clone();
                let val_l_c = val_l.clone();
                let ppt_warned_c = ppt_warned.clone();
                confirm_risk(
                    s,
                    "High power limit",
                    &format!(
                        "{lim_label} at {v} {unit_sym} is near the firmware maximum ({lim_max} {unit_sym}).\n\n\
                         {}\n\n\
                         Continue only if cooling is strong.",
                        if unit_celsius {
                            "A high cutoff lets the CPU/GPU run hotter before firmware throttling kicks in."
                        } else {
                            "Sustained high limits increase heat and fan noise."
                        }
                    ),
                    "Use high limit",
                    move |ok| {
                        if !ok {
                            suppress_r.set(true);
                            scale_r.set_value(prev as f64);
                            val_l_c.set_text(&format!("{prev} {unit_sym}"));
                            ppt_warned_c.set(false);
                            suppress_r.set(false);
                            return;
                        }
                        ensure_custom_then_ppt(&overlay, &drop_c, &choices_c, &queue, &id, v);
                    },
                );
                return;
            }
            ensure_custom_then_ppt(&overlay, &drop_c, &choices_c, &queue, &id, v);
        });
        ppt_scales_slot
            .borrow_mut()
            .push((lim.id.to_string(), scale, val));
    }

    let ppt_visible = current == "custom" && !ppt_scales_slot.borrow().is_empty();
    ppt_group.set_visible(ppt_visible);
    gate.track(&ppt_group);
    if !ppt_scales_slot.borrow().is_empty() {
        page.append(&ppt_group);
    }
    *ppt_group_slot.borrow_mut() = Some(ppt_group);
}

pub(crate) fn set_fan_metric(value: &gtk::Label, detail: &gtk::Label, rpm: u32, target: u32) {
    if target == 0 {
        if rpm == 0 {
            value.set_text("Auto");
            detail.set_text("Firmware curve");
        } else {
            value.set_text(&format!("{rpm}"));
            detail.set_text("Auto");
        }
    } else if rpm == 0 {
        value.set_text(&format!("~{target}"));
        detail.set_text("Manual target");
    } else {
        value.set_text(&format!("{rpm}"));
        detail.set_text(&format!("→ {target} RPM"));
    }
}

pub(crate) fn set_fan_metrics_from_sensors(
    labels: &[(u8, gtk::Label, gtk::Label)],
    s: &legion_core::sensors::SensorReadings,
) {
    for (id, value, detail) in labels {
        let (rpm, target) = match id {
            1 => (s.fan1_rpm, s.fan1_target),
            2 => (s.fan2_rpm, s.fan2_target),
            4 => (s.fan4_rpm, s.fan4_target),
            _ => (
                legion_core::fans::read_rpm(*id).unwrap_or(0),
                legion_core::fans::read_target(*id).unwrap_or(0),
            ),
        };
        set_fan_metric(value, detail, rpm, target);
    }
}

/// One-line preview of what a preset will change — shown under the picker so
/// Load isn't apply-and-pray.
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
Max Power (Extreme) pushes the highest turbo the BIOS allows. Without strong \
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
        "Max Power can overheat this laptop",
        MAX_POWER_WARNING,
        "Use Max Power anyway",
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
