//! Temperature history chart for the Home page.
//!
//! The service keeps one sample every 5 s for the last hour, so the chart
//! is full as soon as it opens, even after the window was closed. A toggle
//! switches between the last 5 minutes and the last hour; thin markers show
//! where the power mode changed. With an older service (no history over
//! IPC) it falls back to the 2 s samples this window collected itself.
//!
//! `LEGION_HISTORY_DEMO=5m|1h` fills the chart with generated samples and
//! opens it, for screenshots of a fresh instance.

use super::*;
use legion_core::comms::{mode_code, mode_name, TempHistory, TempSample};
use std::collections::VecDeque;

const MIN_T: f64 = 30.0;
const MAX_T: f64 = 100.0;
const MARGIN_L: f64 = 44.0;
const MARGIN_R: f64 = 6.0;
const MARGIN_T: f64 = 20.0;
const MARGIN_B: f64 = 18.0;
/// Window-local fallback: 2 s samples for one hour.
const LOCAL_INTERVAL_S: u16 = 2;
const LOCAL_CAP: usize = 1800;
const SPANS: [u32; 2] = [300, 3600];
const REFRESH: Duration = Duration::from_secs(5);

struct State {
    service: Option<TempHistory>,
    local: VecDeque<TempSample>,
    span_s: u32,
    fetching: bool,
    demo: bool,
}

impl State {
    /// Samples to draw, oldest first, and their spacing in seconds.
    fn series(&self) -> (f64, Vec<TempSample>) {
        match &self.service {
            Some(h) if !h.samples.is_empty() => (h.interval_s.max(1) as f64, h.samples.clone()),
            _ => (LOCAL_INTERVAL_S as f64, self.local.iter().copied().collect()),
        }
    }
}

/// Indices where the power mode differs from the previous known sample.
pub(crate) fn mode_changes(samples: &[TempSample]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut prev = 0u8;
    for (i, s) in samples.iter().enumerate() {
        if s.mode == 0 {
            continue;
        }
        if prev != 0 && s.mode != prev {
            out.push(i);
        }
        prev = s.mode;
    }
    out
}

/// Deterministic sample generator: an hour of plausible readings with three
/// mode changes and a stretch where the GPU sleeps. Used by the demo env
/// var and the tests.
pub(crate) fn demo_samples(n: usize) -> Vec<TempSample> {
    (0..n)
        .map(|i| {
            let f = i as f32 / n.max(1) as f32;
            let (mode, base) = match f {
                f if f < 0.35 => ("balanced", 55.0),
                f if f < 0.70 => ("performance", 74.0),
                f if f < 0.93 => ("low-power", 48.0),
                _ => ("balanced", 58.0),
            };
            let wave = (i as f32 * 0.21).sin() * 4.0 + (i as f32 * 0.047).sin() * 6.0;
            let cpu = base + wave;
            let gpu = if (0.15..0.30).contains(&f) {
                -1.0
            } else {
                base - 6.0 + (i as f32 * 0.13).cos() * 3.0
            };
            TempSample {
                cpu,
                gpu,
                mode: mode_code(mode),
            }
        })
        .collect()
}

fn rounded_rect(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    use std::f64::consts::{FRAC_PI_2, PI};
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.close_path();
}

fn time_label(secs_ago: u32) -> String {
    if secs_ago == 0 {
        "Now".into()
    } else {
        format!("{} min", secs_ago / 60)
    }
}

