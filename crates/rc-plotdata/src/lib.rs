//! Plot model shared by the live run view and the PDF export: trace points,
//! event labels, fraction spans, axis ticks. No GUI types here: points are
//! `[x, y]` in data units, colours are `[r, g, b]` floats 0–1.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use rc_core::logtext::{flow_from_log, set_buffer_valve_from_log, set_inject_from_log};
use rc_core::{AktaEvent, EventKind, Fraction, InjectValue, InstrumentState, TrendPoint};
use regex::Regex;

pub type Pt = [f64; 2];
pub type Rgb = [f32; 3];

/// The x axis scale `xs`: 0 = time in min, otherwise volume
/// divided by `xs` — 1 for mL, the column volume for column volumes (CV).
pub type XScale = f64;
pub const XS_TIME: XScale = 0.0;
pub const XS_VOLUME: XScale = 1.0;

pub fn x(time_ms: f64, vol_ml: f64, xs: XScale) -> f64 {
    if xs == 0.0 {
        time_ms / 60_000.0
    } else {
        vol_ml / xs
    }
}
pub fn trend_x(p: &TrendPoint, xs: XScale) -> f64 {
    x(p.cum_time_ms, p.cum_vol_ml, xs)
}
pub fn event_x(e: &AktaEvent, xs: XScale) -> f64 {
    x(e.cum_time_ms, e.cum_vol_ml, xs)
}
pub fn xlabel(xs: XScale) -> &'static str {
    if xs == 0.0 {
        "Time (min)"
    } else if xs == 1.0 {
        "Volume (mL)"
    } else {
        "Column volumes (CV)"
    }
}

/// A plotted channel. Flow is not measured: it is the set flow from the
/// "Flow … ml/min" log messages, drawn as steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Channel {
    Uv,
    Pctb,
    Cond,
    Ph,
    Pres,
    Flow,
}

impl Channel {
    pub const ALL: [Channel; 6] = [
        Channel::Uv,
        Channel::Pctb,
        Channel::Cond,
        Channel::Ph,
        Channel::Pres,
        Channel::Flow,
    ];
    /// Short name (overlay legend): "UV", "%B", "Cond", "pH", "Pressure", "Flow".
    pub fn name(self) -> &'static str {
        match self {
            Channel::Uv => "UV",
            Channel::Pctb => "%B",
            Channel::Cond => "Cond",
            Channel::Ph => "pH",
            Channel::Pres => "Pressure",
            Channel::Flow => "Flow",
        }
    }
    /// Unit: "AU", "%", "mS/cm", "", "MPa", "mL/min".
    pub fn unit(self) -> &'static str {
        match self {
            Channel::Uv => "AU",
            Channel::Pctb => "%",
            Channel::Cond => "mS/cm",
            Channel::Ph => "",
            Channel::Pres => "MPa",
            Channel::Flow => "mL/min",
        }
    }
    /// Axis label of the stacked panel: "UV (AU)", "%B", "Conductivity (mS/cm)", "pH",
    /// "Pressure (MPa)", "Flow (mL/min)".
    pub fn axis_label(self) -> &'static str {
        match self {
            Channel::Uv => "UV (AU)",
            Channel::Pctb => "%B",
            Channel::Cond => "Conductivity (mS/cm)",
            Channel::Ph => "pH",
            Channel::Pres => "Pressure (MPa)",
            Channel::Flow => "Flow (mL/min)",
        }
    }
    /// Name in Settings `start_panels` / the panel toggles (None for UV, always shown).
    pub fn panel_name(self) -> Option<&'static str> {
        match self {
            Channel::Uv => None,
            Channel::Pctb => Some("%B"),
            Channel::Cond => Some("Cond"),
            Channel::Ph => Some("pH"),
            Channel::Pres => Some("Pressure"),
            Channel::Flow => Some("Flow"),
        }
    }
    /// Measured value of a trend point (None for Flow).
    pub fn value(self, p: &TrendPoint) -> Option<f64> {
        match self {
            Channel::Uv => Some(p.uv),
            Channel::Pctb => Some(p.pctb),
            Channel::Cond => Some(p.cond_mscm),
            Channel::Ph => Some(p.ph),
            Channel::Pres => Some(p.pres_mpa),
            Channel::Flow => None,
        }
    }
    /// Overlay y-axis label when this trace is highlighted:
    /// "UV (AU)", "%B (%)", "Cond (mS/cm)", "pH", "Pressure (MPa)", "Flow (mL/min)".
    pub fn overlay_ylabel(self) -> String {
        channel_ylabel(self.name(), self.unit())
    }
    /// Trace colour of the light palette (rc-theme's light mode), used on paper (PDF export).
    pub fn light_color(self) -> Rgb {
        match self {
            Channel::Uv => [0.15, 0.35, 0.70],
            Channel::Pctb => [0.10, 0.55, 0.25],
            Channel::Cond => [0.80, 0.20, 0.10],
            Channel::Ph => [0.55, 0.25, 0.65],
            Channel::Pres => [0.75, 0.45, 0.00],
            Channel::Flow => [0.10, 0.45, 0.55],
        }
    }
}

