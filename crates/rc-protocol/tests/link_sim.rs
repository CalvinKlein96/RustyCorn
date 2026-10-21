//! End-to-end tests of the link layer against a simulated ÄKTAprime (sim/):
//! the connect handshake, sequence numbers, silence, restarts and port
//! failures. No serial port is ever opened.

mod sim;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use rc_core::opc::*;
use rc_core::{AppState, EventKind, SharedApp};
use rc_protocol::frame::*;
use rc_protocol::link::{LinkConfig, run_linklayer_with};
use sim::{METHOD_DUMP, Sim, TREND, fast, opener, payload, wait_until};

fn has_event(app: &SharedApp, kind: EventKind, text: &str) -> bool {
    app.lock()
        .events
        .iter()
        .any(|e| e.kind == kind && e.text == text)
}

fn event_count(app: &SharedApp, kind: EventKind, text: &str) -> usize {
    app.lock()
        .events
        .iter()
        .filter(|e| e.kind == kind && e.text == text)
        .count()
}

fn write_methods(dir: &Path) -> String {
    let long = format!(
        "Method Type        ,{}",
        "Long name with a z and #, ".repeat(3)
    );
    std::fs::write(rc_methods::method_path(dir, 3), "x\r\n").unwrap();
    std::fs::write(
        rc_methods::method_path(dir, 5),
        format!("*****\r\n{long}\r\n\r\n,1   ,0.0\r\n"),
    )
    .unwrap();
    std::fs::write(rc_methods::method_path(dir, 12), "x\n").unwrap();
    long
}

