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
        preferredRendererType: Shape.CurveRenderer
        ShapePath {
            strokeColor: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.12)
            strokeWidth: 4.5
            fillColor: "transparent"
            capStyle: ShapePath.RoundCap
            PathAngleArc {
                centerX: gauge.width / 2; centerY: gauge.height / 2
                radiusX: gauge.size / 2 - 7; radiusY: gauge.size / 2 - 7
                startAngle: 135; sweepAngle: 270
            }
        }
    }
    Shape {
        anchors.fill: parent
        antialiasing: true
        preferredRendererType: Shape.CurveRenderer
        ShapePath {
            strokeColor: gauge.arcColor
            strokeWidth: 4.5
            fillColor: "transparent"
            capStyle: ShapePath.RoundCap
            PathAngleArc {
                centerX: gauge.width / 2; centerY: gauge.height / 2
                radiusX: gauge.size / 2 - 7; radiusY: gauge.size / 2 - 7
                startAngle: 135; sweepAngle: Math.max(0.5, gauge.normalized * 270)
                Behavior on sweepAngle { NumberAnimation { duration: 520; easing.type: Easing.OutCubic } }
            }
        }
    }

    Column {
        anchors.centerIn: parent
        spacing: 1
        width: gauge.size - 16

        Row {
            anchors.horizontalCenter: parent.horizontalCenter
            spacing: 1
            Text {
                text: value < 0 || isNaN(value) ? "—" : Math.round(value).toString()
                font.pixelSize: gauge.size * 0.30
                font.weight: Font.DemiBold
                font.letterSpacing: -0.8
                color: gauge.arcColor
                Behavior on color { ColorAnimation { duration: 320 } }
            }
            Text {
                text: gauge.unit
                anchors.baseline: parent.children[0].baseline
                font.pixelSize: gauge.size * 0.13
                font.weight: Font.Medium
                color: gauge.arcColor
                opacity: 0.85
            }
        }
        Text {
            anchors.horizontalCenter: parent.horizontalCenter
            width: parent.width
            text: gauge.label.toUpperCase()
            font.pixelSize: Math.max(9, gauge.size * 0.11)
            font.letterSpacing: 1.0
            font.weight: Font.Medium
            horizontalAlignment: Text.AlignHCenter
            elide: Text.ElideRight
            color: Kirigami.Theme.textColor
            opacity: 0.55
            visible: gauge.label !== ""
        }
    }
}
