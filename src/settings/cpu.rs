//! CPU pages — features, tuning (thermal/curve-optimizer/stability), power limits.

use super::*;

#[derive(Clone)]
pub(crate) struct CurveOptimizerUi {
    status_row: adw::ActionRow,
    offset_scale: gtk::Scale,
    offset_value: gtk::Label,
    apply_button: gtk::Button,
    reset_button: gtk::Button,
    install_button: gtk::Button,
    refresh_button: gtk::Button,
    startup_switch: adw::SwitchRow,
    persistence_suppress: Rc<Cell<bool>>,
}

pub(crate) fn offsets_text(values: &[i16]) -> String {
    match values.first().copied() {
        None => "—".into(),
        Some(first) if values.iter().all(|value| *value == first) => {
            format!("All cores: {first}")
        }
        Some(_) => "Mixed".into(),
    }
}

pub(crate) fn update_curve_optimizer_ui(
    ui: &CurveOptimizerUi,
    result: Result<DaemonResponse, String>,
) -> Result<(), String> {
    let status = match result {
        Ok(DaemonResponse::CurveOptimizer(status)) => status,
        Ok(DaemonResponse::Error(error)) => return Err(error),
        Ok(other) => return Err(format!("Unexpected daemon response: {other:?}")),
        Err(error) => return Err(error),
    };

    ui.refresh_button.set_sensitive(true);
    ui.status_row.set_subtitle(&status.reason);
    let needs_install = !status.available && status.reason.contains("ryzen_smu driver is not loaded");
    ui.install_button.set_visible(needs_install);
    ui.apply_button.set_sensitive(status.available);
    ui.reset_button.set_sensitive(status.available);
    ui.offset_scale.set_sensitive(status.available);
    // Gate startup: must verify an offset first — daemon enforces this too, but UI should be honest early.
    // Keep switch sensitive only if available; subtitle hint is handled in persistence UI refresh.
    ui.startup_switch.set_sensitive(status.available);

    if !status.available {
        return Ok(());
    }

    let current_val = status
        .current
        .first()
        .copied()
        .filter(|v| status.current.iter().all(|x| x == v));
    let baseline_val = status
        .boot_baseline
        .first()
        .copied()
        .filter(|v| status.boot_baseline.iter().all(|x| x == v));
    let prev = status.previous.filter(|p| Some(*p) != current_val);
    // Keep this unambiguous: the applied offset is live (the slider mirrors
    // it); baseline/history are only context. Never print the same number
    // twice — "reset -4 · previous -4" reads like the value reverted.
    let mut subtitle = format!(
        "Applied {}",
        offsets_text(&status.current).replace("All cores: ", "")
    );
    if let Some(b) = baseline_val {
        if Some(b) != current_val {
            subtitle.push_str(&format!(" · Reset goes back to {b}"));
        }
    }
    if let Some(p) = prev {
        if Some(p) != baseline_val {
            subtitle.push_str(&format!(" · was {p} before"));
        }
    }
    ui.status_row.set_subtitle(&subtitle);
    ui.offset_scale
        .set_range(status.minimum as f64, status.maximum as f64);
    if let Some(current) = status
        .current
        .first()
        .copied()
        .filter(|first| status.current.iter().all(|value| value == first))
    {
        ui.offset_scale.set_value(current as f64);
        ui.offset_value.set_text(&current.to_string());
    } else {
        ui.offset_value.set_text("Mixed");
    }
    Ok(())
}

pub(crate) fn update_curve_optimizer_persistence_ui(
    ui: &CurveOptimizerUi,
    result: Result<DaemonResponse, String>,
) -> Result<(), String> {
    let status = match result {
        Ok(DaemonResponse::CurveOptimizerPersistence(status)) => status,
        Ok(DaemonResponse::Error(error)) => return Err(error),
        Ok(other) => return Err(format!("Unexpected daemon response: {other:?}")),
        Err(error) => return Err(error),
    };
    ui.persistence_suppress.set(true);
    ui.startup_switch.set_active(status.enabled);
    ui.startup_switch.set_sensitive(!status.recovery_blocked);
    if status.recovery_blocked {
        ui.startup_switch
            .set_subtitle("Disabled after an interrupted validation window");
    } else if status.enabled {
        ui.startup_switch.set_subtitle(&format!(
            "Applies {} after a 60-second delay",
            status.offset
        ));
    } else {
        // Hint flow: must Apply & verify first before enabling startup.
        let hint = if ui.offset_scale.is_sensitive() {
            "Off · Apply and verify an offset first"
        } else {
            "Off"
        };
        ui.startup_switch.set_subtitle(hint);
    }
    ui.persistence_suppress.set(false);
    Ok(())
}

