//! Method dump decoding and method files.
//!
//! The ÄKTAprime sends a method dump just before a run starts (OPC 0x2007,
//! content in 0x8008–0x8011, closed by 0x2008). The dump is a printer-formatted
//! text report, escaped like all method text (see `store`); decoded it reads
//! ```text
//!   ** Method Settings
//!   Method base        ,ml
//!   Brkp Volume   Conc Flow  Frac   Buff Inj    Peak   Autozero Eventmark
//!   ,1   ,0.0     ,0   ,0.5  ,0.0   ,8   ,Load  ,0     ,N       ,N
//! ```
//! It holds a "Method Settings" block of `name ,value` lines and a breakpoint
//! table with the columns Brkp, Volume (mL), Conc (%B), Flow (mL/min), Frac (mL),
//! Buff (buffer valve port), Inj (injection valve), Peak (mAU/min), Autozero and
//! Eventmark. The first breakpoint (volume 0) is the state the run starts in.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use regex::Regex;

use rc_core::{AppState, InjectValue};

/// A decoded method dump / method text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MethodDump {
    pub settings: Vec<(String, String)>,
    pub columns: Vec<String>,
    pub breakpoints: Vec<BTreeMap<String, String>>,
}

/// A method dump as the Akta sends it (escaped).
pub fn parse_method_dump(raw: &[u8]) -> MethodDump {
    parse_method_text(&crate::store::unescape_method_text(raw))
}

/// Lines split at CR LF, CR or LF. Unlike `str::lines`, a lone CR also ends a
/// line and a trailing line break leaves an empty last line.
fn text_lines(s: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let b = s.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\r' => {
                lines.push(&s[start..i]);
                i += if b.get(i + 1) == Some(&b'\n') { 2 } else { 1 };
                start = i;
            }
            b'\n' => {
                lines.push(&s[start..i]);
                i += 1;
                start = i;
            }
            _ => i += 1,
        }
    }
    lines.push(&s[start..]);
    lines
}

/// Method text as stored in a PrimeMeth_NNN.txt file.
pub fn parse_method_text(bytes: &[u8]) -> MethodDump {
    let text = String::from_utf8_lossy(bytes);
    let mut m = MethodDump::default();
    for line in text_lines(&text) {
        let line = line.trim_end();
        if line.trim().is_empty() || line.starts_with('*') {
            continue; // rules and headings
        }
        if line.starts_with("Brkp") {
            m.columns = line.split_whitespace().map(str::to_string).collect();
        } else if line.starts_with(',') {
            // Each value is prefixed by ','; padding before it is optional ("99999.9 ,0").
            let values: Vec<&str> = line.split(',').skip(1).map(str::trim).collect();
            if values.len() == m.columns.len() {
                let bp = m
                    .columns
                    .iter()
                    .cloned()
                    .zip(values.iter().map(|v| v.to_string()))
                    .collect();
                m.breakpoints.push(bp);
            }
        } else if let Some((name, value)) = line.split_once(',') {
            m.settings
                .push((name.trim().to_string(), value.trim().to_string()));
        }
    }
    m
}

/// A plain decimal whole number (surrounding spaces allowed); `None` otherwise.
fn parse_int(s: &str) -> Option<i64> {
    s.trim().parse().ok()
}

/// Take the run's starting state from the first breakpoint.
pub fn apply_method_dump(app: &mut AppState, m: &MethodDump) {
    let settings: Vec<String> = m
        .settings
        .iter()
        .map(|(k, v)| format!("{k} = {v}"))
        .collect();
    log::info!("Method settings: {}", settings.join("; "));
    let Some(bp) = m.breakpoints.first() else {
        log::warn!("Method dump without breakpoints");
        return;
    };
    let get = |k: &str| bp.get(k).map(String::as_str);

    if let Some(port) = parse_int(get("Buff").unwrap_or(""))
        && (1..=8).contains(&port)
    {
        app.inst.buffer_port = port;
        app.inst.buffer_port_known = true;
    }
    match get("Inj").unwrap_or("").to_lowercase().as_str() {
        "load" => app.inst.inject_value = InjectValue::Load,
        "inj" => app.inst.inject_value = InjectValue::Inject,
        "waste" => app.inst.inject_value = InjectValue::Waste,
        _ => {}
    }

    let txt = format!(
        "Method start: flow {} ml/min, {} %B, buffer port {}, inject {}, fraction {} ml, {} breakpoints",
        get("Flow").unwrap_or("?"),
        get("Conc").unwrap_or("?"),
        get("Buff").unwrap_or("?"),
        get("Inj").unwrap_or("?"),
        get("Frac").unwrap_or("?"),
        m.breakpoints.len()
    );
    log::info!("{txt}");
    app.pending_method_event = Some(txt);
    app.touch();
}

