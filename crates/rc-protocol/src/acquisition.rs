//! The live data source (instrument link or demo), which can be stopped and
//! restarted with new settings while the window stays open.

use std::fs::File;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use rc_core::csvlog::{CsvLog, raw_capture_path, unique_log_name};
use rc_core::settings::Settings;
use rc_core::{InstrumentState, SharedApp};

/// A data source run on its own thread until the flag is set.
type Source = Box<dyn FnOnce(SharedApp, Arc<AtomicBool>) + Send>;

pub struct Acquisition {
    pub app: SharedApp,
    pub settings: Settings,
    shutdown: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Acquisition {
    pub fn new(app: SharedApp, settings: Settings) -> Acquisition {
        Acquisition {
            app,
            settings,
            shutdown: Arc::new(AtomicBool::new(false)),
            thread: None,
        }
    }

    /// Apply the settings to the app (volume unit, prefix, data
    /// and method folders), reset the run, open a fresh log, then start the demo
    /// or the link thread. Run information is recorded when a run starts (the
    /// method's setup) and by the user, no longer at the top of every log.
    pub fn start(&mut self) {
        self.start_source(None);
    }

    /// [`Acquisition::start`] with `source` feeding the app in place of the
    /// demo or the instrument link (e.g. the link on a simulated Akta in
    /// tests). It runs on its own thread and must return once the flag it is
    /// given is set; `stop` waits for it.
    pub fn start_with_source(
        &mut self,
        source: impl FnOnce(SharedApp, Arc<AtomicBool>) + Send + 'static,
    ) {
        self.start_source(Some(Box::new(source)));
    }

    fn start_source(&mut self, source: Option<Source>) {
        if self.thread.is_some() {
            self.stop();
        }
        let s = &self.settings;
        let log_dir = s.data_dir();
        // the methods folder chosen in Settings, else <data folder>/methods
        let method_dir = s.methods_dir_in(&log_dir);
        // a new, empty methods folder gets the example method
        rc_methods::install_example_methods_logged(&method_dir);
        // The files are created before the app is locked, so the GUI and a
        // still-running source never wait for the disk.
        let log_filename = unique_log_name(&log_dir, &s.log_prefix);
        let (log, log_error) = match CsvLog::create(&log_filename) {
            Ok(log) => (Some(log), None),
            Err(err) => {
                // Without a log the data is still shown; the error is reported.
                log::error!("Could not create the log {}: {err}", log_filename.display());
                let msg = format!(
                    "no log file could be created in {}: {err}",
                    log_dir.display()
                );
                (None, Some(msg))
            }
        };
        let raw = if s.save_raw && !s.demo_mode {
            let path = raw_capture_path(&log_filename);
            match File::create(&path) {
                Ok(f) => Some(f),
                Err(err) => {
                    log::error!("Could not create the raw capture {}: {err}", path.display());
                    None
                }
            }
        } else {
            None
        };
        {
            let mut app = self.app.lock();
            app.volume_unit_ul = s.volume_unit_ul;
            app.fraction_delay_ml = s.fraction_delay_ml;
            app.log_prefix = s.log_prefix.clone();
            app.method_dir = method_dir;
            app.log_dir = log_dir;
            app.inst = InstrumentState::default(); // nothing is known about the new source yet
            app.reset_run();
            app.log_filename = log_filename;
            app.log = log;
            app.log_error = log_error;
            // nothing is known about the next run's method yet
            app.run_info_for_new_log.clear();
            app.pending_method_event = None;
            app.raw = raw;
            app.status = if s.demo_mode {
                "Demo mode — simulated data".to_string()
            } else {
                format!("Opening {} …", s.serial_port)
            };
            app.live = true;
            app.demo = s.demo_mode;
            app.touch();
        }

        // a fresh flag: a thread that is somehow still running keeps its own, set one
        self.shutdown = Arc::new(AtomicBool::new(false));
        let app = self.app.clone();
        let shutdown = self.shutdown.clone();
        let spawned = if let Some(source) = source {
            std::thread::Builder::new()
                .name("data-source".to_string())
                .spawn(move || source(app, shutdown))
        } else if s.demo_mode {
            log::info!("Starting demo mode");
            std::thread::Builder::new()
                .name("demo".to_string())
                .spawn(move || rc_core::demo::run_demo(app, shutdown))
        } else {
            log::info!("Connecting to Akta at {}", s.serial_port);
            // The link polls on its own thread, so a busy GUI cannot silence it.
            let port = s.serial_port.clone();
            std::thread::Builder::new()
                .name("akta-link".to_string())
                .spawn(move || crate::link::run_linklayer(app, &shutdown, &port))
        };
        match spawned {
            Ok(handle) => self.thread = Some(handle),
            Err(err) => {
                log::error!("Could not start the data source thread: {err}");
                let mut app = self.app.lock();
                app.live = false;
                app.status = "Stopped".to_string();
                app.touch();
            }
        }
    }

    /// Stop the thread, close the log (discarding an empty one), status "Stopped".
    pub fn stop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread.take()
            && handle.join().is_err()
        {
            log::error!("The data source thread ended with a panic");
        }
        let mut app = self.app.lock();
        app.close_log();
        app.raw = None;
        app.log_error = None;
        app.status = "Stopped".to_string();
        app.live = false;
        app.demo = false;
        app.touch();
    }

    pub fn restart(&mut self) {
        self.stop();
        self.start();
    }

    pub fn is_running(&self) -> bool {
        self.thread.is_some()
    }
}

