//! PDF export of the displayed chromatogram: the stacked panels and the
//! overlay, with the limits shown on screen. The figure is drawn as
//! SVG and converted to PDF with svg2pdf; always light colours (paper).
//!
//! One page: the stacked panels (one per visible channel, with their on-screen
//! y-limits and the shared x-limits), then the overlay below them (every
//! channel scaled to 0–100 %). Full data, not downsampled. Fonts are the
//! embedded Inter files, so the output does not depend on system fonts.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use rc_core::{AktaEvent, Fraction, TrendPoint};
use rc_plotdata::{self as pd, Channel, EventDisplay, LineStyle, Pt, Rgb, XScale};
use svg2pdf::usvg;

/// One stacked panel: its channel and the y-range shown on screen.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelSpec {
    pub channel: Channel,
    pub ylim: (f64, f64),
}

/// Everything needed to draw the export (copied out of the AppState).
#[derive(Clone, Debug)]
pub struct ExportRequest {
    pub trend: Vec<TrendPoint>,
    pub events: Vec<AktaEvent>,
    pub fractions: Vec<Fraction>,
    /// visible panels top to bottom, UV first
    pub panels: Vec<PanelSpec>,
    pub xs: XScale,
    /// x-range shown on screen
    pub xlim: (f64, f64),
    /// overlay y-range: the on-screen one when the overlay is the view shown, else OVERLAY_YLIMS
    pub overlay_ylim: (f64, f64),
    /// the highlighted overlay trace, if any
    pub highlighted: Option<Channel>,
    /// optional title line (e.g. the log file name)
    pub title: Option<String>,
    /// which event lines are drawn (the run page's "Events" choice); fraction
    /// spans and numbers are drawn in every setting
    pub events_shown: EventDisplay,
}

impl Default for ExportRequest {
    /// No data, time axis, limits from the data, every event shown.
    fn default() -> Self {
        ExportRequest {
            trend: Vec::new(),
            events: Vec::new(),
            fractions: Vec::new(),
            panels: Vec::new(),
            xs: pd::XS_TIME,
            xlim: (f64::NAN, f64::NAN),
            overlay_ylim: pd::OVERLAY_YLIMS,
            highlighted: None,
            title: None,
            events_shown: EventDisplay::All,
        }
    }
}

/// `<dir>/<prefix>_<yyyymmdd_HHMMSS>_figure_<n>.pdf` ("akta" for an empty prefix).
pub fn figure_path(dir: &Path, prefix: &str, n: u32) -> PathBuf {
    let prefix = if prefix.is_empty() { "akta" } else { prefix };
    let stamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    dir.join(format!("{prefix}_{stamp}_figure_{n}.pdf"))
}

/// Render and write the PDF.
pub fn save_pdf(req: &ExportRequest, path: &Path) -> Result<(), String> {
    let result = pdf_bytes(req).and_then(|pdf| {
        std::fs::write(path, pdf).map_err(|e| format!("Could not write {}: {e}", path.display()))
    });
    match &result {
        Ok(()) => log::info!("Saved plot to {}", path.display()),
        Err(err) => log::error!("Could not save plot: {err}"),
    }
    result
}

/// The figure as PDF bytes.
pub fn pdf_bytes(req: &ExportRequest) -> Result<Vec<u8>, String> {
    if req.trend.is_empty() {
        return Err("Nothing to save yet — no data on the plot".to_string());
    }
    let svg = render_svg(req);
    let opt = usvg::Options {
        fontdb: FONTDB.clone(),
        font_family: "Inter".to_string(),
        ..Default::default()
    };
    let tree = usvg::Tree::from_str(&svg, &opt).map_err(|e| format!("SVG error: {e}"))?;
    // 96 dpi: one SVG pixel is 0.75 pt, so a 1000 px wide figure is 750 pt wide.
    svg2pdf::to_pdf(
        &tree,
        svg2pdf::ConversionOptions::default(),
        svg2pdf::PageOptions { dpi: 96.0 },
    )
    .map_err(|e| format!("PDF error: {e}"))
}

