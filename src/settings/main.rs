mod lighting;

mod perkey;

mod queue;

mod tray;

mod about;
mod battery;
mod cooling;
mod cpu;
mod fix;
mod overview;
mod profiles;
mod welcome;
mod widgets;

// Page builders live in the sibling modules above; re-exported here so
// every module (and the tests below) sees one flat namespace.
pub(crate) use about::*;
pub(crate) use battery::*;
pub(crate) use cooling::*;
pub(crate) use cpu::*;
pub(crate) use fix::*;
pub(crate) use overview::*;
pub(crate) use profiles::*;
pub(crate) use welcome::*;

use legion_core::comms::{send_command, DaemonCommand, DaemonResponse};
use queue::ApplyQueue;
use widgets::*;

use adw::prelude::*;
use gtk::{gio, glib, Align, Orientation};
use gtk4 as gtk;
use include_dir::{include_dir, Dir};
use libadwaita as adw;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

type PptScales = Rc<RefCell<Vec<(String, gtk::Scale, gtk::Label)>>>;
type PendingUpdate = Rc<RefCell<Option<legion_core::update::ReleaseInfo>>>;

fn color_icon(svg: &'static [u8], size: i32) -> gtk::Image {
    let bytes = glib::Bytes::from_static(svg);
    match gtk::gdk::Texture::from_bytes(&bytes) {
        Ok(texture) => {
            let image = gtk::Image::from_paintable(Some(&texture));
            image.set_pixel_size(size);
            image
        }
        Err(error) => {
            log::warn!("failed to decode bundled SVG icon: {error}");
            gtk::Image::from_icon_name("image-missing-symbolic")
        }
    }
}

fn main() {
    legion_core::logging::init("legion-settings");
    let hidden = std::env::args().any(|a| a == "--hidden" || a == "--tray" || a == "--autostart");
    if hidden {
        log::info!("starting GUI hidden to tray (pid={})", std::process::id());
    } else {
        log::info!("starting GUI (pid={})", std::process::id());
    }
    // Stash --hidden so build_ui can hide the window and keep tray.
    std::env::set_var("LEGION_HIDDEN", if hidden { "1" } else { "0" });

    let app = adw::Application::builder()
        .application_id("com.encomjp.legion-settings")
        .build();
    // Register --hidden flag so GApplication doesn't treat it as unknown.
    app.add_main_option(
        "hidden",
        glib::Char::from(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        "Start hidden to tray (for autostart)",
        None,
    );

    app.connect_startup(|_| {
        let _ = adw::init();
        adw::StyleManager::default().set_color_scheme(adw::ColorScheme::Default);

        let provider = gtk::CssProvider::new();
        provider.load_from_string(include_str!("style.css"));
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
        log::debug!("GTK/Adwaita + CSS ready");
    });

    app.connect_activate(build_ui);
    app.run();
    log::info!("GUI exited");
}

fn toast_ok(overlay: &adw::ToastOverlay, msg: &str) {
    log::info!("ui: {msg}");
    let t = adw::Toast::new(msg);
    t.set_timeout(2);
    overlay.add_toast(t);
}

fn toast_error(overlay: &adw::ToastOverlay, msg: &str) {
    log::warn!("ui error: {msg}");
    let label = gtk::Label::new(Some(msg));
    label.add_css_class("toast-error");
    // Wrap instead of letting the message stretch the toast (and with it the
    // whole window) to the full single-line width.
    label.set_wrap(true);
    label.set_max_width_chars(56);
    label.set_xalign(0.0);
    let t = adw::Toast::new("");
    t.set_custom_title(Some(&label));
    t.set_timeout(4);
    overlay.add_toast(t);
}

/// Neutral informational toast (no success/error styling, longer timeout for
/// multi-sentence explanations).
fn toast_info(overlay: &adw::ToastOverlay, msg: &str) {
    log::info!("ui: {msg}");
    let t = adw::Toast::new(msg);
    t.set_timeout(6);
    overlay.add_toast(t);
}

fn toast_with_button(
    overlay: &adw::ToastOverlay,
    msg: &str,
    button: &str,
    timeout: u32,
    on_click: impl Fn() + 'static,
) {
    let t = adw::Toast::new(msg);
    t.set_timeout(timeout);
    t.set_button_label(Some(button));
    t.connect_button_clicked(move |_| on_click());
    overlay.add_toast(t);
}

/// Quiet row-suffix button for opening external links.
fn flat_open_button(tooltip: &str) -> gtk::Button {
    let btn = gtk::Button::with_label("Open");
    btn.add_css_class("flat");
    btn.add_css_class("open-btn");
    btn.set_valign(Align::Center);
    tip(&btn, tooltip);
    btn
}

/// Advisory shown on the Battery page when the EC let the pack charge past
/// the configured limit while the laptop was off (documented firmware
/// behavior). Shared by the snapshot poll and the page-switch handler.
const PAGE_TITLES: &[(&str, &str)] = &[
    ("overview", "Home"),
    ("cpu", "CPU"),
    ("cooling-fans", "Cooling"),
    ("lighting", "Lighting"),
    ("battery-status", "Battery"),
    ("profiles", "Profiles"),
    ("about", "Settings"),
];

fn page_title(name: &str) -> Option<&'static str> {
    PAGE_TITLES
        .iter()
        .find(|(id, _)| *id == name)
        .map(|(_, title)| *title)
}

/// Map any page id (current or legacy) to its top-level stack page.
fn top_level_page(id: &str) -> &'static str {
    match id {
        "cpu" | "cpu-features" | "cpu-tuning" | "cpu-power" => "cpu",
        "about" | "about-setup" | "about-hardware" | "about-storage" | "about-help" | "fix"
        | "fix-audio" | "fix-lighting" | "fix-logs" => "about",
        "lighting" | "lighting-keyboard" | "lighting-front" | "lighting-rear" | "lighting-logo"
        | "lighting-more" => "lighting",
        "battery-status" | "battery-limit" => "battery-status",
        "cooling-fans" => "cooling-fans",
        "profiles" => "profiles",
        _ => "overview",
    }
}

/// Initial hub tab for a (possibly legacy) page id, if it names one.
/// `about-storage` is a legacy id — Storage was merged into Hardware.
fn hub_initial_tab(id: &str) -> Option<&'static str> {
    match id {
        "cpu-features" => Some("features"),
        "cpu-tuning" => Some("tuning"),
        "cpu-power" => Some("power"),
        "about-setup" => Some("setup"),
        "about-hardware" | "about-storage" => Some("hardware"),
        "about-help" => Some("help"),
        "fix" | "fix-audio" => Some("fix"),
        "fix-lighting" => Some("fix"),
        "fix-logs" => Some("fix"),
        "lighting-keyboard" => Some("keyboard"),
        "lighting-front" => Some("front"),
        "lighting-rear" => Some("rear"),
        "lighting-logo" => Some("logo"),
        "lighting-more" => Some("more"),
        _ => None,
    }
}

/// Horizontal-tab hub: an AdwViewSwitcher bar on top, the sub-pages below.
/// Each sub-page scrolls inside its own page_shell so the bar stays pinned.
/// Returns the hub box plus the inner tab stack so callers can select a tab
/// at runtime (e.g. the welcome dialog jumping straight to Setup).
fn hub_page(
    children: Vec<(gtk::Box, &'static str, &'static str)>,
    initial: Option<&str>,
) -> (gtk::Box, adw::ViewStack) {
    let tabs = adw::ViewStack::new();
    tabs.set_vexpand(true);
    for (page, id, title) in children {
        let w = page_shell(&page);
        // ViewSwitcher shows an icon when the page has one — missing icons
        // render as the red image-missing tile the user reported. Use a
        // valid symbolic icon per tab so the pill never falls back to the
        // placeholder; Wide policy will then show icon + title cleanly.
        let icon = match id {
            "setup" => "preferences-system-symbolic",
            "fix" => "applications-engineering-symbolic",
            "hardware" => "computer-symbolic",
            "help" => "help-about-symbolic",
            "features" => "applications-engineering-symbolic",
            "tuning" => "applications-system-symbolic",
            "power" => "battery-symbolic",
            "keyboard" => "input-keyboard-symbolic",
            "front" => "video-display-symbolic",
            "rear" => "video-display-symbolic",
            "logo" => "emblem-favorite-symbolic",
            "more" => "open-menu-symbolic",
            _ => "preferences-other-symbolic",
        };
        tabs.add_titled_with_icon(&w, Some(id), title, icon);
        let p = tabs.page(&w);
        p.set_needs_attention(false);
    }
    if let Some(id) = initial {
        if tabs.child_by_name(id).is_some() {
            tabs.set_visible_child_name(id);
        }
    }
    let switcher = adw::ViewSwitcher::new();
    switcher.set_stack(Some(&tabs));
    switcher.set_policy(adw::ViewSwitcherPolicy::Wide);
    switcher.set_halign(Align::Center);

    let bar = gtk::Box::new(Orientation::Vertical, 0);
    bar.add_css_class("hub-bar");
    bar.append(&switcher);

    let hub = gtk::Box::new(Orientation::Vertical, 0);
    hub.set_vexpand(true);
    hub.append(&bar);
    hub.append(&tabs);
    (hub, tabs)
}

fn copy_daemon_fix_cmd() {
    if let Some(display) = gtk::gdk::Display::default() {
        display
            .clipboard()
            .set_text("sudo systemctl enable --now legion-control");
    }
}

/// Wait briefly for the service socket after a successful start command.
fn wait_for_daemon_socket() -> Result<(), String> {
    for _ in 0..25 {
        std::thread::sleep(Duration::from_millis(120));
        if daemon_ok() {
            log::info!("daemon socket is reachable");
            return Ok(());
        }
    }
    Err("Service start returned OK but the control socket is not reachable yet".into())
}

