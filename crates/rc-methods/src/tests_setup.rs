//! The run setup next to a method (PrimeMeth_NNN_setup.txt) and which stored
//! method a run-start dump is.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use rc_core::opc::*;
use rc_core::runinfo::METHOD_REF_KEY;

use crate::*;

/// The run-start dump of a manual method, as the ÄKTAprime (V2.01) sends it (escaped).
const METHOD_DUMP: &str = concat!(
    "{l{5}{2 Method Dump}{2}{2 AKTAprime|3Ver|2, V2.01|?}{2}{2 Dump Format Ver|2, V1.00|?}",
    "{2}{l{5}}}{B}{2 Method Settings}{B}Method Type|8,Manaul Method}Method|=,1}Method base|8,ml}",
    "Frac base|:,ml}Max pressure (kPa) ,1000}Alarm (min/ml)|5,0.0}}{E}{2 Method Breakpoints}{E}|E}",
    "Brkp Volume|3Conc Flow|2Frac|3Buff Inj|4Peak|3Autozzero Eventmark}|Z(mAu/min)}}",
    ",1|3,0.0|5,0|3,0.5|2,0.0|3,8|3,Load|2,0|5,N|7,N},2|3,99999.9 ,0|3,0.5|2,0.0|3,8|3,Load|2,0|5,N|7,N}"
);

fn setup(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

fn instrument_file(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/instrument")
        .join(name);
    fs::read(path).expect("instrument file")
}

#[test]
fn setup_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    assert!(method_setup(dir, 5).is_empty());
    let s = setup(&[
        ("Column", " HisTrap 5 ml, Ni "),
        ("Column volume (mL)", "5"),
        ("Port 1", "Buffer A: 50 mM Tris"),
        ("Port 8", "Lysate\nline 2"),
        ("Port 3", "  "),
        ("Sample", "per run, not kept"),
        ("Unknown", "x"),
    ]);
    set_method_setup(dir, 5, &s).unwrap();
    let text = fs::read_to_string(method_setup_path(dir, 5)).unwrap();
    assert_eq!(
        text,
        format!(
            "{METHOD_SETUP_HEADER}\nColumn: HisTrap 5 ml, Ni\nColumn volume (mL): 5\n\
             Port 1: Buffer A: 50 mM Tris\nPort 8: Lysate line 2\n"
        )
    );
    assert_eq!(
        method_path(dir, 5).with_file_name("PrimeMeth_005_setup.txt"),
        method_setup_path(dir, 5)
    );
    let back = method_setup(dir, 5);
    assert_eq!(
        back,
        setup(&[
            ("Column", "HisTrap 5 ml, Ni"),
            ("Column volume (mL)", "5"),
            ("Port 1", "Buffer A: 50 mM Tris"),
            ("Port 8", "Lysate line 2"),
        ])
    );
    assert_eq!(normalized_setup(&s), back);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(method_setup_path(dir, 5))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o644);
    }

    // the sidecar is no method: not listed, not loadable, not found as one
    assert!(stored_methods(dir).is_empty());
    assert_eq!(find_method_file(dir, 5), None);
    fs::write(method_path(dir, 5), b"x\r\n").unwrap();
    assert_eq!(stored_methods(dir), [5]);
    let payload = method_list_payload(&stored_methods(dir), 1, true);
    assert_eq!(&payload[..4], &[5, 0, 0xff, 0xff]);
    let (msgs, _) = method_load_messages(dir, 5).unwrap();
    assert_eq!(msgs.len(), 3); // begin, the one line, end: the method file only

    // case-insensitive (files copied from Windows), and an empty setup removes the file
    fs::remove_file(method_setup_path(dir, 5)).unwrap();
    fs::write(dir.join("primemeth_005_SETUP.txt"), "Column: S200\n").unwrap();
    assert_eq!(method_setup(dir, 5), setup(&[("Column", "S200")]));
    set_method_setup(dir, 5, &setup(&[("Column", "S75")])).unwrap();
    assert_eq!(
        names_in(dir),
        ["PrimeMeth_005.txt", "PrimeMeth_005_setup.txt"]
    );
    set_method_setup(dir, 5, &BTreeMap::new()).unwrap();
    assert_eq!(names_in(dir), ["PrimeMeth_005.txt"]);
    set_method_setup(dir, 5, &BTreeMap::new()).unwrap(); // nothing to remove: fine
    assert_eq!(fs::read(method_path(dir, 5)).unwrap(), b"x\r\n"); // never touched

    // comments, other keys and lines without a separator are skipped
    assert_eq!(
        parse_method_setup("# c\r\nColumn:S200\r\nnonsense\nSample: x\nBuffer B:  \n"),
        setup(&[("Column", "S200")])
    );
}

