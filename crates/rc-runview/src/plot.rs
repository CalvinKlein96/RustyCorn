//! The chromatogram: stacked panels with a linked x axis, or all traces on
//! one overlay axis; drawn with egui's painter. Interactions: drag = rectangle zoom, right-drag = pan,
//! scroll = zoom, Ctrl-click = reset (resume following), Alt/Shift-click =
//! inspect the nearest moment; hovering shows every value under the mouse.

use std::f32::consts::FRAC_PI_2;

use egui::epaint::TextShape;
use egui::text::{LayoutJob, TextFormat};
use egui::{
    Align2, Color32, CursorIcon, FontId, LayerId, Order, Painter, PointerButton, Pos2, Rect, Sense,
    Shape, Stroke, StrokeKind, pos2, vec2,
};
use rc_plotdata::{Channel, Pt};
use rc_theme::{FS_BODY, FS_SMALL, Palette};

use crate::view::Pane;
use crate::{BoxDrag, RunView, channel_color, ticks};

/// Same fixed room for the y tick labels in every panel, so plots stay aligned
/// and do not shift when the values (and so the tick label widths) change.
const YTICK_W: f32 = 42.0;
const TICK: f32 = 4.0;
const PAD_L: f32 = 10.0;
const PAD_R: f32 = 14.0;
const PAD_T: f32 = 12.0;
/// x tick labels and the x axis label below the bottom panel
const XAXIS_H: f32 = 42.0;
/// rotated y-axis label of the overlay
const YLABEL_W: f32 = 18.0;
const GAP: f32 = 12.0;
const LINE_W: f32 = 1.6;
/// Opacity of the grid lines (the text colour), faint like the PDF export's.
const GRID_ALPHA: f32 = 0.07;
/// keep far-away points (deep zoom) within what f32 tessellation handles well
const FAR: f32 = 1.0e6;

/// Data ↔ screen mapping of one pane.
#[derive(Clone, Copy)]
struct Xf {
    r: Rect,
    x: (f64, f64),
    y: (f64, f64),
}

impl Xf {
    fn sx(&self, x: f64) -> f32 {
        let t = (x - self.x.0) / (self.x.1 - self.x.0);
        (self.r.left() + (t * self.r.width() as f64) as f32).clamp(-FAR, FAR)
    }
    fn sy(&self, y: f64) -> f32 {
        let t = (y - self.y.0) / (self.y.1 - self.y.0);
        (self.r.bottom() - (t * self.r.height() as f64) as f32).clamp(-FAR, FAR)
    }
    fn pos(&self, p: Pt) -> Pos2 {
        pos2(self.sx(p[0]), self.sy(p[1]))
    }
    fn data_x(&self, sx: f32) -> f64 {
        self.x.0 + (sx - self.r.left()) as f64 / self.r.width() as f64 * (self.x.1 - self.x.0)
    }
    fn data_y(&self, sy: f32) -> f64 {
        self.y.0 + (self.r.bottom() - sy) as f64 / self.r.height() as f64 * (self.y.1 - self.y.0)
    }
}

/// Panel name inside the panel (top right): fit for short panels, unlike rotated axis titles.
fn pane_title(ch: Channel) -> &'static str {
    match ch {
        Channel::Uv => "UV (AU)",
        Channel::Pctb => "%B",
        Channel::Cond => "Cond (mS/cm)",
        Channel::Ph => "pH",
        Channel::Pres => "Pressure (MPa)",
        Channel::Flow => "Flow (mL/min)",
    }
}

/// Polyline points of a step plot (`stairs`, step = :post).
fn steps(pts: &[Pt]) -> Vec<Pt> {
    let mut out = Vec::with_capacity(pts.len() * 2);
    for (i, p) in pts.iter().enumerate() {
        if i > 0 {
            out.push([p[0], pts[i - 1][1]]);
        }
        out.push(*p);
    }
    out
}

fn polyline(painter: &Painter, xf: &Xf, pts: &[Pt], stroke: Stroke) {
    let line: Vec<Pos2> = pts
        .iter()
        .filter(|p| p[1].is_finite())
        .map(|p| xf.pos(*p))
        .collect();
    match line.len() {
        0 => {}
        1 => {
            painter.circle_filled(line[0], stroke.width, stroke.color);
        }
        _ => {
            painter.add(Shape::line(line, stroke));
        }
    }
}

