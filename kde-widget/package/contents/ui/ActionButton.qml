import QtQuick 2.15
import QtQuick.Controls 2.15 as QQC2
import org.kde.kirigami 2.20 as Kirigami

// Flat rounded button used for quick actions and the segmented mode picker.
// Built on AbstractButton so it is keyboard focusable and exposes a button
// (or radio, when `radio` is set) role to assistive tech.
QQC2.AbstractButton {
    id: btn
    property color fillColor: "transparent"     // resting fill
    property color activeColor: Kirigami.Theme.highlightColor
    property color textColor: Kirigami.Theme.textColor
    property color borderColor: "transparent"
    property bool solid: false                  // true: always painted with activeColor
    property bool radio: false
    property int radiusPx: 9

    implicitHeight: 44
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus
    Accessible.role: radio ? Accessible.RadioButton : Accessible.Button
    Accessible.name: text
    Accessible.checked: checked
    opacity: enabled ? 1.0 : 0.5

    background: Rectangle {
        radius: btn.radiusPx
        color: (btn.solid || btn.checked) ? (btn.down ? Qt.darker(btn.activeColor, 1.15) : btn.hovered ? Qt.lighter(btn.activeColor, 1.08) : btn.activeColor)
             : btn.down ? Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.14)
             : btn.hovered ? Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.09)
             : btn.fillColor
        border.width: btn.visualFocus ? 2 : (btn.borderColor.a > 0 ? 1 : 0)
        border.color: btn.visualFocus ? Kirigami.Theme.focusColor : btn.borderColor
        Behavior on color { ColorAnimation { duration: 140 } }
    }
    contentItem: Text {
        text: btn.text
        font.pixelSize: Kirigami.Theme.defaultFont.pixelSize
        font.weight: (btn.checked || btn.solid) ? Font.DemiBold : Font.Medium
        color: (btn.solid || btn.checked) ? "white" : btn.textColor
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
        elide: Text.ElideRight
    }
}