pub(crate) fn refresh_curve_optimizer_persistence(
    ui: &CurveOptimizerUi,
    toast_overlay: Option<&adw::ToastOverlay>,
) {
    let ui = ui.clone();
    let overlay = toast_overlay.cloned();
    run_daemon_command_async(DaemonCommand::GetCurveOptimizerPersistence, move |result| {
        if let Err(error) = update_curve_optimizer_persistence_ui(&ui, result) {
            ui.persistence_suppress.set(true);
            ui.startup_switch.set_active(false);
            ui.startup_switch.set_sensitive(false);
            ui.startup_switch.set_subtitle(&error);
            ui.persistence_suppress.set(false);
            if let Some(overlay) = overlay {
                toast_error(&overlay, &error);
            }
        }
    });
}

pub(crate) fn refresh_curve_optimizer(
    ui: &CurveOptimizerUi,
    toast_overlay: Option<&adw::ToastOverlay>,
) {
    ui.refresh_button.set_sensitive(false);
    refresh_curve_optimizer_persistence(ui, toast_overlay);
    let ui = ui.clone();
    let overlay = toast_overlay.cloned();
    run_daemon_command_async(DaemonCommand::GetCurveOptimizer, move |result| {
        if let Err(error) = update_curve_optimizer_ui(&ui, result) {
            ui.refresh_button.set_sensitive(true);
            ui.status_row.set_subtitle(&error);
            ui.apply_button.set_sensitive(false);
            ui.reset_button.set_sensitive(false);
            ui.offset_scale.set_sensitive(false);
            if let Some(overlay) = overlay {
                toast_error(&overlay, &error);
            }
        }
    });
}

pub(crate) fn set_curve_optimizer_persistence_async(
    ui: CurveOptimizerUi,
    overlay: adw::ToastOverlay,
    enabled: bool,
    offset: i16,
) {
    ui.startup_switch.set_sensitive(false);
    let refresh_ui = ui.clone();
    let refresh_overlay = overlay.clone();
    run_daemon_command_async(
        DaemonCommand::SetCurveOptimizerPersistence {
            enabled,
            offset,
            acknowledge: true,
        },
        move |result| match update_curve_optimizer_persistence_ui(&ui, result) {
            Ok(()) => {
                if enabled {
                    toast_ok(&overlay, "Startup undervolt enabled");
                } else {
                    toast_ok(&overlay, "Startup undervolt disabled");
                }
            }
            Err(error) => {
                toast_error(&overlay, &error);
                refresh_curve_optimizer_persistence(&refresh_ui, Some(&refresh_overlay));
            }
        },
    );
}

