//! Method editor page: the stored methods on the
//! left, the selected method's settings and breakpoint table on the right;
//! new / duplicate / save / rename, with validation so the Akta can load it.
//!
//! The model and the actions are in [`editing`] (no UI); this file draws them.

pub mod editing;

use std::collections::BTreeSet;
use std::hash::Hash;

use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Id, Key, Layout, Margin, Rect, RichText, Sense,
    Stroke, StrokeKind, UiBuilder, pos2, vec2,
};
use rc_core::SharedApp;
use rc_core::runinfo::{self, METHOD_NOTES_KEY, column_volume_ok};
use rc_methods::{BP_COLUMNS, METHOD_END_VOLUME, MethodFile, method_name_ok, value_choices};
use rc_theme::{
    FS_BODY, FS_SMALL, Palette, medium, mono, palette, primary_button, section_title, semibold,
    soft_chip,
};

pub use editing::{COLUMN_VOLUME, Editing, EditorCore, ListItem, Notice};
use editing::{check_text, col_head, col_label, col_unit, col_width, folder_text};

/// Width of the list of methods.
const LIST_W: f32 = 330.0;
/// Space between cards.
const GAP: f32 = 14.0;
/// Space between the cards and the window's edges (the Live page's margin).
const PAGE_MARGIN_X: f32 = 12.0;
const PAGE_MARGIN_Y: f32 = 10.0;
const CARD_PAD: i8 = 14;
const LIST_ROW_H: f32 = 46.0;
/// Breakpoint table geometry.
const ROW_H: f32 = 34.0;
const FIELD_H: f32 = 26.0;
const NUM_W: f32 = 34.0;
const COL_GAP: f32 = 6.0;
const ACT_W: f32 = 56.0;
/// The settings in the editor: name in the file, label.
const SETTING_FIELDS: [(&str, &str); 4] = [
    ("Method base", "Method base"),
    ("Frac base", "Fraction base"),
    ("Max pressure (kPa)", "Max pressure (kPa)"),
    ("Alarm (min/ml)", "Alarm (min/ml)"),
];

/// What a click asks for; applied after drawing.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Action {
    Open(u16),
    New,
    Duplicate,
    Delete,
    Save,
    SaveAs,
    Revert,
    AddRow(usize),
    RemoveRow(usize),
    /// a value was typed or chosen
    Edited,
}

/// `MethodFile::problems` for the method as it is now (recomputed only when it changes).
struct Checked {
    file: MethodFile,
    msgs: Vec<String>,
    bad: BTreeSet<(usize, String)>,
}

impl Checked {
    fn is_bad(&self, row: usize, col: &str) -> bool {
        self.bad.iter().any(|(r, c)| *r == row && c == col)
    }
}

pub struct MethodEditor {
    /// read for `method_dir`, `methods_changed` and `last_loaded_method`
    pub app: SharedApp,
    /// built when the page is first drawn, not at start-up
    core: Option<EditorCore>,
    seen_changed: u64,
    last_pass: Option<u64>,
    /// the "Save as number" box
    save_as: String,
    checked: Option<Checked>,
    /// breakpoint (row, column) to focus on the next frame
    focus: Option<(usize, usize)>,
    /// the Run setup card unfolded (None: unfolded when the window is tall enough)
    setup_open: Option<bool>,
}

impl MethodEditor {
    pub fn new(app: SharedApp) -> MethodEditor {
        MethodEditor {
            app,
            core: None,
            seen_changed: 0,
            last_pass: None,
            save_as: String::new(),
            checked: None,
            focus: None,
            setup_open: None,
        }
    }

    /// True while there are unsaved edits (the app asks before closing the tab).
    pub fn is_dirty(&self) -> bool {
        self.core.as_ref().is_some_and(|c| c.ed.is_dirty())
    }

    /// The editor's state (None until the page was first drawn).
    pub fn core(&self) -> Option<&EditorCore> {
        self.core.as_ref()
    }

    /// Re-read the methods folder (also done when `methods_changed` changes
    /// and when the page is shown again).
    pub fn refresh(&mut self) {
        if let Some(core) = self.core.as_mut() {
            core.refresh();
        }
    }

