//! Lighting — one subtab per surface + per-key painter.

use crate::perkey;
use crate::widgets::{effect_tooltip, labeled_row_tip, page_lede, section_tip, tip};
use legion_core::config::ZoneEffect;
use legion_core::keyboard::{RgbEffect, RgbZone};

use adw::prelude::*;
use gtk::{glib, Align, Orientation};
use gtk4 as gtk;
use libadwaita as adw;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

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

pub fn build_lighting(
    toast_overlay: &adw::ToastOverlay,
    app: &adw::Application,
) -> (gtk::Box, adw::ViewStack) {
    let cfg = legion_core::config::get();
    let page = page_lede("");

    let brush = Rc::new(Cell::new((cfg.ui_r, cfg.ui_g, cfg.ui_b)));

    let tabs = adw::ViewStack::new();
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
        ),
        Some("front"),
        "Front",
        "go-down-symbolic",
    );
    tabs.add_titled_with_icon(
        &build_zone_tab(
            "Rear bar",
            "Rear / hinge accent LEDs",
            RgbZone::Rear,
            &cfg.rear,
            toast_overlay,
        ),
        Some("rear"),
        "Rear",
        "go-up-symbolic",
    );
    tabs.add_titled_with_icon(
        &build_zone_tab(
            "Lid logo",
            "Logo colour — power switch is under More",
            RgbZone::Logo,
            &cfg.logo,
            toast_overlay,
        ),
        Some("logo"),
        "Logo",
        "starred-symbolic",
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

fn build_keyboard_tab(
    brush: &Rc<Cell<(u8, u8, u8)>>,
    layer: &ZoneEffect,
    toast: &adw::ToastOverlay,
    app: &adw::Application,
) -> gtk::Box {
    let box_ = gtk::Box::new(Orientation::Vertical, 0);
    box_.set_margin_top(14);

    let (sec_fx, fx_card) = section_tip("Whole keyboard", None);
    fx_card.append(&zone_editor(RgbZone::Keyboard, layer, true, toast));
    box_.append(&sec_fx);

    let (sec_brush, brush_card) = section_tip("Paint colour", None);
    brush_card.append(&colour_toolbar(brush));
    box_.append(&sec_brush);

    let (sec_map, map_card) = section_tip("Per-key lighting", None);
    let open_btn = gtk::Button::with_label("Open individual key lighting…");
    open_btn.add_css_class("suggested-action");
    open_btn.add_css_class("pill-btn");
    open_btn.set_halign(Align::Start);
    tip(
        &open_btn,
        "Opens a dedicated window with a full-size per-key painter",
    );
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
    map_card.append(&open_btn);
    box_.append(&sec_map);

    box_
}

fn open_perkey_window(app: &adw::Application, brush: Rc<Cell<(u8, u8, u8)>>) -> adw::Window {
    let win = adw::Window::builder()
        .application(app)
        .title("Individual key lighting")
        .default_width(1180)
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
        "Paint individual keys with the brush colour — opens full-size so keys stay easy to hit",
    );
    body.append(&lede);

    let brush_row = colour_toolbar(&brush);
    body.append(&brush_row);
    body.append(&perkey::build_perkey_editor(brush));

    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
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
    _blurb: &str,
    zone: RgbZone,
    layer: &ZoneEffect,
    toast: &adw::ToastOverlay,
) -> gtk::Box {
    let box_ = gtk::Box::new(Orientation::Vertical, 0);
    box_.set_margin_top(14);

    let (sec, card) = section_tip(title, None);
    card.append(&zone_editor(zone, layer, false, toast));
    box_.append(&sec);
    box_
}

