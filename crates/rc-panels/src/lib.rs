//! Run-view side panels: the right column (instrument state, flow-path
//! schematic, run info), the run log and the status bar.
//!
//! Everything reads `rc_theme::palette`, so light and dark mode differ only
//! in rc-theme. Numbers are set in JetBrains Mono.

use std::collections::BTreeMap;
use std::time::Duration;

use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{
    Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Layout, Margin, Pos2, Rect, RichText,
    Sense, Shape, Stroke, StrokeKind, UiBuilder, pos2, vec2,
};
use rc_core::{AktaEvent, EventKind, InjectValue, InstrumentState, TrendPoint};
use rc_plotdata::XScale;
use rc_theme::{FS_BODY, FS_READING, FS_SMALL, FS_TITLE, Palette, medium, mono, semibold};

/// Which moment the panels show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShownMode {
    /// following the live instrument
    Live,
    /// a past moment picked with the time slider / inspector
    Past,
    /// an opened log: its end, until a moment is picked
    EndOfLog,
}

/// Everything the side panel shows.
#[derive(Clone, Copy)]
pub struct PanelData<'a> {
    /// instrument state: live, or rebuilt from the log at the shown moment
    pub state: &'a InstrumentState,
    /// trend point at the shown moment (None before the first point)
    pub point: Option<&'a TrendPoint>,
    /// set flow in force at the shown moment (mL/min)
    pub flow: Option<f64>,
    /// run information recorded in this log (Sample, Column, Port N names…)
    pub info: &'a BTreeMap<String, String>,
    pub mode: ShownMode,
    /// true for an opened (read-only) run
    pub viewer: bool,
    /// a button on the Run info card to edit it ("Edit"), which also shows the
    /// card when nothing is recorded yet; None: no button
    pub edit_label: Option<&'a str>,
}

/// Space between the cards of the right column.
const CARD_GAP: f32 = 10.0;

/// The right column: run card, run info card, readings card, flow path card.
/// Returns true when the Run info card's edit button was clicked.
pub fn side_panel(ui: &mut egui::Ui, d: &PanelData) -> bool {
    run_card(ui, d);
    ui.add_space(CARD_GAP - ui.spacing().item_spacing.y);
    let mut edit = false;
    if has_run_info(d.info) || d.edit_label.is_some() {
        edit = runinfo_card(ui, d);
        ui.add_space(CARD_GAP - ui.spacing().item_spacing.y);
    }
    readings_card(ui, d);
    ui.add_space(CARD_GAP - ui.spacing().item_spacing.y);
    flow_path_card(ui, d);
    edit
}

// ── Small helpers ────────────────────────────────────────────────────────────

/// A number with `prec` decimals, or "—" when missing or not finite.
pub fn fmt_value(v: Option<f64>, prec: usize) -> String {
    match v {
        Some(x) if x.is_finite() => format!("{x:.prec$}"),
        _ => "—".to_string(),
    }
}

fn text_format(font_id: FontId, color: Color32) -> TextFormat {
    TextFormat {
        font_id,
        color,
        valign: Align::BOTTOM,
        ..Default::default()
    }
}

/// A number in mono followed by its unit in small muted text ("12.34 min").
fn value_job(value: &str, unit: &str, size: f32, t: &Palette) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.append(value, 0.0, text_format(mono(size), t.text));
    if !unit.is_empty() && value != "—" {
        job.append(unit, 3.0, text_format(medium(FS_SMALL), t.text_dim));
    }
    job
}

/// A soft chip with an optional leading dot (live, connection, REC):
/// the colour as text on a tint of it. `dot` = the dot's opacity (blink).
pub fn dot_chip(ui: &mut egui::Ui, text: &str, color: Color32, dot: Option<f32>) -> egui::Response {
    let t = rc_theme::palette(ui.ctx());
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), semibold(FS_SMALL), color);
    let (pad, r, gap) = (8.0, 3.0, 5.0);
    let dot_w = if dot.is_some() { 2.0 * r + gap } else { 0.0 };
    let size = vec2(
        2.0 * pad + dot_w + galley.size().x,
        (galley.size().y + 5.0).max(18.0),
    );
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::same(9), t.tint(color, 0.18));
        let mut x = rect.left() + pad;
        if let Some(a) = dot {
            p.circle_filled(
                pos2(x + r, rect.center().y),
                r,
                color.gamma_multiply(a.clamp(0.0, 1.0)),
            );
            x += dot_w;
        }
        p.galley(
            pos2(x, rect.center().y - galley.size().y / 2.0),
            galley,
            color,
        );
    }
    resp
}

/// A key/value line (run card, run info): muted key in a fixed column, the
/// value wrapped to at most `max_rows` lines (full text on hover when cut).
fn kv_row(ui: &mut egui::Ui, key: &str, value: &str, value_font: FontId, max_rows: usize) {
    let t = rc_theme::palette(ui.ctx());
    const KEY_W: f32 = 92.0;
    let w = ui.available_width();
    let mut job =
        LayoutJob::single_section(value.to_owned(), text_format(value_font.clone(), t.text));
    job.wrap = TextWrapping {
        max_width: (w - KEY_W).max(24.0),
        max_rows,
        break_anywhere: false,
        overflow_character: Some('…'),
    };
    let galley = ui.painter().layout_job(job);
    let line_h = ui.painter().fonts_mut(|f| f.row_height(&value_font));
    let (rect, resp) = ui.allocate_exact_size(vec2(w, galley.size().y.max(line_h)), Sense::hover());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        p.text(
            pos2(rect.left(), rect.top() + line_h / 2.0),
            Align2::LEFT_CENTER,
            key,
            medium(FS_SMALL),
            t.text_dim,
        );
        let elided = galley.elided;
        p.galley(pos2(rect.left() + KEY_W, rect.top()), galley, t.text);
        if elided {
            resp.on_hover_text(value);
        }
    }
}

// ── Run card ─────────────────────────────────────────────────────────────────