    /// Follow the shared state: folder, and files changed by the Akta.
    fn sync(&mut self, ctx: &egui::Context) {
        let (dir, changed) = {
            let a = self.app.lock();
            (a.method_dir.clone(), a.methods_changed)
        };
        let pass = ctx.cumulative_pass_nr();
        let shown_again = self.last_pass.is_some_and(|p| pass > p + 1);
        self.last_pass = Some(pass);
        match self.core.as_mut() {
            None => self.core = Some(EditorCore::open(dir)),
            Some(core) if core.dir != dir => core.set_dir(dir),
            Some(core) if changed != self.seen_changed || shown_again => core.refresh(),
            Some(_) => {}
        }
        self.seen_changed = changed;
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.sync(ui.ctx());
        let last_loaded = self.app.lock().last_loaded_method;
        let Some(core) = self.core.as_mut() else {
            return;
        };
        if self.checked.as_ref().is_none_or(|c| c.file != core.ed.file) {
            let (msgs, bad) = core.ed.file.problems();
            self.checked = Some(Checked {
                file: core.ed.file.clone(),
                msgs,
                bad,
            });
        }
        let Some(checked) = self.checked.as_ref() else {
            return;
        };
        let t = palette(ui.ctx());
        let mut acts = Vec::new();

        // the page's outer margin, as the Live page's
        let area = ui.available_rect_before_wrap();
        let full = area.shrink2(vec2(PAGE_MARGIN_X, PAGE_MARGIN_Y));
        let list_w = LIST_W.min((full.width() * 0.3).max(260.0));
        let left = Rect::from_min_max(full.min, pos2(full.left() + list_w, full.bottom()));
        let right = Rect::from_min_max(pos2(left.right() + GAP, full.top()), full.max);

        // ── Left: the stored methods, how to run one ─────────────────────────
        let help_h = 182.0;
        let list_rect =
            Rect::from_min_max(left.min, pos2(left.right(), left.bottom() - help_h - GAP));
        let help_rect = Rect::from_min_max(pos2(left.left(), list_rect.bottom() + GAP), left.max);
        card_at(ui, list_rect, "list", |ui| {
            draw_list(ui, &t, core, last_loaded, &mut acts)
        });
        card_at(ui, help_rect, "help", |ui| draw_help(ui, &t));

        // ── Right: the method being edited ───────────────────────────────────
        let mut rui = ui.new_child(
            UiBuilder::new()
                .id_salt("rc-methodeditor-right")
                .max_rect(right)
                .layout(Layout::top_down(Align::Min)),
        );
        rui.spacing_mut().item_spacing.y = GAP;
        card(&mut rui, |ui| {
            draw_header(ui, &t, core, &mut self.save_as, &mut acts)
        });
        card(&mut rui, |ui| {
            draw_settings(ui, &t, &mut core.ed, checked, &mut acts)
        });
        // a short window keeps room for the breakpoints: the setup folds to one line
        let open = self.setup_open.unwrap_or(area.height() >= 720.0);
        if card(&mut rui, |ui| {
            draw_setup(ui, &t, &mut core.ed, open, &mut acts)
        }) {
            self.setup_open = Some(!open);
        }
        let rest = rui.available_rect_before_wrap();
        if rest.height() > 120.0 {
            card_at(&mut rui, rest, "table", |ui| {
                draw_table(ui, &t, &mut core.ed, checked, &mut self.focus, &mut acts)
            });
        }
        ui.advance_cursor_after_rect(area);

        for a in acts {
            match a {
                Action::Open(n) => core.open_method(n),
                Action::New => core.new_method(),
                Action::Duplicate => core.duplicate(),
                Action::Delete => core.delete(),
                Action::Save => core.save(),
                Action::SaveAs => core.save_as_text(&self.save_as),
                Action::Revert => core.revert(),
                Action::AddRow(i) => {
                    let at = core.add_row(i);
                    self.focus = Some((at, 0));
                }
                Action::RemoveRow(i) => core.remove_row(i),
                // a warning about the method as it was no longer applies
                Action::Edited => core.ed.pending.clear(),
            }
        }
    }
}

// ── Building blocks ──────────────────────────────────────────────────────────

fn card_frame(t: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(t.card)
        .stroke(Stroke::new(1.0, t.card_border))
        .corner_radius(CornerRadius::same(t.radius as u8))
        .inner_margin(Margin::same(CARD_PAD))
}

/// A card as wide as the page, as tall as its contents.
fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let t = palette(ui.ctx());
    card_frame(&t)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            add(ui)
        })
        .inner
}

