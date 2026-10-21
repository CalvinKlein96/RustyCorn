//! The editor's model, without any UI: which method is being edited, what changed, and the
//! actions of the Methods tab (open, new, duplicate, save, save as, revert,
//! delete, refresh). Files are read, checked and written with the model in
//! rc-methods, which writes exactly what the instrument writes, so a method
//! saved here loads on the Akta like its own.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use rc_core::runinfo::{METHOD_NOTES_KEY, column_volume_ok, normalize_column_volume};
use rc_methods::{
    BP_COLUMNS, METHOD_NUMBERS, MethodFile, find_method_file, method_name_ok, method_names,
    method_setup, normalized_setup, retire_method_file, save_method_file, set_method_name,
    set_method_setup, stored_methods,
};

/// Run setup key of the column volume (validated as a number).
pub const COLUMN_VOLUME: &str = "Column volume (mL)";

/// Heading of a breakpoint table column (method-file column name → shown name).
pub fn col_label(col: &str) -> &'static str {
    match col {
        "Volume" => "Volume",
        "Conc" => "%B",
        "Flow" => "Flow",
        "Frac" => "Fraction",
        "Buff" => "Buffer",
        "Inj" => "Inj. valve",
        "Peak" => "Peak",
        "Autozero" => "Autozero",
        "Eventmark" => "Event mark",
        _ => "",
    }
}

/// Unit of a breakpoint column, as the method's bases make it ("" for none).
pub fn col_unit(col: &str, m: &MethodFile) -> String {
    match col {
        "Volume" if m.setting("Method base") == "min" => "min".into(),
        "Volume" => "ml".into(),
        "Flow" => "ml/min".into(),
        "Frac" => m.setting("Frac base").to_string(),
        "Peak" => "mAU/min".into(),
        _ => String::new(),
    }
}

/// Column heading with its unit: "Volume (ml)", "Time (min)" …
pub fn col_head(col: &str, m: &MethodFile) -> String {
    if col == "Volume" && m.setting("Method base") == "min" {
        return "Time (min)".into();
    }
    let unit = col_unit(col, m);
    if unit.is_empty() {
        col_label(col).to_string()
    } else {
        format!("{} ({unit})", col_label(col))
    }
}

/// Column width in points, before widening for the heading.
pub fn col_width(col: &str) -> f32 {
    match col {
        "Volume" => 84.0,
        "Conc" | "Buff" => 52.0,
        "Flow" | "Frac" | "Peak" => 64.0,
        _ => 76.0,
    }
}

/// One line about a run setup, for the folded Run setup card.
pub fn setup_summary(setup: &BTreeMap<String, String>) -> String {
    let s = normalized_setup(setup);
    if s.is_empty() {
        return "Not set: runs of this method get no column or buffer names.".into();
    }
    let mut parts: Vec<String> = Vec::new();
    if let Some(c) = s.get("Column") {
        parts.push(c.clone());
    }
    if let Some(cv) = s.get(COLUMN_VOLUME) {
        parts.push(format!("{cv} mL"));
    }
    if let Some(b) = s.get("Buffer B") {
        parts.push(format!("B: {b}"));
    }
    let ports: Vec<String> = (1..=8)
        .filter(|k| s.contains_key(&format!("Port {k}")))
        .map(|k| k.to_string())
        .collect();
    match ports.len() {
        0 => {}
        1 => parts.push(format!("port {} named", ports[0])),
        _ => parts.push(format!("ports {} named", ports.join(", "))),
    }
    if s.contains_key(METHOD_NOTES_KEY) {
        parts.push("notes".into());
    }
    parts.join("  ·  ")
}

/// The first method number not in use; None when all 1–999 are taken.
pub fn first_free(stored: &[u16]) -> Option<u16> {
    let used: BTreeSet<u16> = stored.iter().copied().collect();
    METHOD_NUMBERS.into_iter().find(|n| !used.contains(n))
}

