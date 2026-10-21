//! CSV logging, flushed after every row, log file names, and the log reader.
//!
//! The file format is fixed: other programs read these logs, and older logs
//! must keep opening. A log is a comment line with the start time, the header
//! [`CSV_HEADER`], then one row per trend point or event:
//! - `wall_time` as `yyyy-mm-ddTHH:MM:SS[.sss]` (milliseconds only when non-zero),
//! - `cum_time_ms` with 1 decimal, `cum_vol_mL` with 6,
//! - TREND rows: uv 8, cond 4, pH 2, %B 2, pressure 4, temperature 2 decimals,
//!   empty message,
//! - event rows: empty value columns and the text in double quotes, with
//!   commas turned into semicolons and line breaks into spaces.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use chrono::{Local, NaiveDateTime, Timelike};
use regex::Regex;

use crate::logtext::{parse_f64, trim_space};
use crate::runinfo::{run_info, run_info_texts};
use crate::types::{AktaEvent, AppState, EventKind, TrendPoint};

pub const CSV_HEADER: &str = "wall_time,cum_time_ms,cum_vol_mL,type,uv_AU,cond_mScm,ph,pctb_pct,pres_MPa,temp_degC,message\n";

/// First line of every log, followed by the start time ([`log_now`]).
pub const CSV_COMMENT_PREFIX: &str = "# Akta PRIME data log started ";

/// An open CSV log file.
#[derive(Debug)]
pub struct CsvLog {
    pub path: PathBuf,
    file: File,
}

impl CsvLog {
    /// Create the file and write the comment line and header.
    pub fn create(path: &Path) -> io::Result<CsvLog> {
        let mut file = File::create(path)?;
        let head = format!("{CSV_COMMENT_PREFIX}{}\n{CSV_HEADER}", log_now());
        file.write_all(head.as_bytes())?;
        file.flush()?;
        log::info!("Logging to {}", path.display());
        Ok(CsvLog {
            path: path.to_path_buf(),
            file,
        })
    }

    pub fn write_trend(&mut self, pt: &TrendPoint) -> io::Result<()> {
        self.write_row(&trend_row(&log_now(), pt))
    }

    pub fn write_event(&mut self, ev: &AktaEvent) -> io::Result<()> {
        self.write_row(&event_row(&log_now(), ev))
    }

    // One write per row, so a crash never leaves half a row; File is unbuffered,
    // so the row is on disk (in the OS cache) when this returns.
    fn write_row(&mut self, row: &str) -> io::Result<()> {
        self.file.write_all(row.as_bytes())?;
        self.file.flush()
    }
}

/// Wall-time text of the log format: `2026-10-20T15:52:04.754`, with the
/// milliseconds left out when they are zero (`2026-10-20T15:54:03`), and the
/// year written with at least four digits.
pub fn log_datetime(t: &NaiveDateTime) -> String {
    let ms = t.nanosecond() / 1_000_000 % 1000; // leap-second nanos ≥ 1e9
    let base = t.format("%Y-%m-%dT%H:%M:%S");
    if ms == 0 {
        base.to_string()
    } else {
        format!("{base}.{ms:03}")
    }
}

/// The local time now in the log format (truncated to ms).
pub fn log_now() -> String {
    log_datetime(&Local::now().naive_local())
}

/// `x` with `digits` decimals, as C's `printf("%.Nf")` rounds the exact binary
/// value; infinities are written "Inf" and "-Inf" (Rust would write "inf").
fn fixed(x: f64, digits: usize) -> String {
    if x.is_infinite() {
        if x > 0.0 { "Inf" } else { "-Inf" }.to_string()
    } else {
        format!("{x:.digits$}")
    }
}

/// Event text as written into the quoted message column: commas become
/// semicolons and newlines spaces. Quotes inside the text are not escaped:
/// readers strip the surrounding quotes and keep the rest.
pub fn escape_message(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            ',' => ';',
            '\n' => ' ',
            c => c,
        })
        .collect()
}

