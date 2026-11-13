use super::*;

/// Folder with real run logs for the export tests (set RC_EXAMPLE_LOGS=<dir>).
const LOGS_ENV: &str = "RC_EXAMPLE_LOGS";

/// A real log, copied first so the original is never touched.
/// None when no example log folder is set or the log is not in it.
fn real_log(name: &str, dir: &Path) -> Option<rc_core::AppState> {
    let Some(logs) = std::env::var_os(LOGS_ENV) else {
        eprintln!("skipping: {LOGS_ENV} not set");
        return None;
    };
    let src = Path::new(&logs).join(name);
    if !src.exists() {
        eprintln!("skipping: {} not found", src.display());
        return None;
    }
    let dst = dir.join(name);
    std::fs::copy(&src, &dst).expect("copy log");
    Some(rc_core::csvlog::load_csv_log(&dst).expect("load log"))
}

fn request(
    app: &rc_core::AppState,
    channels: &[Channel],
    xs: XScale,
    highlighted: Option<Channel>,
) -> ExportRequest {
    let xmax = app
        .trend
        .last()
        .map_or(1.0, |p| rc_plotdata::trend_x(p, xs));
    ExportRequest {
        trend: app.trend.clone(),
        events: app.events.clone(),
        fractions: app.fractions.clone(),
        panels: channels
            .iter()
            .map(|&ch| {
                let pts = rc_plotdata::channel_points(ch, &app.trend, &app.events, xs);
                PanelSpec {
                    channel: ch,
                    ylim: rc_plotdata::auto_ylim(ch, rc_plotdata::y_extrema(&pts)),
                }
            })
            .collect(),
        xs,
        xlim: rc_plotdata::padded(0.0, xmax, 0.02),
        overlay_ylim: rc_plotdata::OVERLAY_YLIMS,
        highlighted,
        title: Some("akta_20261020_135011.csv".to_string()),
        events_shown: EventDisplay::All,
        show_fractions: true,
    }
}

/// Where examples are written for a look (set RC_EXPORT_EXAMPLES=<dir>).
fn example_dir() -> Option<PathBuf> {
    std::env::var_os("RC_EXPORT_EXAMPLES").map(PathBuf::from)
}

#[test]
fn figure_path_format() {
    let p = figure_path(Path::new("/data/logs"), "akta", 3);
    let name = p.file_name().and_then(|n| n.to_str()).expect("name");
    assert_eq!(p.parent(), Some(Path::new("/data/logs")));
    assert!(name.ends_with("_akta_figure_3.pdf"), "{name}");
    let stamp = &name[..name.len() - "_akta_figure_3.pdf".len()];
    assert_eq!(stamp.len(), 15, "{stamp}"); // yyyymmdd_HHMMSS
    assert_eq!(&stamp[8..9], "_");
    assert!(
        stamp
            .chars()
            .filter(|c| *c != '_')
            .all(|c| c.is_ascii_digit())
    );
    let p = figure_path(Path::new(""), "", 1);
    assert!(p.to_string_lossy().ends_with("_akta_figure_1.pdf"));
}

#[test]
fn empty_run_is_not_saved() {
    let dir = tempfile::tempdir().expect("tempdir");
    let req = ExportRequest {
        panels: vec![PanelSpec {
            channel: Channel::Uv,
            ylim: (0.0, 1.0),
        }],
        xlim: (0.0, 1.0),
        ..Default::default()
    };
    let path = dir.path().join("x.pdf");
    assert!(save_pdf(&req, &path).is_err());
    assert!(!path.exists());
}

