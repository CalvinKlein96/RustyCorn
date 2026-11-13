//! Run information forms, every field in each. The column and buffers come
//! with the PC method a run starts from (its run setup, written in the method
//! editor) and are filled in; a method started from the instrument's own memory
//! has none, so they can be given here too:
//!
//! * when a run starts, a window (not modal: recording goes on) asks for the
//!   run information, filled in with what is recorded (the run setup) and the
//!   last user; it stays until answered and moves on to a newer run if one
//!   starts first. The Live page's Run info card reopens it ("Edit") while the
//!   run goes on; changed fields are recorded in the log at once.
//! * an opened run (a finished log) can be given any run information later
//!   ("Edit run info"); it is appended to the end of the CSV file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use egui::{Align, Align2, Layout, Margin, RichText, Ui};
use parking_lot::Mutex;
use rc_core::AppState;
use rc_core::runinfo::{
    METHOD_NOTES_KEY, METHOD_SETUP_FIELDS, RUN_INFO_FIELDS, column_volume_ok, field_label,
    is_multiline_field, normalize_column_volume, run_info,
};
use rc_theme::{FS_BODY, FS_SMALL, medium, palette, semibold};

use crate::console::display_name;

/// What a form is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormKind {
    /// a run has just started
    NewRun,
    /// the Live page's "Edit": the current run, while it goes on
    LiveEdit,
    /// an opened log: every run information field, appended to the file
    Viewer,
}

/// A run information form being filled in.
#[derive(Clone, Debug, PartialEq)]
pub struct InfoForm {
    pub kind: FormKind,
    /// the log the values are recorded in
    pub log: PathBuf,
    /// the fields as typed (keys from `keys()`)
    pub fields: BTreeMap<String, String>,
    /// why the last Save did not work
    pub error: String,
}

const COLUMN_VOLUME: &str = "Column volume (mL)";

impl InfoForm {
    fn with(kind: FormKind, log: &Path, values: &BTreeMap<String, String>) -> InfoForm {
        let fields = RUN_INFO_FIELDS
            .iter()
            .map(|k| (k.to_string(), values.get(*k).cloned().unwrap_or_default()))
            .collect();
        InfoForm {
            kind,
            log: log.to_path_buf(),
            fields,
            error: String::new(),
        }
    }

    /// A new run: what is recorded for it so far (its method's run setup) and
    /// the last user name.
    pub fn new_run(log: &Path, recorded: &BTreeMap<String, String>, last_user: &str) -> InfoForm {
        InfoForm::live(FormKind::NewRun, log, recorded, last_user)
    }

    /// The current run's information as recorded (the last user if none).
    pub fn live_edit(log: &Path, recorded: &BTreeMap<String, String>, last_user: &str) -> InfoForm {
        InfoForm::live(FormKind::LiveEdit, log, recorded, last_user)
    }

    fn live(
        kind: FormKind,
        log: &Path,
        recorded: &BTreeMap<String, String>,
        last_user: &str,
    ) -> InfoForm {
        let mut values = recorded.clone();
        values
            .entry("User".to_string())
            .or_insert_with(|| last_user.to_string());
        InfoForm::with(kind, log, &values)
    }

    /// Every field of an opened run, with its latest recorded value.
    pub fn viewer(log: &Path, recorded: &BTreeMap<String, String>) -> InfoForm {
        InfoForm::with(FormKind::Viewer, log, recorded)
    }

    pub fn title(&self) -> String {
        let name = display_name(&self.log);
        match self.kind {
            FormKind::NewRun => format!("New run started — {name}"),
            FormKind::LiveEdit => format!("Run info — {name}"),
            FormKind::Viewer => format!("Edit run info — {name}"),
        }
    }

    /// The values to record: trimmed, a decimal comma in the column volume
    /// written as a point (the CSV writer would turn a comma into ';').
    pub fn values(&self) -> BTreeMap<String, String> {
        self.fields
            .iter()
            .map(|(k, v)| {
                let v = if k == COLUMN_VOLUME {
                    normalize_column_volume(v)
                } else {
                    v.trim().to_string()
                };
                (k.clone(), v)
            })
            .collect()
    }

    /// Problems that stop a save.
    pub fn problems(&self) -> Vec<String> {
        match self.fields.get(COLUMN_VOLUME) {
            Some(cv) if !column_volume_ok(cv) => vec![format!(
                "{COLUMN_VOLUME}: \"{}\" is not a number",
                cv.trim()
            )],
            _ => Vec::new(),
        }
    }

