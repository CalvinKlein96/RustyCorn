//! The run page: toolbar, stacked or overlay
//! chromatogram, time slider, run log, side panel and status bar, for the live
//! run or an opened log.
//!
//! Layout, one area per job:
//!   toolbar      how the data is viewed            │ right column: what the
//!   plots, slider, run log                          │ instrument is doing
//!   status bar   connection, recording, mouse help
//!
//! The plots are drawn with egui's painter (plot.rs), not egui_plot: linked x,
//! collapsed panels, fixed tick-label room, box zoom on drag and the pixel
//! layout of the event labels need exact control, and a hand-made plot draws
//! only what changed. The AppState lock is taken once per refresh to copy new
//! data (data.rs); everything else works on that copy and is cached by data
//! version and view (traces.rs), so a frame without changes recomputes nothing.

mod data;
mod export;
mod plot;
mod ticks;
mod traces;
mod view;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use egui::{Align, Layout, Margin, Rect, RichText, Stroke, UiBuilder, pos2, vec2};
use rc_core::{EventKind, InstrumentState, SharedApp};
use rc_panels::{PanelData, RunLogState, ShownMode, StatusInfo};
use rc_plotdata::{Channel, EventDisplay, FractionSpan, LineStyle, Pt, XS_TIME, XS_VOLUME, XScale};

use view::{Pane, ViewState};

#[derive(Clone, Debug)]
pub struct RunViewOptions {
    /// an opened (read-only) log instead of the live run
    pub viewer: bool,
    /// plot panels switched on at start (Settings::start_panels)
    pub start_panels: Vec<String>,
}

/// The x axis (toolbar menu).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum XMode {
    #[default]
    Time,
    Volume,
    ColumnVolumes,
}

impl XMode {
    pub const ALL: [XMode; 3] = [XMode::Time, XMode::Volume, XMode::ColumnVolumes];
    pub fn label(self) -> &'static str {
        match self {
            XMode::Time => "Plot vs time",
            XMode::Volume => "Plot vs volume",
            XMode::ColumnVolumes => "Plot vs column volumes",
        }
    }
}

const HELP: &str =
    "Drag: zoom · Right-drag: pan · Ctrl-click: reset · Alt/Shift-click: inspect moment";
/// Windows narrower than this hide the mouse help.
const HELP_MIN_WIDTH: f32 = 1350.0;

/// The moment the time slider selects: at the right end it follows the newest data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Selection {
    following: bool,
    index: usize,
}

impl Selection {
    /// New data: keep the newest point selected while following.
    fn on_data(&mut self, n: usize) {
        if self.following || self.index + 1 >= n {
            self.following = true;
            self.index = n.saturating_sub(1);
        }
    }

    /// Pick point `i`; picking the last one means following again.
    fn select(&mut self, i: usize, n: usize) {
        self.index = i.min(n.saturating_sub(1));
        self.following = self.index + 1 >= n;
    }

    fn select_end(&mut self, n: usize) {
        self.following = true;
        self.index = n.saturating_sub(1);
    }
}

/// An event line in x units.
struct EvLine {
    x: f64,
    color: egui::Color32,
    dashed: bool,
}

/// Data derived from the snapshot, recomputed only when the data or the x scale change.
#[derive(Default)]
struct Derived {
    /// data version, theme (dark) and event display the event lines and labels were made for
    version: Option<(u64, bool, EventDisplay)>,
    /// data version the run info was read from
    info_version: Option<u64>,
    info: BTreeMap<String, String>,
    cv: f64,
    /// labelled events (as `events_shown` allows): (time ms, volume mL) and label text
    label_tv: Vec<(f64, f64)>,
    label_text: Vec<String>,
    /// events with a line: (time ms, volume mL, colour, dashed)
    line_tv: Vec<(f64, f64, egui::Color32, bool)>,
    xkey: Option<(u64, u64)>,
    /// counts the rebuilds of `label_x`: the label layout is keyed by it, so it
    /// never outlives the labels it was made for (the toolbar can change the
    /// event display or x units after `derive`, within the same frame)
    label_gen: u64,
    label_x: Vec<f64>,
    lines: Vec<EvLine>,
    spans: Vec<FractionSpan>,
    flow: Vec<Pt>,
    /// x range of the trend and of the flow set points
    trend_x: Option<(f64, f64)>,
    flow_x: Option<(f64, f64)>,
    flow_y: Option<(f64, f64)>,
}

/// The instrument state at the selected past moment (replayed from the log).
struct Shown {
    version: u64,
    index: usize,
    state: InstrumentState,
    flow: Option<f64>,
}

/// Event label anchors laid out for one view (rc_plotdata::event_label_positions).
#[derive(Default)]
struct LabelLayout {
    key: Option<[u64; 8]>,
    pos: Vec<Pt>,
}

/// Frame timing of `ui` (CPU time spent in the run view, without tessellation).
#[derive(Default, Clone, Copy, Debug)]
pub struct Perf {
    pub frames: u64,
    pub last_ms: f32,
    pub max_ms: f32,
    pub sum_ms: f64,
}

/// Rectangle zoom in progress (primary drag in a pane).
#[derive(Clone, Copy, Debug)]
struct BoxDrag {
    pane: Pane,
    start: egui::Pos2,
}