pub fn first_free_in(dir: &Path) -> Option<u16> {
    first_free(&stored_methods(dir))
}

/// Short description of a method for the list, and whether the Akta can load it.
pub fn summary_of(m: &MethodFile) -> (String, bool) {
    let typ = m.setting("Method Type").trim();
    let nbp = if m.rows.len() == 1 {
        "1 breakpoint".to_string()
    } else {
        format!("{} breakpoints", m.rows.len())
    };
    let text = if typ.is_empty() || typ == "-" {
        nbp
    } else {
        typ.to_string()
    };
    (text, m.problems().0.is_empty())
}

/// The description of stored method `n`.
pub fn method_summary(dir: &Path, n: u16) -> (String, bool) {
    match read_stored(dir, n) {
        Ok(m) => summary_of(&m),
        Err(_) => ("not a method file".into(), false),
    }
}

/// At most `n` characters, "…" at the end when cut.
pub fn shorten(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// The folder line above the list ("Folder: …tail" for long paths).
pub fn folder_text(dir: &Path) -> String {
    let d = dir.display().to_string();
    let len = d.chars().count();
    if len > 40 {
        format!("Folder: …{}", d.chars().skip(len - 39).collect::<String>())
    } else {
        format!("Folder: {d}")
    }
}

/// Validation summary under the table: the text and whether all is well.
pub fn check_text(msgs: &[String]) -> (String, bool) {
    if msgs.is_empty() {
        return (
            "No problems found: the Akta can load this method.".into(),
            true,
        );
    }
    let mut s = msgs.iter().take(4).cloned().collect::<Vec<_>>().join("\n");
    if msgs.len() > 4 {
        s.push_str(&format!("\n… and {} more", msgs.len() - 4));
    }
    (s, false)
}

fn read_stored(dir: &Path, n: u16) -> Result<MethodFile, String> {
    let path = find_method_file(dir, n).ok_or_else(|| format!("method {n} not found"))?;
    let bytes = fs::read(&path).map_err(|e| e.to_string())?;
    MethodFile::read(&bytes)
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// One entry of the list of methods.
#[derive(Clone, Debug, PartialEq)]
pub struct ListItem {
    pub number: u16,
    /// name from names.txt, if any
    pub name: Option<String>,
    /// method type or number of breakpoints; "new, not saved" for the unsaved new method
    pub summary: String,
    /// the Akta can load it
    pub ok: bool,
    /// a file exists for it
    pub stored: bool,
}

impl ListItem {
    /// The entry's text as the method list shows it (name or summary, then the notes).
    pub fn text(&self, last_loaded: u16) -> String {
        let mut text = self.name.clone().unwrap_or_else(|| self.summary.clone());
        if self.number == last_loaded {
            text.push_str(" · last loaded");
        }
        if !self.ok {
            text.push_str(" · cannot be loaded");
        }
        text
    }
}

/// What is being edited.
#[derive(Clone, Debug, PartialEq)]
pub struct Editing {
    pub number: u16,
    /// `number` exists as a file with the content `saved`
    pub on_disk: bool,
    /// what is being edited
    pub file: MethodFile,
    /// as last read or written
    pub saved: MethodFile,
    /// name in the editor (kept on the PC only, see method_names)
    pub name: String,
    pub saved_name: String,
    /// run setup in the editor: column, buffers and notes (kept on the PC only, see method_setup)
    pub setup: BTreeMap<String, String>,
    /// as last read or written (empty for a new method)
    pub saved_setup: BTreeMap<String, String>,
    /// an action that waits for a confirming second click
    pub pending: String,
}

impl Editing {
    pub fn new(number: u16, file: MethodFile, on_disk: bool, name: String) -> Editing {
        let saved_name = if on_disk { name.clone() } else { String::new() };
        Editing {
            number,
            on_disk,
            saved: file.clone(),
            file,
            name,
            saved_name,
            setup: BTreeMap::new(),
            saved_setup: BTreeMap::new(),
            pending: String::new(),
        }
    }

    /// The run setup was changed (blank fields and spaces do not count).
    pub fn setup_dirty(&self) -> bool {
        normalized_setup(&self.setup) != normalized_setup(&self.saved_setup)
    }

    /// Unsaved work: edits to a stored method, or a new method that differs from the template.
    pub fn is_dirty(&self) -> bool {
        self.name.trim() != self.saved_name
            || self.setup_dirty()
            || if self.on_disk {
                self.file != self.saved
            } else {
                self.file != MethodFile::new_method()
            }
    }

    /// The value of a run setup field in the editor ("" when not set).
    pub fn setup_value(&self, key: &str) -> &str {
        self.setup.get(key).map_or("", String::as_str)
    }

    /// Problems of the run setup that stop a save.
    pub fn setup_problems(&self) -> Vec<String> {
        let cv = self.setup_value(COLUMN_VOLUME);
        if column_volume_ok(cv) {
            Vec::new()
        } else {
            vec![format!(
                "{COLUMN_VOLUME}: \"{}\" is not a number",
                cv.trim()
            )]
        }
    }

    /// The run setup as it is saved: trimmed, blank fields dropped, a decimal comma
    /// in the column volume written as a point.
    pub fn setup_to_save(&self) -> BTreeMap<String, String> {
        let mut setup = normalized_setup(&self.setup);
        if let Some(cv) = setup.get_mut(COLUMN_VOLUME) {
            *cv = normalize_column_volume(cv);
        }
        setup
    }

    /// "new, not saved yet", "unsaved changes" or "saved".
    pub fn state_text(&self) -> &'static str {
        if !self.on_disk {
            "new, not saved yet"
        } else if self.is_dirty() {
            "unsaved changes"
        } else {
            "saved"
        }
    }

    /// A new breakpoint below row `i` (0-based) with its values but no volume;
    /// below the end it extends the method. Returns the new row's index.
    pub fn add_row(&mut self, i: usize) -> usize {
        let Some(src) = self.file.rows.get(i) else {
            // nothing to copy (an empty table): the start of a new method
            let mut row = MethodFile::new_method()
                .rows
                .into_iter()
                .next()
                .unwrap_or_default();
            row.resize(BP_COLUMNS.len(), String::new());
            self.file.rows.push(row);
            return self.file.rows.len() - 1;
        };
        let mut row = src.clone();
        row.resize(BP_COLUMNS.len(), String::new());
        row[0].clear();
        self.file.rows.insert(i + 1, row);
        i + 1
    }

    /// Delete row `i` (0-based).
    pub fn remove_row(&mut self, i: usize) {
        if i < self.file.rows.len() {
            self.file.rows.remove(i);
        }
    }

    /// A risky action runs on the second click: true when `key` was already pending.
    pub fn confirm(&mut self, key: &str) -> bool {
        if self.pending == key {
            self.pending.clear();
            true
        } else {
            self.pending = key.to_string();
            false
        }
    }
}

/// A message under the toolbar: done, or why not.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Notice {
    pub text: String,
    pub err: bool,
}

