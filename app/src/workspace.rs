//! The application window: a top bar with tabs (live
//! data, opened runs, settings, methods) and menu buttons, above the page of
//! the active tab. Only the visible page is drawn. Run information is asked for
//! in windows over the pages (runinfo_forms), not on a page of its own.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use egui::{Align, Align2, CornerRadius, Layout, Margin, RichText, Sense, Stroke, Ui, Vec2};
use rc_core::AppState;
use rc_methodeditor::MethodEditor;
use rc_protocol::Acquisition;
use rc_runview::{RunView, RunViewOptions};
use rc_theme::{FS_BODY, medium, palette, semibold};

use crate::console::display_name;
use crate::runinfo_forms::{self, FormAction, InfoForm, LivePrompt};
use crate::settings_page::SettingsPage;

/// Which window this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// live data (demo or instrument) with all pages
    Live,
    /// `--methods`: only the method editor, no menu buttons
    MethodsOnly,
}

pub const LIVE: &str = "Live";
pub const SETTINGS: &str = "Settings";
pub const METHODS: &str = "Methods";
/// RUSTYCORN_SHOW_PAGES entry that opens the run info form of the shown page
pub const EDIT_RUN_INFO: &str = "Edit run info";
/// RUSTYCORN_SHOW_PAGES entries that switch the shown run page to the overlay
/// plot and open its run log
pub const SHOW_OVERLAY: &str = "Overlay";
pub const SHOW_RUN_LOG: &str = "Run log";
/// RUSTYCORN_SHOW_PAGES entry that never asks for the run info of new runs
pub const NO_RUN_PROMPT: &str = "No run prompt";

/// Longest notice shown in the top bar, in characters.
const NOTICE_MAX_CHARS: usize = 120;

enum Content {
    Live(Box<RunView>),
    /// an opened log, read-only
    Run(Box<RunView>),
    /// the Settings page; its state lives in `Workspace::settings_page` (kept for reuse)
    Settings,
    Methods(Box<MethodEditor>),
}

struct Page {
    id: u64,
    title: String,
    closable: bool,
    content: Content,
}

/// Messages of the "Analyze Run…" background work: the file dialog, then the parsing.
enum OpenMsg {
    Picked(Option<PathBuf>),
    Loaded(PathBuf, Box<std::io::Result<AppState>>),
}

pub struct Workspace<'a> {
    acq: &'a mut Acquisition,
    mode: Mode,
    pages: Vec<Page>,
    active: u64,
    next_id: u64,
    /// last error of a menu action, shown in the top bar
    notice: String,
    settings_page: Option<SettingsPage>,
    open_tx: Sender<OpenMsg>,
    open_rx: Receiver<OpenMsg>,
    /// the file dialog is open
    picking: bool,
    /// files being parsed
    loading: Vec<PathBuf>,
    /// a tab with unsaved method edits whose × was clicked once
    confirm_close: Option<u64>,
    /// the window was asked to close once with unsaved method edits
    confirm_quit: bool,
    /// the "no log file" notice is shown
    no_log_notice: bool,
    /// sample, user and notes of a new live run
    live_prompt: LivePrompt,
    /// "Edit run info" of opened runs, by page id
    run_forms: HashMap<u64, InfoForm>,
    /// new runs open the run info form (off only for screenshots)
    run_prompt: bool,
}

/// The top-bar notice for an error: "Error: " and the first
/// [`NOTICE_MAX_CHARS`] characters of the message.
pub fn error_notice(msg: &str) -> String {
    let msg: String = msg.chars().take(NOTICE_MAX_CHARS).collect();
    format!("Error: {msg}")
}

/// Tab title of an opened log, and the prefix its exported plots get (named after the run).
pub fn run_title_and_prefix(path: &Path) -> (String, String) {
    let title = display_name(path);
    let prefix = if title.to_lowercase().ends_with(".csv") {
        title[..title.len() - 4].to_string()
    } else {
        title.clone()
    };
    (title, prefix)
}

