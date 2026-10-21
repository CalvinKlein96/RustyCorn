//! What part of the data the plots show: follow-live autoscale and the frozen
//! limits after a user zoom or pan (rectangle zoom, drag pan, scroll zoom). Pure
//! logic, no egui types, so it is unit tested.

use rc_plotdata::{Channel, OVERLAY_YLIMS, auto_ylim};

/// One plot area: a stacked panel or the overlay axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pane {
    Chan(Channel),
    Overlay,
}

/// Axis limits of every pane; x is shared by all of them (linked axes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    pub x: (f64, f64),
    /// stacked panels, indexed by `Channel as usize`
    pub y: [(f64, f64); 6],
    pub ov: (f64, f64),
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            x: (0.0, 1.0),
            y: [(0.0, 1.0); 6],
            ov: OVERLAY_YLIMS,
        }
    }
}

impl Limits {
    pub fn y_of(&self, pane: Pane) -> (f64, f64) {
        match pane {
            Pane::Chan(ch) => self.y[ch as usize],
            Pane::Overlay => self.ov,
        }
    }

    fn y_mut(&mut self, pane: Pane) -> &mut (f64, f64) {
        match pane {
            Pane::Chan(ch) => &mut self.y[ch as usize],
            Pane::Overlay => &mut self.ov,
        }
    }
}

/// Data extents the autoscale fits: x over the drawn traces, y per channel.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Extents {
    pub x: Option<(f64, f64)>,
    pub y: [Option<(f64, f64)>; 6],
}

/// Full-extent autoscale of every pane: 5 %
/// margins; %B keeps its fixed range, flow starts at 0, the overlay keeps
/// OVERLAY_YLIMS. The same rules as the PDF export (`rc_plotdata::auto_ylim`).
pub fn auto_limits(ext: &Extents) -> Limits {
    let x = match ext.x {
        Some((lo, hi)) if lo.is_finite() && hi.is_finite() => rc_plotdata::padded(lo, hi, 0.05),
        _ => (0.0, 1.0),
    };
    let mut lim = Limits {
        x,
        ..Limits::default()
    };
    for ch in Channel::ALL {
        lim.y[ch as usize] = auto_ylim(ch, ext.y[ch as usize].map(flat_below_f32));
    }
    lim.ov = OVERLAY_YLIMS;
    lim
}

/// A range narrower than single precision resolves is flat (a baseline with
/// 1e-10 noise must not fill the panel).
pub(crate) fn flat_below_f32((lo, hi): (f64, f64)) -> (f64, f64) {
    if hi - lo <= lo.abs().max(hi.abs()) * 1e-6 {
        let m = (lo + hi) / 2.0;
        (m, m)
    } else {
        (lo, hi)
    }
}

fn ordered((a, b): (f64, f64)) -> (f64, f64) {
    if a <= b { (a, b) } else { (b, a) }
}

/// Follow-live state machine. While following, the limits track the data on
/// every refresh; a user zoom or pan freezes them, so a zoomed view stays put
/// while the run continues; Ctrl-click (or the toolbar toggle) resumes.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewState {
    pub follow: bool,
    pub lim: Limits,
}

impl Default for ViewState {
    fn default() -> Self {
        ViewState {
            follow: true,
            lim: Limits::default(),
        }
    }
}

/// Smallest range a zoom may produce, relative to the value magnitude, so the
/// axis never collapses to a point.
const MIN_SPAN_REL: f64 = 1e-9;

fn valid_span((lo, hi): (f64, f64)) -> bool {
    let span = hi - lo;
    span.is_finite() && span > MIN_SPAN_REL * lo.abs().max(hi.abs()).max(1e-12)
}

impl ViewState {
    /// While following, take the autoscaled limits (called every frame).
    pub fn track(&mut self, auto: &Limits) {
        if self.follow {
            self.lim = *auto;
        }
    }

    /// Resume following: the next `track` rescales everything.
    pub fn resume(&mut self) {
        self.follow = true;
    }

    /// Stop following and keep the limits on screen.
    pub fn freeze(&mut self) {
        self.follow = false;
    }

    /// Rectangle zoom in `pane`: x for every pane (linked), y for that pane only.
    pub fn zoom_to(&mut self, pane: Pane, x: (f64, f64), y: (f64, f64)) {
        let (x, y) = (ordered(x), ordered(y));
        if !valid_span(x) || !valid_span(y) {
            return;
        }
        self.follow = false;
        self.lim.x = x;
        *self.lim.y_mut(pane) = y;
    }

    /// Drag pan by a data-unit offset (x shared, y of that pane).
    pub fn pan(&mut self, pane: Pane, dx: f64, dy: f64) {
        if !(dx.is_finite() && dy.is_finite()) {
            return;
        }
        self.follow = false;
        self.lim.x = (self.lim.x.0 + dx, self.lim.x.1 + dx);
        let y = self.lim.y_mut(pane);
        *y = (y.0 + dy, y.1 + dy);
    }

    /// Scroll zoom around `(cx, cy)` in `pane`; factors < 1 zoom in.
    pub fn zoom_at(&mut self, pane: Pane, (cx, cy): (f64, f64), fx: f64, fy: f64) {
        let scale = |(lo, hi): (f64, f64), c: f64, f: f64| (c + (lo - c) * f, c + (hi - c) * f);
        let x = scale(self.lim.x, cx, fx);
        let y = scale(self.lim.y_of(pane), cy, fy);
        if !valid_span(x) || !valid_span(y) {
            return;
        }
        self.follow = false;
        self.lim.x = x;
        *self.lim.y_mut(pane) = y;
    }

