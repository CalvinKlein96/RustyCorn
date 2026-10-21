//! OPC handlers: apply decoded messages to the AppState (`dispatch_rx_opc`
//! and the `handle_*` functions).
//!
//! Every function here runs with the AppState locked (by the link thread) and
//! does no slow work: the CSV/raw files are flushed line by line, nothing else.

use std::collections::BTreeMap;
use std::io::Write;

use rc_core::csvlog::log_side_file;
use rc_core::logtext;
use rc_core::opc::*;
use rc_core::{AppState, EventKind, InjectValue, TrendPoint};

use crate::frame::{le_uint, printable};

/// Run states of OPC 0x2003.
pub fn run_state_name(raw: u8) -> String {
    match raw {
        0x00 => "End".to_string(),
        0x01 => "Run".to_string(),
        0x02 => "Pause".to_string(),
        0x03 => "Hold".to_string(),
        0x04 => "Continue".to_string(),
        _ => format!("Unknown(0x{raw:x})"),
    }
}

/// Instrument text as a String. Texts that are not UTF-8 are read as Latin-1
/// so no byte is lost.
pub fn text_from_bytes(data: &[u8]) -> String {
    match std::str::from_utf8(data) {
        Ok(s) => s.to_string(),
        Err(_) => data.iter().map(|&b| b as char).collect(),
    }
}

pub fn handle_trend_data(data: &[u8], app: &mut AppState) {
    if data.len() < 14 {
        log::warn!("OPC 0x2001: short payload ({} bytes)", data.len());
        return;
    }
    let dt_ms = data[0] as f64 * 10.0; // Δt: units 10 ms
    let dv_ml = app.vol_ml(data[1] as f64); // Δv: volume counts → mL
    let inst = &mut app.inst;
    inst.cum_time_ms += dt_ms;
    inst.cum_vol_ml += dv_ml;
    // The ÄKTAprime sends OPC 0x2003 only on state changes, so run time/volume
    // would otherwise freeze at the run-start value. Advance them with the trend
    // deltas during a run; the next 0x2003 overwrites them with the instrument's values.
    if matches!(
        inst.run_state.as_str(),
        "Run" | "Pause" | "Hold" | "Continue"
    ) {
        inst.run_time_ms += dt_ms;
        inst.run_vol_ml += dv_ml;
    }

    let d = |i: usize| data[i] as i32;
    let mut uv_raw = d(2) | (d(3) << 8) | (d(4) << 16);
    if (data[4] & 0x80) != 0 {
        uv_raw |= -16_777_216; // sign-extend from bit 23
    }
    let cond_raw = d(5) | (d(6) << 8) | (d(7) << 16);
    let ph_raw = d(8) | ((d(9) & 0x07) << 8);
    let pctb_raw = (d(9) >> 3) | ((d(10) & 0x1f) << 5);
    let pres_raw = (d(10) >> 5) | (d(11) << 3);
    let temp_raw = (d(12) | ((d(13) & 0x07) << 8)) - 100;

    let pt = TrendPoint {
        cum_time_ms: inst.cum_time_ms,
        cum_vol_ml: inst.cum_vol_ml,
        uv: uv_raw as f64 / 1e6,
        cond_mscm: cond_raw as f64 / 1e3,
        ph: ph_raw as f64 / 100.0,
        pctb: pctb_raw as f64 / 10.0,
        pres_mpa: pres_raw as f64 / 1e3,
        temp_degc: temp_raw as f64 / 10.0,
    };
    log::debug!(
        "Trend  t={:8.1}s  v={:8.4}mL  UV={:+.6}  COND={:6.3}mS/cm  %B={:5.1}  P={:6.3}  T={:5.1}°C",
        pt.cum_time_ms / 1000.0,
        pt.cum_vol_ml,
        pt.uv,
        pt.cond_mscm,
        pt.pctb,
        pt.pres_mpa,
        pt.temp_degc
    );
    app.push_trend(pt);
}

pub fn handle_base_time_vol(data: &[u8], app: &mut AppState) {
    if data.len() < 8 {
        log::warn!("OPC 0x2002: short payload");
        return;
    }
    let base_time_ms = le_uint(data, 0, 4) as f64 * 10.0;
    let base_vol_ml = app.vol_ml(le_uint(data, 4, 4) as f64);
    // A base that jumps backwards means the instrument's run counter has
    // restarted — a new run. Roll to a fresh log so traces do not overlap.
    if base_time_ms < app.inst.cum_time_ms {
        app.roll_log();
    }
    app.inst.cum_time_ms = base_time_ms;
    app.inst.cum_vol_ml = base_vol_ml;
    app.touch();
    log::info!(
        "Base time/vol:  t={:.1}s  v={:.4}mL",
        base_time_ms / 1000.0,
        base_vol_ml
    );
}

