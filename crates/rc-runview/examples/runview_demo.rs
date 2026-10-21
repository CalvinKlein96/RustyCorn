//! Test harness for the run view: a window with one RunView over
//!   * a copied CSV log (`--csv <file>`, viewer mode), or
//!   * the core demo source (`--demo`, live), or
//!   * a synthetic run of N points (`--points N`), optionally growing live (`--rate HZ`).
//!
//! Other flags (for screenshots and timing): `--light`, `--overlay`, `--log`,
//! `--highlight uv`, `--select I`, `--zoom LO,HI`, `--panels "%B,Cond"`,
//! `--xmode time|volume|cv`, `--hover X,Y` (simulated mouse position),
//! `--events all|nosteps|none`, `--size WxH`, `--perf` (frame times on stderr every 2 s), `--quit-after S`,
//! `--pan W` (scripted pan of a W-wide x-window, new window every frame),
//! `--no-input` (ignore the real mouse/keyboard: scripted screenshots).
//! Never opens a serial port; never writes the user's settings.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use rc_core::{AppState, EventKind, SharedApp, TrendPoint};
use rc_plotdata::Channel;
use rc_runview::{RunView, RunViewOptions, XMode};

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}

/// A chromatogram-like point i of a synthetic run (0.1 s, 0.0017 mL per point).
fn synth_point(i: usize) -> TrendPoint {
    let t = i as f64 * 100.0;
    let min = t / 60_000.0;
    let v = i as f64 * 0.0017;
    let peak = |c: f64, w: f64, h: f64| h * (-((min - c) / w).powi(2)).exp();
    let period = 120.0; // min
    let m = min % period;
    let pctb = if m < 20.0 {
        0.0
    } else if m < 80.0 {
        (m - 20.0) / 60.0 * 100.0
    } else {
        100.0
    };
    let uv = 0.002
        + peak(10.0 + (min / period).floor() * period, 1.5, 0.3)
        + peak(45.0 + (min / period).floor() * period, 3.0, 0.9)
        + peak(60.0 + (min / period).floor() * period, 2.0, 0.5)
        + 0.001 * (i as f64 * 0.37).sin();
    TrendPoint {
        cum_time_ms: t,
        cum_vol_ml: v,
        uv,
        cond_mscm: 10.0 + pctb * 0.8 + 0.05 * (min * 0.3).sin(),
        ph: 7.4 - pctb * 0.01,
        pctb,
        pres_mpa: 0.38 + 0.03 * (min * 0.1).sin(),
        temp_degc: 22.0,
    }
}

fn synth_events(app: &mut AppState, upto: usize, from: usize) {
    for i in from..upto {
        if i % 6000 == 0 {
            app.inst.cum_time_ms = i as f64 * 100.0;
            app.inst.cum_vol_ml = i as f64 * 0.0017;
            app.push_event(
                EventKind::Log,
                format!("Flow {}.0 ml/min", 1 + (i / 6000) % 3),
            );
        }
        if i % 20_000 == 10 {
            app.push_event(EventKind::Inject, "Inject event (8 bytes)");
            app.push_event(EventKind::Log, "Injection Valve Inj");
        }
        if i % 9000 == 4500 {
            app.push_event(EventKind::Step, "Step t=0.0s  v=0.0000mL");
            app.push_event(EventKind::Log, format!("Break point {}", i / 9000 + 2));
        }
        // fractions: 3 min tubes in the middle of every 2 h block
        let min = (i as f64 * 100.0 / 60_000.0) % 120.0;
        app.inst.cum_time_ms = i as f64 * 100.0;
        app.inst.cum_vol_ml = i as f64 * 0.0017;
        if (30.0..90.0).contains(&min) {
            app.inst.fraction_vol_ml = 2.0;
            app.inst.fraction_number = ((min - 30.0) / 3.0) as i64 + 1;
        } else {
            app.inst.fraction_vol_ml = 0.0;
        }
        app.update_fraction_state();
        app.trend.push(synth_point(i));
    }
    app.touch();
}

struct Demo {
    view: RunView,
    hover: Option<egui::Pos2>,
    no_input: bool,
    perf: bool,
    last_print: Instant,
    frames: u32,
    max_cpu: f32,
    sum_cpu: f32,
    quit_at: Option<Instant>,
}