/// One TREND row, `wall` being the wall-time text.
pub fn trend_row(wall: &str, pt: &TrendPoint) -> String {
    format!(
        "{wall},{},{},TREND,{},{},{},{},{},{},\n",
        fixed(pt.cum_time_ms, 1),
        fixed(pt.cum_vol_ml, 6),
        fixed(pt.uv, 8),
        fixed(pt.cond_mscm, 4),
        fixed(pt.ph, 2),
        fixed(pt.pctb, 2),
        fixed(pt.pres_mpa, 4),
        fixed(pt.temp_degc, 2),
    )
}

/// One event row, `wall` being the wall-time text.
pub fn event_row(wall: &str, ev: &AktaEvent) -> String {
    format!(
        "{wall},{},{},{},,,,,,,\"{}\"\n",
        fixed(ev.cum_time_ms, 1),
        fixed(ev.cum_vol_ml, 6),
        ev.kind.csv_name(),
        escape_message(&ev.text),
    )
}

/// `<dir>/<prefix>_<yyyymmdd_HHMMSS>.csv` ("akta" when there is no prefix).
pub fn new_log_name(dir: &Path, prefix: &str) -> PathBuf {
    let prefix = if prefix.is_empty() { "akta" } else { prefix };
    dir.join(format!(
        "{prefix}_{}.csv",
        Local::now().format("%Y%m%d_%H%M%S")
    ))
}

/// A fresh log file name that never reuses an existing file (two starts in
/// one second): `…_2.csv`, `…_3.csv`, …
pub fn unique_log_name(dir: &Path, prefix: &str) -> PathBuf {
    let mut name = new_log_name(dir, prefix);
    let mut n = 1;
    while name.exists() {
        n += 1;
        name = log_side_file(&new_log_name(dir, prefix), &format!("_{n}.csv"));
    }
    name
}

/// A file that belongs to the log `log`: its `.csv` ending replaced by `suffix`
/// (`_raw.txt`, `_method_3.met`). A name without `.csv` gets the suffix
/// appended, so it can never be the log itself.
pub fn log_side_file(log: &Path, suffix: &str) -> PathBuf {
    let s = log.as_os_str().to_string_lossy();
    match s.strip_suffix(".csv") {
        Some(stem) => PathBuf::from(format!("{stem}{suffix}")),
        None => PathBuf::from(format!("{s}{suffix}")),
    }
}

/// The raw instrument capture saved next to a log (`<log>_raw.txt`).
pub fn raw_capture_path(log: &Path) -> PathBuf {
    log_side_file(log, "_raw.txt")
}

/// True if a log with these contents is worth keeping: any trend data, or an
/// event other than connecting, run information or link messages.
fn has_data(trend: &[TrendPoint], events: &[AktaEvent]) -> bool {
    !trend.is_empty()
        || events.iter().any(|e| {
            !matches!(e.kind, EventKind::Connect | EventKind::Info) && !e.text.starts_with("Link:")
        })
}

fn remove_if_exists(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => log::warn!("Could not delete {}: {err}", path.display()),
    }
}

static REBUILD_FRACTION_SIZE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)Fraction size\s+([0-9]+(?:\.[0-9]+)?)\s+ml").expect("valid regex")
});
static REBUILD_FEED_TUBE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)Feed Tube Nr\s+(\d+)").expect("valid regex"));