impl<'a> Workspace<'a> {
    pub fn new(acq: &'a mut Acquisition, mode: Mode) -> Workspace<'a> {
        let (open_tx, open_rx) = mpsc::channel();
        let mut ws = Workspace {
            acq,
            mode,
            pages: Vec::new(),
            active: 0,
            next_id: 1,
            // a settings file that could not be read cleanly; in full, as it names the
            // backup copy (the top bar shortens it and shows all of it on hover)
            notice: rc_core::settings::take_load_problem()
                .map(|m| format!("Error: {m}"))
                .unwrap_or_default(),
            settings_page: None,
            open_tx,
            open_rx,
            picking: false,
            loading: Vec::new(),
            confirm_close: None,
            confirm_quit: false,
            no_log_notice: false,
            // a run that started while the window was opening is asked about too
            live_prompt: LivePrompt::default(),
            run_forms: HashMap::new(),
            run_prompt: true,
        };
        match mode {
            Mode::Live => {
                let opts = RunViewOptions {
                    viewer: false,
                    start_panels: ws.acq.settings.start_panels.clone(),
                };
                let mut view = RunView::new(ws.acq.app.clone(), opts);
                view.set_edit_run_info(true);
                let id = ws.add_page(LIVE, false, Content::Live(Box::new(view)));
                ws.active = id;
            }
            Mode::MethodsOnly => {
                let editor = MethodEditor::new(ws.acq.app.clone());
                let id = ws.add_page(METHODS, false, Content::Methods(Box::new(editor)));
                ws.active = id;
            }
        }
        // RUSTYCORN_SHOW_PAGES="Settings,/data/run.csv,Edit run info": open these
        // pages (a .csv as an opened run) and the shown page's run info form at
        // start; "Overlay" and "Run log" set up the shown run page (for
        // screenshots and checks of the pages without clicking through)
        if let (Mode::Live, Ok(pages)) = (mode, std::env::var("RUSTYCORN_SHOW_PAGES")) {
            for name in pages.split(',').map(str::trim) {
                match name {
                    SETTINGS => ws.open_settings(),
                    METHODS => ws.open_methods(),
                    EDIT_RUN_INFO => ws.open_run_info_form(ws.active),
                    SHOW_OVERLAY | SHOW_RUN_LOG => ws.set_run_view(ws.active, name == SHOW_OVERLAY),
                    NO_RUN_PROMPT => ws.run_prompt = false,
                    _ if name.to_lowercase().ends_with(".csv") => {
                        match rc_core::csvlog::load_csv_log(Path::new(name)) {
                            Ok(app) => ws.add_run_page(PathBuf::from(name), app),
                            Err(err) => log::error!("Could not open {name}: {err}"),
                        }
                    }
                    _ => {}
                }
            }
        }
        ws
    }

    /// Overlay plot (`overlay`) or run log on for the run page `id`.
    fn set_run_view(&mut self, id: u64, overlay: bool) {
        if let Some(Content::Live(view) | Content::Run(view)) = self
            .pages
            .iter_mut()
            .find(|p| p.id == id)
            .map(|p| &mut p.content)
        {
            if overlay {
                view.set_overlay(true);
            } else {
                view.set_show_log(true);
            }
        }
    }

    fn add_page(&mut self, title: &str, closable: bool, content: Content) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.pages.push(Page {
            id,
            title: title.to_string(),
            closable,
            content,
        });
        id
    }

    /// Show the page titled `title` if there is one (the first such page).
    fn show_existing(&mut self, title: &str) -> bool {
        match self.pages.iter().find(|p| p.title == title) {
            Some(p) => {
                self.active = p.id;
                true
            }
            None => false,
        }
    }

    fn show_page(&mut self, id: u64) {
        self.active = id;
        self.confirm_close = None;
    }

    fn close_page(&mut self, id: u64) {
        let Some(idx) = self.pages.iter().position(|p| p.id == id) else {
            return;
        };
        if !self.pages[idx].closable {
            return;
        }
        if let Content::Methods(editor) = &self.pages[idx].content
            && editor.is_dirty()
            && self.confirm_close != Some(id)
        {
            self.confirm_close = Some(id);
            self.notice =
                "The method editor has unsaved changes. Click × again to discard them.".to_string();
            return;
        }
        self.confirm_close = None;
        if self.notice.starts_with("The method editor") {
            self.notice.clear();
        }
        self.pages.remove(idx);
        self.run_forms.remove(&id);
        if self.active == id {
            self.active = self.pages.first().map_or(0, |p| p.id);
        }
    }