/// Chip text and colour for what is shown: LIVE / VIEWING / END OF LOG / NO DATA.
fn mode_chip(t: &Palette, d: &PanelData) -> (&'static str, Color32, bool) {
    match (d.mode, d.point) {
        (ShownMode::Live, _) => ("LIVE", t.ok, true),
        (_, None) => ("NO DATA", t.text_faint, false),
        (ShownMode::Past, _) => ("VIEWING", t.warn, false),
        (ShownMode::EndOfLog, _) => ("END OF LOG", t.text_dim, false),
    }
}

/// Run: what is shown (live / a past moment / end of log), state and position.
pub fn run_card(ui: &mut egui::Ui, d: &PanelData) {
    let t = rc_theme::palette(ui.ctx());
    let s = d.state;
    rc_theme::card(ui, "Run", |ui| {
        ui.horizontal(|ui| {
            let (txt, col, dot) = mode_chip(&t, d);
            dot_chip(ui, txt, col, dot.then_some(1.0));
            // The run state means nothing before the first data point (the default "---").
            let state = if d.point.is_some() || (d.mode == ShownMode::Live && s.run_state != "---")
            {
                s.run_state.as_str()
            } else {
                ""
            };
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(state)
                        .font(semibold(FS_BODY + 0.5))
                        .color(t.text),
                );
            });
        });
        ui.add_space(4.0);
        // Fixed columns, so changing numbers never shift their neighbours.
        let stats = [
            (
                "Run time",
                fmt_value(Some(s.run_time_ms / 60_000.0), 2),
                "min",
            ),
            ("Run volume", fmt_value(Some(s.run_vol_ml), 3), "mL"),
            ("Set flow", fmt_value(d.flow, 2), "mL/min"),
        ];
        ui.columns_const::<3, _>(|cols| {
            for (ui, (name, value, unit)) in cols.iter_mut().zip(stats) {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(RichText::new(name).font(medium(FS_SMALL)).color(t.text_dim));
                ui.label(value_job(&value, unit, 15.0, &t));
            }
        });
        ui.add_space(4.0);
        ui.spacing_mut().item_spacing.y = 3.0;
        // The step position is only reported live; a past moment rebuilt from the log has none.
        let step = if d.mode == ShownMode::Live {
            format!(
                "{:.2} min · {:.3} mL",
                s.step_time_ms / 60_000.0,
                s.step_vol_ml
            )
        } else {
            "—".to_string()
        };
        kv_row(ui, "Step", &step, mono(11.5), 1);
        let prog = [s.prog_name.trim(), s.version.trim()]
            .into_iter()
            .filter(|x| !x.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        kv_row(
            ui,
            "Programme",
            if prog.is_empty() { "—" } else { &prog },
            FontId::proportional(FS_SMALL + 1.0),
            1,
        );
    });
}

// ── Run info card ────────────────────────────────────────────────────────────

fn has_run_info(info: &BTreeMap<String, String>) -> bool {
    info.values().any(|v| !v.trim().is_empty())
}

/// The run information as (label, value) lines: sample and user, the PC
/// method, the other Run info fields in their usual order, then anything else.
pub fn run_info_lines(info: &BTreeMap<String, String>) -> Vec<(String, String)> {
    let fields = rc_core::runinfo::RUN_INFO_FIELDS;
    let known: Vec<&str> = fields[..2]
        .iter()
        .copied()
        .chain(std::iter::once(rc_core::runinfo::METHOD_REF_KEY))
        .chain(fields[2..].iter().copied())
        .collect();
    let line = |key: &str, value: &str| -> (String, String) {
        match key {
            "Column volume (mL)" => ("Column volume".to_string(), format!("{} mL", value.trim())),
            _ => (key.to_string(), value.trim().to_string()),
        }
    };
    let mut out: Vec<(String, String)> = known
        .iter()
        .filter_map(|k| {
            info.get(*k)
                .filter(|v| !v.trim().is_empty())
                .map(|v| line(k, v))
        })
        .collect();
    out.extend(
        info.iter()
            .filter(|(k, v)| !known.contains(&k.as_str()) && !v.trim().is_empty())
            .map(|(k, v)| line(k, v)),
    );
    out
}

/// Run documentation (Run info), as recorded in this log, with an edit button
/// when `d.edit_label` is set. Nothing is drawn for a log without any and no
/// button. Returns true when the button was clicked.
pub fn runinfo_card(ui: &mut egui::Ui, d: &PanelData) -> bool {
    let has_info = has_run_info(d.info);
    if !has_info && d.edit_label.is_none() {
        return false;
    }
    let t = rc_theme::palette(ui.ctx());
    let mut clicked = false;
    egui::Frame::new()
        .fill(t.card)
        .stroke(Stroke::new(1.0, t.card_border))
        .corner_radius(CornerRadius::same(t.radius as u8))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // the title row of rc_theme::card, with the button at the right
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("RUN INFO")
                        .font(semibold(FS_TITLE))
                        .color(t.text_faint),
                );
                if let Some(label) = d.edit_label {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let button = egui::Button::new(
                            RichText::new(label)
                                .font(medium(FS_SMALL))
                                .color(t.accent_text),
                        )
                        .frame_when_inactive(false)
                        .corner_radius(CornerRadius::same(t.radius_sm as u8))
                        .min_size(vec2(0.0, 20.0));
                        let tip = if d.viewer {
                            "Add or change run information: appended to the end of this log"
                        } else {
                            "Sample, user and notes of the current run"
                        };
                        clicked = ui.add(button).on_hover_text(tip).clicked();
                    });
                }
            });
            ui.add_space(2.0);
            ui.spacing_mut().item_spacing.y = 3.0;
            if !has_info {
                ui.label(
                    RichText::new("Nothing recorded for this run yet.")
                        .font(FontId::proportional(FS_SMALL + 1.0))
                        .color(t.text_faint),
                );
            }
            for (key, value) in run_info_lines(d.info) {
                let rows = if rc_core::runinfo::is_multiline_field(&key) {
                    3
                } else {
                    2
                };
                kv_row(ui, &key, &value, FontId::proportional(FS_SMALL + 1.0), rows);
            }
        });
    clicked
}

