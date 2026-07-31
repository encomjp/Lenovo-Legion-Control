import QtQuick 2.15
import QtQuick.Controls 2.15 as QQC2
import QtQuick.Layouts 1.15
import org.kde.kirigami 2.20 as Kirigami

Item {
    id: row
    property url iconSource
    property string label: ""
    property string temperature: "--"
    property string secondaryValue: ""
    property string tertiaryValue: ""
    property string fanValue: ""
    property bool muted: false

    implicitHeight: rowLayout.implicitHeight + Kirigami.Units.smallSpacing * 2
    Layout.fillWidth: true
    visible: label !== ""

    RowLayout {
        id: rowLayout
        anchors.fill: parent
        anchors.leftMargin: Kirigami.Units.largeSpacing
        anchors.rightMargin: Kirigami.Units.largeSpacing
        spacing: Kirigami.Units.largeSpacing

        Image {
            source: row.iconSource
            sourceSize: Qt.size(18, 18)
            Layout.preferredWidth: 18
            Layout.preferredHeight: 18
            opacity: row.muted ? 0.42 : 0.82
        }

        Kirigami.Heading {
            text: row.label
            level: 5
            Layout.fillWidth: true
            elide: Text.ElideRight
            opacity: row.muted ? 0.55 : 1.0
        }

        Kirigami.Heading {
            text: row.temperature === "--" ? "--" : row.temperature + "°C"
            level: 5
            Layout.preferredWidth: 62
            horizontalAlignment: Text.AlignRight
            color: row.temperature === "--" ? Kirigami.Theme.disabledTextColor : Kirigami.Theme.textColor
        }

        QQC2.Label {
            text: row.secondaryValue
            Layout.preferredWidth: 62
            horizontalAlignment: Text.AlignRight
            opacity: 0.65
            visible: text !== ""
        }

        QQC2.Label {
            text: row.tertiaryValue
            Layout.preferredWidth: 62
            horizontalAlignment: Text.AlignRight
            opacity: 0.65
            visible: text !== ""
        }

        QQC2.Label {
            text: row.fanValue
            Layout.preferredWidth: 58
            horizontalAlignment: Text.AlignRight
            font.weight: Font.DemiBold
            visible: text !== ""
        }
    }
}