    /// The user name to remember for the next run, if one was given.
    pub fn user(&self) -> Option<String> {
        self.fields
            .get("User")
            .map(|u| u.trim().to_string())
            .filter(|u| !u.is_empty())
    }
}

/// The Live page's form: asks at every new run (`AppState::run_serial`).
#[derive(Debug, Default)]
pub struct LivePrompt {
    seen_serial: u64,
    pub form: Option<InfoForm>,
}

impl LivePrompt {
    /// Follow the live state: a new run opens the form (with what is recorded
    /// for it), or moves an unanswered one to the new run: what was typed is
    /// kept, but the new run's own run setup replaces the column and buffers.
    /// True when it did.
    pub fn update(
        &mut self,
        run_serial: u64,
        log: &Path,
        recorded: &BTreeMap<String, String>,
        last_user: &str,
    ) -> bool {
        if run_serial == self.seen_serial {
            return false;
        }
        self.seen_serial = run_serial;
        match &mut self.form {
            Some(form) => {
                form.kind = FormKind::NewRun;
                form.log = log.to_path_buf();
                form.error.clear();
                for key in METHOD_SETUP_FIELDS {
                    if let Some(v) = recorded.get(key) {
                        form.fields.insert(key.to_string(), v.clone());
                    }
                }
            }
            None => self.form = Some(InfoForm::new_run(log, recorded, last_user)),
        }
        true
    }

    /// The last run the form followed (`update`).
    pub fn seen_serial(&self) -> u64 {
        self.seen_serial
    }

    /// The Run info card's "Edit": the current run's form (an open one is kept).
    pub fn edit(&mut self, log: &Path, recorded: &BTreeMap<String, String>, last_user: &str) {
        if self.form.as_ref().is_some_and(|f| f.log == log) {
            return;
        }
        self.form = Some(InfoForm::live_edit(log, recorded, last_user));
    }
}

/// True if `a` and `b` name the same file.
pub fn same_file(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// Record a live form: in the current log when it is still the run's log
/// (only changed fields), else appended to that run's (now finished) log file.
/// The finished log is read and written without holding the app lock.
/// Returns what was done.
pub fn save_live(app: &Mutex<AppState>, form: &InfoForm) -> Result<String, String> {
    if let Some(first) = form.problems().first() {
        return Err(first.clone());
    }
    let values = form.values();
    let name = display_name(&form.log);
    {
        let mut app = app.lock();
        if app.log.is_some() && same_file(&app.log_filename, &form.log) {
            let before = app.events.len();
            app.log_run_info(&values, true);
            let n = app.events.len() - before;
            return Ok(if n == 0 {
                "Nothing new to record.".to_string()
            } else {
                format!("Recorded in {name}.")
            });
        }
    }
    // the run ended and its log was closed (e.g. the acquisition restarted)
    let mut old = rc_core::csvlog::load_csv_log(&form.log)
        .map_err(|e| format!("{name} cannot be read: {e}"))?;
    old.append_run_info(&values)
        .map_err(|e| format!("{name} cannot be written: {e}"))?;
    Ok(format!("Added to {name}."))
}

/// Record a viewer form: append the changed fields to the opened log `app`
/// (its file and its events). Refused for the log being recorded (`recording`).
pub fn save_viewer(
    app: &mut AppState,
    form: &InfoForm,
    recording: Option<&Path>,
) -> Result<String, String> {
    let name = display_name(&form.log);
    if recording.is_some_and(|r| same_file(r, &form.log)) {
        return Err(format!(
            "{name} is the log being recorded now: edit its run info on the Live page."
        ));
    }
    if let Some(first) = form.problems().first() {
        return Err(first.clone());
    }
    let n = app
        .append_run_info(&form.values())
        .map_err(|e| format!("{name} cannot be written: {e}"))?;
    Ok(if n == 0 {
        "Nothing changed.".to_string()
    } else {
        format!("Added {n} field(s) to the end of {name}.")
    })
}

/// Run information recorded in `app`.
pub fn recorded(app: &AppState) -> BTreeMap<String, String> {
    run_info(&app.events)
}

/// What the user did with a form this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormAction {
    None,
    Save,
    /// Skip (new run) or Cancel
    Close,
}