pub struct RunView {
    pub app: SharedApp,
    pub opts: RunViewOptions,
    id: egui::Id,
    snap: data::Snapshot,
    refresh: Duration,
    last_sync: Option<Instant>,
    force_sync: bool,
    xmode: XMode,
    /// last x scale drawn (bits), to rescale when it changes
    last_xs: Option<u64>,
    /// panels switched on, by `Channel as usize` (UV always)
    shown_panels: [bool; 6],
    overlay: bool,
    show_log: bool,
    highlighted: Option<Channel>,
    /// which event lines and labels the plots (and the PDF) show
    events_shown: EventDisplay,
    /// fraction shading and numbers on the plots (and in the PDF)
    show_fractions: bool,
    view: ViewState,
    sel: Selection,
    traces: traces::TraceCache,
    derived: Derived,
    labels: LabelLayout,
    shown: Option<Shown>,
    mark_note: String,
    export: export::Exporter,
    log_state: RunLogState,
    status_override: Option<String>,
    drag: Option<BoxDrag>,
    /// trend index under the mouse (hover line), set while drawing
    hover: Option<usize>,
    /// measured widths of the two toolbar groups (last frame)
    tb_w: [f32; 2],
    /// measured height of slider + run log (last frame)
    bottom_h: f32,
    perf: Perf,
    /// the Run info card has an edit button (`set_edit_run_info`)
    edit_run_info: bool,
    /// its button was clicked (`take_edit_request`)
    edit_requested: bool,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

impl RunView {
    pub fn new(app: SharedApp, opts: RunViewOptions) -> RunView {
        let mut shown_panels = [false; 6];
        shown_panels[Channel::Uv as usize] = true;
        for ch in Channel::ALL {
            if let Some(name) = ch.panel_name() {
                shown_panels[ch as usize] = opts.start_panels.iter().any(|p| p == name);
            }
        }
        RunView {
            app,
            opts,
            id: egui::Id::unique(("rc-runview", NEXT_ID.fetch_add(1, Ordering::Relaxed))),
            snap: data::Snapshot::default(),
            refresh: Duration::from_millis(500),
            last_sync: None,
            force_sync: true,
            xmode: XMode::Time,
            last_xs: None,
            shown_panels,
            overlay: false,
            show_log: false,
            highlighted: None,
            events_shown: EventDisplay::All,
            show_fractions: true,
            view: ViewState::default(),
            // viewer mode starts at the end of the log
            sel: Selection {
                following: true,
                index: 0,
            },
            traces: traces::TraceCache::default(),
            derived: Derived::default(),
            labels: LabelLayout::default(),
            shown: None,
            mark_note: String::new(),
            export: export::Exporter::default(),
            log_state: RunLogState::default(),
            status_override: None,
            drag: None,
            hover: None,
            tb_w: [0.0; 2],
            bottom_h: 40.0,
            perf: Perf::default(),
            edit_run_info: false,
            edit_requested: false,
        }
    }

    // ── Options for the app shell (and the demo) ────────────────────────────

    /// Live data is copied at most this often (Settings → Display refresh); while
    /// live the page also asks for a repaint at this rate.
    pub fn set_refresh_hz(&mut self, hz: f64) {
        let hz = if hz.is_finite() {
            hz.clamp(0.2, 60.0)
        } else {
            2.0
        };
        self.refresh = Duration::from_secs_f64(1.0 / hz);
    }

    /// Connection status shown in the status bar instead of `AppState::status`
    /// (an opened run still shows the state of the live connection).
    /// An edit button on the Run info card ("Edit" while live, "Edit run info"
    /// for an opened run); the app asks `take_edit_request` after drawing.
    pub fn set_edit_run_info(&mut self, on: bool) {
        self.edit_run_info = on;
    }

    /// True once after the Run info card's edit button was clicked.
    pub fn take_edit_request(&mut self) -> bool {
        std::mem::take(&mut self.edit_requested)
    }

    pub fn set_connection_status(&mut self, status: Option<String>) {
        self.status_override = status;
    }

    pub fn set_overlay(&mut self, on: bool) {
        self.overlay = on;
        if !on {
            self.highlighted = None;
        }
    }

    pub fn set_show_log(&mut self, on: bool) {
        self.show_log = on;
    }

    pub fn set_x_mode(&mut self, m: XMode) {
        self.xmode = m;
    }

    /// Switch a panel (by its Settings name: "%B", "Cond", "pH", "Pressure", "Flow").
    pub fn set_panel(&mut self, name: &str, on: bool) {
        if let Some(ch) = Channel::ALL
            .into_iter()
            .find(|c| c.panel_name() == Some(name))
        {
            self.shown_panels[ch as usize] = on;
        }
    }

    /// Which event lines and labels the plots and the PDF show.
    pub fn set_event_display(&mut self, d: EventDisplay) {
        self.events_shown = d;
    }

    pub fn event_display(&self) -> EventDisplay {
        self.events_shown
    }

    /// Fraction shading and numbers on the plots and in the PDF.
    pub fn set_show_fractions(&mut self, on: bool) {
        self.show_fractions = on;
    }

    pub fn shows_fractions(&self) -> bool {
        self.show_fractions
    }

    /// Highlight an overlay trace (None = none).
    pub fn set_highlight(&mut self, ch: Option<Channel>) {
        self.highlighted = ch;
    }

    /// Inspect the trend point `index` (time slider).
    pub fn select_point(&mut self, index: usize) {
        self.sel.select(index, self.snap.trend.len());
    }

    /// Zoom all panels to an x-range (stops following).
    pub fn zoom_x(&mut self, lo: f64, hi: f64) {
        let y = self.view.lim.y_of(Pane::Chan(Channel::Uv));
        self.view.zoom_to(Pane::Chan(Channel::Uv), (lo, hi), y);
    }

