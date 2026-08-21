import QtQuick 2.15
import QtQuick.Controls 2.15 as QQC2
import QtQuick.Layouts 1.15
import org.kde.kirigami 2.20 as Kirigami

QQC2.ApplicationWindow {
    id: root
    width: 432
    height: 600
    visible: true
    title: "widget-test"
    color: "#1a1a1c"

    property string cpuTemp: "73.8"
    property string gpuTemp: "47.0"
    property string gpuPower: "14.9"
    property string cpuPower: "70.3"
    property string fanCpu: "1900"
    property string fanGpu: "1800"
    property string batteryPct: "99"
    property string batteryStatus: "Not charging"
    property string chargeLimit: "80"
    property string profile: "Balanced"
    property string batWatts: "0.0"
    property bool showGauges: true
    readonly property color accentRed: "#c8102e"
    readonly property color benchSteel: "#6b7280"
    readonly property real pagePadding: Math.max(12, Math.min(18, width * 0.04))
    readonly property real gaugeSize: Math.max(64, Math.min(80, (width - pagePadding * 2 - 28) / 2))

    Timer {
        interval: 900
        running: true
        onTriggered: {
            root.grabToImage(function(result) {
                result.saveToFile("/tmp/widget_fixed.png")
                Qt.quit()
            })
        }
    }

    QQC2.ScrollView {
        id: fullScroll
        anchors.fill: parent
        clip: true
        QQC2.ScrollBar.horizontal.policy: QQC2.ScrollBar.AlwaysOff

        ColumnLayout {
            id: fullCol
            width: Math.max(0, fullScroll.availableWidth)
            spacing: 10

            RowLayout {
                Layout.fillWidth: true
                Layout.topMargin: 4
                Layout.leftMargin: root.pagePadding
                Layout.rightMargin: root.pagePadding
                spacing: 9

                Kirigami.Icon {
                    source: Qt.resolvedUrl("icons/cpu.svg")
                    isMask: true
                    color: Kirigami.Theme.textColor
                    Layout.preferredWidth: 18
                    Layout.preferredHeight: 18
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

            RowLayout {
                Layout.fillWidth: true
                Layout.leftMargin: root.pagePadding
                Layout.rightMargin: root.pagePadding
                spacing: 8
                visible: root.showGauges
                Rectangle {
                    Layout.fillWidth: true
                    Layout.preferredHeight: root.gaugeSize + 22
                    radius: 10
                    clip: true
                    color: Qt.rgba(Kirigami.Theme.backgroundColor.r, Kirigami.Theme.backgroundColor.g, Kirigami.Theme.backgroundColor.b, 0.28)
                    border.width: 1
                    border.color: Qt.rgba(1, 1, 1, 0.09)
                    Rectangle { width: 3; anchors.left: parent.left; anchors.top: parent.top; anchors.bottom: parent.bottom; color: root.accentRed }
                    Gauge { anchors.centerIn: parent; size: root.gaugeSize; value: parseFloat(root.cpuTemp); label: "CPU"; unit: "°C"; minValue: 20; maxValue: 100 }
                }
                Rectangle {
                    Layout.fillWidth: true
                    Layout.preferredHeight: root.gaugeSize + 22
                    radius: 10
                    clip: true
                    color: Qt.rgba(Kirigami.Theme.backgroundColor.r, Kirigami.Theme.backgroundColor.g, Kirigami.Theme.backgroundColor.b, 0.28)
                    border.width: 1
                    border.color: Qt.rgba(1, 1, 1, 0.09)
                    Rectangle { width: 3; anchors.left: parent.left; anchors.top: parent.top; anchors.bottom: parent.bottom; color: root.benchSteel }
                    Gauge { anchors.centerIn: parent; size: root.gaugeSize; value: parseFloat(root.gpuTemp); label: "GPU"; unit: "°C"; minValue: 20; maxValue: 100 }
                }
            }

            SectionCard {
                title: "SYSTEM"
                Layout.leftMargin: root.pagePadding
                Layout.rightMargin: root.pagePadding
                ColumnLayout {
                    Layout.fillWidth: true
                    MonitorRow { iconSource: Qt.resolvedUrl("icons/cpu.svg"); label: "Ryzen 9 9955HX3D"; temperature: root.cpuTemp; secondaryValue: root.cpuPower + " W"; fanValue: root.fanCpu + " RPM" }
                    MonitorRow { iconSource: Qt.resolvedUrl("icons/gpu.svg"); label: "GeForce RTX 5080"; temperature: root.gpuTemp; secondaryValue: root.gpuPower + " W"; fanValue: root.fanGpu + " RPM" }
                }
            }

            SectionCard {
                title: "BATTERY"
                Layout.leftMargin: root.pagePadding
                Layout.rightMargin: root.pagePadding
                BatteryBar { percentage: root.batteryPct; batteryStatus: root.batteryStatus; chargeLimit: root.chargeLimit; watts: root.batWatts }
            }

            SectionCard {
                title: "CONTROLS"
                Layout.leftMargin: root.pagePadding
                Layout.rightMargin: root.pagePadding
                GridLayout {
                    Layout.fillWidth: true
                    columns: 2
                    columnSpacing: 8
                    rowSpacing: 6
                    QuickControl { Layout.fillWidth: true; iconSource: Qt.resolvedUrl("icons/profile.svg"); label: "Profile"; valueText: root.profile }
                    QuickControl { Layout.fillWidth: true; iconSource: Qt.resolvedUrl("icons/fan.svg"); label: "CPU Fan"; valueText: root.fanCpu + " RPM" }
                    QuickControl { Layout.fillWidth: true; iconSource: Qt.resolvedUrl("icons/fan.svg"); label: "GPU Fan"; valueText: root.fanGpu + " RPM" }
                    QuickControl { Layout.fillWidth: true; iconSource: Qt.resolvedUrl("icons/charge-limit.svg"); label: "Charge Limit"; valueText: root.chargeLimit + "%" }
                }
            }

            Item {
                Layout.fillWidth: true
                Layout.preferredHeight: 38
                Layout.topMargin: 2
                Layout.leftMargin: root.pagePadding
                Layout.rightMargin: root.pagePadding
                Layout.bottomMargin: 6

                Rectangle {
                    anchors.fill: parent
                    radius: 8
                    color: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.04)
                    border.width: 1
                    border.color: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.12)
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
            }
        }
    }
}