// ── Fonts ─────────────────────────────────────────────────────────────────────

static INTER_REGULAR: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
static INTER_MEDIUM: &[u8] = include_bytes!("../../../assets/fonts/Inter-Medium.ttf");
static INTER_SEMIBOLD: &[u8] = include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf");
static LOGO_PNG: &[u8] = include_bytes!("../../../assets/logo-64.png");

static FONTDB: LazyLock<Arc<usvg::fontdb::Database>> = LazyLock::new(|| {
    let mut db = usvg::fontdb::Database::new();
    for data in [INTER_REGULAR, INTER_MEDIUM, INTER_SEMIBOLD] {
        db.load_font_data(data.to_vec());
    }
    db.set_sans_serif_family("Inter");
    Arc::new(db)
});

static FACE_REGULAR: LazyLock<Option<ttf_parser::Face<'static>>> =
    LazyLock::new(|| ttf_parser::Face::parse(INTER_REGULAR, 0).ok());
static FACE_SEMIBOLD: LazyLock<Option<ttf_parser::Face<'static>>> =
    LazyLock::new(|| ttf_parser::Face::parse(INTER_SEMIBOLD, 0).ok());

#[derive(Clone, Copy, PartialEq)]
enum Weight {
    Regular,
    Medium,
    SemiBold,
}

impl Weight {
    fn css(self) -> u32 {
        match self {
            Weight::Regular => 400,
            Weight::Medium => 500,
            Weight::SemiBold => 600,
        }
    }
}

/// Width of `text` in px at `size` (glyph advances; no kerning).
fn text_width(text: &str, size: f64, weight: Weight) -> f64 {
    let face = if weight == Weight::Regular {
        &*FACE_REGULAR
    } else {
        &*FACE_SEMIBOLD
    };
    let Some(face) = face.as_ref() else {
        return text.chars().count() as f64 * size * 0.56;
    };
    let upem = f64::from(face.units_per_em());
    let units: f64 = text
        .chars()
        .map(|c| {
            face.glyph_index(c)
                .and_then(|g| face.glyph_hor_advance(g))
                .map_or(upem * 0.56, f64::from)
        })
        .sum();
    units / upem * size
}

// ── Look (the light palette, paper axis style) ────────────────────────────────

const TEXT: Rgb = [0.12, 0.14, 0.17];
const MUTED: Rgb = [0.44, 0.47, 0.52];
/// Spines and ticks: the theme's border, a little darker so it holds on paper.
const SPINE: Rgb = [0.70, 0.72, 0.75];
/// Grid lines at the ticks: thin and faint, so the figure does not look busy.
const GRID: Rgb = [0.12, 0.14, 0.17];
const GRID_ALPHA: f32 = 0.07;
const GRID_WIDTH: f64 = 0.5;

const W: f64 = 1000.0;
const PAD: f64 = 16.0;
const FS_TICK: f64 = 11.0;
const FS_LABEL: f64 = 13.0;
const FS_LEGEND: f64 = 11.0;
const FS_FRAC: f64 = 12.0;
const TICK_LEN: f64 = 4.0;
const TICK_GAP: f64 = 4.0;
const LINEWIDTH: f64 = 1.5;
const PANEL_GAP: f64 = 12.0;
const BLOCK_GAP: f64 = 30.0;
const TITLE_H: f64 = 30.0;
/// Room below an axis for its x tick labels and x label.
const XDECO: f64 = TICK_LEN + TICK_GAP + FS_TICK + 8.0 + FS_LABEL + 2.0;

fn rgb(c: Rgb) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(c[0]), b(c[1]), b(c[2]))
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// A plot area in px and the data range it shows.
#[derive(Clone, Copy)]
struct Area {
    left: f64,
    top: f64,
    w: f64,
    h: f64,
    xlim: (f64, f64),
    ylim: (f64, f64),
}

