//! RustyCorn — logging and visualisation for the ÄKTAprime / ÄKTAprime plus:
//! the live window, the read-only viewer for a saved log and the
//! method-editor-only mode.

mod cli;
mod console;
mod crash;
mod interrupt;
mod logo;
mod ports;
mod runinfo_forms;
mod settings_page;
mod viewer;
mod widgets;
mod workspace;

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use rc_core::AppState;
use rc_core::settings::{self, Settings};
use rc_protocol::Acquisition;

use cli::Command;
use workspace::{Mode, Workspace};

/// Default window size; below the minimum the layout gets too cramped (it
/// still fits a 1366×768 screen).
const WINDOW_SIZE: [f32; 2] = [1440.0, 880.0];
const MIN_WINDOW_SIZE: [f32; 2] = [1100.0, 680.0];

fn main() -> ExitCode {
    init_logging();
    crash::install_panic_hook();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match cli::parse_args(&args) {
        Command::Help => {
            println!("{}", cli::USAGE);
            ExitCode::SUCCESS
        }
        Command::Usage => {
            log::error!("{}", cli::USAGE);
            ExitCode::from(2)
        }
        Command::Replay(path) => run_replay(&path),
        Command::Methods => run_method_editor(),
        Command::Live { serial_override } => run_live(serial_override),
    };
    if code != ExitCode::SUCCESS {
        crash::pause_if_own_console();
    }
    code
}

/// Messages on the console as "[ Info: …" / "[ Warning: …". Other
/// libraries (window system, GPU) only report warnings; `RUST_LOG` overrides.
fn init_logging() {
    let ours = [
        "rustycorn",
        "rc_core",
        "rc_methods",
        "rc_protocol",
        "rc_plotdata",
        "rc_export",
        "rc_theme",
        "rc_panels",
        "rc_runview",
        "rc_methodeditor",
    ];
    let filter = std::iter::once("warn".to_string())
        .chain(ours.iter().map(|c| format!("{c}=info")))
        .collect::<Vec<_>>();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(filter.join(",")))
        .format(|buf, record| {
            let level = match record.level() {
                log::Level::Error => "Error",
                log::Level::Warn => "Warning",
                log::Level::Info => "Info",
                log::Level::Debug => "Debug",
                log::Level::Trace => "Trace",
            };
            let style = buf.default_level_style(record.level()).bold();
            writeln!(buf, "{style}[ {level}:{style:#} {}", record.args())
        })
        .init();
}

fn save(s: &Settings) {
    if let Err(err) = settings::save_settings(s) {
        log::error!(
            "Could not save the settings to {}: {err}",
            settings::settings_path().display()
        );
    }
}

/// Settings for this start (the defaults on the very first start).
fn load_settings() -> Settings {
    settings::load_settings()
}

fn native_options(title: &str) -> eframe::NativeOptions {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(title)
        .with_app_id("rustycorn")
        .with_inner_size(window_size(
            std::env::var("RUSTYCORN_WINDOW_SIZE").ok().as_deref(),
        ))
        .with_min_inner_size(MIN_WINDOW_SIZE);
    if let Some(icon) = logo::icon() {
        viewport = viewport.with_icon(icon);
    }
    eframe::NativeOptions {
        viewport,
        ..Default::default()
    }
}

/// The window size at start: `RUSTYCORN_WINDOW_SIZE="1100x680"` (for
/// screenshots and layout checks; never below the minimum), else the default.
fn window_size(spec: Option<&str>) -> [f32; 2] {
    let parsed = spec.and_then(|s| {
        let (w, h) = s.trim().split_once(['x', 'X'])?;
        Some([w.trim().parse::<f32>().ok()?, h.trim().parse::<f32>().ok()?])
    });
    match parsed {
        Some([w, h]) if w.is_finite() && h.is_finite() => {
            [w.max(MIN_WINDOW_SIZE[0]), h.max(MIN_WINDOW_SIZE[1])]
        }
        _ => WINDOW_SIZE,
    }
}

