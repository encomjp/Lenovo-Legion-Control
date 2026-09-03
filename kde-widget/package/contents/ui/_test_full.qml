import QtQuick 2.15
import QtQuick.Controls 2.15 as QQC2
import QtQuick.Layouts 1.15
import org.kde.kirigami 2.20 as Kirigami

// Screenshot harness mirroring main.qml's fullRepresentation (v3 premium).
// Run: qmlscene _test_full.qml  (saves /tmp/widget_v3.png, then quits)
QQC2.ApplicationWindow {
    id: root
    width: 400
    height: 640
    visible: true
    title: "widget-test-v3"
    color: "#1e1e22"

    property string cpuTemp: "64.0"
    property string gpuTemp: "55.0"
    property string gpuPower: "18.5"
    property string cpuPower: "38.5"
    property string fanCpu: "2200"
    property string fanGpu: "2400"
    property string batteryPct: "100"
    property string batteryStatus: "Full"
    property string chargeLimit: "80"
    property string profile: "Performance"
    property string batWatts: "0.0"
    property bool showGauges: true
    property bool showSparklines: true
    property bool daemonOnline: true
    property var tempHistory: [58, 60, 62, 65, 63, 66, 69, 71, 68, 70, 72, 74, 71, 73, 74, 72, 70, 68, 66, 64]
    property var gpuTempHistory: [48, 49, 51, 50, 52, 53, 52, 54, 55, 54, 55, 56, 55, 55, 55, 54, 53, 54, 55, 55]
    readonly property color accentRed: "#c8102e"
    readonly property string profileDisplay: profile
    readonly property real pagePadding: Math.max(12, Math.min(18, width * 0.04))

    Component.onCompleted: console.log("HARNESS-LOADED fullCol=" + fullCol.implicitHeight)
    Timer {
        interval: 1500
        running: true
        onTriggered: {
            console.log("HARNESS-GRAB fullCol=" + fullCol.implicitHeight + " win=" + root.width + "x" + root.height)
            fullCol.grabToImage(function(result) {
                result.saveToFile("/tmp/widget_v3.png")
                console.log("HARNESS-SAVED")
                Qt.quit()
            })
            quitTimer.start()
        }
    }
    Timer {
        id: quitTimer
        interval: 3000
        onTriggered: {
            console.log("HARNESS-QUIT-FALLBACK")
            Qt.quit()
        }
    }

    QQC2.ScrollView {
        id: fullScroll
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: footBar.top
        clip: true
        QQC2.ScrollBar.horizontal.policy: QQC2.ScrollBar.AlwaysOff

        ColumnLayout {
            id: fullCol
            width: Math.max(0, fullScroll.availableWidth)
            spacing: 11

            RowLayout {
                Layout.fillWidth: true
                Layout.topMargin: 4
                Layout.leftMargin: root.pagePadding
                Layout.rightMargin: root.pagePadding
                spacing: 9
                Image {
                    source: Qt.resolvedUrl("icons/app-mark.svg")
                    Layout.preferredWidth: 22
                    Layout.preferredHeight: 22
                    Layout.alignment: Qt.AlignVCenter
                    fillMode: Image.PreserveAspectFit
                    smooth: true
                    mipmap: true
                }
                Text {
                    text: "LEGION CONTROL"
                    font.pixelSize: Kirigami.Theme.defaultFont.pixelSize + 1
                    font.weight: Font.Bold
                    font.letterSpacing: 0.6
                    color: "white"
                    Layout.alignment: Qt.AlignVCenter
                }
                Item { Layout.fillWidth: true }
                Rectangle {
                    Layout.preferredWidth: 8
                    Layout.preferredHeight: 8
                    Layout.alignment: Qt.AlignVCenter
                    radius: 4
                    color: "#2ecc71"
                }
            }

            RowLayout {
                Layout.fillWidth: true
                Layout.leftMargin: root.pagePadding
                Layout.rightMargin: root.pagePadding
                spacing: 10
                visible: root.showGauges
                PerfCard {
                    iconSource: Qt.resolvedUrl("icons/cpu.svg")
                    chipName: "Ryzen 9 9955HX3D"
                    temp: root.cpuTemp
                    power: root.cpuPower
                    fanText: root.fanCpu + " RPM"
                    history: root.tempHistory
                    accentColor: root.accentRed
                    showSparkline: root.showSparklines
                }
                PerfCard {
                    iconSource: Qt.resolvedUrl("icons/gpu.svg")
                    chipName: "GeForce RTX 5080"
                    temp: root.gpuTemp
                    power: root.gpuPower
                    fanText: root.fanGpu + " RPM"
                    history: root.gpuTempHistory
                    accentColor: "#38bdf8"
                    showSparkline: root.showSparklines
                }
            }

            BatteryBar {
                Layout.leftMargin: root.pagePadding
                Layout.rightMargin: root.pagePadding
                percentage: root.batteryPct; batteryStatus: root.batteryStatus; chargeLimit: root.chargeLimit; watts: root.batWatts
            }

            SectionCard {
                title: "TELEMETRY HISTORY"
                badge: "64° / 55°"
                Layout.leftMargin: root.pagePadding
                Layout.rightMargin: root.pagePadding
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 6
                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 12
                        Row { spacing: 5; Rectangle { width: 8; height: 8; radius: 4; color: "#f0524f"; anchors.verticalCenter: parent.verticalCenter } Text { text: "CPU"; font.pixelSize: 10; font.weight: Font.Bold; color: "white"; opacity: 0.70; anchors.verticalCenter: parent.verticalCenter } }
                        Row { spacing: 5; Rectangle { width: 8; height: 8; radius: 4; color: "#38bdf8"; anchors.verticalCenter: parent.verticalCenter } Text { text: "GPU"; font.pixelSize: 10; font.weight: Font.Bold; color: "white"; opacity: 0.70; anchors.verticalCenter: parent.verticalCenter } }
                        Item { Layout.fillWidth: true }
                        Text { text: "30 samples"; font.pixelSize: 9; color: "white"; opacity: 0.40 }
                    }
                    HistoryGraph {
                        Layout.fillWidth: true
                        Layout.preferredHeight: 64
                        cpuPoints: root.tempHistory
                        gpuPoints: root.gpuTempHistory
                        cpuColor: "#f0524f"
                        gpuColor: "#38bdf8"
                    }
                }
            }

            SectionCard {
                title: "CONTROLS"
                Layout.leftMargin: root.pagePadding
                Layout.rightMargin: root.pagePadding
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 8
                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 8
                        Repeater {
                            model: [
                                { label: "Quiet", cli: "quiet", accent: "#3b82f6" },
                                { label: "Balanced", cli: "balanced", accent: "#06b6d4" },
                                { label: "Performance", cli: "performance", accent: "#c8102e" },
                                { label: "Custom", cli: "custom", accent: "#8b5cf6" }
                            ]
                            delegate: Rectangle {
                                id: pill
                                required property var modelData
                                readonly property bool active: root.profileDisplay === modelData.label
                                Layout.fillWidth: true
                                Layout.preferredHeight: 36
                                radius: 10
                                color: pill.active ? pill.modelData.accent : Qt.rgba(1, 1, 1, 0.04)
                                border.width: 1
                                border.color: pill.active ? Qt.lighter(pill.modelData.accent, 1.3) : Qt.rgba(1, 1, 1, 0.10)
                                Rectangle {
                                    anchors.top: parent.top
                                    anchors.left: parent.left
                                    anchors.right: parent.right
                                    anchors.topMargin: 1
                                    anchors.leftMargin: 7
                                    anchors.rightMargin: 7
                                    height: 1
                                    color: pill.active ? Qt.rgba(1, 1, 1, 0.30) : Qt.rgba(1, 1, 1, 0.07)
                                }
                                Text {
                                    anchors.centerIn: parent
                                    width: parent.width - 8
                                    text: pill.modelData.label
                                    font.pixelSize: Kirigami.Theme.smallFont.pixelSize
                                    font.weight: pill.active ? Font.Bold : Font.Medium
                                    horizontalAlignment: Text.AlignHCenter
                                    elide: Text.ElideRight
                                    color: pill.active ? "white" : Kirigami.Theme.textColor
                                }
                            }
                        }
                    }
                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 8
                        QuickControl { iconSource: Qt.resolvedUrl("icons/fan.svg"); label: "CPU Fan"; valueText: root.fanCpu + " RPM" }
                        QuickControl { iconSource: Qt.resolvedUrl("icons/fan.svg"); label: "GPU Fan"; valueText: root.fanGpu + " RPM" }
                        QuickControl { iconSource: Qt.resolvedUrl("icons/charge-limit.svg"); label: "Limit"; valueText: root.chargeLimit + "%" }
                    }
                }
            }

            Item {
                Layout.fillWidth: true
                Layout.fillHeight: true
                Layout.minimumHeight: 0
            }
        }
    }

    Item {
        id: footBar
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        height: 46
        Rectangle {
            anchors.top: parent.top
            anchors.left: parent.left
            anchors.right: parent.right
            height: 1
            color: Qt.rgba(1, 1, 1, 0.08)
        }
        Item {
            anchors.fill: parent
            anchors.leftMargin: root.pagePadding
            anchors.rightMargin: root.pagePadding
            anchors.topMargin: 6
            anchors.bottomMargin: 6
            Rectangle {
                anchors.fill: parent
                radius: 8
                color: Qt.rgba(1, 1, 1, 0.05)
                border.width: 1
                border.color: Qt.rgba(1, 1, 1, 0.12)
            }
            Text {
                anchors.centerIn: parent
                text: "Open Legion Control"
                font.pixelSize: Kirigami.Theme.defaultFont.pixelSize
                font.weight: Font.DemiBold
                color: Kirigami.Theme.textColor
            }
        }
    }
}
