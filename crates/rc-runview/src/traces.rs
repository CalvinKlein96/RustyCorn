//! Downsampled traces for the visible x-window, cached by data version and
//! window: nothing is recomputed in a frame where neither changed. Stale
//! channels of a long run are downsampled in parallel.

use rc_core::TrendPoint;
use rc_plotdata::{Channel, Pt, XScale, display_points, normalise};

/// What a cached trace was computed from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Key {
    version: u64,
    xs: u64,
    lo: u64,
    hi: u64,
}

impl Key {
    fn new(version: u64, xs: XScale, (lo, hi): (f64, f64)) -> Key {
        Key {
            version,
            xs: xs.to_bits(),
            lo: lo.to_bits(),
            hi: hi.to_bits(),
        }
    }
}

#[derive(Default)]
struct Entry {
    key: Option<Key>,
    pts: Vec<Pt>,
    /// overlay version: scaled to 0–100 % of the range below (computed on demand)
    ov_key: Option<(Key, u64, u64)>,
    ov_pts: Vec<Pt>,
    ov_range: (f64, f64),
}

/// Traces of every channel (flow is passed in: its set points are few).
#[derive(Default)]
pub struct TraceCache {
    entries: [Entry; 6],
    /// number of channel downsamplings done (for tests and perf stats)
    pub computed: u64,
}

/// Downsampled points of a measured channel; one monomorphised getter per
/// channel so the inner loop stays tight.
fn display(trend: &[TrendPoint], ch: Channel, xs: XScale, xwin: (f64, f64)) -> Vec<Pt> {
    match ch {
        Channel::Uv => display_points(trend, |p| p.uv, xs, xwin),
        Channel::Pctb => display_points(trend, |p| p.pctb, xs, xwin),
        Channel::Cond => display_points(trend, |p| p.cond_mscm, xs, xwin),
        Channel::Ph => display_points(trend, |p| p.ph, xs, xwin),
        Channel::Pres => display_points(trend, |p| p.pres_mpa, xs, xwin),
        Channel::Flow => Vec::new(),
    }
}

/// Above this many points the stale channels are computed on worker threads.
const PARALLEL_MIN_POINTS: usize = 20_000;

impl TraceCache {
    /// Bring the measured channels in `needed` up to date for this data version
    /// and x-window. Returns true if anything was recomputed.
    pub fn update(
        &mut self,
        trend: &[TrendPoint],
        version: u64,
        xs: XScale,
        xwin: (f64, f64),
        needed: [bool; 6],
    ) -> bool {
        let key = Key::new(version, xs, xwin);
        let stale: Vec<Channel> = Channel::ALL
            .into_iter()
            .filter(|&ch| {
                ch != Channel::Flow
                    && needed[ch as usize]
                    && self.entries[ch as usize].key != Some(key)
            })
            .collect();
        if stale.is_empty() {
            return false;
        }
        if trend.is_empty() {
            for ch in &stale {
                let e = &mut self.entries[*ch as usize];
                e.pts.clear();
                e.key = Some(key);
            }
            return true;
        }
        let results: Vec<Vec<Pt>> = if stale.len() > 1 && trend.len() >= PARALLEL_MIN_POINTS {
            std::thread::scope(|s| {
                let handles: Vec<_> = stale
                    .iter()
                    .map(|&ch| s.spawn(move || display(trend, ch, xs, xwin)))
                    .collect();
                handles
                    .into_iter()
                    .map(|h| match h.join() {
                        Ok(pts) => pts,
                        Err(panic) => std::panic::resume_unwind(panic),
                    })
                    .collect()
            })
        } else {
            stale
                .iter()
                .map(|&ch| display(trend, ch, xs, xwin))
                .collect()
        };
        for (ch, pts) in stale.into_iter().zip(results) {
            let e = &mut self.entries[ch as usize];
            e.pts = pts;
            e.key = Some(key);
            self.computed += 1;
        }
        true
    }

    /// Downsampled points of a measured channel (empty until `update`).
    pub fn points(&self, ch: Channel) -> &[Pt] {
        &self.entries[ch as usize].pts
    }

    /// Overlay trace: `pts` scaled to 0–100 % of `range` (the whole run's range;
    /// None = of the points themselves, as for the set flow). Cached per input.
    pub fn overlay(
        &mut self,
        ch: Channel,
        flow: &[Pt],
        range: Option<(f64, f64)>,
        flow_version: u64,
    ) -> (&[Pt], (f64, f64)) {
        let e = &mut self.entries[ch as usize];
        let base = if ch == Channel::Flow {
            Key {
                version: flow_version,
                xs: 0,
                lo: 0,
                hi: 0,
            }
        } else {
            match e.key {
                Some(k) => k,
                None => return (&[], (0.0, 1.0)),
            }
        };
        let (rlo, rhi) = range.unwrap_or((f64::NAN, f64::NAN));
        let key = (base, rlo.to_bits(), rhi.to_bits());
        if e.ov_key != Some(key) {
            let src: &[Pt] = if ch == Channel::Flow { flow } else { &e.pts };
            if src.is_empty() {
                e.ov_pts.clear();
                e.ov_range = range.unwrap_or((0.0, 1.0));
            } else {
                let (pts, r) = normalise(src, ch, range);
                e.ov_pts = pts;
                e.ov_range = r;
            }
            e.ov_key = Some(key);
        }
        (&e.ov_pts, e.ov_range)
    }
}