    /// Frame timing of the run view so far.
    pub fn perf(&self) -> Perf {
        self.perf
    }

    // ── Data ────────────────────────────────────────────────────────────────

    /// Copy what changed from the shared state (lock held only for the copy).
    /// Live data is batched at the refresh rate (throttled, not per sample).
    fn sync(&mut self) {
        let due = self.force_sync
            || self.opts.viewer
            || !self.snap.live
            || self.last_sync.is_none_or(|t| t.elapsed() >= self.refresh);
        if !due {
            return;
        }
        let changed = {
            let app = self.app.lock();
            self.snap.sync(&app)
        };
        self.force_sync = false;
        self.last_sync = Some(Instant::now());
        if changed {
            self.sel.on_data(self.snap.trend.len());
        }
    }

    fn derive(&mut self, t: &rc_theme::Palette) {
        let cv = self.cv_of_snapshot();
        let xs = self.xs_for(cv);
        let s = &self.snap;
        let d = &mut self.derived;
        let shown = self.events_shown;
        let vkey = (s.version, t.dark, shown);
        if d.version != Some(vkey) {
            d.version = Some(vkey);
            // force the x positions to be redone for the new selection
            d.xkey = None;
            let mut lab = rc_plotdata::labelled_events(&s.events);
            lab.retain(|e| shown.shows(e));
            d.label_tv = lab.iter().map(|e| (e.cum_time_ms, e.cum_vol_ml)).collect();
            d.label_text = lab
                .iter()
                .map(|e| rc_plotdata::event_label_top(e))
                .collect();
            d.line_tv = s
                .events
                .iter()
                .filter(|e| shown.shows(e))
                .filter_map(|e| {
                    let (c, style) = rc_plotdata::event_style(e.kind)?;
                    let c = if e.kind == EventKind::Mark {
                        t.mark
                    } else {
                        rc_theme::rgb3(c)
                    };
                    let col = c.gamma_multiply(rc_plotdata::EVENT_LINE_ALPHA);
                    Some((e.cum_time_ms, e.cum_vol_ml, col, style != LineStyle::Solid))
                })
                .collect();
        }
        let key = (s.version, xs.to_bits());
        if d.xkey != Some(key) {
            d.xkey = Some(key);
            d.label_gen = d.label_gen.wrapping_add(1);
            d.label_x = d
                .label_tv
                .iter()
                .map(|&(t, v)| rc_plotdata::x(t, v, xs))
                .collect();
            d.lines = d
                .line_tv
                .iter()
                .map(|&(t, v, color, dashed)| EvLine {
                    x: rc_plotdata::x(t, v, xs),
                    color,
                    dashed,
                })
                .collect();
            d.spans = rc_plotdata::fraction_spans(&s.fractions, &s.trend, xs);
            d.flow = rc_plotdata::flow_setpoints(&s.events, &s.trend, xs);
            d.trend_x = match (s.trend.first(), s.trend.last()) {
                (Some(a), Some(b)) => {
                    Some((rc_plotdata::trend_x(a, xs), rc_plotdata::trend_x(b, xs)))
                }
                _ => None,
            };
            d.flow_x = match (d.flow.first(), d.flow.last()) {
                (Some(a), Some(b)) => Some((a[0], b[0])),
                _ => None,
            };
            d.flow_y = rc_plotdata::y_extrema(&d.flow);
        }
    }

    /// The x scale: 0 time, 1 mL, column volume → CV (falls back to mL without one).
    fn xs(&self) -> XScale {
        self.xs_for(self.derived.cv)
    }

    fn xs_for(&self, cv: f64) -> XScale {
        match self.xmode {
            XMode::Time => XS_TIME,
            XMode::Volume => XS_VOLUME,
            XMode::ColumnVolumes if cv > 0.0 => cv,
            XMode::ColumnVolumes => XS_VOLUME,
        }
    }

    /// Column volume of the current data (run info is re-read when the data changed).
    fn cv_of_snapshot(&mut self) -> f64 {
        if self.derived.info_version != Some(self.snap.version) {
            self.derived.info_version = Some(self.snap.version);
            self.derived.info = rc_core::runinfo::run_info(&self.snap.events);
            self.derived.cv = rc_core::runinfo::column_volume(&self.derived.info);
        }
        self.derived.cv
    }

    fn xlabel(&self) -> String {
        let mut s = rc_plotdata::xlabel(self.xs()).to_string();
        if self.xmode == XMode::ColumnVolumes && self.derived.cv <= 0.0 {
            s.push_str(" — no column volume in this run's info");
        }
        s
    }

    /// Panes on screen, top to bottom.
    fn panes(&self) -> Vec<Pane> {
        if self.overlay {
            return vec![Pane::Overlay];
        }
        Channel::ALL
            .into_iter()
            .filter(|&c| self.shown_panels[c as usize])
            .map(Pane::Chan)
            .collect()
    }

    /// Channels drawn in the overlay (UV always, the others as toggled).
    fn overlay_channels(&self) -> Vec<Channel> {
        Channel::ALL
            .into_iter()
            .filter(|&c| self.shown_panels[c as usize])
            .collect()
    }

    /// Autoscale extents of what is drawn; the x of hidden panels does not count.
    fn extents(&self) -> view::Extents {
        let d = &self.derived;
        let flow_drawn = self.shown_panels[Channel::Flow as usize];
        let x = match (d.trend_x, d.flow_x.filter(|_| flow_drawn)) {
            (Some(a), Some(b)) => Some((a.0.min(b.0), a.1.max(b.1))),
            (a, b) => a.or(b),
        };
        let mut y = self.snap.yext;
        y[Channel::Flow as usize] = d.flow_y;
        view::Extents { x, y }
    }

