//! Decoders for instrument log-message texts, shared by the live link and by
//! replaying a log (the instrument state at a moment of a saved run).

use std::sync::LazyLock;

use regex::Regex;

use crate::types::{AppState, InjectValue, InstrumentState};

static FLOW_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^Flow\s+([0-9]+(?:\.[0-9]+)?)\s*ml/min$").expect("valid regex")
});
// "Buffer Valve Pos 3"; a change made on the front panel is prefixed "Manual Set; ".
static BUFFER_VALVE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:Manual Set;\s*)?Buffer Valve Pos\s+([1-8])$").expect("valid regex")
});
static FRACTION_SIZE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^Fraction size\s+([0-9]+(?:\.[0-9]+)?)\s+ml$").expect("valid regex")
});
// The instrument's own word for fraction size 0 (a "Fraction Off" method step).
static FRACTION_OFF_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^Fraction\s+Off$").expect("valid regex"));
static FEED_TUBE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^Feed Tube Nr\s+(\d+)$").expect("valid regex"));

/// Whitespace trimmed from log texts: ASCII whitespace, U+0085 and the Unicode
/// category Zs, but not the line/paragraph separators U+2028/U+2029 (which
/// Rust's `is_whitespace` also counts), so such a character stays part of a value.
pub fn is_trim_space(c: char) -> bool {
    c.is_whitespace() && c != '\u{2028}' && c != '\u{2029}'
}

/// `s` without leading and trailing whitespace (see [`is_trim_space`]).
pub fn trim_space(s: &str) -> &str {
    s.trim_matches(is_trim_space)
}

/// A decimal number as found in logs: surrounding whitespace is allowed,
/// "Inf"/"NaN" are accepted, anything else (a decimal comma, digit separators,
/// an empty text) is None.
pub fn parse_f64(s: &str) -> Option<f64> {
    let s = trim_space(s);
    if s.is_empty() {
        return None;
    }
    s.parse::<f64>().ok()
}

/// Set flow, as reported by "Flow 1.0 ml/min" log messages.
pub fn flow_from_log(text: &str) -> Option<f64> {
    let caps = FLOW_RE.captures(trim_space(text))?;
    caps[1].parse().ok()
}

/// Fraction size in mL from a "Fraction size 1.0 ml" message (0 = to waste);
/// "Fraction Off" is size 0.
pub fn fraction_size_from_log(text: &str) -> Option<f64> {
    let text = trim_space(text);
    if FRACTION_OFF_RE.is_match(text) {
        return Some(0.0);
    }
    let caps = FRACTION_SIZE_RE.captures(text)?;
    caps[1].parse().ok()
}

/// Tube number from a "Feed Tube Nr 7" message, at least 1.
pub fn feed_tube_from_log(text: &str) -> Option<i64> {
    let caps = FEED_TUBE_RE.captures(trim_space(text))?;
    caps[1].parse::<i64>().ok().map(|n| n.max(1))
}

/// Buffer valve port 1–8 from "Buffer Valve Pos 3" / "Manual Set; Buffer Valve Pos 3".
pub fn buffer_valve_from_log(text: &str) -> Option<i64> {
    let caps = BUFFER_VALVE_RE.captures(trim_space(text))?;
    caps[1].parse().ok()
}

/// Injection valve position from "Injection Valve Load/Inj/Waste".
pub fn inject_from_log(text: &str) -> Option<InjectValue> {
    match trim_space(text).to_lowercase().as_str() {
        "injection valve load" => Some(InjectValue::Load),
        "injection valve inj" => Some(InjectValue::Inject),
        "injection valve waste" => Some(InjectValue::Waste),
        _ => None,
    }
}

/// Apply an injection valve message to the state; true if the text set the valve.
pub fn set_inject_from_log(text: &str, inst: &mut InstrumentState) -> bool {
    match inject_from_log(text) {
        Some(v) => {
            inst.inject_value = v;
            true
        }
        None => false,
    }
}

