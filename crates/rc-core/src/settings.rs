//! User settings, stored as TOML in `settings.toml` in the settings folder
//! ([`config_dir`]): on Windows the folder of the program file, so one folder
//! holds RustyCorn with its settings and (by default) its data; elsewhere the
//! user config folder (`<config>/RustyCorn`), unless a `settings.toml` lies
//! next to the program. The folder can be overridden with the
//! `RUSTYCORN_CONFIG_DIR` environment variable (or, in tests,
//! [`set_config_dir_override`]), which every test must use so the user's real
//! settings are never touched.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

/// switchable plot panels (UV is always shown)
pub const ALL_PANELS: [&str; 5] = ["%B", "Cond", "pH", "Pressure", "Flow"];
/// Display refresh choices (Hz) and their labels.
pub const REFRESH_HZ: [f64; 4] = [1.0, 2.0, 4.0, 10.0];
pub const REFRESH_OPTIONS: [&str; 4] = [
    "1× per second",
    "2× per second",
    "4× per second",
    "10× per second",
];
pub const VOLUME_UNIT_OPTIONS: [(f64, &str); 2] = [
    (1.0, "1 µL per count (ÄKTAprime)"),
    (10.0, "10 µL per count (ÄKTAprime plus)"),
];

/// Environment variable that overrides the settings folder (tests, portable use).
pub const CONFIG_DIR_ENV: &str = "RUSTYCORN_CONFIG_DIR";

/// Name of the settings file in the settings folder.
pub const SETTINGS_FILE: &str = "settings.toml";

/// The graphics interface the window is drawn with. `Auto` lets wgpu try every
/// one the system offers; a driver that crashes while it is being tried takes
/// the program down with it (no error, the window never opens), so the others
/// use only that one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GpuBackend {
    Auto,
    Vulkan,
    Dx12,
    Metal,
    Gl,
}

impl GpuBackend {
    pub const ALL: [GpuBackend; 5] = [
        GpuBackend::Auto,
        GpuBackend::Vulkan,
        GpuBackend::Dx12,
        GpuBackend::Metal,
        GpuBackend::Gl,
    ];

    /// The name in settings.toml and on the command line.
    pub fn name(self) -> &'static str {
        match self {
            GpuBackend::Auto => "auto",
            GpuBackend::Vulkan => "vulkan",
            GpuBackend::Dx12 => "dx12",
            GpuBackend::Metal => "metal",
            GpuBackend::Gl => "gl",
        }
    }

    /// The name shown on the Settings page.
    pub fn label(self) -> &'static str {
        match self {
            GpuBackend::Auto => "Automatic",
            GpuBackend::Vulkan => "Vulkan",
            GpuBackend::Dx12 => "DirectX 12",
            GpuBackend::Metal => "Metal",
            GpuBackend::Gl => "OpenGL",
        }
    }

    /// From its name, in any case; "opengl" is taken for "gl".
    pub fn parse(text: &str) -> Option<GpuBackend> {
        let text = text.trim().to_lowercase();
        if text == "opengl" {
            return Some(GpuBackend::Gl);
        }
        GpuBackend::ALL.into_iter().find(|b| b.name() == text)
    }

    /// OpenGL on Windows, where probing every interface crashed in some
    /// graphics drivers; elsewhere wgpu's own choice.
    pub fn platform_default() -> GpuBackend {
        if cfg!(target_os = "windows") {
            GpuBackend::Gl
        } else {
            GpuBackend::Auto
        }
    }

    /// The choices that exist on this system (for the Settings page).
    pub fn choices() -> &'static [GpuBackend] {
        if cfg!(target_os = "windows") {
            &[
                GpuBackend::Auto,
                GpuBackend::Gl,
                GpuBackend::Dx12,
                GpuBackend::Vulkan,
            ]
        } else if cfg!(target_os = "macos") {
            &[GpuBackend::Auto, GpuBackend::Metal]
        } else {
            &[GpuBackend::Auto, GpuBackend::Gl, GpuBackend::Vulkan]
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// simulated data instead of the instrument
    pub demo_mode: bool,
    pub serial_port: String,
    /// µL per volume count
    pub volume_unit_ul: f64,
    /// CSV files are named <prefix>_<date>_<time>.csv
    pub log_prefix: String,
    /// folder for log files and plots; "" = [`default_data_dir`]
    pub data_dir: String,
    /// folder of the methods stored on the PC (PrimeMeth_NNN.txt, names.txt,
    /// run setups); "" = `<data folder>/methods`.
    pub methods_dir: String,
    /// dark colour theme
    pub dark_mode: bool,
    /// live GUI refreshes per second
    pub refresh_hz: f64,
    /// user name asked for at the last run start (prefills the next one).
    /// Column and buffer names are kept with each method, not here.
    pub last_user: String,
    /// plot panels switched on when a page opens
    pub start_panels: Vec<String>,
    /// also save every instrument message (_raw.txt) and method dumps (.met)
    pub save_raw: bool,
    /// graphics interface of the window; applies at the next start
    pub gpu_backend: GpuBackend,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            demo_mode: false,
            serial_port: default_serial_port(),
            volume_unit_ul: crate::DEFAULT_VOLUME_UNIT_UL,
            log_prefix: "akta".to_string(),
            data_dir: String::new(),
            methods_dir: String::new(),
            dark_mode: false,
            refresh_hz: 2.0,
            last_user: String::new(),
            start_panels: ALL_PANELS.iter().map(|s| s.to_string()).collect(),
            save_raw: false,
            gpu_backend: GpuBackend::platform_default(),
        }
    }
}

