import QtQuick 2.15
import QtQuick.Layouts 1.15
import org.kde.kirigami 2.20 as Kirigami

// Elite hardware monitor card: chip identity + fan pill on top,
// big temperature + power badge in the middle, rich gradient graph below.
// CPU and GPU instances sit side by side at identical 142px height.
Item {
    id: perf
    property url iconSource
    property string chipName: "CPU"
    property string temp: "--"
    property string power: "--"
    property string fanText: "Auto"
    property var history: []
    property color accentColor: "#c8102e"
    property bool showSparkline: true
    property bool dimmed: false

    Layout.fillWidth: true
    Layout.preferredHeight: 142
    implicitHeight: 142

    readonly property color tempColor: {
        var t = parseFloat(perf.temp)
        if (perf.temp === "--" || isNaN(t) || t < 0) return Kirigami.Theme.disabledTextColor
        if (t >= 80) return "#f0524f"
        if (t >= 60) return "#f5a524"
        return "#34d399"
    }
    readonly property bool hasPower: perf.power !== "--" && perf.power !== "" && parseFloat(perf.power) >= 0
    readonly property string tempText: {
        var t = parseFloat(perf.temp)
        if (perf.temp === "--" || isNaN(t) || t < 0) return "—"
        return Math.round(t).toString()
    }

    Rectangle {
        anchors.fill: parent
        radius: 12
        clip: true
        color: Qt.rgba(Kirigami.Theme.backgroundColor.r, Kirigami.Theme.backgroundColor.g, Kirigami.Theme.backgroundColor.b, 0.32)
        border.width: 1
        border.color: Qt.rgba(1, 1, 1, 0.09)
        // Subtle top glass highlight.
        Rectangle {
            anchors.top: parent.top
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.topMargin: 1
            anchors.leftMargin: 8
            anchors.rightMargin: 8
            height: 1
            color: Qt.rgba(1, 1, 1, 0.07)
        }
        // Left edge accent stripe with soft glow.
        Rectangle {
            width: 3
            anchors.left: parent.left
            anchors.top: parent.top
            anchors.bottom: parent.bottom
            color: perf.accentColor
        }
        Rectangle {
            width: 10
            anchors.left: parent.left
            anchors.leftMargin: 3
            anchors.top: parent.top
            anchors.bottom: parent.bottom
            color: Qt.rgba(perf.accentColor.r, perf.accentColor.g, perf.accentColor.b, 0.10)
        }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.leftMargin: 13
        anchors.rightMargin: 10
        anchors.topMargin: 9
        anchors.bottomMargin: 8
        spacing: 3

        RowLayout {
            Layout.fillWidth: true
            spacing: 5
            Kirigami.Icon {
                source: perf.iconSource
                isMask: true
                color: Kirigami.Theme.textColor
                Layout.preferredWidth: 13
                Layout.preferredHeight: 13
                Layout.alignment: Qt.AlignVCenter
                opacity: perf.dimmed ? 0.30 : 0.75
            }
            Text {
                text: perf.chipName
                font.pixelSize: Kirigami.Theme.smallFont.pixelSize
                font.weight: Font.DemiBold
                Layout.fillWidth: true
                Layout.minimumWidth: 0
                Layout.alignment: Qt.AlignVCenter
                elide: Text.ElideRight
                color: Kirigami.Theme.textColor
                opacity: perf.dimmed ? 0.40 : 0.80
            }
            // Fan speed pill with mini fan icon.
            Rectangle {
                Layout.alignment: Qt.AlignVCenter
                Layout.preferredHeight: 18
                implicitWidth: fanRow.implicitWidth + 12
                radius: 9
                color: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.06)
                border.width: 1
                border.color: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.10)
                Row {
                    id: fanRow
                    anchors.centerIn: parent
                    spacing: 3
                    Kirigami.Icon {
                        source: Qt.resolvedUrl("icons/fan.svg")
                        isMask: true
                        color: Kirigami.Theme.textColor
                        width: 9
                        height: 9
                        anchors.verticalCenter: parent.verticalCenter
                        opacity: perf.dimmed ? 0.30 : 0.60
                    }
                    Text {
                        text: perf.fanText
                        font.pixelSize: Kirigami.Theme.smallFont.pixelSize - 1
                        font.weight: Font.Medium
                        anchors.verticalCenter: parent.verticalCenter
                        color: Kirigami.Theme.textColor
                        opacity: perf.dimmed ? 0.30 : 0.65
                    }
                }
            }
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: 4
            Text {
                text: perf.tempText
                font.pixelSize: 30
                font.weight: Font.Bold
                font.letterSpacing: -1.0
                Layout.alignment: Qt.AlignBottom
                color: perf.tempColor
                opacity: perf.dimmed ? 0.45 : 1.0
                Behavior on color { ColorAnimation { duration: 220 } }
            }
            Text {
                text: "°C"
                font.pixelSize: Kirigami.Theme.smallFont.pixelSize
                font.weight: Font.Medium
                Layout.alignment: Qt.AlignBottom
                Layout.bottomMargin: 6
                color: perf.tempColor
                opacity: perf.dimmed ? 0.35 : 0.70
            }
            Item { Layout.fillWidth: true }
            Rectangle {
                visible: perf.hasPower
                Layout.alignment: Qt.AlignVCenter
                Layout.preferredHeight: 20
                implicitWidth: powerLabel.implicitWidth + 14
                radius: 10
                color: Qt.rgba(perf.accentColor.r, perf.accentColor.g, perf.accentColor.b, 0.16)
                border.width: 1
                border.color: Qt.rgba(perf.accentColor.r, perf.accentColor.g, perf.accentColor.b, 0.38)
                Text {
                    id: powerLabel
                    anchors.centerIn: parent
                    text: perf.power + " W"
                    font.pixelSize: Kirigami.Theme.smallFont.pixelSize
                    font.weight: Font.Bold
                    color: perf.accentColor
                }
            }
        }

        Item {
            Layout.fillWidth: true
            Layout.preferredHeight: 36
            Layout.topMargin: 2
            // Subtle dotted baseline grid behind the curve.
            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                anchors.bottomMargin: 2
                height: 1
                color: Qt.rgba(1, 1, 1, 0.10)
                opacity: perf.dimmed ? 0.4 : 1.0
            }
            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                anchors.topMargin: 10
                height: 1
                color: Qt.rgba(1, 1, 1, 0.05)
                opacity: perf.dimmed ? 0.4 : 1.0
            }
            Sparkline {
                anchors.fill: parent
                visible: perf.showSparkline && perf.history.length > 1
                points: perf.history
                lineColor: perf.accentColor
                strokeWidth: 1.8
                opacity: perf.dimmed ? 0.35 : 1.0
            }
            // Reserve the graph's space even when hidden so both cards
            // always keep identical height.
            Item {
                anchors.fill: parent
                visible: !(perf.showSparkline && perf.history.length > 1)
            }
        }
    }
}
