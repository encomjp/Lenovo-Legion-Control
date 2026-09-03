import QtQuick 2.15
import QtQuick.Shapes 1.15

// Dual-stream telemetry history: CPU + GPU curves overlaid in one graph,
// like the desktop app. Shared normalisation so relative levels are honest,
// per-stream gradient fills, subtle dotted gridlines.
Item {
    id: hist
    property var cpuPoints: []
    property var gpuPoints: []
    property color cpuColor: "#f0524f"
    property color gpuColor: "#38bdf8"
    property int maxPoints: 30
    property real strokeWidth: 1.7

    implicitHeight: 64
    implicitWidth: 120

    onCpuPointsChanged: updatePaths()
    onGpuPointsChanged: updatePaths()
    onWidthChanged: updatePaths()
    onHeightChanged: updatePaths()
    onCpuColorChanged: updatePaths()
    onGpuColorChanged: updatePaths()

    // Subtle gridlines behind the curves, evenly distributed.
    Rectangle {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.topMargin: 14
        height: 1
        color: Qt.rgba(1, 1, 1, 0.06)
    }
    Rectangle {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.verticalCenter: parent.verticalCenter
        height: 1
        color: Qt.rgba(1, 1, 1, 0.06)
    }
    Rectangle {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        anchors.bottomMargin: 2
        height: 1
        color: Qt.rgba(1, 1, 1, 0.12)
    }

    Shape {
        id: canvas
        anchors.fill: parent
        antialiasing: true
        // CPU fill (under).
        ShapePath {
            id: cpuFill
            strokeColor: "transparent"
            fillColor: "transparent"
            fillGradient: LinearGradient {
                x1: 0
                y1: 0
                x2: 0
                y2: hist.height
                GradientStop { position: 0.0; color: Qt.rgba(hist.cpuColor.r, hist.cpuColor.g, hist.cpuColor.b, 0.30) }
                GradientStop { position: 1.0; color: Qt.rgba(hist.cpuColor.r, hist.cpuColor.g, hist.cpuColor.b, 0.04) }
            }
            PathPolyline { id: cpuFillPoly }
        }
        // GPU fill (under).
        ShapePath {
            id: gpuFill
            strokeColor: "transparent"
            fillColor: "transparent"
            fillGradient: LinearGradient {
                x1: 0
                y1: 0
                x2: 0
                y2: hist.height
                GradientStop { position: 0.0; color: Qt.rgba(hist.gpuColor.r, hist.gpuColor.g, hist.gpuColor.b, 0.30) }
                GradientStop { position: 1.0; color: Qt.rgba(hist.gpuColor.r, hist.gpuColor.g, hist.gpuColor.b, 0.04) }
            }
            PathPolyline { id: gpuFillPoly }
        }
        ShapePath {
            id: cpuLine
            strokeColor: hist.cpuColor
            strokeWidth: hist.strokeWidth
            fillColor: "transparent"
            capStyle: ShapePath.RoundCap
            joinStyle: ShapePath.RoundJoin
            PathPolyline { id: cpuLinePoly }
        }
        ShapePath {
            id: gpuLine
            strokeColor: hist.gpuColor
            strokeWidth: hist.strokeWidth
            fillColor: "transparent"
            capStyle: ShapePath.RoundCap
            joinStyle: ShapePath.RoundJoin
            PathPolyline { id: gpuLinePoly }
        }
    }

    function smooth(data) {
        var n = data.length
        if (n <= 4) return data.slice()
        var sm = data.slice()
        for (var s = 1; s < n - 1; s++)
            sm[s] = (data[s - 1] + 2 * data[s] + data[s + 1]) / 4
        return sm
    }

    function mapToPoints(data, lo, span, w, h) {
        var n = data.length
        var pts = []
        var pad = 3
        for (var i = 0; i < n; i++) {
            var x = w - ((n - 1 - i) / (hist.maxPoints - 1)) * w
            var norm = span < 0.001 ? 0.5 : (data[i] - lo) / span
            var y = pad + (h - 2 * pad) * (1 - norm)
            pts.push(Qt.point(x, y))
        }
        return pts
    }

    function updatePaths() {
        if (hist.width <= 0 || hist.height <= 0) return
        var hasCpu = hist.cpuPoints.length >= 2
        var hasGpu = hist.gpuPoints.length >= 2
        if (!hasCpu && !hasGpu) {
            cpuLinePoly.path = []
            cpuFillPoly.path = []
            gpuLinePoly.path = []
            gpuFillPoly.path = []
            return
        }
        var all = []
        var cpu = [], gpu = []
        if (hasCpu) {
            cpu = smooth(hist.cpuPoints.slice())
            all = all.concat(cpu)
        }
        if (hasGpu) {
            gpu = smooth(hist.gpuPoints.slice())
            all = all.concat(gpu)
        }
        var lo = Math.min.apply(null, all)
        var hi = Math.max.apply(null, all)
        var span = hi - lo
        var w = hist.width
        var h = hist.height
        if (hasCpu) {
            var cp = mapToPoints(cpu, lo, span, w, h)
            cpuLinePoly.path = cp
            cpuFillPoly.path = [Qt.point(cp[0].x, h)].concat(cp).concat([Qt.point(cp[cp.length - 1].x, h)])
        } else {
            cpuLinePoly.path = []
            cpuFillPoly.path = []
        }
        if (hasGpu) {
            var gp = mapToPoints(gpu, lo, span, w, h)
            gpuLinePoly.path = gp
            gpuFillPoly.path = [Qt.point(gp[0].x, h)].concat(gp).concat([Qt.point(gp[gp.length - 1].x, h)])
        } else {
            gpuLinePoly.path = []
            gpuFillPoly.path = []
        }
    }
}
