pragma Singleton
import QtQuick
import AulaGui

// Display text and icons derived from Device state, shared by the header and the pages.
QtObject {
    readonly property string statusText: Device.sim ? I18n.text("status.sim")
                                       : !Device.present ? I18n.text("status.no_device")
                                       : !Device.connected ? I18n.text("status.no_access")
                                       : !Device.supported ? I18n.text("status.unsupported")
                                       : I18n.text("status.connected")
    readonly property string statusIcon: Device.sim ? "science"
                                       : !Device.present ? "usb_off"
                                       : !Device.connected || !Device.supported ? "error" : "check_circle"
    readonly property string firmwareText: Device.firmware.length > 0 ? "v" + Device.firmware : I18n.text("value.unknown")
    readonly property string pollText: Model.snapshot && Model.snapshot.poll_rate_khz
                                       ? I18n.text("poll.value").arg(Model.snapshot.poll_rate_khz) : ""
}
