import QtQuick 2.15
import QtQuick.Controls 2.15 as QQC2
import QtQuick.Layouts 1.15
import org.kde.kirigami 2.20 as Kirigami

Item {
    id: card
    property string iconSource: "cpu-symbolic"
    property string label: "CPU"
    property string value: "--"
    property string unit: "°C"
    property string subValue: ""
    property string subUnit: "W"
    property color valueColor: Kirigami.Theme.textColor
    property bool showSparkline: false
    property var sparkPoints: []
    property color sparkColor: "#44d62c"

    implicitHeight: 52
    Layout.fillWidth: true

    RowLayout {
        anchors.fill: parent
        spacing: Kirigami.Units.smallSpacing

        Kirigami.Icon {
            source: card.iconSource
            Layout.preferredWidth: 20
            Layout.preferredHeight: 20
            opacity: 0.8
        }

        QQC2.Label {
            text: card.label
            Layout.fillWidth: true
            font.pixelSize: Kirigami.Theme.smallFont.pixelSize
            font.weight: Font.DemiBold
            elide: Text.ElideRight
        }

        Sparkline {
            visible: card.showSparkline
            Layout.preferredWidth: 60
            Layout.preferredHeight: 20
            points: card.sparkPoints
            lineColor: card.sparkColor
        }

        QQC2.Label {
            text: card.value === "--" ? "--" : card.value + card.unit
            font.bold: true
            font.pixelSize: Kirigami.Theme.defaultFont.pixelSize
            Layout.preferredWidth: 56
            horizontalAlignment: Text.AlignRight
            color: card.valueColor
            Behavior on color { ColorAnimation { duration: 400 } }
        }

        QQC2.Label {
            text: card.subValue ? card.subValue + " " + card.subUnit : ""
            font.pixelSize: Kirigami.Theme.smallFont.pixelSize
            Layout.preferredWidth: 44
            horizontalAlignment: Text.AlignRight
            opacity: 0.6
        }
    }
}