fn inject_from_runstate(raw: u8) -> InjectValue {
    if raw == 0x00 {
        InjectValue::Load
    } else {
        InjectValue::Inject
    }
}

pub fn handle_run_state(data: &[u8], app: &mut AppState) {
    if data.len() < 10 {
        log::warn!("OPC 0x2003: short payload");
        return;
    }
    let run_time_raw = le_uint(data, 0, 3);
    let run_vol_raw = le_uint(data, 4, 3);
    let state_name = run_state_name(data[8]);
    let inject = inject_from_runstate(data[9]);
    let run_time_ms = run_time_raw as f64 / 10.0;
    let run_vol_ml = app.vol_ml(run_vol_raw as f64);

    let changed = app.inst.run_state != state_name || app.inst.inject_value != inject;

    // Roll to a fresh log on a new run before recording anything for it, so the
    // run-start event and its trend data land in the new file, which opens with
    // the run setup of the PC method the dump matched (if any).
    if logtext::is_run_start(&app.inst.run_state, &state_name) {
        app.start_run();
        // The method dump arrives just before the run starts; record it in the new log.
        if let Some(txt) = app.pending_method_event.take() {
            app.push_event(EventKind::Log, txt);
        }
    }

    let inst = &mut app.inst;
    inst.run_state = state_name;
    inst.run_time_ms = run_time_ms;
    inst.run_vol_ml = run_vol_ml;
    inst.inject_value = inject;
    app.touch();

    if changed {
        let txt = format!(
            "Run state: {}  inject={}",
            app.inst.run_state,
            inject.name()
        );
        app.push_event(EventKind::RunState, txt);
    }
}

pub fn handle_step_time_vol(data: &[u8], app: &mut AppState) {
    if data.len() < 8 {
        log::warn!("OPC 0x2006: short payload");
        return;
    }
    let step_time_ms = le_uint(data, 0, 4) as f64 * 10.0;
    let step_vol_ml = app.vol_ml(le_uint(data, 4, 4) as f64);
    app.inst.step_time_ms = step_time_ms;
    app.inst.step_vol_ml = step_vol_ml;
    app.touch();
    let txt = format!(
        "Step t={:.1}s  v={:.4}mL",
        step_time_ms / 1000.0,
        step_vol_ml
    );
    app.push_event(EventKind::Step, txt);
}

pub fn handle_debug_message(data: &[u8]) {
    let s = text_from_bytes(data).replace('~', "\r").replace('}', "\n");
    log::info!("Debug: {s:?}");
}

/// Inject valve, buffer valve and fraction state from a log text; all three
/// decoders run (non-short-circuit `|`), as one log text may change several.
fn apply_log_text(text: &str, app: &mut AppState) {
    let changed = logtext::set_inject_from_log(text, &mut app.inst)
        | logtext::set_buffer_valve_from_log(text, &mut app.inst)
        | logtext::set_fraction_from_log(text, app);
    if changed {
        app.touch();
    }
    app.push_event(EventKind::Log, text);
}

/// Log messages: single (0x8004) or split over 0x8005, 0x8006…, 0x8007.
pub fn handle_log_message(opc: u16, data: &[u8], app: &mut AppState) {
    let text = text_from_bytes(data);
    match opc {
        OPC_LOG_SINGLE => apply_log_text(&text, app),
        OPC_LOG_BEGIN => app.log_buf = text,
        OPC_LOG_CONT => app.log_buf.push_str(&text),
        OPC_LOG_END => {
            let full = std::mem::take(&mut app.log_buf) + &text;
            apply_log_text(&full, app);
        }
        _ => {}
    }
}

pub fn handle_tubefeed(data: &[u8], app: &mut AppState) {
    if data.len() >= 10 {
        let tube_no = data[8] as i64 | ((data[9] as i64) << 8);
        app.inst.fraction_number = tube_no.max(1);
    } else {
        app.inst.fraction_number += 1;
    }
    // Update fraction state machine
    app.update_fraction_state();
    app.touch();
    app.push_event(
        EventKind::Tubefeed,
        format!("Tubefeed event ({} bytes)", data.len()),
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Raw capture — material for decoding undocumented messages
// ─────────────────────────────────────────────────────────────────────────────
// Every received OPC except trend data and the watchdog is appended to the raw
// log, as are the method messages sent to the Akta (marked TX), and each method
// dump is saved as the instrument sent it, still escaped, to <log>_method_N.met.

/// One raw capture line without its time stamp:
/// `  0x8004  n=15  466c…  |Flow 0.5 ml/min|` (TX lines: `  TX 0x2016 …`).
pub fn capture_line(opc: u16, data: &[u8], tx: bool) -> String {
    let hex: String = data.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}{opc:04x}  n={}  {hex}  |{}|",
        if tx { "  TX 0x" } else { "  0x" },
        data.len(),
        printable(data)
    )
}

