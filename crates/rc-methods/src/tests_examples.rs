//! Unit tests: the example methods of a new methods folder.

use std::fs;

use crate::*;

#[test]
fn example_method_is_valid_and_round_trips() {
    let m = example_his_tag_method();
    let (msgs, bad) = m.problems();
    assert!(msgs.is_empty() && bad.is_empty(), "{msgs:?}");
    let text = m.format_text();
    assert_eq!(MethodFile::read(&text).expect("read"), m);
    assert_eq!(m.normalized(), m);
    // a finite end, not "run until stopped"
    let end = m.rows.last().expect("end");
    assert_eq!(end[0], "60.0");
    // Autozero only at the start
    let autozero: Vec<&str> = m.rows.iter().map(|r| r[7].as_str()).collect();
    assert_eq!(autozero[0], "Y");
    assert!(autozero[1..].iter().all(|&a| a == "N"));
}

#[test]
fn example_methods_go_into_an_empty_folder() {
    let tmp = tempfile::tempdir().expect("tempdir");
    // a folder that does not exist yet counts as new
    let dir = tmp.path().join("methods");
    assert!(install_example_methods(&dir).expect("install"));
    let n = EXAMPLE_METHOD_NUMBER;
    assert_eq!(stored_methods(&dir), vec![n]);
    let path = find_method_file(&dir, n).expect("method file");
    let bytes = fs::read(&path).expect("read");
    assert_eq!(bytes, example_his_tag_method().format_text());
    assert!(
        MethodFile::read(&bytes)
            .expect("parse")
            .problems()
            .0
            .is_empty()
    );
    assert_eq!(method_names(&dir)[&n], EXAMPLE_METHOD_NAME);
    assert_eq!(method_setup(&dir, n), example_his_tag_setup());
    assert!(!dir.join(METHOD_OLD_DIR).exists());
    // the folder is in use now: a second call changes nothing
    assert!(!install_example_methods(&dir).expect("again"));
    assert_eq!(fs::read(&path).expect("read"), bytes);

    // an existing empty folder
    let empty = tmp.path().join("empty");
    fs::create_dir(&empty).expect("mkdir");
    fs::write(empty.join("notes.txt"), "not a method").expect("write");
    assert!(install_example_methods(&empty).expect("install"));
    assert_eq!(stored_methods(&empty), vec![n]);
}

#[test]
fn example_methods_never_go_into_a_folder_in_use() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = example_his_tag_method().format_text();
    // any method file, a run setup alone, names, or old/ (methods deleted)
    let cases: [(&str, &[u8]); 5] = [
        ("PrimeMeth_007.txt", &base),
        ("primemeth_002.TXT", &base),
        ("PrimeMeth_003_setup.txt", b"Column: x\n"),
        ("names.txt", b"005\tGel filtration\n"),
        ("old/PrimeMeth_001_20261020-141532.txt", &base),
    ];
    for (i, (file, bytes)) in cases.iter().enumerate() {
        let dir = tmp.path().join(format!("case{i}"));
        let path = dir.join(file);
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(&path, bytes).expect("write");
        assert!(!install_example_methods(&dir).expect("install"), "{file}");
        assert!(
            find_method_file(&dir, EXAMPLE_METHOD_NUMBER).is_none(),
            "{file}"
        );
        assert!(
            !method_names(&dir).contains_key(&EXAMPLE_METHOD_NUMBER),
            "{file}"
        );
        let mut entries: Vec<_> = fs::read_dir(&dir)
            .expect("read")
            .map(|e| e.expect("entry").file_name())
            .collect();
        entries.sort();
        assert_eq!(entries.len(), 1, "{file}: {entries:?}");
    }
    // an empty old/ alone also means the folder was used
    let dir = tmp.path().join("deleted");
    fs::create_dir_all(dir.join(METHOD_OLD_DIR)).expect("mkdir");
    assert!(!install_example_methods(&dir).expect("install"));
    assert!(stored_methods(&dir).is_empty());
}

#[test]
fn example_method_run_is_recognised() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();
    assert!(install_example_methods(dir).expect("install"));
    let n = EXAMPLE_METHOD_NUMBER;
    // what the Akta sends at run start after loading the method from the PC
    let bytes = fs::read(find_method_file(dir, n).expect("file")).expect("read");
    let dump = parse_method_dump(&escape_method_text(&bytes));
    assert_eq!(dump.breakpoints.len(), 9);
    assert_eq!(match_stored_method(dir, &dump, 0), Some(n));
    let info = run_setup_for_dump(dir, &dump, n);
    assert_eq!(info["Column"], "HisTrap HP 1 ml");
    assert_eq!(info["Column volume (mL)"], "1.0");
    assert_eq!(
        info[rc_core::runinfo::METHOD_REF_KEY],
        format!("001 – {EXAMPLE_METHOD_NAME}")
    );
    // every line can be loaded by the instrument
    assert!(
        method_file_lines(&bytes)
            .iter()
            .all(|l| l.len() <= METHOD_LINE_MAX)
    );
}