// ── Readings card ────────────────────────────────────────────────────────────

/// Readings: big numbers, named in their trace colour.
pub fn readings_card(ui: &mut egui::Ui, d: &PanelData) {
    let t = rc_theme::palette(ui.ctx());
    let p = d.point;
    let get = |f: fn(&TrendPoint) -> f64| p.map(f);
    let items = [
        ("UV", "AU", get(|p| p.uv), 4, t.uv),
        ("%B", "%", get(|p| p.pctb), 1, t.pctb),
        ("Cond", "mS/cm", get(|p| p.cond_mscm), 2, t.cond),
        ("pH", "", get(|p| p.ph), 2, t.ph),
        ("Pressure", "MPa", get(|p| p.pres_mpa), 3, t.pres),
        // not plotted, so not in a trace colour
        ("Temp", "°C", get(|p| p.temp_degc), 1, t.text_dim),
    ];
    rc_theme::card(ui, "Readings", |ui| {
        for (row, chunk) in items.chunks(3).enumerate() {
            if row > 0 {
                ui.add_space(4.0);
            }
            ui.columns_const::<3, _>(|cols| {
                for (ui, &(name, unit, value, prec, color)) in cols.iter_mut().zip(chunk) {
                    let label = if unit.is_empty() {
                        name.to_string()
                    } else {
                        format!("{name} · {unit}")
                    };
                    reading(ui, &label, &fmt_value(value, prec), color);
                }
            });
        }
    });
}

/// One reading: name in its colour, the value in large mono, shrunk to fit a narrow column.
fn reading(ui: &mut egui::Ui, name: &str, value: &str, color: Color32) {
    let t = rc_theme::palette(ui.ctx());
    // JetBrains Mono advances 0.6 em; leave room for 7 characters ("-0.0012").
    let size = FS_READING.min(ui.available_width() / (0.6 * 7.0)).max(13.0);
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.label(RichText::new(name).font(medium(FS_SMALL)).color(color));
    ui.label(RichText::new(value).font(mono(size)).color(t.text));
}

// ── Flow path card ───────────────────────────────────────────────────────────

/// Flow path: one-line valve summary above the schematic.
pub fn flow_path_card(ui: &mut egui::Ui, d: &PanelData) {
    let t = rc_theme::palette(ui.ctx());
    rc_theme::card(ui, "Flow path", |ui| {
        ui.label(
            RichText::new(rc_plotdata::valve_summary(d.state, d.info))
                .font(FontId::proportional(FS_SMALL + 1.0))
                .color(t.text_dim),
        );
        ui.add_space(4.0);
        flow_schematic(ui, d.state, d.point, d.info);
    });
}

/// Data-space window of the schematic: the drawing's extent, without empty margins.
const SX0: f32 = 0.3;
const SX1: f32 = 8.1;
const SY0: f32 = 1.8;
const SY1: f32 = 13.5;

