//! Command line: no argument starts the live window, a
//! serial port overrides the saved one, a `.csv` file opens a read-only viewer
//! and `--methods` only the method editor. `--gpu <backend>` (with any of
//! them) chooses the graphics backend and is remembered.

use std::path::PathBuf;

use rc_core::settings::GpuBackend;

pub const USAGE: &str =
    "Usage: rustycorn [--gpu auto|gl|dx12|vulkan|metal] [serial_port | logfile.csv | --methods]";

/// The parsed command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Args {
    pub command: Command,
    /// `--gpu`: replaces the saved graphics backend (and is saved)
    pub gpu_backend: Option<GpuBackend>,
}

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

/// The arguments after the program name: `--gpu <backend>` or
/// `--gpu=<backend>` anywhere, at most once, and the command.
pub fn parse_args<S: AsRef<str>>(args: &[S]) -> Args {
    let usage = Args {
        command: Command::Usage,
        gpu_backend: None,
    };
    let mut rest = Vec::new();
    let mut gpu_backend = None;
    let mut it = args.iter().map(AsRef::as_ref);
    while let Some(arg) = it.next() {
        let value = match arg.strip_prefix("--gpu") {
            Some("") => it.next(),
            Some(v) if v.starts_with('=') => Some(&v[1..]),
            _ => {
                rest.push(arg);
                continue;
            }
        };
        match value.and_then(GpuBackend::parse) {
            Some(b) if gpu_backend.is_none() => gpu_backend = Some(b),
            _ => return usage,
        }
    }
    Args {
        command: parse_command(&rest),
        gpu_backend,
    }
}

/// The command for the arguments other than `--gpu`.
fn parse_command(args: &[&str]) -> Command {
    match args {
        [] => Command::Live {
            serial_override: None,
        },
        [arg] => {
            let arg = *arg;
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

    fn parse_command<S: AsRef<str>>(args: &[S]) -> Command {
        let parsed = parse_args(args);
        assert_eq!(parsed.gpu_backend, None);
        parsed.command
    }

    #[test]
    fn gpu_backend_with_any_command() {
        let gl = Some(GpuBackend::Gl);
        let live = Command::Live {
            serial_override: None,
        };
        assert_eq!(
            parse_args(&["--gpu", "gl"]),
            Args {
                command: live.clone(),
                gpu_backend: gl
            }
        );
        assert_eq!(parse_args(&["--gpu=OpenGL"]).gpu_backend, gl);
        assert_eq!(
            parse_args(&["COM3", "--gpu", "dx12"]),
            Args {
                command: Command::Live {
                    serial_override: Some("COM3".into())
                },
                gpu_backend: Some(GpuBackend::Dx12)
            }
        );
        assert_eq!(
            parse_args(&["--gpu", "vulkan", "run.csv"]).command,
            Command::Replay(PathBuf::from("run.csv"))
        );
        assert_eq!(
            parse_args(&["--gpu=auto", "--methods"]),
            Args {
                command: Command::Methods,
                gpu_backend: Some(GpuBackend::Auto)
            }
        );
    }

    #[test]
    fn bad_gpu_options_print_the_usage() {
        for args in [
            &["--gpu"][..],
            &["--gpu", "glide"],
            &["--gpu="],
            &["--gpu", "gl", "--gpu", "dx12"],
            &["--gpux"],
            &["--gpu", "gl", "COM3", "run.csv"],
        ] {
            assert_eq!(parse_args(args).command, Command::Usage, "{args:?}");
        }
    }

    #[test]
    fn no_arguments_start_live() {
        assert_eq!(
            parse_command::<&str>(&[]),
            Command::Live {
                serial_override: None
            }
        );
    }

    #[test]
    fn a_port_overrides_the_saved_one() {
        assert_eq!(
            parse_command(&["/dev/ttyUSB1"]),
            Command::Live {
                serial_override: Some("/dev/ttyUSB1".into())
            }
        );
        assert_eq!(
            parse_command(&["COM3"]),
            Command::Live {
                serial_override: Some("COM3".into())
            }
        );
    }

    #[test]
    fn csv_files_open_the_viewer() {
        assert_eq!(
            parse_command(&["run.csv"]),
            Command::Replay(PathBuf::from("run.csv"))
        );
        assert_eq!(
            parse_command(&["/data/RUN.CSV"]),
            Command::Replay(PathBuf::from("/data/RUN.CSV"))
        );
        assert!(is_logfile_arg("a.Csv"));
        assert!(!is_logfile_arg("a.csv.txt"));
    }

    #[test]
    fn methods_only() {
        assert_eq!(parse_command(&["--methods"]), Command::Methods);
    }

    #[test]
    fn everything_else_prints_the_usage() {
        assert_eq!(parse_command(&["--help"]), Command::Help);
        assert_eq!(parse_command(&["-h"]), Command::Help);
        assert_eq!(parse_command(&["--method"]), Command::Usage);
        assert_eq!(parse_command(&["-x"]), Command::Usage);
        assert_eq!(parse_command(&[" "]), Command::Usage);
        assert_eq!(parse_command(&["COM3", "run.csv"]), Command::Usage);
        assert_eq!(parse_command(&["--methods", "COM3"]), Command::Usage);
    }
}
