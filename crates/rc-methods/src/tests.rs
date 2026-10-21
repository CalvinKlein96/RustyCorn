//! Unit tests: method dump, method text codec, method store, method files. The
//! method store end to end over the link layer is tested in rc-protocol.

use std::fs;

use rc_core::opc::*;
use rc_core::{AppState, InjectValue};

use crate::*;

/// A method dump as an ÄKTAprime (V2.01) sends it before a run: manual method,
/// 0.5 ml/min, 0 %B, buffer valve port 8, injection valve Load.
const METHOD_DUMP: &str = concat!(
    "{l{5}{2 Method Dump}{2}{2 AKTAprime|3Ver|2, V2.01|?}{2}{2 Dump Format Ver|2, V1.00|?}",
    "{2}{l{5}}}{B}{2 Method Settings}{B}Method Type|8,Manaul Method}Method|=,1}Method base|8,ml}",
    "Frac base|:,ml}Max pressure (kPa) ,1000}Alarm (min/ml)|5,0.0}}{E}{2 Method Breakpoints}{E}|E}",
    "Brkp Volume|3Conc Flow|2Frac|3Buff Inj|4Peak|3Autozzero Eventmark}|Z(mAu/min)}}",
    ",1|3,0.0|5,0|3,0.5|2,0.0|3,8|3,Load|2,0|5,N|7,N},2|3,99999.9 ,0|3,0.5|2,0.0|3,8|3,Load|2,0|5,N|7,N}"
);

#[test]
fn method_dump() {
    let m = parse_method_dump(METHOD_DUMP.as_bytes());
    let setting = |k: &str| {
        m.settings
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.as_str())
    };
    assert_eq!(setting("Max pressure (kPa)"), Some("1000"));
    assert_eq!(
        m.columns,
        [
            "Brkp",
            "Volume",
            "Conc",
            "Flow",
            "Frac",
            "Buff",
            "Inj",
            "Peak",
            "Autozero",
            "Eventmark"
        ]
    );
    assert_eq!(m.breakpoints.len(), 2); // incl. the end row "99999.9 ,0"
    assert_eq!(m.breakpoints[0]["Buff"], "8");
    assert_eq!(m.breakpoints[0]["Flow"], "0.5");
    assert_eq!(m.breakpoints[1]["Volume"], "99999.9");
    let mut app = AppState::new();
    let generation = app.generation;
    apply_method_dump(&mut app, &m);
    assert!(app.inst.buffer_port == 8 && app.inst.buffer_port_known);
    assert_eq!(app.inst.inject_value, InjectValue::Load);
    let ev = app.pending_method_event.clone().unwrap();
    assert!(ev.contains("buffer port 8"));
    assert_eq!(
        ev,
        "Method start: flow 0.5 ml/min, 0 %B, buffer port 8, inject Load, fraction 0.0 ml, 2 breakpoints"
    );
    assert_ne!(app.generation, generation);
    // the plot model reads the run's start state back from this text
    let re = regex::Regex::new(r"^Method start:.*buffer port (\d+), inject (\w+)").unwrap();
    let c = re.captures(&ev).unwrap();
    assert_eq!((&c[1], &c[2]), ("8", "Load"));

    // no breakpoints: nothing changes
    let mut app = AppState::new();
    apply_method_dump(&mut app, &parse_method_text(b"Method base ,ml\r\n"));
    assert!(app.pending_method_event.is_none() && !app.inst.buffer_port_known);
}

#[test]
fn method_text_codec() {
    let cases = [
        "Flow  0.5 #1 z{|}~ end".to_string(),
        " ".repeat(61) + "x",
        "*".repeat(65),
        "a b * c".to_string(),
        String::new(),
    ];
    for s in &cases {
        assert_eq!(
            unescape_method_text(&escape_method_text(s.as_bytes())),
            s.as_bytes()
        );
        // a '#' would start a link-layer frame
        assert!(!escape_method_text(s.as_bytes()).contains(&b'#'));
    }
    assert_eq!(escape_method_text(b"Max  ,#3 z"), b"Max|2,zy3 zz");
    let dump = unescape_method_text(METHOD_DUMP.as_bytes());
    let head = "*".repeat(65) + "\r\n** Method Dump\r\n";
    assert!(dump.starts_with(head.as_bytes()));
    assert_eq!(
        parse_method_text(&dump).breakpoints,
        parse_method_dump(METHOD_DUMP.as_bytes()).breakpoints
    );
}