impl RunView {
    pub(crate) fn plot(&mut self, ui: &mut egui::Ui, card: Rect) {
        let t = rc_theme::palette(ui.ctx());
        ui.painter().rect(
            card,
            t.radius,
            t.card,
            Stroke::new(1.0, t.card_border),
            StrokeKind::Inside,
        );
        let panes = self.panes();
        let left = PAD_L + YTICK_W + 2.0 * TICK + if self.overlay { YLABEL_W } else { 0.0 };
        let inner = Rect::from_min_max(
            pos2(card.left() + left, card.top() + PAD_T),
            pos2(card.right() - PAD_R, card.bottom() - XAXIS_H),
        );
        if inner.width() < 20.0 || inner.height() < 20.0 {
            return;
        }
        let n = panes.len().max(1) as f32;
        let h = ((inner.height() - GAP * (n - 1.0)) / n).max(8.0);
        let rects: Vec<(Pane, Rect)> = panes
            .iter()
            .enumerate()
            .map(|(i, &p)| {
                (
                    p,
                    Rect::from_min_size(
                        pos2(inner.left(), inner.top() + i as f32 * (h + GAP)),
                        vec2(inner.width(), h),
                    ),
                )
            })
            .collect();

        // ── interactions (they change the view before it is drawn) ──
        let mut hover: Option<(Pane, Pos2)> = None;
        for &(pane, r) in &rects {
            let resp = ui.interact(r, self.id.with(("pane", pane)), Sense::click_and_drag());
            if let Some(pos) = self.interact(ui, pane, r, &resp) {
                hover = Some((pane, pos));
            }
        }
        // a Ctrl-click (resume) or a zoom: limits and traces for the new view
        self.prepare();

        // ── draw ──
        let xlabel = self.xlabel();
        let last = rects.len().saturating_sub(1);
        for (i, &(pane, r)) in rects.iter().enumerate() {
            let xf = Xf {
                r,
                x: self.view.lim.x,
                y: self.view.lim.y_of(pane),
            };
            self.draw_pane(ui, &t, pane, &xf, i == last);
            if i == last {
                // x tick labels and the x label only on the bottom panel
                let p = ui.painter();
                let (xt, xl) =
                    ticks::labelled(xf.x.0, xf.x.1, ((r.width() / 140.0) as usize).clamp(3, 10));
                for (v, l) in xt.iter().zip(&xl) {
                    p.text(
                        pos2(xf.sx(*v), r.bottom() + TICK + 2.0),
                        Align2::CENTER_TOP,
                        l,
                        rc_theme::mono(FS_SMALL),
                        t.text_dim,
                    );
                }
                p.text(
                    pos2(r.center().x, r.bottom() + TICK + 18.0),
                    Align2::CENTER_TOP,
                    &xlabel,
                    FontId::proportional(FS_BODY),
                    t.text_dim,
                );
            }
        }

        // ── the marker and hover lines, through all panels ──
        let (Some(first), Some(lastr)) = (rects.first(), rects.last()) else {
            return;
        };
        let span_y = (first.1.top(), lastr.1.bottom());
        let xf0 = Xf {
            r: first.1,
            x: self.view.lim.x,
            y: (0.0, 1.0),
        };
        let col_painter = ui.painter().with_clip_rect(Rect::from_min_max(
            pos2(inner.left(), span_y.0),
            pos2(inner.right(), span_y.1),
        ));
        let xs = self.xs();
        let n_pts = self.snap.trend.len();
        if !self.sel.following && n_pts > 0 {
            // dashed marker at the selected moment (none while following live)
            let p = &self.snap.trend[self.sel.index.min(n_pts - 1)];
            let sx = xf0.sx(rc_plotdata::trend_x(p, xs));
            col_painter.extend(Shape::dashed_line(
                &[pos2(sx, span_y.0), pos2(sx, span_y.1)],
                Stroke::new(1.5, t.text.gamma_multiply(0.6)),
                6.0,
                4.0,
            ));
        }
        self.hover = None;
        if let Some((pane, pos)) = hover {
            let r = rects
                .iter()
                .find(|(p, _)| *p == pane)
                .map(|(_, r)| *r)
                .unwrap_or(first.1);
            let xf = Xf {
                r,
                x: self.view.lim.x,
                y: self.view.lim.y_of(pane),
            };
            if let Some(i) = self.nearest(xf.data_x(pos.x)) {
                self.hover = Some(i);
                let p = self.snap.trend[i];
                let sx = xf.sx(rc_plotdata::trend_x(&p, xs));
                col_painter.line_segment(
                    [pos2(sx, span_y.0), pos2(sx, span_y.1)],
                    Stroke::new(1.0, t.text_dim.gamma_multiply(0.6)),
                );
                if self.drag.is_none() {
                    let right_half = pos.x > r.center().x;
                    self.tooltip(ui, &t, i, pos, right_half);
                }
            }
        }

        // rectangle zoom in progress
        if let Some(d) = self.drag
            && let (Some(cur), Some((_, r))) = (
                ui.input(|i| i.pointer.latest_pos()),
                rects.iter().find(|(p, _)| *p == d.pane),
            )
        {
            let b = Rect::from_two_pos(d.start, cur).intersect(*r);
            ui.painter().rect(
                b,
                0.0,
                t.accent_soft,
                Stroke::new(1.0, t.accent),
                StrokeKind::Inside,
            );
        }

        // export result, as a toast at the top of the plot
        if let Some((m, _)) = self.export.current() {
            let col = if m.error { t.err } else { t.ok };
            let g =
                ui.painter()
                    .layout_no_wrap(m.text.clone(), rc_theme::medium(FS_SMALL + 1.0), col);
            let size = g.size() + vec2(20.0, 10.0);
            let rr = Rect::from_center_size(
                pos2(inner.center().x, card.top() + size.y / 2.0 + 6.0),
                size,
            );
            let p = ui.painter();
            p.rect(
                rr,
                9.0,
                t.tint(col, 0.15),
                Stroke::new(1.0, col.gamma_multiply(0.5)),
                StrokeKind::Inside,
            );
            p.galley(rr.min + vec2(10.0, 5.0), g, col);
        }
    }