fn draw(area: &gtk::DrawingArea, cr: &gtk::cairo::Context, w: i32, h: i32, st: &State) {
    let (w, h) = (w as f64, h as f64);
    if w < 40.0 || h < 40.0 {
        return;
    }
    let fg = area.color();
    let (fr, fgc, fb) = (fg.red() as f64, fg.green() as f64, fg.blue() as f64);
    let dark = (fr + fgc + fb) / 3.0 > 0.5;
    #[allow(deprecated)]
    let ctx = area.style_context();
    let rgb = |name: &str, alt: (f64, f64, f64)| {
        #[allow(deprecated)]
        ctx.lookup_color(name)
            .map(|c| (c.red() as f64, c.green() as f64, c.blue() as f64))
            .unwrap_or(alt)
    };
    // window_bg is opaque; card_bg is translucent and would fill white.
    #[allow(deprecated)]
    let panel = ctx
        .lookup_color("window_bg_color")
        .filter(|c| c.alpha() > 0.99)
        .map(|c| (c.red() as f64, c.green() as f64, c.blue() as f64))
        .unwrap_or(if dark { (0.14, 0.14, 0.15) } else { (0.98, 0.98, 0.98) });
    let cpu_c = rgb("accent_bg_color", (0.784, 0.063, 0.180));
    let gpu_c = rgb("warning_color", (0.851, 0.596, 0.102));

    let (l, t) = (MARGIN_L, MARGIN_T);
    let pw = (w - MARGIN_L - MARGIN_R).max(10.0);
    let ph = (h - MARGIN_T - MARGIN_B).max(10.0);
    let span = st.span_s as f64;
    let y_of = |c: f64| t + ph - (c.clamp(MIN_T, MAX_T) - MIN_T) / (MAX_T - MIN_T) * ph;
    let x_of = |ago: f64| l + pw - ago / span * pw;

    cr.select_font_face(
        "Sans",
        gtk::cairo::FontSlant::Normal,
        gtk::cairo::FontWeight::Normal,
    );
    cr.set_font_size(10.0);
    cr.set_line_width(1.0);
    for c in [50.0, 70.0, 90.0] {
        cr.set_source_rgba(fr, fgc, fb, 0.10);
        cr.move_to(l, y_of(c));
        cr.line_to(l + pw, y_of(c));
        let _ = cr.stroke();
        cr.set_source_rgba(fr, fgc, fb, 0.6);
        cr.move_to(2.0, y_of(c) + 3.5);
        let _ = cr.show_text(&format!("{c:.0} °C"));
    }
    // Time axis: every minute (5 min view) or every 15 minutes (1 hour).
    let step = if st.span_s <= 300 { 60 } else { 900 };
    let mut ago = 0;
    while ago < st.span_s {
        let x = x_of(ago as f64);
        if ago > 0 {
            cr.set_source_rgba(fr, fgc, fb, 0.06);
            cr.move_to(x, t);
            cr.line_to(x, t + ph);
            let _ = cr.stroke();
        }
        let label = time_label(ago);
        let lw = cr.text_extents(&label).map(|e| e.width()).unwrap_or(30.0);
        let lx = if ago == 0 { x - lw } else { x - lw / 2.0 };
        cr.set_source_rgba(fr, fgc, fb, 0.6);
        cr.move_to(lx, h - 4.0);
        let _ = cr.show_text(&label);
        ago += step;
    }

    let (interval, samples) = st.series();
    let n = samples.len();
    let ago_of = |i: usize| (n - 1 - i) as f64 * interval;
    let first = (0..n).find(|&i| ago_of(i) <= span).unwrap_or(n);
    if n < 2 || n - first < 2 {
        cr.set_source_rgba(fr, fgc, fb, 0.6);
        cr.set_font_size(12.0);
        cr.move_to(l + 10.0, t + ph / 2.0 + 4.0);
        let _ = cr.show_text("Collecting temperature…");
        return;
    }

    // Mode markers below the lines: thin rule plus a small plated label.
    cr.set_font_size(10.0);
    let mut label_end = f64::MIN;
    for i in mode_changes(&samples) {
        if i < first {
            continue;
        }
        let x = x_of(ago_of(i)).round() + 0.5;
        cr.set_source_rgba(fr, fgc, fb, 0.32);
        cr.set_line_width(1.0);
        cr.move_to(x, t - 2.0);
        cr.line_to(x, t + ph);
        let _ = cr.stroke();
        let Some(name) = mode_name(samples[i].mode) else {
            continue;
        };
        let label = friendly_profile(name);
        let lw = cr.text_extents(&label).map(|e| e.width()).unwrap_or(40.0);
        let bw = lw + 10.0;
        let bx = (x - bw / 2.0).clamp(l, l + pw - bw);
        if bx < label_end + 4.0 {
            continue; // would overlap the previous label: keep only the rule
        }
        label_end = bx + bw;
        rounded_rect(cr, bx, 1.0, bw, 16.0, 4.0);
        cr.set_source_rgb(panel.0, panel.1, panel.2);
        let _ = cr.fill_preserve();
        cr.set_source_rgba(fr, fgc, fb, 0.25);
        let _ = cr.stroke();
        cr.set_source_rgba(fr, fgc, fb, 0.85);
        cr.move_to(bx + 5.0, 13.0);
        let _ = cr.show_text(&label);
    }

    for (gpu, c) in [(false, cpu_c), (true, gpu_c)] {
        cr.set_source_rgba(c.0, c.1, c.2, 0.95);
        cr.set_line_width(1.8);
        cr.set_line_join(gtk::cairo::LineJoin::Round);
        let mut pen_up = true;
        for (i, s) in samples.iter().enumerate().skip(first) {
            let v = if gpu { s.gpu } else { s.cpu } as f64;
            // 0 / -1 means "no sensor" or "GPU asleep": leave a gap.
            if v <= 0.0 {
                pen_up = true;
                continue;
            }
            let (x, y) = (x_of(ago_of(i)), y_of(v));
            if pen_up {
                cr.move_to(x, y);
                pen_up = false;
            } else {
                cr.line_to(x, y);
            }
        }
        let _ = cr.stroke();
    }
}

