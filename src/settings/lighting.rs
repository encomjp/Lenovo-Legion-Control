//! Lighting — one subtab per surface + per-key painter.

use super::{run_daemon_command_async, toast_error};
use crate::perkey;
use crate::widgets::{
    effect_tooltip, labeled_row_tip, page_lede, primary_button_tip, secondary_button_tip,
    section_tip, tip,
};
use legion_core::comms::{DaemonCommand, DaemonResponse};
use legion_core::config::ZoneEffect;
use legion_core::keyboard::{RgbEffect, RgbZone};
use adw::prelude::*;
use gtk::{glib, Align, Orientation};
use gtk4 as gtk;
use libadwaita as adw;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const EFFECT_IDS: &[&str] = &[
    "static",
    "color-pulse",
    "color-wave",
    "rainbow-wave",
    "screw-rainbow",
    "smooth",
    "color-change",
    "rain",
    "ripple",
    "reactive",
    "off",
];

/// Neutral dot for the zone tabs (no sun/moon/star metaphors).
const ZONE_ICON: &str = "media-record-symbolic";

const EFFECT_LABELS: &[&str] = &[
    "Static",
    "Pulse",
    "Wave",
    "Rainbow",
    "Spiral",
    "Smooth",
    "Color shift",
    "Rain",
    "Ripple",
    "Reactive",
    "Off",
];

/// Hardware lighting capability — `device::probe_lighting()`.
/// Spectrum gets every tab; FourZone and White collapse to the notice page.
fn lighting_kind() -> legion_core::device::LightingKind {
    legion_core::device::detect().capabilities.lighting_kind
}

pub fn build_lighting(
    toast_overlay: &adw::ToastOverlay,
    app: &adw::Application,
) -> (gtk::Box, adw::ViewStack) {
    let cfg = legion_core::config::get();
    let kind = lighting_kind();
    let page = page_lede("");

    // Legacy 4-zone boards get their own page (separate 33-byte protocol,
    // `legion_core::fourzone`); never the Spectrum tabs.
    if kind == legion_core::device::LightingKind::FourZone {
        return build_fourzone_page(&cfg, page, toast_overlay);
    }

    // White-only boards: the Spectrum write path targets 048d:c197 only, so
    // zone tabs would write to nothing and trigger Fix-page scares ("RGB
    // panic"). Collapse to a single notice + the brightness control the EC owns.
    if kind != legion_core::device::LightingKind::Spectrum {
        return build_simple_backlight_page(
            &cfg,
            page,
            "Keyboard backlight",
            "This model has a single-zone white backlight. Brightness: fn+Q cycles 50%, 100% and Off.",
            toast_overlay,
        );
    }

    let brush = Rc::new(Cell::new((cfg.ui_r, cfg.ui_g, cfg.ui_b)));

    let tabs = adw::ViewStack::new();
    tabs.set_hhomogeneous(false);
    tabs.set_vhomogeneous(false);
    tabs.set_vexpand(true);
    tabs.add_titled_with_icon(
        &build_keyboard_tab(&brush, &cfg.keyboard, toast_overlay, app),
        Some("keyboard"),
        "Keyboard",
        "input-keyboard-symbolic",
    );
    tabs.add_titled_with_icon(
        &build_zone_tab(
            "Front bar",
            "Chin and front accent LEDs",
            RgbZone::Front,
            &cfg.front,
            toast_overlay,
            None,
        ),
        Some("front"),
        "Front",
        ZONE_ICON,
    );
    tabs.add_titled_with_icon(
        &build_zone_tab(
            "Rear bar",
            "Rear and hinge accent LEDs",
            RgbZone::Rear,
            &cfg.rear,
            toast_overlay,
            None,
        ),
        Some("rear"),
        "Rear",
        ZONE_ICON,
    );
    tabs.add_titled_with_icon(
        &build_zone_tab(
            "Lid logo",
            "Logo colour and brightness",
            RgbZone::Logo,
            &cfg.logo,
            toast_overlay,
            Some(logo_power_row(&cfg, toast_overlay)),
        ),
        Some("logo"),
        "Logo",
        ZONE_ICON,
    );
    tabs.add_titled_with_icon(
        &build_more_tab(&cfg, toast_overlay),
        Some("more"),
        "More",
        "view-more-symbolic",
    );

    page.append(&tabs);

    if cfg.lighting_mode == "per-key" {
        tabs.set_visible_child_name("keyboard");
    }

    (page, tabs)
}

const FZ_SPEED_LABELS: &[&str] = &["Slowest", "Slow", "Fast", "Fastest"];
const FZ_BRIGHTNESS_LABELS: &[&str] = &["Off", "Low", "High"];
const FZ_DIR_LABELS: &[&str] = &["Left to right", "Right to left"];

/// Which controls make sense for an effect: (colours, speed, direction).
fn fz_sensitivity(fx: legion_core::fourzone::FzEffect) -> (bool, bool, bool) {
    use legion_core::fourzone::FzEffect;
    (
        fx.uses_colors(),
        fx != FzEffect::Static,
        fx == FzEffect::Wave,
    )
}

