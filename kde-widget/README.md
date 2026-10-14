# Legion Control — KDE Plasma Widget

Live telemetry and quick controls for Lenovo Legion laptops.

## Features
- **Panel chip (compact view)** — power-mode colour dot, CPU temperature and a fan glyph. Works in horizontal and vertical panels. Click to expand, **scroll to cycle power modes**, hover for a tooltip with mode, temperatures, fans and battery.
- **Power mode selector** — Quiet / Balanced / Performance / Extreme (max-power) segmented control. Mode colours: Quiet `#3A7BD5`, Balanced `#C8102E`, Performance `#E0892B`, Extreme/Custom `#8E5BD6`.
- **CPU / GPU / Fans cards** — temperature, power, optional sparklines, GPU (MUX) mode label.
- **Fan rows** — live RPM for CPU and GPU fans with an `auto` / `manual` badge; click a row to step through manual presets (auto, 3000, 3500, 4000, 4500 RPM). "Fans · auto" restores all fans to auto.
- **Battery bar** with charge limit (click the limit to cycle 100 / 80 / 60 %).
- **Quick buttons** — Keyboard light on/off, Fans auto, **Fn Lock** switch (shown only when `legion-cli fn-lock` reports support), read-only **GPU mode** label (`legion-cli gpu-mode`), and **Open Legion Control**.
- **Configurable** — refresh interval, CPU/GPU/Fans cards toggle, sparklines toggle.
- **Theme-aware** — Kirigami colours, adapts to Breeze Dark / Light.

## Install
```bash
cd kde-widget
chmod +x install.sh
./install.sh
```

Or manually:
```bash
kpackagetool6 --type Plasma/Applet -i kde-widget/package
```

Then right-click desktop or panel → Add Widgets → search "Legion Control". The installer never restarts Plasma; if an updated widget does not refresh, re-add it or restart `plasmashell` yourself.

## Uninstall
```bash
./kde-widget/uninstall.sh
```

## Requirements
- KDE Plasma 6 (tested on 6.7.3)
- `legion-cli` installed in one of the widget's searched paths: `/usr/local/bin/legion-cli`, `/usr/bin/legion-cli`, or `~/.local/bin/legion-cli`
- `legion-daemon` running (`systemctl is-active legion-control`)

## Command safety
All writes go through `legion-command.sh`, which only forwards a fixed whitelist: `set-profile {quiet|balanced|performance|max-power|custom}`, `set-fan {1|2} {0|3000|3500|4000|4500}`, `fan-auto`, `charge-limit {60|80|100}`, `set-kbd {0|1|2}`, `fn-lock [on|off]`, and read-only `status|info|profile|battery|fan|kbd|gpu-mode`. Anything else exits 64 without running `legion-cli`.

## Files
```
kde-widget/
├── CMakeLists.txt
├── install.sh
├── uninstall.sh
├── README.md
└── package/
    ├── metadata.json
    └── contents/
        ├── config/            # config.qml, configGeneral.qml, main.xml
        └── ui/
            ├── main.qml          # PlasmoidItem: compact chip + expanded popup
            ├── AppMark.qml       # Brand mark drawn in QML (no SVG file dependency)
            ├── MiniCard.qml      # CPU / GPU / Fans card
            ├── ActionButton.qml  # Segmented-mode / quick-action button
            ├── Sparkline.qml     # Mini history chart used inside cards
            ├── legion-poll.sh    # Sensor poller (status/profile/battery/kbd)
            ├── legion-info.sh    # One-shot: CPU/GPU names, Fn Lock, GPU mode
            ├── legion-command.sh # Whitelisted legion-cli wrapper
            ├── legion-settings.sh
            └── icons/
```
`BatteryBar`, `PerfCard`, `QuickControl`, `SectionCard`, `HistoryGraph` and `_test_full.qml` are legacy components from the previous layout and are no longer used by `main.qml`.
