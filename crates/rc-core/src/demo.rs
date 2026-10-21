//! Simulated Akta data stream for testing and demonstrating without hardware.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::types::{AppState, EventKind, InjectValue, SharedApp, TrendPoint};

pub const DEMO_FRACTIONS_PER_RUN: i64 = 20;

/// Real time per demo step; each step is 2 s of simulated time (10× speed-up).
pub const DEMO_STEP_SLEEP: Duration = Duration::from_millis(200);
/// Pause after a run has ended, so the ended run is visible before the restart.
pub const DEMO_END_PAUSE: Duration = Duration::from_secs(1);
/// Sleeps are cut into slices this long so a stop request is noticed quickly.
const SLEEP_SLICE: Duration = Duration::from_millis(20);

/// Runs demo runs until `shutdown` is set; call on its own thread. Holds the
/// app lock only while applying each step (never while sleeping).
///
/// Runs repeat indefinitely. Each run ends after DEMO_FRACTIONS_PER_RUN
/// fractions; the next one rolls over to a fresh log file (via `start_run`),
/// exercising the automatic per-run restart of logging.
pub fn run_demo(app: SharedApp, shutdown: Arc<AtomicBool>) {
    run_demo_with(app, shutdown, DEMO_STEP_SLEEP, DEMO_END_PAUSE);
}

/// [`run_demo`] with other sleep times (tests run it with zero sleeps).
pub fn run_demo_with(
    app: SharedApp,
    shutdown: Arc<AtomicBool>,
    step: Duration,
    end_pause: Duration,
) {
    app.lock()
        .push_event(EventKind::Connect, "Demo mode — simulated Akta");
    while !shutdown.load(Ordering::Relaxed) {
        run_demo_run(&app, &shutdown, step, end_pause);
    }
}

/// Sleep for `d`, waking early (returning false) when `shutdown` is set.
fn sleep_unless_shutdown(d: Duration, shutdown: &AtomicBool) -> bool {
    let end = Instant::now() + d;
    loop {
        if shutdown.load(Ordering::Relaxed) {
            return false;
        }
        let now = Instant::now();
        if now >= end {
            return true;
        }
        std::thread::sleep((end - now).min(SLEEP_SLICE));
    }
}

/// Simulated signals at `t_min` minutes into the run: (uv, cond, ph, pctb, pres).
fn demo_signals(t_min: f64) -> (f64, f64, f64, f64, f64) {
    let uv = 0.02
        + 1.2 * (-((t_min - 4.0).powi(2)) / (2.0 * 0.4f64.powi(2))).exp()
        + 0.5 * (-((t_min - 6.5).powi(2)) / (2.0 * 0.2f64.powi(2))).exp();
    let cond = 10.0 + 2.0 * (t_min * 0.5).sin();
    let ph = 7.0 + 0.4 * (t_min * 0.3).sin();
    let pctb = (if t_min < 3.0 {
        5.0
    } else {
        5.0 + 30.0 * (t_min - 3.0)
    })
    .clamp(5.0, 100.0);
    let pres = 0.40 + 0.02 * (t_min * 2.0).sin();
    (uv, cond, ph, pctb, pres)
}

/// What a demo step did to the run.
#[derive(PartialEq)]
enum StepResult {
    Continue,
    RunEnded,
}

/// Apply simulated step `step` (1-based); `cum_vol` is the run volume so far (mL).
fn apply_step(app: &mut AppState, step: u64, cum_vol: &mut f64) -> StepResult {
    let t_ms = step as f64 * 2_000.0; // 2 s per step (fixed time)
    let t_min = t_ms / 60_000.0;

    // Flow rate: 1 mL/min baseline with a slow sinusoidal variation ±0.5 mL/min,
    // so time and volume axes diverge visibly.
    let flow_ml_per_min = 1.0 + 0.5 * (t_min * 0.8).sin();
    let flow_ml_per_ms = flow_ml_per_min / 60_000.0;
    *cum_vol += flow_ml_per_ms * 2_000.0; // Δv for this 2 s step
    let v_ml = *cum_vol;

    let (uv, cond, ph, pctb, pres) = demo_signals(t_min);

    // Keep the InstrumentState accumulators in sync so push_event timestamps events correctly.
    app.inst.cum_time_ms = t_ms;
    app.inst.cum_vol_ml = v_ml;
    app.push_trend(TrendPoint {
        cum_time_ms: t_ms,
        cum_vol_ml: v_ml,
        uv,
        cond_mscm: cond,
        ph,
        pctb,
        pres_mpa: pres,
        temp_degc: 22.0,
    });
    app.inst.run_time_ms = t_ms;
    app.inst.run_vol_ml = v_ml;

    // Cycle inject state every 40 demo steps: UNKNOWN -> LOAD -> INJECT -> WASTE.
    if (step - 1).is_multiple_of(40) {
        const VALVES: [InjectValue; 4] = [
            InjectValue::Unknown,
            InjectValue::Load,
            InjectValue::Inject,
            InjectValue::Waste,
        ];
        let new_valve = VALVES[((step - 1) / 40 % 4) as usize];
        app.inst.inject_value = new_valve;
        app.push_event(
            EventKind::Inject,
            format!("Valve → {} (demo)", new_valve.label()),
        );
    }

    // Cycle fraction size every 30 steps, including zero (= route to waste).
    if (step - 1).is_multiple_of(30) {
        const SIZES: [f64; 4] = [0.0, 0.5, 1.0, 2.0];
        let new_size = SIZES[((step - 1) / 30 % 4) as usize];
        app.inst.fraction_vol_ml = new_size;
        app.update_fraction_state();
        app.push_event(EventKind::Log, format!("Fraction size {new_size:.1} ml"));
    }

    // Advance the fraction number periodically, only while fractions are collected.
    if step.is_multiple_of(20) && app.inst.fraction_vol_ml > 0.0 {
        app.inst.fraction_number += 1;
        app.update_fraction_state();
        let n = app.inst.fraction_number;
        app.push_event(EventKind::Tubefeed, format!("Feed Tube Nr {n}"));

        // End this run once enough fractions have been collected.
        if n >= DEMO_FRACTIONS_PER_RUN {
            app.inst.run_state = "End".to_string();
            app.push_event(EventKind::RunState, "Run state: End  inject=IV_LOAD");
            return StepResult::RunEnded;
        }
    }
    app.touch();
    StepResult::Continue
}