impl AppState {
    /// Start a fresh log file for a new run: close the
    /// current CSV, open a new timestamped one, clear the display and write
    /// `run_info_for_new_log` into it. A no-op returning false when the current
    /// log has no trend data yet, so back-to-back run-start signals and the first
    /// run after connecting do not spawn empty files.
    pub fn roll_log(&mut self) -> bool {
        if self.trend.is_empty() {
            return false;
        }
        // The unique name only differs from `new_log_name` when that file
        // already exists (two runs started within one second): never overwrite a log.
        let name = unique_log_name(&self.log_dir, &self.log_prefix);
        // Open the new file first: if that fails, keep logging to the old one
        // rather than losing the run.
        let log = match CsvLog::create(&name) {
            Ok(log) => log,
            Err(err) => {
                log::error!("Could not start a new log {}: {err}", name.display());
                return false;
            }
        };
        self.log = Some(log); // drops (closes) the old file
        self.log_filename = name;
        self.log_error = None;
        self.reset_run();
        let info = self.run_info_for_new_log.clone();
        self.log_run_info(&info, false);
        self.run_serial += 1;
        log::info!(
            "New run detected — logging to {}",
            self.log_filename.display()
        );
        self.touch();
        true
    }

    /// A run starts (run state End/--- → Run, or a demo run): roll to a fresh
    /// log, or keep the current one if it has no data yet (the first run after
    /// connecting) and record the run's method setup there. The setup is used
    /// up: the next run gets one only from its own method dump.
    pub fn start_run(&mut self) {
        if !self.roll_log() {
            let info = self.run_info_for_new_log.clone();
            self.log_run_info(&info, true);
            self.run_serial += 1;
        }
        self.run_info_for_new_log.clear();
        self.touch();
    }

    /// Where the last row of the log is: the run time and volume of its last
    /// trend point or event, whichever is later.
    pub fn log_end_position(&self) -> (f64, f64) {
        let pt = self.trend.last().map(|p| (p.cum_time_ms, p.cum_vol_ml));
        let ev = self.events.last().map(|e| (e.cum_time_ms, e.cum_vol_ml));
        match (pt, ev) {
            (Some(p), Some(e)) => {
                if e.0 >= p.0 {
                    e
                } else {
                    p
                }
            }
            (Some(x), None) | (None, Some(x)) => x,
            (None, None) => (0.0, 0.0),
        }
    }

    /// Add run information to a finished log (`log_filename`, not open here):
    /// the fields of `info` that differ from the latest recorded values are
    /// appended to the end of the file as EVT_INFO rows, at the log's last run
    /// time and volume, and to `events`. Nothing already in the file is
    /// rewritten, and readers take the latest value of a field, so the file
    /// stays readable by any reader of the format. Returns the number of fields recorded.
    pub fn append_run_info(&mut self, info: &BTreeMap<String, String>) -> io::Result<usize> {
        let texts = run_info_texts(info, &run_info(&self.events));
        if texts.is_empty() {
            return Ok(0);
        }
        let path = self.log_filename.clone();
        let (cum_time_ms, cum_vol_ml) = self.log_end_position();
        let events: Vec<AktaEvent> = texts
            .into_iter()
            .map(|text| AktaEvent {
                cum_time_ms,
                cum_vol_ml,
                kind: EventKind::Info,
                text,
            })
            .collect();
        let mut file = std::fs::OpenOptions::new().append(true).open(&path)?;
        // a log cut off mid-row (crash, full disk) must not swallow the first new row
        let ends_in_newline = {
            use std::io::{Read, Seek, SeekFrom};
            let mut f = File::open(&path)?;
            let len = f.seek(SeekFrom::End(0))?;
            let mut last = [0u8; 1];
            len == 0 || {
                f.seek(SeekFrom::End(-1))?;
                f.read_exact(&mut last)?;
                last[0] == b'\n'
            }
        };
        let wall = log_now();
        let mut rows = String::new();
        if !ends_in_newline {
            rows.push('\n');
        }
        for ev in &events {
            rows.push_str(&event_row(&wall, ev));
        }
        // one write, so the rows go in whole or not at all as far as possible
        file.write_all(rows.as_bytes())?;
        file.flush()?;
        let n = events.len();
        for ev in events {
            log::info!("[{}] {} (added to {})", ev.kind, ev.text, path.display());
            self.events.push(ev);
        }
        self.touch();
        Ok(n)
    }