// ─────────────────────────────────────────────────────────────────────────────
// Stored method files — the model behind the method editor
// ─────────────────────────────────────────────────────────────────────────────
// A PrimeMeth_NNN.txt file is the decoded method dump: a fixed header, the
// settings (name padded to 19 characters, ',' value) and the breakpoint table,
// each field ',' + value padded to its column width. The instrument loads it only
// with at least two breakpoints: the last one is the end of the method
// (99999.9 = run until stopped). `format_text` writes exactly what the
// instrument writes (header, padding, CRLF), checked against its own files.

pub const BP_COLUMNS: [&str; 9] = [
    "Volume",
    "Conc",
    "Flow",
    "Frac",
    "Buff",
    "Inj",
    "Peak",
    "Autozero",
    "Eventmark",
];
/// Brkp, then BP_COLUMNS but the last (unpadded)
const BP_WIDTHS: [usize; 9] = [4, 8, 4, 5, 6, 4, 6, 6, 8];
pub const METHOD_END_VOLUME: &str = "99999.9";
pub const INJ_OPTIONS: [&str; 3] = ["Load", "Inj", "Waste"];
pub const BASE_OPTIONS: [&str; 2] = ["ml", "min"];
pub const YN_OPTIONS: [&str; 2] = ["N", "Y"];

const TABLE_COLUMNS_LINE: &str =
    "Brkp Volume   Conc Flow  Frac   Buff Inj    Peak   Autozero Eventmark";

/// `s` padded with spaces on the right to `n` characters (not bytes).
fn rpad(s: &str, n: usize) -> String {
    let len = s.chars().count();
    let mut out = String::with_capacity(s.len() + n.saturating_sub(len));
    out.push_str(s);
    out.extend(std::iter::repeat_n(' ', n.saturating_sub(len)));
    out
}

fn method_head() -> Vec<String> {
    vec![
        "*".repeat(65),
        "** Method Dump".into(),
        "**".into(),
        rpad("** AKTAprime   Ver  , V2.01", 42),
        "**".into(),
        rpad("** Dump Format Ver  , V1.00", 42),
        "**".into(),
        "*".repeat(65),
        String::new(),
        String::new(),
        "*".repeat(18),
        "** Method Settings".into(),
        "*".repeat(18),
    ]
}

fn method_table_head() -> Vec<String> {
    vec![
        String::new(),
        "*".repeat(21),
        "** Method Breakpoints".into(),
        "*".repeat(21),
        " ".repeat(21),
        TABLE_COLUMNS_LINE.into(),
        " ".repeat(42) + "(mAu/min)",
        String::new(),
    ]
}

/// A method file: settings in file order (values as written, no padding) and
/// breakpoint rows with values in `BP_COLUMNS` order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MethodFile {
    pub settings: Vec<(String, String)>,
    pub rows: Vec<Vec<String>>,
}

#[derive(PartialEq)]
enum Section {
    Head,
    Settings,
    Table,
}

impl MethodFile {
    /// A new method: like one programmed on the instrument ("-"), start and open end.
    pub fn new_method() -> MethodFile {
        let row: Vec<String> = ["0.0", "0", "1.0", "0.0", "1", "Load", "0", "N", "N"]
            .map(String::from)
            .to_vec();
        let mut end = row.clone();
        end[0] = METHOD_END_VOLUME.to_string();
        let settings = [
            ("Method Type", "-"),
            ("Method", "-"),
            ("Method base", "ml"),
            ("Frac base", "ml"),
            ("Max pressure (kPa)", "500"),
            ("Alarm (min/ml)", "0.0"),
        ]
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .to_vec();
        MethodFile {
            settings,
            rows: vec![row, end],
        }
    }

    /// Parse a method file (as the instrument or the editor writes it): the
    /// settings and the breakpoint rows. Errors on unknown breakpoint columns, a
    /// row with the wrong number of values, or text that is no method file.
    pub fn read(bytes: &[u8]) -> Result<MethodFile, String> {
        let text = String::from_utf8_lossy(bytes);
        let mut m = MethodFile::default();
        let mut section = Section::Head;
        for line in text_lines(&text) {
            if line == "** Method Settings" {
                section = Section::Settings;
            } else if line == "** Method Breakpoints" {
                section = Section::Table;
            } else if line.starts_with("Brkp") {
                let cols: Vec<&str> = line.split_whitespace().collect();
                if cols.first() != Some(&"Brkp") || cols[1..] != BP_COLUMNS {
                    return Err(format!("unknown breakpoint columns: {line}"));
                }
            } else if section == Section::Settings && line.contains(',') && !line.starts_with('*') {
                if let Some((name, value)) = line.split_once(',') {
                    m.settings
                        .push((name.trim().to_string(), value.to_string()));
                }
            } else if section == Section::Table && line.starts_with(',') {
                // [0] is "", [1] the Brkp number
                let values: Vec<String> = line
                    .split(',')
                    .skip(2)
                    .map(|v| v.trim().to_string())
                    .collect();
                if values.len() != BP_COLUMNS.len() {
                    return Err(format!("breakpoint with {} values: {line}", values.len()));
                }
                m.rows.push(values);
            }
        }
        if section != Section::Table {
            return Err("not a method file".into());
        }
        Ok(m)
    }