fn nums(a: &MethodAction) -> Vec<u16> {
    (0..20)
        .step_by(2)
        .map(|i| u16_at(&a.replies[0].1, i))
        .collect()
}

#[test]
fn method_store() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let mut x = MethodTransfer::default();
    let mut handle = |opc: u16, data: &[u8]| x.handle(opc, data, dir);
    assert!(handle(OPC_TREND_DATA, &[]).is_none());

    // The Akta saves a method as number 3, its text split over several messages.
    let raw = METHOD_DUMP.as_bytes();
    assert!(
        handle(OPC_METHOD_SAVE_BEGIN, &[0x03, 0x00])
            .unwrap()
            .replies
            .is_empty()
    );
    for part in raw.chunks(30) {
        handle(OPC_METHOD_LINE, part);
    }
    let r = handle(OPC_METHOD_SAVE_END, &[]).unwrap();
    assert_eq!(r.replies, vec![(OPC_METHOD_SAVE_ACK, vec![0x01])]);
    let file = method_path(dir, 3);
    assert_eq!(fs::read(&file).unwrap(), unescape_method_text(raw));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt; // readable by other users
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o644
        );
    }
    let note = r.note.unwrap();
    assert_eq!(
        note,
        format!("Method 3 saved by the Akta to {}", file.display())
    );
    assert!(!note.contains("only one breakpoint"));
    // A single-breakpoint method is saved, with a warning: the Akta cannot load it back.
    let one_bp = &METHOD_DUMP[..METHOD_DUMP.find("},2|3,99999.9").unwrap() + 1];
    handle(OPC_METHOD_SAVE_BEGIN, &[0x04, 0x00]);
    handle(OPC_METHOD_LINE, one_bp.as_bytes());
    let r4 = handle(OPC_METHOD_SAVE_END, &[]).unwrap();
    assert_eq!(r4.replies, vec![(OPC_METHOD_SAVE_ACK, vec![0x01])]);
    assert!(r4.note.unwrap().ends_with(
        " — it has only one breakpoint, so the Akta cannot load it back; add an end breakpoint"
    ));
    fs::remove_file(method_path(dir, 4)).unwrap();
    let none_open = handle(OPC_METHOD_SAVE_END, &[]).unwrap();
    assert_eq!(none_open.replies, vec![(OPC_METHOD_SAVE_ACK, vec![0x00])]);
    assert_eq!(
        none_open.note.as_deref(),
        Some("Akta finished a method save that never started — not saved")
    );

    // A dump in between is returned for decoding, not saved.
    handle(OPC_METHOD_DUMP_BEGIN, &[]);
    handle(OPC_METHOD_LINE, raw);
    assert_eq!(
        handle(OPC_METHOD_DUMP_END, &[0x00])
            .unwrap()
            .dump
            .as_deref(),
        Some(raw)
    );

    // Listing: ten numbers from the start, up or down, 0xFFFF for empty slots.
    let long = "Method Type        ,".to_string() + &"Long name with a z and #, ".repeat(3);
    fs::write(
        method_path(dir, 5),
        format!("*****\r\n{long}\r\n\r\n,1   ,0.0\r\n"),
    )
    .unwrap();
    fs::write(method_path(dir, 12), "x\n").unwrap();
    fs::write(dir.join("PrimeMeth_tmp.txt"), "x").unwrap();
    assert_eq!(stored_methods(dir), [3, 5, 12]);
    let r = handle(OPC_METHOD_LIST, &[0x01, 0x00, 0x01]).unwrap();
    assert!(r.replies[0].0 == OPC_METHOD_LIST_REPLY && r.replies[0].1.len() == 20);
    let mut want = vec![3, 5, 12];
    want.resize(10, 0xffff);
    assert_eq!(nums(&r), want);
    let mut want = vec![5, 3];
    want.resize(10, 0xffff);
    assert_eq!(
        nums(&handle(OPC_METHOD_LIST, &[0x05, 0x00, 0x00]).unwrap()),
        want
    );
    assert_eq!(
        nums(&handle(OPC_METHOD_LIST, &[0x00, 0x00, 0x01]).unwrap()),
        vec![0xffff; 10]
    );

    // Names from the editor stay on the PC (names.txt) and do not disturb the store.
    set_method_name(dir, 5, "HisTag elution, 5 ml").unwrap();
    set_method_name(dir, 12, "  Wash  ").unwrap();
    let names = method_names(dir);
    assert_eq!(names.len(), 2);
    assert!(names[&5] == "HisTag elution, 5 ml" && names[&12] == "Wash");
    set_method_name(dir, 12, "").unwrap();
    assert_eq!(
        method_names(dir).into_iter().collect::<Vec<_>>(),
        [(5, "HisTag elution, 5 ml".to_string())]
    );
    assert_eq!(stored_methods(dir), [3, 5, 12]);
    assert!(
        method_name_ok("Säule – Elution")
            && !method_name_ok("a\tb")
            && !method_name_ok(&"x".repeat(61))
    );

    // Loading: 0x2013, the lines (long ones in 34-character parts), 0x2014 [0].
    let r = handle(OPC_METHOD_LOAD, &[0x05, 0x00]).unwrap();
    assert_eq!(r.replies[0], (OPC_METHOD_LOAD_BEGIN, vec![]));
    assert_eq!(
        r.replies.last().unwrap(),
        &(OPC_METHOD_LOAD_END, vec![0x00])
    );
    assert_eq!(
        r.note.as_deref(),
        Some("Method 5 sent to the Akta (3 lines)")
    );
    let body = &r.replies[1..r.replies.len() - 1];
    assert!(body.iter().all(|(_, d)| d.len() <= METHOD_CHUNK));
    let mut want = vec![OPC_METHOD_LINE, OPC_METHOD_LINE_BEGIN];
    want.extend(std::iter::repeat_n(OPC_METHOD_LINE_CONT, body.len() - 4));
    want.extend([OPC_METHOD_LINE_END, OPC_METHOD_LINE]);
    assert_eq!(body.iter().map(|(o, _)| *o).collect::<Vec<_>>(), want);
    let mut lines: Vec<Vec<u8>> = Vec::new();
    for (o, d) in body {
        if *o == OPC_METHOD_LINE || *o == OPC_METHOD_LINE_BEGIN {
            lines.push(Vec::new());
        }
        lines.last_mut().unwrap().extend_from_slice(d);
    }
    let lines: Vec<String> = lines
        .iter()
        .map(|l| String::from_utf8(unescape_method_text(l)).unwrap())
        .collect();
    assert_eq!(lines, ["*****", long.as_str(), ",1   ,0.0"]);
    // every text message fits the link layer's ASCII messages: printable, no frame start
    for (o, d) in &r.replies {
        if o & 0x8000 != 0 {
            assert!(
                d.iter().all(|&b| (0x20..=0x7f).contains(&b) && b != b'#'),
                "{o:04x} {d:?}"
            );
        }
    }
    let missing = handle(OPC_METHOD_LOAD, &[0x07, 0x00]).unwrap();
    assert_eq!(missing.replies, vec![(OPC_METHOD_LOAD_END, vec![0x01])]);
    assert_eq!(
        missing.note.as_deref(),
        Some("Method 7 requested, but there is no such file")
    );
    fs::write(
        method_path(dir, 13),
        "ok\r\n".to_string() + &"x".repeat(100) + "\r\n",
    )
    .unwrap();
    let bad = handle(OPC_METHOD_LOAD, &[0x0d, 0x00]).unwrap();
    assert_eq!(
        bad.replies.last().unwrap(),
        &(OPC_METHOD_LOAD_END, vec![0x02])
    );
    assert_eq!(
        bad.note.as_deref(),
        Some(
            "Method 13 sent with an error: line 2 is longer than 99 characters (PrimeMeth_013.txt)"
        )
    );

    // Exists and delete; deleted and overwritten files are kept in old/.
    let exists = |h: &mut dyn FnMut(u16, &[u8]) -> Option<MethodAction>| {
        h(OPC_METHOD_EXISTS, &[0x0c, 0x00]).unwrap().replies
    };
    assert_eq!(
        exists(&mut handle),
        vec![(OPC_METHOD_EXISTS_REPLY, vec![0x01])]
    );
    let del = handle(OPC_METHOD_DELETE, &[0x0c, 0x00]).unwrap();
    assert_eq!(del.replies, vec![(OPC_METHOD_DELETE_REPLY, vec![0x01])]);
    assert_eq!(
        del.note,
        Some(format!(
            "Method 12 deleted by the Akta (moved to {})",
            dir.join("old").display()
        ))
    );
    assert_eq!(
        exists(&mut handle),
        vec![(OPC_METHOD_EXISTS_REPLY, vec![0x00])]
    );
    let del = handle(OPC_METHOD_DELETE, &[0x0c, 0x00]).unwrap();
    assert_eq!(del.replies, vec![(OPC_METHOD_DELETE_REPLY, vec![0x00])]);
    assert_eq!(
        del.note.as_deref(),
        Some("Akta asked to delete method 12 — not deleted")
    );
    handle(OPC_METHOD_SAVE_BEGIN, &[0x03, 0x00]);
    handle(OPC_METHOD_LINE, b"new}");
    handle(OPC_METHOD_SAVE_END, &[]);
    assert_eq!(fs::read_to_string(&file).unwrap(), "new\r\n");
    assert_eq!(fs::read_dir(dir.join("old")).unwrap().count(), 2);

    // Requests without a method number are ignored.
    assert_eq!(
        handle(OPC_METHOD_LOAD, &[0x03]),
        Some(MethodAction::default())
    );
}