/// The Methods tab without its drawing: the methods folder, the method being
/// edited, the list, and the actions on them.
#[derive(Clone, Debug)]
pub struct EditorCore {
    pub dir: PathBuf,
    pub ed: Editing,
    pub notice: Notice,
    pub list: Vec<ListItem>,
}

impl EditorCore {
    /// Start with the first stored method, or a new one.
    pub fn open(dir: PathBuf) -> EditorCore {
        let number = first_free_in(&dir).unwrap_or(0);
        let ed = Editing::new(number, MethodFile::new_method(), false, String::new());
        let mut core = EditorCore {
            dir,
            ed,
            notice: Notice::default(),
            list: Vec::new(),
        };
        if let Some(&first) = stored_methods(&core.dir).first() {
            core.open_stored(first);
        }
        core.rebuild_list();
        core
    }

    fn say(&mut self, msg: impl Into<String>, err: bool) {
        self.notice = Notice {
            text: msg.into(),
            err,
        };
    }

    /// The first click explains a risky action, the second one runs it.
    fn confirm(&mut self, key: &str, msg: &str) -> bool {
        if self.ed.confirm(key) {
            true
        } else {
            self.say(msg, true);
            false
        }
    }

    /// Edit `m` as method `n`; the name and the run setup default
    /// to the stored ones.
    fn edit(
        &mut self,
        n: u16,
        m: MethodFile,
        on_disk: bool,
        name: Option<String>,
        setup: Option<BTreeMap<String, String>>,
    ) {
        let name = name.unwrap_or_else(|| {
            if on_disk {
                method_names(&self.dir).get(&n).cloned().unwrap_or_default()
            } else {
                String::new()
            }
        });
        let saved_setup = if on_disk {
            method_setup(&self.dir, n)
        } else {
            BTreeMap::new()
        };
        self.ed = Editing::new(n, m, on_disk, name);
        self.ed.setup = setup.unwrap_or_else(|| saved_setup.clone());
        self.ed.saved_setup = saved_setup;
        self.rebuild_list();
    }

