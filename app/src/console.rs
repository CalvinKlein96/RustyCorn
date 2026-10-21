//! Terminal output around the window: the start banner, the log-prefix
//! prompt and the closing report of the files written.

use std::io::{self, BufRead, Write};
use std::path::Path;

use rc_core::AppState;
use rc_core::settings::Settings;

fn bar() -> String {
    "─".repeat(64)
}

/// The text printed when the live window starts.
pub fn banner(s: &Settings) -> String {
    let data = s.data_dir();
    let methods = s.methods_dir_in(&data);
    let source = if s.demo_mode {
        "  DEMO MODE — simulated data (switch off under Settings).".to_string()
    } else {
        format!("  Serial port: {} (change under Settings).", s.serial_port)
    };
    [
        String::new(),
        bar(),
        format!("  {} — ÄKTAprime logger", rc_core::APP_NAME),
        "  Logging and visualisation for the ÄKTAprime / ÄKTAprime plus".to_string(),
        format!("  {}", rc_core::GITHUB_URL),
        bar(),
        String::new(),
        source,
        String::new(),
        "  Data are saved as CSV files in the data folder (change in Settings):".to_string(),
        format!("      {}", data.display()),
        "  named <prefix>_<date>_<time>.csv. A new file is started for every".to_string(),
        "  run; logs without any data are discarded.".to_string(),
        String::new(),
        "  Methods the Akta saves to or loads from the PC are in the methods".to_string(),
        "  folder (change in Settings):".to_string(),
        format!("      {}", methods.display()),
        String::new(),
        "  The prefix is a label to help you find these files later, such as".to_string(),
        "  a sample name or your initials. It can also be changed in Settings.".to_string(),
        bar(),
        String::new(),
    ]
    .join("\n")
}

/// Ask for the log-file prefix; ENTER (or end of input) keeps `default_prefix`.
pub fn prompt_log_prefix(
    default_prefix: &str,
    input: &mut impl BufRead,
    out: &mut impl Write,
) -> String {
    let _ = write!(
        out,
        "  Log-file prefix  (press ENTER for \"{default_prefix}\"): "
    );
    let _ = out.flush();
    let mut line = String::new();
    let answer = match input.read_line(&mut line) {
        Ok(_) => line.trim().to_string(),
        Err(_) => String::new(),
    };
    let _ = writeln!(out);
    if answer.is_empty() {
        default_prefix.to_string()
    } else {
        answer
    }
}

/// Ask on the terminal only: when started from a desktop shortcut or with
/// redirected input there is nobody to answer, so the last prefix is kept.
pub fn ask_log_prefix(default_prefix: &str) -> String {
    use std::io::IsTerminal;
    let stdin = io::stdin();
    if !stdin.is_terminal() {
        return default_prefix.to_string();
    }
    prompt_log_prefix(default_prefix, &mut stdin.lock(), &mut io::stdout())
}

/// The summary printed when the window closes (after the acquisition stopped).
pub fn report_output(app: &AppState) -> String {
    let dir = if app.log_dir.as_os_str().is_empty() {
        std::env::current_dir().unwrap_or_default()
    } else {
        app.log_dir.clone()
    };
    let recent = app
        .log_filename
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    [
        String::new(),
        "  Data saved in:".to_string(),
        format!("      {}", dir.display()),
        String::new(),
        "  Files this session are named:".to_string(),
        format!("      {}_<date>_<time>.csv", app.log_prefix),
        String::new(),
        format!("  Most recent run: {recent}"),
        bar(),
    ]
    .join("\n")
}

/// Print the closing report: where the data were saved and the last run's file.
pub fn print_report(app: &AppState) {
    println!();
    println!("{}", bar());
    log::info!(
        "Logging stopped — {} trend points in the final run",
        app.trend.len()
    );
    println!("{}", report_output(app));
}

/// File name for messages ("run.csv"), or the whole path if it has none.
pub fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::path::PathBuf;

    #[test]
    fn banner_names_both_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        let mut s = Settings {
            demo_mode: true,
            data_dir: data.display().to_string(),
            ..Settings::default()
        };
        let text = banner(&s);
        let line = |p: &std::path::Path| format!("      {}\n", p.display());
        assert!(text.contains(&line(&data)), "{text}");
        assert!(text.contains(&line(&data.join("methods"))), "{text}");
        let methods = tmp.path().join("shared methods");
        s.methods_dir = methods.display().to_string();
        let text = banner(&s);
        assert!(
            text.contains(&line(&methods)) && !text.contains(&line(&data.join("methods"))),
            "{text}"
        );
    }

    #[test]
    fn prompt_keeps_the_default_on_enter() {
        let mut out = Vec::new();
        assert_eq!(
            prompt_log_prefix("akta", &mut Cursor::new("\n"), &mut out),
            "akta"
        );
        assert!(String::from_utf8_lossy(&out).contains("press ENTER for \"akta\""));
        assert_eq!(
            prompt_log_prefix("akta", &mut Cursor::new(""), &mut Vec::new()),
            "akta"
        );
    }

    #[test]
    fn prompt_takes_the_typed_prefix() {
        let mut out = Vec::new();
        assert_eq!(
            prompt_log_prefix("akta", &mut Cursor::new("  lysozyme \r\n"), &mut out),
            "lysozyme"
        );
    }

    #[test]
    fn report_names_folder_prefix_and_last_run() {
        let mut app = AppState::new();
        app.log_dir = PathBuf::from("/data/akta");
        app.log_prefix = "akta".into();
        app.log_filename = PathBuf::from("/data/akta/akta_20261020_114429.csv");
        let r = report_output(&app);
        assert!(r.contains("      /data/akta\n"));
        assert!(r.contains("akta_<date>_<time>.csv"));
        assert!(r.contains("Most recent run: akta_20261020_114429.csv"));
    }

    #[test]
    fn display_names() {
        assert_eq!(display_name(Path::new("/a/b/run.csv")), "run.csv");
    }
}
