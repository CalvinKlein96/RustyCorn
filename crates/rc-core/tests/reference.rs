//! Tests against known-good reference outputs in `tests/data`. They are
//! fixed test data and guard the file formats: a change that alters any of
//! them breaks compatibility with existing logs and with other readers.
//!
//! * `writer_reference.csv` — the log written for the rows in
//!   `writer_inputs.txt`, then the run information of
//!   `writer_run_info_events` (see `writer_matches_reference`);
//! * `*.summary.txt` — what reading a CSV must give (counts, every event,
//!   fractions, final instrument state, an FNV hash of all trend values);
//! * `logtext_reference.txt`, `column_volume_reference.txt` — the log-text
//!   decoders on the inputs in `logtext_inputs.txt` and `column_volume_inputs.txt`;
//! * `demo_reference_run1.csv`, `demo_reference_run2_start.csv` — the demo source.
//!
//! Set `RC_REFERENCE_DIR` to a folder of further `<log>.csv` +
//! `<log>.csv.summary.txt` pairs to check those too.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use chrono::NaiveDate;
use rc_core::csvlog::{
    CSV_COMMENT_PREFIX, CSV_HEADER, CsvLog, event_row, load_csv_log, log_datetime, log_now,
    trend_row,
};
use rc_core::logtext;
use rc_core::{AktaEvent, AppState, EventKind, InstrumentState, TrendPoint};

fn data(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join(name)
}

fn read(name: &str) -> String {
    std::fs::read_to_string(data(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).expect("hex"))
        .collect()
}

fn unhex_str(s: &str) -> String {
    String::from_utf8(unhex(s)).expect("utf-8")
}

fn f(bits: &str) -> f64 {
    f64::from_bits(u64::from_str_radix(bits, 16).expect("bits"))
}

fn bits(x: f64) -> String {
    format!("{:016x}", x.to_bits())
}

/// `^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(\.\d{3})?$`
fn is_log_datetime(s: &str) -> bool {
    let b = s.as_bytes();
    let digits = |r: std::ops::Range<usize>| r.into_iter().all(|i| b[i].is_ascii_digit());
    let base = b.len() >= 19
        && digits(0..4)
        && b[4] == b'-'
        && digits(5..7)
        && b[7] == b'-'
        && digits(8..10)
        && b[10] == b'T'
        && digits(11..13)
        && b[13] == b':'
        && digits(14..16)
        && b[16] == b':'
        && digits(17..19);
    base && (b.len() == 19
        || (b.len() == 23 && b[19] == b'.' && digits(20..23) && &s[20..] != "000"))
}

/// The writer inputs: trend points and events in file order.
enum Row {
    Trend(TrendPoint),
    Event(AktaEvent),
}

fn writer_inputs() -> Vec<Row> {
    read("writer_inputs.txt")
        .lines()
        .map(|line| {
            let p: Vec<&str> = line.split(' ').collect();
            if p[0] == "T" {
                let v: Vec<f64> = p[1..9].iter().map(|s| f(s)).collect();
                Row::Trend(TrendPoint {
                    cum_time_ms: v[0],
                    cum_vol_ml: v[1],
                    uv: v[2],
                    cond_mscm: v[3],
                    ph: v[4],
                    pctb: v[5],
                    pres_mpa: v[6],
                    temp_degc: v[7],
                })
            } else {
                Row::Event(AktaEvent {
                    cum_time_ms: f(p[2]),
                    cum_vol_ml: f(p[3]),
                    kind: EventKind::from_csv_name(p[1]).expect("kind"),
                    text: unhex_str(p[4]),
                })
            }
        })
        .collect()
}

/// The run-information part of the writer reference.
fn writer_run_info_events() -> Vec<AktaEvent> {
    let map = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    };
    let mut app = AppState::new();
    app.inst.cum_time_ms = 1234.5;
    app.inst.cum_vol_ml = 0.25;
    app.log_run_info(
        &map(&[
            ("Sample", "  Lysozyme, 5 mg\n"),
            ("Port 3", "Tris\tpH 8"),
            ("Notes", "line1\nline2"),
            ("User", "  "),
            ("Column volume (mL)", "2,5"),
        ]),
        false,
    );
    app.log_run_info(&map(&[("Sample", "Lysozyme, 5 mg"), ("User", "C")]), true);
    app.events
}

