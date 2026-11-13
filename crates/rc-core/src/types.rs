//! Shared data structures.
//!
//! A single [`AppState`] lives behind a mutex
//! ([`SharedApp`]); the link/demo thread mutates it and bumps
//! [`AppState::generation`], and the GUI reads it every frame and caches
//! derived data by generation.

use std::collections::BTreeMap;
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::csvlog::CsvLog;

/// One trend data sample with cumulative time/volume coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct TrendPoint {
    /// cumulative run time from base, ms
    pub cum_time_ms: f64,
    /// cumulative run volume from base, mL
    pub cum_vol_ml: f64,
    /// UV absorbance, AU
    pub uv: f64,
    /// conductivity, mS/cm
    pub cond_mscm: f64,
    /// pH (raw ÷ 100 — scaling documented, unconfirmed on hardware)
    pub ph: f64,
    /// gradient %B (0–100)
    pub pctb: f64,
    /// pressure, MPa (raw ÷ 1000)
    pub pres_mpa: f64,
    /// temperature, °C
    pub temp_degc: f64,
}

/// A fraction collection period with start/end time and volume.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fraction {
    pub fraction_number: i64,
    /// cumulative run time when the fraction started
    pub start_time_ms: f64,
    /// cumulative run volume when the fraction started
    pub start_vol_ml: f64,
    /// cumulative run time when the fraction stopped (if completed)
    pub end_time_ms: Option<f64>,
    /// cumulative run volume when the fraction stopped (if completed)
    pub end_vol_ml: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EventKind {
    Connect,
    Disconnect,
    RunState,
    Inject,
    Tubefeed,
    Log,
    Step,
    /// user event mark, added during a run
    Mark,
    /// run documentation ("Sample: …", "Port 3: …"), not drawn in plots
    Info,
}

impl EventKind {
    pub const ALL: [EventKind; 9] = [
        EventKind::Connect,
        EventKind::Disconnect,
        EventKind::RunState,
        EventKind::Inject,
        EventKind::Tubefeed,
        EventKind::Log,
        EventKind::Step,
        EventKind::Mark,
        EventKind::Info,
    ];

    /// Name used in the CSV `type` column ("EVT_LOG", …).
    pub fn csv_name(self) -> &'static str {
        match self {
            EventKind::Connect => "EVT_CONNECT",
            EventKind::Disconnect => "EVT_DISCONNECT",
            EventKind::RunState => "EVT_RUN_STATE",
            EventKind::Inject => "EVT_INJECT",
            EventKind::Tubefeed => "EVT_TUBEFEED",
            EventKind::Log => "EVT_LOG",
            EventKind::Step => "EVT_STEP",
            EventKind::Mark => "EVT_MARK",
            EventKind::Info => "EVT_INFO",
        }
    }

    pub fn from_csv_name(s: &str) -> Option<EventKind> {
        EventKind::ALL.into_iter().find(|k| k.csv_name() == s)
    }
}

impl std::fmt::Display for EventKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.csv_name())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum InjectValue {
    Load,
    Inject,
    Waste,
    #[default]
    Unknown,
}

impl InjectValue {
    pub const ALL: [InjectValue; 4] = [
        InjectValue::Load,
        InjectValue::Inject,
        InjectValue::Waste,
        InjectValue::Unknown,
    ];

    /// Name as written into event texts ("inject=IV_LOAD"); part of the log format.
    pub fn name(self) -> &'static str {
        match self {
            InjectValue::Load => "IV_LOAD",
            InjectValue::Inject => "IV_INJECT",
            InjectValue::Waste => "IV_WASTE",
            InjectValue::Unknown => "IV_UNKNOWN",
        }
    }

    pub fn from_name(s: &str) -> Option<InjectValue> {
        InjectValue::ALL.into_iter().find(|v| v.name() == s)
    }

    /// Short upper-case label: "LOAD", "INJECT", "WASTE", "UNKNOWN".
    pub fn label(self) -> &'static str {
        match self {
            InjectValue::Load => "LOAD",
            InjectValue::Inject => "INJECT",
            InjectValue::Waste => "WASTE",
            InjectValue::Unknown => "UNKNOWN",
        }
    }
}

impl std::fmt::Display for InjectValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// A discrete instrument event tagged with run coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct AktaEvent {
    pub cum_time_ms: f64,
    pub cum_vol_ml: f64,
    pub kind: EventKind,
    pub text: String,
}

