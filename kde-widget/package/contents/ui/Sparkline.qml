import QtQuick 2.15
import QtQuick.Shapes 1.15

// Compact live-history sparkline: smooth antialiased curve with a rich
// vertical gradient fill (accent at 35% fading to 5%) and light display
// smoothing so single-sample spikes don't look jagged at 36px heights.
Item {
    id: spark
    property var points: []
    property color lineColor: "#e5484d"
    property int maxPoints: 30
    property real strokeWidth: 1.8
    property bool showFill: true

    implicitHeight: 36
    implicitWidth: 80

    onPointsChanged: updatePath()
    onWidthChanged: updatePath()
    onHeightChanged: updatePath()
    onLineColorChanged: updatePath()

    Shape {
        id: canvas
        anchors.fill: parent
        antialiasing: true
        ShapePath {
            id: fillPath
            strokeColor: "transparent"
            fillColor: "transparent"
            fillGradient: LinearGradient {
                x1: 0
                y1: 0
                x2: 0
                y2: spark.height
                GradientStop { position: 0.0; color: Qt.rgba(spark.lineColor.r, spark.lineColor.g, spark.lineColor.b, spark.showFill ? 0.35 : 0.0) }
                GradientStop { position: 1.0; color: Qt.rgba(spark.lineColor.r, spark.lineColor.g, spark.lineColor.b, spark.showFill ? 0.05 : 0.0) }
            }
            PathPolyline { id: fillPoly }
        }
        ShapePath {
            strokeColor: spark.lineColor
            strokeWidth: spark.strokeWidth
            fillColor: "transparent"
            capStyle: ShapePath.RoundCap
            joinStyle: ShapePath.RoundJoin
            PathPolyline { id: linePoly }
        }
    }

    function updatePath() {
        if (spark.width <= 0 || spark.height <= 0) return
        var n = points.length
        if (n < 2) {
            linePoly.path = []
            fillPoly.path = []
            return
        }
        // Light smoothing: interior samples averaged with their neighbours.
        var data = points.slice()
        if (n > 4) {
            var sm = data.slice()
            for (var s = 1; s < n - 1; s++)
                sm[s] = (data[s - 1] + 2 * data[s] + data[s + 1]) / 4
            data = sm
        }
        var min = Math.min.apply(null, data)
        var max = Math.max.apply(null, data)
        var span = max - min
        var w = spark.width
        var h = spark.height
        var pad = 2
        var pts = []
        for (var i = 0; i < n; i++) {
            // Right-align: newest sample always sits at the right edge.
            var x = w - ((n - 1 - i) / (maxPoints - 1)) * w
            var norm = span < 0.001 ? 0.5 : (data[i] - min) / span
            var y = pad + (h - 2 * pad) * (1 - norm)
            pts.push(Qt.point(x, y))
        }
        linePoly.path = pts
        fillPoly.path = [Qt.point(pts[0].x, h)].concat(pts).concat([Qt.point(pts[pts.length - 1].x, h)])
    }
}
