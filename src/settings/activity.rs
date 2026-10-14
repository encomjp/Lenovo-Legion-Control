//! One shared "is anybody looking" signal and the pausable poller built on it.
//!
//! `main.rs` feeds [`set_activity`] from the window state (visible, minimized,
//! focused) and [`set_visible_page`] from the page stack. Every recurring
//! poller created through [`ticker_local`] then follows that state:
//!
//! * window hidden, closed to the tray or minimized: no timer exists at all;
//! * window visible but not focused: at most one tick every 5 s;
//! * window focused: the poller's own interval;
//! * a poller built while a page was being constructed (see [`with_page`])
//!   only runs while that page is the visible one.
//!
//! When a poller wakes up again it fires once straight away so the page never
//! shows stale values.

use super::*;
use std::sync::atomic::AtomicU8;

/// How much of the app the user can currently see.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Activity {
    /// Hidden in the tray, closed or minimized.
    Hidden = 0,
    /// Visible, but another window has the focus.
    Background = 1,
    /// Visible and focused.
    Active = 2,
}

/// Slowest poll rate while the window is visible but unfocused.
const BACKGROUND_INTERVAL: Duration = Duration::from_secs(5);

static ACTIVITY: AtomicU8 = AtomicU8::new(Activity::Active as u8);
/// Top-level page on screen. Shared so worker threads can check it too.
static VISIBLE_PAGE: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

fn visible_page_is(page: &str) -> bool {
    VISIBLE_PAGE.lock().is_ok_and(|v| *v == page)
}

/// For background worker threads: true while `page` is on screen in a window
/// that is shown. Threads skip their sampling round when this is false.
pub(crate) fn page_polling(page: &str) -> bool {
    activity() != Activity::Hidden && visible_page_is(page)
}

/// Current activity. Safe to read from worker threads.
pub(crate) fn activity() -> Activity {
    match ACTIVITY.load(Ordering::Relaxed) {
        0 => Activity::Hidden,
        1 => Activity::Background,
        _ => Activity::Active,
    }
}

struct Ticker {
    base: Duration,
    /// Page this poller belongs to; `None` runs whenever the window is shown.
    page: Option<String>,
    cb: RefCell<Box<dyn FnMut() -> glib::ControlFlow>>,
    /// Armed timer and the interval it was armed with.
    source: RefCell<Option<(glib::SourceId, Duration)>>,
    done: Cell<bool>,
}

thread_local! {
    static TICKERS: RefCell<Vec<Rc<Ticker>>> = const { RefCell::new(Vec::new()) };
    static BUILDING_PAGE: RefCell<Option<String>> = const { RefCell::new(None) };
}

impl Ticker {
    /// Interval this poller should run at right now, `None` when it must rest.
    fn wanted(&self) -> Option<Duration> {
        let act = activity();
        if act == Activity::Hidden {
            return None;
        }
        if let Some(page) = &self.page {
            if !visible_page_is(page) {
                return None;
            }
        }
        Some(if act == Activity::Background {
            self.base.max(BACKGROUND_INTERVAL)
        } else {
            self.base
        })
    }

    fn disarm(&self) {
        if let Some((id, _)) = self.source.borrow_mut().take() {
            if let Some(src) = glib::MainContext::default().find_source_by_id(&id) {
                src.destroy();
            }
        }
    }
}

/// Run the callback once; a `Break` retires the poller for good.
fn fire(t: &Rc<Ticker>) -> glib::ControlFlow {
    if t.done.get() {
        return glib::ControlFlow::Break;
    }
    let Ok(mut cb) = t.cb.try_borrow_mut() else {
        return glib::ControlFlow::Continue;
    };
    let flow = (*cb)();
    drop(cb);
    if flow == glib::ControlFlow::Break {
        t.done.set(true);
        t.disarm();
    }
    flow
}

fn arm(t: &Rc<Ticker>, iv: Duration) {
    let tc = t.clone();
    let tick = move || fire(&tc);
    let id = if iv.subsec_nanos() == 0 && iv.as_secs() >= 1 {
        // Whole seconds: let glib coalesce the wake-ups.
        glib::timeout_add_seconds_local(iv.as_secs() as u32, tick)
    } else {
        glib::timeout_add_local(iv, tick)
    };
    *t.source.borrow_mut() = Some((id, iv));
}