/// A card filling `rect`.
fn card_at<R>(
    ui: &mut egui::Ui,
    rect: Rect,
    salt: &str,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let t = palette(ui.ctx());
    let mut child = ui.new_child(
        UiBuilder::new()
            .id_salt(("rc-methodeditor-card", salt))
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min)),
    );
    child.shrink_clip_rect(rect);
    card_frame(&t)
        .show(&mut child, |ui| {
            ui.set_min_size(ui.available_size());
            ui.spacing_mut().item_spacing.y = 6.0;
            add(ui)
        })
        .inner
}

/// A child ui exactly over `rect`, contents left-aligned and vertically centred.
fn cell_ui(ui: &mut egui::Ui, rect: Rect, salt: impl Hash + std::fmt::Debug) -> egui::Ui {
    ui.new_child(
        UiBuilder::new()
            .id_salt(salt)
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center)),
    )
}

/// Single-line text cut with "…" to `width`.
fn elided(
    ui: &egui::Ui,
    text: &str,
    font: FontId,
    color: Color32,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = TextWrapping::truncate_at_width(width.max(10.0));
    ui.painter().layout_job(job)
}

fn text_width(ui: &egui::Ui, text: &str, font: FontId) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_owned(), font, Color32::WHITE)
        .size()
        .x
}

/// Outline a bad value: its box border in the error colour.
fn mark_bad(ui: &egui::Ui, t: &Palette, rect: Rect) {
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(t.radius_sm as u8),
        Stroke::new(1.2, t.err),
        StrokeKind::Inside,
    );
}

/// A text field for a value; numbers in mono.
#[allow(clippy::too_many_arguments)]
fn value_field(
    ui: &mut egui::Ui,
    t: &Palette,
    text: &mut String,
    id: Id,
    width: f32,
    bad: bool,
    hint: &str,
    font: FontId,
) -> egui::Response {
    let fill = if bad { t.tint(t.err, 0.10) } else { t.field };
    let edit = egui::TextEdit::singleline(text)
        .id(id)
        .font(font)
        .margin(Margin::symmetric(7, 4))
        .background_color(fill)
        .hint_text(RichText::new(hint).color(t.text_faint));
    let resp = ui.add_sized(vec2(width, FIELD_H), edit);
    if bad {
        mark_bad(ui, t, resp.rect);
    }
    resp
}

/// A dropdown for a value with fixed choices; true when it changed.
fn choice_field(
    ui: &mut egui::Ui,
    t: &Palette,
    value: &mut String,
    choices: &[&str],
    salt: impl Hash + std::fmt::Debug,
    width: f32,
    bad: bool,
) -> bool {
    let before = value.clone();
    let resp = egui::ComboBox::from_id_salt(salt)
        .width(width)
        .height(240.0)
        .selected_text(RichText::new(value.as_str()).font(FontId::proportional(FS_BODY)))
        .show_ui(ui, |ui| {
            for &opt in choices {
                ui.selectable_value(value, opt.to_string(), opt);
            }
        });
    if bad {
        mark_bad(ui, t, resp.response.rect);
    }
    *value != before
}

/// A small frameless icon button ("+", "×").
fn icon_button(
    ui: &mut egui::Ui,
    t: &Palette,
    icon: &str,
    strong: bool,
    tip: &str,
) -> egui::Response {
    let color = if strong { t.icon } else { t.text_faint };
    ui.add(
        egui::Button::new(RichText::new(icon).font(medium(15.0)).color(color))
            .frame_when_inactive(false)
            .min_size(vec2(24.0, 24.0))
            .corner_radius(CornerRadius::same(t.radius_sm as u8)),
    )
    .on_hover_text(tip)
}

/// A secondary button; outlined in the error colour while it waits for its confirming click.
fn button(ui: &mut egui::Ui, t: &Palette, text: &str, armed: bool) -> egui::Response {
    let mut b = egui::Button::new(RichText::new(text).font(medium(FS_BODY)).color(if armed {
        t.err
    } else {
        t.text
    }));
    if armed {
        b = b.stroke(Stroke::new(1.0, t.err)).fill(t.tint(t.err, 0.12));
    }
    ui.add(
        b.corner_radius(CornerRadius::same(t.radius_sm as u8))
            .min_size(vec2(0.0, 28.0)),
    )
}

// ── Left column ──────────────────────────────────────────────────────────────

