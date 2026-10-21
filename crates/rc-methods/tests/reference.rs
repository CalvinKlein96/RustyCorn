//! Reference tests: `tests/data/reference.tsv` holds known-good outputs of the
//! method text codec, method files and the method store for fixed inputs, one
//! record per line (tab-separated, kind first, byte strings hex-encoded). They
//! pin the file formats and the instrument exchange byte for byte.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use rc_core::AppState;
use rc_methods::*;

const REFERENCE: &str = include_str!("data/reference.tsv");

fn unhex(s: &str) -> Vec<u8> {
    assert!(s.len().is_multiple_of(2), "odd hex: {s}");
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex_str(s: &str) -> String {
    String::from_utf8(unhex(s)).expect("utf-8")
}

fn records(kind: &str) -> Vec<Vec<&'static str>> {
    let recs: Vec<Vec<&str>> = REFERENCE
        .lines()
        .map(|l| l.split('\t').collect::<Vec<_>>())
        .filter(|f| f[0] == kind)
        .collect();
    assert!(!recs.is_empty(), "no reference records of kind {kind}");
    recs
}

fn msgs_hex(ms: &[rc_core::opc::OpcMessage]) -> String {
    ms.iter()
        .map(|(o, d)| format!("{o:04x}:{}", hex(d)))
        .collect::<Vec<_>>()
        .join(";")
}

/// MethodFile in the reference data's form: "S<k>:<v>" settings, "R:<v>:<v>…" rows.
fn mf_canon(m: &MethodFile) -> String {
    let mut items: Vec<String> = m
        .settings
        .iter()
        .map(|(k, v)| format!("S{}:{}", hex(k.as_bytes()), hex(v.as_bytes())))
        .collect();
    items.extend(m.rows.iter().map(|r| {
        format!(
            "R{}",
            r.iter()
                .map(|v| format!(":{}", hex(v.as_bytes())))
                .collect::<String>()
        )
    }));
    items.join(" ")
}

fn mf_uncanon(s: &str) -> MethodFile {
    let mut m = MethodFile::default();
    for item in s.split(' ').filter(|i| !i.is_empty()) {
        if let Some(kv) = item.strip_prefix('S') {
            let (k, v) = kv.split_once(':').expect("setting");
            m.settings.push((unhex_str(k), unhex_str(v)));
        } else if let Some(r) = item.strip_prefix('R') {
            m.rows.push(r.split(':').skip(1).map(unhex_str).collect());
        } else {
            panic!("bad item {item}");
        }
    }
    m
}

fn dump_canon(m: &MethodDump) -> String {
    let mut items: Vec<String> = m
        .settings
        .iter()
        .map(|(k, v)| format!("S{}:{}", hex(k.as_bytes()), hex(v.as_bytes())))
        .collect();
    items.extend(m.columns.iter().map(|c| format!("C{}", hex(c.as_bytes()))));
    items.extend(m.breakpoints.iter().map(|bp| {
        format!(
            "B{}",
            bp.iter()
                .map(|(k, v)| format!(":{}={}", hex(k.as_bytes()), hex(v.as_bytes())))
                .collect::<String>()
        )
    }));
    items.join(" ")
}

#[test]
fn codec() {
    for r in records("escape") {
        assert_eq!(
            hex(&escape_method_text(&unhex(r[1]))),
            r[2],
            "escape {}",
            r[1]
        );
    }
    for r in records("unescape") {
        assert_eq!(
            hex(&unescape_method_text(&unhex(r[1]))),
            r[2],
            "unescape {}",
            r[1]
        );
    }
    for r in records("linemsgs") {
        assert_eq!(
            msgs_hex(&method_line_messages(&unhex(r[1]))),
            r[2],
            "line messages {}",
            r[1]
        );
    }
    for r in records("filelines") {
        let lines: Vec<String> = method_file_lines(&unhex(r[1]))
            .iter()
            .map(|l| hex(l))
            .collect();
        assert_eq!(lines.join(";"), r[2], "file lines {}", r[1]);
    }
}

#[test]
fn list_payload() {
    for r in records("listpayload") {
        let nums: Vec<u16> = r[1]
            .split(',')
            .filter(|s| !s.is_empty())
            .map(|s| s.parse().unwrap())
            .collect();
        let start: u16 = r[2].parse().unwrap();
        let got = method_list_payload(&nums, start, r[3] == "1");
        assert_eq!(hex(&got), r[4], "list {} from {start} up={}", r[1], r[3]);
    }
}