/// Axis label: "name (unit)", or just the name without a unit.
pub fn channel_ylabel(name: &str, unit: &str) -> String {
    if unit.is_empty() {
        name.to_string()
    } else {
        format!("{name} ({unit})")
    }
}

/// A trace is drawn with at most 2 × DISPLAY_BINS points (min/max per bucket).
pub const DISPLAY_BINS: usize = 1500;
/// Overlay y-range: headroom above 100 %.
pub const OVERLAY_YLIMS: (f64, f64) = (-3.0, 110.0);
/// %B panel y-range: margin so the trace is visible at 0 %B and 100 %B.
pub const PCTB_YLIMS: (f64, f64) = (-3.0, 103.0);
pub const EVENT_LABEL_FONTSIZE: f64 = 8.0;
/// Fraction span fills on paper (PDF), alternating from tube to tube: a light
/// and a slightly darker cool grey at a low alpha, so neighbouring tubes stay
/// apart without tinting the traces. On white they come out at about 245 and 238 of 255. The screen
/// colours are rc-theme's `Palette::frac_fill` (one set per light/dark mode).
pub const FRAC_COLORS: [Rgb; 2] = [[0.70, 0.72, 0.75], [0.50, 0.52, 0.56]];
pub const FRAC_ALPHA: f32 = 0.14;
/// Event lines are drawn with this alpha over their colour.
pub const EVENT_LINE_ALPHA: f32 = 0.55;
/// Fraction numbers on the %B panel: pure blue, 12 pt bold.
pub const FRAC_LABEL_COLOR: Rgb = [0.0, 0.0, 1.0];
/// Overlay: alpha of the traces that are not highlighted, and the line widths.
pub const OVERLAY_DIM_ALPHA: f32 = 0.2;
pub const OVERLAY_LINEWIDTH: f64 = 1.5;
pub const OVERLAY_HIGHLIGHT_LINEWIDTH: f64 = 3.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineStyle {
    Solid,
    Dash,
    DashDot,
}

// Named colours, with the CSS / X11 values of their names.
const HOTPINK: Rgb = [1.0, 105.0 / 255.0, 180.0 / 255.0];
const GREEN: Rgb = [0.0, 128.0 / 255.0, 0.0];
const TEAL: Rgb = [0.0, 128.0 / 255.0, 128.0 / 255.0];
const GREY50: Rgb = [127.0 / 255.0, 127.0 / 255.0, 127.0 / 255.0];
const BLUE: Rgb = [0.0, 0.0, 1.0];
const DARKORANGE: Rgb = [1.0, 140.0 / 255.0, 0.0];
const RED: Rgb = [1.0, 0.0, 0.0];

/// Colour and line style of an event line; None = not drawn (Info).
pub fn event_style(kind: EventKind) -> Option<(Rgb, LineStyle)> {
    Some(match kind {
        EventKind::Inject => (HOTPINK, LineStyle::Solid),
        EventKind::RunState => (GREEN, LineStyle::Dash),
        EventKind::Step => (TEAL, LineStyle::DashDot),
        EventKind::Tubefeed => (GREY50, LineStyle::Solid),
        EventKind::Log => (BLUE, LineStyle::Solid),
        EventKind::Connect => (GREEN, LineStyle::Solid),
        EventKind::Disconnect => (RED, LineStyle::Solid),
        EventKind::Mark => (DARKORANGE, LineStyle::Solid),
        // run information is shown in the run log, not as a line
        EventKind::Info => return None,
    })
}

/// Which events the plots show (run page toolbar, PDF export). Fraction spans
/// and numbers are drawn in every setting; the run log always lists everything.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum EventDisplay {
    #[default]
    All,
    /// everything but the method steps (`is_step_like`)
    NoSteps,
    /// no event lines or labels at all
    None,
}

impl EventDisplay {
    pub const ALL: [EventDisplay; 3] =
        [EventDisplay::All, EventDisplay::NoSteps, EventDisplay::None];
    pub fn label(self) -> &'static str {
        match self {
            EventDisplay::All => "All",
            EventDisplay::NoSteps => "No steps",
            EventDisplay::None => "None",
        }
    }
    /// Is the line / label of `e` drawn?
    pub fn shows(self, e: &AktaEvent) -> bool {
        match self {
            EventDisplay::All => true,
            EventDisplay::NoSteps => !is_step_like(e),
            EventDisplay::None => false,
        }
    }
}

static RE_BREAK_POINT: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^\s*break\s*point\s+\d+\s*$"));