/// Legacy 4-zone RGB keyboard page (Legion 5 2020-2024, IdeaPad Gaming, LOQ):
/// effect, speed, brightness, four zone colours, wave direction. Writes go
/// straight to the hidraw node from this process (udev uaccess), exactly
/// like Spectrum; `legion_core::fourzone` re-checks the descriptor each time.
fn build_fourzone_page(
    cfg: &legion_core::config::AppConfig,
    page: gtk::Box,
    toast_overlay: &adw::ToastOverlay,
) -> (gtk::Box, adw::ViewStack) {
    use legion_core::fourzone::{FzEffect, WaveDir};
    let state = Rc::new(Cell::new(cfg.four_zone.to_state()));
    let s0 = state.get();

    // Changes apply and save themselves after ~250 ms without further edits.
    let commit: Rc<dyn Fn()> = {
        let state = state.clone();
        let toast = toast_overlay.clone();
        let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
        Rc::new(move || {
            if let Some(id) = pending.borrow_mut().take() {
                id.remove();
            }
            let state = state.clone();
            let toast = toast.clone();
            let pending_c = pending.clone();
            let id = glib::timeout_add_local_once(std::time::Duration::from_millis(250), move || {
                pending_c.borrow_mut().take();
                let s = state.get();
                super::dispatch_async(
                    move || legion_core::fourzone::apply_and_save(&s),
                    "4-zone write stopped without a result",
                    move |res| {
                        if let Err(e) = res {
                            toast_error(&toast, &e);
                        }
                    },
                );
            });
            *pending.borrow_mut() = Some(id);
        })
    };

    let (sec, card) = section_tip("4-zone RGB keyboard", None);
    let warn = gtk::Label::new(Some(
        "Experimental, tested by reference packets only. The 33-byte packets follow the layout \
         used by L5P-Keyboard-RGB, l5p-kbl, OpenRGB and Lenovo Legion Toolkit, \
         but this build has not been verified on a 4-zone laptop yet.",
    ));
    warn.set_wrap(true);
    warn.set_xalign(0.0);
    warn.add_css_class("row-sub");
    card.append(&warn);

    let labels: Vec<&str> = FzEffect::ALL.iter().map(|f| f.display_name()).collect();
    let effect_dd = gtk::DropDown::from_strings(&labels);
    effect_dd.set_selected(
        FzEffect::ALL
            .iter()
            .position(|f| *f == s0.effect)
            .unwrap_or(0) as u32,
    );
    tip(
        &effect_dd,
        "Static and Breath use the four zone colours; Wave and Smooth are firmware rainbows",
    );
    card.append(&labeled_row_tip("Effect", "", &effect_dd, None));

    let speed_dd = gtk::DropDown::from_strings(FZ_SPEED_LABELS);
    speed_dd.set_selected((s0.speed.clamp(1, 4) - 1) as u32);
    let speed_row = labeled_row_tip("Speed", "Breath, Wave and Smooth", &speed_dd, None);
    card.append(&speed_row);

    let bri_dd = gtk::DropDown::from_strings(FZ_BRIGHTNESS_LABELS);
    bri_dd.set_selected(s0.brightness.min(2) as u32);
    card.append(&labeled_row_tip(
        "Brightness",
        "Off, low or high",
        &bri_dd,
        None,
    ));

    let dir_dd = gtk::DropDown::from_strings(FZ_DIR_LABELS);
    dir_dd.set_selected(match s0.direction {
        WaveDir::Right => 0,
        WaveDir::Left => 1,
    });
    let dir_row = labeled_row_tip("Wave direction", "Wave only", &dir_dd, None);
    card.append(&dir_row);

    let zones = gtk::Box::new(Orientation::Horizontal, 12);
    zones.set_halign(Align::End);
    for i in 0..4 {
        let [r, g, b] = s0.colors[i];
        let state_c = state.clone();
        let commit_c = commit.clone();
        let picker = colour_picker((r, g, b), &format!("Zone {} colour", i + 1), move |r, g, b| {
            let mut s = state_c.get();
            s.colors[i] = [r, g, b];
            state_c.set(s);
            commit_c();
        });
        let col = gtk::Box::new(Orientation::Vertical, 4);
        let l = gtk::Label::new(Some(&format!("Zone {}", i + 1)));
        l.add_css_class("dim-label");
        col.append(&picker);
        col.append(&l);
        zones.append(&col);
    }
    let zones_row = labeled_row_tip("Zone colours", "Static and Breath", &zones, None);
    card.append(&zones_row);

    // Grey out controls the selected effect ignores.
    let sync = {
        let (speed_row, dir_row, zones_row) =
            (speed_row.clone(), dir_row.clone(), zones_row.clone());
        move |fx: FzEffect| {
            let (c, s, d) = fz_sensitivity(fx);
            zones_row.set_sensitive(c);
            speed_row.set_sensitive(s);
            dir_row.set_sensitive(d);
        }
    };
    sync(s0.effect);
    {
        let state = state.clone();
        let commit = commit.clone();
        effect_dd.connect_selected_notify(move |d| {
            let fx = FzEffect::ALL
                .get(d.selected() as usize)
                .copied()
                .unwrap_or(FzEffect::Static);
            let mut s = state.get();
            s.effect = fx;
            state.set(s);
            sync(fx);
            commit();
        });
    }
    {
        let state = state.clone();
        let commit = commit.clone();
        speed_dd.connect_selected_notify(move |d| {
            let mut s = state.get();
            s.speed = (d.selected() as u8 + 1).clamp(1, 4);
            state.set(s);
            commit();
        });
    }
    {
        let state = state.clone();
        let commit = commit.clone();
        bri_dd.connect_selected_notify(move |d| {
            let mut s = state.get();
            s.brightness = (d.selected() as u8).min(2);
            state.set(s);
            commit();
        });
    }
    {
        let state = state.clone();
        let commit = commit.clone();
        dir_dd.connect_selected_notify(move |d| {
            let mut s = state.get();
            s.direction = if d.selected() == 1 {
                WaveDir::Left
            } else {
                WaveDir::Right
            };
            state.set(s);
            commit();
        });
    }

    sec.append(&card);
    page.append(&sec);

    (page, adw::ViewStack::new()) // single page — no zone tabs
}