#[test]
fn values() {
    for r in records("value") {
        let (name, s) = (unhex_str(r[1]), unhex_str(r[2]));
        if r[3] != "ERR" {
            assert_eq!(
                method_value_ok(&name, &s),
                r[3] == "1",
                "method_value_ok({name:?}, {s:?})"
            );
        }
        if r[4] == "ERR" {
            // ERR: a number field holding no number cannot be normalised; the text
            // is kept as it is.
            assert_eq!(
                normalize_value(&name, &s),
                s,
                "normalize_value({name:?}, {s:?})"
            );
        } else {
            assert_eq!(
                normalize_value(&name, &s),
                unhex_str(r[4]),
                "normalize_value({name:?}, {s:?})"
            );
        }
    }
    for r in records("choices") {
        let got = value_choices(&unhex_str(r[1])).map(|c| {
            c.iter()
                .map(|v| hex(v.as_bytes()))
                .collect::<Vec<_>>()
                .join(",")
        });
        assert_eq!(
            got.as_deref().unwrap_or("-"),
            r[2],
            "choices {}",
            unhex_str(r[1])
        );
    }
}

#[test]
fn names() {
    for r in records("nameok") {
        let s = unhex_str(r[1]);
        // Unassigned code points (Unicode category Cn) are not printable, but
        // Rust's std has no category table, so `method_name_ok` lets them pass.
        if s == "\u{378}" {
            assert!(method_name_ok(&s));
            continue;
        }
        assert_eq!(method_name_ok(&s), r[2] == "1", "method_name_ok({s:?})");
    }
    for r in records("names") {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(METHOD_NAMES_FILE), unhex(r[1])).unwrap();
        let got: Vec<String> = method_names(dir.path())
            .iter()
            .map(|(k, v)| format!("{k}={}", hex(v.as_bytes())))
            .collect();
        assert_eq!(got.join(";"), r[2], "names.txt {:?}", unhex_str(r[1]));
    }
    let dir = tempfile::tempdir().unwrap();
    for r in records("setname") {
        let n: u16 = r[1].parse().unwrap();
        set_method_name(dir.path(), n, &unhex_str(r[2])).unwrap();
        let p = dir.path().join(METHOD_NAMES_FILE);
        let got = if p.is_file() {
            hex(&fs::read(&p).unwrap())
        } else {
            "-".into()
        };
        assert_eq!(
            got,
            r[3],
            "names.txt after naming {n} {:?}",
            unhex_str(r[2])
        );
    }
    // only names.txt is left behind (no temporary files)
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn parse_and_apply() {
    for r in records("parse") {
        let text = unhex(r[1]);
        let m = parse_method_text(&text);
        assert_eq!(
            dump_canon(&m),
            r[2],
            "parse_method_text {:?}",
            String::from_utf8_lossy(&text)
        );
        let mut app = AppState::new();
        apply_method_dump(&mut app, &m);
        assert_eq!(app.inst.buffer_port.to_string(), r[3]);
        assert_eq!(app.inst.buffer_port_known, r[4] == "1");
        assert_eq!(app.inst.inject_value.name(), r[5]);
        let ev = app
            .pending_method_event
            .as_deref()
            .map(|e| hex(e.as_bytes()));
        assert_eq!(
            ev.as_deref().unwrap_or("-"),
            r[6],
            "pending event for {:?}",
            String::from_utf8_lossy(&text)
        );
    }
    for r in records("parsedump") {
        assert_eq!(dump_canon(&parse_method_dump(&unhex(r[1]))), r[2]);
    }
}

#[test]
fn method_files() {
    for r in records("newmethod") {
        assert_eq!(mf_canon(&MethodFile::new_method()), r[1]);
        assert_eq!(hex(&MethodFile::new_method().format_text()), r[2]);
    }
    for r in records("read") {
        let bytes = unhex(r[1]);
        match MethodFile::read(&bytes) {
            Ok(m) => {
                assert_eq!(
                    r[2],
                    "OK",
                    "read should fail: {:?}",
                    String::from_utf8_lossy(&bytes)
                );
                assert_eq!(
                    mf_canon(&m),
                    r[3],
                    "read {:?}",
                    String::from_utf8_lossy(&bytes)
                );
                assert_eq!(
                    hex(&m.format_text()),
                    r[4],
                    "format of {:?}",
                    String::from_utf8_lossy(&bytes)
                );
            }
            Err(e) => {
                assert_eq!(
                    r[2],
                    "ERR",
                    "read failed ({e}): {:?}",
                    String::from_utf8_lossy(&bytes)
                );
                assert_eq!(e, unhex_str(r[3]));
            }
        }
    }
    for r in records("mfile") {
        let m = mf_uncanon(r[1]);
        assert_eq!(mf_canon(&m), r[1]);
        assert_eq!(hex(&m.format_text()), r[2], "format {m:?}");
        if r[3] != "ERR" {
            assert_eq!(mf_canon(&m.normalized()), r[3], "normalized {m:?}");
        }
        if r[4] != "ERR" {
            let (msgs, bad) = m.problems();
            let msgs: Vec<String> = msgs.iter().map(|s| hex(s.as_bytes())).collect();
            assert_eq!(msgs.join(" "), r[4], "problems {m:?}");
            let bad: Vec<String> = bad
                .iter()
                .map(|(i, c)| format!("{i}:{}", hex(c.as_bytes())))
                .collect();
            assert_eq!(bad.join(" "), r[5], "bad cells {m:?}");
        }
        assert_eq!(hex(m.setting("Max pressure (kPa)").as_bytes()), r[6]);
        assert_eq!(m.setting("Nope"), unhex_str(r[7]));
    }
}

