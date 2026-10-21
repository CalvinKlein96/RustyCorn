//! RustyCorn core: the data model shared by every other crate: the run state
//! (`AppState`), the CSV log writer and reader, run information, settings,
//! the decoders of instrument log texts and the demo data source.

pub mod csvlog;
pub mod demo;
pub mod logtext;
pub mod opc;
pub mod runinfo;
pub mod settings;
pub mod types;

pub use types::*;

/// µL per volume count when nothing is configured (ÄKTAprime plus).
/// The ÄKTAprime (not plus) needs 1 µL — set in Settings.
pub const DEFAULT_VOLUME_UNIT_UL: f64 = 10.0;

pub const APP_NAME: &str = "RustyCorn";
pub const GITHUB_URL: &str = "https://github.com/CalvinKlein96/RustyCorn";