    /// True while a log is open and has received data: a trend point or an
    /// instrument event beyond connect, run info and link messages. The same
    /// test decides whether `close_log` keeps the file; the GUI's REC chip uses it.
    pub fn log_has_data(&self) -> bool {
        self.log.is_some() && has_data(&self.trend, &self.events)
    }

    /// Close the current log; a log that never received data (e.g. no
    /// instrument came) is deleted together with its raw capture. The raw
    /// capture is closed in any case.
    pub fn close_log(&mut self) {
        self.raw = None;
        let Some(log) = self.log.take() else {
            return;
        };
        let path = log.path.clone();
        drop(log);
        if !has_data(&self.trend, &self.events) {
            remove_if_exists(&path);
            remove_if_exists(&raw_capture_path(&path));
            log::info!("Discarded {}: no data recorded", path.display());
        }
        self.touch();
    }

    /// Replay the events in order to rebuild the fractions list from scratch
    /// Scans EVT_LOG and EVT_TUBEFEED texts for
    /// "Fraction size" and "Feed Tube Nr".
    pub fn rebuild_fractions(&mut self) {
        self.inst.fraction_number = 1;
        self.inst.fraction_vol_ml = 0.0;
        self.inst.current_fraction_idx = None;
        self.fractions.clear();

        for i in 0..self.events.len() {
            let ev = &self.events[i];
            self.inst.cum_time_ms = ev.cum_time_ms;
            self.inst.cum_vol_ml = ev.cum_vol_ml;
            if !matches!(ev.kind, EventKind::Log | EventKind::Tubefeed) {
                continue;
            }
            let size = REBUILD_FRACTION_SIZE_RE
                .captures(&ev.text)
                .and_then(|c| c[1].parse::<f64>().ok());
            let tube = REBUILD_FEED_TUBE_RE
                .captures(&ev.text)
                .and_then(|c| c[1].parse::<i64>().ok());
            if let Some(size) = size {
                self.inst.fraction_vol_ml = size;
            }
            if let Some(n) = tube {
                self.inst.fraction_number = n.max(1);
            }
            if size.is_some() || tube.is_some() {
                self.update_fraction_state();
            }
        }
        self.touch();
    }
}

/// Parse an Akta CSV log into a fresh AppState: trend
/// points, events and fractions rebuilt from the events. `log` stays None;
/// `log_filename` is set to `path`.
pub fn load_csv_log(path: &Path) -> io::Result<AppState> {
    let bytes = std::fs::read(path)?;
    let text = String::from_utf8_lossy(&bytes);
    let (trend, events) = parse_csv_log(&text);
    let mut app = AppState::new();
    app.trend = trend;
    app.events = events;
    app.rebuild_fractions();
    app.log_filename = path.to_path_buf();
    log::info!(
        "Loaded {} trend points and {} events from \"{}\"",
        app.trend.len(),
        app.events.len(),
        path.display()
    );
    app.touch();
    Ok(app)
}

