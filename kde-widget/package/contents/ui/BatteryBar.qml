import QtQuick 2.15
import QtQuick.Controls 2.15 as QQC2
import QtQuick.Layouts 1.15
import org.kde.kirigami 2.20 as Kirigami

Item {
    id: bat
    property string percentage: "--"
    property string batteryStatus: "Unknown"
    property string watts: "--"
    property string chargeLimit: ""

    implicitHeight: 72
    Layout.fillWidth: true

    readonly property int pct: parseInt(percentage) || 0
    readonly property color fillColor: {
        if (batteryStatus === "Charging") return "#44d62c"
        if (pct <= 15) return Kirigami.Theme.negativeTextColor
        if (pct <= 30) return Kirigami.Theme.neutralTextColor
        return "#44d62c"
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: Kirigami.Units.smallSpacing

        RowLayout {
            Layout.fillWidth: true
            spacing: Kirigami.Units.smallSpacing

            Image {
                source: Qt.resolvedUrl("icons/battery.svg")
                sourceSize: Qt.size(18, 18)
                Layout.preferredWidth: 18
                Layout.preferredHeight: 18
            }

            QQC2.Label {
                text: {
                    if (batteryStatus === "Charging") return "Battery — Charging"
                    if (batteryStatus === "Not charging") return "Battery — Full (Limit)"
                    if (batteryStatus === "Full") return "Battery — Full"
                    return "Battery — Discharging"
                }
                font.weight: Font.DemiBold
            }

            Item { Layout.fillWidth: true }

            QQC2.Label {
                visible: watts !== "--" && watts !== "0.0" && (batteryStatus === "Charging" || batteryStatus === "Discharging")
                text: (batteryStatus === "Charging" ? "+" : "−") + watts + " W"
                font.bold: true
                color: batteryStatus === "Charging" ? Kirigami.Theme.positiveTextColor : Kirigami.Theme.neutralTextColor
            }

            QQC2.Label {
                text: percentage + "%"
                font.bold: true
                font.pixelSize: Kirigami.Theme.defaultFont.pixelSize
            }
        }

        // Animated fill bar
        Item {
            Layout.fillWidth: true
            implicitHeight: 6

            Rectangle {
                anchors.fill: parent
                radius: 3
                color: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.1)
            }

            Rectangle {
                width: parent.width * (pct / 100)
                height: parent.height
                radius: 3
                color: bat.fillColor
                Behavior on width { NumberAnimation { duration: 500; easing.type: Easing.OutCubic } }
                Behavior on color { ColorAnimation { duration: 400 } }

                SequentialAnimation on opacity {
                    running: batteryStatus === "Charging"
                    loops: Animation.Infinite
                    NumberAnimation { to: 0.7; duration: 1200 }
                    NumberAnimation { to: 1.0; duration: 1200 }
                }
            }
        }

        QQC2.Label {
            visible: chargeLimit !== ""
            text: "Charge limit: " + chargeLimit
            font.pixelSize: Kirigami.Theme.smallFont.pixelSize
            color: Kirigami.Theme.positiveTextColor
            opacity: 0.7
        }
    }
}