/// Lines of a CSV with the wall-time field (and the comment line's time) removed.
fn without_wall_time(text: &str) -> Vec<String> {
    text.split_inclusive('\n')
        .map(|line| {
            if let Some(t) = line.strip_prefix(CSV_COMMENT_PREFIX) {
                assert!(is_log_datetime(t.trim_end()), "comment line {line:?}");
                CSV_COMMENT_PREFIX.to_string()
            } else if line == CSV_HEADER {
                line.to_string()
            } else {
                let (wall, rest) = line.split_once(',').expect("row");
                assert!(is_log_datetime(wall), "wall time {wall:?}");
                rest.to_string()
            }
        })
        .collect()
}

#[test]
fn datetime_text_format() {
    // 2026-10-20 03:04:05 with these milliseconds
    let expected = [
        (0, "2026-10-20T03:04:05"),
        (5, "2026-10-20T03:04:05.005"),
        (50, "2026-10-20T03:04:05.050"),
        (500, "2026-10-20T03:04:05.500"),
        (123, "2026-10-20T03:04:05.123"),
        (100, "2026-10-20T03:04:05.100"),
        (10, "2026-10-20T03:04:05.010"),
    ];
    for (ms, text) in expected {
        let t = NaiveDate::from_ymd_opt(2026, 10, 20)
            .and_then(|d| d.and_hms_milli_opt(3, 4, 5, ms))
            .expect("date");
        assert_eq!(log_datetime(&t), text);
    }
    // microseconds are truncated, not rounded (the format has ms resolution)
    let t = NaiveDate::from_ymd_opt(2026, 1, 2)
        .and_then(|d| d.and_hms_micro_opt(23, 59, 59, 999_999))
        .expect("date");
    assert_eq!(log_datetime(&t), "2026-01-02T23:59:59.999");
    let t = NaiveDate::from_ymd_opt(26, 1, 2)
        .and_then(|d| d.and_hms_milli_opt(3, 4, 5, 6))
        .expect("date");
    assert_eq!(log_datetime(&t), "0026-01-02T03:04:05.006");
    assert!(is_log_datetime(&log_now()));
    // and every time stamp in the reference logs has this shape
    for name in [
        "writer_reference.csv",
        "demo_reference_run1.csv",
        "akta_20261020_114429.csv",
    ] {
        without_wall_time(&read(name));
    }
}

#[test]
fn writer_matches_reference() {
    let reference = read("writer_reference.csv");
    let lines: Vec<&str> = reference.split_inclusive('\n').collect();
    assert!(lines[0].starts_with(CSV_COMMENT_PREFIX));
    assert_eq!(lines[1], CSV_HEADER);
    let rows = &lines[2..];

    let mut ours = Vec::new();
    for row in writer_inputs() {
        ours.push(row);
    }
    let info = writer_run_info_events();
    assert_eq!(rows.len(), ours.len() + info.len());
    // Same rows byte for byte when given the reference's wall time.
    for (i, reference_row) in rows.iter().enumerate() {
        let wall = reference_row.split(',').next().expect("wall");
        let row = match ours.get(i) {
            Some(Row::Trend(p)) => trend_row(wall, p),
            Some(Row::Event(e)) => event_row(wall, e),
            None => event_row(wall, &info[i - ours.len()]),
        };
        assert_eq!(&row, reference_row, "row {i}");
    }

    // And through a real file: identical apart from the time stamps.
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("rust.csv");
    let mut app = AppState::new();
    app.log = Some(CsvLog::create(&path).expect("create"));
    for row in writer_inputs() {
        match row {
            Row::Trend(p) => app.push_trend(p),
            Row::Event(e) => {
                app.inst.cum_time_ms = e.cum_time_ms;
                app.inst.cum_vol_ml = e.cum_vol_ml;
                app.push_event(e.kind, e.text);
            }
        }
    }
    for e in writer_run_info_events() {
        app.inst.cum_time_ms = e.cum_time_ms;
        app.inst.cum_vol_ml = e.cum_vol_ml;
        app.push_event(e.kind, e.text);
    }
    app.close_log();
    let written = std::fs::read_to_string(&path).expect("read");
    assert_eq!(without_wall_time(&written), without_wall_time(&reference));
}