/// Trend points and events of a CSV log text (the parsing part of `load_csv_log!`).
/// Comment, header, empty and malformed lines are skipped.
pub fn parse_csv_log(text: &str) -> (Vec<TrendPoint>, Vec<AktaEvent>) {
    let mut trend = Vec::new();
    let mut events = Vec::new();
    for raw_line in text.split_inclusive('\n') {
        // lines end in "\n" or "\r\n"
        let line = match raw_line.strip_suffix('\n') {
            Some(l) => l.strip_suffix('\r').unwrap_or(l),
            None => raw_line,
        };
        if line.starts_with('#') || line.starts_with("wall_time") || trim_space(line).is_empty() {
            continue;
        }
        // Limit to 11 fields; the 11th is the (possibly quoted) message
        let parts: Vec<&str> = line.splitn(11, ',').collect();
        if parts.len() < 4 {
            continue;
        }
        let (Some(cum_time_ms), Some(cum_vol_ml)) = (parse_f64(parts[1]), parse_f64(parts[2]))
        else {
            continue;
        };
        let row_type = trim_space(parts[3]);
        if row_type == "TREND" {
            if parts.len() < 10 {
                continue;
            }
            let num = |i: usize| parse_f64(parts[i]).unwrap_or(0.0);
            trend.push(TrendPoint {
                cum_time_ms,
                cum_vol_ml,
                uv: num(4),
                cond_mscm: num(5),
                ph: num(6),
                pctb: num(7),
                pres_mpa: num(8),
                temp_degc: num(9),
            });
        } else if let Some(kind) = EventKind::from_csv_name(row_type) {
            let text = parts
                .get(10)
                .map_or("", |m| trim_space(m).trim_matches('"'));
            events.push(AktaEvent {
                cum_time_ms,
                cum_vol_ml,
                kind,
                text: text.to_string(),
            });
        }
    }
    (trend, events)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(t: f64) -> TrendPoint {
        TrendPoint {
            cum_time_ms: t,
            cum_vol_ml: t / 1000.0,
            ..TrendPoint::default()
        }
    }

    fn started_log(dir: &Path) -> AppState {
        let mut app = AppState::new();
        app.log_dir = dir.to_path_buf();
        app.log_prefix = "t".into();
        app.log_filename = unique_log_name(dir, "t");
        app.log = Some(CsvLog::create(&app.log_filename).expect("create"));
        app
    }

    #[test]
    fn log_has_data_only_after_real_data() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = AppState::new();
        assert!(!app.log_has_data(), "no log open");
        app = started_log(dir.path());
        assert!(!app.log_has_data(), "empty log");
        app.push_event(EventKind::Connect, "Akta connected");
        app.push_event(EventKind::Info, "Sample: x");
        app.push_event(EventKind::Log, "Link: no response from Akta — waiting");
        assert!(
            !app.log_has_data(),
            "connect, run info and link messages are not data"
        );
        app.push_trend(pt(100.0));
        assert!(app.log_has_data(), "a trend point is data");
        app.close_log();
        assert!(!app.log_has_data(), "closed");
    }

    #[test]
    fn rows() {
        let p = TrendPoint {
            cum_time_ms: 6790.0,
            cum_vol_ml: 0.056,
            uv: -0.000164,
            cond_mscm: 0.022,
            ph: 15.11,
            pctb: 0.0,
            pres_mpa: 0.237,
            temp_degc: 7.3,
        };
        assert_eq!(
            trend_row("W", &p),
            "W,6790.0,0.056000,TREND,-0.00016400,0.0220,15.11,0.00,0.2370,7.30,\n"
        );
        let e = AktaEvent {
            cum_time_ms: 20.0,
            cum_vol_ml: 0.001,
            kind: EventKind::Log,
            text: "a, b\nc".into(),
        };
        assert_eq!(
            event_row("W", &e),
            "W,20.0,0.001000,EVT_LOG,,,,,,,\"a; b c\"\n"
        );
        assert_eq!(fixed(f64::INFINITY, 2), "Inf");
        assert_eq!(fixed(f64::NEG_INFINITY, 2), "-Inf");
        assert_eq!(fixed(f64::NAN, 2), "NaN");
        assert_eq!(fixed(-0.0, 1), "-0.0");
    }

    #[test]
    fn log_names() {
        let dir = tempfile::tempdir().expect("temp dir");
        let name = new_log_name(dir.path(), "");
        let file = name.file_name().and_then(|n| n.to_str()).expect("name");
        assert!(
            file.starts_with("akta_") && file.ends_with(".csv") && file.len() == 24,
            "{file}"
        );
        assert_eq!(name.parent(), Some(dir.path()));
        assert!(
            new_log_name(Path::new(""), "x")
                .to_string_lossy()
                .starts_with("x_")
        );
        // never an existing file
        let a = unique_log_name(dir.path(), "p");
        std::fs::write(&a, "").expect("write");
        let b = unique_log_name(dir.path(), "p");
        assert_ne!(a, b);
        std::fs::write(&b, "").expect("write");
        let c = unique_log_name(dir.path(), "p");
        assert!(!c.exists());
        // (b and c end in _2/_3 unless the second changed in between)
        assert_eq!(
            raw_capture_path(Path::new("/d/x.csv")),
            Path::new("/d/x_raw.txt")
        );
        assert_eq!(
            log_side_file(Path::new("x.csv"), "_method_2.met"),
            Path::new("x_method_2.met")
        );
        assert_eq!(
            raw_capture_path(Path::new("x.txt")),
            Path::new("x.txt_raw.txt")
        );
    }

    #[test]
    fn create_writes_comment_and_header() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("a.csv");
        std::fs::write(&path, "old content\n").expect("write");
        let log = CsvLog::create(&path).expect("create");
        let text = std::fs::read_to_string(&log.path).expect("read");
        let (first, rest) = text.split_once('\n').expect("line");
        assert!(first.starts_with(CSV_COMMENT_PREFIX));
        assert_eq!(rest, CSV_HEADER);
        assert!(CsvLog::create(&dir.path().join("missing").join("a.csv")).is_err());
    }

    #[test]
    fn rows_are_on_disk_at_once() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = started_log(dir.path());
        app.push_trend(pt(100.0));
        app.push_event(EventKind::Mark, "m");
        // still open: another reader sees both rows
        let back = load_csv_log(&app.log_filename).expect("load");
        assert_eq!((back.trend.len(), back.events.len()), (1, 1));
    }

    #[test]
    fn close_keeps_a_log_with_data() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = started_log(dir.path());
        app.push_event(EventKind::Log, "Flow 1.0 ml/min");
        let raw = raw_capture_path(&app.log_filename);
        app.raw = Some(File::create(&raw).expect("raw"));
        app.close_log();
        assert!(app.log.is_none() && app.raw.is_none());
        assert!(app.log_filename.exists() && raw.exists());
        app.close_log(); // twice is fine
        assert!(app.log_filename.exists());
    }

    #[test]
    fn close_discards_an_empty_log() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = started_log(dir.path());
        app.push_event(EventKind::Info, "Sample: x");
        app.push_event(EventKind::Connect, "Akta connected");
        app.push_event(
            EventKind::Log,
            "Link: serial connection lost — reconnecting",
        );
        let raw = raw_capture_path(&app.log_filename);
        app.raw = Some(File::create(&raw).expect("raw"));
        app.close_log();
        assert!(!app.log_filename.exists() && !raw.exists());
        assert!(app.raw.is_none());
        // a trend point alone counts as data
        let mut app = started_log(dir.path());
        app.push_trend(pt(0.0));
        app.close_log();
        assert!(app.log_filename.exists());
    }

    #[test]
    fn roll_starts_a_new_log() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = started_log(dir.path());
        app.run_info_for_new_log.insert("Sample".into(), "S".into());
        app.run_info_for_new_log.insert("User".into(), " ".into());
        // no trend data yet: nothing happens
        app.push_event(EventKind::Connect, "Akta connected");
        let first = app.log_filename.clone();
        assert!(!app.roll_log());
        assert_eq!(app.log_filename, first);
        assert_eq!(app.events.len(), 1);

        app.inst.cum_time_ms = 500.0;
        app.inst.fraction_vol_ml = 1.0;
        app.update_fraction_state();
        app.push_trend(pt(500.0));
        app.inst.prog_name = "P".into();
        app.inst.run_state = "End".into();
        assert!(app.roll_log());
        assert_ne!(app.log_filename, first);
        assert!(app.log_filename.starts_with(dir.path()));
        // display cleared, run state and programme kept, run information written
        assert!(app.trend.is_empty() && app.fractions.is_empty());
        assert_eq!(app.inst.cum_time_ms, 0.0);
        assert_eq!(app.inst.current_fraction_idx, None);
        assert_eq!(
            (app.inst.prog_name.as_str(), app.inst.run_state.as_str()),
            ("P", "End")
        );
        let texts: Vec<&str> = app.events.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, ["Sample: S"]);
        app.push_trend(pt(10.0));
        app.close_log();

        let old = load_csv_log(&first).expect("first");
        assert_eq!((old.trend.len(), old.events.len()), (1, 1));
        let new = load_csv_log(&app.log_filename).expect("second");
        assert_eq!(new.trend.len(), 1);
        assert_eq!(new.events[0].text, "Sample: S");
    }

    #[test]
    fn start_run_records_the_method_setup_once() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = started_log(dir.path());
        let first = app.log_filename.clone();
        app.run_info_for_new_log
            .insert("Column".into(), "HisTrap 5 ml".into());
        app.run_info_for_new_log
            .insert(crate::runinfo::METHOD_REF_KEY.into(), "005 – Ni".into());
        // first run after connecting: no data yet, so no new file, but the setup is recorded
        app.start_run();
        assert_eq!(app.log_filename, first);
        assert_eq!(app.run_serial, 1);
        assert!(app.run_info_for_new_log.is_empty());
        let texts: Vec<&str> = app.events.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, ["PC method: 005 – Ni", "Column: HisTrap 5 ml"]);
        // the next run (no method dump): a new log without any setup
        app.push_trend(pt(10.0));
        app.start_run();
        assert_ne!(app.log_filename, first);
        assert_eq!(app.run_serial, 2);
        assert!(app.events.is_empty());
        app.push_trend(pt(1.0));
        app.close_log();
        let new = load_csv_log(&app.log_filename).expect("second");
        assert!(new.events.is_empty());
    }

    /// A finished log gets run information appended; every line already in the
    /// file stays as it was, and the reader takes the new values.
    #[test]
    fn append_run_info_to_a_finished_log() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = started_log(dir.path());
        app.log_run_info(
            &[("Sample", "Lysozyme"), ("Column volume (mL)", "5")]
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .into(),
            false,
        );
        app.inst.cum_time_ms = 1000.0;
        app.inst.cum_vol_ml = 0.5;
        app.push_trend(pt(1000.0));
        app.push_event(EventKind::Mark, "peak");
        app.inst.cum_time_ms = 2000.0;
        app.push_trend(TrendPoint {
            cum_time_ms: 2000.0,
            cum_vol_ml: 1.25,
            ..TrendPoint::default()
        });
        app.close_log();
        let path = app.log_filename.clone();
        let before = std::fs::read(&path).expect("read");

        let mut viewer = load_csv_log(&path).expect("load");
        assert_eq!(viewer.log_end_position(), (2000.0, 1.25));
        let edit: BTreeMap<String, String> = [
            ("Sample", "Lysozyme"),        // unchanged: not written again
            ("Column volume (mL)", "2.5"), // changed
            ("Notes", " a, b\nc "),        // new
            ("User", ""),                  // empty: nothing
        ]
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .into();
        assert_eq!(viewer.append_run_info(&edit).expect("append"), 2);
        assert_eq!(viewer.append_run_info(&edit).expect("again"), 0);
        let info = run_info(&viewer.events);
        assert_eq!(info["Column volume (mL)"], "2.5");
        assert_eq!(info["Notes"], "a, b\nc");

        let after = std::fs::read(&path).expect("read");
        assert_eq!(
            &after[..before.len()],
            &before[..],
            "existing lines untouched"
        );
        let added = String::from_utf8(after[before.len()..].to_vec()).expect("utf8");
        let rows: Vec<&str> = added.lines().collect();
        assert_eq!(rows.len(), 2);
        let re = Regex::new(r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(\.\d{3})?,").expect("re");
        assert!(rows.iter().all(|r| re.is_match(r)), "{rows:?}");
        assert!(
            rows[0].ends_with(",2000.0,1.250000,EVT_INFO,,,,,,,\"Column volume (mL): 2.5\""),
            "{rows:?}"
        );
        assert!(
            rows[1].ends_with(",2000.0,1.250000,EVT_INFO,,,,,,,\"Notes: a; b c\""),
            "{rows:?}"
        );

        // read back: the latest value of each field wins
        let back = load_csv_log(&path).expect("reload");
        assert_eq!(back.trend.len(), 2);
        let info = run_info(&back.events);
        assert_eq!(info["Sample"], "Lysozyme");
        assert_eq!(crate::runinfo::column_volume(&info), 2.5);
        assert_eq!(info["Notes"], "a; b c");
    }

    #[test]
    fn append_run_info_after_a_cut_off_row_and_errors() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("cut.csv");
        std::fs::write(
            &path,
            format!("# x\n{CSV_HEADER}w,1.0,0.1,TREND,1,2,3,4,5,6,\nw,2.0,0.2,TRE"),
        )
        .expect("write");
        let mut app = load_csv_log(&path).expect("load");
        let info: BTreeMap<String, String> = [("Sample".to_string(), "S".to_string())].into();
        assert_eq!(app.append_run_info(&info).expect("append"), 1);
        let back = load_csv_log(&path).expect("reload");
        assert_eq!(run_info(&back.events)["Sample"], "S");
        assert_eq!(back.events[0].cum_time_ms, 1.0);
        // a file that is gone
        app.log_filename = dir.path().join("gone.csv");
        let other: BTreeMap<String, String> = [("Sample".to_string(), "T".to_string())].into();
        assert!(app.append_run_info(&other).is_err());
        assert_eq!(run_info(&app.events)["Sample"], "S"); // nothing added in memory
    }

    #[test]
    fn roll_keeps_the_old_log_if_no_new_one_can_be_made() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = started_log(dir.path());
        app.push_trend(pt(1.0));
        app.log_dir = dir.path().join("gone");
        assert!(!app.roll_log());
        assert_eq!(app.trend.len(), 1);
        app.push_trend(pt(2.0));
        app.close_log();
        assert_eq!(
            load_csv_log(&app.log_filename).expect("load").trend.len(),
            2
        );
    }

    #[test]
    fn reader_edge_cases() {
        let (trend, events) = parse_csv_log(
            "# c\nwall_time,x\n\n  \r\nw,1,2,TREND,1,2,3,4,5,6\r\nw,1,2,EVT_MARK\nw,1,2,EVT_LOG,,,,,,,  \"a,b\"  \nw,1\n\
             w,x,2,EVT_LOG,,,,,,,\"bad time\"\nw,3,4,TREND,1,2,3,4,5\nw,5,6,EVT_X,,,,,,,\"?\"\nw,7,8,TREND,a,,3,4,5,6,",
        );
        assert_eq!(trend.len(), 2);
        assert_eq!(trend[0].temp_degc, 6.0);
        assert_eq!(
            (trend[1].uv, trend[1].cond_mscm, trend[1].ph),
            (0.0, 0.0, 3.0)
        );
        let texts: Vec<&str> = events.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, ["", "a,b"]);
        assert!(load_csv_log(Path::new("/nonexistent/x.csv")).is_err());
    }

    #[test]
    fn invalid_utf8_does_not_fail() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("x.csv");
        std::fs::write(
            &path,
            b"w,1,2,EVT_LOG,,,,,,,\"\xff\xfe ok\"\nw,1,2,TREND,1,2,3,4,5,6,\n",
        )
        .expect("write");
        let app = load_csv_log(&path).expect("load");
        assert_eq!((app.trend.len(), app.events.len()), (1, 1));
        assert!(app.events[0].text.ends_with(" ok"));
    }
}