/// The flow path: buffer valve ports, Buffer A / B, mixer, injection valve,
/// column, UV / Cond, outlet valve, waste and fraction collector, with the
/// active pipes drawn in colour (port of `_draw_flow_schematic!`).
///
/// Takes the available width; the height follows the width (within the
/// available height when that is limited). Hovering a port or node shows
/// what the run information says about it.
pub fn flow_schematic(
    ui: &mut egui::Ui,
    s: &InstrumentState,
    point: Option<&TrendPoint>,
    info: &BTreeMap<String, String>,
) -> egui::Response {
    let t = rc_theme::palette(ui.ctx());
    let w = ui.available_width().max(180.0);
    let avail_h = ui.available_height();
    let h = if avail_h.is_finite() && avail_h > 0.0 {
        avail_h.clamp(0.78 * w, 1.12 * w)
    } else {
        w
    };
    let (resp, painter) = ui.allocate_painter(vec2(w, h), Sense::hover());
    let r = resp.rect;
    if !ui.is_rect_visible(r) {
        return resp;
    }
    let xu = r.width() / (SX1 - SX0);
    let yu = r.height() / (SY1 - SY0);
    let at = |x: f32, y: f32| pos2(r.left() + (x - SX0) * xu, r.top() + (SY1 - y) * yu);
    // 10 px labels, growing a little on wide panels.
    let k = (w / 420.0).clamp(1.0, 1.2);
    let f_label = semibold(10.0 * k);
    let f_mixer = semibold(11.0 * k);
    let f_note = FontId::proportional(9.0 * k);
    let f_note_bold = semibold(9.0 * k);
    let lw_main = 3.2 * k;
    let lw_thin = 2.6 * k;
    let lw_idle = 1.4 * k;
    let tint = |c: Color32, a: f32| t.tint(c, a);
    let muted = t.text_dim;
    let border = t.card_border;

    // ── Node coordinates (staggered vertically for readability) ──
    let (x_l, x_c, x_r) = (1.3_f32, 4.2_f32, 7.1_f32);
    let x_frac = x_r;
    let y_buf = 11.3;
    let y_mix = 10.3;
    let y_syr = 9.4;
    let y_inj = 8.5;
    let y_loop = 7.6;
    let y_waste_inj = 7.6;
    let y_col = 6.6;
    let y_waste_loop = 5.8;
    let y_uv = 5.0;
    let y_out = 3.6;
    let y_bottom = 2.4;

    // ── Buffer valve: 8 inlet ports in a row above Buffer A, feeding the A side ──
    // Port 1 (the standard buffer A) sits straight above Buffer A; the active
    // port is piped down to a short manifold line, then into Buffer A.
    const N_PORTS: usize = 8;
    let x_vlv = x_l;
    let y_port = 12.45;
    let y_manifold = 12.0;
    let port_x = |i: usize| x_vlv + 0.62 * (i as f32 - 1.0);
    let port_color = |i: usize| match i {
        1 => t.buf_a,
        N_PORTS => t.sample,
        _ => t.buf_port,
    };

    let path_known = s.inject_value != InjectValue::Unknown;
    let is_load = s.inject_value == InjectValue::Load;
    let is_inj = s.inject_value == InjectValue::Inject;
    let is_wst = s.inject_value == InjectValue::Waste;
    let to_frac = s.fraction_vol_ml > 0.0;
    let port = usize::try_from(s.buffer_port)
        .ok()
        .filter(|p| (1..=N_PORTS).contains(p));
    let pcol = port_color(port.unwrap_or(1));
    // The buffer valve is only reported when it moves: an assumed port is drawn dimmed.
    let alpha = if s.buffer_port_known { 1.0 } else { 0.45 };

    // ── Pipes: drawn in colour only while liquid flows through them; idle ones faint ──
    struct Pipe {
        pts: Vec<Pos2>,
        color: Color32,
        width: f32,
        active: bool,
    }
    let mut pipes: Vec<Pipe> = Vec::new();
    let mut pipe = |pts: &[(f32, f32)], color: Color32, width: f32, active: bool| {
        pipes.push(Pipe {
            pts: pts.iter().map(|&(x, y)| at(x, y)).collect(),
            color,
            width,
            active,
        });
    };
    for i in 1..=N_PORTS {
        let active = path_known && Some(i) == port;
        let color = tint(port_color(i), alpha);
        if i == 1 {
            pipe(
                &[(x_vlv, y_port - 0.24), (x_vlv, y_buf + 0.31)],
                color,
                lw_thin,
                active,
            );
        } else {
            let px = port_x(i);
            pipe(
                &[
                    (px, y_port - 0.24),
                    (px, y_manifold),
                    (x_vlv, y_manifold),
                    (x_vlv, y_buf + 0.31),
                ],
                color,
                lw_thin,
                active,
            );
        }
    }
    pipe(
        &[(x_vlv, y_buf - 0.31), (x_c, y_mix + 0.02)],
        tint(pcol, alpha),
        lw_main,
        path_known,
    );
    pipe(
        &[(x_frac, 11.0), (x_c, y_mix + 0.02)],
        t.buf_b,
        lw_main,
        path_known,
    );
    pipe(
        &[(x_c, y_mix - 0.38), (x_c, y_inj + 0.42)],
        t.pipe,
        lw_main,
        path_known,
    );
    // switchable paths (inject valve, syringe, loop)
    pipe(
        &[(x_c, y_inj - 0.42), (x_c, y_col + 0.42)],
        t.pipe,
        lw_main,
        path_known && is_load,
    );
    pipe(
        &[(x_c, y_inj), (x_r, y_loop)],
        t.pipe,
        lw_main,
        path_known && is_inj,
    );
    pipe(
        &[(x_r, y_loop), (x_c, y_col)],
        t.pipe,
        lw_main,
        path_known && is_inj,
    );
    pipe(
        &[(x_c, y_inj), (x_l, y_waste_inj)],
        t.pipe,
        lw_main,
        path_known && is_wst,
    );
    pipe(
        &[(x_r, y_syr - 0.34), (x_r, y_loop + 0.34)],
        t.pipe,
        lw_thin,
        path_known && is_load,
    );
    pipe(
        &[(x_r, y_loop - 0.34), (x_r, y_waste_loop + 0.34)],
        t.pipe,
        lw_thin,
        path_known && is_load,
    );
    let to_column = path_known && (is_load || is_inj);
    pipe(
        &[(x_c, y_col - 0.42), (x_c, y_uv + 0.42)],
        t.pipe,
        lw_main,
        to_column,
    );
    pipe(
        &[(x_c, y_uv - 0.42), (x_c, y_out + 0.40)],
        t.pipe,
        lw_main,
        to_column,
    );
    pipe(
        &[(x_c, y_out), (x_l, y_bottom)],
        muted,
        lw_main,
        path_known && !to_frac,
    );
    pipe(
        &[(x_c, y_out), (x_frac, y_bottom)],
        t.frac,
        lw_main,
        path_known && to_frac,
    );

    for p in pipes.iter().filter(|p| !p.active) {
        painter.add(Shape::line(p.pts.clone(), Stroke::new(lw_idle, t.faint)));
    }
    for p in pipes.iter().filter(|p| p.active) {
        painter.add(Shape::line(p.pts.clone(), Stroke::new(p.width, p.color)));
        // round ends
        for q in [p.pts[0], p.pts[p.pts.len() - 1]] {
            painter.circle_filled(q, p.width / 2.0, p.color);
        }
    }

    // ── Nodes ──
    let mut hover_zones: Vec<(Rect, String)> = Vec::new();
    let text_w = |s: &str, f: &FontId| {
        painter
            .layout_no_wrap(s.to_owned(), f.clone(), t.text)
            .size()
            .x
    };
    let box_h = (0.86 * yu).max(20.0 * k);
    // Boxes grow to fit their label with a margin.
    let node_rect = |cx: f32, cy: f32, w: f32, label: &str| {
        let wpx = (w * xu).max(text_w(label, &f_label) + 18.0 * k);
        Rect::from_center_size(at(cx, cy), vec2(wpx, box_h))
    };
    let radius = CornerRadius::same(t.radius_sm as u8);
    let node = |cx: f32, cy: f32, w: f32, label: &str, fill: Color32, stroke: Stroke| -> Rect {
        let rect = node_rect(cx, cy, w, label);
        painter.rect(rect, radius, fill, stroke, StrokeKind::Inside);
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            label,
            f_label.clone(),
            t.node_text,
        );
        rect
    };
    let diamond = |cx: f32, cy: f32, w: f32, h: f32, fill: Color32, stroke: Color32| {
        let c = at(cx, cy);
        let (hw, hh) = (w * xu / 2.0, h * yu / 2.0);
        let pts = vec![
            pos2(c.x, c.y - hh),
            pos2(c.x + hw, c.y),
            pos2(c.x, c.y + hh),
            pos2(c.x - hw, c.y),
        ];
        painter.add(Shape::convex_polygon(pts, fill, Stroke::new(1.5, stroke)));
        Rect::from_center_size(c, vec2(2.0 * hw, 2.0 * hh))
    };
    let info_of = |key: &str| info.get(key).map(|v| v.trim()).filter(|v| !v.is_empty());

    // The 8 ports: the active one filled with its colour (a tint while only assumed).
    let port_h = (0.48 * yu).max(15.0 * k);
    let port_w = (0.50 * xu).max(text_w("8", &f_label) + 8.0);
    for i in 1..=N_PORTS {
        let rect = Rect::from_center_size(at(port_x(i), y_port), vec2(port_w, port_h));
        let active = Some(i) == port;
        let c = port_color(i);
        let (fill, stroke, text) = if active {
            let text = if s.buffer_port_known {
                ink_on(c, &t)
            } else {
                t.node_text
            };
            (tint(c, alpha), c, text)
        } else {
            (t.node, border, t.node_text)
        };
        painter.rect(
            rect,
            CornerRadius::same(4),
            fill,
            Stroke::new(1.2, stroke),
            StrokeKind::Inside,
        );
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            i.to_string(),
            f_label.clone(),
            text,
        );
        let default = match i {
            1 => " (A)",
            N_PORTS => " (sample)",
            _ => "",
        };
        let mut tip = match info_of(&format!("Port {i}")) {
            Some(name) => format!("Port {i}: {name}"),
            None => format!("Port {i}{default}"),
        };
        if active {
            tip.push_str(if s.buffer_port_known {
                " — active"
            } else {
                " — assumed (not reported yet)"
            });
        }
        hover_zones.push((rect, tip));
    }
    let above = |i: usize| at(port_x(i), y_port) - vec2(0.0, port_h / 2.0 + 2.0);
    painter.text(above(1), Align2::CENTER_BOTTOM, "A", f_note.clone(), muted);
    painter.text(
        above(N_PORTS),
        Align2::CENTER_BOTTOM,
        "Sample",
        f_note.clone(),
        muted,
    );

    // The A side of the mixer: fed by the buffer valve, coloured by its active port.
    let rect = node(
        x_vlv,
        y_buf,
        1.3,
        "Buffer A",
        tint(pcol, 0.25 * alpha),
        Stroke::new(1.5, tint(pcol, alpha)),
    );
    hover_zones.push((rect, format!("Buffer valve port {}", s.buffer_port)));
    let rect = node(
        x_frac,
        y_buf,
        1.30,
        "Buffer B",
        tint(t.buf_b, 0.35),
        Stroke::new(1.2, border),
    );
    if let Some(b) = info_of("Buffer B") {
        hover_zones.push((rect, format!("Buffer B: {b}")));
    }

    // Injection valve: soft tint of its state colour, outlined in it.
    let inj_col = match s.inject_value {
        InjectValue::Unknown => t.err,
        InjectValue::Waste => muted,
        InjectValue::Inject => t.warn,
        InjectValue::Load => t.accent,
    };
    diamond(x_c, y_inj, 2.3, 1.15, tint(inj_col, 0.22), inj_col);
    painter.text(
        at(x_c, y_inj),
        Align2::CENTER_CENTER,
        s.inject_value.label(),
        f_label.clone(),
        t.node_text,
    );

    let plain = Stroke::new(1.2, border);
    let rect = node(x_c, y_col, 1.35, "Column", t.node, plain);
    if let Some(c) = info_of("Column") {
        hover_zones.push((rect, format!("Column: {c}")));
    }
    node(x_c, y_uv, 1.48, "UV / Cond", t.node, plain);
    node(x_r, y_syr, 1.45, "Syringe", t.node, plain);
    node(x_r, y_loop, 1.20, "Loop", t.node, plain);
    node(x_frac, y_bottom, 1.25, "Frac", tint(t.frac, 0.30), plain);
    node(x_l, y_bottom, 1.30, "Waste", t.node, plain);
    node(x_r, y_waste_loop, 1.30, "Waste", t.node, plain);
    node(x_l, y_waste_inj, 1.30, "Waste", t.node, plain);

    // Outlet valve: to the fraction collector or to waste.
    let out_col = if to_frac { t.frac } else { muted };
    let out_rect = diamond(x_c, y_out, 1.6, 1.0, tint(out_col, 0.22), out_col);
    painter.text(
        at(x_c, y_out),
        Align2::CENTER_CENTER,
        "Out",
        f_label.clone(),
        t.node_text,
    );
    let out_txt = if !path_known {
        "PATH UNKNOWN".to_string()
    } else if to_frac {
        format!("TO FRAC #{}", s.fraction_number)
    } else {
        "TO WASTE".to_string()
    };
    painter.text(
        pos2(out_rect.center().x, out_rect.bottom() + 3.0),
        Align2::CENTER_TOP,
        out_txt,
        f_note_bold,
        muted,
    );

    // Mixer: its colour blends the A side (active port) with buffer B by %B.
    let f = point.map_or(0.0, |p| (p.pctb / 100.0).clamp(0.0, 1.0)) as f32;
    let mix = Color32::from_rgb(
        lerp_u8(pcol.r(), t.buf_b.r(), f),
        lerp_u8(pcol.g(), t.buf_b.g(), f),
        lerp_u8(pcol.b(), t.buf_b.b(), f),
    );
    let mixer = Rect::from_center_size(
        at(x_c, y_mix),
        vec2((1.3 * xu).max(text_w("100 %B", &f_mixer) + 18.0 * k), box_h),
    );
    painter.rect(mixer, radius, tint(mix, 0.45), plain, StrokeKind::Inside);
    let mixer_label = point.map_or_else(|| "MIX".to_string(), |p| format!("{:.0} %B", p.pctb));
    painter.text(
        mixer.center(),
        Align2::CENTER_CENTER,
        mixer_label,
        f_mixer,
        t.node_text,
    );

    let tip = resp
        .hover_pos()
        .and_then(|pos| hover_zones.iter().find(|(rect, _)| rect.contains(pos)));
    match tip {
        Some((_, tip)) => {
            let tip = tip.clone();
            resp.on_hover_text_at_pointer(tip)
        }
        None => resp,
    }
}

