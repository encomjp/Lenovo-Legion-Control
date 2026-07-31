import QtQuick 2.15
import QtQuick.Shapes 1.15

Item {
    id: spark
    property var points: []
    property color lineColor: "#44d62c"
    property int maxPoints: 30

    implicitHeight: 24
    implicitWidth: 80

    onPointsChanged: updatePath()

    function push(val) {
        points.push(val)
        if (points.length > maxPoints) points.shift()
        updatePath()
    }

    function clear() {
        points = []
        updatePath()
    }

    Shape {
        id: canvas
        anchors.fill: parent
        antialiasing: true
        ShapePath {
            strokeColor: spark.lineColor
            strokeWidth: 1.5
            fillColor: "transparent"
            PathPolyline { id: polyline }
        }
    }

    function updatePath() {
        if (points.length < 2) return
        var min = Math.min(...points)
        var max = Math.max(...points)
        if (max === min) max = min + 1
        var w = spark.width
        var h = spark.height
        var pts = []
        for (var i = 0; i < points.length; i++) {
            var x = (i / (maxPoints - 1)) * w
            var y = h - ((points[i] - min) / (max - min)) * h
            pts.push(Qt.point(x, y))
        }
        polyline.path = pts
    }
}
