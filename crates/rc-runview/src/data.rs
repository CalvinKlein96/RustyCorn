//! The run view's own copy of the run data. The AppState lock is taken once
//! per refresh to copy what changed (usually a few new points), so drawing
//! never holds the lock and long runs are not copied every frame.

use std::path::PathBuf;

use rc_core::{AktaEvent, AppState, Fraction, InstrumentState, TrendPoint};
use rc_plotdata::Channel;

/// Local copy of the shared state, synced by generation.
#[derive(Default)]
pub struct Snapshot {
    /// AppState generation last copied (None = never)
    pub generation: Option<u64>,
    /// bumped whenever trend, events or fractions changed; caches key on it
    pub version: u64,
    pub trend: Vec<TrendPoint>,
    pub events: Vec<AktaEvent>,
    pub fractions: Vec<Fraction>,
    pub inst: InstrumentState,
    pub status: String,
    /// a CSV log is open and has received data (REC chip)
    pub recording: bool,
    /// file name (no folder) of the log
    pub log_file: String,
    pub log_dir: PathBuf,
    pub log_prefix: String,
    pub live: bool,
    /// running min/max of the measured channels over the whole trend
    pub yext: [Option<(f64, f64)>; 6],
}

fn same_point(a: &TrendPoint, b: &TrendPoint) -> bool {
    // bitwise, so a NaN reading does not force a full copy every refresh
    a.cum_time_ms.to_bits() == b.cum_time_ms.to_bits()
        && a.cum_vol_ml.to_bits() == b.cum_vol_ml.to_bits()
        && a.uv.to_bits() == b.uv.to_bits()
        && a.pctb.to_bits() == b.pctb.to_bits()
}

fn same_event(a: &AktaEvent, b: &AktaEvent) -> bool {
    a.kind == b.kind && a.cum_time_ms.to_bits() == b.cum_time_ms.to_bits() && a.text == b.text
}

/// Make `dst` equal to `src`. When `src` only grew (the usual case: new
/// points appended) just the new tail is copied. Returns the index from which
/// `dst` changed (0 = replaced), or None if it was already equal.
fn sync_vec<T: Clone>(dst: &mut Vec<T>, src: &[T], same: impl Fn(&T, &T) -> bool) -> Option<usize> {
    let n = dst.len();
    let grew =
        src.len() >= n && (n == 0 || (same(&dst[0], &src[0]) && same(&dst[n - 1], &src[n - 1])));
    if grew {
        if src.len() == n {
            return None;
        }
        dst.extend_from_slice(&src[n..]);
        Some(n)
    } else {
        dst.clear();
        dst.extend_from_slice(src);
        Some(0)
    }
}

const MEASURED: [Channel; 5] = [
    Channel::Uv,
    Channel::Pctb,
    Channel::Cond,
    Channel::Ph,
    Channel::Pres,
];

impl Snapshot {
    /// Copy what changed since the last sync. Returns true if the plotted data
    /// (trend, events, fractions) changed.
    pub fn sync(&mut self, app: &AppState) -> bool {
        if self.generation == Some(app.generation) {
            return false;
        }
        self.generation = Some(app.generation);
        let mut changed = false;
        if let Some(from) = sync_vec(&mut self.trend, &app.trend, same_point) {
            if from == 0 {
                self.yext = [None; 6];
            }
            for p in &self.trend[from..] {
                for ch in MEASURED {
                    let Some(v) = ch.value(p) else { continue };
                    if !v.is_finite() {
                        continue;
                    }
                    let e = &mut self.yext[ch as usize];
                    *e = Some(match *e {
                        Some((lo, hi)) => (lo.min(v), hi.max(v)),
                        None => (v, v),
                    });
                }
            }
            changed = true;
        }
        if sync_vec(&mut self.events, &app.events, same_event).is_some() {
            changed = true;
        }
        // fractions are closed in place, and few: compare and copy whole
        if self.fractions != app.fractions {
            self.fractions.clone_from(&app.fractions);
            changed = true;
        }
        self.inst.clone_from(&app.inst);
        self.status.clone_from(&app.status);
        // REC only once this log has data, not as soon as the file is open
        self.recording = app.log_has_data();
        let file = app
            .log_filename
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        if file != self.log_file {
            self.log_file = file;
        }
        self.log_dir.clone_from(&app.log_dir);
        self.log_prefix.clone_from(&app.log_prefix);
        self.live = app.live;
        if changed {
            self.version = self.version.wrapping_add(1);
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(t: f64, uv: f64) -> TrendPoint {
        TrendPoint {
            cum_time_ms: t,
            cum_vol_ml: t / 1000.0,
            uv,
            ..TrendPoint::default()
        }
    }

    #[test]
    fn appends_only_new_points_and_detects_resets() {
        let mut app = AppState::new();
        let mut s = Snapshot::default();
        assert!(s.sync(&app) || s.trend.is_empty());
        for i in 0..5 {
            app.push_trend(pt(i as f64 * 100.0, i as f64));
        }
        assert!(s.sync(&app));
        assert_eq!(s.trend.len(), 5);
        assert_eq!(s.yext[Channel::Uv as usize], Some((0.0, 4.0)));
        let v = s.version;
        // unchanged generation: nothing to do
        assert!(!s.sync(&app));
        assert_eq!(s.version, v);
        // more points: appended
        app.push_trend(pt(500.0, -1.0));
        assert!(s.sync(&app));
        assert_eq!(s.trend.len(), 6);
        assert_eq!(s.yext[Channel::Uv as usize], Some((-1.0, 4.0)));
        // a new run: replaced, extents recomputed
        app.reset_run();
        app.push_trend(pt(0.0, 7.0));
        assert!(s.sync(&app));
        assert_eq!(s.trend, app.trend);
        assert_eq!(s.yext[Channel::Uv as usize], Some((7.0, 7.0)));
        // events and live state follow too
        app.inst.cum_time_ms = 42.0;
        app.push_event(rc_core::EventKind::Mark, "x");
        assert!(s.sync(&app));
        assert_eq!(s.events.len(), 1);
        assert_eq!(s.inst.cum_time_ms, 42.0);
    }

    #[test]
    fn nan_readings_do_not_force_copies() {
        let mut app = AppState::new();
        app.push_trend(pt(0.0, f64::NAN));
        let mut s = Snapshot::default();
        s.sync(&app);
        app.push_trend(pt(1.0, f64::NAN));
        assert_eq!(sync_vec(&mut s.trend, &app.trend, same_point), Some(1));
        assert_eq!(s.yext[Channel::Uv as usize], None);
    }
}
