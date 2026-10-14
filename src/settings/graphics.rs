//! Graphics — GPU working mode (MUX: Hybrid ↔ dGPU only).
//!
//! Everything starts hidden. Status comes from the root daemon on a worker
//! thread (the acpi_call backend needs root even to read), and the group only
//! appears when the firmware reports MUX support — or, on a verified model,
//! when the one missing piece is the acpi_call module.

use super::*;
use legion_core::gpu_mode::{GpuMode, GpuModeStatus};

const MODES: [GpuMode; 2] = [GpuMode::Hybrid, GpuMode::DgpuOnly];

fn mode_index(m: GpuMode) -> u32 {
    MODES.iter().position(|x| *x == m).unwrap_or(0) as u32
}

fn fetch_status_blocking() -> Result<GpuModeStatus, String> {
    match send_command(DaemonCommand::GetGpuMode) {
        Ok(DaemonResponse::GpuMode(st)) => Ok(st),
        Ok(DaemonResponse::Error(e)) => Err(e),
        Ok(_) => Err("Unexpected response from service".into()),
        Err(e) if is_version_skew_error(&e) => Err("Service is outdated. Reinstall to update".into()),
        Err(e) => Err(e),
    }
}

/// Widgets refreshed from one `GpuModeStatus`.
#[derive(Clone)]
struct GpuModeUi {
    group: adw::PreferencesGroup,
    banner: adw::Banner,
    combo: adw::ComboRow,
    active_row: adw::ActionRow,
    active_value: gtk::Label,
    note_row: adw::ActionRow,
    setup_row: adw::ActionRow,
    setup_btn: gtk::Button,
    guard: Rc<Cell<bool>>,
    shown: Rc<Cell<Option<GpuMode>>>,
}

impl GpuModeUi {
    fn select_silently(&self, m: GpuMode) {
        self.guard.set(true);
        self.combo.set_selected(mode_index(m));
        self.guard.set(false);
        self.shown.set(Some(m));
    }

    fn apply(&self, st: &GpuModeStatus) {
        let name = |m: Option<GpuMode>| m.map_or("Unknown", |m| m.label());
        if st.supported {
            self.group.set_visible(true);
            self.setup_row.set_visible(false);
            self.combo.set_visible(true);
            self.active_row.set_visible(true);
            if let Some(m) = st.configured {
                self.select_silently(m);
            }
            self.combo.set_sensitive(st.model_verified);
            self.combo.set_subtitle(&if st.model_verified {
                "Firmware setting, applies after restart".to_string()
            } else {
                "Not verified on this model, so switching is off".to_string()
            });
            tip(
                &self.combo,
                &format!(
                    "Hybrid: the integrated GPU drives the panel and the NVIDIA GPU renders on demand. \
                     dGPU only: the panel is wired straight to the NVIDIA GPU. Applies after a restart.\n\nBackend: {}",
                    st.backend
                ),
            );
            self.active_value.set_label(name(st.active));
            self.banner.set_revealed(st.reboot_pending);
            self.banner.set_title(&format!(
                "Restart required. {} applies on the next boot",
                name(st.configured)
            ));
            self.note_row.set_visible(!st.detail.is_empty());
            self.note_row.set_subtitle(&st.detail);
        } else if st.gamezone_present && !st.model_verified {
            // GameZone exists but the call strings are unverified here: no
            // ACPI call is made, the control is only explained.
            self.group.set_visible(true);
            self.combo.set_visible(false);
            self.active_row.set_visible(false);
            self.banner.set_revealed(false);
            self.note_row.set_visible(false);
            self.setup_row.set_visible(true);
            self.setup_btn.set_visible(false);
            self.setup_row.set_subtitle(&format!(
                "Not verified on this model ({}). Change the GPU mode in the BIOS (F2).",
                st.model_note
            ));
        } else if st.needs_acpi_call() {
            // Verified model with the GameZone interface but no backend yet.
            self.group.set_visible(true);
            self.combo.set_visible(false);
            self.active_row.set_visible(false);
            self.banner.set_revealed(false);
            self.note_row.set_visible(false);
            self.setup_row.set_visible(true);
            self.setup_btn.set_visible(st.acpi_call_installed);
            self.setup_row.set_subtitle(if st.acpi_call_installed {
                "Switching needs the acpi_call component. Load it for this session; nothing changes until you pick a mode."
            } else {
                "Switching needs the acpi_call component, which isn't installed. Install your distribution's acpi_call package, then reopen this page."
            });
        } else {
            self.group.set_visible(false);
        }
    }
}

