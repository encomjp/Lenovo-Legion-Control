//! StatusNotifierItem (system tray) for Legion Settings.

use ksni::menu::*;
use ksni::Tray;
use std::sync::mpsc;

#[derive(Debug)]
pub enum TrayCmd {
    Show,
    Quit,
}

#[derive(Debug)]
pub struct LegionTray {
    pub tx: mpsc::Sender<TrayCmd>,
}

impl Tray for LegionTray {
    fn id(&self) -> String {
        "com.encomjp.legion-settings".into()
    }

    fn title(&self) -> String {
        "Legion Control".into()
    }

    fn icon_name(&self) -> String {
        "com.encomjp.legion-settings-tray".into()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "Legion Control".into(),
            description: "Fans, lights, and power — click to show".into(),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.tx.send(TrayCmd::Show);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: "Show Legion Control".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.tx.send(TrayCmd::Show);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.tx.send(TrayCmd::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

pub fn spawn(tx: mpsc::Sender<TrayCmd>) {
    use ksni::blocking::TrayMethods;
    let tray = LegionTray { tx };
    if let Err(e) = tray.spawn() {
        log::warn!("system tray unavailable: {e}");
    }
}
