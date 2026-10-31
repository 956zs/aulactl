import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import AulaGui

// Device name with status, firmware and poll-rate chips, a refresh button and the busy indicator.
RowLayout {
    id: header

    component Chip: Rectangle {
        id: chip

        property string icon
        property string text
        property color tint: Theme.surfaceVariantContent

        implicitWidth: chipRow.implicitWidth + 24
        implicitHeight: 32
        radius: height / 2
        color: Theme.surfaceContainerHigh

        RowLayout {
            id: chipRow
            anchors.centerIn: parent
            spacing: 6

            Icon {
                name: chip.icon
                size: 18
                filled: true
                color: chip.tint
            }

            Label {
                text: chip.text
                font.pixelSize: 13
                font.weight: Font.Medium
                color: Theme.surfaceContent
            }
        }
    }

    spacing: 12

    ColumnLayout {
        Layout.fillWidth: true
        spacing: 10

        Label {
            Layout.fillWidth: true
            text: I18n.text("device.name")
            elide: Text.ElideRight
            font.pixelSize: 24
            font.weight: Font.DemiBold
            color: Theme.surfaceContent
        }

        Flow {
            Layout.fillWidth: true
            spacing: 8

            Chip {
                icon: Format.statusIcon
                text: Format.statusText
                tint: Device.connected && Device.supported ? Theme.primary : Theme.surfaceVariantContent
            }

            Chip {
                visible: Device.connected
                icon: "memory"
                text: Format.firmwareText
            }

            Chip {
                visible: Format.pollText.length > 0
                icon: "speed"
                text: Format.pollText
            }

            Chip {
                visible: Device.live
                icon: "sensors"
                text: I18n.text("status.streaming")
                tint: Theme.primary
            }
        }
    }

    TonalButton {
        Layout.alignment: Qt.AlignTop
        icon: "refresh"
        text: I18n.text("common.refresh")
        enabled: !Device.busy && !Device.live && !Device.calibrating
        onClicked: Device.refresh()
    }

    BusyIndicator {
        Layout.alignment: Qt.AlignTop
        implicitWidth: 36
        implicitHeight: 36
        running: Device.busy
        visible: running
    }
}