fn sync(t: &Rc<Ticker>) {
    if t.done.get() {
        return;
    }
    let old = t.source.borrow().as_ref().map(|(_, iv)| *iv);
    let want = t.wanted();
    if old == want {
        return;
    }
    t.disarm();
    let Some(iv) = want else {
        return;
    };
    arm(t, iv);
    // Waking from rest (or speeding up): refresh right away.
    if old.is_none_or(|o| o > iv) {
        let tc = t.clone();
        glib::idle_add_local_once(move || {
            if tc.source.borrow().is_some() {
                fire(&tc);
            }
        });
    }
}

fn sync_all() {
    let all: Vec<Rc<Ticker>> = TICKERS.with(|l| {
        let mut l = l.borrow_mut();
        l.retain(|t| !t.done.get());
        l.clone()
    });
    for t in &all {
        sync(t);
    }
}

/// Drop-in for `glib::timeout_add_local` for recurring pollers. The callback
/// follows the window and page state described in the module docs. One-shot
/// result pumps (a channel waiting for a single answer) should keep using
/// plain glib timeouts.
pub(crate) fn ticker_local(interval: Duration, f: impl FnMut() -> glib::ControlFlow + 'static) {
    let t = Rc::new(Ticker {
        base: interval,
        page: BUILDING_PAGE.with(|b| b.borrow().clone()),
        cb: RefCell::new(Box::new(f)),
        source: RefCell::new(None),
        done: Cell::new(false),
    });
    TICKERS.with(|l| l.borrow_mut().push(t.clone()));
    // First arming never fires early: the creator already has fresh data.
    if let Some(iv) = t.wanted() {
        arm(&t, iv);
    }
}

/// Tag every poller created inside `build` with `page`, so it only runs while
/// that page is the visible one.
pub(crate) fn with_page<T>(page: &str, build: impl FnOnce() -> T) -> T {
    let prev = BUILDING_PAGE.with(|b| b.replace(Some(page.to_string())));
    let out = build();
    BUILDING_PAGE.with(|b| *b.borrow_mut() = prev);
    out
}

/// Feed the window state in; pollers rearm or rest accordingly.
pub(crate) fn set_activity(next: Activity) {
    if ACTIVITY.swap(next as u8, Ordering::Relaxed) != next as u8 {
        log::debug!("activity: {next:?}");
        sync_all();
    }
}

/// Tell the pollers which top-level page is showing.
pub(crate) fn set_visible_page(page: &str) {
    let changed = match VISIBLE_PAGE.lock() {
        Ok(mut v) if *v != page => {
            *v = page.to_string();
            true
        }
        _ => false,
    };
    if changed {
        sync_all();
    }
}

/// Derive the activity from a window: hidden, minimized or occluded windows
/// rest, an unfocused one slows down.
pub(crate) fn window_activity(window: &adw::ApplicationWindow) -> Activity {
    use gtk::gdk::ToplevelState;
    if !window.is_visible() {
        return Activity::Hidden;
    }
    let resting = window
        .surface()
        .and_then(|s| s.downcast::<gtk::gdk::Toplevel>().ok())
        .is_some_and(|t| {
            t.state()
                .intersects(ToplevelState::MINIMIZED | ToplevelState::SUSPENDED)
        });
    if resting {
        Activity::Hidden
    } else if window.is_active() {
        Activity::Active
    } else {
        Activity::Background
    }
}

/// Keep [`set_activity`] in step with `window` for its whole life.
pub(crate) fn follow_window(window: &adw::ApplicationWindow) {
    let update = {
        let w = window.clone();
        move || set_activity(window_activity(&w))
    };
    let u = update.clone();
    window.connect_is_active_notify(move |_| u());
    let u = update.clone();
    window.connect_visible_notify(move |_| u());
    let u = update.clone();
    window.connect_map(move |_| u());
    let u = update.clone();
    window.connect_unmap(move |_| u());
    let u = update.clone();
    window.connect_realize(move |w| {
        if let Some(top) = w.surface().and_then(|s| s.downcast::<gtk::gdk::Toplevel>().ok()) {
            let u = u.clone();
            top.connect_state_notify(move |_| u());
        }
    });
    update();
}