    /// The method file text as the instrument writes it (CRLF line endings,
    /// padded columns).
    pub fn format_text(&self) -> Vec<u8> {
        let mut lines = method_head();
        lines.extend(
            self.settings
                .iter()
                .map(|(k, v)| format!("{},{v}", rpad(k, 19))),
        );
        lines.extend(method_table_head());
        for (i, row) in self.rows.iter().enumerate() {
            let mut vals: Vec<&str> = Vec::with_capacity(row.len() + 1);
            let brkp = (i + 1).to_string();
            vals.push(&brkp);
            vals.extend(row.iter().map(String::as_str));
            let mut line = String::new();
            for (v, &w) in vals.iter().zip(BP_WIDTHS.iter()) {
                line.push(',');
                line.push_str(&rpad(v, w.max(v.chars().count() + 1)));
            }
            line.push(',');
            line.push_str(vals.last().copied().unwrap_or(""));
            lines.push(line);
        }
        let mut out = lines.join("\r\n");
        out.push_str("\r\n");
        out.into_bytes()
    }

    /// Setting value or "".
    pub fn setting(&self, name: &str) -> &str {
        self.settings
            .iter()
            .find(|(k, _)| k == name)
            .map_or("", |(_, v)| v.as_str())
    }

    pub fn set_setting(&mut self, name: &str, value: &str) {
        match self.settings.iter_mut().find(|(k, _)| k == name) {
            Some((_, v)) => *v = value.to_string(),
            None => self.settings.push((name.to_string(), value.to_string())),
        }
    }

    /// Every setting and breakpoint value written as the instrument writes it
    /// (see [`normalize_value`]).
    pub fn normalized(&self) -> MethodFile {
        MethodFile {
            settings: self
                .settings
                .iter()
                .map(|(k, v)| (k.clone(), normalize_value(k, v)))
                .collect(),
            rows: self
                .rows
                .iter()
                .map(|row| {
                    BP_COLUMNS
                        .iter()
                        .zip(row)
                        .map(|(c, v)| normalize_value(c, v))
                        .collect()
                })
                .collect(),
        }
    }

    /// What is wrong with the method (invalid values, too few breakpoints,
    /// volumes out of order): messages, and bad values as (row, column name),
    /// rows numbered from 1, row 0 for the settings.
    pub fn problems(&self) -> (Vec<String>, BTreeSet<(usize, String)>) {
        let mut msgs = Vec::new();
        let mut bad = BTreeSet::new();
        for (k, v) in &self.settings {
            if !method_value_ok(k, v) {
                bad.insert((0, k.clone()));
                msgs.push(format!("{k}: \"{v}\" is not a valid value"));
            }
        }
        for (i, row) in self.rows.iter().enumerate() {
            for (c, v) in BP_COLUMNS.iter().zip(row) {
                if !method_value_ok(c, v) {
                    bad.insert((i + 1, c.to_string()));
                    msgs.push(format!(
                        "Breakpoint {}, {c}: \"{v}\" is not a valid value",
                        i + 1
                    ));
                }
            }
        }
        if self.rows.len() < 2 {
            msgs.push(
                "At least two breakpoints are needed: the Akta cannot load a method without its end breakpoint"
                    .into(),
            );
        }
        let vols: Vec<f64> = self
            .rows
            .iter()
            .map(|r| match r.first() {
                Some(v) if method_value_ok("Volume", v) => {
                    number_text(v).parse().unwrap_or(f64::NAN)
                }
                _ => f64::NAN,
            })
            .collect();
        if let Some(&v1) = vols.first()
            && !v1.is_nan()
            && v1 != 0.0
        {
            bad.insert((1, "Volume".to_string()));
            msgs.push("The first breakpoint must be at 0.0".into());
        }
        for i in 1..vols.len() {
            // false for NaN, already reported
            if vols[i] <= vols[i - 1] {
                bad.insert((i + 1, "Volume".to_string()));
                msgs.push(format!(
                    "Breakpoint {} must come after breakpoint {i}",
                    i + 1
                ));
            }
        }
        (msgs, bad)
    }
}

