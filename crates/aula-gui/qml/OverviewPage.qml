import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts
import AulaGui

// Device status and capabilities, the base layer at a glance, host-side profiles, backup / restore, poll
// rate, win-lock, language and factory reset.
ScrollView {
    id: page

    contentWidth: availableWidth
    clip: true

    readonly property var snap: Model.snapshot
    readonly property var winLockFlags: ["win", "shift_tab", "alt_f4", "alt_tab"]

    function featureKey(name) {
        return "feature." + name.replace(/[^a-z0-9]+/g, "_")
    }

    function setPollRate(khz) {
        confirm.ask(I18n.text("overview.poll_title"), I18n.text("overview.poll_confirm").arg(khz), I18n.text("apply.apply"),
                    () => Model.apply({ poll_rate_khz: khz }))
    }

    function setWinLock(flag, on) {
        const w = Model.copy(page.snap.win_lock)
        w[flag] = on
        Model.apply({ win_lock: w })
    }

    function saveProfile() {
        const name = profileName.text.trim()
        if (Device.profileNameProblem(name).length > 0)
            return
        if (Model.profiles.indexOf(name) >= 0)
            confirm.ask(I18n.text("overview.profile_overwrite_title"), I18n.text("overview.profile_overwrite").arg(name),
                        I18n.text("overview.overwrite"), () => Device.saveProfile(name, true), true)
        else
            Device.saveProfile(name, false)
        profileName.text = ""
    }

    GridLayout {
        width: page.availableWidth
        columns: page.availableWidth >= 980 ? 2 : 1
        columnSpacing: 16
        rowSpacing: 16

        ColumnLayout {
            Layout.fillWidth: true
            Layout.preferredWidth: 1
            Layout.alignment: Qt.AlignTop
            spacing: 16

            Card {
                Layout.fillWidth: true
                title: I18n.text("overview.device")
                icon: "keyboard_keys"

                InfoRow {
                    Layout.fillWidth: true
                    icon: Format.statusIcon
                    iconColor: Device.connected && Device.supported ? Theme.primary : Theme.surfaceVariantContent
                    label: I18n.text("overview.status")
                    value: Format.statusText
                }

                InfoRow {
                    Layout.fillWidth: true
                    visible: Device.productCode.length > 0
                    icon: "badge"
                    label: I18n.text("overview.product_code")
                    value: Device.productCode
                }

                InfoRow {
                    Layout.fillWidth: true
                    visible: Device.firmware.length > 0
                    icon: "memory"
                    label: I18n.text("overview.firmware")
                    value: Device.buildDate.length > 0 ? Format.firmwareText + "  (" + Device.buildDate + ")" : Format.firmwareText
                }

                InfoRow {
                    Layout.fillWidth: true
                    visible: Device.node.length > 0
                    icon: "usb"
                    label: I18n.text("overview.node")
                    value: Device.sim ? I18n.text("overview.node_sim") : Device.node
                }

                InfoRow {
                    Layout.fillWidth: true
                    visible: Model.snapshot !== null
                    icon: "straighten"
                    label: I18n.text("overview.travel")
                    value: I18n.text("overview.travel_value").arg(Model.mm(Model.travel.min)).arg(Model.mm(Model.travel.max))
                }

                Label {
                    visible: Model.info !== null
                    text: I18n.text("overview.features")
                    font.weight: Font.Medium
                    color: Theme.surfaceContent
                }

                Flow {
                    Layout.fillWidth: true
                    visible: Model.info !== null
                    spacing: 6

                    Repeater {
                        model: Model.info?.features ?? []

                        delegate: Rectangle {
                            id: feature

                            required property var modelData

                            implicitWidth: featureRow.implicitWidth + 20
                            implicitHeight: 30
                            radius: 15
                            color: modelData.on ? Theme.secondaryContainer : "transparent"
                            border.width: modelData.on ? 0 : 1
                            border.color: Theme.outlineVariant

                            Row {
                                id: featureRow
                                anchors.centerIn: parent
                                spacing: 4

                                Icon {
                                    anchors.verticalCenter: parent.verticalCenter
                                    name: feature.modelData.on ? "check" : "close"
                                    size: 16
                                    color: feature.modelData.on ? Theme.secondaryContainerContent : Theme.surfaceVariantContent
                                }

                                Label {
                                    anchors.verticalCenter: parent.verticalCenter
                                    text: I18n.text(page.featureKey(feature.modelData.name))
                                    font.pixelSize: 12
                                    color: feature.modelData.on ? Theme.secondaryContainerContent : Theme.surfaceVariantContent
                                }
                            }
                        }
                    }
                }
            }

            Card {
                Layout.fillWidth: true
                visible: Model.loaded
                title: I18n.text("overview.base_layer")
                icon: "keyboard"

                KeyboardView {
                    Layout.fillWidth: true
                    interactive: false
                    values: {
                        const v = {}
                        const base = Model.snapshot?.keymap.base ?? {}
                        for (const k of Model.layout.keys) {
                            const a = base[k.code]
                            if (a === undefined)
                                continue
                            if (a !== KeyTables.baseDefault(k.code) && a !== "Default")
                                v[k.code] = {
                                    text: KeyTables.label(a),
                                    color: Theme.secondaryContainer,
                                    textColor: Theme.secondaryContainerContent
                                }
                        }
                        return v
                    }
                }

                Label {
                    Layout.fillWidth: true
                    text: I18n.text("overview.base_layer_help")
                    wrapMode: Text.Wrap
                    font.pixelSize: 13
                    color: Theme.surfaceVariantContent
                }
            }

            Card {
                Layout.fillWidth: true
                visible: Model.loaded && page.snap.poll_rate_khz !== null
                title: I18n.text("overview.poll_title")
                icon: "speed"
                enabled: Model.editable

                Label {
                    Layout.fillWidth: true
                    text: I18n.text("overview.poll_help")
                    wrapMode: Text.Wrap
                    font.pixelSize: 13
                    color: Theme.surfaceVariantContent
                }

                SegmentedButtons {
                    model: [1, 2, 4, 8]
                    current: page.snap?.poll_rate_khz ?? 0
                    labelFor: v => I18n.text("poll.value").arg(v)
                    onActivated: v => page.setPollRate(v)
                }
            }

            Card {
                Layout.fillWidth: true
                visible: Model.loaded
                title: I18n.text("overview.win_lock")
                icon: "lock"
                enabled: Model.editable

                Label {
                    Layout.fillWidth: true
                    text: I18n.text("overview.win_lock_help")
                    wrapMode: Text.Wrap
                    font.pixelSize: 13
                    color: Theme.surfaceVariantContent
                }

                Repeater {
                    model: page.winLockFlags

                    delegate: SettingRow {
                        id: lockRow

                        required property string modelData

                        Layout.fillWidth: true
                        title: I18n.text("overview.lock_" + modelData)

                        ToggleSwitch {
                            checked: page.snap?.win_lock[lockRow.modelData] ?? false
                            onToggled: on => page.setWinLock(lockRow.modelData, on)
                        }
                    }
                }
            }
        }

        ColumnLayout {
            Layout.fillWidth: true
            Layout.preferredWidth: 1
            Layout.alignment: Qt.AlignTop
            spacing: 16

            Card {
                Layout.fillWidth: true
                title: I18n.text("overview.profiles")
                icon: "bookmarks"

                Label {
                    Layout.fillWidth: true
                    text: I18n.text("overview.profiles_help")
                    wrapMode: Text.Wrap
                    font.pixelSize: 13
                    color: Theme.surfaceVariantContent
                }

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 8

                    TextField {
                        id: profileName
                        Layout.fillWidth: true
                        placeholderText: I18n.text("overview.profile_name")
                        maximumLength: 32
                        Material.accent: Theme.primary
                        onAccepted: page.saveProfile()
                    }

                    TonalButton {
                        icon: "save"
                        text: I18n.text("overview.save_profile")
                        enabled: Model.editable && profileName.text.trim().length > 0
                                 && Device.profileNameProblem(profileName.text.trim()).length === 0
                        onClicked: page.saveProfile()
                    }
                }

                Label {
                    Layout.fillWidth: true
                    readonly property string problem: profileName.text.trim().length > 0 ? Device.profileNameProblem(profileName.text.trim()) : ""
                    visible: problem.length > 0
                    text: problem
                    wrapMode: Text.Wrap
                    font.pixelSize: 12
                    color: Theme.error
                }

                Label {
                    Layout.fillWidth: true
                    visible: Model.profiles.length === 0
                    text: I18n.text("overview.no_profiles")
                    color: Theme.surfaceVariantContent
                }

                Repeater {
                    model: Model.profiles

                    delegate: Rectangle {
                        id: profile

                        required property string modelData

                        Layout.fillWidth: true
                        implicitHeight: 52
                        radius: 14
                        color: Theme.surfaceContainerHighest

                        RowLayout {
                            anchors.fill: parent
                            anchors.leftMargin: 14
                            anchors.rightMargin: 8
                            spacing: 8

                            Icon {
                                name: "bookmark"
                                size: 20
                                color: Theme.primary
                            }

                            Label {
                                Layout.fillWidth: true
                                text: profile.modelData
                                elide: Text.ElideRight
                                font.weight: Font.Medium
                                color: Theme.surfaceContent
                            }

                            TonalButton {
                                icon: "upload"
                                text: I18n.text("overview.apply_profile")
                                enabled: Model.editable
                                onClicked: changes.ask(I18n.text("overview.apply_profile_title").arg(profile.modelData),
                                                       I18n.text("overview.apply_profile_text"),
                                                       JSON.parse(Device.profilePreview(profile.modelData)),
                                                       I18n.text("apply.apply"), () => Device.applyProfile(profile.modelData))
                            }

                            ToolButton {
                                implicitWidth: 40
                                onClicked: confirm.ask(I18n.text("overview.delete_profile_title"),
                                                       I18n.text("overview.delete_profile").arg(profile.modelData),
                                                       I18n.text("common.delete"), () => Device.deleteProfile(profile.modelData), true)

                                contentItem: Icon {
                                    name: "delete"
                                    size: 20
                                    color: Theme.surfaceVariantContent
                                }
                            }
                        }
                    }
                }
            }

            Card {
                Layout.fillWidth: true
                title: I18n.text("overview.backups")
                icon: "history"

                Label {
                    Layout.fillWidth: true
                    text: I18n.text("overview.backups_help")
                    wrapMode: Text.Wrap
                    font.pixelSize: 13
                    color: Theme.surfaceVariantContent
                }

                TonalButton {
                    icon: "backup"
                    text: I18n.text("overview.backup_now")
                    enabled: Model.editable
                    onClicked: Device.backupNow()
                }

                Label {
                    Layout.fillWidth: true
                    visible: Model.backups.length === 0
                    text: I18n.text("overview.no_backups")
                    color: Theme.surfaceVariantContent
                }

                Repeater {
                    model: Model.backups.slice(0, 8)

                    delegate: Rectangle {
                        id: backup

                        required property var modelData

                        Layout.fillWidth: true
                        implicitHeight: 48
                        radius: 14
                        color: Theme.surfaceContainerHighest

                        RowLayout {
                            anchors.fill: parent
                            anchors.leftMargin: 14
                            anchors.rightMargin: 8
                            spacing: 8

                            Icon {
                                name: backup.modelData.auto ? "schedule" : "backup"
                                size: 20
                                color: Theme.surfaceVariantContent
                            }

                            Label {
                                Layout.fillWidth: true
                                text: backup.modelData.name
                                elide: Text.ElideMiddle
                                font.family: "monospace"
                                font.pixelSize: 13
                                color: Theme.surfaceContent
                            }

                            Label {
                                text: I18n.text(backup.modelData.auto ? "overview.backup_auto" : "overview.backup_manual")
                                font.pixelSize: 12
                                color: Theme.surfaceVariantContent
                            }

                            TonalButton {
                                icon: "settings_backup_restore"
                                text: I18n.text("overview.restore")
                                enabled: Model.editable
                                onClicked: {
                                    const p = backup.modelData.path
                                    changes.danger = true
                                    changes.ask(I18n.text("overview.restore_title"), I18n.text("overview.restore_text").arg(backup.modelData.name),
                                                JSON.parse(Device.restorePreview(p)), I18n.text("overview.restore"), () => Device.restore(p))
                                }
                            }
                        }
                    }
                }
            }

            Card {
                Layout.fillWidth: true
                title: I18n.text("language.title")
                icon: "translate"

                ChoiceGrid {
                    Layout.fillWidth: true
                    equalWidths: false
                    columns: 3
                    choices: ["auto", "zh_TW", "en"].map(l => ({ label: I18n.text("language." + l), value: l }))
                    current: StringTable.setting
                    onChosen: l => StringTable.setLanguage(l)
                }
            }

            Card {
                Layout.fillWidth: true
                visible: Device.connected && Device.supported
                title: I18n.text("overview.factory_title")
                icon: "restart_alt"

                Label {
                    Layout.fillWidth: true
                    text: I18n.text("overview.factory_help")
                    wrapMode: Text.Wrap
                    font.pixelSize: 13
                    color: Theme.surfaceVariantContent
                }

                TonalButton {
                    icon: "restart_alt"
                    text: I18n.text("overview.factory_reset")
                    enabled: Model.editable
                    onClicked: confirm.ask(I18n.text("overview.factory_title"), I18n.text("overview.factory_confirm"),
                                           I18n.text("overview.factory_reset"), () => Device.factoryReset(), true)
                }
            }
        }
    }

    ChangesDialog {
        id: changes
        onClosed: danger = false
    }

    AppDialog {
        id: confirm
    }
}