fn build_more_tab(cfg: &legion_core::config::AppConfig, toast: &adw::ToastOverlay) -> gtk::Box {
    let toast = toast.clone();
    let box_ = gtk::Box::new(Orientation::Vertical, 0);
    box_.set_margin_top(14);

    let (sec_look, look) = section_tip("Brightness and logo", None);

    let bright = gtk::Scale::with_range(Orientation::Horizontal, 0.0, 9.0, 1.0);
    bright.set_value(cfg.brightness as f64);
    bright.set_draw_value(true);
    bright.add_css_class("brightness-slider");
    bright.set_digits(0);
    bright.set_hexpand(true);
    bright.set_width_request(240);
    let bright_suppress = Rc::new(Cell::new(true));
    let bright_suppress_c = bright_suppress.clone();
    let bri_ticket = Rc::new(Cell::new(0u32));
    let toast_bri = toast.clone();
    bright.connect_value_changed(move |s| {
        if bright_suppress_c.get() {
            return;
        }
        let level = s.value().round().clamp(0.0, 9.0) as u8;
        let t = bri_ticket.get().wrapping_add(1);
        bri_ticket.set(t);
        let bri_ticket = bri_ticket.clone();
        let toast_c = toast_bri.clone();
        glib::timeout_add_local_once(Duration::from_millis(80), move || {
            if bri_ticket.get() == t {
                legion_core::keyboard::set_rgb_brightness_async(level);
                let msg = if level == 0 {
                    "Lights off".to_string()
                } else {
                    format!("Brightness → {level}")
                };
                let t = adw::Toast::new(&msg);
                t.set_timeout(1);
                toast_c.add_toast(t);
            }
        });
    });
    bright_suppress.set(false);
    tip(
        &bright,
        "Spectrum brightness 0–9 · 0 turns lights off · saved to your config",
    );
    look.append(&labeled_row_tip(
        "Brightness",
        "0 off · 9 max",
        &bright,
        Some("Applies to keyboard and accent Spectrum zones"),
    ));

    let logo = gtk::Switch::builder().active(cfg.logo_on).build();
    tip(
        &logo,
        "Lid Legion logo LED power — colour is set under the Logo tab",
    );
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
    look.append(&labeled_row_tip(
        "Lid logo power",
        "Hardware on/off (colour is on the Logo tab)",
        &logo,
        Some("Turns the physical lid logo LED on or off"),
    ));
    box_.append(&sec_look);

    let (sec_all, all_card) = section_tip("Everything at once", None);
    tip(
        &all_card,
        "Writes one effect to keyboard, front, rear, and logo together",
    );
    all_card.append(&zone_editor(RgbZone::All, &cfg.keyboard, false, &toast));
    box_.append(&sec_all);

    let (sec_r, restore_card) = section_tip("Session", None);
    let restore = gtk::Button::with_label("Re-apply saved");
    restore.add_css_class("flat");
    tip(
        &restore,
        "Push ~/.config/legion-control/settings.json to the Spectrum hardware again",
    );
    let toast_r = toast.clone();
    restore.connect_clicked(move |_| {
        legion_core::keyboard::restore_lighting_async();
        let t = adw::Toast::new("Restored saved lighting");
        t.set_timeout(2);
        toast_r.add_toast(t);
    });
    restore_card.append(&labeled_row_tip(
        "Hardware sync",
        "~/.config/legion-control/settings.json",
        &restore,
        Some("Useful after sleep or if lights drifted from the saved look"),
    ));
    box_.append(&sec_r);

    box_
}