#[test]
fn setup_moves_with_its_method() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let old = dir.join(METHOD_OLD_DIR);
    let text = unescape_method_text(METHOD_DUMP.as_bytes());

    // the Akta saves over method 4: the old file and its setup go to old/ together
    save_method_file(dir, 4, &text).unwrap();
    set_method_setup(dir, 4, &setup(&[("Column", "A")])).unwrap();
    save_method_file(dir, 4, &text).unwrap();
    assert!(method_setup(dir, 4).is_empty());
    let retired = names_in(&old);
    assert_eq!(retired.len(), 2, "{retired:?}");
    let stem = retired[0].strip_suffix(".txt").unwrap();
    assert!(stem.starts_with("PrimeMeth_004_"), "{retired:?}");
    assert_eq!(retired[1], format!("{stem}_setup.txt"));
    // what is in old/ is no method or setup of the store
    assert_eq!(stored_methods(dir), [4]);

    // deleted by the Akta: both moved
    set_method_setup(dir, 4, &setup(&[("Column", "B")])).unwrap();
    let mut tr = MethodTransfer::default();
    let a = tr.handle(OPC_METHOD_DELETE, &[4, 0], dir).unwrap();
    assert_eq!(a.replies, vec![(OPC_METHOD_DELETE_REPLY, vec![1])]);
    assert!(names_in(dir).iter().all(|f| !f.starts_with("PrimeMeth_")));
    // (two retirements in one second get the same time stamp, so look by content)
    assert!(names_in(&old).iter().any(|f| {
        f.ends_with("_setup.txt")
            && fs::read_to_string(old.join(f))
                .unwrap()
                .contains("Column: B")
    }));

    // a setup without its method does not pass to a new method of that number
    set_method_setup(dir, 7, &setup(&[("Column", "orphan")])).unwrap();
    save_method_file(dir, 7, &text).unwrap();
    assert!(method_setup(dir, 7).is_empty());
    assert!(
        names_in(&old)
            .iter()
            .any(|f| f.starts_with("PrimeMeth_007_") && f.ends_with("_setup.txt"))
    );

    // retire_method_file returns the method's new place
    set_method_setup(dir, 7, &setup(&[("Column", "C")])).unwrap();
    let dest = retire_method_file(&method_path(dir, 7)).unwrap();
    assert!(dest.starts_with(&old) && dest.is_file());
    let stem = dest.file_stem().unwrap().to_string_lossy().into_owned();
    assert_eq!(
        fs::read_to_string(old.join(format!("{stem}_setup.txt"))).unwrap(),
        format!("{METHOD_SETUP_HEADER}\nColumn: C\n")
    );
}

#[test]
fn run_start_dump_matches_its_method_file() {
    // The files are decoded dumps: a dump with a file's content matches it.
    // Check with the instrument's own files, sent escaped as the Akta sends a
    // dump (the real run-start dump of a PC method also names it: see
    // a_loaded_pc_method_is_found_by_its_number_and_content).
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let one = instrument_file("PrimeMeth_001.txt");
    let six = instrument_file("PrimeMeth_006.txt");
    fs::write(method_path(dir, 1), &one).unwrap();
    fs::write(method_path(dir, 6), &six).unwrap();
    for (bytes, n) in [(&one, 1), (&six, 6)] {
        let dump = parse_method_dump(&escape_method_text(bytes));
        assert_eq!(matching_stored_methods(dir, &dump), [n]);
        assert_eq!(match_stored_method(dir, &dump, 0), Some(n));
    }

    // the instrument's run-start dump of a manual method is the same text layout
    // as a file: written back byte for byte, and found once stored
    let manual = unescape_method_text(METHOD_DUMP.as_bytes());
    assert_eq!(MethodFile::read(&manual).unwrap().format_text(), manual);
    let dump = parse_method_dump(METHOD_DUMP.as_bytes());
    assert_eq!(match_stored_method(dir, &dump, 0), None);
    fs::write(method_path(dir, 8), &manual).unwrap();
    assert_eq!(match_stored_method(dir, &dump, 0), Some(8));

    // robust to padding, line endings and number spellings
    let text = String::from_utf8(one.clone()).unwrap();
    let loose = text
        .replace("\r\n", "\n")
        .replace(",1.0  ,", ",1  ,")
        .replace(",0     ,", ",0.00 ,")
        .replace("Method base        ,ml", "Method base ,  ml  ")
        .replace(",Load  ,", ",Load,");
    assert_ne!(loose.as_bytes(), &one[..]);
    let dump = parse_method_dump(&escape_method_text(loose.as_bytes()));
    assert_eq!(match_stored_method(dir, &dump, 0), Some(1));

    // but not to a different value, setting or breakpoint count
    for (from, to) in [
        (",10.0    ,0", ",10.5    ,0"),
        ("Max pressure (kPa) ,500", "Max pressure (kPa) ,400"),
        (",7   ,50.0", ",7   ,51.0"),
    ] {
        assert!(text.contains(from), "{from}");
        let changed = text.replace(from, to);
        let dump = parse_method_dump(&escape_method_text(changed.as_bytes()));
        assert_eq!(match_stored_method(dir, &dump, 0), None, "{to}");
    }
    let mut fewer = MethodFile::read(&one).unwrap();
    fewer.rows.pop();
    let dump = parse_method_dump(&escape_method_text(&fewer.format_text()));
    assert_eq!(match_stored_method(dir, &dump, 0), None);
    // nothing understood: no match, even with an empty store entry
    fs::write(method_path(dir, 9), b"").unwrap();
    assert_eq!(
        match_stored_method(dir, &parse_method_dump(b"garbage"), 0),
        None
    );
}

