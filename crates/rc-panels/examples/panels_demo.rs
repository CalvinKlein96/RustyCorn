//! Visual check of the run-view panels: side panel with switchable instrument
//! states, the run log with sample events, the status bar in each state, and a
//! light/dark toggle.
//!
//! `cargo run -p rc-panels --example panels_demo -- [--preset N] [--status N] [--light] [--rec] [--xs N]`

use std::collections::BTreeMap;

use egui::{Margin, RichText, Stroke};
use rc_core::{AktaEvent, EventKind, InjectValue, InstrumentState, TrendPoint};
use rc_panels::{PanelData, RunLogState, ShownMode, StatusInfo};

struct Preset {
    name: &'static str,
    state: InstrumentState,
    point: Option<TrendPoint>,
    flow: Option<f64>,
    mode: ShownMode,
    viewer: bool,
}

fn presets() -> Vec<Preset> {
    let base = InstrumentState {
        run_state: "Run".into(),
        cum_time_ms: 754_000.0,
        cum_vol_ml: 12.57,
        run_time_ms: 754_000.0,
        run_vol_ml: 12.57,
        prog_name: "PrimeMeth 3 (Q gradient)".into(),
        version: "V2.01".into(),
        step_time_ms: 134_000.0,
        step_vol_ml: 2.233,
        buffer_port: 1,
        buffer_port_known: true,
        inject_value: InjectValue::Load,
        ..Default::default()
    };
    let pt = |pctb: f64| TrendPoint {
        cum_time_ms: 754_000.0,
        cum_vol_ml: 12.57,
        uv: 0.0543,
        cond_mscm: 11.99,
        ph: 7.31,
        pctb,
        pres_mpa: 0.392,
        temp_degc: 22.0,
    };
    vec![
        Preset {
            name: "Live · load",
            state: base.clone(),
            point: Some(pt(5.0)),
            flow: Some(1.0),
            mode: ShownMode::Live,
            viewer: false,
        },
        Preset {
            name: "Live · inject → frac",
            state: InstrumentState {
                inject_value: InjectValue::Inject,
                buffer_port: 3,
                fraction_vol_ml: 1.0,
                fraction_number: 4,
                ..base.clone()
            },
            point: Some(pt(45.0)),
            flow: Some(1.0),
            mode: ShownMode::Live,
            viewer: false,
        },
        Preset {
            name: "Past · waste, port 8 assumed",
            state: InstrumentState {
                inject_value: InjectValue::Waste,
                buffer_port: 8,
                buffer_port_known: false,
                prog_name: String::new(),
                version: String::new(),
                ..base.clone()
            },
            point: Some(pt(100.0)),
            flow: Some(0.5),
            mode: ShownMode::Past,
            viewer: true,
        },
        Preset {
            name: "End of log · unknown valve",
            state: InstrumentState {
                inject_value: InjectValue::Unknown,
                buffer_port_known: false,
                ..base.clone()
            },
            point: Some(pt(0.0)),
            flow: None,
            mode: ShownMode::EndOfLog,
            viewer: true,
        },
        Preset {
            name: "No data",
            state: InstrumentState::default(),
            point: None,
            flow: None,
            mode: ShownMode::EndOfLog,
            viewer: true,
        },
    ]
}

const STATUSES: [&str; 5] = [
    "● Connected on /dev/ttyUSB0",
    "Waiting for Akta on /dev/ttyUSB0",
    "Serial port /dev/ttyUSB0 not available — retrying",
    "Demo mode — simulated data",
    "",
];

fn sample_events() -> Vec<AktaEvent> {
    let e = |t_s: f64, v: f64, kind: EventKind, text: &str| AktaEvent {
        cum_time_ms: t_s * 1000.0,
        cum_vol_ml: v,
        kind,
        text: text.to_string(),
    };
    let mut ev = vec![
        e(0.0, 0.0, EventKind::Info, "Sample: Lysate batch 7"),
        e(0.0, 0.0, EventKind::Info, "Column: HiTrap Q HP 5 mL"),
        e(0.0, 0.0, EventKind::Info, "Port 3: NaCl 1 M"),
        e(0.0, 0.0, EventKind::Connect, "Akta connected"),
        e(
            0.0,
            0.0,
            EventKind::Log,
            "Method start: flow 1.0 ml/min; 0 %B; buffer port 1; inject Load; fraction 0.0 ml; 7 breakpoints",
        ),
        e(
            0.0,
            0.0,
            EventKind::RunState,
            "Run state: Run  inject=IV_LOAD",
        ),
        e(0.7, 0.013, EventKind::Log, "Flow 1.0 ml/min"),
        e(120.0, 2.0, EventKind::Step, "Step t=120.0s  v=2.0000mL"),
        e(180.0, 3.0, EventKind::Inject, "Inject event (2 bytes)"),
        e(240.0, 4.0, EventKind::Mark, "Sample loaded"),
        e(300.0, 5.0, EventKind::Log, "Fraction size 1.0 ml"),
    ];
    for n in 1..=12 {
        ev.push(e(
            300.0 + 30.0 * n as f64,
            5.0 + 0.5 * n as f64,
            EventKind::Tubefeed,
            &format!("Feed Tube Nr {n}"),
        ));
    }
    ev.push(e(670.0, 11.0, EventKind::Log, "Buffer Valve Pos 3"));
    ev.push(e(700.0, 11.6, EventKind::Mark, "Peak 1"));
    ev.push(e(
        754.0,
        12.57,
        EventKind::Log,
        "Link: no response from Akta — waiting",
    ));
    ev
}