/// Fallback page for white-only lighting: a notice + the kbd_backlight
/// brightness slider the EC owns.
/// Writes go through the root daemon (`SetKbdBrightness`) — the user-space
/// GTK process cannot write `/sys/class/leds/.../brightness` directly.
fn build_simple_backlight_page(
    _cfg: &legion_core::config::AppConfig,
    page: gtk::Box,
    title: &str,
    note: &str,
    toast_overlay: &adw::ToastOverlay,
) -> (gtk::Box, adw::ViewStack) {
    let (sec, card) = section_tip(title, Some(note));
    let note = gtk::Label::new(Some(
        "This keyboard has white backlight only, no RGB zones or effects.\n\
         Adjust brightness with fn+Q (50%, 100%, Off) or with the slider here.",
    ));
    note.set_wrap(true);
    note.set_xalign(0.0);
    note.add_css_class("row-sub");
    card.append(&note);

    // Brightness slider — reads are local sysfs, writes go through the daemon.
    let level = legion_core::keyboard::brightness().unwrap_or(0);
    let max = legion_core::keyboard::max_brightness().unwrap_or(2).max(1);
    let row = adw::ActionRow::builder()
        .title("Backlight brightness")
        .subtitle(backlight_label(level, max))
        .activatable(false)
        .build();
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, max as f64, 1.0);
    scale.set_value(level.min(max) as f64);
    scale.set_hexpand(true);
    scale.set_valign(Align::Center);
    scale.set_increments(1.0, 1.0);
    scale.set_digits(0);
    scale.set_draw_value(false);
    // Stepped marks: 0=Off, then even percent steps (max==2 → Off/50%/100%).
    for lvl in 0..=max {
        scale.add_mark(
            lvl as f64,
            gtk::PositionType::Bottom,
            Some(&backlight_label(lvl, max)),
        );
    }
    let row_c = row.clone();
    let overlay = toast_overlay.clone();
    // Guard against recursion when snapping the slider to integer steps, and
    // skip duplicate daemon writes while dragging within one step.
    let suppress = Rc::new(Cell::new(false));
    let suppress_c = suppress.clone();
    let last_sent = Rc::new(Cell::new(level.min(max)));
    scale.connect_value_changed(move |s| {
        if suppress_c.get() {
            return;
        }
        let maxv = legion_core::keyboard::max_brightness().unwrap_or(2).max(1);
        let v = s.value().round().clamp(0.0, maxv as f64) as u8;
        // Snap fractional drags to the integer step without re-entering.
        if (s.value() - v as f64).abs() > f64::EPSILON {
            suppress_c.set(true);
            s.set_value(v as f64);
            suppress_c.set(false);
        }
        row_c.set_subtitle(&backlight_label(v, maxv));
        if v == last_sent.get() {
            return;
        }
        last_sent.set(v);
        let overlay = overlay.clone();
        let row_c = row_c.clone();
        run_daemon_command_async(DaemonCommand::SetKbdBrightness(v), move |result| match result {
            Ok(DaemonResponse::Ok) => {}
            Ok(DaemonResponse::Error(e)) => {
                row_c.set_subtitle(&format!("Error: {e}"));
                toast_error(&overlay, &e);
            }
            Err(e) => {
                toast_error(&overlay, &e);
            }
            _ => {}
        });
    });
    row.add_suffix(&scale);
    card.append(&row);
    sec.append(&card);
    page.append(&sec);

    let tabs = adw::ViewStack::new(); // empty — no zone tabs on white-only boards
    (page, tabs)
}

/// Human label for a brightness level: 0 -> "Off", otherwise the rounded
/// percentage of `max` (u32 math, since u8 `level * 100` overflows).
fn backlight_label(level: u8, max: u8) -> String {
    let max = max.max(1) as u32;
    if level == 0 {
        "Off".to_string()
    } else {
        format!("{}%", (level as u32 * 100 + max / 2) / max)
    }
}