/// The run-start dump of PC method 9 on an ÄKTAprime (debug run, 2026-11-13):
/// "Method Type" "PC Method" and "Method" "9" where its file has "-".
const PC_METHOD_9_DUMP: &str = "{l{5}{2 Method Dump}{2}{2 AKTAprime|3Ver|2, V2.01|?}{2}{2 Dump Format Ver|2, V1.00|?}{2}{l{5}}}{B}{2 Method Settings}{B}Method Type|8,PC Method}Method|=,9}Method base|8,ml}Frac base|:,ml}Max pressure (kPa) ,1000}Alarm (min/ml)|5,0.0}}{E}{2 Method Breakpoints}{E}|E}Brkp Volume|3Conc Flow|2Frac|3Buff Inj|4Peak|3Autozzero Eventmark}|Z(mAu/min)}},1|3,0.0|5,0|3,0.5|2,1.0|3,1|3,Load|2,0|5,Y|7,N},2|3,1.0|5,0|3,0.5|2,0.0|3,1|3,Load|2,0|5,N|7,N},3|3,1.5|5,0|3,0.5|2,1.0|3,1|3,Load|2,0|5,N|7,N},4|3,4.0|5,0|3,0.5|2,0.0|3,1|3,Load|2,0|5,N|7,N}";

#[test]
fn a_loaded_pc_method_is_found_by_its_number_and_content() {
    let dump = parse_method_dump(PC_METHOD_9_DUMP.as_bytes());
    assert_eq!(pc_method_number(&dump), Some(9));
    // the file the PC sent: "-" where the dump says where the method is kept
    let file = unescape_method_text(
        PC_METHOD_9_DUMP
            .replace(
                "Method Type|8,PC Method}Method|=,9}",
                "Method Type|8,-}Method|=,-}",
            )
            .as_bytes(),
    );
    assert_eq!(pc_method_number(&parse_method_text(&file)), None);

    // stored as 9: its run setup is the run's
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    save_method_file(dir, 9, &file).unwrap();
    set_method_setup(
        dir,
        9,
        &setup(&[("Column", "Superdex 200"), ("Column volume (mL)", "24")]),
    )
    .unwrap();
    assert_eq!(match_stored_method(dir, &dump, 0), Some(9));
    let info = run_setup_for_dump(dir, &dump, 9);
    assert_eq!(info["Column"], "Superdex 200");
    assert_eq!(info["Column volume (mL)"], "24");
    assert_eq!(info[METHOD_REF_KEY], "009");

    // stored under another number only: found by its content
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    save_method_file(dir, 4, &file).unwrap();
    assert_eq!(match_stored_method(dir, &dump, 0), Some(4));
    // and so is a copy in the instrument's memory
    let memory = parse_method_dump(
        PC_METHOD_9_DUMP
            .replace(
                "Method Type|8,PC Method}Method|=,9}",
                "Method Type|8,Manaul Method}Method|=,3}",
            )
            .as_bytes(),
    );
    assert_eq!(pc_method_number(&memory), None);
    assert_eq!(match_stored_method(dir, &memory, 0), Some(4));
    // a different method stays unmatched
    let other = PC_METHOD_9_DUMP.replace(",4|3,4.0|", ",4|3,5.0|");
    assert_ne!(other, PC_METHOD_9_DUMP);
    assert_eq!(
        match_stored_method(dir, &parse_method_dump(other.as_bytes()), 0),
        None
    );
}

#[test]
fn several_matches_prefer_the_last_loaded() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let text = unescape_method_text(METHOD_DUMP.as_bytes());
    for n in [2, 5, 9] {
        save_method_file(dir, n, &text).unwrap();
    }
    let dump = parse_method_dump(METHOD_DUMP.as_bytes());
    assert_eq!(matching_stored_methods(dir, &dump), [2, 5, 9]);
    assert_eq!(match_stored_method(dir, &dump, 5), Some(5));
    assert_eq!(match_stored_method(dir, &dump, 9), Some(9));
    // the last loaded method is another one: the lowest number
    assert_eq!(match_stored_method(dir, &dump, 3), Some(2));
    assert_eq!(match_stored_method(dir, &dump, 0), Some(2));
}

