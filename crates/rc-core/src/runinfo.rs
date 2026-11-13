//! Run documentation: sample, user, column, buffer names per buffer valve
//! port, notes, written into every log as EVT_INFO
//! events ("Sample: …"), so a CSV documents itself and an opened old run shows
//! its own information.
//!
//! Where the values come from: the column and buffer names and the method's
//! notes ("Method notes") belong to a method (its "run setup", stored next to
//! the PC method file)
//! and are recorded when the Akta starts that method; sample, user and notes are
//! asked for when a run starts. A finished log can be given more run information
//! later: it is appended, and the latest value of a field wins.

use std::collections::BTreeMap;

use crate::logtext::{parse_f64, trim_space};
use crate::types::{AktaEvent, AppState, EventKind};

pub const RUN_INFO_FIELDS: [&str; 15] = [
    "Sample",
    "User",
    "Column",
    "Column volume (mL)",
    "Buffer B",
    "Port 1",
    "Port 2",
    "Port 3",
    "Port 4",
    "Port 5",
    "Port 6",
    "Port 7",
    "Port 8",
    METHOD_NOTES_KEY,
    "Notes",
];

/// The fields that belong to a method (its run setup): column, buffers and
/// the method's notes.
pub const METHOD_SETUP_FIELDS: [&str; 12] = [
    "Column",
    "Column volume (mL)",
    "Buffer B",
    "Port 1",
    "Port 2",
    "Port 3",
    "Port 4",
    "Port 5",
    "Port 6",
    "Port 7",
    "Port 8",
    METHOD_NOTES_KEY,
];

/// Notes kept with a method (its run setup), recorded in the log of every run
/// of it. Not "Notes": that is the per-run field asked for at run start.
pub const METHOD_NOTES_KEY: &str = "Method notes";

/// The fields that can span several lines (a multi-line text box).
pub fn is_multiline_field(key: &str) -> bool {
    key == "Notes" || key == METHOD_NOTES_KEY
}

/// The fields that belong to the run itself, not to its method's run setup.
pub const PER_RUN_FIELDS: [&str; 3] = ["Sample", "User", "Notes"];

/// Which PC method the run was started from ("005 – Gel filtration"); recorded
/// with the method's run setup. Not one of RUN_INFO_FIELDS: it is not edited.
pub const METHOD_REF_KEY: &str = "PC method";

/// The label shown for a field: the buffer valve's A port and sample port are named.
pub fn field_label(key: &str) -> &str {
    match key {
        "Port 1" => "Port 1 (A)",
        "Port 8" => "Port 8 (sample)",
        _ => key,
    }
}

/// True if `text` can be a column volume: empty, or a number ≥ 0 (decimal comma allowed).
pub fn column_volume_ok(text: &str) -> bool {
    let t = trim_space(text);
    t.is_empty() || parse_f64(&t.replace(',', ".")).is_some_and(|v| v.is_finite() && v >= 0.0)
}

/// A column volume as recorded: a decimal comma becomes a point, since the CSV
/// writer turns commas into semicolons ("2,5" would read back as "2;5").
pub fn normalize_column_volume(text: &str) -> String {
    trim_space(text).replace(',', ".")
}

/// Latest value of every field recorded in `events` (EVT_INFO "Key: value").
pub fn run_info(events: &[AktaEvent]) -> BTreeMap<String, String> {
    let mut info = BTreeMap::new();
    for e in events.iter().filter(|e| e.kind == EventKind::Info) {
        if let Some((key, value)) = e.text.split_once(": ")
            && !value.is_empty()
        {
            info.insert(key.to_string(), value.to_string());
        }
    }
    info
}

/// Column volume in mL from the run information, or 0 when not given.
/// A decimal comma is accepted ("2,5").
pub fn column_volume(info: &BTreeMap<String, String>) -> f64 {
    let text = info
        .get("Column volume (mL)")
        .map(String::as_str)
        .unwrap_or("");
    match parse_f64(&text.replace(',', ".")) {
        // NaN would poison every column-volume axis; treat it as "not given"
        Some(v) if !v.is_nan() => v.max(0.0),
        _ => 0.0,
    }
}

