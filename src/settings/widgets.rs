//! Small helpers for Legion Settings.

use adw::prelude::*;
use gtk::{glib, Align, Orientation};
use gtk4 as gtk;
use libadwaita as adw;
use std::cell::RefCell;
use std::rc::Rc;

pub fn friendly_profile(name: &str) -> String {
    match name {
        "low-power" | "quiet" => "Quiet".into(),
        "balanced" => "Balanced".into(),
        "performance" => "Performance".into(),
        "max-power" => "Max Power".into(),
        "custom" => "Custom".into(),
        other => other.to_string(),
    }
}

/// One-line help under the profile picker.
pub fn profile_blurb(name: &str) -> &'static str {
    match name {
        "low-power" | "quiet" => "Lower power and cooler fans",
        "balanced" => "Mixed speed and noise",
        "performance" => "Higher power for heavy loads",
        "max-power" => "Maximum firmware power",
        "custom" => "Manual CPU and GPU limits",
        _ => "CPU and GPU power mode",
    }
}

/// Page width variant for the clamp.
#[allow(dead_code)]
pub enum PageWidth {
    Standard, // ~760 px for settings forms
    Wide,     // 1120 px for dashboards and lighting
}

pub fn page_shell(body: &impl glib::object::IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    page_shell_width(body, PageWidth::Standard)
}

pub fn page_shell_width(
    body: &impl glib::object::IsA<gtk::Widget>,
    width: PageWidth,
) -> gtk::ScrolledWindow {
    use gtk::PolicyType;
    let max = match width {
        PageWidth::Standard => 760,
        PageWidth::Wide => 1120,
    };
    let clamp = libadwaita::Clamp::builder()
        .maximum_size(max)
        .tightening_threshold((max as f64 * 0.7) as i32)
        .build();
    clamp.set_child(Some(body));

    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .propagate_natural_height(true)
        .child(&clamp)
        .build()
}

pub fn page_box(title: &str, subtitle: &str) -> gtk::Box {
    let page = gtk::Box::new(Orientation::Vertical, 18);
    page.add_css_class("page");
    page.set_vexpand(true);

    if !title.is_empty() {
        let t = gtk::Label::new(Some(title));
        t.add_css_class("page-title");
        t.set_halign(Align::Start);
        page.append(&t);
    }
    if !subtitle.is_empty() {
        let s = gtk::Label::new(Some(subtitle));
        s.add_css_class("page-sub");
        s.set_halign(Align::Start);
        s.set_wrap(true);
        if title.is_empty() {
            s.add_css_class("page-sub-solo");
            s.add_css_class("page-lede");
        }
        page.append(&s);
    }
    page
}

/// Page body with lede only — header bar already shows the page title.
pub fn page_lede(subtitle: &str) -> gtk::Box {
    page_box("", subtitle)
}

/// Mark a primary button busy (spinner + operation label).
pub fn set_busy(btn: &gtk::Button, busy: bool, idle_label: &str) {
    if busy {
        btn.set_sensitive(false);
        let row = gtk::Box::new(Orientation::Horizontal, 8);
        row.set_halign(Align::Center);
        let spin = gtk::Spinner::new();
        spin.set_spinning(true);
        row.append(&spin);
        let l = gtk::Label::new(Some(idle_label));
        row.append(&l);
        btn.set_child(Some(&row));
    } else {
        btn.set_sensitive(true);
        btn.set_label(idle_label);
    }
}

/// Adwaita preferences group (boxed list + title/description).
pub fn pref_group(title: &str, description: Option<&str>) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    if !title.is_empty() {
        g.set_title(title);
    }
    if let Some(d) = description {
        if !d.is_empty() {
            g.set_description(Some(d));
        }
    }
    g
}

/// Read-only property-style ActionRow (title + value subtitle).
pub fn property_row(title: &str, value: &str, tooltip: Option<&str>) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(value)
        .build();
    tip(&row, tooltip.unwrap_or(""));
    row.add_css_class("property");
    row.set_subtitle_selectable(true);
    row.set_activatable(false);
    row
}