/// Live instrument status.
#[derive(Clone, Debug, PartialEq)]
pub struct InstrumentState {
    pub run_state: String,
    pub cum_time_ms: f64,
    pub cum_vol_ml: f64,
    pub run_time_ms: f64,
    pub run_vol_ml: f64,
    pub inject_value: InjectValue,
    pub prog_name: String,
    pub version: String,
    pub step_time_ms: f64,
    pub step_vol_ml: f64,
    pub fraction_number: i64,
    pub fraction_vol_ml: f64,
    /// index into `AppState::fractions` of the active fraction (0-based), or None if idle
    pub current_fraction_idx: Option<usize>,
    /// buffer valve inlet port 1–8 feeding the A side
    pub buffer_port: i64,
    /// false until a "Buffer Valve Pos N" message confirms `buffer_port`
    pub buffer_port_known: bool,
}

impl Default for InstrumentState {
    /// The buffer valve is only reported when it moves, so start from the standard
    /// port 1 and mark it as assumed until the first report.
    fn default() -> Self {
        InstrumentState {
            run_state: "---".to_string(),
            cum_time_ms: 0.0,
            cum_vol_ml: 0.0,
            run_time_ms: 0.0,
            run_vol_ml: 0.0,
            inject_value: InjectValue::Unknown,
            prog_name: String::new(),
            version: String::new(),
            step_time_ms: 0.0,
            step_vol_ml: 0.0,
            fraction_number: 1,
            fraction_vol_ml: 0.0,
            current_fraction_idx: None,
            buffer_port: 1,
            buffer_port_known: false,
        }
    }
}

/// Application-wide shared state: the instrument state, the run's trend and
/// events, the open log and raw capture, the method store and the connection
/// status.
///
/// Every mutation must end with [`AppState::touch`] so the GUI notices it.
/// The helper methods below already do.
#[derive(Debug)]
pub struct AppState {
    pub trend: Vec<TrendPoint>,
    pub events: Vec<AktaEvent>,
    pub inst: InstrumentState,
    pub fractions: Vec<Fraction>,

    /// multi-frame log message buffer (OPC 0x8005–0x8007)
    pub log_buf: String,
    /// open CSV log (None until a log is started, and for opened/replayed runs)
    pub log: Option<CsvLog>,
    /// log filename shown in the GUI status bar
    pub log_filename: PathBuf,
    /// why the live log could not be created (`log` stays None and nothing is
    /// recorded while data keep coming); None when the log is open or not wanted
    pub log_error: Option<String>,
    /// base name for rolled-over log files ("<timestamp>_<prefix>.csv" per run)
    pub log_prefix: String,
    /// folder for log files and exported plots ("" = current folder)
    pub log_dir: PathBuf,
    /// Run information for the run about to start: the run
    /// setup of the PC method the Akta's method dump matched, and which method
    /// that was. Recorded when the run starts, then cleared, so a run without a
    /// matching dump gets none.
    pub run_info_for_new_log: BTreeMap<String, String>,
    /// Counts the runs started (`start_run`, `roll_log`); the GUI asks for the
    /// sample, user and notes of a new run when it changes.
    pub run_serial: u64,

    /// raw capture of every non-trend instrument message (`<log>_raw.txt`)
    pub raw: Option<File>,
    /// number of method dumps saved this session
    pub method_count: u32,
    /// folder of the PC method store
    pub method_dir: PathBuf,
    /// bumped when the method store changed on disk
    pub methods_changed: u64,
    /// the method the Akta last loaded from the PC, 0 = none
    pub last_loaded_method: u16,
    /// "Method start: …" event text waiting for the run to start
    pub pending_method_event: Option<String>,

    /// µL per instrument volume count: 1 for ÄKTAprime, 10 for ÄKTAprime plus
    pub volume_unit_ul: f64,
    /// delay volume (mL) between the UV cell and the fraction collector; 0 = none
    /// (see `end_fractionation`)
    pub fraction_delay_ml: f64,
    /// connection state for the GUI top/status bar
    pub status: String,
    /// true while a live source (link or demo) is feeding this state
    pub live: bool,
    /// true for the demo source
    pub demo: bool,

    /// bumped by every change; the GUI caches derived data by it
    pub generation: u64,
}

