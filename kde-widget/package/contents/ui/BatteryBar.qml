import QtQuick 2.15
import QtQuick.Layouts 1.15
import org.kde.kirigami 2.20 as Kirigami

// Premium energy capsule: frosted glass card holding a
// icon + large % + status pill + limit tag row above a
// rounded gradient energy bar with glass gloss and segment ticks.
Item {
    id: bat
    property string percentage: "--"
    property string batteryStatus: "Unknown"
    property string watts: "--"
    property string chargeLimit: ""

    Layout.fillWidth: true
    Layout.preferredHeight: 68
    implicitHeight: 68

    readonly property int pct: Math.max(0, Math.min(100, parseInt(percentage) || 0))
    readonly property color fillTop: {
        if (batteryStatus === "Charging") return "#34d399"
        if (pct <= 15) return "#f87171"
        if (pct <= 30) return "#fbbf24"
        return "#10b981"
    }
    readonly property color fillBottom: {
        if (batteryStatus === "Charging") return "#059669"
        if (pct <= 15) return "#dc2626"
        if (pct <= 30) return "#d97706"
        return "#059669"
    }
    readonly property string stateText: {
        if (batteryStatus === "Charging") return "CHARGING"
        if (batteryStatus === "Not charging") return "LIMIT HOLD"
        if (batteryStatus === "Full") return "FULL"
        return "DISCHARGING"
    }
    readonly property color stateColor: {
        if (batteryStatus === "Charging") return "#34d399"
        if (batteryStatus === "Full") return "#10b981"
        if (batteryStatus === "Not charging") return "#f5a524"
        return Kirigami.Theme.textColor
    }
    readonly property bool showWatts: watts !== "--" && watts !== "0.0"
        && (batteryStatus === "Charging" || batteryStatus === "Discharging")
    readonly property string limitText: chargeLimit !== "" ? "Limit " + chargeLimit + "%" : "Standard 100%"
    readonly property color limitColor: chargeLimit !== "" ? "#f5a524" : Kirigami.Theme.textColor

    Rectangle {
        anchors.fill: parent
        radius: 12
        color: Qt.rgba(Kirigami.Theme.backgroundColor.r, Kirigami.Theme.backgroundColor.g, Kirigami.Theme.backgroundColor.b, 0.32)
        border.width: 1
        border.color: Qt.rgba(1, 1, 1, 0.08)
        // Top glass highlight.
        Rectangle {
            anchors.top: parent.top
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.topMargin: 1
            anchors.leftMargin: 10
            anchors.rightMargin: 10
            height: 1
            color: Qt.rgba(1, 1, 1, 0.07)
        }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.leftMargin: 13
        anchors.rightMargin: 13
        anchors.topMargin: 9
        anchors.bottomMargin: 10
        spacing: 7

        RowLayout {
            Layout.fillWidth: true
            spacing: 7

            Kirigami.Icon {
                source: Qt.resolvedUrl("icons/battery.svg")
                isMask: true
                color: bat.fillTop
                Layout.preferredWidth: 16
                Layout.preferredHeight: 16
                Layout.alignment: Qt.AlignVCenter
                opacity: 0.95
                Behavior on color { ColorAnimation { duration: 300 } }
            }
            Text {
                text: bat.percentage + "%"
                font.pixelSize: Kirigami.Theme.defaultFont.pixelSize + 2
                font.weight: Font.Bold
                font.letterSpacing: -0.3
                Layout.alignment: Qt.AlignVCenter
                color: Kirigami.Theme.textColor
            }
            // Sleek status pill with soft glow border.
            Rectangle {
                Layout.alignment: Qt.AlignVCenter
                Layout.preferredHeight: 18
                implicitWidth: stateLabel.implicitWidth + 14
                radius: 9
                color: Qt.rgba(bat.stateColor.r, bat.stateColor.g, bat.stateColor.b, 0.16)
                border.width: 1
                border.color: Qt.rgba(bat.stateColor.r, bat.stateColor.g, bat.stateColor.b, 0.45)
                Text {
                    id: stateLabel
                    anchors.centerIn: parent
                    text: bat.stateText
                    font.pixelSize: Kirigami.Theme.smallFont.pixelSize - 2
                    font.weight: Font.Bold
                    font.letterSpacing: 0.8
                    color: bat.stateColor
                }
            }
            Item { Layout.fillWidth: true }
            Text {
                visible: bat.showWatts
                text: (bat.batteryStatus === "Charging" ? "+" : "−") + bat.watts + " W"
                font.pixelSize: Kirigami.Theme.smallFont.pixelSize
                font.weight: Font.Bold
                Layout.alignment: Qt.AlignVCenter
                color: bat.batteryStatus === "Charging" ? "#34d399" : Kirigami.Theme.textColor
                opacity: 0.90
            }
            // Conservation limit tag on the right.
            Rectangle {
                Layout.alignment: Qt.AlignVCenter
                Layout.preferredHeight: 18
                implicitWidth: limitLabel.implicitWidth + 12
                radius: 9
                color: bat.chargeLimit !== ""
                    ? Qt.rgba(0.96, 0.65, 0.14, 0.14)
                    : Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.06)
                border.width: 1
                border.color: bat.chargeLimit !== ""
                    ? Qt.rgba(0.96, 0.65, 0.14, 0.40)
                    : Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.10)
                Text {
                    id: limitLabel
                    anchors.centerIn: parent
                    text: bat.limitText
                    font.pixelSize: Kirigami.Theme.smallFont.pixelSize - 1
                    font.weight: Font.DemiBold
                    color: bat.limitColor
                    opacity: bat.chargeLimit !== "" ? 0.95 : 0.60
                }
            }
        }

        Item {
            Layout.fillWidth: true
            Layout.preferredHeight: 8

            // Track with subtle border.
            Rectangle {
                anchors.fill: parent
                radius: 4
                color: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.08)
                border.width: 1
                border.color: Qt.rgba(1, 1, 1, 0.08)
            }

            Item {
                anchors.fill: parent
                clip: true

                Rectangle {
                    id: energyFill
                    width: Math.max(8, parent.width * (bat.pct / 100))
                    visible: bat.pct > 0
                    height: parent.height
                    radius: 4
                    gradient: Gradient {
                        GradientStop { position: 0.0; color: bat.fillTop }
                        GradientStop { position: 1.0; color: bat.fillBottom }
                    }
                    border.width: 1
                    border.color: Qt.rgba(1, 1, 1, 0.12)
                    Behavior on width { NumberAnimation { duration: 440; easing.type: Easing.OutCubic } }
                    SequentialAnimation on opacity {
                        running: bat.batteryStatus === "Charging"
                        loops: Animation.Infinite
                        NumberAnimation { to: 0.70; duration: 1100; easing.type: Easing.InOutQuad }
                        NumberAnimation { to: 1.0; duration: 1100; easing.type: Easing.InOutQuad }
                    }
                    // Glass gloss across the top of the fill.
                    Rectangle {
                        anchors.top: parent.top
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.topMargin: 1
                        anchors.leftMargin: 2
                        anchors.rightMargin: 2
                        height: 2
                        radius: 1
                        color: Qt.rgba(1, 1, 1, 0.28)
                    }
                }
            }

            // Segment ticks for an authentic meter feel.
            Row {
                anchors.fill: parent
                Item { width: parent.width * 0.25 - 0.5; height: 1 }
                Rectangle { width: 1; height: parent.height; color: Qt.rgba(0, 0, 0, 0.35) }
                Item { width: parent.width * 0.25 - 1; height: 1 }
                Rectangle { width: 1; height: parent.height; color: Qt.rgba(0, 0, 0, 0.35) }
                Item { width: parent.width * 0.25 - 1; height: 1 }
                Rectangle { width: 1; height: parent.height; color: Qt.rgba(0, 0, 0, 0.35) }
                Item { width: parent.width * 0.25 - 0.5; height: 1 }
            }
        }
    }
}