/// Draw `form` as a window (not modal) over the page.
pub fn show_form(ctx: &egui::Context, form: &mut InfoForm, id: egui::Id) -> FormAction {
    let t = palette(ctx);
    // the live form near the top, an opened run's in the middle: both can be open
    let centred = form.kind == FormKind::Viewer;
    let width = 640.0;
    let screen = ctx.content_rect();
    let mut action = FormAction::None;
    egui::Window::new(form.title())
        .id(id)
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .pivot(if centred {
            Align2::CENTER_CENTER
        } else {
            Align2::CENTER_TOP
        })
        .default_pos(if centred {
            screen.center()
        } else {
            egui::pos2(screen.center().x, screen.top() + 72.0)
        })
        .frame(
            egui::Frame::window(&ctx.global_style())
                .fill(t.card)
                .inner_margin(Margin {
                    left: 20,
                    right: 20,
                    top: 16,
                    bottom: 14,
                }),
        )
        .show(ctx, |ui| {
            ui.set_width(width);
            header(ui, form);
            ui.add_space(10.0);
            fields(ui, form);
            ui.add_space(6.0);
            let problems = form.problems();
            if let Some(p) = problems.first() {
                ui.label(RichText::new(p).font(small_font()).color(t.err));
            } else if !form.error.is_empty() {
                ui.add(egui::Label::new(RichText::new(&form.error).color(t.err)).wrap());
            }
            ui.add_space(6.0);
            action = buttons(ui, form.kind);
        });
    action
}

fn small_font() -> egui::FontId {
    egui::FontId::proportional(FS_SMALL + 0.5)
}

fn header(ui: &mut Ui, form: &InfoForm) {
    let t = palette(ui.ctx());
    let (title, note) = match form.kind {
        FormKind::NewRun => (
            "New run started",
            "Recording goes on. Column and buffers come from the PC method's run setup; \
             for a method started on the Akta, give them here. All can be changed while \
             the run goes on (Run info card: Edit).",
        ),
        FormKind::LiveEdit => (
            "Run info",
            "The current run. Changed fields are recorded in the log at once; an emptied \
             field keeps its old value.",
        ),
        FormKind::Viewer => (
            "Edit run info",
            "Changed fields are added to the end of the log; nothing in it is rewritten. \
             An emptied field keeps its old value.",
        ),
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).font(semibold(16.0)).color(t.text));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let name = display_name(&form.log);
            ui.add(
                egui::Label::new(
                    RichText::new(&name)
                        .font(rc_theme::mono(FS_SMALL))
                        .color(t.text_dim),
                )
                .truncate(),
            )
            .on_hover_text(form.log.display().to_string());
        });
    });
    ui.add(egui::Label::new(RichText::new(note).font(small_font()).color(t.text_dim)).wrap());
}

/// Labels and fields in a grid; the notes get several lines.
fn grid(ui: &mut Ui, form: &mut InfoForm, keys: &[&str], width: f32, id: &str) {
    let t = palette(ui.ctx());
    let label_w = 112.0;
    let field_w = (width - label_w - 14.0).max(120.0);
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([14.0, 8.0])
        .min_col_width(label_w)
        .show(ui, |ui| {
            for key in keys {
                ui.label(
                    RichText::new(field_label(key))
                        .font(medium(FS_BODY))
                        .color(t.text_dim),
                );
                let value = form.fields.entry(key.to_string()).or_default();
                let edit = if is_multiline_field(key) {
                    egui::TextEdit::multiline(value).desired_rows(if *key == METHOD_NOTES_KEY {
                        3
                    } else {
                        4
                    })
                } else {
                    egui::TextEdit::singleline(value)
                };
                let edit = match *key {
                    COLUMN_VOLUME => edit.hint_text("e.g. 1.0"),
                    "Sample" => edit.hint_text("What is run"),
                    _ => edit,
                };
                let resp = ui.add(
                    edit.id(egui::Id::unique((id, *key)))
                        .desired_width(field_w)
                        .margin(Margin::symmetric(8, 5)),
                );
                if *key == COLUMN_VOLUME && !column_volume_ok(value) {
                    ui.painter().rect_stroke(
                        resp.rect,
                        egui::CornerRadius::same(t.radius_sm as u8),
                        egui::Stroke::new(1.2, t.err),
                        egui::StrokeKind::Inside,
                    );
                }
                ui.end_row();
            }
        });
}

