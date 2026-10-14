//! Fan-curve editor: a `gtk::DrawingArea` with 8 draggable points.
//!
//! Pointer: drag a point. Keyboard (the area is focusable): Tab / Shift+Tab
//! or Home / End select a point, arrow keys move it (Left/Right ±1 °C,
//! Up/Down ±1 %, hold Shift for ±5). Every edit goes through
//! `fan_curve::constrain_point`, so the curve is always valid.

use gtk4 as gtk;
use gtk4::prelude::*;
use gtk4::{gdk, glib};
use legion_core::fan_curve::{self, CurvePoints, POINTS};
use std::cell::RefCell;
use std::rc::Rc;

const AXIS_T_MIN: f64 = 30.0;
const AXIS_T_MAX: f64 = 100.0;
const MARGIN_L: f64 = 44.0;
const MARGIN_R: f64 = 22.0;
const MARGIN_T: f64 = 10.0;
const MARGIN_B: f64 = 30.0;
const HIT_RADIUS: f64 = 16.0;
const ACCENT: (f64, f64, f64) = (0.784, 0.063, 0.180);
const NOW: (f64, f64, f64) = (0.878, 0.537, 0.169);

type ChangeSlot = Rc<RefCell<Option<Rc<dyn Fn()>>>>;

struct State {
    points: CurvePoints,
    focused: usize,
    now_c: Option<f32>,
    /// (point index, pixel x, pixel y at drag start)
    drag: Option<(usize, f64, f64)>,
}

#[derive(Clone)]
pub(crate) struct CurveEditor {
    pub area: gtk::DrawingArea,
    state: Rc<RefCell<State>>,
    on_change: ChangeSlot,
}

#[derive(Clone, Copy)]
struct Geo {
    l: f64,
    t: f64,
    w: f64,
    h: f64,
}

impl Geo {
    fn new(width: i32, height: i32) -> Self {
        Geo {
            l: MARGIN_L,
            t: MARGIN_T,
            w: (width as f64 - MARGIN_L - MARGIN_R).max(10.0),
            h: (height as f64 - MARGIN_T - MARGIN_B).max(10.0),
        }
    }
    fn x(&self, temp: f64) -> f64 {
        self.l + (temp.clamp(AXIS_T_MIN, AXIS_T_MAX) - AXIS_T_MIN) / (AXIS_T_MAX - AXIS_T_MIN) * self.w
    }
    fn y(&self, pct: f64) -> f64 {
        self.t + self.h - pct.clamp(0.0, 100.0) / 100.0 * self.h
    }
    fn temp(&self, x: f64) -> f64 {
        AXIS_T_MIN + (x - self.l) / self.w * (AXIS_T_MAX - AXIS_T_MIN)
    }
    fn pct(&self, y: f64) -> f64 {
        (self.t + self.h - y) / self.h * 100.0
    }
}

impl CurveEditor {
    pub fn new() -> Self {
        let area = gtk::DrawingArea::new();
        area.set_content_height(190);
        area.set_content_width(420);
        area.set_hexpand(true);
        area.set_focusable(true);
        area.set_can_focus(true);
        area.set_tooltip_text(Some(
            "Drag the points, or focus the chart and use Tab to pick a point and the arrow keys to move it (Shift = bigger steps)",
        ));
        let state = Rc::new(RefCell::new(State {
            points: fan_curve::default_points("balanced"),
            focused: 0,
            now_c: None,
            drag: None,
        }));
        let ed = CurveEditor {
            area,
            state,
            on_change: Rc::new(RefCell::new(None)),
        };
        ed.install_draw();
        ed.install_pointer();
        ed.install_keys();
        ed.update_accessible();
        ed
    }

    pub fn points(&self) -> CurvePoints {
        self.state.borrow().points
    }

    /// Replace the curve (does not fire the change callback).
    pub fn set_points(&self, p: CurvePoints) {
        {
            let mut st = self.state.borrow_mut();
            if st.points == p {
                return;
            }
            st.points = p;
        }
        self.update_accessible();
        self.area.queue_draw();
    }

    pub fn set_now(&self, temp_c: Option<f32>) {
        self.state.borrow_mut().now_c = temp_c;
        self.area.queue_draw();
    }

    pub fn is_dragging(&self) -> bool {
        self.state.borrow().drag.is_some()
    }

    pub fn connect_changed(&self, f: impl Fn() + 'static) {
        *self.on_change.borrow_mut() = Some(Rc::new(f));
    }

    fn fire_changed(&self) {
        self.update_accessible();
        self.area.queue_draw();
        let cb = self.on_change.borrow().clone();
        if let Some(cb) = cb {
            cb();
        }
    }

    fn move_focused(&self, idx: usize, temp: i32, pct: i32) {
        {
            let mut st = self.state.borrow_mut();
            let np = fan_curve::constrain_point(&st.points, idx, temp, pct);
            if st.points[idx] == np {
                return;
            }
            st.points[idx] = np;
        }
        self.fire_changed();
    }

