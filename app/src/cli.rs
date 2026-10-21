//! Command line: no argument starts the live window, a
//! serial port overrides the saved one, a `.csv` file opens a read-only viewer
//! and `--methods` only the method editor.

use std::path::PathBuf;

pub const USAGE: &str = "Usage: rustycorn [serial_port | logfile.csv | --methods]";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// The main window; the source (demo or link) comes from Settings. A port
    /// given on the command line replaces the saved one (and is saved).
    Live { serial_override: Option<String> },
    /// A read-only viewer for a CSV log.
    Replay(PathBuf),
    /// Only the method editor: no serial link and no log file.
    Methods,
    /// `--help`: print the usage and exit successfully.
    Help,
    /// Anything else: print the usage and fail.
    Usage,
}

/// Whether an argument names a log file to open (ends in `.csv`, any case)
/// rather than a serial port.
pub fn is_logfile_arg(arg: &str) -> bool {
    arg.to_lowercase().ends_with(".csv")
}

/// The command for the arguments after the program name.
pub fn parse_args<S: AsRef<str>>(args: &[S]) -> Command {
    match args {
        [] => Command::Live {
            serial_override: None,
        },
        [arg] => {
            let arg = arg.as_ref();
            match arg {
                "--methods" => Command::Methods,
                "-h" | "--help" => Command::Help,
                // No serial port name starts with '-': an unknown option must not be
                // taken for a port (and saved to the settings).
                _ if arg.starts_with('-') => Command::Usage,
                _ if is_logfile_arg(arg) => Command::Replay(PathBuf::from(arg)),
                _ if arg.trim().is_empty() => Command::Usage,
                _ => Command::Live {
                    serial_override: Some(arg.to_string()),
                },
            }
        }
        _ => Command::Usage,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_arguments_start_live() {
        assert_eq!(
            parse_args::<&str>(&[]),
            Command::Live {
                serial_override: None
            }
        );
    }

    #[test]
    fn a_port_overrides_the_saved_one() {
        assert_eq!(
            parse_args(&["/dev/ttyUSB1"]),
            Command::Live {
                serial_override: Some("/dev/ttyUSB1".into())
            }
        );
        assert_eq!(
            parse_args(&["COM3"]),
            Command::Live {
                serial_override: Some("COM3".into())
            }
        );
    }

    #[test]
    fn csv_files_open_the_viewer() {
        assert_eq!(
            parse_args(&["run.csv"]),
            Command::Replay(PathBuf::from("run.csv"))
        );
        assert_eq!(
            parse_args(&["/data/RUN.CSV"]),
            Command::Replay(PathBuf::from("/data/RUN.CSV"))
        );
        assert!(is_logfile_arg("a.Csv"));
        assert!(!is_logfile_arg("a.csv.txt"));
    }

    #[test]
    fn methods_only() {
        assert_eq!(parse_args(&["--methods"]), Command::Methods);
    }

    #[test]
    fn everything_else_prints_the_usage() {
        assert_eq!(parse_args(&["--help"]), Command::Help);
        assert_eq!(parse_args(&["-h"]), Command::Help);
        assert_eq!(parse_args(&["--method"]), Command::Usage);
        assert_eq!(parse_args(&["-x"]), Command::Usage);
        assert_eq!(parse_args(&[" "]), Command::Usage);
        assert_eq!(parse_args(&["COM3", "run.csv"]), Command::Usage);
        assert_eq!(parse_args(&["--methods", "COM3"]), Command::Usage);
    }
}