/// The fields: run details left, buffer valve right.
fn fields(ui: &mut Ui, form: &mut InfoForm) {
    let t = palette(ui.ctx());
    let left = [
        "Sample",
        "User",
        "Column",
        COLUMN_VOLUME,
        "Buffer B",
        METHOD_NOTES_KEY,
        "Notes",
    ];
    let right = [
        "Port 1", "Port 2", "Port 3", "Port 4", "Port 5", "Port 6", "Port 7", "Port 8",
    ];
    let gap = 20.0;
    let col_w = (ui.available_width() - gap) / 2.0;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for (title, keys, id) in [
            ("Run", &left[..], "rc-runinfo-left"),
            ("Buffer valve", &right[..], "rc-runinfo-right"),
        ] {
            ui.vertical(|ui| {
                ui.set_width(col_w);
                ui.label(
                    RichText::new(title.to_uppercase())
                        .font(semibold(rc_theme::FS_TITLE))
                        .color(t.text_faint),
                );
                ui.add_space(4.0);
                grid(ui, form, keys, col_w, id);
            });
        }
    });
}

fn buttons(ui: &mut Ui, kind: FormKind) -> FormAction {
    let t = palette(ui.ctx());
    let mut action = FormAction::None;
    ui.horizontal(|ui| {
        ui.spacing_mut().button_padding = egui::vec2(18.0, 6.0);
        if rc_theme::primary_button(ui, "Save").clicked() {
            action = FormAction::Save;
        }
        let close = if kind == FormKind::NewRun {
            "Skip"
        } else {
            "Cancel"
        };
        let tip = if kind == FormKind::NewRun {
            "Record nothing for this run now (the Run info card's Edit asks again)"
        } else {
            "Close without saving"
        };
        let b = egui::Button::new(RichText::new(close).font(medium(FS_BODY)).color(t.text))
            .corner_radius(egui::CornerRadius::same(t.radius_sm as u8));
        if ui.add(b).on_hover_text(tip).clicked() {
            action = FormAction::Close;
        }
    });
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use rc_core::EventKind;
    use rc_core::csvlog::{CsvLog, load_csv_log};

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// Every run information field, empty but for `pairs`.
    fn all_fields(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        let mut m: BTreeMap<String, String> = RUN_INFO_FIELDS
            .iter()
            .map(|k| (k.to_string(), String::new()))
            .collect();
        m.extend(map(pairs));
        m
    }

    #[test]
    fn new_run_opens_and_moves() {
        let mut p = LivePrompt::default();
        let none = BTreeMap::new();
        // no run yet
        assert!(!p.update(0, Path::new("a.csv"), &none, "Alex"));
        assert!(p.form.is_none());
        // the first run (also one that started while the window opened)
        assert!(p.update(3, Path::new("a.csv"), &none, "Alex"));
        p.form = None; // skipped
        assert!(!p.update(3, Path::new("a.csv"), &none, "Alex"));
        assert!(p.form.is_none());
        // a run of a PC method: its run setup is filled in, every field can be changed
        let setup = map(&[("Column", "S200"), ("Column volume (mL)", "24")]);
        assert!(p.update(4, Path::new("b.csv"), &setup, "Alex"));
        let form = p.form.as_mut().unwrap();
        assert_eq!(form.kind, FormKind::NewRun);
        assert_eq!(form.title(), "New run started — b.csv");
        assert_eq!(
            form.fields,
            all_fields(&[
                ("User", "Alex"),
                ("Column", "S200"),
                ("Column volume (mL)", "24")
            ])
        );
        form.fields.insert("Sample".into(), "Lysozyme".into());
        form.fields.insert("Port 3".into(), "Tris".into());
        // the same run again: nothing changes
        assert!(!p.update(4, Path::new("b.csv"), &none, "x"));
        // another run before it was answered: the form moves there, keeping what
        // was typed; its own run setup replaces the column
        let other = map(&[("Column", "HisTrap"), ("Column volume (mL)", "1")]);
        assert!(p.update(5, Path::new("c.csv"), &other, "x"));
        let form = p.form.as_ref().unwrap();
        assert_eq!(form.log, Path::new("c.csv"));
        assert_eq!(form.fields["Sample"], "Lysozyme");
        assert_eq!(form.fields["User"], "Alex");
        assert_eq!(form.fields["Port 3"], "Tris");
        assert_eq!(form.fields["Column"], "HisTrap");
        assert_eq!(form.fields["Column volume (mL)"], "1");
        // answered (closed): the next run opens a fresh one; a method started
        // on the Akta has no run setup, so the column volume is empty to fill in
        p.form = None;
        assert!(p.update(6, Path::new("d.csv"), &none, "Anna"));
        let form = p.form.as_ref().unwrap();
        assert_eq!(form.fields, all_fields(&[("User", "Anna")]));
    }

    #[test]
    fn edit_reopens_the_current_run() {
        let mut p = LivePrompt::default();
        let recorded = map(&[("Sample", "BSA"), ("Column", "S200"), ("Notes", "n")]);
        p.edit(Path::new("a.csv"), &recorded, "Alex");
        let form = p.form.as_ref().unwrap();
        assert_eq!(form.kind, FormKind::LiveEdit);
        assert_eq!(
            form.fields,
            all_fields(&[
                ("Sample", "BSA"),
                ("User", "Alex"),
                ("Column", "S200"),
                ("Notes", "n")
            ])
        );
        // an open form for that run is kept as it is
        p.form
            .as_mut()
            .unwrap()
            .fields
            .insert("Sample".into(), "typed".into());
        p.edit(Path::new("a.csv"), &recorded, "Alex");
        assert_eq!(p.form.as_ref().unwrap().fields["Sample"], "typed");
        // a new run moves it and makes it the new-run form
        assert!(p.update(1, Path::new("b.csv"), &BTreeMap::new(), "Alex"));
        assert_eq!(p.form.as_ref().unwrap().kind, FormKind::NewRun);
    }

    #[test]
    fn live_save_records_changed_fields_in_the_current_log() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = AppState::new();
        app.log_filename = dir.path().join("run.csv");
        app.log = Some(CsvLog::create(&app.log_filename).unwrap());
        app.push_event(EventKind::Info, "User: Alex");
        let app = Mutex::new(app);
        let mut form = InfoForm::new_run(&app.lock().log_filename, &BTreeMap::new(), "Alex");
        form.fields.insert("Sample".into(), " Lysozyme ".into());
        form.fields.insert("Notes".into(), "two\nlines".into());
        assert_eq!(save_live(&app, &form).unwrap(), "Recorded in run.csv.");
        let texts: Vec<String> = app.lock().events.iter().map(|e| e.text.clone()).collect();
        assert_eq!(
            texts,
            ["User: Alex", "Sample: Lysozyme", "Notes: two\nlines"]
        );
        assert_eq!(form.user().as_deref(), Some("Alex"));
        assert_eq!(save_live(&app, &form).unwrap(), "Nothing new to record.");

        // the run's log was closed meanwhile: appended to that file
        {
            let mut a = app.lock();
            a.push_trend(rc_core::TrendPoint::default()); // (a log without data is discarded)
            a.close_log();
        }
        let mut next = AppState::new();
        next.log_filename = dir.path().join("next.csv");
        next.log = Some(CsvLog::create(&next.log_filename).unwrap());
        let next = Mutex::new(next);
        form.fields.insert("Sample".into(), "BSA".into());
        assert_eq!(save_live(&next, &form).unwrap(), "Added to run.csv.");
        assert!(next.lock().events.is_empty());
        let back = load_csv_log(&dir.path().join("run.csv")).unwrap();
        assert_eq!(run_info(&back.events)["Sample"], "BSA");
    }

    /// A run of a method started on the Akta has no run setup: the column and
    /// its volume are given while it runs, and the column volume axis has it.
    #[test]
    fn column_volume_given_during_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = AppState::new();
        app.log_filename = dir.path().join("run.csv");
        app.log = Some(CsvLog::create(&app.log_filename).unwrap());
        app.push_event(EventKind::Info, "Sample: BSA");
        let app = Mutex::new(app);
        let recorded = recorded(&app.lock());
        let mut form = InfoForm::live_edit(&app.lock().log_filename, &recorded, "Alex");
        assert_eq!(form.fields["Sample"], "BSA");
        assert_eq!(form.fields[COLUMN_VOLUME], "");
        form.fields.insert("Column".into(), "Superdex 200".into());
        form.fields.insert(COLUMN_VOLUME.into(), "24,0".into());
        assert_eq!(save_live(&app, &form).unwrap(), "Recorded in run.csv.");
        let info = run_info(&app.lock().events);
        assert_eq!(info["Column"], "Superdex 200");
        assert_eq!(info[COLUMN_VOLUME], "24.0");
        assert_eq!(rc_core::runinfo::column_volume(&info), 24.0);
        // a wrong value is not saved
        form.fields.insert(COLUMN_VOLUME.into(), "big".into());
        assert!(save_live(&app, &form).is_err());
    }

    #[test]
    fn viewer_save_appends_and_refuses_the_recording_log() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.csv");
        {
            let mut app = AppState::new();
            app.log_filename = path.clone();
            app.log = Some(CsvLog::create(&path).unwrap());
            app.push_event(EventKind::Info, "Column: S200");
            app.push_trend(rc_core::TrendPoint {
                cum_time_ms: 5000.0,
                cum_vol_ml: 0.5,
                ..Default::default()
            });
            app.close_log();
        }
        let mut viewer = load_csv_log(&path).unwrap();
        let mut form = InfoForm::viewer(&path, &recorded(&viewer));
        assert_eq!(form.fields["Column"], "S200");
        assert_eq!(form.fields.len(), RUN_INFO_FIELDS.len());
        form.fields
            .insert("Column volume (mL)".into(), "24,0".into());
        form.fields.insert("Sample".into(), "Ferritin".into());

        // the log being recorded is edited on the Live page
        let err = save_viewer(&mut viewer, &form, Some(&path)).unwrap_err();
        assert!(err.contains("being recorded"), "{err}");
        assert_eq!(viewer.events.len(), 1);

        let msg = save_viewer(&mut viewer, &form, Some(&dir.path().join("live.csv"))).unwrap();
        assert_eq!(msg, "Added 2 field(s) to the end of old.csv.");
        let info = recorded(&viewer);
        assert_eq!(info["Column volume (mL)"], "24.0");
        assert_eq!(rc_core::runinfo::column_volume(&info), 24.0);
        let back = load_csv_log(&path).unwrap();
        assert_eq!(run_info(&back.events), info);
        assert_eq!(
            save_viewer(&mut viewer, &form, None).unwrap(),
            "Nothing changed."
        );

        // a bad column volume, and a file that cannot be written
        form.fields
            .insert("Column volume (mL)".into(), "lots".into());
        assert!(
            save_viewer(&mut viewer, &form, None)
                .unwrap_err()
                .contains("not a number")
        );
        form.fields.insert("Column volume (mL)".into(), "1".into());
        viewer.log_filename = dir.path().join("missing").join("x.csv");
        form.log = viewer.log_filename.clone();
        assert!(
            save_viewer(&mut viewer, &form, None)
                .unwrap_err()
                .contains("cannot be written")
        );
    }

    #[test]
    fn viewer_edits_method_notes_apart_from_run_notes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.csv");
        {
            let mut app = AppState::new();
            app.log_filename = path.clone();
            app.log = Some(CsvLog::create(&path).unwrap());
            app.push_event(EventKind::Info, "Method notes: first\nsecond");
            app.push_event(EventKind::Info, "Notes: run");
            app.push_trend(rc_core::TrendPoint {
                cum_time_ms: 5000.0,
                ..Default::default()
            });
            app.close_log();
        }
        let mut viewer = load_csv_log(&path).unwrap();
        let mut form = InfoForm::viewer(&path, &recorded(&viewer));
        // read back from the CSV, the line break is a space
        assert_eq!(form.fields[METHOD_NOTES_KEY], "first second");
        assert_eq!(form.fields["Notes"], "run");
        form.fields
            .insert(METHOD_NOTES_KEY.into(), "changed\nnotes ".into());
        assert_eq!(
            save_viewer(&mut viewer, &form, None).unwrap(),
            "Added 1 field(s) to the end of old.csv."
        );
        let back = run_info(&load_csv_log(&path).unwrap().events);
        assert_eq!(back[METHOD_NOTES_KEY], "changed notes");
        assert_eq!(back["Notes"], "run");
        // the run's own form has every field too
        assert!(
            InfoForm::new_run(&path, &BTreeMap::new(), "")
                .fields
                .contains_key(METHOD_NOTES_KEY)
        );
    }

    #[test]
    fn files_compare_by_path() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.csv");
        std::fs::write(&a, "").unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        assert!(same_file(&a, &sub.join("..").join("a.csv")));
        assert!(!same_file(&a, &dir.path().join("b.csv")));
    }
}