    /// Follow-live autoscale and the traces for the current x-window.
    fn prepare(&mut self) {
        let auto = view::auto_limits(&self.extents());
        self.view.track(&auto);
        let mut needed = [false; 6];
        if self.overlay {
            for c in self.overlay_channels() {
                needed[c as usize] = true;
            }
        } else {
            needed = self.shown_panels;
        }
        let xs = self.xs();
        let xwin = self.view.xwin();
        self.traces
            .update(&self.snap.trend, self.snap.version, xs, xwin, needed);
    }

    /// Range (lo, hi) of an overlay trace that maps to 0–100 %, and its scaled points.
    fn overlay_trace(&mut self, ch: Channel) -> (&[Pt], (f64, f64)) {
        let range = if ch == Channel::Flow {
            None
        } else {
            self.snap.yext[ch as usize].map(view::flat_below_f32)
        };
        let fkey = self.snap.version.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ self.xs().to_bits();
        self.traces.overlay(ch, &self.derived.flow, range, fkey)
    }

    fn add_mark(&mut self) {
        let note = self.mark_note.trim();
        let text = if note.is_empty() {
            "Mark".to_string()
        } else {
            note.to_string()
        };
        self.app.lock().push_event(EventKind::Mark, text);
        self.mark_note.clear();
        self.force_sync = true;
    }

    /// Save the displayed plot (visible panels, current limits) to PDF.
    fn save_pdf(&mut self, ctx: &egui::Context) {
        self.force_sync = true;
        self.sync();
        if self.snap.trend.is_empty() {
            log::warn!("Nothing to save yet — no data on the plot");
            self.export
                .say("Nothing to save yet — no data on the plot", true);
            return;
        }
        let panels = Channel::ALL
            .into_iter()
            .filter(|&c| self.shown_panels[c as usize])
            .map(|c| rc_export::PanelSpec {
                channel: c,
                ylim: self.view.lim.y[c as usize],
            })
            .collect();
        let req = rc_export::ExportRequest {
            trend: self.snap.trend.clone(),
            events: self.snap.events.clone(),
            fractions: self.snap.fractions.clone(),
            panels,
            xs: self.xs(),
            xlim: self.view.lim.x,
            overlay_ylim: if self.overlay {
                self.view.lim.ov
            } else {
                rc_plotdata::OVERLAY_YLIMS
            },
            highlighted: self.highlighted,
            title: (!self.snap.log_file.is_empty()).then(|| self.snap.log_file.clone()),
            events_shown: self.events_shown,
            show_fractions: self.show_fractions,
        };
        self.export.counter += 1;
        let prefix = if self.snap.log_prefix.is_empty() {
            "akta"
        } else {
            self.snap.log_prefix.as_str()
        };
        let path = rc_export::figure_path(&self.snap.log_dir, prefix, self.export.counter);
        self.export.start(req, path, ctx);
    }

    // ── Shown moment: live, or a past moment picked with the time slider ──────

    /// (state, point, flow, mode) for the side panel. The state is the live
    /// instrument while following, else rebuilt from the log at that moment.
    fn shown(&mut self) -> (Option<usize>, ShownMode) {
        let n = self.snap.trend.len();
        let idx = (n > 0).then(|| self.sel.index.min(n - 1));
        let mode = if self.sel.following && !self.opts.viewer {
            ShownMode::Live
        } else if self.sel.following {
            ShownMode::EndOfLog
        } else {
            ShownMode::Past
        };
        if let Some(i) = idx {
            let v = self.snap.version;
            if !self
                .shown
                .as_ref()
                .is_some_and(|s| s.version == v && s.index == i)
            {
                let p = &self.snap.trend[i];
                let state = if mode == ShownMode::Live {
                    InstrumentState::default()
                } else {
                    rc_plotdata::state_at(&self.snap.events, p)
                };
                let flow = rc_plotdata::flow_at(&self.snap.events, p);
                self.shown = Some(Shown {
                    version: v,
                    index: i,
                    state,
                    flow,
                });
            }
        }
        (idx, mode)
    }

    // ── Page ─────────────────────────────────────────────────────────────────

    /// Draw the whole page into `ui` (the area below the app's top bar).
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let t0 = Instant::now();
        let ctx = ui.ctx().clone();
        self.sync();
        self.export.poll();
        self.derive(&rc_theme::palette(&ctx));
        let xs_bits = self.xs().to_bits();
        if self.last_xs.is_some_and(|b| b != xs_bits) {
            // new x units: fit the data in them (a zoom reset)
            self.view.resume();
        }
        self.last_xs = Some(xs_bits);
        if self
            .highlighted
            .is_some_and(|c| !self.shown_panels[c as usize])
        {
            // a highlighted trace that was just hidden cannot stay highlighted
            self.highlighted = None;
        }
        self.prepare();

        let t = rc_theme::palette(&ctx);
        let full = ui.available_rect_before_wrap();
        ui.painter().rect_filled(full, 0.0, t.dock);
        let page = full.shrink2(vec2(12.0, 10.0));
        let gap = 10.0;
        let status_h = rc_panels::STATUS_BAR_H + 12.0;
        let status = Rect::from_min_max(pos2(page.left(), page.bottom() - status_h), page.max);
        let body = Rect::from_min_max(page.min, pos2(page.right(), status.top() - gap));
        // the right column keeps at least 340 px
        let rightw = (0.25 * full.width())
            .clamp(340.0, 480.0)
            .min(body.width() * 0.5);
        let right = Rect::from_min_max(pos2(body.right() - rightw, body.top()), body.max);
        let left = Rect::from_min_max(body.min, pos2(right.left() - gap, body.bottom()));