#[test]
fn run_setup_of_a_dump() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let text = unescape_method_text(METHOD_DUMP.as_bytes());
    let dump = parse_method_dump(METHOD_DUMP.as_bytes());
    // no stored method: nothing, whatever was loaded last
    assert!(run_setup_for_dump(dir, &dump, 3).is_empty());

    save_method_file(dir, 3, &text).unwrap();
    // a matching method without setup or name: only the reference
    assert_eq!(
        run_setup_for_dump(dir, &dump, 0),
        setup(&[(METHOD_REF_KEY, "003")])
    );
    set_method_name(dir, 3, "Desalting").unwrap();
    set_method_setup(
        dir,
        3,
        &setup(&[("Column", "HiTrap Desalting"), ("Port 8", "Sample")]),
    )
    .unwrap();
    // a hand-edited setup file with per-run fields: those are not taken from the method
    let path = method_setup_path(dir, 3);
    let mut s = fs::read_to_string(&path).unwrap();
    s.push_str("Sample: not from the method\nUser: nobody\n");
    fs::write(&path, s).unwrap();
    assert_eq!(
        run_setup_for_dump(dir, &dump, 0),
        setup(&[
            (METHOD_REF_KEY, "003 – Desalting"),
            ("Column", "HiTrap Desalting"),
            ("Port 8", "Sample"),
        ])
    );
    assert_eq!(method_reference(dir, 4), "004");
}

#[test]
fn method_notes_keep_their_line_breaks() {
    use rc_core::runinfo::METHOD_NOTES_KEY;
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let notes = "Equilibrate 5 CV first.\nLoad: 2 ml, C:\\temp\\n.txt\r\n\n  indented line";
    let s = setup(&[
        ("Column", "S200"),
        (METHOD_NOTES_KEY, &format!("  {notes} \n")),
        ("Port 1", "Water"),
    ]);
    set_method_setup(dir, 7, &s).unwrap();
    let text = fs::read_to_string(method_setup_path(dir, 7)).unwrap();
    // one line per field: the line breaks and backslashes escaped, explained in a comment
    assert_eq!(
        text,
        format!(
            "{METHOD_SETUP_HEADER}\n{METHOD_SETUP_ESCAPES_NOTE}\nColumn: S200\nPort 1: Water\n\
             Method notes: Equilibrate 5 CV first.\\nLoad: 2 ml, C:\\\\temp\\\\n.txt\\n\\n  indented line\n"
        )
    );
    let back = method_setup(dir, 7);
    let want = "Equilibrate 5 CV first.\nLoad: 2 ml, C:\\temp\\n.txt\n\n  indented line";
    assert_eq!(back[METHOD_NOTES_KEY], want);
    assert_eq!(back["Column"], "S200");
    assert_eq!(normalized_setup(&s), back);
    // saving what was read changes nothing (no spurious "unsaved changes")
    assert_eq!(normalized_setup(&back), back);

    // notes on one line, without backslashes: no comment needed
    set_method_setup(dir, 7, &setup(&[(METHOD_NOTES_KEY, "short")])).unwrap();
    assert_eq!(
        fs::read_to_string(method_setup_path(dir, 7)).unwrap(),
        format!("{METHOD_SETUP_HEADER}\nMethod notes: short\n")
    );

    // escapes only apply to the notes: older files and other fields read as before
    let old = "# Run setup (column, buffers) from the RustyCorn method editor; not sent to the instrument.\n\
               Column: C:\\new\\\\x\nPort 2: a\\nb\n";
    assert_eq!(
        parse_method_setup(old),
        setup(&[("Column", "C:\\new\\\\x"), ("Port 2", "a\\nb")])
    );
    // a lone backslash, and one at the end, are kept; blank notes are no notes
    assert_eq!(unescape_setup_value("a\\tb\\"), "a\\tb\\");
    assert_eq!(escape_setup_value("a\rb\r\nc"), "a\\nb\\nc");
    assert!(parse_method_setup("Method notes: \\n\\n\n").is_empty());

    // the notes are recorded with the method's run setup when a run of it starts
    let text = unescape_method_text(METHOD_DUMP.as_bytes());
    let dump = parse_method_dump(METHOD_DUMP.as_bytes());
    save_method_file(dir, 3, &text).unwrap();
    set_method_setup(dir, 3, &setup(&[(METHOD_NOTES_KEY, "line 1\nline 2")])).unwrap();
    let info = run_setup_for_dump(dir, &dump, 3);
    assert_eq!(info[METHOD_NOTES_KEY], "line 1\nline 2");
    assert!(!info.contains_key("Notes"));
}
