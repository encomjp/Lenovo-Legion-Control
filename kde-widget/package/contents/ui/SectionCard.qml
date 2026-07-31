import QtQuick 2.15
import QtQuick.Layouts 1.15
import org.kde.kirigami 2.20 as Kirigami

Item {
    id: card

    default property alias content: contentHost.data
    implicitHeight: contentHost.implicitHeight + Kirigami.Units.largeSpacing * 2
    Layout.fillWidth: true

    Rectangle {
        anchors.fill: parent
        radius: Kirigami.Units.largeSpacing
        color: Qt.rgba(Kirigami.Theme.backgroundColor.r,
                       Kirigami.Theme.backgroundColor.g,
                       Kirigami.Theme.backgroundColor.b, 0.28)
        border.width: 1
        border.color: Qt.rgba(Kirigami.Theme.textColor.r,
                               Kirigami.Theme.textColor.g,
                               Kirigami.Theme.textColor.b, 0.10)
    }

    ColumnLayout {
        id: contentHost
        anchors.fill: parent
        anchors.margins: Kirigami.Units.smallSpacing
        spacing: 0
    }
}