/// A method step: the instrument's step events (EVT_STEP, "Step t=… v=…") and
/// its "Break point N" log messages, which come with every breakpoint of the
/// method. What the step does ("Flow …", "Gradient …", valves, marks) is not a
/// step and stays visible with `EventDisplay::NoSteps`.
pub fn is_step_like(e: &AktaEvent) -> bool {
    match e.kind {
        EventKind::Step => true,
        EventKind::Log => RE_BREAK_POINT.is_match(&e.text),
        _ => false,
    }
}

/// Index of the first point at or beyond x (x grows along the run); 0-based, may be len.
pub fn first_at_or_after(trend: &[TrendPoint], x: f64, xs: XScale) -> usize {
    trend.partition_point(|p| trend_x(p, xs) < x)
}

// ── Display downsampling ──────────────────────────────────────────────────────
// A trace is drawn with at most 2 × DISPLAY_BINS points: the data in the shown
// x-window is split into DISPLAY_BINS buckets that each keep their minimum and
// maximum, so peaks look the same as with every point drawn while long runs stay
// light to draw. Recording and PDF export use the full data.

/// Downsampled points of one channel in the x-window `(xlo, xhi)` (±inf = open).
pub fn display_points(
    trend: &[TrendPoint],
    get_y: impl Fn(&TrendPoint) -> f64,
    xs: XScale,
    xwin: (f64, f64),
) -> Vec<Pt> {
    let n = trend.len();
    if n == 0 {
        return Vec::new();
    }
    let pt = |i: usize| [trend_x(&trend[i], xs), get_y(&trend[i])];
    let (xlo, xhi) = xwin;
    // The window, plus one point either side so lines run to the plot edges.
    let i0 = if xlo.is_infinite() {
        0
    } else {
        first_at_or_after(trend, xlo, xs).saturating_sub(1)
    };
    let i1 = if xhi.is_infinite() {
        n - 1
    } else {
        first_at_or_after(trend, xhi, xs).min(n - 1)
    };
    if i1 < i0 {
        return vec![pt(i0.min(n - 1))];
    }
    let m = i1 - i0 + 1;
    if m <= 2 * DISPLAY_BINS {
        return (i0..=i1).map(pt).collect();
    }
    let mut pts = Vec::with_capacity(2 * DISPLAY_BINS);
    for b in 0..DISPLAY_BINS {
        let a = i0 + (m * b) / DISPLAY_BINS;
        let z = i0 + (m * (b + 1)) / DISPLAY_BINS - 1;
        let (mut imin, mut imax) = (a, a);
        let mut ymin = get_y(&trend[a]);
        let mut ymax = ymin;
        for (i, p) in trend.iter().enumerate().take(z + 1).skip(a + 1) {
            let y = get_y(p);
            if y < ymin {
                ymin = y;
                imin = i;
            }
            if y > ymax {
                ymax = y;
                imax = i;
            }
        }
        pts.push(pt(imin.min(imax)));
        if imin != imax {
            pts.push(pt(imin.max(imax)));
        }
    }
    pts
}

/// Step-plot points of the set flow: one per flow log
/// event, with the last value held up to the newest data point.
pub fn flow_setpoints(events: &[AktaEvent], trend: &[TrendPoint], xs: XScale) -> Vec<Pt> {
    let mut pts: Vec<Pt> = events
        .iter()
        .filter(|e| e.kind == EventKind::Log)
        .filter_map(|e| flow_from_log(&e.text).map(|f| [event_x(e, xs), f]))
        .collect();
    if let (Some(&[lx, ly]), Some(last)) = (pts.last(), trend.last()) {
        let xend = trend_x(last, xs);
        if xend > lx {
            pts.push([xend, ly]);
        }
    }
    pts
}

/// Every point of a channel in x units (export; flow = set points).
pub fn channel_points(
    ch: Channel,
    trend: &[TrendPoint],
    events: &[AktaEvent],
    xs: XScale,
) -> Vec<Pt> {
    match ch {
        Channel::Flow => flow_setpoints(events, trend, xs),
        _ => trend
            .iter()
            .map(|p| [trend_x(p, xs), ch.value(p).unwrap_or(0.0)])
            .collect(),
    }
}

/// (min, max) of the y values of `pts`; None when empty. NaNs are skipped.
pub fn y_extrema(pts: &[Pt]) -> Option<(f64, f64)> {
    extrema(pts.iter().map(|p| p[1]))
}

/// (min, max) of a channel over the whole trend (the overlay
/// range); None when empty.
pub fn trend_extrema(
    trend: &[TrendPoint],
    get_y: impl Fn(&TrendPoint) -> f64,
) -> Option<(f64, f64)> {
    extrema(trend.iter().map(get_y))
}

fn extrema(vals: impl Iterator<Item = f64>) -> Option<(f64, f64)> {
    vals.filter(|v| !v.is_nan()).fold(None, |acc, v| match acc {
        None => Some((v, v)),
        Some((lo, hi)) => Some((lo.min(v), hi.max(v))),
    })
}

