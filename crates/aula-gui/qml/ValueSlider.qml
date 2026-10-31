import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts
import AulaGui

// A labelled slider with its value as text. Shows `value` only; moving emits `moved` with the new
// value (whole steps) and waits for it to come back. `mixed` shows "mixed" when the selection differs.
ColumnLayout {
    id: row

    property string title
    property string help
    property real from: 0
    property real to: 100
    property real stepSize: 1
    property real value: 0
    property bool mixed: false
    // Text for a value.
    property var format: v => String(v)

    signal moved(real value)

    spacing: 2
    opacity: enabled ? 1 : 0.5

    RowLayout {
        Layout.fillWidth: true
        spacing: 8

        Label {
            Layout.fillWidth: true
            text: row.title
            font.weight: Font.Medium
            color: Theme.surfaceContent
            elide: Text.ElideRight
        }

        Rectangle {
            implicitWidth: valueLabel.implicitWidth + 20
            implicitHeight: 28
            radius: 8
            color: row.mixed ? Theme.surfaceContainerHighest : Theme.secondaryContainer

            Label {
                id: valueLabel
                anchors.centerIn: parent
                text: row.mixed ? I18n.text("common.mixed") : row.format(slider.value)
                font.weight: Font.DemiBold
                font.features: { "tnum": 1 }
                color: row.mixed ? Theme.surfaceVariantContent : Theme.secondaryContainerContent
            }
        }
    }

    Label {
        Layout.fillWidth: true
        visible: row.help.length > 0
        text: row.help
        wrapMode: Text.Wrap
        font.pixelSize: 12
        color: Theme.surfaceVariantContent
    }

    Slider {
        id: slider
        Layout.fillWidth: true
        from: row.from
        to: row.to
        stepSize: row.stepSize
        snapMode: Slider.SnapAlways
        value: row.value
        Material.accent: Theme.primary
        onMoved: row.moved(value)
        // Keep following `value` after a drag.
        onPressedChanged: if (!pressed) value = Qt.binding(() => row.value)
    }
}
