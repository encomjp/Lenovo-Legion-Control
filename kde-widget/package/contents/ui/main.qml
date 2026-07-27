import QtQuick 2.15
import QtQuick.Controls 2.15 as QQC2
import QtQuick.Layouts 1.15
import org.kde.plasma.plasmoid 2.0
import org.kde.kirigami 2.20 as Kirigami
import org.kde.plasma.plasma5support 2.0 as Plasma5Support

PlasmoidItem {
    id: root

    // --- Sensor properties ---
    property string cpuTemp: "--"
    property string cpuName: "CPU"
    property string gpuTemp: "--"
    property string gpuName: "dGPU"
    property string gpuPower: "--"
    property string fanCpu: "--"
    property string fanGpu: "--"
    property string fanAux: "--"
    property string batteryPct: "--"
    property string batteryStatus: "--"
    property string chargeLimit: ""
    property string profile: "--"
    property string kbdBrightness: "--"
    property string logoOn: "false"
    property bool daemonOnline: false
    property var tempHistory: []
    property real _lastWriteTime: 0

    // --- Config ---
    property int refreshInterval: Plasmoid.configuration.RefreshInterval || 2
    property bool showGauges: Plasmoid.configuration.ShowGauges !== false
    property bool showSparklines: Plasmoid.configuration.ShowSparklines !== false

    switchWidth: Kirigami.Units.gridUnit * 14
    switchHeight: Kirigami.Units.gridUnit * 10

    // === COMPACT (panel) ===
    compactRepresentation: MouseArea {
        id: compact
        Layout.minimumWidth: compactRow.implicitWidth + Kirigami.Units.largeSpacing * 2
        Layout.minimumHeight: Kirigami.Units.iconSizes.small * 1.5
        hoverEnabled: true
        onClicked: root.expanded = !root.expanded

        RowLayout {
            id: compactRow
            anchors.centerIn: parent
            spacing: Kirigami.Units.smallSpacing

            Kirigami.Icon {
                source: "legion-settings"
                Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
                Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
            }

            QQC2.Label {
                text: cpuTemp !== "--" ? cpuTemp + "°" : ""
                font.pixelSize: Kirigami.Theme.smallFont.pixelSize
                color: {
                    var t = parseFloat(cpuTemp)
                    if (t > 90) return Kirigami.Theme.negativeTextColor
                    if (t > 75) return Kirigami.Theme.neutralTextColor
                    return Kirigami.Theme.textColor
                }
            }
        }

        QQC2.ToolTip {
            text: {
                var l = ["Legion Control"]
                if (cpuTemp !== "--") l.push("CPU: " + cpuTemp + "°C")
                if (gpuTemp !== "--") l.push("dGPU: " + gpuTemp + "°C")
                if (fanCpu !== "--") l.push("Fan CPU: " + (fanCpu === "0" ? "Auto" : fanCpu + " RPM"))
                if (batteryPct !== "--") l.push("Battery: " + batteryPct + "%")
                if (profile !== "--") l.push("Profile: " + profile)
                if (!daemonOnline) l.push("⚠ Daemon offline")
                return l.join("\n")
            }
            visible: compact.containsMouse
            delay: 300
        }
    }

    // === FULL (expanded) ===
    fullRepresentation: QQC2.ScrollView {
        Layout.minimumWidth: Kirigami.Units.gridUnit * 24
        Layout.preferredWidth: Kirigami.Units.gridUnit * 26
        Layout.maximumWidth: Kirigami.Units.gridUnit * 30

        ColumnLayout {
            id: fullCol
            width: parent.width
            spacing: Kirigami.Units.largeSpacing

            // === HEADER ===
            RowLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.largeSpacing

                Kirigami.Icon {
                    source: "legion-settings"
                    Layout.preferredWidth: Kirigami.Units.iconSizes.large
                    Layout.preferredHeight: Kirigami.Units.iconSizes.large
                }

                Kirigami.Heading {
                    text: "Legion Control"
                    level: 3
                    font.weight: Font.DemiBold
                }

                Item { Layout.fillWidth: true }

                Rectangle {
                    Layout.preferredWidth: 8
                    Layout.preferredHeight: 8
                    radius: 4
                    color: root.daemonOnline ? "#44d62c" : "#ff4444"
                    Behavior on color { ColorAnimation { duration: 300 } }
                }

                QQC2.Label {
                    text: root.daemonOnline ? "Online" : "Offline"
                    font.pixelSize: Kirigami.Theme.smallFont.pixelSize
                    color: root.daemonOnline ? Kirigami.Theme.positiveTextColor : Kirigami.Theme.negativeTextColor
                }
            }

            // === TEMPERATURE GAUGES ===
            RowLayout {
                Layout.fillWidth: true
                Layout.alignment: Qt.AlignHCenter
                spacing: Kirigami.Units.largeSpacing * 2
                visible: root.showGauges

                Gauge {
                    size: 80
                    value: parseFloat(root.cpuTemp)
                    label: root.cpuName
                    unit: "°C"
                    minValue: 20
                    maxValue: 100
                }

                Gauge {
                    size: 80
                    value: parseFloat(root.gpuTemp)
                    label: root.gpuName
                    unit: "°C"
                    minValue: 20
                    maxValue: 100
                }
            }

            // === METRIC CARDS ===
            Rectangle {
                Layout.fillWidth: true
                implicitHeight: metricsCol.implicitHeight + Kirigami.Units.largeSpacing * 2
                radius: Kirigami.Units.largeSpacing
                color: Qt.rgba(Kirigami.Theme.backgroundColor.r, Kirigami.Theme.backgroundColor.g, Kirigami.Theme.backgroundColor.b, 0.3)
                border.width: 1
                border.color: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.1)

                ColumnLayout {
                    id: metricsCol
                    anchors.fill: parent
                    anchors.margins: Kirigami.Units.smallSpacing
                    spacing: Kirigami.Units.smallSpacing

                    MetricCard {
                        iconSource: "cpu-symbolic"
                        label: root.cpuName
                        value: root.cpuTemp
                        unit: "°C"
                        valueColor: {
                            var t = parseFloat(root.cpuTemp)
                            if (t > 90) return Kirigami.Theme.negativeTextColor
                            if (t > 75) return Kirigami.Theme.neutralTextColor
                            return Kirigami.Theme.positiveTextColor
                        }
                        showSparkline: root.showSparklines
                        sparkPoints: root.tempHistory
                        sparkColor: "#44d62c"
                    }

                    MetricCard {
                        iconSource: "video-display-symbolic"
                        label: root.gpuName
                        value: root.gpuTemp === "--" || parseFloat(root.gpuTemp) < 0 ? "Off" : root.gpuTemp
                        unit: "°C"
                        subValue: root.gpuPower >= 0 ? root.gpuPower : ""
                        subUnit: "W"
                        valueColor: {
                            var t = parseFloat(root.gpuTemp)
                            if (t < 0) return Kirigami.Theme.disabledTextColor
                            if (t > 85) return Kirigami.Theme.negativeTextColor
                            if (t > 70) return Kirigami.Theme.neutralTextColor
                            return Kirigami.Theme.positiveTextColor
                        }
                    }

                    MetricCard {
                        iconSource: "speedometer-symbolic"
                        label: "CPU Fan"
                        value: root.fanCpu === "0" ? "Auto" : root.fanCpu
                        unit: " RPM"
                    }

                    MetricCard {
                        iconSource: "speedometer-symbolic"
                        label: "GPU Fan"
                        value: root.fanGpu === "0" ? "Auto" : root.fanGpu
                        unit: " RPM"
                    }

                    MetricCard {
                        iconSource: "speedometer-symbolic"
                        label: "Aux Fan"
                        value: root.fanAux === "0" ? "Auto" : root.fanAux
                        unit: " RPM"
                    }
                }
            }

            // === BATTERY ===
            BatteryBar {
                Layout.fillWidth: true
                percentage: root.batteryPct
                status: root.batteryStatus
                chargeLimit: root.chargeLimit
            }

            // === QUICK CONTROLS ===
            Rectangle {
                Layout.fillWidth: true
                implicitHeight: quickCol.implicitHeight + Kirigami.Units.largeSpacing * 2
                radius: Kirigami.Units.largeSpacing
                color: Qt.rgba(Kirigami.Theme.backgroundColor.r, Kirigami.Theme.backgroundColor.g, Kirigami.Theme.backgroundColor.b, 0.3)
                border.width: 1
                border.color: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.1)

                ColumnLayout {
                    id: quickCol
                    anchors.fill: parent
                    anchors.margins: Kirigami.Units.smallSpacing
                    spacing: 0

                    QuickControl {
                        iconSource: "system-run"
                        label: "Profile"
                        valueText: root.profile
                        valueColor: Kirigami.Theme.positiveTextColor
                        onClicked: {
                            root._lastWriteTime = Date.now()
                            var profiles = ["quiet", "balanced", "performance", "max-power", "custom"]
                            var idx = profiles.indexOf(root.profile.toLowerCase().split(" ")[0])
                            if (idx < 0) idx = 0
                            var next = profiles[(idx + 1) % profiles.length]
                            executable.exec("legion-cli set-profile " + next)
                            refreshTimer.restart()
                        }
                    }

                    Rectangle { Layout.fillWidth: true; Layout.leftMargin: Kirigami.Units.largeSpacing; Layout.rightMargin: Kirigami.Units.largeSpacing; implicitHeight: 1; color: Kirigami.Theme.alternateBackgroundColor }

                    QuickControl {
                        iconSource: "speedometer-symbolic"
                        label: "CPU Fan"
                        valueText: root.fanCpu === "0" ? "Auto" : root.fanCpu + " RPM"
                        onClicked: {
                            root._lastWriteTime = Date.now()
                            var presets = [0, 3000, 3500, 4000, 4500]
                            var cur = parseInt(root.fanCpu) || 0
                            var idx = presets.indexOf(cur)
                            if (idx < 0) idx = 0
                            var next = presets[(idx + 1) % presets.length]
                            executable.exec("legion-cli set-fan 1 " + next)
                            refreshTimer.restart()
                        }
                    }

                    Rectangle { Layout.fillWidth: true; Layout.leftMargin: Kirigami.Units.largeSpacing; Layout.rightMargin: Kirigami.Units.largeSpacing; implicitHeight: 1; color: Kirigami.Theme.alternateBackgroundColor }

                    QuickControl {
                        iconSource: "brightness-high-symbolic"
                        label: "KB Brightness"
                        valueText: {
                            var b = parseInt(root.kbdBrightness)
                            if (b === 0) return "Off"
                            if (b === 1) return "Low"
                            if (b === 2) return "High"
                            return "--"
                        }
                        onClicked: {
                            root._lastWriteTime = Date.now()
                            var cur = parseInt(root.kbdBrightness) || 0
                            var next = (cur + 1) % 3
                            executable.exec("legion-cli set-kbd " + next)
                            refreshTimer.restart()
                        }
                    }

                    Rectangle { Layout.fillWidth: true; Layout.leftMargin: Kirigami.Units.largeSpacing; Layout.rightMargin: Kirigami.Units.largeSpacing; implicitHeight: 1; color: Kirigami.Theme.alternateBackgroundColor }

                    QuickControl {
                        iconSource: "preferences-desktop-display-color"
                        label: "Logo LED"
                        valueText: root.logoOn === "true" ? "On" : "Off"
                        on: root.logoOn === "true"
                        valueColor: root.logoOn === "true" ? Kirigami.Theme.positiveTextColor : Kirigami.Theme.disabledTextColor
                        onClicked: {
                            root._lastWriteTime = Date.now()
                            var next = root.logoOn === "true" ? "off" : "on"
                            executable.exec("legion-cli set-logo " + next)
                            refreshTimer.restart()
                        }
                    }

                    Rectangle { Layout.fillWidth: true; Layout.leftMargin: Kirigami.Units.largeSpacing; Layout.rightMargin: Kirigami.Units.largeSpacing; implicitHeight: 1; color: Kirigami.Theme.alternateBackgroundColor }

                    QuickControl {
                        iconSource: "battery-good-charging-symbolic"
                        label: "Charge Limit"
                        valueText: {
                            if (root.chargeLimit === "") return "100%"
                            return root.chargeLimit + "%"
                        }
                        valueColor: root.chargeLimit !== "" ? Kirigami.Theme.positiveTextColor : Kirigami.Theme.disabledTextColor
                        onClicked: {
                            root._lastWriteTime = Date.now()
                            var limits = [100, 80, 60]
                            var cur = parseInt(root.chargeLimit) || 100
                            var idx = limits.indexOf(cur)
                            if (idx < 0) idx = 0
                            var next = limits[(idx + 1) % limits.length]
                            executable.exec("legion-cli charge-limit " + next)
                            refreshTimer.restart()
                        }
                    }
                }
            }

            // === FOOTER ===
            RowLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.smallSpacing

                QQC2.Button {
                    text: "Open Settings"
                    icon.name: "preferences-system"
                    onClicked: executable.exec("legion-settings")
                }

                Item { Layout.fillWidth: true }

                QQC2.Label {
                    text: root.cpuName
                    font.pixelSize: Kirigami.Theme.smallFont.pixelSize
                    opacity: 0.5
                }
            }
        }
    }

    // === EXECUTOR ===
    Plasma5Support.DataSource {
        id: executable
        engine: "executable"
        connectedSources: []
        function exec(cmd) { connectSource(cmd) }
        onNewData: function(sourceName, data) { disconnectSource(sourceName) }
    }

    // === SENSOR POLLER ===
    Plasma5Support.DataSource {
        id: sensorSource
        engine: "executable"
        interval: root.refreshInterval * 1000
        connectedSources: [sensorCmd]

        // Resolve poll script: use Plasmoid internal file path
        property string sensorCmd: "bash " + Plasmoid.file("contents/ui/legion-poll.sh")

        onNewData: function(sourceName, data) {
            var stdout = data["stdout"]
            if (!stdout || stdout.trim() === "") {
                root.daemonOnline = false
                return
            }
            root.daemonOnline = true

            var lines = stdout.split("\n")
            for (var i = 0; i < lines.length; i++) {
                var line = lines[i].trim()
                if (!line) continue
                var eq = line.indexOf("=")
                if (eq < 1) continue
                var key = line.substring(0, eq)
                var val = line.substring(eq + 1).trim()
                if (!val) continue

                var writeGuard = (Date.now() - root._lastWriteTime) < 2500

                switch (key) {
                    case "CPU_TEMP":
                        if (!writeGuard) cpuTemp = val
                        break
                    case "DGPU_TEMP":
                        if (!writeGuard) gpuTemp = val
                        break
                    case "DGPU_POWER":
                        gpuPower = val
                        break
                    case "FAN_CPU":
                        if (!writeGuard) fanCpu = val
                        break
                    case "FAN_GPU":
                        if (!writeGuard) fanGpu = val
                        break
                    case "FAN_AUX":
                        if (!writeGuard) fanAux = val
                        break
                    case "BATTERY":
                        batteryPct = val
                        break
                    case "BAT_STATUS":
                        batteryStatus = val
                        break
                    case "CHARGE_LIMIT":
                        chargeLimit = val === "100" ? "" : val
                        break
                    case "PROFILE":
                        if (!writeGuard) profile = val
                        break
                    case "KBD_BRIGHTNESS":
                        if (!writeGuard) kbdBrightness = val
                        break
                    case "LOGO":
                        if (!writeGuard) logoOn = val === "on" ? "true" : "false"
                        break
                    case "CPU_NAME":
                        cpuName = val
                        break
                    case "GPU_NAME":
                        gpuName = val
                        break
                }
            }

            // Push CPU temp to history for sparkline
            if (cpuTemp !== "--") {
                tempHistory.push(parseFloat(cpuTemp))
                if (tempHistory.length > 30) tempHistory.shift()
            }
        }
    }

    // === RE-READ AFTER WRITE ===
    Timer {
        id: refreshTimer
        interval: 800
        onTriggered: sensorSource.connectSource(sensorSource.sensorCmd)
    }
}