/// The range of a whole channel that maps to 0–100 % in the overlay (what
/// `normalise` uses when given all points): %B is 0–100, flow starts at 0.
pub fn overlay_range(
    ch: Channel,
    trend: &[TrendPoint],
    events: &[AktaEvent],
    xs: XScale,
) -> (f64, f64) {
    let (lo, hi) = match ch {
        Channel::Pctb => return (0.0, 100.0),
        Channel::Flow => y_extrema(&flow_setpoints(events, trend, xs)),
        _ => trend_extrema(trend, |p| ch.value(p).unwrap_or(0.0)),
    }
    .unwrap_or((0.0, 0.0));
    if ch == Channel::Flow {
        (0.0, hi)
    } else {
        (lo, hi)
    }
}

/// Overlay scaling to 0–100 % of the trace's range.
/// Returns the scaled points and the (lo, hi) mapping to 0–100 %.
/// %B already is a percentage; flow is scaled from 0 so it reads as a fraction
/// of the top flow. `range` = (lo, hi) of the whole trace when `pts` is only a
/// downsampled part of it.
pub fn normalise(pts: &[Pt], ch: Channel, range: Option<(f64, f64)>) -> (Vec<Pt>, (f64, f64)) {
    if ch == Channel::Pctb {
        return (pts.to_vec(), (0.0, 100.0));
    }
    let (mut lo, hi) = match range.or_else(|| y_extrema(pts)) {
        Some(r) => r,
        None => return (Vec::new(), (0.0, 0.0)),
    };
    if ch == Channel::Flow {
        lo = 0.0;
    }
    let span = hi - lo;
    let scaled = if span > 0.0 {
        pts.iter()
            .map(|p| [p[0], (p[1] - lo) / span * 100.0])
            .collect()
    } else {
        pts.iter().map(|p| [p[0], 50.0]).collect()
    };
    (scaled, (lo, hi))
}

/// Overlay legend entry: "━━  UV   (0.012–1.22 AU)".
pub fn overlay_label(name: &str, unit: &str, range: (f64, f64)) -> String {
    let unit = if unit.is_empty() {
        String::new()
    } else {
        format!(" {unit}")
    };
    format!(
        "━━  {name}   ({}–{}{unit})",
        fmt_g(range.0, 3),
        fmt_g(range.1, 3)
    )
}

/// `v` to `prec` significant digits, formatted like C printf `%.<prec>g`
/// (trailing zeros dropped, exponent signed with at least two digits), except
/// that the fixed or scientific style is chosen from the exponent at `prec` + 1
/// significant digits, so 999.5 prints as "1000", not "1e+03". Non-finite
/// values print as "NaN", "Inf" and "-Inf".
pub fn fmt_g(v: f64, prec: usize) -> String {
    if v.is_nan() {
        return "NaN".to_string();
    }
    if v.is_infinite() {
        return if v > 0.0 {
            "Inf".to_string()
        } else {
            "-Inf".to_string()
        };
    }
    let p = prec.max(1);
    if v == 0.0 {
        return if v.is_sign_negative() {
            "-0".to_string()
        } else {
            "0".to_string()
        };
    }
    let exponent = |s: &str| {
        s.split_once('e')
            .and_then(|(_, e)| e.parse::<i32>().ok())
            .unwrap_or(0)
    };
    let exp = exponent(&format!("{v:.p$e}"));
    if exp < -4 || exp >= p as i32 {
        let sci = format!("{:.*e}", p - 1, v);
        let (mant, _) = sci.split_once('e').unwrap_or((sci.as_str(), ""));
        let exp = exponent(&sci);
        let sign = if exp < 0 { '-' } else { '+' };
        format!("{}e{sign}{:02}", strip_zeros(mant), exp.abs())
    } else {
        let decimals = (p as i32 - 1 - exp).max(0) as usize;
        strip_zeros(&format!("{v:.decimals$}")).to_string()
    }
}

fn strip_zeros(s: &str) -> &str {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        s
    }
}

