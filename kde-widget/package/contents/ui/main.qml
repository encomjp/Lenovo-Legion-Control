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
    property string kbdBrightness: "--"
    property string logoOn: "false"
    property bool daemonOnline: false
    property string batWatts: "--"
    property string cliCommand: "bash " + Qt.resolvedUrl("legion-command.sh").toString().replace("file://", "")
    property var tempHistory: []
    property real _lastWriteTime: 0
    property int refreshInterval: Plasmoid.configuration.RefreshInterval || 2
    property bool showGauges: Plasmoid.configuration.ShowGauges !== false
    property bool showSparklines: Plasmoid.configuration.ShowSparklines !== false

    readonly property color accentRed: "#c8102e"
    readonly property color benchAmber: "#d9981a"
    readonly property color benchSteel: "#6b7280"
    readonly property string cpuDisplayName: compactHardwareName(cpuName, "CPU")
    readonly property string gpuDisplayName: compactHardwareName(gpuName, "GPU")
    readonly property string profileDisplay: profileTitle(profile)

    function compactHardwareName(name, fallback) {
        var value = (name || "").replace(/\s+/g, " ").trim()
        value = value.replace(/^AMD\s+Ryzen\s+/i, "Ryzen ")
        value = value.replace(/^NVIDIA\s+GeForce\s+/i, "GeForce ")
        value = value.replace(/\s+\d+-Core\s+Processor$/i, "")
        value = value.replace(/\s+Laptop\s+GPU$/i, "")
        return value || fallback
    }

    function profileTitle(name) {
        var key = (name || "").toLowerCase().replace(/\s+/g, "-")
        if (key === "quiet" || key === "low-power") return "Quiet"
        if (key === "balanced") return "Balanced"
        if (key === "performance") return "Performance"
        if (key === "max-power") return "Max Power"
        if (key === "custom") return "Custom"
        return name || "--"
    }

    switchWidth: Kirigami.Units.gridUnit * 18
    switchHeight: Kirigami.Units.gridUnit * 16

    // ── Compact: bench readout — temp + live status dot ────────────
    compactRepresentation: PlasmaCore.ToolTipArea {
        id: compact
        anchors.fill: parent
        Layout.minimumWidth: compactRow.implicitWidth + Kirigami.Units.smallSpacing * 3
        Layout.minimumHeight: Kirigami.Units.iconSizes.smallMedium
        Layout.preferredWidth: compactRow.implicitWidth + Kirigami.Units.smallSpacing * 3
        Layout.fillHeight: true
        mainText: "Legion Control"
        subText: {
            var l = []
            if (root.cpuTemp !== "--") l.push("CPU: " + root.cpuTemp + "°C")
            if (root.gpuTemp !== "--" && parseFloat(root.gpuTemp) >= 0) l.push("dGPU: " + root.gpuTemp + "°C" + (root.gpuPower !== "--" && parseFloat(root.gpuPower) >= 0 ? " · " + root.gpuPower + " W" : ""))
            if (root.fanCpu !== "--") l.push("Fan CPU: " + (root.fanCpu === "0" ? "Auto" : root.fanCpu + " RPM"))
            if (root.fanGpu !== "--") l.push("Fan GPU: " + (root.fanGpu === "0" ? "Auto" : root.fanGpu + " RPM"))
            if (root.batteryPct !== "--") l.push("Battery: " + root.batteryPct + "%" + (root.batWatts !== "--" && root.batWatts !== "0.0" ? " (" + root.batWatts + " W)" : ""))
            if (root.profile !== "--") l.push("Profile: " + root.profile)
            return l.join("\n")
        }
        location: Plasmoid.location
        active: !root.expanded

        readonly property color tempColor: {
            var t = parseFloat(root.cpuTemp)
            if (root.cpuTemp === "--" || isNaN(t)) return Kirigami.Theme.disabledTextColor
            if (t >= 90) return root.accentRed
            if (t >= 78) return root.benchAmber
            return Kirigami.Theme.textColor
        }

        Rectangle {
            anchors.fill: parent
            anchors.topMargin: 4
            anchors.bottomMargin: 4
            anchors.leftMargin: 1
            anchors.rightMargin: 1
            radius: 7
            color: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.09)
            opacity: hoverArea.containsMouse ? 1 : 0
            Behavior on opacity { NumberAnimation { duration: 140 } }
        }

        RowLayout {
            id: compactRow
            anchors.centerIn: parent
            spacing: Kirigami.Units.smallSpacing

            Kirigami.Icon {
                source: Qt.resolvedUrl("icons/cpu.svg")
                isMask: true
                color: compact.tempColor
                implicitWidth: Kirigami.Units.iconSizes.small
                implicitHeight: Kirigami.Units.iconSizes.small
                Layout.alignment: Qt.AlignVCenter
                opacity: 0.85
                Behavior on color { ColorAnimation { duration: 220 } }
            }

            Text {
                text: root.cpuTemp === "--" ? "—" : Math.round(parseFloat(root.cpuTemp)) + "°"
                font.pixelSize: Kirigami.Theme.defaultFont.pixelSize
                font.weight: Font.DemiBold
                Layout.alignment: Qt.AlignVCenter
                color: compact.tempColor
                Behavior on color { ColorAnimation { duration: 220 } }
            }

        }

        MouseArea {
            id: hoverArea
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: root.expanded = !root.expanded
        }
    }

    // ── Expanded: glass bench — matches app cards ──────────────────
    fullRepresentation: Item {
        id: fullRoot
        Layout.minimumWidth: Kirigami.Units.gridUnit * 20
        Layout.preferredWidth: Kirigami.Units.gridUnit * 24
        Layout.maximumWidth: Kirigami.Units.gridUnit * 28
        Layout.minimumHeight: fullCol.implicitHeight + Kirigami.Units.largeSpacing * 2
        Layout.preferredHeight: fullCol.implicitHeight + Kirigami.Units.largeSpacing * 2
        implicitWidth: Kirigami.Units.gridUnit * 24
        implicitHeight: fullCol.implicitHeight + Kirigami.Units.largeSpacing * 2

        readonly property real pagePadding: Math.max(12, Math.min(18, width * 0.04))
        readonly property real gaugeSize: Math.max(64, Math.min(80, (width - pagePadding * 2 - 28) / 2))

        QQC2.ScrollView {
            id: fullScroll
            anchors.fill: parent
            clip: true
            contentWidth: availableWidth
            QQC2.ScrollBar.horizontal.policy: QQC2.ScrollBar.AlwaysOff

            ColumnLayout {
                id: fullCol
                width: Math.max(0, fullScroll.availableWidth)
                spacing: 10

            // ── Header — same title/subtitle rhythm as the app ──
            RowLayout {
                Layout.fillWidth: true
                Layout.topMargin: 4
                Layout.leftMargin: fullRoot.pagePadding
                Layout.rightMargin: fullRoot.pagePadding
                spacing: 9

                Kirigami.Icon {
                    source: Qt.resolvedUrl("icons/cpu.svg")
                    isMask: true
                    color: Kirigami.Theme.textColor
                    implicitWidth: 18
                    implicitHeight: 18
                    Layout.alignment: Qt.AlignVCenter
                    opacity: 0.88
                }
                Text {
                    text: "LEGION CONTROL"
                    font.pixelSize: Kirigami.Theme.defaultFont.pixelSize + 1
                    font.weight: Font.Bold
                    font.letterSpacing: 0.6
                    color: Kirigami.Theme.textColor
                    Layout.alignment: Qt.AlignVCenter
                }
                Item { Layout.fillWidth: true }
            }

            // ── Metric chips — the same compact readout language as Home ──
            RowLayout {
                Layout.fillWidth: true
                Layout.leftMargin: fullRoot.pagePadding
                Layout.rightMargin: fullRoot.pagePadding
                spacing: 8
                visible: root.showGauges
                Rectangle {
                    Layout.fillWidth: true
                    Layout.preferredHeight: fullRoot.gaugeSize + 22
                    radius: 10
                    clip: true
                    color: Qt.rgba(Kirigami.Theme.backgroundColor.r, Kirigami.Theme.backgroundColor.g, Kirigami.Theme.backgroundColor.b, 0.28)
                    border.width: 1
                    border.color: Qt.rgba(1, 1, 1, 0.09)
                    Rectangle {
                        width: 3
                        anchors.left: parent.left
                        anchors.top: parent.top
                        anchors.bottom: parent.bottom
                        color: root.accentRed
                    }
                    Gauge { anchors.centerIn: parent; size: fullRoot.gaugeSize; value: parseFloat(root.cpuTemp); label: "CPU"; unit: "°C"; minValue: 20; maxValue: 100 }
                }
                Rectangle {
                    Layout.fillWidth: true
                    Layout.preferredHeight: fullRoot.gaugeSize + 22
                    radius: 10
                    clip: true
                    color: Qt.rgba(Kirigami.Theme.backgroundColor.r, Kirigami.Theme.backgroundColor.g, Kirigami.Theme.backgroundColor.b, 0.28)
                    border.width: 1
                    border.color: Qt.rgba(1, 1, 1, 0.09)
                    Rectangle {
                        width: 3
                        anchors.left: parent.left
                        anchors.top: parent.top
                        anchors.bottom: parent.bottom
                        color: root.benchSteel
                    }
                    Gauge { anchors.centerIn: parent; size: fullRoot.gaugeSize; value: parseFloat(root.gpuTemp); label: "GPU"; unit: "°C"; minValue: 20; maxValue: 100 }
                }
            }

            // ── System ───────────────────────────────────────────
            SectionCard {
                title: "SYSTEM"
                Layout.leftMargin: fullRoot.pagePadding
                Layout.rightMargin: fullRoot.pagePadding
                ColumnLayout {
                    Layout.fillWidth: true
                    MonitorRow { iconSource: Qt.resolvedUrl("icons/cpu.svg"); label: root.cpuDisplayName; temperature: root.cpuTemp; secondaryValue: parseFloat(root.cpuPower) >= 0 ? root.cpuPower + " W" : "—"; fanValue: root.fanCpu === "0" ? "AUTO" : root.fanCpu === "--" ? "" : root.fanCpu + " RPM" }
                    MonitorRow { iconSource: Qt.resolvedUrl("icons/gpu.svg"); label: root.gpuDisplayName; temperature: root.gpuTemp === "--" || parseFloat(root.gpuTemp) < 0 ? "—" : root.gpuTemp; secondaryValue: parseFloat(root.gpuPower) >= 0 ? root.gpuPower + " W" : "—"; fanValue: root.fanGpu === "0" ? "AUTO" : root.fanGpu === "--" ? "" : root.fanGpu + " RPM"; muted: parseFloat(root.gpuTemp) < 0 }
                }
            }

            // ── Battery ──────────────────────────────────────────
            SectionCard {
                title: "BATTERY"
                Layout.leftMargin: fullRoot.pagePadding
                Layout.rightMargin: fullRoot.pagePadding
                BatteryBar { percentage: root.batteryPct; batteryStatus: root.batteryStatus; chargeLimit: root.chargeLimit; watts: root.batWatts }
            }

            // ── Controls ─────────────────────────────────────────
            SectionCard {
                title: "CONTROLS"
                Layout.leftMargin: fullRoot.pagePadding
                Layout.rightMargin: fullRoot.pagePadding
                GridLayout {
                    Layout.fillWidth: true
                    Layout.minimumWidth: 0
                    columns: 2
                    columnSpacing: 8
                    rowSpacing: 6
                    QuickControl {
                        Layout.fillWidth: true
                        iconSource: Qt.resolvedUrl("icons/profile.svg"); label: "Profile"; valueText: root.profileDisplay; valueColor: Kirigami.Theme.textColor
                        onClicked: {
                            var p=["quiet","balanced","performance","max-power","custom"]
                            var current=(root.profile || "").toLowerCase().replace(/\s+/g, "-")
                            var i=p.indexOf(current); if(i<0)i=0
                            var next=p[(i+1)%p.length]
                            root.profile=next
                            root._lastWriteTime=Date.now()
                            executable.exec(root.cliCommand+" set-profile "+next)
                            refreshTimer.restart()
                        }
                    }
                    QuickControl {
                        Layout.fillWidth: true
                        iconSource: Qt.resolvedUrl("icons/fan.svg"); label: "CPU Fan"; valueText: root.fanCpu === "0" ? "Auto" : root.fanCpu + " RPM"
                        onClicked: { root._lastWriteTime = Date.now(); var pr=[0,3000,3500,4000,4500]; var c=parseInt(root.fanCpu)||0; var i=pr.indexOf(c); if(i<0)i=0; executable.exec(root.cliCommand+" set-fan 1 "+pr[(i+1)%pr.length]); refreshTimer.restart() }
                    }
                    QuickControl {
                        Layout.fillWidth: true
                        iconSource: Qt.resolvedUrl("icons/fan.svg"); label: "GPU Fan"; valueText: root.fanGpu === "0" ? "Auto" : root.fanGpu + " RPM"
                        onClicked: { root._lastWriteTime = Date.now(); var pr=[0,3000,3500,4000,4500]; var c=parseInt(root.fanGpu)||0; var i=pr.indexOf(c); if(i<0)i=0; executable.exec(root.cliCommand+" set-fan 2 "+pr[(i+1)%pr.length]); refreshTimer.restart() }
                    }
                    QuickControl {
                        Layout.fillWidth: true
                        iconSource: Qt.resolvedUrl("icons/charge-limit.svg"); label: "Charge Limit"; valueText: root.chargeLimit === "" ? "100%" : root.chargeLimit+"%"; valueColor: root.chargeLimit!==""?Kirigami.Theme.textColor:Kirigami.Theme.disabledTextColor
                        onClicked: { root._lastWriteTime = Date.now(); var L=[100,80,60]; var c=parseInt(root.chargeLimit)||100; var i=L.indexOf(c); if(i<0)i=0; executable.exec(root.cliCommand+" charge-limit "+L[(i+1)%L.length]); refreshTimer.restart() }
                    }
                }
            }

            // ── Foot — open app ──────────────────────────────────
            Item {
                Layout.fillWidth: true
                Layout.preferredHeight: 38
                Layout.topMargin: 2
                Layout.leftMargin: fullRoot.pagePadding
                Layout.rightMargin: fullRoot.pagePadding
                Layout.bottomMargin: 6

                Rectangle {
                    id: openBtnBg
                    anchors.fill: parent
                    radius: 8
                    color: openMa.pressed
                        ? Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.12)
                        : openMa.containsMouse
                            ? Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.08)
                            : Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.04)
                    border.width: 1
                    border.color: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.12)
                    Behavior on color { ColorAnimation { duration: 140 } }
                }

                Row {
                    anchors.centerIn: parent
                    spacing: 8
                    Kirigami.Icon {
                        source: "applications-system"
                        isMask: true
                        color: Kirigami.Theme.textColor
                        width: 16; height: 16
                        anchors.verticalCenter: parent.verticalCenter
                        opacity: 0.85
                    }
                    Text {
                        text: "Open Legion Control"
                        font.pixelSize: Kirigami.Theme.defaultFont.pixelSize
                        font.weight: Font.DemiBold
                        color: Kirigami.Theme.textColor
                        anchors.verticalCenter: parent.verticalCenter
                    }
                }

                MouseArea {
                    id: openMa
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: executable.exec("bash " + Qt.resolvedUrl("legion-settings.sh").toString().replace("file://",""))
                }
            }
        }
        }
    }

    Plasma5Support.DataSource { id: executable; engine: "executable"; connectedSources: []; function exec(cmd){connectSource(cmd)} onNewData: function(s,d){disconnectSource(s)} }
    Plasma5Support.DataSource {
        id: sensorSource; engine: "executable"; interval: root.refreshInterval*1000; connectedSources: [sensorCmd]
        property string sensorCmd: "bash "+Qt.resolvedUrl("legion-poll.sh").toString().replace("file://","")
        onNewData: function(sourceName, data){
            var stdout=data["stdout"]; if(!stdout||stdout.trim()===""){root.daemonOnline=false;return}
            var lines=stdout.split("\n"); var pollSucceeded=false
            for(var i=0;i<lines.length;i++){var line=lines[i].trim(); if(!line)continue; var eq=line.indexOf("="); if(eq<1)continue; var key=line.substring(0,eq); var val=line.substring(eq+1).trim(); if(!val)continue; var writeGuard=(Date.now()-root._lastWriteTime)<2500; switch(key){case "LEGION_OK":pollSucceeded=val==="1";break;case "LEGION_DAEMON_OFFLINE":case "LEGION_CLI_NOT_FOUND":pollSucceeded=false;break;case "CPU_TEMP":if(!writeGuard)cpuTemp=val;break;case "CPU_POWER":cpuPower=val;break;case "DGPU_TEMP":if(!writeGuard)gpuTemp=val;break;case "DGPU_POWER":gpuPower=val;break;case "FAN_CPU":if(!writeGuard)fanCpu=val;break;case "FAN_GPU":if(!writeGuard)fanGpu=val;break;case "FAN_AUX":if(!writeGuard)fanAux=val;break;case "BATTERY":batteryPct=val;break;case "BAT_STATUS":batteryStatus=val;break;case "CHARGE_LIMIT":chargeLimit=val==="100"?"":val;break;case "BAT_POWER":batWatts=val;break;case "PROFILE":if(!writeGuard)profile=val;break;case "KBD_BRIGHTNESS":if(!writeGuard)kbdBrightness=val;break;case "LOGO":if(!writeGuard)logoOn=val==="on"?"true":"false";break}}
            root.daemonOnline=pollSucceeded
            if(cpuTemp!=="--"){var h=root.tempHistory.slice(); h.push(parseFloat(cpuTemp)); if(h.length>30)h.shift(); root.tempHistory=h}
        }
    }
    Plasma5Support.DataSource {
        id: infoSource; engine: "executable"; connectedSources: ["bash "+Qt.resolvedUrl("legion-info.sh").toString().replace("file://","")]
        onNewData: function(sourceName, data){var stdout=data["stdout"]; if(!stdout)return; var lines=stdout.split("\n"); for(var i=0;i<lines.length;i++){var line=lines[i].trim(); var eq=line.indexOf("="); if(eq<1)continue; var key=line.substring(0,eq); var val=line.substring(eq+1).trim(); switch(key){case "CPU_NAME":cpuName=val;break;case "GPU_NAME":gpuName=val;break}} disconnectSource(sourceName)}
    }
    Timer { id: refreshTimer; interval: 800; onTriggered: sensorSource.connectSource(sensorSource.sensorCmd) }
}
