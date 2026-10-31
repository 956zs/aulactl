import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import AulaGui

// Pending edits of a page: how many, and Preview / Discard / Apply.
Rectangle {
    id: bar

    property int count: 0
    property string countText: I18n.text("apply.pending").arg(count)
    property bool canApply: Model.editable

    signal apply
    signal discard
    signal preview

    implicitHeight: row.implicitHeight + 20
    radius: 16
    color: count > 0 ? Theme.secondaryContainer : Theme.surfaceContainerHigh

    Behavior on color {
        ColorAnimation { duration: 150 }
    }

    RowLayout {
        id: row
        anchors.fill: parent
        anchors.margins: 10
        anchors.leftMargin: 16
        spacing: 10

        Icon {
            name: bar.count > 0 ? "edit_note" : "check_circle"
            size: 22
            color: bar.count > 0 ? Theme.secondaryContainerContent : Theme.surfaceVariantContent
        }

        Label {
            Layout.fillWidth: true
            text: bar.count > 0 ? bar.countText : I18n.text("apply.none")
            elide: Text.ElideRight
            font.weight: Font.Medium
            color: bar.count > 0 ? Theme.secondaryContainerContent : Theme.surfaceVariantContent
        }

        TonalButton {
            visible: bar.count > 0
            icon: "difference"
            text: I18n.text("apply.preview")
            onClicked: bar.preview()
        }

        TonalButton {
            visible: bar.count > 0
            icon: "undo"
            text: I18n.text("apply.discard")
            onClicked: bar.discard()
        }

        Button {
            visible: bar.count > 0
            enabled: bar.canApply
            text: I18n.text("apply.apply")
            icon.source: ""
            highlighted: true
            onClicked: bar.apply()
        }
    }
}