struct Demo {
    presets: Vec<Preset>,
    preset: usize,
    status: usize,
    dark: bool,
    recording: bool,
    xs: f64,
    events: Vec<AktaEvent>,
    info: BTreeMap<String, String>,
    log: RunLogState,
    clicked: Option<usize>,
}

impl eframe::App for Demo {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let t = rc_theme::palette(ui.ctx());
        egui::Panel::top("controls")
            .frame(
                egui::Frame::new()
                    .fill(t.chrome)
                    .inner_margin(Margin::same(10)),
            )
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (i, p) in self.presets.iter().enumerate() {
                        if rc_theme::tab(ui, p.name, i == self.preset).clicked() {
                            self.preset = i;
                        }
                    }
                });
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    for (i, s) in STATUSES.iter().enumerate() {
                        let label = if s.is_empty() { "(opened run)" } else { s };
                        if ui.selectable_label(i == self.status, label).clicked() {
                            self.status = i;
                        }
                    }
                    ui.checkbox(&mut self.recording, "Recording");
                    if ui.checkbox(&mut self.dark, "Dark").changed() {
                        rc_theme::apply(ui.ctx(), self.dark);
                    }
                    egui::ComboBox::from_id_salt("xs")
                        .selected_text(rc_plotdata::xlabel(self.xs))
                        .show_ui(ui, |ui| {
                            for xs in [0.0, 1.0, 5.0] {
                                ui.selectable_value(&mut self.xs, xs, rc_plotdata::xlabel(xs));
                            }
                        });
                });
            });
        let p = &self.presets[self.preset];
        let status = STATUSES[self.status];
        egui::Panel::bottom("status").frame(egui::Frame::new().fill(t.dock).inner_margin(Margin::same(10))).show(ui, |ui| {
            // as the run view places it
            egui::Frame::new()
                .fill(t.card)
                .stroke(Stroke::new(1.0, t.card_border))
                .corner_radius(t.radius_sm as u8)
                .inner_margin(Margin::symmetric(8, 5))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    rc_panels::status_bar(
                        ui,
                        &StatusInfo {
                            status: if p.viewer { "" } else { status },
                            recording: self.recording,
                            log_filename: "/home/lab/logs/akta_demo_20261014_085438.csv",
                            viewer: p.viewer,
                            n_points: 6034,
                            run_time_ms: 1_816_000.0,
                            help: "Drag: zoom · Right-drag: pan · Ctrl-click: reset · Alt/Shift-click: inspect moment",
                        },
                    );
                });
        });
        egui::Panel::right("side")
            .exact_size(390.0)
            .frame(
                egui::Frame::new()
                    .fill(t.dock)
                    .inner_margin(Margin::same(10)),
            )
            .show(ui, |ui| {
                let d = PanelData {
                    state: &p.state,
                    point: p.point.as_ref(),
                    flow: p.flow,
                    info: &self.info,
                    mode: p.mode,
                    viewer: p.viewer,
                    edit_label: Some(if p.viewer { "Edit run info" } else { "Edit" }),
                };
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| rc_panels::side_panel(ui, &d));
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(t.dock)
                    .inner_margin(Margin::same(14)),
            )
            .show(ui, |ui| {
                rc_theme::section_title(ui, "Run log");
                ui.add_space(4.0);
                if let Some(i) = rc_panels::run_log(ui, &self.events, self.xs, &mut self.log, 9) {
                    self.clicked = Some(i);
                }
                ui.add_space(8.0);
                let msg = match self.clicked {
                    Some(i) => format!("Clicked event {i}: {}", self.events[i].text),
                    None => "Click a log line to inspect it".to_string(),
                };
                ui.label(RichText::new(msg).color(t.text_dim));
                ui.add_space(12.0);
                rc_theme::section_title(ui, "Flow schematic, narrow");
                ui.allocate_ui(egui::vec2(300.0, 320.0), |ui| {
                    rc_panels::flow_schematic(ui, &p.state, p.point.as_ref(), &self.info);
                });
            });
    }
}

fn arg(args: &[String], name: &str) -> Option<usize> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    let presets = presets();
    let preset = arg(&args, "--preset").unwrap_or(0).min(presets.len() - 1);
    let status = arg(&args, "--status").unwrap_or(0).min(STATUSES.len() - 1);
    let dark = !args.iter().any(|a| a == "--light");
    let recording = args.iter().any(|a| a == "--rec");
    let xs = [0.0, 1.0, 5.0][arg(&args, "--xs").unwrap_or(0).min(2)];
    let events = sample_events();
    let mut info = BTreeMap::new();
    for e in events.iter().filter(|e| e.kind == EventKind::Info) {
        if let Some((k, v)) = e.text.split_once(": ") {
            info.insert(k.to_string(), v.to_string());
        }
    }
    info.insert("User".into(), "Alex".into());
    info.insert("Column volume (mL)".into(), "5.0".into());
    info.insert(
        "Notes".into(),
        "Equilibrate 5 CV before loading; elute with a 20 CV linear gradient to 100 %B.".into(),
    );
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("RustyCorn panels demo")
            .with_inner_size([1280.0, 900.0]),
        ..Default::default()
    };
    eframe::run_native(
        "RustyCorn panels demo",
        options,
        Box::new(move |cc| {
            rc_theme::install_fonts(&cc.egui_ctx);
            rc_theme::apply(&cc.egui_ctx, dark);
            Ok(Box::new(Demo {
                presets,
                preset,
                status,
                dark,
                recording,
                xs,
                events,
                info,
                log: RunLogState::default(),
                clicked: None,
            }))
        }),
    )
}
