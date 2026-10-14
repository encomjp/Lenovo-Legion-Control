import QtQuick 2.15
import QtQuick.Layouts 1.15
import org.kde.kirigami 2.20 as Kirigami

// Small stat card: caption, big mono value, secondary line, optional sparkline.
Rectangle {
    id: card
    property string title: ""
    property string value: "--"
    property string sub: ""
    property color accent: Kirigami.Theme.textColor
    property var history: []
    property bool showSparkline: false
    property bool dimmed: false

    Layout.fillWidth: true
    Layout.preferredWidth: 1
    implicitHeight: cardCol.implicitHeight + 20
    radius: 10
    color: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.05)
    border.width: 1
    border.color: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.07)

    Accessible.role: Accessible.StaticText
    Accessible.name: title + " " + value + " " + sub

    ColumnLayout {
        id: cardCol
        anchors.fill: parent
        anchors.margins: 10
        spacing: 1
        opacity: card.dimmed ? 0.5 : 1.0

        Text {
            text: card.title
            font.pixelSize: Kirigami.Theme.smallFont.pixelSize
            color: Kirigami.Theme.textColor
            opacity: 0.65
        }
        Text {
            text: card.value
            font.pixelSize: Math.round(Kirigami.Theme.defaultFont.pixelSize * 1.6)
            color: Kirigami.Theme.textColor
            Layout.fillWidth: true
            elide: Text.ElideRight
        }
        Text {
            text: card.sub
            font.pixelSize: Kirigami.Theme.smallFont.pixelSize
            color: Kirigami.Theme.textColor
            opacity: 0.65
            Layout.fillWidth: true
            elide: Text.ElideRight
        }
        Sparkline {
            visible: card.showSparkline && card.history.length > 1
            Layout.fillWidth: true
            Layout.preferredHeight: 18
            Layout.topMargin: 2
            points: card.history
            lineColor: card.accent
            strokeWidth: 1.4
        }
    }
}
