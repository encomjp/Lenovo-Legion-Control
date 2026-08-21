import QtQuick 2.15
import QtQuick.Layouts 1.15
import org.kde.kirigami 2.20 as Kirigami

Item {
    id: bat
    property string percentage: "--"
    property string batteryStatus: "Unknown"
    property string watts: "--"
    property string chargeLimit: ""

    implicitHeight: 64
    Layout.fillWidth: true

    readonly property int pct: Math.max(0, Math.min(100, parseInt(percentage) || 0))
    readonly property color fillColor: {
        if (batteryStatus === "Charging") return "#2ecc71"
        if (pct <= 15) return "#e8566e"
        if (pct <= 30) return "#d9981a"
        return "#2ecc71"
    }
    readonly property string stateText: {
        if (batteryStatus === "Charging") return "CHARGING"
        if (batteryStatus === "Not charging") return "LIMIT HOLD"
        if (batteryStatus === "Full") return "FULL"
        return "DISCHARGING"
    }
    readonly property color stateColor: {
        if (batteryStatus === "Charging" || batteryStatus === "Full") return "#2ecc71"
        if (batteryStatus === "Not charging") return "#d9981a"
        return Kirigami.Theme.textColor
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.topMargin: 2
        anchors.bottomMargin: 2
        spacing: 8

        RowLayout {
            Layout.fillWidth: true
            spacing: 8

            Kirigami.Icon {
                source: Qt.resolvedUrl("icons/battery.svg")
                isMask: true
                color: Kirigami.Theme.textColor
                Layout.preferredWidth: 16
                Layout.preferredHeight: 16
                Layout.alignment: Qt.AlignVCenter
                opacity: 0.70
            }
            Text {
                text: bat.stateText
                font.pixelSize: Kirigami.Theme.smallFont.pixelSize
                font.weight: Font.DemiBold
                font.letterSpacing: 0.8
                Layout.alignment: Qt.AlignVCenter
                color: bat.stateColor
                opacity: 0.90
            }
            Text {
                visible: bat.chargeLimit !== ""
                text: "· LIMIT " + bat.chargeLimit + "%"
                font.pixelSize: Kirigami.Theme.smallFont.pixelSize
                font.weight: Font.DemiBold
                font.letterSpacing: 0.5
                Layout.alignment: Qt.AlignVCenter
                color: "#d9981a"
                opacity: 0.90
            }
            Item { Layout.fillWidth: true }
            Text {
                visible: bat.watts !== "--" && bat.watts !== "0.0" && (bat.batteryStatus === "Charging" || bat.batteryStatus === "Discharging")
                text: (bat.batteryStatus === "Charging" ? "+" : "−") + bat.watts + " W"
                font.pixelSize: Kirigami.Theme.smallFont.pixelSize
                font.weight: Font.DemiBold
                Layout.alignment: Qt.AlignVCenter
                color: bat.batteryStatus === "Charging" ? "#2ecc71" : Kirigami.Theme.textColor
                opacity: 0.80
            }
            Text {
                text: bat.percentage + "%"
                font.pixelSize: Kirigami.Theme.defaultFont.pixelSize + 1
                font.weight: Font.DemiBold
                Layout.alignment: Qt.AlignVCenter
                color: Kirigami.Theme.textColor
            }
        }

        Item {
            Layout.fillWidth: true
            implicitHeight: 8

            Rectangle {
                id: track
                anchors.fill: parent
                radius: 4
                color: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.08)
                border.width: 1
                border.color: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.05)
            }

            Item {
                anchors.fill: parent
                anchors.margins: 1
                clip: true

                Rectangle {
                    width: Math.max(0, parent.width * (bat.pct / 100))
                    height: parent.height
                    radius: 3
                    color: bat.fillColor
                    Behavior on width { NumberAnimation { duration: 440; easing.type: Easing.OutCubic } }
                    Behavior on color { ColorAnimation { duration: 320 } }
                    SequentialAnimation on opacity {
                        running: bat.batteryStatus === "Charging"
                        loops: Animation.Infinite
                        NumberAnimation { to: 0.70; duration: 1100; easing.type: Easing.InOutQuad }
                        NumberAnimation { to: 1.0; duration: 1100; easing.type: Easing.InOutQuad }
                    }
                }
            }
        }
    }
}
