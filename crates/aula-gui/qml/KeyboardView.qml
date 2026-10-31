import QtQuick
import QtQuick.Controls
import AulaGui

// The board drawn from the core layout table (protocol section 14), scaled to the width.
//
// `values` maps key codes to what a key cap shows: { text, sub, color, textColor, fill, fillColor,
// marker, dim, disabled }; keys without an entry show their legend. A key with its own text shows the
// legend small in the corner. Clicks emit `keyClicked`; with
// `selectable` a drag selects a rectangle (`rectSelected`); with `painting` a drag emits `keyPainted` for
// every key it touches.
Item {
    id: view

    property var values: ({})
    property var selected: []
    property bool selectable: false
    property bool painting: false
    property bool interactive: true
    // Key height in u (keys are 1 u high; this leaves the gap below).
    readonly property real gapU: 3 / 38
    readonly property real unit: width / Model.layout.width
    property string hovered: ""

    signal keyClicked(string code, int modifiers)
    signal rectSelected(var codes, bool add)
    signal keyPainted(string code)

    implicitWidth: 760
    implicitHeight: unit * Model.layout.height

    function codesAt(x0, y0, x1, y1) {
        return JSON.parse(KeyTables.keysInRect(x0 / unit, y0 / unit, x1 / unit, y1 / unit))
    }

    Repeater {
        model: Model.layout.keys

        delegate: Rectangle {
            id: cap

            required property var modelData
            readonly property var v: view.values[modelData.code] ?? ({})
            readonly property bool isSelected: view.selected.indexOf(modelData.code) >= 0
            readonly property bool isHovered: view.hovered === modelData.code

            x: modelData.x * view.unit
            y: modelData.y * view.unit
            width: (modelData.w - view.gapU) * view.unit
            height: (1 - view.gapU) * view.unit
            radius: Math.max(4, view.unit * 0.16)
            color: cap.v.color ?? Theme.surfaceContainerHighest
            border.width: cap.isSelected ? 2.5 : 1
            border.color: cap.isSelected ? Theme.primary
                        : cap.isHovered && view.interactive && !cap.v.disabled ? Theme.outline : Theme.outlineVariant
            opacity: cap.v.disabled ? 0.35 : cap.v.dim ? 0.6 : 1
            clip: true

            Behavior on color {
                enabled: !Device.live
                ColorAnimation { duration: 120 }
            }

            // Depth / value bar from the bottom.
            Rectangle {
                visible: (cap.v.fill ?? 0) > 0
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                height: parent.height * Math.min(1, cap.v.fill ?? 0)
                radius: cap.radius
                color: cap.v.fillColor ?? Qt.alpha(Theme.primary, 0.55)
            }

            // Peak marker: a line at a fraction of the height from the bottom.
            Rectangle {
                visible: (cap.v.marker ?? 0) > 0
                x: 3
                width: parent.width - 6
                height: 2
                y: parent.height * (1 - Math.min(1, cap.v.marker ?? 0)) - 1
                color: Theme.error
            }

            Label {
                visible: cap.v.text !== undefined && view.unit >= 34
                x: 4
                y: 2
                width: parent.width - 8
                text: cap.modelData.label
                elide: Text.ElideRight
                font.pixelSize: Math.max(8, view.unit * 0.17)
                color: Theme.surfaceVariantContent
                opacity: 0.8
            }

            Column {
                anchors.centerIn: parent
                anchors.verticalCenterOffset: cap.v.text !== undefined && view.unit >= 34 ? view.unit * 0.07 : 0
                width: parent.width - 6
                spacing: 0

                Label {
                    width: parent.width
                    horizontalAlignment: Text.AlignHCenter
                    text: cap.v.text ?? cap.modelData.label
                    elide: Text.ElideRight
                    fontSizeMode: Text.HorizontalFit
                    minimumPixelSize: 8
                    font.pixelSize: Math.max(9, view.unit * 0.26)
                    font.weight: cap.isSelected ? Font.DemiBold : Font.Medium
                    color: cap.v.textColor ?? Theme.surfaceContent
                }

                Label {
                    visible: text.length > 0
                    width: parent.width
                    horizontalAlignment: Text.AlignHCenter
                    text: cap.v.sub ?? ""
                    elide: Text.ElideRight
                    fontSizeMode: Text.HorizontalFit
                    minimumPixelSize: 7
                    font.pixelSize: Math.max(8, view.unit * 0.2)
                    color: cap.v.textColor ?? Theme.surfaceVariantContent
                    opacity: 0.9
                }
            }
        }
    }

    // Rubber band while dragging a selection.
    Rectangle {
        id: band
        visible: false
        color: Qt.alpha(Theme.primary, 0.12)
        border.width: 1
        border.color: Theme.primary
        radius: 4
    }

    MouseArea {
        id: mouse

        property point start
        property bool dragging: false

        anchors.fill: parent
        enabled: view.interactive
        hoverEnabled: true
        cursorShape: view.painting ? Qt.CrossCursor : Qt.PointingHandCursor
        acceptedButtons: Qt.LeftButton

        function keyAt(x, y) {
            const codes = view.codesAt(x, y, x, y)
            return codes.length > 0 ? codes[0] : ""
        }

        onPositionChanged: m => {
            view.hovered = keyAt(m.x, m.y)
            if (!pressed)
                return
            if (view.painting) {
                if (view.hovered.length > 0 && view.values[view.hovered]?.disabled !== true)
                    view.keyPainted(view.hovered)
                return
            }
            if (!view.selectable)
                return
            if (!dragging && Math.abs(m.x - start.x) + Math.abs(m.y - start.y) > 6)
                dragging = true
            if (dragging) {
                band.visible = true
                band.x = Math.min(start.x, m.x)
                band.y = Math.min(start.y, m.y)
                band.width = Math.abs(m.x - start.x)
                band.height = Math.abs(m.y - start.y)
            }
        }
        onExited: view.hovered = ""
        onPressed: m => {
            start = Qt.point(m.x, m.y)
            dragging = false
            if (view.painting) {
                const code = keyAt(m.x, m.y)
                if (code.length > 0 && view.values[code]?.disabled !== true)
                    view.keyPainted(code)
            }
        }
        onReleased: m => {
            band.visible = false
            if (view.painting)
                return
            const add = (m.modifiers & (Qt.ShiftModifier | Qt.ControlModifier)) !== 0
            if (dragging) {
                dragging = false
                view.rectSelected(view.codesAt(start.x, start.y, m.x, m.y), add)
                return
            }
            const code = keyAt(m.x, m.y)
            if (code.length > 0 && view.values[code]?.disabled !== true)
                view.keyClicked(code, m.modifiers)
        }
    }
}
