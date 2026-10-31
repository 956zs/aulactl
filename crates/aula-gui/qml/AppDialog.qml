import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import AulaGui

// Modal message. With `cancelText` empty it is a plain notice with one button. `ask` sets everything and
// runs `callback` on OK; `danger` colours OK as destructive.
Dialog {
    id: dialog

    property string text
    property string okText: I18n.text("dialog.ok")
    property string cancelText
    property bool danger: false
    property var onOk: null

    function ask(title, text, okText, callback, danger) {
        dialog.title = title
        dialog.text = text
        dialog.okText = okText
        dialog.cancelText = I18n.text("dialog.cancel")
        dialog.danger = danger ?? false
        dialog.onOk = callback
        open()
    }

    parent: Overlay.overlay
    anchors.centerIn: parent
    // Fixed: a width bound to the overlay makes the wrapped text loop on implicitHeight.
    width: 460
    modal: true
    Material.background: Theme.surfaceContainerHigh
    Material.foreground: Theme.surfaceContent
    Material.accent: Theme.primary
    Material.roundedScale: Material.ExtraLargeScale

    contentItem: Label {
        text: dialog.text
        wrapMode: Text.Wrap
        color: Theme.surfaceVariantContent
    }

    footer: DialogButtonBox {
        Material.background: "transparent"

        Button {
            visible: dialog.cancelText.length > 0
            text: dialog.cancelText
            flat: true
            DialogButtonBox.buttonRole: DialogButtonBox.RejectRole
        }

        Button {
            text: dialog.okText
            flat: true
            Material.foreground: dialog.danger ? Theme.error : Theme.primary
            DialogButtonBox.buttonRole: DialogButtonBox.AcceptRole
        }
    }

    onAccepted: if (onOk) onOk()
}