/// The first `n` characters (not bytes) of `s`, or all of it.
fn first_chars(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

fn re(pat: &str) -> Regex {
    Regex::new(pat).expect("valid built-in regex")
}

static RE_INJ_VALVE: LazyLock<Regex> = LazyLock::new(|| re(r"Valve\s*→\s*([A-Za-z_]+)"));
static RE_RUN_STATE_TOP: LazyLock<Regex> = LazyLock::new(|| re(r"Run state:\s*([^\s]+)"));
static RE_FRAC_SIZE_TOP: LazyLock<Regex> = LazyLock::new(|| re(r"Fraction size (\d+(?:\.\d+)?)"));
static RE_LOG_SHORT: LazyLock<[(Regex, &'static str); 5]> = LazyLock::new(|| {
    [
        (
            re(r"(?i)^(?:Manual Set;\s*)?Buffer Valve Pos\s+(\d+)"),
            "Buffer ",
        ),
        (re(r"(?i)^Injection Valve (\w+)"), "Inj valve "),
        (re(r"(?i)^Flow ([0-9.]+) ml/min"), "Flow "),
        (
            re(r"(?i)^(?:Manual Set;\s*)?Concentration ([0-9.]+) %B"),
            "%B ",
        ),
        (re(r"(?i)^Feed Tube Nr\s+(\d+)"), "Tube "),
    ]
});
static RE_FEED_TUBE_ANY: LazyLock<Regex> = LazyLock::new(|| re(r"Feed Tube Nr\s+(\d+)"));

/// Short label drawn at the top of an event line.
pub fn event_label_top(e: &AktaEvent) -> String {
    match e.kind {
        EventKind::Inject => match RE_INJ_VALVE.captures(&e.text) {
            Some(c) => format!("INJ {}", c[1].to_uppercase()),
            None => "INJECT".to_string(),
        },
        EventKind::RunState => match RE_RUN_STATE_TOP.captures(&e.text) {
            Some(c) => c[1].to_uppercase(),
            None => "STATE".to_string(),
        },
        EventKind::Step => "STEP".to_string(),
        EventKind::Log => {
            // Special handling for fraction size zero-transitions
            if e.text.contains("Fraction size")
                && let Some(c) = RE_FRAC_SIZE_TOP.captures(&e.text)
            {
                let val: f64 = c[1].parse().unwrap_or(0.0);
                return if val > 0.0 { "FRAC START" } else { "FRAC STOP" }.to_string();
            }
            let t = e.text.trim();
            for (re, short) in RE_LOG_SHORT.iter() {
                if let Some(c) = re.captures(t) {
                    return format!("{short}{}", &c[1]);
                }
            }
            if t.starts_with("Method start") {
                return "Method start".to_string();
            }
            if t.chars().count() > 20 {
                format!("{}…", first_chars(t, 18))
            } else {
                t.to_string()
            }
        }
        EventKind::Mark => {
            let txt = &e.text;
            if txt.chars().count() > 16 {
                format!("Mark: {}…", first_chars(txt, 15))
            } else {
                format!("Mark: {txt}")
            }
        }
        EventKind::Connect => "CONNECT".to_string(),
        EventKind::Disconnect => "DISCONNECT".to_string(),
        EventKind::Tubefeed | EventKind::Info => e.kind.csv_name().to_string(),
    }
}

/// Fraction number drawn near the %B baseline for a tube feed event.
pub fn fraction_label_bottom(e: &AktaEvent) -> String {
    // Just the fraction number (no "F#" prefix)
    match RE_FEED_TUBE_ANY.captures(&e.text) {
        Some(c) => c[1].to_string(),
        None => "?".to_string(),
    }
}

/// Events drawn as labelled lines on the UV panel: everything but Info and tube
/// feeds; "Fraction size" log events only on 0↔>0 transitions.
pub fn labelled_events(events: &[AktaEvent]) -> Vec<&AktaEvent> {
    let is_frac_size = |e: &AktaEvent| e.kind == EventKind::Log && e.text.contains("Fraction size");
    // Fraction volume of every parsable "Fraction size" message, to detect zero-transitions.
    // These are paired with the "Fraction size" events by position, so an
    // unparsable message shifts the pairing and the last ones read as 0
    // (pinned by the tests).
    let frac_vol_states: Vec<f64> = events
        .iter()
        .filter(|e| is_frac_size(e))
        .filter_map(|e| {
            RE_FRAC_SIZE_TOP
                .captures(&e.text)
                .and_then(|c| c[1].parse().ok())
        })
        .collect();
    let mut filtered = Vec::new();
    let mut prev = 0.0;
    let mut idx = 0;
    for e in events {
        if matches!(e.kind, EventKind::Tubefeed | EventKind::Info) {
            // tube feeds are shown as spans, run information in the run log
            continue;
        } else if is_frac_size(e) {
            let cur = frac_vol_states.get(idx).copied().unwrap_or(0.0);
            idx += 1;
            // only on zero-transitions: 0→>0 or >0→0
            if (prev == 0.0 && cur > 0.0) || (prev > 0.0 && cur == 0.0) {
                filtered.push(e);
            }
            prev = cur;
        } else {
            filtered.push(e);
        }
    }
    filtered
}

/// Screen layout of event labels: `lims` =
/// (x0, y0, width, height) of the visible data area, `px` = its size in
/// pixels, `avoid` = (left, right, top, bottom) px box measured from the top.
/// Returns label anchor points (left/top aligned) in data coordinates.
///
/// Labels start just right of their event line, in rows from the top of the
/// visible area: a label moves down a row only if it would overlap one already
/// placed. Laid out in screen pixels, so it is redone whenever the view changes.
pub fn event_label_positions(
    xs: &[f64],
    labels: &[String],
    lims: (f64, f64, f64, f64),
    px: (f64, f64),
    avoid: Option<(f64, f64, f64, f64)>,
) -> Vec<Pt> {
    let (x0, y0, w, h) = lims;
    let (pxw, pxh) = (px.0.max(1.0), px.1.max(1.0));
    let gap = 3.0; // px between line and label
    let rowh = EVENT_LABEL_FONTSIZE * 1.3; // px per row
    let mut row_end: Vec<f64> = Vec::new(); // right edge (px) of each row
    let mut pts = vec![[0.0, 0.0]; xs.len()];
    // rows are numbered from 0 at the top
    let blocked = |r: usize, start: f64, stop: f64| match avoid {
        Some((left, right, top, bottom)) => {
            let r = r as f64;
            start < right && stop > left && 2.0 + (r + 1.0) * rowh > top && 2.0 + r * rowh < bottom
        }
        None => false,
    };
    let mut order: Vec<usize> = (0..xs.len()).collect();
    order.sort_by(|&a, &b| xs[a].total_cmp(&xs[b]));
    for i in order {
        let nchars = labels.get(i).map_or(0, |l| l.chars().count()) as f64;
        let start = (xs[i] - x0) / w * pxw + gap;
        let stop = start + nchars * EVENT_LABEL_FONTSIZE * 0.72 + gap;
        let mut r = 0;
        while (r < row_end.len() && row_end[r] >= start) || blocked(r, start, stop) {
            r += 1;
        }
        if row_end.len() <= r {
            row_end.resize(r + 1, f64::NEG_INFINITY);
        }
        row_end[r] = stop;
        pts[i] = [
            xs[i] + gap / pxw * w,
            y0 + h - (2.0 + r as f64 * rowh) / pxh * h,
        ];
    }
    pts
}

/// A shaded fraction span in x units, with its fill colour and number.
#[derive(Clone, Debug, PartialEq)]
pub struct FractionSpan {
    pub x0: f64,
    pub x1: f64,
    /// paper colour, `FRAC_COLORS[shade]`
    pub color: Rgb,
    /// 0 or 1, alternating along the fraction list: which of the two fills
    pub shade: usize,
    pub number: i64,
}

impl FractionSpan {
    /// Where the fraction number is drawn (centre of the span).
    pub fn mid(&self) -> f64 {
        (self.x0 + self.x1) / 2.0
    }
}

/// Spans of the fractions; an open one runs to the newest data point.
pub fn fraction_spans(fracs: &[Fraction], trend: &[TrendPoint], xs: XScale) -> Vec<FractionSpan> {
    let lastx = trend.last().map_or(0.0, |p| trend_x(p, xs));
    fracs
        .iter()
        .enumerate()
        .filter_map(|(i, f)| {
            let x0 = x(f.start_time_ms, f.start_vol_ml, xs);
            let x1 = match (f.end_time_ms, f.end_vol_ml) {
                (Some(t), Some(v)) => x(t, v, xs),
                _ => lastx,
            };
            let shade = i % FRAC_COLORS.len();
            (x1 > x0).then(|| FractionSpan {
                x0,
                x1,
                color: FRAC_COLORS[shade],
                shade,
                number: f.fraction_number,
            })
        })
        .collect()
}

// In memory the event text has ", inject" with a comma, but the
// CSV writer turns them into ";", so a reopened log needs "[,;]" to match.
static RE_RUN_STATE: LazyLock<Regex> = LazyLock::new(|| re(r"Run state:\s*(\S+)\s+inject=(\w+)"));
static RE_METHOD_START: LazyLock<Regex> =
    LazyLock::new(|| re(r"^Method start:.*buffer port (\d+)[,;] inject (\w+)"));
static RE_FRAC_SIZE: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)^Fraction size\s+([0-9.]+)\s+ml$"));
static RE_FEED_TUBE: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^Feed Tube Nr\s+(\d+)$"));