fn zone_editor(
    zone: RgbZone,
    layer: &ZoneEffect,
    compact: bool,
    toast: &adw::ToastOverlay,
) -> gtk::Box {
    let toast = toast.clone();
    let wrap = gtk::Box::new(Orientation::Vertical, 0);
    let color = Rc::new(Cell::new((layer.r, layer.g, layer.b)));
    let speed = Rc::new(Cell::new(layer.speed.clamp(1, 3)));
    let brightness = Rc::new(Cell::new(layer.brightness.min(9)));

    let effect_dd = gtk::DropDown::from_strings(EFFECT_LABELS);
    let fx_sel = EFFECT_IDS
        .iter()
        .position(|e| *e == layer.effect)
        .unwrap_or(0) as u32;
    effect_dd.set_selected(fx_sel);
    tip(
        &effect_dd,
        effect_tooltip(EFFECT_IDS.get(fx_sel as usize).copied().unwrap_or("static")),
    );
    effect_dd.connect_selected_notify(move |d| {
        let idx = d.selected() as usize;
        let id = EFFECT_IDS.get(idx).copied().unwrap_or("static");
        tip(d, effect_tooltip(id));
    });

    let speed_dd = gtk::DropDown::from_strings(&["Slow", "Normal", "Fast"]);
    tip(
        &speed_dd,
        "How fast animated effects move — ignored for Static and Off",
    );
    speed_dd.set_selected((speed.get().saturating_sub(1) as u32).min(2));
    let speed_c = speed.clone();
    speed_dd.connect_selected_notify(move |d| {
        speed_c.set((d.selected() as u8).saturating_add(1).min(3));
    });

    let picker = colour_picker(color.get(), {
        let color = color.clone();
        move |r, g, b| color.set((r, g, b))
    });
    tip(
        &picker,
        "Colour for Static / Pulse / Wave and other coloured effects — click to open the colour dialog",
    );

    if compact {
        let row = gtk::Box::new(Orientation::Horizontal, 12);
        row.set_valign(Align::Center);
        effect_dd.set_hexpand(true);
        row.append(&effect_dd);
        row.append(&speed_dd);
        row.append(&picker);
        row.append(&apply_button(
            &effect_dd,
            &color,
            &speed,
            &brightness,
            zone,
            &toast,
        ));
        wrap.append(&row);
    } else {
        let hex = gtk::Entry::builder()
            .placeholder_text("#RRGGBB")
            .width_chars(9)
            .max_length(7)
            .build();
        tip(
            &hex,
            "Type a hex colour like #C8102E and press Return — same as the colour picker",
        );
        let (r, g, b) = color.get();
        hex.set_text(&format!("#{r:02X}{g:02X}{b:02X}"));
        let color_h = color.clone();
        let picker_h = picker.clone();
        hex.connect_activate(move |e| {
            if let Some((r, g, b)) = parse_hex(&e.text()) {
                color_h.set((r, g, b));
                set_picker_rgba(&picker_h, r, g, b);
            }
        });
        let hex_s = hex.clone();
        let color_s = color.clone();
        picker.connect_rgba_notify(move |p| {
            let (r, g, b) = rgba_to_rgb(p.rgba());
            color_s.set((r, g, b));
            hex_s.set_text(&format!("#{r:02X}{g:02X}{b:02X}"));
        });

        let fx_box = gtk::Box::new(Orientation::Horizontal, 8);
        fx_box.set_valign(Align::Center);
        effect_dd.set_hexpand(true);
        fx_box.append(&effect_dd);
        fx_box.append(&speed_dd);
        wrap.append(&labeled_row_tip(
            "Effect",
            "Animation and playback speed",
            &fx_box,
            Some("Lighting animation for this surface — speed only affects animated effects"),
        ));

        let colour_box = gtk::Box::new(Orientation::Horizontal, 8);
        colour_box.set_valign(Align::Center);
        colour_box.append(&picker);
        colour_box.append(&hex);
        wrap.append(&labeled_row_tip(
            "Colour",
            "Picker, or type #RRGGBB and press Return",
            &colour_box,
            Some("Used by Static, Pulse, Wave, and other coloured effects"),
        ));

        let bright_slider = gtk::Scale::with_range(Orientation::Horizontal, 0.0, 9.0, 1.0);
        bright_slider.set_value(brightness.get() as f64);
        bright_slider.set_draw_value(true);
        bright_slider.set_digits(0);
        bright_slider.set_hexpand(true);
        bright_slider.set_width_request(220);
        bright_slider.add_css_class("brightness-slider");
        let bri_c = brightness.clone();
        bright_slider.connect_value_changed(move |s| {
            bri_c.set(s.value().round().clamp(0.0, 9.0) as u8);
        });
        tip(
            &bright_slider,
            "Brightness 0–9 for this zone · 0 turns it off",
        );
        wrap.append(&labeled_row_tip(
            "Brightness",
            "0 off · 9 full",
            &bright_slider,
            Some("Per-zone brightness — independent of other zones"),
        ));

        let apply = apply_button(&effect_dd, &color, &speed, &brightness, zone, &toast);
        apply.set_halign(Align::End);
        apply.set_margin_top(10);
        wrap.append(&apply);
    }

    wrap
}

fn colour_toolbar(brush: &Rc<Cell<(u8, u8, u8)>>) -> gtk::Box {
    let row = gtk::Box::new(Orientation::Horizontal, 12);
    row.set_valign(Align::Center);

    let picker = colour_picker(brush.get(), {
        let brush = brush.clone();
        move |r, g, b| {
            brush.set((r, g, b));
            legion_core::config::set_ui_color(r, g, b);
        }
    });

    let text = gtk::Box::new(Orientation::Vertical, 0);
    text.set_hexpand(true);
    let t = gtk::Label::new(Some("Brush"));
    t.add_css_class("row-title");
    t.set_halign(Align::Start);
    tip(
        &t,
        "Active paint colour for per-key mode — also used by coloured lighting effects",
    );
    let s = gtk::Label::new(Some("Any RGB — dialog or hex"));
    s.add_css_class("row-sub");
    s.set_halign(Align::Start);
    tip(
        &s,
        "Use the colour dialog, a preset swatch, or type #RRGGBB and press Return",
    );
    text.append(&t);
    text.append(&s);
    tip(
        &row,
        "Choose the colour you paint keys with — saved as your UI brush colour",
    );
    row.append(&text);
    row.append(&picker);
    tip(
        &picker,
        "Brush colour for per-key painting — also used when you Apply a coloured effect",
    );

    let hex = gtk::Entry::builder()
        .placeholder_text("#RRGGBB")
        .width_chars(9)
        .max_length(7)
        .build();
    tip(&hex, "Type a hex colour like #C8102E and press Return");
    let (r, g, b) = brush.get();
    hex.set_text(&format!("#{r:02X}{g:02X}{b:02X}"));
    let brush_h = brush.clone();
    let picker_h = picker.clone();
    hex.connect_activate(move |e| {
        if let Some((r, g, b)) = parse_hex(&e.text()) {
            brush_h.set((r, g, b));
            legion_core::config::set_ui_color(r, g, b);
            set_picker_rgba(&picker_h, r, g, b);
        }
    });
    row.append(&hex);

    let hex_s = hex.clone();
    picker.connect_rgba_notify(move |p| {
        let (r, g, b) = rgba_to_rgb(p.rgba());
        hex_s.set_text(&format!("#{r:02X}{g:02X}{b:02X}"));
    });

    for (name, r, g, b) in [
        ("Red", 200u8, 16u8, 46u8),
        ("White", 255, 255, 255),
        ("Ice", 120, 220, 255),
        ("Off", 0, 0, 0),
    ] {
        let btn = gtk::Button::new();
        btn.add_css_class("swatch");
        btn.add_css_class(&format!("swatch-{:02x}{:02x}{:02x}", r, g, b));
        tip(
            &btn,
            &format!("Preset “{name}” (#{r:02X}{g:02X}{b:02X}) — click to set brush colour"),
        );
        let brush_p = brush.clone();
        let picker_p = picker.clone();
        let hex_p = hex.clone();
        btn.connect_clicked(move |_| {
            brush_p.set((r, g, b));
            legion_core::config::set_ui_color(r, g, b);
            set_picker_rgba(&picker_p, r, g, b);
            hex_p.set_text(&format!("#{r:02X}{g:02X}{b:02X}"));
        });
        row.append(&btn);
    }

    row
}

