use std::fs;
use std::path::Path;

use rc_methods::{METHOD_END_VOLUME, MethodFile, method_names, method_path, stored_methods};

use crate::editing::*;

/// PrimeMeth_001.txt as the instrument writes it (CRLF, padded columns).
fn akta_method_text() -> Vec<u8> {
    let lines = [
        "*****************************************************************",
        "** Method Dump",
        "**",
        "** AKTAprime   Ver  , V2.01               ",
        "**",
        "** Dump Format Ver  , V1.00               ",
        "**",
        "*****************************************************************",
        "",
        "",
        "******************",
        "** Method Settings",
        "******************",
        "Method Type        ,-",
        "Method             ,-",
        "Method base        ,ml",
        "Frac base          ,ml",
        "Max pressure (kPa) ,500",
        "Alarm (min/ml)     ,0.0",
        "",
        "*********************",
        "** Method Breakpoints",
        "*********************",
        "                     ",
        "Brkp Volume   Conc Flow  Frac   Buff Inj    Peak   Autozero Eventmark",
        "                                          (mAu/min)",
        "",
        ",1   ,0.0     ,0   ,1.0  ,0.0   ,1   ,Load  ,0     ,Y       ,N",
        ",2   ,10.0    ,0   ,1.0  ,0.0   ,8   ,Load  ,0     ,N       ,N",
        ",3   ,50.0    ,100 ,1.0  ,1.0   ,1   ,Load  ,0     ,N       ,N",
    ];
    (lines.join("\r\n") + "\r\n").into_bytes()
}

fn dir_with(methods: &[u16]) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    for &n in methods {
        fs::write(method_path(tmp.path(), n), akta_method_text()).unwrap();
    }
    tmp
}

fn old_files(dir: &Path) -> usize {
    fs::read_dir(dir.join("old"))
        .map(|r| r.count())
        .unwrap_or(0)
}

#[test]
fn first_free_number() {
    assert_eq!(first_free(&[]), Some(1));
    assert_eq!(first_free(&[1, 2, 4]), Some(3));
    assert_eq!(first_free(&[2, 3]), Some(1));
    let all: Vec<u16> = (1..=999).collect();
    assert_eq!(first_free(&all), None);
}

#[test]
fn shorten_and_folder() {
    assert_eq!(shorten("abc", 5), "abc");
    assert_eq!(shorten("abcdef", 5), "abcd…");
    assert_eq!(shorten("äöüäöü", 4), "äöü…");
    assert_eq!(
        folder_text(Path::new("/data/methods")),
        "Folder: /data/methods"
    );
    let long = "/home/someone/a/very/long/path/to/the/data/folder/methods";
    let txt = folder_text(Path::new(long));
    assert!(txt.starts_with("Folder: …"));
    assert_eq!(txt.chars().count(), "Folder: …".chars().count() + 39);
    assert!(txt.ends_with("folder/methods"));
}

#[test]
fn check_messages() {
    let (t, ok) = check_text(&[]);
    assert!(ok);
    assert_eq!(t, "No problems found: the Akta can load this method.");
    let msgs: Vec<String> = (1..=6).map(|i| format!("problem {i}")).collect();
    let (t, ok) = check_text(&msgs);
    assert!(!ok);
    assert_eq!(
        t,
        "problem 1\nproblem 2\nproblem 3\nproblem 4\n… and 2 more"
    );
    assert_eq!(check_text(&msgs[..2]).0, "problem 1\nproblem 2");
}

#[test]
fn method_numbers_typed() {
    assert_eq!(parse_method_number("6"), Some(6));
    assert_eq!(parse_method_number(" 006 "), Some(6));
    assert_eq!(parse_method_number("999"), Some(999));
    for bad in ["", "0", "000", "1000", "abc", "-1", "1.0", "+5"] {
        assert_eq!(parse_method_number(bad), None, "{bad}");
    }
}