/// Try to start the service without prompting, then use one authorized setup
/// transaction. Do not chain run0 and pkexec: both can open an auth dialog.
pub(crate) fn start_legion_control() -> Result<(), String> {
    log::info!("trying to start daemon via: systemctl start legion-control");
    match std::process::Command::new("systemctl")
        .args(["start", "legion-control"])
        .output()
    {
        Ok(out) if out.status.success() => {
            log::info!("systemctl start succeeded — waiting for socket");
            return wait_for_daemon_socket();
        }
        Ok(out) => {
            let error = String::from_utf8_lossy(&out.stderr).trim().to_string();
            log::debug!("unprivileged service start failed: {error}");
        }
        Err(error) => log::debug!("unprivileged service start unavailable: {error}"),
    }

    if setup_helper_path().is_some() || appimage_root().is_some() {
        log::info!("starting daemon through one PolicyKit setup transaction");
        return run_setup_helper_blocking("enable-daemon").and_then(|_| wait_for_daemon_socket());
    }

    // Keep a single fallback for installations that have a unit but no setup
    // helper. This is still one pkexec invocation, never run0 plus pkexec.
    log::info!("starting daemon via: pkexec systemctl start legion-control");
    let output = std::process::Command::new("pkexec")
        .args(["systemctl", "start", "legion-control"])
        .output()
        .map_err(|error| format!("Cannot start daemon through PolicyKit: {error}"))?;
    if output.status.success() {
        wait_for_daemon_socket()
    } else {
        let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if error.is_empty() {
            format!(
                "pkexec systemctl start legion-control exited {}",
                output.status
            )
        } else {
            error
        })
    }
}

fn sync_daemon_ui(
    online: bool,
    dot: &gtk::Box,
    conn_l: &gtk::Label,
    conn_s: &gtk::Label,
    foot: &gtk::Box,
    banner: &adw::Banner,
    gate: &DaemonGate,
    pending: &PendingUpdate,
) {
    log::debug!("daemon ui sync: online={online}");
    apply_conn_status(dot, conn_l, conn_s, foot, online);
    gate.set_online(online);
    apply_banner_state(banner, online, pending);
}

fn apply_banner_state(banner: &adw::Banner, online: bool, pending: &PendingUpdate) {
    if !online {
        banner.set_title("Service offline — fans, profile, and charge need it");
        banner.set_button_label(Some("Start daemon"));
        banner.set_revealed(true);
        return;
    }
    if let Some(info) = pending.borrow().as_ref() {
        banner.set_title(&format!(
            "New version available: v{} (installed v{})",
            info.version,
            legion_core::update::CURRENT_VERSION
        ));
        banner.set_button_label(Some("Update"));
        banner.set_revealed(true);
        return;
    }
    banner.set_revealed(false);
}

/// True when an IPC transport error means daemon/GUI ABI skew (bincode
/// rejected the frame) rather than a hardware or connection failure.
/// Each caller maps this to its own user-facing recovery message.
fn is_version_skew_error(e: &str) -> bool {
    e.contains("variant index") || e.contains("Parse:")
}

/// Shared worker-thread dispatch used by every async wrapper here: run
/// `work` on a background thread and deliver its result to `done` on the
/// GTK main loop via a 100 ms poll. Nothing in `work` may touch widgets.
/// If the worker dies before sending (panic), `done` receives
/// `Err(stopped_msg)`.
pub(crate) fn dispatch_async<T, F>(
    work: F,
    stopped_msg: &'static str,
    done: impl FnOnce(Result<T, String>) + 'static,
) where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(work());
    });
    let callback = Rc::new(RefCell::new(Some(done)));
    glib::timeout_add_local(Duration::from_millis(100), move || {
        match receiver.try_recv() {
            Ok(result) => {
                if let Some(done) = callback.borrow_mut().take() {
                    done(result);
                }
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => {
                if let Some(done) = callback.borrow_mut().take() {
                    done(Err(stopped_msg.to_string()));
                }
                glib::ControlFlow::Break
            }
        }
    });
}

/// Sync charge-limit write — only safe on worker threads.
fn apply_charge_limit_blocking(pct: u32) -> Result<(), String> {
    match send_command(DaemonCommand::SetChargeLimit(pct)) {
        Ok(DaemonResponse::Ok) => Ok(()),
        Ok(DaemonResponse::Error(e)) => Err(e),
        Err(e) if is_version_skew_error(&e) => Err("Service outdated — reinstall to update".into()),
        _ => legion_core::battery::set_charge_limit_pct(pct)
            .map_err(|_| "Service outdated — reinstall to update".into()),
    }
}

/// Charge-limit write without blocking GTK's main loop.
fn apply_charge_limit(pct: u32, done: impl FnOnce(Result<(), String>) + 'static) {
    dispatch_async(
        move || apply_charge_limit_blocking(pct),
        "Charge-limit request stopped without a result",
        done,
    );
}

pub(crate) fn daemon_ok() -> bool {
    matches!(
        send_command(DaemonCommand::GetProfile),
        Ok(DaemonResponse::Profile(_))
    )
}