/// Append the message to the raw capture, if one is open (trend data and
/// watchdog messages are left out).
pub fn capture_opc(app: &mut AppState, opc: u16, data: &[u8], tx: bool) {
    if opc == OPC_TREND_DATA || opc == OPC_WATCHDOG {
        return;
    }
    let Some(io) = app.raw.as_mut() else { return };
    let stamp = chrono::Local::now().format("%H:%M:%S%.3f");
    let line = capture_line(opc, data, tx);
    if let Err(err) = writeln!(io, "{stamp}{line}").and_then(|_| io.flush()) {
        log::warn!("Could not write the raw capture: {err}");
    }
}

/// A method dump has arrived (escaped text, see `rc_methods::store`). Reads the
/// methods folder to find which PC method it is; the link thread uses
/// [`method_dump_received_with`] to do that without holding the app lock.
pub fn method_dump_received(app: &mut AppState, raw: &[u8]) {
    let setup = dump_run_setup(&app.method_dir, app.last_loaded_method, raw);
    method_dump_received_with(app, raw, setup);
}

/// The run information a run of this dump gets: the run setup of the PC method
/// with the same content, and which method that is; empty if none matches.
/// Reads the methods folder only.
pub fn dump_run_setup(
    method_dir: &std::path::Path,
    last_loaded: u16,
    raw: &[u8],
) -> BTreeMap<String, String> {
    let m = rc_methods::parse_method_dump(raw);
    rc_methods::run_setup_for_dump(method_dir, &m, last_loaded)
}

/// [`method_dump_received`] with the run setup already looked up.
pub fn method_dump_received_with(app: &mut AppState, raw: &[u8], setup: BTreeMap<String, String>) {
    if app.raw.is_some() {
        // saved only with "Save raw instrument data" on
        app.method_count += 1;
        let path = log_side_file(
            &app.log_filename,
            &format!("_method_{}.met", app.method_count),
        );
        match std::fs::write(&path, raw) {
            Ok(()) => log::info!(
                "Saved method transfer ({} bytes) to {}",
                raw.len(),
                path.display()
            ),
            Err(err) => log::warn!(
                "Could not save the method transfer to {}: {err}",
                path.display()
            ),
        }
    }
    let m = rc_methods::parse_method_dump(raw);
    rc_methods::apply_method_dump(app, &m);
    // replaces what an earlier dump left: only this run's method counts
    app.run_info_for_new_log = setup;
    app.touch();
}

// ─────────────────────────────────────────────────────────────────────────────
// dispatch_rx_opc
// ─────────────────────────────────────────────────────────────────────────────

