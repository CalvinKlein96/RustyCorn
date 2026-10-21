//! Example methods written into a new, empty methods folder, so a first look
//! at the method editor (and a first run from the PC) has something to show.
//! They are written with the normal store functions, so the files are exactly
//! what the method editor writes. A folder already in use is never touched,
//! and a method the user deleted is never brought back.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use rc_core::runinfo::METHOD_NOTES_KEY;

use crate::method::MethodFile;
use crate::store::{
    METHOD_FILE_PREFIX, METHOD_NAMES_FILE, METHOD_OLD_DIR, save_method_file, set_method_name,
    set_method_setup,
};

/// Number of the example His-tag purification method.
pub const EXAMPLE_METHOD_NUMBER: u16 = 1;
/// Its name in names.txt.
pub const EXAMPLE_METHOD_NAME: &str = "His-tag purification, HisTrap 1 ml (example)";

/// A His-tag purification on a 1 ml HisTrap column at 1.0 ml/min: equilibrate
/// 5 ml, load 10 ml of sample through buffer valve port 8, wash 10 ml, elute
/// with a 20 ml linear gradient to 100 %B and hold it 5 ml, collecting 1 ml
/// fractions, then back to 0 %B and re-equilibrate. %B runs linearly between
/// breakpoints, so a level is held with two breakpoints at the same %B.
pub fn example_his_tag_method() -> MethodFile {
    // Volume, Conc (%B), Flow, Frac, Buff, Autozero
    let rows: [(&str, &str, &str, &str, &str); 9] = [
        ("0.0", "0", "0.0", "1", "Y"),    // equilibrate
        ("5.0", "0", "0.0", "8", "N"),    // load the sample (10 ml)
        ("15.0", "0", "0.0", "1", "N"),   // wash
        ("25.0", "0", "1.0", "1", "N"),   // gradient start, fractions on
        ("45.0", "100", "1.0", "1", "N"), // gradient end
        ("50.0", "100", "1.0", "1", "N"), // hold 100 %B
        ("50.1", "0", "0.0", "1", "N"),   // step back to 0 %B, fractions off
        ("55.0", "0", "0.0", "1", "N"),   // re-equilibrated
        ("60.0", "0", "0.0", "1", "N"),   // end
    ];
    let mut m = MethodFile::new_method();
    m.rows = rows
        .iter()
        .map(|&(vol, conc, frac, buff, autozero)| {
            [vol, conc, "1.0", frac, buff, "Load", "0", autozero, "N"]
                .map(String::from)
                .to_vec()
        })
        .collect();
    m
}

/// The run setup of the example method.
pub fn example_his_tag_setup() -> BTreeMap<String, String> {
    let notes = [
        "1. Equilibrate with 5 ml binding buffer (port 1).",
        "2. Load 10 ml of sample through buffer valve port 8.",
        "3. Wash with 10 ml binding buffer.",
        "4. Elute with a 20 ml linear gradient to 100 %B, then hold 100 %B for 5 ml; \
         1 ml fractions from 25 to 50 ml.",
        "5. Back to 0 %B at 50.1 ml and re-equilibrate; the method ends at 60 ml.",
        "All at 1.0 ml/min. The 500 kPa pressure limit is the column's; lower it if \
         the system's own back pressure is high.",
    ]
    .join("\n");
    [
        ("Column", "HisTrap HP 1 ml"),
        ("Column volume (mL)", "1.0"),
        (
            "Buffer B",
            "20 mM sodium phosphate, 500 mM NaCl, 500 mM imidazole, pH 7.4",
        ),
        (
            "Port 1",
            "Binding buffer: 20 mM sodium phosphate, 500 mM NaCl, 20 mM imidazole, pH 7.4",
        ),
        ("Port 8", "Sample (filtered, 0.45 µm)"),
        (METHOD_NOTES_KEY, notes.as_str()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

/// True when `dir` is a new methods folder: it does not exist yet, or it holds
/// no `PrimeMeth_*.txt` file, no names.txt and no `old/` folder. False when it
/// cannot be read.
fn is_fresh_methods_folder(dir: &Path) -> bool {
    if !dir.exists() {
        return true;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    let prefix = METHOD_FILE_PREFIX.to_lowercase();
    for entry in entries {
        let Ok(entry) = entry else {
            return false;
        };
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if (name.starts_with(&prefix) && name.ends_with(".txt"))
            || name == METHOD_NAMES_FILE
            || name == METHOD_OLD_DIR
        {
            return false;
        }
    }
    true
}

/// Writes the example methods into `dir` if it is a new methods folder (see
/// [`is_fresh_methods_folder`]); returns whether it did. A folder already in
/// use is left as it is.
pub fn install_example_methods(dir: &Path) -> io::Result<bool> {
    if !is_fresh_methods_folder(dir) {
        return Ok(false);
    }
    let n = EXAMPLE_METHOD_NUMBER;
    save_method_file(dir, n, &example_his_tag_method().format_text())?;
    set_method_name(dir, n, EXAMPLE_METHOD_NAME)?;
    set_method_setup(dir, n, &example_his_tag_setup())?;
    log::info!(
        "New methods folder {}: example method {n:03} written ({EXAMPLE_METHOD_NAME})",
        dir.display()
    );
    Ok(true)
}

/// [`install_example_methods`], with an error only logged (for callers that go
/// on without the example).
pub fn install_example_methods_logged(dir: &Path) {
    if let Err(err) = install_example_methods(dir) {
        log::warn!(
            "Could not write the example method into {}: {err}",
            dir.display()
        );
    }
}