fn build_ui(app: &adw::Application) {
    if !app.windows().is_empty() {
        app.windows()[0].present();
        return;
    }

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Legion Control")
        .default_width(1060)
        .default_height(680)
        .build();

    let toast_overlay = adw::ToastOverlay::new();
    let apply_queue = ApplyQueue::new(&toast_overlay);
    let daemon_gate = DaemonGate::new();
    let mode_drop_slot: Rc<RefCell<Option<adw::ComboRow>>> = Rc::new(RefCell::new(None));
    let profile_choices_slot: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let ppt_group_slot: Rc<RefCell<Option<adw::PreferencesGroup>>> = Rc::new(RefCell::new(None));
    let ppt_scales_slot: PptScales = Rc::new(RefCell::new(Vec::new()));
    let ppt_suppress_slot: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    let trend_feed_slot: Rc<RefCell<Option<Rc<dyn Fn(f64, f64)>>>> = Rc::new(RefCell::new(None));
    let split = adw::NavigationSplitView::new();
    split.set_min_sidebar_width(228.0);
    split.set_max_sidebar_width(280.0);
    split.set_sidebar_width_fraction(0.2);

    let stack = adw::ViewStack::new();
    stack.set_vexpand(true);

    // LEGION_PAGE can name a hub tab (e.g. cpu-tuning) — resolve before hubs.
    let legion_page_req = std::env::var("LEGION_PAGE").ok();

    let (lighting_page, lighting_tabs) = lighting::build_lighting(&toast_overlay, app);
    let battery_page = build_battery_pages(&toast_overlay, &daemon_gate);
    // Fix is now a tab inside Settings — keep the left rail short.
    let fix_compact = build_fix_compact(&toast_overlay, &daemon_gate);
    // Shared suppress flag: programmatic telemetry switches (welcome window,
    // guided setup) must not re-trigger the Settings page's opt-out nudge.
    let telemetry_sync: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    let (
        about_setup_page,
        about_help_page,
        about_hardware_page,
        welcome_consent,
        welcome_share_switch,
    ) = build_about_pages(&toast_overlay, &telemetry_sync);

    // Lighting hub: the zone ViewStack existed but had no visible switcher —
    // the sidebar used to reset it to "keyboard", leaving zones unreachable.
    let lighting_hub = {
        let switcher = adw::ViewSwitcher::new();
        switcher.set_stack(Some(&lighting_tabs));
        switcher.set_policy(adw::ViewSwitcherPolicy::Wide);
        switcher.set_halign(Align::Center);
        let bar = gtk::Box::new(Orientation::Vertical, 0);
        bar.add_css_class("hub-bar");
        bar.append(&switcher);
        let hub = gtk::Box::new(Orientation::Vertical, 0);
        hub.set_vexpand(true);
        hub.append(&bar);
        hub.append(&page_shell_width(&lighting_page, PageWidth::Wide));
        hub
    };
    if let Some(tab) = legion_page_req.as_deref().and_then(hub_initial_tab) {
        if lighting_tabs.child_by_name(tab).is_some() {
            lighting_tabs.set_visible_child_name(tab);
        }
    }

    let about_initial = legion_page_req.as_deref().and_then(hub_initial_tab);
    let (about_hub, about_tabs) = hub_page(
        vec![
            (about_setup_page, "setup", "Setup"),
            (fix_compact, "fix", "Fix"),
            (about_hardware_page, "hardware", "Hardware"),
            (about_help_page, "help", "Help"),
        ],
        about_initial,
    );

    stack.add_titled(
        &page_shell_width(
            &build_overview(
                &toast_overlay,
                &apply_queue,
                &daemon_gate,
                &mode_drop_slot,
                &profile_choices_slot,
                &ppt_group_slot,
                &ppt_scales_slot,
                &ppt_suppress_slot,
                &trend_feed_slot,
            ),
            PageWidth::Wide,
        ),
        Some("overview"),
        "Home",
    );
    stack.add_titled(
        &page_shell_width(
            &build_cooling_overview_page(&toast_overlay, &apply_queue, &daemon_gate),
            PageWidth::Wide,
        ),
        Some("cooling-fans"),
        "Cooling",
    );
    stack.add_titled(&lighting_hub, Some("lighting"), "Lighting");
    stack.add_titled(
        &page_shell(&battery_page),
        Some("battery-status"),
        "Battery",
    );
    stack.add_titled(
        &page_shell(&build_profiles_page(
            &toast_overlay,
            &daemon_gate,
            &mode_drop_slot,
            &profile_choices_slot,
        )),
        Some("profiles"),
        "Profiles",
    );
    stack.add_titled(&about_hub, Some("about"), "Settings");

    let sidebar_box = gtk::Box::new(Orientation::Vertical, 0);
    let brand = gtk::Box::new(Orientation::Horizontal, 12);
    brand.add_css_class("sidebar-brand");
    brand.set_valign(Align::Center);
    let brand_icon = color_icon(include_bytes!("../../data/icons/app-mark.svg"), 40);
    brand.append(&brand_icon);
    let brand_text = gtk::Box::new(Orientation::Vertical, 0);
    let brand_name = gtk::Label::new(Some("Legion Control"));
    brand_name.add_css_class("brand-name");
    brand_name.set_halign(Align::Start);
    tip(&brand_name, "Unofficial Legion laptop control");
    brand_text.append(&brand_name);
    brand.append(&brand_text);
    // Allow dragging the window from the brand strip (sidebar).
    let brand_handle = gtk::WindowHandle::new();
    brand_handle.set_child(Some(&brand));
    sidebar_box.append(&brand_handle);

    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .propagate_natural_height(true)
        .vexpand(true)
        .build();
    let nav_box = gtk::Box::new(Orientation::Vertical, 2);
    nav_box.set_margin_top(2);
    nav_box.set_margin_bottom(4);

    // Flat rail: one row per destination, no expandable sections — sub-pages
    // live in horizontal tab bars at the top of each hub page.
    let nav_list = gtk::ListBox::new();
    nav_list.set_selection_mode(gtk::SelectionMode::Single);
    nav_list.add_css_class("navigation-sidebar");
    nav_list.add_css_class("sidebar-flat");
    for (icon, title, tooltip) in [
        (
            include_bytes!("../../data/icons/home.svg").as_slice(),
            "Home",
            "Temperatures, fans, battery, power mode",
        ),
        (
            include_bytes!("../../data/icons/cpu.svg"),
            "CPU",
            "Features (boost, threading) · Tuning (thermal, undervolt) · Power limits",
        ),
        (
            include_bytes!("../../data/icons/cooling.svg"),
            "Cooling",
            "All fans at a glance — per-fan tuning and reset",
        ),
        (
            include_bytes!("../../data/icons/lighting.svg"),
            "Lighting",
            "Keyboard, front and rear bars, logo, per-key",
        ),
        (
            include_bytes!("../../data/icons/battery.svg"),
            "Battery",
            "Status and charge limit",
        ),
        (
            include_bytes!("../../data/icons/profiles.svg"),
            "Profiles",
            "Save and restore presets",
        ),
        (
            include_bytes!("../../data/icons/about.svg"),
            "Settings",
            "Setup, fix, hardware, help",
        ),
    ] {
        let row = adw::ActionRow::builder()
            .title(title)
            .activatable(true)
            .build();
        let ic = color_icon(icon, 15);
        ic.set_opacity(0.72);
        row.add_prefix(&ic);
        tip(&row, tooltip);
        nav_list.append(&row);
    }
    nav_box.append(&nav_list);
    scroll.set_child(Some(&nav_box));
    sidebar_box.append(&scroll);

    let foot = gtk::Box::new(Orientation::Horizontal, 10);
    foot.add_css_class("sidebar-foot");
    tip(
        &foot,
        "Shows whether the root legion-control service is reachable for fans, profile, and charge",
    );
    let dot = gtk::Box::new(Orientation::Vertical, 0);
    dot.add_css_class("conn-dot");
    let foot_text = gtk::Box::new(Orientation::Vertical, 0);
    foot_text.set_hexpand(true);
    let conn_l = gtk::Label::new(Some("Connected"));
    conn_l.add_css_class("conn-label");
    conn_l.set_halign(Align::Start);
    tip(&conn_l, "Service connection status");
    let conn_s = gtk::Label::new(Some("Service ready"));
    conn_s.add_css_class("conn-sub");
    conn_s.set_halign(Align::Start);
    tip(&conn_s, "Click to check service status");
    foot_text.append(&conn_l);
    foot_text.append(&conn_s);
    foot.append(&dot);
    foot.append(&foot_text);
    sidebar_box.append(&foot);

    // Optimistic startup state; the async probe below corrects banner, gate,
    // and connection strip — a hung daemon must not delay the first frame.
    apply_conn_status(&dot, &conn_l, &conn_s, &foot, true);
    daemon_gate.set_online(true);

    // No top bar here: the brand strip (WindowHandle) is the drag area and
    // an empty header bar wasted ~50px, which pushed the nav rail + status
    // foot off short windows (900 px screens clipped the About row).
    let sidebar_toolbar = adw::ToolbarView::new();
    sidebar_toolbar.set_content(Some(&sidebar_box));

    let sidebar_page = adw::NavigationPage::builder()
        .title("Legion")
        .child(&sidebar_toolbar)
        .build();

    // HeaderBar must be the top of the content page (CSD drag). Banner is a
    // second top bar — never above the titlebar in a plain Box.
    let content_toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    let window_title = adw::WindowTitle::new("Home", "");
    header.set_title_widget(Some(&window_title));

    let menu = gio::Menu::new();
    menu.append(Some("About Legion Control"), Some("win.about"));
    menu.append(Some("Report an issue"), Some("win.report-issue"));
    menu.append(Some("Donate"), Some("win.donate"));
    let menu_btn = gtk::MenuButton::builder()
        .tooltip_text("Menu")
        .menu_model(&menu)
        .primary(true)
        .build();
    menu_btn.set_child(Some(&color_icon(
        include_bytes!("../../data/icons/menu.svg"),
        22,
    )));
    header.pack_end(&menu_btn);
    content_toolbar.add_top_bar(&header);

    let banner = adw::Banner::new("Service offline — fans, profile, and charge need it");
    banner.set_button_label(Some("Start daemon"));
    banner.set_revealed(false);
    content_toolbar.add_top_bar(&banner);
    content_toolbar.set_content(Some(&stack));
    content_toolbar.set_vexpand(true);
    let pending_update: PendingUpdate = Rc::new(RefCell::new(None));

    // One startup probe off the main loop: fixes the banner/gate/conn strip
    // and runs the session restore only when the daemon answered.
    {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(daemon_ok());
        });
        let dot_p = dot.clone();
        let conn_l_p = conn_l.clone();
        let conn_s_p = conn_s.clone();
        let foot_p = foot.clone();
        let banner_p = banner.clone();
        let gate_p = daemon_gate.clone();
        let overlay_p = toast_overlay.clone();
        let pending_p = pending_update.clone();
        glib::timeout_add_local(Duration::from_millis(120), move || {
            let online = match rx.try_recv() {
                Ok(ok) => ok,
                Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => false,
            };
            sync_daemon_ui(
                online, &dot_p, &conn_l_p, &conn_s_p, &foot_p, &banner_p, &gate_p, &pending_p,
            );
            if online {
                // Sensor warm-up for the first overview poll.
                std::thread::spawn(|| {
                    let _ = send_command(DaemonCommand::GetSensors);
                });
                if legion_core::config::get().restore_on_launch {
                    restore_last_session(&overlay_p);
                }
            }
            glib::ControlFlow::Break
        });
    }

    // The conn strip advertises "click to check service status" — deliver
    // that: re-probe the socket on demand and toast the verdict.
    {
        let click = gtk::GestureClick::new();
        let dot_c = dot.clone();
        let conn_l_c = conn_l.clone();
        let conn_s_c = conn_s.clone();
        let foot_c = foot.clone();
        let banner_c = banner.clone();
        let gate_c = daemon_gate.clone();
        let overlay_c = toast_overlay.clone();
        let pending_c = pending_update.clone();
        click.connect_released(move |_, _, _, _| {
            conn_s_c.set_text("Checking…");
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(daemon_ok());
            });
            let dot_c = dot_c.clone();
            let conn_l_c = conn_l_c.clone();
            let conn_s_c = conn_s_c.clone();
            let foot_c = foot_c.clone();
            let banner_c = banner_c.clone();
            let gate_c = gate_c.clone();
            let overlay_c = overlay_c.clone();
            let pending_c = pending_c.clone();
            glib::timeout_add_local(Duration::from_millis(120), move || match rx.try_recv() {
                Ok(online) => {
                    sync_daemon_ui(
                        online, &dot_c, &conn_l_c, &conn_s_c, &foot_c, &banner_c, &gate_c,
                        &pending_c,
                    );
                    if online {
                        toast_ok(&overlay_c, "Daemon online — service ready");
                    } else {
                        toast_error(&overlay_c, "Daemon offline — start legion-control.service");
                    }
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            });
        });
        foot.add_controller(click);
        foot.set_cursor(gtk::gdk::Cursor::from_name("pointer", None).as_ref());
    }

    let overlay_banner = toast_overlay.clone();
    let dot_b = dot.clone();
    let conn_l_b = conn_l.clone();
    let conn_s_b = conn_s.clone();
    let foot_b = foot.clone();
    let banner_b = banner.clone();
    let gate_b = daemon_gate.clone();
    let pending_b = pending_update.clone();
    let starting = Rc::new(Cell::new(false));
    let starting_b = starting.clone();
    banner.connect_button_clicked(move |_| {
        if starting_b.get() {
            return;
        }
        if banner_b.button_label().as_deref() == Some("Update") {
            if let Some(info) = pending_b.borrow().clone() {
                about::prompt_update_dialog(&info);
            }
            return;
        }
        let overlay_ready = overlay_banner.clone();
        let dot_r = dot_b.clone();
        let conn_l_r = conn_l_b.clone();
        let conn_s_r = conn_s_b.clone();
        let foot_r = foot_b.clone();
        let banner_r = banner_b.clone();
        let gate_r = gate_b.clone();
        let pending_r = pending_b.clone();
        let starting_r = starting_b.clone();
        run_daemon_command_async(DaemonCommand::GetProfile, move |result| {
            if matches!(result, Ok(DaemonResponse::Profile(_))) {
                sync_daemon_ui(
                    true, &dot_r, &conn_l_r, &conn_s_r, &foot_r, &banner_r, &gate_r, &pending_r,
                );
                toast_ok(&overlay_ready, "Control service is ready");
                return;
            }
            starting_r.set(true);
            banner_r.set_button_label(Some("Starting…"));
            toast_ok(&overlay_ready, "Starting control service…");

            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(start_legion_control());
            });

            let overlay = overlay_ready.clone();
            let dot = dot_r.clone();
            let conn_l = conn_l_r.clone();
            let conn_s = conn_s_r.clone();
            let foot = foot_r.clone();
            let banner = banner_r.clone();
            let gate = gate_r.clone();
            let pending = pending_r.clone();
            let starting = starting_r.clone();
            glib::timeout_add_local(Duration::from_millis(200), move || match rx.try_recv() {
                Ok(Ok(())) => {
                    starting.set(false);
                    banner.set_button_label(Some("Start daemon"));
                    sync_daemon_ui(
                        true, &dot, &conn_l, &conn_s, &foot, &banner, &gate, &pending,
                    );
                    toast_ok(&overlay, "Control service started");
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    starting.set(false);
                    banner.set_button_label(Some("Start daemon"));
                    sync_daemon_ui(
                        false, &dot, &conn_l, &conn_s, &foot, &banner, &gate, &pending,
                    );
                    let overlay_c = overlay.clone();
                    toast_with_button(
                        &overlay,
                        &format!("Could not start daemon — {e}"),
                        "Copy fix",
                        8,
                        move || {
                            copy_daemon_fix_cmd();
                            toast_ok(&overlay_c, "Command copied — paste in a terminal");
                        },
                    );
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    starting.set(false);
                    banner.set_button_label(Some("Start daemon"));
                    glib::ControlFlow::Break
                }
            });
        });
    });

    let content_page = adw::NavigationPage::builder()
        .title("Home")
        .child(&content_toolbar)
        .build();

    fn nav_to(
        stack: &adw::ViewStack,
        page: &adw::NavigationPage,
        split: &adw::NavigationSplitView,
        title_widget: &adw::WindowTitle,
        name: &str,
        title: &str,
    ) {
        stack.set_visible_child_name(name);
        page.set_title(title);
        title_widget.set_title(title);
        if split.is_collapsed() {
            split.set_show_content(true);
        }
    }
    let show_page: Rc<dyn Fn(&'static str, &'static str)> = {
        let stack = stack.clone();
        let page = content_page.clone();
        let split = split.clone();
        let title_widget = window_title.clone();
        Rc::new(move |name: &'static str, title: &'static str| {
            nav_to(&stack, &page, &split, &title_widget, name, title)
        })
    };
    // The CPU hub is registered here because its Power tab needs show_page
    // to jump back to the Custom-watts controls on Home.
    let cpu_initial = legion_page_req.as_deref().and_then(hub_initial_tab);
    let (cpu_hub, _cpu_tabs) = hub_page(
        vec![
            (
                build_cpu_features_page(&toast_overlay, &daemon_gate),
                "features",
                "Features",
            ),
            (
                build_cpu_tuning_page(&toast_overlay, &daemon_gate),
                "tuning",
                "Tuning",
            ),
            (
                build_cpu_power_page(&toast_overlay, &show_page),
                "power",
                "Power",
            ),
        ],
        cpu_initial,
    );
    stack.add_titled(&cpu_hub, Some("cpu"), "CPU");

    // One flat list: row order mirrors the rail above. Fix lives inside
    // Settings — keep the rail short.
    const FLAT_IDS: &[&str] = &[
        "overview",
        "cpu",
        "cooling-fans",
        "lighting",
        "battery-status",
        "profiles",
        "about",
    ];
    {
        let show = show_page.clone();
        nav_list.connect_row_selected(move |_, row| {
            let Some(r) = row else {
                return;
            };
            if let Some(id) = FLAT_IDS.get(r.index() as usize) {
                if let Some(title) = page_title(id) {
                    show(id, title);
                }
            }
        });
    }
    let about_action = gio::SimpleAction::new("about", None);
    let win_about = window.clone();
    about_action.connect_activate(move |_, _| {
        show_about_dialog(&win_about);
    });
    window.add_action(&about_action);

    let report_action = gio::SimpleAction::new("report-issue", None);
    report_action.connect_activate(|_, _| {
        open_uri("https://github.com/encomjp/Lenovo-Legion-Control/issues/new");
    });
    window.add_action(&report_action);

    let donate_action = gio::SimpleAction::new("donate", None);
    donate_action.connect_activate(move |_, _| {
        open_uri("https://www.paypal.com/donate/?hosted_button_id=H4SCC24R8KS4A");
    });
    window.add_action(&donate_action);

    // Ctrl+1…8 jump straight to the rail pages (order mirrors the sidebar).
    for (idx, id) in FLAT_IDS.iter().enumerate() {
        let action = gio::SimpleAction::new(&format!("goto-{idx}"), None);
        let show = show_page.clone();
        let id = *id;
        action.connect_activate(move |_, _| {
            if let Some(title) = page_title(id) {
                show(id, title);
            }
        });
        window.add_action(&action);
        app.set_accels_for_action(&format!("win.goto-{idx}"), &[&format!("<Ctrl>{}", idx + 1)]);
    }

    split.set_sidebar(Some(&sidebar_page));
    split.set_content(Some(&content_page));

    // Dev/screenshots: LEGION_PAGE=<name> opens a specific page at startup
    // (e.g. LEGION_PAGE=cpu-tuning legion-settings). Legacy ids resolve to
    // their hub + tab. Harmless if unset.
    if let Some(page) = legion_page_req {
        let top = top_level_page(&page);
        if stack.child_by_name(top).is_some() {
            stack.set_visible_child_name(top);
            // Keep header in sync — the stack override bypasses nav_to().
            if let Some(title) = page_title(top) {
                content_page.set_title(title);
                window_title.set_title(title);
            }
            // Mirror the rail selection so screenshots show the highlight.
            if top == "overview" {
                nav_list.unselect_all();
            } else if let Some(idx) = FLAT_IDS.iter().position(|id| *id == top) {
                if let Some(row) = nav_list.row_at_index(idx as i32) {
                    nav_list.select_row(Some(&row));
                }
            }
        }
    }

    // Collapse sidebar on narrow widths (Adwaita breakpoint HIG — use sp for Large Text).
    let bp = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        860.0,
        adw::LengthUnit::Sp,
    ));
    bp.add_setter(&split, "collapsed", Some(&true.to_value()));
    window.add_breakpoint(bp);

    // Dev aid (mirrors LEGION_PAGE): LEGION_DEBUG_LAYOUT=1 logs every widget
    // whose minimum width exceeds the window — pinpoints "exceeds width"
    // offenders when a page stops scaling at narrow sizes.
    if std::env::var_os("LEGION_DEBUG_LAYOUT").is_some() {
        let probe_root = window.clone();
        glib::timeout_add_local(Duration::from_secs(2), move || {
            fn walk(w: &gtk::Widget, depth: usize, out: &mut Vec<String>) {
                let (min, _, _, _) = w.measure(gtk::Orientation::Horizontal, -1);
                if min > 900 {
                    out.push(format!(
                        "{}{} ({}) min={min} classes={:?}",
                        "  ".repeat(depth),
                        w.type_().name(),
                        w.widget_name(),
                        w.css_classes()
                    ));
                }
                let mut child = w.first_child();
                while let Some(c) = child {
                    walk(&c, depth + 1, out);
                    child = c.next_sibling();
                }
            }
            let mut out = Vec::new();
            walk(probe_root.upcast_ref::<gtk::Widget>(), 0, &mut out);
            if out.is_empty() {
                log::info!("layout probe: no widget wider than 900px");
            } else {
                for line in out {
                    log::warn!("layout probe: {line}");
                }
            }
            glib::ControlFlow::Break
        });
    }

    // Robust header sync: whenever the stack's visible child changes by ANY
    // path (nav_to, LEGION_PAGE, welcome dialog, future code), update the
    // header + NavigationPage titles from a single name→title map. This
    // makes the "stuck on Home" desync class structurally impossible.
    {
        let page = content_page.clone();
        let title_widget = window_title.clone();
        stack.connect_visible_child_notify(move |stk| {
            let Some(child) = stk.visible_child() else {
                return;
            };
            let name = match stk.page(&child).name() {
                Some(n) => n,
                None => return,
            };
            let Some(title) = page_title(&name) else {
                return;
            };
            page.set_title(title);
            title_widget.set_title(title);
        });
    }

    toast_overlay.set_child(Some(&split));
    window.set_content(Some(&toast_overlay));

    // Close hides to tray (Quit from tray exits).
    let win_hide = window.clone();
    window.connect_close_request(move |_| {
        win_hide.set_visible(false);
        glib::Propagation::Stop
    });

    let (tray_tx, tray_rx) = mpsc::channel::<tray::TrayCmd>();
    std::thread::spawn(move || tray::spawn(tray_tx));
    let win_tray = window.clone();
    let app_tray = app.clone();
    glib::timeout_add_local(Duration::from_millis(250), move || {
        while let Ok(cmd) = tray_rx.try_recv() {
            match cmd {
                tray::TrayCmd::Show => {
                    win_tray.set_visible(true);
                    win_tray.present();
                }
                tray::TrayCmd::Quit => {
                    app_tray.quit();
                }
            }
        }
        glib::ControlFlow::Continue
    });

    // Restore Spectrum from disk (HID path — independent of daemon).
    let cfg = legion_core::config::get();
    legion_core::keyboard::set_rgb_brightness_async(cfg.brightness);
    legion_core::keyboard::set_logo_async(cfg.logo_on);
    legion_core::keyboard::restore_lighting_async();

    show_welcome_if_needed(
        &window,
        &stack,
        Some(&about_tabs),
        &welcome_consent,
        Some(&welcome_share_switch),
        &telemetry_sync,
    );

    // Click the connection strip to re-check the daemon.
    let foot_click = gtk::GestureClick::new();
    let dot_f = dot.clone();
    let conn_l_f = conn_l.clone();
    let conn_s_f = conn_s.clone();
    let foot_f = foot.clone();
    let banner_f = banner.clone();
    let overlay_f = toast_overlay.clone();
    let gate_f = daemon_gate.clone();
    let pending_f = pending_update.clone();
    foot_click.connect_released(move |_, _, _, _| {
        let overlay_r = overlay_f.clone();
        let dot_r = dot_f.clone();
        let conn_l_r = conn_l_f.clone();
        let conn_s_r = conn_s_f.clone();
        let foot_r = foot_f.clone();
        let banner_r = banner_f.clone();
        let gate_r = gate_f.clone();
        let pending_r = pending_f.clone();
        run_daemon_command_async(DaemonCommand::GetProfile, move |result| {
            let ok = matches!(result, Ok(DaemonResponse::Profile(_)));
            sync_daemon_ui(
                ok, &dot_r, &conn_l_r, &conn_s_r, &foot_r, &banner_r, &gate_r, &pending_r,
            );
            if ok {
                toast_ok(&overlay_r, "Control service is ready");
            } else {
                let overlay = overlay_r.clone();
                toast_with_button(
                    &overlay_r,
                    "Service offline — start it from the banner",
                    "Copy fix",
                    5,
                    move || {
                        copy_daemon_fix_cmd();
                        toast_ok(&overlay, "Command copied — paste in a terminal");
                    },
                );
            }
        });
    });
    foot.add_controller(foot_click);
    tip(
        &foot,
        "Shows whether the root legion-control service is reachable — click to check again",
    );

    // Live daemon status in the sidebar (and banner).
    // IPC runs on a worker thread — a slow daemon must not freeze the UI.
    let dot_p = dot.clone();
    let conn_l_p = conn_l.clone();
    let conn_s_p = conn_s.clone();
    let foot_p = foot.clone();
    let banner_p = banner.clone();
    let gate_p = daemon_gate.clone();
    let pending_p = pending_update.clone();
    glib::timeout_add_local(Duration::from_secs(5), move || {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(daemon_ok());
        });
        let dot_q = dot_p.clone();
        let conn_l_q = conn_l_p.clone();
        let conn_s_q = conn_s_p.clone();
        let foot_q = foot_p.clone();
        let banner_q = banner_p.clone();
        let gate_q = gate_p.clone();
        let pending_q = pending_p.clone();
        glib::timeout_add_local(Duration::from_millis(250), move || match rx.try_recv() {
            Ok(ok) => {
                sync_daemon_ui(
                    ok, &dot_q, &conn_l_q, &conn_s_q, &foot_q, &banner_q, &gate_q, &pending_q,
                );
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        });
        glib::ControlFlow::Continue
    });

    let hidden = std::env::var("LEGION_HIDDEN").is_ok_and(|v| v == "1");
    if hidden {
        // Autostart: stay in tray, do not pop window. Tray thread still handles Show via menu.
        window.set_visible(false);
    } else {
        window.present();
        maybe_finish_pending_restage(&toast_overlay);
    }

    // ─── Startup update notification ───
    // One background GitHub check ~8 s after launch (let the daemon probe
    // finish first). Newer release → Home banner + dialog. Silent when up
    // to date or when the check fails (offline machines must not see noise).
    {
        let banner_u = banner.clone();
        let pending_u = pending_update.clone();
        glib::timeout_add_local_once(Duration::from_secs(8), move || {
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(legion_core::update::check_latest_release());
            });
            glib::timeout_add_local(Duration::from_millis(200), move || {
                match rx.try_recv() {
                    Ok(result) => match result {
                        Ok(info) if info.is_newer => {
                            *pending_u.borrow_mut() = Some(info.clone());
                            apply_banner_state(&banner_u, true, &pending_u);
                            if !hidden {
                                let info_c = info.clone();
                                glib::timeout_add_local_once(
                                    Duration::from_millis(400),
                                    move || {
                                        about::prompt_update_dialog(&info_c);
                                    },
                                );
                            }
                            log::info!("update check: {} available", info.version);
                        }
                        Ok(info) => {
                            log::debug!("update check: up to date (latest v{})", info.version);
                        }
                        Err(e) => {
                            log::debug!("update check failed (silent): {e}");
                        }
                    },
                    Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                    Err(mpsc::TryRecvError::Disconnected) => return glib::ControlFlow::Break,
                }
                glib::ControlFlow::Break
            });
        });
    }
}