impl Area {
    fn sx(&self, x: f64) -> f64 {
        self.left + (x - self.xlim.0) / (self.xlim.1 - self.xlim.0) * self.w
    }
    fn sy(&self, y: f64) -> f64 {
        self.top + self.h - (y - self.ylim.0) / (self.ylim.1 - self.ylim.0) * self.h
    }
    fn bottom(&self) -> f64 {
        self.top + self.h
    }
    fn right(&self) -> f64 {
        self.left + self.w
    }
}

struct Svg {
    out: String,
    clips: usize,
}

impl Svg {
    #[allow(clippy::too_many_arguments)]
    fn text(
        &mut self,
        x: f64,
        y: f64,
        s: &str,
        size: f64,
        weight: Weight,
        color: Rgb,
        anchor: &str,
    ) {
        let _ = writeln!(
            self.out,
            r#"<text x="{x:.2}" y="{y:.2}" font-family="Inter" font-size="{size}" font-weight="{}" fill="{}" text-anchor="{anchor}">{}</text>"#,
            weight.css(),
            rgb(color),
            esc(s)
        );
    }
    fn vtext(&mut self, x: f64, y: f64, s: &str, size: f64, color: Rgb) {
        let _ = writeln!(
            self.out,
            r#"<text transform="translate({x:.2} {y:.2}) rotate(-90)" font-family="Inter" font-size="{size}" font-weight="400" fill="{}" text-anchor="middle">{}</text>"#,
            rgb(color),
            esc(s)
        );
    }
    fn line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, color: Rgb, width: f64) {
        let _ = writeln!(
            self.out,
            r#"<line x1="{x0:.2}" y1="{y0:.2}" x2="{x1:.2}" y2="{y1:.2}" stroke="{}" stroke-width="{width}"/>"#,
            rgb(color)
        );
    }
    /// Opens a group clipped to the area; close with `end_clip`.
    fn begin_clip(&mut self, a: &Area) {
        self.clips += 1;
        let id = self.clips;
        let _ = writeln!(
            self.out,
            r#"<clipPath id="c{id}"><rect x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}"/></clipPath><g clip-path="url(#c{id})">"#,
            a.left, a.top, a.w, a.h
        );
    }
    fn end_clip(&mut self) {
        self.out.push_str("</g>\n");
    }
}

/// Points of a trace that matter for the window: every segment that reaches
/// into it (full resolution), so lines run to the edges.
fn in_window(pts: &[Pt], xlim: (f64, f64)) -> Vec<Pt> {
    let mut keep = vec![false; pts.len()];
    for (i, p) in pts.iter().enumerate() {
        if p[0] >= xlim.0 && p[0] <= xlim.1 {
            keep[i] = true;
        }
        if let Some(q) = pts.get(i + 1)
            && p[0].min(q[0]) <= xlim.1
            && p[0].max(q[0]) >= xlim.0
        {
            keep[i] = true;
            keep[i + 1] = true;
        }
    }
    pts.iter()
        .zip(keep)
        .filter(|(_, k)| *k)
        .map(|(p, _)| *p)
        .collect()
}

/// SVG path data of a polyline (`steps` = a post step plot: each value holds until the next x); NaNs break it.
fn path_data(a: &Area, pts: &[Pt], steps: bool) -> String {
    let mut d = String::new();
    let mut pen = false;
    let clampy = |y: f64| y.clamp(-1e5, 1e5);
    for p in pts {
        if !p[0].is_finite() || !p[1].is_finite() {
            pen = false;
            continue;
        }
        let (x, y) = (a.sx(p[0]).clamp(-1e5, 1e5), clampy(a.sy(p[1])));
        if !pen {
            let _ = write!(d, "M{x:.2} {y:.2}");
            pen = true;
        } else if steps {
            let _ = write!(d, "H{x:.2}V{y:.2}");
        } else {
            let _ = write!(d, "L{x:.2} {y:.2}");
        }
    }
    d
}