/// Compare `load_csv_log` with the reference summary of the same file.
fn check_summary(csv: &Path, summary: &str) {
    let app = load_csv_log(csv).expect("load");
    let name = csv.display();
    let mut events = app.events.iter();
    let mut fractions = app.fractions.iter();
    for line in summary.lines() {
        let p: Vec<&str> = line.split(' ').collect();
        match p[0] {
            "trend" => assert_eq!(
                app.trend.len(),
                p[1].parse::<usize>().expect("n"),
                "{name} trend"
            ),
            "trend_hash" => {
                let mut h: u64 = 0xcbf29ce484222325;
                for t in &app.trend {
                    for x in [
                        t.cum_time_ms,
                        t.cum_vol_ml,
                        t.uv,
                        t.cond_mscm,
                        t.ph,
                        t.pctb,
                        t.pres_mpa,
                        t.temp_degc,
                    ] {
                        h = (h ^ x.to_bits()).wrapping_mul(0x100000001b3);
                    }
                }
                assert_eq!(format!("{h:016x}"), p[1], "{name} trend values");
            }
            "events" => assert_eq!(
                app.events.len(),
                p[1].parse::<usize>().expect("n"),
                "{name} events"
            ),
            "event" => {
                let e = events.next().expect("event");
                assert_eq!(e.kind.csv_name(), p[1], "{name}");
                assert_eq!(bits(e.cum_time_ms), p[2], "{name} {e:?}");
                assert_eq!(bits(e.cum_vol_ml), p[3], "{name} {e:?}");
                assert_eq!(e.text, unhex_str(&p[4][1..]), "{name}");
            }
            "fractions" => {
                assert_eq!(
                    app.fractions.len(),
                    p[1].parse::<usize>().expect("n"),
                    "{name} fractions"
                )
            }
            "fraction" => {
                let fr = fractions.next().expect("fraction");
                let opt = |x: Option<f64>| x.map_or("nothing".to_string(), bits);
                assert_eq!(
                    [
                        fr.fraction_number.to_string(),
                        bits(fr.start_time_ms),
                        bits(fr.start_vol_ml),
                        opt(fr.end_time_ms),
                        opt(fr.end_vol_ml)
                    ],
                    p[1..6],
                    "{name}"
                );
            }
            "inst" => {
                let s = &app.inst;
                let idx = s
                    .current_fraction_idx
                    .map_or("nothing".to_string(), |i| i.to_string());
                assert_eq!(
                    [
                        bits(s.cum_time_ms),
                        bits(s.cum_vol_ml),
                        s.fraction_number.to_string(),
                        bits(s.fraction_vol_ml),
                        idx
                    ],
                    p[1..6],
                    "{name} instrument state"
                );
            }
            other => panic!("unknown summary line {other}"),
        }
    }
    assert!(events.next().is_none() && fractions.next().is_none());
    assert_eq!(app.log_filename, csv);
    assert!(app.log.is_none());
}

#[test]
fn reader_matches_reference() {
    for name in [
        "synthetic_log.csv",
        "writer_reference.csv",
        "akta_20261020_114429.csv",
        "akta_20261020_120938.csv",
        "akta_20261020_134212.csv",
    ] {
        let summary = match name {
            "synthetic_log.csv" => "synthetic_log.summary.txt".to_string(),
            "writer_reference.csv" => "writer_reference.summary.txt".to_string(),
            _ => format!("{name}.summary.txt"),
        };
        check_summary(&data(name), &read(&summary));
    }
}

/// Optional: more logs with reference summaries in `$RC_REFERENCE_DIR`.
#[test]
fn reader_matches_reference_on_more_logs() {
    let Some(dir) = std::env::var_os("RC_REFERENCE_DIR") else {
        return;
    };
    let mut n = 0;
    for entry in std::fs::read_dir(dir).expect("dir") {
        let path = entry.expect("entry").path();
        if path.extension().is_some_and(|e| e == "csv") {
            let mut s = path.clone().into_os_string();
            s.push(".summary.txt");
            if let Ok(summary) = std::fs::read_to_string(&s) {
                check_summary(&path, &summary);
                n += 1;
            }
        }
    }
    eprintln!("checked {n} logs against their summaries");
    assert!(n > 0);
}

#[test]
fn round_trip_keeps_values() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut app = AppState::new();
    app.log_dir = dir.path().to_path_buf();
    app.log_prefix = "test".into();
    app.log_filename = rc_core::csvlog::new_log_name(&app.log_dir, &app.log_prefix);
    assert_eq!(app.log_filename.parent(), Some(dir.path()));
    app.log = Some(CsvLog::create(&app.log_filename).expect("create"));
    let pt = |t: f64, uv: f64| TrendPoint {
        cum_time_ms: t * 60_000.0,
        cum_vol_ml: t / 2.0,
        uv,
        cond_mscm: 10.0,
        ph: 7.0,
        pctb: 0.0,
        pres_mpa: 0.3,
        temp_degc: 20.0,
    };
    let points: Vec<TrendPoint> = (0..=20)
        .map(|i| pt(i as f64 / 10.0, (i as f64 / 10.0).sin()))
        .collect();
    for p in &points {
        app.push_trend(*p);
    }
    app.push_event(EventKind::Mark, "sample loaded");
    app.push_event(EventKind::Info, "Sample: Lysozyme");
    app.close_log();
    let back = load_csv_log(&app.log_filename).expect("load");
    assert_eq!(back.trend.len(), points.len());
    assert!((back.trend[20].uv - points[20].uv).abs() < 1e-6);
    let kinds: Vec<EventKind> = back.events.iter().map(|e| e.kind).collect();
    assert_eq!(kinds, [EventKind::Mark, EventKind::Info]);
}

