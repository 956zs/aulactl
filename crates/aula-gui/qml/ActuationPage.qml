import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts
import AulaGui

// Per-key actuation point, rapid trigger, dead band and switch type. Edits go into drafts of the snapshot's
// `trigger`, `dead_band` and `switch_types` sections; Apply writes only the keys that changed.
ScrollView {
    id: page

    contentWidth: availableWidth
    clip: true

    property var trigger: ({})
    property var deadBand: null
    property var switchTypes: null
    property var selection: []

    readonly property var base: Model.snapshot
    readonly property var sections: ({ trigger: trigger, dead_band: deadBand, switch_types: switchTypes })
    readonly property var changedKeys: {
        const out = []
        if (!base)
            return out
        for (const k of Model.layout.keys) {
            const c = k.code
            if (!Model.same(trigger[c], base.trigger[c])
                    || (deadBand && !Model.same(deadBand[c], base.dead_band?.[c]))
                    || (switchTypes && switchTypes[c] !== base.switch_types?.[c]))
                out.push(c)
        }
        return out
    }
    readonly property string first: selection.length > 0 ? selection[0] : ""
    readonly property var firstTrigger: first.length > 0 ? trigger[first] ?? null : null
    readonly property bool canEdit: Model.editable && selection.length > 0 && firstTrigger !== null

    function reset() {
        trigger = Model.copy(base?.trigger ?? {})
        deadBand = Model.copy(base?.dead_band ?? null)
        switchTypes = Model.copy(base?.switch_types ?? null)
    }

    // Whether `get(code)` differs between the selected keys.
    function mixed(get) {
        if (selection.length < 2)
            return false
        const v = JSON.stringify(get(selection[0]))
        return selection.some(c => JSON.stringify(get(c)) !== v)
    }

    // Applies `edit(entry)` to a copy of the selected keys' entries of `table` and returns the copy.
    function edited(table, edit) {
        const t = Model.copy(table)
        for (const c of selection)
            if (t[c] !== undefined)
                t[c] = edit(t[c])
        return t
    }

    function setTrigger(field, value) {
        trigger = edited(trigger, t => {
            t[field] = value
            if (field === "press" && t.mode === "rapid")
                t.release = value
            return t
        })
    }

    function setMode(mode) {
        trigger = edited(trigger, t => {
            t.mode = mode
            if (mode === "rapid")
                t.release = t.press
            return t
        })
    }

    function modeText(t) {
        if (!t)
            return ""
        switch (t.mode) {
        case "rapid": return "RT " + Model.mm(t.press)
        case "rapid-split": return "RT " + Model.mm(t.press) + "/" + Model.mm(t.release)
        default: return ""
        }
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
            title: I18n.text("actuation.keys")
            icon: "keyboard"

            Label {
                Layout.fillWidth: true
                text: I18n.text("select.hint")
                wrapMode: Text.Wrap
                font.pixelSize: 13
                color: Theme.surfaceVariantContent
            }

            KeyboardView {
                Layout.fillWidth: true
                Layout.maximumWidth: 1000
                Layout.alignment: Qt.AlignHCenter
                selectable: true
                interactive: Model.loaded
                selected: page.selection
                values: {
                    const v = {}
                    for (const k of Model.layout.keys) {
                        const t = page.trigger[k.code]
                        if (!t)
                            continue
                        const changed = page.changedKeys.indexOf(k.code) >= 0
                        v[k.code] = {
                            text: Model.mm(t.actuation),
                            sub: page.modeText(t),
                            fill: t.actuation / Model.travel.max,
                            fillColor: Qt.alpha(t.mode === "fixed" ? Theme.outline : Theme.primary, 0.28),
                            color: changed ? Theme.secondaryContainer : Theme.surfaceContainerHighest
                        }
                    }
                    return v
                }
                onKeyClicked: (code, mods) => page.selection = JSON.parse(KeyTables.click(JSON.stringify(page.selection), code,
                                                                     (mods & (Qt.ShiftModifier | Qt.ControlModifier)) !== 0))
                onRectSelected: (codes, add) => page.selection = JSON.parse(KeyTables.merge(JSON.stringify(page.selection), JSON.stringify(codes), add))
            }

            SelectionBar {
                Layout.fillWidth: true
                selection: page.selection
                onChosen: codes => page.selection = codes
            }
        }

        ApplyBar {
            Layout.fillWidth: true
            visible: Model.loaded
            count: page.changedKeys.length
            countText: I18n.text("actuation.pending").arg(page.changedKeys.length)
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
                Layout.alignment: Qt.AlignTop
                title: I18n.text("actuation.trigger")
                icon: "height"
                enabled: page.canEdit

                Label {
                    Layout.fillWidth: true
                    visible: page.selection.length === 0
                    text: I18n.text("actuation.select_first")
                    wrapMode: Text.Wrap
                    color: Theme.surfaceVariantContent
                }

                SegmentedButtons {
                    model: ["fixed", "rapid", "rapid-split"]
                    current: page.mixed(c => page.trigger[c]?.mode) ? "" : page.firstTrigger?.mode ?? ""
                    labelFor: m => I18n.text("actuation.mode_" + m.replace("-", "_"))
                    onActivated: m => page.setMode(m)
                }

                Label {
                    Layout.fillWidth: true
                    text: I18n.text("actuation.mode_help_" + (page.firstTrigger?.mode ?? "fixed").replace("-", "_"))
                    wrapMode: Text.Wrap
                    font.pixelSize: 13
                    color: Theme.surfaceVariantContent
                }

                ValueSlider {
                    Layout.fillWidth: true
                    title: I18n.text("actuation.point")
                    help: I18n.text("actuation.point_help")
                    from: Model.travel.min
                    to: Model.travel.max
                    value: page.firstTrigger?.actuation ?? Model.travel.min
                    mixed: page.mixed(c => page.trigger[c]?.actuation)
                    format: v => I18n.text("common.mm").arg(Model.mm(v))
                    onMoved: v => page.setTrigger("actuation", Math.round(v))
                }

                ValueSlider {
                    Layout.fillWidth: true
                    visible: (page.firstTrigger?.mode ?? "fixed") !== "fixed"
                    title: I18n.text(page.firstTrigger?.mode === "rapid-split" ? "actuation.press" : "actuation.sensitivity")
                    help: I18n.text(page.firstTrigger?.mode === "rapid-split" ? "actuation.press_help" : "actuation.sensitivity_help")
                    from: 1
                    to: Model.maxSensitivity
                    value: page.firstTrigger?.press ?? 1
                    mixed: page.mixed(c => page.trigger[c]?.press)
                    format: v => I18n.text("common.mm").arg(Model.mm(v))
                    onMoved: v => page.setTrigger("press", Math.round(v))
                }

                ValueSlider {
                    Layout.fillWidth: true
                    visible: page.firstTrigger?.mode === "rapid-split"
                    title: I18n.text("actuation.release")
                    help: I18n.text("actuation.release_help")
                    from: 1
                    to: Model.maxSensitivity
                    value: page.firstTrigger?.release ?? 1
                    mixed: page.mixed(c => page.trigger[c]?.release)
                    format: v => I18n.text("common.mm").arg(Model.mm(v))
                    onMoved: v => page.setTrigger("release", Math.round(v))
                }
            }

            ColumnLayout {
                Layout.fillWidth: true
                Layout.preferredWidth: 1
                Layout.alignment: Qt.AlignTop
                spacing: 16

                Card {
                    Layout.fillWidth: true
                    visible: page.deadBand !== null
                    title: I18n.text("actuation.dead_band")
                    icon: "vertical_align_center"
                    enabled: page.canEdit

                    Label {
                        Layout.fillWidth: true
                        text: I18n.text("actuation.dead_band_help")
                        wrapMode: Text.Wrap
                        font.pixelSize: 13
                        color: Theme.surfaceVariantContent
                    }

                    ValueSlider {
                        Layout.fillWidth: true
                        title: I18n.text("actuation.dead_top")
                        from: 0
                        to: 100
                        value: page.deadBand?.[page.first]?.top ?? 0
                        mixed: page.mixed(c => page.deadBand?.[c]?.top)
                        format: v => I18n.text("common.mm").arg(Model.mm(v))
                        onMoved: v => page.deadBand = page.edited(page.deadBand, d => ({ top: Math.round(v), bottom: d.bottom }))
                    }

                    ValueSlider {
                        Layout.fillWidth: true
                        title: I18n.text("actuation.dead_bottom")
                        from: 0
                        to: 100
                        value: page.deadBand?.[page.first]?.bottom ?? 0
                        mixed: page.mixed(c => page.deadBand?.[c]?.bottom)
                        format: v => I18n.text("common.mm").arg(Model.mm(v))
                        onMoved: v => page.deadBand = page.edited(page.deadBand, d => ({ top: d.top, bottom: Math.round(v) }))
                    }
                }

                Card {
                    Layout.fillWidth: true
                    visible: page.switchTypes !== null
                    title: I18n.text("actuation.switch")
                    icon: "toggle_on"
                    enabled: page.canEdit

                    SettingRow {
                        Layout.fillWidth: true
                        title: I18n.text("actuation.switch_type")
                        help: I18n.text("actuation.switch_help")

                        DropDown {
                            implicitWidth: 220
                            choices: (Model.info?.switches ?? []).map(s => ({ label: s.name, value: s.id }))
                            current: page.mixed(c => page.switchTypes?.[c]) ? -1 : page.switchTypes?.[page.first] ?? -1
                            onChosen: id => page.switchTypes = page.edited(page.switchTypes, () => id)
                        }
                    }
                }

                Card {
                    Layout.fillWidth: true
                    title: I18n.text("actuation.reset_title")
                    icon: "restart_alt"

                    Label {
                        Layout.fillWidth: true
                        text: I18n.text("actuation.reset_help")
                        wrapMode: Text.Wrap
                        font.pixelSize: 13
                        color: Theme.surfaceVariantContent
                    }

                    TonalButton {
                        icon: "restart_alt"
                        text: I18n.text("actuation.reset")
                        enabled: Model.editable
                        onClicked: confirm.ask(I18n.text("actuation.reset_title"), I18n.text("actuation.reset_confirm"),
                                               I18n.text("actuation.reset"), () => Device.resetTriggers(), true)
                    }
                }
            }
        }
    }

    ChangesDialog {
        id: changes
    }

    AppDialog {
        id: confirm
    }
}
