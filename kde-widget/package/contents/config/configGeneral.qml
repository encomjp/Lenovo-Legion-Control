import QtQuick 2.15
import QtQuick.Controls 2.15 as QQC2
import QtQuick.Layouts 1.15
import org.kde.kirigami 2.20 as Kirigami
import org.kde.plasma.plasmoid 2.0
import org.kde.plasma.configuration 2.0

Kirigami.ScrollablePage {
    property alias cfg_RefreshInterval: refreshSpin.value
    property alias cfg_ShowGauges: gaugesCheck.checked
    property alias cfg_ShowSparklines: sparklinesCheck.checked

    ColumnLayout {
        width: parent.width
        spacing: Kirigami.Units.largeSpacing
        anchors.margins: Kirigami.Units.largeSpacing

        Kirigami.Heading {
            text: i18n("Widget settings")
            level: 3
        }

        RowLayout {
            QQC2.Label {
                text: i18n("Refresh interval:")
            }
            QQC2.SpinBox {
                id: refreshSpin
                from: 1
                to: 10
                textFromValue: function(value, locale) {
                    return qsTr("%1 s").arg(value)
                }
                valueFromText: function(text, locale) {
                    return parseInt(text, 10)
                }
            }
        }

        QQC2.CheckBox {
            id: gaugesCheck
            text: i18n("Show circular temperature gauges")
        }

        QQC2.CheckBox {
            id: sparklinesCheck
            text: i18n("Show temperature sparklines")
        }

        QQC2.Label {
            Layout.fillWidth: true
            text: i18n("Changes apply after closing the configuration dialog.")
            font.pointSize: Kirigami.Theme.smallFont.pointSize
            opacity: 0.6
            wrapMode: Text.Wrap
        }
    }
}
