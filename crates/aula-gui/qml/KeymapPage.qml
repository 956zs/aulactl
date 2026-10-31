import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts
import AulaGui

// Base and Fn layer: click a key, pick what it does. Keys whose Fn function is fixed by the firmware cannot
// be changed on the Fn layer. Edits go into a draft of the snapshot's `keymap` section.
ScrollView {
    id: page

    contentWidth: availableWidth
    clip: true

    property var keymap: ({ base: {}, fn: {} })
    property string fnLayer: "base"
    property string current: ""

    readonly property var base: Model.snapshot
    readonly property var layerMap: keymap[fnLayer] ?? {}
    readonly property var changed: {
        const out = []
        if (!base)
            return out
        for (const l of ["base", "fn"])
            for (const k of Model.layout.keys)
                if (keymap[l]?.[k.code] !== base.keymap[l][k.code])
                    out.push(l + ":" + k.code)
        return out
    }

    function reset() {
        keymap = Model.copy(base?.keymap ?? { base: {}, fn: {} })
    }

    function defaultFor(layer, code) {
        return layer === "base" ? KeyTables.baseDefault(code) : "Default"
    }

    function locked(code) {
        return fnLayer === "fn" && (Model.fnFixed.indexOf(code) >= 0 || code === "KeyFn")
    }

    function setAction(layer, code, action) {
        const m = Model.copy(keymap)
        m[layer][code] = action
        keymap = m
    }

    function resetLayer(layer) {
        const m = Model.copy(keymap)
        for (const k of Model.layout.keys)
            if (!(layer === "fn" && Model.fnFixed.indexOf(k.code) >= 0))
                m[layer][k.code] = defaultFor(layer, k.code)
        keymap = m
    }

    // Picker groups for a key: a macro can only sit on a key with a normal key code (not Fn).
    function groupsFor(code) {
        const all = Model.actionGroups.map(g => g.id)
        return code === "KeyFn" ? all.filter(g => g !== "macros") : all
    }

    function pick(code) {
        current = code
        const key = Model.keyByCode(code)
        picker.pick(I18n.text("keymap.pick_title").arg(key ? key.label : code).arg(I18n.text("keymap.layer_" + fnLayer)),
                    layerMap[code], defaultFor(fnLayer, code), groupsFor(code),
                    name => page.setAction(page.fnLayer, code, name))
    }

    Connections {
        target: Device
        function onSnapshotJsonChanged() {
            page.reset()
        }
    }
    Component.onCompleted: reset()

    ColumnLayout {
        width: page.availableWidth
        spacing: 16

        Card {
            Layout.fillWidth: true
            visible: Model.loaded
            title: I18n.text("keymap.title")
            icon: "keyboard"

            RowLayout {
                Layout.fillWidth: true
                spacing: 16

                SegmentedButtons {
                    model: ["base", "fn"]
                    current: page.fnLayer
                    labelFor: l => I18n.text("keymap.layer_" + l)
                    onActivated: l => page.fnLayer = l
                }

                Label {
                    Layout.fillWidth: true
                    text: I18n.text("keymap.help_" + page.fnLayer)
                    wrapMode: Text.Wrap
                    font.pixelSize: 13
                    color: Theme.surfaceVariantContent
                }
            }

            KeyboardView {
                Layout.fillWidth: true
                Layout.maximumWidth: 1000
                Layout.alignment: Qt.AlignHCenter
                interactive: Model.editable
                selected: page.current.length > 0 ? [page.current] : []
                values: {
                    const v = {}
                    for (const k of Model.layout.keys) {
                        const a = page.layerMap[k.code]
                        if (a === undefined)
                            continue
                        const isDefault = a === "Default" || a === page.defaultFor(page.fnLayer, k.code)
                        const edited = page.changed.indexOf(page.fnLayer + ":" + k.code) >= 0
                        const locked = page.locked(k.code)
                        const e = {
                            color: edited ? Theme.secondaryContainer : !isDefault ? Qt.alpha(Theme.primary, 0.22)
                                                                     : Theme.surfaceContainerHighest,
                            disabled: locked
                        }
                        if (!isDefault || a !== "Default" && page.fnLayer === "fn")
                            e.text = KeyTables.label(a)
                        else if (page.fnLayer === "fn")
                            e.dim = true
                        if (locked)
                            e.sub = I18n.text("keymap.fixed")
                        v[k.code] = e
                    }
                    return v
                }
                onKeyClicked: code => page.pick(code)
            }

            Flow {
                Layout.fillWidth: true
                spacing: 16

                component Legend: Row {
                    property color swatch
                    property string text
                    spacing: 6

                    Rectangle {
                        width: 16
                        height: 16
                        radius: 4
                        color: parent.swatch
                        border.width: 1
                        border.color: Theme.outlineVariant
                    }

                    Label {
                        text: parent.text
                        font.pixelSize: 12
                        color: Theme.surfaceVariantContent
                    }
                }

                Legend { swatch: Qt.alpha(Theme.primary, 0.22); text: I18n.text("keymap.legend_remapped") }
                Legend { swatch: Theme.secondaryContainer; text: I18n.text("keymap.legend_edited") }
                Legend { visible: page.fnLayer === "fn"; swatch: Theme.surfaceContainerHighest; text: I18n.text("keymap.legend_fixed") }
            }
        }

        ApplyBar {
            Layout.fillWidth: true
            visible: Model.loaded
            count: page.changed.length
            countText: I18n.text("keymap.pending").arg(page.changed.length)
            onDiscard: page.reset()
            onApply: Model.apply({ keymap: page.keymap })
            onPreview: changes.ask(I18n.text("apply.preview_title"), I18n.text("apply.confirm_text"),
                                   Model.preview({ keymap: page.keymap }), I18n.text("apply.apply"),
                                   () => Model.apply({ keymap: page.keymap }))
        }

        GridLayout {
            Layout.fillWidth: true
            visible: Model.loaded
            columns: page.availableWidth >= 980 ? 2 : 1
            columnSpacing: 16
            rowSpacing: 16

            Card {
                Layout.fillWidth: true
                Layout.preferredWidth: 1
                Layout.alignment: Qt.AlignTop
                title: I18n.text("keymap.key")
                icon: "touch_app"

                Label {
                    Layout.fillWidth: true
                    visible: page.current.length === 0
                    text: I18n.text("keymap.click_key")
                    wrapMode: Text.Wrap
                    color: Theme.surfaceVariantContent
                }

                InfoRow {
                    Layout.fillWidth: true
                    visible: page.current.length > 0
                    icon: "keyboard"
                    label: I18n.text("keymap.physical")
                    value: Model.keyByCode(page.current)?.label ?? ""
                }

                InfoRow {
                    Layout.fillWidth: true
                    visible: page.current.length > 0
                    icon: "arrow_forward"
                    label: I18n.text("keymap.layer_" + page.fnLayer)
                    value: Model.actionName(page.layerMap[page.current])
                }

                InfoRow {
                    Layout.fillWidth: true
                    visible: page.current.length > 0
                    icon: "settings_backup_restore"
                    label: I18n.text("keymap.default")
                    value: Model.actionName(page.defaultFor(page.fnLayer, page.current))
                }

                Flow {
                    Layout.fillWidth: true
                    visible: page.current.length > 0
                    spacing: 8

                    TonalButton {
                        icon: "edit"
                        text: I18n.text("keymap.change")
                        enabled: Model.editable && !page.locked(page.current)
                        onClicked: page.pick(page.current)
                    }

                    TonalButton {
                        icon: "settings_backup_restore"
                        text: I18n.text("picker.reset")
                        enabled: Model.editable && !page.locked(page.current)
                        onClicked: page.setAction(page.fnLayer, page.current, page.defaultFor(page.fnLayer, page.current))
                    }
                }
            }

            Card {
                Layout.fillWidth: true
                Layout.preferredWidth: 1
                Layout.alignment: Qt.AlignTop
                title: I18n.text("keymap.reset_title")
                icon: "restart_alt"

                Label {
                    Layout.fillWidth: true
                    text: I18n.text("keymap.reset_help")
                    wrapMode: Text.Wrap
                    font.pixelSize: 13
                    color: Theme.surfaceVariantContent
                }

                Flow {
                    Layout.fillWidth: true
                    spacing: 8

                    TonalButton {
                        icon: "restart_alt"
                        text: I18n.text("keymap.reset_base")
                        enabled: Model.editable
                        onClicked: page.resetLayer("base")
                    }

                    TonalButton {
                        icon: "restart_alt"
                        text: I18n.text("keymap.reset_fn")
                        enabled: Model.editable
                        onClicked: page.resetLayer("fn")
                    }
                }
            }
        }
    }

    ActionPicker {
        id: picker
    }

    ChangesDialog {
        id: changes
    }
}