fn mode_name(m: TransferMode) -> &'static str {
    match m {
        TransferMode::None => "none",
        TransferMode::Dump => "dump",
        TransferMode::Save => "save",
    }
}

fn dir_files(dir: &Path) -> String {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|f| dir.join(f).is_file())
        .collect();
    names.sort();
    names
        .iter()
        .map(|f| {
            format!(
                "{}={}",
                hex(f.as_bytes()),
                hex(&fs::read(dir.join(f)).unwrap())
            )
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// A recorded method store exchange (requests, replies, folder contents), replayed in a
/// fresh folder.
#[test]
fn store_exchange() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let dir_text = dir.display().to_string();
    let ts = regex::Regex::new(r"_\d{8}-\d{6}").unwrap();
    let mut x = MethodTransfer::default();
    let mut steps = 0;
    for line in REFERENCE.lines() {
        let r: Vec<&str> = line.split('\t').collect();
        match r[0] {
            "step" => {
                steps += 1;
                let opc = u16::from_str_radix(r[1], 16).unwrap();
                let data = unhex(r[2]);
                let what = format!("step {steps}: 0x{opc:04x} [{}]", r[2]);
                // Loading a *directory* named PrimeMeth_030.txt: the reference records
                // an empty method sent as loaded; we report that it could not be read
                // (0x2014 [1]) instead.
                if opc == 0x2012 && data == [30, 0] {
                    let a = x.handle(opc, &data, dir).unwrap();
                    assert_eq!(a.replies, vec![(0x2014, vec![1])]);
                    assert_eq!(a.note.as_deref(), Some("Method 30 could not be read"));
                    continue;
                }
                match x.handle(opc, &data, dir) {
                    None => assert_eq!(r[3], "NOTHING", "{what}"),
                    Some(a) => {
                        assert_eq!(msgs_hex(&a.replies), r[3], "replies, {what}");
                        let note = a
                            .note
                            .map(|n| hex(n.replace(&dir_text, "<DIR>").as_bytes()));
                        assert_eq!(
                            note.as_deref().unwrap_or("-"),
                            r[4],
                            "note, {what}: {}",
                            if r[4] == "-" {
                                "-".into()
                            } else {
                                unhex_str(r[4])
                            }
                        );
                        assert_eq!(
                            a.dump.map(|d| hex(&d)).as_deref().unwrap_or("-"),
                            r[5],
                            "dump, {what}"
                        );
                    }
                }
                assert_eq!(mode_name(x.mode), r[6], "mode, {what}");
                assert_eq!(x.number.to_string(), r[7], "number, {what}");
                assert_eq!(hex(&x.buf), r[8], "buffer, {what}");
            }
            "file" => fs::write(dir.join(unhex_str(r[1])), unhex(r[2])).unwrap(),
            "mkdir" => fs::create_dir(dir.join(unhex_str(r[1]))).unwrap(),
            "stored" => {
                let got: Vec<String> = stored_methods(dir).iter().map(u16::to_string).collect();
                assert_eq!(got.join(","), r[1], "stored methods after step {steps}");
            }
            "files" => assert_eq!(dir_files(dir), r[1], "folder contents after step {steps}"),
            "old" => {
                let old = dir.join(METHOD_OLD_DIR);
                let got = if old.is_dir() {
                    let mut v: Vec<String> = fs::read_dir(&old)
                        .unwrap()
                        .map(|e| {
                            ts.replace(&e.unwrap().file_name().into_string().unwrap(), "_TS")
                                .into_owned()
                        })
                        .collect();
                    v.sort();
                    v.join(",")
                } else {
                    "-".into()
                };
                assert_eq!(got, r[1], "old/ after step {steps}");
            }
            _ => {}
        }
    }
    assert!(steps > 100);
}

/// The instrument's own files read and write back byte for byte.
#[test]
fn instrument_files_round_trip() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/instrument");
    let mut n = 0;
    let mut by_name = BTreeMap::new();
    for e in fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        let bytes = fs::read(&p).unwrap();
        let m = MethodFile::read(&bytes).unwrap();
        assert_eq!(m.format_text(), bytes, "{}", p.display());
        by_name.insert(p.file_name().unwrap().to_string_lossy().into_owned(), m);
        n += 1;
    }
    assert!(n >= 2);
    // what the instrument wrote is what the editor writes: normalising changes nothing
    for (name, m) in &by_name {
        assert_eq!(&m.normalized(), m, "{name}");
    }
}
