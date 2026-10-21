//! A whole recording, end to end: the acquisition runs the real link layer on
//! a simulated ÄKTAprime (sim/), which loads a PC method that has a run setup,
//! starts a run of it (method dump, run state), sends trend data, log
//! messages and a fraction; a mark is added as from the GUI. After the
//! acquisition stops, the CSV it wrote must follow the log format exactly
//! and read back (load_csv_log) to the same points, events, fractions and run
//! information. No serial port is ever opened, and the user's settings are
//! never read or written.

mod sim;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use rc_core::csvlog::{
    CSV_COMMENT_PREFIX, CSV_HEADER, escape_message, load_csv_log, log_side_file, raw_capture_path,
};
use rc_core::opc::*;
use rc_core::runinfo::{METHOD_REF_KEY, column_volume, run_info};
use rc_core::settings::{Settings, set_config_dir_override};
use rc_core::{AppState, EventKind, SharedApp};
use rc_protocol::Acquisition;
use rc_protocol::link::run_linklayer_with;
use sim::{METHOD_DUMP, Sim, TREND, fast, opener, payload, wait_until};

fn run_state(state: u8) -> Vec<u8> {
    vec![0, 0, 0, 0, 0, 0, 0, 0, state, 0x00]
}

fn log_single(text: &str) -> Vec<u8> {
    payload(&[(OPC_LOG_SINGLE, text.as_bytes())])
}

fn has_event(app: &SharedApp, kind: EventKind, text: &str) -> bool {
    app.lock()
        .events
        .iter()
        .any(|e| e.kind == kind && e.text == text)
}

/// A log time stamp: `2026-10-21T09:15:02` (ISO 8601, no zone) with optional `.mmm`.
fn is_log_datetime(s: &str) -> bool {
    let b = s.as_bytes();
    let digits = |r: std::ops::Range<usize>| {
        r.into_iter()
            .all(|i| b.get(i).is_some_and(u8::is_ascii_digit))
    };
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
    base && (b.len() == 19 || (b.len() == 23 && b[19] == b'.' && digits(20..23)))
}

/// `%.Nf` text: digits, a point and exactly `n` decimals (optionally negative).
fn is_fixed(s: &str, n: usize) -> bool {
    let s = s.strip_prefix('-').unwrap_or(s);
    match s.split_once('.') {
        Some((int, frac)) => {
            !int.is_empty()
                && int.bytes().all(|c| c.is_ascii_digit())
                && frac.len() == n
                && frac.bytes().all(|c| c.is_ascii_digit())
        }
        None => false,
    }
}