fn draw_list(
    ui: &mut egui::Ui,
    t: &Palette,
    core: &EditorCore,
    last_loaded: u16,
    acts: &mut Vec<Action>,
) {
    ui.horizontal(|ui| {
        section_title(ui, "Methods on the PC");
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let n = core.list.iter().filter(|i| i.stored).count();
            ui.label(
                RichText::new(format!("{n} stored"))
                    .font(FontId::proportional(FS_SMALL))
                    .color(t.text_faint),
            );
        });
    });
    let folder = folder_text(&core.dir);
    ui.add(
        egui::Label::new(
            RichText::new(folder)
                .font(FontId::proportional(FS_SMALL))
                .color(t.text_dim),
        )
        .truncate(),
    )
    .on_hover_text(core.dir.display().to_string());
    ui.add_space(4.0);

    let buttons_h = 40.0;
    let list_h = (ui.available_height() - buttons_h).max(60.0);
    egui::ScrollArea::vertical()
        .id_salt("rc-methodeditor-list")
        .max_height(list_h)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            if core.list.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new("No methods yet.").color(t.text_dim));
            }
            for item in &core.list {
                let active = item.number == core.ed.number;
                let armed = core.ed.pending == format!("open {}", item.number);
                if list_row(ui, t, item, active, armed, last_loaded).clicked() {
                    acts.push(Action::Open(item.number));
                }
            }
        });

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if button(ui, t, "New", core.ed.pending == "new")
            .on_hover_text("A new method under the first free number")
            .clicked()
        {
            acts.push(Action::New);
        }
        if button(ui, t, "Duplicate", false)
            .on_hover_text("This method as a new one")
            .clicked()
        {
            acts.push(Action::Duplicate);
        }
        let armed = core.ed.pending == format!("delete {}", core.ed.number);
        if button(ui, t, "Delete", armed)
            .on_hover_text("Moves the file to methods/old")
            .clicked()
        {
            acts.push(Action::Delete);
        }
    });
}

/// One method in the list: number, name (or summary), and what else to know.
fn list_row(
    ui: &mut egui::Ui,
    t: &Palette,
    item: &ListItem,
    active: bool,
    armed: bool,
    last_loaded: u16,
) -> egui::Response {
    let (rect, resp) =
        ui.allocate_exact_size(vec2(ui.available_width(), LIST_ROW_H), Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let p = ui.painter();
    let r = CornerRadius::same(t.radius_sm as u8);
    if active {
        p.rect_filled(rect, r, t.accent_soft);
    } else if resp.hovered() {
        p.rect_filled(rect, r, t.hover);
    }
    if armed {
        p.rect_stroke(rect, r, Stroke::new(1.0, t.err), StrokeKind::Inside);
    }
    let num_color = if !item.ok {
        t.err
    } else if active {
        t.accent_text
    } else {
        t.text_dim
    };
    p.text(
        pos2(rect.left() + 12.0, rect.center().y),
        Align2::LEFT_CENTER,
        format!("{:03}", item.number),
        mono(13.0),
        num_color,
    );

    let x = rect.left() + 56.0;
    let w = rect.right() - 10.0 - x;
    let (title, title_color) = match &item.name {
        Some(name) => (name.as_str(), t.text),
        None if !item.stored => (item.summary.as_str(), t.warn),
        None => (item.summary.as_str(), t.text),
    };
    let title_font = if active {
        semibold(FS_BODY)
    } else {
        medium(FS_BODY)
    };
    let title_g = elided(ui, title, title_font, title_color, w);

    // second line: the summary when the name took its place, then the notes
    let mut notes: Vec<(String, Color32)> = Vec::new();
    if item.name.is_some() {
        notes.push((item.summary.clone(), t.text_faint));
    }
    if item.number == last_loaded {
        notes.push(("last loaded".into(), t.accent_text));
    }
    if !item.ok {
        notes.push(("cannot be loaded".into(), t.err));
    }
    let mut job = LayoutJob::default();
    for (k, (s, c)) in notes.iter().enumerate() {
        if k > 0 {
            job.append(
                " · ",
                0.0,
                egui::TextFormat::simple(FontId::proportional(FS_SMALL), t.text_faint),
            );
        }
        job.append(
            s,
            0.0,
            egui::TextFormat::simple(FontId::proportional(FS_SMALL), *c),
        );
    }
    job.wrap = TextWrapping::truncate_at_width(w.max(10.0));
    let p = ui.painter();
    if notes.is_empty() {
        p.galley(
            pos2(x, rect.center().y - title_g.size().y / 2.0),
            title_g,
            t.text,
        );
    } else {
        let notes_g = p.layout_job(job);
        let h = title_g.size().y + 2.0 + notes_g.size().y;
        let y = rect.center().y - h / 2.0;
        let th = title_g.size().y;
        p.galley(pos2(x, y), title_g, t.text);
        p.galley(pos2(x, y + th + 2.0), notes_g, t.text_faint);
    }
    resp.on_hover_text(item.text(last_loaded))
}

/// How a method stored here runs on the instrument.
fn draw_help(ui: &mut egui::Ui, t: &Palette) {
    section_title(ui, "Run on the Akta");
    ui.add_space(2.0);
    let steps = [
        "Save the method here; it is stored under its number.",
        "On the Akta choose Run Stored Method, then PC.",
        "Choose the method's number: the Akta loads it from this PC.",
    ];
    for (k, s) in steps.iter().enumerate() {
        ui.horizontal_top(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 9.0, t.accent_soft);
            ui.painter().text(
                r.center(),
                Align2::CENTER_CENTER,
                (k + 1).to_string(),
                semibold(FS_SMALL),
                t.accent_text,
            );
            ui.add(egui::Label::new(RichText::new(*s).color(t.text_dim)).wrap());
        });
    }
    ui.add(
        egui::Label::new(
            RichText::new("Methods the Akta saves to the PC show up in the list.")
                .font(FontId::proportional(FS_SMALL))
                .color(t.text_faint),
        )
        .wrap(),
    );
}

