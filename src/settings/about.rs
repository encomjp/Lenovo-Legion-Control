//! About hub — setup, hardware, help: updates, components, widget, diagnostics.

use super::*;

pub(crate) const KDE_WIDGET_ID: &str = "com.github.encomjp.legioncontrol";

pub(crate) static KDE_WIDGET_PACKAGE: Dir<'static> =
    include_dir!("$CARGO_MANIFEST_DIR/kde-widget/package");

/// Shared GTK widgets for the Custom-mode PPT sliders group.
pub(crate) fn show_about_dialog(parent: &impl glib::object::IsA<gtk::Widget>) {
    let about = adw::AboutDialog::builder()
        .application_name("Legion Control")
        .application_icon("com.encomjp.legion-settings")
        .developer_name("europeanpepe (encomjp)")
        .version(env!("CARGO_PKG_VERSION"))
        .comments(
            "Unofficial tool for Lenovo Legion laptops.\n\n\
             Not affiliated with Lenovo.",
        )
        .website("https://github.com/encomjp/")
        .issue_url("https://github.com/encomjp/Lenovo-Legion-Control/issues/new")
        .license_type(gtk::License::Gpl20Only)
        .developers(["europeanpepe (encomjp)"])
        .copyright("© europeanpepe / encomjp")
        .build();
    about.add_link("Author on GitHub", "https://github.com/encomjp/");
    about.add_link(
        "Donate (PayPal)",
        "https://www.paypal.com/donate/?hosted_button_id=H4SCC24R8KS4A",
    );
    about.add_link(
        "Report an issue",
        "https://github.com/encomjp/Lenovo-Legion-Control/issues/new",
    );
    about.add_link(
        "Spectrum protocol notes",
        "https://github.com/alstergee/legion-spectrum-control",
    );
    about.present(Some(parent));
}

pub(crate) fn kde_widget_installed() -> bool {
    std::process::Command::new("kpackagetool6")
        .args(["--type", "Plasma/Applet", "-l"])
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).contains(KDE_WIDGET_ID))
        .unwrap_or(false)
}

pub(crate) fn extract_kde_widget() -> Result<std::path::PathBuf, String> {
    let destination =
        std::env::temp_dir().join(format!("legion-control-widget-{}", std::process::id()));
    if destination.exists() {
        std::fs::remove_dir_all(&destination)
            .map_err(|error| format!("Cannot clear widget staging directory: {error}"))?;
    }
    KDE_WIDGET_PACKAGE
        .extract(&destination)
        .map_err(|error| format!("Cannot extract bundled widget: {error}"))?;
    Ok(destination)
}

pub(crate) fn install_kde_widget() -> Result<(), String> {
    let package = extract_kde_widget()?;
    let run = |operation: &str| {
        std::process::Command::new("kpackagetool6")
            .args(["--type", "Plasma/Applet", operation])
            .arg(&package)
            .output()
    };

    let first = run("-i")
        .map_err(|error| format!("kpackagetool6 is required (install KDE Plasma 6): {error}"))?;
    if first.status.success() {
        return Ok(());
    }

    let update = run("-u").map_err(|error| format!("Cannot update KDE widget: {error}"))?;
    if update.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&update.stderr);
        Err(format!("KDE widget installation failed: {}", stderr.trim()))
    }
}