/// Spectrum brightness is 0..=9; show it as a percentage.
fn level_label(level: u8) -> String {
    backlight_label(level.min(9), 9)
}

/// Width of the shared label column so every control starts at the same x.
const ROW_LABEL_W: i32 = 96;

/// One aligned row: title in a fixed-width column, control after it.
fn field_row(title: &str, field: &impl IsA<gtk::Widget>, tooltip: &str) -> gtk::Box {
    let row = gtk::Box::new(Orientation::Horizontal, 12);
    row.add_css_class("panel-row");
    let t = gtk::Label::new(Some(title));
    t.add_css_class("row-title");
    t.set_xalign(0.0);
    t.set_valign(Align::Center);
    t.set_width_request(ROW_LABEL_W);
    row.append(&t);
    let f = field.as_ref();
    f.set_valign(Align::Center);
    row.append(f);
    tip(&row, tooltip);
    row
}

fn build_keyboard_tab(
    brush: &Rc<Cell<(u8, u8, u8)>>,
    layer: &ZoneEffect,
    toast: &adw::ToastOverlay,
    app: &adw::Application,
) -> gtk::Box {
    let box_ = gtk::Box::new(Orientation::Vertical, 10);
    box_.set_margin_top(4);

    let (sec_fx, fx_card) = section_tip("Whole keyboard", Some("One effect across every key"));
    fx_card.append(&zone_editor(RgbZone::Keyboard, layer, true, toast));
    box_.append(&sec_fx);

    let (sec_map, map_card) = section_tip(
        "Per-key lighting",
        Some("Paint single keys with the brush colour"),
    );
    let open_btn = secondary_button_tip(
        "Open key painter",
        Some("Opens a dedicated window with a full-size per-key painter"),
    );
    open_btn.set_halign(Align::End);
    map_card.append(&colour_toolbar(brush, Some(&open_btn)));
    let brush_w = brush.clone();
    let app_w = app.clone();
    let win_slot: Rc<RefCell<Option<adw::Window>>> = Rc::new(RefCell::new(None));
    let win_slot_c = win_slot.clone();
    open_btn.connect_clicked(move |_| {
        if let Some(existing) = win_slot_c.borrow().as_ref() {
            existing.present();
            return;
        }
        let win = open_perkey_window(&app_w, brush_w.clone());
        let slot = win_slot_c.clone();
        win.connect_close_request(move |_| {
            *slot.borrow_mut() = None;
            glib::Propagation::Proceed
        });
        *win_slot_c.borrow_mut() = Some(win);
    });
    box_.append(&sec_map);

    box_
}

fn open_perkey_window(app: &adw::Application, brush: Rc<Cell<(u8, u8, u8)>>) -> adw::Window {
    let win = adw::Window::builder()
        .application(app)
        .title("Individual key lighting")
        .default_width(900)
        .default_height(620)
        .build();

    let toast = adw::ToastOverlay::new();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());

    let body = gtk::Box::new(Orientation::Vertical, 16);
    body.add_css_class("page");
    body.set_margin_top(8);
    body.set_margin_bottom(24);
    body.set_margin_start(20);
    body.set_margin_end(20);

    let lede = gtk::Label::new(Some(
        "Click or drag keys to paint. Uses the brush colour from the Lighting tab.",
    ));
    lede.add_css_class("page-lede");
    lede.set_halign(Align::Start);
    lede.set_wrap(true);
    tip(
        &lede,
        "Paint individual keys with the brush colour, opens full-size so keys stay easy to hit",
    );
    body.append(&lede);

    let brush_row = colour_toolbar(&brush, None);
    body.append(&brush_row);
    body.append(&perkey::build_perkey_editor(brush));

    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .child(&body)
        .build();
    toolbar.set_content(Some(&scroll));
    toast.set_child(Some(&toolbar));
    win.set_content(Some(&toast));
    win.present();
    win
}

fn build_zone_tab(
    title: &str,
    blurb: &str,
    zone: RgbZone,
    layer: &ZoneEffect,
    toast: &adw::ToastOverlay,
    extra: Option<gtk::Box>,
) -> gtk::Box {
    let box_ = gtk::Box::new(Orientation::Vertical, 10);
    box_.set_margin_top(4);

    let (sec, card) = section_tip(title, Some(blurb));
    card.append(&zone_editor(zone, layer, true, toast));
    if let Some(extra) = extra {
        card.append(&extra);
    }
    box_.append(&sec);
    box_
}

/// The lid-logo LED on/off switch: lives on the Logo tab next to its
/// colour/brightness editor instead of hiding under More.
fn logo_power_row(cfg: &legion_core::config::AppConfig, toast: &adw::ToastOverlay) -> gtk::Box {
    let logo = gtk::Switch::builder().active(cfg.logo_on).build();
    let logo_suppress = Rc::new(Cell::new(true));
    let logo_suppress_c = logo_suppress.clone();
    let toast_logo = toast.clone();
    logo.connect_active_notify(move |s| {
        if logo_suppress_c.get() {
            return;
        }
        legion_core::keyboard::set_logo_async(s.is_active());
        let t = adw::Toast::new(if s.is_active() { "Logo on" } else { "Logo off" });
        t.set_timeout(1);
        toast_logo.add_toast(t);
    });
    logo_suppress.set(false);
    logo.set_halign(Align::Start);
    field_row("Logo LED", &logo, "Turns the physical lid logo LED on or off")
}