pub(crate) fn build_curve_optimizer(
    toast_overlay: &adw::ToastOverlay,
    gate: &DaemonGate,
) -> adw::PreferencesGroup {
    let group = pref_group("Curve Optimizer", None);
    tip(
        &group,
        "All-core CPU offset via ryzen_smu. Unstable values can crash the system.",
    );

    let status_row = adw::ActionRow::builder()
        .title("AMD backend")
        .subtitle("Checking…")
        .activatable(false)
        .build();
    let install_button = primary_button_tip(
        "Install backend",
        Some("Installs the bundled ryzen_smu driver through PolicyKit and DKMS"),
    );
    install_button.set_visible(false);
    let refresh_button = gtk::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text("Refresh")
        .valign(Align::Center)
        .build();
    status_row.add_suffix(&install_button);
    status_row.add_suffix(&refresh_button);
    group.add(&status_row);

    let offset_row = adw::ActionRow::builder()
        .title("All-core offset")
        .subtitle("")
        .activatable(false)
        .build();
    tip(
        &offset_row,
        "All-core Curve Optimizer offset (-30..0, more negative = more undervolt)",
    );
    let offset_value = gtk::Label::new(Some("—"));
    offset_value.add_css_class("numeric");
    offset_value.add_css_class("scale-value");
    offset_value.set_width_chars(4);
    offset_value.set_xalign(1.0);
    let adjustment = gtk::Adjustment::new(0.0, -30.0, 0.0, 1.0, 5.0, 0.0);
    let offset_scale = gtk::Scale::new(Orientation::Horizontal, Some(&adjustment));
    offset_scale.set_draw_value(false);
    offset_scale.set_digits(0);
    offset_scale.set_hexpand(true);
    offset_scale.set_width_request(220);
    offset_scale.set_sensitive(false);
    offset_row.add_suffix(&offset_scale);
    offset_row.add_suffix(&offset_value);
    group.add(&offset_row);

    let actions_row = adw::ActionRow::builder()
        .title("Current session")
        .activatable(false)
        .build();
    tip(
        &actions_row,
        "Verified by firmware readback via ryzen_smu (daemon checks SMU probe before applying)",
    );
    let reset_button = gtk::Button::builder()
        .label("Reset")
        .valign(Align::Center)
        .sensitive(false)
        .build();
    reset_button.add_css_class("pill-btn");
    let apply_button = gtk::Button::builder()
        .label("Apply")
        .valign(Align::Center)
        .sensitive(false)
        .build();
    apply_button.add_css_class("destructive-action");
    apply_button.add_css_class("pill-btn");
    actions_row.add_suffix(&reset_button);
    actions_row.add_suffix(&apply_button);
    group.add(&actions_row);

    let startup_switch = adw::SwitchRow::builder()
        .title("Apply after startup")
        .subtitle("Checking…")
        .sensitive(false)
        .build();
    group.add(&startup_switch);
    let persistence_suppress = Rc::new(Cell::new(false));

    let ui = CurveOptimizerUi {
        status_row,
        offset_scale,
        offset_value,
        apply_button,
        reset_button,
        install_button,
        refresh_button,
        startup_switch,
        persistence_suppress,
    };

    let value_label = ui.offset_value.clone();
    ui.offset_scale.connect_value_changed(move |scale| {
        value_label.set_text(&(scale.value().round() as i16).to_string());
    });

    let ui_startup = ui.clone();
    let overlay = toast_overlay.clone();
    ui.startup_switch.connect_active_notify(move |row| {
        if ui_startup.persistence_suppress.get() {
            return;
        }
        let enabled = row.is_active();
        let offset = ui_startup.offset_scale.value().round() as i16;
        if !enabled {
            set_curve_optimizer_persistence_async(
                ui_startup.clone(),
                overlay.clone(),
                false,
                offset,
            );
            return;
        }

        let ui = ui_startup.clone();
        let overlay = overlay.clone();
        confirm_risk(
            row,
            "Apply undervolt after startup?",
            "Legion Control waits 60 seconds before applying it. If validation is interrupted, the next start disables it automatically.",
            "Enable",
            move |accepted| {
                if accepted {
                    set_curve_optimizer_persistence_async(ui, overlay, true, offset);
                } else {
                    ui.persistence_suppress.set(true);
                    ui.startup_switch.set_active(false);
                    ui.persistence_suppress.set(false);
                }
            },
        );
    });

    let ui_refresh = ui.clone();
    let overlay = toast_overlay.clone();
    ui.refresh_button
        .connect_clicked(move |_| refresh_curve_optimizer(&ui_refresh, Some(&overlay)));

    let ui_install = ui.clone();
    let overlay = toast_overlay.clone();
    ui.install_button.connect_clicked(move |button| {
        button.set_sensitive(false);
        button.set_label("Installing…");
        let button = button.clone();
        let ui = ui_install.clone();
        let overlay = overlay.clone();
        run_setup_helper("install-ryzen-smu", move |result| match result {
            Ok(_) => {
                button.set_label("Install backend");
                button.set_sensitive(true);
                toast_ok(&overlay, "AMD tuning backend installed");
                refresh_curve_optimizer(&ui, Some(&overlay));
            }
            Err(error) => {
                button.set_label("Install backend");
                button.set_sensitive(true);
                toast_error(&overlay, &error);
            }
        });
    });

    let ui_apply = ui.clone();
    let overlay = toast_overlay.clone();
    ui.apply_button.connect_clicked(move |button| {
        let offset = ui_apply.offset_scale.value().round() as i16;
        let parent = button.clone();
        let ui = ui_apply.clone();
        let overlay = overlay.clone();
        confirm_risk(
            &parent,
            "Apply Curve Optimizer offset?",
            &format!(
                "Apply {offset} to all CPU cores until reboot?\n\nAn unstable offset can crash the system or corrupt active work."
            ),
            &format!("Apply {offset}"),
            move |confirmed| {
                if !confirmed {
                    return;
                }
                ui.apply_button.set_sensitive(false);
                ui.apply_button.set_label("Applying…");
                let ui_done = ui.clone();
                let overlay = overlay.clone();
                run_daemon_command_async(
                    DaemonCommand::SetCurveOptimizer {
                        offset,
                        acknowledge: true,
                    },
                    move |result| {
                        ui_done.apply_button.set_label("Apply");
                        match update_curve_optimizer_ui(&ui_done, result) {
                            Ok(()) => {
                                toast_ok(&overlay, "Curve Optimizer offset verified");
                                refresh_curve_optimizer_persistence(&ui_done, Some(&overlay));
                            }
                            Err(error) => {
                                ui_done.apply_button.set_sensitive(true);
                                toast_error(&overlay, &error);
                            }
                        }
                    },
                );
            },
        );
    });

    let ui_reset = ui.clone();
    let overlay = toast_overlay.clone();
    ui.reset_button.connect_clicked(move |button| {
        let parent = button.clone();
        let ui = ui_reset.clone();
        let overlay = overlay.clone();
        confirm_risk(
            &parent,
            "Reset Curve Optimizer?",
            "Restore the values observed when the daemon started?",
            "Reset",
            move |confirmed| {
                if !confirmed {
                    return;
                }
                ui.reset_button.set_sensitive(false);
                ui.reset_button.set_label("Resetting…");
                let ui_done = ui.clone();
                let overlay = overlay.clone();
                run_daemon_command_async(
                    DaemonCommand::ResetCurveOptimizerAcknowledged { acknowledge: true },
                    move |result| {
                        ui_done.reset_button.set_label("Reset");
                        match update_curve_optimizer_ui(&ui_done, result) {
                            Ok(()) => {
                                toast_ok(&overlay, "Curve Optimizer baseline restored");
                                refresh_curve_optimizer_persistence(&ui_done, Some(&overlay));
                            }
                            Err(error) => {
                                ui_done.reset_button.set_sensitive(true);
                                toast_error(&overlay, &error);
                            }
                        }
                    },
                );
            },
        );
    });

    refresh_curve_optimizer(&ui, None);
    gate.track(&group);
    group
}

