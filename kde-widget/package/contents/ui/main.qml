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
    property string batWatts: "--"
    property string cliCommand: "bash " + Qt.resolvedUrl("legion-command.sh").toString().replace("file://", "")
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

            Image {
                source: Qt.resolvedUrl("icons/cpu.svg")
                sourceSize: Qt.size(Kirigami.Units.iconSizes.smallMedium, Kirigami.Units.iconSizes.smallMedium)
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
                if (!daemonOnline) l.push("Daemon offline")
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

            // === SYSTEM MONITOR ===
            SectionCard {
                ColumnLayout {
                    Layout.fillWidth: true
                    MonitorRow {
                        iconSource: Qt.resolvedUrl("icons/cpu.svg")
                        label: root.cpuName
                        temperature: root.cpuTemp
                        fanValue: root.fanCpu === "0" ? "Auto" : root.fanCpu + " RPM"
                    }
                    MonitorRow {
                        iconSource: Qt.resolvedUrl("icons/gpu.svg")
                        label: root.gpuName
                        temperature: root.gpuTemp === "--" || parseFloat(root.gpuTemp) < 0 ? "--" : root.gpuTemp
                        secondaryValue: parseFloat(root.gpuPower) >= 0 ? root.gpuPower + " W" : ""
                        fanValue: root.fanGpu === "0" ? "Auto" : root.fanGpu + " RPM"
                        muted: parseFloat(root.gpuTemp) < 0
                    }
                }
            }

            // === BATTERY ===
            SectionCard {
                BatteryBar {
                    percentage: root.batteryPct
                    batteryStatus: root.batteryStatus
                    chargeLimit: root.chargeLimit
                    watts: root.batWatts
                }
            }

            // === CONTROLS ===
            SectionCard {
                QuickControl {
                    iconSource: Qt.resolvedUrl("icons/profile.svg")
                    label: "Profile"
                    valueText: root.profile
                    valueColor: Kirigami.Theme.positiveTextColor
                    onClicked: {
                        root._lastWriteTime = Date.now()
                        var profiles = ["quiet", "balanced", "performance", "max-power", "custom"]
                        var idx = profiles.indexOf(root.profile.toLowerCase().split(" ")[0])
                        if (idx < 0) idx = 0
                        var next = profiles[(idx + 1) % profiles.length]
                        executable.exec(root.cliCommand + " set-profile " + next)
                        refreshTimer.restart()
                    }
                }
                QuickControl {
                    iconSource: Qt.resolvedUrl("icons/fan.svg")
                    label: "CPU Fan"
                    valueText: root.fanCpu === "0" ? "Auto" : root.fanCpu + " RPM"
                    onClicked: {
                        root._lastWriteTime = Date.now()
                        var presets = [0, 3000, 3500, 4000, 4500]
                        var cur = parseInt(root.fanCpu) || 0
                        var idx = presets.indexOf(cur)
                        if (idx < 0) idx = 0
                        var next = presets[(idx + 1) % presets.length]
                        executable.exec(root.cliCommand + " set-fan 1 " + next)
                        refreshTimer.restart()
                    }
                }
                QuickControl {
                    iconSource: Qt.resolvedUrl("icons/fan.svg")
                    label: "GPU Fan"
                    valueText: root.fanGpu === "0" ? "Auto" : root.fanGpu + " RPM"
                    onClicked: {
                        root._lastWriteTime = Date.now()
                        var presets = [0, 3000, 3500, 4000, 4500]
                        var cur = parseInt(root.fanGpu) || 0
                        var idx = presets.indexOf(cur)
                        if (idx < 0) idx = 0
                        var next = presets[(idx + 1) % presets.length]
                        executable.exec(root.cliCommand + " set-fan 2 " + next)
                        refreshTimer.restart()
                    }
                }
                QuickControl {
                    iconSource: Qt.resolvedUrl("icons/charge-limit.svg")
                    label: "Charge Limit"
                    valueText: root.chargeLimit === "" ? "100%" : root.chargeLimit + "%"
                    valueColor: root.chargeLimit !== "" ? Kirigami.Theme.positiveTextColor : Kirigami.Theme.disabledTextColor
                    onClicked: {
                        root._lastWriteTime = Date.now()
                        var limits = [100, 80, 60]
                        var cur = parseInt(root.chargeLimit) || 100
                        var idx = limits.indexOf(cur)
                        if (idx < 0) idx = 0
                        var next = limits[(idx + 1) % limits.length]
                        executable.exec(root.cliCommand + " charge-limit " + next)
                        refreshTimer.restart()
                    }
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

        // Resolve the helper relative to this QML file in Plasma 6.
        property string sensorCmd: "bash " + Qt.resolvedUrl("legion-poll.sh").toString().replace("file://", "")

        onNewData: function(sourceName, data) {
            var stdout = data["stdout"]
            if (!stdout || stdout.trim() === "") {
                root.daemonOnline = false
                return
            }
            var lines = stdout.split("\n")
            var pollSucceeded = false
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
                    case "LEGION_OK":
                        pollSucceeded = val === "1"
                        break
                    case "LEGION_DAEMON_OFFLINE":
                    case "LEGION_CLI_NOT_FOUND":
                        pollSucceeded = false
                        break
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
                    case "BAT_POWER":
                        batWatts = val
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
                }
            }
            root.daemonOnline = pollSucceeded

            // Push CPU temp to history for sparkline
            if (cpuTemp !== "--") {
                var h = root.tempHistory.slice()
                h.push(parseFloat(cpuTemp))
                if (h.length > 30) h.shift()
                root.tempHistory = h
            }
        }
    }

    // === ONE-SHOT STATIC INFO (CPU/GPU names) ===
    Plasma5Support.DataSource {
        id: infoSource
        engine: "executable"
        connectedSources: ["bash " + Qt.resolvedUrl("legion-info.sh").toString().replace("file://", "")]
        onNewData: function(sourceName, data) {
            var stdout = data["stdout"]
            if (!stdout) return
            var lines = stdout.split("\n")
            for (var i = 0; i < lines.length; i++) {
                var line = lines[i].trim()
                var eq = line.indexOf("=")
                if (eq < 1) continue
                var key = line.substring(0, eq)
                var val = line.substring(eq + 1).trim()
                switch (key) {
                    case "CPU_NAME": cpuName = val; break
                    case "GPU_NAME": gpuName = val; break
                }
            }
            disconnectSource(sourceName)
        }
    }

    // === RE-READ AFTER WRITE ===
    Timer {
        id: refreshTimer
        interval: 800
        onTriggered: sensorSource.connectSource(sensorSource.sensorCmd)
    }
}