fn colour_picker(
    initial: (u8, u8, u8),
    on_change: impl Fn(u8, u8, u8) + 'static,
) -> gtk::ColorDialogButton {
    let dialog = gtk::ColorDialog::new();
    dialog.set_with_alpha(false);
    let picker = gtk::ColorDialogButton::new(Some(dialog));
    picker.add_css_class("colour-picker");
    set_picker_rgba(&picker, initial.0, initial.1, initial.2);
    picker.connect_rgba_notify(move |p| {
        let (r, g, b) = rgba_to_rgb(p.rgba());
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

fn apply_button(
    effect_dd: &gtk::DropDown,
    color: &Rc<Cell<(u8, u8, u8)>>,
    speed: &Rc<Cell<u8>>,
    brightness: &Rc<Cell<u8>>,
    zone: RgbZone,
    toast: &adw::ToastOverlay,
) -> gtk::Button {
    let apply = gtk::Button::with_label("Apply");
    apply.add_css_class("suggested-action");
    tip(
        &apply,
        "Send this effect, colour, speed, and brightness to the Spectrum controller now",
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
        let t = adw::Toast::new(&format!("{label} → {}", zone.display_name()));
        t.set_timeout(2);
        toast_a.add_toast(t);
    });
    apply
}

fn parse_hex(s: &str) -> Option<(u8, u8, u8)> {
    let t = s.trim().trim_start_matches('#');
    // Byte-length checks don't imply char boundaries: reject non-ASCII before
    // slicing so multibyte input (e.g. "äää", exactly 6 bytes) can't panic.
    if !t.is_ascii() {
        return None;
    }
    if t.len() != 6 {
        return None;
    }
    Some((
        u8::from_str_radix(&t[0..2], 16).ok()?,
        u8::from_str_radix(&t[2..4], 16).ok()?,
        u8::from_str_radix(&t[4..6], 16).ok()?,
    ))
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
    fn parse_hex_table() {
        #[allow(clippy::type_complexity)]
        let cases: &[(&str, Option<(u8, u8, u8)>)] = &[
            ("#FF8000", Some((255, 128, 0))),
            ("ff8000", Some((255, 128, 0))),
            ("  #ff8000 ", Some((255, 128, 0))), // surrounding whitespace trimmed
            ("", None),                          // empty
            ("#", None),                         // hash only
            ("FF80", None),                      // too short
            ("FF80000", None),                   // too long
            ("##FF8000", Some((255, 128, 0))),   // all leading hashes trimmed
            ("GG8000", None),                    // non-hex first pair
            ("FF80G0", None),                    // non-hex last pair
            ("€€€", None),                       // multibyte UTF-8 — must not panic
            ("äää", None), // multibyte UTF-8, exactly 6 bytes — must not panic
        ];
        for (input, expected) in cases {
            assert_eq!(parse_hex(input), *expected, "input {input:?}");
        }
    }

    #[test]
    fn parse_hex_multibyte_never_panics() {
        // Regression: any non-ASCII input must return None instead of slicing
        // through a multi-byte char boundary.
        for input in ["äää", "€€€", "🦀🦀", "#äää", "FF€€00"] {
            assert!(parse_hex(input).is_none(), "input {input:?}");
        }
    }
}
