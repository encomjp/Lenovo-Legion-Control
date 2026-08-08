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
    property int size: 88

    implicitWidth: size
    implicitHeight: size

    readonly property color arcColor: {
        if (value < 0 || isNaN(value)) return Kirigami.Theme.disabledTextColor
        if (value >= 90) return "#e8566e"
        if (value >= 80) return "#d9981a"
        return Kirigami.Theme.textColor
    }
    readonly property real normalized: {
        if (isNaN(value) || value < 0) return 0
        return Math.max(0, Math.min(1, (value - minValue) / (maxValue - minValue)))
    }

    Shape {
        anchors.fill: parent
        antialiasing: true
        ShapePath {
            strokeColor: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.10)
            strokeWidth: 5
            fillColor: "transparent"
            capStyle: ShapePath.RoundCap
            PathAngleArc {
                centerX: gauge.width / 2; centerY: gauge.height / 2
                radiusX: gauge.size / 2 - 6; radiusY: gauge.size / 2 - 6
                startAngle: 135; sweepAngle: 270
            }
        }
    }
    Shape {
        anchors.fill: parent
        antialiasing: true
        ShapePath {
            strokeColor: gauge.arcColor
            strokeWidth: 5
            fillColor: "transparent"
            capStyle: ShapePath.RoundCap
            PathAngleArc {
                centerX: gauge.width / 2; centerY: gauge.height / 2
                radiusX: gauge.size / 2 - 6; radiusY: gauge.size / 2 - 6
                startAngle: 135; sweepAngle: Math.max(0.5, gauge.normalized * 270)
                Behavior on sweepAngle { NumberAnimation { duration: 520; easing.type: Easing.OutCubic } }
            }
        }
    }
    // hot threshold tick
    Shape {
        anchors.fill: parent
        antialiasing: true
        visible: gauge.value >= 0
        ShapePath {
            strokeColor: Qt.rgba(232/255, 86/255, 110/255, 0.55)
            strokeWidth: 1.5
            fillColor: "transparent"
            PathAngleArc {
                centerX: gauge.width/2; centerY: gauge.height/2
                radiusX: gauge.size/2 - 6; radiusY: gauge.size/2 - 6
                startAngle: 135 + 270*0.88; sweepAngle: 2.5
            }
        }
    }

    Column {
        anchors.centerIn: parent
        spacing: 0
        Text {
            anchors.horizontalCenter: parent.horizontalCenter
            text: value < 0 || isNaN(value) ? "—" : Math.round(value).toString()
            font.pixelSize: gauge.size * 0.32
            font.weight: Font.DemiBold
            font.letterSpacing: -1.0
            color: gauge.arcColor
            Behavior on color { ColorAnimation { duration: 320 } }
        }
        Text {
            anchors.horizontalCenter: parent.horizontalCenter
            text: gauge.label.toUpperCase()
            font.pixelSize: gauge.size * 0.095
            font.letterSpacing: 1.2
            font.weight: Font.Medium
            color: Kirigami.Theme.textColor
            opacity: 0.50
            visible: gauge.label !== ""
        }
    }
}