/// ComboRow backed by a string list (Adwaita boxed-list pattern).
pub fn string_combo_row(
    title: &str,
    subtitle: &str,
    labels: &[&str],
    selected: u32,
) -> adw::ComboRow {
    let model = gtk::StringList::new(labels);
    let row = adw::ComboRow::builder()
        .title(title)
        .subtitle(subtitle)
        .model(&model)
        .selected(selected)
        .build();
    let expr = gtk::PropertyExpression::new(
        gtk::StringObject::static_type(),
        Option::<gtk::Expression>::None,
        "string",
    );
    row.set_expression(Some(expr));
    row
}

pub fn section_tip(title: &str, blurb: Option<&str>) -> (gtk::Box, gtk::Box) {
    let wrap = gtk::Box::new(Orientation::Vertical, 0);
    wrap.add_css_class("section");
    let head = gtk::Box::new(Orientation::Vertical, 0);
    head.add_css_class("section-head");
    let label = gtk::Label::new(Some(title));
    label.add_css_class("section-label");
    label.set_halign(Align::Start);
    head.append(&label);
    if let Some(b) = blurb {
        if !b.is_empty() {
            let sub = gtk::Label::new(Some(b));
            sub.add_css_class("section-sub");
            sub.set_halign(Align::Start);
            sub.set_wrap(true);
            head.append(&sub);
        }
    }
    wrap.append(&head);
    let card = gtk::Box::new(Orientation::Vertical, 0);
    card.add_css_class("card");
    wrap.append(&card);
    (wrap, card)
}

/// Hover text for Spectrum lighting effects.
pub fn effect_tooltip(id: &str) -> &'static str {
    match id {
        "static" => "Solid colour on this surface",
        "color-pulse" => "Fades your colour in and out",
        "color-wave" => "Your colour sweeps across the keys or bar",
        "rainbow-wave" => "Rainbow colours move across the surface",
        "screw-rainbow" => "Spiral rainbow pattern",
        "smooth" => "Soft colour blend animation",
        "color-change" => "Cycles through colours smoothly",
        "rain" => "Raindrop-style sparkles",
        "ripple" => "Ripple rings from the centre",
        "reactive" => "Lights react when you press keys",
        "off" => "Turns this surface's lighting off",
        _ => "Lighting effect for this surface",
    }
}

/// Longer hover tip for each platform profile.
pub fn profile_tooltip(name: &str) -> &'static str {
    match name {
        "low-power" | "quiet" => "Quiet (blue LED): lower power and cooler fans",
        "balanced" => "Balanced (white LED): default mixed mode",
        "performance" => "Performance (red LED): higher CPU and GPU power",
        "max-power" => "Max Power (purple LED): highest firmware limits",
        "custom" => "Custom (purple LED): manual CPU and GPU limits",
        _ => "Changes how hard the laptop pushes CPU and GPU",
    }
}

pub fn tip(widget: &impl glib::object::IsA<gtk::Widget>, text: &str) {
    let w = widget.as_ref();
    if text.is_empty() {
        w.set_tooltip_text(None);
    } else {
        w.set_has_tooltip(true);
        w.set_tooltip_text(Some(text));
    }
}

