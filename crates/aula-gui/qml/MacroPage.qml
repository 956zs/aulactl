import QtQuick
import QtQuick as QQ
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts
import AulaGui

// The board's 10 macro slots: step editor, recording from the keyboard, play mode, and binding a slot to a
// key (keymap action `MacroN`). Edits go into drafts of the snapshot's `macros` and `keymap` sections.
ScrollView {
    id: page

    contentWidth: availableWidth
    clip: true

    property var macros: []
    property var keymap: ({ base: {}, fn: {} })
    property int slot: 0
    property string bindLayer: "base"
    property int selectedEvent: -1
    property bool recording: false
    property var recorded: []

    readonly property int maxEvents: 62
    readonly property var base: Model.snapshot
    readonly property var sections: ({ macros: macros, keymap: keymap })
    readonly property var macro: macros[slot] ?? null
    readonly property var changedSlots: {
        const out = []
        if (!base)
            return out
        for (let i = 0; i < 10; i++)
            if (!Model.same(macros[i] ?? null, base.macros[i] ?? null))
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
    // Problems of the edited slots: [{ slot, text }].
    readonly property var problems: changedSlots.filter(i => macros[i] !== null)
                                                .map(i => ({ slot: i, text: KeyTables.macroProblem(JSON.stringify(macros[i])) }))
                                                .filter(p => p.text.length > 0)
    readonly property string slotProblem: problems.find(p => p.slot === slot)?.text ?? ""

    function reset() {
        macros = Model.copy(base?.macros ?? [])
        keymap = Model.copy(base?.keymap ?? { base: {}, fn: {} })
        selectedEvent = -1
    }

    // Keys bound to macro slot `s` in the draft: [{ layer, code }].
    function boundKeys(s) {
        const out = []
        for (const l of ["base", "fn"])
            for (const c in keymap[l])
                if (keymap[l][c] === "Macro" + s)
                    out.push({ layer: l, code: c })
        return out
    }

    function boundText(s) {
        return boundKeys(s).map(b => (b.layer === "fn" ? "Fn+" : "") + (Model.keyByCode(b.code)?.label ?? b.code)).join(", ")
    }

    function setMacro(m) {
        const all = Model.copy(macros)
        all[slot] = m
        macros = all
    }

    function editMacro(edit) {
        const m = Model.copy(macro)
        edit(m)
        setMacro(m)
    }

    function setEvents(events, select) {
        editMacro(m => m.events = events)
        selectedEvent = Math.min(select, events.length - 1)
    }

    function insertEvents(list) {
        const events = Model.copy(macro.events)
        if (events.length + list.length > maxEvents) {
            Model.notify(I18n.text("macros.full").arg(maxEvents))
            return
        }
        const at = selectedEvent >= 0 ? selectedEvent + 1 : events.length
        events.splice(at, 0, ...list)
        setEvents(events, at + list.length - 1)
    }

    function moveEvent(from, to) {
        const events = Model.copy(macro.events)
        if (to < 0 || to >= events.length)
            return
        const [e] = events.splice(from, 1)
        events.splice(to, 0, e)
        setEvents(events, to)
    }

    function createMacro() {
        setMacro({ id: slot, mode: "once", repeat: 1, events: [] })
        selectedEvent = -1
    }

    // Empties the slot and gives keys bound to it their default action back.
    function deleteMacro() {
        const km = Model.copy(keymap)
        for (const b of boundKeys(slot))
            km[b.layer][b.code] = b.layer === "base" ? KeyTables.baseDefault(b.code) : "Default"
        keymap = km
        setMacro(null)
        selectedEvent = -1
    }

    function eventText(e) {
        if (e.key !== undefined)
            return KeyTables.label(e.key) + "  ·  " + e.key
        return I18n.text("macros.mouse_" + e.mouse)
    }

    function startRecording() {
        recorded = []
        recording = true
        recorder.forceActiveFocus()
    }

    function stopRecording(replace) {
        recording = false
        if (recorded.length === 0)
            return
        const r = JSON.parse(KeyTables.macroFromRecording(JSON.stringify(recorded), defaultDelay.value))
        if (r.error) {
            Model.notify(r.error)
            return
        }
        if (r.dropped > 0)
            Model.notify(I18n.text("macros.dropped").arg(r.dropped))
        const events = replace ? r.events : Model.copy(macro.events).concat(r.events)
        setEvents(events.slice(0, maxEvents), events.length - 1)
    }

    function bindToggle(code) {
        const km = Model.copy(keymap)
        const name = "Macro" + slot
        km[bindLayer][code] = km[bindLayer][code] === name ? (bindLayer === "base" ? KeyTables.baseDefault(code) : "Default") : name
        keymap = km
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

        ApplyBar {
            Layout.fillWidth: true
            visible: Model.loaded
            count: page.changedSlots.length + page.changedKeys
            canApply: Model.editable && page.problems.length === 0 && !page.recording
            onDiscard: page.reset()
            onApply: Model.apply(page.sections)
            onPreview: changes.ask(I18n.text("apply.preview_title"), I18n.text("apply.confirm_text"),
                                   Model.preview(page.sections), page.problems.length === 0 ? I18n.text("apply.apply") : "",
                                   () => Model.apply(page.sections))
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
                Layout.maximumWidth: page.availableWidth >= 980 ? 340 : 100000
                Layout.alignment: Qt.AlignTop
                title: I18n.text("macros.slots")
                icon: "list"

                Repeater {
                    model: 10

                    delegate: Rectangle {
                        id: item

                        required property int index
                        readonly property var m: page.macros[index] ?? null
                        readonly property bool active: page.slot === index
                        readonly property bool changed: page.changedSlots.indexOf(index) >= 0
                        readonly property string bound: page.boundText(index)

                        Layout.fillWidth: true
                        implicitHeight: 56
                        radius: 14
                        color: active ? Theme.secondaryContainer
                             : hover.hovered ? Qt.tint(Theme.surfaceContainerHighest, Qt.alpha(Theme.surfaceContent, 0.06))
                             : Theme.surfaceContainerHighest
                        border.width: active ? 2 : 0
                        border.color: Theme.primary

                        RowLayout {
                            anchors.fill: parent
                            anchors.leftMargin: 14
                            anchors.rightMargin: 14
                            spacing: 10

                            Icon {
                                name: item.m ? "keyboard_command_key" : "add"
                                size: 20
                                color: item.active ? Theme.secondaryContainerContent : item.m ? Theme.primary : Theme.outline
                            }

                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 0

                                Label {
                                    Layout.fillWidth: true
                                    text: "M" + item.index + "  ·  " + (item.m ? I18n.text("macros.event_count").arg(item.m.events.length)
                                                                             : I18n.text("common.empty"))
                                    elide: Text.ElideRight
                                    font.weight: Font.Medium
                                    color: item.active ? Theme.secondaryContainerContent : Theme.surfaceContent
                                }

                                Label {
                                    Layout.fillWidth: true
                                    visible: item.bound.length > 0
                                    text: I18n.text("macros.bound_to").arg(item.bound)
                                    elide: Text.ElideRight
                                    font.pixelSize: 12
                                    color: item.active ? Theme.secondaryContainerContent : Theme.surfaceVariantContent
                                }
                            }

                            Rectangle {
                                visible: item.changed
                                implicitWidth: 8
                                implicitHeight: 8
                                radius: 4
                                color: Theme.primary
                            }
                        }

                        HoverHandler {
                            id: hover
                            cursorShape: Qt.PointingHandCursor
                        }

                        TapHandler {
                            enabled: !page.recording
                            onTapped: {
                                page.slot = item.index
                                page.selectedEvent = -1
                            }
                        }
                    }
                }
            }

            ColumnLayout {
                Layout.fillWidth: true
                Layout.preferredWidth: 2
                Layout.alignment: Qt.AlignTop
                spacing: 16

                Card {
                    Layout.fillWidth: true
                    visible: page.macro === null
                    title: I18n.text("macros.slot_title").arg(page.slot)
                    icon: "keyboard_command_key"

                    Label {
                        Layout.fillWidth: true
                        text: I18n.text("macros.slot_empty")
                        wrapMode: Text.Wrap
                        color: Theme.surfaceVariantContent
                    }

                    TonalButton {
                        icon: "add"
                        text: I18n.text("macros.create")
                        enabled: Model.editable
                        onClicked: page.createMacro()
                    }
                }

                Card {
                    Layout.fillWidth: true
                    visible: page.macro !== null
                    enabled: Model.editable
                    title: I18n.text("macros.slot_title").arg(page.slot)
                    icon: "keyboard_command_key"

                    SettingRow {
                        Layout.fillWidth: true
                        title: I18n.text("macros.play_mode")
                        help: I18n.text("macros.mode_help_" + (page.macro?.mode ?? "once").replace("-", "_"))
                    }

                    SegmentedButtons {
                        model: ["once", "repeat", "toggle", "while-held"]
                        current: page.macro?.mode ?? ""
                        labelFor: m => I18n.text("macros.mode_" + m.replace("-", "_"))
                        onActivated: m => page.editMacro(x => {
                            x.mode = m
                            if (m !== "repeat")
                                x.repeat = 1
                        })
                    }

                    SettingRow {
                        Layout.fillWidth: true
                        visible: page.macro?.mode === "repeat"
                        title: I18n.text("macros.repeat")

                        SpinBox {
                            from: 1
                            to: 65535
                            editable: true
                            value: page.macro?.repeat ?? 1
                            Material.accent: Theme.primary
                            onValueModified: page.editMacro(x => x.repeat = value)
                        }
                    }

                    Label {
                        Layout.fillWidth: true
                        visible: page.slotProblem.length > 0
                        text: page.slotProblem
                        wrapMode: Text.Wrap
                        color: Theme.error
                    }

                    // Recorder: keys pressed while it has focus become steps with the real gaps as delays.
                    Rectangle {
                        id: recorder

                        Layout.fillWidth: true
                        implicitHeight: recRow.implicitHeight + 24
                        radius: 14
                        color: page.recording ? Theme.errorContainer : Theme.surfaceContainerHighest
                        border.width: activeFocus ? 2 : 1
                        border.color: page.recording ? Theme.error : Theme.outlineVariant
                        focus: page.recording

                        // QQ.Keys: the AulaGui `Keys` singleton hides the attached type.
                        QQ.Keys.onPressed: event => {
                            event.accepted = true
                            if (!page.recording || event.isAutoRepeat)
                                return
                            const name = KeyTables.usageFromScanCode(event.nativeScanCode)
                            if (name.length > 0)
                                page.recorded = page.recorded.concat([{ name: name, down: true, t: Date.now() }])
                        }
                        QQ.Keys.onReleased: event => {
                            event.accepted = true
                            if (!page.recording || event.isAutoRepeat)
                                return
                            const name = KeyTables.usageFromScanCode(event.nativeScanCode)
                            if (name.length > 0)
                                page.recorded = page.recorded.concat([{ name: name, down: false, t: Date.now() }])
                        }

                        RowLayout {
                            id: recRow
                            anchors.fill: parent
                            anchors.margins: 12
                            spacing: 10

                            Icon {
                                name: page.recording ? "radio_button_checked" : "keyboard"
                                size: 22
                                color: page.recording ? Theme.error : Theme.surfaceVariantContent

                                SequentialAnimation on opacity {
                                    running: page.recording
                                    loops: Animation.Infinite
                                    alwaysRunToEnd: true
                                    NumberAnimation { to: 0.3; duration: 600 }
                                    NumberAnimation { to: 1; duration: 600 }
                                }
                            }

                            Label {
                                Layout.fillWidth: true
                                text: page.recording ? I18n.text("macros.recording").arg(page.recorded.length)
                                                     : I18n.text("macros.record_help")
                                wrapMode: Text.Wrap
                                color: page.recording ? Theme.errorContainerContent : Theme.surfaceVariantContent
                            }

                            TonalButton {
                                visible: !page.recording
                                icon: "fiber_manual_record"
                                text: I18n.text("macros.record")
                                onClicked: page.startRecording()
                            }

                            TonalButton {
                                visible: page.recording
                                icon: "playlist_add"
                                text: I18n.text("macros.stop_append")
                                onClicked: page.stopRecording(false)
                            }

                            TonalButton {
                                visible: page.recording
                                icon: "stop"
                                text: I18n.text("macros.stop_replace")
                                onClicked: page.stopRecording(true)
                            }
                        }

                        TapHandler {
                            onTapped: if (page.recording) recorder.forceActiveFocus()
                        }
                    }

                    Flow {
                        Layout.fillWidth: true
                        spacing: 8
                        enabled: !page.recording

                        TonalButton {
                            icon: "add"
                            text: I18n.text("macros.add_key")
                            onClicked: picker.pick(I18n.text("macros.add_key"), "", "", page.keyGroups, name => {
                                const d = defaultDelay.value
                                page.insertEvents([{ key: name, down: true, delay_ms: d }, { key: name, down: false, delay_ms: d }])
                            })
                        }

                        TonalButton {
                            icon: "mouse"
                            text: I18n.text("macros.add_mouse")
                            onClicked: mouseMenu.open()

                            Menu {
                                id: mouseMenu
                                y: parent.height

                                Repeater {
                                    model: [1, 2, 4]
                                    delegate: MenuItem {
                                        required property int modelData
                                        text: I18n.text("macros.mouse_" + modelData)
                                        onTriggered: {
                                            const d = defaultDelay.value
                                            page.insertEvents([{ mouse: modelData, down: true, delay_ms: d },
                                                               { mouse: modelData, down: false, delay_ms: d }])
                                        }
                                    }
                                }
                            }
                        }

                        TonalButton {
                            icon: "timer"
                            text: I18n.text("macros.set_all_delays")
                            enabled: (page.macro?.events.length ?? 0) > 0
                            onClicked: page.setEvents(page.macro.events.map(e => Object.assign({}, e, { delay_ms: defaultDelay.value })),
                                                      page.selectedEvent)
                        }

                        TonalButton {
                            icon: "clear_all"
                            text: I18n.text("common.clear")
                            enabled: (page.macro?.events.length ?? 0) > 0
                            onClicked: page.setEvents([], -1)
                        }

                        TonalButton {
                            icon: "delete"
                            text: I18n.text("macros.delete")
                            onClicked: confirm.ask(I18n.text("macros.delete"),
                                                   page.boundKeys(page.slot).length > 0
                                                       ? I18n.text("macros.delete_bound").arg(page.boundText(page.slot))
                                                       : I18n.text("macros.delete_confirm").arg(page.slot),
                                                   I18n.text("common.delete"), () => page.deleteMacro(), true)
                        }
                    }

                    SettingRow {
                        Layout.fillWidth: true
                        title: I18n.text("macros.default_delay")
                        help: I18n.text("macros.default_delay_help")

                        SpinBox {
                            id: defaultDelay
                            from: 0
                            to: 65535
                            value: 20
                            editable: true
                            stepSize: 5
                            Material.accent: Theme.primary
                        }
                    }

                    Label {
                        Layout.fillWidth: true
                        text: I18n.text("macros.steps").arg(page.macro?.events.length ?? 0).arg(page.maxEvents)
                              + "  ·  " + I18n.text("macros.total").arg((page.macro?.events ?? []).reduce((a, e) => a + e.delay_ms, 0))
                        font.weight: Font.DemiBold
                        color: Theme.surfaceContent
                    }

                    Label {
                        Layout.fillWidth: true
                        visible: (page.macro?.events.length ?? 0) === 0
                        text: I18n.text("macros.no_steps")
                        wrapMode: Text.Wrap
                        color: Theme.surfaceVariantContent
                    }

                    Repeater {
                        model: page.macro?.events ?? []

                        delegate: Rectangle {
                            id: step

                            required property var modelData
                            required property int index
                            readonly property bool active: page.selectedEvent === index

                            Layout.fillWidth: true
                            implicitHeight: 48
                            radius: 12
                            color: active ? Theme.secondaryContainer : Theme.surfaceContainerHighest

                            TapHandler {
                                onTapped: page.selectedEvent = step.active ? -1 : step.index
                            }

                            RowLayout {
                                anchors.fill: parent
                                anchors.leftMargin: 12
                                anchors.rightMargin: 6
                                spacing: 8

                                Label {
                                    Layout.preferredWidth: 26
                                    text: step.index + 1
                                    font.pixelSize: 12
                                    color: Theme.surfaceVariantContent
                                }

                                Icon {
                                    name: step.modelData.down ? "arrow_downward" : "arrow_upward"
                                    size: 20
                                    color: step.modelData.down ? Theme.primary : Theme.surfaceVariantContent
                                }

                                Label {
                                    Layout.fillWidth: true
                                    text: page.eventText(step.modelData)
                                    elide: Text.ElideRight
                                    font.weight: Font.Medium
                                    color: step.active ? Theme.secondaryContainerContent : Theme.surfaceContent
                                }

                                Icon {
                                    name: "timer"
                                    size: 18
                                    color: Theme.surfaceVariantContent
                                }

                                SpinBox {
                                    implicitWidth: 150
                                    from: 0
                                    to: 65535
                                    editable: true
                                    stepSize: 5
                                    value: step.modelData.delay_ms
                                    Material.accent: Theme.primary
                                    onValueModified: {
                                        const events = Model.copy(page.macro.events)
                                        events[step.index].delay_ms = value
                                        page.setEvents(events, page.selectedEvent)
                                    }
                                }

                                ToolButton {
                                    text: "▲"
                                    enabled: step.index > 0
                                    onClicked: page.moveEvent(step.index, step.index - 1)
                                }

                                ToolButton {
                                    text: "▼"
                                    enabled: step.index < (page.macro?.events.length ?? 0) - 1
                                    onClicked: page.moveEvent(step.index, step.index + 1)
                                }

                                ToolButton {
                                    text: "✕"
                                    onClicked: {
                                        const events = Model.copy(page.macro.events)
                                        events.splice(step.index, 1)
                                        page.setEvents(events, Math.min(page.selectedEvent, events.length - 1))
                                    }
                                }
                            }
                        }
                    }
                }

                Card {
                    Layout.fillWidth: true
                    visible: page.macro !== null
                    enabled: Model.editable && !page.recording
                    title: I18n.text("macros.bind_title")
                    icon: "link"

                    Label {
                        Layout.fillWidth: true
                        text: I18n.text("macros.bind_help").arg(page.slot)
                        wrapMode: Text.Wrap
                        font.pixelSize: 13
                        color: Theme.surfaceVariantContent
                    }

                    SegmentedButtons {
                        model: ["base", "fn"]
                        current: page.bindLayer
                        labelFor: l => I18n.text("macros.layer_" + l)
                        onActivated: l => page.bindLayer = l
                    }

                    KeyboardView {
                        Layout.fillWidth: true
                        Layout.maximumWidth: 900
                        Layout.alignment: Qt.AlignHCenter
                        values: {
                            const v = {}
                            const name = "Macro" + page.slot
                            const km = page.keymap[page.bindLayer] ?? {}
                            for (const k of Model.layout.keys) {
                                const a = km[k.code] ?? "Default"
                                const mine = a === name
                                const fixed = k.code === "KeyFn" || (page.bindLayer === "fn" && Model.fnFixed.indexOf(k.code) >= 0)
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
            }
        }
    }

    readonly property var keyGroups: ["letters", "digits", "modifiers", "editing", "symbols", "function", "numpad", "international"]


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
