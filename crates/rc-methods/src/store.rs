//! The PC method store: the ÄKTAprime saves methods to the PC and loads them
//! back ("Run Stored Method → PC"), with the PC acting as a numbered file store.
//! This follows the instrument's PC protocol and the method file layout of
//! PrimeView (UNICORN 5), so PrimeView's method files and ours are
//! interchangeable: one text file per method,
//! `<methods folder>/PrimeMeth_NNN.txt` (NNN = 001–999), CRLF line endings.
//!
//! ```text
//! Instrument → PC                            PC → instrument
//!   0x2007                 dump begin          –   (method report before a run)
//!   0x2008 [x]             dump end            –
//!   0x2009 [u16 n]         save begin          –
//!   0x8008–0x8011          method text         –   (during a dump or save)
//!   0x2010                 save end            0x2011 [1 saved / 0 not saved]
//!   0x2015 [u16 n, u8 up]  list methods        0x2016 [10 × u16 numbers, 0xFFFF = none]
//!   0x2019 [u16 n]         does n exist?       0x2020 [1 / 0]
//!   0x2017 [u16 n]         delete n            0x2018 [1 deleted / 0 not]
//!   0x2012 [u16 n]         load n              0x2013, the file's lines, 0x2014 [0 ok / 2 error]
//!                                              or just 0x2014 [1] if there is no such file
//! ```
//! Numbers are little-endian. Method text travels in ASCII messages: 0x8008 is a
//! whole line; a line longer than 34 characters goes as 0x8009, 0x8010…, 0x8011.
//!
//! Text escaping (both directions; '#' starts a link-layer frame):
//! `zy` → '#', `z<c>` → c (for c in `z { | } ~`), `~` → CR, `}` → CR LF,
//! `|<n>` → n spaces, `{<n>` → n '*' (n = count + '0', runs of 2–60).

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};

use regex::Regex;

use rc_core::opc::*;
use rc_core::runinfo::{METHOD_SETUP_FIELDS, is_multiline_field};

pub const METHOD_FILE_PREFIX: &str = "PrimeMeth_";
pub const METHOD_NUMBERS: std::ops::RangeInclusive<u16> = 1..=999;
/// numbers per 0x2016 reply
pub const METHOD_LIST_LENGTH: usize = 10;
/// characters per method text message
pub const METHOD_CHUNK: usize = 34;
/// longest line a load may send; longer lines abort the load
pub const METHOD_LINE_MAX: usize = 99;
/// empty slot in a 0x2016 reply
pub const METHOD_NONE: u16 = 0xffff;
pub const METHOD_NAMES_FILE: &str = "names.txt";
/// first line of names.txt
pub const METHOD_NAMES_HEADER: &str =
    "# Method names from the RustyCorn method editor; not sent to the instrument.";
/// folder (inside the methods folder) for files the Akta deleted or overwrote
pub const METHOD_OLD_DIR: &str = "old";

pub fn method_path(dir: &Path, n: u16) -> PathBuf {
    dir.join(format!("{METHOD_FILE_PREFIX}{n:03}.txt"))
}

static METHOD_FILE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^primemeth_([0-9]{3})\.txt$").expect("valid method file regex")
});

/// File names in `dir`, sorted; empty if it cannot be read.
fn dir_entries(dir: &Path) -> Vec<String> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}

/// Numbers of the stored methods, ascending.
pub fn stored_methods(dir: &Path) -> Vec<u16> {
    if !dir.is_dir() {
        return Vec::new();
    }
    let mut nums = Vec::new();
    for f in dir_entries(dir) {
        let Some(m) = METHOD_FILE_RE.captures(&f) else {
            continue;
        };
        let Ok(n) = m[1].parse::<u16>() else {
            continue;
        };
        if METHOD_NUMBERS.contains(&n) && dir.join(&f).is_file() {
            nums.push(n);
        }
    }
    nums.sort_unstable();
    nums.dedup();
    nums
}

/// The file of method n, matched case-insensitively (UNICORN runs on Windows).
pub fn find_method_file(dir: &Path, n: u16) -> Option<PathBuf> {
    let path = method_path(dir, n);
    if path.is_file() {
        return Some(path);
    }
    if !dir.is_dir() {
        return None;
    }
    let want = format!("{METHOD_FILE_PREFIX}{n:03}.txt").to_lowercase();
    dir_entries(dir)
        .into_iter()
        .find(|f| f.to_lowercase() == want)
        .map(|f| dir.join(f))
}

