use cxx_qt_build::{CxxQtBuilder, QmlFile, QmlModule};

const QML_FILES: &[&str] = &[
    "qml/Main.qml",
    "qml/NavRail.qml",
    "qml/HeaderBar.qml",
    "qml/StatusBanner.qml",
    "qml/Card.qml",
    "qml/Icon.qml",
    "qml/InfoRow.qml",
    "qml/SegmentedButtons.qml",
    "qml/TonalButton.qml",
    "qml/Snackbar.qml",
    "qml/SettingRow.qml",
    "qml/ToggleSwitch.qml",
    "qml/DropDown.qml",
    "qml/AppDialog.qml",
    "qml/ChoiceGrid.qml",
    "qml/KeyboardView.qml",
    "qml/ValueSlider.qml",
    "qml/ApplyBar.qml",
    "qml/ChangesDialog.qml",
    "qml/ActionPicker.qml",
    "qml/ColorField.qml",
    "qml/SelectionBar.qml",
    "qml/OverviewPage.qml",
    "qml/ActuationPage.qml",
    "qml/KeymapPage.qml",
    "qml/MacroPage.qml",
    "qml/AdvancedPage.qml",
    "qml/LightingPage.qml",
    "qml/LivePage.qml",
];
const QML_SINGLETONS: &[&str] = &["qml/Format.qml", "qml/I18n.qml", "qml/Model.qml"];

fn main() {
    let files = QML_FILES
        .iter()
        .map(|f| QmlFile::from(*f))
        .chain(QML_SINGLETONS.iter().map(|f| QmlFile::from(*f).singleton(true)));
    let builder = CxxQtBuilder::new_qml_module(QmlModule::new("AulaGui").qml_files(files))
        .qt_module("Quick")
        .files(["src/bridge/device.rs", "src/bridge/i18n.rs", "src/bridge/keys.rs", "src/bridge/sessions.rs", "src/bridge/theme.rs"]);
    // SAFETY: only adds a warning flag. GCC 16 warns about QChar inside the Qt headers.
    let builder = unsafe {
        builder.cc_builder(|cc| {
            cc.flag_if_supported("-Wno-sfinae-incomplete");
        })
    };
    builder.build();
}
