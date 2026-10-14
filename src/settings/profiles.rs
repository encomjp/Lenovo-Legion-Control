//! Named presets — save, load, delete, and session restore.

use super::*;

/// IPC half of a profile apply — daemon writes, config, and keyboard calls.
/// Must run off the GTK main loop; returns the collected per-part errors.
pub(crate) fn apply_profile_blocking(
    p: &legion_core::config::UserProfile,
    apply_platform_mode: bool,
) -> Vec<String> {
    let mut errors = Vec::new();

    if apply_platform_mode {
        match send_command(DaemonCommand::SetProfile(p.platform_profile.clone())) {
            Ok(DaemonResponse::Error(e)) => errors.push(format!("profile: {e}")),
            Err(e) => errors.push(format!("profile: {e}")),
            _ => {}
        }
    }

    // PPT attributes are writable only while the firmware is in Custom mode.
    // Retain saved values for other modes, but do not send writes that the EC
    // will reject with EBUSY.
    if apply_platform_mode && p.platform_profile == "custom" {
        for (id, watts) in &p.ppt {
            match send_command(DaemonCommand::SetFwAttr {
                name: id.clone(),
                value: watts.to_string(),
            }) {
                Ok(DaemonResponse::Error(e)) => errors.push(format!("ppt {id}: {e}")),
                Err(e) => errors.push(format!("ppt {id}: {e}")),
                _ => {}
            }
        }
    }

    // `fan4` is the third-fan slot; resolve it to the real id (fan3 on
    // legion_hwmon) so the write lands on an existing channel.
    let [f1, f2, aux] = legion_core::fans::slot_ids();
    for (fan, rpm) in [(f1, p.fan1), (f2, p.fan2), (aux, p.fan4)] {
        match send_command(DaemonCommand::SetFanTarget(fan, rpm)) {
            Ok(DaemonResponse::Error(e)) => errors.push(format!("fan {fan}: {e}")),
            Err(e) => errors.push(format!("fan {fan}: {e}")),
            _ => {}
        }
    }

    if let Err(e) = apply_charge_limit_blocking(p.charge_limit) {
        errors.push(format!("charge limit: {e}"));
    }

    if legion_core::keyboard::spectrum_present() {
        legion_core::keyboard::set_rgb_brightness_async(p.brightness);
        legion_core::keyboard::set_logo_async(p.logo_on);
        legion_core::keyboard::restore_lighting_async();
    }

    errors
}

