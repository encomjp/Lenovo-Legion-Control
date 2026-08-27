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

    for (fan, rpm) in [(1u8, p.fan1), (2, p.fan2), (4, p.fan4)] {
        match send_command(DaemonCommand::SetFanTarget(fan, rpm)) {
            Ok(DaemonResponse::Error(e)) => errors.push(format!("fan {fan}: {e}")),
            Err(e) => errors.push(format!("fan {fan}: {e}")),
            _ => {}
        }
    }

    if let Err(e) = apply_charge_limit_blocking(p.charge_limit) {
        errors.push(format!("charge limit: {e}"));
    }

    legion_core::keyboard::set_rgb_brightness_async(p.brightness);
    legion_core::keyboard::set_logo_async(p.logo_on);
    legion_core::keyboard::restore_lighting_async();

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

    let group = pref_group("Named presets", None);

    let names = legion_core::config::list_profile_names();
    let name_refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    let labels: Vec<&str> = if name_refs.is_empty() {
        vec!["(none yet)"]
    } else {
        name_refs
    };
    let active = {
        let cur = legion_core::config::get().active_profile;
        names.iter().position(|n| n == &cur).unwrap_or(0) as u32
    };
    let picker = string_combo_row("Profile", "", &labels, active);
    tip(&picker, "Presets are stored in ~/.config/legion-control/");
    group.add(&picker);

    // Preview what the selected preset will change before Load applies it.
    {
        let update_summary = move |p: &adw::ComboRow| {
            let names = legion_core::config::list_profile_names();
            let text = names
                .get(p.selected() as usize)
                .and_then(|n| legion_core::config::get_named_profile(n))
                .map(|prof| profile_summary(&prof))
                .unwrap_or_else(|| {
                    "Nothing saved yet — snapshot your setup with Save current".into()
                });
            p.set_subtitle(&text);
        };
        update_summary(&picker);
        picker.connect_selected_notify(update_summary);
    }

    let entry = gtk::Entry::builder()
        .placeholder_text("Name for new profile")
        .hexpand(true)
        .build();
    tip(&entry, "Type a name, then Save current");
    let entry_row = adw::ActionRow::builder()
        .title("New name")
        .activatable(false)
        .build();
    tip(
        &entry_row,
        "Type a short name for this preset, then press Save current",
    );
    entry_row.add_suffix(&entry);
    group.add(&entry_row);

    let btns = gtk::Box::new(Orientation::Horizontal, 8);
    let save = primary_button_tip(
        "Save current",
        Some("Snapshot power mode, PPT, fans, lighting, and charge limit"),
    );
    let load = gtk::Button::with_label("Load");
    load.add_css_class("pill-btn");
    tip(&load, "Apply the selected named profile now");
    let del = gtk::Button::with_label("Delete");
    del.add_css_class("flat");
    del.add_css_class("pill-btn");
    tip(&del, "Remove the selected named profile from disk");
    // Buttons live in their own full-width row: a 3-button suffix inside an
    // ActionRow overflows below ~900 px window width (NN/g: responsive).
    btns.append(&save);
    btns.append(&load);
    btns.append(&del);
    btns.set_margin_top(4);
    btns.set_margin_bottom(4);
    btns.set_halign(Align::End);
    group.add(&btns);

    let restore_sw = adw::SwitchRow::builder()
        .title("Restore last session on launch")
        .active(legion_core::config::get().restore_on_launch)
        .build();
    tip(
        &restore_sw,
        "When on, Legion Control re-applies your last power mode, fans, charge limit, and lighting after launch",
    );
    restore_sw.connect_active_notify(|row| {
        legion_core::config::update(|cfg| cfg.restore_on_launch = row.is_active());
    });
    group.add(&restore_sw);

    gate.track(&group);
    page.append(&group);

    let overlay = toast_overlay.clone();
    let entry_s = entry.clone();
    let picker_s = picker.clone();
    save.connect_clicked(move |_| {
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
        for fan in [1u8, 2, 4] {
            legion_core::config::remember_fan(
                fan,
                legion_core::fans::read_target(fan).unwrap_or(0),
            );
        }
        legion_core::config::save_named_profile(&name);
        toast_ok(&overlay, &format!("Saved profile “{name}”"));
        let names = legion_core::config::list_profile_names();
        let model = gtk::StringList::new(&names.iter().map(|s| s.as_str()).collect::<Vec<_>>());
        picker_s.set_model(Some(&model));
        if let Some(i) = names.iter().position(|n| n == &name) {
            picker_s.set_selected(i as u32);
        }
        entry_s.set_text("");
    });

    let overlay = toast_overlay.clone();
    let picker_l = picker.clone();
    let mode_slot = mode_drop_slot.clone();
    let choices_slot = profile_choices_slot.clone();
    load.connect_clicked(move |_| {
        let names = legion_core::config::list_profile_names();
        let idx = picker_l.selected() as usize;
        let Some(name) = names.get(idx) else {
            toast_error(&overlay, "No profile selected");
            return;
        };
        let Some(p) = legion_core::config::get_named_profile(name) else {
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
    });

    let overlay = toast_overlay.clone();
    let picker_d = picker.clone();
    del.connect_clicked(move |_| {
        let names = legion_core::config::list_profile_names();
        let idx = picker_d.selected() as usize;
        let Some(name) = names.get(idx).cloned() else {
            return;
        };
        legion_core::config::delete_named_profile(&name);
        let names = legion_core::config::list_profile_names();
        let model = if names.is_empty() {
            gtk::StringList::new(&["(none yet)"])
        } else {
            gtk::StringList::new(&names.iter().map(|s| s.as_str()).collect::<Vec<_>>())
        };
        picker_d.set_model(Some(&model));
        picker_d.set_selected(0);
        toast_ok(&overlay, &format!("Deleted “{name}”"));
    });

    page
}