fn apply_conn_status(
    dot: &gtk::Box,
    conn_l: &gtk::Label,
    conn_s: &gtk::Label,
    foot: &gtk::Box,
    online: bool,
) {
    if online {
        dot.remove_css_class("off");
        conn_l.set_text("Connected");
        conn_s.set_text("Service ready");
        tip(conn_l, "Root legion-control service is reachable");
        tip(
            conn_s,
            "Fans, profile, charge, and PPT commands can be applied",
        );
        tip(dot, "Green = daemon online");
        tip(
            foot,
            "Root legion-control service is reachable for fans, profile, and charge",
        );
    } else {
        dot.add_css_class("off");
        conn_l.set_text("Offline");
        conn_s.set_text("Start legion-control service");
        tip(
            conn_l,
            "Service offline — fans, profile, and charge need it",
        );
        tip(
            conn_s,
            "Use Start daemon in the banner, or: sudo systemctl enable --now legion-control",
        );
        tip(dot, "Red = daemon offline");
        tip(
            foot,
            "Service offline — start it from the banner, or: sudo systemctl enable --now legion-control",
        );
    }
}

fn open_uri(uri: &str) {
    let uri = uri.to_string();
    match std::process::Command::new("xdg-open").arg(&uri).spawn() {
        Ok(_) => {}
        Err(e) => log::warn!("failed to open {uri}: {e}"),
    }
}