// ── Run setup ─────────────────────────────────────────────────────────────────
// The column and buffers a method is run with, and its notes
// (rc_core::runinfo::METHOD_SETUP_FIELDS), stay on the PC, next to the method:
// <methods folder>/PrimeMeth_NNN_setup.txt, one "Key: value" line per field, as
// the EVT_INFO events of a log write them. The notes may span several lines:
// in the file a line break is written "\n" and a backslash "\\", so each field
// stays on one line (files without notes are as before). The method file
// itself is never touched, so it stays what the Akta reads. The setup goes
// wherever the method file goes (old/ when it is deleted or replaced).

/// Ending of a run setup file: `PrimeMeth_005.txt` → `PrimeMeth_005_setup.txt`.
pub const METHOD_SETUP_SUFFIX: &str = "_setup.txt";
/// First line of a run setup file.
pub const METHOD_SETUP_HEADER: &str = "# Run setup (column, buffers, notes) from the RustyCorn method editor; not sent to the instrument.";
/// Second line of a run setup file with multi-line notes: how to read them.
pub const METHOD_SETUP_ESCAPES_NOTE: &str =
    "# In Method notes, \\n is a line break and \\\\ a backslash.";

pub fn method_setup_path(dir: &Path, n: u16) -> PathBuf {
    dir.join(format!("{METHOD_FILE_PREFIX}{n:03}{METHOD_SETUP_SUFFIX}"))
}

/// The run setup file that belongs to the method file `method_file`, if there
/// is one (matched case-insensitively, like the method files).
pub fn setup_file_of(method_file: &Path) -> Option<PathBuf> {
    let dir = method_file.parent().unwrap_or(Path::new(""));
    let stem = method_file.file_stem()?.to_string_lossy();
    let want = format!("{stem}{METHOD_SETUP_SUFFIX}");
    let exact = dir.join(&want);
    if exact.is_file() {
        return Some(exact);
    }
    let want = want.to_lowercase();
    let dir_or_cwd = if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dir
    };
    dir_entries(dir_or_cwd)
        .into_iter()
        .find(|f| f.to_lowercase() == want)
        .map(|f| dir.join(f))
        .filter(|p| p.is_file())
}

/// The run setup file of method n, if there is one.
pub fn find_method_setup_file(dir: &Path, n: u16) -> Option<PathBuf> {
    setup_file_of(&method_path(dir, n))
}

/// A multi-line value on one line: line breaks (`\n`, `\r\n`, `\r`) become
/// `\n`, a backslash `\\`.
pub fn escape_setup_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push_str("\\n");
            }
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out
}

/// Inverse of [`escape_setup_value`]; a backslash before anything else is kept.
pub fn unescape_setup_value(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, chars.peek()) {
            ('\\', Some('n')) => {
                chars.next();
                out.push('\n');
            }
            ('\\', Some('\\')) => {
                chars.next();
                out.push('\\');
            }
            (c, _) => out.push(c),
        }
    }
    out
}

/// The run setup in a setup file's text: the METHOD_SETUP_FIELDS found, values
/// trimmed (the notes' line breaks decoded); comments, empty values and other
/// keys are left out.
pub fn parse_method_setup(text: &str) -> BTreeMap<String, String> {
    let mut setup = BTreeMap::new();
    for line in text.lines() {
        if line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        if !METHOD_SETUP_FIELDS.contains(&key) || value.is_empty() {
            continue;
        }
        let value = if is_multiline_field(key) {
            unescape_setup_value(value).trim().to_string()
        } else {
            value.to_string()
        };
        if !value.is_empty() {
            setup.insert(key.to_string(), value);
        }
    }
    setup
}

