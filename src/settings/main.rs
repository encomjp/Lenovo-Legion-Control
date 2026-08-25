//! Legion Control — standalone GTK4 / libadwaita app.

mod lighting;
mod perkey;
mod queue;
mod tray;
mod widgets;

use legion_core::comms::{send_command, DaemonCommand, DaemonResponse};
use queue::ApplyQueue;
use widgets::*;

use adw::prelude::*;
use gtk::{gio, glib, Align, Orientation};
use gtk4 as gtk;
use include_dir::{include_dir, Dir};
use libadwaita as adw;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

const KDE_WIDGET_ID: &str = "com.github.encomjp.legioncontrol";
static KDE_WIDGET_PACKAGE: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/kde-widget/package");

/// Shared GTK widgets for the Custom-mode PPT sliders group.
type PptScales = Rc<RefCell<Vec<(String, gtk::Scale, gtk::Label)>>>;

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

/// Single source of truth for top-level page ids → header titles. nav_to callers,
/// the LEGION_PAGE override, and the visible-child sync all read this so the
/// maps can never disagree.
const PAGE_TITLES: &[(&str, &str)] = &[
    ("overview", "Home"),
    ("cpu", "CPU"),
    ("cooling-fans", "Cooling"),
    ("lighting", "Lighting"),
    ("battery-status", "Battery"),
    ("fix", "Fix"),
    ("profiles", "Profiles"),
    ("about", "About"),
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
        "about" | "about-setup" | "about-hardware" | "about-storage" | "about-help" => "about",
        "lighting" | "lighting-keyboard" => "lighting",
        "battery-status" | "battery-limit" => "battery-status",
        "fix" | "fix-audio" | "fix-lighting" | "fix-logs" => "fix",
        "cooling-fans" => "cooling-fans",
        "profiles" => "profiles",
        _ => "overview",
    }
}

/// Initial hub tab for a (possibly legacy) page id, if it names one.
fn hub_initial_tab(id: &str) -> Option<&'static str> {
    match id {
        "cpu-features" => Some("features"),
        "cpu-tuning" => Some("tuning"),
        "cpu-power" => Some("power"),
        "about-setup" => Some("setup"),
        "about-hardware" => Some("hardware"),
        "about-storage" => Some("storage"),
        "about-help" => Some("help"),
        "fix-audio" => Some("fix-audio"),
        "fix-lighting" => Some("fix-lighting"),
        "fix-logs" => Some("fix-logs"),
        _ => None,
    }
}