/// Platform default serial port (Linux /dev/ttyUSB0, Windows COM1, macOS /dev/tty.usbserial-110).
pub fn default_serial_port() -> String {
    if cfg!(target_os = "windows") {
        "COM1"
    } else if cfg!(target_os = "macos") {
        "/dev/tty.usbserial-110"
    } else {
        "/dev/ttyUSB0"
    }
    .to_string()
}

static CONFIG_DIR_OVERRIDE: RwLock<Option<PathBuf>> = RwLock::new(None);

/// What was wrong with the settings file at the last load, for the GUI notice.
static LOAD_PROBLEM: RwLock<Option<String>> = RwLock::new(None);

/// The problem the last load found in the settings file (and clears it): the
/// file could not be read, was not valid TOML, or had invalid values. The file
/// was then copied aside, so saving the defaults never loses what was in it.
pub fn take_load_problem() -> Option<String> {
    LOAD_PROBLEM.write().take()
}

/// Use `dir` as the settings folder for this process (None: back to the
/// default). Takes precedence over `RUSTYCORN_CONFIG_DIR`; meant for tests,
/// which cannot set environment variables without `unsafe` in Rust 2024.
pub fn set_config_dir_override(dir: Option<PathBuf>) {
    *CONFIG_DIR_OVERRIDE.write() = dir;
}

/// The settings folder: `$RUSTYCORN_CONFIG_DIR`, else the program's folder
/// ([`portable_dir`]), else `<config>/RustyCorn`.
pub fn config_dir() -> PathBuf {
    if let Some(dir) = CONFIG_DIR_OVERRIDE.read().clone() {
        return dir;
    }
    if let Some(dir) = std::env::var_os(CONFIG_DIR_ENV).filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    portable_dir().unwrap_or_else(user_config_dir)
}

/// `<config>/RustyCorn`: `%APPDATA%\RustyCorn`, `~/.config/RustyCorn`, …
fn user_config_dir() -> PathBuf {
    dirs::config_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(crate::APP_NAME)
}

/// The folder of the program file when RustyCorn keeps its settings (and by
/// default its data) there: on Windows always, so the unpacked folder holds
/// everything; elsewhere only when a settings.toml lies next to the program
/// (an installed program must not write into `~/.local/bin`). None when that
/// folder cannot be written to.
pub fn portable_dir() -> Option<PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let exe = std::env::current_exe().ok()?;
        portable_dir_for(exe.parent()?, cfg!(target_os = "windows"))
    })
    .clone()
}