pub type SharedApp = Arc<Mutex<AppState>>;

impl Default for AppState {
    fn default() -> Self {
        AppState {
            trend: Vec::new(),
            events: Vec::new(),
            inst: InstrumentState::default(),
            fractions: Vec::new(),
            log_buf: String::new(),
            log: None,
            log_filename: PathBuf::new(),
            log_error: None,
            log_prefix: "akta".to_string(),
            log_dir: PathBuf::new(),
            run_info_for_new_log: BTreeMap::new(),
            run_serial: 0,
            raw: None,
            method_count: 0,
            method_dir: PathBuf::new(),
            methods_changed: 0,
            last_loaded_method: 0,
            pending_method_event: None,
            volume_unit_ul: crate::DEFAULT_VOLUME_UNIT_UL,
            fraction_delay_ml: 0.0,
            status: String::new(),
            live: false,
            demo: false,
            generation: 0,
        }
    }
}

/// Time (ms) at which the trend reached volume `vol` (mL), interpolated between
/// the points around it; None outside the trend.
fn time_at_volume(trend: &[TrendPoint], vol: f64) -> Option<f64> {
    let i = trend.partition_point(|p| p.cum_vol_ml < vol);
    let (a, b) = (trend.get(i.checked_sub(1)?)?, trend.get(i)?);
    if b.cum_vol_ml <= a.cum_vol_ml {
        return Some(b.cum_time_ms);
    }
    let f = (vol - a.cum_vol_ml) / (b.cum_vol_ml - a.cum_vol_ml);
    Some(a.cum_time_ms + f * (b.cum_time_ms - a.cum_time_ms))
}

