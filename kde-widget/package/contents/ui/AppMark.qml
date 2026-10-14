import QtQuick 2.15

// Legion Control brand mark drawn in pure QML. Replaces the SVG <Image>,
// which rendered as an empty square when the SVG image plugin was missing
// or the file URL did not resolve. Nothing here touches the filesystem.
Item {
    id: mark
    property color tileColor: "#C8102E"
    property color glyphColor: "#FFFFFF"
    implicitWidth: 22
    implicitHeight: 22

    Rectangle {
        anchors.fill: parent
        radius: Math.round(mark.width * 0.28)
        color: mark.tileColor
    }
    // "L" glyph: vertical stem + foot.
    Rectangle {
        x: mark.width * 0.30
        y: mark.height * 0.22
        width: Math.max(2, mark.width * 0.13)
        height: mark.height * 0.56
        radius: width / 2
        color: mark.glyphColor
    }
    Rectangle {
        x: mark.width * 0.30
        y: mark.height * 0.78 - Math.max(2, mark.height * 0.13)
        width: mark.width * 0.42
        height: Math.max(2, mark.height * 0.13)
        radius: height / 2
        color: mark.glyphColor
    }
}