fn build_more_tab(cfg: &legion_core::config::AppConfig, toast: &adw::ToastOverlay) -> gtk::Box {
    let toast = toast.clone();
    let box_ = gtk::Box::new(Orientation::Vertical, 10);
    box_.set_margin_top(4);

    let (sec_all, all_card) = section_tip(
        "All lights",
        Some("One effect on keyboard, front, rear and logo together"),
    );
    all_card.append(&zone_editor(RgbZone::All, &cfg.keyboard, false, &toast));
    box_.append(&sec_all);

    let (sec_r, restore_card) = section_tip("Restore", None);
    let restore = secondary_button_tip(
        "Reapply",
        Some("Push the saved lighting settings to the keyboard again"),
    );
    let toast_r = toast.clone();
    restore.connect_clicked(move |_| {
        legion_core::keyboard::restore_lighting_async();
        let t = adw::Toast::new("Saved lighting reapplied");
        t.set_timeout(2);
        toast_r.add_toast(t);
    });
    restore_card.append(&labeled_row_tip(
        "Reapply saved lighting",
        "Use after sleep or if colours look wrong",
        &restore,
        Some("Sends your saved effect, colour and brightness to the hardware again"),
    ));
    box_.append(&sec_r);

    box_
}

/// Effects that sweep through hues rather than using the chosen colour.
fn is_rainbow(id: &str) -> bool {
    matches!(id, "rainbow-wave" | "screw-rainbow" | "smooth" | "color-change")
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (f64, f64, f64) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = (h.rem_euclid(360.0)) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    (r + m, g + m, b + m)
}

/// Slim, flat preview of what a zone is set to: the colour (or a hue
/// sweep for rainbow effects), dimmed with brightness, plus its name and
/// brightness. Returns the widget and a refresh closure.
fn preview_strip(
    effect_dd: &gtk::DropDown,
    color: &Rc<Cell<(u8, u8, u8)>>,
    brightness: &Rc<Cell<u8>>,
    show_pct: bool,
) -> (gtk::Overlay, Rc<dyn Fn()>) {
    let effect_id = {
        let dd = effect_dd.clone();
        move || {
            EFFECT_IDS
                .get(dd.selected() as usize)
                .copied()
                .unwrap_or("static")
        }
    };
    // Lit amount: 0 (off) .. 1 (full), kept visible at low levels.
    let alpha_for = |level: u8| 0.3 + 0.7 * (level.min(9) as f64 / 9.0);

    let area = gtk::DrawingArea::new();
    area.add_css_class("lighting-preview");
    area.set_content_height(40);
    area.set_hexpand(true);
    {
        let (color, brightness, effect_id) = (color.clone(), brightness.clone(), effect_id.clone());
        area.set_draw_func(move |area, cr, w, h| {
            let (w, h) = (w as f64, h as f64);
            let r = 8.0;
            let pi = std::f64::consts::PI;
            let outline = |cr: &gtk::cairo::Context| {
                cr.new_sub_path();
                cr.arc(w - r - 0.5, r + 0.5, r, -pi / 2.0, 0.0);
                cr.arc(w - r - 0.5, h - r - 0.5, r, 0.0, pi / 2.0);
                cr.arc(r + 0.5, h - r - 0.5, r, pi / 2.0, pi);
                cr.arc(r + 0.5, r + 0.5, r, pi, 3.0 * pi / 2.0);
                cr.close_path();
            };
            let fg = area.color();
            let id = effect_id();
            let level = brightness.get();
            let (cr_, cg, cb) = color.get();
            let (cr_, cg, cb) = (cr_ as f64 / 255.0, cg as f64 / 255.0, cb as f64 / 255.0);
            let a = alpha_for(level);
            outline(cr);
            if id == "off" || level == 0 {
                cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.06);
            } else if is_rainbow(id) {
                let g = gtk::cairo::LinearGradient::new(0.0, 0.0, w, 0.0);
                for i in 0..=6 {
                    let (hr, hg, hb) = hsl_to_rgb(i as f64 * 60.0, 0.62, 0.66);
                    g.add_color_stop_rgba(i as f64 / 6.0, hr, hg, hb, a);
                }
                let _ = cr.set_source(&g);
            } else if matches!(id, "color-pulse" | "color-wave" | "ripple") {
                let g = gtk::cairo::LinearGradient::new(0.0, 0.0, w, 0.0);
                g.add_color_stop_rgba(0.0, cr_, cg, cb, a);
                g.add_color_stop_rgba(1.0, cr_, cg, cb, a * 0.55);
                let _ = cr.set_source(&g);
            } else {
                cr.set_source_rgba(cr_, cg, cb, a);
            }
            let _ = cr.fill_preserve();
            cr.set_line_width(1.0);
            cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.14);
            let _ = cr.stroke();
        });
    }

    let name = gtk::Label::new(None);
    name.add_css_class("lighting-preview-text");
    name.set_hexpand(true);
    name.set_xalign(0.0);
    let pct = gtk::Label::new(None);
    pct.add_css_class("lighting-preview-text");
    pct.add_css_class("numeric");
    pct.set_visible(show_pct);
    let labels = gtk::Box::new(Orientation::Horizontal, 8);
    labels.set_margin_start(14);
    labels.set_margin_end(14);
    labels.set_valign(Align::Center);
    labels.set_can_target(false);
    labels.append(&name);
    labels.append(&pct);

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&area));
    overlay.add_overlay(&labels);

    let refresh: Rc<dyn Fn()> = {
        let (area, color, brightness) = (area.clone(), color.clone(), brightness.clone());
        Rc::new(move || {
            let id = effect_id();
            let level = brightness.get();
            let off = id == "off" || level == 0;
            let idx = EFFECT_IDS.iter().position(|e| *e == id).unwrap_or(0);
            name.set_text(EFFECT_LABELS.get(idx).copied().unwrap_or("Static"));
            pct.set_text(&level_label(level));
            // Text colour follows what is behind it.
            let (r, g, b) = color.get();
            let lum = (0.2126 * r as f64 + 0.7152 * g as f64 + 0.0722 * b as f64) / 255.0;
            let class = if off || alpha_for(level) < 0.6 {
                None
            } else if is_rainbow(id) || lum > 0.5 {
                Some("on-light")
            } else {
                Some("on-dark")
            };
            for l in [&name, &pct] {
                l.remove_css_class("on-light");
                l.remove_css_class("on-dark");
                if let Some(c) = class {
                    l.add_css_class(c);
                }
            }
            area.queue_draw();
        })
    };
    refresh();
    (overlay, refresh)
}