// ── Right column ─────────────────────────────────────────────────────────────

fn draw_header(
    ui: &mut egui::Ui,
    t: &Palette,
    core: &EditorCore,
    save_as: &mut String,
    acts: &mut Vec<Action>,
) {
    let ed = &core.ed;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("Method {:03}", ed.number))
                .font(semibold(20.0))
                .color(t.text),
        );
        let state = ed.state_text();
        let color = if ed.on_disk && !ed.is_dirty() {
            t.text_dim
        } else {
            t.warn
        };
        soft_chip(ui, state, color);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if primary_button(ui, "  Save  ")
                .on_hover_text("Save under this number")
                .clicked()
            {
                acts.push(Action::Save);
            }
            ui.add_space(10.0);
            let armed = ed.pending.starts_with("replace ");
            if button(ui, t, "Save as", armed).clicked() {
                acts.push(Action::SaveAs);
            }
            let ok = save_as.trim().is_empty() || editing::parse_method_number(save_as).is_some();
            let resp = value_field(
                ui,
                t,
                save_as,
                Id::unique("rc-methodeditor-save-as"),
                58.0,
                !ok,
                "1–999",
                mono(FS_BODY),
            );
            if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                acts.push(Action::SaveAs);
            }
            ui.label(RichText::new("Save as number").color(t.text_dim));
            ui.add_space(10.0);
            if button(ui, t, "Revert", false)
                .on_hover_text("Discard the changes")
                .clicked()
            {
                acts.push(Action::Revert);
            }
        });
    });
    if !core.notice.text.is_empty() {
        let color = if core.notice.err { t.err } else { t.ok };
        ui.add(egui::Label::new(RichText::new(&core.notice.text).color(color)).wrap());
    }
}

fn field_label(ui: &mut egui::Ui, t: &Palette, text: &str) {
    ui.label(RichText::new(text).font(medium(FS_SMALL)).color(t.text_dim));
}

fn draw_settings(
    ui: &mut egui::Ui,
    t: &Palette,
    ed: &mut Editing,
    checked: &Checked,
    acts: &mut Vec<Action>,
) {
    section_title(ui, "Method settings");
    ui.add_space(2.0);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            field_label(ui, t, "Name");
            let w = (ui.available_width() - 260.0).clamp(220.0, 460.0);
            let bad = !method_name_ok(&ed.name);
            let resp = value_field(
                ui,
                t,
                &mut ed.name,
                Id::unique("rc-methodeditor-name"),
                w,
                bad,
                "Name of the method, kept on this PC only",
                FontId::proportional(FS_BODY),
            );
            if resp.changed() {
                acts.push(Action::Edited);
            }
        });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            field_label(ui, t, "As set on the Akta");
            ui.add_space(4.0);
            let info = format!(
                "Method type: {}   ·   Method: {}",
                ed.file.setting("Method Type").trim(),
                ed.file.setting("Method").trim()
            );
            ui.label(RichText::new(info).color(t.text_dim));
        });
    });
    ui.add_space(6.0);
    ui.horizontal_top(|ui| {
        for (name, label) in SETTING_FIELDS {
            ui.vertical(|ui| {
                field_label(ui, t, label);
                let bad = checked.is_bad(0, name);
                let mut value = ed.file.setting(name).to_string();
                let changed = if let Some(choices) = value_choices(name) {
                    choice_field(
                        ui,
                        t,
                        &mut value,
                        choices,
                        ("rc-methodeditor-setting", name),
                        92.0,
                        bad,
                    )
                } else {
                    value_field(
                        ui,
                        t,
                        &mut value,
                        Id::unique(("rc-methodeditor-setting", name)),
                        92.0,
                        bad,
                        "",
                        mono(FS_BODY),
                    )
                    .changed()
                };
                if changed {
                    ed.file.set_setting(name, &value);
                    acts.push(Action::Edited);
                }
            });
            ui.add_space(18.0);
        }
    });
}