fn lerp_u8(a: u8, b: u8, f: f32) -> u8 {
    ((1.0 - f) * a as f32 + f * b as f32).round() as u8
}

/// Text colour on a solid fill: white on dark or saturated colours, the
/// darkest surface on light ones (the bright dark-mode port colours).
fn ink_on(fill: Color32, t: &Palette) -> Color32 {
    let lum = 0.2126 * fill.r() as f32 + 0.7152 * fill.g() as f32 + 0.0722 * fill.b() as f32;
    if lum > 165.0 {
        if t.dark { t.dock } else { t.text }
    } else {
        Color32::WHITE
    }
}

// ── Run log ──────────────────────────────────────────────────────────────────

/// State of the run log list (filter text, scroll position…).
#[derive(Default)]
pub struct RunLogState {
    pub filter: String,
}

/// Height of one line of the run log.
pub const LOG_ROW_H: f32 = 19.0;

/// Kind name shown in the log: the CSV name without "EVT_".
pub fn kind_name(k: EventKind) -> &'static str {
    let n = k.csv_name();
    n.strip_prefix("EVT_").unwrap_or(n)
}

/// Does an event match the filter text (case-insensitive, text or kind)?
pub fn log_matches(e: &AktaEvent, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty()
        || e.text.to_lowercase().contains(&q)
        || e.kind.csv_name().to_lowercase().contains(&q)
}

