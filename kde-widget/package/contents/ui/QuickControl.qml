import QtQuick 2.15
import QtQuick.Layouts 1.15
import org.kde.kirigami 2.20 as Kirigami

Item {
    id: qc
    property string iconSource: "icons/profile.svg"
    property string label: "Profile"
    property string valueText: "--"
    property color valueColor: Kirigami.Theme.textColor
    signal clicked()

    implicitHeight: 34
    Layout.fillWidth: true

    Rectangle {
        anchors.fill: parent
        radius: 9
        color: ma.pressed ? Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.10)
              : ma.containsMouse ? Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.06)
              : "transparent"
        border.width: ma.containsMouse || ma.pressed ? 1 : 0
        border.color: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.10)
        Behavior on color { ColorAnimation { duration: 140 } }
    }

    MouseArea {
        id: ma
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: qc.clicked()
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: Kirigami.Units.smallSpacing + 4
        anchors.rightMargin: Kirigami.Units.smallSpacing + 4
        spacing: Kirigami.Units.smallSpacing + 2

        Kirigami.Icon {
            source: Qt.resolvedUrl(qc.iconSource)
            isMask: true
            color: Kirigami.Theme.textColor
            implicitWidth: 13
            implicitHeight: 13
            Layout.alignment: Qt.AlignVCenter
            opacity: 0.60
        }

        Text {
            text: qc.label
            font.pixelSize: Kirigami.Theme.defaultFont.pixelSize
            font.weight: Font.Medium
            Layout.alignment: Qt.AlignVCenter
            color: Kirigami.Theme.textColor
            opacity: 0.92
        }

        Item { Layout.fillWidth: true }

        Text {
            text: qc.valueText
            font.pixelSize: Kirigami.Theme.defaultFont.pixelSize
            font.weight: Font.DemiBold
            Layout.alignment: Qt.AlignVCenter
            color: qc.valueColor
            opacity: 0.95
        }

        Text {
            text: "›"
            font.pixelSize: 13
            font.weight: Font.DemiBold
            Layout.alignment: Qt.AlignVCenter
            color: ma.containsMouse ? "#e8566e" : Kirigami.Theme.textColor
            opacity: ma.containsMouse ? 0.95 : 0.35
            Behavior on color { ColorAnimation { duration: 140 } }
        }
    }
}