fn trace(svg: &mut Svg, a: &Area, pts: &[Pt], steps: bool, color: Rgb, alpha: f32, width: f64) {
    let d = path_data(a, &in_window(pts, a.xlim), steps);
    if d.is_empty() {
        return;
    }
    let _ = writeln!(
        svg.out,
        r#"<path d="{d}" fill="none" stroke="{}" stroke-opacity="{alpha}" stroke-width="{width}" stroke-linejoin="round" stroke-linecap="round"/>"#,
        rgb(color)
    );
}

fn fraction_spans(svg: &mut Svg, a: &Area, spans: &[pd::FractionSpan]) {
    for s in spans {
        let (x0, x1) = (
            a.sx(s.x0).max(a.left - 1.0),
            a.sx(s.x1).min(a.right() + 1.0),
        );
        if x1 <= x0 {
            continue;
        }
        let _ = writeln!(
            svg.out,
            r#"<rect x="{x0:.2}" y="{:.2}" width="{:.2}" height="{:.2}" fill="{}" fill-opacity="{}"/>"#,
            a.top,
            x1 - x0,
            a.h,
            rgb(s.color),
            pd::FRAC_ALPHA
        );
    }
}

/// Faint grid lines at the tick positions (inside the area).
fn grid(svg: &mut Svg, a: &Area, xticks: &[f64], yticks: &[f64]) {
    let mut d = String::new();
    for &t in xticks {
        let x = a.sx(t);
        // not on the left spine
        if x > a.left + 0.5 && x <= a.right() + 0.5 {
            let _ = write!(d, "M{x:.2} {:.2}V{:.2}", a.top, a.bottom());
        }
    }
    for &t in yticks {
        let y = a.sy(t);
        // not on the bottom spine
        if y >= a.top - 0.5 && y < a.bottom() - 0.5 {
            let _ = write!(d, "M{:.2} {y:.2}H{:.2}", a.left, a.right());
        }
    }
    if d.is_empty() {
        return;
    }
    let _ = writeln!(
        svg.out,
        r#"<path class="grid" d="{d}" fill="none" stroke="{}" stroke-opacity="{GRID_ALPHA}" stroke-width="{GRID_WIDTH}"/>"#,
        rgb(GRID)
    );
}

fn event_lines(svg: &mut Svg, a: &Area, events: &[AktaEvent], xs: XScale, shown: EventDisplay) {
    for e in events.iter().filter(|e| shown.shows(e)) {
        let Some((color, style)) = pd::event_style(e.kind) else {
            continue;
        };
        let x = pd::event_x(e, xs);
        if !(x >= a.xlim.0 && x <= a.xlim.1) {
            continue;
        }
        let dash = match style {
            LineStyle::Solid => "",
            LineStyle::Dash => r#" stroke-dasharray="4 3""#,
            LineStyle::DashDot => r#" stroke-dasharray="5 2.5 1.2 2.5""#,
        };
        let px = a.sx(x);
        let _ = writeln!(
            svg.out,
            r#"<line x1="{px:.2}" y1="{:.2}" x2="{px:.2}" y2="{:.2}" stroke="{}" stroke-opacity="{}" stroke-width="0.8"{dash}/>"#,
            a.top,
            a.bottom(),
            rgb(color),
            pd::EVENT_LINE_ALPHA
        );
    }
}

/// Left and bottom spines (open frame), y ticks with labels, x ticks with
/// labels only when `xlabels`.
fn axes(
    svg: &mut Svg,
    a: &Area,
    yticks: &[f64],
    ylabels: &[String],
    ycolor: Rgb,
    xticks: &[f64],
    xlabels: bool,
) {
    for (t, l) in yticks.iter().zip(ylabels) {
        let y = a.sy(*t);
        if y < a.top - 0.5 || y > a.bottom() + 0.5 {
            continue;
        }
        svg.line(a.left - TICK_LEN, y, a.left, y, SPINE, 1.0);
        svg.text(
            a.left - TICK_LEN - TICK_GAP,
            y + FS_TICK * 0.36,
            l,
            FS_TICK,
            Weight::Regular,
            ycolor,
            "end",
        );
    }
    let xl = pd::tick_labels(xticks);
    for (t, l) in xticks.iter().zip(&xl) {
        let x = a.sx(*t);
        if x < a.left - 0.5 || x > a.right() + 0.5 {
            continue;
        }
        svg.line(x, a.bottom(), x, a.bottom() + TICK_LEN, SPINE, 1.0);
        if xlabels {
            let y = a.bottom() + TICK_LEN + TICK_GAP + FS_TICK * 0.75;
            svg.text(x, y, l, FS_TICK, Weight::Regular, MUTED, "middle");
        }
    }
    svg.line(a.left, a.top, a.left, a.bottom() + 0.5, SPINE, 1.0);
    svg.line(a.left - 0.5, a.bottom(), a.right(), a.bottom(), SPINE, 1.0);
}

