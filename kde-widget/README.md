# Legion Control — KDE Plasma Widget

Live telemetry and quick controls for Lenovo Legion laptops.

## Features
- **Circular temperature gauges** — animated SVG arcs with color zones (green/yellow/orange/red)
- **Real-time sparklines** — mini history charts for CPU temperature (last 30 samples)
- **Metric cards** — CPU, dGPU, fan RPMs with icons and color-coded values
- **Battery bar** — animated fill with charging pulse and charge limit badge
- **Quick controls** — click to cycle profile, fan, KB brightness, logo, charge limit
- **Daemon status** — green/red dot showing if legion-daemon is connected
- **Compact panel mode** — small icon + color-coded CPU temp in your panel
- **Configurable** — refresh interval, gauges toggle, sparklines toggle
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
- `legion-cli` installed at `/usr/local/bin/legion-cli`
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
            ├── Sparkline.qml      # Mini history chart
            ├── MetricCard.qml     # Sensor row with icon
            ├── QuickControl.qml   # Clickable cycle control
            ├── BatteryBar.qml     # Animated battery section
            └── legion-poll.sh     # Sensor data poller
```
