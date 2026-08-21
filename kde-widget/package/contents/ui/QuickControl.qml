import QtQuick 2.15
import QtQuick.Layouts 1.15
import org.kde.kirigami 2.20 as Kirigami

Item {
    id: qc
    property url iconSource
    property string label: "Profile"
    property string valueText: "--"
    property color valueColor: Kirigami.Theme.textColor
    signal clicked()

    implicitHeight: 42
    implicitWidth: 0
    Layout.fillWidth: true
    Layout.minimumWidth: 0

    Rectangle {
        anchors.fill: parent
        radius: 8
        color: ma.pressed ? Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.10)
              : ma.containsMouse ? Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.06)
              : Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.025)
        border.width: 1
        border.color: ma.containsMouse || ma.pressed
            ? Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.14)
            : Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.05)
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
        anchors.leftMargin: 10
        anchors.rightMargin: 8
        spacing: 6

        Item {
            Layout.preferredWidth: 16
            Layout.minimumWidth: 16
            Layout.maximumWidth: 16
            Layout.preferredHeight: 16
            Layout.alignment: Qt.AlignVCenter
            Kirigami.Icon {
                anchors.centerIn: parent
                source: qc.iconSource
                isMask: true
                color: Kirigami.Theme.textColor
                width: 15
                height: 15
                opacity: 0.70
            }
        }

        Text {
            text: qc.label
            font.pixelSize: Kirigami.Theme.defaultFont.pixelSize
            font.weight: Font.Medium
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            Layout.alignment: Qt.AlignVCenter
            elide: Text.ElideRight
            color: Kirigami.Theme.textColor
            opacity: 0.92
        }

        Text {
            text: qc.valueText
            font.pixelSize: Kirigami.Theme.defaultFont.pixelSize
            font.weight: Font.DemiBold
            Layout.alignment: Qt.AlignVCenter
            Layout.maximumWidth: 88
            horizontalAlignment: Text.AlignRight
            elide: Text.ElideRight
            color: qc.valueColor
            opacity: 0.95
        }

        Text {
            text: "›"
            font.pixelSize: 16
            font.weight: Font.DemiBold
            Layout.preferredWidth: 10
            Layout.alignment: Qt.AlignVCenter
            horizontalAlignment: Text.AlignHCenter
            color: ma.containsMouse ? "#e8566e" : Kirigami.Theme.textColor
            opacity: ma.containsMouse ? 0.95 : 0.40
            Behavior on color { ColorAnimation { duration: 140 } }
        }
    }
}
