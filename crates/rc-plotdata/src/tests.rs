//! Unit tests. "Golden" values are known-good reference outputs that pin the
//! behaviour: number formats, label texts and layout, downsampling.

use super::*;

/// Trend point at `t` min: volume t/2 mL, UV 0.1, the other channels constant.
fn pt(t: f64) -> TrendPoint {
    pt_uv(t, 0.1)
}
fn pt_uv(t: f64, uv: f64) -> TrendPoint {
    TrendPoint {
        cum_time_ms: t * 60_000.0,
        cum_vol_ml: t / 2.0,
        uv,
        cond_mscm: 10.0,
        ph: 7.0,
        pctb: 0.0,
        pres_mpa: 0.3,
        temp_degc: 20.0,
    }
}
/// Event at `t` min (volume t/2 mL).
fn ev(t: f64, kind: EventKind, text: &str) -> AktaEvent {
    AktaEvent {
        cum_time_ms: t * 60_000.0,
        cum_vol_ml: t / 2.0,
        kind,
        text: text.to_string(),
    }
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn x_scale_and_labels() {
    assert_eq!(x(120_000.0, 3.0, XS_TIME), 2.0);
    assert_eq!(x(120_000.0, 3.0, XS_VOLUME), 3.0);
    assert_eq!(x(120_000.0, 3.0, 1.5), 2.0);
    assert_eq!(xlabel(0.0), "Time (min)");
    assert_eq!(xlabel(1.0), "Volume (mL)");
    assert_eq!(xlabel(5.0), "Column volumes (CV)");
    assert_eq!(Channel::Ph.overlay_ylabel(), "pH");
    assert_eq!(Channel::Uv.overlay_ylabel(), "UV (AU)");
}

#[test]
fn event_colours_match_reference() {
    // CSS / X11 named colours: hotpink, green, teal, grey50, blue, green, red, darkorange
    let golden: [(EventKind, Rgb, LineStyle); 8] = [
        (
            EventKind::Inject,
            [1.0, 0.411_764_7, 0.705_882_4],
            LineStyle::Solid,
        ),
        (
            EventKind::RunState,
            [0.0, 0.501_960_8, 0.0],
            LineStyle::Dash,
        ),
        (
            EventKind::Step,
            [0.0, 0.501_960_8, 0.501_960_8],
            LineStyle::DashDot,
        ),
        (
            EventKind::Tubefeed,
            [0.498_039_2, 0.498_039_2, 0.498_039_2],
            LineStyle::Solid,
        ),
        (EventKind::Log, [0.0, 0.0, 1.0], LineStyle::Solid),
        (
            EventKind::Connect,
            [0.0, 0.501_960_8, 0.0],
            LineStyle::Solid,
        ),
        (EventKind::Disconnect, [1.0, 0.0, 0.0], LineStyle::Solid),
        (EventKind::Mark, [1.0, 0.549_019_6, 0.0], LineStyle::Solid),
    ];
    for (kind, rgb, style) in golden {
        let (c, s) = event_style(kind).expect("drawn");
        assert_eq!(s, style, "{kind}");
        for k in 0..3 {
            assert!((c[k] - rgb[k]).abs() < 1e-6, "{kind}: {c:?} vs {rgb:?}");
        }
    }
    assert!(event_style(EventKind::Info).is_none());
}

/// The trend the reference outputs are computed on: x = i/480 min, i = 1..30000.
fn golden_trend() -> Vec<TrendPoint> {
    (1..=30_000)
        .map(|i| {
            let f = i as f64;
            TrendPoint {
                cum_time_ms: f * 125.0,
                cum_vol_ml: f * 0.001,
                uv: (f * 0.37).sin() * (f * 0.011).cos() + if i == 5000 { 9.0 } else { 0.0 },
                cond_mscm: 10.0,
                ph: 7.0,
                pctb: 0.0,
                pres_mpa: 0.3,
                temp_degc: 20.0,
            }
        })
        .collect()
}

#[test]
fn first_at_or_after_matches_reference() {
    let trend = golden_trend();
    // expected 1-based point numbers, hence the `- 1`
    for (xq, want) in [
        (-1.0, 1),
        (0.0, 1),
        (1.0 / 480.0, 1),
        (10.0, 4800),
        (10.00001, 4801),
        (62.5, 30000),
        (63.0, 30001),
    ] {
        assert_eq!(first_at_or_after(&trend, xq, 0.0), want - 1, "x = {xq}");
    }
    assert_eq!(first_at_or_after(&[], 1.0, 0.0), 0);
}

#[test]
fn display_points_match_reference() {
    let trend = golden_trend();
    let golden = include_str!("../testdata/display_points_reference.txt");
    let mut n = 0;
    for line in golden.lines() {
        // "dp <lo> <hi> <count> <i1,i2,…>" with 1-based point numbers (= x·480)
        let f: Vec<&str> = line.split(' ').collect();
        let parse = |s: &str| match s {
            "-Inf" => f64::NEG_INFINITY,
            "Inf" => f64::INFINITY,
            s => s.parse().expect("number"),
        };
        let (mut lo, hi) = (parse(f[1]), parse(f[2]));
        // The reference x values are single precision, where 3.3f0 < 3.3, so the
        // point at exactly 3.3 min lies before the window there.
        if lo == 3.3 {
            lo += 1e-9;
        }
        let count: usize = f[3].parse().expect("count");
        let want: Vec<i64> = f[4].split(',').map(|s| s.parse().expect("index")).collect();
        let pts = display_points(&trend, |p| p.uv, 0.0, (lo, hi));
        assert_eq!(pts.len(), count, "window ({lo}, {hi})");
        let got: Vec<i64> = pts.iter().map(|p| (p[0] * 480.0).round() as i64).collect();
        assert_eq!(got, want, "window ({lo}, {hi})");
        n += 1;
    }
    assert_eq!(n, 7);
}

#[test]
fn display_downsampling() {
    let trend: Vec<TrendPoint> = (1..=30_000)
        .map(|i| pt_uv(i as f64 / 480.0, if i == 5000 { 9.0 } else { 0.0 }))
        .collect();
    let all = display_points(&trend, |p| p.uv, 0.0, (f64::NEG_INFINITY, f64::INFINITY));
    assert!(all.len() <= 2 * DISPLAY_BINS);
    assert_eq!(all.iter().map(|p| p[1]).fold(f64::MIN, f64::max), 9.0); // a one-point peak survives
    let zoom = display_points(&trend, |p| p.uv, 0.0, (10.0, 11.0));
    assert_eq!(zoom.len(), 482); // all 480 points in the window + 2 edges
    // empty and tiny inputs
    assert!(display_points(&[], |p| p.uv, 0.0, (0.0, 1.0)).is_empty());
    let one = display_points(&trend[..1], |p| p.uv, 0.0, (5.0, 6.0));
    assert_eq!(one.len(), 1);
}

#[test]
fn normalise_traces() {
    let (s, r) = normalise(&[[0.0, 2.0], [1.0, 4.0]], Channel::Uv, None);
    assert!(close(s[1][1], 100.0, 1e-12) && close(s[0][1], 0.0, 1e-12));
    assert_eq!(r, (2.0, 4.0));
    // %B is already a percentage
    let (s, r) = normalise(&[[0.0, 20.0]], Channel::Pctb, None);
    assert_eq!((s[0][1], r), (20.0, (0.0, 100.0)));
    // flow is scaled from 0
    let (s, r) = normalise(&[[0.0, 1.0], [1.0, 2.0]], Channel::Flow, None);
    assert_eq!(r, (0.0, 2.0));
    assert!(close(s[0][1], 50.0, 1e-12));
    // a flat trace sits at 50 %
    let (s, _) = normalise(&[[0.0, 3.0], [1.0, 3.0]], Channel::Cond, None);
    assert_eq!(s[1][1], 50.0);
    // given range (downsampled part of a trace)
    let (s, r) = normalise(&[[0.0, 5.0]], Channel::Ph, Some((0.0, 10.0)));
    assert_eq!((s[0][1], r), (50.0, (0.0, 10.0)));
    let (s, r) = normalise(&[], Channel::Uv, None);
    assert!(s.is_empty() && r == (0.0, 0.0));
}

#[test]
fn overlay_labels_match_reference() {
    // range → text, for ("UV", "AU", (lo, hi)) and ("pH", "", …)
    let cases = [
        ((0.012, 1.2234), "0.012–1.22"),
        ((0.0, 100.0), "0–100"),
        ((-0.000123456, 12345.6), "-0.000123–1.23e+04"),
        ((1.0e-5, 9.995), "1e-05–9.99"),
        ((0.0, 0.0), "0–0"),
        ((999.5, 1.0e7), "1000–1e+07"),
    ];
    for ((lo, hi), txt) in cases {
        assert_eq!(
            overlay_label("UV", "AU", (lo, hi)),
            format!("━━  UV   ({txt} AU)")
        );
        assert_eq!(
            overlay_label("pH", "", (lo, hi)),
            format!("━━  pH   ({txt})")
        );
    }
}

#[test]
fn printf_g() {
    for (v, s) in [
        (0.0, "0"),
        (1.0, "1"),
        (123.0, "123"),
        (1234.0, "1.23e+03"),
        (0.0001, "0.0001"),
        (0.00001234, "1.23e-05"),
        (-2.5, "-2.5"),
        (99.95, "100"),
        (999.5, "1000"),
        (9.9996, "10"),
        (0.1, "0.1"),
        (1e100, "1e+100"),
    ] {
        assert_eq!(fmt_g(v, 3), s, "{v}");
    }
    assert_eq!(fmt_g(f64::NAN, 3), "NaN");
}

#[test]
fn event_labels_match_reference() {
    use EventKind::*;
    // kind, text → label at the top, fraction label at the bottom
    let cases: [(EventKind, &str, &str, &str); 25] = [
        (Inject, "Valve → inject (demo)", "INJ INJECT", "?"),
        (Inject, "Inject event (8 bytes)", "INJECT", "?"),
        (RunState, "Run state: Run  inject=IV_LOAD", "RUN", "?"),
        (RunState, "weird", "STATE", "?"),
        (Step, "Step t=0.0s  v=0.0000mL", "STEP", "?"),
        (Log, "Fraction size 1.0 ml", "FRAC START", "?"),
        (Log, "Fraction size 0.0 ml", "FRAC STOP", "?"),
        (Log, "Manual Set; Buffer Valve Pos 3", "Buffer 3", "?"),
        (Log, "Injection Valve Inj", "Inj valve Inj", "?"),
        (Log, "Flow 1.0 ml/min", "Flow 1.0", "?"),
        (Log, "Manual Set; Concentration 12.5 %B", "%B 12.5", "?"),
        (Log, "Feed Tube Nr 7", "Tube 7", "7"),
        (
            Log,
            "Method start: flow 1.0 ml/min; 0 %B; buffer port 1",
            "Method start",
            "?",
        ),
        (
            Log,
            "Gradient; Length 50.0 ml; Target 100 %B",
            "Gradient; Length 5…",
            "?",
        ),
        (Log, "  Autozero  ", "Autozero", "?"),
        (
            Log,
            "Error: 62 Check that tube position is OK   ",
            "Error: 62 Check th…",
            "?",
        ),
        (Mark, "Mark", "Mark: Mark", "?"),
        (
            Mark,
            "sample loaded into the loop äöü",
            "Mark: sample loaded i…",
            "?",
        ),
        (Connect, "x", "CONNECT", "?"),
        (Disconnect, "x", "DISCONNECT", "?"),
        (Tubefeed, "Tubefeed event (10 bytes)", "EVT_TUBEFEED", "?"),
        (Info, "Sample: x", "EVT_INFO", "?"),
        (Log, "Fraction size x ml", "Fraction size x ml", "?"),
        (Log, "Feed Tube Nr  12 extra", "Tube 12", "12"),
        (
            Log,
            "Ünïcödé text that is longer than twenty",
            "Ünïcödé text that …",
            "?",
        ),
    ];
    for (kind, text, top, bottom) in cases {
        let e = ev(0.0, kind, text);
        assert_eq!(event_label_top(&e), top, "{kind} {text:?}");
        assert_eq!(fraction_label_bottom(&e), bottom, "{kind} {text:?}");
    }
}

#[test]
fn label_positions_match_reference() {
    let xs = [0.0, 0.01, 0.5, 0.02, 3.0, 3.0, 9.5, 2.0];
    let labels: Vec<String> = [
        "INJECT",
        "RUN",
        "Flow 1.0",
        "STEP",
        "Buffer 8",
        "Mark: hello",
        "END",
        "FRAC START",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let lims = (0.0, -0.1, 10.0, 1.2);
    // reference positions: a 10 × 1.2 data area at (0, -0.1) shown in 800 × 300 px
    let free = [
        [0.0375, 1.092],
        [0.0475, 1.0504],
        [0.5375, 1.092],
        [0.0575, 1.0088],
        [3.0375, 1.092],
        [3.0375, 1.0504],
        [9.5375, 1.092],
        [2.0375, 1.092],
    ];
    let avoided = [
        [0.0375, 1.092],
        [0.0475, 1.0504],
        [0.5375, 1.092],
        [0.0575, 1.0088],
        [3.0375, 1.0088],
        [3.0375, 0.9672],
        [9.5375, 1.092],
        [2.0375, 1.0088],
    ];
    for (avoid, want) in [(None, free), (Some((100.0, 300.0, 0.0, 20.0)), avoided)] {
        let got = event_label_positions(&xs, &labels, lims, (800.0, 300.0), avoid);
        for (g, w) in got.iter().zip(want) {
            // the reference positions are single precision
            assert!(
                close(g[0], w[0], 1e-5) && close(g[1], w[1], 1e-5),
                "{avoid:?}: {g:?} vs {w:?}"
            );
        }
    }
    assert!(event_label_positions(&[], &[], lims, (800.0, 300.0), None).is_empty());
}

#[test]
fn labelled_events_match_reference() {
    use EventKind::*;
    let evs = [
        ev(0.0, Log, "Fraction size 0.0 ml"),
        ev(1.0, Log, "Fraction size 1.0 ml"),
        ev(2.0, Log, "Fraction size 2.0 ml"),
        ev(3.0, Tubefeed, "Tubefeed"),
        ev(4.0, Log, "Fraction size 0.0 ml"),
        ev(5.0, Info, "Sample: x"),
        ev(6.0, Log, "Fraction size bad"),
        ev(7.0, Log, "Fraction size 3.0 ml"),
        ev(8.0, Mark, "m"),
    ];
    let got: Vec<usize> = labelled_events(&evs)
        .iter()
        .map(|e| {
            evs.iter()
                .position(|x| std::ptr::eq(x, *e))
                .expect("from evs")
        })
        .collect();
    // expected: indices (0-based) 1, 4, 6, 7, 8
    assert_eq!(got, vec![1, 4, 6, 7, 8]);
}

#[test]
fn fraction_spans_from_fractions() {
    let trend = [pt(0.0), pt(10.0)];
    let fr = |n, t0: f64, t1: Option<f64>| Fraction {
        fraction_number: n,
        start_time_ms: t0 * 60_000.0,
        start_vol_ml: t0 / 2.0,
        end_time_ms: t1.map(|t| t * 60_000.0),
        end_vol_ml: t1.map(|t| t / 2.0),
    };
    let fracs = [
        fr(1, 1.0, Some(2.0)),
        fr(2, 2.0, Some(2.0)),
        fr(3, 2.0, Some(3.0)),
        fr(4, 3.0, None),
    ];
    let spans = fraction_spans(&fracs, &trend, 0.0);
    assert_eq!(spans.len(), 3); // the empty one is skipped
    assert_eq!((spans[0].x0, spans[0].x1, spans[0].number), (1.0, 2.0, 1));
    assert_eq!((spans[0].color, spans[0].shade), (FRAC_COLORS[0], 0));
    // colour by position in the fraction list, not among the drawn spans
    assert_eq!(
        (spans[1].number, spans[1].color, spans[1].shade),
        (3, FRAC_COLORS[0], 0)
    );
    // the open fraction runs to the newest point
    assert_eq!(
        (spans[2].x0, spans[2].x1, spans[2].color, spans[2].shade),
        (3.0, 10.0, FRAC_COLORS[1], 1)
    );
    assert_eq!(spans[2].mid(), 6.5);
    // in mL
    let spans = fraction_spans(&fracs, &trend, 1.0);
    assert_eq!((spans[2].x0, spans[2].x1), (1.5, 5.0));
}

#[test]
fn state_at_past_moments() {
    use EventKind::*;
    let events = [
        ev(
            0.1,
            Log,
            "Method start: flow 0.5 ml/min, 0 %B, buffer port 1, inject Load, fraction 0.0 ml, 2 breakpoints",
        ),
        ev(0.2, RunState, "Run state: Run  inject=IV_LOAD"),
        ev(1.0, Log, "Buffer Valve Pos 8"),
        ev(3.0, Log, "Fraction size 1.0 ml"),
        ev(4.0, Log, "Injection Valve Inj"),
    ];
    let s = state_at(&events, &pt(2.0));
    assert!(
        s.run_state == "Run"
            && s.buffer_port == 8
            && s.inject_value == InjectValue::Load
            && s.fraction_vol_ml == 0.0
    );
    let s = state_at(&events, &pt(5.0));
    assert!(s.inject_value == InjectValue::Inject && s.fraction_vol_ml == 1.0);
    assert_eq!(
        state_at(&events, &pt(0.05)).inject_value,
        InjectValue::Unknown
    );
    assert_eq!(
        flow_at(&[ev(0.5, Log, "Flow 0.5 ml/min")], &pt(1.0)),
        Some(0.5)
    );
    assert_eq!(flow_at(&[ev(0.5, Log, "Flow 0.5 ml/min")], &pt(0.2)), None);
}

#[test]
fn state_and_valve_summary_match_reference() {
    use EventKind::*;
    let events = [
        ev(
            0.1,
            Log,
            "Method start: flow 0.5 ml/min, 0 %B, buffer port 3, inject Inj, fraction 0.0 ml, 2 breakpoints",
        ),
        ev(0.2, RunState, "Run state: Run  inject=IV_LOAD"),
        ev(1.0, Log, "Buffer Valve Pos 8"),
        ev(3.0, Log, "Fraction size 1.0 ml"),
        ev(3.5, Log, "Feed Tube Nr 0"),
        ev(4.0, Log, "Injection Valve Inj"),
        ev(4.5, Log, "Feed Tube Nr 4"),
    ];
    let info: BTreeMap<String, String> = [("Port 3".to_string(), "Tris".to_string())].into();
    let none = BTreeMap::new();
    // t, run_state, inject, port, known, frac vol, frac number, valve summaries
    let golden = [
        (
            0.05,
            "---",
            InjectValue::Unknown,
            1,
            false,
            0.0,
            1,
            "Valve unknown · Buffer 1 (A) · Outlet waste",
            "Valve unknown · Buffer 1 (A) · Outlet waste",
        ),
        (
            0.15,
            "---",
            InjectValue::Inject,
            3,
            true,
            0.0,
            1,
            "Valve inject · Buffer 3 (Tris) · Outlet waste",
            "Valve inject · Buffer 3 · Outlet waste",
        ),
        (
            0.5,
            "Run",
            InjectValue::Load,
            3,
            true,
            0.0,
            1,
            "Valve load · Buffer 3 (Tris) · Outlet waste",
            "Valve load · Buffer 3 · Outlet waste",
        ),
        (
            2.0,
            "Run",
            InjectValue::Load,
            8,
            true,
            0.0,
            1,
            "Valve load · Buffer 8 (sample) · Outlet waste",
            "Valve load · Buffer 8 (sample) · Outlet waste",
        ),
        (
            3.7,
            "Run",
            InjectValue::Load,
            8,
            true,
            1.0,
            1,
            "Valve load · Buffer 8 (sample) · Outlet frac #1, 1.00 mL",
            "Valve load · Buffer 8 (sample) · Outlet frac #1, 1.00 mL",
        ),
        (
            5.0,
            "Run",
            InjectValue::Inject,
            8,
            true,
            1.0,
            4,
            "Valve inject · Buffer 8 (sample) · Outlet frac #4, 1.00 mL",
            "Valve inject · Buffer 8 (sample) · Outlet frac #4, 1.00 mL",
        ),
    ];
    for (t, rs, inj, port, known, fv, fnum, vs_info, vs) in golden {
        let s = state_at(&events, &pt(t));
        assert_eq!(s.run_state, rs, "t={t}");
        assert_eq!(s.inject_value, inj, "t={t}");
        assert_eq!((s.buffer_port, s.buffer_port_known), (port, known), "t={t}");
        assert_eq!((s.fraction_vol_ml, s.fraction_number), (fv, fnum), "t={t}");
        assert_eq!((s.cum_time_ms, s.run_time_ms), (t * 60_000.0, t * 60_000.0));
        assert_eq!((s.cum_vol_ml, s.run_vol_ml), (t / 2.0, t / 2.0));
        assert_eq!(valve_summary(&s, &info), vs_info, "t={t}");
        assert_eq!(valve_summary(&s, &none), vs, "t={t}");
    }
}

#[test]
fn method_start_from_a_reopened_log() {
    // The CSV writer turns "," into ";" in event texts.
    let events = [ev(
        0.1,
        EventKind::Log,
        "Method start: flow 1.0 ml/min; 0 %B; buffer port 8; inject Waste; fraction 0.0 ml; 2 breakpoints",
    )];
    let s = state_at(&events, &pt(1.0));
    assert_eq!(
        (s.buffer_port, s.buffer_port_known, s.inject_value),
        (8, true, InjectValue::Waste)
    );
}

#[test]
fn flow_steps_and_channel_points() {
    use EventKind::*;
    let trend = [pt(0.0), pt(1.0), pt(4.0)];
    let events = [
        ev(0.5, Log, "Flow 1.0 ml/min"),
        ev(1.0, Log, "Concentration 10 %B"),
        ev(2.0, Log, "Flow 2.5 ml/min"),
        ev(2.5, Mark, "Flow 9.0 ml/min"), // only log events count
    ];
    let pts = flow_setpoints(&events, &trend, 0.0);
    assert_eq!(pts, vec![[0.5, 1.0], [2.0, 2.5], [4.0, 2.5]]);
    assert_eq!(channel_points(Channel::Flow, &trend, &events, 0.0), pts);
    assert!(flow_setpoints(&[], &trend, 0.0).is_empty());
    // no hold point when the last flow message is at the newest point
    assert_eq!(
        flow_setpoints(&events[..1], &trend[..1], 0.0),
        vec![[0.5, 1.0]]
    );
    let uv = channel_points(Channel::Uv, &trend, &events, 1.0);
    assert_eq!(uv, vec![[0.0, 0.1], [0.5, 0.1], [2.0, 0.1]]);
    assert_eq!(
        overlay_range(Channel::Flow, &trend, &events, 0.0),
        (0.0, 2.5)
    );
    assert_eq!(
        overlay_range(Channel::Pctb, &trend, &events, 0.0),
        (0.0, 100.0)
    );
    assert_eq!(
        overlay_range(Channel::Cond, &trend, &events, 0.0),
        (10.0, 10.0)
    );
}

#[test]
fn nice_tick_values() {
    // Known-good tick sets (steps of 1, 2, 2.5 or 5 per decade).
    assert_eq!(nice_ticks(0.0, 1.0, 6), vec![0.0, 0.2, 0.4, 0.6, 0.8, 1.0]);
    assert_eq!(
        nice_ticks(-3.0, 103.0, 6),
        vec![0.0, 20.0, 40.0, 60.0, 80.0, 100.0]
    );
    assert_eq!(
        nice_ticks(-0.0123, 1.234, 6),
        vec![0.0, 0.25, 0.5, 0.75, 1.0]
    );
    assert_eq!(nice_ticks(0.0, 92.0, 6), vec![0.0, 20.0, 40.0, 60.0, 80.0]);
    assert_eq!(
        nice_ticks(13.0, 14.0, 6),
        vec![13.0, 13.2, 13.4, 13.6, 13.8, 14.0]
    );
    assert_eq!(
        nice_ticks(-3.0, 110.0, 6),
        vec![0.0, 20.0, 40.0, 60.0, 80.0, 100.0]
    );
    assert_eq!(nice_ticks(0.0, 0.00375, 6), vec![0.0, 0.001, 0.002, 0.003]);
    // reversed, degenerate, non-finite
    assert_eq!(nice_ticks(1.0, 0.0, 6), nice_ticks(0.0, 1.0, 6));
    assert_eq!(nice_ticks(2.0, 2.0, 5), vec![2.0]);
    assert!(nice_ticks(0.0, f64::NAN, 5).is_empty());
    // a large offset (values far from 0 relative to their span)
    let t = nice_ticks(1000.0, 1000.5, 5);
    assert!(
        t.len() >= 2 && t.iter().all(|v| (1000.0..=1000.5).contains(v)),
        "{t:?}"
    );
    // always inside the range, a few of them, evenly spaced
    for (lo, hi) in [
        (0.0, 7.3),
        (-0.0021, 0.0154),
        (5.0, 1e6),
        (-50.0, -49.0),
        (0.03, 0.031),
    ] {
        for n in [3, 5, 6] {
            let t = nice_ticks(lo, hi, n);
            assert!(
                t.len() >= 2 && t.len() <= 2 * n + 1,
                "{lo}..{hi} n={n}: {t:?}"
            );
            assert!(
                t.iter().all(|&v| v >= lo - 1e-12 && v <= hi + 1e-12),
                "{lo}..{hi}: {t:?}"
            );
            let d: Vec<f64> = t.windows(2).map(|w| w[1] - w[0]).collect();
            assert!(
                d.iter().all(|x| (x - d[0]).abs() < 1e-9 * (hi - lo)),
                "{t:?}"
            );
        }
    }
}

#[test]
fn tick_labels_match_reference() {
    // one shared number of decimals, the fewest that show every tick exactly
    let cases: [(&[f64], &[&str]); 7] = [
        (&[0.0, 0.5, 1.0], &["0.0", "0.5", "1.0"]),
        (&[0.0, 0.25, 0.5], &["0.00", "0.25", "0.50"]),
        (&[-0.2, 0.0, 0.2], &["−0.2", "0.0", "0.2"]),
        (&[0.0, 20.0, 40.0], &["0", "20", "40"]),
        (&[0.001, 0.002], &["0.001", "0.002"]),
        (&[10.0, 20.0, 30.0], &["10", "20", "30"]),
        (&[1.5, 2.0, 2.5], &["1.5", "2.0", "2.5"]),
    ];
    for (v, want) in cases {
        assert_eq!(
            tick_labels(v),
            want.iter().map(|s| s.to_string()).collect::<Vec<_>>()
        );
    }
    assert_eq!(tick_labels(&[-0.0]), vec!["0"]);
    // close ticks far from 0 keep distinct labels (single precision would merge them)
    assert_eq!(
        tick_labels(&[1000.001, 1000.002, 1000.003]),
        vec!["1000.001", "1000.002", "1000.003"]
    );
    assert_eq!(
        tick_labels(&[16777216.0, 16777217.0]),
        vec!["16777216", "16777217"]
    );
    assert_eq!(tick_labels(&[0.1 + 0.2, 0.4]), vec!["0.3", "0.4"]);
}

#[test]
fn overlay_ticks_in_real_units() {
    // UV 0–2 AU mapped to 0–100 %, shown from -3 % to 110 %
    let (pos, labels) = overlay_real_ticks((0.0, 2.0), OVERLAY_YLIMS);
    assert_eq!(labels, vec!["0.0", "0.5", "1.0", "1.5", "2.0"]);
    assert_eq!(pos, vec![0.0, 25.0, 50.0, 75.0, 100.0]);
    let (pos, labels) = overlay_real_ticks((3.0, 3.0), OVERLAY_YLIMS);
    assert_eq!((pos, labels), (vec![50.0], vec!["3".to_string()]));
}

#[test]
fn auto_ranges() {
    assert_eq!(auto_ylim(Channel::Pctb, Some((0.0, 50.0))), PCTB_YLIMS);
    assert_eq!(auto_ylim(Channel::Uv, Some((0.0, 1.0))), (-0.05, 1.05));
    assert_eq!(auto_ylim(Channel::Flow, Some((0.5, 1.0))), (0.0, 1.025));
    let (a, b) = auto_ylim(Channel::Cond, Some((10.0, 10.0)));
    assert!(a < 10.0 && b > 10.0);
    assert_eq!(
        trend_extrema(&[pt(0.0), pt_uv(1.0, -2.0)], |p| p.uv),
        Some((-2.0, 0.1))
    );
    assert_eq!(y_extrema(&[]), None);
}

#[test]
fn fraction_greys_are_subtle_but_apart() {
    // on white paper: both light, the second a little darker
    let on_white = |c: Rgb| c.map(|v| 1.0 - FRAC_ALPHA * (1.0 - v));
    let (a, b) = (on_white(FRAC_COLORS[0]), on_white(FRAC_COLORS[1]));
    for (x, y) in a.iter().zip(&b) {
        assert!(*x > 0.94 && *y > 0.90, "{a:?} {b:?}");
        assert!(x - y > 0.02, "neighbouring tubes must differ: {a:?} {b:?}");
    }
    // a neutral (slightly cool) grey, no hue
    for c in FRAC_COLORS {
        assert!(c[2] - c[0] < 0.08 && c[2] >= c[0]);
    }
}

#[test]
fn steps_and_event_display() {
    use EventKind::*;
    let ev = |kind, text: &str| AktaEvent {
        cum_time_ms: 0.0,
        cum_vol_ml: 0.0,
        kind,
        text: text.into(),
    };
    // as in akta_20261020_135011.csv
    let step = ev(Step, "Step t=60.0s  v=0.9990mL");
    let bp = ev(Log, "Break point 2");
    let bp10 = ev(Log, "Break point 10");
    let others = [
        ev(
            Log,
            "Method start: flow 1.0 ml/min; 0 %B; buffer port 1; inject Load; fraction 0.0 ml; 11 breakpoints",
        ),
        ev(Log, "Gradient; Length 50.0 ml; Target 100 %B"),
        ev(Log, "Flow 20.0 ml/min"),
        ev(Log, "Event Mark; from method"),
        ev(Log, "Breakpoints: 3"),
        ev(Inject, "Inject event (8 bytes)"),
        ev(RunState, "Run state: End  inject=IV_INJECT"),
        ev(Mark, "Break point 2"),
    ];
    assert!(is_step_like(&step) && is_step_like(&bp) && is_step_like(&bp10));
    assert!(is_step_like(&ev(Log, "  break point 3 ")));
    for e in &others {
        assert!(!is_step_like(e), "{e:?}");
        assert!(EventDisplay::NoSteps.shows(e));
        assert!(EventDisplay::All.shows(e) && !EventDisplay::None.shows(e));
    }
    assert!(!EventDisplay::NoSteps.shows(&step) && !EventDisplay::NoSteps.shows(&bp));
    assert!(EventDisplay::All.shows(&step));
    assert_eq!(EventDisplay::default(), EventDisplay::All);
    assert_eq!(
        EventDisplay::ALL.map(|d| d.label()),
        ["All", "No steps", "None"]
    );
}