/// Text of a run setup file: the header, then the fields set, in their usual
/// order. The notes keep their line breaks (escaped); in other values a line
/// break becomes a space (one field per line).
pub fn format_method_setup(setup: &BTreeMap<String, String>) -> String {
    let mut text = format!("{METHOD_SETUP_HEADER}\n");
    let mut lines = String::new();
    let mut escaped = false;
    for key in METHOD_SETUP_FIELDS {
        let value = setup.get(key).map_or("", |v| v.trim());
        if value.is_empty() {
            continue;
        }
        let value: String = if is_multiline_field(key) {
            let v = escape_setup_value(value);
            escaped |= v != value;
            v
        } else {
            value
                .chars()
                .map(|c| if c == '\r' || c == '\n' { ' ' } else { c })
                .collect()
        };
        lines.push_str(&format!("{key}: {value}\n"));
    }
    if escaped {
        text.push_str(&format!("{METHOD_SETUP_ESCAPES_NOTE}\n"));
    }
    text.push_str(&lines);
    text
}

/// Only the METHOD_SETUP_FIELDS with a value, trimmed (what a save keeps).
pub fn normalized_setup(setup: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    parse_method_setup(&format_method_setup(setup))
}

/// The run setup of method n (empty if it has none or it cannot be read).
pub fn method_setup(dir: &Path, n: u16) -> BTreeMap<String, String> {
    let Some(path) = find_method_setup_file(dir, n) else {
        return BTreeMap::new();
    };
    match fs::read(&path) {
        Ok(bytes) => parse_method_setup(&String::from_utf8_lossy(&bytes)),
        Err(err) => {
            log::warn!("Could not read {}: {err}", path.display());
            BTreeMap::new()
        }
    }
}

/// Sets the run setup of method n; an empty setup removes the file. The
/// method file is not touched.
pub fn set_method_setup(dir: &Path, n: u16, setup: &BTreeMap<String, String>) -> io::Result<()> {
    let setup = normalized_setup(setup);
    let existing = find_method_setup_file(dir, n);
    if setup.is_empty() {
        return match existing {
            Some(path) => fs::remove_file(path),
            None => Ok(()),
        };
    }
    if existing.is_some() && method_setup(dir, n) == setup {
        return Ok(());
    }
    fs::create_dir_all(dir)?;
    let tmp = write_temp_file(dir, format_method_setup(&setup).as_bytes())?;
    // a differently cased old file would otherwise stay next to the new one
    if let Some(old) = existing.filter(|p| *p != method_setup_path(dir, n)) {
        let _ = fs::remove_file(old);
    }
    rename_or_clean(&tmp, &method_setup_path(dir, n))
}

// ── Method names ──────────────────────────────────────────────────────────────
// Names given in the method editor stay on the PC: <methods folder>/names.txt,
// one "NNN<tab>name" line per method. The instrument never sees them.

static NAME_LINE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([0-9]{1,3})\t(.+)$").expect("valid name line regex"));

/// Names given to methods on the PC (names.txt).
pub fn method_names(dir: &Path) -> BTreeMap<u16, String> {
    let mut names = BTreeMap::new();
    let path = dir.join(METHOD_NAMES_FILE);
    if !path.is_file() {
        return names;
    }
    let bytes = match fs::read(&path) {
        Ok(b) => b,
        Err(err) => {
            log::warn!("Could not read {}: {err}", path.display());
            return names;
        }
    };
    let text = String::from_utf8_lossy(&bytes);
    // `lines` drops the "\n" or "\r\n" ending each line.
    for line in text.lines() {
        if let Some(m) = NAME_LINE_RE.captures(line)
            && let Ok(n) = m[1].parse::<u16>()
        {
            names.insert(n, m[2].to_string());
        }
    }
    names
}

/// Printable characters: letters, marks, numbers, punctuation, symbols and space
/// separators; not control, format, line/paragraph separator or private-use characters.
fn is_print(c: char) -> bool {
    let u = c as u32;
    !(c.is_control()
        || matches!(u,
            0x00ad | 0x0600..=0x0605 | 0x061c | 0x06dd | 0x070f | 0x0890..=0x0891 | 0x08e2
            | 0x180e | 0x200b..=0x200f | 0x2028..=0x202e | 0x2060..=0x2064 | 0x2066..=0x206f
            | 0xfeff | 0xfff9..=0xfffb | 0x110bd | 0x110cd | 0x13430..=0x1343f
            | 0x1bca0..=0x1bca3 | 0x1d173..=0x1d17a | 0xe0001 | 0xe0020..=0xe007f
            | 0xe000..=0xf8ff | 0xf0000..=0xffffd | 0x100000..=0x10fffd))
}

