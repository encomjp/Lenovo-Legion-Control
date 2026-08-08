import QtQuick 2.15
import QtQuick.Layouts 1.15
import org.kde.kirigami 2.20 as Kirigami

Item {
    id: card
    property string title: ""
    property string badge: ""
    property color badgeColor: Kirigami.Theme.textColor
    default property alias content: contentHost.data

    implicitHeight: headerRow.implicitHeight + contentHost.implicitHeight
                + Kirigami.Units.largeSpacing + Kirigami.Units.smallSpacing * 2
    Layout.fillWidth: true

    Rectangle {
        anchors.fill: parent
        radius: 14
        color: Qt.rgba(Kirigami.Theme.backgroundColor.r, Kirigami.Theme.backgroundColor.g, Kirigami.Theme.backgroundColor.b, 0.36)
        border.width: 1
        border.color: Qt.rgba(1, 1, 1, 0.10)
    }

    // inner glass highlight
    Rectangle {
        anchors.fill: parent
        anchors.margins: 1
        radius: 13
        color: "transparent"
        border.width: 1
        border.color: Qt.rgba(1, 1, 1, 0.05)
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.leftMargin: Kirigami.Units.largeSpacing
        anchors.rightMargin: Kirigami.Units.largeSpacing
        anchors.topMargin: Kirigami.Units.smallSpacing + 4
        anchors.bottomMargin: Kirigami.Units.smallSpacing + 4
        spacing: Kirigami.Units.smallSpacing

        RowLayout {
            id: headerRow
            Layout.fillWidth: true
            spacing: Kirigami.Units.smallSpacing
            visible: card.title !== ""

            Text {
                text: card.title
                font.pixelSize: Kirigami.Theme.smallFont.pixelSize
                font.weight: Font.Bold
                font.letterSpacing: 1.2
                color: Kirigami.Theme.textColor
                opacity: 0.62
                Layout.alignment: Qt.AlignVCenter
            }
            Item { Layout.fillWidth: true }
            Text {
                visible: card.badge !== ""
                text: card.badge
                font.pixelSize: Kirigami.Theme.smallFont.pixelSize - 2
                font.weight: Font.DemiBold
                font.letterSpacing: 0.6
                color: card.badgeColor
                Layout.alignment: Qt.AlignVCenter
            }
        }

        // hairline rule under the header — matches app section-label rule
        Rectangle {
            Layout.fillWidth: true
            height: 1
            visible: card.title !== ""
            color: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.08)
        }

        ColumnLayout {
            id: contentHost
            Layout.fillWidth: true
            spacing: 2
        }
    }
}