impl eframe::App for Demo {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        if self.no_input {
            // scripted runs ignore the real mouse and keyboard (the window may pop up under them)
            raw.events.retain(|e| {
                !matches!(
                    e,
                    egui::Event::PointerMoved(_)
                        | egui::Event::MouseWheel { .. }
                        | egui::Event::PointerButton { .. }
                        | egui::Event::Key { .. }
                        | egui::Event::Text(_)
                        | egui::Event::Zoom(_)
                        | egui::Event::MouseMoved(_)
                        | egui::Event::PointerGone
                )
            });
        }
        if let Some(p) = self.hover {
            raw.events.push(egui::Event::PointerMoved(p));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let t = rc_theme::palette(ui.ctx());
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(t.dock))
            .show(ui, |ui| self.view.ui(ui));
        if self.quit_at.is_some_and(|q| Instant::now() >= q) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.perf {
            ui.ctx().request_repaint(); // continuous: measure the worst case
            let cpu = frame.info().cpu_usage.unwrap_or(0.0) * 1000.0;
            self.frames += 1;
            self.max_cpu = self.max_cpu.max(cpu);
            self.sum_cpu += cpu;
            if self.last_print.elapsed() >= Duration::from_secs(2) {
                let p = self.view.perf();
                eprintln!(
                    "frames {:4}  run-view ui: last {:.2} ms, max {:.2} ms | eframe cpu/frame: mean {:.2} ms, max {:.2} ms",
                    self.frames,
                    p.last_ms,
                    p.max_ms,
                    self.sum_cpu / self.frames.max(1) as f32,
                    self.max_cpu
                );
                self.frames = 0;
                self.max_cpu = 0.0;
                self.sum_cpu = 0.0;
                self.last_print = Instant::now();
            }
        }
    }
}

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let shutdown = Arc::new(AtomicBool::new(false));
    let scratch = std::env::temp_dir().join("rc-runview-demo");
    let _ = std::fs::create_dir_all(&scratch);

    let (app, viewer): (SharedApp, bool) = if let Some(csv) = arg("--csv") {
        let path = std::path::PathBuf::from(&csv);
        let mut st = rc_core::csvlog::load_csv_log(&path)
            .unwrap_or_else(|e| panic!("cannot read {csv}: {e}"));
        st.log_filename = path.clone();
        st.log_dir = scratch.clone();
        st.touch();
        (st.shared(), true)
    } else if flag("--demo") {
        let mut st = AppState::new();
        st.live = true;
        st.demo = true;
        st.status = "Demo mode — simulated Akta".into();
        st.log_dir = scratch.clone();
        let shared = st.shared();
        let (a, s) = (shared.clone(), shutdown.clone());
        std::thread::spawn(move || rc_core::demo::run_demo(a, s));
        (shared, false)
    } else {
        let n: usize = arg("--points")
            .and_then(|s| s.parse().ok())
            .unwrap_or(300_000);
        let mut st = AppState::new();
        st.status = "● Connected (synthetic)".into();
        st.log_dir = scratch.clone();
        st.push_event(EventKind::Info, "Sample: Synthetic lysate");
        st.push_event(EventKind::Info, "Column: 5 ml HisTrap");
        st.push_event(EventKind::Info, "Column volume (mL): 5.0");
        synth_events(&mut st, n, 0);
        let rate: f64 = arg("--rate").and_then(|s| s.parse().ok()).unwrap_or(0.0);
        st.live = rate > 0.0;
        let shared = st.shared();
        if rate > 0.0 {
            let (a, s) = (shared.clone(), shutdown.clone());
            std::thread::spawn(move || {
                let mut i = n;
                while !s.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_secs_f64(1.0 / rate));
                    let mut g = a.lock();
                    synth_events(&mut g, i + 1, i);
                    i += 1;
                }
            });
        }
        (shared, false)
    };

    let start_panels: Vec<String> = arg("--panels")
        .map(|s| {
            s.split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect()
        })
        .unwrap_or_else(|| {
            ["%B", "Cond", "Pressure", "Flow"]
                .iter()
                .map(|s| s.to_string())
                .collect()
        });
    let mut view = RunView::new(
        app.clone(),
        RunViewOptions {
            viewer,
            start_panels,
        },
    );
    view.set_refresh_hz(arg("--refresh").and_then(|s| s.parse().ok()).unwrap_or(4.0));
    view.set_overlay(flag("--overlay"));
    view.set_show_log(flag("--log"));
    if viewer {
        view.set_connection_status(Some(
            "Serial port /nonexistent/x not available — retrying".into(),
        ));
    }
    view.set_x_mode(match arg("--xmode").as_deref() {
        Some("volume") => XMode::Volume,
        Some("cv") => XMode::ColumnVolumes,
        _ => XMode::Time,
    });
    view.set_highlight(match arg("--highlight").as_deref() {
        Some("uv") => Some(Channel::Uv),
        Some("cond") => Some(Channel::Cond),
        Some("pctb") => Some(Channel::Pctb),
        _ => None,
    });
    view.set_event_display(match arg("--events").as_deref() {
        Some("nosteps") => rc_plotdata::EventDisplay::NoSteps,
        Some("none") => rc_plotdata::EventDisplay::None,
        _ => rc_plotdata::EventDisplay::All,
    });
    let select: Option<usize> = arg("--select").and_then(|s| s.parse().ok());
    let zoom = arg("--zoom").and_then(|s| {
        let (a, b) = s.split_once(',')?;
        Some((a.parse::<f64>().ok()?, b.parse::<f64>().ok()?))
    });
    let hover = arg("--hover").and_then(|s| {
        let (a, b) = s.split_once(',')?;
        Some(egui::pos2(a.parse().ok()?, b.parse().ok()?))
    });
    let (w, h) = arg("--size")
        .and_then(|s| {
            let (a, b) = s.split_once('x')?;
            Some((a.parse::<f32>().ok()?, b.parse::<f32>().ok()?))
        })
        .unwrap_or((1440.0, 900.0));
    let dark = !flag("--light");
    let perf = flag("--perf");
    let no_input = flag("--no-input");
    let pan_width: Option<f64> = arg("--pan").and_then(|s| s.parse().ok());
    let quit_at = arg("--quit-after")
        .and_then(|s| s.parse::<f64>().ok())
        .map(|s| Instant::now() + Duration::from_secs_f64(s));

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("RustyCorn runview demo")
            .with_inner_size([w, h]),
        ..Default::default()
    };
    let res = eframe::run_native(
        "RustyCorn runview demo",
        options,
        Box::new(move |cc| {
            rc_theme::install_fonts(&cc.egui_ctx);
            rc_theme::apply(&cc.egui_ctx, dark);
            let demo = Demo {
                view,
                hover,
                no_input,
                perf,
                last_print: Instant::now(),
                frames: 0,
                max_cpu: 0.0,
                sum_cpu: 0.0,
                quit_at,
            };
            Ok(Box::new(Deferred {
                inner: Pending {
                    pan_width,
                    select,
                    zoom,
                },
                demo,
                frame: 0,
            }))
        }),
    );
    shutdown.store(true, Ordering::Relaxed);
    res
}

/// Selection and zoom requests applied after the first frames (they need synced data).
struct Pending {
    pan_width: Option<f64>,
    select: Option<usize>,
    zoom: Option<(f64, f64)>,
}

struct Deferred {
    demo: Demo,
    inner: Pending,
    frame: u32,
}

impl eframe::App for Deferred {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.demo.raw_input_hook(ctx, raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.frame += 1;
        if self.frame == 3 {
            if let Some(i) = self.inner.select.take() {
                self.demo.view.select_point(i);
            }
            if let Some((a, b)) = self.inner.zoom.take() {
                self.demo.view.zoom_x(a, b);
            }
        }
        if let Some(w) = self.inner.pan_width {
            // scripted pan: a new x-window every frame (worst case: re-downsample each frame)
            let x0 = (self.frame as f64 * 0.5) % w;
            self.demo.view.zoom_x(x0, x0 + w);
            ui.ctx().request_repaint();
        }
        if self.frame < 5 {
            ui.ctx().request_repaint();
        }
        self.demo.ui(ui, frame);
    }
}