/// The whole life of a connection, step by step.
#[test]
fn link_against_simulated_akta() {
    let data = tempfile::tempdir().unwrap();
    let method_dir = data.path().join("methods");
    std::fs::create_dir_all(&method_dir).unwrap();
    let long = write_methods(&method_dir);
    // the method the dump below reports, stored as PC method 8 with a run setup
    // (the setup file must not show up as a method on the Akta)
    std::fs::write(
        rc_methods::method_path(&method_dir, 8),
        rc_methods::unescape_method_text(METHOD_DUMP.as_bytes()),
    )
    .unwrap();
    let setup = [
        ("Column", "HiTrap Q 1 ml"),
        ("Column volume (mL)", "1"),
        ("Port 1", "Buffer A"),
    ]
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .into();
    rc_methods::set_method_setup(&method_dir, 8, &setup).unwrap();

    let app = AppState {
        volume_unit_ul: 10.0,
        log_dir: data.path().to_path_buf(),
        log_prefix: "sim".to_string(),
        log_filename: data.path().join("sim_start.csv"),
        method_dir: method_dir.clone(),
        ..AppState::default()
    };
    let app = app.shared();
    {
        let mut a = app.lock();
        a.log = Some(rc_core::csvlog::CsvLog::create(&a.log_filename).unwrap());
    }

    // What the Akta sends once the driver is enabled: base, a method dump, the
    // run start, trend data, a split log message, a tube change, and method
    // store requests (list, load, save).
    let mut on_enable = vec![
        payload(&[(OPC_BASE_TIME_VOL, &[0x10, 0x27, 0, 0, 0xe8, 0x03, 0, 0])]),
        payload(&[(OPC_METHOD_DUMP_BEGIN, &[])]),
    ];
    let dump = METHOD_DUMP.as_bytes();
    for chunk in dump.chunks(30 * 4) {
        let msgs: Vec<(u16, &[u8])> = chunk.chunks(30).map(|c| (OPC_METHOD_LINE, c)).collect();
        on_enable.push(payload(&msgs));
    }
    on_enable.push(payload(&[(OPC_METHOD_DUMP_END, &[0x00])]));
    on_enable.push(payload(&[(
        OPC_RUN_STATE,
        &[0, 0, 0, 0, 0, 0, 0, 0, 0x01, 0x00],
    )]));
    for _ in 0..4 {
        on_enable.push(payload(&[(OPC_TREND_DATA, &TREND)]));
    }
    on_enable.push(payload(&[
        (OPC_TREND_DATA, &TREND),
        (OPC_LOG_BEGIN, b"Buffer Valve "),
        (OPC_LOG_CONT, b"Pos"),
    ]));
    on_enable.push(payload(&[(OPC_LOG_END, b" 3"), (OPC_WATCHDOG, &[])]));
    on_enable.push(payload(&[(OPC_TUBEFEED, &[0, 0, 0, 0, 0, 0, 0, 0, 2, 0])]));
    on_enable.push(payload(&[(OPC_METHOD_LIST, &[0x01, 0x00, 0x01])]));
    on_enable.push(payload(&[(OPC_METHOD_LOAD, &[0x05, 0x00])]));
    on_enable.push(payload(&[(OPC_METHOD_SAVE_BEGIN, &[0x07, 0x00])]));
    for chunk in dump.chunks(30 * 4) {
        let msgs: Vec<(u16, &[u8])> = chunk.chunks(30).map(|c| (OPC_METHOD_LINE, c)).collect();
        on_enable.push(payload(&msgs));
    }
    on_enable.push(payload(&[(OPC_METHOD_SAVE_END, &[])]));

    let sim = Arc::new(Mutex::new(Sim {
        app: Some(app.clone()),
        fail_opens: 2,
        ignore_frames: 3,
        noise: true,
        on_enable,
        ..Sim::default()
    }));
    let shutdown = Arc::new(AtomicBool::new(false));
    let link = {
        let (app, sim, shutdown) = (app.clone(), sim.clone(), shutdown.clone());
        thread::spawn(move || run_linklayer_with(&app, &shutdown, "SIM", opener(&sim), &fast()))
    };

    // ── Port missing at first, then the connect handshake ──────────────────────
    wait_until("connected", 10, || {
        app.lock().status == "● Connected on SIM"
    });
    assert!(has_event(&app, EventKind::Connect, "Akta connected"));
    assert_eq!(sim.lock().opens, 3);
    {
        let s = sim.lock();
        let st = &s.statuses;
        let pos = |t: &str| {
            st.iter()
                .position(|x| x == t)
                .unwrap_or_else(|| panic!("{t:?} not in {st:?}"))
        };
        assert!(pos("Serial port SIM not available — retrying") < pos("Waiting for Akta on SIM"));
        // unanswered polls first, then UA; nothing but polls until connected
        assert!(s.pc_frames[..4].iter().all(|f| f == b"#0S5\r"));
    }

    // ── Identity request, version → driver enable, data, method store ──────────
    wait_until("method save acknowledged", 10, || {
        sim.lock()
            .received
            .iter()
            .any(|m| m.0 == OPC_METHOD_SAVE_ACK)
    });
    {
        let s = sim.lock();
        assert_eq!(s.received[0], (OPC_IDENTITY_REQUEST, vec![0x00]));
        assert_eq!(s.received[1], (OPC_IDENTITY_REQUEST, vec![0x01]));
        // the first I-frame of the PC carries SN 1 (SN is incremented before sending)
        let first_i = s
            .pc_frames
            .iter()
            .find(|f| (f[2] & 0xf0) == CTRL_IFRAME)
            .unwrap();
        assert_eq!((first_i[2] >> 2) & 0x03, 1);
        assert_eq!(s.bad_pc_frames, 0);

        // list: ten numbers from 1 up
        let list = s
            .received
            .iter()
            .find(|m| m.0 == OPC_METHOD_LIST_REPLY)
            .unwrap();
        let nums: Vec<u16> = list
            .1
            .chunks(2)
            .map(|c| c[0] as u16 | ((c[1] as u16) << 8))
            .collect();
        assert_eq!(
            nums,
            [3, 5, 8, 12, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff]
        );

        // load: 0x2013, the lines (long ones in parts), 0x2014 [0]
        let begin = s
            .received
            .iter()
            .position(|m| m.0 == OPC_METHOD_LOAD_BEGIN)
            .unwrap();
        let end = s
            .received
            .iter()
            .position(|m| m.0 == OPC_METHOD_LOAD_END)
            .unwrap();
        assert_eq!(s.received[end].1, vec![0x00]);
        let mut lines: Vec<Vec<u8>> = Vec::new();
        for (o, d) in &s.received[begin + 1..end] {
            if *o == OPC_METHOD_LINE || *o == OPC_METHOD_LINE_BEGIN {
                lines.push(Vec::new());
            }
            lines.last_mut().unwrap().extend_from_slice(d);
        }
        let lines: Vec<String> = lines
            .iter()
            .map(|l| String::from_utf8(rc_methods::unescape_method_text(l)).unwrap())
            .collect();
        assert_eq!(lines, ["*****", long.as_str(), ",1   ,0.0"]);

        assert_eq!(
            s.received.last().unwrap(),
            &(OPC_METHOD_SAVE_ACK, vec![0x01])
        );
    }
    {
        let a = app.lock();
        assert_eq!(a.inst.prog_name, "Manual run");
        assert_eq!(a.inst.version, "V2.01");
        assert_eq!(a.inst.run_state, "Run");
        assert_eq!(a.trend.len(), 5);
        assert_eq!(a.trend[4].cum_time_ms, 100_000.0 + 5.0 * 100.0);
        assert_eq!(a.trend[0].uv, 1.0);
        assert_eq!((a.inst.buffer_port, a.inst.buffer_port_known), (3, true)); // dump said 8, then the log 3
        assert_eq!(a.fractions.len(), 0); // no fraction size given: tube change only
        assert_eq!(a.inst.fraction_number, 2);
        assert_eq!(a.last_loaded_method, 5);
        assert!(a.methods_changed >= 2); // load and save
        let kinds: Vec<(EventKind, &str)> =
            a.events.iter().map(|e| (e.kind, e.text.as_str())).collect();
        let method_start = kinds
            .iter()
            .position(|(k, t)| *k == EventKind::Log && t.starts_with("Method start:"));
        let run = kinds.iter().position(|(k, _)| *k == EventKind::RunState);
        assert!(method_start.unwrap() < run.unwrap(), "{kinds:?}");
        // the dump is PC method 8: its run setup is recorded before the run starts
        // (the first run after connecting: in the log opened at the start)
        let setup_at = |t: &str| kinds.iter().position(|x| *x == (EventKind::Info, t));
        let reference = setup_at("PC method: 008").expect("method reference");
        assert!(setup_at("Column: HiTrap Q 1 ml").unwrap() > reference);
        assert!(setup_at("Port 1: Buffer A").unwrap() < method_start.unwrap());
        assert_eq!(
            rc_core::runinfo::column_volume(&rc_core::runinfo::run_info(&a.events)),
            1.0
        );
        assert_eq!(a.run_serial, 1);
        assert!(a.run_info_for_new_log.is_empty());
        assert!(kinds.contains(&(EventKind::Log, "Buffer Valve Pos 3")));
        assert!(
            kinds
                .iter()
                .any(|(k, t)| *k == EventKind::Log && t.starts_with("Method 7 saved by the Akta"))
        );
        assert!(a.pending_method_event.is_none());
    }
    assert_eq!(
        std::fs::read(rc_methods::method_path(&method_dir, 7)).unwrap(),
        rc_methods::unescape_method_text(METHOD_DUMP.as_bytes())
    );

    // ── A duplicate I-frame is not applied twice; a skipped SN is accepted ─────
    let n0 = app.lock().trend.len();
    {
        let mut s = sim.lock();
        s.lf_endings = true; // LF ends a frame like CR
        s.duplicate_next = true;
        s.outbox.push_back(payload(&[(OPC_TREND_DATA, &TREND)]));
    }
    wait_until("duplicate sent", 10, || sim.lock().dups_sent == 1);
    sim.lock()
        .outbox
        .push_back(payload(&[(OPC_TREND_DATA, &TREND)]));
    wait_until("trend after the duplicate", 10, || {
        app.lock().trend.len() >= n0 + 2
    });
    thread::sleep(Duration::from_millis(50));
    assert_eq!(app.lock().trend.len(), n0 + 2);
    {
        let mut s = sim.lock();
        s.skip_sn_next = true;
        s.outbox.push_back(payload(&[(OPC_TREND_DATA, &TREND)]));
    }
    wait_until("trend after a skipped SN", 10, || {
        app.lock().trend.len() == n0 + 3
    });
    sim.lock()
        .outbox
        .push_back(payload(&[(OPC_TREND_DATA, &TREND)]));
    wait_until("trend after the resync", 10, || {
        app.lock().trend.len() == n0 + 4
    });
    sim.lock().lf_endings = false;

    // ── The Akta restarts its link: CTRL_CONN while connected → resync ─────────
    sim.lock().send_ua_next = true;
    wait_until("resync", 10, || {
        has_event(&app, EventKind::Log, "Link: Akta re-established the link")
    });
    wait_until("driver enabled again", 10, || {
        sim.lock().count((OPC_IDENTITY_REQUEST, &[0x01])) == 2
    });
    assert_eq!(sim.lock().count((OPC_IDENTITY_REQUEST, &[0x00])), 2);
    assert_eq!(app.lock().status, "● Connected on SIM");

    // ── Silence: stay connected, report it, probe with polls, recover ──────────
    let from = {
        let mut s = sim.lock();
        s.silent_until_polls = 3;
        s.pc_frames.len()
    };
    wait_until("silence reported", 10, || {
        has_event(
            &app,
            EventKind::Log,
            "Link: no response from Akta — waiting",
        )
    });
    wait_until("responding again", 10, || {
        has_event(&app, EventKind::Log, "Link: Akta responding again")
    });
    wait_until("status connected", 10, || {
        app.lock().status == "● Connected on SIM"
    });
    {
        let s = sim.lock();
        assert!(
            s.statuses
                .iter()
                .any(|t| t == "No response from Akta on SIM — waiting")
        );
        let during: Vec<&Vec<u8>> = s.pc_frames[from..].iter().collect();
        let polls = during.iter().filter(|f| f.as_slice() == b"#0S5\r").count();
        let others = during.iter().filter(|f| f.as_slice() != b"#0S5\r").count();
        assert!(polls >= 3, "probes: {polls}");
        assert!(
            others > polls,
            "polls interleaved with receive-ready frames"
        );
    }
    assert!(!has_event(&app, EventKind::Disconnect, "Akta disconnected"));
    assert_eq!(sim.lock().count((OPC_IDENTITY_REQUEST, &[0x00])), 2); // no reset needed

    // ── Silence because the Akta reset its link: a probe poll gets UA → resync ─
    {
        let mut s = sim.lock();
        s.connected = false; // waits for SNRM, ignores everything else
    }
    wait_until("second silence", 10, || {
        event_count(
            &app,
            EventKind::Log,
            "Link: no response from Akta — waiting",
        ) == 2
    });
    wait_until("resync after probe", 10, || {
        event_count(&app, EventKind::Log, "Link: Akta re-established the link") == 2
    });
    assert_eq!(
        event_count(&app, EventKind::Log, "Link: Akta responding again"),
        2
    );
    wait_until("identity after probe resync", 10, || {
        sim.lock().count((OPC_IDENTITY_REQUEST, &[0x00])) == 3
    });

    // ── The port fails: new session, reconnect ─────────────────────────────────
    sim.lock().fail_io = true;
    wait_until("reconnected", 10, || {
        event_count(&app, EventKind::Connect, "Akta connected") == 2 && sim.lock().opens == 4
    });
    assert!(has_event(
        &app,
        EventKind::Log,
        "Link: serial connection lost — reconnecting"
    ));
    {
        let s = sim.lock();
        let st = &s.statuses;
        let lost = st
            .iter()
            .rposition(|t| t == "Connection to SIM lost — reconnecting")
            .unwrap();
        assert!(
            st[lost + 1..]
                .iter()
                .any(|t| t == "Waiting for Akta on SIM"),
            "{st:?}"
        );
        assert_eq!(s.bad_pc_frames, 0);
    }
    wait_until("identity after reconnect", 10, || {
        sim.lock().count((OPC_IDENTITY_REQUEST, &[0x00])) == 4
    });
    wait_until("status connected", 10, || {
        app.lock().status == "● Connected on SIM"
    });

    // ── Shutdown ───────────────────────────────────────────────────────────────
    shutdown.store(true, Ordering::Relaxed);
    let t = Instant::now();
    while !link.is_finished() {
        assert!(t.elapsed() < Duration::from_secs(5), "link did not stop");
        thread::sleep(Duration::from_millis(5));
    }
    link.join().unwrap();
    // the log never closes on silence or reconnects: everything went to the files
    let a = app.lock();
    let csv = std::fs::read_to_string(&a.log_filename).unwrap();
    assert!(csv.contains("EVT_RUN_STATE") && csv.contains(",TREND,"));
}