    fn open_settings(&mut self) {
        if self.show_existing(SETTINGS) {
            return;
        }
        if self.settings_page.is_none() {
            self.settings_page = Some(SettingsPage::new(self.acq));
        }
        let id = self.add_page(SETTINGS, true, Content::Settings);
        self.show_page(id);
    }

    /// The run info form of page `id`: the current run's on the Live page, an
    /// opened run's on its page.
    fn open_run_info_form(&mut self, id: u64) {
        let Some(page) = self.pages.iter().find(|p| p.id == id) else {
            return;
        };
        match &page.content {
            Content::Live(_) => {
                let (log, recorded) = {
                    let app = self.acq.app.lock();
                    (app.log_filename.clone(), runinfo_forms::recorded(&app))
                };
                self.live_prompt
                    .edit(&log, &recorded, &self.acq.settings.last_user);
            }
            Content::Run(view) => {
                let form = {
                    let app = view.app.lock();
                    InfoForm::viewer(&app.log_filename, &runinfo_forms::recorded(&app))
                };
                self.run_forms.entry(id).or_insert(form);
            }
            _ => {}
        }
    }

    /// A new run on the Live source opens (or moves) the form for its sample, user and notes.
    fn follow_runs(&mut self) {
        if self.mode != Mode::Live || !self.run_prompt {
            return;
        }
        let (serial, log, live) = {
            let app = self.acq.app.lock();
            (app.run_serial, app.log_filename.clone(), app.live)
        };
        if live {
            self.live_prompt
                .update(serial, &log, &self.acq.settings.last_user);
        }
    }

    /// The run info windows: the live one over every page, an opened run's over its page.
    fn forms_ui(&mut self, ctx: &egui::Context) {
        if let Some(form) = self.live_prompt.form.as_mut() {
            match runinfo_forms::show_form(ctx, form, egui::Id::unique("rc-live-runinfo")) {
                FormAction::None => {}
                FormAction::Close => self.live_prompt.form = None,
                FormAction::Save => {
                    let result = runinfo_forms::save_live(&self.acq.app, form);
                    match result {
                        Ok(msg) => {
                            log::info!("Run info: {msg}");
                            if let Some(user) = form.user()
                                && user != self.acq.settings.last_user
                            {
                                self.acq.settings.last_user = user;
                                if let Err(err) =
                                    rc_core::settings::save_settings(&self.acq.settings)
                                {
                                    log::error!("Could not save the settings: {err}");
                                }
                            }
                            self.live_prompt.form = None;
                        }
                        Err(err) => form.error = err,
                    }
                }
            }
        }
        let active = self.active;
        let Some(form) = self.run_forms.get_mut(&active) else {
            return;
        };
        let Some(Content::Run(view)) = self
            .pages
            .iter()
            .find(|p| p.id == active)
            .map(|p| &p.content)
        else {
            return;
        };
        match runinfo_forms::show_form(ctx, form, egui::Id::unique(("rc-run-runinfo", active))) {
            FormAction::None => {}
            FormAction::Close => {
                self.run_forms.remove(&active);
            }
            FormAction::Save => {
                let recording = {
                    let live = self.acq.app.lock();
                    (live.log.is_some()).then(|| live.log_filename.clone())
                };
                let result =
                    runinfo_forms::save_viewer(&mut view.app.lock(), form, recording.as_deref());
                match result {
                    Ok(msg) => {
                        log::info!("Run info: {msg}");
                        self.run_forms.remove(&active);
                    }
                    Err(err) => form.error = err,
                }
            }
        }
    }

    fn open_methods(&mut self) {
        if self.show_existing(METHODS) {
            return;
        }
        let editor = MethodEditor::new(self.acq.app.clone());
        let closable = self.mode != Mode::MethodsOnly;
        let id = self.add_page(METHODS, closable, Content::Methods(Box::new(editor)));
        self.show_page(id);
    }