pub fn labeled_row_tip(
    title: &str,
    subtitle: &str,
    suffix: &impl glib::object::IsA<gtk::Widget>,
    tooltip: Option<&str>,
) -> gtk::Box {
    let row = gtk::Box::new(Orientation::Vertical, 8);
    row.add_css_class("row");
    row.set_hexpand(true);
    tip(&row, tooltip.unwrap_or(""));

    let top = gtk::Box::new(Orientation::Horizontal, 12);
    top.set_hexpand(true);
    top.set_valign(Align::Center);

    let text = gtk::Box::new(Orientation::Vertical, 0);
    text.set_hexpand(true);
    text.set_valign(Align::Center);
    let t = gtk::Label::new(Some(title));
    t.add_css_class("row-title");
    t.set_halign(Align::Start);
    text.append(&t);
    if !subtitle.is_empty() {
        let s = gtk::Label::new(Some(subtitle));
        s.add_css_class("row-sub");
        s.set_halign(Align::Start);
        s.set_wrap(true);
        text.append(&s);
    }
    top.append(&text);

    if suffix.as_ref().is::<gtk::Scale>() {
        let wrap = gtk::Box::new(Orientation::Vertical, 0);
        wrap.set_hexpand(true);
        let w = suffix.clone().upcast::<gtk::Widget>();
        w.set_hexpand(true);
        w.set_halign(Align::Fill);
        wrap.append(&w);
        let marks = gtk::Label::new(Some("0  ·  3  ·  6  ·  9"));
        marks.add_css_class("scale-marks");
        marks.set_halign(Align::Fill);
        marks.set_xalign(0.5);
        wrap.append(&marks);
        row.append(&top);
        row.append(&wrap);
    } else {
        let sfx = suffix.clone().upcast::<gtk::Widget>();
        sfx.set_valign(Align::Center);
        sfx.set_halign(Align::End);
        top.append(&sfx);
        row.append(&top);
    }
    row
}

pub fn metric_chip_tip(title: &str, tooltip: Option<&str>) -> (gtk::Box, gtk::Label, gtk::Label) {
    let box_ = gtk::Box::new(Orientation::Vertical, 0);
    box_.add_css_class("metric-chip");
    box_.set_hexpand(true);
    tip(&box_, tooltip.unwrap_or(""));
    let l = gtk::Label::new(Some(title));
    l.add_css_class("label");
    l.set_halign(Align::Start);
    let v = gtk::Label::new(Some("—"));
    v.add_css_class("value");
    v.add_css_class("numeric");
    v.set_halign(Align::Start);
    let d = gtk::Label::new(Some(""));
    d.add_css_class("detail");
    d.set_halign(Align::Start);
    box_.append(&l);
    box_.append(&v);
    box_.append(&d);
    (box_, v, d)
}

pub fn status_pill_tip(text: &str, kind: &str, tooltip: Option<&str>) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("status-pill");
    l.add_css_class(match kind {
        "ok" => "status-ok",
        "warn" => "status-warn",
        "bad" => "status-bad",
        _ => "status-muted",
    });
    l.set_halign(Align::Center);
    l.set_valign(Align::Center);
    l.set_justify(gtk::Justification::Center);
    l.set_xalign(0.5);
    l.set_yalign(0.5);
    if let Some(tt) = tooltip {
        tip(&l, tt);
    }
    l
}

/// Primary action button with consistent sizing.
pub fn primary_button_tip(label: &str, tooltip: Option<&str>) -> gtk::Button {
    let btn = gtk::Button::with_label(label);
    btn.add_css_class("suggested-action");
    btn.add_css_class("pill-btn");
    btn.set_halign(Align::Start);
    if let Some(tt) = tooltip {
        tip(&btn, tt);
    }
    btn
}

/// Widgets that need the root legion-control service — greyed out while offline.
#[derive(Clone, Debug)]
pub struct DaemonGate {
    widgets: Rc<RefCell<Vec<gtk::Widget>>>,
}

impl DaemonGate {
    pub fn new() -> Self {
        Self {
            widgets: Rc::new(RefCell::new(Vec::new())),
        }
    }

    pub fn track(&self, w: &impl glib::object::IsA<gtk::Widget>) {
        self.widgets
            .borrow_mut()
            .push(w.clone().upcast::<gtk::Widget>());
    }

    pub fn set_online(&self, online: bool) {
        for w in self.widgets.borrow().iter() {
            w.set_sensitive(online);
        }
    }
}