#[test]
fn column_heads() {
    let mut m = MethodFile::new_method();
    let heads: Vec<String> = rc_methods::BP_COLUMNS
        .iter()
        .map(|c| col_head(c, &m))
        .collect();
    assert_eq!(
        heads,
        [
            "Volume (ml)",
            "%B",
            "Flow (ml/min)",
            "Fraction (ml)",
            "Buffer",
            "Inj. valve",
            "Peak (mAU/min)",
            "Autozero",
            "Event mark"
        ]
    );
    m.set_setting("Method base", "min");
    m.set_setting("Frac base", "min");
    assert_eq!(col_head("Volume", &m), "Time (min)");
    assert_eq!(col_head("Frac", &m), "Fraction (min)");
}

#[test]
fn summaries() {
    let mut m = MethodFile::new_method();
    assert_eq!(summary_of(&m), ("2 breakpoints".to_string(), true));
    m.rows.truncate(1);
    assert_eq!(summary_of(&m), ("1 breakpoint".to_string(), false)); // no end breakpoint
    m.set_setting("Method Type", " Gradient ");
    assert_eq!(summary_of(&m).0, "Gradient");

    let tmp = dir_with(&[3]);
    fs::write(method_path(tmp.path(), 4), "not a method").unwrap();
    assert_eq!(
        method_summary(tmp.path(), 3),
        ("3 breakpoints".to_string(), true)
    );
    assert_eq!(
        method_summary(tmp.path(), 4),
        ("not a method file".to_string(), false)
    );
    assert_eq!(
        method_summary(tmp.path(), 5),
        ("not a method file".to_string(), false)
    );
}

#[test]
fn list_item_text() {
    let mut it = ListItem {
        number: 6,
        name: None,
        summary: "11 breakpoints".into(),
        ok: true,
        stored: true,
    };
    assert_eq!(it.text(0), "11 breakpoints");
    assert_eq!(it.text(6), "11 breakpoints · last loaded");
    it.name = Some("HisTag".into());
    it.ok = false;
    assert_eq!(it.text(6), "HisTag · last loaded · cannot be loaded");
}

#[test]
fn row_edits() {
    let mut ed = Editing::new(1, MethodFile::new_method(), false, String::new());
    assert!(!ed.is_dirty());
    // "+" on the start: a copy below without its volume
    let at = ed.add_row(0);
    assert_eq!(at, 1);
    assert_eq!(ed.file.rows.len(), 3);
    assert_eq!(ed.file.rows[1][0], "");
    assert_eq!(ed.file.rows[1][1..], ed.file.rows[0][1..]);
    assert_eq!(ed.file.rows[2][0], METHOD_END_VOLUME);
    assert!(ed.is_dirty());
    // "+" on the end extends the method
    assert_eq!(ed.add_row(2), 3);
    assert_eq!(ed.file.rows.len(), 4);
    ed.remove_row(3);
    ed.remove_row(1);
    assert_eq!(ed.file, MethodFile::new_method());
    assert!(!ed.is_dirty());
    ed.remove_row(10); // out of range: nothing
    // an empty table gets the start breakpoint
    ed.file.rows.clear();
    assert_eq!(ed.add_row(0), 0);
    assert_eq!(ed.file.rows[0], MethodFile::new_method().rows[0]);

    // the name counts, trimmed
    let mut ed = Editing::new(2, MethodFile::new_method(), true, "A".into());
    assert!(!ed.is_dirty());
    ed.name = " A ".into();
    assert!(!ed.is_dirty());
    ed.name = "B".into();
    assert!(ed.is_dirty());
    assert_eq!(ed.state_text(), "unsaved changes");

    // confirm: the second click with the same key
    assert!(!ed.confirm("delete 2"));
    assert!(!ed.confirm("new"));
    assert!(ed.confirm("new"));
    assert!(ed.pending.is_empty());
}