    /// Edit stored method `n`; if it cannot be read, an empty method under its
    /// number (saving it replaces the file).
    fn open_stored(&mut self, n: u16) {
        match read_stored(&self.dir, n) {
            Ok(m) => self.edit(n, m, true, None, None),
            Err(_) => {
                let name = method_names(&self.dir).get(&n).cloned().unwrap_or_default();
                self.ed = Editing::new(n, MethodFile::new_method(), false, name.clone());
                self.ed.saved_name = name;
                // kept: saving writes the method and this setup again
                self.ed.saved_setup = method_setup(&self.dir, n);
                self.ed.setup = self.ed.saved_setup.clone();
                self.rebuild_list();
            }
        }
    }

    /// The list: the stored methods and the new one being edited.
    pub fn rebuild_list(&mut self) {
        let stored = stored_methods(&self.dir);
        let names = method_names(&self.dir);
        let mut all: BTreeSet<u16> = stored.iter().copied().collect();
        if !self.ed.on_disk && self.ed.number != 0 {
            all.insert(self.ed.number);
        }
        self.list = all
            .into_iter()
            .map(|n| {
                let is_stored = stored.contains(&n);
                let (summary, ok) = if is_stored {
                    method_summary(&self.dir, n)
                } else {
                    ("new, not saved".into(), true)
                };
                ListItem {
                    number: n,
                    name: names.get(&n).cloned(),
                    summary,
                    ok,
                    stored: is_stored,
                }
            })
            .collect();
    }

