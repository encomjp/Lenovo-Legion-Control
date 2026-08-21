import QtQuick 2.15
import QtQuick.Layouts 1.15
import org.kde.kirigami 2.20 as Kirigami

Item {
    id: row
    property url iconSource
    property string label: ""
    property string temperature: "--"
    property string secondaryValue: ""
    property string fanValue: ""
    property bool muted: false

    implicitHeight: 38
    Layout.fillWidth: true
    visible: label !== ""

    readonly property color tempColor: {
        if (row.temperature === "--" || row.temperature === "—") return Kirigami.Theme.disabledTextColor
        var t = parseFloat(row.temperature)
        if (isNaN(t)) return Kirigami.Theme.disabledTextColor
        if (t >= 90) return "#e8566e"
        if (t >= 80) return "#d9981a"
        return Kirigami.Theme.textColor
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: 2
        anchors.rightMargin: 2
        spacing: 8

        Item {
            Layout.preferredWidth: 18
            Layout.minimumWidth: 18
            Layout.maximumWidth: 18
            Layout.preferredHeight: 18
            Layout.alignment: Qt.AlignVCenter
            Kirigami.Icon {
                anchors.centerIn: parent
                source: row.iconSource
                isMask: true
                color: Kirigami.Theme.textColor
                width: 16
                height: 16
                opacity: row.muted ? 0.30 : 0.70
            }
        }

        Text {
            text: row.label
            font.pixelSize: Kirigami.Theme.defaultFont.pixelSize + 1
            font.weight: Font.Medium
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            Layout.alignment: Qt.AlignVCenter
            elide: Text.ElideRight
            color: Kirigami.Theme.textColor
            opacity: row.muted ? 0.40 : 0.92
        }

        Text {
            text: row.temperature === "--" ? "—" : row.temperature + "°C"
            font.pixelSize: Kirigami.Theme.defaultFont.pixelSize + 1
            font.weight: Font.DemiBold
            Layout.preferredWidth: 58
            Layout.minimumWidth: 58
            Layout.maximumWidth: 58
            Layout.alignment: Qt.AlignVCenter
            horizontalAlignment: Text.AlignRight
            color: row.tempColor
            opacity: row.muted ? 0.45 : 1.0
            Behavior on color { ColorAnimation { duration: 220 } }
        }

        Text {
            text: row.secondaryValue !== "" ? row.secondaryValue : " "
            font.pixelSize: Kirigami.Theme.smallFont.pixelSize
            Layout.preferredWidth: 52
            Layout.minimumWidth: 52
            Layout.maximumWidth: 52
            Layout.alignment: Qt.AlignVCenter
            horizontalAlignment: Text.AlignRight
            color: Kirigami.Theme.textColor
            opacity: row.secondaryValue !== "" ? 0.68 : 0
        }

        Text {
            text: row.fanValue !== "" ? row.fanValue : " "
            font.pixelSize: Kirigami.Theme.smallFont.pixelSize
            Layout.preferredWidth: 72
            Layout.minimumWidth: 72
            Layout.maximumWidth: 72
            Layout.alignment: Qt.AlignVCenter
            horizontalAlignment: Text.AlignRight
            color: Kirigami.Theme.textColor
            opacity: row.fanValue !== "" ? (row.muted ? 0.35 : 0.72) : 0
        }
    }
}
