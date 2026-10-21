//! Which PC method a run was started from. The Akta does not say: it sends the
//! method it is about to run as a dump (0x2007 … 0x2008), and a method it
//! loaded from the PC arrives with the same text as its PrimeMeth_NNN.txt file
//! (the files are decoded dumps; confirmed on the instrument for method 1).
//! The dump is compared with the stored methods by content, not by bytes:
//! settings and breakpoint values after normalisation, so padding, line endings
//! and number spellings ("1" / "1.0") do not matter.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use rc_core::runinfo::{METHOD_REF_KEY, METHOD_SETUP_FIELDS};

use crate::method::{BP_COLUMNS, MethodDump, normalize_value, parse_method_text};
use crate::store::{find_method_file, method_names, method_setup, stored_methods};

/// What identifies a method's content: its settings (name → normalised value)
/// and its breakpoints (values in `BP_COLUMNS` order, normalised).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MethodContent {
    pub settings: BTreeMap<String, String>,
    pub breakpoints: Vec<Vec<String>>,
}

impl MethodContent {
    pub fn of(m: &MethodDump) -> MethodContent {
        let settings = m
            .settings
            .iter()
            .map(|(k, v)| {
                let k = k.trim();
                (k.to_string(), normalize_value(k, v.trim()))
            })
            .collect();
        let breakpoints = m
            .breakpoints
            .iter()
            .map(|bp| {
                BP_COLUMNS
                    .iter()
                    .map(|c| normalize_value(c, bp.get(*c).map_or("", |v| v.trim())))
                    .collect()
            })
            .collect();
        MethodContent {
            settings,
            breakpoints,
        }
    }
}

/// Numbers of the stored methods with the content of `dump`, ascending. A dump
/// without breakpoints (not understood) matches nothing.
pub fn matching_stored_methods(dir: &Path, dump: &MethodDump) -> Vec<u16> {
    if dump.breakpoints.is_empty() {
        return Vec::new();
    }
    let want = MethodContent::of(dump);
    stored_methods(dir)
        .into_iter()
        .filter(|&n| {
            let Some(path) = find_method_file(dir, n) else {
                return false;
            };
            match fs::read(&path) {
                Ok(bytes) => MethodContent::of(&parse_method_text(&bytes)) == want,
                Err(err) => {
                    log::warn!("Could not read {}: {err}", path.display());
                    false
                }
            }
        })
        .collect()
}

/// The stored method the Akta is running: the one with the dump's content;
/// of several, the one it last loaded from the PC, else the lowest number.
pub fn match_stored_method(dir: &Path, dump: &MethodDump, last_loaded: u16) -> Option<u16> {
    let found = matching_stored_methods(dir, dump);
    if found.contains(&last_loaded) {
        Some(last_loaded)
    } else {
        found.first().copied()
    }
}

/// "005 – Gel filtration" (or "005" for a method without a name).
pub fn method_reference(dir: &Path, n: u16) -> String {
    match method_names(dir).get(&n) {
        Some(name) => format!("{n:03} – {name}"),
        None => format!("{n:03}"),
    }
}

/// Run information for a run of `dump`: the run setup of the matching PC
/// method and which method it is (`METHOD_REF_KEY`). Empty when no stored
/// method matches: a run is never given another method's setup.
pub fn run_setup_for_dump(
    dir: &Path,
    dump: &MethodDump,
    last_loaded: u16,
) -> BTreeMap<String, String> {
    let Some(n) = match_stored_method(dir, dump, last_loaded) else {
        log::info!("The method dump matches no method stored on the PC");
        return BTreeMap::new();
    };
    let mut info: BTreeMap<String, String> = method_setup(dir, n)
        .into_iter()
        .filter(|(k, _)| METHOD_SETUP_FIELDS.contains(&k.as_str()))
        .collect();
    let reference = method_reference(dir, n);
    log::info!("The method dump is PC method {reference}");
    info.insert(METHOD_REF_KEY.to_string(), reference);
    info
}