/// Instrument state at a past moment, rebuilt by replaying the events
/// up to `p` with the same decoders the live link uses. Every event is in the
/// CSV, so this works for reopened logs too.
pub fn state_at(events: &[AktaEvent], p: &TrendPoint) -> InstrumentState {
    let mut s = InstrumentState::default();
    for e in events.iter().filter(|e| e.cum_time_ms <= p.cum_time_ms) {
        match e.kind {
            EventKind::RunState => {
                if let Some(c) = RE_RUN_STATE.captures(&e.text) {
                    s.run_state = c[1].to_string();
                    s.inject_value = InjectValue::from_name(&c[2]).unwrap_or(s.inject_value);
                }
            }
            EventKind::Log => apply_log_text(&e.text, &mut s),
            _ => {}
        }
    }
    s.cum_time_ms = p.cum_time_ms;
    s.run_time_ms = p.cum_time_ms;
    s.cum_vol_ml = p.cum_vol_ml;
    s.run_vol_ml = p.cum_vol_ml;
    s
}

fn apply_log_text(text: &str, s: &mut InstrumentState) {
    let t = text.trim();
    if let Some(c) = RE_METHOD_START.captures(text) {
        if let Ok(port) = c[1].parse() {
            s.buffer_port = port;
            s.buffer_port_known = true;
        }
        s.inject_value = match c[2].to_lowercase().as_str() {
            "load" => InjectValue::Load,
            "inj" => InjectValue::Inject,
            "waste" => InjectValue::Waste,
            _ => s.inject_value,
        };
    } else if let Some(c) = RE_FRAC_SIZE.captures(t) {
        if let Ok(v) = c[1].parse() {
            s.fraction_vol_ml = v;
        }
    } else if let Some(c) = RE_FEED_TUBE.captures(t) {
        if let Ok(n) = c[1].parse::<i64>() {
            s.fraction_number = n.max(1);
        }
    } else if !set_inject_from_log(text, s) {
        set_buffer_valve_from_log(text, s);
    }
}