#[test]
fn method_store_case_insensitive_and_out_of_range() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    fs::write(dir.join("primemeth_020.TXT"), "x\r\n").unwrap();
    assert_eq!(stored_methods(dir), [20]);
    assert_eq!(
        find_method_file(dir, 20),
        Some(dir.join("primemeth_020.TXT"))
    );
    assert_eq!(find_method_file(dir, 21), None);
    assert_eq!(find_method_file(&dir.join("missing"), 1), None);
    assert!(stored_methods(&dir.join("missing")).is_empty());
    // a save over a differently cased file retires it
    save_method_file(dir, 20, b"y\r\n").unwrap();
    assert_eq!(stored_methods(dir), [20]);
    assert_eq!(fs::read(method_path(dir, 20)).unwrap(), b"y\r\n");
    let old: Vec<String> = fs::read_dir(dir.join("old"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert!(old.len() == 1 && old[0].starts_with("primemeth_020_") && old[0].ends_with(".txt"));

    // numbers outside 1–999 are not saved
    let mut x = MethodTransfer::default();
    x.handle(OPC_METHOD_SAVE_BEGIN, &[0xe8, 0x03], dir);
    x.handle(OPC_METHOD_LINE, b"x}", dir);
    let r = x.handle(OPC_METHOD_SAVE_END, &[], dir).unwrap();
    assert_eq!(r.replies, vec![(OPC_METHOD_SAVE_ACK, vec![0x00])]);
    assert_eq!(
        r.note.as_deref(),
        Some("Method 1000 from the Akta could not be saved")
    );
    assert!(!method_path(dir, 1000).exists());
}

#[test]
fn method_files() {
    // A file as the Akta writes it reads and writes back unchanged.
    let bytes = unescape_method_text(METHOD_DUMP.as_bytes());
    let m = MethodFile::read(&bytes).unwrap();
    assert_eq!(m.format_text(), bytes);
    assert!(m.setting("Max pressure (kPa)") == "1000" && m.rows.len() == 2);
    assert_eq!(
        m.rows[0],
        ["0.0", "0", "0.5", "0.0", "8", "Load", "0", "N", "N"]
    );
    assert!(m.problems().0.is_empty());
    assert_eq!(
        MethodFile::read(&MethodFile::new_method().format_text()).unwrap(),
        MethodFile::new_method()
    );
    assert!(MethodFile::read(b"no method here").is_err());

    // Values: what may be entered, and how it is written.
    assert!(method_value_ok("Flow", "0,5") && normalize_value("Flow", "0,5") == "0.5");
    assert!(normalize_value("Volume", "10") == "10.0" && normalize_value("Conc", "05") == "5");
    assert!(normalize_value("Peak", "0.10") == "0.1" && normalize_value("Peak", "0") == "0");
    assert!(!method_value_ok("Flow", "51") && !method_value_ok("Conc", "100.5"));
    assert!(!method_value_ok("Buff", "9") && !method_value_ok("Inj", "load"));
    assert!(!method_value_ok("Method Type", "a,b") && method_value_ok("Method Type", "-"));

    // Problems that stop the Akta loading a method.
    let mut one = m.clone();
    one.rows.pop();
    let (msgs, _) = one.problems();
    assert!(msgs.len() == 1 && msgs[0].contains("two breakpoints"));
    let mut bad = m.clone();
    bad.rows[0][0] = "1.0".into();
    bad.rows[1][0] = "0.5".into();
    bad.rows[1][2] = "x".into();
    let (msgs, cells) = bad.problems();
    assert!(cells.contains(&(1, "Volume".into())) && cells.contains(&(2, "Volume".into())));
    assert!(cells.contains(&(2, "Flow".into())) && msgs.len() == 3);
    // Saving normalises: 1 → 1.0 in the file.
    let mut m2 = m.clone();
    m2.rows[0][2] = "1".into();
    assert!(
        String::from_utf8(m2.normalized().format_text())
            .unwrap()
            .contains(",1.0  ,")
    );

    // settings
    let mut m3 = MethodFile::new_method();
    assert_eq!(m3.setting("Method base"), "ml");
    assert_eq!(m3.setting("nope"), "");
    m3.set_setting("Method base", "min");
    m3.set_setting("Extra", "1");
    assert_eq!(m3.setting("Method base"), "min");
    assert_eq!(
        m3.settings.last().unwrap(),
        &("Extra".to_string(), "1".to_string())
    );
    assert_eq!(value_choices("Inj"), Some(&INJ_OPTIONS[..]));
    assert_eq!(value_choices("Volume"), None);
}

#[test]
fn list_payload_and_lines() {
    let p = method_list_payload(&[3, 5, 12], 4, true);
    assert_eq!(&p[..4], &[5, 0, 12, 0]);
    assert!(p[4..].iter().all(|&b| b == 0xff));
    assert_eq!(
        method_list_payload(&[3, 5, 12], 1000, false),
        vec![0xff; 20]
    );
    let nums: Vec<u16> = (1..=30).collect();
    let p = method_list_payload(&nums, 25, false);
    assert_eq!(
        (0..20)
            .step_by(2)
            .map(|i| u16_at(&p, i))
            .collect::<Vec<_>>(),
        (16..=25).rev().collect::<Vec<_>>()
    );
    assert_eq!(
        method_file_lines(b"a\r\n\r\nb\rc\n"),
        [b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]
    );
    let msgs = method_line_messages("x".repeat(69).as_bytes());
    assert_eq!(
        msgs.iter().map(|(o, d)| (*o, d.len())).collect::<Vec<_>>(),
        [
            (OPC_METHOD_LINE_BEGIN, 34),
            (OPC_METHOD_LINE_CONT, 34),
            (OPC_METHOD_LINE_END, 1)
        ]
    );
    assert!(is_method_opc(OPC_METHOD_LIST) && !is_method_opc(OPC_METHOD_LIST_REPLY));
}