    // ── Analyze Run… ────────────────────────────────────────────────────────────
    // The dialog and the parsing run on their own threads; the window keeps
    // drawing and picks the results up in `poll_open`.

    fn open_run(&mut self, ctx: &egui::Context) {
        self.notice.clear();
        if self.picking {
            return;
        }
        let start = self.acq.settings.data_dir();
        let tx = self.open_tx.clone();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("open run dialog".into())
            .spawn(move || {
                let path = rfd::FileDialog::new()
                    .set_title("Analyze Run")
                    .add_filter("CSV log", &["csv", "CSV"])
                    .set_directory(&start)
                    .pick_file();
                log::debug!("File dialog returned: {path:?}");
                let _ = tx.send(OpenMsg::Picked(path));
                ctx.request_repaint();
            });
        match spawned {
            Ok(_) => self.picking = true,
            Err(err) => {
                self.notice = error_notice(&format!("could not open the file dialog: {err}"))
            }
        }
    }

    fn load_run(&mut self, ctx: &egui::Context, path: PathBuf) {
        if !path.is_file() {
            self.notice = error_notice(&format!("{} is not a file", path.display()));
            return;
        }
        let (title, _) = run_title_and_prefix(&path);
        if self.show_existing(&title) || self.loading.contains(&path) {
            return;
        }
        let tx = self.open_tx.clone();
        let ctx = ctx.clone();
        let job_path = path.clone();
        let spawned = std::thread::Builder::new()
            .name("open run".into())
            .spawn(move || {
                let result = rc_core::csvlog::load_csv_log(&job_path);
                let _ = tx.send(OpenMsg::Loaded(job_path, Box::new(result)));
                ctx.request_repaint();
            });
        match spawned {
            Ok(_) => self.loading.push(path),
            Err(err) => self.notice = error_notice(&format!("could not read {title}: {err}")),
        }
    }

    /// A tab for an opened log.
    fn add_run_page(&mut self, path: PathBuf, mut app: AppState) {
        let (title, prefix) = run_title_and_prefix(&path);
        app.log_filename = path.clone();
        // plots exported from this tab go next to the CSV, named after the run
        app.log_dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        app.log_prefix = prefix;
        app.touch();
        log::info!("Opened {}", path.display());
        let opts = RunViewOptions {
            viewer: true,
            start_panels: self.acq.settings.start_panels.clone(),
        };
        let mut view = RunView::new(app.shared(), opts);
        view.set_edit_run_info(true);
        let id = self.add_page(&title, true, Content::Run(Box::new(view)));
        self.show_page(id);
    }

    fn poll_open(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.open_rx.try_recv() {
            match msg {
                OpenMsg::Picked(path) => {
                    self.picking = false;
                    if let Some(path) = path {
                        self.load_run(ctx, path);
                    }
                }
                OpenMsg::Loaded(path, result) => {
                    self.loading.retain(|p| p != &path);
                    let (title, _) = run_title_and_prefix(&path);
                    match *result {
                        Ok(app) => self.add_run_page(path, app),
                        Err(err) => {
                            log::error!("Could not open {}: {err}", path.display());
                            self.notice = error_notice(&format!("could not open {title}: {err}"));
                        }
                    }
                }
            }
        }
    }

    // ── Top bar ──────────────────────────────────────────────────────────────

