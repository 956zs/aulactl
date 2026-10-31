pragma Singleton
import QtQuick
import AulaGui

// The keyboard's settings (the snapshot JSON from Device), the constant tables from Keys, parsed once,
// and the write path: a page edits copies of its snapshot sections and applies them as a whole snapshot.
QtObject {
    readonly property var snapshot: Device.snapshotJson.length > 0 ? JSON.parse(Device.snapshotJson) : null
    readonly property var info: Device.infoJson.length > 0 ? JSON.parse(Device.infoJson) : null
    readonly property var layout: JSON.parse(KeyTables.layoutJson)
    readonly property var actionGroups: JSON.parse(KeyTables.actionsJson)
    readonly property var usages: JSON.parse(KeyTables.usagesJson)
    readonly property var modes: JSON.parse(KeyTables.modesJson)
    readonly property var switches: JSON.parse(KeyTables.switchesJson)
    readonly property var profiles: JSON.parse(Device.profilesJson)
    readonly property var backups: JSON.parse(Device.backupsJson)
    readonly property var live: JSON.parse(Device.liveJson)
    readonly property var calibrated: JSON.parse(Device.calibratedJson)

    readonly property bool loaded: snapshot !== null && info !== null && Device.loaded
    // Input only with settings read from the board and nothing else running, so an edit never queues
    // behind a write that changes what it was based on.
    readonly property bool editable: loaded && Device.connected && !Device.busy && !Device.live && !Device.calibrating

    readonly property var travel: snapshot ? snapshot.device.travel : ({ max: 340, min: 8, unit_num: 1, high_precision: false })
    readonly property real unitMm: travel.unit_num / (travel.high_precision ? 1000 : 100)
    readonly property int maxSensitivity: Math.ceil(travel.max / 2)
    readonly property var fnFixed: info ? info.fnFixed : []

    // A message for the snackbar (Main.qml shows it).
    signal notify(string text)

    function copy(v) {
        return v === undefined ? undefined : JSON.parse(JSON.stringify(v))
    }

    function same(a, b) {
        return JSON.stringify(a) === JSON.stringify(b)
    }

    function mm(raw) {
        return KeyTables.mm(raw, unitMm)
    }

    function keyByCode(code) {
        return layout.keys.find(k => k.code === code) ?? null
    }

    // The current snapshot with the top-level `sections` replaced by a page's edits.
    function withSections(sections) {
        const s = copy(snapshot)
        for (const k in sections)
            s[k] = sections[k]
        return s
    }

    function apply(sections) {
        Device.apply(JSON.stringify(withSections(sections)))
    }

    // `{ changes, tables }` or `{ error }`.
    function preview(sections) {
        return JSON.parse(Device.preview(JSON.stringify(withSections(sections))))
    }

    // Display name of an action: the string table for named functions, else the core name.
    function actionName(name) {
        if (name === undefined || name === null)
            return ""
        const key = "action." + name
        const t = I18n.text(key)
        return t === key ? name : t
    }

    function featureOn(name) {
        return info ? (info.features.find(f => f.name === name)?.on ?? false) : false
    }
}
