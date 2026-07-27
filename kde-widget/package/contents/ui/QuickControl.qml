import QtQuick 2.15
import QtQuick.Controls 2.15 as QQC2
import QtQuick.Layouts 1.15
import org.kde.kirigami 2.20 as Kirigami

Item {
    id: qc
    property string iconSource: "system-run"
    property string label: "Profile"
    property string valueText: "--"
    property color valueColor: Kirigami.Theme.positiveTextColor
    property bool on: true

    signal clicked()

    implicitHeight: 44
    Layout.fillWidth: true

    MouseArea {
        id: ma
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: qc.clicked()

        Rectangle {
            anchors.fill: parent
            radius: 6
            color: ma.containsMouse
                ? Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 0.15)
                : "transparent"
            Behavior on color { ColorAnimation { duration: 150 } }
        }

        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: Kirigami.Units.largeSpacing
            anchors.rightMargin: Kirigami.Units.largeSpacing

            Kirigami.Icon {
                source: qc.iconSource
                Layout.preferredWidth: 18
                Layout.preferredHeight: 18
                opacity: qc.on ? 0.9 : 0.4
            }

            QQC2.Label {
                text: qc.label
                font.pixelSize: Kirigami.Theme.smallFont.pixelSize
                font.weight: Font.DemiBold
            }

            Item { Layout.fillWidth: true }

            QQC2.Label {
                text: qc.valueText
                font.pixelSize: Kirigami.Theme.smallFont.pixelSize
                font.bold: true
                color: qc.valueColor
            }

            Kirigami.Icon {
                source: "go-next-symbolic"
                Layout.preferredWidth: 12
                Layout.preferredHeight: 12
                opacity: 0.3
            }
        }
    }
}
