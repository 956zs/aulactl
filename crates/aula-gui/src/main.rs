//! aula-gui: settings app for the AULA WIN 60 HE keyboard.

mod actions;
mod bridge;
mod config;
mod evdev;
mod files;
mod format;
mod geometry;
mod link;
mod live;
mod macro_edit;
mod palette;
mod strings;
mod worker;

use cxx_qt_lib::{
    QGuiApplication, QMap, QMapPair_QString_QVariant, QQmlApplicationEngine, QQuickStyle, QString, QUrl, QVariant,
};
use std::process::ExitCode;
use std::sync::atomic::Ordering;
use std::time::Duration;

/// Main.qml in the `AulaGui` QML module registered by build.rs.
const MAIN_QML: &str = "qrc:/qt/qml/AulaGui/qml/Main.qml";
/// Base Qt Quick Controls style; the QML recolours it from `Theme`.
const STYLE: &str = "Material";
/// Matches data/aula-gui.desktop; also the Wayland app id.
const APP_ID: &str = "aula-gui";
/// `AULA_GUI_SIM=1` runs against the simulator like `--sim`.
const SIM_ENV: &str = "AULA_GUI_SIM";
/// How long closing waits for the worker to close a travel stream or end calibration.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(3);
/// After a signal: how long the window gets to quit through the worker before the process exits anyway
/// (it never does when the window failed to load).
const SIGNAL_GRACE: Duration = Duration::from_secs(5);

/// Development options. Everything else on the command line is left to Qt.
#[derive(Debug, Default, PartialEq)]
struct Options {
    /// `--sim`: use the built-in simulator of the board instead of the hidraw device.
    sim: bool,
    /// `--screenshot PATH`: save the window as an image once the settings are loaded (at most 20 s after
    /// start), then quit.
    screenshot: Option<String>,
    /// `--page N`: start on navigation entry N.
    page: i32,
}

fn parse_options(args: &[String], env_sim: Option<&str>) -> Result<Options, String> {
    let mut opts = Options { sim: matches!(env_sim, Some(v) if !v.is_empty() && v != "0"), ..Options::default() };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--sim" => opts.sim = true,
            "--screenshot" => {
                opts.screenshot = Some(it.next().filter(|p| !p.is_empty()).ok_or("--screenshot needs a path")?.clone())
            }
            "--page" => {
                let n = it.next().ok_or("--page needs a number")?;
                opts.page = n.parse().ok().filter(|n| *n >= 0).ok_or_else(|| format!("--page: '{n}' is not a page number"))?;
            }
            _ => {}
        }
    }
    Ok(opts)
}

extern "C" fn on_signal(_: libc::c_int) {
    // Only an atomic store: async-signal-safe. The worker closes streams and asks the window to quit.
    worker::SIGNALLED.store(true, Ordering::Relaxed);
}

fn install_signal_handlers() {
    let handler = on_signal as extern "C" fn(libc::c_int);
    for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        // SAFETY: the handler only performs an atomic store.
        unsafe { libc::signal(sig, handler as libc::sighandler_t) };
    }
    // Normally the worker sees the flag, closes the stream and the window quits. Without a window (or a
    // worker) nothing would: exit after a grace period.
    let watcher = std::thread::Builder::new().name("aula-signals".into()).spawn(|| loop {
        std::thread::sleep(Duration::from_millis(100));
        if worker::SIGNALLED.load(Ordering::Relaxed) {
            std::thread::sleep(SIGNAL_GRACE);
            worker::shutdown(SHUTDOWN_WAIT);
            std::process::exit(1);
        }
    });
    if let Err(e) = watcher {
        eprintln!("aula-gui: cannot watch for signals: {e}");
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opts = match parse_options(&args, std::env::var(SIM_ENV).ok().as_deref()) {
        Ok(opts) => opts,
        Err(e) => {
            eprintln!("aula-gui: {e}");
            return ExitCode::FAILURE;
        }
    };
    let _ = bridge::device::SIM.set(opts.sim);
    install_signal_handlers();

    QQuickStyle::set_style(&QString::from(STYLE));
    let mut app = QGuiApplication::new();
    let mut engine = QQmlApplicationEngine::new();
    let (Some(mut app), Some(mut engine)) = (app.as_mut(), engine.as_mut()) else {
        eprintln!("aula-gui: cannot create the Qt application");
        return ExitCode::FAILURE;
    };
    app.as_mut().set_application_name(&QString::from(APP_ID));
    QGuiApplication::set_desktop_file_name(&QString::from(APP_ID));

    let mut initial = QMap::<QMapPair_QString_QVariant>::default();
    initial.insert(QString::from("startPage"), QVariant::from(&opts.page));
    if let Some(path) = &opts.screenshot {
        initial.insert(QString::from("screenshotPath"), QVariant::from(&QString::from(path)));
    }
    engine.as_mut().set_initial_properties(&initial);
    engine
        .as_mut()
        .on_object_creation_failed(|_, url| {
            eprintln!("aula-gui: cannot load {url}");
            std::process::exit(1);
        })
        .release();
    engine.as_mut().load(&QUrl::from(MAIN_QML));

    let code = app.exec();
    // The window is gone: close the travel stream / end calibration before the process exits.
    worker::shutdown(SHUTDOWN_WAIT);
    if opts.sim {
        // The simulator's profiles and backups only live for this run.
        files::remove_sim_dir();
    }
    match code {
        0 => ExitCode::SUCCESS,
        _ => ExitCode::FAILURE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Options, String> {
        parse_options(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>(), None)
    }

    #[test]
    fn no_options() {
        assert_eq!(parse(&[]), Ok(Options::default()));
    }

    #[test]
    fn sim_screenshot_and_page() {
        let o = parse(&["--sim", "--page", "3", "--screenshot", "/tmp/s.png"]).unwrap();
        assert_eq!(o, Options { sim: true, screenshot: Some("/tmp/s.png".into()), page: 3 });
    }

    #[test]
    fn sim_from_the_environment() {
        let none: [String; 0] = [];
        assert!(parse_options(&none, Some("1")).unwrap().sim);
        assert!(!parse_options(&none, Some("0")).unwrap().sim);
        assert!(!parse_options(&none, Some("")).unwrap().sim);
        assert!(!parse_options(&none, None).unwrap().sim);
    }

    #[test]
    fn qt_arguments_are_ignored() {
        assert_eq!(parse(&["-platform", "offscreen", "--page", "0"]), Ok(Options::default()));
    }

    #[test]
    fn missing_or_bad_values() {
        assert!(parse(&["--screenshot"]).is_err());
        assert!(parse(&["--screenshot", ""]).is_err());
        assert!(parse(&["--page"]).is_err());
        assert!(parse(&["--page", "x"]).is_err());
        assert!(parse(&["--page", "-1"]).is_err());
    }
}