#[test]
fn starts_with_first_stored_or_new() {
    let empty = tempfile::tempdir().unwrap();
    let core = EditorCore::open(empty.path().to_path_buf());
    assert_eq!(core.ed.number, 1);
    assert!(!core.ed.on_disk);
    assert_eq!(core.list.len(), 1);
    assert_eq!(core.list[0].summary, "new, not saved");

    let tmp = dir_with(&[5, 2]);
    rc_methods::set_method_name(tmp.path(), 2, "Wash").unwrap();
    let core = EditorCore::open(tmp.path().to_path_buf());
    assert_eq!(core.ed.number, 2);
    assert!(core.ed.on_disk);
    assert_eq!(core.ed.name, "Wash");
    assert!(!core.ed.is_dirty());
    assert_eq!(core.ed.state_text(), "saved");
    assert_eq!(
        core.list.iter().map(|i| i.number).collect::<Vec<_>>(),
        [2, 5]
    );
    assert_eq!(core.list[0].name.as_deref(), Some("Wash"));
}

#[test]
fn save_writes_what_the_akta_writes() {
    let tmp = dir_with(&[1]);
    let dir = tmp.path();
    let mut core = EditorCore::open(dir.to_path_buf());
    // save as another number, unchanged: byte for byte the instrument's file
    core.save_as(7);
    assert_eq!(fs::read(method_path(dir, 7)).unwrap(), akta_method_text());
    assert_eq!(core.ed.number, 7);
    assert!(core.ed.on_disk && !core.ed.is_dirty());
    assert_eq!(
        core.notice.text,
        "Saved as PrimeMeth_007.txt. On the Akta: Run Stored Method, PC, number 7."
    );
    assert!(!core.notice.err);

    // values are normalised on save
    core.ed.file.rows[1][0] = "12".into();
    core.ed.file.rows[1][1] = "05".into();
    core.ed.file.rows[1][2] = "0,5".into();
    core.ed.file.rows[1][6] = "0.10".into();
    core.ed.file.set_setting("Alarm (min/ml)", "1");
    core.ed.name = "  Gradient test ".into();
    assert_eq!(core.ed.state_text(), "unsaved changes");
    core.save();
    let saved = MethodFile::read(&fs::read(method_path(dir, 7)).unwrap()).unwrap();
    assert_eq!(saved.rows[1][..3], ["12.0", "5", "0.5"]);
    assert_eq!(saved.rows[1][6], "0.1");
    assert_eq!(saved.setting("Alarm (min/ml)"), "1.0");
    assert_eq!(core.ed.file, saved); // the editor shows the saved values
    assert_eq!(
        method_names(dir).get(&7).map(String::as_str),
        Some("Gradient test")
    );
    assert_eq!(old_files(dir), 1); // the previous 007 is kept
    assert!(!core.is_dirty_for_test());

    // only the name changed: names.txt only
    let before = fs::read(method_path(dir, 7)).unwrap();
    core.ed.name = "Renamed".into();
    core.save();
    assert_eq!(core.notice.text, "Name saved.");
    assert_eq!(fs::read(method_path(dir, 7)).unwrap(), before);
    assert_eq!(old_files(dir), 1);
    assert_eq!(
        method_names(dir).get(&7).map(String::as_str),
        Some("Renamed")
    );
    core.ed.name.clear();
    core.save();
    assert_eq!(core.notice.text, "Name removed.");
    assert!(!method_names(dir).contains_key(&7));
}

impl EditorCore {
    fn is_dirty_for_test(&self) -> bool {
        self.ed.is_dirty()
    }
}

#[test]
fn save_refuses_problems() {
    let tmp = dir_with(&[1]);
    let dir = tmp.path();
    let mut core = EditorCore::open(dir.to_path_buf());
    core.ed.file.rows[1][0] = "0.0".into(); // not after breakpoint 1
    core.save();
    assert!(core.notice.err);
    assert!(
        core.notice.text.starts_with("Not saved: "),
        "{}",
        core.notice.text
    );
    assert_eq!(fs::read(method_path(dir, 1)).unwrap(), akta_method_text());
    core.ed.file.rows[1][0] = "10.0".into();
    core.ed.name = "x".repeat(61);
    core.save();
    assert_eq!(
        core.notice.text,
        "Not saved: the name may have at most 60 characters."
    );
    assert_eq!(old_files(dir), 0);
}