pub(crate) fn apply_profile(
    p: &legion_core::config::UserProfile,
    overlay: &adw::ToastOverlay,
    toast: bool,
    apply_platform_mode: bool,
) {
    legion_core::config::apply_profile_to_config(p);

    let ok_msg = format!("Restored · {}", friendly_profile(&p.platform_profile));
    let (sender, receiver) = mpsc::channel();
    let p = p.clone();
    std::thread::spawn(move || {
        let _ = sender.send(apply_profile_blocking(&p, apply_platform_mode));
    });

    let overlay = overlay.clone();
    glib::timeout_add_local(Duration::from_millis(150), move || {
        match receiver.try_recv() {
            Ok(errors) => {
                if toast {
                    if errors.is_empty() {
                        toast_ok(&overlay, &ok_msg);
                    } else {
                        overlay.add_toast(adw::Toast::new(&format!(
                            "{} error(s) applying profile",
                            errors.len()
                        )));
                    }
                }
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

// ─── Overview ───────────────────────────────────────────────────────────────

pub(crate) fn profile_summary(p: &legion_core::config::UserProfile) -> String {
    let fan = |v: u32| {
        if v == 0 {
            "auto".to_string()
        } else {
            v.to_string()
        }
    };
    format!(
        "{} · fans {}/{}/{} · brightness {} · limit {}% · rgb {}",
        friendly_profile(&p.platform_profile),
        fan(p.fan1),
        fan(p.fan2),
        fan(p.fan4),
        p.brightness,
        p.charge_limit,
        p.lighting_mode
    )
}

pub(crate) fn build_profiles_page(
    toast_overlay: &adw::ToastOverlay,
    gate: &DaemonGate,
    mode_drop_slot: &Rc<RefCell<Option<adw::ComboRow>>>,
    profile_choices_slot: &Rc<RefCell<Vec<String>>>,
) -> gtk::Box {
    let page = page_lede("");

    // ── Save the current setup (the one primary action on this view) ────
    let save_group = pref_group("Save current setup", None);
    let entry = gtk::Entry::builder()
        .placeholder_text("Profile name")
        .hexpand(true)
        .valign(Align::Center)
        .build();
    tip(&entry, "Type a name, then Save current");
    let entry_row = adw::ActionRow::builder()
        .title("Name")
        .subtitle("Snapshots power mode, limits, fans, lighting and charge limit")
        .activatable(false)
        .build();
    tip(
        &entry_row,
        "Presets are stored in ~/.config/legion-control/",
    );
    entry_row.add_suffix(&entry);
    let save = primary_button_tip(
        "Save current",
        Some("Snapshot power mode, PPT, fans, lighting, and charge limit"),
    );
    save.set_valign(Align::Center);
    entry_row.add_suffix(&save);
    save_group.add(&entry_row);
    gate.track(&save_group);
    page.append(&save_group);

    // ── Saved profiles: one row each with Load / Delete ─────────────────
    let list_group = pref_group("Saved profiles", None);
    gate.track(&list_group);
    page.append(&list_group);
    let rows: Rc<RefCell<Vec<adw::ActionRow>>> = Rc::new(RefCell::new(Vec::new()));
    type Refresh = Rc<RefCell<Option<Rc<dyn Fn()>>>>;
    let refresh: Refresh = Rc::new(RefCell::new(None));

    let rebuild = {
        let list_group = list_group.clone();
        let rows = rows.clone();
        let refresh = refresh.clone();
        let overlay = toast_overlay.clone();
        let mode_slot = mode_drop_slot.clone();
        let choices_slot = profile_choices_slot.clone();
        Rc::new(move || {
            for r in rows.borrow_mut().drain(..) {
                list_group.remove(&r);
            }
            let names = legion_core::config::list_profile_names();
            let active = legion_core::config::get().active_profile;
            if names.is_empty() {
                let empty = adw::ActionRow::builder()
                    .title("Nothing saved yet")
                    .subtitle("Name your current setup above and press Save current")
                    .activatable(false)
                    .build();
                list_group.add(&empty);
                rows.borrow_mut().push(empty);
                return;
            }
            for name in names {
                let summary = legion_core::config::get_named_profile(&name)
                    .map(|p| profile_summary(&p))
                    .unwrap_or_else(|| "Unreadable profile".into());
                let short = legion_core::config::get_named_profile(&name)
                    .map(|p| {
                        format!(
                            "{} · charge limit {}%",
                            friendly_profile(&p.platform_profile),
                            p.charge_limit
                        )
                    })
                    .unwrap_or_else(|| "Unreadable profile".into());
                let row = adw::ActionRow::builder()
                    .title(glib::markup_escape_text(&name))
                    .subtitle(glib::markup_escape_text(&short))
                    .activatable(false)
                    .build();
                tip(&row, &summary);
                if name == active {
                    let badge = status_pill_tip("Active", "ok", Some("Last profile you loaded"));
                    badge.set_valign(Align::Center);
                    row.add_prefix(&badge);
                }
                let load = secondary_button_tip("Load", Some("Apply this profile now"));
                let del = destructive_button_tip("Delete", Some("Remove this profile from disk"));
                row.add_suffix(&load);
                row.add_suffix(&del);

                {
                    let name = name.clone();
                    let overlay = overlay.clone();
                    let mode_slot = mode_slot.clone();
                    let choices_slot = choices_slot.clone();
                    let refresh = refresh.clone();
                    load.connect_clicked(move |_| {
                        let Some(p) = legion_core::config::get_named_profile(&name) else {
                            toast_error(&overlay, "Profile missing");
                            return;
                        };
                        apply_profile(&p, &overlay, true, true);
                        legion_core::config::update(|cfg| cfg.active_profile = name.clone());
                        let choices = choices_slot.borrow();
                        if let Some(i) = choices.iter().position(|c| c == &p.platform_profile) {
                            if let Some(drop) = mode_slot.borrow().as_ref() {
                                drop.set_selected(i as u32);
                            }
                        }
                        let again = refresh.borrow().clone();
                        if let Some(f) = again {
                            f();
                        }
                    });
                }
                {
                    let name = name.clone();
                    let overlay = overlay.clone();
                    let refresh = refresh.clone();
                    del.connect_clicked(move |_| {
                        legion_core::config::delete_named_profile(&name);
                        toast_ok(&overlay, &format!("Deleted “{name}”"));
                        let again = refresh.borrow().clone();
                        if let Some(f) = again {
                            f();
                        }
                    });
                }
                list_group.add(&row);
                rows.borrow_mut().push(row);
            }
        })
    };
    {
        let r = rebuild.clone();
        *refresh.borrow_mut() = Some(Rc::new(move || r()));
    }
    rebuild();

    let overlay = toast_overlay.clone();
    let entry_s = entry.clone();
    let rebuild_s = rebuild.clone();
    let save_action = move || {
        let name = entry_s.text().to_string();
        let name = name.trim().to_string();
        if name.is_empty() {
            toast_error(&overlay, "Enter a profile name first");
            return;
        }
        legion_core::config::remember_platform_profile(&legion_core::profile::current());
        for lim in legion_core::profile::ppt_limits() {
            legion_core::config::remember_ppt(lim.id, lim.current);
        }
        for fan in legion_core::fans::slot_ids() {
            legion_core::config::remember_fan(
                fan,
                // Out-of-range firmware targets save as auto, not max.
                legion_core::fans::display_target(fan),
            );
        }
        legion_core::config::save_named_profile(&name);
        toast_ok(&overlay, &format!("Saved profile “{name}”"));
        entry_s.set_text("");
        rebuild_s();
    };
    let save_action = Rc::new(save_action);
    {
        let a = save_action.clone();
        save.connect_clicked(move |_| a());
    }
    {
        let a = save_action;
        entry.connect_activate(move |_| a());
    }

    page
}