/// [`portable_dir`] for a program in `dir`; `always`: also without a settings file there.
fn portable_dir_for(dir: &Path, always: bool) -> Option<PathBuf> {
    if dir.join(SETTINGS_FILE).is_file() {
        return Some(dir.to_path_buf());
    }
    if !always {
        return None;
    }
    match is_writable(dir) {
        Ok(()) => Some(dir.to_path_buf()),
        Err(err) => {
            log::warn!(
                "Cannot write to the program folder {} ({err}) — the settings are kept in {}",
                dir.display(),
                user_config_dir().display()
            );
            None
        }
    }
}

/// Whether a file can be created in `dir` (it is removed again).
fn is_writable(dir: &Path) -> io::Result<()> {
    let probe = dir.join(format!(".rustycorn-write-test-{}", std::process::id()));
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)?;
    std::fs::remove_file(&probe)
}

/// The first time RustyCorn keeps its settings in the program folder, copy
/// those of an earlier start from the user config folder (`%APPDATA%\RustyCorn`)
/// there, so nothing has to be set again; the old file is left alone. Returns
/// the file copied.
pub fn adopt_user_settings() -> Option<PathBuf> {
    let portable = portable_dir()?;
    if config_dir() != portable {
        return None; // overridden
    }
    let old = user_config_dir().join(SETTINGS_FILE);
    adopt_settings_file(&old, &portable.join(SETTINGS_FILE)).then_some(old)
}

/// Copy `old` to `target` when there is an `old` and no `target` yet.
fn adopt_settings_file(old: &Path, target: &Path) -> bool {
    if target.exists() || !old.is_file() {
        return false;
    }
    match std::fs::copy(old, target) {
        Ok(_) => {
            log::info!(
                "Settings taken over from {} into {}",
                old.display(),
                target.display()
            );
            true
        }
        Err(err) => {
            log::warn!(
                "Could not copy the settings {} to {}: {err}",
                old.display(),
                target.display()
            );
            false
        }
    }
}

/// The data folder when none is chosen: the program's folder when RustyCorn
/// keeps its settings there ([`portable_dir`]), else the current folder.
pub fn default_data_dir() -> PathBuf {
    portable_dir()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn settings_path() -> PathBuf {
    config_dir().join(SETTINGS_FILE)
}

/// Load the settings; missing file or fields fall back to the defaults.
pub fn load_settings() -> Settings {
    load_settings_from(&settings_path())
}

pub fn save_settings(s: &Settings) -> io::Result<()> {
    save_settings_to(s, &settings_path())
}

/// Settings from the TOML file at `path`. A missing file gives the defaults; a
/// broken one is reported and gives the defaults; a bad field gives that
/// field's default.
pub fn load_settings_from(path: &Path) -> Settings {
    let (s, problem) = load_settings_checked(path);
    *LOAD_PROBLEM.write() = problem;
    s
}

/// [`load_settings_from`], returning the problem (see [`take_load_problem`])
/// instead of storing it for the GUI.
pub fn load_settings_checked(path: &Path) -> (Settings, Option<String>) {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return (Settings::default(), None),
        Err(err) => {
            let problem = report_problem(path, &format!("could not be read ({err})"));
            return (Settings::default(), Some(problem));
        }
    };
    match text.parse::<toml::Table>() {
        Ok(table) => {
            let mut invalid = Vec::new();
            let s = settings_from_table(&table, &path.display().to_string(), &mut invalid);
            let problem = (!invalid.is_empty()).then(|| {
                report_problem(
                    path,
                    &format!("has invalid values for {}", invalid.join(", ")),
                )
            });
            (s, problem)
        }
        Err(err) => {
            let first = err
                .to_string()
                .lines()
                .next()
                .unwrap_or_default()
                .to_string();
            (
                Settings::default(),
                Some(report_problem(
                    path,
                    &format!("is not valid TOML ({first})"),
                )),
            )
        }
    }
}