impl Drop for Acquisition {
    fn drop(&mut self) {
        if self.thread.is_some() {
            self.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use rc_core::AppState;

    use super::*;

    fn settings(dir: &std::path::Path) -> Settings {
        // never the user's settings
        rc_core::settings::set_config_dir_override(Some(dir.join("config")));
        Settings {
            data_dir: dir.join("data").to_string_lossy().into_owned(),
            log_prefix: "acq".to_string(),
            volume_unit_ul: 1.0,
            // a path that cannot exist: no device is ever opened
            serial_port: "/nonexistent/rustycorn-test-port".to_string(),
            last_user: "Alex".to_string(),
            ..Settings::default()
        }
    }

    fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
        let end = Instant::now() + Duration::from_secs(10);
        while !cond() {
            assert!(Instant::now() < end, "timed out waiting for: {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn csv_files(dir: &std::path::Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".csv") || n.ends_with(".txt"))
            .collect();
        v.sort();
        v
    }

    #[test]
    fn link_start_and_stop_without_instrument() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = settings(tmp.path());
        s.save_raw = true;
        let app = AppState::default().shared();
        // left over from before: a setup for a run that never started
        app.lock()
            .run_info_for_new_log
            .insert("Column".to_string(), "stale".to_string());
        let mut acq = Acquisition::new(app.clone(), s);
        acq.start();
        assert!(acq.is_running());
        let data = tmp.path().join("data");
        {
            let a = app.lock();
            assert!(a.live && !a.demo);
            assert_eq!(a.volume_unit_ul, 1.0);
            assert_eq!(a.log_prefix, "acq");
            assert_eq!(a.log_dir, data);
            assert_eq!(a.method_dir, data.join("methods"));
            assert!(a.log.is_some() && a.raw.is_some() && a.log_error.is_none());
            assert!(a.log_filename.starts_with(&data));
            // no run information until a run starts
            assert!(a.events.is_empty());
            assert!(a.run_info_for_new_log.is_empty());
        }
        let files = csv_files(&data);
        assert_eq!(files.len(), 2, "{files:?}"); // log and raw capture
        assert!(files[0].ends_with("_acq.csv") && files[1].ends_with("_acq_raw.txt"));
        wait_until("port reported missing", || {
            app.lock().status
                == "Serial port /nonexistent/rustycorn-test-port not available — retrying"
        });

        let t = Instant::now();
        acq.stop();
        assert!(
            t.elapsed() < Duration::from_secs(1),
            "stop waits for a 2 s retry sleep"
        );
        assert!(!acq.is_running());
        let a = app.lock();
        assert_eq!(a.status, "Stopped");
        assert!(!a.live && a.log.is_none() && a.raw.is_none());
        // nothing was recorded: the log and its raw capture are discarded
        assert!(csv_files(&data).is_empty());
    }

    #[test]
    fn methods_folder_of_its_own() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = settings(tmp.path());
        s.demo_mode = true;
        let methods = tmp.path().join("shared methods");
        s.methods_dir = methods.display().to_string();
        let app = AppState::default().shared();
        let mut acq = Acquisition::new(app.clone(), s);
        acq.start();
        {
            let a = app.lock();
            assert_eq!(a.method_dir, methods);
            assert_eq!(a.log_dir, tmp.path().join("data"));
        }
        assert!(methods.is_dir());
        // a new methods folder gets the example method
        assert_eq!(
            rc_methods::stored_methods(&methods),
            vec![rc_methods::EXAMPLE_METHOD_NUMBER]
        );
        acq.stop();
        // a folder in use is left as it is, even if its methods were deleted
        let path = rc_methods::method_path(&methods, rc_methods::EXAMPLE_METHOD_NUMBER);
        rc_methods::retire_method_file(&path).unwrap();
        acq.start();
        assert!(rc_methods::stored_methods(&methods).is_empty());
        acq.stop();
        // back to the default: <data folder>/methods, from the next start on
        acq.settings.methods_dir.clear();
        acq.start();
        assert_eq!(
            app.lock().method_dir,
            tmp.path().join("data").join("methods")
        );
        acq.stop();
    }

    #[cfg(unix)]
    #[test]
    fn a_log_that_cannot_be_created_is_reported() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let mut s = settings(tmp.path());
        s.demo_mode = true;
        let data = tmp.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o555)).unwrap();
        if std::fs::File::create(data.join("probe")).is_ok() {
            return; // running as root: permissions do not stop us
        }
        let app = AppState::default().shared();
        let mut acq = Acquisition::new(app.clone(), s);
        acq.start();
        {
            let a = app.lock();
            assert!(a.log.is_none() && a.live);
            let err = a.log_error.as_deref().unwrap_or_default();
            assert!(err.starts_with("no log file could be created in "), "{err}");
        }
        wait_until("demo data without a log", || !app.lock().trend.is_empty());
        acq.stop();
        assert!(app.lock().log_error.is_none());
        std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn demo_start_restart_stop() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = settings(tmp.path());
        s.demo_mode = true;
        s.save_raw = true; // never in demo mode
        let app = AppState::default().shared();
        let mut acq = Acquisition::new(app.clone(), s);
        acq.start();
        {
            let a = app.lock();
            assert_eq!(a.status, "Demo mode — simulated data");
            assert!(a.live && a.demo && a.raw.is_none());
        }
        wait_until("demo data", || !app.lock().trend.is_empty());
        let first = app.lock().log_filename.clone();

        acq.restart();
        let second = app.lock().log_filename.clone();
        assert_ne!(first, second);
        assert!(first.exists(), "a log with data is kept");
        wait_until("demo data after restart", || !app.lock().trend.is_empty());
        drop(acq); // stops
        let a = app.lock();
        assert_eq!(a.status, "Stopped");
        assert!(!a.live && !a.demo);
        assert!(second.exists());
        let csv = std::fs::read_to_string(&second).unwrap();
        assert!(csv.contains(",TREND,") && !csv.contains("EVT_INFO"));
        assert!(a.run_serial >= 2, "a run started after each start");
    }
}