/// With the instrument's own timing (100 ms polls, 500 ms reply timeout).
#[test]
fn link_with_default_timing() {
    let app = AppState::default().shared();
    let sim = Arc::new(Mutex::new(Sim {
        app: Some(app.clone()),
        on_enable: vec![
            payload(&[(OPC_TREND_DATA, &TREND)]),
            payload(&[(OPC_TREND_DATA, &TREND)]),
        ],
        ..Sim::default()
    }));
    let shutdown = Arc::new(AtomicBool::new(false));
    let link = {
        let (app, sim, shutdown) = (app.clone(), sim.clone(), shutdown.clone());
        thread::spawn(move || {
            run_linklayer_with(&app, &shutdown, "SIM", opener(&sim), &LinkConfig::default())
        })
    };
    let t = Instant::now();
    wait_until("two trend points", 10, || app.lock().trend.len() == 2);
    // UA, identity, name+version, driver enable, two trend frames: one frame per poll
    assert!(
        t.elapsed() >= Duration::from_millis(350),
        "polls every 100 ms"
    );
    {
        let s = sim.lock();
        assert_eq!(
            s.received,
            vec![
                (OPC_IDENTITY_REQUEST, vec![0x00]),
                (OPC_IDENTITY_REQUEST, vec![0x01])
            ]
        );
    }
    shutdown.store(true, Ordering::Relaxed);
    link.join().unwrap();
    assert_eq!(app.lock().status, "● Connected on SIM");
}

