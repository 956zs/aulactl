import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts
import AulaGui

// The 40 advanced-key slots (DKS, mod-tap, toggle), binding a slot to a key (keymap action `AdvN`), and the
// SOCD pairs. Edits go into drafts of the snapshot's `advanced`, `socd` and `keymap` sections.
ScrollView {
    id: page

    contentWidth: availableWidth
    clip: true

    property var advanced: []
    property var socd: null
    property var keymap: ({ base: {}, fn: {} })
    property int slot: 0
    property string bindLayer: "base"

    readonly property var base: Model.snapshot
    readonly property var sections: ({ advanced: advanced, socd: socd, keymap: keymap })
    readonly property var adv: advanced[slot] ?? ({ type: "empty" })
    // Routes are in 0.1 mm.
    readonly property int maxRoute: Math.max(1, Math.floor(Model.travel.max * Model.unitMm * 10 + 1e-6))
    readonly property var keyGroups: ["letters", "digits", "modifiers", "editing", "symbols", "function", "numpad", "international"]
    // Everything but macros and other advanced keys, which cannot be used inside an advanced key.
    readonly property var actionGroups: ["special", "letters", "digits", "modifiers", "editing", "symbols", "function",
                                         "numpad", "international", "media", "lighting", "mouse", "shortcuts"]
    readonly property var changedSlots: {
        const out = []
        if (!base)
            return out
        for (let i = 0; i < advanced.length; i++)
            if (!Model.same(advanced[i], base.advanced[i]))
                out.push(i)
        return out
    }
    readonly property int changedKeys: {
        if (!base)
            return 0
        let n = 0
        for (const l of ["base", "fn"])
            for (const c in keymap[l])
                if (keymap[l][c] !== base.keymap[l][c])
                    n++
        return n
    }
    readonly property int changedSocd: {
        if (!base || !socd || !base.socd)
            return 0
        let n = socd.enabled !== base.socd.enabled ? 1 : 0
        if (!Model.same(socd.pairs, base.socd.pairs))
            n++
        return n
    }

    function reset() {
        advanced = Model.copy(base?.advanced ?? [])
        socd = Model.copy(base?.socd ?? null)
        keymap = Model.copy(base?.keymap ?? { base: {}, fn: {} })
    }

    function boundKeys(s) {
        const out = []
        for (const l of ["base", "fn"])
            for (const c in keymap[l])
                if (keymap[l][c] === "Adv" + s)
                    out.push({ layer: l, code: c })
        return out
    }

    function boundText(s) {
        return boundKeys(s).map(b => (b.layer === "fn" ? "Fn+" : "") + (Model.keyByCode(b.code)?.label ?? b.code)).join(", ")
    }

    function setAdv(a) {
        const all = Model.copy(advanced)
        all[slot] = a
        advanced = all
    }

    function editAdv(edit) {
        const a = Model.copy(adv)
        edit(a)
        setAdv(a)
    }

    // A new slot of `type` with the official defaults.
    function setType(type) {
        const act = 15
        const bottom = Math.max(1, maxRoute - 4)
        switch (type) {
        case "dks":
            setAdv({ type: "dks", routes: [act, bottom, bottom, act], keys: [{ action: "KeyA", phases: [1, 0, 0, 0] }] })
            break
        case "mod-tap":
            setAdv({ type: "mod-tap", hold: "ControlLeft", tap: "Escape", threshold: 20 })
            break
        case "toggle":
            setAdv({ type: "toggle", action: "ShiftLeft" })
            break
        default:
            setAdv({ type: "empty" })
        }
    }

    function clearSlot() {
        const km = Model.copy(keymap)
        for (const b of boundKeys(slot))
            km[b.layer][b.code] = b.layer === "base" ? KeyTables.baseDefault(b.code) : "Default"
        keymap = km
        setAdv({ type: "empty" })
    }

    function bindToggle(code) {
        const km = Model.copy(keymap)
        const name = "Adv" + slot
        km[bindLayer][code] = km[bindLayer][code] === name ? (bindLayer === "base" ? KeyTables.baseDefault(code) : "Default") : name
        keymap = km
    }

    function typeShort(a) {
        switch (a?.type) {
        case "dks": return "DKS"
        case "mod-tap": return "MT"
        case "toggle": return "TGL"
        case "other": return "?"
        default: return ""
        }
    }

    function actionText(name) {
        const l = KeyTables.label(name)
        return (l.length > 0 ? l : name) + "  ·  " + Model.actionName(name)
    }

    function editPairs(edit) {
        const s = Model.copy(socd)
        edit(s.pairs)
        socd = s
    }

    Connections {
        target: Device
        function onSnapshotJsonChanged() {
            page.reset()
        }
    }
    Component.onCompleted: reset()

    // A button showing an action; a click opens the picker.
    component ActionButton: Rectangle {
        id: ab

        property string action
        property var groups: page.actionGroups
        property string title

        signal picked(string name)

        implicitWidth: Math.max(140, abLabel.implicitWidth + 28)
        implicitHeight: 40
        radius: 12
        color: abHover.hovered ? Qt.tint(Theme.surfaceContainerHighest, Qt.alpha(Theme.surfaceContent, 0.08))
                               : Theme.surfaceContainerHighest
        border.width: 1
        border.color: Theme.outlineVariant
        opacity: enabled ? 1 : 0.5

        Label {
            id: abLabel
            anchors.centerIn: parent
            width: Math.min(implicitWidth, parent.width - 16)
            text: page.actionText(ab.action)
            elide: Text.ElideRight
            font.weight: Font.Medium
            color: Theme.surfaceContent
        }

        HoverHandler {
            id: abHover
            cursorShape: Qt.PointingHandCursor
        }

        TapHandler {
            onTapped: picker.pick(ab.title, ab.action, "", ab.groups, name => ab.picked(name))
        }
    }

    // A DKS phase value that cycles none -> tap -> hold -> held on click.
    component PhaseChip: Rectangle {
        id: pc

        property int value: 0

        signal cycled(int value)

        implicitWidth: 76
        implicitHeight: 36
        radius: 10
        color: value === 0 ? Theme.surfaceContainerHighest : value === 1 ? Theme.primaryContainer : Theme.secondaryContainer
        border.width: 1
        border.color: Theme.outlineVariant

        Label {
            anchors.centerIn: parent
            text: I18n.text("advanced.phase_" + pc.value)
            font.pixelSize: 13
            font.weight: Font.Medium
            color: pc.value === 1 ? Theme.primaryContainerContent : pc.value > 1 ? Theme.secondaryContainerContent : Theme.surfaceVariantContent
        }

        HoverHandler {
            cursorShape: Qt.PointingHandCursor
        }

        TapHandler {
            onTapped: pc.cycled((pc.value + 1) % 4)
        }
    }

    ColumnLayout {
        width: page.availableWidth
        spacing: 16

        ApplyBar {
            Layout.fillWidth: true
            visible: Model.loaded
            count: page.changedSlots.length + page.changedKeys + page.changedSocd
            onDiscard: page.reset()
            onApply: Model.apply(page.sections)
            onPreview: changes.ask(I18n.text("apply.preview_title"), I18n.text("apply.confirm_text"),
                                   Model.preview(page.sections), I18n.text("apply.apply"), () => Model.apply(page.sections))
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
                Layout.maximumWidth: page.availableWidth >= 980 ? 360 : 100000
                Layout.alignment: Qt.AlignTop
                title: I18n.text("advanced.slots")
                icon: "grid_view"

                GridLayout {
                    Layout.fillWidth: true
                    columns: 5
                    columnSpacing: 6
                    rowSpacing: 6

                    Repeater {
                        model: page.advanced.length

                        delegate: Rectangle {
                            id: tile

                            required property int index
                            readonly property var a: page.advanced[index]
                            readonly property bool active: page.slot === index
                            readonly property bool used: a.type !== "empty"
                            readonly property bool bound: page.boundKeys(index).length > 0

                            Layout.fillWidth: true
                            implicitHeight: 50
                            radius: 10
                            color: active ? Theme.secondaryContainer
                                 : used ? Theme.primaryContainer
                                 : tileHover.hovered ? Qt.tint(Theme.surfaceContainerHighest, Qt.alpha(Theme.surfaceContent, 0.06))
                                 : Theme.surfaceContainerHighest
                            border.width: active ? 2 : 0
                            border.color: Theme.primary

                            Column {
                                anchors.centerIn: parent

                                Label {
                                    anchors.horizontalCenter: parent.horizontalCenter
                                    text: "A" + tile.index
                                    font.pixelSize: 12
                                    color: tile.active ? Theme.secondaryContainerContent
                                         : tile.used ? Theme.primaryContainerContent : Theme.surfaceVariantContent
                                }

                                Label {
                                    anchors.horizontalCenter: parent.horizontalCenter
                                    text: page.typeShort(tile.a) + (tile.bound ? " •" : "")
                                    visible: text.length > 0
                                    font.pixelSize: 12
                                    font.weight: Font.DemiBold
                                    color: tile.active ? Theme.secondaryContainerContent : Theme.primaryContainerContent
                                }
                            }

                            Rectangle {
                                visible: page.changedSlots.indexOf(tile.index) >= 0
                                anchors.top: parent.top
                                anchors.right: parent.right
                                anchors.margins: 5
                                width: 7
                                height: 7
                                radius: 4
                                color: Theme.primary
                            }

                            HoverHandler {
                                id: tileHover
                                cursorShape: Qt.PointingHandCursor
                            }

                            TapHandler {
                                onTapped: page.slot = tile.index
                            }
                        }
                    }
                }

                Label {
                    Layout.fillWidth: true
                    text: I18n.text("advanced.slots_help")
                    wrapMode: Text.Wrap
                    font.pixelSize: 12
                    color: Theme.surfaceVariantContent
                }
            }

            ColumnLayout {
                Layout.fillWidth: true
                Layout.preferredWidth: 2
                Layout.alignment: Qt.AlignTop
                spacing: 16

                Card {
                    Layout.fillWidth: true
                    enabled: Model.editable
                    title: I18n.text("advanced.slot_title").arg(page.slot)
                    icon: "tune"

                    SegmentedButtons {
                        visible: page.adv.type !== "other"
                        model: ["empty", "dks", "mod-tap", "toggle"]
                        current: page.adv.type
                        labelFor: t => I18n.text("advanced.type_" + t.replace("-", "_"))
                        onActivated: t => page.setType(t)
                    }

                    Label {
                        Layout.fillWidth: true
                        text: I18n.text("advanced.help_" + page.adv.type.replace("-", "_"))
                        wrapMode: Text.Wrap
                        font.pixelSize: 13
                        color: Theme.surfaceVariantContent
                    }

                    // ---- DKS ----
                    ColumnLayout {
                        Layout.fillWidth: true
                        visible: page.adv.type === "dks"
                        spacing: 10

                        Repeater {
                            model: 4

                            delegate: ValueSlider {
                                required property int index

                                Layout.fillWidth: true
                                title: I18n.text("advanced.route_" + (index + 1))
                                from: 1
                                to: page.maxRoute
                                value: page.adv.routes?.[index] ?? 1
                                format: v => I18n.text("common.mm").arg((v / 10).toFixed(1))
                                onMoved: v => page.editAdv(a => a.routes[index] = Math.round(v))
                            }
                        }

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: 8

                            Label {
                                Layout.fillWidth: true
                                text: I18n.text("advanced.dks_keys")
                                font.weight: Font.DemiBold
                                color: Theme.surfaceContent
                            }

                            Repeater {
                                model: ["advanced.phase_col_1", "advanced.phase_col_2", "advanced.phase_col_3", "advanced.phase_col_4"]

                                delegate: Label {
                                    required property string modelData
                                    Layout.preferredWidth: 76
                                    horizontalAlignment: Text.AlignHCenter
                                    text: I18n.text(modelData)
                                    font.pixelSize: 12
                                    wrapMode: Text.Wrap
                                    color: Theme.surfaceVariantContent
                                }
                            }

                            Item {
                                Layout.preferredWidth: 40
                            }
                        }

                        Repeater {
                            model: page.adv.type === "dks" ? page.adv.keys : []

                            delegate: RowLayout {
                                id: dksRow

                                required property var modelData
                                required property int index

                                Layout.fillWidth: true
                                spacing: 8

                                ActionButton {
                                    Layout.fillWidth: true
                                    title: I18n.text("advanced.dks_key").arg(dksRow.index + 1)
                                    action: dksRow.modelData.action
                                    onPicked: name => page.editAdv(a => a.keys[dksRow.index].action = name)
                                }

                                Repeater {
                                    model: 4

                                    delegate: PhaseChip {
                                        required property int index
                                        value: dksRow.modelData.phases[index]
                                        onCycled: v => page.editAdv(a => a.keys[dksRow.index].phases[index] = v)
                                    }
                                }

                                ToolButton {
                                    Layout.preferredWidth: 40
                                    text: "✕"
                                    enabled: page.adv.keys.length > 1
                                    onClicked: page.editAdv(a => a.keys.splice(dksRow.index, 1))
                                }
                            }
                        }

                        TonalButton {
                            icon: "add"
                            text: I18n.text("advanced.dks_add")
                            enabled: page.adv.type === "dks" && page.adv.keys.length < 4
                            onClicked: page.editAdv(a => a.keys.push({ action: "KeyB", phases: [0, 0, 1, 0] }))
                        }
                    }

                    // ---- Mod-tap ----
                    ColumnLayout {
                        Layout.fillWidth: true
                        visible: page.adv.type === "mod-tap"
                        spacing: 10

                        SettingRow {
                            Layout.fillWidth: true
                            title: I18n.text("advanced.mt_hold")
                            help: I18n.text("advanced.mt_hold_help")

                            ActionButton {
                                title: I18n.text("advanced.mt_hold")
                                action: page.adv.hold ?? ""
                                onPicked: name => page.editAdv(a => a.hold = name)
                            }
                        }

                        SettingRow {
                            Layout.fillWidth: true
                            title: I18n.text("advanced.mt_tap")
                            help: I18n.text("advanced.mt_tap_help")

                            ActionButton {
                                title: I18n.text("advanced.mt_tap")
                                action: page.adv.tap ?? ""
                                onPicked: name => page.editAdv(a => a.tap = name)
                            }
                        }

                        ValueSlider {
                            Layout.fillWidth: true
                            title: I18n.text("advanced.mt_threshold")
                            help: I18n.text("advanced.mt_threshold_help")
                            from: 15
                            to: 100
                            value: page.adv.threshold ?? 20
                            format: v => I18n.text("common.ms").arg(Math.round(v) * 10)
                            onMoved: v => page.editAdv(a => a.threshold = Math.round(v))
                        }
                    }

                    // ---- Toggle ----
                    SettingRow {
                        Layout.fillWidth: true
                        visible: page.adv.type === "toggle"
                        title: I18n.text("advanced.tgl_action")

                        ActionButton {
                            title: I18n.text("advanced.tgl_action")
                            action: page.adv.action ?? ""
                            onPicked: name => page.editAdv(a => a.action = name)
                        }
                    }

                    // ---- Other (RS / RKRT / unknown): read-only ----
                    Label {
                        Layout.fillWidth: true
                        visible: page.adv.type === "other"
                        text: page.adv.payload ?? ""
                        wrapMode: Text.WrapAnywhere
                        font.family: "monospace"
                        font.pixelSize: 12
                        color: Theme.surfaceVariantContent
                    }

                    TonalButton {
                        visible: page.adv.type !== "empty"
                        icon: "delete"
                        text: I18n.text("advanced.clear")
                        onClicked: confirm.ask(I18n.text("advanced.clear"),
                                               page.boundKeys(page.slot).length > 0
                                                   ? I18n.text("advanced.clear_bound").arg(page.boundText(page.slot))
                                                   : I18n.text("advanced.clear_confirm").arg(page.slot),
                                               I18n.text("common.clear"), () => page.clearSlot(), true)
                    }
                }

                Card {
                    Layout.fillWidth: true
                    visible: page.adv.type !== "empty"
                    enabled: Model.editable
                    title: I18n.text("advanced.bind_title")
                    icon: "link"

                    Label {
                        Layout.fillWidth: true
                        text: I18n.text("advanced.bind_help").arg(page.slot)
                        wrapMode: Text.Wrap
                        font.pixelSize: 13
                        color: Theme.surfaceVariantContent
                    }

                    SegmentedButtons {
                        model: ["base", "fn"]
                        current: page.bindLayer
                        labelFor: l => I18n.text("advanced.layer_" + l)
                        onActivated: l => page.bindLayer = l
                    }

                    KeyboardView {
                        Layout.fillWidth: true
                        Layout.maximumWidth: 900
                        Layout.alignment: Qt.AlignHCenter
                        values: {
                            const v = {}
                            const name = "Adv" + page.slot
                            const km = page.keymap[page.bindLayer] ?? {}
                            for (const k of Model.layout.keys) {
                                const a = km[k.code] ?? "Default"
                                const mine = a === name
                                const fixed = page.bindLayer === "fn" && Model.fnFixed.indexOf(k.code) >= 0
                                v[k.code] = {
                                    text: KeyTables.label(a).length > 0 ? KeyTables.label(a) : k.label,
                                    color: mine ? Theme.primary : Theme.surfaceContainerHighest,
                                    textColor: mine ? Theme.primaryContent : Theme.surfaceContent,
                                    disabled: fixed
                                }
                            }
                            return v
                        }
                        onKeyClicked: code => page.bindToggle(code)
                    }
                }

                Card {
                    Layout.fillWidth: true
                    visible: page.socd !== null
                    enabled: Model.editable
                    title: I18n.text("advanced.socd_title")
                    icon: "swap_horiz"

                    SettingRow {
                        Layout.fillWidth: true
                        title: I18n.text("advanced.socd_enabled")
                        help: I18n.text("advanced.socd_help")
                              + (Model.info?.socdHotkey ? " " + I18n.text("advanced.socd_hotkey") : "")

                        ToggleSwitch {
                            checked: page.socd?.enabled ?? false
                            onToggled: on => {
                                const s = Model.copy(page.socd)
                                s.enabled = on
                                page.socd = s
                            }
                        }
                    }

                    Label {
                        Layout.fillWidth: true
                        visible: (page.socd?.pairs.length ?? 0) === 0
                        text: I18n.text("advanced.socd_none")
                        wrapMode: Text.Wrap
                        color: Theme.surfaceVariantContent
                    }

                    Repeater {
                        model: page.socd?.pairs ?? []

                        delegate: RowLayout {
                            id: pairRow

                            required property var modelData
                            required property int index

                            Layout.fillWidth: true
                            spacing: 8

                            ActionButton {
                                title: I18n.text("advanced.socd_key1")
                                groups: page.keyGroups
                                action: pairRow.modelData.key1
                                onPicked: name => page.editPairs(p => p[pairRow.index].key1 = name)
                            }

                            ActionButton {
                                title: I18n.text("advanced.socd_key2")
                                groups: page.keyGroups
                                action: pairRow.modelData.key2
                                onPicked: name => page.editPairs(p => p[pairRow.index].key2 = name)
                            }

                            DropDown {
                                Layout.fillWidth: true
                                implicitWidth: 200
                                choices: ["last-wins", "key1-wins", "key2-wins", "neutral"]
                                         .map(m => ({ label: I18n.text("advanced.socd_" + m.replace("-", "_")), value: m }))
                                current: pairRow.modelData.mode
                                onChosen: m => page.editPairs(p => p[pairRow.index].mode = m)
                            }

                            ToolButton {
                                text: "✕"
                                onClicked: page.editPairs(p => p.splice(pairRow.index, 1))
                            }
                        }
                    }

                    TonalButton {
                        icon: "add"
                        text: I18n.text("advanced.socd_add")
                        enabled: (page.socd?.pairs.length ?? 20) < 20
                        onClicked: page.editPairs(p => p.push({ mode: "last-wins", key1: "KeyA", key2: "KeyD" }))
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

    AppDialog {
        id: confirm
    }
}
