import QtQuick 2.15
import QtQuick.Controls 2.15 as QQC2
import QtQuick.Layouts 1.15
import org.kde.plasma.plasmoid 2.0
import org.kde.kirigami 2.20 as Kirigami
import org.kde.plasma.core as PlasmaCore
import org.kde.plasma.plasma5support 2.0 as Plasma5Support

PlasmoidItem {
    id: root
    property string cpuTemp: "--"
    property string cpuName: "CPU"
    property string gpuTemp: "--"
    property string gpuName: "dGPU"
    property string cpuPower: "--"
    property string gpuPower: "--"
    property string fanCpu: "--"
    property string fanGpu: "--"
    property string fanAux: "--"
    property string batteryPct: "--"
    property string batteryStatus: "--"
    property string chargeLimit: ""
    property string profile: "--"
    property bool daemonOnline: false
    property string batWatts: "--"
    // Keyboard backlight level ("" = unknown), Fn Lock ("" = unsupported,
    // otherwise "on"/"off") and the read-only GPU (MUX) mode label.
    property string kbdLevel: ""
    property string fnLock: ""
    property string gpuMode: ""
    // Fan targets requested from this widget (0 = auto). The daemon only
    // reports live RPM, so "manual" is tracked from what we last asked for.
    property int fanCpuTarget: 0
    property int fanGpuTarget: 0
    property var tempHistory: []
    property var gpuTempHistory: []
    property real _lastWriteTime: 0
    property int refreshInterval: Plasmoid.configuration.RefreshInterval || 2
    property bool showGauges: Plasmoid.configuration.ShowGauges !== false
    property bool showSparklines: Plasmoid.configuration.ShowSparklines !== false

    // Shell-quoted absolute path of a helper next to this file. Paths with
    // spaces or quotes survive; the helper names are fixed literals below.
    function scriptPath(name) {
        var p = decodeURIComponent(Qt.resolvedUrl(name).toString().replace(/^file:\/\//, ""))
        return "'" + p.replace(/'/g, "'\\''") + "'"
    }
    readonly property string cliCommand: "bash " + scriptPath("legion-command.sh")

    readonly property color accentRed: "#C8102E"
    readonly property string cpuDisplayName: compactHardwareName(cpuName, "CPU")
    readonly property string gpuDisplayName: compactHardwareName(gpuName, "GPU")

    // Power modes: label, legion-cli argument and accent colour.
    readonly property var modes: [
        { label: "Quiet",       short: "Quiet",    cli: "quiet",       color: "#3A7BD5" },
        { label: "Balanced",    short: "Balanced", cli: "balanced",    color: "#C8102E" },
        { label: "Performance", short: "Perf.",    cli: "performance", color: "#E0892B" },
        { label: "Extreme",     short: "Extreme",  cli: "max-power",   color: "#8E5BD6" }
    ]
    readonly property string profileKey: profileKeyOf(profile)
    readonly property int modeIndex: {
        for (var i = 0; i < modes.length; i++)
            if (modes[i].cli === profileKey) return i
        return -1
    }
    readonly property string modeLabel: modeIndex >= 0 ? modes[modeIndex].label
        : profileKey === "custom" ? "Custom" : (profile === "--" ? "—" : profile)
    // Custom (and unknown) profiles share the Extreme/Custom purple.
    readonly property color modeColor: modeIndex >= 0 ? modes[modeIndex].color
        : (profile === "--" ? Kirigami.Theme.disabledTextColor : "#8E5BD6")

    readonly property bool cpuHot: parseFloat(root.cpuTemp) >= 90
    readonly property string healthText: !daemonOnline ? "service offline" : cpuHot ? "running hot" : "healthy"
    readonly property color healthColor: !daemonOnline ? "#E67E22" : cpuHot ? "#E5484D" : "#7FD69E"

    function compactHardwareName(name, fallback) {
        var value = (name || "").replace(/\s+/g, " ").trim()
        value = value.replace(/^AMD\s+Ryzen\s+/i, "Ryzen ")
        value = value.replace(/^NVIDIA\s+GeForce\s+/i, "GeForce ")
        value = value.replace(/\s+\d+-Core\s+Processor$/i, "")
        value = value.replace(/\s+Laptop\s+GPU$/i, "")
        return value || fallback
    }

    // Map whatever `legion-cli profile` prints onto a stable key.
    function profileKeyOf(name) {
        var key = (name || "").toLowerCase().replace(/[\s\(\)]+/g, "-").replace(/-+$/, "")
        if (key === "quiet" || key === "low-power" || key === "quiet-low-power") return "quiet"
        if (key === "balanced") return "balanced"
        if (key === "performance") return "performance"
        if (key === "max-power" || key === "extreme") return "max-power"
        if (key === "custom") return "custom"
        return ""
    }

    function isNum(v) { return v !== "--" && v !== "" && !isNaN(parseFloat(v)) }
    function tempText(v) { return isNum(v) && parseFloat(v) >= 0 ? Math.round(parseFloat(v)) + "°" : "—" }
    function fanIsAuto(target) { return target === 0 }
    function rpmShort(v) {
        if (!isNum(v)) return "—"
        var n = parseInt(v)
        return n >= 1000 ? (n / 1000).toFixed(1) + "k" : String(n)
    }
    function maxFanRpm() {
        var a = isNum(fanCpu) ? parseInt(fanCpu) : 0
        var b = isNum(fanGpu) ? parseInt(fanGpu) : 0
        return Math.max(a, b)
    }

    function sendCli(args) {
        // `args` is always assembled from fixed literals / validated ints
        // and re-checked by the whitelist in legion-command.sh.
        _lastWriteTime = Date.now()
        executable.exec(cliCommand + " " + args)
        refreshTimer.restart()
    }

    function setMode(i) {
        if (!daemonOnline || i < 0 || i >= modes.length) return
        profile = modes[i].cli
        sendCli("set-profile " + modes[i].cli)
    }
    function cycleMode(step) {
        var n = modes.length
        var cur = modeIndex < 0 ? (step > 0 ? -1 : 0) : modeIndex
        setMode(((cur + step) % n + n) % n)
    }

    readonly property var fanPresets: [0, 3000, 3500, 4000, 4500]
    function nextFanPreset(cur) {
        var i = fanPresets.indexOf(cur)
        return fanPresets[(i + 1) % fanPresets.length]
    }
    function cycleFan(fan) {
        if (!daemonOnline) return
        var next = nextFanPreset(fan === 1 ? fanCpuTarget : fanGpuTarget)
        if (fan === 1) fanCpuTarget = next; else fanGpuTarget = next
        sendCli("set-fan " + fan + " " + next)
    }
    function allFansAuto() {
        if (!daemonOnline) return
        fanCpuTarget = 0
        fanGpuTarget = 0
        sendCli("fan-auto")
    }
    function toggleKeyboard() {
        if (!daemonOnline || kbdLevel === "") return
        var next = parseInt(kbdLevel) > 0 ? 0 : 2
        kbdLevel = String(next)
        sendCli("set-kbd " + next)
    }
    function setFnLock(on) {
        if (fnLock === "") return
        fnLock = on ? "on" : "off"
        sendCli("fn-lock " + (on ? "on" : "off"))
    }
    function cycleChargeLimit() {
        if (!daemonOnline) return
        var L = [100, 80, 60]
        var c = parseInt(chargeLimit) || 100
        var i = L.indexOf(c)
        var next = L[(i < 0 ? 0 : i + 1) % L.length]
        chargeLimit = next === 100 ? "" : String(next)
        sendCli("charge-limit " + next)
    }

    switchWidth: Kirigami.Units.gridUnit * 18
    switchHeight: Kirigami.Units.gridUnit * 16

    // Plasma 6: the compact representation hosts a ToolTipArea (below);
    // these helpers feed it.
    readonly property string tooltipTitle: "Legion Control · " + modeLabel

    function tooltipSubText() {
        var l = []
        if (!daemonOnline) l.push("Service offline")
        if (isNum(cpuTemp)) l.push("CPU: " + cpuTemp + "°C" + (isNum(cpuPower) ? " · " + cpuPower + " W" : ""))
        if (isNum(gpuTemp) && parseFloat(root.gpuTemp) >= 0) l.push("dGPU: " + gpuTemp + "°C" + (isNum(gpuPower) && parseFloat(gpuPower) >= 0 ? " · " + gpuPower + " W" : ""))
        if (isNum(fanCpu)) l.push("Fan CPU: " + fanCpu + " RPM" + (fanIsAuto(fanCpuTarget) ? " (auto)" : " (manual)"))
        if (isNum(fanGpu)) l.push("Fan GPU: " + fanGpu + " RPM" + (fanIsAuto(fanGpuTarget) ? " (auto)" : " (manual)"))
        if (isNum(batteryPct)) l.push("Battery: " + batteryPct + "%" + (batWatts !== "--" && batWatts !== "0.0" ? " (" + batWatts + " W)" : ""))
        l.push("Scroll to change power mode")
        return l.join("\n")
    }

    // ── Compact (panel): mode dot + CPU temp + fan glyph ─────────────
    compactRepresentation: MouseArea {
        id: compact
        // Plasma 6: formFactor is Vertical for side panels.
        readonly property bool vertical: Plasmoid.formFactor === PlasmaCore.Types.Vertical
        property real wheelAcc: 0

        hoverEnabled: true
        acceptedButtons: Qt.LeftButton
        cursorShape: Qt.PointingHandCursor
        activeFocusOnTab: true
        Accessible.role: Accessible.Button
        Accessible.name: "Legion Control, " + root.modeLabel + ", CPU " + root.tempText(root.cpuTemp)

        Layout.minimumWidth: vertical ? 0 : chip.implicitWidth
        Layout.preferredWidth: vertical ? -1 : chip.implicitWidth
        Layout.minimumHeight: vertical ? chip.implicitHeight : Kirigami.Units.iconSizes.smallMedium
        Layout.preferredHeight: vertical ? chip.implicitHeight : -1

        onClicked: root.expanded = !root.expanded
        Keys.onPressed: function(ev) {
            if (ev.key === Qt.Key_Return || ev.key === Qt.Key_Enter || ev.key === Qt.Key_Space) {
                root.expanded = !root.expanded
                ev.accepted = true
            }
        }
        // Scroll cycles power modes; one step per wheel notch (120 units).
        onWheel: function(wheel) {
            var d = wheel.angleDelta.y !== 0 ? wheel.angleDelta.y : wheel.angleDelta.x
            wheelAcc += d
            if (Math.abs(wheelAcc) >= 120) {
                root.cycleMode(wheelAcc > 0 ? -1 : 1)
                wheelAcc = 0
            }
            wheel.accepted = true
        }

        Rectangle {
            id: chip
            readonly property int pad: Kirigami.Units.smallSpacing + 2
            implicitWidth: chipGrid.implicitWidth + pad * 2
            implicitHeight: chipGrid.implicitHeight + pad * 2
            anchors.centerIn: parent
            width: compact.vertical ? Math.min(parent.width, implicitWidth) : implicitWidth
            height: compact.vertical ? implicitHeight : Math.min(parent.height - 4, implicitHeight)
            radius: 8
            color: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, compact.containsMouse ? 0.16 : 0.08)
            border.width: compact.activeFocus ? 2 : 1
            border.color: compact.activeFocus ? Kirigami.Theme.focusColor : Qt.rgba(root.modeColor.r, root.modeColor.g, root.modeColor.b, 0.85)
            Behavior on color { ColorAnimation { duration: 140 } }
            Behavior on border.color { ColorAnimation { duration: 200 } }

            GridLayout {
                id: chipGrid
                anchors.centerIn: parent
                flow: compact.vertical ? GridLayout.TopToBottom : GridLayout.LeftToRight
                rows: compact.vertical ? 3 : 1
                columns: compact.vertical ? 1 : 3
                rowSpacing: Kirigami.Units.smallSpacing
                columnSpacing: Kirigami.Units.smallSpacing + 2

                Rectangle {
                    Layout.preferredWidth: 8
                    Layout.preferredHeight: 8
                    Layout.alignment: Qt.AlignCenter
                    radius: 4
                    color: root.modeColor
                    Behavior on color { ColorAnimation { duration: 200 } }
                }
                Text {
                    Layout.alignment: Qt.AlignCenter
                    text: root.tempText(root.cpuTemp)
                    font.family: "monospace"
                    font.pixelSize: compact.vertical ? Math.max(9, Math.round(Kirigami.Theme.defaultFont.pixelSize * 0.9)) : Kirigami.Theme.defaultFont.pixelSize
                    font.weight: Font.DemiBold
                    color: root.cpuHot ? "#E5484D" : Kirigami.Theme.textColor
                }
                Kirigami.Icon {
                    Layout.alignment: Qt.AlignCenter
                    Layout.preferredWidth: Kirigami.Units.iconSizes.small
                    Layout.preferredHeight: Kirigami.Units.iconSizes.small
                    source: Qt.resolvedUrl("icons/fan.svg")
                    fallback: "applications-system"
                    isMask: true
                    color: Kirigami.Theme.textColor
                    opacity: 0.8
                }
            }
        }

        PlasmaCore.ToolTipArea {
            anchors.fill: parent
            mainText: root.tooltipTitle
            subText: root.tooltipSubText()
        }
    }

    // ── Expanded popup ───────────────────────────────────────────────
    fullRepresentation: Item {
        id: fullRoot
        readonly property int pad: 16
        Layout.minimumWidth: Kirigami.Units.gridUnit * 19
        Layout.preferredWidth: Kirigami.Units.gridUnit * 21
        Layout.maximumWidth: Kirigami.Units.gridUnit * 26
        Layout.minimumHeight: col.implicitHeight + pad * 2
        Layout.preferredHeight: col.implicitHeight + pad * 2
        implicitWidth: Kirigami.Units.gridUnit * 21
        implicitHeight: col.implicitHeight + pad * 2

        ColumnLayout {
            id: col
            anchors.fill: parent
            anchors.margins: fullRoot.pad
            spacing: 12

            // Header: brand mark + title + health text
            RowLayout {
                Layout.fillWidth: true
                spacing: 8
                AppMark {
                    Layout.preferredWidth: 22
                    Layout.preferredHeight: 22
                    Layout.alignment: Qt.AlignVCenter
                }
                Text {
                    text: "Legion Control"
                    font.pixelSize: Kirigami.Theme.defaultFont.pixelSize + 1
                    font.weight: Font.DemiBold
                    color: Kirigami.Theme.textColor
                    Layout.alignment: Qt.AlignVCenter
                }
                Item { Layout.fillWidth: true }
                Text {
                    text: root.healthText
                    font.pixelSize: Kirigami.Theme.smallFont.pixelSize
                    color: root.healthColor
                    Layout.alignment: Qt.AlignVCenter
                    Behavior on color { ColorAnimation { duration: 220 } }
                }
            }

            // Power mode segmented control
            Rectangle {
                Layout.fillWidth: true
                implicitHeight: 48
                radius: 10
                color: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.08)
                Accessible.role: Accessible.RadioButton
                Accessible.name: "Power mode"
                RowLayout {
                    anchors.fill: parent
                    anchors.margins: 4
                    spacing: 4
                    Repeater {
                        model: root.modes
                        delegate: ActionButton {
                            required property var modelData
                            required property int index
                            Layout.fillWidth: true
                            Layout.preferredWidth: 1
                            Layout.fillHeight: true
                            radio: true
                            radiusPx: 7
                            // Short text fits the 4-up segment; full name for a11y.
                            text: modelData.short
                            Accessible.name: modelData.label
                            checked: root.modeIndex === index
                            activeColor: modelData.color
                            enabled: root.daemonOnline
                            onClicked: root.setMode(index)
                        }
                    }
                }
            }

            // Offline notice — controls are dead without the daemon
            Text {
                visible: !root.daemonOnline
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                text: "Service offline — mode, fan and charge controls are unavailable."
                font.pixelSize: Kirigami.Theme.smallFont.pixelSize
                color: "#E67E22"
            }

            // CPU / GPU / Fans mini cards
            RowLayout {
                Layout.fillWidth: true
                spacing: 8
                visible: root.showGauges
                MiniCard {
                    title: "CPU"
                    value: root.tempText(root.cpuTemp)
                    sub: root.isNum(root.cpuPower) ? root.cpuPower + " W" : root.cpuDisplayName
                    accent: "#F0524F"
                    history: root.tempHistory
                    showSparkline: root.showSparklines
                }
                MiniCard {
                    title: "GPU"
                    value: root.tempText(root.gpuTemp)
                    sub: root.gpuMode !== "" ? root.gpuMode
                        : (root.isNum(root.gpuPower) && parseFloat(root.gpuPower) >= 0 ? root.gpuPower + " W" : root.gpuDisplayName)
                    accent: "#38BDF8"
                    history: root.gpuTempHistory
                    showSparkline: root.showSparklines
                    dimmed: root.isNum(root.gpuTemp) && parseFloat(root.gpuTemp) < 0
                }
                MiniCard {
                    title: "Fans"
                    value: root.rpmShort(String(root.maxFanRpm()))
                    sub: root.fanIsAuto(root.fanCpuTarget) && root.fanIsAuto(root.fanGpuTarget) ? "auto" : "manual"
                }
            }

            // Fan rows: live RPM + auto/manual; click to step through presets
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 4
                Repeater {
                    model: [
                        { name: "CPU fan", fan: 1 },
                        { name: "GPU fan", fan: 2 }
                    ]
                    delegate: ActionButton {
                        required property var modelData
                        readonly property int target: modelData.fan === 1 ? root.fanCpuTarget : root.fanGpuTarget
                        readonly property string rpm: modelData.fan === 1 ? root.fanCpu : root.fanGpu
                        Layout.fillWidth: true
                        implicitHeight: 34
                        fillColor: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.05)
                        enabled: root.daemonOnline
                        text: ""
                        Accessible.name: modelData.name + " " + (root.isNum(rpm) ? rpm + " RPM" : "unknown") + ", " + (target === 0 ? "auto" : "manual " + target + " RPM") + ". Activate to change."
                        onClicked: root.cycleFan(modelData.fan)
                        contentItem: RowLayout {
                            spacing: 8
                            Text {
                                text: modelData.name
                                Layout.leftMargin: 10
                                font.pixelSize: Kirigami.Theme.smallFont.pixelSize + 1
                                color: Kirigami.Theme.textColor
                                opacity: 0.8
                            }
                            Item { Layout.fillWidth: true }
                            Text {
                                text: !root.isNum(rpm) ? "—" : (Number(rpm) === 0 && target === 0 ? "Idle" : rpm + " RPM")
                                font.family: "monospace"
                                font.pixelSize: Kirigami.Theme.smallFont.pixelSize + 1
                                color: Kirigami.Theme.textColor
                            }
                            Rectangle {
                                Layout.rightMargin: 8
                                Layout.preferredWidth: modeTag.implicitWidth + 12
                                Layout.preferredHeight: 18
                                radius: 9
                                color: target === 0 ? Qt.rgba(0.5, 0.84, 0.62, 0.18) : Qt.rgba(0.88, 0.54, 0.17, 0.25)
                                Text {
                                    id: modeTag
                                    anchors.centerIn: parent
                                    text: target === 0 ? "auto" : "manual " + (target / 1000).toFixed(1) + "k"
                                    font.pixelSize: Kirigami.Theme.smallFont.pixelSize - 1
                                    color: target === 0 ? "#7FD69E" : "#E0892B"
                                }
                            }
                        }
                    }
                }
            }

            // Battery with charge limit (click limit to cycle 100/80/60)
            Rectangle {
                Layout.fillWidth: true
                implicitHeight: batCol.implicitHeight + 20
                radius: 10
                color: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.05)
                border.width: 1
                border.color: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.07)
                ColumnLayout {
                    id: batCol
                    anchors.fill: parent
                    anchors.leftMargin: 12
                    anchors.rightMargin: 12
                    anchors.topMargin: 10
                    anchors.bottomMargin: 10
                    spacing: 8
                    RowLayout {
                        Layout.fillWidth: true
                        Text {
                            text: "Battery " + (root.isNum(root.batteryPct) ? root.batteryPct + " %" : "—")
                                + (root.batteryStatus === "Charging" ? " · charging" : "")
                            font.pixelSize: Kirigami.Theme.defaultFont.pixelSize
                            color: Kirigami.Theme.textColor
                        }
                        Item { Layout.fillWidth: true }
                        QQC2.AbstractButton {
                            id: limitBtn
                            enabled: root.daemonOnline
                            hoverEnabled: true
                            focusPolicy: Qt.StrongFocus
                            text: "limit " + (root.chargeLimit === "" ? "100" : root.chargeLimit) + " %"
                            Accessible.name: "Charge limit " + (root.chargeLimit === "" ? "100" : root.chargeLimit) + " percent. Activate to change."
                            onClicked: root.cycleChargeLimit()
                            padding: 2
                            contentItem: Text {
                                text: limitBtn.text
                                font.pixelSize: Kirigami.Theme.smallFont.pixelSize + 1
                                color: root.chargeLimit !== "" ? "#F5A524" : Kirigami.Theme.textColor
                                opacity: limitBtn.hovered ? 1.0 : 0.65
                                font.underline: limitBtn.hovered
                            }
                            background: Rectangle {
                                color: "transparent"
                                border.width: limitBtn.visualFocus ? 2 : 0
                                border.color: Kirigami.Theme.focusColor
                                radius: 4
                            }
                        }
                    }
                    Rectangle {
                        Layout.fillWidth: true
                        Layout.preferredHeight: 6
                        radius: 3
                        color: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.12)
                        Rectangle {
                            width: parent.width * Math.max(0, Math.min(100, parseInt(root.batteryPct) || 0)) / 100
                            height: parent.height
                            radius: 3
                            color: (parseInt(root.batteryPct) || 0) <= 15 && root.batteryStatus !== "Charging" ? "#E5484D" : "#2E9E5B"
                            Behavior on width { NumberAnimation { duration: 300 } }
                        }
                    }
                }
            }

            // Quick actions
            RowLayout {
                Layout.fillWidth: true
                spacing: 8
                ActionButton {
                    Layout.fillWidth: true
                    Layout.preferredWidth: 1
                    text: "Keyboard light" + (root.kbdLevel === "" ? "" : " · " + (parseInt(root.kbdLevel) > 0 ? "on" : "off"))
                    fillColor: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.05)
                    borderColor: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.18)
                    enabled: root.daemonOnline && root.kbdLevel !== ""
                    onClicked: root.toggleKeyboard()
                }
                ActionButton {
                    Layout.fillWidth: true
                    Layout.preferredWidth: 1
                    text: "Fans · " + (root.fanIsAuto(root.fanCpuTarget) && root.fanIsAuto(root.fanGpuTarget) ? "auto" : "set auto")
                    fillColor: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.05)
                    borderColor: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.18)
                    enabled: root.daemonOnline
                    onClicked: root.allFansAuto()
                }
            }

            // System row: GPU mode (read-only) + Fn Lock (only when supported)
            RowLayout {
                Layout.fillWidth: true
                spacing: 8
                visible: root.gpuMode !== "" || root.fnLock !== ""
                Text {
                    visible: root.gpuMode !== ""
                    text: "GPU mode: " + root.gpuMode
                    font.pixelSize: Kirigami.Theme.smallFont.pixelSize + 1
                    color: Kirigami.Theme.textColor
                    opacity: 0.75
                    Accessible.name: "GPU mode " + root.gpuMode + " (read only)"
                }
                Item { Layout.fillWidth: true }
                QQC2.Switch {
                    id: fnSwitch
                    visible: root.fnLock !== ""
                    text: "Fn Lock"
                    checked: root.fnLock === "on"
                    enabled: root.daemonOnline
                    onToggled: root.setFnLock(checked)
                    Accessible.name: "Fn Lock"
                }
            }

            // Open the full app
            ActionButton {
                Layout.fillWidth: true
                text: "Open Legion Control"
                solid: true
                activeColor: root.accentRed
                onClicked: executable.exec("bash " + root.scriptPath("legion-settings.sh"))
            }
        }
    }

    Plasma5Support.DataSource { id: executable; engine: "executable"; connectedSources: []; function exec(cmd){connectSource(cmd)} onNewData: function(s,d){disconnectSource(s)} }
    Plasma5Support.DataSource {
        id: sensorSource; engine: "executable"; interval: root.refreshInterval*1000; connectedSources: [sensorCmd]
        property string sensorCmd: "bash " + root.scriptPath("legion-poll.sh")
        onNewData: function(sourceName, data){
            var stdout=data["stdout"]; if(!stdout||stdout.trim()===""){root.daemonOnline=false;return}
            var lines=stdout.split("\n"); var pollSucceeded=false
            var writeGuard=(Date.now()-root._lastWriteTime)<2500;
            for(var i=0;i<lines.length;i++){
                var line=lines[i].trim(); if(!line)continue;
                var eq=line.indexOf("="); if(eq<1)continue;
                var key=line.substring(0,eq); var val=line.substring(eq+1).trim(); if(!val)continue;
                switch(key){
                    case "LEGION_OK":pollSucceeded=val==="1";break;
                    case "LEGION_DAEMON_OFFLINE":case "LEGION_CLI_NOT_FOUND":pollSucceeded=false;break;
                    case "CPU_TEMP":root.cpuTemp=val;break;
                    case "CPU_POWER":root.cpuPower=val;break;
                    case "DGPU_TEMP":root.gpuTemp=val;break;
                    case "DGPU_POWER":root.gpuPower=val;break;
                    case "FAN_CPU":root.fanCpu=val;break;
                    case "FAN_GPU":root.fanGpu=val;break;
                    case "FAN_AUX":root.fanAux=val;break;
                    case "BATTERY":root.batteryPct=val;break;
                    case "BAT_STATUS":root.batteryStatus=val;break;
                    case "CHARGE_LIMIT":if(!writeGuard)root.chargeLimit=val==="100"?"":val;break;
                    case "BAT_POWER":root.batWatts=val;break;
                    case "PROFILE":if(!writeGuard)root.profile=val;break;
                    case "KBD_LEVEL":if(!writeGuard)root.kbdLevel=val;break;
                }
            }
            root.daemonOnline=pollSucceeded
            if(root.cpuTemp!=="--"){
                var valTemp=parseFloat(root.cpuTemp);
                if(!isNaN(valTemp)){
                    if (root.tempHistory.length === 0) {
                        root.tempHistory = [valTemp, valTemp, valTemp, valTemp]
                    } else {
                        var h=root.tempHistory.slice(); h.push(valTemp); if(h.length>30)h.shift(); root.tempHistory=h
                    }
                }
            }
            if(root.gpuTemp!=="--"){
                var gpuVal=parseFloat(root.gpuTemp);
                if(!isNaN(gpuVal) && gpuVal>=0){
                    if (root.gpuTempHistory.length === 0) {
                        root.gpuTempHistory = [gpuVal, gpuVal, gpuVal, gpuVal]
                    } else {
                        var gh=root.gpuTempHistory.slice(); gh.push(gpuVal); if(gh.length>30)gh.shift(); root.gpuTempHistory=gh
                    }
                }
            }
        }
    }
    Plasma5Support.DataSource {
        id: infoSource; engine: "executable"; connectedSources: ["bash " + root.scriptPath("legion-info.sh")]
        onNewData: function(sourceName, data){
            var stdout=data["stdout"]; if(!stdout)return
            var lines=stdout.split("\n")
            for(var i=0;i<lines.length;i++){
                var line=lines[i].trim(); var eq=line.indexOf("="); if(eq<1)continue
                var key=line.substring(0,eq); var val=line.substring(eq+1).trim()
                switch(key){
                    case "CPU_NAME":root.cpuName=val;break
                    case "GPU_NAME":root.gpuName=val;break
                    case "FN_LOCK":root.fnLock=(val==="on"||val==="off")?val:"";break
                    case "GPU_MODE":root.gpuMode=val;break
                }
            }
            disconnectSource(sourceName)
        }
    }
    function refreshNow() {
        // disconnect+connect makes the interval source run immediately
        // instead of waiting out the current interval — plain
        // connectSource on an already-connected source is a no-op.
        sensorSource.disconnectSource(sensorSource.sensorCmd)
        sensorSource.connectSource(sensorSource.sensorCmd)
    }
    // Small delay after a write so the daemon reflects it before we re-read.
    Timer { id: refreshTimer; interval: 400; onTriggered: root.refreshNow() }
}