    /// Click on a method in the list.
    pub fn open_method(&mut self, n: u16) {
        if n == self.ed.number {
            return;
        }
        if self.ed.is_dirty()
            && !self.confirm(
                &format!("open {n}"),
                &format!(
                    "Method {} has unsaved changes. Click again to discard them.",
                    self.ed.number
                ),
            )
        {
            return;
        }
        let Some(path) = find_method_file(&self.dir, n) else {
            return self.rebuild_list();
        };
        let m = fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|b| MethodFile::read(&b));
        match m {
            Ok(m) => {
                self.edit(n, m, true, None, None);
                self.say("", false);
            }
            Err(e) => self.say(format!("Cannot open {}: {e}", file_name(&path)), true),
        }
    }

    fn start_new(&mut self, m: MethodFile, name: String, setup: BTreeMap<String, String>) {
        let Some(n) = first_free_in(&self.dir) else {
            return self.say("All method numbers 1–999 are in use.", true);
        };
        self.edit(n, m, false, Some(name), Some(setup));
        self.say(format!("Method {n} is new: edit it, then Save."), false);
    }

    /// The New button: a new method under the first free number.
    pub fn new_method(&mut self) {
        if !self.ed.is_dirty()
            || self.confirm("new", "Unsaved changes. Click New again to discard them.")
        {
            self.start_new(MethodFile::new_method(), String::new(), BTreeMap::new());
        }
    }

    /// The Duplicate button: the method being edited, with its run setup, as a new one.
    pub fn duplicate(&mut self) {
        let name = self.ed.name.trim();
        let name = if name.is_empty() {
            String::new()
        } else {
            format!("{name} (copy)")
        };
        self.start_new(self.ed.file.clone(), name, self.ed.setup.clone());
    }

    /// The Save button.
    pub fn save(&mut self) {
        self.save_as(self.ed.number);
    }

    /// The Save as button with the number typed next to it.
    pub fn save_as_text(&mut self, s: &str) {
        match parse_method_number(s) {
            Some(n) => self.save_as(n),
            None => self.say("Enter a method number from 1 to 999.", true),
        }
    }

    /// Save as method `n`; refuses while there are problems.
    pub fn save_as(&mut self, n: u16) {
        let (msgs, _) = self.ed.file.problems();
        if let Some(first) = msgs.first() {
            return self.say(format!("Not saved: {first}"), true);
        }
        if !method_name_ok(&self.ed.name) {
            return self.say("Not saved: the name may have at most 60 characters.", true);
        }
        if let Some(first) = self.ed.setup_problems().first() {
            return self.say(format!("Not saved: {first}"), true);
        }
        let name = self.ed.name.trim().to_string();
        let setup = self.ed.setup_to_save();
        let same = self.ed.on_disk && n == self.ed.number;
        if same && self.ed.file == self.ed.saved {
            // only the name and/or the run setup changed: the method file stays as it is
            let name_changed = name != self.ed.saved_name;
            let setup_changed = self.ed.setup_dirty();
            if name_changed && let Err(e) = set_method_name(&self.dir, n, &name) {
                log::error!("Could not save the name of method {n}: {e}");
                return self.say(format!("Name not saved: {e}"), true);
            }
            if setup_changed && let Err(e) = set_method_setup(&self.dir, n, &setup) {
                log::error!("Could not save the run setup of method {n}: {e}");
                return self.say(format!("Run setup not saved: {e}"), true);
            }
            self.edit(n, self.ed.file.clone(), true, None, None);
            let msg = match (name_changed, setup_changed) {
                (true, true) => "Name and run setup saved.",
                (false, true) => "Run setup saved.",
                (_, false) if name.is_empty() => "Name removed.",
                _ => "Name saved.",
            };
            return self.say(msg, false);
        }
        let replacing = !same && find_method_file(&self.dir, n).is_some();
        if replacing
            && !self.confirm(
                &format!("replace {n}"),
                &format!(
                    "Method {n} already exists. Click again to replace it (the old version is kept in methods/old)."
                ),
            )
        {
            return;
        }
        let m = self.ed.file.normalized();
        let path = match save_method_file(&self.dir, n, &m.format_text()) {
            Ok(p) => p,
            Err(e) => {
                log::error!("Could not save method {n}: {e}");
                return self.say(format!("Not saved: {e}"), true);
            }
        };
        if let Err(e) = set_method_name(&self.dir, n, &name) {
            log::error!("Could not save the name of method {n}: {e}");
        }
        // (saving replaced any setup of an earlier method n: it went to old/ with it)
        if let Err(e) = set_method_setup(&self.dir, n, &setup) {
            log::error!("Could not save the run setup of method {n}: {e}");
            self.edit(n, m, true, None, Some(setup));
            return self.say(
                format!(
                    "Saved as {}, but the run setup was not saved: {e}",
                    file_name(&path)
                ),
                true,
            );
        }
        self.edit(n, m, true, None, None);
        self.say(
            format!(
                "Saved as {}. On the Akta: Run Stored Method, PC, number {n}.",
                file_name(&path)
            ),
            false,
        );
    }

    /// The Revert button: back to the file, or to the template for a new method.
    pub fn revert(&mut self) {
        if self.ed.on_disk {
            let n = self.ed.number;
            match read_stored(&self.dir, n) {
                Ok(m) => self.edit(n, m, true, None, None),
                Err(e) => return self.say(format!("Cannot read method {n}: {e}"), true),
            }
        } else {
            self.edit(self.ed.number, MethodFile::new_method(), false, None, None);
        }
        self.say("Changes discarded.", false);
    }

    /// The Delete button: a stored method goes to methods/old (second click);
    /// a new one is just dropped.
    pub fn delete(&mut self) {
        let n = self.ed.number;
        if self.ed.on_disk {
            if !self.confirm(
                &format!("delete {n}"),
                &format!("Click Delete again to delete method {n} (it is kept in methods/old)."),
            ) {
                return;
            }
            if let Some(path) = find_method_file(&self.dir, n)
                && let Err(e) = retire_method_file(&path)
            {
                log::error!("Could not delete method {n}: {e}");
                return self.say(format!("Method {n} not deleted: {e}"), true);
            }
            if let Err(e) = set_method_name(&self.dir, n, "") {
                log::error!("Could not remove the name of method {n}: {e}");
            }
            self.say(format!("Method {n} deleted (moved to methods/old)."), false);
        }
        match stored_methods(&self.dir).first() {
            Some(&first) => self.open_stored(first),
            None => {
                let free = first_free_in(&self.dir).unwrap_or(0);
                self.edit(free, MethodFile::new_method(), false, None, None);
            }
        }
    }

    /// The methods folder moved (Settings): show what is there.
    pub fn set_dir(&mut self, dir: PathBuf) {
        if dir != self.dir {
            self.dir = dir;
            self.refresh();
        }
    }

    /// Files changed outside the editor: by the Akta, by hand, or the folder moved.
    pub fn refresh(&mut self) {
        let n = self.ed.number;
        let disk_name = method_names(&self.dir).get(&n).cloned().unwrap_or_default();
        if self.ed.on_disk && disk_name != self.ed.saved_name {
            // renamed outside the editor
            if self.ed.name.trim() == self.ed.saved_name {
                self.ed.name = disk_name.clone();
            }
            self.ed.saved_name = disk_name;
        }
        if self.ed.on_disk {
            // the run setup changed outside the editor
            let disk_setup = method_setup(&self.dir, n);
            if disk_setup != normalized_setup(&self.ed.saved_setup) {
                if !self.ed.setup_dirty() {
                    self.ed.setup = disk_setup.clone();
                }
                self.ed.saved_setup = disk_setup;
            }
        }
        if self.ed.on_disk {
            match read_stored(&self.dir, n).ok() {
                None => {
                    self.ed.on_disk = false;
                    self.say(
                        format!("Method {n} is no longer in the folder; Save writes it again."),
                        true,
                    );
                }
                Some(m) if m != self.ed.saved => {
                    if self.ed.is_dirty() {
                        self.ed.saved = m;
                        self.say(
                            format!(
                                "Method {n} was replaced while you were editing it. Saving replaces that \
                                 version (it is kept in methods/old)."
                            ),
                            true,
                        );
                    } else {
                        self.edit(n, m, true, None, None);
                        return self.say(
                            format!("Method {n} was replaced on disk; showing the new version."),
                            false,
                        );
                    }
                }
                Some(_) => {}
            }
        }
        self.rebuild_list();
    }

    /// "+" on a row.
    pub fn add_row(&mut self, i: usize) -> usize {
        let at = self.ed.add_row(i);
        self.say(
            format!("Breakpoint {} added: enter its volume.", at + 1),
            false,
        );
        at
    }

    /// "×" on a row.
    pub fn remove_row(&mut self, i: usize) {
        self.ed.remove_row(i);
        self.say("", false);
    }
}

/// A method number typed in the "Save as number" box (1–999).
pub fn parse_method_number(s: &str) -> Option<u16> {
    let s = s.trim();
    if s.is_empty() || s.len() > 3 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<u16>().ok().filter(|n| METHOD_NUMBERS.contains(n))
}