// What each value may be. The number limits are generous where the instrument's
// own are not known; Method base/Frac base "min" and Inj "Inj" are not yet seen in
// a method from the instrument.
struct ValueRule {
    re: Regex,
    lo: f64,
    hi: f64,
}

static VALUE_RULES: LazyLock<BTreeMap<&'static str, ValueRule>> = LazyLock::new(|| {
    [
        ("Volume", r"^[0-9]{1,5}(\.[0-9])?$", 0.0, 99999.9),
        ("Conc", r"^[0-9]{1,3}$", 0.0, 100.0),
        ("Flow", r"^[0-9]{1,2}(\.[0-9])?$", 0.0, 50.0),
        ("Frac", r"^[0-9]{1,3}(\.[0-9])?$", 0.0, 100.0),
        ("Buff", r"^[1-8]$", 1.0, 8.0),
        ("Peak", r"^[0-9]{1,3}(\.[0-9]{1,2})?$", 0.0, 999.99),
        ("Max pressure (kPa)", r"^[0-9]{1,4}$", 1.0, 1000.0),
        ("Alarm (min/ml)", r"^[0-9]{1,3}(\.[0-9])?$", 0.0, 999.9),
    ]
    .into_iter()
    .map(|(name, re, lo, hi)| {
        (
            name,
            ValueRule {
                re: Regex::new(re).expect("valid value regex"),
                lo,
                hi,
            },
        )
    })
    .collect()
});

const ONE_DECIMAL: [&str; 4] = ["Volume", "Flow", "Frac", "Alarm (min/ml)"];

/// A decimal comma is accepted while typing.
fn number_text(s: &str) -> String {
    s.trim().replace(',', ".")
}

/// Allowed values for a choice field (Inj, Autozero, Eventmark, Method base, Frac base).
pub fn value_choices(name: &str) -> Option<&'static [&'static str]> {
    match name {
        "Inj" => Some(&INJ_OPTIONS),
        "Autozero" | "Eventmark" => Some(&YN_OPTIONS),
        "Method base" | "Frac base" => Some(&BASE_OPTIONS),
        _ => None,
    }
}

pub fn method_value_ok(name: &str, s: &str) -> bool {
    if let Some(choices) = value_choices(name) {
        return choices.contains(&s.trim());
    }
    if let Some(rule) = VALUE_RULES.get(name) {
        let t = number_text(s);
        return rule.re.is_match(&t)
            && t.parse::<f64>().is_ok_and(|v| rule.lo <= v && v <= rule.hi);
    }
    // free text (Method Type, Method): printable ASCII without the field separator
    !s.trim().is_empty()
        && s.chars().count() <= 60
        && s.chars().all(|c| (' '..='~').contains(&c) && c != ',')
}

/// The value as the instrument writes it: 0.5 → "0.5", 1 → "1.0", %B 05 → "5", peak 0.10 → "0.1".
/// A number field that cannot be written as a number (no number; Inf, NaN or
/// out of range for a whole-number field) is returned unchanged, so the
/// problem stays visible to the user instead of being replaced.
pub fn normalize_value(name: &str, s: &str) -> String {
    if !VALUE_RULES.contains_key(name) {
        return if value_choices(name).is_some() {
            s.trim().to_string()
        } else {
            s.to_string()
        };
    }
    let Ok(v) = number_text(s).parse::<f64>() else {
        return s.to_string();
    };
    if ONE_DECIMAL.contains(&name) {
        return fixed(v, 1);
    }
    // Only finite values within the i64 range become whole numbers.
    let as_int = |v: f64| (v.is_finite() && v.abs() < 9.2e18).then_some(v as i64);
    if name == "Peak" {
        return if v.fract() == 0.0 {
            as_int(v).map_or_else(|| s.to_string(), |i| i.to_string())
        } else {
            fixed(v, 2).trim_end_matches('0').to_string()
        };
    }
    as_int(v.round_ties_even()).map_or_else(|| s.to_string(), |i| i.to_string())
}

/// `v` with `prec` decimals (printf `%.<prec>f`); Inf, -Inf and NaN as "Inf",
/// "-Inf" and "NaN".
fn fixed(v: f64, prec: usize) -> String {
    if v.is_nan() {
        "NaN".into()
    } else if v.is_infinite() {
        if v > 0.0 { "Inf".into() } else { "-Inf".into() }
    } else {
        format!("{v:.prec$}")
    }
}
