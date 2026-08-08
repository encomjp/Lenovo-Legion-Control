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

    implicitHeight: 30
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
        spacing: Kirigami.Units.smallSpacing + 2

        Kirigami.Icon {
            source: row.iconSource
            isMask: true
            color: Kirigami.Theme.textColor
            implicitWidth: 14
            implicitHeight: 14
            Layout.alignment: Qt.AlignVCenter
            opacity: row.muted ? 0.30 : 0.55
        }

        Text {
            text: row.label
            font.pixelSize: Kirigami.Theme.defaultFont.pixelSize
            font.weight: Font.Medium
            Layout.fillWidth: true
            Layout.alignment: Qt.AlignVCenter
            elide: Text.ElideRight
            color: Kirigami.Theme.textColor
            opacity: row.muted ? 0.40 : 0.92
        }

        Text {
            text: row.temperature === "--" ? "—" : row.temperature + "°C"
            font.pixelSize: Kirigami.Theme.defaultFont.pixelSize + 1
            font.weight: Font.DemiBold
            Layout.alignment: Qt.AlignVCenter
            horizontalAlignment: Text.AlignRight
            color: row.tempColor
            opacity: row.muted ? 0.45 : 1.0
            Behavior on color { ColorAnimation { duration: 220 } }
        }

        Text {
            text: row.secondaryValue
            font.pixelSize: Kirigami.Theme.smallFont.pixelSize
            Layout.preferredWidth: 52
            Layout.alignment: Qt.AlignVCenter
            horizontalAlignment: Text.AlignRight
            color: Kirigami.Theme.textColor
            opacity: 0.62
            visible: text !== ""
        }

        Text {
            text: row.fanValue
            font.pixelSize: Kirigami.Theme.smallFont.pixelSize
            Layout.preferredWidth: 76
            Layout.alignment: Qt.AlignVCenter
            horizontalAlignment: Text.AlignRight
            color: Kirigami.Theme.textColor
            opacity: row.muted ? 0.35 : 0.72
            visible: text !== ""
        }
    }
}