fn xlabel(svg: &mut Svg, a: &Area, label: &str) {
    let y = a.bottom() + TICK_LEN + TICK_GAP + FS_TICK + 8.0 + FS_LABEL * 0.75;
    svg.text(
        a.left + a.w / 2.0,
        y,
        label,
        FS_LABEL,
        Weight::Regular,
        MUTED,
        "middle",
    );
}

/// A valid (lo, hi) with hi > lo, else `fallback`.
fn valid(lim: (f64, f64), fallback: (f64, f64)) -> (f64, f64) {
    if lim.0.is_finite() && lim.1.is_finite() && lim.1 > lim.0 {
        lim
    } else {
        fallback
    }
}

/// One overlay trace, ready to draw.
struct OverlayTrace {
    ch: Channel,
    scaled: Vec<Pt>,
    range: (f64, f64),
}

/// The figure as an SVG document (stacked panels, then the overlay below them).
pub fn render_svg(req: &ExportRequest) -> String {
    let xs = req.xs;
    let panels: Vec<PanelSpec> = if req.panels.is_empty() {
        vec![PanelSpec {
            channel: Channel::Uv,
            ylim: (f64::NAN, f64::NAN),
        }]
    } else {
        req.panels.clone()
    };
    let n = panels.len();
    // page: 1000 × (200·n + 500) px for n panels plus the overlay; the overlay legend goes below that
    let h_plots = 200.0 * n as f64 + 500.0;

    // Data, in x units: the full resolution of every visible channel.
    let points: Vec<Vec<Pt>> = panels
        .iter()
        .map(|p| pd::channel_points(p.channel, &req.trend, &req.events, xs))
        .collect();
    let data_x = points
        .iter()
        .flatten()
        .map(|p| p[0])
        .chain(req.trend.iter().map(|p| pd::trend_x(p, xs)))
        .filter(|x| x.is_finite())
        .fold(None, |acc: Option<(f64, f64)>, x| {
            Some(acc.map_or((x, x), |(a, b)| (a.min(x), b.max(x))))
        });
    let xfall = data_x.map_or((0.0, 1.0), |(a, b)| pd::padded(a, b, 0.0));
    let xlim = valid(req.xlim, xfall);
    let xticks = pd::nice_ticks(xlim.0, xlim.1, 8);

    // y ranges and ticks of the stacked panels
    let ylims: Vec<(f64, f64)> = panels
        .iter()
        .zip(&points)
        .map(|(p, pts)| valid(p.ylim, pd::auto_ylim(p.channel, pd::y_extrema(pts))))
        .collect();
    let yticks: Vec<Vec<f64>> = ylims.iter().map(|l| pd::nice_ticks(l.0, l.1, 4)).collect();
    let ylabels: Vec<Vec<String>> = yticks.iter().map(|t| pd::tick_labels(t)).collect();

    // Overlay: each channel scaled to 0–100 % of its whole range.
    let traces: Vec<OverlayTrace> = panels
        .iter()
        .zip(&points)
        .filter(|(_, pts)| !pts.is_empty())
        .map(|(p, pts)| {
            let (scaled, range) = pd::normalise(pts, p.channel, None);
            OverlayTrace {
                ch: p.channel,
                scaled,
                range,
            }
        })
        .collect();
    let ov_ylim = valid(req.overlay_ylim, pd::OVERLAY_YLIMS);
    // A highlighted trace that is not shown cannot stay highlighted (as on screen).
    let highlighted = req
        .highlighted
        .and_then(|h| traces.iter().find(|t| t.ch == h));
    let hl = highlighted.map(|t| t.ch);
    let (ov_ticks, ov_labels, ov_ylabel, ov_color) = match highlighted {
        Some(t) => {
            let (pos, labels) = pd::overlay_real_ticks(t.range, ov_ylim);
            (pos, labels, t.ch.overlay_ylabel(), t.ch.light_color())
        }
        None => {
            let t = pd::nice_ticks(ov_ylim.0, ov_ylim.1, 6);
            let l = pd::tick_labels(&t);
            (t, l, "Scaled (% of range)".to_string(), MUTED)
        }
    };

    // Same room for the y tick labels in every panel, so the plots line up.
    let tick_w = ylabels
        .iter()
        .flatten()
        .chain(&ov_labels)
        .map(|l| text_width(l, FS_TICK, Weight::Regular))
        .fold(16.0, f64::max);
    let left = PAD + FS_LABEL + 10.0 + tick_w + TICK_GAP + TICK_LEN;
    let right = W - PAD - 10.0;
    let pw = right - left;

    // Overlay legend below the overlay's x label, in rows as wide as the plot.
    let legend = legend_layout(&traces, hl, pw);
    let h_total = h_plots + legend.height;

    let caption = caption(req);
    let top = PAD + if caption.is_some() { TITLE_H } else { 0.0 };
    let avail = h_plots - top - PAD - BLOCK_GAP;
    let ov_cell = avail * 500.0 / h_plots;
    let st_cell = avail - ov_cell;
    let ph = ((st_cell - XDECO - PANEL_GAP * (n as f64 - 1.0)) / n as f64).max(40.0);

    let mut svg = Svg {
        out: String::with_capacity(1 << 16),
        clips: 0,
    };
    let _ = writeln!(
        svg.out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{W}" height="{h_total}" viewBox="0 0 {W} {h_total}">"#
    );
    let _ = writeln!(
        svg.out,
        r##"<rect x="0" y="0" width="{W}" height="{h_total}" fill="#ffffff"/>"##
    );

    if let Some((title, sub)) = &caption {
        let logo = 22.0;
        let _ = writeln!(
            svg.out,
            r#"<image x="{PAD}" y="{PAD}" width="{logo}" height="{logo}" xlink:href="data:image/png;base64,{}"/>"#,
            base64(LOGO_PNG)
        );
        let tx = PAD + logo + 8.0;
        let base = PAD + logo / 2.0 + 13.0 * 0.36;
        svg.text(tx, base, title, 13.0, Weight::SemiBold, TEXT, "start");
        if !sub.is_empty() {
            let x2 = tx + text_width(title, 13.0, Weight::SemiBold) + 12.0;
            svg.text(
                x2,
                base,
                sub,
                FS_TICK + 1.0,
                Weight::Regular,
                MUTED,
                "start",
            );
        }
    }

    // ── Stacked panels ────────────────────────────────────────────────────────
    let spans = pd::fraction_spans(&req.fractions, &req.trend, xs);
    let mut y = top;
    for (i, p) in panels.iter().enumerate() {
        let a = Area {
            left,
            top: y,
            w: pw,
            h: ph,
            xlim,
            ylim: ylims[i],
        };
        let bottom = i + 1 == n;
        svg.begin_clip(&a);
        if matches!(p.channel, Channel::Uv | Channel::Pctb) {
            fraction_spans(&mut svg, &a, &spans);
        }
        grid(&mut svg, &a, &xticks, &yticks[i]);
        if p.channel == Channel::Uv {
            event_lines(&mut svg, &a, &req.events, xs, req.events_shown);
        }
        trace(
            &mut svg,
            &a,
            &points[i],
            p.channel == Channel::Flow,
            p.channel.light_color(),
            1.0,
            LINEWIDTH,
        );
        if p.channel == Channel::Pctb {
            // fraction numbers near the 0 % baseline
            for s in &spans {
                let m = s.mid();
                if m >= xlim.0 && m <= xlim.1 {
                    svg.text(
                        a.sx(m),
                        a.sy(1.0) - 2.0,
                        &s.number.to_string(),
                        FS_FRAC,
                        Weight::SemiBold,
                        pd::FRAC_LABEL_COLOR,
                        "middle",
                    );
                }
            }
        }
        svg.end_clip();
        axes(
            &mut svg,
            &a,
            &yticks[i],
            &ylabels[i],
            MUTED,
            &xticks,
            bottom,
        );
        svg.vtext(
            PAD + FS_LABEL * 0.8,
            a.top + a.h / 2.0,
            p.channel.axis_label(),
            FS_LABEL,
            MUTED,
        );
        if bottom {
            xlabel(&mut svg, &a, pd::xlabel(xs));
        }
        y += ph + PANEL_GAP;
    }

    // ── Overlay ───────────────────────────────────────────────────────────────
    let ov_top = top + st_cell + BLOCK_GAP;
    let a = Area {
        left,
        top: ov_top,
        w: pw,
        h: (ov_cell - XDECO).max(60.0),
        xlim,
        ylim: ov_ylim,
    };
    svg.begin_clip(&a);
    fraction_spans(&mut svg, &a, &spans);
    grid(&mut svg, &a, &xticks, &ov_ticks);
    event_lines(&mut svg, &a, &req.events, xs, req.events_shown);
    // the highlighted trace last, so it is on top
    let mut order: Vec<&OverlayTrace> = traces.iter().collect();
    order.sort_by_key(|t| Some(t.ch) == hl);
    for t in order {
        let (alpha, width) = overlay_style(t.ch, hl);
        trace(
            &mut svg,
            &a,
            &t.scaled,
            t.ch == Channel::Flow,
            t.ch.light_color(),
            alpha,
            width,
        );
    }
    svg.end_clip();
    axes(&mut svg, &a, &ov_ticks, &ov_labels, ov_color, &xticks, true);
    svg.vtext(
        PAD + FS_LABEL * 0.8,
        a.top + a.h / 2.0,
        &ov_ylabel,
        FS_LABEL,
        ov_color,
    );
    xlabel(&mut svg, &a, pd::xlabel(xs));
    draw_legend(&mut svg, &a, &legend, &traces, hl);

    svg.out.push_str("</svg>\n");
    svg.out
}