fn zone_editor(
    zone: RgbZone,
    layer: &ZoneEffect,
    show_brightness: bool,
    toast: &adw::ToastOverlay,
) -> gtk::Box {
    let toast = toast.clone();
    let wrap = gtk::Box::new(Orientation::Vertical, 0);
    let color = Rc::new(Cell::new((layer.r, layer.g, layer.b)));
    let speed = Rc::new(Cell::new(layer.speed.clamp(1, 3)));
    // Zones without their own slider (All lights) apply the saved level, or
    // full brightness if that was off.
    let start_level = match layer.brightness.min(9) {
        0 if !show_brightness => 9,
        n => n,
    };
    let brightness = Rc::new(Cell::new(start_level));

    let effect_dd = gtk::DropDown::from_strings(EFFECT_LABELS);
    let fx_sel = EFFECT_IDS
        .iter()
        .position(|e| *e == layer.effect)
        .unwrap_or(0) as u32;
    effect_dd.set_selected(fx_sel);
    effect_dd.set_halign(Align::Start);
    effect_dd.set_width_request(200);
    tip(
        &effect_dd,
        effect_tooltip(EFFECT_IDS.get(fx_sel as usize).copied().unwrap_or("static")),
    );

    // Zones other than All Lights save and apply themselves shortly after the
    // last change. All Lights overwrites every zone, so it keeps a button.
    let auto = zone != RgbZone::All;
    let commit = auto_commit(&effect_dd, &color, &speed, &brightness, zone);
    let (strip, refresh) = preview_strip(&effect_dd, &color, &brightness, show_brightness);
    strip.set_margin_bottom(6);
    wrap.append(&strip);

    wrap.append(&field_row(
        "Effect",
        &effect_dd,
        "Lighting animation for this surface",
    ));

    let speed_dd = gtk::DropDown::from_strings(&["Slow", "Normal", "Fast"]);
    speed_dd.set_halign(Align::Start);
    speed_dd.set_width_request(200);
    speed_dd.set_selected((speed.get().saturating_sub(1) as u32).min(2));
    let speed_c = speed.clone();
    let commit_s = commit.clone();
    speed_dd.connect_selected_notify(move |d| {
        speed_c.set((d.selected() as u8).saturating_add(1).min(3));
        commit_s();
    });
    let speed_row = field_row("Speed", &speed_dd, "How fast the animation moves");
    let sync_speed = {
        let speed_row = speed_row.clone();
        move |idx: usize| {
            let id = EFFECT_IDS.get(idx).copied().unwrap_or("static");
            speed_row.set_visible(!matches!(id, "static" | "off"));
        }
    };
    sync_speed(fx_sel as usize);
    wrap.append(&speed_row);

    {
        let refresh = refresh.clone();
        let commit = commit.clone();
        effect_dd.connect_selected_notify(move |d| {
            let idx = d.selected() as usize;
            let id = EFFECT_IDS.get(idx).copied().unwrap_or("static");
            tip(d, effect_tooltip(id));
            sync_speed(idx);
            refresh();
            commit();
        });
    }

    let picker = colour_picker(color.get(), "Colour", {
        let color = color.clone();
        let refresh = refresh.clone();
        let commit = commit.clone();
        move |r, g, b| {
            color.set((r, g, b));
            refresh();
            commit();
        }
    });
    let colour_box = gtk::Box::new(Orientation::Horizontal, 12);
    colour_box.append(&picker);
    colour_box.append(&preset_swatches(&picker));
    wrap.append(&field_row(
        "Colour",
        &colour_box,
        "Used by Static, Pulse, Wave and the other single-colour effects",
    ));

    if show_brightness {
        let bright_slider = gtk::Scale::with_range(Orientation::Horizontal, 0.0, 9.0, 1.0);
        bright_slider.set_value(brightness.get() as f64);
        bright_slider.set_draw_value(false);
        bright_slider.set_digits(0);
        bright_slider.set_hexpand(true);
        bright_slider.set_width_request(160);
        let bright_val = gtk::Label::new(Some(&level_label(brightness.get())));
        bright_val.add_css_class("numeric");
        bright_val.add_css_class("scale-value");
        bright_val.set_xalign(1.0);
        let bri_val_c = bright_val.clone();
        let bri_c = brightness.clone();
        let refresh_b = refresh.clone();
        let commit_b = commit.clone();
        bright_slider.connect_value_changed(move |s| {
            let level = s.value().round().clamp(0.0, 9.0) as u8;
            bri_val_c.set_text(&level_label(level));
            bri_c.set(level);
            refresh_b();
            commit_b();
        });
        tip(&bright_slider, "0% turns this zone off");
        let bright_box = gtk::Box::new(Orientation::Horizontal, 10);
        bright_box.set_hexpand(true);
        bright_box.append(&bright_slider);
        bright_box.append(&bright_val);
        wrap.append(&field_row("Brightness", &bright_box, "Brightness of this zone only"));
    }

    if !auto {
        let apply = apply_button(&effect_dd, &color, &speed, &brightness, zone, &toast);
        apply.set_halign(Align::End);
        apply.set_margin_top(8);
        wrap.append(&apply);
    }

    wrap
}