/// Every line of a log in the log format; returns the number of rows.
fn check_csv_format(path: &Path) -> usize {
    let text = std::fs::read_to_string(path).unwrap();
    assert!(!text.contains('\r'), "LF line ends only");
    assert!(text.ends_with('\n'), "every row ends with a newline");
    let mut lines = text.lines();
    let first = lines.next().unwrap();
    let started = first
        .strip_prefix(CSV_COMMENT_PREFIX)
        .unwrap_or_else(|| panic!("comment line: {first:?}"));
    assert!(is_log_datetime(started), "start time {started:?}");
    assert_eq!(format!("{}\n", lines.next().unwrap()), CSV_HEADER);
    let mut rows = 0;
    for line in lines {
        rows += 1;
        let f: Vec<&str> = line.splitn(11, ',').collect();
        assert_eq!(f.len(), 11, "{line:?}");
        assert!(is_log_datetime(f[0]), "wall time in {line:?}");
        assert!(
            is_fixed(f[1], 1) && is_fixed(f[2], 6),
            "time/volume in {line:?}"
        );
        if f[3] == "TREND" {
            let decimals = [8, 4, 2, 2, 4, 2];
            for (i, n) in decimals.into_iter().enumerate() {
                assert!(is_fixed(f[4 + i], n), "field {} of {line:?}", 4 + i);
            }
            assert_eq!(f[10], "", "{line:?}");
        } else {
            assert!(
                EventKind::from_csv_name(f[3]).is_some(),
                "row type in {line:?}"
            );
            assert!(f[4..10].iter().all(|x| x.is_empty()), "{line:?}");
            // the message is quoted and has no comma of its own (written as ';')
            let msg = f[10];
            assert!(
                msg.len() >= 2 && msg.starts_with('"') && msg.ends_with('"'),
                "{line:?}"
            );
            assert!(!msg.contains(','), "{line:?}");
        }
    }
    rows
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn record_a_run_and_read_it_back() {
    let tmp = tempfile::tempdir().unwrap();
    set_config_dir_override(Some(tmp.path().join("config"))); // never the user's settings
    let data = tmp.path().join("data");
    let settings = Settings {
        demo_mode: false,
        serial_port: "SIM".to_string(), // the source below replaces the link on a real port
        volume_unit_ul: 10.0,
        log_prefix: "e2e".to_string(),
        data_dir: data.to_string_lossy().into_owned(),
        save_raw: true,
        ..Settings::default()
    };

    // Two PC methods with the dump's content; the Akta loads 9, so the run is 9.
    let method_dir = rc_core::settings::method_dir(&data);
    std::fs::create_dir_all(&method_dir).unwrap();
    let text = rc_methods::unescape_method_text(METHOD_DUMP.as_bytes());
    for n in [4, 9] {
        std::fs::write(rc_methods::method_path(&method_dir, n), &text).unwrap();
    }
    rc_methods::set_method_name(&method_dir, 9, "Q column, step").unwrap();
    let setup: BTreeMap<String, String> = [
        ("Column", "HiTrap Q HP 5 ml"),
        ("Column volume (mL)", "5"),
        ("Buffer B", "1 M NaCl, pH 8"),
        ("Port 1", "Buffer A"),
    ]
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .into();
    rc_methods::set_method_setup(&method_dir, 9, &setup).unwrap();
    rc_methods::set_method_setup(
        &method_dir,
        4,
        &BTreeMap::from([("Column".into(), "wrong".into())]),
    )
    .unwrap();

    // What the Akta sends once the driver is enabled.
    let mut script = vec![
        payload(&[(OPC_BASE_TIME_VOL, &[0x10, 0x27, 0, 0, 0xe8, 0x03, 0, 0])]),
        payload(&[(OPC_METHOD_LOAD, &[9, 0])]),
        payload(&[(OPC_METHOD_DUMP_BEGIN, &[])]),
    ];
    for chunk in METHOD_DUMP.as_bytes().chunks(30 * 4) {
        let msgs: Vec<(u16, &[u8])> = chunk.chunks(30).map(|c| (OPC_METHOD_LINE, c)).collect();
        script.push(payload(&msgs));
    }
    script.push(payload(&[(OPC_METHOD_DUMP_END, &[0x00])]));
    script.push(payload(&[(OPC_RUN_STATE, &run_state(0x01))]));
    for _ in 0..3 {
        script.push(payload(&[(OPC_TREND_DATA, &TREND)]));
    }
    script.push(log_single("Fraction size 1.0 ml"));
    for _ in 0..3 {
        script.push(payload(&[(OPC_TREND_DATA, &TREND)]));
    }
    script.push(payload(&[
        (OPC_LOG_BEGIN, b"Feed Tube "),
        (OPC_LOG_END, b"Nr 2"),
    ]));
    for _ in 0..2 {
        script.push(payload(&[(OPC_TREND_DATA, &TREND)]));
    }
    script.push(log_single("Fraction size 0.0 ml"));
    script.push(payload(&[(OPC_TREND_DATA, &TREND)]));
    script.push(log_single("Buffer Valve Pos 3"));

    let sim = Arc::new(Mutex::new(Sim {
        on_enable: script,
        ..Sim::default()
    }));
    let app = AppState::default().shared();
    let mut acq = Acquisition::new(app.clone(), settings);
    {
        let sim = sim.clone();
        acq.start_with_source(move |app, shutdown| {
            run_linklayer_with(&app, &shutdown, "SIM", opener(&sim), &fast());
        });
    }
    let first_log = app.lock().log_filename.clone();
    assert!(first_log.starts_with(&data));

    wait_until("the script", 20, || {
        has_event(&app, EventKind::Log, "Buffer Valve Pos 3")
    });
    assert_eq!(app.lock().last_loaded_method, 9);
    // the GUI: a mark, and the sample and user asked for at the run start
    app.lock().push_event(EventKind::Mark, "Peak 1, top");
    let per_run: BTreeMap<String, String> = [("Sample", "Lysate, clarified"), ("User", "Alex")]
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .into();
    app.lock().log_run_info(&per_run, true);

    // A second run without a method dump: a new log, and no method setup in it.
    {
        let mut s = sim.lock();
        s.outbox
            .push_back(payload(&[(OPC_RUN_STATE, &run_state(0x00))]));
        s.outbox
            .push_back(payload(&[(OPC_RUN_STATE, &run_state(0x01))]));
        s.outbox.push_back(payload(&[(OPC_TREND_DATA, &TREND)]));
    }
    wait_until("the second run", 20, || {
        let a = app.lock();
        a.log_filename != first_log && a.trend.len() == 1
    });
    let second_log = app.lock().log_filename.clone();
    acq.stop();
    assert!(!acq.is_running());
    // what was shown live when it stopped: the second run
    let live = app.lock();
    assert_eq!(live.status, "Stopped");
    assert!(live.log.is_none() && live.raw.is_none() && live.log_error.is_none());
    assert_eq!(live.run_serial, 2);
    assert_eq!(live.trend.len(), 1);

    // ── The first run's log ─────────────────────────────────────────────────────
    let rows = check_csv_format(&first_log);
    let back = load_csv_log(&first_log).unwrap();
    assert_eq!(
        rows,
        back.trend.len() + back.events.len(),
        "every row reads back"
    );
    assert_eq!(back.trend.len(), 9);
    for (i, p) in back.trend.iter().enumerate() {
        assert!(
            close(p.cum_time_ms, 100_000.0 + 100.0 * (i + 1) as f64, 0.05),
            "{p:?}"
        );
        assert_eq!(p.uv, 1.0);
    }
    let texts: Vec<(EventKind, &str)> = back
        .events
        .iter()
        .map(|e| (e.kind, e.text.as_str()))
        .collect();
    let pos = |k: EventKind, t: &str| {
        texts
            .iter()
            .position(|x| *x == (k, t))
            .unwrap_or_else(|| panic!("{t:?} in {texts:?}"))
    };
    // connected → method setup → method start → run → data events → mark → sample
    let connected = pos(EventKind::Connect, "Akta connected");
    let reference = pos(EventKind::Info, "PC method: 009 – Q column; step");
    let column = pos(EventKind::Info, "Column: HiTrap Q HP 5 ml");
    let method_start = texts
        .iter()
        .position(|(k, t)| *k == EventKind::Log && t.starts_with("Method start:"))
        .unwrap();
    let run = pos(EventKind::RunState, "Run state: Run  inject=IV_LOAD");
    let frac = pos(EventKind::Log, "Fraction size 1.0 ml");
    let mark = pos(EventKind::Mark, "Peak 1; top");
    let sample = pos(EventKind::Info, "Sample: Lysate; clarified");
    assert!(
        connected < reference && reference < column && column < method_start,
        "{texts:?}"
    );
    assert!(
        method_start < run && run < frac && frac < mark && mark < sample,
        "{texts:?}"
    );
    assert!(texts.contains(&(EventKind::Log, "Feed Tube Nr 2")));
    assert!(
        !texts.iter().any(|(_, t)| t.contains("wrong")),
        "method 4's setup was not used"
    );

    // run information: the method setup, the PC method and the user's fields
    let info = run_info(&back.events);
    assert_eq!(info[METHOD_REF_KEY], "009 – Q column; step");
    assert_eq!(info["Column"], "HiTrap Q HP 5 ml");
    assert_eq!(info["Buffer B"], "1 M NaCl; pH 8");
    assert_eq!(info["Port 1"], "Buffer A");
    assert_eq!(info["Sample"], "Lysate; clarified");
    assert_eq!(info["User"], "Alex");
    assert_eq!(column_volume(&info), 5.0);

    // fractions: tube 1 from "Fraction size 1.0", tube 2 from the tube change, closed by size 0
    assert_eq!(back.fractions.len(), 2, "{:?}", back.fractions);
    let (f1, f2) = (back.fractions[0], back.fractions[1]);
    assert_eq!((f1.fraction_number, f2.fraction_number), (1, 2));
    assert!(
        close(f1.start_time_ms, 100_300.0, 0.05) && close(f1.end_time_ms.unwrap(), 100_600.0, 0.05)
    );
    assert!(
        close(f2.start_time_ms, 100_600.0, 0.05) && close(f2.end_time_ms.unwrap(), 100_800.0, 0.05)
    );
    assert!(back.inst.current_fraction_idx.is_none());

    // ── The second run's log: its own data, no method setup ──────────────────────
    check_csv_format(&second_log);
    let back2 = load_csv_log(&second_log).unwrap();
    assert_eq!(back2.trend.len(), live.trend.len());
    for (a, b) in back2.trend.iter().zip(&live.trend) {
        assert!(
            close(a.cum_time_ms, b.cum_time_ms, 0.05) && close(a.cum_vol_ml, b.cum_vol_ml, 1e-6)
        );
        assert!(
            close(a.uv, b.uv, 1e-8)
                && close(a.cond_mscm, b.cond_mscm, 1e-4)
                && close(a.ph, b.ph, 0.005)
        );
        assert!(
            close(a.pctb, b.pctb, 0.005)
                && close(a.pres_mpa, b.pres_mpa, 1e-4)
                && close(a.temp_degc, b.temp_degc, 0.005)
        );
    }
    assert_eq!(back2.events.len(), live.events.len());
    for (a, b) in back2.events.iter().zip(&live.events) {
        assert_eq!(
            (a.kind, a.text.as_str()),
            (b.kind, escape_message(&b.text).as_str())
        );
        assert!(
            close(a.cum_time_ms, b.cum_time_ms, 0.05) && close(a.cum_vol_ml, b.cum_vol_ml, 1e-6)
        );
    }
    assert!(run_info(&back2.events).is_empty(), "{:?}", back2.events);

    // ── Raw capture: the instrument messages and the method dump ─────────────────
    let raw = std::fs::read_to_string(raw_capture_path(&first_log)).unwrap();
    assert!(
        raw.contains("  0x8004 ") && raw.contains("  TX 0x"),
        "{raw}"
    );
    let met = std::fs::read(log_side_file(&first_log, "_method_1.met")).unwrap();
    assert_eq!(met, METHOD_DUMP.as_bytes());
    drop(live);
    set_config_dir_override(None);
}