fn overlay_style(ch: Channel, highlighted: Option<Channel>) -> (f32, f64) {
    let dim = highlighted.is_some_and(|h| h != ch);
    let alpha = if dim { pd::OVERLAY_DIM_ALPHA } else { 1.0 };
    let width = if highlighted == Some(ch) {
        pd::OVERLAY_HIGHLIGHT_LINEWIDTH
    } else {
        pd::OVERLAY_LINEWIDTH
    };
    (alpha, width)
}

/// The overlay legend, laid out: entries (trace index, x offset from the
/// plot's left edge, row) and the height it adds below the page.
struct Legend {
    labels: Vec<String>,
    entries: Vec<(usize, f64, usize)>,
    rows: usize,
    height: f64,
}

const LEGEND_PATCH: f64 = 22.0;
const LEGEND_PATCH_GAP: f64 = 7.0;
/// between two entries of a row
const LEGEND_SPACING: f64 = 26.0;
const LEGEND_ROW_H: f64 = 18.0;
/// between the overlay's x label and the first legend row
const LEGEND_TOP: f64 = 10.0;

/// Each trace with its range, in rows under the overlay (centred, wrapped to
/// `width`), so the legend never covers data (as it would inside the plot area).
fn legend_layout(traces: &[OverlayTrace], highlighted: Option<Channel>, width: f64) -> Legend {
    let labels: Vec<String> = traces
        .iter()
        .map(|t| pd::overlay_label(t.ch.name(), t.ch.unit(), t.range).replacen("━━  ", "", 1))
        .collect();
    let widths: Vec<f64> = traces
        .iter()
        .zip(&labels)
        .map(|(t, l)| {
            let weight = if highlighted == Some(t.ch) {
                Weight::Medium
            } else {
                Weight::Regular
            };
            LEGEND_PATCH + LEGEND_PATCH_GAP + text_width(l, FS_LEGEND, weight)
        })
        .collect();
    // greedy rows, then each row centred
    let mut rows: Vec<Vec<usize>> = Vec::new();
    let mut row_w = 0.0;
    for (i, w) in widths.iter().enumerate() {
        match rows.last_mut() {
            Some(r) if row_w + LEGEND_SPACING + w <= width => {
                r.push(i);
                row_w += LEGEND_SPACING + w;
            }
            _ => {
                rows.push(vec![i]);
                row_w = *w;
            }
        }
    }
    let mut entries = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        let total: f64 =
            row.iter().map(|&i| widths[i]).sum::<f64>() + LEGEND_SPACING * (row.len() as f64 - 1.0);
        let mut x = ((width - total) / 2.0).max(0.0);
        for &i in row {
            entries.push((i, x, r));
            x += widths[i] + LEGEND_SPACING;
        }
    }
    let height = if rows.is_empty() {
        0.0
    } else {
        LEGEND_TOP + LEGEND_ROW_H * rows.len() as f64
    };
    Legend {
        labels,
        entries,
        rows: rows.len(),
        height,
    }
}