/// Apply a buffer valve message to the state (the port is then known, not
/// assumed); true if the text set the valve.
pub fn set_buffer_valve_from_log(text: &str, inst: &mut InstrumentState) -> bool {
    match buffer_valve_from_log(text) {
        Some(port) => {
            inst.buffer_port = port;
            inst.buffer_port_known = true;
            true
        }
        None => false,
    }
}

/// Apply a fraction size or feed tube message, updating the fraction state
/// machine through the app; true if the text changed anything.
pub fn set_fraction_from_log(text: &str, app: &mut AppState) -> bool {
    let mut changed = false;
    if let Some(size) = fraction_size_from_log(text) {
        app.inst.fraction_vol_ml = size;
        changed = true;
    }
    if let Some(n) = feed_tube_from_log(text) {
        app.inst.fraction_number = n;
        changed = true;
    }
    // Update the fraction state machine if anything changed
    if changed {
        app.update_fraction_state();
        app.touch();
    }
    changed
}

/// Whether a run state change `prev` → `now` starts a run: a genuine new run enters "Run" from
/// an ended or idle state (not a resume from Pause/Hold/Continue, which stay
/// within the same run).
pub fn is_run_start(prev: &str, now: &str) -> bool {
    now == "Run" && (prev == "End" || prev == "---")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::EventKind;

    #[test]
    fn valve_log_messages() {
        let mut s = InstrumentState::default();
        assert!(!s.buffer_port_known && s.buffer_port == 1); // assumed until reported
        assert!(set_buffer_valve_from_log("Buffer Valve Pos 8", &mut s) && s.buffer_port == 8);
        assert!(s.buffer_port_known);
        assert!(
            set_buffer_valve_from_log("Manual Set; Buffer Valve Pos 3", &mut s)
                && s.buffer_port == 3
        );
        assert!(!set_buffer_valve_from_log("Buffer Valve Pos 9", &mut s) && s.buffer_port == 3);
        assert!(
            set_inject_from_log("Injection Valve Inj", &mut s)
                && s.inject_value == InjectValue::Inject
        );
        assert_eq!(flow_from_log("Flow 0.5 ml/min"), Some(0.5));
        assert_eq!(flow_from_log("Concentration 10 %B"), None);
    }

    #[test]
    fn flow_variants() {
        assert_eq!(flow_from_log("  flow 12 ML/MIN \r\n"), Some(12.0));
        assert_eq!(flow_from_log("Flow 1.25ml/min"), Some(1.25));
        assert_eq!(flow_from_log("Flow 1. ml/min"), None);
        assert_eq!(flow_from_log("Flow .5 ml/min"), None);
        assert_eq!(flow_from_log("Flow 0.5 ml/min extra"), None);
        assert_eq!(flow_from_log("Set Flow 0.5 ml/min"), None);
        assert_eq!(flow_from_log("Flow0.5 ml/min"), None);
        assert_eq!(flow_from_log(""), None);
    }

    #[test]
    fn inject_variants() {
        let mut s = InstrumentState::default();
        assert!(set_inject_from_log(" INJECTION VALVE LOAD ", &mut s));
        assert_eq!(s.inject_value, InjectValue::Load);
        assert!(set_inject_from_log("Injection Valve Waste", &mut s));
        assert_eq!(s.inject_value, InjectValue::Waste);
        assert!(!set_inject_from_log("Injection Valve Inject", &mut s));
        assert!(!set_inject_from_log("Injection  Valve Load", &mut s));
        assert_eq!(s.inject_value, InjectValue::Waste);
    }

    #[test]
    fn buffer_valve_variants() {
        let mut s = InstrumentState::default();
        assert!(set_buffer_valve_from_log(
            "manual set;buffer valve pos   5",
            &mut s
        ));
        assert_eq!(s.buffer_port, 5);
        assert!(!set_buffer_valve_from_log("Buffer Valve Pos 0", &mut s));
        assert!(!set_buffer_valve_from_log("Buffer Valve Pos 12", &mut s));
        assert!(!set_buffer_valve_from_log(
            "Manual Set; Buffer Valve Pos",
            &mut s
        ));
        assert!(set_buffer_valve_from_log("\tBuffer Valve Pos 1 ", &mut s));
        assert_eq!(s.buffer_port, 1);
    }

    #[test]
    fn fraction_messages_drive_fractions() {
        let mut app = AppState::new();
        app.inst.cum_time_ms = 1000.0;
        app.inst.cum_vol_ml = 0.5;
        let g = app.generation;
        assert!(set_fraction_from_log("Fraction size 1.0 ml", &mut app));
        assert!(app.generation > g);
        assert_eq!(app.inst.fraction_vol_ml, 1.0);
        assert_eq!(app.fractions.len(), 1);
        assert_eq!(app.fractions[0].fraction_number, 1);
        assert_eq!(app.inst.current_fraction_idx, Some(0));

        app.inst.cum_time_ms = 2000.0;
        app.inst.cum_vol_ml = 1.5;
        assert!(set_fraction_from_log("Feed Tube Nr 2", &mut app));
        assert_eq!(app.fractions.len(), 2);
        assert_eq!(app.fractions[0].end_time_ms, Some(2000.0));
        assert_eq!(app.fractions[1].fraction_number, 2);

        // tube 0 counts as tube 1
        assert!(set_fraction_from_log("feed tube nr 0", &mut app));
        assert_eq!(app.inst.fraction_number, 1);
        assert_eq!(app.fractions.len(), 3);

        app.inst.cum_time_ms = 3000.0;
        assert!(set_fraction_from_log("Fraction size 0 ml", &mut app));
        assert_eq!(app.inst.current_fraction_idx, None);
        assert_eq!(app.fractions[2].end_time_ms, Some(3000.0));

        // "Fraction Off" (a method step) stops as size 0 does
        assert!(set_fraction_from_log("Fraction size 2.0 ml", &mut app));
        assert_eq!(app.fractions.len(), 4);
        app.inst.cum_time_ms = 4000.0;
        assert!(set_fraction_from_log(" fraction off ", &mut app));
        assert_eq!(app.inst.fraction_vol_ml, 0.0);
        assert_eq!(app.inst.current_fraction_idx, None);
        assert_eq!(app.fractions[3].end_time_ms, Some(4000.0));
        assert!(!set_fraction_from_log("Fraction Off now", &mut app));

        // anchored: these do not match
        assert!(!set_fraction_from_log("Fraction size 1.0 ml/min", &mut app));
        assert!(!set_fraction_from_log("New Fraction size 1.0 ml", &mut app));
        assert!(!set_fraction_from_log("Fraction size 1.0ml", &mut app));
        assert!(!set_fraction_from_log("Feed Tube Nr", &mut app));
        assert!(!set_fraction_from_log(
            "Feed Tube Nr 99999999999999999999999",
            &mut app
        ));
        assert_eq!(app.fractions.len(), 4);
        // no events are recorded by the decoder itself
        assert!(app.events.iter().all(|e| e.kind != EventKind::Log));
    }

    #[test]
    fn run_start() {
        assert!(is_run_start("End", "Run"));
        assert!(is_run_start("---", "Run"));
        assert!(!is_run_start("Pause", "Run"));
        assert!(!is_run_start("Hold", "Run"));
        assert!(!is_run_start("Continue", "Run"));
        assert!(!is_run_start("Run", "Run"));
        assert!(!is_run_start("End", "Pause"));
        assert!(!is_run_start("End", "run"));
    }

    #[test]
    fn log_number_parsing() {
        assert_eq!(parse_f64(" 1.5\t"), Some(1.5));
        assert_eq!(parse_f64("+1.5"), Some(1.5));
        assert_eq!(parse_f64(".5"), Some(0.5));
        assert_eq!(parse_f64("5."), Some(5.0));
        assert_eq!(parse_f64("1E3"), Some(1000.0));
        assert_eq!(parse_f64("-Inf"), Some(f64::NEG_INFINITY));
        assert!(parse_f64("NaN").is_some_and(f64::is_nan));
        assert_eq!(parse_f64(""), None);
        assert_eq!(parse_f64("  "), None);
        assert_eq!(parse_f64("1,5"), None);
        assert_eq!(parse_f64("1_000"), None);
        assert_eq!(trim_space("\u{2028}x "), "\u{2028}x");
    }
}
