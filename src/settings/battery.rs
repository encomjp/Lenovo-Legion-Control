//! Battery page — live stats, charge limit, EC advisory.

use super::*;

pub(crate) const OFF_CHARGE_HINT: &str = "Battery charged past the limit while the laptop was off — EC behavior. It settles to ~80% with use; unplug AC when off to prevent it.";

/// Single source of truth for top-level page ids → header titles. nav_to callers,
/// the LEGION_PAGE override, and the visible-child sync all read this so the
/// maps can never disagree.
pub(crate) fn build_battery_pages(
    toast_overlay: &adw::ToastOverlay,
    gate: &DaemonGate,
) -> gtk::Box {
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
    // Keep the EC advisory on the Battery page instead of repeatedly adding a
    // global toast while the snapshot poll runs.
    let off_charge_hint = gtk::Label::new(Some(OFF_CHARGE_HINT));
    off_charge_hint.add_css_class("hint");
    off_charge_hint.set_halign(Align::Fill);
    off_charge_hint.set_hexpand(true);
    off_charge_hint.set_wrap(true);
    off_charge_hint.set_xalign(0.0);
    off_charge_hint.set_visible(false);
    status_page.append(&off_charge_hint);
    glib::timeout_add_local(Duration::from_millis(300), move || {
        match snap_rx.try_recv() {
            Ok(s) => {
                if !off_charge_hint.is_visible()
                    && s.pct
                        .is_some_and(|p| legion_core::battery::above_limiter_band(s.limit, p))
                {
                    off_charge_hint.set_visible(true);
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