    fn top_bar(&mut self, ui: &mut Ui) {
        let t = palette(ui.ctx());
        let mut show: Option<u64> = None;
        let mut close: Option<u64> = None;
        ui.horizontal_centered(|ui| {
            wordmark(ui);
            ui.add_space(14.0);
            let (rect, _) = ui.allocate_exact_size(Vec2::new(1.0, 18.0), Sense::hover());
            ui.painter().vline(
                rect.center().x,
                rect.y_range(),
                Stroke::new(1.0, t.separator),
            );
            ui.add_space(10.0);

            // Flat tabs: the active one bold with an accent underline; × closes.
            for page in &self.pages {
                let active = page.id == self.active;
                ui.scope(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let tab = rc_theme::tab(ui, &page.title, active);
                    if tab.clicked() {
                        show = Some(page.id);
                    }
                    if page.closable {
                        let pending = self.confirm_close == Some(page.id);
                        if close_button(ui, pending).on_hover_text("Close").clicked() {
                            close = Some(page.id);
                        }
                    }
                });
                ui.add_space(10.0);
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if self.mode == Mode::Live {
                    let active_title = self
                        .pages
                        .iter()
                        .find(|p| p.id == self.active)
                        .map(|p| p.title.clone());
                    let button = |ui: &mut Ui, text: &str| {
                        let selected = active_title.as_deref() == Some(text);
                        ui.add(menu_button_widget(text, selected, &t))
                    };
                    if button(ui, SETTINGS).clicked() {
                        self.open_settings();
                    }
                    if button(ui, METHODS).clicked() {
                        self.open_methods();
                    }
                    let busy = self.picking || !self.loading.is_empty();
                    let open = ui
                        .add_enabled(!self.picking, menu_button_widget("Analyze Run…", false, &t));
                    if open.clicked() {
                        self.open_run(ui.ctx());
                    }
                    if busy {
                        ui.spinner().on_hover_text(if self.picking {
                            "Choosing a file…"
                        } else {
                            "Reading the log…"
                        });
                    }
                    ui.add_space(8.0);
                }
                if !self.notice.is_empty() {
                    let notice = ui
                        .add(
                            egui::Label::new(
                                RichText::new(&self.notice)
                                    .font(medium(FS_BODY))
                                    .color(t.err),
                            )
                            .truncate()
                            .sense(Sense::click()),
                        )
                        .on_hover_text(format!("{}\n\nClick to dismiss.", self.notice));
                    if notice.clicked() {
                        self.notice.clear();
                    }
                }
            });
        });
        if let Some(id) = show {
            self.show_page(id);
        }
        if let Some(id) = close {
            self.close_page(id);
        }
    }

    fn page_ui(&mut self, ui: &mut Ui) {
        let Some(idx) = self.pages.iter().position(|p| p.id == self.active) else {
            return;
        };
        let mut edit = false;
        match &mut self.pages[idx].content {
            Content::Live(view) => {
                view.set_refresh_hz(self.acq.settings.refresh_hz);
                view.ui(ui);
                edit = view.take_edit_request();
            }
            // An opened run also shows the live connection in its status bar.
            Content::Run(view) => {
                let status = (self.mode == Mode::Live).then(|| self.acq.app.lock().status.clone());
                view.set_connection_status(status);
                view.ui(ui);
                edit = view.take_edit_request();
            }
            Content::Settings => {
                let page = self
                    .settings_page
                    .get_or_insert_with(|| SettingsPage::new(self.acq));
                page.ui(ui, self.acq);
            }
            Content::Methods(editor) => editor.ui(ui),
        }
        if edit {
            self.open_run_info_form(self.active);
        }
    }

    fn active_content(&self) -> Option<&Content> {
        self.pages
            .iter()
            .find(|p| p.id == self.active)
            .map(|p| &p.content)
    }

    fn methods_dirty(&self) -> bool {
        self.pages
            .iter()
            .any(|p| matches!(&p.content, Content::Methods(e) if e.is_dirty()))
    }

    /// The acquisition keeps running when its log file cannot be created; say so
    /// in the top bar, since nothing is being recorded then.
    fn check_log(&mut self) {
        if self.mode != Mode::Live {
            return;
        }
        let error = if self.acq.is_running() {
            self.acq.app.lock().log_error.clone()
        } else {
            None
        };
        match error {
            Some(err) if !self.no_log_notice => {
                self.notice = error_notice(&format!("data are not being saved — {err}"));
                self.no_log_notice = true;
            }
            None if self.no_log_notice => {
                if self.notice.contains("data are not being saved") {
                    self.notice.clear();
                }
                self.no_log_notice = false;
            }
            _ => {}
        }
    }

    /// Unsaved method edits: the first close request only warns (like × on the tab).
    fn handle_close_request(&mut self, ctx: &egui::Context) {
        if !ctx.input(|i| i.viewport().close_requested()) || crate::interrupt::requested() {
            return;
        }
        if self.methods_dirty() && !self.confirm_quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_quit = true;
            self.notice =
                "The method editor has unsaved changes. Close the window again to discard them."
                    .to_string();
        }
    }

    /// Live pages redraw at the display refresh rate from Settings; the method
    /// editor checks now and then for methods the Akta saved.
    fn schedule_repaint(&self, ctx: &egui::Context) {
        match self.active_content() {
            Some(Content::Live(_)) if self.acq.is_running() => {
                let hz = self.acq.settings.refresh_hz.clamp(0.2, 60.0);
                ctx.request_repaint_after(Duration::from_secs_f64(1.0 / hz));
            }
            Some(Content::Methods(_)) if self.mode == Mode::Live && self.acq.is_running() => {
                ctx.request_repaint_after(Duration::from_secs(1));
            }
            _ => {}
        }
    }
}