        let id = self.id;
        let child = |r: Rect, salt: &str| {
            UiBuilder::new()
                .id_salt((id, salt))
                .max_rect(r)
                .layout(Layout::top_down(Align::Min))
        };
        ui.scope_builder(child(left, "left"), |ui| {
            ui.set_clip_rect(left.expand(2.0).intersect(ui.clip_rect()));
            self.left_column(ui);
        });
        ui.scope_builder(child(right, "right"), |ui| self.right_column(ui));
        ui.scope_builder(child(status, "status"), |ui| {
            self.status_bar(ui, full.width())
        });
        ui.advance_cursor_after_rect(full);

        if self.snap.live && !self.opts.viewer {
            ctx.request_repaint_after(self.refresh);
        }
        if let Some((_, left)) = self.export.current() {
            ctx.request_repaint_after(left);
        }
        let ms = t0.elapsed().as_secs_f32() * 1000.0;
        let p = &mut self.perf;
        p.frames += 1;
        p.last_ms = ms;
        p.max_ms = p.max_ms.max(ms);
        p.sum_ms += ms as f64;
    }

    fn left_column(&mut self, ui: &mut egui::Ui) {
        self.toolbar(ui);
        ui.add_space(2.0);
        if self.overlay {
            self.legend(ui);
        }
        let avail = ui.available_rect_before_wrap();
        let bottom = (avail.bottom() - self.bottom_h).max(avail.top() + 120.0);
        let plot_rect = Rect::from_min_max(avail.min, pos2(avail.right(), bottom));
        self.plot(ui, plot_rect);
        ui.advance_cursor_after_rect(plot_rect);
        let r = ui
            .scope(|ui| {
                ui.add_space(4.0);
                self.slider_row(ui);
                if self.show_log {
                    self.run_log(ui);
                }
            })
            .response
            .rect;
        let h = r.height() + ui.spacing().item_spacing.y;
        if (h - self.bottom_h).abs() > 0.5 {
            self.bottom_h = h;
            ui.ctx().request_repaint();
        }
    }