#[test]
fn logtext_matches_reference() {
    for line in read("logtext_reference.txt").lines() {
        let (hex, rest) = line.split_once(' ').expect("line");
        let s = unhex_str(hex);
        let fields: BTreeMap<&str, Vec<&str>> = rest
            .split(' ')
            .map(|kv| {
                let (k, v) = kv.split_once('=').expect("k=v");
                (k, v.split(':').collect())
            })
            .collect();
        let flow = logtext::flow_from_log(&s).map_or("nothing".to_string(), bits);
        assert_eq!(flow, fields["flow"][0], "flow {s:?}");

        let mut i1 = InstrumentState::default();
        let inj = logtext::set_inject_from_log(&s, &mut i1);
        assert_eq!(
            [inj.to_string(), i1.inject_value.name().to_string()],
            fields["inject"][..],
            "inject {s:?}"
        );

        let mut i2 = InstrumentState::default();
        let buf = logtext::set_buffer_valve_from_log(&s, &mut i2);
        assert_eq!(
            [
                buf.to_string(),
                i2.buffer_port.to_string(),
                i2.buffer_port_known.to_string()
            ],
            fields["buffer"][..],
            "buffer {s:?}"
        );

        let mut app = AppState::new();
        let frac = logtext::set_fraction_from_log(&s, &mut app);
        let f = &fields["fraction"];
        // "error": the regex accepts a non-ASCII digit that is not a number; such
        // a text is ignored (no fraction change).
        let want = if f[0] == "error" { "false" } else { f[0] };
        assert_eq!(
            [
                frac.to_string(),
                bits(app.inst.fraction_vol_ml),
                app.inst.fraction_number.to_string(),
                app.fractions.len().to_string()
            ],
            [want, f[1], f[2], f[3]],
            "fraction {s:?}"
        );

        let rs = &fields["runstart"];
        assert_eq!(logtext::is_run_start(&s, "Run").to_string(), rs[0], "{s:?}");
        assert_eq!(logtext::is_run_start("End", &s).to_string(), rs[1], "{s:?}");
    }
}

#[test]
fn column_volume_matches_reference() {
    for line in read("column_volume_reference.txt").lines() {
        let (hex, b) = line.split_once(' ').expect("line");
        let info = BTreeMap::from([("Column volume (mL)".to_string(), unhex_str(hex))]);
        assert_eq!(bits(rc_core::runinfo::column_volume(&info)), b, "{info:?}");
    }
}

#[test]
fn demo_matches_reference() {
    let dir = tempfile::tempdir().expect("temp dir");
    let first = dir.path().join("demo_first.csv");
    let mut state = AppState::new();
    state.log_dir = dir.path().to_path_buf();
    state.log_prefix = "demo".into();
    state.log = Some(CsvLog::create(&first).expect("create"));
    state.log_filename = first.clone();
    let app = state.shared();
    let shutdown = Arc::new(AtomicBool::new(false));
    let thread = {
        let (app, shutdown) = (app.clone(), shutdown.clone());
        std::thread::spawn(move || {
            rc_core::demo::run_demo_with(app, shutdown, Duration::from_millis(1), Duration::ZERO)
        })
    };
    let run2_reference = read("demo_reference_run2_start.csv");
    let run2_trend = run2_reference.matches(",TREND,").count();
    let t0 = Instant::now();
    let second = loop {
        {
            let a = app.lock();
            if a.log_filename != first && a.trend.len() >= run2_trend {
                break a.log_filename.clone();
            }
        }
        assert!(t0.elapsed() < Duration::from_secs(60), "no second run");
        std::thread::sleep(Duration::from_millis(1));
    };
    shutdown.store(true, Ordering::Relaxed);
    thread.join().expect("demo thread");
    app.lock().close_log();

    // the first run in full
    let ours = std::fs::read_to_string(&first).expect("read");
    let reference = read("demo_reference_run1.csv");
    assert_eq!(without_wall_time(&ours), without_wall_time(&reference));
    // the second run starts like the reference
    let ours = without_wall_time(&std::fs::read_to_string(&second).expect("read"));
    let reference = without_wall_time(&run2_reference);
    assert!(ours.len() >= reference.len());
    assert_eq!(ours[..reference.len()], reference[..]);
}