    /// Mouse handling of one pane; returns the pointer position when hovering it.
    fn interact(
        &mut self,
        ui: &egui::Ui,
        pane: Pane,
        r: Rect,
        resp: &egui::Response,
    ) -> Option<Pos2> {
        let xf = Xf {
            r,
            x: self.view.lim.x,
            y: self.view.lim.y_of(pane),
        };
        let mods = ui.input(|i| i.modifiers);
        // drag: rectangle zoom
        if resp.drag_started_by(PointerButton::Primary) {
            let start = ui
                .input(|i| i.pointer.press_origin())
                .or(resp.interact_pointer_pos());
            self.drag = start.map(|start| BoxDrag { pane, start });
        }
        if resp.drag_stopped_by(PointerButton::Primary)
            && let (Some(d), Some(end)) = (
                self.drag.take(),
                resp.interact_pointer_pos()
                    .or(ui.input(|i| i.pointer.latest_pos())),
            )
            && d.pane == pane
        {
            let b = Rect::from_two_pos(d.start, end).intersect(r);
            if b.width() >= 4.0 && b.height() >= 4.0 {
                self.view.zoom_to(
                    pane,
                    (xf.data_x(b.left()), xf.data_x(b.right())),
                    (xf.data_y(b.bottom()), xf.data_y(b.top())),
                );
            }
        }
        // right-drag: pan
        if resp.dragged_by(PointerButton::Secondary) {
            let d = resp.drag_delta();
            if d != egui::Vec2::ZERO {
                let dx = -d.x as f64 / r.width() as f64 * (xf.x.1 - xf.x.0);
                let dy = d.y as f64 / r.height() as f64 * (xf.y.1 - xf.y.0);
                self.view.pan(pane, dx, dy);
            }
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        }
        if resp.clicked() {
            if mods.command || mods.ctrl {
                // Ctrl-click: reset the zoom and follow the data again
                self.view.resume();
            } else if (mods.alt || mods.shift)
                && let Some(pos) = resp.interact_pointer_pos()
                && let Some(i) = self.nearest(xf.data_x(pos.x))
            {
                // Alt- or Shift-click jumps the slider to the nearest data point
                self.sel.select(i, self.snap.trend.len());
            }
        }
        let pos = resp.hover_pos().filter(|p| r.contains(*p))?;
        if self.drag.is_none() && !resp.dragged() {
            // scroll: zoom around the mouse; sideways scroll pans
            let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta()));
            let f = ((-scroll.y as f64) * 0.0025).exp() / zoom as f64;
            if (f - 1.0).abs() > 1e-6 {
                self.view
                    .zoom_at(pane, (xf.data_x(pos.x), xf.data_y(pos.y)), f, f);
            }
            if scroll.x != 0.0 {
                self.view.pan(
                    pane,
                    -scroll.x as f64 / r.width() as f64 * (xf.x.1 - xf.x.0),
                    0.0,
                );
            }
            ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
        }
        Some(pos)
    }

    /// Index of the trend point nearest to data x.
    fn nearest(&self, x: f64) -> Option<usize> {
        let td = &self.snap.trend;
        if td.is_empty() || !x.is_finite() {
            return None;
        }
        let xs = self.xs();
        let mut i = rc_plotdata::first_at_or_after(td, x, xs).min(td.len() - 1);
        if i > 0
            && (rc_plotdata::trend_x(&td[i - 1], xs) - x).abs()
                < (rc_plotdata::trend_x(&td[i], xs) - x).abs()
        {
            i -= 1;
        }
        Some(i)
    }

    fn draw_pane(&mut self, ui: &egui::Ui, t: &Palette, pane: Pane, xf: &Xf, bottom: bool) {
        let r = xf.r;
        let clip = ui.painter().with_clip_rect(r.intersect(ui.clip_rect()));
        clip.rect_filled(r, 0.0, t.plot_bg);
        let is = |c: Channel| pane == Pane::Chan(c);

        // fraction collection spans on UV, %B and the overlay
        if is(Channel::Uv) || is(Channel::Pctb) || pane == Pane::Overlay {
            for s in &self.derived.spans {
                if s.x1 < xf.x.0 || s.x0 > xf.x.1 {
                    continue;
                }
                let x0 = xf.sx(s.x0).max(r.left());
                let x1 = xf.sx(s.x1).min(r.right());
                let c = t.frac_fill[s.shade % t.frac_fill.len()];
                clip.rect_filled(Rect::from_x_y_ranges(x0..=x1, r.y_range()), 0.0, c);
            }
        }
        // grid at the ticks, very faint as in the PDF export: above the fraction
        // shading, below event lines and traces; none on the spines
        let grid = Stroke::new(1.0, t.text.gamma_multiply(GRID_ALPHA));
        let (gx, _) = ticks::labelled(xf.x.0, xf.x.1, ((r.width() / 140.0) as usize).clamp(3, 10));
        for v in gx {
            let sx = xf.sx(v).round() + 0.5;
            if sx > r.left() + 1.0 && sx < r.right() - 1.0 {
                clip.vline(sx, r.y_range(), grid);
            }
        }
        let (gy, _, _) = self.y_ticks(t, pane, xf);
        for v in gy {
            let sy = xf.sy(v).round() + 0.5;
            if sy > r.top() + 1.0 && sy < r.bottom() - 1.0 {
                clip.hline(r.x_range(), sy, grid);
            }
        }
        // event lines on UV and the overlay (solid, or dashed per event_style)
        if is(Channel::Uv) || pane == Pane::Overlay {
            let mut shapes = Vec::new();
            for l in &self.derived.lines {
                if l.x < xf.x.0 || l.x > xf.x.1 {
                    continue;
                }
                let sx = xf.sx(l.x).round() + 0.5;
                let stroke = Stroke::new(1.0, l.color);
                if l.dashed {
                    Shape::dashed_line_many(
                        &[pos2(sx, r.top()), pos2(sx, r.bottom())],
                        stroke,
                        5.0,
                        3.0,
                        &mut shapes,
                    );
                } else {
                    shapes.push(Shape::vline(sx, r.y_range(), stroke));
                }
            }
            clip.extend(shapes);
        }

        // traces
        match pane {
            Pane::Chan(Channel::Flow) => {
                polyline(
                    &clip,
                    xf,
                    &steps(&self.derived.flow),
                    Stroke::new(LINE_W, t.flow),
                );
            }
            Pane::Chan(ch) => {
                polyline(
                    &clip,
                    xf,
                    self.traces.points(ch),
                    Stroke::new(LINE_W, channel_color(t, ch)),
                );
            }
            Pane::Overlay => {
                let chans = self.overlay_channels();
                // dimmed traces first, the highlighted one on top
                let mut order: Vec<Channel> = chans
                    .iter()
                    .copied()
                    .filter(|c| Some(*c) != self.highlighted)
                    .collect();
                order.extend(self.highlighted.filter(|c| chans.contains(c)));
                for ch in order {
                    let on = self.highlighted == Some(ch);
                    let dim = self.highlighted.is_some() && !on;
                    let col = channel_color(t, ch);
                    let col = if dim {
                        col.gamma_multiply(rc_plotdata::OVERLAY_DIM_ALPHA)
                    } else {
                        col
                    };
                    let w = if on {
                        rc_plotdata::OVERLAY_HIGHLIGHT_LINEWIDTH
                    } else {
                        rc_plotdata::OVERLAY_LINEWIDTH
                    } as f32;
                    let (pts, _) = self.overlay_trace(ch);
                    let pts = if ch == Channel::Flow {
                        steps(pts)
                    } else {
                        pts.to_vec()
                    };
                    polyline(&clip, xf, &pts, Stroke::new(w, col));
                }
            }
        }

        // panel name (top right)
        let mut avoid = None;
        if let Pane::Chan(ch) = pane {
            let g = ui.painter().layout_no_wrap(
                pane_title(ch).to_string(),
                rc_theme::semibold(FS_SMALL + 1.0),
                channel_color(t, ch),
            );
            let pos = pos2(r.right() - 6.0 - g.size().x, r.top() + 4.0);
            avoid = Some((
                (pos.x - r.left() - 4.0) as f64,
                r.width() as f64,
                0.0,
                (g.size().y + 6.0) as f64,
            ));
            // a soft backing so the name stays legible over a trace
            clip.rect_filled(
                Rect::from_min_size(pos, g.size()).expand2(vec2(4.0, 1.0)),
                4.0,
                t.plot_bg.gamma_multiply(0.85),
            );
            clip.galley(pos, g, t.text);
        }

        // event labels, laid out in pixels from the top of the visible area
        if is(Channel::Uv) || pane == Pane::Overlay {
            self.draw_event_labels(
                &clip,
                t,
                xf,
                if pane == Pane::Overlay { None } else { avoid },
            );
        }
        // fraction numbers in the %B panel near the baseline
        if is(Channel::Pctb) {
            let col = if t.dark {
                t.uv
            } else {
                rc_theme::rgb3(rc_plotdata::FRAC_LABEL_COLOR)
            };
            // numbers that would overlap the previous one are left out (narrow fractions)
            let mut right = f32::NEG_INFINITY;
            for s in &self.derived.spans {
                let m = s.mid();
                if m < xf.x.0 || m > xf.x.1 {
                    continue;
                }
                let g = ui.painter().layout_no_wrap(
                    s.number.to_string(),
                    rc_theme::semibold(12.0),
                    col,
                );
                let x = xf.sx(m);
                if x - g.size().x / 2.0 < right + 3.0 {
                    continue;
                }
                right = x + g.size().x / 2.0;
                clip.galley(pos2(x - g.size().x / 2.0, xf.sy(1.0) - g.size().y), g, col);
            }
        }

        // axes: open frame (left and bottom spines), ticks, y tick labels
        let p = ui.painter();
        let axis = Stroke::new(1.0, t.axis);
        p.line_segment([pos2(r.left(), r.top()), pos2(r.left(), r.bottom())], axis);
        p.line_segment(
            [pos2(r.left(), r.bottom()), pos2(r.right(), r.bottom())],
            axis,
        );
        let (xt, _) = ticks::labelled(xf.x.0, xf.x.1, ((r.width() / 140.0) as usize).clamp(3, 10));
        for v in xt {
            let sx = xf.sx(v);
            p.line_segment([pos2(sx, r.bottom()), pos2(sx, r.bottom() + TICK)], axis);
        }
        let _ = bottom;
        let (yt, yl, ycol) = self.y_ticks(t, pane, xf);
        let fits = r.height() >= 28.0;
        // Ticks of a nearly flat trace need many decimals ("0.0200010"): such
        // labels get a smaller font rather than being cut off at the card edge.
        // (the overlay's rotated axis label sits just left of the room)
        let room = if pane == Pane::Overlay {
            YTICK_W + 2.0
        } else {
            YTICK_W + PAD_L - 2.0
        };
        let widest = yl
            .iter()
            .map(|l| {
                p.layout_no_wrap(l.clone(), rc_theme::mono(FS_SMALL), ycol)
                    .size()
                    .x
            })
            .fold(0.0, f32::max);
        let yfont = if widest > room {
            rc_theme::mono((FS_SMALL * room / widest).max(7.0))
        } else {
            rc_theme::mono(FS_SMALL)
        };
        for (i, v) in yt.iter().enumerate() {
            let sy = xf.sy(*v);
            if sy < r.top() - 0.5 || sy > r.bottom() + 0.5 {
                continue;
            }
            p.line_segment([pos2(r.left() - TICK, sy), pos2(r.left(), sy)], axis);
            if fits || i == 0 || i + 1 == yt.len() {
                p.text(
                    pos2(r.left() - TICK - 4.0, sy),
                    Align2::RIGHT_CENTER,
                    &yl[i],
                    yfont.clone(),
                    ycol,
                );
            }
        }
        // overlay: y label (rotated), in % of range or the highlighted trace's units
        if pane == Pane::Overlay {
            let (text, col) = match self.highlighted {
                Some(ch) => (
                    rc_plotdata::channel_ylabel(ch.name(), ch.unit()),
                    channel_color(t, ch),
                ),
                None => ("Scaled (% of range)".to_string(), t.text_dim),
            };
            let g = ui
                .painter()
                .layout_no_wrap(text, FontId::proportional(FS_BODY), col);
            let x = r.left() - TICK - 8.0 - YTICK_W - g.size().y;
            let pos = pos2(x, r.center().y + g.size().x / 2.0);
            p.add(TextShape::new(pos, g, col).with_angle(-FRAC_PI_2));
        }
    }

    /// y ticks of a pane: about 3 in the stacked panels, 5 in the overlay, or the
    /// highlighted trace's real values.
    fn y_ticks(&mut self, t: &Palette, pane: Pane, xf: &Xf) -> (Vec<f64>, Vec<String>, Color32) {
        match pane {
            Pane::Chan(_) => {
                let target = if xf.r.height() < 60.0 { 2 } else { 3 };
                let (v, l) = ticks::labelled(xf.y.0, xf.y.1, target);
                (v, l, t.text_dim)
            }
            Pane::Overlay => match self.highlighted {
                Some(ch) => {
                    let (_, range) = self.overlay_trace(ch);
                    let (v, l) = rc_plotdata::overlay_real_ticks(range, xf.y);
                    (v, l, channel_color(t, ch))
                }
                None => {
                    let (v, l) = ticks::labelled(xf.y.0, xf.y.1, 5);
                    (v, l, t.text_dim)
                }
            },
        }
    }

    fn draw_event_labels(
        &mut self,
        painter: &Painter,
        t: &Palette,
        xf: &Xf,
        avoid: Option<(f64, f64, f64, f64)>,
    ) {
        let d = &self.derived;
        if d.label_x.is_empty() {
            return;
        }
        let lims = (xf.x.0, xf.y.0, xf.x.1 - xf.x.0, xf.y.1 - xf.y.0);
        let px = (xf.r.width() as f64, xf.r.height() as f64);
        let av = avoid.unwrap_or((0.0, 0.0, 0.0, 0.0));
        let key = [
            self.snap.version,
            self.xs().to_bits(),
            lims.0.to_bits(),
            lims.1.to_bits(),
            lims.2.to_bits(),
            lims.3.to_bits(),
            (px.0 as f32).to_bits() as u64 | (((px.1 as f32).to_bits() as u64) << 32),
            av.0.to_bits(),
            av.3.to_bits(),
            self.events_shown as u64,
        ];
        if self.labels.key != Some(key) {
            self.labels.pos =
                rc_plotdata::event_label_positions(&d.label_x, &d.label_text, lims, px, avoid);
            self.labels.key = Some(key);
        }
        let font = rc_theme::medium(rc_plotdata::EVENT_LABEL_FONTSIZE as f32);
        for (i, a) in self.labels.pos.iter().enumerate() {
            if d.label_x[i] < xf.x.0 || d.label_x[i] > xf.x.1 {
                continue;
            }
            let pos = xf.pos(*a);
            if pos.y > xf.r.bottom() {
                continue;
            }
            painter.text(
                pos,
                Align2::LEFT_TOP,
                &d.label_text[i],
                font.clone(),
                t.text_dim,
            );
        }
    }

    /// Hover readout: every value at the point under the mouse.
    fn tooltip(&self, ui: &egui::Ui, t: &Palette, i: usize, mouse: Pos2, left_side: bool) {
        let p = &self.snap.trend[i];
        let flow = {
            let xs = self.xs();
            let x = rc_plotdata::trend_x(p, xs);
            let f = &self.derived.flow;
            let k = f.partition_point(|q| q[0] <= x);
            (k > 0).then(|| f[k - 1][1])
        };
        let mut rows: Vec<(&str, Color32, String, &str)> = vec![
            ("UV", t.uv, format!("{:.4}", p.uv), "AU"),
            ("%B", t.pctb, format!("{:.1}", p.pctb), "%"),
            ("Cond", t.cond, format!("{:.2}", p.cond_mscm), "mS/cm"),
            ("pH", t.ph, format!("{:.2}", p.ph), ""),
            ("Pressure", t.pres, format!("{:.3}", p.pres_mpa), "MPa"),
        ];
        if let Some(f) = flow {
            rows.push(("Flow", t.flow, format!("{f:.2}"), "mL/min"));
        }
        let painter = ui
            .ctx()
            .layer_painter(LayerId::new(Order::Tooltip, self.id.with("tip")));
        let head = painter.layout_no_wrap(
            format!(
                "{:.2} min · {:.3} mL",
                p.cum_time_ms / 60_000.0,
                p.cum_vol_ml
            ),
            rc_theme::semibold(FS_SMALL + 1.0),
            t.text,
        );
        let name_font = rc_theme::medium(FS_SMALL);
        let val_font = rc_theme::mono(FS_SMALL + 0.5);
        let names: Vec<_> = rows
            .iter()
            .map(|r| painter.layout_no_wrap(r.0.to_string(), name_font.clone(), r.1))
            .collect();
        let vals: Vec<_> = rows
            .iter()
            .map(|r| {
                let mut job = LayoutJob::default();
                job.append(
                    &r.2,
                    0.0,
                    TextFormat {
                        font_id: val_font.clone(),
                        color: t.text,
                        ..Default::default()
                    },
                );
                if !r.3.is_empty() {
                    job.append(
                        r.3,
                        4.0,
                        TextFormat {
                            font_id: rc_theme::medium(FS_SMALL - 1.0),
                            color: t.text_dim,
                            ..Default::default()
                        },
                    );
                }
                painter.layout_job(job)
            })
            .collect();
        let name_w = names.iter().map(|g| g.size().x).fold(0.0, f32::max);
        let val_w = vals.iter().map(|g| g.size().x).fold(0.0, f32::max);
        let row_h = 16.0;
        let pad = 9.0;
        let w = (name_w + 12.0 + val_w).max(head.size().x) + 2.0 * pad;
        let h = head.size().y + 6.0 + rows.len() as f32 * row_h + 2.0 * pad;
        let off = 14.0;
        let mut min = if left_side {
            pos2(mouse.x - off - w, mouse.y - h / 2.0)
        } else {
            pos2(mouse.x + off, mouse.y - h / 2.0)
        };
        let screen = ui.ctx().content_rect();
        min.y = min.y.clamp(
            screen.top() + 4.0,
            (screen.bottom() - h - 4.0).max(screen.top()),
        );
        let rect = Rect::from_min_size(min, vec2(w, h));
        painter.add(egui::epaint::RectShape::filled(
            rect.translate(vec2(0.0, 3.0)).expand(1.0),
            7.0,
            t.shadow.gamma_multiply(0.5),
        ));
        painter.rect(
            rect,
            7.0,
            t.card,
            Stroke::new(1.0, t.card_border),
            StrokeKind::Inside,
        );
        let mut y = rect.top() + pad;
        let hh = head.size().y;
        painter.galley(pos2(rect.left() + pad, y), head, t.text);
        y += hh + 6.0;
        for (n, v) in names.into_iter().zip(vals) {
            let cy = y + row_h / 2.0;
            painter.galley(pos2(rect.left() + pad, cy - n.size().y / 2.0), n, t.text);
            painter.galley(
                pos2(rect.left() + pad + name_w + 12.0, cy - v.size().y / 2.0),
                v,
                t.text,
            );
            y += row_h;
        }
    }
}