/// Horizontal-tab hub: an AdwViewSwitcher bar on top, the sub-pages below.
/// Each sub-page scrolls inside its own page_shell so the bar stays pinned.
fn hub_page(
    children: Vec<(gtk::Box, &'static str, &'static str)>,
    initial: Option<&str>,
) -> gtk::Box {
    let tabs = adw::ViewStack::new();
    tabs.set_vexpand(true);
    for (page, id, title) in children {
        tabs.add_titled(&page_shell(&page), Some(id), title);
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
    hub
}

fn copy_daemon_fix_cmd() {
    if let Some(display) = gtk::gdk::Display::default() {
        display
            .clipboard()
            .set_text("sudo systemctl enable --now legion-control");
    }
}

/// Try to start the system legion-control unit (plain → run0 → pkexec).
fn start_legion_control() -> Result<(), String> {
    let attempts: &[&[&str]] = &[
        &["systemctl", "start", "legion-control"],
        &["run0", "systemctl", "start", "legion-control"],
        &["pkexec", "systemctl", "start", "legion-control"],
    ];
    let mut last = "Could not start legion-control".to_string();
    for argv in attempts {
        log::info!("trying to start daemon via: {}", argv.join(" "));
        match std::process::Command::new(argv[0])
            .args(&argv[1..])
            .output()
        {
            Ok(out) if out.status.success() => {
                log::info!("{} succeeded — waiting for socket", argv.join(" "));
                for _ in 0..25 {
                    std::thread::sleep(Duration::from_millis(120));
                    if daemon_ok() {
                        log::info!("daemon socket is reachable");
                        return Ok(());
                    }
                }
                return Err(
                    "Service start returned OK but the control socket is not reachable yet".into(),
                );
            }
            Ok(out) => {
                let err = String::from_utf8_lossy(&out.stderr);
                let err = err.trim();
                if !err.is_empty() {
                    last = err.to_string();
                } else {
                    last = format!("{} exited {}", argv.join(" "), out.status);
                }
                log::warn!("start attempt failed: {last}");
            }
            Err(e) => {
                last = format!("{}: {e}", argv[0]);
                log::warn!("start attempt failed: {last}");
            }
        }
    }
    Err(last)
}

fn sync_daemon_ui(
    online: bool,
    dot: &gtk::Box,
    conn_l: &gtk::Label,
    conn_s: &gtk::Label,
    foot: &gtk::Box,
    banner: &adw::Banner,
    gate: &DaemonGate,
) {
    log::debug!("daemon ui sync: online={online}");
    apply_conn_status(dot, conn_l, conn_s, foot, online);
    banner.set_revealed(!online);
    gate.set_online(online);
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
fn dispatch_async<T, F>(
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

fn daemon_ok() -> bool {
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
        .default_width(1180)
        .default_height(760)
        .build();

    let toast_overlay = adw::ToastOverlay::new();
    let apply_queue = ApplyQueue::new(&toast_overlay);
    let daemon_gate = DaemonGate::new();
    let mode_drop_slot: Rc<RefCell<Option<adw::ComboRow>>> = Rc::new(RefCell::new(None));
    let profile_choices_slot: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let ppt_group_slot: Rc<RefCell<Option<adw::PreferencesGroup>>> = Rc::new(RefCell::new(None));
    let ppt_scales_slot: PptScales = Rc::new(RefCell::new(Vec::new()));
    let ppt_suppress_slot: Rc<Cell<bool>> = Rc::new(Cell::new(false));
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
    let fix_page = build_fix_page(&toast_overlay, &daemon_gate);
    let (about_setup_page, about_help_page, about_hardware_page, about_storage_page) =
        build_about_pages(&toast_overlay);

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
        hub.append(&page_shell(&lighting_page));
        hub
    };
    if legion_page_req.as_deref() == Some("lighting-keyboard") {
        lighting_tabs.set_visible_child_name("keyboard");
    }

    let about_initial = legion_page_req.as_deref().and_then(hub_initial_tab);
    let about_hub = hub_page(
        vec![
            (about_setup_page, "setup", "Setup"),
            (about_hardware_page, "hardware", "Hardware"),
            (about_storage_page, "storage", "Storage"),
            (about_help_page, "help", "Help"),
        ],
        about_initial,
    );

    stack.add_titled(
        &page_shell(&build_overview(
            &toast_overlay,
            &apply_queue,
            &daemon_gate,
            &mode_drop_slot,
            &profile_choices_slot,
            &ppt_group_slot,
            &ppt_scales_slot,
            &ppt_suppress_slot,
        )),
        Some("overview"),
        "Overview",
    );
    stack.add_titled(
        &page_shell(&build_cooling_overview_page(
            &toast_overlay,
            &apply_queue,
            &daemon_gate,
        )),
        Some("cooling-fans"),
        "Cooling Fans",
    );
    stack.add_titled(&lighting_hub, Some("lighting"), "Lighting");
    stack.add_titled(
        &page_shell(&battery_page),
        Some("battery-status"),
        "Battery",
    );
    stack.add_titled(&page_shell(&fix_page), Some("fix"), "Fix");
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
    stack.add_titled(&about_hub, Some("about"), "About");

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
            include_bytes!("../../data/icons/fix.svg"),
            "Fix",
            "Diagnostics and repair — speakers, Spectrum RGB, service logs",
        ),
        (
            include_bytes!("../../data/icons/profiles.svg"),
            "Profiles",
            "Save and restore presets",
        ),
        (
            include_bytes!("../../data/icons/about.svg"),
            "About",
            "Setup, hardware, storage, help",
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

    let sidebar_toolbar = adw::ToolbarView::new();
    sidebar_toolbar.add_top_bar(&adw::HeaderBar::new());
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
        glib::timeout_add_local(Duration::from_millis(120), move || {
            let online = match rx.try_recv() {
                Ok(ok) => ok,
                Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => false,
            };
            sync_daemon_ui(
                online, &dot_p, &conn_l_p, &conn_s_p, &foot_p, &banner_p, &gate_p,
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

    let overlay_banner = toast_overlay.clone();
    let dot_b = dot.clone();
    let conn_l_b = conn_l.clone();
    let conn_s_b = conn_s.clone();
    let foot_b = foot.clone();
    let banner_b = banner.clone();
    let gate_b = daemon_gate.clone();
    let starting = Rc::new(Cell::new(false));
    let starting_b = starting.clone();
    banner.connect_button_clicked(move |_| {
        if starting_b.get() {
            return;
        }
        let overlay_ready = overlay_banner.clone();
        let dot_r = dot_b.clone();
        let conn_l_r = conn_l_b.clone();
        let conn_s_r = conn_s_b.clone();
        let foot_r = foot_b.clone();
        let banner_r = banner_b.clone();
        let gate_r = gate_b.clone();
        let starting_r = starting_b.clone();
        run_daemon_command_async(DaemonCommand::GetProfile, move |result| {
            if matches!(result, Ok(DaemonResponse::Profile(_))) {
                sync_daemon_ui(
                    true, &dot_r, &conn_l_r, &conn_s_r, &foot_r, &banner_r, &gate_r,
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
            let starting = starting_r.clone();
            glib::timeout_add_local(Duration::from_millis(200), move || match rx.try_recv() {
                Ok(Ok(())) => {
                    starting.set(false);
                    banner.set_button_label(Some("Start daemon"));
                    sync_daemon_ui(true, &dot, &conn_l, &conn_s, &foot, &banner, &gate);
                    toast_ok(&overlay, "Control service started");
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    starting.set(false);
                    banner.set_button_label(Some("Start daemon"));
                    sync_daemon_ui(false, &dot, &conn_l, &conn_s, &foot, &banner, &gate);
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
    let cpu_hub = hub_page(
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

    // One flat list: row order mirrors the rail above.
    const FLAT_IDS: &[&str] = &[
        "overview",
        "cpu",
        "cooling-fans",
        "lighting",
        "battery-status",
        "fix",
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
        open_uri("https://github.com/encomjp/");
    });
    window.add_action(&report_action);

    let donate_action = gio::SimpleAction::new("donate", None);
    donate_action.connect_activate(move |_, _| {
        open_uri("https://www.paypal.com/donate/?hosted_button_id=H4SCC24R8KS4A");
    });
    window.add_action(&donate_action);

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

    show_welcome_if_needed(&window, &stack);

    // Click the connection strip to re-check the daemon.
    let foot_click = gtk::GestureClick::new();
    let dot_f = dot.clone();
    let conn_l_f = conn_l.clone();
    let conn_s_f = conn_s.clone();
    let foot_f = foot.clone();
    let banner_f = banner.clone();
    let overlay_f = toast_overlay.clone();
    let gate_f = daemon_gate.clone();
    foot_click.connect_released(move |_, _, _, _| {
        let overlay_r = overlay_f.clone();
        let dot_r = dot_f.clone();
        let conn_l_r = conn_l_f.clone();
        let conn_s_r = conn_s_f.clone();
        let foot_r = foot_f.clone();
        let banner_r = banner_f.clone();
        let gate_r = gate_f.clone();
        run_daemon_command_async(DaemonCommand::GetProfile, move |result| {
            let ok = matches!(result, Ok(DaemonResponse::Profile(_)));
            sync_daemon_ui(
                ok, &dot_r, &conn_l_r, &conn_s_r, &foot_r, &banner_r, &gate_r,
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
        glib::timeout_add_local(Duration::from_millis(250), move || match rx.try_recv() {
            Ok(ok) => {
                sync_daemon_ui(
                    ok, &dot_q, &conn_l_q, &conn_s_q, &foot_q, &banner_q, &gate_q,
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

fn show_about_dialog(parent: &impl glib::object::IsA<gtk::Widget>) {
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
        .issue_url("https://github.com/encomjp/")
        .license_type(gtk::License::Gpl20Only)
        .developers(["europeanpepe (encomjp)"])
        .copyright("© europeanpepe / encomjp")
        .build();
    about.add_link("Author on GitHub", "https://github.com/encomjp/");
    about.add_link(
        "Donate (PayPal)",
        "https://www.paypal.com/donate/?hosted_button_id=H4SCC24R8KS4A",
    );
    about.add_link("Report an issue", "https://github.com/encomjp/");
    about.add_link(
        "Spectrum protocol notes",
        "https://github.com/alstergee/legion-spectrum-control",
    );
    about.present(Some(parent));
}

fn open_uri(uri: &str) {
    let uri = uri.to_string();
    match std::process::Command::new("xdg-open").arg(&uri).spawn() {
        Ok(_) => {}
        Err(e) => log::warn!("failed to open {uri}: {e}"),
    }
}

fn setup_helper_path() -> Option<&'static str> {
    [
        "/usr/libexec/legion-control-setup", // Fedora, Arch, source --prefix /usr
        "/usr/local/libexec/legion-control-setup", // source installs (default prefix)
        "/usr/lib/legion-control-setup",     // Debian-style relocation, just in case
    ]
    .into_iter()
    .find(|path| std::path::Path::new(path).is_file())
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

/// Run one fixed PolicyKit setup operation without blocking GTK's main loop.
fn run_setup_helper(operation: &'static str, done: impl FnOnce(Result<String, String>) + 'static) {
    let helper = match setup_helper_path() {
        Some(path) => path,
        None => {
            done(Err(
                "Setup helper is missing; reinstall Legion Control from the current package".into(),
            ));
            return;
        }
    };
    dispatch_async(
        move || {
            let result = std::process::Command::new("pkexec")
                .arg(helper)
                .arg(operation)
                .output()
                .map_err(|error| format!("Cannot start PolicyKit setup: {error}"))
                .and_then(|output| {
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
                });
            result
        },
        "Setup helper stopped without a result",
        done,
    );
}

fn show_welcome_if_needed(parent: &impl glib::object::IsA<gtk::Widget>, stack: &adw::ViewStack) {
    if legion_core::config::welcome_seen() {
        return;
    }
    let dialog = adw::AlertDialog::new(
        Some("Welcome to Legion Control"),
        Some(
            "Unofficial community tool for Lenovo Legion laptops.\n\n\
             Not affiliated with Lenovo. Use at your own risk.\n\n\
             Choose optional components now, or change them later under About.\n\n\
             This build is alpha: anonymous diagnostics stay OFF unless you enable them in Setup.",
        ),
    );
    dialog.add_response("ok", "Not now");
    dialog.add_response("donate", "Donate");
    dialog.add_response("issues", "Report an issue");
    dialog.add_response("setup", "First-time setup");
    dialog.set_response_appearance("donate", adw::ResponseAppearance::Suggested);
    dialog.set_response_appearance("setup", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("setup"));
    dialog.set_close_response("ok");
    let stack = stack.clone();
    dialog.connect_response(None, move |_, response| {
        legion_core::config::mark_welcome_seen();
        match response {
            "donate" => open_uri("https://www.paypal.com/donate/?hosted_button_id=H4SCC24R8KS4A"),
            "issues" => open_uri("https://github.com/encomjp/"),
            "setup" => stack.set_visible_child_name("about-setup"),
            _ => {}
        }
    });
    let root = parent.as_ref().root();
    let win = root.and_then(|r| r.downcast::<gtk::Window>().ok());
    dialog.present(win.as_ref());
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

/// IPC half of a profile apply — daemon writes, config, and keyboard calls.
/// Must run off the GTK main loop; returns the collected per-part errors.
fn apply_profile_blocking(
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

fn apply_profile(
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

#[allow(clippy::too_many_arguments)]
fn build_overview(
    toast_overlay: &adw::ToastOverlay,
    apply_queue: &ApplyQueue,
    gate: &DaemonGate,
    mode_drop_slot: &Rc<RefCell<Option<adw::ComboRow>>>,
    profile_choices_slot: &Rc<RefCell<Vec<String>>>,
    ppt_group_slot: &Rc<RefCell<Option<adw::PreferencesGroup>>>,
    ppt_scales_slot: &PptScales,
    ppt_suppress_slot: &Rc<Cell<bool>>,
) -> gtk::Box {
    let page = page_lede("");

    let metrics = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .max_children_per_line(3)
        .min_children_per_line(2)
        .homogeneous(true)
        .column_spacing(12)
        .row_spacing(12)
        .build();
    metrics.add_css_class("metric-grid");

    let (cpu_chip, cpu_v, cpu_d) =
        metric_chip_tip("CPU", Some("Package temperature (°C) and busy percentage"));
    let (gpu_chip, gpu_v, gpu_d) =
        metric_chip_tip("GPU", Some("Discrete GPU temperature (°C) and utilization"));
    let (bat_chip, bat_v, bat_d) = metric_chip_tip(
        "Battery",
        Some("Charge level · subtitle shows the current power mode"),
    );

    metrics.append(&cpu_chip);
    metrics.append(&gpu_chip);
    metrics.append(&bat_chip);

    let fan_channels = legion_core::fans::channels();
    let mut fan_metric_labels: Vec<(u8, gtk::Label, gtk::Label)> = Vec::new();
    for ch in &fan_channels {
        let tip_txt = match ch.id {
            1 => "CPU fan speed in RPM (Auto = firmware curve)",
            2 => "GPU fan speed in RPM (Auto = firmware curve)",
            4 => "Auxiliary chassis fan speed in RPM",
            _ => "Fan speed in RPM (Auto = firmware curve)",
        };
        let short = match ch.id {
            1 => "Fan · CPU".to_string(),
            2 => "Fan · GPU".to_string(),
            4 => "Fan · Aux".to_string(),
            _ => format!("Fan · {}", ch.id),
        };
        let (chip, v, d) = metric_chip_tip(&short, Some(tip_txt));
        metrics.append(&chip);
        fan_metric_labels.push((ch.id, v, d));
    }
    page.append(&metrics);

    let power = pref_group("Power mode", None);
    tip(
        &power,
        "Quiet / Balanced / Performance / Max / Custom — Custom unlocks CPU PPT and GPU AC power sliders below",
    );

    let choices = legion_core::profile::choices();
    *profile_choices_slot.borrow_mut() = choices.clone();
    let labels: Vec<String> = choices.iter().map(|c| friendly_profile(c)).collect();
    let label_refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
    let current = legion_core::profile::current();
    let active = choices.iter().position(|c| *c == current).unwrap_or(0) as u32;

    let drop = string_combo_row("Mode", "", &label_refs, active);
    tip(&drop, profile_tooltip(&current));
    power.add(&drop);
    *mode_drop_slot.borrow_mut() = Some(drop.clone());

    let overlay = toast_overlay.clone();
    let choices_c = choices.clone();
    let ppt_box_slot = ppt_group_slot.clone();
    let ppt_scales_slot_c = ppt_scales_slot.clone();
    let ppt_suppress_slot_c = ppt_suppress_slot.clone();
    let profile_guard = Rc::new(Cell::new(false));
    let last_ok = Rc::new(Cell::new(active));
    let profile_guard_n = profile_guard.clone();
    let last_ok_n = last_ok.clone();
    drop.connect_selected_notify(move |d| {
        if profile_guard_n.get() {
            return;
        }
        let idx = d.selected() as usize;
        if idx >= choices_c.len() {
            return;
        }
        let name = choices_c[idx].clone();
        tip(d, profile_tooltip(&name));

        let apply = {
            let name = name.clone();
            let overlay = overlay.clone();
            let ppt_box_slot = ppt_box_slot.clone();
            let ppt_scales_slot = ppt_scales_slot_c.clone();
            let ppt_suppress_slot = ppt_suppress_slot_c.clone();
            move || {
                if let Some(ppt_box) = ppt_box_slot.borrow().as_ref() {
                    apply_platform_profile(
                        &name,
                        &overlay,
                        ppt_box,
                        &ppt_scales_slot,
                        &ppt_suppress_slot,
                    );
                } else {
                    let overlay_e = overlay.clone();
                    run_daemon_command_async(
                        DaemonCommand::SetProfile(name.clone()),
                        move |result| match result {
                            Ok(DaemonResponse::Ok) => {
                                legion_core::config::remember_platform_profile(&name);
                                toast_ok(
                                    &overlay_e,
                                    &format!("Switched to {}", friendly_profile(&name)),
                                );
                            }
                            Ok(DaemonResponse::Error(e)) => toast_error(&overlay_e, &e),
                            Err(e) => toast_error(&overlay_e, &e),
                            _ => {}
                        },
                    );
                }
            }
        };

        if name == "max-power" && last_ok_n.get() != idx as u32 {
            let drop_r = d.clone();
            let choices_c = choices_c.clone();
            let profile_guard = profile_guard_n.clone();
            let last_ok = last_ok_n.clone();
            let prev = last_ok.get();
            confirm_max_power(d, move |accepted| {
                if !accepted {
                    profile_guard.set(true);
                    drop_r.set_selected(prev);
                    let prev_name = choices_c
                        .get(prev as usize)
                        .cloned()
                        .unwrap_or_else(|| "balanced".into());
                    tip(&drop_r, profile_tooltip(&prev_name));
                    profile_guard.set(false);
                    return;
                }
                apply();
                last_ok.set(
                    choices_c
                        .iter()
                        .position(|c| c == "max-power")
                        .unwrap_or(prev as usize) as u32,
                );
            });
            return;
        }

        if name == "custom" && last_ok_n.get() != idx as u32 {
            let drop_r = d.clone();
            let choices_c = choices_c.clone();
            let profile_guard = profile_guard_n.clone();
            let last_ok = last_ok_n.clone();
            let prev = last_ok.get();
            confirm_custom_power(d, move |accepted| {
                if !accepted {
                    profile_guard.set(true);
                    drop_r.set_selected(prev);
                    let prev_name = choices_c
                        .get(prev as usize)
                        .cloned()
                        .unwrap_or_else(|| "balanced".into());
                    tip(&drop_r, profile_tooltip(&prev_name));
                    profile_guard.set(false);
                    return;
                }
                apply();
                last_ok.set(
                    choices_c
                        .iter()
                        .position(|c| c == "custom")
                        .unwrap_or(prev as usize) as u32,
                );
            });
            return;
        }

        apply();
        last_ok_n.set(idx as u32);
    });
    page.append(&power);
    gate.track(&power);

    // Custom PPT / GPU sliders — shown on Home when Mode is Custom.
    attach_custom_ppt_group(
        &page,
        toast_overlay,
        apply_queue,
        gate,
        &drop,
        &choices,
        &current,
        ppt_group_slot,
        ppt_scales_slot,
        ppt_suppress_slot,
    );

    if let Some(pct) = legion_core::battery::capacity() {
        bat_v.set_text(&format!("{pct}%"));
        bat_d.set_text(&legion_core::battery::status().unwrap_or_default());
    }

    let cpu_chip_c = cpu_chip.clone();
    let gpu_chip_c = gpu_chip.clone();
    let mode_drop_poll = drop.clone();
    let choices_poll = choices.clone();
    let profile_guard_poll = profile_guard.clone();
    let last_ok_poll = last_ok.clone();
    let last_firmware_mode = Rc::new(RefCell::new(current));
    let _ = legion_core::sensors::sample_cpu_usage_pct();

    // Data collected off the main thread each tick. IPC and nvidia-smi can
    // block for seconds — they must never run inside the GTK main loop.
    struct DashboardPoll {
        cpu_pct: f64,
        gpu_pct: f64,
        firmware_mode: Option<String>,
        sensors: Option<legion_core::sensors::SensorReadings>,
        cpu_w: Option<f64>,
        local: Option<legion_core::sensors::SensorReadings>,
        local_battery: Option<(u32, String)>,
    }
    let (dash_tx, dash_rx) = mpsc::channel::<DashboardPoll>();
    std::thread::Builder::new()
        .name("dashboard-poll".into())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_secs(2));
            let cpu_pct = legion_core::sensors::sample_cpu_usage_pct();
            let gpu_pct = legion_core::sensors::sample_gpu_usage_pct();
            let firmware_mode = match send_command(DaemonCommand::GetProfile) {
                Ok(DaemonResponse::Profile(mode)) => Some(mode),
                _ => None,
            };
            let sensors = match send_command(DaemonCommand::GetSensors) {
                Ok(DaemonResponse::Sensors(s)) => Some(s),
                _ => None,
            };
            let cpu_w = match send_command(DaemonCommand::GetCpuPower) {
                Ok(DaemonResponse::CpuPower(w)) if w > 0.5 => Some(w),
                _ => None,
            };
            let (local, local_battery) = if sensors.is_none() {
                (
                    Some(legion_core::sensors::read_all()),
                    legion_core::battery::capacity()
                        .map(|pct| (pct, legion_core::battery::status().unwrap_or_default())),
                )
            } else {
                (None, None)
            };
            if dash_tx
                .send(DashboardPoll {
                    cpu_pct,
                    gpu_pct,
                    firmware_mode,
                    sensors,
                    cpu_w,
                    local,
                    local_battery,
                })
                .is_err()
            {
                break; // UI closed
            }
        })
        .ok();

    // Drain poll results on the main thread and update the widgets.
    glib::timeout_add_local(Duration::from_millis(200), move || {
        while let Ok(poll) = dash_rx.try_recv() {
            // Fn+Q changes happen outside this process.  Poll the daemon's
            // hardware-authoritative profile and update the row without allowing
            // the programmatic selection change to emit a SetProfile command.
            if let Some(firmware_mode) = poll.firmware_mode {
                let changed = *last_firmware_mode.borrow() != firmware_mode;
                if changed {
                    *last_firmware_mode.borrow_mut() = firmware_mode.clone();
                    if let Some(index) = choices_poll.iter().position(|c| c == &firmware_mode) {
                        let index = index as u32;
                        profile_guard_poll.set(true);
                        mode_drop_poll.set_selected(index);
                        mode_drop_poll.set_subtitle(profile_blurb(&firmware_mode));
                        tip(&mode_drop_poll, profile_tooltip(&firmware_mode));
                        last_ok_poll.set(index);
                        profile_guard_poll.set(false);
                        legion_core::config::remember_platform_profile(&firmware_mode);
                    }
                }
            }

            if let Some(s) = poll.sensors {
                let c = if s.ec_cpu > 0.0 { s.ec_cpu } else { s.cpu_temp };
                cpu_v.set_text(&format!("{c:.0} °C"));
                tint_temp(&cpu_chip_c, c);
                cpu_d.set_text(&match poll.cpu_w {
                    Some(w) => format!("{:.0}% · {w:.0} W", poll.cpu_pct),
                    None => format!("{:.0}%", poll.cpu_pct),
                });

                let g = if s.dgpu_temp < 0.0 {
                    s.ec_gpu
                } else {
                    s.dgpu_temp.max(s.ec_gpu)
                };
                gpu_v.set_text(&format!("{g:.0} °C"));
                tint_temp(&gpu_chip_c, g);
                if s.dgpu_power >= 0.0 {
                    gpu_d.set_text(&format!("{:.0}% · {:.0} W", poll.gpu_pct, s.dgpu_power));
                } else {
                    gpu_d.set_text(&format!("{:.0}%", poll.gpu_pct));
                }

                set_fan_metrics_from_sensors(&fan_metric_labels, &s);
                bat_v.set_text(&format!("{}%", s.battery_pct));
                bat_d.set_text(&friendly_profile(&s.profile));
            } else {
                if let Some((pct, status)) = &poll.local_battery {
                    bat_v.set_text(&format!("{pct}%"));
                    bat_d.set_text(status);
                }
                if let Some(local) = &poll.local {
                    let c = if local.ec_cpu > 0.0 {
                        local.ec_cpu
                    } else {
                        local.cpu_temp
                    };
                    if c > 0.0 {
                        cpu_v.set_text(&format!("{c:.0} °C"));
                        tint_temp(&cpu_chip_c, c);
                        cpu_d.set_text(&format!("{:.0}%", poll.cpu_pct));
                    }
                    let g = if local.dgpu_temp < 0.0 {
                        local.ec_gpu
                    } else {
                        local.dgpu_temp.max(local.ec_gpu)
                    };
                    if g > 0.0 {
                        gpu_v.set_text(&format!("{g:.0} °C"));
                        tint_temp(&gpu_chip_c, g);
                        gpu_d.set_text(&format!("{:.0}%", poll.gpu_pct));
                    }
                    set_fan_metrics_from_sensors(&fan_metric_labels, local);
                }
            }
        }
        glib::ControlFlow::Continue
    });

    page
}

#[derive(Clone)]
struct CurveOptimizerUi {
    status_row: adw::ActionRow,
    offset_scale: gtk::Scale,
    offset_value: gtk::Label,
    apply_button: gtk::Button,
    reset_button: gtk::Button,
    install_button: gtk::Button,
    refresh_button: gtk::Button,
    startup_switch: adw::SwitchRow,
    persistence_suppress: Rc<Cell<bool>>,
}

fn offsets_text(values: &[i16]) -> String {
    match values.first().copied() {
        None => "—".into(),
        Some(first) if values.iter().all(|value| *value == first) => {
            format!("All cores: {first}")
        }
        Some(_) => "Mixed".into(),
    }
}

fn update_curve_optimizer_ui(
    ui: &CurveOptimizerUi,
    result: Result<DaemonResponse, String>,
) -> Result<(), String> {
    let status = match result {
        Ok(DaemonResponse::CurveOptimizer(status)) => status,
        Ok(DaemonResponse::Error(error)) => return Err(error),
        Ok(other) => return Err(format!("Unexpected daemon response: {other:?}")),
        Err(error) => return Err(error),
    };

    ui.refresh_button.set_sensitive(true);
    ui.status_row.set_subtitle(&status.reason);
    ui.install_button.set_visible(!status.available);
    ui.apply_button.set_sensitive(status.available);
    ui.reset_button.set_sensitive(status.available);
    ui.offset_scale.set_sensitive(status.available);
    // Gate startup: must verify an offset first — daemon enforces this too, but UI should be honest early.
    // Keep switch sensitive only if available; subtitle hint is handled in persistence UI refresh.
    ui.startup_switch.set_sensitive(status.available);

    if !status.available {
        return Ok(());
    }

    let current_val = status
        .current
        .first()
        .copied()
        .filter(|v| status.current.iter().all(|x| x == v));
    let baseline_val = status
        .boot_baseline
        .first()
        .copied()
        .filter(|v| status.boot_baseline.iter().all(|x| x == v));
    let prev = status.previous.filter(|p| Some(*p) != current_val);
    // Keep this unambiguous: the applied offset is live (the slider mirrors
    // it); baseline/history are only context. Never print the same number
    // twice — "reset -4 · previous -4" reads like the value reverted.
    let mut subtitle = format!(
        "Applied {}",
        offsets_text(&status.current).replace("All cores: ", "")
    );
    if let Some(b) = baseline_val {
        if Some(b) != current_val {
            subtitle.push_str(&format!(" · Reset goes back to {b}"));
        }
    }
    if let Some(p) = prev {
        if Some(p) != baseline_val {
            subtitle.push_str(&format!(" · was {p} before"));
        }
    }
    ui.status_row.set_subtitle(&subtitle);
    ui.offset_scale
        .set_range(status.minimum as f64, status.maximum as f64);
    if let Some(current) = status
        .current
        .first()
        .copied()
        .filter(|first| status.current.iter().all(|value| value == first))
    {
        ui.offset_scale.set_value(current as f64);
        ui.offset_value.set_text(&current.to_string());
    } else {
        ui.offset_value.set_text("Mixed");
    }
    Ok(())
}

fn update_curve_optimizer_persistence_ui(
    ui: &CurveOptimizerUi,
    result: Result<DaemonResponse, String>,
) -> Result<(), String> {
    let status = match result {
        Ok(DaemonResponse::CurveOptimizerPersistence(status)) => status,
        Ok(DaemonResponse::Error(error)) => return Err(error),
        Ok(other) => return Err(format!("Unexpected daemon response: {other:?}")),
        Err(error) => return Err(error),
    };
    ui.persistence_suppress.set(true);
    ui.startup_switch.set_active(status.enabled);
    ui.startup_switch.set_sensitive(!status.recovery_blocked);
    if status.recovery_blocked {
        ui.startup_switch
            .set_subtitle("Disabled after an interrupted validation window");
    } else if status.enabled {
        ui.startup_switch.set_subtitle(&format!(
            "Applies {} after a 60-second delay",
            status.offset
        ));
    } else {
        // Hint flow: must Apply & verify first before enabling startup.
        let hint = if ui.offset_scale.is_sensitive() {
            "Off · Apply and verify an offset first"
        } else {
            "Off"
        };
        ui.startup_switch.set_subtitle(hint);
    }
    ui.persistence_suppress.set(false);
    Ok(())
}

fn refresh_curve_optimizer_persistence(
    ui: &CurveOptimizerUi,
    toast_overlay: Option<&adw::ToastOverlay>,
) {
    let ui = ui.clone();
    let overlay = toast_overlay.cloned();
    run_daemon_command_async(DaemonCommand::GetCurveOptimizerPersistence, move |result| {
        if let Err(error) = update_curve_optimizer_persistence_ui(&ui, result) {
            ui.persistence_suppress.set(true);
            ui.startup_switch.set_active(false);
            ui.startup_switch.set_sensitive(false);
            ui.startup_switch.set_subtitle(&error);
            ui.persistence_suppress.set(false);
            if let Some(overlay) = overlay {
                toast_error(&overlay, &error);
            }
        }
    });
}

fn refresh_curve_optimizer(ui: &CurveOptimizerUi, toast_overlay: Option<&adw::ToastOverlay>) {
    ui.refresh_button.set_sensitive(false);
    refresh_curve_optimizer_persistence(ui, toast_overlay);
    let ui = ui.clone();
    let overlay = toast_overlay.cloned();
    run_daemon_command_async(DaemonCommand::GetCurveOptimizer, move |result| {
        if let Err(error) = update_curve_optimizer_ui(&ui, result) {
            ui.refresh_button.set_sensitive(true);
            ui.status_row.set_subtitle(&error);
            ui.apply_button.set_sensitive(false);
            ui.reset_button.set_sensitive(false);
            ui.offset_scale.set_sensitive(false);
            if let Some(overlay) = overlay {
                toast_error(&overlay, &error);
            }
        }
    });
}

fn set_curve_optimizer_persistence_async(
    ui: CurveOptimizerUi,
    overlay: adw::ToastOverlay,
    enabled: bool,
    offset: i16,
) {
    ui.startup_switch.set_sensitive(false);
    let refresh_ui = ui.clone();
    let refresh_overlay = overlay.clone();
    run_daemon_command_async(
        DaemonCommand::SetCurveOptimizerPersistence {
            enabled,
            offset,
            acknowledge: true,
        },
        move |result| match update_curve_optimizer_persistence_ui(&ui, result) {
            Ok(()) => {
                if enabled {
                    toast_ok(&overlay, "Startup undervolt enabled");
                } else {
                    toast_ok(&overlay, "Startup undervolt disabled");
                }
            }
            Err(error) => {
                toast_error(&overlay, &error);
                refresh_curve_optimizer_persistence(&refresh_ui, Some(&refresh_overlay));
            }
        },
    );
}

fn build_curve_optimizer(toast_overlay: &adw::ToastOverlay) -> adw::PreferencesGroup {
    let group = pref_group("Curve Optimizer", None);
    tip(
        &group,
        "All-core CPU offset via ryzen_smu. Unstable values can crash the system.",
    );

    let status_row = adw::ActionRow::builder()
        .title("AMD backend")
        .subtitle("Checking…")
        .activatable(false)
        .build();
    let install_button = primary_button_tip(
        "Install backend",
        Some("Installs the bundled ryzen_smu driver through PolicyKit and DKMS"),
    );
    install_button.set_visible(false);
    let refresh_button = gtk::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text("Refresh")
        .valign(Align::Center)
        .build();
    status_row.add_suffix(&install_button);
    status_row.add_suffix(&refresh_button);
    group.add(&status_row);

    let offset_row = adw::ActionRow::builder()
        .title("All-core offset")
        .subtitle("")
        .activatable(false)
        .build();
    tip(
        &offset_row,
        "All-core Curve Optimizer offset (-30..0, more negative = more undervolt)",
    );
    let offset_value = gtk::Label::new(Some("—"));
    offset_value.add_css_class("numeric");
    offset_value.set_width_chars(4);
    let adjustment = gtk::Adjustment::new(0.0, -30.0, 0.0, 1.0, 5.0, 0.0);
    let offset_scale = gtk::Scale::new(Orientation::Horizontal, Some(&adjustment));
    offset_scale.set_draw_value(false);
    offset_scale.set_digits(0);
    offset_scale.set_hexpand(true);
    offset_scale.set_width_request(220);
    offset_scale.set_sensitive(false);
    offset_row.add_suffix(&offset_scale);
    offset_row.add_suffix(&offset_value);
    group.add(&offset_row);

    let actions_row = adw::ActionRow::builder()
        .title("Current session")
        .subtitle("Verified by firmware readback")
        .activatable(false)
        .build();
    tip(
        &actions_row,
        "Verified by firmware readback via ryzen_smu (daemon checks SMU probe before applying)",
    );
    let reset_button = gtk::Button::builder()
        .label("Reset")
        .valign(Align::Center)
        .sensitive(false)
        .build();
    reset_button.add_css_class("pill-btn");
    let apply_button = gtk::Button::builder()
        .label("Apply")
        .valign(Align::Center)
        .sensitive(false)
        .build();
    apply_button.add_css_class("destructive-action");
    apply_button.add_css_class("pill-btn");
    actions_row.add_suffix(&reset_button);
    actions_row.add_suffix(&apply_button);
    group.add(&actions_row);

    let startup_switch = adw::SwitchRow::builder()
        .title("Apply after startup")
        .subtitle("Checking…")
        .sensitive(false)
        .build();
    group.add(&startup_switch);
    let persistence_suppress = Rc::new(Cell::new(false));

    let ui = CurveOptimizerUi {
        status_row,
        offset_scale,
        offset_value,
        apply_button,
        reset_button,
        install_button,
        refresh_button,
        startup_switch,
        persistence_suppress,
    };

    let value_label = ui.offset_value.clone();
    ui.offset_scale.connect_value_changed(move |scale| {
        value_label.set_text(&(scale.value().round() as i16).to_string());
    });

    let ui_startup = ui.clone();
    let overlay = toast_overlay.clone();
    ui.startup_switch.connect_active_notify(move |row| {
        if ui_startup.persistence_suppress.get() {
            return;
        }
        let enabled = row.is_active();
        let offset = ui_startup.offset_scale.value().round() as i16;
        if !enabled {
            set_curve_optimizer_persistence_async(
                ui_startup.clone(),
                overlay.clone(),
                false,
                offset,
            );
            return;
        }

        let ui = ui_startup.clone();
        let overlay = overlay.clone();
        confirm_risk(
            row,
            "Apply undervolt after startup?",
            "Legion Control waits 60 seconds before applying it. If validation is interrupted, the next start disables it automatically.",
            "Enable",
            move |accepted| {
                if accepted {
                    set_curve_optimizer_persistence_async(ui, overlay, true, offset);
                } else {
                    ui.persistence_suppress.set(true);
                    ui.startup_switch.set_active(false);
                    ui.persistence_suppress.set(false);
                }
            },
        );
    });

    let ui_refresh = ui.clone();
    let overlay = toast_overlay.clone();
    ui.refresh_button
        .connect_clicked(move |_| refresh_curve_optimizer(&ui_refresh, Some(&overlay)));

    let ui_install = ui.clone();
    let overlay = toast_overlay.clone();
    ui.install_button.connect_clicked(move |button| {
        button.set_sensitive(false);
        button.set_label("Installing…");
        let button = button.clone();
        let ui = ui_install.clone();
        let overlay = overlay.clone();
        run_setup_helper("install-ryzen-smu", move |result| match result {
            Ok(_) => {
                button.set_label("Install backend");
                button.set_sensitive(true);
                toast_ok(&overlay, "AMD tuning backend installed");
                refresh_curve_optimizer(&ui, Some(&overlay));
            }
            Err(error) => {
                button.set_label("Install backend");
                button.set_sensitive(true);
                toast_error(&overlay, &error);
            }
        });
    });

    let ui_apply = ui.clone();
    let overlay = toast_overlay.clone();
    ui.apply_button.connect_clicked(move |button| {
        let offset = ui_apply.offset_scale.value().round() as i16;
        let parent = button.clone();
        let ui = ui_apply.clone();
        let overlay = overlay.clone();
        confirm_risk(
            &parent,
            "Apply Curve Optimizer offset?",
            &format!(
                "Apply {offset} to all CPU cores until reboot?\n\nAn unstable offset can crash the system or corrupt active work."
            ),
            &format!("Apply {offset}"),
            move |confirmed| {
                if !confirmed {
                    return;
                }
                ui.apply_button.set_sensitive(false);
                ui.apply_button.set_label("Applying…");
                let ui_done = ui.clone();
                let overlay = overlay.clone();
                run_daemon_command_async(
                    DaemonCommand::SetCurveOptimizer {
                        offset,
                        acknowledge: true,
                    },
                    move |result| {
                        ui_done.apply_button.set_label("Apply");
                        match update_curve_optimizer_ui(&ui_done, result) {
                            Ok(()) => toast_ok(&overlay, "Curve Optimizer offset verified"),
                            Err(error) => {
                                ui_done.apply_button.set_sensitive(true);
                                toast_error(&overlay, &error);
                            }
                        }
                    },
                );
            },
        );
    });

    let ui_reset = ui.clone();
    let overlay = toast_overlay.clone();
    ui.reset_button.connect_clicked(move |button| {
        let parent = button.clone();
        let ui = ui_reset.clone();
        let overlay = overlay.clone();
        confirm_risk(
            &parent,
            "Reset Curve Optimizer?",
            "Restore the values observed when the daemon started?",
            "Reset",
            move |confirmed| {
                if !confirmed {
                    return;
                }
                ui.reset_button.set_sensitive(false);
                ui.reset_button.set_label("Resetting…");
                let ui_done = ui.clone();
                let overlay = overlay.clone();
                run_daemon_command_async(
                    DaemonCommand::ResetCurveOptimizerAcknowledged { acknowledge: true },
                    move |result| {
                        ui_done.reset_button.set_label("Reset");
                        match update_curve_optimizer_ui(&ui_done, result) {
                            Ok(()) => toast_ok(&overlay, "Curve Optimizer baseline restored"),
                            Err(error) => {
                                ui_done.reset_button.set_sensitive(true);
                                toast_error(&overlay, &error);
                            }
                        }
                    },
                );
            },
        );
    });

    refresh_curve_optimizer(&ui, None);
    group
}

fn build_cpu_features_page(toast_overlay: &adw::ToastOverlay, gate: &DaemonGate) -> gtk::Box {
    let page = page_lede("");
    let features = build_cpu_features(toast_overlay);
    gate.track(&features);
    page.append(&features);
    page
}

fn build_cpu_power_page(
    _toast_overlay: &adw::ToastOverlay,
    go_home: &Rc<dyn Fn(&'static str, &'static str)>,
) -> gtk::Box {
    let page = page_lede("");
    let mode = legion_core::profile::current();

    // Guidance row — live wording, one clear way out.
    let guide = pref_group("Power limits", None);
    tip(
        &guide,
        "CPU PPT and GPU power sliders are edited on Home — they unlock when Power mode is Custom",
    );
    let lock_row = adw::ActionRow::builder()
        .title(if mode == "custom" {
            "Custom mode is active"
        } else {
            "Locked outside Custom mode"
        })
        .subtitle(if mode == "custom" {
            "Adjust the watts below on Home → Power mode"
        } else {
            "Set Power mode on Home to Custom to unlock these sliders"
        })
        .activatable(false)
        .build();
    tip(
        &lock_row,
        "Firmware PPT/attribute writes are only accepted while the EC is in Custom mode — other modes use firmware defaults",
    );
    let go_home = go_home.clone();
    let go_home_btn = primary_button_tip(
        "Edit on Home",
        Some("Opens Home with the Power mode picker and the Custom-watt sliders"),
    );
    go_home_btn.connect_clicked(move |_| {
        go_home("overview", "Home");
    });
    lock_row.add_suffix(&go_home_btn);
    guide.add(&lock_row);
    page.append(&guide);

    // Live preview of the same sliders Home shows — greyed out here. Values
    // mirror the firmware read so the page informs instead of pointing.
    let preview = pref_group("Custom watts", None);
    tip(
        &preview,
        "Read-only mirror of the Custom-mode limits — edit them on Home",
    );
    for lim in legion_core::profile::all_ppt_limits() {
        let row = adw::ActionRow::builder()
            .title(lim.label)
            .activatable(false)
            .build();
        tip(&row, &format!("{} · {}–{} W", lim.label, lim.min, lim.max));
        let val = gtk::Label::new(Some(&format!("{} W", lim.current)));
        val.add_css_class("dim-label");
        val.add_css_class("numeric");
        row.add_suffix(&val);
        let adj = gtk::Adjustment::new(
            lim.current as f64,
            lim.min as f64,
            lim.max as f64,
            1.0,
            5.0,
            0.0,
        );
        let scale = gtk::Scale::new(Orientation::Horizontal, Some(&adj));
        scale.set_draw_value(false);
        scale.set_hexpand(true);
        scale.set_width_request(160);
        tip(
            &scale,
            &format!("{} · {}–{} W", lim.label, lim.min, lim.max),
        );
        row.add_suffix(&scale);
        preview.add(&row);
    }
    preview.set_sensitive(false);
    page.append(&preview);
    page
}

fn autostart_enabled() -> bool {
    dirs::config_dir()
        .map(|d| {
            d.join("autostart")
                .join("com.encomjp.legion-settings.desktop")
        })
        .is_some_and(|p| p.exists())
}

fn set_autostart(enabled: bool) -> Result<(), String> {
    let Some(dir) = dirs::config_dir().map(|d| d.join("autostart")) else {
        return Err("Cannot locate autostart directory".into());
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create autostart dir: {e}"))?;
    let dest = dir.join("com.encomjp.legion-settings.desktop");
    if enabled {
        // Resolve installed .desktop first (survives packaging), fall back to source tree.
        let installed = [
            "/usr/local/share/applications/com.encomjp.legion-settings.desktop",
            "/usr/share/applications/com.encomjp.legion-settings.desktop",
        ];
        let mut content: Option<String> = None;
        for p in installed {
            if let Ok(s) = std::fs::read_to_string(p) {
                content = Some(s);
                break;
            }
        }
        let mut content = content.unwrap_or_else(|| {
            std::fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("data/gui/com.encomjp.legion-settings.desktop"),
            )
            .unwrap_or_else(|_| {
                "[Desktop Entry]\nName=Legion Control\nExec=legion-settings\nIcon=com.encomjp.legion-settings\nType=Application\n".into()
            })
        });
        // Ensure it autostarts hidden to tray — no window pop on login.
        if !content.contains("Exec=") {
            content.push_str("\nExec=legion-settings --hidden\n");
        } else if !content.contains("--hidden") {
            content = content.replace("Exec=legion-settings", "Exec=legion-settings --hidden");
        }
        if !content.contains("X-GNOME-Autostart-enabled") {
            if !content.ends_with('\n') {
                content.push('\n');
            }
            content.push_str("X-GNOME-Autostart-enabled=true\n");
        }
        if !content.contains("Hidden=") {
            content.push_str("Hidden=false\n");
        }
        std::fs::write(&dest, content).map_err(|e| format!("Cannot write autostart entry: {e}"))?;
    } else {
        let _ = std::fs::remove_file(&dest);
    }
    Ok(())
}

fn build_cpu_tuning_page(toast_overlay: &adw::ToastOverlay, gate: &DaemonGate) -> gtk::Box {
    let page = page_lede("");
    // Cards on top — squares/overview style, then tuning controls below. Tooltips (hover) carry the how-to.
    let thermal = build_thermal_card(toast_overlay, gate);
    let co = build_curve_optimizer(toast_overlay);
    // Autostart as a small row on this Tuning tab (hover tip explains it).
    let autostart_row = adw::SwitchRow::builder()
        .title("Launch at login")
        .subtitle(if autostart_enabled() {
            "On · opens on login"
        } else {
            "Off"
        })
        .active(autostart_enabled())
        .build();
    tip(
        &autostart_row,
        "Adds Legion Control to Desktop autostart (~/.config/autostart) so tuning controls are available after login. System daemon (fans/profile) starts separately via systemd.",
    );
    let autostart_toast = toast_overlay.clone();
    autostart_row.connect_active_notify(move |row| {
        let on = row.is_active();
        match set_autostart(on) {
            Ok(()) => {
                row.set_subtitle(if on { "On · opens on login" } else { "Off" });
                toast_ok(
                    &autostart_toast,
                    if on {
                        "Autostart enabled"
                    } else {
                        "Autostart disabled"
                    },
                );
            }
            Err(e) => {
                row.set_active(!on);
                toast_error(&autostart_toast, &e);
            }
        }
    });
    let autostart_group = pref_group("Startup", None);
    tip(
        &autostart_group,
        "Login autostart for the Settings app (user session). The root daemon autostarts via systemd regardless.",
    );
    autostart_group.add(&autostart_row);
    // Order: chips-first tuning (thermal) already has its own chips on top internally;
    // then undervolt + stability + autostart.
    page.append(&thermal);
    page.append(&co);
    page.append(&build_stability_group(toast_overlay));
    page.append(&autostart_group);
    page
}

fn build_stability_group(toast_overlay: &adw::ToastOverlay) -> adw::PreferencesGroup {
    // Reuse the existing stability page's group — extract the group only, no page wrapper.
    let page = build_cpu_stability_page(toast_overlay);
    // build_cpu_stability_page returns a gtk::Box with one PreferencesGroup child; unwrap it.
    // Fallback: build inline if structure changes — keep a minimal group.
    if let Some(child) = page.first_child() {
        if let Ok(group) = child.clone().downcast::<adw::PreferencesGroup>() {
            return group;
        }
    }
    // Fallback recompose — should never hit.
    let g = pref_group("Stability test", None);
    let row = adw::ActionRow::builder()
        .title("Stability test")
        .subtitle("5 minutes")
        .build();
    g.add(&row);
    g
}

const STABILITY_TEST_SECS: u64 = 300;

enum StabilityEvent {
    Progress(u64),
    Finished { cancelled: bool, errors: u64 },
}

fn stability_memory_pass(seed: u64, memory: &mut [u64]) -> bool {
    for (index, value) in memory.iter_mut().enumerate() {
        let mixed = seed
            .wrapping_add((index as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15))
            .rotate_left((index & 63) as u32);
        *value = mixed ^ 0xa5a5_5a5a_d3c3_b4b4;
    }
    memory.iter().enumerate().all(|(index, value)| {
        let mixed = seed
            .wrapping_add((index as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15))
            .rotate_left((index & 63) as u32);
        *value == mixed ^ 0xa5a5_5a5a_d3c3_b4b4
    })
}

fn spawn_stability_test(stop: Arc<AtomicBool>, tx: mpsc::Sender<StabilityEvent>) {
    spawn_stability_test_for(stop, tx, Duration::from_secs(STABILITY_TEST_SECS));
}

fn spawn_stability_test_for(
    stop: Arc<AtomicBool>,
    tx: mpsc::Sender<StabilityEvent>,
    duration: Duration,
) {
    std::thread::spawn(move || {
        let deadline = Instant::now() + duration;
        let errors = Arc::new(AtomicU64::new(0));
        let worker_count = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1);
        let mut workers = Vec::with_capacity(worker_count);

        for worker_id in 0..worker_count {
            let stop = stop.clone();
            let errors = errors.clone();
            workers.push(std::thread::spawn(move || {
                let mut seed = 0x6a09_e667_f3bc_c909_u64 ^ worker_id as u64;
                let mut memory = vec![0_u64; 1 << 17];
                while !stop.load(Ordering::Relaxed) && Instant::now() < deadline {
                    if !stability_memory_pass(seed, &mut memory) {
                        errors.fetch_add(1, Ordering::Relaxed);
                    }
                    for _ in 0..200_000 {
                        seed ^= seed << 13;
                        seed ^= seed >> 7;
                        seed ^= seed << 17;
                        seed = seed.wrapping_mul(0xd6e8_feb8_6659_fd93);
                    }
                    std::hint::black_box(seed);
                }
            }));
        }

        let mut cancelled = false;
        while Instant::now() < deadline {
            if stop.load(Ordering::Relaxed) {
                cancelled = true;
                break;
            }
            let remaining = deadline.saturating_duration_since(Instant::now()).as_secs();
            if tx.send(StabilityEvent::Progress(remaining)).is_err() {
                stop.store(true, Ordering::Relaxed);
                cancelled = true;
                break;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        stop.store(true, Ordering::Relaxed);
        for worker in workers {
            if worker.join().is_err() {
                errors.fetch_add(1, Ordering::Relaxed);
            }
        }
        let _ = tx.send(StabilityEvent::Finished {
            cancelled,
            errors: errors.load(Ordering::Relaxed),
        });
    });
}

fn build_cpu_stability_page(toast_overlay: &adw::ToastOverlay) -> gtk::Box {
    let page = page_lede("");
    let group = pref_group("Stability test", None);
    let status = adw::ActionRow::builder()
        .title("Ready")
        .subtitle("5 minutes · all CPU threads")
        .build();
    group.add(&status);

    // One button, two roles — Start test ↔ Stop. A separate Stop button left
    // an odd half-empty actions row.
    let actions = adw::ActionRow::new();
    let run_button = gtk::Button::with_label("Start test");
    run_button.add_css_class("suggested-action");
    run_button.add_css_class("pill-btn");
    actions.add_suffix(&run_button);
    group.add(&actions);
    page.append(&group);

    let active_stop: Rc<RefCell<Option<Arc<AtomicBool>>>> = Rc::new(RefCell::new(None));
    let running = {
        let run_button = run_button.clone();
        let status = status.clone();
        Rc::new(move |on: bool| {
            if on {
                run_button.set_label("Stop");
                run_button.remove_css_class("suggested-action");
                run_button.add_css_class("destructive-action");
                status.set_title("Testing…");
                status.set_subtitle("5:00 remaining");
            } else {
                run_button.set_label("Start test");
                run_button.remove_css_class("destructive-action");
                run_button.add_css_class("suggested-action");
            }
        })
    };

    let stop_slot = active_stop.clone();
    let running_start = running.clone();
    let overlay = toast_overlay.clone();
    run_button.connect_clicked(move |_| {
        // While a test runs the same button requests the stop.
        if let Some(stop) = stop_slot.borrow().as_ref() {
            stop.store(true, Ordering::Relaxed);
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        *stop_slot.borrow_mut() = Some(stop.clone());
        running_start(true);

        let (tx, rx) = mpsc::channel();
        spawn_stability_test(stop, tx);

        let stop_slot = stop_slot.clone();
        let status = status.clone();
        let running = running_start.clone();
        let overlay = overlay.clone();
        glib::timeout_add_local(Duration::from_millis(250), move || {
            while let Ok(event) = rx.try_recv() {
                match event {
                    StabilityEvent::Progress(seconds) => {
                        status.set_subtitle(&format!(
                            "{}:{:02} remaining",
                            seconds / 60,
                            seconds % 60
                        ));
                    }
                    StabilityEvent::Finished { cancelled, errors } => {
                        *stop_slot.borrow_mut() = None;
                        running(false);
                        if cancelled {
                            status.set_title("Stopped");
                            status.set_subtitle("No result");
                        } else if errors == 0 {
                            status.set_title("Quick test passed");
                            status.set_subtitle("No errors found in this 5-minute run");
                            toast_ok(&overlay, "CPU stability test passed");
                        } else {
                            status.set_title("Errors detected");
                            status.set_subtitle("Reset the undervolt and test again");
                            toast_error(&overlay, "CPU stability errors detected");
                        }
                        return glib::ControlFlow::Break;
                    }
                }
            }
            glib::ControlFlow::Continue
        });
    });

    page
}

/// Build Custom-mode CPU PPT + GPU AC power sliders and attach them to `page`.
#[allow(clippy::too_many_arguments)]
fn attach_custom_ppt_group(
    page: &gtk::Box,
    toast_overlay: &adw::ToastOverlay,
    apply_queue: &ApplyQueue,
    gate: &DaemonGate,
    drop: &adw::ComboRow,
    choices: &[String],
    current: &str,
    ppt_group_slot: &Rc<RefCell<Option<adw::PreferencesGroup>>>,
    ppt_scales_slot: &PptScales,
    ppt_suppress_slot: &Rc<Cell<bool>>,
) {
    let caps = legion_core::device::detect().capabilities;
    let peak_tgp = caps.peak_gpu_w.unwrap_or(175);
    let peak_src = caps.peak_gpu_source;
    let ppt_group = pref_group(
        "Custom power limits",
        Some(&format!(
            "Only active in Custom mode. Peak GPU TGP on this laptop is {peak_tgp} W (Performance / Max). \
             The GPU AC power slider below is a separate BIOS knob (firmware-capped)."
        )),
    );
    tip(
        &ppt_group,
        &format!(
            "CPU PPT + GPU AC processing-power target. Peak GPU TGP is {peak_tgp} W ({peak_src}) in Performance/Max — \
             the AC offset attribute max is set by the BIOS (often 130 W), not the absolute TGP."
        ),
    );

    let peak_row = adw::ActionRow::builder()
        .title("Peak GPU TGP (this laptop)")
        .subtitle(format!(
            "{peak_tgp} W — {peak_src} · already available in Performance and Max Power"
        ))
        .activatable(false)
        .build();
    tip(
        &peak_row,
        &format!(
            "Detected peak is {peak_tgp} W ({peak_src}). Performance and Max Power unlock that ceiling. \
             Custom mode uses separate firmware attributes; the AC power target max is BIOS-limited."
        ),
    );
    ppt_group.add(&peak_row);

    ppt_scales_slot.borrow_mut().clear();
    for lim in legion_core::profile::all_ppt_limits() {
        let row = adw::ActionRow::builder()
            .title(lim.label)
            .activatable(false)
            .build();
        let ppt_tip = match lim.id {
            "ppt_pl1_spl" => format!(
                "Sustained power limit (SPL) — long-term CPU watts · range {}–{} W",
                lim.min, lim.max
            ),
            "ppt_pl2_sppt" => format!(
                "Slow boost (SPPT) — medium burst · range {}–{} W",
                lim.min, lim.max
            ),
            "ppt_pl3_fppt" => format!(
                "Peak burst (FPPT) — short turbo · range {}–{} W",
                lim.min, lim.max
            ),
            "ppt_cpu_cl" => format!(
                "CPU cross-load share · range {}–{} W",
                lim.min, lim.max
            ),
            "gpu_nv_ac_offset" => format!(
                "GPU AC total processing power target (BIOS attribute). Firmware range {}–{} W. \
                 This is not the absolute {peak_tgp} W NVIDIA TGP — Performance/Max already use {peak_tgp} W.",
                lim.min, lim.max
            ),
            "gpu_nv_ctgp" => format!(
                "GPU configurable TGP (cTGP) · range {}–{} W",
                lim.min, lim.max
            ),
            "gpu_nv_ppab" => format!(
                "GPU PPAB boost · range {}–{} W",
                lim.min, lim.max
            ),
            "gpu_nv_cpu_boost" => format!(
                "GPU↔CPU dynamic boost share · range {}–{} W",
                lim.min, lim.max
            ),
            _ => format!("{} · {}–{} W", lim.label, lim.min, lim.max),
        };
        tip(&row, &ppt_tip);

        let val = gtk::Label::new(Some(&format!("{} W", lim.current)));
        val.add_css_class("dim-label");
        val.add_css_class("numeric");
        tip(&val, &ppt_tip);
        row.add_suffix(&val);

        let adj = gtk::Adjustment::new(
            lim.current as f64,
            lim.min as f64,
            lim.max as f64,
            1.0,
            5.0,
            0.0,
        );
        let scale = gtk::Scale::new(Orientation::Horizontal, Some(&adj));
        scale.set_draw_value(false);
        scale.set_hexpand(true);
        scale.set_width_request(160);
        tip(&scale, &ppt_tip);
        row.add_suffix(&scale);
        ppt_group.add(&row);

        let overlay = toast_overlay.clone();
        let queue = apply_queue.clone();
        let id = lim.id.to_string();
        let lim_max = lim.max;
        let lim_label = lim.label.to_string();
        let val_l = val.clone();
        let drop_c = drop.clone();
        let choices_c = choices.to_vec();
        let suppress = ppt_suppress_slot.clone();
        let ppt_warned = Rc::new(Cell::new(false));
        let scale_c = scale.clone();
        scale.connect_value_changed(move |s| {
            if suppress.get() {
                return;
            }
            let v = s.value().round() as u32;
            val_l.set_text(&format!("{v} W"));
            let warn_at = (lim_max as f64 * 0.92).round() as u32;
            if v >= warn_at && !ppt_warned.get() {
                ppt_warned.set(true);
                let scale_r = scale_c.clone();
                let suppress_r = suppress.clone();
                let prev = (warn_at.saturating_sub(5)).max(lim_max / 2);
                let overlay = overlay.clone();
                let queue = queue.clone();
                let id = id.clone();
                let lim_label = lim_label.clone();
                let choices_c = choices_c.clone();
                let drop_c = drop_c.clone();
                confirm_risk(
                    s,
                    "High power limit",
                    &format!(
                        "{lim_label} at {v} W is near the firmware maximum ({lim_max} W).\n\n\
                         Sustained high limits increase heat and fan noise.\n\n\
                         Continue only if cooling is strong."
                    ),
                    "Use high limit",
                    move |ok| {
                        if !ok {
                            suppress_r.set(true);
                            scale_r.set_value(prev as f64);
                            suppress_r.set(false);
                            return;
                        }
                        ensure_custom_then_ppt(&overlay, &drop_c, &choices_c, &queue, &id, v);
                    },
                );
                return;
            }
            ensure_custom_then_ppt(&overlay, &drop_c, &choices_c, &queue, &id, v);
        });
        ppt_scales_slot
            .borrow_mut()
            .push((lim.id.to_string(), scale, val));
    }

    let ppt_visible = current == "custom" && !ppt_scales_slot.borrow().is_empty();
    ppt_group.set_visible(ppt_visible);
    gate.track(&ppt_group);
    if !ppt_scales_slot.borrow().is_empty() {
        page.append(&ppt_group);
    }
    *ppt_group_slot.borrow_mut() = Some(ppt_group);
}

fn set_fan_metric(value: &gtk::Label, detail: &gtk::Label, rpm: u32, target: u32) {
    if target == 0 {
        if rpm == 0 {
            value.set_text("Auto");
            detail.set_text("Firmware curve");
        } else {
            value.set_text(&format!("{rpm}"));
            detail.set_text("Auto");
        }
    } else if rpm == 0 {
        value.set_text(&format!("~{target}"));
        detail.set_text("Manual target");
    } else {
        value.set_text(&format!("{rpm}"));
        detail.set_text(&format!("→ {target} rpm"));
    }
}

fn set_fan_metrics_from_sensors(
    labels: &[(u8, gtk::Label, gtk::Label)],
    s: &legion_core::sensors::SensorReadings,
) {
    for (id, value, detail) in labels {
        let (rpm, target) = match id {
            1 => (s.fan1_rpm, s.fan1_target),
            2 => (s.fan2_rpm, s.fan2_target),
            4 => (s.fan4_rpm, s.fan4_target),
            _ => (
                legion_core::fans::read_rpm(*id).unwrap_or(0),
                legion_core::fans::read_target(*id).unwrap_or(0),
            ),
        };
        set_fan_metric(value, detail, rpm, target);
    }
}

fn build_profiles_page(
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
    let picker = string_combo_row("Profile", "Pick a saved preset", &labels, active);
    tip(&picker, "Presets are stored in ~/.config/legion-control/");
    group.add(&picker);

    let entry = gtk::Entry::builder()
        .placeholder_text("Name for new profile")
        .hexpand(true)
        .build();
    tip(&entry, "Type a name, then Save current");
    let entry_row = adw::ActionRow::builder()
        .title("New name")
        .subtitle("Name for this profile")
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
        .subtitle("Re-apply power, fans, charge, and lighting when the app starts")
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

fn tint_temp(chip: &gtk::Box, temp: f64) {
    chip.remove_css_class("hot");
    chip.remove_css_class("warm");
    if temp >= 90.0 {
        chip.add_css_class("hot");
    } else if temp >= 78.0 {
        chip.add_css_class("warm");
    }
}

fn apply_platform_profile(
    name: &str,
    overlay: &adw::ToastOverlay,
    ppt_box: &adw::PreferencesGroup,
    ppt_scales: &PptScales,
    ppt_suppress: &Rc<Cell<bool>>,
) {
    let overlay = overlay.clone();
    let ppt_box = ppt_box.clone();
    let ppt_scales = ppt_scales.clone();
    let ppt_suppress = ppt_suppress.clone();
    let name = name.to_string();
    run_daemon_command_async(
        DaemonCommand::SetProfile(name.clone()),
        move |result| match result {
            Ok(DaemonResponse::Ok) => {
                legion_core::config::remember_platform_profile(&name);
                let show = name == "custom" && !ppt_scales.borrow().is_empty();
                ppt_box.set_visible(show);
                if show {
                    ppt_suppress.set(true);
                    for lim in legion_core::profile::all_ppt_limits() {
                        for (id, scale, label) in ppt_scales.borrow().iter() {
                            if id == lim.id {
                                scale.set_value(lim.current as f64);
                                label.set_text(&format!("{} W", lim.current));
                            }
                        }
                    }
                    ppt_suppress.set(false);
                }
                toast_ok(
                    &overlay,
                    &format!("Switched to {}", friendly_profile(&name)),
                );
            }
            Ok(DaemonResponse::Error(e)) => toast_error(&overlay, &e),
            Err(e) => toast_error(&overlay, &e),
            _ => {}
        },
    );
}

const MAX_POWER_WARNING: &str = "\
Max Power (Extreme) pushes the highest turbo the BIOS allows. Without strong \
cooling the laptop can overheat, throttle, or shut down.

Continue only if you accept the risk.";

const CUSTOM_POWER_WARNING: &str = "\
Custom mode unlocks manual CPU and GPU power limits. Raising them increases \
heat and fan noise. Inadequate cooling can cause throttling or shutdown.";

fn confirm_max_power(
    parent: &impl glib::object::IsA<gtk::Widget>,
    done: impl FnOnce(bool) + 'static,
) {
    confirm_risk(
        parent,
        "Max Power can overheat this laptop",
        MAX_POWER_WARNING,
        "Use Max Power anyway",
        done,
    );
}

fn confirm_custom_power(
    parent: &impl glib::object::IsA<gtk::Widget>,
    done: impl FnOnce(bool) + 'static,
) {
    confirm_risk(
        parent,
        "Custom power limits",
        CUSTOM_POWER_WARNING,
        "Use Custom",
        done,
    );
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

fn ensure_custom_then_ppt(
    overlay: &adw::ToastOverlay,
    drop: &adw::ComboRow,
    choices: &[String],
    queue: &ApplyQueue,
    id: &str,
    watts: u32,
) {
    let cur = legion_core::profile::current();
    if cur != "custom" {
        if let Some(idx) = choices.iter().position(|c| c == "custom") {
            drop.set_selected(idx as u32);
        }
        let overlay = overlay.clone();
        let queue = queue.clone();
        let id = id.to_string();
        run_daemon_command_async(DaemonCommand::SetProfile("custom".into()), move |result| {
            match result {
                Ok(DaemonResponse::Ok) => {
                    queue.set_fw_attr(&id, watts.to_string());
                    legion_core::config::remember_ppt(&id, watts);
                }
                Ok(DaemonResponse::Error(e)) => toast_error(&overlay, &e),
                Err(e) => toast_error(&overlay, &e),
                _ => {}
            }
        });
        return;
    }
    queue.set_fw_attr(id, watts.to_string());
    legion_core::config::remember_ppt(id, watts);
}

fn build_cpu_features(toast_overlay: &adw::ToastOverlay) -> adw::PreferencesGroup {
    let group = pref_group("CPU features", None);

    // Hyperthreading / SMT
    if legion_core::cpu::smt_available() {
        let active = legion_core::cpu::smt_active().unwrap_or(true);
        let smt = adw::SwitchRow::builder()
            .title("Hyperthreading (SMT)")
            .subtitle(legion_core::cpu::smt_summary())
            .active(active)
            .build();
        tip(&smt, "On = all logical CPUs. Off = one thread per core.");
        let overlay = toast_overlay.clone();
        let guard = Rc::new(Cell::new(false));
        let guard_n = guard.clone();
        smt.connect_active_notify(move |row| {
            if guard_n.get() {
                return;
            }
            let on = row.is_active();
            if !on {
                let row_r = row.clone();
                let overlay = overlay.clone();
                let guard = guard_n.clone();
                let n = legion_core::cpu::logical_cpus().max(2);
                let half = (n / 2).max(1);
                confirm_risk(
                    row,
                    "Turn off hyperthreading?",
                    &format!(
                        "Disabling SMT halves logical CPUs (about {n} → {half} on this laptop).\n\n\
                         A few games gain 1% lows; most workloads get slower. Changes apply immediately."
                    ),
                    "Disable SMT",
                    move |ok| {
                        if !ok {
                            guard.set(true);
                            row_r.set_active(true);
                            guard.set(false);
                            return;
                        }
                        apply_smt(&overlay, &row_r, false, &guard);
                    },
                );
                // Revert visually until confirmed — SwitchRow already flipped.
                guard_n.set(true);
                row.set_active(true);
                guard_n.set(false);
                return;
            }
            apply_smt(&overlay, row, true, &guard_n);
        });
        group.add(&smt);
    }

    if legion_core::cpu::boost_available() {
        let on = legion_core::cpu::boost_enabled().unwrap_or(true);
        let boost = adw::SwitchRow::builder()
            .title("CPU boost (turbo)")
            .subtitle(if on {
                "Frequency boost allowed"
            } else {
                "Locked to base clocks — cooler and quieter"
            })
            .active(on)
            .build();
        tip(
            &boost,
            "Off caps turbo for cooler/quieter running. On allows boost under load — runs hotter.",
        );
        let overlay = toast_overlay.clone();
        let guard = Rc::new(Cell::new(false));
        let guard_n = guard.clone();
        boost.connect_active_notify(move |row| {
            if guard_n.get() {
                return;
            }
            let want = row.is_active();
            // Enabling boost while on Max Power — extra nudge.
            if want && legion_core::profile::current() == "max-power" {
                let row_r = row.clone();
                let overlay = overlay.clone();
                let guard = guard_n.clone();
                confirm_risk(
                    row,
                    "Boost + Max Power",
                    "CPU boost with Max Power / Extreme pushes the highest clocks and heat. \
                     Only continue with strong cooling.",
                    "Enable",
                    move |ok| {
                        if !ok {
                            guard.set(true);
                            row_r.set_active(false);
                            guard.set(false);
                            return;
                        }
                        apply_boost(&overlay, &row_r, true, &guard);
                    },
                );
                guard_n.set(true);
                row.set_active(false);
                guard_n.set(false);
                return;
            }
            apply_boost(&overlay, row, want, &guard_n);
        });
        group.add(&boost);
    }

    group
}

fn apply_smt(overlay: &adw::ToastOverlay, row: &adw::SwitchRow, on: bool, guard: &Rc<Cell<bool>>) {
    let overlay = overlay.clone();
    let row = row.clone();
    let guard = guard.clone();
    let revert = {
        let row = row.clone();
        let guard = guard.clone();
        move |overlay: &adw::ToastOverlay, msg: &str| {
            guard.set(true);
            row.set_active(!on);
            guard.set(false);
            toast_error(overlay, msg);
        }
    };
    run_daemon_command_async(DaemonCommand::SetSmt(on), move |result| match result {
        Ok(DaemonResponse::Ok) => {
            let row_c = row.clone();
            let guard_c = guard.clone();
            run_daemon_command_async(DaemonCommand::GetSmt, move |r| {
                if let Ok(DaemonResponse::Smt {
                    active,
                    logical_cpus,
                    ..
                }) = r
                {
                    guard_c.set(true);
                    row_c.set_active(active);
                    row_c.set_subtitle(&format!(
                        "{} · {logical_cpus} logical CPUs",
                        if active { "On" } else { "Off" }
                    ));
                    guard_c.set(false);
                }
            });
            toast_ok(
                &overlay,
                if on {
                    "Hyperthreading on"
                } else {
                    "Hyperthreading off"
                },
            );
        }
        Ok(DaemonResponse::Error(e)) => revert(&overlay, &e),
        Err(e) if is_version_skew_error(&e) => {
            revert(
                &overlay,
                "Update the control service for SMT (reinstall daemon)",
            );
        }
        Err(e) => revert(&overlay, &e),
        _ => {}
    });
}

fn apply_boost(
    overlay: &adw::ToastOverlay,
    row: &adw::SwitchRow,
    on: bool,
    guard: &Rc<Cell<bool>>,
) {
    let overlay = overlay.clone();
    let row = row.clone();
    let guard = guard.clone();
    let revert = {
        let row = row.clone();
        let guard = guard.clone();
        move |overlay: &adw::ToastOverlay, msg: &str| {
            guard.set(true);
            row.set_active(!on);
            guard.set(false);
            toast_error(overlay, msg);
        }
    };
    run_daemon_command_async(DaemonCommand::SetBoost(on), move |result| match result {
        Ok(DaemonResponse::Ok) => {
            row.set_subtitle(if on {
                "Frequency boost allowed"
            } else {
                "Locked to base clocks — cooler and quieter"
            });
            toast_ok(&overlay, if on { "CPU boost on" } else { "CPU boost off" });
        }
        Ok(DaemonResponse::Error(e)) => revert(&overlay, &e),
        Err(e) if is_version_skew_error(&e) => {
            revert(
                &overlay,
                "Update the control service for boost (reinstall daemon)",
            );
        }
        Err(e) => revert(&overlay, &e),
        _ => {}
    });
}

// ─── Cooling ────────────────────────────────────────────────────────────────

fn build_cooling_overview_page(
    toast_overlay: &adw::ToastOverlay,
    apply_queue: &ApplyQueue,
    gate: &DaemonGate,
) -> gtk::Box {
    let page = page_lede(
        "All fans at a glance — use CPU/GPU/Aux pages for per-fan tuning, or reset when done.",
    );
    let overview = pref_group("Fans overview", None);
    tip(
        &overview,
        "Same fan_card controls as the per-fan pages, stacked for overview",
    );
    let channels = legion_core::fans::channels();
    for ch in &channels {
        overview.add(&fan_card(
            ch.id,
            &ch.title,
            ch.min_rpm as f64,
            ch.max_rpm as f64,
            toast_overlay,
            apply_queue,
        ));
    }
    if channels.is_empty() {
        overview.add(&property_row(
            "No fans detected",
            "Check the daemon and HWMon",
            Some("Fan channels missing — check legion-control service"),
        ));
    }
    let reset = pref_group("Automatic mode", None);
    let btn = primary_button_tip(
        "All fans automatic",
        Some("Clears manual RPM on all detected fans — returns to the firmware fan curve"),
    );
    let queue = apply_queue.clone();
    let fan_ids: Vec<u8> = channels.iter().map(|c| c.id).collect();
    btn.connect_clicked(move |_| {
        for fan in &fan_ids {
            queue.set_fan(*fan, 0);
        }
    });
    let row = adw::ActionRow::builder()
        .title("Reset all")
        .subtitle("Clears any manual RPM targets")
        .activatable(false)
        .build();
    tip(&row, "Recommended after testing loud manual speeds");
    row.add_suffix(&btn);
    reset.add(&row);
    page.append(&overview);
    page.append(&reset);
    gate.track(&overview);
    gate.track(&reset);
    page
}

const THERMAL_TJMAX_WARNING: &str = "\
96–98 °C is above the 9955HX3D TjMax (95 °C). Sustained use above TjMax can \
degrade the CPU or reduce its lifespan.

Only continue if you accept this risk.";

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

fn fan_card(
    fan: u8,
    title: &str,
    min_rpm: f64,
    max_rpm: f64,
    _toast_overlay: &adw::ToastOverlay,
    apply_queue: &ApplyQueue,
) -> adw::PreferencesGroup {
    let sec_tip = match fan {
        1 => "CPU cooling fan",
        2 => "Discrete GPU cooling fan",
        4 => "Auxiliary chassis fan",
        _ => "Fan control for this channel",
    };
    let group = pref_group(title, None);
    tip(&group, sec_tip);

    let rpm_l = gtk::Label::new(Some(&legion_core::fans::rpm_label(fan)));
    rpm_l.add_css_class("numeric");
    rpm_l.add_css_class("fan-rpm");
    tip(
        &rpm_l,
        "Live RPM from the EC — may show Auto or 0 while firmware is driving the fan",
    );

    let auto = legion_core::fans::read_target(fan).unwrap_or(0) == 0;
    let sw = adw::SwitchRow::builder()
        .title(if auto { "Automatic" } else { "Manual" })
        .subtitle(if auto {
            "Firmware temperature curve"
        } else {
            "Fixed speed below"
        })
        .active(auto)
        .build();
    tip(
        &sw,
        "On = Automatic firmware curve · Off = Manual fixed RPM with the slider",
    );
    sw.add_suffix(&rpm_l);
    group.add(&sw);

    let scale = gtk::Scale::with_range(Orientation::Horizontal, min_rpm, max_rpm, 100.0);
    scale.set_draw_value(true);
    scale.set_digits(0);
    scale.set_hexpand(true);
    scale.set_width_request(180);
    tip(
        &scale,
        &format!(
            "Target RPM for {title} · enabled only in Manual · about {min_rpm:.0}–{max_rpm:.0}"
        ),
    );
    let cur = legion_core::fans::read_target(fan).unwrap_or(0);
    if cur > 0 {
        scale.set_value(cur as f64);
        scale.set_sensitive(true);
        scale.set_draw_value(true);
    } else {
        scale.set_value(min_rpm);
        scale.set_sensitive(false);
        scale.set_draw_value(false);
    }

    let speed_row = adw::ActionRow::builder()
        .title("Speed")
        .subtitle("Fixed RPM when Automatic is off")
        .activatable(false)
        .build();
    tip(
        &speed_row,
        "Automatic follows temperature · Manual holds a fixed speed until you change it",
    );
    speed_row.add_suffix(&scale);
    group.add(&speed_row);

    let high_accepted = Rc::new(Cell::new(false));
    let scale_s = scale.clone();
    let sw_title = sw.clone();
    let queue = apply_queue.clone();
    let suppressing = Rc::new(Cell::new(false));
    let suppressing_s = suppressing.clone();
    let high_s = high_accepted.clone();
    sw.connect_active_notify(move |s| {
        if suppressing_s.get() {
            return;
        }
        if s.is_active() {
            scale_s.set_sensitive(false);
            scale_s.set_draw_value(false);
            sw_title.set_title("Automatic");
            sw_title.set_subtitle("Firmware temperature curve");
            queue.set_fan(fan, 0);
        } else {
            let rpm = scale_s.value() as u32;
            let warn_rpm = (max_rpm * 0.85).round() as u32;
            if rpm >= warn_rpm && !high_s.get() {
                let scale_r = scale_s.clone();
                let sw_r = s.clone();
                let sw_title = sw_title.clone();
                let queue = queue.clone();
                let suppressing = suppressing_s.clone();
                let high_s = high_s.clone();
                confirm_risk(
                    s,
                    "Very high fan speed",
                    &format!(
                        "Manual {rpm} RPM is near the maximum for this fan (~{max_rpm:.0}).\n\n\
                         This holds the fan near its maximum speed and increases noise and power use."
                    ),
                    "Keep manual",
                    move |ok| {
                        if !ok {
                            suppressing.set(true);
                            sw_r.set_active(true);
                            scale_r.set_sensitive(false);
                            scale_r.set_draw_value(false);
                            sw_title.set_title("Automatic");
                            sw_title.set_subtitle("Firmware temperature curve");
                            suppressing.set(false);
                            return;
                        }
                        high_s.set(true);
                        scale_r.set_sensitive(true);
                        scale_r.set_draw_value(true);
                        sw_title.set_title("Manual");
                        sw_title.set_subtitle("Fixed speed below");
                        queue.set_fan(fan, rpm);
                    },
                );
                return;
            }
            scale_s.set_sensitive(true);
            scale_s.set_draw_value(true);
            sw_title.set_title("Manual");
            sw_title.set_subtitle("Fixed speed below");
            queue.set_fan(fan, rpm);
        }
    });

    let sw_sc = sw.clone();
    let suppressing_sc = suppressing.clone();
    let queue_sc = apply_queue.clone();
    let high_sc = high_accepted.clone();
    scale.connect_value_changed(move |sc| {
        if sw_sc.is_active() || suppressing_sc.get() {
            return;
        }
        let rpm = sc.value() as u32;
        let warn_rpm = (max_rpm * 0.85).round() as u32;
        if rpm >= warn_rpm && !high_sc.get() {
            let sc_r = sc.clone();
            let queue = queue_sc.clone();
            let suppressing = suppressing_sc.clone();
            let high_sc = high_sc.clone();
            let safe = (max_rpm * 0.7).round() as u32;
            confirm_risk(
                sc,
                "Very high fan speed",
                &format!(
                    "Manual {rpm} RPM is near the maximum for this fan (~{max_rpm:.0}).\n\n\
                     This holds the fan near its maximum speed and increases noise and power use."
                ),
                "Use high speed",
                move |ok| {
                    if !ok {
                        suppressing.set(true);
                        sc_r.set_value(safe as f64);
                        suppressing.set(false);
                        queue.set_fan(fan, safe);
                        return;
                    }
                    high_sc.set(true);
                    queue.set_fan(fan, rpm);
                },
            );
            return;
        }
        queue_sc.set_fan(fan, rpm);
    });

    // Sysfs read on a worker thread; only the label update touches GTK.
    glib::timeout_add_local(Duration::from_secs(2), move || {
        let (tx, rx) = mpsc::channel();
        let fan = fan;
        std::thread::spawn(move || {
            let _ = tx.send(legion_core::fans::rpm_label(fan));
        });
        let rpm_l_c = rpm_l.clone();
        glib::timeout_add_local(Duration::from_millis(50), move || match rx.try_recv() {
            Ok(text) => {
                rpm_l_c.set_text(&text);
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => glib::ControlFlow::Break,
        });
        glib::ControlFlow::Continue
    });

    group
}

// ─── Power ──────────────────────────────────────────────────────────────────

fn build_battery_pages(toast_overlay: &adw::ToastOverlay, gate: &DaemonGate) -> gtk::Box {
    let status_page = page_lede("");

    // Chips on top — 3×2: Capacity · Voltage · Power · Health · Cycles · Limit
    let chips = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .max_children_per_line(3)
        .min_children_per_line(2)
        .homogeneous(true)
        .column_spacing(12)
        .row_spacing(12)
        .build();
    chips.add_css_class("metric-grid");
    chips.set_margin_bottom(12);
    let (cap_chip, cap_v, cap_d) = metric_chip_tip("Capacity", None);
    tip(
        &cap_chip,
        "Battery charge level from sysfs BAT0/capacity — 0–100%",
    );
    let (volt_chip, volt_v, volt_d) = metric_chip_tip("Voltage", None);
    tip(
        &volt_chip,
        "Pack voltage in volts — drops as the pack empties",
    );
    let (power_chip, power_v, power_d) = metric_chip_tip("Power", None);
    tip(
        &power_chip,
        "Watts in or out right now — positive while charging, drain while on battery",
    );
    let (health_chip, health_v, health_d) = metric_chip_tip("Health", None);
    tip(
        &health_chip,
        "Wear vs design capacity — 100% is a fresh pack",
    );
    let (cycles_chip, cycles_v, cycles_d) = metric_chip_tip("Cycles", None);
    tip(
        &cycles_chip,
        "Charge cycle count from the battery gauge — grows with use",
    );
    let (limit_chip, limit_v, limit_d) = metric_chip_tip("Limit", None);
    tip(
        &limit_chip,
        "Charge cap (60/80/100%) — set it in the rows below",
    );
    chips.append(&cap_chip);
    chips.append(&volt_chip);
    chips.append(&power_chip);
    chips.append(&health_chip);
    chips.append(&cycles_chip);
    chips.append(&limit_chip);
    status_page.append(&chips);

    let hero = pref_group("Battery", None);
    tip(
        &hero,
        "Live charge and status from the laptop's battery gauge — updates every few seconds from sysfs, no daemon needed for readouts",
    );

    let pct_row = adw::ActionRow::builder()
        .title("Charge")
        .subtitle("—%")
        .activatable(false)
        .build();
    tip(&pct_row, "How full the battery is right now (0–100%)");
    pct_row.add_css_class("property");
    let st_row = adw::ActionRow::builder()
        .title("Status")
        .subtitle("Reading…")
        .activatable(false)
        .build();
    tip(
        &st_row,
        "Charging = plugged in · Discharging = on battery · Full = topped up · Not charging = holding at limit",
    );
    st_row.add_css_class("property");
    hero.add(&pct_row);
    hero.add(&st_row);
    status_page.append(&hero);

    let stats = pref_group("Details", None);
    tip(
        &stats,
        "Extra battery stats — expand when you need them: voltage, power, energy, health, cycles, pack identity",
    );
    let details_exp = adw::ExpanderRow::builder()
        .title("Show details")
        .subtitle("")
        .build();
    tip(
        &details_exp,
        "Tap to expand voltage, power draw, energy, health, cycles, and pack identity",
    );
    let detail_meta = [
        (
            "Voltage",
            "Battery pack voltage in volts — drops as the pack empties",
        ),
        (
            "Power",
            "Watts in or out right now — positive while charging, drain while on battery",
        ),
        (
            "Energy",
            "Watt-hours left / full design capacity of this pack",
        ),
        ("Health", "Wear level vs design capacity"),
        (
            "Cycles",
            "Charge cycle count reported by the battery (when available)",
        ),
        ("Cell", "Manufacturer, model, and chemistry of the pack"),
    ];
    let detail_rows: Vec<adw::ActionRow> = detail_meta
        .into_iter()
        .map(|(title, tip_text)| {
            let row = property_row(title, "—", Some(tip_text));
            tip(&row, tip_text);
            details_exp.add_row(&row);
            row
        })
        .collect();
    stats.add(&details_exp);
    status_page.append(&stats);

    let lim = pref_group("Charge limit", None);
    tip(
        &lim,
        "60% conserve · 80% balanced life · 100% full tank — caps how far the battery charges while plugged in, helps longevity; needs the legion-control service",
    );
    let pills = gtk::Box::new(Orientation::Horizontal, 12);
    pills.set_halign(Align::Center);
    pills.set_margin_top(8);
    pills.set_margin_bottom(8);
    let store: Rc<RefCell<Vec<gtk::Button>>> = Rc::new(RefCell::new(Vec::new()));
    let cur = legion_core::battery::charge_limit_pct();

    for pct in [60u32, 80, 100] {
        let tip_text = match pct {
            60 => {
                "Conservation (~60%): stops charging early so the pack sits cooler — best if the laptop stays plugged in most of the time"
            }
            80 => {
                "Long life (~80%): more runtime than 60%, gentler than always charging to 100%"
            }
            _ => {
                "Full charge (100%): maximum capacity — use before travel or long unplugged sessions"
            }
        };
        let btn = gtk::Button::with_label(&format!("{pct}%"));
        btn.add_css_class("charge-pill");
        tip(&btn, tip_text);
        if pct == cur {
            btn.add_css_class("suggested-action");
        }
        let overlay = toast_overlay.clone();
        let store_c = store.clone();
        let this = btn.clone();
        btn.connect_clicked(move |_| {
            for b in store_c.borrow().iter() {
                b.remove_css_class("suggested-action");
            }
            this.add_css_class("suggested-action");
            let overlay = overlay.clone();
            apply_charge_limit(pct, move |result| match result {
                Ok(()) => {
                    legion_core::config::set_charge_limit(pct);
                    toast_ok(&overlay, &format!("Charge limit set to {pct}%"));
                }
                Err(e) => toast_error(&overlay, &e),
            });
        });
        store.borrow_mut().push(btn.clone());
        pills.append(&btn);
    }
    let pill_row = adw::ActionRow::builder()
        .title("Stop charging at")
        .subtitle("")
        .activatable(false)
        .build();
    tip(
        &pill_row,
        "Lenovo conservation / long-life modes — 60% conserve, 80% long life, 100% full — selected button highlighted in red",
    );
    pill_row.add_suffix(&pills);
    lim.add(&pill_row);
    gate.track(&lim);
    status_page.append(&lim);

    let volt_l = detail_rows[0].clone();
    let pow_l = detail_rows[1].clone();
    let en_l = detail_rows[2].clone();
    let health_l = detail_rows[3].clone();
    let cycles_l = detail_rows[4].clone();
    let cell_l = detail_rows[5].clone();

    // Prime chips immediately — otherwise first frame shows "—" until the 3 s tick.
    {
        if let Some(pct) = legion_core::battery::capacity() {
            cap_v.set_text(&format!("{pct}%"));
            cap_d.set_text(&legion_core::battery::status().unwrap_or_default());
            pct_row.set_subtitle(&format!("{pct}%"));
            st_row
                .set_subtitle(&legion_core::battery::status().unwrap_or_else(|| "Unknown".into()));
        }
        if let Some(v) = legion_core::battery::voltage() {
            volt_v.set_text(&format!("{v:.2} V"));
        }
        if let Some(p) = legion_core::battery::power_w() {
            power_v.set_text(&format!("{p:.1} W"));
        }
        if let Some(h) = legion_core::battery::health_pct() {
            health_v.set_text(&format!("{h:.0}%"));
            health_d.set_text(if h < 80.0 { "worn" } else { "good" });
        }
        if let Some(c) = legion_core::battery::cycles() {
            cycles_v.set_text(&format!("{c}"));
            cycles_d.set_text("charge cycles");
        }
        limit_v.set_text(&format!("{}%", legion_core::battery::charge_limit_pct()));
        limit_d.set_text("charge cap");
    }
    // Keep chips + rows in sync every 3 s
    let cap_v_c = cap_v.clone();
    let cap_d_c = cap_d.clone();
    let volt_v_c = volt_v.clone();
    let volt_d_c = volt_d.clone();
    let power_v_c = power_v.clone();
    let power_d_c = power_d.clone();
    let health_v_c = health_v.clone();
    let health_d_c = health_d.clone();
    let cap_chip_c = cap_chip.clone();
    let volt_chip_c = volt_chip.clone();
    let power_chip_c = power_chip.clone();
    let health_chip_c = health_chip.clone();
    let cycles_v_c = cycles_v.clone();
    let cycles_d_c = cycles_d.clone();
    let limit_v_c = limit_v.clone();
    let limit_d_c = limit_d.clone();
    // All sysfs reads happen on a worker thread; only widget updates run on
    // the GTK loop (same pattern as the Overview poller).
    #[derive(Default)]
    struct BatterySnapshot {
        pct: Option<u32>,
        status: Option<String>,
        voltage: Option<f64>,
        power_w: Option<f64>,
        energy_now: Option<f64>,
        energy_full: Option<f64>,
        health: Option<f64>,
        cycles: Option<u32>,
        limit: u32,
        mfr: String,
        model: String,
        tech: String,
    }
    let (snap_tx, snap_rx) = mpsc::channel();
    std::thread::spawn(move || loop {
        let snap = BatterySnapshot {
            pct: legion_core::battery::capacity(),
            status: legion_core::battery::status(),
            voltage: legion_core::battery::voltage(),
            power_w: legion_core::battery::power_w(),
            energy_now: legion_core::battery::energy_now_wh(),
            energy_full: legion_core::battery::energy_full_wh(),
            health: legion_core::battery::health_pct(),
            cycles: legion_core::battery::cycles(),
            limit: legion_core::battery::charge_limit_pct(),
            mfr: legion_core::battery::manufacturer().unwrap_or_default(),
            model: legion_core::battery::model_name().unwrap_or_default(),
            tech: legion_core::battery::technology().unwrap_or_default(),
        };
        if snap_tx.send(snap).is_err() {
            return; // GUI is gone
        }
        std::thread::sleep(Duration::from_secs(3));
    });
    let overlay = toast_overlay.clone();
    // One-time-per-session hint: the EC charges past the limiter while the
    // laptop is off/asleep (documented behavior) — explain the surprise
    // instead of leaving a confusing 98% unexplained.
    let off_charge_hint: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    glib::timeout_add_local(Duration::from_millis(300), move || {
        match snap_rx.try_recv() {
            Ok(s) => {
                if !off_charge_hint.get() && s.limit < 100 && s.pct.is_some_and(|p| p > 85) {
                    off_charge_hint.set(true);
                    toast_info(
                        &overlay,
                        "Battery charged past the limit while the laptop was off — EC behavior. It settles to ~80% with use; unplug AC when off to prevent it.",
                    );
                }
                if let Some(pct) = s.pct {
                    pct_row.set_subtitle(&format!("{pct}%"));
                    cap_v_c.set_text(&format!("{pct}%"));
                    cap_d_c.set_text(s.status.as_deref().unwrap_or_default());
                    st_row.set_subtitle(s.status.as_deref().unwrap_or("Unknown"));
                    // Tint capacity chip: warm when discharging low, hot when very low? Use level thresholds.
                    cap_chip_c.remove_css_class("hot");
                    cap_chip_c.remove_css_class("warm");
                    if pct <= 20 {
                        cap_chip_c.add_css_class("hot");
                    } else if pct <= 40 {
                        cap_chip_c.add_css_class("warm");
                    }
                }
                if let Some(v) = s.voltage {
                    volt_l.set_subtitle(&format!("{v:.2} V"));
                    volt_v_c.set_text(&format!("{v:.2} V"));
                    volt_d_c.set_text("");
                    volt_chip_c.remove_css_class("hot");
                    volt_chip_c.remove_css_class("warm");
                } else {
                    volt_v_c.set_text("—");
                    volt_d_c.set_text("no sensor");
                }
                if let Some(p) = s.power_w {
                    pow_l.set_subtitle(&format!("{p:.1} W"));
                    power_v_c.set_text(&format!("{p:.1} W"));
                    // detail: charging vs discharging
                    let st = s.status.as_deref().unwrap_or_default();
                    power_d_c.set_text(if st == "Charging" {
                        "charging"
                    } else if st == "Discharging" {
                        "discharging"
                    } else {
                        ""
                    });
                    power_chip_c.remove_css_class("hot");
                    power_chip_c.remove_css_class("warm");
                    if p.abs() > 45.0 {
                        power_chip_c.add_css_class("warm");
                    }
                } else {
                    pow_l.set_subtitle("—");
                    power_v_c.set_text("—");
                    power_d_c.set_text("no sensor");
                }
                if let (Some(n), Some(f)) = (s.energy_now, s.energy_full) {
                    en_l.set_subtitle(&format!("{n:.1} / {f:.1} Wh"))
                }
                if let Some(h) = s.health {
                    health_l.set_subtitle(&format!("{h:.0}%"));
                    health_v_c.set_text(&format!("{h:.0}%"));
                    health_d_c.set_text(if h < 80.0 { "worn" } else { "good" });
                    health_chip_c.remove_css_class("hot");
                    health_chip_c.remove_css_class("warm");
                    if h < 70.0 {
                        health_chip_c.add_css_class("hot");
                    } else if h < 85.0 {
                        health_chip_c.add_css_class("warm");
                    }
                } else {
                    health_v_c.set_text("—");
                    health_d_c.set_text("no sensor");
                }
                if let Some(c) = s.cycles {
                    cycles_l.set_subtitle(&format!("{c}"));
                    cycles_v_c.set_text(&format!("{c}"));
                    cycles_d_c.set_text("charge cycles");
                } else {
                    cycles_v_c.set_text("—");
                    cycles_d_c.set_text("no sensor");
                }
                limit_v_c.set_text(&format!("{}%", s.limit));
                limit_d_c.set_text("charge cap");
                cell_l.set_subtitle(format!("{} {} · {}", s.mfr, s.model, s.tech).trim());
                glib::ControlFlow::Continue
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => glib::ControlFlow::Break,
        }
    });

    status_page
}

// ─── Troubleshoot ───────────────────────────────────────────────────────────

fn build_fix_audio_page(toast_overlay: &adw::ToastOverlay) -> gtk::Box {
    let page = page_lede("");
    page.append(&build_speakers_section(toast_overlay));
    page
}

/// One "Fix" destination with an internal switcher instead of three sidebar
/// rows — keeps the rail short while all diagnostics stay one click away.
fn build_fix_page(toast_overlay: &adw::ToastOverlay, gate: &DaemonGate) -> gtk::Box {
    let page = page_lede("");

    let inner = adw::ViewStack::new();
    inner.set_vexpand(true);
    inner.add_titled(
        &page_shell(&build_fix_audio_page(toast_overlay)),
        Some("fix-audio"),
        "Speakers",
    );
    inner.add_titled(
        &page_shell(&build_fix_lighting_page(toast_overlay, gate)),
        Some("fix-lighting"),
        "Lighting",
    );
    inner.add_titled(
        &page_shell(&build_fix_logs_page(toast_overlay)),
        Some("fix-logs"),
        "Logs",
    );

    // Same horizontal tab bar as the CPU/About/Lighting hubs.
    let switcher = adw::ViewSwitcher::new();
    switcher.set_stack(Some(&inner));
    switcher.set_policy(adw::ViewSwitcherPolicy::Wide);
    switcher.set_halign(Align::Center);
    tip(
        &switcher,
        "Speaker audio issues · stuck or dark Spectrum RGB · recent control-service output",
    );
    let bar = gtk::Box::new(Orientation::Vertical, 0);
    bar.add_css_class("hub-bar");
    bar.append(&switcher);
    page.append(&bar);
    page.append(&inner);
    page
}

fn build_fix_lighting_page(toast_overlay: &adw::ToastOverlay, gate: &DaemonGate) -> gtk::Box {
    let page = page_lede("");
    page.append(&build_lighting_reset_section(toast_overlay, gate));
    page
}

fn build_fix_logs_page(toast_overlay: &adw::ToastOverlay) -> gtk::Box {
    let page = page_lede("");
    page.append(&build_logs_section(toast_overlay));
    page
}

fn build_lighting_reset_section(
    toast_overlay: &adw::ToastOverlay,
    gate: &DaemonGate,
) -> adw::PreferencesGroup {
    use legion_core::rgb_panic::{self, Health};

    let group = pref_group(
        "Keyboard lighting issue",
        Some(
            "Detects Spectrum HID / kernel USB faults when lights go black, then soft-resets or USB-resets the controller",
        ),
    );

    let diag0 = rgb_panic::diagnose();
    let (pill_text, pill_kind) = rgb_pill(diag0.health);
    let pill = status_pill_tip(pill_text, pill_kind, Some(rgb_pill_tooltip(diag0.health)));

    let status_row = adw::ActionRow::builder()
        .title(&diag0.summary)
        .subtitle(rgb_short_help(diag0.health))
        .activatable(false)
        .build();
    tip(
        &status_row,
        "Scans hidraw 048d:c197, ioctl health, brightness vs saved config, and kernel USB/HID errors",
    );
    status_row.add_suffix(&pill);
    group.add(&status_row);

    let expander = adw::ExpanderRow::builder()
        .title("Technical details")
        .subtitle("HID path, permissions, ioctl, kernel log hits")
        .build();
    tip(
        &expander,
        "Expand for HID path, permissions, ioctl health, and kernel USB/HID log hits",
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
        "Raw diagnostic lines — selectable so you can copy them into a bug report",
    );
    expander.add_row(&details);
    group.add(&expander);

    let btn = primary_button_tip(
        match diag0.health {
            Health::Ok => "Re-check RGB",
            Health::SoftIssue => "Repair lighting",
            Health::HardwareBroken => "USB reset & restore",
            Health::NotApplicable => "Check RGB",
        },
        Some(
            "Soft lighting reset → fix permissions → USB reset → hid-generic rebind → restore saved look (daemon)",
        ),
    );
    let action = adw::ActionRow::builder()
        .title("Auto-fix")
        .subtitle("Daemon watches kernel HID faults and can auto-fix in the background")
        .activatable(false)
        .build();
    tip(
        &action,
        "Needs updated legion-control service for USB reset privileges",
    );
    action.add_suffix(&btn);
    group.add(&action);
    gate.track(&btn);

    let overlay = toast_overlay.clone();
    let pill_c = pill.clone();
    let status_c = status_row.clone();
    let details_c = details.clone();
    let expander_c = expander.clone();
    let btn_c = btn.clone();
    btn.connect_clicked(move |_| {
        set_busy(&btn_c, true, "Repair lighting");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let report = match send_command(DaemonCommand::FixRgbPanic) {
                Ok(DaemonResponse::RgbFixReport {
                    steps,
                    errors,
                    health,
                    summary,
                }) => {
                    let health = match health.as_str() {
                        "ok" => Health::Ok,
                        "soft-issue" => Health::SoftIssue,
                        "broken" => Health::HardwareBroken,
                        _ => Health::NotApplicable,
                    };
                    Ok((steps, errors, health, summary))
                }
                Ok(DaemonResponse::Error(e)) => Err(e),
                Err(e) if is_version_skew_error(&e) => {
                    let r = rgb_panic::troubleshoot();
                    Ok((r.steps, r.errors, r.after.health, r.after.summary))
                }
                Err(e) => Err(e),
                _ => Err("Unexpected daemon response".into()),
            };
            let _ = tx.send(report);
        });

        let overlay = overlay.clone();
        let pill_c = pill_c.clone();
        let status_c = status_c.clone();
        let details_c = details_c.clone();
        let expander_c = expander_c.clone();
        let btn_c = btn_c.clone();
        glib::timeout_add_local(Duration::from_millis(200), move || match rx.try_recv() {
            Ok(Ok((steps, errors, health, summary))) => {
                let (pt, pk) = rgb_pill(health);
                set_pill(&pill_c, pt, pk);
                tip(&pill_c, rgb_pill_tooltip(health));
                status_c.set_title(&summary);
                status_c.set_subtitle(rgb_short_help(health));
                let mut body = steps.iter().map(|s| format!("· {s}")).collect::<Vec<_>>();
                if !errors.is_empty() {
                    body.push(String::new());
                    body.push("Problems:".into());
                    body.extend(errors.iter().map(|s| format!("· {s}")));
                }
                details_c.set_text(&body.join("\n"));
                expander_c.set_expanded(true);
                match health {
                    Health::Ok => toast_ok(&overlay, "Spectrum RGB healthy"),
                    Health::SoftIssue => {
                        toast_error(&overlay, "Partially fixed — check Lighting tab")
                    }
                    Health::HardwareBroken => {
                        toast_error(&overlay, "Still broken — try reboot or USB replug")
                    }
                    Health::NotApplicable => toast_ok(&overlay, &summary),
                }
                let idle = match health {
                    Health::Ok => "Re-check RGB",
                    Health::SoftIssue => "Repair lighting",
                    Health::HardwareBroken => "USB reset & restore",
                    Health::NotApplicable => "Check RGB",
                };
                set_busy(&btn_c, false, idle);
                glib::ControlFlow::Break
            }
            Ok(Err(e)) => {
                set_pill(&pill_c, "Failed", "bad");
                details_c.set_text(&e);
                expander_c.set_expanded(true);
                toast_error(&overlay, &e);
                set_busy(&btn_c, false, "Repair lighting");
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => {
                set_busy(&btn_c, false, "Repair lighting");
                toast_error(&overlay, "RGB fix failed");
                glib::ControlFlow::Break
            }
        });
    });

    group
}

fn build_logs_section(toast_overlay: &adw::ToastOverlay) -> adw::PreferencesGroup {
    use legion_core::comms::{send_command, DaemonCommand, DaemonResponse};

    let group = pref_group("Daemon logs", None);

    let text_view = gtk::TextView::new();
    text_view.set_editable(false);
    text_view.set_monospace(true);
    text_view.set_wrap_mode(gtk::WrapMode::WordChar);
    text_view.set_top_margin(6);
    text_view.set_bottom_margin(6);
    text_view.set_left_margin(8);
    text_view.set_right_margin(8);
    let buffer = text_view.buffer();
    buffer.set_text("Click Fetch logs to retrieve recent daemon output.");

    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&text_view));
    scroll.set_vexpand(true);
    scroll.set_min_content_height(200);
    scroll.set_max_content_height(400);
    group.add(&scroll);

    let fetch_btn = primary_button_tip(
        "Fetch logs",
        Some("Query the last 100 log lines from the daemon"),
    );
    let copy_btn = primary_button_tip("Copy", Some("Copy log text to clipboard"));
    let level_btn = primary_button_tip(
        "Verbose",
        Some("Toggle daemon between info and debug logging"),
    );

    let btn_row = gtk::Box::new(Orientation::Horizontal, 8);
    btn_row.set_margin_top(4);
    btn_row.set_margin_bottom(8);
    btn_row.append(&fetch_btn);
    btn_row.append(&copy_btn);
    btn_row.append(&level_btn);
    group.add(&btn_row);

    // Fetch logs
    let overlay = toast_overlay.clone();
    let buf = buffer.clone();
    fetch_btn.connect_clicked(move |_| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let text = match send_command(DaemonCommand::GetRecentLogs(100)) {
                Ok(DaemonResponse::RecentLogs(t)) => {
                    if t.is_empty() {
                        "(no log entries)".into()
                    } else {
                        t
                    }
                }
                Ok(DaemonResponse::Error(e)) => format!("daemon error: {e}"),
                Err(e) => format!("ipc error: {e}"),
                _ => "Unexpected response".into(),
            };
            let _ = tx.send(text);
        });
        let o = overlay.clone();
        let b = buf.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || match rx.try_recv() {
            Ok(text) => {
                b.set_text(&text);
                o.add_toast(adw::Toast::new("Logs fetched"));
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => {
                o.add_toast(adw::Toast::new("Failed to fetch logs"));
                glib::ControlFlow::Break
            }
        });
    });

    // Copy to clipboard
    let buf_copy = buffer.clone();
    copy_btn.connect_clicked(move |_| {
        if let Some(display) = gtk::gdk::Display::default() {
            let (start, end) = buf_copy.bounds();
            let text = buf_copy.text(&start, &end, false);
            display.clipboard().set_text(&text);
        }
    });

    // Toggle verbose / quiet
    let level_btn_c = level_btn.clone();
    let overlay_for_level = toast_overlay.clone();
    level_btn.connect_clicked(move |btn| {
        // Button shows "Verbose" when daemon is at info → clicking switches to debug.
        // Button shows "Quiet" when daemon is at debug → clicking switches back to info.
        let switching_to_debug = btn.label().is_some_and(|l| l == "Verbose");
        let new_level = if switching_to_debug { "debug" } else { "info" };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let r = send_command(DaemonCommand::SetLogLevel(new_level.into()));
            let _ = tx.send(r);
        });
        let btn_c = level_btn_c.clone();
        let overlay = overlay_for_level.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || match rx.try_recv() {
            Ok(Ok(DaemonResponse::Ok)) => {
                btn_c.set_label(if switching_to_debug {
                    "Quiet"
                } else {
                    "Verbose"
                });
                overlay.add_toast(adw::Toast::new(if switching_to_debug {
                    "Debug logging on"
                } else {
                    "Info logging on"
                }));
                glib::ControlFlow::Break
            }
            Ok(Ok(DaemonResponse::Error(e))) => {
                overlay.add_toast(adw::Toast::new(&format!("Failed: {e}")));
                glib::ControlFlow::Break
            }
            _ => {
                overlay.add_toast(adw::Toast::new("Daemon unreachable"));
                glib::ControlFlow::Break
            }
        });
    });

    group
}

fn rgb_pill(health: legion_core::rgb_panic::Health) -> (&'static str, &'static str) {
    use legion_core::rgb_panic::Health;
    match health {
        Health::Ok => ("OK", "ok"),
        Health::SoftIssue => ("Panic", "warn"),
        Health::HardwareBroken => ("Not responding", "bad"),
        Health::NotApplicable => ("N/A", "muted"),
    }
}

fn rgb_pill_tooltip(health: legion_core::rgb_panic::Health) -> &'static str {
    use legion_core::rgb_panic::Health;
    match health {
        Health::Ok => "Spectrum HID answering — brightness and ioctl look fine",
        Health::SoftIssue => {
            "Lights stuck off, permissions, or recent kernel USB blip — auto-fix should help"
        }
        Health::HardwareBroken => {
            "HID ioctl dead or device missing — needs USB reset (daemon) or replug"
        }
        Health::NotApplicable => "No 048d:c197 Spectrum controller on this machine",
    }
}

fn rgb_short_help(health: legion_core::rgb_panic::Health) -> &'static str {
    use legion_core::rgb_panic::Health;
    match health {
        Health::Ok => {
            "Controller healthy. Daemon still watches kernel HID faults in the background."
        }
        Health::SoftIssue => "Lighting not responding — run Auto-fix to restore it.",
        Health::HardwareBroken => {
            "HID not responding — Auto-fix will USB-reset and rebind hid-generic."
        }
        Health::NotApplicable => "No Spectrum RGB hardware detected.",
    }
}

// ─── About ──────────────────────────────────────────────────────────────────

fn kde_widget_installed() -> bool {
    std::process::Command::new("kpackagetool6")
        .args(["--type", "Plasma/Applet", "-l"])
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).contains(KDE_WIDGET_ID))
        .unwrap_or(false)
}

fn extract_kde_widget() -> Result<std::path::PathBuf, String> {
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

fn install_kde_widget() -> Result<(), String> {
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

fn remove_kde_widget() -> Result<(), String> {
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

fn build_kde_widget_section(toast_overlay: &adw::ToastOverlay) -> adw::PreferencesGroup {
    let installed = kde_widget_installed();
    let group = pref_group("KDE Plasma widget", None);
    let row = adw::ActionRow::builder()
        .title("Legion Control widget")
        .subtitle(if installed {
            "Installed — add it from Plasma’s widget picker"
        } else {
            "Not installed — requires KDE Plasma 6"
        })
        .activatable(false)
        .build();

    let actions = gtk::Box::new(Orientation::Horizontal, 6);
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
            row_c.set_subtitle("Installed — add it from Plasma’s widget picker");
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
            row_c.set_subtitle("Not installed — requires KDE Plasma 6");
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

/// Alpha diagnostics opt-in — privacy disclosure, consent switch, self-check
/// runner, and on-demand send. All of it works without the daemon running.
fn build_diagnostics_section(toast_overlay: &adw::ToastOverlay) -> adw::PreferencesGroup {
    let group = pref_group("Alpha diagnostics (anonymous)", None);

    // Caption-style disclosure row — the exact data contract shown verbatim.
    let disclosure = adw::ActionRow::builder()
        .activatable(false)
        .subtitle(
            "Alpha program: with your consent, one anonymized JSON report is sent per click/schedule — \
             hardware model, distro/kernel, sensor readings, fan states, battery health stats, thermal & \
             Curve Optimizer settings, a settings digest, recent daemon log lines, and self-check results. \
             NEVER included: hostname, username, serials, MACs, IPs, per-key colors, profile names. \
             Off by default.",
        )
        .build();
    group.add(&disclosure);

    let share_row = adw::SwitchRow::builder()
        .title("Share anonymous diagnostics")
        .subtitle("Off by default · sends only on demand or schedule")
        .active(legion_core::config::get().diagnostics.enabled)
        .build();
    tip(
        &share_row,
        "Consent switch — nothing is collected or sent while this is off",
    );
    group.add(&share_row);

    {
        let overlay = toast_overlay.clone();
        share_row.connect_active_notify(move |row| {
            let enabled = row.is_active();
            legion_core::config::update(|c| c.diagnostics.enabled = enabled);
            if enabled {
                toast_ok(&overlay, "Anonymous diagnostics enabled");
            } else {
                toast_ok(&overlay, "Anonymous diagnostics disabled");
            }
        });
    }

    let check_row = adw::ActionRow::builder()
        .title("Self-check")
        .subtitle("Read-only checks of config, battery, fans, sensors, and lighting")
        .activatable(false)
        .build();
    let run_btn = gtk::Button::with_label("Run");
    run_btn.set_valign(Align::Center);
    run_btn.add_css_class("pill-btn");
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

    let send_row = adw::ActionRow::builder()
        .title("Send now")
        .subtitle("Collects and uploads one anonymized report")
        .activatable(false)
        .build();
    let send_btn = gtk::Button::with_label("Send now");
    send_btn.set_valign(Align::Center);
    send_btn.add_css_class("pill-btn");
    tip(
        &send_btn,
        "Collects and sends one anonymized report immediately",
    );
    send_row.add_suffix(&send_btn);
    group.add(&send_row);

    {
        let overlay = toast_overlay.clone();
        let send_btn_connect = send_btn.clone();
        let send_btn_closure = send_btn.clone();
        send_btn_connect.connect_clicked(move |_| {
            send_btn_closure.set_sensitive(false);
            let overlay = overlay.clone();
            let btn_inner = send_btn_closure.clone();
            dispatch_async(
                move || legion_core::diagnostics::collect_and_send(None),
                "Diagnostics send stopped without a result",
                move |result| {
                    btn_inner.set_sensitive(true);
                    match result {
                        Ok(_) => toast_info(&overlay, "Diagnostics sent — thank you!"),
                        Err(error) => toast_error(&overlay, &error),
                    }
                },
            );
        });
    }

    group
}

fn build_components_section(toast_overlay: &adw::ToastOverlay) -> adw::PreferencesGroup {
    let group = pref_group("First-time setup", None);

    let daemon_active = std::path::Path::new(legion_core::comms::SYSTEM_SOCKET).exists();
    let daemon_row = adw::ActionRow::builder()
        .title("Hardware control daemon")
        .subtitle(if daemon_active {
            "Active — required for privileged hardware controls"
        } else {
            "Inactive — enable it to use hardware controls"
        })
        .activatable(false)
        .build();
    let daemon_button = primary_button_tip(
        if daemon_active { "Enabled" } else { "Enable" },
        Some("Uses a narrowly scoped PolicyKit helper; no shell command is accepted"),
    );
    daemon_button.set_sensitive(!daemon_active);
    daemon_row.add_suffix(&daemon_button);
    group.add(&daemon_row);

    let overlay = toast_overlay.clone();
    let row = daemon_row.clone();
    let button = daemon_button.clone();
    daemon_button.connect_clicked(move |_| {
        button.set_sensitive(false);
        button.set_label("Enabling…");
        let overlay = overlay.clone();
        let row = row.clone();
        let button = button.clone();
        run_setup_helper("enable-daemon", move |result| match result {
            Ok(_) => {
                row.set_subtitle("Active — required for privileged hardware controls");
                button.set_label("Enabled");
                toast_ok(&overlay, "Hardware daemon enabled");
            }
            Err(error) => {
                button.set_label("Enable");
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
                    "Installed · firmware read-only probe passed".to_string()
                }
                Ok(DaemonResponse::CurveOptimizer(status)) => status.reason,
                _ if smu_installed => "Driver loaded · restart the daemon to probe firmware".into(),
                _ => "Optional · enables temporary AMD Curve Optimizer controls".into(),
            };
            smu_row_c.set_subtitle(&smu_status);
        });
    }
    let smu_actions = gtk::Box::new(Orientation::Horizontal, 6);
    smu_actions.set_valign(Align::Center);
    let remove_smu = gtk::Button::builder()
        .label("Remove")
        .sensitive(smu_installed)
        .tooltip_text("Unload and remove the optional ryzen_smu DKMS driver")
        .build();
    let install_smu = primary_button_tip(
        if smu_installed {
            "Installed"
        } else {
            "Install"
        },
        Some("Builds the bundled, pinned ryzen_smu source through DKMS using PolicyKit"),
    );
    install_smu.set_sensitive(!smu_installed);
    smu_actions.append(&remove_smu);
    smu_actions.append(&install_smu);
    smu_row.add_suffix(&smu_actions);
    group.add(&smu_row);

    let overlay = toast_overlay.clone();
    let row = smu_row.clone();
    let install = install_smu.clone();
    let remove = remove_smu.clone();
    install_smu.connect_clicked(move |_| {
        install.set_sensitive(false);
        install.set_label("Installing…");
        let overlay = overlay.clone();
        let row = row.clone();
        let install = install.clone();
        let remove = remove.clone();
        run_setup_helper("install-ryzen-smu", move |result| match result {
            Ok(_) => {
                row.set_subtitle("Installed · no tuning value was written");
                install.set_label("Installed");
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
                row.set_subtitle("Optional · enables temporary AMD Curve Optimizer controls");
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

fn build_about_pages(
    toast_overlay: &adw::ToastOverlay,
) -> (gtk::Box, gtk::Box, gtk::Box, gtk::Box) {
    let setup_page = page_lede("");
    let help_page = page_lede("");
    let hardware_page = page_lede("");
    let storage_page = page_lede("");
    let info = legion_core::device::detect();

    setup_page.append(&build_components_section(toast_overlay));
    setup_page.append(&build_kde_widget_section(toast_overlay));
    setup_page.append(&build_diagnostics_section(toast_overlay));

    let help = pref_group("Help", None);
    let report_row = adw::ActionRow::builder()
        .title("Report an issue")
        .subtitle("GitHub — bugs and feature requests")
        .activatable(true)
        .build();
    tip(
        &report_row,
        "Opens https://github.com/encomjp/ — report bugs or request features",
    );
    report_row.connect_activated(|_| {
        open_uri("https://github.com/encomjp/");
    });
    let report_open = flat_open_button("Opens GitHub in your browser");
    report_open.connect_clicked(|_| {
        open_uri("https://github.com/encomjp/");
    });
    report_row.add_suffix(&report_open);
    help.add(&report_row);

    let donate_row = adw::ActionRow::builder()
        .title("Donate")
        .subtitle("PayPal — optional support for the project")
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
            "CPU model string from /proc/cpuinfo",
        ),
        (
            "Graphics",
            info.gpu_model.as_str(),
            "Discrete GPU name from nvidia-smi when available",
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
    storage_page.append(&lighting);
    (setup_page, help_page, hardware_page, storage_page)
}

fn build_speakers_section(toast_overlay: &adw::ToastOverlay) -> adw::PreferencesGroup {
    use legion_core::audio::{self, Health};

    let group = pref_group(
        "Speakers",
        Some(
            "Gen 10 woofers use an AW88399 smart amp — tinny sound usually means the amp isn’t loaded or audio is muted / on the wrong output",
        ),
    );
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
        .subtitle("ACPI amp, modules, firmware, mute, PipeWire sink")
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
            Health::NotApplicable => "Check speakers",
        },
        Some(amp_action_tooltip(diag0.health)),
    );
    let action = adw::ActionRow::builder()
        .title("Repair")
        .subtitle("Unmute, restart PipeWire, prefer onboard sink")
        .activatable(false)
        .build();
    tip(&action, amp_action_tooltip(diag0.health));
    action.add_suffix(&btn);
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

fn amp_pill(health: legion_core::audio::Health) -> (&'static str, &'static str) {
    use legion_core::audio::Health;
    match health {
        Health::Ok => ("OK", "ok"),
        Health::SoftIssue => ("Needs fix", "warn"),
        Health::HardwareBroken => ("Broken", "bad"),
        Health::NotApplicable => ("N/A", "muted"),
    }
}

fn amp_pill_tooltip(health: legion_core::audio::Health) -> &'static str {
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

fn amp_action_tooltip(health: legion_core::audio::Health) -> &'static str {
    use legion_core::audio::Health;
    match health {
        Health::Ok => "Re-runs unmute, PipeWire restart, and sets the onboard speakers as default",
        Health::SoftIssue => {
            "Unmutes Speaker/Master, restarts PipeWire, and switches to onboard speakers"
        }
        Health::HardwareBroken => {
            "Still tries unmute/PipeWire — will not pretend the amp driver is fixed if it is missing"
        }
        Health::NotApplicable => "Runs a speaker health check and soft recovery if possible",
    }
}

fn amp_short_help(health: legion_core::audio::Health) -> &'static str {
    use legion_core::audio::Health;
    match health {
        Health::Ok => "Smart amp is connected. You can still refresh if sound feels off.",
        Health::SoftIssue => {
            "Amp is fine — volume, mute, or the wrong output is likely the issue."
        }
        Health::HardwareBroken => {
            "The woofer amp isn’t loaded. Soft fixes help mute/sink issues only — you may need a patched kernel."
        }
        Health::NotApplicable => "No AW88399 smart amp found on this machine.",
    }
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
                    other => panic!("hub tab {tab} mapped into non-hub page {other}"),
                };
                let _ = hub;
                let valid = match top {
                    "cpu" => ["features", "tuning", "power"].contains(&tab),
                    "about" => ["setup", "hardware", "storage", "help"].contains(&tab),
                    "fix" => ["fix-audio", "fix-lighting", "fix-logs"].contains(&tab),
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