/// Build the "Temperature history" expander and the 2 s feed the Home
/// page poller calls (used only when the service has no history).
pub(crate) fn build_history() -> (gtk::Expander, Rc<dyn Fn(f64, f64)>) {
    let demo = std::env::var("LEGION_HISTORY_DEMO").ok();
    let state = Rc::new(RefCell::new(State {
        service: demo.as_ref().map(|_| TempHistory {
            interval_s: 5,
            samples: demo_samples(720),
        }),
        local: VecDeque::new(),
        span_s: if demo.as_deref() == Some("1h") { 3600 } else { 300 },
        fetching: false,
        demo: demo.is_some(),
    }));

    let card = gtk::Box::new(Orientation::Vertical, 6);
    card.add_css_class("panel");
    card.set_margin_top(6);
    let area = gtk::DrawingArea::new();
    area.set_content_height(140);
    area.set_hexpand(true);
    area.update_property(&[gtk::accessible::Property::Label("Temperature history")]);
    tip(&area, "CPU and GPU temperature. Lines mark power mode changes");
    {
        let st = state.clone();
        area.set_draw_func(move |a, cr, w, h| draw(a, cr, w, h, &st.borrow()));
    }
    card.append(&area);

    let foot = gtk::Box::new(Orientation::Horizontal, 16);
    foot.add_css_class("cc-legend");
    for (name, css) in [("CPU", "legend-cpu"), ("GPU", "legend-gpu")] {
        let l = gtk::Label::new(Some(name));
        l.add_css_class(css);
        l.set_valign(Align::Center);
        foot.append(&l);
    }
    let spacer = gtk::Box::new(Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    foot.append(&spacer);
    let active = SPANS.iter().position(|s| *s == state.borrow().span_s).unwrap_or(0) as u32;
    let (seg, buttons) = segmented(&["5 min".to_string(), "1 hour".to_string()], active);
    seg.set_valign(Align::Center);
    foot.append(&seg);
    for (i, b) in buttons.iter().enumerate() {
        let st = state.clone();
        let area = area.clone();
        b.connect_toggled(move |b| {
            if b.is_active() {
                st.borrow_mut().span_s = SPANS[i];
                area.queue_draw();
            }
        });
    }
    card.append(&foot);

    let expander = gtk::Expander::new(Some("Temperature history"));
    expander.add_css_class("section-expander");
    expander.set_child(Some(&card));

    let fetch: Rc<dyn Fn()> = {
        let st = state.clone();
        let area = area.clone();
        Rc::new(move || {
            {
                let mut s = st.borrow_mut();
                if s.fetching || s.demo {
                    return;
                }
                s.fetching = true;
            }
            let st = st.clone();
            let area = area.clone();
            dispatch_async(
                || match send_command(DaemonCommand::GetTempHistory) {
                    Ok(DaemonResponse::TempHistory(h)) => Ok(Some(h)),
                    // Older service or offline: draw the local samples.
                    _ => Ok(None),
                },
                "History request stopped without a result",
                move |res: Result<Option<TempHistory>, String>| {
                    let mut s = st.borrow_mut();
                    s.fetching = false;
                    s.service = res.ok().flatten();
                    drop(s);
                    area.queue_draw();
                },
            )
        })
    };

    let timer: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    {
        let fetch = fetch.clone();
        let area = area.clone();
        let timer = timer.clone();
        expander.connect_expanded_notify(move |e| {
            if !e.is_expanded() {
                if let Some(id) = timer.borrow_mut().take() {
                    id.remove();
                }
                return;
            }
            fetch();
            if timer.borrow().is_some() {
                return;
            }
            let fetch = fetch.clone();
            let area = area.clone();
            let id = glib::timeout_add_local(REFRESH, move || {
                // Only while someone can see it.
                if area.is_mapped() {
                    fetch();
                }
                glib::ControlFlow::Continue
            });
            timer.replace(Some(id));
        });
    }
    if demo.is_some() {
        expander.set_expanded(true);
        // Bring the chart into view for screenshots (focus scrolls the page).
        let btn = buttons[active as usize].clone();
        glib::timeout_add_local_once(Duration::from_millis(1500), move || {
            btn.grab_focus();
        });
    }

    let feed: Rc<dyn Fn(f64, f64)> = {
        let st = state.clone();
        let area = area.clone();
        Rc::new(move |cpu: f64, gpu: f64| {
            let mut s = st.borrow_mut();
            let sample = TempSample {
                cpu: cpu as f32,
                gpu: if gpu > 0.0 { gpu as f32 } else { -1.0 },
                mode: mode_code(&legion_core::profile::current_quiet()),
            };
            s.local.push_back(sample);
            while s.local.len() > LOCAL_CAP {
                s.local.pop_front();
            }
            let local_only = s.service.is_none();
            drop(s);
            if local_only && area.is_mapped() {
                area.queue_draw();
            }
        })
    };
    (expander, feed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_samples_cover_an_hour_with_mode_changes_and_gaps() {
        let s = demo_samples(720);
        assert_eq!(s.len(), 720);
        assert!(s.iter().all(|x| (30.0..100.0).contains(&x.cpu)));
        assert!(s.iter().any(|x| x.gpu < 0.0), "GPU asleep stretch");
        assert!(s.iter().all(|x| x.gpu < 0.0 || (30.0..100.0).contains(&x.gpu)));
        let changes = mode_changes(&s);
        assert_eq!(changes.len(), 3);
        // The last 5 minutes (60 samples) still hold one change to mark.
        assert!(changes.iter().any(|&i| i >= 720 - 60));
    }

    #[test]
    fn mode_changes_skip_unknown_samples() {
        let m = |mode| TempSample {
            cpu: 50.0,
            gpu: -1.0,
            mode,
        };
        let s = [m(2), m(0), m(2), m(3), m(0), m(3), m(1)];
        assert_eq!(mode_changes(&s), vec![3, 6]);
        assert!(mode_changes(&[]).is_empty());
    }

    #[test]
    fn time_labels() {
        assert_eq!(time_label(0), "Now");
        assert_eq!(time_label(60), "1 min");
        assert_eq!(time_label(900), "15 min");
    }
}