/// Set flow (mL/min) in force at `p`, None before the first flow message.
pub fn flow_at(events: &[AktaEvent], p: &TrendPoint) -> Option<f64> {
    events
        .iter()
        .rev()
        .filter(|e| e.kind == EventKind::Log && e.cum_time_ms <= p.cum_time_ms)
        .find_map(|e| flow_from_log(&e.text))
}

/// One-line valve summary for the flow-path card;
/// ports named in Run info show their buffer.
pub fn valve_summary(s: &InstrumentState, info: &BTreeMap<String, String>) -> String {
    let name = info
        .get(&format!("Port {}", s.buffer_port))
        .map_or("", String::as_str);
    let port = if !name.is_empty() {
        format!("{} ({name})", s.buffer_port)
    } else if s.buffer_port == 1 {
        "1 (A)".to_string()
    } else if s.buffer_port == 8 {
        "8 (sample)".to_string()
    } else {
        s.buffer_port.to_string()
    };
    let outlet = if s.fraction_vol_ml > 0.0 {
        format!("frac #{}, {:.2} mL", s.fraction_number, s.fraction_vol_ml)
    } else {
        "waste".to_string()
    };
    format!(
        "Valve {} · Buffer {port} · Outlet {outlet}",
        s.inject_value.label().to_lowercase()
    )
}

// ── Ticks ─────────────────────────────────────────────────────────────────────

/// Step multipliers per decade (matplotlib MaxNLocator style).
const TICK_STEPS: [f64; 5] = [1.0, 2.0, 2.5, 5.0, 10.0];

/// "Nice" tick values covering [lo, hi], about `target` of them, spaced by 1,
/// 2, 2.5 or 5 times a power of ten (a MaxNLocator-style search); ticks lie
/// inside [lo, hi].
pub fn nice_ticks(lo: f64, hi: f64, target: usize) -> Vec<f64> {
    if !lo.is_finite() || !hi.is_finite() {
        return Vec::new();
    }
    let (vmin, vmax) = if lo <= hi { (lo, hi) } else { (hi, lo) };
    if vmin == vmax {
        return vec![vmin + 0.0];
    }
    let n = target.max(1) as f64;
    // scale_range: the decade exponents are truncated towards zero, not floored
    let dv = vmax - vmin;
    let meanv = (vmax + vmin) / 2.0;
    let offset = if meanv.abs() / dv < 100.0 {
        0.0
    } else {
        10f64.powf(meanv.abs().log10().trunc()).copysign(meanv)
    };
    let scale = 10f64.powf((dv / n).log10().trunc());
    let (lo_, hi_) = (vmin - offset, vmax - offset);
    let raw_step = (hi_ - lo_) / n;
    // staircase: the steps a decade lower (without the 10), the steps, then 10 × the second step
    let mut steps: Vec<f64> = TICK_STEPS[..TICK_STEPS.len() - 1]
        .iter()
        .map(|s| 0.1 * s)
        .collect();
    steps.extend(TICK_STEPS);
    steps.push(10.0 * TICK_STEPS[1]);
    for s in &mut steps {
        *s *= scale;
    }
    let istep = steps
        .iter()
        .position(|&s| s >= raw_step)
        .unwrap_or(steps.len() - 1);
    let tol_of = |step: f64| {
        if offset.abs() > 0.0 {
            let digits = (offset.abs() / step).log10();
            10f64.powf(digits - 12.0).clamp(1e-10, 0.4999)
        } else {
            1e-10
        }
    };
    let mut ticks: Vec<f64> = Vec::new();
    let mut step = steps[istep];
    for is in (0..=istep).rev() {
        step = steps[is];
        let best_vmin = (lo_ / step).trunc() * step;
        let tol = tol_of(step);
        // le: largest k with k*step <= x; ge: smallest k with k*step >= x
        let le = |x: f64| {
            let (d, m) = ((x / step).trunc(), x % step);
            if (m / step - 1.0).abs() < tol {
                d + 1.0
            } else {
                d
            }
        };
        let ge = |x: f64| {
            let (d, m) = ((x / step).trunc(), x % step);
            if (m / step).abs() < tol { d } else { d + 1.0 }
        };
        let (k0, k1) = (le(lo_ - best_vmin), ge(hi_ - best_vmin));
        ticks.clear();
        let mut k = k0;
        while k <= k1 && ticks.len() < 10_000 {
            ticks.push(k * step + best_vmin);
            k += 1.0;
        }
        let shown = ticks.iter().filter(|&&t| lo_ <= t && t <= hi_).count();
        if shown >= 2 {
            break;
        }
    }
    // Small tolerance: a tick that lands exactly on a limit must not be lost to rounding.
    let eps = dv * 1e-9;
    let vals: Vec<f64> = ticks
        .iter()
        .map(|t| t + offset)
        .filter(|&t| vmin - eps <= t && t <= vmax + eps)
        .collect();
    // Round off float noise (0.08000000001 → 0.08) to the digits the step needs.
    let exponent = step.abs().log10().floor() as i32;
    let digits = (-exponent + 1).max(0);
    let f = 10f64.powi(digits);
    vals.into_iter()
        .map(|v| (v * f).round() / f + 0.0)
        .collect()
}

