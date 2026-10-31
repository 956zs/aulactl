import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import AulaGui

// Quick key groups for multi-select: all, WASD, letters, digits, modifiers, a row, none.
Flow {
    id: bar

    property var selection: []

    signal chosen(var codes)

    spacing: 6

    component GroupChip: Rectangle {
        id: chip

        property string text
        property string selector

        implicitWidth: chipLabel.implicitWidth + 24
        implicitHeight: 32
        radius: 10
        color: hover.hovered ? Qt.tint(Theme.surfaceContainerHighest, Qt.alpha(Theme.surfaceContent, 0.08))
                             : Theme.surfaceContainerHighest
        border.width: 1
        border.color: Theme.outlineVariant
        opacity: bar.enabled ? 1 : 0.5

        Label {
            id: chipLabel
            anchors.centerIn: parent
            text: chip.text
            font.pixelSize: 13
            font.weight: Font.Medium
            color: Theme.surfaceContent
        }

        HoverHandler {
            id: hover
            cursorShape: Qt.PointingHandCursor
        }

        TapHandler {
            onTapped: bar.chosen(chip.selector.length > 0 ? JSON.parse(KeyTables.select(chip.selector)) : [])
        }
    }

    GroupChip { text: I18n.text("select.all"); selector: "all" }
    GroupChip { text: "WASD"; selector: "wasd" }
    GroupChip { text: I18n.text("select.letters"); selector: "letters" }
    GroupChip { text: I18n.text("select.digits"); selector: "digits" }
    GroupChip { text: I18n.text("select.mods"); selector: "mods" }

    Repeater {
        model: 5
        delegate: GroupChip {
            required property int index
            text: I18n.text("select.row").arg(index + 1)
            selector: "row:" + index
        }
    }

    GroupChip { text: I18n.text("select.none"); selector: "" }

    Label {
        height: 32
        verticalAlignment: Text.AlignVCenter
        leftPadding: 6
        text: I18n.text("select.count").arg(bar.selection.length)
        font.pixelSize: 13
        color: Theme.surfaceVariantContent
    }
}
