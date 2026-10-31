import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts
import AulaGui

// A colour as "#rrggbb": swatch, hex field, preset swatches and a hue slider. Shows `color` only;
// changes emit `picked`.
ColumnLayout {
    id: field

    property string color: "#ff0000"
    property bool showPresets: true

    signal picked(string color)

    readonly property var presets: ["#ff0000", "#ff7f00", "#ffff00", "#7fff00", "#00ff00", "#00ff7f", "#00ffff",
                                    "#007fff", "#0000ff", "#7f00ff", "#ff00ff", "#ff007f", "#ffffff", "#000000"]

    function hex(c) {
        const h = v => ("0" + Math.round(v * 255).toString(16)).slice(-2)
        return "#" + h(c.r) + h(c.g) + h(c.b)
    }

    spacing: 8
    opacity: enabled ? 1 : 0.5

    RowLayout {
        spacing: 10

        Rectangle {
            implicitWidth: 40
            implicitHeight: 40
            radius: 12
            color: field.color
            border.width: 1
            border.color: Theme.outline
        }

        TextField {
            id: hexField
            implicitWidth: 120
            text: field.color
            font.family: "monospace"
            validator: RegularExpressionValidator { regularExpression: /#?[0-9a-fA-F]{0,6}/ }
            Material.accent: Theme.primary
            onEditingFinished: {
                const t = text.startsWith("#") ? text : "#" + text
                if (/^#[0-9a-fA-F]{6}$/.test(t))
                    field.picked(t.toLowerCase())
                text = Qt.binding(() => field.color)
            }
        }

        Slider {
            id: hue
            Layout.fillWidth: true
            from: 0
            to: 359
            stepSize: 1
            value: Math.max(0, Qt.color(field.color).hsvHue) * 359
            Material.accent: Qt.hsva(value / 359, 1, 1, 1)
            onMoved: field.picked(field.hex(Qt.hsva(value / 359, 1, 1, 1)))
            onPressedChanged: if (!pressed) value = Qt.binding(() => Math.max(0, Qt.color(field.color).hsvHue) * 359)
        }
    }

    Flow {
        Layout.fillWidth: true
        visible: field.showPresets
        spacing: 6

        Repeater {
            model: field.presets

            delegate: Rectangle {
                required property string modelData
                width: 26
                height: 26
                radius: 13
                color: modelData
                border.width: modelData === field.color ? 3 : 1
                border.color: modelData === field.color ? Theme.primary : Theme.outlineVariant

                HoverHandler {
                    cursorShape: Qt.PointingHandCursor
                }

                TapHandler {
                    onTapped: field.picked(parent.modelData)
                }
            }
        }
    }
}
