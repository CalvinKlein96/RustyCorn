//! Ctrl-C in the terminal closes the window the normal way, so the program
//! still stops the acquisition and prints its report. A second Ctrl-C quits at
//! once, in case the window does not respond; the CSV log is flushed after every
//! row, so no data is lost either way.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

static INTERRUPTED: AtomicBool = AtomicBool::new(false);
static CONTEXT: OnceLock<egui::Context> = OnceLock::new();

pub fn install() {
    let result = ctrlc::set_handler(|| {
        if INTERRUPTED.swap(true, Ordering::SeqCst) {
            std::process::exit(130);
        }
        if let Some(ctx) = CONTEXT.get() {
            ctx.request_repaint();
        }
    });
    if let Err(err) = result {
        log::debug!("No Ctrl-C handler: {err}");
    }
}

/// The window to wake up when Ctrl-C arrives.
pub fn set_context(ctx: &egui::Context) {
    let _ = CONTEXT.set(ctx.clone());
}

/// True once Ctrl-C was pressed.
pub fn requested() -> bool {
    INTERRUPTED.load(Ordering::SeqCst)
}

/// Close the window if Ctrl-C was pressed; returns true then. Call every frame.
pub fn close_if_requested(ctx: &egui::Context) -> bool {
    if !requested() {
        return false;
    }
    static ONCE: AtomicBool = AtomicBool::new(false);
    if !ONCE.swap(true, Ordering::SeqCst) {
        log::info!("Interrupted");
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
    true
}
