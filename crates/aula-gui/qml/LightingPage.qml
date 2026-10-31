import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts
import AulaGui

// Lighting effect and the two custom per-key colour slots. Edits go into a draft of the snapshot's
// `lighting` section; Apply writes only the tables that changed. Music rhythm is switched at once.
ScrollView {
    id: page

    contentWidth: availableWidth
    clip: true

    property var lighting: null
    property string brush: "#ff0000"
    property bool eyedropper: false
    property var selection: []

    readonly property var base: Model.snapshot ? Model.snapshot.lighting : null
    readonly property var sections: ({ lighting: lighting })
    readonly property var effect: lighting ? lighting.effect : null
    readonly property var modeInfo: effect ? Model.modes.find(m => m.id === effect.mode) ?? null : null
    readonly property var selectable: Model.info ? Model.info.modes.selectable : []
    readonly property int slot: lighting ? lighting.active_custom_slot : 0
    readonly property var colors: lighting && lighting.custom ? lighting.custom[slot] : null
    readonly property bool isCustom: effect !== null && effect.mode === 10
    readonly property int changeCount: {
        if (!base || !lighting)
            return 0
        let n = 0
        if (!Model.same(lighting.effect, base.effect))
            n += 1
        if (lighting.active_custom_slot !== base.active_custom_slot)
            n += 1
        if (lighting.custom && base.custom)
            for (let s = 0; s < 2; s++)
                for (const k of Model.layout.keys)
                    if (lighting.custom[s][k.code] !== base.custom[s][k.code])
                        n += 1
        return n
    }

    function reset() {
        lighting = Model.copy(base ?? null)
        selection = []
    }

    function setEffect(field, value) {
        const l = Model.copy(lighting)
        l.effect[field] = value
        lighting = l
    }

    function setMode(id) {
        const l = Model.copy(lighting)
        l.effect.mode = id
        const info = Model.modes.find(m => m.id === id)
        // Keep the direction valid for the new mode.
        if (info && info.directions.length > 0 && !info.directions.some(d => d.id === l.effect.direction))
            l.effect.direction = info.directions[0].id
        lighting = l
    }

    function setSlot(s) {
        const l = Model.copy(lighting)
        l.active_custom_slot = s
        lighting = l
    }

    function paint(codes, color) {
        if (!colors)
            return
        const l = Model.copy(lighting)
        for (const c of codes)
            l.custom[slot][c] = color
        lighting = l
    }

    // Black or white text on a key colour.
    function textOn(hex) {
        const c = Qt.color(hex)
        return 0.299 * c.r + 0.587 * c.g + 0.114 * c.b > 0.55 ? "#000000" : "#ffffff"
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
            count: page.changeCount
            onDiscard: page.reset()
            onApply: Model.apply(page.sections)
            onPreview: changes.ask(I18n.text("apply.preview_title"), I18n.text("apply.confirm_text"),
                                   Model.preview(page.sections), I18n.text("apply.apply"), () => Model.apply(page.sections))
        }

        GridLayout {
            Layout.fillWidth: true
            visible: Model.loaded && page.lighting !== null
            columns: page.availableWidth >= 980 ? 2 : 1
            columnSpacing: 16
            rowSpacing: 16

            Card {
                Layout.fillWidth: true
                Layout.preferredWidth: 1
                Layout.alignment: Qt.AlignTop
                title: I18n.text("lighting.effect")
                icon: "auto_awesome"
                enabled: Model.editable

                SettingRow {
                    Layout.fillWidth: true
                    title: I18n.text("lighting.on")

                    ToggleSwitch {
                        checked: page.effect?.on ?? false
                        onToggled: on => page.setEffect("on", on)
                    }
                }

                ChoiceGrid {
                    Layout.fillWidth: true
                    columns: 3
                    choices: Model.modes.filter(m => page.selectable.indexOf(m.id) >= 0)
                                        .map(m => ({ label: I18n.text("lighting.mode_" + m.name.replace(/-/g, "_")), value: m.id }))
                    current: page.effect?.mode ?? -1
                    onChosen: id => page.setMode(id)
                }
            }

            Card {
                Layout.fillWidth: true
                Layout.preferredWidth: 1
                Layout.alignment: Qt.AlignTop
                title: I18n.text("lighting.settings")
                icon: "tune"
                enabled: Model.editable

                ValueSlider {
                    Layout.fillWidth: true
                    title: I18n.text("lighting.brightness")
                    from: 0
                    to: Model.info?.modes.maxBrightness ?? 4
                    value: page.effect?.brightness ?? 0
                    onMoved: v => page.setEffect("brightness", Math.round(v))
                }

                ValueSlider {
                    Layout.fillWidth: true
                    visible: page.modeInfo?.speed ?? false
                    title: I18n.text("lighting.speed")
                    from: 0
                    to: Model.info?.modes.maxSpeed ?? 4
                    value: page.effect?.speed ?? 0
                    onMoved: v => page.setEffect("speed", Math.round(v))
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    visible: (page.modeInfo?.directions.length ?? 0) > 0
                    spacing: 6

                    Label {
                        text: I18n.text("lighting.direction")
                        font.weight: Font.Medium
                        color: Theme.surfaceContent
                    }

                    ChoiceGrid {
                        Layout.fillWidth: true
                        columns: 3
                        choices: (page.modeInfo?.directions ?? []).map(d => ({ label: I18n.text("lighting.dir_" + d.name), value: d.id }))
                        current: page.effect?.direction ?? -1
                        onChosen: d => page.setEffect("direction", d)
                    }
                }

                SettingRow {
                    Layout.fillWidth: true
                    visible: (page.modeInfo?.colors ?? "none") !== "none"
                    title: I18n.text("lighting.full_color")
                    help: I18n.text("lighting.full_color_help")

                    ToggleSwitch {
                        checked: page.effect?.full_color ?? false
                        onToggled: on => page.setEffect("full_color", on)
                    }
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    visible: (page.modeInfo?.colors ?? "none") !== "none" && !(page.effect?.full_color ?? false)
                    spacing: 6

                    Label {
                        text: I18n.text("lighting.fg")
                        font.weight: Font.Medium
                        color: Theme.surfaceContent
                    }

                    ColorField {
                        Layout.fillWidth: true
                        color: page.effect?.fg ?? "#ffffff"
                        onPicked: c => page.setEffect("fg", c)
                    }
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    visible: page.modeInfo?.colors === "fg-bg"
                    spacing: 6

                    Label {
                        text: I18n.text("lighting.bg")
                        font.weight: Font.Medium
                        color: Theme.surfaceContent
                    }

                    ColorField {
                        Layout.fillWidth: true
                        showPresets: false
                        color: page.effect?.bg ?? "#000000"
                        onPicked: c => page.setEffect("bg", c)
                    }
                }

                Label {
                    Layout.fillWidth: true
                    visible: page.isCustom
                    text: I18n.text("lighting.custom_hint")
                    wrapMode: Text.Wrap
                    font.pixelSize: 13
                    color: Theme.surfaceVariantContent
                }
            }
        }

        Card {
            Layout.fillWidth: true
            visible: Model.loaded && page.colors !== null
            title: I18n.text("lighting.custom")
            icon: "brush"
            enabled: Model.editable

            Label {
                Layout.fillWidth: true
                text: page.isCustom ? I18n.text("lighting.custom_help") : I18n.text("lighting.custom_inactive")
                wrapMode: Text.Wrap
                font.pixelSize: 13
                color: Theme.surfaceVariantContent
            }

            RowLayout {
                spacing: 16

                Label {
                    text: I18n.text("lighting.slot")
                    font.weight: Font.Medium
                    color: Theme.surfaceContent
                }

                SegmentedButtons {
                    model: [0, 1]
                    current: page.slot
                    labelFor: s => I18n.text("common.slot").arg(s + 1)
                    onActivated: s => page.setSlot(s)
                }
            }

            KeyboardView {
                Layout.fillWidth: true
                Layout.maximumWidth: 1000
                Layout.alignment: Qt.AlignHCenter
                painting: !page.eyedropper
                interactive: Model.editable
                selected: page.selection
                values: {
                    const v = {}
                    if (!page.colors)
                        return v
                    for (const k of Model.layout.keys) {
                        const c = page.colors[k.code] ?? "#000000"
                        v[k.code] = { color: c, textColor: page.textOn(c) }
                    }
                    return v
                }
                onKeyPainted: code => page.paint([code], page.brush)
                onKeyClicked: (code, mods) => {
                    if (page.eyedropper) {
                        page.brush = page.colors[code] ?? page.brush
                        page.eyedropper = false
                    }
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 16

                ColorField {
                    Layout.fillWidth: true
                    color: page.brush
                    onPicked: c => page.brush = c
                }

                TonalButton {
                    icon: "colorize"
                    text: page.eyedropper ? I18n.text("lighting.eyedropper_on") : I18n.text("lighting.eyedropper")
                    onClicked: page.eyedropper = !page.eyedropper
                }
            }

            Label {
                Layout.fillWidth: true
                text: I18n.text("lighting.fill_help")
                wrapMode: Text.Wrap
                font.pixelSize: 13
                color: Theme.surfaceVariantContent
            }

            SelectionBar {
                Layout.fillWidth: true
                selection: page.selection
                onChosen: codes => page.selection = codes
            }

            Flow {
                Layout.fillWidth: true
                spacing: 8

                TonalButton {
                    icon: "format_color_fill"
                    text: I18n.text("lighting.fill_selection")
                    enabled: page.selection.length > 0
                    onClicked: page.paint(page.selection, page.brush)
                }

                TonalButton {
                    icon: "select_all"
                    text: I18n.text("lighting.fill_all")
                    onClicked: page.paint(Model.layout.keys.map(k => k.code), page.brush)
                }

                TonalButton {
                    icon: "format_color_reset"
                    text: I18n.text("lighting.clear")
                    onClicked: page.paint(Model.layout.keys.map(k => k.code), "#000000")
                }
            }
        }

        Card {
            Layout.fillWidth: true
            visible: Model.loaded && (Model.info?.musicRhythm ?? false)
            title: I18n.text("lighting.music")
            icon: "music_note"

            SettingRow {
                Layout.fillWidth: true
                title: I18n.text("lighting.music_mode")
                help: I18n.text("lighting.music_help")

                ToggleSwitch {
                    enabled: Model.editable
                    checked: Model.info?.musicRhythmOn ?? false
                    onToggled: on => Device.setMusicRhythm(on)
                }
            }
        }
    }

    ChangesDialog {
        id: changes
    }
}