pub(crate) fn build_cpu_features_page(
    toast_overlay: &adw::ToastOverlay,
    gate: &DaemonGate,
) -> gtk::Box {
    let page = page_lede("");
    let features = build_cpu_features(toast_overlay);
    gate.track(&features);
    page.append(&features);
    page
}

pub(crate) fn build_cpu_power_page(
    toast_overlay: &adw::ToastOverlay,
    go_home: &Rc<dyn Fn(&'static str, &'static str)>,
) -> gtk::Box {
    let page = page_lede("");
    let all_limits = legion_core::profile::all_ppt_limits();
    if all_limits.is_empty() {
        let empty_group = pref_group("Power limits", None);
        let row = adw::ActionRow::builder()
            .title("Not available on this model")
            .subtitle("Configurable CPU/GPU wattage targets (PPT/TGP) are not exposed by this system's ACPI/WMI firmware.")
            .activatable(false)
            .build();
        empty_group.add(&row);
        page.append(&empty_group);
        return page;
    }
    let mode = legion_core::profile::current();

    // Guidance row — live wording, one clear way out.
    let guide = pref_group("Power limits", None);
    tip(
        &guide,
        "CPU PPT and GPU power sliders are edited on Home — they unlock when Power mode is Custom",
    );
    let lock_row = adw::ActionRow::builder()
        .title(if mode == "custom" {
            "Custom mode is active"
        } else {
            "Locked outside Custom mode"
        })
        .activatable(false)
        .build();
    tip(
        &lock_row,
        "Firmware PPT/attribute writes are only accepted while the EC is in Custom mode — other modes use firmware defaults",
    );
    let go_home = go_home.clone();
    let overlay = toast_overlay.clone();
    let go_home_btn = primary_button_tip(
        "Switch to Custom & edit",
        Some("Switches Power mode to Custom (unlocks the watt sliders on Home) and jumps there"),
    );
    go_home_btn.connect_clicked(move |_| {
        // One click instead of three: apply the Custom profile here, then
        // jump Home where the sliders are unlocked. The Home mode picker
        // syncs itself from firmware on its next poll.
        go_home("overview", "Home");
        let overlay = overlay.clone();
        run_daemon_command_async(DaemonCommand::SetProfile("custom".into()), move |result| {
            match result {
                Ok(DaemonResponse::Ok) => {
                    legion_core::config::remember_platform_profile("custom");
                    toast_ok(&overlay, "Power mode → Custom — sliders unlocked");
                }
                Ok(DaemonResponse::Error(e)) => toast_error(&overlay, &e),
                Err(e) => toast_error(&overlay, &e),
                _ => {}
            }
        });
    });
    lock_row.add_suffix(&go_home_btn);
    guide.add(&lock_row);
    page.append(&guide);

    // Live preview of the same sliders Home shows — greyed out here. Values
    // mirror the firmware read so the page informs instead of pointing.
    let preview = pref_group("Custom watts", None);
    tip(
        &preview,
        "Read-only mirror of the Custom-mode limits — edit them on Home",
    );
    for lim in legion_core::profile::all_ppt_limits() {
        let row = adw::ActionRow::builder()
            .title(lim.label)
            .activatable(false)
            .build();
        tip(&row, &format!("{} · {}", lim.label, lim.range_label()));
        let val = gtk::Label::new(Some(&lim.value_label(lim.current)));
        val.add_css_class("dim-label");
        val.add_css_class("numeric");
        val.add_css_class("scale-value");
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
        tip(&scale, &format!("{} · {}", lim.label, lim.range_label()));
        row.add_suffix(&scale);
        row.add_suffix(&val);
        preview.add(&row);
    }
    preview.set_sensitive(false);
    page.append(&preview);
    page
}