fn draw_legend(
    svg: &mut Svg,
    a: &Area,
    legend: &Legend,
    traces: &[OverlayTrace],
    highlighted: Option<Channel>,
) {
    if legend.rows == 0 {
        return;
    }
    // first row just below the x label
    let top = a.bottom() + XDECO + LEGEND_TOP;
    for &(i, dx, row) in &legend.entries {
        let t = &traces[i];
        let x = a.left + dx;
        let cy = top + LEGEND_ROW_H * row as f64 + LEGEND_ROW_H / 2.0;
        let (alpha, width) = overlay_style(t.ch, highlighted);
        let _ = writeln!(
            svg.out,
            r#"<line x1="{x:.2}" y1="{cy:.2}" x2="{:.2}" y2="{cy:.2}" stroke="{}" stroke-opacity="{alpha}" stroke-width="{width}" stroke-linecap="round"/>"#,
            x + LEGEND_PATCH,
            rgb(t.ch.light_color())
        );
        let weight = if highlighted == Some(t.ch) {
            Weight::Medium
        } else {
            Weight::Regular
        };
        let tx = x + LEGEND_PATCH + LEGEND_PATCH_GAP;
        svg.text(
            tx,
            cy + FS_LEGEND * 0.36,
            &legend.labels[i],
            FS_LEGEND,
            weight,
            TEXT,
            "start",
        );
    }
}

/// Title line: the request's title (log file name) and, small and muted, the
/// sample and column from the run information.
fn caption(req: &ExportRequest) -> Option<(String, String)> {
    let title = req.title.as_ref().filter(|t| !t.trim().is_empty())?;
    let info = rc_core::runinfo::run_info(&req.events);
    let mut parts = Vec::new();
    if let Some(s) = info.get("Sample").filter(|s| !s.is_empty()) {
        parts.push(format!("Sample {s}"));
    }
    if let Some(c) = info.get("Column").filter(|s| !s.is_empty()) {
        parts.push(format!("Column {c}"));
    }
    if let Some(m) = info.get("PC method").filter(|s| !s.is_empty()) {
        parts.push(format!("Method {m}"));
    }
    Some((title.trim().to_string(), parts.join("  ·  ")))
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let v = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for k in 0..4 {
            if k <= c.len() {
                s.push(T[((v >> (18 - 6 * k)) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

#[cfg(test)]
mod tests;
