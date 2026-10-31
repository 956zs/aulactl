import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts
import AulaGui

// Picks a key action from the core keycode tables: grouped, searchable, or typed as a combination
// ("Ctrl+Shift+Esc"). `groups` limits which picker groups are offered.
Dialog {
    id: picker

    property string current
    property string defaultAction
    property var groups: []
    property var onPick: null
    property string group: "letters"
    property string customError

    readonly property var offered: Model.actionGroups.filter(g => picker.groups.length === 0 || picker.groups.indexOf(g.id) >= 0)
    readonly property string query: search.text.trim().toLowerCase()
    readonly property var items: {
        if (query.length === 0)
            return offered.find(g => g.id === group)?.items ?? []
        const out = []
        for (const g of offered)
            for (const i of g.items)
                if (i.name.toLowerCase().includes(query) || i.label.toLowerCase().includes(query)
                        || Model.actionName(i.name).toLowerCase().includes(query))
                    out.push(i)
        return out
    }

    // Opens the picker; `callback(name)` gets the chosen action text.
    function pick(title, current, defaultAction, groups, callback) {
        picker.title = title
        picker.current = current ?? ""
        picker.defaultAction = defaultAction ?? ""
        picker.groups = groups ?? []
        picker.onPick = callback
        search.text = ""
        custom.text = ""
        customError = ""
        const holder = offered.find(g => g.items.some(i => i.name === picker.current))
        group = holder ? holder.id : (offered.length > 0 ? offered[Math.min(1, offered.length - 1)].id : "")
        open()
    }

    function choose(name) {
        const cb = onPick
        close()
        if (cb)
            cb(name)
    }

    parent: Overlay.overlay
    anchors.centerIn: parent
    width: Math.min(820, (parent ? parent.width : 900) - 60)
    height: Math.min(640, (parent ? parent.height : 700) - 60)
    modal: true
    Material.background: Theme.surfaceContainerHigh
    Material.foreground: Theme.surfaceContent
    Material.accent: Theme.primary
    Material.roundedScale: Material.ExtraLargeScale
    onOpened: search.forceActiveFocus()

    contentItem: ColumnLayout {
        spacing: 10

        TextField {
            id: search
            Layout.fillWidth: true
            placeholderText: I18n.text("picker.search")
            Material.accent: Theme.primary
        }

        Flow {
            Layout.fillWidth: true
            visible: picker.query.length === 0
            spacing: 6

            Repeater {
                model: picker.offered

                delegate: Rectangle {
                    id: chip

                    required property var modelData
                    readonly property bool active: picker.group === modelData.id

                    implicitWidth: chipText.implicitWidth + 22
                    implicitHeight: 30
                    radius: 15
                    color: active ? Theme.secondaryContainer
                         : chipHover.hovered ? Qt.tint(Theme.surfaceContainerHighest, Qt.alpha(Theme.surfaceContent, 0.08))
                         : Theme.surfaceContainerHighest

                    Label {
                        id: chipText
                        anchors.centerIn: parent
                        text: I18n.text("group." + chip.modelData.id)
                        font.pixelSize: 13
                        font.weight: Font.Medium
                        color: chip.active ? Theme.secondaryContainerContent : Theme.surfaceContent
                    }

                    HoverHandler {
                        id: chipHover
                        cursorShape: Qt.PointingHandCursor
                    }

                    TapHandler {
                        onTapped: picker.group = chip.modelData.id
                    }
                }
            }
        }

        GridView {
            id: grid
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            cellWidth: Math.floor(width / Math.max(1, Math.floor(width / 112)))
            cellHeight: 62
            model: picker.items
            ScrollBar.vertical: ScrollBar {}

            delegate: Item {
                id: cell

                required property var modelData

                width: grid.cellWidth
                height: grid.cellHeight

                Rectangle {
                    anchors.fill: parent
                    anchors.margins: 3
                    radius: 10
                    color: cell.modelData.name === picker.current ? Theme.primaryContainer
                         : cellHover.hovered ? Qt.tint(Theme.surfaceContainerHighest, Qt.alpha(Theme.surfaceContent, 0.08))
                         : Theme.surfaceContainerHighest
                    border.width: cell.modelData.name === picker.current ? 2 : 0
                    border.color: Theme.primary

                    Column {
                        anchors.centerIn: parent
                        width: parent.width - 8

                        Label {
                            width: parent.width
                            horizontalAlignment: Text.AlignHCenter
                            text: cell.modelData.label.length > 0 ? cell.modelData.label : "—"
                            elide: Text.ElideRight
                            font.weight: Font.DemiBold
                            color: Theme.surfaceContent
                        }

                        Label {
                            width: parent.width
                            horizontalAlignment: Text.AlignHCenter
                            text: Model.actionName(cell.modelData.name)
                            elide: Text.ElideRight
                            font.pixelSize: 11
                            color: Theme.surfaceVariantContent
                        }
                    }

                    HoverHandler {
                        id: cellHover
                        cursorShape: Qt.PointingHandCursor
                    }

                    TapHandler {
                        onTapped: picker.choose(cell.modelData.name)
                    }
                }
            }
        }

        RowLayout {
            Layout.fillWidth: true
            visible: picker.groups.length === 0 || picker.groups.indexOf("modifiers") >= 0
            spacing: 8

            TextField {
                id: custom
                Layout.fillWidth: true
                placeholderText: I18n.text("picker.custom")
                Material.accent: Theme.primary
                onTextChanged: picker.customError = ""
                onAccepted: useCustom.clicked()
            }

            TonalButton {
                id: useCustom
                icon: "check"
                text: I18n.text("picker.use")
                enabled: custom.text.trim().length > 0
                onClicked: {
                    const r = JSON.parse(KeyTables.parseAction(custom.text))
                    if (r.error) {
                        picker.customError = r.error
                        return
                    }
                    picker.choose(r.name)
                }
            }
        }

        Label {
            Layout.fillWidth: true
            visible: picker.customError.length > 0
            text: picker.customError
            wrapMode: Text.Wrap
            font.pixelSize: 12
            color: Theme.error
        }
    }

    footer: DialogButtonBox {
        Material.background: "transparent"

        Button {
            visible: picker.defaultAction.length > 0
            text: I18n.text("picker.reset")
            flat: true
            onClicked: picker.choose(picker.defaultAction)
        }

        Button {
            text: I18n.text("dialog.cancel")
            flat: true
            DialogButtonBox.buttonRole: DialogButtonBox.RejectRole
        }
    }
}
