import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts
import AulaGui

// Shows what a write would change (core snapshot diff) and asks for confirmation. With `okText` empty it
// only shows the list.
Dialog {
    id: dialog

    property string text
    property var changes: []
    property string error
    property string okText: I18n.text("apply.apply")
    property bool danger: false
    property var onOk: null

    // `preview` is `{ changes }` or `{ error }` from Device/Model.preview.
    function ask(title, text, preview, okText, callback) {
        dialog.title = title
        dialog.text = text
        dialog.error = preview.error ?? ""
        dialog.changes = preview.changes ?? []
        dialog.okText = okText
        dialog.onOk = callback
        open()
    }

    parent: Overlay.overlay
    anchors.centerIn: parent
    width: Math.min(620, (parent ? parent.width : 700) - 80)
    height: Math.min(implicitHeight, (parent ? parent.height : 600) - 80)
    modal: true
    Material.background: Theme.surfaceContainerHigh
    Material.foreground: Theme.surfaceContent
    Material.accent: Theme.primary
    Material.roundedScale: Material.ExtraLargeScale

    contentItem: ColumnLayout {
        spacing: 10

        Label {
            Layout.fillWidth: true
            visible: dialog.text.length > 0
            text: dialog.text
            wrapMode: Text.Wrap
            color: Theme.surfaceVariantContent
        }

        Label {
            Layout.fillWidth: true
            visible: dialog.error.length > 0
            text: dialog.error
            wrapMode: Text.Wrap
            color: Theme.error
        }

        Label {
            Layout.fillWidth: true
            visible: dialog.error.length === 0
            text: dialog.changes.length === 0 ? I18n.text("changes.none") : I18n.text("changes.count").arg(dialog.changes.length)
            font.weight: Font.DemiBold
            color: Theme.surfaceContent
        }

        ListView {
            Layout.fillWidth: true
            Layout.preferredHeight: Math.min(contentHeight, 320)
            visible: dialog.changes.length > 0
            clip: true
            model: dialog.changes
            spacing: 2
            ScrollBar.vertical: ScrollBar {}

            delegate: Label {
                required property string modelData
                width: ListView.view.width - 12
                text: modelData
                wrapMode: Text.WrapAnywhere
                font.family: "monospace"
                font.pixelSize: 12
                color: Theme.surfaceVariantContent
            }
        }
    }

    footer: DialogButtonBox {
        Material.background: "transparent"

        Button {
            text: dialog.okText.length > 0 ? I18n.text("dialog.cancel") : I18n.text("dialog.ok")
            flat: true
            DialogButtonBox.buttonRole: DialogButtonBox.RejectRole
        }

        Button {
            visible: dialog.okText.length > 0
            enabled: dialog.error.length === 0 && dialog.changes.length > 0
            text: dialog.okText
            flat: true
            Material.foreground: dialog.danger ? Theme.error : Theme.primary
            DialogButtonBox.buttonRole: DialogButtonBox.AcceptRole
        }
    }

    onAccepted: if (onOk) onOk()
}