/// Keep a copy of a settings file that could not be read cleanly before the
/// defaults are saved over it; returns the message for the GUI.
fn report_problem(path: &Path, what: &str) -> String {
    let mut msg = format!(
        "The settings file {} {what}; the defaults are used",
        path.display()
    );
    let mut backup = path.as_os_str().to_owned();
    backup.push(format!(
        ".broken-{}",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    ));
    let backup = PathBuf::from(backup);
    match std::fs::copy(path, &backup) {
        Ok(_) => msg.push_str(&format!(
            " instead. The old file is kept as {}.",
            backup.display()
        )),
        Err(err) => msg.push_str(&format!(" instead (it could not be copied aside: {err}).")),
    }
    log::warn!("{msg}");
    msg
}

/// Write the settings to `path` (creating its folder), via a temporary file
/// that is renamed, so a crash never leaves half a file.
pub fn save_settings_to(s: &Settings, path: &Path) -> io::Result<()> {
    let text = toml::to_string(s).map_err(io::Error::other)?;
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

impl Settings {
    /// The data folder in use: the saved one if it exists or can be created, else
    /// [`default_data_dir`].
    pub fn data_dir(&self) -> PathBuf {
        if self.data_dir.is_empty() {
            return default_data_dir();
        }
        match std::fs::create_dir_all(&self.data_dir) {
            Ok(()) => PathBuf::from(&self.data_dir),
            Err(err) => {
                let here = default_data_dir();
                log::warn!(
                    "Data folder {} is not usable — saving to {} instead: {err}",
                    self.data_dir,
                    here.display()
                );
                here
            }
        }
    }

    /// The methods folder in use: the chosen one if it exists or can be
    /// created, else (and when none is chosen) `<data folder>/methods`.
    pub fn methods_dir(&self) -> PathBuf {
        self.methods_dir_in(&self.data_dir())
    }

    /// [`Settings::methods_dir`] with the data folder already known.
    pub fn methods_dir_in(&self, data_folder: &Path) -> PathBuf {
        let default = method_dir(data_folder);
        if self.methods_dir.is_empty() {
            return default;
        }
        match std::fs::create_dir_all(&self.methods_dir) {
            Ok(()) => PathBuf::from(&self.methods_dir),
            Err(err) => {
                log::warn!(
                    "Methods folder {} is not usable — using {} instead: {err}",
                    self.methods_dir,
                    default.display()
                );
                default
            }
        }
    }

    /// True when the methods folder is `<data folder>/methods` (none chosen).
    pub fn methods_dir_is_default(&self) -> bool {
        self.methods_dir.is_empty()
    }
}

/// The default methods folder, `<data folder>/methods`.
pub fn method_dir(data_folder: &Path) -> PathBuf {
    data_folder.join("methods")
}

/// Field by field, so one bad value only resets that field to its default.
fn settings_from_table(t: &toml::Table, source: &str, invalid: &mut Vec<String>) -> Settings {
    let d = Settings::default();
    let boolean = |v: &toml::Value| v.as_bool();
    let string = |v: &toml::Value| v.as_str().map(str::to_string);
    // Integers are accepted as numbers; a rate or unit must be positive.
    let positive = |v: &toml::Value| toml_number(v).filter(|x| x.is_finite() && *x > 0.0);
    let panels = |v: &toml::Value| {
        v.as_array()?
            .iter()
            .map(|x| x.as_str().map(str::to_string))
            .collect::<Option<Vec<_>>>()
    };
    let gpu = |v: &toml::Value| v.as_str().and_then(GpuBackend::parse);

    Settings {
        demo_mode: table_field(t, "demo_mode", &boolean, d.demo_mode, source, invalid),
        serial_port: table_field(t, "serial_port", &string, d.serial_port, source, invalid),
        volume_unit_ul: table_field(
            t,
            "volume_unit_ul",
            &positive,
            d.volume_unit_ul,
            source,
            invalid,
        ),
        log_prefix: table_field(t, "log_prefix", &string, d.log_prefix, source, invalid),
        data_dir: table_field(t, "data_dir", &string, d.data_dir, source, invalid),
        methods_dir: table_field(t, "methods_dir", &string, d.methods_dir, source, invalid),
        dark_mode: table_field(t, "dark_mode", &boolean, d.dark_mode, source, invalid),
        refresh_hz: table_field(t, "refresh_hz", &positive, d.refresh_hz, source, invalid),
        last_user: table_field(t, "last_user", &string, d.last_user, source, invalid),
        start_panels: table_field(t, "start_panels", &panels, d.start_panels, source, invalid),
        save_raw: table_field(t, "save_raw", &boolean, d.save_raw, source, invalid),
        gpu_backend: table_field(t, "gpu_backend", &gpu, d.gpu_backend, source, invalid),
    }
}

/// The value under `name`, converted by `conv`; a missing key or a value
/// `conv` rejects gives `default`.
fn table_field<T>(
    t: &toml::Table,
    name: &str,
    conv: &dyn Fn(&toml::Value) -> Option<T>,
    default: T,
    source: &str,
    invalid: &mut Vec<String>,
) -> T {
    let Some(v) = t.get(name) else {
        return default;
    };
    conv(v).unwrap_or_else(|| {
        log::warn!("Settings {source}: ignoring invalid {name} = {v} (using the default)");
        invalid.push(name.to_string());
        default
    })
}

fn toml_number(v: &toml::Value) -> Option<f64> {
    match v {
        toml::Value::Float(x) => Some(*x),
        toml::Value::Integer(i) => Some(*i as f64),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp dir")
    }

    /// The `.broken-…` copies next to `path`.
    fn backups(path: &Path) -> Vec<PathBuf> {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let mut v: Vec<PathBuf> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(&format!("{name}.broken-"))
            })
            .collect();
        v.sort();
        v
    }

    #[test]
    fn broken_settings_file_is_kept_aside() {
        let dir = temp();
        let path = dir.path().join("settings.toml");
        let text = "demo_mode = true\ndemo_mode = false\nserial_port = \"COM4\"\n"; // duplicate key
        std::fs::write(&path, text).unwrap();
        let (s, problem) = load_settings_checked(&path);
        assert_eq!(s, Settings::default());
        let problem = problem.expect("a problem is reported");
        assert!(problem.contains("not valid TOML"), "{problem}");
        let copies = backups(&path);
        assert_eq!(copies.len(), 1);
        assert_eq!(std::fs::read_to_string(&copies[0]).unwrap(), text);
        assert!(
            problem.contains(&copies[0].display().to_string()),
            "{problem}"
        );
    }

    #[test]
    fn invalid_values_are_reported_and_kept_aside() {
        let dir = temp();
        let path = dir.path().join("settings.toml");
        std::fs::write(
            &path,
            "serial_port = \"COM4\"\nrefresh_hz = -1\ndark_mode = \"yes\"\n",
        )
        .unwrap();
        let (s, problem) = load_settings_checked(&path);
        assert_eq!(s.serial_port, "COM4", "valid fields are kept");
        assert_eq!(s.refresh_hz, Settings::default().refresh_hz);
        let problem = problem.expect("a problem is reported");
        assert!(
            problem.contains("refresh_hz") && problem.contains("dark_mode"),
            "{problem}"
        );
        assert_eq!(backups(&path).len(), 1);
    }

    #[test]
    fn good_or_missing_settings_report_nothing() {
        let dir = temp();
        let path = dir.path().join("settings.toml");
        assert_eq!(load_settings_checked(&path), (Settings::default(), None));
        save_settings_to(
            &Settings {
                serial_port: "COM4".into(),
                ..Settings::default()
            },
            &path,
        )
        .unwrap();
        let (s, problem) = load_settings_checked(&path);
        assert_eq!((s.serial_port.as_str(), problem), ("COM4", None));
        assert!(backups(&path).is_empty());
    }

    #[test]
    fn defaults() {
        // every panel shown, no raw capture
        let s = Settings::default();
        assert!(!s.demo_mode && !s.dark_mode && !s.save_raw);
        assert_eq!(s.volume_unit_ul, crate::DEFAULT_VOLUME_UNIT_UL);
        assert_eq!(s.log_prefix, "akta");
        assert_eq!(s.data_dir, "");
        assert_eq!(s.methods_dir, "");
        assert_eq!(s.refresh_hz, 2.0);
        assert!(s.last_user.is_empty());
        assert_eq!(s.start_panels, ALL_PANELS);
        #[cfg(target_os = "linux")]
        assert_eq!(s.serial_port, "/dev/ttyUSB0");
        #[cfg(target_os = "linux")]
        assert_eq!(s.gpu_backend, GpuBackend::Auto);
        #[cfg(target_os = "windows")]
        assert_eq!(s.gpu_backend, GpuBackend::Gl);
    }

    #[test]
    fn gpu_backend_names() {
        for b in GpuBackend::ALL {
            assert_eq!(GpuBackend::parse(b.name()), Some(b));
            let text = toml::to_string(&Settings {
                gpu_backend: b,
                ..Settings::default()
            })
            .unwrap();
            assert!(
                text.contains(&format!("gpu_backend = \"{}\"", b.name())),
                "{text}"
            );
        }
        assert_eq!(GpuBackend::parse(" OpenGL "), Some(GpuBackend::Gl));
        assert_eq!(GpuBackend::parse("DX12"), Some(GpuBackend::Dx12));
        assert_eq!(GpuBackend::parse("d3d"), None);
        assert!(GpuBackend::choices().contains(&GpuBackend::Auto));
        assert!(GpuBackend::choices().contains(&GpuBackend::platform_default()));
    }

    #[test]
    fn gpu_backend_in_the_file() {
        let dir = temp();
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, "gpu_backend = \"GL\"\n").unwrap();
        assert_eq!(load_settings_checked(&path).0.gpu_backend, GpuBackend::Gl);
        std::fs::write(&path, "gpu_backend = \"glide\"\n").unwrap();
        let (s, problem) = load_settings_checked(&path);
        assert_eq!(s.gpu_backend, GpuBackend::platform_default());
        assert!(problem.unwrap().contains("gpu_backend"));
    }

    #[test]
    fn portable_folder() {
        let dir = temp();
        // without a settings file: only where always wanted (Windows)
        assert_eq!(portable_dir_for(dir.path(), false), None);
        assert_eq!(
            portable_dir_for(dir.path(), true).as_deref(),
            Some(dir.path())
        );
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            0,
            "no probe left"
        );
        // a settings file next to the program: everywhere
        std::fs::write(dir.path().join(SETTINGS_FILE), "").unwrap();
        assert_eq!(
            portable_dir_for(dir.path(), false).as_deref(),
            Some(dir.path())
        );
        // a folder that cannot be written to (here: one that does not exist)
        assert_eq!(portable_dir_for(&dir.path().join("missing"), true), None);
    }

    #[test]
    fn earlier_settings_are_adopted_once() {
        let dir = temp();
        let old = dir.path().join("appdata").join(SETTINGS_FILE);
        let target = dir.path().join("program").join(SETTINGS_FILE);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        assert!(!adopt_settings_file(&old, &target), "nothing to adopt");
        std::fs::create_dir_all(old.parent().unwrap()).unwrap();
        std::fs::write(&old, "serial_port = \"COM4\"\n").unwrap();
        assert!(adopt_settings_file(&old, &target));
        assert_eq!(load_settings_checked(&target).0.serial_port, "COM4");
        assert!(old.is_file(), "the old file is kept");
        // never over existing settings
        std::fs::write(&old, "serial_port = \"COM9\"\n").unwrap();
        assert!(!adopt_settings_file(&old, &target));
        assert_eq!(load_settings_checked(&target).0.serial_port, "COM4");
    }

    #[test]
    fn round_trip() {
        let dir = temp();
        let path = dir.path().join("sub").join("settings.toml");
        let mut s = Settings {
            demo_mode: true,
            serial_port: "COM7".into(),
            volume_unit_ul: 1.0,
            log_prefix: "alex".into(),
            data_dir: "/tmp/x y".into(),
            methods_dir: "/srv/akta methods".into(),
            dark_mode: true,
            refresh_hz: 10.0,
            start_panels: vec!["pH".into()],
            save_raw: true,
            gpu_backend: GpuBackend::Vulkan,
            ..Settings::default()
        };
        s.last_user = "Alex \"K\", lab 2".into();
        save_settings_to(&s, &path).expect("save");
        assert!(!dir.path().join("sub").join("settings.toml.tmp").exists());
        assert_eq!(load_settings_from(&path), s);
        // also through serde
        let text = std::fs::read_to_string(&path).expect("read");
        assert_eq!(toml::from_str::<Settings>(&text).expect("serde"), s);
        // empty panels survive too
        s.start_panels.clear();
        save_settings_to(&s, &path).expect("save");
        assert_eq!(load_settings_from(&path), s);
    }

    #[test]
    fn missing_and_invalid_fields_use_defaults() {
        let dir = temp();
        let path = dir.path().join("settings.toml");
        assert_eq!(load_settings_from(&path), Settings::default()); // no file
        std::fs::write(&path, "demo_mode = true\n").expect("write");
        let s = load_settings_from(&path);
        assert!(s.demo_mode);
        assert_eq!(
            Settings {
                demo_mode: false,
                ..s
            },
            Settings::default()
        );

        std::fs::write(
            &path,
            "demo_mode = \"yes\"\nserial_port = 3\nvolume_unit_ul = 1\nlog_prefix = \"p\"\n\
             refresh_hz = -4.0\ndark_mode = true\nstart_panels = [\"pH\", 3]\nlast_user = 5\n\
             unknown_key = 1\n",
        )
        .expect("write");
        let s = load_settings_from(&path);
        let d = Settings::default();
        assert!(!s.demo_mode);
        assert_eq!(s.serial_port, d.serial_port);
        assert_eq!(s.volume_unit_ul, 1.0); // integer accepted
        assert_eq!(s.log_prefix, "p");
        assert_eq!(s.refresh_hz, 2.0);
        assert!(s.dark_mode);
        assert_eq!(s.start_panels, d.start_panels);
        assert!(s.last_user.is_empty());
    }

    #[test]
    fn broken_file_gives_defaults() {
        let dir = temp();
        let path = dir.path().join("settings.toml");
        for junk in [
            "demo_mode = = true",
            "[[[",
            "\u{0}\u{1}garbage",
            "a = 1\na = 2\n",
        ] {
            std::fs::write(&path, junk).expect("write");
            assert_eq!(load_settings_from(&path), Settings::default(), "{junk:?}");
        }
        std::fs::write(&path, [0xff, 0xfe, 0x00]).expect("write");
        assert_eq!(load_settings_from(&path), Settings::default());
        // a folder where the file should be
        let path = dir.path().join("folder.toml");
        std::fs::create_dir(&path).expect("mkdir");
        assert_eq!(load_settings_from(&path), Settings::default());
    }

    #[test]
    fn config_dir_override() {
        let dir = temp();
        set_config_dir_override(Some(dir.path().to_path_buf()));
        assert_eq!(config_dir(), dir.path());
        assert_eq!(settings_path(), dir.path().join("settings.toml"));
        let s = Settings {
            log_prefix: "override".into(),
            ..Settings::default()
        };
        save_settings(&s).expect("save");
        assert_eq!(load_settings(), s);
        set_config_dir_override(None);
        assert_ne!(config_dir(), dir.path());
        assert!(
            config_dir().ends_with(crate::APP_NAME)
                || std::env::var_os(CONFIG_DIR_ENV).is_some()
                || portable_dir().is_some()
        );
    }

    /// Runs in a child process with RUSTYCORN_CONFIG_DIR set (see `config_dir_env`).
    #[test]
    #[ignore = "run by config_dir_env in a child process"]
    fn config_dir_env_child() {
        let want = std::env::var_os("RC_EXPECT_CONFIG_DIR").expect("expected dir");
        assert_eq!(config_dir(), PathBuf::from(want));
    }

    #[test]
    fn config_dir_env() {
        // Setting the variable needs `unsafe` in Rust 2024, so check it in a child process.
        let dir = temp();
        let exe = std::env::current_exe().expect("test binary");
        let status = std::process::Command::new(exe)
            .args([
                "--exact",
                "settings::tests::config_dir_env_child",
                "--ignored",
                "--quiet",
            ])
            .env(CONFIG_DIR_ENV, dir.path())
            .env("RC_EXPECT_CONFIG_DIR", dir.path())
            .stdout(std::process::Stdio::null())
            .status()
            .expect("run child");
        assert!(status.success());
    }

    #[test]
    fn data_dir_fallbacks() {
        let cwd = default_data_dir();
        #[cfg(target_os = "linux")]
        assert_eq!(cwd, std::env::current_dir().expect("cwd"));
        assert_eq!(Settings::default().data_dir(), cwd);
        let dir = temp();
        let wanted = dir.path().join("a").join("b");
        let s = Settings {
            data_dir: wanted.display().to_string(),
            ..Settings::default()
        };
        assert_eq!(s.data_dir(), wanted);
        assert!(wanted.is_dir());
        // a file in the way: not usable, use the default folder
        let file = dir.path().join("file");
        std::fs::write(&file, "x").expect("write");
        let s = Settings {
            data_dir: file.join("sub").display().to_string(),
            ..Settings::default()
        };
        assert_eq!(s.data_dir(), cwd);
        assert_eq!(method_dir(&wanted), wanted.join("methods"));
    }

    #[test]
    fn methods_dir_setting() {
        let dir = temp();
        let data = dir.path().join("data");
        let path = dir.path().join("settings.toml");
        // an older settings file without the key: the default, <data folder>/methods
        std::fs::write(
            &path,
            format!(
                "demo_mode = true\ndata_dir = {:?}\n",
                data.display().to_string()
            ),
        )
        .expect("write");
        let s = load_settings_from(&path);
        assert!(s.demo_mode && s.methods_dir_is_default());
        assert_eq!(s.methods_dir(), data.join("methods"));
        assert_eq!(
            s.methods_dir_in(Path::new("/x")),
            Path::new("/x").join("methods")
        );

        // a folder of its own: created, kept apart from the data folder, saved
        let methods = dir.path().join("shared").join("methods here");
        let s = Settings {
            methods_dir: methods.display().to_string(),
            ..s
        };
        assert!(!s.methods_dir_is_default());
        assert_eq!(s.methods_dir(), methods);
        assert!(methods.is_dir());
        assert_eq!(s.methods_dir_in(&data), methods);
        save_settings_to(&s, &path).expect("save");
        assert!(
            std::fs::read_to_string(&path)
                .expect("read")
                .contains("methods_dir = ")
        );
        assert_eq!(load_settings_from(&path), s);

        // not usable (a file in the way): back to <data folder>/methods
        let file = dir.path().join("file");
        std::fs::write(&file, "x").expect("write");
        let s = Settings {
            methods_dir: file.join("sub").display().to_string(),
            ..s
        };
        assert_eq!(s.methods_dir(), data.join("methods"));

        // a bad value: the default
        std::fs::write(&path, "methods_dir = 3\n").expect("write");
        assert_eq!(load_settings_from(&path).methods_dir, "");
    }
}
