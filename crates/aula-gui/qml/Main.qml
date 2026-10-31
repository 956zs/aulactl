import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts
import AulaGui

ApplicationWindow {
    id: window

    width: 1280
    height: 860
    minimumWidth: 960
    minimumHeight: 620
    visible: true
    title: I18n.text("app.title")
    color: Theme.background
    font.family: Theme.fontFamily
    font.pixelSize: 15

    // Set from the command line by main.rs.
    property int startPage: 0
    property string screenshotPath

    readonly property int livePage: 6

    Material.theme: Theme.dark ? Material.Dark : Material.Light
    Material.accent: Theme.primary
    Material.primary: Theme.primary
    Material.background: Theme.background
    Material.foreground: Theme.surfaceContent

    // Everything visible, as one item so --screenshot can grab it.
    Rectangle {
        id: surface
        anchors.fill: parent
        color: Theme.background

        RowLayout {
            anchors.fill: parent
            anchors.margins: 12
            anchors.leftMargin: 4
            spacing: 4

            NavRail {
                id: rail
                Layout.fillHeight: true
                // Calibration must be finished on its page.
                enabled: !Device.calibrating
                currentIndex: Math.min(window.startPage, 6)
                model: [
                    { label: "nav.overview", icon: "dashboard" },
                    { label: "nav.actuation", icon: "height" },
                    { label: "nav.keymap", icon: "keyboard" },
                    { label: "nav.macros", icon: "keyboard_command_key" },
                    { label: "nav.advanced", icon: "tune" },
                    { label: "nav.lighting", icon: "palette" },
                    { label: "nav.live", icon: "monitoring" }
                ]
                // The travel stream belongs to the Live page.
                onCurrentIndexChanged: if (currentIndex !== window.livePage && Device.live) Device.stopLive()
            }

            Rectangle {
                Layout.fillWidth: true
                Layout.fillHeight: true
                radius: 24
                color: Theme.surfaceContainerLow

                ColumnLayout {
                    anchors.fill: parent
                    anchors.margins: 20
                    spacing: 16

                    HeaderBar {
                        Layout.fillWidth: true
                    }

                    StatusBanner {
                        Layout.fillWidth: true
                    }

                    StackLayout {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        currentIndex: rail.currentIndex

                        OverviewPage {}
                        ActuationPage {}
                        KeymapPage {}
                        MacroPage {}
                        AdvancedPage {}
                        LightingPage {}
                        LivePage {}
                    }
                }
            }
        }

        Snackbar {
            id: snackbar
            anchors.horizontalCenter: parent.horizontalCenter
            anchors.bottom: parent.bottom
            anchors.bottomMargin: 28
            z: 10
        }
    }

    onClosing: {
        if (Device.live)
            Device.stopLive()
        if (Device.calibrating)
            Device.stopCalibration()
    }

    // --screenshot: grab once the settings are loaded (plus a moment to settle), or after 20 s.
    Timer {
        property int ticks: 0

        running: window.screenshotPath.length > 0
        interval: 500
        repeat: true
        onTriggered: {
            ticks += 1
            if (ticks < 4 || (!Model.loaded && ticks < 40))
                return
            stop()
            if (!Model.loaded)
                console.warn("screenshot taken before the settings were read")
            surface.grabToImage(result => {
                if (!result.saveToFile(window.screenshotPath))
                    console.warn("cannot save", window.screenshotPath)
                Qt.quit()
            })
        }
    }

    Connections {
        target: Device
        function onErrorOccurred(message) {
            snackbar.show(I18n.text("error.message").arg(message))
        }
        function onNotice(key, arg) {
            const text = I18n.text(key)
            snackbar.show(text.includes("%1") ? text.arg(arg) : text, "info")
        }
        function onQuitRequested() {
            Qt.quit()
        }
    }

    Connections {
        target: StringTable
        function onFailed(message) {
            snackbar.show(message)
        }
    }

    Connections {
        target: Model
        function onNotify(text) {
            snackbar.show(text, "info")
        }
    }
}