/// Brush colour row: picker and presets, plus an optional trailing button.
fn colour_toolbar(brush: &Rc<Cell<(u8, u8, u8)>>, trailing: Option<&gtk::Button>) -> gtk::Box {
    let row = gtk::Box::new(Orientation::Horizontal, 12);
    row.set_valign(Align::Center);

    let t = gtk::Label::new(Some("Brush colour"));
    t.add_css_class("row-title");
    t.set_xalign(0.0);
    t.set_width_request(ROW_LABEL_W);
    tip(&t, "Paint colour for per-key mode, also used by coloured effects");
    row.append(&t);

    let picker = colour_picker(brush.get(), "Brush colour", {
        let brush = brush.clone();
        move |r, g, b| {
            brush.set((r, g, b));
            legion_core::config::set_ui_color(r, g, b);
        }
    });
    row.append(&picker);
    row.append(&preset_swatches(&picker));

    let spacer = gtk::Box::new(Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    row.append(&spacer);
    if let Some(btn) = trailing {
        row.append(btn);
    }
    row
}

/// Quick colour presets. Picking one drives the colour button, which in turn
/// notifies whoever owns the colour.
fn preset_swatches(picker: &gtk::ColorDialogButton) -> gtk::Box {
    let row = gtk::Box::new(Orientation::Horizontal, 8);
    row.set_valign(Align::Center);
    for (name, r, g, b) in [
        ("Red", 200u8, 16u8, 46u8),
        ("White", 255, 255, 255),
        ("Ice", 120, 220, 255),
        ("Black", 0, 0, 0),
    ] {
        let btn = gtk::Button::new();
        btn.add_css_class("swatch");
        btn.add_css_class(&format!("swatch-{:02x}{:02x}{:02x}", r, g, b));
        btn.set_valign(Align::Center);
        tip(&btn, &format!("{name} #{r:02X}{g:02X}{b:02X}"));
        let picker_p = picker.clone();
        btn.connect_clicked(move |_| set_picker_rgba(&picker_p, r, g, b));
        row.append(&btn);
    }
    row
}

/// Colour button; the hex value lives in its tooltip only.
fn colour_picker(
    initial: (u8, u8, u8),
    what: &str,
    on_change: impl Fn(u8, u8, u8) + 'static,
) -> gtk::ColorDialogButton {
    let dialog = gtk::ColorDialog::new();
    dialog.set_with_alpha(false);
    let picker = gtk::ColorDialogButton::new(Some(dialog));
    picker.add_css_class("colour-picker");
    picker.set_valign(Align::Center);
    picker.set_halign(Align::Start);
    set_picker_rgba(&picker, initial.0, initial.1, initial.2);
    let what = what.to_string();
    let hex_tip = move |p: &gtk::ColorDialogButton, (r, g, b): (u8, u8, u8)| {
        tip(p, &format!("{what} #{r:02X}{g:02X}{b:02X}"));
    };
    hex_tip(&picker, initial);
    picker.connect_rgba_notify(move |p| {
        let (r, g, b) = rgba_to_rgb(p.rgba());
        hex_tip(p, (r, g, b));
        on_change(r, g, b);
    });
    picker
}

fn set_picker_rgba(picker: &gtk::ColorDialogButton, r: u8, g: u8, b: u8) {
    picker.set_rgba(&gtk::gdk::RGBA::new(
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        1.0,
    ));
}

fn rgba_to_rgb(rgba: gtk::gdk::RGBA) -> (u8, u8, u8) {
    (
        (rgba.red().clamp(0.0, 1.0) * 255.0).round() as u8,
        (rgba.green().clamp(0.0, 1.0) * 255.0).round() as u8,
        (rgba.blue().clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

/// Debounced live apply: waits for ~250 ms without changes, then sends the
/// state through `apply_lighting`, which also writes the saved zone config.
fn auto_commit(
    effect_dd: &gtk::DropDown,
    color: &Rc<Cell<(u8, u8, u8)>>,
    speed: &Rc<Cell<u8>>,
    brightness: &Rc<Cell<u8>>,
    zone: RgbZone,
) -> Rc<dyn Fn()> {
    let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    let (effect_dd, color, speed, brightness) = (
        effect_dd.clone(),
        color.clone(),
        speed.clone(),
        brightness.clone(),
    );
    Rc::new(move || {
        if zone == RgbZone::All {
            return;
        }
        if let Some(id) = pending.borrow_mut().take() {
            id.remove();
        }
        let pending_c = pending.clone();
        let (effect_dd, color, speed, brightness) = (
            effect_dd.clone(),
            color.clone(),
            speed.clone(),
            brightness.clone(),
        );
        let id = glib::timeout_add_local_once(std::time::Duration::from_millis(250), move || {
            pending_c.borrow_mut().take();
            let id = EFFECT_IDS
                .get(effect_dd.selected() as usize)
                .copied()
                .unwrap_or("static");
            let (r, g, b) = color.get();
            apply_lighting(id, r, g, b, speed.get(), brightness.get(), zone);
        });
        *pending.borrow_mut() = Some(id);
    })
}

fn apply_button(
    effect_dd: &gtk::DropDown,
    color: &Rc<Cell<(u8, u8, u8)>>,
    speed: &Rc<Cell<u8>>,
    brightness: &Rc<Cell<u8>>,
    zone: RgbZone,
    toast: &adw::ToastOverlay,
) -> gtk::Button {
    let apply = primary_button_tip(
        "Use on all lights",
        Some("Send this effect, colour, speed and brightness to every light now"),
    );
    let effect_dd_c = effect_dd.clone();
    let color_a = color.clone();
    let speed_a = speed.clone();
    let brightness_a = brightness.clone();
    let toast_a = toast.clone();
    apply.connect_clicked(move |_| {
        let idx = effect_dd_c.selected() as usize;
        let id = EFFECT_IDS.get(idx).copied().unwrap_or("static");
        let (r, g, b) = color_a.get();
        apply_lighting(id, r, g, b, speed_a.get(), brightness_a.get(), zone);
        let label = EFFECT_LABELS.get(idx).copied().unwrap_or("Effect");
        let msg = if id == "off" {
            format!("{} off", zone.display_name())
        } else {
            format!(
                "{label} on {}, {}",
                zone.display_name(),
                level_label(brightness_a.get())
            )
        };
        let t = adw::Toast::new(&msg);
        t.set_timeout(2);
        toast_a.add_toast(t);
    });
    apply
}

fn apply_lighting(effect: &str, r: u8, g: u8, b: u8, speed: u8, brightness: u8, zone: RgbZone) {
    if effect.eq_ignore_ascii_case("off") {
        legion_core::keyboard::set_rgb_effect_zone_async(RgbEffect::Static, 0, 0, 0, 2, 9, zone);
        return;
    }
    if let Some(fx) = RgbEffect::from_name(effect) {
        legion_core::keyboard::set_rgb_effect_zone_async(fx, r, g, b, speed, brightness, zone);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spectrum_levels_read_as_percent() {
        assert_eq!(level_label(0), "Off");
        assert_eq!(level_label(1), "11%");
        assert_eq!(level_label(5), "56%");
        assert_eq!(level_label(9), "100%");
    }

    #[test]
    fn fourzone_controls_follow_effect() {
        use legion_core::fourzone::FzEffect;
        assert_eq!(fz_sensitivity(FzEffect::Static), (true, false, false));
        assert_eq!(fz_sensitivity(FzEffect::Breath), (true, true, false));
        assert_eq!(fz_sensitivity(FzEffect::Wave), (false, true, true));
        assert_eq!(fz_sensitivity(FzEffect::Smooth), (false, true, false));
        assert_eq!(FZ_BRIGHTNESS_LABELS.len(), 3); // index == brightness byte
        assert_eq!(FZ_SPEED_LABELS.len(), 4); // index + 1 == speed byte
    }

    #[test]
    fn backlight_label_steps() {
        // Y7000P white backlight: max==2 → Off / 50% / 100%.
        assert_eq!(super::backlight_label(0, 2), "Off");
        assert_eq!(super::backlight_label(1, 2), "50%");
        assert_eq!(super::backlight_label(2, 2), "100%");
        // Generic max uses u32 percent math (no u8 overflow).
        assert_eq!(super::backlight_label(0, 0), "Off");
        assert_eq!(super::backlight_label(3, 9), "33%");
    }
}