impl AppState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn shared(self) -> SharedApp {
        Arc::new(Mutex::new(self))
    }

    /// Mark the state as changed: bumps `generation`, which the GUI compares
    /// to redraw and to rebuild cached data.
    pub fn touch(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    /// Convert an instrument volume count to mL.
    pub fn vol_ml(&self, raw: f64) -> f64 {
        raw * self.volume_unit_ul / 1000.0
    }

    /// Update fraction state from the current instrument state.
    /// Detects transitions: fraction_vol_ml going 0→>0 (start), >0→0 (stop), or a
    /// fraction_number change (new tube). Returns true if the fractions changed.
    pub fn update_fraction_state(&mut self) -> bool {
        let inst = &mut self.inst;
        let fracs = &mut self.fractions;
        let collecting = inst.fraction_vol_ml > 0.0;
        let mut changed = false;

        match (collecting, inst.current_fraction_idx) {
            (true, None) => {
                fracs.push(Fraction {
                    fraction_number: inst.fraction_number,
                    start_time_ms: inst.cum_time_ms,
                    start_vol_ml: inst.cum_vol_ml,
                    end_time_ms: None,
                    end_vol_ml: None,
                });
                inst.current_fraction_idx = Some(fracs.len() - 1);
                changed = true;
            }
            (true, Some(idx)) => {
                if let Some(current) = fracs.get(idx).copied()
                    && current.fraction_number != inst.fraction_number
                {
                    // new tube: close the current fraction and start the next
                    fracs[idx] = Fraction {
                        end_time_ms: Some(inst.cum_time_ms),
                        end_vol_ml: Some(inst.cum_vol_ml),
                        ..current
                    };
                    fracs.push(Fraction {
                        fraction_number: inst.fraction_number,
                        start_time_ms: inst.cum_time_ms,
                        start_vol_ml: inst.cum_vol_ml,
                        end_time_ms: None,
                        end_vol_ml: None,
                    });
                    inst.current_fraction_idx = Some(fracs.len() - 1);
                    changed = true;
                }
            }
            (false, Some(idx)) => {
                if let Some(current) = fracs.get(idx).copied() {
                    fracs[idx] = Fraction {
                        end_time_ms: Some(inst.cum_time_ms),
                        end_vol_ml: Some(inst.cum_vol_ml),
                        ..current
                    };
                }
                inst.current_fraction_idx = None;
                changed = true;
            }
            (false, None) => {}
        }
        if changed {
            self.touch();
        }
        changed
    }

    /// A tube feed with the instrument's tube number: tube `n` from now on. An
    /// ÄKTAprime sends tube 0 when fractionation ends (the collector leaves the
    /// last tube, just before "Fraction Off"): that is no new tube and changes
    /// nothing; "Fraction Off" or size 0 closes the fraction.
    pub fn feed_tube(&mut self, n: i64) {
        if n >= 1 {
            self.inst.fraction_number = n;
            self.update_fraction_state();
        }
        self.touch();
    }

    /// A tube feed without a tube number: from logs written before the number
    /// was recorded ("Tubefeed event (10 bytes)") or a short message. The tubes
    /// are counted: a feed before anything was collected in this run selects
    /// tube 1, every later one the next tube. Returns the tube number. (The
    /// feed sent when fractionation stops is no tube: see
    /// `csvlog::is_stop_feed`.)
    pub fn feed_next_tube(&mut self) -> i64 {
        // (tested with a real instrument log in csvlog)
        if !self.fractions.is_empty() {
            self.inst.fraction_number += 1;
        }
        self.update_fraction_state();
        self.touch();
        self.inst.fraction_number
    }

    /// The run ended and the pump stopped. An open fraction ends here (the
    /// method's last breakpoint sends no "Fraction Off"). The ÄKTAprime moves
    /// its rack `fraction_delay_ml` after each fraction boundary it reports, so
    /// a tube holds what passed the UV cell between two boundaries; but what
    /// passed it in the last `fraction_delay_ml` is still in the tubing to the
    /// collector when the pump stops and reaches no tube. The fractions are
    /// cut there: a tube that would only have started after it stays empty
    /// (no length).
    pub fn end_fractionation(&mut self) {
        if self.inst.current_fraction_idx.is_some() {
            self.inst.fraction_vol_ml = 0.0;
            self.update_fraction_state();
        }
        if self.fraction_delay_ml <= 0.0 {
            return;
        }
        let cut = self.inst.cum_vol_ml - self.fraction_delay_ml;
        let trend_time = time_at_volume(&self.trend, cut);
        for f in &mut self.fractions {
            let (Some(end_vol), Some(end_time)) = (f.end_vol_ml, f.end_time_ms) else {
                continue;
            };
            if end_vol <= cut {
                continue;
            }
            if f.start_vol_ml >= cut {
                f.end_vol_ml = Some(f.start_vol_ml);
                f.end_time_ms = Some(f.start_time_ms);
            } else {
                // without trend data: at the fraction's own (constant) flow
                let own = f.start_time_ms
                    + (cut - f.start_vol_ml) / (end_vol - f.start_vol_ml)
                        * (end_time - f.start_time_ms);
                f.end_vol_ml = Some(cut);
                f.end_time_ms = Some(trend_time.unwrap_or(own));
            }
        }
        self.touch();
    }

    /// Clear all per-run state so a fresh run starts from an empty display:
    /// empties trend, events and fractions and resets the instrument
    /// accumulators (keeping programme name, version, run state and valves).
    pub fn reset_run(&mut self) {
        self.trend.clear();
        self.events.clear();
        self.fractions.clear();
        let inst = &mut self.inst;
        inst.cum_time_ms = 0.0;
        inst.cum_vol_ml = 0.0;
        inst.run_time_ms = 0.0;
        inst.run_vol_ml = 0.0;
        inst.step_time_ms = 0.0;
        inst.step_vol_ml = 0.0;
        inst.fraction_number = 1;
        inst.fraction_vol_ml = 0.0;
        inst.current_fraction_idx = None;
        self.touch();
    }

    /// Record an event at the current run position, in memory and in the CSV log.
    pub fn push_event(&mut self, kind: EventKind, text: impl Into<String>) {
        let ev = AktaEvent {
            cum_time_ms: self.inst.cum_time_ms,
            cum_vol_ml: self.inst.cum_vol_ml,
            kind,
            text: text.into(),
        };
        log::info!("[{kind}] {}", ev.text);
        if let Some(log) = self.log.as_mut()
            && let Err(err) = log.write_event(&ev)
        {
            log::error!("Could not write to the CSV log: {err}");
        }
        self.events.push(ev);
        self.touch();
    }

    /// Record a trend point, in memory and in the CSV log.
    pub fn push_trend(&mut self, pt: TrendPoint) {
        if let Some(log) = self.log.as_mut()
            && let Err(err) = log.write_trend(&pt)
        {
            log::error!("Could not write to the CSV log: {err}");
        }
        self.trend.push(pt);
        self.touch();
    }
}