fn synthetic() -> ExportRequest {
    let trend: Vec<TrendPoint> = (0..2000)
        .map(|i| {
            let t = i as f64 * 0.01;
            TrendPoint {
                cum_time_ms: t * 60_000.0,
                cum_vol_ml: t,
                uv: (-(t - 10.0).powi(2)).exp() + if i == 1000 { f64::NAN } else { 0.0 },
                cond_mscm: 5.0 + t,
                ph: 7.0,
                pctb: (t * 5.0).min(100.0),
                pres_mpa: 0.3,
                temp_degc: 20.0,
            }
        })
        .collect();
    let ev = |t: f64, kind, text: &str| AktaEvent {
        cum_time_ms: t * 60_000.0,
        cum_vol_ml: t,
        kind,
        text: text.into(),
    };
    use rc_core::EventKind::*;
    ExportRequest {
        events: vec![
            ev(0.0, Info, "Sample: Lysozyme & <friends>"),
            ev(0.5, Log, "Flow 1.0 ml/min"),
            ev(2.0, Inject, "Inject event (8 bytes)"),
            ev(5.0, RunState, "Run state: Pause  inject=IV_LOAD"),
            ev(6.0, Step, "Step"),
            ev(8.0, Log, "Flow 2.0 ml/min"),
            ev(9.0, Mark, "peak"),
        ],
        fractions: vec![
            Fraction {
                fraction_number: 1,
                start_time_ms: 600_000.0,
                start_vol_ml: 10.0,
                end_time_ms: Some(660_000.0),
                end_vol_ml: Some(11.0),
            },
            Fraction {
                fraction_number: 2,
                start_time_ms: 660_000.0,
                start_vol_ml: 11.0,
                end_time_ms: None,
                end_vol_ml: None,
            },
        ],
        panels: Channel::ALL
            .iter()
            .map(|&ch| PanelSpec {
                channel: ch,
                ylim: (f64::NAN, f64::NAN),
            })
            .collect(),
        xs: 0.0,
        xlim: (5.0, 15.0),
        overlay_ylim: rc_plotdata::OVERLAY_YLIMS,
        highlighted: Some(Channel::Cond),
        title: Some("synthetic".into()),
        events_shown: EventDisplay::All,
        show_fractions: true,
        trend,
    }
}