    fn update_accessible(&self) {
        let st = self.state.borrow();
        let desc: Vec<String> = st
            .points
            .iter()
            .map(|(t, p)| format!("{t} °C {p} %"))
            .collect();
        let sel = st.points[st.focused];
        let text = format!(
            "Point {} of {POINTS} selected: {} °C, {} %. Curve: {}",
            st.focused + 1,
            sel.0,
            sel.1,
            desc.join(", ")
        );
        drop(st);
        self.area.update_property(&[
            gtk::accessible::Property::Label("Fan curve editor"),
            gtk::accessible::Property::Description(&text),
        ]);
    }

    fn install_draw(&self) {
        let state = self.state.clone();
        self.area.set_draw_func(move |area, cr, w, h| {
            let st = state.borrow();
            let g = Geo::new(w, h);
            let fg = area.color();
            let (fr, fgc, fb) = (fg.red() as f64, fg.green() as f64, fg.blue() as f64);
            let light_text = (fr + fgc + fb) / 3.0 > 0.5; // dark theme
            let pad = if light_text { 0.18 } else { 1.0 };

            // Plot background.
            cr.set_source_rgba(fr, fgc, fb, 0.04);
            cr.rectangle(g.l, g.t, g.w, g.h);
            let _ = cr.fill();

            // Safety zone (≥ 90 °C).
            cr.set_source_rgba(ACCENT.0, ACCENT.1, ACCENT.2, 0.09);
            cr.rectangle(g.x(90.0), g.t, g.x(100.0) - g.x(90.0), g.h);
            let _ = cr.fill();

            cr.set_font_size(11.0);
            // Grid + axis labels.
            cr.set_line_width(1.0);
            for pct in [0.0, 25.0, 50.0, 75.0, 100.0] {
                cr.set_source_rgba(fr, fgc, fb, if pct == 0.0 { 0.35 } else { 0.12 });
                cr.move_to(g.l, g.y(pct));
                cr.line_to(g.l + g.w, g.y(pct));
                let _ = cr.stroke();
                cr.set_source_rgba(fr, fgc, fb, 0.65);
                cr.move_to(6.0, g.y(pct) + 4.0);
                let _ = cr.show_text(&format!("{pct:.0}%"));
            }
            let mut tc = AXIS_T_MIN;
            while tc <= AXIS_T_MAX {
                cr.set_source_rgba(fr, fgc, fb, 0.08);
                cr.move_to(g.x(tc), g.t);
                cr.line_to(g.x(tc), g.t + g.h);
                let _ = cr.stroke();
                cr.set_source_rgba(fr, fgc, fb, 0.65);
                cr.move_to(g.x(tc) - 14.0, g.t + g.h + 16.0);
                let _ = cr.show_text(&format!("{tc:.0}°C"));
                tc += 10.0;
            }
            cr.set_source_rgba(ACCENT.0, ACCENT.1, ACCENT.2, 0.95);
            cr.move_to(g.x(90.0) + 4.0, g.t + g.h - 6.0);
            let _ = cr.show_text("safety max");

            // Curve polyline (flat extension to both axis edges).
            cr.set_source_rgb(ACCENT.0, ACCENT.1, ACCENT.2);
            cr.set_line_width(2.5);
            cr.set_line_join(gtk::cairo::LineJoin::Round);
            let first = st.points[0];
            cr.move_to(g.x(AXIS_T_MIN), g.y(first.1 as f64));
            for &(t, p) in st.points.iter() {
                cr.line_to(g.x(t as f64), g.y(p as f64));
            }
            let last = st.points[POINTS - 1];
            cr.line_to(g.x(AXIS_T_MAX), g.y(last.1 as f64));
            let _ = cr.stroke();

            // "Now" marker.
            if let Some(now) = st.now_c {
                let nx = g.x(now as f64);
                cr.set_source_rgb(NOW.0, NOW.1, NOW.2);
                cr.set_line_width(2.0);
                cr.set_dash(&[5.0, 5.0], 0.0);
                cr.move_to(nx, g.t);
                cr.line_to(nx, g.t + g.h);
                let _ = cr.stroke();
                cr.set_dash(&[], 0.0);
                let label = format!("now {now:.0}°C");
                let ext = cr.text_extents(&label).map(|e| e.width()).unwrap_or(50.0);
                let lx = if nx + 6.0 + ext > g.l + g.w { nx - 6.0 - ext } else { nx + 6.0 };
                cr.move_to(lx, g.t + 13.0);
                let _ = cr.show_text(&label);
            }

            // Points.
            let has_focus = area.has_focus();
            for (i, &(t, p)) in st.points.iter().enumerate() {
                let (x, y) = (g.x(t as f64), g.y(p as f64));
                let selected = i == st.focused;
                cr.arc(x, y, 6.0, 0.0, std::f64::consts::TAU);
                if selected {
                    cr.set_source_rgb(ACCENT.0, ACCENT.1, ACCENT.2);
                } else {
                    cr.set_source_rgb(pad, pad, pad);
                }
                let _ = cr.fill_preserve();
                cr.set_source_rgb(ACCENT.0, ACCENT.1, ACCENT.2);
                cr.set_line_width(3.0);
                let _ = cr.stroke();
                if selected && has_focus {
                    cr.arc(x, y, 12.0, 0.0, std::f64::consts::TAU);
                    cr.set_source_rgba(fr, fgc, fb, 0.85);
                    cr.set_line_width(2.0);
                    let _ = cr.stroke();
                }
            }
            // Readout of the selected point.
            let (st_t, st_p) = st.points[st.focused];
            let readout = format!("{st_t} °C · {st_p} %");
            let ext = cr.text_extents(&readout).map(|e| e.width()).unwrap_or(70.0);
            let sx = g.x(st_t as f64);
            let sy = g.y(st_p as f64);
            let rx = (sx - ext / 2.0).clamp(g.l, g.l + g.w - ext);
            let ry = if sy < g.t + 40.0 { sy + 26.0 } else { sy - 16.0 };
            cr.set_source_rgba(fr, fgc, fb, 0.9);
            cr.move_to(rx, ry);
            let _ = cr.show_text(&readout);
        });
    }

