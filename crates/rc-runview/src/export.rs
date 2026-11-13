//! "Save PDF": the displayed chromatogram (visible panels, current limits) is
//! rendered by rc-export on a background thread, so a long run never stalls
//! the GUI.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use rc_export::ExportRequest;

const NOT_AVAILABLE: &str = "PDF export not available yet";

/// How long the result message stays on screen.
pub const MESSAGE_SECS: f64 = 8.0;

pub struct Message {
    pub text: String,
    pub error: bool,
    pub at: Instant,
}

#[derive(Default)]
pub struct Exporter {
    /// figures saved from this page (numbers the files)
    pub counter: u32,
    pending: Option<Receiver<(PathBuf, Result<(), String>)>>,
    pub message: Option<Message>,
}

impl Exporter {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub fn say(&mut self, text: impl Into<String>, error: bool) {
        self.message = Some(Message {
            text: text.into(),
            error,
            at: Instant::now(),
        });
    }

    /// Render `req` to `path` on a worker thread; `ctx` is repainted when done.
    pub fn start(&mut self, req: ExportRequest, path: PathBuf, ctx: &egui::Context) {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("pdf-export".into())
            .spawn(move || {
                // a panic in the renderer must not take the GUI down with it
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    rc_export::save_pdf(&req, &path)
                }))
                .unwrap_or_else(|_| Err(NOT_AVAILABLE.to_string()));
                // the page may be gone by now; nothing to report then
                let _ = tx.send((path, res));
                ctx.request_repaint();
            });
        match spawned {
            Ok(_) => {
                self.pending = Some(rx);
                self.say("Saving plot…", false);
            }
            Err(err) => {
                log::error!("Could not save plot: {err}");
                self.say(format!("Could not save plot: {err}"), true);
            }
        }
    }

    /// Pick up a finished export.
    pub fn poll(&mut self) {
        let Some(rx) = &self.pending else { return };
        match rx.try_recv() {
            Ok((path, Ok(()))) => {
                log::info!("Saved plot to {}", path.display());
                self.say(format!("Saved plot to {}", path.display()), false);
                self.pending = None;
            }
            Ok((path, Err(err))) => {
                log::error!("Could not save plot {}: {err}", path.display());
                let msg = if err == NOT_AVAILABLE {
                    err
                } else {
                    format!("Could not save plot: {err}")
                };
                self.say(msg, true);
                self.pending = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                // the worker died without an answer
                self.say(NOT_AVAILABLE, true);
                self.pending = None;
            }
        }
    }

    /// The message to show now, if any, and how long it stays.
    pub fn current(&mut self) -> Option<(&Message, Duration)> {
        let keep = self
            .message
            .as_ref()
            .is_some_and(|m| self.pending.is_some() || m.at.elapsed().as_secs_f64() < MESSAGE_SECS);
        if !keep {
            self.message = None;
        }
        let left = self.message.as_ref().map(|m| {
            Duration::from_secs_f64((MESSAGE_SECS - m.at.elapsed().as_secs_f64()).max(0.05))
        });
        self.message.as_ref().zip(left)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> ExportRequest {
        ExportRequest {
            trend: vec![rc_core::TrendPoint::default()],
            events: Vec::new(),
            fractions: Vec::new(),
            panels: Vec::new(),
            xs: 0.0,
            xlim: (0.0, 1.0),
            overlay_ylim: rc_plotdata::OVERLAY_YLIMS,
            highlighted: None,
            title: None,
            events_shown: rc_plotdata::EventDisplay::All,
            show_fractions: true,
        }
    }

    #[test]
    fn export_reports_its_outcome_without_crashing() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut ex = Exporter::default();
        ex.start(
            request(),
            dir.path().join("figure.pdf"),
            &egui::Context::default(),
        );
        assert!(ex.busy());
        let t0 = Instant::now();
        while ex.busy() && t0.elapsed() < Duration::from_secs(30) {
            std::thread::sleep(Duration::from_millis(10));
            ex.poll();
        }
        assert!(!ex.busy());
        let (m, _) = ex.current().expect("a message");
        // either saved, or a clear error message instead of a crash
        assert!(m.text.starts_with("Saved plot") || m.error, "{}", m.text);
    }
}