#[test]
fn svg_has_every_part() {
    let req = synthetic();
    let svg = render_svg(&req);
    // one clipped area per panel plus the overlay
    assert_eq!(svg.matches("<clipPath").count(), Channel::ALL.len() + 1);
    // page size 1000 × (200·n + 500), plus the legend below the overlay
    let h = 1700.0 + legend_height(&req);
    assert!(h > 1700.0 + LEGEND_ROW_H);
    assert!(
        svg.contains(&format!(r#"width="1000" height="{h}""#)),
        "{}",
        &svg[..300]
    );
    for label in [
        "UV (AU)",
        "Conductivity (mS/cm)",
        "Pressure (MPa)",
        "Flow (mL/min)",
        "Time (min)",
        "Cond (mS/cm)",
    ] {
        assert!(svg.contains(&format!(">{label}<")), "{label}");
    }
    // text is escaped, run info caption present
    assert!(svg.contains("Sample Lysozyme &amp; &lt;friends&gt;"));
    // legend entries with ranges, without the "━━" bullet
    assert!(
        svg.contains(">UV   (0–1 AU)<") || svg.contains(">UV   ("),
        "{}",
        &svg[..200]
    );
    assert!(!svg.contains('━'));
    // fraction numbers on %B
    assert!(svg.contains(">1</text>") && svg.contains(">2</text>"));
    // the info event is not drawn, nor those outside x 5–15: 4 event lines per area (UV, overlay)
    assert_eq!(svg.matches(r#"stroke-opacity="0.55""#).count(), 8);
    // it parses and converts
    let pdf = pdf_bytes(&req).expect("pdf");
    assert!(pdf.starts_with(b"%PDF-"));
    if let Some(out) = example_dir() {
        std::fs::write(out.join("synthetic.pdf"), pdf).expect("write example");
    }
}

/// Height the overlay legend adds to the page (as render_svg lays it out).
fn legend_height(req: &ExportRequest) -> f64 {
    let svg = render_svg(req);
    let h: f64 = svg
        .split_once(r#"height=""#)
        .and_then(|(_, r)| r.split_once('"'))
        .and_then(|(v, _)| v.parse().ok())
        .expect("page height");
    h - (200.0 * req.panels.len().max(1) as f64 + 500.0)
}

/// y of every `<text>` whose content is `label`.
fn text_ys(svg: &str, label: &str) -> Vec<f64> {
    svg.lines()
        .filter(|l| l.starts_with("<text") && l.contains(&format!(">{label}</text>")))
        .filter_map(|l| {
            l.split_once(r#" y=""#)
                .and_then(|(_, r)| r.split_once('"'))
                .and_then(|(v, _)| v.parse().ok())
        })
        .collect()
}

#[test]
fn event_display_choice_is_followed() {
    let mut req = synthetic();
    let lines = |req: &ExportRequest| render_svg(req).matches(r#"stroke-opacity="0.55""#).count();
    assert_eq!(lines(&req), 8);
    // the Step (and a "Break point N" log line) go; everything else stays
    req.events.push(AktaEvent {
        cum_time_ms: 7.0 * 60_000.0,
        cum_vol_ml: 7.0,
        kind: rc_core::EventKind::Log,
        text: "Break point 3".into(),
    });
    assert_eq!(lines(&req), 10);
    req.events_shown = EventDisplay::NoSteps;
    assert_eq!(lines(&req), 6);
    req.events_shown = EventDisplay::None;
    let svg = render_svg(&req);
    assert_eq!(svg.matches(r#"stroke-opacity="0.55""#).count(), 0);
    // fraction shading and numbers stay
    assert!(svg.contains(">1</text>") && svg.contains(">2</text>"));
    assert!(svg.contains(&format!(r#"fill-opacity="{}""#, rc_plotdata::FRAC_ALPHA)));
}

#[test]
fn grid_in_every_panel() {
    let req = synthetic();
    let svg = render_svg(&req);
    assert_eq!(
        svg.matches(r#"<path class="grid""#).count(),
        Channel::ALL.len() + 1
    );
    assert!(svg.contains(&format!(
        r#"stroke-opacity="{GRID_ALPHA}" stroke-width="{GRID_WIDTH}""#
    )));
    let mut one = synthetic();
    one.panels.truncate(1);
    assert_eq!(render_svg(&one).matches(r#"<path class="grid""#).count(), 2);
}

#[test]
fn legend_sits_below_the_overlay() {
    let req = synthetic();
    let svg = render_svg(&req);
    let xlab = text_ys(&svg, "Time (min)");
    assert_eq!(xlab.len(), 2, "stacked and overlay x labels");
    let overlay_xlabel = xlab.iter().copied().fold(f64::MIN, f64::max);
    let page_h = 1700.0 + legend_height(&req);
    let legend_ys: Vec<f64> = svg
        .lines()
        .filter(|l| l.starts_with("<text") && l.contains("   ("))
        .filter_map(|l| {
            l.split_once(r#" y=""#)
                .and_then(|(_, r)| r.split_once('"'))
                .and_then(|(v, _)| v.parse().ok())
        })
        .collect();
    assert_eq!(legend_ys.len(), Channel::ALL.len());
    for y in legend_ys {
        assert!(
            y > overlay_xlabel + FS_LEGEND && y < page_h - PAD / 2.0,
            "legend text at {y}, x label at {overlay_xlabel}, page {page_h}"
        );
    }
    // the six entries do not fit one row of the plot width: they wrap, centred
    let traces: Vec<OverlayTrace> = Channel::ALL
        .iter()
        .map(|&ch| OverlayTrace {
            ch,
            scaled: vec![],
            range: (0.0, 123.456),
        })
        .collect();
    let l = legend_layout(&traces, None, 400.0);
    assert!(l.rows > 1);
    assert_eq!(l.height, LEGEND_TOP + LEGEND_ROW_H * l.rows as f64);
    assert!(l.entries.iter().all(|&(_, x, _)| (0.0..400.0).contains(&x)));
    let wide = legend_layout(&traces, None, 5000.0);
    assert_eq!(wide.rows, 1);
    assert!(legend_layout(&[], None, 500.0).height == 0.0);
}

#[test]
fn hidden_trace_is_not_highlighted() {
    let mut req = synthetic();
    req.panels.truncate(1); // UV only
    req.highlighted = Some(Channel::Flow);
    let svg = render_svg(&req);
    assert!(svg.contains(">Scaled (% of range)<"));
    assert!(!svg.contains(&format!(
        r#"stroke-opacity="{}""#,
        rc_plotdata::OVERLAY_DIM_ALPHA
    )));
}

#[test]
fn zoomed_window_keeps_only_what_is_seen() {
    let pts: Vec<Pt> = (0..100).map(|i| [i as f64, 0.0]).collect();
    let w = in_window(&pts, (10.5, 20.5));
    assert_eq!(w.first().map(|p| p[0]), Some(10.0));
    assert_eq!(w.last().map(|p| p[0]), Some(21.0));
    assert_eq!(w.len(), 12);
    // a long segment across the whole window is kept
    assert_eq!(
        in_window(&[[0.0, 1.0], [100.0, 2.0]], (40.0, 50.0)).len(),
        2
    );
    assert!(in_window(&pts, (200.0, 300.0)).is_empty());
}

#[test]
fn base64_encoding() {
    assert_eq!(base64(b""), "");
    assert_eq!(base64(b"f"), "Zg==");
    assert_eq!(base64(b"fo"), "Zm8=");
    assert_eq!(base64(b"foo"), "Zm9v");
    assert_eq!(base64(b"foobar"), "Zm9vYmFy");
}

#[test]
fn text_is_measured_with_the_embedded_font() {
    let w = text_width("100", 11.0, Weight::Regular);
    assert!(w > 15.0 && w < 25.0, "{w}");
    assert!(
        text_width("Wide title", 13.0, Weight::SemiBold)
            > text_width("Wide title", 11.0, Weight::Regular)
    );
}

#[test]
fn export_of_a_real_log() {
    let dir = tempfile::tempdir().expect("tempdir");
    let Some(app) = real_log("akta_20261020_135011.csv", dir.path()) else {
        return;
    };
    let out = example_dir().unwrap_or_else(|| dir.path().to_path_buf());
    type Case<'a> = (
        &'a str,
        &'a [Channel],
        XScale,
        Option<Channel>,
        EventDisplay,
    );
    let cases: [Case; 5] = [
        ("all_time", &Channel::ALL, 0.0, None, EventDisplay::All),
        (
            "uv_pctb_volume_highlight",
            &[Channel::Uv, Channel::Pctb, Channel::Flow],
            1.0,
            Some(Channel::Uv),
            EventDisplay::All,
        ),
        (
            "uv_only_cv",
            &[Channel::Uv],
            5.0,
            Some(Channel::Flow),
            EventDisplay::All,
        ),
        (
            "uv_pctb_nosteps",
            &[Channel::Uv, Channel::Pctb, Channel::Cond],
            0.0,
            None,
            EventDisplay::NoSteps,
        ),
        (
            "uv_pctb_noevents",
            &[Channel::Uv, Channel::Pctb],
            1.0,
            None,
            EventDisplay::None,
        ),
    ];
    for (name, chans, xs, hl, shown) in cases {
        let mut req = request(&app, chans, xs, hl);
        req.events_shown = shown;
        let path = out.join(format!("{name}.pdf"));
        save_pdf(&req, &path).expect("save");
        let pdf = std::fs::read(&path).expect("read back");
        assert!(
            pdf.starts_with(b"%PDF-") && pdf.len() > 10_000,
            "{name}: {} bytes",
            pdf.len()
        );
        if example_dir().is_some() {
            std::fs::write(out.join(format!("{name}.svg")), render_svg(&req)).expect("svg");
        }
    }
}

#[test]
fn export_of_a_long_real_log() {
    let dir = tempfile::tempdir().expect("tempdir");
    let Some(app) = real_log("akta_20261020_155048.csv", dir.path()) else {
        return;
    };
    let out = example_dir().unwrap_or_else(|| dir.path().to_path_buf());
    let mut req = request(
        &app,
        &[Channel::Uv, Channel::Cond, Channel::Pres],
        0.0,
        None,
    );
    req.title = Some("akta_20261020_155048.csv".into());
    let path = out.join("long_run.pdf");
    save_pdf(&req, &path).expect("save");
    assert!(std::fs::read(&path).expect("read").starts_with(b"%PDF-"));
}

#[test]
fn fraction_numbers_in_the_panel_and_the_overlay() {
    let mut req = synthetic();
    for f in &mut req.fractions {
        f.fraction_number += 40; // 41, 42: numbers nothing else on the page shows
    }
    req.xlim = (0.0, 20.0); // the middle of the open fraction 42 in view
    let count =
        |req: &ExportRequest, n: &str| render_svg(req).matches(&format!(">{n}</text>")).count();
    // the %B panel and the overlay
    assert_eq!(count(&req, "41"), 2);
    assert_eq!(count(&req, "42"), 2);
    // without the %B panel, still in the overlay
    req.panels.retain(|p| p.channel != Channel::Pctb);
    assert_eq!(count(&req, "41"), 1);
    assert_eq!(count(&req, "42"), 1);
    // switched off: neither numbers nor shading
    let shading = |req: &ExportRequest| render_svg(req).matches("fill-opacity=\"0.14\"").count();
    assert!(shading(&req) > 0);
    req.show_fractions = false;
    assert_eq!(count(&req, "41") + count(&req, "42"), 0);
    assert_eq!(shading(&req), 0);
}