/// Position of an event: the current x unit first, then the other one
/// (min ↔ mL; mL for column volumes).
pub fn log_position(e: &AktaEvent, xs: XScale) -> (String, String) {
    let min = format!("{:.2} min", e.cum_time_ms / 60_000.0);
    let ml = format!("{:.3} mL", e.cum_vol_ml);
    if xs == rc_plotdata::XS_TIME {
        (min, ml)
    } else if xs == rc_plotdata::XS_VOLUME {
        (ml, min)
    } else {
        (format!("{:.2} CV", rc_plotdata::event_x(e, xs)), ml)
    }
}

/// Colour of an event's kind and text: marks as their plot line, run information muted.
fn kind_colors(k: EventKind, t: &Palette) -> (Color32, Color32) {
    match k {
        EventKind::Mark => (t.mark, t.mark),
        EventKind::Info => (t.text_faint, t.text_dim),
        _ => (t.text_dim, t.text),
    }
}

/// Every event as a filterable list; returns the index (into `events`) of a
/// clicked line, so the run view can inspect that moment.
///
/// Newest at the bottom; while scrolled to the end the list follows new
/// events. Always `max_rows` lines tall, so the layout around it stays put.
pub fn run_log(
    ui: &mut egui::Ui,
    events: &[AktaEvent],
    xs: XScale,
    state: &mut RunLogState,
    max_rows: usize,
) -> Option<usize> {
    let t = rc_theme::palette(ui.ctx());
    let rows: Vec<usize> = (0..events.len())
        .filter(|&i| log_matches(&events[i], &state.filter))
        .collect();

    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut state.filter)
                .hint_text("Filter the run log…")
                .desired_width(260.0),
        );
        if !state.filter.is_empty()
            && ui
                .add(egui::Button::new(RichText::new("Clear").color(t.text_dim)).frame(false))
                .clicked()
        {
            state.filter.clear();
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let n = events.len();
            let count = if rows.len() == n {
                format!("{n} event{}", if n == 1 { "" } else { "s" })
            } else {
                format!("{} of {n} events", rows.len())
            };
            ui.label(
                RichText::new(count)
                    .font(FontId::proportional(FS_SMALL))
                    .color(t.text_faint),
            );
        });
    });

    let mut clicked = None;
    let max_rows = max_rows.max(1);
    let list_h = max_rows as f32 * LOG_ROW_H;
    egui::Frame::new()
        .fill(t.card)
        .stroke(Stroke::new(1.0, t.card_border))
        .corner_radius(CornerRadius::same(t.radius as u8))
        .inner_margin(Margin::symmetric(4, 4))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let cols = LogColumns::measure(ui, &t);
            let out = egui::ScrollArea::vertical()
                .id_salt("rc_panels_run_log")
                .auto_shrink([false, false])
                .min_scrolled_height(list_h)
                .max_height(list_h)
                .stick_to_bottom(true)
                .show_rows(ui, LOG_ROW_H, rows.len(), |ui, range| {
                    for &i in &rows[range] {
                        if log_row(ui, &events[i], xs, &cols, &t).clicked() {
                            clicked = Some(i);
                        }
                    }
                });
            if rows.is_empty() {
                let msg = if events.is_empty() {
                    "No events yet"
                } else {
                    "No event matches the filter"
                };
                ui.painter().text(
                    out.inner_rect.center(),
                    Align2::CENTER_CENTER,
                    msg,
                    FontId::proportional(FS_BODY),
                    t.text_faint,
                );
            }
        });
    clicked
}

/// Column widths of the run log, from the mono font.
struct LogColumns {
    pos1: f32,
    pos2: f32,
    kind: f32,
}

impl LogColumns {
    fn measure(ui: &egui::Ui, t: &Palette) -> LogColumns {
        let w = |s: &str, f: FontId| {
            ui.painter()
                .layout_no_wrap(s.to_owned(), f, t.text)
                .size()
                .x
        };
        LogColumns {
            pos1: w("0000.00 min", log_mono()),
            pos2: w("000.000 mL", log_mono()),
            kind: w("DISCONNECT", semibold(FS_TITLE)),
        }
    }
}

fn log_mono() -> FontId {
    mono(11.5)
}

