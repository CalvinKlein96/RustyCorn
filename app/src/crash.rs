//! Errors that end the program must stay readable. Started by a double-click,
//! Windows closes the console window the moment the program exits, taking the
//! error message with it; so a panic is also written to `crash.log` in the
//! settings folder, and on Windows the console waits for Enter first.

use std::io::Write;

/// Report panics on the console (as before), in `crash.log`, and keep a console
/// window of our own open after a panic of the main (GUI) thread. Panics of the
/// link thread are caught and logged there, and the program goes on.
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default(info);
        let thread = std::thread::current()
            .name()
            .unwrap_or("unnamed")
            .to_string();
        let backtrace = std::backtrace::Backtrace::force_capture();
        append_crash_log(&format!("panic in thread '{thread}': {info}\n{backtrace}"));
        if thread == "main" {
            pause_if_own_console();
        }
    }));
}

fn append_crash_log(text: &str) {
    let dir = rc_core::settings::config_dir();
    let path = dir.join("crash.log");
    let written = std::fs::create_dir_all(&dir).and_then(|()| {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        writeln!(
            f,
            "──── {} ────\n{text}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        )
    });
    match written {
        Ok(()) => eprintln!("\nThe error was also written to {}", path.display()),
        Err(err) => eprintln!("\nCould not write {}: {err}", path.display()),
    }
}

/// On Windows, wait for Enter when the console window belongs to this program
/// alone (it was started by a double-click and closes with it). From a Command
/// Prompt, or on other systems, return at once.
pub fn pause_if_own_console() {
    #[cfg(windows)]
    {
        let mut ids = [0u32; 2];
        // SAFETY: the buffer holds `ids.len()` process ids, as the call is told.
        let n = unsafe {
            windows_sys::Win32::System::Console::GetConsoleProcessList(ids.as_mut_ptr(), 2)
        };
        if n == 1 {
            eprintln!("\nPress Enter to close this window.");
            let _ = std::io::stdin().read_line(&mut String::new());
        }
    }
}