#[test]
fn save_as_existing_needs_confirmation() {
    let tmp = dir_with(&[1, 2]);
    let dir = tmp.path();
    let mut core = EditorCore::open(dir.to_path_buf());
    core.ed.file.rows[2][1] = "50".into();
    core.save_as_text("2");
    assert!(core.notice.err);
    assert!(core.notice.text.starts_with("Method 2 already exists."));
    assert_eq!(fs::read(method_path(dir, 2)).unwrap(), akta_method_text());
    core.save_as_text("2");
    assert_eq!(core.ed.number, 2);
    assert_eq!(
        MethodFile::read(&fs::read(method_path(dir, 2)).unwrap())
            .unwrap()
            .rows[2][1],
        "50"
    );
    assert_eq!(old_files(dir), 1);
    core.save_as_text("abc");
    assert_eq!(core.notice.text, "Enter a method number from 1 to 999.");
}

#[test]
fn new_duplicate_revert_delete() {
    let tmp = dir_with(&[1, 2]);
    let dir = tmp.path();
    rc_methods::set_method_name(dir, 1, "Load").unwrap();
    let mut core = EditorCore::open(dir.to_path_buf());

    // Duplicate: first free number, name with "(copy)", not saved
    core.duplicate();
    assert_eq!(core.ed.number, 3);
    assert!(!core.ed.on_disk);
    assert_eq!(core.ed.name, "Load (copy)");
    assert!(core.ed.is_dirty());
    assert_eq!(core.notice.text, "Method 3 is new: edit it, then Save.");
    assert!(core.list.iter().any(|i| i.number == 3 && !i.stored));

    // New while there are unsaved changes: second click
    core.new_method();
    assert_eq!(core.ed.pending, "new");
    assert_eq!(core.ed.name, "Load (copy)");
    core.new_method();
    assert_eq!(core.ed.file, MethodFile::new_method());
    assert!(core.ed.name.is_empty() && !core.ed.is_dirty());

    // opening another method with unsaved edits: second click
    core.open_method(1);
    assert_eq!(core.ed.number, 1);
    core.ed.file.rows[0][1] = "20".into();
    core.open_method(2);
    assert_eq!(core.ed.number, 1);
    assert!(core.notice.text.contains("unsaved changes"));
    core.revert();
    assert_eq!(core.notice.text, "Changes discarded.");
    assert!(!core.ed.is_dirty());
    core.open_method(2);
    assert_eq!(core.ed.number, 2);

    // Delete: second click, file to old/, name removed
    core.open_method(1);
    core.delete();
    assert!(method_path(dir, 1).exists());
    core.delete();
    assert!(!method_path(dir, 1).exists());
    assert_eq!(old_files(dir), 1);
    assert!(!method_names(dir).contains_key(&1));
    assert_eq!(core.ed.number, 2);
    assert_eq!(stored_methods(dir), [2]);
    core.delete();
    core.delete();
    assert!(stored_methods(dir).is_empty());
    assert_eq!(core.ed.number, 1);
    assert!(!core.ed.on_disk);
}

#[test]
fn refresh_follows_the_folder() {
    let tmp = dir_with(&[1]);
    let dir = tmp.path();
    let mut core = EditorCore::open(dir.to_path_buf());

    // renamed and replaced outside the editor (by the Akta or another editor)
    rc_methods::set_method_name(dir, 1, "Outside").unwrap();
    let mut m = MethodFile::read(&akta_method_text()).unwrap();
    m.rows[1][1] = "40".into();
    fs::write(method_path(dir, 1), m.format_text()).unwrap();
    fs::write(method_path(dir, 9), akta_method_text()).unwrap();
    core.refresh();
    assert_eq!(core.ed.name, "Outside");
    assert_eq!(core.ed.file, m);
    assert!(core.notice.text.contains("replaced on disk"));
    assert_eq!(core.list.len(), 2);

    // replaced while being edited: the edits stay, Save replaces that version
    core.ed.file.rows[1][1] = "60".into();
    fs::write(method_path(dir, 1), akta_method_text()).unwrap();
    core.refresh();
    assert_eq!(core.ed.file.rows[1][1], "60");
    assert!(
        core.notice.err
            && core
                .notice
                .text
                .contains("was replaced while you were editing")
    );

    // gone from the folder
    fs::remove_file(method_path(dir, 1)).unwrap();
    core.refresh();
    assert!(!core.ed.on_disk);
    assert!(core.notice.text.contains("no longer in the folder"));

    // another folder
    let other = dir_with(&[4]);
    core.set_dir(other.path().to_path_buf());
    assert_eq!(stored_methods(&core.dir), [4]);
    assert!(core.list.iter().any(|i| i.number == 4));
}