/// Apply one OPC message from the Akta to the app state. Method store
/// requests are only captured here; the link answers them (`link::method_opc`).
pub fn dispatch_rx_opc(app: &mut AppState, opc: u16, data: &[u8]) {
    capture_opc(app, opc, data, false);
    match opc {
        OPC_PROG_NAME => {
            let name = text_from_bytes(&data[..data.len().min(100)])
                .trim_end_matches('\0')
                .to_string();
            log::info!("Programme name: {name}");
            app.inst.prog_name = name;
            app.touch();
        }
        OPC_VERSION_STRING => {
            let ver = text_from_bytes(&data[..data.len().min(20)])
                .trim_end_matches('\0')
                .to_string();
            log::info!("Version: {ver}");
            app.inst.version = ver;
            app.touch();
        }
        OPC_TREND_DATA => handle_trend_data(data, app),
        OPC_BASE_TIME_VOL => handle_base_time_vol(data, app),
        OPC_RUN_STATE => handle_run_state(data, app),
        OPC_INJECT => app.push_event(
            EventKind::Inject,
            format!("Inject event ({} bytes)", data.len()),
        ),
        OPC_TUBEFEED => handle_tubefeed(data, app),
        OPC_STEP_TIME_VOL => handle_step_time_vol(data, app),
        OPC_DEBUG_MSG => handle_debug_message(data),
        OPC_LOG_SINGLE | OPC_LOG_BEGIN | OPC_LOG_CONT | OPC_LOG_END => {
            handle_log_message(opc, data, app)
        }
        OPC_WATCHDOG => {} // silently drop
        _ if rc_methods::is_method_opc(opc) => {
            // answered on the link thread (link::method_opc)
            log::debug!("Method OPC 0x{opc:04x}: {} bytes", data.len());
        }
        _ => log::info!("Unknown OPC 0x{opc:04x}: {} bytes", data.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rc_core::{AppState, EventKind, InjectValue};

    /// A method dump as an ÄKTAprime (V2.01) sends it before a run:
    /// manual method, 0.5 ml/min, 0 %B, buffer valve port 8, injection valve Load.
    pub(crate) const METHOD_DUMP: &str = concat!(
        "{l{5}{2 Method Dump}{2}{2 AKTAprime|3Ver|2, V2.01|?}{2}{2 Dump Format Ver|2, V1.00|?}",
        "{2}{l{5}}}{B}{2 Method Settings}{B}Method Type|8,Manaul Method}Method|=,1}Method base|8,ml}",
        "Frac base|:,ml}Max pressure (kPa) ,1000}Alarm (min/ml)|5,0.0}}{E}{2 Method Breakpoints}{E}|E}",
        "Brkp Volume|3Conc Flow|2Frac|3Buff Inj|4Peak|3Autozzero Eventmark}|Z(mAu/min)}}",
        ",1|3,0.0|5,0|3,0.5|2,0.0|3,8|3,Load|2,0|5,N|7,N},2|3,99999.9 ,0|3,0.5|2,0.0|3,8|3,Load|2,0|5,N|7,N}"
    );

    fn app(unit: f64) -> AppState {
        AppState {
            volume_unit_ul: unit,
            ..AppState::default()
        }
    }

    fn texts(app: &AppState) -> Vec<(EventKind, String)> {
        app.events
            .iter()
            .map(|e| (e.kind, e.text.clone()))
            .collect()
    }

    const TRENDS: [[u8; 14]; 4] = [
        [
            10, 7, 0x40, 0x42, 0x0f, 0x10, 0x27, 0x00, 0xbc, 0x02, 0x50, 0xa0, 0x5e, 0x01,
        ],
        [
            0xff, 0xff, 0x00, 0x00, 0x80, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        ],
        [
            1, 0, 0x18, 0xfc, 0xff, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
        [
            50, 25, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x12, 0x34, 0x56, 0x78,
        ],
    ];

    /// Known-good values for decoded trend data (bit-exact floats).
    #[test]
    fn trend_data_matches_reference() {
        let reference: [(f64, [[f64; 8]; 4]); 2] = [
            (
                10.0,
                [
                    [100.0, 0.07, 1.0, 10.0, 7.0, 51.2, 1.282, 25.0],
                    [
                        2650.0,
                        2.6199999999999997,
                        -8.388608,
                        16777.215,
                        20.47,
                        102.3,
                        2.047,
                        194.7,
                    ],
                    [
                        2660.0,
                        2.6199999999999997,
                        -0.001,
                        0.001,
                        0.0,
                        0.0,
                        0.0,
                        -10.0,
                    ],
                    [
                        3160.0,
                        2.8699999999999997,
                        5.649426,
                        12360.312,
                        2.22,
                        60.6,
                        0.416,
                        -1.4,
                    ],
                ],
            ),
            (
                1.0,
                [
                    [100.0, 0.007, 1.0, 10.0, 7.0, 51.2, 1.282, 25.0],
                    [
                        2650.0, 0.262, -8.388608, 16777.215, 20.47, 102.3, 2.047, 194.7,
                    ],
                    [2660.0, 0.262, -0.001, 0.001, 0.0, 0.0, 0.0, -10.0],
                    [
                        3160.0,
                        0.28700000000000003,
                        5.649426,
                        12360.312,
                        2.22,
                        60.6,
                        0.416,
                        -1.4,
                    ],
                ],
            ),
        ];
        for (unit, want) in reference {
            let mut a = app(unit);
            for d in TRENDS {
                dispatch_rx_opc(&mut a, OPC_TREND_DATA, &d);
            }
            assert_eq!(a.trend.len(), 4);
            for (p, w) in a.trend.iter().zip(want) {
                let got = [
                    p.cum_time_ms,
                    p.cum_vol_ml,
                    p.uv,
                    p.cond_mscm,
                    p.ph,
                    p.pctb,
                    p.pres_mpa,
                    p.temp_degc,
                ];
                assert_eq!(got, w, "unit {unit}");
            }
        }
        // short payloads are ignored
        let mut a = app(10.0);
        handle_trend_data(&[1, 2, 3], &mut a);
        assert!(a.trend.is_empty() && a.generation == 0);
    }

    /// The volume unit scales the volume counter.
    #[test]
    fn volume_unit() {
        assert!((app(1.0).vol_ml(1000.0) - 1.0).abs() < 1e-12);
        assert!((app(10.0).vol_ml(1000.0) - 10.0).abs() < 1e-12);
    }

    /// Run time and volume advance with trend data, only while running.
    #[test]
    fn run_time_and_volume_advance_with_trend_data() {
        let mut a = app(1.0);
        let mut data = [0u8; 14];
        data[0] = 10; // Δt = 100 ms
        data[1] = 1; // Δv = 1 µL
        handle_trend_data(&data, &mut a); // not running: no change
        assert_eq!(a.inst.run_time_ms, 0.0);
        a.inst.run_state = "Run".to_string();
        for _ in 0..600 {
            handle_trend_data(&data, &mut a);
        }
        assert!((a.inst.run_time_ms - 60_000.0).abs() < 1e-6);
        assert!((a.inst.run_vol_ml - 0.6).abs() < 1e-9);
    }

    /// Known-good state after base, run state, step, programme name and version messages.
    #[test]
    fn state_messages_match_reference() {
        let mut a = app(10.0);
        dispatch_rx_opc(
            &mut a,
            OPC_BASE_TIME_VOL,
            &[0x10, 0x27, 0, 0, 0xe8, 0x03, 0, 0],
        );
        assert_eq!((a.inst.cum_time_ms, a.inst.cum_vol_ml), (100_000.0, 10.0));
        dispatch_rx_opc(
            &mut a,
            OPC_RUN_STATE,
            &[0x9a, 0x02, 0x31, 0x00, 0xc4, 0x09, 0x00, 0x00, 0x01, 0x01],
        );
        assert_eq!(a.inst.run_state, "Run");
        assert_eq!((a.inst.run_time_ms, a.inst.run_vol_ml), (321_193.0, 25.0));
        assert_eq!(a.inst.inject_value, InjectValue::Inject);
        // the same state again records no event
        dispatch_rx_opc(
            &mut a,
            OPC_RUN_STATE,
            &[0x9a, 0x02, 0x31, 0x00, 0xc4, 0x09, 0x00, 0x00, 0x01, 0x01],
        );
        dispatch_rx_opc(&mut a, OPC_RUN_STATE, &[0, 0, 0, 0, 0, 0, 0, 0, 0x07, 0x00]);
        dispatch_rx_opc(
            &mut a,
            OPC_STEP_TIME_VOL,
            &[0x39, 0x30, 0, 0, 0x15, 0xcd, 0x5b, 0x07],
        );
        assert_eq!(
            (a.inst.step_time_ms, a.inst.step_vol_ml),
            (123_450.0, 1.23456789e6)
        );
        dispatch_rx_opc(&mut a, OPC_PROG_NAME, b"Manual run\0\0");
        dispatch_rx_opc(
            &mut a,
            OPC_VERSION_STRING,
            "ÄKTAprime V2.01 build 743\0".as_bytes(),
        );
        assert_eq!(a.inst.prog_name, "Manual run");
        assert_eq!(a.inst.version, "ÄKTAprime V2.01 bui"); // first 20 bytes
        assert_eq!(
            texts(&a),
            vec![
                (
                    EventKind::RunState,
                    "Run state: Run  inject=IV_INJECT".to_string()
                ),
                (
                    EventKind::RunState,
                    "Run state: Unknown(0x7)  inject=IV_LOAD".to_string()
                ),
                (
                    EventKind::Step,
                    "Step t=123.5s  v=1234567.8900mL".to_string()
                ),
            ]
        );
        assert!(
            a.events
                .iter()
                .all(|e| e.cum_time_ms == 100_000.0 && e.cum_vol_ml == 10.0)
        );
        // short payloads are ignored
        let g = a.generation;
        handle_base_time_vol(&[1, 2, 3], &mut a);
        handle_run_state(&[1, 2, 3], &mut a);
        handle_step_time_vol(&[1], &mut a);
        assert_eq!(a.generation, g);
    }

    fn csv_rows(path: &std::path::Path) -> Vec<String> {
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// A run start (End/--- → Run) rolls to a new log that opens with the run
    /// information and the pending method event, before the run-state event.
    #[test]
    fn run_start_rolls_log_with_pending_method_event() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = app(10.0);
        a.log_dir = dir.path().to_path_buf();
        a.log_prefix = "test".to_string();
        a.log_filename = dir.path().join("first.csv");
        a.log = Some(rc_core::csvlog::CsvLog::create(&a.log_filename).unwrap());
        a.run_info_for_new_log = [("Column".to_string(), "Superdex".to_string())].into();

        // no trend data yet: the first run start keeps the current log
        dispatch_rx_opc(&mut a, OPC_RUN_STATE, &[0, 0, 0, 0, 0, 0, 0, 0, 0x00, 0x00]);
        assert_eq!(a.inst.run_state, "End");
        dispatch_rx_opc(&mut a, OPC_TREND_DATA, &TRENDS[0]);
        a.pending_method_event = Some("Method start: flow 0.5 ml/min".to_string());

        dispatch_rx_opc(&mut a, OPC_RUN_STATE, &[0, 0, 0, 0, 0, 0, 0, 0, 0x01, 0x00]);
        assert_ne!(a.log_filename, dir.path().join("first.csv"));
        assert!(
            a.log_filename
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("test_")
        );
        assert!(a.trend.is_empty() && a.pending_method_event.is_none());
        assert_eq!(
            texts(&a),
            vec![
                (EventKind::Info, "Column: Superdex".to_string()),
                (EventKind::Log, "Method start: flow 0.5 ml/min".to_string()),
                (
                    EventKind::RunState,
                    "Run state: Run  inject=IV_LOAD".to_string()
                ),
            ]
        );
        let rows = csv_rows(&a.log_filename);
        assert!(
            rows[2].ends_with("EVT_INFO,,,,,,,\"Column: Superdex\""),
            "{rows:?}"
        );
        assert!(
            rows[3].contains("EVT_LOG") && rows[4].contains("EVT_RUN_STATE"),
            "{rows:?}"
        );
        let first = csv_rows(&dir.path().join("first.csv"));
        assert!(first.iter().any(|r| r.contains(",TREND,")));

        // Pause and resume stay in the same run (and log)
        dispatch_rx_opc(&mut a, OPC_TREND_DATA, &TRENDS[0]);
        let log = a.log_filename.clone();
        dispatch_rx_opc(&mut a, OPC_RUN_STATE, &[0, 0, 0, 0, 0, 0, 0, 0, 0x02, 0x00]);
        dispatch_rx_opc(&mut a, OPC_RUN_STATE, &[0, 0, 0, 0, 0, 0, 0, 0, 0x01, 0x00]);
        assert_eq!(a.log_filename, log);
        assert_eq!(a.trend.len(), 1);

        // a base time that jumps backwards is a new run as well
        assert_eq!(a.inst.cum_time_ms, 100.0);
        dispatch_rx_opc(&mut a, OPC_BASE_TIME_VOL, &[0x0a, 0, 0, 0, 0, 0, 0, 0]); // 100 ms: not back
        assert_eq!(a.log_filename, log);
        dispatch_rx_opc(&mut a, OPC_BASE_TIME_VOL, &[0x05, 0, 0, 0, 0, 0, 0, 0]);
        assert_ne!(a.log_filename, log);
        assert!(a.trend.is_empty());
        assert_eq!(a.inst.cum_time_ms, 50.0);
    }

    #[test]
    fn log_messages_set_valves_and_fractions() {
        let mut a = app(10.0);
        // split over three messages
        dispatch_rx_opc(&mut a, OPC_LOG_BEGIN, b"Buffer Valve ");
        dispatch_rx_opc(&mut a, OPC_LOG_CONT, b"Pos");
        dispatch_rx_opc(&mut a, OPC_LOG_END, b" 3");
        assert!(a.log_buf.is_empty());
        assert_eq!((a.inst.buffer_port, a.inst.buffer_port_known), (3, true));
        dispatch_rx_opc(&mut a, OPC_LOG_SINGLE, b"Injection Valve Inj");
        assert_eq!(a.inst.inject_value, InjectValue::Inject);
        dispatch_rx_opc(&mut a, OPC_LOG_SINGLE, b"Flow 0.5 ml/min");

        // fractions: size > 0 starts, a new tube closes and opens, size 0 stops
        dispatch_rx_opc(
            &mut a,
            OPC_BASE_TIME_VOL,
            &[0x10, 0x27, 0, 0, 0xe8, 0x03, 0, 0],
        );
        dispatch_rx_opc(&mut a, OPC_LOG_SINGLE, b"Fraction size 1.0 ml");
        assert_eq!(a.fractions.len(), 1);
        dispatch_rx_opc(&mut a, OPC_TREND_DATA, &TRENDS[0]);
        let mut tube = [0u8; 10];
        tube[8] = 2;
        dispatch_rx_opc(&mut a, OPC_TUBEFEED, &tube);
        assert_eq!(a.fractions.len(), 2);
        assert_eq!(a.fractions[0].end_time_ms, Some(100_100.0));
        assert_eq!(a.fractions[1].fraction_number, 2);
        dispatch_rx_opc(&mut a, OPC_TUBEFEED, &[0u8; 4]); // short: next tube
        assert_eq!(a.inst.fraction_number, 3);
        assert_eq!(a.fractions.len(), 3);
        dispatch_rx_opc(&mut a, OPC_LOG_SINGLE, b"Fraction size 0.0 ml");
        assert!(a.inst.current_fraction_idx.is_none());
        assert!(a.fractions.iter().all(|f| f.end_time_ms.is_some()));
        dispatch_rx_opc(&mut a, OPC_INJECT, &[0u8; 8]);

        assert_eq!(
            texts(&a),
            vec![
                (EventKind::Log, "Buffer Valve Pos 3".to_string()),
                (EventKind::Log, "Injection Valve Inj".to_string()),
                (EventKind::Log, "Flow 0.5 ml/min".to_string()),
                (EventKind::Log, "Fraction size 1.0 ml".to_string()),
                (EventKind::Tubefeed, "Tubefeed event (10 bytes)".to_string()),
                (EventKind::Tubefeed, "Tubefeed event (4 bytes)".to_string()),
                (EventKind::Log, "Fraction size 0.0 ml".to_string()),
                (EventKind::Inject, "Inject event (8 bytes)".to_string()),
            ]
        );
        // debug, watchdog, method and unknown OPCs record nothing
        let n = a.events.len();
        for opc in [OPC_DEBUG_MSG, OPC_WATCHDOG, OPC_METHOD_LIST, 0x2fff] {
            dispatch_rx_opc(&mut a, opc, b"x~}");
        }
        assert_eq!(a.events.len(), n);
    }

    #[test]
    fn texts_that_are_not_utf8_are_kept() {
        assert_eq!(text_from_bytes(b"Flow"), "Flow");
        assert_eq!(text_from_bytes(&[0x35, 0xb5, 0x6c]), "5µl"); // Latin-1
    }

    #[test]
    fn raw_capture_lines_match_reference() {
        // A capture line: "17:09:57.664  0x8004  n=15  466c6f7720302e35206d6c2f6d696e  |Flow 0.5 ml/min|"
        assert_eq!(
            capture_line(OPC_LOG_SINGLE, b"Flow 0.5 ml/min", false),
            "  0x8004  n=15  466c6f7720302e35206d6c2f6d696e  |Flow 0.5 ml/min|"
        );
        assert_eq!(
            capture_line(OPC_METHOD_LIST_REPLY, &[3, 0, 0xff, 0xff], true),
            "  TX 0x2016  n=4  0300ffff  |....|"
        );

        let dir = tempfile::tempdir().unwrap();
        let raw = dir.path().join("run_raw.txt");
        let mut a = app(10.0);
        a.raw = Some(std::fs::File::create(&raw).unwrap());
        dispatch_rx_opc(&mut a, OPC_LOG_SINGLE, b"Flow 0.5 ml/min");
        dispatch_rx_opc(&mut a, OPC_TREND_DATA, &TRENDS[0]); // not captured
        dispatch_rx_opc(&mut a, OPC_WATCHDOG, &[1, 2]); // not captured
        capture_opc(&mut a, OPC_METHOD_LIST_REPLY, &[3, 0, 0xff, 0xff], true);
        let lines = csv_rows(&raw);
        assert_eq!(lines.len(), 2);
        let re = regex::Regex::new(r"^\d\d:\d\d:\d\d\.\d\d\d  ").unwrap();
        assert!(lines.iter().all(|l| re.is_match(l)), "{lines:?}");
        assert!(
            lines[0].ends_with("  0x8004  n=15  466c6f7720302e35206d6c2f6d696e  |Flow 0.5 ml/min|")
        );
        assert!(lines[1][12..].starts_with("  TX 0x2016"));
    }

    /// A method dump through the protocol: saved as .met with raw capture
    /// on, decoded, and the run's starting state taken from the first breakpoint.
    #[test]
    fn method_dump_is_saved_and_applied() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = app(10.0);
        a.log_filename = dir.path().join("akta_1.csv");
        method_dump_received(&mut a, METHOD_DUMP.as_bytes());
        assert!(!dir.path().join("akta_1_method_1.met").exists()); // raw capture off
        assert_eq!((a.inst.buffer_port, a.inst.buffer_port_known), (8, true));
        assert_eq!(a.inst.inject_value, InjectValue::Load);
        assert!(
            a.pending_method_event
                .as_deref()
                .is_some_and(|t| t.contains("buffer port 8"))
        );

        a.raw = Some(std::fs::File::create(dir.path().join("akta_1_raw.txt")).unwrap());
        method_dump_received(&mut a, METHOD_DUMP.as_bytes());
        method_dump_received(&mut a, METHOD_DUMP.as_bytes());
        assert_eq!(
            std::fs::read(dir.path().join("akta_1_method_1.met")).unwrap(),
            METHOD_DUMP.as_bytes()
        );
        assert!(dir.path().join("akta_1_method_2.met").exists());
        assert_eq!(a.method_count, 2);
    }

    /// A started log with some trend data, its method folder holding the dump's
    /// method as PC method 4 with a run setup.
    fn app_with_pc_method(dir: &std::path::Path) -> AppState {
        let mut a = app(10.0);
        a.log_dir = dir.to_path_buf();
        a.log_prefix = "run".to_string();
        a.log_filename = dir.join("first.csv");
        a.log = Some(rc_core::csvlog::CsvLog::create(&a.log_filename).unwrap());
        a.method_dir = dir.join("methods");
        let text = rc_methods::unescape_method_text(METHOD_DUMP.as_bytes());
        rc_methods::save_method_file(&a.method_dir, 4, &text).unwrap();
        rc_methods::set_method_name(&a.method_dir, 4, "Manual 0.5").unwrap();
        let setup = [
            ("Column", "Superdex 75"),
            ("Column volume (mL)", "24"),
            ("Port 8", "Sample"),
        ]
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .into();
        rc_methods::set_method_setup(&a.method_dir, 4, &setup).unwrap();
        dispatch_rx_opc(&mut a, OPC_RUN_STATE, &[0, 0, 0, 0, 0, 0, 0, 0, 0x00, 0x00]);
        dispatch_rx_opc(&mut a, OPC_TREND_DATA, &TRENDS[0]);
        a
    }

    const RUN: [u8; 10] = [0, 0, 0, 0, 0, 0, 0, 0, 0x01, 0x00];
    const END: [u8; 10] = [0, 0, 0, 0, 0, 0, 0, 0, 0x00, 0x00];

    /// The dump before a run is PC method 4: the run's log opens with that
    /// method's run setup and which method it was.
    #[test]
    fn run_start_records_the_matching_method_setup() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = app_with_pc_method(dir.path());
        let serial = a.run_serial;
        method_dump_received(&mut a, METHOD_DUMP.as_bytes());
        assert_eq!(a.run_info_for_new_log["PC method"], "004 – Manual 0.5");
        dispatch_rx_opc(&mut a, OPC_RUN_STATE, &RUN);
        assert_eq!(a.run_serial, serial + 1);
        assert!(a.run_info_for_new_log.is_empty());
        let t = texts(&a);
        assert_eq!(
            t[..4],
            [
                (EventKind::Info, "PC method: 004 – Manual 0.5".to_string()),
                (EventKind::Info, "Column: Superdex 75".to_string()),
                (EventKind::Info, "Column volume (mL): 24".to_string()),
                (EventKind::Info, "Port 8: Sample".to_string()),
            ]
        );
        assert!(t[4].1.starts_with("Method start:"), "{t:?}");
        let info = rc_core::runinfo::run_info(&a.events);
        assert_eq!(rc_core::runinfo::column_volume(&info), 24.0);
        assert!(!info.contains_key("Sample") && !info.contains_key("User"));
        // the file says the same
        let back = rc_core::csvlog::load_csv_log(&a.log_filename).unwrap();
        assert_eq!(rc_core::runinfo::run_info(&back.events), info);
    }

    /// A run whose dump matches no PC method, and a run without any dump, get
    /// no method setup, never the one of an earlier run or of the last loaded method.
    #[test]
    fn run_start_without_matching_dump_has_no_setup() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = app_with_pc_method(dir.path());
        a.last_loaded_method = 4;
        // a dump of another method (different flow)
        let other = METHOD_DUMP.replacen(",0.5|2,0.0|3,8", ",0.7|2,0.0|3,8", 1);
        assert_ne!(other, METHOD_DUMP);
        method_dump_received(&mut a, other.as_bytes());
        assert!(a.run_info_for_new_log.is_empty());
        dispatch_rx_opc(&mut a, OPC_RUN_STATE, &RUN);
        assert!(
            a.events.iter().all(|e| e.kind != EventKind::Info),
            "{:?}",
            texts(&a)
        );

        // a matching dump for a run that then starts …
        method_dump_received(&mut a, METHOD_DUMP.as_bytes());
        dispatch_rx_opc(&mut a, OPC_TREND_DATA, &TRENDS[0]);
        dispatch_rx_opc(&mut a, OPC_RUN_STATE, &END);
        dispatch_rx_opc(&mut a, OPC_RUN_STATE, &RUN);
        assert!(texts(&a).contains(&(EventKind::Info, "Column: Superdex 75".to_string())));
        // … and the next run without a dump: nothing
        dispatch_rx_opc(&mut a, OPC_TREND_DATA, &TRENDS[0]);
        dispatch_rx_opc(&mut a, OPC_RUN_STATE, &END);
        dispatch_rx_opc(&mut a, OPC_RUN_STATE, &RUN);
        assert!(
            a.events.iter().all(|e| e.kind != EventKind::Info),
            "{:?}",
            texts(&a)
        );
        let back = rc_core::csvlog::load_csv_log(&a.log_filename).unwrap();
        assert!(rc_core::runinfo::run_info(&back.events).is_empty());
    }
}