/// One simulated chromatographic run: streams until shutdown or until
/// DEMO_FRACTIONS_PER_RUN fractions have been collected, then ends the run.
fn run_demo_run(app: &SharedApp, shutdown: &AtomicBool, step_sleep: Duration, end_pause: Duration) {
    {
        let mut a = app.lock();
        // counter restarts → new file (from the second run on), cleared display
        a.start_run();
        a.inst.run_state = "Run".to_string();
        a.push_event(EventKind::RunState, "Run state: Run  inject=IV_LOAD");
    }
    let mut step = 0u64;
    let mut cum_vol = 0.0; // accumulated volume in mL (not derived from a fixed flow)
    while sleep_unless_shutdown(step_sleep, shutdown) {
        step += 1;
        let result = apply_step(&mut app.lock(), step, &mut cum_vol);
        if result == StepResult::RunEnded {
            sleep_unless_shutdown(end_pause, shutdown); // the ended run stays visible briefly
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::csvlog::{CsvLog, load_csv_log};

    /// Steps until the run ends, as `run_demo_run` does without sleeping.
    fn simulate_run(app: &mut AppState) -> u64 {
        app.inst.run_state = "Run".to_string();
        app.push_event(EventKind::RunState, "Run state: Run  inject=IV_LOAD");
        let mut cum_vol = 0.0;
        for step in 1.. {
            if apply_step(app, step, &mut cum_vol) == StepResult::RunEnded {
                return step;
            }
        }
        unreachable!()
    }

    #[test]
    fn signals_match_formulas() {
        // the signal formulas written out at t = 4 min, 0.5 min and 10 min
        let (uv, cond, ph, pctb, pres) = demo_signals(4.0);
        assert!((uv - (0.02 + 1.2 + 0.5 * (-(2.5f64.powi(2)) / 0.08).exp())).abs() < 1e-12);
        assert!((cond - (10.0 + 2.0 * 2.0f64.sin())).abs() < 1e-12);
        assert!((ph - (7.0 + 0.4 * 1.2f64.sin())).abs() < 1e-12);
        assert_eq!(pctb, 35.0);
        assert!((pres - (0.40 + 0.02 * 8.0f64.sin())).abs() < 1e-12);
        assert_eq!(demo_signals(0.5).3, 5.0);
        assert_eq!(demo_signals(10.0).3, 100.0);
    }

    #[test]
    fn one_run() {
        let mut app = AppState::new();
        let steps = simulate_run(&mut app);
        assert_eq!(app.trend.len() as u64, steps);
        assert_eq!(app.inst.run_state, "End");
        assert_eq!(app.inst.fraction_number, DEMO_FRACTIONS_PER_RUN);
        assert_eq!(app.trend[0].cum_time_ms, 2000.0);
        assert_eq!(app.trend[1].cum_time_ms, 4000.0);
        assert!(
            app.trend
                .windows(2)
                .all(|w| w[1].cum_vol_ml > w[0].cum_vol_ml)
        );
        assert!(app.trend.iter().all(|p| p.temp_degc == 22.0));

        let texts = |kind| -> Vec<String> {
            app.events
                .iter()
                .filter(|e| e.kind == kind)
                .map(|e| e.text.clone())
                .collect()
        };
        assert_eq!(
            texts(EventKind::RunState),
            [
                "Run state: Run  inject=IV_LOAD",
                "Run state: End  inject=IV_LOAD"
            ]
        );
        let inject = texts(EventKind::Inject);
        assert_eq!(
            inject[..4],
            [
                "Valve → UNKNOWN (demo)",
                "Valve → LOAD (demo)",
                "Valve → INJECT (demo)",
                "Valve → WASTE (demo)"
            ]
        );
        let sizes = texts(EventKind::Log);
        assert_eq!(
            sizes[..5],
            [
                "Fraction size 0.0 ml",
                "Fraction size 0.5 ml",
                "Fraction size 1.0 ml",
                "Fraction size 2.0 ml",
                "Fraction size 0.0 ml"
            ]
        );
        let tubes = texts(EventKind::Tubefeed);
        assert_eq!(tubes.len() as i64, DEMO_FRACTIONS_PER_RUN - 1);
        assert_eq!(tubes.last().map(String::as_str), Some("Feed Tube Nr 20"));
        // the first inject event is at step 1, the first fraction size change at step 31
        let first = app
            .events
            .iter()
            .find(|e| e.text == "Fraction size 0.5 ml")
            .expect("event");
        assert_eq!(first.cum_time_ms, 62_000.0);
        // fractions: every tube collected while the size was > 0
        assert!(!app.fractions.is_empty());
        assert_eq!(
            app.fractions.last().map(|f| f.fraction_number),
            Some(DEMO_FRACTIONS_PER_RUN)
        );
    }

    #[test]
    fn fractions_rebuilt_from_the_log_match() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("demo.csv");
        let mut app = AppState::new();
        app.log = Some(CsvLog::create(&path).expect("create"));
        simulate_run(&mut app);
        app.close_log();
        let back = load_csv_log(&path).expect("load");
        assert_eq!(back.trend.len(), app.trend.len());
        assert_eq!(back.events.len(), app.events.len());
        assert_eq!(back.fractions.len(), app.fractions.len());
        for (a, b) in back.fractions.iter().zip(&app.fractions) {
            assert_eq!(a.fraction_number, b.fraction_number);
            assert!((a.start_time_ms - b.start_time_ms).abs() < 0.05);
            assert!((a.start_vol_ml - b.start_vol_ml).abs() < 1e-6);
        }
    }

    #[test]
    fn thread_runs_rolls_and_stops_quickly() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut state = AppState::new();
        state.log_dir = dir.path().to_path_buf();
        state.log_prefix = "demo".into();
        // a method setup for the first run only
        state
            .run_info_for_new_log
            .insert("Column".into(), "Demo column".into());
        let name = crate::csvlog::unique_log_name(dir.path(), "demo");
        state.log = Some(CsvLog::create(&name).expect("create"));
        state.log_filename = name.clone();
        let app = state.shared();
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread = {
            let (app, shutdown) = (app.clone(), shutdown.clone());
            std::thread::spawn(move || run_demo_with(app, shutdown, Duration::ZERO, Duration::ZERO))
        };
        // wait for the second run: a new log, without the first run's setup
        let t0 = Instant::now();
        loop {
            {
                let a = app.lock();
                if a.log_filename != name && !a.trend.is_empty() {
                    assert_eq!(a.events[0].text, "Run state: Run  inject=IV_LOAD");
                    assert!(a.run_serial >= 2);
                    assert!(a.run_info_for_new_log.is_empty());
                    break;
                }
            }
            assert!(t0.elapsed() < Duration::from_secs(20), "no second run");
            std::thread::sleep(Duration::from_millis(5));
        }
        shutdown.store(true, Ordering::Relaxed);
        thread.join().expect("demo thread");
        app.lock().close_log();
        let first = load_csv_log(&name).expect("first log");
        assert_eq!(first.events[0].text, "Demo mode — simulated Akta");
        // the first run did not roll (no data yet): its setup went into this log
        assert_eq!(first.events[1].text, "Column: Demo column");
        assert_eq!(first.events[2].text, "Run state: Run  inject=IV_LOAD");
        assert_eq!(
            first.events.last().map(|e| e.text.as_str()),
            Some("Run state: End  inject=IV_LOAD")
        );
    }

    #[test]
    fn stop_is_quick_with_real_sleeps() {
        let app = AppState::new().shared();
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread = {
            let (app, shutdown) = (app.clone(), shutdown.clone());
            std::thread::spawn(move || run_demo(app, shutdown))
        };
        // the lock is free while the demo sleeps: the GUI never waits long for it
        for _ in 0..25 {
            let t = Instant::now();
            drop(app.lock());
            assert!(t.elapsed() < Duration::from_millis(20), "{:?}", t.elapsed());
            std::thread::sleep(Duration::from_millis(20));
        }
        let t = Instant::now();
        shutdown.store(true, Ordering::Relaxed);
        thread.join().expect("demo thread");
        assert!(
            t.elapsed() < Duration::from_millis(150),
            "{:?}",
            t.elapsed()
        );
        let a = app.lock();
        assert!(!a.trend.is_empty() && a.trend.len() <= 3);
        assert_eq!(a.events[0].kind, EventKind::Connect);
    }
}