    // ── Toolbar: two groups — view controls, and which panels are shown.
    // In a narrow window the panel group moves to a second row.
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let fits = self.tb_w[0] + self.tb_w[1] + 28.0 <= ui.available_width();
        if fits {
            ui.horizontal(|ui| {
                let a = ui
                    .horizontal(|ui| self.toolbar_view(ui))
                    .response
                    .rect
                    .width();
                ui.add_space(28.0 - ui.spacing().item_spacing.x);
                let b = ui
                    .horizontal(|ui| self.toolbar_panels(ui))
                    .response
                    .rect
                    .width();
                self.tb_w = [a, b];
            });
        } else {
            let a = ui
                .horizontal(|ui| self.toolbar_view(ui))
                .response
                .rect
                .width();
            let b = ui
                .horizontal(|ui| self.toolbar_panels(ui))
                .response
                .rect
                .width();
            self.tb_w = [a, b];
        }
    }

    fn toolbar_view(&mut self, ui: &mut egui::Ui) {
        let t = rc_theme::palette(ui.ctx());
        ui.spacing_mut().item_spacing.x = 10.0;
        egui::ComboBox::from_id_salt((self.id, "xmode"))
            .selected_text(self.xmode.label())
            .width(170.0)
            .show_ui(ui, |ui| {
                for m in XMode::ALL {
                    ui.selectable_value(&mut self.xmode, m, m.label());
                }
            });
        if ui
            .button(if self.overlay { "Stacked" } else { "Overlay" })
            .clicked()
        {
            self.set_overlay(!self.overlay);
            if self.view.follow {
                self.view.resume();
            }
        }
        // Events: all, without the method steps, or none (a cleaner plot);
        // the run log lists everything. Fractions have their own switch.
        egui::ComboBox::from_id_salt((self.id, "events"))
            .selected_text(format!("Events: {}", self.events_shown.label()))
            .width(136.0)
            .show_ui(ui, |ui| {
                for d in EventDisplay::ALL {
                    ui.selectable_value(&mut self.events_shown, d, d.label());
                }
            })
            .response
            .on_hover_text("Event lines and labels on the plot and in the PDF; \"No steps\" hides the method steps and break points");
        ui.checkbox(&mut self.show_fractions, "Fractions")
            .on_hover_text("Fraction shading and numbers on the plot and in the PDF");
        let save = ui.add_enabled(!self.export.busy(), egui::Button::new("Save PDF"));
        if save
            .on_hover_text("Save the plot as shown to a PDF next to the log")
            .clicked()
        {
            self.save_pdf(ui.ctx());
        }
        ui.checkbox(&mut self.show_log, "Show log");
        // Event marks: type a note (optional) and press Mark or Enter; live runs only.
        if !self.opts.viewer {
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.mark_note)
                    .hint_text(RichText::new("Mark note…").color(t.text_faint))
                    .desired_width(150.0),
            );
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui
                .button("Mark")
                .on_hover_text("Add an event mark at the current moment")
                .clicked()
                || enter
            {
                self.add_mark();
            }
        }
    }

    fn toolbar_panels(&mut self, ui: &mut egui::Ui) {
        let t = rc_theme::palette(ui.ctx());
        ui.spacing_mut().item_spacing.x = 12.0;
        for ch in [
            Channel::Pctb,
            Channel::Cond,
            Channel::Ph,
            Channel::Pres,
            Channel::Flow,
        ] {
            let name = ch.panel_name().unwrap_or(ch.name());
            let mut on = self.shown_panels[ch as usize];
            let label = RichText::new(name).color(if on {
                channel_color(&t, ch)
            } else {
                t.text_dim
            });
            if ui.checkbox(&mut on, label).changed() {
                self.shown_panels[ch as usize] = on;
            }
        }
        ui.add_space(4.0);
        // Follow live: auto-rescale to all data on each update. Switched off as soon
        // as the user zooms or pans, so a zoomed view stays put while the run continues.
        let mut follow = self.view.follow;
        if ui.checkbox(&mut follow, "Follow live").changed() {
            if follow {
                self.view.resume()
            } else {
                self.view.freeze()
            }
        }
    }

    /// Overlay legend: one button per visible trace above the plot, so it never
    /// covers data. Clicking one highlights that trace (y-axis in its units);
    /// clicking it again clears it.
    fn legend(&mut self, ui: &mut egui::Ui) {
        let t = rc_theme::palette(ui.ctx());
        let chans = self.overlay_channels();
        let ncol = if ui.available_width() >= 1000.0 { 6 } else { 3 };
        let mut clicked = None;
        egui::Grid::new((self.id, "legend"))
            .spacing(vec2(6.0, 4.0))
            .show(ui, |ui| {
                for (i, ch) in chans.iter().copied().enumerate() {
                    let (_, range) = self.overlay_trace(ch);
                    let label = rc_plotdata::overlay_label(ch.name(), ch.unit(), range);
                    let label = label.trim_start_matches('━').trim_start().to_string();
                    if legend_button(ui, &t, ch, &label, self.highlighted == Some(ch)).clicked() {
                        clicked = Some(ch);
                    }
                    if (i + 1) % ncol == 0 {
                        ui.end_row();
                    }
                }
            });
        if let Some(ch) = clicked {
            self.highlighted = if self.highlighted == Some(ch) {
                None
            } else {
                Some(ch)
            };
        }
        ui.add_space(2.0);
    }

    // ── Time slider: inspect a past moment ─────────────────────────────────────
    // Selects a trend point; at the right end it follows the newest data.
    fn slider_row(&mut self, ui: &mut egui::Ui) {
        let t = rc_theme::palette(ui.ctx());
        let n = self.snap.trend.len();
        ui.horizontal(|ui| {
            let label_w = 170.0;
            let btn_w = 56.0;
            let sp = ui.spacing().item_spacing.x;
            ui.spacing_mut().slider_width =
                (ui.available_width() - label_w - btn_w - 3.0 * sp).max(60.0);
            let mut idx = self.sel.index.min(n.saturating_sub(1));
            let r = ui.add_enabled(
                n > 1,
                egui::Slider::new(&mut idx, 0..=n.saturating_sub(1))
                    .show_value(false)
                    .trailing_fill(true),
            );
            if r.changed() {
                self.sel.select(idx, n);
            }
            let text = if self.sel.following && !self.opts.viewer {
                "Live".to_string()
            } else if let Some(p) = self.snap.trend.get(idx) {
                format!(
                    "{:.2} min · {:.3} mL",
                    p.cum_time_ms / 60_000.0,
                    p.cum_vol_ml
                )
            } else {
                String::new()
            };
            let color = if self.sel.following {
                t.text_dim
            } else {
                t.warn
            };
            ui.add_sized(
                [label_w, 20.0],
                egui::Label::new(RichText::new(text).font(rc_theme::mono(12.0)).color(color))
                    .truncate(),
            );
            let btn = ui.add_sized(
                [btn_w, 22.0],
                egui::Button::new(if self.opts.viewer { "End" } else { "Live" }),
            );
            if btn.clicked() {
                self.sel.select_end(n);
            }
        });
    }

    // ── Run log: every event as a list below the slider ────────────────────────
    fn run_log(&mut self, ui: &mut egui::Ui) {
        // Lines in the log: fewer on short windows, so the plots keep their room.
        let h = ui.ctx().content_rect().height();
        let rows = (((h - 650.0) / 17.0).floor() as i64).clamp(4, 10) as usize;
        ui.add_space(4.0);
        let clicked =
            rc_panels::run_log(ui, &self.snap.events, self.xs(), &mut self.log_state, rows);
        let n = self.snap.trend.len();
        if let Some(e) = clicked.and_then(|i| self.snap.events.get(i))
            && n > 0
        {
            // the trend point at (or just after) the event's time
            let k =
                rc_plotdata::first_at_or_after(&self.snap.trend, e.cum_time_ms / 60_000.0, XS_TIME);
            self.sel.select(k.min(n - 1), n);
            ui.ctx().request_repaint();
        }
    }

    // ── Right column: cards for the run, the readings and the flow path ───────
    fn right_column(&mut self, ui: &mut egui::Ui) {
        let (idx, mode) = self.shown();
        let point = idx.and_then(|i| self.snap.trend.get(i));
        let (state, flow) = match (&self.shown, mode) {
            (Some(s), ShownMode::Live) => (&self.snap.inst, s.flow),
            (Some(s), _) if idx.is_some() => (&s.state, s.flow),
            _ => (&self.snap.inst, None),
        };
        let edit_label = self.edit_run_info.then_some(if self.opts.viewer {
            "Edit run info"
        } else {
            "Edit"
        });
        let d = PanelData {
            state,
            point,
            flow,
            info: &self.derived.info,
            mode,
            viewer: self.opts.viewer,
            edit_label,
        };
        let clicked = egui::ScrollArea::vertical()
            .id_salt((self.id, "side"))
            .auto_shrink([false, false])
            .show(ui, |ui| rc_panels::side_panel(ui, &d))
            .inner;
        self.edit_requested |= clicked;
    }

    // ── Status bar: connection · recording · mouse help ────────────────────────
    fn status_bar(&mut self, ui: &mut egui::Ui, window_w: f32) {
        let t = rc_theme::palette(ui.ctx());
        let status = self.status_override.as_deref().unwrap_or(&self.snap.status);
        let info = StatusInfo {
            status,
            recording: !self.opts.viewer && self.snap.recording,
            log_filename: &self.snap.log_file,
            viewer: self.opts.viewer,
            n_points: self.snap.trend.len(),
            run_time_ms: self.snap.trend.last().map_or(0.0, |p| p.cum_time_ms),
            help: if window_w >= HELP_MIN_WIDTH { HELP } else { "" },
        };
        egui::Frame::new()
            .fill(t.card)
            .stroke(Stroke::new(1.0, t.card_border))
            .corner_radius(t.radius_sm as u8)
            .inner_margin(Margin::symmetric(8, 5))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                rc_panels::status_bar(ui, &info);
            });
    }
}