fn setup_helper_path() -> Option<PathBuf> {
    // Stable host helpers first — they have a matching polkit policy with
    // auth_admin_keep (one prompt covers several operations). The AppImage
    // mount path is deliberately NOT a candidate: pkexec can never execute
    // anything inside the image because the squashfs FUSE mount is only
    // readable by the launching user, so root gets "Permission denied".
    // Portable installs bootstrap a stable helper instead (see
    // bootstrap_appimage_setup).
    let candidates: Vec<PathBuf> = vec![
        "/usr/libexec/legion-control-setup".into(),
        "/usr/local/libexec/legion-control-setup".into(),
        "/usr/lib/legion-control-setup".into(),
    ];
    candidates.into_iter().find(|path| path.is_file())
}

/// Root of the running AppImage bundle, if any. The official runtime exports
/// APPDIR; without it, fall back to the /tmp/.mount_* squashfs mount path.
fn appimage_root() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("APPDIR") {
        let dir = PathBuf::from(dir);
        if dir.is_dir() {
            return Some(dir);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let root = exe.parent()?.parent()?.parent()?;
    root.file_name()?
        .to_str()?
        .starts_with(".mount_")
        .then(|| root.to_path_buf())
}

/// One authorized transaction for portable (AppImage) installs.
///
/// pkexec can never execute anything inside an AppImage: the squashfs FUSE
/// mount is only readable by the launching user, so root's open() fails with
/// EACCES ("Error accessing …: Permission denied"). Instead the GUI streams a
/// root-owned tar payload through a single `pkexec sh` transaction; root
/// extracts it to fixed paths (stable helper, polkit policy, daemon, unit,
/// DKMS source), reloads systemd, and runs the freshly installed helper for
/// the requested operation. Every later setup action then matches the polkit
/// policy (auth_admin_keep) instead of generic per-call auth.
fn bootstrap_appimage_setup(operation: &str) -> Result<String, String> {
    let usr = appimage_root()
        .ok_or_else(|| "Not running from an AppImage bundle".to_string())?
        .join("usr");
    let helper = usr.join("libexec/legion-control-setup");
    let unit = usr.join("lib/systemd/system/legion-control.service");
    if !helper.is_file() || !unit.is_file() {
        return Err("portable bundle is missing the setup helper or unit file".into());
    }

    let stage = std::env::temp_dir().join(format!(
        "legion-bootstrap-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    let result = (|| -> Result<(), String> {
        std::fs::create_dir_all(&stage).map_err(|e| format!("cannot create staging dir: {e}"))?;
        // Owner-only: a predictable /tmp name must never let another local
        // user swap the payload that root is about to install.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&stage, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| format!("cannot lock staging dir: {e}"))?;
        }
        for dir in [
            "usr/local/bin",
            "usr/local/libexec",
            "usr/local/lib/legion-control/ryzen_smu",
            "usr/share/polkit-1/actions",
            "etc/systemd/system",
        ] {
            std::fs::create_dir_all(stage.join(dir))
                .map_err(|e| format!("cannot create staging dir {dir}: {e}"))?;
        }
        std::fs::copy(
            &helper,
            stage.join("usr/local/libexec/legion-control-setup"),
        )
        .map_err(|e| format!("cannot stage setup helper: {e}"))?;
        std::fs::copy(
            usr.join("bin/legion-daemon"),
            stage.join("usr/local/bin/legion-daemon"),
        )
        .map_err(|e| format!("cannot stage daemon: {e}"))?;
        // The staged daemon lives in /usr/local/bin — same rewrite the
        // helper's enable-daemon staging applies.
        let unit_text =
            std::fs::read_to_string(&unit).map_err(|e| format!("cannot read bundled unit: {e}"))?;
        std::fs::write(
            stage.join("etc/systemd/system/legion-control.service"),
            unit_text.replace(
                "ExecStart=/usr/bin/legion-daemon",
                "ExecStart=/usr/local/bin/legion-daemon",
            ),
        )
        .map_err(|e| format!("cannot stage unit: {e}"))?;
        std::fs::copy(
            usr.join("share/polkit-1/actions/com.encomjp.legion-control.policy"),
            stage.join("usr/share/polkit-1/actions/com.encomjp.legion-control.policy"),
        )
        .map_err(|e| format!("cannot stage polkit policy: {e}"))?;
        for entry in std::fs::read_dir(usr.join("lib/legion-control/ryzen_smu"))
            .map_err(|e| format!("cannot read bundled ryzen_smu: {e}"))?
        {
            let entry = entry.map_err(|e| format!("cannot inspect bundle entry: {e}"))?;
            if entry.file_type().map_err(|e| e.to_string())?.is_file() {
                std::fs::copy(
                    entry.path(),
                    stage
                        .join("usr/local/lib/legion-control/ryzen_smu")
                        .join(entry.file_name()),
                )
                .map_err(|e| format!("cannot stage ryzen_smu: {e}"))?;
            }
        }

        let mut tar = std::process::Command::new("tar")
            .current_dir(&stage)
            .args(["--owner=0", "--group=0", "-cf", "-", "usr", "etc"])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot start tar: {e}"))?;
        let payload = tar
            .stdout
            .take()
            .ok_or_else(|| "tar produced no payload".to_string())?;
        let output = std::process::Command::new("pkexec")
            .args([
                "/bin/sh",
                "-c",
                "tar -C / -xpf - && systemctl daemon-reload \
                 && /usr/local/libexec/legion-control-setup \"$1\" \
                 && systemctl try-restart legion-control.service",
                "sh",
                operation,
            ])
            .stdin(payload)
            .output()
            .map_err(|error| format!("Cannot start PolicyKit setup: {error}"))?;
        let _ = tar.wait();
        if output.status.success() {
            Ok(())
        } else {
            let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Err(if error.is_empty() {
                format!("Setup was cancelled or failed ({})", output.status)
            } else {
                error
            })
        }
    })();
    let _ = std::fs::remove_dir_all(&stage);
    result?;

    Ok(format!(
        "bootstrap staged stable helper + policy ({operation})"
    ))
}