fn refresh(ui: &GpuModeUi) {
    let ui = ui.clone();
    dispatch_async(
        fetch_status_blocking,
        "GPU mode probe stopped without a result",
        move |res| match res {
            Ok(st) => ui.apply(&st),
            Err(e) => {
                log::debug!("gpu mode: status unavailable: {e}");
                ui.group.set_visible(false);
            }
        },
    );
}

/// "Graphics" group with the GPU working-mode selector.
pub(crate) fn build_gpu_mode_group(
    toast_overlay: &adw::ToastOverlay,
    gate: &DaemonGate,
) -> adw::PreferencesGroup {
    let group = pref_group("Graphics", None);
    group.set_visible(false);
    gate.track(&group);

    let banner = adw::Banner::new("Restart required");
    banner.set_revealed(false);
    group.add(&banner);

    let model = gtk::StringList::new(&[]);
    for m in MODES {
        model.append(m.label());
    }
    let combo = adw::ComboRow::builder()
        .title("GPU mode")
        .model(&model)
        .visible(false)
        .build();
    combo.add_css_class("bordered-combo");
    tip(
        &combo,
        "Hybrid: the integrated GPU drives the panel and the NVIDIA GPU renders on demand. \
         dGPU only: the MUX wires the panel straight to the NVIDIA GPU. Applies after a restart.",
    );
    group.add(&combo);

    let active_row = adw::ActionRow::builder()
        .title("Active this boot")
        .activatable(false)
        .build();
    let active_value = gtk::Label::new(Some("Unknown"));
    active_value.add_css_class("dim-label");
    active_value.set_valign(Align::Center);
    active_row.add_suffix(&active_value);
    active_row.set_visible(false);
    group.add(&active_row);

    let note_row = property_row("Note", "", None);
    note_row.set_visible(false);
    group.add(&note_row);

    let setup_row = adw::ActionRow::builder()
        .title("GPU mode")
        .visible(false)
        .build();
    let setup_btn = gtk::Button::with_label("Load acpi_call");
    setup_btn.set_valign(Align::Center);
    setup_row.add_suffix(&setup_btn);
    group.add(&setup_row);

    let ui = GpuModeUi {
        group: group.clone(),
        banner,
        combo: combo.clone(),
        active_row,
        active_value,
        note_row,
        setup_row,
        setup_btn: setup_btn.clone(),
        guard: Rc::new(Cell::new(false)),
        shown: Rc::new(Cell::new(None)),
    };

    {
        let ui = ui.clone();
        let overlay = toast_overlay.clone();
        setup_btn.connect_clicked(move |btn| {
            btn.set_sensitive(false);
            let ui = ui.clone();
            let overlay = overlay.clone();
            let btn = btn.clone();
            run_setup_helper("load-acpi-call", move |result| {
                btn.set_sensitive(true);
                match result {
                    Ok(_) => {
                        toast_ok(&overlay, "acpi_call loaded");
                        refresh(&ui);
                    }
                    Err(e) => toast_error(&overlay, &e),
                }
            });
        });
    }

    {
        let ui = ui.clone();
        let overlay = toast_overlay.clone();
        combo.connect_selected_notify(move |row| {
            if ui.guard.get() {
                return;
            }
            let Some(&target) = MODES.get(row.selected() as usize) else {
                return;
            };
            let previous = ui.shown.get();
            if previous == Some(target) {
                return;
            }
            // Revert visually until the user confirms and the daemon agrees.
            if let Some(p) = previous {
                ui.select_silently(p);
            }
            let ui = ui.clone();
            let overlay = overlay.clone();
            confirm_risk(
                row,
                &format!("Switch GPU mode to {}?", target.label()),
                legion_core::gpu_mode::RISK_TEXT,
                "Switch, restart later",
                move |ok| {
                    if !ok {
                        return;
                    }
                    ui.combo.set_sensitive(false);
                    let ui = ui.clone();
                    let overlay = overlay.clone();
                    run_daemon_command_async(
                        DaemonCommand::SetGpuMode {
                            mode: target,
                            acknowledge: true,
                        },
                        move |result| {
                            ui.combo.set_sensitive(true);
                            match result {
                                Ok(DaemonResponse::GpuMode(st)) => {
                                    ui.apply(&st);
                                    toast_ok(
                                        &overlay,
                                        &format!(
                                            "GPU mode set to {}. Restart to apply",
                                            target.label()
                                        ),
                                    );
                                }
                                Ok(DaemonResponse::Error(e)) => toast_error(&overlay, &e),
                                Err(e) if is_version_skew_error(&e) => {
                                    toast_error(&overlay, "Service is outdated. Reinstall to update")
                                }
                                Err(e) => toast_error(&overlay, &e),
                                Ok(_) => toast_error(&overlay, "Unexpected response from service"),
                            }
                        },
                    );
                },
            );
        });
    }

    refresh(&ui);
    group
}
