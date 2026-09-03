import QtQuick 2.15
import QtQuick.Layouts 1.15
import org.kde.kirigami 2.20 as Kirigami

Item {
    id: card
    property string title: ""
    property string badge: ""
    property color badgeColor: Kirigami.Theme.textColor
    default property alias content: contentHost.data

    implicitHeight: col.implicitHeight + Kirigami.Units.smallSpacing * 2 + 6
    Layout.fillWidth: true

    Rectangle {
        anchors.fill: parent
        radius: 12
        color: Qt.rgba(Kirigami.Theme.backgroundColor.r, Kirigami.Theme.backgroundColor.g, Kirigami.Theme.backgroundColor.b, 0.32)
        border.width: 1
        border.color: Qt.rgba(1, 1, 1, 0.09)
    }

    ColumnLayout {
        id: col
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.leftMargin: Kirigami.Units.largeSpacing - 4
        anchors.rightMargin: Kirigami.Units.largeSpacing - 4
        anchors.topMargin: Kirigami.Units.smallSpacing + 1
        anchors.bottomMargin: Kirigami.Units.smallSpacing + 2
        spacing: 5

        RowLayout {
            id: headerRow
            Layout.fillWidth: true
            spacing: Kirigami.Units.smallSpacing
            visible: card.title !== ""

            Text {
                text: card.title
                font.pixelSize: Kirigami.Theme.smallFont.pixelSize + 1
                font.weight: Font.Bold
                font.letterSpacing: 1.2
                color: Kirigami.Theme.textColor
                opacity: 0.72
                Layout.alignment: Qt.AlignVCenter
            }
            Item { Layout.fillWidth: true }
            Text {
                visible: card.badge !== ""
                text: card.badge
                font.pixelSize: Kirigami.Theme.smallFont.pixelSize - 1
                font.weight: Font.DemiBold
                font.letterSpacing: 0.6
                color: card.badgeColor
                Layout.alignment: Qt.AlignVCenter
            }
        }

        Rectangle {
            Layout.fillWidth: true
            height: 1
            visible: card.title !== ""
            color: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.11)
        }

        ColumnLayout {
            id: contentHost
            Layout.fillWidth: true
            spacing: 4
        }
    }
}