/// Restage daemon/helper from the *running* AppImage squashfs, not the
/// previously copied host helper (which would be one version behind).
pub(crate) fn restage_from_running_appimage() -> Result<String, String> {
    if appimage_root().is_none() {
        return Err("Not running from an AppImage bundle".into());
    }
    bootstrap_appimage_setup("enable-daemon")
}

fn maybe_finish_pending_restage(overlay: &adw::ToastOverlay) {
    if !legion_core::update::has_pending_restage() {
        return;
    }
    if legion_core::update::running_appimage_path().is_none() {
        legion_core::update::clear_pending_restage();
        return;
    }
    if !legion_core::update::daemon_was_staged() {
        legion_core::update::clear_pending_restage();
        return;
    }
    let overlay = overlay.clone();
    glib::timeout_add_local_once(Duration::from_millis(600), move || {
        dispatch_async(
            restage_from_running_appimage,
            "Service refresh stopped without a result",
            move |result| match result {
                Ok(_) => {
                    legion_core::update::clear_pending_restage();
                    toast_ok(&overlay, "Background service updated to this version");
                }
                Err(e) => {
                    toast_error(
                        &overlay,
                        &format!("Could not refresh the background service — {e}"),
                    );
                }
            },
        );
    });
}

/// Run one daemon IPC request without blocking GTK's main loop.
fn run_daemon_command_async(
    command: DaemonCommand,
    done: impl FnOnce(Result<DaemonResponse, String>) + 'static,
) {
    dispatch_async(
        move || send_command(command),
        "Daemon request stopped without a result",
        done,
    );
}

/// Run one fixed PolicyKit setup operation synchronously.
pub(crate) fn run_setup_helper_blocking(operation: &str) -> Result<String, String> {
    if setup_helper_path().is_none() {
        if appimage_root().is_some() {
            log::info!("bootstrap: staging stable setup helper via one PolicyKit transaction");
            return bootstrap_appimage_setup(operation);
        }
        return Err(
            "Setup helper is missing; reinstall Legion Control from the current package".into(),
        );
    }
    let helper = setup_helper_path().expect("stable helper checked above");
    let output = std::process::Command::new("pkexec")
        .arg(&helper)
        .arg(operation)
        .output()
        .map_err(|error| format!("Cannot start PolicyKit setup: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if error.is_empty() {
            format!("Setup was cancelled or failed ({})", output.status)
        } else {
            error
        })
    }
}

/// Run one fixed PolicyKit setup operation without blocking GTK's main loop.
fn run_setup_helper(operation: &'static str, done: impl FnOnce(Result<String, String>) + 'static) {
    dispatch_async(
        move || run_setup_helper_blocking(operation),
        "Setup helper stopped without a result",
        done,
    );
}

fn restore_last_session(_overlay: &adw::ToastOverlay) {
    // Firmware/Fn+Q is authoritative at startup.  Restoring the rest of the
    // previous session must never overwrite a mode selected before the GUI
    // opened (for example Quiet -> saved Balanced).
    std::thread::spawn(move || {
        let mut p = legion_core::config::get().last_session;
        if let Ok(DaemonResponse::Profile(current)) = send_command(DaemonCommand::GetProfile) {
            p.platform_profile = current.clone();
            legion_core::config::remember_platform_profile(&current);
        }
        apply_profile_blocking(&p, false);
    });
}