fn log_row(
    ui: &mut egui::Ui,
    e: &AktaEvent,
    xs: XScale,
    cols: &LogColumns,
    t: &Palette,
) -> egui::Response {
    let (rect, resp) =
        ui.allocate_exact_size(vec2(ui.available_width(), LOG_ROW_H), Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let p = ui.painter_at(rect);
    if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(t.radius_sm as u8), t.hover);
    }
    let (kind_col, text_col) = kind_colors(e.kind, t);
    let (pos1, pos2_) = log_position(e, xs);
    let y = rect.center().y;
    let gap = 14.0;
    let mut x = rect.left() + 8.0;
    p.text(
        pos2(x + cols.pos1, y),
        Align2::RIGHT_CENTER,
        pos1,
        log_mono(),
        t.text_dim,
    );
    x += cols.pos1 + gap;
    p.text(
        pos2(x + cols.pos2, y),
        Align2::RIGHT_CENTER,
        pos2_,
        log_mono(),
        t.text_faint,
    );
    x += cols.pos2 + gap;
    p.text(
        pos2(x, y),
        Align2::LEFT_CENTER,
        kind_name(e.kind),
        semibold(FS_TITLE),
        kind_col,
    );
    x += cols.kind + gap;
    let mut job = LayoutJob::single_section(
        e.text.clone(),
        text_format(FontId::proportional(FS_BODY), text_col),
    );
    job.wrap = TextWrapping::truncate_at_width((rect.right() - 8.0 - x).max(10.0));
    let galley = p.layout_job(job);
    let elided = galley.elided;
    p.galley(pos2(x, y - galley.size().y / 2.0), galley, text_col);
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    if elided {
        resp.on_hover_text(&e.text)
    } else {
        resp
    }
}

// ── Status bar ───────────────────────────────────────────────────────────────

/// What the status bar shows.
pub struct StatusInfo<'a> {
    /// connection status text (AppState::status); "" for an opened run
    pub status: &'a str,
    /// a CSV log is open and has received data (REC chip)
    pub recording: bool,
    pub log_filename: &'a str,
    pub viewer: bool,
    pub n_points: usize,
    /// run time of the newest trend point (ms), shown while recording
    pub run_time_ms: f64,
    /// mouse help text (hidden when narrow)
    pub help: &'a str,
}

/// Height of the status bar row.
pub const STATUS_BAR_H: f32 = 24.0;
/// Blink period of the REC dot: it flips every 0.6 s.
const BLINK_S: f64 = 0.6;

/// The recording slot text: the run time while recording,
/// the number of points for an opened run.
pub fn recording_text(s: &StatusInfo) -> String {
    let file = std::path::Path::new(s.log_filename)
        .file_name()
        .map_or_else(
            || s.log_filename.to_string(),
            |f| f.to_string_lossy().into_owned(),
        );
    let points = format!(
        "{} point{}",
        s.n_points,
        if s.n_points == 1 { "" } else { "s" }
    );
    if s.viewer {
        format!("Viewing {file} · {points}")
    } else if !s.recording {
        "Not recording".to_string()
    } else {
        format!("{file} · {:.1} min", s.run_time_ms / 60_000.0)
    }
}