/// A pending I-frame is kept while probing and sent again once the Akta answers.
#[test]
fn pending_frame_survives_silence() {
    let data = tempfile::tempdir().unwrap();
    let method_dir = data.path().to_path_buf();
    write_methods(&method_dir);
    let app = AppState {
        method_dir,
        ..AppState::default()
    }
    .shared();
    let sim = Arc::new(Mutex::new(Sim {
        app: Some(app.clone()),
        ..Sim::default()
    }));
    let shutdown = Arc::new(AtomicBool::new(false));
    let link = {
        let (app, sim, shutdown) = (app.clone(), sim.clone(), shutdown.clone());
        thread::spawn(move || run_linklayer_with(&app, &shutdown, "SIM", opener(&sim), &fast()))
    };
    wait_until("driver enabled", 10, || {
        sim.lock().count((OPC_IDENTITY_REQUEST, &[0x01])) == 1
    });
    {
        // the Akta asks whether method 5 exists, then goes silent: the reply is lost
        let mut s = sim.lock();
        s.outbox
            .push_back(payload(&[(OPC_METHOD_EXISTS, &[0x05, 0x00])]));
        s.ignore_after_send = 15; // the reply and ~300 ms after it
    }
    wait_until("exists reply", 10, || {
        sim.lock().count((OPC_METHOD_EXISTS_REPLY, &[0x01])) == 1
    });
    shutdown.store(true, Ordering::Relaxed);
    link.join().unwrap();
    assert!(has_event(
        &app,
        EventKind::Log,
        "Link: no response from Akta — waiting"
    ));
    let s = sim.lock();
    // the reply I-frame went out again after the probing, with the same SN
    let replies: Vec<&Vec<u8>> = s
        .pc_frames
        .iter()
        .filter(|f| {
            (f[2] & 0xf0) == CTRL_IFRAME
                && decode_opc_payload(&f[3..f.len() - 2])[0].0 == OPC_METHOD_EXISTS_REPLY
        })
        .collect();
    assert!(replies.len() >= 2, "{}", replies.len());
    assert!(replies.iter().all(|f| f[2] == replies[0][2]));
}