pub(crate) fn autostart_enabled() -> bool {
    dirs::config_dir()
        .map(|d| {
            d.join("autostart")
                .join("com.encomjp.legion-settings.desktop")
        })
        .is_some_and(|p| p.exists())
}

pub(crate) fn set_autostart(enabled: bool) -> Result<(), String> {
    let Some(dir) = dirs::config_dir().map(|d| d.join("autostart")) else {
        return Err("Cannot locate autostart directory".into());
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create autostart dir: {e}"))?;
    let dest = dir.join("com.encomjp.legion-settings.desktop");
    if enabled {
        // Resolve installed .desktop first (survives packaging), fall back to source tree.
        let installed = [
            "/usr/local/share/applications/com.encomjp.legion-settings.desktop",
            "/usr/share/applications/com.encomjp.legion-settings.desktop",
        ];
        let mut content: Option<String> = None;
        for p in installed {
            if let Ok(s) = std::fs::read_to_string(p) {
                content = Some(s);
                break;
            }
        }
        let mut content = content.unwrap_or_else(|| {
            std::fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("data/gui/com.encomjp.legion-settings.desktop"),
            )
            .unwrap_or_else(|_| {
                "[Desktop Entry]\nName=Legion Control\nExec=legion-settings\nIcon=com.encomjp.legion-settings\nType=Application\n".into()
            })
        });
        // Ensure it autostarts hidden to tray — no window pop on login.
        if !content.contains("Exec=") {
            content.push_str("\nExec=legion-settings --hidden\n");
        } else if !content.contains("--hidden") {
            content = content.replace("Exec=legion-settings", "Exec=legion-settings --hidden");
        }
        if !content.contains("X-GNOME-Autostart-enabled") {
            if !content.ends_with('\n') {
                content.push('\n');
            }
            content.push_str("X-GNOME-Autostart-enabled=true\n");
        }
        if !content.contains("Hidden=") {
            content.push_str("Hidden=false\n");
        }
        std::fs::write(&dest, content).map_err(|e| format!("Cannot write autostart entry: {e}"))?;
    } else {
        let _ = std::fs::remove_file(&dest);
    }
    Ok(())
}

pub(crate) fn build_cpu_tuning_page(
    toast_overlay: &adw::ToastOverlay,
    gate: &DaemonGate,
) -> gtk::Box {
    let page = page_lede("");
    // Cards on top — squares/overview style, then tuning controls below. Tooltips (hover) carry the how-to.
    let thermal = build_thermal_card(toast_overlay, gate);
    let co = build_curve_optimizer(toast_overlay, gate);
    // Autostart as a small row on this Tuning tab (hover tip explains it).
    let autostart_row = adw::SwitchRow::builder()
        .title("Launch at login")
        .subtitle(if autostart_enabled() {
            "On · opens on login"
        } else {
            "Off"
        })
        .active(autostart_enabled())
        .build();
    tip(
        &autostart_row,
        "Adds Legion Control to Desktop autostart (~/.config/autostart) so tuning controls are available after login. System daemon (fans/profile) starts separately via systemd.",
    );
    let autostart_toast = toast_overlay.clone();
    autostart_row.connect_active_notify(move |row| {
        let on = row.is_active();
        match set_autostart(on) {
            Ok(()) => {
                row.set_subtitle(if on { "On · opens on login" } else { "Off" });
                toast_ok(
                    &autostart_toast,
                    if on {
                        "Autostart enabled"
                    } else {
                        "Autostart disabled"
                    },
                );
            }
            Err(e) => {
                row.set_active(!on);
                toast_error(&autostart_toast, &e);
            }
        }
    });
    let autostart_group = pref_group("Startup", None);
    tip(
        &autostart_group,
        "Login autostart for the Settings app (user session). The root daemon autostarts via systemd regardless.",
    );
    autostart_group.add(&autostart_row);
    // Order: chips-first tuning (thermal) already has its own chips on top internally;
    // then undervolt (AMD only) + stability + autostart.
    let is_intel = legion_core::device::detect().cpu_model.to_lowercase().contains("intel");
    page.append(&thermal);
    if !is_intel {
        page.append(&co);
    }
    page.append(&build_stability_group(toast_overlay));
    page.append(&autostart_group);
    page
}