/// The run setup: column, the buffer on each buffer valve port and the
/// method's notes. Kept on the PC and recorded in the log of every run of this method. Folded (`open`
/// false) it is one line. Returns true when Show/Hide was clicked.
fn draw_setup(
    ui: &mut egui::Ui,
    t: &Palette,
    ed: &mut Editing,
    open: bool,
    acts: &mut Vec<Action>,
) -> bool {
    let bad_cv = !column_volume_ok(ed.setup_value(COLUMN_VOLUME));
    let mut toggle = false;
    ui.horizontal(|ui| {
        section_title(ui, "Run setup");
        let note = if open {
            "Kept on this PC; recorded in the log of every run of this method".to_string()
        } else {
            editing::setup_summary(&ed.setup)
        };
        let color = if !open && bad_cv {
            t.err
        } else if open {
            t.text_faint
        } else {
            t.text_dim
        };
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let label = if open { "Hide" } else { "Show" };
            let tip = if open {
                "Fold the run setup to one line"
            } else {
                "Column, buffers and notes of this method"
            };
            toggle = ui
                .add(
                    egui::Button::new(
                        RichText::new(label)
                            .font(medium(FS_SMALL))
                            .color(t.accent_text),
                    )
                    .frame_when_inactive(false)
                    .corner_radius(CornerRadius::same(t.radius_sm as u8))
                    .min_size(vec2(0.0, 20.0)),
                )
                .on_hover_text(tip)
                .clicked();
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(note)
                            .font(FontId::proportional(FS_SMALL))
                            .color(color),
                    )
                    .truncate(),
                );
            });
        });
    });
    if !open {
        return toggle;
    }
    ui.add_space(2.0);
    let gap = 10.0;
    let full = ui.available_width();
    let mut field =
        |ui: &mut egui::Ui, ed: &mut Editing, key: &str, width: f32, hint: &str, font: FontId| {
            ui.vertical(|ui| {
                ui.set_width(width);
                ui.add(
                    egui::Label::new(
                        RichText::new(runinfo::field_label(key))
                            .font(medium(FS_SMALL))
                            .color(t.text_dim),
                    )
                    .truncate(),
                );
                let mut value = ed.setup_value(key).to_string();
                let bad = key == COLUMN_VOLUME && !column_volume_ok(&value);
                let resp = value_field(
                    ui,
                    t,
                    &mut value,
                    Id::unique(("rc-methodeditor-setup", key)),
                    width,
                    bad,
                    hint,
                    font,
                );
                if resp.changed() {
                    ed.setup.insert(key.to_string(), value);
                    acts.push(Action::Edited);
                }
                if bad {
                    resp.on_hover_text("A number in mL, e.g. 5 or 24.0 (a decimal comma is fine)");
                }
            });
        };
    // column, its volume and buffer B; then the eight buffer valve ports
    let cv_w = 120.0;
    let wide = ((full - cv_w - 2.0 * gap) / 2.0).max(120.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        field(
            ui,
            ed,
            "Column",
            wide,
            "e.g. HiTrap Q 1 ml",
            FontId::proportional(FS_BODY),
        );
        field(ui, ed, COLUMN_VOLUME, cv_w, "e.g. 1.0", mono(FS_BODY));
        field(ui, ed, "Buffer B", wide, "", FontId::proportional(FS_BODY));
    });
    ui.add_space(2.0);
    let port_w = ((full - 7.0 * gap) / 8.0).max(60.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for k in 1..=8 {
            field(
                ui,
                ed,
                &format!("Port {k}"),
                port_w,
                "",
                FontId::proportional(FS_BODY),
            );
        }
    });
    ui.add_space(2.0);
    if notes_field(ui, t, ed, full) {
        acts.push(Action::Edited);
    }
    toggle
}