pub fn method_name_ok(s: &str) -> bool {
    s.chars().count() <= 60 && s.chars().all(|c| is_print(c) && c != '\t')
}

/// Sets (or with "" removes) the name of method n.
pub fn set_method_name(dir: &Path, n: u16, name: &str) -> io::Result<()> {
    let mut names = method_names(dir);
    let name = name.trim();
    if names.get(&n).map_or("", String::as_str) == name {
        return Ok(());
    }
    if name.is_empty() {
        names.remove(&n);
    } else {
        names.insert(n, name.to_string());
    }
    let mut text = format!("{METHOD_NAMES_HEADER}\n");
    for (k, v) in &names {
        text.push_str(&format!("{k:03}\t{v}\n"));
    }
    fs::create_dir_all(dir)?;
    let tmp = write_temp_file(dir, text.as_bytes())?;
    rename_or_clean(&tmp, &dir.join(METHOD_NAMES_FILE))
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A new, uniquely named temporary file in `dir` holding `bytes`, readable by
/// other users (mode 0o644). Unique: the editor and the Akta may save at the same moment.
fn write_temp_file(dir: &Path, bytes: &[u8]) -> io::Result<PathBuf> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    loop {
        let k = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = dir.join(format!("rc_{}_{nanos:09}_{k}.tmp", std::process::id()));
        let mut f = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
        {
            Ok(f) => f,
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        };
        let written = f.write_all(bytes).and_then(|_| f.sync_all()).and_then(|_| {
            drop(f);
            set_readable(&tmp)
        });
        return match written {
            Ok(()) => Ok(tmp),
            Err(err) => {
                let _ = fs::remove_file(&tmp);
                Err(err)
            }
        };
    }
}

#[cfg(unix)]
fn set_readable(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o644))
}

#[cfg(not(unix))]
fn set_readable(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// Move `tmp` over `dest`; remove `tmp` if that fails.
fn rename_or_clean(tmp: &Path, dest: &Path) -> io::Result<()> {
    fs::rename(tmp, dest).inspect_err(|_| {
        let _ = fs::remove_file(tmp);
    })
}

// ── Text codec ────────────────────────────────────────────────────────────────

pub fn escape_method_text(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() + s.len() / 8);
    let n = s.len();
    let mut i = 0;
    while i < n {
        let c = s[i];
        if c == b'#' {
            out.extend_from_slice(b"zy");
        } else if (0x7a..=0x7e).contains(&c) {
            out.extend_from_slice(&[0x7a, c]);
        } else if c == 0x0d {
            out.push(b'~');
        } else if c == 0x0a {
            out.push(b'}');
        } else if (c == b' ' || c == b'*') && i + 1 < n && s[i + 1] == c {
            let mut run = 0u8;
            while i < n && s[i] == c && run < 60 {
                run += 1;
                i += 1;
            }
            out.extend_from_slice(&[if c == b' ' { b'|' } else { b'{' }, 0x30 + run]);
            continue;
        } else {
            out.push(c);
        }
        i += 1;
    }
    out
}

pub fn unescape_method_text(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * 2);
    let n = s.len();
    let mut i = 0;
    while i < n {
        let c = s[i];
        if c == b'~' {
            out.push(0x0d);
        } else if c == b'}' {
            out.extend_from_slice(&[0x0d, 0x0a]);
        } else if c == 0x7a && i + 1 < n {
            i += 1;
            out.push(if s[i] == b'y' { b'#' } else { s[i] });
        } else if (c == b'|' || c == b'{') && i + 1 < n {
            i += 1;
            let fill = if c == b'|' { b' ' } else { b'*' };
            let count = usize::from(s[i].saturating_sub(0x30));
            out.extend(std::iter::repeat_n(fill, count));
        } else {
            out.push(c);
        }
        i += 1;
    }
    out
}

/// One line as method text messages (0x8008, or 0x8009/0x8010…/0x8011 chunks).
pub fn method_line_messages(line: &[u8]) -> Vec<OpcMessage> {
    let enc = escape_method_text(line);
    if enc.len() <= METHOD_CHUNK {
        return vec![(OPC_METHOD_LINE, enc)];
    }
    let parts: Vec<&[u8]> = enc.chunks(METHOD_CHUNK).collect();
    let last = parts.len() - 1;
    parts
        .into_iter()
        .enumerate()
        .map(|(i, p)| {
            let opc = if i == 0 {
                OPC_METHOD_LINE_BEGIN
            } else if i == last {
                OPC_METHOD_LINE_END
            } else {
                OPC_METHOD_LINE_CONT
            };
            (opc, p.to_vec())
        })
        .collect()
}

