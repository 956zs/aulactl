import QtQuick
// QtQuick's attached `Keys` is shadowed by the AulaGui `Keys` singleton.
import QtQuick as QQ
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts
import AulaGui

// Live travel heatmap (the board's travel stream), a key tester from the host's key events, and any-key
// calibration. The stream never starts by itself and stops when the page is left.
ScrollView {
    id: page

    contentWidth: availableWidth
    clip: true

    // Key tester: codes held now, codes pressed at least once, last events (newest first).
    property var held: []
    property var tested: []
    property var events: []
    property string calibrationMessage

    readonly property bool current: StackLayout.isCurrentItem && visible

    onCurrentChanged: if (!current && Device.live) Device.stopLive()

    // Heat colour for a depth fraction: green (shallow) through yellow to red (bottomed out).
    function rampColor(f) {
        const t = Math.max(0, Math.min(1, f))
        return Qt.hsva((1 - t) * 0.33, 0.7, Theme.dark ? 0.8 : 0.9, 0.8)
    }

    function testerKey(event, down) {
        event.accepted = true
        if (event.isAutoRepeat)
            return
        const code = KeyTables.codeFromScanCode(event.nativeScanCode)
        const name = KeyTables.usageFromScanCode(event.nativeScanCode)
        if (down) {
            if (code.length > 0 && held.indexOf(code) < 0)
                held = held.concat([code])
            if (code.length > 0 && tested.indexOf(code) < 0)
                tested = tested.concat([code])
        } else {
            held = held.filter(c => c !== code)
        }
        const label = name.length > 0 ? name : I18n.text("live.tester_unknown").arg(event.nativeScanCode)
        events = [{ text: (down ? "↓ " : "↑ ") + label + (code.length === 0 && name.length > 0 ? " " + I18n.text("live.tester_offboard") : "") }]
            .concat(events).slice(0, 12)
    }

    Connections {
        target: Device
        function onCalibrationResult(outcome, code) {
            page.calibrationMessage = outcome === "failed" ? I18n.text("live.cal_failed").arg(code)
                                    : I18n.text("live.cal_" + outcome)
        }
        function onCalibratingChanged() {
            if (Device.calibrating)
                page.calibrationMessage = ""
        }
    }

    ColumnLayout {
        width: page.availableWidth
        spacing: 16

        Card {
            Layout.fillWidth: true
            visible: Model.loaded
            title: I18n.text("live.heatmap")
            icon: "monitoring"

            Label {
                Layout.fillWidth: true
                text: I18n.text("live.heatmap_help")
                wrapMode: Text.Wrap
                font.pixelSize: 13
                color: Theme.surfaceVariantContent
            }

            RowLayout {
                spacing: 8

                TonalButton {
                    icon: Device.live ? "stop" : "play_arrow"
                    enabled: Device.live || (Model.editable && page.current)
                    text: Device.live ? I18n.text("live.stop") : I18n.text("live.start")
                    onClicked: Device.live ? Device.stopLive() : Device.startLive()
                }

                TonalButton {
                    icon: "restart_alt"
                    text: I18n.text("live.reset_peaks")
                    enabled: Device.live
                    onClicked: Device.resetLive()
                }

                Item { Layout.fillWidth: true }

                Rectangle {
                    implicitWidth: 120
                    implicitHeight: 12
                    radius: 6
                    gradient: Gradient {
                        orientation: Gradient.Horizontal
                        GradientStop { position: 0; color: page.rampColor(0) }
                        GradientStop { position: 0.5; color: page.rampColor(0.5) }
                        GradientStop { position: 1; color: page.rampColor(1) }
                    }
                }

                Label {
                    text: I18n.text("live.legend").arg(Model.mm(Model.travel.max))
                    font.pixelSize: 12
                    color: Theme.surfaceVariantContent
                }
            }

            KeyboardView {
                Layout.fillWidth: true
                Layout.maximumWidth: 1000
                Layout.alignment: Qt.AlignHCenter
                // Out of the way while calibrating, so the calibration card is in view.
                visible: !Device.calibrating
                interactive: false
                values: {
                    const v = {}
                    const live = Model.live
                    const max = Model.travel.max
                    for (const k of Model.layout.keys) {
                        const s = live[k.code]
                        if (!s)
                            continue
                        v[k.code] = {
                            text: Model.mm(s.d),
                            sub: s.n > 0 ? "×" + s.n : "",
                            fill: s.d / max,
                            fillColor: page.rampColor(s.d / max),
                            marker: s.p / max
                        }
                    }
                    return v
                }
            }
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
                title: I18n.text("live.tester")
                icon: "keyboard"

                Label {
                    Layout.fillWidth: true
                    text: I18n.text("live.tester_help")
                    wrapMode: Text.Wrap
                    font.pixelSize: 13
                    color: Theme.surfaceVariantContent
                }

                Rectangle {
                    id: tester
                    Layout.fillWidth: true
                    implicitHeight: testerColumn.implicitHeight + 24
                    radius: 14
                    color: Theme.surfaceContainerHighest
                    border.width: activeFocus ? 2 : 1
                    border.color: activeFocus ? Theme.primary : Theme.outlineVariant
                    activeFocusOnTab: true

                    QQ.Keys.onPressed: event => page.testerKey(event, true)
                    QQ.Keys.onReleased: event => page.testerKey(event, false)
                    onActiveFocusChanged: if (!activeFocus) page.held = []

                    TapHandler {
                        onTapped: tester.forceActiveFocus()
                    }

                    ColumnLayout {
                        id: testerColumn
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.top: parent.top
                        anchors.margins: 12
                        spacing: 8

                        Label {
                            Layout.fillWidth: true
                            text: tester.activeFocus ? I18n.text("live.tester_active") : I18n.text("live.tester_click")
                            horizontalAlignment: Text.AlignHCenter
                            font.weight: Font.Medium
                            color: tester.activeFocus ? Theme.primary : Theme.surfaceVariantContent
                        }

                        KeyboardView {
                            Layout.fillWidth: true
                            interactive: false
                            values: {
                                const v = {}
                                for (const c of page.tested)
                                    v[c] = { color: Theme.secondaryContainer, textColor: Theme.secondaryContainerContent }
                                for (const c of page.held)
                                    v[c] = { color: Theme.primary, textColor: Theme.primaryContent }
                                return v
                            }
                        }
                    }
                }

                RowLayout {
                    Layout.fillWidth: true

                    Label {
                        Layout.fillWidth: true
                        text: I18n.text("live.tester_count").arg(page.tested.length).arg(Model.layout.keys.length)
                        font.weight: Font.Medium
                        color: Theme.surfaceContent
                    }

                    TonalButton {
                        icon: "delete_sweep"
                        text: I18n.text("common.clear")
                        onClicked: {
                            page.held = []
                            page.tested = []
                            page.events = []
                        }
                    }
                }

                Flow {
                    Layout.fillWidth: true
                    spacing: 6

                    Repeater {
                        model: page.events

                        delegate: Label {
                            required property var modelData
                            text: modelData.text
                            font.family: "monospace"
                            font.pixelSize: 12
                            padding: 4
                            color: Theme.surfaceVariantContent
                            background: Rectangle {
                                radius: 6
                                color: Theme.surfaceContainerHighest
                            }
                        }
                    }
                }
            }

            Card {
                Layout.fillWidth: true
                Layout.preferredWidth: 1
                Layout.alignment: Qt.AlignTop
                visible: Model.info?.calibration ?? false
                title: I18n.text("live.calibration")
                icon: "straighten"

                Label {
                    Layout.fillWidth: true
                    text: I18n.text("live.cal_help")
                    wrapMode: Text.Wrap
                    font.pixelSize: 13
                    color: Theme.surfaceVariantContent
                }

                RowLayout {
                    spacing: 12

                    TonalButton {
                        visible: !Device.calibrating
                        enabled: Model.editable && !Device.live
                        icon: "straighten"
                        text: I18n.text("live.cal_start")
                        onClicked: confirm.ask(I18n.text("live.calibration"), I18n.text("live.cal_confirm"),
                                               I18n.text("live.cal_start"), () => Device.startCalibration(), true)
                    }

                    TonalButton {
                        visible: Device.calibrating
                        icon: "done"
                        text: I18n.text("live.cal_finish")
                        onClicked: Device.stopCalibration()
                    }

                    Label {
                        visible: Device.calibrating
                        text: I18n.text("live.cal_progress").arg(Model.calibrated.length).arg(Model.layout.keys.length)
                        font.weight: Font.DemiBold
                        color: Theme.primary
                    }
                }

                Label {
                    Layout.fillWidth: true
                    visible: Device.live && !Device.calibrating
                    text: I18n.text("live.cal_stop_live")
                    wrapMode: Text.Wrap
                    font.pixelSize: 13
                    color: Theme.surfaceVariantContent
                }

                ProgressBar {
                    Layout.fillWidth: true
                    visible: Device.calibrating
                    from: 0
                    to: Model.layout.keys.length
                    value: Model.calibrated.length
                    Material.accent: Theme.primary
                }

                KeyboardView {
                    Layout.fillWidth: true
                    visible: Device.calibrating || Model.calibrated.length > 0
                    interactive: false
                    values: {
                        const v = {}
                        for (const c of Model.calibrated)
                            v[c] = { color: Theme.primaryContainer, textColor: Theme.primaryContainerContent }
                        return v
                    }
                }

                Label {
                    Layout.fillWidth: true
                    visible: page.calibrationMessage.length > 0
                    text: page.calibrationMessage
                    wrapMode: Text.Wrap
                    font.weight: Font.Medium
                    color: Theme.surfaceContent
                }
            }
        }
    }

    AppDialog {
        id: confirm
    }
}
