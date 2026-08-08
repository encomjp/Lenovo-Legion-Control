import QtQuick 2.15
import QtQuick.Controls 2.15 as QQC2
import QtQuick.Layouts 1.15
import org.kde.plasma.plasmoid 2.0
import org.kde.kirigami 2.20 as Kirigami
import org.kde.plasma.plasma5support 2.0 as Plasma5Support

PlasmoidItem {
    id: root
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
    property int refreshInterval: Plasmoid.configuration.RefreshInterval || 2
    property bool showGauges: Plasmoid.configuration.ShowGauges !== false
    property bool showSparklines: Plasmoid.configuration.ShowSparklines !== false

    readonly property color accentRed: "#c8102e"
    readonly property color benchAmber: "#d9981a"
    readonly property color benchSteel: "#6b7280"

    switchWidth: Kirigami.Units.gridUnit * 14
    switchHeight: Kirigami.Units.gridUnit * 10

    // ── Compact: bench readout — temp + live status dot ────────────
    compactRepresentation: Item {
        id: compact
        anchors.fill: parent
        Layout.minimumWidth: compactRow.implicitWidth + Kirigami.Units.smallSpacing * 3
        Layout.minimumHeight: Kirigami.Units.iconSizes.smallMedium
        Layout.preferredWidth: compactRow.implicitWidth + Kirigami.Units.smallSpacing * 3
        Layout.fillHeight: true

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

            Rectangle {
                implicitWidth: 6
                implicitHeight: 6
                radius: 3
                Layout.alignment: Qt.AlignVCenter
                color: root.daemonOnline ? "#1a7a3a" : "#c8102e"
                SequentialAnimation on opacity {
                    running: root.daemonOnline
                    loops: Animation.Infinite
                    NumberAnimation { to: 0.40; duration: 900; easing.type: Easing.InOutQuad }
                    NumberAnimation { to: 1.0; duration: 900; easing.type: Easing.InOutQuad }
                }
            }
        }

        MouseArea {
            id: hoverArea
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: root.expanded = !root.expanded

            QQC2.ToolTip {
                text: {
                    var l = ["Legion Control"]
                    if (root.cpuTemp !== "--") l.push("CPU: " + root.cpuTemp + "°C")
                    if (root.gpuTemp !== "--" && parseFloat(root.gpuTemp) >= 0) l.push("dGPU: " + root.gpuTemp + "°C" + (root.gpuPower !== "--" && parseFloat(root.gpuPower) >= 0 ? " · " + root.gpuPower + " W" : ""))
                    if (root.fanCpu !== "--") l.push("Fan CPU: " + (root.fanCpu === "0" ? "Auto" : root.fanCpu + " RPM"))
                    if (root.fanGpu !== "--") l.push("Fan GPU: " + (root.fanGpu === "0" ? "Auto" : root.fanGpu + " RPM"))
                    if (root.batteryPct !== "--") l.push("Battery: " + root.batteryPct + "%" + (root.batWatts !== "--" && root.batWatts !== "0.0" ? " (" + root.batWatts + " W)" : ""))
                    if (root.profile !== "--") l.push("Profile: " + root.profile)
                    if (!root.daemonOnline) l.push("Daemon offline")
                    return l.join("\n")
                }
                visible: hoverArea.containsMouse
                delay: 280
            }
        }
    }

    // ── Expanded: glass bench — matches app cards ──────────────────
    fullRepresentation: Item {
        id: fullRoot
        Layout.minimumWidth: Kirigami.Units.gridUnit * 22
        Layout.preferredWidth: Kirigami.Units.gridUnit * 24
        Layout.minimumHeight: fullCol.implicitHeight + Kirigami.Units.largeSpacing * 2
        Layout.preferredHeight: fullCol.implicitHeight + Kirigami.Units.largeSpacing * 2
        implicitWidth: Kirigami.Units.gridUnit * 24
        implicitHeight: fullCol.implicitHeight + Kirigami.Units.largeSpacing * 2

        QQC2.ScrollView {
            id: fullScroll
            anchors.fill: parent
            clip: true
            QQC2.ScrollBar.horizontal.policy: QQC2.ScrollBar.AlwaysOff

        ColumnLayout {
            id: fullCol
            width: fullScroll.availableWidth
            spacing: Kirigami.Units.smallSpacing + 2

            // ── Header — brand + status pill ─────────────────────
            RowLayout {
                Layout.fillWidth: true
                Layout.topMargin: Kirigami.Units.smallSpacing
                Layout.leftMargin: Kirigami.Units.smallSpacing
                Layout.rightMargin: Kirigami.Units.smallSpacing
                spacing: Kirigami.Units.smallSpacing

                Kirigami.Icon {
                    source: Qt.resolvedUrl("icons/cpu.svg")
                    isMask: true
                    color: Kirigami.Theme.textColor
                    implicitWidth: 15
                    implicitHeight: 15
                    Layout.alignment: Qt.AlignVCenter
                    opacity: 0.75
                }
                Text {
                    text: "LEGION CONTROL"
                    font.pixelSize: Kirigami.Theme.smallFont.pixelSize
                    font.weight: Font.Bold
                    font.letterSpacing: 1.4
                    color: Kirigami.Theme.textColor
                    opacity: 0.60
                    Layout.alignment: Qt.AlignVCenter
                }
                Item { Layout.fillWidth: true }
                Rectangle {
                    Layout.preferredHeight: 19
                    Layout.preferredWidth: statusText.implicitWidth + 16
                    Layout.alignment: Qt.AlignVCenter
                    radius: 9
                    color: root.daemonOnline ? Qt.rgba(46/255,204/255,113/255,0.14) : Qt.rgba(232/255,86/255,110/255,0.13)
                    border.width: 1
                    border.color: root.daemonOnline ? Qt.rgba(46/255,204/255,113/255,0.26) : Qt.rgba(232/255,86/255,110/255,0.24)
                    Text {
                        id: statusText
                        anchors.centerIn: parent
                        text: root.daemonOnline ? "ONLINE" : "OFFLINE"
                        font.pixelSize: Kirigami.Theme.smallFont.pixelSize - 3
                        font.weight: Font.Bold
                        font.letterSpacing: 0.8
                        color: root.daemonOnline ? "#2ecc71" : "#e8566e"
                    }
                }
            }

            // ── Gauges ───────────────────────────────────────────
            RowLayout {
                Layout.fillWidth: true
                Layout.alignment: Qt.AlignHCenter
                spacing: Kirigami.Units.largeSpacing * 2.5
                Layout.topMargin: Kirigami.Units.smallSpacing
                Layout.bottomMargin: 2
                visible: root.showGauges
                Gauge { size: 92; value: parseFloat(root.cpuTemp); label: root.cpuName; unit: "°C"; minValue: 20; maxValue: 100 }
                Gauge { size: 92; value: parseFloat(root.gpuTemp); label: root.gpuName; unit: "°C"; minValue: 20; maxValue: 100 }
            }

            // ── System ───────────────────────────────────────────
            SectionCard {
                title: "SYSTEM"
                badge: (root.fanCpu !== "--" ? "CPU " + (root.fanCpu === "0" ? "AUTO" : root.fanCpu + " RPM") : "")
                   + ((root.fanCpu !== "--" && root.fanGpu !== "--") ? " · " : "")
                   + (root.fanGpu !== "--" ? "GPU " + (root.fanGpu === "0" ? "AUTO" : root.fanGpu + " RPM") : "")
                badgeColor: Kirigami.Theme.textColor
                ColumnLayout {
                    Layout.fillWidth: true
                    MonitorRow { iconSource: Qt.resolvedUrl("icons/cpu.svg"); label: root.cpuName; temperature: root.cpuTemp }
                    MonitorRow { iconSource: Qt.resolvedUrl("icons/gpu.svg"); label: root.gpuName; temperature: root.gpuTemp === "--" || parseFloat(root.gpuTemp) < 0 ? "—" : root.gpuTemp; secondaryValue: parseFloat(root.gpuPower) >= 0 ? root.gpuPower + " W" : ""; muted: parseFloat(root.gpuTemp) < 0 }
                }
            }

            // ── Battery ──────────────────────────────────────────
            SectionCard {
                title: "BATTERY"
                BatteryBar { percentage: root.batteryPct; batteryStatus: root.batteryStatus; chargeLimit: root.chargeLimit; watts: root.batWatts }
            }

            // ── Controls ─────────────────────────────────────────
            SectionCard {
                title: "CONTROLS"
                QuickControl {
                    iconSource: Qt.resolvedUrl("icons/profile.svg"); label: "Profile"; valueText: root.profile; valueColor: Kirigami.Theme.textColor
                    onClicked: { root._lastWriteTime = Date.now(); var p=["quiet","balanced","performance","max-power","custom"]; var i=p.indexOf(root.profile.toLowerCase().split(" ")[0]); if(i<0)i=0; executable.exec(root.cliCommand+" set-profile "+p[(i+1)%p.length]); refreshTimer.restart() }
                }
                QuickControl {
                    iconSource: Qt.resolvedUrl("icons/fan.svg"); label: "CPU Fan"; valueText: root.fanCpu === "0" ? "Auto" : root.fanCpu + " RPM"
                    onClicked: { root._lastWriteTime = Date.now(); var pr=[0,3000,3500,4000,4500]; var c=parseInt(root.fanCpu)||0; var i=pr.indexOf(c); if(i<0)i=0; executable.exec(root.cliCommand+" set-fan 1 "+pr[(i+1)%pr.length]); refreshTimer.restart() }
                }
                QuickControl {
                    iconSource: Qt.resolvedUrl("icons/fan.svg"); label: "GPU Fan"; valueText: root.fanGpu === "0" ? "Auto" : root.fanGpu + " RPM"
                    onClicked: { root._lastWriteTime = Date.now(); var pr=[0,3000,3500,4000,4500]; var c=parseInt(root.fanGpu)||0; var i=pr.indexOf(c); if(i<0)i=0; executable.exec(root.cliCommand+" set-fan 2 "+pr[(i+1)%pr.length]); refreshTimer.restart() }
                }
                QuickControl {
                    iconSource: Qt.resolvedUrl("icons/charge-limit.svg"); label: "Charge Limit"; valueText: root.chargeLimit === "" ? "100%" : root.chargeLimit+"%"; valueColor: root.chargeLimit!==""?Kirigami.Theme.textColor:Kirigami.Theme.disabledTextColor
                    onClicked: { root._lastWriteTime = Date.now(); var L=[100,80,60]; var c=parseInt(root.chargeLimit)||100; var i=L.indexOf(c); if(i<0)i=0; executable.exec(root.cliCommand+" charge-limit "+L[(i+1)%L.length]); refreshTimer.restart() }
                }
            }

            // ── Foot — open app ──────────────────────────────────
            RowLayout {
                Layout.fillWidth: true
                Layout.topMargin: 2
                Layout.leftMargin: Kirigami.Units.smallSpacing
                Layout.rightMargin: Kirigami.Units.smallSpacing
                Layout.bottomMargin: Kirigami.Units.smallSpacing
                spacing: Kirigami.Units.smallSpacing
                QQC2.Button {
                    text: "Open Legion Control"
                    icon.name: "applications-system"
                    flat: true
                    Layout.fillWidth: true
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
            for(var i=0;i<lines.length;i++){var line=lines[i].trim(); if(!line)continue; var eq=line.indexOf("="); if(eq<1)continue; var key=line.substring(0,eq); var val=line.substring(eq+1).trim(); if(!val)continue; var writeGuard=(Date.now()-root._lastWriteTime)<2500; switch(key){case "LEGION_OK":pollSucceeded=val==="1";break;case "LEGION_DAEMON_OFFLINE":case "LEGION_CLI_NOT_FOUND":pollSucceeded=false;break;case "CPU_TEMP":if(!writeGuard)cpuTemp=val;break;case "DGPU_TEMP":if(!writeGuard)gpuTemp=val;break;case "DGPU_POWER":gpuPower=val;break;case "FAN_CPU":if(!writeGuard)fanCpu=val;break;case "FAN_GPU":if(!writeGuard)fanGpu=val;break;case "FAN_AUX":if(!writeGuard)fanAux=val;break;case "BATTERY":batteryPct=val;break;case "BAT_STATUS":batteryStatus=val;break;case "CHARGE_LIMIT":chargeLimit=val==="100"?"":val;break;case "BAT_POWER":batWatts=val;break;case "PROFILE":if(!writeGuard)profile=val;break;case "KBD_BRIGHTNESS":if(!writeGuard)kbdBrightness=val;break;case "LOGO":if(!writeGuard)logoOn=val==="on"?"true":"false";break}}
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