#[test]
fn editor_is_not_dirty_before_drawn() {
    let app = rc_core::AppState::new().shared();
    let ed = crate::MethodEditor::new(app);
    assert!(!ed.is_dirty());
    assert!(ed.core().is_none());
}

fn setup_of(pairs: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn run_setup_is_part_of_the_method() {
    let tmp = dir_with(&[1]);
    let dir = tmp.path();
    rc_methods::set_method_setup(dir, 1, &setup_of(&[("Column", "S200")])).unwrap();
    let mut core = EditorCore::open(dir.to_path_buf());
    assert_eq!(core.ed.setup_value("Column"), "S200");
    assert!(!core.ed.is_dirty());

    // blanks and spaces do not count as changes
    core.ed.setup.insert("Port 3".into(), "  ".into());
    core.ed.setup.insert("Column".into(), " S200 ".into());
    assert!(!core.ed.is_dirty());

    // a setup-only change saves only the setup file
    let method_before = fs::read(method_path(dir, 1)).unwrap();
    core.ed
        .setup
        .insert("Column volume (mL)".into(), "2,5".into());
    core.ed.setup.insert("Port 8".into(), "Sample".into());
    assert!(core.ed.is_dirty() && core.ed.setup_dirty());
    assert_eq!(core.ed.state_text(), "unsaved changes");
    core.save();
    assert!(!core.notice.err, "{:?}", core.notice);
    assert_eq!(core.notice.text, "Run setup saved.");
    assert_eq!(fs::read(method_path(dir, 1)).unwrap(), method_before);
    assert_eq!(old_files(dir), 0);
    // the decimal comma is written as a point
    assert_eq!(
        rc_methods::method_setup(dir, 1),
        setup_of(&[
            ("Column", "S200"),
            ("Column volume (mL)", "2.5"),
            ("Port 8", "Sample")
        ])
    );
    assert!(!core.ed.is_dirty());

    // name and setup together, still without touching the method file
    core.ed.name = "Gel filtration".into();
    core.ed.setup.insert("Buffer B".into(), "1 M NaCl".into());
    core.save();
    assert_eq!(core.notice.text, "Name and run setup saved.");
    assert_eq!(fs::read(method_path(dir, 1)).unwrap(), method_before);
    assert_eq!(method_names(dir)[&1], "Gel filtration");

    // removing every field removes the file
    core.ed.setup.clear();
    core.save();
    assert!(rc_methods::find_method_setup_file(dir, 1).is_none());
}

#[test]
fn column_volume_must_be_a_number() {
    let tmp = dir_with(&[1]);
    let mut core = EditorCore::open(tmp.path().to_path_buf());
    core.ed.setup.insert(COLUMN_VOLUME.into(), "5 ml".into());
    assert_eq!(core.ed.setup_problems().len(), 1);
    core.save();
    assert!(
        core.notice.err && core.notice.text.contains("Column volume"),
        "{:?}",
        core.notice
    );
    assert!(rc_methods::method_setup(tmp.path(), 1).is_empty());
    // also with a changed method: nothing is written
    core.ed.file.rows[0][2] = "2.0".into();
    let before = fs::read(method_path(tmp.path(), 1)).unwrap();
    core.save();
    assert!(core.notice.err);
    assert_eq!(fs::read(method_path(tmp.path(), 1)).unwrap(), before);
    core.ed.setup.insert(COLUMN_VOLUME.into(), " 24,0 ".into());
    assert!(core.ed.setup_problems().is_empty());
    core.save();
    assert!(!core.notice.err, "{:?}", core.notice);
    assert_eq!(
        rc_methods::method_setup(tmp.path(), 1)[COLUMN_VOLUME],
        "24.0"
    );
}

#[test]
fn run_setup_follows_save_as_duplicate_and_new() {
    let tmp = dir_with(&[1, 2]);
    let dir = tmp.path();
    rc_methods::set_method_setup(dir, 1, &setup_of(&[("Column", "A")])).unwrap();
    rc_methods::set_method_setup(dir, 2, &setup_of(&[("Column", "B")])).unwrap();
    let mut core = EditorCore::open(dir.to_path_buf());
    assert_eq!(core.ed.number, 1);

    // a full save keeps the setup with the method (the old pair goes to old/)
    core.ed.file.rows[1][2] = "2.0".into();
    core.ed.setup.insert("Port 1".into(), "Water".into());
    core.save();
    assert!(!core.notice.err, "{:?}", core.notice);
    assert_eq!(
        rc_methods::method_setup(dir, 1),
        setup_of(&[("Column", "A"), ("Port 1", "Water")])
    );
    assert_eq!(old_files(dir), 2); // method and its setup

    // save as another (existing) number: the edited setup goes there, the source keeps its own
    core.ed.setup.insert("Column".into(), "C".into());
    core.save_as(2);
    assert!(core.notice.err); // needs confirmation
    core.save_as(2);
    assert!(!core.notice.err, "{:?}", core.notice);
    assert_eq!(core.ed.number, 2);
    assert_eq!(
        rc_methods::method_setup(dir, 2),
        setup_of(&[("Column", "C"), ("Port 1", "Water")])
    );
    assert_eq!(
        rc_methods::method_setup(dir, 1),
        setup_of(&[("Column", "A"), ("Port 1", "Water")])
    );
    assert_eq!(old_files(dir), 4); // also method 2 and its setup "B"

    // duplicate: a new method with the same setup, unsaved
    core.duplicate();
    assert_eq!(core.ed.number, 3);
    assert!(!core.ed.on_disk && core.ed.is_dirty());
    assert_eq!(core.ed.setup_value("Column"), "C");
    assert!(rc_methods::find_method_setup_file(dir, 3).is_none());
    core.save();
    assert!(!core.notice.err, "{:?}", core.notice);
    assert_eq!(rc_methods::method_setup(dir, 3)["Column"], "C");

    // new: an empty setup; a new method with only a setup is unsaved work
    core.new_method();
    assert_eq!(core.ed.number, 4);
    assert!(core.ed.setup.is_empty() && !core.ed.is_dirty());
    core.ed.setup.insert("Column".into(), "D".into());
    assert!(core.ed.is_dirty());
    // revert drops it
    core.revert();
    assert!(core.ed.setup.is_empty() && !core.ed.is_dirty());

    // delete moves the setup to old/ with the method
    core.open_method(3);
    core.delete();
    core.delete();
    assert!(rc_methods::find_method_setup_file(dir, 3).is_none());
    assert_eq!(old_files(dir), 6);
}

#[test]
fn run_setup_changed_on_disk() {
    let tmp = dir_with(&[1]);
    let dir = tmp.path();
    let mut core = EditorCore::open(dir.to_path_buf());
    rc_methods::set_method_setup(dir, 1, &setup_of(&[("Column", "X")])).unwrap();
    core.refresh();
    assert_eq!(core.ed.setup_value("Column"), "X");
    assert!(!core.ed.is_dirty());
    // with edits in the editor: kept, and still unsaved
    core.ed.setup.insert("Column".into(), "mine".into());
    rc_methods::set_method_setup(dir, 1, &setup_of(&[("Column", "Y")])).unwrap();
    core.refresh();
    assert_eq!(core.ed.setup_value("Column"), "mine");
    assert!(core.ed.is_dirty());
    core.revert();
    assert_eq!(core.ed.setup_value("Column"), "Y");
}

#[test]
fn setup_summaries() {
    assert!(setup_summary(&setup_of(&[("Port 2", " ")])).starts_with("Not set"));
    assert_eq!(
        setup_summary(&setup_of(&[
            ("Column", "S200"),
            ("Column volume (mL)", "24"),
            ("Buffer B", "NaCl"),
            ("Port 1", "A"),
            ("Port 8", "S")
        ])),
        "S200  ·  24 mL  ·  B: NaCl  ·  ports 1, 8 named"
    );
    assert_eq!(setup_summary(&setup_of(&[("Port 3", "x")])), "port 3 named");
}

#[test]
fn method_notes_are_part_of_the_run_setup() {
    use rc_core::runinfo::METHOD_NOTES_KEY;
    let tmp = dir_with(&[1, 2]);
    let dir = tmp.path();
    let mut core = EditorCore::open(dir.to_path_buf());
    assert_eq!(core.ed.setup_value(METHOD_NOTES_KEY), "");
    let method_before = fs::read(method_path(dir, 1)).unwrap();

    // typing notes is an unsaved change; Save writes only the setup file
    let notes = "Equilibrate 5 CV first.\nLoad 2 ml at 0.5 ml/min";
    core.ed.setup.insert(METHOD_NOTES_KEY.into(), notes.into());
    assert!(core.ed.is_dirty() && core.ed.setup_dirty());
    core.save();
    assert_eq!(core.notice.text, "Run setup saved.");
    assert_eq!(fs::read(method_path(dir, 1)).unwrap(), method_before);
    assert_eq!(rc_methods::method_setup(dir, 1)[METHOD_NOTES_KEY], notes);
    // read back after the save, with its line break: not dirty
    assert_eq!(core.ed.setup_value(METHOD_NOTES_KEY), notes);
    assert!(!core.ed.is_dirty());
    // a reopened editor shows them as saved
    let again = EditorCore::open(dir.to_path_buf());
    assert_eq!(again.ed.setup_value(METHOD_NOTES_KEY), notes);
    assert!(!again.ed.is_dirty());
    // trailing blank lines and spaces are no change
    core.ed
        .setup
        .insert(METHOD_NOTES_KEY.into(), format!("{notes}\n\n  "));
    assert!(!core.ed.is_dirty());
    // another line is
    core.ed
        .setup
        .insert(METHOD_NOTES_KEY.into(), format!("{notes}\n\nWash after"));
    assert!(core.ed.is_dirty());
    assert_eq!(core.ed.state_text(), "unsaved changes");
    core.revert();
    assert_eq!(core.ed.setup_value(METHOD_NOTES_KEY), notes);

    // save as: the notes go with the method to the other number
    core.ed
        .setup
        .insert(METHOD_NOTES_KEY.into(), "copied\nnotes".into());
    core.save_as(2);
    core.save_as(2); // (replacing needs a second click)
    assert!(!core.notice.err, "{:?}", core.notice);
    assert_eq!(
        rc_methods::method_setup(dir, 2)[METHOD_NOTES_KEY],
        "copied\nnotes"
    );
    assert_eq!(rc_methods::method_setup(dir, 1)[METHOD_NOTES_KEY], notes);

    // duplicate: the new method has them, and saving writes them
    core.duplicate();
    assert_eq!(core.ed.number, 3);
    assert_eq!(core.ed.setup_value(METHOD_NOTES_KEY), "copied\nnotes");
    core.save();
    assert!(!core.notice.err, "{:?}", core.notice);
    assert_eq!(
        rc_methods::method_setup(dir, 3)[METHOD_NOTES_KEY],
        "copied\nnotes"
    );

    // emptying the notes removes them (and the file, when nothing else is set)
    core.ed.setup.insert(METHOD_NOTES_KEY.into(), "  \n".into());
    assert!(core.ed.is_dirty());
    core.save();
    assert!(rc_methods::find_method_setup_file(dir, 3).is_none());

    assert_eq!(
        setup_summary(&setup_of(&[("Column", "S200"), (METHOD_NOTES_KEY, "x")])),
        "S200  ·  notes"
    );
}
