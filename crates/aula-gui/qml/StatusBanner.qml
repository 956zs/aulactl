import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import AulaGui

// Shown while the pages cannot be used: no keyboard, no access, another board, or the first read running.
Rectangle {
    id: banner

    readonly property string mode: !Device.present ? "offline"
                                  : !Device.connected ? "access"
                                  : !Device.supported ? "unsupported"
                                  : !Model.loaded ? "loading" : ""

    visible: mode.length > 0
    implicitHeight: row.implicitHeight + 2 * 16
    radius: 20
    color: Theme.secondaryContainer

    RowLayout {
        id: row
        anchors.fill: parent
        anchors.margins: 16
        anchors.leftMargin: 20
        spacing: 16

        Icon {
            name: banner.mode === "offline" ? "usb_off" : banner.mode === "loading" ? "hourglass_top" : "error"
            size: 28
            color: Theme.secondaryContainerContent

            SequentialAnimation on opacity {
                running: banner.visible && banner.mode === "loading"
                loops: Animation.Infinite
                alwaysRunToEnd: true
                NumberAnimation { to: 0.35; duration: 900; easing.type: Easing.InOutSine }
                NumberAnimation { to: 1; duration: 900; easing.type: Easing.InOutSine }
            }
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 2

            Label {
                Layout.fillWidth: true
                text: I18n.text("banner." + banner.mode)
                wrapMode: Text.Wrap
                font.pixelSize: 15
                font.weight: Font.DemiBold
                color: Theme.secondaryContainerContent
            }

            Label {
                Layout.fillWidth: true
                visible: text.length > 0
                text: banner.mode === "unsupported" ? I18n.text("banner.unsupported_hint").arg(Device.productCode)
                    : banner.mode === "access" && Device.lastError.length > 0 ? Device.lastError
                    : banner.mode === "loading" ? "" : I18n.text("banner." + banner.mode + "_hint")
                wrapMode: Text.Wrap
                font.pixelSize: 13
                color: Theme.secondaryContainerContent
                opacity: 0.85
            }
        }
    }
}