pub(crate) fn build_stability_group(toast_overlay: &adw::ToastOverlay) -> adw::PreferencesGroup {
    // Reuse the existing stability page's group — extract the group only, no page wrapper.
    let page = build_cpu_stability_page(toast_overlay);
    // build_cpu_stability_page returns a gtk::Box with one PreferencesGroup child; unwrap it.
    // Fallback: build inline if structure changes — keep a minimal group.
    if let Some(child) = page.first_child() {
        if let Ok(group) = child.clone().downcast::<adw::PreferencesGroup>() {
            return group;
        }
    }
    // Fallback recompose — should never hit.
    let g = pref_group("Stability test", None);
    let row = adw::ActionRow::builder().title("Stability test").build();
    g.add(&row);
    g
}

pub(crate) const STABILITY_TEST_SECS: u64 = 300;

pub(crate) enum StabilityEvent {
    Progress(u64),
    Finished { cancelled: bool, errors: u64 },
}

pub(crate) fn stability_memory_pass(seed: u64, memory: &mut [u64]) -> bool {
    for (index, value) in memory.iter_mut().enumerate() {
        let mixed = seed
            .wrapping_add((index as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15))
            .rotate_left((index & 63) as u32);
        *value = mixed ^ 0xa5a5_5a5a_d3c3_b4b4;
    }
    memory.iter().enumerate().all(|(index, value)| {
        let mixed = seed
            .wrapping_add((index as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15))
            .rotate_left((index & 63) as u32);
        *value == mixed ^ 0xa5a5_5a5a_d3c3_b4b4
    })
}

pub(crate) fn spawn_stability_test(stop: Arc<AtomicBool>, tx: mpsc::Sender<StabilityEvent>) {
    spawn_stability_test_for(stop, tx, Duration::from_secs(STABILITY_TEST_SECS));
}