/// Fonts and colours, before the first frame.
fn setup_context(ctx: &egui::Context, dark: bool) {
    rc_theme::install_fonts(ctx);
    rc_theme::apply(ctx, dark);
    interrupt::set_context(ctx);
}

/// The main window; the live source (demo or instrument) is chosen in Settings.
fn run_live(serial_override: Option<String>) -> ExitCode {
    let mut settings = load_settings();
    if let Some(port) = serial_override {
        settings.serial_port = port;
        save(&settings);
    }
    console_banner(&settings);
    // Ask for the prefix before anything is recorded; the last one is the default.
    settings.log_prefix = console::ask_log_prefix(&settings.log_prefix);
    save(&settings);

    interrupt::install();
    let dark = settings.dark_mode;
    let app = AppState::new().shared();
    let mut acq = Acquisition::new(app.clone(), settings);
    acq.start();

    let result = eframe::run_native(
        rc_core::APP_NAME,
        native_options(rc_core::APP_NAME),
        Box::new(|cc| {
            setup_context(&cc.egui_ctx, dark);
            log::info!("GUI opened");
            Ok(Box::new(Workspace::new(&mut acq, Mode::Live)))
        }),
    );
    // stops the source (what it is still applying gets applied), then closes the files
    acq.stop();
    console::print_report(&app.lock());
    window_result(result)
}

fn console_banner(s: &Settings) {
    println!("{}", console::banner(s));
}

/// Only the method editor: no serial link and no log file, e.g. to prepare
/// methods away from the instrument (`rustycorn --methods`).
fn run_method_editor() -> ExitCode {
    let settings = load_settings();
    let dark = settings.dark_mode;
    let app = AppState::new().shared();
    let dir = settings.methods_dir(); // (creates the data folder, or the chosen one)
    log::info!("Method editor — methods in {}", dir.display());
    rc_methods::install_example_methods_logged(&dir);
    app.lock().method_dir = dir;
    interrupt::install();
    let mut acq = Acquisition::new(app, settings); // never started
    let title = format!("{} — Methods", rc_core::APP_NAME);
    let result = eframe::run_native(
        &title,
        native_options(&title),
        Box::new(|cc| {
            setup_context(&cc.egui_ctx, dark);
            Ok(Box::new(Workspace::new(&mut acq, Mode::MethodsOnly)))
        }),
    );
    window_result(result)
}

/// A read-only window for one CSV log.
fn run_replay(path: &Path) -> ExitCode {
    if !path.is_file() {
        log::error!("File not found: {}", path.display());
        return ExitCode::FAILURE;
    }
    log::info!("Replay mode - loading \"{}\"", path.display());
    let settings = load_settings();
    let mut app = match rc_core::csvlog::load_csv_log(path) {
        Ok(app) => app,
        Err(err) => {
            log::error!("Could not read {}: {err}", path.display());
            return ExitCode::FAILURE;
        }
    };
    app.log_filename = path.to_path_buf();
    app.touch();
    let app = app.shared();

    interrupt::install();
    let title = format!("{} Viewer", rc_core::APP_NAME);
    let result = eframe::run_native(
        &title,
        native_options(&title),
        Box::new(|cc| {
            setup_context(&cc.egui_ctx, settings.dark_mode);
            log::info!("GUI opened (replay)");
            Ok(Box::new(viewer::ViewerApp::new(app)))
        }),
    );
    window_result(result)
}

fn window_result(result: eframe::Result) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            log::error!("Could not open the window: {err}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_size_from_the_environment() {
        assert_eq!(window_size(None), WINDOW_SIZE);
        assert_eq!(window_size(Some("1920x1080")), [1920.0, 1080.0]);
        assert_eq!(window_size(Some(" 1200 X 700 ")), [1200.0, 700.0]);
        assert_eq!(window_size(Some("800x600")), MIN_WINDOW_SIZE);
        assert_eq!(window_size(Some("big")), WINDOW_SIZE);
        assert_eq!(window_size(Some("inf x 900")), WINDOW_SIZE);
    }
}