/// Texts of the EVT_INFO events that record `info`: the method reference and
/// the RUN_INFO_FIELDS in their order, stripped, empty ones left out, and those
/// already in `recorded` with the same value skipped.
pub fn run_info_texts(
    info: &BTreeMap<String, String>,
    recorded: &BTreeMap<String, String>,
) -> Vec<String> {
    std::iter::once(METHOD_REF_KEY)
        .chain(RUN_INFO_FIELDS)
        .filter_map(|key| {
            let value = trim_space(info.get(key).map(String::as_str).unwrap_or(""));
            (!value.is_empty() && recorded.get(key).is_none_or(|v| v != value))
                .then(|| format!("{key}: {value}"))
        })
        .collect()
}

impl AppState {
    /// Record `info` in the current log as EVT_INFO events; `only_changed`
    /// skips fields already recorded with the same value.
    pub fn log_run_info(&mut self, info: &BTreeMap<String, String>, only_changed: bool) {
        let recorded = if only_changed {
            run_info(&self.events)
        } else {
            BTreeMap::new()
        };
        for text in run_info_texts(info, &recorded) {
            self.push_event(EventKind::Info, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn run_information() {
        let mut app = AppState::new();
        app.log_run_info(
            &map(&[
                ("Sample", "Lysozyme"),
                ("Column volume (mL)", "2,5"),
                ("Port 3", "Tris"),
            ]),
            false,
        );
        let info = run_info(&app.events);
        assert_eq!(info["Sample"], "Lysozyme");
        assert_eq!(info["Port 3"], "Tris");
        assert_eq!(column_volume(&info), 2.5); // decimal comma accepted
        let n = app.events.len();
        app.log_run_info(&map(&[("Sample", "Lysozyme"), ("User", "C")]), true);
        assert_eq!(app.events.len(), n + 1); // only the new field is recorded
        assert_eq!(app.events.last().map(|e| e.text.as_str()), Some("User: C"));
    }

    #[test]
    fn fields_in_fixed_order_and_stripped() {
        let mut app = AppState::new();
        app.inst.cum_time_ms = 1234.0;
        app.log_run_info(
            &map(&[
                ("Notes", "  long run \n"),
                ("Port 8", "Water"),
                ("Sample", "S1"),
                ("Unknown key", "ignored"),
                ("User", "   "),
            ]),
            false,
        );
        let texts: Vec<&str> = app.events.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, ["Sample: S1", "Port 8: Water", "Notes: long run"]);
        assert!(
            app.events
                .iter()
                .all(|e| e.kind == EventKind::Info && e.cum_time_ms == 1234.0)
        );
        // only_changed with a changed value records it again
        app.log_run_info(&map(&[("Sample", "S2"), ("Port 8", "Water")]), true);
        assert_eq!(app.events.len(), 4);
        assert_eq!(run_info(&app.events)["Sample"], "S2"); // the latest value wins
        // without only_changed everything is written again
        app.log_run_info(&map(&[("Sample", "S2")]), false);
        assert_eq!(app.events.len(), 5);
    }

    #[test]
    fn run_info_parsing() {
        let ev = |kind, text: &str| AktaEvent {
            cum_time_ms: 0.0,
            cum_vol_ml: 0.0,
            kind,
            text: text.to_string(),
        };
        let events = [
            ev(EventKind::Info, "Sample: a: b"),
            ev(EventKind::Info, "No separator"),
            ev(EventKind::Info, "Empty: "),
            ev(EventKind::Info, "Column:5 ml"),
            ev(EventKind::Log, "User: not info"),
            ev(EventKind::Info, "Port 1: Water"),
            ev(EventKind::Info, "Port 1: "),
        ];
        let info = run_info(&events);
        assert_eq!(info, map(&[("Sample", "a: b"), ("Port 1", "Water")]));
    }

    #[test]
    fn method_reference_and_setup_fields() {
        let mut app = AppState::new();
        app.log_run_info(
            &map(&[
                (METHOD_REF_KEY, "005 – Ni"),
                ("Port 1", "A"),
                ("Sample", "S"),
            ]),
            false,
        );
        let texts: Vec<&str> = app.events.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, ["PC method: 005 – Ni", "Sample: S", "Port 1: A"]);
        // the field sets cover RUN_INFO_FIELDS exactly once, and the reference is not one of them
        let mut all: Vec<&str> = METHOD_SETUP_FIELDS
            .iter()
            .chain(&PER_RUN_FIELDS)
            .copied()
            .collect();
        all.sort_unstable();
        let mut want = RUN_INFO_FIELDS.to_vec();
        want.sort_unstable();
        assert_eq!(all, want);
        assert!(!RUN_INFO_FIELDS.contains(&METHOD_REF_KEY));
        assert_eq!(field_label("Port 1"), "Port 1 (A)");
        assert_eq!(field_label("Port 8"), "Port 8 (sample)");
        assert_eq!(field_label("Column"), "Column");
    }

    #[test]
    fn method_notes_are_their_own_field() {
        // the method's notes and the run's notes are both recorded, under their own keys
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("run.csv");
        let mut app = AppState::new();
        app.log_filename = path.clone();
        app.log = Some(crate::csvlog::CsvLog::create(&path).expect("log"));
        app.log_run_info(
            &map(&[
                ("Column", "S200"),
                (METHOD_NOTES_KEY, "Equilibrate first.\nThen load, 2 ml"),
                ("Notes", "run notes"),
            ]),
            false,
        );
        let texts: Vec<&str> = app.events.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "Column: S200",
                "Method notes: Equilibrate first.\nThen load, 2 ml",
                "Notes: run notes"
            ]
        );
        let info = run_info(&app.events);
        assert_eq!(info["Notes"], "run notes");
        assert_eq!(
            info[METHOD_NOTES_KEY],
            "Equilibrate first.\nThen load, 2 ml"
        );
        // in the CSV a line break becomes a space and a comma a semicolon, as for any event
        app.push_trend(crate::TrendPoint::default());
        app.close_log();
        let back = crate::csvlog::load_csv_log(&path).expect("read");
        let info = run_info(&back.events);
        assert_eq!(info[METHOD_NOTES_KEY], "Equilibrate first. Then load; 2 ml");
        assert_eq!(info["Notes"], "run notes");
        assert!(
            METHOD_SETUP_FIELDS.contains(&METHOD_NOTES_KEY)
                && !PER_RUN_FIELDS.contains(&METHOD_NOTES_KEY)
        );
        assert!(
            is_multiline_field("Notes")
                && is_multiline_field(METHOD_NOTES_KEY)
                && !is_multiline_field("Column")
        );
    }

    #[test]
    fn column_volume_checks() {
        for ok in ["", "  ", "5", "2,5", " 0.5 ", "0", "1e1"] {
            assert!(column_volume_ok(ok), "{ok:?}");
        }
        for bad in ["abc", "5 ml", "-1", "NaN", "Inf", "2,5,1"] {
            assert!(!column_volume_ok(bad), "{bad:?}");
        }
        assert_eq!(normalize_column_volume(" 2,5 "), "2.5");
        assert_eq!(
            column_volume(&map(&[(
                "Column volume (mL)",
                &normalize_column_volume("2,5")
            )])),
            2.5
        );
    }

    #[test]
    fn column_volume_values() {
        let cv = |s: &str| column_volume(&map(&[("Column volume (mL)", s)]));
        assert_eq!(column_volume(&BTreeMap::new()), 0.0);
        assert_eq!(cv("5"), 5.0);
        assert_eq!(cv(" 1.5 "), 1.5);
        assert_eq!(cv("-3"), 0.0);
        assert_eq!(cv("5 ml"), 0.0);
        assert_eq!(cv("NaN"), 0.0);
        assert_eq!(cv(""), 0.0);
    }
}