pub(crate) fn spawn_stability_test_for(
    stop: Arc<AtomicBool>,
    tx: mpsc::Sender<StabilityEvent>,
    duration: Duration,
) {
    std::thread::spawn(move || {
        let deadline = Instant::now() + duration;
        let errors = Arc::new(AtomicU64::new(0));
        let worker_count = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1);
        let mut workers = Vec::with_capacity(worker_count);

        for worker_id in 0..worker_count {
            let stop = stop.clone();
            let errors = errors.clone();
            workers.push(std::thread::spawn(move || {
                let mut seed = 0x6a09_e667_f3bc_c909_u64 ^ worker_id as u64;
                let mut memory = vec![0_u64; 1 << 17];
                while !stop.load(Ordering::Relaxed) && Instant::now() < deadline {
                    if !stability_memory_pass(seed, &mut memory) {
                        errors.fetch_add(1, Ordering::Relaxed);
                    }
                    for _ in 0..200_000 {
                        seed ^= seed << 13;
                        seed ^= seed >> 7;
                        seed ^= seed << 17;
                        seed = seed.wrapping_mul(0xd6e8_feb8_6659_fd93);
                    }
                    std::hint::black_box(seed);
                }
            }));
        }

        let mut cancelled = false;
        while Instant::now() < deadline {
            if stop.load(Ordering::Relaxed) {
                cancelled = true;
                break;
            }
            let remaining = deadline.saturating_duration_since(Instant::now()).as_secs();
            if tx.send(StabilityEvent::Progress(remaining)).is_err() {
                stop.store(true, Ordering::Relaxed);
                cancelled = true;
                break;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        stop.store(true, Ordering::Relaxed);
        for worker in workers {
            if worker.join().is_err() {
                errors.fetch_add(1, Ordering::Relaxed);
            }
        }
        let _ = tx.send(StabilityEvent::Finished {
            cancelled,
            errors: errors.load(Ordering::Relaxed),
        });
    });
}

pub(crate) fn build_cpu_stability_page(toast_overlay: &adw::ToastOverlay) -> gtk::Box {
    let page = page_lede("");
    let group = pref_group("Stability test", None);
    let status = adw::ActionRow::builder().title("Ready").build();
    group.add(&status);

    // One button, two roles — Start test ↔ Stop. A separate Stop button left
    // an odd half-empty actions row.
    let actions = adw::ActionRow::new();
    let run_button = gtk::Button::with_label("Start test");
    run_button.add_css_class("suggested-action");
    run_button.add_css_class("pill-btn");
    actions.add_suffix(&run_button);
    group.add(&actions);
    page.append(&group);

    let active_stop: Rc<RefCell<Option<Arc<AtomicBool>>>> = Rc::new(RefCell::new(None));
    let running = {
        let run_button = run_button.clone();
        let status = status.clone();
        Rc::new(move |on: bool| {
            if on {
                run_button.set_label("Stop");
                run_button.remove_css_class("suggested-action");
                run_button.add_css_class("destructive-action");
                status.set_title("Testing…");
                status.set_subtitle("5:00 remaining");
            } else {
                run_button.set_label("Start test");
                run_button.remove_css_class("destructive-action");
                run_button.add_css_class("suggested-action");
            }
        })
    };

    let stop_slot = active_stop.clone();
    let running_start = running.clone();
    let overlay = toast_overlay.clone();
    run_button.connect_clicked(move |_| {
        // While a test runs the same button requests the stop.
        if let Some(stop) = stop_slot.borrow().as_ref() {
            stop.store(true, Ordering::Relaxed);
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        *stop_slot.borrow_mut() = Some(stop.clone());
        running_start(true);

        let (tx, rx) = mpsc::channel();
        spawn_stability_test(stop, tx);

        let stop_slot = stop_slot.clone();
        let status = status.clone();
        let running = running_start.clone();
        let overlay = overlay.clone();
        glib::timeout_add_local(Duration::from_millis(250), move || {
            while let Ok(event) = rx.try_recv() {
                match event {
                    StabilityEvent::Progress(seconds) => {
                        status.set_subtitle(&format!(
                            "{}:{:02} remaining",
                            seconds / 60,
                            seconds % 60
                        ));
                    }
                    StabilityEvent::Finished { cancelled, errors } => {
                        *stop_slot.borrow_mut() = None;
                        running(false);
                        if cancelled {
                            status.set_title("Stopped");
                            status.set_subtitle("No result");
                        } else if errors == 0 {
                            status.set_title("Quick test passed");
                            status.set_subtitle("No errors found in this 5-minute run");
                            toast_ok(&overlay, "CPU stability test passed");
                        } else {
                            status.set_title("Errors detected");
                            status.set_subtitle("Reset the undervolt and test again");
                            toast_error(&overlay, "CPU stability errors detected");
                        }
                        return glib::ControlFlow::Break;
                    }
                }
            }
            glib::ControlFlow::Continue
        });
    });

    page
}

/// Build Custom-mode CPU PPT + GPU AC power sliders and attach them to `page`.
pub(crate) fn build_cpu_features(toast_overlay: &adw::ToastOverlay) -> adw::PreferencesGroup {
    let group = pref_group("CPU features", None);

    // Hyperthreading / SMT
    if legion_core::cpu::smt_available() {
        let active = legion_core::cpu::smt_active().unwrap_or(true);
        let smt = adw::SwitchRow::builder()
            .title("Hyperthreading (SMT)")
            .subtitle(legion_core::cpu::smt_summary())
            .active(active)
            .build();
        tip(&smt, "On = all logical CPUs. Off = one thread per core.");
        let overlay = toast_overlay.clone();
        let guard = Rc::new(Cell::new(false));
        let guard_n = guard.clone();
        smt.connect_active_notify(move |row| {
            if guard_n.get() {
                return;
            }
            let on = row.is_active();
            if !on {
                let row_r = row.clone();
                let overlay = overlay.clone();
                let guard = guard_n.clone();
                let n = legion_core::cpu::logical_cpus().max(2);
                let half = (n / 2).max(1);
                confirm_risk(
                    row,
                    "Turn off hyperthreading?",
                    &format!(
                        "Disabling SMT halves logical CPUs (about {n} → {half} on this laptop).\n\n\
                         A few games gain 1% lows; most workloads get slower. Changes apply immediately."
                    ),
                    "Disable SMT",
                    move |ok| {
                        if !ok {
                            guard.set(true);
                            row_r.set_active(true);
                            guard.set(false);
                            return;
                        }
                        apply_smt(&overlay, &row_r, false, &guard);
                    },
                );
                // Revert visually until confirmed — SwitchRow already flipped.
                guard_n.set(true);
                row.set_active(true);
                guard_n.set(false);
                return;
            }
            apply_smt(&overlay, row, true, &guard_n);
        });
        group.add(&smt);
    }

    if legion_core::cpu::boost_available() {
        let on = legion_core::cpu::boost_enabled().unwrap_or(true);
        let boost = adw::SwitchRow::builder()
            .title("CPU boost (turbo)")
            .subtitle(if on {
                "Frequency boost allowed"
            } else {
                "Locked to base clocks"
            })
            .active(on)
            .build();
        tip(
            &boost,
            "Off caps turbo for cooler/quieter running. On allows boost under load — runs hotter.",
        );
        let overlay = toast_overlay.clone();
        let guard = Rc::new(Cell::new(false));
        let guard_n = guard.clone();
        boost.connect_active_notify(move |row| {
            if guard_n.get() {
                return;
            }
            let want = row.is_active();
            // Enabling boost while on Max Power — extra nudge.
            if want && legion_core::profile::current() == "max-power" {
                let row_r = row.clone();
                let overlay = overlay.clone();
                let guard = guard_n.clone();
                confirm_risk(
                    row,
                    "Boost + Max Power",
                    "CPU boost with Max Power / Extreme pushes the highest clocks and heat. \
                     Only continue with strong cooling.",
                    "Enable",
                    move |ok| {
                        if !ok {
                            guard.set(true);
                            row_r.set_active(false);
                            guard.set(false);
                            return;
                        }
                        apply_boost(&overlay, &row_r, true, &guard);
                    },
                );
                guard_n.set(true);
                row.set_active(false);
                guard_n.set(false);
                return;
            }
            apply_boost(&overlay, row, want, &guard_n);
        });
        group.add(&boost);
    }

    group
}

pub(crate) fn apply_smt(
    overlay: &adw::ToastOverlay,
    row: &adw::SwitchRow,
    on: bool,
    guard: &Rc<Cell<bool>>,
) {
    let overlay = overlay.clone();
    let row = row.clone();
    let guard = guard.clone();
    let revert = {
        let row = row.clone();
        let guard = guard.clone();
        move |overlay: &adw::ToastOverlay, msg: &str| {
            guard.set(true);
            row.set_active(!on);
            guard.set(false);
            toast_error(overlay, msg);
        }
    };
    run_daemon_command_async(DaemonCommand::SetSmt(on), move |result| match result {
        Ok(DaemonResponse::Ok) => {
            let row_c = row.clone();
            let guard_c = guard.clone();
            run_daemon_command_async(DaemonCommand::GetSmt, move |r| {
                if let Ok(DaemonResponse::Smt {
                    active,
                    logical_cpus,
                    ..
                }) = r
                {
                    guard_c.set(true);
                    row_c.set_active(active);
                    row_c.set_subtitle(&format!(
                        "{} · {logical_cpus} logical CPUs",
                        if active { "On" } else { "Off" }
                    ));
                    guard_c.set(false);
                }
            });
            toast_ok(
                &overlay,
                if on {
                    "Hyperthreading on"
                } else {
                    "Hyperthreading off"
                },
            );
        }
        Ok(DaemonResponse::Error(e)) => revert(&overlay, &e),
        Err(e) if is_version_skew_error(&e) => {
            revert(
                &overlay,
                "Update the control service for SMT (reinstall daemon)",
            );
        }
        Err(e) => revert(&overlay, &e),
        _ => {}
    });
}

pub(crate) fn apply_boost(
    overlay: &adw::ToastOverlay,
    row: &adw::SwitchRow,
    on: bool,
    guard: &Rc<Cell<bool>>,
) {
    let overlay = overlay.clone();
    let row = row.clone();
    let guard = guard.clone();
    let revert = {
        let row = row.clone();
        let guard = guard.clone();
        move |overlay: &adw::ToastOverlay, msg: &str| {
            guard.set(true);
            row.set_active(!on);
            guard.set(false);
            toast_error(overlay, msg);
        }
    };
    run_daemon_command_async(DaemonCommand::SetBoost(on), move |result| match result {
        Ok(DaemonResponse::Ok) => {
            row.set_subtitle(if on {
                "Frequency boost allowed"
            } else {
                "Locked to base clocks"
            });
            toast_ok(&overlay, if on { "CPU boost on" } else { "CPU boost off" });
        }
        Ok(DaemonResponse::Error(e)) => revert(&overlay, &e),
        Err(e) if is_version_skew_error(&e) => {
            revert(
                &overlay,
                "Update the control service for boost (reinstall daemon)",
            );
        }
        Err(e) => revert(&overlay, &e),
        _ => {}
    });
}

// ─── Cooling ────────────────────────────────────────────────────────────────

pub(crate) const THERMAL_TJMAX_WARNING: &str = "\
96–98 °C is above the 9955HX3D TjMax (95 °C). Sustained use above TjMax can \
degrade the CPU or reduce its lifespan.

Only continue if you accept this risk.";
