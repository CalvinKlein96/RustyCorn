//! Which PC method a run was started from. The Akta sends the method it is
//! about to run as a dump (0x2007 … 0x2008). Its first two settings say where
//! the method is kept, not what it does: "-" / "-" in a file saved to the PC,
//! "Manaul Method" (sic) and the slot for one in the instrument's memory, and
//! "PC Method" and the number for one loaded from the PC (debug run on the
//! instrument, 2026-11-13: PrimeMeth_009.txt ran as "PC Method" / "9"). So:
//!
//! * a dump of "PC Method" N is PC method N, when it is stored;
//! * any other dump is compared with the stored methods by content, not by
//!   bytes: the other settings and the breakpoint values after normalisation,
//!   so padding, line endings and number spellings ("1" / "1.0") do not matter.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use rc_core::runinfo::{METHOD_REF_KEY, METHOD_SETUP_FIELDS};

use crate::method::{BP_COLUMNS, MethodDump, normalize_value, parse_method_text};
use crate::store::{find_method_file, method_names, method_setup, stored_methods};

/// The settings that say where a method is kept ("Method Type", "Method"), not
/// what it does: not part of its content.
const STORAGE_SETTINGS: [&str; 2] = ["Method Type", "Method"];

/// What identifies a method's content: its settings (name → normalised value)
/// but the `STORAGE_SETTINGS`, and its breakpoints (values in `BP_COLUMNS`
/// order, normalised).
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
            .filter(|(k, _)| !STORAGE_SETTINGS.contains(&k.trim()))
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

/// The PC method number a dump names: "Method Type" "PC Method" and "Method" N.
pub fn pc_method_number(dump: &MethodDump) -> Option<u16> {
    let setting = |name: &str| {
        dump.settings
            .iter()
            .find(|(k, _)| k.trim() == name)
            .map(|(_, v)| v.trim())
    };
    if !setting("Method Type")?.eq_ignore_ascii_case("PC Method") {
        return None;
    }
    setting("Method")?.parse().ok()
}

/// The stored method the Akta is running: the PC method the dump names, when
/// it is stored; else the one with the dump's content, of several the one it
/// last loaded from the PC, else the lowest number.
pub fn match_stored_method(dir: &Path, dump: &MethodDump, last_loaded: u16) -> Option<u16> {
    if let Some(n) = pc_method_number(dump)
        && let Some(path) = find_method_file(dir, n)
    {
        let same = fs::read(&path)
            .is_ok_and(|b| MethodContent::of(&parse_method_text(&b)) == MethodContent::of(dump));
        if !same {
            log::warn!(
                "The Akta runs PC method {n}, but {} has changed since it was loaded",
                path.display()
            );
        }
        return Some(n);
    }
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