/// The lines a load sends: split at CR or LF, empty lines dropped.
pub fn method_file_lines(bytes: &[u8]) -> Vec<Vec<u8>> {
    bytes
        .split(|&b| b == 0x0d || b == 0x0a)
        .filter(|l| !l.is_empty())
        .map(<[u8]>::to_vec)
        .collect()
}

// ── Requests ──────────────────────────────────────────────────────────────────

/// Little-endian u16 at byte `i` (0-based).
pub fn u16_at(data: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([data[i], data[i + 1]])
}

/// 0x2016 payload: the first ten stored methods from `start` (inclusive) up or down.
/// A start outside 1–999 lists nothing.
pub fn method_list_payload(nums: &[u16], start: u16, up: bool) -> Vec<u8> {
    let mut found: Vec<u16> = if !METHOD_NUMBERS.contains(&start) {
        Vec::new()
    } else if up {
        let mut f: Vec<u16> = nums.iter().copied().filter(|&n| n >= start).collect();
        f.sort_unstable();
        f
    } else {
        let mut f: Vec<u16> = nums.iter().copied().filter(|&n| n <= start).collect();
        f.sort_unstable_by(|a, b| b.cmp(a));
        f
    };
    found.truncate(METHOD_LIST_LENGTH);
    found.resize(METHOD_LIST_LENGTH, METHOD_NONE);
    found.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// Move a file the instrument is about to overwrite or delete to `<dir>/old/`,
/// so nothing written in the method editor is ever lost. Returns the new path.
/// The method's run setup goes with it (`old/PrimeMeth_NNN_<time>_setup.txt`).
pub fn retire_method_file(path: &Path) -> io::Result<PathBuf> {
    let setup = setup_file_of(path);
    let dest = retire_to_old(path, "")?;
    if let Some(setup) = setup
        && let Err(err) = retire_to_old(&setup, METHOD_SETUP_SUFFIX)
    {
        log::warn!("Could not move {} to old/: {err}", setup.display());
    }
    Ok(dest)
}

/// Move `path` to `<its dir>/old/<method stem>_<time><ending>`, where the
/// method stem is the file's stem without `ending` (".txt" when empty).
fn retire_to_old(path: &Path, ending: &str) -> io::Result<PathBuf> {
    let old = path.parent().unwrap_or(Path::new("")).join(METHOD_OLD_DIR);
    fs::create_dir_all(&old)?;
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (stem, ending) = if ending.is_empty() {
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        (stem, ".txt")
    } else {
        let cut = name.len().saturating_sub(ending.len());
        (name.get(..cut).unwrap_or(&name).to_string(), ending)
    };
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let dest = old.join(format!("{stem}_{stamp}{ending}"));
    fs::rename(path, &dest)?;
    Ok(dest)
}

/// Save method text (unescaped) as method `n`; returns the path written.
pub fn save_method_file(dir: &Path, n: u16, bytes: &[u8]) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let path = method_path(dir, n);
    let tmp = write_temp_file(dir, bytes)?;
    if let Some(existing) = find_method_file(dir, n) {
        if let Err(err) = retire_method_file(&existing) {
            let _ = fs::remove_file(&tmp);
            return Err(err);
        }
    } else if let Some(orphan) = find_method_setup_file(dir, n) {
        // a run setup left without its method must not pass to a new method n
        if let Err(err) = retire_to_old(&orphan, METHOD_SETUP_SUFFIX) {
            log::warn!("Could not move {} to old/: {err}", orphan.display());
        }
    }
    rename_or_clean(&tmp, &path)?;
    Ok(path)
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Messages that send method `n` to the Akta, and a note for the run log.
pub fn method_load_messages(dir: &Path, n: u16) -> io::Result<(Vec<OpcMessage>, String)> {
    let Some(path) = find_method_file(dir, n) else {
        return Ok((
            vec![(OPC_METHOD_LOAD_END, vec![0x01])],
            format!("Method {n} requested, but there is no such file"),
        ));
    };
    let mut msgs: Vec<OpcMessage> = vec![(OPC_METHOD_LOAD_BEGIN, Vec::new())];
    let lines = method_file_lines(&fs::read(&path)?);
    // The lines before a bad one are sent, then the error is reported.
    for (i, line) in lines.iter().enumerate() {
        let i = i + 1;
        let bad = if line.len() > METHOD_LINE_MAX {
            format!("line {i} is longer than {METHOD_LINE_MAX} characters")
        } else if line
            .iter()
            .any(|&b| !(0x20..=0x7e).contains(&b) && b != 0x09)
        {
            format!("line {i} has a non-ASCII character")
        } else {
            String::new()
        };
        if !bad.is_empty() {
            msgs.push((OPC_METHOD_LOAD_END, vec![0x02]));
            let note = format!(
                "Method {n} sent with an error: {bad} ({})",
                file_name(&path)
            );
            return Ok((msgs, note));
        }
        msgs.extend(method_line_messages(line));
    }
    msgs.push((OPC_METHOD_LOAD_END, vec![0x00]));
    Ok((
        msgs,
        format!("Method {n} sent to the Akta ({} lines)", lines.len()),
    ))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TransferMode {
    #[default]
    None,
    Dump,
    Save,
}

/// State of the method text transfer in progress (dump or save).
#[derive(Clone, Debug, Default)]
pub struct MethodTransfer {
    pub mode: TransferMode,
    pub number: u16,
    /// text received so far, still escaped
    pub buf: Vec<u8>,
}

/// What a received method message calls for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MethodAction {
    /// messages to send to the instrument
    pub replies: Vec<OpcMessage>,
    /// for the run log
    pub note: Option<String>,
    /// a finished method dump (escaped text)
    pub dump: Option<Vec<u8>>,
}

impl MethodAction {
    fn reply(replies: Vec<OpcMessage>, note: impl Into<String>) -> MethodAction {
        MethodAction {
            replies,
            note: Some(note.into()),
            dump: None,
        }
    }
}

pub fn is_method_opc(opc: u16) -> bool {
    matches!(
        opc,
        OPC_METHOD_DUMP_BEGIN
            | OPC_METHOD_DUMP_END
            | OPC_METHOD_SAVE_BEGIN
            | OPC_METHOD_SAVE_END
            | OPC_METHOD_LINE
            | OPC_METHOD_LINE_BEGIN
            | OPC_METHOD_LINE_CONT
            | OPC_METHOD_LINE_END
            | OPC_METHOD_LOAD
            | OPC_METHOD_LIST
            | OPC_METHOD_DELETE
            | OPC_METHOD_EXISTS
    )
}

impl MethodTransfer {
    /// Handle one message of the method protocol; None for any other message.
    /// Touches only the methods folder, so it can run on the link thread.
    pub fn handle(&mut self, opc: u16, data: &[u8], dir: &Path) -> Option<MethodAction> {
        if !is_method_opc(opc) {
            return None;
        }
        match opc {
            OPC_METHOD_DUMP_BEGIN => {
                self.mode = TransferMode::Dump;
                self.buf.clear();
                return Some(MethodAction::default());
            }
            OPC_METHOD_DUMP_END => {
                if self.mode != TransferMode::Dump {
                    return Some(MethodAction::default());
                }
                self.mode = TransferMode::None;
                let dump = std::mem::take(&mut self.buf);
                return Some(MethodAction {
                    dump: Some(dump),
                    ..MethodAction::default()
                });
            }
            OPC_METHOD_SAVE_BEGIN => {
                if data.len() == 2 {
                    self.mode = TransferMode::Save;
                    self.number = u16_at(data, 0);
                } else {
                    self.mode = TransferMode::None;
                    log::warn!("Method save request with {} bytes ignored", data.len());
                }
                self.buf.clear();
                return Some(MethodAction::default());
            }
            OPC_METHOD_LINE
            | OPC_METHOD_LINE_BEGIN
            | OPC_METHOD_LINE_CONT
            | OPC_METHOD_LINE_END => {
                if self.mode == TransferMode::None {
                    log::debug!("Method text outside a transfer ignored");
                } else {
                    self.buf.extend_from_slice(data);
                }
                return Some(MethodAction::default());
            }
            OPC_METHOD_SAVE_END => return Some(self.save_end(dir)),
            _ => {}
        }

        // Requests: each carries at least a method number.
        if data.len() < 2 {
            log::warn!("Method request 0x{opc:x} with {} bytes ignored", data.len());
            return Some(MethodAction::default());
        }
        let n = u16_at(data, 0);
        Some(match opc {
            OPC_METHOD_LIST => {
                let up = data.len() < 3 || data[2] != 0;
                let payload = method_list_payload(&stored_methods(dir), n, up);
                let listed: Vec<String> = payload
                    .chunks(2)
                    .map(|p| u16::from_le_bytes([p[0], p[1]]))
                    .filter(|&v| v != METHOD_NONE)
                    .map(|v| v.to_string())
                    .collect();
                log::info!(
                    "Akta asks for methods from {n} {}: {}",
                    if up { "up" } else { "down" },
                    if listed.is_empty() {
                        "none".to_string()
                    } else {
                        listed.join(", ")
                    }
                );
                MethodAction {
                    replies: vec![(OPC_METHOD_LIST_REPLY, payload)],
                    ..MethodAction::default()
                }
            }
            OPC_METHOD_EXISTS => {
                let exists = find_method_file(dir, n).is_some();
                log::info!(
                    "Akta asks whether method {n} exists: {}",
                    if exists { "yes" } else { "no" }
                );
                MethodAction {
                    replies: vec![(OPC_METHOD_EXISTS_REPLY, vec![u8::from(exists)])],
                    ..MethodAction::default()
                }
            }
            OPC_METHOD_DELETE => {
                let mut ok = false;
                if let Some(path) = find_method_file(dir, n) {
                    match retire_method_file(&path) {
                        Ok(_) => ok = true,
                        Err(err) => log::error!("Could not delete method {n}: {err}"),
                    }
                }
                let note = if ok {
                    format!(
                        "Method {n} deleted by the Akta (moved to {})",
                        dir.join(METHOD_OLD_DIR).display()
                    )
                } else {
                    format!("Akta asked to delete method {n} — not deleted")
                };
                MethodAction::reply(vec![(OPC_METHOD_DELETE_REPLY, vec![u8::from(ok)])], note)
            }
            _ => match method_load_messages(dir, n) {
                // OPC_METHOD_LOAD
                Ok((msgs, note)) => MethodAction::reply(msgs, note),
                Err(err) => {
                    log::error!("Could not read method {n}: {err}");
                    MethodAction::reply(
                        vec![(OPC_METHOD_LOAD_END, vec![0x01])],
                        format!("Method {n} could not be read"),
                    )
                }
            },
        })
    }

    fn save_end(&mut self, dir: &Path) -> MethodAction {
        if self.mode != TransferMode::Save {
            return MethodAction::reply(
                vec![(OPC_METHOD_SAVE_ACK, vec![0x00])],
                "Akta finished a method save that never started — not saved",
            );
        }
        let n = self.number;
        let text = unescape_method_text(&self.buf);
        self.mode = TransferMode::None;
        self.buf.clear();
        let saved = if METHOD_NUMBERS.contains(&n) {
            save_method_file(dir, n, &text)
        } else {
            Err(io::Error::other(format!(
                "method number {n} is outside 1–999"
            )))
        };
        match saved {
            Ok(path) => {
                let mut note = format!("Method {n} saved by the Akta to {}", path.display());
                // The Akta saves a method with a single breakpoint but cannot load it back
                // ("Reading Error on PC"): it needs a last breakpoint marking the end.
                if crate::method::parse_method_text(&text).breakpoints.len() < 2 {
                    note.push_str(
                        " — it has only one breakpoint, so the Akta cannot load it back; add an end breakpoint",
                    );
                    log::warn!("{note}");
                }
                MethodAction::reply(vec![(OPC_METHOD_SAVE_ACK, vec![0x01])], note)
            }
            Err(err) => {
                log::error!("Could not save method {n}: {err}");
                MethodAction::reply(
                    vec![(OPC_METHOD_SAVE_ACK, vec![0x00])],
                    format!("Method {n} from the Akta could not be saved"),
                )
            }
        }
    }
}