    fn hit(&self, x: f64, y: f64) -> Option<usize> {
        let g = Geo::new(self.area.width(), self.area.height());
        let st = self.state.borrow();
        let mut best: Option<(usize, f64)> = None;
        for (i, &(t, p)) in st.points.iter().enumerate() {
            let d = ((g.x(t as f64) - x).powi(2) + (g.y(p as f64) - y).powi(2)).sqrt();
            if d <= HIT_RADIUS && best.is_none_or(|(_, bd)| d < bd) {
                best = Some((i, d));
            }
        }
        best.map(|(i, _)| i)
    }

    fn install_pointer(&self) {
        let drag = gtk::GestureDrag::new();
        drag.set_button(gdk::BUTTON_PRIMARY);
        let ed = self.clone();
        drag.connect_drag_begin(move |_, x, y| {
            ed.area.grab_focus();
            if let Some(i) = ed.hit(x, y) {
                let g = Geo::new(ed.area.width(), ed.area.height());
                let (t, p) = ed.state.borrow().points[i];
                {
                    let mut st = ed.state.borrow_mut();
                    st.focused = i;
                    st.drag = Some((i, g.x(t as f64), g.y(p as f64)));
                }
                ed.update_accessible();
                ed.area.queue_draw();
            }
        });
        let ed = self.clone();
        drag.connect_drag_update(move |_, dx, dy| {
            let d = ed.state.borrow().drag;
            if let Some((i, sx, sy)) = d {
                let g = Geo::new(ed.area.width(), ed.area.height());
                ed.move_focused(
                    i,
                    g.temp(sx + dx).round() as i32,
                    g.pct(sy + dy).round() as i32,
                );
            }
        });
        let ed = self.clone();
        drag.connect_drag_end(move |_, _, _| {
            ed.state.borrow_mut().drag = None;
            ed.area.queue_draw();
        });
        self.area.add_controller(drag);

        let focus = gtk::EventControllerFocus::new();
        let area = self.area.clone();
        focus.connect_enter(glib::clone!(
            #[weak]
            area,
            move |_| area.queue_draw()
        ));
        let area = self.area.clone();
        focus.connect_leave(glib::clone!(
            #[weak]
            area,
            move |_| area.queue_draw()
        ));
        self.area.add_controller(focus);
    }

    fn install_keys(&self) {
        let keys = gtk::EventControllerKey::new();
        let ed = self.clone();
        keys.connect_key_pressed(move |_, key, _, mods| {
            let step = if mods.contains(gdk::ModifierType::SHIFT_MASK) { 5 } else { 1 };
            let (idx, (t, p)) = {
                let st = ed.state.borrow();
                (st.focused, st.points[st.focused])
            };
            let (t, p) = (t as i32, p as i32);
            let select = |ed: &CurveEditor, i: usize| {
                ed.state.borrow_mut().focused = i;
                ed.update_accessible();
                ed.area.queue_draw();
            };
            match key {
                gdk::Key::Left => ed.move_focused(idx, t - step, p),
                gdk::Key::Right => ed.move_focused(idx, t + step, p),
                gdk::Key::Up => ed.move_focused(idx, t, p + step),
                gdk::Key::Down => ed.move_focused(idx, t, p - step),
                gdk::Key::Home => select(&ed, 0),
                gdk::Key::End => select(&ed, POINTS - 1),
                gdk::Key::Tab | gdk::Key::ISO_Left_Tab => {
                    let back = key == gdk::Key::ISO_Left_Tab
                        || mods.contains(gdk::ModifierType::SHIFT_MASK);
                    if back && idx > 0 {
                        select(&ed, idx - 1);
                    } else if !back && idx + 1 < POINTS {
                        select(&ed, idx + 1);
                    } else {
                        // Ends of the chart: let focus leave the widget.
                        return glib::Propagation::Proceed;
                    }
                }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        self.area.add_controller(keys);
    }
}
