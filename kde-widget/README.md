# Legion Control — KDE Plasma Widget

Live telemetry and quick controls for Lenovo Legion laptops.

## Features
- **Circular temperature gauges** — optional animated CPU and dGPU temperature arcs with color zones (green/yellow/orange/red)
- **System Monitor card** — CPU and dGPU rows with temperatures, fan values, and dGPU power when available
- **Battery bar** — animated fill with charging pulse, watts, and charge-limit label
- **Quick controls** — click to cycle Profile, CPU Fan, GPU Fan, and Charge Limit
- **Compact panel mode** — small icon + color-coded CPU temperature in your panel
- **Daemon status** — compact tooltip status for the legion-control service
- **Configurable** — refresh interval, gauges toggle, and a sparkline setting (the current main view does not instantiate a sparkline)
- **Theme-aware** — uses Kirigami colors, adapts to Breeze Dark / Light

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

Then right-click desktop → Add Widgets → search "Legion Control".

## Uninstall
```bash
./kde-widget/uninstall.sh
```

## Requirements
- KDE Plasma 6 (tested on 6.7.3)
- `legion-cli` installed in one of the widget's searched paths: `/usr/local/bin/legion-cli`, `/usr/bin/legion-cli`, or `~/.local/bin/legion-cli`
- `legion-daemon` running (`systemctl is-active legion-control`)

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
        ├── config/main.xml
        └── ui/
            ├── main.qml          # Main plasmoid
            ├── Gauge.qml          # Circular temp gauge
            ├── Sparkline.qml      # Reusable mini history chart (not instantiated by main.qml)
            ├── MetricCard.qml     # Reusable metric card (not instantiated by main.qml)
            ├── QuickControl.qml   # Clickable cycle control
            ├── BatteryBar.qml     # Animated battery section
            └── legion-poll.sh     # Sensor data poller
```
