import QtQuick 2.15
import QtQuick.Shapes 1.15
import org.kde.kirigami 2.20 as Kirigami

Item {
    id: gauge
    property real value: 0
    property real minValue: 0
    property real maxValue: 100
    property string unit: "°C"
    property string label: ""
    property int size: 64

    implicitWidth: size
    implicitHeight: size

    readonly property color arcColor: {
        if (value < 0) return Kirigami.Theme.disabledTextColor
        if (value >= 90) return "#ff4444"
        if (value >= 80) return "#ff8800"
        if (value >= 70) return "#ffcc00"
        return "#44d62c"
    }

    readonly property real normalized: {
        if (value < 0) return 0
        return Math.max(0, Math.min(1, (value - minValue) / (maxValue - minValue)))
    }

    // Background track
    Shape {
        anchors.fill: parent
        antialiasing: true
        ShapePath {
            strokeColor: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.1)
            strokeWidth: 5
            fillColor: "transparent"
            capStyle: ShapePath.RoundCap
            PathAngleArc {
                centerX: gauge.width / 2
                centerY: gauge.height / 2
                radiusX: gauge.size / 2 - 6
                radiusY: gauge.size / 2 - 6
                startAngle: 135
                sweepAngle: 270
            }
        }
    }

    // Value arc (animated)
    Shape {
        anchors.fill: parent
        antialiasing: true
        ShapePath {
            strokeColor: gauge.arcColor
            strokeWidth: 5
            fillColor: "transparent"
            capStyle: ShapePath.RoundCap
            PathAngleArc {
                centerX: gauge.width / 2
                centerY: gauge.height / 2
                radiusX: gauge.size / 2 - 6
                radiusY: gauge.size / 2 - 6
                startAngle: 135
                sweepAngle: gauge.normalized * 270
                Behavior on sweepAngle {
                    NumberAnimation { duration: 600; easing.type: Easing.OutCubic }
                }
            }
        }
    }

    // Center text
    Column {
        anchors.centerIn: parent
        spacing: 0
        Text {
            anchors.horizontalCenter: parent.horizontalCenter
            text: value < 0 ? "--" : Math.round(value).toString()
            font.pixelSize: gauge.size * 0.28
            font.bold: true
            color: gauge.arcColor
            Behavior on color { ColorAnimation { duration: 400 } }
        }
        Text {
            anchors.horizontalCenter: parent.horizontalCenter
            text: unit
            font.pixelSize: gauge.size * 0.12
            color: Kirigami.Theme.disabledTextColor
        }
    }
}