    /// The x-window to downsample: everything while following, else the visible range.
    pub fn xwin(&self) -> (f64, f64) {
        if self.follow {
            (f64::NEG_INFINITY, f64::INFINITY)
        } else {
            self.lim.x
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ext() -> Extents {
        let mut e = Extents {
            x: Some((0.0, 10.0)),
            ..Extents::default()
        };
        e.y[Channel::Uv as usize] = Some((0.0, 2.0));
        e.y[Channel::Pctb as usize] = Some((0.0, 50.0));
        e.y[Channel::Flow as usize] = Some((1.0, 5.0));
        e.y[Channel::Cond as usize] = Some((3.0, 3.0));
        e
    }

    #[test]
    fn autoscale_margins_and_fixed_ranges() {
        let lim = auto_limits(&ext());
        assert_eq!(lim.x, (-0.5, 10.5));
        assert_eq!(lim.y[Channel::Uv as usize], (-0.1, 2.1));
        assert_eq!(lim.y[Channel::Pctb as usize], rc_plotdata::PCTB_YLIMS);
        let (lo, hi) = lim.y[Channel::Flow as usize];
        assert_eq!(lo, 0.0);
        assert!((hi - 5.2).abs() < 1e-12, "{hi}");
        // zero-width range widened around the value
        let (lo, hi) = lim.y[Channel::Cond as usize];
        assert!(lo < 3.0 && hi > 3.0 && ((lo + hi) / 2.0 - 3.0).abs() < 1e-12);
        assert_eq!(lim.ov, OVERLAY_YLIMS);
        // no data: a unit range
        let none = auto_limits(&Extents::default());
        assert_eq!(none.x, (0.0, 1.0));
        assert_eq!(
            none.y[Channel::Flow as usize],
            rc_plotdata::auto_ylim(Channel::Flow, None)
        );
    }

    #[test]
    fn float_noise_is_flat() {
        let mut e = Extents::default();
        e.y[Channel::Uv as usize] = Some((0.02, 0.02 + 1.5e-10));
        let (lo, hi) = auto_limits(&e).y[Channel::Uv as usize];
        assert!(hi - lo > 1e-4, "{lo}..{hi}");
    }

    #[test]
    fn follow_tracks_until_user_zoom() {
        let mut v = ViewState::default();
        let a = auto_limits(&ext());
        v.track(&a);
        assert!(v.follow);
        assert_eq!(v.lim, a);
        assert_eq!(v.xwin(), (f64::NEG_INFINITY, f64::INFINITY));

        v.zoom_to(Pane::Chan(Channel::Uv), (4.0, 2.0), (0.5, 1.0));
        assert!(!v.follow);
        assert_eq!(v.lim.x, (2.0, 4.0));
        assert_eq!(v.lim.y[Channel::Uv as usize], (0.5, 1.0));
        // other panels keep their y, the x is linked
        assert_eq!(v.lim.y[Channel::Flow as usize], a.y[Channel::Flow as usize]);
        assert_eq!(v.xwin(), (2.0, 4.0));

        // new data while frozen: limits stay put
        let mut e2 = ext();
        e2.x = Some((0.0, 20.0));
        v.track(&auto_limits(&e2));
        assert_eq!(v.lim.x, (2.0, 4.0));

        // Ctrl-click resumes and rescales
        v.resume();
        v.track(&auto_limits(&e2));
        assert!(v.follow);
        assert_eq!(v.lim.x, (-1.0, 21.0));
    }

    #[test]
    fn pan_and_scroll_zoom() {
        let mut v = ViewState::default();
        v.track(&auto_limits(&ext()));
        let y0 = v.lim.ov;
        v.pan(Pane::Overlay, 1.0, 2.0);
        assert!(!v.follow);
        assert_eq!(v.lim.x, (0.5, 11.5));
        assert_eq!(v.lim.ov, (y0.0 + 2.0, y0.1 + 2.0));

        let mut v = ViewState::default();
        v.track(&auto_limits(&ext()));
        v.zoom_at(Pane::Chan(Channel::Uv), (5.0, 1.0), 0.5, 0.5);
        assert_eq!(v.lim.x, (2.25, 7.75));
        let (lo, hi) = v.lim.y[Channel::Uv as usize];
        assert!((lo - 0.45).abs() < 1e-12 && (hi - 1.55).abs() < 1e-12);
    }

    #[test]
    fn degenerate_zooms_are_ignored() {
        let mut v = ViewState::default();
        let a = auto_limits(&ext());
        v.track(&a);
        v.zoom_to(Pane::Chan(Channel::Uv), (3.0, 3.0), (0.0, 1.0));
        assert!(v.follow, "a zero-width box must not freeze the view");
        assert_eq!(v.lim, a);
        v.zoom_to(Pane::Chan(Channel::Uv), (f64::NAN, 3.0), (0.0, 1.0));
        assert_eq!(v.lim, a);
        v.pan(Pane::Chan(Channel::Uv), f64::INFINITY, 0.0);
        assert_eq!(v.lim, a);
    }
}