fn confirm_risk(
    parent: &impl glib::object::IsA<gtk::Widget>,
    title: &str,
    body: &str,
    proceed_label: &str,
    done: impl FnOnce(bool) + 'static,
) {
    let dialog = adw::AlertDialog::new(Some(title), Some(body));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("proceed", proceed_label);
    dialog.set_response_appearance("proceed", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    let done = Cell::new(Some(done));
    dialog.connect_response(None, move |_, response| {
        if let Some(cb) = done.take() {
            cb(response == "proceed");
        }
    });

    let root = parent.as_ref().root();
    let win = root.and_then(|r| r.downcast::<gtk::Window>().ok());
    dialog.present(win.as_ref());
}

fn build_thermal_card(toast: &adw::ToastOverlay, gate: &DaemonGate) -> gtk::Box {
    // Same visual grammar as the sections below (Curve Optimizer etc.):
    // chips row directly on the page, then a plain boxed-list group — no
    // extra .card wrapper that made this section wider than its neighbours.
    let page = gtk::Box::new(Orientation::Vertical, 18);
    let group = pref_group("Thermal throttle", None);
    tip(
        &group,
        "Governor clamps scaling_max_freq when hot — gentle 100 MHz steps near the limit, up to 300 MHz for big overshoots, 1 s poll with sensor-spike smoothing. Restores 7 °C below on a 100 MHz/s ramp. TjMax 95 °C is the hardware failsafe (daemon-native port of cpu-throttle-95.sh, k10temp CPU/CCD2 temps).",
    );

    // Bare switch in the group header — the slider below explains itself,
    // no duplicated title/subtitle row.
    let enabled = gtk::Switch::new();
    enabled.set_valign(Align::Center);
    tip(
        &enabled,
        "On = daemon steps scaling_max_freq when temp ≥ max; Off = no clamp",
    );
    group.set_header_suffix(Some(&enabled));

    let value = gtk::Label::new(Some("90 °C"));
    value.add_css_class("numeric");
    value.add_css_class("scale-value");
    let adj = gtk::Adjustment::new(90.0, 70.0, 98.0, 1.0, 5.0, 0.0);
    let scale = gtk::Scale::new(Orientation::Horizontal, Some(&adj));
    scale.add_css_class("thermal-scale");
    scale.set_draw_value(false);
    scale.set_digits(0);
    scale.set_hexpand(true);
    scale.set_width_request(180);
    value.set_width_chars(6);
    value.set_xalign(1.0);
    tip(&scale, "Maximum temperature — restore point is 7 °C below");
    tip(&value, "Current threshold");
    // TjMax tick labels under the trough
    let scale_marks = gtk::Box::new(Orientation::Horizontal, 0);
    scale_marks.add_css_class("scale-marks");
    scale_marks.set_hexpand(true);
    scale_marks.set_halign(Align::Fill);
    let mark_70 = gtk::Label::new(Some("70"));
    mark_70.set_halign(Align::Start);
    mark_70.set_hexpand(true);
    let mark_95 = gtk::Label::new(Some("95 TjMax"));
    mark_95.set_halign(Align::Center);
    mark_95.set_hexpand(true);
    let mark_98 = gtk::Label::new(Some("98"));
    mark_98.set_halign(Align::End);
    mark_98.set_hexpand(true);
    scale_marks.append(&mark_70);
    scale_marks.append(&mark_95);
    scale_marks.append(&mark_98);

    // Stacked column: title + live value on one line, full-width slider
    // below, marks aligned to the slider. (The old ActionRow squeezed the
    // scale into a suffix box next to the title, which overflowed the card
    // edge and duplicated the switch row's state text.)
    let temp_title = gtk::Label::new(Some("Maximum temperature"));
    temp_title.add_css_class("row-title");
    temp_title.set_halign(Align::Start);
    tip(
        &temp_title,
        "70–98 °C slider — 96–98 °C needs explicit confirmation (above TjMax)",
    );
    value.set_halign(Align::End);
    value.set_hexpand(true);
    let temp_top = gtk::Box::new(Orientation::Horizontal, 12);
    temp_top.set_hexpand(true);
    temp_top.append(&temp_title);
    temp_top.append(&value);
    let temp_box = gtk::Box::new(Orientation::Vertical, 6);
    temp_box.set_hexpand(true);
    temp_box.append(&temp_top);
    temp_box.append(&scale);
    temp_box.append(&scale_marks);
    group.add(&temp_box);

    // Muted slider when throttling off — also used by initial load + toggle.
    let apply_mute: Rc<dyn Fn(bool)> = {
        let scale_c = scale.clone();
        let row_c = temp_box.clone();
        let marks_c = scale_marks.clone();
        Rc::new(move |on: bool| {
            if on {
                scale_c.remove_css_class("muted");
                row_c.set_sensitive(true);
                marks_c.set_opacity(1.0);
            } else {
                scale_c.add_css_class("muted");
                row_c.set_sensitive(false);
                marks_c.set_opacity(0.42);
            }
        })
    };

    let chips = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .max_children_per_line(3)
        .min_children_per_line(2)
        .homogeneous(true)
        .column_spacing(12)
        .row_spacing(12)
        .build();
    chips.add_css_class("metric-grid");
    let (cpu_temp_chip, cpu_temp_v, cpu_temp_d) =
        metric_chip_tip("CPU temp", Some("Main CPU temperature (k10temp Tctl)"));
    let (cpu_temp_2_chip, cpu_temp_2_v, cpu_temp_2_d) = metric_chip_tip(
        "CPU CCD 2",
        Some("Second CPU CCD temperature (k10temp Tccd2)"),
    );
    let (freq_chip, freq_v, freq_d) = metric_chip_tip(
        "Max freq",
        Some("Current scaling_max_freq across online CPUs"),
    );
    chips.append(&cpu_temp_chip);
    chips.append(&cpu_temp_2_chip);
    chips.append(&freq_chip);
    chips.set_margin_bottom(0);
    page.append(&chips);
    page.append(&group);
    gate.track(&group);
    gate.track(&chips);

    // Shared state
    let suppress: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    let last_max: Rc<Cell<u8>> = Rc::new(Cell::new(90));
    // Last config confirmed synced with the daemon — the poll compares against
    // this (not the live widgets) so a mid-drag slider or pending confirm
    // dialog is never mistaken for external drift.
    let last_on: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    let acked: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    let debounce: Rc<Cell<u32>> = Rc::new(Cell::new(0));

    // Immediate daemon write (no hysteresis UI, no inline ack).
    let do_apply = {
        let scale_c = scale.clone();
        let enabled_c = enabled.clone();
        let toast_c = toast.clone();
        let suppress_c = suppress.clone();
        let last_max_c = last_max.clone();
        let last_on_c = last_on.clone();
        let acked_c = acked.clone();
        let value_c = value.clone();
        let apply_mute_c = apply_mute.clone();
        Rc::new(move |max_temp: u8, enabled_val: bool, acknowledge: bool| {
            let enabled_cc = enabled_c.clone();
            let toast_cc = toast_c.clone();
            let suppress_cc = suppress_c.clone();
            let last_max_cc = last_max_c.clone();
            let last_on_cc = last_on_c.clone();
            let acked_cc = acked_c.clone();
            let scale_cc = scale_c.clone();
            let value_cc = value_c.clone();
            let apply_mute_cc = apply_mute_c.clone();
            run_daemon_command_async(
                DaemonCommand::SetThermal {
                    enabled: enabled_val,
                    max_temp,
                    acknowledge,
                },
                move |result| match result {
                    Ok(DaemonResponse::ThermalStatus(st)) => {
                        suppress_cc.set(true);
                        enabled_cc.set_active(st.config.enabled);
                        scale_cc.set_value(st.config.max_temp as f64);
                        value_cc.set_text(&format!("{} °C", st.config.max_temp));
                        apply_mute_cc(st.config.enabled);
                        suppress_cc.set(false);
                        // Persist ack state: daemon accepted this max, so if it was ≥96 we are acked
                        if st.config.max_temp >= 96 {
                            acked_cc.set(true);
                        } else {
                            acked_cc.set(false);
                        }
                        last_max_cc.set(st.config.max_temp);
                        last_on_cc.set(st.config.enabled);
                    }
                    Ok(DaemonResponse::Error(e)) => toast_error(&toast_cc, &e),
                    Ok(other) => toast_error(&toast_cc, &format!("Unexpected response: {other:?}")),
                    Err(e) => toast_error(&toast_cc, &e),
                },
            );
        })
    };

    // Debounced apply from slider — gate 96–98 through confirm_risk.
    let do_apply_debounced: Rc<dyn Fn()> = {
        let scale_c = scale.clone();
        let enabled_c = enabled.clone();
        let debounce_c = debounce.clone();
        let toast_c = toast.clone();
        let suppress_c = suppress.clone();
        let last_max_c = last_max.clone();
        let acked_c = acked.clone();
        let value_c = value.clone();
        let do_apply_c = do_apply.clone();
        Rc::new(move || {
            let ticket = debounce_c.get().wrapping_add(1);
            debounce_c.set(ticket);
            let scale_cc = scale_c.clone();
            let enabled_cc = enabled_c.clone();
            let _toast_cc = toast_c.clone();
            let suppress_cc = suppress_c.clone();
            let last_max_cc = last_max_c.clone();
            let acked_cc = acked_c.clone();
            let value_cc = value_c.clone();
            let debounce_cc = debounce_c.clone();
            let do_apply_cc = do_apply_c.clone();
            glib::timeout_add_local_once(Duration::from_millis(140), move || {
                if debounce_cc.get() != ticket {
                    return;
                }
                let max_temp = scale_cc.value().round().clamp(70.0, 98.0) as u8;
                let enabled_val = enabled_cc.is_active();
                // Live label + subtitle — always reflect slider immediately
                value_cc.set_text(&format!("{max_temp} °C"));
                suppress_cc.set(true);
                suppress_cc.set(false);
                if max_temp >= 96 && !acked_cc.get() {
                    let scale_for_dialog = scale_cc.clone();
                    let suppress_for_dialog = suppress_cc.clone();
                    let last = last_max_cc.get();
                    let do_apply_ok = do_apply_cc.clone();
                    confirm_risk(
                        &scale_cc,
                        "Exceed TjMax 95 °C?",
                        THERMAL_TJMAX_WARNING,
                        &format!("Use {max_temp} °C anyway"),
                        move |ok| {
                            if !ok {
                                suppress_for_dialog.set(true);
                                scale_for_dialog.set_value(last as f64);
                                value_cc.set_text(&format!("{last} °C"));
                                suppress_for_dialog.set(false);
                                return;
                            }
                            acked_cc.set(true);
                            do_apply_ok(max_temp, enabled_val, true);
                        },
                    );
                    return;
                }
                if max_temp < 96 {
                    acked_cc.set(false);
                }
                let ack = acked_cc.get();
                do_apply_cc(max_temp, enabled_val, ack);
            });
        })
    };

    // Initial load
    {
        let scale_c = scale.clone();
        let enabled_c = enabled.clone();
        let value_c = value.clone();
        let suppress_c = suppress.clone();
        let last_max_c = last_max.clone();
        let last_on_c = last_on.clone();
        let acked_c = acked.clone();
        let cpu_temp_v_c = cpu_temp_v.clone();
        let cpu_temp_d_c = cpu_temp_d.clone();
        let cpu_temp_2_v_c = cpu_temp_2_v.clone();
        let cpu_temp_2_d_c = cpu_temp_2_d.clone();
        let freq_v_c = freq_v.clone();
        let freq_d_c = freq_d.clone();
        let cpu_temp_chip_c = cpu_temp_chip.clone();
        let cpu_temp_2_chip_c = cpu_temp_2_chip.clone();
        let freq_chip_c = freq_chip.clone();
        let apply_mute_c = apply_mute.clone();
        run_daemon_command_async(DaemonCommand::GetThermalStatus, move |result| {
            suppress_c.set(true);
            match result {
                Ok(DaemonResponse::ThermalStatus(st)) => {
                    enabled_c.set_active(st.config.enabled);
                    scale_c.set_value(st.config.max_temp as f64);
                    value_c.set_text(&format!("{} °C", st.config.max_temp));
                    last_max_c.set(st.config.max_temp);
                    last_on_c.set(st.config.enabled);
                    acked_c.set(st.config.max_temp >= 96);
                    apply_mute_c(st.config.enabled);
                    let cpu_temp_c = st.cpu_temp_mc.map(|v| v as f64 / 1000.0);
                    let cpu_temp_2_c = st.cpu_temp_2_mc.map(|v| v as f64 / 1000.0);
                    if let Some(c) = cpu_temp_c {
                        cpu_temp_v_c.set_text(&format!("{c:.1} °C"));
                        cpu_temp_d_c.set_text(if st.active { "throttling" } else { "idle" });
                        tint_temp(&cpu_temp_chip_c, c);
                    } else {
                        cpu_temp_v_c.set_text("—");
                        cpu_temp_d_c.set_text("no sensor");
                        tint_temp(&cpu_temp_chip_c, 0.0);
                    }
                    if let Some(c) = cpu_temp_2_c {
                        cpu_temp_2_v_c.set_text(&format!("{c:.1} °C"));
                        cpu_temp_2_d_c.set_text("");
                        tint_temp(&cpu_temp_2_chip_c, c);
                    } else {
                        cpu_temp_2_v_c.set_text("—");
                        cpu_temp_2_d_c.set_text("no sensor");
                        tint_temp(&cpu_temp_2_chip_c, 0.0);
                    }
                    freq_v_c.set_text(&format!("{:.2} GHz", st.cur_max_freq as f64 / 1_000_000.0));
                    freq_d_c.set_text(if st.active { "clamped" } else { "full" });
                    let tint_c = cpu_temp_c
                        .into_iter()
                        .chain(cpu_temp_2_c)
                        .fold(f64::NAN, f64::max);
                    if tint_c.is_finite() {
                        tint_temp(&freq_chip_c, tint_c);
                    } else {
                        tint_temp(&freq_chip_c, 0.0);
                    }
                }
                Ok(DaemonResponse::Error(e)) => {
                    cpu_temp_v_c.set_text("—");
                    cpu_temp_d_c.set_text(&e);
                    cpu_temp_2_v_c.set_text("—");
                    freq_v_c.set_text("—");
                }
                Ok(other) => {
                    cpu_temp_v_c.set_text("—");
                    cpu_temp_d_c.set_text(&format!("{other:?}"));
                }
                Err(e) => {
                    cpu_temp_v_c.set_text("—");
                    cpu_temp_d_c.set_text(&e);
                }
            }
            suppress_c.set(false);
        });
    }

    {
        let do_apply_c = do_apply.clone();
        let suppress_c = suppress.clone();
        let scale_c = scale.clone();
        let acked_c = acked.clone();
        let apply_mute_c = apply_mute.clone();
        enabled.connect_active_notify(move |row| {
            if suppress_c.get() {
                return;
            }
            let on = row.is_active();
            let max_temp = scale_c.value().round().clamp(70.0, 98.0) as u8;
            (*apply_mute_c)(on);
            if max_temp >= 96 && !acked_c.get() {
                let row_c = row.clone();
                let suppress_c2 = suppress_c.clone();
                let acked_cc = acked_c.clone();
                let do_apply_ok = do_apply_c.clone();
                let apply_mute_ok = apply_mute_c.clone();
                confirm_risk(
                    row,
                    "Exceed TjMax 95 °C?",
                    THERMAL_TJMAX_WARNING,
                    &format!("Use {max_temp} °C anyway"),
                    move |ok| {
                        if !ok {
                            suppress_c2.set(true);
                            row_c.set_active(!on);
                            (*apply_mute_ok)(false);
                            suppress_c2.set(false);
                            return;
                        }
                        acked_cc.set(true);
                        do_apply_ok(max_temp, on, true);
                        (*apply_mute_ok)(on);
                    },
                );
                suppress_c.set(true);
                row.set_active(!on);
                suppress_c.set(false);
                return;
            }
            let ack = acked_c.get();
            do_apply_c(max_temp, on, ack);
            (*apply_mute_c)(on);
        });
    }

    {
        let debounced = do_apply_debounced.clone();
        let suppress_c = suppress.clone();
        scale.connect_value_changed(move |_| {
            if suppress_c.get() {
                return;
            }
            debounced();
        });
    }

    let cpu_temp_v_p = cpu_temp_v.clone();
    let cpu_temp_d_p = cpu_temp_d.clone();
    let cpu_temp_2_v_p = cpu_temp_2_v.clone();
    let cpu_temp_2_d_p = cpu_temp_2_d.clone();
    let freq_v_p = freq_v.clone();
    let freq_d_p = freq_d.clone();
    let cpu_temp_chip_p = cpu_temp_chip.clone();
    let cpu_temp_2_chip_p = cpu_temp_2_chip.clone();
    let freq_chip_p = freq_chip.clone();
    let enabled_p = enabled.clone();
    let scale_p = scale.clone();
    let value_p = value.clone();
    let suppress_p = suppress.clone();
    let last_max_p = last_max.clone();
    let last_on_p = last_on.clone();
    let acked_p = acked.clone();
    let apply_mute_p = apply_mute.clone();
    glib::timeout_add_local(Duration::from_secs(2), move || {
        let cpu_temp_v_c = cpu_temp_v_p.clone();
        let cpu_temp_d_c = cpu_temp_d_p.clone();
        let cpu_temp_2_v_c = cpu_temp_2_v_p.clone();
        let cpu_temp_2_d_c = cpu_temp_2_d_p.clone();
        let freq_v_c = freq_v_p.clone();
        let freq_d_c = freq_d_p.clone();
        let cpu_temp_chip_c = cpu_temp_chip_p.clone();
        let cpu_temp_2_chip_c = cpu_temp_2_chip_p.clone();
        let freq_chip_c = freq_chip_p.clone();
        let enabled_c = enabled_p.clone();
        let scale_c = scale_p.clone();
        let value_c = value_p.clone();
        let suppress_c = suppress_p.clone();
        let last_max_c = last_max_p.clone();
        let last_on_c = last_on_p.clone();
        let acked_c = acked_p.clone();
        let apply_mute_c = apply_mute_p.clone();
        run_daemon_command_async(
            DaemonCommand::GetThermalStatus,
            move |result| match result {
                Ok(DaemonResponse::ThermalStatus(st)) => {
                    let cpu_temp_c = st.cpu_temp_mc.map(|v| v as f64 / 1000.0);
                    let cpu_temp_2_c = st.cpu_temp_2_mc.map(|v| v as f64 / 1000.0);
                    if let Some(c) = cpu_temp_c {
                        cpu_temp_v_c.set_text(&format!("{c:.1} °C"));
                        cpu_temp_d_c.set_text(if st.active { "throttling" } else { "idle" });
                        tint_temp(&cpu_temp_chip_c, c);
                    } else {
                        cpu_temp_v_c.set_text("—");
                        cpu_temp_d_c.set_text("no sensor");
                        tint_temp(&cpu_temp_chip_c, 0.0);
                    }
                    if let Some(c) = cpu_temp_2_c {
                        cpu_temp_2_v_c.set_text(&format!("{c:.1} °C"));
                        cpu_temp_2_d_c.set_text("");
                        tint_temp(&cpu_temp_2_chip_c, c);
                    } else {
                        cpu_temp_2_v_c.set_text("—");
                        cpu_temp_2_d_c.set_text("no sensor");
                        tint_temp(&cpu_temp_2_chip_c, 0.0);
                    }
                    freq_v_c.set_text(&format!("{:.2} GHz", st.cur_max_freq as f64 / 1_000_000.0));
                    freq_d_c.set_text(if st.active { "clamped" } else { "full" });
                    let tint_c = cpu_temp_c
                        .into_iter()
                        .chain(cpu_temp_2_c)
                        .fold(f64::NAN, f64::max);
                    if tint_c.is_finite() {
                        tint_temp(&freq_chip_c, tint_c);
                    } else {
                        tint_temp(&freq_chip_c, 0.0);
                    }
                    // External drift (CLI, daemon restart, another window):
                    // re-sync the controls to the daemon. Compared against the
                    // last confirmed state, never the live widgets, so an
                    // in-flight drag or open confirm dialog is left alone.
                    if st.config.enabled != last_on_c.get()
                        || st.config.max_temp != last_max_c.get()
                    {
                        suppress_c.set(true);
                        enabled_c.set_active(st.config.enabled);
                        scale_c.set_value(st.config.max_temp as f64);
                        value_c.set_text(&format!("{} °C", st.config.max_temp));
                        apply_mute_c(st.config.enabled);
                        suppress_c.set(false);
                        acked_c.set(st.config.max_temp >= 96);
                        last_max_c.set(st.config.max_temp);
                        last_on_c.set(st.config.enabled);
                    }
                }
                Ok(DaemonResponse::Error(e)) => cpu_temp_d_c.set_text(&e),
                Err(e) => cpu_temp_d_c.set_text(&e),
                _ => {}
            },
        );
        glib::ControlFlow::Continue
    });

    page
}

fn set_pill(label: &gtk::Label, text: &str, kind: &str) {
    for c in ["status-ok", "status-warn", "status-bad", "status-muted"] {
        label.remove_css_class(c);
    }
    label.set_text(text);
    label.add_css_class(match kind {
        "ok" => "status-ok",
        "warn" => "status-warn",
        "bad" => "status-bad",
        _ => "status-muted",
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_kde_widget_extracts_with_required_files() {
        let path = extract_kde_widget().expect("bundled widget should extract");
        assert!(path.join("metadata.json").is_file());
        assert!(path.join("contents/ui/main.qml").is_file());
        assert!(path.join("contents/ui/legion-poll.sh").is_file());
        std::fs::remove_dir_all(path).expect("temporary widget should be removable");
    }

    #[test]
    fn stability_memory_pass_is_deterministic() {
        let mut memory = vec![0_u64; 4096];
        assert!(stability_memory_pass(0x1234_5678, &mut memory));
        let first = memory.clone();
        assert!(stability_memory_pass(0x1234_5678, &mut memory));
        assert_eq!(memory, first);
    }

    #[test]
    fn short_stability_test_completes_without_errors() {
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        spawn_stability_test_for(stop, tx, Duration::from_millis(50));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            assert!(Instant::now() < deadline, "stability test timed out");
            match rx.recv_timeout(Duration::from_millis(250)) {
                Ok(StabilityEvent::Finished { cancelled, errors }) => {
                    assert!(!cancelled);
                    assert_eq!(errors, 0);
                    break;
                }
                Ok(StabilityEvent::Progress(_)) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(error) => panic!("stability test channel failed: {error}"),
            }
        }
    }

    #[test]
    fn offsets_text_formats_uniform_mixed_and_empty() {
        assert_eq!(offsets_text(&[]), "—");
        assert_eq!(offsets_text(&[-15, -15, -15]), "All cores: -15");
        assert_eq!(offsets_text(&[-15, -4, -15]), "Mixed");
    }

    #[test]
    fn page_titles_resolve_for_every_top_level_page() {
        for (id, title) in PAGE_TITLES {
            assert_eq!(page_title(id), Some(*title), "page_title({id})");
        }
        assert_eq!(page_title("does-not-exist"), None);
    }

    #[test]
    fn legacy_page_ids_resolve_to_a_registered_hub_tab() {
        // Every id the LEGION_PAGE override accepts (current + legacy) must
        // land on a top-level stack page, and hub tabs must belong to it.
        let known = [
            "overview",
            "cpu",
            "cpu-features",
            "cpu-tuning",
            "cpu-power",
            "cooling-fans",
            "lighting",
            "lighting-keyboard",
            "lighting-front",
            "lighting-rear",
            "lighting-logo",
            "lighting-more",
            "battery-status",
            "battery-limit",
            "fix",
            "fix-audio",
            "fix-lighting",
            "fix-logs",
            "profiles",
            "about",
            "about-setup",
            "about-hardware",
            "about-storage",
            "about-help",
        ];
        for id in known {
            let top = top_level_page(id);
            assert!(
                page_title(top).is_some(),
                "top_level_page({id}) = {top} has no title"
            );
            if let Some(tab) = hub_initial_tab(id) {
                let hub = match top {
                    "cpu" => "cpu hub",
                    "about" => "about hub",
                    "fix" => "fix hub",
                    "lighting" => "lighting hub",
                    other => panic!("hub tab {tab} mapped into non-hub page {other}"),
                };
                let _ = hub;
                let valid = match top {
                    "cpu" => ["features", "tuning", "power"].contains(&tab),
                    "about" => ["setup", "fix", "hardware", "help"].contains(&tab),
                    "fix" => ["fix-audio", "fix-lighting", "fix-logs"].contains(&tab),
                    "lighting" => ["keyboard", "front", "rear", "logo", "more"].contains(&tab),
                    _ => false,
                };
                assert!(valid, "tab {tab} not registered in {top} hub");
            }
        }
        // Unknown ids fall back to Home without a tab.
        assert_eq!(top_level_page("garbage"), "overview");
        assert_eq!(hub_initial_tab("garbage"), None);
    }
}
