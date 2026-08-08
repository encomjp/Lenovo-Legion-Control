import QtQuick 2.15
import QtQuick.Controls 2.15 as QQC2
import QtQuick.Layouts 1.15
import org.kde.kirigami 2.20 as Kirigami

QQC2.ApplicationWindow {
    id: root
    width: 460
    height: 720
    visible: true
    title: "widget-test"
    color: "#1a1a1c"

    property string cpuTemp: "78.2"
    property string cpuName: "CPU"
    property string gpuTemp: "51.0"
    property string gpuName: "dGPU"
    property string gpuPower: "28.4"
    property string fanCpu: "4400"
    property string fanGpu: "4500"
    property string batteryPct: "100"
    property string batteryStatus: "Not charging"
    property string chargeLimit: "80"
    property string profile: "Max Power"
    property bool daemonOnline: true
    property string batWatts: "0.0"
    property bool showGauges: true

    QQC2.ScrollView {
        id: fullScroll
        anchors.fill: parent
        clip: true
        QQC2.ScrollBar.horizontal.policy: QQC2.ScrollBar.AlwaysOff

        ColumnLayout {
            id: fullCol
            width: fullScroll.availableWidth
            spacing: Kirigami.Units.smallSpacing + 2

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

            SectionCard {
                title: "SYSTEM"
                badge: "CPU " + root.fanCpu + " RPM · GPU " + root.fanGpu + " RPM"
                badgeColor: Kirigami.Theme.textColor
                ColumnLayout {
                    Layout.fillWidth: true
                    MonitorRow { iconSource: Qt.resolvedUrl("icons/cpu.svg"); label: root.cpuName; temperature: root.cpuTemp }
                    MonitorRow { iconSource: Qt.resolvedUrl("icons/gpu.svg"); label: root.gpuName; temperature: root.gpuTemp; secondaryValue: root.gpuPower + " W" }
                }
            }

            SectionCard {
                title: "BATTERY"
                BatteryBar { percentage: root.batteryPct; batteryStatus: root.batteryStatus; chargeLimit: root.chargeLimit; watts: root.batWatts }
            }

            SectionCard {
                title: "CONTROLS"
                QuickControl { iconSource: Qt.resolvedUrl("icons/profile.svg"); label: "Profile"; valueText: root.profile }
                QuickControl { iconSource: Qt.resolvedUrl("icons/fan.svg"); label: "CPU Fan"; valueText: root.fanCpu + " RPM" }
                QuickControl { iconSource: Qt.resolvedUrl("icons/fan.svg"); label: "GPU Fan"; valueText: root.fanGpu + " RPM" }
                QuickControl { iconSource: Qt.resolvedUrl("icons/charge-limit.svg"); label: "Charge Limit"; valueText: root.chargeLimit + "%" }
            }

            RowLayout {
                Layout.fillWidth: true
                Layout.topMargin: 2
                Layout.leftMargin: Kirigami.Units.smallSpacing
                Layout.rightMargin: Kirigami.Units.smallSpacing
                Layout.bottomMargin: Kirigami.Units.smallSpacing
                QQC2.Button {
                    text: "Open Legion Control"
                    icon.name: "applications-system"
                    flat: true
                    Layout.fillWidth: true
                }
            }
        }
    }
}