/// Trace colour of a channel in the active palette.
fn channel_color(t: &rc_theme::Palette, ch: Channel) -> egui::Color32 {
    match ch {
        Channel::Uv => t.uv,
        Channel::Pctb => t.pctb,
        Channel::Cond => t.cond,
        Channel::Ph => t.ph,
        Channel::Pres => t.pres,
        Channel::Flow => t.flow,
    }
}

/// A legend entry: a line swatch and "UV   (0.012–1.22 AU)" in the trace colour.
fn legend_button(
    ui: &mut egui::Ui,
    t: &rc_theme::Palette,
    ch: Channel,
    label: &str,
    on: bool,
) -> egui::Response {
    let col = channel_color(t, ch);
    let font = if on {
        rc_theme::semibold(12.0)
    } else {
        rc_theme::medium(12.0)
    };
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, col);
    let sw = 16.0;
    let size = vec2(
        8.0 + sw + 6.0 + galley.size().x + 8.0,
        galley.size().y + 8.0,
    );
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let fill = if on {
            t.accent_soft
        } else if resp.hovered() {
            t.hover
        } else {
            t.card
        };
        p.rect(
            rect,
            4.0,
            fill,
            Stroke::new(1.0, col.gamma_multiply(0.6)),
            egui::StrokeKind::Inside,
        );
        let y = rect.center().y;
        p.line_segment(
            [pos2(rect.left() + 8.0, y), pos2(rect.left() + 8.0 + sw, y)],
            Stroke::new(if on { 3.0 } else { 2.0 }, col),
        );
        p.galley(
            pos2(rect.left() + 8.0 + sw + 6.0, y - galley.size().y / 2.0),
            galley,
            col,
        );
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_follows_until_a_past_point_is_picked() {
        let mut s = Selection {
            following: true,
            index: 0,
        };
        s.on_data(10);
        assert_eq!(
            s,
            Selection {
                following: true,
                index: 9
            }
        );
        s.select(3, 10);
        assert!(!s.following);
        s.on_data(20);
        assert_eq!(s.index, 3, "a picked moment stays put as data arrives");
        s.select(19, 20);
        assert!(s.following, "the right end follows again");
        s.on_data(25);
        assert_eq!(s.index, 24);
        // a new run (fewer points) clamps and follows
        s.select(5, 25);
        s.on_data(3);
        assert_eq!(
            s,
            Selection {
                following: true,
                index: 2
            }
        );
        s.select_end(0);
        assert_eq!(s.index, 0);
    }

    #[test]
    fn start_panels_and_toggles() {
        let app = rc_core::AppState::new().shared();
        let mut v = RunView::new(
            app,
            RunViewOptions {
                viewer: true,
                start_panels: vec!["%B".into(), "Flow".into()],
            },
        );
        assert_eq!(
            v.panes(),
            [
                Pane::Chan(Channel::Uv),
                Pane::Chan(Channel::Pctb),
                Pane::Chan(Channel::Flow)
            ]
        );
        v.set_panel("Flow", false);
        v.set_panel("pH", true);
        assert_eq!(
            v.panes(),
            [
                Pane::Chan(Channel::Uv),
                Pane::Chan(Channel::Pctb),
                Pane::Chan(Channel::Ph)
            ]
        );
        v.set_overlay(true);
        assert_eq!(v.panes(), [Pane::Overlay]);
        assert_eq!(
            v.overlay_channels(),
            [Channel::Uv, Channel::Pctb, Channel::Ph]
        );
    }

    #[test]
    fn column_volumes_fall_back_to_ml() {
        let mut app = rc_core::AppState::new();
        app.push_trend(rc_core::TrendPoint {
            cum_time_ms: 60_000.0,
            cum_vol_ml: 10.0,
            ..Default::default()
        });
        let shared = app.shared();
        let mut v = RunView::new(
            shared.clone(),
            RunViewOptions {
                viewer: true,
                start_panels: vec![],
            },
        );
        v.set_x_mode(XMode::ColumnVolumes);
        v.sync();
        v.derive(&rc_theme::Palette::studio_dark());
        assert_eq!(v.xs(), XS_VOLUME);
        assert!(v.xlabel().contains("no column volume"));
        shared
            .lock()
            .push_event(EventKind::Info, "Column volume (mL): 5.0");
        v.sync();
        v.derive(&rc_theme::Palette::studio_dark());
        assert_eq!(v.xs(), 5.0);
        assert_eq!(v.xlabel(), "Column volumes (CV)");
        // run info added later (viewer edit): the latest value counts
        shared
            .lock()
            .push_event(EventKind::Info, "Column volume (mL): 2.5");
        v.sync();
        v.derive(&rc_theme::Palette::studio_dark());
        assert_eq!(v.xs(), 2.5);
        assert!(!v.take_edit_request());
    }

    #[test]
    fn event_display_filters_lines_and_labels() {
        let mut app = rc_core::AppState::new();
        app.push_trend(rc_core::TrendPoint {
            cum_time_ms: 60_000.0,
            cum_vol_ml: 1.0,
            ..Default::default()
        });
        app.push_event(EventKind::Step, "Step t=60.0s  v=0.9990mL");
        app.push_event(EventKind::Log, "Break point 2");
        app.push_event(EventKind::Log, "Flow 20.0 ml/min");
        app.push_event(EventKind::Mark, "peak");
        let mut v = RunView::new(
            app.shared(),
            RunViewOptions {
                viewer: true,
                start_panels: vec![],
            },
        );
        v.sync();
        let pal = rc_theme::Palette::studio_light();
        let counts = |v: &mut RunView| {
            v.derive(&pal);
            (v.derived.lines.len(), v.derived.label_text.clone())
        };
        assert_eq!(
            counts(&mut v),
            (
                4,
                vec![
                    "STEP".into(),
                    "Break point 2".into(),
                    "Flow 20.0".into(),
                    "Mark: peak".into()
                ]
            )
        );
        v.set_event_display(EventDisplay::NoSteps);
        assert_eq!(
            counts(&mut v),
            (2, vec!["Flow 20.0".to_string(), "Mark: peak".to_string()])
        );
        v.set_event_display(EventDisplay::None);
        assert_eq!(counts(&mut v), (0, vec![]));
        v.set_event_display(EventDisplay::All);
        assert_eq!(counts(&mut v).0, 4);
    }

    /// Whole frames, drawn headless: stacked panels and the overlay with
    /// fractions and event labels, the event display changing between frames.
    #[test]
    fn frames_draw_fractions_and_labels_in_every_view() {
        let mut app = rc_core::AppState::new();
        for i in 0..=40 {
            let v = i as f64 * 0.5;
            app.inst.cum_time_ms = v * 60_000.0;
            app.inst.cum_vol_ml = v;
            app.push_trend(rc_core::TrendPoint {
                cum_time_ms: v * 60_000.0,
                cum_vol_ml: v,
                pctb: v * 5.0,
                ..Default::default()
            });
            match i {
                2 => app.push_event(EventKind::Step, "Step t=60.0s  v=1.0000mL"),
                4 => {
                    app.inst.fraction_vol_ml = 2.0;
                    app.update_fraction_state();
                    app.push_event(EventKind::Log, "Fraction size 2.0 ml");
                }
                8 | 12 | 16 => {
                    let n = app.feed_next_tube();
                    app.push_event(EventKind::Tubefeed, format!("Feed Tube Nr {n}"));
                }
                18 => app.push_event(EventKind::Step, "Step t=540.0s  v=9.0000mL"),
                _ => {}
            }
        }
        assert_eq!(app.fractions.len(), 4);
        let mut v = RunView::new(
            app.shared(),
            RunViewOptions {
                viewer: true,
                start_panels: rc_plotdata::Channel::ALL
                    .iter()
                    .map(|c| c.name().to_string())
                    .collect(),
            },
        );
        let ctx = egui::Context::default();
        let frame = |v: &mut RunView| {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1400.0, 900.0))),
                ..Default::default()
            };
            ctx.run_ui(input, |ui| v.ui(ui))
                .drop_without_applying_deltas();
        };
        for overlay in [false, true] {
            v.overlay = overlay;
            for d in [
                EventDisplay::All,
                EventDisplay::NoSteps,
                EventDisplay::None,
                EventDisplay::All,
            ] {
                let label_gen = v.derived.label_gen;
                let before = v.derived.label_x.len();
                v.set_event_display(d);
                frame(&mut v);
                frame(&mut v);
                // a new label set always comes with a new layout key
                if v.derived.label_x.len() != before {
                    assert_ne!(v.derived.label_gen, label_gen);
                }
                // a layout made for the current labels has one anchor per label
                if v.labels.key.map(|k| k[0]) == Some(v.derived.label_gen) {
                    assert_eq!(v.labels.pos.len(), v.derived.label_x.len());
                }
            }
        }
        assert_eq!(v.derived.spans.len(), 4);
        // the Fractions switch: a frame drawn without them
        assert!(v.shows_fractions());
        v.set_show_fractions(false);
        frame(&mut v);
        assert!(!v.shows_fractions());
    }

    #[test]
    fn marks_go_into_the_run() {
        let app = rc_core::AppState::new().shared();
        let mut v = RunView::new(
            app.clone(),
            RunViewOptions {
                viewer: false,
                start_panels: vec![],
            },
        );
        v.mark_note = "  peak 1 ".into();
        v.add_mark();
        v.add_mark();
        let a = app.lock();
        let texts: Vec<_> = a.events.iter().map(|e| (e.kind, e.text.as_str())).collect();
        assert_eq!(
            texts,
            [(EventKind::Mark, "peak 1"), (EventKind::Mark, "Mark")]
        );
        assert!(v.mark_note.is_empty());
    }
}
