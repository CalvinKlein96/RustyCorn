//! ÄKTAprime serial protocol: link layer, OPC messages and data acquisition.
//!
//! The link layer runs on its own OS thread and polls the instrument every
//! 100 ms; decoded OPC messages are applied to the shared AppState under its
//! lock (short critical sections), method store requests are answered on the
//! link thread so replies never wait for the GUI.

pub mod acquisition;
pub mod frame;
pub mod handlers;
pub mod link;

pub use acquisition::Acquisition;

pub const BAUD_RATE: u32 = 9600;
pub const POLL_INTERVAL_MS: u64 = 100;
/// silence before we warn and start probing (never disconnects)
pub const IDLE_TIMEOUT_MS: u64 = 5000;
/// while silent, send a poll frame this often to catch a link reset
pub const PROBE_INTERVAL_MS: u64 = 1000;
pub const REPLY_TIMEOUT_MS: u64 = 500;
