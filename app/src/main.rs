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
use rc_core::settings::{self, GpuBackend, Settings};
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
    let args = cli::parse_args(&args);
    let gpu = args.gpu_backend;
    let code = match args.command {
        Command::Help => {
            println!("{}", cli::USAGE);
            ExitCode::SUCCESS
        }
        Command::Usage => {
            log::error!("{}", cli::USAGE);
            ExitCode::from(2)
        }
        Command::Replay(path) => run_replay(&path, gpu),
        Command::Methods => run_method_editor(gpu),
        Command::Live { serial_override } => run_live(serial_override, gpu),
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

/// Settings for this start (the defaults on the very first start). The first
/// time the settings are kept in the program folder, those of an earlier start
/// are copied there; a backend given with `--gpu` replaces the saved one (and
/// is saved).
fn load_settings(gpu: Option<GpuBackend>) -> Settings {
    settings::adopt_user_settings();
    let mut s = settings::load_settings();
    if let Some(gpu) = gpu.filter(|&b| b != s.gpu_backend) {
        s.gpu_backend = gpu;
        save(&s);
    }
    s
}

fn native_options(title: &str, gpu: GpuBackend) -> eframe::NativeOptions {
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
    let mut options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    if let Some(backends) = wgpu_backends(gpu)
        && let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut options.wgpu_options.wgpu_setup
    {
        setup.instance_descriptor.backends = backends;
    }
    options
}

/// The wgpu backends to try for `gpu`; None: wgpu's default (every one, or
/// those in `WGPU_BACKEND`).
fn wgpu_backends(gpu: GpuBackend) -> Option<eframe::wgpu::Backends> {
    use eframe::wgpu::Backends;
    match gpu {
        GpuBackend::Auto => None,
        GpuBackend::Vulkan => Some(Backends::VULKAN),
        GpuBackend::Dx12 => Some(Backends::DX12),
        GpuBackend::Metal => Some(Backends::METAL),
        GpuBackend::Gl => Some(Backends::GL),
    }
}

/// Which graphics adapter and interface the window got (for bug reports).
fn log_renderer(cc: &eframe::CreationContext) {
    if let Some(rs) = &cc.wgpu_render_state {
        let info = rs.adapter.get_info();
        log::info!("Graphics: {} via {}", info.name, info.backend);
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
fn setup_context(cc: &eframe::CreationContext, dark: bool) {
    log_renderer(cc);
    let ctx = &cc.egui_ctx;
    rc_theme::install_fonts(ctx);
    rc_theme::apply(ctx, dark);
    interrupt::set_context(ctx);
}

/// The main window; the live source (demo or instrument) is chosen in Settings.
fn run_live(serial_override: Option<String>, gpu: Option<GpuBackend>) -> ExitCode {
    let mut settings = load_settings(gpu);
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
    let gpu = settings.gpu_backend;
    let app = AppState::new().shared();
    let mut acq = Acquisition::new(app.clone(), settings);
    acq.start();

    let result = eframe::run_native(
        rc_core::APP_NAME,
        native_options(rc_core::APP_NAME, gpu),
        Box::new(|cc| {
            setup_context(cc, dark);
            log::info!("GUI opened");
            Ok(Box::new(Workspace::new(&mut acq, Mode::Live)))
        }),
    );
    // stops the source (what it is still applying gets applied), then closes the files
    acq.stop();
    console::print_report(&app.lock());
    window_result(result, gpu)
}

fn console_banner(s: &Settings) {
    println!("{}", console::banner(s));
}

/// Only the method editor: no serial link and no log file, e.g. to prepare
/// methods away from the instrument (`rustycorn --methods`).
fn run_method_editor(gpu: Option<GpuBackend>) -> ExitCode {
    let settings = load_settings(gpu);
    let dark = settings.dark_mode;
    let gpu = settings.gpu_backend;
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
        native_options(&title, gpu),
        Box::new(|cc| {
            setup_context(cc, dark);
            Ok(Box::new(Workspace::new(&mut acq, Mode::MethodsOnly)))
        }),
    );
    window_result(result, gpu)
}

/// A read-only window for one CSV log.
fn run_replay(path: &Path, gpu: Option<GpuBackend>) -> ExitCode {
    if !path.is_file() {
        log::error!("File not found: {}", path.display());
        return ExitCode::FAILURE;
    }
    log::info!("Replay mode - loading \"{}\"", path.display());
    let settings = load_settings(gpu);
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
        native_options(&title, settings.gpu_backend),
        Box::new(|cc| {
            setup_context(cc, settings.dark_mode);
            log::info!("GUI opened (replay)");
            Ok(Box::new(viewer::ViewerApp::new(app)))
        }),
    );
    window_result(result, settings.gpu_backend)
}

fn window_result(result: eframe::Result, gpu: GpuBackend) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            log::error!("Could not open the window: {err}");
            let others: Vec<&str> = GpuBackend::choices()
                .iter()
                .filter(|&&b| b != gpu)
                .map(|b| b.name())
                .collect();
            log::error!(
                "The graphics backend \"{}\" did not work; try another one: rustycorn --gpu <{}>",
                gpu.name(),
                others.join("|")
            );
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