pub(crate) fn remove_kde_widget() -> Result<(), String> {
    let output = std::process::Command::new("kpackagetool6")
        .args(["--type", "Plasma/Applet", "-r", KDE_WIDGET_ID])
        .output()
        .map_err(|error| format!("Cannot run kpackagetool6: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

pub(crate) fn build_updates_section(toast_overlay: &adw::ToastOverlay) -> adw::PreferencesGroup {
    let group = pref_group("Updates &amp; Releases", None);
    let row = adw::ActionRow::builder()
        .title("Version &amp; Updates")
        .subtitle(format!(
            "Installed: v{} · Checking…",
            legion_core::update::CURRENT_VERSION
        ))
        .activatable(false)
        .build();
    row.add_css_class("updates-row");

    let actions = gtk::Box::new(Orientation::Horizontal, 8);
    actions.set_valign(Align::Center);
    actions.set_homogeneous(true);
    let check_btn = gtk::Button::builder()
        .label("Check for updates")
        .tooltip_text("Look for a newer Legion Control release")
        .valign(Align::Center)
        .build();
    check_btn.add_css_class("pill-btn");
    check_btn.set_size_request(156, -1);
    check_btn.set_halign(Align::Fill);
    check_btn.set_hexpand(true);
    let update_btn = primary_button_tip(
        "Update now",
        Some("Download and install the matching release without opening a browser"),
    );
    update_btn.set_size_request(156, -1);
    update_btn.set_halign(Align::Fill);
    update_btn.set_hexpand(true);
    update_btn.set_sensitive(false);
    update_btn.remove_css_class("suggested-action");

    actions.append(&check_btn);
    actions.append(&update_btn);
    row.add_suffix(&actions);
    group.add(&row);

    let overlay = toast_overlay.clone();
    let row_c = row.clone();
    let check_btn_c = check_btn.clone();
    let update_btn_c = update_btn.clone();
    let latest: Rc<RefCell<Option<legion_core::update::ReleaseInfo>>> = Rc::new(RefCell::new(None));
    let latest_apply = latest.clone();

    update_btn.connect_clicked(move |_| {
        if let Some(info) = latest_apply.borrow().clone() {
            prompt_update_dialog(&info);
        }
    });

    let run_check = {
        let row = row_c.clone();
        let check_btn = check_btn_c.clone();
        let update_btn = update_btn_c.clone();
        let overlay = overlay.clone();
        let latest = latest.clone();
        Rc::new(move |interactive: bool| {
            check_btn.set_sensitive(false);
            check_btn.set_label("Checking…");
            let row = row.clone();
            let check_btn = check_btn.clone();
            let update_btn = update_btn.clone();
            let overlay = overlay.clone();
            let latest = latest.clone();
            dispatch_async(
                legion_core::update::check_latest_release,
                "Update check thread stopped",
                move |result| {
                    check_btn.set_sensitive(true);
                    check_btn.set_label("Check for updates");
                    match result {
                        Ok(info) => {
                            *latest.borrow_mut() = Some(info.clone());
                            if info.is_newer {
                                let can_apply = legion_core::update::can_apply(&info);
                                update_btn.set_sensitive(can_apply);
                                if can_apply {
                                    update_btn.add_css_class("suggested-action");
                                } else {
                                    update_btn.remove_css_class("suggested-action");
                                }
                                row.set_subtitle(&format!(
                                    "New version available: v{} (installed: v{})",
                                    info.version,
                                    legion_core::update::CURRENT_VERSION
                                ));
                                if !can_apply {
                                    let hint = legion_core::update::manual_update_hint();
                                    update_btn.set_tooltip_text(Some(&hint));
                                }
                                if interactive {
                                    prompt_update_dialog(&info);
                                }
                            } else {
                                update_btn.set_sensitive(false);
                                update_btn.remove_css_class("suggested-action");
                                row.set_subtitle(&format!(
                                    "Up to date (v{} is the latest release)",
                                    legion_core::update::CURRENT_VERSION
                                ));
                                if interactive {
                                    toast_ok(&overlay, "Legion Control is up to date");
                                }
                            }
                        }
                        Err(e) => {
                            update_btn.set_sensitive(false);
                            row.set_subtitle(&format!(
                                "Installed: v{} · Update check failed: {e}",
                                legion_core::update::CURRENT_VERSION
                            ));
                            if interactive {
                                toast_error(&overlay, &format!("Update check failed: {e}"));
                            }
                        }
                    }
                },
            );
        })
    };

    let check_closure = run_check.clone();
    check_btn.connect_clicked(move |_| {
        check_closure(true);
    });

    // Check automatically in the background on About page load
    let auto_closure = run_check;
    glib::timeout_add_local_once(Duration::from_millis(500), move || {
        auto_closure(false);
    });

    group
}

fn active_settings_window() -> Option<gtk::Window> {
    gtk::gio::Application::default()
        .and_then(|app| app.downcast::<gtk::Application>().ok())
        .and_then(|app| app.active_window())
}

pub(crate) fn prompt_update_dialog(info: &legion_core::update::ReleaseInfo) {
    let can_apply = legion_core::update::can_apply(info);
    let kind = legion_core::update::detect_install_kind();
    let headline = legion_core::update::changelog_headline(&info.body);
    let notes = legion_core::update::changelog_notes(&info.body);

    let win = adw::Window::builder()
        .title("Update available")
        .default_width(520)
        .default_height(380)
        .modal(true)
        .build();
    win.add_css_class("update-dialog");
    win.set_resizable(false);
    if let Some(parent) = active_settings_window() {
        win.set_transient_for(Some(&parent));
    }

    let header = adw::HeaderBar::new();
    header.add_css_class("flat");

    let stack = adw::ViewStack::new();
    stack.set_vexpand(true);

    let release = gtk::Box::new(Orientation::Vertical, 12);
    release.set_margin_top(16);
    release.set_margin_bottom(8);
    release.set_margin_start(24);
    release.set_margin_end(24);
    release.set_halign(Align::Fill);
    release.set_valign(Align::Center);
    release.set_vexpand(true);

    let version = gtk::Label::new(Some(&format!(
        "v{}  →  v{}",
        legion_core::update::CURRENT_VERSION,
        info.version
    )));
    version.add_css_class("update-version");
    version.set_wrap(true);
    version.set_justify(gtk::Justification::Center);
    release.append(&version);

    let blurb = gtk::Label::new(Some(&headline));
    blurb.add_css_class("update-blurb");
    blurb.set_wrap(true);
    blurb.set_justify(gtk::Justification::Center);
    blurb.set_max_width_chars(46);
    release.append(&blurb);

    let how = gtk::Label::new(Some(if can_apply {
        kind.apply_blurb()
    } else {
        "Open the GitHub release to download this version."
    }));
    how.add_css_class("dim-label");
    how.set_wrap(true);
    how.set_justify(gtk::Justification::Center);
    how.set_max_width_chars(46);
    release.append(&how);

    // Breeze (KDE) often lacks GNOME-only symbolic names; missing icons
    // render as the red broken-image tile.
    stack.add_titled_with_icon(
        &release,
        Some("release"),
        "Release",
        "view-refresh-symbolic",
    );

    let notes_text = if notes.is_empty() {
        "No release notes for this version.".to_string()
    } else {
        notes
    };
    let notes_label = gtk::Label::new(Some(&notes_text));
    notes_label.add_css_class("update-notes");
    notes_label.set_wrap(true);
    notes_label.set_xalign(0.0);
    notes_label.set_yalign(0.0);
    notes_label.set_selectable(true);
    let notes_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .propagate_natural_height(false)
        .child(&notes_label)
        .build();
    notes_scroll.set_vexpand(true);
    notes_scroll.set_hexpand(true);
    notes_scroll.set_margin_top(8);
    notes_scroll.set_margin_bottom(4);
    notes_scroll.set_margin_start(16);
    notes_scroll.set_margin_end(16);
    stack.add_titled_with_icon(
        &notes_scroll,
        Some("notes"),
        "What's new",
        "help-about-symbolic",
    );

    let switcher = adw::ViewSwitcher::new();
    switcher.set_stack(Some(&stack));
    switcher.set_policy(adw::ViewSwitcherPolicy::Wide);
    header.set_title_widget(Some(&switcher));

    let later = gtk::Button::with_label("Later");
    later.add_css_class("pill-btn");
    later.set_hexpand(true);
    // Source installs can do a full cargo build (2-4 min) *or* a quick AppImage swap
    let is_source_with_appimage =
        kind == legion_core::update::InstallKind::Source && info.appimage.is_some();
    let (action, appimage_btn) = if is_source_with_appimage {
        let a = primary_button_tip("Build from source", Some("Rebuilds locally — 2-4 min"));
        let b = gtk::Button::with_label("Use AppImage");
        b.set_tooltip_text(Some("Download AppImage (10s) instead of building"));
        b.add_css_class("pill-btn");
        (a, Some(b))
    } else if can_apply {
        (
            primary_button_tip("Update now", Some(kind.apply_blurb())),
            None,
        )
    } else {
        (
            primary_button_tip("Open release", Some("Open the GitHub release page")),
            None,
        )
    };
    action.set_hexpand(true);
    action.set_halign(Align::Fill);

    let actions = gtk::Box::new(Orientation::Horizontal, 8);
    actions.set_margin_top(10);
    actions.set_margin_bottom(14);
    actions.set_margin_start(16);
    actions.set_margin_end(16);
    actions.set_homogeneous(true);
    actions.append(&later);
    if let Some(ref b) = appimage_btn {
        actions.append(b);
    }
    actions.append(&action);

    let page = gtk::Box::new(Orientation::Vertical, 0);
    page.append(&stack);
    page.append(&actions);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&page));
    win.set_content(Some(&toolbar));
    win.set_default_widget(Some(&action));

    let win_later = win.clone();
    later.connect_clicked(move |_| {
        win_later.close();
    });

    let win_action = win.clone();
    let info_c = info.clone();
    let info_appimage = info.clone();
    let is_source_with_appimage_c = is_source_with_appimage;
    action.connect_clicked(move |_| {
        win_action.close();
        if can_apply {
            let forced = if is_source_with_appimage_c {
                Some(legion_core::update::InstallKind::Source)
            } else {
                None
            };
            begin_in_app_update_with_kind(&info_c, forced);
        } else if let Err(e) = gtk::gio::AppInfo::launch_default_for_uri(
            &info_c.html_url,
            None::<&gtk::gio::AppLaunchContext>,
        ) {
            let err = adw::AlertDialog::new(
                Some("Could not open the release"),
                Some(&format!("{e}\n\n{}", info_c.html_url)),
            );
            err.add_response("ok", "OK");
            err.present(active_settings_window().as_ref());
        }
    });
    if let Some(btn) = appimage_btn {
        let win2 = win.clone();
        let info2 = info_appimage.clone();
        btn.connect_clicked(move |_| {
            win2.close();
            begin_in_app_update_with_kind(&info2, Some(legion_core::update::InstallKind::AppImage));
        });
    }

    win.present();
}

fn begin_in_app_update_with_kind(
    info: &legion_core::update::ReleaseInfo,
    forced_kind: Option<legion_core::update::InstallKind>,
) {
    let info = info.clone();
    let kind = forced_kind.unwrap_or_else(legion_core::update::detect_install_kind);
    let dialog = adw::AlertDialog::new(
        Some("Updating Legion Control"),
        Some(&format!("Updating to v{}…", info.version)),
    );
    let bar = gtk::ProgressBar::new();
    bar.set_show_text(true);
    bar.set_pulse_step(0.08);
    dialog.set_extra_child(Some(&bar));
    dialog.set_can_close(false);
    dialog.present(active_settings_window().as_ref());

    let (ptx, prx) = mpsc::channel();
    let info_w = info.clone();
    let finished = Rc::new(Cell::new(false));
    let finished_done = finished.clone();
    let forced_kind_c = forced_kind;
    dispatch_async(
        move || {
            if let Some(k) = forced_kind_c {
                legion_core::update::apply_update_for_kind(&info_w, k, |phase, bytes, total| {
                    let _ = ptx.send((phase, bytes, total));
                })
            } else {
                legion_core::update::apply_update(&info_w, |phase, bytes, total| {
                    let _ = ptx.send((phase, bytes, total));
                })
            }
        },
        "Update download stopped",
        {
            let dialog = dialog.clone();
            move |result| {
                finished_done.set(true);
                match result {
                    Ok(outcome) => {
                        dialog.set_can_close(true);
                        dialog.force_close();
                        prompt_restart_dialog(outcome, &info);
                    }
                    Err(e) => {
                        dialog.set_can_close(true);
                        dialog.force_close();
                        let err = adw::AlertDialog::new(Some("Update failed"), Some(&e));
                        err.add_response("ok", "OK");
                        err.set_default_response(Some("ok"));
                        err.present(active_settings_window().as_ref());
                    }
                }
            }
        },
    );

    // Keep current phase + cargo tail so Building can pulse with live log
    let current_phase: Rc<RefCell<Option<legion_core::update::UpdatePhase>>> =
        Rc::new(RefCell::new(None));
    let tail: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let current_phase_c = current_phase.clone();
    let tail_c = tail.clone();
    glib::timeout_add_local(Duration::from_millis(80), move || {
        if finished.get() {
            return glib::ControlFlow::Break;
        }
        let mut last = None;
        while let Ok(msg) = prx.try_recv() {
            if let legion_core::update::UpdatePhase::BuildingLog(ref s) = msg.0 {
                *tail_c.borrow_mut() = Some(s.clone());
            }
            last = Some(msg);
        }
        if let Some((phase, bytes, total)) = last {
            *current_phase_c.borrow_mut() = Some(phase.clone());
            match phase {
                legion_core::update::UpdatePhase::Downloading => {
                    dialog.set_heading(Some("Downloading…"));
                    if let Some(t) = total.filter(|t| *t > 0) {
                        bar.set_fraction((bytes as f64 / t as f64).clamp(0.0, 1.0));
                        bar.set_text(Some(&format!(
                            "{:.1} / {:.1} MB",
                            bytes as f64 / 1_000_000.0,
                            t as f64 / 1_000_000.0
                        )));
                    } else {
                        bar.pulse();
                    }
                }
                legion_core::update::UpdatePhase::Verifying => {
                    dialog.set_heading(Some("Verifying checksum…"));
                    bar.set_fraction(1.0);
                    bar.set_text(Some("sha256"));
                }
                legion_core::update::UpdatePhase::Building
                | legion_core::update::UpdatePhase::BuildingLog(_) => {
                    dialog.set_heading(Some("Building from source…"));
                    bar.pulse();
                    let display = phase
                        .building_tail()
                        .map(|s| s.to_string())
                        .or_else(|| tail_c.borrow().clone())
                        .unwrap_or_else(|| "cargo — 2-4 min on first build".into());
                    bar.set_text(Some(&display));
                }
                legion_core::update::UpdatePhase::Installing => {
                    dialog.set_heading(Some("Installing…"));
                    bar.set_fraction(1.0);
                    bar.set_text(Some(kind.label()));
                }
            }
        } else {
            let is_building = current_phase_c
                .borrow()
                .as_ref()
                .is_some_and(|p| p.is_building());
            if is_building {
                bar.pulse();
                if let Some(t) = tail_c.borrow().clone() {
                    bar.set_text(Some(&t));
                }
            }
        }
        glib::ControlFlow::Continue
    });
}

fn prompt_restart_dialog(
    outcome: legion_core::update::ApplyOutcome,
    info: &legion_core::update::ReleaseInfo,
) {
    let extra = if outcome.needs_daemon_restage {
        "\n\nAfter restart you will get one password prompt to refresh the background service."
    } else {
        ""
    };
    let dialog = adw::AlertDialog::new(
        Some("Update ready"),
        Some(&format!(
            "v{} is installed. Restart Legion Control to switch to it.{extra}",
            info.version
        )),
    );
    dialog.add_response("later", "Later");
    dialog.add_response("restart", "Restart now");
    dialog.set_response_appearance("restart", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("restart"));
    dialog.set_close_response("later");
    dialog.connect_response(None, move |_, response| {
        if response != "restart" {
            return;
        }
        match legion_core::update::spawn_relaunch(&outcome.relaunch, &[]) {
            Ok(()) => {
                if let Some(app) = gtk::gio::Application::default() {
                    app.quit();
                } else {
                    std::process::exit(0);
                }
            }
            Err(e) => {
                let err = adw::AlertDialog::new(
                    Some("Could not restart"),
                    Some(&format!(
                        "{e}\n\nQuit Legion Control from the tray or window and start it again."
                    )),
                );
                err.add_response("ok", "OK");
                err.present(active_settings_window().as_ref());
            }
        }
    });
    dialog.present(active_settings_window().as_ref());
}

pub(crate) fn build_kde_widget_section(toast_overlay: &adw::ToastOverlay) -> adw::PreferencesGroup {
    let installed = kde_widget_installed();
    let group = pref_group("KDE Plasma widget", None);
    let row = adw::ActionRow::builder()
        .title("Legion Control widget")
        .subtitle(if installed {
            "Installed"
        } else {
            "Not installed"
        })
        .activatable(false)
        .build();

    let actions = gtk::Box::new(Orientation::Horizontal, 8);
    actions.set_valign(Align::Center);
    let preview = gtk::Button::builder()
        .label("Preview")
        .tooltip_text("Open the widget in plasmawindowed")
        .sensitive(installed)
        .build();
    let remove = gtk::Button::builder()
        .label("Remove")
        .tooltip_text("Remove the per-user widget package")
        .sensitive(installed)
        .build();
    let install = primary_button_tip(
        if installed {
            "Update widget"
        } else {
            "Install widget"
        },
        Some("Installs the bundled widget with kpackagetool6 — no root password needed"),
    );
    actions.append(&preview);
    actions.append(&remove);
    actions.append(&install);
    row.add_suffix(&actions);
    group.add(&row);

    let overlay = toast_overlay.clone();
    let row_c = row.clone();
    let install_c = install.clone();
    let remove_c = remove.clone();
    let preview_c = preview.clone();
    install.connect_clicked(move |_| match install_kde_widget() {
        Ok(()) => {
            row_c.set_subtitle("Installed");
            install_c.set_label("Update widget");
            remove_c.set_sensitive(true);
            preview_c.set_sensitive(true);
            toast_ok(&overlay, "KDE widget installed");
        }
        Err(error) => toast_error(&overlay, &error),
    });

    let overlay = toast_overlay.clone();
    let row_c = row.clone();
    let install_c = install.clone();
    let remove_c = remove.clone();
    let preview_c = preview.clone();
    remove.connect_clicked(move |_| match remove_kde_widget() {
        Ok(()) => {
            row_c.set_subtitle("Not installed");
            install_c.set_label("Install widget");
            remove_c.set_sensitive(false);
            preview_c.set_sensitive(false);
            toast_ok(&overlay, "KDE widget removed");
        }
        Err(error) => toast_error(&overlay, &error),
    });

    let overlay = toast_overlay.clone();
    preview.connect_clicked(move |_| {
        match std::process::Command::new("plasmawindowed")
            .arg(KDE_WIDGET_ID)
            .spawn()
        {
            Ok(_) => toast_ok(&overlay, "Widget preview opened"),
            Err(error) => toast_error(&overlay, &format!("Cannot open preview: {error}")),
        }
    });

    group
}

/// Run-once result handler for the opt-out nudge dialog (wrapped in an
/// `Option` so the `Fn` connect callback can take it on the first response).
pub(crate) type DisableNudgeCallback =
    std::rc::Rc<std::cell::RefCell<Option<Box<dyn FnOnce(bool)>>>>;

/// Nudge shown whenever the user tries to opt out of telemetry. Stresses
/// that the anonymised data is what enables support for more laptop models;
/// the opt-out is applied only after explicit confirmation. `on_result(true)`
/// means "disable anyway"; `false` means "keep telemetry on".
pub(crate) fn confirm_disable_telemetry(
    win: Option<&gtk::Window>,
    on_result: impl FnOnce(bool) + 'static,
) {
    let dialog = adw::AlertDialog::new(
        Some("Keep telemetry on?"),
        Some(
            "Your data helps add support for more laptop models.\n\n\
             Legion Control has been tested on only ONE laptop. The anonymous \
             diagnostics your laptop sends are what help other people with the \
             same model get support.\n\n\
             Disabling telemetry means we're unable to provide support for \
             people with your model of laptop.",
        ),
    );
    dialog.add_response("keep", "Keep telemetry on");
    dialog.add_response("disable", "Disable anyway");
    dialog.set_default_response(Some("keep"));
    dialog.set_close_response("keep");
    // connect_response needs an `Fn` callback, but we only want to run the
    // result handler once — take it out of an Option on the first call.
    let callback: DisableNudgeCallback =
        std::rc::Rc::new(std::cell::RefCell::new(Some(Box::new(on_result))));
    dialog.connect_response(None, move |_, r| {
        if let Some(cb) = callback.borrow_mut().take() {
            cb(r == "disable");
        }
    });
    dialog.present(win);
}

/// Alpha diagnostics (opt-out) — privacy disclosure, telemetry switch,
/// self-check runner, and on-demand send. All of it works without the
/// daemon running.
///
/// Returns live handles alongside the group: a `Cell<bool>` mirroring the
/// telemetry switch (shared with the welcome dialog so "Opt out" can flip it)
/// and the switch itself.
pub(crate) fn build_diagnostics_section(
    toast_overlay: &adw::ToastOverlay,
    sync: &Rc<Cell<bool>>,
) -> (adw::PreferencesGroup, Rc<Cell<bool>>, adw::SwitchRow) {
    // No long disclosure paragraph here — the switch subtitle and hover tips
    // carry what matters, and the boxed list holds only actionable rows.
    let group = pref_group("Alpha diagnostics (anonymous)", None);

    // Live consent mirror — updated by the switch handler below, read by the
    // Send-now gating, and handed to show_welcome_if_needed by the caller.
    let consent = Rc::new(Cell::new(legion_core::config::get().diagnostics.enabled));

    let share_row = adw::SwitchRow::builder()
        .title("Share anonymous diagnostics")
        .active(legion_core::config::get().diagnostics.enabled)
        .build();
    tip(
        &share_row,
        "Opt-out switch — telemetry is on by default · turn off to stop automatic sending · Send now requires this to be ON",
    );
    group.add(&share_row);

    // Send-now row — built before the consent handler below so the switch can
    // keep the button's sensitivity glued to the consent state (both ways).
    let send_row = adw::ActionRow::builder()
        .title("Send now")
        .activatable(false)
        .build();
    let send_btn = gtk::Button::with_label("Send now");
    send_btn.set_valign(Align::Center);
    send_btn.add_css_class("pill-btn");
    send_btn.set_size_request(110, -1);
    send_btn.set_halign(Align::Center);
    tip(
        &send_btn,
        "Collects and sends one anonymized report immediately",
    );
    send_row.add_suffix(&send_btn);
    // Initial state from the consent mirror: no consent → nothing may be sent.
    send_btn.set_sensitive(consent.get());

    {
        let overlay = toast_overlay.clone();
        let send_gate = send_btn.clone();
        let consent_gate = consent.clone();
        let sync_gate = sync.clone();
        share_row.connect_active_notify(move |row| {
            // Programmatic flips from the welcome window / guided setup are
            // already nudge-confirmed at their source — never re-ask here.
            if sync_gate.get() {
                return;
            }
            let enabled = row.is_active();
            if enabled {
                // Turning ON: apply immediately.
                consent_gate.set(true);
                send_gate.set_sensitive(true);
                legion_core::config::update(|c| c.diagnostics.enabled = true);
                toast_ok(&overlay, "Anonymous diagnostics enabled");
            } else {
                // Turning OFF: nudge before allowing the opt-out.
                let row_c = row.clone();
                let consent_c = consent_gate.clone();
                let send_c = send_gate.clone();
                let overlay_c = overlay.clone();
                let win = overlay_c
                    .root()
                    .and_then(|r| r.downcast::<gtk::Window>().ok());
                confirm_disable_telemetry(win.as_ref(), move |confirmed| {
                    if confirmed {
                        consent_c.set(false);
                        send_c.set_sensitive(false);
                        legion_core::config::update(|c| c.diagnostics.enabled = false);
                        toast_ok(&overlay_c, "Anonymous diagnostics disabled");
                    } else {
                        // Revert the switch to ON — telemetry stays enabled.
                        row_c.set_active(true);
                    }
                });
            }
        });
    }

    let check_row = adw::ActionRow::builder()
        .title("Self-check")
        .activatable(false)
        .build();
    let run_btn = gtk::Button::with_label("Run");
    run_btn.set_valign(Align::Center);
    run_btn.add_css_class("pill-btn");
    run_btn.set_size_request(110, -1);
    run_btn.set_halign(Align::Center);
    tip(
        &run_btn,
        "Runs read-only local checks — nothing is written or sent",
    );
    check_row.add_suffix(&run_btn);
    group.add(&check_row);

    // Results land here once the first run completes; hidden until then.
    let results = gtk::Label::new(None);
    results.add_css_class("detail-body");
    results.set_halign(Align::Start);
    results.set_wrap(true);
    results.set_xalign(0.0);
    results.set_selectable(true);
    results.set_margin_start(12);
    results.set_margin_end(12);
    results.set_margin_top(4);
    results.set_margin_bottom(8);
    tip(
        &results,
        "One line per check — selectable so you can copy it into a bug report",
    );
    let expander = gtk::Expander::builder()
        .label("Self-check results")
        .child(&results)
        .visible(false)
        .build();
    group.add(&expander);

    {
        let overlay = toast_overlay.clone();
        let run_btn_connect = run_btn.clone();
        let run_btn_closure = run_btn.clone();
        let expander_closure = expander.clone();
        let results_closure = results.clone();
        run_btn_connect.connect_clicked(move |_| {
            run_btn_closure.set_sensitive(false);
            run_btn_closure.set_label("Running…");
            let overlay = overlay.clone();
            let run_btn = run_btn_closure.clone();
            let expander = expander_closure.clone();
            let results = results_closure.clone();
            dispatch_async(
                move || {
                    Ok::<Vec<legion_core::selftest::SelfCheck>, String>(
                        legion_core::selftest::run_self_checks(),
                    )
                },
                "Self-check stopped without a result",
                move |result| {
                    run_btn.set_sensitive(true);
                    run_btn.set_label("Run");
                    match result {
                        Ok(checks) => {
                            let total = checks.len();
                            let passed = checks.iter().filter(|c| c.ok).count();
                            let lines: Vec<String> = checks
                                .iter()
                                .map(|c| {
                                    format!(
                                        "{} {} — {}",
                                        if c.ok { "✓" } else { "✗" },
                                        c.name,
                                        c.detail
                                    )
                                })
                                .collect();
                            results.set_text(&lines.join("\n"));
                            expander.set_label(Some(&format!(
                                "Self-check results ({passed}/{total} passed)"
                            )));
                            expander.set_expanded(true);
                            expander.set_visible(true);
                        }
                        Err(error) => toast_error(&overlay, &error),
                    }
                },
            );
        });
    }

    // Appended here so the visual order stays: disclosure → consent →
    // self-check → results → send (the row itself is built above).
    group.add(&send_row);

    {
        let overlay = toast_overlay.clone();
        let send_btn_connect = send_btn.clone();
        let send_btn_closure = send_btn.clone();
        let consent_closure = consent.clone();
        send_btn_connect.connect_clicked(move |_| {
            send_btn_closure.set_sensitive(false);
            send_btn_closure.set_label("Sending…");
            let overlay = overlay.clone();
            let btn_inner = send_btn_closure.clone();
            let consent_done = consent_closure.clone();
            dispatch_async(
                move || legion_core::diagnostics::collect_and_send(None),
                "Diagnostics send stopped without a result",
                move |result| {
                    // Re-enable only if consent is STILL on — it may have
                    // been toggled off while this send was in flight.
                    btn_inner.set_sensitive(consent_done.get());
                    btn_inner.set_label("Send now");
                    match result {
                        Ok(_) => toast_info(&overlay, "Diagnostics sent — thank you!"),
                        Err(error) => toast_error(&overlay, &error),
                    }
                },
            );
        });
    }

    (group, consent, share_row)
}

pub(crate) fn build_components_section(toast_overlay: &adw::ToastOverlay) -> adw::PreferencesGroup {
    let group = pref_group("First-time setup", None);

    let daemon_active = std::path::Path::new(legion_core::comms::SYSTEM_SOCKET).exists();
    let current_ver = legion_core::update::CURRENT_VERSION;
    let daemon_ver_opt = legion_core::comms::query_daemon_version().ok();
    let (subtitle, is_mismatch, is_up_to_date) = if !daemon_active {
        ("Inactive".to_string(), false, false)
    } else {
        match &daemon_ver_opt {
            Some(v) if v == current_ver => (format!("Active (v{v})"), false, true),
            Some(v) => (
                format!("Active (v{v} — outdated; restart to update to v{current_ver})"),
                true,
                false,
            ),
            None => (
                format!("Active (legacy pre-v0.2.11 — restart to update to v{current_ver})"),
                true,
                false,
            ),
        }
    };

    let daemon_row = adw::ActionRow::builder()
        .title("Hardware control daemon")
        .subtitle(&subtitle)
        .activatable(false)
        .build();

    let daemon_suffix = gtk::Box::new(Orientation::Horizontal, 8);
    daemon_suffix.set_valign(Align::Center);
    let daemon_button = primary_button_tip(
        if is_mismatch { "Restart daemon" } else { "Enable" },
        Some("Uses a narrowly scoped PolicyKit helper; no shell command is accepted"),
    );
    let daemon_pill = status_pill_tip("Enabled", "ok", Some("legion-control.service is active and up to date"));
    if is_up_to_date {
        daemon_suffix.append(&daemon_pill);
    } else {
        daemon_suffix.append(&daemon_button);
    }
    daemon_row.add_suffix(&daemon_suffix);
    group.add(&daemon_row);

    let overlay = toast_overlay.clone();
    let row = daemon_row.clone();
    let suffix = daemon_suffix.clone();
    let button = daemon_button.clone();
    let pill = daemon_pill.clone();
    daemon_button.connect_clicked(move |_| {
        button.set_sensitive(false);
        button.set_label("Updating…");
        let overlay = overlay.clone();
        let row = row.clone();
        let suffix = suffix.clone();
        let button = button.clone();
        let pill = pill.clone();
        run_setup_helper("enable-daemon", move |result| match result {
            Ok(_) => {
                row.set_subtitle(&format!("Active (v{})", legion_core::update::CURRENT_VERSION));
                suffix.remove(&button);
                pill.set_text("Enabled");
                suffix.append(&pill);
                toast_ok(&overlay, "Hardware daemon updated and restarted");
            }
            Err(error) => {
                button.set_label("Retry");
                button.set_sensitive(true);
                toast_error(&overlay, &error);
            }
        });
    });

    let smu_installed = std::path::Path::new("/sys/kernel/ryzen_smu_drv").is_dir();
    let smu_row = adw::ActionRow::builder()
        .title("AMD tuning backend")
        .subtitle("Checking backend…")
        .activatable(false)
        .build();
    {
        let smu_row_c = smu_row.clone();
        run_daemon_command_async(DaemonCommand::GetCurveOptimizer, move |result| {
            let smu_status = match result {
                Ok(DaemonResponse::CurveOptimizer(status)) if status.available => {
                    "Installed".to_string()
                }
                Ok(DaemonResponse::CurveOptimizer(status)) => status.reason,
                _ if smu_installed => "Driver loaded".into(),
                _ => "Optional".into(),
            };
            smu_row_c.set_subtitle(&smu_status);
        });
    }
    let smu_actions = gtk::Box::new(Orientation::Horizontal, 8);
    smu_actions.set_valign(Align::Center);
    let remove_smu = gtk::Button::builder()
        .label("Remove")
        .sensitive(smu_installed)
        .tooltip_text("Unload and remove the optional ryzen_smu DKMS driver")
        .build();
    let install_smu = primary_button_tip(
        "Install",
        Some("Builds the bundled, pinned ryzen_smu source through DKMS using PolicyKit"),
    );
    let smu_pill = status_pill_tip(
        "Installed",
        "ok",
        Some("ryzen_smu driver is loaded — Curve Optimizer is available on the CPU page"),
    );
    smu_actions.append(&remove_smu);
    if smu_installed {
        smu_actions.append(&smu_pill);
    } else {
        smu_actions.append(&install_smu);
    }
    smu_row.add_suffix(&smu_actions);
    group.add(&smu_row);

    let overlay = toast_overlay.clone();
    let row = smu_row.clone();
    let install = install_smu.clone();
    let remove = remove_smu.clone();
    let actions = smu_actions.clone();
    let pill = smu_pill.clone();
    install_smu.connect_clicked(move |_| {
        install.set_sensitive(false);
        install.set_label("Installing…");
        let overlay = overlay.clone();
        let row = row.clone();
        let install = install.clone();
        let remove = remove.clone();
        let actions = actions.clone();
        let pill = pill.clone();
        run_setup_helper("install-ryzen-smu", move |result| match result {
            Ok(_) => {
                row.set_subtitle("Installed");
                actions.remove(&install);
                actions.append(&pill);
                remove.set_sensitive(true);
                toast_ok(
                    &overlay,
                    "AMD tuning backend installed — open CPU to review it",
                );
            }
            Err(error) => {
                install.set_label("Install");
                install.set_sensitive(true);
                toast_error(&overlay, &error);
            }
        });
    });

    let overlay = toast_overlay.clone();
    let row = smu_row.clone();
    let install = install_smu.clone();
    let remove = remove_smu.clone();
    remove_smu.connect_clicked(move |_| {
        remove.set_sensitive(false);
        let overlay = overlay.clone();
        let row = row.clone();
        let install = install.clone();
        let remove = remove.clone();
        run_setup_helper("remove-ryzen-smu", move |result| match result {
            Ok(_) => {
                row.set_subtitle("Optional");
                install.set_label("Install");
                install.set_sensitive(true);
                toast_ok(&overlay, "AMD tuning backend removed");
            }
            Err(error) => {
                remove.set_sensitive(true);
                toast_error(&overlay, &error);
            }
        });
    });

    group
}

pub(crate) fn build_about_pages(
    toast_overlay: &adw::ToastOverlay,
    sync: &Rc<Cell<bool>>,
) -> (
    gtk::Box,
    gtk::Box,
    gtk::Box,
    // Live diagnostics consent state + switch, threaded to
    // show_welcome_if_needed so the welcome window can flip them.
    Rc<Cell<bool>>,
    adw::SwitchRow,
) {
    let setup_page = page_lede("");
    let help_page = page_lede("");
    let hardware_page = page_lede("");
    let info = legion_core::device::detect();

    setup_page.append(&build_updates_section(toast_overlay));
    setup_page.append(&build_components_section(toast_overlay));
    setup_page.append(&build_kde_widget_section(toast_overlay));
    let (diag_group, diag_consent, diag_share_switch) =
        build_diagnostics_section(toast_overlay, sync);
    setup_page.append(&diag_group);

    let help = pref_group("Help", None);
    let report_row = adw::ActionRow::builder()
        .title("Report an issue")
        .activatable(true)
        .build();
    tip(
        &report_row,
        "Opens https://github.com/encomjp/Lenovo-Legion-Control/issues/new — report bugs or request features",
    );
    report_row.connect_activated(|_| {
        open_uri("https://github.com/encomjp/Lenovo-Legion-Control/issues/new");
    });
    let report_open = flat_open_button("Opens GitHub in your browser");
    report_open.connect_clicked(|_| {
        open_uri("https://github.com/encomjp/Lenovo-Legion-Control/issues/new");
    });
    report_row.add_suffix(&report_open);
    help.add(&report_row);

    let donate_row = adw::ActionRow::builder()
        .title("Donate")
        .activatable(true)
        .build();
    donate_row.add_prefix(&color_icon(
        include_bytes!("../../data/icons/donate.svg"),
        24,
    ));
    tip(
        &donate_row,
        "Opens the PayPal donate page — optional support for continued development",
    );
    donate_row.connect_activated(|_| {
        open_uri("https://www.paypal.com/donate/?hosted_button_id=H4SCC24R8KS4A");
    });
    let donate_open = flat_open_button("Opens PayPal in your browser");
    donate_open.connect_clicked(|_| {
        open_uri("https://www.paypal.com/donate/?hosted_button_id=H4SCC24R8KS4A");
    });
    donate_row.add_suffix(&donate_open);
    help.add(&donate_row);

    help_page.append(&help);

    let legal = pref_group("Legal notice", None);
    legal.add(&property_row(
        "Not Lenovo",
        "Unofficial community tool",
        Some("Not affiliated with, endorsed by, or recommended by Lenovo. Use at your own risk."),
    ));
    legal.add(&property_row(
        "Author",
        "europeanpepe (encomjp)",
        Some("Credits and contact via GitHub"),
    ));
    legal.add(&property_row(
        "GitHub",
        "https://github.com/encomjp/",
        Some("Author profile and related projects"),
    ));
    help_page.append(&legal);

    let laptop = pref_group("This laptop", None);
    let gen_s = if info.gen > 0 {
        format!("Gen {}", info.gen)
    } else {
        "Unknown".into()
    };
    let match_s = if info.profile_matched {
        format!("Yes · {}", info.profile_source)
    } else {
        info.profile_source.clone()
    };
    let peak_s = match info.capabilities.peak_gpu_w {
        Some(w) => format!("{w} W ({})", info.capabilities.peak_gpu_source),
        None => info.capabilities.peak_gpu_source.clone(),
    };
    let fans_s = if info.capabilities.fans.is_empty() {
        "None detected".into()
    } else {
        info.capabilities
            .fans
            .iter()
            .map(|f| format!("{} {}–{} RPM", f.title, f.min_rpm, f.max_rpm))
            .collect::<Vec<_>>()
            .join(" · ")
    };
    let ppt_s = if info.capabilities.ppt_attrs.is_empty() {
        "None writable".into()
    } else {
        info.capabilities
            .ppt_attrs
            .iter()
            .map(|a| a.split(' ').next().unwrap_or(a.as_str()).to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    for (k, v, tip_text) in [
        (
            "Model",
            info.model.as_str(),
            "Marketing name from DMI (product_version / family) when available",
        ),
        (
            "Machine type",
            info.machine_type.as_str(),
            "Lenovo MT code (e.g. 83RU) — used for profile matching",
        ),
        (
            "Series",
            info.series.as_str(),
            "Matched series from the built-in Legion / LOQ model database",
        ),
        (
            "Generation",
            gen_s.as_str(),
            "Approximate Legion generation from the model database",
        ),
        (
            "BIOS",
            info.bios_version.as_str(),
            "UEFI version — first four letters are the LenovoLegionLinux BIOS family key",
        ),
        (
            "BIOS family",
            info.bios_prefix.as_str(),
            "BIOS prefix used by LenovoLegionLinux DMI allowlist (e.g. SMCN, Q7CN, GKCN)",
        ),
        (
            "Profile match",
            match_s.as_str(),
            "Whether this chassis matched a curated profile and where that entry came from",
        ),
        (
            "EC / fans",
            info.ec_chip.as_str(),
            "Embedded controller path used for fan sensors",
        ),
        (
            "Fan backend",
            info.capabilities.fan_backend.as_str(),
            "Kernel hwmon driver providing fan RPM / targets",
        ),
        (
            "Fan ranges",
            fans_s.as_str(),
            "Live min/max from hwmon (falls back to profile defaults)",
        ),
        (
            "Lighting",
            info.capabilities.lighting.as_str(),
            "USB HID lighting devices detected on this machine",
        ),
        (
            "Peak GPU TGP",
            peak_s.as_str(),
            "Maximum GPU board power — nvidia-smi when available, else PSREF heuristic",
        ),
        (
            "Custom PPT attrs",
            ppt_s.as_str(),
            "Writable firmware-attributes for Custom mode CPU/GPU power",
        ),
        (
            "Processor",
            info.cpu_model.as_str(),
            "CPU model from /proc/cpuinfo, mapped through data/cpu-ids.yaml",
        ),
        (
            "Graphics",
            info.gpu_model.as_str(),
            "Discrete GPU name: nvidia-smi when awake, else PCI ID map, else lspci",
        ),
    ] {
        laptop.add(&property_row(k, v, Some(tip_text)));
    }
    if !info.profile_notes.is_empty() {
        laptop.add(&property_row(
            "Notes",
            &info.profile_notes,
            Some("Quirks and capabilities notes from the matched model profile"),
        ));
    }
    hardware_page.append(&laptop);

    let lighting = pref_group("Lighting and profiles", None);
    let saved = legion_core::config::config_dir_display();
    lighting.add(&property_row(
        "Controller",
        &info.capabilities.lighting,
        Some("USB HID lighting devices probed at launch"),
    ));
    lighting.add(&property_row(
        "Saved in",
        &format!("{saved}/settings.json"),
        Some("Effects, colours, charge limit, fans, and named profiles — restored on launch when enabled"),
    ));
    lighting.add(&property_row(
        "Tray",
        "Close hides to tray · Quit from tray menu",
        Some("Left-click the tray icon to show the window again"),
    ));
    // Storage was a redundant duplicate of Hardware — merged here so the
    // About hub stays at Setup / Hardware / Help only.
    hardware_page.append(&lighting);
    (
        setup_page,
        help_page,
        hardware_page,
        diag_consent,
        diag_share_switch,
    )
}

pub(crate) fn build_speakers_section(toast_overlay: &adw::ToastOverlay) -> adw::PreferencesGroup {
    use legion_core::audio::{self, Health};

    let group = pref_group("Speakers", None);
    let diag0 = audio::diagnose();
    let (pill_text, pill_kind) = amp_pill(diag0.health);
    let pill = status_pill_tip(pill_text, pill_kind, Some(amp_pill_tooltip(diag0.health)));

    let status_row = adw::ActionRow::builder()
        .title(&diag0.summary)
        .subtitle(amp_short_help(diag0.health))
        .activatable(false)
        .build();
    tip(
        &status_row,
        "Plain-language summary — expand details for raw checks",
    );
    status_row.add_suffix(&pill);
    group.add(&status_row);

    let expander = adw::ExpanderRow::builder()
        .title("Technical details")
        .build();
    tip(
        &expander,
        "ACPI amp presence, kernel modules, firmware, mute, and PipeWire sink",
    );
    let details = gtk::Label::new(Some(&diag0.details.join("\n")));
    details.add_css_class("detail-body");
    details.set_halign(Align::Start);
    details.set_wrap(true);
    details.set_xalign(0.0);
    details.set_selectable(true);
    details.set_margin_start(12);
    details.set_margin_end(12);
    details.set_margin_top(4);
    details.set_margin_bottom(8);
    tip(
        &details,
        "Raw speaker diagnostic lines — selectable for copying into a bug report",
    );
    expander.add_row(&details);
    group.add(&expander);

    let btn = primary_button_tip(
        match diag0.health {
            Health::Ok => "Refresh and re-check",
            Health::SoftIssue => "Repair speakers",
            Health::HardwareBroken => "Try soft fix anyway",
            Health::NotApplicable => "Not applicable",
        },
        Some(amp_action_tooltip(diag0.health)),
    );
    // Gen10-only feature: grey out on hardware without AW88399 (all 83JG/83DG in fleet).
    // Fleet 83RU is the only AWDZ8399 host; kernel 7.3 ships the driver, so this
    // section will be removed entirely once 7.3 is stable.
    if diag0.health == Health::NotApplicable {
        btn.set_sensitive(false);
        btn.set_tooltip_text(Some(
            "No AW88399 smart-amp on this model — speaker fix is Gen10 Pro 7 only and will be removed once kernel 7.3 ships",
        ));
    }
    let action = adw::ActionRow::builder()
        .title("Repair")
        .subtitle(if diag0.health == Health::NotApplicable {
            "Not applicable on this hardware"
        } else {
            ""
        })
        .activatable(false)
        .build();
    tip(&action, amp_action_tooltip(diag0.health));
    action.add_suffix(&btn);
    action.set_sensitive(diag0.health != Health::NotApplicable);
    group.add(&action);

    let overlay = toast_overlay.clone();
    let pill_c = pill.clone();
    let status_c = status_row.clone();
    let details_c2 = details.clone();
    let expander_c = expander.clone();
    let btn_c = btn.clone();
    btn.connect_clicked(move |_| {
        set_busy(&btn_c, true, "Repair speakers");

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(audio::troubleshoot());
        });

        let overlay = overlay.clone();
        let pill_c = pill_c.clone();
        let status_c = status_c.clone();
        let details_c2 = details_c2.clone();
        let expander_c = expander_c.clone();
        let btn_c = btn_c.clone();
        glib::timeout_add_local(Duration::from_millis(200), move || match rx.try_recv() {
            Ok(report) => {
                let (pt, pk) = amp_pill(report.after.health);
                set_pill(&pill_c, pt, pk);
                tip(&pill_c, amp_pill_tooltip(report.after.health));
                status_c.set_title(&report.after.summary);
                status_c.set_subtitle(amp_short_help(report.after.health));
                let mut body = report.after.details.clone();
                if !report.steps.is_empty() {
                    body.push(String::new());
                    body.push("What we did:".into());
                    body.extend(report.steps.iter().map(|s| format!("· {s}")));
                }
                if !report.errors.is_empty() {
                    body.push(String::new());
                    body.push("Problems:".into());
                    body.extend(report.errors.iter().map(|s| format!("· {s}")));
                }
                details_c2.set_text(&body.join("\n"));
                expander_c.set_expanded(true);

                match report.after.health {
                    Health::Ok => toast_ok(&overlay, "Speakers look good"),
                    Health::SoftIssue => {
                        toast_error(&overlay, "Still needs attention — check sound settings")
                    }
                    Health::HardwareBroken => toast_error(
                        &overlay,
                        "Amp driver still missing — soft fix can’t finish this",
                    ),
                    Health::NotApplicable => toast_ok(&overlay, &report.after.summary),
                }

                let idle = match report.after.health {
                    Health::Ok => "Refresh and re-check",
                    Health::SoftIssue => "Repair speakers",
                    Health::HardwareBroken => "Try soft fix anyway",
                    Health::NotApplicable => "Check speakers",
                };
                set_busy(&btn_c, false, idle);
                tip(&btn_c, amp_action_tooltip(report.after.health));
                tip(&pill_c, amp_pill_tooltip(report.after.health));
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => {
                set_busy(&btn_c, false, "Repair speakers");
                toast_error(&overlay, "Something went wrong");
                glib::ControlFlow::Break
            }
        });
    });

    group
}

pub(crate) fn amp_pill(health: legion_core::audio::Health) -> (&'static str, &'static str) {
    use legion_core::audio::Health;
    match health {
        Health::Ok => ("OK", "ok"),
        Health::SoftIssue => ("Needs fix", "warn"),
        Health::HardwareBroken => ("Broken", "bad"),
        Health::NotApplicable => ("N/A", "muted"),
    }
}

pub(crate) fn amp_pill_tooltip(health: legion_core::audio::Health) -> &'static str {
    use legion_core::audio::Health;
    match health {
        Health::Ok => "Smart amp connected and unmuted — speakers should have full bass",
        Health::SoftIssue => {
            "Hardware amp is up — mute, volume, or wrong output needs a soft reset"
        }
        Health::HardwareBroken => {
            "Amp ACPI/driver/firmware incomplete — soft fix cannot invent a missing kernel driver"
        }
        Health::NotApplicable => "This machine does not expose an AW88399 smart amp",
    }
}

pub(crate) fn amp_action_tooltip(health: legion_core::audio::Health) -> &'static str {
    use legion_core::audio::Health;
    match health {
        Health::Ok => "Re-runs unmute, PipeWire restart, and sets the onboard speakers as default",
        Health::SoftIssue => {
            "Unmutes Speaker/Master, restarts PipeWire, and switches to onboard speakers"
        }
        Health::HardwareBroken => {
            "Still tries unmute/PipeWire — will not pretend the amp driver is fixed if it is missing"
        }
        Health::NotApplicable => {
            "Gen10 Pro 7 only (AW88399) — no action on this hardware; will be removed once kernel 7.3 ships"
        }
    }
}

pub(crate) fn amp_short_help(health: legion_core::audio::Health) -> &'static str {
    use legion_core::audio::Health;
    match health {
        Health::Ok => "Smart amp is connected. You can still refresh if sound feels off.",
        Health::SoftIssue => {
            "Amp is fine — volume, mute, or the wrong output is likely the issue."
        }
        Health::HardwareBroken => {
            "The woofer amp isn’t loaded. Soft fixes help mute/sink issues only — you may need a patched kernel."
        }
        Health::NotApplicable => {
            "No AW88399 smart amp on this model — Gen10 Pro 7 only. Feature will be removed once kernel 7.3 ships."
        }
    }
}