/// Connection · recording · log file · mouse help. Every item has a
/// fixed-width slot (chips sit at its left), so a changing text or the
/// blinking chip never moves the others.
///
/// Draws one row of `STATUS_BAR_H` and no frame: the caller places it (the
/// run view wraps it in a card).
pub fn status_bar(ui: &mut egui::Ui, s: &StatusInfo) {
    let t = rc_theme::palette(ui.ctx());
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), STATUS_BAR_H), Sense::hover());
    let clip = ui.clip_rect().intersect(row);
    const GAP: f32 = 10.0;
    let mut x = row.left();
    let mut slot = |width: f32| {
        let r = Rect::from_min_max(
            pos2(x, row.top()),
            pos2((x + width).min(row.right()), row.bottom()),
        );
        x += width + GAP;
        r
    };
    let in_slot = |ui: &mut egui::Ui, r: Rect, add: &mut dyn FnMut(&mut egui::Ui)| {
        if r.width() <= 0.0 {
            return;
        }
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(r)
                .layout(Layout::left_to_right(Align::Center)),
        );
        child.set_clip_rect(clip.intersect(r));
        add(&mut child);
    };

    if !s.status.is_empty() {
        // Connection chip: green connected, amber waiting, red port missing, blue demo.
        let col = t.status_color(s.status);
        let txt = s.status.replace("● ", "");
        in_slot(ui, slot(330.0), &mut |ui| {
            dot_chip(ui, &txt, col, Some(1.0));
        });
        let demo = s.status.starts_with("Demo");
        in_slot(ui, slot(56.0), &mut |ui| {
            if demo {
                rc_theme::soft_chip(ui, "DEMO", t.demo);
            }
        });
    }

    // REC chip with a blinking dot while writing a log; the text (and so its width) stays.
    let rec = s.recording && !s.viewer;
    in_slot(ui, slot(120.0), &mut |ui| {
        if rec {
            let now = ui.input(|i| i.time);
            let on = (now / BLINK_S).floor() as i64 % 2 == 0;
            ui.ctx().request_repaint_after(Duration::from_secs_f64(
                BLINK_S - now.rem_euclid(BLINK_S) + 0.005,
            ));
            dot_chip(ui, "REC", t.rec, Some(if on { 1.0 } else { 0.3 }));
        } else {
            rc_theme::chip(ui, "NOT RECORDING", t.field, t.text_faint);
        }
    });

    let label = recording_text(s);
    in_slot(ui, slot(380.0), &mut |ui| {
        ui.add(
            egui::Label::new(
                RichText::new(&label)
                    .font(FontId::proportional(FS_SMALL + 1.0))
                    .color(t.text_dim),
            )
            .truncate(),
        );
    });

    // Mouse help, right-aligned in what is left; hidden when it does not fit.
    if !s.help.is_empty() {
        let rest = Rect::from_min_max(pos2(x, row.top()), row.max);
        let g = ui.painter().layout_no_wrap(
            s.help.to_owned(),
            FontId::proportional(FS_SMALL),
            t.text_faint,
        );
        if g.size().x <= rest.width() {
            let pos = pos2(rest.right() - g.size().x, row.center().y - g.size().y / 2.0);
            ui.painter()
                .with_clip_rect(clip)
                .galley(pos, g, t.text_faint);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(kind: EventKind, text: &str, t_ms: f64, v: f64) -> AktaEvent {
        AktaEvent {
            cum_time_ms: t_ms,
            cum_vol_ml: v,
            kind,
            text: text.to_string(),
        }
    }

    #[test]
    fn values_format_or_dash() {
        assert_eq!(fmt_value(Some(1.23456), 2), "1.23");
        assert_eq!(fmt_value(None, 2), "—");
        assert_eq!(fmt_value(Some(f64::NAN), 2), "—");
    }

    #[test]
    fn filter_matches_text_and_kind() {
        let e = ev(EventKind::Mark, "Peak 1", 0.0, 0.0);
        assert!(log_matches(&e, ""));
        assert!(log_matches(&e, "  peak "));
        assert!(log_matches(&e, "mark"));
        assert!(log_matches(&e, "evt_m"));
        assert!(!log_matches(&e, "flow"));
    }

    #[test]
    fn kind_names_drop_prefix() {
        assert_eq!(kind_name(EventKind::RunState), "RUN_STATE");
        assert_eq!(kind_name(EventKind::Info), "INFO");
    }

    #[test]
    fn positions_follow_x_units() {
        let e = ev(EventKind::Log, "Flow 1.0 ml/min", 90_000.0, 3.0);
        assert_eq!(
            log_position(&e, 0.0),
            ("1.50 min".to_string(), "3.000 mL".to_string())
        );
        assert_eq!(
            log_position(&e, 1.0),
            ("3.000 mL".to_string(), "1.50 min".to_string())
        );
        assert_eq!(
            log_position(&e, 2.0),
            ("1.50 CV".to_string(), "3.000 mL".to_string())
        );
    }

    #[test]
    fn recording_slot_text() {
        let mut s = StatusInfo {
            status: "",
            recording: false,
            log_filename: "/data/logs/akta_20261020.csv",
            viewer: true,
            n_points: 88,
            run_time_ms: 0.0,
            help: "",
        };
        assert_eq!(recording_text(&s), "Viewing akta_20261020.csv · 88 points");
        s.viewer = false;
        assert_eq!(recording_text(&s), "Not recording");
        s.recording = true;
        assert_eq!(recording_text(&s), "akta_20261020.csv · 0.0 min");
        s.run_time_ms = 754_000.0;
        assert_eq!(recording_text(&s), "akta_20261020.csv · 12.6 min");
        s.viewer = true;
        s.n_points = 1;
        assert_eq!(recording_text(&s), "Viewing akta_20261020.csv · 1 point");
    }

    #[test]
    fn run_info_in_field_order() {
        let mut info = BTreeMap::new();
        info.insert("Port 3".to_string(), "NaCl 1 M".to_string());
        info.insert("Sample".to_string(), "Lysate".to_string());
        info.insert("Column volume (mL)".to_string(), "5.0".to_string());
        info.insert("Extra".to_string(), "x".to_string());
        info.insert("User".to_string(), "  ".to_string());
        info.insert("PC method".to_string(), "005 – Ni".to_string());
        let lines = run_info_lines(&info);
        let keys: Vec<&str> = lines.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            keys,
            ["Sample", "PC method", "Column volume", "Port 3", "Extra"]
        );
        assert_eq!(lines[2].1, "5.0 mL");
        assert!(!has_run_info(&BTreeMap::new()));
    }

    /// Every panel draws in both themes and every state without panicking.
    #[test]
    fn panels_draw_in_every_state() {
        let ctx = egui::Context::default();
        rc_theme::install_fonts(&ctx);
        let mut info = BTreeMap::new();
        info.insert("Sample".to_string(), "Lysate".to_string());
        info.insert("Port 3".to_string(), "NaCl 1 M".to_string());
        let point = TrendPoint {
            cum_time_ms: 60_000.0,
            cum_vol_ml: 1.0,
            pctb: 40.0,
            ..Default::default()
        };
        let events = vec![
            ev(EventKind::Info, "Sample: Lysate", 0.0, 0.0),
            ev(EventKind::Mark, "Peak", 30_000.0, 0.5),
            ev(EventKind::Log, "Flow 1.0 ml/min", 30_000.0, 0.5),
        ];
        let mut log = RunLogState::default();
        for dark in [true, false] {
            rc_theme::apply(&ctx, dark);
            for (inject, port, known, frac, mode, pt) in [
                (
                    InjectValue::Load,
                    1,
                    true,
                    0.0,
                    ShownMode::Live,
                    Some(&point),
                ),
                (
                    InjectValue::Inject,
                    3,
                    true,
                    1.0,
                    ShownMode::Past,
                    Some(&point),
                ),
                (
                    InjectValue::Waste,
                    8,
                    false,
                    0.0,
                    ShownMode::EndOfLog,
                    Some(&point),
                ),
                (
                    InjectValue::Unknown,
                    12,
                    false,
                    0.0,
                    ShownMode::EndOfLog,
                    None,
                ),
            ] {
                let state = InstrumentState {
                    inject_value: inject,
                    buffer_port: port,
                    buffer_port_known: known,
                    fraction_vol_ml: frac,
                    ..Default::default()
                };
                let d = PanelData {
                    state: &state,
                    point: pt,
                    flow: Some(1.0),
                    info: &info,
                    mode,
                    viewer: false,
                    edit_label: None,
                };
                for filter in ["", "peak", "nothing"] {
                    log.filter = filter.to_string();
                    let out = ctx.run_ui(Default::default(), |ui| {
                        assert!(!side_panel(ui, &d));
                        // the edit button shows the card even without run info
                        let empty = BTreeMap::new();
                        for info in [&info, &empty] {
                            let e = PanelData {
                                info,
                                edit_label: Some("Edit"),
                                ..d
                            };
                            assert!(!side_panel(ui, &e));
                        }
                        run_log(ui, &events, 0.0, &mut log, 5);
                        for (status, recording, viewer) in [
                            ("● Connected on /dev/ttyUSB0", true, false),
                            ("Demo mode — simulated data", false, false),
                            ("", false, true),
                        ] {
                            status_bar(
                                ui,
                                &StatusInfo {
                                    status,
                                    recording,
                                    log_filename: "a.csv",
                                    viewer,
                                    n_points: 3,
                                    run_time_ms: 60_000.0,
                                    help: "Drag: zoom",
                                },
                            );
                        }
                    });
                    out.drop_without_applying_deltas();
                }
            }
        }
    }
}
