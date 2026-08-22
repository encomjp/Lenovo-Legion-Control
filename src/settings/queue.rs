//! Coalesced daemon writes for the settings UI.
//!
//! Rapid slider / switch spam keeps only the latest value per key, then flushes
//! after a short quiet period — same idea as the Spectrum HID worker.

use legion_core::comms::{send_command, DaemonCommand, DaemonResponse};

use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct ApplyQueue {
    inner: Rc<Inner>,
}

#[derive(Debug)]
struct Inner {
    overlay: adw::ToastOverlay,
    fans: RefCell<HashMap<u8, u32>>,
    attrs: RefCell<HashMap<String, String>>,
    ticket: Cell<u32>,
    busy_toast: RefCell<Option<adw::Toast>>,
}

impl ApplyQueue {
    pub fn new(overlay: &adw::ToastOverlay) -> Self {
        Self {
            inner: Rc::new(Inner {
                overlay: overlay.clone(),
                fans: RefCell::new(HashMap::new()),
                attrs: RefCell::new(HashMap::new()),
                ticket: Cell::new(0),
                busy_toast: RefCell::new(None),
            }),
        }
    }

    pub fn set_fan(&self, fan: u8, rpm: u32) {
        self.inner.fans.borrow_mut().insert(fan, rpm);
        legion_core::config::remember_fan(fan, rpm);
        self.kick();
    }

    pub fn set_fw_attr(&self, name: impl Into<String>, value: impl Into<String>) {
        self.inner
            .attrs
            .borrow_mut()
            .insert(name.into(), value.into());
        self.kick();
    }

    fn kick(&self) {
        // Coalesce quietly — only show feedback once values flush.
        let ticket = self.inner.ticket.get().wrapping_add(1);
        self.inner.ticket.set(ticket);
        let inner = self.inner.clone();
        glib::timeout_add_local_once(Duration::from_millis(140), move || {
            if inner.ticket.get() != ticket {
                return;
            }
            flush(inner);
        });
    }
}

fn flush(inner: Rc<Inner>) {
    let fans: Vec<(u8, u32)> = inner.fans.borrow_mut().drain().collect();
    let attrs: Vec<(String, String)> = inner.attrs.borrow_mut().drain().collect();
    if fans.is_empty() && attrs.is_empty() {
        return;
    }
    log::info!(
        "apply-queue flush: {} fan(s), {} fw-attr(s)",
        fans.len(),
        attrs.len()
    );

    if let Some(old) = inner.busy_toast.borrow_mut().take() {
        old.dismiss();
    }
    let t = adw::Toast::new("Applying…");
    t.set_timeout(2);
    t.set_priority(adw::ToastPriority::High);
    inner.overlay.add_toast(t.clone());
    *inner.busy_toast.borrow_mut() = Some(t);

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut errors = Vec::new();
        let mut ok_bits = Vec::new();
        for (fan, rpm) in fans {
            match send_command(DaemonCommand::SetFanTarget(fan, rpm)) {
                Ok(DaemonResponse::Ok) => {
                    if rpm == 0 {
                        ok_bits.push(format!("Fan {fan} → automatic"));
                    } else {
                        ok_bits.push(format!("Fan {fan} → {rpm} RPM"));
                    }
                }
                Ok(DaemonResponse::Error(e)) | Err(e) => {
                    log::warn!("apply-queue fan {fan}: {e}");
                    errors.push(e);
                }
                _ => errors.push(format!("Fan {fan}: unexpected response")),
            }
        }
        for (name, value) in attrs {
            match send_command(DaemonCommand::SetFwAttr {
                name: name.clone(),
                value: value.clone(),
            }) {
                Ok(DaemonResponse::Ok) => ok_bits.push(format!("{name} → {value} W")),
                Ok(DaemonResponse::Error(e)) | Err(e) => {
                    log::warn!("apply-queue {name}: {e}");
                    errors.push(e);
                }
                _ => errors.push(format!("{name}: unexpected response")),
            }
        }
        log::debug!(
            "apply-queue done: {} ok, {} err",
            ok_bits.len(),
            errors.len()
        );
        let _ = tx.send((ok_bits, errors));
    });

    glib::timeout_add_local(Duration::from_millis(40), move || match rx.try_recv() {
        Ok((oks, errs)) => {
            if let Some(old) = inner.busy_toast.borrow_mut().take() {
                old.dismiss();
            }
            if !errs.is_empty() {
                // Surface every failure, not just the first — multi-fan or
                // multi-attr batches must not collapse silently.
                let detail = if errs.len() <= 3 {
                    errs.join(" · ")
                } else {
                    format!("{} · …and {} more", errs[..3].join(" · "), errs.len() - 3)
                };
                let msg = if errs.len() == 1 {
                    detail
                } else {
                    format!("{} changes failed: {detail}", errs.len())
                };
                let label = gtk4::Label::new(Some(&msg));
                label.add_css_class("toast-error");
                let t = adw::Toast::new("");
                t.set_custom_title(Some(&label));
                t.set_timeout(4);
                inner.overlay.add_toast(t);
            } else if oks.len() > 1 {
                let t = adw::Toast::new(&format!("Applied {} changes", oks.len()));
                t.set_timeout(2);
                inner.overlay.add_toast(t);
            } else if let Some(msg) = oks.last() {
                let t = adw::Toast::new(msg);
                t.set_timeout(2);
                inner.overlay.add_toast(t);
            }
            glib::ControlFlow::Break
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(_) => glib::ControlFlow::Break,
    });
}