/// The method's notes, several lines (kept with the run setup, recorded in
/// the log of every run). True when they were edited.
fn notes_field(ui: &mut egui::Ui, t: &Palette, ed: &mut Editing, width: f32) -> bool {
    ui.label(
        RichText::new("Notes")
            .font(medium(FS_SMALL))
            .color(t.text_dim),
    );
    let mut value = ed.setup_value(METHOD_NOTES_KEY).to_string();
    let edit = egui::TextEdit::multiline(&mut value)
        .id(Id::unique("rc-methodeditor-method-notes"))
        .font(FontId::proportional(FS_BODY))
        .margin(Margin::symmetric(7, 5))
        .background_color(t.field)
        .desired_rows(2)
        .desired_width(width)
        .hint_text(
            RichText::new("Notes on this method, e.g. how to prepare the column; recorded in the log of every run")
                .color(t.text_faint),
        );
    let changed = ui.add(edit).changed();
    if changed {
        ed.setup.insert(METHOD_NOTES_KEY.to_string(), value);
    }
    changed
}

fn draw_table(
    ui: &mut egui::Ui,
    t: &Palette,
    ed: &mut Editing,
    checked: &Checked,
    focus: &mut Option<(usize, usize)>,
    acts: &mut Vec<Action>,
) {
    let n = ed.file.rows.len();
    ui.horizontal(|ui| {
        section_title(ui, "Breakpoints");
        let count = if n == 1 {
            "1 breakpoint".to_string()
        } else {
            format!("{n} breakpoints")
        };
        ui.label(
            RichText::new(count)
                .font(FontId::proportional(FS_SMALL))
                .color(t.text_faint),
        );
    });
    ui.add_space(2.0);

    // Column widths: the fixed ones (`col_width`), widened for the heading.
    let head_font = medium(FS_SMALL);
    let widths: Vec<f32> = BP_COLUMNS
        .iter()
        .map(|c| {
            let label = if *c == "Volume" && ed.file.setting("Method base") == "min" {
                "Time"
            } else {
                col_label(c)
            };
            let w = text_width(ui, label, head_font.clone()).max(text_width(
                ui,
                &col_unit(c, &ed.file),
                head_font.clone(),
            ));
            col_width(c).max(w + 16.0)
        })
        .collect();
    let total_w = NUM_W + widths.iter().map(|w| w + COL_GAP).sum::<f32>() + COL_GAP + ACT_W;

    // The validation and the end-of-method hint below the table.
    let (check, ok) = check_text(&checked.msgs);
    let check_lines = check.lines().count().max(1) as f32;
    let bottom_h = check_lines * 19.0 + 40.0;
    let table_h = (ui.available_height() - bottom_h).max(80.0);

    ui.allocate_ui(vec2(ui.available_width(), table_h), |ui| {
        egui::ScrollArea::horizontal()
            .id_salt("rc-methodeditor-table-h")
            .content_margin(Margin::ZERO)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                // heading
                let (head, _) = ui.allocate_exact_size(vec2(total_w, 34.0), Sense::hover());
                let p = ui.painter();
                p.text(
                    pos2(head.left() + NUM_W - 6.0, head.top() + 10.0),
                    Align2::RIGHT_CENTER,
                    "#",
                    head_font.clone(),
                    t.text_faint,
                );
                let mut x = head.left() + NUM_W + COL_GAP;
                for (c, col) in BP_COLUMNS.iter().enumerate() {
                    let (label, unit) =
                        if *col == "Volume" && ed.file.setting("Method base") == "min" {
                            ("Time", "min".to_string())
                        } else {
                            (col_label(col), col_unit(col, &ed.file))
                        };
                    let resp = ui.interact(
                        Rect::from_min_size(pos2(x, head.top()), vec2(widths[c], head.height())),
                        Id::unique(("rc-methodeditor-head", c)),
                        Sense::hover(),
                    );
                    let p = ui.painter();
                    p.text(
                        pos2(x + 7.0, head.top() + 10.0),
                        Align2::LEFT_CENTER,
                        label,
                        head_font.clone(),
                        t.text_dim,
                    );
                    if !unit.is_empty() {
                        p.text(
                            pos2(x + 7.0, head.top() + 24.0),
                            Align2::LEFT_CENTER,
                            unit,
                            FontId::proportional(FS_SMALL - 0.5),
                            t.text_faint,
                        );
                    }
                    resp.on_hover_text(col_head(col, &ed.file));
                    x += widths[c] + COL_GAP;
                }
                ui.painter().hline(
                    head.x_range(),
                    head.bottom() - 0.5,
                    Stroke::new(1.0, t.separator),
                );

                let body_h = (ui.available_height()).max(40.0);
                egui::ScrollArea::vertical()
                    .id_salt("rc-methodeditor-table-v")
                    .content_margin(Margin::ZERO)
                    .max_height(body_h)
                    .auto_shrink([true, true])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        if n == 0 {
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("No breakpoints.").color(t.text_dim));
                                if button(ui, t, "Add breakpoint", false).clicked() {
                                    acts.push(Action::AddRow(0));
                                }
                            });
                        }
                        for i in 0..n {
                            draw_row(ui, t, ed, checked, i, &widths, total_w, focus, acts);
                        }
                    });
            });
    });

    ui.add_space(4.0);
    ui.horizontal_top(|ui| {
        let color = if ok { t.ok } else { t.err };
        let (r, _) = ui.allocate_exact_size(vec2(10.0, 17.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 4.0, color);
        ui.add(egui::Label::new(RichText::new(check).color(color)).wrap());
    });
    ui.label(
        RichText::new(format!(
            "The last breakpoint is the end of the method ({METHOD_END_VOLUME} = run until stopped)."
        ))
        .font(FontId::proportional(FS_SMALL))
        .color(t.text_faint),
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_row(
    ui: &mut egui::Ui,
    t: &Palette,
    ed: &mut Editing,
    checked: &Checked,
    i: usize,
    widths: &[f32],
    total_w: f32,
    focus: &mut Option<(usize, usize)>,
    acts: &mut Vec<Action>,
) {
    let n = ed.file.rows.len();
    let (rect, _) = ui.allocate_exact_size(vec2(total_w, ROW_H), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let hovered = ui.rect_contains_pointer(rect);
    if hovered {
        ui.painter().rect_filled(
            rect.shrink2(vec2(0.0, 1.0)),
            CornerRadius::same(t.radius_sm as u8),
            t.tint(t.hover, 0.6),
        );
    }
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, t.separator),
    );

    let row_bad = checked.bad.iter().any(|(r, _)| *r == i + 1);
    let end = i + 1 == n;
    let label = if end {
        "end".to_string()
    } else {
        (i + 1).to_string()
    };
    let label_color = if row_bad { t.err } else { t.text_faint };
    let font = if end {
        medium(FS_SMALL)
    } else {
        mono(FS_SMALL + 0.5)
    };
    ui.painter().text(
        pos2(rect.left() + NUM_W - 6.0, rect.center().y),
        Align2::RIGHT_CENTER,
        label,
        font,
        label_color,
    );

    let row = &mut ed.file.rows[i];
    if row.len() < BP_COLUMNS.len() {
        row.resize(BP_COLUMNS.len(), String::new());
    }
    let mut x = rect.left() + NUM_W + COL_GAP;
    for (c, col) in BP_COLUMNS.iter().enumerate() {
        let cell = Rect::from_min_size(pos2(x, rect.top()), vec2(widths[c], ROW_H));
        let mut cui = cell_ui(ui, cell, ("rc-methodeditor-cell", i, c));
        let bad = checked.is_bad(i + 1, col);
        let value = &mut row[c];
        if let Some(choices) = value_choices(col) {
            if choice_field(
                &mut cui,
                t,
                value,
                choices,
                ("rc-methodeditor-choice", i, c),
                widths[c],
                bad,
            ) {
                acts.push(Action::Edited);
            }
        } else {
            let id = Id::unique(("rc-methodeditor-bp", i, c));
            let hint = if c == 0 {
                if end { METHOD_END_VOLUME } else { "e.g. 10.0" }
            } else {
                ""
            };
            let resp = value_field(&mut cui, t, value, id, widths[c], bad, hint, mono(FS_BODY));
            if resp.changed() {
                acts.push(Action::Edited);
            }
            if *focus == Some((i, c)) {
                resp.request_focus();
                resp.scroll_to_me(Some(Align::Center));
                *focus = None;
            }
        }
        x += widths[c] + COL_GAP;
    }
    let act = Rect::from_min_size(pos2(x, rect.top()), vec2(ACT_W, ROW_H));
    let mut aui = cell_ui(ui, act, ("rc-methodeditor-act", i));
    aui.spacing_mut().item_spacing.x = 2.0;
    if icon_button(&mut aui, t, "+", hovered, "Add a breakpoint below this one").clicked() {
        acts.push(Action::AddRow(i));
    }
    if icon_button(&mut aui, t, "×", hovered, "Delete this breakpoint").clicked() {
        acts.push(Action::RemoveRow(i));
    }
}

#[cfg(test)]
mod tests;