impl eframe::App for Workspace<'_> {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        crate::interrupt::close_if_requested(ctx);
        self.poll_open(ctx);
        if let Some(page) = &mut self.settings_page {
            page.poll(self.acq);
        }
        self.handle_close_request(ctx);
        self.check_log();
        self.follow_runs();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let t = palette(ui.ctx());
        egui::Panel::top("rustycorn-top-bar")
            .frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin {
                left: 16,
                right: 14,
                top: 9,
                bottom: 9,
            }))
            .show_separator_line(true)
            .show(ui, |ui| self.top_bar(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(t.dock))
            .show(ui, |ui| self.page_ui(ui));
        self.forms_ui(ui.ctx());
        self.schedule_repaint(ui.ctx());
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.panel_fill.to_normalized_gamma_f32()
    }
}

/// The app name at the left of the top bar: the acorn logo and the name.
fn wordmark(ui: &mut Ui) {
    let t = palette(ui.ctx());
    match crate::logo::mark(ui.ctx()) {
        Some(tex) => {
            ui.add(egui::Image::new(&tex).fit_to_exact_size(Vec2::splat(24.0)));
        }
        None => {
            ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
        }
    }
    ui.add_space(2.0);
    ui.label(
        RichText::new(rc_core::APP_NAME)
            .font(semibold(15.0))
            .color(t.text),
    );
}

/// The × after a closable tab; red once it waits for a confirming second click.
fn close_button(ui: &mut Ui, pending: bool) -> egui::Response {
    let t = palette(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(22.0, 20.0), Sense::click());
    let hovered = resp.hovered();
    if hovered {
        ui.painter()
            .rect_filled(rect.shrink(1.0), CornerRadius::same(4), t.hover);
    }
    let color = if pending {
        t.err
    } else if hovered {
        t.text
    } else {
        t.text_faint
    };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        "×",
        egui::FontId::proportional(15.0),
        color,
    );
    resp
}

/// A top-bar menu button; `selected` while its page is shown.
fn menu_button_widget<'t>(
    text: &'t str,
    selected: bool,
    t: &rc_theme::Palette,
) -> egui::Button<'t> {
    let color = if selected { t.accent_text } else { t.text };
    let button = egui::Button::new(RichText::new(text).font(medium(FS_BODY)).color(color))
        .corner_radius(CornerRadius::same(t.radius_sm as u8))
        .min_size(Vec2::new(0.0, 26.0));
    if selected {
        button
            .fill(t.accent_soft)
            .stroke(Stroke::new(1.0, t.accent_border))
    } else {
        button
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_tabs_are_named_after_the_file() {
        let (title, prefix) = run_title_and_prefix(Path::new("/data/akta_20261020_114429.csv"));
        assert_eq!(title, "akta_20261020_114429.csv");
        assert_eq!(prefix, "akta_20261020_114429");
        assert_eq!(run_title_and_prefix(Path::new("RUN.CSV")).1, "RUN");
    }

    #[test]
    fn notices_are_short() {
        let n = error_notice(&"x".repeat(500));
        assert_eq!(n.chars().count(), "Error: ".len() + NOTICE_MAX_CHARS);
        assert_eq!(error_notice("boom"), "Error: boom");
    }
}