/// Tick labels with one shared number of decimals, the fewest that show every
/// value exactly; negatives use the minus sign "−".
/// Worked out in double precision, and never fewer decimals than it takes to
/// tell neighbouring ticks apart.
pub fn tick_labels(vals: &[f64]) -> Vec<String> {
    let fmt = |v: f64, prec: usize| {
        let s = format!("{:.prec$}", v + 0.0);
        match s.strip_prefix('-') {
            Some(rest) if rest.chars().any(|c| c.is_ascii_digit() && c != '0') => {
                format!("−{rest}")
            }
            Some(rest) => rest.to_string(), // "-0.0" → "0.0"
            None => s,
        }
    };
    let finite: Vec<f64> = vals.iter().copied().filter(|v| v.is_finite()).collect();
    let scale = finite
        .iter()
        .fold(0.0f64, |m, v| m.max(v.abs()))
        .max(1e-300);
    let exact = |prec: usize| {
        let f = 10f64.powi(prec as i32);
        finite
            .iter()
            .all(|&v| ((v * f).round() / f - v).abs() <= 1e-9 * scale)
    };
    let distinct = |prec: usize| {
        let labels: Vec<String> = finite.iter().map(|&v| fmt(v, prec)).collect();
        labels
            .windows(2)
            .zip(finite.windows(2))
            .all(|(l, v)| v[0] == v[1] || l[0] != l[1])
    };
    let prec = (0..=15).find(|&p| exact(p) && distinct(p)).unwrap_or(15);
    vals.iter().map(|&v| fmt(v, prec)).collect()
}

/// Overlay y ticks in the real units of the highlighted trace: `range` =
/// (lo, hi) of the trace, which maps to
/// 0–100 %; `ylim` = the overlay's visible y-range in %. Ticks are chosen on the
/// real values (about 6) and returned as (scaled positions, labels).
pub fn overlay_real_ticks(range: (f64, f64), ylim: (f64, f64)) -> (Vec<f64>, Vec<String>) {
    let (lo, hi) = range;
    let span = hi - lo;
    if span.is_nan() || span <= 0.0 {
        return (vec![50.0], tick_labels(&[lo]));
    }
    let toreal = |v: f64| lo + v / 100.0 * span;
    let real = nice_ticks(toreal(ylim.0), toreal(ylim.1), 6);
    let pos = real.iter().map(|t| (t - lo) / span * 100.0).collect();
    (pos, tick_labels(&real))
}

/// Auto y-range of a panel from its data range, with 5 % margins; %B
/// is fixed at (-3, 103) and flow starts at 0.
pub fn auto_ylim(ch: Channel, data: Option<(f64, f64)>) -> (f64, f64) {
    if ch == Channel::Pctb {
        return PCTB_YLIMS;
    }
    let (lo, hi) = data.unwrap_or((0.0, 1.0));
    let (mut a, b) = padded(lo, hi, 0.05);
    if ch == Channel::Flow {
        a = 0.0;
    }
    if b > a { (a, b) } else { (a, a + 1.0) }
}

/// (lo, hi) widened by `margin` of its span on both sides; a zero-width range
/// is widened around its value.
pub fn padded(lo: f64, hi: f64, margin: f64) -> (f64, f64) {
    let span = hi - lo;
    if span > 0.0 {
        (lo - margin * span, hi + margin * span)
    } else {
        let d = if lo == 0.0 { 0.5 } else { lo.abs() * 0.1 };
        (lo - d, hi + d)
    }
}

#[cfg(test)]
mod tests;
