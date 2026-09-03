import QtQuick 2.15
import QtQuick.Layouts 1.15
import org.kde.kirigami 2.20 as Kirigami

// Polished command-deck tile: rounded dark glass, top highlight,
// clean icon + caps title, prominent readout with chevron affordance.
Item {
    id: qc
    property url iconSource
    property string label: "Profile"
    property string valueText: "--"
    property color valueColor: Kirigami.Theme.textColor
    signal clicked()

    implicitHeight: 62
    implicitWidth: 0
    Layout.fillWidth: true
    Layout.minimumWidth: 0
    Layout.preferredHeight: 62

    scale: ma.pressed ? 0.97 : 1.0
    Behavior on scale { NumberAnimation { duration: 120; easing.type: Easing.OutQuad } }

    Rectangle {
        anchors.fill: parent
        radius: 10
        color: ma.pressed ? Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.10)
              : ma.containsMouse ? Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.06)
              : Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.03)
        border.width: 1
        border.color: ma.containsMouse || ma.pressed
            ? Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.16)
            : Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.07)
        Behavior on color { ColorAnimation { duration: 140 } }
        Behavior on border.color { ColorAnimation { duration: 140 } }
        // Subtle top highlight for glass depth.
        Rectangle {
            anchors.top: parent.top
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.topMargin: 1
            anchors.leftMargin: 8
            anchors.rightMargin: 8
            height: 1
            color: Qt.rgba(1, 1, 1, 0.08)
        }
    }

    MouseArea {
        id: ma
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: qc.clicked()
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.leftMargin: 6
        anchors.rightMargin: 6
        anchors.topMargin: 7
        anchors.bottomMargin: 7
        spacing: 3

        RowLayout {
            Layout.fillWidth: true
            Layout.alignment: Qt.AlignHCenter
            spacing: 4
            Kirigami.Icon {
                source: qc.iconSource
                isMask: true
                color: Kirigami.Theme.textColor
                Layout.preferredWidth: 12
                Layout.preferredHeight: 12
                Layout.alignment: Qt.AlignVCenter
                opacity: 0.65
            }
            Text {
                text: qc.label.toUpperCase()
                font.pixelSize: Kirigami.Theme.smallFont.pixelSize - 2
                font.weight: Font.Bold
                font.letterSpacing: 0.8
                Layout.alignment: Qt.AlignVCenter
                elide: Text.ElideRight
                color: Kirigami.Theme.textColor
                opacity: 0.62
            }
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: 2
            Item { Layout.fillWidth: true }
            Text {
                text: qc.valueText
                font.pixelSize: Kirigami.Theme.defaultFont.pixelSize + 1
                font.weight: Font.Bold
                font.letterSpacing: -0.2
                Layout.alignment: Qt.AlignVCenter
                elide: Text.ElideRight
                color: qc.valueColor
                opacity: 0.96
            }
            Kirigami.Icon {
                source: Qt.resolvedUrl("icons/chevron.svg")
                isMask: true
                color: Kirigami.Theme.textColor
                Layout.preferredWidth: 8
                Layout.preferredHeight: 8
                Layout.alignment: Qt.AlignVCenter
                opacity: ma.containsMouse ? 0.65 : 0.32
                Behavior on opacity { NumberAnimation { duration: 140 } }
            }
            Item { Layout.fillWidth: true }
        }
    }
}
